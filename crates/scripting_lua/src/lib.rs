pub mod blocked;
mod globals;
pub mod lua_runner;
pub use lua_runner::{PendingWork, run_lua};
pub mod run_script;
/// Re-exported because they appear in [`run_lua`]'s signature.
pub use factorio_bot_scripting::{OutputSink, Stream};
/// The one place a script *name* is turned into a script *file*. Re-exported
/// at the crate root so no caller has to know which module it lives in — and,
/// more to the point, so no caller is tempted to resolve a name itself.
pub use run_script::{
    ResolvedScript, RunScriptError, language_by_filename, resolve_script, run_script,
    run_script_file,
};
/// Holds each `__doc_entry_*` string to the binding installed beside it.
/// Tests only: it asserts about this crate's own source and its live module
/// tables, and nothing outside the test harness calls into it.
#[cfg(test)]
mod doc_guard;
mod error;
pub mod factory_stage1_lib;
pub mod lua_docs;
pub mod research_run_lib;
mod sandbox;
pub mod supervisor_lib;
