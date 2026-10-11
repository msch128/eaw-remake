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

/// Margin of predicted square-law strength a task force asks over what it believes defends the target (**estimate**). Tuned over 16 seeds of the all-AI match
/// (`galaxy-script/tests/ai_match.rs`): 1.25 took 99 enemy-owner captures against 150 for the plain AI, 0.8 takes 141 (94%) at the same fleet size.
pub const FORCE_MARGIN: f64 = 0.8;

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
}

#[derive(Debug)]
enum Job {
    Units(UnitJob),
    Structures(StructJob),
    Move { block: u64, tf: u64, to: PlanetId, started: f64, issued: f64, retries: u32 },
    Invade { block: u64, tf: u64, target: PlanetId, deadline: f64 },
    /// Waits at the stage until the partner forces of a pincer attack are ready, then leaves at a time that makes all arrive together.
    Rally { block: u64, tf: u64, to: PlanetId, started: f64 },
}

#[derive(Debug)]
struct UnitJob {
    block: u64,
    tf: u64,
    stage: PlanetId,
    wanted: Vec<String>,
    pending: Vec<Pending>,
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
    /// Why ship produce jobs failed (`unit: reason`).
    pub failure_reasons: HashMap<String, u32>,
    /// Moves ordered.
    pub moves: u32,
    /// Pincer attacks whose forces left together.
    pub pincers: u32,
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
    /// Money earmarked per `(player, goal category)`; see [`TaskForces::fund`].
    accounts: HashMap<(PlayerId, String), f64>,
    /// Why the production of a plan failed, for the statistics (taken when the plan ends).
    failure_detail: HashMap<u64, String>,
    /// Diagnostics per task force: what the last `Produce_Force` asked for and how it ended.
    produce_note: HashMap<u64, String>,
    plan_category: HashMap<u64, String>,
    /// Believed hostile strength per player and planet (set by the engine from the strategic layer; empty = use AI power).
    pub(crate) believed: HashMap<PlayerId, Vec<f64>>,
    sizing: HashMap<PlayerId, f64>,
    /// Number of coordinated forces attacking the same target (pincer); each is sized for its share.
    split: HashMap<(PlayerId, PlanetId), u32>,
    rally: HashMap<(PlayerId, PlanetId), Vec<u64>>,
    depart: HashMap<u64, f64>,
    /// Fleets released after assembling a force for a target stay reserved for that target for a while.
    reserved: HashMap<FleetId, (PlanetId, f64)>,
    clock: f64,
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

/// Planets from which `stage` can be reached without passing a system that would stop the player's ships (an enemy star base):
/// ships sent from anywhere else would be blockaded and destroyed one by one on the way.
fn safe_from(snap: &Snapshot, player: PlayerId, stage: PlanetId) -> Vec<bool> {
    let blocks = |p: PlanetId| snap.owner[p as usize].is_some_and(|o| snap.enemy[player as usize][o as usize]) && snap.starbase[p as usize].0 > 0;
    let mut seen = vec![false; snap.adjacency.len()];
    let mut q = VecDeque::new();
    seen[stage as usize] = true;
    q.push_back(stage);
    while let Some(p) = q.pop_front() {
        if p != stage && blocks(p) {
            continue; // reached, but nothing travels on through it
        }
        for &n in &snap.adjacency[p as usize] {
            if !seen[n as usize] {
                seen[n as usize] = true;
                q.push_back(n);
            }
        }
    }
    // A blockading system itself is no starting point for the player's ships.
    for (p, s) in seen.iter_mut().enumerate() {
        if p as PlanetId != stage && blocks(p as PlanetId) {
            *s = false;
        }
    }
    seen
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
    /// Factor on the force margin of a player (difficulty contrast and personality).
    pub fn set_sizing(&mut self, player: PlayerId, factor: f64) {
        self.sizing.insert(player, factor);
    }

    /// Sets what a player believes hostile forces hold at every planet (square-law strength); enables predicted-outcome sizing.
    pub fn set_believed(&mut self, player: PlayerId, per_planet: Vec<f64>) {
        self.believed.insert(player, per_planet);
    }

    /// The sizing factor of a player (1.0 if none).
    pub fn sizing_of(&self, player: PlayerId) -> f64 {
        self.sizing.get(&player).copied().unwrap_or(1.0)
    }

    /// Declares that `parts` forces attack `target` together; each is sized for its share.
    pub fn set_split(&mut self, player: PlayerId, target: PlanetId, parts: u32) {
        self.split.insert((player, target), parts);
    }

    /// Remembers the goal category a plan belongs to; its spending comes from that category's account.
    pub fn set_plan_category(&mut self, plan: u64, category: &str) {
        self.plan_category.insert(plan, category.to_ascii_lowercase());
    }

    /// Earmarks `amount` credits for the categories of `player` in proportion to `fractions` (the budget equations of the
    /// player's template). Plans may only spend what their category holds; money nobody can use stays in the treasury, which
    /// is what makes a player "wealthy" (an estimate inferred from how the budget and wealth equations fit together; the
    /// engine's exact bookkeeping is not visible in the data).
    pub fn fund(&mut self, player: PlayerId, fractions: &HashMap<String, f64>, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        for (cat, f) in fractions {
            if *f > 0.0 {
                *self.accounts.entry((player, cat.clone())).or_default() += amount * f;
            }
        }
    }

    /// Money of `player` that is earmarked for plans and not spent yet.
    pub fn earmarked(&self, player: PlayerId) -> f64 {
        self.accounts.iter().filter(|((p, _), _)| *p == player).map(|(_, v)| *v).sum()
    }

    /// What a plan may still spend; `None` = unlimited (category without a budget).
    fn allowance(&self, player: PlayerId, plan: u64) -> Option<f64> {
        let cat = self.plan_category.get(&plan)?;
        self.accounts.get(&(player, cat.clone())).copied()
    }

    fn charge(&mut self, player: PlayerId, plan: u64, cost: f64) {
        if let Some(cat) = self.plan_category.get(&plan)
            && let Some(a) = self.accounts.get_mut(&(player, cat.clone()))
        {
            *a = (*a - cost).max(0.0);
        }
    }

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
    /// The recorded cause of a plan's production failure, if any (removes it).
    pub fn take_failure_detail(&mut self, plan: u64) -> Option<String> {
        self.failure_detail.remove(&plan)
    }

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
        self.plan_category.remove(&plan);
        self.jobs.retain(|j| !ids.contains(&job_tf(j)));
    }

    fn release(&mut self, tf: u64) {
        let clock = self.clock;
        if let Some(t) = self.forces.get_mut(&tf) {
            if let Some(f) = t.fleet.take() {
                self.owned_fleets.remove(&f);
                if let (Some(target), ForceKind::Units, true) = (t.target, t.kind, t.worked) {
                    // Estimate: five minutes of game time to be picked up by the next plan for the same target.
                    self.reserved.insert(f, (target, clock + 300.0));
                }
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
                    if t.result == Some(false) {
                        let plan = t.plan;
                        self.failure_detail.entry(plan).or_insert_with(|| "plan script reported failure (force lost or target not held)".to_string());
                    }
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
                let safe = self.stage_safety(snap, t);
                let covered = self.free_store_covers(sim, t.player, &wanted, t.target, safe.as_deref());
                self.produce_note.insert(tf_id, format!("check: wanted {wanted:?} covers {covered}"));
                HostValue::Bool(covered)
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
                    (ForceKind::Units, Some(to)) if self.forces.get(&tf_id).is_some_and(|t| self.split.get(&(t.player, to)).is_some_and(|n| *n > 1)) => {
                        let player = self.forces[&tf_id].player;
                        self.rally.entry((player, to)).or_default().push(tf_id);
                        self.jobs.push(Job::Rally { block, tf: tf_id, to, started: now });
                    }
                    (ForceKind::Units, Some(to)) => {
                        let moved = self.start_move(sim, tf_id, to);
                        if moved {
                            self.jobs.push(Job::Move { block, tf: tf_id, to, started: now, issued: now, retries: 0 });
                        } else {
                            if let Some(t) = self.forces.get(&tf_id) {
                                self.failure_detail.entry(t.plan).or_insert_with(|| "move order refused by the sim".to_string());
                            }
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

    /// Planets from which ships can reach the stage of a unit force without passing a blockade (`None`: no such restriction).
    fn stage_safety(&self, snap: &Snapshot, t: &TaskForce) -> Option<Vec<bool>> {
        if t.kind == ForceKind::Structure {
            return None;
        }
        t.stage.or_else(|| self.choose_stage(snap, t)).map(|stage| safe_from(snap, t.player, stage))
    }

    /// The list of unit types a force should consist of.
    fn compose(&self, sim: &dyn AiSim, snap: &Snapshot, t: &TaskForce, _rng: &mut u64) -> Vec<String> {
        // The composition is a function of (player, target, force) so that repeated asks (ready check, production, a later plan for
        // the same target) agree and ships built by an earlier plan are recognised on the free store.
        let mut seeded: u64 = 0x9E37_79B9_7F4A_7C15 ^ (u64::from(t.player) << 40) ^ (u64::from(t.target.unwrap_or(0)) << 8) ^ t.name.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(u64::from(b)));
        let rng = &mut seeded;
        let mut candidates: Vec<(String, String, f64)> = Vec::new(); // name, category, power
        // Ships built at a planet that cannot reach the stage safely would be lost on the way: only the planets that can count.
        let safe_planets: Vec<PlanetId> = match t.stage.or_else(|| self.choose_stage(snap, t)) {
            Some(stage) if t.kind != ForceKind::Structure => {
                let safe = safe_from(snap, t.player, stage);
                snap.owned[t.player as usize].iter().copied().filter(|&p| safe[p as usize]).take(40).collect()
            }
            _ => snap.owned[t.player as usize].iter().copied().take(40).collect(),
        };
        for name in sim.buildable_units(t.player) {
            if t.reqs.excluded.contains(&name.to_ascii_lowercase()) {
                continue;
            }
            let Some(info) = sim.unit_info(&name) else { continue };
            if info.is_hero && t.reqs.deny_hero {
                continue;
            }
            // Only ships some owned planet can build right now (tech, shipyard, slots); money may still be missing.
            let buildable = safe_planets
                .iter()
                .any(|&pl| matches!(sim.can_build_unit(t.player, pl, &name), Ok(()) | Err(Refused::NoCredits)));
            if !buildable {
                continue;
            }
            candidates.push((name, info.category.to_ascii_uppercase(), info.power.max(1.0)));
        }
        // Ships already on the free store are part of the candidate list (whether or not they could be built now) and are used first,
        // so a force assembled by an earlier plan is recognised as ready.
        let mut avail: HashMap<String, u32> = HashMap::new();
        if t.reqs.allow_free_store {
            for ((_, u), (_, c)) in self.free_ships(sim, t.player, t.target, self.stage_safety(snap, t).as_deref()) {
                *avail.entry(u).or_default() += c;
            }
            let mut names: Vec<String> = avail.keys().cloned().collect();
            names.sort();
            for u in names {
                if candidates.iter().any(|c| c.0.eq_ignore_ascii_case(&u)) || t.reqs.excluded.contains(&u) {
                    continue;
                }
                if let Some(info) = sim.unit_info(&u).filter(|i| !(i.is_hero && t.reqs.deny_hero)) {
                    candidates.push((info.name.clone(), info.category.to_ascii_uppercase(), info.power.max(1.0)));
                }
            }
        }
        let matches = |c: &(String, String, f64), types: &[String]| types.iter().any(|ty| ty.eq_ignore_ascii_case(&c.0) || (is_category(ty) && ty.eq_ignore_ascii_case(&c.1)));
        let role_of = |category: &str, name: &str| -> usize {
            let cat = category.to_ascii_uppercase();
            let n = name.to_ascii_lowercase();
            if cat.contains("CAPITAL") || cat.contains("SUPER") || cat.contains("BATTLESHIP") || cat.contains("CRUISER") {
                0 // 40% Capitals / Line
            } else if cat.contains("FRIGATE") || cat.contains("CORVETTE") {
                1 // 30% Frigates / Escorts
            } else if cat.contains("CARRIER") || n.contains("carrier") {
                2 // 30% Carriers / Strike / Fighters
            } else {
                2
            }
        };
        let mut role_counts = [0usize; 3];
        let role_targets = [0.40f64, 0.30f64, 0.30f64];
        let role_weight = |c: &(String, String, f64), counts: &[usize; 3]| -> f64 {
            let r = role_of(&c.1, &c.0);
            let total = (counts[0] + counts[1] + counts[2]) as f64;
            if total >= 2.0 {
                let share = (counts[r] as f64) / total;
                let mult = (role_targets[r] / share.max(0.05)).clamp(0.25, 4.0);
                c.2.sqrt() * mult
            } else {
                c.2.sqrt() * role_targets[r] * 3.0
            }
        };
        let pick = |rng: &mut u64, pool: &[&(String, String, f64)], counts: &[usize; 3]| -> Option<String> {
            let total: f64 = pool.iter().map(|c| role_weight(c, counts)).sum();
            if pool.is_empty() || total <= 0.0 {
                return None;
            }
            let mut r = next_rand(rng) * total;
            for c in pool {
                r -= role_weight(c, counts);
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
                if let Some(n) = pick(rng, &pool, &role_counts) {
                    if let Some(c) = candidates.iter().find(|c| c.0 == n) {
                        role_counts[role_of(&c.1, &c.0)] += 1;
                    }
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
            // Predicted-outcome sizing: what the player believes defends the target (square-law strength) times the margin it
            // asks for (`FORCE_MARGIN` estimate x plan contrast scale x difficulty/personality sizing), shared between pincer forces.
            let believed = self.believed.get(&t.player).and_then(|v| t.target.map(|tp| v[tp as usize]));
            let strength_mode = believed.is_some() && candidates.first().is_some_and(|c| sim.force_strength(&[(c.0.clone(), 1)]) > 0.0);
            let split = t.target.and_then(|tp| self.split.get(&(t.player, tp))).copied().unwrap_or(1).max(1);
            let needed_strength = believed.unwrap_or(0.0) * FORCE_MARGIN * scale * self.sizing.get(&t.player).copied().unwrap_or(1.0) / f64::from(split);
            let strength_of = |w: &[String]| {
                let mut counts: Vec<(String, u32)> = Vec::new();
                for n in w {
                    match counts.iter_mut().find(|c| c.0 == *n) {
                        Some(c) => c.1 += 1,
                        None => counts.push((n.clone(), 1)),
                    }
                }
                sim.force_strength(&counts)
            };
            while (wanted.len() < min_total || if strength_mode { strength_of(&wanted) < needed_strength } else { power < needed }) && wanted.len() < cap && guard < 200 {
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
                let free_pool: Vec<_> = candidates.iter().filter(|c| matches(c, &chosen.types) && avail.get(&c.0.to_ascii_lowercase()).copied().unwrap_or(0) > 0).collect();
                let pool: Vec<_> = if free_pool.is_empty() { candidates.iter().filter(|c| matches(c, &chosen.types)).collect() } else { free_pool };
                if let Some(n) = pick(rng, &pool, &role_counts) {
                    if let Some(c) = candidates.iter().find(|c| c.0 == n) {
                        role_counts[role_of(&c.1, &c.0)] += 1;
                    }
                    power += candidates.iter().find(|c| c.0 == n).map_or(0.0, |c| c.2);
                    if let Some(a) = avail.get_mut(&n.to_ascii_lowercase()) {
                        *a = a.saturating_sub(1);
                    }
                    wanted.push(n);
                } else {
                    break;
                }
            }
        }
        wanted
    }

    fn free_ships(&self, sim: &dyn AiSim, player: PlayerId, target: Option<PlanetId>, safe: Option<&[bool]>) -> HashMap<(PlanetId, String), (FleetId, u32)> {
        let mut out: HashMap<(PlanetId, String), (FleetId, u32)> = HashMap::new();
        for f in sim.fleets_of(player) {
            let Some(at) = f.at else { continue };
            if safe.is_some_and(|s| !s.get(at as usize).copied().unwrap_or(false)) {
                continue;
            }
            if self.owned_fleets.contains(&f.id) {
                continue;
            }
            if let Some(&(for_target, until)) = self.reserved.get(&f.id)
                && until > self.clock
                && Some(for_target) != target
            {
                continue;
            }
            for (u, c) in &f.units {
                let e = out.entry((at, u.to_ascii_lowercase())).or_insert((f.id, 0));
                e.1 += c;
            }
        }
        out
    }

    fn free_store_covers(&self, sim: &dyn AiSim, player: PlayerId, wanted: &[String], target: Option<PlanetId>, safe: Option<&[bool]>) -> bool {
        let free = self.free_ships(sim, player, target, safe);
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
            ForceKind::Ground => self.finish(block, true),
            ForceKind::Structure => {
                let planet = t.target.or(stage_arg).or_else(|| self.choose_stage(snap, t));
                let Some(planet) = planet else { return self.finish(block, false) };
                let groups: VecDeque<_> = t.reqs.groups.iter().filter(|g| g.max > 0).cloned().collect();
                if groups.is_empty() {
                    return self.finish(block, true);
                }
                self.jobs.push(Job::Structures(StructJob { block, tf: tf_id, planet, groups, queued: None, built: false, deadline: now + 900.0 }));
            }
            ForceKind::Units => {
                let stage = stage_arg.or(t.stage).or_else(|| self.choose_stage(snap, t));
                let Some(stage) = stage else {
                    self.failure_detail.insert(t.plan, "no staging planet".to_string());
                    return self.finish(block, false);
                };
                let wanted = self.compose(sim, snap, t, rng);
                let safe = self.stage_safety(snap, t);
                let free_cover = self.free_store_covers(sim, t.player, &wanted, t.target, safe.as_deref());
                let free_now: Vec<String> = self.free_ships(sim, t.player, t.target, safe.as_deref()).iter().map(|((p, u), (_, c))| format!("{u}x{c}@{p}")).collect();
                let note = format!(" | produce: stage {stage}, wanted {wanted:?}, covers {free_cover}, free {free_now:?}");
                self.produce_note.entry(tf_id).or_default().push_str(&note);
                if let Some(t) = self.forces.get_mut(&tf_id) {
                    t.stage = Some(stage);
                }
                if wanted.is_empty() {
                    if let Some(t) = self.forces.get(&tf_id) {
                        self.failure_detail.insert(t.plan, "no ship to build (empty composition)".to_string());
                    }
                    return self.finish(block, false);
                }
                self.jobs.push(Job::Units(UnitJob {
                    block,
                    tf: tf_id,
                    stage,
                    wanted,
                    pending: Vec::new(),
                    deadline: now + 1200.0,
                    refused: HashMap::new(),
                }));
            }
        }
    }

    fn start_move(&mut self, sim: &mut dyn AiSim, tf_id: u64, to: PlanetId) -> bool {
        let Some(t) = self.forces.get(&tf_id) else { return false };
        let (plan, player, fleet) = (t.plan, t.player, t.fleet);
        let Some(fleet) = fleet else {
            let running = self.jobs.iter().find_map(|j| match j { Job::Units(u) if u.tf == tf_id => Some(format!("production still running (plan stopped waiting): wanted {} pending {}", u.wanted.len(), u.pending.len())), _ => None });
            let note = self.produce_note.get(&tf_id).cloned().unwrap_or_else(|| "Produce_Force never ran".to_string());
            if std::env::var("AI_TRACE_FORCE").is_ok() {
                eprintln!("no fleet at launch: plan {plan} tf {tf_id}: {} | {note}", running.as_deref().unwrap_or("production ended without a fleet"));
            }
            let cause = running.map_or_else(|| if note.contains("produce: ") && note.contains(", covers true,") { "free store promised the force but production ended without a fleet" } else { "production ended without a fleet" }, |_| "production still running when the plan gave up waiting");
            self.failure_detail.insert(plan, format!("force has no fleet at launch ({cause})"));
            return false;
        };
        if sim.fleet(fleet).is_some_and(|f| f.at == Some(to)) {
            return true;
        }
        self.stats.moves += 1;
        sim.move_fleet(player, fleet, to).is_ok()
    }

    /// Advances all pending commands.
    pub fn tick(&mut self, sim: &mut dyn AiSim, snap: &Snapshot, rng: &mut u64) {
        let now = sim.time();
        self.clock = now;
        self.reserved.retain(|_, (_, until)| *until > now);
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
                    if let Some(j) = self.tick_move(sim, block, tf, to, started, issued, retries, now) {
                        keep.push(j);
                    }
                }
                Job::Rally { block, tf, to, started } => {
                    if let Some(j) = self.tick_rally(sim, block, tf, to, started, now) {
                        keep.push(j);
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
        let mut ids: Vec<u64> = self.forces.keys().copied().collect();
        ids.sort_unstable();
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

    /// Pincer rendezvous: once every partner is staged (or after 90 s) each force departs so that all arrive together.
    fn tick_rally(&mut self, sim: &mut dyn AiSim, block: u64, tf: u64, to: PlanetId, started: f64, now: f64) -> Option<Job> {
        let Some(t) = self.forces.get(&tf) else {
            self.finish(block, false);
            return None;
        };
        let player = t.player;
        if !self.depart.contains_key(&tf) {
            let parts = self.split.get(&(player, to)).copied().unwrap_or(1) as usize;
            let members: Vec<u64> = self.rally.get(&(player, to)).cloned().unwrap_or_default().into_iter().filter(|m| self.forces.contains_key(m)).collect();
            if members.len() < parts && now - started < 90.0 {
                return Some(Job::Rally { block, tf, to, started });
            }
            let etas: Vec<(u64, f64)> = members.iter().map(|m| (*m, self.forces[m].fleet.and_then(|f| sim.eta(f, to)).unwrap_or(0.0))).collect();
            let longest = etas.iter().map(|e| e.1).fold(0.0, f64::max);
            for (m, e) in etas {
                self.depart.insert(m, now + (longest - e).max(0.0));
            }
            self.stats.pincers += 1;
        }
        if now < self.depart.get(&tf).copied().unwrap_or(now) {
            return Some(Job::Rally { block, tf, to, started });
        }
        self.depart.remove(&tf);
        if let Some(v) = self.rally.get_mut(&(player, to)) {
            v.retain(|m| *m != tf);
        }
        if self.start_move(sim, tf, to) {
            return Some(Job::Move { block, tf, to, started: now, issued: now, retries: 0 });
        }
        self.finish(block, false);
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn tick_move(&mut self, sim: &mut dyn AiSim, block: u64, tf: u64, to: PlanetId, started: f64, issued: f64, retries: u32, now: f64) -> Option<Job> {
        let Some(t) = self.forces.get(&tf) else {
            self.finish(block, false);
            return None;
        };
        let (player, fleet) = (t.player, t.fleet);
        if !t.pieces.is_empty() && Some(to) == t.target {
            if let Some(stage) = t.stage {
                self.gather(sim, player, tf, stage);
                let t_ref = self.forces.get(&tf);
                if t_ref.is_some_and(|tf_obj| !tf_obj.pieces.is_empty()) && now - started < 60.0 {
                    return Some(Job::Move { block, tf, to, started, issued, retries });
                }
            }
        }
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
        let fail = |me: &mut Self, j: &UnitJob, unit: &str, why: &str, detail: String| {
            if std::env::var("AI_TRACE_FORCE").is_ok() {
                eprintln!("production failed: plan {plan} tf {} stage {} {why}: {detail}; wanted {:?} pending {}", j.tf, j.stage, j.wanted, j.pending.len());
            }
            me.produce_note.entry(j.tf).or_default().push_str(&format!(" | failed: {why}"));
            me.failure_detail.insert(plan, detail);
            me.stats.production_failures += 1;
            *me.stats.failure_reasons.entry(format!("{unit}: {why}")).or_default() += 1;
            if let Some(t) = me.forces.get_mut(&j.tf) {
                t.result = Some(false);
            }
            me.events.push(PlanEvent { plan, function: format!("{name}_Production_Failed"), args: vec![HostValue::TaskForce(j.tf), HostValue::Type(unit.to_string())] });
            me.finish(j.block, false);
        };
        if now > j.deadline {
            let unit = j.wanted.first().cloned().unwrap_or_default();
            fail(self, &j, &unit, "deadline", "deadline".to_string());
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
        let dist = hop_distances(snap, j.stage);
        let safe = safe_from(snap, player, j.stage);
        let mut remaining: Vec<String> = Vec::new();
        let mut free = if allow_free { self.free_ships(sim, player, self.forces.get(&j.tf).and_then(|t| t.target), Some(&safe)) } else { HashMap::new() };
        for unit in std::mem::take(&mut j.wanted) {
            let key = unit.to_ascii_lowercase();
            let mut best: Option<(PlanetId, FleetId, u32)> = None;
            let mut entries: Vec<_> = free.iter().collect();
            entries.sort_by_key(|((pl, _), _)| *pl);
            for ((pl, u), (fid, cnt)) in entries {
                if *u == key && *cnt > 0 && safe[*pl as usize] && best.is_none_or(|b| dist[*pl as usize] < dist[b.0 as usize]) {
                    best = Some((*pl, *fid, *cnt));
                }
            }
            if let Some((pl, _, _)) = best
                && self.claim_unit(sim, player, j.tf, pl, &unit, j.stage)
            {
                self.stats.free_store_claims += 1;
                if let Some(e) = free.get_mut(&(pl, key)) {
                    e.1 -= 1;
                }
                continue;
            }
            // Queue it at the nearest planet that accepts it.
            let mut planets: Vec<PlanetId> = snap.owned[player as usize].clone();
            planets.retain(|&p| safe[p as usize]);
            planets.sort_by_key(|&p| dist[p as usize]);
            let mut queued = false;
            let mut no_credits = false;
            let unit_cost = sim.unit_info(&unit).map_or(0.0, |u| u.cost);
            let affordable = self.allowance(player, plan).is_none_or(|a| a >= unit_cost);
            if !affordable {
                no_credits = true;
            }
            for pl in planets.into_iter().take(40) {
                if !affordable {
                    break;
                }
                match sim.queue_unit(player, pl, &unit) {
                    Ok(cost) => {
                        self.charge(player, plan, cost);
                        self.stats.ships_queued += 1;
                        self.stats.credits_spent += cost;
                        if let Some(t) = self.forces.get_mut(&j.tf) {
                            t.worked = true;
                        }
                        j.pending.push(Pending { unit: unit.clone(), planet: pl });
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
                        // Why every planet said no: the most common explanation over the player's planets.
                        let mut why: HashMap<String, u32> = HashMap::new();
                        for &pl in snap.owned[player as usize].iter().take(40) {
                            *why.entry(sim.explain_build_unit(player, pl, &unit)).or_default() += 1;
                        }
                        let mut why: Vec<_> = why.into_iter().collect();
                        why.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                        let detail = format!("refused: {} ({})", why.iter().take(3).map(|w| format!("{} x{}", w.0, w.1)).collect::<Vec<_>>().join(", "), unit);
                        fail(self, &j, &unit, "refused by every planet", detail);
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
            let cost_hint = if starbase {
                sim.starbase_upgrade_cost(player, j.planet).unwrap_or(0.0)
            } else {
                sim.structure_info(ty).map_or(0.0, |s| s.cost)
            };
            if self.allowance(player, plan).is_some_and(|a| a < cost_hint.max(1.0)) {
                return Some(j);
            }
            let r = if starbase { sim.queue_starbase(player, j.planet) } else { sim.queue_structure(player, j.planet, ty) };
            match r {
                Ok(cost) => {
                    self.charge(player, plan, cost);
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
        Job::Move { tf, .. } | Job::Invade { tf, .. } | Job::Rally { tf, .. } => *tf,
    }
}

/// The long-lived part of the task force bookkeeping: money earmarked per goal category, force sizing and fleets reserved for a target.
/// Forces, jobs and their command blocks belong to plan scripts, which are not saved (a loaded game starts with none).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskForceMemory {
    /// Money per `(player, category)`.
    pub accounts: Vec<(PlayerId, String, f64)>,
    /// Sizing factor per player.
    pub sizing: Vec<(PlayerId, f64)>,
    /// `(fleet, target planet, reserved until)`.
    pub reserved: Vec<(FleetId, PlanetId, f64)>,
    /// Task force clock.
    pub clock: f64,
}

impl TaskForces {
    /// Copies the long-lived bookkeeping.
    pub fn snapshot(&self) -> TaskForceMemory {
        let mut m = TaskForceMemory {
            accounts: self.accounts.iter().map(|((p, c), v)| (*p, c.clone(), *v)).collect(),
            sizing: self.sizing.iter().map(|(p, v)| (*p, *v)).collect(),
            reserved: self.reserved.iter().map(|(f, (p, t))| (*f, *p, *t)).collect(),
            clock: self.clock,
        };
        m.accounts.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        m.sizing.sort_by_key(|x| x.0);
        m.reserved.sort_by_key(|x| x.0);
        m
    }

    /// Puts the bookkeeping back into a task force set without forces.
    pub fn restore(&mut self, m: &TaskForceMemory) {
        self.accounts = m.accounts.iter().map(|(p, c, v)| ((*p, c.clone()), *v)).collect();
        self.sizing = m.sizing.iter().copied().collect();
        self.reserved = m.reserved.iter().map(|&(f, p, t)| (f, (p, t))).collect();
        self.clock = m.clock;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn role_doctrine_balances_capitals_frigates_and_carriers() {
        let role_of = |category: &str, name: &str| -> usize {
            let cat = category.to_ascii_uppercase();
            let n = name.to_ascii_lowercase();
            if cat.contains("CAPITAL") || cat.contains("SUPER") || cat.contains("BATTLESHIP") || cat.contains("CRUISER") {
                0
            } else if cat.contains("FRIGATE") || cat.contains("CORVETTE") {
                1
            } else if cat.contains("CARRIER") || n.contains("carrier") {
                2
            } else {
                2
            }
        };
        assert_eq!(role_of("CAPITAL", "ISD"), 0);
        assert_eq!(role_of("FRIGATE", "Nebulon"), 1);
        assert_eq!(role_of("CORVETTE", "CR90"), 1);
        assert_eq!(role_of("FIGHTER", "X-Wing"), 2);
        assert_eq!(role_of("OTHER", "Quasar_Carrier"), 2);
    }
}

