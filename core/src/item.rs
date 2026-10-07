//! アイテムの種類。薬と巻物は、ゲームごとに見た目（未識別名）がシャッフルされる。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
}

impl ItemKind {
    pub const ALL: [ItemKind; 6] = [
        ItemKind::Healing,
        ItemKind::Poison,
        ItemKind::Sleep,
        ItemKind::Identify,
        ItemKind::MagicMap,
        ItemKind::Teleport,
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
        }
    }

    /// マップ上の記号。薬は `!`、巻物は `?`。
    pub fn glyph(self) -> char {
        if self.is_potion() {
            '!'
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
