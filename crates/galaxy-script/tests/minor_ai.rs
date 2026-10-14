//! Every non-pirate faction plays under the Remake AI: it spends credits and builds ships within 20 weeks (skipped without the game and mod).

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

#[test]
fn every_non_pirate_faction_spends_and_builds() {
    // Once with every faction under the AI, once with a human Rebel: the minors (Republic, Warlords ...) must build either way.
    for human in ["none", "Rebel"] {
        play(human);
    }
}

fn play(human: &str) {
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
        fs.clone(),
        data,
        &campaign,
        SimConfig { victory_enabled: false, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some(human.to_owned()), game_scoring: false, ..ScriptedConfig::default() },
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
    let mut no_ships = Vec::new();
    sim.with_sim(|s| {
        let w = s.world();
        for (i, f) in w.factions.iter().enumerate() {
            let st = &s.galaxy().factions[i];
            let owned = s.galaxy().owned_planets(i as u32).count();
            let passive = ["neutral", "pirates", "hostile"].iter().any(|n| f.name.eq_ignore_ascii_case(n));
            let is_human = f.name.eq_ignore_ascii_case(human) || (human == "Rebel" && f.name.eq_ignore_ascii_case("Rebellion"));
            eprintln!("human={human} {}: planets {owned}, spent {:.0}, ships built {}", f.name, st.stats.credits_spent, st.stats.ships_built);
            if !passive && !is_human && owned > 0 {
                if st.stats.credits_spent <= 0.0 {
                    idle.push(f.name.clone());
                }
                if st.stats.ships_built == 0 {
                    no_ships.push(f.name.clone());
                }
            }
        }
    });
    assert!(idle.is_empty(), "human {human}: factions that spent nothing in 20 weeks: {idle:?}");
    assert!(no_ships.is_empty(), "human {human}: factions that built no ships in 20 weeks: {no_ships:?}");
}
