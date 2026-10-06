use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use colonrogue_core::record::Event;
use colonrogue_core::Game;

const MAX_THOUGHTS: usize = 50;

/// JSONL 記録を追いかけ、同じゲームをローカルで再現する。
pub struct Watcher {
    path: String,
    offset: u64,
    partial: Vec<u8>,
    pub game: Game,
    /// (その時点のターン, 理由)
    pub thoughts: Vec<(u32, String)>,
    /// 記録の状態と再現結果が食い違った
    pub desync: bool,
    /// 記録を1件以上読んだ
    pub started: bool,
}

impl Watcher {
    pub fn new(path: &str) -> Watcher {
        Watcher {
            path: path.to_string(),
            offset: 0,
            partial: Vec::new(),
            game: Game::new(1),
            thoughts: Vec::new(),
            desync: false,
            started: false,
        }
    }

    /// 新しい行を読んで反映する。何か変わったら true。
    pub fn poll(&mut self) -> io::Result<bool> {
        let mut f = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let mut changed = false;
        let len = f.metadata()?.len();
        if len < self.offset {
            // 作り直された
            self.offset = 0;
            self.partial.clear();
            self.thoughts.clear();
            self.started = false;
            changed = true;
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        if buf.is_empty() {
            return Ok(changed);
        }
        self.offset += buf.len() as u64;
        self.partial.extend_from_slice(&buf);
        while let Some(i) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            self.apply(String::from_utf8_lossy(&line).trim());
            changed = true;
        }
        Ok(changed)
    }

    fn apply(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        match Event::parse(line) {
            Ok(Event::NewGame { seed }) => {
                self.game = Game::new(seed);
                self.thoughts.clear();
                self.desync = false;
                self.started = true;
            }
            Ok(Event::Command {
                command,
                thought,
                depth,
                turn,
                ..
            }) => {
                self.started = true;
                self.game.run(&command);
                if (self.game.depth(), self.game.turn()) != (depth, turn) {
                    self.desync = true;
                }
                if let Some(t) = thought {
                    self.thoughts.push((turn, t));
                    if self.thoughts.len() > MAX_THOUGHTS {
                        self.thoughts.remove(0);
                    }
                }
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn follows_appended_lines_and_partial_writes() {
        let dir = std::env::temp_dir().join(format!("colonrogue-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        let _ = std::fs::remove_file(&path);

        let mut live = Game::new(5);
        let mut lines = vec![Event::NewGame { seed: 5 }.to_line()];
        for o in live.run_script("explore; travel >") {
            lines.push(Event::from_outcome(&o, Some("階段を探そう")).to_line());
        }

        let mut w = Watcher::new(path.to_str().unwrap());
        assert!(!w.poll().unwrap()); // ファイルがまだない

        let mut f = std::fs::File::create(&path).unwrap();
        // 最初の2行を書き、3行目は途中までしか書かない
        writeln!(f, "{}", lines[0]).unwrap();
        writeln!(f, "{}", lines[1]).unwrap();
        let (head, tail) = lines[2].split_at(lines[2].len() / 2);
        write!(f, "{head}").unwrap();
        assert!(w.poll().unwrap());
        assert_eq!(w.thoughts.len(), 1);

        writeln!(f, "{tail}").unwrap();
        assert!(w.poll().unwrap());
        assert!(!w.desync);
        assert_eq!(w.game.observe_text(100), live.observe_text(100));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
