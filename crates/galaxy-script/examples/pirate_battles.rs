//! Measurement tool for the Pirates' growth (docs/RULES.md 1.9): the main fleet of each tech stage attacks typical Pirate planets of a
//! Ground Up game, or the galaxy week is timed with the Pirates grown.
//!
//! ```text
//! pirate_battles <fleets.txt> [tactical] [tseeds=<n>] [tech=<t>] [weeks=<n>] [campaign=<name>]
//! pirate_battles tick [weeks=<n>] [tech=<t>]
//! ```
//!
//! `fleets.txt` holds `FLEET <faction> tech <t> ...: name=n,name=n` lines (`pirate_census` output); for every tech level the costliest fleet of
//! that level is used. The Pirate state is the Ground Up start (story start spawn) plus `weeks` (18) galaxy weeks with the human at that tech
//! and no AI. For the median, the 90th percentile and the strongest Pirate planet (not Lehon, with a map) it prints 10 auto-resolve previews
//! and, with `tactical`, `tseeds` headless tactical battles. `tick` times the galaxy week (wall ms) with every Pirate planet held.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use battle_sim::{Arena, Catalog};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, Fleet, FleetLocation, GroupKind, Sim, SimConfig, run_headless};

fn arg<T: std::str::FromStr>(args: &[String], key: &str) -> Option<T> {
    args.iter().find_map(|a| a.strip_prefix(key)).and_then(|t| t.parse().ok())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tick = args.first().is_some_and(|a| a == "tick");
    let tactical = args.iter().any(|a| a == "tactical");
    let campaign_name = args.iter().find_map(|a| a.strip_prefix("campaign=")).unwrap_or("GC_Full_Web_Way_Rebel");
    let only_tech: Option<i32> = arg(&args, "tech=");
    let weeks: u32 = arg(&args, "weeks=").unwrap_or(18);
    let tseeds: u64 = arg(&args, "tseeds=").unwrap_or(1);
    let install = GameInstall::detect().expect("game install");
    let gfs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign(campaign_name).expect("campaign").clone();
    let cfg = SimConfig { victory_enabled: false, ground_up: true, auto_resolve_battles: false, seed: 7, battle_repeat_seconds: 0.0, ..SimConfig::default() };
    let mut scripted = ScriptedSim::new(fs.clone(), data.clone(), &campaign, cfg.clone(), ScriptedConfig { remake_ai: false, human: campaign.starting_active_player.clone(), game_scoring: false, ..ScriptedConfig::default() }).expect("sim");
    scripted.step(1.0);
    let snap = scripted.with_sim(|s| s.snapshot());
    drop(scripted);
    let fresh = |tech: i32| {
        let mut sim = Sim::new(&data, &campaign, cfg.clone()).expect("sim");
        sim.restore(snap.clone()).expect("restore");
        sim.set_ai_factions(std::iter::empty());
        let human = sim.world().victory.active_player.expect("human");
        sim.galaxy_mut().factions[human as usize].tech = tech;
        sim
    };
    let pirate_units = |sim: &Sim| -> u32 {
        let pirates = sim.world().passive_faction().unwrap();
        sim.galaxy().fleets.values().filter(|f| f.owner == pirates).map(|f| f.unit_count()).sum()
    };
    if tick {
        let tech = only_tech.unwrap_or(5);
        let mut sim = fresh(tech);
        let week = sim.world().config.week_seconds;
        let mut ms = Vec::new();
        for _ in 0..weeks {
            let t0 = Instant::now();
            sim.step(week);
            ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let last: Vec<f64> = ms[ms.len() / 2..].to_vec();
        let mut sorted = last.clone();
        sorted.sort_by(f64::total_cmp);
        println!("tick: tech {tech}, {weeks} weeks, {} Pirate units at the end; wall ms per galaxy week over the last {} weeks: mean {:.2} median {:.2} max {:.2}", pirate_units(&sim), last.len(), last.iter().sum::<f64>() / last.len() as f64, sorted[sorted.len() / 2], sorted.last().unwrap());
        return;
    }
    let fleets_file = args.first().expect("fleets file");
    let text = std::fs::read_to_string(fleets_file).expect("read fleets");
    // tech -> (faction, cost, units)
    let mut best: BTreeMap<i32, (String, f32, Vec<(String, u32)>)> = BTreeMap::new();
    for line in text.lines().filter(|l| l.starts_with("FLEET ")) {
        let mut it = line.split_whitespace();
        it.next();
        let faction = it.next().unwrap().to_string();
        if !["Empire", "Rebel", "Underworld"].contains(&faction.as_str()) {
            continue;
        }
        it.next();
        let tech: i32 = it.next().unwrap().parse().unwrap();
        let cost: f32 = line.split(" cost ").nth(1).and_then(|r| r.split(':').next()).and_then(|c| c.parse().ok()).unwrap_or(0.0);
        let list = line.split(": ").nth(1).unwrap_or("");
        let units: Vec<(String, u32)> = list.split(',').filter_map(|e| e.split_once('=')).map(|(n, c)| (n.to_string(), c.parse().unwrap())).collect();
        if best.get(&tech).is_none_or(|b| cost > b.1) {
            best.insert(tech, (faction, cost, units));
        }
    }
    for (&tech, (faction, cost, units)) in &best {
        if only_tech.is_some_and(|t| t != tech) {
            continue;
        }
        let mut sim = fresh(tech);
        let w = sim.world().clone();
        let pirates = w.passive_faction().unwrap();
        for _ in 0..weeks {
            sim.step(w.config.week_seconds);
        }
        let attacker = w.faction_id(faction).unwrap();
        // Pirate planets (not Lehon, with a map) by the cost of the Pirate fleets in orbit.
        let mut planets: Vec<(f32, u32, u32)> = (0..w.planets.len() as u32)
            .filter(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner == Some(pirates) && w.planets[p as usize].space_map.is_some() && !w.planets[p as usize].name.to_ascii_lowercase().contains("lehon"))
            .map(|p| {
                let fl: Vec<&Fleet> = sim.galaxy().fleets_at(p).filter(|f| f.owner == pirates).collect();
                (fl.iter().flat_map(|f| f.units.iter()).map(|(&u, &n)| w.units[u as usize].cost * n as f32).sum::<f32>(), fl.iter().map(|f| f.unit_count()).sum::<u32>(), p)
            })
            .collect();
        planets.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let n = planets.len();
        println!(
            "== tech {tech}: attacker {faction} main fleet cost {cost:.0} ({} units); {n} Pirate planets, {} Pirate units, fleet cost median {:.0} p90 {:.0}",
            units.iter().map(|u| u.1).sum::<u32>(),
            pirate_units(&sim),
            planets[n / 2].0,
            planets[n * 9 / 10].0
        );
        for (label, idx) in [("median", n / 2), ("p90", n * 9 / 10), ("max", n - 1)] {
            let (pcost, pn, planet) = planets[idx];
            let mut s = Sim::new(&data, &campaign, cfg.clone()).expect("sim");
            s.restore(sim.snapshot()).expect("restore");
            s.set_ai_factions(std::iter::empty());
            s.galaxy_mut().factions[attacker as usize].tech = tech;
            let neighbour = w.routes.iter().find_map(|r| (r.a == planet).then_some(r.b).or((r.b == planet).then_some(r.a))).unwrap_or(planet);
            let pf: Vec<String> = s.galaxy().fleets_at(planet).filter(|f| f.owner == pirates).flat_map(|f| f.units.iter().map(|(&u, &c)| format!("{}={c}", w.units[u as usize].name))).collect();
            {
                let g = s.galaxy_mut();
                let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
                let us = units.iter().filter_map(|(nm, c)| w.unit_id(nm).map(|u| (u, *c))).collect();
                g.fleets.insert(id, Fleet { id, owner: attacker, units: us, location: FleetLocation::AtPlanet(planet), previous_planet: neighbour, slot: 0 });
            }
            let battle = s.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending battle");
            let sb = s.galaxy().planets[planet as usize].starbase.map(|b| w.starbases[b as usize].name.clone());
            let (mut wins, mut lost_share) = (0, 0.0);
            let seeds = 10;
            for seed in 1..=seeds {
                let p = s.preview_auto_resolve(battle.id, seed).unwrap();
                wins += u32::from(p.winner == Some(attacker));
                let a = p.sides.iter().find(|x| x.0 == attacker).unwrap();
                lost_share += 1.0 - a.2 as f32 / a.1.max(1) as f32;
            }
            println!("  {label} planet {} base {sb:?} Pirate fleet cost {pcost:.0} units {pn}: {}", w.planets[planet as usize].name, pf.join(","));
            println!("    auto-resolve: attacker wins {wins}/{seeds}, mean attacker units lost {:.0}%", 100.0 * lost_share / seeds as f32);
            if tactical {
                let req = s.tactical_setup(battle.id).expect("request");
                let names: Vec<&str> = req.sides.iter().flat_map(|x| x.groups.iter().chain(x.reserve.iter()).map(|g| g.name.as_str())).collect();
                let catalog = Arc::new(Catalog::build(&data, Some(&gfs), &names));
                let arena = req.map.as_ref().and_then(|m| Arena::load(&data, &gfs, m)).map(Arc::new);
                for ts in 0..tseeds {
                    let mut req = req.clone();
                    req.seed = req.seed.wrapping_add(ts * 7919);
                    let t0 = Instant::now();
                    let r = run_headless(&req, &catalog, arena.clone()).expect("result");
                    let ai = req.sides.iter().position(|x| x.faction == attacker).unwrap();
                    let (mut c0, mut c1, mut n0, mut n1) = (0.0, 0.0, 0, 0);
                    for (gi, g) in req.sides[ai].groups.iter().enumerate().filter(|(_, g)| g.kind == GroupKind::Unit) {
                        let c = w.units[g.type_id as usize].cost;
                        c0 += c * g.count as f32;
                        n0 += g.count;
                        let alive = r.survived[ai][gi];
                        c1 += c * alive as f32;
                        n1 += alive;
                    }
                    let pw = r.winner.map(|i| req.sides[i].faction_name.clone());
                    println!("    tactical: winner {pw:?} after {:.0} s, attacker units {n1}/{n0} alive, cost lost {:.0}% (wall {:.1} s)", r.duration, 100.0 * (1.0 - c1 / c0.max(1.0)), t0.elapsed().as_secs_f32());
                }
            }
        }
    }
}
