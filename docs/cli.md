# Mio CLI Reference

[日本語版](cli-jp.md)

## mio-compositor

```text
mio-compositor [OPTIONS]
```

| Option | Description |
|---|---|
| `-c PROGRAM`, `--command PROGRAM` | Launch one program after Mio starts |
| `--config PATH` | Use an explicit KDL configuration file |
| `--check-config` | Validate the configuration and exit without falling back |
| `--backend winit` | Use the nested winit backend; this is the default |
| `--backend udev` | Use the direct DRM/KMS backend |
| `--xwayland-satellite` | Start `xwayland-satellite` |
| `--xwayland-display :N` | Select its X display; defaults to `:100` |
| `--virtual-outputs N` | Split the nested window into N development outputs |
| `-h`, `--help` | Print help and exit successfully |
| `-V`, `--version` | Print the version and exit successfully |

`--virtual-outputs` accepts only positive integers and is available only with the
`winit` backend. Unknown options, missing arguments, and invalid values produce an error
and a non-zero exit status.

`--command` is one executable name, not a shell command line. Declare persistent
programs with multiple arguments as argv in KDL using `spawn-at-startup`.

## Shutdown

`SIGINT`, `SIGTERM`, and IPC `quit` use the same clean shutdown path. Mio stops the event
loop, reaps the xwayland-satellite process it started, and removes the IPC socket.
