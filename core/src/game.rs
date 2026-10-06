use std::collections::VecDeque;

use crate::command::{self, Command, Dir, TravelTarget};
use crate::map::{idx, Map, H, W};
use crate::rng::Rng;

const FOV_RADIUS: i32 = 9;
const EXPLORE_STEP_LIMIT: u32 = 1000;

pub struct LogEntry {
    pub turn: u32,
    pub text: String,
}

/// 1コマンドの実行結果。
pub struct Outcome {
    pub command: String,
    pub ok: bool,
    pub message: String,
    /// 実行直後の階とターン
    pub depth: u32,
    pub turn: u32,
}

/// 描画用の1マス。
pub struct Cell {
    pub ch: char,
    /// 今見えている
    pub visible: bool,
    /// 一度でも見た（記憶している）
    pub seen: bool,
}

pub struct Game {
    seed: u64,
    rng: Rng,
    depth: u32,
    turn: u32,
    pos: (i32, i32),
    stairs: (i32, i32),
    map: Map,
    log: Vec<LogEntry>,
}

impl Game {
    pub fn new(seed: u64) -> Game {
        let mut rng = Rng::new(seed);
        let g = Map::generate(&mut rng);
        let mut game = Game {
            seed,
            rng,
            depth: 1,
            turn: 0,
            pos: g.start,
            stairs: g.stairs,
            map: g.map,
            log: Vec::new(),
        };
        game.map.update_fov(game.pos, FOV_RADIUS);
        game.push_log("冒険が始まった。");
        game
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
    pub fn depth(&self) -> u32 {
        self.depth
    }
    pub fn turn(&self) -> u32 {
        self.turn
    }
    pub fn pos(&self) -> (i32, i32) {
        self.pos
    }
    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    fn push_log(&mut self, text: &str) {
        self.log.push(LogEntry {
            turn: self.turn,
            text: text.to_string(),
        });
    }

    fn new_level(&mut self) {
        let g = Map::generate(&mut self.rng);
        self.map = g.map;
        self.pos = g.start;
        self.stairs = g.stairs;
        self.map.update_fov(self.pos, FOV_RADIUS);
    }

    fn step_to(&mut self, p: (i32, i32)) {
        self.pos = p;
        self.turn += 1;
        self.map.update_fov(self.pos, FOV_RADIUS);
    }

    /// 現在地から、既知の歩ける床だけを通って最寄りのゴールまでの経路（現在地は含まない）。
    fn find_path(&self, is_goal: &dyn Fn((i32, i32)) -> bool) -> Option<Vec<(i32, i32)>> {
        let n = (W * H) as usize;
        let mut prev = vec![usize::MAX; n];
        let start = idx(self.pos.0, self.pos.1);
        prev[start] = start;
        let mut q = VecDeque::new();
        q.push_back(self.pos);
        while let Some(p) = q.pop_front() {
            if p != self.pos && is_goal(p) {
                let mut path = Vec::new();
                let mut c = idx(p.0, p.1);
                while c != start {
                    path.push(((c as i32) % W, (c as i32) / W));
                    c = prev[c];
                }
                path.reverse();
                return Some(path);
            }
            for d in Dir::ALL {
                let (dx, dy) = d.delta();
                let np = (p.0 + dx, p.1 + dy);
                if !Map::in_bounds(np.0, np.1)
                    || !self.map.is_seen(np.0, np.1)
                    || !self.map.tile(np.0, np.1).walkable()
                {
                    continue;
                }
                let ni = idx(np.0, np.1);
                if prev[ni] != usize::MAX {
                    continue;
                }
                prev[ni] = idx(p.0, p.1);
                q.push_back(np);
            }
        }
        None
    }

    /// 既知の歩ける床で、まだ見ていない（壁ではない）隣接マスを持つ場所。
    fn is_frontier(&self, p: (i32, i32)) -> bool {
        Dir::ALL.iter().any(|d| {
            let (dx, dy) = d.delta();
            let (x, y) = (p.0 + dx, p.1 + dy);
            Map::in_bounds(x, y) && !self.map.is_seen(x, y) && self.map.tile(x, y).walkable()
        })
    }

    /// `;` 区切りで複数コマンドを順に実行する。失敗した時点で止まる。
    pub fn run_script(&mut self, script: &str) -> Vec<Outcome> {
        let mut outs = Vec::new();
        for part in script.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let o = self.run(part);
            let ok = o.ok;
            outs.push(o);
            if !ok {
                break;
            }
        }
        outs
    }

    /// 1つのコロンコマンド文字列を実行する。
    pub fn run(&mut self, line: &str) -> Outcome {
        match command::parse(line) {
            Ok(cmd) => self.exec(cmd),
            Err(msg) => Outcome {
                command: line.trim().to_string(),
                ok: false,
                message: msg,
                depth: self.depth,
                turn: self.turn,
            },
        }
    }

    pub fn exec(&mut self, cmd: Command) -> Outcome {
        let (ok, message) = match cmd {
            Command::Move(d) => {
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                if self.map.tile(t.0, t.1).walkable() {
                    self.step_to(t);
                    if self.pos == self.stairs {
                        (true, "階段の上にいる。(descend で降りられる)".to_string())
                    } else {
                        (true, format!("{}へ進んだ。", d.name()))
                    }
                } else {
                    (false, "壁にぶつかった。".to_string())
                }
            }
            Command::Descend => {
                if self.pos == self.stairs {
                    self.depth += 1;
                    self.turn += 1;
                    self.new_level();
                    (true, format!("地下{}階に降りた。", self.depth))
                } else {
                    (false, "ここに階段はない。".to_string())
                }
            }
            Command::Wait => {
                self.turn += 1;
                (true, "1ターン待った。".to_string())
            }
            Command::Look => (true, self.describe_stairs()),
            Command::Travel(TravelTarget::Stairs) => {
                if self.pos == self.stairs {
                    (true, "すでに階段の上にいる。".to_string())
                } else if !self.map.is_seen(self.stairs.0, self.stairs.1) {
                    (false, "階段の場所をまだ知らない。explore で探そう。".to_string())
                } else {
                    let stairs = self.stairs;
                    match self.find_path(&|p| p == stairs) {
                        Some(path) => {
                            let n = path.len();
                            for p in path {
                                self.step_to(p);
                            }
                            (true, format!("階段まで{n}歩移動した。"))
                        }
                        None => (false, "階段までの道がつながっていない。".to_string()),
                    }
                }
            }
            Command::Explore => self.explore(),
        };
        self.push_log(&message);
        Outcome {
            command: cmd.to_string(),
            ok,
            message,
            depth: self.depth,
            turn: self.turn,
        }
    }

    fn explore(&mut self) -> (bool, String) {
        let stairs_known_before = self.map.is_seen(self.stairs.0, self.stairs.1);
        let mut steps = 0;
        loop {
            if steps >= EXPLORE_STEP_LIMIT {
                return (true, format!("{steps}歩探索した。(上限)"));
            }
            let path = self.find_path(&|p| self.is_frontier(p));
            let Some(path) = path else {
                return if steps == 0 {
                    (true, "もう探索する場所がない。".to_string())
                } else {
                    (true, format!("{steps}歩探索して、探索し尽くした。"))
                };
            };
            self.step_to(path[0]);
            steps += 1;
            if !stairs_known_before && self.map.is_seen(self.stairs.0, self.stairs.1) {
                return (true, format!("{steps}歩探索して、階段を見つけた。"));
            }
        }
    }

    fn describe_stairs(&self) -> String {
        if !self.map.is_seen(self.stairs.0, self.stairs.1) {
            return "階段はまだ見つけていない。".to_string();
        }
        let dx = self.stairs.0 - self.pos.0;
        let dy = self.stairs.1 - self.pos.1;
        if dx == 0 && dy == 0 {
            return "階段の上にいる。".to_string();
        }
        let mut parts = Vec::new();
        if dx != 0 {
            parts.push(format!("{}に{}", if dx > 0 { "東" } else { "西" }, dx.abs()));
        }
        if dy != 0 {
            parts.push(format!("{}に{}", if dy > 0 { "南" } else { "北" }, dy.abs()));
        }
        format!("階段は{}の位置にある。", parts.join("、"))
    }

    pub fn cell(&self, x: i32, y: i32) -> Cell {
        if (x, y) == self.pos {
            return Cell {
                ch: '@',
                visible: true,
                seen: true,
            };
        }
        let seen = self.map.is_seen(x, y);
        Cell {
            ch: if seen { self.map.tile(x, y).glyph() } else { ' ' },
            visible: self.map.is_visible(x, y),
            seen,
        }
    }

    pub fn map_lines(&self) -> Vec<String> {
        (0..H)
            .map(|y| (0..W).map(|x| self.cell(x, y).ch).collect())
            .collect()
    }

    /// エージェント向けのテキスト観測。ログは直近 `log_lines` 件。
    pub fn observe_text(&self, log_lines: usize) -> String {
        let mut s = format!(
            "== 地下{}階 / ターン{} / 位置({},{}) ==\n",
            self.depth, self.turn, self.pos.0, self.pos.1
        );
        for line in self.map_lines() {
            s.push_str(line.trim_end());
            s.push('\n');
        }
        s.push_str("-- ログ --\n");
        let start = self.log.len().saturating_sub(log_lines);
        for e in &self.log[start..] {
            s.push_str(&format!("[{}] {}\n", e.turn, e.text));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_world() {
        let a = Game::new(42);
        let b = Game::new(42);
        assert_eq!(a.pos(), b.pos());
        assert_eq!(a.map_lines(), b.map_lines());
    }

    #[test]
    fn wall_costs_no_turn() {
        let mut g = Game::new(1);
        // 壁にぶつかるまで西へ進む
        for _ in 0..100 {
            let o = g.run("move west");
            if !o.ok {
                let t = g.turn();
                assert!(!g.run("move west").ok);
                assert_eq!(g.turn(), t);
                return;
            }
        }
        panic!("壁にぶつからなかった");
    }

    #[test]
    fn explore_finds_stairs_then_descend() {
        for seed in 0..30 {
            let mut g = Game::new(seed);
            let mut found = false;
            for _ in 0..50 {
                let o = g.run("explore");
                assert!(o.ok);
                if o.message.contains("階段を見つけた") {
                    found = true;
                    break;
                }
                if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                    break;
                }
            }
            assert!(found, "seed {seed}: 階段が見つからなかった");
            let outs = g.run_script("travel >; descend");
            assert!(outs.iter().all(|o| o.ok), "seed {seed}: {:?}", outs.last().map(|o| &o.message));
            assert_eq!(g.depth(), 2);
        }
    }

    #[test]
    fn script_stops_on_failure() {
        let mut g = Game::new(3);
        let outs = g.run_script("descend; wait");
        assert_eq!(outs.len(), 1);
        assert!(!outs[0].ok);
    }
}
