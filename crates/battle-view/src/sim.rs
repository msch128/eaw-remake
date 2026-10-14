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
    /// Ship id this hardpoint is shooting at (its current volley, else the ship's auto target); `u32::MAX` = none. Turrets turn toward it.
    pub target: u32,
    /// Hardpoint of the target being aimed at (`u16::MAX` = the ship's centre).
    pub target_hp: u16,
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
    /// Setup group index.
    pub group: u32,
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
    /// Whether this ship can jump to hyperspace during a retreat (has functional drives).
    pub can_retreat: bool,
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
    /// Population per side as the unit cap counts it (`Battle::pop_in_play`: what lives plus what is on its way).
    pub pop: Vec<i32>,
    /// Once the battle is over: per side the ships it lost, by definition `(def, count)` (destroyed, not retreated; spawned squadrons and stations included).
    pub losses: Vec<Vec<(battle_sim::ShipId, u32)>>,
    /// Per side: seconds until its retreat jump (`None` when no retreat was ordered).
    pub retreat_jump_in: Vec<Option<f32>>,
    /// Initial spawn / arrival yaw for each side.
    pub spawn_yaw: Vec<f32>,
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
                group: s.group,
                order: s.order,
                path: s.path.clone(),
                seen_by: (0..battle.side_count().min(8)).filter(|&side| battle.is_visible_to(side as u8, s.id)).fold(0u8, |m, side| m | (1 << side)),
                hps: s
                    .hps
                    .iter()
                    .map(|h| HpSnap {
                        alive: h.alive,
                        health: h.health,
                        target: if h.pulses_left > 0 || s.target == battle_sim::NONE { h.target } else { s.target },
                        target_hp: s.target_hp,
                    })
                    .collect(),
                abilities: s.abilities.iter().map(|a| (a.active, a.cooldown)).collect(),
                autocast: s.abilities.iter().map(|a| a.autocast).collect(),
                reserve_squadrons: s.reserve.iter().map(|r| r.1).sum(),
                can_retreat: battle.can_retreat(s.id),
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
        pop: (0..battle.side_count() as u8).map(|s| battle.pop_in_play(s)).collect(),
        losses: if battle.is_over() { lost_ships(battle) } else { Vec::new() },
        retreat_jump_in: (0..battle.side_count() as u8).map(|s| battle.retreat_jump_in(s)).collect(),
        spawn_yaw: (0..battle.side_count() as u8).map(|s| battle.spawn_yaw(s)).collect(),
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
            let ready = have(&q, tick + 1) || q.done;
            if ready || !block || Instant::now() > deadline {
                let get = |q: &Queue, t: u64| {
                    if q.done && first.is_some_and(|f| t >= f + q.snaps.len() as u64) {
                        return q.snaps.back().cloned();
                    }
                    first
                        .filter(|&f| t >= f)
                        .and_then(|f| q.snaps.get((t - f) as usize))
                        .cloned()
                };
                return (get(&q, tick), if q.done && !have(&q, tick + 1) { None } else { get(&q, tick + 1) });
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




/// The units every side lost, by definition: a squadron counts once when all of its fighters are dead and none retreated (not once per
/// fighter); ships without a squadron count one by one.
fn lost_ships(battle: &Battle) -> Vec<Vec<(battle_sim::ShipId, u32)>> {
    lost_units(battle.side_count(), battle.ships.iter().map(|s| (s.side, s.def, s.squad, s.alive, s.retreated)))
}

/// `ships` is `(side, definition, squad, alive, retreated)` per ship.
fn lost_units(
    sides: usize,
    ships: impl Iterator<Item = (u8, battle_sim::ShipId, u32, bool, bool)>,
) -> Vec<Vec<(battle_sim::ShipId, u32)>> {
    let mut out: Vec<std::collections::BTreeMap<battle_sim::ShipId, u32>> = vec![Default::default(); sides];
    // squad -> (side, definition of the first member, any member alive or retreated)
    let mut squads: std::collections::BTreeMap<u32, (u8, battle_sim::ShipId, bool)> = Default::default();
    for (side, def, squad, alive, retreated) in ships {
        if squad == battle_sim::NONE {
            if !alive && !retreated {
                if let Some(m) = out.get_mut(side as usize) {
                    *m.entry(def).or_default() += 1;
                }
            }
            continue;
        }
        squads.entry(squad).or_insert((side, def, false)).2 |= alive || retreated;
    }
    for (side, def, present) in squads.into_values() {
        if !present {
            if let Some(m) = out.get_mut(side as usize) {
                *m.entry(def).or_default() += 1;
            }
        }
    }
    out.into_iter().map(|m| m.into_iter().collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squadron_counts_once_and_station_counts() {
        let none = battle_sim::NONE;
        // side 0: squad 7 of 4 fighters (def 5) all dead, squad 8 of 2 with one alive; side 1: a dead station (def 9) and a retreated ship
        let mut ships: Vec<(u8, u32, u32, bool, bool)> = (0..4).map(|_| (0, 5, 7, false, false)).collect();
        ships.extend([(0, 5, 8, false, false), (0, 5, 8, true, false), (1, 9, none, false, false), (1, 3, none, false, true)]);
        let lost = lost_units(2, ships.into_iter());
        assert_eq!(lost[0], vec![(5, 1)]);
        assert_eq!(lost[1], vec![(9, 1)]);
    }

    #[test]
    fn fetch_after_done_returns_terminal_snapshot_without_blocking() {
        let shared = Arc::new(Shared::default());
        let (tx, _rx) = channel::<(u8, Command)>();
        let handle = SimHandle {
            shared: shared.clone(),
            commands: tx,
            join: None,
            dt: 0.1,
        };

        {
            let mut q = shared.state.lock().unwrap();
            let mut snap = Snapshot::default();
            snap.tick = 10;
            snap.over = true;
            q.snaps.push_back(Arc::new(snap));
            q.done = true;
        }

        let t0 = Instant::now();
        let (cur, next) = handle.fetch(20, true);
        assert!(t0.elapsed().as_millis() < 500, "fetch must not block when done");
        assert!(cur.is_some(), "terminal snapshot should be returned");
        assert_eq!(cur.unwrap().tick, 10);
        assert!(next.is_none());
    }
}
