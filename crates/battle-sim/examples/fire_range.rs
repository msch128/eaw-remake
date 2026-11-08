//! Where an Executor fires and hits, and whether the enemy answers: one Executor against a Rebel fleet (6 MC-80, 6 Nebulon-B, 4 Assault
//! Frigates, 6 CR-90, 6 X-Wing squadrons; the Rebels are the defender with AI, the Executor has none).
//!
//! ```text
//! fire_range [advance] [seed=N] [secs=S]
//! ```
//! Prints `Targeting_Max_Attack_Distance` and the weapon hardpoints' `Fire_Range_Distance`, then the distances of the Executor's shots
//! (muzzle to target centre), of its hits (its centre to the impact) and of the hits on it (attacker to impact) as percentiles, and when
//! the first hits fell. `advance` gives the Executor an attack-move into the Rebel fleet at the start; without it, it holds its place.
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn pct(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f32 * p) as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let advance = args.iter().any(|a| a == "advance");
    let seed: u64 = args.iter().find_map(|a| a.strip_prefix("seed=")).and_then(|v| v.parse().ok()).unwrap_or(1);
    let secs: f32 = args.iter().find_map(|a| a.strip_prefix("secs=")).and_then(|v| v.parse().ok()).unwrap_or(240.0);
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, _) = load_galaxy(&fs);
    let rebels: &[(&str, u32)] = &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 6), ("Assault_Frigate_Mark_II", 4), ("CR-90_Corvette", 6), ("X-Wing_Squadron", 6)];
    let exe: &[(&str, u32)] = &[("Executor_Star_Dreadnought", 1)];
    let names: Vec<&str> = rebels.iter().chain(exe.iter()).map(|x| x.0).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let id = cat.ship_id("Executor_Star_Dreadnought").unwrap();
    let d = cat.ship(id);
    if let Some(u) = data.objects.space_unit("Executor_Star_Dreadnought") {
        println!("data: Targeting_Max_Attack_Distance {:?} sight/sensor {}", u.targeting_max_distance, d.sensor_range);
    }
    println!("def: max_weapon_range {:.0} targeting_range {:.0} preferred {:.0} radius {:.0}", d.max_weapon_range, d.targeting_range, d.preferred_range, d.radius);
    let mut ranges: Vec<(String, f32, usize)> = Vec::new();
    for h in d.hardpoints.iter().filter(|h| h.kind == HpKind::Weapon) {
        let key = format!("{} proj={:?}", h.name.split('_').take(3).collect::<Vec<_>>().join("_"), h.projectile.map(|p| cat.projectiles[p as usize].name.clone()));
        if let Some(e) = ranges.iter_mut().find(|e| e.0 == key && e.1 == h.range) {
            e.2 += 1;
        } else {
            ranges.push((key, h.range, 1));
        }
    }
    ranges.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (k, r, n) in ranges.iter().take(12) {
        println!("  hardpoint x{n} range {r:.0} {k}");
    }
    let side = |name: &str, units: &[(&str, u32)], ai: bool| SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }).collect(),
        structures: vec![],
        ai,
    };
    let sides = vec![side("Empire", exe, false), side("Rebels", rebels, true)];
    let config = BattleConfig { seed, defenders: vec![false, true], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let exe_id = b.ships.iter().find(|s| s.def == id).unwrap().id;
    let mut fire_d = Vec::new();
    let mut hit_exe_out = Vec::new();
    let mut hit_exe_in = Vec::new();
    let (mut first_in, mut first_out) = (None, None);
    let mut start_gap = None;
    let mut ordered = false;
    while !b.is_over() && b.time() < secs {
        if advance && !ordered {
            let rebels: Vec<Vec3> = b.ships.iter().filter(|s| s.side == 1 && s.alive).map(|s| s.pos).collect();
            let to = rebels.iter().fold(Vec3::ZERO, |a, &p| a + p) * (1.0 / rebels.len().max(1) as f32);
            let _ = b.command(0, Command::AttackMove { ships: vec![exe_id], to });
            ordered = true;
        }
        let t = b.time();
        let evs = b.step().to_vec();
        if start_gap.is_none() {
            let e = &b.ships[exe_id as usize];
            let near = b.ships.iter().filter(|s| s.side == 1 && s.alive).map(|s| s.pos.dist(e.pos)).fold(f32::MAX, f32::min);
            start_gap = Some(near);
        }
        for ev in evs {
            match ev {
                Event::ProjectileFired { ship, from, .. } if ship == exe_id => {
                    let tg = b.ships[exe_id as usize].target;
                    if tg != u32::MAX && (tg as usize) < b.ships.len() {
                        fire_d.push(from.dist(b.ships[tg as usize].pos));
                    }
                }
                Event::Hit { ship, attacker, pos, .. } => {
                    if attacker == exe_id {
                        let a = b.ships[exe_id as usize].pos;
                        hit_exe_out.push(a.dist(pos));
                        first_out.get_or_insert(t);
                    } else if ship == exe_id {
                        hit_exe_in.push(b.ships[attacker as usize].pos.dist(pos));
                        first_in.get_or_insert(t);
                    }
                }
                _ => {}
            }
        }
    }
    let e = &b.ships[exe_id as usize];
    println!("scenario {}: start gap to nearest enemy {:.0}; ends {:.0} s; Executor alive {} hull {:.0}", if advance { "advance" } else { "hold" }, start_gap.unwrap_or(0.0), b.time(), e.alive, e.hull);
    for (name, v, first) in [("Executor fire (muzzle->target)", &mut fire_d, None), ("Executor hits (ship->impact)", &mut hit_exe_out, first_out), ("Hits on Executor (attacker->impact)", &mut hit_exe_in, first_in)] {
        println!(
            "{name}: n {} first {:?} min {:.0} p50 {:.0} p90 {:.0} p99 {:.0} max {:.0}",
            v.len(), first, pct(v, 0.0), pct(v, 0.5), pct(v, 0.9), pct(v, 0.99), pct(v, 1.0)
        );
    }
    let alive = b.ships.iter().filter(|s| s.side == 1 && s.alive && !s.retreated).count();
    let ret = b.ships.iter().filter(|s| s.side == 1 && s.retreated).count();
    println!("rebels left {alive}, retreated {ret}");
}
