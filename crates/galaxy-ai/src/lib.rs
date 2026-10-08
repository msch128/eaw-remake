//! The Empire at War galactic AI on top of galaxy-sim. See `AI.md`.

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