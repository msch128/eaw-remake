//! Lua userdata handles for engine objects (players, planets / game objects, object types, threads).
//!
//! Like the engine's wrappers, handle methods are *bound*: `obj.Get_Owner()` and `obj:Get_Owner()` both
//! work (a leading argument equal to the handle itself is dropped). Handles are canonical per
//! `(kind, id)` so they can be used as table keys.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{AnyUserData, Lua, MetaMethod, MultiValue, UserData, UserDataMethods, Value};

use crate::api::{self, Shared};

/// What a handle refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A player.
    Player,
    /// A game object (unit, structure) or planet.
    Object,
    /// A game object type (`Find_Object_Type`).
    Type,
    /// A script thread (`Create_Thread`).
    Thread,
    /// The `Script` global of a script instance.
    Script,
    /// A story plot (`Get_Story_Plot`).
    Plot,
    /// A story event of a plot (`plot.Get_Event`).
    PlotEvent,
    /// An AI task force of a plan.
    TaskForce,
    /// A command block returned by task force commands.
    Block,
}

/// An engine object reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle {
    /// Kind.
    pub kind: Kind,
    /// Id within the kind.
    pub id: u64,
}

/// A handle together with the context its methods need.
#[derive(Clone)]
pub struct HandleData {
    /// The handle.
    pub handle: Handle,
    /// Shared runtime state.
    pub shared: Rc<RefCell<Shared>>,
    /// Script instance the handle lives in.
    pub script: usize,
}

impl UserData for HandleData {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_function(MetaMethod::Index, |lua, (ud, key): (AnyUserData, Value)| {
            if !matches!(key, Value::String(_)) {
                return Ok(Value::Nil);
            }
            // Bound methods are created once per handle and name (weak-keyed cache), because scripts call
            // `obj.Method()` in tight loops and creating a closure per access dominated their cost.
            let cache: mlua::Table =
                match lua.named_registry_value::<Option<mlua::Table>>("method_cache")? {
                    Some(t) => t,
                    None => {
                        let t = lua.create_table()?;
                        let mt = lua.create_table()?;
                        mt.set("__mode", "k")?;
                        t.set_metatable(Some(mt))?;
                        lua.set_named_registry_value("method_cache", t.clone())?;
                        t
                    }
                };
            let per_handle: mlua::Table = match cache.get::<Option<mlua::Table>>(ud.clone())? {
                Some(t) => t,
                None => {
                    let t = lua.create_table()?;
                    cache.set(ud.clone(), t.clone())?;
                    t
                }
            };
            if let Some(f) = per_handle.get::<Option<mlua::Function>>(key.clone())? {
                return Ok(Value::Function(f));
            }
            let name = match &key {
                Value::String(s) => s.to_string_lossy().to_string(),
                _ => return Ok(Value::Nil),
            };
            let this = ud.borrow::<HandleData>()?.clone();
            // Players, objects and types count their calls per method name; other kinds count themselves.
            let counted = matches!(this.handle.kind, Kind::Player | Kind::Object | Kind::Type);
            let (counter, activity) = {
                let mut s = this.shared.borrow_mut();
                (s.method_counter(&name), s.activity.clone())
            };
            let f = lua.create_function(move |lua, args: MultiValue| {
                if counted {
                    counter.set(counter.get() + 1);
                    activity.set(activity.get() + 1);
                }
                let mut args: Vec<Value> = args.into_iter().collect();
                // Drop an explicit `self` (colon syntax).
                if let Some(Value::UserData(ud)) = args.first()
                    && let Ok(h) = ud.borrow::<HandleData>()
                    && h.handle == this.handle
                {
                    args.remove(0);
                }
                api::call_method(lua, &this, &name, args)
            })?;
            per_handle.set(key, f.clone())?;
            Ok(Value::Function(f))
        });
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other
                .borrow::<HandleData>()
                .map(|o| o.handle == this.handle)
                .unwrap_or(false))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(api::handle_to_string(this))
        });
    }
}

/// Returns the canonical userdata for a handle, creating it on first use.
pub fn make(lua: &Lua, data: HandleData) -> mlua::Result<Value> {
    let registry: mlua::Table = match lua.named_registry_value::<Option<mlua::Table>>("handles")? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            let mt = lua.create_table()?;
            mt.set("__mode", "v")?;
            t.set_metatable(Some(mt))?;
            lua.set_named_registry_value("handles", t.clone())?;
            t
        }
    };
    let key = ((data.handle.kind as u64) << 56) | (data.handle.id & 0x00FF_FFFF_FFFF_FFFF);
    let key = key as f64;
    if let Value::UserData(ud) = registry.get::<Value>(key)? {
        return Ok(Value::UserData(ud));
    }
    let ud = lua.create_userdata(data)?;
    registry.set(key, ud.clone())?;
    Ok(Value::UserData(ud))
}
