//! Headless deterministic tactical space battle simulation for Empire at War: Forces of
//! Corruption (Remake 4.0 data). See `RULES.md` for the implemented rules and their data sources.
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
mod math;
mod placement;
pub mod nav;
mod rng;
mod sim;

pub use battle::{
    AbilityState, Battle, BattleConfig, SideMult, StartHealth, BattleResult, BattleSetup, Command, CommandError, Event, GroupResult, HitKind, HpState,
    HUNT_GRID, NONE, NO_CELL, NO_HP, NO_TAG, REINFORCE_TAG, Order, Projectile, Ship, SideSetup, SquadSlot, Stance, UnitSetup,
};
pub use defs::{BonusDef, CombatMod, ModKind, WeakenDef, LureDef, BombDef, EnergyDef, ClusterDef, HealDef, CorruptDef, RedirectDef, SuperLaserDef, ZoneDef, 
    AbilityDef, Catalog, DamageMatrix, DirectWeaponDef, HpDef, HpKind, PrioritySet, ProjDef, ProjId, ShipDef, ShipId, SpecialWeaponDef, TargetClass,
    TractorDef,
};
pub use arena::{Arena, Marker, Obstacle};
pub use math::Vec3;
pub use placement::{no_spawn_zones, placement_check, PlacementUnit};
pub use rng::Rng;
pub use sim::{setup, simulate};
pub use cheats::{BattleCheat, Cheats};
