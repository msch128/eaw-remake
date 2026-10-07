//! Rule tests with hand-made definitions (no game data needed).

use std::sync::Arc;

use battle_sim::*;

fn laser() -> ProjDef {
    ProjDef {
        name: "laser".into(),
        category: "laser".into(),
        damage: 100.0,
        speed: 2000.0,
        max_distance: 9000.0,
        shield_damage: true,
        hull_damage: true,
        ..ProjDef::default()
    }
}

fn gun(name: &str, health: f32) -> HpDef {
    HpDef {
        name: name.into(),
        kind: HpKind::Weapon,
        health,
        destroyable: health > 0.0,
        targetable: true,
        offset: Vec3::new(10.0, 0.0, 0.0),
        forward: Vec3::new(1.0, 0.0, 0.0),
        projectile: Some(0),
        range: 6000.0,
        half_yaw: std::f32::consts::PI,
        half_pitch: std::f32::consts::FRAC_PI_2,
        recharge_min: 0.5,
        recharge_max: 0.5,
        pulses: 1,
        ..HpDef::default()
    }
}

fn ship(name: &str, hull: f32, shield: f32, hps: Vec<HpDef>) -> ShipDef {
    ShipDef {
        name: name.into(),
        class: TargetClass::Frigate,
        is_static: true,
        hull,
        shield,
        shield_regen: 0.0,
        radius: 100.0,
        hardpoints: hps,
        population: 5,
        ..ShipDef::default()
    }
}

fn setup(catalog: Catalog, a: &str, b: &str, na: u32, nb: u32, config: BattleConfig) -> BattleSetup {
    let catalog = Arc::new(catalog);
    let side = |n: &str, c: u32| SideSetup {
        name: n.into(),
        units: vec![UnitSetup { ship: catalog.ship_id(n).unwrap(), count: c, tag: 7 }],
        structures: vec![],
        ai: true,
    };
    BattleSetup { sides: vec![side(a, na), side(b, nb)], catalog, config }
}

fn cfg() -> BattleConfig {
    BattleConfig { half_size: 6000.0, ..BattleConfig::default() }
}

#[test]
fn shields_absorb_before_hull() {
    let cat = Catalog::from_defs(vec![ship("gunner", 1000.0, 0.0, vec![gun("g", 0.0)]), ship("target", 500.0, 400.0, vec![])], vec![laser()]);
    let mut b = Battle::new(setup(cat, "gunner", "target", 1, 1, cfg()));
    let mut shield_hits = 0;
    let mut first_hull = None;
    for i in 0..2000 {
        for e in b.step() {
            if let Event::Hit { kind, ship, .. } = e
                && *ship == 1
            {
                match kind {
                    HitKind::Shield => shield_hits += 1,
                    _ if first_hull.is_none() => first_hull = Some(shield_hits),
                    _ => {}
                }
            }
        }
        if b.is_over() {
            assert!(i > 10);
            break;
        }
    }
    assert!(b.is_over());
    assert_eq!(b.result().winner, Some(0));
    // 400 shield / 100 damage: four hits are absorbed (the fifth overflows into the hull).
    assert!((3..=4).contains(&first_hull.unwrap()), "hull first hit after {first_hull:?} shield hits");
    assert!(b.ships[1].hull <= 0.0);
}

#[test]
fn destroyed_hardpoint_stops_firing() {
    // The target has a fragile weapon hardpoint and a lot of hull.
    let cat = Catalog::from_defs(
        vec![ship("attacker", 1.0e6, 0.0, vec![gun("g", 0.0), gun("g2", 0.0), gun("g3", 0.0)]), ship("target", 5000.0, 0.0, vec![gun("weak", 150.0)])],
        vec![laser()],
    );
    let mut b = Battle::new(setup(cat, "attacker", "target", 1, 1, cfg()));
    let mut destroyed_at = None;
    let mut fired_after = 0;
    for _ in 0..3000 {
        for e in b.step().to_vec() {
            match e {
                Event::HardpointDestroyed { ship: 1, hardpoint: 0, kind } => {
                    assert_eq!(kind, HpKind::Weapon);
                    destroyed_at = Some(b.tick());
                }
                Event::ProjectileFired { ship: 1, .. } if destroyed_at.is_some() => fired_after += 1,
                _ => {}
            }
        }
        if b.is_over() {
            break;
        }
    }
    assert!(destroyed_at.is_some(), "hardpoint destroyed before the hull");
    assert_eq!(fired_after, 0, "a destroyed weapon cannot fire");
}

#[test]
fn parallel_matches_sequential() {
    let make = |parallel: bool| {
        let cat = Catalog::from_defs(
            vec![ship("a", 3000.0, 800.0, vec![gun("g", 100.0), gun("g2", 100.0)]), ship("b", 3500.0, 600.0, vec![gun("g", 100.0)])],
            vec![laser()],
        );
        let mut c = cfg();
        c.parallel = parallel;
        c.seed = 99;
        let mut b = Battle::new(setup(cat, "a", "b", 20, 20, c));
        for _ in 0..600 {
            b.step();
        }
        b.state_hash()
    };
    assert_eq!(make(true), make(false));
    assert_eq!(make(true), make(true));
}

#[test]
fn retreat_needs_countdown_and_ships_leave() {
    let mut mobile = ship("runner", 1000.0, 0.0, vec![]);
    mobile.is_static = false;
    mobile.max_speed = 1500.0;
    mobile.accel = 3000.0;
    mobile.turn_rate = 6.0;
    let cat = Catalog::from_defs(vec![mobile, ship("idle", 1000.0, 0.0, vec![])], vec![laser()]);
    let mut c = cfg();
    c.retreat_countdown = 2.0;
    let mut b = Battle::new(setup(cat, "runner", "idle", 2, 1, c));
    assert_eq!(b.command(0, Command::Retreat), Err(CommandError::RetreatNotAllowed));
    for _ in 0..70 {
        b.step();
    }
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..900 {
        b.step();
        if b.is_over() {
            break;
        }
    }
    assert!(b.is_over());
    let r = b.result();
    assert_eq!(r.winner, Some(1));
    let g = r.groups.iter().find(|g| g.side == 0).unwrap();
    assert_eq!((g.total, g.survived, g.retreated), (2, 2, 2));
}

#[test]
fn reinforcements_respect_unit_cap() {
    let cat = Catalog::from_defs(vec![ship("a", 1.0e6, 0.0, vec![]), ship("b", 1.0e6, 0.0, vec![])], vec![laser()]);
    let mut c = cfg();
    c.unit_cap = 12;
    let mut b = Battle::new(setup(cat, "a", "b", 1, 1, c));
    let a = b.catalog().ship_id("a").unwrap();
    // 5 population in play, cap 12: one more unit fits, three do not.
    assert_eq!(b.command(0, Command::Reinforce { ship: a, count: 3 }), Err(CommandError::UnitCap));
    b.command(0, Command::Reinforce { ship: a, count: 1 }).unwrap();
    let mut arrived = false;
    for _ in 0..400 {
        arrived |= b.step().iter().any(|e| matches!(e, Event::ReinforcementsArrived { side: 0, .. }));
    }
    assert!(arrived);
    assert_eq!(b.ships.iter().filter(|s| s.side == 0).count(), 2);
}

#[test]
fn carriers_launch_and_replace_squadrons() {
    let mut fighter = ship("fighter", 20.0, 0.0, vec![]);
    fighter.class = TargetClass::Fighter;
    fighter.is_static = false;
    fighter.max_speed = 100.0;
    fighter.accel = 100.0;
    fighter.turn_rate = 2.0;
    fighter.radius = 10.0;
    let squad = ShipDef { name: "squad".into(), is_squadron: true, members: vec![(0, 2)], class: TargetClass::Fighter, ..ShipDef::default() };
    let mut carrier = ship("carrier", 1.0e6, 0.0, vec![HpDef { kind: HpKind::FighterBay, health: 100.0, ..HpDef::default() }]);
    carrier.carried = vec![(1, 1)];
    carrier.reserve = vec![(1, 1)];
    carrier.spawn_delay = 1.0;
    let gunner = ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0), gun("g2", 0.0), gun("g3", 0.0), gun("g4", 0.0)]);
    let cat = Catalog::from_defs(vec![fighter, squad, carrier, gunner], vec![laser()]);
    let mut b = Battle::new(setup(cat, "carrier", "gunner", 1, 1, cfg()));
    assert_eq!(b.ships.iter().filter(|s| s.def == 0).count(), 2, "starting squadron launched");
    let mut launches = 0;
    for _ in 0..1500 {
        launches += b.step().iter().filter(|e| matches!(e, Event::SquadronLaunched { .. })).count();
    }
    assert_eq!(launches, 1, "one replacement from the reserve");
    assert_eq!(b.ships.iter().filter(|s| s.def == 0).count(), 4);
}

