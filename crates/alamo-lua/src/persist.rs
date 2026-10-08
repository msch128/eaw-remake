//! Saving the data of a running script: serialisable Lua values and the snapshot of a [`crate::ScriptRuntime`].
//!
//! Live coroutines cannot be serialised, so a save keeps what scripts *know* instead of where they are: the engine's `GlobalValue` store, the story
//! flags, the story plots (event states, counters, timers), the interned type names and, for every long-lived script instance (story and scoring scripts,
//! not AI plans), the global variables that changed since the script was set up. A global is saved if it is made of numbers, strings, booleans,
//! tables of those and player / object / type handles (stored as `(kind, id)`, which stay valid because fleet and type ids are saved too). Functions,
//! coroutines, task forces and other handles are dropped, entries that hold them are left out of their table.
//!
//! On load the scripts are started afresh (as at the start of a game) and the saved globals are put back; their main threads restart from the top,
//! which is what the engine does on a saved game too. Scripts in the middle of a state-machine state (`Define_State` callbacks that loop with `Sleep`)
//! resume in the idle state; the story engine has already recorded which events fired, so the events are not triggered again.

use std::collections::HashMap;

use mlua::{Lua, Value};

use crate::api::GVal;
use crate::handles::{Handle, HandleData, Kind};
use crate::story::StorySnapshot;

/// A Lua value that can be saved.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LuaVal {
    /// Boolean.
    Bool(bool),
    /// Number (Lua 5.1 has one number type).
    Num(f64),
    /// String.
    Str(String),
    /// Table as key/value pairs.
    Table(Vec<(LuaVal, LuaVal)>),
    /// Player, object or type handle: `(kind, id)` with kind 0 = player, 1 = object, 2 = type.
    Handle(u8, u64),
}

const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 100_000;

/// Converts a Lua value; `None` for anything that cannot be saved.
pub fn to_val(v: &Value, depth: usize, seen: &mut Vec<usize>, nodes: &mut usize) -> Option<LuaVal> {
    *nodes += 1;
    if *nodes > MAX_NODES || depth > MAX_DEPTH {
        return None;
    }
    match v {
        Value::Boolean(b) => Some(LuaVal::Bool(*b)),
        Value::Integer(i) => Some(LuaVal::Num(*i as f64)),
        Value::Number(n) => n.is_finite().then_some(LuaVal::Num(*n)),
        Value::String(s) => Some(LuaVal::Str(s.to_string_lossy().to_string())),
        Value::UserData(ud) => {
            let h = ud.borrow::<HandleData>().ok()?;
            let kind = match h.handle.kind {
                Kind::Player => 0,
                Kind::Object => 1,
                Kind::Type => 2,
                _ => return None,
            };
            Some(LuaVal::Handle(kind, h.handle.id))
        }
        Value::Table(t) => {
            let ptr = t.to_pointer() as usize;
            if seen.contains(&ptr) {
                return None;
            }
            seen.push(ptr);
            let mut out = Vec::new();
            for (k, val) in t.pairs::<Value, Value>().flatten() {
                if let (Some(k), Some(val)) = (to_val(&k, depth + 1, seen, nodes), to_val(&val, depth + 1, seen, nodes)) {
                    out.push((k, val));
                }
            }
            seen.pop();
            // Hash order is not stable: sort so that equal tables compare equal.
            out.sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
            Some(LuaVal::Table(out))
        }
        _ => None,
    }
}

/// Builds the Lua value of a saved one.
pub fn from_val(lua: &Lua, shared: &std::rc::Rc<std::cell::RefCell<crate::api::Shared>>, script: usize, v: &LuaVal) -> mlua::Result<Value> {
    Ok(match v {
        LuaVal::Bool(b) => Value::Boolean(*b),
        LuaVal::Num(n) => Value::Number(*n),
        LuaVal::Str(s) => Value::String(lua.create_string(s)?),
        LuaVal::Handle(kind, id) => {
            let kind = match kind {
                0 => Kind::Player,
                1 => Kind::Object,
                _ => Kind::Type,
            };
            crate::handles::make(lua, HandleData { handle: Handle { kind, id: *id }, shared: shared.clone(), script })?
        }
        LuaVal::Table(entries) => {
            let t = lua.create_table()?;
            for (k, val) in entries {
                t.set(from_val(lua, shared, script, k)?, from_val(lua, shared, script, val)?)?;
            }
            Value::Table(t)
        }
    })
}

/// Globals of one script instance that differ from its freshly started state.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptSnapshot {
    /// Instance index (scripts are started in the same order when a save is loaded).
    pub index: usize,
    /// Script name, checked on load.
    pub name: String,
    /// Changed or new global variables.
    pub globals: Vec<(String, LuaVal)>,
}

/// Everything a script runtime saves.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct RuntimeSnapshot {
    /// Game time in seconds.
    pub time: f64,
    /// Script random generator state.
    pub rng: u64,
    /// `GlobalValue` store.
    pub global_values: Vec<(String, GVal)>,
    /// Story flags.
    pub story_flags: Vec<String>,
    /// Interned object type names (handles refer to them by index).
    pub types: Vec<String>,
    /// Story plots.
    pub story: StorySnapshot,
    /// Long-lived scripts.
    pub scripts: Vec<ScriptSnapshot>,
}

/// Serialisable globals of a Lua state (not the standard library or engine functions).
pub fn globals_of(lua: &Lua) -> HashMap<String, LuaVal> {
    let mut out = HashMap::new();
    for (k, v) in lua.globals().pairs::<String, Value>().flatten() {
        if k == "_G" || k == "_VERSION" {
            continue;
        }
        let (mut seen, mut nodes) = (Vec::new(), 0);
        if let Some(val) = to_val(&v, 0, &mut seen, &mut nodes) {
            out.insert(k, val);
        }
    }
    out
}
