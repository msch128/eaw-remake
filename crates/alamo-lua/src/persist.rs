//! Saving the data of a running script: serialisable Lua values and the snapshot of a [`crate::ScriptRuntime`].
//!
//! A save keeps the engine's `GlobalValue` store, the story flags, the story plots (event states, counters, timers), the interned type names and,
//! for every long-lived script instance (story and scoring scripts, not AI plans), the global variables that changed since the script was set up
//! and where each of its threads stopped. A value is saved if it is made of numbers, strings, booleans, tables of those and player / object /
//! type / thread handles (stored as `(kind, id)`, which stay valid because fleet, type and thread ids are saved too). Functions, task forces and
//! other handles are dropped, entries that hold them are left out of their table.
//!
//! The engine saves the coroutines themselves (the library keeps no locals across a yield for its persistence code, `Sleep` keeps its start
//! time in `ThreadValue`), so a loaded script goes on at the instruction it stopped at. A Lua 5.1 coroutine cannot be written to a file, so a save
//! keeps what tells where a thread is: the function it runs and its arguments, its `ThreadValue` store, and the Lua frames it waits in (source,
//! line, local variables). On load each thread is started again with the game shut off ([`crate::api::Shared::resuming`]: spawns, credits,
//! rewards and messages do nothing; story, `GlobalValue` store, story flags, random generator and globals are put back afterwards) and run until
//! it waits in the same frames; there its locals and its `ThreadValue` store are set to the saved ones. Each wait it passes on the way counts as
//! over (`GetCurrentTime` jumps ahead). A state machine of `PGStateMachine.lua` saved in the update loop of a state thus runs the `OnEnter` of
//! that state with the game shut off and goes on in the loop; a `Sleep` goes on with the time it had left, a loop at its saved index.
//!
//! A save of a build that wrote no threads ([`ScriptSnapshot::threads`] is `None`) brings `main` to its first wait the same way, and the threads
//! that part starts to theirs: the entry code of the current state does not reach the game again.

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
    /// Player, object, type or thread handle: `(kind, id)` with kind 0 = player, 1 = object, 2 = type, 3 = thread.
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
                Kind::Thread => 3,
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

/// [`to_val`] of one value.
pub fn value_of(v: &Value) -> Option<LuaVal> {
    to_val(v, 0, &mut Vec::new(), &mut 0)
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
                3 => Kind::Thread,
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

/// One long-lived script instance: its globals that differ from its freshly started state and where its threads stopped.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptSnapshot {
    /// Instance index (scripts are started in the same order when a save is loaded).
    pub index: usize,
    /// Script name, checked on load.
    pub name: String,
    /// Changed or new global variables.
    pub globals: Vec<(String, LuaVal)>,
    /// The threads in service order (`main` first). `None` in saves of builds that kept only the globals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<Vec<ThreadSnapshot>>,
    /// Game time of the next service and the number of services in a row without an engine call. `None` in older saves: due at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<(f64, u32)>,
    /// The script had ended (its `main` returned, `ScriptExit`, an error): it does not run in the loaded game either.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ended: bool,
}

impl ScriptSnapshot {
    /// Every saved value of the script (globals, thread arguments, thread values, locals), for rewriting the ids in handles.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut LuaVal> {
        let threads = self.threads.iter_mut().flatten().flat_map(|t| {
            let args = t.args.iter_mut().flatten();
            let locals = t.stack.iter_mut().flat_map(|f| f.locals.iter_mut().filter_map(|l| l.2.as_mut()));
            args.chain(t.values.iter_mut()).chain(locals)
        });
        self.globals.iter_mut().map(|(_, v)| v).chain(threads)
    }
}

/// Where a thread of a script stopped.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ThreadSnapshot {
    /// Thread id (0 = `main`; scripts keep the ids of the others to kill them).
    pub id: i64,
    /// Name of the global function the thread runs. `None` for a function no global names: such a thread cannot be started again.
    pub function: Option<String>,
    /// Arguments of its first call (`None`: nil or a value that cannot be saved).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Option<LuaVal>>,
    /// Its `ThreadValue` store (where `Sleep` keeps its start time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<LuaVal>,
    /// The Lua frames it waits in, innermost first. Empty: it has not run yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stack: Vec<FrameSnapshot>,
}

/// A Lua function call a thread waits in.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameSnapshot {
    /// Chunk name of the function (the script path).
    pub source: String,
    /// Line the function is defined at.
    pub defined: i32,
    /// Line it is at.
    pub line: i32,
    /// Its local variables (loop counters included): slot, name and value (`None` = nil). Locals that cannot be saved are left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locals: Vec<(u32, String, Option<LuaVal>)>,
}

impl FrameSnapshot {
    /// The same call at the same line.
    pub fn same_place(&self, other: &FrameSnapshot) -> bool {
        self.same_function(other) && self.line == other.line
    }

    /// A call of the same function.
    pub fn same_function(&self, other: &FrameSnapshot) -> bool {
        self.source == other.source && self.defined == other.defined
    }
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
    /// Id of the last thread created (`None` in older saves).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_thread: Option<i64>,
}

/// A global of the Lua standard library. Not script data: `package.loaded._G` mirrors every global (saves of earlier builds hold it), the
/// others are tables of functions. Not saved, and not put back from a save.
pub fn is_library(name: &str) -> bool {
    matches!(name, "_G" | "_VERSION" | "package" | "string" | "table" | "math" | "coroutine" | "io" | "os" | "debug")
}

/// Serialisable globals of a Lua state (not the standard library or engine functions).
pub fn globals_of(lua: &Lua) -> HashMap<String, LuaVal> {
    let mut out = HashMap::new();
    for (k, v) in lua.globals().pairs::<String, Value>().flatten() {
        if is_library(&k) {
            continue;
        }
        let (mut seen, mut nodes) = (Vec::new(), 0);
        if let Some(val) = to_val(&v, 0, &mut seen, &mut nodes) {
            out.insert(k, val);
        }
    }
    out
}
