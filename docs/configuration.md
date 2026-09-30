# Mio 設定リファレンス

Mioの設定はKDLで記述する。標準パスは`$XDG_CONFIG_HOME/mio/config.kdl`、
`XDG_CONFIG_HOME`がない場合は`$HOME/.config/mio/config.kdl`である。
`--config PATH`で別ファイルを指定できる。

変更前に構文を検査できる。

```sh
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

`bind`を1つでも書くと組み込みkeybind一式を置き換える。`edge-command`も同様に、
1つでも書くと組み込みのedge command一式を置き換える。それ以外の省略項目は組み込み値を使う。

## Top-level node

| Node | 内容 |
|---|---|
| `output { scale NUMBER }` | Wayland Outputのscale。`0.5..=4.0`、既定値`1.0` |
| `appearance { ... }` | Windowと背景の外観 |
| `effects { ... }` | blur、shadow、航跡、開閉transition |
| `animation { speed NUMBER }` | animation全体の速度。`0`で無効 |
| `camera { viewport W H }` | 通常倍率で画面に入るGrid数 |
| `placement { initial-size W H }` | 新規Windowの初期Grid size |
| `mouse { ... }` | Mouse全体設定（現在はcursor非表示時間） |
| `mouse-bind "GESTURE" "ACTION"` | Mouse gestureへActionを割り当てる |
| `include "PATH"` | 別のKDL設定ファイルをその位置へ読み込む |
| `spawn-at-startup "PROGRAM" "ARG"...` | 起動後に一度だけ実行するargv |
| `edge-command "EDGE" "PROGRAM" "ARG"...` | Output端の短い右clickで実行するargv |
| `bind "CHORD" "ACTION"` | keybind |
| `bind "CHORD" "spawn" "PROGRAM" "ARG"...` | keybindから外部commandを実行 |
| `window-rule { ... }` | Window Propertyの設定rule |

未知のnodeやoptionはerrorになる。

## 値の基本形式

- colorは`"#RRGGBB"`またはalpha付きの`"#RRGGBBAA"`
- size、幅、時間などは各表に記載した範囲の数値
- commandはshell文字列ではなく`"PROGRAM" "ARG"...`のargv
- pathは`include`を除いてshellの`~`展開や環境変数展開を行わない

設定を再読み込みしてerrorになった場合は、直前の有効な設定を維持して画面上にerrorを表示する。
起動時の設定が不正な場合は組み込み既定値で起動するため、設定を修正して`reload-config`を実行できる。

## Output

文字、UI、cursorなどの基準となるWayland Output scaleを指定する。Camera zoomとは別の設定で、
fractional scaleも利用できる。設定再読み込み時にも既存Outputへ反映される。

```kdl
output {
    scale 1.25
}
```

## Cursor theme and size

Cursor themeと論理sizeはKDLではなく、Mio起動時の`XCURSOR_THEME`と`XCURSOR_SIZE`を使用する。
たとえば`XCURSOR_SIZE=24`はOutput scaleにかかわらず24 logical pixelを指定する。Mioは
themeに要求sizeそのものの画像がない場合も、最も近い画像を要求sizeへ拡大縮小する。
環境変数は起動時に読み込むため、変更後はMio sessionを再起動する必要がある。

## Include

`include`は別のKDLファイルを、記述した位置へ展開する。相対pathは`include`を書いた
ファイルのディレクトリを基準に解決する。読み込み先でも`include`を使用できるが、循環参照は
errorになる。

```kdl
include "wallpaper.kdl"
```

指定先が存在しない場合だけはerrorにせず無視する。これは、壁紙選択ツールなどが後から生成する
任意設定を安全に読み込むための挙動である。存在するファイルが読めない場合やKDL・設定値が不正な
場合は通常どおりerrorを表示する。

例えば`wallpaper.kdl`を次のどちらかに更新すれば、次回のMio起動時に選択したbackendを起動できる。

```kdl
spawn-at-startup "mpvpaper" "*" "/path/to/wallpaper.mp4"
// または
spawn-at-startup "awww-daemon"
spawn-at-startup "awww" "img" "/path/to/wallpaper.png"
```

設定reloadでも読み込み先は再評価される。ただし`spawn-at-startup`はreload時には実行されないため、
壁紙commandの変更は次回のMio起動時に反映される。

## Appearance

| Option | 既定値 | 値・意味 |
|---|---:|---|
| `background-color` | `"#FFFFFF"` | `"#RRGGBB"`または`"#RRGGBBAA"` |
| `window-border-width` | `1` | logical pixel、`0..=4096`。`0`で無効 |
| `window-border-color` | `"#FFFFFF2E"` | border color |
| `focus-indicator-width` | `320` | 水光の横幅、logical pixel、`0..=4096` |
| `focus-indicator-height` | `2` | 水光の高さ、logical pixel、`0..=4096` |
| `focus-indicator-color` | `"#FFFFFF"` | 水光の色 |
| `corner-radius` | `0` | logical pixel、`0..=4096`。`0`で無効 |
| `gaps` | `24` | 各Window辺のinset、logical pixel、`0..=4096` |
| `opacity` | `1.0` | `0.0..=1.0`。全Windowの既定opacity |
| `opacity-toggle A B` | `1.0 0.8` | `toggle-opacity`で切り替える異なる2値 |

`opacity`はclient surface全体へ適用される。application内部の背景だけを透明にして文字や画像を
不透明に保つことはできない。その表現が必要な場合はapplication側で背景alphaを設定する。

## Effects

| Option | 既定値 | 値・意味 |
|---|---:|---|
| `blur-passes` | `3` | `1..=8`。多いほど滑らかだがGPU負荷が増える |
| `blur-offset` | `2.0` | `0.5..=20.0` |
| `shadow-radius` | `16.0` | `0.0..=256.0`。`0`で無効 |
| `shadow-offset X Y` | `0 0` | 各`-4096..=4096` logical pixel |
| `shadow-color` | `"#00000040"` | shadow color |
| `cursor-wake` | `true` | `true` / `false` |
| `cursor-wake-threshold` | `1600` | `1..=10000`。航跡を発生させる移動速度 |
| `cursor-wake-strength` | `0.032` | `0.0..=0.2`。背景屈折の強さ |
| `cursor-wake-width` | `8.5` | `1.0..=64.0` logical pixel |
| `cursor-wake-duration` | `1400` | `100..=10000` ms |
| `window-transition` | `"water"` | `"water"` / `"sci-fi"` / `"none"` |
| `window-transition-duration` | `420` | `100..=5000` ms |

## Animation、Camera、Placement

| Node | 既定値 | 内容 |
|---|---:|---|
| `animation { speed NUMBER }` | `1.0` | `0`でanimation無効。大きいほど早く収束する |
| `camera { viewport W H }` | `8 8` | 通常倍率でOutputに対応するWorldのGrid数 |
| `placement { initial-size W H }` | `8 8` | 新規Windowの初期Grid size |

`viewport`と`initial-size`の幅・高さは正の整数である。Camera zoomはこの論理viewportを変えず、
同じWorldを遠くから表示する。

## Mouse

`mouse-bind GESTURE ACTION`は、keybindと同じく入力から動作への向きで記述する。
button名は`left`、`right`、`middle`で、`right+left`はrightを保持してleftを押すordered chordである。

| Gesture | 用途 |
|---|---|
| `BUTTON-drag` | `camera-pan` |
| `BUTTON-wheel` | `camera-zoom` |
| `BUTTON+BUTTON-drag` | `move-window` |
| `window-edge+BUTTON-drag` | `resize-window` |
| `output-edge+BUTTON-click` | `place-next` |
| `BUTTON-click` / `BUTTON+BUTTON-click` | keybindと共通のAction。`clicks=1..=5` |

dragやwheelは継続的なadapter操作へ結び、clickは共有Actionへ結ぶ。click対象WindowへFocusを
移してからActionを記述順に実行する。単button clickは同じbuttonへ一つだけ割り当てられる。
省略したbindingやActionは暗黙に補われない。

```kdl
mouse-bind "right+left-click" clicks=1 {
    action "camera-center"
    action "camera-zoom" 1.0
}
```

`action`にはkeybindと同じAction名・引数を指定でき、`spawn`もargv形式で利用できる。

`mouse { cursor-hide-delay-ms N }`はpointer停止後にcursorを隠すまでの時間をmsで指定する。
既定値は`0`（自動非表示なし）。キー入力中はcursorを隠し、pointerを動かすと再表示する。

`spawn-at-startup`は最初のOutputが利用可能になると直ちに起動するが、外部の壁紙clientが
最初のbufferを描くまでの間は`background-color`が見える。起動時の色変化を目立たなくするには、
壁紙に近い色を`background-color`へ指定する。

## Startup commandとedge command

`spawn-at-startup`はMio起動ごとに一度だけ実行する。設定reloadでは再実行しない。Mioが作成した
`WAYLAND_DISPLAY`、`MIO_SOCKET`、`XDG_CURRENT_DESKTOP=mio`、`XDG_SESSION_DESKTOP=mio`、
`MIO_BACKEND`を継承する。Outputがまだない場合は、最初のOutputが利用可能になるまで起動を保留する。

```kdl
spawn-at-startup "waybar"
spawn-at-startup "fcitx5" "-d"
```

`edge-command`はOutput端の短い右clickに外部commandを割り当てる。edgeは`left`、`right`、`top`、
`bottom`である。一つでも宣言すると組み込みedge command一式を置き換える。既定値はbottom edgeの
`wofi --show drun`である。

```kdl
edge-command "bottom" "wofi" "--show" "drun"
```

commandでpipe、redirect、glob、環境変数展開等が必要な場合だけshellを明示する。

```kdl
spawn-at-startup "sh" "-c" "program-a | program-b"
```

## Keybind

chordは`Ctrl`、`Alt`、`Shift`、`Super`とkeyを`+`で連結する。同じchordを重複して
宣言できない。keyには1文字、またはxkbcommonのkeysym名を指定できる。

代表例：

- `Space`、`Enter`、`Tab`、`Escape`、`BackSpace`
- `Left`、`Right`、`Up`、`Down`、`Home`、`End`、`PageUp`、`PageDown`
- `Insert`、`Delete`、`PrintScreen`、`Menu`
- `F1`から`F35`
- `KP_0`、`KP_Add`などのkeypad key
- `XF86AudioRaiseVolume`、`XF86AudioMute`、`XF86MonBrightnessUp`などのXF86 key

`Esc`、`SpaceBar`、`PrtSc`、`PgUp`、`PgDn`等の一般的な別名も利用できる。文字keyの
判定は修飾後の記号ではなくkeymapのbase keysymを使うため、例えば`Shift+1`は`!`ではなく
`1`として記述する。

```kdl
bind "Super+Space" "spawn" "wofi" "--show" "drun"
bind "PrintScreen" "spawn" "grim"
```

方向Actionの末尾は`left`、`right`、`up`、`down`のいずれかに置き換える。

| Action | 内容 |
|---|---|
| `focus-DIRECTION` | 指定方向へFocusを移す |
| `camera-DIRECTION` | Cameraを1 viewport進める |
| `camera-nudge-DIRECTION` | CameraをGrid 1 cell進める |
| `move-DIRECTION` | focused WindowをGrid 1 cell移動する |
| `resize-DIRECTION` | focused WindowをGrid 1 cell resizeする |
| `place-next-DIRECTION` | 次に開くWindowの配置方向を1回指定する |
| `camera-zoom VALUE` | Cameraを絶対倍率`0.1..=1.0`へ変更する |
| `camera-center` | focused Windowが画面中央に来るようCameraを移動する |
| `close` | focused Windowを閉じる |
| `cycle-output` | active Outputを切り替える |
| `toggle-floating` | tiled / floatingを切り替える |
| `toggle-fullscreen` | fullscreenを切り替える |
| `toggle-maximized` | maximizedを切り替える |
| `toggle-window-size` | 初期幅と半幅を切り替える |
| `toggle-opacity` | `opacity-toggle`の2値を切り替える |
| `clear-opacity` | runtime opacity overrideを消す |
| `toggle-blur` | blurを切り替える |
| `toggle-cursor-wake` | カーソル航跡を切り替える |
| `toggle-overview` | Overview倍率を切り替える |
| `select-overview` | focused Windowを選択して通常倍率へ戻す |
| `reload-config` | 設定を再読み込みする |

`camera-zoom`だけは第3引数を取る。

```kdl
bind "Super+5" "camera-zoom" 0.5
```

外部commandは`spawn` Actionでargvを直接指定する。shell文字列として解釈しないため、pipeや
環境変数展開などが必要な場合だけ`sh -c`または`bash -c`を明示する。

```kdl
bind "Super+W" "spawn" "bash" "-c" "$HOME/.shellscript/select-wallpaper-kaname.sh"
```

## Window Rule

`app-id`と`title`は完全一致であり、少なくとも一方が必要である。Propertyの優先順位は
Default、matched Window Rule、runtime overrideの順である。複数Ruleが一致すると
ファイル順に合成され、同じPropertyは後のRuleが上書きする。
`*`や正規表現は解釈しない。全Windowへopacityを設定する場合は、全件一致Ruleではなく
`appearance.opacity`を使う。

```kdl
window-rule {
    match app-id="foot" title="terminal"
    opacity 0.9
    floating false
    blur true
}
```

利用できるPropertyは`opacity`、`floating`、`blur`である。runtime overrideはWindowごとに
保持され、設定ファイル自体を書き換えない。再適用、clear、寿命を含む詳細は
[Window RuleとPropertyリファレンス](window-properties.md)を参照する。
初回表示前にRuleで`floating=true`になった新規Windowは、起動直前にfocusされていた
Windowへ重ねて配置する。reloadや手動のfloating切替では既存Windowを移動しない。
