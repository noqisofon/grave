//! 状態異常。プレイヤーにも敵にも付く。
//!
//! 種類ごとの性質は `DEFS` の表（データ）で決まる。アイテムの効果は
//! 「どの状態を何ターン付与するか」という表で書ける（`Status` と ターン数を渡すだけ）。

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Status {
    /// 毎ターン1ダメージ。重ねがけで長引く
    Poisoned,
    /// 移動・攻撃の向きがずれる
    Confused,
    /// 敵の名前と記号がでたらめに見える
    Hallucinating,
    /// 何も見えない
    Blind,
    /// 眠っていて行動できない
    Asleep,
    /// 体が固まって行動できない（停止）
    Paralyzed,
    /// 行動が倍になる
    Hasted,
    /// 行動が半分になる
    Slowed,
    /// 落とし穴や毒矢の罠を避ける
    Levitating,
    /// 敵から見つかりにくい（敵なら姿が見えない）
    Invisible,
    /// 透明なものが見える
    SeeInvisible,
    /// 敵が逃げ出す
    Scared,
    /// 敵が怒って、どこにいても追ってくる
    Enraged,
}

pub struct StatusDef {
    pub status: Status,
    /// 表示名（短く）
    pub name: &'static str,
    /// 記録（JSONL）で使う英字の名前
    pub key: &'static str,
    /// 付与されたとき・解けたときのプレイヤー向けメッセージ
    pub start: &'static str,
    pub end: &'static str,
    /// 影響の説明（観測の「状態」欄に出す）
    pub effect: &'static str,
    /// プレイヤーにとって有利な状態か
    pub good: bool,
    pub on_player: bool,
    pub on_monster: bool,
    /// 重ねがけで残りターンを足す（足せる上限）。None なら長い方を取る
    pub stack_cap: Option<u32>,
}

pub const DEFS: [StatusDef; Status::COUNT] = [
    StatusDef {
        status: Status::Poisoned,
        name: "毒",
        key: "poisoned",
        start: "毒を受けた！",
        end: "毒が抜けた。",
        effect: "毎ターン1ダメージ。自然回復しない",
        good: false,
        on_player: true,
        on_monster: true,
        stack_cap: Some(15),
    },
    StatusDef {
        status: Status::Confused,
        name: "混乱",
        key: "confused",
        start: "頭がくらくらする。",
        end: "混乱が収まった。",
        effect: "移動や攻撃の向きがときどきずれる。自動移動は使えない",
        good: false,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Hallucinating,
        name: "幻覚",
        key: "hallucinating",
        start: "目の前の景色がぐにゃりと歪んだ。",
        end: "幻覚が覚めた。",
        effect: "敵の名前と記号がでたらめに見える（HPや位置は本物）",
        good: false,
        on_player: true,
        on_monster: false,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Blind,
        name: "盲目",
        key: "blind",
        start: "目の前が真っ暗になった！",
        end: "目が見えるようになった。",
        effect: "マップも敵も見えない。巻物は読めない。自動移動は使えない",
        good: false,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Asleep,
        name: "睡眠",
        key: "asleep",
        start: "ぐっすり眠ってしまった…。",
        end: "目が覚めた。",
        effect: "行動できない。覚めるまで時間が過ぎる",
        good: false,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Paralyzed,
        name: "停止",
        key: "paralyzed",
        start: "体が固まって動けない！",
        end: "体が動くようになった。",
        effect: "行動できない。解けるまで時間が過ぎる",
        good: false,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Hasted,
        name: "加速",
        key: "hasted",
        start: "体が軽い。動きが速くなった。",
        end: "加速が切れた。",
        effect: "1ターンに2回行動できる",
        good: true,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Slowed,
        name: "減速",
        key: "slowed",
        start: "体が重い。動きが遅くなった。",
        end: "減速が切れた。",
        effect: "1回の行動に2ターンかかる",
        good: false,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Levitating,
        name: "浮遊",
        key: "levitating",
        start: "体がふわりと浮いた。",
        end: "浮遊が切れて、地面に降りた。",
        effect: "落とし穴と毒矢の罠を避ける(眠りガスは防げない)",
        good: true,
        on_player: true,
        on_monster: false,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Invisible,
        name: "透明",
        key: "invisible",
        start: "体が透き通って見えなくなった。",
        end: "姿が見えるようになった。",
        effect: "敵は2マス以内に近づかないとこちらに気づかない",
        good: true,
        on_player: true,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::SeeInvisible,
        name: "透明視認",
        key: "see_invisible",
        start: "透明なものが見えるようになった。",
        end: "透明なものが見えなくなった。",
        effect: "透明な敵が見える",
        good: true,
        on_player: true,
        on_monster: false,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Scared,
        name: "怯え",
        key: "scared",
        start: "怯えている。",
        end: "怯えが収まった。",
        effect: "逃げ回って攻撃してこない（追い詰められたら別）",
        good: false,
        on_player: false,
        on_monster: true,
        stack_cap: None,
    },
    StatusDef {
        status: Status::Enraged,
        name: "激怒",
        key: "enraged",
        start: "怒り狂っている。",
        end: "怒りが収まった。",
        effect: "見えない場所からでも追ってくる",
        good: false,
        on_player: false,
        on_monster: true,
        stack_cap: None,
    },
];

impl Status {
    pub const COUNT: usize = 13;
    pub const ALL: [Status; Status::COUNT] = [
        Status::Poisoned,
        Status::Confused,
        Status::Hallucinating,
        Status::Blind,
        Status::Asleep,
        Status::Paralyzed,
        Status::Hasted,
        Status::Slowed,
        Status::Levitating,
        Status::Invisible,
        Status::SeeInvisible,
        Status::Scared,
        Status::Enraged,
    ];

    pub fn def(self) -> &'static StatusDef {
        &DEFS[self as usize]
    }
    pub fn name(self) -> &'static str {
        self.def().name
    }
    pub fn key(self) -> &'static str {
        self.def().key
    }
    pub fn from_key(key: &str) -> Option<Status> {
        Status::ALL.iter().copied().find(|s| s.key() == key)
    }
}

/// 状態ごとの残りターン数。0 は「かかっていない」。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StatusSet([u32; Status::COUNT]);

impl StatusSet {
    pub fn get(&self, s: Status) -> u32 {
        self.0[s as usize]
    }
    pub fn has(&self, s: Status) -> bool {
        self.get(s) > 0
    }

    /// 状態を付ける。重ねがけは、足す状態は足し（上限あり）、それ以外は長い方を取る。
    /// 付けたあとの残りターン数を返す。
    pub fn apply(&mut self, s: Status, turns: u32) -> u32 {
        let slot = &mut self.0[s as usize];
        *slot = match s.def().stack_cap {
            Some(cap) => (*slot + turns).min(cap),
            None => (*slot).max(turns),
        };
        *slot
    }

    pub fn clear(&mut self, s: Status) -> bool {
        std::mem::take(&mut self.0[s as usize]) > 0
    }

    /// かかっている状態を残りターンつきで（定義の順に）。
    pub fn active(&self) -> Vec<(Status, u32)> {
        Status::ALL
            .iter()
            .copied()
            .filter_map(|s| self.has(s).then(|| (s, self.get(s))))
            .collect()
    }

    /// 1ターン進める。今ちょうど切れた状態を返す。
    pub fn tick(&mut self) -> Vec<Status> {
        let mut expired = Vec::new();
        for s in Status::ALL {
            let slot = &mut self.0[s as usize];
            if *slot > 0 {
                *slot -= 1;
                if *slot == 0 {
                    expired.push(s);
                }
            }
        }
        expired
    }

    /// `混乱8 幻覚3` のような短い表示。
    pub fn short_text(&self) -> String {
        self.active()
            .iter()
            .map(|(s, n)| format!("{}{n}", s.name()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 1コマンドの間に起きた状態の変化。記録（JSONL）に残す。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusEvent {
    /// `player` か、敵の名前
    pub target: String,
    pub status: Status,
    pub change: Change,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// 付与（付けたあとの残りターン数）
    Apply(u32),
    /// 時間切れ、または治療で解除
    End,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defs_are_in_enum_order_and_keys_are_unique() {
        for (i, d) in DEFS.iter().enumerate() {
            assert_eq!(d.status as usize, i, "{:?}", d.status);
            assert_eq!(Status::from_key(d.key), Some(d.status));
            assert!(d.on_player || d.on_monster, "{:?}", d.status);
        }
        assert_eq!(Status::ALL[Status::COUNT - 1] as usize, Status::COUNT - 1);
    }

    #[test]
    fn apply_stacks_poison_up_to_the_cap_and_takes_the_longer_of_others() {
        let mut s = StatusSet::default();
        assert_eq!(s.apply(Status::Poisoned, 10), 10);
        assert_eq!(s.apply(Status::Poisoned, 10), 15);
        assert_eq!(s.apply(Status::Confused, 8), 8);
        assert_eq!(s.apply(Status::Confused, 5), 8);
        assert_eq!(s.apply(Status::Confused, 12), 12);
        assert_eq!(s.short_text(), "毒15 混乱12");
    }

    #[test]
    fn tick_counts_down_and_reports_what_just_expired() {
        let mut s = StatusSet::default();
        s.apply(Status::Blind, 2);
        s.apply(Status::Hasted, 1);
        assert_eq!(s.tick(), vec![Status::Hasted]);
        assert!(s.has(Status::Blind) && !s.has(Status::Hasted));
        assert_eq!(s.tick(), vec![Status::Blind]);
        assert!(s.active().is_empty());
        assert!(s.tick().is_empty());
    }
}
