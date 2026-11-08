//! Battle state, setup, commands, events and results.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::defs::*;
use crate::math::Vec3;

/// Tag value for units without a fleet (structures, stations).
pub const NO_TAG: u32 = u32::MAX;
/// Tag of the results groups of reinforcements (`Command::Reinforce`).
pub const REINFORCE_TAG: u32 = u32::MAX - 1;
/// Tag of the results groups of super ship phases that came out of a destroyed phase (`Destruction_Survivors`): not units the galaxy sent.
pub const PHASE_TAG: u32 = u32::MAX - 2;
/// "No entity" marker for projectile targets.
pub const NONE: u32 = u32::MAX;

/// Tunable battle constants.
#[derive(Debug, Clone)]
pub struct BattleConfig {
    /// Fixed time step in seconds (default 1/30).
    pub dt: f32,
    /// Tech level of each side's owner (`Starting_` / `Reserve_Spawned_Units_Tech_N` pick the level they launch); missing = 0.
    pub side_tech: Vec<u8>,
    /// Big ships plan paths around rocks and stations and steer around hulls, fighters keep out of hulls (default on; off = the old
    /// behaviour of only being pushed out of overlaps, kept for comparisons).
    pub navigation: bool,
    /// Half size of the square battle area.
    pub half_size: f32,
    /// RNG seed.
    pub seed: u64,
    /// Record per-step events (projectile fired, hits, ...). Disable for headless speed.
    pub record_events: bool,
    /// Battle is a draw after this many seconds.
    pub max_time: f32,
    /// Seconds before retreat becomes possible (`Space_Retreat_Allowed_Countdown_Seconds`).
    pub retreat_countdown: f32,
    /// Seconds between a retreat order and the jump to hyperspace: ships turn to their spawn side and fly there meanwhile; when it runs out, units with a living drive
    /// escape. For sides without an entry in `side_retreat_jump`: 30, the faction `Space_Retreat_Countdown_Seconds` every Remake faction has.
    pub retreat_jump_seconds: f32,
    /// Per side: the faction's `Space_Retreat_Countdown_Seconds` (`FACTIONS.XML`). Missing entries use `retreat_jump_seconds`.
    pub side_retreat_jump: Vec<f32>,
    /// Seconds between target scans per ship.
    pub retarget_interval: f32,
    /// Population cap per side for reinforcements (`Space_Tactical_Unit_Cap`).
    pub unit_cap: i32,
    /// Fog of war: ships only target what a friendly ship sees (`Space_FOW_Reveal_Range` of each ship). Off = everything is visible.
    pub fog_of_war: bool,
    /// Seconds an enemy stays visible after it left every sensor range (`SpaceFOWRegrowTime`).
    pub fog_linger: f32,
    /// A stealthed ship is only seen by enemies within this distance (the data has no value: estimate).
    pub stealth_detection: f32,
    /// Seconds before reinforcements arrive (`Skirmish_Reinforcement_Delay_Frames` 150 at 30 fps).
    pub reinforce_delay: f32,
    /// Radius of the no-spawn zone around every enemy structure when its data has no `Reinforcement_Prevention_Radius` (none of the space structures
    /// has one; the Remake AI equations
    /// `Target_Is_Near_Friendly_Unit` / `Target_Is_Not_Close_To_Enemy_Units` use 2000 for "near", so that number is the estimate).
    pub no_spawn_radius: f32,
    /// Free space needed at the arrival point (`Space_Reinforcement_Collision_Check_Distance`).
    pub reinforce_clearance: f32,
    /// Use rayon for per-entity work.
    pub parallel: bool,
    /// Use the `<set>_Attack_Move` targeting sets (category exclusions and hardpoint sniping) for ships
    /// on `AttackMove` orders. The Remake scripts only assign those sets to ground task forces, so the
    /// default is off and space ships use their own `Targeting_Priority_Set`.
    pub attack_move_sets: bool,
    /// Scales the standoff distance ships keep from their target (1.0 = range holding 80% of their damage).
    pub engage_range_scale: f32,
    /// Shields only recharge after this many seconds without taking damage (0 = recharge continuously).
    /// Estimate: the data only gives `ShieldRechargeIntervalInSecs`, not whether combat pauses it.
    pub shield_regen_delay: f32,
    /// Arena (bounds, spawn markers, asteroids) from a space map; `None` = empty square of `half_size`.
    pub arena: Option<std::sync::Arc<crate::arena::Arena>>,
    /// Difficulty multipliers per side (index = side); missing entries mean 1.0.
    pub side_mults: Vec<SideMult>,
    /// Per side: may the AI of this side order a retreat when it is clearly losing (`Allowed_To_Retreat_Space`: only in the campaign, only for
    /// a faction holding more than one planet, never Pirates). Missing entries mean true.
    pub retreat_allowed: Vec<bool>,
    /// Damage carried over from earlier battles: ships of the group start with this share of hull and shield.
    pub start_health: Vec<StartHealth>,
    /// Stage carried squadron launches sequentially from hangar bay doors over time (defaults to true).
    pub staged_hangar_launch: bool,
    /// Per side: units waiting outside the battle that the side's AI calls in one at a time while the population cap has room
    /// (`Want_To_Reinforce_Units`; the galaxy's reinforcement pool). Missing entries mean none; sides without AI ignore it.
    pub ai_reserve: Vec<Vec<(ShipId, u32)>>,
    /// Per side: the side defends the planet (`Variable_Self.IsDefender` of the AI retreat rule), with or without a station. Missing entries:
    /// a side that brought structures defends.
    pub defenders: Vec<bool>,
    /// Per side: story grants `(unit name, ability name)`, both lower-cased. A locked redirect applies when this side's set contains the ship.
    pub powers: Vec<BTreeSet<(String, String)>>,
    /// Per side: battlefield modifiers the side brings besides those of its units, e.g. from upgrades (`FieldModDef::of_object`).
    /// Missing entries mean none.
    pub side_field_mods: Vec<Vec<crate::defs::FieldModDef>>,
    /// Per side: salvage abilities the side brings besides those of its units, e.g. from upgrades (`SalvageDef::of_object`). Missing
    /// entries mean none.
    pub side_salvage: Vec<Vec<crate::defs::SalvageDef>>,
    /// Per side: galactic credits the side may spend on tactical builds (`Tactical_Build_Cost_Campaign`). Missing entries: none.
    pub side_credits: Vec<i64>,
    /// Per side: level of its star base (`Variable_Self.BaseLevel` of the AI; 0 without one). Missing entries: 0.
    pub side_base_level: Vec<u32>,
}

impl Default for BattleConfig {
    fn default() -> Self {
        Self {
            dt: 1.0 / 30.0,
            navigation: true,
            side_tech: Vec::new(),
            half_size: 10_000.0,
            seed: 1,
            record_events: true,
            max_time: 3600.0,
            retreat_countdown: 60.0,
            retreat_jump_seconds: 30.0,
            side_retreat_jump: Vec::new(),
            retarget_interval: 0.4,
            unit_cap: 150,
            reinforce_delay: 5.0,
            fog_of_war: true,
            fog_linger: 15.0,
            stealth_detection: 500.0,
            no_spawn_radius: 2000.0,
            reinforce_clearance: 150.0,
            parallel: true,
            shield_regen_delay: 0.0,
            attack_move_sets: false,
            engage_range_scale: 1.0,
            arena: None,
            side_mults: Vec::new(),
            retreat_allowed: Vec::new(),
            start_health: Vec::new(),
            staged_hangar_launch: true,
            ai_reserve: Vec::new(),
            defenders: Vec::new(),
            powers: Vec::new(),
            side_field_mods: Vec::new(),
            side_salvage: Vec::new(),
            side_credits: Vec::new(),
            side_base_level: Vec::new(),
        }
    }
}

/// A group of identical units of one fleet.
#[derive(Debug, Clone, Copy)]
pub struct UnitSetup {
    /// Definition (a squadron expands into its fighters).
    pub ship: ShipId,
    /// How many.
    pub count: u32,
    /// Caller-defined fleet tag, reported back in results.
    pub tag: u32,
}

/// One side of a battle.
#[derive(Debug, Clone, Default)]
pub struct SideSetup {
    /// Display name (faction).
    pub name: String,
    /// Fleet units.
    pub units: Vec<UnitSetup>,
    /// Stations and structures (star base, orbital defenses); they never move.
    pub structures: Vec<ShipId>,
    /// Let the built-in AI give default orders to this side.
    pub ai: bool,
}

/// Everything needed to start a battle.
#[derive(Debug, Clone)]
pub struct BattleSetup {
    /// Definitions.
    pub catalog: Arc<Catalog>,
    /// Sides; index = side id.
    pub sides: Vec<SideSetup>,
    /// Constants.
    pub config: BattleConfig,
}

/// Fire behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    /// Chase and attack anything in sight.
    Aggressive,
    /// Fire at targets in range but do not leave position.
    Defensive,
    /// Do not fire unless explicitly ordered to attack.
    HoldFire,
}

/// Movement and attack order of a ship.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Order {
    /// No order: engage targets that come into range.
    Idle,
    /// Move to a point (still fires on the way).
    Move(Vec3),
    /// Move to a point, chasing enemies met on the way.
    AttackMove(Vec3),
    /// Attack a specific ship.
    Attack(u32),
    /// Guard a place: engage enemies near it, pursue them a limited way (`Guard_Chase_Range`) and return.
    Guard(Vec3),
    /// Guard (follow) a friendly ship: stay with it, engage enemies near it, pursue them a limited way and return.
    GuardShip(u32),
    /// Leave the battle area.
    Retreat,
}

/// Commands from the player or AI.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Move ships to a point.
    Move {
        /// Ships.
        ships: Vec<u32>,
        /// Destination.
        to: Vec3,
    },
    /// Turns the automatic use of an ability on or off (only abilities with `AbilityDef::supports_autofire`).
    SetAutocast {
        /// Ship.
        ship: u32,
        /// Index in the ship definition's abilities.
        ability: u16,
        /// New state.
        on: bool,
    },
    /// Attack-move ships to a point.
    AttackMove {
        /// Ships.
        ships: Vec<u32>,
        /// Destination.
        to: Vec3,
    },
    /// Attack an enemy ship.
    Attack {
        /// Ships.
        ships: Vec<u32>,
        /// Target ship.
        target: u32,
    },
    /// Attack one hardpoint of an enemy ship with all weapons that can reach it.
    AttackHardpoint {
        /// Ships.
        ships: Vec<u32>,
        /// Target ship.
        target: u32,
        /// Hardpoint index in the target's definition.
        hardpoint: u16,
    },
    /// Guard a place or follow and protect a friendly ship (the command bar's guard button, `TEXT_TOOLTIP_GUARD_BUTTON`).
    Guard {
        /// Ships.
        ships: Vec<u32>,
        /// What to guard.
        target: GuardTarget,
    },
    /// Stop moving and attacking.
    Stop {
        /// Ships.
        ships: Vec<u32>,
    },
    /// Trigger an ability.
    UseAbility {
        /// Ship.
        ship: u32,
        /// Index into the ship definition's abilities.
        ability: u16,
    },
    /// Change stance.
    SetStance {
        /// Ships.
        ships: Vec<u32>,
        /// New stance.
        stance: Stance,
    },
    /// Use an ability on a place: a `FOW_REVEAL_PING` shows the area around `pos` to the side.
    UseAbilityAt {
        /// Ship.
        ship: u32,
        /// Index into the ship definition's abilities.
        ability: u16,
        /// Target position.
        pos: Vec3,
    },
    /// Order the whole side to leave the battlefield.
    Retreat,
    /// Call the side's retreat off while its hyperspace jump is still being prepared (EaW: the Retreat button pressed again during the countdown);
    /// the units stop and wait for orders. Nothing happens when no retreat is running.
    CancelRetreat,
    /// A cheat of the F2 menu (see [`crate::cheats`]).
    Cheat(crate::cheats::BattleCheat),
    /// Buy a tactical build item offered by one of the side's structures (pays now, builds in the side's queue; see `buildables.rs`).
    TacticalBuild {
        /// Item (`Catalog::buildables`).
        item: crate::buildables::BuildId,
    },
    /// Take the last queued job of a tactical build item out of the queue; its credits come back.
    CancelTacticalBuild {
        /// Item.
        item: crate::buildables::BuildId,
    },
    /// Call in reinforcements (limited by the unit cap, arrive after a delay) at the side's reinforcement point.
    Reinforce {
        /// Definition.
        ship: ShipId,
        /// Number of units.
        count: u32,
    },
    /// Call in reinforcements at a chosen position (`Battle::reinforcement_zone_check` says whether it is allowed): near own ships or structures,
    /// away from the enemy, inside the map. They drop out of hyperspace there after the reinforcement delay.
    ReinforceAt {
        /// Definition.
        ship: ShipId,
        /// Number of units.
        count: u32,
        /// Where they arrive (battle-sim coordinates).
        pos: Vec3,
        /// Turn of the arrival heading against the side's hyperspace vector in radians (the placement ghost's rotation).
        yaw_offset: f32,
    },
}

/// Why a command was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Unknown ship, or it does not belong to the side or is dead.
    InvalidShip,
    /// Retreat is not allowed yet (countdown).
    RetreatNotAllowed,
    /// Ability unknown or not ready.
    AbilityUnavailable,
    /// Unit cap reached.
    UnitCap,
    /// Hardpoint index out of range.
    InvalidHardpoint,
    /// The battle is over.
    BattleOver,
    /// An enemy interdictor prevents retreating.
    RetreatBlocked,
    /// An own unit with `Retreat_Prevention_Ability` (Death Star II, Titan) prevents retreating.
    RetreatPrevented,
    /// The chosen reinforcement position is not in an area the side can see (fog of war).
    NotVisible,
    /// The chosen reinforcement position lies inside the no-spawn zone of an enemy structure.
    InNoSpawnZone,
    /// The chosen reinforcement position is outside the battle area.
    OutsideBattleArea,
    /// None of the side's structures offers this tactical build (or nothing to cancel).
    NotBuildable,
    /// The side has too few credits for the tactical build.
    NotEnoughCredits,
    /// The side already has (or builds) as many of the item as `Build_Limit_Current_For_All_Allies` allows.
    BuildLimit,
    /// A `Tactical_Build_Prerequisites` item is neither built nor queued.
    MissingPrerequisite,
}

/// What was hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitKind {
    /// Absorbed by shields.
    Shield,
    /// Hull damage with no hardpoint nearby.
    Hull,
    /// Damaged this hardpoint (index into the definition).
    Hardpoint(u16),
}

/// Something that happened in a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A projectile left a hardpoint or hull weapon.
    ProjectileFired {
        /// Firing ship.
        ship: u32,
        /// Hardpoint index, `None` for hull weapons.
        hardpoint: Option<u16>,
        /// Projectile type.
        projectile: ProjId,
        /// Muzzle position.
        from: Vec3,
        /// Initial velocity.
        velocity: Vec3,
    },
    /// A projectile hit a ship.
    Hit {
        /// Victim.
        ship: u32,
        /// Ship that fired the projectile.
        attacker: u32,
        /// Projectile type.
        projectile: ProjId,
        /// Impact point.
        pos: Vec3,
        /// What absorbed it.
        kind: HitKind,
        /// Damage dealt after armor multipliers.
        damage: f32,
    },
    /// A hardpoint was destroyed.
    HardpointDestroyed {
        /// Ship.
        ship: u32,
        /// Hardpoint index.
        hardpoint: u16,
        /// Function of the hardpoint.
        kind: HpKind,
    },
    /// A ship was destroyed.
    ShipDestroyed {
        /// Ship.
        ship: u32,
        /// Position at death.
        pos: Vec3,
    },
    /// A carrier or station launched a squadron.
    SquadronLaunched {
        /// Carrier.
        carrier: u32,
        /// New fighters.
        ships: Vec<u32>,
    },
    /// Reinforcements were accepted and are on their way (arrive after the reinforcement delay).
    ReinforcementsCalled {
        /// Side.
        side: u8,
        /// Ship definition.
        ship: ShipId,
        /// Units.
        count: u32,
    },
    /// A call for reinforcements was refused (unit cap).
    ReinforcementsRefused {
        /// Side.
        side: u8,
        /// Ship definition.
        ship: ShipId,
        /// Units.
        count: u32,
    },
    /// A reinforcement wave arrived.
    ReinforcementsArrived {
        /// Side.
        side: u8,
        /// New ships.
        ships: Vec<u32>,
    },
    /// A side ordered the retreat; its units jump to hyperspace after `jump_in` seconds.
    RetreatOrdered {
        /// Side.
        side: u8,
        /// Seconds until the jump.
        jump_in: f32,
    },
    /// A gravity well stopped the retreat of a side before its jump.
    RetreatAborted {
        /// Side.
        side: u8,
    },
    /// A side called its retreat off before the jump (`Command::CancelRetreat`).
    RetreatCancelled {
        /// Side.
        side: u8,
    },
    /// A ship left the battlefield.
    ShipRetreated {
        /// Ship.
        ship: u32,
    },
    /// An ability was activated.
    AbilityActivated {
        /// Ship.
        ship: u32,
        /// Ability index.
        ability: u16,
    },
    /// A `SUPER_LASER` was aimed: the charge runs for `fire_in` seconds, then the shot lands (`SuperLaserFired`).
    SuperLaserCharging {
        /// Firing ship.
        ship: u32,
        /// Chosen target.
        target: u32,
        /// Seconds until the shot (`Fire_Frame` / 30).
        fire_in: f32,
    },
    /// A `SUPER_LASER` shot was fired and its target destroyed.
    SuperLaserFired {
        /// Firing ship.
        ship: u32,
        /// Target.
        target: u32,
        /// Where the target was.
        at: Vec3,
    },
    /// A missile or torpedo was destroyed by a missile shield.
    ProjectileIntercepted {
        /// Projectile type.
        projectile: ProjId,
        /// Where it was destroyed.
        pos: Vec3,
    },
    /// A destroyed super ship phase gave way to the next (`Destruction_Survivors`).
    PhaseSpawned {
        /// The destroyed ship.
        ship: u32,
        /// The new ship.
        into: u32,
    },
    /// A ship entered an `Is_Nebula` zone (any ship; only `NEBULA` ships lose their abilities there).
    EnteredNebula {
        /// Ship.
        ship: u32,
    },
    /// A ship entered an `Is_Asteroid_Field` zone (any ship; only `ASTEROID_FIELD_DAMAGE` ships take damage there).
    EnteredAsteroidField {
        /// Ship.
        ship: u32,
    },
    /// `Asteroid_Field_Damage` landed on a ship (it also reports an [`Event::Hit`] with projectile [`NO_PROJ`]).
    AsteroidFieldHit {
        /// Ship.
        ship: u32,
        /// Where it landed.
        pos: Vec3,
    },
    /// An object script brought a unit into the battle (a hero's escort, Subjugator reinforcements), or an ability placed a helper object
    /// (repair droids, jamming pods; `hyperspace` false). See `spawns.rs`. Such units belong to the battle only.
    Spawned {
        /// Side the unit fights for.
        side: u8,
        /// New ships (a squadron's craft).
        ships: Vec<u32>,
        /// The unit whose script spawned it.
        by: u32,
        /// It dropped out of hyperspace (else it appeared in place).
        hyperspace: bool,
    },
    /// A tactical build was paid and queued (`Command::TacticalBuild`).
    BuildQueued {
        /// Side.
        side: u8,
        /// Item (`Catalog::buildables`).
        item: crate::buildables::BuildId,
    },
    /// A queued tactical build left the queue and its credits came back.
    BuildCancelled {
        /// Side.
        side: u8,
        /// Item.
        item: crate::buildables::BuildId,
    },
    /// A tactical build finished.
    BuildFinished {
        /// Side.
        side: u8,
        /// Item.
        item: crate::buildables::BuildId,
    },
    /// The battle ended.
    BattleOver {
        /// Winning side, `None` for a draw.
        winner: Option<u8>,
    },
}

/// Survivors of one setup group.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupResult {
    /// Side.
    pub side: u8,
    /// Fleet tag from the setup.
    pub tag: u32,
    /// Definition from the setup.
    pub ship: ShipId,
    /// Units that started.
    pub total: u32,
    /// Units still alive or retreated at the end (a squadron counts if at least half of it did).
    pub survived: u32,
    /// Units that left the battle by retreating (subset of `survived`).
    pub retreated: u32,
    /// `(hull, shield)` as a fraction of the maximum for every surviving ship of the group (single ships only, not squadrons),
    /// healthiest first.
    pub health: Vec<(f32, f32)>,
}

/// Outcome of a battle.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleResult {
    /// Winning side; `None` for a draw or mutual destruction.
    pub winner: Option<u8>,
    /// Battle duration in seconds.
    pub duration: f32,
    /// Survivors per setup group.
    pub groups: Vec<GroupResult>,
    /// Net credits delta per side for the galaxy: attrition penalties (negative) and refunds (positive), minus the credits spent on
    /// tactical builds, plus the salvage credits.
    pub attrition_credits: Vec<i32>,
    /// Salvage credits per side (`Bounty_On_Faction_Ability`; the summary's "Salvage collected"). Already included in `attrition_credits`.
    pub salvage_credits: Vec<i32>,
    /// `GlobalValue.Set` calls the ships' `GameObject` scripts made in the battle, in order (`Set_SSD_*_State`: the phase level of a super
    /// ship when it appears, 0 when its last phase dies), see [`ShipDef::on_create`].
    pub globals: Vec<(String, i32)>,
}

/// Ability runtime state.
#[derive(Debug, Clone, Copy, Default)]
pub struct AbilityState {
    /// Seconds of effect left.
    pub active: f32,
    /// Seconds until usable again.
    pub cooldown: f32,
    /// Target of a targeted ability (tractor beam); `NONE` when unused.
    pub target: u32,
    /// Automatic use: the ship uses the ability like the AI does (set with `Command::SetAutocast`; starts on for abilities with
    /// `Active_By_Default`). Ships of AI sides use every ability automatically whatever this says.
    pub autocast: bool,
}

/// Hardpoint runtime state.
#[derive(Debug, Clone, Copy)]
pub struct HpState {
    /// Remaining health.
    pub health: f32,
    /// False once destroyed.
    pub alive: bool,
    /// Seconds until the next volley may start.
    pub cooldown: f32,
    /// Pulses still to fire in the current volley.
    pub pulses_left: u32,
    /// Seconds until the next pulse.
    pub pulse_timer: f32,
    /// Target of the current volley.
    pub target: u32,
}

/// A launched or starting squadron of a carrier.
#[derive(Debug, Clone)]
pub struct SquadSlot {
    /// Squadron definition.
    pub def: ShipId,
    /// Fighters of the current incarnation.
    pub members: Vec<u32>,
    /// Seconds until a replacement launches (when all members are dead).
    pub respawn_in: f32,
}

/// A ship, fighter, station or structure in the battle.
#[derive(Debug, Clone)]
pub struct Ship {
    /// Index in [`Battle::ships`] (stable).
    pub id: u32,
    /// Definition.
    pub def: ShipId,
    /// Side.
    pub side: u8,
    /// Position.
    pub pos: Vec3,
    /// Heading in radians (0 = +X).
    pub yaw: f32,
    /// Current speed, units per second.
    pub speed: f32,
    /// Hull points.
    pub hull: f32,
    /// Shield points.
    pub shield: f32,
    /// False once destroyed.
    pub alive: bool,
    /// True once it flew out of the battle area.
    pub retreated: bool,
    /// Hardpoints.
    pub hps: Vec<HpState>,
    /// Current order.
    pub order: Order,
    /// Stance.
    pub stance: Stance,
    /// Current auto target.
    pub target: u32,
    /// Hardpoint of the target being aimed at (`u16::MAX` = none).
    pub target_hp: u16,
    /// Hull weapon cooldown.
    pub direct_cd: f32,
    /// Hull weapon pulses left.
    pub direct_pulses: u32,
    /// Hull weapon pulse timer.
    pub direct_timer: f32,
    /// Special ground-to-space weapon cooldown.
    pub special_cd: f32,
    /// Special weapon pulses left.
    pub special_pulses: u32,
    /// Special weapon pulse timer.
    pub special_timer: f32,
    /// Special weapon target.
    pub special_target: u32,
    /// Distance a launched fighter still has to fly level out of its hangar before it follows orders (`Fighter_Bay_Flyout_Distance`).
    pub flyout_left: f32,
    /// Direction of the first leg out of the hangar, flown until the fighter is clear of its carrier's hull (zero once it is, or when the
    /// way out is level from the start).
    pub flyout_exit: Vec3,
    /// Setup group (for results), `NONE` for launched/reinforcement units.
    pub group: u32,
    /// Squadron index for fighters, `NONE` otherwise.
    pub squad: u32,
    /// Formation offset relative to the squadron leader.
    pub formation: Vec3,
    /// Carrier that launched this fighter.
    pub carrier: u32,
    /// Squadron slots (carriers and stations).
    pub slots: Vec<SquadSlot>,
    /// Remaining replacement squadrons.
    pub reserve: Vec<(ShipId, u32)>,
    /// Abilities.
    pub abilities: Vec<AbilityState>,
    /// Seconds since the ship last took damage.
    pub since_hit: f32,
    /// Seconds left before a depleted shield may regenerate again (`Depleted_Shield_Disable_Time`).
    pub shield_lock: f32,
    /// Inside an `Is_Asteroid_Field` zone this step.
    pub in_asteroid_field: bool,
    /// Inside an `Is_Nebula` zone this step.
    pub in_nebula: bool,
    /// Seconds the current target has been the target (`Targeting_Stickiness_Time_Threshold`).
    pub target_age: f32,
    /// Where the ship last rested idle without a target: the place `Idle_Chase_Range` is measured from.
    pub rest_at: Vec3,
    /// A destroyed ship whose `Destruction_Survivors` ships were spawned and whose `Death_Clone` script ran.
    pub phase_done: bool,
    /// Seconds left before the ship may use special abilities again (`Nebula_Ability_Disable_Time` after it left a nebula).
    pub ability_lock: f32,
    /// Tractor beam speed reduction currently applied to this ship (0..1).
    pub tractor_cut: f32,
    /// Planned route of a big ship that is going somewhere: the waypoints still ahead, the last one being the destination. Empty when the
    /// ship is not moving to a place or the straight line is clear.
    pub path: Vec<Vec3>,
    pub(crate) path_goal: Vec3,
    pub(crate) path_age: f32,
    /// Ion stun seconds left.
    pub stun: f32,
    /// Ion speed reduction fraction while stunned.
    pub stun_speed_cut: f32,
    /// Ion fire rate reduction fraction while stunned.
    pub stun_rate_cut: f32,
    /// `BLAST` charged: the blast hardpoints that have not fired their blast shot yet (`None` = no blast armed).
    pub blast_pending: Option<Vec<u16>>,
    /// Shield generator still alive (or none exists).
    pub shield_gen_ok: bool,
    /// Speed factor from destroyed engines.
    pub engine_factor: f32,
    /// `HUNT`: the map cell (`HUNT_GRID` x `HUNT_GRID`) the hunter is searching, `NO_CELL` when it has none.
    pub hunt_cell: u8,
    /// `HUNT`: seconds since the hunter chose its search cell.
    pub hunt_since: f32,
    /// Fighter strafing phase timer (positive = breaking away).
    pub phase: f32,
    /// Fighter break-away heading change.
    pub phase_turn: f32,
    /// Sideways position in the fleet line (battle units, from the fleet axis); under attack-move orders the ship keeps this offset so the fleet advances line abreast.
    pub lane: f32,
    /// Nearby enemies sorted by priority and distance.
    pub candidates: Vec<u32>,
    pub(crate) scan_in: f32,
    /// Game time until which each side (the first four) still sees this ship (fog of war).
    pub(crate) seen_until: [f32; 4],
    /// Game time at which this ship last damaged a unit of each side (the first four): that side's AI knows where it is for a while, even
    /// when it cannot see it (the shots show where they come from).
    pub(crate) hit_side_at: [f32; 4],
    /// Seconds until each of the first two `ShipDef::heals` pulses.
    pub(crate) heal_in: [f32; 2],
    /// Multipliers from friendly `Combat_Bonus_Ability` auras (refreshed once per second).
    pub(crate) bonus: Bonus,
    /// Active `Projectile_Combat_Mod`s per `ModKind`: value and seconds left.
    pub(crate) mods: [(f32, f32); ModKind::COUNT],
    /// AI formation: the slot this ship holds in its side's advancing group, the group's speed and heading (`None` = not in a group).
    pub(crate) group_slot: Option<(Vec3, f32, f32)>,
    /// Group move: this ship's place in the formation at the destination (`None` = not in one; see `formation.rs`).
    pub(crate) move_slot: Option<crate::formation::MoveSlot>,
    /// AI: the side's priority target, focused when it is within reach (`NONE` = none).
    pub(crate) focus: u32,
    /// Hit points lost per second on its own (helper objects: `ShipDef::decay` plus the placing ability's `Own_Damage_Per_Second`).
    pub decay: f32,
}


/// A projectile in flight.
#[derive(Debug, Clone, Copy)]
pub struct Projectile {
    /// Position.
    pub pos: Vec3,
    /// Velocity (units per second).
    pub vel: Vec3,
    /// Definition.
    pub def: ProjId,
    /// Firing side.
    pub side: u8,
    /// Firing ship.
    pub owner: u32,
    /// Homing target.
    pub target: u32,
    /// Hardpoint that should absorb the hit (`u16::MAX` = nearest).
    pub aim_hp: u16,
    /// Remaining flight distance.
    pub range_left: f32,
}

/// A scheduled reinforcement wave.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reinforcement {
    pub at: f32,
    pub side: u8,
    pub ship: ShipId,
    pub count: u32,
    /// Chosen arrival position (`None` = the side reinforcement point).
    pub pos: Option<Vec3>,
    /// Turn of the arrival heading against the side's hyperspace vector.
    pub yaw_offset: f32,
}

/// Group bookkeeping for results.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    pub side: u8,
    pub tag: u32,
    pub ship: ShipId,
    pub members: Vec<u32>,
}

pub const NO_HP: u16 = u16::MAX;
/// [`Event::Hit`] projectile of damage that no projectile dealt (asteroid fields).
pub const NO_PROJ: ProjId = ProjId::MAX;

/// Cells per side of the square the map is cut into for the `HUNT` search (8 x 8 = 64, one bit each in a `u64`).
pub const HUNT_GRID: usize = 8;
/// `Ship::hunt_cell` of a hunter that has no search cell.
pub const NO_CELL: u8 = u8::MAX;

/// The battle simulation.
pub struct Battle {
    pub(crate) catalog: Arc<Catalog>,
    pub(crate) config: BattleConfig,
    pub(crate) sides: Vec<SideSetup>,
    /// All ships ever present (dead ones stay so ids remain valid).
    pub ships: Vec<Ship>,
    /// Projectiles in flight.
    pub projectiles: Vec<Projectile>,
    pub(crate) squads: Vec<Vec<u32>>,
    pub(crate) groups: Vec<Group>,
    /// `GlobalValue.Set` calls of the ships' scripts so far ([`BattleResult::globals`]).
    pub(crate) globals: Vec<(String, i32)>,
    pub(crate) start_health_used: std::collections::HashMap<(u8, u32, ShipId), usize>,
    pub(crate) pending: Vec<Reinforcement>,
    pub(crate) events: Vec<Event>,
    pub(crate) tick: u64,
    pub(crate) time: f32,
    pub(crate) over: bool,
    pub(crate) winner: Option<u8>,
    pub(crate) spawn_pos: Vec<Vec3>,
    pub(crate) side_places: Vec<Vec<(Vec3, f32)>>,
    /// Events caused by commands; they are reported with the next step (a step starts with a clean event list).
    pub(crate) cmd_events: Vec<Event>,
    pub(crate) retreat_ordered: Vec<bool>,
    /// Per side: whether its fleet was battered (the nearly-destroyed clause of `ai_retreat`) when the AI ordered its retreat.
    pub(crate) retreat_battered: Vec<Option<bool>>,
    /// Cheats in force (the F2 menu); see [`crate::cheats`].
    pub cheats: crate::cheats::Cheats,
    /// Per side: battle time at which the retreat jump happens (`None` = no retreat ordered).
    pub(crate) retreat_jump_at: Vec<Option<f32>>,
    /// `HUNT`: per side, the map cells (bit per cell of the `HUNT_GRID` square) that a hunter has already searched.
    pub(crate) hunt_seen: [u64; 4],
    /// Where the ability being activated is aimed (`Command::UseAbilityAt`); `None` = at the ship itself.
    pub(crate) ability_at: Option<Vec3>,
    /// `LURE`: (ship, caster) pairs of enemies currently forced to attack a lure caster.
    pub(crate) lured: Vec<(u32, u32)>,
    /// `HARMONIC_BOMB`s waiting to go off: side, place, time, what they do.
    pub(crate) bombs: Vec<(u8, Vec3, f32, BombDef)>,
    /// `ENERGY_WEAPON` beams: shooter, target, end time, hull damage per second.
    pub(crate) beams: Vec<(u32, u32, f32, f32)>,
    /// `CLUSTER_BOMB`s waiting to go off: side, place, time, what they do.
    pub(crate) clusters: Vec<(u8, Vec3, f32, ClusterDef)>,
    pub(crate) grid_cells: Vec<Vec<u32>>,
    /// Areas revealed to a side for a while: `(side, centre, radius, until)`.
    pub(crate) reveals: Vec<(u8, Vec3, f32, f32)>,
    /// Damaging areas (`BUZZ_DROIDS`): owner side, centre, radius, enemy damage per second, end time.
    /// Super laser shots on their way: shooter, target, firing time.
    pub(crate) lasers: Vec<(u32, u32, f32)>,
    /// Shield drains (`CORRUPT_SYSTEMS`): target, shield points per second, end time.
    pub(crate) drains: Vec<(u32, f32, f32)>,
    /// Shot abilities fired (`LUCKY_SHOT`): shooter, projectile, target, damage factor, time the factor lapses. The first hit of that shooter's
    /// projectile on the target takes it.
    pub(crate) shots: Vec<(u32, ProjId, u32, f32, f32)>,
    /// Charging shield flares (`SHIELD_FLARE`): ship, time the wave goes off.
    pub(crate) flares: Vec<(u32, f32)>,
    /// Shield leech beams (`LEECH_SHIELDS`): caster, ability index, target.
    pub(crate) leeches: Vec<(u32, u16, u32)>,
    /// Areas dropped by `BUZZ_DROIDS`: side, centre, radius, enemy damage per second, end time, missile-jamming radius (0 = none).
    pub(crate) zones: Vec<(u8, Vec3, f32, f32, f32, f32)>,
    /// Salvage credits each side earned so far (`Bounty_On_Faction_Ability`).
    pub(crate) salvage: Vec<f32>,
    /// Distance field of rocks and stations for path planning (`None` without navigation or obstacles).
    pub(crate) nav_grid: Option<crate::nav::NavGrid>,
    /// Per side: `(search point index, time it was set)` of the AI search for hidden enemies.
    pub(crate) search: Vec<(u32, f32)>,
    /// Per side: the tactical AI's formation, priority target and reserve (see `ai.rs`).
    pub(crate) ai: Vec<crate::ai::AiSide>,
    /// Group moves under way, in the order they were given (see `formation.rs`).
    pub(crate) formations: Vec<crate::formation::Formation>,
    /// Units object scripts spawned that have not arrived yet (see `spawns.rs`).
    pub(crate) scripted: Vec<crate::spawns::ScriptedArrival>,
    /// Ships below this index have had their spawn script started.
    pub(crate) scripts_seen: usize,
    /// Per side: galactic credits left for tactical builds, and spent so far (net of refunds).
    pub(crate) credits: Vec<i64>,
    pub(crate) spent: Vec<i64>,
    /// Production queues of all sides, in order (see `buildables.rs`).
    pub(crate) build_jobs: Vec<crate::buildables::BuildJob>,
    /// Per side: finished tactical builds.
    pub(crate) built: Vec<Vec<crate::buildables::BuildId>>,
    /// Debuff items finished in this battle: (time their auras start, item).
    pub(crate) debuffs: Vec<(f32, crate::buildables::BuildId)>,
}

impl Battle {
    /// Elapsed battle time.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Number of steps simulated.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// True once the battle ended.
    pub fn is_over(&self) -> bool {
        self.over
    }

    /// The winning side (`None` if still ongoing or a draw).
    pub fn winner(&self) -> Option<u8> {
        self.winner
    }

    /// Definitions used by this battle.
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    /// Configuration.
    pub fn config(&self) -> &BattleConfig {
        &self.config
    }

    /// Events of the last step (empty unless `record_events`).
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Number of sides.
    pub fn side_count(&self) -> usize {
        self.sides.len()
    }

    /// The ship that leads squadron `squad`: its first member alive (the member nearest the middle of the formation, see `lead_order`).
    pub fn squad_leader(&self, squad: u32) -> Option<u32> {
        self.squads.get(squad as usize)?.iter().copied().find(|&i| self.ships[i as usize].alive)
    }

    /// Initial spawn / arrival yaw for `side` (from `side_places`).
    pub fn spawn_yaw(&self, side: u8) -> f32 {
        self.side_places.get(side as usize).and_then(|p| p.first()).map_or(if side == 0 { 0.0 } else { std::f32::consts::PI }, |p| p.1)
    }

    /// Alive ships (not retreated) of a side, optionally only those with weapons.
    pub fn alive_count(&self, side: u8) -> usize {
        self.ships.iter().filter(|s| s.side == side && s.alive && !s.retreated).count()
    }

    /// Total remaining hull + shield of a side's living ships.
    pub fn side_health(&self, side: u8) -> f32 {
        self.ships.iter().filter(|s| s.side == side && s.alive && !s.retreated).map(|s| s.hull + s.shield).sum()
    }

    /// Stable hash of the full state, for determinism checks.
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.tick.hash(&mut h);
        for s in &self.ships {
            (s.alive, s.retreated, s.pos.x.to_bits(), s.pos.y.to_bits(), s.yaw.to_bits(), s.hull.to_bits(), s.shield.to_bits()).hash(&mut h);
            for hp in &s.hps {
                (hp.alive, hp.health.to_bits(), hp.pulses_left).hash(&mut h);
            }
        }
        for p in &self.projectiles {
            (p.pos.x.to_bits(), p.pos.y.to_bits(), p.pos.z.to_bits(), p.def, p.owner).hash(&mut h);
        }
        h.finish()
    }

    /// The result so far (final once [`Battle::is_over`]).
    pub fn result(&self) -> BattleResult {
        let mut groups = Vec::new();
        for g in &self.groups {
            let total = g.members.len() as u32;
            let ok = g.members.iter().filter(|&&m| self.ships[m as usize].alive).count() as u32;
            let ret = g.members.iter().filter(|&&m| self.ships[m as usize].alive && self.ships[m as usize].retreated).count() as u32;
            let def = self.catalog.ship(g.ship);
            // A squadron is one unit made of several fighters.
            let (t, s, r) = if def.is_squadron {
                let half = total.div_ceil(2);
                (1, u32::from(ok >= half && ok > 0), u32::from(ok >= half && ret >= half))
            } else {
                (total, ok, ret)
            };
            let health: Vec<(f32, f32)> = if def.is_squadron {
                Vec::new()
            } else {
                g.members
                    .iter()
                    .map(|&m| &self.ships[m as usize])
                    .filter(|s| s.alive)
                    .map(|s| ((s.hull / def.hull.max(1.0)).clamp(0.0, 1.0), if def.shield > 0.0 { (s.shield / def.shield).clamp(0.0, 1.0) } else { 1.0 }))
                    .collect()
            };
            match groups.iter_mut().find(|x: &&mut GroupResult| x.side == g.side && x.tag == g.tag && x.ship == g.ship) {
                Some(e) => {
                    e.total += t;
                    e.survived += s;
                    e.retreated += r;
                    e.health.extend(health.iter().copied());
                }
                None => groups.push(GroupResult { side: g.side, tag: g.tag, ship: g.ship, total: t, survived: s, retreated: r, health: health.clone() }),
            }
        }
        for g in &mut groups {
            g.health.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.total_cmp(&a.1)));
        }
        let mut attrition_credits = vec![0i32; self.sides.len()];
        for s in &self.ships {
            if !s.alive && !s.retreated {
                let def = self.catalog.ship(s.def);
                if (s.side as usize) < attrition_credits.len() {
                    attrition_credits[s.side as usize] += def.attrition_credits;
                }
            }
        }
        let salvage_credits: Vec<i32> = (0..self.sides.len()).map(|i| self.salvage.get(i).copied().unwrap_or(0.0).round() as i32).collect();
        for (a, s) in attrition_credits.iter_mut().zip(&salvage_credits) {
            *a += s;
        }
        // Credits spent on tactical builds leave the battle with the result.
        for (a, s) in attrition_credits.iter_mut().zip(&self.spent) {
            *a -= *s as i32;
        }
        BattleResult { winner: self.winner, duration: self.time, groups, attrition_credits, salvage_credits, globals: self.globals.clone() }
    }
}

/// Starting health of the single ships of one setup group (`UnitSetup`) or of a structure group (`tag` = `NO_TAG`).
#[derive(Debug, Clone, PartialEq)]
pub struct StartHealth {
    /// Side.
    pub side: u8,
    /// Fleet tag of the group.
    pub tag: u32,
    /// Definition.
    pub ship: ShipId,
    /// `(hull, shield)` fractions, one per ship in spawn order; ships beyond the list start at full health.
    pub health: Vec<(f32, f32)>,
}

/// Damage, hull and shield multipliers of one side (`DifficultyAdjustments.xml`). Damage dealt by the side is scaled
/// by `damage`; damage it takes is divided by `health` (hull) or `shield`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SideMult {
    /// Multiplier on damage dealt.
    pub damage: f32,
    /// Multiplier on hull points.
    pub health: f32,
    /// Multiplier on shield points.
    pub shield: f32,
}

impl Default for SideMult {
    fn default() -> Self {
        Self { damage: 1.0, health: 1.0, shield: 1.0 }
    }
}

impl Ship {
    /// Multipliers from friendly combat bonuses (auras and tactical builds) at the last step.
    pub fn bonus(&self) -> Bonus {
        self.bonus
    }

    /// Current value of a timed `Projectile_Combat_Mod` / `WEAKEN_ENEMY` effect on this ship (0 = none; -0.25 = 25% less).
    pub fn mod_value(&self, kind: ModKind) -> f32 {
        self.mods[kind as usize].0
    }
}

/// What a [`Command::Guard`] protects.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GuardTarget {
    /// A place.
    Place(Vec3),
    /// A ship of the own side (the guards follow it).
    Ship(u32),
}
