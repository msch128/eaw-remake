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

#[test]
fn escort_corvette_steers_clear_of_capital_ship_with_buffer() {
    let corvette = ship("corvette", TargetClass::Corvette, 200.0, 60.0, false);
    let cap = ship("cap", TargetClass::Capital, 0.0, 400.0, true);
    let cat = Arc::new(Catalog::from_defs(vec![corvette, cap], vec![]));
    let config = BattleConfig { arena: Some(arena(&[])), record_events: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("a", "corvette", &cat, 1), side("b", "cap", &cat, 1)], config });
    let (corv_id, cap_id) = (ids(&b, 0)[0], ids(&b, 1)[0]);

    b.ships[cap_id as usize].pos = Vec3::new(0.0, 0.0, 0.0);
    b.ships[corv_id as usize].pos = Vec3::new(-1200.0, 50.0, 0.0);
    b.command(0, Command::Move { ships: vec![corv_id], to: Vec3::new(1200.0, 50.0, 0.0) }).unwrap();

    let mut closest_dist = f32::MAX;
    for _ in 0..30 * 40 {
        b.step();
        let d = b.ships[corv_id as usize].pos.dist(b.ships[cap_id as usize].pos);
        closest_dist = closest_dist.min(d);
    }
    let cap_hull_radius = 400.0 * 0.75;
    assert!(closest_dist > cap_hull_radius, "corvette glides around capital hull: {closest_dist} vs hull radius {cap_hull_radius}");
    assert!(b.ships[corv_id as usize].pos.x > 800.0, "corvette reaches the other side: pos {:?}", b.ships[corv_id as usize].pos);
}

#[test]
fn ships_and_fighters_keep_out_of_a_long_hull_along_its_whole_length() {
    // An Executor-like hull, 6000 long and 800 wide, whose collision radius is clamped to 700: a point 2000 ahead of its centre lies
    // outside every circle of that radius but deep inside the hull.
    let mut long = ship("long", TargetClass::Capital, 0.0, 700.0, false);
    long.hull_shape = Some(nav::Hull { front: 3000.0, back: 3000.0, half_width: 400.0 });
    let frigate = ship("frigate", TargetClass::Frigate, 150.0, 100.0, false);
    let mut fighter = ship("fighter", TargetClass::Fighter, 300.0, 10.0, false);
    fighter.turn_rate = 3.0;
    let cat = Arc::new(Catalog::from_defs(vec![frigate, long, fighter], vec![]));
    let config = BattleConfig { arena: Some(arena(&[])), record_events: false, ..BattleConfig::default() };
    let sides = vec![
        SideSetup { name: "a".into(), units: vec![UnitSetup { ship: cat.ship_id("frigate").unwrap(), count: 1, tag: 1 }, UnitSetup { ship: cat.ship_id("fighter").unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false },
        side("b", "long", &cat, 1),
    ];
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let mine = ids(&b, 0);
    let (frig, ftr) = (mine[0], mine[1]);
    let big = ids(&b, 1)[0];
    b.ships[big as usize].pos = Vec3::ZERO;
    b.ships[big as usize].yaw = 0.0;
    b.ships[frig as usize].pos = Vec3::new(2000.0, -2500.0, 0.0);
    b.ships[ftr as usize].pos = Vec3::new(-2000.0, 2500.0, 0.0);
    let onto = Vec3::new(2000.0, 0.0, 0.0);
    b.command(0, Command::Move { ships: vec![frig], to: onto }).unwrap();
    b.command(0, Command::Move { ships: vec![ftr], to: Vec3::new(-2000.0, 0.0, 0.0) }).unwrap();
    let (long_hull, frig_hull) = (cat.ship(b.ships[big as usize].def).footprint(), cat.ship(b.ships[frig as usize].def).footprint());
    let (mut deepest, mut fighter_closest) = (0.0f32, f32::MAX);
    for t in 0..30 * 40 {
        b.step();
        let (f, o, x) = (&b.ships[frig as usize], &b.ships[big as usize], &b.ships[ftr as usize]);
        if let Some((depth, _)) = nav::hull_overlap((f.pos, f.yaw, frig_hull), (o.pos, o.yaw, long_hull), 0.0, 1.0) {
            deepest = deepest.max(depth);
        }
        if t > 30 * 5 {
            fighter_closest = fighter_closest.min(nav::core_distance(x.pos, long_hull, o.pos, o.yaw).0);
        }
    }
    let f = &b.ships[frig as usize];
    println!("frigate deepest in the long hull {deepest:.0}, ends at {:?}; fighter closest to the hull's axis {fighter_closest:.0}", f.pos);
    assert!(deepest < 10.0, "the frigate stays out of the long hull: {deepest:.0} units deep");
    assert!(f.pos.dist(onto) < 1200.0, "and waits at the hull's edge: {:?}", f.pos);
    assert!(fighter_closest > 400.0, "fighters stay outside the long hull, not just off its axis: {fighter_closest:.0}");
}

/// One craft inside the volume of a ship's model hull: how deep (the way out, plan or height, whichever is shorter).
#[derive(Debug, Clone, Copy)]
struct Inside {
    craft: u32,
    depth: f32,
}

/// Finds the mobile units of any class that are inside the volume of another unit's model hull (its plan-view capsule within the full half
/// width and its model's height range, no margin): the centre of the unit, which is where it is drawn. Not counted: a craft while it flies out of
/// its hangar and in the tick the flyout ends (the one place a craft is inside a hull by design), the ship a unit carries, and hulls without a
/// model shape.
#[derive(Default)]
struct HullWatch {
    flying_out: Vec<bool>,
}

impl HullWatch {
    fn inside(&mut self, b: &Battle, cat: &Catalog) -> Vec<Inside> {
        let small = |s: &Ship| matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
        self.flying_out.resize(b.ships.len(), false);
        let mut out = Vec::new();
        for s in b.ships.iter().filter(|s| s.alive && !s.retreated && !cat.ship(s.def).is_static) {
            let flying_out = s.flyout_exit != Vec3::ZERO || s.flyout_left > 0.0;
            let was = std::mem::replace(&mut self.flying_out[s.id as usize], flying_out);
            if flying_out || was {
                continue;
            }
            for o in b.ships.iter().filter(|o| o.alive && !o.retreated && !small(o) && o.id != s.id && o.carrier != s.id) {
                let odef = cat.ship(o.def);
                let Some(hull) = odef.hull_shape else { continue };
                let (cd, _) = nav::core_distance(s.pos, hull, o.pos, o.yaw);
                let plan = hull.half_width - cd;
                let up = odef.hull_z.map_or(f32::MAX, |(lo, hi)| (s.pos.z - (o.pos.z + lo)).min(o.pos.z + hi - s.pos.z));
                if plan > 0.0 && up > 0.0 {
                    out.push(Inside { craft: s.id, depth: plan.min(up) });
                }
            }
        }
        out
    }
}

#[test]
fn fighters_attack_a_huge_hull_from_outside_and_still_shoot_at_it() {
    // An Executor-like hull, 9000 long and 3000 wide and 690 high, whose collision radius is clamped to 700: the middle lies 1500 units from the
    // skin at its side, where nothing may fly. Attack runs used to end inside the hull (the aim is the middle), where the craft fired from.
    let mut long = ship("long", TargetClass::Capital, 0.0, 700.0, true);
    long.hull_shape = Some(nav::Hull { front: 4500.0, back: 4500.0, half_width: 1500.0 });
    long.hull_z = Some((-230.0, 460.0));
    let mut fighter = ship("fighter", TargetClass::Fighter, 120.0, 10.0, false);
    fighter.turn_rate = 2.0;
    fighter.hardpoints = vec![HpDef {
        name: "laser".into(),
        kind: HpKind::Weapon,
        health: 10.0,
        forward: Vec3::new(1.0, 0.0, 0.0),
        projectile: Some(0),
        range: 500.0,
        half_yaw: std::f32::consts::PI,
        half_pitch: std::f32::consts::FRAC_PI_2,
        arc_yaw: std::f32::consts::PI,
        arc_pitch: std::f32::consts::FRAC_PI_2,
        recharge_min: 0.5,
        recharge_max: 0.5,
        pulses: 1,
        ..HpDef::default()
    }];
    let laser = ProjDef { name: "laser".into(), category: "laser".into(), damage: 10.0, speed: 2000.0, max_distance: 3000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() };
    let cat = Arc::new(Catalog::from_defs(vec![long, fighter], vec![laser]));
    let config = BattleConfig { arena: None, half_size: 12_000.0, record_events: false, fog_of_war: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("a", "fighter", &cat, 6), side("b", "long", &cat, 1)], config });
    let big = ids(&b, 1)[0];
    b.ships[big as usize].pos = Vec3::ZERO;
    b.ships[big as usize].yaw = 0.0;
    let fighters = ids(&b, 0);
    // They start 600 units off the skin at the Executor's side.
    for (i, &f) in fighters.iter().enumerate() {
        let d = &mut b.ships[f as usize];
        d.pos = Vec3::new(-300.0 + i as f32 * 120.0, -2200.0, 0.0);
        d.yaw = std::f32::consts::FRAC_PI_2;
    }
    b.command(0, Command::Attack { ships: fighters.clone(), target: big }).unwrap();
    let vol = nav::HullVolume { hull: cat.ship(b.ships[big as usize].def).hull_shape.unwrap(), z: cat.ship(b.ships[big as usize].def).hull_z };
    let hull_before = b.ships[big as usize].hull;
    let (mut watch, mut inside, mut closest) = (HullWatch::default(), 0u32, f32::MAX);
    // A pass ends in the peel-off (`Ship::phase` starts).
    let mut passes = vec![0u32; fighters.len()];
    let mut peeling = vec![false; fighters.len()];
    for _ in 0..30 * 60 {
        b.step();
        inside += watch.inside(&b, &cat).len() as u32;
        for (k, &f) in fighters.iter().enumerate() {
            let s = &b.ships[f as usize];
            closest = closest.min(vol.distance_to(s.pos, b.ships[big as usize].pos, b.ships[big as usize].yaw));
            if s.phase > 0.0 && !peeling[k] {
                passes[k] += 1;
            }
            peeling[k] = s.phase > 0.0;
        }
    }
    let damage = hull_before - b.ships[big as usize].hull;
    println!("fighters at the huge hull: {inside} craft-ticks inside, closest {closest:.0} off the skin, passes {passes:?}, hull damage {damage:.0}");
    assert_eq!(inside, 0, "no fighter is ever inside the hull");
    assert!(closest < 200.0, "but they run in on it: {closest:.0} units off the skin at the closest");
    assert!(passes.iter().all(|&p| p >= 3), "and pull out and run in again: {passes:?}");
    assert!(damage > 500.0, "and their guns reach it from there: {damage:.0} hull damage in a minute");
}

/// Rebel squadrons attack the Empire's Executor (it holds its place; harmless or not) for `seconds`: the craft-ticks inside any hull, the
/// deepest, and the damage the Executor took.
fn squadrons_attack_an_executor(data: &alamo_data::GameData, fs: &GameFs, seed: u64, seconds: f32, harmless: bool) -> (u32, f32, f32) {
    let names = ["Piett_Executor_P1", "X-Wing_Squadron", "Y-Wing_Squadron", "A-Wing_Squadron", "B-Wing_Squadron"];
    let cat = Arc::new(Catalog::build(data, Some(fs), &names));
    let mk = |name: &str, units: &[(&str, u32)], ai: bool| SideSetup { name: name.into(), units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }).collect(), structures: vec![], ai };
    let sides = vec![mk("Empire", &[("Piett_Executor_P1", 1)], false), mk("Rebel", &[("X-Wing_Squadron", 3), ("Y-Wing_Squadron", 3), ("A-Wing_Squadron", 2), ("B-Wing_Squadron", 2)], true)];
    // A harmless Executor leaves every craft alive for the whole minute (the worst case for the count); otherwise it shoots them down as it does in play.
    let side_mults = if harmless { vec![SideMult { damage: 0.0, health: 1.0, shield: 1.0 }, SideMult { damage: 1.0, health: 1.0, shield: 1.0 }] } else { Vec::new() };
    let config = BattleConfig { arena: None, half_size: 12_000.0, seed, record_events: false, side_mults, retreat_allowed: vec![false, false], fog_of_war: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let exec = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    b.ships[exec as usize].pos = Vec3::ZERO;
    b.ships[exec as usize].yaw = 0.0;
    // The squadrons start 4500 units off the Executor's side.
    let crafts: Vec<u32> = b.ships.iter().filter(|s| s.side == 1).map(|s| s.id).collect();
    for (i, &c) in crafts.iter().enumerate() {
        let d = &mut b.ships[c as usize];
        d.pos = Vec3::new(-1500.0 + (i % 8) as f32 * 150.0, -4500.0 - (i / 8) as f32 * 150.0, 0.0);
        d.yaw = std::f32::consts::FRAC_PI_2;
    }
    let start = (b.ships[exec as usize].shield, b.ships[exec as usize].hps.iter().map(|h| h.health).sum::<f32>());
    let (mut watch, mut samples, mut deepest) = (HullWatch::default(), 0u32, 0.0f32);
    for _ in 0..(30.0 * seconds) as usize {
        b.step();
        for i in watch.inside(&b, &cat) {
            samples += 1;
            deepest = deepest.max(i.depth);
        }
    }
    let e = &b.ships[exec as usize];
    (samples, deepest, (start.0 - e.shield) + (start.1 - e.hps.iter().map(|h| h.health).sum::<f32>()))
}

#[test]
fn squadrons_attacking_an_executor_never_fly_into_it_and_still_hurt_it() {
    // The Executor of the data (4544 long in front of its middle, 1548 half wide, 683 high). On main (87ab720) the craft of that battle (the 60
    // attackers and the Executor's own squadrons) spent 293905-330214 craft-ticks inside it in a minute (seeds 1-3, up to 228 units deep); it
    // took 6638 / 6530 / 6793 damage with it harmless and 2986 / 2805 / 2859 with it shooting back, now 8676-8838 and 6629-7006.
    let Some((data, fs)) = real_data() else { return };
    let runs: Vec<(bool, u64, (u32, f32, f32))> = std::thread::scope(|sc| {
        let jobs: Vec<_> = [true, false]
            .into_iter()
            .flat_map(|harmless| (1..=2).map(move |seed| (harmless, seed)))
            .map(|(harmless, seed)| {
                let (data, fs) = (&data, &fs);
                sc.spawn(move || (harmless, seed, squadrons_attack_an_executor(data, fs, seed, 60.0, harmless)))
            })
            .collect();
        jobs.into_iter().map(|j| j.join().expect("Executor battle")).collect()
    });
    for (harmless, seed, (samples, deepest, taken)) in runs {
        println!("Executor, harmless {harmless}, seed {seed}: {samples} craft-ticks inside a hull, deepest {deepest:.0}, the Executor took {taken:.0}");
        assert_eq!(samples, 0, "no craft flies into the Executor (harmless {harmless}, seed {seed}): {deepest:.0} units deep");
        assert!(taken > 4000.0, "but they still hurt it from outside (harmless {harmless}, seed {seed}): {taken:.0}");
    }
}

/// `seconds` of a standard fleet battle (both sides AI) of `seed`: the unit-ticks of craft (fighters, bombers, hangar craft) and of big ships
/// inside another unit's model hull.
fn craft_and_ships_inside_hulls(data: &alamo_data::GameData, fs: &GameFs, seed: u64, seconds: f32) -> (u32, u32) {
    let empire = [("Star_Destroyer", 4), ("Victory_Destroyer", 3), ("Arquitens_E", 2), ("Acclamator", 1), ("TIE_Fighter_Squadron", 4), ("TIE_Bomber_Squadron", 2)];
    let rebel = [("MC-80_Star_Cruiser", 3), ("Nebulon_B_Frigate", 3), ("Assault_Frigate_Mark_II", 2), ("CR-90_Corvette", 3), ("X-Wing_Squadron", 4), ("Y-Wing_Squadron", 2)];
    let names: Vec<&str> = empire.iter().chain(rebel.iter()).map(|x| x.0).collect();
    let cat = Arc::new(Catalog::build(data, Some(fs), &names));
    let mk = |name: &str, units: &[(&str, u32)]| SideSetup { name: name.into(), units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }).collect(), structures: vec![], ai: true };
    let config = BattleConfig { seed, record_events: false, retreat_allowed: vec![false, false], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![mk("Empire", &empire), mk("Rebel", &rebel)], config });
    let (mut watch, mut craft, mut big) = (HullWatch::default(), 0u32, 0u32);
    for _ in 0..(30.0 * seconds) as usize {
        b.step();
        for i in watch.inside(&b, &cat) {
            let s = &b.ships[i.craft as usize];
            if matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber) || s.carrier != u32::MAX {
                craft += 1;
            } else {
                big += 1;
            }
        }
        if b.is_over() {
            break;
        }
    }
    (craft, big)
}

#[test]
fn no_craft_flies_through_a_hull_in_a_fleet_battle() {
    // Eight big ships a side with their squadrons and the garrison craft of the frigates and cruisers (gunships, shuttles, bombers), 150 s. On
    // main (87ab720) the craft spent 162422-173635 unit-ticks (seeds 1-3) inside the hulls of other ships, up to 66 units deep.
    let Some((data, fs)) = real_data() else { return };
    let runs: Vec<(u32, u32)> = std::thread::scope(|sc| {
        let jobs: Vec<_> = (1..=3)
            .map(|seed| {
                let (data, fs) = (&data, &fs);
                sc.spawn(move || craft_and_ships_inside_hulls(data, fs, seed, 150.0))
            })
            .collect();
        jobs.into_iter().map(|j| j.join().expect("fleet battle")).collect()
    });
    for (seed, &(craft, big)) in (1..).zip(&runs) {
        println!("fleet battle, seed {seed}: craft {craft} unit-ticks inside a model hull, big ships {big}");
        assert_eq!(craft, 0, "no fighter, bomber or hangar craft is ever inside a hull (seed {seed})");
        assert_eq!(big, 0, "nor a big ship (seed {seed})");
    }
}

/// Squadrons of one kind (`count` of them, harmless target) attack `target` for a minute: the mean damage over three seeds.
fn mean_damage_to(data: &alamo_data::GameData, fs: &GameFs, target: &str, squadron: &str, count: u32) -> f32 {
    let cat = Arc::new(Catalog::build(data, Some(fs), &[target, squadron]));
    let mk = |name: &str, n: &str, c: u32, ai: bool| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }], structures: vec![], ai };
    let mut total = 0.0;
    for seed in 1..=3 {
        let side_mults = vec![SideMult { damage: 0.0, health: 1.0, shield: 1.0 }, SideMult { damage: 1.0, health: 1.0, shield: 1.0 }];
        let config = BattleConfig { half_size: 12_000.0, seed, record_events: false, side_mults, retreat_allowed: vec![false, false], fog_of_war: false, ..BattleConfig::default() };
        let sides = vec![mk("Defender", target, 1, false), mk("Attacker", squadron, count, true)];
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
        let tgt = ids(&b, 0)[0];
        b.ships[tgt as usize].pos = Vec3::ZERO;
        b.ships[tgt as usize].yaw = 0.0;
        for (i, c) in ids(&b, 1).into_iter().enumerate() {
            let d = &mut b.ships[c as usize];
            d.pos = Vec3::new(-1500.0 + (i % 8) as f32 * 150.0, -4500.0 - (i / 8) as f32 * 150.0, 0.0);
            d.yaw = std::f32::consts::FRAC_PI_2;
        }
        let start = b.ships[tgt as usize].shield + b.ships[tgt as usize].hps.iter().map(|h| h.health).sum::<f32>();
        for _ in 0..30 * 60 {
            b.step();
        }
        total += start - b.ships[tgt as usize].shield - b.ships[tgt as usize].hps.iter().map(|h| h.health).sum::<f32>();
    }
    total / 3.0
}

#[test]
fn bombers_and_fighters_hurt_the_ships_they_attack_as_much_as_before() {
    // Damage in a minute from four squadrons of one kind on a harmless ship, mean of seeds 1-3. On main (87ab720), flying into the hull's
    // middle: 11580 (Y-wings on a Star Destroyer), 13585 (Y-wings on a Victory), 8295 (TIE bombers on an MC-80), 3451 (X-wings on a Star
    // Destroyer); with the runs ending at the hull's skin 10535, 13696, 8964 and 3118. Held to 80% of main.
    let Some((data, fs)) = real_data() else { return };
    let cases = [("Star_Destroyer", "Y-Wing_Squadron", 11580.0), ("Victory_Destroyer", "Y-Wing_Squadron", 13585.0), ("MC-80_Star_Cruiser", "TIE_Bomber_Squadron", 8295.0), ("Star_Destroyer", "X-Wing_Squadron", 3451.0)];
    let damages: Vec<f32> = std::thread::scope(|sc| {
        let jobs: Vec<_> = cases
            .iter()
            .map(|&(target, squadron, _)| {
                let (data, fs) = (&data, &fs);
                sc.spawn(move || mean_damage_to(data, fs, target, squadron, 4))
            })
            .collect();
        jobs.into_iter().map(|j| j.join().expect("attack battle")).collect()
    });
    for (&(target, squadron, main), damage) in cases.iter().zip(damages) {
        println!("{squadron} on {target}: {damage:.0} (main {main:.0})");
        assert!(damage >= 0.8 * main, "{squadron} still hurt a {target}: {damage:.0}, main {main:.0}");
    }
}
