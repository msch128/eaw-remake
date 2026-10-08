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
    let planet = (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && sim.galaxy().planets[p as usize].owner == Some(def)).expect("a defender planet");
    // The star base of the requested level in the defender's chain.
    let mut sb = w.first_starbase[def as usize].expect("chain");
    while w.starbases[sb as usize].level < level {
        sb = w.starbases[sb as usize].next.expect("next level");
    }
    sim.set_starbase(planet, Some(sb));
    sim.galaxy_mut().planets[planet as usize].structures.clear();
    sim.spawn_units(att, planet, w.unit_id(&unit_name).unwrap(), count);
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("battle");
    let req = sim.tactical_setup(battle.id).unwrap();
    println!("{} at {} ({}): {}x {} vs {} level {} star base ({})", w.planets[planet as usize].name, req.map.clone().unwrap_or_default(), req.planet_name, count, unit_name, def_name, level, w.starbases[sb as usize].name);
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().map(|g| g.name.as_str())).collect();
    let catalog = Arc::new(battle_sim::Catalog::build(&data, Some(&fs), &names));
    let arena = req.map.as_ref().and_then(|m| battle_sim::Arena::load(&data, &fs, m)).map(Arc::new);
    let mut last_station = f32::NAN;
    let mut obs = |b: &battle_sim::Battle| {
        let station = b.ships.iter().filter(|s| s.alive && s.side == 1 && catalog.ship(s.def).is_static && catalog.ship(s.def).hull > 5000.0).max_by(|a, c| catalog.ship(a.def).hull.total_cmp(&catalog.ship(c.def).hull));
        let Some(st) = station else { println!("t={:.0}: the station is gone", b.time()); return };
        let mine: Vec<_> = b.ships.iter().filter(|s| s.alive && s.side == 0).collect();
        let dists: Vec<f32> = mine.iter().map(|s| s.pos.dist(st.pos)).collect();
        let near = dists.iter().filter(|&&d| d < catalog.ship(st.def).radius).count();
        let mean = dists.iter().sum::<f32>() / dists.len().max(1) as f32;
        let dps = if last_station.is_nan() { 0.0 } else { (last_station - (st.hull + st.shield)) / 10.0 };
        last_station = st.hull + st.shield;
        println!(
            "t={:>4.0}s station {} hull {:.0}/{:.0} shield {:.0}/{:.0}  dps taken {:>7.0}  attackers {} mean dist {:.0} (station radius {:.0}), inside radius {}, preferred range {:.0}",
            b.time(), catalog.ship(st.def).name, st.hull, catalog.ship(st.def).hull, st.shield, catalog.ship(st.def).shield, dps, mine.len(), mean, catalog.ship(st.def).radius, near,
            mine.first().map_or(0.0, |s| catalog.ship(s.def).preferred_range)
        );
    };
    let res = galaxy_sim::tactical::run_headless_observed(&req, &catalog, arena, &mut obs).unwrap();
    println!("winner side {:?} after {:.0}s, survivors {:?}", res.winner, res.duration, res.survived);
}
