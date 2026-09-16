mod activation;
mod compositor;
mod layer_shell;
mod session_lock;
mod xdg_shell;

use smithay::{
    delegate_alpha_modifier, delegate_content_type, delegate_cursor_shape, delegate_data_control,
    delegate_data_device, delegate_dmabuf,
    delegate_fractional_scale, delegate_idle_inhibit,
    delegate_input_method_manager, delegate_keyboard_shortcuts_inhibit, delegate_output,
    delegate_pointer_constraints, delegate_presentation, delegate_primary_selection,
    delegate_relative_pointer, delegate_seat,
    delegate_single_pixel_buffer, delegate_text_input_manager, delegate_viewporter,
    delegate_virtual_keyboard_manager, delegate_xdg_activation, delegate_xdg_decoration,
    delegate_xdg_foreign, delegate_xdg_toplevel_icon,
    desktop::{PopupKind, PopupManager, WindowSurfaceType},
    input::{
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::{CursorImageStatus, Focus, PointerHandle},
        Seat, SeatHandler, SeatState,
    },
    reexports::wayland_server::{
        protocol::wl_surface::WlSurface,
        Resource,
    },
    utils::{Point, Serial},
    wayland::{
        compositor::with_states,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        fractional_scale::{with_fractional_scale, FractionalScaleHandler},
        idle_inhibit::IdleInhibitHandler,
        input_method::{InputMethodHandler, PopupSurface},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
        output::OutputHandler,
        pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler},
        seat::WaylandFocus,
        selection::{
            data_device::{
                set_data_device_focus, DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
            },
            primary_selection::{
                set_primary_focus, PrimarySelectionHandler, PrimarySelectionState,
            },
            wlr_data_control::{DataControlHandler, DataControlState},
            SelectionHandler,
        },
        shell::xdg::{decoration::XdgDecorationHandler, ToplevelSurface},
        tablet_manager::TabletSeatHandler,
        xdg_toplevel_icon::XdgToplevelIconHandler,
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;

use crate::state::{DndIcon, MioState};
use tracing::info;

impl SeatHandler for MioState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_image_status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client.clone());
        set_primary_focus(&self.display_handle, seat, client);
        self.sync_shortcut_inhibitors(focused);
    }
}

delegate_seat!(MioState);
impl TabletSeatHandler for MioState {}
delegate_cursor_shape!(MioState);

impl SelectionHandler for MioState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for MioState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for MioState {
    fn dropped(
        &mut self,
        _target: Option<smithay::input::dnd::DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, smithay::utils::Logical>,
    ) {
        self.dnd_icon = None;
    }
}

impl WaylandDndGrabHandler for MioState {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        grab_type: GrabType,
    ) {
        self.dnd_icon = icon.map(|surface| DndIcon {
            surface,
            offset: Point::default(),
        });

        match grab_type {
            GrabType::Pointer => {
                let Some(pointer) = seat.get_pointer() else {
                    source.cancel();
                    self.dnd_icon = None;
                    return;
                };
                let Some(start_data) = pointer.grab_start_data() else {
                    source.cancel();
                    self.dnd_icon = None;
                    return;
                };
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                let Some(touch) = seat.get_touch() else {
                    source.cancel();
                    self.dnd_icon = None;
                    return;
                };
                let Some(start_data) = touch.grab_start_data() else {
                    source.cancel();
                    self.dnd_icon = None;
                    return;
                };
                let grab = DnDGrab::new_touch(&self.display_handle, start_data, source, seat);
                touch.set_grab(self, grab, serial);
            }
        }
    }
}
delegate_data_device!(MioState);

impl PrimarySelectionHandler for MioState {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

delegate_primary_selection!(MioState);

impl DataControlHandler for MioState {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

delegate_data_control!(MioState);

impl IdleInhibitHandler for MioState {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibiting_surfaces.push(surface);
        info!(
            count = self.idle_inhibiting_surfaces.len(),
            "idle inhibited"
        );
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        if let Some(index) = self
            .idle_inhibiting_surfaces
            .iter()
            .position(|candidate| candidate == &surface)
        {
            self.idle_inhibiting_surfaces.swap_remove(index);
        }
        info!(
            count = self.idle_inhibiting_surfaces.len(),
            "idle inhibition updated"
        );
    }
}

delegate_idle_inhibit!(MioState);
delegate_viewporter!(MioState);

impl FractionalScaleHandler for MioState {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let Some(scale) = self
            .space
            .outputs()
            .next()
            .map(|output| output.current_scale().fractional_scale())
        else {
            return;
        };
        with_states(&surface, |states| {
            with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(scale);
            });
        });
    }
}

delegate_fractional_scale!(MioState);

delegate_text_input_manager!(MioState);

impl InputMethodHandler for MioState {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(error) = self.popups.track_popup(PopupKind::InputMethod(surface)) {
            tracing::warn!(%error, "failed to track input-method popup");
        }
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            if let Err(error) =
                PopupManager::dismiss_popup(&parent, &PopupKind::InputMethod(surface))
            {
                tracing::warn!(%error, "failed to dismiss input-method popup");
            }
        }
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {}

    fn parent_geometry(
        &self,
        parent: &WlSurface,
    ) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        self.space
            .elements()
            .find_map(|window| {
                (window.wl_surface().as_deref() == Some(parent)).then(|| window.geometry())
            })
            .or_else(|| {
                self.space.outputs().find_map(|output| {
                    let map = smithay::desktop::layer_map_for_output(output);
                    let layer = map.layer_for_surface(parent, WindowSurfaceType::ALL)?;
                    map.layer_geometry(layer)
                })
            })
            .unwrap_or_default()
    }
}

delegate_input_method_manager!(MioState);
delegate_virtual_keyboard_manager!(MioState);

impl KeyboardShortcutsInhibitHandler for MioState {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        let focused = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        if focused.as_ref() == Some(inhibitor.wl_surface()) {
            inhibitor.activate();
        } else {
            inhibitor.inactivate();
        }
        self.keyboard_shortcut_inhibitors.push(inhibitor);
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.keyboard_shortcut_inhibitors
            .retain(|candidate| candidate != &inhibitor);
    }
}

delegate_keyboard_shortcuts_inhibit!(MioState);
delegate_relative_pointer!(MioState);

impl PointerConstraintsHandler for MioState {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        let active = with_pointer_constraint(surface, pointer, |constraint| {
            constraint.is_some_and(|constraint| constraint.is_active())
        });
        if !active {
            return;
        }
        if let Some((current, origin)) = self.surface_under(pointer.current_location()) {
            if current == *surface {
                pointer.set_location(origin + location);
            }
        }
    }
}

delegate_pointer_constraints!(MioState);

impl DmabufHandler for MioState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        self.pending_dmabuf_imports.push((dmabuf, notifier));
    }
}

delegate_dmabuf!(MioState);
delegate_single_pixel_buffer!(MioState);
delegate_alpha_modifier!(MioState);
delegate_content_type!(MioState);

impl MioState {
    fn sync_shortcut_inhibitors(&self, focused: Option<&WlSurface>) {
        for inhibitor in &self.keyboard_shortcut_inhibitors {
            let should_be_active = focused == Some(inhibitor.wl_surface());
            if should_be_active && !inhibitor.is_active() {
                inhibitor.activate();
            } else if !should_be_active && inhibitor.is_active() {
                inhibitor.inactivate();
            }
        }
    }

    pub fn cleanup_idle_inhibitors(&mut self) {
        let previous = self.idle_inhibiting_surfaces.len();
        self.idle_inhibiting_surfaces.retain(Resource::is_alive);
        if self.idle_inhibiting_surfaces.len() != previous {
            info!(
                count = self.idle_inhibiting_surfaces.len(),
                "dead idle inhibitors removed"
            );
        }
    }
}

impl OutputHandler for MioState {}
delegate_output!(MioState);
delegate_xdg_activation!(MioState);

impl XdgDecorationHandler for MioState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        set_mio_decoration(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        set_mio_decoration(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        set_mio_decoration(&toplevel);
    }
}

fn set_mio_decoration(toplevel: &ToplevelSurface) {
    toplevel.with_pending_state(|state| {
        state.decoration_mode = Some(mio_decoration_mode());
    });
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}

const fn mio_decoration_mode() -> DecorationMode {
    DecorationMode::ServerSide
}

delegate_xdg_decoration!(MioState);
delegate_presentation!(MioState);
impl XdgToplevelIconHandler for MioState {}
delegate_xdg_toplevel_icon!(MioState);

impl XdgForeignHandler for MioState {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}

delegate_xdg_foreign!(MioState);

#[cfg(test)]
mod decoration_tests {
    use super::*;

    #[test]
    fn mio_uses_its_minimal_server_side_decoration() {
        assert_eq!(mio_decoration_mode(), DecorationMode::ServerSide);
    }
}
