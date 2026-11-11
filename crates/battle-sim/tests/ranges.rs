//! Effective weapon ranges on the real Remake data: what a hardpoint's `Fire_Range_Distance` lets a ship do. Skipped when the game is not installed.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some((load_galaxy(&fs).0, fs))
}

const TARGET: &str = "MC-80_Star_Cruiser";

/// `unit` with an attack order on a standing cruiser that holds its fire and cannot die, `dist` away (centre to centre). Squadrons are removed.
fn duel(cat: &Arc<Catalog>, unit: &str, dist: f32) -> (Battle, u32, u32) {
    let u = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: 1, tag: 1 };
    let sides = vec![
        SideSetup { name: "a".into(), units: vec![u(unit)], structures: vec![], ai: false },
        SideSetup { name: "b".into(), units: vec![u(TARGET)], structures: vec![], ai: false },
    ];
    let config = BattleConfig { half_size: 20_000.0, fog_of_war: false, record_events: true, staged_hangar_launch: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let me = b.ships.iter().find(|s| s.side == 0 && s.def == cat.ship_id(unit).unwrap()).unwrap().id;
    let it = b.ships.iter().find(|s| s.side == 1 && s.def == cat.ship_id(TARGET).unwrap()).unwrap().id;
    for s in &mut b.ships {
        if s.id != me && s.id != it {
            s.alive = false;
        }
    }
    b.ships[me as usize].pos = Vec3::new(-dist, 0.0, 0.0);
    b.ships[me as usize].yaw = 0.0;
    b.ships[it as usize].pos = Vec3::ZERO;
    b.ships[it as usize].stance = Stance::HoldFire;
    b.ships[it as usize].hull = 1.0e9;
    for h in &mut b.ships[it as usize].hps {
        h.health = 1.0e9;
    }
    b.command(0, Command::Attack { ships: vec![me], target: it }).unwrap();
    (b, me, it)
}

/// Damage `me` did to `it` in this step with the projectiles named `names`.
fn damage_of(events: &[Event], cat: &Catalog, me: u32, it: u32, names: &[&str]) -> f32 {
    events
        .iter()
        .map(|e| match e {
            Event::Hit { ship, attacker, projectile, damage, .. } if *ship == it && *attacker == me && names.iter().any(|n| cat.projectiles[*projectile as usize].name.eq_ignore_ascii_case(n)) => *damage,
            _ => 0.0,
        })
        .sum()
}

#[test]
fn torpedo_sphere_torpedoes_reach_a_target_at_their_fire_range() {
    // `HARD_POINT_WEAPON_TORPEDO` x 182: `Fire_Range_Distance` 10000. `Proj_Proton_Torpedo`: `Projectile_Max_Flight_Distance` 2500, homing.
    // The torpedoes were fired from 10000 and ran out after 2500 units: a Torpedo Sphere held 9000 from its target never hit it.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Torpedo_Sphere", TARGET]));
    let (mut b, me, it) = duel(&cat, "Torpedo_Sphere", 9000.0);
    let hold = b.ships[me as usize].pos;
    let (mut torpedo, mut first) = (0.0f32, None);
    for step in 0..30 * 90 {
        // Held in place: the measurement is the reach of the weapon, not the approach.
        b.ships[me as usize].pos = hold;
        b.ships[me as usize].speed = 0.0;
        let events = b.step().to_vec();
        let d = damage_of(&events, &cat, me, it, &["Proj_Proton_Torpedo_TS", "Proj_Proton_Torpedo_TS_Damage"]);
        if d > 0.0 && first.is_none() {
            first = Some(step as f32 / 30.0);
        }
        torpedo += d;
    }
    let first = first.expect("no torpedo of a Torpedo Sphere 9000 from its target reached it in 90 s");
    // 9000 less the two hulls at 300 units per second: about half a minute of flight.
    assert!((20.0..45.0).contains(&first), "first torpedo hit after {first} s");
    assert!(torpedo > 10_000.0, "torpedo damage in 90 s: {torpedo}");
}

#[test]
fn a_homing_shot_inside_its_flight_distance_keeps_the_data_value() {
    // A TIE bomber's proton torpedo is the same projectile fired from 1500 at most: nothing changes for it. Its torpedoes still run out
    // 2500 units from the muzzle when the target is gone.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Torpedo_Sphere", TARGET]));
    let (mut b, me, it) = duel(&cat, "Torpedo_Sphere", 2000.0);
    let id = cat.projectile_id("Proj_Proton_Torpedo_TS").unwrap();
    let hold = b.ships[me as usize].pos;
    let (mut longest, mut launched) = (0.0f32, false);
    for _ in 0..30 * 40 {
        b.step();
        b.ships[me as usize].pos = hold;
        b.ships[me as usize].speed = 0.0;
        // Once torpedoes are on their way the target jumps away and the ship holds its fire: the shots in flight chase until they run out.
        launched |= b.projectiles.iter().filter(|p| p.def == id).count() >= 10;
        if launched {
            b.ships[it as usize].pos = Vec3::new(15_000.0, 0.0, 0.0);
            b.ships[me as usize].stance = Stance::HoldFire;
            b.ships[me as usize].order = Order::Idle;
        }
        for p in b.projectiles.iter().filter(|p| p.def == id) {
            longest = longest.max(p.pos.dist(hold));
        }
    }
    assert!(launched, "no torpedoes were launched");
    assert!(longest > 1500.0, "torpedoes flew {longest}");
    assert!(longest <= 2500.0 + 500.0, "a torpedo launched at a target 2000 away flew {longest} from the ship (flight distance 2500, tubes up to 500 from the middle)");
}

/// Closes from 15000 with an attack order for 240 s. Returns (centre distance at the first damaging hit, projectile of that hit, distance
/// where the ship came to rest, the largest excess of a shot's reach over its hardpoint's range). The reach of a shot is measured from the
/// muzzle to the hardpoint of the target it is aimed at, or to 0.6 of the target's hit radius short of its middle: the larger of the two
/// allowances is taken off here. The cruiser launches squadrons during the run, which the guns in reach of them fire at as well.
fn approach(cat: &Arc<Catalog>, unit: &str) -> (f32, String, f32, f32) {
    let (mut b, me, it) = duel(cat, unit, 15_000.0);
    let def = cat.ship(b.ships[me as usize].def);
    let (mut first, mut rest, mut excess) = (None, None, f32::MIN);
    for step in 0..30 * 240 {
        let dist = b.ships[me as usize].pos.dist(b.ships[it as usize].pos);
        let before: Vec<Vec3> = b.ships.iter().map(|s| s.pos).collect();
        for e in b.step().to_vec() {
            match e {
                Event::ProjectileFired { ship, hardpoint: Some(h), from, .. } if ship == me => {
                    let target = b.ships[me as usize].hps[h as usize].target as usize;
                    let tdef = cat.ship(b.ships[target].def);
                    let hit_radius = if tdef.class as u8 >= TargetClass::Frigate as u8 { tdef.radius * 0.75 } else { tdef.radius };
                    let at = before.get(target).copied().unwrap_or(b.ships[target].pos);
                    excess = excess.max(from.dist(at) - (hit_radius * 0.6).max(tdef.hardpoint_reach) - def.hardpoints[h as usize].range);
                }
                Event::Hit { ship, attacker, projectile, damage, .. } if ship == it && attacker == me && damage > 0.0 && first.is_none() => {
                    first = Some((dist, cat.projectiles[projectile as usize].name.clone()));
                }
                _ => {}
            }
        }
        if rest.is_none() && step > 60 && b.ships[me as usize].speed < 0.5 {
            rest = Some(b.ships[me as usize].pos.dist(b.ships[it as usize].pos));
        }
    }
    let (d, p) = first.unwrap_or_else(|| panic!("{unit} never hit"));
    (d, p, rest.unwrap_or_else(|| panic!("{unit} never stopped")), excess)
}

#[test]
fn a_torpedo_tube_counts_its_warhead() {
    // One tube: (20 + 400) every 45 s = 9.33 damage per second. It counted the torpedo's 20 alone (0.44), so the 182 tubes were 7% of the
    // Torpedo Sphere's output and its stand-off came from its four quad turbolasers: 0.8 x 5000 = 4000. Now 0.8 x 10000 = 8000.
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Torpedo_Sphere"]);
    let ts = cat.ship(cat.ship_id("Torpedo_Sphere").unwrap());
    let tube = ts.hardpoints.iter().find(|h| h.name == "TorSph_01").expect("tube");
    assert_eq!(cat.shot_damage(tube.projectile.unwrap()), 420.0);
    assert!((tube.dps - 420.0 / 45.0).abs() < 1e-3, "{}", tube.dps);
    assert_eq!(ts.hardpoints.iter().filter(|h| h.kind == HpKind::Weapon && h.range == 10_000.0).count(), 182);
    assert_eq!(ts.preferred_range, 8000.0);
}

#[test]
fn the_range_of_a_facing_hardpoint_is_not_a_weapon_range() {
    // `Proj_Facing_Dummy` hardpoints: 50000 on the Annihilator and the Puifier, 9000 on the Executor, 9900 on the Torpedo Sphere. They turn
    // the bow and count for no range: the longest weapon and the stand-off come from the damaging hardpoints.
    let Some((data, fs)) = load() else { return };
    let names = ["Annihilator_Super_Star_Destroyer", "Puifier_Super_Star_Destroyer", "Executor_Star_Dreadnought_P1", "Torpedo_Sphere"];
    let cat = Catalog::build(&data, Some(&fs), &names);
    let numbers: Vec<(f32, f32, f32, f32)> = names
        .iter()
        .map(|n| {
            let d = cat.ship(cat.ship_id(n).unwrap());
            let facing = d.hardpoints.iter().filter(|h| h.kind == HpKind::Facing).map(|h| h.range).fold(0.0, f32::max);
            (facing, d.max_weapon_range, d.targeting_range, d.preferred_range)
        })
        .collect();
    // (facing hardpoint, longest weapon, `Targeting_Max_Attack_Distance`, stand-off)
    assert_eq!(numbers, vec![(50_000.0, 4500.0, 4400.0, 3325.0), (50_000.0, 6000.0, 4400.0, 4275.0), (9000.0, 11_000.0, 8900.0, 7200.0), (9900.0, 10_000.0, 9900.0, 8000.0)]);
}

#[test]
fn the_big_ships_open_fire_and_stand_off_where_their_weapon_ranges_put_them() {
    // Centre to centre against a standing Mon Calamari cruiser, closing from 15000 with an attack order. A range is measured from the muzzle
    // (an Executor's bow guns are 4500 units ahead of its middle) to 0.6 of the target's hit radius short of its middle.
    //   unit            first damaging hit              comes to rest
    //   Torpedo Sphere  8169, a torpedo (was 5245,      8169 (was 3969: inside the reach of nothing but its 10 guns,
    //                   a quad turbolaser)              out of the reach of its torpedoes' 2500 flight)
    //   Annihilator     5556, ion cannon 4500           3201
    //   Puifier         6158, heavy turbolaser 6000     4212
    //   Executor        11951, turbolaser 9000          7179
    let Some((data, fs)) = load() else { return };
    let table = [
        ("Torpedo_Sphere", 8169.0, "Proj_Proton_Torpedo_TS", 8169.0),
        ("Annihilator_Super_Star_Destroyer", 5556.0, "Proj_Ship_Ion_Cannon_Large_Triple", 3201.0),
        ("Puifier_Super_Star_Destroyer", 6158.0, "Proj_Ship_Turbolaser_Green_Heavy", 4212.0),
        ("Executor_Star_Dreadnought_P1", 11_951.0, "Proj_Ship_Turbolaser_Green_Dual", 7179.0),
    ];
    for (unit, first, with, rest) in table {
        let cat = Arc::new(Catalog::build(&data, Some(&fs), &[unit, TARGET]));
        let (d, p, r, excess) = approach(&cat, unit);
        assert!((d - first).abs() < 25.0 && p == with, "{unit}: first damaging hit at {d} with {p}");
        assert!((r - rest).abs() < 25.0, "{unit}: came to rest at {r}");
        // No shot left a hardpoint whose `Fire_Range_Distance` did not reach (the ship closes during a step: 5 units of slack).
        assert!(excess <= 5.0, "{unit}: a hardpoint fired {excess} beyond its range");
    }
}
