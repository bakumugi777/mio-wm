# Mio 1.0 Readiness

この文書はPhase 18の公開面と、`docs/requirements.md`の1.0候補要件を分けて追跡する。
文書やAPIが固定できても、実機検証が不足していれば1.0 readyとは判定しない。

最終監査日: 2026-09-30

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

## 確認済み項目

以下はこの開発期間中に実機または自動testで確認済みである。

- native Wayland applicationの起動、入力、移動、resize、popup
- clipboardのapplication間copy/paste
- drag-and-dropとdrag icon
- fullscreen、session lock、config reload、正常終了
- xwayland-satellite経由のX11 application
- OBSによるscreen captureと録画再生
- fractional scale 1.25でのpointer hit test、cursor wake、grim、OBS capture
- ゲーム用途のpointer/keyboard入力
- 画面開閉、Camera、Overview、Window Rule、runtime Property
- NixOS moduleの`nixos-rebuild`、SDDMのMio sessionへのログインと`mioctl quit`による終了
- Mio session内のportal、推奨package、xwayland-satellite
- サブ機で1週間の日常利用と、そこで発見した仕上げ項目の修正後確認
- メイン機でMioを日常的に継続利用
- fcitx5による日本語入力、preedit、確定、候補選択、候補popupの移動追従、lock/unlock後の復帰

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
