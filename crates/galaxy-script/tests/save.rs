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
fn loaded_game_equals_the_saved_state_and_plays_on() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ai_config());
    weeks(&mut sim, 6);
    // A player command in the middle; whether it is accepted does not matter.
    let w = sim.with_sim(|s| s.world().clone());
    let empire = w.faction_id("Empire").unwrap();
    let _ = sim.apply(Command::BuildUnit { faction: empire, planet: w.planet_id("Coruscant_WW").unwrap(), unit: w.unit_id("TIE_Fighter_Squadron").unwrap() });
    weeks(&mut sim, 24);
    let path = std::env::temp_dir().join("space_peace_test_save.json");
    sim.save(&path).expect("save");
    let size = std::fs::metadata(&path).unwrap().len();

    let t = std::time::Instant::now();
    let mut loaded = ScriptedSim::load(&path, c.fs.clone(), c.data.clone()).expect("load");
    println!("save {} KB, load took {:?}, warnings {:?}", size / 1024, t.elapsed(), loaded.load_warnings);
    // The loaded state is the saved state.
    let (a, b) = (sim.state(), loaded.state());
    assert_eq!(a.sim, b.sim, "galaxy-sim state");
    assert_eq!(a.runtime.story.plots, b.runtime.story.plots, "story plots");
    assert_eq!(a.runtime.global_values, b.runtime.global_values, "global values");
    assert_eq!(a.runtime.scripts, b.runtime.scripts, "script globals");
    println!("saved: {} story plots, {} global values, {} scripts with changed globals ({} values)", a.runtime.story.plots.len(), a.runtime.global_values.len(), a.runtime.scripts.len(), a.runtime.scripts.iter().map(|s| s.globals.len()).sum::<usize>());
    assert_eq!(a.ai.as_ref().map(|m| (m.brains.clone(), m.task_forces.clone(), m.strategy.clone())), b.ai.as_ref().map(|m| (m.brains.clone(), m.task_forces.clone(), m.strategy.clone())), "AI memory");
    assert_eq!(sim.checkpoint(), loaded.checkpoint());
    assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);

    // It plays on for 20 weeks without new errors; the original does too. (A continuation is not bit-identical: plans in flight are not saved.)
    let errors_before = loaded.diagnostics().script_errors;
    weeks(&mut loaded, 20);
    weeks(&mut sim, 20);
    assert_eq!(loaded.diagnostics().script_errors, errors_before, "{:?}", loaded.diagnostics().last_script_error);
    let (cl, co) = (loaded.checkpoint(), sim.checkpoint());
    assert_eq!(cl.week, co.week);
    println!("continuation identical to the original: {} (loaded {:x}, original {:x})", cl.galaxy_hash == co.galaxy_hash, cl.galaxy_hash, co.galaxy_hash);
    // A loaded game can be saved again.
    loaded.save(&path).expect("resave");
}

#[test]
fn bad_saves_are_rejected() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ScriptedConfig { game_scoring: false, ..ai_config() });
    weeks(&mut sim, 2);
    let mut file = sim.to_save().unwrap();

    let mut bad = file.clone();
    bad.version = 99;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Version(99))));

    let mut bad = file.clone();
    bad.checkpoint.galaxy_hash ^= 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Corrupt { .. })));

    let mut bad = file.clone();
    bad.header.world[0] += 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::DataMismatch)));

    let mut bad = file.clone();
    bad.header.fingerprint ^= 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::DataMismatch)));

    // A state that does not fit the world is refused, not applied.
    let mut bad = file.clone();
    bad.state.sim.galaxy.planets.pop();
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Setup(_))));

    file.header.campaign = "nope".into();
    assert!(matches!(ScriptedSim::from_save(file, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::UnknownCampaign(_))));
}

/// Manual measurement: `cargo test --release -p galaxy-script --test save load_time -- --ignored --nocapture`.
#[test]
#[ignore]
fn load_time_of_a_200_week_save() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ai_config());
    weeks(&mut sim, 200);
    let path = std::env::temp_dir().join("space_peace_test_save200.json");
    let t = std::time::Instant::now();
    sim.save(&path).expect("save");
    println!("save: {:?}, {} KB", t.elapsed(), std::fs::metadata(&path).unwrap().len() / 1024);
    let t = std::time::Instant::now();
    let loaded = ScriptedSim::load(&path, c.fs.clone(), c.data.clone()).expect("load");
    println!("load of a week-{} save: {:?}", loaded.checkpoint().week, t.elapsed());
}

#[test]
fn debug_grant_is_gated_and_survives_a_save() {
    let Some(c) = ctx() else { return };
    let campaign = c.data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let cfg = |debug| SimConfig { victory_enabled: false, seed: 5, debug_commands: debug, ..SimConfig::default() };
    let sc = ScriptedConfig { game_scoring: false, remake_ai: false, difficulty: None, human: Some("Rebel".into()), ..ScriptedConfig::default() };
    let w = |s: &ScriptedSim| s.with_sim(|x| x.world().clone());
    let mut off = ScriptedSim::new(c.fs.clone(), c.data.clone(), &campaign, cfg(false), sc.clone()).unwrap();
    let world = w(&off);
    let (rebel, home, unit) = (world.faction_id("Rebel").unwrap(), world.factions[world.faction_id("Rebel").unwrap() as usize].home.unwrap(), world.unit_id("MC-80_Star_Cruiser").unwrap());
    assert!(off.apply(Command::DebugGrant { faction: rebel, planet: home, unit, count: 2 }).is_err(), "off by default");
    let mut sim = ScriptedSim::new(c.fs.clone(), c.data.clone(), &campaign, cfg(true), sc).unwrap();
    let count = |s: &ScriptedSim| s.with_sim(|x| x.galaxy().fleets.values().filter(|f| f.owner == rebel).map(|f| f.units.get(&unit).copied().unwrap_or(0)).sum::<u32>());
    let before = count(&sim);
    sim.apply(Command::DebugGrant { faction: rebel, planet: home, unit, count: 2 }).expect("granted");
    weeks(&mut sim, 1);
    assert_eq!(count(&sim), before + 2);
    let path = std::env::temp_dir().join("space_peace_test_debug_grant.json");
    sim.save(&path).expect("save");
    let loaded = ScriptedSim::load(&path, c.fs.clone(), c.data.clone()).expect("load");
    assert_eq!(count(&loaded), before + 2, "the replay repeats the grant");
}
