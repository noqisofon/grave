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

/// 杖を向ける先。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZapTarget {
    /// 8方向のどれか
    Dir(Dir),
    /// いちばん近い、見えている敵
    Nearest,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Move(Dir),
    Attack(Dir),
    Descend,
    /// アミュレットを持っているとき、足元の階段で上の階へ登る（地下1階なら地上へ脱出）
    Ascend,
    /// 薬を飲む
    Quaff(char),
    /// 食べ物・キノコを食べる
    Eat(char),
    /// 巻物を読む。（識別の巻物のための）任意の対象つき
    Read(char, Option<char>),
    /// 杖を振る。向きか、いちばん近い敵。自分に効く杖は向きが要らない
    Zap(char, Option<ZapTarget>),
    /// 装備中のランタンに油つぼで燃料を継ぎ足す（文字を省くと最初の油つぼ）
    Refill(Option<char>),
    Inventory,
    /// 持ち物の文字の武器・防具を身につける
    Equip(char),
    /// 装備中の武器・防具をはずす
    Unequip(char),
    /// 持ち物の文字の物を、個数ぶん足元に捨てる
    Drop(char, u32),
    /// 足元の物を1個拾う（番号を省くと一番上の物）
    Pickup(Option<u32>),
    Travel(TravelTarget),
    Explore,
    Wait,
    /// その場に指定ターンだけ留まる（敵に襲われたら中断）
    Stay(u32),
    Look,
    /// 見つけた罠を解除する（向きを省くと足元）。確率で成功する
    Disarm(Option<Dir>),
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Move(d) => write!(f, "move {}", d.name()),
            Command::Attack(d) => write!(f, "attack {}", d.name()),
            Command::Descend => write!(f, "descend"),
            Command::Ascend => write!(f, "ascend"),
            Command::Quaff(c) => write!(f, "quaff {c}"),
            Command::Eat(c) => write!(f, "eat {c}"),
            Command::Read(c, None) => write!(f, "read {c}"),
            Command::Read(c, Some(t)) => write!(f, "read {c} {t}"),
            Command::Zap(c, None) => write!(f, "zap {c}"),
            Command::Zap(c, Some(ZapTarget::Dir(d))) => write!(f, "zap {c} {}", d.name()),
            Command::Zap(c, Some(ZapTarget::Nearest)) => write!(f, "zap {c} nearest"),
            Command::Refill(None) => write!(f, "refill"),
            Command::Refill(Some(c)) => write!(f, "refill {c}"),
            Command::Inventory => write!(f, "inventory"),
            Command::Equip(c) => write!(f, "equip {c}"),
            Command::Unequip(c) => write!(f, "unequip {c}"),
            Command::Drop(c, 1) => write!(f, "drop {c}"),
            Command::Drop(c, n) => write!(f, "drop {c} {n}"),
            Command::Pickup(None) => write!(f, "pickup"),
            Command::Pickup(Some(n)) => write!(f, "pickup {n}"),
            Command::Travel(TravelTarget::Stairs) => write!(f, "travel >"),
            Command::Explore => write!(f, "explore"),
            Command::Wait => write!(f, "wait"),
            Command::Stay(n) => write!(f, "stay {n}"),
            Command::Look => write!(f, "look"),
            Command::Disarm(None) => write!(f, "disarm"),
            Command::Disarm(Some(d)) => write!(f, "disarm {}", d.name()),
        }
    }
}

/// 補完用のコマンド名（正本）。
pub const COMMAND_NAMES: &[&str] = &[
    "move",
    "attack",
    "descend",
    "ascend",
    "quaff",
    "eat",
    "read",
    "zap",
    "refill",
    "inventory",
    "equip",
    "unequip",
    "drop",
    "pickup",
    "travel",
    "explore",
    "wait",
    "stay",
    "look",
    "disarm",
];

pub const COMMAND_HELP: &str = "\
move <dir>     1歩移動 (north/south/east/west/northeast/northwest/southeast/southwest, 略: n s e w ne nw se sw)
               敵のいる方向へ move すると攻撃になる
attack <dir>   その方向の敵を攻撃する (敵がいなければ失敗、ターン消費なし)
descend        足元の階段で下の階へ降りる (地下30階が最深部。アミュレットを持っていると降りられない)
ascend         アミュレットを持っているとき、足元の階段で上の階へ登る (地下1階で登ると地上へ脱出してクリア)
quaff <文字>   薬を飲む (薬以外には使えない。未識別の薬は使うと正体が分かる。悪い薬もある)
eat <文字>     食べ物・キノコを食べる (食べ物以外には使えない)
zap <文字> [向き|nearest]  杖を振る (1ターン。向きは move と同じ 8 方向。nearest は一番近い見えている敵。自分に効く杖は向き不要)。使用回数があり、0 になると使えない。残りの回数は inventory とステータスに出る
read <文字> [対象]  巻物を読む (巻物以外には使えない。盲目だと読めない)。識別の巻物は対象の文字を指定できる
inventory      持ち物の一覧 (ターン消費なし。装備中のものには (装備中) と付く)
equip <文字>   武器・防具・指輪・光源を身につける (武器・防具は1つずつ、指輪は2つまで。付け替えもこれ。光源は今の光源と持ち替える)
unequip <文字> 装備や指輪をはずす (呪われたものははずせない)
refill [文字]  装備中のランタンに油つぼで燃料を継ぎ足す (文字は油つぼ。省くと最初の油つぼ。松明には使えない。1ターン)
drop <文字> [数]  持ち物を足元に捨てる (1ターン。数を省くと1個。装備中のものは先に unequip。捨てた物は歩いても stay しても自動では拾われない)
pickup [番号]  足元の物を1個拾う (別名 get。1ターン。番号は look や観測の「足元」の番号。省くと番号1。捨てた物もこれで拾える)
travel > (<)   既知の階段まで自動移動
explore        未探索の場所へ自動移動 (階段を見つける・敵が見える・攻撃を受けると止まる。敵が見えている間は使えない)
wait           1ターン待つ
stay [ターン数]  その場に指定ターンだけ留まる。足元のアイテムを拾う (省略すると1ターン。敵に襲われたり体力が危なくなったら中断。上限1000)
look           階段や見えている敵の位置を調べる (ターン消費なし)
disarm [向き]  見つけた罠(^)を解除する (1ターン。向きは隣のマス、省くと足元。成功率は約60%で、器用さの指輪で上がり、混乱で下がる。失敗すると3回に1回は罠が作動する(浮遊中でも毒矢と眠りガスは作動する。落とし穴は浮いていれば落ちない)。隠れた罠は解除できない)
複数のコマンドは ; で区切って連続実行できる (失敗したらそこで止まる)
時間が経つと満腹度が減り、0 になると体力が削られる。食べ物 (パン・干し肉) やキノコで回復する。パンは腐っていることがある。キノコは最初は未識別で、食べると毒になるものもある。毒状態では1ターンごとに1ダメージを受け、自然回復しない (回復の薬・解毒の薬で治る)。
敵を倒すと経験値を得て、たまるとレベルが上がる(最大HP+4でその分回復、2レベルごとに攻撃+1)。武器は攻撃のダメージを、防具は受けるダメージ(最低1)を変える。素手は攻撃 2〜4。
装備は1個ずつ別物で、品質(接頭辞の語: Basic/Okay… < Superior/Prime… < Mystical/Sanctified… < Eldritch/Primeval…)が上がるほど強い。Uncommon 以上には特殊効果(of X)が付くことがあり、名前の (?) は接尾辞や正確な補正値が未識別という印。識別の巻物か、装備して一定ターン経つと分かる。呪われた装備(装備して初めて分かる)は強いが、はずせない。
状態異常は「状態」欄に残りターンつきで出る: 毒(毎ターン1ダメージ) 混乱(移動や攻撃の向きがずれる) 幻覚(敵の名前と記号がでたらめになる。HPと位置は本物) 盲目(マップも敵も見えない。巻物は読めない。観測にマップは出ない) 睡眠・停止(行動できず、解けるまで時間が過ぎる) 加速(1ターンに2回行動) 減速(1回の行動に2ターン) 浮遊(どの罠にもかからない) 透明(敵は2マス以内でないと気づかない) 透明視認(透明な敵が見える)。敵にも付き、見えている敵の横に [混乱5] のように出る。混乱・盲目の間は travel / explore が使えない。
床には隠れた罠がある(落とし穴・毒矢・眠りガス)。踏むか、近くに立っていると見つかり、^ で表示される。見つけた罠は travel / explore が避ける。浮遊中はどの罠も作動しない。
指輪(=): 防御・腕力・器用さ・ダメージ増加・再生・消化遅延・隠密・探索・透明視認・装飾 (+呪われた「怒らせる」「テレポート癖」)。見た目(宝石名)では正体が分からず、身につけて30ターン過ごすか識別すると分かる(効果は未識別でも効く)。強さが負の指輪や呪われた指輪は、身につけると外せない(呪い解除の巻物で外せる)。
光源(~): 最初に松明(燃料1500)を装備している。燃料は1ターンごとに減り、ステータスの「光源:松明 燃料N/M」と inventory に出る。100を切ると知らせ、0で燃え尽きて視界が半径2に狭まる(敵はこちらを見つけられる)。松明を拾って equip で持ち替えるか、ランタン(燃料4000、地下3階〜)に油つぼ(~, +1500)を refill で継ぎ足す。
アイテムの上を歩くと自動で拾う。薬と巻物は最初は未識別で、使うと正体が分かる (ゲームごとに見た目と効果の対応が変わる)
クリア条件: 地下30階の魔除けのアミュレット(,)を手に入れ、階段を登って地上まで持ち帰る。アミュレットを持つと階段は登り階段(<)になる。
凡例: @ 自分  ^ 見つけた罠  > 階段(アミュレットを持つと <)  , アミュレット  ! 薬  ? 巻物  / 杖  = 指輪  ~ 光源・油つぼ  ) 武器  [ 防具  % 食べ物・キノコ  s スライム  b コウモリ  g ゴブリン  O オーガ  S 毒グモ  a アクアター(殴られると鎧が錆びる)";

/// 持ち物の文字（小文字1つ）。
fn letter_arg(s: &str) -> Option<char> {
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) if ch.is_ascii_lowercase() => Some(ch),
        _ => None,
    }
}

/// 持ち物を扱うコマンドの第1引数（持ち物の文字）。
fn item_letter(head: &str, args: &[&str]) -> Result<char, String> {
    args.first()
        .and_then(|a| letter_arg(a))
        .ok_or_else(|| format!("{head} には持ち物の文字が必要です (例: {head} a)"))
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
        "ascend" | "up" => Ok(Command::Ascend),
        "inventory" | "i" => Ok(Command::Inventory),
        "quaff" | "drink" | "q" => Ok(Command::Quaff(item_letter(head, &args)?)),
        "eat" => Ok(Command::Eat(item_letter(head, &args)?)),
        "read" => {
            let letter = item_letter(head, &args)?;
            let target = match args.get(1) {
                Some(a) => Some(letter_arg(a).ok_or_else(|| format!("不正な対象: {a}"))?),
                None => None,
            };
            Ok(Command::Read(letter, target))
        }
        "zap" | "aim" => {
            let letter = item_letter(head, &args)?;
            let target = match args.get(1).copied() {
                None => None,
                Some("nearest" | "near" | "target") => Some(ZapTarget::Nearest),
                Some(a) => {
                    Some(ZapTarget::Dir(Dir::parse(a).ok_or_else(|| {
                        format!("不明な向き: {a} (8方向か nearest)")
                    })?))
                }
            };
            Ok(Command::Zap(letter, target))
        }
        "refill" | "fuel" => match args.first() {
            None => Ok(Command::Refill(None)),
            Some(a) => Ok(Command::Refill(Some(
                letter_arg(a).ok_or_else(|| format!("不正な油つぼの文字: {a}"))?,
            ))),
        },
        "use" | "u" => Err(
            "use はない。薬は quaff、食べ物は eat、巻物は read、装備は equip を使う".to_string(),
        ),
        "equip" | "e" | "wield" | "wear" | "unequip" | "r" | "remove" => {
            let equip = matches!(head, "equip" | "e" | "wield" | "wear");
            let letter = args
                .first()
                .and_then(|a| letter_arg(a))
                .ok_or_else(|| format!("{head} には持ち物の文字が必要です (例: {head} a)"))?;
            Ok(if equip {
                Command::Equip(letter)
            } else {
                Command::Unequip(letter)
            })
        }
        "drop" => {
            let letter = args
                .first()
                .and_then(|a| letter_arg(a))
                .ok_or("drop には持ち物の文字が必要です (例: drop a / drop b 3)")?;
            let n = match args.get(1) {
                None => 1,
                Some(a) => match a.parse::<u32>() {
                    Ok(n) if n >= 1 => n,
                    _ => return Err(format!("drop の個数は 1 以上の数字です: {a}")),
                },
            };
            Ok(Command::Drop(letter, n))
        }
        "pickup" | "get" => match args.first() {
            None => Ok(Command::Pickup(None)),
            Some(a) => match a.parse::<u32>() {
                Ok(n) if n >= 1 => Ok(Command::Pickup(Some(n))),
                _ => Err(format!("pickup の番号は 1 以上の数字です: {a}")),
            },
        },
        "travel" | "t" => match args.first().copied() {
            Some(">") | Some("<") | Some("stairs") => Ok(Command::Travel(TravelTarget::Stairs)),
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
        "disarm" => match args.first() {
            None => Ok(Command::Disarm(None)),
            Some(a) => Dir::parse(a)
                .map(|d| Command::Disarm(Some(d)))
                .ok_or_else(|| format!("不明な向き: {a} (8方向)")),
        },
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
    fn item_commands_are_separate_and_use_is_gone() {
        assert_eq!(parse("quaff b"), Ok(Command::Quaff('b')));
        assert_eq!(parse("eat c"), Ok(Command::Eat('c')));
        assert_eq!(parse("read d"), Ok(Command::Read('d', None)));
        assert_eq!(parse("read d a"), Ok(Command::Read('d', Some('a'))));
        assert!(parse("quaff").is_err());
        assert!(parse("use a").unwrap_err().contains("quaff"));
        assert!(parse("u a").is_err());
    }

    #[test]
    fn zap_parses_directions_and_nearest() {
        assert_eq!(
            parse("zap c east"),
            Ok(Command::Zap('c', Some(ZapTarget::Dir(Dir::E))))
        );
        assert_eq!(
            parse("zap c n"),
            Ok(Command::Zap('c', Some(ZapTarget::Dir(Dir::N))))
        );
        assert_eq!(
            parse("zap c nearest"),
            Ok(Command::Zap('c', Some(ZapTarget::Nearest)))
        );
        assert_eq!(parse("aim c"), Ok(Command::Zap('c', None)));
        assert!(parse("zap").is_err());
        assert!(parse("zap c sideways").is_err());
    }

    #[test]
    fn drop_and_pickup_parse() {
        assert_eq!(parse("drop a"), Ok(Command::Drop('a', 1)));
        assert_eq!(parse("drop b 3"), Ok(Command::Drop('b', 3)));
        assert!(parse("drop").is_err());
        assert!(parse("drop a 0").is_err());
        assert!(parse("drop a x").is_err());
        assert_eq!(parse("pickup"), Ok(Command::Pickup(None)));
        assert_eq!(parse("get 2"), Ok(Command::Pickup(Some(2))));
        assert!(parse("pickup 0").is_err());
        assert!(parse("pickup x").is_err());
        // d は今までどおり descend の別名
        assert_eq!(parse("d"), Ok(Command::Descend));
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
            Command::Ascend,
            Command::Quaff('a'),
            Command::Eat('b'),
            Command::Read('c', None),
            Command::Read('c', Some('a')),
            Command::Zap('b', None),
            Command::Zap('b', Some(ZapTarget::Dir(Dir::SE))),
            Command::Zap('b', Some(ZapTarget::Nearest)),
            Command::Refill(None),
            Command::Refill(Some('d')),
            Command::Inventory,
            Command::Equip('c'),
            Command::Unequip('c'),
            Command::Drop('a', 1),
            Command::Drop('b', 3),
            Command::Pickup(None),
            Command::Pickup(Some(2)),
            Command::Travel(TravelTarget::Stairs),
            Command::Explore,
            Command::Wait,
            Command::Stay(1),
            Command::Stay(4),
            Command::Look,
            Command::Disarm(None),
            Command::Disarm(Some(Dir::NW)),
        ] {
            assert_eq!(parse(&c.to_string()), Ok(c));
        }
    }
}
