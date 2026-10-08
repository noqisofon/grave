mod keymap;
mod watch;

use std::io::{self, Write};
use std::time::Duration;

use grave_core::map::{H, W};
use grave_core::record::Event as RecEvent;
use grave_core::journal;
use grave_core::{Game, COMMAND_NAMES};
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
use keymap::Keymap;

/// ゲームコマンド以外の、TUI 側で処理する組み込みコマンド。
const BUILTINS: &[&str] = &["map", "unmap", "new", "quit", "help"];

#[derive(PartialEq)]
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
            "q" | "quit" => {
                self.quit = true;
                true
            }
            "help" => {
                self.status = "キー: hjklyubn 移動(敵に向かうと攻撃) / q 使う(q の後に文字) / i 持ち物 / > 降りる / < 登る(アミュレット所持時) / _ 階段へ / x 探索 / z 待つ / ; 見る / 数字+キーで反復 / . 繰り返し / :map :unmap :new :q".to_string();
                true
            }
            "new" => {
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
        if self.overlay {
            self.overlay = false;
            // 閉じるためのキーはそれだけで終わり。ほかのキーは閉じたうえで普通に働く
            if matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i') | KeyCode::Char(' ')) {
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
                        // 例: q → ":use " を開いて、文字を打ってもらう
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
            KeyCode::Esc => self.count = None,
            _ => {}
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
        let footer = match self.mode {
            Mode::Command => format!(":{}", self.cmdline),
            Mode::Normal => {
                let count = self.count.map(|c| c.to_string()).unwrap_or_default();
                format!("{}  {}", self.status, count)
            }
        };
        draw_scene(out, &self.game, None, &footer, self.mode == Mode::Command, self.overlay)
    }
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
    let log_top = (H + 2) as u16;
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
    show_inventory: bool,
) -> io::Result<()> {
    // ちらつき対策: 1フレームぶんを一度に組み立てて、1回の書き込みで送る
    let mut frame: Vec<u8> = Vec::with_capacity(8 * 1024);
    render_scene(&mut frame, game, thoughts, footer, cursor, show_inventory)?;
    out.write_all(&frame)?;
    out.flush()
}

fn render_scene(
    out: &mut Vec<u8>,
    game: &Game,
    thoughts: Option<&[(u32, String)]>,
    footer: &str,
    cursor: bool,
    show_inventory: bool,
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
        queue!(out, MoveTo(0, lay.footer + 1), Clear(ClearType::FromCursorDown))?;
    }
    queue!(out, MoveTo(0, lay.footer), Print(clip_cols(footer, (cols as usize).saturating_sub(1))))?;
    if show_inventory {
        draw_inventory_overlay(out, game, cols)?;
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

/// 持ち物を、マップの右上に重ねて描く。
fn draw_inventory_overlay(out: &mut impl Write, game: &Game, cols: u16) -> io::Result<()> {
    let items = game.inventory_lines();
    let mut lines: Vec<(String, Color)> = vec![("持ち物".to_string(), Color::Cyan)];
    if items.is_empty() {
        lines.push(("(なし)".to_string(), Color::White));
    }
    for l in items.iter().take(H as usize - 2) {
        let color = if l.contains("(装備中)") { Color::Yellow } else { Color::White };
        lines.push((l.clone(), color));
    }
    lines.push(("(Esc か i で閉じる)".to_string(), Color::DarkGrey));
    // マップの右端にそろえる。端末が狭ければ端末の右端まで
    let right = (W as u16).min(cols) as usize;
    let inner = lines.iter().map(|(l, _)| display_width(l)).max().unwrap_or(0);
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

fn draw_text_screen(out: &mut impl Write, title: &str, text: &str) -> io::Result<()> {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let lines = wrap(text, (cols as usize).saturating_sub(2).max(10));
    queue!(
        out,
        Clear(ClearType::All),
        MoveTo(0, 0),
        SetForegroundColor(Color::Cyan),
        Print(title),
        ResetColor
    )?;
    for (i, l) in lines.iter().take((rows as usize).saturating_sub(3)).enumerate() {
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

fn run_watch(path: &str) -> io::Result<()> {
    let mut w = watch::Watcher::new(path);
    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, Hide)?;

    let result = (|| -> io::Result<()> {
        let mut dirty = true;
        let mut show_journal = false;
        let mut show_inv = false;
        loop {
            if w.poll()? {
                dirty = true;
            }
            if dirty {
                if show_journal && !w.journals.is_empty() {
                    let text = w.journals.last().unwrap();
                    draw_text_screen(&mut out, "冒険日誌 (j で戻る / q で終了)", text)?;
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
                    let diary = if w.journals.is_empty() { "" } else { "  (j で日誌)" };
                    let footer = format!("観戦中: {path}{state}{diary}  (i で持ち物 / q で終了)");
                    draw_scene(&mut out, &w.game, Some(&w.thoughts), &footer, false, show_inv)?;
                }
                dirty = false;
            }
            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        let ctrl_c = k.modifiers.contains(KeyModifiers::CONTROL)
                            && k.code == KeyCode::Char('c');
                        // Esc は、持ち物を開いているときはそれを閉じるだけ
                        if k.code == KeyCode::Esc && show_inv {
                            show_inv = false;
                            dirty = true;
                            continue;
                        }
                        if ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                            break;
                        }
                        if k.code == KeyCode::Char('i') {
                            show_inv = !show_inv;
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
    })();

    execute!(out, Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
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

    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, Hide)?;

    let result = (|| -> io::Result<()> {
        while !app.quit {
            app.draw(&mut out)?;
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                    break;
                }
                match app.mode {
                    Mode::Normal => app.on_key_normal(k.code),
                    Mode::Command => app.on_key_command(k.code),
                }
            }
        }
        Ok(())
    })();

    execute!(out, Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    println!("seed: {}", app.game.seed());
    result
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
        assert!(app.game.turn() <= t0 + 3 && app.game.turn() > t0, "{}", app.game.turn());
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
        render_scene(&mut frame, &app.game, None, "footer", false, false).unwrap();
        let text = String::from_utf8_lossy(&frame);
        assert!(!text.contains("\x1b[2J"), "全画面クリアが残っている");
        assert!(text.starts_with("\x1b[?2026h"), "{:?}", &text[..text.len().min(20)]);
        assert!(text.ends_with("\x1b[?2026l"));
        // 行末まで消す指示が、マップの各行に入っている
        assert!(text.matches("\x1b[K").count() >= H as usize);
    }

    #[test]
    fn q_opens_the_command_line_with_use() {
        let mut app = App::new(1);
        press(&mut app, "q");
        assert!(app.mode == Mode::Command);
        assert_eq!(app.cmdline, "use ");
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
}
