//! Lua runtime for Empire at War scripts: loader, cooperative threads, story plots and engine API
//! bindings through the [`ScriptHost`] trait. The `lua_survey` example inventories what the shipped scripts call.
//!
//! The engine uses Lua 5.0; this crate embeds a vendored Lua 5.1 (`mlua`) with the compatibility the scripts need.
//! Every script instance gets its own Lua state (`runtime`), with a case-insensitive `require` through the game file
//! system and cooperative threads (`Create_Thread`, `Sleep`, `PumpEvents`). `api` and `ai_api` bind the engine functions
//! and forward them to a [`ScriptHost`]; `story` runs the plot events of `Xml/Story_*.xml`; `persist` snapshots a running
//! runtime for saves. Unknown engine calls are logged once and return nil; presentation calls are ignored silently.
//! No script text is stored in this repository.

pub mod ai_api;
pub mod api;
pub mod handles;
pub mod host;
pub mod persist;
pub mod runtime;
pub mod scan;
pub mod story;

pub use host::{HostValue, PlanEvent, TaskForceSpec, 
    GameMode, MockHost, ObjectFilter, ObjectId, ObjectInfo, PlayerId, PlayerInfo, ScriptHost,
    SpawnAt, StoryReward,
};
pub use persist::{LuaVal, RuntimeSnapshot, ScriptSnapshot};
pub use runtime::{ScriptError, ScriptRuntime, ScriptValue};
