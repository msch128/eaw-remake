//! The AI engine: goal evaluation, budgets and plan selection for every AI player.
//!
//! Every [`AiEngine::update`] (a fixed interval of game time per player):
//!
//! 1. a [`Snapshot`] of the galaxy is taken;
//! 2. the player's template budget equations give each goal category a share of the money;
//! 3. every galactic goal of the player's function sets is evaluated: its candidate targets (planets
//!    chosen by `AIGoalApplicationFlags` and `Reachability`) are scored by the goal's desire equation;
//! 4. the best (goal, target) pairs that fit the category caps and have a plan script with
//!    `Category = <goal>` become [`PlanRequest`]s, which the host starts as Lua plan instances.
//!
//! Plans run through [`alamo_lua`] and call back into the [`TaskForces`] engine.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use alamo_lua::{HostValue, PlanEvent, TaskForceSpec};

use crate::data::{AiData, AiPlayerDef, AiTemplate, GoalDef};
use crate::equation::{EvalCache, EvalStats, Params};
use crate::perception::{PerceptionCtx, PlayerBrainView, Snapshot};
use crate::sim::{AiSim, PlanetId, PlayerId};
use crate::strategy::Strategy;

/// How long a plan may run relative to its goal's `Time_Limit x Build_Time_Delay_Tolerance` before the engine kills it (the data only gives the limit; the factor is an estimate).
const PLAN_LIMIT_FACTOR: f64 = 4.0;
use crate::taskforce::{TaskForceStats, TaskForces};

/// A plan the host should start.
#[derive(Debug, Clone)]
pub struct PlanRequest {
    /// Plan instance id (use it with `spawn_plan` / `kill_plan`).
    pub plan: u64,
    /// Script name (`Conquer_Enemy_Planet`).
    pub script: String,
    /// Player running the plan.
    pub player: PlayerId,
    /// Target planet, if the goal has one.
    pub target: Option<PlanetId>,
    /// Goal name.
    pub goal: String,
}

#[derive(Debug, Clone)]
struct ActiveGoal {
    goal: String,
    category: String,
    target: Option<PlanetId>,
    plan: u64,
    started: f64,
    limit: f64,
}

#[derive(Debug)]
struct GoalEntry {
    def: GoalDef,
    function: String,
}

#[derive(Debug, Clone)]
struct Outcome {
    goal: String,
    target: Option<PlanetId>,
    time: f64,
    /// The goal is not started again for this target before this many seconds passed.
    cooldown: f64,
    failed: bool,
}

struct PlayerBrain {
    player: PlayerId,
    def: AiPlayerDef,
    template: Option<AiTemplate>,
    goals: Vec<GoalEntry>,
    active: Vec<ActiveGoal>,
    /// Finished plans: used for cooldowns and failure learning.
    failures: Vec<Outcome>,
    fractions: HashMap<String, f64>,
    next_eval: f64,
    /// Earliest time each goal is evaluated again.
    goal_next: Vec<f64>,
    bonuses: Vec<(String, PlanetId, f64, f64)>,
    /// Game time money was last earmarked for the budget categories.
    last_fund: Option<f64>,
    /// Next time the AI considers a tech upgrade.
    next_tech: f64,
}

/// Report counters.
#[derive(Debug, Default, Clone)]
pub struct AiStats {
    /// Plans started per `(player, goal)`.
    pub plans_started: BTreeMap<(PlayerId, String), u32>,
    /// Plans that ended with success per `(player, goal)`.
    pub plans_succeeded: BTreeMap<(PlayerId, String), u32>,
    /// Plans that ended with failure per `(player, goal)`.
    pub plans_failed: BTreeMap<(PlayerId, String), u32>,
    /// Goals that won on desire but have no plan script.
    pub goals_without_plan: BTreeMap<String, u32>,
    /// Equation evaluations.
    pub evaluations: u64,
    /// Nanoseconds spent evaluating goals.
    pub eval_nanos: u128,
    /// Nanoseconds spent advancing task force jobs (production, moves).
    pub tick_nanos: u128,
    /// Nanoseconds spent refreshing the strategic layer.
    pub strategy_nanos: u128,
    /// Tech upgrades the AI queued.
    pub tech_upgrades: u32,
    /// Nanoseconds spent building perception snapshots.
    pub snapshot_nanos: u128,
    /// Unknown variables / missing equations aggregated over all evaluations.
    pub eval: EvalStats,
    /// Task force statistics.
    pub taskforces: TaskForceStats,
}

/// The AI for all AI-controlled factions of a match.
pub struct AiEngine {
    data: Arc<AiData>,
    plans: HashMap<String, Vec<String>>,
    brains: Vec<PlayerBrain>,
    snap: Option<Snapshot>,
    rng: u64,
    next_plan: u64,
    /// Seconds between goal evaluations per player.
    pub interval: f64,
    /// Task forces and their commands.
    pub task_forces: TaskForces,
    /// Strategic layer (influence maps, utility scoring); see `strategy.rs`.
    pub strategy: Strategy,
    strategy_at: f64,
    stats: AiStats,
    expired: Vec<u64>,
}

/// Plan scripts by goal: scans `Scripts/AI/*.lua` (not the land / space mode subdirectories) for
/// `Category = "<goal>"`.
pub fn discover_plans(src: &dyn alamo_fs::FileSource) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for path in src.list("Scripts/AI") {
        let Some(rest) = path.strip_prefix("SCRIPTS/AI/") else { continue };
        if rest.contains('/') || !rest.ends_with(".LUA") {
            continue;
        }
        let Ok(bytes) = src.read(&path) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with("--") {
                continue;
            }
            if let Some(rest) = l.strip_prefix("Category")
                && let Some((_, v)) = rest.split_once('=')
            {
                // A script can serve several goals: `Category = "Build_Starbase_Upgrade | Build_Initial_Starbase"`.
                let cats: Vec<String> = v.trim().trim_matches('"').trim_matches('\'').split('|').map(|c| c.trim().to_ascii_lowercase()).filter(|c| !c.is_empty()).collect();
                if !cats.is_empty() {
                    let name = rest_name(&path);
                    for cat in cats {
                        out.entry(cat).or_default().push(name.clone());
                    }
                    break;
                }
            }
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

fn rest_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).trim_end_matches(".LUA").to_string()
}

impl AiEngine {
    /// Creates the engine.
    pub fn new(data: Arc<AiData>, plans: HashMap<String, Vec<String>>) -> Self {
        Self {
            data,
            plans,
            brains: Vec::new(),
            snap: None,
            rng: 0x2545_F491_4F6C_DD1D,
            next_plan: 0,
            interval: 8.0,
            task_forces: TaskForces::default(),
            strategy: Strategy::default(),
            strategy_at: f64::NAN,
            stats: AiStats::default(),
            expired: Vec::new(),
        }
    }

    /// Statistics so far.
    pub fn stats(&self) -> AiStats {
        let mut s = self.stats.clone();
        s.taskforces = self.task_forces.stats.clone();
        s
    }

    /// The loaded AI data.
    pub fn data(&self) -> &Arc<AiData> {
        &self.data
    }

    /// Plan scripts by goal category.
    pub fn plans(&self) -> &HashMap<String, Vec<String>> {
        &self.plans
    }

    /// Registers an AI player of the given AI player type.
    pub fn add_player(&mut self, player: PlayerId, ai_type: &str) -> Result<(), String> {
        let def = self.data.players.get(&ai_type.to_ascii_lowercase()).cloned().ok_or_else(|| format!("unknown AI player type {ai_type}"))?;
        let template = def.galactic_templates.iter().find_map(|t| self.data.templates.get(&t.to_ascii_lowercase()).cloned());
        let mut goals = Vec::new();
        let mut seen = HashSet::new();
        for set in &def.function_sets {
            for (goal, function) in self.data.function_sets.get(&set.to_ascii_lowercase()).into_iter().flatten() {
                let Some(g) = self.data.goals.get(&goal.to_ascii_lowercase()) else { continue };
                if !g.game_mode.eq_ignore_ascii_case("galactic") || !seen.insert(goal.to_ascii_lowercase()) {
                    continue;
                }
                goals.push(GoalEntry { def: g.clone(), function: function.clone() });
            }
        }
        self.brains.push(PlayerBrain {
            player,
            def,
            template,
            goals,
            active: Vec::new(),
            failures: Vec::new(),
            fractions: HashMap::new(),
            next_eval: 2.0 + 3.0 * self.brains.len() as f64,
            goal_next: Vec::new(),
            bonuses: Vec::new(),
            last_fund: None,
            next_tech: 0.0,
        });
        Ok(())
    }

    /// Players the engine runs.
    pub fn players(&self) -> Vec<PlayerId> {
        self.brains.iter().map(|b| b.player).collect()
    }

    /// Active goals of a player as `(goal, target planet, seconds running)`.
    pub fn active_goals(&self, player: PlayerId, now: f64) -> Vec<(String, Option<PlanetId>, f64)> {
        self.brains.iter().find(|b| b.player == player).map(|b| b.active.iter().map(|a| (a.goal.clone(), a.target, now - a.started)).collect()).unwrap_or_default()
    }

    fn snapshot(&mut self, sim: &dyn AiSim, max_age: f64) -> &Snapshot {
        let stale = self.snap.as_ref().is_none_or(|s| sim.time() - s.time > max_age);
        if stale {
            let s = Snapshot::build(sim, self.snap.as_ref());
            self.snap = Some(s);
        }
        self.snap.as_ref().unwrap()
    }

    /// Plan instances that outlived their goal's limit and should be killed.
    pub fn take_expired(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.expired)
    }

    /// Notifies the engine that a plan ended.
    pub fn plan_finished(&mut self, plan: u64, now: f64) {
        let failed = self.task_forces.plan_failed(plan);
        let worked = self.task_forces.plan_worked(plan);
        for b in &mut self.brains {
            if let Some(i) = b.active.iter().position(|a| a.plan == plan) {
                let a = b.active.remove(i);
                let key = (b.player, a.goal.clone());
                let def = self.data.goals.get(&a.goal.to_ascii_lowercase());
                let offensive = a.category.eq_ignore_ascii_case("offensive");
                let tracking = def.map_or(0.0, |d| f64::from(d.tracking_duration));
                let limit = def.map_or(90.0, |d| f64::from(d.time_limit));
                // Failed build plans are retried rarely (nothing to build there); plans that found nothing to
                // do wait longer than ones that built something.
                let cooldown = match (failed, worked, offensive) {
                    (true, _, true) => tracking.max(60.0),
                    (true, _, false) => 300.0,
                    (false, false, _) => 600.0,
                    // An offensive plan that only assembled its force is proposed again at once: the next plan finds the ships ready and attacks.
                    (false, true, true) => self.interval * 2.0,
                    (false, true, false) => limit.max(60.0),
                };
                if failed {
                    *self.stats.plans_failed.entry(key).or_default() += 1;
                } else {
                    *self.stats.plans_succeeded.entry(key).or_default() += 1;
                }
                b.failures.push(Outcome { goal: a.goal, target: a.target, time: now, cooldown, failed });
            }
        }
        self.task_forces.drop_plan(plan);
    }

    /// Runs goal evaluation for players that are due, advances task force commands and returns plans to
    /// start.
    pub fn update(&mut self, sim: &mut dyn AiSim) -> Vec<PlanRequest> {
        let now = sim.time();
        let snap_age = 4.0;
        // Task force jobs run every call.
        {
            let t_snap = std::time::Instant::now();
            self.snapshot(sim, snap_age);
            self.stats.snapshot_nanos += t_snap.elapsed().as_nanos();
            let snap = self.snap.take().unwrap();
            let mut rng = self.rng;
            let t_tick = std::time::Instant::now();
            self.task_forces.tick(sim, &snap, &mut rng);
            self.stats.tick_nanos += t_tick.elapsed().as_nanos();
            self.rng = rng;
            self.snap = Some(snap);
        }
        // Expire plans that run far beyond the goal's time limit.
        for b in &self.brains {
            for a in &b.active {
                if now - a.started > a.limit {
                    self.expired.push(a.plan);
                }
            }
        }
        let mut requests = Vec::new();
        let due: Vec<usize> = (0..self.brains.len()).filter(|&i| now >= self.brains[i].next_eval && !sim.is_eliminated(self.brains[i].player)).collect();
        if due.is_empty() {
            return requests;
        }
        let t0 = std::time::Instant::now();
        let snap = self.snapshot(sim, 0.5).clone();
        if self.strategy.config.enabled && (self.strategy_at.is_nan() || snap.time - self.strategy_at >= 4.0) {
            let t_s = std::time::Instant::now();
            self.strategy.refresh(&snap);
            self.strategy_at = snap.time;
            self.stats.strategy_nanos += t_s.elapsed().as_nanos();
            for b in &self.brains {
                let (p, t) = (self.strategy.personality_at(b.player, snap.time), self.strategy.tier(b.player));
                self.task_forces.set_sizing(b.player, t.contrast * (p.caution / p.aggression.max(0.1)).sqrt());
                let believed: Vec<f64> = (0..snap.owner.len() as u32).map(|pl| self.strategy.believed_hostile_strength(&snap, b.player, pl)).collect();
                self.task_forces.set_believed(b.player, believed);
            }
        }
        // Tech upgrades. In the Remake data the AI has no galactic research plan: `Story/Tech_Levels_*.xml` slave the AI factions' tech level to the one a player
        // reaches (`STORY_TECH_LEVEL` events -> `SET_TECH_LEVEL` rewards, run by galaxy-script), the Empire's own chain needs land-base research buildings that do not
        // exist in the space-only game. So with a human player the AI follows the human through the story scripts. Only in games without any human (AI vs AI,
        // spectating) nothing would ever advance, so there this fallback (an **estimate**) applies: every 4 weeks a faction with more than 20000 credits tries
        // the next level at its best planet, level L only from week 10 x L on.
        let week_seconds = if snap.weeks > 0.0 { snap.time / snap.weeks } else { 60.0 };
        for bi in 0..self.brains.len() {
            let (player, next) = (self.brains[bi].player, self.brains[bi].next_tech);
            let me = player as usize;
            if now < next || sim.is_eliminated(player) || snap.human.iter().any(|h| *h) {
                continue;
            }
            self.brains[bi].next_tech = now + 4.0 * week_seconds;
            if snap.tech[me] >= snap.max_tech[me] || snap.weeks < 10.0 * f64::from(snap.tech[me]) || snap.credits[me] < 20_000.0 {
                continue;
            }
            let mut planets = snap.owned[me].clone();
            planets.sort_by_key(|&p| std::cmp::Reverse(snap.starbase[p as usize].0));
            for pl in planets.into_iter().take(8) {
                if sim.queue_tech(player, pl).is_ok() {
                    self.stats.tech_upgrades += 1;
                    break;
                }
            }
        }
        for i in due {
            let sleep = self.strategy.tier(self.brains[i].player).goal_sleep;
            self.brains[i].next_eval = now + self.interval * (1.0 + sleep);
            self.evaluate_player(i, &snap, now, &mut requests);
        }
        self.stats.eval_nanos += t0.elapsed().as_nanos();
        requests
    }

    fn view(&self, i: usize) -> PlayerBrainView {
        let b = &self.brains[i];
        let mut v = PlayerBrainView::default();
        for a in &b.active {
            *v.active_goals.entry(a.category.to_ascii_lowercase()).or_default() += 1;
        }
        v.budget_fraction = b.fractions.clone();
        v
    }

    fn evaluate_player(&mut self, i: usize, snap: &Snapshot, now: f64, out: &mut Vec<PlanRequest>) {
        let data = self.data.clone();
        let player = self.brains[i].player;
        let me = player as usize;
        let mut rng = self.rng;
        let mut stats = EvalStats::default();
        let mut evals = 0u64;

        // Budgets.
        let view = self.view(i);
        let mut fractions: HashMap<String, f64> = HashMap::new();
        if let Some(t) = &self.brains[i].template {
            let mut total = 0.0;
            for (cat, eq) in &t.budget {
                let mut ctx = PerceptionCtx { snap, player, target: None, brain: &view, equations: &data.equations, rng: &mut rng, stats: &mut EvalStats::default() };
                let w = data.equations.evaluate(eq, &mut ctx, &mut stats).max(0.0);
                evals += 1;
                fractions.insert(cat.to_ascii_lowercase(), w);
                total += w;
            }
            if total > 0.0 {
                for v in fractions.values_mut() {
                    *v /= total;
                }
            }
        }
        self.brains[i].fractions = fractions.clone();
        // Earmark money by budget share: the treasury at the first evaluation, then the net income since the last one.
        let amount = match self.brains[i].last_fund {
            None => snap.credits[me],
            Some(t) if snap.weeks > 0.0 => snap.net_income[me] * (now - t) / (snap.time / snap.weeks),
            Some(_) => 0.0,
        };
        // Windfalls (capture bonuses, rewards, the treasury of a loaded game) are not income: whatever the treasury holds beyond the
        // earmarked money and `AI_RESERVE_WEEKS` of income is budgeted too, so the AI does not sit on a growing hoard (estimate).
        let surplus = (snap.credits[me] - self.task_forces.earmarked(player) - amount - AI_RESERVE_WEEKS * snap.net_income[me].max(0.0)).max(0.0);
        let amount = amount + surplus;
        self.brains[i].last_fund = Some(now);
        self.task_forces.fund(player, &fractions, amount);
        let view = self.view(i);
        let enabled: HashSet<String> = self.brains[i]
            .template
            .as_ref()
            .map(|t| t.goal_categories.iter().map(|c| c.to_ascii_lowercase()).collect())
            .unwrap_or_default();

        // Distances (hops) from owned planets for reachability.
        let reach = reach_from_owned(snap, player, 3);

        // Goals that are due, can still start a plan and have a budget.
        struct Eligible {
            gi: usize,
            friendly: bool,
            enemy: bool,
            neutral: bool,
            needs_reach: bool,
        }
        let mut eligible: Vec<Eligible> = Vec::new();
        for (gi, g) in self.brains[i].goals.iter().enumerate() {
            let cat = g.def.category.to_ascii_lowercase();
            if (!enabled.is_empty() && !enabled.contains(&cat) && cat != "nobudget") || !self.plans.contains_key(&g.def.name.to_ascii_lowercase()) {
                continue;
            }
            let b = &self.brains[i];
            if now < b.goal_next.get(gi).copied().unwrap_or(0.0) {
                continue;
            }
            let active_in_cat = b.active.iter().filter(|a| a.category.eq_ignore_ascii_case(&g.def.category)).count();
            match fractions.get(&cat) {
                Some(s) if *s <= 0.0 => continue,
                Some(s) if active_in_cat >= ((s * (3 + b.def.goal_set_extension_size as usize) as f64).ceil() as usize).max(1) => continue,
                None if active_in_cat >= 2 => continue,
                _ => {}
            }
            if b.active.iter().filter(|a| a.goal.eq_ignore_ascii_case(&g.def.name)).count() >= 2 {
                continue;
            }
            let flags: Vec<String> = g.def.flags.iter().map(|f| f.to_ascii_lowercase()).collect();
            eligible.push(Eligible {
                gi,
                friendly: flags.iter().any(|f| f == "friendly"),
                enemy: flags.iter().any(|f| f == "enemy"),
                neutral: flags.iter().any(|f| f == "neutral"),
                needs_reach: !g.def.reachability.eq_ignore_ascii_case("any"),
            });
        }
        // Infrastructure goals are re-evaluated less often than offensive ones.
        for e in &eligible {
            let slow = !self.brains[i].goals[e.gi].def.category.eq_ignore_ascii_case("offensive");
            let wait = if slow { self.interval * 4.0 } else { self.interval };
            let b = &mut self.brains[i];
            if b.goal_next.len() <= e.gi {
                b.goal_next.resize(e.gi + 1, 0.0);
            }
            b.goal_next[e.gi] = now + wait;
        }

        let mut candidates: Vec<(f64, usize, Option<PlanetId>)> = Vec::new();
        let mut cache = EvalCache::new();
        for pl in 0..snap.owner.len() as u32 {
            if !snap.real[pl as usize] {
                continue;
            }
            let owner = snap.owner[pl as usize];
            cache.clear();
            let mut ctx = PerceptionCtx { snap, player, target: Some(pl), brain: &view, equations: &data.equations, rng: &mut rng, stats: &mut EvalStats::default() };
            for e in &eligible {
                let ok = match owner {
                    Some(o) if o == player => e.friendly,
                    Some(o) => e.enemy && snap.enemy[me][o as usize],
                    None => e.neutral,
                };
                if !ok || (e.needs_reach && owner != Some(player) && reach[pl as usize] == u32::MAX) {
                    continue;
                }
                let g = &self.brains[i].goals[e.gi];
                let b = &self.brains[i];
                if b.failures.iter().any(|f| f.target == Some(pl) && now - f.time < f.cooldown && f.goal.eq_ignore_ascii_case(&g.def.name)) {
                    continue;
                }
                let mut desire = data.equations.evaluate_cached(&g.function, &mut ctx, &mut stats, &mut cache);
                evals += 1;
                if desire <= 0.0 {
                    continue;
                }
                // Failure learning and desire bonuses.
                let fails = b.failures.iter().filter(|f| f.failed && f.target == Some(pl) && now - f.time < f64::from(g.def.tracking_duration.max(1.0)) && f.goal.eq_ignore_ascii_case(&g.def.name)).count() as f64;
                desire += fails * f64::from(g.def.per_failure_desire_adjust);
                desire += b.bonuses.iter().filter(|x| x.0.eq_ignore_ascii_case(&g.def.name) && x.1 == pl && now < x.3).map(|x| x.2).sum::<f64>();
                if self.strategy.config.enabled {
                    desire *= self.strategy.factor(snap, player, pl, g.def.category.eq_ignore_ascii_case("offensive"));
                }
                if desire > 0.0 {
                    candidates.push((desire, e.gi, Some(pl)));
                }
            }
        }        self.brains[i].failures.retain(|f| now - f.time < f.cooldown.max(600.0));
        self.brains[i].bonuses.retain(|b| now < b.3);
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));

        // Activation.
        let ext = self.brains[i].def.goal_set_extension_size as usize;
        let max_total = 3 + ext;
        let mut started = 0;
        for (_, gi, target) in candidates {
            if started >= 3 || self.brains[i].active.len() >= max_total {
                break;
            }
            let (goal, category, limit) = {
                let g = &self.brains[i].goals[gi];
                (g.def.name.clone(), g.def.category.clone(), f64::from(g.def.time_limit.max(60.0) * g.def.build_time_delay_tolerance.max(1.0)) * PLAN_LIMIT_FACTOR)
            };
            let b = &self.brains[i];
            if b.active.iter().any(|a| a.goal.eq_ignore_ascii_case(&goal) && a.target == target) {
                continue;
            }
            if b.active.iter().filter(|a| a.goal.eq_ignore_ascii_case(&goal)).count() >= 2 {
                continue;
            }
            let cat_l = category.to_ascii_lowercase();
            let share = fractions.get(&cat_l).copied();
            if let Some(s) = share {
                if s <= 0.0 {
                    continue;
                }
                let cap = ((s * max_total as f64).ceil() as usize).max(1);
                if b.active.iter().filter(|a| a.category.eq_ignore_ascii_case(&category)).count() >= cap {
                    continue;
                }
            } else if b.active.iter().filter(|a| a.category.eq_ignore_ascii_case(&category)).count() >= 2 {
                continue;
            }
            let Some(scripts) = self.plans.get(&goal.to_ascii_lowercase()) else {
                *self.stats.goals_without_plan.entry(goal).or_default() += 1;
                continue;
            };
            // Pincer: a target too strong for one force but within reach of two is attacked by two coordinated plans.
            let copies = match target {
                Some(tp) if self.strategy.config.enabled && category.eq_ignore_ascii_case("offensive") && snap.owner[tp as usize].is_some_and(|o| snap.enemy[me][o as usize]) && self.brains[i].active.len() + 2 <= max_total => {
                    let need = self.strategy.believed_hostile_strength(snap, player, tp) * 1.25 * self.task_forces.sizing_of(player);
                    let own = self.strategy.total_strength(player);
                    if need > 0.5 * own && need < 0.9 * own { 2 } else { 1 }
                }
                _ => 1,
            };
            if copies > 1
                && let Some(tp) = target
            {
                self.task_forces.set_split(player, tp, copies);
            }
            for _ in 0..copies {
                let script = scripts[(self.next_plan as usize) % scripts.len()].clone();
                self.next_plan += 1;
                let plan = self.next_plan;
                *self.stats.plans_started.entry((player, goal.clone())).or_default() += 1;
                self.task_forces.set_plan_category(plan, &category);
                self.brains[i].active.push(ActiveGoal { goal: goal.clone(), category: category.clone(), target, plan, started: now, limit });
                out.push(PlanRequest { plan, script, player, target, goal: goal.clone() });
            }
            started += 1;
        }
        self.rng = rng;
        self.stats.evaluations += evals;
        for (k, v) in stats.unknown_variables {
            *self.stats.eval.unknown_variables.entry(k).or_default() += v;
        }
        for (k, v) in stats.missing_equations {
            *self.stats.eval.missing_equations.entry(k).or_default() += v;
        }
    }

    // ---- script-facing API ----------------------------------------------------------------------------------

    /// `EvaluatePerception(name, player, target)`.
    pub fn evaluate_perception(&mut self, sim: &dyn AiSim, name: &str, player: PlayerId, target: Option<u64>) -> Option<f64> {
        let key = name.to_ascii_lowercase();
        self.data.equations.get(&key)?;
        let target = target.and_then(|o| sim.object_planet(o));
        let idx = self.brains.iter().position(|b| b.player == player);
        let view = idx.map(|i| self.view(i)).unwrap_or_default();
        let data = self.data.clone();
        let mut rng = self.rng;
        self.snapshot(sim, 1.0);
        let snap = self.snap.as_ref().unwrap();
        let mut stats = EvalStats::default();
        let mut ctx = PerceptionCtx { snap, player, target, brain: &view, equations: &data.equations, rng: &mut rng, stats: &mut EvalStats::default() };
        let v = data.equations.evaluate(&key, &mut ctx, &mut stats);
        self.rng = rng;
        for (k, n) in stats.unknown_variables {
            *self.stats.eval.unknown_variables.entry(k).or_default() += n;
        }
        for (k, n) in stats.missing_equations {
            *self.stats.eval.missing_equations.entry(k).or_default() += n;
        }
        Some(v)
    }

    /// Creates a task force for a plan.
    pub fn create_taskforce(&mut self, sim: &dyn AiSim, spec: &TaskForceSpec) -> u64 {
        self.task_forces.create(sim, spec)
    }

    /// Method call on a task force.
    pub fn taskforce_call(&mut self, sim: &mut dyn AiSim, tf: u64, method: &str, args: &[HostValue]) -> Option<HostValue> {
        self.snapshot(sim, 1.0);
        let snap = self.snap.take().unwrap();
        let mut rng = self.rng;
        let r = self.task_forces.call(sim, &snap, &mut rng, tf, method, args);
        self.rng = rng;
        self.snap = Some(snap);
        r
    }

    /// State of a command block.
    pub fn block_status(&self, block: u64) -> (bool, Option<bool>) {
        self.task_forces.block_status(block)
    }

    /// Callbacks for plan scripts.
    pub fn take_plan_events(&mut self) -> Vec<PlanEvent> {
        self.task_forces.take_events()
    }

    /// Engine AI functions (`FindTarget`, `GiveDesireBonus`, ...).
    pub fn ai_call(&mut self, sim: &mut dyn AiSim, function: &str, args: &[HostValue]) -> Option<HostValue> {
        let now = sim.time();
        match function {
            "GiveDesireBonus" => {
                // (player, goal, target, bonus, duration)
                let (Some(HostValue::Player(p)), Some(HostValue::Str(goal)), Some(HostValue::Object(t))) = (args.first(), args.get(1), args.get(2)) else { return Some(HostValue::Nil) };
                let pl = sim.object_planet(*t)?;
                let (bonus, dur) = (num(args.get(3)), num(args.get(4)));
                if let Some(b) = self.brains.iter_mut().find(|b| b.player == *p) {
                    b.bonuses.push((goal.clone(), pl, bonus, now + dur.max(1.0)));
                }
                Some(HostValue::Nil)
            }
            "WaitForStarbase" => Some(HostValue::Bool(true)),
            "FindTarget" | "FindTarget.Reachable_Target" => self.find_target(sim, function, args),
            "_FindStageArea" => {
                let (Some(HostValue::Player(p)), Some(HostValue::Object(t))) = (args.first(), args.get(1)) else { return Some(HostValue::Nil) };
                let target = sim.object_planet(*t)?;
                self.snapshot(sim, 1.0);
                let snap = self.snap.as_ref().unwrap();
                let dist = hop_dist(snap, target);
                let best = snap.owned[*p as usize].iter().copied().min_by_key(|&o| dist[o as usize]);
                Some(best.map_or(HostValue::Nil, |b| HostValue::Object(sim.planet_object(b))))
            }
            "_ProduceObject" => {
                // (player, type name, stage)
                let (Some(HostValue::Player(p)), Some(name)) = (args.first(), args.get(1)) else { return Some(HostValue::Nil) };
                let name = match name {
                    HostValue::Str(s) | HostValue::Type(s) => s.clone(),
                    _ => return Some(HostValue::Nil),
                };
                let planet = match args.get(2) {
                    Some(HostValue::Object(o)) => sim.object_planet(*o),
                    _ => None,
                };
                let ok = planet.is_some_and(|pl| sim.queue_structure(*p, pl, &name).is_ok() || sim.queue_unit(*p, pl, &name).is_ok());
                let block = self.task_forces.immediate_block(ok);
                Some(HostValue::Block(block))
            }
            "Find_Path" => Some(HostValue::List(Vec::new())),
            _ => None,
        }
    }

    fn find_target(&mut self, sim: &mut dyn AiSim, function: &str, args: &[HostValue]) -> Option<HostValue> {
        // FindTarget(taskforce, perception, flags, probability)
        // FindTarget.Reachable_Target(player, perception, flags, reachability, probability, source)
        let (player, perception, flags, prob) = if function == "FindTarget" {
            let tf = match args.first() {
                Some(HostValue::TaskForce(t)) => *t,
                _ => return Some(HostValue::Nil),
            };
            let player = self.task_forces.forces.get(&tf)?.player;
            (player, args.get(1), args.get(2), args.get(3))
        } else {
            let player = match args.first() {
                Some(HostValue::Player(p)) => *p,
                _ => return Some(HostValue::Nil),
            };
            (player, args.get(1), args.get(2), args.get(4))
        };
        let Some(HostValue::Str(perception)) = perception else { return Some(HostValue::Nil) };
        let flags = match flags {
            Some(HostValue::Str(f)) => f.to_ascii_lowercase(),
            _ => "friendly | enemy | neutral".into(),
        };
        let prob = if prob.is_some() { num(prob).clamp(0.0, 1.0) } else { 1.0 };
        let key = perception.to_ascii_lowercase();
        let data = self.data.clone();
        data.equations.get(&key)?;
        let idx = self.brains.iter().position(|b| b.player == player);
        let view = idx.map(|i| self.view(i)).unwrap_or_default();
        self.snapshot(sim, 1.0);
        let snap = self.snap.as_ref().unwrap();
        let me = player as usize;
        let mut rng = self.rng;
        let mut scored: Vec<(f64, PlanetId)> = Vec::new();
        let mut stats = EvalStats::default();
        for pl in 0..snap.owner.len() as u32 {
            if !snap.real[pl as usize] {
                continue;
            }
            let ok = match snap.owner[pl as usize] {
                Some(o) if o == player => flags.contains("friendly"),
                Some(o) => flags.contains("enemy") && snap.enemy[me][o as usize],
                None => flags.contains("neutral"),
            };
            if !ok {
                continue;
            }
            let mut ctx = PerceptionCtx { snap, player, target: Some(pl), brain: &view, equations: &data.equations, rng: &mut rng, stats: &mut EvalStats::default() };
            let v = data.equations.evaluate(&key, &mut ctx, &mut stats);
            if v > 0.0 {
                scored.push((v, pl));
            }
        }
        self.rng = rng;
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let pick = if scored.len() > 1 && prob < 1.0 {
            // With probability `prob` take the best, otherwise one of the others.
            let r = (self.rng >> 11) as f64 / (1u64 << 53) as f64;
            self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            if r < prob { scored[0] } else { scored[1 + (self.rng as usize % (scored.len() - 1))] }
        } else {
            *scored.first()?
        };
        Some(HostValue::Object(sim.planet_object(pick.1)))
    }
}

fn num(v: Option<&HostValue>) -> f64 {
    match v {
        Some(HostValue::Num(n)) => *n,
        _ => 0.0,
    }
}

fn hop_dist(snap: &Snapshot, from: PlanetId) -> Vec<u32> {
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

/// Hop distance from the player's planets (multi-source BFS), `u32::MAX` beyond `max_hops`.
fn reach_from_owned(snap: &Snapshot, player: PlayerId, max_hops: u32) -> Vec<u32> {
    let mut d = vec![u32::MAX; snap.adjacency.len()];
    let mut q = VecDeque::new();
    for &p in &snap.owned[player as usize] {
        d[p as usize] = 0;
        q.push_back(p);
    }
    while let Some(p) = q.pop_front() {
        if d[p as usize] >= max_hops {
            continue;
        }
        for &n in &snap.adjacency[p as usize] {
            if d[n as usize] == u32::MAX {
                d[n as usize] = d[p as usize] + 1;
                q.push_back(n);
            }
        }
    }
    d
}

/// Parameters helper used by tests of the scripted evaluators.
pub fn no_params() -> Params {
    Params::default()
}

/// Per-player memory of the goal system.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrainMemory {
    /// Player.
    pub player: PlayerId,
    /// Next goal evaluation.
    pub next_eval: f64,
    /// Earliest evaluation per goal (in the order of the player goal list).
    pub goal_next: Vec<f64>,
    /// `(goal, planet, value, until)` desire bonuses.
    pub bonuses: Vec<(String, PlanetId, f64, f64)>,
    /// Time money was last earmarked.
    pub last_fund: Option<f64>,
    /// Next tech consideration.
    pub next_tech: f64,
    /// Budget fractions per category.
    pub fractions: Vec<(String, f64)>,
    /// Finished plans (cooldowns and failure learning): `(goal, target, time, cooldown, failed)`.
    pub failures: Vec<(String, Option<PlanetId>, f64, f64, bool)>,
}

/// What the AI keeps between turns and what a save stores: goal timers and learned failures, budgets, force sizing, reservations, the strategic layer memory.
/// Not stored: plans in flight (their Lua coroutines cannot be saved) and the task forces they own. A loaded game starts with no plan running; the ships
/// and queued items stay where they are and the goals pick them up again (ships on the free store are used first).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiMemory {
    /// Engine random generator.
    pub rng: u64,
    /// Next plan id.
    pub next_plan: u64,
    /// Time of the last strategy refresh (`None` = never).
    pub strategy_at: Option<f64>,
    /// Players.
    pub brains: Vec<BrainMemory>,
    /// Task force bookkeeping.
    pub task_forces: crate::taskforce::TaskForceMemory,
    /// Strategic layer.
    pub strategy: crate::strategy::StrategyMemory,
}

impl AiEngine {
    /// Copies the long-lived state.
    pub fn memory(&self) -> AiMemory {
        AiMemory {
            rng: self.rng,
            next_plan: self.next_plan,
            strategy_at: (!self.strategy_at.is_nan()).then_some(self.strategy_at),
            brains: self
                .brains
                .iter()
                .map(|b| {
                    let mut fractions: Vec<(String, f64)> = b.fractions.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    fractions.sort_by(|a, b| a.0.cmp(&b.0));
                    BrainMemory {
                        player: b.player,
                        next_eval: b.next_eval,
                        goal_next: b.goal_next.clone(),
                        bonuses: b.bonuses.clone(),
                        last_fund: b.last_fund,
                        next_tech: b.next_tech,
                        fractions,
                        failures: b.failures.iter().map(|o| (o.goal.clone(), o.target, o.time, o.cooldown, o.failed)).collect(),
                    }
                })
                .collect(),
            task_forces: self.task_forces.snapshot(),
            strategy: self.strategy.snapshot(),
        }
    }

    /// Puts the memory into an engine set up like the saved one (same players); plans in flight are dropped. Returns warnings.
    pub fn restore_memory(&mut self, m: &AiMemory) -> Vec<String> {
        let mut warnings = Vec::new();
        self.rng = m.rng;
        self.next_plan = m.next_plan;
        self.strategy_at = m.strategy_at.unwrap_or(f64::NAN);
        for bm in &m.brains {
            let Some(b) = self.brains.iter_mut().find(|b| b.player == bm.player) else {
                warnings.push(format!("AI player {} of the save is not in this game", bm.player));
                continue;
            };
            b.next_eval = bm.next_eval;
            b.goal_next = bm.goal_next.clone();
            b.bonuses = bm.bonuses.clone();
            b.last_fund = bm.last_fund;
            b.next_tech = bm.next_tech;
            b.fractions = bm.fractions.iter().cloned().collect();
            b.failures = bm.failures.iter().map(|(g, t, time, cooldown, failed)| Outcome { goal: g.clone(), target: *t, time: *time, cooldown: *cooldown, failed: *failed }).collect();
            b.active.clear();
        }
        self.task_forces.restore(&m.task_forces);
        self.strategy.restore(&m.strategy);
        self.expired.clear();
        warnings
    }
}

/// Weeks of income the AI keeps unbudgeted in its treasury; everything above is earmarked for plans (estimate, the data has no rule).
const AI_RESERVE_WEEKS: f64 = 4.0;
