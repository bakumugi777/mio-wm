//! Minimal nested Smithay compositor.

use std::{error::Error, process::ExitCode};
#[cfg(test)]
use std::{ffi::OsStr, process::Command};

use calloop::{
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod animation;
mod config;
mod effects;
mod handlers;
mod image_copy_capture;
mod input;
mod ipc;
mod layout;
mod screencopy;
mod state;
mod udev;
mod winit;
mod xwayland;

use config::ConfigManager;
use state::MioState;

type MainResult<T = ()> = Result<T, Box<dyn Error>>;

pub struct CalloopData {
    state: MioState,
    display_handle: DisplayHandle,
}

fn main() -> ExitCode {
    if let Err(error) = run() {
        eprintln!("mio-compositor: {error}");
        let mut source = error.source();
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

#[allow(clippy::too_many_lines)]
fn run() -> MainResult {
    init_logging()?;

    let options = match Options::parse()? {
        ParsedOptions::Run(options) => options,
        ParsedOptions::Help => {
            print!("{}", compositor_usage());
            return Ok(());
        }
        ParsedOptions::Version => {
            println!("mio-compositor {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    };
    if options.check_config {
        let config = ConfigManager::load(options.config_path, options.explicit_config)?;
        println!("configuration is valid: {}", config.display_path());
        return Ok(());
    }
    let (config, config_error) =
        ConfigManager::load_or_default(options.config_path, options.explicit_config);
    let startup_config_error = config_error.map(|error| {
        tracing::error!(
            %error,
            path = %config.display_path(),
            "configuration load failed; starting with built-in defaults; fix the file and reload"
        );
        error.to_string()
    });

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    event_loop.handle().insert_source(
        Signals::new(&[Signal::SIGINT, Signal::SIGTERM])?,
        |event, (), data| {
            info!(signal = ?event.signal(), "shutdown signal received");
            data.state.loop_signal.stop();
        },
    )?;
    let display: Display<MioState> = Display::new()?;
    let display_handle = display.handle();
    let mut state = MioState::new(&mut event_loop, display, config)?;
    state.config_error = startup_config_error;

    let mut data = CalloopData {
        state,
        display_handle,
    };
    data.state.backend_name = options.backend.name();

    let viewport = data.state.config.config().viewport;
    for index in 1..options.virtual_outputs {
        let x = i64::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(i64::try_from(viewport.width()).ok()?))
            .ok_or("--virtual-outputs position exceeds the coordinate model")?;
        data.state
            .world
            .add_output(mio_core::Camera::new(
                mio_core::GridPoint::new(x, 0),
                viewport,
            ))
            .map_err(|error| error.to_string())?;
    }

    match options.backend {
        BackendKind::Winit => winit::init(&mut event_loop, &mut data)?,
        BackendKind::Udev => {
            if options.virtual_outputs != 1 {
                return Err("--virtual-outputs is available only with --backend winit".into());
            }
            udev::init(&mut event_loop, &mut data)?;
        }
    }
    data.state.init_ipc(&mut event_loop)?;
    event_loop.handle().insert_source(
        Timer::from_duration(std::time::Duration::from_secs(1)),
        |_, &mut (), data| {
            data.state.poll_xwayland_satellite();
            data.state.poll_spawned_commands();
            data.state.poll_xdg_clients(std::time::Instant::now());
            TimeoutAction::ToDuration(std::time::Duration::from_secs(1))
        },
    )?;
    info!(
        project = mio_core::PROJECT_NAME,
        socket = ?data.state.socket_name,
        "compositor ready"
    );

    if options.xwayland_satellite {
        if let Err(error) = data.state.start_xwayland_satellite(
            std::ffi::OsStr::new("xwayland-satellite"),
            &options.xwayland_display,
        ) {
            tracing::warn!(%error, "xwayland-satellite unavailable; continuing with native Wayland only");
        }
    }

    let startup_commands = data.state.config.config().startup_commands.clone();
    for command in startup_commands {
        data.state
            .queue_startup_command(command.into_iter().map(std::ffi::OsString::from).collect());
    }

    if let Some(command) = options.command {
        data.state.queue_startup_command(vec![command]);
    }
    let event_result = event_loop.run(None, &mut data, |data| {
        if let Err(error) = data.display_handle.flush_clients() {
            tracing::warn!(%error, "failed to flush Wayland clients");
        }
    });
    data.state.stop_xwayland_satellite();
    data.state.cleanup_ipc();
    event_result?;
    Ok(())
}

#[cfg(test)]
fn startup_command(
    program: &OsStr,
    wayland_display: &OsStr,
    ipc_socket: Option<&std::path::Path>,
    xwayland_display: Option<&str>,
    backend_name: &str,
) -> Command {
    let mut command = Command::new(program);
    command
        .env("WAYLAND_DISPLAY", wayland_display)
        .env("XDG_CURRENT_DESKTOP", "mio")
        .env("XDG_SESSION_DESKTOP", "mio")
        .env("MIO_BACKEND", backend_name);
    if let Some(path) = ipc_socket {
        command.env("MIO_SOCKET", path);
    } else {
        command.env_remove("MIO_SOCKET");
    }
    match xwayland_display {
        Some(display) => {
            command.env("DISPLAY", display);
        }
        None => {
            command.env_remove("DISPLAY");
        }
    }
    command
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct Options {
    backend: BackendKind,
    command: Option<std::ffi::OsString>,
    config_path: Option<std::path::PathBuf>,
    explicit_config: bool,
    check_config: bool,
    xwayland_satellite: bool,
    xwayland_display: String,
    virtual_outputs: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum BackendKind {
    #[default]
    Winit,
    Udev,
}

impl BackendKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Winit => "winit",
            Self::Udev => "udev",
        }
    }
}

enum ParsedOptions {
    Run(Options),
    Help,
    Version,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            backend: BackendKind::default(),
            command: None,
            config_path: None,
            explicit_config: false,
            check_config: false,
            xwayland_satellite: false,
            xwayland_display: ":100".to_owned(),
            virtual_outputs: 1,
        }
    }
}

impl Options {
    fn parse() -> MainResult<ParsedOptions> {
        Self::parse_from(std::env::args_os().skip(1))
    }

    fn parse_from(args: impl IntoIterator<Item = std::ffi::OsString>) -> MainResult<ParsedOptions> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("-h" | "--help") => return Ok(ParsedOptions::Help),
                Some("-V" | "--version") => return Ok(ParsedOptions::Version),
                Some("-c" | "--command") => {
                    options.command = Some(args.next().ok_or("--command requires a program")?);
                }
                Some("--config") => {
                    options.config_path =
                        Some(args.next().ok_or("--config requires a path")?.into());
                    options.explicit_config = true;
                }
                Some("--check-config") => options.check_config = true,
                Some("--backend") => {
                    options.backend = match args
                        .next()
                        .ok_or("--backend requires winit or udev")?
                        .to_str()
                    {
                        Some("winit") => BackendKind::Winit,
                        Some("udev") => BackendKind::Udev,
                        _ => return Err("--backend must be winit or udev".into()),
                    };
                }
                Some("--xwayland-satellite") => options.xwayland_satellite = true,
                Some("--xwayland-display") => {
                    options.xwayland_display = args
                        .next()
                        .ok_or("--xwayland-display requires :NUMBER")?
                        .into_string()
                        .map_err(|_| "--xwayland-display must be valid UTF-8")?;
                }
                Some("--virtual-outputs") => {
                    options.virtual_outputs = args
                        .next()
                        .ok_or("--virtual-outputs requires a number")?
                        .to_str()
                        .ok_or("--virtual-outputs must be valid UTF-8")?
                        .parse()
                        .map_err(|_| "--virtual-outputs must be a positive number")?;
                    if options.virtual_outputs == 0 {
                        return Err("--virtual-outputs must be a positive number".into());
                    }
                }
                _ => return Err(compositor_usage().into()),
            }
        }
        Ok(ParsedOptions::Run(options))
    }
}

fn compositor_usage() -> &'static str {
    concat!(
        "Usage: mio-compositor [OPTIONS]\n",
        "\n",
        "Options:\n",
        "  -c, --command PROGRAM       Start an application inside Mio\n",
        "      --config PATH           Load this KDL configuration file\n",
        "      --check-config          Validate configuration without starting Mio\n",
        "      --backend BACKEND       Select winit (default) or udev/DRM\n",
        "      --xwayland-satellite    Enable optional X11 compatibility\n",
        "      --xwayland-display :N   Select the X display (default: :100)\n",
        "      --virtual-outputs N     Split the nested window for development\n",
        "  -h, --help                  Show this help\n",
        "  -V, --version               Show the version\n",
    )
}

fn init_logging() -> MainResult {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{compositor_usage, startup_command, BackendKind, Options, ParsedOptions};

    #[test]
    fn startup_command_receives_only_mio_display_endpoints() {
        let native = startup_command(
            OsStr::new("client"),
            OsStr::new("wayland-7"),
            Some(std::path::Path::new("/run/user/1000/mio.sock")),
            None,
            "udev",
        );
        let (_, wayland_display) = native
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("WAYLAND_DISPLAY"))
            .expect("WAYLAND_DISPLAY has an explicit value");
        assert_eq!(wayland_display, Some(OsStr::new("wayland-7")));
        let (_, ipc_socket) = native
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("MIO_SOCKET"))
            .expect("MIO_SOCKET has an explicit value");
        assert_eq!(ipc_socket, Some(OsStr::new("/run/user/1000/mio.sock")));
        let (_, native_display) = native
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("DISPLAY"))
            .expect("DISPLAY has an explicit removal");
        assert_eq!(native_display, None);
        let (_, current_desktop) = native
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("XDG_CURRENT_DESKTOP"))
            .expect("XDG_CURRENT_DESKTOP has an explicit value");
        assert_eq!(current_desktop, Some(OsStr::new("mio")));
        let (_, session_desktop) = native
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("XDG_SESSION_DESKTOP"))
            .expect("XDG_SESSION_DESKTOP has an explicit value");
        assert_eq!(session_desktop, Some(OsStr::new("mio")));
        let (_, backend) = native
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("MIO_BACKEND"))
            .expect("MIO_BACKEND has an explicit value");
        assert_eq!(backend, Some(OsStr::new("udev")));

        let compatibility = startup_command(
            OsStr::new("client"),
            OsStr::new("wayland-7"),
            None,
            Some(":100"),
            "winit",
        );
        let (_, compatibility_display) = compatibility
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("DISPLAY"))
            .expect("DISPLAY has an explicit value");
        assert_eq!(compatibility_display, Some(OsStr::new(":100")));
    }

    #[test]
    fn compositor_help_is_a_successful_parse_mode() {
        let parsed = Options::parse_from([std::ffi::OsString::from("--help")]).unwrap();
        assert!(matches!(parsed, ParsedOptions::Help));
        assert!(compositor_usage().contains("--check-config"));
    }

    #[test]
    fn compositor_version_is_a_successful_parse_mode() {
        let parsed = Options::parse_from([std::ffi::OsString::from("--version")]).unwrap();
        assert!(matches!(parsed, ParsedOptions::Version));
    }

    #[test]
    fn compositor_backend_is_explicitly_selectable() {
        let ParsedOptions::Run(options) = Options::parse_from([
            std::ffi::OsString::from("--backend"),
            std::ffi::OsString::from("udev"),
        ])
        .unwrap() else {
            panic!("backend selection should run the compositor");
        };
        assert_eq!(options.backend, BackendKind::Udev);
    }
}
