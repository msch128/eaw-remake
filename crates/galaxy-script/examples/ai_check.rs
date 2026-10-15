//! Player-feel check of the Remake AI over a long match: expansion, fleets, idle fleets, unused credits, unit variety, heroes, recaptures.
//!
//! ```text
//! ai_check <weeks> <campaign> <human faction | none> <easy|normal|hard|brutal> [groundup]
//! ```
//! Prints a table every 10 weeks and a verdict block per faction at the end.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{DifficultyTier, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

#[derive(Default)]
struct Track {
    /// Fleet id -> (week since which its location and size did not change, size).
    still: HashMap<u32, (u64, u32, Option<u32>)>,
    worst_idle: u64,
    idle_samples: u64,
    fleet_samples: u64,
    credit_pile_weeks: u64,
    broke_weeks: u64,
    max_credits: f64,
    max_planets: usize,
    first_planet_gain_week: Option<u64>,
    owned_ever: HashSet<u32>,
    attacks_won: u32,
    attacks_lost: u32,
    lost_weeks: Vec<u64>,
    planets_at: [Option<usize>; 3],
    recaptures: u32,
    captures: u32,
    lost_planets: u32,
    peak_ships: u32,
    hero_weeks: u64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(60);
    let campaign_name = args.get(1).map_or("GC_Full_Web_Way_Rebel", String::as_str);
    let human = args.get(2).map_or("none", String::as_str).to_string();
    let diff = match args.get(3).map_or("normal", String::as_str) {
        "easy" => DifficultyTier::Easy,
        "hard" => DifficultyTier::Hard,
        "brutal" => DifficultyTier::Brutal,
        _ => DifficultyTier::Normal,
    };
    let ground_up = args.iter().any(|a| a == "groundup");

    let install = GameInstall::detect().expect("game install not found");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some(&std::env::var("MOD").unwrap_or_else(|_| "1770851727".to_owned()))).expect("cannot open game files"));
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign(campaign_name).expect("campaign not found").clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ground_up, auto_resolve_battles: std::env::var_os("ATTACKS").is_none(), ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some(human.clone()), game_scoring: false, strategy: true, difficulty: Some(diff), ..ScriptedConfig::default() },
    )
    .expect("cannot build simulation");
    let names: Vec<String> = sim.with_sim(|s| s.world().factions.iter().map(|f| f.name.clone()).collect());
    if let Ok(pn) = std::env::var("PLANET") {
        sim.with_sim(|s| {
            let (w, g) = (s.world(), s.galaxy());
            for (i, p) in w.planets.iter().enumerate().filter(|(_, p)| p.name.to_ascii_lowercase().contains(&pn.to_ascii_lowercase())) {
                let ps = &g.planets[i];
                println!("planet {} owner {:?}: credit_value {}, gamble {}, mining {}, total {}", p.name, ps.owner.map(|o| w.factions[o as usize].name.clone()), p.credit_value, ps.gamble, p.mining_colony, w.planet_income(g, i as u32));
                for &st in &ps.structures {
                    let s = &w.structures[st as usize];
                    println!("   structure {}: pct {} abs {} mining pct {} abs {}", s.name, s.income_pct, s.income_abs, s.income_pct_mining, s.income_abs_mining);
                }
                if let Some(sb) = ps.starbase {
                    let b = &w.starbases[sb as usize];
                    println!("   starbase {}: pct {} abs {}", b.name, b.income_pct, b.income_abs);
                }
                if let Some(o) = ps.owner {
                    println!("   faction credits_factor {}", w.factions[o as usize].credits_factor);
                }
            }
        });
    }
    println!("{campaign_name}, human {human}, {diff:?}, ground_up {ground_up}, {weeks} weeks; start errors {:?}", sim.start_errors);
    let mut track: Vec<Track> = (0..names.len()).map(|_| Track::default()).collect();
    let mut types_built: Vec<BTreeMap<String, u32>> = vec![BTreeMap::new(); names.len()];

    for f in 0..names.len() {
        track[f].planets_at[0] = Some(sim.with_sim(|s| s.galaxy().factions[f].planets_owned as usize));
    }
    let mut done = 0u64;
    while done < weeks {
        let owners_before: Vec<Option<u32>> = sim.with_sim(|s| s.galaxy().planets.iter().map(|p| p.owner).collect());
        let r = sim.step(1.0);
        for e in &r.sim_events {
            match e {
                Event::BattleResolved { planet, winner, losses, .. } => {
                    let defender = owners_before[*planet as usize];
                    let mut involved: Vec<u32> = losses.iter().map(|l| l.0).collect();
                    involved.extend(*winner);
                    involved.sort_unstable();
                    involved.dedup();
                    for f in involved.into_iter().filter(|f| Some(*f) != defender && (*f as usize) < names.len()) {
                        if *winner == Some(f) { track[f as usize].attacks_won += 1; } else { track[f as usize].attacks_lost += 1; }
                    }
                }
                Event::PlanetCaptured { old_owner: Some(o), .. } if (*o as usize) < track.len() => track[*o as usize].lost_weeks.push(done + 1),
                _ => {}
            }
        }
        for e in &r.sim_events {
            match e {
                Event::PlanetCaptured { planet, new_owner, old_owner, .. } => {
                    let n = *new_owner as usize;
                    if n < track.len() {
                        track[n].captures += 1;
                        if track[n].owned_ever.contains(planet) && old_owner.is_some() {
                            track[n].recaptures += 1;
                        }
                    }
                    if let Some(o) = old_owner.filter(|o| (*o as usize) < track.len()) {
                        track[o as usize].lost_planets += 1;
                    }
                }
                Event::BattleResolved { planet, winner, losses, .. } if std::env::var_os("BATTLES").is_some() => {
                    let who = |f: Option<u32>| f.map_or("-".to_owned(), |f| names[f as usize].clone());
                    let at = sim.with_sim(|s| (s.world().planets[*planet as usize].display_name.clone(), s.galaxy().planets[*planet as usize].owner));
                    println!("  battle week {done}: {} owner {} winner {} losses {:?}", at.0, who(at.1), who(*winner), losses.iter().map(|(f, n)| format!("{}:{n}", &names[*f as usize][..names[*f as usize].len().min(4)])).collect::<Vec<_>>());
                }
                Event::BattlePending(b) if std::env::var_os("ATTACKS").is_some() => {
                    let who = std::env::var("ATTACKS").unwrap_or_default();
                    let line = sim.with_sim(|s| {
                        use galaxy_sim::combat::Source;
                        let w = s.world();
                        let sides = s.battle_sides(b);
                        let fname = |f: u32| names[f as usize].clone();
                        let involved = sides.iter().any(|sd| who.is_empty() || who == "1" || fname(sd.faction).eq_ignore_ascii_case(&who));
                        if !involved {
                            return None;
                        }
                        let mut parts = Vec::new();
                        for sd in &sides {
                            let units: Vec<(Source, u32)> = sd.members.iter().map(|m| (m.source, m.count)).collect();
                            let ships: u32 = sd.members.iter().filter(|m| matches!(m.source, Source::Unit(_))).map(|m| m.count).sum();
                            let plain = w.strength(units.iter().copied());
                            let tuned = w.tuned_strength(units.iter().copied());
                            let fixed: Vec<String> = sd.members.iter().filter(|m| !matches!(m.source, Source::Unit(_))).map(|m| match m.source { Source::Starbase(x) => w.starbases[x as usize].name.clone(), Source::Structure(x) => format!("{}x{}", m.count, w.structures[x as usize].name), Source::Garrison(u) => format!("g{}x{}", m.count, w.units[u as usize].name), Source::Unit(_) => String::new() }).collect();
                            let pu = w.strength(units.iter().copied().filter(|m| matches!(m.0, Source::Unit(_))));
                            let pf = w.strength(units.iter().copied().filter(|m| !matches!(m.0, Source::Unit(_))));
                            parts.push(format!("{} {} ships plain {:.0} (ships {:.0} + fixed {:.0} = {:.0} summed) tuned {:.0} [{}]", fname(sd.faction), ships, plain, pu, pf, pu + pf, tuned, fixed.join(",")));
                        }
                        let mut wins = Vec::new();
                        for seed in 1..=9u64 {
                            let p = s.preview_auto_resolve(b.id, seed);
                            wins.push(p.and_then(|p| p.winner).map_or("-".to_owned(), |f| fname(f)[..4.min(fname(f).len())].to_owned()));
                        }
                        Some(format!("  attack week {done} t={:.0}s at {}: {} || preview winners {:?}", s.galaxy().time, w.planets[b.planet as usize].display_name, parts.join("  VS  "), wins))
                    });
                    if let Some(l) = line {
                        println!("{l}");
                    }
                    let evs = sim.with_sim_mut(|s| s.resolve_battle(b.id).unwrap_or_default());
                    for e in evs {
                        if let Event::BattleResolved { winner, losses, .. } = &e {
                            if std::env::var_os("ATTACKS").is_some() {
                                println!("      result: winner {:?} losses {:?}", winner.map(|f| names[f as usize].clone()), losses);
                            }
                        }
                    }
                }
                Event::BattlePending(b) if std::env::var_os("PENDING").is_some() => {
                    sim.with_sim(|s| {
                        let w = s.world();
                        let sides = s.battle_sides(b);
                        let txt: Vec<String> = sides
                            .iter()
                            .map(|sd| {
                                let mut m: Vec<String> = sd
                                    .members
                                    .iter()
                                    .map(|m| match m.source {
                                        galaxy_sim::combat::Source::Unit(u) => format!("{}x{}", m.count, w.units[u as usize].name),
                                        galaxy_sim::combat::Source::Starbase(x) => format!("SB:{}", w.starbases[x as usize].name),
                                        galaxy_sim::combat::Source::Structure(x) => format!("{}x{}", m.count, w.structures[x as usize].name),
                                        galaxy_sim::combat::Source::Garrison(u) => format!("garr {}x{}", m.count, w.units[u as usize].name),
                                    })
                                    .collect();
                                m.truncate(14);
                                format!("{}: {}", names[sd.faction as usize], m.join(", "))
                            })
                            .collect();
                        println!("  pending week {done} at {}: {}", w.planets[b.planet as usize].display_name, txt.join("  ||  "));
                    });
                }
                Event::WeekEnded { week } => {
                    done = *week;
                    for f in 0..names.len() {
                        let n = sim.with_sim(|s| s.galaxy().factions[f].planets_owned as usize);
                        if done == 0 || done == weeks / 2 || done == weeks {
                            let slot = if done == weeks { 2 } else if done == 0 { 0 } else { 1 };
                            track[f].planets_at[slot] = Some(n);
                        }
                    }
                    sim.with_sim(|s| {
                        let g = s.galaxy();
                        let w = s.world();
                        for f in 0..names.len() {
                            let fs = &g.factions[f];
                            if fs.eliminated {
                                continue;
                            }
                            let t = &mut track[f];
                            for p in g.owned_planets(f as u32) {
                                t.owned_ever.insert(p);
                            }
                            let planets = fs.planets_owned as usize;
                            t.max_planets = t.max_planets.max(planets);
                            t.max_credits = t.max_credits.max(fs.credits);
                            if fs.credits < 0.0 {
                                t.broke_weeks += 1;
                            }
                            // Credits that pile up: more than 25 weeks of income unspent after week 15 (AI factions only).
                            if done > 15 && names[f] != human && fs.income_per_week > 0.0 && fs.credits > 25.0 * fs.income_per_week {
                                t.credit_pile_weeks += 1;
                            }
                            let mut ships = 0;
                            let mut heroes = 0;
                            for fl in g.fleets.values().filter(|x| x.owner == f as u32) {
                                ships += fl.unit_count();
                                for (&u, &c) in &fl.units {
                                    if w.units[u as usize].hero && c > 0 {
                                        heroes += 1;
                                    }
                                }
                                let key = (fl.planet(), fl.unit_count());
                                let ent = t.still.entry(fl.id).or_insert((done, key.1, key.0));
                                if (ent.1, ent.2) != (key.1, key.0) {
                                    *ent = (done, key.1, key.0);
                                }
                                t.fleet_samples += 1;
                                let idle = done - ent.0;
                                if idle >= 20 && key.0.is_some() && names[f] != human {
                                    t.idle_samples += 1;
                                }
                                t.worst_idle = t.worst_idle.max(idle);
                            }
                            t.peak_ships = t.peak_ships.max(ships);
                            if heroes > 0 {
                                t.hero_weeks += 1;
                            }
                            if t.first_planet_gain_week.is_none() && planets > 1 {
                                t.first_planet_gain_week = Some(done);
                            }
                        }
                        if done % 10 == 0 {
                            let line: Vec<String> = (0..names.len())
                                .filter(|&f| !g.factions[f].eliminated && (g.factions[f].planets_owned > 0 || g.fleets.values().any(|x| x.owner == f as u32)))
                                .map(|f| {
                                    let fs = &g.factions[f];
                                    let ships: u32 = g.fleets.values().filter(|x| x.owner == f as u32).map(|x| x.unit_count()).sum();
                                    format!("{}:{}p/{}s/{:.0}k/pop{}", &names[f][..names[f].len().min(4)], fs.planets_owned, ships, fs.credits / 1000.0, fs.population_used)
                                })
                                .collect();
                            println!("week {done:>3}  {}", line.join("  "));
                        }
                    });
                }
                _ => {}
            }
        }
    }
    // Why a structure family cannot be built (STRUCT=<substring> FACTION=<name>).
    if let (Ok(sub), Ok(fac)) = (std::env::var("STRUCT"), std::env::var("FACTION")) {
        sim.with_sim(|s| {
            let w = s.world();
            let g = s.galaxy();
            let Some(f) = w.faction_id(&fac) else { return };
            let mut tally: BTreeMap<String, u32> = BTreeMap::new();
            for (i, st) in w.structures.iter().enumerate().filter(|(_, st)| st.name.to_ascii_lowercase().contains(&sub.to_ascii_lowercase())) {
                for p in g.owned_planets(f) {
                    let r = w.check_build_structure(g, f, p, i as u32);
                    *tally.entry(format!("{} -> {:?}", st.name, r.map_err(|e| format!("{e:?}")))).or_default() += 1;
                }
            }
            println!("\n== {sub} for {fac}: {tally:#?}");
        });
    }
    if std::env::var_os("ROSTER").is_some() {
        sim.with_sim(|s| {
            let w = s.world();
            for f in 0..names.len() {
                let mut by: BTreeMap<String, (u32, f64, f64, f32)> = BTreeMap::new();
                for &u in &w.buildable_units[f] {
                    let ut = &w.units[u as usize];
                    if ut.hero || ut.role != galaxy_sim::world::UnitRole::Ship {
                        continue;
                    }
                    let st = w.strength([(galaxy_sim::combat::Source::Unit(u), 1)]) as f64;
                    let e = by.entry(format!("{:?}", ut.category)).or_default();
                    e.0 += 1;
                    e.1 += f64::from(ut.cost);
                    e.2 += st;
                    e.3 += ut.population as f32;
                }
                if by.is_empty() {
                    continue;
                }
                let line: Vec<String> = by.iter().map(|(c, (n, cost, st, pop))| format!("{c}: {n} types, avg cost {:.0}, avg strength {:.0}, strength/cost {:.2}, avg pop {:.1}", cost / f64::from(*n), st / f64::from(*n), st / cost.max(1.0), pop / *n as f32)).collect();
                println!("roster {}: {}", names[f], line.join(" | "));
                let mut costs: Vec<f64> = w.buildable_units[f].iter().map(|&u| &w.units[u as usize]).filter(|u| !u.hero && u.role == galaxy_sim::world::UnitRole::Ship && !matches!(u.category, alamo_data::ShipCategory::Other)).map(|u| f64::from(u.cost)).collect();
                costs.sort_by(|a, b| a.total_cmp(b));
                if !costs.is_empty() {
                    let med = costs[costs.len() / 2];
                    let cheap = costs.iter().filter(|c| **c < 3000.0).count() as f64 / costs.len() as f64;
                    println!("identity {}: {} ship types, median cost {:.0}, share under 3000 credits {:.0}%, top-quartile mean {:.0}", names[f], costs.len(), med, 100.0 * cheap, costs[costs.len() * 3 / 4..].iter().sum::<f64>() / (costs.len() - costs.len() * 3 / 4) as f64);
                }
            }
        });
    }
    if std::env::var_os("HEROES").is_some() {
        sim.with_sim(|s| {
            let w = s.world();
            for f in 0..names.len() {
                let heroes: Vec<String> = w.units.iter().enumerate().filter(|(_, u)| u.hero).map(|(i, u)| (i as u32, u.name.clone())).filter(|(i, _)| w.buildable_units[f].contains(i)).map(|(_, n)| n).collect();
                if !heroes.is_empty() {
                    println!("buildable heroes {}: {}", names[f], heroes.join(", "));
                }
            }
        });
    }
    // Composition of what each faction owns at the end.
    sim.with_sim(|s| {
        let g = s.galaxy();
        let w = s.world();
        for f in 0..names.len() {
            for fl in g.fleets.values().filter(|x| x.owner == f as u32) {
                for (&u, &c) in &fl.units {
                    *types_built[f].entry(w.units[u as usize].name.clone()).or_default() += c;
                }
            }
        }
        println!("\nTABLE | faction | planets wk0 / mid / end | attacks won / lost | credits | idle fleet-weeks (20+) of fleet-weeks | max planets lost within 8 weeks |");
        for f in 0..names.len() {
            let t = &track[f];
            if t.max_planets == 0 {
                continue;
            }
            let mut worst = 0usize;
            for &w0 in &t.lost_weeks {
                worst = worst.max(t.lost_weeks.iter().filter(|&&w| w >= w0 && w < w0 + 8).count());
            }
            let at = |i: usize| t.planets_at[i].map_or("-".to_owned(), |v| v.to_string());
            println!("TABLE | {} | {} / {} / {} | {} / {} | {:.0}k | {} of {} | {} |", names[f], at(0), at(1), at(2), t.attacks_won, t.attacks_lost, g.factions[f].credits / 1000.0, t.idle_samples, t.fleet_samples, worst);
        }
        println!("\n== verdict per faction ({done} weeks)");
        for f in 0..names.len() {
            let fs = &g.factions[f];
            let t = &track[f];
            if t.max_planets == 0 && fs.planets_owned == 0 {
                continue;
            }
            let total: u32 = types_built[f].values().sum();
            let top = types_built[f].iter().max_by_key(|x| *x.1).map(|(n, c)| (n.clone(), *c));
            let top_share = top.as_ref().map_or(0.0, |x| f64::from(x.1) / f64::from(total.max(1)));
            println!(
                "{:11} planets {} (max {}, lost {}, captured {}, recaptured {}), credits {:.0} (max {:.0}), income {:.0}/wk, pop {}/{}, tech {}, ships now {} (peak {}), {} types, top {:?} = {:.0}%",
                names[f], fs.planets_owned, t.max_planets, t.lost_planets, t.captures, t.recaptures, fs.credits, t.max_credits, fs.income_per_week, fs.population_used, fs.population_cap, fs.tech, total, t.peak_ships, types_built[f].len(), top.map(|x| x.0), top_share * 100.0
            );
            println!(
                "{:11} stalls: worst fleet idle {} wk, idle>=20wk fleet-weeks {} of {}, credit pile weeks {}, broke weeks {}, hero weeks {}, spent {:.0}% of earned, battles won {} lost {}",
                "", t.worst_idle, t.idle_samples, t.fleet_samples, t.credit_pile_weeks, t.broke_weeks, t.hero_weeks, 100.0 * fs.stats.credits_spent / fs.stats.credits_earned.max(1.0), fs.stats.battles_won, fs.stats.battles_lost
            );
        }
    });
}
