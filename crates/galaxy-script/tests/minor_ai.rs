//! Every non-pirate faction plays under the Remake AI: it spends credits and builds ships within 20 weeks (skipped without the game and mod).

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

#[test]
fn every_non_pirate_faction_spends_and_builds() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some(std::env::var("SP_TEST_HUMAN").unwrap_or_else(|_| "none".into())), game_scoring: false, ..ScriptedConfig::default() },
    )
    .unwrap();
    assert!(sim.start_errors.is_empty(), "{:?}", sim.start_errors);
    let mut weeks = 0;
    while weeks < 20 {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week } = e {
                weeks = week;
            }
        }
    }
    let mut idle = Vec::new();
    sim.with_sim(|s| {
        let w = s.world();
        for (i, f) in w.factions.iter().enumerate() {
            let st = &s.galaxy().factions[i];
            let owned = s.galaxy().owned_planets(i as u32).count();
            println!("{:14} planets {owned:3} spent {:9.0} ships built {:3} structures {:3} credits {:.0}", f.name, st.stats.credits_spent, st.stats.ships_built, st.stats.structures_built, st.credits);
            let passive = ["neutral", "pirates", "hostile"].iter().any(|n| f.name.eq_ignore_ascii_case(n));
            if !passive && owned > 0 && st.stats.credits_spent <= 0.0 {
                idle.push(f.name.clone());
            }
        }
    });
    assert!(idle.is_empty(), "factions that spent nothing in 20 weeks: {idle:?}");
}
