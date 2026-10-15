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
use crate::persist::{LuaVal, RuntimeSnapshot, ScriptSnapshot};
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
    /// `None` once the script ended: its Lua state (hundreds of KB with the API and prelude) is dropped so long games do not leak one VM per plan.
    lua: Option<Lua>,
    threads: Vec<ThreadSlot>,
    next_service: f64,
    /// Consecutive services without any engine call.
    idle: u32,
    alive: bool,
    /// Plan instance id for AI plans.
    plan: Option<u64>,
    /// The end of a plan has been reported through `take_finished_plans`.
    reported: bool,
    /// Serialisable globals right after the script was set up (long-lived scripts only), the reference for what a save has to keep.
    baseline: HashMap<String, LuaVal>,
}

/// Runs scripts against a [`ScriptHost`].
pub struct ScriptRuntime {
    shared: Rc<RefCell<Shared>>,
    scripts: Vec<Instance>,
    errors: Vec<ScriptError>,
    profile: Vec<(std::time::Duration, u32)>,
    idle_backoff: f64,
    finished_plans: Vec<u64>,
}

/// Services without an engine call after which a script is considered idle.
const IDLE_SERVICES: u32 = 5;

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
            method_hits: HashMap::new(),
            fast_hits: HashMap::new(),
            event_queues: HashMap::new(),
            event_params: HashMap::new(),
            activity: Rc::new(std::cell::Cell::new(0)),
            compiled: HashMap::new(),
            wake: false,
            method_logged: HashSet::new(),
            type_cache: HashMap::new(),
            player_names: HashMap::new(),
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
            profile: Vec::new(),
            idle_backoff: 0.6,
            finished_plans: Vec::new(),
        }
    }

    /// Longest service interval for scripts that only poll (no engine calls for 5 services); 0 disables the
    /// back-off. Default 0.6 s, i.e. an idle script is serviced every 0.6 s instead of every ServiceRate.
    pub fn set_idle_backoff(&mut self, seconds: f64) {
        self.idle_backoff = seconds;
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
        let s = self.shared.borrow();
        let mut out = s.api_hits.clone();
        for (name, c) in &s.fast_hits {
            if c.get() > 0 {
                *out.entry(name.clone()).or_default() += c.get();
            }
        }
        for (k, v) in &s.method_hits {
            if v.get() > 0 {
                out.insert(format!(":{k}"), v.get());
            }
        }
        out
    }

    /// API names that reached an unimplemented stub.
    pub fn unimplemented(&self) -> BTreeSet<String> {
        self.shared.borrow().unknown.clone()
    }

    /// Number of running script instances.
    pub fn alive_scripts(&self) -> usize {
        self.scripts.iter().filter(|s| s.alive).count()
    }

    /// Wall time spent servicing each script instance: `(name, total, services)`.
    pub fn profile(&self) -> Vec<(String, std::time::Duration, u32)> {
        let names = self.shared.borrow().script_names.clone();
        self.profile
            .iter()
            .enumerate()
            .map(|(i, (d, n))| (names[i].clone(), *d, *n))
            .collect()
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
        self.spawn_inner(name, globals, None)
    }

    /// Starts an AI plan script (`Scripts/AI/*.lua`): like [`spawn_script`](Self::spawn_script) with
    /// `PlayerObject` and `Target` set, but instead of `main` the plan's `TaskForce` declaration is turned
    /// into host task forces (global `SpaceForce`, `ReserveForce`, ...) and each `<Name>_Thread` function
    /// becomes a thread. The plan ends when all its threads end or `ScriptExit()` is called.
    pub fn spawn_plan(
        &mut self,
        name: &str,
        plan: u64,
        player: PlayerId,
        target: Option<u64>,
    ) -> Result<usize, String> {
        let mut globals = vec![("PlayerObject", ScriptValue::Player(player))];
        if let Some(t) = target {
            globals.push(("Target", ScriptValue::Object(t)));
        }
        self.spawn_inner(name, &globals, Some((plan, player, target)))
    }

    /// Plan instances that ended since the last call.
    pub fn take_finished_plans(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.finished_plans)
    }

    /// Stops a plan instance.
    pub fn kill_plan(&mut self, plan: u64) {
        for s in &mut self.scripts {
            if s.plan == Some(plan) && s.alive {
                s.alive = false;
                s.threads.clear();
            }
        }
        self.sweep_finished_plans();
    }

    fn sweep_finished_plans(&mut self) {
        // Ended scripts give their Lua state back.
        for s in &mut self.scripts {
            if !s.alive && s.lua.is_some() {
                s.threads.clear();
                s.lua = None;
            }
        }
        for s in &mut self.scripts {
            if let Some(p) = s.plan
                && !s.alive
                && !s.reported
            {
                s.reported = true;
                self.finished_plans.push(p);
            }
        }
    }

    fn spawn_inner(
        &mut self,
        name: &str,
        globals: &[(&str, ScriptValue)],
        plan: Option<(u64, PlayerId, Option<u64>)>,
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
            load_cached(&lua, &sh, "<prelude>", || Ok(api::PRELUDE.to_string()))?;
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
            if let Some((plan_id, player, target)) = plan {
                // Common_Base_Definitions clears PlayerObject / Target; the engine sets them afterwards.
                for (k, v) in globals {
                    if matches!(*k, "PlayerObject" | "Target") {
                        let val = match v {
                            ScriptValue::Player(p) => Some(crate::handles::make(&lua, HandleData { handle: Handle { kind: Kind::Player, id: u64::from(*p) }, shared: sh.clone(), script: id })?),
                            ScriptValue::Object(o) => Some(crate::handles::make(&lua, HandleData { handle: Handle { kind: Kind::Object, id: *o }, shared: sh.clone(), script: id })?),
                            _ => None,
                        };
                        if let Some(val) = val {
                            g.set(*k, val)?;
                        }
                    }
                }
                // Task forces declared by the plan.
                let mut specs: Vec<(String, Vec<String>)> = Vec::new();
                if let Value::Table(defs) = g.get::<Value>("TaskForce")? {
                    for def in defs.sequence_values::<Value>().flatten() {
                        let Value::Table(def) = def else { continue };
                        let mut strings = def.sequence_values::<Value>().flatten().filter_map(|v| match v {
                            Value::String(s) => Some(s.to_string_lossy().to_string()),
                            _ => None,
                        });
                        if let Some(first) = strings.next() {
                            specs.push((first, strings.collect()));
                        }
                    }
                }
                let mut numbers: Vec<(String, f64)> = Vec::new();
                for (k, v) in g.pairs::<String, Value>().flatten() {
                    match v {
                        Value::Number(n) => numbers.push((k, n)),
                        Value::Integer(n) => numbers.push((k, n as f64)),
                        Value::Boolean(b) if k == "IgnoreTarget" => numbers.push((k, f64::from(u8::from(b)))),
                        _ => {}
                    }
                }
                for (tf_name, requirements) in specs {
                    let spec = crate::host::TaskForceSpec {
                        plan: plan_id,
                        name: tf_name.clone(),
                        requirements,
                        player,
                        target,
                        numbers: numbers.clone(),
                    };
                    let Some(tf) = sh.borrow_mut().host.create_taskforce(&spec) else { continue };
                    let handle = crate::handles::make(
                        &lua,
                        HandleData {
                            handle: Handle { kind: Kind::TaskForce, id: tf },
                            shared: sh.clone(),
                            script: id,
                        },
                    )?;
                    g.set(tf_name.as_str(), handle)?;
                    if let Value::Function(f) = g.get::<Value>(format!("{tf_name}_Thread"))? {
                        let tid = {
                            let mut s = sh.borrow_mut();
                            s.next_thread += 1;
                            s.next_thread
                        };
                        threads.push(ThreadSlot { id: tid, thread: lua.create_thread(f)?, first_args: None });
                    }
                }
            } else if let Value::Function(main) = g.get::<Value>("main")? {
                threads.push(ThreadSlot {
                    id: 0,
                    thread: lua.create_thread(main)?,
                    first_args: None,
                });
            }
            Ok(threads)
        };
        let plan_id = plan.map(|p| p.0);
        let mut baseline = HashMap::new();
        match setup() {
            Ok(threads) => {
                let alive = !threads.is_empty();
                if plan_id.is_none() {
                    baseline = crate::persist::globals_of(&lua);
                }
                self.scripts.push(Instance {
                    lua: Some(lua),
                    threads,
                    next_service: 0.0,
                    idle: 0,
                    alive,
                    plan: plan_id,
                    reported: false,
                    baseline,
                });
                self.sweep_finished_plans();
                Ok(id)
            }
            Err(e) => {
                let msg = e.to_string();
                self.errors.push(ScriptError {
                    script: name.to_string(),
                    message: msg.clone(),
                });
                self.scripts.push(Instance {
                    lua: Some(lua),
                    threads: Vec::new(),
                    next_service: 0.0,
                    idle: 0,
                    alive: false,
                    plan: plan_id,
                    reported: false,
                    baseline,
                });
                self.sweep_finished_plans();
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

    /// Copies what a save has to keep (see [`crate::persist`]).
    pub fn snapshot(&self) -> RuntimeSnapshot {
        let sh = self.shared.borrow();
        let mut global_values: Vec<(String, crate::api::GVal)> = sh.global_values.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        global_values.sort_by(|a, b| a.0.cmp(&b.0));
        let mut story_flags: Vec<String> = sh.story_flags.iter().cloned().collect();
        story_flags.sort();
        let mut scripts = Vec::new();
        for (index, inst) in self.scripts.iter().enumerate() {
            let (Some(lua), None, true) = (&inst.lua, inst.plan, inst.alive) else { continue };
            let mut globals: Vec<(String, LuaVal)> = crate::persist::globals_of(lua).into_iter().filter(|(k, v)| inst.baseline.get(k) != Some(v)).collect();
            if globals.is_empty() {
                continue;
            }
            globals.sort_by(|a, b| a.0.cmp(&b.0));
            scripts.push(ScriptSnapshot { index, name: sh.script_names[index].clone(), globals });
        }
        RuntimeSnapshot { time: sh.time, rng: sh.rng, global_values, story_flags, types: sh.types.clone(), story: sh.story.snapshot(), scripts }
    }

    /// Puts a snapshot into a runtime that was just set up like the saved one (same story plots and long-lived scripts, nothing serviced yet).
    /// Returns warnings about parts that did not fit.
    pub fn restore(&mut self, snap: &RuntimeSnapshot) -> Vec<String> {
        let mut warnings = Vec::new();
        {
            let mut sh = self.shared.borrow_mut();
            sh.time = snap.time;
            sh.rng = snap.rng;
            sh.global_values = snap.global_values.iter().cloned().collect();
            sh.story_flags = snap.story_flags.iter().cloned().collect();
            sh.types = snap.types.clone();
            warnings.extend(sh.story.restore(&snap.story));
        }
        for s in &snap.scripts {
            let name_ok = self.shared.borrow().script_names.get(s.index).is_some_and(|n| *n == s.name);
            let Some(lua) = self.scripts.get(s.index).filter(|_| name_ok).and_then(|i| i.lua.clone()) else {
                warnings.push(format!("script {} of the save is not running in this game", s.name));
                continue;
            };
            for (k, v) in &s.globals {
                match crate::persist::from_val(&lua, &self.shared, s.index, v) {
                    Ok(val) => {
                        if let Err(e) = lua.globals().set(k.as_str(), val) {
                            warnings.push(format!("{}: global {k}: {e}", s.name));
                        }
                    }
                    Err(e) => warnings.push(format!("{}: global {k}: {e}", s.name)),
                }
            }
        }
        let now = snap.time;
        for inst in &mut self.scripts {
            inst.next_service = inst.next_service.max(now);
        }
        warnings
    }

    /// Reports a story occurrence from the game (`STORY_CONQUER`, `STORY_CONSTRUCT`, ...).
    pub fn story_notify(&mut self, kind: &str, param: &str) {
        let now = self.time();
        let fired = self.shared.borrow_mut().story.notify(&[kind], param, now);
        self.dispatch(fired);
    }

    /// Reports a finished space battle of `faction` (`STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES`).
    pub fn story_battle(&mut self, won: bool, faction: &str) {
        let now = self.time();
        let kind = if won { "STORY_WIN_BATTLES" } else { "STORY_LOSE_BATTLES" };
        let fired = self.shared.borrow_mut().story.notify_battle(kind, faction, now);
        self.dispatch(fired);
    }

    /// Advances game time by `dt` seconds: story timers, script services.
    pub fn update(&mut self, dt: f64) {
        self.sweep_finished_plans();
        let now = {
            let mut s = self.shared.borrow_mut();
            s.time += dt;
            s.time
        };
        // Story flags were set since the last update: idle scripts may be waiting for them.
        if std::mem::take(&mut self.shared.borrow_mut().wake) {
            for s in &mut self.scripts {
                s.idle = 0;
                s.next_service = s.next_service.min(now);
            }
        }
        // Scripts run first so that their state machines are initialised before story events arrive.
        for sid in 0..self.scripts.len() {
            if self.scripts[sid].alive && now >= self.scripts[sid].next_service {
                let t0 = std::time::Instant::now();
                let activity = self.shared.borrow().activity.get();
                self.service(sid);
                let p = &mut self.profile;
                if p.len() <= sid {
                    p.resize(sid + 1, (std::time::Duration::ZERO, 0));
                }
                p[sid].0 += t0.elapsed();
                p[sid].1 += 1;
                let rate = self.service_rate(sid);
                // A script that polls without calling any engine function backs off (up to
                // `idle_backoff` seconds) until something happens again.
                let inst = &mut self.scripts[sid];
                if self.shared.borrow().activity.get() == activity {
                    inst.idle = (inst.idle + 1).min(1000);
                } else {
                    inst.idle = 0;
                }
                let interval = if inst.idle >= IDLE_SERVICES {
                    rate.max(self.idle_backoff)
                } else {
                    rate
                };
                inst.next_service = now + interval;
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
        self.deliver_plan_events();
        self.sweep_finished_plans();
    }

    /// Queues the plan callbacks the host produced; the plan's own `PumpEvents` runs them inside its
    /// coroutine (so they may `Sleep`), exactly like the engine's event queue.
    fn deliver_plan_events(&mut self) {
        let events = self.shared.borrow_mut().host.take_plan_events();
        for ev in events {
            let Some(idx) = self.scripts.iter().position(|s| s.plan == Some(ev.plan) && s.alive) else { continue };
            self.shared.borrow_mut().event_queues.entry(idx).or_default().push_back((ev.function, ev.args));
            self.scripts[idx].idle = 0;
            self.scripts[idx].next_service = self.scripts[idx].next_service.min(self.time());
        }
    }
    fn service_rate(&self, sid: usize) -> f64 {
        let rate = self.scripts[sid].lua.as_ref().map(|l| l.globals().get::<Value>("ServiceRate"));
        match rate.unwrap_or(Ok(Value::Nil)) {
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
                self.scripts[f.script].idle = 0;
                self.scripts[f.script].next_service =
                    self.scripts[f.script].next_service.min(self.time());
                let name = self.shared.borrow().script_names[f.script].clone();
                self.shared.borrow_mut().current_thread = -1;
                let Some(lua) = self.scripts[f.script].lua.clone() else { continue };
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

/// Compiles `text` once per path and loads the cached bytecode into later states.
fn load_cached(lua: &Lua, sh: &Rc<RefCell<Shared>>, key: &str, source: impl FnOnce() -> mlua::Result<String>) -> mlua::Result<Value> {
    let cached = sh.borrow().compiled.get(key).cloned();
    let code = match cached {
        Some(c) => c,
        None => {
            let text = source()?;
            let f = match lua.load(text.as_str()).set_name(key).into_function() {
                Ok(f) => f,
                // The Clone Wars submod ships `Conquer_Enemy_Planet.lua` with a line that starts with a comma right after a line ending in one (`..., ,"Warlord_Corvette = 0, 10"`):
                // no Lua accepts that, so its AI could never attack. The comma is dropped and the script loads (workaround, RULES.md).
                Err(e) if e.to_string().contains("unexpected symbol near ','") || e.to_string().contains("'}' expected") => {
                    let fixed = repair_double_commas(&text);
                    if fixed == text {
                        return Err(e);
                    }
                    lua.load(fixed.as_str()).set_name(key).into_function()?
                }
                Err(e) => return Err(e),
            };
            let code = Rc::new(f.dump(false));
            sh.borrow_mut().compiled.insert(key.to_string(), code.clone());
            code
        }
    };
    lua.load(&code[..]).set_name(key).set_mode(mlua::ChunkMode::Binary).eval::<Value>()
}

fn run_file(lua: &Lua, sh: &Rc<RefCell<Shared>>, path: &str) -> mlua::Result<Value> {
    load_cached(lua, sh, path, || {
        let bytes = sh.borrow().fs.read(path).map_err(mlua::Error::external)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })
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

/// Repairs the comma slips of a table constructor written one entry per line: a line-leading comma that follows a line ending in a comma is dropped, and a string entry that is
/// followed by another string entry without a comma gets one. Comment and blank lines in between are skipped.
fn repair_double_commas(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let significant = |l: &str| {
        let t = l.trim();
        !(t.is_empty() || t.starts_with("--"))
    };
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    let mut prev: Option<usize> = None;
    for i in 0..lines.len() {
        if !significant(lines[i]) {
            continue;
        }
        let t = lines[i].trim();
        if let Some(p) = prev {
            let pt = out[p].trim().to_owned();
            if pt.ends_with(',') && t.starts_with(',') {
                let at = out[i].find(',').unwrap_or(0);
                out[i].remove(at);
            } else if pt.ends_with('"') && t.starts_with('"') {
                let end = out[p].trim_end_matches(['\r', '\n']).len();
                out[p].insert(end, ',');
            }
        }
        prev = Some(i);
    }
    out.concat()
}

#[cfg(test)]
mod repair_tests {
    use super::*;

    #[test]
    fn stray_and_missing_commas_in_a_table_are_repaired() {
        let src = "t = {\n  \"a\",\n  --c\n  ,\"b\"\n  ,\"c\"\n  -- d\n  \"d\",\n  \"e\"\n}\n";
        assert_eq!(repair_double_commas(src), "t = {\n  \"a\",\n  --c\n  \"b\"\n  ,\"c\",\n  -- d\n  \"d\",\n  \"e\"\n}\n");
        assert_eq!(repair_double_commas("x = 1\n"), "x = 1\n");
    }
}
