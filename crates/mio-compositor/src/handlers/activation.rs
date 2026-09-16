use std::time::Duration;

use smithay::{
    input::Seat,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
    wayland::xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};
use tracing::{debug, info};

use crate::state::MioState;

const TOKEN_MAX_AGE: Duration = Duration::from_secs(10);

impl XdgActivationHandler for MioState {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let Some((serial, seat_resource)) = data.serial else {
            return false;
        };
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        Seat::from_resource(&seat_resource) == Some(self.seat.clone())
            && keyboard
                .last_enter()
                .is_some_and(|last_enter| serial.is_no_older_than(&last_enter))
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.xdg_activation_state.remove_token(&token);
        if !activation_is_fresh(token_data.timestamp.elapsed()) {
            debug!("ignored expired XDG activation token");
            return;
        }
        let Some(id) = self.window_id_for_surface(&surface) else {
            debug!("ignored XDG activation for unmanaged surface");
            return;
        };
        self.activate_window(id, SERIAL_COUNTER.next_serial());
        info!(window = id.get(), "XDG activation focused Window");
    }
}

fn activation_is_fresh(age: Duration) -> bool {
    age < TOKEN_MAX_AGE
}

#[cfg(test)]
mod tests {
    use super::{activation_is_fresh, TOKEN_MAX_AGE};
    use std::time::Duration;

    #[test]
    fn activation_tokens_expire_at_policy_boundary() {
        assert!(activation_is_fresh(Duration::from_secs(9)));
        assert!(!activation_is_fresh(TOKEN_MAX_AGE));
    }
}
