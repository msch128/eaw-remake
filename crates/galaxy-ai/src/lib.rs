//! The Empire at War galactic AI on top of galaxy-sim. See `AI.md`.

pub mod data;
pub mod engine;
pub mod equation;
pub mod perception;
pub mod requirements;
pub mod sim;
pub mod taskforce;

pub use data::{AiData, AiPlayerDef, AiTemplate, GoalDef, campaign_ai_players};
pub use engine::{AiEngine, AiStats, PlanRequest, discover_plans};
pub use equation::{EquationContext, EquationSet, EvalStats, Expr, ParamValue, Params};
pub use sim::{AiSim, FleetId, FleetView, PlanetId, PlayerId, QueueView, Refused, StructureInfo, UnitInfo};