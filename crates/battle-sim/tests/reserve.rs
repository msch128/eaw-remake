//! A side's reserve (units of its fleets waiting outside the battle) belongs to the battle: the side is beaten only when its units on the
//! field and its reserve are gone or have retreated. Hand-made definitions (no game data needed).

use std::sync::Arc;

use battle_sim::*;

fn laser() -> ProjDef {
    ProjDef { name: "laser".into(), category: "laser".into(), damage: 100.0, speed: 2000.0, max_distance: 30_000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() }
}

fn gun(range: f32) -> HpDef {
    HpDef {
        name: "g".into(),
        kind: HpKind::Weapon,
        targetable: true,
        offset: Vec3::new(10.0, 0.0, 0.0),
        forward: Vec3::new(1.0, 0.0, 0.0),
        projectile: Some(0),
        range,
        half_yaw: std::f32::consts::PI,
        half_pitch: std::f32::consts::FRAC_PI_2,
        arc_yaw: std::f32::consts::PI,
        arc_pitch: std::f32::consts::FRAC_PI_2,
        recharge_min: 0.5,
        recharge_max: 0.5,
        pulses: 1,
        ..HpDef::default()
    }
}

/// A unit of 5 population: `weak` dies to a few hits and cannot shoot, `gunner` reaches the whole map and cannot be killed.
fn defs() -> Arc<Catalog> {
    let unit = |name: &str, hull: f32, hps: Vec<HpDef>, range: f32, sensor: f32| ShipDef {
        name: name.into(),
        class: TargetClass::Frigate,
        is_static: false,
        hull,
        radius: 100.0,
        hardpoints: hps,
        population: 5,
        max_weapon_range: range,
        targeting_range: range,
        preferred_range: range,
        sensor_range: sensor,
        ..ShipDef::default()
    };
    Arc::new(Catalog::from_defs(vec![unit("weak", 300.0, vec![], 0.0, 1250.0), unit("gunner", 1.0e6, vec![gun(30_000.0)], 30_000.0, 30_000.0)], vec![laser()]))
}

/// Side 0 fields one `weak` unit and holds `reserve` more outside the battle; the cap (5) has room for one unit at a time. Side 1 is one `gunner`.
fn battle(reserve: u32, weak_ai: bool) -> Battle {
    let cat = defs();
    let side = |n: &str, ai: bool| SideSetup { name: n.into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 0 }], structures: vec![], ai };
    let mut config = BattleConfig { half_size: 6000.0, unit_cap: 5, ..BattleConfig::default() };
    config.retreat_allowed = vec![false, false];
    config.reserve = vec![vec![(cat.ship_id("weak").unwrap(), reserve)]];
    Battle::new(BattleSetup { sides: vec![side("weak", weak_ai), side("gunner", true)], catalog: cat, config })
}

fn dead_of_side(b: &Battle, side: u8) -> usize {
    b.ships.iter().filter(|s| s.side == side && !s.alive).count()
}

#[test]
fn an_ai_side_fights_on_until_its_reserve_is_used_up() {
    let mut b = battle(3, true);
    let (mut called, mut over_with_dead) = (0, None);
    for _ in 0..30 * 600 {
        for e in b.step() {
            if let Event::ReinforcementsCalled { side: 0, count, .. } = e {
                called += count;
            }
        }
        if b.is_over() {
            over_with_dead = Some(dead_of_side(&b, 0));
            break;
        }
    }
    assert_eq!(called, 3, "the AI calls every unit of its reserve in, one as each loss makes room");
    assert_eq!(over_with_dead, Some(4), "the battle is over when the unit on the field and the three of the reserve are destroyed");
    let r = b.result();
    assert_eq!(r.winner, Some(1));
    let reinforced: u32 = r.groups.iter().filter(|g| g.side == 0 && g.tag == REINFORCE_TAG).map(|g| g.total).sum();
    assert_eq!(reinforced, 3, "the result reports the called units");
    assert_eq!(b.reserve_left(0), 0);
}

#[test]
fn a_side_without_a_reserve_is_beaten_with_its_last_unit() {
    let mut b = battle(0, true);
    b.run_to_end(30 * 600);
    assert!(b.is_over());
    assert_eq!((b.result().winner, dead_of_side(&b, 0)), (Some(1), 1));
}

#[test]
fn a_player_whose_field_is_empty_can_still_call_the_reserve_in() {
    // Nobody commands side 0. Its unit on the field dies; the two in the reserve keep it in the fight.
    let mut b = battle(2, false);
    for _ in 0..30 * 120 {
        b.step();
    }
    assert_eq!(dead_of_side(&b, 0), 1);
    assert!(!b.is_over(), "the reserve has not entered and has not left: the battle goes on");
    assert_eq!(b.reserve_left(0), 2);
    // With nothing on the field the side sees nothing; the fog must not lock its reserve out (the no-spawn zones and the map bounds still apply).
    let weak = b.catalog().ship_id("weak").unwrap();
    let at = Vec3::new(1500.0, -2500.0, 0.0);
    assert_eq!(b.reinforcement_zone_check(0, at), Ok(()));
    b.command(0, Command::ReinforceAt { ship: weak, count: 1, pos: at, yaw_offset: 0.0 }).expect("called in");
    assert_eq!(b.reserve_left(0), 1, "a called unit leaves the reserve");
    for _ in 0..30 * 120 {
        b.step();
    }
    assert_eq!(dead_of_side(&b, 0), 2);
    assert!(!b.is_over(), "one unit still waits outside");
    b.command(0, Command::Reinforce { ship: weak, count: 1 }).expect("called in");
    b.run_to_end(30 * 600);
    assert!(b.is_over());
    assert_eq!((b.result().winner, dead_of_side(&b, 0), b.reserve_left(0)), (Some(1), 3, 0));
}

#[test]
fn a_retreat_takes_the_reserve_out_of_the_battle() {
    let mut b = battle(2, false);
    for _ in 0..30 * 120 {
        b.step();
    }
    assert!(!b.is_over());
    // The player gives the fight up with nothing on the field: the reserve leaves without having entered.
    b.command(0, Command::Retreat).expect("retreat ordered");
    b.run_to_end(30 * 600);
    assert!(b.is_over());
    let r = b.result();
    assert_eq!(r.winner, Some(1));
    assert!(!r.groups.iter().any(|g| g.side == 0 && g.tag == REINFORCE_TAG), "nothing was called in, so the galaxy keeps the two units");
}

#[test]
fn the_ai_calls_one_reserve_unit_every_15_s() {
    // `ReinforceSpace.lua`: one unit per call, then `Sleep(15)`; `AotR_Reinforce_Units` excludes itself, so calls do not overlap. The cap (20)
    // has room for all three units of the reserve from the start; neither side can shoot (`weak` has no weapon), so nothing dies.
    let cat = defs();
    let side = |n: &str| SideSetup { name: n.into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 0 }], structures: vec![], ai: true };
    let mut config = BattleConfig { half_size: 6000.0, unit_cap: 20, ..BattleConfig::default() };
    config.retreat_allowed = vec![false, false];
    config.reserve = vec![vec![(cat.ship_id("weak").unwrap(), 3)]];
    let mut b = Battle::new(BattleSetup { sides: vec![side("weak"), side("weak")], catalog: cat, config });
    let mut calls = Vec::new();
    while b.time() < 40.0 {
        let n = b.step().iter().filter(|e| matches!(e, Event::ReinforcementsCalled { side: 0, .. })).count();
        calls.extend(std::iter::repeat_n(b.time(), n));
    }
    println!("calls at {calls:?}");
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[0] < 2.0 && calls.windows(2).all(|w| (15.0..16.5).contains(&(w[1] - w[0]))), "one every 15 s: {calls:?}");
}
