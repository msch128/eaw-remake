//! AI-vs-AI scripted match with the Remake galactic AI; skipped when the game or the mod is not installed.

use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

fn build(remake_ai: bool) -> Option<ScriptedSim> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ..SimConfig::default() },
        // Nobody is human: every faction (the campaign's own Rebel included) is played by the AI.
        ScriptedConfig { remake_ai, human: Some("none".into()), game_scoring: false, ..ScriptedConfig::default() },
    )
    .ok()?;
    if !remake_ai {
        sim.with_sim_mut(|s| s.set_ai_factions(0..s.world().factions.len() as u32));
    }
    Some(sim)
}

#[test]
fn remake_ai_builds_moves_and_attacks() {
    let Some(mut sim) = build(true) else { return };
    assert!(sim.start_errors.is_empty(), "{:?}", sim.start_errors);
    let names: Vec<String> = sim.with_sim(|s| s.world().factions.iter().map(|f| f.name.clone()).collect());
    let ships = |sim: &ScriptedSim| sim.with_sim(|s| s.galaxy().fleets.values().map(|f| f.unit_count()).sum::<u32>());
    let ships_at_start = ships(&sim);
    let (mut weeks, mut captures, mut battles, mut week_ms) = (0, 0, 0, Vec::new());
    let mut t = Instant::now();
    while weeks < 20 {
        for e in sim.step(1.0).sim_events {
            match e {
                Event::WeekEnded { week } => {
                    weeks = week;
                    week_ms.push(t.elapsed().as_secs_f64() * 1e3);
                    t = Instant::now();
                }
                Event::PlanetCaptured { .. } => captures += 1,
                Event::BattlePending(_) | Event::BattleResolved { .. } => battles += 1,
                _ => {}
            }
        }
    }
    let st = sim.ai_stats().expect("Remake AI is on");
    let tf = &st.taskforces;
    week_ms.sort_by(f64::total_cmp);
    println!(
        "20 weeks: ships {ships_at_start} -> {}, queued {} ships / {} structures, free-store claims {}, plans {}, captures {captures}, battle events {battles}",
        ships(&sim),
        tf.ships_queued,
        tf.structures_queued,
        tf.free_store_claims,
        st.plans_started.values().sum::<u32>()
    );
    println!("wall time per week: median {:.0} ms, worst {:.0} ms (sim {:?}, scripts {:?}, ai {:?} in total)", week_ms[week_ms.len() / 2], week_ms[week_ms.len() - 1], sim.timing.sim, sim.timing.scripts, sim.timing.ai);

    assert!(tf.ships_queued > 20, "the AI should build ships");
    assert!(tf.structures_queued > 20, "the AI should build structures");
    assert!(captures > 0, "the AI should conquer planets");
    // Every faction, the campaign's own Rebel included, started plans.
    let rebel = names.iter().position(|n| n == "Rebel").unwrap() as u32;
    assert!(st.plans_started.keys().any(|(p, _)| *p == rebel), "Rebel AI never acted");
    assert!(sim.runtime().errors().is_empty());
}

#[test]
fn built_in_ai_stays_the_default() {
    let Some(sim) = build(false) else { return };
    assert!(sim.ai_stats().is_none());
    assert!(!ScriptedConfig::default().remake_ai);
}
