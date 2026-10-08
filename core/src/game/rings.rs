//! 指輪と光源。
//!
//! 指輪の毎ターンの効果と光源の燃料の減りは、`tick_equipment` の1か所にまとめてある。
//! 防御・腕力・攻撃などの常に効く値は、`ring_fx` でそのときの装備から集計する。

use super::*;
use crate::item::{RingEffect, OIL_FLASK_FUEL};

/// 身につけている指輪の効果の合計。
#[derive(Default, Clone, Copy, Debug)]
pub(super) struct RingFx {
    pub protection: i32,
    pub strength: i32,
    pub dexterity: i32,
    pub damage: i32,
    pub regeneration: i32,
    pub slow_digestion: i32,
    pub stealth: i32,
    pub searching: i32,
    pub see_invisible: bool,
    pub aggravate: bool,
    pub teleportitis: bool,
}

/// 指輪を身につけて、この間に正体が分かる。
const IDENTIFY_RING_AFTER: u32 = 30;
/// 燃料が残りこれだけになったら知らせる。
const LIGHT_LOW: i32 = 100;
/// テレポート癖の指輪が発動する確率（1/N、ターンごと）。
const TELEPORTITIS_ONE_IN: i32 = 40;

impl Game {
    /// 身につけている指輪（ストックの文字つき）。
    fn worn_rings(&self) -> Vec<(char, Tool)> {
        self.rings
            .iter()
            .flatten()
            .filter_map(|&l| self.inventory.iter().find(|s| s.letter == l).and_then(|s| s.tool.map(|t| (l, t))))
            .collect()
    }

    pub(super) fn ring_fx(&self) -> RingFx {
        let mut fx = RingFx::default();
        for (_, t) in self.worn_rings() {
            let Some(r) = t.kind.ring_def() else { continue };
            match r.effect {
                RingEffect::Protection => fx.protection += t.val,
                RingEffect::Strength => fx.strength += t.val,
                RingEffect::Dexterity => fx.dexterity += t.val,
                RingEffect::Damage => fx.damage += t.val,
                RingEffect::Regeneration => fx.regeneration += t.val,
                RingEffect::SlowDigestion => fx.slow_digestion += t.val,
                RingEffect::Stealth => fx.stealth += t.val,
                RingEffect::Searching => fx.searching += t.val,
                RingEffect::SeeInvisible => fx.see_invisible = true,
                RingEffect::Aggravate => fx.aggravate = true,
                RingEffect::Teleportitis => fx.teleportitis = true,
                RingEffect::Trinket => {}
            }
        }
        fx
    }

    /// 装備している（武器・防具・指輪）か。
    pub(super) fn is_equipped(&self, letter: char) -> bool {
        self.weapon == Some(letter) || self.armor == Some(letter) || self.rings.contains(&Some(letter))
    }

    /// 指輪の表示名。種類が分からなければ見た目、分かれば本名に強さを添える。
    pub(super) fn ring_name(&self, t: &Tool) -> String {
        if !self.known[t.kind.index()] {
            return self.looks[t.kind.index()].to_string();
        }
        let has_n = t.kind.ring_def().is_some_and(|r| r.effect.has_magnitude());
        match (has_n, t.identified) {
            (true, true) => format!("{} {:+}", t.kind.true_name(), t.val),
            (true, false) => format!("{} (+?)", t.kind.true_name()),
            (false, _) => t.kind.true_name().to_string(),
        }
    }

    /// 指輪の効果の説明（識別済みのときだけ使う）。
    fn ring_effect_text(t: &Tool) -> String {
        t.kind.ring_def().map_or(String::new(), |r| r.effect.describe(t.val))
    }

    /// 指輪を指にはめる。
    pub(super) fn equip_ring(&mut self, letter: char, t: Tool) -> (bool, String, bool) {
        if self.rings.contains(&Some(letter)) {
            return (false, format!("{}はすでにはめている。", self.ring_name(&t)), false);
        }
        let Some(slot) = self.rings.iter().position(|r| r.is_none()) else {
            return (
                false,
                "両手の指輪がふさがっている(指輪は2つまで)。先に unequip ではずそう。".to_string(),
                false,
            );
        };
        self.rings[slot] = Some(letter);
        let before = self.ring_name(&t);
        let mut msg = format!("{before}を指にはめた。");
        let kind_known = self.known[t.kind.index()];
        if t.cursed {
            // 呪いは身につけて初めて分かる。正体も明らかになる
            self.reveal_ring(letter);
            let t2 = self.tool_of(letter).expect("はめた指輪は持ち物にある");
            msg.push_str(&format!(" {}だった！ ({})", self.ring_name(&t2), Self::ring_effect_text(&t2)));
            if t2.is_sticky() {
                msg.push_str(" 呪われていた！ もうはずせない。");
            }
        } else if kind_known {
            self.reveal_ring(letter);
            let t2 = self.tool_of(letter).expect("はめた指輪は持ち物にある");
            msg.push_str(&format!(" {} ({})", self.ring_name(&t2), Self::ring_effect_text(&t2)));
        } else {
            msg.push_str(" (効果はまだ分からない。身につけているうちに分かる)");
        }
        (true, msg, true)
    }

    fn tool_of(&self, letter: char) -> Option<Tool> {
        self.inventory.iter().find(|s| s.letter == letter).and_then(|s| s.tool)
    }

    /// 指輪の種類と強さを明らかにする。
    pub(super) fn reveal_ring(&mut self, letter: char) {
        if let Some(t) = self.inventory.iter_mut().find(|s| s.letter == letter).and_then(|s| s.tool.as_mut()) {
            t.identified = true;
            self.known[t.kind.index()] = true;
        }
    }

    /// 指輪をはずす。
    pub(super) fn unequip_ring(&mut self, letter: char, t: Tool) -> (bool, String, bool) {
        let Some(slot) = self.rings.iter().position(|r| *r == Some(letter)) else {
            return (false, format!("{}ははめていない。", self.ring_name(&t)), false);
        };
        if t.is_sticky() {
            return (false, format!("{}は呪われていて、はずせない。", self.ring_name(&t)), false);
        }
        self.rings[slot] = None;
        (true, format!("{}をはずした。", self.ring_name(&t)), true)
    }

    /// 光源の表示名と燃料。
    pub(super) fn light_text(&self, t: &Tool) -> String {
        let max = t.kind.light_def().map_or(0, |l| l.max_fuel);
        format!("{} [燃料 {}/{}]", t.kind.true_name(), t.val, max)
    }

    /// 持ち物の光源に持ち替える。今の光源は持ち物に戻る。
    pub(super) fn equip_light(&mut self, letter: char, t: Tool) -> (bool, String, bool) {
        let si = self.inventory.iter().position(|s| s.letter == letter).expect("光源は持ち物にある");
        let old = self.light.replace(t);
        let mut msg = format!("{}に持ち替えた。", self.light_text(&t));
        match old {
            Some(o) => {
                self.inventory[si].kind = o.kind;
                self.inventory[si].tool = Some(o);
                msg.push_str(&format!(" 今までの{}は持ち物に戻した。({letter})", self.light_text(&o)));
            }
            None => {
                self.inventory.remove(si);
            }
        }
        self.refresh_fov();
        (true, msg, true)
    }

    /// `refill [油つぼの文字]`: ランタンに油を継ぎ足す。
    pub(super) fn refill_cmd(&mut self, flask: Option<char>) -> (bool, String, bool) {
        let Some(l) = self.light else {
            return (false, "光源を持っていない。".to_string(), false);
        };
        let def = l.kind.light_def().expect("光源には仕様がある");
        if !def.refillable {
            return (
                false,
                format!("{}には油を継ぎ足せない。ランタンを装備しよう。", l.kind.true_name()),
                false,
            );
        }
        if l.val >= def.max_fuel {
            return (false, format!("{}はもう満タンだ。", l.kind.true_name()), false);
        }
        let si = match flask {
            Some(c) => match self.inventory.iter().position(|s| s.letter == c) {
                Some(i) if self.inventory[i].kind == ItemKind::OilFlask => i,
                Some(_) => return (false, format!("{c} は油つぼではない。"), false),
                None => return (false, format!("持ち物 {c} はない。"), false),
            },
            None => match self.inventory.iter().position(|s| s.kind == ItemKind::OilFlask) {
                Some(i) => i,
                None => return (false, "油つぼを持っていない。".to_string(), false),
            },
        };
        let was_dark = l.val <= 0;
        let added = OIL_FLASK_FUEL.min(def.max_fuel - l.val);
        let now = {
            let light = self.light.as_mut().expect("光源がある");
            light.val += added;
            light.val
        };
        self.inventory[si].count -= 1;
        if self.inventory[si].count == 0 {
            self.inventory.remove(si);
        }
        if was_dark {
            self.refresh_fov();
        }
        (
            true,
            format!(
                "油を継ぎ足した。燃料 +{added} ({now}/{}){}",
                def.max_fuel,
                if was_dark { " 再び辺りが明るくなった。" } else { "" }
            ),
            true,
        )
    }

    /// 毎ターンの、装備に関する効果（指輪・光源）。ターン処理から呼ばれる唯一の入口。
    pub(super) fn tick_equipment(&mut self, fx: &RingFx) {
        // 自然回復。再生の指輪があれば速く、敵がいても回復する
        let interval = if fx.regeneration > 0 { (REGEN_INTERVAL as i32 - 3 * fx.regeneration).max(2) as u32 } else { REGEN_INTERVAL };
        if self.turn.is_multiple_of(interval)
            && self.hp < self.max_hp
            && !self.status.has(Status::Poisoned)
            && self.food > 0
            && (fx.regeneration > 0 || !(0..self.monsters.len()).any(|i| self.monster_aware(i)))
        {
            self.hp += 1;
        }
        // 罠を探す（探索の指輪で広く、確実に）
        self.search_traps(1 + fx.searching.max(0), 25 + 25 * fx.searching.max(0));
        // テレポート癖
        if fx.teleportitis && self.rng.range(0, TELEPORTITIS_ONE_IN) == 0 {
            self.teleport_player();
            self.note("指輪が妖しく光り、突然どこかへ飛ばされた！");
            self.alert = Some("指輪のせいで飛ばされて中断した。".to_string());
            self.trigger_trap();
        }
        // 光源の燃料
        if let Some(l) = self.light.as_mut() {
            if l.val > 0 {
                l.val -= 1;
                let name = l.kind.true_name();
                match l.val {
                    0 => {
                        self.note(&format!("{name}が燃え尽きた！ 辺りが暗くなり、視界が狭まった。"));
                        self.alert = Some("明かりが消えて中断した。".to_string());
                        self.refresh_fov();
                    }
                    LIGHT_LOW => {
                        self.note(&format!("{name}の火が弱くなってきた。(燃料 {LIGHT_LOW})"));
                        self.alert = Some("明かりが弱くなって中断した。".to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    /// 身につけている未識別の指輪は、時間が積もる。十分に経つと正体が分かる。
    pub(super) fn tick_rings_worn(&mut self) {
        for (letter, t) in self.worn_rings() {
            if t.identified && self.known[t.kind.index()] {
                continue;
            }
            let done = {
                let tool = self
                    .inventory
                    .iter_mut()
                    .find(|s| s.letter == letter)
                    .and_then(|s| s.tool.as_mut())
                    .expect("はめた指輪は持ち物にある");
                tool.worn += 1;
                tool.worn >= IDENTIFY_RING_AFTER
            };
            if done {
                let old = self.ring_name(&t);
                self.reveal_ring(letter);
                let t2 = self.tool_of(letter).expect("はめた指輪は持ち物にある");
                self.note(&format!(
                    "身につけているうちに、{old}の正体が分かった。{} ({})",
                    self.ring_name(&t2),
                    Self::ring_effect_text(&t2)
                ));
            }
        }
    }
}
