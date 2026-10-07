//! Lua runtime for Empire at War scripts: loader, cooperative threads, story plots and engine API
//! bindings through the [`ScriptHost`] trait. See `API.md` for the inventory of what scripts call.

pub mod api;
pub mod handles;
pub mod host;
pub mod runtime;
pub mod scan;
pub mod story;

pub use host::{
    GameMode, MockHost, ObjectFilter, ObjectId, ObjectInfo, PlayerId, PlayerInfo, ScriptHost,
    SpawnAt, StoryReward,
};
pub use runtime::{ScriptError, ScriptRuntime, ScriptValue};
