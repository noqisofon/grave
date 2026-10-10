use super::*;

fn monster(kind: &'static MonsterKind, pos: (i32, i32), hp: i32) -> Monster {
    Monster {
        kind,
        name: kind.name,
        glyph: kind.glyph,
        hp,
        max_hp: hp,
        pos,
        status: StatusSet::default(),
        cancelled: false,
    }
}

fn slime(pos: (i32, i32), hp: i32) -> Monster {
    monster(&crate::monster::SLIME, pos, hp)
}

/// 開始位置の東隣にスライムを置く（開始位置は部屋の中央なので必ず歩ける）。
fn with_adjacent_slime(seed: u64, hp: i32) -> Game {
    let mut g = Game::new(seed);
    g.monsters.clear();
    let p = (g.pos.0 + 1, g.pos.1);
    assert!(g.map.tile(p.0, p.1).walkable());
    g.monsters.push(slime(p, hp));
    g
}

/// 開始位置の東隣に、指定の敵を置いたゲーム。プレイヤーのHPは十分に高くする。
fn with_adjacent(seed: u64, kind: &'static MonsterKind) -> Game {
    let mut g = Game::new(seed);
    g.monsters.clear();
    g.floor_items.clear();
    g.traps.clear();
    g.hp = 1000;
    g.max_hp = 1000;
    let p = (g.pos.0 + 1, g.pos.1);
    assert!(g.map.tile(p.0, p.1).walkable());
    g.monsters.push(monster(kind, p, 1000));
    g
}

/// 持ち物に装備品を直接入れた静かなゲーム。
fn with_gear(kinds: &[ItemKind]) -> Game {
    let mut g = quiet(2);
    for k in kinds {
        g.take(*k).unwrap();
    }
    g
}

#[test]
fn equip_weapon_changes_attack_range() {
    let mut g = with_gear(&[ItemKind::Axe]);
    let o = g.run("equip a");
    assert!(o.ok, "{}", o.message);
    assert!(o.message.contains("Axeを装備した"), "{}", o.message);
    assert!(g.inventory_lines()[0].contains("(装備中)"));
    assert!(g.observe_text(3).contains("攻撃 5〜9"));
    // 敵の隣で殴る: ダメージは 5..=9
    let p = (g.pos.0 + 1, g.pos.1);
    let mut seen = std::collections::HashSet::new();
    for seed_off in 0..40 {
        g.monsters.clear();
        g.monsters.push(monster(&crate::monster::OGRE, p, 1000));
        g.hp = 1000;
        g.max_hp = 1000;
        let _ = seed_off;
        let before = g.monsters[0].hp;
        g.run("attack east");
        seen.insert(before - g.monsters[0].hp);
    }
    assert!(seen.iter().all(|d| (5..=9).contains(d)), "{seen:?}");
    assert!(seen.len() > 1);
}

#[test]
fn equip_replaces_same_slot_and_unequip_works() {
    let mut g = with_gear(&[ItemKind::Dagger, ItemKind::Sword, ItemKind::Leather]);
    assert!(g.run("equip a").ok);
    let o = g.run("equip b");
    assert!(
        o.ok && o.message.contains("Daggerをはずした"),
        "{}",
        o.message
    );
    assert!(g.run("equip c").ok); // 防具は別枠
    let lines = g.inventory_lines();
    assert!(!lines[0].contains("(装備中)"));
    assert!(lines[1].contains("(装備中)"));
    assert!(lines[2].contains("(装備中)"));
    // すでに装備している・装備していないものは失敗してターンを使わない
    let t = g.turn();
    assert!(!g.run("equip b").ok);
    assert!(!g.run("unequip a").ok);
    assert_eq!(g.turn(), t);
    assert!(g.run("unequip b").ok);
    assert!(g.observe_text(3).contains("攻撃 2〜4"));
    assert_eq!(g.turn(), t + 1);
}

#[test]
fn equip_only_takes_gear_and_consumables_need_the_matching_verb() {
    let mut g = with_gear(&[ItemKind::Plate, ItemKind::Healing]);
    assert!(g.run("equip a").message.contains("Plate Armorを装備した"));
    let o = g.run("equip b");
    assert!(!o.ok);
    assert!(!g.run("unequip b").ok);
    assert!(!g.run("equip z").ok);

    // 種類の合わない動詞は、ターンを使わず失敗する
    let mut g = with_gear(&[
        ItemKind::Plate,
        ItemKind::Healing,
        ItemKind::Bread,
        ItemKind::Teleport,
    ]);
    let turn = g.turn();
    for (cmd, hint) in [
        ("quaff a", "equip"),
        ("eat b", "quaff"),
        ("read b", "quaff"),
        ("quaff c", "eat"),
        ("read c", "eat"),
        ("eat d", "read"),
        ("quaff d", "read"),
        ("eat a", "equip"),
    ] {
        let o = g.run(cmd);
        assert!(!o.ok && o.message.contains(hint), "{cmd}: {}", o.message);
    }
    assert_eq!(g.turn(), turn);
}

#[test]
fn armor_reduces_damage_but_never_below_one() {
    let mut g = with_adjacent(3, &crate::monster::OGRE);
    g.take(ItemKind::Plate);
    g.run("equip a");
    let mut dmgs = std::collections::HashSet::new();
    for _ in 0..40 {
        let before = g.hp;
        g.run("wait");
        if before != g.hp {
            dmgs.insert(before - g.hp);
        }
    }
    // オーガ 3..=6 から 3 引いて、最低 1
    assert!(dmgs.iter().all(|d| (1..=3).contains(d)), "{dmgs:?}");
    let mut g = with_adjacent(3, &crate::monster::BAT);
    g.take(ItemKind::Plate);
    g.run("equip a");
    for _ in 0..20 {
        let before = g.hp;
        g.run("wait");
        let d = before - g.hp;
        assert!(d >= 0);
    }
    assert!(g.hp < 1000, "最低1ダメージは通るはず");
}

#[test]
fn gear_spawns_by_depth_and_is_known_on_pickup() {
    let mut seen = std::collections::HashSet::new();
    for seed in 0..80 {
        let mut g = Game::new(seed);
        for depth in [1u32, 5] {
            g.depth = depth;
            g.spawn_items();
            for f in &g.floor_items {
                let k = &f.item;
                assert!(k.kind().min_depth() <= depth, "{k:?} at {depth}");
                seen.insert(k.kind());
            }
        }
    }
    // 深い階では装備品も実際に出る（どの種類が出るかは乱数次第なので、種類までは問わない）
    assert!(seen.iter().any(|k| k.is_weapon()));
    assert!(seen.iter().any(|k| k.is_armor()));
    let mut g = quiet(4);
    let p = (g.pos.0 + 1, g.pos.1);
    g.floor_items.push(FloorItem::new(p, ItemKind::Sword));
    let o = g.run("move east");
    assert!(o.message.contains("Swordを拾った"), "{}", o.message);
}

/// 指定の個体を持ち物に直接入れる。
fn give(g: &mut Game, gear: Gear) -> char {
    g.take(Item::Gear(gear)).unwrap()
}

fn quality_gear(kind: ItemKind, quality: crate::item::Quality, bonus: i32) -> Gear {
    Gear {
        kind,
        quality,
        word: quality.words()[0],
        bonus,
        suffix: None,
        identified: true,
        worn: 0,
        enchant: 0,
        protected: false,
        freed: false,
    }
}

#[test]
fn quality_weights_shift_deeper_and_always_sum_to_100() {
    use crate::item::Quality;
    for d in 1..=30 {
        let w = Quality::weights(d);
        assert_eq!(w.iter().sum::<i32>(), 100, "depth {d}");
        assert!(w.iter().all(|x| *x >= 0), "depth {d}: {w:?}");
    }
    assert_eq!(Quality::weights(1)[2..], [0, 0]);
    assert!(Quality::weights(20)[3] > 0);
    assert!(Quality::weights(20)[0] < Quality::weights(1)[0]);
}

#[test]
fn rolled_gear_stays_inside_its_quality() {
    use crate::item::Quality;
    let mut rng = Rng::new(9);
    let mut seen = std::collections::HashSet::new();
    for i in 0..2000 {
        let depth = 1 + (i % 30);
        let g = Gear::roll(&mut rng, ItemKind::Sword, depth);
        let (lo, hi) = g.quality.bonus_range();
        assert!((lo..=hi).contains(&g.bonus), "{g:?}");
        assert!(g.quality.words().contains(&g.word), "{g:?}");
        seen.insert(g.quality);
    }
    assert_eq!(seen.len(), Quality::ALL.len());
}

#[test]
fn gear_drops_are_deterministic_for_a_seed() {
    let drops = |seed: u64| {
        let mut g = Game::new(seed);
        let mut v = Vec::new();
        for depth in [3u32, 9, 15] {
            g.depth = depth;
            g.spawn_items();
            v.extend(g.floor_items.iter().cloned());
        }
        v
    };
    for seed in 0..10 {
        assert_eq!(drops(seed), drops(seed), "seed {seed}");
    }
    assert!((0..30).any(|s| drops(s) != drops(s + 100)));
}

#[test]
fn gear_does_not_stack_and_takes_one_letter_each() {
    let mut g = quiet(2);
    let a = g.take(ItemKind::Dagger).unwrap();
    let b = g.take(ItemKind::Dagger).unwrap();
    assert_ne!(a, b);
    assert!(g.inventory.iter().all(|s| s.count == 1));
    // 薬は今までどおり重なる
    let p1 = g.take(ItemKind::Healing).unwrap();
    assert_eq!(g.take(ItemKind::Healing), Some(p1));
    assert_eq!(g.inventory.len(), 3);
}

#[test]
fn quality_bonus_applies_to_attack_range_and_defense() {
    use crate::item::Quality;
    let mut g = quiet(2);
    let w = give(&mut g, quality_gear(ItemKind::Sword, Quality::Rare, 3));
    let a = give(&mut g, quality_gear(ItemKind::Chain, Quality::Uncommon, 2));
    assert!(g.run(&format!("equip {w}")).ok);
    assert!(g.run(&format!("equip {a}")).ok);
    assert_eq!(g.attack_range(), (4 + 3, 7 + 3));
    assert_eq!(g.defense(), 2 + 2);
    let o = g.observe_text(3);
    assert!(o.contains("攻撃 7〜10") && o.contains("防御 4"), "{o}");
    // 実際の殴りダメージも範囲内
    let p = (g.pos.0 + 1, g.pos.1);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..60 {
        g.monsters.clear();
        g.monsters.push(monster(&crate::monster::OGRE, p, 1000));
        g.hp = 1000;
        g.run("attack east");
        seen.insert(1000 - g.monsters[0].hp);
    }
    assert!(seen.iter().all(|d| (7..=10).contains(d)), "{seen:?}");
}

fn suffix_gear(kind: ItemKind, suffix: Suffix) -> Gear {
    Gear {
        kind,
        quality: crate::item::Quality::Rare,
        word: "Sanctified",
        bonus: 2,
        suffix: Some(suffix),
        identified: false,
        worn: 0,
        enchant: 0,
        protected: false,
        freed: false,
    }
}

#[test]
fn rolled_suffixes_match_the_slot_and_only_uncommon_or_better_have_them() {
    use crate::item::Quality;
    let mut rng = Rng::new(5);
    let mut seen = std::collections::HashSet::new();
    for i in 0..3000 {
        let kind = if i % 2 == 0 {
            ItemKind::Axe
        } else {
            ItemKind::Plate
        };
        let g = Gear::roll(&mut rng, kind, 1 + (i % 30) as u32);
        if let Some(s) = g.suffix {
            assert_ne!(g.quality, Quality::Common, "{g:?}");
            let pool: Vec<Suffix> = if kind.is_weapon() {
                Suffix::WEAPON.iter().map(|x| x.0).collect()
            } else {
                Suffix::ARMOR.iter().map(|x| x.0).collect()
            };
            assert!(pool.contains(&s), "{g:?}");
            seen.insert(s);
        }
        assert_eq!(g.identified, g.quality == Quality::Common, "{g:?}");
    }
    assert_eq!(seen.len(), Suffix::WEAPON.len() + Suffix::ARMOR.len());
}

#[test]
fn might_adds_attack_and_vampire_heals_on_hit() {
    let mut g = with_adjacent(3, &crate::monster::OGRE);
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
    assert!(g.run(&format!("equip {w}")).ok);
    assert_eq!(g.attack_range(), (4 + 2 + 1, 7 + 2 + 1));

    let mut g = with_adjacent(3, &crate::monster::OGRE);
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vampire));
    g.run(&format!("equip {w}"));
    g.hp = 500;
    let o = g.run("attack east");
    // 敵の反撃で減る分があるので、吸った分のメッセージで確かめる
    assert!(o.message.contains("HP+1"), "{}", o.message);
}

#[test]
fn vigor_heals_only_on_a_kill() {
    let mut g = with_adjacent(3, &crate::monster::SLIME);
    g.monsters[0].hp = 1000;
    g.monsters[0].max_hp = 1000;
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vigor));
    g.run(&format!("equip {w}"));
    g.hp = 500;
    let o = g.run("attack east");
    assert!(!o.message.contains("HP+"), "{}", o.message);
    g.monsters[0].hp = 1;
    let o = g.run("attack east");
    assert!(
        o.message.contains("倒した") && o.message.contains("HP+2"),
        "{}",
        o.message
    );
}

#[test]
fn thorns_hurt_attackers_and_can_kill_them() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    let a = give(&mut g, suffix_gear(ItemKind::Leather, Suffix::Thorns));
    g.run(&format!("equip {a}"));
    let before = g.monsters[0].hp;
    let o = g.run("wait");
    assert!(o.message.contains("トゲ"), "{}", o.message);
    assert_eq!(g.monsters[0].hp, before - 1);
    // とどめを刺す: 取り除かれて経験値が入る
    g.monsters[0].hp = 1;
    let xp = g.xp;
    let o = g.run("wait");
    assert!(
        o.message.contains("トゲ") && o.message.contains("倒した"),
        "{}",
        o.message
    );
    assert!(g.monsters.is_empty());
    assert!(g.xp > xp);
}

#[test]
fn warding_blocks_poison_from_every_source() {
    let mut g = with_adjacent(3, &crate::monster::SPIDER);
    let a = give(&mut g, suffix_gear(ItemKind::Leather, Suffix::Warding));
    g.run(&format!("equip {a}"));
    for _ in 0..40 {
        g.run("wait");
    }
    assert_eq!(g.poison(), 0);
    assert!(g.hp < 1000, "攻撃自体は受ける");
    g.take(ItemKind::PoisonShroom);
    let l = g
        .inventory
        .iter()
        .find(|s| s.kind == ItemKind::PoisonShroom)
        .unwrap()
        .letter;
    let o = g.run(&format!("eat {l}"));
    assert_eq!(g.poison(), 0, "{}", o.message);
}

#[test]
fn famine_drains_extra_food() {
    let mut plain = quiet(2);
    let mut cursed = quiet(2);
    let a = give(&mut cursed, suffix_gear(ItemKind::Leather, Suffix::Famine));
    cursed.run(&format!("equip {a}"));
    let (f0, f1) = (plain.food, cursed.food);
    plain.run("stay 20");
    cursed.run("stay 20");
    assert_eq!(f0 - plain.food, 20);
    assert!(f1 - cursed.food >= 29, "{} -> {}", f1, cursed.food);
}

#[test]
fn cursed_gear_cannot_be_unequipped_or_swapped() {
    let mut g = quiet(2);
    let c = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
    let d = give(&mut g, Gear::plain(ItemKind::Dagger));
    // 装備するまで呪いは分からない
    assert!(
        !g.inventory_lines()[0].contains("呪"),
        "{:?}",
        g.inventory_lines()
    );
    let o = g.run(&format!("equip {c}"));
    assert!(o.ok && o.message.contains("呪われていた"), "{}", o.message);
    let t = g.turn();
    let o = g.run(&format!("unequip {c}"));
    assert!(!o.ok && o.message.contains("呪われていて"), "{}", o.message);
    let o = g.run(&format!("equip {d}"));
    assert!(!o.ok && o.message.contains("呪われていて"), "{}", o.message);
    assert_eq!(g.turn(), t, "失敗はターンを使わない");
    assert_eq!(g.weapon, Some(c));
    // 呪いの攻撃+3 は効いている
    assert_eq!(g.attack_range(), (5 + 2 + 3, 9 + 2 + 3));
}

#[test]
fn unidentified_gear_shows_prefix_but_hides_suffix() {
    let mut g = quiet(2);
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vampire));
    let line = g.inventory_lines().remove(0);
    assert!(line.contains("Sanctified Sword (?)"), "{line}");
    assert!(!line.contains("Vampire"), "{line}");
    // 床の上でも同じ見え方
    let p = (g.pos.0 + 1, g.pos.1);
    g.floor_items.push(FloorItem::new(
        p,
        Item::Gear(suffix_gear(ItemKind::Axe, Suffix::Thorns)),
    ));
    g.map.update_fov(g.pos, FOV_RADIUS);
    let look = g.run("look").message;
    assert!(
        look.contains("Sanctified Axe (?)") && !look.contains("Thorns"),
        "{look}"
    );
    // 普通の品は隠すものがないので (?) が付かない
    g.take(ItemKind::Dagger);
    assert!(g
        .inventory_lines()
        .iter()
        .any(|l| l.contains("Basic Dagger [") && !l.contains("(?)")));
    let _ = w;
}

#[test]
fn identify_scroll_works_on_gear_by_target_and_automatically() {
    let mut g = quiet(2);
    g.known[ItemKind::Identify.index()] = true;
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Vampire));
    let s1 = g.take(ItemKind::Identify).unwrap();
    let o = g.run(&format!("read {s1} {w}"));
    assert!(
        o.ok && o.message.contains("of the Vampire"),
        "{}",
        o.message
    );
    assert!(g.inventory_lines()[0].contains("Sanctified Sword of the Vampire"));
    assert!(!g.inventory_lines()[0].contains("(?)"));
    // 識別済みの装備は対象にできず、巻物も減らない
    let s2 = g.take(ItemKind::Identify).unwrap();
    let o = g.run(&format!("read {s2} {w}"));
    assert!(
        !o.ok && o.message.contains("すでに識別済み"),
        "{}",
        o.message
    );
    // 対象を省くと、未識別の装備が自動で選ばれる
    let a = give(&mut g, suffix_gear(ItemKind::Plate, Suffix::Warding));
    let o = g.run(&format!("read {s2}"));
    assert!(o.ok && o.message.contains("of Warding"), "{}", o.message);
    assert!(
        g.inventory
            .iter()
            .find(|s| s.letter == a)
            .unwrap()
            .gear
            .unwrap()
            .identified
    );
}

#[test]
fn cursed_gear_is_identified_by_scroll_before_wearing() {
    let mut g = quiet(2);
    g.known[ItemKind::Identify.index()] = true;
    let c = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
    let s = g.take(ItemKind::Identify).unwrap();
    let o = g.run(&format!("read {s} {c}"));
    assert!(o.message.contains("呪われていて"), "{}", o.message);
}

#[test]
fn worn_gear_is_identified_after_enough_turns() {
    let mut g = quiet(2);
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
    let spare = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Might));
    g.run(&format!("equip {w}"));
    for _ in 0..(IDENTIFY_AFTER_WORN - 2) {
        g.run("wait");
    }
    assert!(g.inventory_lines()[0].contains("(?)"));
    let mut told = false;
    for _ in 0..4 {
        told |= g.run("wait").message.contains("正体が分かった");
    }
    assert!(told);
    assert!(g.inventory_lines()[0].contains("of Might") && !g.inventory_lines()[0].contains("(?)"));
    // 装備していないものは、時間が経っても分からない
    assert!(g.inventory_lines()[1].contains("(?)"));
    let _ = spare;
}

#[test]
fn inventory_lines_show_stats_diff_and_identification() {
    use crate::item::Quality;
    let mut g = quiet(2);
    let known = quality_gear(ItemKind::Sword, Quality::Uncommon, 2);
    let a = give(&mut g, Gear::plain(ItemKind::Dagger));
    let b = give(&mut g, known);
    let c = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
    let l = g.inventory_lines();
    // 素手(2〜4)との差
    assert!(
        l[0].contains("Basic Dagger [攻撃 3〜5 (装備比 +1〜+1)]"),
        "{}",
        l[0]
    );
    assert!(
        l[1].contains("Basic Sword [攻撃 6〜9 (装備比 +4〜+5)]")
            || l[1].contains("Sword [攻撃 6〜9"),
        "{}",
        l[1]
    );
    // 未識別は、正確な値ではなく分かる範囲だけ
    assert!(
        l[2].contains("Sanctified Sword (?) [攻撃 4〜7 +(2〜3)?]"),
        "{}",
        l[2]
    );
    assert!(!l[2].contains("装備比"), "{}", l[2]);
    g.run(&format!("equip {a}"));
    let l = g.inventory_lines();
    assert!(
        l[0].contains("(装備中)") && !l[0].contains("装備比"),
        "{}",
        l[0]
    );
    // 差は今の装備(短剣 3〜5)から
    assert!(l[1].contains("(装備比 +3〜+4)"), "{}", l[1]);
    let _ = (b, c);
}

#[test]
fn inventory_command_and_heading_carry_the_full_picture() {
    let mut g = quiet(2);
    let a = give(&mut g, suffix_gear(ItemKind::Plate, Suffix::Thorns));
    g.run(&format!("equip {a}"));
    let o = g.observe_text(3);
    // 見出しは品質補正を含む値 (板金 3 + 補正 2)
    assert!(o.contains("防御 5"), "{o}");
    assert!(
        o.contains("a) Sanctified Plate Armor (?) [防御 3 +(2〜3)?] (装備中)"),
        "{o}"
    );
}

#[test]
fn spider_killed_by_thorns_does_not_poison() {
    let mut g = with_adjacent(3, &crate::monster::SPIDER);
    let a = give(&mut g, suffix_gear(ItemKind::Leather, Suffix::Thorns));
    g.run(&format!("equip {a}"));
    for _ in 0..40 {
        g.monsters.clear();
        let p = (g.pos.0 + 1, g.pos.1);
        g.monsters.push(monster(&crate::monster::SPIDER, p, 1));
        g.status.clear(Status::Poisoned);
        g.run("wait");
        assert_eq!(g.poison(), 0, "倒された毒グモが毒を撒いた");
        assert!(g.monsters.is_empty());
    }
}

/// 足元に、印のない物 `fresh` と、捨てた物 `dropped` を置く。
fn put_underfoot(g: &mut Game, fresh: &[Item], dropped: &[Item]) {
    for it in fresh {
        g.floor_items.push(FloorItem::new(g.pos, *it));
    }
    for it in dropped {
        g.floor_items.push(FloorItem {
            pos: g.pos,
            item: *it,
            dropped: true,
        });
    }
}

#[test]
fn drop_puts_gear_underfoot_and_marks_it() {
    let mut g = quiet(2);
    let a = give(&mut g, Gear::plain(ItemKind::Dagger));
    let t = g.turn();
    let o = g.run(&format!("drop {a}"));
    assert!(
        o.ok && o.message.contains("Basic Daggerを足元に捨てた"),
        "{}",
        o.message
    );
    assert_eq!(g.turn(), t + 1);
    assert!(g.inventory.is_empty());
    assert_eq!(g.floor_items.len(), 1);
    assert!(g.floor_items[0].dropped && g.floor_items[0].pos == g.pos);
    // 個体の情報(未識別など)はそのまま床へ
    let w = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Might));
    g.run(&format!("drop {w}"));
    assert_eq!(
        g.floor_items[1].item,
        Item::Gear(suffix_gear(ItemKind::Sword, Suffix::Might))
    );
}

#[test]
fn drop_with_a_count_drops_that_many_one_by_one() {
    let mut g = quiet(2);
    for _ in 0..3 {
        g.take(ItemKind::Healing);
    }
    let o = g.run("drop a 2");
    assert!(o.ok && o.message.contains("2個"), "{}", o.message);
    assert_eq!(g.inventory[0].count, 1);
    assert_eq!(g.floor_items.iter().filter(|f| f.dropped).count(), 2);
    // 数を省くと1個。使い切ると枠が空く
    assert!(g.run("drop a").ok);
    assert!(g.inventory.is_empty());
    assert_eq!(g.floor_items.len(), 3);
}

#[test]
fn drop_fails_without_spending_a_turn() {
    let mut g = quiet(2);
    let w = give(&mut g, Gear::plain(ItemKind::Dagger));
    let c = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
    g.take(ItemKind::Healing);
    g.run(&format!("equip {w}"));
    // 装備中
    let o = g.run(&format!("drop {w}"));
    assert!(!o.ok && o.message.contains("unequip"), "{}", o.message);
    // 呪われて装備中
    g.run(&format!("unequip {w}"));
    g.run(&format!("equip {c}"));
    let t = g.turn();
    let o = g.run(&format!("drop {c}"));
    assert!(!o.ok && o.message.contains("呪われて"), "{}", o.message);
    // 持ち物にない文字、持っている数より多い
    assert!(!g.run("drop z").ok);
    assert!(!g.run("drop c 5").ok);
    assert_eq!(g.turn(), t, "失敗はターンを使わない");
    assert!(g.floor_items.is_empty());
    assert_eq!(g.inventory.len(), 3);
    // 呪われていても、まだ身につけていなければ捨てられる(呪いが漏れない)
    let c2 = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
    assert!(g.run(&format!("drop {c2}")).ok);
}

#[test]
fn dropped_items_are_not_picked_up_by_walking_or_staying() {
    let mut g = quiet(2);
    let a = give(&mut g, Gear::plain(ItemKind::Dagger));
    g.run(&format!("drop {a}"));
    let here = g.pos;
    assert!(g.run("move east").ok);
    let o = g.run("move west");
    assert_eq!(g.pos, here);
    assert!(!o.message.contains("拾った"), "{}", o.message);
    let o = g.run("stay 3");
    assert!(!o.message.contains("拾った"), "{}", o.message);
    assert!(g.inventory.is_empty());
    assert_eq!(g.floor_items.len(), 1);
}

#[test]
fn explore_does_not_go_for_dropped_items() {
    let mut g = quiet(3);
    g.map.reveal_all();
    let far = *g
        .find_path(&|q| {
            g.map.tile(q.0, q.1) == Tile::Floor && (q.0 - g.pos.0).abs() + (q.1 - g.pos.1).abs() > 6
        })
        .unwrap()
        .last()
        .unwrap();
    g.floor_items.push(FloorItem {
        pos: far,
        item: ItemKind::Healing.into(),
        dropped: true,
    });
    let start = g.pos;
    let o = g.run("explore");
    assert!(
        o.message.contains("もう探索する場所がない"),
        "{}",
        o.message
    );
    assert_eq!(g.pos, start);
    // 印のない物なら、今までどおり拾いに行く
    g.floor_items[0].dropped = false;
    g.run("explore");
    assert_eq!(g.pos, far);
    assert_eq!(g.inventory.len(), 1);
}

#[test]
fn walking_onto_a_tile_picks_only_the_top_undropped_item() {
    let mut g = quiet(2);
    let p = (g.pos.0 + 1, g.pos.1);
    g.floor_items.push(FloorItem {
        pos: p,
        item: ItemKind::Sleep.into(),
        dropped: true,
    });
    g.floor_items.push(FloorItem::new(p, ItemKind::Healing));
    g.floor_items.push(FloorItem::new(p, ItemKind::Bread));
    let o = g.run("move east");
    assert!(o.message.contains("を拾った"), "{}", o.message);
    assert_eq!(g.inventory.len(), 1);
    assert_eq!(g.inventory[0].kind, ItemKind::Healing, "印のない先頭");
    assert_eq!(g.floor_items.len(), 2);
}

#[test]
fn pickup_without_a_number_takes_undropped_first_then_dropped() {
    let mut g = quiet(2);
    // 捨てた物を先に置いても、印のない物が先に拾われる
    put_underfoot(&mut g, &[], &[ItemKind::Sleep.into()]);
    put_underfoot(&mut g, &[ItemKind::Healing.into()], &[]);
    let t = g.turn();
    let o = g.run("pickup");
    assert!(o.ok && o.message.contains("を拾った"), "{}", o.message);
    assert_eq!(g.inventory[0].kind, ItemKind::Healing);
    assert_eq!(g.turn(), t + 1);
    // 印のない物がなくなれば、捨てた物の先頭
    let o = g.run("get");
    assert!(o.ok, "{}", o.message);
    assert_eq!(g.inventory.len(), 2);
    assert!(g.floor_items.is_empty());
    // 足元に何もなければ失敗(ターンを使わない)
    let t = g.turn();
    let o = g.run("pickup");
    assert!(!o.ok && o.message.contains("何もない"), "{}", o.message);
    assert_eq!(g.turn(), t);
}

#[test]
fn pickup_by_number_can_take_dropped_items_and_checks_range() {
    let mut g = quiet(2);
    put_underfoot(
        &mut g,
        &[ItemKind::Healing.into()],
        &[ItemKind::Sleep.into(), ItemKind::Bread.into()],
    );
    let t = g.turn();
    for bad in ["pickup 4", "pickup 99"] {
        let o = g.run(bad);
        assert!(!o.ok && o.message.contains("1〜3"), "{}", o.message);
    }
    assert_eq!(g.turn(), t);
    // 番号は表示の順 (1 印なし / 2,3 捨てた物)
    let o = g.run("pickup 3");
    assert!(o.ok, "{}", o.message);
    assert_eq!(g.inventory[0].kind, ItemKind::Bread);
    assert_eq!(g.turn(), t + 1);
    assert_eq!(g.floor_items.len(), 2);
}

#[test]
fn pickup_fails_when_the_inventory_is_full_without_spending_a_turn() {
    let mut g = quiet(2);
    for c in 'a'..='z' {
        g.inventory.push(Stack {
            letter: c,
            kind: ItemKind::Dagger,
            count: 1,
            gear: Some(Gear::plain(ItemKind::Dagger)),
            tool: None,
        });
    }
    put_underfoot(&mut g, &[ItemKind::Healing.into()], &[]);
    let t = g.turn();
    let o = g.run("pickup");
    assert!(!o.ok && o.message.contains("いっぱい"), "{}", o.message);
    assert_eq!(g.turn(), t);
    assert_eq!(g.floor_items.len(), 1);
}

#[test]
fn full_inventory_drop_then_pickup_takes_only_the_potion() {
    let mut g = quiet(2);
    for c in 'a'..='z' {
        g.inventory.push(Stack {
            letter: c,
            kind: ItemKind::Dagger,
            count: 1,
            gear: Some(Gear::plain(ItemKind::Dagger)),
            tool: None,
        });
    }
    // 1. 満杯で、足元に薬。歩いて乗っても拾えない
    let p = (g.pos.0 + 1, g.pos.1);
    g.floor_items.push(FloorItem::new(p, ItemKind::Healing));
    let o = g.run("move east");
    assert!(o.message.contains("いっぱい"), "{}", o.message);
    assert!(g.inventory.iter().all(|s| s.kind == ItemKind::Dagger));
    // 2. 不要な装備を drop して空きを作る
    assert!(g.run("drop a").ok);
    let u = g.underfoot_text();
    assert!(
        u.contains("1) ") && u.contains("2) Basic Dagger (捨てた)"),
        "{u}"
    );
    // 3. 引数なしの pickup は薬だけを拾い、捨てた装備は拾い直さない
    let o = g.run("pickup");
    assert!(o.ok, "{}", o.message);
    assert!(g.inventory.iter().any(|s| s.kind == ItemKind::Healing));
    assert_eq!(g.floor_items.len(), 1);
    assert!(g.floor_items[0].dropped);
    // stay しても捨てた装備は拾わない
    g.run("stay 2");
    assert_eq!(g.floor_items.len(), 1);
}

#[test]
fn underfoot_list_is_numbered_in_pickup_order_and_in_the_observation() {
    let mut g = quiet(2);
    assert!(g.underfoot_text().is_empty());
    assert!(!g.observe_text(3).contains("足元"));
    put_underfoot(&mut g, &[], &[Item::Gear(Gear::plain(ItemKind::Sword))]);
    put_underfoot(&mut g, &[ItemKind::Teleport.into()], &[]);
    let look = g.run("look").message;
    let name = g.looks[ItemKind::Teleport.index()];
    assert!(
        look.contains(&format!("足元: 1) {name}  2) Basic Sword (捨てた)")),
        "{look}"
    );
    let o = g.observe_text(3);
    assert!(
        o.contains(&format!("足元: 1) {name}  2) Basic Sword (捨てた)")),
        "{o}"
    );
}

#[test]
fn drop_and_pickup_are_deterministic_for_a_seed_and_script() {
    let play = |seed: u64| {
        let mut g = Game::new(seed);
        let mut out = Vec::new();
        for script in [
            "explore",
            "pickup",
            "drop a",
            "pickup 1",
            "explore",
            "drop b",
            "stay 3",
            "pickup",
            "inventory",
        ] {
            let o = g.run(script);
            out.push(format!("{} {} {} {}", o.ok, o.message, o.turn, o.hp));
        }
        out.push(g.observe_text(50));
        out
    };
    for seed in 0..6 {
        assert_eq!(play(seed), play(seed), "seed {seed}");
    }
}

#[test]
fn cataclysm_adds_one_to_every_hit_taken_even_through_armor() {
    let run = |cursed: bool| {
        let mut g = with_adjacent(3, &crate::monster::BAT);
        let a = give(
            &mut g,
            quality_gear(ItemKind::Plate, crate::item::Quality::Ancient, 5),
        );
        g.run(&format!("equip {a}"));
        if cursed {
            let w = give(&mut g, suffix_gear(ItemKind::Axe, Suffix::Cataclysm));
            g.run(&format!("equip {w}"));
        }
        let mut hits = Vec::new();
        for _ in 0..30 {
            let before = g.hp;
            g.run("wait");
            if g.hp < before {
                hits.push(before - g.hp);
            }
        }
        hits
    };
    let plain = run(false);
    let cursed = run(true);
    // 防御8 の鎧ならコウモリの攻撃は最低の 1 まで減る。呪いがあるとその最低が 2 になる
    assert!(!plain.is_empty() && !cursed.is_empty());
    assert_eq!(*plain.iter().min().unwrap(), 1);
    assert_eq!(*cursed.iter().min().unwrap(), 2);
}

#[test]
fn stay_passes_exactly_the_given_turns_and_defaults_to_one() {
    let mut g = quiet(1);
    let t = g.turn();
    let o = g.run("stay 4");
    assert!(
        o.ok && o.message.contains("4ターン留まった"),
        "{}",
        o.message
    );
    assert_eq!(g.turn(), t + 4);
    assert_eq!(o.command, "stay 4");
    let o = g.run("stay");
    assert!(o.ok);
    assert_eq!(o.command, "stay 1");
    assert_eq!(g.turn(), t + 5);
    assert!(!g.run("stay 0").ok);
    assert_eq!(g.turn(), t + 5);
    // 記録にはコロンやバッククォートが付かない
    assert_eq!(g.run("`stay 2").command, "stay 2");
}

#[test]
fn stay_stops_when_attacked() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    let t = g.turn();
    let o = g.run("stay 10");
    assert!(
        o.message
            .contains("1ターン留まったところで、攻撃を受けて中断した"),
        "{}",
        o.message
    );
    assert_eq!(g.turn(), t + 1);
}

/// `stay` の結果メッセージ（「Nターン留まった…」）から N を取り出す。
fn reported_stay_turns(message: &str) -> u32 {
    let head = message.split("ターン留まった").next().unwrap();
    head.chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("ターン数が読めない: {message}"))
}

#[test]
fn stay_advances_exactly_the_reported_number_of_turns() {
    // 敵のいる本物のゲームで何度も stay する。中断されても、されなくても、
    // 進んだターン数は報告された数と一致し、指定数を超えない
    for seed in 0..40 {
        let mut g = Game::new(seed);
        for _ in 0..8 {
            if g.is_dead() {
                break;
            }
            let t = g.turn();
            let o = g.run("stay 4");
            assert!(o.ok, "seed {seed}: {}", o.message);
            let n = reported_stay_turns(&o.message);
            assert!((1..=4).contains(&n), "seed {seed}: {}", o.message);
            assert_eq!(g.turn() - t, n, "seed {seed}: {}", o.message);
        }
    }
}

#[test]
fn interrupted_stay_counts_the_interrupting_turn_once() {
    // 攻撃を受けた回のターンは「留まったターン」に1回だけ数える
    for limit in [1, 2, 5, 10] {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        let t = g.turn();
        let o = g.run(&format!("stay {limit}"));
        let n = reported_stay_turns(&o.message);
        assert!(o.message.contains("攻撃を受けて中断した"), "{}", o.message);
        assert_eq!(g.turn() - t, n, "limit {limit}: {}", o.message);
        assert!(n <= limit);
    }
}

#[test]
fn failed_stay_spends_no_turn() {
    let mut g = quiet(1);
    let t = g.turn();
    for bad in ["stay 0", "stay -1", "stay many", "stay 1001"] {
        assert!(!g.run(bad).ok, "{bad}");
    }
    assert_eq!(g.turn(), t);
}

#[test]
fn stay_picks_up_the_item_underfoot_but_wait_does_not() {
    let mut g = quiet(1);
    let here = g.pos;
    g.floor_items.push(FloorItem::new(here, ItemKind::Healing));
    let o = g.run("wait");
    assert!(
        o.ok && g.inventory.is_empty() && g.floor_items.len() == 1,
        "{}",
        o.message
    );
    let t = g.turn();
    let o = g.run("stay 2");
    assert!(o.message.contains("を拾った"), "{}", o.message);
    assert!(g.floor_items.is_empty());
    assert_eq!(g.inventory.len(), 1);
    assert_eq!(g.turn(), t + 2); // 拾うのにターンは使わない
                                 // 何もなければ、ただ留まるだけ
    let o = g.run("stay");
    assert!(!o.message.contains("拾った"), "{}", o.message);
}

/// 開始位置の東 `dist` の床が見えている seed を探す。
fn seed_with_visible_floor_east(dist: i32) -> (Game, (i32, i32)) {
    for seed in 0..200 {
        let mut g = quiet(seed);
        g.map.update_fov(g.pos, FOV_RADIUS);
        let p = (g.pos.0 + dist, g.pos.1);
        if g.map.tile(p.0, p.1).walkable() && g.map.is_visible(p.0, p.1) {
            return (g, p);
        }
    }
    panic!("条件に合う seed がなかった");
}

#[test]
fn stay_is_not_stopped_by_an_enemy_that_is_merely_in_view() {
    // 遠くに見えているだけのオーガ(2ターンに1回しか動けない)は、2ターンの間は届かない
    let (mut g, p) = seed_with_visible_floor_east(6);
    g.monsters.push(monster(&crate::monster::OGRE, p, 1000));
    let t = g.turn();
    let o = g.run("stay 2");
    assert!(o.message.contains("2ターン留まった。"), "{}", o.message);
    assert_eq!(g.turn(), t + 2);
}

#[test]
fn hunger_grows_warns_and_then_hurts() {
    let mut g = quiet(1);
    g.food = HUNGRY_AT + 1;
    let o = g.run("wait");
    assert!(o.message.contains("お腹が空いてきた"), "{}", o.message);
    assert!(g.status_text().contains("空腹"));
    g.food = WEAK_AT + 1;
    assert!(g.run("wait").message.contains("ひどく空腹"));
    g.food = 1;
    let o = g.run("wait");
    assert!(o.message.contains("飢えて体力が削られ"), "{}", o.message);
    // 飢餓の間は毎ターン1ダメージで、自然回復もしない
    let hp = g.hp();
    for _ in 0..REGEN_INTERVAL {
        g.run("wait");
    }
    assert_eq!(g.hp(), hp - REGEN_INTERVAL as i32);
    assert!(g.observe_text(3).contains("飢餓"));
}

#[test]
fn starvation_can_kill() {
    let mut g = quiet(1);
    g.food = 0;
    g.hp = 2;
    let o = g.run("wait");
    assert!(o.ok);
    let o = g.run("wait");
    assert!(g.is_dead(), "{}", o.message);
    assert!(o.message.contains("飢えで力尽きた"), "{}", o.message);
}

#[test]
fn auto_walk_stops_when_hunger_sets_in() {
    let mut g = quiet(2);
    g.food = HUNGRY_AT + 3;
    let o = g.run("explore");
    assert!(o.ok);
    assert!(o.message.contains("お腹が空いて"), "{}", o.message);
    assert!(g.turn() <= 5, "{}", g.turn());
}

#[test]
fn poison_hurts_each_turn_blocks_regen_and_wears_off() {
    let mut g = quiet(1);
    g.hp = 10;
    g.status.apply(Status::Poisoned, 3);
    let mut text = String::new();
    for _ in 0..3 {
        text.push_str(&g.run("wait").message);
    }
    assert_eq!(g.hp(), 7);
    assert!(text.contains("毒で1ダメージ"), "{text}");
    assert!(text.contains("毒が抜けた"), "{text}");
    assert_eq!(g.poison(), 0);
    // 毒の間は自然回復しない
    g.hp = 20;
    g.status.apply(Status::Poisoned, 15);
    let hp = g.hp();
    for _ in 0..REGEN_INTERVAL {
        g.run("wait");
    }
    assert_eq!(g.hp(), hp - REGEN_INTERVAL as i32);
}

#[test]
fn poison_can_kill_and_healing_potion_cures_it() {
    let mut g = quiet(1);
    g.hp = 1;
    g.status.apply(Status::Poisoned, 5);
    let o = g.run("wait");
    assert!(
        g.is_dead() && o.message.contains("毒で力尽きた"),
        "{}",
        o.message
    );

    let mut g = with_gear(&[ItemKind::Healing]);
    g.status.apply(Status::Poisoned, 9);
    let o = g.run("quaff a");
    assert!(o.message.contains("毒が抜けた"), "{}", o.message);
    assert_eq!(g.poison(), 0);
}

#[test]
fn antidote_cures_poison_and_is_harmless_otherwise() {
    let mut g = with_gear(&[ItemKind::Antidote, ItemKind::Antidote]);
    g.status.apply(Status::Poisoned, 9);
    let o = g.run("quaff a");
    assert!(o.ok && o.message.contains("毒が抜けた"), "{}", o.message);
    assert_eq!(g.poison(), 0);
    let o = g.run("quaff a");
    assert!(
        o.ok && o.message.contains("毒にはかかっていなかった"),
        "{}",
        o.message
    );
}

#[test]
fn experience_potion_grants_xp_and_can_level_up() {
    let mut g = with_gear(&[ItemKind::Experience]);
    let (level, max_hp) = (g.level(), g.max_hp);
    let o = g.run("quaff a");
    assert!(
        o.ok && o.message.contains("レベルが上がった"),
        "{}",
        o.message
    );
    assert_eq!(g.level(), level + 1);
    // 次のレベルに届いたちょうどの経験値になる
    assert_eq!(g.xp(), 5 * level * (level + 1));
    assert_eq!(g.max_hp, max_hp + HP_PER_LEVEL);
}

#[test]
fn bad_potions_are_at_most_a_third_of_potion_weight() {
    let potions: Vec<ItemKind> = ItemKind::ALL
        .into_iter()
        .filter(|k| k.is_potion())
        .collect();
    let total: u32 = potions.iter().map(|k| k.weight()).sum();
    let bad: u32 = potions
        .iter()
        .filter(|k| k.is_bad())
        .map(|k| k.weight())
        .sum();
    assert!(bad * 3 <= total, "{bad}/{total}");
    assert!(potions.len() <= crate::item::POTION_LOOKS.len());
}

#[test]
fn poison_stops_auto_walk_when_hp_is_low() {
    let mut g = quiet(2);
    g.hp = DANGER_HP + 2;
    g.status.apply(Status::Poisoned, 10);
    let o = g.run("explore");
    assert!(o.message.contains("体力が危ない"), "{}", o.message);
    assert!(!g.is_dead());
}

#[test]
fn bread_feeds_and_is_sometimes_rotten() {
    let (mut rotten, mut fine) = (0, 0);
    for seed in 0..60 {
        let mut g = with_gear(&[ItemKind::Bread]);
        g.rng = Rng::new(seed);
        g.food = 100;
        let o = g.run("eat a");
        assert!(o.ok);
        if o.message.contains("腐っていた") {
            rotten += 1;
            assert!(g.poison() > 0 && g.food < 200, "{}", o.message);
        } else {
            fine += 1;
            assert!(g.food >= 240, "{}", o.message); // 100 + 150 - 1ターン
        }
    }
    assert!(rotten > 0 && fine > rotten, "{rotten} {fine}");
}

#[test]
fn eating_is_capped_at_full() {
    let mut g = with_gear(&[ItemKind::Jerky]);
    g.food = MAX_FOOD - 10;
    let o = g.run("eat a");
    assert!(o.message.contains("満腹度が10回復"), "{}", o.message);
    assert!(g.food <= MAX_FOOD);
}

#[test]
fn mushrooms_are_unidentified_until_eaten() {
    let g = with_gear(&[
        ItemKind::PoisonShroom,
        ItemKind::VigorShroom,
        ItemKind::EdibleShroom,
    ]);
    let lines = g.inventory_lines();
    assert!(
        lines
            .iter()
            .all(|l| l.contains("キノコ") && l.contains("未識別")),
        "{lines:?}"
    );
    assert!(lines
        .iter()
        .all(|l| !l.contains("毒キノコ") && !l.contains("元気")));
    let mut g = with_gear(&[ItemKind::PoisonShroom]);
    let o = g.run("eat a");
    assert!(o.message.contains("これは毒キノコだった"), "{}", o.message);
    assert!(g.poison() >= 7, "{}", g.poison());
    let mut g = with_gear(&[ItemKind::VigorShroom]);
    g.hp = 5;
    let o = g.run("eat a");
    assert!(
        o.message.contains("これは元気キノコだった") && g.hp() >= 12,
        "{}",
        o.message
    );
    assert!(g.known[ItemKind::VigorShroom.index()]);
}

#[test]
fn spider_bites_can_poison() {
    let mut g = with_adjacent(2, &crate::monster::SPIDER);
    let mut saw = false;
    for _ in 0..40 {
        let o = g.run("wait");
        if o.message.contains("毒を受けた！") {
            saw = true;
            assert!(g.poison() > 0);
            break;
        }
    }
    assert!(saw, "毒グモに噛まれても毒にならなかった");
}

#[test]
fn every_floor_has_food() {
    for seed in 0..40 {
        let mut g = Game::new(seed);
        for depth in 1..=5u32 {
            g.depth = depth;
            g.spawn_items();
            assert!(
                g.floor_items.iter().any(|f| f.item.kind().is_food()),
                "seed {seed} depth {depth}"
            );
        }
    }
}

#[test]
fn spawn_respects_min_depth() {
    for seed in 0..30 {
        for depth in 1..=6u32 {
            let mut g = Game::new(seed);
            g.depth = depth;
            g.spawn_monsters();
            for m in &g.monsters {
                assert!(
                    m.kind.min_depth <= depth,
                    "seed {seed} depth {depth} {}",
                    m.name
                );
                assert_eq!(m.hp, m.kind.hp_at(depth));
            }
        }
    }
}

#[test]
fn shallow_floors_have_only_slimes_and_bats_and_deep_floors_have_all() {
    let mut seen = std::collections::HashSet::new();
    let mut seen_shallow = std::collections::HashSet::new();
    for seed in 0..60 {
        let mut g = Game::new(seed);
        g.depth = 1;
        g.spawn_monsters();
        seen_shallow.extend(g.monsters.iter().map(|m| m.glyph));
        g.depth = 6;
        g.spawn_monsters();
        seen.extend(g.monsters.iter().map(|m| m.glyph));
    }
    assert!(
        seen_shallow.iter().all(|c| *c == 's' || *c == 'b'),
        "{seen_shallow:?}"
    );
    for c in ['s', 'b', 'g', 'O'] {
        assert!(seen.contains(&c), "{c} が現れなかった: {seen:?}");
    }
}

#[test]
fn bat_acts_twice_per_turn_and_flutters() {
    let mut double = false;
    let mut missed_turns = 0;
    for seed in 0..40 {
        let mut g = with_adjacent(seed, &crate::monster::BAT);
        for _ in 0..10 {
            let o = g.run("wait");
            let n = o.message.matches("コウモリの攻撃").count();
            assert!(n <= 2, "{}", o.message);
            if n == 2 {
                double = true;
            }
            if n == 0 {
                missed_turns += 1;
            }
        }
    }
    assert!(double, "2回攻撃が一度もなかった");
    assert!(
        missed_turns > 0,
        "ふらふら動くはずなのに毎ターン攻撃してきた"
    );
}

#[test]
fn ogre_acts_every_other_turn_and_hits_hard() {
    let mut g = with_adjacent(3, &crate::monster::OGRE);
    let mut attacks = 0;
    for _ in 0..10 {
        let before = g.hp;
        let o = g.run("wait");
        if o.message.contains("オーガの攻撃") {
            attacks += 1;
            let dmg = before - g.hp;
            assert!((3..=6).contains(&dmg), "dmg {dmg}");
        } else {
            assert_eq!(before, g.hp);
        }
    }
    assert_eq!(attacks, 5);
}

#[test]
fn goblin_hits_for_at_least_two() {
    let mut g = with_adjacent(5, &crate::monster::GOBLIN);
    let mut attacks = 0;
    for _ in 0..30 {
        let before = g.hp;
        let o = g.run("wait");
        let dmg = before - g.hp;
        if o.message.contains("ゴブリンの攻撃") {
            attacks += 1;
            assert!((2..=4).contains(&dmg), "dmg {dmg}");
        }
    }
    assert_eq!(attacks, 30);
}

#[test]
fn same_seed_same_world() {
    let a = Game::new(42);
    let b = Game::new(42);
    assert_eq!(a.pos(), b.pos());
    assert_eq!(a.map_lines(), b.map_lines());
    assert_eq!(a.monsters.len(), b.monsters.len());
}

#[test]
fn monsters_spawn_away_from_start() {
    for seed in 0..20 {
        let g = Game::new(seed);
        assert!(!g.monsters.is_empty());
        for m in &g.monsters {
            let (dx, dy) = (m.pos.0 - g.pos.0, m.pos.1 - g.pos.1);
            assert!(dx * dx + dy * dy >= 64, "seed {seed}");
        }
    }
}

#[test]
fn wall_costs_no_turn() {
    let mut g = Game::new(1);
    g.monsters.clear();
    // 壁にぶつかるまで西へ進む
    for _ in 0..100 {
        let o = g.run("move west");
        if !o.ok {
            let t = g.turn();
            assert!(!g.run("move west").ok);
            assert_eq!(g.turn(), t);
            return;
        }
    }
    panic!("壁にぶつからなかった");
}

#[test]
fn explore_finds_stairs_then_descend() {
    for seed in 0..30 {
        let mut g = Game::new(seed);
        g.monsters.clear();
        g.traps.clear();
        let mut found = false;
        for _ in 0..50 {
            let o = g.run("explore");
            assert!(o.ok);
            if o.message.contains("階段を見つけた") {
                found = true;
                break;
            }
            if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                break;
            }
        }
        assert!(found, "seed {seed}: 階段が見つからなかった");
        // 空腹の知らせなどで途中で止まることがあるので、着くまで繰り返す
        for _ in 0..5 {
            let o = g.run("travel >");
            assert!(o.ok, "seed {seed}: {}", o.message);
            if o.message.contains("階段まで") || o.message.contains("すでに階段") {
                break;
            }
        }
        let o = g.run("descend");
        assert!(o.ok, "seed {seed}: {}", o.message);
        assert_eq!(g.depth(), 2);
    }
}

#[test]
fn script_stops_on_failure() {
    let mut g = Game::new(3);
    let outs = g.run_script("descend; wait");
    assert_eq!(outs.len(), 1);
    assert!(!outs[0].ok);
}

#[test]
fn bump_attack_kills_and_the_slime_hits_back() {
    let mut g = with_adjacent_slime(1, 5);
    // 5 HP に対して与えるダメージは 2〜4 なので、最初の一撃では倒れず反撃される
    let first = g.run("move east");
    assert!(first.ok);
    assert!(first.message.contains("ダメージを与えた"));
    assert!(first.message.contains("攻撃！"));
    assert!(g.hp() < g.max_hp());
    for _ in 0..10 {
        if g.monsters.is_empty() {
            break;
        }
        assert!(g.run("attack east").ok);
    }
    assert!(g.monsters.is_empty());
    assert!(!g.is_dead());
}

#[test]
fn a_visible_slime_always_closes_in_and_attacks() {
    let mut checked = 0;
    for seed in 0..15 {
        let mut g = quiet(seed);
        // 見えていて、3マス離れた歩ける場所を探す
        let spot = (-3..=3)
            .flat_map(|dy| (-3..=3).map(move |dx| (dx, dy)))
            .filter(|(dx, dy): &(i32, i32)| dx.abs().max(dy.abs()) == 3)
            .map(|(dx, dy)| (g.pos.0 + dx, g.pos.1 + dy))
            .find(|p| g.map.tile(p.0, p.1).walkable() && g.map.is_visible(p.0, p.1));
        let Some(spot) = spot else { continue };
        g.monsters.push(slime(spot, 50));
        for _ in 0..6 {
            g.run("wait");
        }
        assert!(g.hp() < g.max_hp(), "seed {seed}: 近づいてこなかった");
        checked += 1;
    }
    assert!(checked >= 5);
}

#[test]
fn attack_needs_a_target() {
    let mut g = Game::new(1);
    g.monsters.clear();
    let t = g.turn();
    let o = g.run("attack east");
    assert!(!o.ok);
    assert_eq!(g.turn(), t);
}

#[test]
fn death_ends_the_game() {
    let mut g = with_adjacent_slime(1, 5);
    g.hp = 1;
    let o = g.run("wait");
    assert!(g.is_dead());
    assert!(o.message.contains("力尽きた"));
    let o = g.run("wait");
    assert!(!o.ok);
    assert!(o.message.contains("ゲームオーバー"));
}

#[test]
fn explore_and_travel_refuse_while_an_enemy_is_visible() {
    let mut g = with_adjacent_slime(1, 5);
    let o = g.run("explore");
    assert!(!o.ok);
    assert!(o.message.contains("敵が見えている"));
    let o = g.run("travel >");
    assert!(!o.ok);
}

#[test]
fn hp_regenerates_when_no_enemy_is_around() {
    let mut g = Game::new(1);
    g.monsters.clear();
    g.hp = 10;
    for _ in 0..REGEN_INTERVAL {
        g.run("wait");
    }
    assert_eq!(g.hp(), 11);
}

#[test]
fn observation_lists_visible_enemies() {
    let g = with_adjacent_slime(1, 5);
    let text = g.observe_text(5);
    assert!(text.contains("-- 見えている敵 --"));
    assert!(text.contains("スライム HP 5/5 (東に1)"));
    assert!(text.contains(&format!("HP {}/{}", g.hp(), g.max_hp())));
}

/// 敵もアイテムもいない状態のゲーム。
fn quiet(seed: u64) -> Game {
    let mut g = Game::new(seed);
    g.monsters.clear();
    g.floor_items.clear();
    g.traps.clear();
    g
}

#[test]
fn looks_are_unique_and_stable_per_seed() {
    let a = Game::new(9);
    let b = Game::new(9);
    assert_eq!(a.looks, b.looks);
    for k in ItemKind::ALL {
        assert_eq!(a.known[k.index()], k.starts_known(), "{k:?}");
    }
    let groups: [fn(ItemKind) -> bool; 3] =
        [|k| k.is_potion(), |k| k.is_scroll(), |k| k.is_mushroom()];
    for in_group in groups {
        let names: Vec<_> = ItemKind::ALL
            .iter()
            .filter(|k| in_group(**k))
            .map(|k| a.looks[k.index()])
            .collect();
        for (i, x) in names.iter().enumerate() {
            assert!(!x.is_empty());
            for y in &names[i + 1..] {
                assert_ne!(x, y);
            }
        }
    }
    // ゲームによって対応が変わる
    let differs = (0..20).any(|seed| Game::new(seed).looks != a.looks);
    assert!(differs);
}

#[test]
fn items_spawn_on_floor_tiles() {
    for seed in 0..20 {
        let g = Game::new(seed);
        assert!(!g.floor_items.is_empty());
        for f in &g.floor_items {
            let p = &f.pos;
            assert_eq!(g.map.tile(p.0, p.1), Tile::Floor);
            assert_ne!(*p, g.pos);
        }
    }
}

#[test]
fn walking_onto_an_item_picks_it_up() {
    let mut g = quiet(1);
    let p = (g.pos.0 + 1, g.pos.1);
    g.floor_items.push(FloorItem::new(p, ItemKind::Healing));
    let o = g.run("move east");
    assert!(o.ok && o.message.contains("拾った"), "{}", o.message);
    assert!(g.floor_items.is_empty());
    assert_eq!(g.inventory.len(), 1);
    assert_eq!(g.inventory[0].letter, 'a');
    assert_eq!(g.inventory[0].kind, ItemKind::Healing);
}

#[test]
fn same_kind_stacks_and_letters_are_stable() {
    let mut g = quiet(1);
    assert_eq!(g.take(ItemKind::Poison), Some('a'));
    assert_eq!(g.take(ItemKind::Healing), Some('b'));
    assert_eq!(g.take(ItemKind::Poison), Some('a'));
    assert_eq!(g.inventory[0].count, 2);
    // a を使い切っても b の文字は変わらない
    g.known[ItemKind::Poison.index()] = true;
    g.hp = 20;
    g.run("quaff a");
    g.run("quaff a");
    assert_eq!(g.inventory.len(), 1);
    assert_eq!(g.inventory[0].letter, 'b');
}

#[test]
fn healing_potion_heals_and_identifies() {
    let mut g = quiet(1);
    g.take(ItemKind::Healing);
    g.take(ItemKind::Healing);
    g.hp = 5;
    let o = g.run("quaff a");
    assert!(o.ok);
    assert_eq!(g.hp(), 15);
    assert!(g.known[ItemKind::Healing.index()]);
    assert!(o.message.contains("回復の薬だった"), "{}", o.message);
    let o = g.run("quaff a");
    assert!(o.ok);
    assert!(!o.message.contains("だった！"));
    assert_eq!(g.hp(), 20);
    assert!(g.inventory.is_empty());
    assert!(!g.run("quaff a").ok);
}

#[test]
fn poison_hurts_and_can_kill() {
    let mut g = quiet(1);
    g.take(ItemKind::Poison);
    let o = g.run("quaff a");
    assert!(o.ok);
    assert_eq!(g.hp(), 15);

    let mut g = quiet(1);
    g.take(ItemKind::Poison);
    g.hp = 5;
    let o = g.run("quaff a");
    assert!(g.is_dead());
    assert!(o.message.contains("ゲームオーバー"));
}

#[test]
fn sleeping_passes_turns_while_an_enemy_attacks() {
    let mut g = with_adjacent_slime(1, 50);
    g.floor_items.clear();
    g.take(ItemKind::Sleep);
    let t = g.turn();
    let o = g.run("quaff a");
    assert!(o.ok);
    assert_eq!(g.turn(), t + 5);
    assert!(g.hp() < g.max_hp());
    assert!(o.message.contains("目が覚めた"), "{}", o.message);
}

#[test]
fn identify_scroll_reveals_another_item() {
    let mut g = quiet(1);
    g.take(ItemKind::Healing); // a
    g.take(ItemKind::Identify); // b
    let o = g.run("read b");
    assert!(o.ok, "{}", o.message);
    assert!(g.known[ItemKind::Healing.index()]);
    assert!(g.known[ItemKind::Identify.index()]);
    assert!(o.message.contains("回復の薬だと分かった"), "{}", o.message);
    assert_eq!(g.inventory.len(), 1);
    assert_eq!(g.inventory[0].kind, ItemKind::Healing);
}

#[test]
fn identify_scroll_with_an_explicit_target() {
    let mut g = quiet(1);
    g.take(ItemKind::Healing); // a
    g.take(ItemKind::Poison); // b
    g.take(ItemKind::Identify); // c
    let o = g.run("read c b");
    assert!(o.ok, "{}", o.message);
    assert!(g.known[ItemKind::Poison.index()]);
    assert!(!g.known[ItemKind::Healing.index()]);
    // すでに識別済みの対象は選べない
    g.take(ItemKind::Identify);
    let o = g.run("read c b");
    assert!(!o.ok);
}

#[test]
fn identify_scroll_without_targets() {
    // 正体を知らない巻物は、読むと消費して正体だけ分かる
    let mut g = quiet(1);
    g.take(ItemKind::Identify);
    let o = g.run("read a");
    assert!(o.ok);
    assert!(o.message.contains("何も起こらなかった"));
    assert!(g.inventory.is_empty());
    // 正体を知っている巻物は、対象がなければ消費せず失敗する
    let mut g = quiet(1);
    g.take(ItemKind::Identify);
    g.known[ItemKind::Identify.index()] = true;
    let t = g.turn();
    let o = g.run("read a");
    assert!(!o.ok);
    assert_eq!(g.inventory.len(), 1);
    assert_eq!(g.turn(), t);
}

#[test]
fn teleport_moves_the_player() {
    let mut g = quiet(1);
    g.take(ItemKind::Teleport);
    let old = g.pos;
    assert!(g.run("read a").ok);
    assert_ne!(g.pos, old);
    assert!(g.map.tile(g.pos.0, g.pos.1).walkable());
}

#[test]
fn magic_map_reveals_the_stairs() {
    let mut checked = 0;
    for seed in 0..20 {
        let mut g = quiet(seed);
        if g.map.is_seen(g.stairs.0, g.stairs.1) {
            continue;
        }
        g.take(ItemKind::MagicMap);
        assert!(g.run("read a").ok);
        assert!(g.map.is_seen(g.stairs.0, g.stairs.1));
        let o = g.run("travel >");
        assert!(o.ok, "seed {seed}: {}", o.message);
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn explore_collects_every_item_on_the_floor() {
    for seed in 0..10 {
        let mut g = Game::new(seed);
        g.monsters.clear();
        g.traps.clear();
        let n = g.floor_items.len();
        assert!(n > 0);
        for _ in 0..200 {
            let o = g.run("explore");
            if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                break;
            }
        }
        assert!(g.floor_items.is_empty(), "seed {seed}");
        let total: u32 = g.inventory.iter().map(|s| s.count).sum();
        assert_eq!(total as usize, n, "seed {seed}");
    }
}

#[test]
fn inventory_command_and_observation() {
    let mut g = quiet(1);
    let o = g.run("inventory");
    assert!(o.ok && o.message.contains("持ち物はない"));
    g.take(ItemKind::Healing);
    let o = g.run("inventory");
    assert!(o.message.contains("a) "));
    assert!(o.message.contains(g.looks[ItemKind::Healing.index()]));
    let text = g.observe_text(5);
    assert!(text.contains("-- 持ち物 --"));
    assert!(text.contains("(未識別)"));
}

#[test]
fn look_mentions_known_floor_items() {
    let mut g = quiet(1);
    let p = (g.pos.0 + 2, g.pos.1);
    g.floor_items.push(FloorItem::new(p, ItemKind::Teleport));
    g.map.update_fov(g.pos, FOV_RADIUS);
    let o = g.run("look");
    assert!(o.message.contains("東に2"), "{}", o.message);
    assert!(g.observe_text(1).contains('?'));
}

#[test]
fn dying_from_a_poison_potion_does_not_advance_another_turn() {
    let mut g = with_gear(&[ItemKind::Poison]);
    g.hp = 3;
    g.status.apply(Status::Poisoned, 4);
    let t = g.turn();
    let o = g.run("quaff a");
    assert!(g.is_dead());
    assert_eq!(g.turn(), t, "{}", o.message);
    assert_eq!(
        o.message.matches("ゲームオーバー").count(),
        1,
        "{}",
        o.message
    );
}

#[test]
fn full_inventory_says_so_and_explore_does_not_chase_items() {
    let mut g = quiet(3);
    for (i, c) in ('a'..='z').enumerate() {
        let kind = if i == 0 {
            ItemKind::Dagger
        } else {
            ItemKind::Bread
        };
        g.inventory.push(Stack {
            letter: c,
            kind,
            count: 1,
            gear: kind.is_equipment().then(|| Gear::plain(kind)),
            tool: None,
        });
    }
    assert!(!g.can_take(ItemKind::Healing));
    assert!(g.can_take(ItemKind::Bread));
    g.map.reveal_all();
    let far = |g: &Game, d: i32| {
        *g.find_path(&|q| {
            g.map.tile(q.0, q.1) == Tile::Floor && (q.0 - g.pos.0).abs() + (q.1 - g.pos.1).abs() > d
        })
        .unwrap()
        .last()
        .unwrap()
    };
    let (a, b) = (far(&g, 4), far(&g, 12));
    g.floor_items.push(FloorItem::new(a, ItemKind::Healing));
    g.floor_items.push(FloorItem::new(b, ItemKind::Healing));
    let o = g.run("explore");
    assert!(
        o.message.contains("もう探索する場所がない"),
        "{}",
        o.message
    );
    assert_eq!(g.turn(), 0);
    // 踏めば、拾えないと分かる
    g.floor_items.clear();
    g.floor_items.push(FloorItem::new(g.pos, ItemKind::Healing));
    let o = g.run("stay");
    assert!(o.message.contains("持ち物がいっぱい"), "{}", o.message);
    assert_eq!(g.floor_items.len(), 1);
}

/// 最深部で、アミュレットのある床を探して持たせる。
fn deep_game_with_amulet() -> Game {
    let mut g = quiet(5);
    g.depth = AMULET_DEPTH;
    g.spawn_items();
    g
}

#[test]
fn amulet_only_on_the_deepest_floor_and_only_once() {
    let mut g = quiet(5);
    for d in 1..AMULET_DEPTH {
        g.depth = d;
        g.spawn_items();
        assert!(g.amulet.is_none(), "depth {d}");
    }
    let mut g = deep_game_with_amulet();
    assert!(g.amulet.is_some());
    g.has_amulet = true;
    g.spawn_items();
    assert!(g.amulet.is_none());
}

#[test]
fn picking_up_the_amulet_turns_the_stairs_upward() {
    let mut g = deep_game_with_amulet();
    let a = g.amulet.unwrap();
    g.pos = a;
    g.map.reveal_all();
    g.run("stay");
    assert!(g.has_amulet && g.amulet.is_none());
    assert!(g
        .inventory_lines()
        .iter()
        .any(|l| l.contains("アミュレット")));
    assert!(g.status_text().contains("アミュレット"));
    let (sx, sy) = g.stairs;
    assert_eq!(g.cell(sx, sy).ch, '<');
    g.pos = g.stairs;
    let o = g.run("descend");
    assert!(!o.ok && o.message.contains("登り階段"), "{}", o.message);
    let o = g.run("ascend");
    assert!(o.ok && g.depth() == AMULET_DEPTH - 1, "{}", o.message);
    assert!(g.amulet.is_none());
}

#[test]
fn cannot_ascend_without_the_amulet_or_descend_past_the_bottom() {
    let mut g = quiet(5);
    g.pos = g.stairs;
    assert!(!g.run("ascend").ok);
    g.depth = AMULET_DEPTH;
    let o = g.run("descend");
    assert!(!o.ok && o.message.contains("最深部"), "{}", o.message);
    assert_eq!(g.depth(), AMULET_DEPTH);
}

#[test]
fn escaping_from_depth_one_with_the_amulet_wins() {
    let mut g = quiet(5);
    g.has_amulet = true;
    g.pos = g.stairs;
    let t = g.turn();
    let o = g.run("ascend");
    assert!(o.ok && g.is_won() && !g.is_dead(), "{}", o.message);
    assert!(o.message.contains("クリア"));
    assert_eq!(g.turn(), t);
    assert!(!g.run("wait").ok);
}

#[test]
fn explore_goes_for_a_seen_amulet() {
    let mut g = deep_game_with_amulet();
    g.map.reveal_all();
    let o = g.run("explore");
    assert!(g.has_amulet, "{}", o.message);
    assert!(o.message.contains("アミュレット"), "{}", o.message);
}

#[test]
fn killing_gives_xp_and_levels_up_with_more_hp_and_attack() {
    let mut g = with_adjacent_slime(3, 1);
    g.floor_items.clear();
    g.hp = 10;
    assert_eq!((g.level(), g.xp_for_next()), (1, 10));
    // スライム(基本3)を3体倒すと 9、4体目で 12 ≥ 10 → レベル2
    let mut text = String::new();
    for n in 0..4 {
        g.monsters.clear();
        g.monsters.push(slime((g.pos.0 + 1, g.pos.1), 1));
        let o = g.run("attack east");
        text.push_str(&o.message);
        assert_eq!(g.level(), if n < 3 { 1 } else { 2 }, "{n} {}", o.message);
    }
    assert!(
        text.contains("経験値 +3") && text.contains("レベルが上がった！ Lv2"),
        "{text}"
    );
    assert_eq!(g.max_hp(), PLAYER_MAX_HP + HP_PER_LEVEL);
    assert!(g.hp() >= 10 + HP_PER_LEVEL - 4, "{}", g.hp());
    assert!(g.observe_text(3).contains("Lv2"));
}

#[test]
fn attack_grows_every_two_levels_and_level_up_stops_auto_walk() {
    let mut g = quiet(2);
    assert_eq!(g.attack_range(), (2, 4));
    g.level = 3;
    assert_eq!(g.attack_range(), (3, 5));
    g.level = 1;
    g.xp = g.xp_for_next() - 1;
    g.gain_xp(1);
    assert_eq!(g.level(), 2);
    assert!(g.alert.is_some());
    // 一気に複数レベル上がることもある
    g.gain_xp(1000);
    assert!(g.level() > 4 && g.xp() < g.xp_for_next());
}

// ---- 状態異常 ----

#[test]
fn confusion_sometimes_sends_you_the_wrong_way_and_a_stumble_costs_a_turn() {
    let mut g = quiet(1);
    g.status.apply(Status::Confused, 1000);
    let (mut reeled, mut straight) = (0, 0);
    for _ in 0..60 {
        let before = g.turn();
        let o = g.run("move east");
        if o.message.contains("混乱して") {
            reeled += 1;
            // 向きがずれたときは、壁にぶつかってもターンを使って成功扱い
            assert!(o.ok && g.turn() > before, "{}", o.message);
        } else {
            straight += 1;
        }
    }
    assert!(reeled > 10 && straight > 10, "{reeled} {straight}");
    // 混乱していなければ、ずれない
    let mut g = quiet(1);
    for _ in 0..30 {
        assert!(!g.run("move east").message.contains("混乱して"));
    }
}

#[test]
fn confusion_and_blindness_forbid_auto_walk() {
    let mut g = quiet(1);
    g.status.apply(Status::Confused, 5);
    let o = g.run("explore");
    assert!(!o.ok && o.message.contains("混乱"), "{}", o.message);
    assert!(!g.run("travel >").ok);
    let mut g = quiet(1);
    g.status.apply(Status::Blind, 5);
    let o = g.run("explore");
    assert!(!o.ok && o.message.contains("見え"), "{}", o.message);
}

#[test]
fn blindness_hides_the_map_and_enemies_and_says_so() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    let seen_before = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| g.map.is_seen(x, y))
        .count();
    g.inflict(Status::Blind, 30);
    let text = g.observe_text(5);
    assert!(text.contains("目が見えない"), "{text}");
    assert!(text.contains("盲目"), "{text}");
    // マップの行も、敵の一覧も出ない
    assert!(!text.contains('@') || !text.contains("###"), "{text}");
    assert!(!text.contains("ゴブリン"), "{text}");
    assert!(g.visible_enemies().is_empty());
    // 襲われても正体は分からない
    let o = g.run("wait");
    assert!(o.message.contains("何かの攻撃！"), "{}", o.message);
    assert!(!o.message.contains("ゴブリン"), "{}", o.message);
    // 殴られた向きは分かる
    assert!(o.message.contains("(東から)"), "{}", o.message);
    // 見えない相手のHPは分からない
    let o = g.run("attack east");
    assert!(
        o.message.contains("手応えがあった") && !o.message.contains("与えた。(HP"),
        "{}",
        o.message
    );
    // 手探りで動いても、新しい場所は覚えない
    for _ in 0..5 {
        g.run("move west");
    }
    let seen_after = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| g.map.is_seen(x, y))
        .count();
    assert_eq!(seen_before, seen_after);
    assert!(g.run("look").message.contains("目が見えない"));
}

#[test]
fn sight_returns_when_blindness_ends() {
    let mut g = quiet(2);
    g.inflict(Status::Blind, 3);
    assert!(!g.map.is_visible(g.pos.0, g.pos.1));
    for _ in 0..3 {
        g.run("wait");
    }
    assert!(!g.status.has(Status::Blind));
    assert!(g.map.is_visible(g.pos.0, g.pos.1));
    assert!(g
        .log()
        .iter()
        .any(|l| l.text.contains("目が見えるようになった")));
}

#[test]
fn hallucination_scrambles_names_and_glyphs_but_not_hp_or_position() {
    let mut g = with_adjacent(3, &crate::monster::OGRE);
    g.monsters[0].hp = 7;
    g.monsters[0].max_hp = 10;
    g.inflict(Status::Hallucinating, 1000);
    let mut names = std::collections::HashSet::new();
    let mut glyphs = std::collections::HashSet::new();
    for _ in 0..40 {
        let e = &g.visible_enemies()[0];
        assert_eq!((e.hp, e.max_hp, e.pos), (7, 10, (g.pos.0 + 1, g.pos.1)));
        names.insert(e.name);
        glyphs.insert(e.glyph);
        g.turn += 1; // ターンが進むと見え方が変わる
    }
    assert!(
        names.len() >= 3 && glyphs.len() >= 3,
        "{names:?} {glyphs:?}"
    );
    // 同じターンの見え方は安定していて、観測が乱数を消費しない
    let a = g.observe_text(3);
    assert_eq!(a, g.observe_text(3));
    assert!(g.observe_text(3).contains("幻覚"));
    // 一覧と地図の記号は、同じでたらめを指している
    let e = &g.visible_enemies()[0];
    assert_eq!(g.cell(e.pos.0, e.pos.1).ch, e.glyph);
}

#[test]
fn observing_never_changes_what_happens() {
    let play = |observe: bool| {
        let mut g = with_adjacent(5, &crate::monster::GOBLIN);
        g.inflict(Status::Hallucinating, 100);
        let mut out = Vec::new();
        for _ in 0..10 {
            if observe {
                let _ = g.observe_text(5);
                let _ = g.visible_enemies();
            }
            out.push(g.run("attack east").message);
        }
        out
    };
    assert_eq!(play(true), play(false));
}

#[test]
fn sleep_and_paralysis_pass_time_until_they_wear_off() {
    let mut g = quiet(1);
    g.inflict(Status::Paralyzed, 6);
    let t = g.turn();
    let o = g.run("wait");
    assert_eq!(g.turn() - t, 6, "{}", o.message);
    assert!(o.message.contains("体が動くようになった"), "{}", o.message);
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.inflict(Status::Asleep, 4);
    let hp = g.hp;
    let o = g.run("wait");
    assert!(
        g.hp < hp && o.message.contains("目が覚めた"),
        "{}",
        o.message
    );
}

#[test]
fn haste_halves_and_slow_doubles_the_cost_of_actions() {
    let mut g = quiet(1);
    g.status.apply(Status::Hasted, 1000);
    let t = g.turn();
    for _ in 0..10 {
        g.run("wait");
    }
    assert_eq!(g.turn() - t, 5);
    let mut g = quiet(1);
    g.status.apply(Status::Slowed, 1000);
    let t = g.turn();
    for _ in 0..10 {
        g.run("wait");
    }
    assert_eq!(g.turn() - t, 20);
    // 打ち消し合う
    let mut g = quiet(1);
    g.status.apply(Status::Slowed, 1000);
    g.status.apply(Status::Hasted, 1000);
    let t = g.turn();
    for _ in 0..10 {
        g.run("wait");
    }
    assert_eq!(g.turn() - t, 10);
}

#[test]
fn statuses_count_down_each_turn_and_announce_the_end() {
    let mut g = quiet(1);
    g.inflict(Status::Confused, 3);
    g.inflict(Status::Levitating, 2);
    assert_eq!(g.status.short_text(), "混乱3 浮遊2");
    let o = g.run("stay 1");
    assert_eq!(
        o.statuses,
        vec![(Status::Confused, 2), (Status::Levitating, 1)]
    );
    let o = g.run("stay 1");
    assert!(o.message.contains("浮遊が切れて"), "{}", o.message);
    assert!(o
        .status_events
        .iter()
        .any(|e| e.status == Status::Levitating && e.change == Change::End));
    assert!(g.observe_text(3).contains("混乱: 残り1ターン"));
}

#[test]
fn outcome_carries_status_changes_for_the_record() {
    use crate::record::Event;
    let mut g = quiet(1);
    g.take(ItemKind::Sleep);
    let o = g.run("quaff a");
    assert!(
        o.status_events.iter().any(|e| e.target == "player"
            && e.status == Status::Asleep
            && matches!(e.change, Change::Apply(_))),
        "{:?}",
        o.status_events
    );
    assert!(o
        .status_events
        .iter()
        .any(|e| e.status == Status::Asleep && e.change == Change::End));
    let line = Event::from_outcome(&o, None).to_line();
    assert!(
        line.contains("\"status_events\"") && line.contains("asleep") && line.contains("apply"),
        "{line}"
    );
    assert_eq!(Event::parse(&line), Ok(Event::from_outcome(&o, None)));
    // 状態のないコマンドは、余計な欄を作らない
    let o = g.run("wait");
    assert!(!Event::from_outcome(&o, None).to_line().contains("status"));
}

fn put_trap(g: &mut Game, kind: TrapKind) -> (i32, i32) {
    let p = (g.pos.0 + 1, g.pos.1);
    assert!(g.map.tile(p.0, p.1).walkable());
    g.traps.push(Trap {
        pos: p,
        kind,
        revealed: false,
    });
    p
}

#[test]
fn traps_are_hidden_until_stepped_on_and_then_shown_on_the_map() {
    let mut g = quiet(3);
    let p = put_trap(&mut g, TrapKind::Dart);
    assert_ne!(g.cell(p.0, p.1).ch, '^');
    let hp = g.hp;
    let o = g.run("move east");
    assert!(o.message.contains("毒矢"), "{}", o.message);
    assert!(g.hp < hp && g.status.has(Status::Poisoned));
    g.run("move west");
    assert_eq!(g.cell(p.0, p.1).ch, '^');
    assert!(g.run("look").message.contains("毒矢の罠"));
}

#[test]
fn trapdoor_drops_you_a_floor_and_levitation_ignores_traps() {
    let mut g = quiet(3);
    put_trap(&mut g, TrapKind::Trapdoor);
    let o = g.run("move east");
    assert_eq!(g.depth(), 2, "{}", o.message);
    assert!(o.message.contains("落とし穴に落ちた"), "{}", o.message);
    let mut g = quiet(3);
    let p = put_trap(&mut g, TrapKind::Trapdoor);
    g.inflict(Status::Levitating, 20);
    let o = g.run("move east");
    assert_eq!((g.depth(), g.pos()), (1, p), "{}", o.message);
    assert!(!o.message.contains("落ちた"));
    // 眠りガス
    let mut g = quiet(3);
    put_trap(&mut g, TrapKind::SleepGas);
    let t = g.turn();
    let o = g.run("move east");
    assert!(
        g.turn() - t >= 5 && o.message.contains("目が覚めた"),
        "{}",
        o.message
    );
}

#[test]
fn trapdoors_are_never_placed_where_they_cannot_drop() {
    let mut g = quiet(4);
    g.depth = AMULET_DEPTH;
    for _ in 0..30 {
        g.spawn_traps();
        assert!(g.traps.iter().all(|t| t.kind != TrapKind::Trapdoor));
    }
    let mut g = quiet(4);
    g.has_amulet = true;
    for _ in 0..30 {
        g.spawn_traps();
        assert!(g.traps.iter().all(|t| t.kind != TrapKind::Trapdoor));
    }
}

#[test]
fn standing_next_to_a_trap_can_reveal_it() {
    let mut g = quiet(3);
    let p = put_trap(&mut g, TrapKind::Dart);
    for _ in 0..60 {
        g.run("wait");
    }
    assert!(g.traps[0].revealed, "60ターン隣に立って見つからなかった");
    assert_eq!(g.cell(p.0, p.1).ch, '^');
}

#[test]
fn explore_walks_around_a_known_trap() {
    let mut g = quiet(3);
    let p = put_trap(&mut g, TrapKind::Trapdoor);
    g.traps[0].revealed = true;
    for _ in 0..40 {
        let o = g.run("explore");
        if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
            break;
        }
    }
    assert_eq!(g.depth(), 1);
    assert_ne!(g.pos(), p, "既知の罠のマスに乗った");
}

#[test]
fn a_hidden_trap_stepped_on_while_exploring_fires_exactly_once() {
    let mut fired = 0;
    for seed in 0..40 {
        let mut g = quiet(seed);
        g.hp = 1000;
        g.max_hp = 1000;
        // 開始位置から少し離れた床に、毒矢の罠を隠す
        let Some(p) = (1..W - 1)
            .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
            .find(|&(x, y)| {
                g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs().max((y - g.pos.1).abs()) == 4
            })
        else {
            continue;
        };
        g.traps.push(Trap {
            pos: p,
            kind: TrapKind::Dart,
            revealed: false,
        });
        let mut msgs = String::new();
        for _ in 0..30 {
            let o = g.run("explore");
            msgs.push_str(&o.message);
            if o.message.contains("探索し尽くした") || o.message.contains("もう探索") {
                break;
            }
        }
        let n = msgs.matches("毒矢の罠だ").count();
        assert!(n <= 1, "seed {seed}: {n}回発動した");
        fired += n;
    }
    assert!(fired > 0, "どのseedでも罠を踏まなかった");
}

#[test]
fn an_invisible_player_is_only_noticed_up_close() {
    // 東に4マス歩ける床がある seed を探す
    let (mut g, far) = (3..200)
        .map(|seed| with_adjacent(seed, &crate::monster::GOBLIN))
        .find_map(|g| {
            let far = (g.pos.0 + 4, g.pos.1);
            g.map.tile(far.0, far.1).walkable().then_some((g, far))
        })
        .expect("条件に合う seed がある");
    g.hp = 1000;
    g.monsters[0].pos = far;
    assert!(g.monster_aware(0));
    g.status.apply(Status::Invisible, 100);
    assert!(!g.monster_aware(0));
    g.monsters[0].pos = (g.pos.0 + 2, g.pos.1);
    assert!(g.monster_aware(0));
}

#[test]
fn a_paralyzed_or_sleeping_monster_does_not_act() {
    for st in [Status::Paralyzed, Status::Asleep] {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        let hp = g.hp;
        assert!(g.inflict_monster(0, st, 5));
        g.run("wait");
        g.run("wait");
        assert_eq!(g.hp, hp, "{st:?}");
        // 切れたら殴ってくる
        for _ in 0..6 {
            g.run("wait");
        }
        assert!(g.hp < hp, "{st:?}");
    }
}

#[test]
fn a_scared_monster_flees_and_a_cornered_one_fights() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.inflict_monster(0, Status::Scared, 50);
    let hp = g.hp;
    let d0 = (g.monsters[0].pos.0 - g.pos.0).abs();
    for _ in 0..3 {
        g.run("wait");
    }
    let d1 = (g.monsters[0].pos.0 - g.pos.0)
        .abs()
        .max((g.monsters[0].pos.1 - g.pos.1).abs());
    assert!(d1 > d0 && g.hp == hp, "{d0} {d1}");
    // 逃げ場がなければ戦う
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.inflict_monster(0, Status::Scared, 50);
    let m = g.monsters[0].pos;
    for d in Dir::ALL {
        let (dx, dy) = d.delta();
        let n = (m.0 + dx, m.1 + dy);
        if n != g.pos && g.map.tile(n.0, n.1).walkable() {
            g.monsters.push(monster(&crate::monster::SLIME, n, 1000));
            g.inflict_monster(g.monsters.len() - 1, Status::Paralyzed, 100);
        }
    }
    let hp = g.hp;
    g.run("wait");
    assert!(g.hp < hp);
}

#[test]
fn poison_kills_monsters_and_gives_experience() {
    let mut g = with_adjacent(3, &crate::monster::SLIME);
    g.monsters[0].hp = 2;
    g.monsters[0].max_hp = 2;
    g.inflict_monster(0, Status::Poisoned, 10);
    let xp = g.xp();
    let hp = g.hp;
    g.run("wait");
    g.run("wait");
    assert!(g.monsters.is_empty());
    assert!(g.xp() > xp && g.hp >= hp - 3);
    assert!(g.log().iter().any(|l| l.text.contains("毒で倒れた")));
}

#[test]
fn hasted_monsters_act_twice_and_slowed_ones_every_other_turn() {
    let hits = |st: Option<Status>| {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        if let Some(s) = st {
            g.inflict_monster(0, s, 1000);
        }
        let mut n = 0;
        for _ in 0..20 {
            n += g.run("wait").message.matches("の攻撃！").count();
        }
        n
    };
    let (base, fast, slow) = (
        hits(None),
        hits(Some(Status::Hasted)),
        hits(Some(Status::Slowed)),
    );
    assert_eq!(base, 20);
    assert_eq!(fast, 40);
    assert_eq!(slow, 10);
}

#[test]
fn statuses_that_make_no_sense_for_monsters_are_refused() {
    let mut g = with_adjacent(3, &crate::monster::SLIME);
    assert!(!g.inflict_monster(0, Status::Hallucinating, 5));
    assert!(!g.inflict_monster(0, Status::Levitating, 5));
    assert!(g.monsters[0].status.active().is_empty());
    assert!(g.inflict_monster(0, Status::Confused, 5));
    assert_eq!(g.visible_enemies()[0].statuses, vec![(Status::Confused, 5)]);
    assert!(g.observe_text(3).contains("[混乱5]"));
}

#[test]
fn poison_is_a_status_like_the_others() {
    let mut g = quiet(1);
    g.try_poison(5);
    assert!(g.observe_text(3).contains("毒: 残り5ターン"));
    let o = g.run("stay 1");
    assert_eq!(o.statuses, vec![(Status::Poisoned, 4)]);
}

#[test]
fn a_blind_player_cannot_read_scrolls_but_can_still_drink() {
    let mut g = quiet(1);
    g.take(ItemKind::Teleport);
    g.take(ItemKind::Healing);
    g.inflict(Status::Blind, 20);
    let t = g.turn();
    let o = g.run("read a");
    assert!(
        !o.ok && o.message.contains("読めない") && g.turn() == t,
        "{}",
        o.message
    );
    assert!(g.run("quaff b").ok);
}

// ---- 段階2: 薬と巻物 ----

/// その物を1つ持って、静かな場所にいるゲーム（文字は a）。
fn holding(kind: ItemKind) -> Game {
    let mut g = quiet(2);
    g.hp = 10;
    g.take(kind);
    g
}

fn use_item(g: &mut Game, letter: char) -> Outcome {
    let kind = g
        .inventory
        .iter()
        .find(|s| s.letter == letter)
        .unwrap()
        .kind;
    g.run(&format!(
        "{} {letter}",
        if kind.is_scroll() { "read" } else { "quaff" }
    ))
}

#[test]
fn the_original_rogue_variety_of_potions_and_scrolls_exists() {
    let count = |f: fn(ItemKind) -> bool| ItemKind::ALL.iter().filter(|k| f(**k)).count();
    assert!(count(|k| k.is_potion()) >= 16);
    assert!(count(|k| k.is_scroll()) >= 13);
    for name in [
        "回復の薬",
        "大回復の薬",
        "力の薬",
        "レベルアップの薬",
        "力回復の薬",
        "加速の薬",
        "モンスター探知の薬",
        "アイテム探知の薬",
        "透明視認の薬",
        "浮遊の薬",
        "混乱の薬",
        "幻覚の薬",
        "毒の薬",
        "盲目の薬",
        "眠りの薬",
        "識別の巻物",
        "転移の巻物",
        "地図の巻物",
        "武器強化の巻物",
        "防具強化の巻物",
        "呪い解除の巻物",
        "防具保護の巻物",
        "モンスター混乱の巻物",
        "モンスター停止の巻物",
        "睡眠の巻物",
        "怯えの巻物",
        "モンスター生成の巻物",
        "怒りの巻物",
    ] {
        assert!(
            ItemKind::ALL.iter().any(|k| k.true_name() == name),
            "{name}"
        );
    }
    // 見た目が足りている
    assert!(count(|k| k.is_potion()) <= crate::item::POTION_LOOKS.len());
    assert!(count(|k| k.is_scroll()) <= crate::item::SCROLL_LOOKS.len());
}

#[test]
fn every_potion_and_scroll_can_be_used_and_identifies_itself_the_same_way() {
    for kind in ItemKind::ALL
        .into_iter()
        .filter(|k| k.is_potion() || k.is_scroll())
    {
        for seed in [1u64, 2, 3] {
            let mut g = with_adjacent(seed, &crate::monster::GOBLIN);
            g.take(kind);
            // 識別の巻物の対象になる未知の物も用意する
            g.take(ItemKind::Bread);
            let before = g.display_name(kind);
            let o = use_item(&mut g, 'a');
            assert!(
                o.ok || kind == ItemKind::Identify,
                "{kind:?}: {}",
                o.message
            );
            if !o.ok {
                continue;
            }
            // 未識別のときは「見た目を使った。これは本物の名前だった！」の形に揃っている
            assert!(
                o.message.starts_with(&format!("{before}を"))
                    && o.message
                        .contains(&format!("これは{}だった！", kind.true_name())),
                "{kind:?}: {}",
                o.message
            );
            assert!(g.known[kind.index()]);
            assert!(
                g.inventory.iter().all(|s| s.kind != kind),
                "{kind:?} が残っている"
            );
            // 2個目は本名で出る
            g.take(kind);
            if !g.dead && !g.incapacitated() {
                let letter = g.inventory.iter().find(|s| s.kind == kind).unwrap().letter;
                let o = use_item(&mut g, letter);
                if o.ok {
                    assert!(
                        o.message.starts_with(&format!("{}を", kind.true_name())),
                        "{kind:?}: {}",
                        o.message
                    );
                    assert!(!o.message.contains("これは"), "{}", o.message);
                }
            }
        }
    }
}

#[test]
fn extra_healing_raises_max_hp_when_it_overflows() {
    let mut g = holding(ItemKind::ExtraHealing);
    g.max_hp = 20;
    g.hp = 20;
    let o = use_item(&mut g, 'a');
    assert!(o.message.contains("最大HPが2増えた"), "{}", o.message);
    assert_eq!((g.hp, g.max_hp), (22, 22));
    // 30 以上足りないときは、全部回復に使われて最大HPは増えない
    let mut g = holding(ItemKind::ExtraHealing);
    g.max_hp = 50;
    g.hp = 10;
    use_item(&mut g, 'a');
    assert_eq!((g.hp, g.max_hp), (40, 50));
}

#[test]
fn strength_changes_attack_and_can_be_drained_and_restored() {
    let mut g = holding(ItemKind::Strength);
    let (lo, hi) = g.attack_range();
    use_item(&mut g, 'a');
    assert_eq!((g.strength, g.max_strength), (11, 11));
    g.take(ItemKind::Strength);
    use_item(&mut g, 'a');
    assert_eq!(g.attack_range(), (lo + 1, hi + 1));
    // 毒の薬で腕力が下がり、力回復の薬で戻る
    g.take(ItemKind::Poison);
    g.hp = 20;
    g.max_hp = 20;
    let o = use_item(&mut g, 'a');
    assert_eq!(g.strength, 10, "{}", o.message);
    assert!(o.message.contains("力が抜けた"), "{}", o.message);
    g.take(ItemKind::RestoreStrength);
    let o = use_item(&mut g, 'a');
    assert_eq!(g.strength, 12, "{}", o.message);
    assert!(g.observe_text(3).contains("腕力 12/12"));
    // 下限
    g.strength = 3;
    g.take(ItemKind::Poison);
    g.hp = 20;
    use_item(&mut g, 'a');
    assert_eq!(g.strength, 3);
}

#[test]
fn status_potions_apply_their_status_for_the_listed_turns() {
    for (kind, st) in [
        (ItemKind::Haste, Status::Hasted),
        (ItemKind::SeeInvisible, Status::SeeInvisible),
        (ItemKind::Levitation, Status::Levitating),
        (ItemKind::Confusion, Status::Confused),
        (ItemKind::Hallucination, Status::Hallucinating),
        (ItemKind::Blindness, Status::Blind),
    ] {
        let mut g = holding(kind);
        let o = use_item(&mut g, 'a');
        assert!(g.status.has(st), "{kind:?} {}", o.message);
        assert!(o
            .status_events
            .iter()
            .any(|e| e.status == st && matches!(e.change, Change::Apply(_))));
        assert!(o.message.contains(st.def().start), "{}", o.message);
    }
}

#[test]
fn a_blindness_potion_blinds_and_the_observation_says_so() {
    let mut g = holding(ItemKind::Blindness);
    use_item(&mut g, 'a');
    let t = g.observe_text(3);
    assert!(
        t.contains("目が見えない") && t.contains("盲目: 残り"),
        "{t}"
    );
}

#[test]
fn detect_monsters_marks_them_on_the_map_until_the_next_command() {
    let mut g = holding(ItemKind::DetectMonsters);
    g.monsters
        .push(monster(&crate::monster::GOBLIN, (g.pos.0 + 30, g.pos.1), 9));
    let far = g.monsters[0].pos;
    assert!(!g.map.is_visible(far.0, far.1));
    let o = use_item(&mut g, 'a');
    assert!(o.message.contains("ゴブリン"), "{}", o.message);
    assert_eq!(g.cell(far.0, far.1).ch, 'g');
    assert!(g.observe_text(3).contains("探知した敵"));
    g.run("look");
    assert_ne!(g.cell(far.0, far.1).ch, 'g');
    // 敵がいなければそう言う
    let mut g = holding(ItemKind::DetectMonsters);
    assert!(use_item(&mut g, 'a').message.contains("敵の気配はない"));
}

#[test]
fn detect_items_reveals_where_things_lie_and_keeps_the_map_memory() {
    let mut g = holding(ItemKind::DetectItems);
    let p = (1..W - 1)
        .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
        .find(|&(x, y)| g.map.tile(x, y) == Tile::Floor && !g.map.is_seen(x, y))
        .unwrap();
    g.floor_items.push(FloorItem::new(p, ItemKind::Bread));
    let o = use_item(&mut g, 'a');
    assert!(o.message.contains("パン"), "{}", o.message);
    assert!(g.map.is_seen(p.0, p.1));
    assert_eq!(g.cell(p.0, p.1).ch, '%');
}

#[test]
fn enchant_scrolls_strengthen_the_worn_gear_and_say_so_when_nothing_is_worn() {
    let mut g = with_gear(&[
        ItemKind::Sword,
        ItemKind::Leather,
        ItemKind::EnchantWeapon,
        ItemKind::EnchantArmor,
    ]);
    let o = g.run("read c");
    assert!(
        o.ok && o.message.contains("装備していない"),
        "{}",
        o.message
    );
    g.take(ItemKind::EnchantWeapon);
    g.run("equip a");
    g.run("equip b");
    let (lo, hi) = g.attack_range();
    let def = g.defense();
    let o = g.run("read c");
    assert!(o.message.contains("強化+1"), "{}", o.message);
    assert_eq!(g.attack_range(), (lo + 1, hi + 1));
    let o = g.run("read d");
    assert!(o.message.contains("強化+1"), "{}", o.message);
    assert_eq!(g.defense(), def + 1);
    assert!(g.inventory_lines().iter().any(|l| l.contains("防御 2")));
}

#[test]
fn remove_curse_lets_you_take_off_cursed_gear_but_the_drawback_stays() {
    let mut g = quiet(2);
    let a = give(&mut g, suffix_gear(ItemKind::Sword, Suffix::Cataclysm));
    g.run(&format!("equip {a}"));
    assert!(!g.run(&format!("unequip {a}")).ok);
    g.take(ItemKind::RemoveCurse);
    let b = g
        .inventory
        .iter()
        .find(|s| s.kind == ItemKind::RemoveCurse)
        .unwrap()
        .letter;
    let o = g.run(&format!("read {b}"));
    assert!(o.message.contains("呪いの束縛が解けた"), "{}", o.message);
    assert!(g.run(&format!("unequip {a}")).ok);
    assert!(g.gear_of(Some(a)).unwrap().is_cursed());
}

#[test]
fn protect_armor_stops_rust_and_aquators_rust_unprotected_armor() {
    let mut g = with_adjacent(3, &crate::monster::AQUATOR);
    let a = give(&mut g, Gear::plain(ItemKind::Chain));
    g.run(&format!("equip {a}"));
    let def = g.defense();
    for _ in 0..3 {
        g.run("wait");
    }
    assert_eq!(g.defense(), def - 3);
    assert!(g.log().iter().any(|l| l.text.contains("錆びた")));
    // 保護
    let mut g = with_adjacent(3, &crate::monster::AQUATOR);
    let a = give(&mut g, Gear::plain(ItemKind::Chain));
    g.run(&format!("equip {a}"));
    g.take(ItemKind::ProtectArmor);
    let b = g
        .inventory
        .iter()
        .find(|s| s.kind == ItemKind::ProtectArmor)
        .unwrap()
        .letter;
    g.run(&format!("read {b}"));
    let def = g.defense();
    for _ in 0..3 {
        g.run("wait");
    }
    assert_eq!(g.defense(), def);
    assert!(g.log().iter().any(|l| l.text.contains("錆びなかった")));
}

#[test]
fn monster_scrolls_afflict_what_you_can_see() {
    for (kind, st) in [
        (ItemKind::ConfuseMonster, Status::Confused),
        (ItemKind::ScareMonster, Status::Scared),
    ] {
        let mut g = with_adjacent(3, &crate::monster::GOBLIN);
        g.take(kind);
        let o = g.run("read a");
        assert!(g.monsters[0].status.has(st), "{kind:?} {}", o.message);
        assert!(o
            .status_events
            .iter()
            .any(|e| e.target == "ゴブリン" && e.status == st));
    }
    // 停止は近くの敵だけ
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    let far = (g.pos.0 + 5, g.pos.1);
    if g.map.tile(far.0, far.1).walkable() && g.map.is_visible(far.0, far.1) {
        g.monsters.push(monster(&crate::monster::SLIME, far, 1000));
        g.take(ItemKind::HoldMonster);
        g.run("read a");
        assert!(g.monsters[0].status.has(Status::Paralyzed));
        assert!(!g.monsters[1].status.has(Status::Paralyzed));
    }
    // 相手がいなければそう言う
    let mut g = holding(ItemKind::HoldMonster);
    assert!(use_item(&mut g, 'a')
        .message
        .contains("効く相手がいなかった"));
}

#[test]
fn create_monster_and_aggravate_and_slumber_are_the_bad_scrolls() {
    let mut g = holding(ItemKind::CreateMonster);
    let o = use_item(&mut g, 'a');
    assert_eq!(g.monsters.len(), 1, "{}", o.message);
    let m = g.monsters[0].pos;
    assert!((m.0 - g.pos.0).abs() <= 1 && (m.1 - g.pos.1).abs() <= 1);
    let mut g = holding(ItemKind::Aggravate);
    g.monsters
        .push(monster(&crate::monster::GOBLIN, (g.pos.0 + 30, g.pos.1), 9));
    use_item(&mut g, 'a');
    assert!(g.monsters[0].status.has(Status::Enraged));
    let mut g = holding(ItemKind::Slumber);
    let t = g.turn();
    let o = use_item(&mut g, 'a');
    assert!(
        g.turn() - t >= 6 && o.message.contains("目が覚めた"),
        "{}",
        o.message
    );
}

#[test]
fn an_enraged_monster_hunts_you_from_anywhere() {
    let mut g = (0..60)
        .map(quiet)
        .find(|g| {
            (1..W - 1).any(|x| {
                (1..H - 1).any(|y| {
                    g.map.tile(x, y) == Tile::Floor
                        && (x - g.pos.0).abs() + (y - g.pos.1).abs() > 25
                })
            })
        })
        .expect("遠い床がある seed");
    let far = (1..W - 1)
        .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
        .find(|&(x, y)| {
            g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs() + (y - g.pos.1).abs() > 25
        })
        .unwrap();
    let id = g.add_monster(&crate::monster::SLIME, far);
    assert!(!g.monster_aware(id), "遠い敵は怒っていなければ気づかない");
    g.inflict_monster(id, Status::Enraged, 100);
    assert!(g.monster_aware(id));
    let mut moved = false;
    for _ in 0..5 {
        g.run("wait");
        moved |= g.monsters[id].pos != far;
    }
    assert!(moved);
}

#[test]
fn identify_finds_unknown_potions_and_scrolls_among_the_new_kinds() {
    let mut g = quiet(2);
    g.take(ItemKind::Identify);
    g.take(ItemKind::Blindness);
    let o = g.run("read a b");
    assert!(o.ok && !g.status.has(Status::Blind), "{}", o.message);
    assert!(g.known[ItemKind::Blindness.index()]);
    assert!(g.run("quaff b").message.starts_with("盲目の薬を飲んだ。"));
}

#[test]
fn bad_things_stay_a_minority_of_the_floor_drops() {
    for class in [Class::Potion, Class::Scroll] {
        let all: Vec<ItemKind> = ItemKind::ALL
            .into_iter()
            .filter(|k| k.class() == class)
            .collect();
        let total: u32 = all.iter().map(|k| k.weight()).sum();
        let bad: u32 = all.iter().filter(|k| k.is_bad()).map(|k| k.weight()).sum();
        assert!(bad * 3 <= total, "{class:?} {bad}/{total}");
    }
}

// ---- 段階3: 杖 ----

/// 充填数 `charges` の杖を1本持つ（文字は a）。
fn with_wand(kind: ItemKind, charges: i32) -> Game {
    let mut g = quiet(2);
    g.hp = 20;
    g.max_hp = 20;
    g.take(Tool::charged(kind, charges));
    g
}

/// 東隣に敵を置いて杖を持つ。
fn wand_vs_adjacent(kind: ItemKind, mk: &'static MonsterKind, hp: i32) -> Game {
    let mut g = with_adjacent(3, mk);
    g.monsters[0].hp = hp;
    g.monsters[0].max_hp = hp;
    g.take(Tool::charged(kind, 5));
    g
}

#[test]
fn all_the_original_wand_kinds_exist() {
    for name in [
        "光の杖",
        "透明化の杖",
        "雷の杖",
        "火の杖",
        "冷気の杖",
        "変身の杖",
        "魔法の矢の杖",
        "敵加速の杖",
        "敵減速の杖",
        "生命吸収の杖",
        "消去の杖",
        "敵テレポートの杖",
        "自分テレポートの杖",
    ] {
        assert!(
            ItemKind::ALL
                .iter()
                .any(|k| k.is_wand() && k.true_name() == name),
            "{name}"
        );
    }
    let g = Game::new(5);
    let wands: Vec<&str> = ItemKind::ALL
        .iter()
        .filter(|k| k.is_wand())
        .map(|k| g.looks[k.index()])
        .collect();
    let uniq: std::collections::HashSet<_> = wands.iter().collect();
    assert_eq!(uniq.len(), wands.len());
    assert!(wands.iter().all(|l| l.ends_with('杖')));
}

#[test]
fn zapping_uses_a_charge_identifies_the_wand_and_reports_what_is_left() {
    let mut g = wand_vs_adjacent(ItemKind::WandMissile, &crate::monster::OGRE, 1000);
    let before = g.display_name(ItemKind::WandMissile);
    assert!(
        g.inventory_lines()[0].contains("[残り5回]") && g.inventory_lines()[0].contains("(未識別)")
    );
    assert!(
        g.status_text().contains("杖残り[a:5]"),
        "{}",
        g.status_text()
    );
    let o = g.run("zap a east");
    assert!(o.ok, "{}", o.message);
    assert!(
        o.message
            .starts_with(&format!("{before}を振った。これは魔法の矢の杖だった！")),
        "{}",
        o.message
    );
    assert!(o.message.contains("(残り4回)"), "{}", o.message);
    assert!(g.monsters[0].hp < 1000);
    assert!(
        g.inventory_lines()[0].contains("魔法の矢の杖 [残り4回]")
            && !g.inventory_lines()[0].contains("未識別")
    );
    assert!(g.observe_text(3).contains("杖残り[a:4]"));
    // 2回目からは本名で
    let o = g.run("zap a east");
    assert!(
        o.message.starts_with("魔法の矢の杖を振った。") && !o.message.contains("これは"),
        "{}",
        o.message
    );
}

#[test]
fn an_empty_wand_cannot_be_used_and_costs_nothing() {
    let mut g = wand_vs_adjacent(ItemKind::WandMissile, &crate::monster::OGRE, 1000);
    for _ in 0..4 {
        assert!(g.run("zap a east").ok);
    }
    let o = g.run("zap a east");
    assert!(
        o.ok && o.message.contains("魔力は尽きた") && !o.message.contains("0回"),
        "{}",
        o.message
    );
    assert!(g.inventory_lines()[0].contains("[残り0回]"));
    let (t, hp) = (g.turn(), g.monsters[0].hp);
    let o = g.run("zap a east");
    assert!(!o.ok && o.message.contains("充填数が0"), "{}", o.message);
    assert_eq!((g.turn(), g.monsters[0].hp), (t, hp));
    // 捨てて拾い直しても、残りは0のまま
    assert!(g.run("drop a").ok);
    assert!(g.run("pickup").ok);
    assert!(g.inventory_lines()[0].contains("[残り0回]"));
}

#[test]
fn zap_needs_a_direction_a_wand_and_a_visible_target() {
    let mut g = with_wand(ItemKind::WandFire, 3);
    let t = g.turn();
    let o = g.run("zap a");
    assert!(
        !o.ok && o.message.contains("向きが要る") && g.turn() == t,
        "{}",
        o.message
    );
    let o = g.run("zap a nearest");
    assert!(!o.ok && o.message.contains("狙える敵"), "{}", o.message);
    assert_eq!(g.tool_charges('a'), 3);
    assert!(!g.run("zap b east").ok);
    g.take(ItemKind::Bread);
    let o = g.run("zap b east");
    assert!(!o.ok && o.message.contains("杖ではない"), "{}", o.message);
    let o = g.run("quaff a");
    assert!(!o.ok && o.message.contains("zap"), "{}", o.message);
}

impl Game {
    fn tool_charges(&self, letter: char) -> i32 {
        self.inventory
            .iter()
            .find(|s| s.letter == letter)
            .and_then(|s| s.tool)
            .map_or(-1, |t| t.val)
    }
}

#[test]
fn bolts_hit_the_first_enemy_in_line_and_nearest_aims_by_itself() {
    let mut g = quiet(3);
    g.hp = 1000;
    g.max_hp = 1000;
    g.take(Tool::charged(ItemKind::WandFire, 5));
    // 東に2体並べる
    let (p1, p2) = ((g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1));
    assert!(g.map.tile(p2.0, p2.1).walkable());
    g.monsters.push(monster(&crate::monster::OGRE, p1, 500));
    g.monsters.push(monster(&crate::monster::OGRE, p2, 500));
    g.inflict_monster(0, Status::Paralyzed, 100);
    g.inflict_monster(1, Status::Paralyzed, 100);
    g.run("zap a east");
    assert!(g.monsters[0].hp < 500 && g.monsters[1].hp == 500);
    g.run("zap a nearest");
    assert!(g.monsters[0].hp < 490 && g.monsters[1].hp == 500);
    // 反対側には何もない
    let o = g.run("zap a west");
    assert!(o.message.contains("何にも当たらなかった"), "{}", o.message);
}

#[test]
fn lightning_pierces_every_enemy_in_the_line() {
    let mut g = quiet(3);
    g.hp = 1000;
    g.max_hp = 1000;
    g.take(Tool::charged(ItemKind::WandLightning, 5));
    let (p1, p2) = ((g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1));
    assert!(g.map.tile(p2.0, p2.1).walkable());
    g.monsters.push(monster(&crate::monster::OGRE, p1, 500));
    g.monsters.push(monster(&crate::monster::OGRE, p2, 500));
    g.inflict_monster(0, Status::Paralyzed, 100);
    g.inflict_monster(1, Status::Paralyzed, 100);
    g.run("zap a east");
    assert!(g.monsters[0].hp < 500 && g.monsters[1].hp < 500);
}

#[test]
fn nearest_lightning_keeps_going_past_the_target_and_always_hits_what_you_see() {
    let mut g = quiet(3);
    g.hp = 1000;
    g.max_hp = 1000;
    g.take(Tool::charged(ItemKind::WandLightning, 5));
    let ps = [
        (g.pos.0 + 1, g.pos.1),
        (g.pos.0 + 2, g.pos.1),
        (g.pos.0 + 3, g.pos.1),
    ];
    assert!(ps.iter().all(|p| g.map.tile(p.0, p.1).walkable()));
    for p in ps {
        g.monsters.push(monster(&crate::monster::OGRE, p, 500));
        let i = g.monsters.len() - 1;
        g.inflict_monster(i, Status::Paralyzed, 100);
    }
    g.run("zap a nearest");
    assert!(
        g.monsters.iter().all(|m| m.hp < 500),
        "{:?}",
        g.monsters.iter().map(|m| m.hp).collect::<Vec<_>>()
    );
}

#[test]
fn diagonal_nearest_lightning_pierces_along_the_diagonal() {
    let mut g = quiet(3);
    g.hp = 1000;
    g.max_hp = 1000;
    g.take(Tool::charged(ItemKind::WandLightning, 5));
    // 斜めの線を床にして、2体を並べる
    for k in 1..=4 {
        g.map.set_tile(g.pos.0 + k, g.pos.1 + k, Tile::Floor);
    }
    for k in [1, 3] {
        g.monsters.push(monster(
            &crate::monster::OGRE,
            (g.pos.0 + k, g.pos.1 + k),
            500,
        ));
        let i = g.monsters.len() - 1;
        g.inflict_monster(i, Status::Paralyzed, 100);
    }
    g.run("zap a nearest");
    assert!(
        g.monsters.iter().all(|m| m.hp < 500),
        "{:?}",
        g.monsters.iter().map(|m| m.hp).collect::<Vec<_>>()
    );
}

#[test]
fn a_blocked_extension_falls_back_to_the_line_to_the_target() {
    let mut g = quiet(3);
    g.hp = 1000;
    g.max_hp = 1000;
    g.take(Tool::charged(ItemKind::WandMissile, 5));
    let me = g.pos;
    // 延長線と直線が途中で食い違う配置(敵までは通るが、手前の1マスだけ違う)を探す
    let mut setup = None;
    'search: for dx in -8..=8i32 {
        for dy in -8..=8i32 {
            let t = (me.0 + dx, me.1 + dy);
            let m = dx.abs().max(dy.abs());
            if m < 3 || dx * dx + dy * dy > 64 {
                continue;
            }
            let long = Map::line(me, (me.0 + dx * 12 / m, me.1 + dy * 12 / m));
            let direct = Map::line(me, t);
            if !long.contains(&t) {
                continue;
            }
            let pos_t = long.iter().position(|p| *p == t).unwrap();
            if let Some(i) = (0..pos_t).find(|&i| long[i] != direct[i]) {
                setup = Some((t, long[i], long, direct));
                break 'search;
            }
        }
    }
    let (t, blocker, long, direct) = setup.expect("食い違う配置がある");
    for p in long.iter().chain(direct.iter()) {
        g.map.set_tile(p.0, p.1, Tile::Floor);
    }
    g.map.set_tile(blocker.0, blocker.1, Tile::Wall);
    g.refresh_fov();
    g.monsters.push(monster(&crate::monster::OGRE, t, 500));
    g.inflict_monster(0, Status::Paralyzed, 100);
    assert!(g.can_see_monster(0), "直線が通っているので見えるはず");
    // 延長線は敵の手前で塞がれているが、直線に戻って敵に当たる
    assert!(!g.open_cells(&long).contains(&t));
    assert_eq!(
        g.bolt_cells(crate::command::ZapTarget::Nearest).unwrap(),
        direct
    );
    g.run("zap a nearest");
    assert!(g.monsters[0].hp < 500);
}

#[test]
fn nearest_hits_every_visible_enemy_in_open_rooms() {
    // どの向きにいる見えている敵にも、nearest なら必ず当たる(充填だけ減ることがない)
    let mut checked = 0;
    for seed in 0..60u64 {
        let mut g = quiet(seed);
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(Tool::charged(ItemKind::WandMissile, 5));
        for (dx, dy) in [(3, 2), (-4, 1), (2, -3), (-2, -2), (5, 0), (0, 4), (4, 3)] {
            let p = (g.pos.0 + dx, g.pos.1 + dy);
            if !g.map.tile(p.0, p.1).walkable() || !g.map.is_visible(p.0, p.1) {
                continue;
            }
            g.monsters.clear();
            g.monsters.push(monster(&crate::monster::OGRE, p, 500));
            g.inflict_monster(0, Status::Paralyzed, 100);
            g.inventory[0].tool.as_mut().unwrap().val = 5;
            let o = g.run("zap a nearest");
            assert!(
                g.monsters[0].hp < 500,
                "seed {seed} {dx},{dy}: {}",
                o.message
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}

#[test]
fn bolts_do_not_pass_through_walls() {
    let mut g = quiet(3);
    g.take(Tool::charged(ItemKind::WandFire, 5));
    // 東隣に壁を立て、その向こうに敵を置く
    let (wall, beyond) = ((g.pos.0 + 1, g.pos.1), (g.pos.0 + 2, g.pos.1));
    g.map.set_tile(wall.0, wall.1, Tile::Wall);
    g.map.set_tile(beyond.0, beyond.1, Tile::Floor);
    g.monsters.push(monster(&crate::monster::OGRE, beyond, 50));
    let o = g.run("zap a east");
    assert_eq!(g.monsters[0].hp, 50, "{}", o.message);
    assert!(o.message.contains("何にも当たらなかった"), "{}", o.message);
}

#[test]
fn killing_with_a_wand_gives_experience() {
    let mut g = wand_vs_adjacent(ItemKind::WandFire, &crate::monster::SLIME, 3);
    let xp = g.xp();
    let o = g.run("zap a east");
    assert!(o.message.contains("を倒した"), "{}", o.message);
    assert!(g.monsters.is_empty() && g.xp() > xp);
}

#[test]
fn cold_slows_what_it_hits_and_slow_and_haste_wands_use_statuses() {
    let mut g = wand_vs_adjacent(ItemKind::WandCold, &crate::monster::OGRE, 1000);
    g.run("zap a east");
    assert!(g.monsters[0].status.has(Status::Slowed));
    let mut g = wand_vs_adjacent(ItemKind::WandSlow, &crate::monster::GOBLIN, 1000);
    let o = g.run("zap a east");
    assert!(g.monsters[0].status.has(Status::Slowed), "{}", o.message);
    assert!(o
        .status_events
        .iter()
        .any(|e| e.target == "ゴブリン" && e.status == Status::Slowed));
    let mut g = wand_vs_adjacent(ItemKind::WandHaste, &crate::monster::GOBLIN, 1000);
    g.run("zap a east");
    assert!(g.monsters[0].status.has(Status::Hasted));
}

#[test]
fn drain_life_heals_you_by_the_damage_dealt() {
    let mut g = wand_vs_adjacent(ItemKind::WandDrain, &crate::monster::OGRE, 1000);
    g.hp = 5;
    g.monsters[0].status.apply(Status::Paralyzed, 100); // 反撃を受けない
    let o = g.run("zap a east");
    assert!(
        g.hp > 5 && o.message.contains("生命力を吸い取った"),
        "{} hp={}",
        o.message,
        g.hp
    );
    assert_eq!(1000 - g.monsters[0].hp, g.hp - 5);
}

#[test]
fn invisible_monsters_vanish_from_view_until_you_can_see_invisible() {
    let mut g = wand_vs_adjacent(ItemKind::WandInvisibility, &crate::monster::GOBLIN, 1000);
    let o = g.run("zap a east");
    assert!(o.message.contains("姿が消えた"), "{}", o.message);
    assert!(g.visible_enemies().is_empty());
    assert_ne!(g.cell(g.pos.0 + 1, g.pos.1).ch, 'g');
    // 見えなくても殴られる。向きは分かる
    let o = g.run("wait");
    assert!(o.message.contains("何かの攻撃！(東から)"), "{}", o.message);
    g.status.apply(Status::SeeInvisible, 50);
    assert_eq!(g.visible_enemies().len(), 1);
    assert_eq!(g.cell(g.pos.0 + 1, g.pos.1).ch, 'g');
}

#[test]
fn polymorph_changes_the_kind_and_keeps_the_health_ratio() {
    for seed in 1..20 {
        let mut g = wand_vs_adjacent(ItemKind::WandPolymorph, &crate::monster::OGRE, 5);
        g.depth = 4;
        g.monsters[0].max_hp = 10;
        g.monsters[0].status.apply(Status::Paralyzed, 100);
        g.rng = Rng::new(seed);
        g.run("zap a east");
        let m = &g.monsters[0];
        assert!(!std::ptr::eq(m.kind, &crate::monster::OGRE), "seed {seed}");
        assert_eq!((m.name, m.glyph), (m.kind.name, m.kind.glyph));
        assert_eq!(m.max_hp, m.kind.hp_at(4));
        assert!(m.hp >= 1 && m.hp <= m.max_hp);
    }
}

#[test]
fn cancellation_strips_statuses_and_special_powers() {
    let mut g = wand_vs_adjacent(ItemKind::WandCancel, &crate::monster::SPIDER, 1000);
    g.inflict_monster(0, Status::Hasted, 50);
    let o = g.run("zap a east");
    assert!(
        g.monsters[0].status.active().is_empty() && g.monsters[0].cancelled,
        "{}",
        o.message
    );
    for _ in 0..40 {
        g.run("wait");
    }
    assert!(
        !g.status.has(Status::Poisoned),
        "消去された毒グモが毒を撒いた"
    );
    assert!(o
        .status_events
        .iter()
        .any(|e| e.status == Status::Hasted && e.change == Change::End));
}

#[test]
fn teleport_other_sends_the_monster_away_and_teleport_self_moves_you() {
    let mut g = wand_vs_adjacent(ItemKind::WandTeleportOther, &crate::monster::GOBLIN, 1000);
    let before = g.monsters[0].pos;
    let o = g.run("zap a east");
    assert_ne!(g.monsters[0].pos, before, "{}", o.message);
    let mut g = with_wand(ItemKind::WandTeleportSelf, 3);
    let before = g.pos;
    // 向きは要らない
    let o = g.run("zap a");
    assert!(o.ok && g.pos != before, "{}", o.message);
    assert!(o.message.contains("(残り2回)"));
}

#[test]
fn the_light_wand_maps_the_corridor_ahead() {
    let mut tried = false;
    for seed in 0..60 {
        let mut g = quiet(seed);
        g.take(Tool::charged(ItemKind::WandLight, 3));
        g.map.forget_all();
        for d in Dir::ALL {
            let (dx, dy) = d.delta();
            let open = (1..=6).all(|k| g.map.tile(g.pos.0 + dx * k, g.pos.1 + dy * k).walkable());
            let far = (g.pos.0 + 6 * dx, g.pos.1 + 6 * dy);
            if open {
                let o = g.run(&format!("zap a {}", d.name()));
                assert!(g.map.is_seen(far.0, far.1), "{}", o.message);
                assert!(o.message.contains("照らされた"));
                tried = true;
                break;
            }
        }
        if tried {
            break;
        }
    }
    assert!(tried, "条件に合う場所がなかった");
}

#[test]
fn confusion_can_send_a_bolt_the_wrong_way_even_with_nearest() {
    for cmd in ["zap a east", "zap a nearest"] {
        let mut g = wand_vs_adjacent(ItemKind::WandMissile, &crate::monster::OGRE, 100000);
        g.monsters[0].status.apply(Status::Paralyzed, 100000);
        g.status.apply(Status::Confused, 100000);
        let (mut misses, mut hits) = (0, 0);
        for _ in 0..40 {
            let hp = g.monsters[0].hp;
            g.run(cmd);
            if g.monsters[0].hp == hp {
                misses += 1
            } else {
                hits += 1
            }
            g.inventory[0].tool.as_mut().unwrap().val = 5;
        }
        assert!(misses > 5 && hits > 5, "{cmd}: {misses} {hits}");
    }
}

#[test]
fn stealth_and_invisibility_never_make_adjacent_enemies_unaware() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.take(ring(ItemKind::RingStealth, 3));
    g.run("equip a");
    g.status.apply(Status::Invisible, 100);
    assert!(g.monster_aware(0));
    assert_eq!(g.notice_radius(), 2);
    g.status.clear(Status::Invisible);
    assert_eq!(g.notice_radius(), 2);
    g.rings[1] = g.rings[0]; // 強すぎる隠密(+6相当)でも下限がある
    assert!(g.notice_radius() >= 2);
}

#[test]
fn drinking_haste_costs_a_turn_and_then_actions_alternate() {
    let mut g = quiet(1);
    g.take(ItemKind::Haste);
    let t = g.turn();
    g.run("quaff a");
    assert_eq!(g.turn() - t, 1);
    let t = g.turn();
    for _ in 0..4 {
        g.run("wait");
    }
    assert_eq!(g.turn() - t, 2);
}

#[test]
fn wands_are_individuals_and_survive_drop_and_pickup_with_their_charges() {
    let mut g = quiet(2);
    g.take(Tool::charged(ItemKind::WandFire, 4));
    g.take(Tool::charged(ItemKind::WandFire, 2));
    assert_eq!(g.inventory.len(), 2);
    assert!(
        g.inventory_lines()[0].contains("[残り4回]")
            && g.inventory_lines()[1].contains("[残り2回]")
    );
    g.run("drop a");
    assert!(g.underfoot_text().contains("火の杖") || g.underfoot_text().contains("杖"));
    g.run("pickup");
    assert!(g.inventory_lines().iter().any(|l| l.contains("[残り4回]")));
}

#[test]
fn floor_wands_have_charges_inside_the_listed_range() {
    let mut rng = Rng::new(9);
    for k in ItemKind::ALL.into_iter().filter(|k| k.is_wand()) {
        for _ in 0..20 {
            let Item::Tool(t) = Item::roll(&mut rng, k, 5) else {
                panic!()
            };
            let (lo, hi) = k.zap().unwrap().charges;
            assert!((lo..=hi).contains(&t.val), "{k:?} {}", t.val);
        }
    }
    // 深いところの床には杖も落ちる
    let mut found = false;
    for seed in 0..60 {
        let mut g = Game::new(seed);
        g.depth = 6;
        g.spawn_items();
        found |= g.floor_items.iter().any(|f| f.item.kind().is_wand());
    }
    assert!(found);
}

#[test]
fn identify_scroll_can_identify_an_unknown_wand() {
    let mut g = quiet(2);
    g.take(ItemKind::Identify);
    g.take(Tool::charged(ItemKind::WandSlow, 4));
    let o = g.run("read a b");
    assert!(o.ok && g.known[ItemKind::WandSlow.index()], "{}", o.message);
    assert!(g.inventory_lines()[0].contains("敵減速の杖 [残り4回]"));
}

// ---- 段階4: 指輪と光源 ----

fn ring(kind: ItemKind, val: i32) -> Tool {
    let mut t = Tool::charged(kind, val);
    t.identified = false;
    t.cursed = val < 0 || kind.ring_def().is_some_and(|r| r.always_cursed);
    t
}

/// 指輪を持って（まだはめていない）いる静かなゲーム。文字は a, b, ...
fn with_rings(rings: &[Tool]) -> Game {
    let mut g = quiet(2);
    g.hp = 20;
    g.max_hp = 20;
    for r in rings {
        g.take(*r);
    }
    g
}

#[test]
fn all_the_rogue_rings_exist_with_gem_looks() {
    for name in [
        "防御の指輪",
        "腕力の指輪",
        "器用さの指輪",
        "ダメージ増加の指輪",
        "再生の指輪",
        "消化遅延の指輪",
        "隠密の指輪",
        "探索の指輪",
        "透明視認の指輪",
        "装飾の指輪",
        "怒らせる指輪",
        "テレポート癖の指輪",
    ] {
        assert!(
            ItemKind::ALL
                .iter()
                .any(|k| k.is_ring() && k.true_name() == name),
            "{name}"
        );
    }
    let g = Game::new(7);
    let looks: Vec<&str> = ItemKind::ALL
        .iter()
        .filter(|k| k.is_ring())
        .map(|k| g.looks[k.index()])
        .collect();
    assert_eq!(
        looks.iter().collect::<std::collections::HashSet<_>>().len(),
        looks.len()
    );
    assert!(
        looks.iter().all(|l| l.ends_with("の指輪")) && !looks.iter().any(|l| l.contains("防御"))
    );
}

#[test]
fn rings_are_unknown_until_worn_for_a_while_and_effects_apply_meanwhile() {
    let mut g = with_rings(&[ring(ItemKind::RingProtection, 2)]);
    let look = g.looks[ItemKind::RingProtection.index()];
    assert!(g.inventory_lines()[0].contains(look) && g.inventory_lines()[0].contains("未識別"));
    let def = g.defense();
    let o = g.run("equip a");
    assert!(
        o.ok && o.message.contains("効果はまだ分からない"),
        "{}",
        o.message
    );
    assert_eq!(g.defense(), def + 2, "識別前でも効果は効いている");
    assert!(g.inventory_lines()[0].contains("(装備中)"));
    for _ in 0..27 {
        g.run("wait");
    }
    assert!(!g.known[ItemKind::RingProtection.index()]);
    let mut msg = String::new();
    for _ in 0..4 {
        msg.push_str(&g.run("wait").message);
    }
    assert!(
        msg.contains("正体が分かった") && msg.contains("防御の指輪 +2"),
        "{msg}"
    );
    assert!(g.known[ItemKind::RingProtection.index()]);
    assert!(
        g.inventory_lines()[0].contains("防御の指輪 +2")
            && g.inventory_lines()[0].contains("防御+2")
    );
}

#[test]
fn a_known_kind_is_identified_as_soon_as_it_is_worn() {
    let mut g = with_rings(&[ring(ItemKind::RingStrength, 1)]);
    g.known[ItemKind::RingStrength.index()] = true;
    assert!(
        g.inventory_lines()[0].contains("腕力の指輪 (+?)"),
        "{}",
        g.inventory_lines()[0]
    );
    let o = g.run("equip a");
    assert!(
        o.message.contains("腕力の指輪 +1") && o.message.contains("腕力+1"),
        "{}",
        o.message
    );
}

#[test]
fn two_ring_slots_and_unequip() {
    let mut g = with_rings(&[
        ring(ItemKind::RingTrinket, 0),
        ring(ItemKind::RingSearching, 1),
        ring(ItemKind::RingStealth, 1),
    ]);
    assert!(g.run("equip a").ok && g.run("equip b").ok);
    let o = g.run("equip c");
    assert!(
        !o.ok && o.message.contains("ふさがっている"),
        "{}",
        o.message
    );
    assert!(!g.run("equip a").ok);
    assert!(g.run("unequip a").ok);
    assert!(g.run("equip c").ok);
    assert!(!g.run("unequip a").ok);
    // 装備中の指輪は捨てられない
    assert!(!g.run("drop b").ok);
    assert!(g.run("unequip b").ok && g.run("drop b").ok);
}

#[test]
fn cursed_rings_cannot_be_removed_until_remove_curse() {
    let mut g = with_rings(&[ring(ItemKind::RingProtection, -2)]);
    let def = g.defense();
    let o = g.run("equip a");
    assert!(
        o.message.contains("呪われていた") && o.message.contains("防御の指輪 -2"),
        "{}",
        o.message
    );
    assert_eq!(g.defense(), def - 2);
    let o = g.run("unequip a");
    assert!(!o.ok && o.message.contains("はずせない"), "{}", o.message);
    assert!(!g.run("drop a").ok);
    assert!(g.inventory_lines()[0].contains("(呪われている)"));
    g.take(ItemKind::RemoveCurse);
    let o = g.run("read b");
    assert!(o.message.contains("呪いの束縛が解けた"), "{}", o.message);
    assert!(g.run("unequip a").ok);
    assert!(g.inventory_lines()[0].contains("(呪い解除済み)"));
}

#[test]
fn strength_damage_and_dexterity_rings() {
    let mut g = with_rings(&[
        ring(ItemKind::RingStrength, 2),
        ring(ItemKind::RingDamage, 1),
    ]);
    let (lo, hi) = g.attack_range();
    g.run("equip a");
    assert_eq!(g.attack_range(), (lo + 1, hi + 1)); // 腕力 +2 → 攻撃 +1
    assert!(g.observe_text(3).contains("腕力 12/10"));
    g.run("equip b");
    assert_eq!(g.attack_range(), (lo + 2, hi + 2));
    // 器用さ: 敵の攻撃をかわす
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.take(ring(ItemKind::RingDexterity, 5));
    g.run("equip a");
    let mut dodged = 0;
    let mut hit = 0;
    for _ in 0..60 {
        let m = g.run("wait").message;
        dodged += m.matches("かわした").count();
        hit += m.matches("の攻撃！").count();
    }
    assert!(dodged > 10 && hit > 10, "{dodged} {hit}");
    // 負の器用さ: 自分の攻撃が空振りする
    let mut g = with_adjacent(3, &crate::monster::OGRE);
    g.monsters[0].status.apply(Status::Paralyzed, 1000);
    g.take(ring(ItemKind::RingDexterity, -3));
    g.run("equip a");
    let whiffs = (0..60)
        .filter(|_| g.run("attack east").message.contains("空を切った"))
        .count();
    assert!(whiffs > 8 && whiffs < 40, "{whiffs}");
}

#[test]
fn regeneration_heals_faster_even_with_enemies_around() {
    let mut g = quiet(2);
    g.take(ring(ItemKind::RingRegeneration, 2));
    g.run("equip a");
    g.hp = 1;
    g.max_hp = 100;
    for _ in 0..40 {
        g.run("wait");
    }
    assert!(g.hp > 40 / 4, "{}", g.hp);
    // 敵が見えていても治る
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.monsters[0].status.apply(Status::Paralyzed, 1000);
    g.take(ring(ItemKind::RingRegeneration, 3));
    g.run("equip a");
    g.hp = 1;
    for _ in 0..20 {
        g.run("wait");
    }
    assert!(g.hp > 3);
}

#[test]
fn slow_digestion_makes_hunger_grow_slower() {
    let hunger = |with_ring: bool| {
        let mut g = quiet(2);
        if with_ring {
            g.take(ring(ItemKind::RingSlowDigestion, 1));
            g.run("equip a");
        }
        let f = g.food;
        for _ in 0..40 {
            g.run("wait");
        }
        f - g.food
    };
    let (plain, slow) = (hunger(false), hunger(true));
    assert!(
        slow * 2 <= plain + 2 && slow * 2 + 2 >= plain,
        "{plain} {slow}"
    );
}

#[test]
fn stealth_shrinks_the_distance_enemies_notice_you() {
    let mut g = (3..200)
        .map(|seed| with_adjacent(seed, &crate::monster::GOBLIN))
        .find(|g| {
            g.map.tile(g.pos.0 + 5, g.pos.1).walkable() && g.map.los(g.pos, (g.pos.0 + 5, g.pos.1))
        })
        .expect("東に5マス見通せる seed がある");
    let far = (g.pos.0 + 5, g.pos.1);
    g.monsters[0].pos = far;
    assert!(g.monster_aware(0));
    g.take(ring(ItemKind::RingStealth, 2));
    g.run("equip a");
    assert!(!g.monster_aware(0)); // 9 - 6 = 3 < 5
    g.monsters[0].pos = (g.pos.0 + 3, g.pos.1);
    assert!(g.monster_aware(0));
}

#[test]
fn the_searching_ring_finds_traps_more_surely_and_from_further() {
    let mut g = quiet(3);
    let p = (g.pos.0 + 3, g.pos.1);
    assert!(g.map.tile(p.0, p.1).walkable());
    g.traps.push(Trap {
        pos: p,
        kind: TrapKind::Dart,
        revealed: false,
    });
    g.take(ring(ItemKind::RingSearching, 3));
    g.run("equip a");
    for _ in 0..3 {
        g.run("wait");
    }
    assert!(
        g.traps[0].revealed,
        "探索の指輪でも3マス先の罠が見つからなかった"
    );
}

#[test]
fn see_invisible_ring_shows_invisible_enemies() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.monsters[0].status.apply(Status::Invisible, 1000);
    assert!(g.visible_enemies().is_empty());
    g.take(ring(ItemKind::RingSeeInvisible, 1));
    g.run("equip a");
    assert_eq!(g.visible_enemies().len(), 1);
}

#[test]
fn the_aggravate_ring_pulls_every_enemy_toward_you_and_cannot_be_removed() {
    let mut g = quiet(2);
    let far = (1..W - 1)
        .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
        .find(|&(x, y)| {
            g.map.tile(x, y) == Tile::Floor && (x - g.pos.0).abs() + (y - g.pos.1).abs() > 25
        })
        .unwrap();
    let id = g.add_monster(&crate::monster::SLIME, far);
    assert!(!g.monster_aware(id));
    g.take(ring(ItemKind::RingAggravate, 1));
    let o = g.run("equip a");
    assert!(o.message.contains("呪われていた"), "{}", o.message);
    assert!(g.monster_aware(id));
    let mut moved = false;
    for _ in 0..4 {
        g.run("wait");
        moved |= g.monsters[id].pos != far;
    }
    assert!(moved);
    assert!(!g.run("unequip a").ok);
}

#[test]
fn the_teleportitis_ring_throws_you_around_now_and_then() {
    let mut g = quiet(2);
    g.take(ring(ItemKind::RingTeleportitis, 1));
    g.run("equip a");
    let mut jumps = 0;
    for _ in 0..400 {
        g.hp = g.max_hp;
        g.food = MAX_FOOD;
        if g.run("wait").message.contains("飛ばされた") {
            jumps += 1;
        }
    }
    assert!((3..40).contains(&jumps), "{jumps}");
}

#[test]
fn identify_scroll_reveals_a_ring_and_warns_about_a_curse() {
    let mut g = with_rings(&[ring(ItemKind::RingDamage, -2)]);
    g.take(ItemKind::Identify);
    let o = g.run("read b a");
    assert!(
        o.ok && o.message.contains("ダメージ増加の指輪 -2") && o.message.contains("呪われている"),
        "{}",
        o.message
    );
    assert!(g.known[ItemKind::RingDamage.index()]);
    // 呪いと分かっていれば、はめる前に避けられる
    assert!(g.inventory_lines()[0].contains("(呪われている)"));
}

#[test]
fn rolled_rings_carry_a_strength_and_some_are_cursed() {
    let mut rng = Rng::new(3);
    let (mut cursed, mut total) = (0, 0);
    for k in ItemKind::ALL.into_iter().filter(|k| k.is_ring()) {
        for _ in 0..80 {
            let t = Tool::roll(&mut rng, k);
            let r = k.ring_def().unwrap();
            total += 1;
            assert!(!t.identified);
            if r.always_cursed {
                assert!(t.cursed);
            } else if !r.cursable {
                assert!(!t.cursed && t.val >= 0, "{k:?} {t:?}");
            } else {
                assert_eq!(t.cursed, t.val < 0, "{k:?}");
                assert!((1..=3).contains(&t.val.abs()));
            }
            cursed += t.cursed as u32;
        }
    }
    assert!(
        cursed * 10 > total && cursed * 2 < total,
        "{cursed}/{total}"
    );
}

// ---- 光源 ----

#[test]
fn you_start_with_a_lit_torch_and_its_fuel_is_always_shown() {
    let g = quiet(2);
    assert!(
        g.status_text().contains("光源:松明 燃料1500/1500"),
        "{}",
        g.status_text()
    );
    assert!(g
        .observe_text(3)
        .contains("光源: 松明 [燃料 1500/1500] (装備中)"));
    assert!(g.light_line().contains("松明"));
    let mut g = quiet(2);
    assert!(g
        .run("inventory")
        .message
        .contains("光源: 松明 [燃料 1500/1500]"));
}

#[test]
fn fuel_burns_each_turn_warns_when_low_and_darkens_when_out() {
    let mut g = quiet(2);
    g.light = Some(Tool::charged(ItemKind::Torch, 103));
    assert!(g.map.is_visible(g.pos.0 + 6, g.pos.1) || !g.map.tile(g.pos.0 + 6, g.pos.1).walkable());
    let mut warned = false;
    for _ in 0..3 {
        warned |= g.run("wait").message.contains("火が弱くなってきた");
    }
    assert!(warned);
    assert_eq!(g.light.unwrap().val, 100);
    for _ in 0..99 {
        g.run("wait");
    }
    let o = g.run("wait");
    assert!(o.message.contains("燃え尽きた"), "{}", o.message);
    assert_eq!(g.light.unwrap().val, 0);
    assert!(g.status_text().contains("暗闇"));
    // 視界が狭まる: 半径2より遠くは見えない
    let far = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| g.map.is_visible(x, y))
        .all(|(x, y)| {
            (x - g.pos.0).pow(2) + (y - g.pos.1).pow(2) <= crate::item::DARK_RADIUS.pow(2)
        });
    assert!(far);
    // 燃料は減り続けない
    g.run("wait");
    assert_eq!(g.light.unwrap().val, 0);
}

#[test]
fn running_out_of_light_interrupts_auto_walk() {
    let mut g = quiet(2);
    g.light = Some(Tool::charged(ItemKind::Torch, 5));
    let o = g.run("explore");
    assert!(o.message.contains("明かりが消えて中断"), "{}", o.message);
}

#[test]
fn swapping_lights_returns_the_old_one_and_a_lantern_can_be_refilled() {
    let mut g = quiet(2);
    g.light = Some(Tool::charged(ItemKind::Torch, 40));
    g.take(Tool::charged(ItemKind::Lantern, 1000));
    let o = g.run("equip a");
    assert!(o.ok && o.message.contains("ランタン"), "{}", o.message);
    assert_eq!(g.light.unwrap().kind, ItemKind::Lantern);
    // 古い松明が同じ文字に戻る
    assert!(
        g.inventory_lines()[0].starts_with("a) 松明 [燃料"),
        "{:?}",
        g.inventory_lines()
    );
    // 油を継ぎ足す
    let o = g.run("refill");
    assert!(
        !o.ok && o.message.contains("油つぼを持っていない"),
        "{}",
        o.message
    );
    g.take(ItemKind::OilFlask);
    g.take(ItemKind::OilFlask);
    let t = g.turn();
    let o = g.run("refill");
    assert!(o.ok && g.turn() == t + 1, "{}", o.message);
    let fuel = g.light.unwrap().val;
    assert!((2490..=2500).contains(&fuel), "{fuel}");
    assert!(g
        .inventory_lines()
        .iter()
        .any(|l| l.contains("油つぼ") && !l.contains("x2")));
    // 満タンを超えない
    g.run("refill");
    let max = ItemKind::Lantern.light_def().unwrap().max_fuel;
    assert!(g.light.unwrap().val <= max);
    g.light = Some(Tool::charged(ItemKind::Lantern, max));
    let o = g.run("refill");
    assert!(!o.ok && o.message.contains("満タン"), "{}", o.message);
}

#[test]
fn a_torch_cannot_be_refilled_and_refilling_a_dead_lantern_relights_the_room() {
    let mut g = quiet(2);
    g.take(ItemKind::OilFlask);
    let o = g.run("refill");
    assert!(!o.ok && o.message.contains("継ぎ足せない"), "{}", o.message);
    g.light = Some(Tool::charged(ItemKind::Lantern, 0));
    g.refresh_fov();
    assert!(!g.map.is_visible(g.pos.0 + 5, g.pos.1));
    let o = g.run("refill");
    assert!(o.ok && o.message.contains("明るくなった"), "{}", o.message);
    assert!(g.map.is_visible(g.pos.0 + 1, g.pos.1));
    assert!(!g.run("refill").ok);
}

#[test]
fn dark_does_not_blind_the_monsters() {
    let mut g = with_adjacent(3, &crate::monster::GOBLIN);
    g.light = Some(Tool::charged(ItemKind::Torch, 0));
    g.refresh_fov();
    let far = (g.pos.0 + 5, g.pos.1);
    if g.map.tile(far.0, far.1).walkable() {
        g.monsters[0].pos = far;
        assert!(g.monster_aware(0));
        assert!(g.visible_enemies().is_empty(), "暗闇で遠くの敵が見えている");
    }
}

#[test]
fn lights_and_oil_appear_on_the_floor_and_oil_stacks() {
    let mut found = (false, false, false);
    for seed in 0..200 {
        let mut g = Game::new(seed);
        g.depth = 4;
        g.spawn_items();
        for f in &g.floor_items {
            match f.item.kind() {
                ItemKind::Torch => found.0 = true,
                ItemKind::Lantern => found.1 = true,
                ItemKind::OilFlask => found.2 = true,
                _ => {}
            }
        }
    }
    assert!(found.0 && found.1 && found.2, "{found:?}");
    let mut g = quiet(2);
    g.take(ItemKind::OilFlask);
    g.take(ItemKind::OilFlask);
    assert_eq!(g.inventory.len(), 1);
    assert!(g.inventory_lines()[0].contains("油つぼ x2"));
    // 光源は重ならず、燃料の量が分かる
    g.take(Tool::charged(ItemKind::Torch, 700));
    g.take(Tool::charged(ItemKind::Torch, 300));
    let lines = g.inventory_lines();
    assert!(
        lines[1].contains("[燃料 700/1500]") && lines[2].contains("[燃料 300/1500]"),
        "{lines:?}"
    );
}

/// 全種類のアイテムを持たせて乱暴に遊び、全メッセージを返す。途中で不変条件も確かめる。
fn fuzz_play(seed: u64, steps: usize) -> Vec<String> {
    let verbs = [
        "quaff", "read", "eat", "equip", "unequip", "drop", "zap", "refill",
    ];
    let dirs = [
        "north",
        "south",
        "east",
        "west",
        "northeast",
        "northwest",
        "southeast",
        "southwest",
        "nearest",
        "",
    ];
    let mut out = Vec::new();
    let mut g = Game::new(seed);
    let mut r = Rng::new(seed ^ 0xABCDEF);
    for k in ItemKind::ALL {
        if g.has_free_letter() {
            g.take(Item::roll(&mut r, k, 10));
        }
    }
    for step in 0..steps {
        if g.is_dead() || g.is_won() {
            break;
        }
        let letter = (b'a' + r.range(0, 26) as u8) as char;
        let cmd = match r.range(0, 12) {
            0 | 1 => format!(
                "{} {letter}",
                verbs[r.range(0, verbs.len() as i32) as usize]
            ),
            2 => format!(
                "zap {letter} {}",
                dirs[r.range(0, dirs.len() as i32) as usize]
            ),
            3 => "explore".to_string(),
            4 => "travel >; descend".to_string(),
            5 => "pickup".to_string(),
            6 => format!("read {letter} {}", (b'a' + r.range(0, 26) as u8) as char),
            7 => "stay 5".to_string(),
            8 => "inventory; look".to_string(),
            _ => format!("move {}", dirs[r.range(0, 8) as usize]),
        };
        for o in g.run_script(&cmd) {
            assert!(
                o.hp <= g.max_hp(),
                "seed {seed} step {step}: {} -> hp {}",
                o.command,
                o.hp
            );
            out.push(format!(
                "{} | {} | {} | {}",
                o.command, o.ok, o.message, o.turn
            ));
        }
        let text = g.observe_text(5);
        assert!(text.contains("光源:"), "{text}");
        out.push(text);
        // 装備している物は、必ず持ち物にある
        for l in [g.weapon, g.armor, g.rings[0], g.rings[1]]
            .into_iter()
            .flatten()
        {
            assert!(
                g.inventory.iter().any(|s| s.letter == l),
                "seed {seed}: 装備 {l} が持ち物にない"
            );
        }
        assert!(g.rings[0].is_none() || g.rings[0] != g.rings[1]);
    }
    out
}

#[test]
fn random_play_with_every_item_never_panics_and_stays_consistent() {
    for seed in 0..40u64 {
        fuzz_play(seed, 400);
    }
}

#[test]
fn the_same_seed_and_script_always_give_the_same_game() {
    for seed in [1u64, 7, 23, 31] {
        assert_eq!(fuzz_play(seed, 300), fuzz_play(seed, 300), "seed {seed}");
    }
}

#[test]
fn teleportitis_during_auto_walk_stops_it_and_still_picks_up_what_you_land_on() {
    let (mut jumped, mut landed) = (0, 0);
    for seed in 0..40 {
        let mut g = Game::new(seed);
        g.monsters.clear();
        g.traps.clear();
        g.hp = 1000;
        g.max_hp = 1000;
        g.take(ring(ItemKind::RingTeleportitis, 1));
        g.run("equip a");
        // 床のどこに飛んでも品物がある状態にする(拾いが必ず問われる)
        let spots: Vec<(i32, i32)> = (1..W - 1)
            .flat_map(|x| (1..H - 1).map(move |y| (x, y)))
            .filter(|&(x, y)| g.map.tile(x, y) == Tile::Floor && (x, y) != g.pos)
            .collect();
        for p in spots {
            g.floor_items.push(FloorItem::new(p, ItemKind::OilFlask));
        }
        for cmd in ["explore", "explore", "travel >", "explore"] {
            g.food = MAX_FOOD;
            let o = g.run(cmd);
            if o.message.contains("飛ばされて中断") || o.message.contains("飛ばされた")
            {
                jumped += 1;
                // 着いた場所に物があれば、飛んだ時点で拾っている(足元に取り残さない)
                // (1マスにつき自動で拾うのは1個だけ。飛んだ直後に拾った知らせが続く)
                if let Some((_, after)) = o.message.rsplit_once("突然どこかへ飛ばされた！")
                {
                    // 拾っていないなら、そのマスにはもう何も残っていない(通った場所に飛んだ)
                    assert!(
                        after.contains("拾った")
                            || after.contains("いっぱい")
                            || g.floor_order(g.pos).is_empty(),
                        "seed {seed}: {}",
                        o.message
                    );
                    landed += 1;
                }
            }
        }
    }
    assert!(
        jumped > 0 && landed > 0,
        "テレポート癖が発動しなかった({jumped}/{landed})"
    );
}

#[test]
fn disarm_removes_a_known_trap_with_some_probability() {
    let (mut ok, mut fail) = (0, 0);
    for seed in 0..60 {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.rng = Rng::new(seed);
        let p = put_trap(&mut g, TrapKind::SleepGas);
        g.traps[0].revealed = true;
        let t = g.turn();
        let o = g.run("disarm east");
        assert!(o.ok && g.turn() > t, "{}", o.message);
        if o.message.contains("解除した") {
            assert!(g.traps.is_empty());
            assert_ne!(g.cell(p.0, p.1).ch, '^');
            ok += 1;
        } else {
            assert!(
                o.message.contains("失敗") && g.traps.len() == 1,
                "{}",
                o.message
            );
            fail += 1;
        }
    }
    assert!(ok > 20 && fail > 5, "{ok} {fail}");
}

#[test]
fn disarm_needs_a_known_trap_and_sight_and_can_trigger_on_failure() {
    let mut g = quiet(3);
    put_trap(&mut g, TrapKind::Dart); // 隠れている
    let t = g.turn();
    let o = g.run("disarm east");
    assert!(
        !o.ok && o.message.contains("見つけた罠がない") && g.turn() == t,
        "{}",
        o.message
    );
    g.traps[0].revealed = true;
    g.status.apply(Status::Blind, 10);
    assert!(!g.run("disarm east").ok);
    g.status.clear(Status::Blind);
    // 失敗すると罠が作動することがある
    let mut fired = false;
    for seed in 0..80 {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.rng = Rng::new(seed);
        put_trap(&mut g, TrapKind::Dart);
        g.traps[0].revealed = true;
        let o = g.run("disarm east");
        if o.message.contains("作動した") {
            fired = true;
            assert!(
                g.hp < 1000 && g.status.has(Status::Poisoned),
                "{}",
                o.message
            );
            break;
        }
    }
    assert!(fired);
    // 器用さの指輪で成功率が上がる
    let mut g = quiet(3);
    let base = g.disarm_percent();
    g.take(ring(ItemKind::RingDexterity, 3));
    g.run("equip a");
    assert!(g.disarm_percent() > base);
}

#[test]
fn a_failed_disarm_next_to_a_trapdoor_never_drops_you() {
    for seed in 0..60 {
        let mut g = quiet(3);
        g.rng = Rng::new(seed);
        put_trap(&mut g, TrapKind::Trapdoor);
        g.traps[0].revealed = true;
        g.run("disarm east");
        assert_eq!(g.depth(), 1);
    }
}

/// 失敗して罠が作動する seed を探して、そのゲームを返す(作動前の状態)。
fn disarm_failure_that_fires(
    kind: TrapKind,
    under_foot: bool,
    setup: impl Fn(&mut Game),
) -> Option<(Game, Outcome)> {
    for seed in 0..300 {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        g.rng = Rng::new(seed);
        let p = put_trap(&mut g, kind);
        g.traps[0].revealed = true;
        if under_foot {
            g.pos = p;
        }
        setup(&mut g);
        let o = g.run(if under_foot { "disarm" } else { "disarm east" });
        if o.message.contains("作動した！") {
            return Some((g, o));
        }
    }
    None
}

#[test]
fn a_failed_disarm_of_the_trapdoor_underfoot_drops_you() {
    let (g, o) =
        disarm_failure_that_fires(TrapKind::Trapdoor, true, |_| {}).expect("作動する seed がある");
    assert_eq!(g.depth(), 2, "{}", o.message);
}

#[test]
fn a_slipped_disarm_fires_darts_and_gas_even_while_floating_but_never_drops_a_floating_player() {
    // 毒矢と眠りガスは、浮いていても足元でも隣でも作動する
    for (kind, foot) in [
        (TrapKind::Dart, true),
        (TrapKind::Dart, false),
        (TrapKind::SleepGas, true),
        (TrapKind::SleepGas, false),
    ] {
        let (g, o) = disarm_failure_that_fires(kind, foot, |g| {
            g.status.apply(Status::Levitating, 1000);
        })
        .unwrap_or_else(|| panic!("{kind:?} foot={foot}: 作動する seed がない"));
        let effect = match kind {
            TrapKind::Dart => g.hp < 1000 && g.status.has(Status::Poisoned),
            // 失敗メッセージ自体にも「眠りガス」と入るので、実際に眠って起きたことで確かめる
            _ => o.message.contains("ぐっすり眠って") && o.message.contains("目が覚めた"),
        };
        assert!(effect, "{kind:?} foot={foot}: {}", o.message);
    }
    // 落とし穴は、浮いていれば足元でも落ちない
    let mut slipped = false;
    for seed in 0..120 {
        let mut g = quiet(3);
        g.rng = Rng::new(seed);
        let p = put_trap(&mut g, TrapKind::Trapdoor);
        g.traps[0].revealed = true;
        g.pos = p;
        g.status.apply(Status::Levitating, 1000);
        let o = g.run("disarm");
        assert_eq!(g.depth(), 1, "{}", o.message);
        slipped |= o.message.contains("浮いているので落ちなかった");
    }
    assert!(slipped);
}

#[test]
fn levitation_walks_over_every_kind_of_trap() {
    for kind in TrapKind::ALL {
        let mut g = quiet(3);
        g.hp = 1000;
        g.max_hp = 1000;
        let p = put_trap(&mut g, kind);
        g.status.apply(Status::Levitating, 100);
        let o = g.run("move east");
        assert_eq!(
            (g.depth(), g.pos(), g.hp),
            (1, p, 1000),
            "{kind:?}: {}",
            o.message
        );
        assert!(
            !g.status.has(Status::Poisoned) && !g.status.has(Status::Asleep),
            "{kind:?}"
        );
    }
}

#[test]
fn auto_walk_goes_through_known_traps_only_while_floating() {
    for kind in TrapKind::ALL {
        let mut g = quiet(3);
        let p = put_trap(&mut g, kind);
        g.traps[0].revealed = true;
        g.map.mark_seen(p.0, p.1);
        assert!(g.find_path(&|q| q == p).is_none(), "{kind:?}");
        g.status.apply(Status::Levitating, 100);
        assert!(g.find_path(&|q| q == p).is_some(), "{kind:?}");
    }
}

#[test]
fn confusion_lowers_the_disarm_chance_and_the_message_shows_it() {
    let mut g = quiet(3);
    let base = g.disarm_percent();
    g.status.apply(Status::Confused, 50);
    assert_eq!(g.disarm_percent(), base - 30);
    let p = put_trap(&mut g, TrapKind::SleepGas);
    g.traps[0].revealed = true;
    let _ = p;
    g.status.clear(Status::Confused);
    let o = g.run("disarm east");
    assert!(
        o.message.contains(&format!("成功率{base}%")),
        "{}",
        o.message
    );
}

#[test]
fn disarming_a_known_trap_lets_auto_walk_path_through_it() {
    let mut g = quiet(3);
    let p = put_trap(&mut g, TrapKind::Dart);
    g.traps[0].revealed = true;
    g.map.mark_seen(p.0, p.1);
    assert!(
        g.find_path(&|q| q == p).is_none(),
        "既知の罠は経路から外れる"
    );
    g.hp = 1000;
    g.max_hp = 1000;
    for seed in 0..100 {
        g.rng = Rng::new(seed);
        if g.run("disarm east").message.contains("解除した") {
            break;
        }
    }
    assert!(g.traps.is_empty());
    assert!(
        g.find_path(&|q| q == p).is_some(),
        "解除した罠のマスには入れる"
    );
}

#[test]
fn disarm_rejects_bad_directions_with_the_same_wording_as_zap() {
    assert!(crate::command::parse("disarm sideways")
        .unwrap_err()
        .contains("不明な向き"));
    assert!(crate::command::parse("zap a sideways")
        .unwrap_err()
        .contains("不明な向き"));
}

#[test]
fn travel_stops_before_a_known_trap_when_levitation_wears_off_on_the_way() {
    let mut tested = false;
    for seed in 0..60 {
        let mut g = quiet(seed);
        g.hp = 1000;
        g.max_hp = 1000;
        g.map.reveal_all();
        let Some(path) = g.find_path(&|p| p == g.stairs) else {
            continue;
        };
        if path.len() < 6 {
            continue;
        }
        // 経路の4歩目に既知の罠を置き、浮遊は3歩ぶんだけ持たせる
        let trap_at = path[3];
        g.traps.push(Trap {
            pos: trap_at,
            kind: TrapKind::Dart,
            revealed: true,
        });
        g.status.apply(Status::Levitating, 3);
        let o = g.run("travel >");
        assert_ne!(g.pos(), trap_at, "seed {seed}: {}", o.message);
        assert!(
            !g.status.has(Status::Poisoned) && g.hp == 1000,
            "seed {seed}: {}",
            o.message
        );
        assert!(
            o.message.contains("毒矢の罠がある"),
            "seed {seed}: {}",
            o.message
        );
        tested = true;
        break;
    }
    assert!(tested, "条件に合う seed がない");
}

#[test]
fn disarm_without_a_direction_finds_a_single_adjacent_trap() {
    let mut g = quiet(3);
    g.hp = 1000;
    g.max_hp = 1000;
    put_trap(&mut g, TrapKind::SleepGas);
    // 見つけていない罠は対象にならない
    assert!(!g.run("disarm").ok);
    g.traps[0].revealed = true;
    let o = g.run("disarm");
    assert!(o.ok && o.message.contains("眠りガスの罠"), "{}", o.message);
}

#[test]
fn disarm_without_a_direction_asks_when_several_traps_are_adjacent() {
    let mut g = quiet(3);
    put_trap(&mut g, TrapKind::SleepGas);
    let (x, y) = g.pos;
    let other = [(x - 1, y), (x, y - 1), (x, y + 1)]
        .into_iter()
        .find(|p| g.map.tile(p.0, p.1).walkable())
        .expect("隣に歩ける床がある");
    g.traps.push(Trap {
        pos: other,
        kind: TrapKind::Dart,
        revealed: true,
    });
    g.traps[0].revealed = true;
    let t = g.turn();
    let o = g.run("disarm");
    assert!(!o.ok && o.message.contains("向きを指定"), "{}", o.message);
    assert_eq!(g.turn(), t);
}

#[test]
fn the_log_is_trimmed_but_keeps_the_latest_entries() {
    let mut g = quiet(3);
    for i in 0..(LOG_KEEP * 5) {
        g.push_log(&format!("log {i}"));
    }
    assert!(g.log().len() < LOG_KEEP * 2, "{}", g.log().len());
    assert!(g.log().len() >= LOG_KEEP);
    // 末尾は最新のまま、並びも崩れない
    let last = g.log().last().unwrap();
    assert_eq!(last.text, format!("log {}", LOG_KEEP * 5 - 1));
    let n = g.log().len();
    assert_eq!(g.log()[n - 2].text, format!("log {}", LOG_KEEP * 5 - 2));
    // 観測は末尾の数件を読むので、切り詰めても変わらない
    assert!(g.observe_text(3).contains(&last.text));
}

#[test]
fn look_names_a_seen_amulet_as_its_own_sentence() {
    let mut g = quiet(3);
    let p = (g.pos.0 + 1, g.pos.1);
    assert!(g.map.tile(p.0, p.1).walkable());
    g.amulet = Some(p);
    g.map.mark_seen(p.0, p.1);
    let o = g.run("look");
    assert!(
        o.ok && o.message.contains("魔除けのアミュレットが東に1にある。"),
        "{}",
        o.message
    );
    assert!(!o.message.contains(", "), "{}", o.message);
}

#[test]
fn stay_counts_actions_like_wait_so_haste_and_slow_apply() {
    let mut g = quiet(3);
    g.status.apply(Status::Hasted, 100);
    let t = g.turn();
    assert!(g.run("stay 4").ok);
    assert_eq!(g.turn(), t + 2, "加速中は4回の行動で2ターン");

    let mut g = quiet(3);
    g.status.apply(Status::Slowed, 100);
    let t = g.turn();
    assert!(g.run("stay 2").ok);
    assert_eq!(g.turn(), t + 4, "減速中は2回の行動で4ターン");
}
