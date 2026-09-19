# Overview

MioのOverviewは別画面やWindow一覧ではなく、同じWorldをCameraで遠くから見る操作である。
WindowのWorld座標、サイズ、相対位置、identityは変化しない。

## 操作

標準設定では次を使う。

| Key | 動作 |
|---|---|
| `Super+V` | 通常倍率`1.0`とOverview倍率`0.35`を切り替える |
| `Super+S` | focused Windowを中央へ置き、倍率`1.0`へ戻す |
| `Super+Arrow` / `Super+H/J/K/L` | World上のWindowへFocusを移す |
| `Super+Ctrl+Arrow` | Cameraを1 viewport移動する |
| `Super+1`〜`Super+0` | Overviewに限定せず絶対倍率を直接選ぶ |

Overview中のFocus移動も通常と同じWindowとActionを使う。専用の複製Windowや選択一覧はない。
`select-overview`は現在focusedなWindowをCamera中央へ置いて通常倍率へ戻すため、先に方向Focusで
対象を選ぶ。

## 配置との関係

Overviewは表示変換だけであり、新規Windowの配置探索範囲を広げない。zoom out中も通常viewportを
基準に配置するため、遠景に見えている空き領域全体を自動的に埋める動作にはならない。

Window、gap、popup、subsurfaceは同じpresentation scaleで縮小される。見た目上の距離とWorldの
占有範囲の対応を維持し、Overview専用layoutは作らない。

より低レベルなCamera Actionの意味は[Cameraリファレンス](camera.md)を参照する。

