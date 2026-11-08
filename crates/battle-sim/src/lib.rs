//! Headless deterministic tactical space battle simulation for Empire at War: Forces of
//! Corruption (Remake 4.0 data). See docs/RULES.md for the implemented rules and their data sources.
//!
//! Fixed 30 Hz step, rayon-parallel target scans, per-entity random generators seeded by `(seed, tick, id)`, so a
//! battle gives the same result with or without threads. `defs.rs` builds the immutable `Catalog` (ships, hardpoints,
//! projectiles, damage matrix) from the game data and the `.alo` models; `battle.rs` holds state, commands, events and
//! results; `sim.rs` the step systems; `ai.rs` the side-level tactical AI; `formation.rs` and `nav.rs` group moves and
//! steering; `arena.rs` the playable area read from a `.ted` map; `placement.rs` the reinforcement-zone rule shared with
//! the view. Engine rules the data does not state are named estimates in `BattleConfig` or `defs.rs`.
//!
//! ```no_run
//! # use battle_sim::*;
//! # use std::sync::Arc;
//! # fn demo(data: &alamo_data::GameData, fs: &dyn alamo_fs::FileSource) {
//! let catalog = Arc::new(Catalog::build(data, Some(fs), &["Star_Destroyer", "MC-80_Star_Cruiser"]));
//! let side = |name: &str, ship: &str| SideSetup {
//!     name: name.into(),
//!     units: vec![UnitSetup { ship: catalog.ship_id(ship).unwrap(), count: 3, tag: 0 }],
//!     structures: vec![],
//!     ai: true,
//! };
//! let mut battle = Battle::new(BattleSetup {
//!     catalog: catalog.clone(),
//!     sides: vec![side("Empire", "Star_Destroyer"), side("Rebel", "MC-80_Star_Cruiser")],
//!     config: BattleConfig::default(),
//! });
//! while !battle.is_over() {
//!     for _event in battle.step() { /* render / sound */ }
//! }
//! println!("{:?}", battle.result());
//! # }
//! ```

mod ai;
mod arena;
mod battle;
pub mod cheats;
mod defs;
mod formation;
mod math;
mod placement;
pub mod nav;
mod rng;
mod sim;

pub use battle::{
    AbilityState, Battle, BattleConfig, SideMult, StartHealth, BattleResult, BattleSetup, Command, CommandError, Event, GroupResult, HitKind, HpState,
    HUNT_GRID, NONE, NO_CELL, NO_HP, NO_PROJ, NO_TAG, PHASE_TAG, REINFORCE_TAG, GuardTarget, Order, Projectile, Ship, SideSetup, SquadSlot, Stance, UnitSetup,
};
pub use defs::{Bonus, ChaseRules, BonusDef, CombatMod, ModKind, WeakenDef, LureDef, BombDef, EnergyDef, ClusterDef, HealDef, CorruptDef, RedirectDef, SuperLaserDef, ZoneDef, 
    AbilityDef, Catalog, DamageMatrix, DirectWeaponDef, FormationRules, HpDef, HpKind, PrioritySet, ProjDef, ProjId, ShipDef, ShipId, SpecialWeaponDef, TargetClass,
    TractorDef,
};
pub use arena::{Arena, Marker, Obstacle, PropZones, prop_zones};
pub use math::Vec3;
pub use placement::{no_spawn_zones, placement_check, PlacementUnit};
pub use rng::Rng;
pub use sim::{setup, simulate};
pub use cheats::{BattleCheat, Cheats};
