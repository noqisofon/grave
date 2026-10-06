//! stdio で動く最小の MCP サーバー。1行1メッセージの JSON-RPC。

use std::io::{self, BufRead, Write};

use colonrogue_core::{Game, COMMAND_HELP};
use serde_json::{json, Value};

const LOG_LINES: usize = 8;

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
                    "command": { "type": "string", "description": "e.g. \"explore\" or \"travel >; descend\"" }
                },
                "required": ["command"]
            }
        },
        {
            "name": "observe",
            "description": "Show the current map (@ = you, > = stairs, remembered tiles stay) and the recent message log. Does not consume a turn.",
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

fn call_tool(game: &mut Game, name: &str, args: &Value) -> (String, bool) {
    match name {
        "command" => {
            let Some(script) = args.get("command").and_then(Value::as_str) else {
                return ("command (string) is required".to_string(), true);
            };
            let outs = game.run_script(script);
            let mut text = String::new();
            let mut is_error = false;
            for o in &outs {
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
            (
                format!("New game (seed {seed}).\n\n{}", game.observe_text(LOG_LINES)),
                false,
            )
        }
        "help" => (COMMAND_HELP.to_string(), false),
        other => (format!("unknown tool: {other}"), true),
    }
}

fn handle(game: &mut Game, req: &Value) -> Option<Value> {
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
                "serverInfo": { "name": "colonrogue", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let (text, is_error) = call_tool(game, name, &args);
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

fn main() -> io::Result<()> {
    let mut game = Game::new(1);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&mut game, &req),
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
