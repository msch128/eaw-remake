//! Save/load round trip against the real data: a loaded game continues exactly like the original.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{DifficultyTier, SaveError, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Event, SimConfig};

struct Ctx {
    fs: Arc<dyn FileSource>,
    data: Arc<GameData>,
}

fn ctx() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    Some(Ctx { fs, data: Arc::new(data) })
}

fn new_sim(c: &Ctx, config: ScriptedConfig) -> ScriptedSim {
    let campaign = c.data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    ScriptedSim::new(c.fs.clone(), c.data.clone(), &campaign, SimConfig { victory_enabled: false, seed: 5, ..SimConfig::default() }, config).unwrap()
}

fn weeks(sim: &mut ScriptedSim, n: u64) {
    let start = sim.checkpoint().week;
    while sim.checkpoint().week < start + n {
        let r = sim.step(1.0);
        let _ = r.sim_events.iter().any(|e| matches!(e, Event::WeekEnded { .. }));
    }
}

fn ai_config() -> ScriptedConfig {
    ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: true, difficulty: Some(DifficultyTier::Normal), ..ScriptedConfig::default() }
}

#[test]
fn loaded_game_continues_identically() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ai_config());
    weeks(&mut sim, 6);
    // A player command in the middle (journaled); whether it is accepted does not matter, the replay must agree.
    let w = sim.with_sim(|s| s.world().clone());
    let empire = w.faction_id("Empire").unwrap();
    let home = w.planet_id("Coruscant_WW").unwrap();
    let unit = w.unit_id("TIE_Fighter_Squadron").unwrap();
    let r1 = sim.apply(Command::BuildUnit { faction: empire, planet: home, unit });
    weeks(&mut sim, 2);
    let path = std::env::temp_dir().join("space_peace_test_save.json");
    sim.save(&path).expect("save");
    println!("save file: {} bytes at week {}", std::fs::metadata(&path).unwrap().len(), sim.checkpoint().week);

    // Original keeps going.
    weeks(&mut sim, 5);
    let original = (sim.checkpoint(), sim.ai_stats().unwrap().taskforces.ships_queued, sim.ai_stats().unwrap().plans_started.values().sum::<u32>());

    // Loaded copy: same position after loading, same continuation after 5 more weeks.
    let t = std::time::Instant::now();
    let mut loaded = ScriptedSim::load(&path, c.fs.clone(), c.data.clone()).expect("load");
    println!("load (replay of 8 weeks) took {:?}", t.elapsed());
    assert_eq!(loaded.checkpoint().week, 8);
    weeks(&mut loaded, 5);
    let again = (loaded.checkpoint(), loaded.ai_stats().unwrap().taskforces.ships_queued, loaded.ai_stats().unwrap().plans_started.values().sum::<u32>());
    assert_eq!(original, again, "a loaded game must continue exactly like the original (command result {r1:?})");
    // Story/script state agrees as well.
    assert_eq!(sim.script_log().len(), loaded.script_log().len());
    // A loaded game can be saved again.
    loaded.save(&path).expect("resave");
}

#[test]
fn bad_or_unsafe_saves_are_rejected() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ScriptedConfig { game_scoring: false, ..ai_config() });
    weeks(&mut sim, 2);
    let mut file = sim.to_save().unwrap();

    let mut bad = file.clone();
    bad.version = 99;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Version(99))));

    let mut bad = file.clone();
    bad.checkpoint.galaxy_hash ^= 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Diverged { .. })));

    let mut bad = file.clone();
    bad.header.world[0] += 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::DataMismatch)));

    file.header.campaign = "nope".into();
    assert!(matches!(ScriptedSim::from_save(file, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::UnknownCampaign(_))));

    // Unjournaled changes make the game unsaveable.
    sim.with_sim_mut(|s| s.add_credits(0, 1.0));
    assert!(matches!(sim.to_save(), Err(SaveError::UnjournaledChanges)));
}
