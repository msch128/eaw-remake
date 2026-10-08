//! Mutable simulation state plus the command and event types of the public API.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::rng::Rng;
use crate::world::*;

/// Stable identity of one ship; survives fleet merges, splits and moves. Ids are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct ShipId(pub u64);

/// Where a fleet is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FleetLocation {
    /// In orbit of a planet.
    AtPlanet(PlanetId),
    /// In hyperspace along a lane.
    Moving {
        /// Planet the current hop started at.
        from: PlanetId,
        /// Planet the current hop ends at.
        to: PlanetId,
        /// Route index of the hop.
        route: u32,
        /// Distance already travelled on this hop.
        progress: f32,
        /// Planets after `to` on the way to the destination.
        rest: Vec<PlanetId>,
    },
}

/// A group of ships moving together.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fleet {
    /// Unique id.
    pub id: FleetId,
    /// Owner.
    pub owner: FactionId,
    /// Unit counts by type.
    pub units: BTreeMap<UnitTypeId, u32>,
    /// Current location.
    pub location: FleetLocation,
    /// Planet the fleet came from (retreat target).
    pub previous_planet: PlanetId,
}

impl Fleet {
    /// Planet the fleet is orbiting, if any.
    pub fn planet(&self) -> Option<PlanetId> {
        match self.location {
            FleetLocation::AtPlanet(p) => Some(p),
            FleetLocation::Moving { .. } => None,
        }
    }

    /// Total number of ships/squadrons.
    pub fn unit_count(&self) -> u32 {
        self.units.values().sum()
    }
}

/// What a production item builds.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProductionKind {
    /// A ship, squadron or research item.
    Unit(UnitTypeId),
    /// A planetary/orbital structure.
    Structure(StructureTypeId),
    /// A star base level.
    Starbase(StarbaseTypeId),
    /// The faction's next tech level (index into `tech_upgrades`).
    Tech(usize),
}

/// One entry of a planet's build queue. Cost is paid when the item is queued.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProductionItem {
    /// What is being built.
    pub kind: ProductionKind,
    /// Seconds left.
    pub remaining: f32,
    /// Total build time in seconds.
    pub total: f32,
    /// Credits paid.
    pub cost: f32,
}

/// Production lane of a planet; each lane builds one item at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Lane {
    /// Ships, squadrons, research.
    Units = 0,
    /// Structures.
    Structures = 1,
    /// Star base and tech upgrades.
    Base = 2,
}

/// Dynamic state of a planet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanetState {
    /// Owner; `None` = neutral/unowned.
    pub owner: Option<FactionId>,
    /// Current star base level.
    pub starbase: Option<StarbaseTypeId>,
    /// Built structures.
    pub structures: Vec<StructureTypeId>,
    /// Build queues indexed by [`Lane`].
    pub queues: [VecDeque<ProductionItem>; 3],
    /// Faction currently holding orbit uncontested and seconds accumulated.
    pub capture: Option<(FactionId, f32)>,
    /// Damage the star base carries over from battles (`None` = undamaged).
    pub base_health: Option<UnitHealth>,
    /// Destroyed by a superweapon: an asteroid field that nobody can own or build on.
    pub destroyed: bool,
    /// Current income swing of a gambling structure (a fraction of the planet's base income; see `IncomeGamble`).
    pub gamble: f32,
    /// Game time of the next gambling evaluation.
    pub gamble_at: f64,
}

/// Hull and shield of a ship or star base as a fraction of the maximum (1.0 = undamaged).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnitHealth {
    /// Hull fraction.
    pub hull: f32,
    /// Shield fraction.
    pub shield: f32,
}

impl UnitHealth {
    /// Undamaged.
    pub const FULL: UnitHealth = UnitHealth { hull: 1.0, shield: 1.0 };

    /// True if nothing is missing.
    pub fn is_full(&self) -> bool {
        self.hull >= 0.999 && self.shield >= 0.999
    }
}

/// Dynamic state of a faction.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactionState {
    /// Current credits.
    pub credits: f64,
    /// Current tech level.
    pub tech: i32,
    /// Objects unlocked by research (lower-cased names).
    pub unlocked: BTreeSet<String>,
    /// Units built over the whole game, for lifetime build limits.
    pub built_lifetime: BTreeMap<UnitTypeId, u32>,
    /// True once the faction has no planets and no fleets.
    pub eliminated: bool,
    /// Cached income per week (updated every tick).
    pub income_per_week: f64,
    /// Population used by living units.
    pub population_used: i32,
    /// Population capacity.
    pub population_cap: i32,
    /// Planets owned.
    pub planets_owned: u32,
    /// Difficulty multipliers of this faction (neutral 1.0 unless set with [`crate::Sim::set_faction_difficulty`]).
    pub difficulty: Difficulty,
    /// Game time (seconds) at which the faction's superweapons are ready again.
    pub superweapon_ready_at: f64,
    /// Running totals for the end-of-game screen.
    pub stats: FactionStats,
}

/// Per-faction statistics, part of the state hash (so a save replay rebuilds them).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactionStats {
    /// Battles the faction won.
    pub battles_won: u32,
    /// Battles the faction lost (wiped out).
    pub battles_lost: u32,
    /// Ships and squadrons finished by production.
    pub ships_built: u32,
    /// Ships and squadrons destroyed in battles.
    pub ships_lost: u32,
    /// Structures and star base levels finished.
    pub structures_built: u32,
    /// Planets taken from someone else or from the neutrals (capture completed).
    pub planets_captured: u32,
    /// Planets that left the faction's hands.
    pub planets_lost: u32,
    /// Credits gained: positive net income, bonuses, script rewards.
    pub credits_earned: f64,
    /// Credits paid for production (net of refunds).
    pub credits_spent: f64,
}

/// A battle waiting to be resolved (by a tactical battle or auto-resolve).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PendingBattle {
    /// Unique id.
    pub id: u32,
    /// Where the battle takes place.
    pub planet: PlanetId,
    /// Fleets in orbit grouped per faction.
    pub fleets: Vec<FleetId>,
    /// Factions involved (fleets and/or planetary defenses).
    pub factions: Vec<FactionId>,
    /// Owner of the planet whose defenses take part, if any.
    pub defender: Option<FactionId>,
}

/// The mutable part of a match.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Galaxy {
    /// Elapsed galactic time in seconds.
    pub time: f64,
    /// Planets, indexed by [`PlanetId`].
    pub planets: Vec<PlanetState>,
    /// Factions, indexed by [`FactionId`].
    pub factions: Vec<FactionState>,
    /// Fleets by id (ordered so iteration is deterministic).
    pub fleets: BTreeMap<FleetId, Fleet>,
    /// Battles waiting for resolution.
    pub battles: Vec<PendingBattle>,
    /// Winner, once decided.
    pub winner: Option<FactionId>,
    /// Runtime alliances (sorted pairs); allied factions are not hostile to each other. Set with
    /// [`crate::Sim::set_alliance`].
    pub allied: std::collections::BTreeSet<(FactionId, FactionId)>,
    /// Stable per-ship ids of every fleet, kept in sync with `Fleet::units` (see [`crate::Sim::ships_of`]).
    pub ships: BTreeMap<FleetId, Vec<(ShipId, UnitTypeId)>>,
    /// Damage carried between battles per ship (absent = undamaged); repaired over time at owned planets.
    pub damage: BTreeMap<ShipId, UnitHealth>,
    /// `(planet, faction)` pairs that already received the planet's capture bonus.
    pub rewarded: std::collections::BTreeSet<(PlanetId, FactionId)>,
    pub(crate) next_ship_id: u64,
    pub(crate) next_fleet_id: FleetId,
    pub(crate) next_battle_id: u32,
    pub(crate) rng: Rng,
    pub(crate) week: u64,
    pub(crate) ai_timer: f32,
    pub(crate) tick_accum: f32,
}

/// Instructions from the UI or AI. All commands are validated; invalid ones return a [`CommandError`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Command {
    /// Queue a ship/squadron/research item on a planet.
    BuildUnit {
        /// Issuing faction.
        faction: FactionId,
        /// Building planet.
        planet: PlanetId,
        /// Catalog unit.
        unit: UnitTypeId,
    },
    /// Queue the next star base level (or the first one).
    UpgradeStarbase {
        /// Issuing faction.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
    },
    /// Queue a structure.
    BuildStructure {
        /// Issuing faction.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
        /// Catalog structure.
        structure: StructureTypeId,
    },
    /// Queue the next tech level upgrade at a planet.
    UpgradeTech {
        /// Issuing faction.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
    },
    /// Debug: put `count` ships of `unit` into the faction's fleet orbiting `planet` for free (population, cost and requirements ignored).
    /// Refused unless `SimConfig::debug_commands` is set. Journaled like every command, so saves replay it.
    DebugGrant {
        /// Receiving faction.
        faction: FactionId,
        /// Planet to appear at.
        planet: PlanetId,
        /// Unit type.
        unit: UnitTypeId,
        /// Number of ships.
        count: u32,
    },
    /// Fire a superweapon (a unit with `Planet_Destruction_Ability`, e.g. the Death Star) of a fleet in orbit at the planet it orbits.
    DestroyPlanet {
        /// Issuing faction.
        faction: FactionId,
        /// Planet in whose orbit the superweapon is.
        planet: PlanetId,
    },
    /// Send a fleet to a planet along hyperspace lanes.
    MoveFleet {
        /// Issuing faction.
        faction: FactionId,
        /// Fleet.
        fleet: FleetId,
        /// Destination planet.
        to: PlanetId,
    },
    /// Like [`Command::MoveFleet`], but the route goes around systems that would stop the fleet (hostile star base or fleet) when there
    /// is a way around. Used by the AI; the player's orders take the plain fastest route and run into blockades, as in the game.
    MoveFleetAround {
        /// Issuing faction.
        faction: FactionId,
        /// Fleet.
        fleet: FleetId,
        /// Destination planet.
        to: PlanetId,
    },
    /// Merge `from` into `into` (same owner, both in orbit of the same planet).
    MergeFleets {
        /// Issuing faction.
        faction: FactionId,
        /// Surviving fleet.
        into: FleetId,
        /// Fleet to absorb.
        from: FleetId,
    },
    /// Split units off a fleet into a new fleet.
    SplitFleet {
        /// Issuing faction.
        faction: FactionId,
        /// Fleet to split.
        fleet: FleetId,
        /// Units to move to the new fleet.
        units: Vec<(UnitTypeId, u32)>,
    },
    /// Send a fleet back to the planet it came from.
    /// Remove an entry from a planet's production queue and refund it (`SimConfig::cancel_refund`).
    CancelProduction {
        /// Issuing faction.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
        /// Queue.
        lane: Lane,
        /// Position in the queue (0 = the item being built).
        index: u32,
    },
    /// Send a fleet back where it came from.
    Retreat {
        /// Issuing faction.
        faction: FactionId,
        /// Fleet.
        fleet: FleetId,
    },
}

/// Why a command was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Unknown planet/fleet/unit id or the fleet does not belong to the faction.
    Invalid,
    /// The faction does not own the planet.
    NotOwner,
    /// The faction has been eliminated.
    Eliminated,
    /// Not enough credits.
    InsufficientCredits,
    /// A requirement (tech, star base level, structure, lock) is not met.
    RequirementsNotMet(String),
    /// Population cap reached.
    PopulationCap,
    /// Build limit reached.
    BuildLimit,
    /// All slots / queue positions are used.
    NoSlot,
    /// The faction cannot build this object.
    NotAvailable,
    /// There is no hyperspace path to the destination.
    NoPath,
    /// The fleet is in a state where the order does not apply (e.g. in hyperspace).
    WrongState,
}

/// Result of a successfully applied command.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandOk {
    /// Nothing special to report.
    Done,
    /// A new fleet was created.
    NewFleet(FleetId),
    /// A production entry was cancelled; the credits returned.
    Refunded(f64),
}

/// Something that happened during a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A galactic week ended; income was paid out continuously during it.
    WeekEnded {
        /// Week number (1-based).
        week: u64,
    },
    /// A queued item finished.
    ConstructionComplete {
        /// Owner.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
        /// What was built.
        kind: ProductionKind,
    },
    /// A fleet reached its destination planet.
    FleetArrived {
        /// Fleet.
        fleet: FleetId,
        /// Planet.
        planet: PlanetId,
    },
    /// Hostile forces met; a battle must be resolved.
    BattlePending(PendingBattle),
    /// A battle was resolved.
    BattleResolved {
        /// Battle id.
        id: u32,
        /// Planet.
        planet: PlanetId,
        /// Winning faction, `None` for a draw or mutual destruction.
        winner: Option<FactionId>,
        /// Ships lost per faction.
        losses: Vec<(FactionId, u32)>,
    },
    /// A planet changed owner.
    PlanetCaptured {
        /// Planet.
        planet: PlanetId,
        /// Previous owner.
        old_owner: Option<FactionId>,
        /// New owner.
        new_owner: FactionId,
    },
    /// A faction reached a higher tech level (by research, or set by a script).
    TechAdvanced {
        /// Faction.
        faction: FactionId,
        /// New tech level.
        level: i32,
    },
    /// A faction won a battle (it was the only side left).
    BattleWon {
        /// Faction.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
    },
    /// A faction took part in a battle and lost it (or was wiped out in a draw).
    BattleLost {
        /// Faction.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
    },
    /// Ships were destroyed in a battle (one event per faction and unit type).
    UnitsDestroyed {
        /// Owner.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
        /// Unit type.
        unit: UnitTypeId,
        /// Number lost.
        count: u32,
        /// Ship ids that were lost.
        ships: Vec<ShipId>,
    },
    /// A hero unit was destroyed (also reported as `UnitsDestroyed`; the Remake's story timers respawn it, see `galaxy-script`).
    HeroDestroyed {
        /// Owner.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
        /// Hero unit type.
        unit: UnitTypeId,
    },
    /// A conqueror received the planet's capture bonus.
    CaptureBonus {
        /// Conqueror.
        faction: FactionId,
        /// Planet.
        planet: PlanetId,
        /// Credits.
        credits: f64,
    },
    /// A superweapon destroyed a planet (it is an asteroid field now; the ships of everyone else in orbit are lost).
    PlanetDestroyed {
        /// Planet.
        planet: PlanetId,
        /// Faction that fired.
        by: FactionId,
        /// Previous owner.
        owner: Option<FactionId>,
        /// Ships lost in orbit.
        ships_lost: u32,
    },
    /// A faction lost its last planet and fleet.
    FactionEliminated {
        /// Faction.
        faction: FactionId,
    },
    /// The game is decided.
    Victory {
        /// Winner.
        faction: FactionId,
    },
}

impl Galaxy {
    /// Completed galactic weeks (the number of `WeekEnded` events so far); the UI shows `week() + 1`.
    pub fn week(&self) -> u64 {
        self.week
    }

    pub(crate) fn new(world: &World, seed: u64) -> Self {
        Galaxy {
            time: 0.0,
            planets: vec![PlanetState::default(); world.planets.len()],
            factions: world
                .factions
                .iter()
                .map(|f| FactionState { credits: f.start_credits, tech: f.start_tech, ..FactionState::default() })
                .collect(),
            fleets: BTreeMap::new(),
            battles: Vec::new(),
            winner: None,
            allied: Default::default(),
            ships: BTreeMap::new(),
            damage: BTreeMap::new(),
            rewarded: Default::default(),
            next_ship_id: 1,
            next_fleet_id: 1,
            next_battle_id: 1,
            rng: Rng::new(seed),
            week: 0,
            ai_timer: 0.0,
            tick_accum: 0.0,
        }
    }

    /// Brings the per-ship id lists in line with the unit counts of every fleet: surplus ids (lost
    /// ships) are dropped from the back, missing ones get fresh ids, removed fleets lose their list.
    /// Called by the simulation; call it before reading [`Galaxy::ships`] after editing fleets directly.
    pub fn sync_ships(&mut self) {
        self.ships.retain(|id, _| self.fleets.contains_key(id));
        for (id, fleet) in &self.fleets {
            let list = self.ships.entry(*id).or_default();
            // Drop ids of types the fleet no longer has enough of.
            let mut seen: BTreeMap<UnitTypeId, u32> = BTreeMap::new();
            list.retain(|(_, t)| {
                let n = seen.entry(*t).or_default();
                *n += 1;
                *n <= fleet.units.get(t).copied().unwrap_or(0)
            });
            for (&u, &want) in &fleet.units {
                let have = seen.get(&u).copied().unwrap_or(0).min(want);
                for _ in have..want {
                    list.push((ShipId(self.next_ship_id), u));
                    self.next_ship_id += 1;
                }
            }
        }
        if !self.damage.is_empty() {
            let alive: std::collections::BTreeSet<ShipId> = self.ships.values().flatten().map(|(s, _)| *s).collect();
            self.damage.retain(|s, _| alive.contains(s));
        }
    }

    /// Removes and returns up to `count` ship ids of type `unit` from a fleet's id list (the units
    /// themselves must be removed by the caller).
    pub(crate) fn take_ships(&mut self, fleet: FleetId, unit: UnitTypeId, count: u32) -> Vec<(ShipId, UnitTypeId)> {
        let mut out = Vec::new();
        if let Some(list) = self.ships.get_mut(&fleet) {
            for _ in 0..count {
                match list.iter().rposition(|(_, t)| *t == unit) {
                    Some(pos) => out.push(list.remove(pos)),
                    None => break,
                }
            }
        }
        out
    }

    /// True if `a` and `b` are at war: hostile in the data and not allied at runtime.
    pub fn hostile(&self, world: &World, a: FactionId, b: FactionId) -> bool {
        world.hostile(a, b) && !self.allied.contains(&(a.min(b), a.max(b)))
    }

    /// Fleets of `faction` orbiting `planet`.
    pub fn fleets_at(&self, planet: PlanetId) -> impl Iterator<Item = &Fleet> {
        self.fleets.values().filter(move |f| f.location == FleetLocation::AtPlanet(planet))
    }

    /// Planets owned by `faction`.
    pub fn owned_planets(&self, faction: FactionId) -> impl Iterator<Item = PlanetId> + '_ {
        self.planets.iter().enumerate().filter(move |(_, p)| p.owner == Some(faction)).map(|(i, _)| i as PlanetId)
    }

    /// Stable 64-bit hash of the whole state, for determinism checks.
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.time.to_bits().hash(&mut h);
        for p in &self.planets {
            p.owner.hash(&mut h);
            p.starbase.hash(&mut h);
            p.structures.hash(&mut h);
            p.destroyed.hash(&mut h);
            p.gamble.to_bits().hash(&mut h);
            for q in &p.queues {
                for it in q {
                    it.remaining.to_bits().hash(&mut h);
                }
                q.len().hash(&mut h);
            }
        }
        for f in &self.factions {
            f.credits.to_bits().hash(&mut h);
            f.tech.hash(&mut h);
            f.eliminated.hash(&mut h);
            f.superweapon_ready_at.to_bits().hash(&mut h);
            let s = &f.stats;
            (s.battles_won, s.battles_lost, s.ships_built, s.ships_lost, s.structures_built, s.planets_captured, s.planets_lost).hash(&mut h);
            (s.credits_earned.to_bits(), s.credits_spent.to_bits()).hash(&mut h);
        }
        for (id, fl) in &self.fleets {
            id.hash(&mut h);
            fl.owner.hash(&mut h);
            fl.units.iter().collect::<Vec<_>>().hash(&mut h);
            for (s, hp) in self.damage.iter().filter(|(s, _)| self.ships.get(id).is_some_and(|l| l.iter().any(|(x, _)| x == *s))) {
                (s, hp.hull.to_bits(), hp.shield.to_bits()).hash(&mut h);
            }
            match &fl.location {
                FleetLocation::AtPlanet(p) => (0u8, *p).hash(&mut h),
                FleetLocation::Moving { to, progress, .. } => (1u8, *to, progress.to_bits()).hash(&mut h),
            }
        }
        h.finish()
    }
}

/// Per-faction difficulty multipliers (`DifficultyAdjustments.xml`, see `alamo_data::DifficultyAdjustment`). Credits
/// and build time act in the galaxy layer; damage, health and shield in battles (statistical and tactical).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Difficulty {
    /// Multiplier on positive income.
    pub credit: f32,
    /// Multiplier on build times.
    pub build_time: f32,
    /// Multiplier on damage dealt in battle.
    pub damage: f32,
    /// Multiplier on hull points in battle.
    pub health: f32,
    /// Multiplier on shield points in battle.
    pub shield: f32,
}

impl Default for Difficulty {
    fn default() -> Self {
        Self { credit: 1.0, build_time: 1.0, damage: 1.0, health: 1.0, shield: 1.0 }
    }
}

impl Difficulty {
    /// From a table entry (galactic build time).
    pub fn from_adjustment(a: &alamo_data::DifficultyAdjustment) -> Self {
        Self { credit: a.credit_multiplier, build_time: a.galactic_build_time, damage: a.damage, health: a.health, shield: a.shield }
    }
}
