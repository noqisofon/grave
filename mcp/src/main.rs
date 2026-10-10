//! stdio で動く最小の MCP サーバー。1行1メッセージの JSON-RPC。

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};

use grave_core::record::Event;
use grave_core::{journal, Game, COMMAND_HELP};
use serde_json::{json, Value};

const LOG_LINES: usize = 8;
/// 対応する MCP のプロトコルバージョン（新しい順）。
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_RECORD_PATH: &str = "grave-record.jsonl";

/// セッション記録 (JSONL)。書き込みに失敗してもゲームは止めない。
struct Recorder {
    file: Option<File>,
    /// 今のゲームの記録（ファイルに書かない設定でも保持する。日誌の素材になる）
    history: Vec<Event>,
}

impl Recorder {
    fn open(path: Option<&str>) -> Recorder {
        let file = path.and_then(|p| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| eprintln!("grave-mcp: 記録ファイルを開けない ({p}): {e}"))
                .ok()
        });
        Recorder {
            file,
            history: Vec::new(),
        }
    }

    fn write(&mut self, ev: &Event) {
        self.history.push(ev.clone());
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{}", ev.to_line())
                .and_then(|_| f.flush())
                .is_err()
            {
                self.file = None; // 以降は記録しない
            }
        }
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "command",
            "description": format!(
                "Run one or more colon-commands in the roguelike and get the resulting text observation.\n\
                 Separate multiple commands with ';' (execution stops at the first failure).\n\n{COMMAND_HELP}"
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "e.g. \"explore\" or \"travel >; descend\"" },
                    "thought": { "type": "string", "description": "Optional. One or two sentences on why you chose this. Shown to human spectators and used for the adventure journal." }
                },
                "required": ["command"]
            }
        },
        {
            "name": "observe",
            "description": "Show the current map (@ = you, > = stairs (< once you hold the amulet), , = the Amulet on depth 30, s = slime, ! = potion, ? = scroll, / = wand, = = ring, ~ = light/oil, ^ = known trap; remembered tiles stay), active status effects with remaining turns (poison, confusion, blindness, hallucination, ...; while blind no map is shown), visible enemies, the items under your feet (numbered, as used by pickup), your inventory (wands show remaining charges; the equipped light shows its fuel) and the recent message log. Does not consume a turn.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "new_game",
            "description": "Start a new game. The same seed with the same commands always gives the same result.",
            "inputSchema": {
                "type": "object",
                "properties": { "seed": { "type": "integer", "description": "optional seed (default 1)" } }
            }
        },
        {
            "name": "journal",
            "description": "Get the raw material for your adventure journal: a mechanical summary of this game (per-depth highlights, your own thoughts, lowest HP, cause of death). Write the journal in your own words from it, then save it with journal_write. Does not consume a turn.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "journal_write",
            "description": "Save the adventure journal you wrote (Japanese, first person, a few paragraphs: what happened, why you chose what you did, what you regret). It is stored in the session record next to your commands.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string", "description": "the journal text" } },
                "required": ["text"]
            }
        },
        {
            "name": "help",
            "description": "List the available commands.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn call_tool(game: &mut Game, rec: &mut Recorder, name: &str, args: &Value) -> (String, bool) {
    match name {
        "command" => {
            let Some(script) = args.get("command").and_then(Value::as_str) else {
                return ("command (string) is required".to_string(), true);
            };
            let thought = args
                .get("thought")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let outs = game.run_script(script);
            let mut text = String::new();
            let mut is_error = false;
            for (i, o) in outs.iter().enumerate() {
                // 理由は呼び出しの最初のコマンドにだけ添える
                rec.write(&Event::from_outcome(o, if i == 0 { thought } else { None }));
                text.push_str(&format!(
                    "> {}\n  {}: {}\n",
                    o.command,
                    if o.ok { "OK" } else { "FAILED" },
                    o.message
                ));
                is_error |= !o.ok;
            }
            if outs.is_empty() {
                text.push_str("(empty command)\n");
                is_error = true;
            }
            // 失敗で止まったときは、実行しなかった分を知らせる
            let total = script.split(';').filter(|c| !c.trim().is_empty()).count();
            if outs.len() < total {
                text.push_str(&format!(
                    "(stopped at the failure: the remaining {} command(s) were not run)\n",
                    total - outs.len()
                ));
            }
            if game.is_won() {
                text.push_str(
                    "\nクリア！ 地上へ脱出した。journal ツールで冒険日誌の素材が得られる。\n",
                );
            }
            if game.is_dead() {
                text.push_str("\nゲームオーバー。journal ツールで冒険日誌の素材が得られる。\n");
            }
            text.push('\n');
            text.push_str(&game.observe_text(LOG_LINES));
            (text, is_error)
        }
        "observe" => (game.observe_text(LOG_LINES), false),
        "new_game" => {
            let seed = args.get("seed").and_then(Value::as_u64).unwrap_or(1);
            *game = Game::new(seed);
            rec.history.clear();
            rec.write(&Event::new_game(seed));
            (
                format!(
                    "New game (seed {seed}).\n\n{}",
                    game.observe_text(LOG_LINES)
                ),
                false,
            )
        }
        "journal" => {
            let mut text = journal::digest(&rec.history);
            let written = journal::journal_texts(&rec.history);
            if !written.is_empty() {
                text.push_str(&format!(
                    "\n(このゲームの日誌はすでに {} 件保存されている)\n",
                    written.len()
                ));
            }
            text.push_str(
                "\n---\n上は機械的にまとめた記録。これをもとに、あなた自身の言葉で冒険日誌を書いてください\n\
                 (一人称・日本語・数段落。出来事だけでなく、判断の理由や失敗への反省も)。\n\
                 書けたら journal_write ツールで保存します。\n",
            );
            (text, false)
        }
        "journal_write" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if text.is_empty() {
                return ("text (non-empty string) is required".to_string(), true);
            }
            rec.write(&Event::Journal {
                text: text.to_string(),
            });
            ("日誌を保存した。".to_string(), false)
        }
        "help" => (COMMAND_HELP.to_string(), false),
        other => (format!("unknown tool: {other}"), true),
    }
}

/// ツールの実行中に panic しても、サーバーごと落とさずエラーとして返す。
/// （ゲームの状態は panic した時点のまま残る。直らなければ new_game でやり直せる）
fn guarded(f: impl FnOnce() -> (String, bool)) -> (String, bool) {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        (
            "internal error: the game panicked while running this tool. \
             The game state may be inconsistent; call new_game to start over."
                .to_string(),
            true,
        )
    })
}

fn handle(game: &mut Game, rec: &mut Recorder, req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone(); // id が無ければ通知なので返信しない
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            // 要求された版に対応していればそれを、そうでなければ対応する最新の版を返す
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|v| SUPPORTED_VERSIONS.contains(v))
                .unwrap_or(SUPPORTED_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "grave", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let (text, is_error) = guarded(|| call_tool(game, rec, name, &args));
            json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
        }
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {method}") }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// `--record <path>` で記録先を変える。`--no-record` で記録しない。
fn record_path() -> Option<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--no-record") {
        return None;
    }
    match args.iter().position(|a| a == "--record") {
        Some(i) => args.get(i + 1).cloned(),
        None => Some(DEFAULT_RECORD_PATH.to_string()),
    }
}

fn main() -> io::Result<()> {
    let mut game = Game::new(1);
    let mut rec = Recorder::open(record_path().as_deref());
    rec.write(&Event::new_game(1));
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&mut game, &mut rec, &req),
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            })),
        };
        if let Some(r) = reply {
            writeln!(stdout, "{r}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (Game, Recorder) {
        let mut rec = Recorder::open(None);
        rec.write(&Event::new_game(1));
        (Game::new(1), rec)
    }

    fn call(game: &mut Game, rec: &mut Recorder, name: &str, args: Value) -> (String, bool) {
        call_tool(game, rec, name, &args)
    }

    fn rpc(game: &mut Game, rec: &mut Recorder, req: Value) -> Option<Value> {
        handle(game, rec, &req)
    }

    #[test]
    fn initialize_picks_a_supported_protocol_version() {
        let (mut g, mut r) = fresh();
        let ask = |v: &str| json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":v}});
        let res = rpc(&mut g, &mut r, ask("2025-03-26")).unwrap();
        assert_eq!(res["result"]["protocolVersion"], "2025-03-26");
        // 知らない版を要求されたら、対応する最新の版を返す
        let res = rpc(&mut g, &mut r, ask("1999-01-01")).unwrap();
        assert_eq!(res["result"]["protocolVersion"], SUPPORTED_VERSIONS[0]);
        assert_eq!(res["result"]["serverInfo"]["name"], "grave");
    }

    #[test]
    fn notifications_get_no_reply_and_unknown_methods_get_an_error() {
        let (mut g, mut r) = fresh();
        let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(rpc(&mut g, &mut r, note).is_none());
        let res = rpc(
            &mut g,
            &mut r,
            json!({"jsonrpc":"2.0","id":7,"method":"nope"}),
        )
        .unwrap();
        assert_eq!(res["id"], 7);
        assert_eq!(res["error"]["code"], -32601);
        let ping = rpc(
            &mut g,
            &mut r,
            json!({"jsonrpc":"2.0","id":8,"method":"ping"}),
        )
        .unwrap();
        assert_eq!(ping["result"], json!({}));
    }

    #[test]
    fn tools_list_names_every_tool_the_dispatcher_handles() {
        let (mut g, mut r) = fresh();
        let res = rpc(
            &mut g,
            &mut r,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .unwrap();
        let names: Vec<&str> = res["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "command",
                "observe",
                "new_game",
                "journal",
                "journal_write",
                "help"
            ]
        );
        for n in names {
            let (text, is_error) = call(&mut g, &mut r, n, json!({"command": "wait", "text": "x"}));
            assert!(!is_error, "{n}: {text}");
        }
    }

    #[test]
    fn command_runs_scripts_records_them_and_reports_failures() {
        let (mut g, mut r) = fresh();
        let (text, err) = call(
            &mut g,
            &mut r,
            "command",
            json!({"command": "wait; wait", "thought": " 様子見 "}),
        );
        assert!(
            !err && text.contains("> wait") && text.contains("== 地下1階"),
            "{text}"
        );
        // 最初のコマンドにだけ thought が付き、前後の空白は落ちる
        let thoughts: Vec<Option<&str>> = r
            .history
            .iter()
            .filter_map(|e| match e {
                Event::Command { thought, .. } => Some(thought.as_deref()),
                _ => None,
            })
            .collect();
        assert_eq!(thoughts, [Some("様子見"), None]);

        // 失敗で止まり、実行しなかった分を知らせる
        let (text, err) = call(
            &mut g,
            &mut r,
            "command",
            json!({"command": "descend; wait; wait"}),
        );
        assert!(err && text.contains("FAILED"), "{text}");
        assert!(text.contains("remaining 2 command(s)"), "{text}");

        let (text, err) = call(&mut g, &mut r, "command", json!({"command": " ; "}));
        assert!(err && text.contains("(empty command)"), "{text}");
        let (_, err) = call(&mut g, &mut r, "command", json!({}));
        assert!(err);
    }

    #[test]
    fn new_game_resets_the_game_and_the_history() {
        let (mut g, mut r) = fresh();
        call(&mut g, &mut r, "command", json!({"command": "wait"}));
        let (text, err) = call(&mut g, &mut r, "new_game", json!({"seed": 5}));
        assert!(!err && text.contains("seed 5"), "{text}");
        assert_eq!(g.seed(), 5);
        assert_eq!(g.turn(), 0);
        assert_eq!(r.history, [Event::new_game(5)]);
        // seed が数でなければ 1 になる
        call(&mut g, &mut r, "new_game", json!({"seed": "x"}));
        assert_eq!(g.seed(), 1);
    }

    #[test]
    fn journal_write_needs_text_and_is_kept_in_the_history() {
        let (mut g, mut r) = fresh();
        let (_, err) = call(&mut g, &mut r, "journal_write", json!({"text": "  "}));
        assert!(err);
        let (_, err) = call(
            &mut g,
            &mut r,
            "journal_write",
            json!({"text": " 今日の日誌 "}),
        );
        assert!(!err);
        let (text, _) = call(&mut g, &mut r, "journal", json!({}));
        assert!(text.contains("保存されている"), "{text}");
        assert_eq!(
            grave_core::journal::journal_texts(&r.history),
            ["今日の日誌"]
        );
    }

    #[test]
    fn unknown_tools_are_errors() {
        let (mut g, mut r) = fresh();
        let (text, err) = call(&mut g, &mut r, "dance", json!({}));
        assert!(err && text.contains("unknown tool"));
    }

    #[test]
    fn a_panic_in_a_tool_becomes_an_error_result() {
        let (text, err) = guarded(|| panic!("boom"));
        assert!(err && text.contains("new_game"), "{text}");
        assert_eq!(
            guarded(|| ("ok".to_string(), false)),
            ("ok".to_string(), false)
        );
    }
}
