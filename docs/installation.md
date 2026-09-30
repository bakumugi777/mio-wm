# Installation

MioはNix flake packageとNixOS module、および汎用installerを提供している。moduleを有効にすると
Wayland sessionがdisplay managerへ登録され、SDDM等のsession一覧からMioを選択できる。
物理Multi-monitorの実機検証は完了していないため、既存desktopを削除せずに導入する。

> [!WARNING]
> 現時点で実機動作を検証しているdistributionはNixOSだけである。Arch Linux、Debian / Ubuntu、
> Fedoraおよびその他のLinux向け手順は、build dependencyと標準Wayland sessionの構成に基づいて
> 整備しているが、そのdistribution上でのdirect session、portal、seat accessは未検証である。
> 初回は既存desktop内でnested起動し、既存desktopを削除せずに試すこと。

## NixOSへ導入する

system flakeの`inputs`へMioを追加する。

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    mio = {
      url = "github:bakumugi777/mio-wm";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, mio, ... }: {
    nixosConfigurations.HOSTNAME = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        mio.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

`configuration.nix`でmoduleとdisplay managerを有効にする。

```nix
{
  services.displayManager.sddm.enable = true;

  programs.mio = {
    enable = true;
    xwayland.enable = true;
  };
}
```

`programs.mio.enable`はMio package、Wayland session、画面共有用portalを導入する。
session起動処理はMioのWayland socketが準備できた後にDBus環境とportalを自動更新するため、
ユーザーのKDLへportal用commandを書く必要はない。
また、GNOME KeyringのSecrets componentを導入し、SDDM等のPAM loginで解除されたlogin
keyringをMio sessionから利用可能にする。これはGitHub CLIやbrowser等の認証情報保存に使われ、
GNOME desktop本体を導入または起動するものではない。Mioは全XDG autostartを開始しないため、
KDLで明示した常駐applicationがこの対応によって二重起動することはない。
標準設定を試せるよう、既定ではFoot、Waybar、Wofiも導入する。不要なら
`programs.mio.recommendedPackages = false;`にする。X11互換が不要なら
`programs.mio.xwayland.enable = false;`にする。

開発中の作業treeを直接使う場合、GitHub URLの代わりに次を指定できる。

```nix
mio.url = "path:/home/shin/Desktop/dev/mio";
```

構成を反映した後に一度ログアウトし、SDDMのsession一覧から「Mio」を選ぶ。通常はOS全体の
再起動までは不要である。display manager自体の更新が反映されない場合だけ再起動する。

設定ファイルは`$XDG_CONFIG_HOME/mio/config.kdl`または`$HOME/.config/mio/config.kdl`へ置く。
未配置でも組み込み既定値で起動する。repositoryの設定例を使う場合は、既存ファイルを確認して
から`config/mio.kdl`をコピーする。

追加の起動引数はmoduleからも指定できる。

```nix
programs.mio.extraSessionArguments = [
  "--config"
  "/home/shin/.config/mio/config.kdl"
];
```

個人のhome pathをsystem設定へ固定したくない場合は、このoptionを使わず標準設定pathを使う。

## Flake packageだけをbuildする

repository rootで実行する。

```sh
nix build .#mio
./result/bin/mio-compositor --version
./result/bin/mio-compositor \
  --config ./result/share/mio/config.kdl \
  --check-config
```

生成物には`mio-compositor`、`mioctl`、direct backend用の`mio-session`、設定例、
`mio.desktop`が含まれる。

## NixOS開発環境

repository rootで開発shellへ入る。

```sh
nix-shell
cargo build --workspace
```

`shell.nix`はRust toolchainと、Smithayのwinit・DRM/KMS backendに必要なnative libraryを提供する。
NixOSではこのshell外から直接binaryを実行すると、`libwayland.so`などを見つけられない場合がある。

## その他のLinux環境

Rust 1.85と、Wayland、xkbcommon、libinput、libseat、udev、GBM、EGL、OpenGLの開発libraryが
必要になる。distributionごとのpackage名は異なる。依存を導入後、repository rootで実行する。
以下は実機検証済みdistributionの一覧ではなく、source buildに必要なpackageの導入例である。

### Arch Linux

```sh
sudo pacman -S --needed base-devel git rust pkgconf wayland libdrm libxkbcommon \
  libinput seatd systemd-libs mesa libdisplay-info
```

### Debian / Ubuntu

```sh
sudo apt update
sudo apt install build-essential git cargo pkg-config libwayland-dev libdrm-dev \
  libxkbcommon-dev libinput-dev libseat-dev libudev-dev libgbm-dev \
  libegl1-mesa-dev libgl1-mesa-dev libdisplay-info-dev
```

distributionの`cargo`がRust 1.85より古い場合は、rustup等でrepositoryの
`rust-toolchain.toml`を満たすtoolchainを導入する。

### Fedora

```sh
sudo dnf install gcc gcc-c++ make git cargo pkgconf-pkg-config wayland-devel \
  libdrm-devel libxkbcommon-devel libinput-devel libseat-devel systemd-devel \
  mesa-libgbm-devel mesa-libEGL-devel mesa-libGL-devel libdisplay-info-devel
```

### その他のdistribution

次のpkg-config moduleを提供するdevelopment packageを導入する。

```text
wayland-server  libdrm  libdisplay-info  gbm  libinput
libseat         libudev xkbcommon        egl  gl
```

正確なpackage名は各distributionのpackage検索を利用する。Mioのinstallerはpackage managerを
自動操作しないため、安全に次のcommandで不足を確認できる。

```sh
./install.sh check
sudo ./install.sh install
```

`check`は不足しているcommandとpkg-config moduleを表示し、Arch Linux、Debian/Ubuntu、Fedoraでは
対応する依存packageの導入例も表示する。package managerを自動実行することはない。

repositoryの取得から導入までは次のとおり。

```sh
git clone https://github.com/bakumugi777/mio-wm.git
cd mio-wm
./install.sh check
sudo ./install.sh install
```

`install`は`cargo build --release --locked --workspace`を実行し、既定では次へ配置する。

```text
/usr/local/bin/mio-compositor
/usr/local/bin/mioctl
/usr/local/bin/mio-session
/usr/local/share/mio/config.kdl
/usr/local/share/wayland-sessions/mio.desktop
```

配置先は`PREFIX`で変更でき、package作成用のstaging rootは`DESTDIR`で指定できる。

```sh
PREFIX="$HOME/.local" ./install.sh install
DESTDIR="$PWD/pkg" PREFIX=/usr ./install.sh install
```

ユーザーprefixへの導入はroot権限を必要としないが、display managerはログイン前に動作するため、
ユーザー側の`share/wayland-sessions`を検索しない実装もある。SDDM等から確実に選択可能にする場合は
`/usr`または`/usr/local`へのsystem-wide導入を使う。

導入時に作成したmanifestだけを対象に削除できる。

```sh
sudo ./install.sh uninstall
PREFIX="$HOME/.local" ./install.sh uninstall
```

別の`PREFIX`や`DESTDIR`で削除する場合は、導入時と同じ値を指定する。ユーザーの
`$XDG_CONFIG_HOME/mio/config.kdl`は導入・削除ともに変更しない。

手動で開発buildだけを行う場合は従来どおり実行できる。

```sh
cargo build --workspace
```

生成物は`target/debug/mio-compositor`と`target/debug/mioctl`である。

## 設定

設定例を標準pathへ配置する場合は、手書きの既存設定を上書きしないよう確認してから
`config/mio.kdl`を次のいずれかへ置く。

```text
$XDG_CONFIG_HOME/mio/config.kdl
$HOME/.config/mio/config.kdl
```

開発中はcopyせず、`--config config/mio.kdl`でrepository内の設定例を直接指定できる。

## NixOS以外でのDesktop session

`install.sh`は標準のWayland session entryを配置するため、その検索先を読むSDDM、GDM、greetd系
greeter等からMioを選択できる。seat access、portal、任意のxwayland-satellite、Foot・Waybar・Wofi
等のsession構成要素はdistribution側で導入する。最初の確認には
[Getting Started](getting-started.md)のnested起動を使う。

direct backendには、logindまたはseatdを通じてDRM・input deviceへアクセスできるlogin sessionが
必要である。手動でTTYから起動できてもdisplay managerから失敗する場合は、session entryの検索先、
PAM/login session、seatd groupまたはlogindの状態を確認する。

画面共有・OBS録画には`xdg-desktop-portal`と`xdg-desktop-portal-wlr`、X11 applicationには
`xwayland-satellite`が別途必要である。launcher、bar、terminal、notification daemon、IME、
lock screenはMio本体へ内蔵しないため、利用者が好みのprogramを導入してKDLの
`spawn-at-startup`や`bind ... "spawn"`から起動する。

## NixOS module option

| Option | 既定値 | 内容 |
|---|---:|---|
| `programs.mio.enable` | `false` | packageとdisplay-manager sessionを有効化 |
| `programs.mio.package` | flakeのMio package | sessionで起動するpackage |
| `programs.mio.extraSessionArguments` | `[]` | compositorへ追加するargv |
| `programs.mio.xwayland.enable` | `false` | xwayland-satelliteを導入・起動 |
| `programs.mio.xwayland.display` | `":100"` | satelliteが使用するX display |
| `programs.mio.portal.enable` | `true` | wlr screencast/screenshot portalを有効化 |
| `programs.mio.recommendedPackages` | `true` | Foot、Waybar、Wofiを導入 |
