# Window RuleとPropertyリファレンス

MioはWindowの挙動と外観を、一つのProperty systemで扱う。現在利用できるPropertyは
`opacity`、`floating`、`blur`である。

## 実効値の優先順位

各Propertyは独立に、次の順で実効値を決める。

```text
Default < Matched Config Rules < Runtime Override
```

上位層にそのPropertyの値がなければ、直下の層へ戻る。runtime overrideをclearすると
一致中のConfig Ruleの値へ戻り、Config RuleにもなければDefaultへ戻る。

runtime overrideはWindow ID単位である。同じapp-idの別Windowへ波及せず、手書きのKDLも
書き換えない。Windowが閉じられると、そのWindowのoverrideも寿命を終える。

## Ruleの一致

`window-rule`は`app-id`、`title`の完全一致を使い、少なくとも一方が必要である。両方を
指定した場合はAND条件になる。正規表現とglobは現在未対応である。

```kdl
window-rule {
    match app-id="foot"
    opacity 0.9
}

window-rule {
    match app-id="foot" title="main"
    blur true
    opacity 1.0
}
```

複数Ruleが一致した場合は、ファイルに書いた順にPropertyを合成する。同じPropertyを複数の
一致Ruleが指定した場合だけ、後のRuleが上書きする。上の例で`foot`かつtitleが`main`なら、
実効Config層は`opacity=1.0`と`blur=true`になる。

app-idまたはtitleがclientによって変更された場合と、設定の再読み込みに成功した場合は、
既存WindowのConfig Rule層を全て再計算する。この再計算で以前だけ一致していた値は残らない。
runtime override層は保持され、そのPropertyについて引き続きConfig層より優先される。

## floating

`floating=true`は同じWorld内でGrid衝突制約を緩めるPropertyである。別workspaceや別座標系へ
Windowを移す機能ではない。runtimeで`floating=false`を指定すればConfig Ruleの`true`より
優先され、clearすれば再びRuleの値へ戻る。

## 設定再読み込み

`reload-config`は新しい設定全体の検証に成功してから置き換える。不正な設定なら直前の有効な
設定とPropertyを維持する。成功時は既存WindowへRuleを再適用するが、起動時commandは
再実行しない。

