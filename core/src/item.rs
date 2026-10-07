//! アイテムの種類。薬と巻物は、ゲームごとに見た目（未識別名）がシャッフルされる。

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ItemKind {
    /// HPを回復する
    Healing,
    /// ダメージを受ける
    Poison,
    /// 眠って数ターン無防備になる
    Sleep,
    /// 未識別の持ち物を1つ識別する
    Identify,
    /// フロアの地図が分かる
    MagicMap,
    /// ランダムな場所へ移動する
    Teleport,
    // ここから装備品。見た目の偽装はなく、最初から名前が分かる
    Dagger,
    Sword,
    Axe,
    Leather,
    Chain,
    Plate,
}

impl ItemKind {
    pub const COUNT: usize = 12;
    pub const ALL: [ItemKind; ItemKind::COUNT] = [
        ItemKind::Healing,
        ItemKind::Poison,
        ItemKind::Sleep,
        ItemKind::Identify,
        ItemKind::MagicMap,
        ItemKind::Teleport,
        ItemKind::Dagger,
        ItemKind::Sword,
        ItemKind::Axe,
        ItemKind::Leather,
        ItemKind::Chain,
        ItemKind::Plate,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn is_potion(self) -> bool {
        matches!(self, ItemKind::Healing | ItemKind::Poison | ItemKind::Sleep)
    }

    pub fn true_name(self) -> &'static str {
        match self {
            ItemKind::Healing => "回復の薬",
            ItemKind::Poison => "毒の薬",
            ItemKind::Sleep => "眠りの薬",
            ItemKind::Identify => "識別の巻物",
            ItemKind::MagicMap => "地図の巻物",
            ItemKind::Teleport => "転移の巻物",
            ItemKind::Dagger => "短剣",
            ItemKind::Sword => "剣",
            ItemKind::Axe => "斧",
            ItemKind::Leather => "革の鎧",
            ItemKind::Chain => "鎖かたびら",
            ItemKind::Plate => "板金鎧",
        }
    }

    pub fn is_weapon(self) -> bool {
        self.weapon_dmg().is_some()
    }

    pub fn is_armor(self) -> bool {
        self.armor() > 0
    }

    pub fn is_equipment(self) -> bool {
        self.is_weapon() || self.is_armor()
    }

    /// 武器の与えるダメージ (最小, 最大)。どちらも含む。
    pub fn weapon_dmg(self) -> Option<(i32, i32)> {
        match self {
            ItemKind::Dagger => Some((3, 5)),
            ItemKind::Sword => Some((4, 7)),
            ItemKind::Axe => Some((5, 9)),
            _ => None,
        }
    }

    /// 防具が敵のダメージを減らす量（ただしダメージは最低1）。
    pub fn armor(self) -> i32 {
        match self {
            ItemKind::Leather => 1,
            ItemKind::Chain => 2,
            ItemKind::Plate => 3,
            _ => 0,
        }
    }

    /// 装備品の性能の説明。
    pub fn stats_text(self) -> String {
        match (self.weapon_dmg(), self.armor()) {
            (Some((lo, hi)), _) => format!("攻撃 {lo}〜{hi}"),
            (None, a) if a > 0 => format!("防御 {a}"),
            _ => String::new(),
        }
    }

    /// この深さから床に現れる。
    pub fn min_depth(self) -> u32 {
        match self {
            ItemKind::Sword | ItemKind::Chain => 2,
            ItemKind::Axe => 3,
            ItemKind::Plate => 4,
            _ => 1,
        }
    }

    /// マップ上の記号。薬は `!`、巻物は `?`、武器は `)`、防具は `[`。
    pub fn glyph(self) -> char {
        if self.is_potion() {
            '!'
        } else if self.is_weapon() {
            ')'
        } else if self.is_armor() {
            '['
        } else {
            '?'
        }
    }

    /// 床への出やすさ（相対的な重み）。
    pub fn weight(self) -> u32 {
        match self {
            ItemKind::Healing => 3,
            ItemKind::Poison => 2,
            ItemKind::Sleep => 2,
            ItemKind::Identify => 2,
            ItemKind::MagicMap => 2,
            ItemKind::Teleport => 1,
            ItemKind::Dagger => 2,
            ItemKind::Leather => 2,
            ItemKind::Sword => 1,
            ItemKind::Chain => 1,
            ItemKind::Axe => 1,
            ItemKind::Plate => 1,
        }
    }
}

/// 薬の見た目の候補。ここから種類の数だけ選んで割り当てる。
pub const POTION_LOOKS: [&str; 5] = ["赤い薬", "青い薬", "緑の薬", "黄色い薬", "白い薬"];

/// 巻物の見た目の候補。
pub const SCROLL_LOOKS: [&str; 5] = [
    "「ルク」の巻物",
    "「ネム」の巻物",
    "「ザラ」の巻物",
    "「ポロ」の巻物",
    "「ミト」の巻物",
];
