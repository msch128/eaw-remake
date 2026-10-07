//! The script runtime: script instances (one Lua state each, like the engine), `require` through the game
//! file system, cooperative threads and the service loop.
//!
//! # Threading model (as the Alamo scripts expect it)
//!
//! * A script file is loaded into its own Lua state, `Base_Definitions()` runs (it calls the script's
//!   `Definitions()`), then `main()` runs as thread 0.
//! * `PumpEvents()` (a Lua function in `PGBase.lua`) yields `true` to the engine; `ScriptExit()` yields
//!   `false`. `Sleep` and `BlockOnCommand` are loops around `PumpEvents`, so *every thread is resumed once
//!   per service* and the library code decides when to continue.
//! * A script is serviced every `ServiceRate` seconds of game time (the Lua global, default 0.2 s). All
//!   threads of the script are resumed in creation order.
//! * `Create_Thread(fn_or_name, ...)` makes additional coroutines in the same state. When `main` returns or
//!   `ScriptExit` is called the script ends. A Lua error ends the script and is recorded in
//!   [`ScriptRuntime::errors`].
//! * Story plots (see [`crate::story`]) call `Story_Event_Trigger(name)` in the plot's script.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use alamo_data::xml;
use alamo_fs::{FileSource, normalize};
use mlua::{Function, Lua, MultiValue, Table, Thread, ThreadStatus, Value};

use crate::api::{self, NewThread, Shared};
use crate::handles::{Handle, HandleData, Kind};
use crate::host::{PlayerId, ScriptHost};
use crate::story::{Fired, Plot, StoryEngine};

/// Value for a global handed to a new script instance.
#[derive(Debug, Clone)]
pub enum ScriptValue {
    /// A string.
    Str(String),
    /// A number.
    Num(f64),
    /// A boolean.
    Bool(bool),
    /// A player handle.
    Player(PlayerId),
    /// A game object handle.
    Object(u64),
}

/// A script that ended with an error.
#[derive(Debug, Clone)]
pub struct ScriptError {
    /// Script name.
    pub script: String,
    /// Lua error text.
    pub message: String,
}

struct ThreadSlot {
    id: i64,
    thread: Thread,
    first_args: Option<MultiValue>,
}

struct Instance {
    lua: Lua,
    threads: Vec<ThreadSlot>,
    next_service: f64,
    alive: bool,
}

/// Runs scripts against a [`ScriptHost`].
pub struct ScriptRuntime {
    shared: Rc<RefCell<Shared>>,
    scripts: Vec<Instance>,
    errors: Vec<ScriptError>,
}

fn is_script_path(p: &str) -> bool {
    p.ends_with(".LUA")
}

impl ScriptRuntime {
    /// Creates a runtime; indexes `Scripts/**/*.lua` of `fs`.
    pub fn new(fs: Arc<dyn FileSource>, host: Box<dyn ScriptHost>) -> Self {
        let mut index: HashMap<String, String> = HashMap::new();
        for p in fs.list("Scripts").into_iter().filter(|p| is_script_path(p)) {
            let base = p
                .rsplit('/')
                .next()
                .unwrap_or(&p)
                .trim_end_matches(".LUA")
                .to_ascii_lowercase();
            let better = |new: &str, old: &str| {
                let lib = |s: &str| s.contains("/LIBRARY/");
                (lib(new) && !lib(old)) || (lib(new) == lib(old) && new.len() < old.len())
            };
            match index.get(&base) {
                Some(old) if !better(&p, old) => {}
                _ => {
                    index.insert(base, p);
                }
            }
        }
        let shared = Shared {
            host,
            fs,
            index,
            time: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15,
            global_values: HashMap::new(),
            current_thread: -1,
            logged: HashSet::new(),
            log: Vec::new(),
            api_hits: BTreeMap::new(),
            unknown: BTreeSet::new(),
            new_threads: Vec::new(),
            killed: Vec::new(),
            exit: HashSet::new(),
            script_names: Vec::new(),
            types: Vec::new(),
            pending_fired: Vec::new(),
            next_thread: 0,
            story: StoryEngine::default(),
            story_flags: HashSet::new(),
        };
        Self {
            shared: Rc::new(RefCell::new(shared)),
            scripts: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Game time in seconds.
    pub fn time(&self) -> f64 {
        self.shared.borrow().time
    }

    /// Runs `f` with the host (for tests and for the game to push state).
    pub fn with_host<R>(&self, f: impl FnOnce(&mut dyn ScriptHost) -> R) -> R {
        f(&mut *self.shared.borrow_mut().host)
    }

    /// Errors of scripts that died.
    pub fn errors(&self) -> &[ScriptError] {
        &self.errors
    }

    /// The script log (`DebugMessage`, errors, unimplemented-API notes), capped at 5000 lines.
    pub fn log(&self) -> Vec<String> {
        self.shared.borrow().log.clone()
    }

    /// Call counts for every engine API function that scripts used (implemented, ignored or stubbed).
    pub fn api_hits(&self) -> BTreeMap<String, usize> {
        self.shared.borrow().api_hits.clone()
    }

    /// API names that reached an unimplemented stub.
    pub fn unimplemented(&self) -> BTreeSet<String> {
        self.shared.borrow().unknown.clone()
    }

    /// Number of running script instances.
    pub fn alive_scripts(&self) -> usize {
        self.scripts.iter().filter(|s| s.alive).count()
    }

    /// Story events that have fired so far.
    pub fn story_fired(&self) -> usize {
        self.shared.borrow().story.fired_count()
    }

    /// Resolves a script name (`PGStateMachine`, `Story/Foo`, `foo.lua`) to its normalized path.
    pub fn resolve_script(&self, name: &str) -> Option<String> {
        resolve(&self.shared.borrow(), name)
    }

    /// Starts a script: loads the file, runs `Base_Definitions()` and creates the `main` thread.
    /// `globals` are set before the file is loaded (`Object`, `PlayerObject`, ...).
    pub fn spawn_script(
        &mut self,
        name: &str,
        globals: &[(&str, ScriptValue)],
    ) -> Result<usize, String> {
        let path = self
            .resolve_script(name)
            .ok_or_else(|| format!("script not found: {name}"))?;
        let id = self.scripts.len();
        let lua = Lua::new();
        self.shared.borrow_mut().script_names.push(name.to_string());
        let sh = self.shared.clone();
        let setup = || -> mlua::Result<Vec<ThreadSlot>> {
            api::install(&lua, &sh, id)?;
            lua.load(api::PRELUDE).set_name("prelude").exec()?;
            install_require(&lua, &sh, id)?;
            let g = lua.globals();
            let script_handle = crate::handles::make(
                &lua,
                HandleData {
                    handle: Handle {
                        kind: Kind::Script,
                        id: id as u64,
                    },
                    shared: sh.clone(),
                    script: id,
                },
            )?;
            g.set("Script", script_handle)?;
            for (k, v) in globals {
                let val = match v {
                    ScriptValue::Str(s) => Value::String(lua.create_string(s)?),
                    ScriptValue::Num(n) => Value::Number(*n),
                    ScriptValue::Bool(b) => Value::Boolean(*b),
                    ScriptValue::Player(p) => crate::handles::make(
                        &lua,
                        HandleData {
                            handle: Handle {
                                kind: Kind::Player,
                                id: u64::from(*p),
                            },
                            shared: sh.clone(),
                            script: id,
                        },
                    )?,
                    ScriptValue::Object(o) => crate::handles::make(
                        &lua,
                        HandleData {
                            handle: Handle {
                                kind: Kind::Object,
                                id: *o,
                            },
                            shared: sh.clone(),
                            script: id,
                        },
                    )?,
                };
                g.set(*k, val)?;
            }
            sh.borrow_mut().current_thread = -1;
            run_file(&lua, &sh, &path)?;
            if let Value::Function(f) = g.get::<Value>("Base_Definitions")? {
                f.call::<()>(())?;
            }
            let mut threads = Vec::new();
            if let Value::Function(main) = g.get::<Value>("main")? {
                threads.push(ThreadSlot {
                    id: 0,
                    thread: lua.create_thread(main)?,
                    first_args: None,
                });
            }
            Ok(threads)
        };
        match setup() {
            Ok(threads) => {
                let alive = !threads.is_empty();
                self.scripts.push(Instance {
                    lua,
                    threads,
                    next_service: 0.0,
                    alive,
                });
                Ok(id)
            }
            Err(e) => {
                let msg = e.to_string();
                self.errors.push(ScriptError {
                    script: name.to_string(),
                    message: msg.clone(),
                });
                self.scripts.push(Instance {
                    lua,
                    threads: Vec::new(),
                    next_service: 0.0,
                    alive: false,
                });
                Err(msg)
            }
        }
    }

    /// Reads a `Story_Mode_Plots` style file (`<Lua_Script>` / `<Active_Plot>` pairs, as referenced by
    /// campaigns' `*_Story_Name`) and starts the scripts and plots in it.
    pub fn load_story_plots(
        &mut self,
        plots_xml: &str,
        faction: Option<&str>,
    ) -> Result<usize, String> {
        let fs = self.shared.borrow().fs.clone();
        let bytes = fs
            .read(&format!("Xml/{plots_xml}"))
            .map_err(|e| format!("{plots_xml}: {e}"))?;
        let mut warnings = Vec::new();
        let root =
            xml::parse(&bytes, plots_xml, &mut warnings).ok_or("unparsable story plots file")?;
        let mut script: Option<usize> = None;
        let mut started = 0;
        for c in &root.children {
            match c.name.as_str() {
                "lua_script" => {
                    let script_name = c.text_trimmed().to_string();
                    script = None;
                    if !script_name.is_empty() {
                        match self.spawn_script(&script_name, &[]) {
                            Ok(s) => script = Some(s),
                            Err(e) => self
                                .shared
                                .borrow_mut()
                                .log
                                .push(format!("story script {script_name}: {e}")),
                        }
                    }
                }
                "active_plot" => {
                    let file = c.text_trimmed().to_string();
                    if file.is_empty() {
                        continue;
                    }
                    let Ok(b) = fs.read(&format!("Xml/{file}")) else {
                        self.shared
                            .borrow_mut()
                            .log
                            .push(format!("plot not found: {file}"));
                        continue;
                    };
                    if let Some(plot) = Plot::parse(&file, &b, &mut warnings) {
                        let now = self.time();
                        self.shared.borrow_mut().story.activate(
                            plot,
                            script.unwrap_or(usize::MAX),
                            faction.map(str::to_string),
                            now,
                        );
                        started += 1;
                    }
                }
                _ => {}
            }
        }
        Ok(started)
    }

    /// Reports a story occurrence from the game (`STORY_CONQUER`, `STORY_CONSTRUCT`, ...).
    pub fn story_notify(&mut self, kind: &str, param: &str) {
        let now = self.time();
        let fired = self.shared.borrow_mut().story.notify(&[kind], param, now);
        self.dispatch(fired);
    }

    /// Advances game time by `dt` seconds: story timers, script services.
    pub fn update(&mut self, dt: f64) {
        let now = {
            let mut s = self.shared.borrow_mut();
            s.time += dt;
            s.time
        };
        // Scripts run first so that their state machines are initialised before story events arrive.
        for sid in 0..self.scripts.len() {
            if self.scripts[sid].alive && now >= self.scripts[sid].next_service {
                self.service(sid);
                let rate = self.service_rate(sid);
                self.scripts[sid].next_service = now + rate;
            }
        }
        let fired = self.shared.borrow_mut().story.tick(now);
        self.dispatch(fired);
        // Events fired by `Story_Event(...)` calls made by scripts (rewards already applied).
        for _ in 0..8 {
            let fired: Vec<Fired> = std::mem::take(&mut self.shared.borrow_mut().pending_fired);
            if fired.is_empty() {
                break;
            }
            self.call_triggers(fired);
        }
    }

    fn service_rate(&self, sid: usize) -> f64 {
        match self.scripts[sid].lua.globals().get::<Value>("ServiceRate") {
            Ok(Value::Number(n)) if n > 0.0 => n,
            Ok(Value::Integer(n)) if n > 0 => n as f64,
            _ => 0.2,
        }
    }

    fn dispatch(&mut self, fired: Vec<Fired>) {
        for f in &fired {
            self.shared.borrow_mut().apply_rewards(f);
        }
        self.call_triggers(fired);
    }

    /// Calls `Story_Event_Trigger(event)` in the plot's script for each fired event.
    fn call_triggers(&mut self, fired: Vec<Fired>) {
        for f in fired {
            if f.script < self.scripts.len() && self.scripts[f.script].alive {
                let name = self.shared.borrow().script_names[f.script].clone();
                self.shared.borrow_mut().current_thread = -1;
                let lua = &self.scripts[f.script].lua;
                if let Ok(Value::Function(func)) = lua.globals().get::<Value>("Story_Event_Trigger")
                    && let Err(e) = func.call::<()>(f.event.as_str())
                {
                    self.errors.push(ScriptError {
                        script: name,
                        message: format!("Story_Event_Trigger({}): {e}", f.event),
                    });
                    self.scripts[f.script].alive = false;
                }
            }
        }
    }

    fn service(&mut self, sid: usize) {
        let name = self.shared.borrow().script_names[sid].clone();
        let mut i = 0;
        while i < self.scripts[sid].threads.len() {
            let (tid, thread, args) = {
                let slot = &mut self.scripts[sid].threads[i];
                (slot.id, slot.thread.clone(), slot.first_args.take())
            };
            self.shared.borrow_mut().current_thread = tid;
            let result: mlua::Result<MultiValue> = thread.resume(args.unwrap_or_default());
            self.shared.borrow_mut().current_thread = -1;
            let finished = match result {
                // A finished coroutine, or one that yielded `false` (`ScriptExit`), is done.
                Ok(vals) => {
                    thread.status() != ThreadStatus::Resumable
                        || matches!(vals.iter().next(), Some(Value::Boolean(false)))
                }
                Err(e) => {
                    self.errors.push(ScriptError {
                        script: name.clone(),
                        message: e.to_string(),
                    });
                    self.scripts[sid].alive = false;
                    break;
                }
            };
            // Adopt threads created meanwhile.
            let created: Vec<NewThread> = std::mem::take(&mut self.shared.borrow_mut().new_threads);
            for nt in created {
                if nt.script == sid {
                    self.scripts[sid].threads.push(ThreadSlot {
                        id: nt.id,
                        thread: nt.thread,
                        first_args: Some(nt.args),
                    });
                }
            }
            let killed: Vec<(usize, i64)> = std::mem::take(&mut self.shared.borrow_mut().killed);
            let exit = self.shared.borrow_mut().exit.remove(&sid);
            if exit || (tid == 0 && finished) {
                // The main thread ending (or ScriptExit) ends the script.
                self.scripts[sid].alive = false;
                break;
            }
            let mut remove_self = finished;
            for (s, t) in killed {
                if s == sid {
                    if t == tid {
                        remove_self = true;
                    }
                    self.scripts[sid]
                        .threads
                        .retain(|th| th.id != t || th.id == tid);
                }
            }
            if remove_self {
                self.scripts[sid].threads.remove(i);
            } else {
                i += 1;
            }
        }
        if self.scripts[sid].alive && self.scripts[sid].threads.is_empty() {
            self.scripts[sid].alive = false;
        }
        // Drop threads created by a script that died.
        if !self.scripts[sid].alive {
            self.scripts[sid].threads.clear();
        }
    }
}

fn resolve(s: &Shared, name: &str) -> Option<String> {
    let n = normalize(name);
    let n = n.trim_start_matches("SCRIPTS/");
    let base = n
        .rsplit('/')
        .next()
        .unwrap_or(n)
        .trim_end_matches(".LUA")
        .to_ascii_lowercase();
    s.index.get(&base).cloned()
}

fn run_file(lua: &Lua, sh: &Rc<RefCell<Shared>>, path: &str) -> mlua::Result<Value> {
    let bytes = sh.borrow().fs.read(path).map_err(mlua::Error::external)?;
    let text = String::from_utf8_lossy(&bytes);
    lua.load(text.as_ref()).set_name(path).eval::<Value>()
}

/// `require(name)`: loads a script file once per state, resolving the name through the script index.
fn install_require(lua: &Lua, sh: &Rc<RefCell<Shared>>, _script: usize) -> mlua::Result<()> {
    let loaded: Table = lua.create_table()?;
    lua.set_named_registry_value("loaded_scripts", loaded)?;
    let shc = sh.clone();
    let f: Function = lua.create_function(move |lua, name: String| -> mlua::Result<Value> {
        let Some(path) = resolve(&shc.borrow(), &name) else {
            return Err(mlua::Error::runtime(format!(
                "require: script '{name}' not found"
            )));
        };
        let loaded: Table = lua.named_registry_value("loaded_scripts")?;
        if loaded.contains_key(path.as_str())? {
            return loaded.get::<Value>(path.as_str());
        }
        // Mark as loaded first so circular requires terminate.
        loaded.set(path.as_str(), true)?;
        let ret = run_file(lua, &shc, &path)?;
        let ret = if ret == Value::Nil {
            Value::Boolean(true)
        } else {
            ret
        };
        loaded.set(path.as_str(), ret.clone())?;
        Ok(ret)
    })?;
    lua.globals().set("require", f.clone())?;
    lua.globals().set("Require", f)?;
    Ok(())
}
