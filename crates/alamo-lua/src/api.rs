//! Engine API bindings: global functions, namespaces and handle methods.
//!
//! Implemented calls talk to the [`ScriptHost`]. Cinematic / camera / UI / audio calls are accepted and
//! ignored (counted in the hit statistics). Every other API name found in the scripts is bound to a stub
//! that logs once and returns `nil`, so unknown calls never crash a script.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use alamo_fs::FileSource;
use mlua::{Function, Lua, MultiValue, Table, Thread, Value};

use crate::handles::{self, Handle, HandleData, Kind};
use crate::host::{GameMode, ObjectFilter, ObjectId, PlayerId, ScriptHost, SpawnAt};
use crate::story::StoryEngine;

include!("api_names.rs");

/// Namespaces whose members are all bound (unknown members become logging stubs).
const STUB_NAMESPACES: &[&str] = &[
    "FogOfWar",
    "DiscreteDistribution",
    "WeightedTypeList",
    "SFXManager",
    "FindPlanet",
    "FindTarget",
    "Intervention",
];

/// Value stored by `GlobalValue.Set`.
#[derive(Debug, Clone, PartialEq)]
pub enum GVal {
    /// Number.
    Num(f64),
    /// String.
    Str(String),
    /// Boolean.
    Bool(bool),
}

/// A thread created by `Create_Thread`, waiting to be adopted by the scheduler.
pub struct NewThread {
    /// Script instance.
    pub script: usize,
    /// Thread id.
    pub id: i64,
    /// The coroutine.
    pub thread: Thread,
    /// Arguments for the first resume.
    pub args: MultiValue,
}

/// State shared by the runtime and all script instances.
pub struct Shared {
    /// The game.
    pub host: Box<dyn ScriptHost>,
    /// Game files.
    pub fs: Arc<dyn FileSource>,
    /// Script file index: lower-case base name -> normalized path.
    pub index: HashMap<String, String>,
    /// Game time in seconds.
    pub time: f64,
    /// Random generator state.
    pub rng: u64,
    /// `GlobalValue` store.
    pub global_values: HashMap<String, GVal>,
    /// Thread id currently running (-1 outside threads).
    pub current_thread: i64,
    /// API names already reported as unimplemented.
    pub logged: HashSet<String>,
    /// Script log lines (`DebugMessage`, ...), capped.
    pub log: Vec<String>,
    /// Call counts per API name (implemented, ignored and stubbed).
    pub api_hits: BTreeMap<String, usize>,
    /// Call counts of handle methods by name (without the leading colon).
    pub method_hits: HashMap<String, Rc<std::cell::Cell<usize>>>,
    /// Per-function counters (see ast_counter); one entry per bound function and script instance.
    /// Engine-call counter that polling scripts do not move (see `ScriptRuntime::set_idle_backoff`).
    pub activity: Rc<std::cell::Cell<u64>>,
    /// Compiled chunks by script path (plus `<prelude>`): every plan runs in a fresh Lua state, compiling is the expensive part.
    pub compiled: HashMap<String, Rc<Vec<u8>>>,
    /// Set when story flags changed: idle scripts must be serviced again.
    pub wake: bool,
    /// Events queued for plan scripts (GetEvent): function name and arguments, per script instance.
    pub event_queues: HashMap<usize, std::collections::VecDeque<(String, Vec<crate::host::HostValue>)>>,
    /// Arguments of the event GetEvent returned last, per script instance.
    pub event_params: HashMap<usize, Vec<crate::host::HostValue>>,
    /// Per-function counters (see `fast_counter`); one entry per bound function and script instance.
    pub fast_hits: Vec<(String, Rc<std::cell::Cell<usize>>)>,
    /// Methods already reported as unimplemented.
    pub method_logged: HashSet<String>,
    /// Cache for `Type` numeric lookups by (type id, kind).
    pub type_cache: HashMap<(u64, u8), Option<f32>>,
    /// Lower-cased player names by id.
    pub player_names: HashMap<PlayerId, String>,
    /// Names that hit an unimplemented stub.
    pub unknown: BTreeSet<String>,
    /// Threads created during the current resume.
    pub new_threads: Vec<NewThread>,
    /// `(script, thread)` pairs to kill.
    pub killed: Vec<(usize, i64)>,
    /// Scripts that called `_ScriptExit`.
    pub exit: HashSet<usize>,
    /// Script names by instance id.
    pub script_names: Vec<String>,
    /// Interned object type names.
    pub types: Vec<String>,
    /// Events fired by `Story_Event` calls whose script callbacks are still to be made.
    pub pending_fired: Vec<crate::story::Fired>,
    /// Next thread id.
    pub next_thread: i64,
    /// Story plots.
    pub story: StoryEngine,
    /// Story flags (`Check_Story_Flag`).
    pub story_flags: HashSet<String>,
}

impl Shared {
    /// Interns a type name.
    pub fn type_id(&mut self, name: &str) -> u64 {
        if let Some(i) = self.types.iter().position(|t| t.eq_ignore_ascii_case(name)) {
            return i as u64;
        }
        self.types.push(name.to_string());
        (self.types.len() - 1) as u64
    }

    pub(crate) fn next_random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Applies the effects of a fired story event that do not need Lua: `TRIGGER_AI` sets a story flag
    /// (read by `Check_Story_Flag`), all non-graph rewards go to the host.
    pub fn apply_rewards(&mut self, f: &crate::story::Fired) {
        if let Some((dialog, chapter)) = &f.dialog {
            self.host
                .story_dialog(dialog, *chapter, &f.event, f.faction.as_deref());
        }
        for r in &f.host_rewards {
            if r.kind == "TRIGGER_AI" {
                let flag = r
                    .params
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let faction = r
                    .params
                    .get(1)
                    .cloned()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                self.wake = true;
                self.story_flags.insert(if faction.is_empty() {
                    flag
                } else {
                    format!("{flag}|{faction}")
                });
            }
            let reward = crate::host::StoryReward {
                kind: r.kind.clone(),
                params: r.params.clone(),
                event: f.event.clone(),
                faction: f.faction.clone(),
            };
            self.host.story_reward(&reward);
        }
    }

    /// The call counter of a handle method (created on first use; one per method name).
    pub(crate) fn method_counter(&mut self, name: &str) -> Rc<std::cell::Cell<usize>> {
        if let Some(c) = self.method_hits.get(name) {
            return c.clone();
        }
        let c = Rc::new(std::cell::Cell::new(0));
        self.method_hits.insert(name.to_string(), c.clone());
        c
    }

    pub(crate) fn push_log(&mut self, line: String) {
        if self.log.len() < 5000 {
            self.log.push(line);
        }
    }
}

pub(crate) type Sh = Rc<RefCell<Shared>>;

pub(crate) fn hit(sh: &Sh, name: &str) {
    let mut s = sh.borrow_mut();
    s.activity.set(s.activity.get() + 1);
    // Look up first: inserting needs an owned key, existing entries must not allocate.
    if let Some(c) = s.api_hits.get_mut(name) {
        *c += 1;
    } else {
        s.api_hits.insert(name.to_string(), 1);
    }
}

/// A call counter owned by one bound function; merged into [`Shared::api_hits`] when read. Avoids a map
/// lookup per call for the hot engine functions.
fn fast_counter(sh: &Sh, name: &str) -> Rc<std::cell::Cell<usize>> {
    let c = Rc::new(std::cell::Cell::new(0));
    sh.borrow_mut()
        .fast_hits
        .push((name.to_string(), c.clone()));
    c
}

/// Reports an unimplemented method (first use is logged; later calls only count).
fn unknown_method(sh: &Sh, name: &str) {
    let mut s = sh.borrow_mut();
    if !s.method_logged.contains(name) {
        s.method_logged.insert(name.to_string());
        let full = format!(":{name}");
        s.unknown.insert(full.clone());
        s.host.unimplemented_api(&full);
        s.push_log(format!("[unimplemented API] {full}"));
    }
}

pub(crate) fn unknown(sh: &Sh, name: &str) {
    let mut s = sh.borrow_mut();
    s.activity.set(s.activity.get() + 1);
    if let Some(c) = s.api_hits.get_mut(name) {
        *c += 1;
    } else {
        s.api_hits.insert(name.to_string(), 1);
    }
    if !s.logged.contains(name) {
        s.logged.insert(name.to_string());
        s.unknown.insert(name.to_string());
        s.host.unimplemented_api(name);
        s.push_log(format!("[unimplemented API] {name}"));
    }
}

/// Calls that are accepted and ignored: presentation (cinematics, camera, UI, audio) and AI hints.
fn is_noop(name: &str) -> bool {
    const PARTS: &[&str] = &[
        "Cinematic",
        "Camera",
        "Letter_Box",
        "Fade_Screen",
        "Lock_Controls",
        "Radar_Blip",
        "Music",
        "Speech",
        "Bink",
        "Highlight",
        "SFX",
        "Ambient_VO",
        "Weather",
        "Remove_All_Text",
        "Dialog",
        "Particle",
        "Play_Animation",
        "Hide",
        "Fog",
        "Zoom",
        "Scroll",
        "Volume",
        "Lightning",
        "Markup",
        "Reveal",
        "Planet_Highlight",
        "Objective",
        "Face_Immediate",
        "Turn_To_Face",
        "Teleport",
        "Set_Selectable",
        "Suspend_AI",
        "Prevent_",
        "Suspend_Locomotor",
        "Override_Max_Speed",
        "Cancel_Fast_Forward",
        "Select_Object",
        "Allow_Localized",
        "Set_Cannot_Be_Killed",
        "Reward_Parameter",
        "Event_Parameter",
        "Hyperspace",
        "Reset",
        "Suspend",
        "Activate",
        "Debug",
    ];
    PARTS.iter().any(|p| name.contains(p))
}

pub(crate) fn handle_value(lua: &Lua, sh: &Sh, script: usize, kind: Kind, id: u64) -> mlua::Result<Value> {
    handles::make(
        lua,
        HandleData {
            handle: Handle { kind, id },
            shared: sh.clone(),
            script,
        },
    )
}

fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Integer(i) => Some(*i as f64),
        Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
        _ => None,
    }
}

fn to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_string_lossy().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

pub(crate) fn as_handle(v: &Value) -> Option<HandleData> {
    match v {
        Value::UserData(ud) => ud.borrow::<HandleData>().ok().map(|h| h.clone()),
        _ => None,
    }
}

fn list_to_table(lua: &Lua, items: Vec<Value>) -> mlua::Result<Value> {
    Ok(Value::Table(lua.create_sequence_from(items)?))
}

fn position_table(lua: &Lua, p: [f32; 3]) -> mlua::Result<Value> {
    let t = lua.create_table()?;
    t.set("x", p[0])?;
    t.set("y", p[1])?;
    t.set("z", p[2])?;
    Ok(Value::Table(t))
}

fn player_value(lua: &Lua, sh: &Sh, script: usize, p: Option<PlayerId>) -> mlua::Result<Value> {
    match p {
        Some(p) => handle_value(lua, sh, script, Kind::Player, u64::from(p)),
        None => Ok(Value::Nil),
    }
}

pub(crate) fn handle_to_string(h: &HandleData) -> String {
    let s = h.shared.borrow();
    match h.handle.kind {
        Kind::Player => format!(
            "Player({})",
            s.host
                .player(h.handle.id as PlayerId)
                .map(|p| p.name)
                .unwrap_or_default()
        ),
        Kind::Object => match s.host.object(h.handle.id) {
            Some(o) => format!("Object({}#{})", o.type_name, o.id),
            None => format!("Object(#{} invalid)", h.handle.id),
        },
        Kind::Type => format!(
            "Type({})",
            s.types
                .get(h.handle.id as usize)
                .cloned()
                .unwrap_or_default()
        ),
        Kind::Thread => format!("Thread({})", h.handle.id),
        Kind::Script => s
            .script_names
            .get(h.handle.id as usize)
            .cloned()
            .unwrap_or_default(),
        Kind::Plot => format!("Plot({})", h.handle.id),
        Kind::PlotEvent => format!("PlotEvent({})", h.handle.id),
        Kind::TaskForce => format!("TaskForce({})", h.handle.id),
        Kind::Block => format!("Block({})", h.handle.id),
    }
}

/// Dispatches a method call on a handle.
pub(crate) fn call_method(
    lua: &Lua,
    h: &HandleData,
    name: &str,
    args: Vec<Value>,
) -> mlua::Result<Value> {
    let sh = &h.shared;
    let script = h.script;
    let id = h.handle.id;
    let a = |i: usize| args.get(i);
    match h.handle.kind {
        Kind::Script => {
            hit(sh, &format!("Script.{name}"));
            Ok(Value::Boolean(false))
        }
        Kind::TaskForce => crate::ai_api::call_taskforce(lua, h, name, &args),
        Kind::Block => crate::ai_api::call_block(lua, h, name),
        Kind::Plot => {
            hit(sh, &format!("Plot.{name}"));
            match name {
                "Get_Event" => {
                    let ev = a(0).and_then(to_string).unwrap_or_default();
                    let idx = sh.borrow().story.event_index(id as usize, &ev);
                    match idx {
                        Some(e) => {
                            handle_value(lua, sh, script, Kind::PlotEvent, (id << 32) | e as u64)
                        }
                        None => {
                            sh.borrow_mut()
                                .push_log(format!("Plot.Get_Event: unknown event '{ev}'"));
                            Ok(Value::Nil)
                        }
                    }
                }
                "Reset" => {
                    let mut s = sh.borrow_mut();
                    let now = s.time;
                    s.story.reset_plot_index(id as usize, now);
                    Ok(Value::Nil)
                }
                _ => Ok(Value::Nil),
            }
        }
        Kind::PlotEvent => {
            hit(sh, &format!("PlotEvent.{name}"));
            if name == "Set_Reward_Parameter" {
                let idx = a(0).and_then(to_f64).unwrap_or(0.0) as usize;
                let value = a(1).and_then(|v| type_name_of(sh, v)).unwrap_or_default();
                sh.borrow_mut().story.set_reward_param(
                    (id >> 32) as usize,
                    (id & 0xFFFF_FFFF) as usize,
                    idx,
                    &value,
                );
            }
            Ok(Value::Nil)
        }
        Kind::Thread => {
            hit(sh, &format!("Thread.{name}"));
            Ok(Value::Nil)
        }
        Kind::Type => match name {
            "Get_Name" => Ok(Value::String(
                lua.create_string(
                    sh.borrow()
                        .types
                        .get(id as usize)
                        .cloned()
                        .unwrap_or_default(),
                )?,
            )),
            "Is_Valid" => Ok(Value::Boolean(true)),
            "Get_Combat_Rating" | "Get_Build_Cost" | "Get_Score_Cost_Credits" => {
                let key = match name {
                    "Get_Combat_Rating" => "combat_rating",
                    "Get_Build_Cost" => "build_cost",
                    _ => "score_cost",
                };
                let ck = (id, name.len() as u8);
                let cached = sh.borrow().type_cache.get(&ck).copied();
                let v = match cached {
                    Some(v) => v,
                    None => {
                        let mut s = sh.borrow_mut();
                        let tn = s.types.get(id as usize).cloned().unwrap_or_default();
                        let v = s.host.type_number(&tn, key);
                        s.type_cache.insert(ck, v);
                        v
                    }
                };
                Ok(Value::Number(f64::from(v.unwrap_or(0.0))))
            }
            _ => {
                if !is_noop(name) {
                    unknown_method(sh, name);
                }
                Ok(Value::Nil)
            }
        },
        Kind::Player => {
            let pid = id as PlayerId;
            let info = sh.borrow().host.player(pid);
            match name {
                "Is_Valid" => Ok(Value::Boolean(info.is_some())),
                "Get_Faction_Name" => Ok(Value::String(
                    lua.create_string(info.map(|i| i.name.to_uppercase()).unwrap_or_default())?,
                )),
                "Get_Name" => Ok(Value::String(
                    lua.create_string(info.map(|i| i.name).unwrap_or_default())?,
                )),
                "Is_Human" => Ok(Value::Boolean(info.is_some_and(|i| i.is_human))),
                "Is_Neutral" => Ok(Value::Boolean(info.is_some_and(|i| i.is_neutral))),
                "Get_ID" => Ok(Value::Number(f64::from(pid))),
                "Get_Credits" => Ok(Value::Number(info.map_or(0.0, |i| f64::from(i.credits)))),
                "Get_Difficulty" => Ok(Value::String(
                    lua.create_string(info.map(|i| i.difficulty).unwrap_or_default())?,
                )),
                "Get_Tech_Level" => {
                    Ok(Value::Number(info.map_or(0.0, |i| f64::from(i.tech_level))))
                }
                "Give_Money" => {
                    let amount = a(0).and_then(to_f64).unwrap_or(0.0) as f32;
                    sh.borrow_mut().host.give_money(pid, amount);
                    Ok(Value::Nil)
                }
                "Set_Tech_Level" => {
                    let lvl = a(0).and_then(to_f64).unwrap_or(0.0) as i32;
                    sh.borrow_mut().host.set_tech_level(pid, lvl);
                    Ok(Value::Nil)
                }
                "Make_Ally" => {
                    if let Some(o) = a(0).and_then(as_handle) {
                        sh.borrow_mut().host.make_ally(pid, o.handle.id as PlayerId);
                    }
                    Ok(Value::Nil)
                }
                "Is_Enemy" | "Is_Ally" => {
                    let enemy = a(0)
                        .and_then(as_handle)
                        .is_some_and(|o| sh.borrow().host.is_enemy(pid, o.handle.id as PlayerId));
                    Ok(Value::Boolean(if name == "Is_Enemy" {
                        enemy
                    } else {
                        !enemy
                    }))
                }
                _ => {
                    if !is_noop(name) {
                        unknown_method(sh, name);
                    }
                    Ok(Value::Nil)
                }
            }
        }
        Kind::Object => {
            let info = sh.borrow().host.object(id);
            if name == "Is_Valid" {
                return Ok(Value::Boolean(info.is_some()));
            }
            let Some(o) = info else {
                // Calls on dead objects are harmless no-ops in the engine.
                return Ok(Value::Nil);
            };
            match name {
                "Get_Type" => {
                    let t = sh.borrow_mut().type_id(&o.type_name);
                    handle_value(lua, sh, script, Kind::Type, t)
                }
                "Get_Name" => Ok(Value::String(lua.create_string(&o.name)?)),
                "Get_Owner" => player_value(lua, sh, script, o.owner),
                "Get_Planet_Location" => match o.planet {
                    Some(p) => handle_value(lua, sh, script, Kind::Object, p),
                    None if o.is_planet => handle_value(lua, sh, script, Kind::Object, o.id),
                    None => Ok(Value::Nil),
                },
                "Get_Parent_Object" => Ok(Value::Nil),
                "Get_Position" => position_table(lua, o.position),
                "Get_Hull" => Ok(Value::Number(f64::from(o.hull))),
                "Get_ID" => Ok(Value::Number(o.id as f64)),
                "Get_Distance" => {
                    let other = a(0)
                        .and_then(as_handle)
                        .and_then(|t| sh.borrow().host.object(t.handle.id));
                    Ok(Value::Number(other.map_or(0.0, |t| {
                        let d: f32 = (0..3)
                            .map(|i| (o.position[i] - t.position[i]).powi(2))
                            .sum();
                        f64::from(d.sqrt())
                    })))
                }
                "Is_Category" => {
                    let cat = a(0).and_then(to_string).unwrap_or_default();
                    Ok(Value::Boolean(
                        o.categories.iter().any(|c| c.eq_ignore_ascii_case(&cat)),
                    ))
                }
                "Despawn" => {
                    sh.borrow_mut().host.despawn(id);
                    Ok(Value::Nil)
                }
                "Change_Owner" => {
                    if let Some(p) = a(0).and_then(as_handle) {
                        sh.borrow_mut()
                            .host
                            .change_owner(id, p.handle.id as PlayerId);
                    }
                    Ok(Value::Nil)
                }
                "Move_To" => {
                    if let Some(t) = a(0).and_then(as_handle) {
                        sh.borrow_mut()
                            .host
                            .move_to(id, SpawnAt::Planet(t.handle.id));
                    }
                    Ok(Value::Nil)
                }
                "Is_Hero"
                | "Has_Property"
                | "Is_In_Garrison"
                | "Has_Garrison"
                | "Is_Planet_Destroyed" => Ok(Value::Boolean(false)),
                _ => {
                    if !is_noop(name) {
                        unknown_method(sh, name);
                    }
                    Ok(Value::Nil)
                }
            }
        }
    }
}

fn type_name_of(sh: &Sh, v: &Value) -> Option<String> {
    if let Some(h) = as_handle(v) {
        return match h.handle.kind {
            Kind::Type => sh.borrow().types.get(h.handle.id as usize).cloned(),
            Kind::Object => sh.borrow().host.object(h.handle.id).map(|o| o.type_name),
            _ => None,
        };
    }
    to_string(v)
}

fn find_objects(sh: &Sh, what: Option<&Value>, extra: Option<&Value>) -> Vec<ObjectId> {
    let mut filter = ObjectFilter::default();
    let mut name = None;
    if let Some(w) = what {
        if let Some(h) = as_handle(w) {
            match h.handle.kind {
                Kind::Player => filter.owner = Some(h.handle.id as PlayerId),
                Kind::Object => filter.planet = Some(h.handle.id),
                _ => name = type_name_of(sh, w),
            }
        } else {
            name = type_name_of(sh, w);
        }
    }
    if let Some(h) = extra.and_then(as_handle) {
        match h.handle.kind {
            Kind::Player => filter.owner = Some(h.handle.id as PlayerId),
            Kind::Object => filter.planet = Some(h.handle.id),
            _ => {}
        }
    }
    let s = sh.borrow();
    match name {
        Some(n) => {
            filter.type_name = Some(n.clone());
            let by_type = s.host.find_objects(&filter);
            if !by_type.is_empty() {
                return by_type;
            }
            filter.type_name = None;
            filter.category = Some(n);
            s.host.find_objects(&filter)
        }
        None => s.host.find_objects(&filter),
    }
}

fn spawn_units(lua: &Lua, sh: &Sh, script: usize, args: &[Value]) -> mlua::Result<Value> {
    let Some(what) = args.first() else {
        return list_to_table(lua, vec![]);
    };
    let mut types = Vec::new();
    match what {
        Value::Table(t) => {
            for v in t.clone().sequence_values::<Value>().flatten() {
                types.extend(type_name_of(sh, &v));
            }
        }
        v => types.extend(type_name_of(sh, v)),
    }
    let at = match args.get(1) {
        Some(v) => match (as_handle(v), v) {
            (Some(h), _) if h.handle.kind == Kind::Object => {
                let planet = sh.borrow().host.object(h.handle.id).map(|o| {
                    if o.is_planet {
                        o.id
                    } else {
                        o.planet.unwrap_or(o.id)
                    }
                });
                planet.map(SpawnAt::Planet)
            }
            (None, Value::Table(t)) => Some(SpawnAt::Position([
                t.get("x").unwrap_or(0.0),
                t.get("y").unwrap_or(0.0),
                t.get("z").unwrap_or(0.0),
            ])),
            _ => None,
        },
        None => None,
    };
    let owner = args
        .get(2)
        .and_then(as_handle)
        .filter(|h| h.handle.kind == Kind::Player)
        .map(|h| h.handle.id as PlayerId);
    let (Some(at), Some(owner)) = (at, owner) else {
        return list_to_table(lua, vec![]);
    };
    let mut out = Vec::new();
    for t in types {
        let id = sh.borrow_mut().host.spawn(&t, at, owner);
        if let Some(id) = id {
            out.push(handle_value(lua, sh, script, Kind::Object, id)?);
        }
    }
    list_to_table(lua, out)
}

fn stub(lua: &Lua, sh: &Sh, name: String, noop: bool) -> mlua::Result<Function> {
    let sh = sh.clone();
    lua.create_function(move |_, _: MultiValue| {
        if noop {
            hit(&sh, &name);
        } else {
            unknown(&sh, &name);
        }
        Ok(Value::Nil)
    })
}

/// Installs the engine API into a fresh Lua state for script instance `script`.
pub fn install(lua: &Lua, sh: &Sh, script: usize) -> mlua::Result<()> {
    let g = lua.globals();

    // Stubs first; real implementations below overwrite them.
    for name in GLOBAL_STUBS {
        if name.starts_with(|c: char| c.is_ascii_uppercase())
            && g.get::<Value>(*name)? == Value::Nil
        {
            g.set(*name, stub(lua, sh, (*name).to_string(), is_noop(name))?)?;
        }
    }
    for ns in STUB_NAMESPACES {
        let t = lua.create_table()?;
        let mt = lua.create_table()?;
        let shc = sh.clone();
        let nsn = (*ns).to_string();
        mt.set(
            "__index",
            lua.create_function(move |lua, (_t, key): (Value, Value)| {
                let key = to_string(&key).unwrap_or_default();
                let full = format!("{nsn}.{key}");
                Ok(Value::Function(stub(
                    lua,
                    &shc,
                    full.clone(),
                    is_noop(&full),
                )?))
            })?,
        )?;
        t.set_metatable(Some(mt))?;
        g.set(*ns, t)?;
    }

    macro_rules! def {
        ($sh:ident, $name:expr, |$lua:ident, $args:ident| $body:block) => {{
            let shc = sh.clone();
            let counter = fast_counter(sh, $name);
            let activity = if matches!($name, "GetThreadID" | "Check_Story_Flag") {
                None
            } else {
                Some(sh.borrow().activity.clone())
            };
            let f = lua.create_function(move |$lua, $args: MultiValue| -> mlua::Result<Value> {
                let $args: Vec<Value> = $args.into_iter().collect();
                counter.set(counter.get() + 1);
                if let Some(a) = &activity {
                    a.set(a.get() + 1);
                }
                #[allow(unused_variables)]
                let $sh = &shc;
                $body
            })?;
            g.set($name, f)?;
        }};
    }

    // ---- core ---------------------------------------------------------------------------------
    {
        let shc = sh.clone();
        let t = lua.create_table()?;
        let mt = lua.create_table()?;
        let shc2 = shc.clone();
        let counter = fast_counter(sh, "GetCurrentTime");
        mt.set(
            "__call",
            lua.create_function(move |_, _: MultiValue| {
                counter.set(counter.get() + 1);
                Ok(shc2.borrow().time)
            })?,
        )?;
        t.set_metatable(Some(mt))?;
        let shc3 = shc.clone();
        t.set(
            "Frame",
            lua.create_function(move |_, _: MultiValue| Ok((shc3.borrow().time * 30.0).floor()))?,
        )?;
        g.set("GetCurrentTime", t)?;
    }
    def!(sh, "GetThreadID", |lua, args| {
        let _ = (lua, args);
        Ok(Value::Number(sh.borrow().current_thread as f64))
    });
    def!(sh, "_ScriptExit", |lua, args| {
        let _ = (lua, args);
        sh.borrow_mut().exit.insert(script);
        Ok(Value::Nil)
    });
    for name in [
        "DebugMessage",
        "ScriptMessage",
        "_ScriptMessage",
        "_MessagePopup",
        "_OuputDebug",
        "OutputDebug",
        "MessageBox",
        "MessagBox",
        "ScriptError",
        "_CustomScriptMessage",
    ] {
        let shc = sh.clone();
        let f = lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
            hit(&shc, name);
            let text = args
                .iter()
                .filter_map(to_string)
                .collect::<Vec<_>>()
                .join(" ");
            let _ = lua;
            let mut s = shc.borrow_mut();
            let who = s.script_names.get(script).cloned().unwrap_or_default();
            if name == "ScriptError" || name == "MessageBox" {
                s.push_log(format!("[{who}] {name}: {text}"));
            }
            s.host.log(&who, &text);
            Ok(Value::Nil)
        })?;
        g.set(name, f)?;
    }
    def!(sh, "Game_Message", |lua, args| {
        let _ = lua;
        let text = args.first().and_then(to_string).unwrap_or_default();
        sh.borrow_mut().host.message(&text);
        Ok(Value::Nil)
    });
    def!(sh, "Get_Game_Mode", |lua, args| {
        let _ = args;
        Ok(Value::String(
            lua.create_string(sh.borrow().host.game_mode().as_str())?,
        ))
    });
    def!(sh, "Is_Campaign_Game", |lua, args| {
        let _ = (lua, args);
        Ok(Value::Boolean(
            sh.borrow().host.game_mode() == GameMode::Galactic,
        ))
    });
    def!(sh, "Is_Multiplayer_Mode", |lua, args| {
        let _ = (lua, args);
        Ok(Value::Boolean(false))
    });

    // ---- threads ------------------------------------------------------------------------------
    {
        let shc = sh.clone();
        let t = lua.create_table()?;
        let mt = lua.create_table()?;
        let shc2 = shc.clone();
        mt.set(
            "__call",
            lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
                hit(&shc2, "Create_Thread");
                let mut args: Vec<Value> = args.into_iter().collect();
                if args.is_empty() {
                    return Ok(Value::Nil);
                }
                args.remove(0); // the table itself
                let f = match args.first() {
                    Some(Value::Function(f)) => f.clone(),
                    Some(Value::String(s)) => match lua.globals().get::<Value>(s.to_str()?)? {
                        Value::Function(f) => f,
                        _ => {
                            unknown(&shc2, "Create_Thread(missing function)");
                            return Ok(Value::Nil);
                        }
                    },
                    _ => return Ok(Value::Nil),
                };
                let rest: Vec<Value> = args.into_iter().skip(1).collect();
                let thread = lua.create_thread(f)?;
                let id = {
                    let mut s = shc2.borrow_mut();
                    s.next_thread += 1;
                    s.next_thread
                };
                shc2.borrow_mut().new_threads.push(NewThread {
                    script,
                    id,
                    thread,
                    args: MultiValue::from_iter(rest),
                });
                handle_value(lua, &shc2, script, Kind::Thread, id as u64)
            })?,
        )?;
        t.set_metatable(Some(mt))?;
        let shc3 = shc.clone();
        t.set(
            "Kill",
            lua.create_function(move |_, args: MultiValue| {
                hit(&shc3, "Create_Thread.Kill");
                if let Some(h) = args.iter().next().and_then(as_handle) {
                    shc3.borrow_mut().killed.push((script, h.handle.id as i64));
                }
                Ok(())
            })?,
        )?;
        let shc4 = shc.clone();
        t.set(
            "Is_Thread_Active",
            lua.create_function(move |_, _: MultiValue| {
                hit(&shc4, "Create_Thread.Is_Thread_Active");
                Ok(true)
            })?,
        )?;
        g.set("Create_Thread", t)?;
        // `Thread.Kill(handle)` alias.
        let th = lua.create_table()?;
        let shc5 = shc.clone();
        th.set(
            "Kill",
            lua.create_function(move |_, args: MultiValue| {
                hit(&shc5, "Thread.Kill");
                if let Some(h) = args.iter().next().and_then(as_handle) {
                    shc5.borrow_mut().killed.push((script, h.handle.id as i64));
                }
                Ok(())
            })?,
        )?;
        g.set("Thread", th)?;
    }

    // ---- random / global values ---------------------------------------------------------------
    {
        let gr = lua.create_table()?;
        let mt = lua.create_table()?;
        let free = {
            let shc = sh.clone();
            lua.create_function(move |_, args: MultiValue| {
                hit(&shc, "GameRandom.Free_Random");
                let vals: Vec<f64> = args.iter().filter_map(to_f64).collect();
                let (lo, hi) = match vals.as_slice() {
                    [a, b, ..] => (*a as i64, *b as i64),
                    [a] => (0, *a as i64),
                    _ => (0, 1),
                };
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                let r = shc.borrow_mut().next_random();
                Ok((lo + (r % ((hi - lo + 1) as u64)) as i64) as f64)
            })?
        };
        let free2 = free.clone();
        mt.set(
            "__call",
            lua.create_function(move |_, args: MultiValue| {
                free2.call::<f64>(args.into_iter().skip(1).collect::<MultiValue>())
            })?,
        )?;
        gr.set_metatable(Some(mt))?;
        gr.set("Free_Random", free)?;
        let shc = sh.clone();
        gr.set(
            "Get_Float",
            lua.create_function(move |_, _: MultiValue| {
                hit(&shc, "GameRandom.Get_Float");
                Ok((shc.borrow_mut().next_random() >> 11) as f64 / (1u64 << 53) as f64)
            })?,
        )?;
        g.set("GameRandom", gr)?;

        let gv = lua.create_table()?;
        let shc = sh.clone();
        gv.set(
            "Set",
            lua.create_function(move |_, args: MultiValue| {
                hit(&shc, "GlobalValue.Set");
                let a: Vec<Value> = args.into_iter().collect();
                if let (Some(k), Some(v)) = (a.first().and_then(to_string), a.get(1)) {
                    let val = match v {
                        Value::Boolean(b) => Some(GVal::Bool(*b)),
                        Value::String(s) => Some(GVal::Str(s.to_string_lossy().to_string())),
                        other => to_f64(other).map(GVal::Num),
                    };
                    let mut s = shc.borrow_mut();
                    match val {
                        Some(v) => {
                            s.global_values.insert(k, v);
                        }
                        None => {
                            s.global_values.remove(&k);
                        }
                    }
                }
                Ok(())
            })?,
        )?;
        let shc = sh.clone();
        gv.set(
            "Get",
            lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
                hit(&shc, "GlobalValue.Get");
                let k = args.iter().next().and_then(to_string).unwrap_or_default();
                Ok(match shc.borrow().global_values.get(&k) {
                    Some(GVal::Num(n)) => Value::Number(*n),
                    Some(GVal::Str(s)) => Value::String(lua.create_string(s)?),
                    Some(GVal::Bool(b)) => Value::Boolean(*b),
                    None => Value::Nil,
                })
            })?,
        )?;
        g.set("GlobalValue", gv)?;
    }

    // ---- lookups ------------------------------------------------------------------------------
    def!(sh, "Find_Player", |lua, args| {
        let name = args.first().and_then(to_string).unwrap_or_default();
        let found = {
            let s = shc_borrow(sh);
            if name.eq_ignore_ascii_case("local") {
                s.host.local_player()
            } else {
                s.host.find_player(&name)
            }
        };
        player_value(lua, sh, script, found)
    });
    def!(sh, "Find_Object_Type", |lua, args| {
        let name = args.first().and_then(to_string).unwrap_or_default();
        if !sh.borrow().host.object_type_exists(&name) {
            return Ok(Value::Nil);
        }
        let t = sh.borrow_mut().type_id(&name);
        handle_value(lua, sh, script, Kind::Type, t)
    });
    def!(sh, "Find_All_Objects_Of_Type", |lua, args| {
        let ids = find_objects(sh, args.first(), args.get(1));
        let vals = ids
            .into_iter()
            .map(|i| handle_value(lua, sh, script, Kind::Object, i))
            .collect::<mlua::Result<Vec<_>>>()?;
        list_to_table(lua, vals)
    });
    def!(sh, "Find_First_Object", |lua, args| {
        match find_objects(sh, args.first(), args.get(1)).first() {
            Some(i) => handle_value(lua, sh, script, Kind::Object, *i),
            None => Ok(Value::Nil),
        }
    });
    def!(sh, "Find_Nearest", |lua, args| {
        let origin = args
            .first()
            .and_then(as_handle)
            .and_then(|h| sh.borrow().host.object(h.handle.id));
        let cands = find_objects(sh, args.get(1), args.get(2));
        let best = cands
            .into_iter()
            .filter(|i| Some(*i) != origin.as_ref().map(|o| o.id))
            .filter_map(|i| sh.borrow().host.object(i))
            .min_by(|a, b| {
                let d = |o: &crate::host::ObjectInfo| {
                    origin.as_ref().map_or(0.0, |p| {
                        (0..3)
                            .map(|k| (p.position[k] - o.position[k]).powi(2))
                            .sum::<f32>()
                    })
                };
                d(a).total_cmp(&d(b))
            });
        match best {
            Some(o) => handle_value(lua, sh, script, Kind::Object, o.id),
            None => Ok(Value::Nil),
        }
    });
    {
        // `FindPlanet("Name")` and `FindPlanet.Get_All_Planets()`.
        let t = lua.create_table()?;
        let mt = lua.create_table()?;
        let shc = sh.clone();
        mt.set(
            "__call",
            lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
                hit(&shc, "FindPlanet");
                let name = args.iter().nth(1).and_then(to_string).unwrap_or_default();
                let id = shc.borrow().host.planet_by_name(&name);
                match id {
                    Some(i) => handle_value(lua, &shc, script, Kind::Object, i),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
        t.set_metatable(Some(mt))?;
        let shc = sh.clone();
        t.set(
            "Get_All_Planets",
            lua.create_function(move |lua, _: MultiValue| -> mlua::Result<Value> {
                hit(&shc, "FindPlanet.Get_All_Planets");
                let ids = shc.borrow().host.all_planets();
                let vals = ids
                    .into_iter()
                    .map(|i| handle_value(lua, &shc, script, Kind::Object, i))
                    .collect::<mlua::Result<Vec<_>>>()?;
                list_to_table(lua, vals)
            })?,
        )?;
        g.set("FindPlanet", t)?;
    }
    def!(sh, "EvaluatePerception", |lua, args| {
        let _ = lua;
        let name = args.first().and_then(to_string).unwrap_or_default();
        let player = args
            .get(1)
            .and_then(as_handle)
            .map(|h| h.handle.id as PlayerId);
        let target = args.get(2).and_then(as_handle).map(|h| h.handle.id);
        let v = player.and_then(|p| sh.borrow().host.evaluate_perception(&name, p, target));
        match v {
            Some(v) => Ok(Value::Number(f64::from(v))),
            None => {
                unknown(sh, &format!("EvaluatePerception({name})"));
                Ok(Value::Number(0.0))
            }
        }
    });
    def!(sh, "Spawn_Unit", |lua, args| {
        spawn_units(lua, sh, script, &args)
    });
    def!(sh, "Reinforce_Unit", |lua, args| {
        spawn_units(lua, sh, script, &args)
    });
    def!(sh, "Create_Generic_Object", |lua, args| {
        let list = spawn_units(lua, sh, script, &args)?;
        match list {
            Value::Table(t) => t.get::<Value>(1),
            _ => Ok(Value::Nil),
        }
    });

    // ---- weighted random lists ------------------------------------------------------------------
    // `DiscreteDistribution.Create()` / `WeightedTypeList.Create()`: native, because the spawn scripts
    // sample them hundreds of thousands of times.
    for class in ["DiscreteDistribution", "WeightedTypeList"] {
        let ns = lua.create_table()?;
        let shc = sh.clone();
        ns.set(
            "Create",
            lua.create_function(move |lua, _: MultiValue| -> mlua::Result<Table> {
                hit(&shc, "WeightedList.Create");
                let items: Rc<RefCell<(Vec<Value>, Vec<f64>)>> =
                    Rc::new(RefCell::new((Vec::new(), Vec::new())));
                let obj = lua.create_table()?;
                let list = items.clone();
                obj.set(
                    "Insert",
                    lua.create_function(move |_, args: MultiValue| {
                        // `list.Insert(item, weight)` or `list:Insert(item, weight)`.
                        let mut a: Vec<Value> = args.into_iter().collect();
                        if a.len() >= 3
                            || matches!(a.first(), Some(Value::Table(_)))
                                && a.len() == 2
                                && to_f64(&a[1]).is_none()
                        {
                            a.remove(0);
                        }
                        let mut it = a.into_iter();
                        let Some(item) = it.next() else { return Ok(()) };
                        let weight = it.next().as_ref().and_then(to_f64).unwrap_or(1.0);
                        let mut l = list.borrow_mut();
                        let total = l.1.last().copied().unwrap_or(0.0) + weight;
                        l.0.push(item);
                        l.1.push(total);
                        Ok(())
                    })?,
                )?;
                let list = items.clone();
                let shr = shc.clone();
                obj.set(
                    "Sample",
                    lua.create_function(move |_, _: MultiValue| -> mlua::Result<Value> {
                        let l = list.borrow();
                        let Some(&total) = l.1.last() else {
                            return Ok(Value::Nil);
                        };
                        if total <= 0.0 {
                            return Ok(Value::Nil);
                        }
                        let r = (shr.borrow_mut().next_random() >> 11) as f64 / (1u64 << 53) as f64
                            * total;
                        let idx = l.1.partition_point(|&c| c < r).min(l.0.len() - 1);
                        Ok(l.0[idx].clone())
                    })?,
                )?;
                let list = items.clone();
                obj.set(
                    "Size",
                    lua.create_function(move |_, _: MultiValue| Ok(list.borrow().0.len()))?,
                )?;
                let list = items.clone();
                obj.set(
                    "Clear",
                    lua.create_function(move |_, _: MultiValue| {
                        let mut l = list.borrow_mut();
                        l.0.clear();
                        l.1.clear();
                        Ok(())
                    })?,
                )?;
                obj.set("Parse", lua.create_function(|_, _: MultiValue| Ok(()))?)?;
                Ok(obj)
            })?,
        )?;
        g.set(class, ns)?;
    }

    def!(sh, "DumpCallStack", |lua, args| {
        let _ = args;
        Ok(Value::String(lua.create_string("<call stack not available>")?))
    });

    // ---- GetEvent: callbacks the host queued for a plan, run by the script's own `PumpEvents` ----------
    {
        let ge = lua.create_table()?;
        let mt = lua.create_table()?;
        let shc = sh.clone();
        mt.set(
            "__call",
            lua.create_function(move |lua, _: MultiValue| -> mlua::Result<Value> {
                loop {
                    let next = {
                        let mut s = shc.borrow_mut();
                        let ev = s.event_queues.get_mut(&script).and_then(|q| q.pop_front());
                        if let Some((_, args)) = &ev {
                            s.event_params.insert(script, args.clone());
                        }
                        ev
                    };
                    let Some((function, _)) = next else { return Ok(Value::Nil) };
                    if let Value::Function(f) = lua.globals().get::<Value>(function.as_str())? {
                        hit(&shc, "GetEvent");
                        return Ok(Value::Function(f));
                    }
                }
            })?,
        )?;
        ge.set_metatable(Some(mt))?;
        let shc = sh.clone();
        ge.set(
            "Params",
            lua.create_function(move |lua, _: MultiValue| -> mlua::Result<Value> {
                let params = shc.borrow().event_params.get(&script).cloned().unwrap_or_default();
                if params.is_empty() {
                    return Ok(Value::Nil);
                }
                let t = lua.create_table()?;
                for (i, p) in params.into_iter().enumerate() {
                    t.set(i + 1, crate::ai_api::from_host(lua, &shc, script, p)?)?;
                }
                Ok(Value::Table(t))
            })?,
        )?;
        let shc = sh.clone();
        ge.set(
            "Reset",
            lua.create_function(move |_, _: MultiValue| {
                shc.borrow_mut().event_queues.remove(&script);
                Ok(())
            })?,
        )?;
        g.set("GetEvent", ge)?;
    }

    // ---- story ----------------------------------------------------------------------------------
    def!(sh, "Story_Event", |lua, args| {
        let _ = lua;
        if let Some(n) = args.first().and_then(to_string) {
            // Rewards are applied now (they may depend on parameters set just before the call);
            // the `Story_Event_Trigger` callbacks follow from the runtime.
            let mut s = sh.borrow_mut();
            let now = s.time;
            let fired = s.story.notify(&[], &n, now);
            for f in &fired {
                s.apply_rewards(f);
            }
            s.pending_fired.extend(fired);
        }
        Ok(Value::Nil)
    });
    def!(sh, "Check_Story_Flag", |lua, args| {
        let _ = lua;
        // Fast path: nothing was ever flagged (the common case for polling scripts).
        if sh.borrow().story_flags.is_empty() {
            return Ok(Value::Boolean(false));
        }
        let player = args
            .first()
            .and_then(as_handle)
            .and_then(|h| sh.borrow().host.player(h.handle.id as PlayerId));
        let flag = args
            .get(1)
            .and_then(to_string)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let clear = matches!(args.get(3), Some(Value::Boolean(true)));
        let faction = player
            .map(|p| p.name.to_ascii_lowercase())
            .unwrap_or_default();
        let mut s = sh.borrow_mut();
        let keyed = format!("{flag}|{faction}");
        let found = s.story_flags.contains(&keyed) || s.story_flags.contains(&flag);
        if found && clear {
            s.story_flags.remove(&keyed);
            s.story_flags.remove(&flag);
        }
        Ok(Value::Boolean(found))
    });
    def!(sh, "Get_Story_Plot", |lua, args| {
        let file = args.first().and_then(to_string).unwrap_or_default();
        let idx = sh.borrow().story.plot_index(&file);
        match idx {
            Some(i) => handle_value(lua, sh, script, Kind::Plot, i as u64),
            None => {
                sh.borrow_mut()
                    .push_log(format!("Get_Story_Plot: plot '{file}' is not active"));
                Ok(Value::Nil)
            }
        }
    });
    crate::ai_api::install(lua, sh, script)?;
    Ok(())
}

fn shc_borrow(sh: &Sh) -> std::cell::Ref<'_, Shared> {
    sh.borrow()
}

/// Lua helpers that depend on the thread model (per-thread values, event queue, `Script`).
pub const PRELUDE: &str = r#"
-- Lua 5.0 compatibility: collectgarbage(limit) took a number.
local collectgarbage_51 = collectgarbage
collectgarbage = function(opt, ...)
  if type(opt) == "number" then return 0 end
  return collectgarbage_51(opt, ...)
end
local thread_values = {}
local function tv()
  local id = GetThreadID()
  local t = thread_values[id]
  if not t then t = {}; thread_values[id] = t end
  return t
end
ThreadValue = setmetatable({}, { __call = function(_, key) return tv()[key] end })
ThreadValue.Get = function(key) return tv()[key] end
ThreadValue.Set = function(key, value) tv()[key] = value end
ThreadValue.Reset = function() thread_values[GetThreadID()] = nil end


"#;
