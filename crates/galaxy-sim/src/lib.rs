//! Headless, deterministic galactic conquest simulation for Empire at War: Forces of Corruption
//! (Remake 4.0 data), space only. See docs/RULES.md for the rules and their sources.
//!
//! `World` is the static, shared (`Arc`) part built once from the game data (planets, hyperspace lanes, factions,
//! unit, structure and star base catalogs); `Galaxy` is the mutable state; `Sim` drives time, commands and events.
//! All randomness comes from a seeded generator and there is no global state, so equal inputs give equal games.
//! Player and AI actions are serialisable `Command`s. Battles are resolved statistically (`combat.rs`) or handed to
//! `battle-sim` as plain data (`tactical.rs`). `api.rs` is the direct manipulation API used by `galaxy-script`.
//! Game rules whose numbers the data lacks are named estimates in `SimConfig`.
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
pub mod agents;
mod catalog;
pub mod combat;
mod api;
pub mod production;
pub mod requirements;
mod sight;
pub mod rng;
pub mod sim;
pub mod tactical;
pub mod state;
pub mod world;

pub use sim::{AutoPreview, Sim, SimSnapshot};
pub use state::{
    Command, CommandError, CommandOk, Event, FactionState, FactionStats, ShipId, Fleet, FleetLocation, FLEET_SLOTS, Galaxy, Lane, PendingBattle, PlanetState,
    ProductionItem, ProductionKind, Difficulty, UnitHealth,
};
pub use world::{IncomeGamble, 
    BattleResolver, CombatStats, ObjectKind, FactionId, FactionInfo, FleetId, PlanetId, PlanetInfo, RouteInfo, SimConfig, SimError, StarbaseTypeId,
    StarbaseType, StructureType, StructureTypeId, UnitRole, UnitType, UnitTypeId, VictoryRules, Weapon, World,
};
pub use production::{DestroyRefusal, PlanetCapacity, SlotUse};
pub use requirements::Unmet;
pub use sight::Sight;
pub use agents::{ActiveSiphon, SiphonAbility, SpyAbility, SpyFlags, SpyReport};
pub use tactical::{GroupKind, TacticalError, TacticalGroup, TacticalRequest, TacticalResult, TacticalSide, run_headless, run_headless_observed};
