//! Units that object scripts bring into a battle (`spawns.rs`): hand-made definitions here, the Remake's scripts further down.

use std::sync::Arc;

use battle_sim::*;

fn ship(name: &str, class: TargetClass, radius: f32) -> ShipDef {
    ShipDef { name: name.into(), class, hull: 1000.0, radius, max_speed: 100.0, turn_rate: 1.0, accel: 50.0, ..ShipDef::default() }
}

fn gun() -> HpDef {
    HpDef {
        name: "gun".into(),
        kind: HpKind::Weapon,
        projectile: Some(0),
        range: 3000.0,
        arc_yaw: std::f32::consts::PI,
        arc_pitch: std::f32::consts::FRAC_PI_2,
        half_yaw: std::f32::consts::PI,
        half_pitch: std::f32::consts::FRAC_PI_2,
        recharge_min: 0.5,
        recharge_max: 0.5,
        pulses: 1,
        forward: Vec3::new(1.0, 0.0, 0.0),
        left: Vec3::new(0.0, 1.0, 0.0),
        normal: Vec3::new(0.0, 0.0, 1.0),
        ..HpDef::default()
    }
}

/// Escort script of the shape the Remake's hero scripts have: two escorts at the hero, 90 frames of hyperspace, 1 + 0..0.9 s apart.
fn escort_script(faction: &str, place: SpawnPlace, escort: ShipId) -> Arc<SpawnScriptDef> {
    Arc::new(SpawnScriptDef {
        script: "Test_Escort".into(),
        choices: vec![vec![escort, escort]],
        place,
        faction: faction.into(),
        delay: 0.0,
        hyperspace_in: 3.0,
        gap: SpawnGap { base: 1.0, lo: 0, hi: 9, div: 10.0 },
    })
}

fn units(cat: &Catalog, name: &str, list: &[&str]) -> SideSetup {
    SideSetup { name: name.into(), units: list.iter().map(|n| UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 7 }).collect(), structures: vec![], ai: false }
}

fn config(seed: u64) -> BattleConfig {
    BattleConfig { fog_of_war: false, seed, ..BattleConfig::default() }
}

/// `(battle time, event)` of every `Event::Spawned` within `secs`.
fn spawned(b: &mut Battle, secs: f32) -> Vec<(f32, u8, Vec<u32>, u32, bool)> {
    let mut out = Vec::new();
    while b.time() < secs && !b.is_over() {
        let t = b.time() + b.config().dt;
        for e in b.step() {
            if let Event::Spawned { side, ships, by, hyperspace } = e {
                out.push((t, *side, ships.clone(), *by, *hyperspace));
            }
        }
    }
    out
}

fn escort_catalog(faction: &str) -> Arc<Catalog> {
    let mut hero = ship("Hero", TargetClass::Capital, 300.0);
    hero.spawn_script = Some(escort_script(faction, SpawnPlace::ObjectsOfType("hero".into()), 1));
    Arc::new(Catalog::from_defs(vec![hero, ship("Escort", TargetClass::Frigate, 130.0), ship("Foe", TargetClass::Frigate, 130.0)], vec![ProjDef::default()]))
}

#[test]
fn hero_script_brings_its_escorts_in_one_by_one_after_the_hyperspace_delay() {
    let cat = escort_catalog("Rebel");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    let got = spawned(&mut b, 8.0);
    assert_eq!(got.len(), 2, "{got:?}");
    // The script starts on the first step (1/30 s); each escort arrives 90 frames after its spawn, the second 1.0 to 1.9 s after the first.
    assert!((got[0].0 - (3.0 + 1.0 / 30.0)).abs() < 0.04, "first at {}", got[0].0);
    let gap = got[1].0 - got[0].0;
    assert!((0.99..=1.94).contains(&gap), "gap {gap}");
    let hero = b.ships.iter().find(|s| s.def == 0).unwrap();
    for (_, side, ships, by, hyperspace) in &got {
        assert_eq!((*side, *by, *hyperspace, ships.len()), (0, hero.id, true, 1));
        let e = &b.ships[ships[0] as usize];
        assert_eq!(e.def, 1);
        // Beside the hero, not inside it.
        let d = e.pos.dist(hero.pos);
        assert!((300.0..1500.0).contains(&d), "escort {d} from the hero");
    }
    assert!(b.ships[got[0].2[0] as usize].pos.dist(b.ships[got[1].2[0] as usize].pos) > 150.0, "escorts side by side");
}

#[test]
fn script_units_are_battle_only() {
    let cat = escort_catalog("Rebel");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    spawned(&mut b, 6.0);
    assert_eq!(b.ships.iter().filter(|s| s.def == 1).count(), 2);
    // No results group: the galaxy gets nothing back for them.
    assert!(b.result().groups.iter().all(|g| g.ship != 1), "{:?}", b.result().groups);
}

#[test]
fn script_units_join_the_side_of_their_faction() {
    let cat = escort_catalog("Empire");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    assert!(spawned(&mut b, 6.0).iter().all(|e| e.1 == 1));
    // A faction that is not in the battle: the units fight for the script's owner.
    let cat = escort_catalog("Pirates");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    let got = spawned(&mut b, 6.0);
    assert!(!got.is_empty() && got.iter().all(|e| e.1 == 0));
}

#[test]
fn the_roll_picks_one_list_from_the_seed() {
    let mut hero = ship("Hero", TargetClass::Capital, 300.0);
    let mut script = (*escort_script("Rebel", SpawnPlace::ObjectsOfType("hero".into()), 1)).clone();
    script.choices = vec![vec![1], vec![2, 2], vec![]];
    hero.spawn_script = Some(Arc::new(script));
    let cat = Arc::new(Catalog::from_defs(vec![hero, ship("A", TargetClass::Frigate, 130.0), ship("B", TargetClass::Corvette, 60.0), ship("Foe", TargetClass::Frigate, 130.0)], vec![ProjDef::default()]));
    let run = |seed: u64| {
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(seed) });
        spawned(&mut b, 8.0).iter().map(|e| b.ships[e.2[0] as usize].def).collect::<Vec<_>>()
    };
    let outcomes: Vec<Vec<ShipId>> = (1..40).map(run).collect();
    assert_eq!(run(5), run(5), "same seed, same roll");
    for o in &outcomes {
        assert!(o.is_empty() || *o == vec![1] || *o == vec![2, 2], "{o:?}");
    }
    for want in [vec![], vec![1], vec![2, 2]] {
        assert!(outcomes.contains(&want), "outcome {want:?} never rolled");
    }
}

#[test]
fn a_hardpoint_breaking_off_runs_its_prop_script_at_the_map_markers() {
    let mut target = ship("Target", TargetClass::Capital, 300.0);
    let mut hp = HpDef { name: "turret".into(), kind: HpKind::Other, health: 50.0, destroyable: true, targetable: true, ..HpDef::default() };
    let mut s = (*escort_script("Rebel", SpawnPlace::DefenderPositions, 1)).clone();
    s.choices = vec![vec![1, 1, 1]];
    hp.breakoff_script = Some(Arc::new(s));
    // A second, sturdy hardpoint keeps the target alive.
    let hull = HpDef { name: "hull".into(), kind: HpKind::Other, health: 1.0e6, destroyable: true, ..HpDef::default() };
    target.hardpoints = vec![hp, hull];
    target.max_speed = 0.0;
    let mut shooter = ship("Shooter", TargetClass::Frigate, 130.0);
    shooter.hardpoints = vec![gun()];
    let proj = ProjDef { name: "laser".into(), damage: 100.0, speed: 3000.0, max_distance: 5000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() };
    let cat = Arc::new(Catalog::from_defs(vec![target, ship("Reinforcement", TargetClass::Frigate, 130.0), shooter], vec![proj]));
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Empire", &["Shooter"]), units(&cat, "Rebel", &["Target"])], config: config(2) });
    // Put the shooter next to its target.
    let t = b.ships.iter().find(|s| s.def == 0).unwrap().pos;
    let sh = b.ships.iter().position(|s| s.def == 2).unwrap();
    b.ships[sh].pos = t + Vec3::new(-1200.0, 0.0, 0.0);
    assert!(spawned(&mut b, 2.0).is_empty(), "nothing before the hardpoint breaks");
    let got = spawned(&mut b, 20.0);
    assert_eq!(got.len(), 3, "{got:?}");
    // Without a map the defender positions are side 1's start place.
    let start = b.ships.iter().find(|s| s.def == 0).unwrap().pos;
    for e in &got {
        assert_eq!(e.1, 1);
        assert!(b.ships[e.2[0] as usize].pos.dist(start) < 3000.0);
    }
}
