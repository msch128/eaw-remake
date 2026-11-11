//! Measurements behind the weapon range, fire arc and own-hull rules (docs/RULES.md, battle section), on the real Remake data.
//!
//! ```text
//! firearc_probe flight                     hardpoints whose Fire_Range_Distance exceeds what their projectile can fly
//! firearc_probe ranges <unit>...           range numbers of the catalog per unit
//! firearc_probe approach <unit> [target]   one ship closes from 15000 on a standing target: first shot, first hit, stand-off (order and AI)
//! ```
use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn load() -> (GameData, GameFs) {
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    (load_galaxy(&fs).0, fs)
}

/// Census: per projectile, the weapon hardpoints mounted on a space unit or star base whose range is longer than the projectile's
/// `Projectile_Max_Flight_Distance`.
fn flight(data: &GameData) {
    let mut mounted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for u in data.objects.space_units() {
        for h in &u.hardpoints {
            mounted.entry(h.to_ascii_lowercase()).or_default().push(u.name.clone());
        }
    }
    for b in data.objects.star_bases() {
        for h in &b.hardpoints {
            mounted.entry(h.to_ascii_lowercase()).or_default().push(b.name.clone());
        }
    }
    // projectile -> (hardpoints, of them short, homing, flight distance, min and max range, units)
    let mut rows: BTreeMap<String, (u32, u32, bool, f32, f32, f32, Vec<String>)> = BTreeMap::new();
    let (mut total, mut short, mut short_homing) = (0u32, 0u32, 0u32);
    for (hp, units) in &mounted {
        let Some(h) = data.objects.hardpoint(hp) else { continue };
        let Some(p) = h.projectile.as_deref().and_then(|p| data.objects.projectile(p)) else { continue };
        if !(p.does_shield_damage || p.does_hitpoint_damage) || h.fire_range <= 0.0 {
            continue;
        }
        total += 1;
        let e = rows.entry(p.name.clone()).or_insert((0, 0, p.max_rate_of_turn > 0.0, p.max_flight_distance, f32::MAX, 0.0, Vec::new()));
        e.0 += 1;
        e.4 = e.4.min(h.fire_range);
        e.5 = e.5.max(h.fire_range);
        if p.max_flight_distance > 0.0 && h.fire_range > p.max_flight_distance {
            e.1 += 1;
            short += 1;
            if p.max_rate_of_turn > 0.0 {
                short_homing += 1;
            }
            for u in units {
                if !e.6.contains(u) {
                    e.6.push(u.clone());
                }
            }
        }
    }
    println!("mounted damaging weapon hardpoints: {total}; range beyond the projectile's flight distance: {short} ({short_homing} of them homing)");
    for (name, (n, s, homing, fly, lo, hi, units)) in &rows {
        if *s > 0 {
            println!("  {name}: flight {fly:.0}, {}, {s} of {n} hardpoints longer (ranges {lo:.0}..{hi:.0}); units: {}", if *homing { "homing" } else { "straight" }, units.iter().take(6).cloned().collect::<Vec<_>>().join(", "));
        }
    }
}

fn ranges(cat: &Catalog, data: &GameData, name: &str) {
    let d = cat.ship(cat.ship_id(name).unwrap_or_else(|| panic!("{name}")));
    let tmax = data.objects.space_unit(name).map(|u| u.targeting_max_distance);
    println!("{name}: Targeting_Max_Attack_Distance {tmax:?}; max_weapon_range {:.0} targeting_range {:.0} preferred_range {:.0} radius {:.0}", d.max_weapon_range, d.targeting_range, d.preferred_range, d.radius);
    let mut groups: BTreeMap<(String, String, i64), (u32, f32)> = BTreeMap::new();
    for h in &d.hardpoints {
        let Some(p) = h.projectile else { continue };
        let pd = &cat.projectiles[p as usize];
        let e = groups.entry((format!("{:?}", h.kind), pd.name.clone(), h.range as i64)).or_default();
        e.0 += 1;
        e.1 += h.dps;
    }
    let mut list: Vec<_> = groups.into_iter().collect();
    list.sort_by(|a, b| b.0.2.cmp(&a.0.2));
    for ((kind, proj, range), (n, dps)) in list {
        let pd = &cat.projectiles[cat.projectile_id(&proj).unwrap() as usize];
        println!("  {n:>3} x {kind:<7} range {range:>6} {proj} (flight {:.0}, speed {:.0}, turn {:.1}) dps {dps:.1}", pd.max_distance, pd.speed, pd.turn_rate);
    }
}

/// One `unit` closes from 15000 on a standing `target` that holds its fire. `ai`: the unit's side is played by the tactical AI, else it has an attack order.
fn approach(cat: &Arc<Catalog>, unit: &str, target: &str, ai: bool) {
    let u = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: 1, tag: 1 };
    let sides = vec![
        SideSetup { name: "a".into(), units: vec![u(unit)], structures: vec![], ai },
        SideSetup { name: "b".into(), units: vec![u(target)], structures: vec![], ai: false },
    ];
    let config = BattleConfig { half_size: 20_000.0, fog_of_war: false, record_events: true, staged_hangar_launch: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let me = b.ships.iter().find(|s| s.side == 0 && s.def == cat.ship_id(unit).unwrap()).unwrap().id;
    let it = b.ships.iter().find(|s| s.side == 1 && s.def == cat.ship_id(target).unwrap()).unwrap().id;
    // Everything else (squadrons the two carry) is taken out of the measurement.
    for s in &mut b.ships {
        if s.id != me && s.id != it {
            s.alive = false;
        }
    }
    b.ships[me as usize].pos = Vec3::new(-15_000.0, 0.0, 0.0);
    b.ships[me as usize].yaw = 0.0;
    b.ships[it as usize].pos = Vec3::ZERO;
    b.ships[it as usize].stance = Stance::HoldFire;
    b.ships[it as usize].hull = 1.0e9;
    for h in &mut b.ships[it as usize].hps {
        h.health = 1.0e9;
    }
    if !ai {
        b.command(0, Command::Attack { ships: vec![me], target: it }).unwrap();
    }
    let def = cat.ship(b.ships[me as usize].def);
    let (mut first_shot, mut first_hit, mut shots, mut hits, mut damage) = (None, None, 0u32, 0u32, 0.0f64);
    let mut last_fire_dist = 0.0;
    let mut stop_dist = None;
    for step in 0..30 * 600 {
        let dist = b.ships[me as usize].pos.dist(b.ships[it as usize].pos);
        let events: Vec<Event> = b.step().to_vec();
        for e in events {
            match e {
                Event::ProjectileFired { ship, hardpoint: Some(h), projectile, .. } if ship == me => {
                    shots += 1;
                    last_fire_dist = dist;
                    if first_shot.is_none() {
                        first_shot = Some((dist, step as f32 / 30.0, def.hardpoints[h as usize].name.clone(), cat.projectiles[projectile as usize].name.clone(), def.hardpoints[h as usize].range));
                    }
                }
                Event::Hit { ship, attacker, damage: dmg, projectile, .. } if ship == it && attacker == me && dmg > 0.0 => {
                    hits += 1;
                    damage += dmg as f64;
                    if first_hit.is_none() {
                        first_hit = Some((dist, step as f32 / 30.0, cat.projectiles[projectile as usize].name.clone()));
                    }
                }
                _ => {}
            }
        }
        let s = &b.ships[me as usize];
        if stop_dist.is_none() && step > 60 && s.speed < 0.5 {
            stop_dist = Some((s.pos.dist(b.ships[it as usize].pos), step as f32 / 30.0));
        }
        if !b.ships[me as usize].alive {
            break;
        }
    }
    let end = b.ships[me as usize].pos.dist(b.ships[it as usize].pos);
    println!("{unit} -> {target} ({}):", if ai { "tactical AI" } else { "attack order" });
    println!("  first shot {first_shot:?}");
    println!("  first damaging hit {first_hit:?}");
    println!("  stops at {stop_dist:?}; distance after 600 s {end:.0}; last shot at {last_fire_dist:.0}; shots {shots} hits {hits} damage {damage:.0}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (data, fs) = load();
    match args.first().map(String::as_str) {
        Some("flight") => flight(&data),
        Some("ranges") => {
            let names: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            let cat = Catalog::build(&data, Some(&fs), &names);
            for n in &names {
                ranges(&cat, &data, n);
            }
        }
        Some("approach") => {
            let unit = args.get(1).expect("unit");
            let target = args.get(2).map_or("MC-80_Star_Cruiser", String::as_str);
            let cat = Arc::new(Catalog::build(&data, Some(&fs), &[unit, target]));
            approach(&cat, unit, target, false);
            approach(&cat, unit, target, true);
        }
        _ => eprintln!("usage: firearc_probe flight | ranges <unit>... | approach <unit> [target]"),
    }
}
