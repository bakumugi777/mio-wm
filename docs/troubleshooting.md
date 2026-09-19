# Troubleshooting

## `cargo: command not found`

NixOSではrepository rootで先に`nix-shell`へ入る。Mioのnative library pathもこのshellで設定される。

## `could not load libwayland.so`などの共有library error

NixOSで`target/debug/mio-compositor`を開発shell外から実行した可能性が高い。`nix-shell`内で
起動する。別TTYでも同様に、repositoryへ移動してから`nix-shell`へ入る。

## `failed to initialize Mio`、白画面、極端に重い

- nested検証では`WINIT_UNIX_BACKEND=wayland`を指定する
- direct検証ではgraphical session内ではなくtext VTから`--backend udev`で起動する
- 実際に新しいbinaryをbuildしているか確認する
- `RUST_LOG=info`を付け、選択されたbackendとGPU rendererを確認する

Nestedとdirectは起動条件が異なる。direct用commandへ`WINIT_UNIX_BACKEND`を足してもdirect
backendの問題は解決しない。

## 設定変更が反映されない

```sh
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

起動中は`reload-config`（標準では`Super+R`）を実行する。失敗時は直前の有効設定を維持し、
画面とlogへ原因を出す。別pathで起動していないか、起動logの`configuration loaded path=...`も
確認する。

## 子applicationに`Broken pipe`が出る

Compositorを強制終了するとWayland socketが閉じるため、footやGTK applicationが
`Broken pipe`を報告する。Mioが意図せずcrashしたのでなければclient側の原因ではない。
通常終了にはMio内から`mioctl quit`を使う。

## OBSの画面キャプチャが黒い、止まる、候補がない

- `xdg-desktop-portal`と`xdg-desktop-portal-wlr`が導入済みか確認する
- OS設定を変更した場合はuser serviceまたはsessionを再起動する
- direct backendではKDL例の環境export・portal再接続commandが有効か確認する
- OBSの古いsourceを削除し、新しい「スクリーンキャプチャ」を作る

Mioは標準`ext-image-copy-capture-v1`を優先し、legacy `wlr-screencopy`も互換用に公開する。
session lock中のcaptureは拒否される。

## X11 applicationがdisplayを開けない

`xwayland-satellite`を導入し、Mioを`--xwayland-satellite`付きで起動する。Mioが起動した
applicationにだけ互換用`DISPLAY`が渡る。Mio外のshellから起動する場合は、そのMio sessionの
環境を自動では継承しない。

## IMEが動かない

Mioはtext-input/input-method/virtual-keyboard protocolを公開するが、fcitx5本体の起動、環境変数、
addon設定はsession側の責任である。nested環境と正式なDE sessionでは条件が異なるため、現時点では
統合検証が残っている。

## 詳細log

```sh
WINIT_UNIX_BACKEND=wayland \
RUST_LOG=info,mio_compositor::diagnostics=debug \
cargo run -p mio-compositor -- --config config/mio.kdl --command foot
```

問題報告では、起動command、backend、再現操作、関係するlog行をまとめる。長時間監視には
`contrib/mio-long-run-monitor`を利用できる。

