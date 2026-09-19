use std::{
    ffi::OsString,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    process::ExitCode,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mioctl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(first) = args.next() else {
        print!("{}", usage());
        return Ok(());
    };
    if is_help(&first) {
        print!("{}", usage());
        return Ok(());
    }
    if is_version(&first) {
        println!("mioctl {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (socket, first_command) = if first == "--socket" {
        let path = args.next().ok_or("--socket requires PATH")?;
        let command = args.next().ok_or("--socket requires COMMAND")?;
        if is_help(&command) {
            print!("{}", usage());
            return Ok(());
        }
        (PathBuf::from(path), command)
    } else {
        let path = std::env::var_os("MIO_SOCKET").ok_or(
            "MIO_SOCKET is not set; run inside a Mio-spawned terminal or pass --socket PATH",
        )?;
        (PathBuf::from(path), first)
    };
    let mut command = vec![first_command];
    command.extend(args);
    let request = join_utf8(command)?;

    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(request.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if response_is_error(&response) {
        return Err(format!("Mio rejected request: {response}").into());
    }
    println!("{response}");
    Ok(())
}

fn response_is_error(response: &str) -> bool {
    response.starts_with("{\"ok\":false")
}

fn is_help(argument: &OsString) -> bool {
    argument == "--help" || argument == "-h" || argument == "help"
}

fn is_version(argument: &OsString) -> bool {
    argument == "--version" || argument == "-V"
}

fn join_utf8(words: Vec<OsString>) -> Result<String, &'static str> {
    words
        .into_iter()
        .map(|word| word.into_string().map_err(|_| "arguments must be UTF-8"))
        .collect::<Result<Vec<_>, _>>()
        .map(|words| words.join(" "))
}

fn usage() -> &'static str {
    concat!(
        "Usage: mioctl [--socket PATH] COMMAND [ARG ...]\n",
        "\n",
        "Read commands:\n",
        "  state | windows | focused-window | camera | outputs\n",
        "\n",
        "Compositor command:\n",
        "  quit\n",
        "\n",
        "Action commands:\n",
        "  activate-output ID\n",
        "  focus ID | camera-to ID | close ID | toggle-floating ID\n",
        "  camera-step DIR | move-window ID DIR | resize-window ID DIR\n",
        "  set-property ID opacity FLOAT\n",
        "  set-property ID floating BOOL\n",
        "  set-property ID blur BOOL\n",
        "  clear-property ID opacity|floating|blur\n",
        "  set-opacity ID FLOAT | clear-opacity ID (compatibility aliases)\n",
        "\n",
        "DIR is left, right, up, or down. BOOL is true or false.\n",
        "Without --socket, MIO_SOCKET selects the Mio instance.\n",
        "Use --version or -V to show the mioctl version.\n",
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{is_help, is_version, response_is_error, usage};

    #[test]
    fn recognizes_help_without_a_mio_connection() {
        for argument in ["--help", "-h", "help"] {
            assert!(is_help(&OsString::from(argument)));
        }
        assert!(!is_help(&OsString::from("state")));
        assert!(usage().contains("  quit\n"));
        assert!(usage().contains("set-property ID floating BOOL"));
        assert!(usage().contains("set-opacity ID FLOAT"));
    }

    #[test]
    fn recognizes_version_without_a_mio_connection() {
        assert!(is_version(&OsString::from("--version")));
        assert!(is_version(&OsString::from("-V")));
        assert!(!is_version(&OsString::from("state")));
    }

    #[test]
    fn server_errors_produce_a_failed_cli_result() {
        assert!(response_is_error(
            r#"{"ok":false,"error":"unknown window 9"}"#
        ));
        assert!(!response_is_error(r#"{"ok":true}"#));
        assert!(!response_is_error(r#"{"ok":true,"windows":[]}"#));
    }
}
