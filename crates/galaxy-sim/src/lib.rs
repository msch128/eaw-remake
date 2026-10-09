//! Headless, deterministic galactic conquest simulation for Empire at War: Forces of Corruption
//! (Remake 4.0 data), space only. See `RULES.md` for the rules and their sources.
//!
//! ```no_run
//! # use galaxy_sim::{Sim, SimConfig};
//! # fn demo(data: &alamo_data::GameData) {
//! let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
//! let mut sim = Sim::new(data, campaign, SimConfig::default()).unwrap();
//! sim.set_ai_factions(0..sim.world().factions.len() as u32);
//! let events = sim.step(60.0);
//! # let _ = events;
//! # }
//! ```

pub mod ai;
pub mod cheats;
mod ground_up;
mod catalog;
pub use catalog::tech_level_cost;
pub mod combat;
mod api;
pub mod production;
mod sight;
pub mod rng;
pub mod sim;
pub mod tactical;
pub mod state;
pub mod world;

pub use sim::{AutoPreview, Sim, SimSnapshot};
pub use state::{
    Command, CommandError, CommandOk, Event, FactionState, FactionStats, ShipId, Fleet, FleetLocation, Galaxy, Lane, PendingBattle, PlanetState,
    ProductionItem, ProductionKind, Difficulty,
};
pub use world::{IncomeGamble, 
    BattleResolver, CombatStats, ObjectKind, FactionId, FactionInfo, FleetId, PlanetId, PlanetInfo, RouteInfo, SimConfig, SimError, StarbaseTypeId,
    StarbaseType, StructureType, StructureTypeId, UnitRole, UnitType, UnitTypeId, VictoryRules, Weapon, World,
};
pub use production::{DestroyRefusal, PlanetCapacity, SlotUse};
pub use sight::Sight;
pub use tactical::{GroupKind, TacticalError, TacticalGroup, TacticalRequest, TacticalResult, TacticalSide, run_headless};
