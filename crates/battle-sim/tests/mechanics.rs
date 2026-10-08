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
    let cat = Catalog::from_defs(vec![ship("a", 1.0e6, 0.0, vec![]), ship("b", 1.0e6, 0.0, vec![])], vec![laser()]);
    let mut c = cfg();
    c.unit_cap = 100;
    let mut b = Battle::new(setup(cat, "a", "b", 1, 1, c));
    let a = b.catalog().ship_id("a").unwrap();
    let (own, foe) = (b.ships.iter().find(|s| s.side == 0).unwrap().pos, b.ships.iter().find(|s| s.side == 1).unwrap().pos);
    let toward_enemy = (foe - own).normalized();
    let beside = Vec3::new(own.x + 600.0, own.y + 300.0, own.z);
    // Near an own ship, far from the enemy: accepted.
    assert_eq!(b.reinforcement_zone_check(0, beside), Ok(()));
    // Far from everything of its own.
    let far = own - toward_enemy * 3500.0;
    assert!(matches!(b.reinforcement_zone_check(0, far), Err(CommandError::NotNearOwnForces | CommandError::OutsideBattleArea)), "{:?}", b.reinforcement_zone_check(0, far));
    // Within 2000 of the enemy.
    let near_enemy = foe + Vec3::new(500.0, 0.0, 0.0);
    assert_eq!(b.reinforcement_zone_check(0, near_enemy), Err(CommandError::TooCloseToEnemy));
    // Outside the map.
    assert_eq!(b.reinforcement_zone_check(0, Vec3::new(1.0e6, 0.0, 0.0)), Err(CommandError::OutsideBattleArea));
    assert_eq!(b.command(0, Command::ReinforceAt { ship: a, count: 1, pos: near_enemy }), Err(CommandError::TooCloseToEnemy));
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
    let mut b = Battle::new(setup(cat, "ghost", "watcher", 1, 1, cfg()));
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
    let bomber = mobile("bomber", TargetClass::Bomber, 400.0, 100.0, vec![]);
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
