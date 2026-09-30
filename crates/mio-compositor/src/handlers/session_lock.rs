use smithay::{
    delegate_session_lock,
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::{Physical, Size, SERIAL_COUNTER},
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
};

use crate::state::MioState;

impl SessionLockHandler for MioState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        self.session_locked = true;
        self.pending_session_lock = Some(confirmation);
        self.session_lock_surfaces.clear();
        self.cursor_image_status = CursorImageStatus::default_named();
        self.cursor_override = None;
        self.pending_camera_drag = None;
        self.pending_window_drag = None;
        self.pending_window_resize = None;
        self.pending_floating_chord = None;
        self.pending_edge_placement = None;
        self.pointer_buttons_held = 0;
        self.pending_close_click = None;
        self.closing_pointer_chord = false;
        self.pending_pointer_click = None;
        self.suppressed_window_drag_releases = 0;
        self.suppressed_focus_click = false;
        self.dnd_icon = None;
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(pointer) = self
            .seat
            .get_pointer()
            .filter(smithay::input::pointer::PointerHandle::is_grabbed)
        {
            pointer.unset_grab(self, serial, 0);
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.is_grabbed() {
                keyboard.unset_grab(self);
            }
            keyboard.set_focus(self, None, serial);
        }
        if let Some(touch) = self
            .seat
            .get_touch()
            .filter(smithay::input::touch::TouchHandle::is_grabbed)
        {
            touch.unset_grab(self);
        }
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }

    fn unlock(&mut self) {
        self.session_locked = false;
        self.pending_session_lock = None;
        self.session_lock_surfaces.clear();
        if let Some(id) = self.world.focused() {
            self.activate_window(id, SERIAL_COUNTER.next_serial());
        }
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            return;
        };
        if let Some(mode) = output.current_mode() {
            let size =
                session_lock_logical_size(mode.size, output.current_scale().fractional_scale());
            surface.with_pending_state(|state| {
                state.size = Some(size);
            });
            surface.send_configure();
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(
                self,
                Some(surface.wl_surface().clone()),
                SERIAL_COUNTER.next_serial(),
            );
        }
        self.session_lock_surfaces.push((output, surface));
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }
}

fn session_lock_logical_size(
    size: Size<i32, Physical>,
    output_scale: f64,
) -> Size<u32, smithay::utils::Logical> {
    let size: Size<i32, smithay::utils::Logical> =
        size.to_f64().to_logical(output_scale).to_i32_round();
    Size::from((
        u32::try_from(size.w.max(0)).unwrap_or_default(),
        u32::try_from(size.h.max(0)).unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Physical, Size};

    use super::session_lock_logical_size;

    #[test]
    fn session_lock_configure_size_follows_fractional_output_scale() {
        assert_eq!(
            session_lock_logical_size(Size::<i32, Physical>::from((1920, 1080)), 1.25),
            (1536, 864).into()
        );
        assert_eq!(
            session_lock_logical_size(Size::<i32, Physical>::from((1920, 1080)), 1.0),
            (1920, 1080).into()
        );
    }
}

delegate_session_lock!(MioState);
