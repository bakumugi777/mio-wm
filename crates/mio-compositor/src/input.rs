use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use mio_core::{
    Action, ActionOutcome, Direction, GridSize, Presentation, WindowProperty, WindowPropertyKind,
};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        TouchEvent,
    },
    desktop::layer_map_for_output,
    input::{
        keyboard::{FilterResult, Keysym},
        pointer::{AxisFrame, ButtonEvent, CursorIcon, MotionEvent, RelativeMotionEvent},
        touch::{DownEvent, MotionEvent as TouchMotionEvent, UpEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::with_states,
        input_method::InputMethodSeat,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
        pointer_constraints::{with_pointer_constraint, PointerConstraint},
        shell::wlr_layer::{KeyboardInteractivity, Layer, LayerSurfaceCachedState},
    },
};
use tracing::{debug, info, warn};
use xkbcommon::xkb;

use crate::{
    config::{ConfigAction, Key, KeyChord, MouseButton},
    state::{
        MioState, PendingCameraDrag, PendingCloseClick, PendingFloatingChord, PendingPointerClick,
        PendingWindowDrag, PendingWindowResize, ResizeEdges, ResizeFollowerPreview,
    },
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const POINTER_BUTTON_LEFT: u8 = 1;
const POINTER_BUTTON_RIGHT: u8 = 2;
const POINTER_BUTTON_MIDDLE: u8 = 4;
const CAMERA_DRAG_THRESHOLD: f64 = 6.0;
const EDGE_COMMAND_THRESHOLD: f64 = 24.0;
const CAMERA_WHEEL_ZOOM_MIN: f64 = 0.1;
const CAMERA_WHEEL_ZOOM_SENSITIVITY: f64 = 0.01;
const WINDOW_RESIZE_EDGE: f64 = 8.0;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(275);
const DOUBLE_CLICK_DISTANCE: f64 = 6.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackendInputAction {
    ChangeVt(i32),
}

enum KeyboardAction {
    Config(ConfigAction),
    ChangeVt(i32),
}

impl MioState {
    pub fn process_input_event<I: InputBackend>(
        &mut self,
        event: InputEvent<I>,
    ) -> Option<BackendInputAction> {
        match event {
            InputEvent::Keyboard { event, .. } => return self.process_keyboard::<I>(&event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.process_pointer_motion_absolute::<I>(&event);
            }
            InputEvent::PointerMotion { event, .. } => {
                self.process_pointer_motion_relative::<I>(&event);
            }
            InputEvent::PointerButton { event, .. } => self.process_pointer_button(&event),
            InputEvent::PointerAxis { event, .. } => self.process_pointer_axis(&event),
            InputEvent::TouchDown { event } => self.process_touch_down::<I>(&event),
            InputEvent::TouchUp { event } => self.process_touch_up::<I>(&event),
            InputEvent::TouchMotion { event } => self.process_touch_motion::<I>(&event),
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.frame(self);
                }
            }
            InputEvent::TouchCancel { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.cancel(self);
                }
            }
            _ => {}
        }
        None
    }

    fn process_pointer_motion_relative<B: InputBackend>(&mut self, event: &B::PointerMotionEvent) {
        let Some(output) = self.space.outputs().next() else {
            return;
        };
        let Some(output_geometry) = self.space.output_geometry(output) else {
            return;
        };
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        let current = pointer.current_location();
        let requested = current + event.delta();
        let minimum = output_geometry.loc.to_f64();
        let maximum = (output_geometry.loc + output_geometry.size).to_f64();
        let position = (
            requested.x.clamp(minimum.x, maximum.x - f64::EPSILON),
            requested.y.clamp(minimum.y, maximum.y - f64::EPSILON),
        )
            .into();
        self.process_pointer_position(
            position,
            event.delta(),
            event.delta_unaccel(),
            event.time_msec(),
            event.time(),
        );
    }

    fn process_pointer_motion_absolute<B: InputBackend>(
        &mut self,
        event: &B::PointerMotionAbsoluteEvent,
    ) {
        let Some(output) = self.space.outputs().next() else {
            return;
        };
        let Some(output_geometry) = self.space.output_geometry(output) else {
            return;
        };
        let requested_position =
            event.position_transformed(output_geometry.size) + output_geometry.loc.to_f64();
        let delta = self
            .last_host_pointer_position
            .replace(requested_position)
            .map_or_else(
                || (0.0, 0.0).into(),
                |previous| requested_position - previous,
            );
        self.process_pointer_position(
            requested_position,
            delta,
            delta,
            event.time_msec(),
            event.time(),
        );
    }

    #[allow(clippy::too_many_lines)]
    fn process_pointer_position(
        &mut self,
        requested_position: smithay::utils::Point<f64, smithay::utils::Logical>,
        delta: smithay::utils::Point<f64, smithay::utils::Logical>,
        delta_unaccel: smithay::utils::Point<f64, smithay::utils::Logical>,
        time_msec: u32,
        time: u64,
    ) {
        if self.pending_pointer_click.is_some_and(|click| {
            let delta = requested_position - click.position;
            delta.x.hypot(delta.y) > DOUBLE_CLICK_DISTANCE
        }) {
            self.replay_pending_pointer_click();
        }
        if self.pending_close_click.is_some_and(|click| {
            let delta = requested_position - click.position;
            delta.x.hypot(delta.y) > DOUBLE_CLICK_DISTANCE
        }) {
            self.pending_close_click = None;
        }
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        let old_position = pointer.current_location();
        if let Some(drag) = &mut self.pending_window_drag {
            drag.current = requested_position;
        }
        if let Some(resize) = &mut self.pending_window_resize {
            resize.current = requested_position;
        }
        if self.pending_window_resize.is_some() {
            self.update_window_resize();
        }
        let mut camera_dragging = false;
        if self.pending_window_drag.is_none() {
            if let Some(drag) = &mut self.pending_camera_drag {
                let from_start = requested_position - drag.start;
                if !drag.dragging && camera_drag_started(from_start.x, from_start.y) {
                    drag.dragging = true;
                }
                camera_dragging = drag.dragging;
            }
        }
        if camera_dragging {
            self.pan_camera_to_pointer_position(requested_position);
        }
        let under = self.surface_under(old_position);
        let requested_under = self.surface_under(requested_position);
        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta,
                delta_unaccel,
                utime: time,
            },
        );

        let mut position = requested_position;
        let mut constrained = false;
        if !camera_dragging {
            if let Some((surface, origin)) = &under {
                let same_surface = requested_under
                    .as_ref()
                    .is_some_and(|(candidate, _)| candidate == surface);
                with_pointer_constraint(surface, &pointer, |constraint| {
                    let Some(constraint) = constraint.filter(|constraint| constraint.is_active())
                    else {
                        return;
                    };
                    constrained = match &*constraint {
                        PointerConstraint::Locked(_) => true,
                        PointerConstraint::Confined(_) => {
                            let inside_region = constraint.region().is_none_or(|region| {
                                region.contains((requested_position - *origin).to_i32_round())
                            });
                            !same_surface || !inside_region
                        }
                    };
                });
            }
        }
        if constrained {
            position = old_position;
        }
        let effects = self.config.config().effects;
        let cursor_wake_enabled = self.cursor_wake_override.unwrap_or(effects.cursor_wake);
        self.cursor_wake.record_motion(
            position,
            Instant::now(),
            cursor_wake_enabled,
            effects.cursor_wake_threshold,
        );
        let under = self.surface_under(position);
        pointer.motion(
            self,
            under.clone(),
            &MotionEvent {
                location: position,
                serial: SERIAL_COUNTER.next_serial(),
                time: time_msec,
            },
        );
        pointer.frame(self);

        self.update_resize_cursor(position);

        if let Some((surface, origin)) = under {
            with_pointer_constraint(&surface, &pointer, |constraint| {
                if let Some(constraint) = constraint.filter(|constraint| !constraint.is_active()) {
                    let local = (position - origin).to_i32_round();
                    if constraint
                        .region()
                        .is_none_or(|region| region.contains(local))
                    {
                        constraint.activate();
                    }
                }
            });
        }
    }

    fn pan_camera_to_pointer_position(
        &mut self,
        pointer_position: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        let Some(drag) = self.pending_camera_drag else {
            return;
        };
        let output_id = self.world.active_output();
        let Some(output) = self.outputs.get(&output_id) else {
            return;
        };
        let Some(output_geometry) = self.space.output_geometry(output) else {
            return;
        };
        let camera = *self.world.camera();
        let (from_start_x, from_start_y) = pointer_delta_to_camera_pan(
            pointer_position.x - drag.start.x,
            pointer_position.y - drag.start.y,
            (output_geometry.size.w, output_geometry.size.h),
            camera.viewport_size(),
            drag.start_zoom,
        );
        let target_x = drag.start_camera_x + from_start_x;
        let target_y = drag.start_camera_y + from_start_y;
        let delta_x = target_x - camera.position().x;
        let delta_y = target_y - camera.position().y;
        match self.world.apply(Action::CameraPan { delta_x, delta_y }) {
            Ok(_) => {
                // Direct manipulation follows the pointer instead of chasing it
                // through the ordinary keyboard/camera interpolation.
                self.reset_all_window_presentations();
                self.sync_layout(false);
            }
            Err(error) => debug!(%error, "mouse Camera pan rejected"),
        }
    }

    #[allow(clippy::cast_possible_wrap)] // VT keysyms are validated to the positive 1..=12 range.
    fn process_keyboard<B: InputBackend>(
        &mut self,
        event: &B::KeyboardKeyEvent,
    ) -> Option<BackendInputAction> {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        let key_state = event.state();
        if self.session_locked {
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.input::<(), _>(
                    self,
                    event.key_code(),
                    key_state,
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            return None;
        }
        if let (Some(surface), Some(keyboard)) = (
            self.exclusive_keyboard_layer_surface(),
            self.seat.get_keyboard(),
        ) {
            keyboard.set_focus(self, Some(surface), serial);
            keyboard.input::<(), _>(
                self,
                event.key_code(),
                key_state,
                serial,
                time,
                |_, _, _| FilterResult::Forward,
            );
            return None;
        }
        let shortcuts_inhibited = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|surface| self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface))
            .is_some_and(|inhibitor| inhibitor.is_active());
        if shortcuts_inhibited {
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.input::<(), _>(
                    self,
                    event.key_code(),
                    key_state,
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            return None;
        }
        let bindings = self.config.config().bindings.clone();
        if let Some(keyboard) = self.seat.get_keyboard() {
            let shortcut = keyboard.input::<KeyboardAction, _>(
                self,
                event.key_code(),
                key_state,
                serial,
                time,
                |_, modifiers, handle| {
                    if key_state != KeyState::Pressed {
                        return FilterResult::Forward;
                    }
                    let keysym = handle.modified_sym();
                    if (xkb::keysyms::KEY_XF86Switch_VT_1..=xkb::keysyms::KEY_XF86Switch_VT_12)
                        .contains(&keysym.raw())
                    {
                        let vt = (keysym.raw() - xkb::keysyms::KEY_XF86Switch_VT_1 + 1) as i32;
                        return FilterResult::Intercept(KeyboardAction::ChangeVt(vt));
                    }
                    let Some(key) = key_from_keysym(keysym) else {
                        return FilterResult::Forward;
                    };
                    let chord = KeyChord {
                        ctrl: modifiers.ctrl,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                        logo: modifiers.logo,
                        key,
                    };
                    let shortcut = bindings
                        .iter()
                        .find(|binding| binding.chord == chord)
                        .map(|binding| binding.action);
                    shortcut.map_or(FilterResult::Forward, |action| {
                        FilterResult::Intercept(KeyboardAction::Config(action))
                    })
                },
            );
            match shortcut {
                Some(KeyboardAction::Config(shortcut)) => self.process_shortcut(shortcut, serial),
                Some(KeyboardAction::ChangeVt(vt)) => {
                    return Some(BackendInputAction::ChangeVt(vt));
                }
                None => {}
            }
        }
        None
    }

    fn touch_position<B, E>(
        &self,
        event: &E,
    ) -> Option<smithay::utils::Point<f64, smithay::utils::Logical>>
    where
        B: InputBackend,
        E: AbsolutePositionEvent<B>,
    {
        let output = self.space.outputs().next()?;
        let geometry = self.space.output_geometry(output)?;
        Some(event.position_transformed(geometry.size) + geometry.loc.to_f64())
    }

    fn process_touch_down<B: InputBackend>(&mut self, event: &B::TouchDownEvent) {
        let Some(position) = self.touch_position::<B, _>(event) else {
            return;
        };
        let under = self.surface_under(position);
        let serial = SERIAL_COUNTER.next_serial();
        if let Some((surface, _)) = &under {
            if let Some(id) = self.window_id_for_surface(surface) {
                self.activate_window(id, serial);
            } else if self
                .layer_surface_for_surface(surface)
                .is_some_and(|layer| layer.can_receive_keyboard_focus())
            {
                if let Some(keyboard) = self.seat.get_keyboard() {
                    keyboard.set_focus(self, Some(surface.clone()), serial);
                }
            }
        }
        if let Some(touch) = self.seat.get_touch() {
            touch.down(
                self,
                under,
                &DownEvent {
                    slot: event.slot(),
                    location: position,
                    serial,
                    time: event.time_msec(),
                },
            );
        }
    }

    fn process_touch_up<B: InputBackend>(&mut self, event: &B::TouchUpEvent) {
        if let Some(touch) = self.seat.get_touch() {
            touch.up(
                self,
                &UpEvent {
                    slot: event.slot(),
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                },
            );
        }
    }

    fn process_touch_motion<B: InputBackend>(&mut self, event: &B::TouchMotionEvent) {
        let Some(position) = self.touch_position::<B, _>(event) else {
            return;
        };
        let under = self.surface_under(position);
        if let Some(touch) = self.seat.get_touch() {
            touch.motion(
                self,
                under,
                &TouchMotionEvent {
                    slot: event.slot(),
                    location: position,
                    time: event.time_msec(),
                },
            );
        }
    }

    fn exclusive_keyboard_layer_surface(&self) -> Option<WlSurface> {
        self.layer_shell_state
            .layer_surfaces()
            .rev()
            .find_map(|surface| {
                let state = with_states(surface.wl_surface(), |states| {
                    *states
                        .cached_state
                        .get::<LayerSurfaceCachedState>()
                        .current()
                });
                if state.keyboard_interactivity != KeyboardInteractivity::Exclusive
                    || !matches!(state.layer, Layer::Top | Layer::Overlay)
                {
                    return None;
                }
                self.space.outputs().find_map(|output| {
                    layer_map_for_output(output)
                        .layers()
                        .any(|layer| layer.layer_surface() == &surface)
                        .then(|| surface.wl_surface().clone())
                })
            })
    }

    fn process_shortcut(&mut self, shortcut: ConfigAction, serial: smithay::utils::Serial) {
        let focused = self.world.focused();
        match shortcut {
            ConfigAction::Close => {
                let Some(id) = focused else { return };
                if self.world.apply(Action::CloseWindow(id))
                    == Ok(ActionOutcome::CloseRequested(id))
                {
                    self.begin_close_transition(id);
                }
            }
            ConfigAction::Camera(direction) => {
                if self.world.apply(Action::CameraStep(direction)).is_ok() {
                    self.sync_layout(false);
                }
            }
            ConfigAction::CameraNudge(direction) => {
                if self.world.apply(Action::CameraNudge(direction)).is_ok() {
                    self.sync_layout(false);
                }
            }
            ConfigAction::CameraZoom(zoom) => {
                if self.world.apply(Action::CameraZoom(zoom)).is_ok() {
                    self.sync_layout(false);
                }
            }
            ConfigAction::CycleOutput => {
                if self.world.apply(Action::CycleOutput).is_ok() {
                    self.sync_layout(false);
                }
            }
            ConfigAction::ToggleFloating => {
                let Some(id) = focused else { return };
                match self.world.apply(Action::ToggleFloating(id)) {
                    Ok(_) => {
                        if let Some(window) = self.world.window(id) {
                            info!(window = id.get(), state = ?window.grid_constraint(), "grid constraint changed");
                        }
                        self.sync_layout(false);
                    }
                    Err(error) => debug!(%error, "floating toggle rejected"),
                }
            }
            ConfigAction::Focus(direction) => {
                if let Ok(ActionOutcome::FocusChanged(Some(id))) =
                    self.world.apply(Action::Focus(direction))
                {
                    self.activate_window_after_focus_change(id, serial);
                }
            }
            ConfigAction::ToggleFullscreen => self.toggle_focused_presentation(true),
            ConfigAction::ToggleMaximized => self.toggle_focused_presentation(false),
            ConfigAction::ToggleWindowSize => {
                if let Some(id) = focused {
                    self.toggle_window_size(id);
                }
            }
            ConfigAction::ToggleOpacity => self.toggle_focused_opacity(),
            ConfigAction::ToggleBlur => self.toggle_focused_blur(),
            ConfigAction::ToggleCursorWake => self.toggle_cursor_wake(),
            ConfigAction::ClearOpacity => self.clear_focused_opacity(),
            ConfigAction::ToggleOverview => {
                self.toggle_overview();
            }
            ConfigAction::SelectOverview => {
                self.select_overview_window();
            }
            ConfigAction::Move(direction) => {
                self.move_focused_window(direction);
            }
            ConfigAction::Resize(direction) => self.resize_focused_window(direction),
            ConfigAction::PlaceNext(direction) => {
                let _ = self.world.apply(Action::SetNextPlacement(direction));
                info!(?direction, "next Window placement direction changed");
            }
            ConfigAction::ReloadConfig => match self.config.reload() {
                Ok(()) => {
                    self.config_error = None;
                    let viewport = self.config.config().viewport;
                    if let Err(error) = self.world.resize_camera_viewport(viewport) {
                        warn!(%error, "new viewport rejected after configuration reload");
                    }
                    self.reapply_window_rules();
                    self.sync_layout(true);
                    info!(path = %self.config.display_path(), "configuration reloaded");
                }
                Err(error) => {
                    self.config_error = Some(error.to_string());
                    warn!(
                        %error,
                        path = %self.config.display_path(),
                        "configuration reload rejected; keeping current settings"
                    );
                }
            },
        }
    }

    fn toggle_cursor_wake(&mut self) {
        let configured = self.config.config().effects.cursor_wake;
        let enabled = !self.cursor_wake_override.unwrap_or(configured);
        self.cursor_wake_override = Some(enabled);
        self.cursor_wake.clear();
        info!(enabled, "cursor wake toggled");
    }

    fn toggle_focused_opacity(&mut self) {
        let Some(id) = self.world.focused() else {
            return;
        };
        let Some(opacity) = self
            .world
            .window(id)
            .map(|window| window.effective_properties().opacity)
        else {
            return;
        };
        let opacity = toggled_opacity(opacity, self.config.config().appearance.opacity_toggle);
        if self
            .world
            .apply(Action::SetWindowProperty {
                id,
                property: WindowProperty::Opacity(opacity),
            })
            .is_ok()
        {
            self.sync_layout(false);
        }
    }

    fn toggle_focused_blur(&mut self) {
        let Some(id) = self.world.focused() else {
            return;
        };
        let Some(blur) = self
            .world
            .window(id)
            .map(|window| window.effective_properties().blur)
        else {
            return;
        };
        if self
            .world
            .apply(Action::SetWindowProperty {
                id,
                property: WindowProperty::Blur(!blur),
            })
            .is_ok()
        {
            self.sync_layout(false);
        }
    }

    fn toggle_overview(&mut self) {
        let zoom = self.world.camera().zoom();
        let target = if zoom < 1.0 { 1.0 } else { 0.35 };
        if self.world.apply(Action::CameraZoom(target)).is_ok() {
            self.sync_layout(false);
        }
    }

    fn select_overview_window(&mut self) {
        let Some(id) = self.world.focused() else {
            return;
        };
        let moved = self.world.apply(Action::CameraCenter(id)).is_ok();
        let zoomed = self.world.apply(Action::CameraZoom(1.0)).is_ok();
        if moved && zoomed {
            self.sync_layout(false);
        }
    }

    #[allow(clippy::cast_precision_loss)] // Unit Grid deltas are exact in this practical range.
    fn move_focused_window(&mut self, direction: Direction) {
        let Some(id) = self.world.focused() else {
            return;
        };
        let Some(rect) = self.world.window(id).map(mio_core::Window::rect) else {
            return;
        };
        let delta = direction.delta(1);
        let Ok(origin) = rect.origin().translated(delta.x as f64, delta.y as f64) else {
            return;
        };
        match self
            .world
            .apply(Action::MoveWindowContinuous { id, origin })
        {
            Ok(_) => {
                let _ = self.world.apply(Action::CameraFollow(id));
                self.sync_layout(false);
            }
            Err(error) => debug!(%error, "window move rejected"),
        }
    }

    fn resize_focused_window(&mut self, direction: Direction) {
        let Some(id) = self.world.focused() else {
            return;
        };
        let Some(rect) = self.world.window(id).map(mio_core::Window::rect) else {
            return;
        };
        let (width, height) = match direction {
            Direction::Left => (rect.width().saturating_sub(1), rect.height()),
            Direction::Right => (rect.width().saturating_add(1), rect.height()),
            Direction::Up => (rect.width(), rect.height().saturating_sub(1)),
            Direction::Down => (rect.width(), rect.height().saturating_add(1)),
        };
        let Ok(size) = GridSize::new(width, height) else {
            return;
        };
        match self.world.apply(Action::ResizeWindow { id, size }) {
            Ok(_) => {
                self.sync_layout(true);
            }
            Err(error) => debug!(%error, "window resize rejected"),
        }
    }

    fn toggle_focused_presentation(&mut self, fullscreen: bool) {
        let Some(id) = self.world.focused() else {
            return;
        };
        let action = if fullscreen {
            Action::ToggleFullscreen(id)
        } else {
            Action::ToggleMaximized(id)
        };
        if self.world.apply(action).is_ok() {
            self.sync_layout(true);
        }
    }

    fn clear_focused_opacity(&mut self) {
        let Some(id) = self.world.focused() else {
            return;
        };
        if self
            .world
            .apply(Action::ClearWindowProperty {
                id,
                kind: WindowPropertyKind::Opacity,
            })
            .is_ok()
        {
            self.sync_layout(false);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn process_pointer_button<B, E>(&mut self, event: &E)
    where
        B: InputBackend,
        E: PointerButtonEvent<B>,
    {
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        if event.state() == ButtonState::Pressed
            && self.pending_pointer_click.is_some_and(|click| {
                event.button_code() != click.button || Instant::now() > click.deadline || {
                    let delta = pointer.current_location() - click.position;
                    delta.x.hypot(delta.y) > DOUBLE_CLICK_DISTANCE
                }
            })
        {
            self.replay_pending_pointer_click();
        }
        update_pointer_buttons(
            &mut self.pointer_buttons_held,
            event.button_code(),
            event.state(),
        );
        let mouse = self.config.config().mouse;
        if self.closing_pointer_chord {
            if self.pointer_buttons_held == 0 {
                self.closing_pointer_chord = false;
            }
            return;
        }
        let close_buttons = mouse.close_window.map(mouse_button_code);
        if event.button_code() == close_buttons[0] && event.state() == ButtonState::Released {
            if let Some(click) = self.pending_close_click.take() {
                self.complete_pending_window_click(click);
            }
        }
        if !self.session_locked
            && event.button_code() == close_buttons[1]
            && event.state() == ButtonState::Pressed
            && button_held(self.pointer_buttons_held, mouse.close_window[0])
        {
            let position = pointer.current_location();
            let target = self
                .surface_under(position)
                .and_then(|(surface, _)| self.window_id_for_surface(&surface));
            let now = Instant::now();
            if let Some(clicks) = self
                .pending_close_click
                .and_then(|first| next_close_click_count(first, position, target, now))
            {
                if let Some(pending) = &mut self.pending_close_click {
                    pending.clicks = clicks;
                    pending.position = position;
                    pending.deadline = now + DOUBLE_CLICK_INTERVAL;
                }
            } else {
                self.pending_close_click = None;
            }
        }
        if self.process_floating_chord_release(event.button_code(), event.state()) {
            return;
        }
        if event.state() == ButtonState::Released {
            if let Some((direction, button)) = self.pending_edge_placement {
                if event.button_code() == button {
                    self.pending_edge_placement = None;
                    if let Err(error) = self.world.apply(Action::SetNextPlacement(direction)) {
                        debug!(%error, ?direction, "pointer placement direction rejected");
                    } else {
                        info!(
                            ?direction,
                            "next Window placement selected from Output edge"
                        );
                    }
                    return;
                }
            }
        }
        if self.pending_window_resize.is_some()
            && event.button_code() == mouse_button_code(mouse.resize_window)
            && event.state() == ButtonState::Released
        {
            self.finish_window_resize(pointer.current_location());
            return;
        }
        let released_mask = pointer_button_mask(event.button_code());
        if event.state() == ButtonState::Released
            && released_mask != 0
            && self.suppressed_window_drag_releases & released_mask != 0
        {
            self.suppressed_window_drag_releases &= !released_mask;
            return;
        }
        if self.pending_window_drag.is_some()
            && event.state() == ButtonState::Released
            && self
                .pending_window_drag
                .is_some_and(|drag| drag.buttons & released_mask != 0)
        {
            let close_click = self.pending_window_drag.and_then(|drag| {
                let delta = drag.current - drag.start;
                (event.button_code() == close_buttons[1]
                    && button_held(self.pointer_buttons_held, mouse.close_window[0])
                    && delta.x.hypot(delta.y) <= DOUBLE_CLICK_DISTANCE)
                    .then_some(PendingCloseClick {
                        id: drag.id,
                        position: drag.current,
                        deadline: Instant::now() + DOUBLE_CLICK_INTERVAL,
                        clicks: self
                            .pending_close_click
                            .filter(|pending| pending.id == drag.id)
                            .map_or(1, |pending| pending.clicks),
                    })
            });
            let buttons = self.pending_window_drag.map_or(0, |drag| drag.buttons);
            self.finish_window_drag();
            self.pending_close_click = close_click;
            self.suppressed_window_drag_releases = buttons & self.pointer_buttons_held;
            return;
        }
        if event.button_code() == BTN_LEFT
            && event.state() == ButtonState::Released
            && std::mem::take(&mut self.suppressed_focus_click)
        {
            return;
        }
        if !self.session_locked
            && event.button_code() == mouse_button_code(mouse.resize_window)
            && event.state() == ButtonState::Pressed
            && !button_held(self.pointer_buttons_held, mouse.move_window[0])
            && self.begin_window_resize(pointer.current_location())
        {
            return;
        }
        if !self.session_locked
            && event.state() == ButtonState::Pressed
            && ordered_chord_pressed(
                self.pointer_buttons_held,
                event.button_code(),
                mouse.toggle_floating,
            )
            && self.toggle_floating_with_pointer_chord(pointer.current_location())
        {
            return;
        }
        if !self.session_locked
            && event.button_code() == mouse_button_code(mouse.place_next)
            && event.state() == ButtonState::Pressed
            && self.begin_edge_placement(pointer.current_location())
        {
            return;
        }
        if !self.session_locked
            && event.state() == ButtonState::Pressed
            && ordered_chord_pressed(
                self.pointer_buttons_held,
                event.button_code(),
                mouse.move_window,
            )
            && self.begin_window_drag(pointer.current_location())
        {
            return;
        }
        let replay_camera_press = if self.session_locked {
            None
        } else {
            match self.process_camera_drag_button(event, pointer.current_location()) {
                CameraButtonResult::Consumed => return,
                CameraButtonResult::ReplayClick {
                    button,
                    press_time,
                    window,
                } => Some((button, press_time, window)),
                CameraButtonResult::Forward => None,
            }
        };
        let keyboard = self
            .seat
            .get_keyboard()
            .expect("Mio always creates a keyboard");
        let serial = SERIAL_COUNTER.next_serial();

        let input_method = self.seat.input_method();
        let may_change_focus = (event.state() == ButtonState::Pressed
            || replay_camera_press.is_some())
            && !pointer.is_grabbed()
            && (!keyboard.is_grabbed() || input_method.keyboard_grabbed());
        let release_window = may_change_focus
            .then(|| self.surface_under(pointer.current_location()))
            .flatten()
            .and_then(|(surface, _)| self.window_id_for_surface(&surface));
        let target_window = pointer_click_target(
            replay_camera_press.is_some(),
            replay_camera_press.and_then(|(_, _, window)| window),
            release_window,
        );
        let consume_as_focus_click = should_consume_focus_click(
            event.button_code(),
            event.state(),
            target_window,
            self.world.focused(),
        );

        if may_change_focus {
            self.activate_virtual_output_at(pointer.current_location());
            if let Some((surface, _)) = self.surface_under(pointer.current_location()) {
                if let Some(id) = self.window_id_for_surface(&surface) {
                    self.activate_window(id, serial);
                } else if self
                    .layer_surface_for_surface(&surface)
                    .is_some_and(|layer| layer.can_receive_keyboard_focus())
                {
                    keyboard.set_focus(self, Some(surface.clone()), serial);
                }
            } else {
                for window in self.space.elements() {
                    window.set_activated(false);
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_pending_configure();
                    }
                }
                keyboard.set_focus(self, Option::<WlSurface>::None, serial);
            }
        }

        if consume_as_focus_click {
            self.suppressed_focus_click = true;
            return;
        }

        if let Some((button, press_time, _)) = replay_camera_press {
            self.defer_or_complete_reset_click(
                button,
                press_time,
                event.time_msec(),
                pointer.current_location(),
                target_window,
            );
            return;
        }

        pointer.button(
            self,
            &ButtonEvent {
                button: event.button_code(),
                state: event.state(),
                serial,
                time: event.time_msec(),
            },
        );
        pointer.frame(self);
    }

    fn begin_window_drag(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let Some((surface, _)) = self.surface_under(pointer_location) else {
            return false;
        };
        let Some(id) = self.window_id_for_surface(&surface) else {
            return false;
        };
        let Some(origin) = self.world.window(id).map(|window| window.rect().origin()) else {
            return false;
        };
        self.pending_camera_drag = None;
        let buttons = mouse_chord_mask(self.config.config().mouse.move_window);
        self.pending_window_drag = Some(PendingWindowDrag {
            id,
            buttons,
            origin,
            start: pointer_location,
            current: pointer_location,
        });
        if let Some(window) = self
            .managed_window(id)
            .map(|managed| managed.window.clone())
        {
            self.space.raise_element(&window, false);
        }
        true
    }

    fn toggle_floating_with_pointer_chord(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let Some((surface, _)) = self.surface_under(pointer_location) else {
            return false;
        };
        let Some(id) = self.window_id_for_surface(&surface) else {
            return false;
        };
        self.pending_camera_drag = None;
        let buttons_held = mouse_chord_mask(self.config.config().mouse.toggle_floating);
        self.pending_floating_chord = Some(PendingFloatingChord { id, buttons_held });
        true
    }

    fn apply_pointer_floating_toggle(&mut self, id: mio_core::WindowId) {
        match self.world.apply(Action::ToggleFloating(id)) {
            Ok(_) => {
                self.activate_window_without_camera(id, SERIAL_COUNTER.next_serial());
                if let Some(window) = self.world.window(id) {
                    info!(window = id.get(), state = ?window.grid_constraint(), "pointer floating toggle");
                }
            }
            Err(error) => {
                debug!(%error, "pointer floating toggle rejected");
                self.sync_layout(false);
            }
        }
    }

    fn process_floating_chord_release(&mut self, button: u32, state: ButtonState) -> bool {
        let Some(mut chord) = self.pending_floating_chord else {
            return false;
        };
        let released = pointer_button_mask(button);
        if state != ButtonState::Released || chord.buttons_held & released == 0 {
            return false;
        }
        let id = chord.id;
        let first_release = chord.buttons_held.count_ones() == 2;
        chord.buttons_held &= !released;
        let finished = chord.buttons_held == 0;
        if finished {
            self.pending_floating_chord = None;
        } else {
            self.pending_floating_chord = Some(chord);
        }
        if first_release {
            self.apply_pointer_floating_toggle(id);
        }
        true
    }

    fn begin_edge_placement(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let direction = self.space.outputs().find_map(|output| {
            let geometry = self.space.output_geometry(output)?;
            output_edge_direction(pointer_location.x, pointer_location.y, geometry)
        });
        self.pending_edge_placement = direction.map(|direction| {
            (
                direction,
                mouse_button_code(self.config.config().mouse.place_next),
            )
        });
        direction.is_some()
    }

    fn close_window_with_pointer_multi_click(&mut self, target: Option<mio_core::WindowId>) {
        self.closing_pointer_chord = true;
        self.pending_close_click = None;
        self.pending_pointer_click = None;
        self.pending_camera_drag = None;
        self.pending_floating_chord = None;
        self.pending_edge_placement = None;
        self.pending_window_drag = None;
        self.pending_window_resize = None;
        self.suppressed_window_drag_releases = 0;
        self.cursor_override = None;
        let Some(id) = target else {
            return;
        };
        if self.world.apply(Action::CloseWindow(id)) == Ok(ActionOutcome::CloseRequested(id)) {
            info!(
                window = id.get(),
                "held-button pointer multi-click requested Window close"
            );
            self.begin_close_transition(id);
        }
    }

    fn defer_or_complete_reset_click(
        &mut self,
        button: u32,
        press_time: u32,
        release_time: u32,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        window: Option<mio_core::WindowId>,
    ) {
        let now = Instant::now();
        let next_clicks = self
            .pending_pointer_click
            .filter(|first| pending_click_matches(*first, button, position, window, now))
            .map(|first| first.clicks.saturating_add(1));
        if let Some(clicks) = next_clicks {
            if clicks >= self.config.config().mouse.reset_window_clicks {
                self.pending_pointer_click = None;
                if let Some(id) = window {
                    self.toggle_window_size(id);
                }
            } else if let Some(pending) = &mut self.pending_pointer_click {
                pending.clicks = clicks;
                pending.release_time = release_time;
                pending.deadline = now + DOUBLE_CLICK_INTERVAL;
                pending.position = position;
            }
            return;
        }
        self.replay_pending_pointer_click();
        if self.config.config().mouse.reset_window_clicks == 1 {
            if let Some(id) = window {
                self.toggle_window_size(id);
            }
            return;
        }
        self.pending_pointer_click = Some(PendingPointerClick {
            button,
            press_time,
            release_time,
            deadline: now + DOUBLE_CLICK_INTERVAL,
            position,
            window,
            clicks: 1,
        });
    }

    pub(crate) fn flush_pending_pointer_click(&mut self, now: Instant) {
        if self
            .pending_pointer_click
            .is_some_and(|click| now >= click.deadline)
        {
            self.replay_pending_pointer_click();
        }
        if self
            .pending_close_click
            .is_some_and(|click| now >= click.deadline)
        {
            if let Some(click) = self.pending_close_click.take() {
                self.complete_pending_window_click(click);
            }
        }
    }

    fn complete_pending_window_click(&mut self, click: PendingCloseClick) {
        let mouse = self.config.config().mouse;
        if click.clicks == mouse.close_window_clicks {
            self.close_window_with_pointer_multi_click(Some(click.id));
        } else if click.clicks == mouse.center_window_clicks {
            self.center_window_with_pointer_click(click.id);
        }
    }

    fn center_window_with_pointer_click(&mut self, id: mio_core::WindowId) {
        if self.world.window(id).is_none() {
            return;
        }
        self.activate_window_without_camera(id, SERIAL_COUNTER.next_serial());
        match self.world.apply(Action::CameraCenter(id)) {
            Ok(_) => {
                self.sync_layout(false);
                info!(
                    window = id.get(),
                    "pointer click centered Window with Camera"
                );
            }
            Err(error) => debug!(%error, "pointer Window centering rejected"),
        }
    }

    fn replay_pending_pointer_click(&mut self) {
        let Some(click) = self.pending_pointer_click.take() else {
            return;
        };
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        for _ in 0..click.clicks {
            pointer.button(
                self,
                &ButtonEvent {
                    button: click.button,
                    state: ButtonState::Pressed,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: click.press_time,
                },
            );
            pointer.button(
                self,
                &ButtonEvent {
                    button: click.button,
                    state: ButtonState::Released,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: click.release_time,
                },
            );
        }
        pointer.frame(self);
    }

    fn toggle_window_size(&mut self, id: mio_core::WindowId) {
        let initial_size = self.config.config().initial_window_size;
        match self
            .world
            .apply(Action::ToggleWindowSize { id, initial_size })
        {
            Ok(_) => {
                self.activate_window_without_camera(id, SERIAL_COUNTER.next_serial());
                if let Err(error) = self.world.apply(Action::CameraFollow(id)) {
                    debug!(%error, "failed to reveal resized Window with Camera");
                }
                self.sync_layout(true);
                info!(
                    window = id.get(),
                    ?initial_size,
                    "toggled Window between half and initial width"
                );
            }
            Err(error) => debug!(%error, "Window size toggle rejected"),
        }
    }

    fn begin_window_resize(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let Some((id, edges)) = self.resize_target_at(pointer_location) else {
            return false;
        };
        let Some(rect) = self.world.window(id).map(mio_core::Window::rect) else {
            return false;
        };
        let Some(screen) = self
            .managed_window(id)
            .and_then(|managed| self.space.element_geometry(&managed.window))
            .map(|geometry| crate::layout::ScreenRect {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
        else {
            return false;
        };
        let mut horizontal = BTreeSet::new();
        let mut vertical = BTreeSet::new();
        for (enabled, direction) in [
            (edges.left, Direction::Left),
            (edges.right, Direction::Right),
        ] {
            if enabled {
                horizontal.extend(self.world.edge_followers(id, direction).unwrap_or_default());
            }
        }
        for (enabled, direction) in [(edges.top, Direction::Up), (edges.bottom, Direction::Down)] {
            if enabled {
                vertical.extend(self.world.edge_followers(id, direction).unwrap_or_default());
            }
        }
        let followers = horizontal
            .union(&vertical)
            .filter_map(|follower| {
                let managed = self.managed_window(*follower)?;
                let geometry = self.space.element_geometry(&managed.window)?;
                Some(ResizeFollowerPreview {
                    id: *follower,
                    screen: crate::layout::ScreenRect {
                        x: geometry.loc.x,
                        y: geometry.loc.y,
                        width: geometry.size.w,
                        height: geometry.size.h,
                    },
                    horizontal: horizontal.contains(follower),
                    vertical: vertical.contains(follower),
                })
            })
            .collect();
        self.pending_window_resize = Some(PendingWindowResize {
            id,
            rect,
            screen,
            edges,
            start: pointer_location,
            current: pointer_location,
            followers,
        });
        self.cursor_override = Some(resize_cursor(edges));
        true
    }

    fn finish_window_resize(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        let Some(resize) = self.pending_window_resize.take() else {
            return;
        };
        self.reset_window_presentation(resize.id);
        self.activate_window_without_camera(resize.id, SERIAL_COUNTER.next_serial());
        self.sync_layout(true);
        self.update_resize_cursor(pointer_location);
    }

    fn update_window_resize(&mut self) {
        let Some(resize) = self.pending_window_resize.clone() else {
            return;
        };
        let Some(output_size) = self.output_size else {
            return;
        };
        let camera = *self.world.camera();
        let preview = resize_preview_from_drag(
            resize.screen,
            resize.rect,
            resize.edges,
            resize.current.x - resize.start.x,
            resize.current.y - resize.start.y,
        );
        let (dx, dy) = pointer_delta_to_grid_move(
            resize.current.x - resize.start.x,
            resize.current.y - resize.start.y,
            output_size,
            camera.viewport_size(),
            camera.zoom(),
        );
        if let Some(rect) = resized_rect_from_drag(resize.rect, resize.edges, dx, dy) {
            if self.world.window(resize.id).map(mio_core::Window::rect) != Some(rect) {
                match self.world.apply(Action::ResizeWindowContinuousRect {
                    id: resize.id,
                    rect,
                }) {
                    Ok(_) => self.sync_layout(false),
                    Err(error) => debug!(%error, "pointer Window resize rejected"),
                }
            }
        }
        self.set_interactive_resize_preview(resize.id, preview);
        let follower_delta_x = f64::from(if resize.edges.left {
            preview.x.saturating_sub(resize.screen.x)
        } else {
            preview
                .x
                .saturating_add(preview.width)
                .saturating_sub(resize.screen.x.saturating_add(resize.screen.width))
        });
        let follower_delta_y = f64::from(if resize.edges.top {
            preview.y.saturating_sub(resize.screen.y)
        } else {
            preview
                .y
                .saturating_add(preview.height)
                .saturating_sub(resize.screen.y.saturating_add(resize.screen.height))
        });
        for follower in resize.followers {
            self.set_interactive_move_preview(
                follower.id,
                follower_preview_from_drag(follower, follower_delta_x, follower_delta_y),
            );
        }
    }

    fn resize_target_at(
        &self,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<(mio_core::WindowId, ResizeEdges)> {
        let (surface, _) = self.surface_under(position)?;
        if !window_resize_surface_allowed(self.popups.find_popup(&surface).is_some()) {
            return None;
        }
        let id = self.window_id_for_surface(&surface)?;
        let window = self.world.window(id)?;
        if window.presentation() != Presentation::Normal {
            return None;
        }
        let managed = self.managed_window(id)?;
        let geometry = self.space.element_geometry(&managed.window)?.to_f64();
        let local_x = position.x - geometry.loc.x;
        let local_y = position.y - geometry.loc.y;
        let edges = ResizeEdges {
            left: local_x <= WINDOW_RESIZE_EDGE,
            right: geometry.size.w - local_x <= WINDOW_RESIZE_EDGE,
            top: local_y <= WINDOW_RESIZE_EDGE,
            bottom: geometry.size.h - local_y <= WINDOW_RESIZE_EDGE,
        };
        (edges.left || edges.right || edges.top || edges.bottom).then_some((id, edges))
    }

    fn update_resize_cursor(
        &mut self,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        self.cursor_override = if let Some(resize) = self.pending_window_resize.as_ref() {
            Some(resize_cursor(resize.edges))
        } else if self.pending_camera_drag.is_none() && self.pending_window_drag.is_none() {
            self.resize_target_at(position)
                .map(|(_, edges)| resize_cursor(edges))
        } else {
            None
        };
    }

    #[allow(clippy::cast_precision_loss)] // Grid drag deltas cross into continuous World coordinates.
    fn finish_window_drag(&mut self) {
        let Some(drag) = self.pending_window_drag.take() else {
            return;
        };
        let Some(output_size) = self.output_size else {
            return;
        };
        let camera = *self.world.camera();
        let pointer_moved = {
            let delta = drag.current - drag.start;
            delta.x.hypot(delta.y) > DOUBLE_CLICK_DISTANCE
        };
        let floating = self
            .world
            .window(drag.id)
            .is_some_and(|window| window.grid_constraint() == mio_core::GridConstraint::Floating);
        let (delta_x, delta_y) = if floating {
            pointer_delta_to_world_move(
                drag.current.x - drag.start.x,
                drag.current.y - drag.start.y,
                output_size,
                camera.viewport_size(),
                camera.zoom(),
            )
        } else {
            let (x, y) = pointer_delta_to_grid_move(
                drag.current.x - drag.start.x,
                drag.current.y - drag.start.y,
                output_size,
                camera.viewport_size(),
                camera.zoom(),
            );
            (x as f64, y as f64)
        };
        let Ok(origin) = drag.origin.translated(delta_x, delta_y) else {
            self.reset_window_presentation(drag.id);
            self.sync_layout(false);
            return;
        };
        let moved = match self.world.apply(Action::MoveWindowContinuous {
            id: drag.id,
            origin,
        }) {
            Ok(_) => true,
            Err(error) => {
                debug!(%error, "pointer Window move rejected");
                false
            }
        };
        self.reset_window_presentation(drag.id);
        if moved && pointer_moved {
            self.activate_window_without_camera(drag.id, SERIAL_COUNTER.next_serial());
        } else {
            self.sync_layout(false);
        }
    }

    fn process_camera_drag_button<B, E>(
        &mut self,
        event: &E,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> CameraButtonResult
    where
        B: InputBackend,
        E: PointerButtonEvent<B>,
    {
        let button = self.pending_camera_drag.map_or_else(
            || mouse_button_code(self.config.config().mouse.camera_pan),
            |drag| drag.button,
        );
        if event.button_code() != button {
            return CameraButtonResult::Forward;
        }
        match event.state() {
            ButtonState::Pressed => {
                self.activate_virtual_output_at(pointer_location);
                let camera = *self.world.camera();
                let window = self
                    .surface_under(pointer_location)
                    .and_then(|(surface, _)| self.window_id_for_surface(&surface));
                self.pending_camera_drag = Some(PendingCameraDrag {
                    button,
                    start: pointer_location,
                    window,
                    start_camera_x: camera.position().x,
                    start_camera_y: camera.position().y,
                    start_zoom: camera.zoom(),
                    press_time: event.time_msec(),
                    dragging: false,
                    wheel_used: false,
                });
                CameraButtonResult::Consumed
            }
            ButtonState::Released => {
                let Some(drag) = self.pending_camera_drag.take() else {
                    return CameraButtonResult::Forward;
                };
                if drag.dragging || drag.wheel_used {
                    let camera = self.world.camera();
                    info!(
                        camera_x = camera.position().x,
                        camera_y = camera.position().y,
                        camera_zoom = camera.zoom(),
                        "mouse Camera gesture completed"
                    );
                    CameraButtonResult::Consumed
                } else {
                    let edge = self.space.outputs().find_map(|output| {
                        let geometry = self.space.output_geometry(output)?;
                        output_edge_direction(pointer_location.x, pointer_location.y, geometry)
                    });
                    let command = edge.and_then(|edge| {
                        self.config
                            .config()
                            .edge_commands
                            .iter()
                            .find(|command| command.edge == edge)
                            .map(|command| command.argv.clone())
                    });
                    if let Some(command) = command {
                        self.spawn_command(&command);
                        return CameraButtonResult::Consumed;
                    }
                    CameraButtonResult::ReplayClick {
                        button: drag.button,
                        press_time: drag.press_time,
                        window: drag.window,
                    }
                }
            }
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn process_pointer_axis<B, E>(&mut self, event: &E)
    where
        B: InputBackend,
        E: PointerAxisEvent<B>,
    {
        let source = event.source();
        let amount = |axis| {
            event
                .amount(axis)
                .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
        };
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        let zoom_button = self.config.config().mouse.camera_zoom;
        if !self.session_locked && button_held(self.pointer_buttons_held, zoom_button) {
            if let Some(drag) = &mut self.pending_camera_drag {
                drag.wheel_used = true;
            }
            let vertical = amount(Axis::Vertical);
            if vertical != 0.0 {
                let current = self.world.camera().zoom();
                let target = camera_zoom_from_scroll(current, vertical);
                match self.world.apply(Action::CameraZoom(target)) {
                    Ok(_) => {
                        info!(current, target, vertical, "mouse Camera zoom");
                        self.sync_layout(false);
                    }
                    Err(error) => debug!(%error, "mouse Camera zoom rejected"),
                }
            }
            return;
        }
        if !self.session_locked {
            let location = pointer.current_location();
            let edge = self.space.outputs().find_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                output_edge_direction(location.x, location.y, geometry)
            });
            if let Some(direction) = edge_wheel_focus_direction(
                source,
                edge,
                amount(Axis::Horizontal),
                amount(Axis::Vertical),
            ) {
                if let Ok(ActionOutcome::FocusChanged(Some(id))) =
                    self.world.apply(Action::Focus(direction))
                {
                    self.activate_window_after_focus_change(id, SERIAL_COUNTER.next_serial());
                    info!(
                        ?direction,
                        window = id.get(),
                        "focused Window from Output-edge wheel"
                    );
                }
                return;
            }
        }
        let mut frame = AxisFrame::new(event.time_msec()).source(source);

        for axis in [Axis::Horizontal, Axis::Vertical] {
            let value = amount(axis);
            if value != 0.0 {
                frame = frame.value(axis, value);
                if let Some(discrete) = event.amount_v120(axis) {
                    frame = frame.v120(axis, discrete as i32);
                }
            } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                frame = frame.stop(axis);
            }
        }

        pointer.axis(self, frame);
        pointer.frame(self);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CameraButtonResult {
    Forward,
    Consumed,
    ReplayClick {
        button: u32,
        press_time: u32,
        window: Option<mio_core::WindowId>,
    },
}

fn camera_drag_started(delta_x: f64, delta_y: f64) -> bool {
    delta_x.hypot(delta_y) >= CAMERA_DRAG_THRESHOLD
}

fn resize_cursor(edges: ResizeEdges) -> CursorIcon {
    match (edges.left, edges.right, edges.top, edges.bottom) {
        (true, _, true, _) | (_, true, _, true) => CursorIcon::NwseResize,
        (true, _, _, true) | (_, true, true, _) => CursorIcon::NeswResize,
        (true, _, _, _) | (_, true, _, _) => CursorIcon::EwResize,
        (_, _, true, _) | (_, _, _, true) => CursorIcon::NsResize,
        _ => CursorIcon::Default,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn resized_rect_from_drag(
    rect: mio_core::WorldRect,
    edges: ResizeEdges,
    dx: i64,
    dy: i64,
) -> Option<mio_core::WorldRect> {
    let mut left = rect.x();
    let mut top = rect.y();
    let mut right = rect.right().ok()?;
    let mut bottom = rect.bottom().ok()?;
    if edges.left {
        left += dx as f64;
    }
    if edges.right {
        right += dx as f64;
    }
    if edges.top {
        top += dy as f64;
    }
    if edges.bottom {
        bottom += dy as f64;
    }
    let width = right - left;
    let height = bottom - top;
    if width < 1.0 || height < 1.0 {
        return None;
    }
    mio_core::WorldRect::new(
        mio_core::WorldPoint::new(left, top).ok()?,
        GridSize::new(width.round() as u64, height.round() as u64).ok()?,
    )
    .ok()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn resize_preview_from_drag(
    screen: crate::layout::ScreenRect,
    rect: mio_core::WorldRect,
    edges: ResizeEdges,
    dx: f64,
    dy: f64,
) -> crate::layout::ScreenRect {
    let mut left = i64::from(screen.x);
    let mut top = i64::from(screen.y);
    let mut right = left + i64::from(screen.width);
    let mut bottom = top + i64::from(screen.height);
    let min_width = (f64::from(screen.width) / rect.width() as f64)
        .round()
        .max(1.0) as i64;
    let min_height = (f64::from(screen.height) / rect.height() as f64)
        .round()
        .max(1.0) as i64;
    let dx = dx.round() as i64;
    let dy = dy.round() as i64;
    if edges.left {
        left = (left + dx).min(right - min_width);
    }
    if edges.right {
        right = (right + dx).max(left + min_width);
    }
    if edges.top {
        top = (top + dy).min(bottom - min_height);
    }
    if edges.bottom {
        bottom = (bottom + dy).max(top + min_height);
    }
    crate::layout::ScreenRect {
        x: i32::try_from(left).unwrap_or(if left < 0 { i32::MIN } else { i32::MAX }),
        y: i32::try_from(top).unwrap_or(if top < 0 { i32::MIN } else { i32::MAX }),
        width: i32::try_from(right - left).unwrap_or(i32::MAX).max(1),
        height: i32::try_from(bottom - top).unwrap_or(i32::MAX).max(1),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn follower_preview_from_drag(
    follower: ResizeFollowerPreview,
    dx: f64,
    dy: f64,
) -> crate::layout::ScreenRect {
    crate::layout::ScreenRect {
        x: follower
            .screen
            .x
            .saturating_add(if follower.horizontal { dx as i32 } else { 0 }),
        y: follower
            .screen
            .y
            .saturating_add(if follower.vertical { dy as i32 } else { 0 }),
        ..follower.screen
    }
}

fn should_consume_focus_click(
    button: u32,
    state: ButtonState,
    target: Option<mio_core::WindowId>,
    focused: Option<mio_core::WindowId>,
) -> bool {
    button == BTN_LEFT
        && state == ButtonState::Pressed
        && target.is_some_and(|id| focused != Some(id))
}

const fn window_resize_surface_allowed(is_popup: bool) -> bool {
    !is_popup
}

const fn mouse_button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => BTN_LEFT,
        MouseButton::Right => BTN_RIGHT,
        MouseButton::Middle => BTN_MIDDLE,
    }
}

const fn pointer_button_mask(button: u32) -> u8 {
    match button {
        BTN_LEFT => POINTER_BUTTON_LEFT,
        BTN_RIGHT => POINTER_BUTTON_RIGHT,
        BTN_MIDDLE => POINTER_BUTTON_MIDDLE,
        _ => 0,
    }
}

fn mouse_chord_mask<const N: usize>(buttons: [MouseButton; N]) -> u8 {
    buttons.into_iter().fold(0, |mask, button| {
        mask | pointer_button_mask(mouse_button_code(button))
    })
}

fn button_held(held: u8, button: MouseButton) -> bool {
    held & pointer_button_mask(mouse_button_code(button)) != 0
}

fn ordered_chord_pressed(held: u8, pressed: u32, chord: [MouseButton; 2]) -> bool {
    let chord_mask = mouse_chord_mask(chord);
    pressed == mouse_button_code(chord[1]) && held & chord_mask == chord_mask
}

fn update_pointer_buttons(held: &mut u8, button: u32, state: ButtonState) {
    let bit = match button {
        BTN_LEFT => POINTER_BUTTON_LEFT,
        BTN_RIGHT => POINTER_BUTTON_RIGHT,
        BTN_MIDDLE => POINTER_BUTTON_MIDDLE,
        _ => return,
    };
    match state {
        ButtonState::Pressed => *held |= bit,
        ButtonState::Released => *held &= !bit,
    }
}

fn close_click_matches(
    first: PendingCloseClick,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    window: Option<mio_core::WindowId>,
    now: Instant,
) -> bool {
    let delta = position - first.position;
    window == Some(first.id)
        && now <= first.deadline
        && delta.x.hypot(delta.y) <= DOUBLE_CLICK_DISTANCE
}

fn next_close_click_count(
    first: PendingCloseClick,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    window: Option<mio_core::WindowId>,
    now: Instant,
) -> Option<u8> {
    close_click_matches(first, position, window, now).then(|| first.clicks.saturating_add(1))
}

fn pending_click_matches(
    first: PendingPointerClick,
    button: u32,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    window: Option<mio_core::WindowId>,
    now: Instant,
) -> bool {
    let delta = position - first.position;
    first.button == button
        && first.window.is_some()
        && first.window == window
        && now <= first.deadline
        && delta.x.hypot(delta.y) <= DOUBLE_CLICK_DISTANCE
}

fn pointer_click_target(
    deferred_camera_click: bool,
    pressed_window: Option<mio_core::WindowId>,
    release_window: Option<mio_core::WindowId>,
) -> Option<mio_core::WindowId> {
    if deferred_camera_click {
        pressed_window
    } else {
        release_window
    }
}

fn camera_zoom_from_scroll(current: f64, vertical_scroll: f64) -> f64 {
    (current * (-vertical_scroll * CAMERA_WHEEL_ZOOM_SENSITIVITY).exp())
        .clamp(CAMERA_WHEEL_ZOOM_MIN, 1.0)
}

fn output_edge_direction(
    x: f64,
    y: f64,
    geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
) -> Option<Direction> {
    let left = f64::from(geometry.loc.x);
    let top = f64::from(geometry.loc.y);
    let right = left + f64::from(geometry.size.w);
    let bottom = top + f64::from(geometry.size.h);
    if x < left || x > right || y < top || y > bottom {
        return None;
    }
    [
        ((x - left).abs(), Direction::Left),
        ((right - x).abs(), Direction::Right),
        ((y - top).abs(), Direction::Up),
        ((bottom - y).abs(), Direction::Down),
    ]
    .into_iter()
    .min_by(|left, right| left.0.total_cmp(&right.0))
    .filter(|(distance, _)| *distance <= EDGE_COMMAND_THRESHOLD)
    .map(|(_, direction)| direction)
}

fn edge_wheel_focus_direction(
    source: AxisSource,
    edge: Option<Direction>,
    horizontal: f64,
    vertical: f64,
) -> Option<Direction> {
    if !matches!(source, AxisSource::Wheel | AxisSource::WheelTilt) {
        return None;
    }

    let scroll = match vertical.total_cmp(&0.0) {
        std::cmp::Ordering::Equal => horizontal,
        std::cmp::Ordering::Less | std::cmp::Ordering::Greater => vertical,
    };
    match (edge?, scroll.total_cmp(&0.0)) {
        (Direction::Left | Direction::Right, std::cmp::Ordering::Less) => Some(Direction::Left),
        (Direction::Left | Direction::Right, std::cmp::Ordering::Greater) => Some(Direction::Right),
        (Direction::Up | Direction::Down, std::cmp::Ordering::Less) => Some(Direction::Up),
        (Direction::Up | Direction::Down, std::cmp::Ordering::Greater) => Some(Direction::Down),
        (_, std::cmp::Ordering::Equal) => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn pointer_delta_to_camera_pan(
    horizontal_delta: f64,
    vertical_delta: f64,
    output_size: (i32, i32),
    viewport: GridSize,
    zoom: f64,
) -> (f64, f64) {
    let pixels_x = f64::from(output_size.0.max(1));
    let pixels_y = f64::from(output_size.1.max(1));
    (
        -horizontal_delta * viewport.width() as f64 / (pixels_x * zoom),
        -vertical_delta * viewport.height() as f64 / (pixels_y * zoom),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub(crate) fn pointer_delta_to_grid_move(
    horizontal_delta: f64,
    vertical_delta: f64,
    output_size: (i32, i32),
    viewport: GridSize,
    zoom: f64,
) -> (i64, i64) {
    let pixels_x = f64::from(output_size.0.max(1));
    let pixels_y = f64::from(output_size.1.max(1));
    (
        (horizontal_delta * viewport.width() as f64 / (pixels_x * zoom)).round() as i64,
        (vertical_delta * viewport.height() as f64 / (pixels_y * zoom)).round() as i64,
    )
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn continuous_drag_screen_delta(
    horizontal_delta: f64,
    vertical_delta: f64,
) -> (i32, i32) {
    (
        horizontal_delta.round() as i32,
        vertical_delta.round() as i32,
    )
}

#[allow(clippy::cast_precision_loss)]
fn pointer_delta_to_world_move(
    horizontal_delta: f64,
    vertical_delta: f64,
    output_size: (i32, i32),
    viewport: GridSize,
    zoom: f64,
) -> (f64, f64) {
    let pixels_x = f64::from(output_size.0.max(1));
    let pixels_y = f64::from(output_size.1.max(1));
    (
        horizontal_delta * viewport.width() as f64 / (pixels_x * zoom),
        vertical_delta * viewport.height() as f64 / (pixels_y * zoom),
    )
}

fn toggled_opacity(opacity: f32, values: [f32; 2]) -> f32 {
    if (opacity - values[0]).abs() <= (opacity - values[1]).abs() {
        values[1]
    } else {
        values[0]
    }
}

fn key_from_keysym(symbol: Keysym) -> Option<Key> {
    match symbol {
        Keysym::Left => Some(Key::Left),
        Keysym::Right => Some(Key::Right),
        Keysym::Up => Some(Key::Up),
        Keysym::Down => Some(Key::Down),
        Keysym::Return => Some(Key::Enter),
        _ => char::from_u32(symbol.raw()).map(|value| Key::Letter(value.to_ascii_lowercase())),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod edge_tests {
    use mio_core::{GridRect, WorldRect};
    use smithay::utils::{Logical, Rectangle};

    use super::*;

    #[test]
    fn popup_surfaces_never_start_parent_window_resize() {
        assert!(window_resize_surface_allowed(false));
        assert!(!window_resize_surface_allowed(true));
    }

    #[test]
    fn camera_drag_tracks_pixels_and_zoom_in_world_coordinates() {
        let viewport = GridSize::new(4, 4).unwrap();
        assert_eq!(
            pointer_delta_to_camera_pan(200.0, -100.0, (800, 400), viewport, 1.0),
            (-1.0, 1.0)
        );
        assert_eq!(
            pointer_delta_to_camera_pan(100.0, 50.0, (800, 400), viewport, 0.5),
            (-1.0, -1.0)
        );
    }

    #[test]
    fn tiled_window_drag_commits_to_grid_but_previews_continuously() {
        let viewport = GridSize::new(4, 4).unwrap();
        assert_eq!(
            pointer_delta_to_grid_move(210.0, -90.0, (800, 400), viewport, 1.0),
            (1, -1)
        );
        assert_eq!(
            pointer_delta_to_grid_move(100.0, 50.0, (800, 400), viewport, 0.5),
            (1, 1)
        );
        assert_eq!(continuous_drag_screen_delta(49.0, 24.0), (49, 24));
    }

    #[test]
    fn floating_window_drag_preserves_subcell_motion() {
        let viewport = GridSize::new(8, 8).unwrap();
        assert_eq!(
            pointer_delta_to_world_move(50.0, -25.0, (800, 400), viewport, 1.0),
            (0.5, -0.5)
        );
        assert_eq!(
            pointer_delta_to_world_move(25.0, 12.5, (800, 400), viewport, 0.5),
            (0.5, 0.5)
        );
    }

    #[test]
    fn resize_drag_moves_only_the_grabbed_edges() {
        let rect: WorldRect = GridRect::new(-2, 3, 4, 5).unwrap().into();
        let right = ResizeEdges {
            left: false,
            right: true,
            top: false,
            bottom: false,
        };
        let upper_left = ResizeEdges {
            left: true,
            right: false,
            top: true,
            bottom: false,
        };
        assert_eq!(
            resized_rect_from_drag(rect, right, 2, 9),
            Some(GridRect::new(-2, 3, 6, 5).unwrap().into())
        );
        assert_eq!(
            resized_rect_from_drag(rect, upper_left, 1, -2),
            Some(GridRect::new(-1, 1, 3, 7).unwrap().into())
        );
        assert_eq!(resized_rect_from_drag(rect, upper_left, 4, 0), None);
    }

    #[test]
    fn resize_preview_follows_pointer_pixels_without_abandoning_grid_minimum() {
        let rect: WorldRect = GridRect::new(0, 0, 8, 6).unwrap().into();
        let screen = crate::layout::ScreenRect {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
        };
        let upper_right = ResizeEdges {
            left: false,
            right: true,
            top: true,
            bottom: false,
        };
        assert_eq!(
            resize_preview_from_drag(screen, rect, upper_right, 37.0, -23.0),
            crate::layout::ScreenRect {
                x: 100,
                y: 177,
                width: 837,
                height: 623,
            }
        );
        let left = ResizeEdges {
            left: true,
            right: false,
            top: false,
            bottom: false,
        };
        assert_eq!(
            resize_preview_from_drag(screen, rect, left, 900.0, 0.0),
            crate::layout::ScreenRect {
                x: 800,
                y: 200,
                width: 100,
                height: 600,
            }
        );
        assert_eq!(
            follower_preview_from_drag(
                ResizeFollowerPreview {
                    id: mio_core::WindowId::from_u64(2),
                    screen,
                    horizontal: true,
                    vertical: false,
                },
                37.0,
                -23.0,
            ),
            crate::layout::ScreenRect { x: 137, ..screen }
        );
    }

    #[test]
    fn short_right_click_stays_below_camera_drag_threshold() {
        assert!(!camera_drag_started(3.0, 4.0));
        assert!(camera_drag_started(6.0, 0.0));
    }

    #[test]
    fn only_left_press_on_an_unfocused_window_is_consumed_for_focus() {
        let focused = mio_core::WindowId::from_u64(1);
        let target = mio_core::WindowId::from_u64(2);

        assert!(should_consume_focus_click(
            BTN_LEFT,
            ButtonState::Pressed,
            Some(target),
            Some(focused)
        ));
        assert!(!should_consume_focus_click(
            BTN_LEFT,
            ButtonState::Pressed,
            Some(focused),
            Some(focused)
        ));
        assert!(!should_consume_focus_click(
            BTN_LEFT,
            ButtonState::Released,
            Some(target),
            Some(focused)
        ));
        assert!(!should_consume_focus_click(
            BTN_RIGHT,
            ButtonState::Pressed,
            Some(target),
            Some(focused)
        ));
    }

    #[test]
    fn floating_chord_consumes_both_release_orders() {
        for order in [[BTN_RIGHT, BTN_MIDDLE], [BTN_MIDDLE, BTN_RIGHT]] {
            let mut held = mouse_chord_mask([MouseButton::Right, MouseButton::Middle]);
            held &= !pointer_button_mask(order[0]);
            assert_ne!(held, 0);
            held &= !pointer_button_mask(order[1]);
            assert_eq!(held, 0);
        }
    }

    #[test]
    fn held_button_close_requires_same_window_position_and_deadline() {
        let now = Instant::now();
        let id = mio_core::WindowId::from_u64(7);
        let first = PendingCloseClick {
            id,
            position: (100.0, 80.0).into(),
            deadline: now + DOUBLE_CLICK_INTERVAL,
            clicks: 1,
        };
        assert!(close_click_matches(
            first,
            (104.0, 82.0).into(),
            Some(id),
            now
        ));
        assert!(!close_click_matches(
            first,
            (120.0, 80.0).into(),
            Some(id),
            now
        ));
        assert!(!close_click_matches(
            first,
            (100.0, 80.0).into(),
            Some(mio_core::WindowId::from_u64(8)),
            now
        ));
        assert!(!close_click_matches(
            first,
            (100.0, 80.0).into(),
            Some(id),
            first.deadline + Duration::from_millis(1)
        ));
        assert_eq!(
            next_close_click_count(first, first.position, Some(id), now),
            Some(2)
        );
        let second = PendingCloseClick { clicks: 2, ..first };
        assert_eq!(
            next_close_click_count(second, second.position, Some(id), now),
            Some(3)
        );
    }

    #[test]
    fn configured_mouse_chords_keep_the_declared_order() {
        let chord = [MouseButton::Middle, MouseButton::Left];
        let mut held = 0;
        update_pointer_buttons(&mut held, BTN_LEFT, ButtonState::Pressed);
        assert!(!ordered_chord_pressed(held, BTN_LEFT, chord));

        held = 0;
        update_pointer_buttons(&mut held, BTN_MIDDLE, ButtonState::Pressed);
        assert!(!ordered_chord_pressed(held, BTN_MIDDLE, chord));
        update_pointer_buttons(&mut held, BTN_LEFT, ButtonState::Pressed);
        assert!(ordered_chord_pressed(held, BTN_LEFT, chord));
    }

    #[test]
    fn double_click_requires_same_button_window_position_and_deadline() {
        let now = Instant::now();
        let window = mio_core::WindowId::from_u64(7);
        let first = PendingPointerClick {
            button: BTN_RIGHT,
            press_time: 10,
            release_time: 20,
            deadline: now + DOUBLE_CLICK_INTERVAL,
            position: (100.0, 200.0).into(),
            window: Some(window),
            clicks: 1,
        };
        assert!(pending_click_matches(
            first,
            BTN_RIGHT,
            (104.0, 202.0).into(),
            Some(window),
            now
        ));
        assert!(!pending_click_matches(
            first,
            BTN_MIDDLE,
            first.position,
            Some(window),
            now
        ));
        assert!(!pending_click_matches(
            first,
            BTN_RIGHT,
            (120.0, 200.0).into(),
            Some(window),
            now
        ));
        assert!(!pending_click_matches(
            first,
            BTN_RIGHT,
            first.position,
            Some(window),
            first.deadline + Duration::from_millis(1)
        ));
    }

    #[test]
    fn deferred_camera_click_keeps_the_press_target() {
        let pressed = mio_core::WindowId::from_u64(7);
        let release = mio_core::WindowId::from_u64(8);
        assert_eq!(
            pointer_click_target(true, Some(pressed), Some(release)),
            Some(pressed)
        );
        assert_eq!(pointer_click_target(true, None, Some(release)), None);
        assert_eq!(
            pointer_click_target(false, None, Some(release)),
            Some(release)
        );
    }

    #[test]
    fn edge_commands_select_the_nearest_output_edge() {
        let output = Rectangle::<i32, Logical>::new((100, 50).into(), (800, 600).into());
        assert_eq!(
            output_edge_direction(500.0, 649.0, output),
            Some(Direction::Down)
        );
        assert_eq!(output_edge_direction(500.0, 300.0, output), None);
    }

    #[test]
    fn physical_wheel_at_an_output_edge_selects_axis_and_scroll_selects_direction() {
        assert_eq!(
            edge_wheel_focus_direction(AxisSource::Wheel, Some(Direction::Left), 0.0, 15.0),
            Some(Direction::Right)
        );
        assert_eq!(
            edge_wheel_focus_direction(AxisSource::Wheel, Some(Direction::Right), 0.0, -15.0),
            Some(Direction::Left)
        );
        assert_eq!(
            edge_wheel_focus_direction(AxisSource::Wheel, Some(Direction::Up), 0.0, 15.0),
            Some(Direction::Down)
        );
        assert_eq!(
            edge_wheel_focus_direction(AxisSource::WheelTilt, Some(Direction::Down), -15.0, 0.0),
            Some(Direction::Up)
        );
        assert_eq!(
            edge_wheel_focus_direction(AxisSource::Finger, Some(Direction::Right), 0.0, 15.0),
            None
        );
        assert_eq!(
            edge_wheel_focus_direction(AxisSource::Wheel, None, 0.0, 15.0),
            None
        );
    }

    #[test]
    fn right_wheel_zoom_never_moves_closer_than_the_initial_view() {
        let farther = camera_zoom_from_scroll(1.0, 15.0);
        assert!(farther < 1.0);
        let closer = camera_zoom_from_scroll(farther, -30.0);
        assert_eq!(closer, 1.0);
        assert_eq!(camera_zoom_from_scroll(0.1, 1000.0), 0.1);
    }

    #[test]
    fn opacity_toggle_switches_between_full_and_eighty_percent() {
        assert!((toggled_opacity(1.0, [1.0, 0.8]) - 0.8).abs() < f32::EPSILON);
        assert!((toggled_opacity(0.8, [1.0, 0.8]) - 1.0).abs() < f32::EPSILON);
        assert!((toggled_opacity(0.7, [0.95, 0.7]) - 0.95).abs() < f32::EPSILON);
    }
}
