//! Navigation check on real maps: big ships must not overlap each other, must not sit inside rocks and must not plan paths through rocks.
//!
//! ```text
//! navcheck [planet name part = bilbringi] [seconds = 240] [seed = 1]
//! ```
//! Runs the same battle with navigation off (the old push-out only) and on and prints the counts.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;
use galaxy_sim::*;

fn side(cat: &Catalog, name: &str, units: &[(&str, u32)], structures: &[&str]) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(),
        structures: structures.iter().map(|n| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).collect(),
        ai: true,
    }
}

fn seg_dist(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-6 { 0.0 } else { (((c.x - a.x) * dx + (c.y - a.y) * dy) / l2).clamp(0.0, 1.0) };
    let (px, py) = (a.x + dx * t, a.y + dy * t);
    ((c.x - px).powi(2) + (c.y - py).powi(2)).sqrt()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().cloned().unwrap_or_else(|| "bilbringi".into()).to_ascii_lowercase();
    let seconds: f32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(240.0);
    let seed: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(1);
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let w = sim.world();
    let map = w
        .planets
        .iter()
        .find(|p| p.name.to_ascii_lowercase().contains(&name) && p.space_map.is_some())
        .and_then(|p| p.space_map.clone())
        .unwrap_or_else(|| panic!("no planet {name} with a space map"));
    let arena = Arc::new(Arena::load(&data, &fs, &map).expect("map"));
    println!("{map}: {} rocks, attacker markers {}, half size {:.0}", arena.obstacles.len(), arena.attacker_entries.len(), arena.half_size());
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Victory_Destroyer", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "CR-90_Corvette", "TIE_Fighter_Squadron", "X-Wing_Squadron", "Y-Wing_Squadron", "TIE_Bomber_Squadron"]));
    let _ = &cat;
    for navigation in [false, true] {
        let sides = vec![
            side(&cat, "Empire", &[("Star_Destroyer", 4), ("Victory_Destroyer", 3), ("TIE_Fighter_Squadron", 4), ("TIE_Bomber_Squadron", 2)], &[]),
            side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 4), ("Nebulon_B_Frigate", 4), ("CR-90_Corvette", 3), ("X-Wing_Squadron", 4), ("Y-Wing_Squadron", 2)], &[]),
        ];
        let config = BattleConfig { arena: Some(arena.clone()), seed, navigation, record_events: false, ..BattleConfig::default() };
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
        if std::env::var("TRACE_START").is_ok() {
            for sh in b.ships.iter().filter(|x| x.side == 0 && !matches!(cat.ship(x.def).class, TargetClass::Fighter | TargetClass::Bomber)) {
                println!("  start {} at ({:.0}, {:.0}) radius {:.0}", cat.ship(sh.def).name, sh.pos.x, sh.pos.y, cat.ship(sh.def).radius);
            }
        }
        let (mut overlap_ticks, mut worst_overlap, mut in_rock_ticks, mut path_rock, mut paths, mut max_fighter_in_hull) = (0u32, 0.0f32, 0u32, 0u32, 0u32, 0u32);
        let steps = (seconds * 30.0) as u32;
        for tick in 0..steps {
            let early = false;
            b.step();
            if b.is_over() {
                break;
            }
            let big: Vec<&Ship> = b.ships.iter().filter(|s| s.alive && !s.retreated && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).collect();
            let mobile: Vec<&&Ship> = big.iter().filter(|s| !cat.ship(s.def).is_static).collect();
            let radius = |s: &Ship| {
                let d = cat.ship(s.def);
                if d.class as u8 >= TargetClass::Frigate as u8 { d.radius * 0.75 } else { d.radius }
            };
            let mut overlapping = false;
            for (i, a) in mobile.iter().enumerate() {
                for o in big.iter().filter(|o| o.id != a.id && a.carrier != o.id && o.carrier != a.id) {
                    if cat.ship(o.def).is_static || o.id > a.id {
                        let min = (radius(a) + radius(o)) * 0.6;
                        let d = a.pos.dist(o.pos);
                        if d < min {
                            if early {
                                continue;
                            }
                            if std::env::var("TRACE_OVERLAP").is_ok() && overlap_ticks < 40 {
                                println!("  t={:.1}s {} (side {}, order {:?}, speed {:.0}) vs {} (side {}, static {}) d={:.0} min={:.0}", tick as f32 / 30.0, cat.ship(a.def).name, a.side, a.order, a.speed, cat.ship(o.def).name, o.side, cat.ship(o.def).is_static, d, min);
                            }
                            overlapping = true;
                            worst_overlap = worst_overlap.max(1.0 - d / min);
                        }
                    }
                }
                let _ = i;
                for r in &arena.obstacles {
                    if a.pos.dist(r.pos) < r.radius * 0.9 {
                        in_rock_ticks += 1;
                    }
                }
            }
            if overlapping && !early {
                overlap_ticks += 1;
            }
            if tick % 15 == 0 {
                for a in &mobile {
                    if a.path.len() > 1 {
                        paths += 1;
                        let mut from = a.pos;
                        let mut bad = false;
                        for &wp in &a.path {
                            bad |= arena.obstacles.iter().any(|r| !(from.dist(r.pos) < r.radius || wp.dist(r.pos) < r.radius) && seg_dist(from, wp, r.pos) < r.radius * 0.95);
                            from = wp;
                        }
                        path_rock += u32::from(bad);
                        if bad && std::env::var("TRACE_PATH").is_ok() && path_rock < 4 {
                            println!("  bad path of {} at {:?}: {:?}", cat.ship(a.def).name, a.pos, a.path);
                            for r in &arena.obstacles {
                                let mut f = a.pos;
                                for &wp in &a.path {
                                    if !(f.dist(r.pos) < r.radius || wp.dist(r.pos) < r.radius) && seg_dist(f, wp, r.pos) < r.radius * 0.95 {
                                        println!("     crosses rock at {:?} r {:.0} (segment {:?} -> {:?})", r.pos, r.radius, f, wp);
                                    }
                                    f = wp;
                                }
                            }
                        }
                    }
                }
                for f in b.ships.iter().filter(|s| s.alive && matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)) {
                    if big.iter().any(|h| h.id != f.carrier && f.pos.dist(h.pos) < radius(h) * 0.3) {
                        max_fighter_in_hull += 1;
                    }
                }
            }
        }
        let r = b.result();
        println!(
            "navigation {:5}: {} s, winner {:?}; hull overlap in {overlap_ticks} ticks (worst {:.0}% of the keep-out); big ships inside rocks {in_rock_ticks} ship-ticks; planned paths {paths}, through a rock {path_rock}; fighters deep inside a hull {max_fighter_in_hull} samples",
            navigation,
            r.duration as u32,
            r.winner,
            worst_overlap * 100.0
        );
    }
}
