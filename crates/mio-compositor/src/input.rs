use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use mio_core::{
    Action, ActionOutcome, Direction, GridRect, GridSize, Presentation, WindowProperty,
    WindowPropertyKind,
};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        TouchEvent,
    },
    desktop::layer_map_for_output,
    input::{
        keyboard::{FilterResult, Keysym, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, CursorIcon, MotionEvent, PointerHandle, RelativeMotionEvent,
        },
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
    config::{ConfigAction, Key, KeyBinding, KeyChord, MouseAction, MouseButton, MouseConfig},
    layout::{world_to_screen, ScreenRect},
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
const WINDOW_RESIZE_OUTSIDE: f64 = 8.0;
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

struct PointerClickRequest {
    button: u32,
    press_time: u32,
    release_time: u32,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    window: Option<mio_core::WindowId>,
    required_clicks: u8,
    actions: Vec<ConfigAction>,
}

struct PointerFocusResult {
    target_window: Option<mio_core::WindowId>,
    consume_click: bool,
}

fn configured_mouse_actions(actions: &[MouseAction]) -> impl Iterator<Item = ConfigAction> + '_ {
    actions.iter().filter_map(|action| match action {
        MouseAction::Action(action) => Some(action.clone()),
        _ => None,
    })
}

struct MouseButtonBindings {
    camera_pan: Option<MouseButton>,
    move_window: Option<[MouseButton; 2]>,
    resize_window: Option<MouseButton>,
    toggle_floating: Option<[MouseButton; 2]>,
    place_next: Option<MouseButton>,
    window_click_buttons: Option<[MouseButton; 2]>,
    camera_click_binding: Option<(u8, Vec<ConfigAction>)>,
}

impl MouseButtonBindings {
    fn from_config(mouse: &MouseConfig) -> Self {
        let mut camera_pan = None;
        let mut move_window = None;
        let mut resize_window = None;
        let mut toggle_floating = None;
        let mut place_next = None;
        for binding in &mouse.bindings {
            for action in &binding.actions {
                match action {
                    MouseAction::CameraPan if camera_pan.is_none() => {
                        camera_pan = Some(binding.gesture.buttons[0]);
                    }
                    MouseAction::MoveWindow if move_window.is_none() => {
                        move_window =
                            Some([binding.gesture.buttons[0], binding.gesture.buttons[1]]);
                    }
                    MouseAction::ResizeWindow if resize_window.is_none() => {
                        resize_window = Some(binding.gesture.buttons[0]);
                    }
                    MouseAction::Action(ConfigAction::ToggleFloating)
                        if toggle_floating.is_none() =>
                    {
                        toggle_floating =
                            Some([binding.gesture.buttons[0], binding.gesture.buttons[1]]);
                    }
                    MouseAction::PlaceNext if place_next.is_none() => {
                        place_next = Some(binding.gesture.buttons[0]);
                    }
                    _ => {}
                }
            }
        }
        let mut window_click_buttons = None;
        let mut camera_click_binding = None;
        for binding in mouse.click_bindings() {
            if window_click_buttons.is_none()
                && move_window.is_some_and(|buttons| binding.gesture.buttons == buttons)
            {
                window_click_buttons = move_window;
            }
            if camera_click_binding.is_none()
                && camera_pan.is_some_and(|pan| binding.gesture.buttons == [pan])
            {
                let actions = configured_mouse_actions(&binding.actions).collect();
                camera_click_binding = Some((binding.gesture.clicks, actions));
            }
        }

        Self {
            camera_pan,
            move_window,
            resize_window,
            toggle_floating,
            place_next,
            window_click_buttons,
            camera_click_binding,
        }
    }
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
            InputEvent::PointerButton { event, .. } => {
                self.note_pointer_activity();
                self.process_pointer_button(&event);
            }
            InputEvent::PointerAxis { event, .. } => {
                self.note_pointer_activity();
                self.process_pointer_axis(&event);
            }
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
        self.note_pointer_activity();
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
        self.note_pointer_activity();
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

    fn process_pointer_position(
        &mut self,
        requested_position: smithay::utils::Point<f64, smithay::utils::Logical>,
        delta: smithay::utils::Point<f64, smithay::utils::Logical>,
        delta_unaccel: smithay::utils::Point<f64, smithay::utils::Logical>,
        time_msec: u32,
        time: u64,
    ) {
        self.expire_pointer_clicks_after_motion(requested_position);
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        let old_position = pointer.current_location();
        let camera_dragging = self.update_pointer_interactions(requested_position);
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

        let position = constrained_pointer_position(
            &pointer,
            old_position,
            requested_position,
            camera_dragging,
            under.as_ref(),
            requested_under.as_ref(),
        );
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
        activate_pointer_constraint(&pointer, under.as_ref(), position);
    }

    fn expire_pointer_clicks_after_motion(
        &mut self,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        if self
            .pending_pointer_click
            .is_some_and(|click| click_moved_too_far(click.position, position))
        {
            self.replay_pending_pointer_click();
        }
        if self
            .pending_close_click
            .is_some_and(|click| click_moved_too_far(click.position, position))
        {
            self.pending_close_click = None;
        }
    }

    fn update_pointer_interactions(
        &mut self,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let swap_target = self
            .pending_window_drag
            .and_then(|drag| self.window_swap_candidate(drag.id, position, drag.swap_target));
        if let Some(drag) = &mut self.pending_window_drag {
            drag.current = position;
            drag.swap_target = swap_target;
        }
        if let Some(resize) = &mut self.pending_window_resize {
            resize.current = position;
        }
        if self.pending_window_resize.is_some() {
            self.update_window_resize();
        }
        let mut camera_dragging = false;
        if self.pending_window_drag.is_none() {
            if let Some(drag) = &mut self.pending_camera_drag {
                let from_start = position - drag.start;
                if !drag.dragging && camera_drag_started(from_start.x, from_start.y) {
                    drag.dragging = true;
                }
                camera_dragging = drag.dragging;
            }
        }
        if camera_dragging {
            self.pan_camera_to_pointer_position(position);
        }
        camera_dragging
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

    fn process_keyboard<B: InputBackend>(
        &mut self,
        event: &B::KeyboardKeyEvent,
    ) -> Option<BackendInputAction> {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        let key_state = event.state();
        if key_state == KeyState::Pressed {
            self.hide_cursor_for_keyboard_input();
        }
        if self.session_locked {
            self.forward_keyboard_event::<B>(event, serial, None);
            return None;
        }
        if let Some(surface) = self.exclusive_keyboard_layer_surface() {
            if self.forward_keyboard_event::<B>(event, serial, Some(surface)) {
                return None;
            }
        }
        if self.keyboard_shortcuts_inhibited() {
            self.forward_keyboard_event::<B>(event, serial, None);
            return None;
        }
        let bindings =
            (key_state == KeyState::Pressed).then(|| self.config.config().bindings.clone());
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
                    let keysym = handle
                        .raw_latin_sym_or_raw_current_sym()
                        .unwrap_or_else(|| handle.modified_sym());
                    let action = keyboard_action_from_keysym(
                        bindings.as_deref().unwrap_or_default(),
                        modifiers,
                        keysym,
                    );
                    action.map_or(FilterResult::Forward, FilterResult::Intercept)
                },
            );
            return self.dispatch_keyboard_action(shortcut, serial);
        }
        None
    }

    fn keyboard_shortcuts_inhibited(&self) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|surface| self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface))
            .is_some_and(|inhibitor| inhibitor.is_active())
    }

    fn dispatch_keyboard_action(
        &mut self,
        action: Option<KeyboardAction>,
        serial: smithay::utils::Serial,
    ) -> Option<BackendInputAction> {
        match action {
            Some(KeyboardAction::Config(action)) => {
                self.process_shortcut(action, serial);
                None
            }
            Some(KeyboardAction::ChangeVt(vt)) => Some(BackendInputAction::ChangeVt(vt)),
            None => None,
        }
    }

    fn forward_keyboard_event<B: InputBackend>(
        &mut self,
        event: &B::KeyboardKeyEvent,
        serial: smithay::utils::Serial,
        focus: Option<WlSurface>,
    ) -> bool {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        if let Some(surface) = focus {
            keyboard.set_focus(self, Some(surface), serial);
        }
        keyboard.input::<(), _>(
            self,
            event.key_code(),
            event.state(),
            serial,
            event.time_msec(),
            |_, _, _| FilterResult::Forward,
        );
        true
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
            ConfigAction::Close => self.close_focused_window(focused),
            ConfigAction::Camera(direction) => {
                self.apply_and_sync_layout(Action::CameraStep(direction));
            }
            ConfigAction::CameraCenter => {
                let Some(id) = focused else { return };
                self.apply_and_sync_layout(Action::CameraCenter(id));
            }
            ConfigAction::CameraNudge(direction) => {
                self.apply_and_sync_layout(Action::CameraNudge(direction));
            }
            ConfigAction::CameraZoom(zoom) => {
                self.apply_and_sync_layout(Action::CameraZoom(zoom));
            }
            ConfigAction::CycleOutput => {
                self.apply_and_sync_layout(Action::CycleOutput);
            }
            ConfigAction::ToggleFloating => self.toggle_focused_floating(focused),
            ConfigAction::Focus(direction) => {
                let _ = self.focus_window_direction(direction, focused, serial);
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
            ConfigAction::ToggleOverview => self.toggle_overview(),
            ConfigAction::SelectOverview => self.select_overview_window(),
            ConfigAction::Move(direction) => self.move_focused_window(direction),
            ConfigAction::Resize(direction) => self.resize_focused_window(direction),
            ConfigAction::PlaceNext(direction) => self.set_next_placement(direction),
            ConfigAction::ReloadConfig => self.reload_config(),
            ConfigAction::Spawn(argv) => self.spawn_command(&argv),
        }
    }

    fn close_focused_window(&mut self, focused: Option<mio_core::WindowId>) {
        let Some(id) = focused else { return };
        if self.world.apply(Action::CloseWindow(id)) == Ok(ActionOutcome::CloseRequested(id)) {
            self.begin_close_transition(id);
        }
    }

    fn toggle_focused_floating(&mut self, focused: Option<mio_core::WindowId>) {
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

    fn focus_window_direction(
        &mut self,
        direction: Direction,
        previous_focus: Option<mio_core::WindowId>,
        serial: smithay::utils::Serial,
    ) -> Option<mio_core::WindowId> {
        if let Ok(ActionOutcome::FocusChanged(Some(id))) =
            self.world.apply(Action::Focus(direction))
        {
            self.activate_window_after_focus_change(id, previous_focus, serial);
            Some(id)
        } else {
            None
        }
    }

    fn set_next_placement(&mut self, direction: Direction) {
        let _ = self.world.apply(Action::SetNextPlacement(direction));
        info!(?direction, "next Window placement direction changed");
    }

    fn apply_and_sync_layout(&mut self, action: Action) {
        if self.world.apply(action).is_ok() {
            self.sync_layout(false);
        }
    }

    fn reload_config(&mut self) {
        match self.config.reload() {
            Ok(()) => {
                self.config_error = None;
                let viewport = self.config.config().viewport;
                if let Err(error) = self.world.resize_camera_viewport(viewport) {
                    warn!(%error, "new viewport rejected after configuration reload");
                }
                self.apply_output_scale();
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

    fn process_pointer_button<B, E>(&mut self, event: &E)
    where
        B: InputBackend,
        E: PointerButtonEvent<B>,
    {
        let button = event.button_code();
        let state = event.state();
        let time = event.time_msec();
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        if state == ButtonState::Pressed
            && self.pending_pointer_click.is_some_and(|click| {
                button != click.button
                    || Instant::now() > click.deadline
                    || click_moved_too_far(click.position, pointer.current_location())
            })
        {
            self.replay_pending_pointer_click();
        }
        update_pointer_buttons(&mut self.pointer_buttons_held, button, state);
        let bindings = MouseButtonBindings::from_config(&self.config.config().mouse);
        let position = pointer.current_location();
        if self.finish_active_pointer_interaction(button, state, position, &bindings) {
            return;
        }
        if self.begin_configured_pointer_interaction(button, state, position, &bindings) {
            return;
        }
        let replay_camera_press = if self.session_locked {
            None
        } else {
            match self.process_camera_drag_button(
                button,
                state,
                time,
                position,
                bindings.camera_pan,
            ) {
                CameraButtonResult::Consumed => return,
                CameraButtonResult::ReplayClick {
                    button,
                    press_time,
                    window,
                } => Some((button, press_time, window)),
                CameraButtonResult::Forward => None,
            }
        };
        let serial = SERIAL_COUNTER.next_serial();
        let PointerFocusResult {
            target_window,
            consume_click,
        } = self.resolve_pointer_button_focus(button, state, position, replay_camera_press, serial);
        if consume_click {
            self.suppressed_focus_click = true;
            return;
        }

        if self.handle_replayed_camera_click(
            replay_camera_press,
            bindings.camera_click_binding,
            time,
            position,
            target_window,
        ) {
            return;
        }

        // Camera zoom changes the mapping between the global pointer and client-local
        // coordinates even when the pointer itself has not moved. Refresh the focus
        // origin immediately before forwarding a button so the click never uses the
        // pre-zoom mapping retained by Smithay.
        pointer.motion(
            self,
            self.surface_under(position),
            &MotionEvent {
                location: position,
                serial,
                time,
            },
        );
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    fn handle_replayed_camera_click(
        &mut self,
        replay_camera_press: Option<(u32, u32, Option<mio_core::WindowId>)>,
        camera_click_binding: Option<(u8, Vec<ConfigAction>)>,
        release_time: u32,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        target_window: Option<mio_core::WindowId>,
    ) -> bool {
        let Some((button, press_time, _)) = replay_camera_press else {
            return false;
        };
        if let Some((clicks, actions)) = camera_click_binding {
            self.defer_or_complete_pointer_click(PointerClickRequest {
                button,
                press_time,
                release_time,
                position,
                window: target_window,
                required_clicks: clicks,
                actions,
            });
        } else {
            self.pending_pointer_click = Some(PendingPointerClick {
                button,
                press_time,
                release_time,
                deadline: Instant::now(),
                position,
                window: target_window,
                clicks: 1,
            });
            self.replay_pending_pointer_click();
        }
        true
    }

    fn begin_configured_pointer_interaction(
        &mut self,
        button: u32,
        state: ButtonState,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        bindings: &MouseButtonBindings,
    ) -> bool {
        if self.session_locked || state != ButtonState::Pressed {
            return false;
        }

        if bindings
            .resize_window
            .is_some_and(|resize_button| button == mouse_button_code(resize_button))
            && bindings
                .move_window
                .is_none_or(|buttons| !button_held(self.pointer_buttons_held, buttons[0]))
            && self.begin_window_resize(position)
        {
            return true;
        }

        if let Some(buttons) = bindings.toggle_floating {
            if ordered_chord_pressed(self.pointer_buttons_held, button, buttons)
                && self.toggle_floating_with_pointer_chord(position, buttons)
            {
                return true;
            }
        }

        if let Some(place_button) = bindings.place_next {
            if button == mouse_button_code(place_button)
                && self.begin_edge_placement(position, place_button)
            {
                return true;
            }
        }

        bindings.move_window.is_some_and(|buttons| {
            ordered_chord_pressed(self.pointer_buttons_held, button, buttons)
                && self.begin_window_drag(position, buttons)
        })
    }

    fn resolve_pointer_button_focus(
        &mut self,
        button: u32,
        state: ButtonState,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        replay_camera_press: Option<(u32, u32, Option<mio_core::WindowId>)>,
        serial: smithay::utils::Serial,
    ) -> PointerFocusResult {
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        let keyboard = self
            .seat
            .get_keyboard()
            .expect("Mio always creates a keyboard");
        let input_method = self.seat.input_method();
        let may_change_focus = (state == ButtonState::Pressed || replay_camera_press.is_some())
            && !pointer.is_grabbed()
            && (!keyboard.is_grabbed() || input_method.keyboard_grabbed());
        let release_window = may_change_focus
            .then(|| self.surface_under(position))
            .flatten()
            .and_then(|(surface, _)| self.window_id_for_surface(&surface));
        let target_window = pointer_click_target(
            replay_camera_press.is_some(),
            replay_camera_press.and_then(|(_, _, window)| window),
            release_window,
        );
        let consume_click =
            should_consume_focus_click(button, state, target_window, self.world.focused());

        if may_change_focus {
            self.activate_virtual_output_at(position);
            if let Some((surface, _)) = self.surface_under(position) {
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

        PointerFocusResult {
            target_window,
            consume_click,
        }
    }

    fn finish_active_pointer_interaction(
        &mut self,
        button: u32,
        state: ButtonState,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        bindings: &MouseButtonBindings,
    ) -> bool {
        if self.closing_pointer_chord {
            if self.pointer_buttons_held == 0 {
                self.closing_pointer_chord = false;
            }
            return true;
        }
        let click_buttons = self.update_pending_window_click(
            button,
            state,
            position,
            bindings.window_click_buttons,
        );
        let released_mask = pointer_button_mask(button);
        self.process_floating_chord_release(button, state)
            || self.finish_edge_placement_release(button, state)
            || self.finish_window_resize_release(button, state, position, bindings.resize_window)
            || self.consume_suppressed_window_drag_release(state, released_mask)
            || self.finish_pending_window_drag_release(
                button,
                state,
                released_mask,
                click_buttons,
                bindings.window_click_buttons,
            )
            || self.consume_suppressed_focus_click(button, state)
    }

    fn finish_edge_placement_release(&mut self, button: u32, state: ButtonState) -> bool {
        if state != ButtonState::Released {
            return false;
        }
        let Some((direction, pending_button)) = self.pending_edge_placement else {
            return false;
        };
        if button != pending_button {
            return false;
        }
        self.pending_edge_placement = None;
        if let Err(error) = self.world.apply(Action::SetNextPlacement(direction)) {
            debug!(%error, ?direction, "pointer placement direction rejected");
        } else {
            info!(
                ?direction,
                "next Window placement selected from Output edge"
            );
        }
        true
    }

    fn finish_window_resize_release(
        &mut self,
        button: u32,
        state: ButtonState,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        resize_button: Option<MouseButton>,
    ) -> bool {
        if state != ButtonState::Released
            || self.pending_window_resize.is_none()
            || resize_button.is_none_or(|resize_button| button != mouse_button_code(resize_button))
        {
            return false;
        }
        self.finish_window_resize(position);
        true
    }

    fn consume_suppressed_window_drag_release(
        &mut self,
        state: ButtonState,
        released_mask: u8,
    ) -> bool {
        if state != ButtonState::Released
            || released_mask == 0
            || self.suppressed_window_drag_releases & released_mask == 0
        {
            return false;
        }
        self.suppressed_window_drag_releases &= !released_mask;
        true
    }

    fn consume_suppressed_focus_click(&mut self, button: u32, state: ButtonState) -> bool {
        button == BTN_LEFT
            && state == ButtonState::Released
            && std::mem::take(&mut self.suppressed_focus_click)
    }

    fn update_pending_window_click(
        &mut self,
        button: u32,
        state: ButtonState,
        position: smithay::utils::Point<f64, smithay::utils::Logical>,
        window_click_buttons: Option<[MouseButton; 2]>,
    ) -> Option<[u32; 2]> {
        let click_buttons = window_click_buttons.map(|buttons| buttons.map(mouse_button_code));
        if click_buttons.is_some_and(|buttons| button == buttons[0])
            && state == ButtonState::Released
        {
            if let Some(click) = self.pending_close_click.take() {
                self.complete_pending_window_click(click);
            }
        }
        if !self.session_locked
            && click_buttons.is_some_and(|buttons| button == buttons[1])
            && state == ButtonState::Pressed
            && window_click_buttons
                .is_some_and(|buttons| button_held(self.pointer_buttons_held, buttons[0]))
        {
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
        click_buttons
    }

    fn finish_pending_window_drag_release(
        &mut self,
        button: u32,
        state: ButtonState,
        released_mask: u8,
        click_buttons: Option<[u32; 2]>,
        window_click_buttons: Option<[MouseButton; 2]>,
    ) -> bool {
        if state != ButtonState::Released
            || self
                .pending_window_drag
                .is_none_or(|drag| drag.buttons & released_mask == 0)
        {
            return false;
        }
        let close_click = self.pending_window_drag.and_then(|drag| {
            (click_buttons.is_some_and(|buttons| button == buttons[1])
                && window_click_buttons
                    .is_some_and(|buttons| button_held(self.pointer_buttons_held, buttons[0]))
                && !click_moved_too_far(drag.start, drag.current))
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
        true
    }

    fn begin_window_drag(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
        buttons: [MouseButton; 2],
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
        let buttons = mouse_chord_mask(buttons);
        self.pending_window_drag = Some(PendingWindowDrag {
            id,
            buttons,
            origin,
            start: pointer_location,
            current: pointer_location,
            swap_target: None,
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
        buttons: [MouseButton; 2],
    ) -> bool {
        let Some((surface, _)) = self.surface_under(pointer_location) else {
            return false;
        };
        let Some(id) = self.window_id_for_surface(&surface) else {
            return false;
        };
        self.pending_camera_drag = None;
        let buttons_held = mouse_chord_mask(buttons);
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
        button: MouseButton,
    ) -> bool {
        let direction = self.space.outputs().find_map(|output| {
            let geometry = self.space.output_geometry(output)?;
            output_edge_direction(pointer_location.x, pointer_location.y, geometry)
        });
        let button = mouse_button_code(button);
        self.pending_edge_placement = direction.map(|direction| (direction, button));
        direction.is_some()
    }

    fn defer_or_complete_pointer_click(&mut self, request: PointerClickRequest) {
        let PointerClickRequest {
            button,
            press_time,
            release_time,
            position,
            window,
            required_clicks,
            actions,
        } = request;
        let now = Instant::now();
        let next_clicks = self
            .pending_pointer_click
            .filter(|first| pending_click_matches(*first, button, position, window, now))
            .map(|first| first.clicks.saturating_add(1));
        if let Some(clicks) = next_clicks {
            if clicks >= required_clicks {
                self.pending_pointer_click = None;
                if let Some(id) = window {
                    self.run_window_click_actions(id, actions);
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
        if required_clicks == 1 {
            if let Some(id) = window {
                self.run_window_click_actions(id, actions);
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
        let actions = {
            let mouse = &self.config.config().mouse;
            let Some(buttons) = mouse
                .binding_with_action(&MouseAction::MoveWindow)
                .map(|binding| binding.gesture.buttons.as_slice())
            else {
                return;
            };
            let Some(binding) = mouse.click_bindings().find(|binding| {
                binding.gesture.buttons == buttons && binding.gesture.clicks == click.clicks
            }) else {
                return;
            };
            configured_mouse_actions(&binding.actions).collect::<Vec<_>>()
        };
        if actions.contains(&ConfigAction::Close) {
            self.closing_pointer_chord = true;
        }
        self.run_window_click_actions(click.id, actions);
    }

    fn run_window_click_actions(&mut self, id: mio_core::WindowId, actions: Vec<ConfigAction>) {
        if self.world.window(id).is_none() {
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        self.activate_window_without_camera(id, serial);
        for action in actions {
            self.process_shortcut(action, serial);
        }
        info!(window = id.get(), "pointer click action sequence completed");
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
        if let Some(output_size) = self.output_size {
            let camera = *self.world.camera();
            let (dx, dy) = pointer_delta_to_grid_move(
                resize.current.x - resize.start.x,
                resize.current.y - resize.start.y,
                output_size,
                camera.viewport_size(),
                camera.zoom(),
            );
            if let Some(rect) = resized_rect_from_drag(resize.rect, resize.edges, dx, dy) {
                if let Err(error) = self.world.apply(Action::ResizeWindowRect {
                    id: resize.id,
                    rect,
                }) {
                    debug!(%error, "pointer Window resize commit rejected");
                }
            }
        }
        self.reset_window_presentation(resize.id);
        self.activate_window_without_camera(resize.id, SERIAL_COUNTER.next_serial());
        self.sync_layout(true);
        self.update_resize_cursor(pointer_location);
    }

    fn update_window_resize(&mut self) {
        let Some(resize) = self.pending_window_resize.clone() else {
            return;
        };
        let preview = resize_preview_from_drag(
            resize.screen,
            resize.rect,
            resize.edges,
            resize.current.x - resize.start.x,
            resize.current.y - resize.start.y,
        );
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
        let surface_window = match self.surface_under(position) {
            Some((surface, _)) => {
                if !window_resize_surface_allowed(self.popups.find_popup(&surface).is_some()) {
                    return None;
                }
                Some(self.window_id_for_surface(&surface)?)
            }
            None => None,
        };
        let border_inside = f64::from(self.config.config().appearance.window_border_width);
        self.space.elements().rev().find_map(|managed_window| {
            let managed = self
                .managed_windows
                .iter()
                .find(|managed| managed.window == *managed_window)?;
            if surface_window.is_some_and(|id| id != managed.id)
                || self.world.window(managed.id)?.presentation() != Presentation::Normal
            {
                return None;
            }
            let geometry = self.space.element_geometry(managed_window)?.to_f64();
            resize_edges_at(position, geometry, border_inside, WINDOW_RESIZE_OUTSIDE)
                .map(|edges| (managed.id, edges))
        })
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
        let action = if floating {
            None
        } else {
            drag.swap_target.map(|second| Action::SwapWindows {
                first: drag.id,
                second,
            })
        }
        .unwrap_or(Action::MoveWindowContinuous {
            id: drag.id,
            origin,
        });
        let moved = match self.world.apply(action) {
            Ok(_) => true,
            Err(error) => {
                debug!(%error, "pointer Window move rejected");
                false
            }
        };
        // Drag presentation is the animated base rectangle plus a temporary
        // pointer offset. Preserve that exact release position before removing
        // the offset, then let layout sync retarget it to the committed Grid
        // origin (or swapped origin) through the existing inertial animation.
        if moved && !floating {
            let pointer_delta = drag.current - drag.start;
            self.commit_interactive_move_offset(
                drag.id,
                continuous_drag_screen_delta(pointer_delta.x, pointer_delta.y),
            );
        } else {
            self.reset_window_presentation(drag.id);
        }
        if moved && pointer_moved {
            self.activate_window_without_camera(drag.id, SERIAL_COUNTER.next_serial());
        } else {
            self.sync_layout(false);
        }
    }

    fn window_swap_candidate(
        &self,
        dragged: mio_core::WindowId,
        pointer: smithay::utils::Point<f64, smithay::utils::Logical>,
        current: Option<mio_core::WindowId>,
    ) -> Option<mio_core::WindowId> {
        let output_size = self.output_size?;
        let camera = *self.world.camera();
        let candidate_rect = |id| {
            let window = self.world.window(id)?;
            (window.id() != dragged
                && window.grid_constraint() == mio_core::GridConstraint::Tiled
                && window.presentation() == Presentation::Normal)
                .then(|| world_to_screen(window.rect(), camera, output_size))
                .flatten()
        };

        if current
            .and_then(candidate_rect)
            .is_some_and(|rect| point_in_center_region(pointer, rect, 0.70))
        {
            return current;
        }

        self.world.windows().find_map(|window| {
            candidate_rect(window.id())
                .filter(|rect| point_in_center_region(pointer, *rect, 0.60))
                .map(|_| window.id())
        })
    }

    fn process_camera_drag_button(
        &mut self,
        event_button: u32,
        event_state: ButtonState,
        event_time: u32,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
        configured_button: Option<MouseButton>,
    ) -> CameraButtonResult {
        let Some(button) = self
            .pending_camera_drag
            .map(|drag| drag.button)
            .or_else(|| configured_button.map(mouse_button_code))
        else {
            return CameraButtonResult::Forward;
        };
        if event_button != button {
            return CameraButtonResult::Forward;
        }
        match event_state {
            ButtonState::Pressed => {
                self.begin_camera_drag(button, pointer_location, event_time);
                CameraButtonResult::Consumed
            }
            ButtonState::Released => self.finish_camera_drag(pointer_location),
        }
    }

    fn begin_camera_drag(
        &mut self,
        button: u32,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
        press_time: u32,
    ) {
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
            press_time,
            dragging: false,
            wheel_used: false,
        });
    }

    fn finish_camera_drag(
        &mut self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> CameraButtonResult {
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
            return CameraButtonResult::Consumed;
        }

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

    fn process_pointer_axis<B, E>(&mut self, event: &E)
    where
        B: InputBackend,
        E: PointerAxisEvent<B>,
    {
        let source = event.source();
        let horizontal = normalized_axis_amount(
            event.amount(Axis::Horizontal),
            event.amount_v120(Axis::Horizontal),
        );
        let vertical = normalized_axis_amount(
            event.amount(Axis::Vertical),
            event.amount_v120(Axis::Vertical),
        );
        let pointer = self
            .seat
            .get_pointer()
            .expect("Mio always creates a pointer");
        if self.consume_camera_zoom_axis(vertical) {
            return;
        }
        if self.consume_edge_focus_axis(source, horizontal, vertical, pointer.current_location()) {
            return;
        }
        pointer.axis(
            self,
            pointer_axis_frame::<B, E>(event, source, [horizontal, vertical]),
        );
        pointer.frame(self);
    }

    fn consume_camera_zoom_axis(&mut self, vertical: f64) -> bool {
        let mouse = &self.config.config().mouse;
        let zoom_button = mouse
            .binding_with_action(&MouseAction::CameraZoom)
            .map(|binding| binding.gesture.buttons[0]);
        let active = !self.session_locked
            && zoom_button.is_some_and(|button| button_held(self.pointer_buttons_held, button));
        if !active {
            return false;
        }
        if let Some(drag) = &mut self.pending_camera_drag {
            drag.wheel_used = true;
        }
        if vertical != 0.0 {
            self.pending_camera_zoom_scroll += vertical;
        }
        true
    }

    fn consume_edge_focus_axis(
        &mut self,
        source: AxisSource,
        horizontal: f64,
        vertical: f64,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        if self.session_locked {
            return false;
        }
        let edge = self.space.outputs().find_map(|output| {
            let geometry = self.space.output_geometry(output)?;
            output_edge_direction(location.x, location.y, geometry)
        });
        let Some(direction) = edge_wheel_focus_direction(source, edge, horizontal, vertical) else {
            return false;
        };
        if let Some(id) = self.focus_window_direction(
            direction,
            self.world.focused(),
            SERIAL_COUNTER.next_serial(),
        ) {
            info!(
                ?direction,
                window = id.get(),
                "focused Window from Output-edge wheel"
            );
        }
        true
    }
}

#[allow(clippy::cast_possible_wrap)] // VT keysyms are validated to the positive 1..=12 range.
fn keyboard_action_from_keysym(
    bindings: &[KeyBinding],
    modifiers: &ModifiersState,
    keysym: Keysym,
) -> Option<KeyboardAction> {
    if (xkb::keysyms::KEY_XF86Switch_VT_1..=xkb::keysyms::KEY_XF86Switch_VT_12)
        .contains(&keysym.raw())
    {
        let vt = (keysym.raw() - xkb::keysyms::KEY_XF86Switch_VT_1 + 1) as i32;
        return Some(KeyboardAction::ChangeVt(vt));
    }
    let key = key_from_keysym(keysym)?;
    let chord = KeyChord {
        ctrl: modifiers.ctrl,
        alt: modifiers.alt,
        shift: modifiers.shift,
        logo: modifiers.logo,
        key,
    };
    bindings
        .iter()
        .find(|binding| binding.chord == chord)
        .map(|binding| KeyboardAction::Config(binding.action.clone()))
}

fn constrained_pointer_position(
    pointer: &PointerHandle<MioState>,
    old_position: smithay::utils::Point<f64, smithay::utils::Logical>,
    requested_position: smithay::utils::Point<f64, smithay::utils::Logical>,
    camera_dragging: bool,
    under: Option<&(
        WlSurface,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    )>,
    requested_under: Option<&(
        WlSurface,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    )>,
) -> smithay::utils::Point<f64, smithay::utils::Logical> {
    if camera_dragging {
        return requested_position;
    }
    let Some((surface, origin)) = under else {
        return requested_position;
    };
    let same_surface = requested_under.is_some_and(|(candidate, _)| candidate == surface);
    let constrained = with_pointer_constraint(surface, pointer, |constraint| {
        let Some(constraint) = constraint.filter(|constraint| constraint.is_active()) else {
            return false;
        };
        match &*constraint {
            PointerConstraint::Locked(_) => true,
            PointerConstraint::Confined(_) => {
                let inside_region = constraint.region().is_none_or(|region| {
                    region.contains((requested_position - *origin).to_i32_round())
                });
                !same_surface || !inside_region
            }
        }
    });
    if constrained {
        old_position
    } else {
        requested_position
    }
}

fn activate_pointer_constraint(
    pointer: &PointerHandle<MioState>,
    under: Option<&(
        WlSurface,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    )>,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
) {
    let Some((surface, origin)) = under else {
        return;
    };
    with_pointer_constraint(surface, pointer, |constraint| {
        if let Some(constraint) = constraint.filter(|constraint| !constraint.is_active()) {
            let local = (position - *origin).to_i32_round();
            if constraint
                .region()
                .is_none_or(|region| region.contains(local))
            {
                constraint.activate();
            }
        }
    });
}

#[allow(clippy::cast_possible_truncation)]
fn pointer_axis_frame<B, E>(event: &E, source: AxisSource, amounts: [f64; 2]) -> AxisFrame
where
    B: InputBackend,
    E: PointerAxisEvent<B>,
{
    let mut frame = AxisFrame::new(event.time_msec()).source(source);
    for (axis, value) in [Axis::Horizontal, Axis::Vertical].into_iter().zip(amounts) {
        if value != 0.0 {
            frame = frame.value(axis, value);
            if let Some(discrete) = event.amount_v120(axis) {
                frame = frame.v120(axis, discrete as i32);
            }
        } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
            frame = frame.stop(axis);
        }
    }
    frame
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
) -> Option<GridRect> {
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
    GridRect::new(
        left.round() as i64,
        top.round() as i64,
        width.round() as u64,
        height.round() as u64,
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

fn resize_edges_at(
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    geometry: smithay::utils::Rectangle<f64, smithay::utils::Logical>,
    inside: f64,
    outside: f64,
) -> Option<ResizeEdges> {
    let local = position - geometry.loc;
    let in_vertical_span = local.y >= -outside && local.y <= geometry.size.h + outside;
    let in_horizontal_span = local.x >= -outside && local.x <= geometry.size.w + outside;
    let edges = ResizeEdges {
        left: in_vertical_span && local.x >= -outside && local.x <= inside,
        right: in_vertical_span
            && local.x >= geometry.size.w - inside
            && local.x <= geometry.size.w + outside,
        top: in_horizontal_span && local.y >= -outside && local.y <= inside,
        bottom: in_horizontal_span
            && local.y >= geometry.size.h - inside
            && local.y <= geometry.size.h + outside,
    };
    (edges.left || edges.right || edges.top || edges.bottom).then_some(edges)
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
    click_sequence_matches(
        first.position,
        Some(first.id),
        first.deadline,
        position,
        window,
        now,
    )
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
    first.button == button
        && click_sequence_matches(
            first.position,
            first.window,
            first.deadline,
            position,
            window,
            now,
        )
}

fn click_sequence_matches(
    first_position: smithay::utils::Point<f64, smithay::utils::Logical>,
    first_window: Option<mio_core::WindowId>,
    deadline: Instant,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    window: Option<mio_core::WindowId>,
    now: Instant,
) -> bool {
    first_window.is_some()
        && first_window == window
        && now <= deadline
        && !click_moved_too_far(first_position, position)
}

fn click_moved_too_far(
    first: smithay::utils::Point<f64, smithay::utils::Logical>,
    current: smithay::utils::Point<f64, smithay::utils::Logical>,
) -> bool {
    let delta = current - first;
    delta.x.hypot(delta.y) > DOUBLE_CLICK_DISTANCE
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

pub(crate) fn camera_zoom_from_scroll(current: f64, vertical_scroll: f64) -> f64 {
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
    if !matches!(
        source,
        AxisSource::Continuous | AxisSource::Wheel | AxisSource::WheelTilt
    ) {
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

fn normalized_axis_amount(amount: Option<f64>, amount_v120: Option<f64>) -> f64 {
    match amount {
        Some(value) if value != 0.0 => value,
        _ => amount_v120.unwrap_or(0.0) * 15.0 / 120.0,
    }
}

fn point_in_center_region(
    point: smithay::utils::Point<f64, smithay::utils::Logical>,
    rect: ScreenRect,
    fraction: f64,
) -> bool {
    let fraction = fraction.clamp(0.0, 1.0);
    let horizontal_margin = f64::from(rect.width) * (1.0 - fraction) / 2.0;
    let vertical_margin = f64::from(rect.height) * (1.0 - fraction) / 2.0;
    let left = f64::from(rect.x) + horizontal_margin;
    let right = f64::from(rect.x + rect.width) - horizontal_margin;
    let top = f64::from(rect.y) + vertical_margin;
    let bottom = f64::from(rect.y + rect.height) - vertical_margin;
    point.x >= left && point.x <= right && point.y >= top && point.y <= bottom
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
    (symbol.raw() != 0).then(|| crate::config::key_from_keysym_raw(symbol.raw()))
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
    fn resize_region_uses_the_border_and_outside_not_client_content() {
        let geometry = Rectangle::new((10.0, 20.0).into(), (100.0, 80.0).into());
        assert_eq!(
            resize_edges_at((9.0, 60.0).into(), geometry, 1.0, 8.0),
            Some(ResizeEdges {
                left: true,
                right: false,
                top: false,
                bottom: false,
            })
        );
        assert_eq!(
            resize_edges_at((110.0, 101.0).into(), geometry, 1.0, 8.0),
            Some(ResizeEdges {
                left: false,
                right: true,
                top: false,
                bottom: true,
            })
        );
        assert_eq!(
            resize_edges_at((14.0, 60.0).into(), geometry, 1.0, 8.0),
            None
        );
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
            Some(GridRect::new(-2, 3, 6, 5).unwrap())
        );
        assert_eq!(
            resized_rect_from_drag(rect, upper_left, 1, -2),
            Some(GridRect::new(-1, 1, 3, 7).unwrap())
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
            edge_wheel_focus_direction(AxisSource::Continuous, Some(Direction::Right), 0.0, 15.0),
            Some(Direction::Right)
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
    fn wheel_v120_amount_is_used_when_the_pixel_amount_is_zero() {
        assert_eq!(normalized_axis_amount(Some(0.0), Some(120.0)), 15.0);
        assert_eq!(normalized_axis_amount(None, Some(-120.0)), -15.0);
        assert_eq!(normalized_axis_amount(Some(7.5), Some(120.0)), 7.5);
        assert_eq!(normalized_axis_amount(Some(0.0), None), 0.0);
    }

    #[test]
    fn swap_candidate_uses_a_smaller_entry_region_and_larger_exit_region() {
        let rect = ScreenRect {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        };
        let center = smithay::utils::Point::from((200.0, 100.0));
        let hysteresis_only = smithay::utils::Point::from((135.0, 100.0));
        let outside = smithay::utils::Point::from((120.0, 100.0));

        assert!(point_in_center_region(center, rect, 0.60));
        assert!(!point_in_center_region(hysteresis_only, rect, 0.60));
        assert!(point_in_center_region(hysteresis_only, rect, 0.70));
        assert!(!point_in_center_region(outside, rect, 0.70));
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
    fn batched_wheel_zoom_matches_sequential_wheel_events() {
        let sequential = camera_zoom_from_scroll(
            camera_zoom_from_scroll(camera_zoom_from_scroll(1.0, 5.0), 4.0),
            6.0,
        );
        let batched = camera_zoom_from_scroll(1.0, 15.0);

        assert!((sequential - batched).abs() < f64::EPSILON);
    }

    #[test]
    fn opacity_toggle_switches_between_full_and_eighty_percent() {
        assert!((toggled_opacity(1.0, [1.0, 0.8]) - 0.8).abs() < f32::EPSILON);
        assert!((toggled_opacity(0.8, [1.0, 0.8]) - 1.0).abs() < f32::EPSILON);
        assert!((toggled_opacity(0.7, [0.95, 0.7]) - 0.95).abs() < f32::EPSILON);
    }
}
