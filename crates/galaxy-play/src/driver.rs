//! Runs [`galaxy_sim::Sim`] on a dedicated worker thread.
//!
//! The UI thread sends [`Request`]s (commands, "advance by N simulated seconds", battle
//! decisions) over a channel and receives immutable [`Snapshot`]s back. A single worker processes
//! the requests strictly in order, so the result only depends on the request sequence and not on
//! thread timing: the same commands and the same advance amounts give the same state as driving a
//! [`Sim`] directly.
//!
//! Battles are handled by policy: battles that do not involve the human faction are resolved by
//! the worker immediately, battles that do stay pending (and block the passage of time) until
//! the player decides via [`SimDriver::resolve_battle`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use alamo_data::GameData;
use alamo_fs::FileSource;
use galaxy_script::{Diagnostics, LoadProgress, SaveFile, ScriptEvent, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, CommandError, CommandOk, Event, FactionId, Galaxy, PendingBattle, Sim, TacticalRequest, TacticalResult, World};

/// Identifies a submitted command so its result can be matched in a later snapshot.
pub type Ticket = u64;

/// How the player wants a pending battle settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveMode {
    /// Fast statistical auto-resolve.
    Auto,
    /// Headless tactical simulation (`battle-sim`) if the sim has a catalog, else auto-resolve.
    Headless,
}

/// Message from the UI thread to the simulation worker.
#[derive(Debug, Clone)]
pub enum Request {
    /// Apply a command.
    Command {
        /// Matches the [`CommandResult`] in a snapshot.
        ticket: Ticket,
        /// The command.
        command: Command,
    },
    /// Advance simulated time (ignored while a battle involving the human is pending).
    Advance(f32),
    /// Settle a pending battle.
    ResolveBattle {
        /// Battle id from [`PendingBattle::id`].
        id: u32,
        /// Resolver.
        mode: ResolveMode,
    },
    /// Prepare the tactical fight of a pending battle; the request arrives in a snapshot.
    TacticalSetup(u32),
    /// Apply the result of the tactical fight.
    ApplyTactical {
        /// Battle id.
        id: u32,
        /// Result of the fight.
        result: Box<TacticalResult>,
    },
    /// Abandon the tactical fight and auto-resolve the battle instead.
    CancelTactical(u32),
    /// Abandon a half-fought tactical fight: its losses are applied, then the rest is auto-resolved.
    AbandonTactical {
        /// Battle id.
        id: u32,
        /// State of the fight when it was left.
        partial: Box<TacticalResult>,
    },
    /// Write a save file (scripted backend only); the answer arrives in [`Snapshot::saves`].
    Save(PathBuf),
    /// Replace the running game with a save of the same game (replays it; progress in [`SimDriver::load_progress`]); the answer arrives in [`Snapshot::loads`].
    Load(PathBuf),
    /// Stop the worker.
    Shutdown,
}

/// Everything the worker needs to run the game with the Remake scripts, AI and the save journal (`galaxy-script`).
pub struct ScriptSetup {
    /// Game files (scripts, AI data).
    pub fs: Arc<dyn FileSource>,
    /// Game data.
    pub data: Arc<GameData>,
    /// Campaign name.
    pub campaign: String,
    /// Script and AI options (human faction name, difficulty, classic AI, personalities).
    pub config: ScriptedConfig,
    /// A save to replay instead of starting fresh (progress through [`SimDriver::load_progress`]).
    pub load: Option<SaveFile>,
    /// Path of that save (reported back in [`Snapshot::loads`]).
    pub load_path: Option<PathBuf>,
    /// Runtime-only tactical resources: with them battles can be settled by the headless battle simulation (`ResolveMode::Headless` and
    /// `SimConfig::resolver = Tactical`). Not part of saves; reapplied after every load.
    pub tactical: Option<TacticalRuntime>,
}

/// Battle catalog and map arenas for headless tactical resolution.
#[derive(Clone)]
pub struct TacticalRuntime {
    /// Unit catalog (`battle_sim::Catalog::build` with `Sim::tactical_names()`).
    pub catalog: Arc<battle_sim::Catalog>,
    /// Arenas by map name (`Sim::tactical_map_names()`).
    pub arenas: std::collections::HashMap<String, Arc<battle_sim::Arena>>,
}

fn install_tactical(s: &ScriptedSim, t: &Option<TacticalRuntime>) {
    if let Some(t) = t {
        s.with_sim_runtime(|sim| {
            sim.set_tactical_catalog(t.catalog.clone());
            sim.set_tactical_arenas(t.arenas.clone());
        });
    }
}

/// Result of a [`Request::Save`].
#[derive(Debug, Clone, PartialEq)]
pub struct SaveResult {
    /// File.
    pub path: PathBuf,
    /// `Ok` or the reason.
    pub result: Result<(), String>,
}

/// Result of a [`Request::Load`] (or of the save a game was started from): the week reached, or the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadResult {
    /// File.
    pub path: PathBuf,
    /// `Ok(week)` or the reason.
    pub result: Result<u64, String>,
}

/// Replay progress shared between the worker and the UI thread.
#[derive(Debug, Default)]
pub struct LoadState {
    active: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
    week: AtomicU64,
}

impl LoadState {
    fn begin(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.week.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.active.store(true, Ordering::Release);
    }

    fn update(&self, p: LoadProgress) {
        self.done.store(p.done, Ordering::Relaxed);
        self.week.store(p.week, Ordering::Relaxed);
    }

    fn end(&self) {
        self.active.store(false, Ordering::Release);
    }
}

/// A running replay: `fraction` is 0..1, `week` the game week reached so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadProgressView {
    /// Steps replayed.
    pub done: u64,
    /// Steps in the save.
    pub total: u64,
    /// Game week reached.
    pub week: u64,
}

impl LoadProgressView {
    /// Progress 0..1.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 { 1.0 } else { (self.done as f32 / self.total as f32).clamp(0.0, 1.0) }
    }
}

/// Outcome of a submitted command.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandResult {
    /// Ticket returned when the command was submitted.
    pub ticket: Ticket,
    /// The command as submitted.
    pub command: Command,
    /// Whether the simulation accepted it.
    pub result: Result<CommandOk, CommandError>,
}

/// Immutable copy of the simulation state published after each processed batch of requests.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Increasing counter, starting at 0 for the initial state.
    pub seq: u64,
    /// The full mutable game state.
    pub galaxy: Galaxy,
    /// Elapsed galactic seconds.
    pub time: f64,
    /// Number of completed weeks.
    pub week: u64,
    /// Events produced since the previous snapshot.
    pub events: Vec<Event>,
    /// Results of commands processed since the previous snapshot.
    pub results: Vec<CommandResult>,
    /// Pending battles that involve the human faction and wait for a decision.
    pub human_battles: Vec<PendingBattle>,
    /// Tactical fights prepared since the previous snapshot.
    pub tactical: Vec<TacticalRequest>,
    /// How many [`Request::Advance`] messages (before coalescing) have been processed so far.
    pub advances_done: u64,
    /// How many requests of any kind have been processed so far.
    pub requests_done: u64,
    /// Wall-clock microseconds the worker spent on the batch that produced this snapshot.
    pub work_micros: u64,
    /// Events of the Remake scripts (dialogs, messages, buildable changes) since the previous snapshot (scripted backend).
    pub script_events: Vec<ScriptEvent>,
    /// Results of save requests since the previous snapshot.
    pub saves: Vec<SaveResult>,
    /// Results of load requests (and of the save the game was started from) since the previous snapshot.
    pub loads: Vec<LoadResult>,
    /// True for the placeholder shown while the worker is still setting up (scripts, AI, replay of a save): the galaxy is the
    /// starting state, not the game.
    pub loading: bool,
    /// Script/AI error counts and journal size (scripted backend).
    pub diagnostics: Option<Diagnostics>,
}

/// What one faction sees of the galaxy (fog of war): per planet and per fleet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SightView {
    /// Per planet id: `Detailed` = full details, `Seen` = fleets drawn with unknown contents, `Hidden` = only the owner.
    pub planets: Vec<galaxy_sim::Sight>,
    /// Fleets that are drawn, with how much is known (`Detailed` = contents, `Seen` = "?"). A fleet that is missing is not drawn.
    pub fleets: std::collections::BTreeMap<u32, galaxy_sim::Sight>,
}

impl Snapshot {
    /// The fog of war for `viewer` (the human faction): compute it once per snapshot and use it for every marker, orbit box and tooltip.
    pub fn sight(&self, world: &World, viewer: FactionId) -> SightView {
        SightView { planets: self.galaxy.sight_map(world, viewer), fleets: self.galaxy.visible_fleets(world, viewer).into_iter().collect() }
    }

    /// Statistics of a faction so far (battles, ships, planets, credits), rebuilt by a save replay.
    pub fn stats(&self, faction: galaxy_sim::FactionId) -> galaxy_sim::FactionStats {
        self.galaxy.factions.get(faction as usize).map(|f| f.stats).unwrap_or_default()
    }

    /// Starting-state placeholder shown while the worker sets the game up.
    pub fn placeholder(galaxy: Galaxy) -> Snapshot {
        Snapshot {
            seq: 0,
            time: galaxy.time,
            galaxy,
            week: 0,
            events: Vec::new(),
            results: Vec::new(),
            human_battles: Vec::new(),
            tactical: Vec::new(),
            advances_done: 0,
            requests_done: 0,
            work_micros: 0,
            script_events: Vec::new(),
            saves: Vec::new(),
            loads: Vec::new(),
            loading: true,
            diagnostics: None,
        }
    }
}

/// Handle to the simulation worker.
pub struct SimDriver {
    tx: Sender<Request>,
    rx: Mutex<Receiver<Arc<Snapshot>>>,
    handle: Option<JoinHandle<()>>,
    world: Arc<World>,
    human: Option<FactionId>,
    next_ticket: Ticket,
    advances_sent: u64,
    requests_sent: u64,
    progress: Arc<LoadState>,
}

impl SimDriver {
    /// Moves `sim` onto a new worker thread. `human` is the faction controlled by the player (its
    /// battles are not auto-resolved and it is not run by the AI); every other faction is AI
    /// controlled. With `human = None` the whole galaxy is AI controlled.
    ///
    /// For the player to get a choice the sim must have been built with
    /// `SimConfig::auto_resolve_battles = false` (see [`crate::PlayConfig::sim_config`]).
    pub fn spawn(mut sim: Sim, human: Option<FactionId>) -> SimDriver {
        let ai: Vec<FactionId> = (0..sim.world().factions.len() as FactionId).filter(|&f| Some(f) != human).collect();
        sim.set_ai_factions(ai);
        Self::spawn_inner(sim, None, human)
    }

    /// Like [`SimDriver::spawn`] but the worker thread builds a `galaxy_script::ScriptedSim` around `sim` (a fresh simulation): Remake story
    /// scripts, the Remake AI with strategy and difficulty (or the classic AI, see `ScriptedConfig`), and the save journal. All input goes through
    /// the journaled methods, so [`SimDriver::save`] works at any time. Setting up takes about a second on the worker thread; the first snapshot
    /// arrives when it is done (use [`SimDriver::wait_initial`] in tests, or poll). With `setup.load` the worker replays that save instead
    /// (progress in [`SimDriver::load_progress`]).
    pub fn spawn_scripted(sim: Sim, setup: ScriptSetup, human: Option<FactionId>) -> SimDriver {
        Self::spawn_inner(sim, Some(setup), human)
    }

    fn spawn_inner(sim: Sim, script: Option<ScriptSetup>, human: Option<FactionId>) -> SimDriver {
        let world = sim.world().clone();
        let progress = Arc::new(LoadState::default());
        if script.as_ref().is_some_and(|s| s.load.is_some()) {
            progress.begin(script.as_ref().and_then(|s| s.load.as_ref()).map_or(0, SaveFile::total_steps));
        }
        let (tx, req_rx) = mpsc::channel();
        let (snap_tx, rx) = mpsc::channel();
        let p2 = progress.clone();
        let handle = std::thread::Builder::new()
            .name("galaxy-sim".into())
            .spawn(move || worker(sim, script, human, p2, req_rx, snap_tx))
            .expect("spawn simulation thread");
        SimDriver { tx, rx: Mutex::new(rx), handle: Some(handle), world, human, next_ticket: 1, advances_sent: 0, requests_sent: 0, progress }
    }

    /// Asks the worker to write a save file; the result arrives in [`Snapshot::saves`].
    pub fn save(&mut self, path: impl Into<PathBuf>) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::Save(path.into()));
    }

    /// Asks the worker to replace the running game with a save of the same game; the result arrives in [`Snapshot::loads`].
    pub fn load(&mut self, path: impl Into<PathBuf>) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::Load(path.into()));
    }

    /// Progress of a running replay (loading a save), `None` when none runs.
    pub fn load_progress(&self) -> Option<LoadProgressView> {
        self.progress.active.load(Ordering::Acquire).then(|| LoadProgressView { done: self.progress.done.load(Ordering::Relaxed), total: self.progress.total.load(Ordering::Relaxed), week: self.progress.week.load(Ordering::Relaxed) })
    }

    /// The immutable world description (shared with the worker).
    pub fn world(&self) -> &Arc<World> {
        &self.world
    }

    /// The human faction.
    pub fn human(&self) -> Option<FactionId> {
        self.human
    }

    /// Submits a command; its [`CommandResult`] arrives in a later snapshot.
    pub fn send_command(&mut self, command: Command) -> Ticket {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        self.requests_sent += 1;
        let _ = self.tx.send(Request::Command { ticket, command });
        ticket
    }

    /// Asks the worker to simulate `seconds` more galactic seconds.
    pub fn advance(&mut self, seconds: f32) {
        if seconds > 0.0 {
            self.advances_sent += 1;
            self.requests_sent += 1;
            let _ = self.tx.send(Request::Advance(seconds));
        }
    }

    /// Number of advance requests the worker has not finished yet.
    pub fn advances_in_flight(&self, latest: &Snapshot) -> u64 {
        self.advances_sent.saturating_sub(latest.advances_done)
    }

    /// Prepares the tactical fight of a pending battle (answer: [`Snapshot::tactical`]).
    pub fn tactical_setup(&mut self, id: u32) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::TacticalSetup(id));
    }

    /// Applies the result of a tactical fight to its pending battle.
    pub fn apply_tactical(&mut self, id: u32, result: TacticalResult) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::ApplyTactical { id, result: Box::new(result) });
    }

    /// Abandons a half-fought tactical fight: the losses so far count, the rest is auto-resolved.
    pub fn abandon_tactical(&mut self, id: u32, partial: TacticalResult) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::AbandonTactical { id, partial: Box::new(partial) });
    }

    /// Abandons a tactical fight; the battle is auto-resolved.
    pub fn cancel_tactical(&mut self, id: u32) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::CancelTactical(id));
    }

    /// Settles a pending battle.
    pub fn resolve_battle(&mut self, id: u32, mode: ResolveMode) {
        self.requests_sent += 1;
        let _ = self.tx.send(Request::ResolveBattle { id, mode });
    }

    /// Returns all snapshots published since the last call, oldest first. Never blocks.
    pub fn poll(&mut self) -> Vec<Arc<Snapshot>> {
        self.rx.get_mut().expect("snapshot receiver lock").try_iter().collect()
    }

    /// Blocks for the initial snapshot (state before any request). Call right after [`SimDriver::spawn`].
    pub fn wait_initial(&mut self) -> Arc<Snapshot> {
        self.rx.get_mut().expect("snapshot receiver lock").recv().expect("simulation thread died")
    }

    /// Blocks until every request sent so far has been processed and returns every snapshot
    /// received on the way. Used by tests and headless tools.
    pub fn wait_idle(&mut self) -> Vec<Arc<Snapshot>> {
        let mut out = Vec::new();
        let target = self.requests_sent;
        let rx = self.rx.get_mut().expect("snapshot receiver lock");
        while let Ok(s) = rx.recv() {
            let done = s.requests_done >= target;
            out.push(s);
            if done {
                break;
            }
        }
        out
    }

    /// Stops the worker and waits for it.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Request::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SimDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The simulation the worker drives: plain galaxy-sim, or galaxy-sim with the Remake scripts, AI and save journal.
enum Core {
    Plain(Box<Sim>),
    Scripted(Box<ScriptedSim>),
}

impl Core {
    fn galaxy(&self) -> Galaxy {
        match self {
            Core::Plain(s) => s.galaxy().clone(),
            Core::Scripted(s) => s.with_sim(|s| s.galaxy().clone()),
        }
    }

    fn decided(&self) -> bool {
        match self {
            Core::Plain(s) => s.galaxy().winner.is_some(),
            Core::Scripted(s) => s.with_sim(|s| s.galaxy().winner.is_some()),
        }
    }

    fn tick_seconds(&self) -> f32 {
        match self {
            Core::Plain(s) => s.world().config.tick_seconds,
            Core::Scripted(s) => s.with_sim(|s| s.world().config.tick_seconds),
        }
        .max(0.001)
    }

    fn pending_battles(&self) -> Vec<PendingBattle> {
        match self {
            Core::Plain(s) => s.pending_battles().to_vec(),
            Core::Scripted(s) => s.with_sim(|s| s.pending_battles().to_vec()),
        }
    }

    fn step(&mut self, dt: f32, script_events: &mut Vec<ScriptEvent>) -> Vec<Event> {
        match self {
            Core::Plain(s) => s.step(dt),
            Core::Scripted(s) => {
                let r = s.step(dt);
                script_events.extend(r.script_events);
                r.sim_events
            }
        }
    }

    fn apply(&mut self, c: Command) -> Result<CommandOk, CommandError> {
        match self {
            Core::Plain(s) => s.apply(c),
            Core::Scripted(s) => s.apply(c),
        }
    }

    fn tactical_setup(&mut self, id: u32) -> Option<TacticalRequest> {
        match self {
            Core::Plain(s) => s.tactical_setup(id),
            Core::Scripted(s) => s.tactical_setup(id),
        }
    }

    fn apply_tactical(&mut self, id: u32, r: &TacticalResult) -> Vec<Event> {
        match self {
            Core::Plain(s) => s.apply_tactical_result(id, r).unwrap_or_default(),
            Core::Scripted(s) => s.apply_tactical_result(id, r).unwrap_or_default(),
        }
    }

    fn abandon_tactical(&mut self, id: u32, partial: &TacticalResult) -> Vec<Event> {
        let mut events = match self {
            Core::Plain(s) => s.abandon_tactical(id, partial).unwrap_or_default(),
            Core::Scripted(s) => s.abandon_tactical(id, partial).unwrap_or_default(),
        };
        events.extend(self.resolve_auto(id));
        events
    }

    fn cancel_tactical(&mut self, id: u32) {
        match self {
            Core::Plain(s) => s.cancel_tactical(id),
            Core::Scripted(s) => s.cancel_tactical(id),
        }
    }

    fn resolve_auto(&mut self, id: u32) -> Vec<Event> {
        match self {
            Core::Plain(s) => s.resolve_battle_auto(id),
            Core::Scripted(s) => s.resolve_battle_auto(id),
        }
        .unwrap_or_default()
    }

    fn resolve(&mut self, id: u32) -> Vec<Event> {
        match self {
            Core::Plain(s) => s.resolve_battle(id),
            Core::Scripted(s) => s.resolve_battle(id),
        }
        .unwrap_or_default()
    }

    fn resolve_tactical(&mut self, id: u32) -> Vec<Event> {
        match self {
            Core::Plain(s) => s.resolve_battle_tactical(id),
            Core::Scripted(s) => s.resolve_battle_tactical(id),
        }
        .unwrap_or_default()
    }
}

fn involves(human: Option<FactionId>, b: &PendingBattle) -> bool {
    human.is_some_and(|h| b.factions.contains(&h))
}

/// Resolves every pending battle without the human with the configured resolver.
fn resolve_ai_battles(core: &mut Core, human: Option<FactionId>, events: &mut Vec<Event>) {
    let ids: Vec<u32> = core.pending_battles().iter().filter(|b| !involves(human, b)).map(|b| b.id).collect();
    for id in ids {
        events.extend(core.resolve(id));
    }
}

fn human_battles(core: &Core, human: Option<FactionId>) -> Vec<PendingBattle> {
    core.pending_battles().into_iter().filter(|b| involves(human, b)).collect()
}

/// Steps in whole simulation ticks so battles are settled (or the player is asked) the moment
/// they appear. Returns early when a battle waits for the human.
fn advance(core: &mut Core, human: Option<FactionId>, dt: f32, events: &mut Vec<Event>, script_events: &mut Vec<ScriptEvent>) {
    if !human_battles(core, human).is_empty() {
        return;
    }
    let tick = core.tick_seconds();
    let mut remaining = dt;
    while remaining > 0.0 && !core.decided() {
        let d = remaining.min(tick);
        remaining -= d;
        events.extend(core.step(d, script_events));
        resolve_ai_battles(core, human, events);
        if !human_battles(core, human).is_empty() {
            break;
        }
    }
}

/// Builds the scripted core on the worker thread (new game, or replay of a save with progress reporting).
fn build_scripted(sim: Sim, setup: &ScriptSetup, human: Option<FactionId>, progress: &LoadState, loads: &mut Vec<LoadResult>) -> Core {
    let campaign = setup.data.campaign(&setup.campaign).expect("campaign of the game").clone();
    let fresh = |sim: Sim| {
        let mut s = ScriptedSim::from_sim(sim, setup.fs.clone(), setup.data.clone(), &campaign, setup.config.clone());
        install_tactical(&s, &setup.tactical);
        if !setup.config.remake_ai {
            s.set_ai_factions((0..s.with_sim(|x| x.world().factions.len()) as FactionId).filter(|&f| Some(f) != human));
        }
        s
    };
    if let Some(file) = &setup.load {
        progress.begin(file.total_steps());
        let path = setup.load_path.clone().unwrap_or_default();
        let r = ScriptedSim::from_save_progress(file.clone(), setup.fs.clone(), setup.data.clone(), |s| install_tactical(s, &setup.tactical), &mut |p| progress.update(p));
        progress.end();
        match r {
            Ok(s) => {
                loads.push(LoadResult { path, result: Ok(s.checkpoint().week) });
                return Core::Scripted(Box::new(s));
            }
            Err(e) => loads.push(LoadResult { path, result: Err(e.to_string()) }),
        }
    }
    Core::Scripted(Box::new(fresh(sim)))
}

/// In-game load: only a save of the same game (campaign and human) is accepted.
fn load_into(core: &mut Core, path: &std::path::Path, setup: Option<&ScriptSetup>, progress: &LoadState) -> Result<u64, String> {
    let Some(setup) = setup else { return Err("loading needs the scripted backend".into()) };
    let file = SaveFile::read(path).map_err(|e| e.to_string())?;
    if file.header.campaign != setup.campaign || file.header.config.human != setup.config.human {
        return Err("this save belongs to another campaign or player; start the game from the save instead".into());
    }
    progress.begin(file.total_steps());
    let r = ScriptedSim::from_save_progress(file, setup.fs.clone(), setup.data.clone(), |s| install_tactical(s, &setup.tactical), &mut |p| progress.update(p));
    progress.end();
    let s = r.map_err(|e| e.to_string())?;
    let week = s.checkpoint().week;
    *core = Core::Scripted(Box::new(s));
    Ok(week)
}

fn worker(sim: Sim, script: Option<ScriptSetup>, human: Option<FactionId>, progress: Arc<LoadState>, rx: Receiver<Request>, tx: Sender<Arc<Snapshot>>) {
    let mut seq = 0u64;
    let mut advances_done = 0u64;
    let mut requests_done = 0u64;
    let mut first_loads = Vec::new();
    let mut core = match &script {
        Some(setup) => build_scripted(sim, setup, human, &progress, &mut first_loads),
        None => Core::Plain(Box::new(sim)),
    };
    // The setup kept for in-game loads must not carry the save that started the game.
    let script = script.map(|mut s| {
        s.load = None;
        s
    });
    let publish = |core: &Core, seq: &mut u64, p: Published, counts: (u64, u64), micros: u64| {
        let galaxy = core.galaxy();
        let snap = Snapshot {
            seq: *seq,
            time: galaxy.time,
            week: galaxy.week(),
            galaxy,
            events: p.events,
            results: p.results,
            human_battles: human_battles(core, human),
            tactical: p.tactical,
            advances_done: counts.0,
            requests_done: counts.1,
            work_micros: micros,
            script_events: p.script_events,
            saves: p.saves,
            loads: p.loads,
            loading: false,
            diagnostics: match core { Core::Scripted(s) => Some(s.diagnostics()), Core::Plain(_) => None },
        };
        *seq += 1;
        tx.send(Arc::new(snap)).is_ok()
    };
    let first = Published { loads: first_loads, ..Published::default() };
    if !publish(&core, &mut seq, first, (0, 0), 0) {
        return;
    }
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let started = Instant::now();
        let mut p = Published::default();
        requests_done += batch.len() as u64;
        let mut i = 0;
        while i < batch.len() {
            match &batch[i] {
                Request::Shutdown => return,
                Request::Advance(_) => {
                    // Coalesce consecutive advances into one pass.
                    let mut total = 0.0;
                    while let Some(Request::Advance(dt)) = batch.get(i) {
                        total += dt;
                        advances_done += 1;
                        i += 1;
                    }
                    advance(&mut core, human, total, &mut p.events, &mut p.script_events);
                    continue;
                }
                Request::Command { ticket, command } => {
                    let result = core.apply(command.clone());
                    p.results.push(CommandResult { ticket: *ticket, command: command.clone(), result });
                }
                Request::TacticalSetup(id) => p.tactical.extend(core.tactical_setup(*id)),
                Request::ApplyTactical { id, result } => p.events.extend(core.apply_tactical(*id, result)),
                Request::AbandonTactical { id, partial } => p.events.extend(core.abandon_tactical(*id, partial)),
                Request::CancelTactical(id) => {
                    core.cancel_tactical(*id);
                    p.events.extend(core.resolve_auto(*id));
                }
                Request::ResolveBattle { id, mode } => {
                    let evs = match mode {
                        ResolveMode::Auto => core.resolve_auto(*id),
                        ResolveMode::Headless => core.resolve_tactical(*id),
                    };
                    p.events.extend(evs);
                }
                Request::Save(path) => {
                    let result = match &core {
                        Core::Scripted(s) => s.save(path).map_err(|e| e.to_string()),
                        Core::Plain(_) => Err("saving needs the scripted backend (PlayInit::scripted)".to_string()),
                    };
                    p.saves.push(SaveResult { path: path.clone(), result });
                }
                Request::Load(path) => {
                    let result = load_into(&mut core, path, script.as_ref(), &progress);
                    p.loads.push(LoadResult { path: path.clone(), result });
                }
            }
            i += 1;
        }
        let micros = started.elapsed().as_micros() as u64;
        if !publish(&core, &mut seq, p, (advances_done, requests_done), micros) {
            return;
        }
    }
}

#[derive(Default)]
struct Published {
    events: Vec<Event>,
    results: Vec<CommandResult>,
    tactical: Vec<TacticalRequest>,
    script_events: Vec<ScriptEvent>,
    saves: Vec<SaveResult>,
    loads: Vec<LoadResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn involves_checks_membership() {
        let b = PendingBattle { id: 1, planet: 0, fleets: vec![], factions: vec![1, 3], defender: None };
        assert!(involves(Some(3), &b));
        assert!(!involves(Some(2), &b));
        assert!(!involves(None, &b));
    }
}
