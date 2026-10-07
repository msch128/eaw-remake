//! Mutable simulation state plus the command and event types of the public API.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::rng::Rng;
use crate::world::*;

/// Where a fleet is.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Ships, squadrons, research.
    Units = 0,
    /// Structures.
    Structures = 1,
    /// Star base and tech upgrades.
    Base = 2,
}

/// Dynamic state of a planet.
#[derive(Debug, Clone, Default, PartialEq)]
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
}

/// Dynamic state of a faction.
#[derive(Debug, Clone, Default, PartialEq)]
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
}

/// A battle waiting to be resolved (by a tactical battle or auto-resolve).
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
    pub(crate) next_fleet_id: FleetId,
    pub(crate) next_battle_id: u32,
    pub(crate) rng: Rng,
    pub(crate) week: u64,
    pub(crate) ai_timer: f32,
    pub(crate) tick_accum: f32,
}

/// Instructions from the UI or AI. All commands are validated; invalid ones return a [`CommandError`].
#[derive(Debug, Clone, PartialEq)]
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
    /// Send a fleet to a planet along hyperspace lanes.
    MoveFleet {
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
            next_fleet_id: 1,
            next_battle_id: 1,
            rng: Rng::new(seed),
            week: 0,
            ai_timer: 0.0,
            tick_accum: 0.0,
        }
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
        }
        for (id, fl) in &self.fleets {
            id.hash(&mut h);
            fl.owner.hash(&mut h);
            fl.units.iter().collect::<Vec<_>>().hash(&mut h);
            match &fl.location {
                FleetLocation::AtPlanet(p) => (0u8, *p).hash(&mut h),
                FleetLocation::Moving { to, progress, .. } => (1u8, *to, progress.to_bits()).hash(&mut h),
            }
        }
        h.finish()
    }
}
