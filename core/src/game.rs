use std::collections::VecDeque;

use crate::command::{self, Command, Dir, TravelTarget};
use crate::item::{ItemKind, POTION_LOOKS, SCROLL_LOOKS};
use crate::map::{idx, Map, Tile, H, W};
use crate::rng::Rng;

const FOV_RADIUS: i32 = 9;
const EXPLORE_STEP_LIMIT: u32 = 1000;
const PLAYER_MAX_HP: i32 = 20;
/// 敵が見えていないとき、このターン数ごとにHPが1回復する。
const REGEN_INTERVAL: u32 = 10;
const MAX_MONSTERS: usize = 6;

pub struct LogEntry {
    pub turn: u32,
    pub text: String,
}

/// 1コマンドの実行結果。
pub struct Outcome {
    pub command: String,
    pub ok: bool,
    /// プレイヤーの行動の結果に、そのターンの敵の行動（被ダメージなど）を続けたもの
    pub message: String,
    /// 実行直後の階・ターン・HP
    pub depth: u32,
    pub turn: u32,
    pub hp: i32,
}

/// 描画用の1マス。
pub struct Cell {
    pub ch: char,
    /// 今見えている
    pub visible: bool,
    /// 一度でも見た（記憶している）
    pub seen: bool,
}

/// 今見えている敵。
pub struct EnemyView {
    pub name: &'static str,
    pub glyph: char,
    pub hp: i32,
    pub max_hp: i32,
    pub pos: (i32, i32),
}

/// 持ち物の1スタック（同じ種類は重なる）。文字は拾った時に決まり、使い切るまで変わらない。
struct Stack {
    letter: char,
    kind: ItemKind,
    count: u32,
}

struct Monster {
    name: &'static str,
    glyph: char,
    hp: i32,
    max_hp: i32,
    pos: (i32, i32),
}

pub struct Game {
    seed: u64,
    rng: Rng,
    depth: u32,
    turn: u32,
    pos: (i32, i32),
    hp: i32,
    max_hp: i32,
    dead: bool,
    stairs: (i32, i32),
    map: Map,
    monsters: Vec<Monster>,
    floor_items: Vec<((i32, i32), ItemKind)>,
    inventory: Vec<Stack>,
    /// 種類ごとの見た目（未識別名）。ゲームごとにシャッフルされる。
    looks: [&'static str; 6],
    /// 種類ごとに、正体を知っているか
    known: [bool; 6],
    /// 眠りなど、このコマンドのあとに追加で経過するターン
    extra_turns: u32,
    log: Vec<LogEntry>,
    /// 実行中のコマンドで起きた出来事（Outcome に添える）
    events: Vec<String>,
}

fn shuffle<T>(rng: &mut Rng, v: &mut [T]) {
    for i in (1..v.len()).rev() {
        let j = rng.range(0, (i + 1) as i32) as usize;
        v.swap(i, j);
    }
}

fn roll_looks(rng: &mut Rng) -> [&'static str; 6] {
    let mut potions = POTION_LOOKS;
    let mut scrolls = SCROLL_LOOKS;
    shuffle(rng, &mut potions);
    shuffle(rng, &mut scrolls);
    let mut looks = [""; 6];
    let (mut pi, mut si) = (0, 0);
    for k in ItemKind::ALL {
        if k.is_potion() {
            looks[k.index()] = potions[pi];
            pi += 1;
        } else {
            looks[k.index()] = scrolls[si];
            si += 1;
        }
    }
    looks
}

fn rel_text(from: (i32, i32), to: (i32, i32)) -> String {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let mut parts = Vec::new();
    if dx != 0 {
        parts.push(format!("{}に{}", if dx > 0 { "東" } else { "西" }, dx.abs()));
    }
    if dy != 0 {
        parts.push(format!("{}に{}", if dy > 0 { "南" } else { "北" }, dy.abs()));
    }
    parts.join("、")
}

impl Game {
    pub fn new(seed: u64) -> Game {
        let mut rng = Rng::new(seed);
        let g = Map::generate(&mut rng);
        let looks = roll_looks(&mut rng);
        let mut game = Game {
            seed,
            rng,
            depth: 1,
            turn: 0,
            pos: g.start,
            hp: PLAYER_MAX_HP,
            max_hp: PLAYER_MAX_HP,
            dead: false,
            stairs: g.stairs,
            map: g.map,
            monsters: Vec::new(),
            floor_items: Vec::new(),
            inventory: Vec::new(),
            looks,
            known: [false; 6],
            extra_turns: 0,
            log: Vec::new(),
            events: Vec::new(),
        };
        game.spawn_monsters();
        game.spawn_items();
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
    pub fn hp(&self) -> i32 {
        self.hp
    }
    pub fn max_hp(&self) -> i32 {
        self.max_hp
    }
    pub fn is_dead(&self) -> bool {
        self.dead
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

    /// ログに残し、Outcome にも添える。
    fn note(&mut self, text: &str) {
        self.push_log(text);
        self.events.push(text.to_string());
    }

    fn new_level(&mut self) {
        let g = Map::generate(&mut self.rng);
        self.map = g.map;
        self.pos = g.start;
        self.stairs = g.stairs;
        self.spawn_monsters();
        self.spawn_items();
        self.map.update_fov(self.pos, FOV_RADIUS);
    }

    fn spawn_items(&mut self) {
        self.floor_items.clear();
        let want = 3 + if self.depth >= 3 { 1 } else { 0 };
        let total: u32 = ItemKind::ALL.iter().map(|k| k.weight()).sum();
        for _ in 0..300 {
            if self.floor_items.len() >= want {
                break;
            }
            let x = self.rng.range(1, W - 1);
            let y = self.rng.range(1, H - 1);
            if self.map.tile(x, y) != Tile::Floor
                || (x, y) == self.pos
                || self.item_at((x, y)).is_some()
                || self.monster_at((x, y)).is_some()
            {
                continue;
            }
            let mut roll = self.rng.range(0, total as i32) as u32;
            let mut kind = ItemKind::Healing;
            for k in ItemKind::ALL {
                if roll < k.weight() {
                    kind = k;
                    break;
                }
                roll -= k.weight();
            }
            self.floor_items.push(((x, y), kind));
        }
    }

    fn item_at(&self, p: (i32, i32)) -> Option<ItemKind> {
        self.floor_items.iter().find(|(q, _)| *q == p).map(|(_, k)| *k)
    }

    /// 持ち物や床では、正体を知っていれば本当の名前、知らなければ見た目の名前。
    fn display_name(&self, kind: ItemKind) -> &'static str {
        if self.known[kind.index()] {
            kind.true_name()
        } else {
            self.looks[kind.index()]
        }
    }

    /// 持ち物に加える。割り当てた文字を返す。
    fn take(&mut self, kind: ItemKind) -> Option<char> {
        if let Some(s) = self.inventory.iter_mut().find(|s| s.kind == kind) {
            s.count += 1;
            return Some(s.letter);
        }
        let letter = ('a'..='z').find(|c| !self.inventory.iter().any(|s| s.letter == *c))?;
        self.inventory.push(Stack {
            letter,
            kind,
            count: 1,
        });
        self.inventory.sort_by_key(|s| s.letter);
        Some(letter)
    }

    /// 足元のアイテムを拾う。
    fn pickup_here(&mut self) {
        let Some(j) = self.floor_items.iter().position(|(p, _)| *p == self.pos) else {
            return;
        };
        let kind = self.floor_items[j].1;
        if let Some(letter) = self.take(kind) {
            self.floor_items.remove(j);
            let msg = format!("{}を拾った。({letter})", self.display_name(kind));
            self.note(&msg);
        }
    }

    /// 既知の場所にあるアイテム（explore が拾いに行く）。
    fn wants_item_at(&self, p: (i32, i32)) -> bool {
        self.map.is_seen(p.0, p.1) && self.item_at(p).is_some()
    }

    pub fn inventory_lines(&self) -> Vec<String> {
        self.inventory
            .iter()
            .map(|s| {
                let mut line = format!("{}) {}", s.letter, self.display_name(s.kind));
                if s.count > 1 {
                    line.push_str(&format!(" x{}", s.count));
                }
                if !self.known[s.kind.index()] {
                    line.push_str(" (未識別)");
                }
                line
            })
            .collect()
    }

    /// 持ち物を使う。(成功か, メッセージ, 1ターン消費するか)
    fn use_item(&mut self, letter: char, target: Option<char>) -> (bool, String, bool) {
        let Some(si) = self.inventory.iter().position(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let kind = self.inventory[si].kind;
        let k = kind.index();
        let was_known = self.known[k];
        let verb = if kind.is_potion() { "飲んだ" } else { "読んだ" };
        let prefix = if was_known {
            format!("{}を{verb}。", kind.true_name())
        } else {
            format!(
                "{}を{verb}。これは{}だった！",
                self.looks[k],
                kind.true_name()
            )
        };

        let body = match kind {
            ItemKind::Healing => {
                let gained = (self.max_hp - self.hp).min(10);
                self.hp += gained;
                format!("HPが{gained}回復した。(HP {}/{})", self.hp, self.max_hp)
            }
            ItemKind::Poison => {
                self.hp -= 5;
                let mut s = format!(
                    "5のダメージを受けた。(HP {}/{})",
                    self.hp.max(0),
                    self.max_hp
                );
                if self.hp <= 0 {
                    self.dead = true;
                    s.push_str(" 毒で力尽きた…。ゲームオーバー。");
                }
                s
            }
            ItemKind::Sleep => {
                self.extra_turns = 4;
                "ぐっすり眠ってしまった…。".to_string()
            }
            ItemKind::MagicMap => {
                self.map.reveal_all();
                "このフロアの地図が頭に浮かんだ。".to_string()
            }
            ItemKind::Teleport => {
                let old = self.pos;
                for _ in 0..200 {
                    let x = self.rng.range(1, W - 1);
                    let y = self.rng.range(1, H - 1);
                    if self.map.tile(x, y) == Tile::Floor
                        && (x, y) != old
                        && self.monster_at((x, y)).is_none()
                    {
                        self.pos = (x, y);
                        break;
                    }
                }
                self.map.update_fov(self.pos, FOV_RADIUS);
                "景色が一変した。".to_string()
            }
            ItemKind::Identify => {
                let ti = match target {
                    Some(t) if t == letter => {
                        return (false, "その巻物自身は対象にできない。".to_string(), false)
                    }
                    Some(t) => match self.inventory.iter().position(|s| s.letter == t) {
                        Some(i) if self.known[self.inventory[i].kind.index()] => {
                            return (false, format!("{t} はすでに識別済みだ。"), false)
                        }
                        Some(i) => Some(i),
                        None => return (false, format!("持ち物 {t} はない。"), false),
                    },
                    None => self
                        .inventory
                        .iter()
                        .position(|s| s.letter != letter && !self.known[s.kind.index()]),
                };
                match ti {
                    Some(i) => {
                        let tk = self.inventory[i].kind;
                        let old = self.looks[tk.index()];
                        self.known[tk.index()] = true;
                        format!("{old}は{}だと分かった。", tk.true_name())
                    }
                    None if was_known => {
                        return (false, "識別できるものがない。".to_string(), false)
                    }
                    None => "何も起こらなかった。".to_string(),
                }
            }
        };

        self.known[k] = true;
        self.inventory[si].count -= 1;
        if self.inventory[si].count == 0 {
            self.inventory.remove(si);
        }
        (true, format!("{prefix} {body}"), true)
    }

    fn spawn_monsters(&mut self) {
        self.monsters.clear();
        let want = ((2 + self.depth) as usize).min(MAX_MONSTERS);
        for _ in 0..300 {
            if self.monsters.len() >= want {
                break;
            }
            let x = self.rng.range(1, W - 1);
            let y = self.rng.range(1, H - 1);
            let (dx, dy) = (x - self.pos.0, y - self.pos.1);
            // 開始位置から離れた床にだけ置く
            if self.map.tile(x, y) != Tile::Floor
                || dx * dx + dy * dy < 64
                || self.monster_at((x, y)).is_some()
            {
                continue;
            }
            let hp = 4 + 2 * self.depth as i32;
            self.monsters.push(Monster {
                name: "スライム",
                glyph: 's',
                hp,
                max_hp: hp,
                pos: (x, y),
            });
        }
    }

    fn monster_at(&self, p: (i32, i32)) -> Option<usize> {
        self.monsters.iter().position(|m| m.pos == p)
    }

    fn visible_monster_indices(&self) -> Vec<usize> {
        (0..self.monsters.len())
            .filter(|&i| {
                let p = self.monsters[i].pos;
                self.map.is_visible(p.0, p.1)
            })
            .collect()
    }

    pub fn visible_enemies(&self) -> Vec<EnemyView> {
        self.visible_monster_indices()
            .into_iter()
            .map(|i| {
                let m = &self.monsters[i];
                EnemyView {
                    name: m.name,
                    glyph: m.glyph,
                    hp: m.hp,
                    max_hp: m.max_hp,
                    pos: m.pos,
                }
            })
            .collect()
    }

    /// 敵が見えていて自動移動できないなら、その理由。
    fn refuse_if_enemies(&self) -> Option<String> {
        let idxs = self.visible_monster_indices();
        if idxs.is_empty() {
            return None;
        }
        let names: Vec<&str> = idxs.iter().map(|&i| self.monsters[i].name).collect();
        Some(format!(
            "敵が見えている({})。先に倒すか、手動で動こう。",
            names.join("、")
        ))
    }

    /// 1ターン進める。敵が動き、HPが自然回復する。
    fn pass_turn(&mut self) {
        self.turn += 1;
        if self.turn % REGEN_INTERVAL == 0
            && self.hp < self.max_hp
            && self.visible_monster_indices().is_empty()
        {
            self.hp += 1;
        }
        self.monsters_act();
    }

    fn monsters_act(&mut self) {
        for i in 0..self.monsters.len() {
            if self.dead {
                break;
            }
            let (mpos, name) = (self.monsters[i].pos, self.monsters[i].name);
            let (dx, dy) = (self.pos.0 - mpos.0, self.pos.1 - mpos.1);
            // こちらから見えている間だけ追いかけてくる
            // (視線判定は向きによって結果が違うことがあるので、プレイヤーの視界に合わせる)
            if !self.map.is_visible(mpos.0, mpos.1) {
                continue;
            }
            if dx.abs() <= 1 && dy.abs() <= 1 {
                let dmg = self.rng.range(1, 3 + (self.depth as i32 - 1) / 2);
                self.hp -= dmg;
                let msg = format!(
                    "{name}の攻撃！ {dmg}のダメージを受けた。(HP {}/{})",
                    self.hp.max(0),
                    self.max_hp
                );
                self.note(&msg);
                if self.hp <= 0 {
                    self.dead = true;
                    self.note("あなたは力尽きた…。ゲームオーバー。");
                }
            } else if let Some(np) = self.monster_step(mpos) {
                self.monsters[i].pos = np;
            }
        }
    }

    /// 敵が `from` からプレイヤーの隣まで近づくための最初の1歩（床の形を考えた最短経路）。
    fn monster_step(&self, from: (i32, i32)) -> Option<(i32, i32)> {
        let n = (W * H) as usize;
        let mut prev = vec![usize::MAX; n];
        let start = idx(from.0, from.1);
        prev[start] = start;
        let mut q = VecDeque::new();
        q.push_back(from);
        while let Some(p) = q.pop_front() {
            if p != from && (p.0 - self.pos.0).abs() <= 1 && (p.1 - self.pos.1).abs() <= 1 {
                let mut c = idx(p.0, p.1);
                while prev[c] != start {
                    c = prev[c];
                }
                return Some(((c as i32) % W, (c as i32) / W));
            }
            for d in Dir::ALL {
                let (dx, dy) = d.delta();
                let np = (p.0 + dx, p.1 + dy);
                if !self.map.tile(np.0, np.1).walkable()
                    || np == self.pos
                    || self.monster_at(np).is_some()
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

    fn attack_monster(&mut self, i: usize) -> String {
        let dmg = self.rng.range(2, 5);
        self.monsters[i].hp -= dmg;
        let (name, hp, max_hp) = {
            let m = &self.monsters[i];
            (m.name, m.hp, m.max_hp)
        };
        if hp <= 0 {
            self.monsters.remove(i);
            format!("{name}に{dmg}のダメージ。{name}を倒した！")
        } else {
            format!("{name}に{dmg}のダメージを与えた。(HP {hp}/{max_hp})")
        }
    }

    /// 1歩移動して1ターン進める（自動移動用）。
    fn step_to(&mut self, p: (i32, i32)) {
        self.pos = p;
        self.map.update_fov(self.pos, FOV_RADIUS);
        self.pickup_here();
        self.pass_turn();
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
            Err(msg) => self.outcome(line.trim().to_string(), false, msg),
        }
    }

    fn outcome(&self, command: String, ok: bool, message: String) -> Outcome {
        Outcome {
            command,
            ok,
            message,
            depth: self.depth,
            turn: self.turn,
            hp: self.hp,
        }
    }

    pub fn exec(&mut self, cmd: Command) -> Outcome {
        self.events.clear();
        if self.dead {
            return self.outcome(
                cmd.to_string(),
                false,
                "ゲームオーバー。new_game でやり直せる。".to_string(),
            );
        }
        let pos_before = self.pos;
        // (成功か, メッセージ, このコマンド自身が1ターン消費するか)
        let (ok, mut message, spent) = match cmd {
            Command::Move(d) => {
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                if let Some(i) = self.monster_at(t) {
                    (true, self.attack_monster(i), true)
                } else if self.map.tile(t.0, t.1).walkable() {
                    self.pos = t;
                    self.map.update_fov(self.pos, FOV_RADIUS);
                    if self.pos == self.stairs {
                        (true, "階段の上にいる。(descend で降りられる)".to_string(), true)
                    } else {
                        (true, format!("{}へ進んだ。", d.name()), true)
                    }
                } else {
                    (false, "壁にぶつかった。".to_string(), false)
                }
            }
            Command::Attack(d) => {
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                match self.monster_at(t) {
                    Some(i) => (true, self.attack_monster(i), true),
                    None => (false, "そこには何もいない。".to_string(), false),
                }
            }
            Command::Descend => {
                if self.pos == self.stairs {
                    self.depth += 1;
                    self.new_level();
                    (true, format!("地下{}階に降りた。", self.depth), true)
                } else {
                    (false, "ここに階段はない。".to_string(), false)
                }
            }
            Command::Wait => (true, "1ターン待った。".to_string(), true),
            Command::Use(letter, target) => self.use_item(letter, target),
            Command::Inventory => {
                let lines = self.inventory_lines();
                let msg = if lines.is_empty() {
                    "持ち物はない。".to_string()
                } else {
                    format!("持ち物: {}", lines.join(" / "))
                };
                (true, msg, false)
            }
            Command::Look => (true, self.describe_surroundings(), false),
            Command::Travel(TravelTarget::Stairs) => {
                let (ok, msg) = self.travel_to_stairs();
                (ok, msg, false)
            }
            Command::Explore => {
                let (ok, msg) = self.explore();
                (ok, msg, false)
            }
        };
        self.push_log(&message);
        // 歩いたり転移したりして着いた場所のアイテムを拾う
        if self.pos != pos_before && !self.dead {
            self.pickup_here();
        }
        if spent {
            self.pass_turn();
        }
        let slept = self.extra_turns > 0;
        while self.extra_turns > 0 && !self.dead {
            self.extra_turns -= 1;
            self.pass_turn();
        }
        self.extra_turns = 0;
        if slept && !self.dead {
            self.note("目が覚めた。");
        }
        if !self.events.is_empty() {
            message = format!("{message} {}", self.events.join(" "));
        }
        self.outcome(cmd.to_string(), ok, message)
    }

    fn travel_to_stairs(&mut self) -> (bool, String) {
        if self.pos == self.stairs {
            return (true, "すでに階段の上にいる。".to_string());
        }
        if !self.map.is_seen(self.stairs.0, self.stairs.1) {
            return (false, "階段の場所をまだ知らない。explore で探そう。".to_string());
        }
        if let Some(m) = self.refuse_if_enemies() {
            return (false, m);
        }
        let stairs = self.stairs;
        let Some(path) = self.find_path(&|p| p == stairs) else {
            return (false, "階段までの道がつながっていない。".to_string());
        };
        let hp0 = self.hp;
        let mut n = 0;
        for p in path {
            if let Some(i) = self.monster_at(p) {
                return (
                    true,
                    format!("階段へ向かう途中({n}歩)、進路上に{}がいる。", self.monsters[i].name),
                );
            }
            self.step_to(p);
            n += 1;
            if let Some(why) = self.interruption(hp0) {
                return (true, format!("階段へ向かう途中({n}歩)、{why}"));
            }
        }
        (true, format!("階段まで{n}歩移動した。"))
    }

    /// 自動移動を止めるべき事情（死亡・被弾・新たな敵の出現）。
    fn interruption(&self, hp0: i32) -> Option<String> {
        if self.dead {
            return Some("力尽きた。".to_string());
        }
        if self.hp < hp0 {
            return Some("攻撃を受けて中断した。".to_string());
        }
        self.visible_monster_indices()
            .first()
            .map(|&i| format!("{}が現れて中断した。", self.monsters[i].name))
    }

    fn explore(&mut self) -> (bool, String) {
        if let Some(m) = self.refuse_if_enemies() {
            return (false, m);
        }
        let stairs_known_before = self.map.is_seen(self.stairs.0, self.stairs.1);
        let hp0 = self.hp;
        let mut steps = 0;
        loop {
            if steps >= EXPLORE_STEP_LIMIT {
                return (true, format!("{steps}歩探索した。(上限)"));
            }
            let path = self.find_path(&|p| self.is_frontier(p) || self.wants_item_at(p));
            let Some(path) = path else {
                return if steps == 0 {
                    (true, "もう探索する場所がない。".to_string())
                } else {
                    (true, format!("{steps}歩探索して、探索し尽くした。"))
                };
            };
            if let Some(i) = self.monster_at(path[0]) {
                return (
                    true,
                    format!("{steps}歩探索したが、進路上に{}がいる。", self.monsters[i].name),
                );
            }
            self.step_to(path[0]);
            steps += 1;
            if let Some(why) = self.interruption(hp0) {
                return (true, format!("{steps}歩探索したところで、{why}"));
            }
            if !stairs_known_before && self.map.is_seen(self.stairs.0, self.stairs.1) {
                return (true, format!("{steps}歩探索して、階段を見つけた。"));
            }
        }
    }

    fn describe_surroundings(&self) -> String {
        let mut parts = Vec::new();
        if !self.map.is_seen(self.stairs.0, self.stairs.1) {
            parts.push("階段はまだ見つけていない。".to_string());
        } else if self.pos == self.stairs {
            parts.push("階段の上にいる。".to_string());
        } else {
            parts.push(format!(
                "階段は{}の位置にある。",
                rel_text(self.pos, self.stairs)
            ));
        }
        for e in self.visible_enemies() {
            parts.push(format!(
                "{}(HP {}/{})が{}にいる。",
                e.name,
                e.hp,
                e.max_hp,
                rel_text(self.pos, e.pos)
            ));
        }
        for (p, k) in &self.floor_items {
            if self.map.is_seen(p.0, p.1) {
                parts.push(format!(
                    "{} {}が{}にある。",
                    k.glyph(),
                    self.display_name(*k),
                    rel_text(self.pos, *p)
                ));
            }
        }
        parts.join(" ")
    }

    pub fn cell(&self, x: i32, y: i32) -> Cell {
        if (x, y) == self.pos {
            return Cell {
                ch: '@',
                visible: true,
                seen: true,
            };
        }
        if self.map.is_visible(x, y) {
            if let Some(i) = self.monster_at((x, y)) {
                return Cell {
                    ch: self.monsters[i].glyph,
                    visible: true,
                    seen: true,
                };
            }
        }
        let seen = self.map.is_seen(x, y);
        if seen {
            if let Some(k) = self.item_at((x, y)) {
                return Cell {
                    ch: k.glyph(),
                    visible: self.map.is_visible(x, y),
                    seen,
                };
            }
        }
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
            "== 地下{}階 / ターン{} / HP {}/{} / 位置({},{}) ==\n",
            self.depth, self.turn, self.hp, self.max_hp, self.pos.0, self.pos.1
        );
        for line in self.map_lines() {
            s.push_str(line.trim_end());
            s.push('\n');
        }
        let enemies = self.visible_enemies();
        if !enemies.is_empty() {
            s.push_str("-- 見えている敵 --\n");
            for e in enemies {
                s.push_str(&format!(
                    "{} {} HP {}/{} ({})\n",
                    e.glyph,
                    e.name,
                    e.hp,
                    e.max_hp,
                    rel_text(self.pos, e.pos)
                ));
            }
        }
        s.push_str("-- 持ち物 --\n");
        let inv = self.inventory_lines();
        if inv.is_empty() {
            s.push_str("(なし)\n");
        }
        for l in inv {
            s.push_str(&l);
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

    fn slime(pos: (i32, i32), hp: i32) -> Monster {
        Monster {
            name: "スライム",
            glyph: 's',
            hp,
            max_hp: hp,
            pos,
        }
    }

    /// 開始位置の東隣にスライムを置く（開始位置は部屋の中央なので必ず歩ける）。
    fn with_adjacent_slime(seed: u64, hp: i32) -> Game {
        let mut g = Game::new(seed);
        g.monsters.clear();
        let p = (g.pos.0 + 1, g.pos.1);
        assert!(g.map.tile(p.0, p.1).walkable());
        g.monsters.push(slime(p, hp));
        g
    }

    #[test]
    fn same_seed_same_world() {
        let a = Game::new(42);
        let b = Game::new(42);
        assert_eq!(a.pos(), b.pos());
        assert_eq!(a.map_lines(), b.map_lines());
        assert_eq!(a.monsters.len(), b.monsters.len());
    }

    #[test]
    fn monsters_spawn_away_from_start() {
        for seed in 0..20 {
            let g = Game::new(seed);
            assert!(!g.monsters.is_empty());
            for m in &g.monsters {
                let (dx, dy) = (m.pos.0 - g.pos.0, m.pos.1 - g.pos.1);
                assert!(dx * dx + dy * dy >= 64, "seed {seed}");
            }
        }
    }

    #[test]
    fn wall_costs_no_turn() {
        let mut g = Game::new(1);
        g.monsters.clear();
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
            g.monsters.clear();
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
            assert!(
                outs.iter().all(|o| o.ok),
                "seed {seed}: {:?}",
                outs.last().map(|o| &o.message)
            );
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

    #[test]
    fn bump_attack_kills_and_the_slime_hits_back() {
        let mut g = with_adjacent_slime(1, 5);
        // 5 HP に対して与えるダメージは 2〜4 なので、最初の一撃では倒れず反撃される
        let first = g.run("move east");
        assert!(first.ok);
        assert!(first.message.contains("ダメージを与えた"));
        assert!(first.message.contains("攻撃！"));
        assert!(g.hp() < g.max_hp());
        for _ in 0..10 {
            if g.monsters.is_empty() {
                break;
            }
            assert!(g.run("attack east").ok);
        }
        assert!(g.monsters.is_empty());
        assert!(!g.is_dead());
    }

    #[test]
    fn a_visible_slime_always_closes_in_and_attacks() {
        let mut checked = 0;
        for seed in 0..15 {
            let mut g = quiet(seed);
            // 見えていて、3マス離れた歩ける場所を探す
            let spot = (-3..=3)
                .flat_map(|dy| (-3..=3).map(move |dx| (dx, dy)))
                .filter(|(dx, dy): &(i32, i32)| dx.abs().max(dy.abs()) == 3)
                .map(|(dx, dy)| (g.pos.0 + dx, g.pos.1 + dy))
                .find(|p| g.map.tile(p.0, p.1).walkable() && g.map.is_visible(p.0, p.1));
            let Some(spot) = spot else { continue };
            g.monsters.push(slime(spot, 50));
            for _ in 0..6 {
                g.run("wait");
            }
            assert!(g.hp() < g.max_hp(), "seed {seed}: 近づいてこなかった");
            checked += 1;
        }
        assert!(checked >= 5);
    }

    #[test]
    fn attack_needs_a_target() {
        let mut g = Game::new(1);
        g.monsters.clear();
        let t = g.turn();
        let o = g.run("attack east");
        assert!(!o.ok);
        assert_eq!(g.turn(), t);
    }

    #[test]
    fn death_ends_the_game() {
        let mut g = with_adjacent_slime(1, 5);
        g.hp = 1;
        let o = g.run("wait");
        assert!(g.is_dead());
        assert!(o.message.contains("力尽きた"));
        let o = g.run("wait");
        assert!(!o.ok);
        assert!(o.message.contains("ゲームオーバー"));
    }

    #[test]
    fn explore_and_travel_refuse_while_an_enemy_is_visible() {
        let mut g = with_adjacent_slime(1, 5);
        let o = g.run("explore");
        assert!(!o.ok);
        assert!(o.message.contains("敵が見えている"));
        let o = g.run("travel >");
        assert!(!o.ok);
    }

    #[test]
    fn hp_regenerates_when_no_enemy_is_around() {
        let mut g = Game::new(1);
        g.monsters.clear();
        g.hp = 10;
        for _ in 0..REGEN_INTERVAL {
            g.run("wait");
        }
        assert_eq!(g.hp(), 11);
    }

    #[test]
    fn observation_lists_visible_enemies() {
        let g = with_adjacent_slime(1, 5);
        let text = g.observe_text(5);
        assert!(text.contains("-- 見えている敵 --"));
        assert!(text.contains("スライム HP 5/5 (東に1)"));
        assert!(text.contains(&format!("HP {}/{}", g.hp(), g.max_hp())));
    }

    /// 敵もアイテムもいない状態のゲーム。
    fn quiet(seed: u64) -> Game {
        let mut g = Game::new(seed);
        g.monsters.clear();
        g.floor_items.clear();
        g
    }

    #[test]
    fn looks_are_unique_and_stable_per_seed() {
        let a = Game::new(9);
        let b = Game::new(9);
        assert_eq!(a.looks, b.looks);
        assert!(a.known.iter().all(|k| !k));
        for pot in [true, false] {
            let names: Vec<_> = ItemKind::ALL
                .iter()
                .filter(|k| k.is_potion() == pot)
                .map(|k| a.looks[k.index()])
                .collect();
            for (i, x) in names.iter().enumerate() {
                assert!(!x.is_empty());
                for y in &names[i + 1..] {
                    assert_ne!(x, y);
                }
            }
        }
        // ゲームによって対応が変わる
        let differs = (0..20).any(|seed| Game::new(seed).looks != a.looks);
        assert!(differs);
    }

    #[test]
    fn items_spawn_on_floor_tiles() {
        for seed in 0..20 {
            let g = Game::new(seed);
            assert!(!g.floor_items.is_empty());
            for (p, _) in &g.floor_items {
                assert_eq!(g.map.tile(p.0, p.1), Tile::Floor);
                assert_ne!(*p, g.pos);
            }
        }
    }

    #[test]
    fn walking_onto_an_item_picks_it_up() {
        let mut g = quiet(1);
        let p = (g.pos.0 + 1, g.pos.1);
        g.floor_items.push((p, ItemKind::Healing));
        let o = g.run("move east");
        assert!(o.ok && o.message.contains("拾った"), "{}", o.message);
        assert!(g.floor_items.is_empty());
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.inventory[0].letter, 'a');
        assert_eq!(g.inventory[0].kind, ItemKind::Healing);
    }

    #[test]
    fn same_kind_stacks_and_letters_are_stable() {
        let mut g = quiet(1);
        assert_eq!(g.take(ItemKind::Poison), Some('a'));
        assert_eq!(g.take(ItemKind::Healing), Some('b'));
        assert_eq!(g.take(ItemKind::Poison), Some('a'));
        assert_eq!(g.inventory[0].count, 2);
        // a を使い切っても b の文字は変わらない
        g.known[ItemKind::Poison.index()] = true;
        g.hp = 20;
        g.run("use a");
        g.run("use a");
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.inventory[0].letter, 'b');
    }

    #[test]
    fn healing_potion_heals_and_identifies() {
        let mut g = quiet(1);
        g.take(ItemKind::Healing);
        g.take(ItemKind::Healing);
        g.hp = 5;
        let o = g.run("use a");
        assert!(o.ok);
        assert_eq!(g.hp(), 15);
        assert!(g.known[ItemKind::Healing.index()]);
        assert!(o.message.contains("回復の薬だった"), "{}", o.message);
        let o = g.run("use a");
        assert!(o.ok);
        assert!(!o.message.contains("だった！"));
        assert_eq!(g.hp(), 20);
        assert!(g.inventory.is_empty());
        assert!(!g.run("use a").ok);
    }

    #[test]
    fn poison_hurts_and_can_kill() {
        let mut g = quiet(1);
        g.take(ItemKind::Poison);
        let o = g.run("use a");
        assert!(o.ok);
        assert_eq!(g.hp(), 15);

        let mut g = quiet(1);
        g.take(ItemKind::Poison);
        g.hp = 5;
        let o = g.run("use a");
        assert!(g.is_dead());
        assert!(o.message.contains("ゲームオーバー"));
    }

    #[test]
    fn sleeping_passes_turns_while_an_enemy_attacks() {
        let mut g = with_adjacent_slime(1, 50);
        g.floor_items.clear();
        g.take(ItemKind::Sleep);
        let t = g.turn();
        let o = g.run("use a");
        assert!(o.ok);
        assert_eq!(g.turn(), t + 5);
        assert!(g.hp() < g.max_hp());
        assert!(o.message.contains("目が覚めた"), "{}", o.message);
    }

    #[test]
    fn identify_scroll_reveals_another_item() {
        let mut g = quiet(1);
        g.take(ItemKind::Healing); // a
        g.take(ItemKind::Identify); // b
        let o = g.run("use b");
        assert!(o.ok, "{}", o.message);
        assert!(g.known[ItemKind::Healing.index()]);
        assert!(g.known[ItemKind::Identify.index()]);
        assert!(o.message.contains("回復の薬だと分かった"), "{}", o.message);
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.inventory[0].kind, ItemKind::Healing);
    }

    #[test]
    fn identify_scroll_with_an_explicit_target() {
        let mut g = quiet(1);
        g.take(ItemKind::Healing); // a
        g.take(ItemKind::Poison); // b
        g.take(ItemKind::Identify); // c
        let o = g.run("use c b");
        assert!(o.ok, "{}", o.message);
        assert!(g.known[ItemKind::Poison.index()]);
        assert!(!g.known[ItemKind::Healing.index()]);
        // すでに識別済みの対象は選べない
        g.take(ItemKind::Identify);
        let o = g.run("use c b");
        assert!(!o.ok);
    }

    #[test]
    fn identify_scroll_without_targets() {
        // 正体を知らない巻物は、読むと消費して正体だけ分かる
        let mut g = quiet(1);
        g.take(ItemKind::Identify);
        let o = g.run("use a");
        assert!(o.ok);
        assert!(o.message.contains("何も起こらなかった"));
        assert!(g.inventory.is_empty());
        // 正体を知っている巻物は、対象がなければ消費せず失敗する
        let mut g = quiet(1);
        g.take(ItemKind::Identify);
        g.known[ItemKind::Identify.index()] = true;
        let t = g.turn();
        let o = g.run("use a");
        assert!(!o.ok);
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.turn(), t);
    }

    #[test]
    fn teleport_moves_the_player() {
        let mut g = quiet(1);
        g.take(ItemKind::Teleport);
        let old = g.pos;
        assert!(g.run("use a").ok);
        assert_ne!(g.pos, old);
        assert!(g.map.tile(g.pos.0, g.pos.1).walkable());
    }

    #[test]
    fn magic_map_reveals_the_stairs() {
        let mut checked = 0;
        for seed in 0..20 {
            let mut g = quiet(seed);
            if g.map.is_seen(g.stairs.0, g.stairs.1) {
                continue;
            }
            g.take(ItemKind::MagicMap);
            assert!(g.run("use a").ok);
            assert!(g.map.is_seen(g.stairs.0, g.stairs.1));
            let o = g.run("travel >");
            assert!(o.ok, "seed {seed}: {}", o.message);
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn explore_collects_every_item_on_the_floor() {
        for seed in 0..10 {
            let mut g = Game::new(seed);
            g.monsters.clear();
            let n = g.floor_items.len();
            assert!(n > 0);
            for _ in 0..200 {
                let o = g.run("explore");
                if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                    break;
                }
            }
            assert!(g.floor_items.is_empty(), "seed {seed}");
            let total: u32 = g.inventory.iter().map(|s| s.count).sum();
            assert_eq!(total as usize, n, "seed {seed}");
        }
    }

    #[test]
    fn inventory_command_and_observation() {
        let mut g = quiet(1);
        let o = g.run("inventory");
        assert!(o.ok && o.message.contains("持ち物はない"));
        g.take(ItemKind::Healing);
        let o = g.run("inventory");
        assert!(o.message.contains("a) "));
        assert!(o.message.contains(g.looks[ItemKind::Healing.index()]));
        let text = g.observe_text(5);
        assert!(text.contains("-- 持ち物 --"));
        assert!(text.contains("(未識別)"));
    }

    #[test]
    fn look_mentions_known_floor_items() {
        let mut g = quiet(1);
        let p = (g.pos.0 + 2, g.pos.1);
        g.floor_items.push((p, ItemKind::Teleport));
        g.map.update_fov(g.pos, FOV_RADIUS);
        let o = g.run("look");
        assert!(o.message.contains("東に2"), "{}", o.message);
        assert!(g.observe_text(1).contains('?'));
    }
}
