# Mio 1.0 Readiness

この文書はPhase 18の公開面と、`docs/requirements.md`の1.0候補要件を分けて追跡する。
文書やAPIが固定できても、実機検証が不足していれば1.0 readyとは判定しない。

最終監査日: 2026-09-19

## Phase 18 公開面

| 対象 | 状態 | 根拠 |
|---|---|---|
| KDL syntax | 固定候補 | [Configuration](configuration.md)、`--check-config`、parser test |
| Action names | 固定候補 | ConfigurationとKeybindings、組み込み設定との一致test |
| Window Property names | 固定候補 | `opacity`、`floating`、`blur`に統一 |
| IPC | 固定候補 | [IPC](ipc.md)、error終了status test |
| Camera semantics | 固定候補 | [Camera](camera.md)、Core unit test |
| Window Rule semantics | 固定候補 | [Window Property](window-properties.md)、合成順と優先順位test |
| config path | 固定候補 | XDG標準path、`--config`、reload失敗時の保持 |
| CLI interface | 固定候補 | [CLI](cli.md)、help/version parse test |

「固定候補」は、1.0まで互換性を意識して変更する公開面を表す。設計上必要な修正を禁止する
意味ではない。破壊的変更が必要なら文書、設定例、組み込み値、testを同時に更新する。

## 公開文書

Phase 18で要求されたREADME、Installation、Getting Started、Configuration、Keybindings、
Window Rules、Camera Model、Overview、IPC、Troubleshooting、Architecture overviewは揃っている。

## 1.0 blocker

### 物理Multi-monitorの安定性

要件はMulti-monitorの安定を求めているが、現在のdirect backendは単一GPU・単一接続Outputを
主対象としており、実機も1台のmonitorでしか検証できていない。nested仮想Output testは
物理hotplug、異種解像度、scale混在の代替にはならない。

完了条件:

- 2台以上の物理Outputで起動、Focus、Camera、popup、fullscreenを確認
- 接続・切断・再接続を確認
- 異なる解像度とscaleの組合せを確認
- 問題を再現するためのtestまたは診断logを残す

### IMEの安定性

text-input、input-method、virtual-keyboard protocolは実装済みだが、正式なMio desktop sessionで
fcitx5による入力、preedit、候補popupをまだ完了判定していない。

完了条件:

- footとGTK applicationで日本語入力
- preedit、確定、候補選択
- Focus移動、popup位置、Window移動後の追従
- lock/unlock後の復帰

### 日常利用期間

約5.4時間の長時間監視ではRSS、thread、FDが終盤で安定していたが、Phase 17の受け入れ条件は
「一定期間メインWMとして利用し、重大な回避不能問題がないこと」である。単発の長時間起動だけで
完了にはしない。

完了条件:

- 複数日の通常利用
- suspend/resumeとVT切り替えを繰り返す
- crash、入力不能、復帰不能がないことを記録

### Install/session packagingの実機確認

再現可能なNix flake package、NixOS module、Wayland session entryは実装済みであり、隔離build、
設定検査、module評価も自動確認している。残るのは、空の利用者環境へmoduleを導入し、SDDMから
選択してログイン・ログアウトできることの実機確認である。

完了条件:

- NixOS moduleを有効にして`nixos-rebuild`が成功する
- SDDMに「Mio」が表示される
- SDDMからMioへログインし、`mioctl quit`でSDDMへ戻る
- portal、推奨package、任意のXwayland設定がsession内で利用できる

## 確認済み項目

以下はこの開発期間中に実機または自動testで確認済みである。

- native Wayland applicationの起動、入力、移動、resize、popup
- clipboardのapplication間copy/paste
- drag-and-dropとdrag icon
- fullscreen、session lock、config reload、正常終了
- xwayland-satellite経由のX11 application
- OBSによるscreen captureと録画再生
- ゲーム用途のpointer/keyboard入力
- 画面開閉、Camera、Overview、Window Rule、runtime Property

手動確認済み項目も、将来の変更で自動的に保証されるわけではない。release candidateでは再確認する。

## 1.0後へ延期

- Yaldra
- built-in XWayland
- 高度なshader、pseudo-3D、追加layout
- globまたは正規表現Window Rule
- IPC subscription、event stream、remote transport
- runtime rule永続化

## Release candidate手順

1. blockerを解消する、または1.0要件自体を明示的に再決定する
2. `cargo fmt --all -- --check`
3. `cargo test --workspace`
4. `cargo build --workspace`
5. `cargo clippy --workspace --all-targets -- -D warnings`
6. `config/mio.kdl`を`--check-config`で検査する
7. Getting StartedとInstallationを空の利用環境から再現する
8. 手動確認項目をrelease candidate binaryで再実施する
9. 既知制限と互換性変更をrelease noteへ記載する
