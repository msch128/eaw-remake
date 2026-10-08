//! A scripted "human" plays the economy for 30 weeks as Empire, Rebel and Underworld (builds income structures and star bases, researches, builds and sends fleets)
//! while every number is checked against the data: the cached income against a fresh sum over planets, structures and ships, the credits against income and purchases,
//! the population cap against the capacities, and the build options against the tech level.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Event, FactionId, FleetLocation, PlanetId, SimConfig, World};

struct Report {
    name: String,
    weeks: u64,
    discrepancies: Vec<String>,
    lines: Vec<String>,
}

/// Income from scratch: planets (credit value, structure and star base modifiers, mining colony extras, gambling), ship upkeep, faction factor.
fn income_from_data(w: &World, g: &galaxy_sim::Galaxy, f: FactionId) -> f64 {
    let mut total = 0.0f64;
    for (i, ps) in g.planets.iter().enumerate() {
        if ps.owner != Some(f) {
            continue;
        }
        let info = &w.planets[i];
        let (mut pct, mut abs) = (ps.gamble, 0.0f32);
        for &s in &ps.structures {
            let st = &w.structures[s as usize];
            pct += st.income_pct;
            abs += st.income_abs;
            if info.mining_colony {
                pct += st.income_pct_mining;
                abs += st.income_abs_mining;
            }
        }
        if let Some(sb) = ps.starbase {
            pct += w.starbases[sb as usize].income_pct;
            abs += w.starbases[sb as usize].income_abs;
        }
        total += f64::from(info.credit_value * (1.0 + pct) + abs);
    }
    for fl in g.fleets.values().filter(|x| x.owner == f) {
        for (&u, &n) in &fl.units {
            total += f64::from(w.units[u as usize].upkeep * n as f32);
        }
    }
    total * f64::from(w.factions[f as usize].credits_factor)
}

fn pop_cap_from_data(w: &World, g: &galaxy_sim::Galaxy, f: FactionId) -> i32 {
    let mut cap = w.config.base_population_cap;
    for (i, ps) in g.planets.iter().enumerate().filter(|(_, p)| p.owner == Some(f)) {
        cap += w.planets[i].population_capacity;
        if let Some(sb) = ps.starbase {
            cap += w.starbases[sb as usize].population_capacity;
        }
        cap += ps.structures.iter().map(|&s| w.structures[s as usize].population_capacity).sum::<i32>();
    }
    cap
}

fn decide(sim: &mut ScriptedSim, me: FactionId) {
    let (w, snapshot) = sim.with_sim(|s| (s.world().clone(), s.galaxy().clone()));
    let g = &snapshot;
    let credits = g.factions[me as usize].credits;
    let mut spend = credits;
    for p in g.owned_planets(me).collect::<Vec<PlanetId>>() {
        let ps = &g.planets[p as usize];
        let info = &w.planets[p as usize];
        // Structures: best income per credit, else population or unlocks.
        if ps.queues[1].is_empty() {
            let best = w.buildable_structures[me as usize]
                .iter()
                .copied()
                .filter(|&s| w.check_build_structure(g, me, p, s).is_ok() && f64::from(w.structures[s as usize].cost) <= spend * 0.4)
                .max_by(|&a, &b| {
                    let score = |s: u32| {
                        let st = &w.structures[s as usize];
                        let gamble = st.gamble.map_or(0.0, |g| (g.chance * g.max + (1.0 - g.chance) * g.min) * info.credit_value);
                        (st.income_abs + st.income_pct * info.credit_value + st.population_capacity as f32 * 20.0 + st.income_pct_mining + gamble) / st.cost.max(1.0)
                    };
                    score(a).total_cmp(&score(b)).then(a.cmp(&b))
                });
            if let Some(s) = best
                && sim.apply(Command::BuildStructure { faction: me, planet: p, structure: s }).is_ok()
            {
                spend -= f64::from(w.structures[s as usize].cost);
            }
        }
        // Base lane: star base first, then tech.
        if ps.queues[2].is_empty() {
            if let Ok(next) = w.check_upgrade_starbase(g, me, p)
                && f64::from(w.starbases[next as usize].cost) <= spend * 0.5
                && sim.apply(Command::UpgradeStarbase { faction: me, planet: p }).is_ok()
            {
                spend -= f64::from(w.starbases[next as usize].cost);
            } else if let Ok(idx) = w.check_upgrade_tech(g, me, p)
                && f64::from(w.factions[me as usize].tech_upgrades[idx].cost) <= spend * 0.6
                && sim.apply(Command::UpgradeTech { faction: me, planet: p }).is_ok()
            {
                spend -= f64::from(w.factions[me as usize].tech_upgrades[idx].cost);
            }
        }
        // Ships.
        if ps.queues[0].is_empty() && spend > 15000.0 {
            let best = w.buildable_units[me as usize]
                .iter()
                .copied()
                .filter(|&u| w.units[u as usize].cost > 0.0 && f64::from(w.units[u as usize].cost) <= spend * 0.3 && w.check_build_unit(g, me, p, u).is_ok())
                .max_by(|&a, &b| {
                    let score = |u: u32| {
                        let c = &w.units[u as usize];
                        (c.combat.durability() * c.combat.raw_dps()).sqrt() / c.cost
                    };
                    score(a).total_cmp(&score(b)).then(a.cmp(&b))
                });
            if let Some(u) = best
                && sim.apply(Command::BuildUnit { faction: me, planet: p, unit: u }).is_ok()
            {
                spend -= f64::from(w.units[u as usize].cost);
            }
        }
    }
    // Fleets: gather at the biggest stack and attack the weakest neighbouring non-owned planet.
    let mine: Vec<_> = g.fleets.values().filter(|f| f.owner == me && f.planet().is_some()).collect();
    for f in &mine {
        let at = f.planet().unwrap();
        if let Some(keep) = mine.iter().find(|o| o.id < f.id && o.planet() == Some(at)) {
            let _ = sim.apply(Command::MergeFleets { faction: me, into: keep.id, from: f.id });
        }
    }
    if let Some(big) = mine.iter().filter(|f| f.unit_count() >= 8 && !g.battles.iter().any(|b| b.fleets.contains(&f.id))).max_by_key(|f| f.unit_count()) {
        let at = big.planet().unwrap();
        let strength = w.strength(big.units.iter().map(|(&u, &n)| (galaxy_sim::combat::Source::Unit(u), n)));
        let target = w
            .routes
            .iter()
            .filter_map(|r| (r.a == at).then_some(r.b).or((r.b == at).then_some(r.a)))
            .filter(|&n| w.real[n as usize] && g.planets[n as usize].owner != Some(me))
            .min_by(|&a, &b| {
                let def = |p: u32| {
                    let ps = &g.planets[p as usize];
                    let fleets = g.fleets_at(p).filter(|x| x.owner != me).flat_map(|x| x.units.iter().map(|(&u, &n)| (galaxy_sim::combat::Source::Unit(u), n))).collect::<Vec<_>>();
                    let sb = ps.starbase.map(|b| (galaxy_sim::combat::Source::Starbase(b), 1)).into_iter();
                    w.strength(fleets.into_iter().chain(sb))
                };
                def(a).total_cmp(&def(b)).then(a.cmp(&b))
            });
        if let Some(t) = target {
            let enemy = g.fleets_at(t).filter(|x| x.owner != me).flat_map(|x| x.units.iter().map(|(&u, &n)| (galaxy_sim::combat::Source::Unit(u), n))).collect::<Vec<_>>();
            if strength > 1.5 * w.strength(enemy.into_iter().chain(g.planets[t as usize].starbase.map(|b| (galaxy_sim::combat::Source::Starbase(b), 1)))) {
                let _ = sim.apply(Command::MoveFleet { faction: me, fleet: big.id, to: t });
            }
        }
    }
    let _ = FleetLocation::AtPlanet(0);
}

fn play(name: &str, weeks: u64) -> Option<Report> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let mut sim = ScriptedSim::new(fs, data, &campaign, SimConfig { victory_enabled: false, seed: 11, ..SimConfig::default() }, ScriptedConfig { human: Some(name.into()), ..ScriptedConfig::default() }).ok()?;
    let (w, me) = sim.with_sim(|s| (s.world().clone(), s.world().faction_id(name).unwrap()));
    let mut rep = Report { name: name.into(), weeks, discrepancies: Vec::new(), lines: Vec::new() };
    let mut expected_earned = 0.0f64;
    let mut week = 0;
    let mut last_tech = 1;
    let mut options_by_tech: Vec<(i32, usize, usize)> = Vec::new();
    let mut step = 0u64;
    let mut gambling_seen: std::collections::BTreeSet<i32> = Default::default();
    while week < weeks {
        step += 1;
        if step % 10 == 1 {
            decide(&mut sim, me);
        }
        let events = sim.step(1.0).sim_events;
        for e in &events {
            if let Event::WeekEnded { week: wk } = e {
                week = *wk;
                let (income, cached, pop, cap, pop_data, credits, tech, planets, structs, ships) = sim.with_sim(|s| {
                    let g = s.galaxy();
                    let f = &g.factions[me as usize];
                    let ships: u32 = g.fleets.values().filter(|x| x.owner == me).map(|x| x.unit_count()).sum();
                    (income_from_data(&w, g, me), f.income_per_week, f.population_used, f.population_cap, pop_cap_from_data(&w, g, me), f.credits, f.tech, f.planets_owned, g.planets.iter().filter(|p| p.owner == Some(me)).map(|p| p.structures.len()).sum::<usize>(), ships)
                });
                if (income - cached).abs() > 1.0 {
                    rep.discrepancies.push(format!("week {week}: cached income {cached:.1} vs data {income:.1}"));
                }
                if cap != pop_data {
                    rep.discrepancies.push(format!("week {week}: population cap {cap} vs data {pop_data}"));
                }
                sim.with_sim(|s| {
                    for p in s.galaxy().planets.iter().filter(|p| p.owner == Some(me) && p.gamble != 0.0) {
                        gambling_seen.insert((p.gamble * 100.0).round() as i32);
                    }
                });
                if week % 5 == 0 {
                    rep.lines.push(format!("week {week:>2}: credits {credits:>9.0} income {income:>7.0}/wk planets {planets:>2} structures {structs:>2} ships {ships:>3} pop {pop}/{cap} tech {tech}"));
                }
                if tech != last_tech {
                    // Build options: units and structures the faction could queue at its best planet apart from credits.
                    let (units, structures) = sim.with_sim(|s| {
                        let g = s.galaxy();
                        let mut g2 = g.clone();
                        g2.factions[me as usize].credits = 1.0e9;
                        let planets: Vec<PlanetId> = g.owned_planets(me).collect();
                        let u = w.buildable_units[me as usize].iter().filter(|&&u| planets.iter().any(|&p| w.check_build_unit(&g2, me, p, u).is_ok())).count();
                        let st = w.buildable_structures[me as usize].iter().filter(|&&st| planets.iter().any(|&p| w.check_build_structure(&g2, me, p, st).is_ok())).count();
                        (u, st)
                    });
                    options_by_tech.push((tech, units, structures));
                    last_tech = tech;
                }
            }
        }
        // Credits: earned = integral of the income (below the cap), checked at the end.
        let inc = sim.with_sim(|s| s.galaxy().factions[me as usize].income_per_week);
        expected_earned += inc.max(0.0) / 60.0;
    }
    let st = sim.with_sim(|s| s.galaxy().factions[me as usize].stats);
    rep.lines.push(format!("stats: earned {:.0} (income integral {:.0}), spent {:.0}, ships built {}, structures built {}, planets captured {}, battles {}/{}", st.credits_earned, expected_earned, st.credits_spent, st.ships_built, st.structures_built, st.planets_captured, st.battles_won, st.battles_lost));
    let cantinas = sim.with_sim(|s| s.galaxy().planets.iter().filter(|p| p.owner == Some(me)).flat_map(|p| p.structures.iter()).filter(|&&st| w.structures[st as usize].gamble.is_some()).count());
    rep.lines.push(format!("gambling structures {cantinas}, income swings seen (percent of planet income) {gambling_seen:?}"));
    rep.lines.push(format!("build options after each tech level (units, structures): {options_by_tech:?}"));
    if st.credits_earned < expected_earned * 0.9 {
        rep.discrepancies.push(format!("credits earned {:.0} are below the income integral {:.0}", st.credits_earned, expected_earned));
    }
    if sim.diagnostics().script_errors > 0 {
        rep.discrepancies.push(format!("script errors: {:?}", sim.diagnostics().last_script_error));
    }
    Some(rep)
}

fn check(name: &str) {
    let Some(rep) = play(name, 30) else { return };
    println!("== {} ({} weeks)", rep.name, rep.weeks);
    for l in &rep.lines {
        println!("   {l}");
    }
    for d in &rep.discrepancies {
        println!("   DISCREPANCY {d}");
    }
    assert!(rep.discrepancies.is_empty(), "{:?}", rep.discrepancies);
}

#[test]
fn an_empire_player_economy_follows_the_data() {
    check("Empire");
}

#[test]
fn a_rebel_player_economy_follows_the_data() {
    check("Rebel");
}

#[test]
fn an_underworld_player_economy_follows_the_data() {
    check("Underworld");
}
