use std::collections::VecDeque;

use crate::command::{self, Command, Dir, TravelTarget};
use crate::item::{
    Class, Effect, Gear, Item, ItemKind, Suffix, Tool, MUSHROOM_LOOKS, POTION_LOOKS, RING_LOOKS, SCROLL_LOOKS,
    WAND_LOOKS,
};
use crate::map::{idx, Map, Tile, H, W};
use crate::monster::{MonsterKind, KINDS};
use crate::rng::Rng;
use crate::status::{Change, Status, StatusEvent, StatusSet};
use crate::trap::{Trap, TrapKind};

mod effects;
mod rings;
mod wands;

use rings::RingFx;

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
const MAX_MONSTERS: usize = 6;
/// 装備して、このターン数が過ぎると、その装備の正体（接尾辞と補正値）が分かる
const IDENTIFY_AFTER_WORN: u32 = 50;
/// レベルアップで増える最大HP
const HP_PER_LEVEL: i32 = 4;
/// 腕力の初期値。10 より大きいと攻撃が増え、小さいと減る（2ごとに1）
const START_STRENGTH: i32 = 10;
/// 最初に持っている松明の燃料
const START_TORCH_FUEL: i32 = 1500;
/// 強化の巻物で付く、攻撃・防御の加算の限度（錆びで下がる限度も同じ）
const MAX_ENCHANT: i32 = 9;
const MIN_STRENGTH: i32 = 3;
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
    /// このコマンドの間に起きた状態の付与・解除
    pub status_events: Vec<StatusEvent>,
    /// 実行直後にかかっている状態（残りターンつき）
    pub statuses: Vec<(Status, u32)>,
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
    /// 敵にかかっている状態（残りターンつき）
    pub statuses: Vec<(Status, u32)>,
}

/// 持ち物の1スタック（同じ種類は重なる）。文字は拾った時に決まり、使い切るまで変わらない。
/// 装備品は重ならず、`gear` に個体の情報が入る（このとき `count` は常に 1）。
struct Stack {
    letter: char,
    kind: ItemKind,
    count: u32,
    gear: Option<Gear>,
    /// 杖など、数値を持つ道具の個体（これも重ならず、`count` は常に 1）
    tool: Option<Tool>,
}

impl Stack {
    /// 床に落とすときの品物。
    fn item(&self) -> Item {
        match (self.gear, self.tool) {
            (Some(g), _) => Item::Gear(g),
            (_, Some(t)) => Item::Tool(t),
            _ => Item::Plain(self.kind),
        }
    }
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
    status: StatusSet,
    /// 消去の杖で特殊な力（毒・錆び・ふらつき）を失った
    cancelled: bool,
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
    /// はめている指輪（スロットは2つ。持ち物の文字で指す）
    rings: [Option<char>; 2],
    /// 装備中の光源（持ち物の外にある。ほかの光源は持ち物に入り、`equip` で持ち替える）
    light: Option<Tool>,
    /// 腕力（今の値と最大値）。攻撃のダメージに効く。毒の薬などで下がる
    strength: i32,
    max_strength: i32,
    /// 探知で見えた敵の位置と記号。次のコマンドを実行するまで地図に残る
    detect_marks: Vec<((i32, i32), char)>,
    /// 満腹度。時間とともに減り、0 になると体力が削られる
    food: i32,
    /// プレイヤーにかかっている状態（毒を含む）
    status: StatusSet,
    /// 加速中、行動のうち世界のターンが進まない側の番か
    free_action: bool,
    /// 隠れていたり見つかったりした罠
    traps: Vec<Trap>,
    /// 実行中の自動移動を止めるべき出来事（被弾など）
    hit: bool,
    alert: Option<String>,
    log: Vec<LogEntry>,
    /// 実行中のコマンドで起きた出来事（Outcome に添える）
    events: Vec<String>,
    /// 実行中のコマンドで起きた状態の変化（Outcome に添える）
    status_events: Vec<StatusEvent>,
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
    let mut wands = WAND_LOOKS;
    let mut rings = RING_LOOKS;
    shuffle(rng, &mut potions);
    shuffle(rng, &mut scrolls);
    shuffle(rng, &mut shrooms);
    shuffle(rng, &mut wands);
    shuffle(rng, &mut rings);
    let mut looks = [""; ItemKind::COUNT];
    let (mut pi, mut si, mut mi, mut wi, mut ri) = (0, 0, 0, 0, 0);
    for k in ItemKind::ALL {
        let pool = match k.class() {
            Class::Potion => Some((&potions[..], &mut pi)),
            Class::Scroll => Some((&scrolls[..], &mut si)),
            Class::Mushroom => Some((&shrooms[..], &mut mi)),
            Class::Wand => Some((&wands[..], &mut wi)),
            Class::Ring => Some((&rings[..], &mut ri)),
            _ => None,
        };
        looks[k.index()] = match pool {
            Some((names, next)) => {
                *next += 1;
                names[*next - 1]
            }
            None => k.true_name(),
        };
    }
    looks
}

/// 隣の相手のいる向き（`dx`, `dy` は相手から見たプレイヤーの位置なので、相手は逆側）。
fn compass(dx: i32, dy: i32) -> &'static str {
    match (-dx.signum(), -dy.signum()) {
        (0, -1) => "北",
        (0, 1) => "南",
        (1, 0) => "東",
        (-1, 0) => "西",
        (1, -1) => "北東",
        (-1, -1) => "北西",
        (1, 1) => "南東",
        (-1, 1) => "南西",
        _ => "すぐそば",
    }
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
            known: ItemKind::ALL.map(|k| k.starts_known()),
            weapon: None,
            armor: None,
            rings: [None, None],
            light: Some(Tool::charged(ItemKind::Torch, START_TORCH_FUEL)),
            strength: START_STRENGTH,
            max_strength: START_STRENGTH,
            detect_marks: Vec::new(),
            food: MAX_FOOD - 50,
            status: StatusSet::default(),
            free_action: false,
            traps: Vec::new(),
            hit: false,
            alert: None,
            log: Vec::new(),
            events: Vec::new(),
            status_events: Vec::new(),
        };
        game.spawn_monsters();
        game.spawn_items();
        game.spawn_traps();
        game.refresh_fov();
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
        self.spawn_traps();
        self.refresh_fov();
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
        let want = 4 + if self.depth >= 3 { 1 } else { 0 };
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
            Item::Tool(t) => self.display_name(t.kind).to_string(),
        }
    }

    fn has_free_letter(&self) -> bool {
        ('a'..='z').any(|c| !self.inventory.iter().any(|s| s.letter == c))
    }

    /// 持ち物に加えられるか（同じ種類のスタックがあるか、空き文字があるか）。装備は重ならない。
    fn can_take(&self, item: impl Into<Item>) -> bool {
        match item.into() {
            Item::Plain(kind) => self.inventory.iter().any(|s| s.kind == kind) || self.has_free_letter(),
            Item::Gear(_) | Item::Tool(_) => self.has_free_letter(),
        }
    }

    /// 持ち物に加える。割り当てた文字を返す。
    fn take(&mut self, item: impl Into<Item>) -> Option<char> {
        let (kind, gear, tool) = match item.into() {
            Item::Plain(kind) => (kind, None, None),
            Item::Gear(g) => (g.kind, Some(g), None),
            Item::Tool(t) => (t.kind, None, Some(t)),
        };
        if gear.is_none() && tool.is_none() {
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
            tool,
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
        if self.is_equipped(letter) {
            let name = match (stack.gear, stack.tool) {
                (Some(g), _) => g.name(),
                (_, Some(t)) => self.ring_name(&t),
                _ => String::new(),
            };
            return if stack.gear.is_some_and(|g| g.is_sticky()) || stack.tool.is_some_and(|t| t.is_sticky()) {
                (false, format!("{name}は呪われていて、はずせない。捨てられない。"), false)
            } else {
                (false, format!("{name}は装備中だ。先に unequip {letter} ではずそう。"), false)
            };
        }
        if count > stack.count {
            return (false, format!("{letter} は{}個しか持っていない。", stack.count), false);
        }
        let item = stack.item();
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
                if let Some(t) = &s.tool {
                    return self.tool_line(s.letter, t);
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

    /// 装備中の光源の1行（持ち物の外にあるので、一覧とは別に出す）。
    pub fn light_line(&self) -> String {
        match self.light {
            Some(l) => format!("光源: {} (装備中){}", self.light_text(&l), if l.val <= 0 { " 燃え尽きている" } else { "" }),
            None => "光源: なし".to_string(),
        }
    }

    /// 杖などの持ち物の1行。残りの使用回数は識別前でも分かる。
    fn tool_line(&self, letter: char, t: &Tool) -> String {
        if t.kind.is_ring() {
            let mut line = format!("{letter}) {}", self.ring_name(t));
            if !self.known[t.kind.index()] {
                line.push_str(" (未識別)");
            } else if t.identified {
                line.push_str(&format!(" [{}]", t.kind.ring_def().map_or(String::new(), |r| r.effect.describe(t.val))));
                if t.cursed {
                    line.push_str(if t.freed { " (呪い解除済み)" } else { " (呪われている)" });
                }
            }
            if self.rings.contains(&Some(letter)) {
                line.push_str(" (装備中)");
            }
            return line;
        }
        let mut line = format!("{letter}) {}", self.display_name(t.kind));
        if t.kind.is_wand() {
            line.push_str(&format!(" [残り{}回]", t.val));
        } else if t.kind.is_light() {
            let max = t.kind.light_def().map_or(0, |d| d.max_fuel);
            line.push_str(&format!(" [燃料 {}/{}]", t.val, max));
        }
        if !self.known[t.kind.index()] {
            line.push_str(" (未識別)");
        }
        line
    }

    /// 持っている杖の残り回数（ステータス行用）。例: `杖 c:4 d:0`
    fn wands_text(&self) -> String {
        let wands: Vec<String> = self
            .inventory
            .iter()
            .filter_map(|s| s.tool.filter(|t| t.kind.is_wand()).map(|t| format!("{}:{}", s.letter, t.val)))
            .collect();
        wands.join(" ")
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

    /// 識別の対象になるか。薬・巻物・キノコは種類が未知のもの、装備は正体が未識別のもの。
    fn needs_identify(&self, s: &Stack) -> bool {
        match (&s.gear, &s.tool) {
            (Some(g), _) => !g.identified,
            (_, Some(t)) if t.kind.is_ring() => !self.known[s.kind.index()] || !t.identified,
            _ => !self.known[s.kind.index()],
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
        self.inflict(Status::Poisoned, n);
        true
    }

    /// 毒の残りターン。
    #[cfg(test)]
    fn poison(&self) -> u32 {
        self.status.get(Status::Poisoned)
    }

    /// プレイヤーに状態を付ける。状態のはじまりのメッセージを返す。
    fn inflict(&mut self, s: Status, turns: u32) -> String {
        let left = self.status.apply(s, turns);
        self.status_events.push(StatusEvent {
            target: "player".to_string(),
            status: s,
            change: Change::Apply(left),
        });
        if s == Status::Blind {
            self.refresh_fov();
        }
        if s == Status::Hasted {
            // 加速をもたらした行動そのものは、1ターンかかる(次の行動から2回に1回になる)
            self.free_action = true;
        }
        s.def().start.to_string()
    }

    /// プレイヤーの状態を解く。かかっていたなら true。
    fn cure(&mut self, s: Status) -> bool {
        let was = self.status.clear(s);
        if was {
            self.status_events.push(StatusEvent {
                target: "player".to_string(),
                status: s,
                change: Change::End,
            });
            if s == Status::Blind {
                self.refresh_fov();
            }
        }
        was
    }

    /// 敵に状態を付ける。効かない状態なら false。
    fn inflict_monster(&mut self, i: usize, s: Status, turns: u32) -> bool {
        if !s.def().on_monster {
            return false;
        }
        let left = self.monsters[i].status.apply(s, turns);
        self.status_events.push(StatusEvent {
            target: self.monsters[i].name.to_string(),
            status: s,
            change: Change::Apply(left),
        });
        true
    }

    /// 動けない（睡眠・停止）。
    fn incapacitated(&self) -> bool {
        self.status.has(Status::Asleep) || self.status.has(Status::Paralyzed)
    }

    /// 今の視界の半径。
    fn sight_radius(&self) -> i32 {
        match self.light {
            Some(l) if l.val > 0 => FOV_RADIUS,
            _ => crate::item::DARK_RADIUS,
        }
    }

    /// 視界を更新する。盲目なら何も見えず、新しく覚えることもない。
    fn refresh_fov(&mut self) {
        if self.status.has(Status::Blind) {
            self.map.clear_visible();
        } else {
            self.map.update_fov(self.pos, self.sight_radius());
        }
    }

    /// 幻覚で見える敵の種類（ターンと敵ごとに決まる。乱数は使わない）。
    fn hallu_kind(&self, i: usize) -> &'static MonsterKind {
        let mut x = self.seed ^ ((self.turn as u64) << 24) ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        x ^= x >> 33;
        KINDS[(x % KINDS.len() as u64) as usize]
    }

    /// 敵 `i` がいま見えているか（視界の中で、盲目でも透明でもない）。
    fn can_see_monster(&self, i: usize) -> bool {
        let m = &self.monsters[i];
        !self.status.has(Status::Blind)
            && self.map.is_visible(m.pos.0, m.pos.1)
            && (!m.status.has(Status::Invisible)
                || self.status.has(Status::SeeInvisible)
                || self.ring_fx().see_invisible)
    }

    /// メッセージや一覧に出る敵の名前。見えなければ「何か」、幻覚ならでたらめ。
    fn foe_name(&self, i: usize) -> &'static str {
        if !self.can_see_monster(i) {
            "何か"
        } else if self.status.has(Status::Hallucinating) {
            self.hallu_kind(i).name
        } else {
            self.monsters[i].name
        }
    }

    fn foe_glyph(&self, i: usize) -> char {
        if self.status.has(Status::Hallucinating) {
            self.hallu_kind(i).glyph
        } else {
            self.monsters[i].glyph
        }
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
        let mut s = format!("腕力 {}/{} 満腹度 {}/{}", self.effective_strength(), self.max_strength, self.food, MAX_FOOD);
        if self.has_amulet {
            s.push_str(" ★アミュレット所持");
        }
        if let Some(l) = self.hunger_label() {
            s.push_str(&format!("({l})"));
        }
        let wands = self.wands_text();
        if !wands.is_empty() {
            s.push_str(&format!(" 杖残り[{wands}]"));
        }
        match self.light {
            Some(l) => {
                let max = l.kind.light_def().map_or(0, |d| d.max_fuel);
                s.push_str(&format!(" 光源:{} 燃料{}/{}", l.kind.true_name(), l.val, max));
                if l.val <= 0 {
                    s.push_str("(暗闇: 視界が狭い)");
                } else if l.val <= 100 {
                    s.push_str("(もうすぐ尽きる)");
                }
            }
            None => s.push_str(" 光源なし(暗闇: 視界が狭い)"),
        }
        let fx = self.status.short_text();
        if !fx.is_empty() {
            s.push_str(&format!(" {fx}"));
        }
        s
    }

    /// レベルによる攻撃力の上乗せ（2レベルごとに +1）。
    fn level_bonus(&self) -> i32 {
        (self.level as i32 - 1) / 2
    }

    /// 腕力による攻撃の上乗せ。10 を基準に、2ごとに +1（下がると減る）。
    fn strength_bonus(&self) -> i32 {
        (self.effective_strength() - START_STRENGTH) / 2
    }

    /// 指輪を含めた今の腕力。
    fn effective_strength(&self) -> i32 {
        (self.strength + self.ring_fx().strength).max(1)
    }

    /// 武器の攻撃範囲（含む）。装備がなければ素手。レベルの上乗せを含む。
    fn attack_range(&self) -> (i32, i32) {
        let (lo, hi) = self.weapon_gear().and_then(|g| g.weapon_range()).unwrap_or((2, 4));
        let b = self.level_bonus() + self.strength_bonus() + self.ring_fx().damage;
        ((lo + b).max(1), (hi + b).max(1))
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
        self.armor_gear().map_or(0, |g| g.armor_value()) + self.ring_fx().protection
    }

    /// 武器・防具を身につける。(成功か, メッセージ, 1ターン消費するか)
    fn equip(&mut self, letter: char) -> (bool, String, bool) {
        let Some(s) = self.inventory.iter().find(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        if let Some(t) = s.tool {
            match t.kind.class() {
                Class::Ring => return self.equip_ring(letter, t),
                Class::Light => return self.equip_light(letter, t),
                _ => {}
            }
        }
        let Some(gear) = s.gear else {
            return (false, format!("{}は装備できない。", self.display_name(s.kind)), false);
        };
        let slot = if gear.kind.is_weapon() { self.weapon } else { self.armor };
        if slot == Some(letter) {
            return (false, format!("{}はすでに装備している。", gear.name()), false);
        }
        if let Some(cur) = self.gear_of(slot).filter(|g| g.is_sticky()) {
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
        if gear.is_sticky() {
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
        if let Some(t) = s.tool.filter(|t| t.kind.is_ring()) {
            return self.unequip_ring(letter, t);
        }
        let Some(gear) = s.gear else {
            return (false, format!("{}は装備品ではない。", self.display_name(s.kind)), false);
        };
        if self.weapon != Some(letter) && self.armor != Some(letter) {
            return (false, format!("{}は装備していない。", gear.name()), false);
        }
        if gear.is_sticky() {
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
            let kind = self.pick_monster_kind();
            self.add_monster(kind, (x, y));
        }
    }

    /// この階にふさわしい敵の種類を、出やすさの重みで選ぶ。
    fn pick_monster_kind(&mut self) -> &'static MonsterKind {
        let kinds: Vec<&'static MonsterKind> = KINDS
            .iter()
            .copied()
            .filter(|k| k.min_depth <= self.depth)
            .collect();
        let total: u32 = kinds.iter().map(|k| k.weight).sum();
        let mut roll = self.rng.range(0, total as i32) as u32;
        let mut kind = kinds[0];
        for k in &kinds {
            if roll < k.weight {
                kind = k;
                break;
            }
            roll -= k.weight;
        }
        kind
    }

    /// 敵を1体、この階の深さのHPで置く。
    fn add_monster(&mut self, kind: &'static MonsterKind, pos: (i32, i32)) -> usize {
        let hp = kind.hp_at(self.depth);
        self.monsters.push(Monster {
            kind,
            name: kind.name,
            glyph: kind.glyph,
            hp,
            max_hp: hp,
            pos,
            status: StatusSet::default(),
            cancelled: false,
        });
        self.monsters.len() - 1
    }

    fn monster_at(&self, p: (i32, i32)) -> Option<usize> {
        self.monsters.iter().position(|m| m.pos == p)
    }

    fn visible_monster_indices(&self) -> Vec<usize> {
        (0..self.monsters.len()).filter(|&i| self.can_see_monster(i)).collect()
    }

    /// 敵 `i` が、いまこちらに気づいているか。見通せる範囲に入ったとき気づく。
    /// 怒っている敵はどこにいても気づく。透明だと近づかれるまで気づかれない。
    fn monster_aware(&self, i: usize) -> bool {
        let m = &self.monsters[i];
        if m.status.has(Status::Enraged) || self.ring_fx().aggravate {
            return true;
        }
        let r = self.notice_radius();
        let (dx, dy) = (m.pos.0 - self.pos.0, m.pos.1 - self.pos.1);
        dx * dx + dy * dy <= r * r && self.map.los(self.pos, m.pos)
    }

    /// 敵がこちらに気づく距離。
    fn notice_radius(&self) -> i32 {
        // 隠密でも、2マス(隣接を含む)より近づかれたら必ず気づく。透明なら2マスまで縮む
        let r = (FOV_RADIUS - 3 * self.ring_fx().stealth).max(2);
        if self.status.has(Status::Invisible) {
            r.min(2)
        } else {
            r
        }
    }

    pub fn visible_enemies(&self) -> Vec<EnemyView> {
        self.visible_monster_indices()
            .into_iter()
            .map(|i| {
                let m = &self.monsters[i];
                EnemyView {
                    name: self.foe_name(i),
                    glyph: self.foe_glyph(i),
                    hp: m.hp,
                    max_hp: m.max_hp,
                    pos: m.pos,
                    statuses: m.status.active(),
                }
            })
            .collect()
    }

    /// 混乱していると、向きがときどきずれる。(実際の向き, ずれたか)
    fn confuse_dir(&mut self, d: Dir) -> (Dir, bool) {
        if self.status.has(Status::Confused) && self.rng.range(0, 2) == 0 {
            let nd = Dir::ALL[self.rng.range(0, Dir::ALL.len() as i32) as usize];
            (nd, nd != d)
        } else {
            (d, false)
        }
    }

    /// この階に罠を隠す。落とし穴は、最深部とアミュレット所持中には置かない。
    fn spawn_traps(&mut self) {
        self.traps.clear();
        let want = (1 + self.depth / 4).min(4);
        let can_fall = self.depth < AMULET_DEPTH && !self.has_amulet;
        let kinds: Vec<TrapKind> = TrapKind::ALL
            .iter()
            .copied()
            .filter(|k| can_fall || *k != TrapKind::Trapdoor)
            .collect();
        let total: u32 = kinds.iter().map(|k| k.weight()).sum();
        for _ in 0..300 {
            if self.traps.len() as u32 >= want {
                break;
            }
            let x = self.rng.range(1, W - 1);
            let y = self.rng.range(1, H - 1);
            if self.map.tile(x, y) != Tile::Floor
                || (x, y) == self.pos
                || self.traps.iter().any(|t| t.pos == (x, y))
            {
                continue;
            }
            let mut roll = self.rng.range(0, total as i32) as u32;
            let mut kind = kinds[0];
            for k in &kinds {
                if roll < k.weight() {
                    kind = *k;
                    break;
                }
                roll -= k.weight();
            }
            self.traps.push(Trap { pos: (x, y), kind, revealed: false });
        }
    }

    /// 足元に罠があれば発動する。浮遊中は無視する。
    fn trigger_trap(&mut self) {
        let Some(ti) = self.traps.iter().position(|t| t.pos == self.pos) else {
            return;
        };
        if self.status.has(Status::Levitating) {
            return;
        }
        self.fire_trap(ti);
    }

    /// 罠 `ti` を発動する（足元の罠として）。
    fn fire_trap(&mut self, ti: usize) {
        self.traps[ti].revealed = true;
        let kind = self.traps[ti].kind;
        match kind {
            TrapKind::Trapdoor => {
                let dmg = self.rng.range(1, 4);
                self.hp -= dmg;
                if self.hp <= 0 {
                    self.dead = true;
                    self.note("落とし穴に落ちた！ 打ちどころが悪かった…。ゲームオーバー。");
                    return;
                }
                self.depth += 1;
                self.new_level();
                self.note(&format!(
                    "落とし穴に落ちた！ {dmg}のダメージ。地下{}階に着いた。(HP {}/{})",
                    self.depth, self.hp, self.max_hp
                ));
                self.alert = Some("落とし穴に落ちて中断した。".to_string());
            }
            TrapKind::Dart => {
                self.hp -= 2;
                self.hit = true;
                self.note(&format!("毒矢の罠だ！ 2のダメージを受けた。(HP {}/{})", self.hp.max(0), self.max_hp));
                if self.hp <= 0 {
                    self.dead = true;
                    self.note("あなたは力尽きた…。ゲームオーバー。");
                } else if self.try_poison(5) {
                    self.note("毒を受けた！");
                } else {
                    self.note("毒は守りに阻まれた。");
                }
            }
            TrapKind::SleepGas => {
                let msg = self.inflict(Status::Asleep, 5);
                self.note(&format!("眠りガスの罠だ！ {msg}"));
                self.alert = Some("眠りガスで中断した。".to_string());
            }
        }
    }

    /// 罠の解除の成功率(%)。器用さの指輪で上がり、混乱していると下がる。
    fn disarm_percent(&self) -> i32 {
        let mut p = 60 + 10 * self.ring_fx().dexterity;
        if self.status.has(Status::Confused) {
            p -= 30;
        }
        p.clamp(10, 95)
    }

    /// `disarm [向き]`: 見つけた罠を解除する。向きを省くと足元。確率で成功し、失敗すると罠が作動することがある。
    fn disarm(&mut self, d: Option<Dir>) -> (bool, String, bool) {
        if self.status.has(Status::Blind) {
            return (false, "目が見えなくて、罠をいじれない。".to_string(), false);
        }
        let target = match d {
            Some(d) => (self.pos.0 + d.delta().0, self.pos.1 + d.delta().1),
            None => self.pos,
        };
        let Some(ti) = self.traps.iter().position(|t| t.pos == target && t.revealed) else {
            return (false, "そこには見つけた罠がない。(隠れた罠は解除できない。近くに立って探そう)".to_string(), false);
        };
        let kind = self.traps[ti].kind;
        let percent = self.disarm_percent();
        if self.rng.range(0, 100) < percent {
            self.traps.remove(ti);
            self.map.mark_seen(target.0, target.1);
            return (true, format!("{}を解除した！ (成功率{percent}%)", kind.name()), true);
        }
        let mut msg = format!("{}の解除に失敗した。(成功率{percent}%)", kind.name());
        // 失敗すると3回に1回は作動する。足元の罠以外の落とし穴は、落ちずに済む
        if self.rng.range(0, 3) == 0 {
            if self.status.has(Status::Levitating) {
                // 浮いているので、罠は作動しない
                msg.push_str(" 手元が狂ったが、浮いているので罠は作動しなかった。");
            } else if kind == TrapKind::Trapdoor && target != self.pos {
                msg.push_str(" 床板がきしんだが、落ちずに済んだ。");
            } else {
                msg.push_str(" 手元が狂って罠が作動した！");
                self.fire_trap(ti);
            }
        }
        (true, msg, true)
    }

    /// 周りの隠れた罠を見つける。半径 `radius` 以内の罠が、`percent`% の確率で見つかる。
    fn search_traps(&mut self, radius: i32, percent: i32) {
        for ti in 0..self.traps.len() {
            let t = self.traps[ti];
            let (dx, dy) = ((t.pos.0 - self.pos.0).abs(), (t.pos.1 - self.pos.1).abs());
            if t.revealed || dx.max(dy) > radius {
                continue;
            }
            if percent >= 100 || self.rng.range(0, 100) < percent {
                self.traps[ti].revealed = true;
                self.map.mark_seen(t.pos.0, t.pos.1);
                self.note(&format!("{}を見つけた！", t.kind.name()));
            }
        }
    }

    /// 知っている罠（見つけたもの）が `p` にあるか。
    fn known_trap_at(&self, p: (i32, i32)) -> Option<Trap> {
        self.traps.iter().copied().find(|t| t.pos == p && t.revealed)
    }

    /// 敵が見えていて自動移動できないなら、その理由。
    fn refuse_if_enemies(&self) -> Option<String> {
        let idxs = self.visible_monster_indices();
        if idxs.is_empty() {
            return None;
        }
        let names: Vec<&str> = idxs.iter().map(|&i| self.foe_name(i)).collect();
        Some(format!(
            "敵が見えている({})。先に倒すか、手動で動こう。",
            names.join("、")
        ))
    }

    /// 1ターン進める。空腹・毒・状態、指輪と光源の効果、そして敵の行動。
    fn pass_turn(&mut self) {
        self.turn += 1;
        let fx = self.ring_fx();
        self.tick_worn();
        self.tick_rings_worn();
        self.tick_body(&fx);
        if self.dead {
            return;
        }
        self.tick_equipment(&fx);
        self.monsters_act();
    }

    /// プレイヤーの1回の行動のあとに世界を進める。加速中は2回に1回だけ、減速中は2ターン進む。
    fn pass_action(&mut self) {
        let hasted = self.status.has(Status::Hasted);
        let slowed = self.status.has(Status::Slowed);
        if hasted && !slowed {
            self.free_action = !self.free_action;
            if self.free_action {
                return;
            }
        } else {
            self.free_action = false;
        }
        self.pass_turn();
        if slowed && !hasted && !self.dead {
            self.pass_turn();
        }
    }

    /// 1ターンぶんの空腹と毒。
    fn tick_body(&mut self, fx: &RingFx) {
        // Famine の防具は、2ターンに1回、満腹度を余計に減らす。消化遅延の指輪は減る回数を間引く
        let drain = if fx.slow_digestion > 0 && self.turn % (fx.slow_digestion as u32 + 1) != 0 {
            0
        } else if self.armor_suffix() == Some(Suffix::Famine) && self.turn % 2 == 0 {
            2
        } else {
            1
        };
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
        if self.status.has(Status::Poisoned) {
            self.hp -= 1;
            cause = Some("毒");
        }
        if let Some(cause) = cause {
            let msg = format!("{cause}で1ダメージ。(HP {}/{})", self.hp.max(0), self.max_hp);
            self.note(&msg);
            if self.hp <= 0 {
                self.dead = true;
                self.note(&format!("{cause}で力尽きた…。ゲームオーバー。"));
                return;
            } else if self.hp <= DANGER_HP {
                self.alert = Some("体力が危ない。".to_string());
            }
        }
        self.tick_statuses();
    }

    /// 状態の残りターンを1つ減らす。切れたものは知らせる。
    fn tick_statuses(&mut self) {
        for s in self.status.tick() {
            self.status_events.push(StatusEvent {
                target: "player".to_string(),
                status: s,
                change: Change::End,
            });
            if s == Status::Blind {
                self.refresh_fov();
            }
            self.note(s.def().end);
        }
    }

    fn monsters_act(&mut self) {
        for i in 0..self.monsters.len() {
            self.tick_monster(i);
            let kind = self.monsters[i].kind;
            let st = self.monsters[i].status;
            if self.monsters[i].hp <= 0 || st.has(Status::Asleep) || st.has(Status::Paralyzed) {
                continue;
            }
            // 遅い敵・減速した敵は行動が半分に、加速した敵は倍になる（打ち消し合う）
            let net = kind.slow as i32 + st.has(Status::Slowed) as i32 - st.has(Status::Hasted) as i32;
            if net > 0 && self.turn % (1 << net) != 0 {
                continue;
            }
            let acts = kind.actions_per_turn * if net < 0 { 2 } else { 1 };
            for _ in 0..acts {
                // Thorns で倒された敵は、ターンの終わりにまとめて取り除く
                if self.dead || self.monsters[i].hp <= 0 {
                    break;
                }
                self.monster_act(i);
            }
        }
        self.monsters.retain(|m| m.hp > 0);
    }

    /// 敵1体の1ターンぶんの状態（毒のダメージと、残りターンの減少）。
    fn tick_monster(&mut self, i: usize) {
        if self.monsters[i].status.has(Status::Poisoned) {
            self.monsters[i].hp -= 1;
            if self.monsters[i].hp <= 0 {
                let name = self.foe_name(i);
                let xp = self.monsters[i].kind.xp + self.depth - 1;
                self.pending_xp += xp;
                self.note(&format!("{name}は毒で倒れた！ (経験値 +{xp})"));
                return;
            }
        }
        for s in self.monsters[i].status.tick() {
            self.status_events.push(StatusEvent {
                target: self.monsters[i].name.to_string(),
                status: s,
                change: Change::End,
            });
        }
    }

    /// `from` から見て、プレイヤーからいちばん遠ざかれる空きマス（今より遠くなるときだけ）。
    fn flee_step(&self, from: (i32, i32)) -> Option<(i32, i32)> {
        let dist = |p: (i32, i32)| (p.0 - self.pos.0).pow(2) + (p.1 - self.pos.1).pow(2);
        Dir::ALL
            .iter()
            .map(|d| {
                let (dx, dy) = d.delta();
                (from.0 + dx, from.1 + dy)
            })
            .filter(|&p| self.map.tile(p.0, p.1).walkable() && p != self.pos && self.monster_at(p).is_none())
            .filter(|&p| dist(p) > dist(from))
            .max_by_key(|&p| dist(p))
    }

    /// 敵1体の1回の行動。
    fn monster_act(&mut self, i: usize) {
        let (mpos, kind) = (self.monsters[i].pos, self.monsters[i].kind);
        let st = self.monsters[i].status;
        // こちらに気づいている間だけ追いかけてくる
        if !self.monster_aware(i) {
            return;
        }
        if st.has(Status::Scared) {
            if let Some(np) = self.flee_step(mpos) {
                self.monsters[i].pos = np;
                return;
            }
            // 追い詰められたら戦う
        }
        let lost = st.has(Status::Confused) || st.has(Status::Blind);
        if (kind.erratic && !self.monsters[i].cancelled && self.rng.range(0, 3) == 0) || (lost && self.rng.range(0, 2) == 0) {
            if let Some(np) = self.random_free_step(mpos) {
                self.monsters[i].pos = np;
            }
            return;
        }
        let name = self.foe_name(i);
        let (dx, dy) = (self.pos.0 - mpos.0, self.pos.1 - mpos.1);
        if dx.abs() <= 1 && dy.abs() <= 1 {
            // 器用さの指輪で、攻撃をかわすことがある
            let dex = self.ring_fx().dexterity;
            if dex > 0 && self.rng.range(0, 100) < (10 * dex).min(50) {
                self.note(&format!("{name}の攻撃を身軽にかわした！"));
                return;
            }
            let bonus = (self.depth as i32 - 1) / 3;
            let raw = self.rng.range(kind.dmg.0, kind.dmg.1 + 1 + bonus);
            // 防御で減らして「最低1」にしたあとに足す。重い鎧でも呪いの代償は必ず受ける
            let curse = self.weapon_gear().and_then(|g| g.suffix).map_or(0, Suffix::damage_taken_bonus);
            let dmg = (raw - self.defense()).max(1) + curse;
            self.hp -= dmg;
            self.hit = true;
            // 見えない相手でも、殴られた向きは肌で分かる
            let from = if self.can_see_monster(i) {
                String::new()
            } else {
                format!("({}から)", compass(dx, dy))
            };
            let msg = format!(
                "{name}の攻撃！{from} {dmg}のダメージを受けた。(HP {}/{})",
                self.hp.max(0),
                self.max_hp
            );
            self.note(&msg);
            if kind.corrodes && !self.monsters[i].cancelled && self.hp > 0 {
                self.corrode_armor();
            }
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
            if kind.poisons && !self.monsters[i].cancelled && self.hp > 0 && self.monsters[i].hp > 0 && self.rng.range(0, 2) == 0 {
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
        let name = self.foe_name(i);
        // 不器用な指輪のせいで、空振りすることがある
        let dex = self.ring_fx().dexterity;
        if dex < 0 && self.rng.range(0, 100) < (-10 * dex).min(30) {
            return format!("{name}への攻撃は空を切った。");
        }
        let (lo, hi) = self.attack_range();
        let dmg = self.rng.range(lo, hi + 1);
        let seen = self.can_see_monster(i);
        self.monsters[i].hp -= dmg;
        let (hp, max_hp) = {
            let m = &self.monsters[i];
            (m.hp, m.max_hp)
        };
        let suffix = self.weapon_gear().and_then(|g| g.suffix);
        let mut msg = if hp <= 0 {
            let m = self.monsters.remove(i);
            let xp = m.kind.xp + self.depth - 1;
            self.pending_xp += xp;
            format!("{name}に{dmg}のダメージ。{name}を倒した！ (経験値 +{xp})")
        } else {
            if seen {
                format!("{name}に{dmg}のダメージを与えた。(HP {hp}/{max_hp})")
            } else {
                // 見えない相手のHPは分からない
                format!("{name}に{dmg}のダメージを与えた。手応えがあった。")
            }
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
        self.refresh_fov();
        self.trigger_trap();
        if !self.dead {
            self.pickup_here();
            self.pass_action();
        }
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
                    || (!self.status.has(Status::Levitating) && self.known_trap_at(np).is_some())
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
        self.events.clear();
        self.status_events.clear();
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
            status_events: self.status_events.clone(),
            statuses: self.status.active(),
        }
    }

    pub fn exec(&mut self, cmd: Command) -> Outcome {
        self.events.clear();
        self.status_events.clear();
        self.detect_marks.clear();
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
                let (d, reeled) = self.confuse_dir(d);
                let lead = if reeled { format!("混乱して{}へ向かってしまった。 ", d.name()) } else { String::new() };
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                if let Some(i) = self.monster_at(t) {
                    (true, format!("{lead}{}", self.attack_monster(i)), true)
                } else if self.map.tile(t.0, t.1).walkable() {
                    self.pos = t;
                    self.refresh_fov();
                    if self.pos == self.stairs {
                        let hint = if self.has_amulet { "ascend で登れる" } else { "descend で降りられる" };
                        (true, format!("{lead}階段の上にいる。({hint})"), true)
                    } else {
                        (true, format!("{lead}{}へ進んだ。", d.name()), true)
                    }
                } else if reeled {
                    // 混乱してぶつかったときは、ターンを使う
                    (true, format!("{lead}壁にぶつかった。"), true)
                } else {
                    (false, "壁にぶつかった。".to_string(), false)
                }
            }
            Command::Attack(d) => {
                let (d, reeled) = self.confuse_dir(d);
                let lead = if reeled { format!("混乱して{}を攻撃してしまった。 ", d.name()) } else { String::new() };
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                match self.monster_at(t) {
                    Some(i) => (true, format!("{lead}{}", self.attack_monster(i)), true),
                    None if reeled => (true, format!("{lead}空振りした。"), true),
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
            Command::Zap(letter, target) => self.zap_cmd(letter, target),
            Command::Refill(flask) => self.refill_cmd(flask),
            Command::Equip(letter) => self.equip(letter),
            Command::Unequip(letter) => self.unequip(letter),
            Command::Drop(letter, n) => self.drop_cmd(letter, n),
            Command::Pickup(n) => self.pickup_cmd(n),
            Command::Inventory => {
                let lines = self.inventory_lines();
                let msg = if lines.is_empty() {
                    format!("持ち物はない。 {}", self.light_line())
                } else {
                    format!("持ち物: {} / {}", lines.join(" / "), self.light_line())
                };
                (true, msg, false)
            }
            Command::Look => (true, self.describe_surroundings(), false),
            Command::Disarm(d) => self.disarm(d),
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
        // 歩いたり転移したりして着いた場所の罠を踏み、アイテムを拾う。
        // 自動移動は1歩ごとに step_to で済ませているので、ここでもう一度発動させない
        let auto = matches!(cmd, Command::Explore | Command::Travel(_));
        if self.pos != pos_before && !self.dead && !auto {
            self.trigger_trap();
            if !self.dead {
                self.pickup_here();
            }
        }
        if spent && !self.dead {
            self.pass_action();
        }
        // 眠りや停止の間は行動できないので、解けるまで時間が過ぎる
        while self.incapacitated() && !self.dead {
            self.pass_turn();
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

    /// 自動移動できない状態（混乱・盲目）なら、その理由。
    fn refuse_auto_move(&self) -> Option<String> {
        if self.status.has(Status::Confused) {
            Some("混乱していて自動移動できない。".to_string())
        } else if self.status.has(Status::Blind) {
            Some("目が見えなくて自動移動できない。".to_string())
        } else {
            None
        }
    }

    fn travel_to_stairs(&mut self) -> (bool, String) {
        if let Some(m) = self.refuse_auto_move() {
            return (false, m);
        }
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
                    format!("階段へ向かう途中({n}歩)、進路上に{}がいる。", self.foe_name(i)),
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
        if self.incapacitated() {
            return Some("動けなくなって中断した。".to_string());
        }
        if hit {
            return Some("攻撃を受けて中断した。".to_string());
        }
        if let Some(a) = alert {
            return Some(a);
        }
        self.visible_monster_indices()
            .first()
            .map(|&i| format!("{}が現れて中断した。", self.foe_name(i)))
    }

    fn explore(&mut self) -> (bool, String) {
        if let Some(m) = self.refuse_auto_move() {
            return (false, m);
        }
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
                    format!("{steps}歩探索したが、進路上に{}がいる。", self.foe_name(i)),
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
        if self.status.has(Status::Blind) {
            let mut s = "目が見えない。周りの様子は分からない。".to_string();
            let under = self.underfoot_text();
            if !under.is_empty() {
                s.push_str(&format!(" {under}。"));
            }
            return s;
        }
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
            let fx = if e.statuses.is_empty() {
                String::new()
            } else {
                format!("[{}]", e.statuses.iter().map(|(s, n)| format!("{}{n}", s.name())).collect::<Vec<_>>().join(" "))
            };
            parts.push(format!(
                "{}(HP {}/{}){fx}が{}にいる。",
                e.name,
                e.hp,
                e.max_hp,
                rel_text(self.pos, e.pos)
            ));
        }
        for t in self.traps.iter().filter(|t| t.revealed) {
            if t.pos != self.pos {
                parts.push(format!("{} {}が{}にある。", crate::trap::TRAP_GLYPH, t.kind.name(), rel_text(self.pos, t.pos)));
            }
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
        let blind = self.status.has(Status::Blind);
        if let Some(i) = self.monster_at((x, y)).filter(|&i| self.can_see_monster(i)) {
            return Cell {
                ch: self.foe_glyph(i),
                visible: true,
                seen: true,
            };
        }
        if let Some(&(_, ch)) = self.detect_marks.iter().find(|(p, _)| *p == (x, y)) {
            return Cell { ch, visible: false, seen: true };
        }
        let seen = self.map.is_seen(x, y);
        if blind {
            // 目が見えない間は、覚えている地形だけ（物も敵も分からない）
            return Cell {
                ch: match self.map.tile(x, y) {
                    _ if !seen => ' ',
                    Tile::Stairs if self.has_amulet => '<',
                    t => t.glyph(),
                },
                visible: false,
                seen,
            };
        }
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
        if seen && self.known_trap_at((x, y)).is_some() {
            return Cell {
                ch: crate::trap::TRAP_GLYPH,
                visible: self.map.is_visible(x, y),
                seen,
            };
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
        if self.status.has(Status::Blind) {
            s.push_str("-- マップ --\n(目が見えない！ マップも敵も見えない。壁にぶつかっても分からない。盲目が治るまで待つか、手探りで動く)\n");
        } else {
            for line in self.map_lines() {
                s.push_str(line.trim_end());
                s.push('\n');
            }
        }
        let active = self.status.active();
        if !active.is_empty() {
            s.push_str("-- 状態 --\n");
            for (st, n) in active {
                s.push_str(&format!("{}: 残り{n}ターン — {}\n", st.name(), st.def().effect));
            }
        }
        let enemies = self.visible_enemies();
        if !enemies.is_empty() {
            s.push_str("-- 見えている敵 --\n");
            if self.status.has(Status::Hallucinating) {
                s.push_str("(幻覚中: 名前と記号はでたらめ。HPと位置は本物)\n");
            }
            for e in enemies {
                let fx = if e.statuses.is_empty() {
                    String::new()
                } else {
                    format!(
                        " [{}]",
                        e.statuses.iter().map(|(s, n)| format!("{}{n}", s.name())).collect::<Vec<_>>().join(" ")
                    )
                };
                s.push_str(&format!(
                    "{} {} HP {}/{}{fx} ({})\n",
                    e.glyph,
                    e.name,
                    e.hp,
                    e.max_hp,
                    rel_text(self.pos, e.pos)
                ));
            }
        } else if self.status.has(Status::Blind) {
            s.push_str("-- 見えている敵 --\n(目が見えないので敵の有無は分からない)\n");
        }
        if !self.detect_marks.is_empty() {
            s.push_str("-- 探知した敵 (地図に表示中。次の行動で消える) --\n");
            for (p, ch) in &self.detect_marks {
                s.push_str(&format!("{ch} ({})\n", rel_text(self.pos, *p)));
            }
        }
        let under = self.underfoot_text();
        if !under.is_empty() {
            s.push_str(&format!("-- {under} --\n"));
        }
        s.push_str("-- 持ち物 --\n");
        s.push_str(&self.light_line());
        s.push('\n');
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
            status: StatusSet::default(),
            cancelled: false,
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
        g.traps.clear();
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
        Gear { kind, quality, word: quality.words()[0], bonus, suffix: None, identified: true, worn: 0, enchant: 0, protected: false, freed: false }
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
            enchant: 0,
            protected: false,
            freed: false,
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
        assert_eq!(g.poison(), 0);
        assert!(g.hp < 1000, "攻撃自体は受ける");
        g.take(ItemKind::PoisonShroom);
        let l = g.inventory.iter().find(|s| s.kind == ItemKind::PoisonShroom).unwrap().letter;
        let o = g.run(&format!("eat {l}"));
        assert_eq!(g.poison(), 0, "{}", o.message);
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
            g.status.clear(Status::Poisoned);
            g.run("wait");
            assert_eq!(g.poison(), 0, "倒された毒グモが毒を撒いた");
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
            g.inventory.push(Stack { letter: c, kind: ItemKind::Dagger, count: 1, gear: Some(Gear::plain(ItemKind::Dagger)), tool: None });
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
            g.inventory.push(Stack { letter: c, kind: ItemKind::Dagger, count: 1, gear: Some(Gear::plain(ItemKind::Dagger)), tool: None });
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
    fn cataclysm_adds_one_to_every_hit_taken_even_through_armor() {
        let run = |cursed: bool| {
            let mut g = with_adjacent(3, &crate::monster::BAT);
            let a = give(&mut g, quality_gear(ItemKind::Plate, crate::item::Quality::Ancient, 5));
            g.run(&format!("equip {a}"));
            if cursed {
                let w = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
                g.run(&format!("equip {w}"));
            }
            let mut hits = Vec::new();
            for _ in 0..30 {
                let before = g.hp;
                g.run("wait");
                if g.hp < before {
                    hits.push(before - g.hp);
                }
            }
            hits
        };
        let plain = run(false);
        let cursed = run(true);
        // 防御8 の鎧ならコウモリの攻撃は最低の 1 まで減る。呪いがあるとその最低が 2 になる
        assert!(!plain.is_empty() && !cursed.is_empty());
        assert_eq!(*plain.iter().min().unwrap(), 1);
        assert_eq!(*cursed.iter().min().unwrap(), 2);
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
        g.status.apply(Status::Poisoned, 3);
        let mut text = String::new();
        for _ in 0..3 {
            text.push_str(&g.run("wait").message);
        }
        assert_eq!(g.hp(), 7);
        assert!(text.contains("毒で1ダメージ"), "{text}");
        assert!(text.contains("毒が抜けた"), "{text}");
        assert_eq!(g.poison(), 0);
        // 毒の間は自然回復しない
        g.hp = 20;
        g.status.apply(Status::Poisoned, 15);
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
        g.status.apply(Status::Poisoned, 5);
        let o = g.run("wait");
        assert!(g.is_dead() && o.message.contains("毒で力尽きた"), "{}", o.message);

        let mut g = with_gear(&[ItemKind::Healing]);
        g.status.apply(Status::Poisoned, 9);
        let o = g.run("quaff a");
        assert!(o.message.contains("毒が抜けた"), "{}", o.message);
        assert_eq!(g.poison(), 0);
    }

    #[test]
    fn antidote_cures_poison_and_is_harmless_otherwise() {
        let mut g = with_gear(&[ItemKind::Antidote, ItemKind::Antidote]);
        g.status.apply(Status::Poisoned, 9);
        let o = g.run("quaff a");
        assert!(o.ok && o.message.contains("毒が抜けた"), "{}", o.message);
        assert_eq!(g.poison(), 0);
        let o = g.run("quaff a");
        assert!(o.ok && o.message.contains("毒にはかかっていなかった"), "{}", o.message);
    }

    #[test]
    fn experience_potion_grants_xp_and_can_level_up() {
        let mut g = with_gear(&[ItemKind::Experience]);
        let (level, max_hp) = (g.level(), g.max_hp);
        let o = g.run("quaff a");
        assert!(o.ok && o.message.contains("レベルが上がった"), "{}", o.message);
        assert_eq!(g.level(), level + 1);
        // 次のレベルに届いたちょうどの経験値になる
        assert_eq!(g.xp(), 5 * level * (level + 1));
        assert_eq!(g.max_hp, max_hp + HP_PER_LEVEL);
    }

    #[test]
    fn bad_potions_are_at_most_a_third_of_potion_weight() {
        let potions: Vec<ItemKind> = ItemKind::ALL.into_iter().filter(|k| k.is_potion()).collect();
        let total: u32 = potions.iter().map(|k| k.weight()).sum();
        let bad: u32 = potions.iter().filter(|k| k.is_bad()).map(|k| k.weight()).sum();
        assert!(bad * 3 <= total, "{bad}/{total}");
        assert!(potions.len() <= crate::item::POTION_LOOKS.len());
    }

    #[test]
    fn poison_stops_auto_walk_when_hp_is_low() {
        let mut g = quiet(2);
        g.hp = DANGER_HP + 2;
        g.status.apply(Status::Poisoned, 10);
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
                assert!(g.poison() > 0 && g.food < 200, "{}", o.message);
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
        assert!(g.poison() >= 7, "{}", g.poison());
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
                assert!(g.poison() > 0);
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
            g.traps.clear();
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
        g.traps.clear();
        g
    }

    #[test]
    fn looks_are_unique_and_stable_per_seed() {
        let a = Game::new(9);
        let b = Game::new(9);
        assert_eq!(a.looks, b.looks);
        for k in ItemKind::ALL {
            assert_eq!(a.known[k.index()], k.starts_known(), "{k:?}");
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
            g.traps.clear();
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
        g.status.apply(Status::Poisoned, 4);
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
            g.inventory.push(Stack { letter: c, kind, count: 1, gear: kind.is_equipment().then(|| Gear::plain(kind)), tool: None });
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

    // ---- 状態異常 ----

    #[test]
    fn confusion_sometimes_sends_you_the_wrong_way_and_a_stumble_costs_a_turn() {
        let mut g = quiet(1);
        g.status.apply(Status::Confused, 1000);
        let (mut reeled, mut straight) = (0, 0);
        for _ in 0..60 {
            let before = g.turn();
            let o = g.run("move east");
            if o.message.contains("混乱して") {
                reeled += 1;
                // 向きがずれたときは、壁にぶつかってもターンを使って成功扱い
                assert!(o.ok && g.turn() > before, "{}", o.message);
            } else {
                straight += 1;
            }
        }
        assert!(reeled > 10 && straight > 10, "{reeled} {straight}");
        // 混乱していなければ、ずれない
        let mut g = quiet(1);
        for _ in 0..30 {
            assert!(!g.run("move east").message.contains("混乱して"));
        }
    }

    #[test]
    fn confusion_and_blindness_forbid_auto_walk() {
        let mut g = quiet(1);
        g.status.apply(Status::Confused, 5);
        let o = g.run("explore");
        assert!(!o.ok && o.message.contains("混乱"), "{}", o.message);
        assert!(!g.run("travel >").ok);
        let mut g = quiet(1);
        g.status.apply(Status::Blind, 5);
        let o = g.run("explore");
        assert!(!o.ok && o.message.contains("見え"), "{}", o.message);
    }

    #[test]
    fn blindness_hides_the_map_and_enemies_and_says_so() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        let seen_before = (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).filter(|&(x, y)| g.map.is_seen(x, y)).count();
        g.inflict(Status::Blind, 30);
        let text = g.observe_text(5);
        assert!(text.contains("目が見えない"), "{text}");
        assert!(text.contains("盲目"), "{text}");
        // マップの行も、敵の一覧も出ない
        assert!(!text.contains('@') || !text.contains("###"), "{text}");
        assert!(!text.contains("ゴブリン"), "{text}");
        assert!(g.visible_enemies().is_empty());
        // 襲われても正体は分からない
        let o = g.run("wait");
        assert!(o.message.contains("何かの攻撃！"), "{}", o.message);
        assert!(!o.message.contains("ゴブリン"), "{}", o.message);
        // 殴られた向きは分かる
        assert!(o.message.contains("(東から)"), "{}", o.message);
        // 見えない相手のHPは分からない
        let o = g.run("attack east");
        assert!(o.message.contains("手応えがあった") && !o.message.contains("与えた。(HP"), "{}", o.message);
        // 手探りで動いても、新しい場所は覚えない
        for _ in 0..5 {
            g.run("move west");
        }
        let seen_after = (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).filter(|&(x, y)| g.map.is_seen(x, y)).count();
        assert_eq!(seen_before, seen_after);
        assert!(g.run("look").message.contains("目が見えない"));
    }

    #[test]
    fn sight_returns_when_blindness_ends() {
        let mut g = quiet(2);
        g.inflict(Status::Blind, 3);
        assert!(!g.map.is_visible(g.pos.0, g.pos.1));
        for _ in 0..3 {
            g.run("wait");
        }
        assert!(!g.status.has(Status::Blind));
        assert!(g.map.is_visible(g.pos.0, g.pos.1));
        assert!(g.log().iter().any(|l| l.text.contains("目が見えるようになった")));
    }

    #[test]
    fn hallucination_scrambles_names_and_glyphs_but_not_hp_or_position() {
        let mut g = with_adjacent(3, &crate::monster::OGRE);
        g.monsters[0].hp = 7;
        g.monsters[0].max_hp = 10;
        g.inflict(Status::Hallucinating, 1000);
        let mut names = std::collections::HashSet::new();
        let mut glyphs = std::collections::HashSet::new();
        for _ in 0..40 {
            let e = &g.visible_enemies()[0];
            assert_eq!((e.hp, e.max_hp, e.pos), (7, 10, (g.pos.0 + 1, g.pos.1)));
            names.insert(e.name);
            glyphs.insert(e.glyph);
            g.turn += 1; // ターンが進むと見え方が変わる
        }
        assert!(names.len() >= 3 && glyphs.len() >= 3, "{names:?} {glyphs:?}");
        // 同じターンの見え方は安定していて、観測が乱数を消費しない
        let a = g.observe_text(3);
        assert_eq!(a, g.observe_text(3));
        assert!(g.observe_text(3).contains("幻覚"));
        // 一覧と地図の記号は、同じでたらめを指している
        let e = &g.visible_enemies()[0];
        assert_eq!(g.cell(e.pos.0, e.pos.1).ch, e.glyph);
    }

    #[test]
    fn observing_never_changes_what_happens() {
        let play = |observe: bool| {
            let mut g = with_adjacent(5, &crate::monster::GOBLIN);
            g.inflict(Status::Hallucinating, 100);
            let mut out = Vec::new();
            for _ in 0..10 {
                if observe {
                    let _ = g.observe_text(5);
                    let _ = g.visible_enemies();
                }
                out.push(g.run("attack east").message);
            }
            out
        };
        assert_eq!(play(true), play(false));
    }

    #[test]
    fn sleep_and_paralysis_pass_time_until_they_wear_off() {
        let mut g = quiet(1);
        g.inflict(Status::Paralyzed, 6);
        let t = g.turn();
        let o = g.run("wait");
        assert_eq!(g.turn() - t, 6, "{}", o.message);
        assert!(o.message.contains("体が動くようになった"), "{}", o.message);
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.inflict(Status::Asleep, 4);
        let hp = g.hp;
        let o = g.run("wait");
        assert!(g.hp < hp && o.message.contains("目が覚めた"), "{}", o.message);
    }

    #[test]
    fn haste_halves_and_slow_doubles_the_cost_of_actions() {
        let mut g = quiet(1);
        g.status.apply(Status::Hasted, 1000);
        let t = g.turn();
        for _ in 0..10 {
            g.run("wait");
        }
        assert_eq!(g.turn() - t, 5);
        let mut g = quiet(1);
        g.status.apply(Status::Slowed, 1000);
        let t = g.turn();
        for _ in 0..10 {
            g.run("wait");
        }
        assert_eq!(g.turn() - t, 20);
        // 打ち消し合う
        let mut g = quiet(1);
        g.status.apply(Status::Slowed, 1000);
        g.status.apply(Status::Hasted, 1000);
        let t = g.turn();
        for _ in 0..10 {
            g.run("wait");
        }
        assert_eq!(g.turn() - t, 10);
    }

    #[test]
    fn statuses_count_down_each_turn_and_announce_the_end() {
        let mut g = quiet(1);
        g.inflict(Status::Hasted, 3);
        g.inflict(Status::Levitating, 2);
        assert_eq!(g.status.short_text(), "加速3 浮遊2");
        let o = g.run("stay 1");
        assert_eq!(o.statuses, vec![(Status::Hasted, 2), (Status::Levitating, 1)]);
        let o = g.run("stay 1");
        assert!(o.message.contains("浮遊が切れて"), "{}", o.message);
        assert!(o.status_events.iter().any(|e| e.status == Status::Levitating && e.change == Change::End));
        assert!(g.observe_text(3).contains("加速: 残り1ターン"));
    }

    #[test]
    fn outcome_carries_status_changes_for_the_record() {
        use crate::record::Event;
        let mut g = quiet(1);
        g.take(ItemKind::Sleep);
        let o = g.run("quaff a");
        assert!(o.status_events.iter().any(|e| e.target == "player"
            && e.status == Status::Asleep
            && matches!(e.change, Change::Apply(_))), "{:?}", o.status_events);
        assert!(o.status_events.iter().any(|e| e.status == Status::Asleep && e.change == Change::End));
        let line = Event::from_outcome(&o, None).to_line();
        assert!(line.contains("\"status_events\"") && line.contains("asleep") && line.contains("apply"), "{line}");
        assert_eq!(Event::parse(&line), Ok(Event::from_outcome(&o, None)));
        // 状態のないコマンドは、余計な欄を作らない
        let o = g.run("wait");
        assert!(!Event::from_outcome(&o, None).to_line().contains("status"));
    }

    fn put_trap(g: &mut Game, kind: TrapKind) -> (i32, i32) {
        let p = (g.pos.0 + 1, g.pos.1);
        assert!(g.map.tile(p.0, p.1).walkable());
        g.traps.push(Trap { pos: p, kind, revealed: false });
        p
    }

    #[test]
    fn traps_are_hidden_until_stepped_on_and_then_shown_on_the_map() {
        let mut g = quiet(3);
        let p = put_trap(&mut g, TrapKind::Dart);
        assert_ne!(g.cell(p.0, p.1).ch, '^');
        let hp = g.hp;
        let o = g.run("move east");
        assert!(o.message.contains("毒矢"), "{}", o.message);
        assert!(g.hp < hp && g.status.has(Status::Poisoned));
        g.run("move west");
        assert_eq!(g.cell(p.0, p.1).ch, '^');
        assert!(g.run("look").message.contains("毒矢の罠"));
    }

    #[test]
    fn trapdoor_drops_you_a_floor_and_levitation_ignores_traps() {
        let mut g = quiet(3);
        put_trap(&mut g, TrapKind::Trapdoor);
        let o = g.run("move east");
        assert_eq!(g.depth(), 2, "{}", o.message);
        assert!(o.message.contains("落とし穴に落ちた"), "{}", o.message);
        let mut g = quiet(3);
        let p = put_trap(&mut g, TrapKind::Trapdoor);
        g.inflict(Status::Levitating, 20);
        let o = g.run("move east");
        assert_eq!((g.depth(), g.pos()), (1, p), "{}", o.message);
        assert!(!o.message.contains("落ちた"));
        // 眠りガス
        let mut g = quiet(3);
        put_trap(&mut g, TrapKind::SleepGas);
        let t = g.turn();
        let o = g.run("move east");
        assert!(g.turn() - t >= 5 && o.message.contains("目が覚めた"), "{}", o.message);
    }

    #[test]
    fn trapdoors_are_never_placed_where_they_cannot_drop() {
        let mut g = quiet(4);
        g.depth = AMULET_DEPTH;
        for _ in 0..30 {
            g.spawn_traps();
            assert!(g.traps.iter().all(|t| t.kind != TrapKind::Trapdoor));
        }
        let mut g = quiet(4);
        g.has_amulet = true;
        for _ in 0..30 {
            g.spawn_traps();
            assert!(g.traps.iter().all(|t| t.kind != TrapKind::Trapdoor));
        }
    }

    #[test]
    fn standing_next_to_a_trap_can_reveal_it() {
        let mut g = quiet(3);
        let p = put_trap(&mut g, TrapKind::Dart);
        for _ in 0..60 {
            g.run("wait");
        }
        assert!(g.traps[0].revealed, "60ターン隣に立って見つからなかった");
        assert_eq!(g.cell(p.0, p.1).ch, '^');
    }

    #[test]
    fn explore_walks_around_a_known_trap() {
        let mut g = quiet(3);
        let p = put_trap(&mut g, TrapKind::Trapdoor);
        g.traps[0].revealed = true;
        for _ in 0..40 {
            let o = g.run("explore");
            if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                break;
            }
        }
        assert_eq!(g.depth(), 1);
        assert_ne!(g.pos(), p, "既知の罠のマスに乗った");
    }

    #[test]
    fn a_hidden_trap_stepped_on_while_exploring_fires_exactly_once() {
        let mut fired = 0;
        for seed in 0..40 {
            let mut g = quiet(seed);
            g.hp = 1000;
            g.max_hp = 1000;
            // 開始位置から少し離れた床に、毒矢の罠を隠す
            let Some(p) = (1..W - 1)
                .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
                .find(|&(x, y)| g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs().max((y - g.pos.1).abs()) == 4)
            else {
                continue;
            };
            g.traps.push(Trap { pos: p, kind: TrapKind::Dart, revealed: false });
            let mut msgs = String::new();
            for _ in 0..30 {
                let o = g.run("explore");
                msgs.push_str(&o.message);
                if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                    break;
                }
            }
            let n = msgs.matches("毒矢の罠だ").count();
            assert!(n <= 1, "seed {seed}: {n}回発動した");
            fired += n;
        }
        assert!(fired > 0, "どのseedでも罠を踏まなかった");
    }

    #[test]
    fn an_invisible_player_is_only_noticed_up_close() {
        // 東に4マス歩ける床がある seed を探す
        let (mut g, far) = (3..200)
            .map(|seed| with_adjacent(seed, &crate::monster::GOBLIN))
            .find_map(|g| {
                let far = (g.pos.0 + 4, g.pos.1);
                g.map.tile(far.0, far.1).walkable().then_some((g, far))
            })
            .expect("条件に合う seed がある");
        g.hp = 1000;
        g.monsters[0].pos = far;
        assert!(g.monster_aware(0));
        g.status.apply(Status::Invisible, 100);
        assert!(!g.monster_aware(0));
        g.monsters[0].pos = (g.pos.0 + 2, g.pos.1);
        assert!(g.monster_aware(0));
    }

    #[test]
    fn a_paralyzed_or_sleeping_monster_does_not_act() {
        for st in [Status::Paralyzed, Status::Asleep] {
            let mut g = with_adjacent(3, &crate::monster::GOBLIN);
            let hp = g.hp;
            assert!(g.inflict_monster(0, st, 5));
            g.run("wait");
            g.run("wait");
            assert_eq!(g.hp, hp, "{st:?}");
            // 切れたら殴ってくる
            for _ in 0..6 {
                g.run("wait");
            }
            assert!(g.hp < hp, "{st:?}");
        }
    }

    #[test]
    fn a_scared_monster_flees_and_a_cornered_one_fights() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.inflict_monster(0, Status::Scared, 50);
        let hp = g.hp;
        let d0 = (g.monsters[0].pos.0 - g.pos.0).abs();
        for _ in 0..3 {
            g.run("wait");
        }
        let d1 = (g.monsters[0].pos.0 - g.pos.0).abs().max((g.monsters[0].pos.1 - g.pos.1).abs());
        assert!(d1 > d0 && g.hp == hp, "{d0} {d1}");
        // 逃げ場がなければ戦う
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.inflict_monster(0, Status::Scared, 50);
        let m = g.monsters[0].pos;
        for d in Dir::ALL {
            let (dx, dy) = d.delta();
            let n = (m.0 + dx, m.1 + dy);
            if n != g.pos && g.map.tile(n.0, n.1).walkable() {
                g.monsters.push(monster(&crate::monster::SLIME, n, 1000));
                g.inflict_monster(g.monsters.len() - 1, Status::Paralyzed, 100);
            }
        }
        let hp = g.hp;
        g.run("wait");
        assert!(g.hp < hp);
    }

    #[test]
    fn poison_kills_monsters_and_gives_experience() {
        let mut g = with_adjacent(3, &crate::monster::SLIME);
        g.monsters[0].hp = 2;
        g.monsters[0].max_hp = 2;
        g.inflict_monster(0, Status::Poisoned, 10);
        let xp = g.xp();
        let hp = g.hp;
        g.run("wait");
        g.run("wait");
        assert!(g.monsters.is_empty());
        assert!(g.xp() > xp && g.hp >= hp - 3);
        assert!(g.log().iter().any(|l| l.text.contains("毒で倒れた")));
    }

    #[test]
    fn hasted_monsters_act_twice_and_slowed_ones_every_other_turn() {
        let hits = |st: Option<Status>| {
            let mut g = with_adjacent(3, &crate::monster::GOBLIN);
            if let Some(s) = st {
                g.inflict_monster(0, s, 1000);
            }
            let mut n = 0;
            for _ in 0..20 {
                n += g.run("wait").message.matches("の攻撃！").count();
            }
            n
        };
        let (base, fast, slow) = (hits(None), hits(Some(Status::Hasted)), hits(Some(Status::Slowed)));
        assert_eq!(base, 20);
        assert_eq!(fast, 40);
        assert_eq!(slow, 10);
    }

    #[test]
    fn statuses_that_make_no_sense_for_monsters_are_refused() {
        let mut g = with_adjacent(3, &crate::monster::SLIME);
        assert!(!g.inflict_monster(0, Status::Hallucinating, 5));
        assert!(!g.inflict_monster(0, Status::Levitating, 5));
        assert!(g.monsters[0].status.active().is_empty());
        assert!(g.inflict_monster(0, Status::Confused, 5));
        assert_eq!(g.visible_enemies()[0].statuses, vec![(Status::Confused, 5)]);
        assert!(g.observe_text(3).contains("[混乱5]"));
    }

    #[test]
    fn poison_is_a_status_like_the_others() {
        let mut g = quiet(1);
        g.try_poison(5);
        assert!(g.observe_text(3).contains("毒: 残り5ターン"));
        let o = g.run("stay 1");
        assert_eq!(o.statuses, vec![(Status::Poisoned, 4)]);
    }

    #[test]
    fn a_blind_player_cannot_read_scrolls_but_can_still_drink() {
        let mut g = quiet(1);
        g.take(ItemKind::Teleport);
        g.take(ItemKind::Healing);
        g.inflict(Status::Blind, 20);
        let t = g.turn();
        let o = g.run("read a");
        assert!(!o.ok && o.message.contains("読めない") && g.turn() == t, "{}", o.message);
        assert!(g.run("quaff b").ok);
    }

    // ---- 段階2: 薬と巻物 ----

    /// その物を1つ持って、静かな場所にいるゲーム（文字は a）。
    fn holding(kind: ItemKind) -> Game {
        let mut g = quiet(2);
        g.hp = 10;
        g.take(kind);
        g
    }

    fn use_item(g: &mut Game, letter: char) -> Outcome {
        let kind = g.inventory.iter().find(|s| s.letter == letter).unwrap().kind;
        g.run(&format!("{} {letter}", if kind.is_scroll() { "read" } else { "quaff" }))
    }

    #[test]
    fn the_original_rogue_variety_of_potions_and_scrolls_exists() {
        let count = |f: fn(ItemKind) -> bool| ItemKind::ALL.iter().filter(|k| f(**k)).count();
        assert!(count(|k| k.is_potion()) >= 16);
        assert!(count(|k| k.is_scroll()) >= 13);
        for name in [
            "回復の薬", "大回復の薬", "力の薬", "レベルアップの薬", "力回復の薬", "加速の薬", "モンスター探知の薬",
            "アイテム探知の薬", "透明視認の薬", "浮遊の薬", "混乱の薬", "幻覚の薬", "毒の薬", "盲目の薬", "眠りの薬",
            "識別の巻物", "転移の巻物", "地図の巻物", "武器強化の巻物", "防具強化の巻物", "呪い解除の巻物",
            "防具保護の巻物", "モンスター混乱の巻物", "モンスター停止の巻物", "睡眠の巻物", "怯えの巻物",
            "モンスター生成の巻物", "怒りの巻物",
        ] {
            assert!(ItemKind::ALL.iter().any(|k| k.true_name() == name), "{name}");
        }
        // 見た目が足りている
        assert!(count(|k| k.is_potion()) <= crate::item::POTION_LOOKS.len());
        assert!(count(|k| k.is_scroll()) <= crate::item::SCROLL_LOOKS.len());
    }

    #[test]
    fn every_potion_and_scroll_can_be_used_and_identifies_itself_the_same_way() {
        for kind in ItemKind::ALL.into_iter().filter(|k| k.is_potion() || k.is_scroll()) {
            for seed in [1u64, 2, 3] {
                let mut g = with_adjacent(seed, &crate::monster::GOBLIN);
                g.take(kind);
                // 識別の巻物の対象になる未知の物も用意する
                g.take(ItemKind::Bread);
                let before = g.display_name(kind);
                let o = use_item(&mut g, 'a');
                assert!(o.ok || kind == ItemKind::Identify, "{kind:?}: {}", o.message);
                if !o.ok {
                    continue;
                }
                // 未識別のときは「見た目を使った。これは本物の名前だった！」の形に揃っている
                assert!(
                    o.message.starts_with(&format!("{before}を")) && o.message.contains(&format!("これは{}だった！", kind.true_name())),
                    "{kind:?}: {}",
                    o.message
                );
                assert!(g.known[kind.index()]);
                assert!(g.inventory.iter().all(|s| s.kind != kind), "{kind:?} が残っている");
                // 2個目は本名で出る
                g.take(kind);
                if !g.dead && !g.incapacitated() {
                    let letter = g.inventory.iter().find(|s| s.kind == kind).unwrap().letter;
                    let o = use_item(&mut g, letter);
                    if o.ok {
                        assert!(o.message.starts_with(&format!("{}を", kind.true_name())), "{kind:?}: {}", o.message);
                        assert!(!o.message.contains("これは"), "{}", o.message);
                    }
                }
            }
        }
    }

    #[test]
    fn extra_healing_raises_max_hp_when_it_overflows() {
        let mut g = holding(ItemKind::ExtraHealing);
        g.max_hp = 20;
        g.hp = 20;
        let o = use_item(&mut g, 'a');
        assert!(o.message.contains("最大HPが2増えた"), "{}", o.message);
        assert_eq!((g.hp, g.max_hp), (22, 22));
        // 30 以上足りないときは、全部回復に使われて最大HPは増えない
        let mut g = holding(ItemKind::ExtraHealing);
        g.max_hp = 50;
        g.hp = 10;
        use_item(&mut g, 'a');
        assert_eq!((g.hp, g.max_hp), (40, 50));
    }

    #[test]
    fn strength_changes_attack_and_can_be_drained_and_restored() {
        let mut g = holding(ItemKind::Strength);
        let (lo, hi) = g.attack_range();
        use_item(&mut g, 'a');
        assert_eq!((g.strength, g.max_strength), (11, 11));
        g.take(ItemKind::Strength);
        use_item(&mut g, 'a');
        assert_eq!(g.attack_range(), (lo + 1, hi + 1));
        // 毒の薬で腕力が下がり、力回復の薬で戻る
        g.take(ItemKind::Poison);
        g.hp = 20;
        g.max_hp = 20;
        let o = use_item(&mut g, 'a');
        assert_eq!(g.strength, 10, "{}", o.message);
        assert!(o.message.contains("力が抜けた"), "{}", o.message);
        g.take(ItemKind::RestoreStrength);
        let o = use_item(&mut g, 'a');
        assert_eq!(g.strength, 12, "{}", o.message);
        assert!(g.observe_text(3).contains("腕力 12/12"));
        // 下限
        g.strength = 3;
        g.take(ItemKind::Poison);
        g.hp = 20;
        use_item(&mut g, 'a');
        assert_eq!(g.strength, 3);
    }

    #[test]
    fn status_potions_apply_their_status_for_the_listed_turns() {
        for (kind, st) in [
            (ItemKind::Haste, Status::Hasted),
            (ItemKind::SeeInvisible, Status::SeeInvisible),
            (ItemKind::Levitation, Status::Levitating),
            (ItemKind::Confusion, Status::Confused),
            (ItemKind::Hallucination, Status::Hallucinating),
            (ItemKind::Blindness, Status::Blind),
        ] {
            let mut g = holding(kind);
            let o = use_item(&mut g, 'a');
            assert!(g.status.has(st), "{kind:?} {}", o.message);
            assert!(o.status_events.iter().any(|e| e.status == st && matches!(e.change, Change::Apply(_))));
            assert!(o.message.contains(st.def().start), "{}", o.message);
        }
    }

    #[test]
    fn a_blindness_potion_blinds_and_the_observation_says_so() {
        let mut g = holding(ItemKind::Blindness);
        use_item(&mut g, 'a');
        let t = g.observe_text(3);
        assert!(t.contains("目が見えない") && t.contains("盲目: 残り"), "{t}");
    }

    #[test]
    fn detect_monsters_marks_them_on_the_map_until_the_next_command() {
        let mut g = holding(ItemKind::DetectMonsters);
        g.monsters.push(monster(&crate::monster::GOBLIN, (g.pos.0 + 30, g.pos.1), 9));
        let far = g.monsters[0].pos;
        assert!(!g.map.is_visible(far.0, far.1));
        let o = use_item(&mut g, 'a');
        assert!(o.message.contains("ゴブリン"), "{}", o.message);
        assert_eq!(g.cell(far.0, far.1).ch, 'g');
        assert!(g.observe_text(3).contains("探知した敵"));
        g.run("look");
        assert_ne!(g.cell(far.0, far.1).ch, 'g');
        // 敵がいなければそう言う
        let mut g = holding(ItemKind::DetectMonsters);
        assert!(use_item(&mut g, 'a').message.contains("敵の気配はない"));
    }

    #[test]
    fn detect_items_reveals_where_things_lie_and_keeps_the_map_memory() {
        let mut g = holding(ItemKind::DetectItems);
        let p = (1..W - 1)
            .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
            .find(|&(x, y)| g.map.tile(x, y) == Tile::Floor && !g.map.is_seen(x, y))
            .unwrap();
        g.floor_items.push(FloorItem::new(p, ItemKind::Bread));
        let o = use_item(&mut g, 'a');
        assert!(o.message.contains("パン"), "{}", o.message);
        assert!(g.map.is_seen(p.0, p.1));
        assert_eq!(g.cell(p.0, p.1).ch, '%');
    }

    #[test]
    fn enchant_scrolls_strengthen_the_worn_gear_and_say_so_when_nothing_is_worn() {
        let mut g = with_gear(&[ItemKind::Sword, ItemKind::Leather, ItemKind::EnchantWeapon, ItemKind::EnchantArmor]);
        let o = g.run("read c");
        assert!(o.ok && o.message.contains("装備していない"), "{}", o.message);
        g.take(ItemKind::EnchantWeapon);
        g.run("equip a");
        g.run("equip b");
        let (lo, hi) = g.attack_range();
        let def = g.defense();
        let o = g.run("read c");
        assert!(o.message.contains("強化+1"), "{}", o.message);
        assert_eq!(g.attack_range(), (lo + 1, hi + 1));
        let o = g.run("read d");
        assert!(o.message.contains("強化+1"), "{}", o.message);
        assert_eq!(g.defense(), def + 1);
        assert!(g.inventory_lines().iter().any(|l| l.contains("防御 2")));
    }

    #[test]
    fn remove_curse_lets_you_take_off_cursed_gear_but_the_drawback_stays() {
        let mut g = quiet(2);
        let a = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Cataclysm));
        g.run(&format!("equip {a}"));
        assert!(!g.run(&format!("unequip {a}")).ok);
        g.take(ItemKind::RemoveCurse);
        let b = g.inventory.iter().find(|s| s.kind == ItemKind::RemoveCurse).unwrap().letter;
        let o = g.run(&format!("read {b}"));
        assert!(o.message.contains("呪いの束縛が解けた"), "{}", o.message);
        assert!(g.run(&format!("unequip {a}")).ok);
        assert!(g.gear_of(Some(a)).unwrap().is_cursed());
    }

    #[test]
    fn protect_armor_stops_rust_and_aquators_rust_unprotected_armor() {
        let mut g = with_adjacent(3, &crate::monster::AQUATOR);
        let a = give(&mut g, Gear::plain(ItemKind::Chain));
        g.run(&format!("equip {a}"));
        let def = g.defense();
        for _ in 0..3 {
            g.run("wait");
        }
        assert_eq!(g.defense(), def - 3);
        assert!(g.log().iter().any(|l| l.text.contains("錆びた")));
        // 保護
        let mut g = with_adjacent(3, &crate::monster::AQUATOR);
        let a = give(&mut g, Gear::plain(ItemKind::Chain));
        g.run(&format!("equip {a}"));
        g.take(ItemKind::ProtectArmor);
        let b = g.inventory.iter().find(|s| s.kind == ItemKind::ProtectArmor).unwrap().letter;
        g.run(&format!("read {b}"));
        let def = g.defense();
        for _ in 0..3 {
            g.run("wait");
        }
        assert_eq!(g.defense(), def);
        assert!(g.log().iter().any(|l| l.text.contains("錆びなかった")));
    }

    #[test]
    fn monster_scrolls_afflict_what_you_can_see() {
        for (kind, st) in [
            (ItemKind::ConfuseMonster, Status::Confused),
            (ItemKind::ScareMonster, Status::Scared),
        ] {
            let mut g = with_adjacent(3, &crate::monster::GOBLIN);
            g.take(kind);
            let o = g.run("read a");
            assert!(g.monsters[0].status.has(st), "{kind:?} {}", o.message);
            assert!(o.status_events.iter().any(|e| e.target == "ゴブリン" && e.status == st));
        }
        // 停止は近くの敵だけ
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        let far = (g.pos.0 + 5, g.pos.1);
        if g.map.tile(far.0, far.1).walkable() && g.map.is_visible(far.0, far.1) {
            g.monsters.push(monster(&crate::monster::SLIME, far, 1000));
            g.take(ItemKind::HoldMonster);
            g.run("read a");
            assert!(g.monsters[0].status.has(Status::Paralyzed));
            assert!(!g.monsters[1].status.has(Status::Paralyzed));
        }
        // 相手がいなければそう言う
        let mut g = holding(ItemKind::HoldMonster);
        assert!(use_item(&mut g, 'a').message.contains("効く相手がいなかった"));
    }

    #[test]
    fn create_monster_and_aggravate_and_slumber_are_the_bad_scrolls() {
        let mut g = holding(ItemKind::CreateMonster);
        let o = use_item(&mut g, 'a');
        assert_eq!(g.monsters.len(), 1, "{}", o.message);
        let m = g.monsters[0].pos;
        assert!((m.0 - g.pos.0).abs() <= 1 && (m.1 - g.pos.1).abs() <= 1);
        let mut g = holding(ItemKind::Aggravate);
        g.monsters.push(monster(&crate::monster::GOBLIN, (g.pos.0 + 30, g.pos.1), 9));
        use_item(&mut g, 'a');
        assert!(g.monsters[0].status.has(Status::Enraged));
        let mut g = holding(ItemKind::Slumber);
        let t = g.turn();
        let o = use_item(&mut g, 'a');
        assert!(g.turn() - t >= 6 && o.message.contains("目が覚めた"), "{}", o.message);
    }

    #[test]
    fn an_enraged_monster_hunts_you_from_anywhere() {
        let mut g = (0..60).map(quiet).find(|g| {
            (1..W - 1).any(|x| (1..H - 1).any(|y| g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs() + (y - g.pos.1).abs() > 25))
        }).expect("遠い床がある seed");
        let far = (1..W - 1)
            .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
            .find(|&(x, y)| g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs() + (y - g.pos.1).abs() > 25)
            .unwrap();
        let id = g.add_monster(&crate::monster::SLIME, far);
        assert!(!g.monster_aware(id), "遠い敵は怒っていなければ気づかない");
        g.inflict_monster(id, Status::Enraged, 100);
        assert!(g.monster_aware(id));
        let mut moved = false;
        for _ in 0..5 {
            g.run("wait");
            moved |= g.monsters[id].pos != far;
        }
        assert!(moved);
    }

    #[test]
    fn identify_finds_unknown_potions_and_scrolls_among_the_new_kinds() {
        let mut g = quiet(2);
        g.take(ItemKind::Identify);
        g.take(ItemKind::Blindness);
        let o = g.run("read a b");
        assert!(o.ok && !g.status.has(Status::Blind), "{}", o.message);
        assert!(g.known[ItemKind::Blindness.index()]);
        assert!(g.run("quaff b").message.starts_with("盲目の薬を飲んだ。"));
    }

    #[test]
    fn bad_things_stay_a_minority_of_the_floor_drops() {
        for class in [Class::Potion, Class::Scroll] {
            let all: Vec<ItemKind> = ItemKind::ALL.into_iter().filter(|k| k.class() == class).collect();
            let total: u32 = all.iter().map(|k| k.weight()).sum();
            let bad: u32 = all.iter().filter(|k| k.is_bad()).map(|k| k.weight()).sum();
            assert!(bad * 3 <= total, "{class:?} {bad}/{total}");
        }
    }

    // ---- 段階3: 杖 ----

    /// 充填数 `charges` の杖を1本持つ（文字は a）。
    fn with_wand(kind: ItemKind, charges: i32) -> Game {
        let mut g = quiet(2);
        g.hp = 20;
        g.max_hp = 20;
        g.take(Tool::charged(kind, charges));
        g
    }

    /// 東隣に敵を置いて杖を持つ。
    fn wand_vs_adjacent(kind: ItemKind, mk: &'static MonsterKind, hp: i32) -> Game {
        let mut g = with_adjacent(3, mk);
        g.monsters[0].hp = hp;
        g.monsters[0].max_hp = hp;
        g.take(Tool::charged(kind, 5));
        g
    }

    #[test]
    fn all_the_original_wand_kinds_exist() {
        for name in [
            "光の杖", "透明化の杖", "雷の杖", "火の杖", "冷気の杖", "変身の杖", "魔法の矢の杖", "敵加速の杖",
            "敵減速の杖", "生命吸収の杖", "消去の杖", "敵テレポートの杖", "自分テレポートの杖",
        ] {
            assert!(ItemKind::ALL.iter().any(|k| k.is_wand() && k.true_name() == name), "{name}");
        }
        let g = Game::new(5);
        let wands: Vec<&str> = ItemKind::ALL.iter().filter(|k| k.is_wand()).map(|k| g.looks[k.index()]).collect();
        let uniq: std::collections::HashSet<_> = wands.iter().collect();
        assert_eq!(uniq.len(), wands.len());
        assert!(wands.iter().all(|l| l.ends_with('杖')));
    }

    #[test]
    fn zapping_uses_a_charge_identifies_the_wand_and_reports_what_is_left() {
        let mut g = wand_vs_adjacent(ItemKind::WandMissile, &crate::monster::OGRE, 1000);
        let before = g.display_name(ItemKind::WandMissile);
        assert!(g.inventory_lines()[0].contains("[残り5回]") && g.inventory_lines()[0].contains("(未識別)"));
        assert!(g.status_text().contains("杖残り[a:5]"), "{}", g.status_text());
        let o = g.run("zap a east");
        assert!(o.ok, "{}", o.message);
        assert!(o.message.starts_with(&format!("{before}を振った。これは魔法の矢の杖だった！")), "{}", o.message);
        assert!(o.message.contains("(残り4回)"), "{}", o.message);
        assert!(g.monsters[0].hp < 1000);
        assert!(g.inventory_lines()[0].contains("魔法の矢の杖 [残り4回]") && !g.inventory_lines()[0].contains("未識別"));
        assert!(g.observe_text(3).contains("杖残り[a:4]"));
        // 2回目からは本名で
        let o = g.run("zap a east");
        assert!(o.message.starts_with("魔法の矢の杖を振った。") && !o.message.contains("これは"), "{}", o.message);
    }

    #[test]
    fn an_empty_wand_cannot_be_used_and_costs_nothing() {
        let mut g = wand_vs_adjacent(ItemKind::WandMissile, &crate::monster::OGRE, 1000);
        for _ in 0..4 {
            assert!(g.run("zap a east").ok);
        }
        let o = g.run("zap a east");
        assert!(o.ok && o.message.contains("魔力は尽きた") && o.message.contains("0回") == false, "{}", o.message);
        assert!(g.inventory_lines()[0].contains("[残り0回]"));
        let (t, hp) = (g.turn(), g.monsters[0].hp);
        let o = g.run("zap a east");
        assert!(!o.ok && o.message.contains("充填数が0"), "{}", o.message);
        assert_eq!((g.turn(), g.monsters[0].hp), (t, hp));
        // 捨てて拾い直しても、残りは0のまま
        assert!(g.run("drop a").ok);
        assert!(g.run("pickup").ok);
        assert!(g.inventory_lines()[0].contains("[残り0回]"));
    }

    #[test]
    fn zap_needs_a_direction_a_wand_and_a_visible_target() {
        let mut g = with_wand(ItemKind::WandFire, 3);
        let t = g.turn();
        let o = g.run("zap a");
        assert!(!o.ok && o.message.contains("向きが要る") && g.turn() == t, "{}", o.message);
        let o = g.run("zap a nearest");
        assert!(!o.ok && o.message.contains("狙える敵"), "{}", o.message);
        assert_eq!(g.tool_charges('a'), 3);
        assert!(!g.run("zap b east").ok);
        g.take(ItemKind::Bread);
        let o = g.run("zap b east");
        assert!(!o.ok && o.message.contains("杖ではない"), "{}", o.message);
        let o = g.run("quaff a");
        assert!(!o.ok && o.message.contains("zap"), "{}", o.message);
    }

    impl Game {
        fn tool_charges(&self, letter: char) -> i32 {
            self.inventory.iter().find(|s| s.letter == letter).and_then(|s| s.tool).map_or(-1, |t| t.val)
        }
    }

    #[test]
    fn bolts_hit_the_first_enemy_in_line_and_nearest_aims_by_itself() {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(Tool::charged(ItemKind::WandFire, 5));
        // 東に2体並べる
        let (p1, p2) = ((g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1));
        assert!(g.map.tile(p2.0, p2.1).walkable());
        g.monsters.push(monster(&crate::monster::OGRE, p1, 500));
        g.monsters.push(monster(&crate::monster::OGRE, p2, 500));
        g.inflict_monster(0, Status::Paralyzed, 100);
        g.inflict_monster(1, Status::Paralyzed, 100);
        g.run("zap a east");
        assert!(g.monsters[0].hp < 500 && g.monsters[1].hp == 500);
        g.run("zap a nearest");
        assert!(g.monsters[0].hp < 490 && g.monsters[1].hp == 500);
        // 反対側には何もない
        let o = g.run("zap a west");
        assert!(o.message.contains("何にも当たらなかった"), "{}", o.message);
    }

    #[test]
    fn lightning_pierces_every_enemy_in_the_line() {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(Tool::charged(ItemKind::WandLightning, 5));
        let (p1, p2) = ((g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1));
        assert!(g.map.tile(p2.0, p2.1).walkable());
        g.monsters.push(monster(&crate::monster::OGRE, p1, 500));
        g.monsters.push(monster(&crate::monster::OGRE, p2, 500));
        g.inflict_monster(0, Status::Paralyzed, 100);
        g.inflict_monster(1, Status::Paralyzed, 100);
        g.run("zap a east");
        assert!(g.monsters[0].hp < 500 && g.monsters[1].hp < 500);
    }

    #[test]
    fn nearest_lightning_keeps_going_past_the_target_and_always_hits_what_you_see() {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(Tool::charged(ItemKind::WandLightning, 5));
        let ps = [(g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1), (g.pos.0 + 3, g.pos.1)];
        assert!(ps.iter().all(|p| g.map.tile(p.0, p.1).walkable()));
        for p in ps {
            g.monsters.push(monster(&crate::monster::OGRE, p, 500));
            let i = g.monsters.len() - 1;
            g.inflict_monster(i, Status::Paralyzed, 100);
        }
        g.run("zap a nearest");
        assert!(g.monsters.iter().all(|m| m.hp < 500), "{:?}", g.monsters.iter().map(|m| m.hp).collect::<Vec<_>>());
    }

    #[test]
    fn diagonal_nearest_lightning_pierces_along_the_diagonal() {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(Tool::charged(ItemKind::WandLightning, 5));
        // 斜めの線を床にして、2体を並べる
        for k in 1..=4 {
            g.map.set_tile(g.pos.0 + k, g.pos.1 + k, Tile::Floor);
        }
        for k in [1, 3] {
            g.monsters.push(monster(&crate::monster::OGRE, (g.pos.0 + k, g.pos.1 + k), 500));
            let i = g.monsters.len() - 1;
            g.inflict_monster(i, Status::Paralyzed, 100);
        }
        g.run("zap a nearest");
        assert!(g.monsters.iter().all(|m| m.hp < 500), "{:?}", g.monsters.iter().map(|m| m.hp).collect::<Vec<_>>());
    }

    #[test]
    fn a_blocked_extension_falls_back_to_the_line_to_the_target() {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(Tool::charged(ItemKind::WandMissile, 5));
        let me = g.pos;
        // 延長線と直線が途中で食い違う配置(敵までは通るが、手前の1マスだけ違う)を探す
        let mut setup = None;
        'search: for dx in -8..=8i32 {
            for dy in -8..=8i32 {
                let t = (me.0 + dx, me.1 + dy);
                let m = dx.abs().max(dy.abs());
                if m < 3 || dx * dx + dy * dy > 64 {
                    continue;
                }
                let long = Map::line(me, (me.0 + dx * 12 / m, me.1 + dy * 12 / m));
                let direct = Map::line(me, t);
                if !long.contains(&t) {
                    continue;
                }
                let pos_t = long.iter().position(|p| *p == t).unwrap();
                if let Some(i) = (0..pos_t).find(|&i| long[i] != direct[i]) {
                    setup = Some((t, long[i], long, direct));
                    break 'search;
                }
            }
        }
        let (t, blocker, long, direct) = setup.expect("食い違う配置がある");
        for p in long.iter().chain(direct.iter()) {
            g.map.set_tile(p.0, p.1, Tile::Floor);
        }
        g.map.set_tile(blocker.0, blocker.1, Tile::Wall);
        g.refresh_fov();
        g.monsters.push(monster(&crate::monster::OGRE, t, 500));
        g.inflict_monster(0, Status::Paralyzed, 100);
        assert!(g.can_see_monster(0), "直線が通っているので見えるはず");
        // 延長線は敵の手前で塞がれているが、直線に戻って敵に当たる
        assert!(!g.open_cells(&long).contains(&t));
        assert_eq!(g.bolt_cells(crate::command::ZapTarget::Nearest).unwrap(), direct);
        g.run("zap a nearest");
        assert!(g.monsters[0].hp < 500);
    }

    #[test]
    fn nearest_hits_every_visible_enemy_in_open_rooms() {
        // どの向きにいる見えている敵にも、nearest なら必ず当たる(充填だけ減ることがない)
        let mut checked = 0;
        for seed in 0..60u64 {
            let mut g = quiet(seed);
            g.hp = 1000;
            g.max_hp = 1000;
            g.take(Tool::charged(ItemKind::WandMissile, 5));
            for (dx, dy) in [(3, 2), (-4, 1), (2, -3), (-2, -2), (5, 0), (0, 4), (4, 3)] {
                let p = (g.pos.0 + dx, g.pos.1 + dy);
                if !g.map.tile(p.0, p.1).walkable() || !g.map.is_visible(p.0, p.1) {
                    continue;
                }
                g.monsters.clear();
                g.monsters.push(monster(&crate::monster::OGRE, p, 500));
                g.inflict_monster(0, Status::Paralyzed, 100);
                g.inventory[0].tool.as_mut().unwrap().val = 5;
                let o = g.run("zap a nearest");
                assert!(g.monsters[0].hp < 500, "seed {seed} {dx},{dy}: {}", o.message);
                checked += 1;
            }
        }
        assert!(checked > 20, "{checked}");
    }

    #[test]
    fn bolts_do_not_pass_through_walls() {
        let mut g = quiet(3);
        g.take(Tool::charged(ItemKind::WandFire, 5));
        // 東隣に壁を立て、その向こうに敵を置く
        let (wall, beyond) = ((g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1));
        g.map.set_tile(wall.0, wall.1, Tile::Wall);
        g.map.set_tile(beyond.0, beyond.1, Tile::Floor);
        g.monsters.push(monster(&crate::monster::OGRE, beyond, 50));
        let o = g.run("zap a east");
        assert_eq!(g.monsters[0].hp, 50, "{}", o.message);
        assert!(o.message.contains("何にも当たらなかった"), "{}", o.message);
    }

    #[test]
    fn killing_with_a_wand_gives_experience() {
        let mut g = wand_vs_adjacent(ItemKind::WandFire, &crate::monster::SLIME, 3);
        let xp = g.xp();
        let o = g.run("zap a east");
        assert!(o.message.contains("を倒した"), "{}", o.message);
        assert!(g.monsters.is_empty() && g.xp() > xp);
    }

    #[test]
    fn cold_slows_what_it_hits_and_slow_and_haste_wands_use_statuses() {
        let mut g = wand_vs_adjacent(ItemKind::WandCold, &crate::monster::OGRE, 1000);
        g.run("zap a east");
        assert!(g.monsters[0].status.has(Status::Slowed));
        let mut g = wand_vs_adjacent(ItemKind::WandSlow, &crate::monster::GOBLIN, 1000);
        let o = g.run("zap a east");
        assert!(g.monsters[0].status.has(Status::Slowed), "{}", o.message);
        assert!(o.status_events.iter().any(|e| e.target == "ゴブリン" && e.status == Status::Slowed));
        let mut g = wand_vs_adjacent(ItemKind::WandHaste, &crate::monster::GOBLIN, 1000);
        g.run("zap a east");
        assert!(g.monsters[0].status.has(Status::Hasted));
    }

    #[test]
    fn drain_life_heals_you_by_the_damage_dealt() {
        let mut g = wand_vs_adjacent(ItemKind::WandDrain, &crate::monster::OGRE, 1000);
        g.hp = 5;
        g.monsters[0].status.apply(Status::Paralyzed, 100); // 反撃を受けない
        let o = g.run("zap a east");
        assert!(g.hp > 5 && o.message.contains("生命力を吸い取った"), "{} hp={}", o.message, g.hp);
        assert_eq!(1000 - g.monsters[0].hp, g.hp - 5);
    }

    #[test]
    fn invisible_monsters_vanish_from_view_until_you_can_see_invisible() {
        let mut g = wand_vs_adjacent(ItemKind::WandInvisibility, &crate::monster::GOBLIN, 1000);
        let o = g.run("zap a east");
        assert!(o.message.contains("姿が消えた"), "{}", o.message);
        assert!(g.visible_enemies().is_empty());
        assert_ne!(g.cell(g.pos.0 + 1, g.pos.1).ch, 'g');
        // 見えなくても殴られる。向きは分かる
        let o = g.run("wait");
        assert!(o.message.contains("何かの攻撃！(東から)"), "{}", o.message);
        g.status.apply(Status::SeeInvisible, 50);
        assert_eq!(g.visible_enemies().len(), 1);
        assert_eq!(g.cell(g.pos.0 + 1, g.pos.1).ch, 'g');
    }

    #[test]
    fn polymorph_changes_the_kind_and_keeps_the_health_ratio() {
        for seed in 1..20 {
            let mut g = wand_vs_adjacent(ItemKind::WandPolymorph, &crate::monster::OGRE, 5);
            g.depth = 4;
            g.monsters[0].max_hp = 10;
            g.monsters[0].status.apply(Status::Paralyzed, 100);
            g.rng = Rng::new(seed);
            g.run("zap a east");
            let m = &g.monsters[0];
            assert!(!std::ptr::eq(m.kind, &crate::monster::OGRE), "seed {seed}");
            assert_eq!((m.name, m.glyph), (m.kind.name, m.kind.glyph));
            assert_eq!(m.max_hp, m.kind.hp_at(4));
            assert!(m.hp >= 1 && m.hp <= m.max_hp);
        }
    }

    #[test]
    fn cancellation_strips_statuses_and_special_powers() {
        let mut g = wand_vs_adjacent(ItemKind::WandCancel, &crate::monster::SPIDER, 1000);
        g.inflict_monster(0, Status::Hasted, 50);
        let o = g.run("zap a east");
        assert!(g.monsters[0].status.active().is_empty() && g.monsters[0].cancelled, "{}", o.message);
        for _ in 0..40 {
            g.run("wait");
        }
        assert!(!g.status.has(Status::Poisoned), "消去された毒グモが毒を撒いた");
        assert!(o.status_events.iter().any(|e| e.status == Status::Hasted && e.change == Change::End));
    }

    #[test]
    fn teleport_other_sends_the_monster_away_and_teleport_self_moves_you() {
        let mut g = wand_vs_adjacent(ItemKind::WandTeleportOther, &crate::monster::GOBLIN, 1000);
        let before = g.monsters[0].pos;
        let o = g.run("zap a east");
        assert_ne!(g.monsters[0].pos, before, "{}", o.message);
        let mut g = with_wand(ItemKind::WandTeleportSelf, 3);
        let before = g.pos;
        // 向きは要らない
        let o = g.run("zap a");
        assert!(o.ok && g.pos != before, "{}", o.message);
        assert!(o.message.contains("(残り2回)"));
    }

    #[test]
    fn the_light_wand_maps_the_corridor_ahead() {
        let mut tried = false;
        for seed in 0..60 {
            let mut g = quiet(seed);
            g.take(Tool::charged(ItemKind::WandLight, 3));
            g.map.forget_all();
            for d in Dir::ALL {
                let (dx, dy) = d.delta();
                let open = (1..=6).all(|k| g.map.tile(g.pos.0 + dx * k, g.pos.1 + dy * k).walkable());
                let far = (g.pos.0 + 6 * dx, g.pos.1 + 6 * dy);
                if open {
                    let o = g.run(&format!("zap a {}", d.name()));
                    assert!(g.map.is_seen(far.0, far.1), "{}", o.message);
                    assert!(o.message.contains("照らされた"));
                    tried = true;
                    break;
                }
            }
            if tried {
                break;
            }
        }
        assert!(tried, "条件に合う場所がなかった");
    }

    #[test]
    fn confusion_can_send_a_bolt_the_wrong_way_even_with_nearest() {
        for cmd in ["zap a east", "zap a nearest"] {
            let mut g = wand_vs_adjacent(ItemKind::WandMissile, &crate::monster::OGRE, 100000);
            g.monsters[0].status.apply(Status::Paralyzed, 100000);
            g.status.apply(Status::Confused, 100000);
            let (mut misses, mut hits) = (0, 0);
            for _ in 0..40 {
                let hp = g.monsters[0].hp;
                g.run(cmd);
                if g.monsters[0].hp == hp { misses += 1 } else { hits += 1 }
                g.inventory[0].tool.as_mut().unwrap().val = 5;
            }
            assert!(misses > 5 && hits > 5, "{cmd}: {misses} {hits}");
        }
    }

    #[test]
    fn stealth_and_invisibility_never_make_adjacent_enemies_unaware() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.take(ring(ItemKind::RingStealth, 3));
        g.run("equip a");
        g.status.apply(Status::Invisible, 100);
        assert!(g.monster_aware(0));
        assert_eq!(g.notice_radius(), 2);
        g.status.clear(Status::Invisible);
        assert_eq!(g.notice_radius(), 2);
        g.rings[1] = g.rings[0]; // 強すぎる隠密(+6相当)でも下限がある
        assert!(g.notice_radius() >= 2);
    }

    #[test]
    fn drinking_haste_costs_a_turn_and_then_actions_alternate() {
        let mut g = quiet(1);
        g.take(ItemKind::Haste);
        let t = g.turn();
        g.run("quaff a");
        assert_eq!(g.turn() - t, 1);
        let t = g.turn();
        for _ in 0..4 {
            g.run("wait");
        }
        assert_eq!(g.turn() - t, 2);
    }

    #[test]
    fn wands_are_individuals_and_survive_drop_and_pickup_with_their_charges() {
        let mut g = quiet(2);
        g.take(Tool::charged(ItemKind::WandFire, 4));
        g.take(Tool::charged(ItemKind::WandFire, 2));
        assert_eq!(g.inventory.len(), 2);
        assert!(g.inventory_lines()[0].contains("[残り4回]") && g.inventory_lines()[1].contains("[残り2回]"));
        g.run("drop a");
        assert_eq!(g.underfoot_text().contains("火の杖") || g.underfoot_text().contains("杖"), true);
        g.run("pickup");
        assert!(g.inventory_lines().iter().any(|l| l.contains("[残り4回]")));
    }

    #[test]
    fn floor_wands_have_charges_inside_the_listed_range() {
        let mut rng = Rng::new(9);
        for k in ItemKind::ALL.into_iter().filter(|k| k.is_wand()) {
            for _ in 0..20 {
                let Item::Tool(t) = Item::roll(&mut rng, k, 5) else { panic!() };
                let (lo, hi) = k.zap().unwrap().charges;
                assert!((lo..=hi).contains(&t.val), "{k:?} {}", t.val);
            }
        }
        // 深いところの床には杖も落ちる
        let mut found = false;
        for seed in 0..60 {
            let mut g = Game::new(seed);
            g.depth = 6;
            g.spawn_items();
            found |= g.floor_items.iter().any(|f| f.item.kind().is_wand());
        }
        assert!(found);
    }

    #[test]
    fn identify_scroll_can_identify_an_unknown_wand() {
        let mut g = quiet(2);
        g.take(ItemKind::Identify);
        g.take(Tool::charged(ItemKind::WandSlow, 4));
        let o = g.run("read a b");
        assert!(o.ok && g.known[ItemKind::WandSlow.index()], "{}", o.message);
        assert!(g.inventory_lines()[0].contains("敵減速の杖 [残り4回]"));
    }

    // ---- 段階4: 指輪と光源 ----

    fn ring(kind: ItemKind, val: i32) -> Tool {
        let mut t = Tool::charged(kind, val);
        t.identified = false;
        t.cursed = val < 0 || kind.ring_def().is_some_and(|r| r.always_cursed);
        t
    }

    /// 指輪を持って（まだはめていない）いる静かなゲーム。文字は a, b, ...
    fn with_rings(rings: &[Tool]) -> Game {
        let mut g = quiet(2);
        g.hp = 20;
        g.max_hp = 20;
        for r in rings {
            g.take(*r);
        }
        g
    }

    #[test]
    fn all_the_rogue_rings_exist_with_gem_looks() {
        for name in [
            "防御の指輪", "腕力の指輪", "器用さの指輪", "ダメージ増加の指輪", "再生の指輪", "消化遅延の指輪", "隠密の指輪",
            "探索の指輪", "透明視認の指輪", "装飾の指輪", "怒らせる指輪", "テレポート癖の指輪",
        ] {
            assert!(ItemKind::ALL.iter().any(|k| k.is_ring() && k.true_name() == name), "{name}");
        }
        let g = Game::new(7);
        let looks: Vec<&str> = ItemKind::ALL.iter().filter(|k| k.is_ring()).map(|k| g.looks[k.index()]).collect();
        assert_eq!(looks.iter().collect::<std::collections::HashSet<_>>().len(), looks.len());
        assert!(looks.iter().all(|l| l.ends_with("の指輪")) && !looks.iter().any(|l| l.contains("防御")));
    }

    #[test]
    fn rings_are_unknown_until_worn_for_a_while_and_effects_apply_meanwhile() {
        let mut g = with_rings(&[ring(ItemKind::RingProtection, 2)]);
        let look = g.looks[ItemKind::RingProtection.index()];
        assert!(g.inventory_lines()[0].contains(look) && g.inventory_lines()[0].contains("未識別"));
        let def = g.defense();
        let o = g.run("equip a");
        assert!(o.ok && o.message.contains("効果はまだ分からない"), "{}", o.message);
        assert_eq!(g.defense(), def + 2, "識別前でも効果は効いている");
        assert!(g.inventory_lines()[0].contains("(装備中)"));
        for _ in 0..27 {
            g.run("wait");
        }
        assert!(!g.known[ItemKind::RingProtection.index()]);
        let mut msg = String::new();
        for _ in 0..4 {
            msg.push_str(&g.run("wait").message);
        }
        assert!(msg.contains("正体が分かった") && msg.contains("防御の指輪 +2"), "{msg}");
        assert!(g.known[ItemKind::RingProtection.index()]);
        assert!(g.inventory_lines()[0].contains("防御の指輪 +2") && g.inventory_lines()[0].contains("防御+2"));
    }

    #[test]
    fn a_known_kind_is_identified_as_soon_as_it_is_worn() {
        let mut g = with_rings(&[ring(ItemKind::RingStrength, 1)]);
        g.known[ItemKind::RingStrength.index()] = true;
        assert!(g.inventory_lines()[0].contains("腕力の指輪 (+?)"), "{}", g.inventory_lines()[0]);
        let o = g.run("equip a");
        assert!(o.message.contains("腕力の指輪 +1") && o.message.contains("腕力+1"), "{}", o.message);
    }

    #[test]
    fn two_ring_slots_and_unequip() {
        let mut g = with_rings(&[
            ring(ItemKind::RingTrinket, 0),
            ring(ItemKind::RingSearching, 1),
            ring(ItemKind::RingStealth, 1),
        ]);
        assert!(g.run("equip a").ok && g.run("equip b").ok);
        let o = g.run("equip c");
        assert!(!o.ok && o.message.contains("ふさがっている"), "{}", o.message);
        assert!(!g.run("equip a").ok);
        assert!(g.run("unequip a").ok);
        assert!(g.run("equip c").ok);
        assert!(!g.run("unequip a").ok);
        // 装備中の指輪は捨てられない
        assert!(!g.run("drop b").ok);
        assert!(g.run("unequip b").ok && g.run("drop b").ok);
    }

    #[test]
    fn cursed_rings_cannot_be_removed_until_remove_curse() {
        let mut g = with_rings(&[ring(ItemKind::RingProtection, -2)]);
        let def = g.defense();
        let o = g.run("equip a");
        assert!(o.message.contains("呪われていた") && o.message.contains("防御の指輪 -2"), "{}", o.message);
        assert_eq!(g.defense(), def - 2);
        let o = g.run("unequip a");
        assert!(!o.ok && o.message.contains("はずせない"), "{}", o.message);
        assert!(!g.run("drop a").ok);
        assert!(g.inventory_lines()[0].contains("(呪われている)"));
        g.take(ItemKind::RemoveCurse);
        let o = g.run("read b");
        assert!(o.message.contains("呪いの束縛が解けた"), "{}", o.message);
        assert!(g.run("unequip a").ok);
        assert!(g.inventory_lines()[0].contains("(呪い解除済み)"));
    }

    #[test]
    fn strength_damage_and_dexterity_rings() {
        let mut g = with_rings(&[ring(ItemKind::RingStrength, 2), ring(ItemKind::RingDamage, 1)]);
        let (lo, hi) = g.attack_range();
        g.run("equip a");
        assert_eq!(g.attack_range(), (lo + 1, hi + 1)); // 腕力 +2 → 攻撃 +1
        assert!(g.observe_text(3).contains("腕力 12/10"));
        g.run("equip b");
        assert_eq!(g.attack_range(), (lo + 2, hi + 2));
        // 器用さ: 敵の攻撃をかわす
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.take(ring(ItemKind::RingDexterity, 5));
        g.run("equip a");
        let mut dodged = 0;
        let mut hit = 0;
        for _ in 0..60 {
            let m = g.run("wait").message;
            dodged += m.matches("かわした").count();
            hit += m.matches("の攻撃！").count();
        }
        assert!(dodged > 10 && hit > 10, "{dodged} {hit}");
        // 負の器用さ: 自分の攻撃が空振りする
        let mut g = with_adjacent(3, &crate::monster::OGRE);
        g.monsters[0].status.apply(Status::Paralyzed, 1000);
        g.take(ring(ItemKind::RingDexterity, -3));
        g.run("equip a");
        let whiffs = (0..60).filter(|_| g.run("attack east").message.contains("空を切った")).count();
        assert!(whiffs > 8 && whiffs < 40, "{whiffs}");
    }

    #[test]
    fn regeneration_heals_faster_even_with_enemies_around() {
        let mut g = quiet(2);
        g.take(ring(ItemKind::RingRegeneration, 2));
        g.run("equip a");
        g.hp = 1;
        g.max_hp = 100;
        for _ in 0..40 {
            g.run("wait");
        }
        assert!(g.hp >= 1 + 40 / 4, "{}", g.hp);
        // 敵が見えていても治る
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.monsters[0].status.apply(Status::Paralyzed, 1000);
        g.take(ring(ItemKind::RingRegeneration, 3));
        g.run("equip a");
        g.hp = 1;
        for _ in 0..20 {
            g.run("wait");
        }
        assert!(g.hp > 3);
    }

    #[test]
    fn slow_digestion_makes_hunger_grow_slower() {
        let hunger = |with_ring: bool| {
            let mut g = quiet(2);
            if with_ring {
                g.take(ring(ItemKind::RingSlowDigestion, 1));
                g.run("equip a");
            }
            let f = g.food;
            for _ in 0..40 {
                g.run("wait");
            }
            f - g.food
        };
        let (plain, slow) = (hunger(false), hunger(true));
        assert!(slow * 2 <= plain + 2 && slow * 2 + 2 >= plain, "{plain} {slow}");
    }

    #[test]
    fn stealth_shrinks_the_distance_enemies_notice_you() {
        let mut g = (3..200)
            .map(|seed| with_adjacent(seed, &crate::monster::GOBLIN))
            .find(|g| g.map.tile(g.pos.0 + 5, g.pos.1).walkable() && g.map.los(g.pos, (g.pos.0 + 5, g.pos.1)))
            .expect("東に5マス見通せる seed がある");
        let far = (g.pos.0 + 5, g.pos.1);
        g.monsters[0].pos = far;
        assert!(g.monster_aware(0));
        g.take(ring(ItemKind::RingStealth, 2));
        g.run("equip a");
        assert!(!g.monster_aware(0)); // 9 - 6 = 3 < 5
        g.monsters[0].pos = (g.pos.0 + 3, g.pos.1);
        assert!(g.monster_aware(0));
    }

    #[test]
    fn the_searching_ring_finds_traps_more_surely_and_from_further() {
        let mut g = quiet(3);
        let p = (g.pos.0 + 3, g.pos.1);
        assert!(g.map.tile(p.0, p.1).walkable());
        g.traps.push(Trap { pos: p, kind: TrapKind::Dart, revealed: false });
        g.take(ring(ItemKind::RingSearching, 3));
        g.run("equip a");
        for _ in 0..3 {
            g.run("wait");
        }
        assert!(g.traps[0].revealed, "探索の指輪でも3マス先の罠が見つからなかった");
    }

    #[test]
    fn see_invisible_ring_shows_invisible_enemies() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.monsters[0].status.apply(Status::Invisible, 1000);
        assert!(g.visible_enemies().is_empty());
        g.take(ring(ItemKind::RingSeeInvisible, 1));
        g.run("equip a");
        assert_eq!(g.visible_enemies().len(), 1);
    }

    #[test]
    fn the_aggravate_ring_pulls_every_enemy_toward_you_and_cannot_be_removed() {
        let mut g = quiet(2);
        let far = (1..W - 1)
            .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
            .find(|&(x, y)| g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs() + (y - g.pos.1).abs() > 25)
            .unwrap();
        let id = g.add_monster(&crate::monster::SLIME, far);
        assert!(!g.monster_aware(id));
        g.take(ring(ItemKind::RingAggravate, 1));
        let o = g.run("equip a");
        assert!(o.message.contains("呪われていた"), "{}", o.message);
        assert!(g.monster_aware(id));
        let mut moved = false;
        for _ in 0..4 {
            g.run("wait");
            moved |= g.monsters[id].pos != far;
        }
        assert!(moved);
        assert!(!g.run("unequip a").ok);
    }

    #[test]
    fn the_teleportitis_ring_throws_you_around_now_and_then() {
        let mut g = quiet(2);
        g.take(ring(ItemKind::RingTeleportitis, 1));
        g.run("equip a");
        let mut jumps = 0;
        for _ in 0..400 {
            g.hp = g.max_hp;
            g.food = MAX_FOOD;
            if g.run("wait").message.contains("飛ばされた") {
                jumps += 1;
            }
        }
        assert!((3..40).contains(&jumps), "{jumps}");
    }

    #[test]
    fn identify_scroll_reveals_a_ring_and_warns_about_a_curse() {
        let mut g = with_rings(&[ring(ItemKind::RingDamage, -2)]);
        g.take(ItemKind::Identify);
        let o = g.run("read b a");
        assert!(o.ok && o.message.contains("ダメージ増加の指輪 -2") && o.message.contains("呪われている"), "{}", o.message);
        assert!(g.known[ItemKind::RingDamage.index()]);
        // 呪いと分かっていれば、はめる前に避けられる
        assert!(g.inventory_lines()[0].contains("(呪われている)"));
    }

    #[test]
    fn rolled_rings_carry_a_strength_and_some_are_cursed() {
        let mut rng = Rng::new(3);
        let (mut cursed, mut total) = (0, 0);
        for k in ItemKind::ALL.into_iter().filter(|k| k.is_ring()) {
            for _ in 0..80 {
                let t = Tool::roll(&mut rng, k);
                let r = k.ring_def().unwrap();
                total += 1;
                assert!(!t.identified);
                if r.always_cursed {
                    assert!(t.cursed);
                } else if !r.cursable {
                    assert!(!t.cursed && t.val >= 0, "{k:?} {t:?}");
                } else {
                    assert_eq!(t.cursed, t.val < 0, "{k:?}");
                    assert!((1..=3).contains(&t.val.abs()));
                }
                cursed += t.cursed as u32;
            }
        }
        assert!(cursed * 10 > total && cursed * 2 < total, "{cursed}/{total}");
    }

    // ---- 光源 ----

    #[test]
    fn you_start_with_a_lit_torch_and_its_fuel_is_always_shown() {
        let g = quiet(2);
        assert!(g.status_text().contains("光源:松明 燃料1500/1500"), "{}", g.status_text());
        assert!(g.observe_text(3).contains("光源: 松明 [燃料 1500/1500] (装備中)"));
        assert!(g.light_line().contains("松明"));
        let mut g = quiet(2);
        assert!(g.run("inventory").message.contains("光源: 松明 [燃料 1500/1500]"));
    }

    #[test]
    fn fuel_burns_each_turn_warns_when_low_and_darkens_when_out() {
        let mut g = quiet(2);
        g.light = Some(Tool::charged(ItemKind::Torch, 103));
        assert!(g.map.is_visible(g.pos.0 + 6, g.pos.1) || !g.map.tile(g.pos.0 + 6, g.pos.1).walkable());
        let mut warned = false;
        for _ in 0..3 {
            warned |= g.run("wait").message.contains("火が弱くなってきた");
        }
        assert!(warned);
        assert_eq!(g.light.unwrap().val, 100);
        for _ in 0..99 {
            g.run("wait");
        }
        let o = g.run("wait");
        assert!(o.message.contains("燃え尽きた"), "{}", o.message);
        assert_eq!(g.light.unwrap().val, 0);
        assert!(g.status_text().contains("暗闇"));
        // 視界が狭まる: 半径2より遠くは見えない
        let far = (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).filter(|&(x, y)| g.map.is_visible(x, y)).all(|(x, y)| {
            (x - g.pos.0).pow(2) + (y - g.pos.1).pow(2) <= crate::item::DARK_RADIUS.pow(2)
        });
        assert!(far);
        // 燃料は減り続けない
        g.run("wait");
        assert_eq!(g.light.unwrap().val, 0);
    }

    #[test]
    fn running_out_of_light_interrupts_auto_walk() {
        let mut g = quiet(2);
        g.light = Some(Tool::charged(ItemKind::Torch, 5));
        let o = g.run("explore");
        assert!(o.message.contains("明かりが消えて中断"), "{}", o.message);
    }

    #[test]
    fn swapping_lights_returns_the_old_one_and_a_lantern_can_be_refilled() {
        let mut g = quiet(2);
        g.light = Some(Tool::charged(ItemKind::Torch, 40));
        g.take(Tool::charged(ItemKind::Lantern, 1000));
        let o = g.run("equip a");
        assert!(o.ok && o.message.contains("ランタン"), "{}", o.message);
        assert_eq!(g.light.unwrap().kind, ItemKind::Lantern);
        // 古い松明が同じ文字に戻る
        assert!(g.inventory_lines()[0].starts_with("a) 松明 [燃料") , "{:?}", g.inventory_lines());
        // 油を継ぎ足す
        let o = g.run("refill");
        assert!(!o.ok && o.message.contains("油つぼを持っていない"), "{}", o.message);
        g.take(ItemKind::OilFlask);
        g.take(ItemKind::OilFlask);
        let t = g.turn();
        let o = g.run("refill");
        assert!(o.ok && g.turn() == t + 1, "{}", o.message);
        let fuel = g.light.unwrap().val;
        assert!((2490..=2500).contains(&fuel), "{fuel}");
        assert!(g.inventory_lines().iter().any(|l| l.contains("油つぼ") && !l.contains("x2")));
        // 満タンを超えない
        g.run("refill");
        let max = ItemKind::Lantern.light_def().unwrap().max_fuel;
        assert!(g.light.unwrap().val <= max);
        g.light = Some(Tool::charged(ItemKind::Lantern, max));
        let o = g.run("refill");
        assert!(!o.ok && o.message.contains("満タン"), "{}", o.message);
    }

    #[test]
    fn a_torch_cannot_be_refilled_and_refilling_a_dead_lantern_relights_the_room() {
        let mut g = quiet(2);
        g.take(ItemKind::OilFlask);
        let o = g.run("refill");
        assert!(!o.ok && o.message.contains("継ぎ足せない"), "{}", o.message);
        g.light = Some(Tool::charged(ItemKind::Lantern, 0));
        g.refresh_fov();
        assert!(!g.map.is_visible(g.pos.0 + 5, g.pos.1));
        let o = g.run("refill");
        assert!(o.ok && o.message.contains("明るくなった"), "{}", o.message);
        assert!(g.map.is_visible(g.pos.0 + 1, g.pos.1));
        assert!(!g.run("refill").ok);
    }

    #[test]
    fn dark_does_not_blind_the_monsters() {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.light = Some(Tool::charged(ItemKind::Torch, 0));
        g.refresh_fov();
        let far = (g.pos.0 + 5, g.pos.1);
        if g.map.tile(far.0, far.1).walkable() {
            g.monsters[0].pos = far;
            assert!(g.monster_aware(0));
            assert!(g.visible_enemies().is_empty(), "暗闇で遠くの敵が見えている");
        }
    }

    #[test]
    fn lights_and_oil_appear_on_the_floor_and_oil_stacks() {
        let mut found = (false, false, false);
        for seed in 0..200 {
            let mut g = Game::new(seed);
            g.depth = 4;
            g.spawn_items();
            for f in &g.floor_items {
                match f.item.kind() {
                    ItemKind::Torch => found.0 = true,
                    ItemKind::Lantern => found.1 = true,
                    ItemKind::OilFlask => found.2 = true,
                    _ => {}
                }
            }
        }
        assert!(found.0 && found.1 && found.2, "{found:?}");
        let mut g = quiet(2);
        g.take(ItemKind::OilFlask);
        g.take(ItemKind::OilFlask);
        assert_eq!(g.inventory.len(), 1);
        assert!(g.inventory_lines()[0].contains("油つぼ x2"));
        // 光源は重ならず、燃料の量が分かる
        g.take(Tool::charged(ItemKind::Torch, 700));
        g.take(Tool::charged(ItemKind::Torch, 300));
        let lines = g.inventory_lines();
        assert!(lines[1].contains("[燃料 700/1500]") && lines[2].contains("[燃料 300/1500]"), "{lines:?}");
    }

    /// 全種類のアイテムを持たせて乱暴に遊び、全メッセージを返す。途中で不変条件も確かめる。
    fn fuzz_play(seed: u64, steps: usize) -> Vec<String> {
        let verbs = ["quaff", "read", "eat", "equip", "unequip", "drop", "zap", "refill"];
        let dirs = ["north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest", "nearest", ""];
        let mut out = Vec::new();
        let mut g = Game::new(seed);
        let mut r = Rng::new(seed ^ 0xABCDEF);
        for k in ItemKind::ALL {
            if g.has_free_letter() {
                g.take(Item::roll(&mut r, k, 10));
            }
        }
        for step in 0..steps {
            if g.is_dead() || g.is_won() {
                break;
            }
            let letter = (b'a' + r.range(0, 26) as u8) as char;
            let cmd = match r.range(0, 12) {
                0 | 1 => format!("{} {letter}", verbs[r.range(0, verbs.len() as i32) as usize]),
                2 => format!("zap {letter} {}", dirs[r.range(0, dirs.len() as i32) as usize]),
                3 => "explore".to_string(),
                4 => "travel >; descend".to_string(),
                5 => "pickup".to_string(),
                6 => format!("read {letter} {}", (b'a' + r.range(0, 26) as u8) as char),
                7 => "stay 5".to_string(),
                8 => "inventory; look".to_string(),
                _ => format!("move {}", dirs[r.range(0, 8) as usize]),
            };
            for o in g.run_script(&cmd) {
                assert!(o.hp <= g.max_hp(), "seed {seed} step {step}: {} -> hp {}", o.command, o.hp);
                out.push(format!("{} | {} | {} | {}", o.command, o.ok, o.message, o.turn));
            }
            let text = g.observe_text(5);
            assert!(text.contains("光源:"), "{text}");
            out.push(text);
            // 装備している物は、必ず持ち物にある
            for l in [g.weapon, g.armor, g.rings[0], g.rings[1]].into_iter().flatten() {
                assert!(g.inventory.iter().any(|s| s.letter == l), "seed {seed}: 装備 {l} が持ち物にない");
            }
            assert!(g.rings[0].is_none() || g.rings[0] != g.rings[1]);
        }
        out
    }

    #[test]
    fn random_play_with_every_item_never_panics_and_stays_consistent() {
        for seed in 0..40u64 {
            fuzz_play(seed, 400);
        }
    }

    #[test]
    fn the_same_seed_and_script_always_give_the_same_game() {
        for seed in [1u64, 7, 23, 31] {
            assert_eq!(fuzz_play(seed, 300), fuzz_play(seed, 300), "seed {seed}");
        }
    }

    #[test]
    fn teleportitis_during_auto_walk_stops_it_and_still_picks_up_what_you_land_on() {
        let (mut jumped, mut landed) = (0, 0);
        for seed in 0..40 {
            let mut g = Game::new(seed);
            g.monsters.clear();
            g.traps.clear();
            g.hp = 1000;
            g.max_hp = 1000;
            g.take(ring(ItemKind::RingTeleportitis, 1));
            g.run("equip a");
            // 床のどこに飛んでも品物がある状態にする(拾いが必ず問われる)
            let spots: Vec<(i32, i32)> = (1..W - 1)
                .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
                .filter(|&(x, y)| g.map.tile(x, y) == Tile::Floor && (x, y) != g.pos)
                .collect();
            for p in spots {
                g.floor_items.push(FloorItem::new(p, ItemKind::OilFlask));
            }
            for cmd in ["explore", "explore", "travel >", "explore"] {
                g.food = MAX_FOOD;
                let o = g.run(cmd);
                if o.message.contains("飛ばされて中断") || o.message.contains("飛ばされた") {
                    jumped += 1;
                    // 着いた場所に物があれば、飛んだ時点で拾っている(足元に取り残さない)
                    // (1マスにつき自動で拾うのは1個だけ。飛んだ直後に拾った知らせが続く)
                    if let Some((_, after)) = o.message.rsplit_once("突然どこかへ飛ばされた！") {
                        // 拾っていないなら、そのマスにはもう何も残っていない(通った場所に飛んだ)
                        assert!(
                            after.contains("拾った") || after.contains("いっぱい") || g.floor_order(g.pos).is_empty(),
                            "seed {seed}: {}",
                            o.message
                        );
                        landed += 1;
                    }
                }
            }
        }
        assert!(jumped > 0 && landed > 0, "テレポート癖が発動しなかった({jumped}/{landed})");
    }

    #[test]
    fn disarm_removes_a_known_trap_with_some_probability() {
        let (mut ok, mut fail) = (0, 0);
        for seed in 0..60 {
            let mut g = quiet(3);
            g.hp = 1000;
            g.max_hp = 1000;
            g.rng = Rng::new(seed);
            let p = put_trap(&mut g, TrapKind::SleepGas);
            g.traps[0].revealed = true;
            let t = g.turn();
            let o = g.run("disarm east");
            assert!(o.ok && g.turn() > t, "{}", o.message);
            if o.message.contains("解除した") {
                assert!(g.traps.is_empty());
                assert_ne!(g.cell(p.0, p.1).ch, '^');
                ok += 1;
            } else {
                assert!(o.message.contains("失敗") && g.traps.len() == 1, "{}", o.message);
                fail += 1;
            }
        }
        assert!(ok > 20 && fail > 5, "{ok} {fail}");
    }

    #[test]
    fn disarm_needs_a_known_trap_and_sight_and_can_trigger_on_failure() {
        let mut g = quiet(3);
        put_trap(&mut g, TrapKind::Dart); // 隠れている
        let t = g.turn();
        let o = g.run("disarm east");
        assert!(!o.ok && o.message.contains("見つけた罠がない") && g.turn() == t, "{}", o.message);
        g.traps[0].revealed = true;
        g.status.apply(Status::Blind, 10);
        assert!(!g.run("disarm east").ok);
        g.status.clear(Status::Blind);
        // 失敗すると罠が作動することがある
        let mut fired = false;
        for seed in 0..80 {
            let mut g = quiet(3);
            g.hp = 1000;
            g.max_hp = 1000;
            g.rng = Rng::new(seed);
            put_trap(&mut g, TrapKind::Dart);
            g.traps[0].revealed = true;
            let o = g.run("disarm east");
            if o.message.contains("作動した") {
                fired = true;
                assert!(g.hp < 1000 && g.status.has(Status::Poisoned), "{}", o.message);
                break;
            }
        }
        assert!(fired);
        // 器用さの指輪で成功率が上がる
        let mut g = quiet(3);
        let base = g.disarm_percent();
        g.take(ring(ItemKind::RingDexterity, 3));
        g.run("equip a");
        assert!(g.disarm_percent() > base);
    }

    #[test]
    fn a_failed_disarm_next_to_a_trapdoor_never_drops_you() {
        for seed in 0..60 {
            let mut g = quiet(3);
            g.rng = Rng::new(seed);
            put_trap(&mut g, TrapKind::Trapdoor);
            g.traps[0].revealed = true;
            g.run("disarm east");
            assert_eq!(g.depth(), 1);
        }
    }

    /// 失敗して罠が作動する seed を探して、そのゲームを返す(作動前の状態)。
    fn disarm_failure_that_fires(kind: TrapKind, under_foot: bool, setup: impl Fn(&mut Game)) -> Option<(Game, Outcome)> {
        for seed in 0..300 {
            let mut g = quiet(3);
            g.hp = 1000;
            g.max_hp = 1000;
            g.rng = Rng::new(seed);
            let p = put_trap(&mut g, kind);
            g.traps[0].revealed = true;
            if under_foot {
                g.pos = p;
            }
            setup(&mut g);
            let o = g.run(if under_foot { "disarm" } else { "disarm east" });
            if o.message.contains("作動した！") {
                return Some((g, o));
            }
        }
        None
    }

    #[test]
    fn a_failed_disarm_of_the_trapdoor_underfoot_drops_you() {
        let (g, o) = disarm_failure_that_fires(TrapKind::Trapdoor, true, |_| {}).expect("作動する seed がある");
        assert_eq!(g.depth(), 2, "{}", o.message);
    }

    #[test]
    fn levitation_keeps_a_failed_disarm_from_firing_the_trap() {
        for (kind, foot) in [(TrapKind::Dart, false), (TrapKind::Dart, true), (TrapKind::SleepGas, true), (TrapKind::Trapdoor, true)] {
            let mut saw_failure_with_slip = false;
            for seed in 0..120 {
                let mut g = quiet(3);
                g.hp = 1000;
                g.max_hp = 1000;
                g.rng = Rng::new(seed);
                let p = put_trap(&mut g, kind);
                g.traps[0].revealed = true;
                if foot {
                    g.pos = p;
                }
                g.status.apply(Status::Levitating, 1000);
                let o = g.run(if foot { "disarm" } else { "disarm east" });
                assert!(!o.message.contains("作動した！"), "{kind:?}: {}", o.message);
                assert_eq!((g.hp, g.depth(), g.status.has(Status::Poisoned), g.status.has(Status::Asleep)), (1000, 1, false, false), "{kind:?}");
                saw_failure_with_slip |= o.message.contains("浮いているので罠は作動しなかった");
            }
            assert!(saw_failure_with_slip, "{kind:?}: 失敗して手元が狂う場面が一度もなかった");
        }
    }

    #[test]
    fn confusion_lowers_the_disarm_chance_and_the_message_shows_it() {
        let mut g = quiet(3);
        let base = g.disarm_percent();
        g.status.apply(Status::Confused, 50);
        assert_eq!(g.disarm_percent(), base - 30);
        let p = put_trap(&mut g, TrapKind::SleepGas);
        g.traps[0].revealed = true;
        let _ = p;
        g.status.clear(Status::Confused);
        let o = g.run("disarm east");
        assert!(o.message.contains(&format!("成功率{base}%")), "{}", o.message);
    }

    #[test]
    fn disarming_a_known_trap_lets_auto_walk_path_through_it() {
        let mut g = quiet(3);
        let p = put_trap(&mut g, TrapKind::Dart);
        g.traps[0].revealed = true;
        g.map.mark_seen(p.0, p.1);
        assert!(g.find_path(&|q| q == p).is_none(), "既知の罠は経路から外れる");
        g.hp = 1000;
        g.max_hp = 1000;
        for seed in 0..100 {
            g.rng = Rng::new(seed);
            if g.run("disarm east").message.contains("解除した") {
                break;
            }
        }
        assert!(g.traps.is_empty());
        assert!(g.find_path(&|q| q == p).is_some(), "解除した罠のマスには入れる");
    }

    #[test]
    fn disarm_rejects_bad_directions_with_the_same_wording_as_zap() {
        assert!(crate::command::parse("disarm sideways").unwrap_err().contains("不明な向き"));
        assert!(crate::command::parse("zap a sideways").unwrap_err().contains("不明な向き"));
    }
}
