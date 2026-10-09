//! Battle state, setup, commands, events and results.

use std::sync::Arc;

use crate::defs::*;
use crate::math::Vec3;

/// Tag value for units without a fleet (structures, stations).
pub const NO_TAG: u32 = u32::MAX;
/// Tag of the results groups of reinforcements (`Command::Reinforce`).
pub const REINFORCE_TAG: u32 = u32::MAX - 1;
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
    /// escape. **Estimate** (the user named 10 or 20 s; the data has no value).
    pub retreat_jump_seconds: f32,
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
            retreat_jump_seconds: 10.0,
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
    /// Hold a position, engaging within weapon range only.
    Guard(Vec3),
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
    /// The chosen reinforcement position is not in an area the side can see (fog of war).
    NotVisible,
    /// The chosen reinforcement position lies inside the no-spawn zone of an enemy structure.
    InNoSpawnZone,
    /// The chosen reinforcement position is outside the battle area.
    OutsideBattleArea,
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
    /// A missile or torpedo was destroyed by a missile shield.
    ProjectileIntercepted {
        /// Projectile type.
        projectile: ProjId,
        /// Where it was destroyed.
        pos: Vec3,
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
    /// Shield generator still alive (or none exists).
    pub shield_gen_ok: bool,
    /// Speed factor from destroyed engines.
    pub engine_factor: f32,
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
    /// Seconds until each of the first two `ShipDef::heals` pulses.
    pub(crate) heal_in: [f32; 2],
    /// Multipliers from friendly `Combat_Bonus_Ability` auras (refreshed once per second).
    pub(crate) bonus: Bonus,
    /// Active `Projectile_Combat_Mod`s per `ModKind`: value and seconds left.
    pub(crate) mods: [(f32, f32); ModKind::COUNT],
}

/// Multipliers a ship gets from friendly combat bonus auras; `1.0` = none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bonus {
    pub damage: f32,
    pub taken_hull: f32,
    pub taken_shield: f32,
    pub range: f32,
    pub speed: f32,
}

impl Default for Bonus {
    fn default() -> Self {
        Bonus { damage: 1.0, taken_hull: 1.0, taken_shield: 1.0, range: 1.0, speed: 1.0 }
    }
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
}

/// Group bookkeeping for results.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    pub side: u8,
    pub tag: u32,
    pub ship: ShipId,
    pub members: Vec<u32>,
}

pub(crate) const NO_HP: u16 = u16::MAX;

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
    /// Per side: battle time at which the retreat jump happens (`None` = no retreat ordered).
    pub(crate) retreat_jump_at: Vec<Option<f32>>,
    pub(crate) grid_cells: Vec<Vec<u32>>,
    /// Areas revealed to a side for a while: `(side, centre, radius, until)`.
    pub(crate) reveals: Vec<(u8, Vec3, f32, f32)>,
    /// Damaging areas (`BUZZ_DROIDS`): owner side, centre, radius, enemy damage per second, end time.
    /// Super laser shots on their way: shooter, target, firing time.
    pub(crate) lasers: Vec<(u32, u32, f32)>,
    /// Shield drains (`CORRUPT_SYSTEMS`): target, shield points per second, end time.
    pub(crate) drains: Vec<(u32, f32, f32)>,
    pub(crate) zones: Vec<(u8, Vec3, f32, f32, f32)>,
    /// Distance field of rocks and stations for path planning (`None` without navigation or obstacles).
    pub(crate) nav_grid: Option<crate::nav::NavGrid>,
    /// Per side: `(search point index, time it was set)` of the AI search for hidden enemies.
    pub(crate) search: Vec<(u32, f32)>,
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
        BattleResult { winner: self.winner, duration: self.time, groups }
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
