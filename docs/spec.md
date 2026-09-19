# 澪 / Mio — Codex向け実装仕様書

## 0. この文書の目的

本書は、Waylandコンポジタ / ウィンドウマネージャ **「澪 / Mio」** を実装するための設計仕様書である。

実装には主として以下を使用する。

- Rust
- Smithay
- Wayland
- 将来的に Yaldra（僭）によるprogrammable configuration

このプロジェクトでは、機能数を増やすことよりも、**少数の単純な原理から多くの挙動を導出できること**を重視する。

新機能を追加するときは、必ず最初に以下を検討すること。

> 新しい概念を追加せず、既存のprimitiveの組み合わせとして実現できないか？

場当たり的な特殊ケース、専用モード、重複した状態管理は極力追加しない。

---

# 1. プロジェクト概要

Mioは、上下左右へ続く2次元空間を持つWayland compositor / tiling window managerである。

一般的なWMのような「workspace」という独立コンテナを基本概念として持たない。

代わりに、

- World
- Window
- Grid
- Camera

を主要概念とする。

ユーザーは無限に続く2次元世界の上へウィンドウを配置し、ディスプレイはその世界の一部分を表示する「Camera」として振る舞う。

基本的なイメージ：

```text
                     Infinite World

              Window A
             ┌────────┐

      ┌────────────────────┐
      │       Camera       │
      │                    │
      │        ┌───────────────┐
      │        │   Window B    │
      │        │               │
      └────────┼───────────────┘
               │
               └───────────────┘

                                  Window C
                                 ┌────────┐
```

WindowはCamera境界を跨いでよい。

Camera境界は世界の境界ではない。

---

# 2. 最重要設計原則

Mioの設計では以下を原則とする。

## 2.1 少数のprimitive

可能な限り以下のような少数概念からすべてを構築する。

```text
World
Window
Grid
Camera
Focus
Selection
Transform
Property
Action
```

高水準機能はprimitiveの合成として実装する。

例：

```text
「右のwindowへ移動」

select(direction=right)
→ focus(target)
→ camera_to(target)
```

---

## 2.2 モードを増やしすぎない

内部world modelへ専用modeを増やさない。

例えば、

```text
overview mode
```

という別世界を作るのではなく、

```text
camera.zoom < normal_zoom
```

として表現する。

同様に、

```text
floating
```

は別layout systemではなく、

```text
grid constraint disabled
```

として扱う方向を優先する。

---

## 2.3 外部システムの複雑さをMio Coreへ持ち込まない

Wayland、Smithay、X11などの事情によって、Mio内部のworld modelを汚染しないこと。

例：

```text
Wayland / Smithay / XWayland
          ↓
       Adapter
          ↓
────────────────────
        Mio Core
────────────────────
World / Window / Camera / Grid
```

Mio Coreは可能な限りSmithayを知らない純粋Rustロジックとして実装する。

---

# 3. 技術構成

基本構成：

```text
Wayland Clients
      │
      ▼
┌─────────────────────────┐
│ mio-compositor          │
│                         │
│ Smithay                 │
│ protocol handlers       │
│ rendering               │
│ input                   │
│ outputs                 │
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│ mio-core                │
│                         │
│ World                   │
│ Window                  │
│ Grid                    │
│ Camera                  │
│ Focus                   │
│ Property                │
│ Actions                 │
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│ IPC / external shell    │
│                         │
│ Shirube / Kaname        │
│ Waybar等も利用可能       │
└─────────────────────────┘
```

推奨workspace構成：

```text
mio/
├── Cargo.toml
├── Cargo.lock
├── AGENTS.md
├── crates/
│   ├── mio-core/
│   └── mio-compositor/
├── src/
├── docs/
│   ├── architecture.md
│   └── smithay-notes.md
└── config/
```

SmithayそのものをMio repoへコピーしないこと。

開発環境では必要に応じて以下のように別cloneを配置する。

```text
~/src/
├── mio/
├── smithay/
└── niri/
```

`smithay/` および `niri/` は原則として参照用であり、明示的に指示されない限り変更しないこと。

---

# 4. Smithayとの責務分離

Smithay側へ任せるもの：

- Wayland display
- protocol infrastructure
- seat
- keyboard
- pointer
- output
- rendering backend
- DRM/KMS
- xdg-shell
- layer-shell
- popup infrastructure
- damage tracking
-その他低レイヤー処理

Mio側で決定するもの：

- Windowのworld position
- Window size
- tiled / floating
- Window placement
- Window resize
- Focus
- Directional selection
- Camera position
- Camera zoom
- Camera target
- Camera animation
- Overview
- Runtime window properties
- KeybindからActionへの変換
- Mio独自IPC

Smithay APIをMio Coreの型へ直接漏らさないこと。

---

# 5. World

Mioには上下左右へ続く2次元Worldが存在する。

概念的には無限。

実装では十分大きなsigned integer座標を使用してよい。

例：

```rust
struct GridPoint {
    x: i64,
    y: i64,
}
```

Worldにはworkspace boundaryを持たせない。

次のような構造は禁止する。

```rust
struct Workspace {
    windows: Vec<Window>;
}
```

Mioのwindowはworkspaceへ所属するのではなく、World上の座標を持つ。

---

# 6. Grid

Worldは論理Gridを持つ。

GridはWindowの配置・リサイズに使用する最小単位である。

Cameraが表示する領域は複数Grid Cellからなる。

例としてCamera viewportを4×4 Cellと説明することがあるが、

**4×4は確定値ではない。**

built-in defaultと配布設定は、より細かなmove/resizeを可能にする8×8 viewportおよび
8×8 initial Window sizeを使用する。両者を同じ値にすることで初期Windowは画面全体を占める。

これはCameraとWindow gridの粒度関係を説明する例にすぎない。

実際の分割数は後から変更可能な構造にしておくことが望ましい。

---

# 7. Window

WindowはWorld上の矩形である。

基本構造：

```rust
struct GridRect {
    x: i64,
    y: i64,
    width: u64,
    height: u64,
}
```

Windowは最低限以下を持つ。

```rust
struct MioWindow {
    id: WindowId,
    rect: GridRect,
    state: WindowState,
    properties: WindowProperties,
}
```

ただしSmithay固有型はmio-coreへ直接入れない。

---

# 8. Tiling

Mioは基本的にtiling WMである。

通常WindowはGridへsnapされる。

WindowはGrid単位で、

- move
- resize
- place

できる。

新規Windowは、Cameraから見えるWindowがなければCamera中央へ配置する。見えるWindowが
あればfocused Windowを基準とし、未指定方向はrightとする。`SetNextPlacement` Actionは
実行時のfocused Windowと方向を次の配置基準として記録し、そのWindowの指定側から空いて
いる位置を探す。指定は次の成功したplaceで消費し、永続的なlayout modeにはしない。

Tiled Windowのresizeでは、変化する各辺に接する側のTiled Windowを同じ差分だけ
移動する。接するWindowが同じ方向の別のTiled Windowへ接していれば連鎖する。隣接関係は
Action実行時のGridRectから導出し、永続的なlayout groupは持たない。全変更は原子的に
検証・適用し、Floating Windowは追従対象にしない。追従案だけが他のTiled Windowとの
重なりまたは座標overflowで成立せず、対象Window単独のresizeが成立する場合は、追従を
取り消して対象のresizeだけを適用する。対象単独でも重なる場合は操作全体を拒否する。

Mioのtilingは、

> 現在画面を分割する

という概念より、

> World上のGrid領域をWindowが占有する

という概念として扱う。

---

# 9. Floating

必要なWindowはfloating化できる。

Floating Windowも同じWorld上へ存在する。
Floating Windowの矩形はTiled occupancy判定から除外する。Tiled WindowはFloating Windowの下へ
moveおよびresizeでき、描画とpointer hit-testのstackingだけがFloatingを優先する。
新規Tiled Windowのanchor選択と空き領域探索からもFloating Windowを除外する。
また、Camera zoomは表示変換に限定し、配置判定には通常zoomのviewport範囲を用いる。

Floating専用workspaceや別空間を作らない。

概念的には、

```text
Tiled:
grid constraint = enabled

Floating:
grid constraint = disabled
```

と考える。

Window位置のsource of truthはCoreの連続World座標とする。Tiled Windowはその座標をGrid
境界へ拘束し、Floating Windowはcell未満の位置も保持できる。画面pixelはOutputとCameraに
依存するadapter上の表現であり、CoreのWindow座標として保存しない。

候補：

- continuous world coordinate
- finer logical coordinate

実装初期では単純な方式を採用してよい。

描画adapterではeffective `floating` Propertyからz-indexを導出し、floating WindowをTiled
Windowより上へ描画・hit-testする。同じz-index内の順序は通常のfocus/raise順に従う。
これはrender stackingの派生値であり、Coreへ別のfloating Window collectionやstacking
source of truthを追加しない。

adapterはxdg-dialog modal、xdg-toplevel parent、および両軸が固定されたsize constraintを
自動floatingの入力として合成し、共通のruntime `floating` Propertyへ変換する。判定状態は
adapterに留め、Coreへdialog種別を追加しない。

---

# 10. Camera

画面はWorldを表示するCameraである。

Cameraは最低限以下を持つ。

```rust
struct Camera {
    logical_position: WorldPoint,
    render_position: RenderPoint,
    zoom: f64,
    target_zoom: f64,
}
```

必要に応じてtarget positionを分離する。

```rust
target_position
```

も持たせてよい。

CameraはWorldの一定範囲をviewportとして表示する。

---

# 11. Camera Stop

通常Cameraはviewport一つ分ずつ移動する。

例：

viewportが4 Grid Cell幅の場合、

```text
0
4
8
12
16
...
```

と移動する。

ただし、この4という値は例。

重要なのは、

> Cameraには自然な停止地点が存在するが、Worldにはその境界が存在しない

という点である。

したがって、

```text
Workspaces are camera stops, not containers.
```

という説明は概念理解には近い。

ただし実装上はworkspace自体を作らない。

---

# 12. Camera境界を跨ぐWindow

WindowはCameraの通常停止地点の境界を自由に跨ぐことができる。

例：

```text
Camera A       Camera B

0 1 2 3 | 4 5 6 7
        |
    ┌─────────┐
    │ Window  │
    └─────────┘
```

これは特殊ケースではない。

Windowは単にWorld上のRectなので、Cameraがどこに停止するかとは無関係である。

描画時にCamera viewportとWindow rectの交差部分を表示する。

---

# 13. Rendering Transform

World座標からscreen座標へCamera transformを行う。

概念式：

```text
screen_x = (world_x - camera_x) * zoom
screen_y = (world_y - camera_y) * zoom
```

実際には、

- output size
- logical pixels
- scale factor
- fractional scale

等を考慮する。

重要なのは、World座標とrender座標を分離すること。

## Screen capture

Smithayの`ImageCopyCaptureState`、`ImageCaptureSourceState`、
`OutputCaptureSourceState`を使用して`ext-image-copy-capture-v1`を第一のcapture経路とする。
最終描画後かつsubmit前のframebufferをSHMへreadbackし、Smithayの`Frame::success`で
完了とpresentation timeを通知する。`xdg-desktop-portal-wlr`はこの標準経路をlegacy
`wlr-screencopy`より優先して使用する。

`zwlr_screencopy_manager_v1`はgrim等との互換用として残す。Output全体と
Output-local regionを扱い、regionは
Output boundsへclipする。clientが渡すbufferは広告したARGB8888、寸法、strideと一致する
場合のみ使用する。`copy_with_damage`は初期段階では全capture領域をdamageとして返す。

capture用sceneはpointer overlayを含めず、`overlay_cursor`による追加合成も行わない。
直接backendでは最終sceneを一時GLES targetへ描画し、既存のSHM readback経路へ渡す。
通常のDRM scanoutは変更しない。sandbox化されたapplicationはportalの画面選択を経由する。
通常Wayland socketへ直接接続できる非sandbox clientは同じdesktop sessionの信頼領域として
標準・legacy両globalを利用できる。session lock中はcapture要求を拒否する。

---

# 14. Logical State と Render State

CameraおよびWindowの論理状態と、アニメーション中の描画状態を分離する。

例：

```text
logical camera:
0 → 4

render camera:
0.0
0.3
0.9
1.7
2.8
3.6
3.95
4.0
```

この考え方を、

- camera movement
- window movement
- resize
- zoom
- opacity

などへ共通利用する。

Animation専用の特殊状態を大量に増やさない。

---

# 15. Overview

Overviewは専用の別UIではない。

**Cameraを遠くすることで実現する。**

例：

```text
normal:
zoom = 1.0

overview:
zoom = 0.35
```

Overview時もWorldとWindow座標は変化しない。

Cameraのzoomと必要に応じたpositionのみ変化する。

対象Windowを選択した場合：

```text
select window
→ camera moves to target
→ zoom returns to normal
→ focus target
```

というprimitiveの合成として実装する。

公開操作と配置への影響は`docs/overview.md`を正とする。

---

# 16. Focus

FocusはWorld座標を基準にする。

Directional focusとして最低限、

```text
left
right
up
down
```

を持つ。

候補WindowをWorld上の位置関係から選択する。

focusが空の場合、最初のDirectional focusはCamera中心に最も近いWindowを選び、次回以降の
方向選択に使う基準を復旧する。この選択でCamera位置は暗黙に変更せず、通常のFocusと
CameraFollowの合成側が必要な追従だけを行う。

アルゴリズムの詳細は後で調整可能にする。

初期実装では、

- 指定方向に存在する
- 方向角
- 距離

を考慮する単純なnearest方式でよい。

---

# 17. Action

Mio内部では、ユーザー操作を可能な限りActionとして表現する。

例：

```text
Focus(Direction)
MoveWindow(Direction)
ResizeWindow(...)
ResizeWindowRect(...)
CameraStep(Direction)
CameraNudge(Direction)
CameraTo(WindowId)
CameraFollow(WindowId)
CameraCenter(WindowId)
CameraZoom(...)
ToggleFloating
CloseWindow
Spawn(...)
SetWindowProperty(...)
ClearWindowProperty(...)
```

KeybindやIPCから直接内部状態を書き換えず、原則Actionを経由する。

絶対倍率のkeybindは`bind "Super+5" "camera-zoom" 0.5`のように宣言する。
設定値はMouse zoomと同じ`0.1..=1.0`とし、初期設定では`Super+1`〜`Super+9`を
`0.1`〜`0.9`、`Super+0`を最大倍率`1.0`へ割り当てる。

将来的にYaldraからも同じAction / primitiveを使用する。

通常のfocusは`Focus`または`FocusWindow`と`CameraFollow`を合成する。
`CameraFollow`は現在のzoomから実際に見えるWorld範囲を導出する。対象が完全に見えていれば
Cameraを維持し、一部が見切れていれば包含に必要な最小距離だけ移動し、完全に見えない場合
または見える範囲より大きい場合は`CameraCenter`と同じ位置へ移動する。zoom自体は変更しない。
同じfocused Windowへのprotocol上のactivation再通知では`CameraFollow`を再適用しない。
これは手動の`CameraStep`、`CameraNudge`、`CameraPan`で選んだ視点を維持するためである。
ただし、新規Windowの初回presentationはcommit待ちの間にprotocol focusを先行設定していても
意味的な新規focusとして扱い、必ず`CameraFollow`を適用する。
resizeはWindow geometryだけを変更し、Cameraを暗黙に移動しない。

pointer focusで非focused Windowをleft pressした場合、そのpressと対応するreleaseをadapterが
消費し、通常の`FocusWindow`と`CameraFollow`だけを適用する。focused Windowへのleft clickと
right clickのCamera操作・短いclick replayは従来どおりclientへ配送する。

---

# 18. Mouse Camera Operation

現時点で以下を候補仕様とする。

## IME protocol integration

`zwp_text_input_manager_v3`、`zwp_input_method_manager_v2`、および
`zwp_virtual_keyboard_manager_v1`はSmithay adapter内で同じ
Seatへ接続する。text-inputのfocused surface、preedit / commit、およびIME keyboard grabは
SmithayのSeat handlerを通じて転送する。候補Windowは`PopupManager`の既存popup treeへ所属し、
親toplevelまたはlayer surfaceのgeometryから配置する。IME popupをMio CoreのWindowやWorld
geometryとして保持しない。

現在のnested開発backendではinput-methodとvirtual-keyboardのglobalを全clientへ公開する。
system compositorとして運用する前にsecurity-context等を用いて信頼済みIMEへ制限する。

## 18.1 Right Drag

以下でいうbuttonは既定値である。実際の割当はKDLの単一`mouse` blockで宣言し、keyboardと
同じくadapter入力を既存Actionへ接続する。button名は`left`、`right`、`middle`を使用する。
2 button以上の指定は左から押す順序を表す。

```kdl
mouse {
    camera-pan "right"
    camera-zoom "right"
    move-window "right" "left"
    resize-window "left"
    reset-window "right" clicks=2
    toggle-floating "right" "middle"
    place-next "middle"
    center-window "right" "left" clicks=1
    close-window "right" "left" clicks=3
}
```

`close-window`は最初のbuttonを保持したまま、2番目のbuttonを設定回数clickするgestureである。
`move-window`と同じ2 buttonを同じ順序で指定し、先頭は`camera-pan`と一致させる。2番目のpress後に
pointerが閾値を越えて動けばWindow drag、同じWindow上で各click間275ms以内かつ6 logical pixels
以内で`clicks`回に達したら`CloseWindow` Actionとして判定する。途中のclickごとに期限を延長する。
静止したclick列が`center-window clicks`と一致した場合は、判定期限後または先頭buttonのrelease時に
`FocusWindow`と`CameraCenter`を合成し、Window sizeを変えずCamera中央へ配置する。それ以外の
未成立回数ではActionを実行しない。これによりclientへ片側だけのbutton eventを
配送せず、dragとcloseを同じprimitiveから安全に分岐する。不正な組合せはactionableな設定error
としてreloadを拒否し、直前の有効設定を維持する。
`clicks` propertyは宣言時に必須で1から5を受理する。closeの回数を暗黙の固定値にしない。
`center-window`も同じbutton列を使い、必須の`clicks`は1から5とする。centerとcloseの回数の
大小には意味を持たせないが、同じbutton列で同じ回数を指定するとgestureが衝突するため設定errorとする。
`reset-window`は設定click回数の判定中に未成立click列を短時間保留するので、`camera-pan`と同じ
buttonを指定する。`clicks`は宣言時に必須で1から5とする。異なるbutton指定はclient clickを
安全に再送できないため設定errorとする。

右ボタンを押したままCursorを動かすと、Worldを掴む向きへCameraを連続移動する。

```text
RMB + pointer drag
→ continuous camera pan
```

pointer deltaはCamera zoomとOutput sizeを考慮してWorld座標へ変換する。RMB press/releaseは
drag成立時だけcompositorが消費し、通常のCamera animationを介さず画面上のWorldをpointerへ
追従させる。drag閾値未満でreleaseした場合、設定されたOutput edgeなら対応する外部commandを
起動し、それ以外では通常のright clickとしてclientへ転送する。commandはshell文字列ではなく
programとliteral argvとして保持し、MioのWayland / IPC / XWayland endpointを環境へ渡す。

RMB held中のLMB pressはWindow dragへ切り替える。adapterは対象Windowの描画位置だけを移動し、
TiledとFloatingの両方をdrag中はpointerへ連続追従させる。最初のbutton releaseで、Tiledは
pointer deltaをCamera zoomおよびOutput sizeに応じたGrid deltaへ丸め、Floatingは連続deltaのまま
共有`MoveWindow` Actionへ渡す。Core geometryはdrag中にframe単位で
更新せず、左右両buttonのeventもclientへ漏らさない。drag中は対象をadapterのstack上で一時的に
raiseするだけでCore focusを変更しない。`MoveWindow`成功後にだけ`FocusWindow`を適用し、
`CameraFollow`は合成しない。移動失敗時は元のfocusを維持する。

RMB保持中のvertical wheelは`CameraZoom` Actionへ変換する。wheel downはzoomを下げ、wheel upは
上げるが、通常Cameraの`1.0`を越えて拡大しない。RMBを保持しないwheelはclientへ転送する。
例外として物理wheelをOutput端で回すと、共有`Focus` Actionへ変換する。左・右端は横方向の
操作領域として上回転をLeft、下回転をRightへ変換し、上・下端は縦方向の操作領域として
上回転をUp、下回転をDownへ変換する。cornerも既存の最寄り端判定を共有する。
trackpadのFinger/Continuous scrollは端でもclientへ転送する。

Window表示矩形の内側8 logical pixelsはpointer resize handleとする。hover中は辺に応じて
east/westまたはnorth/south、角では対応するdiagonal resize cursorをadapterが優先表示する。
LMB drag中の表示矩形とclient configure sizeはpointerのpixel deltaへ連続追従させる。同時に
Camera zoomとOutput sizeからGrid deltaへ変換し、Grid境界を越えるたび四辺を
`ResizeWindowRect` Actionへ渡す。このActionは対象矩形と変化した各辺に
接するTiled followerを一括検証・適用し、失敗時に部分的なgeometry変更を残さない。
追従案が不成立でも対象単独の矩形が有効なら、followerを元の位置に残してresizeだけを適用する。
drag開始時に同じCoreの導出queryから得たfollowerは、対象の掴んだ辺と同じpixel deltaで
一時表示位置を連続移動する。これはdrag中だけのpresentationであり、固定layout groupにしない。
新bufferを待つ間、旧bufferは縦横別倍率で矩形へ引き伸ばさず、等倍率で表示する。drag中は
FocusとCameraを維持し、終了時にpixel previewを破棄して最終Grid geometryへ収束させた後、
対象へFocusを移す。連続previewはadapter rendering stateでありCore geometryにしない。

RMB held中のMMB pressは、pointer下のWindowに既存の`ToggleFloating` Actionを予約する。
3 button chordへ発展しなかった場合だけ最初のbutton release時に適用する。adapterは保留中の
Camera drag候補を破棄し、RMBとMMBの両方がreleaseされるまで全button eventを消費する。
成功時は`FocusWindow`だけを合成し、`CameraFollow`は行わない。

Output端でMMBを単独clickすると、その端の方向を`SetNextPlacement` Actionとして予約する。
cornerではpointerに最も近い端を選ぶ。通常領域のMMBはclientへ配送する。

Window上でRMBを保持してLMBを設定回数clickすると、そのWindowへ`CloseWindow` Actionを適用する。
途中でpointerが移動閾値を越えた場合は従来どおりWindow dragとなる。close成立後は両buttonが
releaseされるまで後続eventを消費し、client、Focus、Cameraへ副作用を残さない。
静止したLMB single clickは対象へfocusし、`CameraCenter`でsizeを維持したままCamera中央へ戻す。

Window上で`reset-window` buttonを設定された回数clickすると、共有`ResizeWindow` Actionで対象を
初期幅とその半幅の間で切り替える。現在幅が初期幅なら半幅、半幅未満なら半幅、それ以外なら
初期幅とする。奇数の初期幅は半幅を切り上げ、高さは`placement.initial-size`へ戻す。
`toggle-window-size` keybindも同じActionへ変換し、`FocusWindow`と`CameraFollow`を合成して
Camera内へ収める。
同じbutton、同じWindow、6 logical pixels以内で275ms以内の2 clickだけをdouble clickとする。
最初の単clickは判定期限まで保留し、成立しなければ元のtimestampを持つpress/releaseとして
clientへ再送する。pointerが判定範囲外へ動いた場合は期限を待たず再送する。

---

# 19. Animation

Mioの「水」のテーマは主に動きで表現する。

過剰な水滴演出や装飾ではなく、

- camera movement
- zoom
- focus transition
- resize
- subtle ripple

などで流体感を出す。

Focusの静止時の目印はWindow下辺全体のごく薄い水膜と、中央から左右へ減衰する
細い水光の組み合わせとする。水光の細い芯の周囲には、Window内容の視認性を損なわない
低輝度の広いhaloを持たせ、特にWindow下端から下方向へ淡く減衰させる。単なる境界線ではなく
下側へ光を落とす発光として読めるようにする。haloは細い芯より横方向にも広く拡散させ、
最大Camera倍率でも短い白線だけに見えない範囲を確保する。
これはMio全体のVisual effectとは別の設計判断であり、blur、shadow、ripple、transition
などの表現を禁止または軽視するものではない。

Animationは基本的に、

```text
current
target
interpolation
```

という共通モデルで扱う。

設定では最低限、

```text
global animation speed
camera animation
window animation
zoom animation
```

程度を調整可能にする。

---

# 20. Window Properties

Windowには見た目および挙動のPropertyを持たせる。

例：

```text
opacity
blur
focus indicator
focus indicator width
focus indicator color
corner radius
shadow
floating
animation enabled
camera follow
```

Propertyは可能な限り汎用化する。

専用コマンドを大量に追加するのではなく、

```text
set property
clear property
toggle property
```

で操作できる構造を優先する。

---

# 21. Property Priority

Window Propertyには以下の優先順位を持たせる。

```text
Default
  ↓
Matched Config Rules
  ↓
Runtime Override
```

例：

```text
Default opacity:
0.95

Firefox rule:
0.90

Current window runtime override:
1.00

Effective:
1.00
```

Runtime overrideを解除するとConfig Ruleへ戻る。

---

# 22. Runtime Window Override

Runtime中にfocused WindowのPropertyを変更できる機能を正式仕様として持たせる。

これは重要機能である。

用途例：

```text
focused window opacity toggle
focused window blur toggle
focused window floating toggle
focused window focus-indicator toggle
```

外部scriptでconfigを書き換える必要がないようにする。

Keyboardの `toggle-blur` はfocused Windowのeffective blur値を反転し、既存の
`SetWindowProperty` Actionとしてruntime overrideへ適用する。専用のblur状態や設定ファイル
書き換え経路を作らない。

`toggle-opacity` が往復する2値はglobal appearance設定の `opacity-toggle` で指定する。

カーソル航跡はglobalな描画効果であり、`cursor-wake`、速度閾値、屈折強度、
入力幅、持続時間をeffects設定から指定する。`toggle-cursor-wake` は設定値を
書き換えず、実行中の有効状態だけを反転する。無効化時は入力履歴とrendererの
水面キャッシュを破棄し、再有効化しても古い波を再表示しない。
航跡はWorld WindowとBackground/Bottom layerへ作用させ、launcher、bar、notification等の
Top/Overlay layer-shell surface自体は歪ませない。これをapp-id別の例外ではなくshell layerの
描画境界として扱う。

Window開閉transitionは`window-transition`で`water`、`sci-fi`、`none`を
選択する。どの方式も同じOpening / Stable / Closingと進捗値を共有し、renderer shader
だけを切り替える。`window-transition-duration`で基準時間をミリ秒指定する。globalな
`animation.speed`は他のanimationと同様にこの基準時間へ倍率として作用する。
`water`はWindow像の全面を一つの水面として扱い、時間全域で全領域を同時にfadeさせながら
低周波の屈折、にじみ、緩やかな濃淡差によって像全体を液状化し、水へ溶けるように透明化する。
均一fadeだけに見えない強さを持たせるが、上下左右へ走査する境界、部分的な出現順、
細かな粒状欠落、発光するcrestは使用しない。OpeningとClosingを別演出へ分岐せず、同じ進捗の
厳密な逆再生として凝結と溶解を表現する。大きな放射状変形は行わない。
clientが自発的にtoplevelを破棄した場合は、最後に正常描画できたWindow単位のGPU
snapshotをrenderer adapterがClosingVisualとして短時間保持する。WindowはCore Worldから
即座に除去し、snapshotの寿命やtransition進捗をCore stateへ追加しない。snapshotは
surface commit時にだけ更新し、通常frameごとの無条件な再描画を行わない。
操作結果はblurと同様にfocused Windowのruntime Property overrideであり、KDL自体を変更しない。

例となるAction：

```text
SetWindowProperty {
    target: Focused,
    property: Opacity,
    value: 1.0,
}

ClearWindowProperty {
    target: Focused,
    property: Opacity,
}
```

Window単位でOverrideできること。

同じapp-idの他Windowへ自動波及させない。

---

# 23. Configuration

通常設定はKDLを第一候補とする。

将来的にYaldraも利用可能にする。

役割：

```text
KDL
= declarative configuration

Yaldra
= programmable configuration
```

KDLでは既存のMio behaviorのパラメータを設定する。

設定はtop-levelの`include "PATH"`で別のKDLファイルを記述位置へ合成できる。相対pathは
記述元ファイルを基準とし、再帰的な読み込みを許すが循環参照はerrorとする。生成予定の任意設定を
参照できるように、指定先が存在しない場合だけは無視する。存在するファイルの読み取り失敗や
不正な内容は通常の設定errorとして扱う。

起動時commandはtop-levelの`spawn-at-startup "PROGRAM" "ARG"...`として複数記述する。
Wayland socketとIPC socketの準備後に一度だけargvを直接spawnし、shellを介さず、設定reload
では再実行しない。これは外部shellをMio Coreへ取り込む機能ではない。
keybindからの外部commandも`bind "CHORD" "spawn" "PROGRAM" "ARG"...`として同じadapter側の
spawn経路を使う。通常Action名との曖昧さを避けるため、program名をAction位置へ直接書かない。

Yaldraではprimitiveを組み合わせ、behaviorそのものを記述できるようにする。

---

# 24. KDL設定カテゴリ

初期候補：

```text
appearance
effects
animation
camera
outputs
input
window-rule
placement
output
```

必要以上にカテゴリを増やさない。

---

# 25. Appearance

設定可能候補：

```text
background-color
window-border-width
window-border-color
focus-indicator-width
focus-indicator-height
focus-indicator-color
corner-radius
opacity
inactive-opacity
shadow
gaps
```

色は外部theme generator等から設定できるようにする。

`gaps` は通常Windowに割り当てられた表示矩形の四辺を、指定したlogical pixel数だけ
内側へ縮める。隣接する2つのWindow間にはその2倍の空間が生じる。これはadapter側の
表示とclient configureにだけ作用し、World geometry、Grid制約、隣接判定を変更しない。
Camera zoom時の表示gapはzoomと同じ比率で縮尺し、遠景でもWindowとgapの比率、および
World上の占有範囲に対する見た目の対応を維持する。clientの通常configure sizeは変更しない。
最大化およびfullscreenのWindowには適用せず、0で無効化する。
Fullscreenでは角丸を0として描画する。Fullscreen中はKeyboard、Mouse、IPCに共通する
Camera移動およびzoom ActionをCoreが拒否し、解除時までCamera状態を保持する。

Mio内部で巨大なtheme systemを作る必要はない。

Shadowの既定offsetは`0 0`とし、明るい背景でもWindowとShadowの間に背景色の帯を
作らない。方向性のあるShadowが必要な場合だけ設定で明示的にoffsetを与える。

---

# 26. Effects

設定候補：

```text
blur
ripple
shadow
opacity
```

Blur等はWindow RuleおよびRuntime Overrideから変更可能にする。

例：

```kdl
window-rule {
    match app-id="mpv"

    opacity 1.0
    blur false
}
```

---

# 27. Animation Configuration

全体速度を簡単に変更できること。

例：

```kdl
animation {
    speed 1.0
}
```

必要ならカテゴリ別override：

```kdl
animation {
    speed 1.0

    camera {
        speed 1.2
    }

    window {
        speed 0.9
    }
}
```

内部のspring parameterをそのまま大量に公開しない。

Advanced optionとして必要な場合のみ追加する。

設定ファイルを内部実装のdumpにしないこと。

---

# 28. Keybind

KeybindからMio Actionを呼び出す。

例：

```text
Super+H
→ Focus(Left)

Super+J
→ Focus(Down)

Super+K
→ Focus(Up)

Super+L
→ Focus(Right)
```

Keybind parserとAction implementationを分離する。

将来的にYaldraでも同じActionを呼べるようにする。

---

# 29. Window Rule

Window Ruleは単一の汎用mechanismとする。

「appearance rule」「floating rule」「placement rule」などへ不要に分割しない。

Match候補：

```text
app-id
title
```

将来候補：

```text
role
fullscreen
floating state
```

Ruleによって設定可能なProperty例：

```text
opacity
blur
floating
focus indicator
corner radius
shadow
initial size
placement
focus on spawn
camera follow
animation
```

正規表現またはglob matchingは将来対応候補。

現在の`app-id`と`title`は完全一致であり、両方を指定したRuleはAND条件とする。複数Ruleが
一致した場合はconfig記述順にPropertyを合成し、同じPropertyだけを後のRuleで上書きする。
metadata変更またはconfig再読み込み時はConfig Rule層を再計算するが、Window単位のRuntime
Override層は保持する。公開上の詳細は`docs/window-properties.md`を正とする。

---

# 30. ConfigとRuntimeを分離

Mio自身が人間の手書きconfigを頻繁に直接書き換えないこと。

Runtime overrideはmemory上で保持する。

永続保存機能を追加する場合は、

```text
generated-overrides.kdl
```

など、人間のconfigとは別ファイルを使用することを優先する。

コメントや整形を破壊しないこと。

---

# 31. Placement

New Window placementは初期では単純に実装してよい。

候補：

```text
nearest free area
focused window vicinity
current camera viewport
configured direction
```

MioのWorldは無限なので、既存Windowを無理に縮小するより、空いているWorld領域へWindowを配置する方式も積極的に検討する。

複雑なplacement algorithmは将来Yaldraへ委譲可能な設計とする。

---

# 32. XWayland

X11 application compatibilityは必要。

初期方針として、built-in XWayland管理よりも、

**xwayland-satellite**

利用を有力候補とする。

概念：

```text
X11 app
  ↓
xwayland-satellite
  ↓
Wayland
  ↓
Mio
```

Mio CoreではX11 Windowを特別扱いしないことを目標とする。

初期実装では`--xwayland-satellite`を明示した場合だけPATH上のsatelliteを起動する。
X displayは既定で`:100`とし、`--xwayland-display :NUMBER`で変更可能にする。satelliteには
MioのWayland socketを渡し、正常に起動している場合だけMioがspawnするapplicationへ同じ
`DISPLAY`を直接渡す。Mio process全体の環境変数は書き換えない。satelliteの起動失敗または
異常終了はnative Wayland compositorを停止させず、利用可能なX display状態を解除する。
正常終了時には子processを停止・回収する。

固定display番号の衝突回避と`-listenfd`を使ったon-demand activationは後続改善とする。

必要になった場合のみbuilt-in XWaylandを再検討する。

`mio-compositor --help`と`-h`はbackendやWayland socketを初期化せず、起動application、
設定検証、X11互換、開発用virtual Outputを含むoption一覧を表示して正常終了する。
`--version`と`-V`もbackendを初期化せず、実行中buildのversionを表示する。

---

# 33. External Shell

Mioはbarやlauncherを必須内蔵しない。

純正環境：

```text
Mio
Shirube
Kaname
```

ただしMio単体では、

```text
Waybar
fuzzel
custom Quickshell
etc.
```

なども使用可能にする。

Shirube / Kanameを必須依存にしない。

---

# 34. IPC

External shellやCLIからMioへアクセスできるIPCを提供する。

公開するsnapshot情報：

```text
windows
focused window
window title
app-id
window rect
window presentation
camera position
camera zoom
outputs
```

Action command：

```text
focus window
camera to window
set window property
clear window property
move window
resize window
toggle floating
close window
```

IPC側でも内部Actionを再利用する。

初期transportは`$XDG_RUNTIME_DIR/mio-wayland-N.sock`形式のUnix socketとする。Mioは実際の
pathを`MIO_SOCKET`環境変数として子applicationへ渡す。一接続につきUTF-8のcommand lineを
一つ受け取り、上限4096 byte、最初の改行またはEOFでrequest完了とする。serverは未完了の
requestへ短いread timeoutを適用し、socket permissionを`0600`とする。responseは改行を
必要としないJSON objectとする。response書き込みにも短いtimeoutを適用し、一回の
event-loop dispatchで処理する接続数に上限を設ける。残った接続はlevel-triggeredな次の
dispatchで処理し、IPC集中時にもWayland入力と描画を進行させる。

初期command：

```text
quit
state
windows
focused-window
camera
set-opacity ID VALUE
clear-opacity ID
camera-to ID
focus ID
camera-step DIRECTION
move-window ID DIRECTION
resize-window ID DIRECTION
toggle-floating ID
close ID
set-property ID opacity FLOAT
set-property ID floating BOOL
set-property ID blur BOOL
clear-property ID blur
clear-property ID PROPERTY
```

`quit`はWorldを変更するActionではなくcompositor lifecycle commandとする。成功応答を返して
event loopを停止し、SIGINT / SIGTERMと同じcleanup経路へ進む。

`state`はWindow一覧、focused Window、active Camera、Output一覧、Output Camera一覧を、一回の
request中に同じWorld / adapter stateから取得したJSON snapshotとして返す。外部shellは複数の
read requestを組み合わせた際の一時的な不整合を避けられる。各Windowの`presentation`は
`normal`、`maximized`、`fullscreen`のいずれかを返し、`floating`とは独立して公開する。

clientからのfullscreen requestは動画などの標準Wayland用途のため受理する。maximizeおよび
unmaximize requestは、applicationが以前のsession状態を復元してMioのWorld layoutを暗黙に
変更することを避けるため受理しない。最大化はkeyboard、mouse、IPC等から共有
`ToggleMaximized` Actionを明示的に実行した場合だけ変更する。

`mioctl`は`MIO_SOCKET`を既定接続先とし、Mio外部からは`--socket PATH`も指定可能とする。
引数なし、`--help`、`-h`、`help`ではMioへの接続を要求せず、利用可能なread / Action
command、方向、Property値の形式を表示する。
`--version`と`-V`もMioへの接続なしでCLI versionを表示する。
transport errorまたは`{"ok":false}`応答では非ゼロ終了し、成功応答だけを標準出力へ出す。
Window / Camera / PropertyのAction commandは必ず`World::apply(Action)`を経由し、成功後に
adapter layoutを同期する。compositor lifecycleの`quit`はこの対象外とする。
初期serverは短命なlocal CLI接続を対象とする。subscription、非同期event stream、認証、
長時間接続clientはstable IPC段階へ延期する。

---

# 35. Yaldra Integration

YaldraをSmithayへ直接接続しない。

構造：

```text
Smithay / Rust
      ↓
Mio Core API
      ↓
Yaldra Runtime
```

Yaldraへ公開する対象：

```text
World
Window
Camera
Selection
Focus
Action
Property
```

公開しない対象：

```text
raw Smithay state
wl_surface internals
GPU buffers
DRM internals
raw protocol object lifetime
```

YaldraはMioの意味論を操作する。

Wayland低レイヤーを直接操作させない。

---

# 36. KDLとYaldraの役割

KDL：

```text
値を変える
presetを選ぶ
window ruleを書く
keybindを書く
```

Yaldra：

```text
behaviorを作る
primitiveを合成する
focus algorithmを書く
placement algorithmを書く
camera behaviorを書く
macroを定義する
```

Yaldraを使用しなくてもMioは完全に起動・利用可能であること。

---

# 37. 3Dについて

World自体を3Dにしない。

Window placementは2D。

3D depth、perspective、camera rotation、occlusionなどをCore conceptへ追加しない。

将来的にvisual effectとしてpseudo-3Dを加えることは妨げない。

```text
World = 2D
Render effects = optionally pseudo-3D
```

とする。

---

# 38. 非目標

初期開発では以下を優先しない。

- 独自GUI toolkit
- 独自notification daemon
- 独自barのMio本体内蔵
- 独自launcherのMio本体内蔵
- 大量のshader effect
- 3D window world
- 複数の複雑なlayout engine
- X11固有機能の完全再現
- Smithayのfork
- 過度な設定項目
- 全protocolの即時対応

---

# 39. MVP

最初のMioは以下だけでよい。

```text
- Smithay compositor起動
- xdg-toplevel表示
- keyboard input
- pointer input
- focus
- infinite 2D world
- basic grid
- tiled window placement
- move
- resize
- camera movement
- windows can cross camera boundaries
```

この時点でMio固有の概念実証は成立する。

UIの美しさやeffectsは後回し。

---

# 40. 第2段階

MVP後：

```text
- floating
- fullscreen
- layer-shell
- popups
- clipboard
- basic configuration
- keybind
- window rules
- runtime window property overrides
```

---

# 41. 第3段階

その後：

```text
- smooth camera animation
- overview / camera zoom
- mouse camera operations
- IPC
- xwayland-satellite
- multi-output
- IME
- screencopy / screen sharing
- session lock
- fractional scaling
```

## Multi-output Camera

Worldは一つのまま、各Outputが独立したCameraを持つ。

```text
Output A - Camera A -+
                     +-- one World -- Windows
Output B - Camera B -+
```

入力を受けたOutputをactive Outputとし、Camera移動、zoom、新規Window配置はその
Cameraへ作用する。Output切替はCamera Actionの宛先を変えるだけであり、Windowを
Output間で移籍させない。

単一のnested画面で使う --virtual-outputs は初期開発用であり、複数Cameraをhost
Window内で横方向に分割表示する。各Window surface treeはCameraごとの派生Render
Elementとして生成し、仮想Output領域でcropする。Core Windowとclient surfaceは
複製しない。実際の複数 wl_output 公開は後続のadapter実装で行う。
仮想Cameraは共有Action cycle-output（既定 Super+N）で巡回できる。

---

# 42. 第4段階

安定後：

```text
- subtle visual effects
- blur
- ripple
- Shirube integration
- Kaname integration
- Yaldra programmable configuration
```

---

# 43. Smithay調査方針

Smithay全部を最初から読む必要はない。

調査優先順位：

```text
1. Smithay Getting Started
2. smallvil
3. Mio Core設計
4. Anvilの必要部分
5. niriの関連部分
6. Smithay本体の該当module
```

必要なコードだけ調査する。

調査した内容は、

```text
docs/smithay-notes.md
```

へ簡潔に記録する。

Mioが利用しているSmithay revisionと、参照用ローカルcloneのrevisionを可能な限り一致させる。

---

# 44. Codexへの実装ルール

Codexは以下を遵守すること。

1. 新しい大規模dependencyを勝手に追加しない。
2. Smithay本体を変更しない。
3. niri等の参考repoを変更しない。
4. Mio CoreへSmithay型を漏らさない。
5. 特殊ケースを追加する前に既存primitiveで表現できるか確認する。
6. 同じ情報を複数箇所で状態管理しない。
7. Window geometryのsource of truthを一つにする。
8. Cameraのlogical stateとrender stateを区別する。
9. Configurationとruntime overrideを区別する。
10. 高水準操作は可能な限りActionへ落とす。
11. 実装前に既存architectureと矛盾しないか確認する。
12. 不明なSmithay APIを推測だけで使用せず、対象revisionのsource/exampleを確認する。
13. 既存testsを壊さない。
14. mio-coreへ可能な限りunit testを書く。
15. 最初から全protocol対応を狙わない。

---

# 45. 設計判断の優先順位

判断に迷った場合は、以下の順で優先する。

```text
1. Conceptual simplicity
2. Internal consistency
3. Daily usability
4. Extensibility
5. Visual novelty
```

見た目の面白さのためにWorld modelを複雑化しない。

---

# 46. Mioの中心的な一文

Mioの設計を一文で表すなら：

> 世界は一つであり、Windowはその上に存在する。画面は、その世界を覗くCameraにすぎない。

そして開発原則は：

> 新しい機能を新しい概念として追加する前に、既存の少数のprimitiveから導出できないかを考える。

この2点をMioの設計上の最重要原則とする。
