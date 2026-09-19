# Mio CLI リファレンス

## mio-compositor

```text
mio-compositor [OPTIONS]
```

| Option | 内容 |
|---|---|
| `-c PROGRAM`, `--command PROGRAM` | Mioの起動後に1つのprogramを起動する |
| `--config PATH` | 使用するKDL設定を明示する |
| `--check-config` | 設定を検査して終了する。無効な設定へfallbackしない |
| `--backend winit` | nested winit backendを使う。既定値 |
| `--backend udev` | DRM/KMSへ直接出力するbackendを使う |
| `--xwayland-satellite` | `xwayland-satellite`を起動する |
| `--xwayland-display :N` | satelliteのX displayを指定する。既定値は`:100` |
| `--virtual-outputs N` | nested画面をN個の開発用仮想Outputへ分割する |
| `-h`, `--help` | helpを表示して成功終了する |
| `-V`, `--version` | versionを表示して成功終了する |

`--virtual-outputs`は正の整数だけを受理し、`winit` backendでのみ使用できる。
未知のoption、引数不足、不正値はerrorとして非ゼロ終了する。

`--command`はshell command lineではなく、単一の実行ファイル名である。複数引数を伴う
常駐programはKDLの`spawn-at-startup`へargvとして記述する。

## 終了

`SIGINT`、`SIGTERM`、IPCの`quit`はいずれも同じ通常終了経路を使う。Mioはevent loopを停止し、
起動したxwayland-satelliteを回収してIPC socketを削除する。
