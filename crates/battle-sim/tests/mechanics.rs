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
        arc_yaw: std::f32::consts::PI,
        arc_pitch: std::f32::consts::FRAC_PI_2,
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
    // Reinforcements are reported in the results under REINFORCE_TAG.
    let r = b.result();
    let g = r.groups.iter().find(|g| g.side == 0 && g.tag == REINFORCE_TAG).expect("reinforcement group");
    assert_eq!((g.total, g.survived), (1, 1));
}

#[test]
fn losses_make_room_under_the_unit_cap() {
    // Two "weak" ships (5 population each) under a cap of 12: a third does not fit until one has died.
    let cat = Catalog::from_defs(vec![ship("weak", 50.0, 0.0, vec![]), ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0)])], vec![laser()]);
    let mut c = cfg();
    c.unit_cap = 12;
    let mut b = Battle::new(setup(cat, "weak", "gunner", 2, 1, c));
    let weak = b.catalog().ship_id("weak").unwrap();
    assert_eq!(b.pop_in_play(0), 10);
    assert_eq!(b.command(0, Command::Reinforce { ship: weak, count: 1 }), Err(CommandError::UnitCap));
    for _ in 0..3000 {
        b.step();
        if b.pop_in_play(0) <= 5 || b.is_over() {
            break;
        }
    }
    assert_eq!(b.pop_in_play(0), 5, "one weak ship died, one is left");
    let r = b.command(0, Command::Reinforce { ship: weak, count: 1 });
    assert!(r.is_ok(), "the dead ship's population is free again: {r:?}");
    assert_eq!(b.pop_in_play(0), 10, "the arrival on its way counts");
}

#[test]
fn reinforcements_can_be_called_to_a_chosen_position_inside_the_zone() {
    let mut seer = ship("a", 1.0e6, 0.0, vec![]);
    seer.sensor_range = 1250.0;
    let cat = Catalog::from_defs(vec![seer, ship("b", 1.0e6, 0.0, vec![])], vec![laser()]);
    let mut c = cfg();
    c.unit_cap = 100;
    let mut b = Battle::new(setup(cat, "a", "b", 1, 1, c));
    let a = b.catalog().ship_id("a").unwrap();
    let (own, foe) = (b.ships.iter().find(|s| s.side == 0).unwrap().pos, b.ships.iter().find(|s| s.side == 1).unwrap().pos);
    let toward_enemy = (foe - own).normalized();
    let beside = Vec3::new(own.x + 600.0, own.y + 300.0, own.z);
    // Seen by an own ship and outside the enemy structure's no-spawn zone: accepted.
    assert_eq!(b.reinforcement_zone_check(0, beside), Ok(()));
    // Outside every own sensor disc (fog of war).
    let far = own - toward_enemy * 3500.0;
    assert!(matches!(b.reinforcement_zone_check(0, far), Err(CommandError::NotVisible | CommandError::OutsideBattleArea)), "{:?}", b.reinforcement_zone_check(0, far));
    // Inside the no-spawn zone of the enemy structure (2000, the estimate: the data has none).
    let near_enemy = foe + Vec3::new(500.0, 0.0, 0.0);
    assert_eq!(b.reinforcement_zone_check(0, near_enemy), Err(CommandError::InNoSpawnZone));
    // Outside the map.
    assert_eq!(b.reinforcement_zone_check(0, Vec3::new(1.0e6, 0.0, 0.0)), Err(CommandError::OutsideBattleArea));
    assert_eq!(b.command(0, Command::ReinforceAt { ship: a, count: 1, pos: near_enemy }), Err(CommandError::InNoSpawnZone));
    // The accepted call arrives after the delay, at the chosen point (on free space), facing the enemy.
    b.command(0, Command::ReinforceAt { ship: a, count: 2, pos: beside }).unwrap();
    let mut arrived = false;
    for _ in 0..400 {
        arrived |= b.step().iter().any(|e| matches!(e, Event::ReinforcementsArrived { side: 0, .. }));
    }
    assert!(arrived);
    let new: Vec<_> = b.ships.iter().filter(|s| s.side == 0 && s.id > 1).collect();
    assert_eq!(new.len(), 2);
    for s in &new {
        assert!(s.pos.dist(beside) < 1200.0, "arrived near the chosen point: {:?}", s.pos);
        assert!(s.pos.dist(own) >= 150.0, "not on top of the existing ship");
    }
    assert!(new[0].pos.dist(new[1].pos) >= 150.0, "not on top of each other");
}

#[test]
fn fog_of_war_hides_ships_outside_sensor_range_until_seen() {
    let mut sensor = ship("seer", 1.0e6, 0.0, vec![]);
    sensor.is_static = false;
    sensor.sensor_range = 1500.0;
    let mut target = ship("far", 1.0e6, 0.0, vec![]);
    target.is_static = false;
    target.sensor_range = 1500.0;
    let cat = Catalog::from_defs(vec![sensor, target], vec![laser()]);
    let mut b = Battle::new(setup(cat, "seer", "far", 1, 1, cfg()));
    let (s0, s1) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    b.step();
    let apart = b.ships[s0 as usize].pos.dist(b.ships[s1 as usize].pos);
    assert!(apart > 1500.0, "the spawn points are out of sensor range: {apart}");
    assert!(b.is_visible_to(0, s0) && !b.is_visible_to(0, s1) && !b.is_visible_to(1, s0), "nobody sees the other side yet");
    // An ally ship moves into range: now side 0 sees the enemy (and the enemy sees it back).
    let ally_pos = b.ships[s1 as usize].pos + Vec3::new(1000.0, 0.0, 0.0);
    b.ships[s0 as usize].pos = ally_pos;
    for _ in 0..20 {
        b.step();
    }
    assert!(b.is_visible_to(0, s1) && b.is_visible_to(1, s0));
    // Out of sight again: it stays visible for `fog_linger` (15 s), then it is lost.
    b.ships[s0 as usize].pos = b.ships[s1 as usize].pos + Vec3::new(4000.0, 0.0, 0.0);
    for _ in 0..30 * 10 {
        b.step();
    }
    assert!(b.is_visible_to(0, s1), "still remembered after 10 s");
    for _ in 0..30 * 10 {
        b.step();
    }
    assert!(!b.is_visible_to(0, s1), "lost after the linger time");
}

#[test]
fn stealth_and_reveal_pings_change_what_is_seen() {
    let mut ghost = ship("ghost", 1.0e6, 0.0, vec![]);
    ghost.is_static = false;
    ghost.sensor_range = 4000.0;
    ghost.abilities.push(battle_sim::AbilityDef { kind: "STEALTH".into(), stealth: true, toggle: true, duration: f32::MAX / 4.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() });
    let mut watcher = ship("watcher", 1.0e6, 0.0, vec![]);
    watcher.is_static = false;
    watcher.sensor_range = 4000.0;
    watcher.abilities.push(battle_sim::AbilityDef { kind: "FOW_REVEAL_PING".into(), reveal_radius: 1000.0, duration: 5.0, recharge: 60.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() });
    let cat = Catalog::from_defs(vec![ghost, watcher], vec![laser()]);
    let mut su = setup(cat, "ghost", "watcher", 1, 1, cfg());
    su.sides[1].ai = false; // the AI would ping on its own; this test aims the ping by hand
    let mut b = Battle::new(su);
    let (g, w) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    // 3000 apart: inside the 4000 sensor range, outside the 500 stealth detection once stealth is on.
    let wp = b.ships[w as usize].pos;
    b.ships[g as usize].pos = wp + Vec3::new(3000.0, 0.0, 0.0);
    for _ in 0..20 {
        b.step();
    }
    assert!(b.is_visible_to(1, g), "visible without stealth");
    b.command(0, Command::UseAbility { ship: g, ability: 0 }).unwrap();
    for _ in 0..30 * 20 {
        b.step();
    }
    assert!(!b.is_visible_to(1, g), "hidden by stealth beyond the detection range");
    // A reveal ping on the spot shows it for the ping duration.
    let at = b.ships[g as usize].pos;
    b.command(1, Command::UseAbilityAt { ship: w, ability: 0, pos: at }).unwrap();
    for _ in 0..20 {
        b.step();
    }
    assert!(b.is_visible_to(1, g), "revealed by the ping");
    // Close enough, stealth no longer helps.
    b.ships[g as usize].pos = wp + Vec3::new(400.0, 0.0, 0.0);
    for _ in 0..30 * 20 {
        b.step();
    }
    assert!(b.is_visible_to(1, g), "inside the detection range");
}

#[test]
fn healing_abilities_repair_the_ship_or_friends_in_range() {
    let mut medic = ship("medic", 10000.0, 0.0, vec![]);
    medic.heals.push(battle_sim::HealDef { range: 0.0, amount: 4.0, percent: 0.01, interval: 2.0, single: false, classes: 0 });
    let mut tender = ship("tender", 1000.0, 0.0, vec![]);
    tender.heals.push(battle_sim::HealDef { range: 5000.0, amount: 100.0, percent: 0.0, interval: 1.0, single: true, classes: 0 });
    let cat = Catalog::from_defs(vec![medic, tender], vec![laser()]);
    let mut b = Battle::new(setup(cat, "medic", "tender", 1, 1, cfg()));
    let (m, t) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    b.ships[m as usize].hull = 5000.0;
    b.ships[t as usize].hull = 100.0;
    for _ in 0..30 * 10 {
        b.step();
    }
    // 10 s: 5 pulses of 4 + 1 % of 10000 = 104 each.
    let healed = b.ships[m as usize].hull - 5000.0;
    assert!((healed - 520.0).abs() < 110.0, "self repair about 520 hull in 10 s: {healed}");
    // The tender heals itself (range covers it) 100 per second, capped at its hull.
    assert!(b.ships[t as usize].hull > 900.0, "{}", b.ships[t as usize].hull);
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


fn mobile(name: &str, class: TargetClass, speed: f32, hull: f32, hps: Vec<HpDef>) -> ShipDef {
    ShipDef { class, is_static: false, max_speed: speed, accel: speed, turn_rate: 1.5, max_weapon_range: 500.0, targeting_range: 600.0, preferred_range: 300.0, ..ship(name, hull, 0.0, hps) }
}

fn one_side(cat: &Arc<Catalog>, name: &str, count: u32, ai: bool) -> SideSetup {
    SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(name).unwrap(), count, tag: 1 }], structures: vec![], ai }
}

fn activated(b: &mut Battle, steps: usize, ship: u32) -> bool {
    for _ in 0..steps {
        if b.step().iter().any(|e| matches!(e, Event::AbilityActivated { ship: s, .. } if *s == ship)) {
            return true;
        }
    }
    false
}

#[test]
fn ai_uses_speed_shield_and_damage_abilities_by_their_numbers() {
    let speed = AbilityDef { kind: "SPRINT".into(), speed_mult: 2.0, recharge: 20.0, duration: 6.0, ..AbilityDef::default() };
    let regen = AbilityDef { kind: "SHIELD_BOOST".into(), shield_regen_mult: 3.0, recharge: 20.0, duration: 6.0, ..AbilityDef::default() };
    let tough = AbilityDef { kind: "BRACE".into(), take_damage_mult: 0.5, recharge: 20.0, duration: 6.0, ..AbilityDef::default() };
    let mut runner = mobile("runner", TargetClass::Frigate, 100.0, 5000.0, vec![]);
    runner.abilities = vec![speed];
    let mut shielded = mobile("shielded", TargetClass::Frigate, 100.0, 50000.0, vec![]);
    shielded.shield = 3000.0;
    shielded.shield_regen = 10.0;
    shielded.abilities = vec![regen];
    let mut brace = mobile("brace", TargetClass::Frigate, 100.0, 4000.0, vec![]);
    brace.abilities = vec![tough];
    let mut gunner = mobile("gunner", TargetClass::Frigate, 100.0, 1.0e7, vec![gun("g", 0.0)]);
    gunner.is_static = true;
    let cat = Arc::new(Catalog::from_defs(vec![runner, shielded, brace, gunner], vec![laser()]));
    let config = BattleConfig { half_size: 6000.0, retreat_countdown: 0.0, ..BattleConfig::default() };

    // 1. Speed: a retreating ship sprints.
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![one_side(&cat, "runner", 1, true), one_side(&cat, "gunner", 1, true)], config: config.clone() });
    b.command(0, Command::Retreat).expect("retreat");
    assert!(activated(&mut b, 150, 0), "a fleeing ship uses its speed ability");

    // 2. Shield regeneration: when the shield is below half and under fire; 3. damage reduction: when the hull is low and under fire.
    for (name, hp_ok) in [("shielded", true), ("brace", true)] {
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![one_side(&cat, name, 1, true), one_side(&cat, "gunner", 1, true)], config: config.clone() });
        let mut used = false;
        let mut first_use_state = None;
        for _ in 0..30 * 40 {
            if b.step().iter().any(|e| matches!(e, Event::AbilityActivated { ship: 0, .. })) {
                used = true;
                first_use_state = Some((b.ships[0].hull, b.ships[0].shield));
                break;
            }
            if b.is_over() {
                break;
            }
        }
        assert!(used && hp_ok, "{name}: the ability should be used under fire");
        let (hull, shield) = first_use_state.unwrap();
        match name {
            "shielded" => assert!(shield < 1500.0, "regen ability only with a weak shield (shield {shield})"),
            _ => assert!(hull < 0.6 * 4000.0, "damage reduction only with a low hull (hull {hull})"),
        }
    }
}

#[test]
fn fleets_advance_line_abreast_and_stay_together() {
    let fast = mobile("fast", TargetClass::Frigate, 200.0, 5000.0, vec![]);
    let slow = mobile("slow", TargetClass::Frigate, 100.0, 5000.0, vec![]);
    let target = ship("dummy", 1.0e7, 0.0, vec![]);
    let cat = Arc::new(Catalog::from_defs(vec![fast, slow, target], vec![laser()]));
    let side0 = SideSetup { name: "a".into(), units: vec![UnitSetup { ship: 0, count: 3, tag: 1 }, UnitSetup { ship: 1, count: 3, tag: 1 }], structures: vec![], ai: true };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side0, one_side(&cat, "dummy", 1, true)], config: BattleConfig { half_size: 20000.0, record_events: false, ..BattleConfig::default() } });
    for _ in 0..30 * 25 {
        b.step();
    }
    let mine: Vec<_> = b.ships.iter().filter(|s| s.side == 0).collect();
    let (min_x, max_x) = mine.iter().fold((f32::MAX, f32::MIN), |a, s| (a.0.min(s.pos.x), a.1.max(s.pos.x)));
    let (min_y, max_y) = mine.iter().fold((f32::MAX, f32::MIN), |a, s| (a.0.min(s.pos.y), a.1.max(s.pos.y)));
    println!("after 25 s: depth spread {:.0}, lateral spread {:.0}", max_x - min_x, max_y - min_y);
    // Without cohesion the 200 u/s ships would be 2500 units ahead of the 100 u/s ones.
    assert!(max_x - min_x < 900.0, "ships keep together: depth spread {}", max_x - min_x);
    assert!(max_y - min_y > 300.0, "the line stays spread out: lateral spread {}", max_y - min_y);
}

#[test]
fn bombers_make_straight_bombing_runs_on_big_ships() {
    let mut bomber = mobile("bomber", TargetClass::Bomber, 400.0, 100.0, vec![]);
    bomber.radius = 10.0; // a real fighter; the hull keep-out of big ships scales with it
    let target = ship("capital", 1.0e7, 0.0, vec![]);
    let cat = Arc::new(Catalog::from_defs(vec![bomber, target], vec![laser()]));
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![one_side(&cat, "bomber", 1, true), one_side(&cat, "capital", 1, true)], config: BattleConfig { half_size: 12000.0, record_events: false, ..BattleConfig::default() } });
    let tpos = b.ships[1].pos;
    let (mut passes, mut max_after_pass, mut near) = (0, 0.0f32, false);
    for _ in 0..30 * 60 {
        b.step();
        let d = b.ships[0].pos.dist(tpos);
        if d < 120.0 {
            if !near {
                passes += 1;
            }
            near = true;
            max_after_pass = 0.0;
        } else if near {
            max_after_pass = max_after_pass.max(d);
            if d > 1000.0 {
                near = false;
            }
        }
    }
    println!("bomber passes in 60 s: {passes}");
    assert!(passes >= 2, "the bomber comes back for more passes: {passes}");
}

#[test]
fn bomber_targets_shield_generator_and_performs_evasive_vertical_climb() {
    let mut bomber = mobile("bomber", TargetClass::Bomber, 400.0, 100.0, vec![]);
    bomber.radius = 10.0;
    let mut target = ship("capital", 1.0e7, 0.0, vec![]);
    target.class = TargetClass::Capital;
    // Hardpoint 0: Weapon, Hardpoint 1: ShieldGenerator
    target.hardpoints = vec![
        HpDef { name: "turbolaser".into(), kind: HpKind::Weapon, health: 500.0, offset: Vec3::new(100.0, 50.0, 0.0), ..HpDef::default() },
        HpDef { name: "shield_gen".into(), kind: HpKind::ShieldGenerator, health: 500.0, offset: Vec3::new(-100.0, 0.0, 50.0), ..HpDef::default() },
    ];
    let cat = Arc::new(Catalog::from_defs(vec![bomber, target], vec![laser()]));
    let mut b = Battle::new(BattleSetup {
        catalog: cat.clone(),
        sides: vec![one_side(&cat, "bomber", 1, true), one_side(&cat, "capital", 1, true)],
        config: BattleConfig { half_size: 12000.0, record_events: false, ..BattleConfig::default() },
    });
    b.ships[0].pos = Vec3::new(1000.0, 0.0, 0.0);
    let mut saw_vertical_evasion = false;
    let mut saw_shield_gen_targeted = false;
    for _ in 0..30 * 20 {
        b.step();
        let s = &b.ships[0];
        if s.target_hp == 1 {
            saw_shield_gen_targeted = true;
        }
        if s.pos.z.abs() > 30.0 {
            saw_vertical_evasion = true;
        }
    }
    assert!(saw_shield_gen_targeted, "bomber automatically targeted ShieldGenerator");
    assert!(saw_vertical_evasion, "bomber climbed/dived during egress phase");
}

fn duel_loss(attacker: ShipDef, target: ShipDef, seconds: u32) -> (f32, f32) {
    let (a, t) = (attacker.name.clone(), target.name.clone());
    let cat = Catalog::from_defs(vec![attacker, target], vec![laser()]);
    let mut b = Battle::new(setup(cat, &a, &t, 1, 1, cfg()));
    let (ai, ti) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    let (ah, th) = (b.ships[ai as usize].hull, b.ships[ti as usize].hull);
    for _ in 0..30 * seconds {
        b.step();
    }
    (ah - b.ships[ai as usize].hull, th - b.ships[ti as usize].hull)
}

#[test]
fn combat_bonus_auras_raise_damage_and_cut_damage_taken() {
    let target = ship("dummy", 1.0e6, 0.0, vec![]);
    let plain = ship("plain", 1.0e6, 0.0, vec![gun("g", 0.0)]);
    let (_, base) = duel_loss(plain.clone(), target.clone(), 10);
    assert!(base > 500.0, "{base}");
    let mut boosted = plain.clone();
    boosted.name = "boosted".into();
    boosted.bonuses.push(BonusDef { damage: 0.5, classes: 0, stacking: 3, ..BonusDef::default() });
    let (_, more) = duel_loss(boosted, target.clone(), 10);
    // The aura starts after the first second: about +50 % over the rest.
    assert!(more > base * 1.35 && more < base * 1.6, "base {base} boosted {more}");
    // A defence bonus on the target halves what it takes; two bonuses of one stacking category do not add up.
    let mut tough = target.clone();
    tough.name = "tough".into();
    tough.bonuses.push(BonusDef { defense: 1.0, classes: 0, stacking: 3, ..BonusDef::default() });
    tough.bonuses.push(BonusDef { defense: 1.0, classes: 0, stacking: 3, ..BonusDef::default() });
    let (_, less) = duel_loss(plain, tough, 10);
    assert!(less < base * 0.6 && less > base * 0.4, "base {base} tough {less}");
}

#[test]
fn redirect_blaster_blocks_or_reflects_shots() {
    let gunner = ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0)]);
    let plain = ship("plain", 1.0e6, 0.0, vec![]);
    let (_, base) = duel_loss(gunner.clone(), plain.clone(), 10);
    assert!(base > 500.0);
    let mut blocker = plain.clone();
    blocker.name = "blocker".into();
    blocker.redirects.push(RedirectDef { block: 1.0, redirect: 0.0, half_arc: std::f32::consts::PI, classes: 0 });
    let (_, blocked) = duel_loss(gunner.clone(), blocker, 10);
    assert_eq!(blocked, 0.0, "everything blocked");
    let mut mirror = plain.clone();
    mirror.name = "mirror".into();
    mirror.redirects.push(RedirectDef { block: 0.0, redirect: 1.0, half_arc: std::f32::consts::PI, classes: 0 });
    let (hurt_shooter, reflected) = duel_loss(gunner.clone(), mirror, 10);
    assert_eq!(reflected, 0.0);
    assert!(hurt_shooter > base * 0.9, "the shooter takes its own shots: {hurt_shooter} vs {base}");
    // A 15 % block chance blocks roughly 15 %.
    let plain_again = plain.clone();
    let mut partial = plain;
    partial.name = "partial".into();
    partial.redirects.push(RedirectDef { block: 0.15, redirect: 0.0, half_arc: std::f32::consts::PI, classes: 0 });
    let (_, full) = duel_loss(gunner.clone(), plain_again, 20);
    let (_, some) = duel_loss(gunner, partial, 20);
    assert!(some > full * 0.7 && some < full * 0.95, "unblocked {full} partial {some}");
}

#[test]
fn rocket_attack_speeds_up_missile_reloads_only() {
    let mut missile = laser();
    missile.name = "missile".into();
    missile.category = "missile".into();
    let target = ship("dummy", 1.0e7, 0.0, vec![]);
    let run = |category_missile: bool, use_ability: bool| {
        let mut boat = ship("boat", 1.0e6, 0.0, vec![gun("g", 0.0)]);
        boat.abilities.push(AbilityDef { kind: "ROCKET_ATTACK".into(), rocket: true, duration: 10.0, recharge: 30.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() });
        let proj = if category_missile { missile.clone() } else { laser() };
        let cat = Catalog::from_defs(vec![boat, target.clone()], vec![proj]);
        let mut b = Battle::new(setup(cat, "boat", "dummy", 1, 1, cfg()));
        let id = b.ships.iter().find(|s| s.side == 0).unwrap().id;
        if use_ability {
            b.command(0, Command::UseAbility { ship: id, ability: 0 }).unwrap();
        }
        let mut shots = 0;
        for _ in 0..30 * 8 {
            shots += b.step().iter().filter(|e| matches!(e, Event::ProjectileFired { .. })).count();
        }
        shots
    };
    let (plain, rockets, lasers) = (run(true, false), run(true, true), run(false, true));
    assert!(rockets as f32 > plain as f32 * 1.8, "missiles reload faster: {plain} -> {rockets}");
    assert!((lasers as f32 - plain as f32).abs() <= 2.0, "lasers are unaffected: {plain} vs {lasers}");
}

#[test]
fn sensor_jamming_breaks_missile_locks() {
    let mut homing = laser();
    homing.name = "homer".into();
    homing.category = "missile".into();
    homing.turn_rate = 6.0;
    homing.speed = 500.0;
    let run = |jam: bool| {
        let mut target = ship("target", 1.0e7, 0.0, vec![]);
        target.abilities.push(AbilityDef { kind: "SENSOR_JAMMING".into(), jam: true, duration: 15.0, recharge: 60.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() });
        let shooter = ship("shooter", 1.0e6, 0.0, vec![]);
        let cat = Catalog::from_defs(vec![shooter, target], vec![homing.clone()]);
        let mut b = Battle::new(setup(cat, "shooter", "target", 1, 1, cfg()));
        let (s, t) = (b.ships.iter().find(|x| x.side == 0).unwrap().id, b.ships.iter().find(|x| x.side == 1).unwrap().id);
        if jam {
            b.command(1, Command::UseAbility { ship: t, ability: 0 }).unwrap();
        }
        let tp = b.ships[t as usize].pos;
        // A missile launched at right angles to the target, 1500 units off its line.
        b.projectiles.push(Projectile { pos: tp + Vec3::new(0.0, 1500.0, 0.0), vel: Vec3::new(500.0, 0.0, 0.0), def: 0, side: 0, owner: s, target: t, aim_hp: u16::MAX, range_left: 9000.0 });
        for _ in 0..30 * 6 {
            b.step();
        }
        b.ships[t as usize].shield + b.ships[t as usize].hull
    };
    let hp = 1.0e7f32;
    assert!(run(false) < hp, "an unjammed missile curves in and hits");
    assert_eq!(run(true), hp, "a jammed missile flies on and misses");
}

#[test]
fn homing_missile_launches_with_curved_arc_loft_and_tracks_target() {
    let mut missile = laser();
    missile.name = "concussion_missile".into();
    missile.category = "missile".into();
    missile.turn_rate = 2.0;
    missile.speed = 400.0;
    let shooter = ship("missile_ship", 1000.0, 0.0, vec![gun("missile_tube", 500.0)]);
    let target = ship("target_ship", 1000.0, 0.0, vec![]);
    let cat = Catalog::from_defs(vec![shooter, target], vec![missile]);
    let mut b = Battle::new(setup(cat, "missile_ship", "target_ship", 1, 1, cfg()));
    let mut launched_z = None;
    for _ in 0..30 {
        b.step();
        if let Some(p) = b.projectiles.first() {
            launched_z = Some(p.vel.z);
            break;
        }
    }
    let z = launched_z.expect("a missile was launched");
    assert!(z > 50.0, "missile launches with an upward loft for curved trajectory: {z}");
}

#[test]
fn buzz_droid_storm_damages_enemies_in_the_area_over_time() {
    let mut caster = ship("caster", 1.0e6, 0.0, vec![]);
    caster.abilities.push(AbilityDef { kind: "BUZZ_DROIDS".into(), zone: Some(ZoneDef { radius: 500.0, enemy_dps: 100.0, max_range: 50000.0 }), duration: 10.0, recharge: 75.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() });
    let victim = ship("victim", 10000.0, 0.0, vec![]);
    let cat = Catalog::from_defs(vec![caster, victim], vec![laser()]);
    let mut b = Battle::new(setup(cat, "caster", "victim", 1, 1, cfg()));
    let (c, v) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    let at = b.ships[v as usize].pos;
    b.command(0, Command::UseAbilityAt { ship: c, ability: 0, pos: at }).unwrap();
    for _ in 0..30 * 5 {
        b.step();
    }
    let lost = 10000.0 - b.ships[v as usize].hull;
    assert!((lost - 500.0).abs() < 60.0, "5 s of 100 dps: {lost}");
    for _ in 0..30 * 20 {
        b.step();
    }
    let total = 10000.0 - b.ships[v as usize].hull;
    assert!((total - 1000.0).abs() < 120.0, "the storm ends after 10 s: {total}");
    assert_eq!(b.ships[c as usize].hull, 1.0e6, "the caster is not hurt");
}

#[test]
fn super_laser_destroys_the_chosen_ship_after_charging() {
    let mut station = ship("deathstar", 1.0e9, 0.0, vec![]);
    station.abilities.push(AbilityDef { kind: "SUPER_LASER".into(), super_laser: Some(SuperLaserDef { delay: 1.5, classes: TargetClass::Frigate.bit() }), duration: 25.0, recharge: 45.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() });
    let victim = ship("victim", 1.0e6, 1.0e5, vec![]);
    let cat = Catalog::from_defs(vec![station, victim], vec![laser()]);
    let mut b = Battle::new(setup(cat, "deathstar", "victim", 1, 1, cfg()));
    let (c, v) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    // Aiming at empty space fails; aiming at the ship works and takes the charge time.
    assert!(b.command(0, Command::UseAbilityAt { ship: c, ability: 0, pos: Vec3::new(0.0, 1.0e6, 0.0) }).is_err());
    let at = b.ships[v as usize].pos;
    b.command(0, Command::UseAbilityAt { ship: c, ability: 0, pos: at }).unwrap();
    for _ in 0..30 {
        b.step();
    }
    assert!(b.ships[v as usize].alive, "still charging after 1 s");
    for _ in 0..30 {
        b.step();
    }
    assert!(!b.ships[v as usize].alive, "destroyed after the charge time");
}

fn ability(kind: &str) -> AbilityDef {
    AbilityDef { kind: kind.into(), duration: 10.0, recharge: 60.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() }
}

/// True once ability 0 of the first ship of `side` was used (it is active or recharging).
fn used(b: &Battle, side: u8) -> bool {
    let s = b.ships.iter().find(|s| s.side == side).unwrap();
    s.abilities[0].active > 0.0 || s.abilities[0].cooldown > 0.0
}

#[test]
fn the_ai_uses_each_new_ability_in_a_suitable_scenario() {
    let dummy = || ship("dummy", 1.0e7, 0.0, vec![]);
    // Rocket attack: a missile boat with a frigate-sized enemy in range.
    let mut missile = laser();
    missile.name = "missile".into();
    missile.category = "missile".into();
    let mut boat = ship("boat", 1.0e6, 0.0, vec![gun("g", 0.0)]);
    boat.max_weapon_range = 6000.0;
    boat.abilities.push(AbilityDef { rocket: true, ..ability("ROCKET_ATTACK") });
    let mut b = Battle::new(setup(Catalog::from_defs(vec![boat, dummy()], vec![missile.clone()]), "boat", "dummy", 1, 1, cfg()));
    for _ in 0..30 * 4 {
        b.step();
    }
    assert!(used(&b, 0), "rocket attack");

    // Sensor jamming: a missile homes in on the ship.
    let mut homing = missile.clone();
    homing.turn_rate = 3.0;
    let mut target = ship("target", 1.0e6, 0.0, vec![]);
    target.abilities.push(AbilityDef { jam: true, ..ability("SENSOR_JAMMING") });
    let mut b = Battle::new(setup(Catalog::from_defs(vec![dummy(), target], vec![homing]), "dummy", "target", 1, 1, cfg()));
    let (s, t) = (b.ships.iter().find(|x| x.side == 0).unwrap().id, b.ships.iter().find(|x| x.side == 1).unwrap().id);
    let tp = b.ships[t as usize].pos;
    b.projectiles.push(Projectile { pos: tp + Vec3::new(0.0, 1500.0, 0.0), vel: Vec3::new(10.0, 0.0, 0.0), def: 0, side: 0, owner: s, target: t, aim_hp: u16::MAX, range_left: 9000.0 });
    for _ in 0..30 {
        b.step();
    }
    assert!(used(&b, 1), "sensor jamming");

    // Storm on a cluster of three, super laser on a frigate, reveal ping into the fog.
    let mut caster = ship("caster", 1.0e6, 0.0, vec![]);
    caster.abilities.push(AbilityDef { zone: Some(ZoneDef { radius: 5000.0, enemy_dps: 100.0, max_range: 50000.0 }), ..ability("BUZZ_DROIDS") });
    let mut b = Battle::new(setup(Catalog::from_defs(vec![caster, dummy()], vec![laser()]), "caster", "dummy", 1, 3, cfg()));
    for _ in 0..30 * 2 {
        b.step();
    }
    assert!(used(&b, 0), "storm on a cluster");

    let mut dstar = ship("dstar", 1.0e6, 0.0, vec![]);
    dstar.abilities.push(AbilityDef { super_laser: Some(SuperLaserDef { delay: 1.0, classes: TargetClass::Frigate.bit() }), ..ability("SUPER_LASER") });
    let mut b = Battle::new(setup(Catalog::from_defs(vec![dstar, dummy()], vec![laser()]), "dstar", "dummy", 1, 1, cfg()));
    for _ in 0..30 * 3 {
        b.step();
    }
    assert!(used(&b, 0), "super laser");
    assert!(b.ships.iter().filter(|s| s.side == 1).all(|s| !s.alive), "and it hit");

    let mut scout = ship("scout", 1.0e6, 0.0, vec![]);
    scout.abilities.push(AbilityDef { reveal_radius: 1500.0, ..ability("FOW_REVEAL_PING") });
    let fog = BattleConfig { fog_of_war: true, ..cfg() };
    let mut b = Battle::new(setup(Catalog::from_defs(vec![scout, dummy()], vec![laser()]), "scout", "dummy", 1, 1, fog));
    for _ in 0..30 * 3 {
        b.step();
    }
    assert!(used(&b, 0), "reveal ping into the fog");
}

#[test]
fn projectile_combat_mods_weaken_the_ships_hit_for_a_while() {
    let mut debuff = laser();
    debuff.name = "debuff".into();
    debuff.damage = 1.0;
    debuff.combat_mod = Some(CombatMod { kind: ModKind::Defense, value: -0.5, duration: 5.0, classes: 0 });
    let run = |hit_first: bool| {
        let cat = Catalog::from_defs(vec![ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0)]), ship("target", 1.0e6, 0.0, vec![])], vec![laser(), debuff.clone()]);
        let mut b = Battle::new(setup(cat, "gunner", "target", 1, 1, cfg()));
        let (g, t) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
        if hit_first {
            let tp = b.ships[t as usize].pos;
            b.projectiles.push(Projectile { pos: tp - Vec3::new(30.0, 0.0, 0.0), vel: Vec3::new(500.0, 0.0, 0.0), def: 1, side: 0, owner: g, target: NONE, aim_hp: u16::MAX, range_left: 9000.0 });
        }
        let loss = |b: &Battle| 1.0e6 - b.ships[t as usize].hull;
        // Window one: the debuff is on. Window two: it has expired.
        for _ in 0..30 * 5 {
            b.step();
        }
        let first = loss(&b);
        for _ in 0..30 * 5 {
            b.step();
        }
        let second = loss(&b) - first;
        (first, second)
    };
    let (plain1, plain2) = run(false);
    let (weak1, weak2) = run(true);
    assert!(weak1 > plain1 * 1.5, "defense -50% doubles the damage taken: {plain1} -> {weak1}");
    assert!((weak2 - plain2).abs() < plain2 * 0.35, "the debuff wore off: {plain2} vs {weak2}");
}

#[test]
fn autocast_lets_a_player_ship_use_an_ability_like_the_ai() {
    let mut missile = laser();
    missile.name = "missile".into();
    missile.category = "missile".into();
    let mut boat = ship("boat", 1.0e6, 0.0, vec![gun("g", 0.0)]);
    boat.max_weapon_range = 6000.0;
    boat.abilities.push(AbilityDef { rocket: true, supports_autofire: true, ..ability("ROCKET_ATTACK") });
    boat.abilities.push(AbilityDef { supports_autofire: false, ..ability("TURBO") });
    let run = |autocast: Option<bool>| {
        let cat = Catalog::from_defs(vec![boat.clone(), ship("dummy", 1.0e7, 0.0, vec![])], vec![missile.clone()]);
        let mut su = setup(cat, "boat", "dummy", 1, 1, cfg());
        su.sides[0].ai = false; // a player side
        let mut b = Battle::new(su);
        let id = b.ships.iter().find(|s| s.side == 0).unwrap().id;
        if let Some(on) = autocast {
            b.command(0, Command::SetAutocast { ship: id, ability: 0, on }).unwrap();
        }
        for _ in 0..30 * 4 {
            b.step();
        }
        let s = &b.ships[id as usize];
        (s.abilities[0].active > 0.0 || s.abilities[0].cooldown > 0.0, s.abilities[0].autocast)
    };
    assert_eq!(run(None), (false, false), "a player ship does not use abilities on its own");
    assert_eq!(run(Some(true)), (true, true), "with autocast on it fires the ability like the AI");
    assert_eq!(run(Some(false)), (false, false));
    // An ability that does not support automatic use refuses the command.
    let cat = Catalog::from_defs(vec![boat.clone(), ship("dummy", 1.0e7, 0.0, vec![])], vec![missile]);
    let mut b = Battle::new(setup(cat, "boat", "dummy", 1, 1, cfg()));
    let id = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    assert!(b.command(0, Command::SetAutocast { ship: id, ability: 1, on: true }).is_err());
}

fn hangar_defs() -> (ShipDef, ShipDef) {
    let mut fighter = ship("fighter", 20.0, 0.0, vec![]);
    fighter.class = TargetClass::Fighter;
    fighter.is_static = false;
    fighter.max_speed = 100.0;
    fighter.accel = 100.0;
    fighter.turn_rate = 2.0;
    fighter.radius = 10.0;
    let squad = ShipDef { name: "squad".into(), is_squadron: true, members: vec![(0, 2)], class: TargetClass::Fighter, ..ShipDef::default() };
    (fighter, squad)
}

fn bay(x: f32) -> HpDef {
    HpDef { kind: HpKind::FighterBay, health: 100.0, offset: Vec3::new(x, 0.0, 0.0), ..HpDef::default() }
}

#[test]
fn carriers_launch_what_their_owners_tech_level_allows() {
    let (fighter, squad) = hangar_defs();
    let mut carrier = ship("carrier", 1.0e6, 0.0, vec![bay(0.0)]);
    // Tech 0: one squadron and one in reserve; tech 2: three and two (the data has `..._Tech_N` entries per level).
    carrier.spawned_by_tech = vec![(0, vec![(1, 1)], vec![(1, 1)]), (2, vec![(1, 3)], vec![(1, 2)])];
    let dummy = ship("dummy", 1.0e6, 0.0, vec![]);
    let launched = |tech: Option<u8>| {
        let cat = Catalog::from_defs(vec![fighter.clone(), squad.clone(), carrier.clone(), dummy.clone()], vec![laser()]);
        let config = BattleConfig { staged_hangar_launch: false, side_tech: tech.map(|t| vec![t, 0]).unwrap_or_default(), ..cfg() };
        let b = Battle::new(setup(cat, "carrier", "dummy", 1, 1, config));
        let c = b.ships.iter().find(|s| s.side == 0).unwrap();
        (b.ships.iter().filter(|s| s.def == 0).count() / 2, c.reserve.iter().map(|r| r.1).sum::<u32>())
    };
    assert_eq!(launched(None), (1, 1), "no tech given = level 0");
    assert_eq!(launched(Some(1)), (1, 1), "the highest level at or below the owner's");
    assert_eq!(launched(Some(2)), (3, 2));
    assert_eq!(launched(Some(4)), (3, 2));
}

#[test]
fn squadrons_launch_sequentially_from_hangar_over_time() {
    let (fighter, squad) = hangar_defs();
    let mut carrier = ship("carrier", 1.0e6, 0.0, vec![bay(0.0)]);
    carrier.carried = vec![(1, 3)];
    carrier.spawn_delay = 1.0;
    let dummy = ship("dummy", 1.0e6, 0.0, vec![]);
    let cat = Catalog::from_defs(vec![fighter, squad, carrier, dummy], vec![laser()]);
    let mut b = Battle::new(setup(cat, "carrier", "dummy", 1, 1, cfg()));
    // Initially at t=0, only the first squadron (2 fighters) is launched; the other 2 are staged in hangar
    assert_eq!(b.ships.iter().filter(|s| s.def == 0).count() / 2, 1, "first squadron launched at start");
    // After 1 second (30 steps), the second squadron launches from the hangar
    for _ in 0..35 {
        b.step();
    }
    assert_eq!(b.ships.iter().filter(|s| s.def == 0).count() / 2, 2, "second squadron launched after delay");
    // After another second, the third squadron launches
    for _ in 0..35 {
        b.step();
    }
    assert_eq!(b.ships.iter().filter(|s| s.def == 0).count() / 2, 3, "third squadron launched after delay");
}

#[test]
fn squadrons_leave_through_a_working_hangar_and_nothing_launches_without_one() {
    let (fighter, squad) = hangar_defs();
    let mut carrier = ship("carrier", 1.0e6, 0.0, vec![bay(-300.0), bay(300.0)]);
    carrier.carried = vec![(1, 1)];
    carrier.reserve = vec![(1, 3)];
    carrier.spawn_delay = 1.0;
    let dummy = ship("dummy", 1.0e6, 0.0, vec![]);
    let cat = Catalog::from_defs(vec![fighter, squad, carrier, dummy], vec![laser()]);
    let mut b = Battle::new(setup(cat, "carrier", "dummy", 1, 1, cfg()));
    let c = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let kill_squadron = |b: &mut Battle| {
        for s in b.ships.iter_mut().filter(|s| s.def == 0 && s.side == 0) {
            s.alive = false;
        }
    };
    // First bay destroyed: the replacement leaves from the second one (the carrier faces +X, the bays are at its local +-X).
    b.ships[c as usize].hps[0].alive = false;
    kill_squadron(&mut b);
    for _ in 0..30 * 3 {
        b.step();
    }
    let cpos = b.ships[c as usize].pos;
    let yaw = b.ships[c as usize].yaw;
    let want = cpos + Vec3::new(300.0, 0.0, 0.0).rotate_yaw(yaw);
    let fresh: Vec<&Ship> = b.ships.iter().filter(|s| s.def == 0 && s.alive).collect();
    assert_eq!(fresh.len(), 2, "the squadron was replaced");
    assert!(fresh.iter().all(|s| s.pos.dist(want) < 320.0), "launched at the working bay: {:?} vs {want:?}", fresh.iter().map(|s| s.pos).collect::<Vec<_>>());
    // The reserve is visible and shrinks by one.
    assert_eq!(b.ships[c as usize].reserve.iter().map(|r| r.1).sum::<u32>(), 2);
    // Every hangar destroyed: no more launches, the reserve stays.
    b.ships[c as usize].hps[1].alive = false;
    kill_squadron(&mut b);
    for _ in 0..30 * 10 {
        b.step();
    }
    assert_eq!(b.ships.iter().filter(|s| s.def == 0 && s.alive).count(), 0, "no hangar, no launch");
    assert_eq!(b.ships[c as usize].reserve.iter().map(|r| r.1).sum::<u32>(), 2);
}

#[test]
fn corrupt_systems_drains_the_shield_of_the_chosen_ship() {
    let mut caster = ship("caster", 1.0e6, 0.0, vec![]);
    caster.abilities.push(AbilityDef { corrupt: Some(CorruptDef { max_range: 50000.0, duration: 10.0, drain: 100.0, pick_radius: 200.0 }), duration: 10.0, recharge: 120.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, kind: "CORRUPT_SYSTEMS".into(), ..Default::default() });
    let victim = ship("victim", 1.0e6, 5000.0, vec![]);
    let cat = Catalog::from_defs(vec![caster, victim], vec![laser()]);
    let mut b = Battle::new(setup(cat, "caster", "victim", 1, 1, cfg()));
    let (c, v) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    assert!(b.command(0, Command::UseAbilityAt { ship: c, ability: 0, pos: Vec3::new(0.0, 1.0e6, 0.0) }).is_err(), "no ship there");
    let at = b.ships[v as usize].pos;
    b.command(0, Command::UseAbilityAt { ship: c, ability: 0, pos: at }).unwrap();
    for _ in 0..30 * 5 {
        b.step();
    }
    let s = b.ships[v as usize].shield;
    assert!((5000.0 - s - 500.0).abs() < 80.0, "100 per second for 5 s: {s}");
    for _ in 0..30 * 30 {
        b.step();
    }
    assert!(b.ships[v as usize].shield > 3500.0, "the drain ended after 10 s and the shield starts to come back: {}", b.ships[v as usize].shield);
}

// ---- Retreat: turn to the spawn side, jump timer, drives, spawned units ----

fn engine() -> HpDef {
    HpDef { name: "engine".into(), kind: HpKind::Engine, health: 100.0, destroyable: true, targetable: true, ..HpDef::default() }
}

fn runner(name: &str, hps: Vec<HpDef>) -> ShipDef {
    ShipDef { max_speed: 400.0, accel: 4000.0, turn_rate: 3.0, radius: 50.0, ..mobile(name, TargetClass::Frigate, 400.0, 5000.0, hps) }
}

fn retreat_battle(defs: Vec<ShipDef>, side0: &str, n: u32, jump: f32) -> Battle {
    let cat = Arc::new(Catalog::from_defs(defs, vec![laser()]));
    let mut c = cfg();
    c.retreat_countdown = 0.0;
    c.retreat_jump_seconds = jump;
    let sides = vec![one_side(&cat, side0, n, false), one_side(&cat, "idle", 1, true)];
    Battle::new(BattleSetup { sides, catalog: cat, config: c })
}

fn idle_target() -> ShipDef {
    ship("idle", 1.0e6, 0.0, vec![])
}

#[test]
fn retreating_ships_turn_towards_their_spawn_side_and_head_there() {
    let mut b = retreat_battle(vec![runner("runner", vec![]), idle_target()], "runner", 1, 30.0);
    let home = b.ships[0].pos;
    // Fly away first, towards the enemy, so that "home" is behind the ship.
    let enemy = b.ships.iter().find(|s| s.side == 1).unwrap().pos;
    b.command(0, Command::Move { ships: vec![0], to: enemy }).unwrap();
    for _ in 0..150 {
        b.step();
    }
    let away = b.ships[0].pos.dist(home);
    assert!(away > 1000.0, "the ship left its spawn: {away}");
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..90 {
        b.step();
    }
    let s = &b.ships[0];
    let to_home = (home - s.pos).normalized();
    let heading = Vec3::from_yaw(s.yaw);
    assert!(heading.x * to_home.x + heading.y * to_home.y > 0.97, "the ship faces its spawn side after the turn");
    assert!(s.pos.dist(home) < away - 300.0, "and flies back to it: {} -> {}", away, s.pos.dist(home));
}

#[test]
fn the_jump_happens_when_the_timer_runs_out_and_not_before() {
    let mut b = retreat_battle(vec![runner("runner", vec![]), idle_target()], "runner", 2, 10.0);
    assert_eq!(b.retreat_jump_in(0), None);
    b.command(0, Command::Retreat).unwrap();
    assert_eq!(b.command(0, Command::Retreat), Ok(()), "a second order does not restart the timer");
    assert!((b.retreat_jump_in(0).unwrap() - 10.0).abs() < 0.1);
    for _ in 0..(9.5 * 30.0) as usize {
        b.step();
    }
    assert!(b.ships.iter().filter(|s| s.side == 0).all(|s| s.alive && !s.retreated), "still on the battlefield before the timer ends");
    assert!(b.retreat_jump_in(0).unwrap() < 1.0);
    for _ in 0..30 {
        b.step();
    }
    assert!(b.ships.iter().filter(|s| s.side == 0).all(|s| s.alive && s.retreated), "all escaped at the jump");
    assert_eq!(b.retreat_jump_in(0), None);
    assert!(b.is_over());
    let g = b.result().groups.into_iter().find(|g| g.side == 0).unwrap();
    assert_eq!((g.total, g.survived, g.retreated), (2, 2, 2));
}

#[test]
fn a_ship_without_a_living_engine_is_lost_while_one_without_engine_hardpoints_escapes() {
    let mut b = retreat_battle(vec![runner("runner", vec![engine(), engine()]), runner("plain", vec![]), idle_target()], "runner", 3, 3.0);
    assert!(b.can_retreat(0), "ship with intact engines can retreat");
    b.ships[0].hps[0].alive = false; // one of two engines left: still jumps
    assert!(b.can_retreat(0), "ship with 1 of 2 engines intact can retreat");
    b.ships[1].hps[0].alive = false;
    b.ships[1].hps[1].alive = false; // no engine left
    assert!(!b.can_retreat(1), "ship with all engines destroyed cannot retreat");
    assert!(b.can_retreat(2), "ship with all engines intact can retreat");
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..(3.2 * 30.0) as usize {
        b.step();
    }
    assert!(b.ships[0].retreated && b.ships[0].alive, "one living engine is enough");
    assert!(!b.ships[1].retreated && !b.ships[1].alive, "no living engine: the ship is lost");
    assert!(b.ships[2].retreated && b.ships[2].alive);
    let mut p = retreat_battle(vec![runner("plain", vec![]), idle_target()], "plain", 1, 1.0);
    assert!(p.can_retreat(0), "ship without hardpoints can retreat");
    p.command(0, Command::Retreat).unwrap();
    for _ in 0..40 {
        p.step();
    }
    assert!(p.ships[0].retreated, "a ship without engine hardpoints counts as having drives");
}

#[test]
fn destroying_engines_cuts_ship_speed_by_80_percent() {
    let mut eng1 = engine();
    eng1.health = 100.0;
    let mut eng2 = engine();
    eng2.health = 100.0;
    let mut gun_hp = gun("g", 0.0);
    gun_hp.range = 3000.0;
    gun_hp.health = 1000.0;
    let runner_ship = runner("runner", vec![eng1, eng2]);
    let gunner = ship("gunner", 1000.0, 0.0, vec![gun_hp]);
    let cat = Catalog::from_defs(vec![runner_ship, gunner], vec![laser()]);
    let mut b = Battle::new(setup(cat, "runner", "gunner", 1, 1, cfg()));
    b.ships[0].pos = Vec3::ZERO;
    b.ships[1].pos = Vec3::new(200.0, 0.0, 0.0);
    assert_eq!(b.ships[0].engine_factor, 1.0);

    b.command(1, Command::AttackHardpoint { ships: vec![1], target: 0, hardpoint: 0 }).unwrap();
    for _ in 0..100 {
        b.step();
        if !b.ships[0].hps[0].alive {
            break;
        }
    }
    assert!(!b.ships[0].hps[0].alive, "first engine should be destroyed");
    assert!((b.ships[0].engine_factor - 0.60).abs() < 1e-3, "1 of 2 engines destroyed -> 60% speed remaining");

    b.command(1, Command::AttackHardpoint { ships: vec![1], target: 0, hardpoint: 1 }).unwrap();
    for _ in 0..100 {
        b.step();
        if !b.ships[0].hps[1].alive {
            break;
        }
    }
    assert!(!b.ships[0].hps[1].alive, "second engine should be destroyed");
    assert!((b.ships[0].engine_factor - 0.20).abs() < 1e-3, "all engines destroyed -> 20% speed (80% cut)");
}

fn carrier_battle(jump: f32) -> Battle {
    let mut fighter = ship("fighter", 20.0, 0.0, vec![engine()]);
    fighter.class = TargetClass::Fighter;
    fighter.is_static = false;
    fighter.max_speed = 600.0;
    fighter.accel = 600.0;
    fighter.turn_rate = 3.0;
    fighter.radius = 10.0;
    let squad = ShipDef { name: "squad".into(), is_squadron: true, members: vec![(0, 2)], class: TargetClass::Fighter, ..ShipDef::default() };
    let mut carrier = runner("carrier", vec![engine(), HpDef { kind: HpKind::FighterBay, health: 100.0, ..HpDef::default() }]);
    carrier.carried = vec![(1, 1)];
    carrier.reserve = vec![(1, 1)];
    carrier.spawn_delay = 1.0;
    retreat_battle(vec![fighter, squad, carrier, idle_target()], "carrier", 1, jump)
}

#[test]
fn squadrons_spawned_in_the_battle_escape_only_with_their_carrier() {
    // The carrier escapes: its launched squadron comes along.
    let mut b = carrier_battle(2.0);
    let fighters: Vec<usize> = b.ships.iter().filter(|s| s.def == 0).map(|s| s.id as usize).collect();
    assert_eq!(fighters.len(), 2, "the starting squadron was launched");
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..70 {
        b.step();
    }
    let carrier = b.ships.iter().find(|s| s.def == 2).unwrap();
    assert!(carrier.retreated);
    assert!(fighters.iter().all(|&f| b.ships[f].alive && b.ships[f].retreated), "squadron escaped with its carrier");

    // The carrier cannot jump (engine destroyed): carrier and squadron are lost.
    let mut b = carrier_battle(2.0);
    let c = b.ships.iter().position(|s| s.def == 2).unwrap();
    b.ships[c].hps[0].alive = false;
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..70 {
        b.step();
    }
    assert!(!b.ships[c].alive);
    assert!(b.ships.iter().filter(|s| s.def == 0).all(|s| !s.alive && !s.retreated), "squadron stays behind and is lost");
}

#[test]
fn squadrons_called_in_as_reinforcements_do_not_escape_on_their_own() {
    let mut b = carrier_battle(8.0);
    b.command(0, Command::Reinforce { ship: 1, count: 1 }).unwrap();
    for _ in 0..(6.0 * 30.0) as usize {
        b.step();
    }
    let reinforced: Vec<usize> = b.ships.iter().filter(|s| s.def == 0 && s.carrier == u32::MAX).map(|s| s.id as usize).collect();
    assert_eq!(reinforced.len(), 2, "the reinforcement squadron arrived");
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..(8.5 * 30.0) as usize {
        b.step();
    }
    assert!(b.ships.iter().find(|s| s.def == 2).unwrap().retreated, "the carrier escaped");
    assert!(reinforced.iter().all(|&f| !b.ships[f].alive && !b.ships[f].retreated), "reinforcement squadrons are lost");
}

#[test]
fn a_side_stays_in_the_fight_while_anything_it_brought_lives_even_a_fighter() {
    // Side 0 kills side 1's only capital ship at once; side 1 also brought a squadron fighter that nothing can hurt: the battle goes on to the time limit
    // (units spawned during the battle do not count, units of the setup do; see the real-data pirate test for the spawned ones).
    let mut gunner = ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0)]);
    gunner.is_static = false;
    gunner.max_speed = 600.0;
    gunner.turn_rate = 2.0;
    gunner.accel = 400.0;
    let weak = ship("weak", 50.0, 0.0, vec![]);
    let mut fighter = ship("fighter", 1.0e9, 0.0, vec![]);
    fighter.class = TargetClass::Fighter;
    fighter.is_static = false;
    let cat = Arc::new(Catalog::from_defs(vec![gunner, weak, fighter], vec![laser()]));
    let id = |n: &str| cat.ship_id(n).unwrap();
    let side = |n: &str, units: Vec<(&str, u32)>| SideSetup {
        name: n.into(),
        units: units.into_iter().map(|(s, c)| UnitSetup { ship: id(s), count: c, tag: 7 }).collect(),
        structures: vec![],
        ai: false,
    };
    let config = BattleConfig { half_size: 6000.0, max_time: 200.0, fog_of_war: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { sides: vec![side("a", vec![("gunner", 1)]), side("b", vec![("weak", 1), ("fighter", 1)])], catalog: cat.clone(), config });
    let weak_id = b.ships.iter().find(|s| s.side == 1 && s.def == id("weak")).unwrap().id;
    let gunner_id = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    b.command(0, Command::Attack { ships: vec![gunner_id], target: weak_id }).unwrap();
    let mut died_at = None;
    while !b.is_over() && b.time() < 400.0 {
        b.step();
        if died_at.is_none() && !b.ships[weak_id as usize].alive {
            died_at = Some(b.time());
        }
    }
    let died = died_at.expect("the capital ship died");
    assert!(b.time() >= 195.0 && died < 150.0, "the brought fighter keeps its side in the fight until the time limit: died {died:.0}, over {:.0}", b.time());
    assert_eq!(b.result().winner, None);
}

fn hardpoint_target(hp_health: f32, n: usize, hull: f32) -> (Battle, u32, u32) {
    let mut gunner = ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0)]);
    gunner.is_static = true;
    let parts: Vec<HpDef> = (0..n)
        .map(|k| HpDef {
            name: format!("part{k}"),
            kind: HpKind::Other,
            health: hp_health,
            destroyable: true,
            targetable: true,
            offset: Vec3::new(k as f32 * 20.0, 0.0, 0.0),
            ..HpDef::default()
        })
        .collect();
    let target = ship("target", hull, 0.0, parts);
    let cat = Catalog::from_defs(vec![gunner, target], vec![laser()]);
    let mut b = Battle::new(setup(cat, "gunner", "target", 1, 1, cfg()));
    let (g, t) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    b.command(0, Command::Attack { ships: vec![g], target: t }).unwrap();
    (b, g, t)
}

/// Steps until the first hit on `target`; returns the damage of that hit.
fn first_hit(b: &mut Battle, target: u32) -> f32 {
    for _ in 0..30 * 60 {
        for e in b.step() {
            if let Event::Hit { ship, damage, .. } = e
                && *ship == target
            {
                return *damage;
            }
        }
    }
    panic!("no hit");
}

#[test]
fn a_unit_with_hardpoints_dies_when_the_last_hardpoint_is_destroyed_not_by_a_hull_pool() {
    // The hull figure is huge (1e9) and no hit could ever empty it, yet four weak hardpoints (5 hp) die to the spill of a single 100 point hit: 100, 47, 21, 8.
    let (mut b, _, t) = hardpoint_target(5.0, 4, 1.0e9);
    let dmg = first_hit(&mut b, t);
    assert!((dmg - 100.0).abs() < 0.5, "damage {dmg}");
    assert!(b.ships[t as usize].hps.iter().all(|h| !h.alive), "{:?}", b.ships[t as usize].hps.iter().map(|h| h.health).collect::<Vec<_>>());
    assert!(!b.ships[t as usize].alive, "destroyed with its last hardpoint");

    // A unit that keeps hardpoints lives on whatever its hull number says: 5 hardpoints of 1000 hp take the first hit on one of them, nothing spills.
    let (mut b, _, t) = hardpoint_target(1000.0, 5, 1.0);
    let dmg = first_hit(&mut b, t);
    let hps = &b.ships[t as usize].hps;
    assert!(b.ships[t as usize].alive && hps.iter().all(|h| h.alive), "a hit under the health of one hardpoint destroys nothing");
    assert!((hps.iter().map(|h| 1000.0 - h.health).sum::<f32>() - dmg.round()).abs() < 1.5, "the whole hit went into one hardpoint (no spill without overkill)");
    assert!(hps.iter().filter(|h| h.health < 1000.0).count() == 1);
}

#[test]
fn overkill_spills_to_the_next_hardpoints_halving_every_step_in_whole_numbers() {
    // Hardpoints of 60 and a hit of 100: #1 is destroyed with 40 over, #2 gets 20, #3 and #4 are untouched (nothing over).
    let (mut b, _, t) = hardpoint_target(60.0, 4, 1.0e9);
    let d = first_hit(&mut b, t).round();
    let over1 = (d - 60.0).max(0.0);
    let in2 = (over1 / 2.0).floor();
    let in3 = ((in2 - 60.0).max(0.0) / 2.0).floor();
    let mut healths: Vec<f32> = b.ships[t as usize].hps.iter().map(|h| h.health).collect();
    healths.sort_by(f32::total_cmp);
    let mut expected = vec![60.0 - d, 60.0 - in2, 60.0 - in3, 60.0];
    expected.sort_by(f32::total_cmp);
    assert_eq!(healths, expected, "hit {d}, spill {in2} then {in3}");
    // Whole numbers only.
    assert!(healths.iter().all(|h| h.fract() == 0.0));
}

#[test]
fn spawn_yaw_returns_arrival_yaw_for_each_side() {
    let b = carrier_battle(8.0);
    assert!((b.spawn_yaw(0) - 0.0).abs() < 1e-4);
    assert!((b.spawn_yaw(1) - std::f32::consts::PI).abs() < 1e-4);
}

#[test]
fn ground_to_space_special_weapon_fires_at_enemy_capital_ship() {
    let mut cat = Catalog::default();
    cat.projectiles.push(ProjDef {
        name: "special_proj".into(),
        damage: 1500.0,
        speed: 10000.0,
        max_distance: 99000.0,
        shield_damage: true,
        hull_damage: true,
        ..ProjDef::default()
    });
    cat.ships.push(ShipDef {
        name: "hypervelocity_gun".into(),
        class: TargetClass::Structure,
        is_static: true,
        hull: 3000.0,
        special_weapon: Some(SpecialWeaponDef {
            projectile: 0,
            recharge: 5.0,
            pulse_count: 1,
            pulse_delay: 0.5,
            valid_targets: TargetClass::Capital.bit(),
            victory_relevant: false,
        }),
        ..ShipDef::default()
    });
    cat.ships.push(ShipDef {
        name: "enemy_capital".into(),
        class: TargetClass::Capital,
        is_static: true,
        hull: 4000.0,
        shield: 0.0,
        radius: 200.0,
        ..ShipDef::default()
    });
    let cat = Arc::new(cat);
    let sides = vec![
        SideSetup {
            name: "defender".into(),
            units: vec![],
            structures: vec![0], // hypervelocity gun
            ai: true,
        },
        SideSetup {
            name: "attacker".into(),
            units: vec![UnitSetup { ship: 1, count: 1, tag: 0 }], // enemy capital
            structures: vec![],
            ai: false,
        },
    ];
    let mut b = Battle::new(BattleSetup { catalog: cat, sides, config: BattleConfig { dt: 0.1, ..BattleConfig::default() } });
    b.ships[0].special_cd = 0.0;
    let initial_hull = b.ships[1].hull;
    for _ in 0..20 {
        b.step();
    }
    assert!(b.ships[1].hull < initial_hull, "enemy capital should take damage from special ground-to-space weapon");
}

#[test]
fn orphaned_carrier_fighters_die_and_victory_resolves_cleanly() {
    let mut cat = Catalog::default();
    cat.ships.push(ShipDef {
        name: "carrier".into(),
        class: TargetClass::Structure,
        is_static: true,
        hull: 100.0,
        radius: 200.0,
        carried: vec![(1, 1)],
        ..ShipDef::default()
    });
    cat.ships.push(ShipDef {
        name: "fighter_sq".into(),
        is_squadron: true,
        members: vec![(2, 2)],
        ..ShipDef::default()
    });
    cat.ships.push(ShipDef {
        name: "fighter".into(),
        class: TargetClass::Fighter,
        hull: 50.0,
        radius: 10.0,
        ..ShipDef::default()
    });
    cat.ships.push(ShipDef {
        name: "player_ship".into(),
        class: TargetClass::Frigate,
        hull: 1000.0,
        radius: 100.0,
        ..ShipDef::default()
    });
    let cat = Arc::new(cat);
    let sides = vec![
        SideSetup {
            name: "player".into(),
            units: vec![UnitSetup { ship: 3, count: 1, tag: 0 }],
            structures: vec![],
            ai: false,
        },
        SideSetup {
            name: "enemy".into(),
            units: vec![],
            structures: vec![0], // enemy station/carrier
            ai: true,
        },
    ];
    let mut b = Battle::new(BattleSetup { catalog: cat, sides, config: BattleConfig { dt: 0.1, ..BattleConfig::default() } });
    assert_eq!(b.ships.len(), 4); // player frigate, enemy station, 2 fighters
    let enemy_station = b.ships.iter().position(|s| s.side == 1 && s.carrier == NONE).unwrap();
    // Kill the enemy station
    b.ships[enemy_station].hull = 0.0;
    b.ships[enemy_station].alive = false;
    b.step();
    // Orphaned fighters must be dead
    let surviving_enemy: Vec<_> = b.ships.iter().filter(|s| s.side == 1 && s.alive).collect();
    assert!(surviving_enemy.is_empty(), "all enemy fighters should be destroyed with the station");
    assert!(b.is_over(), "battle should be over");
    assert_eq!(b.winner(), Some(0), "player should be victorious");
}

#[test]
fn hardpoint_focus_fire_and_reset_on_destruction() {
    let mut hp1 = gun("hp1", 50.0);
    hp1.offset = Vec3::new(0.0, 50.0, 0.0);
    let mut hp2 = gun("hp2", 50.0);
    hp2.offset = Vec3::new(0.0, -50.0, 0.0);
    let target_ship = ship("target", 200.0, 0.0, vec![hp1, hp2]);

    let mut attacker_gun = gun("atk_gun", 0.0);
    attacker_gun.range = 2000.0;
    attacker_gun.health = 1000.0;
    let attacker = mobile("attacker", TargetClass::Frigate, 200.0, 1000.0, vec![attacker_gun]);

    let cat = Catalog::from_defs(vec![attacker, target_ship], vec![laser()]);
    let mut b = Battle::new(setup(cat, "attacker", "target", 1, 1, cfg()));
    b.ships[0].pos = Vec3::ZERO;
    b.ships[1].pos = Vec3::new(300.0, 0.0, 0.0);

    // Target hardpoint 0
    b.command(0, Command::AttackHardpoint { ships: vec![0], target: 1, hardpoint: 0 }).unwrap();
    assert_eq!(b.ships[0].target_hp, 0);

    // Run until hardpoint 0 dies
    for _ in 0..100 {
        b.step();
        if !b.ships[1].hps[0].alive {
            break;
        }
    }
    assert!(!b.ships[1].hps[0].alive, "hardpoint 0 should be destroyed");
    assert_eq!(b.ships[0].target_hp, NO_HP, "attacker target_hp should reset to NO_HP after hardpoint is destroyed");

    // Cannot order attack on already destroyed hardpoint
    assert_eq!(
        b.command(0, Command::AttackHardpoint { ships: vec![0], target: 1, hardpoint: 0 }),
        Err(CommandError::InvalidHardpoint)
    );
}

#[test]
fn interceptor_doctrine_prioritizes_enemy_bombers_over_closer_fighters() {
    let mut interceptor = ship("A_Wing_Interceptor", 100.0, 50.0, vec![gun("laser", 0.0)]);
    interceptor.class = TargetClass::Fighter;
    interceptor.is_static = false;
    interceptor.max_weapon_range = 3000.0;
    interceptor.targeting_range = 3000.0;

    let mut standard_fighter = ship("X_Wing_Fighter", 100.0, 50.0, vec![gun("laser", 0.0)]);
    standard_fighter.class = TargetClass::Fighter;
    standard_fighter.is_static = false;
    standard_fighter.max_weapon_range = 3000.0;
    standard_fighter.targeting_range = 3000.0;

    let mut enemy_fighter = ship("TIE_Fighter", 50.0, 0.0, vec![]);
    enemy_fighter.class = TargetClass::Fighter;
    enemy_fighter.is_static = false;

    let mut enemy_bomber = ship("TIE_Bomber", 80.0, 0.0, vec![]);
    enemy_bomber.class = TargetClass::Bomber;
    enemy_bomber.is_static = false;

    let cat = Catalog::from_defs(vec![interceptor, standard_fighter, enemy_fighter, enemy_bomber], vec![laser()]);
    let cat = Arc::new(cat);

    // Test 1: Interceptor vs (Closer Fighter at 400m, Farther Bomber at 1200m)
    let setup_intercept = BattleSetup {
        catalog: cat.clone(),
        sides: vec![
            SideSetup { name: "reb".into(), units: vec![UnitSetup { ship: 0, count: 1, tag: 1 }], structures: vec![], ai: true },
            SideSetup { name: "imp".into(), units: vec![UnitSetup { ship: 2, count: 1, tag: 1 }, UnitSetup { ship: 3, count: 1, tag: 1 }], structures: vec![], ai: true },
        ],
        config: BattleConfig { fog_of_war: false, record_events: false, ..cfg() },
    };
    let mut b1 = Battle::new(setup_intercept);
    b1.ships[0].pos = Vec3::ZERO;
    b1.ships[1].pos = Vec3::new(400.0, 0.0, 0.0);
    b1.ships[2].pos = Vec3::new(1200.0, 0.0, 0.0);
    for _ in 0..30 {
        b1.step();
        if b1.ships[0].target != NONE {
            break;
        }
    }
    assert_eq!(b1.ships[0].target, 2, "interceptor prioritizes bomber (id 2) over closer fighter (id 1)");

    // Test 2: Standard Fighter with same distances targets closer fighter (id 1)
    let setup_std = BattleSetup {
        catalog: cat.clone(),
        sides: vec![
            SideSetup { name: "reb".into(), units: vec![UnitSetup { ship: 1, count: 1, tag: 1 }], structures: vec![], ai: true },
            SideSetup { name: "imp".into(), units: vec![UnitSetup { ship: 2, count: 1, tag: 1 }, UnitSetup { ship: 3, count: 1, tag: 1 }], structures: vec![], ai: true },
        ],
        config: BattleConfig { fog_of_war: false, record_events: false, ..cfg() },
    };
    let mut b2 = Battle::new(setup_std);
    b2.ships[0].pos = Vec3::ZERO;
    b2.ships[1].pos = Vec3::new(400.0, 0.0, 0.0);
    b2.ships[2].pos = Vec3::new(1200.0, 0.0, 0.0);
    for _ in 0..30 {
        b2.step();
        if b2.ships[0].target != NONE {
            break;
        }
    }
    assert_eq!(b2.ships[0].target, 1, "standard fighter targets closer enemy fighter (id 1)");
}


