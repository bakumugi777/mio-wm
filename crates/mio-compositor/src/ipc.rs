use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::fs::{FileTypeExt, PermissionsExt},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

use mio_core::{
    Action, ActionOutcome, Direction, GridSize, OutputId, Presentation, WindowId, WindowProperty,
    WindowPropertyKind,
};
use smithay::wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData};
use smithay::{
    reexports::calloop::{generic::Generic, Interest, Mode, PostAction},
    utils::SERIAL_COUNTER,
};
use tracing::{debug, info, warn};

use crate::{state::MioState, CalloopData};

const MAX_REQUEST_BYTES: usize = 4096;
const REQUEST_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_CONNECTIONS_PER_TICK: usize = 4;

impl MioState {
    pub(crate) fn init_ipc(
        &mut self,
        event_loop: &mut smithay::reexports::calloop::EventLoop<CalloopData>,
    ) -> io::Result<()> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
        let socket_component = self.socket_name.to_string_lossy().replace('/', "_");
        let path = PathBuf::from(runtime).join(format!("mio-{socket_component}.sock"));
        let listener = bind_ipc_listener(&path)?;
        listener.set_nonblocking(true)?;
        event_loop
            .handle()
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                |_, listener, data| {
                    for _ in 0..MAX_CONNECTIONS_PER_TICK {
                        match listener.accept() {
                            Ok((stream, _)) => serve(stream, &mut data.state),
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                            Err(error) => {
                                warn!(%error, "failed to accept Mio IPC client");
                                break;
                            }
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(io::Error::other)?;
        info!(socket = %path.display(), "Mio IPC ready");
        self.ipc_socket_path = Some(path);
        Ok(())
    }

    pub(crate) fn handle_ipc_request(&mut self, request: &str) -> String {
        match parse_request(request) {
            Ok(IpcRequest::State) => self.state_json(),
            Ok(IpcRequest::Windows) => self.windows_json(),
            Ok(IpcRequest::FocusedWindow) => self.focused_window_json(),
            Ok(IpcRequest::Camera) => self.camera_json(),
            Ok(IpcRequest::Outputs) => self.outputs_json(),
            Ok(IpcRequest::Quit) => {
                info!("Mio shutdown requested through IPC");
                self.loop_signal.stop();
                "{\"ok\":true}".to_owned()
            }
            Ok(IpcRequest::ActivateOutput { id }) => {
                self.apply_ipc_action(Action::ActivateOutput(id))
            }
            Ok(IpcRequest::Focus { id }) => self.apply_ipc_action(Action::FocusWindow(id)),
            Ok(IpcRequest::CameraStep { direction }) => {
                self.apply_ipc_action(Action::CameraStep(direction))
            }
            Ok(IpcRequest::MoveWindow { id, direction }) => self.ipc_move(id, direction),
            Ok(IpcRequest::ResizeWindow { id, direction }) => self.ipc_resize(id, direction),
            Ok(IpcRequest::ToggleFloating { id }) => {
                self.apply_ipc_action(Action::ToggleFloating(id))
            }
            Ok(IpcRequest::Close { id }) => self.apply_ipc_action(Action::CloseWindow(id)),
            Ok(IpcRequest::SetOpacity { id, opacity }) => {
                self.apply_ipc_action(Action::SetWindowProperty {
                    id,
                    property: WindowProperty::Opacity(opacity),
                })
            }
            Ok(IpcRequest::ClearOpacity { id }) => {
                self.apply_ipc_action(Action::ClearWindowProperty {
                    id,
                    kind: WindowPropertyKind::Opacity,
                })
            }
            Ok(IpcRequest::CameraTo { id }) => self.apply_ipc_action(Action::CameraCenter(id)),
            Ok(IpcRequest::SetProperty { id, property }) => {
                self.apply_ipc_action(Action::SetWindowProperty { id, property })
            }
            Ok(IpcRequest::ClearProperty { id, kind }) => {
                self.apply_ipc_action(Action::ClearWindowProperty { id, kind })
            }
            Err(error) => format!("{{\"ok\":false,\"error\":\"{}\"}}", escape_json(error)),
        }
    }

    fn apply_ipc_action(&mut self, action: Action) -> String {
        let previous_focus = self.world.focused();
        match self.world.apply(action) {
            Ok(outcome) => {
                match outcome {
                    ActionOutcome::FocusChanged(Some(id)) => {
                        self.activate_window_after_focus_change(
                            id,
                            previous_focus,
                            SERIAL_COUNTER.next_serial(),
                        );
                    }
                    ActionOutcome::CloseRequested(id) => {
                        self.begin_close_transition(id);
                    }
                    ActionOutcome::Applied | ActionOutcome::FocusChanged(None) => {
                        self.sync_layout(matches!(action, Action::ResizeWindow { .. }));
                    }
                }
                "{\"ok\":true}".to_owned()
            }
            Err(error) => format!(
                "{{\"ok\":false,\"error\":\"{}\"}}",
                escape_json(&error.to_string())
            ),
        }
    }

    #[allow(clippy::cast_precision_loss)] // Unit Grid deltas are exact in this practical range.
    fn ipc_move(&mut self, id: WindowId, direction: Direction) -> String {
        let Some(rect) = self.world.window(id).map(mio_core::Window::rect) else {
            return unknown_window_response(id);
        };
        let delta = direction.delta(1);
        match rect.origin().translated(delta.x as f64, delta.y as f64) {
            Ok(origin) => self.apply_ipc_action(Action::MoveWindowContinuous { id, origin }),
            Err(error) => error_response(&error.to_string()),
        }
    }

    fn ipc_resize(&mut self, id: WindowId, direction: Direction) -> String {
        let Some(rect) = self.world.window(id).map(mio_core::Window::rect) else {
            return unknown_window_response(id);
        };
        let (width, height) = match direction {
            Direction::Left => (rect.width().saturating_sub(1), rect.height()),
            Direction::Right => (rect.width().saturating_add(1), rect.height()),
            Direction::Up => (rect.width(), rect.height().saturating_sub(1)),
            Direction::Down => (rect.width(), rect.height().saturating_add(1)),
        };
        match GridSize::new(width, height) {
            Ok(size) => self.apply_ipc_action(Action::ResizeWindow { id, size }),
            Err(error) => error_response(&error.to_string()),
        }
    }

    fn windows_json(&self) -> String {
        format!("{{\"ok\":true,\"windows\":{}}}", self.windows_json_value())
    }

    fn windows_json_value(&self) -> String {
        let windows = self
            .world
            .windows()
            .map(|window| self.window_json(window.id()))
            .collect::<Vec<_>>()
            .join(",");
        format!("[{windows}]")
    }

    fn focused_window_json(&self) -> String {
        format!(
            "{{\"ok\":true,\"window\":{}}}",
            self.focused_window_json_value()
        )
    }

    fn focused_window_json_value(&self) -> String {
        self.world
            .focused()
            .map_or_else(|| "null".to_owned(), |id| self.window_json(id))
    }

    fn window_json(&self, id: WindowId) -> String {
        let Some(window) = self.world.window(id) else {
            return "null".to_owned();
        };
        let focused = self.world.focused() == Some(id);
        let rect = window.rect();
        let (app_id, title) = self
            .managed_window(id)
            .and_then(|managed| managed.window.toplevel())
            .map(|toplevel| {
                with_states(toplevel.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|data| data.lock().ok())
                        .map(|data| (data.app_id.clone(), data.title.clone()))
                        .unwrap_or_default()
                })
            })
            .unwrap_or_default();
        format!(
            "{{\"id\":{},\"app_id\":{},\"title\":{},\"rect\":{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}},\"focused\":{focused},\"presentation\":\"{}\",\"opacity\":{},\"floating\":{},\"blur\":{}}}",
            id.get(),
            json_string(app_id.as_deref()),
            json_string(title.as_deref()),
            rect.origin().x,
            rect.origin().y,
            rect.width(),
            rect.height(),
            presentation_name(window.presentation()),
            window.effective_properties().opacity,
            window.effective_properties().floating,
            window.effective_properties().blur,
        )
    }

    fn camera_json(&self) -> String {
        format!("{{\"ok\":true,\"camera\":{}}}", self.camera_json_value())
    }

    fn camera_json_value(&self) -> String {
        let camera = self.world.camera();
        format!(
            "{{\"output_id\":{},\"x\":{},\"y\":{},\"zoom\":{}}}",
            self.world.active_output().get(),
            camera.position().x,
            camera.position().y,
            camera.zoom()
        )
    }

    fn outputs_json(&self) -> String {
        let (outputs, cameras) = self.outputs_json_values();
        format!("{{\"ok\":true,\"outputs\":{outputs},\"cameras\":{cameras}}}")
    }

    fn outputs_json_values(&self) -> (String, String) {
        let outputs = self
            .space
            .outputs()
            .map(|output| {
                let geometry = self.space.output_geometry(output).unwrap_or_default();
                let id = self
                    .outputs
                    .iter()
                    .find_map(|(&id, candidate)| (candidate == output).then_some(id.get()));
                format!(
                    "{{\"id\":{},\"name\":\"{}\",\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"scale\":{}}}",
                    id.map_or_else(|| "null".to_owned(), |id| id.to_string()),
                    escape_json(&output.name()),
                    geometry.loc.x,
                    geometry.loc.y,
                    geometry.size.w,
                    geometry.size.h,
                    output.current_scale().fractional_scale()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let cameras = self
            .world
            .output_cameras()
            .map(|(id, camera)| {
                format!(
                    "{{\"id\":{},\"active\":{},\"x\":{},\"y\":{},\"zoom\":{}}}",
                    id.get(),
                    id == self.world.active_output(),
                    camera.position().x,
                    camera.position().y,
                    camera.zoom()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        (format!("[{outputs}]"), format!("[{cameras}]"))
    }

    fn state_json(&self) -> String {
        let (outputs, cameras) = self.outputs_json_values();
        format!(
            "{{\"ok\":true,\"windows\":{},\"focused_window\":{},\"camera\":{},\"outputs\":{outputs},\"cameras\":{cameras}}}",
            self.windows_json_value(),
            self.focused_window_json_value(),
            self.camera_json_value(),
        )
    }

    pub(crate) fn cleanup_ipc(&mut self) {
        if let Some(path) = self.ipc_socket_path.take() {
            if let Err(error) = std::fs::remove_file(&path) {
                debug!(%error, socket = %path.display(), "failed to remove Mio IPC socket");
            }
        }
    }
}

const fn presentation_name(presentation: Presentation) -> &'static str {
    match presentation {
        Presentation::Normal => "normal",
        Presentation::Maximized => "maximized",
        Presentation::Fullscreen => "fullscreen",
    }
}

fn bind_ipc_listener(path: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => {
            secure_ipc_socket(path)?;
            return Ok(listener);
        }
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
        Err(error) => return Err(error),
    }

    match UnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("another Mio instance is using {}", path.display()),
        )),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            let metadata = std::fs::symlink_metadata(path)?;
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("refusing to replace non-socket path {}", path.display()),
                ));
            }
            std::fs::remove_file(path)?;
            let listener = UnixListener::bind(path)?;
            secure_ipc_socket(path)?;
            Ok(listener)
        }
        Err(error) => Err(error),
    }
}

fn secure_ipc_socket(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

fn serve(mut stream: UnixStream, state: &mut MioState) {
    let response = match stream.set_read_timeout(Some(REQUEST_TIMEOUT)) {
        Ok(()) => match read_request(&mut stream) {
            Ok(bytes) => String::from_utf8(bytes).map_or_else(
                |_| "{\"ok\":false,\"error\":\"request must be UTF-8\"}".to_owned(),
                |request| state.handle_ipc_request(&request),
            ),
            Err(error) => error_response(&error.to_string()),
        },
        Err(error) => error_response(&format!("failed to configure request timeout: {error}")),
    };
    if let Err(error) = stream.set_write_timeout(Some(REQUEST_TIMEOUT)) {
        debug!(%error, "failed to configure Mio IPC response timeout");
        return;
    }
    if let Err(error) = stream.write_all(response.as_bytes()) {
        debug!(%error, "failed to write Mio IPC response");
    }
}

fn read_request(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .take((MAX_REQUEST_BYTES + 2) as u64)
        .read_until(b'\n', &mut bytes)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum IpcRequest {
    Quit,
    State,
    Windows,
    FocusedWindow,
    Camera,
    Outputs,
    ActivateOutput {
        id: OutputId,
    },
    Focus {
        id: WindowId,
    },
    CameraStep {
        direction: Direction,
    },
    MoveWindow {
        id: WindowId,
        direction: Direction,
    },
    ResizeWindow {
        id: WindowId,
        direction: Direction,
    },
    ToggleFloating {
        id: WindowId,
    },
    Close {
        id: WindowId,
    },
    SetOpacity {
        id: WindowId,
        opacity: f32,
    },
    ClearOpacity {
        id: WindowId,
    },
    CameraTo {
        id: WindowId,
    },
    SetProperty {
        id: WindowId,
        property: WindowProperty,
    },
    ClearProperty {
        id: WindowId,
        kind: WindowPropertyKind,
    },
}

fn parse_request(request: &str) -> Result<IpcRequest, &'static str> {
    let mut words = request.split_whitespace();
    let command = words.next().ok_or("empty request")?;
    let parsed = match command {
        "quit" => IpcRequest::Quit,
        "state" => IpcRequest::State,
        "windows" => IpcRequest::Windows,
        "focused-window" => IpcRequest::FocusedWindow,
        "camera" => IpcRequest::Camera,
        "outputs" => IpcRequest::Outputs,
        "activate-output" => IpcRequest::ActivateOutput {
            id: parse_output_id(words.next())?,
        },
        "focus" => IpcRequest::Focus {
            id: parse_id(words.next())?,
        },
        "camera-step" => IpcRequest::CameraStep {
            direction: parse_direction(words.next())?,
        },
        "move-window" => IpcRequest::MoveWindow {
            id: parse_id(words.next())?,
            direction: parse_direction(words.next())?,
        },
        "resize-window" => IpcRequest::ResizeWindow {
            id: parse_id(words.next())?,
            direction: parse_direction(words.next())?,
        },
        "toggle-floating" => IpcRequest::ToggleFloating {
            id: parse_id(words.next())?,
        },
        "close" => IpcRequest::Close {
            id: parse_id(words.next())?,
        },
        "set-opacity" => IpcRequest::SetOpacity {
            id: parse_id(words.next())?,
            opacity: words
                .next()
                .ok_or("set-opacity requires ID and VALUE")?
                .parse()
                .map_err(|_| "opacity must be a number")?,
        },
        "clear-opacity" => IpcRequest::ClearOpacity {
            id: parse_id(words.next())?,
        },
        "camera-to" => IpcRequest::CameraTo {
            id: parse_id(words.next())?,
        },
        "set-property" => IpcRequest::SetProperty {
            id: parse_id(words.next())?,
            property: parse_property(words.next(), words.next())?,
        },
        "clear-property" => IpcRequest::ClearProperty {
            id: parse_id(words.next())?,
            kind: parse_property_kind(words.next())?,
        },
        _ => return Err("unknown command"),
    };
    if words.next().is_some() {
        return Err("too many arguments");
    }
    Ok(parsed)
}

fn parse_direction(value: Option<&str>) -> Result<Direction, &'static str> {
    match value.ok_or("direction is required")? {
        "left" => Ok(Direction::Left),
        "right" => Ok(Direction::Right),
        "up" => Ok(Direction::Up),
        "down" => Ok(Direction::Down),
        _ => Err("direction must be left, right, up, or down"),
    }
}

fn parse_property(name: Option<&str>, value: Option<&str>) -> Result<WindowProperty, &'static str> {
    let value = value.ok_or("property value is required")?;
    match name.ok_or("property name is required")? {
        "opacity" => value
            .parse()
            .map(WindowProperty::Opacity)
            .map_err(|_| "opacity must be a number"),
        "floating" => value
            .parse()
            .map(WindowProperty::Floating)
            .map_err(|_| "floating must be true or false"),
        "blur" => value
            .parse()
            .map(WindowProperty::Blur)
            .map_err(|_| "blur must be true or false"),
        _ => Err("property must be opacity, floating, or blur"),
    }
}

fn parse_property_kind(name: Option<&str>) -> Result<WindowPropertyKind, &'static str> {
    match name.ok_or("property name is required")? {
        "opacity" => Ok(WindowPropertyKind::Opacity),
        "floating" => Ok(WindowPropertyKind::Floating),
        "blur" => Ok(WindowPropertyKind::Blur),
        _ => Err("property must be opacity, floating, or blur"),
    }
}

fn error_response(error: &str) -> String {
    format!("{{\"ok\":false,\"error\":\"{}\"}}", escape_json(error))
}

fn unknown_window_response(id: WindowId) -> String {
    error_response(&format!("unknown window {}", id.get()))
}

fn parse_id(value: Option<&str>) -> Result<WindowId, &'static str> {
    value
        .ok_or("window ID is required")?
        .parse::<u64>()
        .map(WindowId::from_u64)
        .map_err(|_| "window ID must be an unsigned integer")
}

fn parse_output_id(value: Option<&str>) -> Result<OutputId, &'static str> {
    value
        .ok_or("output ID is required")?
        .parse::<u64>()
        .map(OutputId::from_raw)
        .map_err(|_| "output ID must be an unsigned integer")
}

fn json_string(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_owned(),
        |value| format!("\"{}\"", escape_json(value)),
    )
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            character => vec![character],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        os::unix::fs::PermissionsExt,
        os::unix::net::UnixListener,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        bind_ipc_listener, escape_json, parse_request, presentation_name, read_request, IpcRequest,
        MAX_REQUEST_BYTES,
    };
    use mio_core::Presentation;

    fn socket_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mio-{name}-{}-{nonce}.sock", std::process::id()))
    }

    #[test]
    fn parses_read_and_action_requests() {
        assert_eq!(parse_request("quit\n"), Ok(IpcRequest::Quit));
        assert!(parse_request("quit now").is_err());
        assert_eq!(parse_request("state\n"), Ok(IpcRequest::State));
        assert_eq!(parse_request("windows\n"), Ok(IpcRequest::Windows));
        assert!(matches!(
            parse_request("set-opacity 4 0.5"),
            Ok(IpcRequest::SetOpacity { opacity: 0.5, .. })
        ));
        assert!(parse_request("camera-to nope").is_err());
        assert_eq!(
            parse_request("camera-step left"),
            Ok(IpcRequest::CameraStep {
                direction: mio_core::Direction::Left
            })
        );
        assert_eq!(
            parse_request("activate-output 2"),
            Ok(IpcRequest::ActivateOutput {
                id: mio_core::OutputId::from_raw(2)
            })
        );
        assert!(matches!(
            parse_request("set-property 7 floating true"),
            Ok(IpcRequest::SetProperty {
                property: mio_core::WindowProperty::Floating(true),
                ..
            })
        ));
        assert!(matches!(
            parse_request("set-property 7 blur true"),
            Ok(IpcRequest::SetProperty {
                property: mio_core::WindowProperty::Blur(true),
                ..
            })
        ));
        assert!(matches!(
            parse_request("set-property 7 blur true"),
            Ok(IpcRequest::SetProperty {
                property: mio_core::WindowProperty::Blur(true),
                ..
            })
        ));
        assert!(parse_request("resize-window 2 diagonal").is_err());
        assert!(parse_request("close 2 extra").is_err());
    }

    #[test]
    fn serializes_all_window_presentations() {
        assert_eq!(presentation_name(Presentation::Normal), "normal");
        assert_eq!(presentation_name(Presentation::Maximized), "maximized");
        assert_eq!(presentation_name(Presentation::Fullscreen), "fullscreen");
    }

    #[test]
    fn escapes_json_metadata() {
        assert_eq!(escape_json("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }

    #[test]
    fn request_framing_stops_at_newline_and_enforces_the_limit() {
        let mut framed = Cursor::new(b"windows\nignored".to_vec());
        assert_eq!(read_request(&mut framed).unwrap(), b"windows");

        let mut maximum = Cursor::new(vec![b'x'; MAX_REQUEST_BYTES]);
        assert_eq!(read_request(&mut maximum).unwrap().len(), MAX_REQUEST_BYTES);

        let mut oversized = Cursor::new(vec![b'x'; MAX_REQUEST_BYTES + 1]);
        assert_eq!(
            read_request(&mut oversized).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn ipc_socket_is_private_to_the_user() {
        let path = socket_path("permissions");
        let listener = bind_ipc_listener(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        drop(listener);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replaces_stale_ipc_socket() {
        let path = socket_path("stale");
        drop(UnixListener::bind(&path).unwrap());

        let listener = bind_ipc_listener(&path).unwrap();

        drop(listener);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preserves_live_ipc_socket() {
        let path = socket_path("live");
        let listener = UnixListener::bind(&path).unwrap();

        let error = bind_ipc_listener(&path).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        drop(listener);
        std::fs::remove_file(path).unwrap();
    }
}
