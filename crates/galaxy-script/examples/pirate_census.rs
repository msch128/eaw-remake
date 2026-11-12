//! Measurement tool for the Pirates' growth (docs/RULES.md 1.9): Pirate fleets over a headless Remake-AI game.
//! `pirate_census <weeks> <seed> [campaign] [groundup] [human=<faction>|none] [pace] [research] [every=<n>] [load=<save.json>]`
//! Per 5 weeks: Pirate planets, units by category, AI power per planet, star base levels, the major factions' tech, units the
//! week tick added (the Pirates' growth) and units added outside it (the story scripts), captures of Pirate planets; the costliest fleet
//! of every major faction when its tech rises (`FLEET` lines, the input of `pirate_battles`). `pace`: the idle human's tech follows the best AI.
//! `research`: the human builds its research institute and buys every next tech level it can; `research=<n>`: at most one level order
//! every n weeks; `grant`: the human gets the credits a level order lacks (counted). `TECH` lines: every tech level a faction reaches;
//! `AIFLEET` lines (every 10 weeks): the majors' units by tech level and the Remake AI's budget shares and accounts; at the end the units
//! the major factions completed by the unit's tech level and the first of each level (`FIRST`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{DifficultyTier, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Event, FleetLocation, ProductionKind, Sim, SimConfig};

fn pirate_units(s: &Sim, pirates: u32) -> BTreeMap<u32, BTreeMap<u32, u32>> {
    let mut m: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    for f in s.galaxy().fleets.values().filter(|f| f.owner == pirates) {
        if let FleetLocation::AtPlanet(p) = f.location {
            for (&u, &n) in &f.units {
                *m.entry(p).or_default().entry(u).or_default() += n;
            }
        }
    }
    m
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(40);
    let seed: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1);
    let campaign_name = args.get(2).filter(|a| !a.contains('=') && *a != "groundup").map_or("GC_Full_Web_Way_Rebel", String::as_str);
    let ground_up = args.iter().any(|a| a == "groundup");
    let human = args.iter().find_map(|a| a.strip_prefix("human=")).unwrap_or("none").to_string();
    let install = GameInstall::detect().expect("game install");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    // `load=<save.json>`: play on from a save (a copy: the tool never writes it) instead of a new game.
    let mut sim = if let Some(path) = args.iter().find_map(|a| a.strip_prefix("load=")) {
        let t0 = Instant::now();
        let sim = ScriptedSim::load(path, fs, data).expect("load the save");
        println!("loaded {path} in {:?}: week {}, {} load notes, {} warnings {:?}", t0.elapsed(), sim.checkpoint().week, sim.load_notes.len(), sim.load_warnings.len(), sim.load_warnings.iter().take(3).collect::<Vec<_>>());
        sim
    } else {
        let campaign = data.campaign(campaign_name).expect("campaign").clone();
        ScriptedSim::new(
            fs,
            data,
            &campaign,
            SimConfig { victory_enabled: false, seed, ground_up, ..SimConfig::default() },
            ScriptedConfig { remake_ai: true, human: Some(human.clone()), game_scoring: false, strategy: true, difficulty: Some(DifficultyTier::Normal), ..ScriptedConfig::default() },
        )
        .expect("sim")
    };
    let start_week = sim.checkpoint().week;
    let weeks = weeks + start_week;
    let (pirates, names) = sim.with_sim(|s| (s.world().passive_faction().expect("pirates"), s.world().factions.iter().map(|f| f.name.clone()).collect::<Vec<_>>()));
    println!("campaign {campaign_name} ground_up {ground_up} human {human} seed {seed}; start errors {:?}", sim.start_errors);
    let mut captures: BTreeMap<String, u32> = Default::default();
    let mut added_week = 0u32;
    let mut added_other = 0u32;
    let mut added_by_cat_week: BTreeMap<String, u32> = Default::default();
    let mut added_by_cat_other: BTreeMap<String, u32> = Default::default();
    let mut done = start_week;
    let every: u64 = args.iter().find_map(|a| a.strip_prefix("every=")).and_then(|t| t.parse().ok()).unwrap_or(5);
    let mut last_report = u64::MAX;
    let mut tick_ms: Vec<f64> = Vec::new();
    let mut week_start = Instant::now();
    let mut seen_types: BTreeMap<u32, u32> = BTreeMap::new();
    let report = |sim: &ScriptedSim, week: u64, aw: u32, ao: u32| {
        sim.with_sim(|s| {
            let (w, g) = (s.world(), s.galaxy());
            let owned: Vec<usize> = (0..g.planets.len()).filter(|&p| g.planets[p].owner == Some(pirates) && w.real[p]).collect();
            let units = pirate_units(s, pirates);
            let mut cats: BTreeMap<String, u32> = BTreeMap::new();
            let mut powers: Vec<f32> = Vec::new();
            let mut total = 0;
            for &p in &owned {
                let mut pw = 0.0;
                if let Some(us) = units.get(&(p as u32)) {
                    for (&u, &n) in us {
                        let t = &w.units[u as usize];
                        *cats.entry(format!("{:?}", t.category)).or_default() += n;
                        pw += t.ai_power * n as f32;
                        total += n;
                    }
                }
                powers.push(pw);
            }
            powers.sort_by(f32::total_cmp);
            let n = powers.len().max(1) as f32;
            let mean = powers.iter().sum::<f32>() / n;
            let median = powers.get(powers.len() / 2).copied().unwrap_or(0.0);
            let armed = powers.iter().filter(|&&x| x > 0.0).count();
            let mut sbl: BTreeMap<i32, u32> = BTreeMap::new();
            for &p in &owned {
                *sbl.entry(w.starbase_level(g, p as u32)).or_default() += 1;
            }
            let tech: Vec<String> = w.main_factions.iter().map(|&f| format!("{}:{}/{}p", &w.factions[f as usize].name, g.factions[f as usize].tech, g.factions[f as usize].planets_owned)).collect();
            let all_units: u32 = g.fleets.values().map(|f| f.unit_count()).sum();
            println!(
                "week {week:>2}: pirate planets {:>2} ({armed} armed), units {total} {cats:?}, power/planet mean {mean:>6.0} median {median:>5.0} max {:>6.0}, base levels {sbl:?}, added week-tick {aw} other {ao}, tech {}, galaxy units {all_units}",
                owned.len(),
                powers.last().copied().unwrap_or(0.0),
                tech.join(" ")
            );
        });
    };
    report(&sim, 0, 0, 0);
    // Every faction's tech level (the human marked), at the start and at the end.
    let all_tech = |sim: &ScriptedSim, week: u64| {
        sim.with_sim(|s| {
            let (w, g) = (s.world(), s.galaxy());
            let list: Vec<String> = (0..w.factions.len())
                .map(|f| format!("{}{}={}", w.factions[f].name, if w.victory.active_player == Some(f as u32) { "(human)" } else { "" }, g.factions[f].tech))
                .collect();
            println!("ALLTECH week {week}: {}", list.join(" "));
        })
    };
    all_tech(&sim, start_week);
    // `pace`: the (idle) human's tech follows the best major AI faction, as a human who keeps pace would.
    let pace = args.iter().any(|a| a == "pace");
    // `research`: the human gives the orders a researching player gives: its `<F>_Research_Institute` at the first planet that
    // takes it, then every next tech level as soon as a planet's queue and the credits allow (through the same commands as the UI).
    // `research=<n>`: the same, but a level is ordered at most every n weeks (a player who researches at that pace).
    let research_every: Option<u64> = args.iter().find_map(|a| if a == "research" { Some(0) } else { a.strip_prefix("research=").and_then(|n| n.parse().ok()) });
    let research = research_every.is_some();
    let mut last_order: Option<u64> = None;
    let grant = args.iter().any(|a| a == "grant");
    let mut granted = 0.0f64;
    let mut last_tech: BTreeMap<u32, i32> = BTreeMap::new();
    let mut last_refusal = String::new();
    // Units the major factions completed, by the unit's tech level.
    let mut built_by_tech: BTreeMap<String, BTreeMap<i32, u32>> = BTreeMap::new();
    // The week and the type of the first unit of each tech level a major faction completed.
    let mut first_built: BTreeMap<(String, i32), (u64, String)> = BTreeMap::new();
    while done < weeks {
        sim.with_sim_mut(|s| {
            let w = s.world().clone();
            if pace && let Some(h) = w.victory.active_player {
                let best = w.main_factions.iter().filter(|&&f| f != h).map(|&f| s.galaxy().factions[f as usize].tech).max().unwrap_or(1);
                let t = &mut s.galaxy_mut().factions[h as usize].tech;
                *t = (*t).max(best);
            }
            if research && let Some(h) = w.victory.active_player {
                let prefix = w.factions[h as usize].name.chars().next().unwrap_or('X');
                let institute = w.structure_index.get(&format!("{prefix}_research_institute").to_ascii_lowercase()).copied();
                let planets: Vec<u32> = s.galaxy().owned_planets(h).collect();
                let has = |s: &Sim, st: u32| {
                    planets.iter().any(|&p| {
                        let ps = &s.galaxy().planets[p as usize];
                        ps.structures.contains(&st) || ps.queues.iter().flatten().any(|i| i.kind == ProductionKind::Structure(st))
                    })
                };
                let mut why = Vec::new();
                if let Some(st) = institute
                    && !has(s, st)
                {
                    for &p in &planets {
                        match s.apply(Command::BuildStructure { faction: h, planet: p, structure: st }) {
                            Ok(_) => break,
                            Err(e) => why.push(format!("institute {}: {e:?}", w.planets[p as usize].name)),
                        }
                    }
                    // The institute asks for a star base level: the first planet's base goes up (a refused order is only reported).
                    if !has(s, st)
                        && let Some(&p) = planets.first()
                        && let Err(e) = s.apply(Command::UpgradeStarbase { faction: h, planet: p })
                    {
                        why.push(format!("star base {}: {e:?}", w.planets[p as usize].name));
                    }
                }
                let due = match (last_order, research_every) {
                    (Some(at), Some(n)) => done >= at + n,
                    _ => true,
                };
                // `grant`: the human gets the credits its next level lacks (a player with more planets than the idle start's one), logged.
                if due && grant {
                    let fs = &s.galaxy().factions[h as usize];
                    let idx = (fs.tech - w.factions[h as usize].start_tech).max(0) as usize;
                    let lack = f64::from(w.tech_cost(s.galaxy(), h, idx)) - fs.credits;
                    if fs.tech < w.factions[h as usize].max_tech && lack > 0.0 {
                        s.add_credits(h, lack);
                        granted += lack;
                    }
                }
                for &p in planets.iter().filter(|_| due) {
                    match s.apply(Command::UpgradeTech { faction: h, planet: p }) {
                        Ok(_) => {
                            last_order = Some(done);
                            println!("ORDER week {done}: tech level {} at {}", s.galaxy().factions[h as usize].tech + 1, w.planets[p as usize].name);
                            break;
                        }
                        Err(e) => why.push(format!("tech {}: {e:?}", w.planets[p as usize].name)),
                    }
                }
                let why = format!("{institute:?} {why:?}");
                if why != last_refusal {
                    println!("RESEARCH week {done}: {why}");
                    last_refusal = why;
                }
            }
            // The main fleet (largest by AI power) of every major faction when its tech rises.
            for &f in &w.main_factions {
                let tech = s.galaxy().factions[f as usize].tech;
                if last_tech.get(&f) == Some(&tech) {
                    continue;
                }
                last_tech.insert(f, tech);
                let best = s.galaxy().fleets.values().filter(|x| x.owner == f).max_by(|a, b| {
                    let p = |x: &galaxy_sim::Fleet| x.units.iter().map(|(&u, &n)| w.units[u as usize].ai_power.max(w.units[u as usize].cost * 0.1) * n as f32).sum::<f32>();
                    p(a).total_cmp(&p(b))
                });
                if let Some(fl) = best {
                    let list: Vec<String> = fl.units.iter().filter(|(_, n)| **n > 0).map(|(&u, &n)| format!("{}={n}", w.units[u as usize].name)).collect();
                    println!("FLEET {} tech {tech} week {done} units {} cost {:.0}: {}", w.factions[f as usize].name, fl.unit_count(), fl.units.iter().map(|(&u, &n)| w.units[u as usize].cost * n as f32).sum::<f32>(), list.join(","));
                }
            }
        });
        let before = sim.with_sim(|s| pirate_units(s, pirates));
        let r = sim.step(1.0);
        let mut week_tick = false;
        for e in &r.sim_events {
            match e {
                Event::PlanetCaptured { new_owner, old_owner: Some(o), .. } if *o == pirates => {
                    *captures.entry(names[*new_owner as usize].clone()).or_default() += 1;
                }
                Event::ConstructionComplete { faction, kind: ProductionKind::Unit(u), .. } => sim.with_sim(|s| {
                    let w = s.world();
                    if w.main_factions.contains(faction) {
                        let t = w.units[*u as usize].tech;
                        *built_by_tech.entry(names[*faction as usize].clone()).or_default().entry(t).or_default() += 1;
                        first_built.entry((names[*faction as usize].clone(), t)).or_insert((done, w.units[*u as usize].name.clone()));
                    }
                }),
                Event::TechAdvanced { faction, level } => println!("TECH week {done} {} -> {level}", names[*faction as usize]),
                Event::WeekEnded { week } => {
                    done = *week;
                    week_tick = true;
                    tick_ms.push(week_start.elapsed().as_secs_f64() * 1000.0);
                    week_start = Instant::now();
                }
                _ => {}
            }
        }
        sim.with_sim(|s| {
            let after = pirate_units(s, pirates);
            for (p, us) in &after {
                for (u, n) in us {
                    let b = before.get(p).and_then(|m| m.get(u)).copied().unwrap_or(0);
                    if *n > b {
                        let cat = format!("{:?}", s.world().units[*u as usize].category);
                        *seen_types.entry(*u).or_default() += n - b;
                        if week_tick {
                            added_week += n - b;
                            *added_by_cat_week.entry(cat).or_default() += n - b;
                        } else {
                            added_other += n - b;
                            *added_by_cat_other.entry(cat).or_default() += n - b;
                        }
                    }
                }
            }
        });
        if done % every == 0 && done != last_report {
            last_report = done;
            report(&sim, done, added_week, added_other);
            if done % 10 == 0 {
                // What the major factions field by unit tech level, and what the Remake AI budgets and holds per category.
                let mem = sim.ai_memory();
                sim.with_sim(|s| {
                    let (w, g) = (s.world(), s.galaxy());
                    for &f in &w.main_factions {
                        let mut by_tech: BTreeMap<i32, u32> = BTreeMap::new();
                        for fl in g.fleets.values().filter(|x| x.owner == f) {
                            for (&u, &n) in &fl.units {
                                *by_tech.entry(w.units[u as usize].tech).or_default() += n;
                            }
                        }
                        let budget = mem.as_ref().map(|m| {
                            let fr: Vec<String> = m.brains.iter().filter(|b| b.player == f).flat_map(|b| b.fractions.iter().map(|(c, v)| format!("{c}={v:.2}"))).collect();
                            let acc: Vec<String> = m.task_forces.accounts.iter().filter(|(p, _, v)| *p == f && *v > 0.5).map(|(_, c, v)| format!("{c}={v:.0}")).collect();
                            format!("fractions [{}] accounts [{}] credits {:.0}", fr.join(" "), acc.join(" "), g.factions[f as usize].credits)
                        });
                        // Ship types of each tech level some owned planet takes now (credits and a free slot aside), and what blocks the ones
                        // of the faction's own level elsewhere.
                        let mut can: BTreeMap<i32, u32> = BTreeMap::new();
                        let mut blocked: BTreeMap<String, u32> = BTreeMap::new();
                        let tech = g.factions[f as usize].tech;
                        for &u in &w.buildable_units[f as usize] {
                            let ut = &w.units[u as usize];
                            if !matches!(ut.role, galaxy_sim::UnitRole::Ship) {
                                continue;
                            }
                            let mut first_err = None;
                            let ok = g.owned_planets(f).any(|p| match w.check_build_unit(g, f, p, u) {
                                Ok(()) | Err(galaxy_sim::CommandError::InsufficientCredits | galaxy_sim::CommandError::NoSlot) => true,
                                Err(e) => {
                                    first_err.get_or_insert(format!("{e:?}"));
                                    false
                                }
                            });
                            if ok {
                                *can.entry(ut.tech).or_default() += 1;
                            } else if ut.tech == tech && tech >= 2 {
                                *blocked.entry(first_err.unwrap_or_default()).or_default() += 1;
                            }
                        }
                        println!(
                            "AIFLEET week {done} {} tech {tech}: units by tech {by_tech:?}; buildable ship types by tech {can:?}, own-level types blocked {blocked:?}; {}",
                            w.factions[f as usize].name,
                            budget.unwrap_or_default()
                        );
                    }
                });
            }
        }
    }
    all_tech(&sim, done);
    println!("units built by the major factions, by unit tech level: {built_by_tech:?}");
    if grant {
        println!("credits granted to the human for its tech levels: {granted:.0}");
    }
    for ((f, t), (week, unit)) in &first_built {
        println!("FIRST {f} tech {t} week {week} {unit}");
    }
    println!("captures of pirate planets by faction: {captures:?}");
    println!("added in week-tick steps by category: {added_by_cat_week:?}");
    println!("added in other steps by category: {added_by_cat_other:?}");
    tick_ms.sort_by(f64::total_cmp);
    if !tick_ms.is_empty() {
        println!("wall ms per galaxy week: median {:.0} p90 {:.0} max {:.0}", tick_ms[tick_ms.len() / 2], tick_ms[tick_ms.len() * 9 / 10], tick_ms.last().unwrap());
    }
    sim.with_sim(|s| {
        let w = s.world();
        println!("unit types added (name, category, ai_power, cost, tech, count):");
        for (u, n) in &seen_types {
            let t = &w.units[*u as usize];
            println!("  {} {:?} {} {} {} x{n}", t.name, t.category, t.ai_power, t.cost, t.tech);
        }
    });
}
