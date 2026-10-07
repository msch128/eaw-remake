//! Parsers for Alamo engine game data: lenient XML, `.dat` string tables and the galactic
//! conquest data model (campaigns, planets, trade routes, factions).
//!
//! All loading goes through [`alamo_fs::FileSource`] and never fails hard: problems are
//! returned as [`Warning`]s so partially available data (e.g. base-game files that only exist in
//! MEG archives) still yields a usable [`GameData`].

pub mod galaxy;
pub mod model;
pub mod text;
pub mod xml;

pub use galaxy::{GameData, load_galaxy};
pub use model::{
    Campaign, CampaignFaction, Faction, LineSettings, Planet, StartingForce, TradeRoute, TradeRouteLine,
};
pub use text::{TextError, TextTable};
pub use xml::{Element, Warning};
