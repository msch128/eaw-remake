//! Big ships go around rocks, stations and each other; fighters keep out of hulls. Hand-made definitions plus the real Bilbringi and Hoth maps.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn ship(name: &str, class: TargetClass, speed: f32, radius: f32, is_static: bool) -> ShipDef {
    ShipDef {
        name: name.into(),
        class,
        is_static,
        max_speed: speed,
        accel: speed,
        turn_rate: 1.0,
        hull: 1.0e6,
        radius,
        population: 1,
        ..ShipDef::default()
    }
}

fn side(name: &str, ship: &str, cat: &Catalog, count: u32) -> SideSetup {
    SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(ship).unwrap(), count, tag: 1 }], structures: vec![], ai: false }
}

fn arena(rocks: &[(f32, f32, f32)]) -> Arc<Arena> {
    Arc::new(Arena {
        min: [-6000.0, -6000.0],
        max: [6000.0, 6000.0],
        obstacles: rocks.iter().map(|&(x, y, r)| Obstacle { pos: Vec3::new(x, y, 0.0), radius: r, name: "rock".into() }).collect(),
        ..Arena::default()
    })
}

fn ids(b: &Battle, side: u8) -> Vec<u32> {
    b.ships.iter().filter(|s| s.side == side).map(|s| s.id).collect()
}

#[test]
fn a_capital_ship_flies_around_a_rock_on_its_way() {
    let cap = ship("cap", TargetClass::Capital, 150.0, 200.0, false);
    let dummy = ship("dummy", TargetClass::Frigate, 0.0, 50.0, true);
    let cat = Arc::new(Catalog::from_defs(vec![cap, dummy], vec![]));
    let rock = (-1500.0f32, -750.0f32, 500.0f32);
    let config = BattleConfig { arena: Some(arena(&[rock])), record_events: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("a", "cap", &cat, 1), side("b", "dummy", &cat, 1)], config });
    let me = ids(&b, 0)[0];
    let dest = Vec3::new(900.0, -1500.0, 0.0);
    b.command(0, Command::Move { ships: vec![me], to: dest }).unwrap();
    let (mut closest, mut waypoints) = (f32::MAX, 0);
    for _ in 0..30 * 90 {
        b.step();
        let s = &b.ships[me as usize];
        closest = closest.min(s.pos.dist(Vec3::new(rock.0, rock.1, 0.0)) - rock.2);
        waypoints = waypoints.max(s.path.len());
    }
    let s = &b.ships[me as usize];
    assert!(closest > 0.0, "the ship never touches the rock: closest edge distance {closest}");
    assert!(waypoints >= 2, "the planned path has a waypoint around the rock: {waypoints}");
    assert!(s.pos.dist(dest) < 400.0, "and it still arrives: {:?}", s.pos);
}

#[test]
fn two_capitals_meeting_head_on_pass_without_overlapping() {
    let cap = ship("cap", TargetClass::Capital, 150.0, 300.0, false);
    let cat = Arc::new(Catalog::from_defs(vec![cap], vec![]));
    let config = BattleConfig { arena: Some(arena(&[(0.0, 5800.0, 100.0)])), record_events: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("a", "cap", &cat, 1), side("b", "cap", &cat, 1)], config });
    let (a, c) = (ids(&b, 0)[0], ids(&b, 1)[0]);
    let (pa, pc) = (b.ships[a as usize].pos, b.ships[c as usize].pos);
    b.command(0, Command::Move { ships: vec![a], to: pc }).unwrap();
    b.command(1, Command::Move { ships: vec![c], to: pa }).unwrap();
    let mut closest = f32::MAX;
    for _ in 0..30 * 90 {
        b.step();
        closest = closest.min(b.ships[a as usize].pos.dist(b.ships[c as usize].pos));
    }
    // Snapshot radius of a capital is 0.75 of the definition radius; the keep-out is 85% of the two radii together.
    let keep_out = 2.0 * 300.0 * 0.75 * 0.85;
    assert!(closest >= keep_out * 0.95, "hulls stay apart: {closest} vs {keep_out}");
}

#[test]
fn a_fighter_ordered_through_a_capital_does_not_enter_the_hull() {
    let fighter = ship("fighter", TargetClass::Fighter, 300.0, 10.0, false);
    let cap = ship("cap", TargetClass::Capital, 0.0, 600.0, true);
    let cat = Arc::new(Catalog::from_defs(vec![fighter, cap], vec![]));
    let config = BattleConfig { arena: Some(arena(&[])), record_events: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("a", "fighter", &cat, 1), side("b", "cap", &cat, 1)], config });
    let (f, c) = (ids(&b, 0)[0], ids(&b, 1)[0]);
    let hull = b.ships[c as usize].pos;
    b.command(0, Command::Move { ships: vec![f], to: hull }).unwrap();
    let mut closest = f32::MAX;
    for _ in 0..30 * 40 {
        b.step();
        closest = closest.min(b.ships[f as usize].pos.dist(hull));
    }
    assert!(closest >= 600.0 * 0.75 * 0.45 - 1.0, "the fighter stays outside the hull core: {closest}");
}

fn real_data() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

/// Runs a six against six battle on `map` and returns (closest hull approach as a share of the keep-out, ship-ticks inside rocks, state hash).
fn fleet_battle(data: &alamo_data::GameData, fs: &GameFs, map: &str, seed: u64) -> (f32, u32, u64) {
    let arena = Arc::new(Arena::load(data, fs, map).expect("map"));
    let cat = Arc::new(Catalog::build(data, Some(fs), &["Star_Destroyer", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "Victory_Destroyer"]));
    let mk = |name: &str, units: &[(&str, u32)]| SideSetup { name: name.into(), units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }).collect(), structures: vec![], ai: true };
    let sides = vec![mk("Empire", &[("Star_Destroyer", 3), ("Victory_Destroyer", 3)]), mk("Rebel", &[("MC-80_Star_Cruiser", 3), ("Nebulon_B_Frigate", 3)])];
    let config = BattleConfig { arena: Some(arena.clone()), seed, record_events: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let (mut worst, mut in_rock) = (f32::MAX, 0u32);
    for _ in 0..30 * 240 {
        b.step();
        let radius = |s: &Ship| {
            let d = cat.ship(s.def);
            if d.class as u8 >= TargetClass::Frigate as u8 { d.radius * 0.75 } else { d.radius }
        };
        let big: Vec<&Ship> = b.ships.iter().filter(|s| s.alive && !s.retreated && !cat.ship(s.def).is_static && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).collect();
        for (i, a) in big.iter().enumerate() {
            for o in big[i + 1..].iter().filter(|o| a.carrier != o.id && o.carrier != a.id) {
                worst = worst.min(a.pos.dist(o.pos) / ((radius(a) + radius(o)) * 0.85));
            }
            in_rock += arena.obstacles.iter().filter(|r| a.pos.dist(r.pos) < r.radius * 0.9).count() as u32;
        }
        if b.is_over() {
            break;
        }
    }
    (worst, in_rock, b.state_hash())
}

#[test]
fn real_maps_bilbringi_and_hoth_have_no_hull_overlaps_and_no_ships_in_rocks() {
    let Some((data, fs)) = real_data() else { return };
    for map in ["_SPACE_PLANET_VERGESSOASTEROIDS_02.ted", "_Space_Planet_Hoth_01.ted"] {
        let (worst, in_rock, hash) = fleet_battle(&data, &fs, map, 1);
        println!("{map}: closest hull approach {:.0}% of the keep-out, {in_rock} ship-ticks inside rocks", worst * 100.0);
        assert!(worst >= 0.9, "{map}: hulls overlap ({worst})");
        assert_eq!(in_rock, 0, "{map}: ships inside rocks");
        // Deterministic.
        assert_eq!(hash, fleet_battle(&data, &fs, map, 1).2, "{map}: same seed, same battle");
    }
}
