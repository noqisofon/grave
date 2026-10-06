//! セッション記録（JSONL）。1行1イベント。
//!
//! ゲームは seed とコマンド列から完全に再現できるので、マップ自体は記録しない。
//! 観戦側は `new_game` で Game を作り直し、`command` を順に流すだけでよい。

use serde_json::{json, Value};

use crate::game::Outcome;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    NewGame {
        seed: u64,
    },
    Command {
        command: String,
        /// なぜその手を選んだか（任意。エージェントが添える）
        thought: Option<String>,
        ok: bool,
        message: String,
        /// 実行後の状態（観戦側の再現がずれていないか確かめるため）
        depth: u32,
        turn: u32,
        /// 古い記録には無い
        hp: Option<i32>,
    },
}

impl Event {
    pub fn from_outcome(outcome: &Outcome, thought: Option<&str>) -> Event {
        Event::Command {
            command: outcome.command.clone(),
            thought: thought.map(str::to_string),
            ok: outcome.ok,
            message: outcome.message.clone(),
            depth: outcome.depth,
            turn: outcome.turn,
            hp: Some(outcome.hp),
        }
    }

    pub fn to_line(&self) -> String {
        match self {
            Event::NewGame { seed } => json!({ "kind": "new_game", "seed": seed }),
            Event::Command {
                command,
                thought,
                ok,
                message,
                depth,
                turn,
                hp,
            } => {
                let mut v = json!({
                    "kind": "command",
                    "command": command,
                    "ok": ok,
                    "message": message,
                    "depth": depth,
                    "turn": turn,
                });
                if let Some(t) = thought {
                    v["thought"] = Value::String(t.clone());
                }
                if let Some(h) = hp {
                    v["hp"] = json!(h);
                }
                v
            }
        }
        .to_string()
    }

    pub fn parse(line: &str) -> Result<Event, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        match v.get("kind").and_then(Value::as_str) {
            Some("new_game") => Ok(Event::NewGame {
                seed: n("seed").ok_or("seed がない")?,
            }),
            Some("command") => Ok(Event::Command {
                command: s("command").ok_or("command がない")?,
                thought: s("thought"),
                ok: v.get("ok").and_then(Value::as_bool).unwrap_or(true),
                message: s("message").unwrap_or_default(),
                depth: n("depth").unwrap_or(0) as u32,
                turn: n("turn").unwrap_or(0) as u32,
                hp: v.get("hp").and_then(Value::as_i64).map(|h| h as i32),
            }),
            other => Err(format!("不明な kind: {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;

    #[test]
    fn line_roundtrip() {
        let evs = [
            Event::NewGame { seed: 7 },
            Event::Command {
                command: "move west".into(),
                thought: Some("西に行ってみる".into()),
                ok: true,
                message: "westへ進んだ。".into(),
                depth: 1,
                turn: 3,
                hp: Some(18),
            },
            Event::Command {
                command: "descend".into(),
                thought: None,
                ok: false,
                message: "ここに階段はない。".into(),
                depth: 1,
                turn: 3,
                hp: None,
            },
        ];
        for e in evs {
            assert_eq!(Event::parse(&e.to_line()), Ok(e));
        }
    }

    #[test]
    fn replay_reproduces_the_game() {
        let mut live = Game::new(11);
        let mut lines = vec![Event::NewGame { seed: 11 }.to_line()];
        for (i, script) in ["explore", "travel >", "descend", "wait; move north"]
            .iter()
            .enumerate()
        {
            for o in live.run_script(script) {
                let thought = if i == 0 { Some("まず探索") } else { None };
                lines.push(Event::from_outcome(&o, thought).to_line());
            }
        }

        let mut replay: Option<Game> = None;
        for line in &lines {
            match Event::parse(line).unwrap() {
                Event::NewGame { seed } => replay = Some(Game::new(seed)),
                Event::Command {
                    command,
                    depth,
                    turn,
                    hp,
                    ..
                } => {
                    let g = replay.as_mut().unwrap();
                    g.run(&command);
                    assert_eq!((g.depth(), g.turn()), (depth, turn));
                    assert_eq!(Some(g.hp()), hp);
                }
            }
        }
        assert_eq!(replay.unwrap().observe_text(100), live.observe_text(100));
    }
}
