//! Headless galactic conquest where every faction is played by the Remake AI (or, for comparison, by
//! galaxy-sim's built-in AI).
//!
//! ```text
//! ai_match [weeks] [campaign] [stub] [human=<faction>]
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(20);
    let campaign_name = args
        .get(1)
        .filter(|a| !a.contains('=') && a.as_str() != "stub")
        .map_or("GC_Full_Web_Way_Rebel", String::as_str);
    let stub = args.iter().any(|a| a == "stub");
    let human = args.iter().find_map(|a| a.strip_prefix("human=")).unwrap_or("none").to_string();

    let install = GameInstall::detect().expect("game install not found");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files"));
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign(campaign_name).expect("campaign not found").clone();

    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ..SimConfig::default() },
        ScriptedConfig { remake_ai: !stub, human: Some(human), game_scoring: false, ..ScriptedConfig::default() },
    )
    .expect("cannot build simulation");
    if stub {
        sim.with_sim_mut(|s| s.set_ai_factions(0..s.world().factions.len() as u32));
    }
    println!("{} AI, {weeks} weeks, campaign {campaign_name}; start errors {:?}", if stub { "built-in" } else { "Remake" }, sim.start_errors);
    let names: Vec<String> = sim.with_sim(|s| s.world().factions.iter().map(|f| f.name.clone()).collect());

    let summary = |sim: &ScriptedSim, label: &str| {
        let line = sim.with_sim(|s| {
            let g = s.galaxy();
            let ships: Vec<u32> = (0..names.len() as u32).map(|f| g.fleets.values().filter(|x| x.owner == f).map(|x| x.unit_count()).sum()).collect();
            names
                .iter()
                .enumerate()
                .filter(|(i, _)| g.factions[*i].planets_owned > 0 || !g.factions[*i].eliminated)
                .map(|(i, n)| format!("{}:{}p/{}s/{:.0}k", &n[..n.len().min(4)], g.factions[i].planets_owned, ships[i], g.factions[i].credits / 1000.0))
                .collect::<Vec<_>>()
                .join("  ")
        });
        println!("{label:<8} {line}");
    };
    summary(&sim, "start");

    let mut weeks_done = 0;
    let mut per_week: Vec<(f64, f64, f64)> = Vec::new();
    let mut last = sim.timing;
    let mut captures: BTreeMap<String, u32> = BTreeMap::new();
    let mut battles = 0;
    while weeks_done < weeks {
        let r = sim.step(1.0);
        for e in &r.sim_events {
            match e {
                Event::WeekEnded { week } => {
                    weeks_done = *week;
                    let t = sim.timing;
                    per_week.push(((t.sim - last.sim).as_secs_f64() * 1e3, (t.scripts - last.scripts).as_secs_f64() * 1e3, (t.ai - last.ai).as_secs_f64() * 1e3));
                    last = t;
                    if week % 10 == 0 {
                        summary(&sim, &format!("week {week}"));
                    }
                }
                Event::PlanetCaptured { new_owner, .. } => *captures.entry(names[*new_owner as usize].clone()).or_default() += 1,
                Event::BattleResolved { .. } => battles += 1,
                _ => {}
            }
        }
    }
    summary(&sim, "final");
    println!("battles {battles}, captures by faction {captures:?}");

    let n = per_week.len().max(1) as f64;
    let (s, c, a) = per_week.iter().fold((0.0, 0.0, 0.0), |x, w| (x.0 + w.0, x.1 + w.1, x.2 + w.2));
    let worst = per_week.iter().map(|w| w.1 + w.2).fold(0.0, f64::max);
    println!("per week: sim {:.2} ms, plan/story scripts {:.2} ms, AI {:.2} ms (worst script+AI week {:.1} ms)", s / n, c / n, a / n, worst);

    let rt = sim.runtime();
    println!("\nlua scripts alive {}, errors {} (distinct {}), plan start errors {}", rt.alive_scripts(), rt.errors().len(), rt.errors().iter().map(|e| e.message.lines().next().unwrap_or("").to_string()).collect::<std::collections::BTreeSet<_>>().len(), sim.ai_errors.len());
    let mut seen = std::collections::BTreeSet::new();
    for e in rt.errors() {
        let first = format!("{}: {}", e.script, e.message.lines().next().unwrap_or(""));
        if seen.insert(first.clone()) && seen.len() <= 12 {
            println!("  {first}");
        }
    }
    for e in sim.ai_errors.iter().take(5) {
        println!("  plan start error: {e}");
    }
    let un = sim.unimplemented_calls();
    println!("unimplemented API: {un:?}");

    if let Ok(filter) = std::env::var("AI_LOG") {
        for l in sim.script_log().iter().filter(|l| l.contains(&filter)).take(150) {
            println!("  log: {l}");
        }
    }
    if let Some(st) = sim.ai_stats() {
        println!("\n== Remake AI: {} evaluations, {:.1} ms in goal evaluation", st.evaluations, st.eval_nanos as f64 / 1e6);
        let mut per: BTreeMap<u32, Vec<(String, u32, u32, u32)>> = BTreeMap::new();
        for ((p, g), n) in &st.plans_started {
            let ok = st.plans_succeeded.get(&(*p, g.clone())).copied().unwrap_or(0);
            let bad = st.plans_failed.get(&(*p, g.clone())).copied().unwrap_or(0);
            per.entry(*p).or_default().push((g.clone(), *n, ok, bad));
        }
        for (p, mut goals) in per {
            goals.sort_by_key(|g| std::cmp::Reverse(g.1));
            let total: u32 = goals.iter().map(|g| g.1).sum();
            println!("  {} started {total} plans:", names[p as usize]);
            for (g, n, ok, bad) in goals.iter().take(8) {
                println!("      {g:<32} started {n:>4} ok {ok:>4} failed {bad:>4}");
            }
        }
        let tf = &st.taskforces;
        println!(
            "  task forces {:?}\n  free-store claims {}, ships queued {}, structures queued {}, production failures {}, moves {}, arrivals {}, forces lost {}, credits spent {:.0}",
            tf.created, tf.free_store_claims, tf.ships_queued, tf.structures_queued, tf.production_failures, tf.moves, tf.arrivals, tf.forces_lost, tf.credits_spent
        );
        println!("  unimplemented task force methods {:?}", tf.unimplemented);
        println!("  ai cost: goal evaluation {:.0} ms, task force ticks {:.0} ms, snapshots {:.0} ms", st.eval_nanos as f64 / 1e6, st.tick_nanos as f64 / 1e6, st.snapshot_nanos as f64 / 1e6);
        let mut why: Vec<_> = tf.failure_reasons.iter().collect();
        why.sort_by_key(|w| std::cmp::Reverse(*w.1));
        println!("  production failure reasons {:?}", &why[..why.len().min(12)]);
        let mut uv: Vec<_> = st.eval.unknown_variables.iter().collect();
        uv.sort_by_key(|x| std::cmp::Reverse(*x.1));
        println!("  unknown variables {:?}", &uv[..uv.len().min(12)]);
        println!("  missing equations {:?}", st.eval.missing_equations);
        println!("  goals with desire but no plan {:?}", st.goals_without_plan);
    }
}