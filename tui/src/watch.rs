use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use grave_core::record::{Event, RULES_VERSION};
use grave_core::Game;

const MAX_THOUGHTS: usize = 50;

/// JSONL 記録を追いかけ、同じゲームをローカルで再現する。
pub struct Watcher {
    path: String,
    offset: u64,
    partial: Vec<u8>,
    pub game: Game,
    /// (その時点のターン, 理由)
    pub thoughts: Vec<(u32, String)>,
    /// エージェントが書いた日誌（このゲームぶん）
    pub journals: Vec<String>,
    /// 記録の状態と再現結果が食い違った
    pub desync: bool,
    /// 記録を1件以上読んだ
    pub started: bool,
    /// 今のゲームの記録が、今とは違うルール（RULES_VERSION）で録られている
    pub stale_rules: bool,
}

impl Watcher {
    pub fn new(path: &str) -> Watcher {
        Watcher {
            path: path.to_string(),
            offset: 0,
            partial: Vec::new(),
            game: Game::new(1),
            thoughts: Vec::new(),
            journals: Vec::new(),
            desync: false,
            started: false,
            stale_rules: false,
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
            self.journals.clear();
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
            Ok(Event::NewGame { seed, rules }) => {
                self.game = Game::new(seed);
                self.stale_rules = rules != Some(RULES_VERSION);
                self.thoughts.clear();
                self.journals.clear();
                self.desync = false;
                self.started = true;
            }
            Ok(Event::Command {
                command,
                thought,
                depth,
                turn,
                hp,
                ..
            }) => {
                self.started = true;
                self.game.run(&command);
                if (self.game.depth(), self.game.turn()) != (depth, turn)
                    || hp.is_some_and(|h| h != self.game.hp())
                {
                    self.desync = true;
                }
                if let Some(t) = thought {
                    self.thoughts.push((turn, t));
                    if self.thoughts.len() > MAX_THOUGHTS {
                        self.thoughts.remove(0);
                    }
                }
            }
            Ok(Event::Journal { text }) => {
                self.started = true;
                self.journals.push(text);
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
        let dir = std::env::temp_dir().join(format!("grave-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        let _ = std::fs::remove_file(&path);

        let mut live = Game::new(5);
        let mut lines = vec![Event::new_game(5).to_line()];
        for o in live.run_script("wait; wait; wait") {
            lines.push(Event::from_outcome(&o, Some("様子を見よう")).to_line());
        }
        assert_eq!(lines.len(), 4);

        let mut w = Watcher::new(path.to_str().unwrap());
        assert!(!w.poll().unwrap()); // ファイルがまだない

        let mut f = std::fs::File::create(&path).unwrap();
        // 最初の2行を書き、3行目は日本語の文字の途中（UTF-8 のバイト境界の途中）まで書く
        writeln!(f, "{}", lines[0]).unwrap();
        writeln!(f, "{}", lines[1]).unwrap();
        let bytes = lines[2].as_bytes();
        let cut = (bytes.len() / 2..bytes.len())
            .find(|&i| !lines[2].is_char_boundary(i))
            .expect("日本語を含むので必ず見つかる");
        f.write_all(&bytes[..cut]).unwrap();
        assert!(w.poll().unwrap());
        assert_eq!(w.thoughts.len(), 1); // 途中の行はまだ反映されない

        f.write_all(&bytes[cut..]).unwrap();
        f.write_all(b"\n").unwrap();
        writeln!(f, "{}", lines[3]).unwrap();
        assert!(w.poll().unwrap());
        assert_eq!(w.thoughts.len(), 3);
        assert!(!w.desync);
        assert_eq!(w.game.observe_text(100), live.observe_text(100));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collects_journals_and_resets_on_new_game() {
        let dir = std::env::temp_dir().join(format!("grave-watch-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "{}", Event::new_game(1).to_line()).unwrap();
        writeln!(
            f,
            "{}",
            Event::Journal {
                text: "一日目の日誌".into()
            }
            .to_line()
        )
        .unwrap();

        let mut w = Watcher::new(path.to_str().unwrap());
        assert!(w.poll().unwrap());
        assert_eq!(w.journals, vec!["一日目の日誌".to_string()]);

        writeln!(f, "{}", Event::new_game(2).to_line()).unwrap();
        assert!(w.poll().unwrap());
        assert!(w.journals.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flags_records_made_under_other_rules() {
        let dir = std::env::temp_dir().join(format!("grave-watch-rules-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        // rules のない古い記録
        std::fs::write(&path, "{\"kind\":\"new_game\",\"seed\":1}\n").unwrap();
        let mut w = Watcher::new(path.to_str().unwrap());
        w.poll().unwrap();
        assert!(w.started && w.stale_rules);
        // 今のルールで録った記録
        std::fs::write(&path, format!("{}\n", Event::new_game(1).to_line())).unwrap();
        let mut w = Watcher::new(path.to_str().unwrap());
        w.poll().unwrap();
        assert!(w.started && !w.stale_rules);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
