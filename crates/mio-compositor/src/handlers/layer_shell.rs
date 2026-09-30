use smithay::{
    delegate_layer_shell,
    desktop::{layer_map_for_output, LayerSurface, PopupKind},
    output::Output,
    reexports::wayland_server::protocol::{wl_output, wl_surface::WlSurface},
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState,
            LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
        },
    },
};
use tracing::warn;

use crate::state::MioState;

impl WlrLayerShellHandler for MioState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            warn!(%namespace, "layer surface created before an output was available");
            return;
        };
        let layer = LayerSurface::new(surface, namespace);
        if let Err(error) = layer_map_for_output(&output).map_layer(&layer) {
            warn!(%error, "failed to map layer surface");
        };
    }

    fn new_popup(
        &mut self,
        _parent: WlrLayerSurface,
        popup: smithay::wayland::shell::xdg::PopupSurface,
    ) {
        self.unconstrain_popup(&popup);
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(popup)) {
            warn!(%error, "failed to track layer-shell popup");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        self.layer_keyboard_interactivity
            .remove(surface.wl_surface());
        let had_keyboard_focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focused| focused == *surface.wl_surface());
        self.hidden_fullscreen_layers
            .retain(|(_, layer)| layer.layer_surface() != &surface);
        let removed = {
            self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                let layer = map
                    .layers()
                    .find(|layer| layer.layer_surface() == &surface)
                    .cloned()
                    .map(|layer| (output.clone(), layer));
                layer
            })
        };
        if let Some((output, layer)) = removed {
            layer_map_for_output(&output).unmap_layer(&layer);
            self.sync_layout(false);
        }
        if had_keyboard_focus {
            if let Some(focused) = self.world.focused() {
                self.activate_window(focused, SERIAL_COUNTER.next_serial());
            } else if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(
                    self,
                    Option::<WlSurface>::None,
                    SERIAL_COUNTER.next_serial(),
                );
            }
        }
    }
}

delegate_layer_shell!(MioState);

pub fn handle_commit(state: &mut MioState, surface: &WlSurface) {
    let Some(output) = state
        .space
        .outputs()
        .find(|output| {
            layer_map_for_output(output)
                .layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
        .cloned()
    else {
        return;
    };
    let mut map = layer_map_for_output(&output);
    map.arrange();
    let (focused_layer, release_layer_focus) = if let Some(layer) = map
        .layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL)
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .and_then(|data| data.lock().ok())
                .is_some_and(|attributes| attributes.initial_configure_sent)
        });
        if initial_configure_sent {
            layer.layer_surface().send_pending_configure();
        } else {
            layer.layer_surface().send_configure();
        }
        let pending_state = with_states(surface, |states| {
            *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .pending()
        });
        let previous_interactivity = state
            .layer_keyboard_interactivity
            .insert(surface.clone(), pending_state.keyboard_interactivity)
            .unwrap_or(KeyboardInteractivity::None);
        let release_focus = pending_state.keyboard_interactivity == KeyboardInteractivity::None
            && state
                .seat
                .get_keyboard()
                .and_then(|keyboard| keyboard.current_focus())
                .is_some_and(|focused| focused == *surface);
        (
            should_focus_on_commit(previous_interactivity, pending_state).then_some(layer),
            release_focus,
        )
    } else {
        (None, false)
    };
    drop(map);
    if let (Some(layer), Some(keyboard)) = (focused_layer, state.seat.get_keyboard()) {
        keyboard.set_focus(
            state,
            Some(layer.wl_surface().clone()),
            SERIAL_COUNTER.next_serial(),
        );
    } else if release_layer_focus {
        if let Some(focused) = state.world.focused() {
            state.activate_window(focused, SERIAL_COUNTER.next_serial());
        } else if let Some(keyboard) = state.seat.get_keyboard() {
            keyboard.set_focus(
                state,
                Option::<WlSurface>::None,
                SERIAL_COUNTER.next_serial(),
            );
        }
    }
    state.sync_layout(false);
}

fn should_focus_on_commit(
    previous: KeyboardInteractivity,
    pending: LayerSurfaceCachedState,
) -> bool {
    matches!(pending.layer, Layer::Top | Layer::Overlay)
        && (pending.keyboard_interactivity == KeyboardInteractivity::Exclusive
            || (previous != KeyboardInteractivity::OnDemand
                && pending.keyboard_interactivity == KeyboardInteractivity::OnDemand))
}

#[cfg(test)]
mod tests {
    use smithay::wayland::shell::wlr_layer::{
        KeyboardInteractivity, Layer, LayerSurfaceCachedState,
    };

    use super::should_focus_on_commit;

    #[test]
    fn upper_layers_take_focus_for_exclusive_or_new_on_demand_requests() {
        for layer in [Layer::Top, Layer::Overlay] {
            let state = LayerSurfaceCachedState {
                layer,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                ..Default::default()
            };
            assert!(should_focus_on_commit(KeyboardInteractivity::None, state));
        }

        for keyboard_interactivity in [KeyboardInteractivity::None, KeyboardInteractivity::OnDemand]
        {
            let state = LayerSurfaceCachedState {
                layer: Layer::Top,
                keyboard_interactivity,
                ..Default::default()
            };
            assert!(!should_focus_on_commit(keyboard_interactivity, state));
        }

        let state = LayerSurfaceCachedState {
            layer: Layer::Top,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        };
        assert!(should_focus_on_commit(KeyboardInteractivity::None, state));

        let current = LayerSurfaceCachedState {
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        };
        let pending = LayerSurfaceCachedState {
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..current
        };
        assert!(should_focus_on_commit(
            current.keyboard_interactivity,
            pending
        ));
        assert!(!should_focus_on_commit(
            pending.keyboard_interactivity,
            pending
        ));

        let state = LayerSurfaceCachedState {
            layer: Layer::Bottom,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        };
        assert!(!should_focus_on_commit(
            KeyboardInteractivity::Exclusive,
            state
        ));
    }
}
