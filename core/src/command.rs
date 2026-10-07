use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Dir {
    pub const ALL: [Dir; 8] = [
        Dir::N,
        Dir::S,
        Dir::E,
        Dir::W,
        Dir::NE,
        Dir::NW,
        Dir::SE,
        Dir::SW,
    ];

    /// y は下向きが正。
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::N => (0, -1),
            Dir::S => (0, 1),
            Dir::E => (1, 0),
            Dir::W => (-1, 0),
            Dir::NE => (1, -1),
            Dir::NW => (-1, -1),
            Dir::SE => (1, 1),
            Dir::SW => (-1, 1),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::N => "north",
            Dir::S => "south",
            Dir::E => "east",
            Dir::W => "west",
            Dir::NE => "northeast",
            Dir::NW => "northwest",
            Dir::SE => "southeast",
            Dir::SW => "southwest",
        }
    }

    pub fn parse(s: &str) -> Option<Dir> {
        Some(match s {
            "north" | "n" => Dir::N,
            "south" | "s" => Dir::S,
            "east" | "e" => Dir::E,
            "west" | "w" => Dir::W,
            "northeast" | "ne" => Dir::NE,
            "northwest" | "nw" => Dir::NW,
            "southeast" | "se" => Dir::SE,
            "southwest" | "sw" => Dir::SW,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TravelTarget {
    Stairs,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Move(Dir),
    Attack(Dir),
    Descend,
    /// 持ち物の文字と、（識別の巻物のための）任意の対象
    Use(char, Option<char>),
    Inventory,
    /// 持ち物の文字の武器・防具を身につける
    Equip(char),
    /// 装備中の武器・防具をはずす
    Unequip(char),
    Travel(TravelTarget),
    Explore,
    Wait,
    /// その場に指定ターンだけ留まる（敵に襲われたら中断）
    Stay(u32),
    Look,
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Move(d) => write!(f, "move {}", d.name()),
            Command::Attack(d) => write!(f, "attack {}", d.name()),
            Command::Descend => write!(f, "descend"),
            Command::Use(c, None) => write!(f, "use {c}"),
            Command::Use(c, Some(t)) => write!(f, "use {c} {t}"),
            Command::Inventory => write!(f, "inventory"),
            Command::Equip(c) => write!(f, "equip {c}"),
            Command::Unequip(c) => write!(f, "unequip {c}"),
            Command::Travel(TravelTarget::Stairs) => write!(f, "travel >"),
            Command::Explore => write!(f, "explore"),
            Command::Wait => write!(f, "wait"),
            Command::Stay(n) => write!(f, "stay {n}"),
            Command::Look => write!(f, "look"),
        }
    }
}

/// 補完用のコマンド名（正本）。
pub const COMMAND_NAMES: &[&str] = &[
    "move",
    "attack",
    "descend",
    "use",
    "inventory",
    "equip",
    "unequip",
    "travel",
    "explore",
    "wait",
    "stay",
    "look",
];

pub const COMMAND_HELP: &str = "\
move <dir>     1歩移動 (north/south/east/west/northeast/northwest/southeast/southwest, 略: n s e w ne nw se sw)
               敵のいる方向へ move すると攻撃になる
attack <dir>   その方向の敵を攻撃する (敵がいなければ失敗、ターン消費なし)
descend        足元の階段で下の階へ降りる
use <文字> [対象]  持ち物を使う (薬は飲む、巻物は読む、食べ物は食べる。eat でも可)。識別の巻物は対象の文字を指定できる
inventory      持ち物の一覧 (ターン消費なし。装備中のものには (装備中) と付く)
equip <文字>   武器や防具を身につける (武器・防具はそれぞれ1つずつ。付け替えもこれ)
unequip <文字> 装備をはずす
travel >       既知の階段まで自動移動
explore        未探索の場所へ自動移動 (階段を見つける・敵が見える・攻撃を受けると止まる。敵が見えている間は使えない)
wait           1ターン待つ
stay [ターン数]  その場に指定ターンだけ留まる (省略すると1ターン。敵に襲われたり体力が危なくなったら中断。上限1000)
look           階段や見えている敵の位置を調べる (ターン消費なし)
複数のコマンドは ; で区切って連続実行できる (失敗したらそこで止まる)
時間が経つと満腹度が減り、0 になると体力が削られる。食べ物 (パン・干し肉) やキノコで回復する。パンは腐っていることがある。キノコは最初は未識別で、食べると毒になるものもある。毒状態では1ターンごとに1ダメージを受け、自然回復しない (回復の薬で治る)。
武器は攻撃のダメージを、防具は受けるダメージ(最低1)を変える。素手は攻撃 2〜4。
アイテムの上を歩くと自動で拾う。薬と巻物は最初は未識別で、使うと正体が分かる (ゲームごとに見た目と効果の対応が変わる)
凡例: @ 自分  > 階段  ! 薬  ? 巻物  ) 武器  [ 防具  % 食べ物・キノコ  s スライム  b コウモリ  g ゴブリン  O オーガ  S 毒グモ";

/// 持ち物の文字（小文字1つ）。
fn letter_arg(s: &str) -> Option<char> {
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) if ch.is_ascii_lowercase() => Some(ch),
        _ => None,
    }
}

/// `stay` で一度に留まれるターン数の上限。
pub const STAY_LIMIT: u32 = 1000;

/// `:` と `` ` `` は、コマンドの頭に付けてもよい（同じ意味）。
pub fn parse(line: &str) -> Result<Command, String> {
    let line = line.trim_start();
    let line = line
        .strip_prefix(':')
        .or_else(|| line.strip_prefix('`'))
        .unwrap_or(line);
    parse_body(line)
}

fn parse_body(line: &str) -> Result<Command, String> {
    let mut it = line.split_whitespace();
    let head = it.next().ok_or("コマンドが空です")?;
    let args: Vec<&str> = it.collect();
    match head {
        "move" | "m" => {
            let d = args
                .first()
                .ok_or("move には方角が必要です (例: move west)")?;
            Dir::parse(d)
                .map(Command::Move)
                .ok_or_else(|| format!("不明な方角: {d}"))
        }
        "attack" | "a" => {
            let d = args
                .first()
                .ok_or("attack には方角が必要です (例: attack east)")?;
            Dir::parse(d)
                .map(Command::Attack)
                .ok_or_else(|| format!("不明な方角: {d}"))
        }
        "descend" | "d" => Ok(Command::Descend),
        "inventory" | "i" => Ok(Command::Inventory),
        "use" | "u" | "drink" | "read" | "eat" => {
            let letter = args
                .first()
                .and_then(|a| letter_arg(a))
                .ok_or("use には持ち物の文字が必要です (例: use a)")?;
            let target = match args.get(1) {
                Some(a) => Some(letter_arg(a).ok_or_else(|| format!("不正な対象: {a}"))?),
                None => None,
            };
            Ok(Command::Use(letter, target))
        }
        "equip" | "e" | "wield" | "wear" | "unequip" | "r" | "remove" => {
            let equip = matches!(head, "equip" | "e" | "wield" | "wear");
            let letter = args
                .first()
                .and_then(|a| letter_arg(a))
                .ok_or_else(|| {
                    format!("{head} には持ち物の文字が必要です (例: {head} a)")
                })?;
            Ok(if equip {
                Command::Equip(letter)
            } else {
                Command::Unequip(letter)
            })
        }
        "travel" | "t" => match args.first().copied() {
            Some(">") | Some("stairs") => Ok(Command::Travel(TravelTarget::Stairs)),
            Some(other) => Err(format!("不明な移動先: {other} (travel > のみ対応)")),
            None => Err("travel には移動先が必要です (例: travel >)".to_string()),
        },
        "explore" | "x" => Ok(Command::Explore),
        "wait" | "z" => Ok(Command::Wait),
        "stay" => match args.first() {
            None => Ok(Command::Stay(1)),
            Some(a) => match a.parse::<u32>() {
                Ok(n) if (1..=STAY_LIMIT).contains(&n) => Ok(Command::Stay(n)),
                _ => Err(format!("stay のターン数は 1〜{STAY_LIMIT} の数字です: {a}")),
            },
        },
        "look" | "l" => Ok(Command::Look),
        other => Err(format!("不明なコマンド: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        assert_eq!(parse("move west"), Ok(Command::Move(Dir::W)));
        assert_eq!(parse("m ne"), Ok(Command::Move(Dir::NE)));
        assert_eq!(parse("travel >"), Ok(Command::Travel(TravelTarget::Stairs)));
        assert!(parse("move").is_err());
        assert!(parse("dance").is_err());
    }

    #[test]
    fn stay_takes_an_optional_turn_count() {
        assert_eq!(parse("stay"), Ok(Command::Stay(1)));
        assert_eq!(parse("stay 4"), Ok(Command::Stay(4)));
        assert!(parse("stay 0").is_err());
        assert!(parse("stay -1").is_err());
        assert!(parse("stay many").is_err());
        assert!(parse("stay 1001").is_err());
        assert_eq!(parse("stay 1000"), Ok(Command::Stay(1000)));
    }

    #[test]
    fn colon_and_backtick_prefixes_are_accepted() {
        assert_eq!(parse("`stay 4"), Ok(Command::Stay(4)));
        assert_eq!(parse(":stay 4"), Ok(Command::Stay(4)));
        assert_eq!(parse(":wait"), Ok(Command::Wait));
        // 正本の文字列には頭の記号は付かない
        assert_eq!(parse("`stay 4").unwrap().to_string(), "stay 4");
    }

    #[test]
    fn display_roundtrips() {
        for d in Dir::ALL {
            for c in [Command::Move(d), Command::Attack(d)] {
                assert_eq!(parse(&c.to_string()), Ok(c));
            }
        }
        for c in [
            Command::Descend,
            Command::Use('a', None),
            Command::Use('b', Some('a')),
            Command::Inventory,
            Command::Equip('c'),
            Command::Unequip('c'),
            Command::Travel(TravelTarget::Stairs),
            Command::Explore,
            Command::Wait,
            Command::Stay(1),
            Command::Stay(4),
            Command::Look,
        ] {
            assert_eq!(parse(&c.to_string()), Ok(c));
        }
    }
}
