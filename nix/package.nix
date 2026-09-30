{ lib
, rustPlatform
, pkg-config
, makeWrapper
, wayland
, libdrm
, libdisplay-info_0_3
, libgbm
, libinput
, seatd
, udev
, libxkbcommon
, libGL
}:

rustPlatform.buildRustPackage {
  pname = "mio";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type:
      let base = baseNameOf path;
      in !(type == "directory" && builtins.elem base [ ".git" "target" "memo" ]);
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      "smithay-0.7.0" = "sha256-0Zg75LPpgqZxOpd5McByY5wuXKxWPIEoMm/IhikC3qI=";
      "smithay-drm-extras-0.1.0" = "sha256-0Zg75LPpgqZxOpd5McByY5wuXKxWPIEoMm/IhikC3qI=";
    };
  };

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    wayland
    libdrm
    libdisplay-info_0_3
    libgbm
    libinput
    seatd
    udev
    libxkbcommon
    libGL
  ];

  postInstall = ''
    install -Dm644 config/mio.kdl $out/share/mio/config.kdl

    wrapProgram $out/bin/mio-compositor \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [
        wayland
        libdrm
        libdisplay-info_0_3
        libgbm
        libinput
        seatd
        udev
        libxkbcommon
        libGL
      ]}

    makeWrapper $out/bin/mio-compositor $out/bin/mio-session \
      --add-flags "--backend udev"

    install -Dm644 /dev/stdin $out/share/wayland-sessions/mio.desktop <<EOF
    [Desktop Entry]
    Name=Mio
    Comment=The Mio Wayland compositor
    Exec=$out/bin/mio-session
    Type=Application
    DesktopNames=mio
    EOF
  '';

  passthru.providedSessions = [ "mio" ];

  meta = {
    description = "Smithay Wayland compositor with one continuous 2D world";
    homepage = "https://github.com/bakumugi777/mio-wm";
    license = lib.licenses.mit;
    mainProgram = "mio-compositor";
    platforms = lib.platforms.linux;
  };
}
