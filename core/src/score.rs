//! スコアと名前。
//!
//! スコアは「内訳」（階数点・所持金・持ち物）を別々に持ち、合計は内訳から計算する。
//! 記録（JSONL）には内訳の元になる値（到達階数・所持金・持ち物の売値）を残すので、
//! 式を変えても過去の記録を [`Score::from_parts`] で採点し直せる。式を変えたら
//! [`crate::record::RULES_VERSION`] を上げる。

use crate::game::Game;

/// 到達階数1階あたりの点。
pub const POINTS_PER_DEPTH: u32 = 100;

/// 名前の最大の表示幅（全角は2桁）。
pub const NAME_MAX_COLS: usize = 20;

/// 名前を省いたときの名前。
pub const DEFAULT_NAME: &str = "名無し";

/// ゲームの終わり方。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// 死亡
    Died,
    /// アミュレットを持って地上へ生還した
    Cleared,
}

impl Ending {
    pub fn key(self) -> &'static str {
        match self {
            Ending::Died => "died",
            Ending::Cleared => "cleared",
        }
    }

    pub fn from_key(s: &str) -> Option<Ending> {
        match s {
            "died" => Some(Ending::Died),
            "cleared" => Some(Ending::Cleared),
            _ => None,
        }
    }
}

/// スコアの内訳と合計。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Score {
    /// 到達階数 × [`POINTS_PER_DEPTH`]
    pub depth_points: u32,
    /// 所持金
    pub gold: u32,
    /// 持ち物の売値の合計（クリア時だけ数える。死亡時は 0）
    pub items: u32,
}

impl Score {
    /// 記録に残る値から採点する。`item_value` は持ち物の売値の合計で、死亡のときは無視される。
    pub fn from_parts(ending: Ending, max_depth: u32, gold: u32, item_value: u32) -> Score {
        Score {
            depth_points: max_depth * POINTS_PER_DEPTH,
            gold,
            items: match ending {
                Ending::Died => 0,
                Ending::Cleared => item_value,
            },
        }
    }

    pub fn total(&self) -> u32 {
        self.depth_points + self.gold + self.items
    }
}

/// 墓碑（生還のときは帰還の表示）と、スコアの内訳。終わったゲームの結果画面の本文。
/// 続いているゲームには空を返す。TUI も MCP も同じものを出す。
pub fn result_lines(game: &Game) -> Vec<String> {
    const INNER: usize = 32;
    let Some(ending) = game.ending() else {
        return Vec::new();
    };
    let sc = game.score();
    let center = |text: &str| {
        let text = clip(text, INNER);
        let pad = INNER - cols(&text);
        let left = pad / 2;
        format!("|{}{}{}|", " ".repeat(left), text, " ".repeat(pad - left))
    };
    let edge = format!("+{}+", "-".repeat(INNER));
    let mut out = vec![edge.clone()];
    let body: Vec<String> = match ending {
        Ending::Died => vec![
            "ここに眠る".to_string(),
            game.name().to_string(),
            format!("地下{}階", game.max_depth()),
            game.death_cause().unwrap_or("力尽きた").to_string(),
            format!("{}点", sc.total()),
        ],
        Ending::Cleared => vec![
            "生還！".to_string(),
            game.name().to_string(),
            format!("地下{}階まで到達", game.max_depth()),
            "魔除けのアミュレットを持ち帰った".to_string(),
            format!("{}点", sc.total()),
        ],
    };
    out.push(center(""));
    for line in body {
        out.push(center(&line));
    }
    out.push(center(""));
    out.push(edge);
    out.push(format!(
        "階数点   {:>6}  (地下{}階 x {POINTS_PER_DEPTH})",
        sc.depth_points,
        game.max_depth()
    ));
    out.push(format!("所持金   {:>6}", sc.gold));
    out.push(match ending {
        Ending::Cleared => format!("持ち物   {:>6}  (売値の合計)", sc.items),
        Ending::Died => "持ち物   (生還したときだけ加算)".to_string(),
    });
    out.push(format!(
        "合計     {:>6}点  ターン{}",
        sc.total(),
        game.turn()
    ));
    out
}

fn clip(s: &str, max_cols: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        if w + width(c) > max_cols {
            break;
        }
        w += width(c);
        out.push(c);
    }
    out
}

pub(crate) fn cols(s: &str) -> usize {
    s.chars().map(width).sum()
}

/// 文字の表示幅（全角は2桁）。
fn width(c: char) -> usize {
    if c.is_ascii() {
        1
    } else {
        2
    }
}

/// 名前を検査して整える。使える文字は英数字（日本語を含む）と `-` `_` `.`。
/// 空白は使えない（コマンド行で `name=` に続けて書くため）。幅は [`NAME_MAX_COLS`] まで。
pub fn validate_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("名前が空です。".to_string());
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(format!(
            "名前に使えない文字があります: {c:?} (使えるのは英数字・日本語と - _ . だけ)"
        ));
    }
    if cols(name) > NAME_MAX_COLS {
        return Err(format!(
            "名前が長すぎます (半角{NAME_MAX_COLS}文字・全角{}文字まで)。",
            NAME_MAX_COLS / 2
        ));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn death_ignores_item_value_and_clear_counts_it() {
        let d = Score::from_parts(Ending::Died, 4, 230, 999);
        assert_eq!(
            (d.depth_points, d.gold, d.items, d.total()),
            (400, 230, 0, 630)
        );
        let c = Score::from_parts(Ending::Cleared, 30, 100, 250);
        assert_eq!(
            (c.depth_points, c.gold, c.items, c.total()),
            (3000, 100, 250, 3350)
        );
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(
            validate_name(" claude-opus_4.5 "),
            Ok("claude-opus_4.5".into())
        );
        assert_eq!(validate_name("ねっど"), Ok("ねっど".into()));
        assert!(validate_name("").is_err());
        assert!(validate_name("a b").is_err());
        assert!(validate_name("x;quit").is_err());
        assert!(validate_name("あいうえおかきくけこ").is_ok()); // 全角10文字 = 20桁
        assert!(validate_name("あいうえおかきくけこさ").is_err());
        assert!(validate_name(&"a".repeat(21)).is_err());
        assert!(validate_name(&"a".repeat(20)).is_ok());
    }

    #[test]
    fn ending_keys_roundtrip() {
        for e in [Ending::Died, Ending::Cleared] {
            assert_eq!(Ending::from_key(e.key()), Some(e));
        }
        assert_eq!(Ending::from_key("x"), None);
    }
}
