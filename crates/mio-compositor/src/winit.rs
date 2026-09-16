use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use mio_core::WindowId;
use smithay::{
    backend::{
        allocator::Fourcc,
        egl::EGLDevice,
        renderer::{
            damage::{Error as OutputDamageTrackerError, OutputDamageTracker},
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                texture::TextureRenderElement,
                utils::{CropRenderElement, RescaleRenderElement},
                AsRenderElements, Element, Id, Kind, RenderElement, RenderElementStates,
            },
            gles::{GlesRenderer, GlesTexture},
            utils::draw_render_elements,
            Bind, Frame, ImportAll, ImportDma, ImportEgl, Offscreen, Renderer, Texture,
        },
        winit::{self, WinitEvent},
    },
    desktop::{
        layer_map_for_output,
        space::{space_render_elements, SpaceRenderElements},
        utils::{
            send_frames_surface_tree, surface_presentation_feedback_flags_from_states,
            surface_primary_scanout_output, OutputPresentationFeedback,
        },
        LayerSurface,
    },
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{channel, EventLoop},
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    },
    utils::{IsAlive, Logical, Physical, Rectangle, Scale, Transform},
    wayland::compositor::with_states,
    wayland::dmabuf::DmabufFeedbackBuilder,
    wayland::presentation::Refresh,
    wayland::shell::wlr_layer::Layer as WlrLayer,
};
use tracing::{error, warn};

use crate::{
    effects::{
        compile_blur_shaders, compile_cursor_wake_shader, compile_focus_glow_shader,
        compile_rounding_shader, compile_shadow_shader, compile_window_border_shader, BlurOptions,
        CursorWakeElement, CursorWakeFrame, NonOccludingElement, RoundedElement, ShadowOptions,
        WindowBorderOptions,
    },
    state::{MioState, RenderWindow, StateResult},
    CalloopData,
};

type WinitSpaceElements = SpaceRenderElements<
    GlesRenderer,
    <RenderWindow as AsRenderElements<GlesRenderer>>::RenderElement,
>;

smithay::render_elements! {
    WinitSceneElement<=GlesRenderer>;
    Space=WinitSpaceElements,
    CursorWake=CursorWakeElement,
    Upper=NonOccludingElement<WinitSpaceElements>,
}

#[derive(Debug)]
struct WindowRenderSnapshot {
    id: smithay::backend::renderer::element::Id,
    texture: GlesTexture,
    geometry: Rectangle<i32, Physical>,
}

#[derive(Debug)]
struct ClosingVisual {
    snapshot: WindowRenderSnapshot,
    started: Instant,
    duration: Duration,
    effect: crate::config::WindowTransitionEffect,
}

const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct FrameDiagnostics {
    interval_started: Instant,
    frames: u64,
    submitted_frames: u64,
    cpu_time: Duration,
    max_cpu_time: Duration,
    damage_rects: u64,
    damage_pixels: u64,
    blur_window_frames: u64,
    max_blur_windows: usize,
}

impl FrameDiagnostics {
    fn new(now: Instant) -> Self {
        Self {
            interval_started: now,
            frames: 0,
            submitted_frames: 0,
            cpu_time: Duration::ZERO,
            max_cpu_time: Duration::ZERO,
            damage_rects: 0,
            damage_pixels: 0,
            blur_window_frames: 0,
            max_blur_windows: 0,
        }
    }

    fn record(
        &mut self,
        now: Instant,
        cpu_time: Duration,
        damage: Option<(usize, u64)>,
        output_pixels: u64,
        blur_windows: usize,
        submitted: bool,
    ) {
        self.frames += 1;
        self.submitted_frames += u64::from(submitted);
        self.cpu_time += cpu_time;
        self.max_cpu_time = self.max_cpu_time.max(cpu_time);
        if let Some((rects, pixels)) = damage {
            self.damage_rects += rects as u64;
            self.damage_pixels += pixels;
        }
        self.blur_window_frames += blur_windows as u64;
        self.max_blur_windows = self.max_blur_windows.max(blur_windows);

        let elapsed = now.saturating_duration_since(self.interval_started);
        if elapsed < DIAGNOSTICS_INTERVAL {
            return;
        }

        let seconds = elapsed.as_secs_f64().max(f64::EPSILON);
        let frames = self.frames.max(1);
        let possible_pixels = output_pixels.saturating_mul(frames);
        let damage_percent = if possible_pixels == 0 {
            0.0
        } else {
            self.damage_pixels as f64 * 100.0 / possible_pixels as f64
        };
        tracing::debug!(
            target: "mio_compositor::diagnostics",
            fps = self.frames as f64 / seconds,
            submitted_fps = self.submitted_frames as f64 / seconds,
            cpu_frame_avg_ms = self.cpu_time.as_secs_f64() * 1_000.0 / frames as f64,
            cpu_frame_max_ms = self.max_cpu_time.as_secs_f64() * 1_000.0,
            scene_damage_rects_per_frame = self.damage_rects as f64 / frames as f64,
            scene_damage_percent = damage_percent,
            blur_windows_avg = self.blur_window_frames as f64 / frames as f64,
            blur_windows_max = self.max_blur_windows,
            "frame diagnostics"
        );
        *self = Self::new(now);
    }
}

fn damage_metrics(damage: &[Rectangle<i32, Physical>]) -> (usize, u64) {
    let pixels = damage
        .iter()
        .map(|rect| {
            u64::try_from(rect.size.w.max(0)).unwrap_or_default()
                * u64::try_from(rect.size.h.max(0)).unwrap_or_default()
        })
        .sum();
    (damage.len(), pixels)
}

#[allow(clippy::too_many_lines)]
pub fn init(event_loop: &mut EventLoop<CalloopData>, data: &mut CalloopData) -> StateResult<()> {
    let (mut backend, winit_source) = winit::init::<GlesRenderer>()?;
    initialize_dmabuf(&mut backend, &mut data.state);
    let blur_programs = match compile_blur_shaders(backend.renderer()) {
        Ok(programs) => Some(programs),
        Err(error) => {
            warn!(%error, "backdrop blur shader unavailable; blur properties will be ignored");
            None
        }
    };
    let rounding_program = match compile_rounding_shader(backend.renderer()) {
        Ok(program) => Some(program),
        Err(error) => {
            warn!(%error, "corner rounding shader unavailable; corner-radius will be ignored");
            None
        }
    };
    let shadow_program = match compile_shadow_shader(backend.renderer()) {
        Ok(program) => Some(program),
        Err(error) => {
            warn!(%error, "shadow shader unavailable; window shadows will be ignored");
            None
        }
    };
    let focus_glow_program = match compile_focus_glow_shader(backend.renderer()) {
        Ok(program) => Some(program),
        Err(error) => {
            warn!(%error, "focus glow shader unavailable; focus indicator will be hidden");
            None
        }
    };
    let window_border_program = match compile_window_border_shader(backend.renderer()) {
        Ok(program) => Some(program),
        Err(error) => {
            warn!(%error, "window border shader unavailable; window borders will be hidden");
            None
        }
    };
    let cursor_wake_program = match compile_cursor_wake_shader(backend.renderer()) {
        Ok(program) => Some(program),
        Err(error) => {
            warn!(%error, "cursor wake shader unavailable; cursor wake will be ignored");
            None
        }
    };
    let cursor_wake_frame = Rc::new(RefCell::new(CursorWakeFrame::default()));
    let cursor_wake_id = Id::new();
    let mut cursor_wake_commit = 0_usize;
    let cursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|size| size.is_finite() && *size > 0.0)
        .unwrap_or(24.0);
    let mut frame_diagnostics = FrameDiagnostics::new(Instant::now());
    let mut window_snapshots = HashMap::<WindowId, WindowRenderSnapshot>::new();
    let mut closing_visuals = Vec::<ClosingVisual>::new();
    let size = backend.window_size();
    let output_ids = data
        .state
        .world
        .output_cameras()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let mut outputs = Vec::new();
    for (index, (id, region)) in output_ids
        .into_iter()
        .zip(output_regions(size, data.state.virtual_output_count()))
        .enumerate()
    {
        let output = Output::new(
            format!("mio-winit-{}", index + 1),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Mio".into(),
                model: "Nested".into(),
                serial_number: String::new(),
            },
        );
        let _global = output.create_global::<crate::state::MioState>(&data.display_handle);
        let mode = Mode {
            size: (region.size.w, region.size.h).into(),
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Flipped180),
            None,
            Some(region.loc),
        );
        output.set_preferred(mode);
        data.state.space.map_output(&output, region.loc);
        data.state.register_output(id, output.clone());
        outputs.push(output);
    }
    data.state.set_output_size(size.w, size.h);

    let primary_output = outputs
        .first()
        .cloned()
        .ok_or("Mio requires at least one Output")?;
    let mut damage_tracker = OutputDamageTracker::from_output(&primary_output);
    let backend = Rc::new(RefCell::new(backend));
    let redraw_backend = Rc::clone(&backend);
    let (redraw_sender, redraw_channel) = channel::channel();
    data.state.redraw_sender = Some(redraw_sender);
    event_loop
        .handle()
        .insert_source(redraw_channel, move |event, (), _data| {
            if matches!(event, channel::Event::Msg(())) {
                redraw_backend.borrow().window().request_redraw();
            }
        })?;
    event_loop
        .handle()
        .insert_source(winit_source, move |event, (), data| {
            let state = &mut data.state;
            match event {
                WinitEvent::Resized { size, .. } => {
                    for (output, region) in outputs.iter().zip(output_regions(size, outputs.len()))
                    {
                        output.change_current_state(
                            Some(Mode {
                                size: (region.size.w, region.size.h).into(),
                                refresh: 60_000,
                            }),
                            None,
                            None,
                            Some(region.loc),
                        );
                        state.space.map_output(output, region.loc);
                        layer_map_for_output(output).arrange();
                    }
                    state.set_output_size(size.w, size.h);
                }
                WinitEvent::Input(event) => {
                    let _ = state.process_input_event(event);
                    backend.borrow().window().request_redraw();
                }
                WinitEvent::Redraw => {
                    let mut backend = backend.borrow_mut();
                    let diagnostics_enabled = tracing::enabled!(
                        target: "mio_compositor::diagnostics",
                        tracing::Level::DEBUG
                    );
                    let diagnostics_frame_started = diagnostics_enabled.then(Instant::now);
                    let mut diagnostics_damage = None;
                    import_pending_dmabufs(&mut backend, state);
                    state.poll_xwayland_satellite();
                    state.poll_spawned_commands();
                    let now = std::time::Instant::now();
                    state.flush_pending_pointer_click(now);
                    state.poll_xdg_clients(now);
                    let mut animations_active = state.advance_animations(now);
                    if matches!(&state.cursor_image_status, CursorImageStatus::Surface(surface) if !surface.alive())
                    {
                        state.cursor_image_status = CursorImageStatus::default_named();
                    }
                    if state
                        .dnd_icon
                        .as_ref()
                        .is_some_and(|icon| !icon.surface.alive())
                    {
                        state.dnd_icon = None;
                    }
                    let cursor_status = state
                        .cursor_override
                        .map(CursorImageStatus::Named)
                        .unwrap_or_else(|| state.cursor_image_status.clone());
                    apply_host_cursor(backend.window(), &cursor_status);
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);
                    animations_active |=
                        state.sync_focus_indicator_contexts(focus_glow_program.as_ref());
                    let appearance = state.config.config().appearance;
                    let opacity = appearance.opacity;
                    let effects = state.config.config().effects;
                    let cursor_wake_enabled =
                        state.cursor_wake_override.unwrap_or(effects.cursor_wake);
                    let cursor_wake = if !state.session_locked && cursor_wake_enabled {
                        state.cursor_wake.active_wake(
                            now,
                            Duration::from_millis(u64::from(effects.cursor_wake_duration)),
                        )
                    } else {
                        *cursor_wake_frame.borrow_mut() = CursorWakeFrame::default();
                        None
                    };
                    let pointer_overlay_active = state.dnd_icon.is_some()
                        || matches!(
                            &state.cursor_image_status,
                            CursorImageStatus::Surface(_)
                        );
                    // The nested Wayland EGL surface can return EGL_BAD_SURFACE
                    // from eglQuerySurface while clients are actively resizing
                    // or committing translucent content. Treat its history as
                    // unavailable and render current scene damage from scratch.
                    let buffer_age = 0;
                    let blur_options = BlurOptions {
                        passes: effects.blur_passes,
                        offset: effects.blur_offset,
                    };
                    for window in state.space.elements() {
                        window.set_blur_context(blur_programs.as_ref(), blur_options);
                        window.set_rounding_context(
                            rounding_program.as_ref(),
                            appearance.corner_radius,
                            effects.window_transition,
                        );
                        #[allow(clippy::cast_precision_loss)]
                        window.set_window_border_context(
                            window_border_program.as_ref(),
                            (appearance.window_border_width > 0
                                && appearance.window_border_color[3] > 0.0)
                                .then_some(WindowBorderOptions {
                                    width: appearance.window_border_width as f32,
                                    color: appearance.window_border_color,
                                    corner_radius: appearance.corner_radius as f32,
                                }),
                        );
                        window.set_shadow_context(
                            shadow_program.as_ref(),
                            ShadowOptions {
                                radius: effects.shadow_radius * state.world.camera().zoom() as f32,
                                offset: effects.shadow_offset.map(|value| {
                                    (f64::from(value) * state.world.camera().zoom()).round() as i32
                                }),
                                color: effects.shadow_color,
                                corner_radius: (f64::from(appearance.corner_radius)
                                    * state.world.camera().zoom())
                                .round() as u32,
                            },
                        );
                    }
                    let cursor_wake_rendered =
                        cursor_wake.is_some() && cursor_wake_program.is_some();
                    if cursor_wake_rendered {
                        cursor_wake_commit = cursor_wake_commit.wrapping_add(1);
                    }
                    let cursor_wake_element = cursor_wake_program
                        .clone()
                        .zip(cursor_wake.clone())
                        .map(|(programs, wake)| {
                            CursorWakeElement::new(
                                cursor_wake_id.clone(),
                                smithay::backend::renderer::utils::CommitCounter::from(
                                    cursor_wake_commit,
                                ),
                                Rectangle::<i32, Logical>::from_size(size.to_logical(1)),
                                programs,
                                wake,
                                effects.cursor_wake_width,
                                cursor_size,
                                effects.cursor_wake_strength,
                                Rc::clone(&cursor_wake_frame),
                            )
                        });
                    let mut presentation_states = None;
                    let mut scene_damage_for_submit = None;
                    let mut restore_window_surface = match backend.bind() {
                        Ok((renderer, mut framebuffer)) => {
                            if state.session_locked {
                                if let Err(error) =
                                    render_session_lock(state, renderer, &mut framebuffer, size)
                                {
                                    error!(%error, "session lock render failed");
                                    state.loop_signal.stop();
                                    return;
                                }
                            } else if state.virtual_output_count() > 1 {
                                if let Err(error) = render_virtual_outputs(
                                    state,
                                    renderer,
                                    &mut framebuffer,
                                    size,
                                    appearance.background_color,
                                    cursor_wake_element.as_ref(),
                                )
                                {
                                    error!(%error, "virtual Output render failed");
                                    state.loop_signal.stop();
                                    return;
                                }
                            } else {
                                let upper_layer_element_count =
                                    upper_layer_element_count(renderer, &primary_output);
                                let render_result = space_render_elements::<_, RenderWindow, _>(
                                    renderer,
                                    [&state.space],
                                    &primary_output,
                                    opacity,
                                )
                                .map_err(
                                    OutputDamageTrackerError::OutputNoMode,
                                )
                                .and_then(|mut space_elements| {
                                    let layer_split_valid =
                                        upper_layer_element_count <= space_elements.len();
                                    let remaining = if layer_split_valid {
                                        space_elements.split_off(upper_layer_element_count)
                                    } else {
                                        error!(
                                            upper_layer_element_count,
                                            scene_element_count = space_elements.len(),
                                            "upper layer prefix exceeds the Smithay scene; skipping cursor wake"
                                        );
                                        Vec::new()
                                    };
                                    let mut elements = Vec::<WinitSceneElement>::new();
                                    if layer_split_valid && cursor_wake_element.is_some() {
                                        elements.extend(
                                            space_elements
                                                .into_iter()
                                                .map(NonOccludingElement::new)
                                                .map(WinitSceneElement::from),
                                        );
                                    } else {
                                        elements.extend(
                                            space_elements
                                                .into_iter()
                                                .map(WinitSceneElement::from),
                                        );
                                    }
                                    if let (true, Some(cursor_wake_element)) =
                                        (layer_split_valid, &cursor_wake_element)
                                    {
                                        elements.push(cursor_wake_element.clone().into());
                                    }
                                    elements.extend(
                                        remaining.into_iter().map(WinitSceneElement::from),
                                    );
                                    damage_tracker.render_output(
                                        renderer,
                                        &mut framebuffer,
                                        buffer_age,
                                        &elements,
                                        appearance.background_color,
                                    )
                                });
                                match render_result {
                                    Ok(result) => {
                                        diagnostics_damage =
                                            result.damage.map(|damage| damage_metrics(damage));
                                        scene_damage_for_submit = result.damage.cloned();
                                        presentation_states = Some(result.states);
                                    }
                                    Err(error) => {
                                        error!(%error, "render failed");
                                        state.loop_signal.stop();
                                        return;
                                    }
                                }
                            }
                            state.fulfill_screencopies(
                                renderer,
                                &framebuffer,
                                (size.w, size.h).into(),
                                state.start_time.elapsed(),
                            )
                        }
                        Err(error) => {
                            error!(%error, "failed to bind renderer");
                            state.loop_signal.stop();
                            return;
                        }
                    };

                    if !state.session_locked && state.virtual_output_count() == 1 {
                        let snapshot_result = backend.bind().and_then(|(renderer, _)| {
                            capture_window_snapshots(state, renderer, &mut window_snapshots)
                                .map_err(Into::into)
                        });
                        match snapshot_result {
                            Ok(()) => restore_window_surface = true,
                            Err(error) => {
                                warn!(%error, "failed to update Window render snapshots");
                            }
                        }
                    }

                    for transition in state.destroyed_window_transitions.drain(..) {
                        if let Some(snapshot) = window_snapshots.remove(&transition.id) {
                            closing_visuals.push(ClosingVisual {
                                snapshot,
                                started: now,
                                duration: transition.duration,
                                effect: transition.effect,
                            });
                        }
                    }
                    window_snapshots.retain(|id, _| {
                        state.managed_windows.iter().any(|managed| managed.id == *id)
                    });
                    closing_visuals.retain(|visual| now.duration_since(visual.started) < visual.duration);

                    if restore_window_surface {
                        let restore_result =
                            backend.bind().and_then(|(renderer, mut framebuffer)| {
                                renderer
                                    .render(&mut framebuffer, size, Transform::Flipped180)
                                    .map(drop)
                                    .map_err(Into::into)
                            });
                        if let Err(error) = restore_result {
                            error!(%error, "failed to restore render surface after screencopy");
                            state.loop_signal.stop();
                            return;
                        }
                    }

                    if !state.session_locked && !closing_visuals.is_empty() {
                        let closing_result = backend.bind().and_then(|(renderer, mut framebuffer)| {
                            render_closing_visuals(
                                renderer,
                                &mut framebuffer,
                                size,
                                &closing_visuals,
                                rounding_program.as_ref(),
                                state.config.config().appearance.corner_radius,
                                now,
                            )
                            .map_err(Into::into)
                        });
                        if let Err(error) = closing_result {
                            error!(%error, "closing Window snapshot render failed");
                            state.loop_signal.stop();
                            return;
                        }
                    }

                    if let Some(config_error) = (!state.session_locked)
                        .then_some(state.config_error.as_deref())
                        .flatten()
                    {
                        let overlay_result = backend.bind().and_then(|(renderer, mut framebuffer)| {
                            render_config_error_overlay(
                                renderer,
                                &mut framebuffer,
                                size,
                                config_error,
                            )
                            .map_err(Into::into)
                        });
                        if let Err(error) = overlay_result {
                            error!(%error, "configuration error overlay render failed");
                            state.loop_signal.stop();
                            return;
                        }
                    }

                    let cursor_result = backend.bind().and_then(|(renderer, mut framebuffer)| {
                        render_pointer_overlays(state, renderer, &mut framebuffer, size)
                            .map_err(Into::into)
                    });
                    if let Err(error) = cursor_result {
                        error!(%error, "cursor surface render failed");
                        state.loop_signal.stop();
                        return;
                    }

                    let needs_full_submit = state.session_locked
                        || state.virtual_output_count() > 1
                        || !closing_visuals.is_empty()
                        || state.config_error.is_some()
                        || cursor_wake_rendered
                        || pointer_overlay_active;
                    let submitted = if needs_full_submit {
                        match backend.submit(Some(&[damage])) {
                            Ok(()) => true,
                            Err(error) => {
                                error!(%error, "frame submission failed");
                                state.loop_signal.stop();
                                return;
                            }
                        }
                    } else if let Some(scene_damage) = scene_damage_for_submit.as_deref() {
                        match backend.submit(Some(scene_damage)) {
                            Ok(()) => true,
                            Err(error) => {
                                error!(%error, "frame submission failed");
                                state.loop_signal.stop();
                                return;
                            }
                        }
                    } else {
                        false
                    };

                    if let Some(frame_started) = diagnostics_frame_started {
                        let finished = Instant::now();
                        let output_pixels = u64::try_from(size.w.max(0)).unwrap_or_default()
                            * u64::try_from(size.h.max(0)).unwrap_or_default();
                        let blur_windows = state
                            .world
                            .windows()
                            .filter(|window| window.effective_properties().blur)
                            .count();
                        frame_diagnostics.record(
                            finished,
                            finished.saturating_duration_since(frame_started),
                            diagnostics_damage,
                            output_pixels,
                            blur_windows,
                            submitted,
                        );
                    }

                    if submitted {
                        if let Some(states) = presentation_states.as_ref() {
                            present_output_feedback(state, &primary_output, states);
                        }
                    }

                    if state.session_locked {
                        if let Some(confirmation) = state.pending_session_lock.take() {
                            confirmation.lock();
                        }
                    }

                    if state.session_locked {
                        for (output, surface) in &state.session_lock_surfaces {
                            send_frames_surface_tree(
                                surface.wl_surface(),
                                output,
                                state.start_time.elapsed(),
                                Some(Duration::ZERO),
                                |_, _| Some(output.clone()),
                            );
                        }
                    } else {
                        for window in state.space.elements() {
                            for output in &outputs {
                                window.send_frame(
                                    output,
                                    state.start_time.elapsed(),
                                    Some(Duration::ZERO),
                                    |_, _| Some(output.clone()),
                                );
                            }
                        }
                        for output in &outputs {
                            let layers = layer_map_for_output(output)
                                .layers()
                                .cloned()
                                .collect::<Vec<_>>();
                            for layer in layers {
                                layer.send_frame(
                                    output,
                                    state.start_time.elapsed(),
                                    Some(Duration::ZERO),
                                    |_, _| Some(output.clone()),
                                );
                            }
                        }
                    }
                    if let CursorImageStatus::Surface(surface) = &state.cursor_image_status {
                        let pointer_location = state
                            .seat
                            .get_pointer()
                            .expect("Mio always creates a pointer")
                            .current_location();
                        let output = outputs
                            .iter()
                            .find(|output| {
                                state
                                    .space
                                    .output_geometry(output)
                                    .is_some_and(|geometry| geometry.to_f64().contains(pointer_location))
                            })
                            .unwrap_or(&primary_output);
                        send_frames_surface_tree(
                            surface,
                            output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        );
                    }
                    if let Some(icon) = &state.dnd_icon {
                        let pointer_location = state
                            .seat
                            .get_pointer()
                            .expect("Mio always creates a pointer")
                            .current_location();
                        let output = outputs
                            .iter()
                            .find(|output| {
                                state.space.output_geometry(output).is_some_and(|geometry| {
                                    geometry.to_f64().contains(pointer_location)
                                })
                            })
                            .unwrap_or(&primary_output);
                        send_frames_surface_tree(
                            &icon.surface,
                            output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        );
                    }
                    state.space.refresh();
                    state.popups.cleanup();
                    state.cleanup_idle_inhibitors();
                    if let Err(error) = data.display_handle.flush_clients() {
                        warn!(%error, "failed to flush Wayland clients");
                    }
                    if animations_active
                        || !closing_visuals.is_empty()
                        || state.pending_pointer_click.is_some()
                        || cursor_wake_rendered
                    {
                        backend.window().request_redraw();
                    }
                }
                WinitEvent::CloseRequested => state.loop_signal.stop(),
                WinitEvent::Focus(_) => {}
            }
        })?;

    Ok(())
}

fn initialize_dmabuf(
    backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
    state: &mut MioState,
) {
    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .and_then(|device| device.try_get_render_node());
    let formats = backend.renderer().dmabuf_formats();
    let global = match render_node {
        Ok(Some(node)) => {
            match DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build() {
                Ok(feedback) => state
                    .dmabuf_state
                    .create_global_with_default_feedback::<MioState>(
                        &state.display_handle,
                        &feedback,
                    ),
                Err(error) => {
                    warn!(%error, "failed to build DMA-BUF v4 feedback; using v3");
                    state
                        .dmabuf_state
                        .create_global::<MioState>(&state.display_handle, formats)
                }
            }
        }
        Ok(None) => {
            warn!("render node unavailable; DMA-BUF will use protocol v3");
            state
                .dmabuf_state
                .create_global::<MioState>(&state.display_handle, formats)
        }
        Err(error) => {
            warn!(%error, "failed to query EGL render node; DMA-BUF will use protocol v3");
            state
                .dmabuf_state
                .create_global::<MioState>(&state.display_handle, formats)
        }
    };
    state.dmabuf_global = Some(global);

    if let Err(error) = backend.renderer().bind_wl_display(&state.display_handle) {
        warn!(%error, "failed to bind EGL to the Wayland display");
    }
}

fn import_pending_dmabufs(
    backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
    state: &mut MioState,
) {
    for (dmabuf, notifier) in state.pending_dmabuf_imports.drain(..) {
        if backend.renderer().import_dmabuf(&dmabuf, None).is_ok() {
            let _ = notifier.successful::<MioState>();
        } else {
            notifier.failed();
        }
    }
}

fn apply_host_cursor(
    window: &smithay::reexports::winit::window::Window,
    status: &CursorImageStatus,
) {
    match status {
        CursorImageStatus::Hidden => window.set_cursor_visible(false),
        CursorImageStatus::Named(icon) => {
            window.set_cursor_visible(true);
            window.set_cursor(*icon);
        }
        CursorImageStatus::Surface(_) => {
            window.set_cursor_visible(false);
        }
    }
}

fn render_pointer_overlays<R>(
    state: &MioState,
    renderer: &mut R,
    framebuffer: &mut R::Framebuffer<'_>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> Result<(), R::Error>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let pointer = state
        .seat
        .get_pointer()
        .expect("Mio always creates a pointer");
    let pointer_location = pointer.current_location();
    let mut elements = Vec::<WaylandSurfaceRenderElement<R>>::new();
    if let Some(surface) = state
        .cursor_override
        .is_none()
        .then_some(&state.cursor_image_status)
        .and_then(|status| match status {
            CursorImageStatus::Surface(surface) => Some(surface),
            _ => None,
        })
    {
        let hotspot = with_states(surface, |states| {
            states
                .data_map
                .get::<CursorImageSurfaceData>()
                .map_or_else(Default::default, |attributes| {
                    attributes.lock().unwrap().hotspot
                })
        });
        let location = (pointer_location - hotspot.to_f64())
            .to_physical(1.0)
            .to_i32_round();
        elements.extend(render_elements_from_surface_tree(
            renderer,
            surface,
            location,
            1.0,
            1.0,
            Kind::Cursor,
        ));
    }
    if let Some(icon) = &state.dnd_icon {
        let location = (pointer_location + icon.offset.to_f64())
            .to_physical(1.0)
            .to_i32_round();
        elements.extend(render_elements_from_surface_tree(
            renderer,
            &icon.surface,
            location,
            1.0,
            1.0,
            Kind::Unspecified,
        ));
    }
    if elements.is_empty() {
        return Ok(());
    }
    let damage = Rectangle::from_size(size);
    let mut frame = renderer.render(framebuffer, size, Transform::Flipped180)?;
    draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
    frame.finish().map(drop)
}

fn render_config_error_overlay<R>(
    renderer: &mut R,
    framebuffer: &mut R::Framebuffer<'_>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    error: &str,
) -> Result<(), R::Error>
where
    R: Renderer,
{
    let height = size.h.clamp(1, 72);
    let background = Rectangle::new((0, 0).into(), (size.w, height).into());
    let accent = Rectangle::new((0, 0).into(), (6.min(size.w), height).into());
    let mut frame = renderer.render(framebuffer, size, Transform::Flipped180)?;
    frame.draw_solid(
        background,
        &[Rectangle::from_size(background.size)],
        [0.45, 0.03, 0.04, 0.96].into(),
    )?;
    frame.draw_solid(
        accent,
        &[Rectangle::from_size(accent.size)],
        [1.0, 0.72, 0.18, 1.0].into(),
    )?;
    for rectangle in bitmap_text_rects("CONFIG ERROR - USING DEFAULTS", (14, 8), 2)
        .into_iter()
        .chain(bitmap_text_rects("FIX FILE, THEN CTRL+ALT+R", (14, 28), 2))
        .chain(bitmap_text_rects(
            &config_error_summary(
                error,
                usize::try_from((size.w - 28).max(0) / 12).unwrap_or(0),
            ),
            (14, 48),
            2,
        ))
        .filter(|rectangle| rectangle.loc.x < size.w && rectangle.loc.y < height)
    {
        frame.draw_solid(
            rectangle,
            &[Rectangle::from_size(rectangle.size)],
            [1.0, 0.96, 0.92, 1.0].into(),
        )?;
    }
    frame.finish().map(drop)
}

fn bitmap_text_rects(
    text: &str,
    origin: (i32, i32),
    scale: i32,
) -> Vec<Rectangle<i32, smithay::utils::Physical>> {
    let mut rectangles = Vec::new();
    for (character_index, character) in text.chars().enumerate() {
        let Ok(character_index) = i32::try_from(character_index) else {
            break;
        };
        let x = origin
            .0
            .saturating_add(character_index.saturating_mul(6).saturating_mul(scale));
        for (row, bits) in glyph_rows(character).into_iter().enumerate() {
            let Ok(row) = i32::try_from(row) else {
                continue;
            };
            let mut column = 0_i32;
            while column < 5 {
                if bits & (1 << (4 - column)) == 0 {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < 5 && bits & (1 << (4 - column)) != 0 {
                    column += 1;
                }
                rectangles.push(Rectangle::new(
                    (
                        x.saturating_add(start.saturating_mul(scale)),
                        origin.1.saturating_add(row.saturating_mul(scale)),
                    )
                        .into(),
                    ((column - start).saturating_mul(scale), scale).into(),
                ));
            }
        }
    }
    rectangles
}

#[allow(clippy::too_many_lines)]
fn glyph_rows(character: char) -> [u8; 7] {
    match character {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        ',' => [0, 0, 0, 0, 0, 0b00100, 0b01000],
        '.' => [0, 0, 0, 0, 0, 0, 0b00100],
        ':' => [0, 0b00100, 0, 0, 0b00100, 0, 0],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        _ => [0; 7],
    }
}

fn config_error_summary(error: &str, max_chars: usize) -> String {
    let detail = error.split_once(": ").map_or(error, |(_, detail)| detail);
    detail
        .chars()
        .map(|character| {
            let character = character.to_ascii_uppercase();
            if character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '+' | '-' | ',' | '.' | ':' | '/' | '_')
            {
                character
            } else {
                ' '
            }
        })
        .take(max_chars)
        .collect()
}

fn present_output_feedback(
    state: &mut MioState,
    output: &Output,
    render_states: &RenderElementStates,
) {
    let mut feedback = OutputPresentationFeedback::new(output);
    for window in state.space.elements() {
        if state.space.outputs_for_element(window).contains(output) {
            window.take_presentation_feedback(
                &mut feedback,
                surface_primary_scanout_output,
                |surface, _| {
                    surface_presentation_feedback_flags_from_states(surface, render_states)
                },
            );
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.take_presentation_feedback(
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| surface_presentation_feedback_flags_from_states(surface, render_states),
        );
    }
    let refresh = output.current_mode().map_or(Refresh::Unknown, |mode| {
        Refresh::fixed(Duration::from_secs_f64(1_000.0 / f64::from(mode.refresh)))
    });
    feedback.presented(
        state.presentation_clock.now(),
        refresh,
        state.presentation_sequence,
        wp_presentation_feedback::Kind::Vsync,
    );
    state.presentation_sequence = state.presentation_sequence.wrapping_add(1);
}

fn render_session_lock<R>(
    state: &MioState,
    renderer: &mut R,
    framebuffer: &mut R::Framebuffer<'_>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> Result<(), R::Error>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let damage = Rectangle::from_size(size);
    let mut elements = Vec::<WaylandSurfaceRenderElement<R>>::new();
    for (output, surface) in &state.session_lock_surfaces {
        let location = state
            .space
            .output_geometry(output)
            .map_or((0, 0), |geometry| geometry.loc.into());
        elements.extend(render_elements_from_surface_tree(
            renderer,
            surface.wl_surface(),
            location,
            1.0,
            1.0,
            Kind::Unspecified,
        ));
    }
    let mut frame = renderer.render(framebuffer, size, Transform::Flipped180)?;
    frame.clear([0.0, 0.0, 0.0, 1.0].into(), &[damage])?;
    draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
    frame.finish().map(drop)
}

fn output_regions(
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    count: usize,
) -> Vec<Rectangle<i32, smithay::utils::Logical>> {
    let count = i32::try_from(count).unwrap_or(1).max(1);
    (0..count)
        .map(|index| {
            let left = i32::try_from(i64::from(size.w) * i64::from(index) / i64::from(count))
                .unwrap_or_default();
            let right = i32::try_from(i64::from(size.w) * i64::from(index + 1) / i64::from(count))
                .unwrap_or(size.w);
            Rectangle::new(
                (left, 0).into(),
                (right.saturating_sub(left), size.h).into(),
            )
        })
        .collect()
}

fn upper_layer_element_count(renderer: &mut GlesRenderer, output: &Output) -> usize {
    let map = layer_map_for_output(output);
    let output_scale = output.current_scale().fractional_scale();
    map.layers()
        .rev()
        .filter(|layer| matches!(layer.layer(), WlrLayer::Top | WlrLayer::Overlay))
        .filter_map(|layer| map.layer_geometry(layer).map(|geometry| (layer, geometry)))
        .map(|(layer, geometry)| {
            <LayerSurface as AsRenderElements<GlesRenderer>>::render_elements::<
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(
                layer,
                renderer,
                geometry.loc.to_physical_precise_round(output_scale),
                Scale::from(output_scale),
                1.0,
            )
            .len()
        })
        .sum()
}

fn render_virtual_outputs(
    state: &MioState,
    renderer: &mut GlesRenderer,
    framebuffer: &mut <GlesRenderer as smithay::backend::renderer::RendererSuper>::Framebuffer<'_>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    background_color: [f32; 4],
    cursor_wake: Option<&CursorWakeElement>,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    let mut window_elements = Vec::new();
    for view in state.virtual_window_views((size.w, size.h)) {
        let base = smithay::desktop::space::SpaceElement::bbox(&view.window).size;
        let scale_x = f64::from(view.screen.width) / f64::from(base.w.max(1));
        let scale_y = f64::from(view.screen.height) / f64::from(base.h.max(1));
        let location = (view.screen.x, view.screen.y).into();
        let elements: Vec<<RenderWindow as AsRenderElements<GlesRenderer>>::RenderElement> =
            <RenderWindow as AsRenderElements<GlesRenderer>>::render_elements(
                &view.window,
                renderer,
                location,
                1.0.into(),
                1.0,
            );
        for element in elements.into_iter().rev() {
            let element =
                RescaleRenderElement::from_element(element, location, scale_x.min(scale_y));
            if let Some(element) = CropRenderElement::from_element(element, 1.0, view.crop) {
                window_elements.push(element);
            }
        }
    }

    let mut lower_layer_elements = Vec::new();
    let mut upper_layer_elements = Vec::new();
    for output in state.outputs.values() {
        let Some(output_geometry) = state.space.output_geometry(output) else {
            continue;
        };
        let crop = output_geometry.to_physical_precise_round(1.0);
        let map = layer_map_for_output(output);
        for layer_kind in [
            WlrLayer::Background,
            WlrLayer::Bottom,
            WlrLayer::Top,
            WlrLayer::Overlay,
        ] {
            for layer in map.layers_on(layer_kind) {
                let Some(geometry) = map.layer_geometry(layer) else {
                    continue;
                };
                let location = (output_geometry.loc + geometry.loc).to_physical_precise_round(1.0);
                let elements: Vec<<LayerSurface as AsRenderElements<GlesRenderer>>::RenderElement> =
                    <LayerSurface as AsRenderElements<GlesRenderer>>::render_elements(
                        layer,
                        renderer,
                        location,
                        1.0.into(),
                        1.0,
                    );
                for element in elements.into_iter().rev() {
                    let Some(element) = CropRenderElement::from_element(element, 1.0, crop) else {
                        continue;
                    };
                    if matches!(layer_kind, WlrLayer::Background | WlrLayer::Bottom) {
                        lower_layer_elements.push(element);
                    } else {
                        upper_layer_elements.push(element);
                    }
                }
            }
        }
    }

    let mut frame = renderer.render(framebuffer, size, Transform::Flipped180)?;
    let full = Rectangle::from_size(size);
    frame.clear(background_color.into(), &[full])?;
    for element in &lower_layer_elements {
        let geometry = element.geometry(1.0.into());
        let damage = Rectangle::from_size(geometry.size);
        element.draw(&mut frame, element.src(), geometry, &[damage], &[], None)?;
    }
    for element in &window_elements {
        let geometry = element.geometry(1.0.into());
        let damage = Rectangle::from_size(geometry.size);
        element.draw(&mut frame, element.src(), geometry, &[damage], &[], None)?;
    }
    if let Some(cursor_wake) = cursor_wake {
        cursor_wake.capture(&mut frame)?;
        let geometry = cursor_wake.geometry(1.0.into());
        cursor_wake.draw(
            &mut frame,
            cursor_wake.src(),
            geometry,
            &[Rectangle::from_size(geometry.size)],
            &[],
            None,
        )?;
    }
    for element in &upper_layer_elements {
        let geometry = element.geometry(1.0.into());
        let damage = Rectangle::from_size(geometry.size);
        element.draw(&mut frame, element.src(), geometry, &[damage], &[], None)?;
    }

    let count = i32::try_from(state.virtual_output_count()).unwrap_or(1);
    for index in 1..count {
        let x = i32::try_from(i64::from(size.w) * i64::from(index) / i64::from(count))
            .unwrap_or_default();
        let line = Rectangle::new((x.saturating_sub(1), 0).into(), (2, size.h).into());
        frame.draw_solid(
            line,
            &[Rectangle::from_size(line.size)],
            [0.31, 0.68, 0.82, 1.0].into(),
        )?;
    }
    frame.finish().map(drop)
}

fn capture_window_snapshots(
    state: &mut MioState,
    renderer: &mut GlesRenderer,
    snapshots: &mut HashMap<WindowId, WindowRenderSnapshot>,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    for managed in &mut state.managed_windows {
        if !managed.ready_to_present
            || managed.transition == crate::state::WindowTransition::Closing
        {
            continue;
        }
        let Some(mapped_location) = state.space.element_location(&managed.window) else {
            continue;
        };
        let geometry = smithay::desktop::space::SpaceElement::geometry(&managed.window);
        let bbox = smithay::desktop::space::SpaceElement::bbox(&managed.window);
        if bbox.size.w <= 0 || bbox.size.h <= 0 {
            continue;
        }
        let surface_origin = mapped_location - geometry.loc;
        let snapshot_geometry =
            Rectangle::new(surface_origin + bbox.loc, bbox.size).to_physical_precise_round(1.0);
        let size = bbox.size.to_buffer(1, Transform::Normal);
        let recreate = snapshots
            .get(&managed.id)
            .is_none_or(|snapshot| snapshot.texture.size() != size);
        if recreate {
            snapshots.insert(
                managed.id,
                WindowRenderSnapshot {
                    id: smithay::backend::renderer::element::Id::new(),
                    texture: renderer.create_buffer(Fourcc::Abgr8888, size)?,
                    geometry: snapshot_geometry,
                },
            );
        }
        let snapshot = snapshots
            .get_mut(&managed.id)
            .expect("snapshot was inserted above");
        snapshot.geometry = snapshot_geometry;
        if !managed.snapshot_dirty && !recreate {
            continue;
        }
        let render_origin = (-bbox.loc.x, -bbox.loc.y).into();
        let elements: Vec<<RenderWindow as AsRenderElements<GlesRenderer>>::RenderElement> =
            <RenderWindow as AsRenderElements<GlesRenderer>>::render_elements(
                &managed.window,
                renderer,
                render_origin,
                1.0.into(),
                1.0,
            );
        let mut target = renderer.bind(&mut snapshot.texture)?;
        let mut frame =
            renderer.render(&mut target, bbox.size.to_physical(1), Transform::Normal)?;
        let damage = Rectangle::from_size(bbox.size.to_physical(1));
        frame.clear([0.0, 0.0, 0.0, 0.0].into(), &[damage])?;
        draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
        frame.finish().map(drop)?;
        managed.snapshot_dirty = false;
    }
    Ok(())
}

fn render_closing_visuals(
    renderer: &mut GlesRenderer,
    framebuffer: &mut <GlesRenderer as smithay::backend::renderer::RendererSuper>::Framebuffer<'_>,
    size: smithay::utils::Size<i32, Physical>,
    visuals: &[ClosingVisual],
    transition_program: Option<&smithay::backend::renderer::gles::GlesTexProgram>,
    corner_radius: u32,
    now: Instant,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    let context = renderer.context_id();
    let mut frame = renderer.render(framebuffer, size, Transform::Flipped180)?;
    for visual in visuals {
        let progress = closing_visual_progress(
            now.saturating_duration_since(visual.started),
            visual.duration,
        );
        let texture = TextureRenderElement::from_static_texture(
            visual.snapshot.id.clone(),
            context.clone(),
            visual.snapshot.geometry.loc.to_f64(),
            visual.snapshot.texture.clone(),
            1,
            Transform::Normal,
            transition_program.is_none().then_some(progress),
            None,
            Some(visual.snapshot.geometry.size.to_logical(1)),
            None,
            Kind::Unspecified,
        );
        let damage = Rectangle::from_size(visual.snapshot.geometry.size);
        if let Some(program) = transition_program {
            #[allow(clippy::cast_precision_loss)]
            let radius = corner_radius as f32;
            let element = RoundedElement::new(
                texture,
                program.clone(),
                visual.snapshot.geometry,
                radius,
                progress,
                visual.effect.shader_value(),
                -1,
            );
            element.draw(
                &mut frame,
                element.src(),
                element.geometry(Scale::from(1.0)),
                &[damage],
                &[],
                None,
            )?;
        } else {
            <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                &texture,
                &mut frame,
                texture.src(),
                texture.geometry(Scale::from(1.0)),
                &[damage],
                &[],
                None,
            )?;
        }
    }
    frame.finish().map(drop)
}

fn closing_visual_progress(elapsed: Duration, duration: Duration) -> f32 {
    let duration = duration.as_secs_f32().max(f32::EPSILON);
    (1.0 - elapsed.as_secs_f32() / duration).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        bitmap_text_rects, closing_visual_progress, config_error_summary, damage_metrics,
        glyph_rows,
    };
    use smithay::utils::{Physical, Rectangle};

    #[test]
    fn diagnostics_sum_reported_damage_rectangles() {
        let damage = [
            Rectangle::<i32, Physical>::new((0, 0).into(), (20, 10).into()),
            Rectangle::<i32, Physical>::new((40, 30).into(), (5, 6).into()),
            Rectangle::<i32, Physical>::new((0, 0).into(), (0, 8).into()),
        ];
        assert_eq!(damage_metrics(&damage), (3, 230));
    }

    #[test]
    fn destroyed_window_snapshot_progress_runs_from_visible_to_gone() {
        let duration = Duration::from_millis(600);
        assert_eq!(closing_visual_progress(Duration::ZERO, duration), 1.0);
        assert!(
            (closing_visual_progress(Duration::from_millis(300), duration) - 0.5).abs() < 0.001
        );
        assert_eq!(closing_visual_progress(duration, duration), 0.0);
        assert_eq!(
            closing_visual_progress(Duration::from_secs(2), duration),
            0.0
        );
    }

    #[test]
    fn configuration_error_text_has_visible_pixels() {
        assert_ne!(glyph_rows('A'), [0; 7]);
        assert_eq!(glyph_rows(' '), [0; 7]);
        let rectangles = bitmap_text_rects("CONFIG ERROR", (14, 8), 2);
        assert!(!rectangles.is_empty());
        assert!(rectangles.iter().all(|rectangle| {
            rectangle.loc.x >= 14
                && rectangle.loc.y >= 8
                && rectangle.size.w > 0
                && rectangle.size.h > 0
        }));
        assert_eq!(
            config_error_summary(
                "/tmp/mio.kdl: appearance opacity must be between 0 and 1 at line 2",
                80
            ),
            "APPEARANCE OPACITY MUST BE BETWEEN 0 AND 1 AT LINE 2"
        );
    }
}
