# Mio IPC リファレンス

## 接続

Mioは`$XDG_RUNTIME_DIR/mio-WAYLAND_SOCKET.sock`へinstance固有のUnix socketを作り、
所有者だけが読み書きできる`0600`にする。Mioが起動した子processには実際のpathを
`MIO_SOCKET`として渡す。

`mioctl`は既定で`MIO_SOCKET`へ接続する。Mioの外から操作する場合は明示する。

```sh
mioctl --socket /run/user/1000/mio-wayland-2.sock state
```

要求はUTF-8のcommand line 1件で、最初の改行またはEOFまでを読む。上限は4096 byteである。
応答はJSON object 1件で、末尾改行はprotocol上必須ではない。

## mioctlの終了status

- `0`: help/version表示、またはMioが`{"ok":true,...}`を返した
- 非ゼロ: 引数、接続、通信の失敗、またはMioが`{"ok":false,...}`を返した

成功応答は標準出力へ、CLI errorとMioの拒否理由は標準errorへ出す。

## 読み取りcommand

| Command | 応答の主要field |
|---|---|
| `state` | `windows`, `focused_window`, `camera`, `outputs`, `cameras`を同一snapshotで返す |
| `windows` | `windows` |
| `focused-window` | `window`。Focusがなければ`null` |
| `camera` | active Cameraを`camera`へ返す |
| `outputs` | adapter Outputの`outputs`とCore Cameraの`cameras` |

Window objectは次のfieldを持つ。

| Field | 内容 |
|---|---|
| `id` | instance内のWindow ID |
| `app_id`, `title` | xdg-toplevel metadata。なければ`null` |
| `rect` | World Grid上の`x`, `y`, `width`, `height` |
| `focused` | Focus中か |
| `presentation` | `normal`, `maximized`, `fullscreen` |
| `opacity`, `floating`, `blur` | 現在有効なWindow Property |

Camera objectは`output_id`, `x`, `y`, `zoom`を持つ。Output objectは`id`, `name`,
logical geometryの`x`, `y`, `width`, `height`と`scale`を持つ。

## Action command

`DIR`は`left`, `right`, `up`, `down`、`ID`は読み取りcommandで取得したunsigned integerである。

| Command | 内容 |
|---|---|
| `activate-output ID` | active Output Cameraを変更する |
| `focus ID` | WindowへFocusする |
| `camera-to ID` | CameraをWindow中央へ移す |
| `camera-step DIR` | active Cameraを1 viewport進める |
| `move-window ID DIR` | WindowをGrid 1 cell移動する |
| `resize-window ID DIR` | WindowをGrid 1 cell resizeする |
| `toggle-floating ID` | Windowのfloating Propertyを切り替える |
| `close ID` | Windowへcloseを要求する |
| `set-property ID opacity FLOAT` | runtime opacity overrideを設定する |
| `set-property ID floating BOOL` | runtime floating overrideを設定する |
| `set-property ID blur BOOL` | runtime blur overrideを設定する |
| `clear-property ID PROPERTY` | 指定したruntime overrideを消す |

`PROPERTY`は`opacity`, `floating`, `blur`、`BOOL`は`true`または`false`である。
`set-opacity ID FLOAT`と`clear-opacity ID`は互換用aliasとして維持する。新しい連携では
`set-property`と`clear-property`を使う。

Action commandはkeyboardとmouseと同じCore Action経路を使用する。IPC専用のWindow状態を
持たない。

## Lifecycle command

`quit`は`Action`ではなくcompositor lifecycle commandである。`{"ok":true}`を返した後、
通常の終了処理へ進む。

## 応答

成功時は必ず`ok`が`true`になる。

```json
{"ok":true}
```

失敗時は`ok`が`false`になり、`error`へ人間が読める理由を返す。

```json
{"ok":false,"error":"unknown window 9"}
```

現在のIPCは短命なlocal CLI接続用である。subscription、event stream、長時間接続client、
remote transportは提供しない。
