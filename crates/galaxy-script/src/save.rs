//! Save and load of a running game: a *snapshot* of the actual state, so loading takes the same time however long the game has run.
//!
//! A save file (JSON, `eaw-remake-save` format version 2) holds
//! * a header: campaign, [`SimConfig`], [`ScriptedConfig`] (seed, difficulty, personalities, variety) and the name tables of the world
//!   ([`NameTables`]: the name behind every planet, faction, unit, structure and star base id of the state) with their counts and fingerprint;
//! * a checkpoint: state hash, time, week and a per-faction summary (readable without loading the game, for save lists);
//! * the state: the complete galaxy-sim state ([`galaxy_sim::SimSnapshot`]: planets, queues, fleets, damage, statistics, RNG, cooldowns), the script runtime
//!   ([`alamo_lua::RuntimeSnapshot`]: story plots with event states, counters and timers - hero respawn timers live there -, `GlobalValue` store, story flags,
//!   the saved global variables of the long-lived scripts) and the Remake AI memory ([`galaxy_ai::AiMemory`]).
//!
//! Loading builds the match as at the start (campaign, story scripts, AI players), then replaces the state. What a snapshot cannot hold: Lua coroutines
//! (their locals and the point where they sleep) and with them the AI plans in flight. Long-lived scripts restart from the top of their main loop with
//! their saved globals; an AI plan that was running is gone, its ships and queued items stay and the goals start new plans. Events that were produced but
//! not yet reported by `step` and frozen tactical set-ups are not saved (a pending tactical battle is set up again from the state).
//! A continuation from a loaded game is therefore *not* bit-identical to the game that was saved.
//!
//! Ids are indices into catalogs that are rebuilt from the game data on every start, so they move when a build reads more of the data or the
//! mod changes. A load therefore compares the name tables of the file with those of the match and, where they differ, rewrites every id by
//! name (`translate`): what the file does not know is new and stays as in a fresh match, what the file uses and the game data lacks is refused
//! with a message that names it. [`SAVE_VERSION`] is for changes that a reader cannot follow this way.

use std::path::Path;
use std::sync::Arc;

use alamo_data::GameData;
use alamo_fs::FileSource;
use alamo_lua::LuaVal;
use galaxy_sim::{Command, CommandError, CommandOk, Event, IdMap, NameTables, SimConfig, SimSnapshot, TacticalError, TacticalRequest, TacticalResult};
use serde::{Deserialize, Serialize};

use crate::{ScriptedConfig, ScriptedSim};

/// Current file format version.
pub const SAVE_VERSION: u32 = 2;
/// Format id of the files this build writes.
const FORMAT: &str = "eaw-remake-save";
/// Format id of the files written while the game still had its working title. Same format, so they load as before.
const LEGACY_FORMAT: &str = "space-peace-save";

/// Accepts the format id of this build and the one of the files written before the rename.
fn check_format(id: &str) -> Result<(), SaveError> {
    if id == FORMAT || id == LEGACY_FORMAT {
        Ok(())
    } else {
        Err(SaveError::Format(format!("unexpected format {id}")))
    }
}

/// How the match was set up, and the names behind the ids of the state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveHeader {
    /// Campaign name.
    pub campaign: String,
    /// Simulation constants (includes the seed).
    pub sim_config: SimConfig,
    /// Script and AI options.
    pub config: ScriptedConfig,
    /// Counts of planets, unit types and factions of the world that wrote the save.
    pub world: [usize; 3],
    /// Hash of the names of all planets, factions, units and structures of that world (`fingerprint`). With `world` it guards `names`
    /// against damage; the builds before the name tables compared both with their own world, so they still read a save of this build while
    /// the catalog is the same.
    pub fingerprint: u64,
    /// The name behind every planet, faction, unit, structure and star base id of the state. `None` in the files of builds that did not
    /// write them: their ids are those of the catalog of that build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<NameTables>,
}

/// State at the time of saving, checked after loading, and a summary for save-game lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// `Galaxy::state_hash` as the writing build computed it (see `galaxy_sim::HashEra`).
    pub galaxy_hash: u64,
    /// Galactic time in seconds.
    pub time: f64,
    /// Weeks played.
    pub week: u64,
    /// Per faction: `(name, planets, credits, ships)`.
    pub factions: Vec<(String, u32, f64, u32)>,
}

/// The saved state of a match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    /// galaxy-sim.
    pub sim: SimSnapshot,
    /// Story plots, global values and script globals.
    pub runtime: alamo_lua::RuntimeSnapshot,
    /// Time since the last script service step.
    pub script_clock: f64,
    /// Remake AI memory (`None` without it).
    pub ai: Option<galaxy_ai::AiMemory>,
    /// Alliances created by scripts.
    pub allies: Vec<(u32, u32)>,
}

/// The file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFile {
    /// `eaw-remake-save` for the files of this build; files from before the rename say `space-peace-save` and load as well.
    pub format: String,
    /// File format version.
    pub version: u32,
    /// Setup.
    pub header: SaveHeader,
    /// Summary and integrity check.
    pub checkpoint: Checkpoint,
    /// The state.
    pub state: GameState,
}

/// Why saving or loading failed.
#[derive(Debug)]
pub enum SaveError {
    /// File system error.
    Io(std::io::Error),
    /// Not valid JSON for this format.
    Format(String),
    /// Written by a different format version.
    Version(u32),
    /// The campaign of the save does not exist in the loaded data.
    UnknownCampaign(String),
    /// The loaded data differs from the data the save was made with in a way a load cannot follow; the text says how.
    DataMismatch(String),
    /// The restored state does not match the checkpoint (a damaged file).
    Corrupt {
        /// Checkpoint in the file.
        saved: Box<Checkpoint>,
        /// State after restoring.
        restored: Box<Checkpoint>,
    },
    /// The match could not be rebuilt or the state does not fit it.
    Setup(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "save file: {e}"),
            SaveError::Format(e) => write!(f, "save file format: {e}"),
            SaveError::Version(v) if *v > SAVE_VERSION => write!(f, "the save was written by a newer build (file version {v}, this build reads version {SAVE_VERSION})"),
            SaveError::Version(v) => write!(f, "save file version {v} is no longer supported (this build reads version {SAVE_VERSION})"),
            SaveError::UnknownCampaign(c) => write!(f, "campaign {c} not found in the game data"),
            SaveError::DataMismatch(why) => write!(f, "the game data differs from the data this save was made with: {why}"),
            SaveError::Corrupt { saved, restored } => write!(f, "the save is damaged: saved week {} hash {:x}, restored week {} hash {:x}", saved.week, saved.galaxy_hash, restored.week, restored.galaxy_hash),
            SaveError::Setup(e) => write!(f, "cannot rebuild the match: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        SaveError::Io(e)
    }
}

/// FNV-1a over the planet, faction, unit and structure names in this order, each closed by a zero byte. The star bases are not part of it
/// (the builds that introduced the fingerprint left them out, and their files must keep matching).
pub(crate) fn fingerprint(names: &NameTables) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for name in names.planets.iter().chain(&names.factions).chain(&names.units).chain(&names.structures) {
        for b in name.bytes().chain(std::iter::once(0)) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
        }
    }
    h
}

/// Builds the header of a new match.
pub(crate) fn header(campaign: &str, sim_config: &SimConfig, config: &ScriptedConfig, world: &galaxy_sim::World) -> SaveHeader {
    SaveHeader { campaign: campaign.to_string(), sim_config: sim_config.clone(), config: config.clone(), world: [0; 3], fingerprint: 0, names: None }.with_names(world.name_tables())
}

impl SaveHeader {
    /// The header with `names` as its name tables, counts and fingerprint set to match.
    pub fn with_names(mut self, names: NameTables) -> SaveHeader {
        self.world = [names.planets.len(), names.units.len(), names.factions.len()];
        self.fingerprint = fingerprint(&names);
        self.names = Some(names);
        self
    }
}

/// The name tables the ids of a save refer to. `now` are the tables of the match the save is loaded into.
fn saved_names(header: &SaveHeader, now: &NameTables) -> Result<NameTables, SaveError> {
    let fits = |names: &NameTables| header.world == [names.planets.len(), names.units.len(), names.factions.len()] && header.fingerprint == fingerprint(names);
    match &header.names {
        Some(names) if fits(names) => Ok(names.clone()),
        Some(_) => Err(SaveError::Format("the name tables do not match the counts and fingerprint of the header (a damaged file)".into())),
        // A file from before the name tables: its ids are those of the catalog of the build that wrote it.
        None if fits(now) => Ok(now.clone()),
        None => Err(SaveError::DataMismatch(format!(
            "the save has no name tables and was written with {} planets, {} unit types and {} factions; the game data now has {}, {} and {}",
            header.world[0],
            header.world[1],
            header.world[2],
            now.planets.len(),
            now.units.len(),
            now.factions.len()
        ))),
    }
}

/// Rewrites the player and object handles in a saved script value. Returns how many handles name something the game data no longer has;
/// those become a handle that resolves to nothing, as the handle of a destroyed object does.
fn remap_handles(value: &mut LuaVal, map: &IdMap) -> usize {
    match value {
        // Player handle. An id that is no faction (nobody) stays.
        LuaVal::Handle(0, id) => {
            if let Some(now) = u32::try_from(*id).ok().and_then(|f| map.faction(f)) {
                *id = u64::from(now);
            }
            0
        }
        LuaVal::Handle(1, id) => match crate::host::remap_object(*id, map) {
            Some(now) => {
                *id = now;
                0
            }
            None => {
                *id = crate::host::NO_OBJECT;
                1
            }
        },
        LuaVal::Table(entries) => entries.iter_mut().map(|(k, v)| remap_handles(k, map) + remap_handles(v, map)).sum(),
        // Type handles index the type names saved with the runtime; everything else holds no id.
        LuaVal::Handle(..) | LuaVal::Bool(_) | LuaVal::Num(_) | LuaVal::Str(_) => 0,
    }
}

/// Moves a saved state from the ids of the build that wrote it to the ids of `sim` (a freshly built match). Returns the state and what could
/// not be kept.
fn translate(sim: &ScriptedSim, map: &IdMap, state: GameState) -> Result<(GameState, Vec<String>), SaveError> {
    let GameState { sim: snapshot, mut runtime, script_clock, mut ai, allies } = state;
    let snapshot = sim.inner.borrow().sim.remap_snapshot(snapshot, map).map_err(SaveError::DataMismatch)?;
    let mut warnings = Vec::new();
    let gone: usize = runtime.scripts.iter_mut().flat_map(|s| s.globals.iter_mut()).map(|(_, v)| remap_handles(v, map)).sum();
    if gone > 0 {
        warnings.push(format!("{gone} script references to objects the game data no longer has were cleared"));
    }
    // The AI memory is indexed by planet and player; it holds no catalog ids.
    if !map.same_places() && ai.take().is_some() {
        warnings.push("the AI memory was not kept because the planets or factions of the game data changed".into());
    }
    // Pairs are kept as (lower id, higher id), the list sorted.
    let mut allies: Vec<(u32, u32)> = allies.into_iter().filter_map(|(a, b)| Some((map.faction(a)?, map.faction(b)?))).map(|(a, b)| (a.min(b), a.max(b))).collect();
    allies.sort_unstable();
    Ok((GameState { sim: snapshot, runtime, script_clock, ai, allies }, warnings))
}

impl ScriptedSim {
    /// Applies a command.
    pub fn apply(&mut self, cmd: Command) -> Result<CommandOk, CommandError> {
        self.inner.borrow_mut().sim.apply(cmd)
    }

    /// Gives factions to galaxy-sim's built-in AI.
    pub fn set_ai_factions(&mut self, factions: impl IntoIterator<Item = u32>) {
        self.inner.borrow_mut().sim.set_ai_factions(factions);
    }

    /// `Sim::tactical_setup`.
    pub fn tactical_setup(&mut self, battle_id: u32) -> Option<TacticalRequest> {
        self.inner.borrow_mut().sim.tactical_setup(battle_id)
    }

    /// `Sim::apply_tactical_result`.
    pub fn apply_tactical_result(&mut self, battle_id: u32, result: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        let events = self.inner.borrow_mut().sim.apply_tactical_result(battle_id, result)?;
        Ok(self.told(events))
    }

    /// `Sim::abandon_tactical`.
    pub fn abandon_tactical(&mut self, battle_id: u32, partial: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        let events = self.inner.borrow_mut().sim.abandon_tactical(battle_id, partial)?;
        Ok(self.told(events))
    }

    /// `Sim::resolve_battle_auto`.
    pub fn resolve_battle_auto(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        let events = self.inner.borrow_mut().sim.resolve_battle_auto(battle_id)?;
        Some(self.told(events))
    }

    /// `Sim::resolve_battle`.
    pub fn resolve_battle(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        let events = self.inner.borrow_mut().sim.resolve_battle(battle_id)?;
        Some(self.told(events))
    }

    /// `Sim::resolve_battle_tactical`.
    pub fn resolve_battle_tactical(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        let events = self.inner.borrow_mut().sim.resolve_battle_tactical(battle_id)?;
        Some(self.told(events))
    }

    /// Hands the events of a battle settled from outside the step to the scripts and the AI (the step only reads the sim's own events): the
    /// story's `STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES` / `STORY_TACTICAL_DESTROY` and the levels of `Event::TacticalGlobals`.
    fn told(&mut self, events: Vec<Event>) -> Vec<Event> {
        self.note_attack_results(&events);
        self.notify_story(&events);
        events
    }

    /// `Sim::cancel_tactical`.
    pub fn cancel_tactical(&mut self, battle_id: u32) {
        self.inner.borrow_mut().sim.cancel_tactical(battle_id);
    }

    /// State summary used as the save checkpoint.
    pub fn checkpoint(&self) -> Checkpoint {
        self.with_sim(|s| {
            let g = s.galaxy();
            let ships = |f: u32| g.fleets.values().filter(|x| x.owner == f).map(|x| x.unit_count()).sum::<u32>();
            Checkpoint {
                galaxy_hash: g.state_hash(),
                time: g.time,
                week: (g.time / f64::from(s.world().config.week_seconds)) as u64,
                factions: s.world().factions.iter().enumerate().map(|(i, f)| (f.name.clone(), g.factions[i].planets_owned, g.factions[i].credits, ships(i as u32))).collect(),
            }
        })
    }

    /// Copies the state that a save keeps.
    pub fn state(&self) -> GameState {
        let i = self.inner.borrow();
        let mut allies: Vec<(u32, u32)> = i.allies.iter().copied().collect();
        allies.sort_unstable();
        GameState { sim: i.sim.snapshot(), runtime: self.runtime.snapshot(), script_clock: self.script_clock, ai: i.ai.as_ref().map(|a| a.memory()), allies }
    }

    /// The save as a value.
    pub fn to_save(&self) -> Result<SaveFile, SaveError> {
        Ok(SaveFile { format: FORMAT.into(), version: SAVE_VERSION, header: self.header.clone(), checkpoint: self.checkpoint(), state: self.state() })
    }

    /// Writes the save to `path` (JSON).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        let file = self.to_save()?;
        let text = serde_json::to_string(&file).map_err(|e| SaveError::Format(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Reads a save written by [`ScriptedSim::save`]. `fs` and `data` must be the game data the save was made with.
    pub fn load(path: impl AsRef<Path>, fs: Arc<dyn FileSource>, data: Arc<GameData>) -> Result<ScriptedSim, SaveError> {
        Self::load_with(path, fs, data, |_| {})
    }

    /// Like [`ScriptedSim::load`], with a hook that runs on the rebuilt match before the state is put in (for runtime-only setup such as
    /// `with_sim_runtime(|s| s.set_tactical_catalog(..))` when the match resolves battles tactically).
    pub fn load_with(path: impl AsRef<Path>, fs: Arc<dyn FileSource>, data: Arc<GameData>, setup: impl FnOnce(&mut ScriptedSim)) -> Result<ScriptedSim, SaveError> {
        Self::from_save(SaveFile::read(path)?, fs, data, setup)
    }

    /// Rebuilds a match from a save value.
    pub fn from_save(file: SaveFile, fs: Arc<dyn FileSource>, data: Arc<GameData>, setup: impl FnOnce(&mut ScriptedSim)) -> Result<ScriptedSim, SaveError> {
        Self::from_save_progress(file, fs, data, setup, &mut |_| {})
    }

    /// Like [`ScriptedSim::from_save`], reporting progress (a snapshot loads in one step; the callback sees `done == total == 1` at the end).
    pub fn from_save_progress(file: SaveFile, fs: Arc<dyn FileSource>, data: Arc<GameData>, setup: impl FnOnce(&mut ScriptedSim), progress: &mut dyn FnMut(LoadProgress)) -> Result<ScriptedSim, SaveError> {
        check_format(&file.format)?;
        if file.version != SAVE_VERSION {
            return Err(SaveError::Version(file.version));
        }
        let week = file.checkpoint.week;
        progress(LoadProgress { done: 0, total: 1, week });
        let campaign = data.campaign(&file.header.campaign).ok_or_else(|| SaveError::UnknownCampaign(file.header.campaign.clone()))?.clone();
        let mut sim = ScriptedSim::new(fs, data, &campaign, file.header.sim_config.clone(), file.header.config.clone()).map_err(|e| SaveError::Setup(e.to_string()))?;
        let now = sim.with_sim(|s| s.world().name_tables());
        let saved = saved_names(&file.header, &now)?;
        let map = IdMap::new(&saved, &now);
        setup(&mut sim);
        let state = file.state;
        // The checkpoint belongs to the state as the file holds it, hashed the way the writing build hashed: a file from a build that hashed
        // less still verifies, a file whose state was altered does not.
        let held = &state.sim.galaxy;
        let era = held.hash_era(file.checkpoint.galaxy_hash).filter(|_| held.time.to_bits() == file.checkpoint.time.to_bits());
        let Some(era) = era else {
            let found = Checkpoint { galaxy_hash: held.state_hash(), time: held.time, week: held.week(), factions: Vec::new() };
            return Err(SaveError::Corrupt { saved: Box::new(file.checkpoint), restored: Box::new(found) });
        };
        if era != galaxy_sim::HashEra::CURRENT {
            sim.load_notes.push(format!("the checkpoint is of an earlier build generation ({era:?})"));
        }
        let state = if map.is_identity() {
            state
        } else {
            let (state, warnings) = translate(&sim, &map, state)?;
            sim.load_warnings.extend(warnings);
            sim.load_notes.push(format!("ids translated by name: the save knew {} unit types, {} structures and {} star bases, the game data has {}, {} and {}", saved.units.len(), saved.structures.len(), saved.starbases.len(), now.units.len(), now.structures.len(), now.starbases.len()));
            state
        };
        // Equal to the checkpoint of the file unless ids moved.
        let expected = state.sim.galaxy.state_hash_of(era);
        {
            let mut i = sim.inner.borrow_mut();
            i.sim.restore(state.sim).map_err(SaveError::Setup)?;
            i.allies = state.allies.iter().copied().collect();
            i.pending_events.clear();
            if let (Some(ai), Some(mem)) = (i.ai.as_mut(), state.ai.as_ref()) {
                sim.load_warnings.extend(ai.restore_memory(mem));
            }
        }
        let warnings = sim.runtime.restore(&state.runtime);
        sim.load_warnings.extend(warnings);
        sim.script_clock = state.script_clock;
        // Putting the state into the match must not have changed it (in the terms of the build that wrote it).
        let restored = sim.checkpoint();
        if sim.with_sim(|s| s.galaxy().state_hash_of(era)) != expected || restored.time.to_bits() != file.checkpoint.time.to_bits() {
            return Err(SaveError::Corrupt { saved: Box::new(file.checkpoint), restored: Box::new(restored) });
        }
        progress(LoadProgress { done: 1, total: 1, week });
        Ok(sim)
    }
}

/// How far a load has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadProgress {
    /// Steps done.
    pub done: u64,
    /// Steps in total.
    pub total: u64,
    /// Game week of the save.
    pub week: u64,
}

impl SaveFile {
    /// Reads a save file (cheap for save lists: `checkpoint` has week and faction summary).
    pub fn read(path: impl AsRef<Path>) -> Result<SaveFile, SaveError> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(|e| SaveError::Format(e.to_string()))
    }

    /// Progress units of loading this save (1: a snapshot is restored in one go).
    pub fn total_steps(&self) -> u64 {
        1
    }
}

/// Health counters of a running match (for soak tests and debug overlays).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    /// Lua scripts that died with an error.
    pub script_errors: usize,
    /// Message of the latest dead script.
    pub last_script_error: Option<String>,
    /// AI plans that could not be started.
    pub ai_errors: usize,
    /// Story scripts that failed to start.
    pub start_errors: usize,
    /// Always 0 (the save is a snapshot, not a journal).
    pub journal_entries: usize,
    /// Always 0.
    pub journal_steps: u64,
}

impl ScriptedSim {
    /// Error counts.
    pub fn diagnostics(&self) -> Diagnostics {
        let errors = self.runtime().errors();
        Diagnostics {
            script_errors: errors.len(),
            last_script_error: errors.last().map(|e| format!("{}: {}", e.script, e.message)),
            ai_errors: self.ai_errors.len(),
            start_errors: self.start_errors.len(),
            journal_entries: 0,
            journal_steps: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_id_of_this_build_and_the_one_before_the_rename_are_accepted() {
        assert!(check_format("eaw-remake-save").is_ok());
        assert!(check_format("space-peace-save").is_ok());
    }

    #[test]
    fn any_other_format_id_is_refused() {
        for id in ["", "eaw-remake", "eaw-remake-save2", "space-peace-save2", "SPACE-PEACE-SAVE"] {
            assert!(matches!(check_format(id), Err(SaveError::Format(_))), "{id:?} was accepted");
        }
    }

    #[test]
    fn this_build_writes_the_new_id() {
        assert_eq!(FORMAT, "eaw-remake-save");
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The fingerprint is part of every file ever written: these names hashed to this number in the builds that had no name tables.
    #[test]
    fn the_fingerprint_is_the_one_the_earlier_builds_wrote() {
        let t = NameTables { planets: names(&["Yavin", "Hoth"]), factions: names(&["Rebel"]), units: names(&["X-Wing", "Y-Wing"]), structures: names(&["Mine"]), starbases: names(&["Base"]) };
        // FNV-1a 64 of the names in table order, each closed by a zero byte.
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        for b in b"Yavin\0Hoth\0Rebel\0X-Wing\0Y-Wing\0Mine\0" {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3);
        }
        assert_eq!(fingerprint(&t), h);
        assert_eq!(fingerprint(&NameTables { starbases: Vec::new(), ..t.clone() }), h, "star bases are not part of it");
        assert_ne!(fingerprint(&NameTables { units: names(&["Y-Wing", "X-Wing"]), ..t }), h, "the order is");
    }

    /// Script globals hold handles with planet, unit and structure ids inside. Saved with units `[A, B]` and planets `[P, Q]`; now a unit
    /// was added in front and the planets are listed the other way round.
    #[test]
    fn script_handles_follow_the_names() {
        let saved = NameTables { planets: names(&["P", "Q"]), factions: names(&["Rebel", "Empire"]), units: names(&["A", "B"]), structures: names(&["Mine", "Old_Mine"]), starbases: Vec::new() };
        let now = NameTables { planets: names(&["Q", "P"]), factions: names(&["Empire", "Rebel"]), units: names(&["New", "A", "B"]), structures: names(&["Mine"]), starbases: Vec::new() };
        let map = IdMap::new(&saved, &now);
        let planet = |p: u64| (1u64 << 56) | p;
        let ships = |fleet: u64, unit: u64| (2u64 << 56) | (fleet << 24) | unit;
        let structure = |p: u64, s: u64| (3u64 << 56) | (p << 24) | s;
        let mut value = LuaVal::Table(vec![
            (LuaVal::Str("home".into()), LuaVal::Handle(1, planet(0))),
            (LuaVal::Handle(1, ships(77, 1)), LuaVal::Num(3.0)),
            (LuaVal::Str("owner".into()), LuaVal::Handle(0, 1)),
            (LuaVal::Str("nobody".into()), LuaVal::Handle(0, u64::from(u32::MAX))),
            (LuaVal::Str("mine".into()), LuaVal::Table(vec![(LuaVal::Num(1.0), LuaVal::Handle(1, structure(1, 0)))])),
            (LuaVal::Str("gone".into()), LuaVal::Handle(1, structure(1, 1))),
            (LuaVal::Str("type".into()), LuaVal::Handle(2, 1)),
            (LuaVal::Str("other".into()), LuaVal::Handle(1, 9u64 << 56)),
        ]);
        let gone = remap_handles(&mut value, &map);
        let expect = LuaVal::Table(vec![
            (LuaVal::Str("home".into()), LuaVal::Handle(1, planet(1))),
            (LuaVal::Handle(1, ships(77, 2)), LuaVal::Num(3.0)),
            (LuaVal::Str("owner".into()), LuaVal::Handle(0, 0)),
            (LuaVal::Str("nobody".into()), LuaVal::Handle(0, u64::from(u32::MAX))),
            (LuaVal::Str("mine".into()), LuaVal::Table(vec![(LuaVal::Num(1.0), LuaVal::Handle(1, structure(0, 0)))])),
            (LuaVal::Str("gone".into()), LuaVal::Handle(1, crate::host::NO_OBJECT)),
            (LuaVal::Str("type".into()), LuaVal::Handle(2, 1)),
            (LuaVal::Str("other".into()), LuaVal::Handle(1, 9u64 << 56)),
        ]);
        assert_eq!(value, expect);
        assert_eq!(gone, 1, "the structure the game data lost");
    }

    #[test]
    fn a_header_is_checked_against_its_own_name_tables_or_those_of_the_match() {
        let now = NameTables { planets: names(&["P"]), factions: names(&["Rebel"]), units: names(&["A", "B"]), structures: Vec::new(), starbases: Vec::new() };
        let older = NameTables { units: names(&["A"]), ..now.clone() };
        let blank = || SaveHeader { campaign: "c".into(), sim_config: SimConfig::default(), config: ScriptedConfig::default(), world: [0; 3], fingerprint: 0, names: None };
        // Its own tables, whatever the match has.
        assert_eq!(saved_names(&blank().with_names(older.clone()), &now).unwrap(), older);
        // No tables: the counts and the fingerprint must be those of the match.
        let mut bare = blank().with_names(now.clone());
        bare.names = None;
        assert_eq!(saved_names(&bare, &now).unwrap(), now);
        let mut bare = blank().with_names(older.clone());
        bare.names = None;
        let why = saved_names(&bare, &now).unwrap_err().to_string();
        assert_eq!(why, "the game data differs from the data this save was made with: the save has no name tables and was written with 1 planets, 1 unit types and 1 factions; the game data now has 1, 2 and 1");
        // Tables that were changed after the header was sealed.
        let mut damaged = blank().with_names(older);
        damaged.names.as_mut().unwrap().units[0] = "C".into();
        assert!(matches!(saved_names(&damaged, &now), Err(SaveError::Format(_))));
    }

    #[test]
    fn a_version_error_says_which_side_is_older() {
        assert_eq!(SaveError::Version(SAVE_VERSION + 1).to_string(), format!("the save was written by a newer build (file version {}, this build reads version {SAVE_VERSION})", SAVE_VERSION + 1));
        assert_eq!(SaveError::Version(1).to_string(), format!("save file version 1 is no longer supported (this build reads version {SAVE_VERSION})"));
    }
}
