mod globals;
pub mod lua_runner;
pub use lua_runner::{run_lua, PendingWork};
pub mod run_script;
/// Re-exported because they appear in [`run_lua`]'s signature.
pub use factorio_bot_scripting::{OutputSink, Stream};
/// The one place a script *name* is turned into a script *file*. Re-exported
/// at the crate root so no caller has to know which module it lives in — and,
/// more to the point, so no caller is tempted to resolve a name itself.
pub use run_script::{language_by_filename, run_script, run_script_file};
mod error;
pub mod lua_docs;
pub mod roll_best_seed;
mod sandbox;
