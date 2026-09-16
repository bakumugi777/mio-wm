{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    clippy
    foot
    pkg-config
    rustc
    rustfmt
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

  # Smithay's winit backend loads these libraries at runtime. Merely adding the
  # packages to PATH/PKG_CONFIG_PATH is insufficient for dlopen on NixOS.
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
    wayland
    libdrm
    libdisplay-info_0_3
    libgbm
    libinput
    seatd
    udev
    libxkbcommon
    libGL
  ]);
}
