//! The Empire at War galactic AI on top of galaxy-sim.
//!
//! Runs the Remake's own AI data against an abstract simulation ([`AiSim`], implemented for galaxy-sim in
//! `galaxy-script`):
//!
//! * [`data`] loads `Data/Xml/AI` (player types, goal function sets, goals, templates);
//! * [`equation`] and [`perception`] parse and evaluate the perceptual equations (variable definitions are
//!   reconstructed from usage, unknown variables read as 0 and are counted);
//! * [`engine`] scores goals and targets, keeps per-category budgets and starts the plan scripts (Lua, via `alamo-lua`);
//! * [`taskforce`] implements the space-relevant task force calls plans make (`Produce_Force`, `Move_To`, `Release_Forces`, ...);
//! * [`strategy`] is an added layer (influence maps, utility scoring, personalities, difficulty tiers) on top of the
//!   Remake's desires.
//!
//! Estimates (budget bookkeeping, strategy weights, tier multipliers for Brutal and Fair) are named in the code.
//! See docs/ARCHITECTURE.md for the data flow.

pub mod data;
pub mod engine;
pub mod equation;
pub mod perception;
pub mod requirements;
pub mod sim;
pub mod strategy;
pub mod taskforce;

pub use data::{AiData, AiPlayerDef, AiTemplate, GoalDef, campaign_ai_players};
pub use strategy::{AiTier, Opening, Personality, Phase, Strategy, StrategyConfig};
pub use engine::{AiEngine, AiMemory, AiStats, BrainMemory, PlanRequest, PlanTiming, discover_plans};
pub use strategy::StrategyMemory;
pub use taskforce::TaskForceMemory;
pub use equation::{EquationContext, EquationSet, EvalStats, Expr, ParamValue, Params};
pub use sim::{AiSim, FleetId, FleetView, PlanetId, PlayerId, QueueView, Refused, StructureInfo, UnitInfo};