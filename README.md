# Mio / 澪

Mio（澪）は、RustとSmithayで開発しているWaylandコンポジタ兼タイル型ウィンドウ
マネージャーです。

Mioには従来型のワークスペースがありません。ウィンドウは一つの連続した2次元Worldに
配置され、画面はそのWorldを見るCameraとして扱われます。

初めて試す場合は[Installation](docs/installation.md)と
[Getting Started](docs/getting-started.md)から始めてください。

> [!WARNING]
> マルチモニター対応は実験段階です。ネスト環境向けの仮想Outputと、単一Output向けの
> 実験的なDRM/KMSバックエンドがありますが、ホットプラグ、異なる解像度やスケールの
> 混在、実運用上の信頼性はまだ整っていません。通常の検証には単一Outputを使ってください。

## 必要なもの

- `rust-toolchain.toml`で指定されたRust 1.85
- Git（Cargoが固定されたSmithayのリビジョンを取得するため）
- Wayland、xkbcommon、libinput、libseat、udev、GBM、EGL、OpenGLのネイティブライブラリ

NixOSでは、リポジトリに含まれる開発シェルを利用できます。

```sh
nix-shell
```

通常のNixOS sessionとして導入するためのflake packageとNixOS moduleもあります。
SDDMへの登録を含む設定例は[Installation](docs/installation.md)を参照してください。
NixOS以外では`install.sh`を使い、release build、標準Wayland session entryの登録、
manifestに基づくアンインストールを行えます。

## 開発用のネスト起動

普段の開発では、既存のWaylandセッション内にウィンドウとして起動するwinitバックエンドを
使います。`winit`は既定値なので、`--backend`は省略できます。

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info cargo run -p mio-compositor -- \
  --config config/mio.kdl \
  --command foot
```

利用可能な起動オプションは次のコマンドで確認できます。

```sh
cargo run -p mio-compositor -- --help
```

## 実機での直接起動

実験的な`udev`バックエンドは、DRM/KMS、GBM、EGL、libinput、libseatを使ってMioを
ディスプレイへ直接出力します。現在は一つのGPU、一つの接続済みコネクタ、そのコネクタの
優先モードを選択します。

既存のグラフィカルセッション内からではなく、テキストVTへ切り替えて起動してください。
通常はlogindまたはseatdによって、そのVTのseatが有効になっている必要があります。

```sh
nix-shell
RUST_LOG=info cargo run -p mio-compositor -- \
  --backend udev \
  --config config/mio.kdl \
  --command foot
```

VT切り替え時の停止と復帰には対応しています。次の項目はまだ未対応です。

- 複数GPUと複数の物理Output

角丸、影、ウィンドウ枠、フォーカス水光、およびWindowの開閉遷移は直接バックエンドでも
利用できます。
カーソル航跡も直接バックエンドで利用でき、DRM cursor planeが使える場合はカーソル自体を
歪ませず、完成済みの通常画面だけへ効果を適用します。
標準の`ext-image-copy-capture-v1`と、互換用の`zwlr_screencopy_manager_v1`はnested・directの
両バックエンドで利用できます。sandbox化されたアプリケーションはportalの選択画面を経由して
取得します。Mioの通常Waylandソケットへ直接接続できる非sandboxプロセスは、同じdesktop
sessionの信頼領域として扱われ、protocolへ直接アクセスできます。session lock中のcapture
要求は拒否されます。
起動時に選択した単一GPUでは、稼働中Outputの切断、再接続、およびmode一覧の変更を
再走査して復帰します。起動時に接続済みOutputがなくても終了せず、最初の接続を待機します。
`--command`と`spawn-at-startup`のアプリケーションも最初の実Outputが利用可能になるまで
保留されるため、Outputなしを理由に起動直後のクライアントが終了することはありません。
カーソル要素はDRM cursor planeへ割り当て可能な場合にハードウェア表示され、planeがない、
または画像が大きすぎる場合はSmithayによって通常の画面合成へ自動的に戻されます。
`ext-session-lock-v1`によるロック中は、通常のWindowとLayerSurfaceを描画せず、黒い安全
フレーム、lock surface、および操作用カーソルだけを表示します。

これらが整うまでは、通常の開発にはwinitバックエンドを利用してください。

## 起動時のプログラム

毎回起動するプログラムはKDL設定へ記述できます。

```kdl
spawn-at-startup "waybar"
spawn-at-startup "kaname" "--applications"
```

各値はシェル文字列ではなく、そのまま一つの引数として渡されます。コマンドはMioの
WaylandソケットとIPCソケットの準備後に一度だけ実行され、設定の再読み込みでは再実行
されません。

子プロセスにはMioが作成した`WAYLAND_DISPLAY`と`MIO_SOCKET`、Mioセッションを
識別する`XDG_CURRENT_DESKTOP=mio`と`XDG_SESSION_DESKTOP=mio`、実行中のadapterを示す
`MIO_BACKEND=winit`または`MIO_BACKEND=udev`が渡されます。
`xwayland-satellite`が有効な場合は、互換用の`DISPLAY`も渡されます。

## OBSによる画面録画

`xdg-desktop-portal`と`xdg-desktop-portal-wlr`をOSへ導入してください。
NixOS用の参考設定は`memo/nix/configuration.nix`の`xdg.portal`にあります。設定を
現在のNixOS構成へ反映して再buildするまでは、portalのsystemd user unitは作成されません。

`config/mio.kdl`はMio起動時に`dbus-update-activation-environment`を実行し、portalなどの
D-Bus/systemd起動サービスへMioのWayland socketとdesktop名を渡します。direct backendでは
wlr portalとdesktop portalも順番に再接続しますが、nested backendではホスト側portalを
再起動しません。その後OBSでは、
niriで録画に使用できているものと同じ「スクリーンキャプチャ」を選択します。OBSの版や
翻訳によってソース名が異なるため、別名のソースが存在することは前提にしません。この経路では
Smithay標準の`ext-image-copy-capture-v1`が優先され、legacy `wlr-screencopy`はgrim等との
互換用に残ります。通常Waylandソケットへ直接接続できる非sandbox clientはportalを経由せず
capture protocolを利用できるため、Mioはその範囲をdesktop sessionの信頼境界とします。

## 設定

設定例は[config/mio.kdl](config/mio.kdl)にあります。標準の配置先は次のいずれかです。
全設定項目とkeybind Action名は[設定リファレンス](docs/configuration.md)にまとめています。

```text
$XDG_CONFIG_HOME/mio/config.kdl
$HOME/.config/mio/config.kdl
```

明示的なファイル指定と、起動せずに行う設定検証は次のように実行します。

```sh
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

設定の再読み込みに失敗した場合、直前の有効な設定を維持してエラーを表示します。起動時の
設定が不正な場合は組み込み既定値で起動し、修正後の再読み込みによって復旧できます。

### 設定ファイルの分割

`include`で別のKDLファイルを記述位置へ読み込めます。相対パスは`include`を書いた
ファイルのディレクトリを基準に解決され、読み込み先からさらに`include`することもできます。

```kdl
include "wallpaper.kdl"
```

指定先が存在しない場合はエラーにせず無視します。壁紙選択ツールなどが任意設定を後から
生成する用途を想定しています。ただし、存在するファイルが読めない場合、内容が不正な場合、
または循環参照になった場合は設定エラーになります。

設定再読み込み時にも読み込み先は再評価されますが、`spawn-at-startup`はMio起動時にしか
実行されません。例えば選択した壁紙プログラムを次回起動時に復元するファイルを分離できます。

```kdl
// wallpaper.kdl
spawn-at-startup "mpvpaper" "*" "/path/to/wallpaper.mp4"
```

### キー割り当て

キーの組み合わせは`Ctrl`、`Alt`、`Shift`、`Super`とキー名を`+`で連結します。
キーには任意の1文字、またはxkbcommonのkeysym名を指定できます。

```kdl
bind "Super+Space" "spawn" "wofi" "--show" "drun"
bind "PrintScreen" "spawn" "grim"
bind "Super+F12" "close"
bind "XF86AudioRaiseVolume" "spawn" "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "5%+"
```

主な対応キーは次のとおりです。

- `Space`、`Enter`、`Tab`、`Escape`、`BackSpace`
- `Left`、`Right`、`Up`、`Down`、`Home`、`End`、`PageUp`、`PageDown`
- `Insert`、`Delete`、`PrintScreen`、`Menu`
- `F1`から`F35`
- `KP_0`、`KP_Enter`、`KP_Add`などのテンキー
- `XF86AudioRaiseVolume`、`XF86AudioMute`、`XF86MonBrightnessUp`などのメディアキー
- xkbcommonが認識するその他のkeysym名

`Esc`、`SpaceBar`、`PrtSc`、`PrtScr`、`PgUp`、`PgDn`などの一般的な別名も利用できます。
文字キーは修飾後の記号ではなくkeymapの基準キーで判定するため、`Shift+1`は`!`ではなく
`1`と記述します。これにより、可能な範囲でキーボード配列に依存せず同じ割り当てを使えます。

外部コマンドはシェル文字列ではなく、プログラム名と引数をそれぞれ別のKDL文字列として
直接渡します。パイプ、リダイレクト、環境変数展開などシェルの機能が必要な場合だけ、
明示的に`sh -c`または`bash -c`を使用してください。

```kdl
bind "Super+W" "spawn" "sh" "-c" "my-command | another-command"
```

`bind`を一つでも記述すると、組み込みキー割り当て一式は設定内の`bind`で置き換わります。
残したい既定操作もすべて記述してください。同じキーの組み合わせを重複して宣言すると
設定エラーになります。利用できるActionと全設定項目は
[設定リファレンス](docs/configuration.md)を参照してください。

## 基本操作

既定のキー割り当てはSuperを共通の起点とし、機能群ごとにShiftまたはCtrlだけを加えます。

| キー | 操作 |
|---|---|
| `Super+Arrow` / `Super+H/J/K/L` | World上の指定方向へフォーカスを移し、必要ならCameraで表示する |
| `Super+Ctrl+Arrow` / `Super+Ctrl+H/J/K/L` | Cameraを1画面分、左／下／上／右へ移動する |
| `Super+Ctrl+Shift+H/J/K/L` | CameraをGrid 1セル分、左／下／上／右へ移動する |
| `Super+1`〜`Super+9` | Cameraの絶対倍率を0.1〜0.9へ変更する |
| `Super+0` | Cameraを最大倍率1.0へ戻す |
| `Super+Shift+Arrow` | フォーカス中のウィンドウをGrid 1セル分移動する |
| `Super+Ctrl+Shift+Arrow` | フォーカス中のウィンドウをGrid 1セル分リサイズする |
| `Super+Shift+H/J/K/L` | 次のウィンドウを左／下／上／右へ配置する |
| `Super+N` | 操作対象のOutput Cameraを切り替える |
| `Super+F` | タイル／フローティングを切り替える |
| `Super+Enter` | フルスクリーンを切り替える |
| `Super+M` | 最大化を切り替える |
| `Super+Z` | ウィンドウの初期幅／半幅を切り替える |
| `Super+Q` | フォーカス中のウィンドウを閉じる |
| `Super+O` | 不透明度を1.0と0.8の間で切り替える |
| `Super+Shift+O` | 実行時の不透明度上書きを解除する |
| `Super+V` | Overview表示を切り替える |
| `Super+S` | フォーカス中のウィンドウを選択し、通常倍率へ戻す |
| `Super+B` | フォーカス中のウィンドウのぼかしを切り替える |
| `Super+W` | カーソル航跡を切り替える |
| `Super+R` | 設定を再読み込みする |

マウス操作は[config/mio.kdl](config/mio.kdl)の`mouse`セクションで変更できます。
右ダブルクリックと同じ幅切り替えは、任意のキーへ
割り当てられます。例えば`bind "Super+Z" "toggle-window-size"`と記述します。
Camera倍率は`bind "Super+5" "camera-zoom" 0.5`のように個別に変更できます。
既定の考え方は次のとおりです。

- 右ドラッグでCameraを滑らかに移動する
- ウィンドウ上で右ボタンを押しながら左ドラッグすると、ウィンドウを滑らかに移動する
- ウィンドウ端を左ドラッグするとリサイズする
- 右ボタンを押しながらホイールを回すとCameraを拡大・縮小する
- Output端のホイールで、その軸の前後にフォーカスを移動する
- Output端の短い右クリックで`edge-command`を実行する
- Output端の中クリックで、次のウィンドウを配置する方向を選ぶ

複数ボタンの組み合わせはKDLに書いた順序で判定されます。詳しい判定規則と各Actionの
意味は[docs/spec.md](docs/spec.md)を参照してください。

## IPCとmioctl

Mio内で起動した端末には`MIO_SOCKET`が渡されるため、`mioctl`からそのMioインスタンスを
操作できます。全command、応答形式、終了statusは[IPCリファレンス](docs/ipc.md)を
参照してください。compositor自体のoptionは[CLIリファレンス](docs/cli.md)にあります。

```sh
cargo run -p mio-compositor --bin mioctl -- focused-window
cargo run -p mio-compositor --bin mioctl -- camera
cargo run -p mio-compositor --bin mioctl -- windows
cargo run -p mio-compositor --bin mioctl -- outputs
cargo run -p mio-compositor --bin mioctl -- state
```

主な変更操作は次のとおりです。`ID`には`windows`または`state`で得たWindow IDを指定します。

```sh
cargo run -p mio-compositor --bin mioctl -- focus ID
cargo run -p mio-compositor --bin mioctl -- camera-to ID
cargo run -p mio-compositor --bin mioctl -- move-window ID left
cargo run -p mio-compositor --bin mioctl -- resize-window ID down
cargo run -p mio-compositor --bin mioctl -- toggle-floating ID
cargo run -p mio-compositor --bin mioctl -- set-opacity ID 0.5
cargo run -p mio-compositor --bin mioctl -- clear-opacity ID
cargo run -p mio-compositor --bin mioctl -- set-property ID blur true
cargo run -p mio-compositor --bin mioctl -- clear-property ID blur
cargo run -p mio-compositor --bin mioctl -- close ID
```

Mioを正常終了してログアウトするには、Mio内で起動した端末から次を実行します。

```sh
cargo run -p mio-compositor --bin mioctl -- quit
```

インストール済みなら`mioctl quit`だけで実行できます。終了要求はevent loopへ渡され、
Xwayland satelliteの回収とIPC socketの削除を含む通常の終了処理を通ります。

Mio外から操作する場合は、ログに出る`Mio IPC ready`のパスを`--socket PATH`で指定します。
IPCソケットには所有ユーザーだけがアクセスできます。

## 外部ツールとの連携

外部ツールはMioの`state`スナップショットを取得し、選択したWindowに対して
`mioctl focus ID`などの通常のAction経路を利用できます。これは特定ツール専用のAPIではなく、
Mio、Shirube、Kanameは互いを必須依存にしません。

## 実験的な仮想Output

一つのネストウィンドウを左右に分割し、二つのCameraを同時に表示できます。

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info cargo run -p mio-compositor -- \
  --virtual-outputs 2 \
  --command foot
```

左右どちらかをクリックすると、そのCameraが操作対象になります。`Super+N`でも切り替え
できます。これは開発用機能であり、物理マルチモニターの完成を意味しません。

## X11互換

必要に応じて`xwayland-satellite`をインストールし、次のオプションを付けて起動します。

```sh
cargo run -p mio-compositor -- --xwayland-satellite --command foot
```

既定ではX display `:100`を利用します。使用済みの場合は`--xwayland-display :NUMBER`で
変更できます。satelliteの起動に失敗しても、native Waylandクライアントは継続して動作します。

## 日本語入力

Mioはtext-input、input-method、virtual-keyboardのWaylandプロトコルを公開します。
fcitx5などIME本体の起動と設定はセッション側の責任です。候補ウィンドウは通常のpopupとして
扱われます。実際のDE起動環境でのfcitx5統合検証は今後行います。

## 開発時の確認

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

純粋なWorldロジックだけを確認する場合は次を実行します。

```sh
cargo test -p mio-core
```

5秒ごとの描画診断を有効にする場合は、ログターゲットを追加します。

```sh
WINIT_UNIX_BACKEND=wayland \
RUST_LOG=info,mio_compositor::diagnostics=debug \
cargo run -p mio-compositor -- --config config/mio.kdl --command foot
```

## 設計資料

- [docs/requirements.md](docs/requirements.md): 要件と実装フェーズ
- [docs/installation.md](docs/installation.md): 現在のbuild・導入方法と未整備範囲
- [docs/getting-started.md](docs/getting-started.md): nested起動から正常終了までの最短手順
- [docs/configuration.md](docs/configuration.md): KDL設定とkeybind Actionの公開仕様
- [docs/keybindings.md](docs/keybindings.md): 標準keybindと変更方法
- [docs/camera.md](docs/camera.md): Camera Action、追従、zoom、複数Outputでの意味
- [docs/overview.md](docs/overview.md): Overviewの操作とWorldとの関係
- [docs/window-properties.md](docs/window-properties.md): Window Ruleの合成とProperty優先順位
- [docs/cli.md](docs/cli.md): mio-compositorのCLI option
- [docs/ipc.md](docs/ipc.md): mioctl、IPC command、JSON応答
- [docs/troubleshooting.md](docs/troubleshooting.md): 起動、設定、capture等の問題切り分け
- [docs/architecture-overview.md](docs/architecture-overview.md): Mio設計の日本語概要
- [docs/release-readiness.md](docs/release-readiness.md): 1.0監査結果と残作業
- [docs/spec.md](docs/spec.md): 詳細仕様
- [docs/architecture.md](docs/architecture.md): コンポーネント境界と設計
- [docs/smithay-notes.md](docs/smithay-notes.md): Smithay APIの調査記録
- [docs/phases.md](docs/phases.md): フェーズごとの進行状況

Mioの基本方針は次の一文に集約されます。

> 世界は一つ。ウィンドウはその中に住み、画面はその世界を見るカメラにすぎない。
