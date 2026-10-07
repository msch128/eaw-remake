//! AI-facing script API: task force and command block handles, and the engine AI functions plan scripts
//! call (`FindTarget`, `GiveDesireBonus`, `_ProduceObject`, ...). Everything is forwarded to the host.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Sh, as_handle, handle_value, hit, unknown};
use crate::handles::{HandleData, Kind};
use crate::host::{HostValue, PlayerId};

/// Engine AI functions bound to [`crate::ScriptHost::ai_call`].
pub const AI_FUNCTIONS: &[&str] = &[
    "FindDeadlyEnemy",
    "GiveDesireBonus",
    "WaitForStarbase",
    "_ProduceObject",
    "_FindStageArea",
    "Find_Path",
    "Project_By_Unit_Range",
    "Evaluate_In_Galactic_Context",
    "Purge_Goals",
    "Find_Best_Local_Threat_Center",
    "Get_Most_Defended_Position",
    "Find_Nearest_Space_Field",
    "Assemble_Fleet",
    "Get_Difficulty",
    "StringCompare",
    "EvaluateTypeList",
    "Force_Test_Space_Conflict",
    "Spawn_From_Reinforcement_Pool",
];

/// Lua value -> host value.
pub(crate) fn to_host(sh: &Sh, v: &Value) -> HostValue {
    match v {
        Value::Nil => HostValue::Nil,
        Value::Boolean(b) => HostValue::Bool(*b),
        Value::Integer(i) => HostValue::Num(*i as f64),
        Value::Number(n) => HostValue::Num(*n),
        Value::String(s) => HostValue::Str(s.to_string_lossy().to_string()),
        Value::Table(t) => HostValue::List(t.clone().sequence_values::<Value>().flatten().map(|x| to_host(sh, &x)).collect()),
        other => match as_handle(other) {
            Some(h) => match h.handle.kind {
                Kind::Object => HostValue::Object(h.handle.id),
                Kind::Player => HostValue::Player(h.handle.id as PlayerId),
                Kind::Type => HostValue::Type(sh.borrow().types.get(h.handle.id as usize).cloned().unwrap_or_default()),
                Kind::TaskForce => HostValue::TaskForce(h.handle.id),
                Kind::Block => HostValue::Block(h.handle.id),
                _ => HostValue::Nil,
            },
            None => HostValue::Nil,
        },
    }
}

/// Host value -> Lua value.
pub(crate) fn from_host(lua: &Lua, sh: &Sh, script: usize, v: HostValue) -> mlua::Result<Value> {
    Ok(match v {
        HostValue::Nil => Value::Nil,
        HostValue::Bool(b) => Value::Boolean(b),
        HostValue::Num(n) => Value::Number(n),
        HostValue::Str(s) => Value::String(lua.create_string(&s)?),
        HostValue::Object(o) => handle_value(lua, sh, script, Kind::Object, o)?,
        HostValue::Player(p) => handle_value(lua, sh, script, Kind::Player, u64::from(p))?,
        HostValue::Type(name) => {
            let id = sh.borrow_mut().type_id(&name);
            handle_value(lua, sh, script, Kind::Type, id)?
        }
        HostValue::List(items) => {
            let t: Table = lua.create_table()?;
            for (i, it) in items.into_iter().enumerate() {
                t.set(i + 1, from_host(lua, sh, script, it)?)?;
            }
            Value::Table(t)
        }
        HostValue::Block(b) => handle_value(lua, sh, script, Kind::Block, b)?,
        HostValue::TaskForce(t) => handle_value(lua, sh, script, Kind::TaskForce, t)?,
    })
}

/// Methods of a task force handle.
pub(crate) fn call_taskforce(lua: &Lua, h: &HandleData, name: &str, args: &[Value]) -> mlua::Result<Value> {
    let sh = &h.shared;
    hit(sh, &format!("TaskForce.{name}"));
    if name == "Is_Valid" {
        return Ok(Value::Boolean(true));
    }
    let hargs: Vec<HostValue> = args.iter().map(|a| to_host(sh, a)).collect();
    let ret = sh.borrow_mut().host.taskforce_call(h.handle.id, name, &hargs);
    match ret {
        Some(v) => from_host(lua, sh, h.script, v),
        None => {
            unknown(sh, &format!("TaskForce.{name}"));
            Ok(Value::Nil)
        }
    }
}

/// Methods of a command block handle.
pub(crate) fn call_block(lua: &Lua, h: &HandleData, name: &str) -> mlua::Result<Value> {
    let _ = lua;
    let sh = &h.shared;
    let (finished, result) = sh.borrow_mut().host.block_status(h.handle.id);
    Ok(match name {
        "IsFinished" => Value::Boolean(finished),
        "Result" => result.map_or(Value::Nil, Value::Boolean),
        "Is_Valid" => Value::Boolean(true),
        _ => Value::Nil,
    })
}

/// Binds the AI functions into a script's globals.
pub(crate) fn install(lua: &Lua, sh: &Sh, script: usize) -> mlua::Result<()> {
    let g = lua.globals();
    for name in AI_FUNCTIONS {
        let shc = sh.clone();
        let f = lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
            hit(&shc, name);
            let hargs: Vec<HostValue> = args.iter().map(|a| to_host(&shc, a)).collect();
            let ret = shc.borrow_mut().host.ai_call(name, &hargs);
            match ret {
                Some(v) => from_host(lua, &shc, script, v),
                None => {
                    unknown(&shc, name);
                    Ok(Value::Nil)
                }
            }
        })?;
        g.set(*name, f)?;
    }

    // `FindTarget(...)` and `FindTarget.Reachable_Target(...)`.
    let find = lua.create_table()?;
    let mt = lua.create_table()?;
    let shc = sh.clone();
    mt.set(
        "__call",
        lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
            hit(&shc, "FindTarget");
            let hargs: Vec<HostValue> = args.iter().skip(1).map(|a| to_host(&shc, a)).collect();
            let ret = shc.borrow_mut().host.ai_call("FindTarget", &hargs);
            match ret {
                Some(v) => from_host(lua, &shc, script, v),
                None => {
                    unknown(&shc, "FindTarget");
                    Ok(Value::Nil)
                }
            }
        })?,
    )?;
    find.set_metatable(Some(mt))?;
    let shc = sh.clone();
    find.set(
        "Reachable_Target",
        lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
            hit(&shc, "FindTarget.Reachable_Target");
            let hargs: Vec<HostValue> = args.iter().map(|a| to_host(&shc, a)).collect();
            let ret = shc.borrow_mut().host.ai_call("FindTarget.Reachable_Target", &hargs);
            match ret {
                Some(v) => from_host(lua, &shc, script, v),
                None => {
                    unknown(&shc, "FindTarget.Reachable_Target");
                    Ok(Value::Nil)
                }
            }
        })?,
    )?;
    g.set("FindTarget", find)?;
    Ok(())
}
