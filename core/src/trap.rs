//! 罠。床に隠れていて、踏むと発動する。浮遊中は無視できる。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrapKind {
    /// 下の階へ落ちる（最深部とアミュレット所持中には置かれない）
    Trapdoor,
    /// 毒矢。ダメージと毒
    Dart,
    /// 眠りのガス
    SleepGas,
}

impl TrapKind {
    pub const ALL: [TrapKind; 3] = [TrapKind::Trapdoor, TrapKind::Dart, TrapKind::SleepGas];

    /// 浮遊中に避けられるか(地面に仕掛けられた落とし穴と毒矢。ガスは浮いていても吸ってしまう)。
    pub fn avoided_by_levitation(self) -> bool {
        matches!(self, TrapKind::Trapdoor | TrapKind::Dart)
    }

    pub fn name(self) -> &'static str {
        match self {
            TrapKind::Trapdoor => "落とし穴",
            TrapKind::Dart => "毒矢の罠",
            TrapKind::SleepGas => "眠りガスの罠",
        }
    }

    /// 出やすさ（相対的な重み）。
    pub fn weight(self) -> u32 {
        match self {
            TrapKind::Trapdoor => 2,
            TrapKind::Dart => 3,
            TrapKind::SleepGas => 2,
        }
    }
}

/// マップ上の罠 1 つ。見つかる（または踏む）まで `revealed` は false。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trap {
    pub pos: (i32, i32),
    pub kind: TrapKind,
    pub revealed: bool,
}

/// 罠のマップ上の記号（見つけたものだけ）。
pub const TRAP_GLYPH: char = '^';
