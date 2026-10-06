mod keymap;

use std::io::{self, Write};

use colonrogue_core::map::{H, W};
use colonrogue_core::{Game, COMMAND_NAMES};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use keymap::Keymap;

/// ゲームコマンド以外の、TUI 側で処理する組み込みコマンド。
const BUILTINS: &[&str] = &["map", "unmap", "quit", "help"];

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
                self.status = "キー: hjklyubn 移動 / > 降りる / _ 階段へ / x 探索 / z 待つ / ; 見る / 数字+キーで反復 / . 繰り返し / :map :unmap :q".to_string();
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
                        match colonrogue_core::command::parse(&cmd) {
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
                    self.run_with_count(&cmd, n);
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
        queue!(out, Clear(ClearType::All), MoveTo(0, 0))?;
        queue!(
            out,
            Print(format!(
                "地下{}階  ターン{}",
                self.game.depth(),
                self.game.turn()
            ))
        )?;
        for y in 0..H {
            queue!(out, MoveTo(0, (y + 1) as u16))?;
            for x in 0..W {
                let c = self.game.cell(x, y);
                let color = if c.ch == '@' {
                    Color::Yellow
                } else if c.ch == '>' {
                    Color::Green
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
        let log = self.game.log();
        let start = log.len().saturating_sub(4);
        for (i, e) in log[start..].iter().enumerate() {
            queue!(
                out,
                MoveTo(0, log_top + i as u16),
                Print(format!("[{}] {}", e.turn, e.text))
            )?;
        }
        let bottom = log_top + 5;
        queue!(out, MoveTo(0, bottom))?;
        match self.mode {
            Mode::Command => queue!(out, Print(format!(":{}", self.cmdline)), Show)?,
            Mode::Normal => {
                let count = self.count.map(|c| format!("{c}")).unwrap_or_default();
                queue!(out, Hide, Print(format!("{}  {}", self.status, count)))?
            }
        }
        out.flush()
    }
}

fn main() -> io::Result<()> {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
        });
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
    fn tab_completes_unique_prefix() {
        let mut app = App::new(1);
        press(&mut app, ":expl");
        app.on_key_command(KeyCode::Tab);
        assert_eq!(app.cmdline, "explore ");
    }
}
