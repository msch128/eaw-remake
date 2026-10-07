//! Tactical battles: the plain-data hand-over between the galactic layer and a space battle.
//!
//! [`Sim::tactical_setup`] describes a pending battle as a [`TacticalRequest`] (planet, map, per-faction
//! groups of units, star base and structures); whoever plays the battle (the `battle-view` window or
//! [`run_headless`]) reports a [`TacticalResult`] and [`Sim::apply_tactical_result`] writes losses,
//! retreats and the winner back into the galaxy. Both types are plain data (no `Arc`, no references).

use std::sync::Arc;

use battle_sim::{Arena, Battle, BattleConfig, BattleResult, BattleSetup, Catalog, NO_TAG, SideSetup, UnitSetup};

use crate::combat::{CombatResult, Member, SideInput, Source};
use crate::sim::Sim;
use crate::state::{Event, PendingBattle};
use crate::world::{FactionId, FleetId, PlanetId, World};

/// What a [`TacticalGroup`] is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// Ships or squadrons of a fleet (`type_id` is a `UnitTypeId`).
    Unit,
    /// The planet's star base (`type_id` is a `StarbaseTypeId`).
    Starbase,
    /// Armed orbital structures (`type_id` is a `StructureTypeId`).
    Structure,
}

/// A group of identical combatants of one side.
#[derive(Debug, Clone, PartialEq)]
pub struct TacticalGroup {
    /// Kind of the group.
    pub kind: GroupKind,
    /// Type id in the [`World`] tables selected by `kind`.
    pub type_id: u32,
    /// Game object name; this is what the battle catalog is keyed by.
    pub name: String,
    /// Number of units.
    pub count: u32,
    /// Fleet the units belong to (units only).
    pub fleet: Option<FleetId>,
    /// Health fraction at the start (0..1). The galactic layer keeps no damage between battles, so this is
    /// always 1.0 today; a tactical front end may scale hull and shield by it.
    pub health: f32,
}

/// All forces of one faction in a tactical battle.
#[derive(Debug, Clone, PartialEq)]
pub struct TacticalSide {
    /// Faction id.
    pub faction: FactionId,
    /// Faction name, e.g. `Empire`.
    pub faction_name: String,
    /// True for the owner of the planet whose defenses take part.
    pub defender: bool,
    /// Difficulty multipliers of the faction (damage dealt, hull, shield; 1.0 = none); `battle_config` passes them to battle-sim.
    pub damage_mult: f32,
    /// Hull multiplier.
    pub health_mult: f32,
    /// Shield multiplier.
    pub shield_mult: f32,
    /// Groups; indices are used by [`TacticalResult`].
    pub groups: Vec<TacticalGroup>,
}

/// Everything a tactical front end needs to build a space battle.
#[derive(Debug, Clone, PartialEq)]
pub struct TacticalRequest {
    /// Pending battle id.
    pub battle_id: u32,
    /// Planet.
    pub planet: PlanetId,
    /// Planet object name, e.g. `Coruscant_WW`.
    pub planet_name: String,
    /// `Space_Tactical_Map` (.ted file name) of the planet.
    pub map: Option<String>,
    /// Star base level (1-5) of the defender, 0 without a star base.
    pub starbase_level: u32,
    /// Battle seed.
    pub seed: u64,
    /// Battle is a draw after this many seconds.
    pub max_seconds: f32,
    /// Sides in battle order: attackers first, the side with structures (the defender) last. Side `i` is
    /// battle-sim side `i`.
    pub sides: Vec<TacticalSide>,
}

/// Outcome of a tactical battle, shaped like [`TacticalRequest::sides`] (`[side][group]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TacticalResult {
    /// Winning side index; `None` for a draw or mutual destruction.
    pub winner: Option<usize>,
    /// Units still alive (retreated ones included) per side and group.
    pub survived: Vec<Vec<u32>>,
    /// Units that left the battle by retreating per side and group (subset of `survived`).
    pub retreated: Vec<Vec<u32>>,
    /// Battle time in seconds.
    pub duration: f32,
}

/// Why a tactical result could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TacticalError {
    /// No pending battle has this id.
    UnknownBattle,
    /// The result does not match the request (side or group counts differ).
    ShapeMismatch,
}

impl std::fmt::Display for TacticalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TacticalError::UnknownBattle => f.write_str("unknown battle id"),
            TacticalError::ShapeMismatch => f.write_str("tactical result does not match the request"),
        }
    }
}

impl std::error::Error for TacticalError {}

impl TacticalRequest {
    /// Orders `sides` for battle (defender last) and drops garrison squadrons, which the star base definition launches itself.
    pub(crate) fn from_sides(world: &World, battle: &PendingBattle, sides: &[SideInput], seed: u64, difficulty: &[crate::state::Difficulty]) -> Self {
        let world_diff = |f: FactionId| difficulty.get(f as usize).copied().unwrap_or_default();
        let has_structs = |s: &SideInput| s.members.iter().any(|m| matches!(m.source, Source::Starbase(_) | Source::Structure(_)));
        let mut order: Vec<usize> = (0..sides.len()).collect();
        if sides.len() == 2 && has_structs(&sides[0]) && !has_structs(&sides[1]) {
            order.swap(0, 1);
        }
        let planet = &world.planets[battle.planet as usize];
        let mut starbase_level = 0;
        let out = order
            .into_iter()
            .map(|si| {
                let side = &sides[si];
                let groups = side
                    .members
                    .iter()
                    .filter_map(|m| {
                        let (kind, type_id, name) = match m.source {
                            Source::Unit(u) => (GroupKind::Unit, u, world.units[u as usize].name.clone()),
                            Source::Starbase(s) => {
                                starbase_level = world.starbases[s as usize].level.max(0) as u32;
                                (GroupKind::Starbase, s, world.starbases[s as usize].name.clone())
                            }
                            Source::Structure(s) => (GroupKind::Structure, s, world.structures[s as usize].name.clone()),
                            Source::Garrison(_) => return None,
                        };
                        Some(TacticalGroup { kind, type_id, name, count: m.count, fleet: m.fleet, health: 1.0 })
                    })
                    .collect();
                TacticalSide {
                    faction: side.faction,
                    faction_name: world.factions[side.faction as usize].name.clone(),
                    defender: battle.defender == Some(side.faction),
                    damage_mult: world_diff(side.faction).damage,
                    health_mult: world_diff(side.faction).health,
                    shield_mult: world_diff(side.faction).shield,
                    groups,
                }
            })
            .collect();
        TacticalRequest {
            battle_id: battle.id,
            planet: battle.planet,
            planet_name: planet.name.clone(),
            map: planet.space_map.clone(),
            starbase_level,
            seed,
            max_seconds: world.config.tactical_max_seconds,
            sides: out,
        }
    }

    /// Battle constants for this request (seed, arena, time limit); everything else is `BattleConfig::default()`.
    pub fn battle_config(&self, arena: Option<Arc<Arena>>) -> BattleConfig {
        let half_size = arena.as_ref().map_or(BattleConfig::default().half_size, |a| a.half_size());
        let side_mults = self.sides.iter().map(|s| battle_sim::SideMult { damage: s.damage_mult, health: s.health_mult, shield: s.shield_mult }).collect();
        BattleConfig { seed: self.seed, max_time: self.max_seconds, half_size, arena, side_mults, ..BattleConfig::default() }
    }

    /// Builds the `battle-sim` setup. Fleet groups carry their group index as the tag; side `i` of the request is
    /// battle side `i`. Returns `None` if a unit has no definition in `catalog`. `ai` selects the sides the built-in
    /// AI commands (a front end passes `false` for the human side).
    pub fn battle_setup(&self, catalog: &Arc<Catalog>, config: BattleConfig, ai: impl Fn(usize) -> bool) -> Option<BattleSetup> {
        let mut sides = Vec::new();
        for (si, side) in self.sides.iter().enumerate() {
            let mut units = Vec::new();
            let mut structures = Vec::new();
            for (gi, g) in side.groups.iter().enumerate() {
                let ship = catalog.ship_id(&g.name)?;
                match g.kind {
                    GroupKind::Unit => units.push(UnitSetup { ship, count: g.count, tag: gi as u32 }),
                    GroupKind::Starbase => structures.push(ship),
                    GroupKind::Structure => structures.extend(std::iter::repeat_n(ship, g.count as usize)),
                }
            }
            sides.push(SideSetup { name: side.faction_name.clone(), units, structures, ai: ai(si) });
        }
        Some(BattleSetup { catalog: catalog.clone(), sides, config })
    }

    /// Converts a finished `battle-sim` result into a [`TacticalResult`]. Returns `None` if a unit has no definition in `catalog`.
    pub fn result_from(&self, catalog: &Catalog, result: &BattleResult) -> Option<TacticalResult> {
        let mut survived = Vec::new();
        let mut retreated = Vec::new();
        for (si, side) in self.sides.iter().enumerate() {
            let (mut s_row, mut r_row) = (Vec::new(), Vec::new());
            for (gi, g) in side.groups.iter().enumerate() {
                let ship = catalog.ship_id(&g.name)?;
                let tag = if g.kind == GroupKind::Unit { gi as u32 } else { NO_TAG };
                let (mut s, mut r) = (0, 0);
                for gr in result.groups.iter().filter(|x| x.side as usize == si && x.tag == tag && x.ship == ship) {
                    s += gr.survived;
                    r += gr.retreated;
                }
                s = s.min(if g.kind == GroupKind::Starbase { 1 } else { g.count });
                s_row.push(s);
                r_row.push(r.min(s));
            }
            survived.push(s_row);
            retreated.push(r_row);
        }
        Some(TacticalResult { winner: result.winner.map(|w| w as usize), survived, retreated, duration: result.duration })
    }

    /// Combatants of the request, as the galaxy layer sees them.
    fn side_inputs(&self) -> Vec<SideInput> {
        self.sides
            .iter()
            .map(|s| SideInput {
                faction: s.faction,
                members: s
                    .groups
                    .iter()
                    .map(|g| Member {
                        source: match g.kind {
                            GroupKind::Unit => Source::Unit(g.type_id),
                            GroupKind::Starbase => Source::Starbase(g.type_id),
                            GroupKind::Structure => Source::Structure(g.type_id),
                        },
                        fleet: g.fleet,
                        count: g.count,
                    })
                    .collect(),
            })
            .collect()
    }

    /// The request's sides and a result expressed as a [`CombatResult`].
    pub(crate) fn combat_result(&self, result: &TacticalResult) -> (Vec<SideInput>, CombatResult) {
        let sides = self.side_inputs();
        let mut survivors = vec![Vec::new(); sides.len()];
        let mut retreated: Vec<FleetId> = Vec::new();
        for (si, side) in sides.iter().enumerate() {
            let mut fleet_gone: Vec<(FleetId, bool)> = Vec::new();
            for (gi, m) in side.members.iter().enumerate() {
                let left = result.survived.get(si).and_then(|r| r.get(gi)).copied().unwrap_or(0).min(m.count);
                if left == 0 {
                    continue;
                }
                survivors[si].push(Member { count: left, ..*m });
                if let Some(f) = m.fleet {
                    let gone = result.retreated.get(si).and_then(|r| r.get(gi)).copied().unwrap_or(0) >= left;
                    match fleet_gone.iter_mut().find(|x| x.0 == f) {
                        Some(x) => x.1 &= gone,
                        None => fleet_gone.push((f, gone)),
                    }
                }
            }
            retreated.extend(fleet_gone.into_iter().filter(|x| x.1).map(|x| x.0));
        }
        let combat = CombatResult { survivors, winner: result.winner, rounds: (result.duration / 4.0) as u32, retreated };
        (sides, combat)
    }

    fn check_shape(&self, result: &TacticalResult) -> bool {
        let ok = |rows: &Vec<Vec<u32>>| rows.len() == self.sides.len() && rows.iter().zip(&self.sides).all(|(r, s)| r.len() == s.groups.len());
        ok(&result.survived) && ok(&result.retreated) && result.winner.is_none_or(|w| w < self.sides.len())
    }
}

/// Plays the battle of `request` to the end without rendering (all sides AI controlled). Returns `None` if a
/// unit has no definition in `catalog`.
pub fn run_headless(request: &TacticalRequest, catalog: &Arc<Catalog>, arena: Option<Arc<Arena>>) -> Option<TacticalResult> {
    let config = BattleConfig { record_events: false, unit_cap: i32::MAX / 2, ..request.battle_config(arena) };
    let max_steps = (request.max_seconds / config.dt) as u64 + 1;
    let mut battle = Battle::new(request.battle_setup(catalog, config, |_| true)?);
    battle.run_to_end(max_steps);
    request.result_from(catalog, &battle.result())
}

impl Sim {
    /// Describes pending battle `battle_id` for a tactical front end and freezes it: the request is kept so
    /// that [`Sim::apply_tactical_result`] applies the result to exactly these forces. Returns `None` for an
    /// unknown id. Calling it again returns the same request.
    pub fn tactical_setup(&mut self, battle_id: u32) -> Option<TacticalRequest> {
        if let Some(r) = self.tactical_requests.get(&battle_id) {
            return Some(r.clone());
        }
        let battle = self.galaxy.battles.iter().find(|b| b.id == battle_id)?.clone();
        let sides = self.battle_sides(&battle);
        let seed = self.battle_rng(battle_id).next_u64();
        let request = TacticalRequest::from_sides(&self.world, &battle, &sides, seed, &self.galaxy.factions.iter().map(|f| f.difficulty).collect::<Vec<_>>());
        self.tactical_requests.insert(battle_id, request.clone());
        Some(request)
    }

    /// Applies the result of the tactical battle set up with [`Sim::tactical_setup`]: removes destroyed
    /// units, the star base and structures, lets the beaten side's surviving fleets fall back (a draw makes
    /// every attacker retreat) and returns the events (`UnitsDestroyed`, `BattleWon/Lost`, `BattleResolved`).
    /// A planet changes hands afterwards through the normal capture rule (RULES.md section 6): the winner alone in
    /// orbit and no star base left.
    pub fn apply_tactical_result(&mut self, battle_id: u32, result: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        let pos = self.galaxy.battles.iter().position(|b| b.id == battle_id).ok_or(TacticalError::UnknownBattle)?;
        let request = self.tactical_setup(battle_id).ok_or(TacticalError::UnknownBattle)?;
        if !request.check_shape(result) {
            return Err(TacticalError::ShapeMismatch);
        }
        let battle = self.galaxy.battles.remove(pos);
        self.tactical_requests.remove(&battle_id);
        let (sides, combat) = request.combat_result(result);
        Ok(self.apply_combat(&battle, &sides, combat))
    }

    /// Abandons a tactical setup without a result (the battle stays pending, e.g. to auto-resolve it instead).
    pub fn cancel_tactical(&mut self, battle_id: u32) {
        self.tactical_requests.remove(&battle_id);
    }
}
