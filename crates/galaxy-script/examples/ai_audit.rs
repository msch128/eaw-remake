//! Audit of the galactic AI after a long all-AI match: what it builds, whether it holds borders and chokepoints, and
//! which hyperspace lanes its fleets use.
//!
//! ```text
//! ai_audit [weeks] [campaign] [seed]
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, FleetLocation, SimConfig};

/// Brandes betweenness on the unweighted lane graph, normalised to the maximum.
fn betweenness(adj: &[Vec<u32>]) -> Vec<f64> {
    let n = adj.len();
    let mut bc = vec![0.0f64; n];
    for s in 0..n {
        let (mut stack, mut pred, mut sigma, mut dist) = (Vec::new(), vec![Vec::new(); n], vec![0.0f64; n], vec![-1i32; n]);
        sigma[s] = 1.0;
        dist[s] = 0;
        let mut q = std::collections::VecDeque::from([s]);
        while let Some(v) = q.pop_front() {
            stack.push(v);
            for &w in &adj[v] {
                let w = w as usize;
                if dist[w] < 0 {
                    dist[w] = dist[v] + 1;
                    q.push_back(w);
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }
        let mut delta = vec![0.0f64; n];
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                delta[v] += sigma[v] / sigma[w] * (1.0 + delta[w]);
            }
            if w != s {
                bc[w] += delta[w];
            }
        }
    }
    let max = bc.iter().copied().fold(1e-9, f64::max);
    bc.iter().map(|b| b / max).collect()
}

/// Weekly samples of one faction, summed.
#[derive(Default, Clone)]
struct Tally {
    border_planets: u32,
    interior_planets: u32,
    choke_planets: u32,
    border_ships: u64,
    interior_ships: u64,
    choke_ships: u64,
    border_defence: u32,
    interior_defence: u32,
    choke_defence: u32,
    border_level: u32,
    interior_level: u32,
    choke_level: u32,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(30);
    let campaign_name = args.get(1).map_or("GC_Full_Web_Way_Rebel", String::as_str);
    let seed: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(1);
    let install = GameInstall::detect().expect("game install not found");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some(&std::env::var("MOD").unwrap_or_else(|_| "1770851727".to_owned()))).expect("cannot open game files"));
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign(campaign_name).expect("campaign not found").clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, seed, ..SimConfig::default() },
        ScriptedConfig { human: Some("none".into()), game_scoring: false, ..ScriptedConfig::default() },
    )
    .expect("sim");
    let (names, adj, routes, real): (Vec<String>, Vec<Vec<u32>>, Vec<f32>, Vec<bool>) = sim.with_sim(|s| {
        let w = s.world();
        (
            w.factions.iter().map(|f| f.name.clone()).collect(),
            w.adjacency.iter().map(|a| a.iter().map(|&(v, _)| v).collect()).collect(),
            w.routes.iter().map(|r| r.hs_factor).collect(),
            w.real.clone(),
        )
    });
    let bc = betweenness(&adj);
    let mut sorted: Vec<f64> = bc.iter().enumerate().filter(|(i, _)| real[*i]).map(|(_, b)| *b).collect();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let choke_cut = sorted[(sorted.len() / 10).max(1) - 1];
    let majors: Vec<usize> = ["Rebel", "Empire", "Underworld"].iter().filter_map(|n| names.iter().position(|x| x == n)).collect();

    let mut tallies = vec![Tally::default(); names.len()];
    let mut lane_use: BTreeMap<i32, u32> = BTreeMap::new();
    let mut border_fleet_weeks_next_to_enemy = 0u32;
    type PendingInfo = (i32, Option<u32>, Vec<(u32, u32)>, u32);
    let mut pending: BTreeMap<u32, PendingInfo> = BTreeMap::new();
    let mut attacks: Vec<(u32, u32, u32, i32, bool)> = Vec::new();
    // Planets per faction at weeks 0/10/20/30..., and escape moves (a fleet that keeps escaping is a loop).
    let mut timeline: Vec<(u64, Vec<u32>)> = vec![(0, sim.with_sim(|s| s.galaxy().factions.iter().map(|f| f.planets_owned).collect()))];
    let mut escape_starts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut escaping_seen: std::collections::BTreeSet<u32> = Default::default();
    let mut escape_fleet_weeks = 0u32;
    let mut week = 0;
    while week < weeks {
        let prev_fleets = sim.with_sim(|s| s.galaxy().fleets.clone());
        for e in sim.step(1.0).sim_events {
            match &e {
                Event::BattlePending(b) => {
                    let rec = sim.with_sim(|s| {
                        let (w, g) = (s.world(), s.galaxy());
                        let ps = &g.planets[b.planet as usize];
                        let level = ps.starbase.map_or(0, |sb| w.starbases[sb as usize].level.max(0));
                        let per: Vec<(u32, u32)> = b.factions.iter().map(|&f| (f, b.fleets.iter().filter_map(|id| prev_fleets.get(id).or_else(|| g.fleets.get(id))).filter(|x| x.owner == f).map(|x| x.unit_count()).sum::<u32>())).collect();
                        (level, ps.owner, per, b.planet)
                    });
                    pending.insert(b.id, rec);
                }
                Event::BattleResolved { id, winner, .. } => {
                    if let Some((level, owner, per, planet)) = pending.remove(id) {
                        for &(att, n) in per.iter().filter(|(f, _)| Some(*f) != owner) {
                            if !majors.contains(&(att as usize)) {
                                continue;
                            }
                            let def: u32 = per.iter().filter(|(f, _)| *f != att).map(|x| x.1).sum();
                            attacks.push((att, n, def, level, *winner == Some(att)));
                            if attacks.len() <= 40 && std::env::var("TRACE").is_ok() {
                                let (pn, on) = sim.with_sim(|s| (s.world().planets[planet as usize].name.clone(), owner.map(|o| s.world().factions[o as usize].name.clone())));
                                println!("  week {week}: {} attacks {pn} (owner {on:?}) with {n} vs {def} ships, star base level {level}, won {}", names[att as usize], *winner == Some(att));
                            }
                        }
                    }
                }
                _ => {}
            }
            let Event::WeekEnded { week: w } = e else { continue };
            week = w;
            if week % 10 == 0 {
                timeline.push((week, sim.with_sim(|s| s.galaxy().factions.iter().map(|f| f.planets_owned).collect())));
            }
            sim.with_sim(|s| {
                let esc = &s.galaxy().escaping;
                escape_fleet_weeks += esc.len() as u32;
                for &id in esc {
                    if escaping_seen.insert(id) {
                        let owner = s.galaxy().fleets.get(&id).map_or(u32::MAX, |f| f.owner);
                        *escape_starts.entry(owner).or_default() += 1;
                    }
                }
                escaping_seen.retain(|id| esc.contains(id));
            });
            if week % 5 == 0 {
                sim.with_sim(|s| {
                    let g = s.galaxy();
                    let line: Vec<String> = majors.iter().map(|&f| format!("{} {:.0}c pop {}/{} planets {} q{}", &names[f][..3], g.factions[f].credits, g.factions[f].population_used, g.factions[f].population_cap, g.factions[f].planets_owned, g.planets.iter().filter(|p| p.owner == Some(f as u32)).map(|p| p.queues.iter().map(|q| q.len()).sum::<usize>()).sum::<usize>())).collect();
                    println!("week {week}: {}", line.join(" | "));
                });
            }
            sim.with_sim(|s| {
                let (w, g) = (s.world(), s.galaxy());
                for &f in &majors {
                    let t = &mut tallies[f];
                    for p in (0..w.planets.len()).filter(|&p| real[p] && g.planets[p].owner == Some(f as u32)) {
                        let border = adj[p].iter().any(|&n| g.planets[n as usize].owner != Some(f as u32));
                        let choke = bc[p] >= choke_cut;
                        let ships: u32 = g.fleets.values().filter(|x| x.owner == f as u32 && x.location == FleetLocation::AtPlanet(p as u32)).map(|x| x.unit_count()).sum();
                        let defence = g.planets[p]
                            .structures
                            .iter()
                            .filter(|&&st| {
                                let n = w.structures[st as usize].name.to_ascii_lowercase();
                                ["golan", "tower", "defence", "defense", "satellite", "grid", "turret", "shield"].iter().any(|k| n.contains(k))
                            })
                            .count() as u32;
                        let level = g.planets[p].starbase.map_or(0, |sb| w.starbases[sb as usize].level.max(0) as u32);
                        if border {
                            t.border_planets += 1;
                            t.border_ships += u64::from(ships);
                            t.border_defence += defence;
                            t.border_level += level;
                        } else {
                            t.interior_planets += 1;
                            t.interior_ships += u64::from(ships);
                            t.interior_defence += defence;
                            t.interior_level += level;
                        }
                        if choke {
                            t.choke_planets += 1;
                            t.choke_ships += u64::from(ships);
                            t.choke_defence += defence;
                            t.choke_level += level;
                        }
                        if border && ships > 0 && adj[p].iter().any(|&n| g.planets[n as usize].owner.is_some_and(|o| o != f as u32 && g.hostile(w, f as u32, o))) {
                            border_fleet_weeks_next_to_enemy += 1;
                        }
                    }
                }
                for fl in g.fleets.values() {
                    if let FleetLocation::Moving { route, .. } = fl.location
                        && majors.contains(&(fl.owner as usize))
                    {
                        *lane_use.entry((routes[route as usize] * 100.0) as i32).or_default() += 1;
                    }
                }
            });
        }
    }
    println!("== {weeks} weeks, {campaign_name}, seed {seed}; chokepoint = top 10% betweenness (cut {choke_cut:.2})");
    sim.with_sim(|s| {
        let (w, g) = (s.world(), s.galaxy());
        for &f in &majors {
            let t = &tallies[f];
            let avg = |sum: u64, n: u32| if n == 0 { 0.0 } else { sum as f64 / f64::from(n) };
            let owned: Vec<usize> = (0..w.planets.len()).filter(|&p| g.planets[p].owner == Some(f as u32)).collect();
            let mut structs: BTreeMap<String, u32> = BTreeMap::new();
            let mut levels = [0u32; 5];
            for &p in &owned {
                for &st in &g.planets[p].structures {
                    *structs.entry(w.structures[st as usize].name.clone()).or_default() += 1;
                }
                levels[g.planets[p].starbase.map_or(0, |sb| w.starbases[sb as usize].level.clamp(0, 4) as usize)] += 1;
            }
            let mut top: Vec<_> = structs.into_iter().collect();
            top.sort_by_key(|x| std::cmp::Reverse(x.1));
            println!(
                "{:11} planets {:3}, credits {:.0}; star base levels 0..4 {levels:?}; {} distinct structure types, {} built; top {:?}",
                names[f],
                owned.len(),
                g.factions[f].credits,
                top.len(),
                top.iter().map(|x| x.1).sum::<u32>(),
                top.iter().take(5).collect::<Vec<_>>()
            );
            let queued: usize = owned.iter().map(|&p| g.planets[p].queues.iter().map(|q| q.len()).sum::<usize>()).sum();
            let fs = &g.factions[f];
            println!("{:11} pop {}/{}, tech {}, income {:.0}/wk, queued items {queued}, ships built {} lost {}, structures built {}, battles won {} lost {}, credits earned {:.0} spent {:.0} ({:.0}%)", "", fs.population_used, fs.population_cap, fs.tech, fs.income_per_week, fs.stats.ships_built, fs.stats.ships_lost, fs.stats.structures_built, fs.stats.battles_won, fs.stats.battles_lost, fs.stats.credits_earned, fs.stats.credits_spent, 100.0 * fs.stats.credits_spent / fs.stats.credits_earned.max(1.0));
            println!(
                "{:11} per planet-week: ships border {:.1} / interior {:.1} / choke {:.1}; defence structures border {:.2} / interior {:.2} / choke {:.2}; star base level border {:.2} / interior {:.2} / choke {:.2}; planet share border {:.0}% choke {:.0}%",
                "",
                avg(t.border_ships, t.border_planets),
                avg(t.interior_ships, t.interior_planets),
                avg(t.choke_ships, t.choke_planets),
                avg(u64::from(t.border_defence), t.border_planets),
                avg(u64::from(t.interior_defence), t.interior_planets),
                avg(u64::from(t.choke_defence), t.choke_planets),
                avg(u64::from(t.border_level), t.border_planets),
                avg(u64::from(t.interior_level), t.interior_planets),
                avg(u64::from(t.choke_level), t.choke_planets),
                100.0 * f64::from(t.border_planets) / f64::from((t.border_planets + t.interior_planets).max(1)),
                100.0 * f64::from(t.choke_planets) / f64::from((t.border_planets + t.interior_planets).max(1))
            );
        }
    });
    if let Some(st) = sim.ai_stats() {
        for (pi, name) in names.iter().enumerate().filter(|(i, _)| majors.contains(i)) {
            let mut v: Vec<(String, u32, u32, u32)> = st
                .plans_started
                .iter()
                .filter(|((p, _), _)| *p as usize == pi)
                .map(|((_, goal), n)| (goal.clone(), *n, st.plans_succeeded.get(&(pi as u32, goal.clone())).copied().unwrap_or(0), st.plans_failed.get(&(pi as u32, goal.clone())).copied().unwrap_or(0)))
                .collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            println!("{name} plans (goal, started, ok, failed): {:?}", v.iter().take(80).collect::<Vec<_>>());
        }
        for (goal, t) in st.plan_timing.iter().filter(|(g, _)| g.contains("Conquer") || g.contains("Build_Units")) {
            println!("plan timing {goal}: {} plans, mean {:.0} s, {} near their limit, {} failed ({} within 30 s)", t.plans, t.seconds / f64::from(t.plans.max(1)), t.near_limit, t.failed, t.failed_fast);
        }
        let mut causes: Vec<_> = st.fast_failures.iter().filter(|((g, _), _)| g.contains("Conquer_Enemy_Planet")).collect();
        causes.sort_by_key(|x| std::cmp::Reverse(*x.1));
        for ((g, why), n) in causes.iter().take(14) {
            println!("fast failure {g}: {n} x {why}");
        }
        println!("goals without a plan script: {:?}", st.goals_without_plan);
        println!("task forces: {:?}", st.taskforces);
    }
    if let Some(mem) = sim.ai_memory() {
        for (pi, name) in names.iter().enumerate().filter(|(i, _)| majors.contains(i)) {
            let frac: Vec<String> = mem.brains.iter().filter(|b| b.player as usize == pi).flat_map(|b| b.fractions.iter().map(|(c, f)| format!("{c} {:.0}%", f * 100.0))).collect();
            let acc: Vec<String> = mem.task_forces.accounts.iter().filter(|a| a.0 as usize == pi).map(|a| format!("{} {:.0}", a.1, a.2)).collect();
            println!("{name} budget shares [{}]; unspent earmarked per category [{}]", frac.join(", "), acc.join(", "));
        }
    }
    for &f in &majors {
        let mine: Vec<_> = attacks.iter().filter(|a| a.0 as usize == f).collect();
        let wins = mine.iter().filter(|a| a.4).count();
        println!("{} attacks on enemy planets: {} battles, {wins} won", names[f], mine.len());
        for (label, lo, hi) in [("attackers outnumbered (<0.7x)", 0.0, 0.7), ("even (0.7-1.5x)", 0.7, 1.5), ("superior (1.5-3x)", 1.5, 3.0), ("overwhelming (>3x)", 3.0, 1e9), ("no defending ships", -1.0, -0.5)] {
            let sel: Vec<_> = mine.iter().filter(|a| if hi < 0.0 { a.2 == 0 } else { a.2 > 0 && (f64::from(a.1) / f64::from(a.2)) >= lo && (f64::from(a.1) / f64::from(a.2)) < hi }).collect();
            if !sel.is_empty() {
                println!("    {label}: {} battles, {} won; with a star base {} ({} won)", sel.len(), sel.iter().filter(|a| a.4).count(), sel.iter().filter(|a| a.3 > 0).count(), sel.iter().filter(|a| a.3 > 0 && a.4).count());
            }
        }
    }
    let total: u32 = lane_use.values().sum();
    let lane_share = |f: i32| routes.iter().filter(|&&r| (r * 100.0) as i32 == f).count() as f64 / routes.len() as f64;
    println!("fleet-weeks in hyperspace by lane speed (x100): {lane_use:?} of {total}");
    for (f, n) in &lane_use {
        println!("  {f}: {:.0}% of fleet-weeks vs {:.0}% of lanes", 100.0 * f64::from(*n) / f64::from(total.max(1)), 100.0 * lane_share(*f));
    }
    println!("AUDIT planets by week (factions {names:?}):");
    for (w, v) in &timeline {
        println!("AUDIT   week {w}: {v:?}");
    }
    println!("AUDIT attacks launched/won per major: {:?}", majors.iter().map(|&f| (names[f].clone(), attacks.iter().filter(|a| a.0 as usize == f).count(), attacks.iter().filter(|a| a.0 as usize == f && a.4).count())).collect::<Vec<_>>());
    println!("AUDIT escape moves started per faction index {escape_starts:?}, fleet-weeks escaping {escape_fleet_weeks}");
    println!("border planet-weeks with ships next to a hostile planet: {border_fleet_weeks_next_to_enemy}");
}
