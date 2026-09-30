use std::{ffi::OsStr, io};

use tracing::{info, warn};

use crate::state::MioState;

impl MioState {
    pub(crate) fn start_xwayland_satellite(
        &mut self,
        program: &OsStr,
        display_name: &str,
    ) -> io::Result<()> {
        validate_display(display_name)?;
        let child = std::process::Command::new(program)
            .arg(display_name)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .spawn()?;
        info!(program = ?program, x_display = %display_name, pid = child.id(), "xwayland-satellite started");
        self.xwayland_satellite = Some(child);
        self.xwayland_display = Some(display_name.to_owned());
        Ok(())
    }

    pub(crate) fn poll_xwayland_satellite(&mut self) {
        let Some(child) = self.xwayland_satellite.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                warn!(%status, display = ?self.xwayland_display, "xwayland-satellite exited; native Wayland remains available");
                self.xwayland_satellite = None;
                self.xwayland_display = None;
            }
            Ok(None) => {}
            Err(error) => warn!(%error, "failed to query xwayland-satellite status"),
        }
    }

    pub(crate) fn stop_xwayland_satellite(&mut self) {
        let Some(mut child) = self.xwayland_satellite.take() else {
            self.xwayland_display = None;
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            if let Err(error) = child.kill() {
                warn!(%error, "failed to stop xwayland-satellite");
            }
            let _ = child.wait();
        }
        self.xwayland_display = None;
    }
}

fn validate_display(display: &str) -> io::Result<()> {
    let Some(number) = display.strip_prefix(':') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "X display must have the form :NUMBER",
        ));
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "X display must have the form :NUMBER",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_display;

    #[test]
    fn accepts_numeric_x_display() {
        assert!(validate_display(":100").is_ok());
    }

    #[test]
    fn rejects_ambiguous_x_display() {
        assert!(validate_display("100").is_err());
        assert!(validate_display(":").is_err());
        assert!(validate_display(":abc").is_err());
    }
}
