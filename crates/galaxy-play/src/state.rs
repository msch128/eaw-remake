//! Resources, messages and configuration shared by all systems of the plugin.

use std::collections::BTreeSet;
use std::sync::Arc;

use alamo_data::{Campaign, GameData};
use bevy::prelude::*;
use galaxy_sim::{Command, CommandError, Event, FactionId, FleetId, Galaxy, PlanetId, Sim, SimConfig, SimError, World};

use crate::build_menu::{BuildMenu, BuildTarget};
use crate::catalog::PlayCatalog;
use crate::driver::{ScriptSetup, SimDriver, Snapshot};
use alamo_fs::FileSource;
use crate::notify::Notification;

/// Settings of the plugin; insert before adding [`crate::GalaxyPlayPlugin`] to override defaults.
#[derive(Resource, Debug, Clone)]
pub struct PlayConfig {
    /// Simulation constants. `auto_resolve_battles` is forced off so the player gets to decide.
    pub sim: SimConfig,
    /// Simulated seconds per real second for each speed button after "pause"; the default maps to
    /// 1x, 2x, 4x and 8x (one galactic week is `sim.week_seconds` at 1x).
    pub speeds: Vec<f32>,
    /// Speed level at start: 0 = paused, `n` = `speeds[n - 1]`.
    pub initial_speed_level: usize,
    /// Let the plugin pick planets on click (turn off if the host app has its own planet picking
    /// and calls [`PlayerSelection::select_planet`]).
    pub pick_planets: bool,
    /// The host opens the tactical battle itself: "Fight (tactical)" then only sends
    /// [`PlayEvent::OpenTacticalBattle`] and waits for a [`PlayAction::ResolveBattle`]. When off the
    /// plugin settles the battle headless right away.
    pub tactical_hook: bool,
    /// Fleet model size relative to the planet radius unit.
    pub fleet_icon_scale: f32,
    /// Which model represents a fleet.
    pub fleet_model_style: crate::catalog::FleetModelStyle,
    /// Let the AI play the human faction too (headless demos and tests).
    pub human_is_ai: bool,
    /// Real seconds a notification stays in the ticker.
    pub notification_seconds: f64,
    /// Seconds between rebuilds of the UI model.
    pub ui_interval: f32,
    /// Difficulty of the AI factions when the game runs on the scripted backend ([`PlayInit::scripted`]): Easy/Normal/Hard come from the
    /// mod's `DifficultyAdjustments.xml`, Brutal/Fair are our own estimates. Ignored with `classic_ai`.
    pub difficulty: Option<galaxy_script::DifficultyTier>,
    /// Use galaxy-sim's built-in (classic) AI instead of the Remake AI with strategy ([`PlayInit::scripted`]).
    pub classic_ai: bool,
}

impl Default for PlayConfig {
    fn default() -> Self {
        Self {
            sim: SimConfig::default(),
            speeds: vec![1.0, 2.0, 4.0, 8.0],
            initial_speed_level: 1,
            pick_planets: true,
            tactical_hook: false,
            fleet_icon_scale: 0.5,
            fleet_model_style: Default::default(),
            human_is_ai: false,
            notification_seconds: 12.0,
            ui_interval: 0.2,
            difficulty: Some(galaxy_script::DifficultyTier::Normal),
            classic_ai: false,
        }
    }
}

impl PlayConfig {
    /// The simulation config to build the [`Sim`] with: the configured one with auto-resolve off.
    pub fn sim_config(&self) -> SimConfig {
        SimConfig { auto_resolve_battles: false, ..self.sim.clone() }
    }
}

/// Everything the plugin needs to start: the simulation, display data and settings.
pub struct PlayInit {
    /// The simulation (moved onto the worker thread).
    pub sim: Sim,
    /// Names and models.
    pub catalog: PlayCatalog,
    /// Settings.
    pub config: PlayConfig,
    /// Run the game on `galaxy-script` (Remake scripts, Remake AI, save/load) instead of plain galaxy-sim; see [`PlayInit::scripted`].
    pub script: Option<ScriptSetup>,
}

impl PlayInit {
    /// Like [`PlayInit::from_data`] but the game runs with the Remake story scripts, the Remake AI + strategy (`config.classic_ai` = false) at
    /// `config.difficulty`, and supports [`PlayAction::Save`] / [`PlayAction::Load`]. The player faction is the campaign's active player
    /// (every other faction is AI; `config.human_is_ai` makes all of them AI).
    pub fn scripted(data: Arc<GameData>, fs: Arc<dyn FileSource>, campaign: &Campaign, config: PlayConfig) -> Result<Self, SimError> {
        let sim = Sim::new(&data, campaign, config.sim_config())?;
        let catalog = PlayCatalog::from_data(sim.world(), &data);
        let human = if config.human_is_ai { None } else { sim.world().victory.active_player };
        let human_name = human.map_or_else(|| "none".to_string(), |h| sim.world().factions[h as usize].name.clone());
        let script_config = galaxy_script::ScriptedConfig {
            human: Some(human_name),
            remake_ai: !config.classic_ai,
            difficulty: if config.classic_ai { None } else { config.difficulty },
            ..Default::default()
        };
        let script = ScriptSetup { fs, data, campaign: campaign.name.clone(), config: script_config, load: None, load_path: None, tactical: None };
        Ok(Self { sim, catalog, config, script: Some(script) })
    }

    /// Starts a game from a save file written by [`PlayAction::Save`]. The worker replays it (progress: `SimDriver::load_progress`,
    /// placeholder snapshots have `loading = true`); the game settings of the save (campaign, difficulty, classic AI, seed) win over `config`.
    pub fn from_save(path: impl Into<std::path::PathBuf>, data: Arc<GameData>, fs: Arc<dyn FileSource>, mut config: PlayConfig) -> Result<Self, String> {
        let path = path.into();
        let file = galaxy_script::SaveFile::read(&path).map_err(|e| e.to_string())?;
        let campaign = data.campaign(&file.header.campaign).ok_or_else(|| format!("campaign {} not found", file.header.campaign))?.clone();
        config.sim = file.header.sim_config.clone();
        let sim = Sim::new(&data, &campaign, config.sim_config()).map_err(|e| e.to_string())?;
        let catalog = PlayCatalog::from_data(sim.world(), &data);
        let script = ScriptSetup { fs, data, campaign: campaign.name.clone(), config: file.header.config.clone(), load: Some(file), load_path: Some(path), tactical: None };
        Ok(Self { sim, catalog, config, script: Some(script) })
    }

    /// Builds the simulation for `campaign` plus the display catalog.
    pub fn from_data(data: &GameData, campaign: &Campaign, config: PlayConfig) -> Result<Self, SimError> {
        let sim = Sim::new(data, campaign, config.sim_config())?;
        let catalog = PlayCatalog::from_data(sim.world(), data);
        Ok(Self { sim, catalog, config, script: None })
    }

    /// Wraps an existing simulation (it must have been created with `auto_resolve_battles` off).
    pub fn new(sim: Sim, catalog: PlayCatalog, config: PlayConfig) -> Self {
        Self { sim, catalog, config, script: None }
    }
}

/// The running simulation: driver handle plus the two most recent snapshots.
#[derive(Resource)]
pub struct SimState {
    /// Worker handle.
    pub driver: SimDriver,
    /// Latest snapshot.
    pub current: Arc<Snapshot>,
    /// The snapshot before it, for tweening.
    pub previous: Option<Arc<Snapshot>>,
    /// Real time (app clock) the latest snapshot arrived.
    pub received_at: f64,
    /// Immutable world data.
    pub world: Arc<World>,
    /// Battles handed to the host for a tactical fight, waiting for [`PlayAction::ResolveBattle`].
    pub awaiting_external: BTreeSet<u32>,
}

impl SimState {
    /// Latest game state.
    pub fn galaxy(&self) -> &Galaxy {
        &self.current.galaxy
    }

    /// Galactic seconds to extrapolate fleets past the latest snapshot at `now` (real seconds)
    /// when the simulation runs at `rate` galactic seconds per real second. Capped so a stalled
    /// worker does not make ships run away from their lane.
    pub fn extrapolation(&self, now: f64, rate: f32) -> f32 {
        ((now - self.received_at).clamp(0.0, 0.25) as f32) * rate
    }
}

/// The faction the player controls (`Starting_Active_Player`); `None` = spectator.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PlayerFaction(pub Option<FactionId>);

/// Game speed: pause plus the configured multipliers.
#[derive(Resource, Debug, Clone)]
pub struct GameClock {
    /// 0 = paused, `n` = `speeds[n - 1]`.
    pub level: usize,
    /// Level restored by "toggle pause".
    pub resume_level: usize,
    /// Multipliers for levels 1.. .
    pub speeds: Vec<f32>,
    /// Arbitrary multiplier replacing the levels (demos); `Some(0.0)` pauses.
    pub override_multiplier: Option<f32>,
    /// The host holds time (pause menu, tactical battle screen); wins over every speed setting.
    pub hold: bool,
    /// Galactic seconds waiting to be sent to the worker.
    pub pending: f32,
}

impl GameClock {
    /// Clock from the config.
    pub fn new(cfg: &PlayConfig) -> Self {
        let level = cfg.initial_speed_level.min(cfg.speeds.len());
        Self { level, resume_level: level.max(1), speeds: cfg.speeds.clone(), override_multiplier: None, hold: false, pending: 0.0 }
    }

    /// Galactic seconds per real second right now, ignoring forced pauses.
    pub fn rate(&self) -> f32 {
        if self.hold {
            return 0.0;
        }
        match self.override_multiplier {
            Some(m) => m,
            None if self.level == 0 => 0.0,
            None => self.speeds.get(self.level - 1).copied().unwrap_or(1.0),
        }
    }

    /// True when time is stopped by the player.
    pub fn paused(&self) -> bool {
        self.rate() <= 0.0
    }
}

/// What the player has selected.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct PlayerSelection {
    /// Selected planet (shows the build menu when owned by the player).
    pub planet: Option<PlanetId>,
    /// Selected fleets.
    pub fleets: BTreeSet<FleetId>,
}

impl PlayerSelection {
    /// Selects a planet (clears fleet selection unless `keep_fleets`).
    pub fn select_planet(&mut self, planet: Option<PlanetId>) {
        self.planet = planet;
    }
}

/// How the player settles a battle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleChoice {
    /// "Fight (tactical)".
    Fight,
    /// "Auto-resolve".
    AutoResolve,
}

/// Requests from the UI (or the host app) to the game logic.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum PlayAction {
    /// Pause (0) or run at `speeds[n - 1]`.
    SetSpeedLevel(usize),
    /// Run at an arbitrary multiplier (0 = pause).
    SetSpeedMultiplier(f32),
    /// Pause or resume.
    TogglePause,
    /// Select a planet.
    SelectPlanet(Option<PlanetId>),
    /// Replace the fleet selection.
    SelectFleets(Vec<FleetId>),
    /// Order something at a planet.
    Build {
        /// Planet.
        planet: PlanetId,
        /// What.
        target: BuildTarget,
    },
    /// Send the selected fleets to a planet.
    MoveSelected(PlanetId),
    /// Merge the selected fleets (per planet).
    MergeSelected,
    /// Split the selected fleet in half.
    SplitSelected,
    /// Send the selected fleets back where they came from.
    RetreatSelected,
    /// Decide a pending battle. Also the way for a host that opened the tactical battle to report
    /// that it is over (use [`BattleChoice::AutoResolve`] until results can be injected).
    ResolveBattle {
        /// Battle id.
        id: u32,
        /// Decision.
        choice: BattleChoice,
    },
    /// The tactical fight of a battle is over: apply its result to the simulation.
    ApplyTactical {
        /// Battle id.
        id: u32,
        /// Survivors and winner.
        result: Box<galaxy_sim::TacticalResult>,
    },
    /// Abandon the tactical fight of a battle; it is auto-resolved.
    CancelTactical(u32),
    /// Remove a notification.
    DismissNotification(u64),
    /// Cancel a production queue entry of the player's planet; the unspent credits come back (`SimConfig::cancel_refund`).
    CancelProduction {
        /// Planet.
        planet: PlanetId,
        /// Queue.
        lane: galaxy_sim::Lane,
        /// Position in the queue (0 = in production).
        index: u32,
    },
    /// Debug: grant `count` ships of the named unit to the player at `planet` (needs `SimConfig::debug_commands`; journaled, so saves keep working).
    DebugGrant {
        /// Unit object name, e.g. `Star_Destroyer`.
        unit_name: String,
        /// Planet.
        planet: PlanetId,
        /// Number of ships (1..=50).
        count: u32,
    },
    /// Fire the superweapon (Death Star, Galaxy Gun) of the player's fleet orbiting `planet`; answered by a `PlanetDestroyed` notification.
    /// Whether a planet can be fired at: `PlanetInfo::destroyable` and an armed fleet in orbit (`UnitType::planet_destroyer`).
    DestroyPlanet(PlanetId),
    /// Write a save file (needs [`PlayInit::scripted`]); answered by [`PlayEvent::SaveFinished`].
    Save(std::path::PathBuf),
    /// Replace the running game with a save of the same game; answered by [`PlayEvent::LoadFinished`]. Progress: `SimState::driver.load_progress()`.
    /// A save of another campaign or player is refused: restart with [`PlayInit::from_save`] instead.
    Load(std::path::PathBuf),
}

/// Things the host app may want to react to.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum PlayEvent {
    /// A simulation event.
    Sim(Event),
    /// The player chose "Fight (tactical)" and [`PlayConfig::tactical_hook`] is on: the simulation prepared the fight. Open
    /// the battle view for this request, then send [`PlayAction::ApplyTactical`] (or [`PlayAction::CancelTactical`]).
    TacticalReady(Box<galaxy_sim::TacticalRequest>),
    /// A [`PlayAction::Save`] finished.
    SaveFinished {
        /// File.
        path: std::path::PathBuf,
        /// `Ok` or the reason.
        result: Result<(), String>,
    },
    /// A [`PlayAction::Load`] (or the save the game was started from with [`PlayInit::from_save`]) finished: `Ok(week reached)`.
    LoadFinished {
        /// File.
        path: std::path::PathBuf,
        /// `Ok(week)` or the reason.
        result: Result<u64, String>,
    },
    /// A script event of the Remake scripts (dialog, message, buildable changed; scripted backend).
    Script(galaxy_script::ScriptEvent),
    /// The simulation refused an order of the player.
    CommandRejected {
        /// The order.
        command: Command,
        /// Why.
        error: CommandError,
    },
}

/// Data for the top bar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TopBarModel {
    /// Faction display name.
    pub faction: String,
    /// Faction color (sRGB 0..1).
    pub color: [f32; 3],
    /// Credits.
    pub credits: f64,
    /// Credits per week.
    pub income_per_week: f64,
    /// Population in use.
    pub population_used: i32,
    /// Population cap.
    pub population_cap: i32,
    /// Planets owned.
    pub planets: u32,
    /// Weeks completed.
    pub week: u64,
    /// Galactic seconds elapsed.
    pub time: f64,
    /// Winner, if the game is decided.
    pub winner: Option<String>,
}

/// Speed controls state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpeedModel {
    /// Current level (0 = paused).
    pub level: usize,
    /// Multipliers for levels 1.. .
    pub speeds: Vec<f32>,
    /// Current effective multiplier.
    pub rate: f32,
    /// A decision (battle) is holding the game.
    pub held_by_battle: bool,
}

/// One row of the fleet panel.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetRow {
    /// Fleet id.
    pub id: FleetId,
    /// Owner.
    pub owner: FactionId,
    /// Owned by the player.
    pub mine: bool,
    /// Display name of the flagship.
    pub flagship: String,
    /// Ship counts by type name.
    pub ships: Vec<(String, u32)>,
    /// Total ships.
    pub count: u32,
    /// Planet name when parked, else `None`.
    pub at: Option<String>,
    /// Destination name and ETA in galactic seconds when travelling.
    pub heading: Option<(String, f32)>,
}

/// A side of a pending battle.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleSide {
    /// Faction name.
    pub faction: String,
    /// Faction color.
    pub color: [f32; 3],
    /// Ships in orbit.
    pub ships: u32,
    /// Object name of its strongest unit type (the portrait on the battle banner); empty without ships.
    pub flagship: String,
    /// Fights with its star base and structures.
    pub defender: bool,
    /// The player's side.
    pub mine: bool,
}

/// The "battle pending" prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleDialog {
    /// Battle id.
    pub id: u32,
    /// Where.
    pub planet: PlanetId,
    /// Planet name.
    pub planet_name: String,
    /// Parties.
    pub sides: Vec<BattleSide>,
}

/// Everything a UI needs, in plain data. A UI crate reads this resource and sends
/// [`PlayAction`] messages; it never touches the simulation.
#[derive(Resource, Debug, Clone, Default)]
pub struct PlayUiModel {
    /// Increases whenever any field below changes in a way that needs a re-layout.
    pub version: u64,
    /// Top bar.
    pub top: TopBarModel,
    /// Speed buttons.
    pub speed: SpeedModel,
    /// Build menu of the selected planet when the player owns it.
    pub planet: Option<BuildMenu>,
    /// Name of the selected planet if it is not the player's (info only).
    pub other_planet: Option<(String, Option<String>)>,
    /// Selected fleets.
    pub fleets: Vec<FleetRow>,
    /// Ticker entries.
    pub notifications: Vec<Notification>,
    /// Battle prompt, if a battle waits for the player.
    pub battle: Option<BattleDialog>,
    /// Object names of the player's heroes that are alive in a fleet (the hero bar).
    pub heroes: Vec<String>,
    /// Hint for the current drag, e.g. route and ETA.
    pub route_hint: Option<String>,
}

