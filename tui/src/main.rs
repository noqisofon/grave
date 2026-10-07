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
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
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
                self.status = "キー: hjklyubn 移動(敵に向かうと攻撃) / q 使う(q の後に文字) / i 持ち物 / > 降りる / _ 階段へ / x 探索 / z 待つ / ; 見る / 数字+キーで反復 / . 繰り返し / :map :unmap :new :q".to_string();
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
        match code {
            KeyCode::Char(':') => {
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
        draw_scene(out, &self.game, None, &footer, self.mode == Mode::Command)
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// マップ・ログ・（観戦時は）思考・最下行を描く。
fn draw_scene(
    out: &mut impl Write,
    game: &Game,
    thoughts: Option<&[(u32, String)]>,
    footer: &str,
    cursor: bool,
) -> io::Result<()> {
    // 全角文字は2桁ぶん使うので、文字数の上限は桁数の半分にしておく
    let cap = terminal::size().map(|(c, _)| c as usize / 2).unwrap_or(40);
    queue!(out, Clear(ClearType::All), MoveTo(0, 0))?;
    queue!(
        out,
        Print(format!(
            "地下{}階  ターン{}  HP {}/{}{}",
            game.depth(),
            game.turn(),
            game.hp().max(0),
            game.max_hp(),
            if game.is_dead() { "  ★ゲームオーバー (:new で再開)" } else { "" }
        ))
    )?;
    for y in 0..H {
        queue!(out, MoveTo(0, (y + 1) as u16))?;
        for x in 0..W {
            let c = game.cell(x, y);
            let color = if c.ch == '@' {
                Color::Yellow
            } else if c.ch == '>' {
                Color::Green
            } else if c.ch == '!' {
                Color::Magenta
            } else if c.ch == '?' {
                Color::Cyan
            } else if c.ch == ')' || c.ch == '[' {
                Color::Blue
            } else if c.ch.is_ascii_alphabetic() {
                Color::Red
            } else if c.visible {
                Color::White
            } else {
                Color::DarkGrey
            };
            queue!(out, SetForegroundColor(color), Print(c.ch))?;
        }
        queue!(out, ResetColor)?;
    }
    let log_top = (H + 2) as u16;
    let log = game.log();
    let start = log.len().saturating_sub(4);
    for (i, e) in log[start..].iter().enumerate() {
        queue!(
            out,
            MoveTo(0, log_top + i as u16),
            Print(clip(&format!("[{}] {}", e.turn, e.text), cap))
        )?;
    }
    let inv = game.inventory_lines();
    let inv_text = if inv.is_empty() {
        "持ち物: (なし)".to_string()
    } else {
        format!("持ち物: {}", inv.join("  "))
    };
    queue!(out, MoveTo(0, log_top + 4), Print(clip(&inv_text, cap)))?;
    let mut bottom = log_top + 5;
    if let Some(ts) = thoughts {
        let start = ts.len().saturating_sub(3);
        for (i, (turn, t)) in ts[start..].iter().enumerate() {
            queue!(
                out,
                MoveTo(0, log_top + 5 + i as u16),
                SetForegroundColor(Color::Cyan),
                Print(clip(&format!("思[{turn}] {t}"), cap)),
                ResetColor
            )?;
        }
        bottom += 3;
    }
    queue!(out, MoveTo(0, bottom), Print(clip(footer, cap)))?;
    if cursor {
        queue!(out, Show)?;
    } else {
        queue!(out, Hide)?;
    }
    out.flush()
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
                    let footer = format!("観戦中: {path}{state}{diary}  (q で終了)");
                    draw_scene(&mut out, &w.game, Some(&w.thoughts), &footer, false)?;
                }
                dirty = false;
            }
            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        let ctrl_c = k.modifiers.contains(KeyModifiers::CONTROL)
                            && k.code == KeyCode::Char('c');
                        if ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                            break;
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
