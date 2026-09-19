# Cameraリファレンス

Mioの画面は、一つの連続した2次元Worldを見るCameraである。Cameraの移動や倍率変更は
WindowのWorld座標、サイズ、所有関係、寿命を変更しない。Cameraの境界は移動の区切りであり、
workspaceやWindow配置の境界ではない。

## 状態

Cameraは次の状態を持つ。

- `position`: World上の左上位置。小数と負数を取れる
- `viewport`: 倍率1.0で画面に入るGridの幅と高さ
- `zoom`: 絶対倍率。現在のKDL Actionでは`0.1..=1.0`

`camera { viewport W H }`は通常倍率の論理viewportを定める。zoomを小さくすると見える
World範囲が広がるが、論理viewportやWindow geometry自体は変更されない。Overviewも別の
Window配置ではなく、このCamera倍率を使う。

## Action

| Action | 意味 |
|---|---|
| `CameraStep(Direction)` | 設定されたviewport 1個分を移動する |
| `CameraNudge(Direction)` | Grid 1セル分を移動する |
| `CameraPan { delta_x, delta_y }` | 小数を含むWorld差分だけ移動する |
| `CameraTo(Window)` | Window中心を含む自然なviewport stopへ移動する |
| `CameraCenter(Window)` | zoomを変えずWindowを画面中央へ置く |
| `CameraFollow(Window)` | 完全に見えていれば動かず、見切れていれば必要な軸だけ追従する |
| `CameraZoom(value)` | Cameraの絶対倍率を変更する |

`CameraFollow`では、Windowが完全に画面外にある場合、または現在倍率で見える範囲より
Windowが大きい場合は、そのWindowを中央へ置く。倍率は変えない。

FocusとCameraは独立した状態であり、Core Actionも分離されている。Keyboardや画面端操作など、
意味上のFocus移動を行う入力adapterは`Focus`と`CameraFollow`を合成する。pointer focusは
Cameraを勝手に移動せず、既にfocusedなWindowへのprotocol activation再通知でも追従を
繰り返さない。

Fullscreen中は、そのOutputの表示を固定するためCamera Actionを拒否する。Fullscreenを解除
すると再び操作できる。

## 複数Output

各OutputはそれぞれCameraを持つ。入力Actionはactive OutputのCameraを対象とし、
`cycle-output`で対象を切り替える。これはWorldやWindowをOutput別containerへ分割する
仕組みではない。
