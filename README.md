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
同じ seed と同じコマンド列は常に同じ結果になる。

## 今後

モンスター・アイテム・戦闘・識別、数字キー（テンキー）の扱い（現状は反復カウントに使用）、リプレイ記録。
