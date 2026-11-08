//! Super ship damage phases against the real Remake data: a destroyed phase gives way to its `Destruction_Survivors` phase in the same
//! battle. Skipped when the game is not installed.
//!
//! Before: a destroyed Viscount_Star_Defender_P1 was gone; the P2 to P5 phases (and the Subjugator_Zann_P3 ion turrets) never fought.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

/// `phase` alone on side 0 against one Star Destroyer far away (so the battle does not end), no AI.
fn battle(data: &alamo_data::GameData, fs: &GameFs, phase: &str) -> Battle {
    let cat = Arc::new(Catalog::build(data, Some(fs as &dyn FileSource), &[phase, "Star_Destroyer"]));
    let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: 1, tag: 0 };
    let sides = vec![
        SideSetup { name: "Rebel".into(), units: vec![unit(phase)], structures: vec![], ai: false },
        SideSetup { name: "Empire".into(), units: vec![unit("Star_Destroyer")], structures: vec![], ai: false },
    ];
    let mut b = Battle::new(BattleSetup { catalog: cat, sides, config: BattleConfig { seed: 1, fog_of_war: false, ..BattleConfig::default() } });
    for s in b.ships.iter_mut().filter(|s| s.side == 1) {
        s.pos = Vec3::new(40000.0, 40000.0, 0.0);
        s.stance = Stance::HoldFire;
    }
    b
}

/// The living, not launched ship of side 0 whose definition name starts with `prefix`.
fn phase_ship(b: &Battle, prefix: &str) -> Option<u32> {
    let cat = b.catalog();
    b.ships.iter().find(|s| s.alive && s.side == 0 && s.carrier == NONE && cat.ship(s.def).name.starts_with(prefix)).map(|s| s.id)
}

#[test]
fn phase_survivors_come_from_the_data_and_dummies_are_not_ships() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs as &dyn FileSource), &["Viscount_Star_Defender_P1", "Subjugator_Zann_P1", "Piett_Executor_P1", "TIE_Fighter"]);
    let next = |n: &str| -> Vec<(String, u32)> { cat.ship(cat.ship_id(n).unwrap()).survivors.iter().map(|&(d, c)| (cat.ship(d).name.clone(), c)).collect() };
    assert_eq!(next("Viscount_Star_Defender_P1"), [("Viscount_Star_Defender_P2".to_string(), 1)]);
    assert_eq!(next("Viscount_Star_Defender_P4"), [("Viscount_Star_Defender_P5".to_string(), 1)]);
    assert!(next("Viscount_Star_Defender_P5").is_empty(), "the last phase leaves a Death_Clone");
    assert_eq!(next("Subjugator_Zann_P2"), [("Subjugator_Zann_P3".to_string(), 1)]);
    assert!(next("Subjugator_Zann_P4").is_empty());
    assert_eq!(next("Piett_Executor_P4"), [("Piett_Executor_P5".to_string(), 1)]);
    // The TIE's `Destruction_Survivors` is its death penalty dummy (-20 credits), not a ship.
    assert!(next("TIE_Fighter").is_empty());
}

#[test]
fn a_destroyed_viscount_goes_through_its_five_phases_in_one_battle() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, "Viscount_Star_Defender_P1");
    b.step();
    let mut ship = phase_ship(&b, "Viscount_Star_Defender_P1").expect("P1 in battle");
    let order = Order::Move(Vec3::new(-3000.0, 1000.0, 0.0));
    b.ships[ship as usize].order = order;
    let mut seen = vec!["Viscount_Star_Defender_P1".to_string()];
    for phase in 2..=5 {
        // Destroyed where it is.
        let (pos, yaw) = (b.ships[ship as usize].pos, b.ships[ship as usize].yaw);
        b.ships[ship as usize].alive = false;
        let spawned: Vec<u32> = b.step().iter().filter_map(|e| match *e {
            Event::PhaseSpawned { ship: s, into } if s == ship => Some(into),
            _ => None,
        }).collect();
        assert_eq!(spawned.len(), 1, "phase {phase}");
        let next = spawned[0];
        let cat = b.catalog().clone();
        let s = &b.ships[next as usize];
        let def = cat.ship(s.def);
        assert_eq!(def.name, format!("Viscount_Star_Defender_P{phase}"));
        assert!(s.pos.dist(pos) < def.max_speed + 1.0, "in place: {:?} vs {pos:?}", s.pos);
        assert!((s.yaw - yaw).abs() < 0.2, "same facing");
        assert_eq!(s.order, order, "keeps the orders");
        assert!(s.hull > 0.99 * def.hull, "full strength: {} of {}", s.hull, def.hull);
        assert!(!b.is_over(), "the side is not beaten");
        seen.push(def.name.clone());
        ship = next;
    }
    eprintln!("phases: {seen:?}");
    // The last phase leaves no further ship; the battle is lost.
    b.ships[ship as usize].alive = false;
    assert!(!b.step().iter().any(|e| matches!(e, Event::PhaseSpawned { .. })));
    for _ in 0..60 {
        b.step();
    }
    assert_eq!(b.winner(), Some(1));
    // The phases do not count as units the galaxy sent: the one results group of side 0 is P1, lost.
    let r = b.result();
    let sent: Vec<&GroupResult> = r.groups.iter().filter(|g| g.side == 0 && g.tag != PHASE_TAG && b.catalog().ship(g.ship).name.starts_with("Viscount")).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].survived, 0);
}

#[test]
fn subjugator_zann_p3_brings_its_ion_turrets_and_fights() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, "Subjugator_Zann_P2");
    b.step();
    let p2 = phase_ship(&b, "Subjugator_Zann_P2").expect("P2");
    b.ships[p2 as usize].alive = false;
    b.step();
    let p3 = phase_ship(&b, "Subjugator_Zann_P3").expect("P3 after P2 is destroyed");
    let cat = b.catalog().clone();
    let def = cat.ship(b.ships[p3 as usize].def);
    let weapons = def.hardpoints.iter().filter(|h| h.kind == HpKind::Weapon).count();
    eprintln!("Subjugator_Zann_P3: {} hardpoints, {weapons} weapons", def.hardpoints.len());
    assert!(weapons > 0, "the phase is armed");
}
