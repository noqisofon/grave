//! アイテムの種類。薬と巻物は、ゲームごとに見た目（未識別名）がシャッフルされる。

use crate::rng::Rng;

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

    /// 装備品の基本名（接頭辞が付く前）。
    pub fn base_name(self) -> &'static str {
        match self {
            ItemKind::Dagger => "Dagger",
            ItemKind::Sword => "Sword",
            ItemKind::Axe => "Axe",
            ItemKind::Leather => "Leather Armor",
            ItemKind::Chain => "Chain Mail",
            ItemKind::Plate => "Plate Armor",
            _ => self.true_name(),
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

/// 装備の品質ランク。接頭辞の語がランクを表す。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Quality {
    Common,
    Uncommon,
    Rare,
    Ancient,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Common, Quality::Uncommon, Quality::Rare, Quality::Ancient];

    /// このランクを表す接頭辞の候補。
    pub fn words(self) -> &'static [&'static str] {
        match self {
            Quality::Common => &["Basic", "Okay", "Regular", "Usual"],
            Quality::Uncommon => &["Superior", "Prime", "First Rate"],
            Quality::Rare => &["Mystical", "Sanctified", "Glorious"],
            Quality::Ancient => &["Eldritch", "Primeval", "Legendary"],
        }
    }

    /// 攻撃・防御の加算値の幅 (最小, 最大)。Common は補正なし。
    /// 語からは幅しか分からず、個体の正確な値は識別するまで分からない。
    pub fn bonus_range(self) -> (i32, i32) {
        match self {
            Quality::Common => (0, 0),
            Quality::Uncommon => (1, 2),
            Quality::Rare => (2, 3),
            Quality::Ancient => (3, 5),
        }
    }

    /// 接尾辞が付く確率（%）。
    fn suffix_percent(self) -> i32 {
        match self {
            Quality::Common => 0,
            Quality::Uncommon => 30,
            Quality::Rare => 60,
            Quality::Ancient => 100,
        }
    }

    /// 深さごとの出現の重み (Common, Uncommon, Rare, Ancient)。合計は常に 100。
    pub fn weights(depth: u32) -> [i32; 4] {
        let d = depth as i32;
        let uncommon = (10 + 3 * d).min(40);
        let rare = if d >= 3 { ((d - 2) * 2).min(25) } else { 0 };
        let ancient = if d >= 8 { (d - 7).min(10) } else { 0 };
        [100 - uncommon - rare - ancient, uncommon, rare, ancient]
    }
}

/// 接尾辞（`of X`）。装備に付く特殊効果。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Suffix {
    // 武器
    /// 命中するたびにHPが1回復する
    Vampire,
    /// 攻撃が+1される
    Might,
    /// 敵を倒すとHPが2回復する
    Vigor,
    /// 呪い: 攻撃が+3されるが、装備すると外せなくなる
    Cataclysm,
    // 防具
    /// 近接で殴ってきた敵に1ダメージを返す
    Thorns,
    /// 毒を受けない
    Warding,
    /// 呪い: 防御が+2されるが、満腹度が余計に減る
    Famine,
}

impl Suffix {
    pub const WEAPON: [(Suffix, i32); 4] = [
        (Suffix::Vampire, 2),
        (Suffix::Might, 2),
        (Suffix::Vigor, 2),
        (Suffix::Cataclysm, 1),
    ];
    pub const ARMOR: [(Suffix, i32); 3] = [(Suffix::Thorns, 2), (Suffix::Warding, 2), (Suffix::Famine, 1)];

    pub fn name(self) -> &'static str {
        match self {
            Suffix::Vampire => "of the Vampire",
            Suffix::Might => "of Might",
            Suffix::Vigor => "of Vigor",
            Suffix::Cataclysm => "of the Cataclysm",
            Suffix::Thorns => "of Thorns",
            Suffix::Warding => "of Warding",
            Suffix::Famine => "of Famine",
        }
    }

    /// 効果の1行説明。
    pub fn describe(self) -> &'static str {
        match self {
            Suffix::Vampire => "命中するたびにHP+1",
            Suffix::Might => "攻撃+1",
            Suffix::Vigor => "敵を倒すとHP+2",
            Suffix::Cataclysm => "攻撃+3。呪われていて、装備すると外せない",
            Suffix::Thorns => "殴ってきた敵に1ダメージを返す",
            Suffix::Warding => "毒を受けない",
            Suffix::Famine => "防御+2。呪われていて、満腹度が余計に減る",
        }
    }

    /// 呪い。装備して初めて分かる。
    pub fn is_cursed(self) -> bool {
        matches!(self, Suffix::Cataclysm | Suffix::Famine)
    }

    /// 攻撃への加算（武器）。
    pub fn attack_bonus(self) -> i32 {
        match self {
            Suffix::Might => 1,
            Suffix::Cataclysm => 3,
            _ => 0,
        }
    }

    /// 防御への加算（防具）。
    pub fn defense_bonus(self) -> i32 {
        match self {
            Suffix::Famine => 2,
            _ => 0,
        }
    }

    fn roll(rng: &mut Rng, weapon: bool) -> Suffix {
        let pool: &[(Suffix, i32)] = if weapon { &Suffix::WEAPON } else { &Suffix::ARMOR };
        let total: i32 = pool.iter().map(|(_, w)| w).sum();
        let mut r = rng.range(0, total);
        for (s, w) in pool {
            if r < *w {
                return *s;
            }
            r -= w;
        }
        pool[0].0
    }
}

/// 装備品の1個体。装備は1個ずつ別物なので、スタックしない。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gear {
    pub kind: ItemKind,
    pub quality: Quality,
    /// 接頭辞の語（`quality.words()` のどれか）
    pub word: &'static str,
    /// 攻撃・防御への加算値（品質ランクの幅の中。識別するまで正確な値は分からない）
    pub bonus: i32,
    /// 特殊効果（Uncommon 以上で付きうる）
    pub suffix: Option<Suffix>,
    /// 接尾辞と正確な補正値を知っているか。Common は隠すものがないので最初から true
    pub identified: bool,
    /// 装備して過ごしたターン数（一定に達すると識別される）
    pub worn: u32,
}

impl Gear {
    /// 何の変哲もない装備 (Common)。
    pub fn plain(kind: ItemKind) -> Gear {
        debug_assert!(kind.is_equipment());
        Gear {
            kind,
            quality: Quality::Common,
            word: Quality::Common.words()[0],
            bonus: 0,
            suffix: None,
            identified: true,
            worn: 0,
        }
    }

    /// ゲームの乱数から個体を作る。深い階ほど上位ランクが出やすい。
    pub fn roll(rng: &mut Rng, kind: ItemKind, depth: u32) -> Gear {
        let w = Quality::weights(depth);
        let mut r = rng.range(0, 100);
        let mut quality = Quality::Common;
        for (q, wt) in Quality::ALL.iter().zip(w) {
            if r < wt {
                quality = *q;
                break;
            }
            r -= wt;
        }
        let words = quality.words();
        let word = words[rng.range(0, words.len() as i32) as usize];
        let (lo, hi) = quality.bonus_range();
        let bonus = rng.range(lo, hi + 1);
        let suffix = if quality.suffix_percent() > 0 && rng.range(0, 100) < quality.suffix_percent() {
            Some(Suffix::roll(rng, kind.is_weapon()))
        } else {
            None
        };
        Gear {
            kind,
            quality,
            word,
            bonus,
            suffix,
            identified: quality == Quality::Common,
            worn: 0,
        }
    }

    /// 呪われているか。
    pub fn is_cursed(&self) -> bool {
        self.suffix.is_some_and(Suffix::is_cursed)
    }

    /// 品質補正と接尾辞を合わせた攻撃・防御の加算値。
    fn total_bonus(&self) -> i32 {
        match self.suffix {
            Some(s) if self.kind.is_weapon() => self.bonus + s.attack_bonus(),
            Some(s) => self.bonus + s.defense_bonus(),
            None => self.bonus,
        }
    }

    pub fn name(&self) -> String {
        let mut n = format!("{} {}", self.word, self.kind.base_name());
        if !self.identified {
            // 接尾辞があるのか、補正値がいくつなのかは、識別するまで分からない
            n.push_str(" (?)");
        } else if let Some(s) = self.suffix {
            n.push(' ');
            n.push_str(s.name());
        }
        n
    }

    /// 識別したときに分かる中身の説明（接尾辞の効果と、補正値）。
    pub fn reveal_text(&self) -> String {
        let fx = self.suffix.map_or("特殊効果はない".to_string(), |s| s.describe().to_string());
        format!("{}。品質補正 +{}", fx, self.bonus)
    }

    /// 武器の攻撃範囲 (最小, 最大)。武器でなければ None。
    pub fn weapon_range(&self) -> Option<(i32, i32)> {
        self.kind
            .weapon_dmg()
            .map(|(lo, hi)| (lo + self.total_bonus(), hi + self.total_bonus()))
    }

    /// 防具の防御値。防具でなければ 0。
    pub fn armor_value(&self) -> i32 {
        if self.kind.is_armor() {
            self.kind.armor() + self.total_bonus()
        } else {
            0
        }
    }

    /// 未識別の個体の、見える範囲での性能の説明。基本値に、品質ランクの補正の幅を添える。
    pub fn guess_text(&self) -> String {
        let (lo, hi) = self.quality.bonus_range();
        let range = if lo == hi { format!("+{lo}") } else { format!("+({lo}〜{hi})") };
        match self.kind.weapon_dmg() {
            Some((a, b)) => format!("攻撃 {a}〜{b} {range}?"),
            None => format!("防御 {} {range}?", self.kind.armor()),
        }
    }

    /// 性能の説明（品質補正を含む）。
    pub fn stats_text(&self) -> String {
        if let Some((lo, hi)) = self.weapon_range() {
            format!("攻撃 {lo}〜{hi}")
        } else {
            format!("防御 {}", self.armor_value())
        }
    }
}

impl Item {
    /// 床に置くために、種類から品物を作る。装備なら個体を抽選する。
    pub fn roll(rng: &mut Rng, kind: ItemKind, depth: u32) -> Item {
        if kind.is_equipment() {
            Item::Gear(Gear::roll(rng, kind, depth))
        } else {
            Item::Plain(kind)
        }
    }
}

/// 床に落ちているもの・持ち物になるもの。装備は個体、それ以外は種類だけ。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Plain(ItemKind),
    Gear(Gear),
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        match self {
            Item::Plain(k) => *k,
            Item::Gear(g) => g.kind,
        }
    }
}

impl From<ItemKind> for Item {
    fn from(kind: ItemKind) -> Item {
        if kind.is_equipment() {
            Item::Gear(Gear::plain(kind))
        } else {
            Item::Plain(kind)
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
