//! stdio で動く最小の MCP サーバー。1行1メッセージの JSON-RPC。

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};

use grave_core::record::Event;
use grave_core::{Game, COMMAND_HELP};
use serde_json::{json, Value};

const LOG_LINES: usize = 8;
const DEFAULT_RECORD_PATH: &str = "grave-record.jsonl";

/// セッション記録 (JSONL)。書き込みに失敗してもゲームは止めない。
struct Recorder {
    file: Option<File>,
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
        Recorder { file }
    }

    fn write(&mut self, ev: &Event) {
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{}", ev.to_line()).and_then(|_| f.flush()).is_err() {
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
            "description": "Show the current map (@ = you, > = stairs, s = slime, ! = potion, ? = scroll; remembered tiles stay), visible enemies, your inventory and the recent message log. Does not consume a turn.",
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
            text.push('\n');
            text.push_str(&game.observe_text(LOG_LINES));
            (text, is_error)
        }
        "observe" => (game.observe_text(LOG_LINES), false),
        "new_game" => {
            let seed = args.get("seed").and_then(Value::as_u64).unwrap_or(1);
            *game = Game::new(seed);
            rec.write(&Event::NewGame { seed });
            (
                format!("New game (seed {seed}).\n\n{}", game.observe_text(LOG_LINES)),
                false,
            )
        }
        "help" => (COMMAND_HELP.to_string(), false),
        other => (format!("unknown tool: {other}"), true),
    }
}

fn handle(game: &mut Game, rec: &mut Recorder, req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone(); // id が無ければ通知なので返信しない
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18");
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
            let (text, is_error) = call_tool(game, rec, name, &args);
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
    rec.write(&Event::NewGame { seed: 1 });
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
