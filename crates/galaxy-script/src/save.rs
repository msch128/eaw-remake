//! Save and load of a running game: a *snapshot* of the actual state, so loading takes the same time however long the game has run.
//!
//! A save file (JSON, `space-peace-save` format version 2) holds
//! * a header: campaign, [`SimConfig`], [`ScriptedConfig`] (seed, difficulty, personalities, variety), counts and a fingerprint of the game data;
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

use std::path::Path;
use std::sync::Arc;

use alamo_data::GameData;
use alamo_fs::FileSource;
use galaxy_sim::{Command, CommandError, CommandOk, Event, SimConfig, SimSnapshot, TacticalError, TacticalRequest, TacticalResult};
use serde::{Deserialize, Serialize};

use crate::{ScriptedConfig, ScriptedSim};

/// Current file format version.
pub const SAVE_VERSION: u32 = 2;
const FORMAT: &str = "space-peace-save";

/// How the match was set up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveHeader {
    /// Campaign name.
    pub campaign: String,
    /// Simulation constants (includes the seed).
    pub sim_config: SimConfig,
    /// Script and AI options.
    pub config: ScriptedConfig,
    /// Counts of planets, unit types and factions of the world, to detect a different mod version.
    pub world: [usize; 3],
    /// Hash of the names of all planets, factions, units and structures of the world.
    pub fingerprint: u64,
}

/// State at the time of saving, checked after loading, and a summary for save-game lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// `Galaxy::state_hash`.
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
    /// Always `space-peace-save`.
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
    /// The loaded data differs from the data the save was made with.
    DataMismatch,
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
            SaveError::Version(v) => write!(f, "save file version {v} is not supported (expected {SAVE_VERSION})"),
            SaveError::UnknownCampaign(c) => write!(f, "campaign {c} not found in the game data"),
            SaveError::DataMismatch => f.write_str("the game data differs from the data this save was made with"),
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

/// Builds the header of a new match.
pub(crate) fn header(campaign: &str, sim_config: &SimConfig, config: &ScriptedConfig, world: &galaxy_sim::World) -> SaveHeader {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain(std::iter::once(0)) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
        }
    };
    world.planets.iter().for_each(|p| eat(&p.name));
    world.factions.iter().for_each(|f| eat(&f.name));
    world.units.iter().for_each(|u| eat(&u.name));
    world.structures.iter().for_each(|s| eat(&s.name));
    SaveHeader { campaign: campaign.to_string(), sim_config: sim_config.clone(), config: config.clone(), world: [world.planets.len(), world.units.len(), world.factions.len()], fingerprint: h }
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
        self.inner.borrow_mut().sim.apply_tactical_result(battle_id, result)
    }

    /// `Sim::abandon_tactical`.
    pub fn abandon_tactical(&mut self, battle_id: u32, partial: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        self.inner.borrow_mut().sim.abandon_tactical(battle_id, partial)
    }

    /// `Sim::resolve_battle_auto`.
    pub fn resolve_battle_auto(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        self.inner.borrow_mut().sim.resolve_battle_auto(battle_id)
    }

    /// `Sim::resolve_battle`.
    pub fn resolve_battle(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        self.inner.borrow_mut().sim.resolve_battle(battle_id)
    }

    /// `Sim::resolve_battle_tactical`.
    pub fn resolve_battle_tactical(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        self.inner.borrow_mut().sim.resolve_battle_tactical(battle_id)
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
        if file.format != FORMAT {
            return Err(SaveError::Format(format!("unexpected format {}", file.format)));
        }
        if file.version != SAVE_VERSION {
            return Err(SaveError::Version(file.version));
        }
        let week = file.checkpoint.week;
        progress(LoadProgress { done: 0, total: 1, week });
        let campaign = data.campaign(&file.header.campaign).ok_or_else(|| SaveError::UnknownCampaign(file.header.campaign.clone()))?.clone();
        let mut sim = ScriptedSim::new(fs, data, &campaign, file.header.sim_config.clone(), file.header.config.clone()).map_err(|e| SaveError::Setup(e.to_string()))?;
        if sim.header.world != file.header.world || sim.header.fingerprint != file.header.fingerprint {
            return Err(SaveError::DataMismatch);
        }
        setup(&mut sim);
        let state = file.state;
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
        let restored = sim.checkpoint();
        if restored.galaxy_hash != file.checkpoint.galaxy_hash || restored.time.to_bits() != file.checkpoint.time.to_bits() {
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
