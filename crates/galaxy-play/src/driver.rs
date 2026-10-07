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

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

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
    /// Stop the worker.
    Shutdown,
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
}

impl SimDriver {
    /// Moves `sim` onto a new worker thread. `human` is the faction controlled by the player (its
    /// battles are not auto-resolved and it is not run by the AI); every other faction is AI
    /// controlled. With `human = None` the whole galaxy is AI controlled.
    ///
    /// For the player to get a choice the sim must have been built with
    /// `SimConfig::auto_resolve_battles = false` (see [`crate::PlayConfig::sim_config`]).
    pub fn spawn(mut sim: Sim, human: Option<FactionId>) -> SimDriver {
        let world = sim.world().clone();
        let ai: Vec<FactionId> = (0..world.factions.len() as FactionId).filter(|&f| Some(f) != human).collect();
        sim.set_ai_factions(ai);
        let (tx, req_rx) = mpsc::channel();
        let (snap_tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("galaxy-sim".into())
            .spawn(move || worker(sim, human, req_rx, snap_tx))
            .expect("spawn simulation thread");
        SimDriver { tx, rx: Mutex::new(rx), handle: Some(handle), world, human, next_ticket: 1, advances_sent: 0, requests_sent: 0 }
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

fn involves(human: Option<FactionId>, b: &PendingBattle) -> bool {
    human.is_some_and(|h| b.factions.contains(&h))
}

/// Resolves every pending battle without the human with the configured resolver.
fn resolve_ai_battles(sim: &mut Sim, human: Option<FactionId>, events: &mut Vec<Event>) {
    let ids: Vec<u32> = sim.pending_battles().iter().filter(|b| !involves(human, b)).map(|b| b.id).collect();
    for id in ids {
        events.extend(sim.resolve_battle(id).unwrap_or_default());
    }
}

fn human_battles(sim: &Sim, human: Option<FactionId>) -> Vec<PendingBattle> {
    sim.pending_battles().iter().filter(|b| involves(human, b)).cloned().collect()
}

/// Steps in whole simulation ticks so battles are settled (or the player is asked) the moment
/// they appear. Returns early when a battle waits for the human.
fn advance(sim: &mut Sim, human: Option<FactionId>, dt: f32, events: &mut Vec<Event>) {
    if !human_battles(sim, human).is_empty() {
        return;
    }
    let tick = sim.world().config.tick_seconds.max(0.001);
    let mut remaining = dt;
    while remaining > 0.0 && sim.galaxy().winner.is_none() {
        let d = remaining.min(tick);
        remaining -= d;
        events.extend(sim.step(d));
        resolve_ai_battles(sim, human, events);
        if !human_battles(sim, human).is_empty() {
            break;
        }
    }
}

fn worker(mut sim: Sim, human: Option<FactionId>, rx: Receiver<Request>, tx: Sender<Arc<Snapshot>>) {
    let mut seq = 0u64;
    let mut week = 0u64;
    let mut advances_done = 0u64;
    let mut requests_done = 0u64;
    let publish = |sim: &Sim, seq: &mut u64, week: u64, events: Vec<Event>, results: Vec<CommandResult>, tactical: Vec<TacticalRequest>, counts: (u64, u64), micros: u64| {
        let snap = Snapshot {
            seq: *seq,
            galaxy: sim.galaxy().clone(),
            time: sim.galaxy().time,
            week,
            events,
            results,
            human_battles: human_battles(sim, human),
            tactical,
            advances_done: counts.0,
            requests_done: counts.1,
            work_micros: micros,
        };
        *seq += 1;
        tx.send(Arc::new(snap)).is_ok()
    };
    if !publish(&sim, &mut seq, week, Vec::new(), Vec::new(), Vec::new(), (0, 0), 0) {
        return;
    }
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let started = Instant::now();
        let mut events = Vec::new();
        let mut results = Vec::new();
        let mut tactical = Vec::new();
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
                    advance(&mut sim, human, total, &mut events);
                    continue;
                }
                Request::Command { ticket, command } => {
                    let result = sim.apply(command.clone());
                    results.push(CommandResult { ticket: *ticket, command: command.clone(), result });
                }
                Request::TacticalSetup(id) => tactical.extend(sim.tactical_setup(*id)),
                Request::ApplyTactical { id, result } => events.extend(sim.apply_tactical_result(*id, result).unwrap_or_default()),
                Request::CancelTactical(id) => {
                    sim.cancel_tactical(*id);
                    events.extend(sim.resolve_battle_auto(*id).unwrap_or_default());
                }
                Request::ResolveBattle { id, mode } => {
                    let evs = match mode {
                        ResolveMode::Auto => sim.resolve_battle_auto(*id),
                        ResolveMode::Headless => sim.resolve_battle_tactical(*id),
                    };
                    events.extend(evs.unwrap_or_default());
                }
            }
            i += 1;
        }
        for e in &events {
            if let Event::WeekEnded { week: w } = e {
                week = *w;
            }
        }
        let micros = started.elapsed().as_micros() as u64;
        if !publish(&sim, &mut seq, week, events, results, tactical, (advances_done, requests_done), micros) {
            return;
        }
    }
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


