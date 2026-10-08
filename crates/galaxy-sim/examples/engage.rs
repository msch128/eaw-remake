//! Engagement diagnostics: a fleet attacks a planet with a star base; prints standoff distances and the damage the station takes.
//!
//! ```text
//! engage [attacker faction=Empire] [defender faction=Rebel] [attacker unit=Star_Destroyer] [count=6] [starbase level=2]
//! ```

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: &str| args.get(i).cloned().unwrap_or_else(|| d.to_string());
    let (att_name, def_name, unit_name) = (arg(0, "Empire"), arg(1, "Rebel"), arg(2, "Star_Destroyer"));
    let count: u32 = arg(3, "6").parse().unwrap();
    let level: i32 = arg(4, "2").parse().unwrap();

    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (att, def) = (w.faction_id(&att_name).unwrap(), w.faction_id(&def_name).unwrap());
    sim.galaxy_mut().fleets.clear();
    let wanted = std::env::var("PLANET").ok().map(|n| n.to_ascii_lowercase());
    let planet = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && match &wanted { Some(n) => w.planets[p as usize].name.to_ascii_lowercase().contains(n.as_str()), None => sim.galaxy().planets[p as usize].owner == Some(def) })
        .expect("a defender planet");
    sim.set_planet_owner(planet, Some(def));
    // The star base of the requested level in the defender's chain.
    let mut sb = w.first_starbase[def as usize].expect("chain");
    while w.starbases[sb as usize].level < level {
        sb = w.starbases[sb as usize].next.expect("next level");
    }
    sim.set_starbase(planet, Some(sb));
    sim.galaxy_mut().planets[planet as usize].structures.clear();
    // `unit` may be a list `Name:count,Name:count`.
    for part in unit_name.split(',') {
        let (n, c) = part.split_once(':').map_or((part, count), |(n, c)| (n, c.parse().unwrap()));
        sim.spawn_units(att, planet, w.unit_id(n).unwrap_or_else(|| panic!("unknown unit {n}")), c);
    }
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("battle");
    let req = sim.tactical_setup(battle.id).unwrap();
    println!("{} at {} ({}): {}x {} vs {} level {} star base ({})", w.planets[planet as usize].name, req.map.clone().unwrap_or_default(), req.planet_name, count, unit_name, def_name, level, w.starbases[sb as usize].name);
    {
        // Expected damage rate from the data: every weapon of the fleet in range, damage type against the station armor, no misses.
        let sbd = &w.starbases[sb as usize].combat;
        let (mut shield_dps, mut hull_dps) = (0.0f32, 0.0f32);
        for part in unit_name.split(',') {
            let (n, c) = part.split_once(':').map_or((part, count), |(n, c)| (n, c.parse().unwrap()));
            let u = &w.units[w.unit_id(n).unwrap() as usize];
            for wp in &u.combat.weapons {
                shield_dps += c as f32 * wp.shield_dps * w.damage.multiplier(wp.damage_type, sbd.shield_armor);
                hull_dps += c as f32 * wp.hull_dps * w.damage.multiplier(wp.damage_type, sbd.armor);
            }
        }
        println!("data: the whole fleet has {shield_dps:.0} shield dps / {hull_dps:.0} hull dps against the station armor (every weapon in range, no misses). Station shield {:.0} with regen {:.0} per {}s = {:.0}/s, hull {:.0}", sbd.shield, sbd.shield_regen, w.config.week_seconds * 0.0 + 2.0, sbd.shield_regen / 2.0, sbd.hull);
    }
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().map(|g| g.name.as_str())).collect();
    let catalog = Arc::new(battle_sim::Catalog::build(&data, Some(&fs), &names));
    let arena = req.map.as_ref().and_then(|m| battle_sim::Arena::load(&data, &fs, m)).map(Arc::new);
    let mut last_station = f32::NAN;
    let mut obs = |b: &battle_sim::Battle| {
        let station = b.ships.iter().filter(|s| s.alive && s.side == 1 && catalog.ship(s.def).is_static && catalog.ship(s.def).hull > 5000.0).max_by(|a, c| catalog.ship(a.def).hull.total_cmp(&catalog.ship(c.def).hull));
        let Some(st) = station else { println!("t={:.0}: the station is gone", b.time()); return };
        let mine: Vec<_> = b.ships.iter().filter(|s| s.alive && s.side == 0 && !matches!(catalog.ship(s.def).class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber)).collect();
        let dists: Vec<f32> = mine.iter().map(|s| s.pos.dist(st.pos)).collect();
        if b.time() < 11.0 { println!("station at {:?}, first ship at {:?}", st.pos, mine.first().map(|s| s.pos)); }
        let near = dists.iter().filter(|&&d| d < catalog.ship(st.def).radius).count();
        let mean = dists.iter().sum::<f32>() / dists.len().max(1) as f32;
        // Facing: angle between the ship bearing to the station relative to its heading and the bearing at which most of its weapons bear.
        let wrap = |a: f32| { let mut a = a; while a > std::f32::consts::PI { a -= std::f32::consts::TAU; } while a < -std::f32::consts::PI { a += std::f32::consts::TAU; } a };
        let mut errs = Vec::new();
        let mut held = 0;
        for s in mine.iter().filter(|s| s.pos.dist(st.pos) < catalog.ship(s.def).preferred_range * 1.3 + 400.0) {
            let d = catalog.ship(s.def);
            let to = st.pos - s.pos;
            let rel = wrap(to.y.atan2(to.x) - s.yaw);
            let best = d.bearing_dps.iter().enumerate().max_by(|a, c| a.1.total_cmp(c.1)).map(|(k, _)| wrap(k as f32 * std::f32::consts::TAU / 24.0)).unwrap_or(0.0);
            errs.push(wrap(rel - best).abs().to_degrees());
            if s.speed < 5.0 { held += 1; }
        }
        let facing = if errs.is_empty() { String::from("-") } else { format!("{:.0} deg mean over {} ships in range ({} stopped)", errs.iter().sum::<f32>() / errs.len() as f32, errs.len(), held) };
        println!("      facing error {facing}");
        let dps = if last_station.is_nan() { 0.0 } else { (last_station - (st.hull + st.shield)) / 10.0 };
        last_station = st.hull + st.shield;
        println!(
            "t={:>4.0}s station {} hull {:.0}/{:.0} shield {:.0}/{:.0}  dps taken {:>7.0}  attackers {} mean dist {:.0} (station radius {:.0} / model {:.0}), inside radius {}, preferred range {:.0}",
            b.time(), catalog.ship(st.def).name, st.hull, catalog.ship(st.def).hull, st.shield, catalog.ship(st.def).shield, dps, mine.len(), mean, catalog.ship(st.def).radius, catalog.ship(st.def).model_extent, near,
            mine.first().map_or(0.0, |s| catalog.ship(s.def).preferred_range)
        );
    };
    // Player-style orders: ORDER=attack|move|attackmove|hardpoint gives the first side (not AI) that order against the station.
    if let Ok(order) = std::env::var("ORDER") {
        use battle_sim::Command;
        let config = battle_sim::BattleConfig { record_events: false, ..req.battle_config(arena) };
        let mut battle = battle_sim::Battle::new(req.battle_setup(&catalog, config, |side| side != 0).unwrap());
        let station = battle.ships.iter().filter(|s| s.side == 1 && catalog.ship(s.def).is_static).max_by(|a, c| catalog.ship(a.def).hull.total_cmp(&catalog.ship(c.def).hull)).map(|s| (s.id, s.pos)).unwrap();
        let mine: Vec<u32> = battle.ships.iter().filter(|s| s.side == 0 && !matches!(catalog.ship(s.def).class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber)).map(|s| s.id).collect();
        let all: Vec<u32> = battle.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
        let r = match order.as_str() {
            "attack" => battle.command(0, Command::Attack { ships: all, target: station.0 }),
            "move" => battle.command(0, Command::Move { ships: all, to: station.1 }),
            "attackmove" => battle.command(0, Command::AttackMove { ships: all, to: station.1 }),
            _ => Ok(()),
        };
        println!("order {order}: {r:?}, {} capital-class ships", mine.len());
        let mut n = 0u64;
        while !battle.is_over() && battle.time() < 600.0 {
            battle.step();
            n += 1;
            if n.is_multiple_of(300) {
                obs(&battle);
            }
        }
        println!("over {} at {:.0}s", battle.is_over(), battle.time());
        return;
    }
    let res = galaxy_sim::tactical::run_headless_observed(&req, &catalog, arena, &mut obs).unwrap();
    println!("winner side {:?} after {:.0}s, survivors {:?}", res.winner, res.duration, res.survived);
}
