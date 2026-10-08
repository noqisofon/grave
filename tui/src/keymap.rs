use std::collections::BTreeMap;

/// 通常モードのキー → コロンコマンド文字列。vim の :map と同じ発想で、
/// ゲームロジックはキーを知らない。
pub struct Keymap {
    map: BTreeMap<char, String>,
}

impl Keymap {
    pub fn with_defaults() -> Keymap {
        let defaults = [
            ('h', "move west"),
            ('j', "move south"),
            ('k', "move north"),
            ('l', "move east"),
            ('y', "move northwest"),
            ('u', "move northeast"),
            ('b', "move southwest"),
            ('n', "move southeast"),
            ('>', "descend"),
            ('<', "ascend"),
            // 末尾が空白のものは、コマンド行をその文字列で開く（続きを打つ）
            ('q', "quaff "),
            ('E', "eat "),
            ('R', "read "),
            ('e', "equip "),
            ('r', "unequip "),
            ('d', "drop "),
            (',', "pickup"),
            ('i', "inventory"),
            ('_', "travel >"),
            ('x', "explore"),
            ('z', "wait"),
            (';', "look"),
        ];
        Keymap {
            map: defaults
                .into_iter()
                .map(|(k, v)| (k, v.to_string()))
                .collect(),
        }
    }

    pub fn get(&self, c: char) -> Option<&str> {
        self.map.get(&c).map(String::as_str)
    }

    pub fn set(&mut self, c: char, cmd: String) {
        self.map.insert(c, cmd);
    }

    pub fn unmap(&mut self, c: char) -> bool {
        self.map.remove(&c).is_some()
    }

    pub fn list(&self) -> Vec<(char, &str)> {
        self.map.iter().map(|(k, v)| (*k, v.as_str())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_remap() {
        let mut km = Keymap::with_defaults();
        assert_eq!(km.get('h'), Some("move west"));
        km.set('h', "move east".to_string());
        assert_eq!(km.get('h'), Some("move east"));
        assert!(km.unmap('h'));
        assert_eq!(km.get('h'), None);
        assert!(!km.unmap('h'));
    }

    #[test]
    fn every_default_binding_is_a_valid_command() {
        let km = Keymap::with_defaults();
        for (_, cmd) in km.list() {
            // コマンド行を開くだけの割り当ては、続きを打って初めて完成する
            let cmd = if cmd.ends_with(' ') { format!("{cmd}a") } else { cmd.to_string() };
            assert!(grave_core::command::parse(&cmd).is_ok(), "{cmd}");
        }
    }
}
