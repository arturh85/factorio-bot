#[allow(clippy::module_inception)]
mod globals;
pub use globals::create_lua_globals;
pub(crate) mod goal;
pub(crate) mod rcon;
pub mod record;
pub(crate) mod world;

use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::scripts::ScriptPathError;
use factorio_bot_core::types::Position;
use std::path::Path;

/// Re-expresses a script-relative path as a path relative to the sandbox root.
///
/// Relative paths in a script mean "next to this script", so they are joined
/// onto `script_dir` first. The resolvers bound against `root`, so the result
/// is handed back as a root-relative string.
///
/// An absolute (or, on Windows, merely rooted) argument is refused here rather
/// than passed on. It used to be passed through for the resolvers to reject,
/// but `strip_prefix` silently relativised an absolute path that happened to
/// point *inside* the root, so `resolve_write_path` never saw it as absolute
/// and accepted it — the composed system allowed what the unit test on
/// `resolve_write_path` says is refused. One rule, enforced once, before any
/// joining: a script names paths relative to itself.
pub(crate) fn relative_to(
    root: &Path,
    script_dir: &Path,
    requested: &str,
) -> Result<String, ScriptPathError> {
    let requested_path = Path::new(requested);
    // `is_absolute` is false on Windows for `\foo`, which `join` still
    // resolves against the drive root; `has_root` is what catches that.
    if requested_path.is_absolute() || requested_path.has_root() {
        return Err(ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        });
    }
    let joined = script_dir.join(requested_path);
    // A path that climbs out of the root lexically is handed on unchanged: the
    // resolvers canonicalize and refuse it, and refusing it *there* keeps one
    // rejection path instead of two.
    Ok(match joined.strip_prefix(root) {
        Ok(rest) => rest.to_string_lossy().into_owned(),
        Err(_) => requested.to_owned(),
    })
}

/// Reads an `{x=, y=}` table a script passed in, refusing anything else.
///
/// `table.get("x").unwrap()` panicked on a missing or non-numeric field —
/// `rcon.mine(1, "iron-ore", {}, 1)` was enough — and under `panic = "abort"`
/// that ends the process rather than the script. `argument` names the
/// parameter so the message says which one was wrong.
pub(crate) fn position_from_lua(table: &LuaTable, argument: &str) -> LuaResult<Position> {
    let x: f64 = table
        .get("x")
        .map_err(|_| LuaError::RuntimeError(format!("{argument}: expected a number at `x`")))?;
    let y: f64 = table
        .get("y")
        .map_err(|_| LuaError::RuntimeError(format!("{argument}: expected a number at `y`")))?;
    Ok(Position::new(x, y))
}

/// Every filesystem binding reports a refused path the same way.
pub(crate) fn path_error(err: ScriptPathError) -> LuaError {
    LuaError::RuntimeError(err.to_string())
}
