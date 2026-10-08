//! 杖を振る（`zap`）。回数制で、敵に当てる効果は状態の付与を使う。

use super::*;
use crate::command::ZapTarget;
use crate::item::ZapFx;

/// 杖の届く距離。
const ZAP_RANGE: i32 = 12;

impl Game {
    /// 杖を振る。(成功か, メッセージ, 1ターン消費するか)
    pub(super) fn zap_cmd(&mut self, letter: char, target: Option<ZapTarget>) -> (bool, String, bool) {
        let Some(si) = self.inventory.iter().position(|s| s.letter == letter) else {
            return (false, format!("持ち物 {letter} はない。"), false);
        };
        let kind = self.inventory[si].kind;
        let Some(zap) = kind.zap() else {
            return (false, format!("{letter} は杖ではない。zap は杖に使う。"), false);
        };
        let charges = self.inventory[si].tool.map_or(0, |t| t.val);
        if charges <= 0 {
            return (
                false,
                format!("{}は充填数が0で、もう使えない。", self.display_name(kind)),
                false,
            );
        }
        let target = if zap.aimed {
            match target {
                None => {
                    return (
                        false,
                        format!("{}には向きが要る。(例: zap {letter} east / zap {letter} nearest)", self.display_name(kind)),
                        false,
                    )
                }
                Some(ZapTarget::Dir(d)) => {
                    let (d, reeled) = self.confuse_dir(d);
                    if reeled {
                        self.events.push(format!("混乱して{}へ向けてしまった。", d.name()));
                    }
                    Some(ZapTarget::Dir(d))
                }
                Some(ZapTarget::Nearest) => {
                    // 混乱していると、狙いが外れてでたらめな向きに飛ぶことがある
                    if self.status.has(Status::Confused) && self.rng.range(0, 2) == 0 {
                        let d = Dir::ALL[self.rng.range(0, Dir::ALL.len() as i32) as usize];
                        self.events.push(format!("混乱して{}へ向けてしまった。", d.name()));
                        Some(ZapTarget::Dir(d))
                    } else {
                        Some(ZapTarget::Nearest)
                    }
                }
            }
        } else {
            None
        };
        let cells = match target {
            Some(t) => match self.bolt_cells(t) {
                Ok(c) => c,
                Err(msg) => return (false, msg, false),
            },
            None => Vec::new(),
        };

        let k = kind.index();
        let was_known = self.known[k];
        let prefix = if was_known {
            format!("{}を振った。", kind.true_name())
        } else {
            format!("{}を振った。これは{}だった！", self.looks[k], kind.true_name())
        };
        // 充填数を1つ使う
        let left = {
            let t = self.inventory[si].tool.as_mut().expect("杖は個体の情報を持つ");
            t.val -= 1;
            t.val
        };
        self.known[k] = true;
        let body = self.zap_effect(zap.fx, &cells);
        self.monsters.retain(|m| m.hp > 0);
        let tail = if left == 0 { "杖の魔力は尽きた。".to_string() } else { format!("(残り{left}回)") };
        (true, format!("{prefix} {body} {tail}"), true)
    }

    /// 杖が飛ぶ道筋（壁の手前まで。`Light` のために壁も含めて返し、使う側で切る）。
    pub(super) fn bolt_cells(&self, t: ZapTarget) -> Result<Vec<(i32, i32)>, String> {
        match t {
            ZapTarget::Dir(d) => {
                let (dx, dy) = d.delta();
                Ok((1..=ZAP_RANGE).map(|k| (self.pos.0 + dx * k, self.pos.1 + dy * k)).collect())
            }
            ZapTarget::Nearest => {
                let nearest = self
                    .visible_monster_indices()
                    .into_iter()
                    .min_by_key(|&i| {
                        let p = self.monsters[i].pos;
                        (p.0 - self.pos.0).pow(2) + (p.1 - self.pos.1).pow(2)
                    })
                    .ok_or_else(|| "狙える敵が見えない。向きを指定しよう。".to_string())?;
                // 狙った敵のさきまで、同じ向きに伸ばす(貫く雷や、照らす光が先まで届く)
                let t = self.monsters[nearest].pos;
                let (dx, dy) = (t.0 - self.pos.0, t.1 - self.pos.1);
                let m = dx.abs().max(dy.abs()).max(1);
                let long = Map::line(self.pos, (self.pos.0 + dx * ZAP_RANGE / m, self.pos.1 + dy * ZAP_RANGE / m));
                // 延長した線が壁で先に遮られて敵に届かないときは、敵までの線に戻る
                Ok(if self.open_cells(&long).contains(&t) { long } else { Map::line(self.pos, t) })
            }
        }
    }

    /// 道筋のうち壁にぶつかる手前まで。
    pub(super) fn open_cells(&self, cells: &[(i32, i32)]) -> Vec<(i32, i32)> {
        cells
            .iter()
            .copied()
            .take_while(|p| self.map.tile(p.0, p.1).walkable())
            .collect()
    }

    /// 道筋上の最初の敵。
    fn first_monster_on(&self, cells: &[(i32, i32)]) -> Option<usize> {
        self.open_cells(cells).into_iter().find_map(|p| self.monster_at(p))
    }

    fn zap_effect(&mut self, fx: ZapFx, cells: &[(i32, i32)]) -> String {
        match fx {
            ZapFx::Light => {
                let open = self.open_cells(cells);
                // 壁にぶつかった場所まで、周りも含めて地図に書き込む
                let end = cells.get(open.len()).copied();
                for p in open.iter().copied().chain(end) {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            self.map.mark_seen(p.0 + dx, p.1 + dy);
                        }
                    }
                }
                let found: Vec<(usize, (i32, i32))> = open
                    .iter()
                    .filter_map(|&p| self.monster_at(p).map(|i| (i, p)))
                    .collect();
                for &(i, p) in &found {
                    self.detect_marks.push((p, self.monsters[i].glyph));
                }
                let mut s = format!("光の筋が{}マス先まで走り、通り道が照らされた。", open.len());
                for (i, _) in found {
                    s.push_str(&format!(" 光の中に{}がいる。", self.monsters[i].name));
                }
                s
            }
            ZapFx::Damage { lo, hi, beam, drain, then, text } => {
                let hits: Vec<usize> = if beam {
                    self.open_cells(cells).into_iter().filter_map(|p| self.monster_at(p)).collect()
                } else {
                    self.first_monster_on(cells).into_iter().collect()
                };
                if hits.is_empty() {
                    return format!("{text}、何にも当たらなかった。");
                }
                let mut parts = Vec::new();
                for i in hits {
                    let name = self.foe_name(i);
                    let seen = self.can_see_monster(i);
                    let dmg = self.rng.range(lo, hi + 1);
                    self.monsters[i].hp -= dmg;
                    let (hp, max_hp) = (self.monsters[i].hp, self.monsters[i].max_hp);
                    let mut part = if hp <= 0 {
                        let xp = self.monsters[i].kind.xp + self.depth - 1;
                        self.pending_xp += xp;
                        format!("{name}に{dmg}のダメージ。{name}を倒した！ (経験値 +{xp})")
                    } else if seen {
                        format!("{name}に{dmg}のダメージを与えた。(HP {hp}/{max_hp})")
                    } else {
                        format!("{name}に{dmg}のダメージを与えた。")
                    };
                    if drain {
                        let gained = dmg.min(self.max_hp - self.hp);
                        if gained > 0 {
                            self.hp += gained;
                            part.push_str(&format!(" 生命力を吸い取った。HP+{gained} (HP {}/{})", self.hp, self.max_hp));
                        }
                    }
                    if let (Some((st, turns)), true) = (then, hp > 0) {
                        if self.inflict_monster(i, st, turns) {
                            part.push_str(&format!(" {}は{}状態になった。", name, st.name()));
                        }
                    }
                    parts.push(part);
                }
                format!("{text}、{}", parts.join(" "))
            }
            ZapFx::Afflict(st, turns) => match self.first_monster_on(cells) {
                Some(i) => {
                    let name = self.foe_name(i);
                    if st == Status::Invisible {
                        // 透明になる前の姿で書く
                        self.inflict_monster(i, st, turns);
                        format!("{name}の姿が消えた！ (これで{name}は見えない。透明視認なら見える)")
                    } else if self.inflict_monster(i, st, turns) {
                        format!("{name}は{}状態になった。", st.name())
                    } else {
                        format!("{name}には効かなかった。")
                    }
                }
                None => "何にも当たらなかった。".to_string(),
            },
            ZapFx::Polymorph => match self.first_monster_on(cells) {
                Some(i) => {
                    let old = self.foe_name(i);
                    // 今と違う種類から選ぶ
                    let cur = KINDS.iter().position(|k| std::ptr::eq(*k, self.monsters[i].kind)).unwrap_or(0);
                    let mut pick = self.rng.range(0, KINDS.len() as i32 - 1) as usize;
                    if pick >= cur {
                        pick += 1;
                    }
                    let kind = KINDS[pick];
                    let (hp, max_hp) = (self.monsters[i].hp, self.monsters[i].max_hp);
                    let new_max = kind.hp_at(self.depth);
                    let m = &mut self.monsters[i];
                    m.kind = kind;
                    m.name = kind.name;
                    m.glyph = kind.glyph;
                    m.max_hp = new_max;
                    m.hp = (hp * new_max / max_hp.max(1)).max(1);
                    m.cancelled = false;
                    format!("{old}の姿がぐにゃりと変わり、{}になった！ (HP {}/{})", self.foe_name(i), self.monsters[i].hp, new_max)
                }
                None => "何にも当たらなかった。".to_string(),
            },
            ZapFx::Cancel => match self.first_monster_on(cells) {
                Some(i) => {
                    let name = self.foe_name(i);
                    let kind = self.monsters[i].kind;
                    let had_special = kind.poisons || kind.corrodes || kind.erratic;
                    let active = self.monsters[i].status.active();
                    for (st, _) in &active {
                        self.monsters[i].status.clear(*st);
                        self.status_events.push(StatusEvent {
                            target: self.monsters[i].name.to_string(),
                            status: *st,
                            change: Change::End,
                        });
                    }
                    self.monsters[i].cancelled = true;
                    if had_special || !active.is_empty() {
                        format!("{name}にかかっていた力が消え去った。")
                    } else {
                        format!("{name}に特に変わりはなかった。")
                    }
                }
                None => "何にも当たらなかった。".to_string(),
            },
            ZapFx::TeleportOther => match self.first_monster_on(cells) {
                Some(i) => {
                    let name = self.foe_name(i);
                    for _ in 0..200 {
                        let x = self.rng.range(1, W - 1);
                        let y = self.rng.range(1, H - 1);
                        if self.map.tile(x, y) == Tile::Floor
                            && (x, y) != self.pos
                            && self.monster_at((x, y)).is_none()
                        {
                            self.monsters[i].pos = (x, y);
                            break;
                        }
                    }
                    format!("{name}は遠くへ飛ばされた！")
                }
                None => "何にも当たらなかった。".to_string(),
            },
            ZapFx::TeleportSelf => {
                self.teleport_player();
                "景色が一変した。".to_string()
            }
        }
    }
}
