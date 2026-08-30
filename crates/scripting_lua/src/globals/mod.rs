#[allow(clippy::module_inception)]
mod globals;
pub use globals::create_lua_globals;
pub(crate) mod plan;
pub(crate) mod rcon;
pub(crate) mod world;

use std::path::Path;

/// Re-expresses a script-relative path as a path relative to the sandbox root.
///
/// Relative paths in a script mean "next to this script", so they are joined
/// onto `script_dir` first. The resolvers bound against `root`, so the result
/// is handed back as a root-relative string. An argument that is absolute, or
/// one that lands outside the root, is passed through untouched — the
/// resolvers refuse it, and refusing it *there* keeps one rejection path
/// instead of two.
pub(crate) fn relative_to(root: &Path, script_dir: &Path, requested: &str) -> String {
    let joined = script_dir.join(requested);
    match joined.strip_prefix(root) {
        Ok(rest) => rest.to_string_lossy().into_owned(),
        Err(_) => requested.to_owned(),
    }
}
