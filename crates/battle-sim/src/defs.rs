//! Immutable battle definitions (ships, hardpoints, projectiles, damage matrix) built from the
//! game data and, optionally, the `.alo` models for hardpoint positions.
//!
//! # Units and conventions
//! * Battle space: X forward at yaw 0, Y left, Z up, distances in the game's world units.
//! * Speeds and projectile speeds in the XML are per "frame"; they are multiplied by
//!   [`Catalog::speed_scale`] (default 30, the 30 Hz logic tick) to get units per second. Turn rates are per second.
//! * Model space is Z-up with the ship's bow along -Y; it is converted to battle space with
//!   `(x, y, z) -> (-y, x, z)`. A hardpoint fires along its bone's local +X axis.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use alamo_data::{GameData, HardpointDef, ProjectileDef, ShipCategory, SpaceUnitDef, StarBaseDef, StructureDef};
use alamo_fs::FileSource;
use rayon::prelude::*;

use crate::math::Vec3;

/// Index of a ship definition in [`Catalog::ships`].
pub type ShipId = u32;
/// Index of a projectile definition in [`Catalog::projectiles`].
pub type ProjId = u32;

/// Coarse target class used by weapon restrictions and inaccuracy tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum TargetClass {
    /// Fighter.
    Fighter = 0,
    /// Bomber.
    Bomber = 1,
    /// Transport.
    Transport = 2,
    /// Corvette / gunship.
    Corvette = 3,
    /// Frigate.
    Frigate = 4,
    /// Capital ship.
    Capital = 5,
    /// Super capital ship.
    Super = 6,
    /// Space station / structure.
    Structure = 7,
    /// Anything else.
    #[default]
    Other = 8,
}

impl TargetClass {
    /// Number of classes.
    pub const COUNT: usize = 9;

    /// Parses a category token such as `Fighter` or `SpaceStructure`.
    pub fn from_token(t: &str) -> Option<Self> {
        Some(match t.to_ascii_lowercase().as_str() {
            "fighter" => Self::Fighter,
            "bomber" => Self::Bomber,
            "transport" => Self::Transport,
            "corvette" | "gunship" => Self::Corvette,
            "frigate" => Self::Frigate,
            "capital" => Self::Capital,
            "super" | "supercapital" => Self::Super,
            "spacestructure" | "structure" => Self::Structure,
            _ => return None,
        })
    }

    fn from_category(c: ShipCategory) -> Self {
        match c {
            ShipCategory::Fighter => Self::Fighter,
            ShipCategory::Bomber => Self::Bomber,
            ShipCategory::Transport => Self::Transport,
            ShipCategory::Corvette => Self::Corvette,
            ShipCategory::Frigate => Self::Frigate,
            ShipCategory::Capital => Self::Capital,
            ShipCategory::Super => Self::Super,
            ShipCategory::SpaceStructure => Self::Structure,
            ShipCategory::Other => Self::Other,
        }
    }

    /// Bit for class masks.
    pub fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// What a hardpoint does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HpKind {
    /// Fires a damaging projectile.
    Weapon,
    /// Dummy weapon that only makes the ship turn its bow towards the target.
    Facing,
    /// Shield generator: while alive the shield regenerates, when destroyed the shield collapses.
    ShieldGenerator,
    /// Engine: destroying engines slows the ship.
    Engine,
    /// Hangar: needed to launch and replace squadrons.
    FighterBay,
    /// Tractor beam emitter.
    TractorBeam,
    /// Anything else (art, ability enablers).
    #[default]
    Other,
}

/// A hardpoint of a ship definition.
#[derive(Debug, Clone, Default)]
pub struct HpDef {
    /// Object name.
    pub name: String,
    /// Function.
    pub kind: HpKind,
    /// Hit points; 0 for indestructible hardpoints.
    pub health: f32,
    /// `Is_Destroyable`.
    pub destroyable: bool,
    /// `Is_Targetable` (can be aimed at explicitly).
    pub targetable: bool,
    /// Position in ship-local battle space.
    pub offset: Vec3,
    /// Firing direction in ship-local battle space (unit vector).
    pub forward: Vec3,
    /// Mount normal (bone Z axis, orthogonalized to `forward`) in ship-local battle space.
    pub normal: Vec3,
    /// `normal x forward`: the mount's left direction.
    pub left: Vec3,
    /// Effective horizontal arc (radians, half-angle around `forward` in the mount frame): the larger of
    /// the fire cone and the turret rotation extent.
    pub arc_yaw: f32,
    /// Effective vertical arc (half-angle) in the mount frame.
    pub arc_pitch: f32,
    /// True if the bone orientation gave a mount that cannot reach the battle plane, so the arc was
    /// re-centered on the ship's forward axis.
    pub arc_fallback: bool,
    /// True for turrets (`Is_Turret`): they rotate to aim within their extents.
    pub turret: bool,
    /// Turret rotation extent (`Turret_Rotate_Extent_Degrees`) in radians.
    pub turret_yaw: f32,
    /// Turret elevation extent (`Turret_Elevate_Extent_Degrees`) in radians.
    pub turret_pitch: f32,
    /// Projectile fired.
    pub projectile: Option<ProjId>,
    /// Maximum range.
    pub range: f32,
    /// Minimum range.
    pub min_range: f32,
    /// Half of `Fire_Cone_Width` in radians (horizontal arc around `forward`).
    pub half_yaw: f32,
    /// Half of `Fire_Cone_Height` in radians (vertical arc around `forward`).
    pub half_pitch: f32,
    /// Lower bound of the recharge between volleys, seconds.
    pub recharge_min: f32,
    /// Upper bound of the recharge between volleys, seconds.
    pub recharge_max: f32,
    /// Projectiles per volley.
    pub pulses: u32,
    /// Delay between pulses of a volley, seconds.
    pub pulse_delay: f32,
    /// Aim error in world units per target class.
    pub inaccuracy: [f32; TargetClass::COUNT],
    /// Bit mask of target classes the weapon may fire at (0 = any); the inverse of `Fire_Category_Restrictions`.
    pub allowed: u16,
    /// `Full_Salvo_Weapon_Delay_Multiplier`.
    pub full_salvo_mult: f32,
    /// `Power_To_Weapons_Mode_Weapon_Delay_Multiplier`.
    pub power_weapons_mult: f32,
    /// True if the mount bone has a mirrored (left-handed) transform.
    pub mirrored: bool,
    /// Bone Y axis in ship-local battle space (diagnostics).
    pub bone_y: Vec3,
    /// True if a bone gave the position (otherwise it was estimated).
    pub from_bone: bool,
    /// `Fighter_Bay_Flyout_Distance` of a fighter bay: squadrons launched from it fly this far straight out of the hangar before they take orders.
    pub flyout: f32,
    /// Weapons: the bins of `ShipDef::bearing_dps` (bit k = bearing k x 15 degrees) the weapon can bear on, and its damage per second there.
    pub arc_bins: u32,
    pub dps: f32,
}

/// Active ability with `Mod_Multiplier` effects.
#[derive(Debug, Clone, Default)]
pub struct AbilityDef {
    /// `Type`, e.g. `SPRINT`, `FULL_SALVO`.
    pub kind: String,
    /// Speed multiplier while active.
    pub speed_mult: f32,
    /// Shield regeneration multiplier while active.
    pub shield_regen_mult: f32,
    /// Weapon delay multiplier while active (below 1 = faster fire).
    pub weapon_delay_mult: f32,
    /// Incoming damage multiplier while active.
    pub take_damage_mult: f32,
    /// Outgoing damage multiplier while active.
    pub cause_damage_mult: f32,
    /// Seconds until reusable.
    pub recharge: f32,
    /// Seconds the effect lasts.
    pub duration: f32,
    /// True for `FULL_SALVO`, whose effect is defined per hardpoint.
    pub salvo: bool,
    /// `ROCKET_ATTACK` / `SWAP_WEAPONS` (missile boats): for `duration` seconds the missile, rocket and torpedo hardpoints reload
    /// at `ROCKET_RELOAD_MULT` of their normal delay (estimate: the data names the ability but gives no numbers).
    pub rocket: bool,
    /// `Supports_Autofire`: the player may set the ability to automatic use.
    pub supports_autofire: bool,
    /// `Active_By_Default`: automatic use starts on.
    pub autocast_default: bool,
    /// `CORRUPT_SYSTEMS`: aimed with `Command::UseAbilityAt` at an enemy ship within `max_range`; its shield drains for the duration.
    pub corrupt: Option<CorruptDef>,
    /// `SENSOR_JAMMING` (flares): while active, enemy homing missiles aimed at this ship lose their lock and fly straight on.
    pub jam: bool,
    /// `SUPER_LASER`: aimed with `Command::UseAbilityAt`; the chosen ship is destroyed outright after the charge time (estimate: the data
    /// gives `Damage_Per_Frame` 10, which cannot be what kills a capital ship in one shot in the game).
    pub super_laser: Option<SuperLaserDef>,
    /// `BUZZ_DROIDS` that damages enemies in an area (estimate: the area lasts `duration`, 20 s where the data gives no expiration; the
    /// `Own_Damage_Per_Second` of 1 and the repair-drone variants are not modelled).
    pub zone: Option<ZoneDef>,
    /// `MISSILE_SHIELD`: radius in which enemy missiles and torpedoes are destroyed (0 = not a missile shield).
    pub missile_radius: f32,
    /// `Disable_Movement`: the ship cannot move while the ability is active.
    pub disable_movement: bool,
    /// `INTERDICT`: while active, enemies cannot retreat.
    pub interdict: bool,
    /// `HUNT` ("Hunt for Enemies"): while on, the ship leaves its formation, searches the map and chases the enemies it detects.
    pub hunt: bool,
    /// `REPLENISH_WINGMEN` (Vader's and the 181st's squadron leaders): once, brings back every lost wingman of the leader's squadron.
    pub replenish: bool,
    /// `WEAKEN_ENEMY`: the weakening area.
    pub weaken: Option<WeakenDef>,
    /// `LURE`: the taunt area.
    pub lure: Option<LureDef>,
    /// `HARMONIC_BOMB`: what the bomb does.
    pub bomb: Option<BombDef>,
    /// `ENERGY_WEAPON`: the proton beam.
    pub energy: Option<EnergyDef>,
    /// `CLUSTER_BOMB`: the ring of bombs.
    pub cluster: Option<ClusterDef>,
    /// Abilities with `Expiration_Seconds` 0 (missile shield, interdiction) stay on until toggled off.
    pub toggle: bool,
    /// `TRACTOR_BEAM`: beam parameters from the linked `Tractor_Beam_Attack_Ability`.
    pub tractor: Option<TractorDef>,
    /// `STEALTH`: while active the ship is hidden from enemies unless one of them is within `BattleConfig::stealth_detection`.
    pub stealth: bool,
    /// `FOW_REVEAL_PING`: radius of the area it reveals to the owner for the duration (`Effective_Radius`; 0 = not a reveal ping).
    pub reveal_radius: f32,
}

/// A `Force_Healing_Ability` with `Space_Automatic` activation (astromech self repair, repair droids, station repair).
#[derive(Debug, Clone, Default)]
pub struct HealDef {
    /// `Heal_Range`: 0 = the ship itself, otherwise friendly ships within this distance.
    pub range: f32,
    /// `Heal_Amount` hull points per pulse.
    pub amount: f32,
    /// `Heal_Percent`: fraction of the maximum hull per pulse.
    pub percent: f32,
    /// `Heal_Interval_In_Secs`.
    pub interval: f32,
    /// `Single_Target_Heal`: only the most damaged ship in range.
    pub single: bool,
    /// Bit mask of healed classes (`Applicable_Unit_Categories`; 0 = any).
    pub classes: u16,
}

/// A `Combat_Bonus_Ability` with `Space_Automatic` activation: an aura that boosts the friendly ships of some classes
/// (the data gives no radius, so it reaches the whole side). Percentages are fractions (0.25 = +25%).
#[derive(Debug, Clone, Default)]
pub struct BonusDef {
    /// `Damage_Bonus_Percentage`: damage dealt.
    pub damage: f32,
    /// `Health_Bonus_Percentage`: hull damage taken is divided by `1 + health`.
    pub health: f32,
    /// `Shield_Bonus_Percentage`: shield damage taken is divided by `1 + shield` (negative = weaker shields).
    pub shield: f32,
    /// `Defense_Bonus_Percentage`: all damage taken is divided by `1 + defense`.
    pub defense: f32,
    /// `Fire_Range_Bonus_Percentage`: weapon reach.
    pub range: f32,
    /// `Movement_Speed_Bonus_Percentage`: top speed.
    pub speed: f32,
    /// Bit mask of boosted classes (`Applicable_Unit_Categories`; 0 = any).
    pub classes: u16,
    /// `Stacking_Category`: bonuses of one category do not add up, the strongest applies.
    pub stacking: u8,
}

/// A `Redirect_Blaster_Ability` that is on from the start (lightsaber heroes, reflector shields): incoming shots are
/// blocked or sent back at the shooter.
#[derive(Debug, Clone, Default)]
pub struct RedirectDef {
    /// `Block_Chance`: the shot is destroyed.
    pub block: f32,
    /// `Redirect_Chance`: the shot hits its shooter instead (estimate: the shot is not re-aimed, its damage is dealt to the shooter).
    pub redirect: f32,
    /// Half of `Reaction_Arc_In_Degrees` in radians, around the ship's heading.
    pub half_arc: f32,
    /// Bit mask of shooter classes it reacts to (`Applicable_Unit_Categories`; 0 = any).
    pub classes: u16,
}

/// `ENERGY_WEAPON` ("Proton Beam", Piett and the Accuser line): a beam on one enemy ship that does massive hull damage for the ability duration.
#[derive(Debug, Clone, Default)]
pub struct EnergyDef {
    /// `Activation_Max_Range` of the linked `Energy_Weapon_Attack_Ability`.
    pub max_range: f32,
    /// `Damage_Per_Frame` times 30 frames per second (estimate for the frame rate): hull damage per second.
    pub dps: f32,
    /// Bit mask of the targetable classes (`Applicable_Unit_Categories`; 0 = any).
    pub classes: u16,
}

/// `CLUSTER_BOMB` (Javelin heroes): after `delay` seconds `count` bombs have flown outward from the ship and go off, each doing
/// shield damage in its blast radius (`Proj_Ion_Cluster_Bomb`: no hull damage).
#[derive(Debug, Clone, Default)]
pub struct ClusterDef {
    /// `Detonation_Time_In_Secs`.
    pub delay: f32,
    /// `Number_Of_Bombs`.
    pub count: u32,
    /// Bomb `Projectile_Blast_Area_Range`.
    pub radius: f32,
    /// Bomb `Projectile_Blast_Area_Damage` (shield points).
    pub damage: f32,
    /// Distance the bombs have flown from the ship at detonation: bomb speed x delay.
    pub spread: f32,
}

/// `HARMONIC_BOMB`: the ship drops the ability's `Spawned_Object_Type` where it is; after `countdown` seconds the object goes off.
/// Two kinds in the 4.0 data: an instant heal blast for fighters (carriers, `Projectile_Instant_Heal_*`) and a seismic bomb (Slave I,
/// `Projectile_Blast_Area_*`).
#[derive(Debug, Clone, Default)]
pub struct BombDef {
    /// `Bomb_Countdown_Seconds`.
    pub countdown: f32,
    /// `Projectile_Instant_Heal_Radius`.
    pub heal_radius: f32,
    /// `Projectile_Instant_Heal_Health_Increase`: hull points given to each friendly ship of `heal_classes` in the radius.
    pub heal_amount: f32,
    /// `Projectile_Instant_Heal_Targets_Category_Mask` as a class bit mask (0 = any).
    pub heal_classes: u16,
    /// `Projectile_Blast_Area_Range`.
    pub blast_radius: f32,
    /// `Projectile_Blast_Area_Damage` at the centre.
    pub blast_damage: f32,
    /// `Projectile_Blast_Area_Dropoff_Tiers`: the damage falls in this many equal steps towards the edge (0 = no dropoff).
    pub blast_tiers: u32,
}

/// `LURE` ("Lure Enemy Fighters" / "Fortify"): while active the caster takes less damage (the ability's `TAKE_DAMAGE_MULTIPLIER`) and the enemy ships
/// of the listed types within `radius` are forced to attack it. The data pulses every 0.5 s for the duration.
#[derive(Debug, Clone, Default)]
pub struct LureDef {
    /// `Effective_Radius`.
    pub radius: f32,
    /// `Target_Types` (lower case ship names; empty = every enemy).
    pub types: Vec<String>,
}

/// `WEAKEN_ENEMY` (Thrawn, Zsinj): the `Projectile_Weaken_Enemy_*` values of the ability's `Spawned_Object_Type`. A targeted area: for `duration`
/// seconds the enemy ships inside deal `cause_reduction` less damage and fly `speed_cut` slower.
#[derive(Debug, Clone, Default)]
pub struct WeakenDef {
    /// Area radius (the ability's `Effective_Radius`, else `Projectile_Weaken_Enemy_Radius`).
    pub radius: f32,
    /// `Projectile_Weaken_Enemy_Duration_Seconds`.
    pub duration: f32,
    /// `Projectile_Weaken_Enemy_Cause_Damage_Reduction_Percent` (0.25 = 25% less damage dealt).
    pub cause_reduction: f32,
    /// `Projectile_Weaken_Enemy_Take_Damage_Increase_Percent`.
    pub take_increase: f32,
    /// Speed reduction of the projectile's `COMBAT_MOD_SPEED` (0.25 = 25% slower).
    pub speed_cut: f32,
}

/// `BUZZ_DROIDS` with a `Buzz_Droids_Ability` that hurts enemies (Eclipse lightning storm): a damaging area placed where the ability is used.
#[derive(Debug, Clone, Default)]
pub struct ZoneDef {
    /// `Damage_Radius`.
    pub radius: f32,
    /// `Enemy_Damage_Per_Second`: hull damage per second to every enemy ship inside (shields absorb first).
    pub enemy_dps: f32,
    /// `Activation_Max_Range`: how far from the ship the area may be placed.
    pub max_range: f32,
}

/// `SUPER_LASER` (Death Star): a shot at one chosen ship.
#[derive(Debug, Clone, Default)]
pub struct SuperLaserDef {
    /// `Fire_Frame` / 30: seconds between the order and the shot.
    pub delay: f32,
    /// Bit mask of ships it may be aimed at (`Applicable_Unit_Categories`; 0 = any).
    pub classes: u16,
    /// Seconds before the first shot: the Death Star flies into range first (estimate, see `TSW_SPEED_SCALE`); 0 for super lasers without that data.
    pub initial_cooldown: f32,
}

/// What a `Projectile_Combat_Mod` changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    /// `COMBAT_MOD_SPEED`: top speed.
    Speed,
    /// `COMBAT_MOD_FIRE_RATE`: shots per second.
    FireRate,
    /// `COMBAT_MOD_DEFENSE`: all damage taken.
    Defense,
    /// `COMBAT_MOD_HEALTH`: hull damage taken.
    Health,
    /// `COMBAT_MOD_SHIELD`: shield damage taken.
    Shield,
    /// `Projectile_Weaken_Enemy_Cause_Damage_Reduction_Percent` (as a negative value): damage the ship deals.
    CauseDamage,
}

impl ModKind {
    pub(crate) const COUNT: usize = 6;
}

/// A `Projectile_Combat_Mod`: for `duration` seconds the ships hit (and those within the blast range of the impact)
/// have `kind` changed by `value` (-0.25 = 25% less speed / fire rate / defense / health; damage taken is divided by `1 + value`).
/// Negative values hit enemies of the shooter, positive ones help its friends. The strongest value of a kind counts, they do not stack.
#[derive(Debug, Clone)]
pub struct CombatMod {
    /// What changes.
    pub kind: ModKind,
    /// `Projectile_Combat_Mod_Value`.
    pub value: f32,
    /// `Projectile_Combat_Mod_Duration` in seconds.
    pub duration: f32,
    /// Bit mask of affected classes (`Projectile_Combat_Mod_Category_Mask`; 0 = any).
    pub classes: u16,
}

/// `CORRUPT_SYSTEMS` (IG-88 and the CHG): drains the shields of one enemy ship for a while.
#[derive(Debug, Clone, Default)]
pub struct CorruptDef {
    /// `Activation_Max_Range` from the caster.
    pub max_range: f32,
    /// `Duration_In_Seconds`.
    pub duration: f32,
    /// `Shield_Drain_Per_Second`.
    pub drain: f32,
    /// `Inaccuracy_Radius`: how far from the target ship the chosen point may be.
    pub pick_radius: f32,
}

/// Parameters of a tractor beam ability.
#[derive(Debug, Clone, Default)]
pub struct TractorDef {
    /// `Activation_Max_Range`.
    pub range: f32,
    /// `Target_Speed_Decrease_Percent` as a fraction (0.75 = 75% slower).
    pub slow: f32,
    /// Bit mask of target classes (`Applicable_Unit_Categories`; 0 = any).
    pub classes: u16,
}

/// Squadrons at one owner tech level: `(tech, launched at the start, in reserve)`.
pub type SpawnedAt = (u8, Vec<(ShipId, u32)>, Vec<(ShipId, u32)>);

/// A ship, fighter, station or structure.
#[derive(Debug, Clone, Default)]
pub struct ShipDef {
    /// Object name.
    pub name: String,
    /// Display name key.
    pub text_id: Option<String>,
    /// Target class.
    pub class: TargetClass,
    /// Automatic healing abilities (`Force_Healing_Ability`).
    pub heals: Vec<HealDef>,
    /// Shot blocking (`Redirect_Blaster_Ability`).
    pub redirects: Vec<RedirectDef>,
    /// Aura bonuses given to the friendly fleet (`Combat_Bonus_Ability`).
    pub bonuses: Vec<BonusDef>,
    /// `Space_FOW_Reveal_Range`: how far this ship sees (fog of war); 1250 where the data gives none.
    pub sensor_range: f32,
    /// `Reinforcement_Prevention_Radius` of a structure: enemy reinforcements may not arrive this close (0 = the data has none; `BattleConfig::no_spawn_radius` applies).
    pub no_spawn_radius: f32,
    /// True for stations and structures (never move).
    pub is_static: bool,
    /// True for `Squadron` definitions, which expand into `members` when spawned.
    pub is_squadron: bool,
    /// Hull points.
    pub hull: f32,
    /// Shield points.
    pub shield: f32,
    /// Shield regeneration per second.
    pub shield_regen: f32,
    /// Hull armor type index into the damage matrix.
    pub armor: Option<u16>,
    /// Shield armor type index.
    pub shield_armor: Option<u16>,
    /// Collision radius.
    pub radius: f32,
    /// Radius of the model (bones) before the class clamp; stations and big ships are larger than the collision radius.
    pub model_extent: f32,
    /// Plan-view hull from the model's meshes (`None` without a model); ships keep out of it, see [`ShipDef::footprint`].
    pub hull_shape: Option<crate::nav::Hull>,
    /// Height range (lowest, highest z) of the model's meshes (`None` without a model): the fighters a carrier launches leave it below or
    /// beside the hull and keep clear of it, see `launch_path` in the simulation.
    pub hull_z: Option<(f32, f32)>,
    /// Distance of the farthest destroyable hardpoint from the centre (x1.05): a projectile hits the unit within `max(radius, hardpoint_reach)` of its path, so shots aimed at an outlying hardpoint of a
    /// tall station do not fly past it (the collision radius is clamped and was smaller than the model).
    pub hardpoint_reach: f32,
    /// Maximum speed, units per second.
    pub max_speed: f32,
    /// Acceleration, units per second squared.
    pub accel: f32,
    /// Turn rate, radians per second.
    pub turn_rate: f32,
    /// Population cost.
    pub population: i32,
    /// Hardpoints.
    pub hardpoints: Vec<HpDef>,
    /// Weapon fired from the hull (fighters).
    pub direct: Option<DirectWeaponDef>,
    /// Squadron members.
    pub members: Vec<(ShipId, u32)>,
    /// Squadron formation offsets in battle space.
    pub offsets: Vec<Vec3>,
    /// The places the members of a fighter or bomber squadron keep in flight: `offsets` (one per member, the fallback row where the data
    /// has none) spread out by [`spread_formation`]. Empty for other squadrons and hand-made definitions, which fly at `offsets`.
    pub spread: Vec<Vec3>,
    /// The squadrons launched and held in reserve per tech level of the owner (`Starting_` / `Reserve_Spawned_Units_Tech_N`): `(tech, carried,
    /// reserve)`. Empty for hand-made definitions, which use `carried` and `reserve` at every tech level.
    pub spawned_by_tech: Vec<SpawnedAt>,
    /// Squadrons launched at battle start (carriers, stations) at tech level 0 (or the lowest level the data has).
    pub carried: Vec<(ShipId, u32)>,
    /// Replacement squadrons.
    pub reserve: Vec<(ShipId, u32)>,
    /// Seconds between replacement launches.
    pub spawn_delay: f32,
    /// Abilities.
    pub abilities: Vec<AbilityDef>,
    /// `Targeting_Max_Attack_Distance` (0 = derive from weapons).
    pub targeting_range: f32,
    /// Longest weapon range (derived).
    pub max_weapon_range: f32,
    /// Preferred engagement distance (derived).
    pub preferred_range: f32,
    /// Weapon damage output that can bear on a target at a relative bearing (24 bins of 15 degrees,
    /// bin 0 = straight ahead, counter-clockwise). Ships turn to the best bearing when engaging.
    pub bearing_dps: [f32; 24],
    /// True if the ship has a narrow forward weapon / facing hardpoint and wants its bow on target.
    pub bow_weapons: bool,
    /// Model name (`.alo`), for the renderer.
    pub model: Option<String>,
    /// Index into [`Catalog::priority_sets`] of the default `Targeting_Priority_Set`.
    pub priority: Option<u16>,
    /// Index of the AI attack-move variant (`<set>_Attack_Move`), used under `AttackMove` orders.
    pub priority_attack_move: Option<u16>,
    /// Golan-class platform (matches the `Golan` entries of the priority tables).
    pub golan: bool,
    /// Always prevents enemy retreat while alive (`Is_Interdictor`); ships with an `INTERDICT` ability do so while it is on.
    pub interdictor: bool,
    /// Ground-to-space special weapon definition, if this structure fires into orbit.
    pub special_weapon: Option<SpecialWeaponDef>,
    /// `In_Background` (the Death Star): a backdrop object, neither targeted nor hit by anything.
    pub background: bool,
    /// `FormationOrder`: formations put lower orders further ahead.
    pub formation_order: u8,
    /// `Formation_Priority`: within its formation band a higher priority fills the front rows first.
    pub formation_priority: i32,
    /// `FormationSpacing`: multiplies `BetweenFormationSpacing` next to this unit (0 = the data has none: 1).
    pub formation_spacing: f32,
    /// `FormationGrouping` `Solo`: the unit does not join formations.
    pub formation_solo: bool,
    /// `WaitsForFormationFormup` false: in a formation the unit flies at its own speed instead of the group's.
    pub formation_no_wait: bool,
}

impl ShipDef {
    /// Plan-view hull that other ships keep out of: the model's capsule, else the circle of the rule before the model hull (0.85 of the
    /// collision radius, of 0.75 of it from frigates up). The collision radius is clamped per class: 700 for a 9000 unit long Executor.
    pub fn footprint(&self) -> crate::nav::Hull {
        let r = if self.class as u8 >= TargetClass::Frigate as u8 { self.radius * 0.75 } else { self.radius };
        self.hull_shape.unwrap_or_else(|| crate::nav::Hull::round(r * 0.85))
    }

    /// True if this ship acts as a carrier (has fighter bays, carried squadrons, or carrier designation).
    pub fn is_carrier(&self) -> bool {
        !self.carried.is_empty()
            || !self.spawned_by_tech.is_empty()
            || self.hardpoints.iter().any(|h| h.kind == HpKind::FighterBay)
            || self.name.to_ascii_lowercase().contains("carrier")
    }

    /// True if this ship is a support/utility vessel (transports, healers, buff auras, tenders).
    pub fn is_support(&self) -> bool {
        self.class == TargetClass::Transport
            || !self.heals.is_empty()
            || !self.bonuses.is_empty()
            || self.name.to_ascii_lowercase().contains("support")
            || self.name.to_ascii_lowercase().contains("tender")
    }

    /// True if this unit is a fast corvette or gunship capable of wide tactical flanking maneuvers.
    pub fn is_flanker(&self) -> bool {
        !self.is_static
            && !self.is_squadron
            && (self.class == TargetClass::Corvette || (self.max_speed >= 250.0 && self.class != TargetClass::Fighter && self.class != TargetClass::Bomber))
    }
}

/// Hull-mounted weapon of fighters.
#[derive(Debug, Clone, Default)]
pub struct DirectWeaponDef {
    /// Projectile.
    pub projectile: ProjId,
    /// Projectiles per volley.
    pub pulses: u32,
    /// Delay between pulses.
    pub pulse_delay: f32,
    /// Recharge after a volley.
    pub recharge: f32,
    /// Range (`Targeting_Max_Attack_Distance`).
    pub range: f32,
    /// Half cone (`Turret_*_Extent_Degrees`) in radians.
    pub half_cone: f32,
}

/// Ground-to-space special weapon definition (Hypervelocity Gun, Ion Cannon, Seismic Cannon).
#[derive(Debug, Clone, Default)]
pub struct SpecialWeaponDef {
    /// Fired projectile id.
    pub projectile: ProjId,
    /// Recharge between volleys in seconds.
    pub recharge: f32,
    /// Shots per volley.
    pub pulse_count: u32,
    /// Seconds between shots in a volley.
    pub pulse_delay: f32,
    /// Target classes mask (`Frigate | Capital | Super`).
    pub valid_targets: u16,
    /// True if losing this weapon/structure contributes to defeat in space (`Space_Victory_Relevant`).
    pub victory_relevant: bool,
}

/// A projectile type.
#[derive(Debug, Clone, Default)]
pub struct ProjDef {
    /// `Projectile_Combat_Mod`: a timed debuff (or buff) on the ships hit.
    pub combat_mod: Option<CombatMod>,
    /// `Projectile_Weaken_Enemy_*`: the area weakening that `WEAKEN_ENEMY` spawns.
    pub weaken: Option<WeakenDef>,
    /// `Projectile_Instant_Heal_*`: (radius, hull points, class mask) of an instant heal blast.
    pub instant_heal: Option<(f32, f32, u16)>,
    /// `Projectile_Blast_Area_Dropoff_Tiers`.
    pub blast_tiers: u32,
    /// Object name.
    pub name: String,
    /// `Projectile_Category` (lower case), e.g. `laser`, `missile`.
    pub category: String,
    /// Damage type index.
    pub damage_type: Option<u16>,
    /// Damage per hit.
    pub damage: f32,
    /// Speed, units per second.
    pub speed: f32,
    /// Homing turn rate in radians per second (0 = straight).
    pub turn_rate: f32,
    /// Estimate: homing missiles leave the launcher this many radians off the target line (left or right, per shot) and
    /// curve in with their turn rate; 0 for everything else.
    pub arc_offset: f32,
    /// `Projectile_Rocket_Curve_Offset` in radians (`ROCKET` projectiles only): amplitude of the weave in the rocket's heading (estimate of what the
    /// data means, see RULES.md); 0 = flies straight.
    pub rocket_weave: f32,
    /// `Projectile_Rocket_Straight_Distance`: flight distance before the weave starts.
    pub rocket_straight: f32,
    /// `Projectile_Rocket_Curve_Distance`: flight distance one weave lasts.
    pub rocket_curve: f32,
    /// Maximum flight distance.
    pub max_distance: f32,
    /// Hurts shields.
    pub shield_damage: bool,
    /// Hurts hull and hardpoints.
    pub hull_damage: bool,
    /// Area damage radius.
    pub blast_range: f32,
    /// Area damage.
    pub blast_damage: f32,
    /// Max victims of the area damage.
    pub blast_victims: u32,
    /// Ion stun duration in seconds.
    pub ion_duration: f32,
    /// Fraction of speed removed by ion stun.
    pub ion_speed_cut: f32,
    /// Fraction of fire rate removed by ion stun.
    pub ion_rate_cut: f32,
    /// Model name for the renderer.
    pub model: Option<String>,
}

/// Damage-type versus armor-type multipliers with dense indices.
#[derive(Debug, Clone, Default)]
pub struct DamageMatrix {
    damage: HashMap<String, u16>,
    armor: HashMap<String, u16>,
    armors: usize,
    mult: Vec<f32>,
}

impl DamageMatrix {
    fn build(data: &GameData) -> Self {
        let mut m = DamageMatrix::default();
        for (i, d) in data.combat.damage_types.iter().enumerate() {
            m.damage.insert(d.to_ascii_lowercase(), i as u16);
        }
        for (i, a) in data.combat.armor_types.iter().enumerate() {
            m.armor.insert(a.to_ascii_lowercase(), i as u16);
        }
        m.armors = data.combat.armor_types.len().max(1);
        m.mult = vec![1.0; data.combat.damage_types.len().max(1) * m.armors];
        for (d, &di) in &m.damage.clone() {
            for (a, &ai) in &m.armor.clone() {
                m.mult[di as usize * m.armors + ai as usize] = data.combat.multiplier(d, a);
            }
        }
        m
    }

    /// Index of a damage type.
    pub fn damage_index(&self, name: &str) -> Option<u16> {
        self.damage.get(&name.trim().to_ascii_lowercase()).copied()
    }

    /// Index of an armor type.
    pub fn armor_index(&self, name: &str) -> Option<u16> {
        self.armor.get(&name.trim().to_ascii_lowercase()).copied()
    }

    /// Multiplier; 1.0 for unknown types.
    pub fn multiplier(&self, damage: Option<u16>, armor: Option<u16>) -> f32 {
        match (damage, armor) {
            (Some(d), Some(a)) => self.mult[d as usize * self.armors + a as usize],
            _ => 1.0,
        }
    }
}

/// A `Targeting_Priority_Set` from the `*TargetingPriorities.xml` files.
#[derive(Debug, Clone)]
pub struct PrioritySet {
    /// Set name (lower case).
    pub name: String,
    /// Attack priority per target class (lower = preferred); index 9 is the Golan flag. Unlisted
    /// classes are `f32::MAX` and are only engaged as opportunity targets.
    pub weights: [f32; 10],
    /// Classes that are never attacked (`Category_Exclusions`).
    pub excluded: u16,
    /// Preferred hardpoint kinds to shoot at, in order (`Hard_Point_Priorities`).
    pub hardpoints: Vec<HpKind>,
}

impl PrioritySet {
    /// Priority weight of a target (lower is better); `f32::MAX` if unlisted.
    pub fn weight(&self, class: TargetClass, golan: bool) -> f32 {
        let w = self.weights[class as usize];
        if golan { w.min(self.weights[9]) } else { w }
    }
}

/// Formation constants of GameConstants (see `formation.rs`); the defaults are the Remake's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormationRules {
    /// `BetweenFormationSpacing`: clearance between neighbours in a formation beyond what keeps their hulls apart.
    pub spacing: f32,
    /// `Rotate_Formation_Facing_Moves`: a formation faces the direction of its move (else the way its units face).
    pub rotate_facing: bool,
    /// `FinalFormationFacingMinimumAngle` (radians): a ship that parks less than this off the formation's facing does not turn to it.
    pub facing_min_angle: f32,
}

impl Default for FormationRules {
    fn default() -> Self {
        FormationRules { spacing: 50.0, rotate_facing: true, facing_min_angle: 5.0f32.to_radians() }
    }
}

/// All definitions needed by battles.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Ships, stations and squadron definitions.
    pub ships: Vec<ShipDef>,
    /// Projectiles.
    pub projectiles: Vec<ProjDef>,
    /// Damage matrix.
    pub damage: DamageMatrix,
    /// Frame-to-second factor applied to XML speeds (default 30, the 30 Hz logic tick).
    pub speed_scale: f32,
    /// `ShieldRechargeIntervalInSecs`: shields gain `Shield_Refresh_Rate` once per interval.
    pub shield_interval: f32,
    /// `Depleted_Shield_Disable_Time`: seconds a fully drained shield stays offline.
    pub depleted_shield_disable: f32,
    /// Formation constants.
    pub formation: FormationRules,
    /// Targeting priority sets.
    pub priority_sets: Vec<PrioritySet>,
    priority_index: HashMap<String, u16>,
    ship_index: HashMap<String, ShipId>,
    proj_index: HashMap<String, ProjId>,
}

struct ModelInfo {
    /// lower-case bone name -> (position, local X axis, local Z axis) in battle space
    bones: HashMap<String, (Vec3, Vec3, Vec3, bool, Vec3)>,
    radius: f32,
    /// Plan-view hull from the bounding boxes of the visible meshes.
    hull: Option<crate::nav::Hull>,
    /// Height range (lowest, highest z) of the visible meshes.
    height: Option<(f32, f32)>,
}

fn load_model(fs: &dyn FileSource, name: &str) -> Option<ModelInfo> {
    let bytes = fs.read(&format!("Art/Models/{name}")).ok()?;
    let m = alamo_model::Model::parse(&bytes).ok()?;
    let mut bones = HashMap::new();
    let mut radius = 0.0f32;
    for (i, b) in m.bones.iter().enumerate() {
        let t = m.world_transform(i);
        // model (x, y, z) -> battle (-y, x, z)
        let pos = Vec3::new(-t[1][3], t[0][3], t[2][3]);
        let xaxis = Vec3::new(-t[1][0], t[0][0], t[2][0]).normalized();
        let zaxis = Vec3::new(-t[1][2], t[0][2], t[2][2]).normalized();
        radius = radius.max(pos.len());
        let x = Vec3::new(t[0][0], t[1][0], t[2][0]);
        let y = Vec3::new(t[0][1], t[1][1], t[2][1]);
        let z = Vec3::new(t[0][2], t[1][2], t[2][2]);
        let mirrored = x.cross(y).dot(z) < 0.0;
        let yaxis = Vec3::new(-t[1][1], t[0][1], t[2][1]).normalized();
        bones.insert(b.name.to_ascii_lowercase(), (pos, xaxis, zaxis, mirrored, yaxis));
    }
    let (hull, height) = mesh_hull(&m);
    Some(ModelInfo { bones, radius, hull, height })
}

/// Plan-view hull of a model: the reach ahead of and behind the centre and the half width over the bounding box corners of the visible
/// full-detail meshes (hidden damage, collision and shield meshes and shadow volumes left out), in battle space; and the height range
/// (lowest, highest z) of those corners.
fn mesh_hull(m: &alamo_model::Model) -> (Option<crate::nav::Hull>, Option<(f32, f32)>) {
    let (mut front, mut back, mut half_width) = (f32::MIN, f32::MIN, 0.0f32);
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for (i, mesh) in m.meshes.iter().enumerate() {
        if mesh.hidden || mesh.lod > 0 || mesh.is_shadow_volume() {
            continue;
        }
        let Some(c) = m.connections.iter().find(|c| c.object == i) else { continue };
        let t = m.world_transform(c.bone);
        let (lo, hi) = (mesh.bbox_min, mesh.bbox_max);
        for k in 0..8 {
            let p = [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }];
            let w = |r: usize| t[r][0] * p[0] + t[r][1] * p[1] + t[r][2] * p[2] + t[r][3];
            // model (x, y, z) -> battle (-y, x, z): the bow is +x.
            let (bx, by) = (-w(1), w(0));
            front = front.max(bx);
            back = back.max(-bx);
            half_width = half_width.max(by.abs());
            low = low.min(w(2));
            high = high.max(w(2));
        }
    }
    let hull = (front > f32::MIN && half_width > 0.0).then(|| crate::nav::Hull { front: front.max(0.0), back: back.max(0.0), half_width });
    (hull, (low <= high).then_some((low, high)))
}

/// Bone positions of a turret model (`Model_To_Attach`), in its own model space.
struct AttachInfo {
    bones: HashMap<String, Vec3>,
}

fn load_attach(fs: &dyn FileSource, name: &str) -> Option<AttachInfo> {
    let bytes = fs.read(&format!("Art/Models/{name}")).ok()?;
    let m = alamo_model::Model::parse(&bytes).ok()?;
    let bones = m
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let t = m.world_transform(i);
            (b.name.to_ascii_lowercase(), Vec3::new(t[0][3], t[1][3], t[2][3]))
        })
        .collect();
    Some(AttachInfo { bones })
}

fn parse_priority_sets(fs: &dyn FileSource) -> Vec<PrioritySet> {
    use alamo_data::xml::{parse, split_list, Warning};
    let mut warnings: Vec<Warning> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    if let Ok(bytes) = fs.read("Xml/TARGETINGPRIORITYSETFILES.XML")
        && let Some(root) = parse(&bytes, "TARGETINGPRIORITYSETFILES.XML", &mut warnings)
    {
        files = root.children_named("file").map(|f| f.text_trimmed().to_string()).collect();
    }
    let hp_kind = |t: &str| -> Option<HpKind> {
        let u = t.to_ascii_uppercase();
        if u.contains("SHIELD") {
            Some(HpKind::ShieldGenerator)
        } else if u.contains("ENGINE") {
            Some(HpKind::Engine)
        } else if u.contains("FIGHTER_BAY") {
            Some(HpKind::FighterBay)
        } else if u.contains("TRACTOR") {
            Some(HpKind::TractorBeam)
        } else if u.starts_with("WEAPON") {
            Some(HpKind::Weapon)
        } else {
            None
        }
    };
    let mut out = Vec::new();
    for f in files {
        let Ok(bytes) = fs.read(&format!("Xml/{f}")) else { continue };
        let Some(root) = parse(&bytes, &f, &mut warnings) else { continue };
        for set in root.children_named("priority_set") {
            let Some(name) = set.attr("name") else { continue };
            let mut weights = [f32::MAX; 10];
            if let Some(ap) = set.child("attack_priorities") {
                for pair in split_list(&ap.text).chunks(2) {
                    let [class, w] = pair else { continue };
                    let Ok(w) = w.parse::<f32>() else { continue };
                    if class.eq_ignore_ascii_case("golan") {
                        weights[9] = weights[9].min(w);
                    } else if let Some(c) = TargetClass::from_token(class) {
                        weights[c as usize] = weights[c as usize].min(w);
                    }
                }
            }
            let excluded = set
                .child("category_exclusions")
                .map(|e| split_list(&e.text).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit()))
                .unwrap_or(0);
            let mut hardpoints: Vec<HpKind> = Vec::new();
            if let Some(h) = set.child("hard_point_priorities") {
                for t in split_list(&h.text) {
                    if let Some(k) = hp_kind(&t)
                        && !hardpoints.contains(&k)
                    {
                        hardpoints.push(k);
                    }
                }
            }
            out.push(PrioritySet { name: name.trim().to_ascii_lowercase(), weights, excluded, hardpoints });
        }
    }
    out
}

enum Source {
    Unit(Box<SpaceUnitDef>),
    Base(Box<StarBaseDef>),
    Structure(Box<StructureDef>),
}

impl Source {
    fn name(&self) -> &str {
        match self {
            Source::Unit(u) => &u.name,
            Source::Base(b) => &b.name,
            Source::Structure(s) => &s.name,
        }
    }

    fn spawned_names(&self) -> Vec<String> {
        match self {
            Source::Unit(u) => u
                .squadron_units
                .iter()
                .cloned()
                .chain(u.spawned_by_tech.iter().flat_map(|t| t.starting.iter().chain(&t.reserve).map(|x| x.0.clone())))
                .collect(),
            Source::Base(b) => b.spawned_by_tech.iter().flat_map(|t| t.starting.iter().chain(&t.reserve).map(|x| x.0.clone())).collect(),
            Source::Structure(_) => Vec::new(),
        }
    }

    fn model(&self) -> Option<&str> {
        match self {
            Source::Unit(u) => u.space_model.as_deref(),
            Source::Base(b) => b.space_model.as_deref(),
            Source::Structure(s) => s.space_model.as_deref(),
        }
    }

    /// `Scale_Factor` of the object: the renderer scales the whole model (and so every bone) by it, so the bone positions the simulation reads from the
    /// model file must be scaled the same way (U48: a star base's hardpoints floated above its model before).
    fn scale_factor(&self) -> f32 {
        let fields = match self {
            Source::Unit(u) => &u.fields,
            Source::Base(b) => &b.fields,
            Source::Structure(s) => &s.fields,
        };
        fields.get("scale_factor").and_then(|v| v.trim().parse::<f32>().ok()).filter(|s| *s > 0.0).unwrap_or(1.0)
    }

    fn hardpoints(&self) -> &[String] {
        match self {
            Source::Unit(u) => &u.hardpoints,
            Source::Base(b) => &b.hardpoints,
            Source::Structure(s) => &s.hardpoints,
        }
    }
}

fn load_source(data: &GameData, name: &str) -> Option<Source> {
    if let Some(u) = data.objects.space_unit(name) {
        return Some(Source::Unit(Box::new(u)));
    }
    if let Some(b) = data.objects.star_base(name) {
        return Some(Source::Base(Box::new(b)));
    }
    data.objects.structure(name).map(|s| Source::Structure(Box::new(s)))
}

fn hash_unit(s: &str, salt: u32) -> f32 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ salt as u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    ((h >> 33) as f32) / ((1u64 << 31) as f32)
}

fn default_radius(class: TargetClass, is_static: bool) -> f32 {
    if is_static {
        return 250.0;
    }
    match class {
        TargetClass::Fighter | TargetClass::Bomber => 14.0,
        TargetClass::Transport | TargetClass::Corvette => 60.0,
        TargetClass::Frigate => 130.0,
        TargetClass::Capital => 300.0,
        TargetClass::Super => 700.0,
        _ => 100.0,
    }
}

fn radius_range(class: TargetClass, is_static: bool) -> (f32, f32) {
    if is_static {
        return (120.0, 700.0);
    }
    match class {
        TargetClass::Fighter | TargetClass::Bomber => (8.0, 40.0),
        TargetClass::Transport | TargetClass::Corvette => (30.0, 160.0),
        TargetClass::Frigate => (70.0, 320.0),
        TargetClass::Capital => (150.0, 700.0),
        TargetClass::Super => (400.0, 2500.0),
        _ => (30.0, 600.0),
    }
}

fn accel_time(class: TargetClass, is_static: bool) -> f32 {
    if is_static {
        return 1.0;
    }
    match class {
        TargetClass::Fighter | TargetClass::Bomber => 1.5,
        TargetClass::Transport | TargetClass::Corvette => 4.0,
        TargetClass::Frigate => 6.0,
        TargetClass::Capital => 10.0,
        TargetClass::Super => 16.0,
        _ => 6.0,
    }
}

impl Catalog {
    /// Builds a catalog containing `names` (ships, squadrons, star bases, structures) and
    /// everything they spawn. `fs` supplies the `.alo` models for hardpoint positions; without it
    /// hardpoints are placed on a deterministic pseudo layout.
    pub fn build(data: &GameData, fs: Option<&dyn FileSource>, names: &[&str]) -> Catalog {
        let damage = DamageMatrix::build(data);

        // Transitive closure over spawned units.
        let mut sources: Vec<Source> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        while let Some(n) = queue.pop() {
            if !seen.insert(n.to_ascii_lowercase()) {
                continue;
            }
            if let Some(src) = load_source(data, &n) {
                queue.extend(src.spawned_names());
                sources.push(src);
            }
        }
        sources.sort_by_key(|s| s.name().to_ascii_lowercase());

        // Models in parallel.
        let mut model_names: Vec<String> = sources.iter().filter_map(|s| s.model().map(str::to_string)).collect();
        model_names.sort();
        model_names.dedup();
        let models: HashMap<String, Arc<ModelInfo>> = match fs {
            Some(fs) => model_names
                .par_iter()
                .filter_map(|n| load_model(fs, n).map(|m| (n.to_ascii_lowercase(), Arc::new(m))))
                .collect(),
            None => HashMap::new(),
        };

        let priority_sets = fs.map(parse_priority_sets).unwrap_or_default();
        let priority_index: HashMap<String, u16> = priority_sets.iter().enumerate().map(|(i, s)| (s.name.clone(), i as u16)).collect();
        let shield_interval = data.game_constants.get("shieldrechargeintervalinsecs").and_then(|v| v.parse().ok()).unwrap_or(2.0f32).max(0.1);
        let depleted_shield_disable = data.game_constants.get("depleted_shield_disable_time").and_then(|v| v.parse().ok()).unwrap_or(0.0f32).max(0.0);
        let constant = |tag: &str| data.game_constants.get(tag).and_then(|v| v.trim().parse::<f32>().ok());
        let rules = FormationRules::default();
        let formation = FormationRules {
            spacing: constant("betweenformationspacing").unwrap_or(rules.spacing).max(0.0),
            rotate_facing: data.game_constants.get("rotate_formation_facing_moves").map_or(rules.rotate_facing, |v| alamo_data::xml::parse_bool(v)),
            facing_min_angle: constant("finalformationfacingminimumangle").map_or(rules.facing_min_angle, f32::to_radians),
        };
        let mut cat = Catalog { damage, speed_scale: 30.0, shield_interval, depleted_shield_disable, formation, priority_sets, priority_index, ..Catalog::default() };
        // Projectiles.
        let mut proj_names: Vec<String> = Vec::new();
        let hp_defs: Vec<Vec<HardpointDef>> = sources
            .iter()
            .map(|s| {
                let hps = data.objects.hardpoints_of(s.hardpoints());
                for h in &hps {
                    if let Some(p) = &h.projectile {
                        proj_names.push(p.clone());
                    }
                }
                if let Source::Unit(u) = s
                    && let Some(d) = &u.direct_weapon
                    && let Some(p) = d.projectile_types.first()
                {
                    proj_names.push(p.clone());
                }
                if let Source::Unit(u) = s {
                    for sa in u.sub_abilities.iter().filter(|s| s.kind.eq_ignore_ascii_case("Cluster_Bomb_Ability")) {
                        if let Some(n) = sa.fields.get("bomb_type") {
                            proj_names.push(n.trim().to_string());
                        }
                    }
                    for a in u.abilities.iter().filter(|a| a.kind.eq_ignore_ascii_case("WEAKEN_ENEMY") || a.kind.eq_ignore_ascii_case("HARMONIC_BOMB")) {
                        if let Some(n) = a.fields.get("spawned_object_type") {
                            proj_names.push(n.trim().to_string());
                        }
                    }
                }
                if let Source::Structure(st) = s {
                    if let Some(pts) = st.fields.get("projectile_types") {
                        for pt in alamo_data::xml::split_list(pts) {
                            proj_names.push(pt);
                        }
                    }
                }
                hps
            })
            .collect();
        // Turret models: the barrel direction of the attached model gives the real rest direction.
        let mut attach_names: Vec<String> = hp_defs.iter().flatten().filter(|h| h.turret.is_some()).filter_map(|h| h.fields.get("model_to_attach").cloned()).filter(|n| !n.is_empty()).collect();
        attach_names.sort();
        attach_names.dedup();
        let attach: HashMap<String, Arc<AttachInfo>> = match fs {
            Some(fs) => attach_names.par_iter().filter_map(|n| load_attach(fs, n).map(|m| (n.to_ascii_lowercase(), Arc::new(m)))).collect(),
            None => HashMap::new(),
        };
        proj_names.sort();
        proj_names.dedup();
        for pn in &proj_names {
            if let Some(p) = data.objects.projectile(pn) {
                let def = cat.proj_def(&p);
                cat.proj_index.insert(pn.to_ascii_lowercase(), cat.projectiles.len() as ProjId);
                cat.projectiles.push(def);
            }
        }

        // Ship ids first (squadrons refer to members).
        for (i, s) in sources.iter().enumerate() {
            cat.ship_index.insert(s.name().to_ascii_lowercase(), i as ShipId);
        }
        for (i, s) in sources.iter().enumerate() {
            let model = s.model().and_then(|m| models.get(&m.to_ascii_lowercase()));
            let scaled = model.map(|m| {
                let k = s.scale_factor();
                if (k - 1.0).abs() < 1e-4 {
                    return m.clone();
                }
                let bones = m.bones.iter().map(|(n, &(p, x, z, mir, y))| (n.clone(), (Vec3::new(p.x * k, p.y * k, p.z * k), x, z, mir, y))).collect();
                let hull = m.hull.map(|h| crate::nav::Hull { front: h.front * k, back: h.back * k, half_width: h.half_width * k });
                Arc::new(ModelInfo { bones, radius: m.radius * k, hull, height: m.height.map(|(lo, hi)| (lo * k, hi * k)) })
            });
            let model = scaled.as_ref();
            let def = cat.ship_def(s, &hp_defs[i], model.map(|m| &**m), &attach);
            cat.ships.push(def);
        }
        // Squadrons of fighters and bombers fly spread out (their craft are drawn far above true size); other squadrons, like a pair of
        // Tartan patrol cruisers, keep the data offsets.
        for i in 0..cat.ships.len() {
            let d = &cat.ships[i];
            if d.members.is_empty() || !d.members.iter().all(|&(m, _)| matches!(cat.ships[m as usize].class, TargetClass::Fighter | TargetClass::Bomber)) {
                continue;
            }
            let n: usize = d.members.iter().map(|m| m.1 as usize).sum();
            cat.ships[i].spread = spread_formation(&(0..n).map(|k| formation_offset(&d.offsets, k)).collect::<Vec<_>>());
        }
        cat
    }

    fn proj_def(&self, p: &ProjectileDef) -> ProjDef {
        let speed = p.max_speed * self.speed_scale;
        let max_distance = if p.max_flight_distance > 0.0 { p.max_flight_distance } else { 4000.0 };
        let f = |k: &str| p.fields.get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let is_rocket = p.category.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("rocket"));
        ProjDef {
            name: p.name.clone(),
            category: p.category.clone().unwrap_or_default().to_ascii_lowercase(),
            damage_type: p.damage_type.as_deref().and_then(|d| self.damage.damage_index(d)),
            damage: p.damage.max(p.blast_damage),
            speed: speed.max(1.0),
            turn_rate: p.max_rate_of_turn.max(0.0),
            arc_offset: if p.category.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("missile")) { (p.max_rate_of_turn * MISSILE_ARC_PER_TURN_RATE).clamp(0.0, MISSILE_ARC_MAX) } else { 0.0 },
            rocket_weave: if is_rocket && f("projectile_rocket_curve_distance") > 0.0 { f("projectile_rocket_curve_offset").to_radians() } else { 0.0 },
            rocket_straight: f("projectile_rocket_straight_distance"),
            rocket_curve: f("projectile_rocket_curve_distance"),
            max_distance,
            shield_damage: p.does_shield_damage,
            hull_damage: p.does_hitpoint_damage,
            blast_range: p.blast_range,
            blast_damage: p.blast_damage,
            blast_victims: p.blast_max_victims.unwrap_or(10).max(1) as u32,
            combat_mod: p.fields.get("projectile_combat_mod").and_then(|v| match v.trim().to_ascii_uppercase().as_str() {
                "COMBAT_MOD_SPEED" => Some(ModKind::Speed),
                "COMBAT_MOD_FIRE_RATE" => Some(ModKind::FireRate),
                "COMBAT_MOD_DEFENSE" => Some(ModKind::Defense),
                "COMBAT_MOD_HEALTH" => Some(ModKind::Health),
                "COMBAT_MOD_SHIELD" => Some(ModKind::Shield),
                _ => None,
            })
            .map(|kind| CombatMod {
                kind,
                value: f("projectile_combat_mod_value").clamp(-0.9, 10.0),
                duration: f("projectile_combat_mod_duration"),
                classes: p.fields.get("projectile_combat_mod_category_mask").map_or(0, |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit())),
            })
            .filter(|m| m.value != 0.0 && m.duration > 0.0),
            instant_heal: p.fields.get("projectile_instant_heal_on_detonation").is_some_and(|v| alamo_data::xml::parse_bool(v)).then(|| {
                (f("projectile_instant_heal_radius"), f("projectile_instant_heal_health_increase"), p.fields.get("projectile_instant_heal_targets_category_mask").map_or(0, |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit())))
            }),
            blast_tiers: if p.fields.get("projectile_blast_area_dropoff").is_some_and(|v| alamo_data::xml::parse_bool(v)) { f("projectile_blast_area_dropoff_tiers") as u32 } else { 0 },
            weaken: p.fields.get("projectile_weaken_enemy_on_detonation").is_some_and(|v| alamo_data::xml::parse_bool(v)).then(|| WeakenDef {
                radius: f("projectile_weaken_enemy_radius"),
                duration: f("projectile_weaken_enemy_duration_seconds"),
                cause_reduction: f("projectile_weaken_enemy_cause_damage_reduction_percent").clamp(0.0, 0.95),
                take_increase: f("projectile_weaken_enemy_take_damage_increase_percent").max(0.0),
                speed_cut: if p.fields.get("projectile_combat_mod").is_some_and(|v| v.trim().eq_ignore_ascii_case("COMBAT_MOD_SPEED")) { (-f("projectile_combat_mod_value")).clamp(0.0, 0.9) } else { 0.0 },
            }),
            ion_duration: p.ion_stun_duration,
            ion_speed_cut: (p.ion_stun_speed_reduction / 100.0).clamp(0.0, 1.0),
            ion_rate_cut: (p.ion_stun_shot_rate_reduction / 100.0).clamp(0.0, 1.0),
            model: p.space_model.clone().filter(|_| f("scale_factor") >= 0.0),
        }
    }

    fn ship_def(&self, src: &Source, hps: &[HardpointDef], model: Option<&ModelInfo>, attach: &HashMap<String, Arc<AttachInfo>>) -> ShipDef {
        // Common numbers.
        let (name, text_id, class, is_static, is_squadron, hull, shield, regen, armor, sarmor, pop) = match src {
            Source::Unit(u) => (
                u.name.clone(),
                u.text_id.clone(),
                TargetClass::from_category(u.category),
                false,
                u.is_squadron(),
                u.hull,
                u.shield_points,
                u.shield_refresh_rate,
                u.armor_type.clone(),
                u.shield_armor_type.clone(),
                u.space_population.unwrap_or(u.population_value),
            ),
            Source::Base(b) => (
                b.name.clone(),
                b.text_id.clone(),
                TargetClass::Structure,
                true,
                false,
                b.hull,
                b.shield_points,
                b.shield_refresh_rate,
                b.armor_type.clone(),
                b.shield_armor_type.clone(),
                0,
            ),
            Source::Structure(s) => (
                s.name.clone(),
                s.text_id.clone(),
                TargetClass::Structure,
                true,
                false,
                s.hull,
                s.shield_points,
                s.fields.get("shield_refresh_rate").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                s.armor_type.clone(),
                s.fields.get("shield_armor_type").cloned(),
                0,
            ),
        };

        // Radius from model bones (hardpoint bones preferred).
        let (rmin, rmax) = radius_range(class, is_static);
        let mut radius = default_radius(class, is_static);
        if let Some(m) = model {
            let hp_r = hps
                .iter()
                .filter_map(|h| h.fire_bone_a.as_deref().or(h.attachment_bone.as_deref()))
                .filter_map(|b| m.bones.get(&b.to_ascii_lowercase()))
                .map(|(p, _, _, _, _)| p.len())
                .fold(0.0f32, f32::max);
            let r = if hp_r > 0.0 { hp_r * 1.1 } else { m.radius };
            if r > 0.0 {
                radius = r;
            }
        }
        let model_extent = model.map_or(radius, |m| m.radius.max(radius));
        radius = radius.clamp(rmin, rmax);

        // Hardpoints.
        let mut hardpoints = Vec::with_capacity(hps.len());
        for (i, h) in hps.iter().enumerate() {
            hardpoints.push(self.hp_def(h, i, model, attach, radius, class));
        }

        // Mobile warships with destroyable hardpoints have engine hitboxes at the rear.
        // Remake XML omitted explicit engine hardpoints on some capitals (e.g. ISD);
        // synthesize them so engines are always targetable and slow the ship by 80% when destroyed.
        if !is_static && !is_squadron && hardpoints.iter().any(|h| h.destroyable) && !hardpoints.iter().any(|h| h.kind == HpKind::Engine) {
            let total_hp: f32 = hardpoints.iter().filter(|h| h.destroyable).map(|h| h.health).sum();
            let hp_health = (total_hp * 0.08).clamp(1000.0, 3000.0);
            hardpoints.push(HpDef {
                name: format!("{}_Engines", name),
                kind: HpKind::Engine,
                health: hp_health,
                destroyable: true,
                targetable: true,
                offset: Vec3::new(-radius * 0.75, 0.0, 0.0),
                forward: Vec3::new(-1.0, 0.0, 0.0),
                normal: Vec3::new(0.0, 0.0, 1.0),
                left: Vec3::new(0.0, -1.0, 0.0),
                arc_yaw: std::f32::consts::PI,
                arc_pitch: std::f32::consts::FRAC_PI_2,
                ..HpDef::default()
            });
        }

        let (max_speed, turn_rate, accel) = match src {
            Source::Unit(u) if !is_squadron => {
                let speed = u.max_speed * self.speed_scale;
                let turn = if u.max_rate_of_turn > 0.0 { u.max_rate_of_turn } else { 0.5 };
                (speed, turn, speed / accel_time(class, false))
            }
            _ => (0.0, 0.0, 0.0),
        };
        let mut bearing_dps = [0.0f32; 24];
        for h in hardpoints.iter_mut().filter(|h| h.kind == HpKind::Weapon) {
            let Some(p) = h.projectile else { continue };
            let pd = &self.projectiles[p as usize];
            let cycle = ((h.recharge_min + h.recharge_max) * 0.5 + (h.pulses - 1) as f32 * h.pulse_delay).max(0.2);
            let dps = pd.damage * h.pulses as f32 / cycle;
            h.dps = dps;
            for (k, slot) in bearing_dps.iter_mut().enumerate() {
                let a = k as f32 * std::f32::consts::TAU / 24.0;
                let dir = Vec3::new(a.cos(), a.sin(), 0.0);
                if dir.dot(h.left).atan2(dir.dot(h.forward)).abs() <= h.arc_yaw && dir.dot(h.normal).clamp(-1.0, 1.0).asin().abs() <= h.arc_pitch {
                    *slot += dps;
                    h.arc_bins |= 1 << k;
                }
            }
        }
        let mut max_weapon_range = hardpoints
            .iter()
            .filter(|h| h.kind == HpKind::Weapon)
            .map(|h| h.range)
            .fold(0.0f32, f32::max);

        let mut direct = None;
        if let Source::Unit(u) = src
            && let Some(d) = &u.direct_weapon
            && let Some(&pid) = d.projectile_types.first().and_then(|p| self.proj_index.get(&p.to_ascii_lowercase()))
        {
            let range = if u.targeting_max_distance > 0.0 { u.targeting_max_distance } else { 600.0 };
            let cone = u.fields.get("turret_rotate_extent_degrees").and_then(|v| v.parse::<f32>().ok()).unwrap_or(30.0);
            direct = Some(DirectWeaponDef {
                projectile: pid,
                pulses: d.pulse_count.max(1),
                pulse_delay: d.pulse_delay_seconds,
                recharge: d.recharge_seconds.max(0.2),
                range,
                half_cone: cone.to_radians().clamp(0.05, std::f32::consts::PI),
            });
            max_weapon_range = max_weapon_range.max(range);
        }

        let targeting_range = match src {
            Source::Unit(u) => u.targeting_max_distance,
            _ => 0.0,
        };
        // Engage at the range that puts 80% of the weapon damage output in reach. A long-range specialist (its hull-damaging weapons that reach farthest,
        // backed by at least 25% of the damage, out-range that by half again, e.g. a missile ship) holds at 0.8 of that longest range instead.
        let mut by_range: Vec<(f32, f32, bool)> = Vec::new();
        for h in hardpoints.iter().filter(|h| h.kind == HpKind::Weapon) {
            if let Some(p) = h.projectile {
                let pd = &self.projectiles[p as usize];
                let cycle = ((h.recharge_min + h.recharge_max) * 0.5 + (h.pulses - 1) as f32 * h.pulse_delay).max(0.2);
                by_range.push((h.range, pd.damage * h.pulses as f32 / cycle, pd.hull_damage));
            }
        }
        by_range.sort_by(|a, b| b.0.total_cmp(&a.0));
        let cover_at = |share: f32, hull_only: bool| {
            let list: Vec<(f32, f32)> = by_range.iter().filter(|x| x.2 || !hull_only).map(|x| (x.0, x.1)).collect();
            let total: f32 = list.iter().map(|x| x.1).sum();
            let (mut acc, mut cover) = (0.0, 0.0);
            for &(range, dps) in &list {
                acc += dps;
                cover = range;
                if acc >= share * total {
                    break;
                }
            }
            (total, cover)
        };
        let (total, cover) = cover_at(0.8, false);
        let (_, long_cover) = cover_at(0.25, true);
        let preferred_range = if total > 0.0 {
            let usual = cover * 0.95;
            if long_cover * 0.8 > usual * 1.5 { long_cover * 0.8 } else { usual }
        } else if max_weapon_range > 0.0 {
            max_weapon_range * 0.8
        } else {
            0.0
        };
        let bow_weapons = hardpoints
            .iter()
            .any(|h| h.kind == HpKind::Facing || (h.kind == HpKind::Weapon && !h.turret && h.half_yaw < 0.35 && h.projectile.is_some()));

        let resolve = |v: &[(String, u32)]| -> Vec<(ShipId, u32)> {
            v.iter()
                .filter_map(|(n, c)| self.ship_index.get(&n.to_ascii_lowercase()).map(|&id| (id, *c)))
                .collect()
        };
        let spawned_lists = match src {
            Source::Unit(u) => &u.spawned_by_tech[..],
            Source::Base(b) => &b.spawned_by_tech[..],
            Source::Structure(_) => &[][..],
        };
        let spawned_by_tech: Vec<SpawnedAt> = spawned_lists.iter().map(|t| (t.tech, resolve(&t.starting), resolve(&t.reserve))).collect();
        let at_tech0 = alamo_data::spawned_for_tech(spawned_lists, 0);
        let (members, offsets, carried, reserve, spawn_delay, abilities) = match src {
            Source::Unit(u) => {
                let mut members: Vec<(ShipId, u32)> = Vec::new();
                for m in &u.squadron_units {
                    if let Some(&id) = self.ship_index.get(&m.to_ascii_lowercase()) {
                        match members.iter_mut().find(|(i, _)| *i == id) {
                            Some(e) => e.1 += 1,
                            None => members.push((id, 1)),
                        }
                    }
                }
                let offsets = u.squadron_offsets.iter().map(|o| Vec3::from_array(*o)).collect();
                let delay = u.fields.get("spawned_squadron_delay_seconds").and_then(|v| v.parse().ok()).unwrap_or(2.0);
                (members, offsets, at_tech0.map(|t| resolve(&t.starting)).unwrap_or_default(), at_tech0.map(|t| resolve(&t.reserve)).unwrap_or_default(), delay, ability_defs(u, &self.projectiles, &self.proj_index))
            }
            Source::Base(b) => {
                let delay = b.fields.get("spawned_squadron_delay_seconds").and_then(|v| v.parse().ok()).unwrap_or(2.0);
                (Vec::new(), Vec::new(), at_tech0.map(|t| resolve(&t.starting)).unwrap_or_default(), at_tech0.map(|t| resolve(&t.reserve)).unwrap_or_default(), delay, Vec::new())
            }
            Source::Structure(_) => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 2.0, Vec::new()),
        };

        // Targeting priority sets: the unit's own, else one named after its class.
        let own_set = match src {
            Source::Unit(u) => u.targeting_priority_set.clone(),
            _ => None,
        };
        let class_set = match class {
            TargetClass::Fighter => "fighter",
            TargetClass::Bomber => "bomber",
            TargetClass::Transport => "transport",
            TargetClass::Corvette => "corvette",
            TargetClass::Capital => "capital",
            TargetClass::Super => "super",
            _ => "frigate",
        };
        let set_name = own_set.map(|s| s.to_ascii_lowercase()).filter(|s| self.priority_index.contains_key(s)).unwrap_or_else(|| class_set.to_string());
        let priority = self.priority_index.get(&set_name).copied();
        let priority_attack_move = self.priority_index.get(&format!("{set_name}_attack_move")).copied().or(priority);
        let golan = name.to_ascii_lowercase().contains("golan");
        let interdictor = match src {
            Source::Unit(u) => {
                u.fields.get("is_interdictor").is_some_and(|v| alamo_data::xml::parse_bool(v))
            }
            // Gravity well stations: `Is_Interdictor`, and the ability is `Active_By_Default`, so the field is always up.
            Source::Structure(s) => s.fields.get("is_interdictor").is_some_and(|v| alamo_data::xml::parse_bool(v)),
            _ => false,
        };

        let unit_field = |tag: &str| match src {
            Source::Unit(u) => u.fields.get(tag).map(String::as_str),
            _ => None,
        };
        let no_spawn_radius = match src {
            Source::Unit(u) => u.fields.get("reinforcement_prevention_radius").and_then(|v| v.trim().parse::<f32>().ok()),
            Source::Structure(s) => s.fields.get("reinforcement_prevention_radius").and_then(|v| v.trim().parse::<f32>().ok()),
            Source::Base(b) => Some(b.reinforcement_prevention_radius),
        }
        .filter(|&r| r > 0.0)
        .unwrap_or(0.0);
        let sensor_range = match src {
            Source::Unit(u) => u.fields.get("space_fow_reveal_range").and_then(|v| v.trim().parse::<f32>().ok()),
            Source::Structure(s) => s.fields.get("space_fow_reveal_range").and_then(|v| v.trim().parse::<f32>().ok()),
            Source::Base(b) => Some(b.fow_reveal_range),
        }
        .filter(|&r| r > 0.0)
        .unwrap_or(1250.0);
        let heals: Vec<HealDef> = match src {
            Source::Unit(u) => u
                .sub_abilities
                .iter()
                .filter(|a| a.kind.eq_ignore_ascii_case("Force_Healing_Ability") && a.fields.get("activation_style").is_some_and(|v| v.trim().eq_ignore_ascii_case("Space_Automatic")))
                .map(|a| {
                    let num = |k: &str| a.fields.get(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
                    HealDef {
                        range: num("heal_range"),
                        amount: num("heal_amount"),
                        percent: num("heal_percent"),
                        interval: num("heal_interval_in_secs").max(0.5),
                        single: a.fields.get("single_target_heal").is_some_and(|v| alamo_data::xml::parse_bool(v)),
                        classes: a.fields.get("applicable_unit_categories").map_or(0, |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit())),
                    }
                })
                .filter(|h| h.amount > 0.0 || h.percent > 0.0)
                .collect(),
            _ => Vec::new(),
        };
        let bonuses: Vec<BonusDef> = match src {
            Source::Unit(u) => u
                .sub_abilities
                .iter()
                .filter(|a| a.kind.eq_ignore_ascii_case("Combat_Bonus_Ability") && a.fields.get("activation_style").is_some_and(|v| v.trim().eq_ignore_ascii_case("Space_Automatic")))
                .map(|a| {
                    let num = |k: &str| a.fields.get(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0).clamp(-0.9, 10.0);
                    BonusDef {
                        damage: num("damage_bonus_percentage"),
                        health: num("health_bonus_percentage"),
                        shield: num("shield_bonus_percentage"),
                        defense: num("defense_bonus_percentage"),
                        range: num("fire_range_bonus_percentage"),
                        speed: num("movement_speed_bonus_percentage"),
                        classes: a.fields.get("applicable_unit_categories").map_or(0, |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit())),
                        stacking: num("stacking_category") as u8,
                    }
                })
                .filter(|b| b.damage.abs() + b.health.abs() + b.shield.abs() + b.defense.abs() + b.range.abs() + b.speed.abs() > 0.0)
                .collect(),
            _ => Vec::new(),
        };
        let redirects: Vec<RedirectDef> = match src {
            Source::Unit(u) => u
                .sub_abilities
                .iter()
                .filter(|a| a.kind.eq_ignore_ascii_case("Redirect_Blaster_Ability") && !a.fields.get("initially_enabled").is_some_and(|v| !alamo_data::xml::parse_bool(v)))
                .map(|a| {
                    let num = |k: &str| a.fields.get(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0).clamp(0.0, 1.0);
                    RedirectDef {
                        block: num("block_chance"),
                        redirect: num("redirect_chance"),
                        half_arc: a.fields.get("reaction_arc_in_degrees").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(360.0).clamp(0.0, 360.0).to_radians() / 2.0,
                        classes: a.fields.get("applicable_unit_categories").map_or(0, |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit())),
                    }
                })
                .filter(|r| r.block + r.redirect > 0.0)
                .collect(),
            _ => Vec::new(),
        };
        let special_weapon = match src {
            Source::Structure(s) => {
                let is_special = s.fields.get("is_special_weapon_in_space").is_some_and(|v| alamo_data::xml::parse_bool(v));
                if is_special {
                    let proj = s.fields.get("projectile_types").and_then(|pts| {
                        alamo_data::xml::split_list(pts).first().and_then(|p| self.proj_index.get(&p.to_ascii_lowercase()).copied())
                    });
                    proj.map(|projectile| {
                        let recharge = s.fields.get("projectile_fire_recharge_seconds").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(80.0);
                        let pulse_count = s.fields.get("projectile_fire_pulse_count").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1);
                        let pulse_delay = s.fields.get("projectile_fire_pulse_delay_seconds").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.5);
                        let valid_targets = s.fields.get("special_weapon_valid_targets").map_or(
                            TargetClass::Frigate.bit() | TargetClass::Capital.bit() | TargetClass::Super.bit(),
                            |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit()),
                        );
                        let victory_relevant = s.fields.get("space_victory_relevant").map_or(false, |v| alamo_data::xml::parse_bool(v));
                        SpecialWeaponDef {
                            projectile,
                            recharge,
                            pulse_count,
                            pulse_delay,
                            valid_targets,
                            victory_relevant,
                        }
                    })
                } else {
                    None
                }
            }
            _ => None,
        };
        ShipDef {
            name,
            text_id,
            heals,
            redirects,
            bonuses,
            sensor_range,
            no_spawn_radius,
            class,
            is_static,
            is_squadron,
            hull,
            shield,
            shield_regen: regen,
            armor: armor.as_deref().and_then(|a| self.damage.armor_index(a)),
            shield_armor: sarmor.as_deref().and_then(|a| self.damage.armor_index(a)),
            radius,
            model_extent,
            // A mesh hull far smaller than the collision radius is a placeholder model (estimate: under 0.3 of it): the round hull stands in.
            hull_shape: model.and_then(|m| m.hull).filter(|h| h.reach() >= 0.3 * radius),
            hull_z: model.and_then(|m| m.height),
            hardpoint_reach: hardpoints.iter().filter(|h| h.destroyable).map(|h| h.offset.len()).fold(0.0f32, f32::max) * 1.05,
            max_speed,
            accel,
            turn_rate,
            population: pop,
            hardpoints,
            direct,
            spread: Vec::new(),
            members,
            offsets,
            spawned_by_tech,
            carried,
            reserve,
            spawn_delay,
            abilities,
            targeting_range,
            max_weapon_range,
            preferred_range,
            bearing_dps,
            bow_weapons,
            model: src.model().map(str::to_string),
            priority,
            priority_attack_move,
            golan,
            interdictor,
            special_weapon,
            background: matches!(src, Source::Unit(u) if u.fields.get("in_background").is_some_and(|v| alamo_data::xml::parse_bool(v))),
            formation_order: unit_field("formationorder").and_then(|v| v.trim().parse().ok()).unwrap_or(1),
            formation_priority: unit_field("formation_priority").and_then(|v| v.trim().parse().ok()).unwrap_or(1),
            formation_spacing: unit_field("formationspacing").and_then(|v| v.trim().parse().ok()).unwrap_or(1.0),
            formation_solo: unit_field("formationgrouping").is_some_and(|v| v.trim().eq_ignore_ascii_case("solo")),
            formation_no_wait: unit_field("waitsforformationformup").is_some_and(|v| !alamo_data::xml::parse_bool(v)),
        }
    }

    fn hp_def(&self, h: &HardpointDef, index: usize, model: Option<&ModelInfo>, attach: &HashMap<String, Arc<AttachInfo>>, radius: f32, class: TargetClass) -> HpDef {
        let bone = [h.fire_bone_a.as_deref(), h.attachment_bone.as_deref()]
            .into_iter()
            .flatten()
            .find_map(|b| model.and_then(|m| m.bones.get(b.trim().to_ascii_lowercase().as_str())));
        let mut mirrored = false;
        let mut y_axis = Vec3::ZERO;
        let (offset, forward, normal, from_bone) = match bone {
            Some(&(p, x, z, m, y)) => {
                mirrored = m;
                y_axis = y;
                (p, if x.len_sq() > 0.5 { x } else { Vec3::new(1.0, 0.0, 0.0) }, z, true)
            }
            None => {
                // Deterministic spread over the hull: 70% of the radius along the ship, narrow across.
                let u = hash_unit(&h.name, 1) * 2.0 - 1.0;
                let v = hash_unit(&h.name, 2) * 2.0 - 1.0;
                let w = hash_unit(&h.name, 3) * 2.0 - 1.0;
                (Vec3::new(u * radius * 0.7, v * radius * 0.25, w * radius * 0.1), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), false)
            }
        };
        // Turrets: the real rest direction is the barrel direction of the attached turret model,
        // rotated into the ship by the mount bone (the mount bone axes alone are authored arbitrarily).
        let bone_x = forward;
        let mut forward = forward;
        if from_bone
            && h.turret.is_some()
            && let Some(info) = h.fields.get("model_to_attach").and_then(|n| attach.get(&n.trim().to_ascii_lowercase()))
        {
            let fp = [h.fire_bone_a.as_deref(), Some("fp_01")].into_iter().flatten().find_map(|b| info.bones.get(b.trim().to_ascii_lowercase().as_str()));
            let base = info.bones.get("barrel").or_else(|| info.bones.get("turret"));
            if let (Some(&fp), Some(&base)) = (fp, base) {
                let dm = fp - base;
                if dm.len() > 1.0 {
                    let dm = dm.normalized();
                    let rest = bone_x * dm.x + y_axis * dm.y + normal * dm.z;
                    if rest.len_sq() > 0.5 {
                        forward = rest.normalized();
                    }
                }
            }
        }
        // Orthonormal mount frame (forward, left, normal).
        let mut normal = (normal - forward * normal.dot(forward)).normalized();
        if normal.len_sq() < 0.5 {
            normal = Vec3::new(0.0, 0.0, 1.0);
            if forward.dot(normal).abs() > 0.9 {
                normal = Vec3::new(0.0, 1.0, 0.0);
            }
            normal = (normal - forward * normal.dot(forward)).normalized();
        }
        let mut left = normal.cross(forward);
        let (mut forward, mut normal) = (forward, normal);
        let turret_yaw = h.turret.as_ref().map_or(0.0, |t| t.rotate_extent.to_radians());
        let turret_pitch = h.turret.as_ref().map_or(0.0, |t| t.elevate_extent.to_radians());
        let half_yaw = (h.fire_cone_width.to_radians() * 0.5).clamp(0.0, std::f32::consts::PI);
        let half_pitch = (h.fire_cone_height.to_radians() * 0.5).clamp(0.0, std::f32::consts::FRAC_PI_2);
        let mut arc_yaw = half_yaw.max(turret_yaw).min(std::f32::consts::PI);
        let mut arc_pitch = half_pitch.max(turret_pitch).clamp(0.1, std::f32::consts::FRAC_PI_2);
        // Some bones are oriented in ways that cannot reach the battle plane at all (e.g. a gunship's
        // nose gun whose bone axis points sideways); re-center those on the ship's forward axis.
        let mut arc_fallback = false;
        if bone.is_some() && kind_is_weapon(h) {
            let reaches = (0..36).any(|k| {
                let a = k as f32 * std::f32::consts::TAU / 36.0;
                let d = Vec3::new(a.cos(), a.sin(), 0.0);
                d.dot(left).atan2(d.dot(forward)).abs() <= arc_yaw && d.dot(normal).clamp(-1.0, 1.0).asin().abs() <= arc_pitch
            });
            if !reaches {
                arc_fallback = true;
                forward = Vec3::new(1.0, 0.0, 0.0);
                normal = Vec3::new(0.0, 0.0, 1.0);
                left = Vec3::new(0.0, 1.0, 0.0);
                arc_yaw = arc_yaw.max(60f32.to_radians());
                arc_pitch = arc_pitch.max(45f32.to_radians());
            }
        }
        // A weapon that only the super laser ability uses (`Blast_Ability_Fire_Projectile_Type`) never fires on its own.
        let ability_only = h.fields.get("blast_ability_fire_projectile_type").is_some_and(|v| !v.trim().is_empty());
        let projectile = h.projectile.as_deref().filter(|_| !ability_only).and_then(|p| self.proj_index.get(&p.to_ascii_lowercase()).copied());
        let proj = projectile.map(|p| &self.projectiles[p as usize]);
        let t = h.hp_type.to_ascii_uppercase();
        let kind = if t.contains("SHIELD_GENERATOR") {
            HpKind::ShieldGenerator
        } else if t.contains("ENGINE") {
            HpKind::Engine
        } else if t.contains("FIGHTER_BAY") {
            HpKind::FighterBay
        } else if t.contains("TRACTOR") {
            HpKind::TractorBeam
        } else if t.contains("WEAPON") {
            match proj {
                Some(p) if p.hull_damage || p.shield_damage => HpKind::Weapon,
                Some(_) => HpKind::Facing,
                None => HpKind::Other,
            }
        } else {
            HpKind::Other
        };
        let mut inaccuracy = [0.0f32; TargetClass::COUNT];
        for (cat, dist) in &h.fire_inaccuracy {
            if let Some(c) = TargetClass::from_token(cat) {
                inaccuracy[c as usize] = *dist;
            }
        }
        // `Fire_Category_Restrictions` lists the classes the weapon must NOT fire at (only ion cannons
        // use it, forbidding fighters, bombers, gunships, transports and corvettes).
        let forbidden = h.fire_category_restrictions.iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit());
        let allowed = if forbidden == 0 { 0 } else { 0x1FF & !forbidden };
        let f = |k: &str, d: f32| h.fields.get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
        let _ = (index, class);
        HpDef {
            name: h.name.clone(),
            kind,
            health: h.health.unwrap_or(0.0).max(0.0),
            destroyable: h.is_destroyable && h.health.unwrap_or(0.0) > 0.0,
            targetable: h.is_targetable,
            offset,
            forward,
            normal,
            left,
            arc_yaw,
            arc_pitch,
            arc_fallback,
            mirrored,
            bone_y: y_axis,
            turret: h.turret.is_some(),
            turret_yaw,
            turret_pitch,
            projectile,
            range: h.fire_range,
            min_range: h.fire_min_range,
            half_yaw,
            half_pitch,
            recharge_min: h.fire_min_recharge.max(0.05),
            recharge_max: h.fire_max_recharge.max(h.fire_min_recharge).max(0.05),
            pulses: h.fire_pulse_count.max(1),
            pulse_delay: h.fire_pulse_delay.max(0.0),
            inaccuracy,
            allowed,
            full_salvo_mult: f("full_salvo_weapon_delay_multiplier", 1.0),
            power_weapons_mult: f("power_to_weapons_mode_weapon_delay_multiplier", 1.0),
            from_bone,
            flyout: if kind == HpKind::FighterBay { f("fighter_bay_flyout_distance", 0.0).max(0.0) } else { 0.0 },
            arc_bins: 0,
            dps: 0.0,
        }
    }

    /// Builds a catalog from hand-written definitions (tests, tools). Ship and projectile names
    /// become lookup keys.
    pub fn from_defs(ships: Vec<ShipDef>, projectiles: Vec<ProjDef>) -> Catalog {
        let ship_index = ships.iter().enumerate().map(|(i, s)| (s.name.to_ascii_lowercase(), i as ShipId)).collect();
        let proj_index = projectiles.iter().enumerate().map(|(i, p)| (p.name.to_ascii_lowercase(), i as ProjId)).collect();
        Catalog { ships, projectiles, speed_scale: 30.0, shield_interval: 1.0, ship_index, proj_index, ..Catalog::default() }
    }

    /// Ship definition by id.
    pub fn ship(&self, id: ShipId) -> &ShipDef {
        &self.ships[id as usize]
    }

    /// Ship id by case-insensitive name.
    pub fn ship_id(&self, name: &str) -> Option<ShipId> {
        self.ship_index.get(&name.trim().to_ascii_lowercase()).copied()
    }

    /// Projectile id by case-insensitive name.
    pub fn projectile_id(&self, name: &str) -> Option<ProjId> {
        self.proj_index.get(&name.trim().to_ascii_lowercase()).copied()
    }
}

fn ability_defs(u: &SpaceUnitDef, projectiles: &[ProjDef], proj_index: &HashMap<String, ProjId>) -> Vec<AbilityDef> {
    u.abilities
        .iter()
        .filter(|a| {
            a.expiration_seconds.unwrap_or(0.0) > 0.0
                || (a.kind.eq_ignore_ascii_case("LURE") && a.fields.contains_key("effective_radius"))
                || ["FULL_SALVO", "TRACTOR_BEAM", "MISSILE_SHIELD", "INTERDICT", "STEALTH", "FOW_REVEAL_PING", "LASER_DEFENSE", "ROCKET_ATTACK", "SWAP_WEAPONS", "SENSOR_JAMMING", "BUZZ_DROIDS", "SUPER_LASER", "CORRUPT_SYSTEMS", "HUNT", "SPOILER_LOCK", "REPLENISH_WINGMEN", "WEAKEN_ENEMY", "HARMONIC_BOMB", "ENERGY_WEAPON", "CLUSTER_BOMB"].iter().any(|k| a.kind.eq_ignore_ascii_case(k))
        })
        .map(|a| {
            let toggle = a.expiration_seconds.unwrap_or(0.0) <= 0.0 && ["MISSILE_SHIELD", "INTERDICT", "STEALTH", "HUNT", "SPOILER_LOCK"].iter().any(|k| a.kind.eq_ignore_ascii_case(k));
            let ping = a.kind.eq_ignore_ascii_case("FOW_REVEAL_PING");
            // `LASER_DEFENSE` (point defence turrets) shoots down missiles, rockets and torpedoes inside `Effective_Radius` for `Defense_Duration_In_Secs` of the linked ability.
            let laser = a.kind.eq_ignore_ascii_case("LASER_DEFENSE");
            let defence_secs = laser.then(|| {
                a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n))).and_then(|s| s.fields.get("defense_duration_in_secs")).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(30.0)
            });
            let m = |k: &str| a.mod_multipliers.iter().find(|(n, _)| n == k).map(|&(_, v)| v);
            let mut speed_mult = m("SPEED_MULTIPLIER").unwrap_or(1.0);
            let mut shield_regen_mult = m("SHIELD_REGEN_MULTIPLIER").unwrap_or(1.0).max(0.0);
            let mut weapon_delay_mult = m("WEAPON_DELAY_MULTIPLIER").unwrap_or(1.0);
            let mut take_damage_mult = m("TAKE_DAMAGE_MULTIPLIER").unwrap_or(1.0);
            let mut cause_damage_mult = m("CAUSE_DAMAGE_MULTIPLIER").unwrap_or(1.0);

            let k = a.kind.to_ascii_uppercase();
            match k.as_str() {
                "POWER_TO_WEAPONS" => {
                    if cause_damage_mult == 1.0 { cause_damage_mult = 1.35; }
                    if weapon_delay_mult == 1.0 { weapon_delay_mult = 0.6; }
                    if shield_regen_mult == 1.0 { shield_regen_mult = 0.0; }
                    if speed_mult == 1.0 { speed_mult = 0.75; }
                }
                "DEFEND" => {
                    if take_damage_mult == 1.0 { take_damage_mult = 0.65; }
                    if shield_regen_mult == 1.0 { shield_regen_mult = 1.6; }
                    if speed_mult == 1.0 { speed_mult = 0.65; }
                }
                "TURBO" | "SPRINT" => {
                    if speed_mult == 1.0 { speed_mult = 1.8; }
                    if weapon_delay_mult == 1.0 { weapon_delay_mult = 10.0; }
                }
                "CONCENTRATE_FIRE" => {
                    if weapon_delay_mult == 1.0 { weapon_delay_mult = 0.65; }
                    if cause_damage_mult == 1.0 { cause_damage_mult = 1.25; }
                    if take_damage_mult == 1.0 { take_damage_mult = 1.25; }
                }
                "MAXIMUM_FIREPOWER" => {
                    if weapon_delay_mult == 1.0 { weapon_delay_mult = 0.5; }
                    if cause_damage_mult == 1.0 { cause_damage_mult = 1.35; }
                }
                _ => {}
            }

            AbilityDef {
                kind: a.kind.clone(),
                speed_mult,
                shield_regen_mult,
                weapon_delay_mult,
                take_damage_mult,
                cause_damage_mult,
                recharge: a.recharge_seconds.unwrap_or(30.0),
                // A reveal ping has no expiration in the data: it shows the area for 5 s (estimate).
                duration: if toggle { f32::MAX / 4.0 } else if ping { a.expiration_seconds.filter(|&x| x > 0.0).unwrap_or(5.0) } else if let Some(secs) = defence_secs { secs } else if a.kind.eq_ignore_ascii_case("CORRUPT_SYSTEMS") { a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n))).and_then(|s| s.fields.get("duration_in_seconds")).and_then(|v| v.trim().parse().ok()).unwrap_or(30.0) } else { a.expiration_seconds.unwrap_or(12.0) },
                stealth: a.kind.eq_ignore_ascii_case("STEALTH"),
                reveal_radius: if ping { a.fields.get("effective_radius").and_then(|v| v.parse().ok()).unwrap_or(1000.0) } else { 0.0 },
                interdict: a.kind.eq_ignore_ascii_case("INTERDICT"),
                hunt: a.kind.eq_ignore_ascii_case("HUNT"),
                replenish: a.kind.eq_ignore_ascii_case("REPLENISH_WINGMEN"),
                lure: a.kind.eq_ignore_ascii_case("LURE").then(|| LureDef {
                    radius: a.fields.get("effective_radius").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                    types: a.fields.get("target_types").map(|v| alamo_data::xml::split_list(v).iter().map(|t| t.to_ascii_lowercase()).collect()).unwrap_or_default(),
                }).filter(|l| l.radius > 0.0),
                energy: a.kind.eq_ignore_ascii_case("ENERGY_WEAPON").then(|| {
                    let sub = a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
                    let f = |k: &str| sub.and_then(|s| s.fields.get(k)).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
                    EnergyDef {
                        max_range: f("activation_max_range"),
                        dps: f("damage_per_frame") * 30.0,
                        classes: sub.and_then(|s| s.fields.get("applicable_unit_categories")).map_or(0, |v| alamo_data::xml::split_list(v).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit())),
                    }
                }).filter(|e| e.dps > 0.0 && e.max_range > 0.0),
                cluster: a.kind.eq_ignore_ascii_case("CLUSTER_BOMB").then(|| {
                    let sub = a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
                    let f = |k: &str| sub.and_then(|s| s.fields.get(k)).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
                    let p = sub.and_then(|s| s.fields.get("bomb_type")).and_then(|n| proj_index.get(&n.trim().to_ascii_lowercase())).and_then(|&i| projectiles.get(i as usize));
                    ClusterDef {
                        delay: f("detonation_time_in_secs"),
                        count: f("number_of_bombs") as u32,
                        radius: p.map_or(0.0, |p| p.blast_range),
                        damage: p.map_or(0.0, |p| p.blast_damage),
                        spread: p.map_or(0.0, |p| p.speed) * f("detonation_time_in_secs"),
                    }
                }).filter(|c| c.count > 0 && c.damage > 0.0 && c.radius > 0.0),
                bomb: a.kind.eq_ignore_ascii_case("HARMONIC_BOMB").then(|| {
                    let p = a.fields.get("spawned_object_type").and_then(|n| proj_index.get(&n.trim().to_ascii_lowercase())).and_then(|&i| projectiles.get(i as usize));
                    let (heal_radius, heal_amount, heal_classes) = p.and_then(|p| p.instant_heal).unwrap_or_default();
                    BombDef {
                        countdown: a.fields.get("bomb_countdown_seconds").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                        heal_radius,
                        heal_amount,
                        heal_classes,
                        blast_radius: p.map_or(0.0, |p| p.blast_range),
                        blast_damage: p.map_or(0.0, |p| p.blast_damage),
                        blast_tiers: p.map_or(0, |p| p.blast_tiers),
                    }
                }).filter(|b| b.heal_amount > 0.0 || b.blast_damage > 0.0),
                weaken: a.kind.eq_ignore_ascii_case("WEAKEN_ENEMY").then(|| {
                    let p = a.fields.get("spawned_object_type").and_then(|n| proj_index.get(&n.trim().to_ascii_lowercase())).and_then(|&i| projectiles.get(i as usize)).and_then(|p| p.weaken.clone()).unwrap_or(WeakenDef { radius: 2000.0, duration: 60.0, cause_reduction: 0.25, take_increase: 0.0, speed_cut: 0.25 });
                    WeakenDef { radius: a.fields.get("effective_radius").and_then(|v| v.trim().parse().ok()).unwrap_or(p.radius), ..p }
                }),
                toggle,
                salvo: a.kind.eq_ignore_ascii_case("FULL_SALVO"),
                supports_autofire: a.fields.get("supports_autofire").is_some_and(|v| alamo_data::xml::parse_bool(v)),
                autocast_default: a.fields.get("active_by_default").is_some_and(|v| alamo_data::xml::parse_bool(v)),
                corrupt: a.kind.eq_ignore_ascii_case("CORRUPT_SYSTEMS").then(|| {
                    let sub = a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
                    let f = |k: &str, d: f32| sub.and_then(|s| s.fields.get(k)).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(d);
                    CorruptDef { max_range: f("activation_max_range", 1000.0), duration: f("duration_in_seconds", 30.0), drain: f("shield_drain_per_second", 100.0), pick_radius: f("inaccuracy_radius", 100.0) }
                }),
                jam: a.kind.eq_ignore_ascii_case("SENSOR_JAMMING"),
                super_laser: a.kind.eq_ignore_ascii_case("SUPER_LASER").then(|| {
                    let sub = a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
                    let f = |k: &str| sub.and_then(|s| s.fields.get(k)).cloned().unwrap_or_default();
                    SuperLaserDef {
                        delay: f("fire_frame").trim().parse::<f32>().unwrap_or(50.0) / 30.0,
                        classes: alamo_data::xml::split_list(&f("applicable_unit_categories")).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit()),
                        initial_cooldown: tsw_approach_seconds(
                            u.fields.get("tsw_start_distance_from_target").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                            u.fields.get("tsw_attack_distance_from_target").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                            u.fields.get("max_speed").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                        ),
                    }
                }),
                zone: a.kind.eq_ignore_ascii_case("BUZZ_DROIDS").then(|| {
                    let sub = a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
                    let f = |k: &str| sub.and_then(|s| s.fields.get(k)).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
                    ZoneDef { radius: f("damage_radius"), enemy_dps: f("enemy_damage_per_second"), max_range: f("activation_max_range") }
                }).filter(|z| z.radius > 0.0 && z.enemy_dps > 0.0),
                rocket: a.kind.eq_ignore_ascii_case("ROCKET_ATTACK") || a.kind.eq_ignore_ascii_case("SWAP_WEAPONS"),
                missile_radius: if a.kind.eq_ignore_ascii_case("MISSILE_SHIELD") || laser { a.fields.get("effective_radius").and_then(|v| v.parse().ok()).unwrap_or(750.0) } else { 0.0 },
                disable_movement: a.fields.get("disable_movement").is_some_and(|v| alamo_data::xml::parse_bool(v)),
                tractor: a.kind.eq_ignore_ascii_case("TRACTOR_BEAM").then(|| {
                    let sub = a.activated_ability.as_ref().and_then(|n| u.sub_abilities.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
                    let f = |k: &str| sub.and_then(|s| s.fields.get(k)).cloned().unwrap_or_default();
                    TractorDef {
                        range: f("activation_max_range").parse().unwrap_or(2500.0),
                        slow: f("target_speed_decrease_percent").trim().parse::<f32>().unwrap_or(0.5).clamp(0.0, 0.95),
                        classes: alamo_data::xml::split_list(&f("applicable_unit_categories")).iter().filter_map(|t| TargetClass::from_token(t)).fold(0u16, |m, c| m | c.bit()),
                    }
                }),
            }
        })
        .collect()
}

fn kind_is_weapon(h: &HardpointDef) -> bool {
    h.projectile.is_some() && h.hp_type.to_ascii_uppercase().contains("WEAPON")
}

/// Damage per second a definition can put out at its best bearing (hardpoints) or with its hull weapon (fighters); used for strength estimates.
pub fn dps_estimate(d: &ShipDef, cat: &Catalog) -> f32 {
    let hp = d.bearing_dps.iter().copied().fold(0.0f32, f32::max);
    if hp > 0.0 {
        return hp;
    }
    if let Some(w) = &d.direct
        && let Some(p) = cat.projectiles.get(w.projectile as usize)
    {
        let cycle = (w.pulses as f32 * w.pulse_delay + w.recharge).max(0.2);
        return p.damage * w.pulses as f32 / cycle;
    }
    d.members.iter().map(|(m, c)| dps_estimate(cat.ship(*m), cat) * *c as f32).sum()
}

/// Plan distance a fighter or bomber squadron keeps between neighbouring members (estimate). The Remake keeps a squadron's fighters
/// about two fighter widths apart (the user's 1080p video, b0703s at the default camera: 12 TIEs 36-51 px apart centre to centre,
/// about 100 units at 0.47 px per unit), while `Squadron_Offsets` set TIE fighters 20 units apart side by side: drawn at their screen
/// size they ran into a blob.
pub const SQUADRON_GAP: f32 = 100.0;
/// Largest widening of a formation (estimate): offsets the data puts (almost) on the same spot are pushed apart instead.
pub const SQUADRON_SPREAD_MAX: f32 = 5.0;

/// Formation offset `idx` of a squadron: the data's `Squadron_Offsets`, or a staggered row behind the leader where the data has fewer.
pub fn formation_offset(offsets: &[Vec3], idx: usize) -> Vec3 {
    offsets.get(idx).copied().unwrap_or(Vec3::new(-20.0 * idx as f32, 25.0 * (idx % 2) as f32, 0.0))
}

/// The places a squadron's members keep in flight: the plan (x, y) of `offsets` widened (heights as in the data) until the closest two
/// places are [`SQUADRON_GAP`] apart, at most [`SQUADRON_SPREAD_MAX`] times, never narrowed; then places still closer than the gap are
/// pushed apart, half each, in a fixed number of rounds (deterministic; places on the same spot part sideways).
pub fn spread_formation(offsets: &[Vec3]) -> Vec<Vec3> {
    let plan = |a: &Vec3, b: &Vec3| (a.x - b.x).hypot(a.y - b.y);
    let closest = offsets.iter().enumerate().flat_map(|(i, a)| offsets[i + 1..].iter().map(move |b| plan(a, b))).filter(|d| *d >= 1.0).fold(f32::MAX, f32::min);
    let k = (SQUADRON_GAP / closest).clamp(1.0, SQUADRON_SPREAD_MAX);
    let mut out: Vec<Vec3> = offsets.iter().map(|o| Vec3::new(o.x * k, o.y * k, o.z)).collect();
    for _ in 0..32 {
        let mut moved = false;
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                let (dx, dy) = (out[j].x - out[i].x, out[j].y - out[i].y);
                let d = dx.hypot(dy);
                if d >= SQUADRON_GAP - 1e-3 {
                    continue;
                }
                let (ux, uy) = if d > 1e-3 { (dx / d, dy / d) } else { (0.0, 1.0) };
                let push = (SQUADRON_GAP - d) * 0.5;
                out[i].x -= ux * push;
                out[i].y -= uy * push;
                out[j].x += ux * push;
                out[j].y += uy * push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    out
}

/// Estimate: launch offset in radians per radian/second of a missile's `Max_Rate_Of_Turn` (a fast-turning missile can afford a wider arc and
/// still reach its target; the data has no launch angle for missiles).
const MISSILE_ARC_PER_TURN_RATE: f32 = 0.17;
/// Estimate: widest launch offset of a missile in radians (about 34 degrees).
const MISSILE_ARC_MAX: f32 = 0.6;

/// Estimate: game units per second a `Max_Speed` of 1 stands for on the Death Star's approach (`TSW_*` values). The data gives the distances
/// (80000 -> 50000) and a speed of 5 but not the scale; the Remake video shows the countdown at about 600 s at the start, which is the factor 10.
const TSW_SPEED_SCALE: f32 = 10.0;

/// Seconds a Death Star needs to fly from `TSW_Start_Distance_From_Target` into `TSW_Attack_Distance_From_Target` at `Max_Speed`; 0 without data.
pub fn tsw_approach_seconds(start: f32, attack: f32, max_speed: f32) -> f32 {
    if start > attack && attack > 0.0 && max_speed > 0.0 { (start - attack) / (max_speed * TSW_SPEED_SCALE) } else { 0.0 }
}

#[cfg(test)]
mod formation_tests {
    use super::*;

    #[test]
    fn a_spread_formation_keeps_its_shape_and_parts_stacked_places() {
        // The Remake's TIE Fighter squadron: three rows of four, 20 units apart side by side (five times that is the gap), 40 between the
        // rows and 40 up or down.
        let tie: Vec<Vec3> = [40.0f32, 0.0, -40.0].iter().flat_map(|&x| [30.0f32, 10.0, -10.0, -30.0].map(move |y| Vec3::new(x, y, -x))).collect();
        for (a, b) in tie.iter().zip(spread_formation(&tie)) {
            assert_eq!((b.x, b.y, b.z), (a.x * 5.0, a.y * 5.0, a.z), "widened to the gap, the shape only scaled in the plan");
        }
        // TIE Interceptors: two pairs stacked on one spot (6 units apart in height) next to places 6 units off.
        let tin = [(0.0, 9.0, 0.0), (0.0, -9.0, 0.0), (25.0, -33.0, 0.0), (30.0, -30.0, -3.0), (30.0, -30.0, 3.0), (25.0, 33.0, 0.0), (30.0, 30.0, -3.0), (30.0, 30.0, 3.0)]
            .map(|(x, y, z)| Vec3::new(x, y, z));
        let s = spread_formation(&tin);
        for i in 0..s.len() {
            for j in i + 1..s.len() {
                let d = (s[i].x - s[j].x).hypot(s[i].y - s[j].y);
                assert!(d >= SQUADRON_GAP - 0.5, "places {i} and {j} only {d:.1} apart: {s:?}");
            }
        }
        assert_eq!(spread_formation(&tin), s, "deterministic");
        // A formation already wider than the gap stays as it is.
        let wide = [Vec3::new(0.0, 150.0, 0.0), Vec3::new(0.0, -150.0, 0.0), Vec3::new(-150.0, 0.0, 10.0)];
        assert_eq!(spread_formation(&wide), wide.to_vec());
    }
}

#[cfg(test)]
mod tsw_tests {
    use super::*;

    #[test]
    fn death_star_approach_time() {
        assert_eq!(tsw_approach_seconds(80000.0, 50000.0, 5.0), 600.0);
        assert_eq!(tsw_approach_seconds(0.0, 0.0, 5.0), 0.0);
        assert_eq!(tsw_approach_seconds(80000.0, 50000.0, 0.0), 0.0);
    }
}
