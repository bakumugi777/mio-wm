# 澪 / Mio — 要件定義書

## 0. 文書目的

本書は、Waylandコンポジタ / ウィンドウマネージャ **「澪 / Mio」** の要件を定義する。

本書では主に、

- Mioが提供すべき機能
- Mioが満たすべき品質
- Mioの基本的なユーザー体験
- 初期リリースで必要な範囲
- 将来拡張を見越した制約

を定める。

具体的な内部実装方法については、別途 `spec.md` 等の設計仕様書を参照する。

---

# 1. 製品概要

Mioは、Wayland上で動作する独自のtiling compositor / window managerである。

一般的なworkspace型WMとは異なり、Mioではデスクトップ全体を、

**上下左右へ連続する2次元World**

として扱う。

WindowはそのWorld上へ配置される。

DisplayはWorldの一部を表示するCameraとして扱われる。

Mioは、

- タイリングによる整理性
- 2次元空間による自由度
- Cameraによる空間移動
- 少数ルールによる一貫性

を両立することを目的とする。

---

# 2. 基本理念

Mioは以下を主要理念とする。

## 2.1 世界は一つ

Windowをworkspace単位の独立空間へ分離しない。

すべてのWindowは共通のWorld上に存在する。

---

## 2.2 Windowは場所を持つ

Windowは単に「workspace 3に所属する」のではなく、World上の位置と大きさを持つ。

ユーザーはWindowを空間的な位置関係として認識できること。

---

## 2.3 DisplayはCameraである

画面はWorldそのものではなく、その一部分を映すCameraである。

Cameraを移動することで別領域を見る。

---

## 2.4 少ない原理で多くを表現する

Overview、floating、window jump等の機能を、それぞれ独立した複雑な仕組みとして実装するのではなく、可能な限り既存のWorld / Window / Camera / Gridの原理から導出する。

---

# 3. 想定ユーザー

主な対象は以下。

- Linux / Wayland利用者
- tiling WM利用者
- workspace型WMに窮屈さを感じる利用者
- niri等の空間的WMを好む利用者
- キーボード操作を重視する利用者
- マウス操作も併用したい利用者
- WMを細かくカスタマイズしたい利用者
- シンプルな内部原理を好む利用者

Mioは純正shellであるShirube / Kanameの利用を必須としない。

---

# 4. 対応環境要件

## 4.1 OS

Linux上で動作すること。

---

## 4.2 Display Protocol

Wayland compositorとして動作すること。

X11アプリケーションについては、XWayland互換手段を提供することを目標とする。

---

## 4.3 実装言語

主要実装言語はRustとする。

---

## 4.4 Compositor Framework

Smithayを主要基盤として使用する。

---

# 5. World要件

## FR-WORLD-001

Mioは2次元Worldを持たなければならない。

---

## FR-WORLD-002

Worldはユーザー視点では上下左右へ事実上無限に続くものとして扱わなければならない。

---

## FR-WORLD-003

Worldはworkspaceによって分割されてはならない。

---

## FR-WORLD-004

WindowはWorld上の座標を持たなければならない。

---

## FR-WORLD-005

Windowの存在位置はCamera位置に依存してはならない。

Cameraが移動してもWindowのWorld上の位置は変化しないこと。

---

# 6. Grid要件

## FR-GRID-001

WorldはWindow配置用の論理Gridを持たなければならない。

---

## FR-GRID-002

Tiled WindowはGrid単位で配置できなければならない。

通常zoom相当のCamera viewport内にTiled Windowが存在しない場合、新規Tiled Windowは
現在のCamera中央を基準に
配置すること。Camera内にTiled Windowが存在する場合はfocused Tiled Windowを配置基準とし、
方向が未指定ならその右側から空き位置を探すこと。focused WindowがFloatingなら、可視Tiled
Window群の外周を基準にすること。配置方向を指定した場合は、指定時のfocused Tiled Windowと
方向を次の成功した配置1回に限って使用すること。Floating Windowの位置と大きさは新規Tiled
Windowの配置および空き領域探索に影響させないこと。
Overview等のzoomは表示変換であり、新規Windowの配置判定範囲を拡大しないこと。

---

## FR-GRID-003

Tiled WindowはGrid単位でresizeできなければならない。

resizeで移動する辺に接するTiled Windowは、現在のGrid上の隣接関係から導出し、
連鎖を含めて同じ差分だけ移動すること。固定的なlayout groupを別状態として持たず、
resizeと隣接Windowの移動は全体を原子的に適用すること。Floating Windowはこの連鎖へ
参加しない。

Floating Windowは同じWorld上でGrid拘束だけを解除し、Grid cell未満の連続したWorld位置へ
配置できること。Tiled Windowへ戻すときは最寄りのGrid位置へ整列し、既存のTiled Windowと
重なる場合は変更を拒否すること。

---

## FR-GRID-004

Camera viewportは複数のGrid Cellから構成されること。

---

## FR-GRID-005

ViewportのGrid分割数はKDL設定から変更可能であること。built-in defaultは8×8とし、
default initial Window sizeも8×8として画面全体を占める初期表示を維持すること。

---

# 7. Window要件

## FR-WIN-001

Mioは複数のWayland toplevel Windowを管理できなければならない。

---

## FR-WIN-002

WindowはWorld上の矩形領域を持たなければならない。

---

## FR-WIN-003

WindowはCamera viewportの境界を跨いで配置可能でなければならない。

---

## FR-WIN-004

Camera viewport境界を跨いだWindowは、Camera位置に応じて見えている部分のみ正しく描画されなければならない。

---

## FR-WIN-005

Windowはmove可能でなければならない。

---

## FR-WIN-006

Windowはresize可能でなければならない。

---

## FR-WIN-007

Windowはclose可能でなければならない。

非focused Windowの破棄は現在のfocusやCamera位置を変更してはならない。通常のfocused
Windowを破棄した場合は残存Windowを暗黙に選ばず、Seat focusを空にしてCamera位置を維持すること。
この空のfocus状態からDirectional focusを実行した場合は、Camera中心に最も近い残存Windowを
最初の基準としてfocusし、以後のDirectional focusは通常のWorld上の位置関係を使用すること。
破棄したxdg-toplevelに生存中のparentがあれば、その親を汎用fallbackより優先し、通常の
FocusWindow Actionで復帰するがCamera位置は変更しないこと。
parent自身が一時的なdialogである場合は、直接parent、祖先、自動focus直前のWindowを
優先順位付きの復帰候補としてadapterで派生すること。閉じる時点で最初に生存している候補へ
復帰し、中間dialogが残っていればそこへ、先に破棄されていれば元のapplication Windowへ
戻ること。この派生情報をCoreのWindow所有関係にしてはならない。

pointerで得たsurfaceからWindowを逆引きする場合、root xdg-toplevelだけでなくsubsurfaceと
popupを含むsurface tree全体を対象にすること。子surfaceのクリックでも所有Windowへfocusし、
同じ通常のCamera追従経路を使うこと。
非focused Windowに対する最初のleft clickはfocusだけに使用し、pressとreleaseのどちらも
clientへ転送しないこと。すでにfocused Windowへのleft clickは通常どおり転送すること。

xdg-shell clientへ定期的にpingを送り、期限内にpongを返さない場合は一度警告すること。
遅延だけでclientを強制切断せず、遅いpongを受理して監視を再開すること。clientの停止が
compositorや他clientのevent loopを停止させてはならない。

---

## FR-WIN-008

Windowはfocus可能でなければならない。

---

## FR-WIN-009

Fullscreenをサポートしなければならない。

---

## FR-WIN-010

必要に応じてWindowをfloatingへ変更可能でなければならない。

---

# 8. Tiling要件

## FR-TILE-001

通常Windowは原則としてTiled状態で生成されること。

---

## FR-TILE-002

Tiled WindowはGridへsnapされること。

---

## FR-TILE-003

Tilingは「現在画面を分割する」ことに限定されず、World上のGrid領域を占有する仕組みとして動作すること。

---

## FR-TILE-004

新規Windowを配置する際、既存Windowを必ず縮小する必要はない。

空いているWorld領域へ配置できること。

---

## FR-TILE-005

配置アルゴリズムは将来的に差し替え可能な設計であること。

---

# 9. Floating要件

## FR-FLOAT-001

任意のWindowをfloatingへ切り替えられなければならない。

---

## FR-FLOAT-002

Floating Windowも同じWorld上に存在しなければならない。

---

## FR-FLOAT-003

Floating Window専用workspace等を作らないこと。

---

## FR-FLOAT-004

Floating状態からTiled状態へ戻せなければならない。

---

## FR-FLOAT-005

Floating WindowはTiled Windowより上へ描画し、重なった領域のpointer hit-testでも優先すること。
このstacking順はCoreのeffective floating Propertyからadapterが導出し、floating専用workspace、
別Window集合、または重複したfloating状態を作らないこと。同じ層の中ではfocusされたWindowを
最前面とすること。
Floating WindowはWorld Gridを占有せず、Tiled Windowのmoveおよびresizeを妨げないこと。
Floating WindowはGrid cell未満の連続World位置を保持できること。FloatingからTiledへ
戻す場合は最寄りのGrid位置へ整列し、その位置が他のTiled Windowと重なる場合だけ変更を
拒否すること。

---

# 10. Camera要件

## FR-CAM-001

MioはWorldを表示するCamera概念を持たなければならない。

---

## FR-CAM-002

CameraはWorld上の位置を持たなければならない。

---

## FR-CAM-003

Cameraは上下左右へ移動できなければならない。

---

## FR-CAM-004

通常Camera移動では、viewport一個分を基本移動単位として扱うこと。

---

## FR-CAM-005

Cameraの停止地点はWindowの配置境界として扱ってはならない。

---

## FR-CAM-006

Camera移動時もWindowのWorld座標は変化してはならない。

---

## FR-CAM-007

Camera移動は滑らかにアニメーション可能でなければならない。

---

## FR-CAM-008

Cameraはzoom値を持てなければならない。

---

## FR-CAM-009

通常のWindow focusでは、現在のzoomで対象がCamera内に完全に収まっていればCamera位置を
維持すること。一部が見切れていれば全体が見えるために必要な最小距離だけ移動し、完全に
Camera外なら対象を中央へ表示すること。現在のzoomでも収まらないWindowは中央へ表示するが、
focusによってzoom値を暗黙に変更してはならない。Window resizeはCameraを移動してはならない。

---

## FR-CAM-010

KeybindからCameraの絶対zoom値を選択可能にすること。初期設定では`Super+1`から`Super+9`を
`0.1`から`0.9`へ対応させ、`Super+0`を通常Cameraの最大倍率`1.0`へ対応させること。
各bindのzoom値はKDLから変更可能とし、Mouseと同じ共有`CameraZoom` Actionを使用すること。

---

# 11. Overview要件

## FR-OV-001

Overviewは専用の別画面としてではなく、Camera zoomを変更することで実現すること。

---

## FR-OV-002

Overview状態でもWorld座標系は維持されること。

---

## FR-OV-003

Overview状態から任意のWindowを選択できること。

---

## FR-OV-004

OverviewからWindowを選択した際、Cameraが対象Windowへ移動し、通常倍率へ戻れること。

---

## FR-OV-005

OverviewによってWindowそのものの配置が変更されてはならない。

---

# 12. Focus要件

## FR-FOCUS-001

Directional Focusを提供すること。

最低限：

- left
- right
- up
- down

---

## FR-FOCUS-002

Directional FocusはWorld上のWindow位置関係から対象を決定すること。

---

## FR-FOCUS-003

Focus対象が現在Camera外に存在する場合、Cameraを対象へ移動できること。

---

## FR-FOCUS-004

Focus変更とCamera移動は論理的には分離可能であること。

---

# 13. Input要件

## FR-IN-001

KeyboardによるWindow操作をサポートすること。

---

## FR-IN-002

KeyboardによるCamera移動をサポートすること。

---

## FR-IN-003

Keybindは設定ファイルから変更可能でなければならない。

---

## FR-IN-004

Pointer操作をサポートすること。

---

## FR-IN-005

Windowのmouse move / resizeを提供すること。

---

## FR-IN-006

Cameraをmouseから操作できることを目標とする。

Camera pan/zoom、Window move/resize、floating切替、Output端からの次回配置方向、Window closeの
button割当、およびWindowの初期size復帰は、keyboard bindingと同様にKDLから変更可能であること。2 button以上のchordは
押下順を宣言すること。Window closeはmove gestureと同じbutton順で、先頭buttonを保持した
2番目buttonを設定回数clickするgestureとして扱い、不正な組合せは具体的なconfig errorにすること。
closeに必要なclick回数はKDLの`clicks` propertyで1から5の範囲を明示し、宣言時に省略を
許さないこと。
初期size復帰は設定buttonをKDLで明示した回数clickして対象へ共有`ResizeWindow`、`FocusWindow`、
`CameraFollow`を合成すること。単clickは短い判定時間後にclientへ完全なpress/releaseとして
再送し、設定回数成立時はclick列を消費すること。`clicks`は1から5の範囲で宣言時に必須とする。
安全な遅延判定のため、このbuttonは
Camera pan buttonと同一であること。

候補操作：

- RMB drag

RMB pressからreleaseまでのpointer移動量を連続World座標へ変換し、画面上でWorldが
pointerへ追従する向きへCameraをpanする。pressとreleaseはcompositorが消費し、clientへ
片側だけのbutton eventを送らないこと。ただし移動量がdrag閾値に達しなければ、通常の
right clickとしてpressとreleaseをclientへ転送する。上下左右のOutput端にはKDLから任意の
外部commandを割り当てられ、割当済みの端での短いright clickはcommand実行として消費する。

RMBを先に押した状態でLMBを押してdragした場合は、Cameraではなくpress位置のWindowを
移動させること。drag中のpixel位置はadapterの一時的な表示状態とし、release時に
共有ActionでCoreのWorld位置へ確定すること。TiledとFloatingの一時表示はpointerへ連続追従し、
release時にTiled Windowだけ最寄りのGrid位置へ整列すること。移動先が
占有済みなら元位置へ戻す。Floating Windowは連続World位置を保持し、重なりを許可する。
左右両方のpress/releaseを
clientへ転送しないこと。drag開始時とdrag中は元のfocusとCamera位置を維持し、対象Windowは
一時的に前面表示すること。移動成功時だけrelease後に対象へfocusを移し、失敗時は元のfocusを
維持すること。Window dragによってCameraを自動追従させないこと。
Window表示矩形の内側8 logical pixelsをresize handleとし、辺・角のhoverでは方向に対応した
resize cursorを表示すること。LMB dragが最寄りのGrid境界を越えるたび、原子的なresizeを適用し
clientへ新しいsizeを通知すること。応答前のbufferを縦横別々の倍率で変形してはならない。
Tiled Windowでは既存の隣接Window連鎖を利用し、無効な候補では最後の有効geometryを維持すること。
drag中はfocusを維持し、終了後は対象へfocusするがCameraは移動しないこと。
Window上で設定された`reset-window` buttonを複数clickした場合は、共有Actionによって
`placement.initial-size`の初期幅とその半幅を切り替えること。現在幅が初期幅なら半幅、
半幅未満なら半幅、それ以外なら初期幅とし、高さは初期高さへ戻すこと。同じActionを
`toggle-window-size` keybindからも実行可能にすること。
Window上でRMBを先に保持してMMBを押した場合は、対象の共有`ToggleFloating` Actionを予約する。
最初のrelease時に実行し、成功後は対象へfocusするが
Cameraは維持し、失敗時はfocusも変更しないこと。このchordのRMB/MMB pressと両方のreleaseは
clientへ転送しない。Output端でのMMB単独clickは、その端を共有`SetNextPlacement` Actionとして
予約すること。cornerでは最寄りの端を使用し、通常領域のMMBはclientへ転送すること。
Window上でRMBを保持してLMBを設定回数clickした場合は、対象へ共有`CloseWindow` Actionを
適用すること。同じbuttonはRMB+LMB Window moveにも使い、pointerが移動閾値を越えた場合は
drag、同一Window上で各click間の時間・距離条件を満たし設定回数に達した場合はcloseと判定する。
中央配置とcloseのclick回数はKDLでそれぞれ1から5の範囲を明示する。同じbutton列で同じ回数を
指定した場合だけ、同一gestureの衝突として設定errorにする。回数の大小には意味を持たせない。
静止したclick列がその回数と一致した場合は判定期限後またはRMB release時に`FocusWindow`と
`CameraCenter`を合成し、Window sizeを変更せずCamera中央へ配置すること。
成立後は両buttonがreleaseされるまでeventを消費し、FocusとCameraは変更しない。
commandはprogramと引数を分離して実行し、暗黙にshellを介さない。直接操作中は通常のCamera
補間を挟まない。

RMBを保持したpointer wheelはCamera zoomとして消費すること。下方向でzoom out、上方向で
zoom inし、初期値`1.0`を最も近い上限とする。RMBを保持していないwheelはclientへ転送すること。
ただし物理wheelをOutputの左端または右端で回した場合は横方向の共有`Focus`として消費し、
上回転を`Focus(Left)`、下回転を`Focus(Right)`とする。上端または下端では縦方向として、
上回転を`Focus(Up)`、下回転を`Focus(Down)`とする。cornerではpointerに最も近い端を使用する。
trackpad由来のscrollはこの操作に使わずclientへ転送すること。

Layer-shellのexclusive zoneはadapterの描画可能領域として反映し、通常およびmaximized
Windowはその領域を使用すること。Fullscreen WindowはOutput全体を使用すること。この差で
Camera viewportまたはWindowのWorld GridRectを変更しないこと。
描画したLayerSurfaceとそのpopupへframe callbackを返し、configure後の再描画を停止
させないこと。
TopまたはOverlay layerがexclusive keyboard interactivityを要求した場合は、その
LayerSurfaceへkeyboard focusを渡すこと。破棄時は直前からCoreで選択されているWindowへ
focusを戻すこと。TopまたはOverlay layerがon-demand keyboard interactivityへ遷移
した場合も、その遷移時に一度だけfocusを渡すこと。以降は通常のpointer focus規則に従うこと。
表示状態の切り替えによりkeyboard interactivityがnoneへ戻った場合も、Coreで選択中の
Windowへfocusを戻すこと。

Fullscreen中はWaybar等のTop layerを一時的に非表示とし、fullscreen解除時に同じ
LayerSurfaceを復元すること。緊急通知等に使用するOverlay layerは表示を維持すること。
Fullscreen Windowは角丸を適用せず、fullscreen中の共有Camera移動およびzoom Actionは
拒否すること。Camera状態は変更せず、fullscreen解除後に同じ位置とzoomを継続すること。

---

## FR-IN-007

MouseによるCamera操作でも、Keyboard操作と同じCamera modelを利用すること。

---

## FR-IN-008

通常clipboard、primary selection、clipboard manager向けdata-controlを提供し、同じSeatの
keyboard focusへ同期すること。

---

## FR-IN-009

`zwp_idle_inhibit_manager_v1`を提供し、surfaceごとの複数inhibitorを生成・解除の対として
追跡すること。idle timeoutまたはDPMSを実装する際は、有効なinhibitorが存在する間は
idle actionを実行しないこと。

---

## FR-IN-010

動画player等がbufferのcropおよびscaleに使用する`wp_viewporter`を提供すること。
Viewport指定をWindowのWorld GridRectまたはCamera zoomとして扱わないこと。

---

## FR-IN-011

touch capabilityをWayland Seatへ公開し、down / motion / up / frame / cancelを同じsurface
hit-testへ配送すること。touch downによるWindow focusはPointerと同じFocusおよびCamera
Actionの経路を使用すること。session lock中はlock surface以外へtouchを配送しないこと。

---

## FR-IN-012

`zwp_keyboard_shortcuts_inhibit_manager_v1`を提供すること。active inhibitorを持つsurfaceが
keyboard focus中の場合だけMioの通常keybindを抑止し、key eventをclientへ配送すること。
session lockおよびexclusive layer-shellの入力規則はshortcut inhibitorより優先すること。
別surfaceのinhibitorがfocused Windowの操作を抑止してはならない。

---

## FR-IN-011

`wp_fractional_scale_manager_v1`を提供し、surfaceへOutput由来のpreferred scaleを通知する
こと。Fractional output scaleとCamera zoomおよび整数World Gridを混同しないこと。

---

## FR-IN-012

各backendでは`ext-image-copy-capture-v1`と
`ext-output-image-capture-source-v1`によるOutput全体のSHM captureを提供すること。
`zwlr_screencopy_manager_v1`はgrim等のlegacy clientとの互換用としてOutput全体または
指定領域のcaptureを提供してよい。capture対象はCameraが最終的にOutputへ描画した内容とし、
Worldを別layoutへ変換しないこと。client bufferはformat、寸法、stride、範囲を検証し、
失敗時はprotocolのfailedまたはerrorを返すこと。

初期実装はcapture領域全体をdamageとして通知してよい。
sandbox化されたapplicationの画面取得はportalの選択経路を使用すること。通常Wayland
socketへ直接接続できる非sandbox clientは同じdesktop sessionの信頼領域として扱う。
session lock中のcapture要求は拒否すること。

---

## FR-IN-013

`xdg_activation_v1`を提供すること。client作成tokenはMioのSeatと有効な入力serialを
持つ場合だけ受理し、10秒以上経過したtoken、serialなしtoken、管理外surfaceへの要求は
拒否すること。成功したactivationは既存のFocusおよびCamera Action経路を使用し、
tokenを再利用させないこと。

---

# 14. Animation要件

## FR-ANI-001

Camera移動をアニメーション可能にすること。

---

## FR-ANI-002

Window move / resizeをアニメーション可能にすること。

---

## FR-ANI-003

Camera zoomをアニメーション可能にすること。

Windowの吸着、Camera移動、Camera zoomの描画補間は静止状態から緩やかに加速し、
終点付近で減速すること。操作中に目標が更新された場合は現在速度を引き継ぎ、動きを
不連続に開始し直さないこと。

---

## FR-ANI-004

Animation速度を設定可能にすること。

---

## FR-ANI-005

Animationを完全または部分的に無効化可能にすること。

---

## FR-ANI-006

Animationは派手さより、連続性と位置関係の理解を支援することを優先する。

---

# 15. Appearance要件

## FR-APP-001

Focused Windowの視覚的な目印を設定可能にすること。

---

## FR-APP-002

Focus indicatorのwidthとheightを設定可能にすること。

---

## FR-APP-003

Focus indicatorのcolorを設定可能にすること。

---

## FR-APP-004

Focus indicatorはfocused Windowだけに表示すること。

Focus indicatorはWindow下辺のごく薄い水膜と、その中央に重なる細い水光とする。
水膜は下辺全体を静かに結び、水光は中央の芯から横方向へ淡く減衰する。
Window境界の内外にも弱く広がるが、focused Windowの識別を妨げるほど強くしない。
Focusが切り替わったときだけ中央から左右へ短く展開し、静止後は同じ水光へ収束する。
旧focused Windowの水光は逆向きに中央へ縮みながら消えること。
この遷移はrendering stateであり、CoreのFocusやWindow geometryをframeごとに変更しない。
この選択はFocusを示す装飾だけに関するものであり、Mio全体のVisual effectを制限しない。

---

## FR-APP-005

Window corner radiusを設定可能にすること。

---

## FR-APP-006

Window opacityを設定可能にすること。

---

## FR-APP-007

通常Windowには任意の細い内側枠線を描画できること。幅と色を設定可能とし、幅0または
透明色で無効化できること。枠線はWindowの表示矩形とcorner radiusに追従するrendering
要素であり、CoreのWindow geometry、Grid、入力判定を変更しないこと。

---

## FR-APP-007

Window単位でbackdrop Blurを設定可能にすること。

Blurは同じProperty precedenceに従い、無効時は通常描画へ戻ること。Blurを視認するための
opacityは独立したPropertyとし、Blur有効化が暗黙にWindow opacityを変更してはならない。

---

## FR-APP-008

Shadowを設定可能にすることを目標とする。

---

## FR-APP-009

Visual effectは無効化可能でなければならない。

---

## FR-APP-010

Windowやlayer-shell背景が存在しない領域へ描画するWorld背景色を設定可能にすること。
これはadapterの描画設定であり、WorldやCameraの意味論を変更しないこと。

---

## FR-APP-011

通常Windowの表示gapはCamera zoomと同じ比率で縮尺すること。遠景でもWindow、gap、World上の
占有範囲の対応を保ち、見た目では離れているのに配置判定では重なる状態を作らないこと。
この縮尺はclient configure sizeを変更せず、popupおよびsubsurfaceを含むsurface tree全体で
同じpresentation scaleを維持すること。

---

# 16. Window Rule要件

## FR-RULE-001

Window Ruleを定義可能でなければならない。

---

## FR-RULE-002

最低限app-idを条件としてWindowを指定可能でなければならない。

---

## FR-RULE-003

Window titleによる指定も可能にすることを目標とする。

---

## FR-RULE-004

Window Ruleから以下のPropertyを設定可能にすること。

最低限：

- floating
- opacity

将来候補：

- blur
- focus indicator
- corner-radius
- shadow
- initial size
- initial placement
- focus-on-spawn
- animation

---

## FR-RULE-005

Appearance用RuleとFloating用Rule等を別々の仕組みにせず、一つのWindow Rule systemとして提供すること。

---

## FR-RULE-006

複数のWindow Ruleが一致した場合は設定ファイルの記述順にPropertyを合成し、同じPropertyは
後に記述したRuleを優先すること。設定再読み込みまたはclient metadata変更時はConfig Rule層を
再計算し、Runtime Override層は保持すること。

---

# 17. Runtime Window Property要件

この機能はMioの主要要件の一つとする。

## FR-PROP-001

実行中の任意Windowに対しProperty Overrideを設定できなければならない。

---

## FR-PROP-002

最低限focused Windowに対してRuntime Overrideを設定可能でなければならない。

---

## FR-PROP-003

最低限以下をRuntimeから変更可能にする。

- opacity
- floating

将来的に：

- blur
- focus indicator
- corner radius
- shadow
- animation

---

## FR-PROP-004

Runtime OverrideはWindow単位で適用されなければならない。

同じapp-idの別Windowへ自動的に適用してはならない。

---

## FR-PROP-005

Runtime Overrideを解除できなければならない。

---

## FR-PROP-006

Propertyの実効値は原則として以下の優先順位に従う。

```text
Default
↓
Config Window Rule
↓
Runtime Override
```

---

## FR-PROP-007

Runtime Override解除後は、対応するConfig RuleまたはDefault値へ戻ること。

---

## FR-PROP-008

Runtime Property変更のために設定ファイルそのものを書き換える必要がないこと。

---

# 18. Configuration要件

## FR-CONF-001

Mioは外部設定ファイルを持たなければならない。

---

## FR-CONF-002

通常設定形式としてKDLを第一候補とする。

---

## FR-CONF-003

設定ファイルから最低限以下を変更可能にする。

- keybind
- focus indicator
- focus indicator color
- focus indicator width / height
- corner radius
- opacity
- animation speed
- Window Rule

---

## FR-CONF-004

将来的に以下も設定可能にする。

- blur
- shadow
- Camera behavior
- Overview zoom
- placement strategy
- output settings
- mouse camera behavior

---

## FR-CONF-005

内部実装の細かい値を不用意に大量公開しないこと。

設定項目は人間が理解できる抽象度を保つこと。

---

## FR-CONF-006

設定エラーは可能な限り明確に報告すること。

通常起動時に設定の読み込みへ失敗しても、組み込み既定値でcompositorを起動し、元の設定
パスと原因をERRORとして報告すること。元のパスを保持し、修正後のreloadで復帰可能にする。
設定検証専用の`--check-config`はfallbackせず、無効な設定を失敗終了として報告すること。
`mio-compositor --help`はbackendを初期化せず正常終了し、利用可能な起動optionを表示すること。
`mio-compositor --version`はbackendを初期化せず、診断に使えるversionを表示すること。
設定エラー中は通常contentより上へ復旧方法を示す警告を描画し、reload成功時に消すこと。
session lock中は設定内容やエラーの有無をlock surfaceより上へ表示してはならない。
警告には可能な範囲で該当行番号、列番号、設定項目または入力行の概要を含めること。

---

## FR-CONF-007

設定ファイルを実行中に再読み込みできること。新しい設定全体の検証に成功してから置き換え、
失敗時は直前の有効な設定を維持すること。成功時は既存WindowのConfig Rule層を再計算するが、
Window単位のRuntime Overrideと起動済みprocessは維持し、`spawn-at-startup`を再実行しないこと。

---

## FR-CONF-008

KDLから起動時commandを複数指定可能にすること。各commandはprogramとargvを明示し、
shell文字列として解釈しないこと。WaylandおよびIPC endpointの準備後に一度だけ実行し、
設定reloadでは再実行しないこと。spawn失敗はcommandを特定できるwarningとして報告し、
compositorの動作を継続すること。

---

# 19. Yaldra要件

Yaldra対応は初期リリース必須ではない。

## FR-YAL-001

将来的にYaldraをprogrammable configuration languageとして利用可能にする。

---

## FR-YAL-002

KDLを使用しなくてもYaldraだけで高度な設定ができる構造を目標とする。

---

## FR-YAL-003

Yaldraを使用しなくてもMioは完全に利用可能でなければならない。

---

## FR-YAL-004

YaldraからSmithay低レイヤーを直接操作させない。

---

## FR-YAL-005

YaldraにはMioの高水準primitiveを公開する。

例：

- Window
- Camera
- Focus
- Selection
- Action
- Property

---

## FR-YAL-006

Yaldraから以下のような高度な振る舞いを記述可能にすることを目標とする。

- custom focus algorithm
- custom placement
- camera behavior
- composed actions
- macros
- conditional window behavior

---

# 20. X11互換要件

## FR-X11-001

MioはX11 application compatibilityを提供することを目標とする。

---

## FR-X11-002

初期候補としてxwayland-satelliteを利用する。

---

## FR-X11-003

X11 WindowであることをMio CoreのWorld modelへ極力露出しない。

---

## FR-X11-004

XWayland互換機能が利用できない場合でも、Wayland compositorとして起動可能であることが望ましい。

初期統合では`xwayland-satellite`を明示的に有効化し、専用X display番号を指定可能にすること。
satelliteへMioの`WAYLAND_DISPLAY`を渡し、Mioから起動するapplicationへ同じ`DISPLAY`を
継承させること。実行ファイルが存在しない場合またはsatelliteが終了した場合でも、Mioの
native Wayland機能は継続すること。Mio終了時は起動したsatelliteを停止すること。
satelliteを有効にしていない場合、Mioが`--command`で起動するapplicationからhost
compositorの`DISPLAY`を除外し、意図せずhost側のX serverへ接続させないこと。

---

# 21. Wayland Protocol要件

初期MVPではすべてを実装する必要はない。

優先度の高いもの：

- xdg-shell
- seat
- keyboard
- pointer

次段階：

- layer-shell
- popup handling
- xdg-decoration negotiation
- cursor-shape
- relative pointer / pointer constraints
- clipboard / data device
- fullscreen
- presentation-time
- input method / IME
- session lock
- screencopy
- fractional scaling
- linux-dmabuf
- single-pixel-buffer
- alpha-modifier
- xdg-toplevel-icon
- content-type
- xdg-foreign
- multi-output

`ext-session-lock-v1`によるlock中は通常Windowおよび通常LayerSurfaceを描画せず、入力を
lock surfaceだけへ配送すること。lock成功は遮蔽フレームのsubmit後に通知すること。
lockerが異常終了した場合は通常画面を露出せず、明示的なunlock時だけ元の画面とfocusを
復元すること。

`xdg-decoration`ではServerSideを明示し、clientへclient-side title barを描かせないこと。
Mioのserver-side decorationは細いWindow枠線とfocus indicatorを基本とし、title barや
minimize/maximize/close buttonを追加しない。装飾のためにCoreのWindow geometryや
入力Actionとは別のWindow管理概念を導入しないこと。

`wp_presentation`のfeedbackは描画開始時ではなく、対応frameのsubmit成功後に送ること。
時刻はglobalが通知するclockと一致させ、refresh周期と単調増加するsequenceを含めること。

ネストしたwinit backendでは`wp_cursor_shape_manager_v1`によるnamed cursor要求をhost
windowのcursorへ反映すること。Hidden要求ではcursorを隠すこと。従来型cursor surfaceは
指定hotspotを反映して全通常contentより上へ描画し、host cursorとの二重表示を避けること。
lock開始時には以前のclientが設定したcursor surfaceを引き継がないこと。

`zwp_relative_pointer_manager_v1`と`zwp_pointer_constraints_v1`は同じSeatのpointer入力を
利用すること。lock中は絶対座標とfocusを固定したまま相対移動を対象surfaceへ配送し、
confine中は対象surfaceまたは指定regionの外へpointerを移動させないこと。これらは
Wayland adapterの入力制約であり、World上のWindow座標やCamera座標へ状態を追加しない。

`zwp_linux_dmabuf_v1`で通知するformatとmodifierは、実際に使用するrendererがimport可能な
集合から導出すること。buffer作成の成功はrendererによるimport成功後にのみ通知し、失敗した
client bufferによってcompositorを終了しないこと。render nodeを取得できる場合はv4 feedbackを
優先し、取得できない場合だけrenderer formatを使ったv3へfallbackすること。

`wp_single_pixel_buffer_manager_v1`で作られたbufferは通常のsurface bufferとして扱い、
単色面のためにMio独自のWindow種別や描画状態を導入しないこと。

`wp_alpha_modifier_v1`によるsurface alphaはclient所有のsurface状態として扱うこと。
MioのWindow Propertyであるopacityを書き換えず、最終描画時に両者を合成すること。

`xdg_toplevel_icon_manager_v1`によるiconはWayland toplevelのmetadataとして保持し、Mio Coreへ
重複したicon状態を追加しないこと。buffer形式や寸法はprotocol実装で検証し、不正なiconで
compositorを終了しないこと。

`wp_content_type_manager_v1`によるsurface用途はWayland adapterのmetadataとして保持し、
未対応の最適化を行ったように見せかけないこと。将来Output policyが参照してもWindowの
World座標、Camera、Propertyを変更しないこと。

`zxdg_exporter_v2`と`zxdg_importer_v2`による別client間のtoplevel parent指定を受理すること。
handleの破棄時には関係を失効させ、Mio CoreへWindow所有関係やcontainerを追加しないこと。

Wayland data-deviceのdrag iconが指定された場合はpointerへ追従する一時surfaceとして通常
contentより上へ描画すること。drag終了、surface消失、session lock開始時には必ず破棄し、
World Windowやfloating Windowとして登録しないこと。

必要性の低いprotocolを初期から網羅しないこと。

`xdg_toplevel.wm_capabilities`では、compositorが実際に処理する操作だけを広告すること。
現段階ではfullscreenとmaximizeを広告し、未実装のminimizeとwindow menuは広告しない。

`xdg_toplevel.configure_bounds`を利用できるclientには、対象Windowのpresentationに対応する
Output利用可能領域を通知すること。これはclientの寸法決定用metadataであり、World Rectや
Camera viewportの新たなsource of truthにしないこと。

`xdg-dialog-v1`を提供し、modal指定されたtoplevelは共有`floating` Propertyのruntime override
を通して自動的にfloatingにすること。加えて、両軸で非zeroのmin/max sizeが一致する固定サイズ
toplevelも同じ経路で自動的にfloatingにすること。制約はsurface commit時に再評価すること。
自動floating条件が全て解除された場合はoverrideをclearし、ConfigまたはDefault値へ戻すこと。
dialog用workspaceや別座標系を作ってはならない。
従来の`xdg_toplevel.set_parent`だけを使うclientについても、parentを持つ間はtransient
dialogとして同じfloating経路を使うこと。modalまたはparentのどちらかが残る間はfloatingを
維持し、両方が解除された場合だけoverrideをclearすること。
app-idやtitleだけからdialogを推測してはならない。同じapplicationが通常の複数Windowや起動画面を
生成する場合にも、それらをdialogと誤判定しないこと。
自動floating Windowに復帰先Windowがある場合、そのWorld Rectを配置基準として重ね、focused
Windowなら通常のCamera中央化Actionを再適用すること。固定サイズsurfaceはその表示Rect内で
中央に置き、通常Cameraではclient原寸、OverviewではCamera zoomに従って内容ごと縮小すること。
focus indicatorは論理Grid Rectではなく実際の表示geometryを基準に描画すること。
自動floatingが初めて確定した配置では、そのWindowの描画補間を親位置から開始し、一時的な
Tiled配置から移動するanimationを表示しないこと。Cameraも訂正する場合は全Windowの画面座標
補間を訂正後のCameraから開始すること。以後の通常move animationは維持すること。
新規xdg-toplevelはrole生成だけではSpaceへ表示・focusせず、最初のroot surface commitで
dialog判定と自動配置を処理した後に初めて表示・focusすること。commit前の仮Tiled配置を一瞬
描画してはならない。
既存の復帰先Windowがあり、最初のcommitでparent、modal、固定サイズのいずれにも分類できない
場合は、activated configureとSeat focusだけを適用して表示およびCamera移動を次のcommitまで
保留すること。遅れて届く固定サイズ制約のためにCameraを仮Tiled位置へ往復させてはならない。

---

# 22. External Shell要件

## FR-SHELL-001

Mioは独自bar / launcherを必須としない。

---

## FR-SHELL-002

標 / Shirubeを利用可能にする。

---

## FR-SHELL-003

要 / 外部アプリケーションからMioの状態取得とAction実行を可能にする。

Shirubeは状態表示barとして扱い、Window一覧またはWindow switcherを要求しないこと。
Window一覧からの選択はKaname、wofi、自作UIなどの外部アプリケーションがIPC snapshotと
Actionを合成して提供すること。Kaname連携はこの汎用的な外部アプリ連携の一例として扱うこと。
Mio本体へlauncher UIまたは特定アプリ固有のWindow modelを追加してはならない。
Window一覧の各要素は同一snapshot時点のfocus状態を含み、外部アプリケーションが別の照会結果を
突き合わせなくても現在の選択を判別できること。

---

## FR-SHELL-004

Waybar等の一般的な外部shellとの併用を妨げないこと。

---

## FR-SHELL-005

Mio単体でもcompositorとして成立すること。

---

# 23. IPC要件

## FR-IPC-001

外部shellおよびCLI向けIPCを提供すること。

---

## FR-IPC-002

IPCから最低限以下の状態を取得可能にすること。

- Window一覧
- focused Window
- app-id
- title
- World position
- Window size
- Camera position
- Camera zoom
- Output information

---

## FR-IPC-003

IPCから以下のActionを実行可能にすること。

- focus Window
- Camera移動
- CameraをWindowへ移動
- Window move
- Window resize
- floating toggle
- Runtime Property変更
- close Window

初期IPCは`$XDG_RUNTIME_DIR`内のMio instance固有Unix socketを使用し、socket pathを
`MIO_SOCKET`としてMioがspawnするapplicationへ公開すること。要求と応答は上限付きの
一要求一接続とし、最初の改行またはEOFで要求を確定すること。接続したまま要求を完結
しないclientがcompositorを長時間停止させないよう、短いread timeoutを設けること。
応答にも短いwrite timeoutを設け、一回のevent-loop dispatchで処理する接続数を制限して、
接続集中時にもWayland入力と描画へ制御を返すこと。
socketは所有userだけが接続できるpermissionにすること。不正なUTF-8、上限超過、未知
command、不正なWindow IDまたはProperty値はcompositorを停止させず、明示的なerror
responseを返すこと。
`mioctl`はMioへ接続できない環境でも`--help`を表示し、commandと引数形式を確認可能にすること。
`mioctl --version`もMioへ接続せずversionを表示すること。
transport errorおよびIPCの`ok: false`応答では非ゼロ終了し、成功応答だけを成功終了とすること。
`quit`はWindowやWorldへのActionではなくcompositor lifecycle操作としてevent loopを停止し、
signal終了と同じ通常のcleanup経路を通ること。

初期のread APIは`windows`、`focused-window`、`camera`、`outputs`を提供すること。外部shell向けに、
それらを同じ時点のsnapshotとして返す`state`も提供すること。最初のAction APIは
`set-opacity`、`clear-opacity`、`camera-to`を提供し、すべて既存Core Actionへ変換すること。

続くAction APIとして`focus`、`camera-step`、`move-window`、`resize-window`、
`toggle-floating`、`close`、`set-property`、`clear-property`を提供すること。directionは
left / right / up / down、Propertyは共通Property systemのopacity / floating / blurを対象とする。
focus変更はWayland Seat activationへ、close要求は対象xdg-toplevelへ同期すること。

---

# 24. Multi-monitor要件

Multi-monitorは常用版までに必要。

ただしCamera modelの詳細は未確定。

## FR-MON-001

複数outputを認識できなければならない。

---

## FR-MON-002

異なるresolution / scaleを持つoutputを扱えることを目標とする。

---

## FR-MON-003

各Outputは独立したCameraを持つ。Camera Actionと新規Window配置は現在アクティブな
OutputのCameraを対象とする。WindowはOutputへ所属せず、すべて同じWorld上に存在する。

---

## FR-MON-004

Multi-monitor設計によってWorld modelをworkspace型へ変更してはならない。

---

## FR-MON-005

Outputを切り替えても、各Cameraの位置、viewport、zoomを独立して保持しなければならない。
Output切替はWindowのWorld座標、identity、lifetimeを変更してはならない。

---

## FR-MON-006

nested開発backendで複数の仮想Outputを指定した場合、各Cameraの内容を一つのhost
Window内の独立した領域へ同時描画すること。各領域の外へ描画が漏れてはならない。
Pointerで領域を選択した場合、そのOutputをCamera Actionの対象にすること。

---

## FR-MON-007

nested開発backendの各仮想Outputは個別の wl_output globalとして公開し、固有のname、
mode、論理位置を通知すること。Outputごとのlayer-shell配置、frame通知、screencopy
領域は、そのOutputのgeometryを基準にすること。

---

# 25. IME要件

## FR-IME-001

日本語入力を含むWayland Input Method利用を常用版までにサポートする。

---

## FR-IME-002

fcitx5等との正常な利用を目標とする。

---

## FR-IME-003

IME popup / candidate windowの位置がCamera transformによって破綻しないこと。

`zwp_text_input_manager_v3`と`zwp_input_method_manager_v2`を同じSeatへ接続し、
focused surfaceのtext-input状態、IMEによるpreedit / commit、およびkeyboard grabを中継すること。
IMEによるkey event注入用に`zwp_virtual_keyboard_manager_v1`を提供すること。
IME popupは既存のpopup treeへ所属させ、別のWorld Windowとして管理しないこと。

---

# 26. Performance要件

## NFR-PERF-001

通常操作で知覚可能な著しい入力遅延を発生させないこと。

---

## NFR-PERF-002

Camera animation中もPointer / Keyboard inputを処理できること。

---

## NFR-PERF-003

非表示領域のWindowを不必要にフル描画しないこと。

---

## NFR-PERF-004

Damage tracking等のSmithay機能を適切に利用すること。

---

## NFR-PERF-005

Visual effectを無効化することで低負荷状態へ切り替えられること。

---

# 27. 安定性要件

## NFR-STAB-001

単一Windowの異常終了によってMio全体が終了してはならない。
通常の接続終了はdebug、Mioまたはclientが検出したWayland protocol errorはwarningとして
client識別子と理由を記録し、application compatibility問題をcompositor停止と区別できること。
起動時に`--command`またはKDLの`spawn-at-startup`で指定したapplicationのspawnに失敗しても、理由とcommandを警告して
Wayland compositorとしての動作を継続すること。
`SIGINT`と`SIGTERM`はevent loop上で受け取り、Xwayland satelliteの回収とIPC socketの
削除を行う通常の終了経路へ流すこと。IPCの`quit`も同じ終了経路へ流すこと。

---

## NFR-STAB-002

設定エラーによって可能な限りcompositor全体をクラッシュさせないこと。

---

## NFR-STAB-003

Yaldra将来対応時、Yaldra script errorがMio Coreを破壊しない構造にする。

---

## NFR-STAB-004

Smithay API使用時は対象revisionに基づく実装を行う。

---

## NFR-STAB-005

Runtime stateのsource of truthを明確にし、同じ状態を複数箇所で不整合に保持しないこと。

---

# 28. Maintainability要件

## NFR-MAIN-001

Mio CoreとSmithay integrationを分離すること。

---

## NFR-MAIN-002

Mio Coreは可能な限りunit test可能であること。

---

## NFR-MAIN-003

Window placement / Focus / Camera movement等の純粋ロジックをWayland protocol handlerへ直接埋め込まないこと。

---

## NFR-MAIN-004

巨大な一枚State構造へすべての責務を集中させないこと。

---

## NFR-MAIN-005

新機能実装時は、既存primitiveで表現可能かを確認すること。

---

# 29. UX要件

## NFR-UX-001

操作原理が一貫していること。

---

## NFR-UX-002

同種の操作がcontextによって不必要に異なる意味を持たないこと。

---

## NFR-UX-003

Camera移動によってWorld上の位置関係を理解できること。

---

## NFR-UX-004

Overviewへ入っても「別の画面へ切り替わった」感覚ではなく、「同じWorldを遠くから見ている」感覚を維持すること。

---

## NFR-UX-005

Visual noveltyより日常操作の予測可能性を優先すること。

---

# 30. Visual Design要件

## NFR-VIS-001

Mioの水的モチーフは主にmovement / transitionで表現する。

Visual effectはMioを制作する主要な動機および製品個性の一つとして扱う。Window Managementの
正しさから独立させることは、Visual effectの重要度を下げることを意味しない。

---

## NFR-VIS-002

水を想起させる装飾を過度に追加しない。

---

## NFR-VIS-003

Wallpaperや特定のcolor paletteへ依存しない。

---

## NFR-VIS-004

Light / Dark双方のricingを妨げない。

---

## NFR-VIS-005

Visual effectはユーザーが無効化可能であること。

---

# 31. セキュリティ・権限要件

## NFR-SEC-001

外部IPCは必要以上の権限を公開しないこと。

---

## NFR-SEC-002

Yaldraからlow-level compositor internalsへ無制限アクセスさせないこと。

---

## NFR-SEC-003

設定ファイルの読み込みによって意図せず任意commandが実行されない構造をKDL側では維持する。

---

## NFR-SEC-004

任意command実行は明示的な`spawn`等のActionとして扱う。

---

# 32. MVP必須要件

MVPでは以下を必須とする。

### Compositor

- Smithayベースで起動
- Wayland Client接続
- xdg-toplevel表示
- Keyboard
- Pointer
- Focus

### World

- 2D World座標
- Grid
- Window Rect
- Camera
- Camera移動

### Window Management

- basic tiled placement
- move
- resize
- focus
- close

### Mio固有要件

- WindowがCamera viewport境界を跨げる
- Cameraを移動してもWindow位置が維持される
- Camera外WindowがWorld上に存在し続ける

この段階では、

- blur
- XWayland
- Overview
- Yaldra
- Shirube integration

は必須ではない。

---

# 33. Alpha要件

MVP後のAlphaでは以下を目標とする。

- floating
- fullscreen
- layer-shell
- popup
- keybind config
- KDL config
- focus indicator
- corner radius
- opacity
- animation
- Window Rule
- Runtime Window Property Override
- basic IPC

この段階から日常利用の試験を開始できる状態を目指す。

---

# 34. Beta要件

Betaでは常用可能性を重視する。

目標：

- Overview
- smooth Camera
- multi-output
- fractional scaling
- clipboard
- IME
- screencopy
- session lock
- xwayland-satellite
- stable IPC
- crash handling
- config reload

---

# 35. 1.0候補要件

1.0では以下を重視する。

- 長時間常用できる
- 一般的Wayland applicationが問題なく利用可能
- X11 applicationも実用的に利用可能
- Multi-monitorが安定
- IMEが安定
- Screen sharingが利用可能
- Configurationが安定
- Runtime Propertyが安定
- Public API / IPCが一定程度固定

Yaldra integrationは1.0必須ではない。

完成度を優先し、Yaldraのために1.0を遅らせない。

---

# 36. 将来要件

将来的に検討：

- Yaldra programmable config
- Custom Focus algorithm
- Custom Placement algorithm
- Custom Camera behavior
- User-defined action composition
- Subtle ripple effects
- Blur
- Advanced overview navigation
- Shirube deep integration
- Kaname deep integration
- Runtime rule persistence

---

# 37. 非要件

以下はMioの必須要件としない。

- 3D World
- Window depth placement
- Camera rotation
- 独自GUI toolkit
- 独自notification system
- 独自bar必須化
- 独自launcher必須化
- 全Wayland protocol完全対応
- X11 WMの完全再現
- 大量のbuilt-in layout
- 大量のvisual effects
- 特定wallpaperとの統合
- 特定theme generator必須化

---

# 38. 要件判断原則

新しい要求を追加する場合、以下を順に確認する。

1. Mioの基本World modelと矛盾しないか。
2. 新しい概念を増やさず実現できないか。
3. Window / Camera / Grid / Property / Actionの既存primitiveで表現できないか。
4. 日常利用上、本当に価値があるか。
5. KDLの設定項目として追加すべきか、それともYaldra等のprogrammable layerへ任せるべきか。
6. 外観上面白いだけの機能になっていないか。

---

# 39. 受け入れ条件：Mio固有コンセプト

最低限以下のデモが成功した場合、Mioのコアコンセプトが成立したと判断する。

## AC-001

Window AをCamera viewport境界に跨いで配置できる。

---

## AC-002

Cameraを隣接位置へ移動すると、Window Aの表示部分が連続的に変化する。

---

## AC-003

Camera移動前後でWindow AのWorld Rectが変化しない。

---

## AC-004

Camera外へ完全に出たWindowが破棄されず、再びCameraを戻すと同じ位置に表示される。

---

## AC-005

複数WindowをGrid上へ配置できる。

---

## AC-006

Grid単位でWindow resizeできる。

---

## AC-007

Directional FocusでCamera外Windowを選択できる。

---

## AC-008

選択したCamera外WindowへCameraを移動できる。

---

# 40. 受け入れ条件：Runtime Property

## AC-PROP-001

Focused Windowのみopacityを変更できる。

---

## AC-PROP-002

同じapp-idの別Windowへopacity変更が波及しない。

---

## AC-PROP-003

Runtime Overrideを解除すると、元のConfig RuleまたはDefault opacityへ戻る。

---

## AC-PROP-004

Runtime Property変更のためにKDLファイルを書き換える必要がない。

---

# 41. 受け入れ条件：Overview

Overview実装時には以下を満たす。

## AC-OV-001

Overview開始時にWindowのWorld座標が変化しない。

---

## AC-OV-002

OverviewはCamera zoomの変化として表現される。

---

## AC-OV-003

Overview中にWindow間の相対位置が維持される。

---

## AC-OV-004

Overview上で選択したWindowへCameraが移動できる。

---

## AC-OV-005

Overview終了後もWorld layoutが維持される。

---

# 42. 最終的な製品像

Mioは「機能の多いWM」を第一目的としない。

目標は、

**少ない基本原理を理解するだけで、多くの操作を自然に予測できるWM**

である。

ユーザーが理解すべき中心概念は、

```text
World
Grid
Window
Camera
```

である。

理想的には、

- workspaceを覚える必要がない
- Windowの場所を空間的に覚えられる
- Overviewは世界をそのまま遠くから見られる
- TilingとFloatingが同じWorld上に存在する
- 設定とRuntime変更が同じProperty modelで扱われる
- Keyboard / Mouse / IPC / Yaldraが同じAction modelを共有する

状態を目指す。

Mioの根本要件は次の一文に集約される。

> **世界は一つ。Windowはそこに置かれ、Displayはその世界を覗くCameraである。**

また、Mioの設計判断は次の原則に従う。

> **できることを増やすために、覚えるべき原理を増やさない。**
