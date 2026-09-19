# Keybindings

Keybindはすべて共有Actionへ接続され、MouseやIPCと別実装の操作体系を作らない。KDLで`bind`を
1つでも宣言すると、組み込みkeybind一式を置き換えるため、必要なbindをすべて記述する。

## 標準割り当て

| Key | Action |
|---|---|
| `Super+Arrow` / `Super+H/J/K/L` | 方向Focus |
| `Super+Ctrl+Arrow` / `Super+Ctrl+H/J/K/L` | Cameraを1 viewport移動 |
| `Super+Ctrl+Shift+H/J/K/L` | CameraをGrid 1セル移動 |
| `Super+Shift+Arrow` | WindowをGrid 1セル移動 |
| `Super+Ctrl+Shift+Arrow` | WindowをGrid 1セルresize |
| `Super+Shift+H/J/K/L` | 次のWindowの配置方向 |
| `Super+1`〜`Super+9` | 絶対倍率0.1〜0.9 |
| `Super+0` | 絶対倍率1.0 |
| `Super+N` | active Output切り替え |
| `Super+F` | floating切り替え |
| `Super+Enter` | fullscreen切り替え |
| `Super+M` | maximized切り替え |
| `Super+Z` | 初期幅／半幅切り替え |
| `Super+Q` | Windowを閉じる |
| `Super+O` | opacity切り替え |
| `Super+Shift+O` | runtime opacityをclear |
| `Super+B` | blur切り替え |
| `Super+W` | カーソル航跡切り替え |
| `Super+V` | Overview切り替え |
| `Super+S` | Overview選択を確定 |
| `Super+R` | 設定再読み込み |

## KDL例

```kdl
bind "Super+H" "focus-left"
bind "Super+Ctrl+H" "camera-left"
bind "Super+Shift+Left" "move-left"
bind "Super+5" "camera-zoom" 0.5
bind "Super+Q" "close"
```

chordは`Ctrl`、`Alt`、`Shift`、`Super`とkeyを`+`で結ぶ。同じchordは重複できない。
利用可能なAction名と値の範囲は[設定リファレンス](configuration.md)を参照する。

