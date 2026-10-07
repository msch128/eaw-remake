//! Task forces: the groups of units and structures a plan script assembles, moves and releases.
//!
//! Plans see them as objects (`SpaceForce`, `ReserveForce`, `StructureForce`, ...) with methods that
//! return *command blocks*. This module implements the space-relevant methods on top of [`AiSim`]:
//!
//! * `Produce_Force(stage)` composes the force from the requirements (see [`crate::requirements`]), takes
//!   matching idle ships from the free store, queues the missing ones at planets that can build them and
//!   gathers everything at the stage planet. Structure forces queue structures at the target / stage.
//! * `Move_To(planet)` sends the assembled fleet and finishes on arrival (or when it is destroyed).
//! * `Invade()` finishes when the planet changed hands. Ground forces are not simulated: they are
//!   instantly "assembled" and `Launch_Units` / `Land_Units` / `Form_Units` succeed at once.
//! * `Release_Forces()` returns the ships to the free store.

use std::collections::{HashMap, HashSet, VecDeque};

use alamo_lua::{HostValue, PlanEvent, TaskForceSpec};

use crate::perception::Snapshot;
use crate::requirements::{self, Requirements, is_category};
use crate::sim::{AiSim, FleetId, PlanetId, PlayerId, Refused};

/// What a task force consists of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceKind {
    /// Ships (SpaceForce, ReserveForce, ...).
    Units,
    /// Ground units: not simulated.
    Ground,
    /// Structures (StructureForce, BaseForce).
    Structure,
}

/// One task force.
#[derive(Debug)]
pub struct TaskForce {
    /// Id handed to scripts.
    pub id: u64,
    /// Plan instance.
    pub plan: u64,
    /// Owner.
    pub player: PlayerId,
    /// Global name in the plan (`SpaceForce`).
    pub name: String,
    /// Kind.
    pub kind: ForceKind,
    /// Requirements.
    pub reqs: Requirements,
    /// Target planet of the plan.
    pub target: Option<PlanetId>,
    /// Numeric plan globals.
    pub numbers: HashMap<String, f64>,
    /// The assembled fleet.
    pub fleet: Option<FleetId>,
    /// Fleets that belong to the force but are not merged yet.
    pub pieces: Vec<FleetId>,
    /// Staging planet.
    pub stage: Option<PlanetId>,
    /// `Set_As_Goal_System_Removable`.
    pub removable: bool,
    /// `Set_Plan_Result`.
    pub result: Option<bool>,
    /// The force did something in the game (queued production, moved ships, captured).
    pub worked: bool,
    had_units: bool,
}

/// State of a command block.
#[derive(Debug, Clone, Copy, Default)]
pub struct Block {
    /// Done?
    pub finished: bool,
    /// Outcome once finished.
    pub result: Option<bool>,
}

#[derive(Debug)]
struct Pending {
    unit: String,
    planet: PlanetId,
    queued_at: f64,
}

#[derive(Debug)]
enum Job {
    Units(UnitJob),
    Structures(StructJob),
    Move { block: u64, tf: u64, to: PlanetId, started: f64, issued: f64, retries: u32 },
    Invade { block: u64, tf: u64, target: PlanetId, deadline: f64 },
}

#[derive(Debug)]
struct UnitJob {
    block: u64,
    tf: u64,
    stage: PlanetId,
    wanted: Vec<String>,
    pending: Vec<Pending>,
    started: f64,
    deadline: f64,
    refused: HashMap<String, u32>,
}

#[derive(Debug)]
struct StructJob {
    block: u64,
    tf: u64,
    planet: PlanetId,
    /// Remaining groups; the front one is being worked on.
    groups: VecDeque<crate::requirements::Group>,
    /// Name queued for the current group.
    queued: Option<String>,
    /// Something was built (or stood there already).
    built: bool,
    started: f64,
    deadline: f64,
}

/// Counters for reports.
#[derive(Debug, Default, Clone)]
pub struct TaskForceStats {
    /// Task forces created, by name.
    pub created: HashMap<String, u32>,
    /// Ships taken from the free store.
    pub free_store_claims: u32,
    /// Ships queued for production.
    pub ships_queued: u32,
    /// Structures queued.
    pub structures_queued: u32,
    /// Produce jobs that failed.
    pub production_failures: u32,
    /// Moves ordered.
    pub moves: u32,
    /// Moves that arrived.
    pub arrivals: u32,
    /// Forces wiped out.
    pub forces_lost: u32,
    /// Method calls the engine does not implement.
    pub unimplemented: HashMap<String, u32>,
    /// Credits spent through plans.
    pub credits_spent: f64,
}

/// All task forces and their pending commands.
#[derive(Default)]
pub struct TaskForces {
    pub(crate) forces: HashMap<u64, TaskForce>,
    blocks: HashMap<u64, Block>,
    jobs: Vec<Job>,
    owned_fleets: HashSet<FleetId>,
    next_id: u64,
    next_block: u64,
    events: Vec<PlanEvent>,
    /// Statistics.
    pub stats: TaskForceStats,
}

fn kind_of(name: &str) -> ForceKind {
    let l = name.to_ascii_lowercase();
    if l.contains("ground") {
        ForceKind::Ground
    } else if l.contains("structure") || l.contains("base") {
        ForceKind::Structure
    } else {
        ForceKind::Units
    }
}

fn hop_distances(snap: &Snapshot, from: PlanetId) -> Vec<u32> {
    let mut d = vec![u32::MAX; snap.adjacency.len()];
    let mut q = VecDeque::new();
    d[from as usize] = 0;
    q.push_back(from);
    while let Some(p) = q.pop_front() {
        for &n in &snap.adjacency[p as usize] {
            if d[n as usize] == u32::MAX {
                d[n as usize] = d[p as usize] + 1;
                q.push_back(n);
            }
        }
    }
    d
}

fn next_rand(rng: &mut u64) -> f64 {
    let mut x = *rng;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *rng = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

impl TaskForces {
    /// Plan callbacks accumulated since the last call.
    pub fn take_events(&mut self) -> Vec<PlanEvent> {
        std::mem::take(&mut self.events)
    }

    /// Creates a task force from a plan's declaration.
    pub fn create(&mut self, sim: &dyn AiSim, spec: &TaskForceSpec) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let reqs = requirements::parse(&spec.requirements);
        let kind = kind_of(&spec.name);
        *self.stats.created.entry(spec.name.clone()).or_default() += 1;
        self.forces.insert(
            id,
            TaskForce {
                id,
                plan: spec.plan,
                player: spec.player,
                name: spec.name.clone(),
                kind,
                reqs,
                target: spec.target.and_then(|o| sim.object_planet(o)),
                numbers: spec.numbers.iter().cloned().collect(),
                fleet: None,
                pieces: Vec::new(),
                stage: None,
                removable: true,
                result: None,
                worked: false,
                had_units: false,
            },
        );
        id
    }

    /// Whether any task force of the plan changed the game state.
    pub fn plan_worked(&self, plan: u64) -> bool {
        self.forces.values().any(|t| t.plan == plan && t.worked)
    }

    /// Plans whose task forces reported `Set_Plan_Result(false)`.
    pub fn plan_failed(&self, plan: u64) -> bool {
        self.forces.values().any(|t| t.plan == plan && t.result == Some(false))
    }

    /// Frees everything a plan holds (called when the plan ends).
    pub fn drop_plan(&mut self, plan: u64) {
        let ids: Vec<u64> = self.forces.values().filter(|t| t.plan == plan).map(|t| t.id).collect();
        for id in &ids {
            self.release(*id);
        }
        self.forces.retain(|_, t| t.plan != plan);
        self.jobs.retain(|j| !ids.contains(&job_tf(j)));
    }

    fn release(&mut self, tf: u64) {
        if let Some(t) = self.forces.get_mut(&tf) {
            if let Some(f) = t.fleet.take() {
                self.owned_fleets.remove(&f);
            }
            for p in t.pieces.drain(..) {
                self.owned_fleets.remove(&p);
            }
        }
    }

    /// A block that is already finished.
    pub fn immediate_block(&mut self, result: bool) -> u64 {
        let b = self.new_block();
        self.finish(b, result);
        b
    }

    fn new_block(&mut self) -> u64 {
        self.next_block += 1;
        self.blocks.insert(self.next_block, Block::default());
        self.next_block
    }

    fn finish(&mut self, block: u64, result: bool) {
        self.blocks.insert(block, Block { finished: true, result: Some(result) });
    }

    /// State of a block.
    pub fn block_status(&self, block: u64) -> (bool, Option<bool>) {
        self.blocks.get(&block).map_or((true, Some(false)), |b| (b.finished, b.result))
    }

    /// Picks the staging planet for a force: for plans with a target, the owned planet nearest to it
    /// (preferring planets with more build capacity); otherwise the richest owned planet.
    fn choose_stage(&self, snap: &Snapshot, tf: &TaskForce) -> Option<PlanetId> {
        let owned = &snap.owned[tf.player as usize];
        if owned.is_empty() {
            return None;
        }
        if tf.kind == ForceKind::Structure {
            return tf.target.or_else(|| owned.first().copied());
        }
        match tf.target {
            Some(t) => {
                let dist = hop_distances(snap, t);
                owned
                    .iter()
                    .copied()
                    .min_by(|&a, &b| {
                        let key = |p: PlanetId| (dist[p as usize], -(snap.structures[p as usize].len() as i64));
                        key(a).cmp(&key(b))
                    })
            }
            None => owned.iter().copied().max_by(|&a, &b| snap.income[a as usize].total_cmp(&snap.income[b as usize])),
        }
    }

    /// Dispatches a method call from Lua.
    pub fn call(
        &mut self,
        sim: &mut dyn AiSim,
        snap: &Snapshot,
        rng: &mut u64,
        tf_id: u64,
        method: &str,
        args: &[HostValue],
    ) -> Option<HostValue> {
        let now = sim.time();
        let kind = self.forces.get(&tf_id)?.kind;
        let obj_planet = |sim: &dyn AiSim, v: Option<&HostValue>| -> Option<PlanetId> {
            match v {
                Some(HostValue::Object(o)) => sim.object_planet(*o),
                _ => None,
            }
        };
        Some(match method {
            "Set_As_Goal_System_Removable" => {
                if let Some(t) = self.forces.get_mut(&tf_id) {
                    t.removable = matches!(args.first(), Some(HostValue::Bool(true)));
                }
                HostValue::Nil
            }
            "Set_Plan_Result" => {
                if let Some(t) = self.forces.get_mut(&tf_id) {
                    t.result = Some(matches!(args.first(), Some(HostValue::Bool(true))));
                }
                HostValue::Nil
            }
            "Get_Stage" => {
                let t = self.forces.get(&tf_id)?;
                let stage = t.stage.or_else(|| self.choose_stage(snap, t));
                self.forces.get_mut(&tf_id)?.stage = stage;
                match stage {
                    Some(s) => HostValue::Object(sim.planet_object(s)),
                    None => HostValue::Nil,
                }
            }
            "Are_All_Units_On_Free_Store" => {
                if kind != ForceKind::Units {
                    return Some(HostValue::Bool(true));
                }
                let t = self.forces.get(&tf_id)?;
                let wanted = self.compose(sim, snap, t, rng);
                HostValue::Bool(self.free_store_covers(sim, t.player, &wanted))
            }
            "Produce_Force" => {
                let block = self.new_block();
                let stage_arg = obj_planet(sim, args.first());
                self.start_produce(sim, snap, rng, tf_id, block, stage_arg, now);
                HostValue::Block(block)
            }
            "Move_To" => {
                let block = self.new_block();
                let to = obj_planet(sim, args.first());
                match (kind, to) {
                    (ForceKind::Units, Some(to)) => {
                        let moved = self.start_move(sim, tf_id, to);
                        if moved {
                            self.jobs.push(Job::Move { block, tf: tf_id, to, started: now, issued: now, retries: 0 });
                        } else {
                            self.finish(block, false);
                        }
                    }
                    _ => self.finish(block, kind != ForceKind::Units),
                }
                HostValue::Block(block)
            }
            "Invade" => {
                let block = self.new_block();
                let t = self.forces.get(&tf_id)?;
                let target = t.target;
                let player = t.player;
                match target {
                    Some(target) if sim.planet_owner(target) != Some(player) => {
                        // Troops land at once on a planet nobody defends; otherwise the fleet in orbit must win first.
                        let defended = sim.fleets_at(target).iter().any(|f| f.owner != player);
                        if sim.planet_owner(target).is_none() && !defended && sim.capture_planet(player, target).is_ok() {
                            if let Some(t) = self.forces.get_mut(&tf_id) {
                                t.worked = true;
                            }
                            self.finish(block, true);
                        } else {
                            self.jobs.push(Job::Invade { block, tf: tf_id, target, deadline: now + 120.0 });
                        }
                    }
                    _ => self.finish(block, true),
                }
                HostValue::Block(block)
            }
            "Launch_Units" | "Land_Units" | "Form_Units" => {
                let block = self.new_block();
                self.finish(block, true);
                HostValue::Block(block)
            }
            "Release_Forces" => {
                self.release(tf_id);
                HostValue::Nil
            }
            "Release_Unit" | "Test_Target_Contrast" | "Attack_Target" | "Attack_Move" | "Guard_Target" | "Set_Targeting_Priorities" => HostValue::Nil,
            "Get_Force_Count" => {
                let t = self.forces.get(&tf_id)?;
                let n = t.fleet.into_iter().chain(t.pieces.iter().copied()).filter_map(|f| sim.fleet(f)).map(|f| f.units.iter().map(|u| u.1).sum::<u32>()).sum::<u32>();
                HostValue::Num(f64::from(n))
            }
            "Get_Unit_Table" => HostValue::List(Vec::new()),
            "Get_Self_Threat_Max" | "Get_Self_Threat_Sum" => HostValue::Num(0.0),
            "Is_Raid_Capable" => HostValue::Bool(false),
            _ => {
                *self.stats.unimplemented.entry(method.to_string()).or_default() += 1;
                return None;
            }
        })
    }

    // ---- composition ---------------------------------------------------------------------------------

    /// The list of unit types a force should consist of.
    fn compose(&self, sim: &dyn AiSim, snap: &Snapshot, t: &TaskForce, rng: &mut u64) -> Vec<String> {
        let mut candidates: Vec<(String, String, f64)> = Vec::new(); // name, category, power
        for name in sim.buildable_units(t.player) {
            if t.reqs.excluded.contains(&name.to_ascii_lowercase()) {
                continue;
            }
            let Some(info) = sim.unit_info(&name) else { continue };
            if info.is_hero && t.reqs.deny_hero {
                continue;
            }
            candidates.push((name, info.category.to_ascii_uppercase(), info.power.max(1.0)));
        }
        let matches = |c: &(String, String, f64), types: &[String]| types.iter().any(|ty| ty.eq_ignore_ascii_case(&c.0) || (is_category(ty) && ty.eq_ignore_ascii_case(&c.1)));
        let pick = |rng: &mut u64, pool: &[&(String, String, f64)]| -> Option<String> {
            let total: f64 = pool.iter().map(|c| c.2.sqrt()).sum();
            if pool.is_empty() || total <= 0.0 {
                return None;
            }
            let mut r = next_rand(rng) * total;
            for c in pool {
                r -= c.2.sqrt();
                if r <= 0.0 {
                    return Some(c.0.clone());
                }
            }
            pool.last().map(|c| c.0.clone())
        };
        let mut wanted = Vec::new();
        // Count groups first.
        for g in t.reqs.groups.iter().filter(|g| g.percent.is_none()) {
            let pool: Vec<_> = candidates.iter().filter(|c| matches(c, &g.types)).collect();
            for _ in 0..g.max.min(8) {
                if let Some(n) = pick(rng, &pool) {
                    wanted.push(n);
                }
            }
        }
        // Percentage groups: fill until size and (for attacks) power requirements are met.
        let pct: Vec<_> = t.reqs.groups.iter().filter(|g| g.percent.is_some()).collect();
        if !pct.is_empty() {
            let (scale_min, scale_max) = (
                t.numbers.get("MinContrastScale").copied().unwrap_or(1.0),
                t.numbers.get("MaxContrastScale").copied().unwrap_or(1.5),
            );
            let scale = scale_min + (scale_max - scale_min) * next_rand(rng);
            let defence = t.target.map_or(0.0, |tp| {
                snap.power_at(tp, |o| snap.enemy[t.player as usize][o as usize]) + snap.owner[tp as usize].filter(|o| snap.enemy[t.player as usize][*o as usize]).map_or(0.0, |_| f64::from(snap.starbase[tp as usize].0) * 400.0)
            });
            let needed = defence * scale;
            let cap = t.reqs.max_total.unwrap_or(24) as usize;
            let min_total = t.reqs.min_total.max(1) as usize;
            let mut power = 0.0;
            let mut guard = 0;
            while (wanted.len() < min_total || power < needed) && wanted.len() < cap && guard < 200 {
                guard += 1;
                let total_pct: f64 = pct.iter().filter_map(|g| g.percent).sum::<f64>().max(1.0);
                let mut r = next_rand(rng) * total_pct;
                let mut chosen = pct[0];
                for g in &pct {
                    r -= g.percent.unwrap_or(0.0);
                    if r <= 0.0 {
                        chosen = g;
                        break;
                    }
                }
                let pool: Vec<_> = candidates.iter().filter(|c| matches(c, &chosen.types)).collect();
                if let Some(n) = pick(rng, &pool) {
                    power += candidates.iter().find(|c| c.0 == n).map_or(0.0, |c| c.2);
                    wanted.push(n);
                } else {
                    break;
                }
            }
        }
        wanted
    }

    fn free_ships(&self, sim: &dyn AiSim, player: PlayerId) -> HashMap<(PlanetId, String), (FleetId, u32)> {
        let mut out: HashMap<(PlanetId, String), (FleetId, u32)> = HashMap::new();
        for f in sim.fleets_of(player) {
            let Some(at) = f.at else { continue };
            if self.owned_fleets.contains(&f.id) {
                continue;
            }
            for (u, c) in &f.units {
                let e = out.entry((at, u.to_ascii_lowercase())).or_insert((f.id, 0));
                e.1 += c;
            }
        }
        out
    }

    fn free_store_covers(&self, sim: &dyn AiSim, player: PlayerId, wanted: &[String]) -> bool {
        let free = self.free_ships(sim, player);
        let mut have: HashMap<String, u32> = HashMap::new();
        for ((_, u), (_, c)) in &free {
            *have.entry(u.clone()).or_default() += c;
        }
        let mut need: HashMap<String, u32> = HashMap::new();
        for w in wanted {
            *need.entry(w.to_ascii_lowercase()).or_default() += 1;
        }
        !wanted.is_empty() && need.iter().all(|(u, n)| have.get(u).copied().unwrap_or(0) >= *n)
    }

    // ---- production --------------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn start_produce(&mut self, sim: &mut dyn AiSim, snap: &Snapshot, rng: &mut u64, tf_id: u64, block: u64, stage_arg: Option<PlanetId>, now: f64) {
        let Some(t) = self.forces.get(&tf_id) else { return self.finish(block, false) };
        match t.kind {
            ForceKind::Ground => return self.finish(block, true),
            ForceKind::Structure => {
                let planet = t.target.or(stage_arg).or_else(|| self.choose_stage(snap, t));
                let Some(planet) = planet else { return self.finish(block, false) };
                let groups: VecDeque<_> = t.reqs.groups.iter().filter(|g| g.max > 0).cloned().collect();
                if groups.is_empty() {
                    return self.finish(block, true);
                }
                self.jobs.push(Job::Structures(StructJob { block, tf: tf_id, planet, groups, queued: None, built: false, started: now, deadline: now + 900.0 }));
            }
            ForceKind::Units => {
                let stage = stage_arg.or(t.stage).or_else(|| self.choose_stage(snap, t));
                let Some(stage) = stage else { return self.finish(block, false) };
                let wanted = self.compose(sim, snap, t, rng);
                if let Some(t) = self.forces.get_mut(&tf_id) {
                    t.stage = Some(stage);
                }
                if wanted.is_empty() {
                    return self.finish(block, false);
                }
                self.jobs.push(Job::Units(UnitJob {
                    block,
                    tf: tf_id,
                    stage,
                    wanted,
                    pending: Vec::new(),
                    started: now,
                    deadline: now + 1200.0,
                    refused: HashMap::new(),
                }));
            }
        }
    }

    fn start_move(&mut self, sim: &mut dyn AiSim, tf_id: u64, to: PlanetId) -> bool {
        let Some(t) = self.forces.get_mut(&tf_id) else { return false };
        let Some(fleet) = t.fleet else { return false };
        let player = t.player;
        if sim.fleet(fleet).is_some_and(|f| f.at == Some(to)) {
            return true;
        }
        self.stats.moves += 1;
        sim.move_fleet(player, fleet, to).is_ok()
    }

    /// Advances all pending commands.
    pub fn tick(&mut self, sim: &mut dyn AiSim, snap: &Snapshot, rng: &mut u64) {
        let now = sim.time();
        let jobs = std::mem::take(&mut self.jobs);
        let mut keep = Vec::new();
        for job in jobs {
            match job {
                Job::Units(j) => {
                    if let Some(j) = self.tick_units(sim, snap, rng, j, now) {
                        keep.push(Job::Units(j));
                    }
                }
                Job::Structures(j) => {
                    if let Some(j) = self.tick_structures(sim, j, now) {
                        keep.push(Job::Structures(j));
                    }
                }
                Job::Move { block, tf, to, started, issued, retries } => {
                    match self.tick_move(sim, block, tf, to, started, issued, retries, now) {
                        Some(j) => keep.push(j),
                        None => {}
                    }
                }
                Job::Invade { block, tf, target, deadline } => {
                    let player = self.forces.get(&tf).map(|t| t.player);
                    if player.is_some() && sim.planet_owner(target) == player {
                        self.finish(block, true);
                    } else if now > deadline || !self.forces.contains_key(&tf) {
                        self.finish(block, false);
                    } else {
                        keep.push(Job::Invade { block, tf, target, deadline });
                    }
                }
            }
        }
        keep.append(&mut self.jobs);
        self.jobs = keep;
        // Forces whose fleet vanished.
        let ids: Vec<u64> = self.forces.keys().copied().collect();
        for id in ids {
            let (gone, plan, name) = {
                let t = &self.forces[&id];
                let gone = t.had_units && t.fleet.is_some_and(|f| sim.fleet(f).is_none()) && t.pieces.is_empty();
                (gone, t.plan, t.name.clone())
            };
            if gone {
                if let Some(t) = self.forces.get_mut(&id) {
                    if let Some(f) = t.fleet.take() {
                        self.owned_fleets.remove(&f);
                    }
                    t.had_units = false;
                    t.result = Some(false);
                }
                self.stats.forces_lost += 1;
                self.events.push(PlanEvent { plan, function: format!("{name}_No_Units_Remaining"), args: Vec::new() });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tick_move(&mut self, sim: &mut dyn AiSim, block: u64, tf: u64, to: PlanetId, started: f64, issued: f64, retries: u32, now: f64) -> Option<Job> {
        let Some(t) = self.forces.get(&tf) else {
            self.finish(block, false);
            return None;
        };
        let (player, fleet) = (t.player, t.fleet);
        let Some(fleet) = fleet else {
            self.finish(block, false);
            return None;
        };
        match sim.fleet(fleet) {
            None => {
                self.finish(block, false);
                None
            }
            Some(f) if f.at == Some(to) => {
                self.stats.arrivals += 1;
                self.finish(block, true);
                None
            }
            Some(f) => {
                // Stuck somewhere else (retreated, blockaded): re-issue a few times.
                if f.heading_to.is_none() && now - issued > 20.0 {
                    if retries >= 3 || sim.move_fleet(player, fleet, to).is_err() {
                        self.finish(block, false);
                        return None;
                    }
                    return Some(Job::Move { block, tf, to, started, issued: now, retries: retries + 1 });
                }
                if now - started > 900.0 {
                    self.finish(block, false);
                    return None;
                }
                Some(Job::Move { block, tf, to, started, issued, retries })
            }
        }
    }

    fn tick_units(&mut self, sim: &mut dyn AiSim, snap: &Snapshot, rng: &mut u64, mut j: UnitJob, now: f64) -> Option<UnitJob> {
        let Some(t) = self.forces.get(&j.tf) else {
            self.finish(j.block, false);
            return None;
        };
        let player = t.player;
        let plan = t.plan;
        let name = t.name.clone();
        let allow_free = t.reqs.allow_free_store;
        let fail = |me: &mut Self, j: &UnitJob, unit: &str| {
            me.stats.production_failures += 1;
            if let Some(t) = me.forces.get_mut(&j.tf) {
                t.result = Some(false);
            }
            me.events.push(PlanEvent { plan, function: format!("{name}_Production_Failed"), args: vec![HostValue::TaskForce(j.tf), HostValue::Type(unit.to_string())] });
            me.finish(j.block, false);
        };
        if now > j.deadline {
            let unit = j.wanted.first().cloned().unwrap_or_default();
            fail(self, &j, &unit);
            return None;
        }
        // 1. Completed builds: claim the new ship.
        let mut still = Vec::new();
        for p in std::mem::take(&mut j.pending) {
            let in_queue = sim.build_queue(p.planet).iter().any(|q| q.name.eq_ignore_ascii_case(&p.unit));
            if in_queue {
                still.push(p);
                continue;
            }
            match self.claim_unit(sim, player, j.tf, p.planet, &p.unit, j.stage) {
                true => {}
                false => {
                    // Lost (destroyed, taken by someone else): want it again, a bounded number of times.
                    let n = j.refused.entry(p.unit.to_ascii_lowercase()).or_default();
                    *n += 1;
                    if *n < 4 {
                        j.wanted.push(p.unit);
                    }
                }
            }
        }
        j.pending = still;
        // 2. Take wanted units from the free store, else queue them.
        let mut remaining: Vec<String> = Vec::new();
        let mut free = if allow_free { self.free_ships(sim, player) } else { HashMap::new() };
        let dist = hop_distances(snap, j.stage);
        for unit in std::mem::take(&mut j.wanted) {
            let key = unit.to_ascii_lowercase();
            let mut best: Option<(PlanetId, FleetId, u32)> = None;
            for ((pl, u), (fid, cnt)) in &free {
                if *u == key && *cnt > 0 && best.is_none_or(|b| dist[*pl as usize] < dist[b.0 as usize]) {
                    best = Some((*pl, *fid, *cnt));
                }
            }
            if let Some((pl, _, _)) = best {
                if self.claim_unit(sim, player, j.tf, pl, &unit, j.stage) {
                    self.stats.free_store_claims += 1;
                    if let Some(e) = free.get_mut(&(pl, key)) {
                        e.1 -= 1;
                    }
                    continue;
                }
            }
            // Queue it at the nearest planet that accepts it.
            let mut planets: Vec<PlanetId> = snap.owned[player as usize].clone();
            planets.sort_by_key(|&p| dist[p as usize]);
            let mut queued = false;
            let mut no_credits = false;
            for pl in planets.into_iter().take(6) {
                match sim.queue_unit(player, pl, &unit) {
                    Ok(cost) => {
                        self.stats.ships_queued += 1;
                        self.stats.credits_spent += cost;
                        if let Some(t) = self.forces.get_mut(&j.tf) {
                            t.worked = true;
                        }
                        j.pending.push(Pending { unit: unit.clone(), planet: pl, queued_at: now });
                        queued = true;
                        break;
                    }
                    Err(Refused::NoCredits) => {
                        no_credits = true;
                        break;
                    }
                    Err(Refused::Unavailable) => {}
                    Err(Refused::Invalid) => break,
                }
            }
            if !queued {
                if no_credits {
                    remaining.push(unit);
                } else {
                    let n = j.refused.entry(key).or_default();
                    *n += 1;
                    if *n <= 3 {
                        remaining.push(unit);
                    } else {
                        fail(self, &j, &unit);
                        return None;
                    }
                }
            }
        }
        j.wanted = remaining;
        let _ = rng;
        // 3. Gather pieces at the stage.
        self.gather(sim, player, j.tf, j.stage);
        // 4. Done when nothing is wanted/pending and all pieces are merged.
        let t = self.forces.get(&j.tf)?;
        if j.wanted.is_empty() && j.pending.is_empty() && t.pieces.is_empty() {
            if t.fleet.is_some() {
                self.finish(j.block, true);
            } else {
                self.finish(j.block, false);
            }
            return None;
        }
        Some(j)
    }

    /// Splits one `unit` off a fleet at `planet` into a piece of the force and sends it to the stage.
    fn claim_unit(&mut self, sim: &mut dyn AiSim, player: PlayerId, tf: u64, planet: PlanetId, unit: &str, stage: PlanetId) -> bool {
        let owned = &self.owned_fleets;
        let candidate = sim
            .fleets_at(planet)
            .into_iter()
            .filter(|f| f.owner == player && !owned.contains(&f.id))
            .find(|f| f.units.iter().any(|(u, c)| u.eq_ignore_ascii_case(unit) && *c > 0));
        let Some(src) = candidate else { return false };
        let exact = src.units.iter().find(|(u, _)| u.eq_ignore_ascii_case(unit)).map(|(u, _)| u.clone()).unwrap_or_else(|| unit.to_string());
        let whole = src.units.len() == 1 && src.units[0].1 == 1;
        let piece = if whole {
            src.id
        } else {
            match sim.split_fleet(player, src.id, &[(exact, 1)]) {
                Ok(f) => f,
                Err(_) => return false,
            }
        };
        let Some(t) = self.forces.get_mut(&tf) else { return false };
        self.owned_fleets.insert(piece);
        t.had_units = true;
        if t.fleet.is_none() {
            t.fleet = Some(piece);
        } else {
            t.pieces.push(piece);
        }
        if planet != stage {
            let _ = sim.move_fleet(player, piece, stage);
        }
        true
    }

    /// Merges pieces that reached the stage into the main fleet; sends idle strays on.
    fn gather(&mut self, sim: &mut dyn AiSim, player: PlayerId, tf: u64, stage: PlanetId) {
        let Some(t) = self.forces.get_mut(&tf) else { return };
        // The main fleet itself must sit at the stage before merging; if it is the first piece it travels.
        let main = match t.fleet {
            Some(f) => f,
            None => {
                if let Some(first) = t.pieces.pop() {
                    t.fleet = Some(first);
                    first
                } else {
                    return;
                }
            }
        };
        let main_at = sim.fleet(main).and_then(|f| f.at);
        let mut left = Vec::new();
        for p in std::mem::take(&mut t.pieces) {
            match sim.fleet(p) {
                None => {
                    self.owned_fleets.remove(&p);
                }
                Some(f) => {
                    if f.at.is_some() && f.at == main_at {
                        if sim.merge_fleets(player, main, p).is_ok() {
                            self.owned_fleets.remove(&p);
                            continue;
                        }
                    } else if f.at.is_some() && f.heading_to.is_none() && f.at != Some(stage) {
                        let _ = sim.move_fleet(player, p, stage);
                    }
                    left.push(p);
                }
            }
        }
        let Some(t) = self.forces.get_mut(&tf) else { return };
        t.pieces = left;
        if main_at.is_some() && main_at != Some(stage) && sim.fleet(main).is_some_and(|f| f.heading_to.is_none()) {
            let _ = sim.move_fleet(player, main, stage);
        }
        // Pieces count as gathered once everything sits at the stage (merge handles the rest above).
        let _ = (stage, &t.pieces);
    }

    fn tick_structures(&mut self, sim: &mut dyn AiSim, mut j: StructJob, now: f64) -> Option<StructJob> {
        let Some(t) = self.forces.get(&j.tf) else {
            self.finish(j.block, false);
            return None;
        };
        let (player, plan, name) = (t.player, t.plan, t.name.clone());
        let fail = |me: &mut Self, j: &StructJob, what: &str| {
            me.stats.production_failures += 1;
            if let Some(t) = me.forces.get_mut(&j.tf) {
                t.result = Some(false);
            }
            me.events.push(PlanEvent { plan, function: format!("{name}_Production_Failed"), args: vec![HostValue::TaskForce(j.tf), HostValue::Type(what.to_string())] });
            me.finish(j.block, false);
        };
        if now > j.deadline {
            fail(self, &j, "timeout");
            return None;
        }
        loop {
            let Some(group) = j.groups.front().cloned() else {
                // Optional groups that found nothing to build do not make a plan that built nothing a success.
                if j.built {
                    self.finish(j.block, true);
                } else {
                    fail(self, &j, "nothing buildable");
                }
                return None;
            };
            // Wait for an item queued for this group.
            if let Some(q) = &j.queued {
                let queued_now = sim.build_queue(j.planet).iter().any(|x| x.name.eq_ignore_ascii_case(q));
                if queued_now {
                    return Some(j);
                }
                j.queued = None;
                j.groups.pop_front();
                continue;
            }
            // Types of the group that already stand on the planet count as done.
            let built = sim.structures(j.planet);
            if group.types.iter().any(|ty| built.iter().any(|b| b.eq_ignore_ascii_case(ty))) && !group.types.iter().any(|t| sim.is_starbase_type(t)) {
                j.built = true;
                j.groups.pop_front();
                continue;
            }
            // Types that could be queued here (money aside).
            let mut candidates: Vec<(&String, bool)> = Vec::new();
            for ty in &group.types {
                let starbase = sim.is_starbase_type(ty);
                let ok = if starbase { sim.can_upgrade_starbase(player, j.planet) } else { sim.can_build_structure(player, j.planet, ty) };
                if matches!(ok, Ok(()) | Err(Refused::NoCredits)) {
                    candidates.push((ty, starbase));
                }
            }
            let Some(&(ty, starbase)) = candidates.first() else {
                if group.min == 0 {
                    j.groups.pop_front();
                    continue;
                }
                fail(self, &j, group.types.first().map_or("", String::as_str));
                return None;
            };
            let r = if starbase { sim.queue_starbase(player, j.planet) } else { sim.queue_structure(player, j.planet, ty) };
            match r {
                Ok(cost) => {
                    self.stats.structures_queued += 1;
                    self.stats.credits_spent += cost;
                    if let Some(t) = self.forces.get_mut(&j.tf) {
                        t.worked = true;
                    }
                    j.built = true;
                    j.queued = Some(ty.clone());
                    return Some(j);
                }
                Err(Refused::NoCredits) => return Some(j),
                Err(_) => {
                    if group.min == 0 {
                        j.groups.pop_front();
                        continue;
                    }
                    fail(self, &j, ty);
                    return None;
                }
            }
        }
    }
}
fn job_tf(j: &Job) -> u64 {
    match j {
        Job::Units(u) => u.tf,
        Job::Structures(s) => s.tf,
        Job::Move { tf, .. } | Job::Invade { tf, .. } => *tf,
    }
}
