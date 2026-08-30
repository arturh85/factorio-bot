use factorio_bot_core::mlua::prelude::*;

/// The base-library functions that take a *path* and load code from it.
///
/// mlua opens the base library unconditionally (`luaopen_base` in
/// `state/raw.rs`), so unlike `io` and `os` these cannot be excluded by not
/// asking for them and have to be removed by hand.
const BASE_FILE_LOADERS: [&str; 2] = ["dofile", "loadfile"];

/// Creates the interpreter that user scripts run in.
///
/// [`Lua::new`] loads `StdLib::ALL_SAFE`, where "safe" means *memory* safe: it
/// still contains `io`, `os` and `package`. That is `io.open` writing anywhere
/// the process can, `os.execute` running arbitrary commands, and
/// `package.cpath` loading arbitrary native libraries -- which made bounding
/// `file_write`, `file_read`, `include` and `world.draw` pointless, because the
/// four doors stood beside an open wall. Scripts reach this from an
/// unauthenticated HTTP API, so the standard library is an allow-list.
///
/// `table`, `string`, `math` and `coroutine` are pure computation and stay.
/// `load` also stays: it takes a string rather than a path, so it compiles
/// nothing a script could not have written inline, and its error path is
/// hardened separately (see `crate::error`).
pub(crate) fn new_sandboxed_lua() -> LuaResult<Lua> {
    let lua = Lua::new_with(
        LuaStdLib::TABLE | LuaStdLib::STRING | LuaStdLib::MATH | LuaStdLib::COROUTINE,
        LuaOptions::default(),
    )?;
    let globals = lua.globals();
    for loader in BASE_FILE_LOADERS {
        globals.raw_remove(loader)?;
    }
    Ok(lua)
}
