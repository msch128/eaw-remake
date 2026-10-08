//! Runs the battle simulation on a worker thread and publishes one immutable snapshot per step.
//!
//! The render side keeps a virtual clock and asks for the snapshots around it; the worker only
//! simulates a few ticks ahead of that clock, so the simulation cost never lands on the render
//! thread and the result stays deterministic (the clock, not the wall time, decides which steps run).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use battle_sim::{Battle, Command, Event, ProjId, ShipId, Vec3};

/// State of one hardpoint in a snapshot.
#[derive(Debug, Clone, Copy)]
pub struct HpSnap {
    /// False once destroyed.
    pub alive: bool,
    /// Remaining health.
    pub health: f32,
}

/// One ship at the end of a step.
#[derive(Debug, Clone)]
pub struct ShipSnap {
    /// Stable id (index in `Battle::ships`).
    pub id: u32,
    /// Definition.
    pub def: ShipId,
    /// Side.
    pub side: u8,
    /// Sim-space position.
    pub pos: Vec3,
    /// Heading in radians.
    pub yaw: f32,
    /// Hull points.
    pub hull: f32,
    /// Shield points.
    pub shield: f32,
    /// Squadron index for fighters, `u32::MAX` otherwise.
    pub squad: u32,
    /// Bit s set = side s sees this ship (fog of war; everything is seen without it).
    pub seen_by: u8,
    /// Current order (destination of the move line).
    pub order: battle_sim::Order,
    /// Planned route of a big ship on the move: the waypoints still ahead in sim space, the last being the destination. Empty when the
    /// ship is not going to a place. Draw the move line from the ship through these points.
    pub path: Vec<Vec3>,
    /// Hardpoint states (empty for fighters).
    pub hps: Vec<HpSnap>,
    /// Ability state per ability of the definition: (seconds of effect left, seconds until usable).
    pub abilities: Vec<(f32, f32)>,
    /// Squadrons a carrier or station still holds in reserve (replacements for destroyed ones); 0 for everything else.
    pub reserve_squadrons: u32,
    /// Automatic use per ability (same order as `abilities`): the button shows the autocast state.
    pub autocast: Vec<bool>,
}

/// One projectile at the end of a step.
#[derive(Debug, Clone, Copy)]
pub struct ProjSnap {
    /// Sim-space position.
    pub pos: Vec3,
    /// Velocity in units per second.
    pub vel: Vec3,
    /// Definition.
    pub def: ProjId,
}

/// The battle state after `tick` steps.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Step counter (0 = initial state).
    pub tick: u64,
    /// Battle time in seconds.
    pub time: f32,
    /// Living ships, sorted by id.
    pub ships: Vec<ShipSnap>,
    /// Projectiles in flight.
    pub projectiles: Vec<ProjSnap>,
    /// Events of the step that produced this snapshot.
    pub events: Vec<Event>,
    /// True once the battle ended.
    pub over: bool,
    /// Winning side, if any.
    pub winner: Option<u8>,
    /// Final result with survivors per group, once the battle is over.
    pub result: Option<battle_sim::BattleResult>,
    /// Living ships per side (fighters included).
    pub alive: Vec<u32>,
    /// Wall-clock milliseconds the worker needed for the step.
    pub step_ms: f32,
}

impl Snapshot {
    /// Ship by id.
    pub fn ship(&self, id: u32) -> Option<&ShipSnap> {
        self.ships
            .binary_search_by_key(&id, |s| s.id)
            .ok()
            .map(|i| &self.ships[i])
    }
}

fn snapshot(battle: &Battle, events: Vec<Event>, step_ms: f32) -> Snapshot {
    let result = battle.is_over().then(|| battle.result());
    let mut alive = vec![0u32; battle.side_count()];
    let ships: Vec<ShipSnap> = battle
        .ships
        .iter()
        .filter(|s| s.alive && !s.retreated)
        .map(|s| {
            if let Some(a) = alive.get_mut(s.side as usize) {
                *a += 1;
            }
            ShipSnap {
                id: s.id,
                def: s.def,
                side: s.side,
                pos: s.pos,
                yaw: s.yaw,
                hull: s.hull,
                shield: s.shield,
                squad: s.squad,
                order: s.order,
                path: s.path.clone(),
                seen_by: (0..battle.side_count().min(8)).filter(|&side| battle.is_visible_to(side as u8, s.id)).fold(0u8, |m, side| m | (1 << side)),
                hps: s
                    .hps
                    .iter()
                    .map(|h| HpSnap {
                        alive: h.alive,
                        health: h.health,
                    })
                    .collect(),
                abilities: s.abilities.iter().map(|a| (a.active, a.cooldown)).collect(),
                autocast: s.abilities.iter().map(|a| a.autocast).collect(),
                reserve_squadrons: s.reserve.iter().map(|r| r.1).sum(),
            }
        })
        .collect();
    Snapshot {
        tick: battle.tick(),
        time: battle.time(),
        ships,
        projectiles: battle
            .projectiles
            .iter()
            .map(|p| ProjSnap {
                pos: p.pos,
                vel: p.vel,
                def: p.def,
            })
            .collect(),
        events,
        over: battle.is_over(),
        winner: result.as_ref().and_then(|r| r.winner),
        result,
        alive,
        step_ms,
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<Queue>,
    snap_cv: Condvar,
    work_cv: Condvar,
    /// Highest tick the worker has produced (for lock-free progress checks).
    produced: AtomicU64,
}

#[derive(Default)]
struct Queue {
    snaps: VecDeque<Arc<Snapshot>>,
    events: VecDeque<(u64, Vec<Event>)>,
    target: u64,
    stop: bool,
    done: bool,
    /// Result so far, refreshed once per battle second (for abandoning a half-fought battle).
    progress: Option<battle_sim::BattleResult>,
}

/// Handle to the simulation worker.
pub struct SimHandle {
    shared: Arc<Shared>,
    commands: Sender<(u8, Command)>,
    join: Option<JoinHandle<()>>,
    /// Seconds per step.
    pub dt: f32,
}

impl SimHandle {
    /// Spawns the worker; the initial snapshot (tick 0) is available immediately after the first
    /// [`SimHandle::wait_for`].
    pub fn spawn(mut battle: Battle) -> Self {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel::<(u8, Command)>();
        let dt = battle.config().dt;
        let worker = shared.clone();
        let join = std::thread::Builder::new()
            .name("battle-sim".into())
            .spawn(move || {
                {
                    let mut q = worker.state.lock().unwrap();
                    q.snaps.push_back(Arc::new(snapshot(&battle, Vec::new(), 0.0)));
                    worker.snap_cv.notify_all();
                }
                loop {
                    {
                        let mut q = worker.state.lock().unwrap();
                        while !q.stop && (q.done || battle.tick() >= q.target) {
                            q = worker.work_cv.wait(q).unwrap();
                        }
                        if q.stop {
                            return;
                        }
                    }
                    while let Ok((side, cmd)) = rx.try_recv() {
                        let _ = battle.command(side, cmd);
                    }
                    let t0 = Instant::now();
                    let events = battle.step().to_vec();
                    let mut snap = snapshot(&battle, events, t0.elapsed().as_secs_f32() * 1000.0);
                    let over = snap.over;
                    worker.produced.store(snap.tick, Ordering::Release);
                    let mut q = worker.state.lock().unwrap();
                    if snap.tick.is_multiple_of(30) {
                        q.progress = Some(battle.result());
                    }
                    if !snap.events.is_empty() {
                        q.events.push_back((snap.tick, std::mem::take(&mut snap.events)));
                    }
                    q.snaps.push_back(Arc::new(snap));
                    q.done = over;
                    worker.snap_cv.notify_all();
                }
            })
            .expect("spawn battle-sim thread");
        Self {
            shared,
            commands: tx,
            join: Some(join),
            dt,
        }
    }

    /// Sends a player command; it is applied before the next step.
    pub fn command(&self, side: u8, cmd: Command) {
        let _ = self.commands.send((side, cmd));
    }

    /// Asks the worker to simulate up to (and including) `tick`.
    pub fn request(&self, tick: u64) {
        let mut q = self.shared.state.lock().unwrap();
        if tick > q.target {
            q.target = tick;
            self.shared.work_cv.notify_all();
        }
    }

    /// Snapshots for `tick` and `tick + 1`, dropping older ones. With `block` the call waits for
    /// both (or for the end of the battle); without it missing snapshots are `None`.
    pub fn fetch(&self, tick: u64, block: bool) -> (Option<Arc<Snapshot>>, Option<Arc<Snapshot>>) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut q = self.shared.state.lock().unwrap();
        loop {
            while q.snaps.len() > 1 && q.snaps[1].tick <= tick {
                q.snaps.pop_front();
            }
            let first = q.snaps.front().map(|s| s.tick);
            let have = |q: &Queue, t: u64| first.is_some_and(|f| t >= f && ((t - f) as usize) < q.snaps.len());
            let ready = have(&q, tick + 1) || q.done && have(&q, tick);
            if ready || !block || Instant::now() > deadline {
                let get = |q: &Queue, t: u64| {
                    first
                        .filter(|&f| t >= f)
                        .and_then(|f| q.snaps.get((t - f) as usize))
                        .cloned()
                };
                return (get(&q, tick), get(&q, tick + 1));
            }
            let (g, _) = self
                .shared
                .snap_cv
                .wait_timeout(q, Duration::from_millis(50))
                .unwrap();
            q = g;
        }
    }

    /// Events of all steps up to and including `tick` that were not drained yet.
    pub fn drain_events(&self, tick: u64) -> Vec<Event> {
        let mut q = self.shared.state.lock().unwrap();
        let mut out = Vec::new();
        while q.events.front().is_some_and(|e| e.0 <= tick) {
            out.extend(q.events.pop_front().unwrap().1);
        }
        out
    }

    /// Newest snapshot produced so far.
    pub fn latest(&self) -> Option<Arc<Snapshot>> {
        self.shared.state.lock().unwrap().snaps.back().cloned()
    }

    /// The battle's result so far, at most one second old (survivors per group); `None` before the first second.
    pub fn progress(&self) -> Option<battle_sim::BattleResult> {
        self.shared.state.lock().unwrap().progress.clone()
    }

    /// Highest tick produced so far.
    pub fn produced(&self) -> u64 {
        self.shared.produced.load(Ordering::Acquire)
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().stop = true;
        self.shared.work_cv.notify_all();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}



