mod keymap;
mod watch;

use std::io::{self, Write};
use std::time::Duration;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use grave_core::journal;
use grave_core::map::{H, W};
use grave_core::record::Event as RecEvent;
use grave_core::{Class, Game, ItemEntry, ItemKind, COMMAND_HELP, COMMAND_NAMES};
use keymap::Keymap;

/// ゲームコマンド以外の、TUI 側で処理する組み込みコマンド。
const BUILTINS: &[&str] = &["map", "unmap", "new", "new_game", "quit", "exit", "help"];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Mode {
    Normal,
    Command,
}

struct App {
    game: Game,
    keymap: Keymap,
    mode: Mode,
    cmdline: String,
    history: Vec<String>,
    hist_pos: Option<usize>,
    count: Option<u32>,
    last: Option<(String, u32)>,
    status: String,
    quit: bool,
    /// 持ち物を、マップの右上に重ねて出している（次のキーで閉じる）
    overlay: bool,
    /// 持ち物オーバーレイの先頭に出す項目の番号（スクロール位置）
    overlay_scroll: usize,
    /// `:help` の全画面表示。中身は先頭行の番号（スクロール位置）
    help: Option<usize>,
}

impl App {
    fn new(seed: u64) -> App {
        App {
            game: Game::new(seed),
            keymap: Keymap::with_defaults(),
            mode: Mode::Normal,
            cmdline: String::new(),
            history: Vec::new(),
            hist_pos: None,
            count: None,
            last: None,
            status: "h/j/k/l で移動、: でコマンド、:help で一覧".to_string(),
            quit: false,
            overlay: false,
            overlay_scroll: 0,
            help: None,
        }
    }

    /// 1行を実行。組み込みコマンドか、ゲームコマンド（; 区切り可）。成功なら true。
    fn exec_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        let mut it = line.split_whitespace();
        let head = it.next().unwrap_or("");
        let args: Vec<&str> = it.collect();
        match head {
            "" => true,
            "quit" | "exit" => {
                self.quit = true;
                true
            }
            "help" => {
                self.help = Some(0);
                true
            }
            "new" | "new_game" => {
                let seed = args
                    .first()
                    .and_then(|a| a.parse().ok())
                    .unwrap_or_else(random_seed);
                self.game = Game::new(seed);
                self.last = None;
                self.status = format!("新しい冒険 (seed {seed})");
                true
            }
            "map" => {
                if args.is_empty() {
                    let list: Vec<String> = self
                        .keymap
                        .list()
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect();
                    self.status = list.join("  ");
                    return true;
                }
                let mut chars = args[0].chars();
                match (chars.next(), chars.next(), args.len() > 1) {
                    (Some(c), None, true) => {
                        let cmd = args[1..].join(" ");
                        match grave_core::command::parse(&cmd) {
                            Ok(_) => {
                                self.keymap.set(c, cmd.clone());
                                self.status = format!("{c} → {cmd}");
                                true
                            }
                            Err(e) => {
                                self.status = format!("map: {e}");
                                false
                            }
                        }
                    }
                    _ => {
                        self.status = "使い方: :map <1文字> <コマンド>".to_string();
                        false
                    }
                }
            }
            "unmap" => {
                let c = args.first().and_then(|a| {
                    let mut ch = a.chars();
                    match (ch.next(), ch.next()) {
                        (Some(c), None) => Some(c),
                        _ => None,
                    }
                });
                match c {
                    Some(c) if self.keymap.unmap(c) => {
                        self.status = format!("{c} の割り当てを外した");
                        true
                    }
                    _ => {
                        self.status = "使い方: :unmap <割り当て済みの1文字>".to_string();
                        false
                    }
                }
            }
            _ => {
                let outs = self.game.run_script(line);
                let ok = outs.iter().all(|o| o.ok);
                if let Some(last) = outs.last() {
                    self.status = format!(":{}  {}", last.command, last.message);
                }
                // `inventory` を実行したときだけ、持ち物をマップの上に重ねる
                if outs.iter().any(|o| o.command == "inventory") {
                    self.overlay = true;
                    self.overlay_scroll = 0;
                }
                ok
            }
        }
    }

    fn run_with_count(&mut self, cmd: &str, count: u32) {
        for _ in 0..count {
            if !self.exec_line(cmd) || self.quit {
                break;
            }
        }
        self.last = Some((cmd.to_string(), count));
    }

    fn on_key_normal(&mut self, code: KeyCode) {
        if self.help.is_some() {
            self.on_key_help(code);
            return;
        }
        if self.overlay {
            // 方向キーなどは持ち物の一覧をスクロールする
            let total = self.game.inventory_lines().len();
            if let Some(next) = scrolled(self.overlay_scroll, total, OVERLAY_ROWS, code) {
                self.overlay_scroll = next;
                return;
            }
            self.overlay = false;
            // 閉じるためのキーはそれだけで終わり。ほかのキーは閉じたうえで普通に働く
            if matches!(
                code,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i') | KeyCode::Char(' ')
            ) {
                return;
            }
        }
        match code {
            // `:` が正本。打ちにくい人向けに `` ` `` も同じ意味で受け付ける
            KeyCode::Char(':') | KeyCode::Char('`') => {
                self.mode = Mode::Command;
                self.cmdline.clear();
                self.hist_pos = None;
                self.count = None;
            }
            KeyCode::Char(c @ '1'..='9') => {
                let d = c.to_digit(10).unwrap();
                self.count = Some((self.count.unwrap_or(0) * 10 + d).min(999));
            }
            KeyCode::Char('0') if self.count.is_some() => {
                self.count = Some((self.count.unwrap() * 10).min(999));
            }
            KeyCode::Char('.') => {
                self.count = None;
                if let Some((cmd, n)) = self.last.clone() {
                    self.run_with_count(&cmd, n);
                }
            }
            KeyCode::Char(c) => {
                let n = self.count.take().unwrap_or(1);
                if let Some(cmd) = self.keymap.get(c).map(str::to_string) {
                    if cmd.ends_with(' ') {
                        // 例: q → ":quaff " を開いて、文字を打ってもらう
                        self.mode = Mode::Command;
                        self.cmdline = cmd;
                        self.hist_pos = None;
                    } else {
                        self.run_with_count(&cmd, n);
                    }
                } else {
                    self.status = format!("割り当てなし: {c}  (:help)");
                }
            }
            KeyCode::Left => self.run_arrow("move west"),
            KeyCode::Down => self.run_arrow("move south"),
            KeyCode::Up => self.run_arrow("move north"),
            KeyCode::Right => self.run_arrow("move east"),
            KeyCode::Home => self.run_arrow("move northwest"),
            KeyCode::PageUp => self.run_arrow("move northeast"),
            KeyCode::End => self.run_arrow("move southwest"),
            KeyCode::PageDown => self.run_arrow("move southeast"),
            KeyCode::KeypadBegin => self.run_arrow("wait"),
            KeyCode::Esc => self.count = None,
            _ => {}
        }
    }

    /// 全画面のヘルプ。方向キーでスクロールし、Esc / q / Enter で閉じる。
    fn on_key_help(&mut self, code: KeyCode) {
        let (cols, rows) = text_view_size();
        let total = wrap(&help_text(), cols).len();
        let cur = self.help.unwrap_or(0);
        match scrolled(cur, total, rows, code) {
            Some(next) => self.help = Some(next),
            None => {
                if matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.help = None;
                }
            }
        }
    }

    fn run_arrow(&mut self, cmd: &str) {
        let n = self.count.take().unwrap_or(1);
        self.run_with_count(cmd, n);
    }

    fn on_key_command(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.cmdline.clear();
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                // 対象が要るコマンドだけを打って Enter したら、実行せずに「何を？」の候補を出す
                if let Some(head) = item_prompt_head(line.trim()) {
                    self.mode = Mode::Command;
                    self.cmdline = format!("{head} ");
                    self.hist_pos = None;
                    return;
                }
                if !line.trim().is_empty() {
                    self.history.push(line.clone());
                    self.exec_line(&line);
                }
            }
            KeyCode::Backspace => {
                if self.cmdline.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return;
                }
                let p = match self.hist_pos {
                    None => self.history.len() - 1,
                    Some(p) => p.saturating_sub(1),
                };
                self.hist_pos = Some(p);
                self.cmdline = self.history[p].clone();
            }
            KeyCode::Down => {
                if let Some(p) = self.hist_pos {
                    if p + 1 < self.history.len() {
                        self.hist_pos = Some(p + 1);
                        self.cmdline = self.history[p + 1].clone();
                    } else {
                        self.hist_pos = None;
                        self.cmdline.clear();
                    }
                }
            }
            KeyCode::Tab => self.complete(),
            KeyCode::Char(c) => self.cmdline.push(c),
            _ => {}
        }
    }

    /// コマンド行での Ctrl+文字（シェル風の編集キー）。Ctrl+W は直前の1語、Ctrl+U は行全体、
    /// Ctrl+H は Backspace と同じ。ほかの Ctrl+文字は何もしない。
    fn on_ctrl_command(&mut self, c: char) {
        match c.to_ascii_lowercase() {
            'w' => {
                while self.cmdline.ends_with(char::is_whitespace) {
                    self.cmdline.pop();
                }
                while self
                    .cmdline
                    .chars()
                    .last()
                    .is_some_and(|c| !c.is_whitespace())
                {
                    self.cmdline.pop();
                }
            }
            'u' => self.cmdline.clear(),
            'h' => self.on_key_command(KeyCode::Backspace),
            _ => {}
        }
    }

    /// 先頭の語だけをコマンド名として補完する（候補が一意のときのみ）。
    fn complete(&mut self) {
        if self.cmdline.contains(' ') {
            return;
        }
        let cands: Vec<&str> = COMMAND_NAMES
            .iter()
            .chain(BUILTINS.iter())
            .copied()
            .filter(|n| n.starts_with(self.cmdline.as_str()))
            .collect();
        if cands.len() == 1 {
            self.cmdline = format!("{} ", cands[0]);
        }
    }

    fn draw(&self, out: &mut impl Write) -> io::Result<()> {
        if let Some(scroll) = self.help {
            return draw_text_screen(
                out,
                "ヘルプ (↑↓ PgUp PgDn Home End でスクロール / Esc か q で戻る)",
                &help_text(),
                scroll,
            );
        }
        let footer = match self.mode {
            Mode::Command => format!(":{}", self.cmdline),
            Mode::Normal => {
                let count = self.count.map(|c| c.to_string()).unwrap_or_default();
                format!("{}  {}", self.status, count)
            }
        };
        let prompt = if self.mode == Mode::Command {
            prompt_for_cmdline(&self.game, &self.cmdline)
        } else {
            None
        };
        let inv_lines;
        let overlay = if let Some(p) = &prompt {
            Some(OverlayView {
                title: p.title,
                empty: p.empty,
                footer: "(Esc で戻る)",
                items: &p.items,
                scroll: 0,
            })
        } else if self.overlay {
            inv_lines = self.game.inventory_lines();
            Some(OverlayView {
                title: "持ち物",
                empty: "(なし)",
                footer: "(Esc か i で閉じる)",
                items: &inv_lines,
                scroll: self.overlay_scroll,
            })
        } else {
            None
        };
        draw_scene(
            out,
            &self.game,
            None,
            &footer,
            self.mode == Mode::Command,
            overlay.as_ref(),
        )
    }
}

struct OverlayView<'a> {
    title: &'a str,
    empty: &'a str,
    footer: &'a str,
    items: &'a [String],
    /// 先頭に出す項目の番号
    scroll: usize,
}

struct OverlayPrompt {
    title: &'static str,
    empty: &'static str,
    items: Vec<String>,
}

/// 対象の持ち物の文字が要るコマンド名（別名を含む）なら、正式な名前。
/// `refill` は文字を省いてよいので含めない。
fn item_prompt_head(word: &str) -> Option<&'static str> {
    Some(match word {
        "read" | "r" => "read",
        "quaff" | "q" | "drink" => "quaff",
        "eat" | "e" => "eat",
        "equip" | "w" | "wear" | "wield" => "equip",
        "unequip" | "remove" => "unequip",
        "zap" | "aim" => "zap",
        "drop" => "drop",
        _ => return None,
    })
}

/// コマンドライン入力中のコマンドに応じて、対象アイテムの候補一覧と案内を返す。
fn prompt_for_cmdline(game: &Game, cmdline: &str) -> Option<OverlayPrompt> {
    let mut parts = cmdline.split_whitespace();
    let head = parts.next()?;
    let args: Vec<&str> = parts.collect();
    // コマンド名だけを打っている間は出さない。続きを打つ（空白のあと）か Enter で出す
    if args.len() > 1 || (args.is_empty() && !cmdline.ends_with(char::is_whitespace)) {
        return None;
    }
    let entries = game.inventory_entries();
    let (title, empty, filter): (&'static str, &'static str, fn(&ItemEntry) -> bool) = match head {
        "read" | "r" => (
            "どれを読む？ (巻物)",
            "(読める巻物がない)",
            |e| e.kind.is_scroll(),
        ),
        "quaff" | "q" | "drink" => ("どれを飲む？ (薬)", "(飲める薬がない)", |e| {
            e.kind.is_potion()
        }),
        "eat" | "e" => (
            "どれを食べる？",
            "(食べられるものがない)",
            |e| matches!(e.kind.class(), Class::Food | Class::Mushroom),
        ),
        "equip" | "w" | "wear" | "wield" => (
            "どれを装備する？",
            "(装備できるものがない)",
            |e| {
                !e.equipped
                    && (e.kind.is_equipment() || e.kind.is_ring() || e.kind.class() == Class::Light)
            },
        ),
        "unequip" | "remove" => ("どれをはずす？", "(はずせる装備がない)", |e| e.equipped),
        "zap" | "aim" => ("どの杖を振る？", "(振れる杖がない)", |e| {
            e.kind.class() == Class::Wand
        }),
        "drop" => ("どれを捨てる？", "(捨てられるものがない)", |e| !e.equipped),
        "refill" | "fuel" => ("どの油を使う？", "(油つぼがない)", |e| {
            e.kind == ItemKind::OilFlask
        }),
        _ => return None,
    };
    let items: Vec<String> = entries.into_iter().filter(filter).map(|e| e.line).collect();
    Some(OverlayPrompt {
        title,
        empty,
        items,
    })
}

/// 全角を2桁として、`cols` 桁に収まるぶんだけ切り出す。
fn clip_cols(s: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = if c.is_ascii() { 1 } else { 2 };
        if w + cw > cols {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// 画面に出すログの行数。
const LOG_LINES: usize = 3;

/// 画面の行の割り当て。
struct Layout {
    log_top: u16,
    thoughts_top: u16,
    /// 最下行（プロンプト）
    footer: u16,
}

/// 端末の高さ `rows` に合わせた行の割り当て。プロンプトは端末の一番下に置く。
/// 高さが足りないときは、ログ（または思考）の次の行に置く。
fn layout(rows: u16, with_thoughts: bool) -> Layout {
    let log_top = (H + 1) as u16;
    let thoughts_top = log_top + LOG_LINES as u16;
    let natural = thoughts_top + if with_thoughts { 3 } else { 0 };
    Layout {
        log_top,
        thoughts_top,
        footer: natural.max(rows.saturating_sub(1)),
    }
}

/// マップ・ログ・（観戦時は）思考・最下行（プロンプト）を描く。
fn draw_scene(
    out: &mut impl Write,
    game: &Game,
    thoughts: Option<&[(u32, String)]>,
    footer: &str,
    cursor: bool,
    overlay: Option<&OverlayView>,
) -> io::Result<()> {
    // ちらつき対策: 1フレームぶんを一度に組み立てて、1回の書き込みで送る
    let mut frame: Vec<u8> = Vec::with_capacity(8 * 1024);
    render_scene(&mut frame, game, thoughts, footer, cursor, overlay)?;
    out.write_all(&frame)?;
    out.flush()
}

fn render_scene(
    out: &mut Vec<u8>,
    game: &Game,
    thoughts: Option<&[(u32, String)]>,
    footer: &str,
    cursor: bool,
    overlay: Option<&OverlayView>,
) -> io::Result<()> {
    // 全角文字は2桁ぶん使うので、文字数の上限は桁数の半分にしておく
    let (cols, rows) = terminal::size().unwrap_or((80, 30));
    let cap = cols as usize / 2;
    let lay = layout(rows, thoughts.is_some());
    // 画面全体は消さない（消すとちらつく）。変わる行を、その場で消して描き直す。
    // 対応する端末では、フレームが描き終わるまで画面の更新を保留する。
    queue!(out, BeginSynchronizedUpdate, Hide)?;
    for y in std::iter::once(0).chain((H as u16 + 1)..=lay.footer) {
        queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
    }
    queue!(out, MoveTo(0, 0))?;
    queue!(
        out,
        Print(format!(
            "地下{}階  Lv{}  ターン{}  HP {}/{}  {}{}",
            game.depth(),
            game.level(),
            game.turn(),
            game.hp().max(0),
            game.max_hp(),
            game.status_text(),
            if game.is_dead() {
                "  ★ゲームオーバー (:new で再開)"
            } else if game.is_won() {
                "  ★クリア！ (:new で再開)"
            } else {
                ""
            }
        ))
    )?;
    for y in 0..H {
        queue!(out, MoveTo(0, (y + 1) as u16))?;
        for x in 0..W {
            let c = game.cell(x, y);
            let color = if c.ch == '@' {
                Color::Yellow
            } else if c.ch == '>' || c.ch == '<' {
                Color::Green
            } else if c.ch == '!' {
                Color::Magenta
            } else if c.ch == '?' {
                Color::Cyan
            } else if c.ch == '/' {
                Color::DarkCyan
            } else if c.ch == '=' || c.ch == '~' {
                Color::DarkMagenta
            } else if c.ch == '^' {
                Color::DarkRed
            } else if c.ch == ')' || c.ch == '[' {
                Color::Blue
            } else if c.ch == ',' {
                Color::Yellow
            } else if c.ch == '%' {
                Color::DarkYellow
            } else if c.ch.is_ascii_alphabetic() {
                Color::Red
            } else if c.visible {
                Color::White
            } else {
                Color::DarkGrey
            };
            queue!(out, SetForegroundColor(color), Print(c.ch))?;
        }
        // 行の右側に前のフレームの文字が残らないように
        queue!(out, ResetColor, Clear(ClearType::UntilNewLine))?;
    }
    let log = game.log();
    let start = log.len().saturating_sub(LOG_LINES);
    for (i, e) in log[start..].iter().enumerate() {
        queue!(
            out,
            MoveTo(0, lay.log_top + i as u16),
            Print(clip(&format!("[{}] {}", e.turn, e.text), cap))
        )?;
    }
    if let Some(ts) = thoughts {
        let start = ts.len().saturating_sub(3);
        for (i, (turn, t)) in ts[start..].iter().enumerate() {
            queue!(
                out,
                MoveTo(0, lay.thoughts_top + i as u16),
                SetForegroundColor(Color::Cyan),
                Print(clip(&format!("思[{turn}] {t}"), cap)),
                ResetColor
            )?;
        }
    }
    // プロンプトより下に前のフレームの残りがあれば消す（最下行を消さないように注意）
    if lay.footer + 1 < rows {
        queue!(
            out,
            MoveTo(0, lay.footer + 1),
            Clear(ClearType::FromCursorDown)
        )?;
    }
    queue!(
        out,
        MoveTo(0, lay.footer),
        Print(clip_cols(footer, (cols as usize).saturating_sub(1)))
    )?;
    if let Some(ov) = overlay {
        draw_overlay(out, ov, cols)?;
    }
    if cursor {
        queue!(out, Show)?;
    } else {
        queue!(out, Hide)?;
    }
    queue!(out, EndSynchronizedUpdate)?;
    Ok(())
}

/// 全角を2桁として数えた表示幅。
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

/// 持ち物やコマンド候補を、マップの右上に重ねて描く。
fn draw_overlay(out: &mut impl Write, ov: &OverlayView, cols: u16) -> io::Result<()> {
    let mut lines: Vec<(String, Color)> = vec![(ov.title.to_string(), Color::Cyan)];
    if ov.items.is_empty() {
        lines.push((ov.empty.to_string(), Color::White));
    }
    let total = ov.items.len();
    let start = ov.scroll.min(total.saturating_sub(OVERLAY_ROWS));
    for l in ov.items.iter().skip(start).take(OVERLAY_ROWS) {
        let color = if l.contains("(装備中)") {
            Color::Yellow
        } else {
            Color::White
        };
        lines.push((l.clone(), color));
    }
    // 一覧が1画面に収まらないときは、見えている範囲と操作を知らせる
    if total > OVERLAY_ROWS {
        let end = (start + OVERLAY_ROWS).min(total);
        lines.push((
            format!("{}-{}/{}  ↑↓でスクロール", start + 1, end, total),
            Color::DarkGrey,
        ));
    }
    lines.push((ov.footer.to_string(), Color::DarkGrey));
    // マップの右端にそろえる。端末が狭ければ端末の右端まで
    let right = (W as u16).min(cols) as usize;
    let inner = lines
        .iter()
        .map(|(l, _)| display_width(l))
        .max()
        .unwrap_or(0);
    let width = (inner + 2).min(right);
    let x0 = (right - width) as u16;
    for (i, (text, color)) in lines.iter().enumerate() {
        let body = clip_cols(text, width - 2);
        let pad = width - 2 - display_width(&body);
        queue!(
            out,
            MoveTo(x0, 1 + i as u16),
            SetForegroundColor(color.to_owned()),
            Print(format!(" {body}{} ", " ".repeat(pad))),
            ResetColor
        )?;
    }
    Ok(())
}

/// 全角は2桁として、`cols` 桁で折り返す。
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut cur = String::new();
        let mut w = 0;
        for c in para.chars() {
            let cw = if c.is_ascii() { 1 } else { 2 };
            if w + cw > cols {
                lines.push(std::mem::take(&mut cur));
                w = 0;
            }
            cur.push(c);
            w += cw;
        }
        lines.push(cur);
    }
    lines
}

/// 一覧オーバーレイに出せる項目の数。
const OVERLAY_ROWS: usize = H as usize - 2;

/// スクロールのキーなら、動かしたあとの先頭行（`total` 行のうち `page` 行が見える）。
/// スクロールのキーでなければ None。
fn scrolled(cur: usize, total: usize, page: usize, code: KeyCode) -> Option<usize> {
    let max = total.saturating_sub(page);
    let next = match code {
        KeyCode::Up => cur.saturating_sub(1),
        KeyCode::Down => cur + 1,
        KeyCode::PageUp => cur.saturating_sub(page.saturating_sub(1).max(1)),
        KeyCode::PageDown | KeyCode::Char(' ') => cur + page.saturating_sub(1).max(1),
        KeyCode::Home => 0,
        KeyCode::End => max,
        _ => return None,
    };
    Some(next.min(max))
}

/// 全画面テキストの折り返し桁数と、見える行数。
fn text_view_size() -> (usize, usize) {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    (
        (cols as usize).saturating_sub(2).max(10),
        (rows as usize).saturating_sub(3).max(1),
    )
}

/// `:help` の本文。TUI のキー操作と、ゲームのコマンド一覧。
fn help_text() -> String {
    format!("{HELP_KEYS}\n{COMMAND_HELP}")
}

const HELP_KEYS: &str = "\
== キー操作 ==
h j k l / 矢印    西 南 北 東に移動 (敵のいる方向へ移動すると攻撃)
y u b n           北西 北東 南西 南東に移動
Home PgUp End PgDn  テンキーの斜め移動    KeypadBegin  待つ
q + 文字          薬を飲む          e + 文字   食べる
r + 文字          巻物を読む        w / E + 文字  装備する
R / T + 文字      装備をはずす      a / Z + 文字 + 向き  杖を振る
d + 文字          持ち物を捨てる    ,   足元の物を拾う
F                 ランタンに油を継ぐ
i                 持ち物 (↑↓ でスクロール、Esc か i で閉じる)
> / <             階段を降りる / 登る (アミュレット所持時)
_                 階段へ自動移動    x   自動探索    z   待つ    ;   見る
3j                数字+キーで反復   .   直前の操作を繰り返す
: または `        コマンドライン (Tab 補完、↑↓ 履歴、; で連続実行)
:map <1文字> <コマンド>   キーの再割り当て    :unmap <1文字>
:new [seed]       最初からやり直す  :quit   終了 (:q は quaff の略)
罠の解除にはキーがない:  :disarm [向き]

== コマンド一覧 ==";

fn draw_text_screen(
    out: &mut impl Write,
    title: &str,
    text: &str,
    scroll: usize,
) -> io::Result<()> {
    let (wrap_cols, page) = text_view_size();
    let lines = wrap(text, wrap_cols);
    let scroll = scroll.min(lines.len().saturating_sub(page));
    queue!(
        out,
        Clear(ClearType::All),
        MoveTo(0, 0),
        SetForegroundColor(Color::Cyan),
        Print(title),
        ResetColor
    )?;
    for (i, l) in lines.iter().skip(scroll).take(page).enumerate() {
        queue!(out, MoveTo(0, (i + 2) as u16), Print(l))?;
    }
    queue!(out, Hide)?;
    out.flush()
}

/// 記録から冒険の素材と、書かれた日誌を標準出力に出す。
fn run_journal(path: &str) -> io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let events: Vec<RecEvent> = text
        .lines()
        .filter_map(|l| RecEvent::parse(l.trim()).ok())
        .collect();
    if events.is_empty() {
        eprintln!("記録がない: {path}");
        return Ok(());
    }
    println!("{}", journal::digest(&events));
    let written = journal::journal_texts(&events);
    if written.is_empty() {
        println!("(日誌はまだ書かれていない)");
    } else {
        for (i, t) in written.iter().enumerate() {
            println!("## 日誌 {}\n\n{t}\n", i + 1);
        }
    }
    Ok(())
}

/// 生端末モードと代替画面のライフサイクルを管理する RAII ガード。
/// パニック発生時やスコープ終了時にも確実に端末状態を復元する。
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, Hide)?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = execute!(out, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn run_watch(path: &str) -> io::Result<()> {
    let mut w = watch::Watcher::new(path);
    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    let mut dirty = true;
    let mut show_journal = false;
    let mut show_inv = false;
    let mut inv_scroll = 0usize;
    loop {
        if w.poll()? {
            dirty = true;
        }
        if dirty {
            if show_journal && !w.journals.is_empty() {
                let text = w.journals.last().unwrap();
                draw_text_screen(&mut out, "冒険日誌 (j で戻る / q で終了)", text, 0)?;
            } else {
                show_journal = false;
                let state = if !w.started {
                    "  (記録待ち)"
                } else if w.stale_rules {
                    "  ※古いルールの記録 (再現できない)"
                } else if w.desync {
                    "  ※再現がずれている"
                } else {
                    ""
                };
                let diary = if w.journals.is_empty() {
                    ""
                } else {
                    "  (j で日誌)"
                };
                let footer = format!("観戦中: {path}{state}{diary}  (i で持ち物 / q で終了)");
                let inv_lines;
                let overlay = if show_inv {
                    inv_lines = w.game.inventory_lines();
                    Some(OverlayView {
                        title: "持ち物",
                        empty: "(なし)",
                        footer: "(Esc か i で閉じる)",
                        items: &inv_lines,
                        scroll: inv_scroll,
                    })
                } else {
                    None
                };
                draw_scene(
                    &mut out,
                    &w.game,
                    Some(&w.thoughts),
                    &footer,
                    false,
                    overlay.as_ref(),
                )?;
            }
            dirty = false;
        }
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let ctrl_c =
                        k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c');
                    // 持ち物を開いているときは、方向キーなどで一覧をスクロールする
                    if show_inv {
                        let total = w.game.inventory_lines().len();
                        if let Some(next) = scrolled(inv_scroll, total, OVERLAY_ROWS, k.code) {
                            inv_scroll = next;
                            dirty = true;
                            continue;
                        }
                    }
                    // Esc は、持ち物を開いているときはそれを閉じるだけ
                    if k.code == KeyCode::Esc && show_inv {
                        show_inv = false;
                        dirty = true;
                        continue;
                    }
                    if ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                        break;
                    }
                    if is_modified(k.modifiers) {
                        continue;
                    }
                    if k.code == KeyCode::Char('i') {
                        show_inv = !show_inv;
                        inv_scroll = 0;
                        dirty = true;
                    }
                    if k.code == KeyCode::Char('j') && !w.journals.is_empty() {
                        show_journal = !show_journal;
                        dirty = true;
                    }
                }
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        }
    }
    Ok(())
}

/// Ctrl か Alt だけが付いたキーか。Ctrl+Alt は AltGr の配列があるので、ふつうの文字として扱う。
fn is_modified(m: KeyModifiers) -> bool {
    m.contains(KeyModifiers::CONTROL) != m.contains(KeyModifiers::ALT)
}

fn random_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--journal") {
        let path = args
            .get(i + 1)
            .map(String::as_str)
            .unwrap_or("grave-record.jsonl");
        return run_journal(path);
    }
    if let Some(i) = args.iter().position(|a| a == "--watch") {
        let path = args
            .get(i + 1)
            .map(String::as_str)
            .unwrap_or("grave-record.jsonl");
        return run_watch(path);
    }
    let seed = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(random_seed);
    let mut app = App::new(seed);

    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    while !app.quit {
        app.draw(&mut out)?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                break;
            }
            if is_modified(k.modifiers) {
                // Ctrl+L などを、修飾なしの l などとして動かさない。コマンド行の編集キーだけ働く
                if let (Mode::Command, true, KeyCode::Char(c)) = (
                    app.mode,
                    k.modifiers.contains(KeyModifiers::CONTROL),
                    k.code,
                ) {
                    app.on_ctrl_command(c);
                }
                continue;
            }
            match app.mode {
                Mode::Normal => app.on_key_normal(k.code),
                Mode::Command => app.on_key_command(k.code),
            }
        }
    }

    drop(_guard);
    println!("seed: {}", app.game.seed());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            match app.mode {
                Mode::Normal => app.on_key_normal(KeyCode::Char(c)),
                Mode::Command => app.on_key_command(KeyCode::Char(c)),
            }
        }
    }

    #[test]
    fn count_prefix_and_repeat() {
        let mut app = App::new(1);
        let t0 = app.game.turn();
        press(&mut app, "z");
        assert_eq!(app.game.turn(), t0 + 1);
        press(&mut app, "3z");
        assert_eq!(app.game.turn(), t0 + 4);
        press(&mut app, ".");
        assert_eq!(app.game.turn(), t0 + 7);
    }

    #[test]
    fn colon_command_and_map() {
        let mut app = App::new(1);
        let t0 = app.game.turn();
        press(&mut app, ":map w wait");
        app.on_key_command(KeyCode::Enter);
        press(&mut app, "w");
        assert_eq!(app.game.turn(), t0 + 1);
        press(&mut app, ":wait");
        app.on_key_command(KeyCode::Enter);
        assert_eq!(app.game.turn(), t0 + 2);
    }

    #[test]
    fn backtick_opens_the_command_line_like_colon() {
        let mut app = App::new(1);
        let t0 = app.game.turn();
        press(&mut app, "`stay 3");
        assert!(app.mode == Mode::Command);
        app.on_key_command(KeyCode::Enter);
        assert!(
            app.game.turn() <= t0 + 3 && app.game.turn() > t0,
            "{}",
            app.game.turn()
        );
    }

    #[test]
    fn prompt_sits_on_the_bottom_row_when_the_terminal_is_tall_enough() {
        // 高さが十分なら最下行。足りなければログの次の行
        assert_eq!(layout(40, false).footer, 39);
        let tight = layout(24, false);
        assert_eq!(tight.footer, tight.log_top + LOG_LINES as u16);
        // 観戦では思考の3行ぶんだけ下がる
        assert_eq!(layout(24, true).footer, tight.footer + 3);
        assert_eq!(LOG_LINES, 3);
    }

    #[test]
    fn inventory_overlay_opens_only_with_the_inventory_command_and_closes_on_a_key() {
        let mut app = App::new(1);
        assert!(!app.overlay);
        press(&mut app, "z");
        assert!(!app.overlay, "ふつうのコマンドでは出ない");
        press(&mut app, "i");
        assert!(app.overlay);
        // 閉じるキーはそれだけで終わる（移動しない）
        let t = app.game.turn();
        app.on_key_normal(KeyCode::Esc);
        assert!(!app.overlay);
        assert_eq!(app.game.turn(), t);
        // ほかのキーは閉じたうえで働く
        press(&mut app, "i");
        press(&mut app, "z");
        assert!(!app.overlay);
        assert_eq!(app.game.turn(), t + 1);
        // :inventory でも出る
        press(&mut app, ":inventory");
        app.on_key_command(KeyCode::Enter);
        assert!(app.overlay);
    }

    #[test]
    fn clip_cols_counts_wide_characters_as_two() {
        assert_eq!(clip_cols("abcdef", 4), "abcd");
        assert_eq!(clip_cols("持ち物です", 6), "持ち物");
        assert_eq!(clip_cols("持ち物です", 7), "持ち物");
        assert_eq!(clip_cols("短い", 20), "短い");
    }

    #[test]
    fn a_frame_does_not_clear_the_whole_screen() {
        // 全画面を消すとちらつく。行ごとにその場で描き直し、更新を保留で囲む
        let app = App::new(1);
        let mut frame = Vec::new();
        render_scene(&mut frame, &app.game, None, "footer", false, None).unwrap();
        let text = String::from_utf8_lossy(&frame);
        assert!(!text.contains("\x1b[2J"), "全画面クリアが残っている");
        assert!(
            text.starts_with("\x1b[?2026h"),
            "{:?}",
            &text[..text.len().min(20)]
        );
        assert!(text.ends_with("\x1b[?2026l"));
        // 行末まで消す指示が、マップの各行に入っている
        assert!(text.matches("\x1b[K").count() >= H as usize);
    }

    #[test]
    fn q_opens_the_command_line_with_quaff() {
        let mut app = App::new(1);
        press(&mut app, "q");
        assert!(app.mode == Mode::Command);
        assert_eq!(app.cmdline, "quaff ");
        press(&mut app, "a");
        app.on_key_command(KeyCode::Enter);
        // 持ち物がないので失敗するが、コマンドとして実行される
        assert!(app.status.contains("持ち物 a はない"), "{}", app.status);
        assert!(app.mode == Mode::Normal);
    }

    #[test]
    fn wrap_counts_wide_characters_as_two_columns() {
        let lines = wrap("あいうえお\nabcdefghij", 6);
        assert_eq!(lines, vec!["あいう", "えお", "abcdef", "ghij"]);
    }

    #[test]
    fn new_restarts_with_the_given_seed() {
        let mut app = App::new(1);
        press(&mut app, "zzz");
        press(&mut app, ":new 5");
        app.on_key_command(KeyCode::Enter);
        assert_eq!(app.game.seed(), 5);
        assert_eq!(app.game.turn(), 0);
    }

    #[test]
    fn tab_completes_unique_prefix() {
        let mut app = App::new(1);
        press(&mut app, ":expl");
        app.on_key_command(KeyCode::Tab);
        assert_eq!(app.cmdline, "explore ");
    }

    #[test]
    fn keypad_diagonal_movement() {
        let mut app = App::new(1);
        let t0 = app.game.turn();
        app.on_key_normal(KeyCode::KeypadBegin);
        assert_eq!(app.game.turn(), t0 + 1);
        assert_eq!(app.last.as_ref().unwrap().0, "wait");

        app.on_key_normal(KeyCode::Home);
        assert_eq!(app.last.as_ref().unwrap().0, "move northwest");

        app.on_key_normal(KeyCode::PageUp);
        assert_eq!(app.last.as_ref().unwrap().0, "move northeast");

        app.on_key_normal(KeyCode::End);
        assert_eq!(app.last.as_ref().unwrap().0, "move southwest");

        app.on_key_normal(KeyCode::PageDown);
        assert_eq!(app.last.as_ref().unwrap().0, "move southeast");

        // カウント指定との連携 (例: 2歩移動)
        press(&mut app, "2");
        app.on_key_normal(KeyCode::Home);
        assert_eq!(app.last.as_ref().unwrap().1, 2);
    }

    #[test]
    fn new_game_command_restarts_like_new() {
        let mut app = App::new(1);
        press(&mut app, "zzz");
        press(&mut app, ":new_game 7");
        app.on_key_command(KeyCode::Enter);
        assert_eq!(app.game.seed(), 7);
        assert_eq!(app.game.turn(), 0);
    }

    #[test]
    fn quit_exits_but_q_does_not_exit_and_runs_quaff() {
        let mut app = App::new(1);
        // :q だけ打った場合、終了せず quaff の「何を？」が出る
        press(&mut app, ":q");
        app.on_key_command(KeyCode::Enter);
        assert!(!app.quit);
        assert_eq!(app.cmdline, "quaff ");
        app.on_key_command(KeyCode::Esc);

        // :q a も終了せず quaff a として処理される
        press(&mut app, ":q a");
        app.on_key_command(KeyCode::Enter);
        assert!(!app.quit);
        assert!(app.status.contains("持ち物 a はない"), "{}", app.status);

        // :quit で初めて終了する
        press(&mut app, ":quit");
        app.on_key_command(KeyCode::Enter);
        assert!(app.quit);
    }

    #[test]
    fn item_commands_show_candidate_prompts() {
        let app = App::new(1);
        // read / r
        let p_r = prompt_for_cmdline(&app.game, "r ").expect("r should prompt");
        assert_eq!(p_r.title, "どれを読む？ (巻物)");
        let p_read = prompt_for_cmdline(&app.game, "read ").expect("read should prompt");
        assert_eq!(p_read.title, "どれを読む？ (巻物)");

        // quaff / q
        let p_q = prompt_for_cmdline(&app.game, "q ").expect("q should prompt");
        assert_eq!(p_q.title, "どれを飲む？ (薬)");

        // eat / e
        let p_e = prompt_for_cmdline(&app.game, "e ").expect("e should prompt");
        assert_eq!(p_e.title, "どれを食べる？");

        // equip / w
        let p_w = prompt_for_cmdline(&app.game, "w ").expect("w should prompt");
        assert_eq!(p_w.title, "どれを装備する？");

        // unequip / remove
        let p_rm = prompt_for_cmdline(&app.game, "remove ").expect("remove should prompt");
        assert_eq!(p_rm.title, "どれをはずす？");

        // コマンド名だけの間は出さない
        assert!(prompt_for_cmdline(&app.game, "e").is_none());

        // 既に複数引数がある場合はプロンプトを表示しない
        assert!(prompt_for_cmdline(&app.game, "read a extra").is_none());
    }

    #[test]
    fn single_key_opens_item_command_line_promptly() {
        let mut app = App::new(1);
        // 'r' を押すとコマンドラインが 'read ' で開く
        press(&mut app, "r");
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.cmdline, "read ");

        // Esc でキャンセル
        app.on_key_command(KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);

        // 'e' を押すとコマンドラインが 'eat ' で開く
        press(&mut app, "e");
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.cmdline, "eat ");

        app.on_key_command(KeyCode::Esc);

        // 'w' を押すとコマンドラインが 'equip ' で開く
        press(&mut app, "w");
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.cmdline, "equip ");

        app.on_key_command(KeyCode::Esc);

        // 'R' を押すとコマンドラインが 'unequip ' で開く
        press(&mut app, "R");
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.cmdline, "unequip ");
    }

    #[test]
    fn enter_on_a_bare_item_command_opens_the_prompt_instead_of_failing() {
        let mut app = App::new(1);
        press(&mut app, ":e");
        app.on_key_command(KeyCode::Enter);
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.cmdline, "eat ");
        // 続けて文字を打って Enter で実行される
        press(&mut app, "b");
        app.on_key_command(KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.contains("持ち物 b はない"), "{}", app.status);
    }

    #[test]
    fn help_is_a_full_screen_that_scrolls_and_closes() {
        let mut app = App::new(1);
        press(&mut app, ":help");
        app.on_key_command(KeyCode::Enter);
        assert_eq!(app.help, Some(0));
        // 閉じるキー以外ではゲームは進まず、スクロールだけする
        let t = app.game.turn();
        app.on_key_normal(KeyCode::Down);
        app.on_key_normal(KeyCode::Char('z'));
        assert_eq!(app.game.turn(), t);
        assert!(app.help.is_some());
        app.on_key_normal(KeyCode::Esc);
        assert_eq!(app.help, None);
        // 本文には、キー操作の節とコマンド一覧の両方が入る
        let text = help_text();
        assert!(text.contains("キー操作") && text.contains("disarm"));
    }

    #[test]
    fn scrolled_clamps_and_ignores_other_keys() {
        // 20行のうち5行が見える
        assert_eq!(scrolled(0, 20, 5, KeyCode::Up), Some(0));
        assert_eq!(scrolled(0, 20, 5, KeyCode::Down), Some(1));
        assert_eq!(scrolled(0, 20, 5, KeyCode::PageDown), Some(4));
        assert_eq!(scrolled(14, 20, 5, KeyCode::Down), Some(15));
        assert_eq!(scrolled(15, 20, 5, KeyCode::Down), Some(15));
        assert_eq!(scrolled(7, 20, 5, KeyCode::End), Some(15));
        assert_eq!(scrolled(7, 20, 5, KeyCode::Home), Some(0));
        // 1画面に収まるなら動かない
        assert_eq!(scrolled(0, 3, 5, KeyCode::Down), Some(0));
        assert_eq!(scrolled(0, 20, 5, KeyCode::Char('z')), None);
    }

    #[test]
    fn inventory_overlay_scrolls_with_arrow_keys_instead_of_closing() {
        let mut app = App::new(1);
        press(&mut app, "i");
        assert!(app.overlay);
        // 持ち物が少ないと動かないが、閉じもしない
        let t = app.game.turn();
        app.on_key_normal(KeyCode::Down);
        assert!(app.overlay);
        assert_eq!(app.game.turn(), t);
        app.on_key_normal(KeyCode::Esc);
        assert!(!app.overlay);
    }

    #[test]
    fn ctrl_or_alt_alone_counts_as_modified_but_altgr_does_not() {
        assert!(is_modified(KeyModifiers::CONTROL));
        assert!(is_modified(KeyModifiers::ALT));
        assert!(!is_modified(KeyModifiers::NONE));
        assert!(!is_modified(KeyModifiers::SHIFT));
        assert!(!is_modified(KeyModifiers::CONTROL | KeyModifiers::ALT));
    }

    #[test]
    fn ctrl_w_deletes_a_word_and_ctrl_u_the_whole_line() {
        let mut app = App::new(1);
        press(&mut app, ":zap c east");
        app.on_ctrl_command('w');
        assert_eq!(app.cmdline, "zap c ");
        app.on_ctrl_command('w');
        assert_eq!(app.cmdline, "zap ");
        app.on_ctrl_command('W');
        assert_eq!(app.cmdline, "");
        assert!(app.mode == Mode::Command, "空になっても閉じない");
        press(&mut app, "stay 3");
        app.on_ctrl_command('u');
        assert_eq!(app.cmdline, "");
        assert!(app.mode == Mode::Command);
    }

    #[test]
    fn ctrl_h_is_backspace_and_other_ctrl_keys_do_nothing() {
        let mut app = App::new(1);
        press(&mut app, ":wait");
        app.on_ctrl_command('h');
        assert_eq!(app.cmdline, "wai");
        app.on_ctrl_command('l');
        assert_eq!(app.cmdline, "wai");
        // 何も入っていないときの Ctrl+H は Backspace と同じく閉じる
        app.cmdline.clear();
        app.on_ctrl_command('h');
        assert!(app.mode == Mode::Normal);
    }
}
