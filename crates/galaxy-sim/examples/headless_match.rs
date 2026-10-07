//! Runs a full AI-vs-AI galactic conquest match headless and prints a weekly summary.
//!
//! Usage: `cargo run --release -p galaxy-sim --example headless_match -- [weeks] [seed] [campaign] [novictory] [tactical]`

use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::{Event, Sim, SimConfig};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(40);
    let seed: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1);
    let campaign_name = args.get(2).filter(|a| a.as_str() != "novictory" && a.as_str() != "tactical").map_or("GC_Full_Web_Way_Rebel", String::as_str);

    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let t = Instant::now();
    let (data, _) = load_galaxy(&fs);
    println!("data loaded in {:?}", t.elapsed());
    let campaign = data.campaign(campaign_name).expect("campaign not found");

    let t = Instant::now();
    let victory_enabled = !args.iter().any(|a| a == "novictory");
    let tactical = args.iter().any(|a| a == "tactical");
    let resolver = if tactical { galaxy_sim::BattleResolver::Tactical } else { galaxy_sim::BattleResolver::Statistical };
    let config = SimConfig { seed, victory_enabled, resolver, ..SimConfig::default() };
    let mut sim = Sim::new(&data, campaign, config).expect("cannot build simulation");
    sim.set_ai_factions(0..sim.world().factions.len() as u32);
    if tactical {
        let t = Instant::now();
        let names = sim.tactical_names();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        sim.set_tactical_catalog(std::sync::Arc::new(battle_sim::Catalog::build(&data, Some(&fs), &names)));
        println!("tactical catalog built in {:?}", t.elapsed());
        let arenas: std::collections::HashMap<String, std::sync::Arc<battle_sim::Arena>> = sim
            .tactical_map_names()
            .into_iter()
            .filter_map(|n| battle_sim::Arena::load(&data, &fs, &n).map(|a| (n, std::sync::Arc::new(a))))
            .collect();
        println!("{} tactical map arenas loaded", arenas.len());
        sim.set_tactical_arenas(arenas);
    }
    let w = sim.world().clone();
    println!(
        "world built in {:?}: {} planets ({} real), {} routes, {} factions, {} unit types, {} structures, {} star bases",
        t.elapsed(),
        w.planets.len(),
        w.real.iter().filter(|&&r| r).count(),
        w.routes.len(),
        w.factions.len(),
        w.units.len(),
        w.structures.len(),
        w.starbases.len()
    );
    for (i, f) in w.factions.iter().enumerate() {
        let g = &sim.galaxy().factions[i];
        println!(
            "  {:<13} credits {:>7.0} tech {} planets {:>3} income/week {:>7.0} buildable units {:>3} structures {:>3}",
            f.name,
            g.credits,
            g.tech,
            g.planets_owned,
            g.income_per_week,
            w.buildable_units[i].len(),
            w.buildable_structures[i].len()
        );
    }

    let mut battles = 0usize;
    let mut captures = 0usize;
    let mut week_time = Instant::now();
    let total = Instant::now();
    let mut worst = std::time::Duration::ZERO;
    let step = 1.0;
    let mut elapsed_weeks = 0u64;
    let mut hash = 0;
    'outer: while elapsed_weeks < weeks {
        let events = sim.step(step);
        for e in &events {
            match e {
                Event::BattleResolved { .. } => battles += 1,
                Event::PlanetCaptured { .. } => captures += 1,
                Event::FactionEliminated { faction } => println!("    !! {} eliminated", w.factions[*faction as usize].name),
                Event::Victory { faction } => {
                    println!("    !! VICTORY: {}", w.factions[*faction as usize].name);
                    break 'outer;
                }
                Event::WeekEnded { week } => {
                    elapsed_weeks = *week;
                    let d = week_time.elapsed();
                    worst = worst.max(d);
                    week_time = Instant::now();
                    if week % 5 == 0 || *week == 1 {
                        let g = sim.galaxy();
                        let mut line = String::new();
                        for (i, f) in w.factions.iter().enumerate() {
                            let fs = &g.factions[i];
                            if fs.planets_owned > 0 || !fs.eliminated {
                                line.push_str(&format!(" {}:{}/{:.0}k", &f.name[..f.name.len().min(4)], fs.planets_owned, fs.credits / 1000.0));
                            }
                        }
                        let ships: u32 = g.fleets.values().map(|f| f.unit_count()).sum();
                        println!("week {week:>3} [{:>6.1} ms] fleets {:>3} ships {:>4} battles {battles:>4} captures {captures:>4} |{line}", d.as_secs_f64() * 1000.0, g.fleets.len(), ships);
                    }
                }
                _ => {}
            }
        }
        hash = sim.galaxy().state_hash();
    }
    let t = total.elapsed();
    println!(
        "{} weeks in {:?} ({:.2} ms/week avg, worst {:.1} ms), {} battles, {} captures, state hash {hash:016x}",
        elapsed_weeks,
        t,
        t.as_secs_f64() * 1000.0 / elapsed_weeks.max(1) as f64,
        worst.as_secs_f64() * 1000.0,
        battles,
        captures
    );
    let g = sim.galaxy();
    println!("final planets:");
    for (i, f) in w.factions.iter().enumerate() {
        println!("  {:<13} planets {:>3} credits {:>8.0} tech {} pop {}/{}", f.name, g.factions[i].planets_owned, g.factions[i].credits, g.factions[i].tech, g.factions[i].population_used, g.factions[i].population_cap);
    }
}
