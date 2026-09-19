#!/bin/sh

set -eu

PROJECT_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
PREFIX=${PREFIX:-/usr/local}
DESTDIR=${DESTDIR:-}
TARGET_DIR=${CARGO_TARGET_DIR:-"$PROJECT_ROOT/target"}
MANIFEST_PATH="$PREFIX/share/mio/install-manifest.txt"
STAGED_MANIFEST="$DESTDIR$MANIFEST_PATH"

usage() {
    cat <<'EOF'
Usage: ./install.sh COMMAND

Commands:
  check       Check build tools and native dependencies
  install     Build Mio and install binaries, session entry, and config sample
  uninstall   Remove files recorded by the installer

Environment:
  PREFIX      Installation prefix (default: /usr/local)
  DESTDIR     Optional package staging root
  CARGO       Cargo executable (default: cargo)
  CARGO_TARGET_DIR
              Cargo target directory (default: PROJECT_ROOT/target)
  MIO_SKIP_BUILD=1
              Install existing release binaries without rebuilding

Examples:
  ./install.sh check
  sudo ./install.sh install
  PREFIX="$HOME/.local" ./install.sh install
  sudo ./install.sh uninstall
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

require_absolute_prefix() {
    case "$PREFIX" in
        /*) ;;
        *) die "PREFIX must be an absolute path: $PREFIX" ;;
    esac
    case "$PREFIX" in
        *[[:space:]]*) die "PREFIX must not contain whitespace: $PREFIX" ;;
        *) ;;
    esac
}

dependency_hint() {
    distro_id=
    distro_like=
    if [ -r /etc/os-release ]; then
        # The file is defined as shell-compatible key/value data.
        # shellcheck disable=SC1091
        . /etc/os-release
        distro_id=${ID:-}
        distro_like=${ID_LIKE:-}
    fi

    case " $distro_id $distro_like " in
        *" arch "*)
            printf '%s\n' \
                'Arch Linux example:' \
                '  sudo pacman -S --needed base-devel rust pkgconf wayland libxkbcommon libinput seatd systemd-libs mesa libdisplay-info'
            ;;
        *" fedora "*|*" rhel "*)
            printf '%s\n' \
                'Fedora example:' \
                '  sudo dnf install gcc cargo pkgconf-pkg-config wayland-devel libxkbcommon-devel libinput-devel libseat-devel systemd-devel mesa-libgbm-devel mesa-libEGL-devel mesa-libGL-devel libdisplay-info-devel'
            ;;
        *" debian "*|*" ubuntu "*)
            printf '%s\n' \
                'Debian/Ubuntu example:' \
                '  sudo apt install build-essential cargo pkg-config libwayland-dev libxkbcommon-dev libinput-dev libseat-dev libudev-dev libgbm-dev libegl1-mesa-dev libgl1-mesa-dev libdisplay-info-dev'
            ;;
        *)
            printf '%s\n' \
                'Install a Rust toolchain, pkg-config, and development packages for:' \
                '  Wayland, DRM, libdisplay-info, GBM, libinput, libseat, udev, xkbcommon, EGL, and OpenGL.'
            ;;
    esac
}

check_dependencies() {
    missing_tools=
    missing_modules=
    cargo_command=${CARGO:-cargo}

    command -v "$cargo_command" >/dev/null 2>&1 || missing_tools="$missing_tools $cargo_command"
    command -v pkg-config >/dev/null 2>&1 || missing_tools="$missing_tools pkg-config"

    if command -v pkg-config >/dev/null 2>&1; then
        for module in \
            wayland-server \
            libdrm \
            libdisplay-info \
            gbm \
            libinput \
            libseat \
            libudev \
            xkbcommon \
            egl \
            gl
        do
            if ! pkg-config --exists "$module"; then
                missing_modules="$missing_modules $module"
            fi
        done
    fi

    if [ -n "$missing_tools" ] || [ -n "$missing_modules" ]; then
        [ -z "$missing_tools" ] || printf 'Missing tools:%s\n' "$missing_tools" >&2
        [ -z "$missing_modules" ] || printf 'Missing pkg-config modules:%s\n' "$missing_modules" >&2
        dependency_hint >&2
        return 1
    fi

    printf '%s\n' 'Build tools and native dependencies are available.'
}

install_file() {
    source_path=$1
    destination_path=$2
    mode=$3

    install -Dm"$mode" "$source_path" "$DESTDIR$destination_path"
    printf '%s\n' "$destination_path" >> "$manifest_tmp"
}

install_mio() {
    require_absolute_prefix
    check_dependencies

    cargo_command=${CARGO:-cargo}
    if [ "${MIO_SKIP_BUILD:-0}" != 1 ]; then
        "$cargo_command" build --release --locked --workspace --manifest-path "$PROJECT_ROOT/Cargo.toml"
    fi

    compositor="$TARGET_DIR/release/mio-compositor"
    mioctl="$TARGET_DIR/release/mioctl"
    [ -x "$compositor" ] || die "release binary not found: $compositor"
    [ -x "$mioctl" ] || die "release binary not found: $mioctl"

    manifest_tmp=$(mktemp)
    session_tmp=$(mktemp)
    desktop_tmp=$(mktemp)
    trap 'rm -f "$manifest_tmp" "$session_tmp" "$desktop_tmp"' EXIT HUP INT TERM

    cat > "$session_tmp" <<EOF
#!/bin/sh
exec "$PREFIX/bin/mio-compositor" --backend udev "\$@"
EOF

    cat > "$desktop_tmp" <<EOF
[Desktop Entry]
Name=Mio
Comment=The Mio Wayland compositor
Exec=$PREFIX/bin/mio-session
Type=Application
DesktopNames=mio
EOF

    install_file "$compositor" "$PREFIX/bin/mio-compositor" 755
    install_file "$mioctl" "$PREFIX/bin/mioctl" 755
    install_file "$session_tmp" "$PREFIX/bin/mio-session" 755
    install_file "$PROJECT_ROOT/config/mio.kdl" "$PREFIX/share/mio/config.kdl" 644
    install_file "$desktop_tmp" "$PREFIX/share/wayland-sessions/mio.desktop" 644

    printf '%s\n' "$MANIFEST_PATH" >> "$manifest_tmp"
    install -Dm644 "$manifest_tmp" "$STAGED_MANIFEST"

    printf 'Mio was installed under %s%s.\n' "$DESTDIR" "$PREFIX"
    printf 'The configuration sample is %s%s/share/mio/config.kdl.\n' "$DESTDIR" "$PREFIX"
    if [ -n "$DESTDIR" ]; then
        printf '%s\n' 'DESTDIR is set; the staged files have not changed the running system.'
    else
        case "$PREFIX" in
            /usr|/usr/local)
                printf '%s\n' 'Log out, then select Mio in your display manager.'
                ;;
            *)
                printf '%s\n' \
                    'The session entry was installed below PREFIX.' \
                    'Some display managers only discover system-wide session entries.'
                ;;
        esac
    fi
}

uninstall_mio() {
    require_absolute_prefix
    [ -f "$STAGED_MANIFEST" ] || die "install manifest not found: $STAGED_MANIFEST"

    while IFS= read -r installed_path; do
        case "$installed_path" in
            "$PREFIX"/*) rm -f -- "$DESTDIR$installed_path" ;;
            *) die "refusing path outside PREFIX from manifest: $installed_path" ;;
        esac
    done < "$STAGED_MANIFEST"

    rmdir -- "$DESTDIR$PREFIX/share/wayland-sessions" 2>/dev/null || true
    rmdir -- "$DESTDIR$PREFIX/share/mio" 2>/dev/null || true
    rmdir -- "$DESTDIR$PREFIX/share" 2>/dev/null || true
    rmdir -- "$DESTDIR$PREFIX/bin" 2>/dev/null || true
    rmdir -- "$DESTDIR$PREFIX" 2>/dev/null || true

    printf 'Mio files recorded in %s were removed.\n' "$STAGED_MANIFEST"
}

case "${1:-}" in
    check)
        check_dependencies
        ;;
    install)
        install_mio
        ;;
    uninstall)
        uninstall_mio
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
