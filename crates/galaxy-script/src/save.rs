//! Save and load of a running game.
//!
//! A [`ScriptedSim`] cannot serialise its live Lua coroutines (story plots, AI plan scripts), so a save is a *deterministic
//! journal*: the match setup (campaign, `SimConfig`, `ScriptedConfig` with seed, difficulty, personalities) plus every external
//! input in order (time steps, player commands, tactical results). Loading rebuilds the match and replays the journal; because
//! galaxy-sim, the script runtime, the AI engine and the strategy layer are deterministic this reproduces the identical state,
//! including Lua globals, task forces, AI and strategy memory and every RNG. The file also stores a checkpoint (state hash,
//! time, per-faction summary) that is verified after the replay, so a changed mod or code version fails loudly instead of
//! silently continuing a different game.
//!
//! Cost: loading re-simulates the game (about 0.1 s per game week with the Remake AI). Inputs given through
//! [`ScriptedSim::with_sim_mut`] are not journaled; after using it, [`ScriptedSim::save`] refuses with
//! [`SaveError::UnjournaledChanges`]. Use the journaled methods ([`ScriptedSim::apply`] and friends) instead.

use std::path::Path;
use std::sync::Arc;

use alamo_data::GameData;
use alamo_fs::FileSource;
use galaxy_sim::{Command, CommandError, CommandOk, Event, SimConfig, TacticalError, TacticalRequest, TacticalResult};
use serde::{Deserialize, Serialize};

use crate::{ScriptedConfig, ScriptedSim};

/// Current file format version.
pub const SAVE_VERSION: u32 = 1;
const FORMAT: &str = "space-peace-save";

/// One external input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Entry {
    /// `n` steps of `dt` game seconds.
    Steps {
        /// Step length.
        dt: f32,
        /// How many.
        n: u32,
    },
    /// A player or app command.
    Command(Command),
    /// Factions handed to the built-in AI.
    AiFactions(Vec<u32>),
    /// `Sim::tactical_setup`.
    TacticalSetup(u32),
    /// `Sim::apply_tactical_result`.
    TacticalResult {
        /// Battle id.
        id: u32,
        /// Result of the tactical battle.
        result: TacticalResult,
    },
    /// `Sim::resolve_battle_auto`.
    ResolveAuto(u32),
    /// `Sim::cancel_tactical`.
    CancelTactical(u32),
    /// `Sim::resolve_battle` (resolver of the `SimConfig`).
    Resolve(u32),
    /// `Sim::resolve_battle_tactical`.
    ResolveTactical(u32),
}

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
}

/// State at the time of saving, verified after loading, and a summary for save-game lists.
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

/// The file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFile {
    /// Always `space-peace-save`.
    pub format: String,
    /// File format version.
    pub version: u32,
    /// Setup.
    pub header: SaveHeader,
    /// Inputs in order.
    pub journal: Vec<Entry>,
    /// Verification state.
    pub checkpoint: Checkpoint,
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
    /// Replaying the journal did not reproduce the saved state (changed mod data or code).
    Diverged {
        /// Checkpoint in the file.
        saved: Box<Checkpoint>,
        /// State after the replay.
        replayed: Box<Checkpoint>,
    },
    /// `with_sim_mut` was used, those changes are not in the journal.
    UnjournaledChanges,
    /// The match could not be rebuilt.
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
            SaveError::Diverged { saved, replayed } => write!(f, "replay diverged: saved week {} hash {:x}, replayed week {} hash {:x}", saved.week, saved.galaxy_hash, replayed.week, replayed.galaxy_hash),
            SaveError::UnjournaledChanges => f.write_str("the game was changed through with_sim_mut; use the journaled methods to make it saveable"),
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

/// Recorded inputs of a running match.
#[derive(Debug, Clone)]
pub(crate) struct Journal {
    pub header: SaveHeader,
    pub entries: Vec<Entry>,
    /// Set by unjournaled mutation.
    pub tainted: std::cell::Cell<bool>,
}

impl Journal {
    pub fn step(&mut self, dt: f32) {
        if let Some(Entry::Steps { dt: last, n }) = self.entries.last_mut()
            && last.to_bits() == dt.to_bits()
        {
            *n += 1;
            return;
        }
        self.entries.push(Entry::Steps { dt, n: 1 });
    }
}

impl ScriptedSim {
    /// Applies a command and records it for saving.
    pub fn apply(&mut self, cmd: Command) -> Result<CommandOk, CommandError> {
        self.journal.entries.push(Entry::Command(cmd.clone()));
        self.inner.borrow_mut().sim.apply(cmd)
    }

    /// Gives factions to galaxy-sim's built-in AI (journaled).
    pub fn set_ai_factions(&mut self, factions: impl IntoIterator<Item = u32>) {
        let list: Vec<u32> = factions.into_iter().collect();
        self.journal.entries.push(Entry::AiFactions(list.clone()));
        self.inner.borrow_mut().sim.set_ai_factions(list);
    }

    /// `Sim::tactical_setup` (journaled).
    pub fn tactical_setup(&mut self, battle_id: u32) -> Option<TacticalRequest> {
        self.journal.entries.push(Entry::TacticalSetup(battle_id));
        self.inner.borrow_mut().sim.tactical_setup(battle_id)
    }

    /// `Sim::apply_tactical_result` (journaled).
    pub fn apply_tactical_result(&mut self, battle_id: u32, result: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        self.journal.entries.push(Entry::TacticalResult { id: battle_id, result: result.clone() });
        self.inner.borrow_mut().sim.apply_tactical_result(battle_id, result)
    }

    /// `Sim::resolve_battle_auto` (journaled).
    pub fn resolve_battle_auto(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        self.journal.entries.push(Entry::ResolveAuto(battle_id));
        self.inner.borrow_mut().sim.resolve_battle_auto(battle_id)
    }

    /// `Sim::resolve_battle` (journaled).
    pub fn resolve_battle(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        self.journal.entries.push(Entry::Resolve(battle_id));
        self.inner.borrow_mut().sim.resolve_battle(battle_id)
    }

    /// `Sim::resolve_battle_tactical` (journaled).
    pub fn resolve_battle_tactical(&mut self, battle_id: u32) -> Option<Vec<Event>> {
        self.journal.entries.push(Entry::ResolveTactical(battle_id));
        self.inner.borrow_mut().sim.resolve_battle_tactical(battle_id)
    }

    /// `Sim::cancel_tactical` (journaled).
    pub fn cancel_tactical(&mut self, battle_id: u32) {
        self.journal.entries.push(Entry::CancelTactical(battle_id));
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

    /// The save as a value.
    pub fn to_save(&self) -> Result<SaveFile, SaveError> {
        if self.journal.tainted.get() {
            return Err(SaveError::UnjournaledChanges);
        }
        Ok(SaveFile { format: FORMAT.into(), version: SAVE_VERSION, header: self.journal.header.clone(), journal: self.journal.entries.clone(), checkpoint: self.checkpoint() })
    }

    /// Writes the save to `path` (JSON).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        let file = self.to_save()?;
        let text = serde_json::to_string(&file).map_err(|e| SaveError::Format(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Reads and replays a save written by [`ScriptedSim::save`]. `fs` and `data` must be the game data the save was made with.
    pub fn load(path: impl AsRef<Path>, fs: Arc<dyn FileSource>, data: Arc<GameData>) -> Result<ScriptedSim, SaveError> {
        Self::load_with(path, fs, data, |_| {})
    }

    /// Like [`ScriptedSim::load`], with a hook that runs on the rebuilt match before the replay (for runtime-only setup such as
    /// `with_sim_mut(|s| s.set_tactical_catalog(..))` when the match resolves battles tactically).
    pub fn load_with(path: impl AsRef<Path>, fs: Arc<dyn FileSource>, data: Arc<GameData>, setup: impl FnOnce(&mut ScriptedSim)) -> Result<ScriptedSim, SaveError> {
        Self::from_save(SaveFile::read(path)?, fs, data, setup)
    }

    /// Rebuilds a match from a save value.
    pub fn from_save(file: SaveFile, fs: Arc<dyn FileSource>, data: Arc<GameData>, setup: impl FnOnce(&mut ScriptedSim)) -> Result<ScriptedSim, SaveError> {
        Self::from_save_progress(file, fs, data, setup, &mut |_| {})
    }

    /// Like [`ScriptedSim::from_save`], reporting how far the replay is (for a loading bar).
    pub fn from_save_progress(file: SaveFile, fs: Arc<dyn FileSource>, data: Arc<GameData>, setup: impl FnOnce(&mut ScriptedSim), progress: &mut dyn FnMut(LoadProgress)) -> Result<ScriptedSim, SaveError> {
        let total = file.total_steps();
        let mut done = 0u64;
        if file.format != FORMAT {
            return Err(SaveError::Format(format!("unexpected format {}", file.format)));
        }
        if file.version != SAVE_VERSION {
            return Err(SaveError::Version(file.version));
        }
        let campaign = data.campaign(&file.header.campaign).ok_or_else(|| SaveError::UnknownCampaign(file.header.campaign.clone()))?.clone();
        let mut sim = ScriptedSim::new(fs, data, &campaign, file.header.sim_config.clone(), file.header.config.clone()).map_err(|e| SaveError::Setup(e.to_string()))?;
        let w = sim.with_sim(|s| [s.world().planets.len(), s.world().units.len(), s.world().factions.len()]);
        if w != file.header.world {
            return Err(SaveError::DataMismatch);
        }
        setup(&mut sim);
        for e in &file.journal {
            match e {
                Entry::Steps { dt, n } => {
                    for k in 0..*n {
                        sim.step(*dt);
                        done += 1;
                        if k % 20 == 0 {
                            progress(LoadProgress { done, total, week: sim.checkpoint_week() });
                        }
                    }
                }
                Entry::Command(c) => {
                    let _ = sim.apply(c.clone());
                }
                Entry::AiFactions(f) => sim.set_ai_factions(f.iter().copied()),
                Entry::TacticalSetup(id) => {
                    sim.tactical_setup(*id);
                }
                Entry::TacticalResult { id, result } => {
                    let _ = sim.apply_tactical_result(*id, result);
                }
                Entry::ResolveAuto(id) => {
                    sim.resolve_battle_auto(*id);
                }
                Entry::CancelTactical(id) => sim.cancel_tactical(*id),
                Entry::Resolve(id) => {
                    sim.resolve_battle(*id);
                }
                Entry::ResolveTactical(id) => {
                    sim.resolve_battle_tactical(*id);
                }
            }
        }
        progress(LoadProgress { done: total, total, week: sim.checkpoint_week() });
        let replayed = sim.checkpoint();
        if replayed.galaxy_hash != file.checkpoint.galaxy_hash || replayed.time.to_bits() != file.checkpoint.time.to_bits() {
            return Err(SaveError::Diverged { saved: Box::new(file.checkpoint), replayed: Box::new(replayed) });
        }
        Ok(sim)
    }
}

/// Builds the journal header of a new match.
pub(crate) fn header(campaign: &str, sim_config: &SimConfig, config: &ScriptedConfig, world: [usize; 3]) -> SaveHeader {
    SaveHeader { campaign: campaign.to_string(), sim_config: sim_config.clone(), config: config.clone(), world }
}

/// How far a replay has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadProgress {
    /// Steps replayed.
    pub done: u64,
    /// Steps in the save.
    pub total: u64,
    /// Game week reached.
    pub week: u64,
}

impl SaveFile {
    /// Reads a save file without replaying it (cheap: use it for slot lists, `checkpoint` has week and faction summary).
    pub fn read(path: impl AsRef<Path>) -> Result<SaveFile, SaveError> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(|e| SaveError::Format(e.to_string()))
    }

    /// Number of simulation steps the journal replays.
    pub fn total_steps(&self) -> u64 {
        self.journal.iter().map(|e| if let Entry::Steps { n, .. } = e { u64::from(*n) } else { 0 }).sum()
    }
}

impl ScriptedSim {
    pub(crate) fn checkpoint_week(&self) -> u64 {
        self.with_sim(|s| (s.galaxy().time / f64::from(s.world().config.week_seconds)) as u64)
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
    /// Entries in the save journal.
    pub journal_entries: usize,
    /// Steps in the save journal.
    pub journal_steps: u64,
}

impl ScriptedSim {
    /// Error counts and journal size.
    pub fn diagnostics(&self) -> Diagnostics {
        let errors = self.runtime().errors();
        Diagnostics {
            script_errors: errors.len(),
            last_script_error: errors.last().map(|e| format!("{}: {}", e.script, e.message)),
            ai_errors: self.ai_errors.len(),
            start_errors: self.start_errors.len(),
            journal_entries: self.journal.entries.len(),
            journal_steps: self.journal.entries.iter().map(|e| if let Entry::Steps { n, .. } = e { u64::from(*n) } else { 0 }).sum(),
        }
    }
}
