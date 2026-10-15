//! Static (immutable) world data derived once from the game data: planets, hyperspace lanes,
//! factions, unit/structure/star base catalogs and the damage matrix.
//!
//! Everything here is read-only during a match, so it can be shared freely between threads.

use std::collections::{HashMap, HashSet};

use alamo_data::{Campaign, GameData, ShipCategory};

/// Population per side in a space battle (user rule, replaces the data's `Space_Tactical_Unit_Cap` when that is smaller).
pub const USER_SPACE_UNIT_CAP: i32 = 1000;

/// Index of a planet in [`World::planets`].
pub type PlanetId = u32;
/// Index of a faction in [`World::factions`].
pub type FactionId = u32;
/// Index of a unit type in [`World::units`].
pub type UnitTypeId = u32;
/// Index of a structure type in [`World::structures`].
pub type StructureTypeId = u32;
/// Index of a star base level in [`World::starbases`].
pub type StarbaseTypeId = u32;
/// Identifier of a fleet (stable for the lifetime of the fleet).
pub type FleetId = u32;

/// Tunable simulation constants. Defaults follow the Remake data where the data defines them and
/// are documented estimates elsewhere (see RULES.md).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SimConfig {
    /// Seed for all pseudo random decisions (AI tie breaks, combat target selection).
    pub seed: u64,
    /// Length of one galactic "week" in seconds; income is quoted per week.
    pub week_seconds: f32,
    /// Internal fixed time step in seconds.
    pub tick_seconds: f32,
    /// Scale from `Hyperspace_Speed * HS_Speed_Factor` to galaxy map units per second.
    pub galactic_speed_scale: f32,
    /// Multiplier on `Build_Time_Seconds`.
    pub build_time_scale: f32,
    /// Seconds an uncontested hostile fleet must stay in orbit before a planet changes hands.
    pub capture_delay_seconds: f32,
    /// Credits cap per owned planet (`Credit_Cap_Per_Planet`); 0 = no cap (the default: user rule, credits never expire).
    pub credit_cap_per_planet: f64,
    /// Resolve battles automatically at the end of the step instead of emitting
    /// `Event::BattlePending` and waiting for the caller.
    pub auto_resolve_battles: bool,
    /// Seconds between AI decisions.
    pub ai_interval_seconds: f32,
    /// Seconds simulated per auto-resolve combat round.
    pub combat_round_seconds: f32,
    /// Maximum auto-resolve rounds before a battle is declared a draw.
    pub combat_max_rounds: u32,
    /// Population cap of a faction before planets and bases add to it.
    pub base_population_cap: i32,
    /// Maximum queued items per production lane.
    pub max_queue_len: usize,
    /// Evaluate victory conditions (and end the game when one is met).
    pub victory_enabled: bool,
    /// "From the Ground Up" start (see [`crate::ground_up`]): the main factions start with 1 or 3 planets, the rest is passive.
    #[serde(default)]
    pub ground_up: bool,
    /// Allow `Command::DebugGrant` (offscreen tests and debugging only; off by default).
    pub debug_commands: bool,
    /// Estimate (calibrated against the tactical simulation, `examples/calibrate.rs`): how many random enemies a weapon compares before firing at the weakest. 1 = spread at random.
    pub combat_focus: u32,
    /// Estimate (calibrated against the tactical simulation): multiplier on hull, shield and damage of star bases, structures and garrison squadrons in the statistical resolver.
    pub combat_base_factor: f32,
    /// Estimate (calibrated against the tactical simulation): damage efficiency of fighters/bombers, corvettes, frigates and capital ships in the statistical resolver.
    pub combat_efficiency: [f32; 4],
    /// Estimate (measured on the tactical simulation, `examples/calibrate.rs`): share of the ships of a losing fleet that escape the statistical resolver by retreating.
    pub combat_escape: f32,
    /// Estimate (calibrated against the tactical simulation): (ships, exponent) of the damage reduction of very large fleets in the statistical resolver, see `combat::Tuning::crowd`.
    pub combat_crowd: (f32, f32),
    /// The statistical resolver respects `Space_Tactical_Unit_Cap` like the tactical battle: ships over the cap arrive in waves (a rule from the data, not an estimate).
    pub combat_waves: bool,
    /// How battles are resolved by [`crate::Sim::resolve_battle`] and by automatic resolution.
    pub resolver: BattleResolver,
    /// Longest tactical battle in simulated seconds before it is declared a draw.
    pub tactical_max_seconds: f32,
    /// `BattleResolver::Tactical` settles only battles with at most this many ships, structures and squadrons; bigger ones use the statistical resolver
    /// (a full headless battle of a hundred ships takes seconds of CPU; the soak test saw single battles of 25 s). Explicit `resolve_battle_tactical` ignores it.
    pub tactical_resolver_max_ships: u32,
    /// Estimate: share of hull restored per week at an owned planet without a star base (the data has no galactic repair rule).
    pub repair_base_per_week: f32,
    /// Estimate: extra share per week and star base level.
    pub repair_per_starbase_level_per_week: f32,
    /// Estimate: shields are restored this many times faster than hulls.
    pub repair_shield_factor: f32,
    /// Estimate: share of the unspent production cost returned when a queue entry is cancelled (1.0 = all of it; the data has no refund rule).
    pub cancel_refund: f32,
    /// Estimate: multiplier on `Planet_Capture_Bonus_Reward` (paid to a conqueror the first time it takes that planet; the data gives the amount, not the rule).
    pub capture_bonus_scale: f32,
    /// Estimate: seconds a faction's superweapons need to recharge after destroying a planet (the data has no galactic cooldown).
    pub superweapon_cooldown_seconds: f32,
    /// Fleet slot rules (at most `FLEET_SLOTS` fleets per planet, arrivals merge, splits need a free slot) also bind the AI factions (the AI task forces work within it).
    #[serde(default = "default_true")]
    pub fleet_slot_cap_ai: bool,
    /// Override the human player faction (e.g. for playing any faction in From the Ground Up).
    #[serde(default)]
    pub faction_override: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Algorithm used to settle battles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BattleResolver {
    /// Fast statistical rounds (`combat.rs`).
    #[default]
    Statistical,
    /// Simulate the tactical space battle headless at full speed with `battle-sim`. Needs a
    /// catalog from `Sim::set_tactical_catalog`; falls back to `Statistical` without one.
    Tactical,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            week_seconds: 60.0,
            tick_seconds: 1.0,
            galactic_speed_scale: 6.0,
            build_time_scale: 1.0,
            capture_delay_seconds: 5.0,
            credit_cap_per_planet: 0.0,
            auto_resolve_battles: true,
            ai_interval_seconds: 10.0,
            combat_round_seconds: 4.0,
            combat_max_rounds: 600,
            base_population_cap: 40,
            max_queue_len: 8,
            victory_enabled: true,
            ground_up: false,
            debug_commands: false,
            combat_focus: 1,
            combat_base_factor: 1.5,
            combat_efficiency: [0.5, 0.5, 1.0, 1.0],
            combat_escape: 0.12,
            combat_crowd: (25.0, 0.0),
            combat_waves: true,
            resolver: BattleResolver::Statistical,
            tactical_max_seconds: 3600.0,
            tactical_resolver_max_ships: 16,
            repair_base_per_week: 0.05,
            repair_per_starbase_level_per_week: 0.10,
            repair_shield_factor: 4.0,
            cancel_refund: 1.0,
            capture_bonus_scale: 1.0,
            superweapon_cooldown_seconds: 120.0,
            fleet_slot_cap_ai: true,
            faction_override: None,
        }
    }
}

/// A planet (or dummy location) of the campaign.
#[derive(Debug, Clone)]
pub struct PlanetInfo {
    /// Object name, e.g. `Coruscant_WW`.
    pub name: String,
    /// Localized display name.
    pub display_name: String,
    /// Galactic map position (x, y, z).
    pub position: [f32; 3],
    /// `Planet_Credit_Value`: base income per week.
    pub credit_value: f32,
    /// Structure slots in orbit.
    pub space_slots: i32,
    /// Structure slots on the ground.
    pub ground_slots: i32,
    /// `Additional_Population_Capacity` the planet itself grants its owner.
    pub population_capacity: i32,
    /// `Space_Tactical_Map` (.ted file name), the arena of space battles over this planet.
    pub space_map: Option<String>,
    /// `Planet_Surface_Accessible`: ground forces could land.
    pub surface_accessible: bool,
    /// `Planet_Capture_Bonus_Reward`: credits for the faction that conquers the planet.
    pub capture_bonus: f32,
    /// `Planet_Is_Mining_Colony`: income structures with `Additional_Multiplier_On_Mining_Colonies` pay more here.
    pub mining_colony: bool,
    /// `Max_Space_Base`: highest star base level the planet can get (3 on most planets, 1 on small ones; 0 = not given, no limit).
    pub max_space_base: i32,
    /// `Planet_Is_Destroyable_By_TSW` (a superweapon may destroy it). Planets without the tag count as destroyable (estimate).
    pub destroyable: bool,
    /// Innate production modifiers (shipyard bonuses from XML, e.g. Kuat, Rendili, Rothana).
    pub prod_mods: Vec<ProductionMod>,
}

/// A hyperspace lane.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    /// Route name.
    pub name: String,
    /// One end.
    pub a: PlanetId,
    /// Other end.
    pub b: PlanetId,
    /// Euclidean length in map units.
    pub length: f32,
    /// `HS_Speed_Factor`.
    pub hs_factor: f32,
}

/// A faction taking part in the campaign.
#[derive(Debug, Clone)]
pub struct FactionInfo {
    /// Faction name, e.g. `Empire`.
    pub name: String,
    /// RGBA color.
    pub color: [u8; 4],
    /// `Credits_Accumulation_Factor`.
    pub credits_factor: f32,
    /// Factions this one is at war with (everyone else is treated as hostile only if listed).
    pub enemies: HashSet<FactionId>,
    /// Starting credits from the campaign.
    pub start_credits: f64,
    /// Starting tech level.
    pub start_tech: i32,
    /// Maximum tech level.
    pub max_tech: i32,
    /// Home planet.
    pub home: Option<PlanetId>,
    /// `Space_Tactical_Unit_Cap`: population a side may have in a space battle (units beyond it wait in the reinforcement pool); 150 if the data has none.
    pub space_unit_cap: i32,
    /// Tech upgrade chain: `tech_upgrades[n]` raises tech from `start_tech + n` to `+ 1`.
    pub tech_upgrades: Vec<TechUpgrade>,
}

/// A purchasable tech level upgrade (`*_Level_*_Tech_Upgrade`).
#[derive(Debug, Clone)]
pub struct TechUpgrade {
    /// Object name.
    pub name: String,
    /// Campaign cost in credits.
    pub cost: f32,
    /// Build time in seconds.
    pub time: f32,
}

/// One weapon group of a combatant, aggregated per damage type.
#[derive(Debug, Clone, Copy)]
pub struct Weapon {
    /// Damage type index into [`DamageMatrix`], `None` if the damage type is unknown.
    pub damage_type: Option<u16>,
    /// Damage per second against hull (before armor multiplier).
    pub hull_dps: f32,
    /// Damage per second against shields (before armor multiplier).
    pub shield_dps: f32,
    /// Maximum range.
    pub range: f32,
}

/// Combat-relevant numbers of a ship, structure or station.
#[derive(Debug, Clone, Default)]
pub struct CombatStats {
    /// Hull points.
    pub hull: f32,
    /// Shield points.
    pub shield: f32,
    /// Shield regeneration per second.
    pub shield_regen: f32,
    /// Hull armor type index.
    pub armor: Option<u16>,
    /// Shield armor type index.
    pub shield_armor: Option<u16>,
    /// Weapons aggregated by damage type.
    pub weapons: Vec<Weapon>,
}

impl CombatStats {
    /// Raw damage per second ignoring armor.
    pub fn raw_dps(&self) -> f32 {
        self.weapons.iter().map(|w| w.hull_dps.max(w.shield_dps)).sum()
    }

    /// Hull plus shield points.
    pub fn durability(&self) -> f32 {
        self.hull + self.shield
    }

    /// Adds another unit's numbers (for squadrons made of several members).
    pub(crate) fn absorb(&mut self, other: &CombatStats) {
        self.hull += other.hull;
        self.shield += other.shield;
        self.shield_regen += other.shield_regen;
        if self.armor.is_none() {
            self.armor = other.armor;
            self.shield_armor = other.shield_armor;
        }
        for w in &other.weapons {
            self.add_weapon(*w);
        }
    }

    pub(crate) fn add_weapon(&mut self, w: Weapon) {
        if let Some(e) = self.weapons.iter_mut().find(|e| e.damage_type == w.damage_type) {
            e.hull_dps += w.hull_dps;
            e.shield_dps += w.shield_dps;
            e.range = e.range.max(w.range);
        } else {
            self.weapons.push(w);
        }
    }
}

/// What a catalog unit does when its construction finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitRole {
    /// A ship or squadron that joins a fleet.
    Ship,
    /// A research item: completing it unlocks the named object for the faction.
    Research(String),
}

/// A buildable ship, squadron or research item.
#[derive(Debug, Clone)]
pub struct UnitType {
    /// Object name.
    pub name: String,
    /// Display name text key.
    pub text_id: Option<String>,
    /// Ship or research item.
    pub role: UnitRole,
    /// Size class.
    pub category: ShipCategory,
    /// Lower-cased `Affiliation` faction names.
    pub affiliation: Vec<String>,
    /// Combat numbers (for squadrons: summed over members).
    pub combat: CombatStats,
    /// `Hyperspace_Speed`.
    pub hyperspace_speed: f32,
    /// Build cost in credits.
    pub cost: f32,
    /// Build time in seconds (unscaled).
    pub build_time: f32,
    /// Required faction tech level.
    pub tech: i32,
    /// Population used.
    pub population: i32,
    /// Population the unit counts in a space battle (`Space_Override_Population_Value`, else `population`).
    pub space_population: i32,
    /// Required star base level at the building planet.
    pub req_starbase: i32,
    /// `Required_Special_Structures`: every group must be met by one of its members (`A B` = both, `A | B` = either).
    pub req_structures: Vec<Vec<StructureTypeId>>,
    /// If not empty, the unit can only be built on these planets.
    pub req_planets: Vec<PlanetId>,
    /// `Build_Initially_Locked`.
    pub locked: bool,
    /// Max alive + queued per faction.
    pub limit_current: Option<i32>,
    /// Max ever built per faction.
    pub limit_lifetime: Option<i32>,
    /// `Is_Named_Hero`: story respawn timers apply and its loss is reported.
    pub hero: bool,
    /// `Reduce_Production_*_Ability` entries that apply to the owner's builds while the unit exists.
    pub prod_mods: Vec<ProductionMod>,
    /// Has a `Planet_Destruction_Ability` (Death Star, Galaxy Gun).
    pub planet_destroyer: bool,
    /// Has a `Galactic_Stealth_Ability`: fleets with the unit are hidden on the galaxy map from everyone who is not in the same system.
    pub galactic_stealth: bool,
    /// Has a `System_Spy_Ability`: the fleet learns everything about the planet it is at.
    pub system_spy: bool,
    /// Income change per week while the unit exists (negative = upkeep).
    pub upkeep: f32,
    /// `AI_Combat_Power`.
    pub ai_power: f32,
    /// True if the unit can appear in a build menu at all.
    pub buildable: bool,
    /// Fighter groups carried (name, count); informational.
    pub carries: Vec<(String, u32)>,
}

/// A planetary or orbital structure.
#[derive(Debug, Clone)]
pub struct StructureType {
    /// Object name.
    pub name: String,
    /// Display name text key.
    pub text_id: Option<String>,
    /// Lower-cased affiliation.
    pub affiliation: Vec<String>,
    /// Build cost.
    pub cost: f32,
    /// Build time in seconds (unscaled).
    pub build_time: f32,
    /// Required tech level.
    pub tech: i32,
    /// Required star base level.
    pub req_starbase: i32,
    /// `Required_Special_Structures`: every group must be met by one of its members (`A B` = both, `A | B` = either).
    pub req_structures: Vec<Vec<StructureTypeId>>,
    /// `Required_Planets`: if not empty, the structure can only be built on these planets (terrain variants, Starforge, refineries ...).
    pub req_planets: Vec<PlanetId>,
    /// Orbital structures always; a ground structure only if it affects the galactic game (income, population, space
    /// weapons or hardpoints, ship discounts, gambling income) or is a prerequisite of one. See `galaxy-sim/STATUS.md`.
    pub galactic: bool,
    /// Space weapons or hardpoints (orbital structures with hardpoints, or ground-to-space special weapons).
    pub space_reach: bool,
    /// Structure types this one counts as (its `Variant_Of_Existing_Type` ancestors), itself excluded.
    pub ancestors: Vec<StructureTypeId>,
    /// Absolute income change per week.
    pub income_abs: f32,
    /// Fractional income change of the planet's base income.
    pub income_pct: f32,
    /// Extra fractional income on a mining colony (`Percentage_Income_Modifier` x `Additional_Multiplier_On_Mining_Colonies`).
    pub income_pct_mining: f32,
    /// Extra absolute income on a mining colony.
    pub income_abs_mining: f32,
    /// `Planet_Income_Gambling_Ability` (Underworld cantinas): income that swings between a loss and a win.
    pub gamble: Option<IncomeGamble>,
    /// Population capacity granted.
    pub population_capacity: i32,
    /// True if it occupies an orbital slot, false for a ground slot.
    pub orbital: bool,
    /// Max instances per planet.
    pub max_per_planet: i32,
    /// `Build_Initially_Locked`.
    pub locked: bool,
    /// True if the structure appears in a build menu.
    pub buildable: bool,
    /// Combat numbers if the structure fights in space.
    pub combat: Option<CombatStats>,
    /// `Reduce_Production_*_Ability` entries that apply to builds at the planet it stands on.
    pub prod_mods: Vec<ProductionMod>,
}

/// `Planet_Income_Gambling_Ability`: every `interval` seconds the planet's income changes by `max` (with probability `chance`) or `min` times its base value.
/// The data gives the numbers, not the exact rule (estimate: the modifier is applied like `Percentage_Income_Modifier` until the next evaluation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IncomeGamble {
    /// `Evaluation_Interval_In_Secs`.
    pub interval: f32,
    /// `Min_Income_Modifier` (a loss, negative).
    pub min: f32,
    /// `Max_Income_Modifier`.
    pub max: f32,
    /// `Chance_To_Win`.
    pub chance: f32,
}

/// One `Reduce_Production_Price_Ability` or `Reduce_Production_Time_Ability`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductionMod {
    /// Ability name.
    pub name: String,
    /// Fraction taken off the price (0.2 = 20 % cheaper).
    pub price: f32,
    /// Fraction taken off the build time.
    pub time: f32,
    /// `Applicable_Unit_Categories` is `All`.
    pub all: bool,
    /// Applicable size classes.
    pub categories: Vec<alamo_data::ShipCategory>,
    /// `Applicable_Unit_Types`, lower-cased (empty = no type filter).
    pub types: Vec<String>,
    /// `Excluded_Unit_Types`, lower-cased.
    pub excluded: Vec<String>,
    /// `Stacking_Category`: of several abilities in one category only the strongest counts.
    pub stacking: i32,
}

impl ProductionMod {
    /// True if the ability applies to a unit of this name and size class.
    pub fn applies(&self, name: &str, category: alamo_data::ShipCategory) -> bool {
        let lower = name.to_ascii_lowercase();
        if self.excluded.contains(&lower) {
            return false;
        }
        if self.types.contains(&lower) {
            return true;
        }
        (self.all && self.types.is_empty()) || self.categories.contains(&category)
    }
}

/// One level of the orbital star base.
#[derive(Debug, Clone)]
pub struct StarbaseType {
    /// Object name.
    pub name: String,
    /// Lower-cased affiliation.
    pub affiliation: Vec<String>,
    /// Level 1-5.
    pub level: i32,
    /// Next level.
    pub next: Option<StarbaseTypeId>,
    /// Cost of building/upgrading to this level.
    pub cost: f32,
    /// Build time in seconds (unscaled).
    pub build_time: f32,
    /// Required tech level.
    pub tech: i32,
    /// Population capacity.
    pub population_capacity: i32,
    /// Absolute income change per week.
    pub income_abs: f32,
    /// Fractional income change.
    pub income_pct: f32,
    /// Combat numbers.
    pub combat: CombatStats,
    /// Fighter garrison spawned in battle (unit type, count).
    pub garrison: Vec<(UnitTypeId, u32)>,
    /// Production modifiers of this star base level.
    pub prod_mods: Vec<ProductionMod>,
}

/// Damage-type versus armor-type multipliers with dense indices.
#[derive(Debug, Clone, Default)]
pub struct DamageMatrix {
    damage_index: HashMap<String, u16>,
    armor_index: HashMap<String, u16>,
    armors: usize,
    mult: Vec<f32>,
}

impl DamageMatrix {
    fn build(data: &GameData) -> Self {
        let mut m = DamageMatrix::default();
        for (i, d) in data.combat.damage_types.iter().enumerate() {
            m.damage_index.insert(d.to_ascii_lowercase(), i as u16);
        }
        for (i, a) in data.combat.armor_types.iter().enumerate() {
            m.armor_index.insert(a.to_ascii_lowercase(), i as u16);
        }
        m.armors = data.combat.armor_types.len();
        m.mult = vec![1.0; data.combat.damage_types.len() * m.armors.max(1)];
        for (d, di) in &m.damage_index {
            for (a, ai) in &m.armor_index {
                m.mult[*di as usize * m.armors + *ai as usize] = data.combat.multiplier(d, a);
            }
        }
        m
    }

    /// Index of a damage type name.
    pub fn damage(&self, name: &str) -> Option<u16> {
        self.damage_index.get(&name.trim().to_ascii_lowercase()).copied()
    }

    /// Index of an armor type name.
    pub fn armor(&self, name: &str) -> Option<u16> {
        self.armor_index.get(&name.trim().to_ascii_lowercase()).copied()
    }

    /// Multiplier of `damage` against `armor`; 1.0 if either is unknown.
    pub fn multiplier(&self, damage: Option<u16>, armor: Option<u16>) -> f32 {
        match (damage, armor) {
            (Some(d), Some(a)) => self.mult[d as usize * self.armors + a as usize],
            _ => 1.0,
        }
    }
}

/// How a faction (or the human player) wins.
#[derive(Debug, Clone, Default)]
pub struct VictoryRules {
    /// Faction controlled by the human in this campaign (`Starting_Active_Player`).
    pub active_player: Option<FactionId>,
    /// Human wins by holding all these planets.
    pub human_named_planets: Vec<PlanetId>,
    /// Human wins by holding every real planet.
    pub human_all_planets: bool,
    /// AI factions win by holding every real planet.
    pub ai_all_planets: bool,
    /// AI factions win by holding these planets (if non-empty).
    pub ai_named_planets: Vec<PlanetId>,
}

/// What kind of game object a name refers to, as far as the simulation is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A ship or squadron in the unit catalog.
    SpaceUnit,
    /// A star base level.
    Starbase,
    /// A planetary or orbital structure in the catalog.
    Structure,
    /// A ground unit, ground company or ground building: exists in the data, is not simulated.
    Ground,
    /// A hero or hero team: exists in the data, is not simulated.
    Hero,
    /// Anything else (props, markers, unknown names).
    Other,
}

/// Immutable world description.
#[derive(Debug)]
pub struct World {
    /// Constants this world was built with.
    pub config: SimConfig,
    /// Campaign name.
    pub campaign_name: String,
    /// Planets (including dummy art locations, see [`World::is_real`]).
    pub planets: Vec<PlanetInfo>,
    /// For each planet: whether it is a real planet.
    pub real: Vec<bool>,
    /// Lower-cased planet name to id.
    pub planet_index: HashMap<String, PlanetId>,
    /// Hyperspace lanes.
    pub routes: Vec<RouteInfo>,
    /// For each planet: `(neighbor, route index)`.
    pub adjacency: Vec<Vec<(PlanetId, u32)>>,
    /// Factions.
    pub factions: Vec<FactionInfo>,
    /// Lower-cased faction name to id.
    pub faction_index: HashMap<String, FactionId>,
    /// Unit catalog.
    pub units: Vec<UnitType>,
    /// Lower-cased unit name to id.
    pub unit_index: HashMap<String, UnitTypeId>,
    /// Structure catalog.
    pub structures: Vec<StructureType>,
    /// Lower-cased structure name to id.
    pub structure_index: HashMap<String, StructureTypeId>,
    /// Star base levels.
    pub starbases: Vec<StarbaseType>,
    /// Lower-cased star base name to id.
    pub starbase_index: HashMap<String, StarbaseTypeId>,
    /// For each faction: units it may build (affiliation matches, appears in a build menu).
    pub buildable_units: Vec<Vec<UnitTypeId>>,
    /// For each faction: structures it may build.
    pub buildable_structures: Vec<Vec<StructureTypeId>>,
    /// For each structure: number of buildable units that list it as a requirement.
    pub structure_unlocks: Vec<u32>,
    /// For each faction: level-1 star base.
    pub first_starbase: Vec<Option<StarbaseTypeId>>,
    /// Damage/armor multipliers.
    pub damage: DamageMatrix,
    /// Victory conditions.
    pub victory: VictoryRules,
    /// The main (great power) factions: the human player and every faction a full AI plays, not the minors or the passive faction
    /// (see [`World::build_main_factions`]).
    pub main_factions: Vec<FactionId>,
    /// Ground units, heroes and other objects that are named in the data but not simulated
    /// (lower-case name to kind). Ships, structures and star bases are looked up in their catalogs.
    pub unsimulated: HashMap<String, ObjectKind>,
}

/// Why a world could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimError {
    /// The campaign has no locations (menu-only entry).
    NoGalaxy,
    /// No planet of the campaign could be resolved.
    NoPlanets,
}

impl std::fmt::Display for SimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimError::NoGalaxy => write!(f, "campaign defines no galaxy"),
            SimError::NoPlanets => write!(f, "no planets could be resolved"),
        }
    }
}

impl std::error::Error for SimError {}

fn key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

impl World {
    /// True if the fleet slot rules (see [`crate::FLEET_SLOTS`]) bind `faction`: the human player always, the AI factions with `SimConfig::fleet_slot_cap_ai`.
    pub fn slot_limited(&self, faction: FactionId) -> bool {
        self.config.fleet_slot_cap_ai || self.victory.active_player == Some(faction)
    }

    /// Builds the world for `campaign`.
    pub fn build(data: &GameData, campaign: &Campaign, config: SimConfig) -> Result<World, SimError> {
        if !campaign.has_galaxy() {
            return Err(SimError::NoGalaxy);
        }
        let damage = DamageMatrix::build(data);

        // Planets, in `Locations` order so ids are stable.
        let mut planets = Vec::new();
        let mut real = Vec::new();
        let mut planet_index = HashMap::new();
        for name in &campaign.locations {
            let Some(p) = data.planet(name) else { continue };
            if planet_index.contains_key(&key(&p.name)) {
                continue;
            }
            planet_index.insert(key(&p.name), planets.len() as PlanetId);
            real.push(p.is_real_planet());
            let pop = p.fields.get("additional_population_capacity").and_then(|v| v.parse().ok()).unwrap_or(0);
            planets.push(PlanetInfo {
                name: p.name.clone(),
                display_name: data.planet_display_name(p).to_string(),
                position: p.position,
                credit_value: p.credit_value as f32,
                space_slots: p.special_structures_space,
                max_space_base: p.max_space_base,
                mining_colony: p.fields.get("planet_is_mining_colony").is_some_and(|v| alamo_data::xml::parse_bool(v)),
                ground_slots: p.special_structures_land,
                population_capacity: pop,
                space_map: p.space_tactical_map.clone(),
                surface_accessible: p.surface_accessible,
                capture_bonus: p.fields.get("planet_capture_bonus_reward").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                destroyable: p.fields.get("planet_is_destroyable_by_tsw").is_none_or(|v| alamo_data::xml::parse_bool(v)),
                prod_mods: Vec::new(),
            });
        }
        if planets.is_empty() {
            return Err(SimError::NoPlanets);
        }

        // Hyperspace lanes between real planets.
        let mut routes = Vec::new();
        let mut adjacency = vec![Vec::new(); planets.len()];
        for r in data.campaign_trade_routes(campaign) {
            let (Some(&a), Some(&b)) = (planet_index.get(&key(&r.point_a)), planet_index.get(&key(&r.point_b))) else {
                continue;
            };
            if a == b || !real[a as usize] || !real[b as usize] {
                continue;
            }
            let (pa, pb) = (planets[a as usize].position, planets[b as usize].position);
            let length = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt().max(1.0);
            let idx = routes.len() as u32;
            routes.push(RouteInfo {
                name: r.name.clone(),
                a,
                b,
                length,
                hs_factor: if r.hs_speed_factor > 0.0 { r.hs_speed_factor } else { 1.0 },
            });
            adjacency[a as usize].push((b, idx));
            adjacency[b as usize].push((a, idx));
        }

        // Factions of this campaign.
        let mut factions: Vec<FactionInfo> = Vec::new();
        let mut faction_index = HashMap::new();
        for cf in &campaign.factions {
            let Some(def) = data.faction(&cf.name) else { continue };
            faction_index.insert(key(&cf.name), factions.len() as FactionId);
            factions.push(FactionInfo {
                name: def.name.clone(),
                color: def.color.unwrap_or([128, 128, 128, 255]),
                credits_factor: if def.credits_accumulation_factor > 0.0 { def.credits_accumulation_factor } else { 1.0 },
                enemies: HashSet::new(),
                start_credits: cf.starting_credits.unwrap_or(0) as f64,
                start_tech: cf.starting_tech_level.unwrap_or(1),
                max_tech: cf.max_tech_level.unwrap_or(5),
                home: cf.home_location.as_ref().and_then(|h| planet_index.get(&key(h)).copied()),
                tech_upgrades: Vec::new(),
                // User rule: 1000 population per side in a space battle, so super ships (Purifier and Annihilator together) fit.
                // The data's `Space_Tactical_Unit_Cap` (180 in Remake 4.0) is replaced on purpose.
                space_unit_cap: USER_SPACE_UNIT_CAP.max(def.space_unit_cap),
            });
        }
        for (i, f) in factions.iter_mut().enumerate() {
            let Some(def) = data.faction(&f.name) else { continue };
            f.enemies = def
                .enemies
                .iter()
                .filter_map(|e| faction_index.get(&key(e)).copied())
                .filter(|&e| e as usize != i)
                .collect();
        }
        // Everyone not explicitly listed stays hostile too: the Remake lists all factions as enemies.
        let n = factions.len() as FactionId;
        for i in 0..n {
            for j in 0..n {
                if i != j && factions[i as usize].enemies.contains(&j) {
                    factions[j as usize].enemies.insert(i);
                }
            }
        }

        let mut world = World {
            config,
            campaign_name: campaign.name.clone(),
            planets,
            real,
            planet_index,
            routes,
            adjacency,
            factions,
            faction_index,
            units: Vec::new(),
            unit_index: HashMap::new(),
            structures: Vec::new(),
            structure_index: HashMap::new(),
            starbases: Vec::new(),
            starbase_index: HashMap::new(),
            buildable_units: Vec::new(),
            buildable_structures: Vec::new(),
            structure_unlocks: Vec::new(),
            first_starbase: Vec::new(),
            damage,
            victory: VictoryRules::default(),
            main_factions: Vec::new(),
            unsimulated: data
                .objects
                .iter()
                .filter_map(|o| {
                    let k = o.kind.to_ascii_lowercase();
                    let kind = if k.contains("hero") {
                        ObjectKind::Hero
                    } else if k.starts_with("ground") || k == "transportunit" || k == "indigenous_unit" || k == "landbombingunit" {
                        ObjectKind::Ground
                    } else {
                        return None;
                    };
                    Some((o.name.to_ascii_lowercase(), kind))
                })
                .collect(),
        };
        world.build_catalogs(data);
        world.build_tech_upgrades(data);
        world.build_victory(campaign);
        world.build_main_factions(data, campaign);
        Ok(world)
    }
}
