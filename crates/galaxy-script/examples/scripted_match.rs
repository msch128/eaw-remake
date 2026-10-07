//! Headless galactic conquest with the campaign's Lua scripts active.
//!
//! ```text
//! scripted_match [weeks] [campaign] [freestore]
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptEvent, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(20);
    let campaign_name = args
        .get(1)
        .filter(|a| !matches!(a.as_str(), "freestore" | "no-backoff"))
        .map_or("GC_Full_Web_Way_Rebel", String::as_str);
    let freestore = args.iter().any(|a| a == "freestore");
    let no_backoff = args.iter().any(|a| a == "no-backoff");

    let install = GameInstall::detect().expect("game install not found");
    let fs: Arc<dyn FileSource> =
        Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files"));
    let t = Instant::now();
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data
        .campaign(campaign_name)
        .expect("campaign not found")
        .clone();
    println!("data loaded in {:?}", t.elapsed());

    let t = Instant::now();
    let mut sim = ScriptedSim::new(
        fs,
        data.clone(),
        &campaign,
        SimConfig {
            victory_enabled: false,
            ..SimConfig::default()
        },
        ScriptedConfig {
            freestore,
            ..ScriptedConfig::default()
        },
    )
    .expect("cannot build simulation");
    if no_backoff {
        sim.runtime_mut().set_idle_backoff(0.0);
    }
    sim.with_sim_mut(|s| s.set_ai_factions(0..s.world().factions.len() as u32));
    println!(
        "scripted sim built in {:?}; start errors: {:?}",
        t.elapsed(),
        sim.start_errors
    );

    let mut week_wall = Instant::now();
    let mut per_week: Vec<(f64, f64)> = Vec::new();
    let mut last = sim.timing;
    let mut events: Vec<ScriptEvent> = Vec::new();
    let mut weeks_done = 0;
    let mut captures = 0;
    while weeks_done < weeks {
        let report = sim.step(1.0);
        for e in &report.sim_events {
            match e {
                Event::WeekEnded { week } => {
                    weeks_done = *week;
                    let now = sim.timing;
                    per_week.push((
                        (now.sim - last.sim).as_secs_f64() * 1000.0,
                        (now.scripts - last.scripts).as_secs_f64() * 1000.0,
                    ));
                    last = now;
                    week_wall = Instant::now();
                }
                Event::PlanetCaptured { .. } => captures += 1,
                _ => {}
            }
        }
        events.extend(report.script_events);
    }
    let _ = week_wall;

    let rt = sim.runtime();
    println!(
        "\n== {weeks} weeks, {captures} captures, {} scripts alive, {} story events fired",
        rt.alive_scripts(),
        rt.story_fired()
    );
    let (s, c): (f64, f64) = per_week
        .iter()
        .fold((0.0, 0.0), |a, w| (a.0 + w.0, a.1 + w.1));
    let n = per_week.len().max(1) as f64;
    let worst = per_week.iter().map(|w| w.1).fold(0.0, f64::max);
    println!(
        "per week: sim {:.2} ms, scripts {:.2} ms (worst week {:.1} ms scripts)",
        s / n,
        c / n,
        worst
    );

    let series: Vec<String> = per_week
        .iter()
        .take(8)
        .map(|w| format!("{:.1}", w.1))
        .collect();
    println!("script ms of the first weeks: {}", series.join(" "));
    let steady: Vec<f64> = per_week.iter().skip(1).map(|w| w.1).collect();
    println!(
        "steady-state scripts (weeks 2..): {:.2} ms/week",
        steady.iter().sum::<f64>() / steady.len().max(1) as f64
    );

    for (name, d, n) in rt.profile() {
        println!(
            "  script {name:<40} {:>8.1} ms over {n} services",
            d.as_secs_f64() * 1000.0
        );
    }

    let mut sorted = steady.clone();
    sorted.sort_by(f64::total_cmp);
    if !sorted.is_empty() {
        println!(
            "median week {:.2} ms, fastest week {:.2} ms",
            sorted[sorted.len() / 2],
            sorted[0]
        );
    }

    println!("\nerrors ({}):", rt.errors().len());
    for e in rt.errors().iter().take(10) {
        println!("  {}: {}", e.script, e.message.lines().next().unwrap_or(""));
    }
    let stats = sim.host_stats();
    println!(
        "\nscript effects: spawn requests {}, ships {}, structures {}, credits given {:.0}",
        stats.spawn_requests, stats.spawned, stats.structures_added, stats.credits_given
    );
    println!("  spawn outcomes: {:?}", stats.spawn_outcomes);
    println!("  rewards applied: {:?}", stats.rewards_applied);
    println!("  rewards ignored: {:?}", stats.rewards_ignored);
    println!(
        "  spawn requests ignored (no sim representation): {:?}",
        stats.spawn_ignored.iter().take(8).collect::<Vec<_>>()
    );

    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &events {
        let k = match e {
            ScriptEvent::Message(_) => "Message",
            ScriptEvent::Dialog { .. } => "Dialog",
            ScriptEvent::UnitSpawned { .. } => "UnitSpawned",
            ScriptEvent::BuildableChanged { .. } => "BuildableChanged",
            ScriptEvent::BuildMenuFilter { .. } => "BuildMenuFilter",
            ScriptEvent::PlanetChanged { .. } => "PlanetChanged",
            ScriptEvent::UnhandledReward(_) => "UnhandledReward",
        };
        *kinds.entry(k).or_default() += 1;
    }
    println!("\nscript events: {kinds:?}");
    for e in events
        .iter()
        .filter(|e| {
            matches!(
                e,
                ScriptEvent::Dialog { .. } | ScriptEvent::UnitSpawned { .. }
            )
        })
        .take(6)
    {
        println!("  {e:?}");
    }

    let unimpl = sim.unimplemented_calls();
    let hits = rt.api_hits();
    let total: usize = hits.values().sum();
    let missing: usize = unimpl.values().sum();
    println!(
        "\nAPI: {} distinct functions used ({} calls), {} unimplemented ({} calls, {:.2}%)",
        hits.len(),
        total,
        unimpl.len(),
        missing,
        100.0 * missing as f64 / total.max(1) as f64
    );
    let mut u: Vec<_> = unimpl.iter().collect();
    u.sort_by_key(|x| std::cmp::Reverse(*x.1));
    for (name, n) in u.iter().take(25) {
        println!("  {n:>6}  {name}");
    }
    let mut top: Vec<_> = hits.iter().collect();
    top.sort_by_key(|x| std::cmp::Reverse(*x.1));
    println!("top API calls: {:?}", &top[..top.len().min(12)]);
}
