use std::collections::VecDeque;

use crate::command::{self, Command, Dir, TravelTarget};
use crate::item::{
    Class, Effect, Gear, Item, ItemKind, Suffix, Tool, MUSHROOM_LOOKS, POTION_LOOKS, RING_LOOKS,
    SCROLL_LOOKS, WAND_LOOKS,
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
/// ログに残す件数の目安。これを超えたぶんは古い方から捨てる（読むのは末尾の数件だけ）。
const LOG_KEEP: usize = 1000;
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

/// コマンドの直接の実行結果（内部処理用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ActionResult {
    pub ok: bool,
    pub message: String,
    pub spent: bool,
}

impl ActionResult {
    pub fn new(ok: bool, message: impl Into<String>, spent: bool) -> Self {
        Self {
            ok,
            message: message.into(),
            spent,
        }
    }

    pub fn success(message: impl Into<String>, spent: bool) -> Self {
        Self::new(true, message, spent)
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self::new(false, message, false)
    }
}

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

/// 持ち物の1スロットの情報（TUI オーバーレイなどの絞り込み用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEntry {
    pub letter: char,
    pub line: String,
    pub kind: ItemKind,
    pub equipped: bool,
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
        parts.push(format!(
            "{}に{}",
            if dx > 0 { "東" } else { "西" },
            dx.abs()
        ));
    }
    if dy != 0 {
        parts.push(format!(
            "{}に{}",
            if dy > 0 { "南" } else { "北" },
            dy.abs()
        ));
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
        // 毎回ではなく、倍に達したときにまとめて捨てる
        if self.log.len() >= LOG_KEEP * 2 {
            self.log.drain(..self.log.len() - LOG_KEEP);
        }
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
        self.floor_order(p)
            .first()
            .map(|&i| self.floor_items[i].item)
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
            Item::Plain(kind) => {
                self.inventory.iter().any(|s| s.kind == kind) || self.has_free_letter()
            }
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
            self.note(
                "魔除けのアミュレットを手に入れた！ 階段は登り階段になった。地上まで持ち帰ろう。",
            );
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
        self.floor_order(p)
            .first()
            .copied()
            .filter(|&j| !self.floor_items[j].dropped)
    }

    /// 床の物 `j` を持ち物に移す。成功ならそのメッセージ、満杯なら Err のメッセージ。
    fn take_floor(&mut self, j: usize) -> Result<String, String> {
        let item = self.floor_items[j].item;
        match self.take(item) {
            Some(letter) => {
                self.floor_items.remove(j);
                Ok(format!("{}を拾った。({letter})", self.item_name(&item)))
            }
            None => Err(format!(
                "持ち物がいっぱいで、{}を拾えない。",
                self.item_name(&item)
            )),
        }
    }

    /// `pickup [番号]`: 足元の物を1個拾う。番号は足元の一覧の番号（省くと1番）。
    fn pickup_cmd(&mut self, n: Option<u32>) -> ActionResult {
        let order = self.floor_order(self.pos);
        if order.is_empty() {
            return ActionResult::failure("足元には何もない。");
        }
        let n = n.unwrap_or(1) as usize;
        if n > order.len() {
            return ActionResult::failure(format!(
                "足元の番号は 1〜{} だ。{}",
                order.len(),
                self.underfoot_text()
            ));
        }
        match self.take_floor(order[n - 1]) {
            Ok(msg) => ActionResult::success(msg, true),
            Err(msg) => ActionResult::failure(msg),
        }
    }

    /// `drop <文字> [数]`: 持ち物を足元に捨てる。捨てた物には印が付き、自動では拾われない。
    fn drop_cmd(&mut self, letter: char, count: u32) -> ActionResult {
        let Some(si) = self.inventory.iter().position(|s| s.letter == letter) else {
            return ActionResult::failure(format!("持ち物 {letter} はない。"));
        };
        let stack = &self.inventory[si];
        // 装備中のものは捨てられない。呪われていれば、はずせないので同じこと
        if self.is_equipped(letter) {
            let name = match (stack.gear, stack.tool) {
                (Some(g), _) => g.name(),
                (_, Some(t)) => self.ring_name(&t),
                _ => String::new(),
            };
            return if stack.gear.is_some_and(|g| g.is_sticky())
                || stack.tool.is_some_and(|t| t.is_sticky())
            {
                ActionResult::failure(format!("{name}は呪われていて、はずせない。捨てられない。"))
            } else {
                ActionResult::failure(format!(
                    "{name}は装備中だ。先に unequip {letter} ではずそう。"
                ))
            };
        }
        if count > stack.count {
            return ActionResult::failure(format!(
                "{letter} は{}個しか持っていない。",
                stack.count
            ));
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
        ActionResult::success(msg, true)
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
            .chain(
                self.has_amulet
                    .then(|| "★ 魔除けのアミュレット".to_string()),
            )
            .collect()
    }

    /// 持ち物の各スロットの詳細情報（絞り込み用）。
    pub fn inventory_entries(&self) -> Vec<ItemEntry> {
        self.inventory
            .iter()
            .map(|s| {
                let equipped = self.is_equipped(s.letter);
                let line = if let Some(g) = &s.gear {
                    self.gear_line(s.letter, g)
                } else if let Some(t) = &s.tool {
                    self.tool_line(s.letter, t)
                } else {
                    let mut l = format!("{}) {}", s.letter, self.display_name(s.kind));
                    if s.count > 1 {
                        l.push_str(&format!(" x{}", s.count));
                    }
                    if !self.known[s.kind.index()] {
                        l.push_str(" (未識別)");
                    }
                    l
                };
                ItemEntry {
                    letter: s.letter,
                    line,
                    kind: s.kind,
                    equipped,
                }
            })
            .collect()
    }

    /// 装備中の光源の1行（持ち物の外にあるので、一覧とは別に出す）。
    pub fn light_line(&self) -> String {
        match self.light {
            Some(l) => format!(
                "光源: {} (装備中){}",
                self.light_text(&l),
                if l.val <= 0 {
                    " 燃え尽きている"
                } else {
                    ""
                }
            ),
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
                line.push_str(&format!(
                    " [{}]",
                    t.kind
                        .ring_def()
                        .map_or(String::new(), |r| r.effect.describe(t.val))
                ));
                if t.cursed {
                    line.push_str(if t.freed {
                        " (呪い解除済み)"
                    } else {
                        " (呪われている)"
                    });
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
            .filter_map(|s| {
                s.tool
                    .filter(|t| t.kind.is_wand())
                    .map(|t| format!("{}:{}", s.letter, t.val))
            })
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
                    let (clo, chi) = self
                        .weapon_gear()
                        .and_then(|w| w.weapon_range())
                        .unwrap_or((2, 4));
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
                self.note(&format!(
                    "身につけているうちに、{old}の正体が分かった。{text}"
                ));
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
        let mut x = self.seed
            ^ ((self.turn as u64) << 24)
            ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
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
        let mut s = format!(
            "腕力 {}/{} 満腹度 {}/{}",
            self.effective_strength(),
            self.max_strength,
            self.food,
            MAX_FOOD
        );
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
                s.push_str(&format!(
                    " 光源:{} 燃料{}/{}",
                    l.kind.true_name(),
                    l.val,
                    max
                ));
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
        let (lo, hi) = self
            .weapon_gear()
            .and_then(|g| g.weapon_range())
            .unwrap_or((2, 4));
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
    fn equip(&mut self, letter: char) -> ActionResult {
        let Some(s) = self.inventory.iter().find(|s| s.letter == letter) else {
            return ActionResult::failure(format!("持ち物 {letter} はない。"));
        };
        if let Some(t) = s.tool {
            match t.kind.class() {
                Class::Ring => return self.equip_ring(letter, t),
                Class::Light => return self.equip_light(letter, t),
                _ => {}
            }
        }
        let Some(gear) = s.gear else {
            return ActionResult::failure(format!("{}は装備できない。", self.display_name(s.kind)));
        };
        let slot = if gear.kind.is_weapon() {
            self.weapon
        } else {
            self.armor
        };
        if slot == Some(letter) {
            return ActionResult::failure(format!("{}はすでに装備している。", gear.name()));
        }
        if let Some(cur) = self.gear_of(slot).filter(|g| g.is_sticky()) {
            return ActionResult::failure(format!(
                "{}は呪われていて、はずせない。別の装備には替えられない。",
                cur.name()
            ));
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
            if let Some(g) = self
                .inventory
                .iter_mut()
                .find(|s| s.letter == letter)
                .and_then(|s| s.gear.as_mut())
            {
                g.identified = true;
            }
            msg.push_str(&format!(
                " 呪われていた！ もうはずせない。({})",
                gear.suffix.map_or("", Suffix::describe)
            ));
        }
        ActionResult::success(msg, true)
    }

    /// 装備をはずす。
    fn unequip(&mut self, letter: char) -> ActionResult {
        let Some(s) = self.inventory.iter().find(|s| s.letter == letter) else {
            return ActionResult::failure(format!("持ち物 {letter} はない。"));
        };
        if let Some(t) = s.tool.filter(|t| t.kind.is_ring()) {
            return self.unequip_ring(letter, t);
        }
        let Some(gear) = s.gear else {
            return ActionResult::failure(format!(
                "{}は装備品ではない。",
                self.display_name(s.kind)
            ));
        };
        if self.weapon != Some(letter) && self.armor != Some(letter) {
            return ActionResult::failure(format!("{}は装備していない。", gear.name()));
        }
        if gear.is_sticky() {
            return ActionResult::failure(format!("{}は呪われていて、はずせない。", gear.name()));
        }
        if gear.kind.is_weapon() {
            self.weapon = None;
        } else {
            self.armor = None;
        }
        ActionResult::success(format!("{}をはずした。", gear.name()), true)
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
        (0..self.monsters.len())
            .filter(|&i| self.can_see_monster(i))
            .collect()
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
            self.traps.push(Trap {
                pos: (x, y),
                kind,
                revealed: false,
            });
        }
    }

    /// 足元に罠があれば発動する。浮遊中は、どの罠も作動しない。
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
                self.note(&format!(
                    "毒矢の罠だ！ 2のダメージを受けた。(HP {}/{})",
                    self.hp.max(0),
                    self.max_hp
                ));
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
    fn disarm(&mut self, d: Option<Dir>) -> ActionResult {
        if self.status.has(Status::Blind) {
            return ActionResult::failure("目が見えなくて、罠をいじれない。");
        }
        let target = match d {
            Some(d) => (self.pos.0 + d.delta().0, self.pos.1 + d.delta().1),
            None => {
                // 向きを省いたときは足元。足元に見つけた罠がなければ、隣にある見つけた罠を探す
                let here = self.known_trap_at(self.pos).map(|_| self.pos);
                let near: Vec<(i32, i32)> = self
                    .traps
                    .iter()
                    .filter(|t| {
                        t.revealed
                            && t.pos != self.pos
                            && (t.pos.0 - self.pos.0).abs() <= 1
                            && (t.pos.1 - self.pos.1).abs() <= 1
                    })
                    .map(|t| t.pos)
                    .collect();
                match (here, near.as_slice()) {
                    (Some(p), _) => p,
                    (None, [p]) => *p,
                    (None, []) => self.pos,
                    (None, _) => {
                        return ActionResult::failure(
                            "隣に見つけた罠が複数ある。向きを指定しよう。(例: disarm east)",
                        )
                    }
                }
            }
        };
        let Some(ti) = self
            .traps
            .iter()
            .position(|t| t.pos == target && t.revealed)
        else {
            return ActionResult::failure(
                "そこには見つけた罠がない。(隠れた罠は解除できない。近くに立って探そう)",
            );
        };
        let kind = self.traps[ti].kind;
        let percent = self.disarm_percent();
        if self.rng.range(0, 100) < percent {
            self.traps.remove(ti);
            self.map.mark_seen(target.0, target.1);
            return ActionResult::success(
                format!("{}を解除した！ (成功率{percent}%)", kind.name()),
                true,
            );
        }
        let mut msg = format!("{}の解除に失敗した。(成功率{percent}%)", kind.name());
        // 失敗すると3回に1回は作動する。足元の罠以外の落とし穴は、落ちずに済む
        if self.rng.range(0, 3) == 0 {
            // 触っている仕掛けが動くので、毒矢と眠りガスは浮いていても作動する。落とし穴は浮いていれば落ちない
            if kind == TrapKind::Trapdoor && self.status.has(Status::Levitating) {
                msg.push_str(" 手元が狂ったが、浮いているので落ちなかった。");
            } else if kind == TrapKind::Trapdoor && target != self.pos {
                msg.push_str(" 床板がきしんだが、落ちずに済んだ。");
            } else {
                msg.push_str(" 手元が狂って罠が作動した！");
                self.fire_trap(ti);
            }
        }
        ActionResult::success(msg, true)
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
        self.traps
            .iter()
            .copied()
            .find(|t| t.pos == p && t.revealed)
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
        let drain =
            if fx.slow_digestion > 0 && !self.turn.is_multiple_of(fx.slow_digestion as u32 + 1) {
                0
            } else if self.armor_suffix() == Some(Suffix::Famine) && self.turn.is_multiple_of(2) {
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
            let msg = format!(
                "{cause}で1ダメージ。(HP {}/{})",
                self.hp.max(0),
                self.max_hp
            );
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
            let net =
                kind.slow as i32 + st.has(Status::Slowed) as i32 - st.has(Status::Hasted) as i32;
            if net > 0 && !self.turn.is_multiple_of(1 << net) {
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
            .filter(|&p| {
                self.map.tile(p.0, p.1).walkable() && p != self.pos && self.monster_at(p).is_none()
            })
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
        if (kind.erratic && !self.monsters[i].cancelled && self.rng.range(0, 3) == 0)
            || (lost && self.rng.range(0, 2) == 0)
        {
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
            let curse = self
                .weapon_gear()
                .and_then(|g| g.suffix)
                .map_or(0, Suffix::damage_taken_bonus);
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
                    self.note(&format!(
                        "トゲが{name}に1ダメージを返し、倒した！ (経験値 +{xp})"
                    ));
                } else {
                    self.note(&format!("トゲが{name}に1ダメージを返した。"));
                }
            }
            // Thorns で倒された敵は、毒を撒けない
            if kind.poisons
                && !self.monsters[i].cancelled
                && self.hp > 0
                && self.monsters[i].hp > 0
                && self.rng.range(0, 2) == 0
            {
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
            msg.push_str(&format!(
                " 武器が生命を吸った。HP+{gained} (HP {}/{})",
                self.hp, self.max_hp
            ));
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
                "クリア済み。:new でもう一度遊べる。".to_string(),
            );
        }
        if self.dead {
            return self.outcome(
                cmd.to_string(),
                false,
                "ゲームオーバー。:new でやり直せる。".to_string(),
            );
        }
        let pos_before = self.pos;
        // (成功か, メッセージ, このコマンド自身が1ターン消費するか)
        let ActionResult {
            ok,
            mut message,
            spent,
        } = match cmd {
            Command::Move(d) => {
                let (d, reeled) = self.confuse_dir(d);
                let lead = if reeled {
                    format!("混乱して{}へ向かってしまった。 ", d.name())
                } else {
                    String::new()
                };
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                if let Some(i) = self.monster_at(t) {
                    ActionResult::success(format!("{lead}{}", self.attack_monster(i)), true)
                } else if self.map.tile(t.0, t.1).walkable() {
                    self.pos = t;
                    self.refresh_fov();
                    if self.pos == self.stairs {
                        let hint = if self.has_amulet {
                            "ascend で登れる"
                        } else {
                            "descend で降りられる"
                        };
                        ActionResult::success(format!("{lead}階段の上にいる。({hint})"), true)
                    } else {
                        ActionResult::success(format!("{lead}{}へ進んだ。", d.name()), true)
                    }
                } else if reeled {
                    // 混乱してぶつかったときは、ターンを使う
                    ActionResult::success(format!("{lead}壁にぶつかった。"), true)
                } else {
                    ActionResult::failure("壁にぶつかった。")
                }
            }
            Command::Attack(d) => {
                let (d, reeled) = self.confuse_dir(d);
                let lead = if reeled {
                    format!("混乱して{}を攻撃してしまった。 ", d.name())
                } else {
                    String::new()
                };
                let (dx, dy) = d.delta();
                let t = (self.pos.0 + dx, self.pos.1 + dy);
                match self.monster_at(t) {
                    Some(i) => {
                        ActionResult::success(format!("{lead}{}", self.attack_monster(i)), true)
                    }
                    None if reeled => ActionResult::success(format!("{lead}空振りした。"), true),
                    None => ActionResult::failure("そこには何もいない。"),
                }
            }
            Command::Descend => {
                if self.has_amulet {
                    ActionResult::failure(
                        "アミュレットを持っていると、階段は登り階段だ。ascend で登ろう。",
                    )
                } else if self.pos != self.stairs {
                    ActionResult::failure("ここに階段はない。")
                } else if self.depth >= AMULET_DEPTH {
                    ActionResult::failure("ここが最深部だ。魔除けのアミュレットを探そう。")
                } else {
                    self.depth += 1;
                    self.new_level();
                    ActionResult::success(format!("地下{}階に降りた。", self.depth), true)
                }
            }
            Command::Ascend => {
                if !self.has_amulet {
                    ActionResult::failure(
                        "登り階段はない。アミュレットを手に入れると、階段が登り階段になる。",
                    )
                } else if self.pos != self.stairs {
                    ActionResult::failure("ここに階段はない。")
                } else if self.depth == 1 {
                    self.won = true;
                    ActionResult::new(
                        true,
                        "地上の光が見えた！ 魔除けのアミュレットを持ち帰り、地上へ脱出した。クリア！",
                        false,
                    )
                } else {
                    self.depth -= 1;
                    self.new_level();
                    ActionResult::success(format!("地下{}階へ登った。", self.depth), true)
                }
            }
            Command::Wait => ActionResult::success("1ターン待った。", true),
            Command::Stay(n) => {
                let (ok, msg) = self.stay(n);
                ActionResult::new(ok, msg, false)
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
                ActionResult::new(true, msg, false)
            }
            Command::Look => ActionResult::new(true, self.describe_surroundings(), false),
            Command::Disarm(d) => self.disarm(d),
            Command::Travel(TravelTarget::Stairs) => {
                let (ok, msg) = self.travel_to_stairs();
                ActionResult::new(ok, msg, false)
            }
            Command::Explore => {
                let (ok, msg) = self.explore();
                ActionResult::new(ok, msg, false)
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
            return (
                false,
                "階段の場所をまだ知らない。explore で探そう。".to_string(),
            );
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
                    format!(
                        "階段へ向かう途中({n}歩)、進路上に{}がいる。",
                        self.foe_name(i)
                    ),
                );
            }
            // 浮遊が途中で切れて、次のマスが見つけた罠なら、踏む前に止まる
            if let Some(t) = self
                .known_trap_at(p)
                .filter(|_| !self.status.has(Status::Levitating))
            {
                return (
                    true,
                    format!("階段へ向かう途中({n}歩)、進路上に{}がある。", t.kind.name()),
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
                format!(
                    "[{}]",
                    e.statuses
                        .iter()
                        .map(|(s, n)| format!("{}{n}", s.name()))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
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
                parts.push(format!(
                    "{} {}が{}にある。",
                    crate::trap::TRAP_GLYPH,
                    t.kind.name(),
                    rel_text(self.pos, t.pos)
                ));
            }
        }
        if let Some(p) = self.amulet.filter(|p| self.map.is_seen(p.0, p.1)) {
            parts.push(format!(
                ", 魔除けのアミュレットが{}にある。",
                rel_text(self.pos, p)
            ));
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
            return Cell {
                ch,
                visible: false,
                seen: true,
            };
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
                s.push_str(&format!(
                    "{}: 残り{n}ターン — {}\n",
                    st.name(),
                    st.def().effect
                ));
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
                        e.statuses
                            .iter()
                            .map(|(s, n)| format!("{}{n}", s.name()))
                            .collect::<Vec<_>>()
                            .join(" ")
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
mod tests;
