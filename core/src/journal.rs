//! 冒険日誌の素材。
//!
//! 記録（JSONL）から、深さごとの出来事・エージェントの思考・最低HPなどを
//! 機械的にまとめる。日誌そのもの（文章）は、エージェントがこの素材をもとに書く。

use crate::record::Event;

/// 載せる思考の最大数。超えたら均等に間引く。
const MAX_THOUGHTS: usize = 40;
const MAX_LINES_PER_DEPTH: usize = 30;
const MAX_LINE_CHARS: usize = 200;

/// 日誌に載せる価値のある出来事を示す言葉（core が作るメッセージに含まれる）。
const MARKERS: &[&str] = &[
    "倒した",
    "拾った",
    "これは",
    "分かった",
    "力尽きた",
    "現れて",
    "装備した",
    "腐っていた",
    "毒を受けた",
    "空腹",
    "アミュレット",
    "レベルが上がった",
    "クリア！",
];

struct Section {
    depth: u32,
    start_turn: u32,
    kills: u32,
    min_hp: Option<i32>,
    lines: Vec<String>,
    omitted: usize,
    /// (ターン, 思考, 通し番号)
    thoughts: Vec<(u32, String, usize)>,
}

impl Section {
    fn new(depth: u32, start_turn: u32) -> Section {
        Section {
            depth,
            start_turn,
            kills: 0,
            min_hp: None,
            lines: Vec::new(),
            omitted: 0,
            thoughts: Vec::new(),
        }
    }
}

/// 最後の `new_game` 以降（今のゲームぶん）の記録。
pub fn last_game(events: &[Event]) -> &[Event] {
    match events
        .iter()
        .rposition(|e| matches!(e, Event::NewGame { .. }))
    {
        Some(i) => &events[i..],
        None => events,
    }
}

/// 今のゲームについて書かれた日誌。
pub fn journal_texts(events: &[Event]) -> Vec<&str> {
    last_game(events)
        .iter()
        .filter_map(|e| match e {
            Event::Journal { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// 思考を均等に間引くときに残すか。最初と最後は必ず残す。
fn keep_thought(id: usize, total: usize) -> bool {
    if total <= MAX_THOUGHTS {
        return true;
    }
    id == 0
        || id == total - 1
        || (id * MAX_THOUGHTS) / total != ((id + 1) * MAX_THOUGHTS) / total
}

/// 冒険の素材をマークダウンでまとめる。
pub fn digest(events: &[Event]) -> String {
    let events = last_game(events);
    let mut seed = None;
    let mut sections = vec![Section::new(1, 0)];
    let mut commands = 0u32;
    // 最後に見た (階, ターン, HP)
    let mut last: (u32, u32, Option<i32>) = (1, 0, None);
    let mut death: Option<(u32, u32)> = None;
    let mut cleared: Option<u32> = None;
    let mut next_thought_id = 0usize;
    let mut last_thought: Option<&str> = None;
    // ターンが進まなかったコマンドの数（失敗や、すでに探索し尽くした場所での探索など）
    let mut idle = 0u32;
    let mut prev_turn: Option<u32> = None;

    for e in events {
        match e {
            Event::NewGame { seed: s, .. } => seed = Some(*s),
            Event::Journal { .. } => {}
            Event::Command {
                message,
                thought,
                depth,
                turn,
                hp,
                ..
            } => {
                commands += 1;
                if prev_turn == Some(*turn) {
                    idle += 1;
                }
                prev_turn = Some(*turn);
                if *depth != sections.last().map_or(0, |s| s.depth) {
                    sections.push(Section::new(*depth, *turn));
                }
                let sec = sections.last_mut().unwrap();
                if let Some(h) = hp {
                    sec.min_hp = Some(sec.min_hp.map_or(*h, |m| m.min(*h)));
                }
                sec.kills += message.matches("を倒した").count() as u32;
                if cleared.is_none() && message.contains("クリア！") {
                    cleared = Some(*turn);
                }
                if death.is_none() && message.contains("力尽きた") {
                    death = Some((*depth, *turn));
                }
                if MARKERS.iter().any(|m| message.contains(m)) {
                    if sec.lines.len() < MAX_LINES_PER_DEPTH {
                        sec.lines
                            .push(format!("[{turn}] {}", clip(message, MAX_LINE_CHARS)));
                    } else {
                        sec.omitted += 1;
                    }
                }
                if let Some(t) = thought {
                    // 同じ思考が続くときは最初の1回だけ載せる
                    if last_thought != Some(t.as_str()) {
                        sec.thoughts.push((*turn, t.clone(), next_thought_id));
                        next_thought_id += 1;
                        last_thought = Some(t.as_str());
                    }
                }
                last = (*depth, *turn, *hp);
            }
        }
    }

    let mut out = String::from("# 冒険の記録\n\n");
    let status = match death {
        Some((d, t)) => format!("力尽きた (地下{d}階, ターン{t})"),
        None if cleared.is_some() => format!("クリア (アミュレットを持って地上へ脱出, ターン{})", cleared.unwrap()),
        None => match last.2 {
            Some(h) => format!("生存中 (HP {h})"),
            None => "生存中".to_string(),
        },
    };
    out.push_str(&format!(
        "seed: {} / コマンド数: {commands} / 到達: 地下{}階 / ターン{} / 状態: {status}\n",
        seed.map_or("?".to_string(), |s| s.to_string()),
        last.0,
        last.1
    ));
    if idle > 0 {
        out.push_str(&format!(
            "ターンが進まなかったコマンド: {idle}回 (失敗や空振りの繰り返しがないか振り返る材料)\n"
        ));
    }

    for sec in &sections {
        out.push_str(&format!(
            "\n## 地下{}階 (ターン{}〜)\n",
            sec.depth, sec.start_turn
        ));
        let mut facts = vec![format!("倒した敵: {}体", sec.kills)];
        if let Some(h) = sec.min_hp {
            facts.push(format!("最低HP: {h}"));
        }
        out.push_str(&format!("{}\n", facts.join(" / ")));
        for l in &sec.lines {
            out.push_str(&format!("- {l}\n"));
        }
        if sec.omitted > 0 {
            out.push_str(&format!("- (ほか {} 件省略)\n", sec.omitted));
        }
        let thoughts: Vec<_> = sec
            .thoughts
            .iter()
            .filter(|(_, _, id)| keep_thought(*id, next_thought_id))
            .collect();
        if !thoughts.is_empty() {
            out.push_str("\n思考:\n");
            for (turn, t, _) in thoughts {
                out.push_str(&format!("- [{turn}] {}\n", clip(t, MAX_LINE_CHARS)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;

    fn cmd(command: &str, message: &str, thought: Option<&str>, depth: u32, turn: u32, hp: i32) -> Event {
        Event::Command {
            command: command.into(),
            thought: thought.map(str::to_string),
            ok: true,
            message: message.into(),
            depth,
            turn,
            hp: Some(hp),
        }
    }

    fn sample() -> Vec<Event> {
        vec![
            Event::new_game(7),
            cmd("explore", "6歩探索したところで、スライムが現れて中断した。 緑の薬を拾った。(a)", Some("まず探索する"), 1, 6, 20),
            cmd("attack east", "スライムに4のダメージを与えた。(HP 2/6) スライムの攻撃！ 2のダメージを受けた。(HP 12/20)", None, 1, 7, 12),
            cmd("attack east", "スライムに3のダメージ。スライムを倒した！", Some("止めを刺す"), 1, 8, 12),
            cmd("use a", "緑の薬を飲んだ。これは毒の薬だった！ 5のダメージを受けた。(HP 7/20)", Some("未識別の薬を試す"), 1, 9, 7),
            cmd("descend", "地下2階に降りた。", None, 2, 30, 14),
            cmd("wait", "1ターン待った。", None, 2, 31, 14),
            cmd("wait", "1ターン待った。 スライムの攻撃！ 14のダメージを受けた。 あなたは力尽きた…。ゲームオーバー。", Some("油断した"), 2, 32, 0),
        ]
    }

    #[test]
    fn digest_groups_by_depth_and_picks_out_highlights() {
        let d = digest(&sample());
        assert!(d.contains("seed: 7"), "{d}");
        assert!(d.contains("## 地下1階 (ターン0〜)"), "{d}");
        assert!(d.contains("## 地下2階 (ターン30〜)"), "{d}");
        assert!(d.contains("倒した敵: 1体 / 最低HP: 7"), "{d}");
        assert!(d.contains("緑の薬を拾った"), "{d}");
        assert!(d.contains("これは毒の薬だった"), "{d}");
        assert!(d.contains("力尽きた (地下2階, ターン32)"), "{d}");
        assert!(d.contains("[9] 未識別の薬を試す"), "{d}");
        // 普通の殴り合いや待機は載らない
        assert!(!d.contains("1ターン待った。\n"), "{d}");
    }

    #[test]
    fn digest_only_covers_the_latest_game() {
        let mut evs = vec![Event::new_game(1), cmd("explore", "古いゲームの出来事。スライムを倒した！", None, 1, 5, 20)];
        evs.extend(sample());
        let d = digest(&evs);
        assert!(!d.contains("古いゲーム"));
        assert!(d.contains("seed: 7"));
    }

    #[test]
    fn thoughts_are_thinned_but_keep_both_ends() {
        let mut evs = vec![Event::new_game(1)];
        for i in 0..200u32 {
            evs.push(cmd("wait", "1ターン待った。", Some(&format!("考え{i}")), 1, i + 1, 20));
        }
        let d = digest(&evs);
        let n = d.matches("- [").count();
        assert!(n <= MAX_THOUGHTS + 1, "{n}");
        assert!(d.contains("考え0\n"));
        assert!(d.contains("考え199\n"));
    }

    #[test]
    fn repeated_thoughts_are_collapsed_and_idle_commands_are_counted() {
        let mut evs = vec![Event::new_game(1)];
        evs.push(cmd("explore", "3歩探索した。", Some("探索する"), 1, 3, 20));
        // ターンが進まないまま、同じ思考で同じコマンドを繰り返す
        for _ in 0..50 {
            evs.push(cmd("explore", "もう探索する場所がない。", Some("探索する"), 1, 3, 20));
        }
        evs.push(cmd("wait", "1ターン待った。", Some("待つ"), 1, 4, 20));
        evs.push(cmd("wait", "1ターン待った。", Some("探索する"), 1, 5, 20));
        let d = digest(&evs);
        assert_eq!(d.matches("探索する\n").count(), 2, "{d}"); // 最初と、待つの後の1回
        assert!(d.contains("ターンが進まなかったコマンド: 50回"), "{d}");
    }

    #[test]
    fn journals_belong_to_the_latest_game() {
        let evs = vec![
            Event::new_game(1),
            Event::Journal { text: "古い日誌".into() },
            Event::new_game(2),
            Event::Journal { text: "新しい日誌".into() },
        ];
        assert_eq!(journal_texts(&evs), vec!["新しい日誌"]);
    }

    #[test]
    fn digest_of_a_real_session() {
        let mut g = Game::new(12);
        let mut evs = vec![Event::new_game(12)];
        for script in ["explore", "wait", "inventory", "wait; wait"] {
            for o in g.run_script(script) {
                evs.push(Event::from_outcome(&o, Some("様子を見る")));
            }
        }
        let d = digest(&evs);
        assert!(d.starts_with("# 冒険の記録"));
        assert!(d.contains("seed: 12"));
        assert!(d.contains("## 地下1階"));
        assert!(d.contains("思考:"));
    }
}
