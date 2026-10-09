//! Strategic layer on top of the Remake AI: influence maps over the route graph and utility based target scoring.
//! Design and tuning notes: `STRATEGY.md`.
//!
//! The Remake goal functions give every `(goal, target)` a desire from the original equations; the layer multiplies it by
//! [`Strategy::factor`], a value between 0.2 and 3 that expresses what the 2006 equations cannot see: how far the
//! target is, how much enemy strength can reach it, whether the enemy's main stack is elsewhere (flank), whether it cuts
//! off a pocket, how much the faction already owns and whether it should consolidate first. Everything is computed from
//! the [`Snapshot`], is deterministic and cheap (a few thousand operations per refresh).

use std::collections::HashMap;

use crate::perception::Snapshot;
use crate::sim::{PlanetId, PlayerId};

/// Character of a faction's AI; 1.0 is the default for every value.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Personality {
    /// Weight of the attack-friendly terms (value, flank, pocket) and lowers the force margin asked before attacking.
    pub aggression: f64,
    /// Strength of the drive to take neutral planets while the faction is small.
    pub expansion: f64,
    /// Weight of the risk term and raises the force margin.
    pub caution: f64,
}

impl Default for Personality {
    fn default() -> Self {
        Self { aggression: 1.0, expansion: 1.0, caution: 1.0 }
    }
}

/// How a faction opens a match (estimates): the first `StrategyConfig::opening_seconds` bend its personality, fading back to the base values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Opening {
    /// No bias.
    #[default]
    Balanced,
    /// Grab neutral planets first: more expansion, less aggression.
    Expand,
    /// Build up and defend: more caution, less aggression and expansion.
    Turtle,
    /// Strike early at weak targets: more aggression, less caution.
    Raid,
}

impl Opening {
    /// Multipliers (aggression, expansion, caution) at full strength.
    pub fn multipliers(self) -> (f64, f64, f64) {
        match self {
            Opening::Balanced => (1.0, 1.0, 1.0),
            Opening::Expand => (0.8, 1.45, 1.0),
            Opening::Turtle => (0.75, 0.85, 1.35),
            Opening::Raid => (1.35, 0.9, 0.8),
        }
    }
}

/// Difficulty as seen by the AI (the economic multipliers live in `galaxy-sim`).
#[derive(Debug, Clone, PartialEq)]
pub struct AiTier {
    /// Display name.
    pub name: String,
    /// Multiplier on the force advantage the AI asks for (`Galactic_AI_Contrast_Multiplier`; below 1 = bolder).
    pub contrast: f64,
    /// Extra waiting between goal evaluations as a fraction of the evaluation interval (`Galactic_AI_Goal_Cycle_Sleep_Duration`; estimate: the file does not give the unit).
    pub goal_sleep: f64,
    /// The AI sees the whole map (no fog), an optional cheat.
    pub full_map_knowledge: bool,
}

impl Default for AiTier {
    fn default() -> Self {
        Self { name: "Normal".into(), contrast: 1.0, goal_sleep: 0.0, full_map_knowledge: false }
    }
}

/// Configuration of the strategic layer.
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    /// Switch; off keeps the pure Remake scoring.
    pub enabled: bool,
    /// How many hops beyond own planets and fleets a faction sees (sensor range; estimate).
    pub vision_hops: u8,
    /// Personality of every faction that has no entry in `personalities`.
    pub default_personality: Personality,
    /// Personalities by player.
    pub personalities: HashMap<PlayerId, Personality>,
    /// Opening of a player (default `Balanced`).
    pub openings: HashMap<PlayerId, Opening>,
    /// Game seconds an opening lasts (it fades out linearly; estimate: 12 weeks).
    pub opening_seconds: f64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self { enabled: true, vision_hops: 1, default_personality: Personality::default(), personalities: HashMap::new(), openings: HashMap::new(), opening_seconds: 720.0 }
    }
}

/// What a player is currently doing strategically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Normal: attack and expand where the utility is good.
    Expand,
    /// Recently lost planets or badly outgunned by a neighbour: only safe attacks, defend first.
    Consolidate,
}

const HOPS: usize = 3;
const DECAY: f64 = 0.5;
/// Strength of the pull towards the average size on expansion targets (estimate): a faction at 1.6x the average planets sees neutral and pirate planets at exp(-0.6 x LEADER_BRAKE) of their value.
const LEADER_BRAKE: f64 = 0.6;

/// Shared strategic state of all AI players.
#[derive(Debug, Default)]
pub struct Strategy {
    /// Configuration.
    pub config: StrategyConfig,
    /// Tier per player (default tier for players without one).
    pub tiers: HashMap<PlayerId, AiTier>,
    near: Vec<Vec<(u32, u8)>>,
    /// `influence[player][planet]`: strength that can reach the planet, decayed by hops.
    influence: Vec<Vec<f64>>,
    /// Hops from the nearest owned planet.
    dist: Vec<Vec<u8>>,
    visible: Vec<Vec<bool>>,
    /// Last seen hostile strength per player and planet (`NAN` = never seen).
    memory: Vec<Vec<f64>>,
    prior: Vec<f64>,
    phase: Vec<Phase>,
    losing_until: Vec<f64>,
    owned_prev: Vec<(f64, u32)>,
    total: Vec<f64>,
    /// Planets owned relative to the average of the established factions (1.0 = average).
    share: Vec<f64>,
}

impl Strategy {
    /// New layer with a configuration.
    pub fn new(config: StrategyConfig) -> Self {
        Self { config, ..Self::default() }
    }

    /// Personality of a player.
    pub fn personality(&self, p: PlayerId) -> Personality {
        self.config.personalities.get(&p).copied().unwrap_or(self.config.default_personality)
    }

    /// Personality of a player at game time `time`: the base values bent by the opening, which fades out over `opening_seconds`.
    pub fn personality_at(&self, p: PlayerId, time: f64) -> Personality {
        let base = self.personality(p);
        let opening = self.config.openings.get(&p).copied().unwrap_or_default();
        let k = (1.0 - time / self.config.opening_seconds.max(1.0)).clamp(0.0, 1.0);
        let (a, e, c) = opening.multipliers();
        let mix = |m: f64| 1.0 + (m - 1.0) * k;
        Personality { aggression: base.aggression * mix(a), expansion: base.expansion * mix(e), caution: base.caution * mix(c) }
    }

    /// Tier of a player.
    pub fn tier(&self, p: PlayerId) -> AiTier {
        self.tiers.get(&p).cloned().unwrap_or_default()
    }

    /// Current phase of a player.
    pub fn phase(&self, p: PlayerId) -> Phase {
        self.phase.get(p as usize).copied().unwrap_or(Phase::Expand)
    }

    /// True if the player knows what is going on at the planet (own planet, own fleet, in sensor range, or a full-map tier).
    pub fn is_visible(&self, p: PlayerId, pl: PlanetId) -> bool {
        self.visible.get(p as usize).and_then(|v| v.get(pl as usize)).copied().unwrap_or(false)
    }

    /// Influence of a player's strength at a planet.
    pub fn influence(&self, p: PlayerId, pl: PlanetId) -> f64 {
        self.influence.get(p as usize).and_then(|v| v.get(pl as usize)).copied().unwrap_or(0.0)
    }

    /// Strength of everything hostile to `me` that the player believes stands at the planet: the real value where visible,
    /// the last seen value (or the average of what it has seen) otherwise.
    pub fn believed_hostile_strength(&self, snap: &Snapshot, me: PlayerId, pl: PlanetId) -> f64 {
        if self.is_visible(me, pl) {
            hostile_strength_at(snap, me, pl)
        } else {
            match self.memory.get(me as usize).and_then(|m| m.get(pl as usize)) {
                Some(v) if !v.is_nan() => *v,
                _ => self.prior.get(me as usize).copied().unwrap_or(0.0),
            }
        }
    }

    /// Recomputes influence, visibility, memory and phases from a snapshot. Call once per snapshot.
    pub fn refresh(&mut self, snap: &Snapshot) {
        let n = snap.owner.len();
        let players = snap.names.len();
        if self.near.len() != n {
            self.near = (0..n).map(|s| near_list(snap, s as u32)).collect();
            self.memory = vec![vec![f64::NAN; n]; players];
            self.phase = vec![Phase::Expand; players];
            self.prior = vec![0.0; players];
            self.losing_until = vec![0.0; players];
            self.owned_prev = vec![(snap.time, 0); players];
        }
        // Strength per player and planet (fleets + fixed defenses of the owner).
        let mut strength = vec![vec![0.0f64; n]; players];
        self.total = vec![0.0; players];
        for (pl, row) in snap.strength.iter().enumerate().take(n) {
            for &(p, s) in row {
                strength[p as usize][pl] += s;
            }
            if let Some(o) = snap.owner[pl] {
                strength[o as usize][pl] += snap.static_defense[pl];
            }
        }
        // Influence maps.
        self.influence = vec![vec![0.0; n]; players];
        for (p, row) in strength.iter().enumerate() {
            for (q, &s) in row.iter().enumerate().take(n) {
                if s <= 0.0 {
                    continue;
                }
                self.total[p] += s;
                for &(t, h) in &self.near[q] {
                    self.influence[p][t as usize] += s * DECAY.powi(i32::from(h));
                }
            }
        }
        // Hops from owned planets.
        self.dist = (0..players).map(|p| multi_source_hops(snap, &snap.owned[p])).collect();
        // Visibility and memory.
        self.visible = vec![vec![false; n]; players];
        for p in 0..players {
            let full = self.tiers.get(&(p as u32)).is_some_and(|t| t.full_map_knowledge);
            if full {
                self.visible[p] = vec![true; n];
            } else {
                let mut seeds: Vec<usize> = snap.owned[p].iter().map(|&x| x as usize).collect();
                seeds.extend((0..n).filter(|&pl| snap.power[pl].iter().any(|e| e.0 as usize == p)));
                for s in seeds {
                    for &(t, h) in &self.near[s] {
                        if h <= self.config.vision_hops {
                            self.visible[p][t as usize] = true;
                        }
                    }
                }
            }
            let (mut sum, mut cnt) = (0.0, 0);
            for pl in 0..n {
                if self.visible[p][pl] {
                    let h = hostile_strength_at(snap, p as u32, pl as u32);
                    self.memory[p][pl] = h;
                    if h > 0.0 {
                        sum += h;
                        cnt += 1;
                    }
                }
            }
            // What an unseen planet is assumed to hold: the average of the occupied ones the faction has seen.
            self.prior[p] = if cnt > 0 { sum / f64::from(cnt) } else { 0.0 };
        }
        // Relative size: planets owned against the average of the established factions (4+ planets, no pirates).
        let established: Vec<f64> = (0..players).filter(|&p| snap.owned[p].len() >= 4 && !snap.names[p].eq_ignore_ascii_case("pirates")).map(|p| snap.owned[p].len() as f64).collect();
        let average = if established.len() >= 2 { established.iter().sum::<f64>() / established.len() as f64 } else { 0.0 };
        self.share = (0..players).map(|p| if average > 0.0 { snap.owned[p].len() as f64 / average } else { 1.0 }).collect();
        // Phases.
        for p in 0..players {
            let owned = snap.owned[p].len() as u32;
            let (t0, c0) = self.owned_prev[p];
            if snap.time - t0 >= 120.0 {
                if c0 > owned {
                    self.losing_until[p] = snap.time + 240.0;
                }
                self.owned_prev[p] = (snap.time, owned);
            }
            // Strongest hostile faction owning a planet next to ours.
            let mut neighbour_max = 0.0f64;
            for &pl in &snap.owned[p] {
                for &nb in &snap.adjacency[pl as usize] {
                    if let Some(o) = snap.owner[nb as usize]
                        && snap.enemy[p][o as usize]
                    {
                        neighbour_max = neighbour_max.max(self.total[o as usize]);
                    }
                }
            }
            let outgunned = neighbour_max > 0.0 && self.total[p] < 0.35 * neighbour_max;
            self.phase[p] = if snap.time < self.losing_until[p] || outgunned { Phase::Consolidate } else { Phase::Expand };
        }
    }

    /// Value of a planet in 0..~1.2: income, build capability, fortification and slots used.
    fn value(snap: &Snapshot, pl: usize) -> f64 {
        let income = snap.income[pl] / snap.max_income.max(1.0);
        let (orbital, ground) = snap.slots[pl];
        let used = snap.structures[pl].len() as f64 / f64::from((orbital + ground).max(1));
        let yard = snap.structures[pl].iter().any(|s| s.to_ascii_lowercase().contains("shipyard")) || snap.starbase[pl].0 >= 2;
        let base = f64::from(snap.starbase[pl].0) / f64::from(snap.starbase[pl].1.max(1));
        0.5 * income + 0.2 * used.min(1.0) + 0.2 * base + if yard { 0.2 } else { 0.0 }
    }

    /// Utility factor for `(player, target)`, multiplied onto the Remake desire. `offensive` marks goals that go for
    /// enemy or neutral planets; others (own planets) get the defend-the-valuable term.
    pub fn factor(&self, snap: &Snapshot, me: PlayerId, target: PlanetId, offensive: bool) -> f64 {
        if !self.config.enabled || self.influence.is_empty() {
            return 1.0;
        }
        let pers = self.personality_at(me, snap.time);
        let t = target as usize;
        let owner = snap.owner[t];
        let value = Self::value(snap, t);
        let mine = self.influence[me as usize][t];
        let consolidating = self.phase(me) == Phase::Consolidate;
        if owner == Some(me) {
            // Defend what is valuable and threatened: hostile influence beyond what we can bring.
            let threat: f64 = (0..self.influence.len()).filter(|&o| o as u32 != me && snap.enemy[me as usize][o]).map(|o| self.influence[o][t]).sum();
            let need = ((threat - mine).max(0.0) / (mine + threat + 1.0)).clamp(0.0, 1.0);
            return (1.0 + 1.5 * value * need * if consolidating { 1.5 } else { 1.0 }).clamp(0.2, 3.0);
        }
        if !offensive {
            return 1.0;
        }
        let dist = f64::from(self.dist[me as usize][t].min(8));
        let local = self.believed_hostile_strength(snap, me, target);
        let visible = self.is_visible(me, target);
        let near_hostile: f64 = if visible {
            (0..self.influence.len()).filter(|&o| o as u32 != me && snap.enemy[me as usize][o]).map(|o| self.influence[o][t]).sum::<f64>() - local
        } else {
            0.5 * self.prior.get(me as usize).copied().unwrap_or(0.0)
        };
        let risk = ((local + 0.5 * near_hostile.max(0.0)) / (mine + 1e-6 + 0.05 * self.total[me as usize])).clamp(0.0, 3.0);
        let mut u = 0.9 * value * pers.aggression - 0.7 * pers.caution * risk.min(2.5) - 0.25 * (dist - 1.0).max(0.0);
        if dist <= 1.0 {
            u += 0.2; // frontier: cheap to reinforce and supply
        }
        // Measured expansion drive: the leader slows down on neutral and pirate planets, small factions catch up (estimate).
        if owner.is_none_or(|o| snap.names[o as usize].eq_ignore_ascii_case("pirates")) {
            let share = self.share.get(me as usize).copied().unwrap_or(1.0);
            u += LEADER_BRAKE * pers.expansion * (1.0 - share).clamp(-1.0, 0.7);
        }
        match owner {
            None => {
                // Measured expansion drive: strong while small, fades at about a third of the map.
                let unsat = (1.0 - snap.owned[me as usize].len() as f64 / (0.35 * f64::from(snap.real_count.max(1)))).clamp(0.0, 1.0);
                u += 0.5 * pers.expansion * unsat;
            }
            Some(o) => {
                // Flank: weak spot while the owner's main stack is far away.
                let (main_at, main_s) = (0..snap.owner.len())
                    .map(|q| (q, strength_of(snap, o, q)))
                    .fold((t, 0.0), |a, b| if b.1 > a.1 { b } else { a });
                if main_s > 0.0 && local < 0.5 * main_s {
                    let hops = self.near[t].iter().find(|(q, _)| *q as usize == main_at).map_or(HOPS as f64 + 1.0, |x| f64::from(x.1));
                    if hops >= 2.0 {
                        u += 0.6 * pers.aggression * (hops / 4.0).min(1.0);
                    }
                }
                // Pocket: taking this planet isolates owner planets that have no other owner neighbour.
                let isolated = snap.adjacency[t]
                    .iter()
                    .filter(|&&nb| snap.owner[nb as usize] == Some(o) && snap.adjacency[nb as usize].iter().all(|&x| x as usize == t || snap.owner[x as usize] != Some(o)))
                    .count();
                u += 0.4 * pers.aggression * (isolated as f64 / 2.0).min(1.0);
            }
        }
        if consolidating && !(dist <= 1.0 && risk < 0.5) {
            u -= 0.7;
        }
        u.exp().clamp(0.2, 3.0)
    }
}

fn strength_of(snap: &Snapshot, p: PlayerId, pl: usize) -> f64 {
    snap.strength[pl].iter().filter(|e| e.0 == p).map(|e| e.1).sum::<f64>() + if snap.owner[pl] == Some(p) { snap.static_defense[pl] } else { 0.0 }
}

/// Strength of everything hostile to `me` at a planet.
pub fn hostile_strength_at(snap: &Snapshot, me: PlayerId, pl: PlanetId) -> f64 {
    let pl = pl as usize;
    let fleets: f64 = snap.strength[pl].iter().filter(|e| snap.enemy[me as usize][e.0 as usize]).map(|e| e.1).sum();
    let fixed = snap.owner[pl].filter(|o| snap.enemy[me as usize][*o as usize]).map_or(0.0, |_| snap.static_defense[pl]);
    fleets + fixed
}

fn near_list(snap: &Snapshot, from: u32) -> Vec<(u32, u8)> {
    let mut seen = vec![false; snap.adjacency.len()];
    seen[from as usize] = true;
    let mut out = vec![(from, 0u8)];
    let mut frontier = vec![from];
    for h in 1..=HOPS as u8 {
        let mut next = Vec::new();
        for &q in &frontier {
            for &nb in &snap.adjacency[q as usize] {
                if !seen[nb as usize] {
                    seen[nb as usize] = true;
                    out.push((nb, h));
                    next.push(nb);
                }
            }
        }
        frontier = next;
    }
    out
}

fn multi_source_hops(snap: &Snapshot, sources: &[PlanetId]) -> Vec<u8> {
    let mut d = vec![u8::MAX; snap.adjacency.len()];
    let mut q = std::collections::VecDeque::new();
    for &s in sources {
        d[s as usize] = 0;
        q.push_back(s);
    }
    while let Some(x) = q.pop_front() {
        let h = d[x as usize];
        for &nb in &snap.adjacency[x as usize] {
            if d[nb as usize] == u8::MAX {
                d[nb as usize] = h.saturating_add(1);
                q.push_back(nb);
            }
        }
    }
    d
}

impl Strategy {
    /// Total strength of everything a player owns (fleets and fixed defenses) as of the last refresh.
    pub fn total_strength(&self, p: PlayerId) -> f64 {
        self.total.get(p as usize).copied().unwrap_or(0.0)
    }
}

/// What the strategic layer remembers between turns (the rest is recomputed from the world every few seconds).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StrategyMemory {
    /// Last seen hostile strength per player and planet (`-1` = never seen; JSON has no NaN).
    pub memory: Vec<Vec<f64>>,
    /// Prior belief per planet.
    pub prior: Vec<f64>,
    /// Phase per player (1 = consolidate).
    pub phase: Vec<u8>,
    /// Consolidation lasts until this game time.
    pub losing_until: Vec<f64>,
    /// Planets owned at the last check: `(time, count)`.
    pub owned_prev: Vec<(f64, u32)>,
}

impl Strategy {
    /// Copies the memory.
    pub fn snapshot(&self) -> StrategyMemory {
        StrategyMemory {
            memory: self.memory.iter().map(|row| row.iter().map(|&v| if v.is_nan() { -1.0 } else { v }).collect()).collect(),
            prior: self.prior.clone(),
            phase: self.phase.iter().map(|p| u8::from(*p == Phase::Consolidate)).collect(),
            losing_until: self.losing_until.clone(),
            owned_prev: self.owned_prev.clone(),
        }
    }

    /// Puts the memory back; the influence maps are rebuilt by the next refresh.
    pub fn restore(&mut self, m: &StrategyMemory) {
        self.memory = m.memory.iter().map(|row| row.iter().map(|&v| if v < 0.0 { f64::NAN } else { v }).collect()).collect();
        self.prior = m.prior.clone();
        self.phase = m.phase.iter().map(|&p| if p == 1 { Phase::Consolidate } else { Phase::Expand }).collect();
        self.losing_until = m.losing_until.clone();
        self.owned_prev = m.owned_prev.clone();
    }
}
