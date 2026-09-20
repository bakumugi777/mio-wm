//! Direct single-GPU DRM/KMS backend.

// DRM, XCursor, and GLES expose narrower scalar types than Mio's logical model.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::{
    cell::RefCell,
    collections::HashMap,
    io::Read,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode,
        },
        egl::{context::ContextPriority, EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                texture::TextureRenderElement,
                AsRenderElements, Id, Kind,
            },
            gles::{GlesRenderer, GlesTexture},
            utils::draw_render_elements,
            Bind, Frame, ImportDma, ImportEgl, ImportMemWl, Offscreen, Renderer, Texture,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{primary_gpu, UdevBackend, UdevEvent},
    },
    desktop::{
        layer_map_for_output, space::space_render_elements, utils::send_frames_surface_tree,
        LayerSurface,
    },
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    output::{Mode, Output, PhysicalProperties, Scale as OutputScale},
    reexports::{
        calloop::{
            channel,
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        drm::control::{connector, crtc, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::compositor::with_states,
    wayland::dmabuf::DmabufFeedbackBuilder,
    wayland::shell::wlr_layer::Layer as WlrLayer,
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner, SimpleCrtcMapper};
use tracing::{debug, error, info, warn};

use crate::{
    effects::{
        compile_blur_shaders, compile_cursor_wake_shader, compile_focus_glow_shader,
        compile_rounding_shader, compile_shadow_shader, compile_window_border_shader, BlurOptions,
        BlurPrograms, CursorWakeElement, CursorWakeFrame, CursorWakePrograms, NonOccludingElement,
        RoundedElement, ShadowOptions, WindowBorderOptions,
    },
    input::BackendInputAction,
    state::RenderWindow,
    CalloopData,
};
use mio_core::WindowId;

type Scanout =
    DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type OutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;
type DirectSpaceElements = smithay::desktop::space::SpaceRenderElements<
    GlesRenderer,
    <RenderWindow as AsRenderElements<GlesRenderer>>::RenderElement,
>;

smithay::render_elements! {
    DirectRenderElement<=GlesRenderer>;
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, <RenderWindow as smithay::backend::renderer::element::AsRenderElements<GlesRenderer>>::RenderElement>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Closing=RoundedElement<TextureRenderElement<GlesTexture>>,
    ClosingTexture=TextureRenderElement<GlesTexture>,
    CursorWake=CursorWakeElement,
    Upper=NonOccludingElement<DirectSpaceElements>,
}

struct DirectBackend {
    session: LibSeatSession,
    renderer: GlesRenderer,
    node: DrmNode,
    output: Option<Output>,
    output_global: Option<smithay::reexports::wayland_server::backend::GlobalId>,
    crtc: Option<crtc::Handle>,
    scanner: DrmScanner<SimpleCrtcMapper>,
    manager: OutputManager,
    scanout: Option<Scanout>,
    frame_pending: bool,
    repaint_scheduled: bool,
    cursors: HashMap<CursorIcon, SoftwareCursor>,
    cursor_theme_name: String,
    cursor_size: u32,
    cursor_animation: Option<(CursorIcon, Instant)>,
    cursor_plane_assigned: Option<bool>,
    window_snapshots: HashMap<WindowId, WindowRenderSnapshot>,
    closing_visuals: Vec<ClosingVisual>,
    blur_programs: Option<BlurPrograms>,
    rounding_program: Option<smithay::backend::renderer::gles::GlesTexProgram>,
    shadow_program: Option<smithay::backend::renderer::gles::GlesPixelProgram>,
    focus_glow_program: Option<smithay::backend::renderer::gles::GlesPixelProgram>,
    window_border_program: Option<smithay::backend::renderer::gles::GlesPixelProgram>,
    cursor_wake_programs: Option<CursorWakePrograms>,
    cursor_wake_frame: Rc<RefCell<CursorWakeFrame>>,
    cursor_wake_id: Id,
    cursor_wake_commit: usize,
    config_error_overlay: ConfigErrorOverlayState,
    diagnostics_started: std::time::Instant,
    diagnostics_renders: u64,
    diagnostics_submits: u64,
    diagnostics_cpu: std::time::Duration,
    diagnostics_cpu_max: std::time::Duration,
}

#[derive(Default)]
struct ConfigErrorOverlayState {
    key: Option<(smithay::utils::Size<i32, Physical>, String)>,
    ids: Vec<Id>,
    commit: usize,
}

impl ConfigErrorOverlayState {
    fn elements(
        &mut self,
        size: smithay::utils::Size<i32, Physical>,
        error: &str,
    ) -> Vec<DirectRenderElement> {
        let key = (size, error.to_owned());
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.commit = self.commit.wrapping_add(1);
        }
        config_error_overlay_elements(size, error, &mut self.ids, self.commit)
    }
}

#[derive(Debug)]
struct WindowRenderSnapshot {
    id: Id,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HotplugEventKind {
    Connected,
    Disconnected,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HotplugAction {
    Connect,
    Disconnect,
    Reconnect,
    Ignore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectScene {
    Desktop,
    SessionLock,
}

const fn direct_scene(session_locked: bool) -> DirectScene {
    if session_locked {
        DirectScene::SessionLock
    } else {
        DirectScene::Desktop
    }
}

fn classify_hotplug<T: Copy + Eq>(
    active_crtc: Option<T>,
    event_crtc: Option<T>,
    kind: HotplugEventKind,
) -> HotplugAction {
    match (kind, active_crtc, event_crtc) {
        (HotplugEventKind::Connected, None, Some(_)) => HotplugAction::Connect,
        (HotplugEventKind::Disconnected, Some(active), Some(event)) if active == event => {
            HotplugAction::Disconnect
        }
        (HotplugEventKind::Changed, Some(active), Some(event)) if active == event => {
            HotplugAction::Reconnect
        }
        _ => HotplugAction::Ignore,
    }
}

struct SoftwareCursor {
    frames: Vec<SoftwareCursorFrame>,
    cycle_ms: u64,
}

struct SoftwareCursorFrame {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, smithay::utils::Logical>,
    source_size: Size<i32, Logical>,
    nominal_size: u32,
    delay_ms: u64,
}

impl SoftwareCursor {
    fn frame(&self, elapsed: Duration) -> &SoftwareCursorFrame {
        let position = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX) % self.cycle_ms;
        let mut end = 0;
        self.frames
            .iter()
            .find(|frame| {
                end += frame.delay_ms;
                position < end
            })
            .unwrap_or_else(|| self.frames.last().expect("a software cursor has frames"))
    }

    fn animated(&self) -> bool {
        self.frames.len() > 1
    }
}

#[derive(Debug)]
struct DirectEventDiagnostics {
    started: Instant,
    last_watchdog: Instant,
    last_vblank: Option<Instant>,
    watchdog_ticks: u64,
    vblanks: u64,
    input_events: u64,
    watchdog_late_max: Duration,
    vblank_gap_max: Duration,
    vblank_callback_max: Duration,
    input_callback_max: Duration,
}

impl DirectEventDiagnostics {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            last_watchdog: now,
            last_vblank: None,
            watchdog_ticks: 0,
            vblanks: 0,
            input_events: 0,
            watchdog_late_max: Duration::ZERO,
            vblank_gap_max: Duration::ZERO,
            vblank_callback_max: Duration::ZERO,
            input_callback_max: Duration::ZERO,
        }
    }

    fn watchdog(&mut self, now: Instant) {
        const PERIOD: Duration = Duration::from_millis(100);
        let elapsed = now.saturating_duration_since(self.last_watchdog);
        self.last_watchdog = now;
        self.watchdog_ticks += 1;
        self.watchdog_late_max = self.watchdog_late_max.max(elapsed.saturating_sub(PERIOD));
        if now.saturating_duration_since(self.started) >= Duration::from_secs(5) {
            debug!(
                target: "mio_compositor::diagnostics",
                backend = "udev-events",
                watchdog_ticks = self.watchdog_ticks,
                watchdog_late_max_ms = self.watchdog_late_max.as_secs_f64() * 1000.0,
                vblanks = self.vblanks,
                vblank_gap_max_ms = self.vblank_gap_max.as_secs_f64() * 1000.0,
                vblank_callback_max_ms = self.vblank_callback_max.as_secs_f64() * 1000.0,
                input_events = self.input_events,
                input_callback_max_ms = self.input_callback_max.as_secs_f64() * 1000.0,
                "event-loop diagnostics"
            );
            self.started = now;
            self.watchdog_ticks = 0;
            self.vblanks = 0;
            self.input_events = 0;
            self.watchdog_late_max = Duration::ZERO;
            self.vblank_gap_max = Duration::ZERO;
            self.vblank_callback_max = Duration::ZERO;
            self.input_callback_max = Duration::ZERO;
        }
    }

    fn begin_vblank(&mut self, now: Instant) -> Instant {
        if let Some(previous) = self.last_vblank.replace(now) {
            self.vblank_gap_max = self
                .vblank_gap_max
                .max(now.saturating_duration_since(previous));
        }
        self.vblanks += 1;
        now
    }

    fn finish_vblank(&mut self, started: Instant) {
        self.vblank_callback_max = self.vblank_callback_max.max(started.elapsed());
    }

    fn finish_input(&mut self, started: Instant) {
        self.input_events += 1;
        self.input_callback_max = self.input_callback_max.max(started.elapsed());
    }
}

#[allow(clippy::too_many_lines)]
pub fn init(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, session_notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();
    let primary_path = primary_gpu(&seat_name)?.ok_or("no DRM GPU found for the active seat")?;
    let node = DrmNode::from_path(&primary_path)?;
    info!(path = %primary_path.display(), %node, seat = %seat_name, "initializing direct DRM backend");

    let fd = session.open(
        &primary_path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new_with_priority(&display, ContextPriority::High)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    initialize_dmabuf(&mut renderer, node, &mut data.state);
    let blur_programs = compile_blur_shaders(&mut renderer)
        .map_err(|error| warn!(%error, "backdrop blur shader unavailable"))
        .ok();
    let rounding_program = compile_rounding_shader(&mut renderer)
        .map_err(|error| warn!(%error, "corner rounding shader unavailable"))
        .ok();
    let shadow_program = compile_shadow_shader(&mut renderer)
        .map_err(|error| warn!(%error, "shadow shader unavailable"))
        .ok();
    let focus_glow_program = compile_focus_glow_shader(&mut renderer)
        .map_err(|error| warn!(%error, "focus glow shader unavailable"))
        .ok();
    let window_border_program = compile_window_border_shader(&mut renderer)
        .map_err(|error| warn!(%error, "window border shader unavailable"))
        .ok();
    let cursor_wake_programs = compile_cursor_wake_shader(&mut renderer)
        .map_err(|error| warn!(%error, "cursor wake shader unavailable"))
        .ok();
    let (cursors, cursor_theme_name, cursor_size) = load_cursors();

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(node).into());
    let formats = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied();
    let manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        [
            Fourcc::Abgr2101010,
            Fourcc::Argb2101010,
            Fourcc::Abgr8888,
            Fourcc::Argb8888,
        ],
        formats,
    );

    let mut scanner: DrmScanner<SimpleCrtcMapper> = DrmScanner::new();
    let initial_connector = scanner
        .scan_connectors(manager.device())?
        .into_iter()
        .find_map(|event| match event {
            smithay_drm_extras::drm_scanner::DrmScanEvent::Connected {
                connector,
                crtc: Some(crtc),
            } => Some((connector, crtc)),
            _ => None,
        });
    data.state.shm_state.update_formats(renderer.shm_formats());

    let backend = Rc::new(RefCell::new(DirectBackend {
        session: session.clone(),
        renderer,
        node,
        output: None,
        output_global: None,
        crtc: None,
        scanner,
        manager,
        scanout: None,
        frame_pending: false,
        repaint_scheduled: false,
        cursors,
        cursor_theme_name,
        cursor_size,
        cursor_animation: None,
        cursor_plane_assigned: None,
        window_snapshots: HashMap::new(),
        closing_visuals: Vec::new(),
        blur_programs,
        rounding_program,
        shadow_program,
        focus_glow_program,
        window_border_program,
        cursor_wake_programs,
        cursor_wake_frame: Rc::new(RefCell::new(CursorWakeFrame::default())),
        cursor_wake_id: Id::new(),
        cursor_wake_commit: 0,
        config_error_overlay: ConfigErrorOverlayState::default(),
        diagnostics_started: std::time::Instant::now(),
        diagnostics_renders: 0,
        diagnostics_submits: 0,
        diagnostics_cpu: std::time::Duration::ZERO,
        diagnostics_cpu_max: std::time::Duration::ZERO,
    }));

    if let Some((connector, crtc)) = initial_connector {
        backend.borrow_mut().connect_output(data, &connector, crtc);
    } else {
        info!("no connected DRM output; waiting for a connector");
    }

    let event_diagnostics = Rc::new(RefCell::new(DirectEventDiagnostics::new(Instant::now())));
    let watchdog_diagnostics = Rc::clone(&event_diagnostics);
    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_millis(100)),
        move |_, &mut (), _| {
            watchdog_diagnostics.borrow_mut().watchdog(Instant::now());
            TimeoutAction::ToDuration(Duration::from_millis(100))
        },
    )?;

    // A DRM VBlank naturally drives the next animated frame after a submit.
    // If damage tracking produces an empty frame, however, there is no VBlank
    // to continue from. Re-arm that path at roughly the output refresh rate.
    let repaint_backend = Rc::clone(&backend);
    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_millis(16)),
        move |_, &mut (), data| {
            let should_repaint = {
                let mut backend = repaint_backend.borrow_mut();
                if backend.repaint_scheduled
                    && !backend.frame_pending
                    && backend.session.is_active()
                {
                    backend.repaint_scheduled = false;
                    backend.send_frame_callbacks(&data.state);
                    true
                } else {
                    false
                }
            };
            if should_repaint {
                repaint_backend.borrow_mut().render(&mut data.state);
            }
            TimeoutAction::ToDuration(Duration::from_millis(16))
        },
    )?;

    let redraw_backend = Rc::clone(&backend);
    let (redraw_sender, redraw_channel) = channel::channel();
    data.state.redraw_sender = Some(redraw_sender);
    event_loop
        .handle()
        .insert_source(redraw_channel, move |event, (), data| {
            if matches!(event, channel::Event::Msg(())) {
                redraw_backend.borrow_mut().render(&mut data.state);
            }
        })?;

    let drm_backend = Rc::clone(&backend);
    let drm_diagnostics = Rc::clone(&event_diagnostics);
    event_loop
        .handle()
        .insert_source(drm_notifier, move |event, _, data| {
            let callback_started = Instant::now();
            drm_diagnostics.borrow_mut().begin_vblank(callback_started);
            match event {
                DrmEvent::VBlank(crtc) => {
                    let mut backend = drm_backend.borrow_mut();
                    if Some(crtc) == backend.crtc {
                        if let Some(scanout) = backend.scanout.as_mut() {
                            if let Err(error) = scanout.frame_submitted() {
                                error!(%error, "failed to complete submitted DRM frame");
                                backend.frame_pending = false;
                            } else {
                                backend.frame_pending = false;
                                backend.send_frame_callbacks(&data.state);
                                backend.render(&mut data.state);
                            }
                        }
                    }
                }
                DrmEvent::Error(error) => error!(%error, "DRM event failed"),
            }
            drm_diagnostics.borrow_mut().finish_vblank(callback_started);
        })?;

    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "libinput rejected the active seat")?;
    let input_backend = LibinputInputBackend::new(libinput.clone());
    let input_render_backend = Rc::clone(&backend);
    let input_diagnostics = Rc::clone(&event_diagnostics);
    let mut input_session = session.clone();
    event_loop
        .handle()
        .insert_source(input_backend, move |event, &mut (), data| {
            let callback_started = Instant::now();
            if !matches!(
                event,
                InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }
            ) {
                if let Some(BackendInputAction::ChangeVt(vt)) =
                    data.state.process_input_event(event)
                {
                    info!(vt, "switching virtual terminal");
                    if let Err(error) = input_session.change_vt(vt) {
                        error!(%error, vt, "failed to switch virtual terminal");
                    }
                }
                input_render_backend.borrow_mut().render(&mut data.state);
            }
            input_diagnostics
                .borrow_mut()
                .finish_input(callback_started);
        })?;

    let session_backend = Rc::clone(&backend);
    event_loop
        .handle()
        .insert_source(session_notifier, move |event, &mut (), data| {
            let mut backend = session_backend.borrow_mut();
            match event {
                SessionEvent::PauseSession => {
                    libinput.suspend();
                    backend.manager.pause();
                    backend.frame_pending = false;
                }
                SessionEvent::ActivateSession => {
                    if let Err(error) = libinput.resume() {
                        warn!(?error, "failed to resume libinput");
                    }
                    if let Err(error) = backend.manager.lock().activate(false) {
                        error!(%error, "failed to reactivate DRM outputs");
                        return;
                    }
                    backend.render(&mut data.state);
                }
            }
        })?;

    let udev = UdevBackend::new(&seat_name)?;
    let hotplug_backend = Rc::clone(&backend);
    event_loop
        .handle()
        .insert_source(udev, move |event, &mut (), data| {
            let relevant = match event {
                UdevEvent::Changed { device_id }
                | UdevEvent::Removed { device_id }
                | UdevEvent::Added { device_id, .. } => {
                    device_id == hotplug_backend.borrow().node.dev_id()
                }
            };
            if relevant {
                hotplug_backend.borrow_mut().rescan_connectors(data);
            }
        })?;

    backend.borrow_mut().render(&mut data.state);
    Ok(())
}

fn initialize_dmabuf(
    renderer: &mut GlesRenderer,
    node: DrmNode,
    state: &mut crate::state::MioState,
) {
    let formats = renderer.dmabuf_formats();
    let global = match DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build() {
        Ok(feedback) => state
            .dmabuf_state
            .create_global_with_default_feedback::<crate::state::MioState>(
                &state.display_handle,
                &feedback,
            ),
        Err(error) => {
            warn!(%error, "failed to build DMA-BUF v4 feedback; using v3");
            state
                .dmabuf_state
                .create_global::<crate::state::MioState>(&state.display_handle, formats)
        }
    };
    state.dmabuf_global = Some(global);
    if let Err(error) = renderer.bind_wl_display(&state.display_handle) {
        warn!(%error, "failed to bind EGL to the Wayland display");
    }
}

fn load_cursors() -> (HashMap<CursorIcon, SoftwareCursor>, String, u32) {
    let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
    let requested_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(24);
    let theme = xcursor::CursorTheme::load(&theme_name);
    let mut cursors = HashMap::new();
    for icon in [
        CursorIcon::Default,
        CursorIcon::EwResize,
        CursorIcon::NsResize,
        CursorIcon::NeswResize,
        CursorIcon::NwseResize,
    ] {
        if let Some(cursor) = load_theme_cursor(&theme, icon.name(), requested_size) {
            cursors.insert(icon, cursor);
        }
    }
    cursors.entry(CursorIcon::Default).or_insert_with(|| {
        warn!(%theme_name, "default XCursor unavailable; using built-in pointer");
        built_in_cursor()
    });
    (cursors, theme_name, requested_size)
}

fn load_theme_cursor(
    theme: &xcursor::CursorTheme,
    name: &str,
    requested_size: u32,
) -> Option<SoftwareCursor> {
    let images = theme.load_icon(name).and_then(|path| {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .read_to_end(&mut bytes)
            .ok()?;
        xcursor::parser::parse_xcursor(&bytes)
    })?;
    let selected_size = images
        .iter()
        .min_by_key(|image| image.size.abs_diff(requested_size))?
        .size;
    let frames = images
        .into_iter()
        .filter(|image| image.size == selected_size)
        .map(|image| software_cursor_frame(&image))
        .collect::<Vec<_>>();
    let cycle_ms = frames
        .iter()
        .map(|frame| frame.delay_ms)
        .sum::<u64>()
        .max(1);
    (!frames.is_empty()).then_some(SoftwareCursor { frames, cycle_ms })
}

fn software_cursor_frame(image: &xcursor::parser::Image) -> SoftwareCursorFrame {
    let buffer = MemoryRenderBuffer::from_slice(
        &image.pixels_rgba,
        Fourcc::Argb8888,
        (image.width as i32, image.height as i32),
        1,
        Transform::Normal,
        None,
    );
    SoftwareCursorFrame {
        buffer,
        hotspot: (image.xhot as i32, image.yhot as i32).into(),
        source_size: (image.width as i32, image.height as i32).into(),
        nominal_size: image.size.max(1),
        delay_ms: u64::from(image.delay.max(1)),
    }
}

fn built_in_cursor() -> SoftwareCursor {
    let width = 24_u32;
    let height = 24_u32;
    let mut pixels = vec![0_u8; (width * height * 4) as usize];
    for y in 0..18_u32 {
        for x in 0..=y.min(9) {
            let offset = ((y * width + x) * 4) as usize;
            if x == 0 || x == y.min(9) || y == 17 {
                pixels[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
            } else if x < y.min(9) {
                pixels[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    let frame = SoftwareCursorFrame {
        buffer: MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (width as i32, height as i32),
            1,
            Transform::Normal,
            None,
        ),
        hotspot: (1, 1).into(),
        source_size: (width as i32, height as i32).into(),
        nominal_size: width,
        delay_ms: 1,
    };
    SoftwareCursor {
        frames: vec![frame],
        cycle_ms: 1,
    }
}

fn capture_window_snapshots(
    state: &mut crate::state::MioState,
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
                    id: Id::new(),
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

fn fulfill_direct_screencopies(
    state: &mut crate::state::MioState,
    renderer: &mut GlesRenderer,
    elements: &[DirectRenderElement],
    size: smithay::utils::Size<i32, Physical>,
    background: [f32; 4],
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    let buffer_size = (size.w, size.h).into();
    let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Argb8888, buffer_size)?;
    let mut target = renderer.bind(&mut texture)?;
    {
        let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
        let damage = Rectangle::from_size(size);
        frame.clear(background.into(), &[damage])?;
        draw_render_elements(&mut frame, 1.0, elements, &[damage])?;
        frame.finish().map(drop)?;
    }
    state.fulfill_screencopies(
        renderer,
        &target,
        buffer_size,
        state.presentation_clock.now().into(),
    );
    state.fulfill_image_copy_captures(
        renderer,
        &target,
        buffer_size,
        state.presentation_clock.now().into(),
    );
    Ok(())
}

fn closing_visual_elements(
    renderer: &GlesRenderer,
    visuals: &[ClosingVisual],
    transition_program: Option<&smithay::backend::renderer::gles::GlesTexProgram>,
    corner_radius: u32,
    now: Instant,
) -> Vec<DirectRenderElement> {
    let context = renderer.context_id();
    visuals
        .iter()
        .map(|visual| {
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
            if let Some(program) = transition_program {
                #[allow(clippy::cast_precision_loss)]
                let radius = corner_radius as f32;
                DirectRenderElement::Closing(RoundedElement::new(
                    texture,
                    program.clone(),
                    visual.snapshot.geometry,
                    radius,
                    progress,
                    visual.effect.shader_value(),
                    -1,
                ))
            } else {
                DirectRenderElement::ClosingTexture(texture)
            }
        })
        .collect()
}

fn closing_visual_progress(elapsed: Duration, duration: Duration) -> f32 {
    let duration = duration.as_secs_f32().max(f32::EPSILON);
    (1.0 - elapsed.as_secs_f32() / duration).clamp(0.0, 1.0)
}

fn create_output(
    connector: &connector::Info,
    scale: f64,
) -> Result<(Output, usize), Box<dyn std::error::Error>> {
    let (mode, drm_mode) =
        preferred_connector_mode(connector).ok_or("connected DRM connector exposes no modes")?;
    let name = format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id()
    );
    let (width_mm, height_mm) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        name,
        PhysicalProperties {
            size: (
                i32::try_from(width_mm).unwrap_or_default(),
                i32::try_from(height_mm).unwrap_or_default(),
            )
                .into(),
            subpixel: connector.subpixel().into(),
            make: "Unknown".into(),
            model: "Unknown".into(),
            serial_number: String::new(),
        },
    );
    configure_output_mode(&output, drm_mode, scale);
    Ok((output, mode))
}

fn preferred_connector_mode(
    connector: &connector::Info,
) -> Option<(usize, smithay::reexports::drm::control::Mode)> {
    let index = connector
        .modes()
        .iter()
        .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .unwrap_or(0);
    connector
        .modes()
        .get(index)
        .copied()
        .map(|mode| (index, mode))
}

fn configure_output_mode(
    output: &Output,
    drm_mode: smithay::reexports::drm::control::Mode,
    scale: f64,
) {
    let wl_mode = Mode::from(drm_mode);
    info!(
        output = %output.name(),
        width = wl_mode.size.w,
        height = wl_mode.size.h,
        refresh_millihz = wl_mode.refresh,
        "selected DRM output mode"
    );
    output.set_preferred(wl_mode);
    output.change_current_state(
        Some(wl_mode),
        Some(Transform::Normal),
        Some(OutputScale::Fractional(scale)),
        Some((0, 0).into()),
    );
}

fn direct_upper_layer_element_count(renderer: &mut GlesRenderer, output: &Output) -> usize {
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

impl DirectBackend {
    fn record_cursor_plane_assignment(&mut self, assigned: bool) {
        if self.cursor_plane_assigned.replace(assigned) != Some(assigned) {
            debug!(
                backend = "udev",
                hardware_cursor = assigned,
                "cursor plane assignment changed"
            );
        }
    }

    fn rescan_connectors(&mut self, data: &mut CalloopData) {
        let events = match self.scanner.scan_connectors(self.manager.device()) {
            Ok(events) => events,
            Err(error) => {
                warn!(%error, "failed to rescan DRM connectors");
                return;
            }
        };
        for event in events {
            match event {
                DrmScanEvent::Disconnected { connector, crtc } => {
                    if classify_hotplug(self.crtc, crtc, HotplugEventKind::Disconnected)
                        == HotplugAction::Disconnect
                    {
                        info!(connector = %connector.interface().as_str(), "DRM output disconnected");
                        self.disconnect_output(data);
                    }
                }
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    if classify_hotplug(self.crtc, Some(crtc), HotplugEventKind::Connected)
                        == HotplugAction::Connect
                    {
                        self.connect_output(data, &connector, crtc);
                    } else {
                        debug!(connector = %connector.interface().as_str(), "ignoring additional DRM connector in single-Output mode");
                    }
                }
                DrmScanEvent::Changed {
                    connector,
                    crtc: Some(crtc),
                } if classify_hotplug(self.crtc, Some(crtc), HotplugEventKind::Changed)
                    == HotplugAction::Reconnect =>
                {
                    info!(connector = %connector.interface().as_str(), "DRM output modes changed; recreating scanout");
                    self.disconnect_output(data);
                    self.connect_output(data, &connector, crtc);
                }
                _ => {}
            }
        }
    }

    fn disconnect_output(&mut self, data: &mut CalloopData) {
        self.frame_pending = false;
        self.repaint_scheduled = false;
        self.crtc = None;
        self.scanout.take();
        if let Some(output) = self.output.take() {
            data.state.space.unmap_output(&output);
            data.state.space.refresh();
        }
        if let Some(global) = self.output_global.take() {
            data.display_handle
                .remove_global::<crate::state::MioState>(global);
        }
    }

    fn connect_output(
        &mut self,
        data: &mut CalloopData,
        connector: &connector::Info,
        crtc: crtc::Handle,
    ) {
        let Some((mode_index, drm_mode)) = preferred_connector_mode(connector) else {
            warn!(connector = %connector.interface().as_str(), "connected DRM connector exposes no modes");
            return;
        };
        let scale = data.state.config.config().output.scale;
        let (output, _) = match create_output(connector, scale) {
            Ok(output) => output,
            Err(error) => {
                error!(%error, "failed to create connected DRM output");
                return;
            }
        };
        let initialized = self
            .manager
            .lock()
            .initialize_output::<_, DirectSpaceElements>(
                crtc,
                connector.modes()[mode_index],
                &[connector.handle()],
                &output,
                None,
                &mut self.renderer,
                &DrmOutputRenderElements::default(),
            );
        let scanout = match initialized {
            Ok(scanout) => scanout,
            Err(error) => {
                error!(%error, "failed to initialize reconnected DRM output");
                return;
            }
        };
        self.scanout = Some(scanout);
        self.crtc = Some(crtc);
        self.output = Some(output.clone());
        self.output_global =
            Some(output.create_global::<crate::state::MioState>(&data.display_handle));
        data.state.space.map_output(&output, (0, 0));
        let Some(output_id) = data.state.world.output_cameras().next().map(|(id, _)| id) else {
            error!("Mio Core has no Camera for the DRM Output");
            self.disconnect_output(data);
            return;
        };
        data.state.register_output(output_id, output.clone());
        smithay::desktop::layer_map_for_output(&output).arrange();
        let mode = Mode::from(drm_mode);
        data.state.apply_output_scale();
        data.state.launch_pending_startup_commands_if_output_ready();
        self.repaint_scheduled = true;
        info!(output = %output.name(), width = mode.size.w, height = mode.size.h, "DRM output connected");
        self.render(&mut data.state);
    }

    #[allow(clippy::too_many_lines)]
    fn render(&mut self, state: &mut crate::state::MioState) {
        if self.frame_pending || !self.session.is_active() || self.scanout.is_none() {
            return;
        }
        let render_started = std::time::Instant::now();
        self.diagnostics_renders += 1;
        let now = std::time::Instant::now();
        state.flush_pending_pointer_click(now);
        state.poll_xdg_clients(now);
        let mut animations_active = state.advance_animations(now);
        for (dmabuf, notifier) in std::mem::take(&mut state.pending_dmabuf_imports) {
            if self.renderer.import_dmabuf(&dmabuf, None).is_ok() {
                let _ = notifier.successful::<crate::state::MioState>();
            } else {
                notifier.failed();
            }
        }
        if direct_scene(state.session_locked) == DirectScene::SessionLock {
            self.render_session_lock(state, render_started);
            return;
        }
        animations_active |= state.sync_focus_indicator_contexts(self.focus_glow_program.as_ref());
        let appearance = state.config.config().appearance;
        let effects = state.config.config().effects;
        let cursor_wake_enabled = state.cursor_wake_override.unwrap_or(effects.cursor_wake);
        let cursor_wake = if cursor_wake_enabled && self.cursor_wake_programs.is_some() {
            state.cursor_wake.active_wake(
                now,
                Duration::from_millis(u64::from(effects.cursor_wake_duration)),
            )
        } else {
            *self.cursor_wake_frame.borrow_mut() = CursorWakeFrame::default();
            None
        };
        let blur_options = BlurOptions {
            passes: effects.blur_passes,
            offset: effects.blur_offset,
        };
        for window in state.space.elements() {
            window.set_blur_context(self.blur_programs.as_ref(), blur_options);
            window.set_rounding_context(
                self.rounding_program.as_ref(),
                appearance.corner_radius,
                effects.window_transition,
            );
            #[allow(clippy::cast_precision_loss)]
            window.set_window_border_context(
                self.window_border_program.as_ref(),
                (appearance.window_border_width > 0 && appearance.window_border_color[3] > 0.0)
                    .then_some(WindowBorderOptions {
                        width: appearance.window_border_width as f32,
                        color: appearance.window_border_color,
                        corner_radius: appearance.corner_radius as f32,
                    }),
            );
            window.set_shadow_context(
                self.shadow_program.as_ref(),
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
        if let Err(error) =
            capture_window_snapshots(state, &mut self.renderer, &mut self.window_snapshots)
        {
            warn!(%error, "failed to update direct-backend Window snapshots");
        }
        for transition in state.destroyed_window_transitions.drain(..) {
            if let Some(snapshot) = self.window_snapshots.remove(&transition.id) {
                self.closing_visuals.push(ClosingVisual {
                    snapshot,
                    started: now,
                    duration: transition.duration,
                    effect: transition.effect,
                });
            }
        }
        self.window_snapshots.retain(|id, _| {
            state
                .managed_windows
                .iter()
                .any(|managed| managed.id == *id)
        });
        self.closing_visuals
            .retain(|visual| now.duration_since(visual.started) < visual.duration);
        let Some(output) = self.output.clone() else {
            return;
        };
        let upper_layer_element_count =
            direct_upper_layer_element_count(&mut self.renderer, &output);
        let mut space_elements = match space_render_elements::<_, RenderWindow, _>(
            &mut self.renderer,
            [&state.space],
            &output,
            1.0,
        ) {
            Ok(elements) => elements,
            Err(error) => {
                error!(%error, "failed to collect DRM render elements");
                return;
            }
        };
        let mut elements: Vec<DirectRenderElement> = Vec::new();
        let output_scale = output.current_scale().fractional_scale();
        self.append_cursor_elements(state, now, output_scale, &mut elements);
        let cursor_element_count = elements.len();
        if let (Some(config_error), Some(mode)) =
            (state.config_error.as_deref(), output.current_mode())
        {
            elements.extend(self.config_error_overlay.elements(mode.size, config_error));
        }
        let layer_split_valid = upper_layer_element_count <= space_elements.len();
        let remaining_space_elements = if layer_split_valid {
            space_elements.split_off(upper_layer_element_count)
        } else {
            error!(
                upper_layer_element_count,
                scene_element_count = space_elements.len(),
                "upper layer prefix exceeds the Smithay scene; skipping cursor wake"
            );
            Vec::new()
        };
        let cursor_wake_rendered = layer_split_valid && cursor_wake.is_some();
        if cursor_wake_rendered {
            elements.extend(
                space_elements
                    .into_iter()
                    .map(NonOccludingElement::new)
                    .map(DirectRenderElement::from),
            );
        } else {
            elements.extend(space_elements.into_iter().map(DirectRenderElement::from));
        }
        if let (Some(programs), Some(wake), Some(mode)) = (
            self.cursor_wake_programs.clone(),
            layer_split_valid.then_some(cursor_wake).flatten(),
            output.current_mode(),
        ) {
            self.cursor_wake_commit = self.cursor_wake_commit.wrapping_add(1);
            elements.push(
                CursorWakeElement::new(
                    self.cursor_wake_id.clone(),
                    smithay::backend::renderer::utils::CommitCounter::from(self.cursor_wake_commit),
                    Rectangle::<i32, Logical>::from_size(
                        mode.size.to_f64().to_logical(output_scale).to_i32_round(),
                    ),
                    programs,
                    wake,
                    effects.cursor_wake_width,
                    self.cursor_size as f32,
                    effects.cursor_wake_strength,
                    output_scale,
                    Rc::clone(&self.cursor_wake_frame),
                )
                .into(),
            );
        }
        elements.extend(closing_visual_elements(
            &self.renderer,
            &self.closing_visuals,
            self.rounding_program.as_ref(),
            appearance.corner_radius,
            now,
        ));
        elements.extend(
            remaining_space_elements
                .into_iter()
                .map(DirectRenderElement::from),
        );
        if !state.pending_screencopies.is_empty() || state.has_pending_image_copy_captures() {
            if let Some(mode) = output.current_mode() {
                if let Err(error) = fulfill_direct_screencopies(
                    state,
                    &mut self.renderer,
                    &elements[cursor_element_count..],
                    mode.size,
                    appearance.background_color,
                ) {
                    warn!(%error, "failed to render direct-backend screencopy");
                }
            }
        }
        let cursor_plane_assignment = {
            let Some(scanout) = self.scanout.as_mut() else {
                return;
            };
            match scanout.render_frame(
                &mut self.renderer,
                &elements,
                state.config.config().appearance.background_color,
                smithay::backend::drm::compositor::FrameFlags::DEFAULT,
            ) {
                Ok(frame) if !frame.is_empty => {
                    let cursor_plane_assigned = frame.cursor_element.is_some();
                    drop(frame);
                    match scanout.queue_frame(()) {
                        Ok(()) => {
                            self.frame_pending = true;
                            self.diagnostics_submits += 1;
                        }
                        Err(error) => error!(%error, "failed to queue DRM frame"),
                    }
                    Some(cursor_plane_assigned)
                }
                Ok(frame) => {
                    let cursor_plane_assigned = frame.cursor_element.is_some();
                    drop(frame);
                    self.repaint_scheduled = true;
                    Some(cursor_plane_assigned)
                }
                Err(error) => {
                    error!(%error, "failed to render DRM frame");
                    None
                }
            }
        };
        if let Some(assigned) = cursor_plane_assignment {
            self.record_cursor_plane_assignment(assigned);
        }
        if animations_active || !self.closing_visuals.is_empty() || cursor_wake_rendered {
            self.repaint_scheduled = true;
        }
        let cpu = render_started.elapsed();
        self.diagnostics_cpu += cpu;
        self.diagnostics_cpu_max = self.diagnostics_cpu_max.max(cpu);
        let elapsed = self.diagnostics_started.elapsed();
        if elapsed >= std::time::Duration::from_secs(5) {
            let renders = self.diagnostics_renders.max(1);
            debug!(
                target: "mio_compositor::diagnostics",
                backend = "udev",
                render_attempt_fps = self.diagnostics_renders as f64 / elapsed.as_secs_f64(),
                submitted_fps = self.diagnostics_submits as f64 / elapsed.as_secs_f64(),
                cpu_frame_avg_ms = self.diagnostics_cpu.as_secs_f64() * 1000.0 / renders as f64,
                cpu_frame_max_ms = self.diagnostics_cpu_max.as_secs_f64() * 1000.0,
                "frame diagnostics"
            );
            self.diagnostics_started = std::time::Instant::now();
            self.diagnostics_renders = 0;
            self.diagnostics_submits = 0;
            self.diagnostics_cpu = std::time::Duration::ZERO;
            self.diagnostics_cpu_max = std::time::Duration::ZERO;
        }
    }

    fn append_cursor_elements(
        &mut self,
        state: &crate::state::MioState,
        now: Instant,
        output_scale: f64,
        elements: &mut Vec<DirectRenderElement>,
    ) {
        let pointer = state
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        let pointer_location = pointer.current_location();
        let cursor_status = state.effective_cursor_status();
        match cursor_status {
            CursorImageStatus::Surface(surface) if surface.alive() => {
                self.cursor_animation = None;
                let hotspot = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .map_or_else(Default::default, |attributes| {
                            attributes.lock().unwrap().hotspot
                        })
                });
                let cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(
                        &mut self.renderer,
                        &surface,
                        (pointer_location - hotspot.to_f64())
                            .to_physical(output_scale)
                            .to_i32_round(),
                        Scale::from(output_scale),
                        1.0,
                        Kind::Cursor,
                    );
                elements.extend(cursor_elements.into_iter().map(DirectRenderElement::from));
            }
            CursorImageStatus::Hidden | CursorImageStatus::Surface(_) => {
                self.cursor_animation = None;
            }
            CursorImageStatus::Named(icon) => {
                let cursor_elapsed = match self.cursor_animation {
                    Some((current, started)) if current == icon => {
                        now.saturating_duration_since(started)
                    }
                    _ => {
                        self.cursor_animation = Some((icon, now));
                        Duration::ZERO
                    }
                };
                if !self.cursors.contains_key(&icon) {
                    let theme = xcursor::CursorTheme::load(&self.cursor_theme_name);
                    if let Some(cursor) = load_theme_cursor(&theme, icon.name(), self.cursor_size) {
                        self.cursors.insert(icon, cursor);
                    }
                }
                let cursor = self
                    .cursors
                    .get(&icon)
                    .or_else(|| self.cursors.get(&CursorIcon::Default))
                    .expect("the direct backend always has a default cursor");
                if cursor.animated() {
                    self.repaint_scheduled = true;
                }
                let frame = cursor.frame(cursor_elapsed);
                let (cursor_source, cursor_size, hotspot) =
                    software_cursor_geometry(frame, self.cursor_size);
                let location =
                    software_cursor_physical_location(pointer_location, hotspot, output_scale);
                match MemoryRenderBufferRenderElement::from_buffer(
                    &mut self.renderer,
                    location.to_f64(),
                    &frame.buffer,
                    None,
                    Some(cursor_source),
                    Some(cursor_size),
                    Kind::Cursor,
                ) {
                    Ok(cursor) => elements.push(cursor.into()),
                    Err(error) => warn!(%error, "failed to import software cursor"),
                }
            }
        }
        if let Some(icon) = &state.dnd_icon {
            let icon_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    &mut self.renderer,
                    &icon.surface,
                    (pointer_location + icon.offset.to_f64())
                        .to_physical(output_scale)
                        .to_i32_round(),
                    Scale::from(output_scale),
                    1.0,
                    Kind::Unspecified,
                );
            elements.extend(icon_elements.into_iter().map(DirectRenderElement::from));
        }
    }

    fn render_session_lock(&mut self, state: &mut crate::state::MioState, render_started: Instant) {
        let Some(output) = self.output.clone() else {
            return;
        };
        let mut elements = Vec::<DirectRenderElement>::new();
        for (surface_output, surface) in &state.session_lock_surfaces {
            if surface_output != &output {
                continue;
            }
            let output_scale = output.current_scale().fractional_scale();
            let lock_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    &mut self.renderer,
                    surface.wl_surface(),
                    (0, 0),
                    Scale::from(output_scale),
                    1.0,
                    Kind::Unspecified,
                );
            elements.extend(lock_elements.into_iter().map(DirectRenderElement::from));
        }
        let output_scale = output.current_scale().fractional_scale();
        self.append_cursor_elements(state, Instant::now(), output_scale, &mut elements);
        let Some(scanout) = self.scanout.as_mut() else {
            return;
        };
        match scanout.render_frame(
            &mut self.renderer,
            &elements,
            [0.0, 0.0, 0.0, 1.0],
            smithay::backend::drm::compositor::FrameFlags::DEFAULT,
        ) {
            Ok(frame) if !frame.is_empty => {
                drop(frame);
                match scanout.queue_frame(()) {
                    Ok(()) => {
                        self.frame_pending = true;
                        self.diagnostics_submits += 1;
                        if let Some(confirmation) = state.pending_session_lock.take() {
                            confirmation.lock();
                        }
                    }
                    Err(error) => error!(%error, "failed to queue session-lock DRM frame"),
                }
            }
            Ok(_) => {
                self.repaint_scheduled = true;
            }
            Err(error) => error!(%error, "failed to render DRM session-lock frame"),
        }
        let cpu = render_started.elapsed();
        self.diagnostics_cpu += cpu;
        self.diagnostics_cpu_max = self.diagnostics_cpu_max.max(cpu);
    }

    fn send_frame_callbacks(&self, state: &crate::state::MioState) {
        let Some(output) = self.output.as_ref() else {
            return;
        };
        let refresh = output.current_mode().and_then(|mode| {
            (mode.refresh > 0)
                .then(|| std::time::Duration::from_secs_f64(1000.0 / f64::from(mode.refresh)))
        });
        if state.session_locked {
            for (surface_output, surface) in &state.session_lock_surfaces {
                if surface_output == output {
                    send_frames_surface_tree(
                        surface.wl_surface(),
                        surface_output,
                        state.start_time.elapsed(),
                        refresh,
                        |_, _| Some(surface_output.clone()),
                    );
                }
            }
            return;
        }
        for window in state.space.elements() {
            window.send_frame(output, state.start_time.elapsed(), refresh, |_, _| {
                Some(output.clone())
            });
        }
        for layer in smithay::desktop::layer_map_for_output(output).layers() {
            layer.send_frame(output, state.start_time.elapsed(), refresh, |_, _| {
                Some(output.clone())
            });
        }
        if let smithay::input::pointer::CursorImageStatus::Surface(surface) =
            &state.cursor_image_status
        {
            send_frames_surface_tree(
                surface,
                output,
                state.start_time.elapsed(),
                refresh,
                |_, _| Some(output.clone()),
            );
        }
        if let Some(icon) = &state.dnd_icon {
            send_frames_surface_tree(
                &icon.surface,
                output,
                state.start_time.elapsed(),
                refresh,
                |_, _| Some(output.clone()),
            );
        }
    }
}

fn software_cursor_geometry(
    frame: &SoftwareCursorFrame,
    requested_size: u32,
) -> (
    Rectangle<f64, Logical>,
    Size<i32, Logical>,
    Point<i32, Logical>,
) {
    let scale = f64::from(requested_size) / f64::from(frame.nominal_size.max(1));
    let scale_value = |value: i32| (f64::from(value) * scale).round() as i32;
    (
        Rectangle::from_size(frame.source_size.to_f64()),
        Size::from((
            scale_value(frame.source_size.w).max(1),
            scale_value(frame.source_size.h).max(1),
        )),
        Point::from((scale_value(frame.hotspot.x), scale_value(frame.hotspot.y))),
    )
}

fn software_cursor_physical_location(
    pointer_location: Point<f64, Logical>,
    hotspot: Point<i32, Logical>,
    output_scale: f64,
) -> Point<i32, Physical> {
    (pointer_location - hotspot.to_f64())
        .to_physical(output_scale)
        .to_i32_round()
}

fn config_error_overlay_elements(
    size: smithay::utils::Size<i32, Physical>,
    error: &str,
    ids: &mut Vec<Id>,
    commit: usize,
) -> Vec<DirectRenderElement> {
    use smithay::backend::renderer::{
        element::solid::SolidColorRenderElement, utils::CommitCounter,
    };

    let height = size.h.clamp(1, 72);
    let background = Rectangle::new((0, 0).into(), (size.w, height).into());
    let accent = Rectangle::new((0, 0).into(), (6.min(size.w), height).into());
    let commit = CommitCounter::from(commit);
    let mut rectangles =
        crate::winit::bitmap_text_rects("CONFIG ERROR - USING DEFAULTS", (14, 8), 2)
            .into_iter()
            .chain(crate::winit::bitmap_text_rects(
                "FIX FILE, THEN RELOAD CONFIG",
                (14, 28),
                2,
            ))
            .chain(crate::winit::bitmap_text_rects(
                &crate::winit::config_error_summary(
                    error,
                    usize::try_from((size.w - 28).max(0) / 12).unwrap_or(0),
                ),
                (14, 48),
                2,
            ))
            .filter(|rectangle| rectangle.loc.x < size.w && rectangle.loc.y < height)
            .map(|rectangle| (rectangle, [1.0, 0.96, 0.92, 1.0]))
            .collect::<Vec<_>>();
    rectangles.push((accent, [1.0, 0.72, 0.18, 1.0]));
    rectangles.push((background, [0.45, 0.03, 0.04, 0.96]));
    while ids.len() < rectangles.len() {
        ids.push(Id::new());
    }
    rectangles
        .into_iter()
        .enumerate()
        .map(|(index, (rectangle, color))| {
            SolidColorRenderElement::new(
                ids[index].clone(),
                rectangle,
                commit,
                color,
                Kind::Unspecified,
            )
            .into()
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use std::time::Duration;

    use super::{
        classify_hotplug, closing_visual_progress, direct_scene, software_cursor_geometry,
        software_cursor_physical_location, ConfigErrorOverlayState, DirectScene, Fourcc,
        HotplugAction, HotplugEventKind, MemoryRenderBuffer, SoftwareCursorFrame, Transform,
    };
    use smithay::backend::renderer::element::Element;
    use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

    #[test]
    fn closing_visual_progress_runs_backwards_and_clamps() {
        let duration = Duration::from_millis(400);
        assert_eq!(closing_visual_progress(Duration::ZERO, duration), 1.0);
        assert!(
            (closing_visual_progress(Duration::from_millis(200), duration) - 0.5).abs() < 0.001
        );
        assert_eq!(closing_visual_progress(duration, duration), 0.0);
        assert_eq!(
            closing_visual_progress(Duration::from_secs(1), duration),
            0.0
        );
    }

    #[test]
    fn hotplug_only_reconfigures_the_single_active_output() {
        assert_eq!(
            classify_hotplug(None, Some(7), HotplugEventKind::Connected),
            HotplugAction::Connect
        );
        assert_eq!(
            classify_hotplug(Some(7), Some(7), HotplugEventKind::Disconnected),
            HotplugAction::Disconnect
        );
        assert_eq!(
            classify_hotplug(Some(7), Some(7), HotplugEventKind::Changed),
            HotplugAction::Reconnect
        );
        assert_eq!(
            classify_hotplug(Some(7), Some(9), HotplugEventKind::Connected),
            HotplugAction::Ignore
        );
        assert_eq!(
            classify_hotplug(Some(7), Some(9), HotplugEventKind::Disconnected),
            HotplugAction::Ignore
        );
        assert_eq!(
            classify_hotplug(None::<u32>, None, HotplugEventKind::Connected),
            HotplugAction::Ignore
        );
    }

    #[test]
    fn session_lock_excludes_the_desktop_scene() {
        assert_eq!(direct_scene(false), DirectScene::Desktop);
        assert_eq!(direct_scene(true), DirectScene::SessionLock);
    }

    #[test]
    fn configuration_errors_create_a_direct_backend_overlay() {
        let mut overlay = ConfigErrorOverlayState::default();
        let elements = overlay.elements(
            smithay::utils::Size::from((1920, 1080)),
            "unsupported key Space at line 130",
        );
        assert!(elements.len() > 2);
    }

    #[test]
    fn unchanged_configuration_error_keeps_overlay_element_ids() {
        let mut overlay = ConfigErrorOverlayState::default();
        let size = smithay::utils::Size::from((1920, 1080));
        let first = overlay.elements(size, "unsupported key Space at line 130");
        let second = overlay.elements(size, "unsupported key Space at line 130");
        let first_ids = first.iter().map(Element::id).collect::<Vec<_>>();
        let second_ids = second.iter().map(Element::id).collect::<Vec<_>>();
        assert_eq!(first_ids, second_ids);
        assert_eq!(overlay.commit, 1);
    }

    #[test]
    fn changed_configuration_error_advances_overlay_commit() {
        let mut overlay = ConfigErrorOverlayState::default();
        let size = smithay::utils::Size::from((1920, 1080));
        overlay.elements(size, "first error");
        overlay.elements(size, "second error");
        assert_eq!(overlay.commit, 2);
    }

    #[test]
    fn software_cursor_tip_tracks_fractionally_scaled_pointer() {
        let pointer = Point::<f64, Logical>::from((1535.2, 863.2));
        let hotspot = Point::<i32, Logical>::from((4, 7));
        let location = software_cursor_physical_location(pointer, hotspot, 1.25);
        assert_eq!(location, Point::from((1914, 1070)));
        assert_eq!(
            location + Point::<i32, Physical>::from((5, 9)),
            Point::from((1919, 1079))
        );
    }

    #[test]
    fn software_cursor_geometry_honors_requested_size() {
        let frame = SoftwareCursorFrame {
            buffer: MemoryRenderBuffer::from_slice(
                &[0; 36 * 36 * 4],
                Fourcc::Argb8888,
                (36, 36),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: Point::from((6, 9)),
            source_size: Size::from((36, 36)),
            nominal_size: 36,
            delay_ms: 1,
        };

        assert_eq!(
            software_cursor_geometry(&frame, 24),
            (
                Rectangle::from_size(Size::from((36.0, 36.0))),
                Size::from((24, 24)),
                Point::from((4, 6))
            )
        );
    }
}
