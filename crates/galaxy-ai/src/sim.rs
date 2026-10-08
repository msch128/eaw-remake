//! What the AI needs from a simulation: read-only perception queries and the few actions plans take.
//! `galaxy-script` implements it on top of galaxy-sim.

/// Player (faction) index.
pub type PlayerId = u32;
/// Planet index.
pub type PlanetId = u32;
/// Fleet id.
pub type FleetId = u32;

/// A fleet as the AI sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetView {
    /// Fleet id.
    pub id: FleetId,
    /// Owner.
    pub owner: PlayerId,
    /// Planet it orbits, `None` in hyperspace.
    pub at: Option<PlanetId>,
    /// Destination while moving.
    pub heading_to: Option<PlanetId>,
    /// Unit type name and count.
    pub units: Vec<(String, u32)>,
}

/// Static facts about a ship type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnitInfo {
    /// Object name.
    pub name: String,
    /// Size class, upper case (`FIGHTER`, `FRIGATE`, `CAPITAL`, ...).
    pub category: String,
    /// `AI_Combat_Power`-like strength.
    pub power: f64,
    /// Build cost.
    pub cost: f64,
    /// Hero / unique unit (not part of generic task forces).
    pub is_hero: bool,
}

/// Static facts about a structure type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StructureInfo {
    /// Object name.
    pub name: String,
    /// Build cost.
    pub cost: f64,
    /// True for orbital (space) structures.
    pub orbital: bool,
}

/// One item in a planet's build queue.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueView {
    /// Item name (unit, structure or star base).
    pub name: String,
    /// Seconds remaining.
    pub remaining: f64,
}

/// Why a build or move request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Not enough credits (retry later).
    NoCredits,
    /// Requirements not met on this planet (tech, shipyard, locks) or a build limit reached.
    Unavailable,
    /// Fine in principle but busy right now: the production queue of the planet is full or the population cap is reached (retry later).
    Busy,
    /// Unknown name or other permanent problem.
    Invalid,
}

/// The simulation as seen by the AI.
pub trait AiSim {
    // ---- identity and time ---------------------------------------------------------------------
    /// Game time in seconds.
    fn time(&self) -> f64;
    /// Number of players (factions).
    fn player_count(&self) -> u32;
    /// Faction name (`Empire`, `Rebel`, ...).
    fn faction_name(&self, p: PlayerId) -> String;
    /// True if a human plays this faction.
    fn is_human(&self, p: PlayerId) -> bool;
    /// True if the faction is out of the game.
    fn is_eliminated(&self, p: PlayerId) -> bool;
    /// True if the two factions are at war.
    fn is_enemy(&self, a: PlayerId, b: PlayerId) -> bool;
    /// Length of a campaign "week" in seconds.
    fn week_seconds(&self) -> f64;

    // ---- economy ---------------------------------------------------------------------------------
    /// Credits.
    fn credits(&self, p: PlayerId) -> f64;
    /// Net income per week.
    fn income_per_week(&self, p: PlayerId) -> f64;
    /// Tech level.
    fn tech_level(&self, p: PlayerId) -> i32;
    /// Highest tech level the faction can reach.
    fn max_tech_level(&self, p: PlayerId) -> i32;

    // ---- map ---------------------------------------------------------------------------------------
    /// Number of planets (including dummy locations; see [`is_real`](Self::is_real)).
    fn planet_count(&self) -> u32;
    /// True for real planets (not decorative locations).
    fn is_real(&self, pl: PlanetId) -> bool;
    /// Planet name.
    fn planet_name(&self, pl: PlanetId) -> String;
    /// Planet position.
    fn planet_position(&self, pl: PlanetId) -> [f32; 3];
    /// Owner, `None` = neutral.
    fn planet_owner(&self, pl: PlanetId) -> Option<PlayerId>;
    /// Hyperspace neighbours.
    fn neighbors(&self, pl: PlanetId) -> Vec<PlanetId>;
    /// Base income per week of the planet.
    fn planet_income(&self, pl: PlanetId) -> f64;
    /// `(orbital, ground)` structure slots.
    fn structure_slots(&self, pl: PlanetId) -> (i32, i32);
    /// Names of the built structures.
    fn structures(&self, pl: PlanetId) -> Vec<String>;
    /// Current star base level (0 = none) and the highest level that exists for the owner's faction.
    fn starbase_level(&self, pl: PlanetId) -> (i32, i32);
    /// Items being built at the planet.
    fn build_queue(&self, pl: PlanetId) -> Vec<QueueView>;

    // ---- forces ------------------------------------------------------------------------------------
    /// Fleets of all players currently orbiting a planet.
    fn fleets_at(&self, pl: PlanetId) -> Vec<FleetView>;
    /// A fleet by id.
    fn fleet(&self, id: FleetId) -> Option<FleetView>;
    /// All fleets (for free store scans); only fleets of `p`.
    fn fleets_of(&self, p: PlayerId) -> Vec<FleetView>;
    /// Facts about a ship type.
    fn unit_info(&self, name: &str) -> Option<UnitInfo>;
    /// Ship types the faction may build in principle (affiliation, build menu).
    fn buildable_units(&self, p: PlayerId) -> Vec<String>;
    /// True if `name` is a star base level (queued with `queue_starbase`).
    fn is_starbase_type(&self, _name: &str) -> bool {
        false
    }
    /// Structure facts.
    fn structure_info(&self, name: &str) -> Option<StructureInfo>;
    /// Structure types the faction may build in principle.
    fn buildable_structures(&self, p: PlayerId) -> Vec<String>;

    /// Whether a ship could be queued at the planet, apart from the price (Err(NoCredits) means only the money is missing).
    fn can_build_unit(&self, p: PlayerId, pl: PlanetId, unit: &str) -> Result<(), Refused>;
    /// The same for structures.
    fn can_build_structure(&self, p: PlayerId, pl: PlanetId, structure: &str) -> Result<(), Refused>;
    /// The same for the next star base level.
    fn can_upgrade_starbase(&self, p: PlayerId, pl: PlanetId) -> Result<(), Refused>;

    // ---- actions -----------------------------------------------------------------------------------
    /// Queues a ship; cost is paid immediately. Returns the cost.
    fn queue_unit(&mut self, p: PlayerId, pl: PlanetId, unit: &str) -> Result<f64, Refused>;
    /// Queues a structure.
    fn queue_structure(&mut self, p: PlayerId, pl: PlanetId, structure: &str) -> Result<f64, Refused>;
    /// Queues the next star base level.
    fn queue_starbase(&mut self, p: PlayerId, pl: PlanetId) -> Result<f64, Refused>;
    /// Queues the next tech level upgrade at a planet.
    fn queue_tech(&mut self, p: PlayerId, pl: PlanetId) -> Result<f64, Refused>;
    /// Orders a fleet to a planet.
    fn move_fleet(&mut self, p: PlayerId, fleet: FleetId, to: PlanetId) -> Result<(), Refused>;
    /// Splits units off a fleet; returns the new fleet.
    fn split_fleet(&mut self, p: PlayerId, fleet: FleetId, units: &[(String, u32)]) -> Result<FleetId, Refused>;
    /// Merges `from` into `into` (same planet, same owner).
    fn merge_fleets(&mut self, p: PlayerId, into: FleetId, from: FleetId) -> Result<(), Refused>;
    /// Ground troops take a planet that no hostile fleet defends (neutral or abandoned): changes the owner.
    fn capture_planet(&mut self, p: PlayerId, pl: PlanetId) -> Result<(), Refused>;
    /// Hyperspace path between planets (inclusive), if any.
    fn path(&self, from: PlanetId, to: PlanetId) -> Option<Vec<PlanetId>>;

    // ---- combat prediction (optional; the strategy layer degrades gracefully without) ------------------------------
    /// Combined combat strength of a group of ships (Lanchester square-law measure `sqrt(durability x damage rate)`, the
    /// same units as [`defense_strength`](Self::defense_strength)). 0 when unknown.
    fn force_strength(&self, _units: &[(String, u32)]) -> f64 {
        0.0
    }
    /// Strength a force of `attacker` meets at a planet: star base, armed structures, garrison and the fleets of hostile players.
    fn defense_strength(&self, _pl: PlanetId, _attacker: PlayerId) -> f64 {
        0.0
    }
    /// Strength of the fixed defenses of a planet (star base, armed structures, garrison) for its owner; 0 if none or unknown.
    fn planet_defense(&self, _pl: PlanetId) -> f64 {
        0.0
    }
    /// Seconds a fleet needs to reach a planet from where it is, `None` if unknown or unreachable.
    fn eta(&self, _fleet: FleetId, _to: PlanetId) -> Option<f64> {
        None
    }

    // ---- mapping between the script world and ids ----------------------------------------------------
    /// The script object id of a planet.
    fn planet_object(&self, pl: PlanetId) -> u64;
    /// The planet a script object id refers to.
    fn object_planet(&self, object: u64) -> Option<PlanetId>;
}
