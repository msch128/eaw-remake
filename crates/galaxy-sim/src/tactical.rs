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
use crate::state::{Event, FleetLocation, Galaxy, PendingBattle, ShipId, UnitHealth};
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
    /// Hull and shield of every ship of the group (units: `count` entries in the order battle-sim spawns them; the star base: one entry).
    /// Empty = everything undamaged.
    pub unit_health: Vec<UnitHealth>,
}

/// All forces of one faction in a tactical battle.
#[derive(Debug, Clone, PartialEq)]
pub struct TacticalSide {
    /// Faction id.
    pub faction: FactionId,
    /// Tech level of the faction as an index (0 to 4; the galaxy counter starts at 1, `Tech_0` is the level at the start): selects the `..._Spawned_Units_Tech_N` entries its carriers launch.
    pub tech: u8,
    /// Faction name, e.g. `Empire`.
    pub faction_name: String,
    /// True for the owner of the planet whose defenses take part.
    pub defender: bool,
    /// Difficulty multipliers of the faction (damage dealt, hull, shield; 1.0 = none); `battle_config` passes them to battle-sim.
    pub damage_mult: f32,
    /// Hull multiplier.
    pub health_mult: f32,
    /// May the battle AI of this side order a retreat (`Allowed_To_Retreat_Space`: holds more than one planet, not Pirates).
    pub may_retreat: bool,
    /// Shield multiplier.
    pub shield_mult: f32,
    /// Groups; indices are used by [`TacticalResult`].
    pub groups: Vec<TacticalGroup>,
    /// Reinforcement pool: units of the fleets at the planet that did not fit under the space population cap (`Space_Tactical_Unit_Cap` of the faction).
    /// A front end may call them in during the battle with `battle_sim::Command::Reinforce` (they arrive after the reinforcement delay and only if
    /// the cap allows); units never called stay in their fleets.
    pub reserve: Vec<TacticalGroup>,
    /// Population cap of this side in the battle.
    pub unit_cap: i32,
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
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TacticalResult {
    /// Winning side index; `None` for a draw or mutual destruction.
    pub winner: Option<usize>,
    /// Units still alive (retreated ones included) per side and group.
    pub survived: Vec<Vec<u32>>,
    /// Units that left the battle by retreating per side and group (subset of `survived`).
    pub retreated: Vec<Vec<u32>>,
    /// Battle time in seconds.
    pub duration: f32,
    /// Hull and shield of the survivors per side and group (healthiest first); empty = report nothing (everything counts as undamaged).
    pub health: Vec<Vec<Vec<UnitHealth>>>,
    /// Per side and reserve group: how many units were called in (empty = none were).
    pub reserve_called: Vec<Vec<u32>>,
    /// Per side and reserve group: how many of the called units survived (only meaningful where `reserve_called` is above 0).
    pub reserve_survived: Vec<Vec<u32>>,
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

/// Hyperspace slots (`Galaxy::hyperspace`), per fleet: a fleet with a pathfinder ship sends only that unit into the battle at the start and every other
/// unit of the fleet goes to the reserve (a stack of the pathfinder's type keeps one unit in the battle). A fleet with an empty pathfinder slot deploys
/// completely (the population cap still applies afterwards).
fn apply_hyperspace_slots(galaxy: &Galaxy, groups: Vec<TacticalGroup>) -> (Vec<TacticalGroup>, Vec<TacticalGroup>) {
    let has_slot = |g: &TacticalGroup| g.kind == GroupKind::Unit && g.fleet.is_some_and(|f| galaxy.hyperspace_unit(f).is_some());
    if !groups.iter().any(has_slot) {
        return (groups, Vec::new());
    }
    let (mut deployed, mut reserve) = (Vec::new(), Vec::new());
    for mut g in groups {
        if !has_slot(&g) {
            deployed.push(g);
        } else if g.fleet.is_some_and(|f| galaxy.hyperspace_unit(f) == Some(g.type_id)) {
            if g.count > 1 {
                let mut rest = g.clone();
                rest.count = g.count - 1;
                rest.unit_health = if g.unit_health.is_empty() { Vec::new() } else { g.unit_health.split_off(1.min(g.unit_health.len())) };
                reserve.push(rest);
                g.count = 1;
            }
            deployed.push(g);
        } else {
            reserve.push(g);
        }
    }
    (deployed, reserve)
}

/// Units that fit under the population cap are deployed (in fleet order, a group is split where the cap falls), the rest goes to the reserve.
fn split_reserve(world: &World, groups: Vec<TacticalGroup>, cap: i32) -> (Vec<TacticalGroup>, Vec<TacticalGroup>) {
    let (mut deployed, mut reserve, mut used) = (Vec::new(), Vec::new(), 0i32);
    for mut g in groups {
        if g.kind != GroupKind::Unit {
            deployed.push(g);
            continue;
        }
        let pop = world.units[g.type_id as usize].space_population.max(0);
        let fit = if pop == 0 { g.count } else { ((cap - used).max(0) / pop) as u32 }.min(g.count);
        used += pop * fit as i32;
        if fit < g.count {
            let mut rest = g.clone();
            rest.count = g.count - fit;
            rest.unit_health = if g.unit_health.is_empty() { Vec::new() } else { g.unit_health.split_off((fit as usize).min(g.unit_health.len())) };
            reserve.push(rest);
            g.count = fit;
        }
        if g.count > 0 {
            deployed.push(g);
        }
    }
    (deployed, reserve)
}

impl TacticalRequest {
    /// Orders `sides` for battle (defender last) and drops garrison squadrons, which the star base definition launches itself.
    pub(crate) fn from_sides(world: &World, battle: &PendingBattle, sides: &[SideInput], seed: u64, galaxy: &Galaxy) -> Self {
        let world_diff = |f: FactionId| galaxy.factions.get(f as usize).map(|s| s.difficulty).unwrap_or_default();
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
                        let unit_health = match (kind, m.fleet) {
                            (GroupKind::Unit, Some(fleet)) => {
                                let ships: Vec<_> = galaxy.ships.get(&fleet).into_iter().flatten().filter(|(_, t)| *t == type_id).take(m.count as usize).collect();
                                let list: Vec<UnitHealth> = ships.iter().map(|(s, _)| galaxy.damage.get(s).copied().unwrap_or(UnitHealth::FULL)).collect();
                                if list.iter().all(UnitHealth::is_full) { Vec::new() } else { list }
                            }
                            (GroupKind::Starbase, _) => galaxy.planets[battle.planet as usize].base_health.filter(|h| !h.is_full()).into_iter().collect(),
                            _ => Vec::new(),
                        };
                        let health = if unit_health.is_empty() { 1.0 } else { unit_health.iter().map(|h| h.hull).sum::<f32>() / unit_health.len() as f32 };
                        Some(TacticalGroup { kind, type_id, name, count: m.count, fleet: m.fleet, health, unit_health })
                    })
                    .collect();
                let cap = world.factions[side.faction as usize].space_unit_cap;
                let (groups, slot_reserve) = apply_hyperspace_slots(galaxy, groups);
                let (groups, mut reserve) = split_reserve(world, groups, cap);
                reserve.extend(slot_reserve);
                // Reinforcements come in slot order 1, 2, 3 (stable inside a slot).
                reserve.sort_by_key(|g| g.fleet.and_then(|f| galaxy.fleets.get(&f)).map_or(u8::MAX, |f| if f.slot == 0 { u8::MAX } else { f.slot }));
                TacticalSide {
                    reserve,
                    unit_cap: cap,
                    faction: side.faction,
                    tech: galaxy.factions.get(side.faction as usize).map_or(0, |f| (f.tech - 1).clamp(0, 4) as u8),
                    faction_name: world.factions[side.faction as usize].name.clone(),
                    defender: battle.defender == Some(side.faction),
                    damage_mult: world_diff(side.faction).damage,
                    health_mult: world_diff(side.faction).health,
                    may_retreat: galaxy.factions.get(side.faction as usize).is_some_and(|f| f.planets_owned > 1) && !world.factions[side.faction as usize].name.eq_ignore_ascii_case("pirates"),
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
        let retreat_allowed = self.sides.iter().map(|s| s.may_retreat).collect();
        let unit_cap = self.sides.iter().map(|s| s.unit_cap).max().unwrap_or(150);
        let side_tech = self.sides.iter().map(|s| s.tech).collect();
        BattleConfig { side_tech, seed: self.seed, max_time: self.max_seconds, half_size, arena, side_mults, retreat_allowed, unit_cap, ..BattleConfig::default() }
    }

    /// Builds the `battle-sim` setup. Fleet groups carry their group index as the tag; side `i` of the request is
    /// battle side `i`. Returns `None` if a unit has no definition in `catalog`. `ai` selects the sides the built-in
    /// AI commands (a front end passes `false` for the human side).
    pub fn battle_setup(&self, catalog: &Arc<Catalog>, config: BattleConfig, ai: impl Fn(usize) -> bool) -> Option<BattleSetup> {
        self.battle_setup_with(catalog, config, ai, false)
    }

    /// Like [`TacticalRequest::battle_setup`]; with `deploy_reserve` the reinforcement pool is deployed from the start (tag = `groups.len() + reserve index`)
    /// instead of waiting for `Command::Reinforce` (headless battles, where the population cap does not apply).
    pub fn battle_setup_with(&self, catalog: &Arc<Catalog>, mut config: BattleConfig, ai: impl Fn(usize) -> bool, deploy_reserve: bool) -> Option<BattleSetup> {
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
            if deploy_reserve {
                for (ri, g) in side.reserve.iter().enumerate() {
                    units.push(UnitSetup { ship: catalog.ship_id(&g.name)?, count: g.count, tag: (side.groups.len() + ri) as u32 });
                }
            }
            sides.push(SideSetup { name: side.faction_name.clone(), units, structures, ai: ai(si) });
            for (gi, g) in side.groups.iter().enumerate() {
                if g.unit_health.is_empty() {
                    continue;
                }
                let tag = if g.kind == GroupKind::Unit { gi as u32 } else { NO_TAG };
                if let Some(ship) = catalog.ship_id(&g.name) {
                    config.start_health.push(battle_sim::StartHealth { side: si as u8, tag, ship, health: g.unit_health.iter().map(|h| (h.hull, h.shield)).collect() });
                }
            }
        }
        Some(BattleSetup { catalog: catalog.clone(), sides, config })
    }

    /// Converts a finished `battle-sim` result into a [`TacticalResult`]. Returns `None` if a unit has no definition in `catalog`.
    pub fn result_from(&self, catalog: &Catalog, result: &BattleResult) -> Option<TacticalResult> {
        let mut survived = Vec::new();
        let mut retreated = Vec::new();
        let mut health = Vec::new();
        let (mut res_called, mut res_survived) = (Vec::new(), Vec::new());
        for (si, side) in self.sides.iter().enumerate() {
            let (mut s_row, mut r_row, mut h_row) = (Vec::new(), Vec::new(), Vec::new());
            // Reserve groups: deployed ones (headless) carry tag groups.len() + index; called-in reinforcements report under REINFORCE_TAG by ship type.
            let (mut c_row, mut v_row) = (Vec::new(), Vec::new());
            let mut pool: Vec<(battle_sim::ShipId, u32, u32)> = result.groups.iter().filter(|x| x.side as usize == si && x.tag == battle_sim::REINFORCE_TAG).map(|x| (x.ship, x.total, x.survived)).collect();
            for (ri, g) in side.reserve.iter().enumerate() {
                let Some(ship) = catalog.ship_id(&g.name) else {
                    c_row.push(0);
                    v_row.push(0);
                    continue;
                };
                let tag = (side.groups.len() + ri) as u32;
                let (mut called, mut alive) = (0, 0);
                for gr in result.groups.iter().filter(|x| x.side as usize == si && x.tag == tag && x.ship == ship) {
                    called += gr.total;
                    alive += gr.survived;
                }
                // Called-in reinforcements of this ship type are handed out to the reserve groups in order.
                let mut want = g.count.saturating_sub(called);
                for p in pool.iter_mut().filter(|p| p.0 == ship) {
                    let take = p.1.min(want);
                    let surv = p.2.min(take);
                    called += take;
                    alive += surv;
                    p.1 -= take;
                    p.2 -= surv;
                    want -= take;
                }
                let called = called.min(g.count);
                c_row.push(called);
                v_row.push(alive.min(called));
            }
            res_called.push(c_row);
            res_survived.push(v_row);
            for (gi, g) in side.groups.iter().enumerate() {
                let ship = catalog.ship_id(&g.name)?;
                let tag = if g.kind == GroupKind::Unit { gi as u32 } else { NO_TAG };
                let (mut s, mut r) = (0, 0);
                let mut hl: Vec<UnitHealth> = Vec::new();
                for gr in result.groups.iter().filter(|x| x.side as usize == si && x.tag == tag && x.ship == ship) {
                    s += gr.survived;
                    r += gr.retreated;
                    hl.extend(gr.health.iter().map(|&(hull, shield)| UnitHealth { hull, shield }));
                }
                hl.sort_by(|a, b| b.hull.total_cmp(&a.hull).then(b.shield.total_cmp(&a.shield)));
                hl.truncate(g.count as usize);
                h_row.push(hl);
                s = s.min(if g.kind == GroupKind::Starbase { 1 } else { g.count });
                s_row.push(s);
                r_row.push(r.min(s));
            }
            survived.push(s_row);
            retreated.push(r_row);
            health.push(h_row);
        }
        Some(TacticalResult { winner: result.winner.map(|w| w as usize), survived, retreated, duration: result.duration, health, reserve_called: res_called, reserve_survived: res_survived })
    }

    /// Total units per `(source, fleet)` of a side including its reserve, in first-seen order (a group split by the cap is one member again).
    fn merged_members(&self, si: usize) -> Vec<(Source, Option<FleetId>, u32)> {
        let mut out: Vec<(Source, Option<FleetId>, u32)> = Vec::new();
        for g in self.sides[si].groups.iter().chain(&self.sides[si].reserve) {
            let source = match g.kind {
                GroupKind::Unit => Source::Unit(g.type_id),
                GroupKind::Starbase => Source::Starbase(g.type_id),
                GroupKind::Structure => Source::Structure(g.type_id),
            };
            match out.iter_mut().find(|m| m.0 == source && m.1 == g.fleet) {
                Some(m) => m.2 += g.count,
                None => out.push((source, g.fleet, g.count)),
            }
        }
        out
    }

    /// The request's sides and a result expressed as a [`CombatResult`]. Deployed groups and the reserve of one `(unit type, fleet)` are one
    /// member again: its survivors are the deployed survivors plus the reserve units never called plus the called ones that survived.
    pub(crate) fn combat_result(&self, result: &TacticalResult) -> (Vec<SideInput>, CombatResult) {
        let mut sides = Vec::new();
        let mut survivors = vec![Vec::new(); self.sides.len()];
        let mut retreated: Vec<FleetId> = Vec::new();
        for (si, side) in self.sides.iter().enumerate() {
            let members = self.merged_members(si);
            let mut alive: Vec<u32> = vec![0; members.len()];
            let mut fleet_gone: Vec<(FleetId, bool)> = Vec::new();
            let find = |g: &TacticalGroup| {
                let source = match g.kind {
                    GroupKind::Unit => Source::Unit(g.type_id),
                    GroupKind::Starbase => Source::Starbase(g.type_id),
                    GroupKind::Structure => Source::Structure(g.type_id),
                };
                members.iter().position(|m| m.0 == source && m.1 == g.fleet).expect("member exists")
            };
            for (gi, g) in side.groups.iter().enumerate() {
                let left = result.survived.get(si).and_then(|r| r.get(gi)).copied().unwrap_or(0).min(g.count);
                alive[find(g)] += left;
                if left > 0
                    && let Some(f) = g.fleet
                {
                    let gone = result.retreated.get(si).and_then(|r| r.get(gi)).copied().unwrap_or(0) >= left;
                    match fleet_gone.iter_mut().find(|x| x.0 == f) {
                        Some(x) => x.1 &= gone,
                        None => fleet_gone.push((f, gone)),
                    }
                }
            }
            // Reserve units that were never called in fought nothing. A side that won keeps them; a side that lost on the field loses them with it (EaW
            // keeps feeding the reserve in until nothing is left, so a beaten side never leaves ships behind in orbit for a second battle at the same planet),
            // and a side that left by retreating takes them along.
            let lost = result.winner.is_some_and(|w| w != si);
            let retreated_off = fleet_gone.iter().any(|x| x.1);
            for (ri, g) in side.reserve.iter().enumerate() {
                let called = result.reserve_called.get(si).and_then(|r| r.get(ri)).copied().unwrap_or(0).min(g.count);
                let alive_called = result.reserve_survived.get(si).and_then(|r| r.get(ri)).copied().unwrap_or(0).min(called);
                let uncalled = if lost && !retreated_off { 0 } else { g.count - called };
                alive[find(g)] += uncalled + alive_called;
                if lost
                    && retreated_off
                    && uncalled > 0
                    && let Some(f) = g.fleet
                    && !fleet_gone.iter().any(|x| x.0 == f)
                {
                    fleet_gone.push((f, true));
                }
            }
            sides.push(SideInput { faction: side.faction, members: members.iter().map(|&(source, fleet, count)| Member { source, fleet, count }).collect() });
            for (m, left) in sides[si].members.iter().zip(alive) {
                if left > 0 {
                    survivors[si].push(Member { count: left.min(m.count), ..*m });
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
    run_headless_observed(request, catalog, arena, &mut |_| {})
}

/// [`run_headless`] that shows the running battle to `observer` every 10 battle seconds (diagnostics).
pub fn run_headless_observed(request: &TacticalRequest, catalog: &Arc<Catalog>, arena: Option<Arc<Arena>>, observer: &mut dyn FnMut(&Battle)) -> Option<TacticalResult> {
    let config = BattleConfig { record_events: false, ..request.battle_config(arena) };
    let (max_steps, cap) = ((request.max_seconds / config.dt) as u64 + 1, config.unit_cap);
    let check_every = (1.0 / config.dt).round().max(1.0) as u64;
    let mut battle = Battle::new(request.battle_setup_with(catalog, config, |_| true, false)?);
    // The reserve (units that did not fit under the population cap) is called in as losses make room, like a player would.
    let mut waiting: Vec<Vec<(battle_sim::ShipId, u32)>> = request.sides.iter().map(|s| s.reserve.iter().filter_map(|g| catalog.ship_id(&g.name).map(|id| (id, g.count))).collect()).collect();
    let mut steps = 0;
    while !battle.is_over() && steps < max_steps {
        battle.step();
        steps += 1;
        if steps % (check_every * 10) == 0 {
            observer(&battle);
        }
        if steps % check_every != 0 {
            continue;
        }
        for (si, pool) in waiting.iter_mut().enumerate() {
            for (ship, left) in pool.iter_mut().filter(|(_, n)| *n > 0) {
                let pop = catalog.ship(*ship).population.max(0);
                let room = cap - battle.pop_in_play(si as u8);
                let n = if pop == 0 { *left } else { ((room.max(0) / pop) as u32).min(*left) };
                if n > 0 && battle.command(si as u8, battle_sim::Command::Reinforce { ship: *ship, count: n }).is_ok() {
                    *left -= n;
                }
            }
        }
    }
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
        self.galaxy.sync_ships();
        let request = TacticalRequest::from_sides(&self.world, &battle, &sides, seed, &self.galaxy);
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
        let events = self.apply_combat(&battle, &sides, combat);
        self.apply_health(&request, result);
        Ok(events)
    }

    /// Abandons a tactical battle that was only fought in part: the units lost and the hull and shield of the
    /// survivors so far are written into the galaxy, the battle stays pending and the frozen request is dropped.
    /// Resolving it afterwards (auto-resolve) starts from the forces that are left, not from the original ones.
    /// Returns the `UnitsDestroyed` / `HeroDestroyed` events of the losses.
    pub fn abandon_tactical(&mut self, battle_id: u32, partial: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        if !self.galaxy.battles.iter().any(|b| b.id == battle_id) {
            return Err(TacticalError::UnknownBattle);
        }
        let request = self.tactical_setup(battle_id).ok_or(TacticalError::UnknownBattle)?;
        if !request.check_shape(partial) {
            return Err(TacticalError::ShapeMismatch);
        }
        self.tactical_requests.remove(&battle_id);
        let (sides, combat) = request.combat_result(partial);
        let planet = request.planet;
        self.galaxy.sync_ships();
        let (events, _) = self.apply_losses(planet, &sides, &combat.survivors);
        self.apply_health(&request, partial);
        Ok(events)
    }

    /// `faction` gives the fight up (Retreat / F10 / Esc menu) with the state `partial` of the fight: the losses so far count, every surviving unit of
    /// that side leaves the battle by retreating and the opposing side (the defender if there is one) wins; planet defenses of a retreating defender are lost. The retreating fleets take the escape route
    /// (see [`Sim::apply_tactical_result`]); nobody captures anything because of a retreat. A faction that is not part of the battle falls back to
    /// [`Sim::abandon_tactical`] (the battle stays pending).
    pub fn retreat_tactical(&mut self, battle_id: u32, faction: FactionId, partial: &TacticalResult) -> Result<Vec<Event>, TacticalError> {
        let request = self.tactical_setup(battle_id).ok_or(TacticalError::UnknownBattle)?;
        if !request.check_shape(partial) {
            return Err(TacticalError::ShapeMismatch);
        }
        let Some(rs) = request.sides.iter().position(|s| s.faction == faction) else {
            return self.abandon_tactical(battle_id, partial);
        };
        let winner = request.sides.iter().enumerate().filter(|&(i, _)| i != rs).max_by_key(|&(i, s)| (s.defender, std::cmp::Reverse(i))).map(|(i, _)| i);
        let mut result = partial.clone();
        result.winner = winner;
        // Planet defenses (star base, structures, garrison) cannot flee: a defender who leaves the fight loses them, like in a lost battle.
        for (gi, g) in request.sides[rs].groups.iter().enumerate() {
            if g.fleet.is_none() {
                result.survived[rs][gi] = 0;
            }
        }
        result.retreated[rs] = result.survived[rs].clone();
        self.apply_tactical_result(battle_id, &result)
    }

    /// Abandons a tactical setup without a result (the battle stays pending, e.g. to auto-resolve it instead).
    pub fn cancel_tactical(&mut self, battle_id: u32) {
        self.tactical_requests.remove(&battle_id);
    }
}

impl Sim {
    /// Writes the outcome into the galaxy: in Empire at War, surviving ships and star bases heal back
    /// to 100% hull and shields upon battle completion. Damage is removed from `galaxy.damage`.
    pub(crate) fn apply_health(&mut self, request: &TacticalRequest, result: &TacticalResult) {
        let planet = request.planet as usize;
        for (si, side) in request.sides.iter().enumerate() {
            for (gi, g) in side.groups.iter().enumerate() {
                let survived = result.survived.get(si).and_then(|r| r.get(gi)).copied().unwrap_or(0) as usize;
                match (g.kind, g.fleet) {
                    (GroupKind::Unit, Some(fleet)) if survived > 0 => {
                        let ships: Vec<ShipId> = self.galaxy.ships.get(&fleet).into_iter().flatten().filter(|(_, t)| *t == g.type_id).map(|(s, _)| *s).take(survived).collect();
                        for ship in ships {
                            self.galaxy.damage.remove(&ship);
                        }
                    }
                    (GroupKind::Starbase, _) if survived > 0 => {
                        self.galaxy.planets[planet].base_health = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Repairs damaged ships and star bases at the owner's planets (estimate, see `SimConfig::repair_*`).
    pub(crate) fn repair(&mut self, dt: f32) {
        let world = self.world.clone();
        let cfg = &world.config;
        let weeks = dt / cfg.week_seconds;
        let (base, per_level, shield_factor) = (cfg.repair_base_per_week, cfg.repair_per_starbase_level_per_week, cfg.repair_shield_factor);
        let rate = |g: &Galaxy, p: usize| -> f32 { base + per_level * g.planets[p].starbase.map_or(0, |sb| world.starbases[sb as usize].level.max(0)) as f32 };
        if !self.galaxy.damage.is_empty() {
            let mut heal: Vec<(ShipId, f32)> = Vec::new();
            for f in self.galaxy.fleets.values() {
                if let FleetLocation::AtPlanet(p) = f.location
                    && self.galaxy.planets[p as usize].owner == Some(f.owner)
                {
                    let r = rate(&self.galaxy, p as usize) * weeks;
                    for (s, _) in self.galaxy.ships.get(&f.id).into_iter().flatten() {
                        if self.galaxy.damage.contains_key(s) {
                            heal.push((*s, r));
                        }
                    }
                }
            }
            for (s, r) in heal {
                if let Some(h) = self.galaxy.damage.get_mut(&s) {
                    h.hull = (h.hull + r).min(1.0);
                    h.shield = (h.shield + r * shield_factor).min(1.0);
                    if h.is_full() {
                        self.galaxy.damage.remove(&s);
                    }
                }
            }
        }
        for p in 0..self.galaxy.planets.len() {
            if self.galaxy.planets[p].base_health.is_some() && self.galaxy.planets[p].owner.is_some() {
                let r = rate(&self.galaxy, p) * weeks;
                if let Some(h) = &mut self.galaxy.planets[p].base_health {
                    h.hull = (h.hull + r).min(1.0);
                    h.shield = (h.shield + r * shield_factor).min(1.0);
                    if h.is_full() {
                        self.galaxy.planets[p].base_health = None;
                    }
                }
            }
        }
    }
}
