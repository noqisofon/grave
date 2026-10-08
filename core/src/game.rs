use std::collections::VecDeque;

use crate::command::{self, Command, Dir, TravelTarget};
use crate::item::{Gear, Item, ItemKind, Suffix, MUSHROOM_LOOKS, POTION_LOOKS, SCROLL_LOOKS};
use crate::map::{idx, Map, Tile, H, W};
use crate::monster::{MonsterKind, KINDS};
use crate::rng::Rng;

const FOV_RADIUS: i32 = 9;
const EXPLORE_STEP_LIMIT: u32 = 1000;
const PLAYER_MAX_HP: i32 = 20;
/// 敵が見えていないとき、このターン数ごとにHPが1回復する。
const REGEN_INTERVAL: u32 = 10;
/// 満腹度の上限と、空腹の段階
const MAX_FOOD: i32 = 300;
const HUNGRY_AT: i32 = 100;
const WEAK_AT: i32 = 30;
/// この HP 以下で毒や飢えが続くと、自動移動を止めて知らせる
const DANGER_HP: i32 = 5;
const MAX_POISON: u32 = 15;
const MAX_MONSTERS: usize = 6;
/// 装備して、このターン数が過ぎると、その装備の正体（接尾辞と補正値）が分かる
const IDENTIFY_AFTER_WORN: u32 = 50;
/// レベルアップで増える最大HP
const HP_PER_LEVEL: i32 = 4;
/// この階の床に魔除けのアミュレットがある。ここが最深部。
pub const AMULET_DEPTH: u32 = 30;

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
/// 装備品は重ならず、`gear` に個体の情報が入る（このとき `count` は常に 1）。
struct Stack {
    letter: char,
    kind: ItemKind,
    count: u32,
    gear: Option<Gear>,
}

/// 床に落ちている物1個。`dropped` は、プレイヤーが `drop` した物の印
/// （印の付いた物は、歩いても `stay` しても自動では拾われない）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FloorItem {
    pos: (i32, i32),
    item: Item,
    dropped: bool,
}

impl FloorItem {
    /// 生成された物（印なし）。
    fn new(pos: (i32, i32), item: impl Into<Item>) -> FloorItem {
        FloorItem {
            pos,
            item: item.into(),
            dropped: false,
        }
    }
}

struct Monster {
    kind: &'static MonsterKind,
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
    /// プレイヤーのレベルと、これまでに得た経験値の合計
    level: u32,
    xp: u32,
    /// 倒した敵の経験値。メッセージを記録したあとに加算する（ログの順番のため）
    pending_xp: u32,
    dead: bool,
    stairs: (i32, i32),
    map: Map,
    monsters: Vec<Monster>,
    floor_items: Vec<FloorItem>,
    inventory: Vec<Stack>,
    /// 床にあるアミュレット（最深部だけ）
    amulet: Option<(i32, i32)>,
    /// アミュレットを持っているか。持つと階段は登り階段になる
    has_amulet: bool,
    /// アミュレットを持って地上へ脱出した
    won: bool,
    /// 種類ごとの見た目（未識別名）。ゲームごとにシャッフルされる。
    looks: [&'static str; ItemKind::COUNT],
    /// 種類ごとに、正体を知っているか
    known: [bool; ItemKind::COUNT],
    /// 装備中の武器と防具（持ち物の文字で指す）
    weapon: Option<char>,
    armor: Option<char>,
    /// 満腹度。時間とともに減り、0 になると体力が削られる
    food: i32,
    /// 毒の残りターン。1ターンごとに1ダメージ
    poison: u32,
    /// 実行中の自動移動を止めるべき出来事（被弾など）
    hit: bool,
    alert: Option<String>,
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

fn roll_looks(rng: &mut Rng) -> [&'static str; ItemKind::COUNT] {
    let mut potions = POTION_LOOKS;
    let mut scrolls = SCROLL_LOOKS;
    let mut shrooms = MUSHROOM_LOOKS;
    shuffle(rng, &mut potions);
    shuffle(rng, &mut scrolls);
    shuffle(rng, &mut shrooms);
    let mut looks = [""; ItemKind::COUNT];
    let (mut pi, mut si, mut mi) = (0, 0, 0);
    for k in ItemKind::ALL {
        if k.is_equipment() || k.is_food() {
            looks[k.index()] = k.true_name();
        } else if k.is_mushroom() {
            looks[k.index()] = shrooms[mi];
            mi += 1;
        } else if k.is_potion() {
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

/// 持ち物を消費する行動の種類（`quaff` / `eat` / `read`）。
#[derive(Clone, Copy)]
enum Consume {
    Quaff,
    Eat,
    Read,
}

impl Consume {
    fn command(self) -> &'static str {
        match self {
            Consume::Quaff => "quaff",
            Consume::Eat => "eat",
            Consume::Read => "read",
        }
    }
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
            level: 1,
            xp: 0,
            pending_xp: 0,
            dead: false,
            stairs: g.stairs,
            map: g.map,
            monsters: Vec::new(),
            floor_items: Vec::new(),
            inventory: Vec::new(),
            amulet: None,
            has_amulet: false,
            won: false,
            looks,
            known: ItemKind::ALL.map(|k| k.is_equipment() || k.is_food()),
            weapon: None,
            armor: None,
            food: MAX_FOOD - 50,
            poison: 0,
            hit: false,
            alert: None,
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
    pub fn level(&self) -> u32 {
        self.level
    }
    pub fn xp(&self) -> u32 {
        self.xp
    }
    /// 次のレベルに必要な経験値の合計（レベル2 は 10、3 は 30、4 は 60 …）
    pub fn xp_for_next(&self) -> u32 {
        5 * self.level * (self.level + 1)
    }
    pub fn is_dead(&self) -> bool {
        self.dead
    }
    pub fn is_won(&self) -> bool {
        self.won
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
        self.amulet = None;
        if self.depth == AMULET_DEPTH && !self.has_amulet {
            for _ in 0..300 {
                let x = self.rng.range(1, W - 1);
                let y = self.rng.range(1, H - 1);
                let (dx, dy) = (x - self.pos.0, y - self.pos.1);
                if self.map.tile(x, y) == Tile::Floor && dx * dx + dy * dy >= 64 {
                    self.amulet = Some((x, y));
                    break;
                }
            }
        }
        let want = 3 + if self.depth >= 3 { 1 } else { 0 };
        let kinds: Vec<ItemKind> = ItemKind::ALL
            .iter()
            .copied()
            .filter(|k| k.min_depth() <= self.depth)
            .collect();
        let total: u32 = kinds.iter().map(|k| k.weight()).sum();
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
            for k in kinds.iter().copied() {
                if roll < k.weight() {
                    kind = k;
                    break;
                }
                roll -= k.weight();
            }
            let item = Item::roll(&mut self.rng, kind, self.depth);
            self.floor_items.push(FloorItem::new((x, y), item));
        }
        // 飢え死にしないよう、どの階にも食べ物を1つは置く
        let food = if self.depth >= 2 && self.rng.range(0, 4) == 0 {
            ItemKind::Jerky
        } else {
            ItemKind::Bread
        };
        for _ in 0..300 {
            let x = self.rng.range(1, W - 1);
            let y = self.rng.range(1, H - 1);
            if self.map.tile(x, y) == Tile::Floor
                && (x, y) != self.pos
                && self.item_at((x, y)).is_none()
                && self.monster_at((x, y)).is_none()
            {
                self.floor_items.push(FloorItem::new((x, y), food));
                break;
            }
        }
    }

    /// マス `p` の物の `floor_items` 内の番号。「一番上」から順に、印のない物が先、捨てた物があと。
    fn floor_order(&self, p: (i32, i32)) -> Vec<usize> {
        let at = |dropped: bool| {
            self.floor_items
                .iter()
                .enumerate()
                .filter(move |(_, f)| f.pos == p && f.dropped == dropped)
                .map(|(i, _)| i)
        };
        at(false).chain(at(true)).collect()
    }

    /// マス `p` の一番上の物（マップの記号になる）。
    fn item_at(&self, p: (i32, i32)) -> Option<Item> {
        self.floor_order(p).first().map(|&i| self.floor_items[i].item)
    }

    /// 持ち物や床では、正体を知っていれば本当の名前、知らなければ見た目の名前。
    fn display_name(&self, kind: ItemKind) -> &'static str {
        if self.known[kind.index()] {
            kind.true_name()
        } else {
            self.looks[kind.index()]
        }
    }

    /// 品物の表示名。装備は個体の名前、それ以外は種類の名前。
    fn item_name(&self, item: &Item) -> String {
        match item {
            Item::Gear(g) => g.name(),
            Item::Plain(k) => self.display_name(*k).to_string(),
        }
    }

    fn has_free_letter(&self) -> bool {
        ('a'..='z').any(|c| !self.inventory.iter().any(|s| s.letter == c))
    }

    /// 持ち物に加えられるか（同じ種類のスタックがあるか、空き文字があるか）。装備は重ならない。
    fn can_take(&self, item: impl Into<Item>) -> bool {
        match item.into() {
            Item::Plain(kind) => self.inventory.iter().any(|s| s.kind == kind) || self.has_free_letter(),
            Item::Gear(_) => self.has_free_letter(),
        }
    }

    /// 持ち物に加える。割り当てた文字を返す。
    fn take(&mut self, item: impl Into<Item>) -> Option<char> {
        let (kind, gear) = match item.into() {
            Item::Plain(kind) => (kind, None),
            Item::Gear(g) => (g.kind, Some(g)),
        };
        if gear.is_none() {
            if let Some(s) = self.inventory.iter_mut().find(|s| s.kind == kind) {
                s.count += 1;
                return Some(s.letter);
            }
        }
        let letter = ('a'..='z').find(|c| !self.inventory.iter().any(|s| s.letter == *c))?;
        self.inventory.push(Stack {
            letter,
            kind,
            count: 1,
            gear,
        });
        self.inventory.sort_by_key(|s| s.letter);
        Some(letter)
    }

    /// 足元のアイテムを拾う。
    fn pickup_here(&mut self) {
        if self.amulet == Some(self.pos) {
            self.amulet = None;
            self.has_amulet = true;
            self.note("魔除けのアミュレットを手に入れた！ 階段は登り階段になった。地上まで持ち帰ろう。");
            self.alert = Some("アミュレットを手に入れて中断した。".to_string());
        }
        // 自動で拾うのは、印のない物の一番上の1個だけ（捨てた物は拾わない）
        let Some(j) = self.auto_pickable(self.pos) else {
            return;
        };
        match self.take_floor(j) {
            Ok(msg) | Err(msg) => self.note(&msg),
        }
    }

    /// マス `p` で自動的に拾われる物（印のない物の先頭）の `floor_items` 内の番号。
    fn auto_pickable(&self, p: (i32, i32)) -> Option<usize> {
        self.floor_order(p).first().copied().filter(|&j| !self.floor_items[j].dropped)
    }

    /// 床の物 `j` を持ち物に移す。成功ならそのメッセージ、満杯なら Err のメッセージ。
    fn take_floor(&mut self, j: usize) -> Result<String, String> {
        let item = self.floor_items[j].item;
        match self.take(item) {
            Some(letter) => {
                self.floor_items.remove(j);
                Ok(format!("{}を拾った。({letter})", self.item_name(&item)))
            }
            None => Err(format!("持ち物がいっぱいで、{}を拾えない。", self.item_name(&item))),
        }
    }

    /// `pickup [番号]`: 足元の物を1個拾う。番号は足元の一覧の番号（省くと1番）。
    fn pickup_cmd(&mut self, n: Option<u32>) -> (bool, String, bool) {
        let order = self.floor_order(self.pos);
        if order.is_empty() {
            return (false, "足元には何もない。".to_string(), false);
        }
        let n = n.unwrap_or(1) as usize;
        if n > order.len() {
            return (
                false,
                format!("足元の番号は 1〜{} だ。{}", order.len(), self.underfoot_text()),
                false,
            );
        }
        match self.take_floor(order[n - 1]) {
            Ok(msg) => (true, msg, true),
            Err(msg) => (false, msg, false),
        }
    }

    /// `drop <文字> [数]`: 持ち物を足元に捨てる。捨てた物には印が付き、自動では拾われない。
    fn drop_cmd(&mut self, letter: char, count: u32) -> (bool, String, bool) {
        let Some(si) = self.inventory.iter().position(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let stack = &self.inventory[si];
        // 装備中のものは捨てられない。呪われていれば、はずせないので同じこと
        if self.weapon == Some(letter) || self.armor == Some(letter) {
            let name = stack.gear.map_or(String::new(), |g| g.name());
            return if stack.gear.is_some_and(|g| g.is_cursed()) {
                (false, format!("{name}は呪われていて、はずせない。捨てられない。"), false)
            } else {
                (false, format!("{name}は装備中だ。先に unequip {letter} ではずそう。"), false)
            };
        }
        if count > stack.count {
            return (false, format!("{letter} は{}個しか持っていない。", stack.count), false);
        }
        let (kind, gear) = (stack.kind, stack.gear);
        let item = match gear {
            Some(g) => Item::Gear(g),
            None => Item::Plain(kind),
        };
        let name = self.item_name(&item);
        // 重なっている物は、1個ずつ床に置く
        for _ in 0..count {
            self.floor_items.push(FloorItem {
                pos: self.pos,
                item,
                dropped: true,
            });
        }
        self.inventory[si].count -= count;
        if self.inventory[si].count == 0 {
            self.inventory.remove(si);
        }
        let msg = if count == 1 {
            format!("{name}を足元に捨てた。")
        } else {
            format!("{name}を{count}個、足元に捨てた。")
        };
        (true, msg, true)
    }

    /// 足元の物の一覧（番号付き。印のない物が先、捨てた物があと）。何もなければ空。
    fn underfoot_text(&self) -> String {
        let order = self.floor_order(self.pos);
        if order.is_empty() {
            return String::new();
        }
        let items: Vec<String> = order
            .iter()
            .enumerate()
            .map(|(i, &j)| {
                let f = &self.floor_items[j];
                let mark = if f.dropped { " (捨てた)" } else { "" };
                format!("{}) {}{mark}", i + 1, self.item_name(&f.item))
            })
            .collect();
        format!("足元: {}", items.join("  "))
    }

    /// 既知の場所にあるアイテム（explore が拾いに行く）。拾えないものは目指さない。
    fn wants_item_at(&self, p: (i32, i32)) -> bool {
        self.map.is_seen(p.0, p.1)
            && (self.amulet == Some(p)
                // 自動で拾われる物だけを目指す（捨てた物には寄らない）
                || self.auto_pickable(p).is_some_and(|j| self.can_take(self.floor_items[j].item)))
    }

    pub fn inventory_lines(&self) -> Vec<String> {
        self.inventory
            .iter()
            .map(|s| {
                if let Some(g) = &s.gear {
                    return self.gear_line(s.letter, g);
                }
                let mut line = format!("{}) {}", s.letter, self.display_name(s.kind));
                if s.count > 1 {
                    line.push_str(&format!(" x{}", s.count));
                }
                if !self.known[s.kind.index()] {
                    line.push_str(" (未識別)");
                }
                line
            })
            .chain(self.has_amulet.then(|| "★ 魔除けのアミュレット".to_string()))
            .collect()
    }

    /// 装備個体の持ち物の1行。名前・性能・今の装備との差・装備中かどうか。
    /// 未識別なら、正確な値の代わりに分かる範囲だけを出す。
    fn gear_line(&self, letter: char, g: &Gear) -> String {
        let equipped = self.weapon == Some(letter) || self.armor == Some(letter);
        let mut line = format!("{letter}) {} [", g.name());
        if !g.identified {
            line.push_str(&g.guess_text());
        } else {
            line.push_str(&g.stats_text());
            if !equipped {
                // 今の装備（なければ素手・防具なし）との差
                if let Some((lo, hi)) = g.weapon_range() {
                    let (clo, chi) = self.weapon_gear().and_then(|w| w.weapon_range()).unwrap_or((2, 4));
                    line.push_str(&format!(" (装備比 {:+}〜{:+})", lo - clo, hi - chi));
                } else {
                    let cur = self.armor_gear().map_or(0, |a| a.armor_value());
                    line.push_str(&format!(" (装備比 {:+})", g.armor_value() - cur));
                }
            }
        }
        line.push(']');
        if equipped {
            line.push_str(" (装備中)");
        }
        line
    }

    /// 薬を飲む・食べる・巻物を読む。種類が合わないものは失敗（ターン消費なし）。
    /// (成功か, メッセージ, 1ターン消費するか)
    fn consume(&mut self, letter: char, target: Option<char>, how: Consume) -> (bool, String, bool) {
        let Some(si) = self.inventory.iter().position(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let kind = self.inventory[si].kind;
        let fits = match how {
            Consume::Quaff => kind.is_potion(),
            Consume::Eat => kind.is_food() || kind.is_mushroom(),
            Consume::Read => kind.is_scroll(),
        };
        if !fits {
            let (is, instead) = if kind.is_potion() {
                ("薬", "quaff")
            } else if kind.is_scroll() {
                ("巻物", "read")
            } else if kind.is_equipment() {
                ("装備品", "equip")
            } else {
                ("食べ物", "eat")
            };
            return (
                false,
                format!("{letter} は{is}だ。{} ではなく {instead} を使う。", how.command()),
                false,
            );
        }
        let k = kind.index();
        let was_known = self.known[k];
        let verb = if kind.is_potion() {
            "飲んだ"
        } else if kind.is_scroll() {
            "読んだ"
        } else {
            "食べた"
        };
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
                let mut s = format!("HPが{gained}回復した。(HP {}/{})", self.hp, self.max_hp);
                if self.poison > 0 {
                    self.poison = 0;
                    s.push_str(" 毒が抜けた。");
                }
                s
            }
            ItemKind::Bread => {
                // 6個に1個くらいは腐っている
                if self.rng.range(0, 6) == 0 {
                    self.food = (self.food + 30).min(MAX_FOOD);
                    if self.try_poison(6) {
                        "腐っていた！ 毒を受けた。".to_string()
                    } else {
                        "腐っていた！ だが毒は守りに阻まれた。".to_string()
                    }
                } else {
                    self.gain_food(kind.nutrition())
                }
            }
            ItemKind::Jerky => self.gain_food(kind.nutrition()),
            ItemKind::EdibleShroom => {
                format!("おいしい。{}", self.gain_food(kind.nutrition()))
            }
            ItemKind::PoisonShroom => {
                let hurt = if self.try_poison(8) { "毒を受けた。" } else { "毒は守りに阻まれた。" };
                format!("{hurt}{}", self.gain_food(kind.nutrition()))
            }
            ItemKind::VigorShroom => {
                let gained = (self.max_hp - self.hp).min(8);
                self.hp += gained;
                let food = self.gain_food(kind.nutrition());
                format!(
                    "力が湧いてきた。HPが{gained}回復した。(HP {}/{}) {food}",
                    self.hp, self.max_hp
                )
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
            // 装備品は上で equip に回している。ここに来たら作り間違いなので、落とさずに失敗にする
            ItemKind::Dagger
            | ItemKind::Sword
            | ItemKind::Axe
            | ItemKind::Leather
            | ItemKind::Chain
            | ItemKind::Plate => return (false, format!("{letter} は装備品だ。"), false),
            ItemKind::Identify => {
                let ti = match target {
                    Some(t) if t == letter => {
                        return (false, "その巻物自身は対象にできない。".to_string(), false)
                    }
                    Some(t) => match self.inventory.iter().position(|s| s.letter == t) {
                        Some(i) if !self.needs_identify(&self.inventory[i]) => {
                            return (false, format!("{t} はすでに識別済みだ。"), false)
                        }
                        Some(i) => Some(i),
                        None => return (false, format!("持ち物 {t} はない。"), false),
                    },
                    None => self
                        .inventory
                        .iter()
                        .position(|s| s.letter != letter && self.needs_identify(s)),
                };
                match ti {
                    Some(i) if self.inventory[i].gear.is_some() => {
                        let letter = self.inventory[i].letter;
                        let (old, text) = self.identify_gear(letter);
                        format!("{old}の正体が分かった。{text}")
                    }
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

    /// 識別の対象になるか。薬・巻物・キノコは種類が未知のもの、装備は正体が未識別のもの。
    fn needs_identify(&self, s: &Stack) -> bool {
        match &s.gear {
            Some(g) => !g.identified,
            None => !self.known[s.kind.index()],
        }
    }

    /// 装備個体を識別する。(識別前の名前, 識別後の名前と中身の説明)
    fn identify_gear(&mut self, letter: char) -> (String, String) {
        let g = self
            .inventory
            .iter_mut()
            .find(|s| s.letter == letter)
            .and_then(|s| s.gear.as_mut())
            .expect("識別する装備がある");
        let old = g.name();
        g.identified = true;
        (old, format!("{} ({})", g.name(), g.reveal_text()))
    }

    /// 装備している未識別の装備は、身につけた時間が積もる。十分に経つと正体が分かる。
    fn tick_worn(&mut self) {
        for letter in [self.weapon, self.armor].into_iter().flatten() {
            let Some(g) = self
                .inventory
                .iter_mut()
                .find(|s| s.letter == letter)
                .and_then(|s| s.gear.as_mut())
            else {
                continue;
            };
            if g.identified {
                continue;
            }
            g.worn += 1;
            if g.worn >= IDENTIFY_AFTER_WORN {
                let (old, text) = self.identify_gear(letter);
                self.note(&format!("身につけているうちに、{old}の正体が分かった。{text}"));
            }
        }
    }

    /// 防具の接尾辞。
    fn armor_suffix(&self) -> Option<Suffix> {
        self.armor_gear().and_then(|g| g.suffix)
    }

    /// 毒を受ける。Warding の防具なら防いで false を返す。
    fn try_poison(&mut self, n: u32) -> bool {
        if self.armor_suffix() == Some(Suffix::Warding) {
            return false;
        }
        self.poison = (self.poison + n).min(MAX_POISON);
        true
    }

    /// 満腹度を増やす。結果の説明を返す。
    fn gain_food(&mut self, n: i32) -> String {
        let before = self.food;
        self.food = (self.food + n).min(MAX_FOOD);
        format!(
            "満腹度が{}回復した。(満腹度 {}/{})",
            self.food - before,
            self.food,
            MAX_FOOD
        )
    }

    fn hunger_label(&self) -> Option<&'static str> {
        match self.food {
            f if f <= 0 => Some("飢餓"),
            f if f <= WEAK_AT => Some("ひどい空腹"),
            f if f <= HUNGRY_AT => Some("空腹"),
            _ => None,
        }
    }

    /// 満腹度と毒の状態（観測やTUIの見出し用）。
    pub fn status_text(&self) -> String {
        let mut s = format!("満腹度 {}/{}", self.food, MAX_FOOD);
        if self.has_amulet {
            s.push_str(" ★アミュレット所持");
        }
        if let Some(l) = self.hunger_label() {
            s.push_str(&format!("({l})"));
        }
        if self.poison > 0 {
            s.push_str(&format!(" 毒{}", self.poison));
        }
        s
    }

    /// レベルによる攻撃力の上乗せ（2レベルごとに +1）。
    fn level_bonus(&self) -> i32 {
        (self.level as i32 - 1) / 2
    }

    /// 武器の攻撃範囲（含む）。装備がなければ素手。レベルの上乗せを含む。
    fn attack_range(&self) -> (i32, i32) {
        let (lo, hi) = self.weapon_gear().and_then(|g| g.weapon_range()).unwrap_or((2, 4));
        let b = self.level_bonus();
        (lo + b, hi + b)
    }

    /// 経験値を得て、足りればレベルが上がる。
    fn gain_xp(&mut self, n: u32) {
        self.xp += n;
        while self.xp >= self.xp_for_next() {
            self.level += 1;
            self.max_hp += HP_PER_LEVEL;
            self.hp += HP_PER_LEVEL;
            let msg = format!(
                "レベルが上がった！ Lv{} 最大HP {} (HP {}/{})",
                self.level, self.max_hp, self.hp, self.max_hp
            );
            self.note(&msg);
            self.alert = Some("レベルが上がって中断した。".to_string());
        }
    }

    /// 持ち物の文字で指された装備個体。
    fn gear_of(&self, slot: Option<char>) -> Option<&Gear> {
        let l = slot?;
        self.inventory.iter().find(|s| s.letter == l)?.gear.as_ref()
    }

    fn weapon_gear(&self) -> Option<&Gear> {
        self.gear_of(self.weapon)
    }

    fn armor_gear(&self) -> Option<&Gear> {
        self.gear_of(self.armor)
    }

    fn defense(&self) -> i32 {
        self.armor_gear().map_or(0, |g| g.armor_value())
    }

    /// 武器・防具を身につける。(成功か, メッセージ, 1ターン消費するか)
    fn equip(&mut self, letter: char) -> (bool, String, bool) {
        let Some(s) = self.inventory.iter().find(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let Some(gear) = s.gear else {
            return (false, format!("{}は装備できない。", self.display_name(s.kind)), false);
        };
        let slot = if gear.kind.is_weapon() { self.weapon } else { self.armor };
        if slot == Some(letter) {
            return (false, format!("{}はすでに装備している。", gear.name()), false);
        }
        if let Some(cur) = self.gear_of(slot).filter(|g| g.is_cursed()) {
            return (false, format!("{}は呪われていて、はずせない。別の装備には替えられない。", cur.name()), false);
        }
        let old = self.gear_of(slot).map(|g| g.name());
        if gear.kind.is_weapon() {
            self.weapon = Some(letter);
        } else {
            self.armor = Some(letter);
        }
        let mut msg = format!("{}を装備した。({})", gear.name(), gear.stats_text());
        if let Some(o) = old {
            msg.push_str(&format!(" {o}をはずした。"));
        }
        if gear.is_cursed() {
            // 呪いは装備して初めて分かる。正体も明らかになる
            if let Some(g) = self.inventory.iter_mut().find(|s| s.letter == letter).and_then(|s| s.gear.as_mut()) {
                g.identified = true;
            }
            msg.push_str(&format!(" 呪われていた！ もうはずせない。({})", gear.suffix.map_or("", Suffix::describe)));
        }
        (true, msg, true)
    }

    /// 装備をはずす。
    fn unequip(&mut self, letter: char) -> (bool, String, bool) {
        let Some(s) = self.inventory.iter().find(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let Some(gear) = s.gear else {
            return (false, format!("{}は装備品ではない。", self.display_name(s.kind)), false);
        };
        if self.weapon != Some(letter) && self.armor != Some(letter) {
            return (false, format!("{}は装備していない。", gear.name()), false);
        }
        if gear.is_cursed() {
            return (false, format!("{}は呪われていて、はずせない。", gear.name()), false);
        }
        if gear.kind.is_weapon() {
            self.weapon = None;
        } else {
            self.armor = None;
        }
        (true, format!("{}をはずした。", gear.name()), true)
    }

    fn spawn_monsters(&mut self) {
        self.monsters.clear();
        let want = ((2 + self.depth) as usize).min(MAX_MONSTERS);
        let kinds: Vec<&'static MonsterKind> = KINDS
            .iter()
            .copied()
            .filter(|k| k.min_depth <= self.depth)
            .collect();
        let total: u32 = kinds.iter().map(|k| k.weight).sum();
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
            let mut roll = self.rng.range(0, total as i32) as u32;
            let mut kind = kinds[0];
            for k in &kinds {
                if roll < k.weight {
                    kind = k;
                    break;
                }
                roll -= k.weight;
            }
            let hp = kind.hp_at(self.depth);
            self.monsters.push(Monster {
                kind,
                name: kind.name,
                glyph: kind.glyph,
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
        self.tick_worn();
        self.tick_body();
        if self.dead {
            return;
        }
        if self.turn.is_multiple_of(REGEN_INTERVAL)
            && self.hp < self.max_hp
            && self.poison == 0
            && self.food > 0
            && self.visible_monster_indices().is_empty()
        {
            self.hp += 1;
        }
        self.monsters_act();
    }

    /// 1ターンぶんの空腹と毒。
    fn tick_body(&mut self) {
        // Famine の防具は、2ターンに1回、満腹度を余計に減らす
        let drain = if self.armor_suffix() == Some(Suffix::Famine) && self.turn % 2 == 0 { 2 } else { 1 };
        for _ in 0..drain {
            if self.food <= 0 {
                break;
            }
            self.food -= 1;
            match self.food {
                HUNGRY_AT => {
                    self.note("お腹が空いてきた。");
                    self.alert = Some("お腹が空いてきた。".to_string());
                }
                WEAK_AT => {
                    self.note("ひどく空腹だ。何か食べないと倒れる。");
                    self.alert = Some("ひどく空腹になって中断した。".to_string());
                }
                0 => {
                    self.note("飢えて体力が削られていく…。");
                    self.alert = Some("飢えて中断した。".to_string());
                }
                _ => {}
            }
        }
        let mut cause = None;
        if self.food == 0 {
            self.hp -= 1;
            cause = Some("飢え");
        }
        if self.poison > 0 {
            self.poison -= 1;
            self.hp -= 1;
            cause = Some("毒");
        }
        let Some(cause) = cause else { return };
        let msg = format!("{cause}で1ダメージ。(HP {}/{})", self.hp.max(0), self.max_hp);
        self.note(&msg);
        if self.poison == 0 && cause == "毒" && self.hp > 0 {
            self.note("毒が抜けた。");
        }
        if self.hp <= 0 {
            self.dead = true;
            self.note(&format!("{cause}で力尽きた…。ゲームオーバー。"));
        } else if self.hp <= DANGER_HP {
            self.alert = Some("体力が危ない。".to_string());
        }
    }

    fn monsters_act(&mut self) {
        for i in 0..self.monsters.len() {
            let kind = self.monsters[i].kind;
            if kind.slow && self.turn % 2 == 1 {
                continue; // 2ターンに1回しか動けない
            }
            for _ in 0..kind.actions_per_turn {
                // Thorns で倒された敵は、ターンの終わりにまとめて取り除く
                if self.dead || self.monsters[i].hp <= 0 {
                    break;
                }
                self.monster_act(i);
            }
        }
        self.monsters.retain(|m| m.hp > 0);
    }

    /// 敵1体の1回の行動。
    fn monster_act(&mut self, i: usize) {
        let (mpos, name, kind) = (
            self.monsters[i].pos,
            self.monsters[i].name,
            self.monsters[i].kind,
        );
        // こちらから見えている間だけ追いかけてくる
        // (視線判定は向きによって結果が違うことがあるので、プレイヤーの視界に合わせる)
        if !self.map.is_visible(mpos.0, mpos.1) {
            return;
        }
        if kind.erratic && self.rng.range(0, 3) == 0 {
            if let Some(np) = self.random_free_step(mpos) {
                self.monsters[i].pos = np;
            }
            return;
        }
        let (dx, dy) = (self.pos.0 - mpos.0, self.pos.1 - mpos.1);
        if dx.abs() <= 1 && dy.abs() <= 1 {
            let bonus = (self.depth as i32 - 1) / 3;
            let raw = self.rng.range(kind.dmg.0, kind.dmg.1 + 1 + bonus);
            let dmg = (raw - self.defense()).max(1);
            self.hp -= dmg;
            self.hit = true;
            let msg = format!(
                "{name}の攻撃！ {dmg}のダメージを受けた。(HP {}/{})",
                self.hp.max(0),
                self.max_hp
            );
            self.note(&msg);
            if self.armor_suffix() == Some(Suffix::Thorns) {
                self.monsters[i].hp -= 1;
                if self.monsters[i].hp <= 0 {
                    let xp = kind.xp + self.depth - 1;
                    self.pending_xp += xp;
                    self.note(&format!("トゲが{name}に1ダメージを返し、倒した！ (経験値 +{xp})"));
                } else {
                    self.note(&format!("トゲが{name}に1ダメージを返した。"));
                }
            }
            // Thorns で倒された敵は、毒を撒けない
            // Thorns で倒された敵は、毒を撒けない
            if kind.poisons && self.hp > 0 && self.monsters[i].hp > 0 && self.rng.range(0, 2) == 0 {
                if self.try_poison(5) {
                    self.note("毒を受けた！");
                } else {
                    self.note("毒は守りに阻まれた。");
                }
            }
            if self.hp <= 0 {
                self.dead = true;
                self.note("あなたは力尽きた…。ゲームオーバー。");
            }
        } else if let Some(np) = self.monster_step(mpos) {
            self.monsters[i].pos = np;
        }
    }

    /// `from` の周りの、空いている歩ける場所からランダムに1つ。
    fn random_free_step(&mut self, from: (i32, i32)) -> Option<(i32, i32)> {
        let free: Vec<(i32, i32)> = Dir::ALL
            .iter()
            .map(|d| {
                let (dx, dy) = d.delta();
                (from.0 + dx, from.1 + dy)
            })
            .filter(|&p| {
                self.map.tile(p.0, p.1).walkable() && p != self.pos && self.monster_at(p).is_none()
            })
            .collect();
        if free.is_empty() {
            None
        } else {
            Some(free[self.rng.range(0, free.len() as i32) as usize])
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
        let (lo, hi) = self.attack_range();
        let dmg = self.rng.range(lo, hi + 1);
        self.monsters[i].hp -= dmg;
        let (name, hp, max_hp) = {
            let m = &self.monsters[i];
            (m.name, m.hp, m.max_hp)
        };
        let suffix = self.weapon_gear().and_then(|g| g.suffix);
        let mut msg = if hp <= 0 {
            let m = self.monsters.remove(i);
            let xp = m.kind.xp + self.depth - 1;
            self.pending_xp += xp;
            format!("{name}に{dmg}のダメージ。{name}を倒した！ (経験値 +{xp})")
        } else {
            format!("{name}に{dmg}のダメージを与えた。(HP {hp}/{max_hp})")
        };
        let heal = match suffix {
            Some(Suffix::Vampire) => 1,
            Some(Suffix::Vigor) if hp <= 0 => 2,
            _ => 0,
        };
        let gained = heal.min(self.max_hp - self.hp);
        if gained > 0 {
            self.hp += gained;
            msg.push_str(&format!(" 武器が生命を吸った。HP+{gained} (HP {}/{})", self.hp, self.max_hp));
        }
        msg
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
        if self.won {
            return self.outcome(
                cmd.to_string(),
                false,
                "クリア済み。new_game でもう一度遊べる。".to_string(),
            );
        }
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
                        let hint = if self.has_amulet { "ascend で登れる" } else { "descend で降りられる" };
                        (true, format!("階段の上にいる。({hint})"), true)
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
                if self.has_amulet {
                    (false, "アミュレットを持っていると、階段は登り階段だ。ascend で登ろう。".to_string(), false)
                } else if self.pos != self.stairs {
                    (false, "ここに階段はない。".to_string(), false)
                } else if self.depth >= AMULET_DEPTH {
                    (false, "ここが最深部だ。魔除けのアミュレットを探そう。".to_string(), false)
                } else {
                    self.depth += 1;
                    self.new_level();
                    (true, format!("地下{}階に降りた。", self.depth), true)
                }
            }
            Command::Ascend => {
                if !self.has_amulet {
                    (false, "登り階段はない。アミュレットを手に入れると、階段が登り階段になる。".to_string(), false)
                } else if self.pos != self.stairs {
                    (false, "ここに階段はない。".to_string(), false)
                } else if self.depth == 1 {
                    self.won = true;
                    (true, "地上の光が見えた！ 魔除けのアミュレットを持ち帰り、地上へ脱出した。クリア！".to_string(), false)
                } else {
                    self.depth -= 1;
                    self.new_level();
                    (true, format!("地下{}階へ登った。", self.depth), true)
                }
            }
            Command::Wait => (true, "1ターン待った。".to_string(), true),
            Command::Stay(n) => {
                let (ok, msg) = self.stay(n);
                (ok, msg, false)
            }
            Command::Quaff(letter) => self.consume(letter, None, Consume::Quaff),
            Command::Eat(letter) => self.consume(letter, None, Consume::Eat),
            Command::Read(letter, target) => self.consume(letter, target, Consume::Read),
            Command::Equip(letter) => self.equip(letter),
            Command::Unequip(letter) => self.unequip(letter),
            Command::Drop(letter, n) => self.drop_cmd(letter, n),
            Command::Pickup(n) => self.pickup_cmd(n),
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
        let xp = std::mem::take(&mut self.pending_xp);
        if xp > 0 {
            self.gain_xp(xp);
        }
        // 歩いたり転移したりして着いた場所のアイテムを拾う
        if self.pos != pos_before && !self.dead {
            self.pickup_here();
        }
        if spent && !self.dead {
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
        // 敵のターンに倒した敵（Thorns）の経験値
        let xp = std::mem::take(&mut self.pending_xp);
        if xp > 0 {
            self.gain_xp(xp);
        }
        if !self.events.is_empty() {
            message = format!("{message} {}", self.events.join(" "));
        }
        self.outcome(cmd.to_string(), ok, message)
    }

    /// その場に `n` ターン留まる。足元のアイテムを拾う。襲われたり、体力が危なくなったら途中で止まる。
    /// （動かない間は、敵は見えているときしか近づいてこないので「敵が現れる」ことはない）
    fn stay(&mut self, n: u32) -> (bool, String) {
        self.hit = false;
        self.alert = None;
        // 留まるときは、足元にあるアイテムを拾う（拾うこと自体はターンを使わない）
        self.pickup_here();
        for done in 1..=n {
            self.pass_turn();
            let why = if self.dead {
                Some("力尽きた。".to_string())
            } else if std::mem::take(&mut self.hit) {
                Some("攻撃を受けて中断した。".to_string())
            } else {
                self.alert.take()
            };
            if let Some(why) = why {
                return (true, format!("{done}ターン留まったところで、{why}"));
            }
        }
        (true, format!("{n}ターン留まった。"))
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
        self.hit = false;
        self.alert = None;
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
            if let Some(why) = self.interruption() {
                return (true, format!("階段へ向かう途中({n}歩)、{why}"));
            }
        }
        (true, format!("階段まで{n}歩移動した。"))
    }

    /// 自動移動を止めるべき事情（死亡・被弾・新たな敵の出現）。
    fn interruption(&mut self) -> Option<String> {
        let hit = std::mem::take(&mut self.hit);
        let alert = self.alert.take();
        if self.dead {
            return Some("力尽きた。".to_string());
        }
        if hit {
            return Some("攻撃を受けて中断した。".to_string());
        }
        if let Some(a) = alert {
            return Some(a);
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
        self.hit = false;
        self.alert = None;
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
            if let Some(why) = self.interruption() {
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
        if let Some(p) = self.amulet.filter(|p| self.map.is_seen(p.0, p.1)) {
            parts.push(format!(", 魔除けのアミュレットが{}にある。", rel_text(self.pos, p)));
        }
        let under = self.underfoot_text();
        if !under.is_empty() {
            parts.push(format!("{under}。"));
        }
        for f in &self.floor_items {
            if f.pos != self.pos && self.map.is_seen(f.pos.0, f.pos.1) {
                parts.push(format!(
                    "{} {}が{}にある。",
                    f.item.kind().glyph(),
                    self.item_name(&f.item),
                    rel_text(self.pos, f.pos)
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
        if seen && self.amulet == Some((x, y)) {
            return Cell {
                ch: ',',
                visible: self.map.is_visible(x, y),
                seen,
            };
        }
        if seen {
            if let Some(k) = self.item_at((x, y)) {
                return Cell {
                    ch: k.kind().glyph(),
                    visible: self.map.is_visible(x, y),
                    seen,
                };
            }
        }
        Cell {
            ch: match self.map.tile(x, y) {
                _ if !seen => ' ',
                Tile::Stairs if self.has_amulet => '<',
                t => t.glyph(),
            },
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
        let (atk_lo, atk_hi) = self.attack_range();
        let mut s = format!(
            "== 地下{}階 / Lv{} (経験値 {}/{}) / ターン{} / HP {}/{} / 攻撃 {}〜{} / 防御 {} / {} / 位置({},{}) ==\n",
            self.depth,
            self.level,
            self.xp,
            self.xp_for_next(),
            self.turn,
            self.hp,
            self.max_hp,
            atk_lo,
            atk_hi,
            self.defense(),
            self.status_text(),
            self.pos.0,
            self.pos.1
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
        let under = self.underfoot_text();
        if !under.is_empty() {
            s.push_str(&format!("-- {under} --\n"));
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

    fn monster(kind: &'static MonsterKind, pos: (i32, i32), hp: i32) -> Monster {
        Monster {
            kind,
            name: kind.name,
            glyph: kind.glyph,
            hp,
            max_hp: hp,
            pos,
        }
    }

    fn slime(pos: (i32, i32), hp: i32) -> Monster {
        monster(&crate::monster::SLIME, pos, hp)
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

    /// 開始位置の東隣に、指定の敵を置いたゲーム。プレイヤーのHPは十分に高くする。
    fn with_adjacent(seed: u64, kind: &'static MonsterKind) -> Game {
        let mut g = Game::new(seed);
        g.monsters.clear();
        g.floor_items.clear();
        g.hp = 1000;
        g.max_hp = 1000;
        let p = (g.pos.0 + 1, g.pos.1);
        assert!(g.map.tile(p.0, p.1).walkable());
        g.monsters.push(monster(kind, p, 1000));
        g
    }

    /// 持ち物に装備品を直接入れた静かなゲーム。
    fn with_gear(kinds: &[ItemKind]) -> Game {
        let mut g = quiet(2);
        for k in kinds {
            g.take(*k).unwrap();
        }
        g
    }

    #[test]
    fn equip_weapon_changes_attack_range() {
        let mut g = with_gear(&[ItemKind::Axe]);
        let o = g.run("equip a");
        assert!(o.ok, "{}", o.message);
        assert!(o.message.contains("Axeを装備した"), "{}", o.message);
        assert!(g.inventory_lines()[0].contains("(装備中)"));
        assert!(g.observe_text(3).contains("攻撃 5〜9"));
        // 敵の隣で殴る: ダメージは 5..=9
        let p = (g.pos.0 + 1, g.pos.1);
        let mut seen = std::collections::HashSet::new();
        for seed_off in 0..40 {
            g.monsters.clear();
            g.monsters.push(monster(&crate::monster::OGRE, p, 1000));
            g.hp = 1000;
            g.max_hp = 1000;
            let _ = seed_off;
            let before = g.monsters[0].hp;
            g.run("attack east");
            seen.insert(before - g.monsters[0].hp);
        }
        assert!(seen.iter().all(|d| (5..=9).contains(d)), "{seen:?}");
        assert!(seen.len() > 1);
    }

    #[test]
    fn equip_replaces_same_slot_and_unequip_works() {
        let mut g = with_gear(&[ItemKind::Dagger, ItemKind::Sword, ItemKind::Leather]);
        assert!(g.run("equip a").ok);
        let o = g.run("equip b");
        assert!(o.ok && o.message.contains("Daggerをはずした"), "{}", o.message);
        assert!(g.run("equip c").ok); // 防具は別枠
        let lines = g.inventory_lines();
        assert!(!lines[0].contains("(装備中)"));
        assert!(lines[1].contains("(装備中)"));
        assert!(lines[2].contains("(装備中)"));
        // すでに装備している・装備していないものは失敗してターンを使わない
        let t = g.turn();
        assert!(!g.run("equip b").ok);
        assert!(!g.run("unequip a").ok);
        assert_eq!(g.turn(), t);
        assert!(g.run("unequip b").ok);
        assert!(g.observe_text(3).contains("攻撃 2〜4"));
        assert_eq!(g.turn(), t + 1);
    }

    #[test]
    fn equip_only_takes_gear_and_consumables_need_the_matching_verb() {
        let mut g = with_gear(&[ItemKind::Plate, ItemKind::Healing]);
        assert!(g.run("equip a").message.contains("Plate Armorを装備した"));
        let o = g.run("equip b");
        assert!(!o.ok);
        assert!(!g.run("unequip b").ok);
        assert!(!g.run("equip z").ok);

        // 種類の合わない動詞は、ターンを使わず失敗する
        let mut g = with_gear(&[ItemKind::Plate, ItemKind::Healing, ItemKind::Bread, ItemKind::Teleport]);
        let turn = g.turn();
        for (cmd, hint) in [
            ("quaff a", "equip"),
            ("eat b", "quaff"),
            ("read b", "quaff"),
            ("quaff c", "eat"),
            ("read c", "eat"),
            ("eat d", "read"),
            ("quaff d", "read"),
            ("eat a", "equip"),
        ] {
            let o = g.run(cmd);
            assert!(!o.ok && o.message.contains(hint), "{cmd}: {}", o.message);
        }
        assert_eq!(g.turn(), turn);
    }

    #[test]
    fn armor_reduces_damage_but_never_below_one() {
        let mut g = with_adjacent(3, &crate::monster::OGRE);
        g.take(ItemKind::Plate);
        g.run("equip a");
        let mut dmgs = std::collections::HashSet::new();
        for _ in 0..40 {
            let before = g.hp;
            g.run("wait");
            if before != g.hp {
                dmgs.insert(before - g.hp);
            }
        }
        // オーガ 3..=6 から 3 引いて、最低 1
        assert!(dmgs.iter().all(|d| (1..=3).contains(d)), "{dmgs:?}");
        let mut g = with_adjacent(3, &crate::monster::BAT);
        g.take(ItemKind::Plate);
        g.run("equip a");
        for _ in 0..20 {
            let before = g.hp;
            g.run("wait");
            let d = before - g.hp;
            assert!(d >= 0);
        }
        assert!(g.hp < 1000, "最低1ダメージは通るはず");
    }

    #[test]
    fn gear_spawns_by_depth_and_is_known_on_pickup() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..80 {
            let mut g = Game::new(seed);
            for depth in [1u32, 5] {
                g.depth = depth;
                g.spawn_items();
                for f in &g.floor_items {
                    let k = &f.item;
                    assert!(k.kind().min_depth() <= depth, "{k:?} at {depth}");
                    seen.insert(k.kind());
                }
            }
        }
        // 深い階では装備品も実際に出る（どの種類が出るかは乱数次第なので、種類までは問わない）
        assert!(seen.iter().any(|k| k.is_weapon()));
        assert!(seen.iter().any(|k| k.is_armor()));
        let mut g = quiet(4);
        let p = (g.pos.0 + 1, g.pos.1);
        g.floor_items.push(FloorItem::new(p, ItemKind::Sword));
        let o = g.run("move east");
        assert!(o.message.contains("Swordを拾った"), "{}", o.message);
    }

    /// 指定の個体を持ち物に直接入れる。
    fn give(g: &mut Game, gear: Gear) -> char {
        g.take(Item::Gear(gear)).unwrap()
    }

    fn quality_gear(kind: ItemKind, quality: crate::item::Quality, bonus: i32) -> Gear {
        Gear { kind, quality, word: quality.words()[0], bonus, suffix: None, identified: true, worn: 0 }
    }

    #[test]
    fn quality_weights_shift_deeper_and_always_sum_to_100() {
        use crate::item::Quality;
        for d in 1..=30 {
            let w = Quality::weights(d);
            assert_eq!(w.iter().sum::<i32>(), 100, "depth {d}");
            assert!(w.iter().all(|x| *x >= 0), "depth {d}: {w:?}");
        }
        assert_eq!(Quality::weights(1)[2..], [0, 0]);
        assert!(Quality::weights(20)[3] > 0);
        assert!(Quality::weights(20)[0] < Quality::weights(1)[0]);
    }

    #[test]
    fn rolled_gear_stays_inside_its_quality() {
        use crate::item::Quality;
        let mut rng = Rng::new(9);
        let mut seen = std::collections::HashSet::new();
        for i in 0..2000 {
            let depth = 1 + (i % 30);
            let g = Gear::roll(&mut rng, ItemKind::Sword, depth);
            let (lo, hi) = g.quality.bonus_range();
            assert!((lo..=hi).contains(&g.bonus), "{g:?}");
            assert!(g.quality.words().contains(&g.word), "{g:?}");
            seen.insert(g.quality);
        }
        assert_eq!(seen.len(), Quality::ALL.len());
    }

    #[test]
    fn gear_drops_are_deterministic_for_a_seed() {
        let drops = |seed: u64| {
            let mut g = Game::new(seed);
            let mut v = Vec::new();
            for depth in [3u32, 9, 15] {
                g.depth = depth;
                g.spawn_items();
                v.extend(g.floor_items.iter().cloned());
            }
            v
        };
        for seed in 0..10 {
            assert_eq!(drops(seed), drops(seed), "seed {seed}");
        }
        assert!((0..30).any(|s| drops(s) != drops(s + 100)));
    }

    #[test]
    fn gear_does_not_stack_and_takes_one_letter_each() {
        let mut g = quiet(2);
        let a = g.take(ItemKind::Dagger).unwrap();
        let b = g.take(ItemKind::Dagger).unwrap();
        assert_ne!(a, b);
        assert!(g.inventory.iter().all(|s| s.count == 1));
        // 薬は今までどおり重なる
        let p1 = g.take(ItemKind::Healing).unwrap();
        assert_eq!(g.take(ItemKind::Healing), Some(p1));
        assert_eq!(g.inventory.len(), 3);
    }

    #[test]
    fn quality_bonus_applies_to_attack_range_and_defense() {
        use crate::item::Quality;
        let mut g = quiet(2);
        let w = give(&mut g, quality_gear(ItemKind::Sword, Quality::Rare, 3));
        let a = give(&mut g, quality_gear(ItemKind::Chain, Quality::Uncommon, 2));
        assert!(g.run(&format!("equip {w}")).ok);
        assert!(g.run(&format!("equip {a}")).ok);
        assert_eq!(g.attack_range(), (4 + 3, 7 + 3));
        assert_eq!(g.defense(), 2 + 2);
        let o = g.observe_text(3);
        assert!(o.contains("攻撃 7〜10") && o.contains("防御 4"), "{o}");
        // 実際の殴りダメージも範囲内
        let p = (g.pos.0 + 1, g.pos.1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..60 {
            g.monsters.clear();
            g.monsters.push(monster(&crate::monster::OGRE, p, 1000));
            g.hp = 1000;
            g.run("attack east");
            seen.insert(1000 - g.monsters[0].hp);
        }
        assert!(seen.iter().all(|d| (7..=10).contains(d)), "{seen:?}");
    }

    fn suffix_gear(kind: ItemKind, suffix: Suffix) -> Gear {
        Gear {
            kind,
            quality: crate::item::Quality::Rare,
            word: "Sanctified",
            bonus: 2,
            suffix: Some(suffix),
            identified: false,
            worn: 0,
        }
    }

    #[test]
    fn rolled_suffixes_match_the_slot_and_only_uncommon_or_better_have_them() {
        use crate::item::Quality;
        let mut rng = Rng::new(5);
        let mut seen = std::collections::HashSet::new();
        for i in 0..3000 {
            let kind = if i % 2 == 0 { ItemKind::Axe } else { ItemKind::Plate };
            let g = Gear::roll(&mut rng, kind, 1 + (i % 30) as u32);
            if let Some(s) = g.suffix {
                assert_ne!(g.quality, Quality::Common, "{g:?}");
                let pool: Vec<Suffix> = if kind.is_weapon() {
                    Suffix::WEAPON.iter().map(|x| x.0).collect()
                } else {
                    Suffix::ARMOR.iter().map(|x| x.0).collect()
                };
                assert!(pool.contains(&s), "{g:?}");
                seen.insert(s);
            }
            assert_eq!(g.identified, g.quality == Quality::Common, "{g:?}");
        }
        assert_eq!(seen.len(), Suffix::WEAPON.len() + Suffix::ARMOR.len());
    }

    #[test]
    fn might_adds_attack_and_vampire_heals_on_hit() {
        let mut g = with_adjacent(3, &crate::monster::OGRE);
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
        assert!(g.run(&format!("equip {w}")).ok);
        assert_eq!(g.attack_range(), (4 + 2 + 1, 7 + 2 + 1));

        let mut g = with_adjacent(3, &crate::monster::OGRE);
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vampire));
        g.run(&format!("equip {w}"));
        g.hp = 500;
        let o = g.run("attack east");
        // 敵の反撃で減る分があるので、吸った分のメッセージで確かめる
        assert!(o.message.contains("HP+1"), "{}", o.message);
    }

    #[test]
    fn vigor_heals_only_on_a_kill() {
        let mut g = with_adjacent(3, &crate::monster::SLIME);
        g.monsters[0].hp = 1000;
        g.monsters[0].max_hp = 1000;
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vigor));
        g.run(&format!("equip {w}"));
        g.hp = 500;
        let o = g.run("attack east");
        assert!(!o.message.contains("HP+"), "{}", o.message);
        g.monsters[0].hp = 1;
        let o = g.run("attack east");
        assert!(o.message.contains("倒した") && o.message.contains("HP+2"), "{}", o.message);
    }

    #[test]
    fn thorns_hurt_attackers_and_can_kill_them() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        let a = give(&mut g, suffix_gear(ItemKind::Leather, Suffix::Thorns));
        g.run(&format!("equip {a}"));
        let before = g.monsters[0].hp;
        let o = g.run("wait");
        assert!(o.message.contains("トゲ"), "{}", o.message);
        assert_eq!(g.monsters[0].hp, before - 1);
        // とどめを刺す: 取り除かれて経験値が入る
        g.monsters[0].hp = 1;
        let xp = g.xp;
        let o = g.run("wait");
        assert!(o.message.contains("トゲ") && o.message.contains("倒した"), "{}", o.message);
        assert!(g.monsters.is_empty());
        assert!(g.xp > xp);
    }

    #[test]
    fn warding_blocks_poison_from_every_source() {
        let mut g = with_adjacent(3, &crate::monster::SPIDER);
        let a = give(&mut g, suffix_gear(ItemKind::Leather, Suffix::Warding));
        g.run(&format!("equip {a}"));
        for _ in 0..40 {
            g.run("wait");
        }
        assert_eq!(g.poison, 0);
        assert!(g.hp < 1000, "攻撃自体は受ける");
        g.take(ItemKind::PoisonShroom);
        let l = g.inventory.iter().find(|s| s.kind == ItemKind::PoisonShroom).unwrap().letter;
        let o = g.run(&format!("eat {l}"));
        assert_eq!(g.poison, 0, "{}", o.message);
    }

    #[test]
    fn famine_drains_extra_food() {
        let mut plain = quiet(2);
        let mut cursed = quiet(2);
        let a = give(&mut cursed, suffix_gear(ItemKind::Leather, Suffix::Famine));
        cursed.run(&format!("equip {a}"));
        let (f0, f1) = (plain.food, cursed.food);
        plain.run("stay 20");
        cursed.run("stay 20");
        assert_eq!(f0 - plain.food, 20);
        assert!(f1 - cursed.food >= 29, "{} -> {}", f1, cursed.food);
    }

    #[test]
    fn cursed_gear_cannot_be_unequipped_or_swapped() {
        let mut g = quiet(2);
        let c = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
        let d = give(&mut g, Gear::plain(ItemKind::Dagger));
        // 装備するまで呪いは分からない
        assert!(!g.inventory_lines()[0].contains("呪"), "{:?}", g.inventory_lines());
        let o = g.run(&format!("equip {c}"));
        assert!(o.ok && o.message.contains("呪われていた"), "{}", o.message);
        let t = g.turn();
        let o = g.run(&format!("unequip {c}"));
        assert!(!o.ok && o.message.contains("呪われていて"), "{}", o.message);
        let o = g.run(&format!("equip {d}"));
        assert!(!o.ok && o.message.contains("呪われていて"), "{}", o.message);
        assert_eq!(g.turn(), t, "失敗はターンを使わない");
        assert_eq!(g.weapon, Some(c));
        // 呪いの攻撃+3 は効いている
        assert_eq!(g.attack_range(), (5 + 2 + 3, 9 + 2 + 3));
    }

    #[test]
    fn unidentified_gear_shows_prefix_but_hides_suffix() {
        let mut g = quiet(2);
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vampire));
        let line = g.inventory_lines().remove(0);
        assert!(line.contains("Sanctified Sword (?)"), "{line}");
        assert!(!line.contains("Vampire"), "{line}");
        // 床の上でも同じ見え方
        let p = (g.pos.0 + 1, g.pos.1);
        g.floor_items.push(FloorItem::new(p, Item::Gear(suffix_gear(ItemKind::Axe, Suffix::Thorns))));
        g.map.update_fov(g.pos, FOV_RADIUS);
        let look = g.run("look").message;
        assert!(look.contains("Sanctified Axe (?)") && !look.contains("Thorns"), "{look}");
        // 普通の品は隠すものがないので (?) が付かない
        g.take(ItemKind::Dagger);
        assert!(g.inventory_lines().iter().any(|l| l.contains("Basic Dagger [") && !l.contains("(?)")));
        let _ = w;
    }

    #[test]
    fn identify_scroll_works_on_gear_by_target_and_automatically() {
        let mut g = quiet(2);
        g.known[ItemKind::Identify.index()] = true;
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vampire));
        let s1 = g.take(ItemKind::Identify).unwrap();
        let o = g.run(&format!("read {s1} {w}"));
        assert!(o.ok && o.message.contains("of the Vampire"), "{}", o.message);
        assert!(g.inventory_lines()[0].contains("Sanctified Sword of the Vampire"));
        assert!(!g.inventory_lines()[0].contains("(?)"));
        // 識別済みの装備は対象にできず、巻物も減らない
        let s2 = g.take(ItemKind::Identify).unwrap();
        let o = g.run(&format!("read {s2} {w}"));
        assert!(!o.ok && o.message.contains("すでに識別済み"), "{}", o.message);
        // 対象を省くと、未識別の装備が自動で選ばれる
        let a = give(&mut g, suffix_gear(ItemKind::Plate, Suffix::Warding));
        let o = g.run(&format!("read {s2}"));
        assert!(o.ok && o.message.contains("of Warding"), "{}", o.message);
        assert!(g.inventory.iter().find(|s| s.letter == a).unwrap().gear.unwrap().identified);
    }

    #[test]
    fn cursed_gear_is_identified_by_scroll_before_wearing() {
        let mut g = quiet(2);
        g.known[ItemKind::Identify.index()] = true;
        let c = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
        let s = g.take(ItemKind::Identify).unwrap();
        let o = g.run(&format!("read {s} {c}"));
        assert!(o.message.contains("呪われていて"), "{}", o.message);
    }

    #[test]
    fn worn_gear_is_identified_after_enough_turns() {
        let mut g = quiet(2);
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
        let spare = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Might));
        g.run(&format!("equip {w}"));
        for _ in 0..(IDENTIFY_AFTER_WORN - 2) {
            g.run("wait");
        }
        assert!(g.inventory_lines()[0].contains("(?)"));
        let mut told = false;
        for _ in 0..4 {
            told |= g.run("wait").message.contains("正体が分かった");
        }
        assert!(told);
        assert!(g.inventory_lines()[0].contains("of Might") && !g.inventory_lines()[0].contains("(?)"));
        // 装備していないものは、時間が経っても分からない
        assert!(g.inventory_lines()[1].contains("(?)"));
        let _ = spare;
    }

    #[test]
    fn inventory_lines_show_stats_diff_and_identification() {
        use crate::item::Quality;
        let mut g = quiet(2);
        let known = quality_gear(ItemKind::Sword, Quality::Uncommon, 2);
        let a = give(&mut g, Gear::plain(ItemKind::Dagger));
        let b = give(&mut g, known);
        let c = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
        let l = g.inventory_lines();
        // 素手(2〜4)との差
        assert!(l[0].contains("Basic Dagger [攻撃 3〜5 (装備比 +1〜+1)]"), "{}", l[0]);
        assert!(l[1].contains("Basic Sword [攻撃 6〜9 (装備比 +4〜+5)]") || l[1].contains("Sword [攻撃 6〜9"), "{}", l[1]);
        // 未識別は、正確な値ではなく分かる範囲だけ
        assert!(l[2].contains("Sanctified Sword (?) [攻撃 4〜7 +(2〜3)?]"), "{}", l[2]);
        assert!(!l[2].contains("装備比"), "{}", l[2]);
        g.run(&format!("equip {a}"));
        let l = g.inventory_lines();
        assert!(l[0].contains("(装備中)") && !l[0].contains("装備比"), "{}", l[0]);
        // 差は今の装備(短剣 3〜5)から
        assert!(l[1].contains("(装備比 +3〜+4)"), "{}", l[1]);
        let _ = (b, c);
    }

    #[test]
    fn inventory_command_and_heading_carry_the_full_picture() {
        let mut g = quiet(2);
        let a = give(&mut g, suffix_gear(ItemKind::Plate, Suffix::Thorns));
        g.run(&format!("equip {a}"));
        let o = g.observe_text(3);
        // 見出しは品質補正を含む値 (板金 3 + 補正 2)
        assert!(o.contains("防御 5"), "{o}");
        assert!(o.contains("a) Sanctified Plate Armor (?) [防御 3 +(2〜3)?] (装備中)"), "{o}");
    }

    #[test]
    fn spider_killed_by_thorns_does_not_poison() {
        let mut g = with_adjacent(3, &crate::monster::SPIDER);
        let a = give(&mut g, suffix_gear(ItemKind::Leather, Suffix::Thorns));
        g.run(&format!("equip {a}"));
        for _ in 0..40 {
            g.monsters.clear();
            let p = (g.pos.0 + 1, g.pos.1);
            g.monsters.push(monster(&crate::monster::SPIDER, p, 1));
            g.poison = 0;
            g.run("wait");
            assert_eq!(g.poison, 0, "倒された毒グモが毒を撒いた");
            assert!(g.monsters.is_empty());
        }
    }

    /// 足元に、印のない物 `fresh` と、捨てた物 `dropped` を置く。
    fn put_underfoot(g: &mut Game, fresh: &[Item], dropped: &[Item]) {
        for it in fresh {
            g.floor_items.push(FloorItem::new(g.pos, *it));
        }
        for it in dropped {
            g.floor_items.push(FloorItem { pos: g.pos, item: *it, dropped: true });
        }
    }

    #[test]
    fn drop_puts_gear_underfoot_and_marks_it() {
        let mut g = quiet(2);
        let a = give(&mut g, Gear::plain(ItemKind::Dagger));
        let t = g.turn();
        let o = g.run(&format!("drop {a}"));
        assert!(o.ok && o.message.contains("Basic Daggerを足元に捨てた"), "{}", o.message);
        assert_eq!(g.turn(), t + 1);
        assert!(g.inventory.is_empty());
        assert_eq!(g.floor_items.len(), 1);
        assert!(g.floor_items[0].dropped && g.floor_items[0].pos == g.pos);
        // 個体の情報(未識別など)はそのまま床へ
        let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
        g.run(&format!("drop {w}"));
        assert_eq!(g.floor_items[1].item, Item::Gear(suffix_gear(ItemKind::Sword, Suffix::Might)));
    }

    #[test]
    fn drop_with_a_count_drops_that_many_one_by_one() {
        let mut g = quiet(2);
        for _ in 0..3 {
            g.take(ItemKind::Healing);
        }
        let o = g.run("drop a 2");
        assert!(o.ok && o.message.contains("2個"), "{}", o.message);
        assert_eq!(g.inventory[0].count, 1);
        assert_eq!(g.floor_items.iter().filter(|f| f.dropped).count(), 2);
        // 数を省くと1個。使い切ると枠が空く
        assert!(g.run("drop a").ok);
        assert!(g.inventory.is_empty());
        assert_eq!(g.floor_items.len(), 3);
    }

    #[test]
    fn drop_fails_without_spending_a_turn() {
        let mut g = quiet(2);
        let w = give(&mut g, Gear::plain(ItemKind::Dagger));
        let c = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
        g.take(ItemKind::Healing);
        g.run(&format!("equip {w}"));
        let t = g.turn();
        // 装備中
        let o = g.run(&format!("drop {w}"));
        assert!(!o.ok && o.message.contains("unequip"), "{}", o.message);
        // 呪われて装備中
        g.run(&format!("unequip {w}"));
        g.run(&format!("equip {c}"));
        let t = g.turn();
        let o = g.run(&format!("drop {c}"));
        assert!(!o.ok && o.message.contains("呪われて"), "{}", o.message);
        // 持ち物にない文字、持っている数より多い
        assert!(!g.run("drop z").ok);
        assert!(!g.run("drop c 5").ok);
        assert_eq!(g.turn(), t, "失敗はターンを使わない");
        assert!(g.floor_items.is_empty());
        assert_eq!(g.inventory.len(), 3);
        // 呪われていても、まだ身につけていなければ捨てられる(呪いが漏れない)
        let c2 = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
        assert!(g.run(&format!("drop {c2}")).ok);
    }

    #[test]
    fn dropped_items_are_not_picked_up_by_walking_or_staying() {
        let mut g = quiet(2);
        let a = give(&mut g, Gear::plain(ItemKind::Dagger));
        g.run(&format!("drop {a}"));
        let here = g.pos;
        assert!(g.run("move east").ok);
        let o = g.run("move west");
        assert_eq!(g.pos, here);
        assert!(!o.message.contains("拾った"), "{}", o.message);
        let o = g.run("stay 3");
        assert!(!o.message.contains("拾った"), "{}", o.message);
        assert!(g.inventory.is_empty());
        assert_eq!(g.floor_items.len(), 1);
    }

    #[test]
    fn explore_does_not_go_for_dropped_items() {
        let mut g = quiet(3);
        g.map.reveal_all();
        let far = *g
            .find_path(&|q| {
                g.map.tile(q.0, q.1) == Tile::Floor && (q.0 - g.pos.0).abs() + (q.1 - g.pos.1).abs() > 6
            })
            .unwrap()
            .last()
            .unwrap();
        g.floor_items.push(FloorItem { pos: far, item: ItemKind::Healing.into(), dropped: true });
        let start = g.pos;
        let o = g.run("explore");
        assert!(o.message.contains("もう探索する場所がない"), "{}", o.message);
        assert_eq!(g.pos, start);
        // 印のない物なら、今までどおり拾いに行く
        g.floor_items[0].dropped = false;
        g.run("explore");
        assert_eq!(g.pos, far);
        assert_eq!(g.inventory.len(), 1);
    }

    #[test]
    fn walking_onto_a_tile_picks_only_the_top_undropped_item() {
        let mut g = quiet(2);
        let p = (g.pos.0 + 1, g.pos.1);
        g.floor_items.push(FloorItem { pos: p, item: ItemKind::Sleep.into(), dropped: true });
        g.floor_items.push(FloorItem::new(p, ItemKind::Healing));
        g.floor_items.push(FloorItem::new(p, ItemKind::Bread));
        let o = g.run("move east");
        assert!(o.message.contains("を拾った"), "{}", o.message);
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.inventory[0].kind, ItemKind::Healing, "印のない先頭");
        assert_eq!(g.floor_items.len(), 2);
    }

    #[test]
    fn pickup_without_a_number_takes_undropped_first_then_dropped() {
        let mut g = quiet(2);
        // 捨てた物を先に置いても、印のない物が先に拾われる
        put_underfoot(&mut g, &[], &[ItemKind::Sleep.into()]);
        put_underfoot(&mut g, &[ItemKind::Healing.into()], &[]);
        let t = g.turn();
        let o = g.run("pickup");
        assert!(o.ok && o.message.contains("を拾った"), "{}", o.message);
        assert_eq!(g.inventory[0].kind, ItemKind::Healing);
        assert_eq!(g.turn(), t + 1);
        // 印のない物がなくなれば、捨てた物の先頭
        let o = g.run("get");
        assert!(o.ok, "{}", o.message);
        assert_eq!(g.inventory.len(), 2);
        assert!(g.floor_items.is_empty());
        // 足元に何もなければ失敗(ターンを使わない)
        let t = g.turn();
        let o = g.run("pickup");
        assert!(!o.ok && o.message.contains("何もない"), "{}", o.message);
        assert_eq!(g.turn(), t);
    }

    #[test]
    fn pickup_by_number_can_take_dropped_items_and_checks_range() {
        let mut g = quiet(2);
        put_underfoot(&mut g, &[ItemKind::Healing.into()], &[ItemKind::Sleep.into(), ItemKind::Bread.into()]);
        let t = g.turn();
        for bad in ["pickup 4", "pickup 99"] {
            let o = g.run(bad);
            assert!(!o.ok && o.message.contains("1〜3"), "{}", o.message);
        }
        assert_eq!(g.turn(), t);
        // 番号は表示の順 (1 印なし / 2,3 捨てた物)
        let o = g.run("pickup 3");
        assert!(o.ok, "{}", o.message);
        assert_eq!(g.inventory[0].kind, ItemKind::Bread);
        assert_eq!(g.turn(), t + 1);
        assert_eq!(g.floor_items.len(), 2);
    }

    #[test]
    fn pickup_fails_when_the_inventory_is_full_without_spending_a_turn() {
        let mut g = quiet(2);
        for c in 'a'..='z' {
            g.inventory.push(Stack { letter: c, kind: ItemKind::Dagger, count: 1, gear: Some(Gear::plain(ItemKind::Dagger)) });
        }
        put_underfoot(&mut g, &[ItemKind::Healing.into()], &[]);
        let t = g.turn();
        let o = g.run("pickup");
        assert!(!o.ok && o.message.contains("いっぱい"), "{}", o.message);
        assert_eq!(g.turn(), t);
        assert_eq!(g.floor_items.len(), 1);
    }

    #[test]
    fn full_inventory_drop_then_pickup_takes_only_the_potion() {
        let mut g = quiet(2);
        for c in 'a'..='z' {
            g.inventory.push(Stack { letter: c, kind: ItemKind::Dagger, count: 1, gear: Some(Gear::plain(ItemKind::Dagger)) });
        }
        // 1. 満杯で、足元に薬。歩いて乗っても拾えない
        let p = (g.pos.0 + 1, g.pos.1);
        g.floor_items.push(FloorItem::new(p, ItemKind::Healing));
        let o = g.run("move east");
        assert!(o.message.contains("いっぱい"), "{}", o.message);
        assert!(g.inventory.iter().all(|s| s.kind == ItemKind::Dagger));
        // 2. 不要な装備を drop して空きを作る
        assert!(g.run("drop a").ok);
        let u = g.underfoot_text();
        assert!(u.contains("1) ") && u.contains("2) Basic Dagger (捨てた)"), "{u}");
        // 3. 引数なしの pickup は薬だけを拾い、捨てた装備は拾い直さない
        let o = g.run("pickup");
        assert!(o.ok, "{}", o.message);
        assert!(g.inventory.iter().any(|s| s.kind == ItemKind::Healing));
        assert_eq!(g.floor_items.len(), 1);
        assert!(g.floor_items[0].dropped);
        // stay しても捨てた装備は拾わない
        g.run("stay 2");
        assert_eq!(g.floor_items.len(), 1);
    }

    #[test]
    fn underfoot_list_is_numbered_in_pickup_order_and_in_the_observation() {
        let mut g = quiet(2);
        assert!(g.underfoot_text().is_empty());
        assert!(!g.observe_text(3).contains("足元"));
        put_underfoot(&mut g, &[], &[Item::Gear(Gear::plain(ItemKind::Sword))]);
        put_underfoot(&mut g, &[ItemKind::Teleport.into()], &[]);
        let look = g.run("look").message;
        let name = g.looks[ItemKind::Teleport.index()];
        assert!(look.contains(&format!("足元: 1) {name}  2) Basic Sword (捨てた)")), "{look}");
        let o = g.observe_text(3);
        assert!(o.contains(&format!("足元: 1) {name}  2) Basic Sword (捨てた)")), "{o}");
    }

    #[test]
    fn drop_and_pickup_are_deterministic_for_a_seed_and_script() {
        let play = |seed: u64| {
            let mut g = Game::new(seed);
            let mut out = Vec::new();
            for script in ["explore", "pickup", "drop a", "pickup 1", "explore", "drop b", "stay 3", "pickup", "inventory"] {
                let o = g.run(script);
                out.push(format!("{} {} {} {}", o.ok, o.message, o.turn, o.hp));
            }
            out.push(g.observe_text(50));
            out
        };
        for seed in 0..6 {
            assert_eq!(play(seed), play(seed), "seed {seed}");
        }
    }

    #[test]
    fn stay_passes_exactly_the_given_turns_and_defaults_to_one() {
        let mut g = quiet(1);
        let t = g.turn();
        let o = g.run("stay 4");
        assert!(o.ok && o.message.contains("4ターン留まった"), "{}", o.message);
        assert_eq!(g.turn(), t + 4);
        assert_eq!(o.command, "stay 4");
        let o = g.run("stay");
        assert!(o.ok);
        assert_eq!(o.command, "stay 1");
        assert_eq!(g.turn(), t + 5);
        assert!(!g.run("stay 0").ok);
        assert_eq!(g.turn(), t + 5);
        // 記録にはコロンやバッククォートが付かない
        assert_eq!(g.run("`stay 2").command, "stay 2");
    }

    #[test]
    fn stay_stops_when_attacked() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        let t = g.turn();
        let o = g.run("stay 10");
        assert!(o.message.contains("1ターン留まったところで、攻撃を受けて中断した"), "{}", o.message);
        assert_eq!(g.turn(), t + 1);

    }

    /// `stay` の結果メッセージ（「Nターン留まった…」）から N を取り出す。
    fn reported_stay_turns(message: &str) -> u32 {
        let head = message.split("ターン留まった").next().unwrap();
        head.chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .parse()
            .unwrap_or_else(|_| panic!("ターン数が読めない: {message}"))
    }

    #[test]
    fn stay_advances_exactly_the_reported_number_of_turns() {
        // 敵のいる本物のゲームで何度も stay する。中断されても、されなくても、
        // 進んだターン数は報告された数と一致し、指定数を超えない
        for seed in 0..40 {
            let mut g = Game::new(seed);
            for _ in 0..8 {
                if g.is_dead() {
                    break;
                }
                let t = g.turn();
                let o = g.run("stay 4");
                assert!(o.ok, "seed {seed}: {}", o.message);
                let n = reported_stay_turns(&o.message);
                assert!((1..=4).contains(&n), "seed {seed}: {}", o.message);
                assert_eq!(g.turn() - t, n, "seed {seed}: {}", o.message);
            }
        }
    }

    #[test]
    fn interrupted_stay_counts_the_interrupting_turn_once() {
        // 攻撃を受けた回のターンは「留まったターン」に1回だけ数える
        for limit in [1, 2, 5, 10] {
            let mut g = with_adjacent(3, &crate::monster::GOBLIN);
            let t = g.turn();
            let o = g.run(&format!("stay {limit}"));
            let n = reported_stay_turns(&o.message);
            assert!(o.message.contains("攻撃を受けて中断した"), "{}", o.message);
            assert_eq!(g.turn() - t, n, "limit {limit}: {}", o.message);
            assert!(n <= limit);
        }
    }

    #[test]
    fn failed_stay_spends_no_turn() {
        let mut g = quiet(1);
        let t = g.turn();
        for bad in ["stay 0", "stay -1", "stay many", "stay 1001"] {
            assert!(!g.run(bad).ok, "{bad}");
        }
        assert_eq!(g.turn(), t);
    }

    #[test]
    fn stay_picks_up_the_item_underfoot_but_wait_does_not() {
        let mut g = quiet(1);
        let here = g.pos;
        g.floor_items.push(FloorItem::new(here, ItemKind::Healing));
        let o = g.run("wait");
        assert!(o.ok && g.inventory.is_empty() && g.floor_items.len() == 1, "{}", o.message);
        let t = g.turn();
        let o = g.run("stay 2");
        assert!(o.message.contains("を拾った"), "{}", o.message);
        assert!(g.floor_items.is_empty());
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.turn(), t + 2); // 拾うのにターンは使わない
        // 何もなければ、ただ留まるだけ
        let o = g.run("stay");
        assert!(!o.message.contains("拾った"), "{}", o.message);
    }

    /// 開始位置の東 `dist` の床が見えている seed を探す。
    fn seed_with_visible_floor_east(dist: i32) -> (Game, (i32, i32)) {
        for seed in 0..200 {
            let mut g = quiet(seed);
            g.map.update_fov(g.pos, FOV_RADIUS);
            let p = (g.pos.0 + dist, g.pos.1);
            if g.map.tile(p.0, p.1).walkable() && g.map.is_visible(p.0, p.1) {
                return (g, p);
            }
        }
        panic!("条件に合う seed がなかった");
    }

    #[test]
    fn stay_is_not_stopped_by_an_enemy_that_is_merely_in_view() {
        // 遠くに見えているだけのオーガ(2ターンに1回しか動けない)は、2ターンの間は届かない
        let (mut g, p) = seed_with_visible_floor_east(6);
        g.monsters.push(monster(&crate::monster::OGRE, p, 1000));
        let t = g.turn();
        let o = g.run("stay 2");
        assert!(o.message.contains("2ターン留まった。"), "{}", o.message);
        assert_eq!(g.turn(), t + 2);
    }

    #[test]
    fn hunger_grows_warns_and_then_hurts() {
        let mut g = quiet(1);
        g.food = HUNGRY_AT + 1;
        let o = g.run("wait");
        assert!(o.message.contains("お腹が空いてきた"), "{}", o.message);
        assert!(g.status_text().contains("空腹"));
        g.food = WEAK_AT + 1;
        assert!(g.run("wait").message.contains("ひどく空腹"));
        g.food = 1;
        let o = g.run("wait");
        assert!(o.message.contains("飢えて体力が削られ"), "{}", o.message);
        // 飢餓の間は毎ターン1ダメージで、自然回復もしない
        let hp = g.hp();
        for _ in 0..REGEN_INTERVAL {
            g.run("wait");
        }
        assert_eq!(g.hp(), hp - REGEN_INTERVAL as i32);
        assert!(g.observe_text(3).contains("飢餓"));
    }

    #[test]
    fn starvation_can_kill() {
        let mut g = quiet(1);
        g.food = 0;
        g.hp = 2;
        let o = g.run("wait");
        assert!(o.ok);
        let o = g.run("wait");
        assert!(g.is_dead(), "{}", o.message);
        assert!(o.message.contains("飢えで力尽きた"), "{}", o.message);
    }

    #[test]
    fn auto_walk_stops_when_hunger_sets_in() {
        let mut g = quiet(2);
        g.food = HUNGRY_AT + 3;
        let o = g.run("explore");
        assert!(o.ok);
        assert!(o.message.contains("お腹が空いて"), "{}", o.message);
        assert!(g.turn() <= 5, "{}", g.turn());
    }

    #[test]
    fn poison_hurts_each_turn_blocks_regen_and_wears_off() {
        let mut g = quiet(1);
        g.hp = 10;
        g.poison = 3;
        let mut text = String::new();
        for _ in 0..3 {
            text.push_str(&g.run("wait").message);
        }
        assert_eq!(g.hp(), 7);
        assert!(text.contains("毒で1ダメージ"), "{text}");
        assert!(text.contains("毒が抜けた"), "{text}");
        assert_eq!(g.poison, 0);
        // 毒の間は自然回復しない
        g.hp = 20;
        g.poison = 15;
        let hp = g.hp();
        for _ in 0..REGEN_INTERVAL {
            g.run("wait");
        }
        assert_eq!(g.hp(), hp - REGEN_INTERVAL as i32);
    }

    #[test]
    fn poison_can_kill_and_healing_potion_cures_it() {
        let mut g = quiet(1);
        g.hp = 1;
        g.poison = 5;
        let o = g.run("wait");
        assert!(g.is_dead() && o.message.contains("毒で力尽きた"), "{}", o.message);

        let mut g = with_gear(&[ItemKind::Healing]);
        g.poison = 9;
        let o = g.run("quaff a");
        assert!(o.message.contains("毒が抜けた"), "{}", o.message);
        assert_eq!(g.poison, 0);
    }

    #[test]
    fn poison_stops_auto_walk_when_hp_is_low() {
        let mut g = quiet(2);
        g.hp = DANGER_HP + 2;
        g.poison = 10;
        let o = g.run("explore");
        assert!(o.message.contains("体力が危ない"), "{}", o.message);
        assert!(!g.is_dead());
    }

    #[test]
    fn bread_feeds_and_is_sometimes_rotten() {
        let (mut rotten, mut fine) = (0, 0);
        for seed in 0..60 {
            let mut g = with_gear(&[ItemKind::Bread]);
            g.rng = Rng::new(seed);
            g.food = 100;
            let o = g.run("eat a");
            assert!(o.ok);
            if o.message.contains("腐っていた") {
                rotten += 1;
                assert!(g.poison > 0 && g.food < 200, "{}", o.message);
            } else {
                fine += 1;
                assert!(g.food >= 240, "{}", o.message); // 100 + 150 - 1ターン
            }
        }
        assert!(rotten > 0 && fine > rotten, "{rotten} {fine}");
    }

    #[test]
    fn eating_is_capped_at_full() {
        let mut g = with_gear(&[ItemKind::Jerky]);
        g.food = MAX_FOOD - 10;
        let o = g.run("eat a");
        assert!(o.message.contains("満腹度が10回復"), "{}", o.message);
        assert!(g.food <= MAX_FOOD);
    }

    #[test]
    fn mushrooms_are_unidentified_until_eaten() {
        let g = with_gear(&[ItemKind::PoisonShroom, ItemKind::VigorShroom, ItemKind::EdibleShroom]);
        let lines = g.inventory_lines();
        assert!(lines.iter().all(|l| l.contains("キノコ") && l.contains("未識別")), "{lines:?}");
        assert!(lines.iter().all(|l| !l.contains("毒キノコ") && !l.contains("元気")));
        let mut g = with_gear(&[ItemKind::PoisonShroom]);
        let o = g.run("eat a");
        assert!(o.message.contains("これは毒キノコだった"), "{}", o.message);
        assert!(g.poison >= 7, "{}", g.poison);
        let mut g = with_gear(&[ItemKind::VigorShroom]);
        g.hp = 5;
        let o = g.run("eat a");
        assert!(o.message.contains("これは元気キノコだった") && g.hp() >= 12, "{}", o.message);
        assert!(g.known[ItemKind::VigorShroom.index()]);
    }

    #[test]
    fn spider_bites_can_poison() {
        let mut g = with_adjacent(2, &crate::monster::SPIDER);
        let mut saw = false;
        for _ in 0..40 {
            let o = g.run("wait");
            if o.message.contains("毒を受けた！") {
                saw = true;
                assert!(g.poison > 0);
                break;
            }
        }
        assert!(saw, "毒グモに噛まれても毒にならなかった");
    }

    #[test]
    fn every_floor_has_food() {
        for seed in 0..40 {
            let mut g = Game::new(seed);
            for depth in 1..=5u32 {
                g.depth = depth;
                g.spawn_items();
                assert!(
                    g.floor_items.iter().any(|f| f.item.kind().is_food()),
                    "seed {seed} depth {depth}"
                );
            }
        }
    }

    #[test]
    fn spawn_respects_min_depth() {
        for seed in 0..30 {
            for depth in 1..=6u32 {
                let mut g = Game::new(seed);
                g.depth = depth;
                g.spawn_monsters();
                for m in &g.monsters {
                    assert!(m.kind.min_depth <= depth, "seed {seed} depth {depth} {}", m.name);
                    assert_eq!(m.hp, m.kind.hp_at(depth));
                }
            }
        }
    }

    #[test]
    fn shallow_floors_have_only_slimes_and_bats_and_deep_floors_have_all() {
        let mut seen = std::collections::HashSet::new();
        let mut seen_shallow = std::collections::HashSet::new();
        for seed in 0..60 {
            let mut g = Game::new(seed);
            g.depth = 1;
            g.spawn_monsters();
            seen_shallow.extend(g.monsters.iter().map(|m| m.glyph));
            g.depth = 6;
            g.spawn_monsters();
            seen.extend(g.monsters.iter().map(|m| m.glyph));
        }
        assert!(seen_shallow.iter().all(|c| *c == 's' || *c == 'b'), "{seen_shallow:?}");
        for c in ['s', 'b', 'g', 'O'] {
            assert!(seen.contains(&c), "{c} が現れなかった: {seen:?}");
        }
    }

    #[test]
    fn bat_acts_twice_per_turn_and_flutters() {
        let mut double = false;
        let mut missed_turns = 0;
        for seed in 0..40 {
            let mut g = with_adjacent(seed, &crate::monster::BAT);
            for _ in 0..10 {
                let o = g.run("wait");
                let n = o.message.matches("コウモリの攻撃").count();
                assert!(n <= 2, "{}", o.message);
                if n == 2 {
                    double = true;
                }
                if n == 0 {
                    missed_turns += 1;
                }
            }
        }
        assert!(double, "2回攻撃が一度もなかった");
        assert!(missed_turns > 0, "ふらふら動くはずなのに毎ターン攻撃してきた");
    }

    #[test]
    fn ogre_acts_every_other_turn_and_hits_hard() {
        let mut g = with_adjacent(3, &crate::monster::OGRE);
        let mut attacks = 0;
        for _ in 0..10 {
            let before = g.hp;
            let o = g.run("wait");
            if o.message.contains("オーガの攻撃") {
                attacks += 1;
                let dmg = before - g.hp;
                assert!((3..=6).contains(&dmg), "dmg {dmg}");
            } else {
                assert_eq!(before, g.hp);
            }
        }
        assert_eq!(attacks, 5);
    }

    #[test]
    fn goblin_hits_for_at_least_two() {
        let mut g = with_adjacent(5, &crate::monster::GOBLIN);
        let mut attacks = 0;
        for _ in 0..30 {
            let before = g.hp;
            let o = g.run("wait");
            let dmg = before - g.hp;
            if o.message.contains("ゴブリンの攻撃") {
                attacks += 1;
                assert!((2..=4).contains(&dmg), "dmg {dmg}");
            }
        }
        assert_eq!(attacks, 30);
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
            // 空腹の知らせなどで途中で止まることがあるので、着くまで繰り返す
            for _ in 0..5 {
                let o = g.run("travel >");
                assert!(o.ok, "seed {seed}: {}", o.message);
                if o.message.contains("階段まで") || o.message.contains("すでに階段") {
                    break;
                }
            }
            let o = g.run("descend");
            assert!(o.ok, "seed {seed}: {}", o.message);
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
        for k in ItemKind::ALL {
            assert_eq!(a.known[k.index()], k.is_equipment() || k.is_food(), "{k:?}");
        }
        let groups: [fn(ItemKind) -> bool; 3] =
            [|k| k.is_potion(), |k| k.is_scroll(), |k| k.is_mushroom()];
        for in_group in groups {
            let names: Vec<_> = ItemKind::ALL
                .iter()
                .filter(|k| in_group(**k))
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
            for f in &g.floor_items {
                let p = &f.pos;
                assert_eq!(g.map.tile(p.0, p.1), Tile::Floor);
                assert_ne!(*p, g.pos);
            }
        }
    }

    #[test]
    fn walking_onto_an_item_picks_it_up() {
        let mut g = quiet(1);
        let p = (g.pos.0 + 1, g.pos.1);
        g.floor_items.push(FloorItem::new(p, ItemKind::Healing));
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
        g.run("quaff a");
        g.run("quaff a");
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.inventory[0].letter, 'b');
    }

    #[test]
    fn healing_potion_heals_and_identifies() {
        let mut g = quiet(1);
        g.take(ItemKind::Healing);
        g.take(ItemKind::Healing);
        g.hp = 5;
        let o = g.run("quaff a");
        assert!(o.ok);
        assert_eq!(g.hp(), 15);
        assert!(g.known[ItemKind::Healing.index()]);
        assert!(o.message.contains("回復の薬だった"), "{}", o.message);
        let o = g.run("quaff a");
        assert!(o.ok);
        assert!(!o.message.contains("だった！"));
        assert_eq!(g.hp(), 20);
        assert!(g.inventory.is_empty());
        assert!(!g.run("quaff a").ok);
    }

    #[test]
    fn poison_hurts_and_can_kill() {
        let mut g = quiet(1);
        g.take(ItemKind::Poison);
        let o = g.run("quaff a");
        assert!(o.ok);
        assert_eq!(g.hp(), 15);

        let mut g = quiet(1);
        g.take(ItemKind::Poison);
        g.hp = 5;
        let o = g.run("quaff a");
        assert!(g.is_dead());
        assert!(o.message.contains("ゲームオーバー"));
    }

    #[test]
    fn sleeping_passes_turns_while_an_enemy_attacks() {
        let mut g = with_adjacent_slime(1, 50);
        g.floor_items.clear();
        g.take(ItemKind::Sleep);
        let t = g.turn();
        let o = g.run("quaff a");
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
        let o = g.run("read b");
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
        let o = g.run("read c b");
        assert!(o.ok, "{}", o.message);
        assert!(g.known[ItemKind::Poison.index()]);
        assert!(!g.known[ItemKind::Healing.index()]);
        // すでに識別済みの対象は選べない
        g.take(ItemKind::Identify);
        let o = g.run("read c b");
        assert!(!o.ok);
    }

    #[test]
    fn identify_scroll_without_targets() {
        // 正体を知らない巻物は、読むと消費して正体だけ分かる
        let mut g = quiet(1);
        g.take(ItemKind::Identify);
        let o = g.run("read a");
        assert!(o.ok);
        assert!(o.message.contains("何も起こらなかった"));
        assert!(g.inventory.is_empty());
        // 正体を知っている巻物は、対象がなければ消費せず失敗する
        let mut g = quiet(1);
        g.take(ItemKind::Identify);
        g.known[ItemKind::Identify.index()] = true;
        let t = g.turn();
        let o = g.run("read a");
        assert!(!o.ok);
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.turn(), t);
    }

    #[test]
    fn teleport_moves_the_player() {
        let mut g = quiet(1);
        g.take(ItemKind::Teleport);
        let old = g.pos;
        assert!(g.run("read a").ok);
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
            assert!(g.run("read a").ok);
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
        g.floor_items.push(FloorItem::new(p, ItemKind::Teleport));
        g.map.update_fov(g.pos, FOV_RADIUS);
        let o = g.run("look");
        assert!(o.message.contains("東に2"), "{}", o.message);
        assert!(g.observe_text(1).contains('?'));
    }

    #[test]
    fn dying_from_a_poison_potion_does_not_advance_another_turn() {
        let mut g = with_gear(&[ItemKind::Poison]);
        g.hp = 3;
        g.poison = 4;
        let t = g.turn();
        let o = g.run("quaff a");
        assert!(g.is_dead());
        assert_eq!(g.turn(), t, "{}", o.message);
        assert_eq!(o.message.matches("ゲームオーバー").count(), 1, "{}", o.message);
    }

    #[test]
    fn full_inventory_says_so_and_explore_does_not_chase_items() {
        let mut g = quiet(3);
        for (i, c) in ('a'..='z').enumerate() {
            let kind = if i == 0 { ItemKind::Dagger } else { ItemKind::Bread };
            g.inventory.push(Stack { letter: c, kind, count: 1, gear: kind.is_equipment().then(|| Gear::plain(kind)) });
        }
        assert!(!g.can_take(ItemKind::Healing));
        assert!(g.can_take(ItemKind::Bread));
        g.map.reveal_all();
        let far = |g: &Game, d: i32| {
            *g.find_path(&|q| {
                g.map.tile(q.0, q.1) == Tile::Floor
                    && (q.0 - g.pos.0).abs() + (q.1 - g.pos.1).abs() > d
            })
            .unwrap()
            .last()
            .unwrap()
        };
        let (a, b) = (far(&g, 4), far(&g, 12));
        g.floor_items.push(FloorItem::new(a, ItemKind::Healing));
        g.floor_items.push(FloorItem::new(b, ItemKind::Healing));
        let o = g.run("explore");
        assert!(o.message.contains("もう探索する場所がない"), "{}", o.message);
        assert_eq!(g.turn(), 0);
        // 踏めば、拾えないと分かる
        g.floor_items.clear();
        g.floor_items.push(FloorItem::new(g.pos, ItemKind::Healing));
        let o = g.run("stay");
        assert!(o.message.contains("持ち物がいっぱい"), "{}", o.message);
        assert_eq!(g.floor_items.len(), 1);
    }

    /// 最深部で、アミュレットのある床を探して持たせる。
    fn deep_game_with_amulet() -> Game {
        let mut g = quiet(5);
        g.depth = AMULET_DEPTH;
        g.spawn_items();
        g
    }

    #[test]
    fn amulet_only_on_the_deepest_floor_and_only_once() {
        let mut g = quiet(5);
        for d in 1..AMULET_DEPTH {
            g.depth = d;
            g.spawn_items();
            assert!(g.amulet.is_none(), "depth {d}");
        }
        let mut g = deep_game_with_amulet();
        assert!(g.amulet.is_some());
        g.has_amulet = true;
        g.spawn_items();
        assert!(g.amulet.is_none());
    }

    #[test]
    fn picking_up_the_amulet_turns_the_stairs_upward() {
        let mut g = deep_game_with_amulet();
        let a = g.amulet.unwrap();
        g.pos = a;
        g.map.reveal_all();
        g.run("stay");
        assert!(g.has_amulet && g.amulet.is_none());
        assert!(g.inventory_lines().iter().any(|l| l.contains("アミュレット")));
        assert!(g.status_text().contains("アミュレット"));
        let (sx, sy) = g.stairs;
        assert_eq!(g.cell(sx, sy).ch, '<');
        g.pos = g.stairs;
        let o = g.run("descend");
        assert!(!o.ok && o.message.contains("登り階段"), "{}", o.message);
        let o = g.run("ascend");
        assert!(o.ok && g.depth() == AMULET_DEPTH - 1, "{}", o.message);
        assert!(g.amulet.is_none());
    }

    #[test]
    fn cannot_ascend_without_the_amulet_or_descend_past_the_bottom() {
        let mut g = quiet(5);
        g.pos = g.stairs;
        assert!(!g.run("ascend").ok);
        g.depth = AMULET_DEPTH;
        let o = g.run("descend");
        assert!(!o.ok && o.message.contains("最深部"), "{}", o.message);
        assert_eq!(g.depth(), AMULET_DEPTH);
    }

    #[test]
    fn escaping_from_depth_one_with_the_amulet_wins() {
        let mut g = quiet(5);
        g.has_amulet = true;
        g.pos = g.stairs;
        let t = g.turn();
        let o = g.run("ascend");
        assert!(o.ok && g.is_won() && !g.is_dead(), "{}", o.message);
        assert!(o.message.contains("クリア"));
        assert_eq!(g.turn(), t);
        assert!(!g.run("wait").ok);
    }

    #[test]
    fn explore_goes_for_a_seen_amulet() {
        let mut g = deep_game_with_amulet();
        g.map.reveal_all();
        let o = g.run("explore");
        assert!(g.has_amulet, "{}", o.message);
        assert!(o.message.contains("アミュレット"), "{}", o.message);
    }

    #[test]
    fn killing_gives_xp_and_levels_up_with_more_hp_and_attack() {
        let mut g = with_adjacent_slime(3, 1);
        g.floor_items.clear();
        g.hp = 10;
        assert_eq!((g.level(), g.xp_for_next()), (1, 10));
        // スライム(基本3)を3体倒すと 9、4体目で 12 ≥ 10 → レベル2
        let mut text = String::new();
        for n in 0..4 {
            g.monsters.clear();
            g.monsters.push(slime((g.pos.0 + 1, g.pos.1), 1));
            let o = g.run("attack east");
            text.push_str(&o.message);
            assert_eq!(g.level(), if n < 3 { 1 } else { 2 }, "{n} {}", o.message);
        }
        assert!(text.contains("経験値 +3") && text.contains("レベルが上がった！ Lv2"), "{text}");
        assert_eq!(g.max_hp(), PLAYER_MAX_HP + HP_PER_LEVEL);
        assert!(g.hp() >= 10 + HP_PER_LEVEL - 4, "{}", g.hp());
        assert!(g.observe_text(3).contains("Lv2"));
    }

    #[test]
    fn attack_grows_every_two_levels_and_level_up_stops_auto_walk() {
        let mut g = quiet(2);
        assert_eq!(g.attack_range(), (2, 4));
        g.level = 3;
        assert_eq!(g.attack_range(), (3, 5));
        g.level = 1;
        g.xp = g.xp_for_next() - 1;
        g.gain_xp(1);
        assert_eq!(g.level(), 2);
        assert!(g.alert.is_some());
        // 一気に複数レベル上がることもある
        g.gain_xp(1000);
        assert!(g.level() > 4 && g.xp() < g.xp_for_next());
    }
}
