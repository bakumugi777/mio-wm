use std::time::{Duration, Instant};

use mio_core::{Action, Presentation, WindowProperty, WindowPropertyKind};
use smithay::{
    delegate_xdg_dialog, delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy,
    },
    input::{pointer::Focus, Seat},
    reexports::wayland_server::protocol::{wl_output, wl_seat, wl_surface::WlSurface},
    utils::{Scale, Serial, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        shell::xdg::{
            dialog::{ToplevelDialogHint, XdgDialogHandler},
            PopupSurface, PositionerState, ShellClient, SurfaceCachedState, ToplevelSurface,
            XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    config::WindowRule,
    state::{MioState, XdgClientPing},
};

const CLIENT_PING_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_PING_TIMEOUT: Duration = Duration::from_secs(10);

impl XdgShellHandler for MioState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.add_toplevel(surface);
    }

    fn new_client(&mut self, client: ShellClient) {
        self.xdg_clients.push(XdgClientPing::new(
            client,
            Instant::now(),
            CLIENT_PING_INTERVAL,
        ));
    }

    fn client_pong(&mut self, client: ShellClient) {
        if let Some(tracker) = self
            .xdg_clients
            .iter_mut()
            .find(|tracker| tracker.client == client)
        {
            tracker.deadline = None;
            tracker.timed_out = false;
            tracker.next_ping = Instant::now() + CLIENT_PING_INTERVAL;
        }
    }

    fn client_destroyed(&mut self, client: ShellClient) {
        self.xdg_clients.retain(|tracker| tracker.client != client);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.remove_toplevel(&surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!(%error, "failed to track xdg popup");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::from_resource(&seat) else {
            return;
        };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };

        let known_root = self.managed_windows.iter().any(|managed| {
            managed
                .window
                .wl_surface()
                .is_some_and(|candidate| candidate.as_ref() == &root)
        }) || self.space.outputs().any(|output| {
            layer_map_for_output(output)
                .layer_for_surface(&root, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                .is_some()
        });
        if !known_root {
            return;
        }

        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else {
            return;
        };

        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        self.set_requested_presentation(&surface, Presentation::Fullscreen, true);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.set_requested_presentation(&surface, Presentation::Fullscreen, false);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.set_requested_presentation(&surface, Presentation::Maximized, true);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.set_requested_presentation(&surface, Presentation::Maximized, false);
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.apply_window_rules(&surface);
        self.refresh_auto_floating(&surface);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.apply_window_rules(&surface);
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        self.update_toplevel_parent(&surface);
        self.refresh_auto_floating(&surface);
    }
}

delegate_xdg_shell!(MioState);

impl XdgDialogHandler for MioState {
    fn dialog_hint_changed(&mut self, toplevel: ToplevelSurface, _hint: ToplevelDialogHint) {
        self.refresh_auto_floating(&toplevel);
    }
}

impl MioState {
    fn refresh_auto_floating(&mut self, toplevel: &ToplevelSurface) {
        let Some(id) = self.window_id_for_surface(toplevel.wl_surface()) else {
            return;
        };
        let has_parent = toplevel.parent().is_some();
        let is_modal = toplevel_is_modal(toplevel);
        let is_fixed_size = toplevel_is_fixed_size(toplevel);
        let should_float = dialog_should_float([has_parent, is_modal, is_fixed_size]);
        let anchor = should_float
            .then(|| self.automatic_floating_anchor(id))
            .flatten();
        let Some(managed) = self
            .managed_windows
            .iter_mut()
            .find(|managed| managed.id == id)
        else {
            return;
        };
        managed.preserve_committed_size = is_fixed_size;
        if managed.auto_floating == should_float {
            return;
        }
        if let Err(error) = self.world.apply(dialog_floating_action(id, should_float)) {
            tracing::warn!(%error, "failed to update automatic floating property");
            return;
        }
        managed.auto_floating = should_float;
        if let Some(origin) = anchor {
            if let Err(error) = self
                .world
                .apply(Action::MoveWindowContinuous { id, origin })
            {
                tracing::warn!(%error, "failed to place automatic floating Window over its parent");
            } else if self.world.focused() == Some(id) {
                if let Err(error) = self.world.apply(Action::CameraCenter(id)) {
                    tracing::warn!(%error, "failed to center Camera after automatic floating placement");
                }
                self.reset_all_window_presentations();
            } else {
                self.reset_window_presentation(id);
            }
        }
        tracing::debug!(
            window_id = ?id,
            has_parent,
            is_modal,
            is_fixed_size,
            should_float,
            "updated automatic floating"
        );
        self.sync_layout(false);
    }
}

delegate_xdg_dialog!(MioState);

impl MioState {
    pub(crate) fn poll_xdg_clients(&mut self, now: Instant) {
        self.xdg_clients.retain(|tracker| tracker.client.alive());
        for tracker in &mut self.xdg_clients {
            match ping_due(now, tracker.next_ping, tracker.deadline, tracker.timed_out) {
                PingDue::None => {}
                PingDue::Send => {
                    if let Err(error) = tracker.client.send_ping(SERIAL_COUNTER.next_serial()) {
                        tracing::warn!(%error, "failed to ping xdg-shell client");
                    } else {
                        tracker.deadline = Some(now + CLIENT_PING_TIMEOUT);
                    }
                    tracker.next_ping = now + CLIENT_PING_INTERVAL;
                }
                PingDue::Timeout => {
                    tracing::warn!(
                        "xdg-shell client did not respond to ping; keeping it connected"
                    );
                    tracker.timed_out = true;
                }
            }
        }
    }

    pub(crate) fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            tracing::debug!("xdg popup constraint skipped: root surface unavailable");
            return;
        };
        let popup_offset = get_popup_toplevel_coords(&kind);

        let root_geometry = self.managed_windows.iter().find_map(|managed| {
            let toplevel = managed.window.toplevel()?;
            (toplevel.wl_surface() == &root).then(|| {
                let window_geometry = self.space.element_geometry(&managed.window)?;
                let output = self
                    .space
                    .outputs_for_element(&managed.window)
                    .pop()
                    .or_else(|| {
                        self.space
                            .outputs()
                            .filter_map(|output| {
                                let geometry = self.space.output_geometry(output)?;
                                let overlap = rectangle_overlap_area(geometry, window_geometry);
                                (overlap > 0).then(|| (overlap, output.clone()))
                            })
                            .max_by_key(|(overlap, _)| *overlap)
                            .map(|(_, output)| output)
                    })?;
                let output_geometry = self.space.output_geometry(&output)?;
                Some((output_geometry, window_geometry, managed.window.scale()))
            })?
        });
        let root_geometry = root_geometry.or_else(|| {
            self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                let layer =
                    map.layer_for_surface(&root, smithay::desktop::WindowSurfaceType::TOPLEVEL)?;
                let mut layer_geometry = map.layer_geometry(layer)?;
                let output_geometry = self.space.output_geometry(output)?;
                layer_geometry.loc += output_geometry.loc;
                Some((output_geometry, layer_geometry, Scale::from(1.0)))
            })
        });
        let Some((target, root_geometry, scale)) = root_geometry else {
            tracing::debug!("xdg popup constraint skipped: root geometry unavailable");
            return;
        };
        let target = popup_constraint_target(target, root_geometry, scale, popup_offset);
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }

    pub(crate) fn reapply_window_rules(&mut self) {
        let surfaces = self
            .managed_windows
            .iter()
            .filter_map(|managed| managed.window.toplevel().cloned())
            .collect::<Vec<_>>();
        for surface in surfaces {
            self.apply_window_rules(&surface);
        }
    }

    fn apply_window_rules(&mut self, surface: &ToplevelSurface) {
        let metadata = with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| {
                    let data = data.lock().ok()?;
                    Some((data.app_id.clone(), data.title.clone()))
                })
        });
        let Some((app_id, title)) = metadata else {
            return;
        };
        let properties = configured_window_properties(
            self.config.config().appearance.opacity,
            &self.config.config().window_rules,
            app_id.as_deref(),
            title.as_deref(),
        );
        let Some(id) = self.window_id_for_surface(surface.wl_surface()) else {
            return;
        };
        if self
            .world
            .replace_config_window_properties(id, &properties)
            .is_ok()
        {
            self.sync_layout(false);
        }
    }

    fn set_requested_presentation(
        &mut self,
        surface: &ToplevelSurface,
        requested: Presentation,
        enabled: bool,
    ) {
        let Some(id) = self.window_id_for_surface(surface.wl_surface()) else {
            return;
        };
        if !accept_client_presentation_request(requested) {
            tracing::debug!(
                window = id.get(),
                ?requested,
                enabled,
                "ignored client presentation request; use a Mio Action instead"
            );
            // Re-send Mio's authoritative state so applications restoring an
            // old maximized state converge back to the tiled presentation.
            self.sync_layout(true);
            return;
        }
        let Some(current) = self.world.window(id).map(mio_core::Window::presentation) else {
            return;
        };
        if enabled == (current == requested) {
            return;
        }
        let result = match requested {
            Presentation::Fullscreen => self.world.apply(Action::ToggleFullscreen(id)),
            Presentation::Maximized => self.world.apply(Action::ToggleMaximized(id)),
            Presentation::Normal => return,
        };
        if result.is_ok() {
            self.sync_layout(true);
        }
    }
}

const fn accept_client_presentation_request(requested: Presentation) -> bool {
    matches!(requested, Presentation::Fullscreen)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PingDue {
    None,
    Send,
    Timeout,
}

fn dialog_should_float(reasons: [bool; 3]) -> bool {
    reasons.into_iter().any(|reason| reason)
}

fn toplevel_is_modal(toplevel: &ToplevelSurface) -> bool {
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok())
            .is_some_and(|data| matches!(data.dialog_hint, ToplevelDialogHint::Modal))
    })
}

fn toplevel_is_fixed_size(toplevel: &ToplevelSurface) -> bool {
    with_states(toplevel.wl_surface(), |states| {
        let mut guard = states.cached_state.get::<SurfaceCachedState>();
        let state = guard.current();
        fixed_size_constraints(
            (state.min_size.w, state.min_size.h),
            (state.max_size.w, state.max_size.h),
        )
    })
}

fn fixed_size_constraints(min: (i32, i32), max: (i32, i32)) -> bool {
    min.0 > 0 && min.1 > 0 && min == max
}

fn popup_constraint_target(
    mut target: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    root_geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    scale: Scale<f64>,
    popup_offset: smithay::utils::Point<i32, smithay::utils::Logical>,
) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
    target.loc -= root_geometry.loc;
    let mut target = target
        .to_f64()
        .upscale((1.0 / scale.x, 1.0 / scale.y))
        .to_i32_round();
    target.loc -= popup_offset;
    target
}

fn rectangle_overlap_area(
    first: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    second: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
) -> i64 {
    first.intersection(second).map_or(0, |intersection| {
        i64::from(intersection.size.w) * i64::from(intersection.size.h)
    })
}

fn dialog_floating_action(id: mio_core::WindowId, should_float: bool) -> Action {
    if should_float {
        Action::SetWindowProperty {
            id,
            property: WindowProperty::Floating(true),
        }
    } else {
        Action::ClearWindowProperty {
            id,
            kind: WindowPropertyKind::Floating,
        }
    }
}

fn ping_due(
    now: Instant,
    next_ping: Instant,
    deadline: Option<Instant>,
    timed_out: bool,
) -> PingDue {
    if !timed_out && deadline.is_some_and(|deadline| now >= deadline) {
        PingDue::Timeout
    } else if deadline.is_none() && now >= next_ping {
        PingDue::Send
    } else {
        PingDue::None
    }
}

pub fn handle_commit(state: &mut MioState, surface: &WlSurface) {
    if let Some(toplevel) = state.managed_windows.iter().find_map(|managed| {
        managed
            .window
            .toplevel()
            .filter(|toplevel| toplevel.wl_surface() == surface)
            .cloned()
    }) {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("xdg toplevel state exists")
                .lock()
                .expect("xdg toplevel state lock is not poisoned")
                .initial_configure_sent
        });
        if !initial_configure_sent {
            toplevel.send_configure();
        }
        state.refresh_auto_floating(&toplevel);
        if let Some(id) = state.window_id_for_surface(toplevel.wl_surface()) {
            if state.should_defer_unclassified_toplevel(id) {
                state.focus_pending_toplevel(id);
                tracing::debug!(
                    window_id = ?id,
                    "deferring unclassified toplevel presentation until its next commit"
                );
            } else {
                state.present_toplevel(id);
            }
        }
    }

    state.popups.commit(surface);
    if let Some(PopupKind::Xdg(popup)) = state.popups.find_popup(surface) {
        if !popup.is_initial_configure_sent() {
            if let Err(error) = popup.send_configure() {
                tracing::warn!(%error, "failed to configure xdg popup");
            }
        }
    }
}

fn matching_window_rule_properties(
    rules: &[WindowRule],
    app_id: Option<&str>,
    title: Option<&str>,
) -> Vec<WindowProperty> {
    rules
        .iter()
        .filter(|rule| {
            rule.app_id
                .as_ref()
                .is_none_or(|expected| app_id == Some(expected.as_str()))
                && rule
                    .title
                    .as_ref()
                    .is_none_or(|expected| title == Some(expected.as_str()))
        })
        .flat_map(|rule| {
            [
                rule.opacity.map(WindowProperty::Opacity),
                rule.floating.map(WindowProperty::Floating),
                rule.blur.map(WindowProperty::Blur),
            ]
            .into_iter()
            .flatten()
        })
        .collect()
}

fn configured_window_properties(
    default_opacity: f32,
    rules: &[WindowRule],
    app_id: Option<&str>,
    title: Option<&str>,
) -> Vec<WindowProperty> {
    std::iter::once(WindowProperty::Opacity(default_opacity))
        .chain(matching_window_rule_properties(rules, app_id, title))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        accept_client_presentation_request, configured_window_properties, dialog_floating_action,
        dialog_should_float, fixed_size_constraints, matching_window_rule_properties, ping_due,
        rectangle_overlap_area, PingDue,
    };
    use crate::config::WindowRule;
    use mio_core::{Action, Presentation, WindowId, WindowProperty, WindowPropertyKind};
    use smithay::utils::{Logical, Rectangle};

    #[test]
    fn xdg_ping_schedule_distinguishes_send_wait_and_timeout() {
        let now = Instant::now();
        assert_eq!(ping_due(now, now, None, false), PingDue::Send);
        assert_eq!(
            ping_due(now, now, Some(now + Duration::from_secs(1)), false),
            PingDue::None
        );
        assert_eq!(ping_due(now, now, Some(now), false), PingDue::Timeout);
        assert_eq!(ping_due(now, now, Some(now), true), PingDue::None);
    }

    #[test]
    fn dialog_hints_use_the_shared_floating_property_actions() {
        let id = WindowId::from_u64(7);
        assert!(dialog_should_float([true, false, false]));
        assert!(dialog_should_float([false, true, false]));
        assert!(dialog_should_float([false, false, true]));
        assert!(!dialog_should_float([false, false, false]));
        assert_eq!(
            dialog_floating_action(id, true),
            Action::SetWindowProperty {
                id,
                property: WindowProperty::Floating(true),
            }
        );
        assert_eq!(
            dialog_floating_action(id, false),
            Action::ClearWindowProperty {
                id,
                kind: WindowPropertyKind::Floating,
            }
        );
    }

    #[test]
    fn fixed_size_requires_both_axes_to_have_equal_nonzero_limits() {
        assert!(fixed_size_constraints((640, 480), (640, 480)));
        assert!(!fixed_size_constraints((640, 0), (640, 0)));
        assert!(!fixed_size_constraints((640, 480), (800, 480)));
        assert!(!fixed_size_constraints((0, 0), (0, 0)));
    }

    #[test]
    fn popup_output_fallback_uses_actual_window_overlap() {
        let output = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let visible = Rectangle::<i32, Logical>::new((1800, 900).into(), (400, 300).into());
        let outside = Rectangle::<i32, Logical>::new((2000, 1200).into(), (400, 300).into());
        assert_eq!(rectangle_overlap_area(output, visible), 120 * 180);
        assert_eq!(rectangle_overlap_area(output, outside), 0);
    }

    #[test]
    fn clients_may_request_fullscreen_but_not_maximize() {
        assert!(accept_client_presentation_request(Presentation::Fullscreen));
        assert!(!accept_client_presentation_request(Presentation::Maximized));
        assert!(!accept_client_presentation_request(Presentation::Normal));
    }

    #[test]
    fn matching_window_rules_compose_in_file_order() {
        let rules = [
            WindowRule {
                app_id: Some("foot".into()),
                title: None,
                opacity: Some(0.8),
                floating: Some(true),
                blur: None,
            },
            WindowRule {
                app_id: Some("foot".into()),
                title: Some("main".into()),
                opacity: Some(0.6),
                floating: None,
                blur: Some(true),
            },
        ];

        assert_eq!(
            matching_window_rule_properties(&rules, Some("foot"), Some("main")),
            [
                WindowProperty::Opacity(0.8),
                WindowProperty::Floating(true),
                WindowProperty::Opacity(0.6),
                WindowProperty::Blur(true),
            ]
        );
        assert_eq!(
            matching_window_rule_properties(&rules, Some("foot"), Some("other")),
            [WindowProperty::Opacity(0.8), WindowProperty::Floating(true),]
        );
    }

    #[test]
    fn configured_default_opacity_is_overridden_by_matching_rule() {
        let rules = [WindowRule {
            app_id: Some("foot".into()),
            title: None,
            opacity: Some(1.0),
            floating: None,
            blur: None,
        }];

        assert_eq!(
            configured_window_properties(0.75, &rules, Some("firefox"), None),
            [WindowProperty::Opacity(0.75)]
        );
        assert_eq!(
            configured_window_properties(0.75, &rules, Some("foot"), None),
            [WindowProperty::Opacity(0.75), WindowProperty::Opacity(1.0)]
        );
    }
}
