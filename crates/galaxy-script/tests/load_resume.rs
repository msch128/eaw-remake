//! A loaded game goes on where it was saved: no story script runs the entry code of its state a second time, and its threads go on where
//! they waited. Real-data tests; the first ones load copies of saves of a week 142 campaign when `EAW_REMAKE_LOADBUG_SAVES` names them
//! (a path list; saves of a build that kept no threads), the others save and load a fresh Ground Up campaign.

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_lua::{LuaVal, RuntimeSnapshot};
use galaxy_script::{SaveFile, ScriptedConfig, ScriptedSim};
use galaxy_sim::SimConfig;

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
    let data = Arc::new(load_galaxy(&*fs).0);
    Some(Ctx { fs, data })
}

/// The saves named by `EAW_REMAKE_LOADBUG_SAVES`.
fn owner_saves() -> Vec<(String, SaveFile)> {
    let Some(paths) = std::env::var_os("EAW_REMAKE_LOADBUG_SAVES") else { return Vec::new() };
    std::env::split_paths(&paths).map(|p| (p.display().to_string(), SaveFile::read(&p).expect("the save reads"))).collect()
}

fn load(c: &Ctx, file: &SaveFile) -> ScriptedSim {
    ScriptedSim::from_save(file.clone(), c.fs.clone(), c.data.clone(), |_| {}).expect("the save loads")
}

/// Ships per faction and the build locks of every faction: what a script that runs its entry code again changes.
fn footprint(sim: &ScriptedSim) -> (BTreeMap<String, u32>, String) {
    sim.with_sim(|s| {
        let (w, g) = (s.world(), s.galaxy());
        let mut ships = BTreeMap::new();
        for f in g.fleets.values() {
            *ships.entry(w.factions[f.owner as usize].name.clone()).or_default() += f.unit_count();
        }
        let locks = g.factions.iter().map(|f| format!("{:?}{:?}", f.unlocked, f.story_locked)).collect::<String>();
        (ships, locks)
    })
}

/// Where every saved script is: `(name, CurrentState, the function of each thread with the lines of the frames it waits in)`.
fn places(snap: &RuntimeSnapshot) -> Vec<(String, Option<LuaVal>, Vec<(String, Vec<i32>)>)> {
    snap.scripts
        .iter()
        .map(|s| {
            let state = s.globals.iter().find(|(n, _)| n == "CurrentState").map(|(_, v)| v.clone());
            let threads = s.threads.iter().flatten().map(|t| (t.function.clone().unwrap_or_default(), t.stack.iter().map(|f| f.line).collect())).collect();
            (s.name.clone(), state, threads)
        })
        .collect()
}

fn rewards(sim: &ScriptedSim) -> BTreeMap<String, u32> {
    sim.host_stats().reward_log.iter().fold(BTreeMap::new(), |mut m, r| {
        *m.entry(r.reward.kind.clone()).or_default() += 1;
        m
    })
}

/// Where two values differ (paths into the JSON, at most 12).
fn drift(a: &impl serde::Serialize, b: &impl serde::Serialize) -> Vec<String> {
    fn walk(a: &serde_json::Value, b: &serde_json::Value, path: String, out: &mut Vec<String>) {
        use serde_json::Value as V;
        if out.len() >= 12 || a == b {
            return;
        }
        match (a, b) {
            (V::Object(x), V::Object(y)) => {
                for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                    walk(x.get(k).unwrap_or(&V::Null), y.get(k).unwrap_or(&V::Null), format!("{path}.{k}"), out);
                }
            }
            (V::Array(x), V::Array(y)) if x.len() == y.len() => {
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    walk(p, q, format!("{path}[{i}]"), out);
                }
            }
            _ => out.push(format!("{path}: {} | {}", a.to_string().chars().take(100).collect::<String>(), b.to_string().chars().take(100).collect::<String>())),
        }
    }
    let mut out = Vec::new();
    walk(&serde_json::to_value(a).unwrap(), &serde_json::to_value(b).unwrap(), String::new(), &mut out);
    out
}

/// A saved runtime without what a save of a build that kept no threads lacks, and without the library tables it held.
fn as_old(mut snap: RuntimeSnapshot) -> RuntimeSnapshot {
    for s in &mut snap.scripts {
        s.globals.retain(|(k, _)| !alamo_lua::persist::is_library(k));
        (s.threads, s.service) = (None, None);
    }
    snap.scripts.retain(|s| !s.ended);
    snap.next_thread = None;
    snap
}

/// The saves of the stuck campaign: before, the first second after a load spawned 656 units (632 in the other save: the tech 5
/// reinforcements of the minor factions once more) and fired 553 `LOCK_UNIT` / `BUILDABLE_UNIT` rewards (the unit filter locking and
/// unlocking every unit again). Now nothing is spawned and nothing fires, ships and build locks stay as saved, and the unit filter thread of
/// the Empire runs again.
#[test]
fn a_loaded_save_repeats_no_entry_code() {
    let Some(c) = ctx() else { return };
    for (path, file) in owner_saves() {
        let mut sim = load(&c, &file);
        assert!(sim.load_warnings.is_empty(), "{path}: {:?}", sim.load_warnings);
        assert!(file.state.runtime.scripts.iter().all(|s| s.threads.is_none()), "the build that wrote {path} kept no threads");
        let saved = footprint(&sim);
        let saved_ships: BTreeMap<String, u32> = file.checkpoint.factions.iter().filter(|f| f.3 > 0).map(|f| (f.0.clone(), f.3)).collect();
        assert_eq!(saved.0, saved_ships, "{path}: the loaded fleets are those of the file");

        // One second is five script services: every script has run.
        sim.step(1.0);
        let stats = sim.host_stats();
        println!("{path}: first second after the load: {} units spawned, rewards {:?}", stats.spawned, rewards(&sim));
        assert_eq!((stats.spawned, stats.reward_log.len()), (0, 0), "{path}: {:?}", stats.reward_log.iter().take(3).collect::<Vec<_>>());
        // Ships change only by what the factions build and lose in that second.
        assert_eq!(footprint(&sim).1, saved.1, "{path}: build locks are as saved");
        assert!(sim.runtime().errors().is_empty(), "{path}: {:?}", sim.runtime().errors());
        let now = places(&sim.state().runtime);
        for p in &now {
            println!("  {p:?}");
        }
        let filter = now.iter().find(|s| s.0 == "Unit_Filter_WW_E").expect("the unit filter script");
        assert_eq!(filter.2.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), ["main", "ThreadUI"], "{path}: the thread its entry code started runs");

        // Sixty seconds on: still no second set of forces and no second round of build locks.
        for _ in 0..60 {
            sim.step(1.0);
            for id in sim.with_sim(|s| s.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>()) {
                sim.resolve_battle(id);
            }
        }
        let kinds = rewards(&sim);
        println!("{path}: 60 s on: {} units spawned, rewards {kinds:?}", sim.host_stats().spawned);
        assert!(sim.host_stats().spawned <= 2 && !kinds.contains_key("LOCK_UNIT") && !kinds.contains_key("BUILDABLE_UNIT"), "{path}: {} spawned, {kinds:?}", sim.host_stats().spawned);
    }
}

/// Load, save, load, save: the first save after the load is the file plus where the threads wait (and without the library tables the old
/// build wrote), and from there on a save of a loaded game is the save it was loaded from.
#[test]
fn load_save_load_does_not_drift() {
    let Some(c) = ctx() else { return };
    for (path, file) in owner_saves() {
        let first = load(&c, &file);
        let again = first.to_save().expect("save");
        assert_eq!(drift(&as_old(again.state.runtime.clone()), &as_old(file.state.runtime.clone())), Vec::<String>::new(), "{path}: the scripts are as in the file");
        assert_eq!(drift(&again.state.sim, &file.state.sim), Vec::<String>::new(), "{path}: and so is the galaxy");
        assert_eq!(again.checkpoint, file.checkpoint);
        let mut second = load(&c, &again);
        assert!(second.load_warnings.is_empty(), "{path}: {:?}", second.load_warnings);
        assert_eq!(drift(&second.state(), &again.state), Vec::<String>::new(), "{path}: no drift");
        second.step(1.0);
        assert_eq!((second.host_stats().spawned, second.host_stats().reward_log.len()), (0, 0), "{path}");
        assert!(second.runtime().errors().is_empty(), "{path}: {:?}", second.runtime().errors());

        // The game has run on: saved and loaded again, its scripts go on where they were.
        let later = second.to_save().expect("save");
        let mut third = load(&c, &later);
        assert_eq!(drift(&third.state(), &later.state), Vec::<String>::new(), "{path}: no drift after running either");
        let before = footprint(&third);
        third.step(1.0);
        assert_eq!((third.host_stats().spawned, third.host_stats().reward_log.len()), (0, 0), "{path}");
        assert_eq!(footprint(&third).1, before.1, "{path}: build locks");
        assert_eq!(places(&third.state().runtime), places(&{ second.step(1.0); second.state().runtime }), "{path}: the scripts of the third game are where those of the second are");
    }
}

fn ground_up(c: &Ctx) -> ScriptedSim {
    let campaign = c.data.campaign("GC_Full_Web_Way_Imperial").expect("campaign").clone();
    let sim = SimConfig { ground_up: true, victory_enabled: false, seed: 3, ..SimConfig::default() };
    let config = ScriptedConfig { remake_ai: true, human: Some("Empire".into()), game_scoring: true, ..ScriptedConfig::default() };
    ScriptedSim::new(c.fs.clone(), c.data.clone(), &campaign, sim, config).expect("a Ground Up game")
}

/// A fresh Ground Up campaign, saved and loaded in the middle, next to the same game played on without a save. The start state of the
/// story scripts is not entered again (no second set of starting forces, no second round of build locks), and a state the story reaches
/// after the load is entered exactly once: tech level 2 brings the reinforcements of the minor factions one time, as many units in the loaded
/// game as in the game that was never saved, and not again after one more save and load.
#[test]
fn a_round_trip_in_a_fresh_ground_up_campaign_enters_every_state_once() {
    let Some(c) = ctx() else { return };
    let mut original = ground_up(&c);
    assert!(original.start_errors.is_empty(), "{:?}", original.start_errors);
    for _ in 0..90 {
        original.step(1.0);
    }
    let at_start = original.host_stats().spawned;
    assert!(at_start > 0, "the start state spawned the starting forces");
    let file = original.to_save().expect("save");
    println!("saved at 90 s: {at_start} units spawned by scripts so far, rewards {:?}", rewards(&original));
    for p in places(&file.state.runtime) {
        println!("  {p:?}");
    }

    let mut loaded = load(&c, &file);
    assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);
    assert_eq!(drift(&loaded.state(), &file.state), Vec::<String>::new(), "a load changes nothing");
    let before = footprint(&loaded);
    loaded.step(1.0);
    assert_eq!((loaded.host_stats().spawned, loaded.host_stats().reward_log.len()), (0, 0), "nothing is entered again");
    assert_eq!(footprint(&loaded).1, before.1, "build locks");
    original.step(1.0);
    assert_eq!(places(&loaded.state().runtime), places(&original.state().runtime), "every script is where it is in the game that was never saved");

    // The story moves on in both games: the Empire reaches tech level 2 (what the sim reports to the plots when the upgrade completes).
    let mut waves = Vec::new();
    for (name, sim) in [("never saved", &mut original), ("loaded", &mut loaded)] {
        let spawned = sim.host_stats().spawned;
        sim.runtime_mut().story_notify("STORY_TECH_LEVEL", "2");
        for _ in 0..5 {
            sim.step(1.0);
        }
        let wave = sim.host_stats().spawned - spawned;
        println!("{name}: tech level 2 spawned {wave} units");
        assert!(wave > 0, "{name}: the new state is entered");
        // More time brings no second wave.
        for _ in 0..5 {
            sim.step(1.0);
        }
        assert_eq!(sim.host_stats().spawned - spawned, wave, "{name}: and only once");
        waves.push(wave);
    }
    assert_eq!(waves[0], waves[1], "the loaded game gets the wave of the game that was never saved");
    assert_eq!(places(&loaded.state().runtime), places(&original.state().runtime));
    let state = |sim: &ScriptedSim| places(&sim.state().runtime).into_iter().find(|p| p.0 == "Spawn_Neutral_Reinforcements_Web_Way").map(|p| p.1);
    assert_eq!(state(&loaded), Some(Some(LuaVal::Str("Empire_Tech_2".into()))));

    // Saved again in the new state and loaded: the wave does not come a second time.
    let file = loaded.to_save().expect("save");
    let mut third = load(&c, &file);
    assert!(third.load_warnings.is_empty(), "{:?}", third.load_warnings);
    let before = footprint(&third);
    for _ in 0..5 {
        third.step(1.0);
    }
    assert_eq!(third.host_stats().spawned, 0, "the entered state is resumed");
    assert_eq!(footprint(&third).1, before.1);
    assert!(third.runtime().errors().is_empty(), "{:?}", third.runtime().errors());
}
