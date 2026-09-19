# Architecture Overview

Mioの設計は次の一文を基準にする。

> 世界は一つ。ウィンドウはその中に住み、画面はその世界を見るカメラにすぎない。

## 層

```text
Wayland / Smithay / DRM / Input
              ↓
       mio-compositor adapter
              ↓
          Action boundary
              ↓
            mio-core
```

`mio-core`はSmithayへ依存せず、World、Grid、Window、Camera、Focus、Property、Actionの
論理状態を管理する。`mio-compositor`はWayland protocol、入力、描画、animation、IPCを
Coreへ接続するadapterである。

## 一つのWorld

従来型workspace containerは作らない。WindowはWorld座標を持ち、Camera境界を跨いでも同じ
Windowであり続ける。floatingも別workspaceではなく、同じWorld内でGrid制約を緩めた状態である。
Overviewは別layoutではなくCamera zoomである。

## 一つの操作経路

```text
Keyboard / Mouse / IPC / 将来のYaldra
                    ↓
                  Action
                    ↓
                  World
```

入力方法ごとにFocus、移動、resize等を再実装しない。複数primitiveが必要な操作は、既存Actionを
adapterで合成する。

## 一つのProperty system

Windowの挙動と外観はDefault、Config Rule、Runtime Overrideの3層で決める。runtime変更は
Window単位で、手書きKDLを書き換えない。詳細は[Window RuleとProperty](window-properties.md)を
参照する。

## 論理状態と描画状態

AnimationはWorld geometryをframeごとに変更しない。論理位置・目標位置と、補間された表示位置を
分離する。blur、shadow、カーソル航跡等は描画層に属し、Window管理の正しさに必須ではない。

内部の詳細な責務とdata flowは[architecture.md](architecture.md)、要件上の制約は
[requirements.md](requirements.md)を参照する。

