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
    // ここから食べ物。パンと干し肉は名前が分かる。キノコは見た目だけでは分からない
    Bread,
    Jerky,
    EdibleShroom,
    PoisonShroom,
    VigorShroom,
}

impl ItemKind {
    pub const COUNT: usize = 17;
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
        ItemKind::Bread,
        ItemKind::Jerky,
        ItemKind::EdibleShroom,
        ItemKind::PoisonShroom,
        ItemKind::VigorShroom,
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
            ItemKind::Bread => "パン",
            ItemKind::Jerky => "干し肉",
            ItemKind::EdibleShroom => "食用キノコ",
            ItemKind::PoisonShroom => "毒キノコ",
            ItemKind::VigorShroom => "元気キノコ",
        }
    }

    pub fn is_scroll(self) -> bool {
        matches!(self, ItemKind::Identify | ItemKind::MagicMap | ItemKind::Teleport)
    }

    /// 名前が最初から分かる食べ物（パン・干し肉）。
    pub fn is_food(self) -> bool {
        matches!(self, ItemKind::Bread | ItemKind::Jerky)
    }

    /// 見た目だけでは正体が分からないキノコ。
    pub fn is_mushroom(self) -> bool {
        matches!(
            self,
            ItemKind::EdibleShroom | ItemKind::PoisonShroom | ItemKind::VigorShroom
        )
    }

    /// 食べると回復する満腹度。
    pub fn nutrition(self) -> i32 {
        match self {
            ItemKind::Bread => 150,
            ItemKind::Jerky => 100,
            ItemKind::EdibleShroom => 60,
            ItemKind::PoisonShroom => 20,
            ItemKind::VigorShroom => 30,
            _ => 0,
        }
    }

    pub fn is_weapon(self) -> bool {
        matches!(self, ItemKind::Dagger | ItemKind::Sword | ItemKind::Axe)
    }

    pub fn is_armor(self) -> bool {
        matches!(self, ItemKind::Leather | ItemKind::Chain | ItemKind::Plate)
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
        if let Some((lo, hi)) = self.weapon_dmg() {
            format!("攻撃 {lo}〜{hi}")
        } else if self.is_armor() {
            format!("防御 {}", self.armor())
        } else {
            String::new()
        }
    }

    /// この深さから床に現れる。
    pub fn min_depth(self) -> u32 {
        match self {
            ItemKind::Sword | ItemKind::Chain | ItemKind::Jerky | ItemKind::VigorShroom => 2,
            ItemKind::Axe => 3,
            ItemKind::Plate => 4,
            _ => 1,
        }
    }

    /// マップ上の記号。薬は `!`、巻物は `?`、武器は `)`、防具は `[`、食べ物とキノコは `%`。
    pub fn glyph(self) -> char {
        if self.is_potion() {
            '!'
        } else if self.is_weapon() {
            ')'
        } else if self.is_armor() {
            '['
        } else if self.is_food() || self.is_mushroom() {
            '%'
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
            ItemKind::Bread => 4,
            ItemKind::Jerky => 1,
            ItemKind::EdibleShroom => 2,
            ItemKind::PoisonShroom => 1,
            ItemKind::VigorShroom => 1,
        }
    }
}

/// 薬の見た目の候補。ここから種類の数だけ選んで割り当てる。
pub const POTION_LOOKS: [&str; 5] = ["赤い薬", "青い薬", "緑の薬", "黄色い薬", "白い薬"];

/// キノコの見た目の候補。
pub const MUSHROOM_LOOKS: [&str; 4] = ["赤いキノコ", "白いキノコ", "茶色いキノコ", "斑点のキノコ"];

/// 巻物の見た目の候補。
pub const SCROLL_LOOKS: [&str; 5] = [
    "「ルク」の巻物",
    "「ネム」の巻物",
    "「ザラ」の巻物",
    "「ポロ」の巻物",
    "「ミト」の巻物",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_variant_in_index_order() {
        for (i, k) in ItemKind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i, "{k:?}");
        }
        // 最後のバリアント (VigorShroom) の番号が COUNT-1 なら、数え間違いはない
        assert_eq!(ItemKind::VigorShroom.index(), ItemKind::COUNT - 1);
    }

    #[test]
    fn gear_stats_are_consistent_with_its_category() {
        for k in ItemKind::ALL {
            assert_eq!(k.weapon_dmg().is_some(), k.is_weapon(), "{k:?}");
            assert_eq!(k.armor() > 0, k.is_armor(), "{k:?}");
            assert!(k.weight() > 0, "{k:?}");
            if k.is_equipment() {
                assert!(!k.is_potion() && !k.stats_text().is_empty(), "{k:?}");
            }
        }
    }
}
