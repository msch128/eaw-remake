//! Save/load round trip against the real data: a loaded game continues exactly like the original.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{DifficultyTier, SaveError, SaveFile, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, HashEra, IdMap, Lane, LegacyLanes, NameTables, ProductionItem, ProductionKind, SimConfig};

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
    // `n` is the old 60 s week, so a 45 s fiscal week still covers the same seconds.
    let end = sim.checkpoint().time + n as f64 * 60.0;
    while sim.checkpoint().time + 1e-6 < end {
        sim.step(1.0);
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
    let path = std::env::temp_dir().join("eaw_remake_test_save.json");
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

    // Name tables that are not the ones the header was sealed with: a damaged file.
    let mut bad = file.clone();
    bad.header.world[0] += 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Format(_))));

    let mut bad = file.clone();
    bad.header.fingerprint ^= 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Format(_))));

    let mut bad = file.clone();
    bad.header.names.as_mut().unwrap().units.swap(0, 1);
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Format(_))));

    // A file without name tables whose counts or fingerprint are not those of the game data.
    let mut bad = file.clone();
    bad.header.names = None;
    bad.header.world[0] += 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::DataMismatch(_))));

    let mut bad = file.clone();
    bad.header.names = None;
    bad.header.fingerprint ^= 1;
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::DataMismatch(_))));

    // A state that does not fit the world is refused, not applied: by its checkpoint, and with a matching checkpoint by its shape.
    let mut bad = file.clone();
    bad.state.sim.galaxy.planets.pop();
    assert!(matches!(ScriptedSim::from_save(bad.clone(), c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Corrupt { .. })));
    bad.checkpoint.galaxy_hash = bad.state.sim.galaxy.state_hash();
    assert!(matches!(ScriptedSim::from_save(bad, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Setup(_))));

    file.header.campaign = "nope".into();
    assert!(matches!(ScriptedSim::from_save(file, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::UnknownCampaign(_))));
}

#[test]
fn a_new_save_stores_the_fiscal_week_and_an_old_header_keeps_its_week() {
    let Some(c) = ctx() else { return };
    let sim = new_sim(&c, ScriptedConfig { game_scoring: false, ..ai_config() });
    let file = sim.to_save().unwrap();
    assert_eq!(file.header.sim_config.week_seconds, 45.0);
    let loaded = ScriptedSim::from_save(file.clone(), c.fs.clone(), c.data.clone(), |_| {}).unwrap();
    loaded.with_sim(|s| {
        assert_eq!(s.world().config.week_seconds, 45.0);
        assert_eq!(s.world().fiscal_cycle_seconds, 45.0);
    });
    assert_eq!(loaded.checkpoint().galaxy_hash, sim.checkpoint().galaxy_hash);

    let mut old = file;
    old.header.sim_config.week_seconds = 60.0;
    let loaded = ScriptedSim::from_save(old, c.fs.clone(), c.data.clone(), |_| {}).unwrap();
    loaded.with_sim(|s| {
        assert_eq!(s.world().config.week_seconds, 60.0, "an old header keeps the 60 s week");
        assert_eq!(s.world().fiscal_cycle_seconds, 45.0);
    });
    assert_eq!(loaded.checkpoint().galaxy_hash, sim.checkpoint().galaxy_hash);
}

/// A save of a build from before hero gambling: its planets have neither the gambling fields nor the later ones, and its checkpoint is the hash
/// of that time (the fleet slot era). It loads, the missing values start at zero, and saved again it carries the checkpoint of this build.
#[test]
fn a_save_from_before_hero_gambling_loads_with_the_fields_at_their_defaults() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ScriptedConfig { game_scoring: false, ..ai_config() });
    weeks(&mut sim, 2);
    let mut file = sim.to_save().unwrap();
    // The old build had no swing, no siphon and no battle time to save.
    for p in &mut file.state.sim.galaxy.planets {
        (p.hero_gamble, p.hero_gamble_at) = (0.0, 0.0);
        (p.siphon, p.last_space_battle) = (None, None);
    }
    let old_hash = file.state.sim.galaxy.state_hash_of(HashEra::FleetSlots);
    assert_ne!(old_hash, file.state.sim.galaxy.state_hash(), "the two eras hash differently, or the test shows nothing");
    let mut v = serde_json::to_value(&file).unwrap();
    for p in v["state"]["sim"]["galaxy"]["planets"].as_array_mut().unwrap() {
        for field in ["hero_gamble", "hero_gamble_at", "siphon", "last_space_battle"] {
            assert!(p.as_object_mut().unwrap().remove(field).is_some(), "{field}");
        }
    }
    v["checkpoint"]["galaxy_hash"] = old_hash.into();
    let old: SaveFile = serde_json::from_value(v.clone()).expect("the old field set parses");
    let loaded = ScriptedSim::from_save(old, c.fs.clone(), c.data.clone(), |_| {}).expect("the old save loads");
    assert_eq!(loaded.state().sim, file.state.sim);
    assert_eq!(loaded.to_save().unwrap().checkpoint.galaxy_hash, file.state.sim.galaxy.state_hash());

    // The same file with a state that is not the one its checkpoint was made of is still refused.
    v["state"]["sim"]["galaxy"]["factions"][0]["credits"] = 1.0e9.into();
    let tampered: SaveFile = serde_json::from_value(v).unwrap();
    assert!(matches!(ScriptedSim::from_save(tampered, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Corrupt { .. })));
}

/// Files written while the game still had its working title say `space-peace-save`. They are the same format and still load.
#[test]
fn a_save_with_the_old_format_id_still_loads() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ScriptedConfig { game_scoring: false, ..ai_config() });
    weeks(&mut sim, 2);
    let path = std::env::temp_dir().join("eaw_remake_test_old_format.json");
    sim.save(&path).expect("save");
    // A save of this build carries the new id; an old file is the same text with the old one.
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(r#"{"format":"eaw-remake-save","#), "a new save carries the new id");
    std::fs::write(&path, text.replacen("eaw-remake-save", "space-peace-save", 1)).unwrap();
    assert_eq!(SaveFile::read(&path).unwrap().format, "space-peace-save");

    let t = std::time::Instant::now();
    let loaded = ScriptedSim::load(&path, c.fs.clone(), c.data.clone()).expect("the old-format save loads");
    println!("loaded a `space-peace-save` file of week {} in {:?}", loaded.checkpoint().week, t.elapsed());
    assert_eq!(loaded.checkpoint(), sim.checkpoint());
    // Saved again, it carries the new id.
    loaded.save(&path).expect("resave");
    assert_eq!(SaveFile::read(&path).unwrap().format, "eaw-remake-save");

    // An id that is neither of the two is still refused.
    let mut other = SaveFile::read(&path).unwrap();
    other.format = "other-save".into();
    assert!(matches!(ScriptedSim::from_save(other, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Format(_))));
    let _ = std::fs::remove_file(&path);
}

/// Manual measurement: `cargo test --release -p galaxy-script --test save load_time -- --ignored --nocapture`.
#[test]
#[ignore]
fn load_time_of_a_200_week_save() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ai_config());
    weeks(&mut sim, 200);
    let path = std::env::temp_dir().join("eaw_remake_test_save200.json");
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
    let path = std::env::temp_dir().join("eaw_remake_test_debug_grant.json");
    sim.save(&path).expect("save");
    let loaded = ScriptedSim::load(&path, c.fs.clone(), c.data.clone()).expect("load");
    assert_eq!(count(&loaded), before + 2, "the replay repeats the grant");
}

/// Unit counts of every fleet by unit name: what a state means, whatever its ids are.
fn fleets_by_name(g: &galaxy_sim::Galaxy, units: &[String]) -> BTreeMap<u32, BTreeMap<String, u32>> {
    g.fleets.iter().map(|(id, f)| (*id, f.units.iter().map(|(u, n)| (units[*u as usize].clone(), *n)).collect())).collect()
}

/// A match a few weeks in, its save, and the name tables of its world.
fn played(c: &Ctx) -> (ScriptedSim, SaveFile, NameTables) {
    let mut sim = new_sim(c, ScriptedConfig { game_scoring: false, ..ai_config() });
    weeks(&mut sim, 3);
    let file = sim.to_save().unwrap();
    let names = sim.with_sim(|s| s.world().name_tables());
    (sim, file, names)
}

#[test]
fn a_save_carries_the_names_behind_its_ids() {
    let Some(c) = ctx() else { return };
    let (sim, file, names) = played(&c);
    assert_eq!(file.header.names.as_ref(), Some(&names));
    assert_eq!(file.header.world, [names.planets.len(), names.units.len(), names.factions.len()]);
    sim.with_sim(|s| {
        let w = s.world();
        assert_eq!((names.units.len(), names.structures.len(), names.starbases.len()), (w.units.len(), w.structures.len(), w.starbases.len()));
        assert_eq!(names.units[w.unit_id("TIE_Fighter_Squadron").unwrap() as usize], "TIE_Fighter_Squadron");
    });
    // Written and read back, the tables are the same.
    let text = serde_json::to_string(&file).unwrap();
    let back: SaveFile = serde_json::from_str(&text).unwrap();
    assert_eq!(back.header.names, file.header.names);
}

/// A save loads into the build that wrote it without a single id or value moving: saved again it is the same file, byte for byte, but for
/// the two flags with which the story engine asks for a re-evaluation after every load.
#[test]
fn a_loaded_save_written_again_is_the_same_file() {
    let Some(c) = ctx() else { return };
    let (_, file, _) = played(&c);
    let loaded = ScriptedSim::from_save(file.clone(), c.fs.clone(), c.data.clone(), |_| {}).expect("load");
    assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);
    let mut again = loaded.to_save().unwrap();
    assert!(again.state.runtime.story.dirty && again.state.runtime.story.next_due == Some(0.0), "a loaded story is due at once");
    again.state.runtime.story.dirty = file.state.runtime.story.dirty;
    again.state.runtime.story.next_due = file.state.runtime.story.next_due;
    let (before, after) = (serde_json::to_string(&file).unwrap(), serde_json::to_string(&again).unwrap());
    assert_eq!(after.len(), before.len());
    assert!(after == before, "the save changed in a round trip");
}

/// What every catalog addition did to every existing save: the file was written when the catalog had fewer unit types. The newest type is
/// at the end of the table, so the older file simply lacks the name and no id in it differs.
#[test]
fn a_save_written_before_a_unit_type_was_added_loads() {
    let Some(c) = ctx() else { return };
    let (_, file, names) = played(&c);
    let mut older = names.clone();
    let added = older.units.pop().unwrap();
    let last = older.units.len() as u32;
    assert!(file.state.sim.galaxy.fleets.values().all(|f| !f.units.contains_key(&last)), "{added} is in use, pick another");
    let mut old = file.clone();
    old.header = old.header.with_names(older);
    assert_eq!(old.header.world[1] + 1, names.units.len());
    let loaded = ScriptedSim::from_save(old, c.fs.clone(), c.data.clone(), |_| {}).unwrap_or_else(|e| panic!("a save from before {added} existed: {e}"));
    assert_eq!(loaded.state().sim, file.state.sim);
    assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);
    // Saved again it names the catalog of this build.
    assert_eq!(loaded.to_save().unwrap().header.names, Some(names));
}

/// The same when the new type sits in the middle of the catalog (as the phase repair items do): every id above it is one lower in the
/// older file. The older file is made by moving the state to the smaller catalog; that it means the same is checked by name.
#[test]
fn a_save_written_before_a_unit_type_was_inserted_loads_with_its_ids_moved() {
    let Some(c) = ctx() else { return };
    let (sim, file, names) = played(&c);
    let g = &file.state.sim.galaxy;
    // A type no fleet, queue or record uses, below the types that are in use.
    let used: std::collections::BTreeSet<u32> = g.fleets.values().flat_map(|f| f.units.keys().copied()).collect();
    let inserted = (0..*used.iter().next_back().unwrap()).find(|u| !used.contains(u) && g.factions.iter().all(|f| !f.built_lifetime.contains_key(u))).expect("a free unit type");
    let mut older = names.clone();
    let name = older.units.remove(inserted as usize);
    let back = IdMap::new(&names, &older);
    let mut old = file.clone();
    old.state.sim = match sim.with_sim(|s| s.remap_snapshot(file.state.sim.clone(), &back)) {
        Ok(snapshot) => snapshot,
        // Something else uses the type (a queue, a siphon): the state cannot be written for the smaller catalog, which is the right answer.
        Err(why) => panic!("{name} is in use: {why}"),
    };
    old.checkpoint.galaxy_hash = old.state.sim.galaxy.state_hash();
    old.header = old.header.with_names(older.clone());
    // The older file means the same and its ids differ.
    assert_ne!(old.state.sim.galaxy.fleets, g.fleets, "no id moved, the test shows nothing");
    assert_eq!(fleets_by_name(&old.state.sim.galaxy, &older.units), fleets_by_name(g, &names.units));

    let loaded = ScriptedSim::from_save(old, c.fs.clone(), c.data.clone(), |_| {}).unwrap_or_else(|e| panic!("a save from before {name} existed: {e}"));
    assert_eq!(loaded.state().sim, file.state.sim, "every id is back where the name is");
    assert_eq!(loaded.checkpoint(), sim.checkpoint());
    let mut loaded = loaded;
    weeks(&mut loaded, 2);
    assert_eq!(loaded.diagnostics().script_errors, sim.diagnostics().script_errors, "{:?}", loaded.diagnostics().last_script_error);
}

/// Ids follow the names, not the positions: a file whose table lists two unit types the other way round holds the other type's ships.
#[test]
fn ids_of_a_save_mean_what_its_name_table_says() {
    let Some(c) = ctx() else { return };
    let (_, file, names) = played(&c);
    let g = &file.state.sim.galaxy;
    let count = |g: &galaxy_sim::Galaxy, u: u32| g.fleets.values().map(|f| f.units.get(&u).copied().unwrap_or(0)).sum::<u32>();
    // The most common ship type and one nobody has.
    let common = (0..names.units.len() as u32).max_by_key(|&u| count(g, u)).unwrap();
    let unused = (0..names.units.len() as u32).rev().find(|&u| count(g, u) == 0).unwrap();
    let ships = count(g, common);
    assert!(ships > 0);
    let mut swapped = names.clone();
    swapped.units.swap(common as usize, unused as usize);
    let mut other = file.clone();
    other.header = other.header.with_names(swapped);
    let loaded = match ScriptedSim::from_save(other, c.fs.clone(), c.data.clone(), |_| {}) {
        Ok(sim) => sim,
        // Other records of the common type (a queue item ...) may not exist for the unused one; then the load says so by name.
        Err(e) => panic!("{e}"),
    };
    let after = loaded.state().sim.galaxy;
    assert_eq!((count(&after, common), count(&after, unused)), (0, ships), "the ships of {} are {} now", names.units[common as usize], names.units[unused as usize]);
    assert_eq!(after.fleets.values().map(|f| f.unit_count()).sum::<u32>(), g.fleets.values().map(|f| f.unit_count()).sum::<u32>());
}

#[test]
fn a_save_that_uses_what_the_game_data_lacks_is_refused_by_name() {
    let Some(c) = ctx() else { return };
    let (_, file, names) = played(&c);
    let g = &file.state.sim.galaxy;
    let used = *g.fleets.values().next().unwrap().units.keys().next().unwrap();
    let structure = *g.planets.iter().find_map(|p| p.structures.first()).expect("a built structure");
    let mut renamed = names.clone();
    renamed.units[used as usize] = "Ship_Of_Another_Mod".into();
    renamed.structures[structure as usize] = "Station_Of_Another_Mod".into();
    let mut other = file.clone();
    other.header = other.header.with_names(renamed);
    let Err(e) = ScriptedSim::from_save(other, c.fs.clone(), c.data.clone(), |_| {}) else { panic!("loaded a save with unknown objects") };
    assert!(matches!(e, SaveError::DataMismatch(_)), "{e}");
    assert_eq!(e.to_string(), "the game data differs from the data this save was made with: the save uses 2 objects this game data does not have: structure Station_Of_Another_Mod, unit Ship_Of_Another_Mod");
}

/// The files of the builds before the name tables: no tables, only counts and a fingerprint. With the catalog of this build they load as they are.
#[test]
fn a_save_without_name_tables_loads_while_the_catalog_is_the_same() {
    let Some(c) = ctx() else { return };
    let (sim, file, _) = played(&c);
    let mut old = file.clone();
    old.header.names = None;
    let text = serde_json::to_string(&old).unwrap();
    assert!(!text.contains("\"names\""), "the old header has no such field");
    let loaded = ScriptedSim::from_save(serde_json::from_str(&text).unwrap(), c.fs.clone(), c.data.clone(), |_| {}).expect("the old header loads");
    assert_eq!(loaded.checkpoint(), sim.checkpoint());
    assert_eq!(loaded.state().sim, file.state.sim);
}

/// The save as a build wrote it whose unit catalog was today's without the types `dropped` picks: the state moved to that catalog, the header
/// with its counts and fingerprint and, like every file of those builds, without name tables.
fn written_by_an_earlier_build(sim: &ScriptedSim, file: &SaveFile, names: &NameTables, dropped: impl Fn(&str) -> bool) -> (SaveFile, NameTables) {
    let mut older = names.clone();
    older.units.retain(|n| !dropped(n));
    assert!(older.units.len() < names.units.len(), "nothing dropped");
    let mut old = file.clone();
    old.state.sim = sim.with_sim(|s| s.remap_snapshot(file.state.sim.clone(), &IdMap::new(names, &older))).expect("the state fits the smaller catalog");
    old.checkpoint.galaxy_hash = old.state.sim.galaxy.state_hash();
    old.header = old.header.with_names(older.clone());
    old.header.names = None;
    (old, older)
}

/// A file of 2026-11-08: no name tables, and the catalog had no phase repair items yet. They sit in the middle of today's catalog, so the
/// ids of every agent and structure swap behind them moved. The catalog is found by the fingerprint and the ids follow.
#[test]
fn a_save_without_name_tables_from_a_build_with_a_smaller_catalog_loads() {
    let Some(c) = ctx() else { return };
    let (sim, _, names) = played(&c);
    // Something behind the phase repairs in the catalog, so that an id in the state moves: an underboss at Coruscant.
    let (underworld, coruscant, smuggler) = sim.with_sim(|s| (s.world().faction_id("Underworld").unwrap(), s.world().planet_id("Coruscant_WW").unwrap(), s.world().unit_id("Underboss_Team").expect("an agent")));
    sim.with_sim_mut(|s| s.spawn_units(underworld, coruscant, smuggler, 1));
    let file = sim.to_save().unwrap();
    let repairs = names.units.iter().filter(|n| n.starts_with("Repair_Phase_")).count();
    assert!(repairs > 0 && (smuggler as usize) > names.units.iter().position(|n| n.starts_with("Repair_Phase_")).unwrap());

    let (old, older) = written_by_an_earlier_build(&sim, &file, &names, |n| n.starts_with("Repair_Phase_"));
    assert_eq!(old.header.world[1] + repairs, names.units.len());
    assert_ne!(old.state.sim.galaxy.fleets, file.state.sim.galaxy.fleets, "the agent's id is lower in the older file");
    assert_eq!(fleets_by_name(&old.state.sim.galaxy, &older.units), fleets_by_name(&file.state.sim.galaxy, &names.units));
    let text = serde_json::to_string(&old).unwrap();
    assert!(!text.contains("\"names\""));

    let loaded = ScriptedSim::from_save(serde_json::from_str(&text).unwrap(), c.fs.clone(), c.data.clone(), |_| {}).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.state().sim, file.state.sim);
    assert_eq!(loaded.checkpoint(), sim.checkpoint());
    assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);
    assert!(loaded.load_notes.iter().any(|n| n.contains("the catalog of a build without phase repairs")), "{:?}", loaded.load_notes);
    // Saved again, the file names its ids.
    assert_eq!(loaded.to_save().unwrap().header.names, Some(names));
}

/// A file of October 2026: only the ships and squadrons of the data and the hero upgrades.
#[test]
fn a_save_without_name_tables_from_a_build_with_only_the_upgrade_items_loads() {
    let Some(c) = ctx() else { return };
    let (sim, file, names) = played(&c);
    let derived_later: std::collections::BTreeSet<String> = sim.with_sim(|s| {
        s.world().units.iter().filter(|u| u.name.starts_with("Repair_Phase_") || matches!(u.role, galaxy_sim::UnitRole::Dummy | galaxy_sim::UnitRole::Agent | galaxy_sim::UnitRole::StructureSwap { .. })).map(|u| u.name.clone()).collect()
    });
    let (old, _) = written_by_an_earlier_build(&sim, &file, &names, |n| derived_later.contains(n));
    assert_eq!(old.header.world[1] + derived_later.len(), names.units.len());
    let loaded = ScriptedSim::from_save(old, c.fs.clone(), c.data.clone(), |_| {}).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.state().sim, file.state.sim);
    assert!(loaded.load_notes.iter().any(|n| n.contains("without phase repairs, purchases without a ship, agents, structure swaps")), "{:?}", loaded.load_notes);
}

/// A file without name tables whose catalog is none this build can rebuild is refused, and the message says why.
#[test]
fn a_save_without_name_tables_from_an_unknown_catalog_is_refused_with_the_reason() {
    let Some(c) = ctx() else { return };
    let (_, file, names) = played(&c);
    let mut old = file.clone();
    old.header.names = None;
    old.header.world[1] -= 3;
    let e = ScriptedSim::from_save(old.clone(), c.fs.clone(), c.data.clone(), |_| {}).err().expect("refused");
    assert!(matches!(e, SaveError::DataMismatch(_)));
    let expect = format!(
        "the game data differs from the data this save was made with: the save has no name tables, and its catalog ({} unit types, fingerprint {:016x}) is none of the earlier catalogs this build can rebuild from the game data ({} unit types now), so its ids cannot be read",
        names.units.len() - 3,
        file.header.fingerprint,
        names.units.len()
    );
    assert_eq!(e.to_string(), expect);

    // Another map: the planets are part of the header.
    old.header.world = [names.planets.len() + 1, names.units.len(), names.factions.len()];
    let e = ScriptedSim::from_save(old, c.fs.clone(), c.data.clone(), |_| {}).err().expect("refused");
    assert_eq!(
        e.to_string(),
        format!("the game data differs from the data this save was made with: the save has no name tables and was written with {} planets and {} factions; the campaign now has {} and {}", names.planets.len() + 1, names.factions.len(), names.planets.len(), names.factions.len())
    );
}

/// A state saved between a battle and the next tidy-up of the ship lists (the autosave after a battle of an October 2026 build is one): the
/// hyperspace slot of a fleet still names a ship that died. The file is intact, its checkpoint is the hash of exactly this state; putting
/// it into a match prunes the entry. That is a tidied save, not a damaged one.
#[test]
fn a_save_with_a_stale_hyperspace_slot_loads_and_is_tidied() {
    let Some(c) = ctx() else { return };
    let (_, mut file, _) = played(&c);
    let g = &mut file.state.sim.galaxy;
    let (fleet, unit) = g.fleets.iter().find(|(id, _)| !g.hyperspace.contains_key(id)).map(|(id, f)| (*id, *f.units.keys().next().unwrap())).expect("a fleet without a slot ship");
    let before = g.state_hash();
    g.hyperspace.insert(fleet, (galaxy_sim::ShipId(u64::MAX), unit));
    file.checkpoint.galaxy_hash = g.state_hash();
    assert_ne!(file.checkpoint.galaxy_hash, before, "the slot ship is part of the checkpoint");

    let loaded = ScriptedSim::from_save(file, c.fs.clone(), c.data.clone(), |_| {}).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.state().sim.galaxy.hyperspace.get(&fleet), None);
    assert_eq!(loaded.checkpoint().galaxy_hash, before, "the state without the stale entry");
    assert_eq!(loaded.load_warnings, ["the ship lists of the save did not match its fleets and were tidied"]);
}

/// The three lanes a build before the two production queues kept the items of `queues` in: ships and other units, structures, star base and
/// tech levels, each in its order.
fn as_lanes(queues: &[VecDeque<ProductionItem>; 2]) -> LegacyLanes {
    let mut lanes = LegacyLanes::default();
    for item in queues.iter().flatten() {
        let lane = match item.kind {
            ProductionKind::Unit(_) => 0,
            ProductionKind::Structure(_) => 1,
            ProductionKind::Starbase(_) | ProductionKind::Tech(_) => 2,
        };
        lanes[lane].push_back(item.clone());
    }
    lanes
}

/// A save of a build from before the two production queues: every planet has three lanes (`queues`) and up to four of its items were being
/// built at once: a ship, an orbital structure, a ground structure and a star base level. The checkpoint is the hash of that time, over the
/// lanes. The file loads; the lanes are sorted into the space queue and the ground queue, what was being built first and what was waiting
/// behind it, each in lane order; every item keeps what was built of it; from then on only the first item of each queue is built. Saved again, the file has the two queues and the checkpoint of this build.
#[test]
fn a_save_with_the_three_lanes_of_an_earlier_build_loads_into_the_two_queues() {
    let Some(c) = ctx() else { return };
    let mut sim = new_sim(&c, ScriptedConfig { game_scoring: false, ..ai_config() });
    weeks(&mut sim, 2);
    let w = sim.with_sim(|s| s.world().clone());
    let p = w.planet_id("Coruscant_WW").unwrap() as usize;
    let file = sim.to_save().unwrap();
    assert!(file.state.sim.galaxy.planets.iter().any(|ps| ps.queues.iter().any(|q| !q.is_empty())), "the AI has orders waiting, or the test shows little");

    // Coruscant with every lane busy, as the old build had it: the first ship, the first ground and the first orbital structure and the star
    // base level were all being built.
    let item = |kind: ProductionKind, remaining: f32, total: f32| ProductionItem { kind, remaining, total, cost: 1000.0 };
    let unit = |name: &str| ProductionKind::Unit(w.unit_id(name).unwrap_or_else(|| panic!("{name}")));
    let structure = |name: &str| ProductionKind::Structure(w.structure_id(name).unwrap_or_else(|| panic!("{name}")));
    let base = ProductionKind::Starbase(w.starbases.iter().position(|b| b.name == "Empire_Star_Base_3").expect("Empire_Star_Base_3") as u32);
    let coruscant: LegacyLanes = [
        [item(unit("Vigil_Corvette"), 35.0, 45.0), item(unit("UPGRADE_MINING_COMPANY_E_URBAN"), 30.0, 30.0), item(unit("Gladiator_Star_Destroyer"), 75.0, 75.0)].into(),
        [item(structure("TAXATION_AGENCY_URBAN_SPAWNER"), 50.0, 60.0), item(structure("E_Shipyard_3"), 240.0, 250.0), item(structure("Empire_Watchtower"), 15.0, 15.0)].into(),
        [item(base.clone(), 80.0, 90.0)].into(),
    ];
    let mut old = file.state.sim.galaxy.clone();
    for (i, ps) in old.planets.iter_mut().enumerate() {
        ps.legacy_lanes = Some(if i == p { coruscant.clone() } else { as_lanes(&ps.queues) });
        ps.queues = Default::default();
    }
    let old_hash = old.state_hash_of(HashEra::HeroGamble);
    assert_eq!(old.hash_era(old_hash), Some(HashEra::HeroGamble));

    // The file of the old build: `queues` with three lanes per planet, no `build_queues`, the checkpoint of its time.
    let mut v = serde_json::to_value(&file).unwrap();
    for (ps, state) in v["state"]["sim"]["galaxy"]["planets"].as_array_mut().unwrap().iter_mut().zip(&old.planets) {
        let o = ps.as_object_mut().unwrap();
        assert!(o.remove("build_queues").is_some());
        o.insert("queues".into(), serde_json::to_value(state.legacy_lanes.as_ref().unwrap()).unwrap());
    }
    v["checkpoint"]["galaxy_hash"] = old_hash.into();
    let old_file: SaveFile = serde_json::from_value(v.clone()).expect("the old field set parses");
    assert_eq!(old_file.state.sim.galaxy, old);

    let mut loaded = ScriptedSim::from_save(old_file, c.fs.clone(), c.data.clone(), |_| {}).unwrap_or_else(|e| panic!("the old save loads: {e}"));
    assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);
    assert!(loaded.load_notes.iter().any(|n| n.contains("three production lanes of an earlier build")), "{:?}", loaded.load_notes);
    let state = loaded.state().sim.galaxy;
    assert!(state.planets.iter().all(|ps| ps.legacy_lanes.is_none()));
    // Every planet but Coruscant has the queues the match was saved with: the lanes held nothing else.
    for (i, (now, saved)) in state.planets.iter().zip(&file.state.sim.galaxy.planets).enumerate().filter(|(i, _)| *i != p) {
        let by_lane = |q: &[VecDeque<ProductionItem>; 2]| -> Vec<ProductionItem> { as_lanes(q).into_iter().flatten().collect() };
        assert_eq!(by_lane(&now.queues), by_lane(&saved.queues), "planet {}", w.planets[i].name);
        assert!(now.queues.iter().enumerate().all(|(lane, q)| q.iter().all(|it| w.lane_of(&it.kind) as usize == lane)), "planet {}", w.planets[i].name);
    }
    // Coruscant: the corvette, the shipyard and the star base level were being built and lead the space queue, the destroyer waited; the
    // agency was being built and leads the ground queue, the upgrade and the watchtower waited.
    let got = |sim: &ScriptedSim, lane: Lane| sim.with_sim(|s| s.galaxy().planets[p].queues[lane as usize].iter().map(|i| (i.kind.clone(), i.remaining)).collect::<Vec<_>>());
    assert_eq!(
        got(&loaded, Lane::Space),
        [(unit("Vigil_Corvette"), 35.0), (structure("E_Shipyard_3"), 240.0), (base.clone(), 80.0), (unit("Gladiator_Star_Destroyer"), 75.0)]
    );
    assert_eq!(
        got(&loaded, Lane::Ground),
        [(structure("TAXATION_AGENCY_URBAN_SPAWNER"), 50.0), (unit("UPGRADE_MINING_COMPANY_E_URBAN"), 30.0), (structure("Empire_Watchtower"), 15.0)]
    );

    // Played on, only the first of each queue is built; the shipyard, the star base level and the agency keep what the old build had built.
    for _ in 0..5 {
        loaded.step(1.0);
    }
    let (space, ground) = (got(&loaded, Lane::Space), got(&loaded, Lane::Ground));
    assert_eq!((&space[0], &ground[0]), (&(unit("Vigil_Corvette"), 30.0), &(structure("TAXATION_AGENCY_URBAN_SPAWNER"), 45.0)));
    assert_eq!(space[1..4], [(structure("E_Shipyard_3"), 240.0), (base, 80.0), (unit("Gladiator_Star_Destroyer"), 75.0)]);
    assert_eq!(ground[1..3], [(unit("UPGRADE_MINING_COMPANY_E_URBAN"), 30.0), (structure("Empire_Watchtower"), 15.0)]);

    // Saved again it is a file of this build.
    let again = loaded.to_save().unwrap();
    assert_eq!(again.state.sim.galaxy.hash_era(again.checkpoint.galaxy_hash), Some(HashEra::CURRENT));
    let text = serde_json::to_string(&again).unwrap();
    assert!(text.contains("\"build_queues\":[[") && !text.contains("\"queues\":"), "the three lanes are not written again");

    // The same old file with an item moved to another lane is not the state its checkpoint was made of.
    let lanes = v["state"]["sim"]["galaxy"]["planets"][p]["queues"].as_array_mut().unwrap();
    let moved = lanes[2].as_array_mut().unwrap().remove(0);
    lanes[0].as_array_mut().unwrap().push(moved);
    let tampered: SaveFile = serde_json::from_value(v).unwrap();
    assert!(matches!(ScriptedSim::from_save(tampered, c.fs.clone(), c.data.clone(), |_| {}), Err(SaveError::Corrupt { .. })));
}
