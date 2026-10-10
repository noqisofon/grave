//! アイテムの種類と効果。薬・巻物は、ゲームごとに見た目（未識別名）がシャッフルされる。
//!
//! 種類ごとの性質は `ITEMS` の表（データ）で決まる。薬と巻物の効果は `Effect` の並びで書けるので、
//! 新しい薬や巻物は「`ItemKind` に1つ足し、`ITEMS` に1行足す」だけで増やせる。

use crate::rng::Rng;
use crate::status::Status;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ItemKind {
    // ---- 薬（良い）----
    Healing,
    ExtraHealing,
    Strength,
    /// レベルアップの薬
    Experience,
    RestoreStrength,
    Haste,
    DetectMonsters,
    DetectItems,
    SeeInvisible,
    Levitation,
    Antidote,
    // ---- 薬（悪い）----
    Confusion,
    Hallucination,
    Poison,
    Blindness,
    Sleep,
    // ---- 巻物 ----
    Identify,
    MagicMap,
    Teleport,
    EnchantWeapon,
    EnchantArmor,
    RemoveCurse,
    ProtectArmor,
    ConfuseMonster,
    HoldMonster,
    ScareMonster,
    CreateMonster,
    Aggravate,
    /// 読むと眠ってしまう巻物
    Slumber,
    // ---- 杖（回数制）----
    WandLight,
    WandInvisibility,
    WandLightning,
    WandFire,
    WandCold,
    WandPolymorph,
    WandMissile,
    WandHaste,
    WandSlow,
    WandDrain,
    WandCancel,
    WandTeleportOther,
    WandTeleportSelf,
    // ---- 指輪（装備スロットは2つ）。呪われたものは外せない ----
    RingProtection,
    RingStrength,
    RingDexterity,
    RingDamage,
    RingRegeneration,
    RingSlowDigestion,
    RingStealth,
    RingSearching,
    RingSeeInvisible,
    RingTrinket,
    RingAggravate,
    RingTeleportitis,
    // ---- 光源と燃料 ----
    Torch,
    Lantern,
    OilFlask,
    // ---- 装備品。見た目の偽装はなく、最初から名前が分かる ----
    Dagger,
    Sword,
    Axe,
    Leather,
    Chain,
    Plate,
    // ---- 食べ物。パンと干し肉は名前が分かる。キノコは見た目だけでは分からない ----
    Bread,
    Jerky,
    EdibleShroom,
    PoisonShroom,
    VigorShroom,
}

/// アイテムの大きな分類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Potion,
    Scroll,
    Weapon,
    Armor,
    Food,
    Mushroom,
    /// 杖。使用回数（充填数）がある
    Wand,
    /// 指輪。身につけると効果が続く
    Ring,
    /// 光源。燃料が減っていく
    Light,
    /// 燃料（ランタンに継ぎ足す油）
    Fuel,
}

/// 指輪の効き方。数値 `n` は個体ごとの強さ（呪われた指輪では負）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RingEffect {
    /// 防御 +n
    Protection,
    /// 腕力 +n
    Strength,
    /// n>0: 敵の攻撃を10%×n で避ける / n<0: こちらの攻撃が10%×|n| で空振りする
    Dexterity,
    /// 攻撃 +n
    Damage,
    /// HPの自然回復が速くなる
    Regeneration,
    /// 満腹度の減りが 1/(n+1) になる
    SlowDigestion,
    /// 敵に気づかれる距離が 3×n 縮む
    Stealth,
    /// 隠れた罠を見つけやすくなる
    Searching,
    SeeInvisible,
    /// 効果のない装飾
    Trinket,
    /// 呪い: 階じゅうの敵を引き寄せる
    Aggravate,
    /// 呪い: ときどきランダムに飛ばされる
    Teleportitis,
}

impl RingEffect {
    /// 強さ `n` のときの効果の説明。
    pub fn describe(self, n: i32) -> String {
        match self {
            RingEffect::Protection => format!("防御{n:+}"),
            RingEffect::Strength => format!("腕力{n:+}"),
            RingEffect::Dexterity if n >= 0 => {
                format!("敵の攻撃を{}%の確率でかわす", (10 * n).min(50))
            }
            RingEffect::Dexterity => {
                format!("こちらの攻撃が{}%の確率で空振りする", (-10 * n).min(30))
            }
            RingEffect::Damage => format!("攻撃{n:+}"),
            RingEffect::Regeneration => "HPの自然回復が速くなる(敵がいても回復する)".to_string(),
            RingEffect::SlowDigestion => format!("満腹度の減りが{}分の1になる", n + 1),
            RingEffect::Stealth => format!("敵に気づかれる距離が{}縮む", 3 * n),
            RingEffect::Searching => "隠れた罠を見つけやすくなる".to_string(),
            RingEffect::SeeInvisible => "透明な敵が見える".to_string(),
            RingEffect::Trinket => "効果はない(ただの飾り)".to_string(),
            RingEffect::Aggravate => "階じゅうの敵がこちらに引き寄せられる".to_string(),
            RingEffect::Teleportitis => "ときどきランダムな場所へ飛ばされる".to_string(),
        }
    }

    /// 強さの数字を名前に添える種類か（効果が数値で表せるもの）。
    pub fn has_magnitude(self) -> bool {
        !matches!(
            self,
            RingEffect::Trinket
                | RingEffect::Aggravate
                | RingEffect::Teleportitis
                | RingEffect::SeeInvisible
        )
    }
}

/// 指輪の仕様。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RingDef {
    pub effect: RingEffect,
    /// 呪われた（負の値の）個体が見つかることがある
    pub cursable: bool,
    /// 必ず呪われている
    pub always_cursed: bool,
}

/// 光源の仕様。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LightDef {
    /// 燃料の最大値（ターン数）
    pub max_fuel: i32,
    /// 油を継ぎ足せるか
    pub refillable: bool,
}

/// 油つぼ1個で足せる燃料。
pub const OIL_FLASK_FUEL: i32 = 1500;
/// 燃料が尽きたときの視界の半径。
pub const DARK_RADIUS: i32 = 2;

/// 杖を振ったときの効果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZapFx {
    /// 向きの先を照らして、地図に書き込む
    Light,
    /// ダメージ。`beam` なら線上の敵すべてに、そうでなければ最初の1体に。`drain` なら与えたぶん回復する
    Damage {
        lo: i32,
        hi: i32,
        beam: bool,
        drain: bool,
        then: Option<(Status, u32)>,
        /// 「稲妻が走り」のような、飛ぶものの描写
        text: &'static str,
    },
    /// 最初の敵に状態を付ける
    Afflict(Status, u32),
    /// 別の種類の敵に変える
    Polymorph,
    /// 敵の状態と特殊な力を消す
    Cancel,
    /// 最初の敵をどこかへ飛ばす
    TeleportOther,
    /// 自分がどこかへ飛ぶ
    TeleportSelf,
}

/// 杖の仕様。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Zap {
    /// 見つけたときの充填数の幅 (最小, 最大)
    pub charges: (i32, i32),
    /// 向きの指定が要るか（自分に効く杖は要らない）
    pub aimed: bool,
    pub fx: ZapFx,
}

/// 薬や巻物の効果の部品。`ItemDef::effects` に並べて書く。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// HPを回復する。回復しきって余ったら最大HPが `max_up` 増える
    Heal {
        amount: i32,
        max_up: i32,
    },
    /// ダメージを受ける（毒の薬）
    Damage(i32),
    /// 腕力を下げる（最低3）
    LoseStrength(i32),
    /// 腕力の最大値と今の値を上げる
    GainStrength(i32),
    RestoreStrength,
    /// 次のレベルまでの経験値を得る
    LevelUp,
    /// 状態を治す
    Cure(Status),
    /// 自分に状態を付ける（ターン数）
    SelfStatus(Status, u32),
    /// 見えている敵（`radius` マス以内）に状態を付ける
    MonstersStatus {
        status: Status,
        turns: u32,
        radius: i32,
    },
    DetectMonsters,
    DetectItems,
    MagicMap,
    Teleport,
    Identify,
    EnchantWeapon,
    EnchantArmor,
    RemoveCurse,
    ProtectArmor,
    CreateMonster,
    /// 階じゅうの敵を怒らせる
    Aggravate,
}

pub struct ItemDef {
    pub kind: ItemKind,
    pub name: &'static str,
    pub class: Class,
    /// 床への出やすさ（相対的な重み）
    pub weight: u32,
    /// この深さから床に現れる
    pub min_depth: u32,
    /// 薬・巻物の効果（上から順に起きる）
    pub effects: &'static [Effect],
    /// 使うと損をする側か
    pub bad: bool,
    /// 杖の仕様（杖だけ）
    pub zap: Option<Zap>,
    /// 指輪の仕様（指輪だけ）
    pub ring: Option<RingDef>,
    /// 光源の仕様（光源だけ）
    pub light: Option<LightDef>,
}

/// 見えているすべての敵、という意味の半径。
const ALL_IN_SIGHT: i32 = 99;

const fn def(
    kind: ItemKind,
    name: &'static str,
    class: Class,
    weight: u32,
    min_depth: u32,
    bad: bool,
    effects: &'static [Effect],
) -> ItemDef {
    ItemDef {
        kind,
        name,
        class,
        weight,
        min_depth,
        effects,
        bad,
        zap: None,
        ring: None,
        light: None,
    }
}

#[allow(clippy::too_many_arguments)]
const fn wand(
    kind: ItemKind,
    name: &'static str,
    weight: u32,
    min_depth: u32,
    bad: bool,
    charges: (i32, i32),
    aimed: bool,
    fx: ZapFx,
) -> ItemDef {
    ItemDef {
        kind,
        name,
        class: Class::Wand,
        weight,
        min_depth,
        effects: &[],
        bad,
        zap: Some(Zap { charges, aimed, fx }),
        ring: None,
        light: None,
    }
}

const fn ring(
    kind: ItemKind,
    name: &'static str,
    weight: u32,
    effect: RingEffect,
    cursable: bool,
    always_cursed: bool,
) -> ItemDef {
    ItemDef {
        kind,
        name,
        class: Class::Ring,
        weight,
        min_depth: 2,
        effects: &[],
        bad: always_cursed,
        zap: None,
        ring: Some(RingDef {
            effect,
            cursable,
            always_cursed,
        }),
        light: None,
    }
}

const fn light(
    kind: ItemKind,
    name: &'static str,
    weight: u32,
    min_depth: u32,
    max_fuel: i32,
    refillable: bool,
) -> ItemDef {
    ItemDef {
        kind,
        name,
        class: Class::Light,
        weight,
        min_depth,
        effects: &[],
        bad: false,
        zap: None,
        ring: None,
        light: Some(LightDef {
            max_fuel,
            refillable,
        }),
    }
}

const fn bolt(lo: i32, hi: i32, text: &'static str) -> ZapFx {
    ZapFx::Damage {
        lo,
        hi,
        beam: false,
        drain: false,
        then: None,
        text,
    }
}

use Class::{Armor, Food, Mushroom, Potion, Scroll, Weapon};
use Effect as E;

/// 全アイテムの表。`ItemKind` の並びと同じ順で書く（テストが確かめる）。
pub static ITEMS: [ItemDef; ItemKind::COUNT] = [
    // 薬（良い）
    def(
        ItemKind::Healing,
        "回復の薬",
        Potion,
        5,
        1,
        false,
        &[
            E::Heal {
                amount: 10,
                max_up: 0,
            },
            E::Cure(Status::Poisoned),
        ],
    ),
    def(
        ItemKind::ExtraHealing,
        "大回復の薬",
        Potion,
        2,
        3,
        false,
        &[
            E::Heal {
                amount: 30,
                max_up: 2,
            },
            E::Cure(Status::Poisoned),
        ],
    ),
    def(
        ItemKind::Strength,
        "力の薬",
        Potion,
        2,
        2,
        false,
        &[E::GainStrength(1)],
    ),
    def(
        ItemKind::Experience,
        "レベルアップの薬",
        Potion,
        1,
        2,
        false,
        &[E::LevelUp],
    ),
    def(
        ItemKind::RestoreStrength,
        "力回復の薬",
        Potion,
        2,
        1,
        false,
        &[E::RestoreStrength],
    ),
    def(
        ItemKind::Haste,
        "加速の薬",
        Potion,
        2,
        2,
        false,
        &[E::SelfStatus(Status::Hasted, 25)],
    ),
    def(
        ItemKind::DetectMonsters,
        "モンスター探知の薬",
        Potion,
        3,
        1,
        false,
        &[E::DetectMonsters],
    ),
    def(
        ItemKind::DetectItems,
        "アイテム探知の薬",
        Potion,
        3,
        1,
        false,
        &[E::DetectItems],
    ),
    def(
        ItemKind::SeeInvisible,
        "透明視認の薬",
        Potion,
        2,
        1,
        false,
        &[E::SelfStatus(Status::SeeInvisible, 150)],
    ),
    def(
        ItemKind::Levitation,
        "浮遊の薬",
        Potion,
        2,
        1,
        false,
        &[E::SelfStatus(Status::Levitating, 30)],
    ),
    def(
        ItemKind::Antidote,
        "解毒の薬",
        Potion,
        3,
        1,
        false,
        &[E::Cure(Status::Poisoned)],
    ),
    // 薬（悪い）
    def(
        ItemKind::Confusion,
        "混乱の薬",
        Potion,
        2,
        1,
        true,
        &[E::SelfStatus(Status::Confused, 12)],
    ),
    def(
        ItemKind::Hallucination,
        "幻覚の薬",
        Potion,
        2,
        1,
        true,
        &[E::SelfStatus(Status::Hallucinating, 60)],
    ),
    def(
        ItemKind::Poison,
        "毒の薬",
        Potion,
        2,
        1,
        true,
        &[E::Damage(5), E::LoseStrength(2)],
    ),
    def(
        ItemKind::Blindness,
        "盲目の薬",
        Potion,
        2,
        1,
        true,
        &[E::SelfStatus(Status::Blind, 25)],
    ),
    def(
        ItemKind::Sleep,
        "眠りの薬",
        Potion,
        2,
        1,
        true,
        &[E::SelfStatus(Status::Asleep, 5)],
    ),
    // 巻物
    def(
        ItemKind::Identify,
        "識別の巻物",
        Scroll,
        5,
        1,
        false,
        &[E::Identify],
    ),
    def(
        ItemKind::MagicMap,
        "地図の巻物",
        Scroll,
        3,
        1,
        false,
        &[E::MagicMap],
    ),
    def(
        ItemKind::Teleport,
        "転移の巻物",
        Scroll,
        2,
        1,
        false,
        &[E::Teleport],
    ),
    def(
        ItemKind::EnchantWeapon,
        "武器強化の巻物",
        Scroll,
        3,
        1,
        false,
        &[E::EnchantWeapon],
    ),
    def(
        ItemKind::EnchantArmor,
        "防具強化の巻物",
        Scroll,
        3,
        1,
        false,
        &[E::EnchantArmor],
    ),
    def(
        ItemKind::RemoveCurse,
        "呪い解除の巻物",
        Scroll,
        2,
        1,
        false,
        &[E::RemoveCurse],
    ),
    def(
        ItemKind::ProtectArmor,
        "防具保護の巻物",
        Scroll,
        1,
        2,
        false,
        &[E::ProtectArmor],
    ),
    def(
        ItemKind::ConfuseMonster,
        "モンスター混乱の巻物",
        Scroll,
        2,
        1,
        false,
        &[E::MonstersStatus {
            status: Status::Confused,
            turns: 15,
            radius: ALL_IN_SIGHT,
        }],
    ),
    def(
        ItemKind::HoldMonster,
        "モンスター停止の巻物",
        Scroll,
        2,
        2,
        false,
        &[E::MonstersStatus {
            status: Status::Paralyzed,
            turns: 8,
            radius: 2,
        }],
    ),
    def(
        ItemKind::ScareMonster,
        "怯えの巻物",
        Scroll,
        2,
        1,
        false,
        &[E::MonstersStatus {
            status: Status::Scared,
            turns: 20,
            radius: ALL_IN_SIGHT,
        }],
    ),
    def(
        ItemKind::CreateMonster,
        "モンスター生成の巻物",
        Scroll,
        1,
        2,
        true,
        &[E::CreateMonster],
    ),
    def(
        ItemKind::Aggravate,
        "怒りの巻物",
        Scroll,
        1,
        2,
        true,
        &[E::Aggravate],
    ),
    def(
        ItemKind::Slumber,
        "睡眠の巻物",
        Scroll,
        1,
        1,
        true,
        &[E::SelfStatus(Status::Asleep, 6)],
    ),
    // 杖
    wand(
        ItemKind::WandLight,
        "光の杖",
        2,
        1,
        false,
        (6, 10),
        true,
        ZapFx::Light,
    ),
    wand(
        ItemKind::WandInvisibility,
        "透明化の杖",
        1,
        2,
        true,
        (3, 6),
        true,
        ZapFx::Afflict(Status::Invisible, 400),
    ),
    wand(
        ItemKind::WandLightning,
        "雷の杖",
        1,
        3,
        false,
        (3, 5),
        true,
        ZapFx::Damage {
            lo: 6,
            hi: 10,
            beam: true,
            drain: false,
            then: None,
            text: "稲妻が走り",
        },
    ),
    wand(
        ItemKind::WandFire,
        "火の杖",
        1,
        3,
        false,
        (3, 5),
        true,
        bolt(7, 12, "炎が噴き出し"),
    ),
    wand(
        ItemKind::WandCold,
        "冷気の杖",
        1,
        2,
        false,
        (3, 5),
        true,
        ZapFx::Damage {
            lo: 4,
            hi: 7,
            beam: false,
            drain: false,
            then: Some((Status::Slowed, 10)),
            text: "凍てつく冷気が走り",
        },
    ),
    wand(
        ItemKind::WandPolymorph,
        "変身の杖",
        1,
        2,
        true,
        (3, 5),
        true,
        ZapFx::Polymorph,
    ),
    wand(
        ItemKind::WandMissile,
        "魔法の矢の杖",
        3,
        1,
        false,
        (6, 10),
        true,
        bolt(2, 6, "魔法の矢が飛び"),
    ),
    wand(
        ItemKind::WandHaste,
        "敵加速の杖",
        1,
        2,
        true,
        (3, 6),
        true,
        ZapFx::Afflict(Status::Hasted, 30),
    ),
    wand(
        ItemKind::WandSlow,
        "敵減速の杖",
        2,
        1,
        false,
        (4, 7),
        true,
        ZapFx::Afflict(Status::Slowed, 30),
    ),
    wand(
        ItemKind::WandDrain,
        "生命吸収の杖",
        1,
        2,
        false,
        (4, 6),
        true,
        ZapFx::Damage {
            lo: 3,
            hi: 6,
            beam: false,
            drain: true,
            then: None,
            text: "黒い光が走り",
        },
    ),
    wand(
        ItemKind::WandCancel,
        "消去の杖",
        1,
        3,
        false,
        (3, 5),
        true,
        ZapFx::Cancel,
    ),
    wand(
        ItemKind::WandTeleportOther,
        "敵テレポートの杖",
        2,
        1,
        false,
        (4, 7),
        true,
        ZapFx::TeleportOther,
    ),
    wand(
        ItemKind::WandTeleportSelf,
        "自分テレポートの杖",
        1,
        1,
        false,
        (3, 5),
        false,
        ZapFx::TeleportSelf,
    ),
    // 指輪
    ring(
        ItemKind::RingProtection,
        "防御の指輪",
        1,
        RingEffect::Protection,
        true,
        false,
    ),
    ring(
        ItemKind::RingStrength,
        "腕力の指輪",
        1,
        RingEffect::Strength,
        true,
        false,
    ),
    ring(
        ItemKind::RingDexterity,
        "器用さの指輪",
        1,
        RingEffect::Dexterity,
        true,
        false,
    ),
    ring(
        ItemKind::RingDamage,
        "ダメージ増加の指輪",
        1,
        RingEffect::Damage,
        true,
        false,
    ),
    ring(
        ItemKind::RingRegeneration,
        "再生の指輪",
        1,
        RingEffect::Regeneration,
        false,
        false,
    ),
    ring(
        ItemKind::RingSlowDigestion,
        "消化遅延の指輪",
        1,
        RingEffect::SlowDigestion,
        false,
        false,
    ),
    ring(
        ItemKind::RingStealth,
        "隠密の指輪",
        1,
        RingEffect::Stealth,
        false,
        false,
    ),
    ring(
        ItemKind::RingSearching,
        "探索の指輪",
        1,
        RingEffect::Searching,
        false,
        false,
    ),
    ring(
        ItemKind::RingSeeInvisible,
        "透明視認の指輪",
        1,
        RingEffect::SeeInvisible,
        false,
        false,
    ),
    ring(
        ItemKind::RingTrinket,
        "装飾の指輪",
        1,
        RingEffect::Trinket,
        false,
        false,
    ),
    ring(
        ItemKind::RingAggravate,
        "怒らせる指輪",
        1,
        RingEffect::Aggravate,
        false,
        true,
    ),
    ring(
        ItemKind::RingTeleportitis,
        "テレポート癖の指輪",
        1,
        RingEffect::Teleportitis,
        false,
        true,
    ),
    // 光源と燃料
    light(ItemKind::Torch, "松明", 3, 1, 1500, false),
    light(ItemKind::Lantern, "ランタン", 1, 3, 4000, true),
    def(ItemKind::OilFlask, "油つぼ", Class::Fuel, 3, 1, false, &[]),
    // 装備品
    def(ItemKind::Dagger, "短剣", Weapon, 4, 1, false, &[]),
    def(ItemKind::Sword, "剣", Weapon, 3, 2, false, &[]),
    def(ItemKind::Axe, "斧", Weapon, 2, 3, false, &[]),
    def(ItemKind::Leather, "革の鎧", Armor, 4, 1, false, &[]),
    def(ItemKind::Chain, "鎖かたびら", Armor, 3, 2, false, &[]),
    def(ItemKind::Plate, "板金鎧", Armor, 2, 4, false, &[]),
    // 食べ物（効果は game 側。満腹度は nutrition()）
    def(ItemKind::Bread, "パン", Food, 5, 1, false, &[]),
    def(ItemKind::Jerky, "干し肉", Food, 2, 2, false, &[]),
    def(
        ItemKind::EdibleShroom,
        "食用キノコ",
        Mushroom,
        2,
        1,
        false,
        &[],
    ),
    def(
        ItemKind::PoisonShroom,
        "毒キノコ",
        Mushroom,
        1,
        1,
        true,
        &[],
    ),
    def(
        ItemKind::VigorShroom,
        "元気キノコ",
        Mushroom,
        1,
        2,
        false,
        &[],
    ),
];

impl ItemKind {
    pub const COUNT: usize = 68;
    pub const ALL: [ItemKind; ItemKind::COUNT] = [
        ItemKind::Healing,
        ItemKind::ExtraHealing,
        ItemKind::Strength,
        ItemKind::Experience,
        ItemKind::RestoreStrength,
        ItemKind::Haste,
        ItemKind::DetectMonsters,
        ItemKind::DetectItems,
        ItemKind::SeeInvisible,
        ItemKind::Levitation,
        ItemKind::Antidote,
        ItemKind::Confusion,
        ItemKind::Hallucination,
        ItemKind::Poison,
        ItemKind::Blindness,
        ItemKind::Sleep,
        ItemKind::Identify,
        ItemKind::MagicMap,
        ItemKind::Teleport,
        ItemKind::EnchantWeapon,
        ItemKind::EnchantArmor,
        ItemKind::RemoveCurse,
        ItemKind::ProtectArmor,
        ItemKind::ConfuseMonster,
        ItemKind::HoldMonster,
        ItemKind::ScareMonster,
        ItemKind::CreateMonster,
        ItemKind::Aggravate,
        ItemKind::Slumber,
        ItemKind::WandLight,
        ItemKind::WandInvisibility,
        ItemKind::WandLightning,
        ItemKind::WandFire,
        ItemKind::WandCold,
        ItemKind::WandPolymorph,
        ItemKind::WandMissile,
        ItemKind::WandHaste,
        ItemKind::WandSlow,
        ItemKind::WandDrain,
        ItemKind::WandCancel,
        ItemKind::WandTeleportOther,
        ItemKind::WandTeleportSelf,
        ItemKind::RingProtection,
        ItemKind::RingStrength,
        ItemKind::RingDexterity,
        ItemKind::RingDamage,
        ItemKind::RingRegeneration,
        ItemKind::RingSlowDigestion,
        ItemKind::RingStealth,
        ItemKind::RingSearching,
        ItemKind::RingSeeInvisible,
        ItemKind::RingTrinket,
        ItemKind::RingAggravate,
        ItemKind::RingTeleportitis,
        ItemKind::Torch,
        ItemKind::Lantern,
        ItemKind::OilFlask,
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

    pub fn def(self) -> &'static ItemDef {
        &ITEMS[self as usize]
    }

    pub fn class(self) -> Class {
        self.def().class
    }

    pub fn is_potion(self) -> bool {
        self.class() == Class::Potion
    }

    pub fn true_name(self) -> &'static str {
        self.def().name
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

    pub fn is_wand(self) -> bool {
        self.class() == Class::Wand
    }

    pub fn is_ring(self) -> bool {
        self.class() == Class::Ring
    }

    pub fn is_light(self) -> bool {
        self.class() == Class::Light
    }

    /// 個体ごとに数値（充填数・指輪の強さ・燃料）を持つ道具か。重ならず、1個ずつ別の文字に入る。
    pub fn is_tool(self) -> bool {
        matches!(self.class(), Class::Wand | Class::Ring | Class::Light)
    }

    pub fn ring_def(self) -> Option<&'static RingDef> {
        self.def().ring.as_ref()
    }

    pub fn light_def(self) -> Option<&'static LightDef> {
        self.def().light.as_ref()
    }

    /// 杖の仕様。
    pub fn zap(self) -> Option<&'static Zap> {
        self.def().zap.as_ref()
    }

    /// 最初から名前が分かる種類（装備品・食べ物・光源・燃料）。
    pub fn starts_known(self) -> bool {
        self.is_equipment() || matches!(self.class(), Class::Food | Class::Light | Class::Fuel)
    }

    pub fn is_scroll(self) -> bool {
        self.class() == Class::Scroll
    }

    /// 名前が最初から分かる食べ物（パン・干し肉）。
    pub fn is_food(self) -> bool {
        self.class() == Class::Food
    }

    /// 見た目だけでは正体が分からないキノコ。
    pub fn is_mushroom(self) -> bool {
        self.class() == Class::Mushroom
    }

    /// 使うと損をする側（悪い薬・巻物・毒キノコ）か。
    pub fn is_bad(self) -> bool {
        self.def().bad
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
        self.class() == Class::Weapon
    }

    pub fn is_armor(self) -> bool {
        self.class() == Class::Armor
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
        self.def().min_depth
    }

    /// マップ上の記号。薬は `!`、巻物は `?`、武器は `)`、防具は `[`、食べ物とキノコは `%`。
    pub fn glyph(self) -> char {
        match self.class() {
            Class::Potion => '!',
            Class::Scroll => '?',
            Class::Weapon => ')',
            Class::Armor => '[',
            Class::Food | Class::Mushroom => '%',
            Class::Wand => '/',
            Class::Ring => '=',
            Class::Light | Class::Fuel => '~',
        }
    }

    /// 床への出やすさ（相対的な重み）。
    pub fn weight(self) -> u32 {
        self.def().weight
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
    pub const ALL: [Quality; 4] = [
        Quality::Common,
        Quality::Uncommon,
        Quality::Rare,
        Quality::Ancient,
    ];

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
    /// 呪い: 攻撃が+3されるが、受けるダメージが+1され、装備すると外せなくなる
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
    pub const ARMOR: [(Suffix, i32); 3] = [
        (Suffix::Thorns, 2),
        (Suffix::Warding, 2),
        (Suffix::Famine, 1),
    ];

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
            Suffix::Cataclysm => "攻撃+3。受けるダメージ+1。呪われていて、装備すると外せない",
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

    /// 敵から受けるダメージ1回ごとへの加算（呪いの欠点）。
    pub fn damage_taken_bonus(self) -> i32 {
        match self {
            Suffix::Cataclysm => 1,
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
        let pool: &[(Suffix, i32)] = if weapon {
            &Suffix::WEAPON
        } else {
            &Suffix::ARMOR
        };
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
    /// 強化の巻物で足された攻撃・防御（錆びると減る）
    pub enchant: i32,
    /// 防具保護の巻物で、錆びなくなった
    pub protected: bool,
    /// 呪い解除の巻物で、はずせるようになった（呪いの欠点は残る）
    pub freed: bool,
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
            enchant: 0,
            protected: false,
            freed: false,
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
        let suffix = if quality.suffix_percent() > 0 && rng.range(0, 100) < quality.suffix_percent()
        {
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
            enchant: 0,
            protected: false,
            freed: false,
        }
    }

    /// 呪われているか。
    pub fn is_cursed(&self) -> bool {
        self.suffix.is_some_and(Suffix::is_cursed)
    }

    /// 呪いで、はずせなくなっているか（呪い解除の巻物を読むまで）。
    pub fn is_sticky(&self) -> bool {
        self.is_cursed() && !self.freed
    }

    /// 品質補正・接尾辞・強化を合わせた攻撃・防御の加算値。
    fn total_bonus(&self) -> i32 {
        let base = self.bonus + self.enchant;
        match self.suffix {
            Some(s) if self.kind.is_weapon() => base + s.attack_bonus(),
            Some(s) => base + s.defense_bonus(),
            None => base,
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
        let fx = self.suffix.map_or("特殊効果はない".to_string(), |s| {
            s.describe().to_string()
        });
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
        let range = if lo == hi {
            format!("+{lo}")
        } else {
            format!("+({lo}〜{hi})")
        };
        // 強化は自分で見ているので分かる
        let ench = if self.enchant != 0 {
            format!(" 強化{:+}", self.enchant)
        } else {
            String::new()
        };
        match self.kind.weapon_dmg() {
            Some((a, b)) => format!("攻撃 {a}〜{b} {range}?{ench}"),
            None => format!("防御 {} {range}?{ench}", self.kind.armor()),
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

/// 数値を持つ道具の1個体（杖の充填数・指輪の強さ・光源の燃料）。重ならない。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tool {
    pub kind: ItemKind,
    /// 杖: 残りの使用回数（充填数）/ 指輪: 強さ（呪われていれば負）/ 光源: 残りの燃料
    pub val: i32,
    /// 指輪: 呪われている（身につけると外せない）
    pub cursed: bool,
    /// 呪い解除の巻物で、外せるようになった
    pub freed: bool,
    /// 指輪: 強さまで分かっているか
    pub identified: bool,
    /// 指輪を身につけて過ごしたターン数（一定に達すると識別される）
    pub worn: u32,
}

impl Tool {
    /// 値を直接指定して作る。杖や光源はこれで足りる。
    pub fn charged(kind: ItemKind, val: i32) -> Tool {
        Tool {
            kind,
            val,
            cursed: false,
            freed: false,
            identified: !kind.is_ring(),
            worn: 0,
        }
    }

    /// 見つかったときの個体を抽選する。杖は充填数、指輪は強さと呪い、光源は燃料。
    pub fn roll(rng: &mut Rng, kind: ItemKind) -> Tool {
        if let Some(z) = kind.zap() {
            return Tool::charged(kind, rng.range(z.charges.0, z.charges.1 + 1));
        }
        if let Some(l) = kind.light_def() {
            // 使いかけのこともある
            let fuel = rng.range(l.max_fuel / 2, l.max_fuel + 1);
            return Tool::charged(kind, fuel);
        }
        let Some(r) = kind.ring_def() else {
            return Tool::charged(kind, 0);
        };
        let mut t = Tool::charged(kind, 0);
        t.identified = false;
        match r.effect {
            RingEffect::Trinket => {}
            _ if r.always_cursed => {
                t.val = 1;
                t.cursed = true;
            }
            _ => {
                t.val = rng.range(1, 4);
                if r.cursable && rng.range(0, 4) == 0 {
                    t.val = -t.val;
                    t.cursed = true;
                }
            }
        }
        t
    }

    /// 乱数を使わずに作る（テストや初期装備用）。杖は充填数の最小、光源は満タン、指輪は +1。
    pub fn plain(kind: ItemKind) -> Tool {
        if let Some(z) = kind.zap() {
            Tool::charged(kind, z.charges.0)
        } else if let Some(l) = kind.light_def() {
            Tool::charged(kind, l.max_fuel)
        } else if kind.is_ring() {
            let mut t = Tool::charged(
                kind,
                if kind
                    .ring_def()
                    .is_some_and(|r| r.effect == RingEffect::Trinket)
                {
                    0
                } else {
                    1
                },
            );
            t.cursed = kind.ring_def().is_some_and(|r| r.always_cursed);
            t
        } else {
            Tool::charged(kind, 0)
        }
    }

    /// 呪いで外せないか。
    pub fn is_sticky(&self) -> bool {
        self.cursed && !self.freed
    }
}

impl Item {
    /// 床に置くために、種類から品物を作る。装備や道具なら個体を抽選する。
    pub fn roll(rng: &mut Rng, kind: ItemKind, depth: u32) -> Item {
        if kind.is_equipment() {
            Item::Gear(Gear::roll(rng, kind, depth))
        } else if kind.is_tool() {
            Item::Tool(Tool::roll(rng, kind))
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
    Tool(Tool),
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        match self {
            Item::Plain(k) => *k,
            Item::Gear(g) => g.kind,
            Item::Tool(t) => t.kind,
        }
    }
}

impl From<Tool> for Item {
    fn from(t: Tool) -> Item {
        Item::Tool(t)
    }
}

impl From<ItemKind> for Item {
    fn from(kind: ItemKind) -> Item {
        if kind.is_equipment() {
            Item::Gear(Gear::plain(kind))
        } else if kind.is_tool() {
            Item::Tool(Tool::plain(kind))
        } else {
            Item::Plain(kind)
        }
    }
}

/// 薬の見た目の候補。ここから種類の数だけ選んで割り当てる。
pub const POTION_LOOKS: [&str; 20] = [
    "赤い薬",
    "青い薬",
    "緑の薬",
    "黄色い薬",
    "白い薬",
    "黒い薬",
    "紫の薬",
    "橙色の薬",
    "茶色い薬",
    "灰色の薬",
    "桃色の薬",
    "水色の薬",
    "金色の薬",
    "銀色の薬",
    "透明な薬",
    "虹色の薬",
    "濁った薬",
    "泡立つ薬",
    "光る薬",
    "粘つく薬",
];

/// キノコの見た目の候補。
pub const MUSHROOM_LOOKS: [&str; 4] = ["赤いキノコ", "白いキノコ", "茶色いキノコ", "斑点のキノコ"];

/// 指輪の見た目の候補。
pub const RING_LOOKS: [&str; 14] = [
    "ルビーの指輪",
    "サファイアの指輪",
    "エメラルドの指輪",
    "ダイヤモンドの指輪",
    "オパールの指輪",
    "トパーズの指輪",
    "アメジストの指輪",
    "ガーネットの指輪",
    "真珠の指輪",
    "翡翠の指輪",
    "瑪瑙の指輪",
    "黒曜石の指輪",
    "珊瑚の指輪",
    "琥珀の指輪",
];

/// 杖の見た目の候補。
pub const WAND_LOOKS: [&str; 16] = [
    "樫の杖",
    "鉄の杖",
    "銅の杖",
    "銀の杖",
    "金の杖",
    "象牙の杖",
    "骨の杖",
    "黒檀の杖",
    "水晶の杖",
    "ガラスの杖",
    "真鍮の杖",
    "錫の杖",
    "鉛の杖",
    "亜鉛の杖",
    "琥珀の杖",
    "翡翠の杖",
];

/// 巻物の見た目の候補。
pub const SCROLL_LOOKS: [&str; 20] = [
    "「ルク」の巻物",
    "「ネム」の巻物",
    "「ザラ」の巻物",
    "「ポロ」の巻物",
    "「ミト」の巻物",
    "「ヘル」の巻物",
    "「ドゥナ」の巻物",
    "「キス」の巻物",
    "「ワフ」の巻物",
    "「ノア」の巻物",
    "「ラグ」の巻物",
    "「ベス」の巻物",
    "「ソウ」の巻物",
    "「ユル」の巻物",
    "「タン」の巻物",
    "「オズ」の巻物",
    "「フィン」の巻物",
    "「グラ」の巻物",
    "「モア」の巻物",
    "「リエ」の巻物",
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
        assert!(ItemKind::ALL.iter().filter(|k| k.is_wand()).count() <= WAND_LOOKS.len());
        assert!(ItemKind::ALL.iter().filter(|k| k.is_ring()).count() <= RING_LOOKS.len());
    }

    #[test]
    fn gear_stats_are_consistent_with_its_category() {
        for (i, d) in ITEMS.iter().enumerate() {
            assert_eq!(d.kind.index(), i, "{:?} は表の{i}番目にない", d.kind);
            assert_eq!(ItemKind::ALL[i], d.kind);
            // 薬と巻物には効果があり、それ以外には（game 側で扱う食べ物を除いて）ない
            assert_eq!(
                !d.effects.is_empty(),
                matches!(d.class, Class::Potion | Class::Scroll),
                "{:?}",
                d.kind
            );
            assert_eq!(d.zap.is_some(), d.class == Class::Wand, "{:?}", d.kind);
            assert_eq!(d.ring.is_some(), d.class == Class::Ring, "{:?}", d.kind);
            assert_eq!(d.light.is_some(), d.class == Class::Light, "{:?}", d.kind);
            if let Some(z) = d.zap {
                assert!(
                    z.charges.0 >= 1 && z.charges.0 <= z.charges.1,
                    "{:?}",
                    d.kind
                );
            }
        }
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
