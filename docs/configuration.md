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
| `appearance { ... }` | Windowと背景の外観 |
| `effects { ... }` | blur、shadow、航跡、開閉transition |
| `animation { speed NUMBER }` | animation全体の速度。`0`で無効 |
| `camera { viewport W H }` | 通常倍率で画面に入るGrid数 |
| `placement { initial-size W H }` | 新規Windowの初期Grid size |
| `mouse { ... }` | Mouse gesture |
| `spawn-at-startup "PROGRAM" "ARG"...` | 起動後に一度だけ実行するargv |
| `edge-command "EDGE" "PROGRAM" "ARG"...` | Output端の短い右clickで実行するargv |
| `bind "CHORD" "ACTION"` | keybind |
| `window-rule { ... }` | Window Propertyの設定rule |

未知のnodeやoptionはerrorになる。

## Appearance

| Option | 値 |
|---|---|
| `background-color` | `"#RRGGBB"`または`"#RRGGBBAA"` |
| `window-border-width` | logical pixel、`0..=4096` |
| `window-border-color` | color |
| `focus-indicator-width` | logical pixel、`0..=4096` |
| `focus-indicator-height` | logical pixel、`0..=4096` |
| `focus-indicator-color` | color |
| `corner-radius` | logical pixel、`0..=4096`。`0`で無効 |
| `gaps` | logical pixel、`0..=4096` |
| `opacity` | `0.0..=1.0` |
| `opacity-toggle A B` | `toggle-opacity`で切り替える異なる2値 |

## Effects

| Option | 値 |
|---|---|
| `blur-passes` | `1..=8` |
| `blur-offset` | `0.5..=20.0` |
| `shadow-radius` | `0.0..=256.0`。`0`で無効 |
| `shadow-offset X Y` | 各`-4096..=4096` |
| `shadow-color` | color |
| `cursor-wake` | `true` / `false` |
| `cursor-wake-threshold` | `1..=10000` |
| `cursor-wake-strength` | `0.0..=0.2` |
| `cursor-wake-width` | `1.0..=64.0` |
| `cursor-wake-duration` | `100..=10000` ms |
| `window-transition` | `"water"` / `"sci-fi"` / `"none"` |
| `window-transition-duration` | `100..=5000` ms |

## Mouse

button名は`"left"`、`"right"`、`"middle"`を使う。2 buttonの項目は、前者を保持して
後者を押すordered chordである。

| Option | 形式 |
|---|---|
| `camera-pan` | `BUTTON` |
| `camera-zoom` | `BUTTON` |
| `move-window` | `BUTTON BUTTON` |
| `resize-window` | `BUTTON` |
| `reset-window` | `BUTTON clicks=N` |
| `toggle-floating` | `BUTTON BUTTON` |
| `place-next` | `BUTTON` |
| `center-window` | `BUTTON BUTTON clicks=N` |
| `close-window` | `BUTTON BUTTON clicks=N` |

`reset-window`は`camera-pan`と同じbuttonを使う。`center-window`と`close-window`は
`move-window`と同じchordを使い、異なる`clicks`値で区別する。`clicks`は`1..=5`である。

## Keybind

chordは`Ctrl`、`Alt`、`Shift`、`Super`とkeyを`+`で連結する。keyは矢印、
`Enter`、英数字1文字に対応する。同じchordを重複して宣言できない。

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

## Window Rule

`app-id`と`title`は完全一致であり、少なくとも一方が必要である。Propertyの優先順位は
Default、matched Window Rule、runtime overrideの順である。複数Ruleが一致すると
ファイル順に合成され、同じPropertyは後のRuleが上書きする。

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
