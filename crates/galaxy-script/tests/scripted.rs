//! Scripted match against the real Remake data; skipped when the game or the mod is not installed.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptEvent, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

fn build() -> Option<ScriptedSim> {
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
        SimConfig {
            victory_enabled: false,
            ..SimConfig::default()
        },
        ScriptedConfig { remake_ai: false, difficulty: None, ..ScriptedConfig::default() },
    )
    .ok()?;
    sim.with_sim_mut(|s| s.set_ai_factions(0..s.world().factions.len() as u32));
    Some(sim)
}

fn run_weeks(sim: &mut ScriptedSim, weeks: u64) -> Vec<ScriptEvent> {
    let mut out = Vec::new();
    let mut done = 0;
    while done < weeks {
        let r = sim.step(1.0);
        for e in &r.sim_events {
            if let Event::WeekEnded { week } = e {
                done = *week;
            }
        }
        out.extend(r.script_events);
    }
    out
}

#[test]
fn rebel_campaign_scripts_run_clean_for_ten_weeks() {
    let Some(mut sim) = build() else { return };
    assert!(sim.start_errors.is_empty(), "{:?}", sim.start_errors);
    let events = run_weeks(&mut sim, 10);
    let rt = sim.runtime();
    for e in rt.errors() {
        eprintln!("ERROR {}: {}", e.script, e.message);
    }
    assert!(rt.errors().is_empty());
    assert!(
        rt.story_fired() >= 10,
        "story events fired: {}",
        rt.story_fired()
    );
    let stats = sim.host_stats();
    eprintln!("{stats:?}");
    assert!(stats.spawned > 0, "starting forces were spawned");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ScriptEvent::Dialog { .. })),
        "intro dialog reached the app"
    );
    let hits = rt.api_hits();
    let total: usize = hits.values().sum();
    let unimpl: usize = sim.unimplemented_calls().values().sum();
    assert!(
        unimpl * 100 < total,
        "unimplemented calls {unimpl} of {total}"
    );
}

#[test]
fn scripted_match_is_deterministic() {
    let (Some(mut a), Some(mut b)) = (build(), build()) else {
        return;
    };
    run_weeks(&mut a, 3);
    run_weeks(&mut b, 3);
    let ha = a.with_sim(|s| s.galaxy().state_hash());
    let hb = b.with_sim(|s| s.galaxy().state_hash());
    assert_eq!(ha, hb);
}
