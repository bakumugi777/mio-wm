# Mio / 澪

Mio（澪）は、RustとSmithayで開発しているWaylandコンポジタ兼タイル型ウィンドウ
マネージャーです。

Mioには従来型のワークスペースがありません。ウィンドウは一つの連続した2次元Worldに
配置され、画面はそのWorldを見るCameraとして扱われます。

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
- portal等の信頼境界を備えた直接バックエンドでの画面取得

角丸、影、ウィンドウ枠、フォーカス水光、およびWindowの開閉遷移は直接バックエンドでも
利用できます。
カーソル航跡も直接バックエンドで利用でき、DRM cursor planeが使える場合はカーソル自体を
歪ませず、完成済みの通常画面だけへ効果を適用します。
無認証の`zwlr_screencopy_manager_v1`はnested開発バックエンドだけで広告され、直接
バックエンドでは広告されません。
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

子プロセスにはMioが作成した`WAYLAND_DISPLAY`と`MIO_SOCKET`が渡されます。
`xwayland-satellite`が有効な場合は、互換用の`DISPLAY`も渡されます。

## 設定

設定例は[config/mio.kdl](config/mio.kdl)にあります。標準の配置先は次のいずれかです。

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

## 基本操作

既定のキー割り当ては、Mioをniriなどの中でネスト起動しても届きやすい組み合わせにして
あります。

| キー | 操作 |
|---|---|
| `Alt+Arrow` | World上の指定方向へフォーカスを移し、必要ならCameraで表示する |
| `Ctrl+Alt+Arrow` | Cameraを1画面分移動する |
| `Alt+Shift+Arrow` | CameraをGrid 1セル分移動する |
| `Ctrl+Shift+Arrow` | フォーカス中のウィンドウをGrid 1セル分移動する |
| `Ctrl+Alt+Shift+Arrow` | フォーカス中のウィンドウをGrid 1セル分リサイズする |
| `Ctrl+Alt+Shift+H/J/K/L` | 次のウィンドウを左／下／上／右へ配置する |
| `Ctrl+Alt+N` | 操作対象のOutput Cameraを切り替える |
| `Ctrl+Alt+F` | タイル／フローティングを切り替える |
| `Ctrl+Alt+Enter` | フルスクリーンを切り替える |
| `Ctrl+Alt+M` | 最大化を切り替える |
| `Ctrl+Alt+Q` | フォーカス中のウィンドウを閉じる |
| `Ctrl+Alt+O` | 不透明度を1.0と0.8の間で切り替える |
| `Ctrl+Alt+Shift+O` | 実行時の不透明度上書きを解除する |
| `Ctrl+Alt+V` | Overview表示を切り替える |
| `Ctrl+Alt+S` | フォーカス中のウィンドウを選択し、通常倍率へ戻す |
| `Ctrl+Alt+B` | フォーカス中のウィンドウのぼかしを切り替える |
| `Ctrl+Alt+W` | カーソル航跡を切り替える |

マウス操作は[config/mio.kdl](config/mio.kdl)の`mouse`セクションで変更できます。
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
操作できます。

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

## Kaname連携例

[contrib/kaname/mio-kaname-windows](contrib/kaname/mio-kaname-windows)は、Mioの`state`
スナップショットをKanameのprovider用JSON Linesへ変換する例です。
[contrib/kaname/menu-item.json](contrib/kaname/menu-item.json)をKanameのメニューへ追加すると、
選択した項目に対して通常の`mioctl focus ID`経路が使われます。

これはKaname専用APIではなく、MioのIPCを利用する外部アダプターの一例です。Mio、Shirube、
Kanameは互いを必須依存にしません。

## 実験的な仮想Output

一つのネストウィンドウを左右に分割し、二つのCameraを同時に表示できます。

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info cargo run -p mio-compositor -- \
  --virtual-outputs 2 \
  --command foot
```

左右どちらかをクリックすると、そのCameraが操作対象になります。`Ctrl+Alt+N`でも切り替え
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
- [docs/spec.md](docs/spec.md): 詳細仕様
- [docs/architecture.md](docs/architecture.md): コンポーネント境界と設計
- [docs/smithay-notes.md](docs/smithay-notes.md): Smithay APIの調査記録
- [docs/phases.md](docs/phases.md): フェーズごとの進行状況

Mioの基本方針は次の一文に集約されます。

> 世界は一つ。ウィンドウはその中に住み、画面はその世界を見るカメラにすぎない。
