//! 敵の種類。

pub struct MonsterKind {
    pub name: &'static str,
    pub glyph: char,
    /// 深さ1でのHP。深くなるごとに `hp_per_depth` ずつ増える
    pub base_hp: i32,
    pub hp_per_depth: i32,
    /// 攻撃ダメージ (最小, 最大)。深いほど最大がじわじわ増える
    pub dmg: (i32, i32),
    /// 1ターンに行動する回数
    pub actions_per_turn: u32,
    /// 2ターンに1回しか動けない
    pub slow: bool,
    /// ときどき不規則に飛び回る
    pub erratic: bool,
    /// 当たると毒を受けることがある
    pub poisons: bool,
    /// この深さから現れる
    pub min_depth: u32,
    /// 出やすさ（相対的な重み）
    pub weight: u32,
    /// 倒したときの経験値の基本値（深い階ほど上乗せされる）
    pub xp: u32,
}

/// 基本の敵。
pub static SLIME: MonsterKind = MonsterKind {
    name: "スライム",
    glyph: 's',
    base_hp: 6,
    hp_per_depth: 2,
    dmg: (1, 2),
    actions_per_turn: 1,
    slow: false,
    erratic: false,
    poisons: false,
    min_depth: 1,
    weight: 5,
    xp: 3,
};

/// 倒しやすいが、素早く2回動き、ふらふら飛び回る。
pub static BAT: MonsterKind = MonsterKind {
    name: "コウモリ",
    glyph: 'b',
    base_hp: 3,
    hp_per_depth: 1,
    dmg: (1, 2),
    actions_per_turn: 2,
    slow: false,
    erratic: true,
    poisons: false,
    min_depth: 1,
    weight: 3,
    xp: 2,
};

/// 普通の速さだが、殴られると痛い。
pub static GOBLIN: MonsterKind = MonsterKind {
    name: "ゴブリン",
    glyph: 'g',
    base_hp: 9,
    hp_per_depth: 2,
    dmg: (2, 4),
    actions_per_turn: 1,
    slow: false,
    erratic: false,
    poisons: false,
    min_depth: 2,
    weight: 3,
    xp: 6,
};

/// 2ターンに1回しか動けないが、一撃が重くて硬い。
pub static OGRE: MonsterKind = MonsterKind {
    name: "オーガ",
    glyph: 'O',
    base_hp: 10,
    hp_per_depth: 3,
    dmg: (3, 6),
    actions_per_turn: 1,
    slow: true,
    erratic: false,
    poisons: false,
    min_depth: 4,
    weight: 1,
    xp: 12,
};

/// 噛まれると毒を受けることがある。
pub static SPIDER: MonsterKind = MonsterKind {
    name: "毒グモ",
    glyph: 'S',
    base_hp: 5,
    hp_per_depth: 2,
    dmg: (1, 2),
    actions_per_turn: 1,
    slow: false,
    erratic: false,
    poisons: true,
    min_depth: 3,
    weight: 2,
    xp: 5,
};

pub static KINDS: [&MonsterKind; 5] = [&SLIME, &BAT, &GOBLIN, &OGRE, &SPIDER];

impl MonsterKind {
    pub fn hp_at(&self, depth: u32) -> i32 {
        self.base_hp + self.hp_per_depth * (depth as i32 - 1)
    }
}
