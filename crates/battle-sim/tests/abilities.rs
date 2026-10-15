//! Rule tests of the interdiction field and the `HUNT` ability with hand-made definitions (no game data needed).

use std::sync::Arc;

use battle_sim::*;

fn laser() -> ProjDef {
    ProjDef { name: "laser".into(), category: "laser".into(), damage: 100.0, speed: 2000.0, max_distance: 9000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() }
}

fn mobile(name: &str, speed: f32) -> ShipDef {
    ShipDef {
        name: name.into(),
        class: TargetClass::Frigate,
        is_static: false,
        hull: 1.0e6,
        radius: 100.0,
        max_speed: speed,
        accel: 3000.0,
        turn_rate: 6.0,
        population: 5,
        ..ShipDef::default()
    }
}

fn toggle(kind: &str) -> AbilityDef {
    AbilityDef {
        kind: kind.into(),
        toggle: true,
        duration: f32::MAX / 4.0,
        speed_mult: 1.0,
        shield_regen_mult: 1.0,
        weapon_delay_mult: 1.0,
        take_damage_mult: 1.0,
        cause_damage_mult: 1.0,
        ..AbilityDef::default()
    }
}

fn interdict() -> AbilityDef {
    AbilityDef { interdict: true, disable_movement: true, ..toggle("INTERDICT") }
}

fn hunt() -> AbilityDef {
    AbilityDef { hunt: true, ..toggle("HUNT") }
}

fn setup(catalog: Catalog, a: (&str, u32, bool), b: (&str, u32, bool), config: BattleConfig) -> BattleSetup {
    let catalog = Arc::new(catalog);
    let side = |(n, c, ai): (&str, u32, bool)| SideSetup {
        name: n.into(),
        units: vec![UnitSetup { ship: catalog.ship_id(n).unwrap(), count: c, tag: 7 }],
        structures: vec![],
        ai,
    };
    BattleSetup { sides: vec![side(a), side(b)], catalog: catalog.clone(), config }
}

fn ids(b: &Battle, side: u8) -> Vec<u32> {
    b.ships.iter().filter(|s| s.side == side).map(|s| s.id).collect()
}

fn interdictor_battle(ai_interdictor: bool) -> Battle {
    let mut field = mobile("field", 500.0);
    field.abilities.push(interdict());
    let cat = Catalog::from_defs(vec![mobile("runner", 800.0), field], vec![laser()]);
    let config = BattleConfig { half_size: 6000.0, retreat_countdown: 0.0, retreat_jump_seconds: 10.0, ..BattleConfig::default() };
    Battle::new(setup(cat, ("runner", 2, false), ("field", 1, ai_interdictor), config))
}

#[test]
fn interdiction_switched_on_during_the_jump_countdown_cancels_the_retreat() {
    let mut b = interdictor_battle(false);
    let field = ids(&b, 1)[0];
    b.command(0, Command::Retreat).unwrap();
    for _ in 0..60 {
        b.step();
    }
    assert!(b.retreat_jump_in(0).is_some(), "the jump is charging");
    b.command(1, Command::UseAbility { ship: field, ability: 0 }).unwrap();
    let events: Vec<Event> = b.step().to_vec();
    assert!(events.contains(&Event::RetreatAborted { side: 0 }), "{events:?}");
    assert_eq!(b.retreat_jump_in(0), None);
    assert!(ids(&b, 0).iter().all(|&i| !matches!(b.ships[i as usize].order, Order::Retreat)), "the fleet stops running for the edge");
    assert_eq!(b.command(0, Command::Retreat), Err(CommandError::RetreatBlocked));
    // Nobody jumped even after the original countdown.
    for _ in 0..30 * 12 {
        b.step();
    }
    assert!(ids(&b, 0).iter().all(|&i| !b.ships[i as usize].retreated));
}

#[test]
fn interdicted_retreat_works_again_when_the_field_goes_off() {
    let mut b = interdictor_battle(false);
    let field = ids(&b, 1)[0];
    b.command(1, Command::UseAbility { ship: field, ability: 0 }).unwrap();
    assert_eq!(b.command(0, Command::Retreat), Err(CommandError::RetreatBlocked));
    b.command(1, Command::UseAbility { ship: field, ability: 0 }).unwrap();
    assert_eq!(b.ships[field as usize].abilities[0].active, 0.0, "used again, the toggle goes off");
    assert_eq!(b.command(0, Command::Retreat), Ok(()));
}

#[test]
fn the_field_pins_its_own_ship() {
    let mut b = interdictor_battle(false);
    let field = ids(&b, 1)[0];
    b.command(1, Command::UseAbility { ship: field, ability: 0 }).unwrap();
    let start = b.ships[field as usize].pos;
    b.command(1, Command::Move { ships: vec![field], to: Vec3::ZERO }).unwrap();
    for _ in 0..90 {
        b.step();
    }
    assert!(b.ships[field as usize].pos.dist(start) < 1.0, "Disable_Movement: the interdictor cannot move while the field is up");
}

#[test]
fn an_ai_interdictor_switches_on_against_a_retreat_and_off_after_twenty_seconds() {
    let mut b = interdictor_battle(true);
    let field = ids(&b, 1)[0];
    b.step();
    assert_eq!(b.ships[field as usize].abilities[0].active, 0.0, "idle until somebody retreats");
    b.command(0, Command::Retreat).unwrap();
    let mut aborted = false;
    for _ in 0..60 {
        aborted |= b.step().contains(&Event::RetreatAborted { side: 0 });
    }
    assert!(aborted, "the AI interdictor answers the retreat");
    assert!(b.ships[field as usize].abilities[0].active > 0.0);
    assert_eq!(b.command(0, Command::Retreat), Err(CommandError::RetreatBlocked));
    for _ in 0..30 * 21 {
        b.step();
    }
    assert_eq!(b.ships[field as usize].abilities[0].active, 0.0, "Register_Timer(Cancel_Interdiction, 20)");
    assert_eq!(b.command(0, Command::Retreat), Ok(()));
}

fn hunt_battle(fog: bool, hunters: u32, prey: u32, prey_x_spread: bool) -> Battle {
    let mut h = mobile("hunter", 900.0);
    h.abilities.push(hunt());
    h.sensor_range = 1500.0;
    let mut p = mobile("prey", 0.0);
    p.radius = 60.0;
    let cat = Catalog::from_defs(vec![h, p], vec![laser()]);
    let config = BattleConfig { half_size: 6000.0, fog_of_war: fog, ..BattleConfig::default() };
    let mut b = Battle::new(setup(cat, ("hunter", hunters, false), ("prey", prey, false), config));
    if prey_x_spread {
        // Prey spread along a line, 700 apart.
        let origin = b.ships.iter().find(|s| s.side == 1).unwrap().pos;
        for (n, id) in ids(&b, 1).into_iter().enumerate() {
            b.ships[id as usize].pos = origin + Vec3::new(0.0, n as f32 * 700.0, 0.0);
        }
    }
    b
}

fn turn_on_hunt(b: &mut Battle) {
    for id in ids(b, 0) {
        b.command(0, Command::UseAbility { ship: id, ability: 0 }).unwrap();
    }
}

#[test]
fn hunters_chase_visible_enemies_that_idle_ships_leave_alone() {
    let reach = |hunting: bool| {
        let mut b = hunt_battle(false, 2, 1, false);
        if hunting {
            turn_on_hunt(&mut b);
        }
        for _ in 0..30 * 40 {
            b.step();
        }
        let prey = ids(&b, 1)[0];
        ids(&b, 0).iter().map(|&i| b.ships[i as usize].pos.dist(b.ships[prey as usize].pos)).fold(f32::MAX, f32::min)
    };
    let (idle, hunting) = (reach(false), reach(true));
    assert!(hunting < 900.0, "a hunter comes up to the enemy: {hunting}");
    assert!(hunting + 1000.0 < idle, "idle ships stay behind: idle {idle}, hunting {hunting}");
}

#[test]
fn hunters_spread_over_the_enemies() {
    let mut b = hunt_battle(false, 4, 4, true);
    turn_on_hunt(&mut b);
    for _ in 0..30 {
        b.step();
    }
    let mut targets: Vec<u32> = ids(&b, 0)
        .iter()
        .filter_map(|&i| if let Order::Attack(t) = b.ships[i as usize].order { Some(t) } else { None })
        .collect();
    assert_eq!(targets.len(), 4, "every hunter has a target");
    targets.sort_unstable();
    targets.dedup();
    assert_eq!(targets.len(), 4, "four hunters, four different enemies: {targets:?}");
}

#[test]
fn hunters_search_the_map_and_find_hidden_enemies() {
    let mut b = hunt_battle(true, 2, 1, false);
    turn_on_hunt(&mut b);
    let prey = ids(&b, 1)[0];
    let (mut searched, mut engaged) = (false, false);
    let mut cells = std::collections::BTreeSet::new();
    for _ in 0..30 * 120 {
        b.step();
        for id in ids(&b, 0) {
            let s = &b.ships[id as usize];
            if s.hunt_cell != NO_CELL {
                searched = true;
                cells.insert((id, s.hunt_cell));
            }
            engaged |= s.order == Order::Attack(prey);
        }
        if engaged {
            break;
        }
    }
    assert!(searched, "with nothing in sight the hunters search cells of the map");
    assert!(cells.iter().map(|c| c.1).collect::<std::collections::BTreeSet<_>>().len() >= 2, "different cells: {cells:?}");
    assert!(engaged, "and attack the enemy once their sensors detect it");
}

#[test]
fn a_move_order_or_toggling_off_ends_the_hunt() {
    let mut b = hunt_battle(false, 2, 1, false);
    turn_on_hunt(&mut b);
    for _ in 0..30 {
        b.step();
    }
    let [a, c] = ids(&b, 0)[..] else { panic!("two hunters") };
    assert!(matches!(b.ships[a as usize].order, Order::Attack(_)));
    b.command(0, Command::Move { ships: vec![a], to: Vec3::ZERO }).unwrap();
    assert_eq!(b.ships[a as usize].abilities[0].active, 0.0, "an order of its own stops the hunt");
    b.command(0, Command::UseAbility { ship: c, ability: 0 }).unwrap();
    assert_eq!(b.ships[c as usize].abilities[0].active, 0.0);
    assert_eq!(b.ships[c as usize].order, Order::Idle, "toggled off: back to idle");
    for _ in 0..60 {
        b.step();
    }
    assert!(matches!(b.ships[a as usize].order, Order::Move(_)), "the hunt does not overwrite the new order");
}

fn load() -> Option<(alamo_data::GameData, alamo_fs::GameFs)> {
    let install = alamo_fs::GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = alamo_fs::GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = alamo_data::load_galaxy(&fs).0;
    Some((data, fs))
}

#[test]
fn hunt_and_spoiler_lock_are_toggles_in_the_real_data() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["TIE_Fighter", "Rebel_X-Wing", "Immobilizer_Cruiser_Template", "Dominator_Star_Destroyer"]);
    for name in ["TIE_Fighter", "Rebel_X-Wing"] {
        let Some(id) = cat.ship_id(name) else { continue };
        let hunt = cat.ship(id).abilities.iter().find(|a| a.hunt).unwrap_or_else(|| panic!("{name} has HUNT"));
        assert!(hunt.toggle && hunt.recharge >= 0.0, "{name}: {hunt:?}");
    }
    let imm = cat.ship_id("Immobilizer_Cruiser_Template").expect("immobilizer");
    let field = cat.ship(imm).abilities.iter().find(|a| a.interdict).expect("INTERDICT");
    assert!(field.toggle && field.disable_movement, "{field:?}");
}

fn wing_battle() -> Battle {
    let mut wing = mobile("wing", 600.0);
    wing.class = TargetClass::Fighter;
    wing.hull = 100.0;
    let replenish = AbilityDef {
        replenish: true,
        supports_autofire: true,
        autocast_default: true,
        recharge: 99999.0,
        duration: 1.0,
        speed_mult: 1.0,
        shield_regen_mult: 1.0,
        weapon_delay_mult: 1.0,
        take_damage_mult: 1.0,
        cause_damage_mult: 1.0,
        kind: "REPLENISH_WINGMEN".into(),
        ..AbilityDef::default()
    };
    wing.abilities.push(replenish);
    let squad = ShipDef { name: "squad".into(), is_squadron: true, members: vec![(0, 3)], class: TargetClass::Fighter, ..ShipDef::default() };
    let cat = Catalog::from_defs(vec![wing, squad, mobile("far", 0.0)], vec![laser()]);
    Battle::new(setup(cat, ("squad", 1, false), ("far", 1, false), BattleConfig { half_size: 6000.0, ..BattleConfig::default() }))
}

#[test]
fn replenish_wingmen_brings_back_the_squadron_once() {
    let mut b = wing_battle();
    let mine = ids(&b, 0);
    assert_eq!(mine.len(), 3);
    for _ in 0..60 {
        b.step();
    }
    assert_eq!(ids(&b, 0).len(), 3, "nobody lost: the once-per-battle use is not spent");
    assert!(b.ships[mine[0] as usize].abilities[0].cooldown <= 0.0);
    // Two wingmen die: the lone leader calls them back.
    for &m in &mine[1..] {
        b.ships[m as usize].alive = false;
    }
    for _ in 0..60 {
        b.step();
    }
    let alive: Vec<u32> = ids(&b, 0).into_iter().filter(|&i| b.ships[i as usize].alive).collect();
    assert_eq!(alive.len(), 3, "both wingmen are back");
    assert!(b.ships[mine[0] as usize].abilities[0].cooldown > 1000.0, "one use per battle");
    // They die again: nothing comes back.
    for &m in &alive[1..] {
        b.ships[m as usize].alive = false;
    }
    for _ in 0..60 {
        b.step();
    }
    assert_eq!(ids(&b, 0).into_iter().filter(|&i| b.ships[i as usize].alive).count(), 1);
}

#[test]
fn the_gravity_well_station_always_interdicts() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["E_Gravity_Well_Station"]);
    let id = cat.ship_id("E_Gravity_Well_Station").expect("station in the catalog");
    assert!(cat.ship(id).interdictor);
}

fn gun() -> HpDef {
    HpDef {
        name: "gun".into(),
        kind: HpKind::Weapon,
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

fn weaken_battle() -> Battle {
    let mut shooter = mobile("shooter", 0.0);
    shooter.is_static = true;
    shooter.hardpoints.push(gun());
    let mut admiral = mobile("admiral", 0.0);
    admiral.is_static = true;
    admiral.abilities.push(AbilityDef {
        kind: "WEAKEN_ENEMY".into(),
        weaken: Some(WeakenDef { radius: 2000.0, duration: 60.0, cause_reduction: 0.25, take_increase: 0.0, speed_cut: 0.25 }),
        duration: 12.0,
        recharge: 120.0,
        speed_mult: 1.0,
        shield_regen_mult: 1.0,
        weapon_delay_mult: 1.0,
        take_damage_mult: 1.0,
        cause_damage_mult: 1.0,
        ..AbilityDef::default()
    });
    let cat = Catalog::from_defs(vec![shooter, admiral], vec![laser()]);
    Battle::new(setup(cat, ("shooter", 1, false), ("admiral", 1, false), BattleConfig { half_size: 6000.0, ..BattleConfig::default() }))
}

#[test]
fn weaken_enemy_cuts_damage_dealt_and_speed_in_the_area_for_a_minute() {
    let loss = |weaken: bool| {
        let mut b = weaken_battle();
        let (shooter, admiral) = (ids(&b, 0)[0], ids(&b, 1)[0]);
        if weaken {
            let at = b.ships[shooter as usize].pos;
            b.command(1, Command::UseAbilityAt { ship: admiral, ability: 0, pos: at }).unwrap();
            let s = &b.ships[shooter as usize];
            assert!(s.mod_value(ModKind::CauseDamage) < 0.0 && s.mod_value(ModKind::Speed) < 0.0, "the enemy is marked");
        }
        let hull0 = b.ships[admiral as usize].hull;
        for _ in 0..30 * 20 {
            b.step();
        }
        hull0 - b.ships[admiral as usize].hull
    };
    let (plain, weak) = (loss(false), loss(true));
    assert!(plain > 100.0, "the shooter hurts the admiral: {plain}");
    assert!((weak / plain - 0.75).abs() < 0.1, "25% less damage: {plain} vs {weak}");
}

#[test]
fn weaken_enemy_misses_ships_outside_the_area_and_runs_out() {
    let mut b = weaken_battle();
    let (shooter, admiral) = (ids(&b, 0)[0], ids(&b, 1)[0]);
    let far = b.ships[shooter as usize].pos + Vec3::new(5000.0, 0.0, 0.0);
    b.command(1, Command::UseAbilityAt { ship: admiral, ability: 0, pos: far }).unwrap();
    assert!(b.ships[shooter as usize].mod_value(ModKind::CauseDamage) == 0.0, "outside the 2000 radius");
    let at = b.ships[shooter as usize].pos;
    b.ships[admiral as usize].abilities[0].cooldown = 0.0;
    b.ships[admiral as usize].abilities[0].active = 0.0;
    b.command(1, Command::UseAbilityAt { ship: admiral, ability: 0, pos: at }).unwrap();
    assert!(b.ships[shooter as usize].mod_value(ModKind::CauseDamage) < 0.0);
    for _ in 0..30 * 61 {
        b.step();
    }
    assert!(b.ships[shooter as usize].mod_value(ModKind::CauseDamage) == 0.0, "after 60 s the effect is gone");
}

#[test]
fn weaken_enemy_values_come_from_the_spawned_projectile() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Admonitor_Star_Destroyer"]);
    let id = cat.ship_id("Admonitor_Star_Destroyer").expect("ship");
    let w = cat.ship(id).abilities.iter().find_map(|a| a.weaken.clone()).expect("WEAKEN_ENEMY");
    assert_eq!((w.radius, w.duration, w.cause_reduction, w.speed_cut), (2000.0, 60.0, 0.25, 0.25));
}

fn lure_battle() -> Battle {
    let mut bait = mobile("bait", 0.0);
    bait.abilities.push(AbilityDef {
        kind: "LURE".into(),
        lure: Some(LureDef { radius: 4000.0, types: vec!["fighter".into()] }),
        duration: 20.0,
        recharge: 45.0,
        speed_mult: 1.0,
        shield_regen_mult: 1.0,
        weapon_delay_mult: 1.0,
        take_damage_mult: 0.5,
        cause_damage_mult: 1.0,
        ..AbilityDef::default()
    });
    let fighter = mobile("fighter", 600.0);
    let other = mobile("other", 600.0);
    let cat = Catalog::from_defs(vec![bait, fighter, other], vec![laser()]);
    let mut su = setup(cat, ("bait", 1, false), ("fighter", 2, false), BattleConfig { half_size: 6000.0, ..BattleConfig::default() });
    su.sides[1].units.push(UnitSetup { ship: su.catalog.ship_id("other").unwrap(), count: 1, tag: 7 });
    let mut b = Battle::new(su);
    let bait = ids(&b, 0)[0];
    for id in ids(&b, 1) {
        b.ships[id as usize].pos = b.ships[bait as usize].pos + Vec3::new(1500.0, 0.0, 0.0);
    }
    b
}

#[test]
fn lure_forces_listed_enemy_types_in_range_to_attack_the_caster_until_it_ends() {
    let mut b = lure_battle();
    let bait = ids(&b, 0)[0];
    let enemies = ids(&b, 1);
    for &e in &enemies {
        b.command(1, Command::Move { ships: vec![e], to: Vec3::ZERO }).unwrap();
    }
    b.command(0, Command::UseAbility { ship: bait, ability: 0 }).unwrap();
    for _ in 0..40 {
        b.step();
    }
    let lured: Vec<bool> = enemies.iter().map(|&e| b.ships[e as usize].order == Order::Attack(bait)).collect();
    let by_type: Vec<(String, bool)> = enemies.iter().zip(&lured).map(|(&e, &l)| (b.catalog().ship(b.ships[e as usize].def).name.clone(), l)).collect();
    assert!(by_type.iter().filter(|(n, _)| n == "fighter").all(|(_, l)| *l), "listed types attack the bait: {by_type:?}");
    assert!(by_type.iter().filter(|(n, _)| n == "other").all(|(_, l)| !*l), "other types ignore it: {by_type:?}");
    for _ in 0..30 * 21 {
        b.step();
    }
    assert!(enemies.iter().all(|&e| b.ships[e as usize].order != Order::Attack(bait)), "released when the lure ends");
}

#[test]
fn lure_reads_radius_and_target_types_from_the_data() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["TF-24_Ravenous"]);
    let id = cat.ship_id("TF-24_Ravenous").expect("ship");
    let l = cat.ship(id).abilities.iter().find_map(|a| a.lure.clone()).expect("LURE");
    assert_eq!(l.radius, 4000.0);
    assert!(l.types.contains(&"venator".to_string()) && l.types.len() > 10, "{:?}", l.types);
}

fn bomb_ability(bomb: BombDef) -> AbilityDef {
    AbilityDef {
        kind: "HARMONIC_BOMB".into(),
        bomb: Some(bomb),
        duration: 1.0,
        recharge: 60.0,
        speed_mult: 1.0,
        shield_regen_mult: 1.0,
        weapon_delay_mult: 1.0,
        take_damage_mult: 1.0,
        cause_damage_mult: 1.0,
        ..AbilityDef::default()
    }
}

#[test]
fn a_harmonic_heal_blast_repairs_nearby_friendly_fighters_after_the_countdown() {
    let mut carrier = mobile("carrier", 0.0);
    carrier.abilities.push(bomb_ability(BombDef { countdown: 2.0, heal_radius: 750.0, heal_amount: 200.0, heal_classes: TargetClass::Fighter.bit(), ..BombDef::default() }));
    let mut fighter = mobile("fighter", 0.0);
    fighter.class = TargetClass::Fighter;
    fighter.hull = 1000.0;
    let cat = Catalog::from_defs(vec![carrier, fighter, mobile("foe", 0.0)], vec![laser()]);
    let mut su = setup(cat, ("carrier", 1, false), ("foe", 1, false), BattleConfig { half_size: 6000.0, ..BattleConfig::default() });
    su.sides[0].units.push(UnitSetup { ship: su.catalog.ship_id("fighter").unwrap(), count: 2, tag: 7 });
    let mut b = Battle::new(su);
    let mine = ids(&b, 0);
    let (carrier, near, far) = (mine[0], mine[1], mine[2]);
    let c = b.ships[carrier as usize].pos;
    b.ships[near as usize].pos = c + Vec3::new(300.0, 0.0, 0.0);
    b.ships[far as usize].pos = c + Vec3::new(3000.0, 0.0, 0.0);
    b.ships[near as usize].hull = 300.0;
    b.ships[far as usize].hull = 300.0;
    b.command(0, Command::UseAbility { ship: carrier, ability: 0 }).unwrap();
    for _ in 0..30 {
        b.step();
    }
    assert_eq!(b.ships[near as usize].hull, 300.0, "nothing before the countdown ends");
    for _ in 0..60 {
        b.step();
    }
    assert_eq!(b.ships[near as usize].hull, 500.0, "+200 hull inside the radius");
    assert_eq!(b.ships[far as usize].hull, 300.0, "nothing outside it");
}

#[test]
fn a_seismic_bomb_hurts_enemies_in_the_blast_with_falling_damage() {
    let mut boat = mobile("boat", 0.0);
    boat.abilities.push(bomb_ability(BombDef { countdown: 1.0, blast_radius: 300.0, blast_damage: 200.0, blast_tiers: 3, ..BombDef::default() }));
    let mut foe = mobile("foe", 0.0);
    foe.hull = 1000.0;
    foe.radius = 0.0;
    let cat = Catalog::from_defs(vec![boat, foe], vec![laser()]);
    let mut b = Battle::new(setup(cat, ("boat", 1, false), ("foe", 3, false), BattleConfig { half_size: 6000.0, ..BattleConfig::default() }));
    let (boat, foes) = (ids(&b, 0)[0], ids(&b, 1));
    let c = b.ships[boat as usize].pos;
    for (f, d) in foes.iter().zip([50.0, 150.0, 250.0]) {
        b.ships[*f as usize].pos = c + Vec3::new(d, 0.0, 0.0);
    }
    b.command(0, Command::UseAbility { ship: boat, ability: 0 }).unwrap();
    for _ in 0..60 {
        b.step();
    }
    let lost: Vec<f32> = foes.iter().map(|&f| 1000.0 - b.ships[f as usize].hull).collect();
    assert!(lost[0] > lost[1] && lost[1] > lost[2] && lost[2] > 0.0, "damage falls with distance: {lost:?}");
}

#[test]
fn harmonic_bomb_values_come_from_the_spawned_object() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Ton_Falk_Carrier_Template", "Boba_Fett_Slave_I"]);
    let heal = cat.ship(cat.ship_id("Ton_Falk_Carrier_Template").expect("carrier")).abilities.iter().find_map(|a| a.bomb.clone()).expect("fighter repair");
    assert_eq!((heal.heal_radius, heal.heal_amount), (750.0, 200.0));
    let seismic = cat.ship(cat.ship_id("Boba_Fett_Slave_I").expect("slave i")).abilities.iter().find_map(|a| a.bomb.clone()).expect("seismic bomb");
    assert_eq!((seismic.countdown, seismic.blast_radius, seismic.blast_damage, seismic.blast_tiers), (5.0, 300.0, 200.0, 3));
}
