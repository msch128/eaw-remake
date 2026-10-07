//! The variables of the perceptual equations, computed from an [`AiSim`] snapshot.
//!
//! Definitions are reconstructed from how the equations use them (the engine's own definitions are not
//! available). Where a variable is "normalized" it is divided by the largest value on the map; the
//! `NBT*` variants (`SpaceTotalNBTD`, `BaseIncomeNBTP`, ...) are approximated by their plain versions.
//! Anything not listed in `AI.md` reads as 0 and is counted in the evaluation statistics.

use std::collections::HashMap;

use crate::equation::{EquationContext, EquationSet, EvalStats, ParamValue, Params};
use crate::sim::{AiSim, PlanetId, PlayerId};

/// Per-tick facts shared by all evaluations of all players.
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    /// Game time of the snapshot.
    pub time: f64,
    /// Owner per planet.
    pub owner: Vec<Option<PlayerId>>,
    /// Real planets.
    pub real: Vec<bool>,
    /// Number of real planets.
    pub real_count: u32,
    /// Hyperspace neighbours.
    pub adjacency: Vec<Vec<PlanetId>>,
    /// Most neighbours of any planet.
    pub max_neighbors: f64,
    /// Planet base income.
    pub income: Vec<f64>,
    /// Largest base income.
    pub max_income: f64,
    /// Space power per planet and player (only entries > 0).
    pub power: Vec<Vec<(PlayerId, f64)>>,
    /// Largest total power of any single player at any planet.
    pub max_power: f64,
    /// Built structures per planet.
    pub structures: Vec<Vec<String>>,
    /// Structures in production per planet.
    pub queued: Vec<Vec<String>>,
    /// `(orbital, ground)` slots.
    pub slots: Vec<(i32, i32)>,
    /// `(level, max level)` of the star base.
    pub starbase: Vec<(i32, i32)>,
    /// Owned real planets per player.
    pub owned: Vec<Vec<PlanetId>>,
    /// Faction names.
    pub names: Vec<String>,
    /// Human flags.
    pub human: Vec<bool>,
    /// Credits per player.
    pub credits: Vec<f64>,
    /// Net income per week per player.
    pub net_income: Vec<f64>,
    /// Tech levels.
    pub tech: Vec<i32>,
    /// Max tech levels.
    pub max_tech: Vec<i32>,
    /// Hostility matrix.
    pub enemy: Vec<Vec<bool>>,
    /// Total ship power per player.
    pub total_power: Vec<f64>,
    /// Ship types each player owns.
    pub unit_types: Vec<std::collections::HashSet<String>>,
    /// Planet names (lower case, campaign suffix stripped).
    pub short_names: Vec<String>,
    /// Weeks since the start.
    pub weeks: f64,
    /// Combat strength (square-law measure, see [`AiSim::force_strength`]) of the fleets of each player at each planet (entries > 0).
    pub strength: Vec<Vec<(PlayerId, f64)>>,
    /// Strength of the fixed defenses of each planet for its owner.
    pub static_defense: Vec<f64>,
}

impl Snapshot {
    /// Builds a snapshot; `adjacency` and names are re-read only when `static_part` is `None`.
    pub fn build(sim: &dyn AiSim, prev: Option<&Snapshot>) -> Snapshot {
        let n = sim.planet_count() as usize;
        let players = sim.player_count() as usize;
        let mut s = Snapshot { time: sim.time(), ..Snapshot::default() };
        match prev {
            Some(p) => {
                s.adjacency = p.adjacency.clone();
                s.real = p.real.clone();
                s.income = p.income.clone();
                s.slots = p.slots.clone();
                s.short_names = p.short_names.clone();
                s.names = p.names.clone();
                s.max_neighbors = p.max_neighbors;
                s.max_income = p.max_income;
                s.real_count = p.real_count;
                s.max_tech = p.max_tech.clone();
            }
            None => {
                s.adjacency = (0..n as u32).map(|p| sim.neighbors(p)).collect();
                s.real = (0..n as u32).map(|p| sim.is_real(p)).collect();
                s.income = (0..n as u32).map(|p| sim.planet_income(p)).collect();
                s.slots = (0..n as u32).map(|p| sim.structure_slots(p)).collect();
                s.short_names = (0..n as u32)
                    .map(|p| {
                        let name = sim.planet_name(p).to_ascii_lowercase();
                        name.rsplit_once('_').map_or(name.clone(), |(a, b)| if b == "ww" { a.to_string() } else { name.clone() })
                    })
                    .collect();
                s.names = (0..players as u32).map(|p| sim.faction_name(p)).collect();
                s.max_neighbors = s.adjacency.iter().map(|a| a.len() as f64).fold(1.0, f64::max);
                s.max_income = s.income.iter().copied().fold(1.0, f64::max);
                s.real_count = s.real.iter().filter(|r| **r).count() as u32;
                s.max_tech = (0..players as u32).map(|p| sim.max_tech_level(p)).collect();
            }
        }
        s.owner = (0..n as u32).map(|p| sim.planet_owner(p)).collect();
        s.structures = (0..n as u32).map(|p| sim.structures(p)).collect();
        s.queued = (0..n as u32).map(|p| sim.build_queue(p).into_iter().map(|q| q.name).collect()).collect();
        s.starbase = (0..n as u32).map(|p| sim.starbase_level(p)).collect();
        s.human = (0..players as u32).map(|p| sim.is_human(p)).collect();
        s.credits = (0..players as u32).map(|p| sim.credits(p)).collect();
        s.net_income = (0..players as u32).map(|p| sim.income_per_week(p)).collect();
        s.tech = (0..players as u32).map(|p| sim.tech_level(p)).collect();
        s.enemy = (0..players as u32).map(|a| (0..players as u32).map(|b| sim.is_enemy(a, b)).collect()).collect();
        s.owned = vec![Vec::new(); players];
        for (pl, o) in s.owner.iter().enumerate() {
            if let Some(o) = o
                && s.real[pl]
                && (*o as usize) < players
            {
                s.owned[*o as usize].push(pl as u32);
            }
        }
        s.power = vec![Vec::new(); n];
        s.total_power = vec![0.0; players];
        s.unit_types = vec![Default::default(); players];
        let mut unit_power: HashMap<String, f64> = HashMap::new();
        for p in 0..players as u32 {
            for f in sim.fleets_of(p) {
                let mut fp = 0.0;
                for (name, count) in &f.units {
                    let pw = *unit_power.entry(name.clone()).or_insert_with(|| sim.unit_info(name).map_or(1.0, |u| u.power.max(1.0)));
                    fp += pw * f64::from(*count);
                    s.unit_types[p as usize].insert(name.to_ascii_lowercase());
                }
                s.total_power[p as usize] += fp;
                if let Some(at) = f.at {
                    let slot = &mut s.power[at as usize];
                    match slot.iter_mut().find(|e| e.0 == p) {
                        Some(e) => e.1 += fp,
                        None => slot.push((p, fp)),
                    }
                }
            }
        }
        s.max_power = s.power.iter().flatten().map(|e| e.1).fold(1000.0, f64::max);
        // Combat strength of the forces at each planet and of the fixed defenses (for the strategy layer).
        let mut groups: HashMap<(PlanetId, PlayerId), Vec<(String, u32)>> = HashMap::new();
        for p in 0..players as u32 {
            for f in sim.fleets_of(p) {
                if let Some(at) = f.at {
                    groups.entry((at, p)).or_default().extend(f.units.iter().cloned());
                }
            }
        }
        s.strength = vec![Vec::new(); n];
        let mut keys: Vec<_> = groups.keys().copied().collect();
        keys.sort_unstable();
        for (at, p) in keys {
            let v = sim.force_strength(&groups[&(at, p)]);
            if v > 0.0 {
                s.strength[at as usize].push((p, v));
            }
        }
        s.static_defense = (0..n as u32).map(|pl| if s.owner[pl as usize].is_some() && s.real[pl as usize] { sim.planet_defense(pl) } else { 0.0 }).collect();
        s.weeks = s.time / sim.week_seconds().max(1.0);
        s
    }

    /// Power of players satisfying `pred` at a planet.
    pub fn power_at(&self, pl: PlanetId, pred: impl Fn(PlayerId) -> bool) -> f64 {
        self.power[pl as usize].iter().filter(|e| pred(e.0)).map(|e| e.1).sum()
    }
}

/// State of the AI player the evaluation runs for.
#[derive(Debug, Default, Clone)]
pub struct PlayerBrainView {
    /// Active goals per category (lower case).
    pub active_goals: HashMap<String, u32>,
    /// Budget fraction per category (lower case).
    pub budget_fraction: HashMap<String, f64>,
}

/// Evaluation context for one `(player, target)`.
pub struct PerceptionCtx<'a> {
    /// Snapshot.
    pub snap: &'a Snapshot,
    /// The evaluating player.
    pub player: PlayerId,
    /// The target planet.
    pub target: Option<PlanetId>,
    /// Brain state of the player.
    pub brain: &'a PlayerBrainView,
    /// Equations (for the native evaluator scripts).
    pub equations: &'a EquationSet,
    /// Random state.
    pub rng: &'a mut u64,
    /// Statistics sink for nested evaluations.
    pub stats: &'a mut EvalStats,
}

fn matches_type(names: &[&str], candidate: &str) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(candidate))
}

impl PerceptionCtx<'_> {
    fn is_friend(&self, p: PlayerId) -> bool {
        p == self.player
    }

    fn is_foe(&self, p: PlayerId) -> bool {
        self.snap.enemy.get(self.player as usize).and_then(|r| r.get(p as usize)).copied().unwrap_or(false)
    }

    /// Faction name of a planet's owner; unowned real planets belong to the `Pirates`.
    fn owner_faction(&self, pl: PlanetId) -> String {
        match self.snap.owner[pl as usize] {
            Some(o) => self.snap.names[o as usize].clone(),
            None => "Pirates".to_string(),
        }
    }

    fn structure_names(&self, pl: PlanetId, complete_only: bool) -> Vec<&str> {
        let mut v: Vec<&str> = self.snap.structures[pl as usize].iter().map(String::as_str).collect();
        if !complete_only {
            v.extend(self.snap.queued[pl as usize].iter().map(String::as_str));
        }
        v
    }

    fn count_structures(&self, planets: &[PlanetId], types: &[&str], complete_only: bool) -> f64 {
        planets
            .iter()
            .map(|&pl| self.structure_names(pl, complete_only).iter().filter(|s| matches_type(types, s)).count() as f64)
            .sum()
    }

    fn target_planet(&self) -> Option<PlanetId> {
        self.target
    }

    fn force(&self, pl: PlanetId, friendly: bool) -> f64 {
        let p = if friendly { self.snap.power_at(pl, |o| self.is_friend(o)) } else { self.snap.power_at(pl, |o| self.is_foe(o)) };
        // Planets of enemies defend with their star base.
        let owner_foe = self.snap.owner[pl as usize].is_some_and(|o| if friendly { self.is_friend(o) } else { self.is_foe(o) });
        let sb = if owner_foe { f64::from(self.snap.starbase[pl as usize].0) * 400.0 } else { 0.0 };
        p + sb
    }

    fn neighbors_frac(&self, pl: PlanetId, pred: impl Fn(Option<PlayerId>) -> bool) -> f64 {
        let adj = &self.snap.adjacency[pl as usize];
        if adj.is_empty() {
            return 0.0;
        }
        adj.iter().filter(|&&n| pred(self.snap.owner[n as usize])).count() as f64 / adj.len() as f64
    }

    fn two_hop_frac(&self, pl: PlanetId, pred: impl Fn(Option<PlayerId>) -> bool) -> f64 {
        let mut seen = std::collections::HashSet::new();
        for &n in &self.snap.adjacency[pl as usize] {
            for &m in &self.snap.adjacency[n as usize] {
                if m != pl {
                    seen.insert(m);
                }
            }
        }
        if seen.is_empty() {
            return 0.0;
        }
        seen.iter().filter(|&&m| pred(self.snap.owner[m as usize])).count() as f64 / seen.len() as f64
    }

    fn self_var(&mut self, path: &str, params: &Params) -> Option<f64> {
        let s = self.snap;
        let me = self.player as usize;
        let owned = &s.owned[me];
        let owned_n = owned.len() as f64;
        Some(match path {
            "IsFaction" => params.strs("faction").iter().any(|f| f.eq_ignore_ascii_case(&s.names[me])) as u8 as f64,
            "PlanetsControlledUnnormalized" => owned_n,
            "PlanetsControlled" => owned_n / f64::from(s.real_count.max(1)),
            "CreditsUnnormalized" => s.credits[me],
            "NetIncomeUnnormalized" => s.net_income[me],
            "TechLevel" => f64::from(s.tech[me]),
            "CanAdvanceTech" => (s.tech[me] < s.max_tech[me]) as u8 as f64,
            "IsDifficulty" => params.strs("difficulty_level_type").iter().any(|d| d.eq_ignore_ascii_case("normal")) as u8 as f64,
            "ActiveGoals" => {
                let cat = params.str("category").unwrap_or("").to_ascii_lowercase();
                f64::from(self.brain.active_goals.get(&cat).copied().unwrap_or(0))
            }
            "BudgetAllocation" | "BudgetFractionToBuild" => 0.0,
            "StructureCount" | "HasStructure" => {
                let types = params.strs("type");
                let complete = params.num("only_consider_complete").unwrap_or(0.0) > 0.0;
                let n = self.count_structures(owned, &types, complete);
                if path == "HasStructure" { (n > 0.0) as u8 as f64 } else { n }
            }
            "HasUnit" => {
                let types = params.strs("type");
                types.iter().any(|t| s.unit_types[me].contains(&t.to_ascii_lowercase())) as u8 as f64
            }
            "NumStarbaseOfLevel" => {
                let lvl = params.num("level").unwrap_or(1.0) as i32;
                owned.iter().filter(|&&p| s.starbase[p as usize].0 == lvl).count() as f64
            }
            "AverageStarbase" => {
                if owned.is_empty() {
                    0.0
                } else {
                    owned.iter().map(|&p| f64::from(s.starbase[p as usize].0)).sum::<f64>() / owned_n / 5.0
                }
            }
            "AverageGroundbase" => {
                if owned.is_empty() {
                    0.0
                } else {
                    owned.iter().map(|&p| self.groundbase_level(p)).sum::<f64>() / owned_n / 3.0
                }
            }
            "SpaceTotalUnnormalized" => s.total_power[me],
            "SpaceTotal" => (s.total_power[me] / s.max_power).min(1.0),
            "GrossIncomeUnnormalized" => s.net_income[me],
            "IsRetreating" | "CanRetreat" => 0.0,
            _ => return None,
        })
    }

    fn groundbase_level(&self, pl: PlanetId) -> f64 {
        let (_, ground) = self.snap.slots[pl as usize];
        if ground <= 0 {
            return 0.0;
        }
        (self.snap.structures[pl as usize].len() as f64).min(3.0)
    }

    fn target_var(&mut self, path: &str, params: &Params) -> Option<f64> {
        let pl = self.target_planet()?;
        let s = self.snap;
        let idx = pl as usize;
        let owner = s.owner[idx];
        Some(match path {
            "IsFaction" | "Owner.IsFaction" => params.strs("faction").iter().any(|f| f.eq_ignore_ascii_case(&self.owner_faction(pl))) as u8 as f64,
            "IsHumanControlled" => owner.is_some_and(|o| s.human[o as usize]) as u8 as f64,
            "TargetPoliticalControl" => (owner == Some(self.player)) as u8 as f64,
            "IsSurfaceAccessible" => (s.slots[idx].1 > 0 && s.real[idx]) as u8 as f64,
            "IsConnectedTo" => {
                let faction = params.str("faction").unwrap_or("");
                s.adjacency[idx].iter().any(|&n| s.owner[n as usize].is_some_and(|o| s.names[o as usize].eq_ignore_ascii_case(faction))) as u8 as f64
            }
            "TradeRoutes" => s.adjacency[idx].len() as f64 / s.max_neighbors,
            "ActiveTradeRoutes" => {
                s.adjacency[idx].iter().filter(|&&n| s.owner[n as usize] == Some(self.player)).count() as f64 / s.max_neighbors
            }
            "BaseIncome" => s.income[idx] / s.max_income,
            "BaseIncomeNBTP" => s.income[idx],
            "StarbaseLevelUnnormalized" => f64::from(s.starbase[idx].0),
            "StarbaseLevel" => f64::from(s.starbase[idx].0) / f64::from(s.starbase[idx].1.max(1)),
            "MaxStarbaseLevelUnnormalized" => f64::from(s.starbase[idx].1),
            "MaxStarbaseLevel" => 1.0,
            "GroundbaseLevelUnnormalized" => self.groundbase_level(pl),
            "GroundbaseLevel" => self.groundbase_level(pl) / 3.0,
            "MaxGroundbaseLevelUnnormalized" => if s.slots[idx].1 > 0 { 3.0 } else { 0.0 },
            "MaxGroundbaseLevel" => if s.slots[idx].1 > 0 { 1.0 } else { 0.0 },
            "MaxStructureSlotsUnnormalized" => f64::from(s.slots[idx].0 + s.slots[idx].1),
            "OpenStructureSlots" => {
                let used = (s.structures[idx].len() + s.queued[idx].len()) as f64;
                (f64::from(s.slots[idx].0 + s.slots[idx].1) - used).max(0.0)
            }
            "StructureCount" | "HasStructure" => {
                let types = params.strs("type");
                let complete = params.num("only_consider_complete").unwrap_or(0.0) > 0.0;
                let n = self.count_structures(&[pl], &types, complete);
                if path == "HasStructure" { (n > 0.0) as u8 as f64 } else { n }
            }
            "Type.IsType" => {
                let types = params.strs("type");
                matches_type(&types, &s.short_names[idx]) as u8 as f64 + matches_type(&types, &self.snap_planet_full(pl)) as u8 as f64
            }
            "EnemyForce.SpaceTotal" | "EnemyForce.NearbySpaceTotal" | "EnemyForce.SpaceTotalNBTT" => (self.force(pl, false) / s.max_power).min(1.0),
            "EnemyForce.HasSpaceForce" => (self.force(pl, false) > 0.0) as u8 as f64,
            "EnemyForce.GroundTotal" => (self.ground_defence(pl, owner)).min(1.0),
            "EnemyForce.HasGroundForce" => (self.ground_defence(pl, owner) > 0.0) as u8 as f64,
            "FriendlyForce.SpaceTotal" | "FriendlyForce.SpaceTotalNBTD" => (self.force(pl, true) / s.max_power).min(1.0),
            "FriendlyForce.HasSpaceForce" => (self.force(pl, true) > 0.0) as u8 as f64,
            "FriendlyForce.GroundTotal" => if owner == Some(self.player) { (self.ground_defence(pl, owner)).min(1.0) } else { 0.0 },
            "FriendlyInternalConnectivity" => self.neighbors_frac(pl, |o| o == Some(self.player)),
            "FriendlyExternalConnectivity" => self.two_hop_frac(pl, |o| o == Some(self.player)),
            "EnemyInternalConnectivity" => self.neighbors_frac(pl, |o| o.is_some_and(|x| self.is_foe(x))),
            "EnemyExternalConnectivity" => self.two_hop_frac(pl, |o| o.is_some_and(|x| self.is_foe(x))),
            "Hints.ChokePoint" | "Hints.Chokepoint" | "Hints.PriorityTarget" => 0.0,
            _ => return None,
        })
    }

    fn snap_planet_full(&self, pl: PlanetId) -> String {
        // Full object name is not kept in the snapshot; the short name covers the equations' lists.
        self.snap.short_names[pl as usize].clone()
    }

    fn ground_defence(&self, pl: PlanetId, owner: Option<PlayerId>) -> f64 {
        // No simulated ground forces: structures on enemy and neutral planets act as garrison.
        if owner == Some(self.player) {
            return 0.0;
        }
        let n = self.snap.structures[pl as usize].len() as f64;
        (n * 0.12).min(1.0) * f64::from(self.snap.slots[pl as usize].1.min(1))
    }
}

impl EquationContext for PerceptionCtx<'_> {
    fn variable(&mut self, scope: &str, path: &str, params: &Params) -> Option<f64> {
        match scope {
            "self" => self.self_var(path, params),
            "target" => self.target_var(path, params),
            _ => None,
        }
    }

    fn game(&mut self, path: &str, params: &Params) -> Option<f64> {
        let s = self.snap;
        Some(match path {
            "Age" => s.time,
            "IsCampaignGame" => 1.0,
            "Timeline" => (1.0 + (s.weeks / 8.0).floor()).min(5.0),
            "EnemyForce.SpaceTotalUnnormalized" => {
                let _ = params;
                (0..s.total_power.len()).filter(|&p| self.is_foe(p as u32)).map(|p| s.total_power[p]).sum()
            }
            "ForceVisibility" => 1.0,
            _ => return None,
        })
    }

    fn script(&mut self, name: &str, _params: &Params) -> Option<f64> {
        match name {
            "isconnectedtohuman" => {
                let target = self.target?;
                let humans: Vec<PlayerId> = (0..self.snap.human.len() as u32).filter(|&p| self.snap.human[p as usize]).collect();
                let mut connected = 0.0;
                for h in humans {
                    let mut ctx = PerceptionCtx {
                        snap: self.snap,
                        player: h,
                        target: Some(target),
                        brain: self.brain,
                        equations: self.equations,
                        rng: &mut *self.rng,
                        stats: &mut EvalStats::default(),
                    };
                    let mut stats = EvalStats::default();
                    if self.equations.evaluate("is_connected_to_me", &mut ctx, &mut stats) > 0.0 {
                        connected = 1.0;
                    }
                }
                Some(connected)
            }
            _ => None,
        }
    }

    fn random(&mut self) -> f64 {
        let mut x = *self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *self.rng = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Convenience: a `Params` with one string parameter (for tests and internal queries).
pub fn param_str(key: &str, value: &str) -> Params {
    Params(vec![(key.to_string(), ParamValue::Str(value.to_string()))])
}
