# Getting Started

最初は既存のWayland desktop内でnested起動する。問題が起きても現在のsessionへ戻れるため、
Mioの操作と設定を確認する用途に向いている。

NixOSへSDDM sessionとして導入する場合は、先に[Installation](installation.md)のNixOS moduleを
有効にする。nested確認後、ログアウトしてsession一覧から「Mio」を選ぶ。

## 1. Buildと設定検査

NixOSではrepository rootで次を実行する。

```sh
nix-shell
cargo build --workspace
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

最後のcommandが成功終了すればKDLは有効である。

## 2. Nested起動

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info \
cargo run -p mio-compositor -- \
  --config config/mio.kdl \
  --command foot
```

Mioの外枠となるWindowと、その中に`foot`が表示される。標準設定ではwaybar等の
`spawn-at-startup`も起動する。既存desktop側のshortcutが先にキーを奪う場合は、Mouse操作か
競合しない一時的なbindを使う。

## 3. 最低限の操作確認

- `Super+Q`: focused Windowを閉じる
- `Super+Arrow`または`Super+H/J/K/L`: Focus移動
- `Super+Ctrl+Arrow`: Cameraを1画面移動
- `Super+1`〜`Super+0`: Camera倍率を変更
- `Super+V`: Overviewを切り替える
- `Super+R`: KDLを再読み込みする

全操作は[設定リファレンス](configuration.md)を参照する。

## 4. 正常終了

Mio内のterminalから実行する。

```sh
cargo run -p mio-compositor --bin mioctl -- quit
```

installed binaryを使っている場合は`mioctl quit`でよい。terminalを強制終了したり親compositorの
Windowを閉じたりするとclient側に`Broken pipe`が出ることがあるが、通常終了にはIPCを使う。

## 5. Direct backend

Nestedで基本操作を確認してから、text VTで実行する。

```sh
nix-shell
RUST_LOG=info cargo run -p mio-compositor -- \
  --backend udev \
  --config config/mio.kdl \
  --command foot
```

既存のgraphical session内から起動しない。現在は単一GPU・単一接続Outputを主な検証対象とする。
終了できない場合に備えて、別VTからprocessを確認できる状態で試す。
