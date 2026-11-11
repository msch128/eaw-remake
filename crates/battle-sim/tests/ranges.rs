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
