//! Parsers for Alamo engine game data: lenient XML, `.dat` string tables and the galactic
//! conquest data model (campaigns, planets, trade routes, factions).
//!
//! All loading goes through [`alamo_fs::FileSource`] and never fails hard: problems are
//! returned as [`Warning`]s so partially available data (e.g. base-game files that only exist in
//! MEG archives) still yields a usable [`GameData`].
//!
//! `objects` merges every object XML and applies `Variant_Of_Existing_Type` inheritance through `resolve()`; `space`
//! gives typed views (ships, squadrons, hardpoints, projectiles, abilities, star bases, structures); `combat` holds the
//! damage-to-armor matrix and the Gameconstants map; `difficulty` the per-tier multipliers; `text` the English string
//! table only. Unknown tags stay in `fields` maps instead of being rejected. Ground and story data is parsed generically
//! but unused.

pub mod combat;
pub mod difficulty;
pub mod galaxy;
pub mod model;
pub mod objects;
pub mod space;
pub mod text;
pub mod xml;

pub use combat::CombatTable;
pub use difficulty::{DifficultyAdjustment, DifficultyTable};
pub use galaxy::{CampaignSet, GameData, PlayableCampaign, load_galaxy};
pub use model::{
    Campaign, CampaignFaction, Faction, LineSettings, Planet, StartingForce, TradeRoute,
    TradeRouteLine,
};
pub use objects::{GameObject, GameObjects, Resolved};
pub use space::{
    DirectWeapon, HardpointDef, IncomeModifier, ProjectileDef, ShipCategory, SpaceUnitDef,
    StarBaseDef, StructureDef, SubAbility, TurretDef, UnitAbility,
};
pub use space::{SpawnedUnits, spawned_for_tech};
pub use text::{TextError, TextTable};
pub use xml::{Element, Warning};
