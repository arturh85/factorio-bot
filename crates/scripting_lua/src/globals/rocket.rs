// Reachable from one line of user Lua, and this crate builds with
// `panic = "abort"`, so every panic here is a remote kill of the whole server
// process rather than a failed script.
#![deny(clippy::unwrap_used, clippy::expect_used)]

//! Lua API bindings for rocket launch control.
//!
//! Provides `rocket.request{key, payload, ...}`, `rocket.status(key)` and
//! `rocket.launch{key, silo_unit_number}` through remote calls to the
//! BotBridge mod's `botbridge` remote interface.
//!
//! The actual persistent state and event handlers live in
//! `mods/BotBridge/rocket_launch.lua`; this is the Rust-side thin binding
//! that scripts reach.

use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::serde_json;
use std::sync::Arc;

fn rcon_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("rcon: {err}"))
}

/// Escape a string for safe inclusion inside double quotes in a Lua expression
/// that will be sent as an RCON command.
fn escape_lua_string(s: &str) -> String {
    // In Lua double-quoted strings, we need to escape:
    //   \ -> \\  (backslash)
    //   " -> \"  (double quote)
    //   \n, \r, \t are literal escapes
    s.replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn create_lua_rocket(
    lua: &Lua,
    rcon: Arc<FactorioRcon>,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;

    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Rocket launch control
-- Idempotent request-to-launch-to-platform lifecycle.
--
-- @module rocket

local rocket = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return rocket"#))?;

    // ----- rocket.request{key, payload, planet, starter_pack} -----
    let rc = rcon.clone();
    map_table.set(
        "__doc_entry_request",
        String::from(
            r#"
--- Create a rocket launch request.
-- Returns { ok = true, key } or { ok = false, error }.
-- @tparam table opts { key: string, payload?: string, planet?: string,
--                      starter_pack?: {name: string, count: number}[] }
-- @treturn table result
function rocket.request(opts)
end
"#,
        ),
    )?;
    map_table.set(
        "request",
        lua.create_async_function(move |lua, opts: LuaTable| {
            let rc = rc.clone();
            async move {
                let key: String = opts.get("key").map_err(|_| {
                    LuaError::RuntimeError("rocket.request: key is required".into())
                })?;
                let payload: Option<String> = opts.get("payload").ok();
                let planet: Option<String> = opts.get("planet").ok();

                let mut lua_args =
                    format!("{{[\"key\"]=\"{}\"", escape_lua_string(&key));
                if let Some(ref p) = payload {
                    lua_args.push_str(&format!(
                        ",[\"payload\"]=\"{}\"",
                        escape_lua_string(p)
                    ));
                }
                if let Some(ref p) = planet {
                    lua_args.push_str(&format!(
                        ",[\"planet\"]=\"{}\"",
                        escape_lua_string(p)
                    ));
                }
                lua_args.push('}');

                let cmd = format!(
                    "/silent-command remote.call('botbridge','rocket_request',{})",
                    lua_args
                );
                let reply = rc.as_ref().send(&cmd).await.map_err(rcon_error)?;
                let body = reply
                    .ok_or_else(|| LuaError::RuntimeError("rocket.request: no reply".into()))?
                    .join("");
                let decoded: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| {
                        LuaError::RuntimeError(
                            format!("rocket.request: parse error: {e}: {body}")
                        )
                    })?;
                lua.to_value(&decoded)
            }
        })?,
    )?;

    // ----- rocket.status(key) -----
    let rc = rcon.clone();
    map_table.set(
        "__doc_entry_status",
        String::from(
            r#"
--- Query the status of a rocket launch request.
-- Returns a table with key, payload, silo_unit_number and optional tick
-- fields, or { ok = false, error = "..." }.
-- @string key  request key
-- @treturn table status
function rocket.status(key)
end
"#,
        ),
    )?;
    map_table.set(
        "status",
        lua.create_async_function(move |lua, key: String| {
            let rc = rc.clone();
            async move {
                let cmd = format!(
                    "/silent-command remote.call('botbridge','rocket_status',\"{}\")",
                    escape_lua_string(&key)
                );
                let reply = rc.as_ref().send(&cmd).await.map_err(rcon_error)?;
                let body = reply
                    .ok_or_else(|| LuaError::RuntimeError("rocket.status: no reply".into()))?
                    .join("");
                let decoded: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| {
                        LuaError::RuntimeError(
                            format!("rocket.status: parse error: {e}: {body}")
                        )
                    })?;
                lua.to_value(&decoded)
            }
        })?,
    )?;

    // ----- rocket.launch{key, silo_unit_number} -----
    let rc = rcon.clone();
    map_table.set(
        "__doc_entry_launch",
        String::from(
            r#"
--- Order a rocket launch at a specific silo for an existing request.
-- Returns { ok = true, silo_unit_number, launch_ordered_tick } or
-- { ok = false, error = "..." }.
-- @tparam table opts { key: string, silo_unit_number: number }
-- @treturn table result
function rocket.launch(opts)
end
"#,
        ),
    )?;
    map_table.set(
        "launch",
        lua.create_async_function(move |lua, opts: LuaTable| {
            let rc = rc.clone();
            async move {
                let key: String = opts.get("key").map_err(|_| {
                    LuaError::RuntimeError("rocket.launch: key is required".into())
                })?;
                let silo_unit_number: u32 = opts.get("silo_unit_number")
                    .map_err(|_| {
                        LuaError::RuntimeError(
                            "rocket.launch: silo_unit_number is required".into()
                        )
                    })?;
                let cmd = format!(
                    "/silent-command remote.call('botbridge','rocket_launch',\
                     {{[\"key\"]=\"{}\",[\"silo_unit_number\"]={}}})",
                    escape_lua_string(&key), silo_unit_number
                );
                let reply = rc.as_ref().send(&cmd).await.map_err(rcon_error)?;
                let body = reply
                    .ok_or_else(|| LuaError::RuntimeError("rocket.launch: no reply".into()))?
                    .join("");
                let decoded: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| {
                        LuaError::RuntimeError(
                            format!("rocket.launch: parse error: {e}: {body}")
                        )
                    })?;
                lua.to_value(&decoded)
            }
        })?,
    )?;

    // ----- rocket.evidence(key) -----
    let rc = rcon.clone();
    map_table.set(
        "__doc_entry_evidence",
        String::from(
            r#"
--- Build a RocketLaunchEvidence table for a given request key.
-- @string key  request key
-- @treturn table { ok = true, evidence = {...} } or { ok = false, error = "..." }
function rocket.evidence(key)
end
"#,
        ),
    )?;
    map_table.set(
        "evidence",
        lua.create_async_function(move |lua, key: String| {
            let rc = rc.clone();
            async move {
                let cmd = format!(
                    "/silent-command remote.call('botbridge','rocket_evidence',\"{}\")",
                    escape_lua_string(&key)
                );
                let reply = rc.as_ref().send(&cmd).await.map_err(rcon_error)?;
                let body = reply
                    .ok_or_else(|| LuaError::RuntimeError("rocket.evidence: no reply".into()))?
                    .join("");
                let decoded: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| {
                        LuaError::RuntimeError(
                            format!("rocket.evidence: parse error: {e}: {body}")
                        )
                    })?;
                lua.to_value(&decoded)
            }
        })?,
    )?;

    // ----- rocket.list_requests() -----
    let rc = rcon;
    map_table.set(
        "__doc_entry_list_requests",
        String::from(
            r#"
--- List all request keys.
-- @treturn table { ok = true, keys = {string,...} }
function rocket.list_requests()
end
"#,
        ),
    )?;
    map_table.set(
        "list_requests",
        lua.create_async_function(move |lua, ()| {
            let rc = rc.clone();
            async move {
                let cmd = "/silent-command remote.call('botbridge','rocket_list_requests')";
                let reply = rc.as_ref().send(cmd).await.map_err(rcon_error)?;
                let body = reply
                    .ok_or_else(|| {
                        LuaError::RuntimeError(
                            "rocket.list_requests: no reply".into()
                        )
                    })?
                    .join("");
                let decoded: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| {
                        LuaError::RuntimeError(
                            format!("rocket.list_requests: parse error: {e}: {body}")
                        )
                    })?;
                lua.to_value(&decoded)
            }
        })?,
    )?;

    Ok(map_table)
}
