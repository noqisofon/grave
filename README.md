# colonrogue

vim 風コロンコマンドで遊ぶ古典的ローグライク。人間もAIエージェントも同じゲームを遊べる。

## 構成

- `core/` — ヘッドレスのゲームロジック。入力は `move west` のようなコロンコマンドの文字列だけ
- `tui/` — キーバインド層と描画（crossterm）。キーはコマンド文字列に展開するだけ
- `mcp/` — stdio の MCP サーバー。コマンドを受けてテキスト観測を返す

## 遊ぶ

```sh
cargo run -p colonrogue-tui -- [seed]
```

| キー | 動作 |
| --- | --- |
| `h j k l` / 矢印 | 西 南 北 東 |
| `y u b n` | 北西 北東 南西 南東 |
| `>` | 階段を降りる |
| `_` | 既知の階段まで自動移動 (`travel >`) |
| `x` | 自動探索 (`explore`) |
| `z` | 待つ |
| `;` | 階段の位置を調べる |
| `3j` | 数字プレフィックスで反復 |
| `.` | 直前の行動を繰り返す |
| `:` | コロンコマンド (`Tab` 補完, `↑↓` 履歴, `;` で連続実行) |

`:map <1文字> <コマンド>` で再割り当て、`:unmap`、`:q` で終了。

## エージェントから遊ぶ (MCP)

```sh
cargo build --release -p colonrogue-mcp
claude mcp add colonrogue -- ./target/release/colonrogue-mcp
```

ツール: `command`（`;` 区切りで連続実行、失敗で停止）、`observe`、`new_game`（seed指定可）、`help`。
`command` には任意の `thought`（なぜその手を選んだか）を添えられる。
同じ seed と同じコマンド列は常に同じ結果になる。

## 記録と観戦

MCPサーバーは全ての行動を JSONL に追記する（既定は `colonrogue-record.jsonl`。`--record <path>` で変更、`--no-record` で無効）。
1行1イベントで、`new_game`（seed）と `command`（コマンド・結果・`thought`・実行後の階/ターン）の2種類。
ゲームは seed とコマンド列から完全に再現できるので、マップは記録しない。

```sh
# 別の端末で、エージェントのプレイを観戦する
cargo run -p colonrogue-tui -- --watch colonrogue-record.jsonl
```

観戦側は記録を追いかけて同じゲームを再現し、`thought` を画面下部に表示する。
再現結果が記録とずれたら画面にそう出る。

## 今後

モンスター・アイテム・戦闘・識別、冒険日誌の自動生成、数字キー（テンキー）の扱い（現状は反復カウントに使用）、
SSH経由でのエージェント接続とWeb観戦。
