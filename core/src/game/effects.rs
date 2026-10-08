//! アイテムを使う（飲む・読む・食べる）ことと、その効果。
//!
//! 薬と巻物の効果は `item::ITEMS` の表（`Effect` の並び）で決まり、ここはその実行だけを持つ。

use super::*;

impl Game {
    /// 薬を飲む・食べる・巻物を読む。種類が合わないものは失敗（ターン消費なし）。
    /// (成功か, メッセージ, 1ターン消費するか)
    pub(super) fn consume(&mut self, letter: char, target: Option<char>, how: Consume) -> (bool, String, bool) {
        let Some(si) = self.inventory.iter().position(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let kind = self.inventory[si].kind;
        if matches!(how, Consume::Read) && kind.is_scroll() && self.status.has(Status::Blind) {
            return (false, "目が見えなくて、巻物が読めない。".to_string(), false);
        }
        let fits = match how {
            Consume::Quaff => kind.is_potion(),
            Consume::Eat => kind.is_food() || kind.is_mushroom(),
            Consume::Read => kind.is_scroll(),
        };
        if !fits {
            let (is, instead) = match kind.class() {
                Class::Potion => ("薬", "quaff"),
                Class::Scroll => ("巻物", "read"),
                Class::Weapon | Class::Armor => ("装備品", "equip"),
                Class::Food | Class::Mushroom => ("食べ物", "eat"),
                Class::Wand => ("杖", "zap"),
            };
            return (
                false,
                format!("{letter} は{is}だ。{} ではなく {instead} を使う。", how.command()),
                false,
            );
        }
        let k = kind.index();
        let was_known = self.known[k];
        let verb = match kind.class() {
            Class::Potion => "飲んだ",
            Class::Scroll => "読んだ",
            _ => "食べた",
        };
        // 使った物の名前。正体を知らなければ、見た目と正体を並べて知らせる（薬・巻物・キノコ共通）
        let prefix = if was_known {
            format!("{}を{verb}。", kind.true_name())
        } else {
            format!("{}を{verb}。これは{}だった！", self.looks[k], kind.true_name())
        };

        let body = match kind.class() {
            Class::Potion | Class::Scroll => match self.run_effects(kind, letter, target, was_known) {
                Ok(body) => body,
                Err(msg) => return (false, msg, false),
            },
            _ => self.eat_effect(kind),
        };

        self.known[k] = true;
        self.inventory[si].count -= 1;
        if self.inventory[si].count == 0 {
            self.inventory.remove(si);
        }
        (true, format!("{prefix} {body}"), true)
    }

    /// 食べ物とキノコの効果。
    fn eat_effect(&mut self, kind: ItemKind) -> String {
        match kind {
            ItemKind::Bread => {
                // 6個に1個くらいは腐っている
                if self.rng.range(0, 6) == 0 {
                    self.food = (self.food + 30).min(MAX_FOOD);
                    if self.try_poison(6) {
                        "腐っていた！ 毒を受けた。".to_string()
                    } else {
                        "腐っていた！ だが毒は守りに阻まれた。".to_string()
                    }
                } else {
                    self.gain_food(kind.nutrition())
                }
            }
            ItemKind::EdibleShroom => format!("おいしい。{}", self.gain_food(kind.nutrition())),
            ItemKind::PoisonShroom => {
                let hurt = if self.try_poison(8) { "毒を受けた。" } else { "毒は守りに阻まれた。" };
                format!("{hurt}{}", self.gain_food(kind.nutrition()))
            }
            ItemKind::VigorShroom => {
                let gained = (self.max_hp - self.hp).min(8);
                self.hp += gained;
                let food = self.gain_food(kind.nutrition());
                format!(
                    "力が湧いてきた。HPが{gained}回復した。(HP {}/{}) {food}",
                    self.hp, self.max_hp
                )
            }
            _ => self.gain_food(kind.nutrition()),
        }
    }

    /// 薬・巻物の効果を表の順に実行する。識別できる物がないなど、使えないときは Err（消費しない）。
    fn run_effects(
        &mut self,
        kind: ItemKind,
        letter: char,
        target: Option<char>,
        was_known: bool,
    ) -> Result<String, String> {
        let mut parts: Vec<String> = Vec::new();
        for &e in kind.def().effects {
            let text = match e {
                Effect::Identify => self.identify_effect(letter, target, was_known)?,
                e => self.apply_effect(e),
            };
            if !text.is_empty() {
                parts.push(text);
            }
            if self.dead {
                break;
            }
        }
        if !parts.is_empty() {
            return Ok(parts.join(" "));
        }
        // 治すものがなかった薬は、そう伝える
        Ok(match kind.def().effects.iter().find_map(|e| match e {
            Effect::Cure(st) => Some(*st),
            _ => None,
        }) {
            Some(st) => format!("{}にはかかっていなかった。", st.name()),
            None => "何も起こらなかった。".to_string(),
        })
    }

    /// 効果1つを実行して、その結果の説明を返す（何も起きなければ空）。
    fn apply_effect(&mut self, e: Effect) -> String {
        match e {
            Effect::Heal { amount, max_up } => {
                let gained = (self.max_hp - self.hp).min(amount);
                self.hp += gained;
                if amount > gained && max_up > 0 {
                    self.max_hp += max_up;
                    self.hp += max_up;
                    format!("HPが{gained}回復し、最大HPが{max_up}増えた。(HP {}/{})", self.hp, self.max_hp)
                } else {
                    format!("HPが{gained}回復した。(HP {}/{})", self.hp, self.max_hp)
                }
            }
            Effect::Damage(n) => {
                self.hp -= n;
                let mut s = format!("{n}のダメージを受けた。(HP {}/{})", self.hp.max(0), self.max_hp);
                if self.hp <= 0 {
                    self.dead = true;
                    s.push_str(" 毒で力尽きた…。ゲームオーバー。");
                }
                s
            }
            Effect::LoseStrength(n) => {
                let before = self.strength;
                self.strength = (self.strength - n).max(MIN_STRENGTH);
                if self.strength < before {
                    format!("力が抜けた。(腕力 {}/{})", self.strength, self.max_strength)
                } else {
                    String::new()
                }
            }
            Effect::GainStrength(n) => {
                self.max_strength += n;
                self.strength += n;
                format!("力がみなぎる。(腕力 {}/{})", self.strength, self.max_strength)
            }
            Effect::RestoreStrength => {
                if self.strength < self.max_strength {
                    self.strength = self.max_strength;
                    format!("力が戻った。(腕力 {}/{})", self.strength, self.max_strength)
                } else {
                    String::new()
                }
            }
            Effect::LevelUp => {
                let need = self.xp_for_next() - self.xp;
                self.gain_xp(need);
                "体の奥から力が湧き上がる！".to_string()
            }
            Effect::Cure(st) => {
                if self.cure(st) {
                    st.def().end.to_string()
                } else {
                    String::new()
                }
            }
            Effect::SelfStatus(st, turns) => self.inflict(st, turns),
            Effect::MonstersStatus { status, turns, radius } => self.afflict_monsters(status, turns, radius),
            Effect::DetectMonsters => self.detect_monsters(),
            Effect::DetectItems => self.detect_items(),
            Effect::MagicMap => {
                self.map.reveal_all();
                "このフロアの地図が頭に浮かんだ。".to_string()
            }
            Effect::Teleport => {
                self.teleport_player();
                "景色が一変した。".to_string()
            }
            Effect::Identify => String::new(), // run_effects が扱う
            Effect::EnchantWeapon => self.enchant_gear(true),
            Effect::EnchantArmor => self.enchant_gear(false),
            Effect::RemoveCurse => {
                let mut n = 0;
                for g in self.inventory.iter_mut().filter_map(|s| s.gear.as_mut()) {
                    if g.is_sticky() {
                        g.freed = true;
                        n += 1;
                    }
                }
                if n > 0 {
                    format!("{n}個の装備にかかった呪いの束縛が解けた。(はずせるようになったが、欠点は残る)")
                } else {
                    String::new()
                }
            }
            Effect::ProtectArmor => match self.armor_gear_mut() {
                Some(g) => {
                    g.protected = true;
                    format!("{}が薄い光に包まれた。もう錆びない。", g.name())
                }
                None => "鎧を着ていないので、何も起こらず消えた。".to_string(),
            },
            Effect::CreateMonster => {
                match self.random_free_step(self.pos) {
                    Some(p) => {
                        let kind = self.pick_monster_kind();
                        let i = self.add_monster(kind, p);
                        format!("すぐそばに{}が現れた！", self.foe_name(i))
                    }
                    None => String::new(),
                }
            }
            Effect::Aggravate => {
                let n = self.monsters.len();
                for i in 0..n {
                    self.inflict_monster(i, Status::Enraged, 80);
                }
                if n > 0 {
                    format!("不気味な叫びが響き、この階の{n}体の敵が怒り狂った！")
                } else {
                    "不気味な叫びが響いたが、この階に敵はいない。".to_string()
                }
            }
        }
    }

    /// 識別の巻物。対象の文字を指定できる。
    fn identify_effect(&mut self, letter: char, target: Option<char>, was_known: bool) -> Result<String, String> {
        let ti = match target {
            Some(t) if t == letter => return Err("その巻物自身は対象にできない。".to_string()),
            Some(t) => match self.inventory.iter().position(|s| s.letter == t) {
                Some(i) if !self.needs_identify(&self.inventory[i]) => return Err(format!("{t} はすでに識別済みだ。")),
                Some(i) => Some(i),
                None => return Err(format!("持ち物 {t} はない。")),
            },
            None => self
                .inventory
                .iter()
                .position(|s| s.letter != letter && self.needs_identify(s)),
        };
        match ti {
            Some(i) if self.inventory[i].gear.is_some() => {
                let letter = self.inventory[i].letter;
                let (old, text) = self.identify_gear(letter);
                Ok(format!("{old}の正体が分かった。{text}"))
            }
            Some(i) => {
                let tk = self.inventory[i].kind;
                let old = self.looks[tk.index()];
                self.known[tk.index()] = true;
                Ok(format!("{old}は{}だと分かった。", tk.true_name()))
            }
            None if was_known => Err("識別できるものがない。".to_string()),
            None => Ok("何も起こらなかった。".to_string()),
        }
    }

    fn armor_gear_mut(&mut self) -> Option<&mut Gear> {
        let l = self.armor?;
        self.inventory.iter_mut().find(|s| s.letter == l)?.gear.as_mut()
    }

    /// 装備中の武器か防具を +1 強化する。
    fn enchant_gear(&mut self, weapon: bool) -> String {
        let slot = if weapon { self.weapon } else { self.armor };
        let Some(letter) = slot else {
            return format!("{}を装備していないので、何も起こらず消えた。", if weapon { "武器" } else { "防具" });
        };
        let g = self
            .inventory
            .iter_mut()
            .find(|s| s.letter == letter)
            .and_then(|s| s.gear.as_mut())
            .expect("装備している物は持ち物にある");
        g.enchant = (g.enchant + 1).min(MAX_ENCHANT);
        let stats = if g.identified { g.stats_text() } else { g.guess_text() };
        format!("{}が青く光った。強化{:+}。({stats})", g.name(), g.enchant)
    }

    /// 敵に殴られて鎧が錆びる。保護されていれば防げる。
    pub(super) fn corrode_armor(&mut self) {
        let Some(g) = self.armor_gear_mut() else { return };
        if g.protected {
            let name = g.name();
            self.note(&format!("{name}は守られていて、錆びなかった。"));
        } else if g.enchant > -MAX_ENCHANT {
            g.enchant -= 1;
            let name = g.name();
            self.note(&format!("{name}が錆びた！ 防御が1下がった。"));
        }
    }

    /// 見えている敵のうち、`radius` マス以内のすべてに状態を付ける。
    fn afflict_monsters(&mut self, status: Status, turns: u32, radius: i32) -> String {
        let idxs: Vec<usize> = self
            .visible_monster_indices()
            .into_iter()
            .filter(|&i| {
                let p = self.monsters[i].pos;
                (p.0 - self.pos.0).abs().max((p.1 - self.pos.1).abs()) <= radius
            })
            .collect();
        let n = idxs.iter().filter(|&&i| self.inflict_monster(i, status, turns)).count();
        if n == 0 {
            "周りには効く相手がいなかった。".to_string()
        } else {
            format!("{n}体の敵が{}状態になった。", status.name())
        }
    }

    /// ランダムな場所へ移動する。
    pub(super) fn teleport_player(&mut self) {
        let old = self.pos;
        for _ in 0..200 {
            let x = self.rng.range(1, W - 1);
            let y = self.rng.range(1, H - 1);
            if self.map.tile(x, y) == Tile::Floor && (x, y) != old && self.monster_at((x, y)).is_none() {
                self.pos = (x, y);
                break;
            }
        }
        self.refresh_fov();
    }

    /// 階じゅうの敵の位置を地図に出す（次のコマンドまで）。
    fn detect_monsters(&mut self) -> String {
        if self.monsters.is_empty() {
            return "この階に敵の気配はない。".to_string();
        }
        self.detect_marks = self.monsters.iter().map(|m| (m.pos, m.glyph)).collect();
        let list: Vec<String> = self
            .monsters
            .iter()
            .map(|m| format!("{}が{}", m.name, rel_text(self.pos, m.pos)))
            .collect();
        format!("{}体の敵の気配を感じた: {}。", self.monsters.len(), list.join("、"))
    }

    /// 階じゅうの床の物の場所を知る（地図に残る）。
    fn detect_items(&mut self) -> String {
        let spots: Vec<((i32, i32), String, char)> = self
            .floor_items
            .iter()
            .map(|f| (f.pos, self.item_name(&f.item), f.item.kind().glyph()))
            .chain(self.amulet.map(|p| (p, "魔除けのアミュレット".to_string(), ',')))
            .collect();
        if spots.is_empty() {
            return "この階に落ちている物はない。".to_string();
        }
        for (p, _, _) in &spots {
            self.map.mark_seen(p.0, p.1);
        }
        let list: Vec<String> = spots
            .iter()
            .take(10)
            .map(|(p, name, glyph)| format!("{glyph} {name}が{}", rel_text(self.pos, *p)))
            .collect();
        let more = if spots.len() > 10 { format!(" ほか{}個", spots.len() - 10) } else { String::new() };
        format!("{}個の物の気配を感じた: {}{more}。", spots.len(), list.join("、"))
    }
}
