//! Tests of map arenas, targeting priority tables and tractor beam / missile shield / interdiction
//! with the real Remake data; skipped when the game is not installed.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

fn side(cat: &Catalog, name: &str, units: &[(&str, u32)], structures: &[&str], ai: bool) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(),
        structures: structures.iter().map(|n| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).collect(),
        ai,
    }
}

#[test]
fn arena_comes_from_the_space_map() {
    let Some((data, fs)) = load() else { return };
    let arena = Arena::load(&data, &fs, "_Space_Planet_Coruscant_02.ted").expect("map");
    assert!((arena.half_size() - 9250.0).abs() < 1.0);
    assert_eq!(arena.attacker_entries.len(), 3);
    assert_eq!(arena.defender_positions.len(), 3);
    assert!(arena.station.is_some());
    assert!(arena.markers.iter().any(|m| m.name == "Shipyard 1 Marker"));
    assert_eq!(arena.marker_for_structure("E_Shipyard_2").map(|m| m.name.as_str()), Some("Shipyard 2 Marker"));

    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Rebel_Star_Base_2", "E_Shipyard_1", "MC-80_Star_Cruiser"]));
    let sides = vec![
        side(&cat, "Empire", &[("Star_Destroyer", 3)], &[], true),
        side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 2)], &["Rebel_Star_Base_2", "E_Shipyard_1"], true),
    ];
    let config = BattleConfig { arena: Some(Arc::new(arena.clone())), ..BattleConfig::default() };
    let b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    assert!((b.config().half_size - 9250.0).abs() < 1.0);
    // Attackers enter at -Y, defenders sit at +Y.
    let attackers: Vec<_> = b.ships.iter().filter(|s| s.side == 0 && cat.ship(s.def).class == TargetClass::Capital).collect();
    assert!(attackers.iter().all(|s| s.pos.y < -7000.0), "{:?}", attackers.iter().map(|s| s.pos).collect::<Vec<_>>());
    let station = arena.station.unwrap().0;
    let base = b.ships.iter().find(|s| s.def == cat.ship_id("Rebel_Star_Base_2").unwrap()).unwrap();
    assert!(base.pos.dist(station) < 1.0, "star base on the station marker");
    let yard = b.ships.iter().find(|s| s.def == cat.ship_id("E_Shipyard_1").unwrap()).unwrap();
    let marker = arena.marker_for_structure("E_Shipyard_1").unwrap().pos;
    assert!(yard.pos.dist(marker) < 1.0, "shipyard on its marker");
}

#[test]
fn asteroids_block_projectiles_and_ships() {
    let obstacle = Obstacle { pos: Vec3::new(0.0, 0.0, 0.0), radius: 300.0, name: "Asteroid_Large".into() };
    let proj = ProjDef { name: "laser".into(), damage: 100.0, speed: 2000.0, max_distance: 9000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() };
    let gun = HpDef {
        name: "g".into(),
        kind: HpKind::Weapon,
        projectile: Some(0),
        range: 9000.0,
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
    };
    let shooter = ShipDef { name: "shooter".into(), is_static: true, hull: 1.0e6, radius: 100.0, hardpoints: vec![gun], class: TargetClass::Frigate, ..ShipDef::default() };
    let target = ShipDef { name: "target".into(), is_static: true, hull: 1000.0, radius: 100.0, class: TargetClass::Frigate, ..ShipDef::default() };
    let cat = Arc::new(Catalog::from_defs(vec![shooter, target], vec![proj]));
    let mk = |obstacles: Vec<Obstacle>| {
        let arena = Arena { min: [-6000.0; 2], max: [6000.0; 2], attacker_entries: vec![(Vec3::new(-2000.0, 0.0, 0.0), 0.0)], defender_positions: vec![(Vec3::new(2000.0, 0.0, 0.0), std::f32::consts::PI)], obstacles, ..Arena::default() };
        let sides = vec![
            SideSetup { name: "a".into(), units: vec![UnitSetup { ship: 0, count: 1, tag: 0 }], structures: vec![], ai: true },
            SideSetup { name: "b".into(), units: vec![UnitSetup { ship: 1, count: 1, tag: 0 }], structures: vec![], ai: true },
        ];
        Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { arena: Some(Arc::new(arena)), ..BattleConfig::default() } })
    };
    let hit_target = |mut b: Battle| {
        for _ in 0..600 {
            b.step();
        }
        b.ships[1].hull < 1000.0
    };
    assert!(hit_target(mk(vec![])), "clear line of fire");
    assert!(!hit_target(mk(vec![obstacle])), "an asteroid between the ships absorbs every shot");
}

#[test]
fn priority_tables_are_parsed_and_used() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Star_Destroyer", "TIE_Fighter", "TIE_Bomber"]);
    assert!(cat.priority_sets.len() > 20, "{} sets", cat.priority_sets.len());
    let set = |n: &str| cat.priority_sets.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("{n}"));
    let fighter_move = set("fighter_attack_move");
    assert!(fighter_move.excluded & TargetClass::Capital.bit() != 0, "fighters on attack-move ignore capitals");
    assert!(fighter_move.weight(TargetClass::Bomber, false) < fighter_move.weight(TargetClass::Fighter, false));
    assert_eq!(fighter_move.hardpoints.first(), Some(&HpKind::ShieldGenerator));
    let capital = set("capital");
    assert!(capital.weight(TargetClass::Capital, false) < capital.weight(TargetClass::Frigate, false));
    assert!(capital.weight(TargetClass::Structure, true) <= 1.0, "Golan platforms come first");
    let isd = cat.ship(cat.ship_id("Star_Destroyer").unwrap());
    assert_eq!(cat.priority_sets[isd.priority.unwrap() as usize].name, "capital");
    assert!(isd.priority_attack_move.is_some());
    let tie = cat.ship(cat.ship_id("TIE_Fighter").unwrap());
    assert!(tie.priority.is_some());
}

#[test]
fn tractor_beam_slows_its_target() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "CR-90_Corvette"]));
    let isd_def = cat.ship_id("Star_Destroyer").unwrap();
    let idx = cat.ship(isd_def).abilities.iter().position(|a| a.tractor.is_some()).expect("tractor ability");
    let tractor = cat.ship(isd_def).abilities[idx].tractor.clone().unwrap();
    assert!((tractor.slow - 0.75).abs() < 0.01 && tractor.range >= 2500.0, "{tractor:?}");
    let sides = vec![side(&cat, "Empire", &[("Star_Destroyer", 1)], &[], false), side(&cat, "Rebel", &[("CR-90_Corvette", 1)], &[], false)];
    let config = BattleConfig { half_size: 9000.0, fog_of_war: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let corvette = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.command(1, Command::Move { ships: vec![corvette], to: Vec3::new(-1000.0, 0.0, 0.0) }).unwrap();
    let isd = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    // The ISD holds at its weapon stand-off (beyond the beam), so the test closes in by hand.
    b.command(0, Command::Move { ships: vec![isd], to: Vec3::new(-1000.0, 0.0, 0.0) }).unwrap();
    // Out of range: the beam cannot start.
    assert_eq!(b.command(0, Command::UseAbility { ship: isd, ability: idx as u16 }), Err(CommandError::AbilityUnavailable));
    for _ in 0..30 * 120 {
        b.step();
        if b.ships[isd as usize].pos.dist(b.ships[corvette as usize].pos) < tractor.range * 0.9 {
            break;
        }
    }
    b.command(0, Command::UseAbility { ship: isd, ability: idx as u16 }).expect("beam starts in range");
    b.step();
    b.step();
    assert!((b.ships[corvette as usize].tractor_cut - 0.75).abs() < 0.01);
    assert_eq!(b.ships[isd as usize].abilities[idx].target, corvette);
}

#[test]
fn missile_shield_and_interdiction() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Dominator_Star_Destroyer", "Arquitens_E"]));
    let dom = cat.ship_id("Dominator_Star_Destroyer").unwrap();
    let interdict = cat.ship(dom).abilities.iter().position(|a| a.interdict).expect("interdiction ability");
    let idx = cat.ship(dom).abilities.iter().position(|a| a.missile_radius > 0.0).expect("missile shield");
    let sides = vec![side(&cat, "Rebel", &[("Arquitens_E", 6)], &[], true), side(&cat, "Empire", &[("Dominator_Star_Destroyer", 1)], &[], false)];
    let config = BattleConfig { half_size: 5000.0, retreat_countdown: 0.0, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let dom_ship = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.command(1, Command::UseAbility { ship: dom_ship, ability: interdict as u16 }).unwrap();
    assert_eq!(b.command(0, Command::Retreat), Err(CommandError::RetreatBlocked), "an active interdiction field stops retreats");
    b.command(1, Command::UseAbility { ship: dom_ship, ability: idx as u16 }).unwrap();
    let mut intercepted = 0;
    for _ in 0..30 * 40 {
        intercepted += b.step().iter().filter(|e| matches!(e, Event::ProjectileIntercepted { .. })).count();
    }
    assert!(intercepted > 0, "missiles fired at the Dominator are shot down");
}

#[test]
fn laser_defense_becomes_a_missile_defence_ability() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Demagolka"]);
    let id = cat.ship_id("Demagolka").expect("a ship with LASER_DEFENSE");
    let a = cat.ship(id).abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("LASER_DEFENSE")).expect("the ability");
    assert!((a.missile_radius - 750.0).abs() < 1.0 && (a.duration - 30.0).abs() < 1.0 && !a.toggle && !a.disable_movement, "{a:?}");
}

#[test]
fn combat_bonus_and_redirect_abilities_are_read_from_the_data() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Gorgon_Star_Destroyer_II", "Viscount_Star_Defender_P1", "Dragon"]);
    let tie = cat.ship(cat.ship_id("Gorgon_Star_Destroyer_II").expect("gorgon"));
    assert!(tie.bonuses.iter().any(|b| (b.damage - 0.25).abs() < 1e-4 && (b.shield + 0.4).abs() < 1e-4 && b.stacking == 6), "{:?}", tie.bonuses);
    let viscount = cat.ship(cat.ship_id("Viscount_Star_Defender_P1").expect("viscount"));
    let dragon = cat.ship(cat.ship_id("Dragon").expect("dragon"));
    assert!(!viscount.redirects.is_empty() || !dragon.redirects.is_empty(), "an enabled blaster redirect is parsed");
}

#[test]
fn super_laser_storm_rocket_and_jamming_abilities_are_read_from_the_data() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Death_Star_II", "Eclipse_Star_Dreadnought_P4", "MissileBoat"]);
    let ds = cat.ship(cat.ship_id("Death_Star_II").expect("death star"));
    let laser = ds.abilities.iter().find_map(|a| a.super_laser.as_ref()).expect("super laser");
    assert!(laser.delay > 1.0 && laser.delay < 3.0 && laser.classes != 0, "{laser:?}");
    let eclipse = cat.ship(cat.ship_id("Eclipse_Star_Dreadnought_P4").expect("eclipse"));
    let storm = eclipse.abilities.iter().find_map(|a| a.zone.as_ref()).unwrap_or_else(|| panic!("lightning storm: {:?}", eclipse.abilities.iter().map(|a| a.kind.clone()).collect::<Vec<_>>()));
    assert!((storm.enemy_dps - 250.0).abs() < 1.0 && (storm.radius - 5000.0).abs() < 1.0, "{storm:?}");
    let boat = cat.ship(cat.ship_id("MissileBoat").expect("missile boat"));
    assert!(boat.abilities.iter().any(|a| a.rocket));
}

#[test]
fn projectile_combat_mods_are_read_from_the_data() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["T-Wing_Squadron"]);
    let torpedo = cat.projectile_id("Proj_Proton_Torpedo").map(|p| &cat.projectiles[p as usize]).expect("proton torpedo");
    let m = torpedo.combat_mod.as_ref().expect("combat mod");
    assert_eq!(m.kind, ModKind::Health);
    assert!((m.value + 0.25).abs() < 1e-4 && (m.duration - 14.0).abs() < 1e-4 && m.classes != 0, "{m:?}");
}

#[test]
fn autocast_support_and_default_come_from_the_data() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Gorgon_Star_Destroyer_II", "Star_Destroyer"]);
    let gorgon = cat.ship(cat.ship_id("Gorgon_Star_Destroyer_II").unwrap());
    let tractor = gorgon.abilities.iter().find(|a| a.tractor.is_some()).expect("tractor beam");
    assert!(tractor.supports_autofire && !tractor.autocast_default, "the tractor beam can be set to automatic and starts off");
    let isd = cat.ship(cat.ship_id("Star_Destroyer").unwrap());
    assert!(isd.abilities.iter().any(|a| !a.supports_autofire), "abilities without `Supports_Autofire` cannot");
}

#[test]
fn carriers_with_only_higher_tech_entries_still_launch() {
    let Some((data, fs)) = load() else { return };
    // `Secutor_Empire` lists its fighters under `Starting_Spawned_Units_Tech_1` only; they used to be dropped (only Tech_0 was read).
    let cat = Catalog::build(&data, Some(&fs), &["Secutor_Empire"]);
    let d = cat.ship(cat.ship_id("Secutor_Empire").unwrap());
    assert!(!d.spawned_by_tech.is_empty() && d.spawned_by_tech.iter().all(|t| t.0 >= 1), "{:?}", d.spawned_by_tech.iter().map(|t| t.0).collect::<Vec<_>>());
    assert!(!d.carried.is_empty() && !d.reserve.is_empty(), "the lowest level the unit has is used when the owner is below it");
}

#[test]
fn the_star_destroyer_at_the_start_tech_brings_exactly_its_tech_0_wing() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Venator_Empire"]);
    let isd = cat.ship(cat.ship_id("Star_Destroyer").unwrap());
    assert_eq!(isd.spawned_by_tech.iter().map(|t| t.0).collect::<Vec<_>>(), vec![0], "only Tech_0 entries");
    assert!(!isd.carried.is_empty());
    // The Venator of the Empire refits: Republic wings at level index 1, TIEs at index 2 (it needs tech level 2, so it never has index 0).
    let ven = cat.ship(cat.ship_id("Venator_Empire").unwrap());
    assert_eq!(ven.spawned_by_tech.iter().map(|t| t.0).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn corrupt_systems_is_read_from_the_corellian_destroyer() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Corellian_Destroyer"]);
    let chg = cat.ship(cat.ship_id("Corellian_Destroyer").expect("chg"));
    let c = chg.abilities.iter().find_map(|a| a.corrupt.as_ref()).expect("corrupt systems");
    assert!((c.drain - 100.0).abs() < 1e-3 && (c.duration - 30.0).abs() < 1e-3 && (c.max_range - 1000.0).abs() < 1e-3, "{c:?}");
}

/// True when `hp` could shoot an enemy of `class` at `at` from a ship at the origin facing +X. Mirrors `can_fire` for a capital shooter.
fn gun_bears(cat: &Catalog, hp: &HpDef, class: TargetClass, mask: u32, at: Vec3, radius: f32) -> bool {
    let Some(pid) = hp.projectile else { return false };
    if hp.kind != HpKind::Weapon || hp.off_in_normal {
        return false;
    }
    let proj = &cat.projectiles[pid as usize];
    let muzzle = hp.offset;
    let dist = muzzle.dist(at);
    let reach = (dist - radius * 0.6).max(0.0);
    if reach > hp.range || reach < hp.min_range {
        return false;
    }
    if hp.allowed != 0 && hp.allowed & class.bit() == 0 {
        return false;
    }
    if hp.forbid_exact.iter().any(|&m| category_contains(mask, m)) {
        return false;
    }
    if matches!(class, TargetClass::Fighter | TargetClass::Bomber) && (proj.damage >= 150.0 || !proj.hull_damage) {
        return false;
    }
    if hp.arc_yaw >= std::f32::consts::PI - 0.01 && hp.arc_pitch >= std::f32::consts::FRAC_PI_2 - 0.01 {
        return true;
    }
    let to = at - muzzle;
    if to.len() < 1.0 {
        return true;
    }
    let d = to.normalized();
    let slack = (radius / dist.max(1.0)).atan();
    let yaw_off = d.dot(hp.left).atan2(d.dot(hp.forward)).abs();
    let pitch_off = d.dot(hp.normal).clamp(-1.0, 1.0).asin().abs();
    yaw_off <= hp.arc_yaw + slack && pitch_off <= hp.arc_pitch + slack
}

#[test]
fn an_idle_providence_fires_every_gun_that_has_a_target_in_arc() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Providence_Destroyer", "CR-90_Corvette", "Star_Destroyer", "TIE_Fighter"]));
    let mut b = Battle::new(BattleSetup {
        catalog: cat.clone(),
        sides: vec![
            side(&cat, "Neutral", &[("Providence_Destroyer", 1)], &[], false),
            side(&cat, "Empire", &[("CR-90_Corvette", 12), ("Star_Destroyer", 1), ("TIE_Fighter", 8)], &[], false),
        ],
        config: BattleConfig { fog_of_war: false, ..BattleConfig::default() },
    });
    let prov = b.ships.iter().find(|s| cat.ship(s.def).name == "Providence_Destroyer").unwrap().id;
    let origin = b.ships[prov as usize].pos;
    let enemies: Vec<u32> = b.ships.iter().filter(|s| s.id != prov).map(|s| s.id).collect();
    let place = |i: usize| {
        let ang = i as f32 * std::f32::consts::TAU / enemies.len() as f32;
        origin + Vec3::new(ang.cos(), ang.sin(), 0.0) * 1600.0
    };
    let mut fired = std::collections::BTreeSet::new();
    for _ in 0..30 * 60 {
        b.ships[prov as usize].pos = origin;
        b.ships[prov as usize].yaw = 0.0;
        b.ships[prov as usize].speed = 0.0;
        for (i, &id) in enemies.iter().enumerate() {
            b.ships[id as usize].pos = place(i);
            b.ships[id as usize].speed = 0.0;
        }
        for e in b.step() {
            if let Event::ProjectileFired { ship, hardpoint: Some(hp), .. } = e
                && *ship == prov
            {
                fired.insert(*hp);
            }
        }
    }
    let def = cat.ship(b.ships[prov as usize].def);
    let target_only = def.hardpoints.iter().filter(|h| h.target_only && h.kind == HpKind::Weapon && h.projectile.is_some()).count();
    assert_eq!(target_only, 14, "Providence hardpoints with Allow_Opportunity_Fire_When_Targeting false");
    let mut silent = Vec::new();
    let mut quads = 0;
    for (i, hp) in def.hardpoints.iter().enumerate() {
        let covered = enemies.iter().any(|&id| {
            let s = &b.ships[id as usize];
            let d = cat.ship(s.def);
            gun_bears(&cat, hp, d.class, d.category_mask, place(enemies.iter().position(|&e| e == id).unwrap()) - origin, d.radius)
        });
        if covered && hp.target_only {
            quads += 1;
        }
        if covered && !fired.contains(&(i as u16)) {
            silent.push(hp.name.clone());
        }
    }
    assert_eq!(quads, 14, "each targeting-only quad had a ship in range and arc");
    assert!(silent.is_empty(), "guns with a target in range and arc that stayed silent: {silent:?}");
}
