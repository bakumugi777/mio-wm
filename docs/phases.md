# 43. 実装フェーズ定義

Mioは一度に完成を目指さず、以下のフェーズに分けて実装する。

各フェーズでは、前段階の要件を維持したまま次の機能を追加する。

重要原則：

- 先にWorld / Window / Cameraの核を完成させる
- Visual Effectは後回し
- X11互換はMio固有ロジック完成後
- Multi-monitorやIMEは常用段階で追加
- YaldraはMio本体安定後
- 各フェーズ終了時に動作可能な状態を維持する
- 大規模な未完成機能を同時並行で増やさない

---

# Phase 0 — Repository / Architecture Bootstrap

## 目的

Mio開発の土台を構築する。

まだCompositorとして完成させる必要はない。

## 実装項目

- Cargo workspace作成
- `mio-core`
- `mio-compositor`
- `docs`
- `config`
- `AGENTS.md`
- Smithay dependency固定
- Smithay revision固定
- build / test command整理
- logging導入
- error handling基盤

推奨構成：

```text
mio/
├── Cargo.toml
├── Cargo.lock
├── AGENTS.md
├── crates/
│   ├── mio-core/
│   └── mio-compositor/
├── docs/
│   ├── requirements.md
│   ├── spec.md
│   └── smithay-notes.md
└── config/
```

## Codex作業

Codexには最初に以下を調査させる。

1. Smithay Getting Started
2. smallvil
3. 必要なSmithay feature
4. event loop構造
5. 最小compositor起動手順

調査結果は `docs/smithay-notes.md` へ記録する。

## 受け入れ条件

- `cargo build` が成功する
- `cargo test` が成功する
- `mio-core` がSmithayへ依存していない
- Smithay revisionが固定されている
- Architecture documentが存在する

## この段階では実装しないもの

- Grid
- Window placement
- Camera
- Animation
- XWayland
- Blur
- Yaldra

---

# Phase 1 — Minimal Smithay Compositor

## 目的

Wayland compositorとして最低限起動する状態を作る。

この時点ではMio独自WMである必要はない。

## 実装項目

- Wayland display
- event loop
- basic output
- xdg-shell
- xdg-toplevel
- keyboard
- pointer
- seat
- basic focus
- basic rendering

背景は単色でもよい。

Window appearanceも最低限でよい。

## 必須動作

- Wayland clientを起動できる
- Windowが表示される
- Keyboard入力が届く
- Pointer入力が届く
- Windowをfocusできる
- Windowをcloseできる

## 受け入れ条件

以下のような一般的Wayland clientを最低1つ起動できる。

例：

```text
foot
weston-terminal
gtk4-demo
```

Windowが表示され、KeyboardとPointerを利用できればPhase 1完了。

## 後回し

- Tiling
- Infinite World
- Camera
- Animation
- Floating
- Configuration
- XWayland

---

# Phase 2 — Mio Core World Model

## 目的

Smithayから独立したMio固有World modelを完成させる。

ここからMioの本体となる。

## 実装項目

`mio-core`に最低限以下を実装する。

```text
World
Grid
Window
WindowId
GridPoint
GridRect
Camera
Direction
Focus
Action
```

## 必須設計

Windowはworkspaceへ所属させない。

WindowはWorld座標を持つ。

CameraもWorld座標を持つ。

例：

```rust
struct GridRect {
    x: i64,
    y: i64,
    width: u64,
    height: u64,
}
```

## 実装する操作

- add window
- remove window
- move window
- resize window
- focus window
- directional search
- camera move
- basic placement

## Unit Test

このフェーズでは特にUnit Testを重視する。

最低限テストする。

### World

- 負座標へWindowを置ける
- Camera外Windowも保持される

### Window

- move
- resize
- overlap
- Camera境界crossing

### Focus

- left
- right
- up
- down

### Camera

- move
- viewport calculation

## 受け入れ条件

Smithayを起動しなくても、

```text
cargo test -p mio-core
```

だけでWorld modelを検証できること。

このフェーズではRendering不要。

---

# Phase 3 — Mio World Integration

## 目的

Smithay WindowとMio Coreを接続し、初めて「Mioらしい画面」を実現する。

## 実装項目

- Smithay toplevel → MioWindowId mapping
- Mio GridRect → render position変換
- Camera transform
- tiled placement
- Window move
- Window resize
- Camera movement
- focus + camera coordination

## 最重要機能

WindowがCamera viewport境界を跨げること。

例：

```text
Camera A       Camera B

0 1 2 3 | 4 5 6 7
        |
    ┌─────────┐
    │ Window  │
    └─────────┘
```

Camera AからCamera Bへ移動してもWindowのWorld Rectを変更しない。

## 描画

最低限：

```text
screen_position
=
world_position
-
camera_position
```

を実現する。

Animationはまだ不要。

Cameraは瞬間移動でもよい。

## 受け入れ条件

以下のデモを成立させる。

1. Window AをCamera境界に跨いで配置
2. Cameraを右へ移動
3. Window Aの見える部分が変化
4. Cameraを戻す
5. Window Aが元の位置に存在

さらにCamera外Windowも破棄されないこと。

この段階をMioの最初のConcept Proofとする。

---

# Phase 4 — Core Window Management

## 目的

日常WMとして最低限必要なWindow操作を揃える。

## 実装項目

- standard tiled placement
- Window move
- Window resize
- directional focus
- Camera follow
- close
- fullscreen
- maximize相当
- floating toggle
- floating move
- floating resize

## Tiled / Floating

TiledとFloatingを別世界にしない。

```text
Tiled
→ grid constraint enabled

Floating
→ grid constraint disabled
```

という同一Property modelを維持する。

## Placement

初期Placementは単純でよい。

候補：

```text
current camera内の空き領域
↓
focused window付近
↓
指定方向へWorldを拡張
```

複雑なauto-layoutは実装しない。

## 受け入れ条件

Keyboardだけで、

- focus
- move
- resize
- floating toggle
- fullscreen
- close

を操作できること。

---

# Phase 5 — Configuration Foundation

## 目的

Hard-codedな操作と見た目を外部設定へ移す。

## 実装項目

KDL設定を導入する。

最低限：

- keybind
- focus indicator width / height
- focus indicator color
- corner radius
- opacity
- animation speed placeholder
- basic window rule

## Config Validation

設定ファイルのparse errorを明確に表示する。

可能であれば以下を提供する。

```text
mio check
```

または同等の設定検証機能。

## Hot Reload

初期では必須ではない。

ただし後でreloadできる構造にはしておく。

## 受け入れ条件

再コンパイルせず、

- keybind変更
- focus indicator変更
- opacity変更

が可能。

---

# Phase 6 — Window Rules / Runtime Properties

## 目的

Windowの見た目と挙動を共通Property systemへ統合する。

この機能はMioの重要機能の一つ。

## 実装項目

Window Property例：

```text
opacity
floating
focus indicator
corner-radius
blur placeholder
shadow placeholder
animation enabled
```

## Config Rule

最低限、

```text
app-id
title
```

によるmatchを実装。

例：

```kdl
window-rule {
    match app-id="mpv"
    opacity 1.0
}
```

## Runtime Override

Focused WindowへRuntime overrideを付与できる。

例：

```text
SetWindowProperty
ClearWindowProperty
ToggleWindowProperty
```

## Property Priority

必ず以下を維持する。

```text
Default
↓
Matched Config Rule
↓
Runtime Override
```

## 受け入れ条件

同じapp-idのWindowを2つ開き、

片方だけRuntime opacityを変更できること。

Override解除時にはConfig値へ戻ること。

設定ファイルを書き換えないこと。

---

# Phase 7 — Animation System

## 目的

Logical stateとRender stateを分離し、Mioの「流れ」を実現する。

## 実装項目

- generic interpolation
- Camera movement
- Window move
- Window resize
- opacity transition
- zoom interpolation

## 基本モデル

```text
current
target
interpolate
```

で統一する。

Camera専用、Window専用など大量の別animation systemを作らない。

## 設定

最低限：

```text
animation.speed
```

必要に応じて、

```text
camera speed
window speed
zoom speed
```

をoverride可能にする。

## 受け入れ条件

Camera境界を跨ぐWindowを表示した状態でCameraを移動し、

Windowが瞬間切替ではなく連続的に流れて見えること。

---

# Phase 8 — Overview / Camera Zoom

## 目的

MioのCamera modelを活用したOverviewを実装する。

## 実装項目

- Camera zoom
- zoom-out
- zoom-in
- Overview navigation
- Window selection
- target WindowへCamera移動

## 禁止事項

Overview専用のWindow一覧画面を作らない。

World modelを複製しない。

## 基本動作

```text
normal
↓
zoom out
↓
Worldを俯瞰
↓
Window選択
↓
Camera移動
↓
zoom in
```

## 受け入れ条件

Overview前後でWindowのWorld座標が変化しないこと。

Window間の相対位置も維持すること。

---

# Phase 9 — Mouse Camera Operations

## 目的

MouseのみでもMio Worldを自然に移動できるようにする。

## 実装操作

### RMB Drag

```text
RMB + pointer motion
→ Camera pan
```

Worldを直接掴む向きに連続移動し、release時のCamera位置を維持する。

### RMB + Wheel

RMBを保持したwheelでCameraの遠近を連続操作する。初期zoom `1.0`が最も近い状態で、
wheel downで遠ざかり、wheel upで`1.0`まで近づく。

## 将来候補

- Camera stop snap
- weak inertia
- corner diagonal movement
- pointer constraint / relative pointer
- custom sensitivity

## 受け入れ条件

Mouseだけで隣接Camera領域へ移動できること。

Mouse操作でもKeyboardと同じCamera modelを利用していること。

---

# Phase 10 — Wayland Daily-Use Protocols

## 目的

Mioを日常利用可能なWayland compositorへ近づける。

## 実装優先候補

1. layer-shell
2. popup handling
3. clipboard / data device
4. fullscreen refinement
5. idle inhibit
6. session lock
7. input method
8. text input
9. screencopy
10. fractional scaling

## 特に確認するもの

- Quickshell
- Waybar
- fcitx5
- screen sharing
- notification daemon
- launcher

## IME

日本語入力を常用可能にする。

候補Window / popup positionがCamera transformによって壊れないこと。

## 受け入れ条件

通常のWayland desktopとして主要アプリを長時間利用できる状態を目標とする。

---

# Phase 11 — X11 Compatibility

## 目的

X11-only / X11-preferred applicationを利用可能にする。

## 初期方針

`xwayland-satellite` を優先候補とする。

## 実装項目

- availability detection
- startup management
- DISPLAY handling
- failure handling
- optional restart

## 原則

X11 WindowをMio Coreへ特別なWindow種別として持ち込まない。

可能な限り通常Windowとして扱う。

## 受け入れ条件

代表的なX11 applicationを起動し、

- focus
- move
- resize
- Camera movement
- input

が動作すること。

---

# Phase 12 — IPC

## 目的

Mio外部からWorld状態を取得・操作できるようにする。

## Read API

最低限：

```text
windows
focused-window
app-id
title
window rect
camera position
camera zoom
outputs
```

## Action API

最低限：

```text
focus
camera-to
camera-step
move-window
resize-window
toggle-floating
set-property
clear-property
close
```

## 原則

IPC専用操作を増やさない。

内部ActionをIPCから呼び出す。

## 受け入れ条件

CLIから、

```text
focused window取得
↓
opacity override
↓
camera to window
```

などが行えること。

---

# Phase 13 — External Application Integration

## 目的

Mioの状態とActionを、特定のshellへ依存しない形で外部アプリケーションから利用可能にする。

## Integration boundary

外部アプリケーションはIPC snapshotから状態を読み、共通Actionを呼び出す。
Mio Coreへ外部アプリ固有のUI modelまたは状態を追加しない。

## Shirube

利用候補：

- focused Window
- active app
- Camera state
- Runtime Property操作

Window一覧やWindow選択UIはbarの責務に含めない。ShirubeへOverviewまたはWindow switcherを
追加しない。

## Kaname example

利用候補：

- application launch
- Mio Action launch
- World navigation
- command hierarchy

## 原則

Shirube、Kaname、その他の外部アプリケーションは必須依存にしない。

Mio単体でも動作する。

## 受け入れ条件

外部アプリケーションがMio Window一覧をIPC snapshotから取得し、Windowを選択して既存IPC
Actionを通してMio Cameraを対象へ移動してfocusできること。Kaname dynamic provider adapterを
この汎用境界の利用例として提供すること。adapterが存在しなくてもMioおよび各外部アプリは
それぞれ単独で動作すること。

---

# Phase 14 — Multi-Monitor

## 目的

複数OutputでMioを常用可能にする。

## 未確定事項

以下は実装前に設計する。

- OutputごとにCameraを持つか
- Camera間のWorld位置関係
- WindowをOutput境界へ跨がせるか
- 異なるscale間のtransform
- Overviewで複数Outputをどう扱うか

## 絶対条件

Multi-monitor対応のためにworkspace modelを導入しない。

Mio Worldは一つのまま維持する。

## 受け入れ条件

異なるresolution / scaleを持つ2 Outputで、

WindowとCamera操作が破綻しないこと。

---

# Phase 15 — Visual Effects

## 目的

Mio固有の視覚的個性を追加する。

## 候補

- blur
- subtle ripple
- shadow
- cursor-associated disturbance
- transition refinement

## 原則

Visual effectはCore World modelから独立させる。

Effectを無効化してもWindow Managementが完全に動作すること。

Focus indicatorを下辺の控えめな水光にした判断はfocusの目印だけに適用し、Mio全体のVisual effectを
制限しない。Visual effectはMio固有の体験を形作る主要機能として実装する。

## 優先度

Core World / CameraとDaily usabilityの基礎が成立した後は高い。

正しいWindow Managementと分離可能なrendering/effects layerとして、Mioらしい表現を
積極的に探求する。

## 第一段階

Windowのbackdrop blurを共通Propertyとして追加する。既存のDefault、matched Config Rule、
Runtime Overrideの順序に従い、rendererはWindowより背後の内容だけを取得してぼかすこと。
各blur Windowのeffectを通常のWindow render elementsと同じz順へ挿入し、複数blur layerが
重なっても背後の内容だけを順次取得する。virtual Outputへの一般化は後続とする。
ぼかし品質はdual Kawaseの縮小・拡大passで改善し、pass数とsampling offsetだけを高水準の
effects設定として公開する。中間textureやkernel内部値は設定概念にしない。

続く段階では、角丸clipと同じ輪郭を使うsoft shadowをrenderer-only elementとして
Window直背面へ配置する。shadowの拡張描画範囲はWorld geometryやinput regionへ影響させず、
radius 0ではelement自体を生成しない。

cursor-associated disturbanceはPointerを舟に見立てる短い航跡とする。静止時と通常速度では
何も表示せず、設定した高速域へ入った場合だけカーソル付近の完成済み画面を進行方向の
左右へ弱く押し分け、その後方を屈折させる。航跡は幅を
少し広げながら減衰し、停止後は完全に元へ戻る。変位勾配からごく弱い明暗を導出するが、
色付きの光や線は重ねない。航跡はoutput座標だけを持つrenderer-only stateとし、Worldへ
追加しない。

Windowの開閉transitionはwaterまたはSF表現を選択できる。waterは歪んだ像の凝結・溶解、
SFは白い中心線の伸長と上下展開を同じ進捗の順逆で表す。論理Window geometryを補間せず、
閉じるWindowはCore Worldから
先に除去してadapter側の描画寿命だけを短時間保持する。clientが自発的にsurfaceを破棄した
場合も、renderer adapterが最後のWindow snapshotをClosingVisualとして保持し、同じ
transition shaderで消去する。

---

# Phase 16 — Yaldra Integration

## 目的

Mioをprogrammable compositorへ拡張する。

## 前提条件

このフェーズ開始前に、

- Mio Core API安定
- Action model安定
- Property model安定
- KDL config安定
- Yaldra runtime実用可能

であること。

## 初期公開範囲

Yaldraから最低限：

```text
Window
Camera
Focus
Selection
Action
Property
```

へアクセスできるようにする。

## 第一段階

Yaldraによる：

- keybind action
- Window Rule condition
- composed commands

## 第二段階

- custom placement
- custom focus algorithm
- Camera behavior
- event handling

## 禁止事項

Yaldraへ直接、

- wl_surface
- DRM buffers
- Smithay internal state
- unsafe renderer internals

を公開しない。

## 受け入れ条件

例えばYaldraで、

```text
「右にある最寄りWindowへfocusし、
Cameraもそこへ移動する」
```

という高水準操作をprimitiveから定義できること。

---

# Phase 17 — Stabilization / Beta

## 目的

新機能追加より不具合修正を優先し、Mioを日常利用へ耐えられる状態にする。

## 実施内容

- long-running test
- suspend / resume
- monitor hotplug
- application compatibility
- popup bugs
- focus edge cases
- DnD
- clipboard
- IME
- fullscreen
- game compatibility
- XWayland
- config reload
- crash recovery

## 重要方針

このフェーズでは新しい大型機能を極力追加しない。

## 受け入れ条件

開発者自身が一定期間MioをメインWMとして利用し、重大な回避不能問題がないこと。

---

# Phase 18 — 1.0 Preparation

## 目的

公開APIと基本挙動を一定程度固定する。

## 対象

- KDL syntax
- Action names
- Window Property names
- IPC
- Camera semantics
- Window Rule semantics
- config path
- CLI interface

## Documentation

最低限以下を整備する。

```text
README
Installation
Getting Started
Configuration
Keybindings
Window Rules
Camera Model
Overview
IPC
Troubleshooting
Architecture overview
```

## 1.0必須ではないもの

- Yaldra
- 高度なshader
- 大量のlayout
- built-in XWayland
- 高度なpseudo-3D effect

これらのために1.0を遅らせない。

---

# 44. フェーズ間の優先順位

大分類すると以下の4段階に分かれる。

```text
Stage A
「Compositorとして生きる」

Phase 0
Phase 1
```

↓

```text
Stage B
「Mioになる」

Phase 2
Phase 3
Phase 4
```

↓

```text
Stage C
「日常利用できるMioになる」

Phase 5
Phase 6
Phase 7
Phase 8
Phase 9
Phase 10
Phase 11
Phase 12
Phase 14
```

↓

```text
Stage D
「Mioの世界を完成させる」

Phase 13
Phase 15
Phase 16
Phase 17
Phase 18
```

最重要なのはStage Bである。

Mio固有の価値は、

- Infinite 2D World
- Grid Window
- Camera
- Camera boundary crossing

によって成立する。

これらが完成する前にVisual EffectやShell Integrationへ大きく脱線しないこと。

---

# 45. Codex向けフェーズ運用ルール

Codexは原則として、現在指定されているPhaseの範囲だけを実装する。

明示的な指示なしに次Phaseの大規模機能へ着手しない。

例えばPhase 3で、

```text
Camera movementを実装する
```

作業中に、

```text
ついでにOverviewも実装
ついでにBlur追加
ついでにIPC追加
```

などを行わない。

各Phase終了時には必ず：

1. build
2. tests
3. current requirements確認
4. architecture違反確認
5. regression確認

を行う。

Smithay利用方法が不明な場合は推測で実装せず、

- smallvil
- anvil
- niri
- 使用中Smithay revision

を調査してから実装する。

---

# 46. 最初の実装目標

最初に目指す視覚的な成果は以下。

```text
1. Mio起動
2. 2つ以上のWindowを表示
3. World Grid上へ配置
4. Windowの一つをCamera境界へ跨がせる
5. Cameraを右へ移動
6. Windowが連続した同一Windowとして画面を跨ぐ
7. Cameraを戻す
8. Windowが元のWorld位置へ存在している
```

これが成功した時点で、

**Mioの基本思想は実装上成立した**

と判断する。

このデモが完成するまでは、外観や高度なProtocol対応よりWorld / Camera modelを優先する。
