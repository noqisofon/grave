//! セッション記録（JSONL）。1行1イベント。
//!
//! ゲームは seed とコマンド列から完全に再現できるので、マップ自体は記録しない。
//! 観戦側は `new_game` で Game を作り直し、`command` を順に流すだけでよい。

use serde_json::{json, Value};

use crate::game::Outcome;
use crate::status::{Change, Status, StatusEvent};

/// ルールの版。seed とコマンド列から同じ結果にならなくなる変更（マップ・敵・アイテムの
/// 生成や抽選、ダメージ計算、乱数の使い方など）をしたら、必ず 1 上げる。
/// 記録の `new_game` に入り、観戦側が「古いルールで録られた記録」を見分けるのに使う。
/// 上げ忘れは `rules_version_matches_golden_run` が検出する。
pub const RULES_VERSION: u32 = 13;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    NewGame {
        seed: u64,
        /// 録ったときの [`RULES_VERSION`]。古い記録には無い
        rules: Option<u32>,
    },
    /// エージェントが書いた冒険日誌
    Journal {
        text: String,
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
        /// このコマンドの間に起きた状態の付与・解除
        status_events: Vec<StatusEvent>,
        /// 実行後にかかっている状態（残りターンつき）
        statuses: Vec<(Status, u32)>,
    },
}

fn status_event_json(e: &StatusEvent) -> Value {
    let mut v = json!({ "target": e.target, "status": e.status.key() });
    match e.change {
        Change::Apply(turns) => {
            v["change"] = json!("apply");
            v["turns"] = json!(turns);
        }
        Change::End => v["change"] = json!("end"),
    }
    v
}

fn status_event_parse(v: &Value) -> Option<StatusEvent> {
    let change = match v.get("change")?.as_str()? {
        "apply" => Change::Apply(v.get("turns")?.as_u64()? as u32),
        "end" => Change::End,
        _ => return None,
    };
    Some(StatusEvent {
        target: v.get("target")?.as_str()?.to_string(),
        status: Status::from_key(v.get("status")?.as_str()?)?,
        change,
    })
}

impl Event {
    /// 今のルールで新しいゲームを始めた記録。
    pub fn new_game(seed: u64) -> Event {
        Event::NewGame {
            seed,
            rules: Some(RULES_VERSION),
        }
    }

    pub fn from_outcome(outcome: &Outcome, thought: Option<&str>) -> Event {
        Event::Command {
            command: outcome.command.clone(),
            thought: thought.map(str::to_string),
            ok: outcome.ok,
            message: outcome.message.clone(),
            depth: outcome.depth,
            turn: outcome.turn,
            hp: Some(outcome.hp),
            status_events: outcome.status_events.clone(),
            statuses: outcome.statuses.clone(),
        }
    }

    pub fn to_line(&self) -> String {
        match self {
            Event::NewGame { seed, rules } => {
                let mut v = json!({ "kind": "new_game", "seed": seed });
                if let Some(r) = rules {
                    v["rules"] = json!(r);
                }
                v
            }
            Event::Journal { text } => json!({ "kind": "journal", "text": text }),
            Event::Command {
                command,
                thought,
                ok,
                message,
                depth,
                turn,
                hp,
                status_events,
                statuses,
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
                if !status_events.is_empty() {
                    v["status_events"] = Value::Array(status_events.iter().map(status_event_json).collect());
                }
                if !statuses.is_empty() {
                    let mut m = serde_json::Map::new();
                    for (st, n) in statuses {
                        m.insert(st.key().to_string(), json!(n));
                    }
                    v["statuses"] = Value::Object(m);
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
                rules: n("rules").map(|r| r as u32),
            }),
            Some("command") => Ok(Event::Command {
                command: s("command").ok_or("command がない")?,
                thought: s("thought"),
                ok: v.get("ok").and_then(Value::as_bool).unwrap_or(true),
                message: s("message").unwrap_or_default(),
                depth: n("depth").unwrap_or(0) as u32,
                turn: n("turn").unwrap_or(0) as u32,
                hp: v.get("hp").and_then(Value::as_i64).map(|h| h as i32),
                status_events: v
                    .get("status_events")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(status_event_parse).collect())
                    .unwrap_or_default(),
                statuses: v
                    .get("statuses")
                    .and_then(Value::as_object)
                    .map(|m| {
                        Status::ALL
                            .iter()
                            .filter_map(|st| Some((*st, m.get(st.key())?.as_u64()? as u32)))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            Some("journal") => Ok(Event::Journal {
                text: s("text").ok_or("text がない")?,
            }),
            other => Err(format!("不明な kind: {other:?}")),
        }
    }
}

#[cfg(test)]
/// 決まった手順で遊ぶ小さな自動プレイ。戦闘・装備・アイテム・階段をひととおり踏む。
pub fn golden_player(seed: u64, steps: usize, mut sink: impl FnMut(&crate::game::Outcome)) {
    let mut g = crate::Game::new(seed);
    for step in 0..steps {
        if g.is_dead() {
            break;
        }
        let me = g.pos();
        let cmd = match g.visible_enemies().iter().min_by_key(|e| {
            (e.pos.0 - me.0).abs().max((e.pos.1 - me.1).abs())
        }) {
            Some(e) => {
                let (dx, dy) = (e.pos.0 - me.0, e.pos.1 - me.1);
                if dx.abs().max(dy.abs()) <= 1 {
                    let d = crate::Dir::ALL.iter().find(|d| d.delta() == (dx, dy)).unwrap();
                    format!("attack {}", d.name())
                } else {
                    "wait".to_string()
                }
            }
            None => match step % 4 {
                0 => {
                    // 装備を試し、食べられそうなものがあれば食べる
                    let eat = g
                        .inventory_lines()
                        .iter()
                        .find(|l| l.contains("キノコ") || l.contains("パン") || l.contains("干し肉"))
                        .and_then(|l| l.chars().next());
                    let mut c = "equip a; equip b; equip c".to_string();
                    if let Some(letter) = eat {
                        c.push_str(&format!("; eat {letter}"));
                    }
                    c
                }
                1 => "explore".to_string(),
                3 if step % 8 == 3 => "stay 3".to_string(),
                3 => "explore".to_string(),
                _ => "travel >; descend".to_string(),
            },
        };
        // 失敗しても止めず、1つずつ実行する
        for part in cmd.split(';') {
            if g.is_dead() {
                break;
            }
            sink(&g.run(part.trim()));
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
            Event::new_game(7),
            Event::Journal {
                text: "今日は西へ行った。\n毒の薬には気をつけたい。".into(),
            },
            Event::Command {
                command: "move west".into(),
                thought: Some("西に行ってみる".into()),
                ok: true,
                message: "westへ進んだ。".into(),
                depth: 1,
                turn: 3,
                hp: Some(18),
                status_events: vec![
                    StatusEvent {
                        target: "player".into(),
                        status: Status::Confused,
                        change: Change::Apply(8),
                    },
                    StatusEvent {
                        target: "スライム".into(),
                        status: Status::Paralyzed,
                        change: Change::End,
                    },
                ],
                statuses: vec![(Status::Poisoned, 2), (Status::Confused, 8)],
            },
            Event::Command {
                command: "descend".into(),
                thought: None,
                ok: false,
                message: "ここに階段はない。".into(),
                depth: 1,
                turn: 3,
                hp: None,
                status_events: vec![],
                statuses: vec![],
            },
        ];
        for e in evs {
            assert_eq!(Event::parse(&e.to_line()), Ok(e));
        }
    }

    #[test]
    fn replay_reproduces_the_game() {
        let mut live = Game::new(11);
        let mut lines = vec![Event::new_game(11).to_line()];
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
                Event::NewGame { seed, .. } => replay = Some(Game::new(seed)),
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
                Event::Journal { .. } => {}
            }
        }
        assert_eq!(replay.unwrap().observe_text(100), live.observe_text(100));
    }

    #[test]
    fn new_game_carries_rules_version_and_old_records_have_none() {
        let line = Event::new_game(3).to_line();
        assert!(line.contains(&format!("\"rules\":{RULES_VERSION}")), "{line}");
        assert_eq!(
            Event::parse(&line),
            Ok(Event::NewGame { seed: 3, rules: Some(RULES_VERSION) })
        );
        // rules のない古い記録も読める
        assert_eq!(
            Event::parse(r#"{"kind":"new_game","seed":3}"#),
            Ok(Event::NewGame { seed: 3, rules: None })
        );
    }

    /// 決まった seed とコマンドの結果を指紋にして固定する。ルールを変えてこのテストが
    /// 落ちたら、意図した変更なら RULES_VERSION を上げて GOLDEN_* を更新する。
    #[test]
    fn rules_version_matches_golden_run() {
        const GOLDEN_RULES: u32 = 13;
        const GOLDEN_HASH: u64 = 4339741753145154686;
        let mut h: u64 = 0xcbf29ce484222325; // FNV-1a
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        let (mut hits, mut kills, mut equips, mut deepest) = (0, 0, 0, 0);
        let (mut meals, mut poisoned) = (0, 0);
        for seed in 1u64..=12 {
            golden_player(seed, 400, |o| {
                feed(o.message.as_bytes());
                feed(&[o.depth as u8, o.turn as u8, o.hp as u8, o.ok as u8]);
                hits += o.message.matches("の攻撃！").count();
                kills += o.message.matches("を倒した").count();
                equips += o.message.matches("を装備した").count();
                meals += o.message.matches("満腹度が").count();
                poisoned += o.message.matches("毒を受けた").count();
                deepest = deepest.max(o.depth);
            });
        }
        // 指紋が何も踏んでいないと、ルールが変わっても気づけない
        assert!(hits > 20 && kills > 10 && equips >= 2 && deepest >= 3 && meals >= 3 && poisoned >= 1,
            "{hits} {kills} {equips} {deepest} {meals} {poisoned}"
        );
        if RULES_VERSION == GOLDEN_RULES {
            assert_eq!(
                h, GOLDEN_HASH,
                "ルールが変わったようだ。意図した変更なら RULES_VERSION を上げ、GOLDEN_RULES と GOLDEN_HASH ({h}) を更新する"
            );
        } else {
            panic!("RULES_VERSION を上げたので GOLDEN_RULES = {RULES_VERSION}, GOLDEN_HASH = {h} に更新する");
        }
    }
}
