use crate::settings::load_app_settings;
use factorio_bot_core::miette;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_scripting_lua::OutputSink;
use std::sync::Arc;

/// Resolves the workspace scripts root from settings, then delegates.
///
/// The resolution itself lives in `factorio_bot_scripting_lua` so the HTTP
/// server, the GUI, the REPL and the CLI cannot drift apart about what a script
/// name means. This used to call `factorio_bot_core::scripts::scripts_dir`,
/// which prefers `./scripts` relative to the *process's* working directory,
/// while the HTTP handlers resolve workspace-only: from a checkout the editor
/// wrote `workspace/scripts/foo.lua` and this ran `./scripts/foo.lua`.
///
/// `sink`, when present, receives the script's output line by line while it
/// runs; the full transcript is returned either way.
pub async fn run_script_file(
  planner: &mut Planner,
  path: &str,
  bot_count: u8,
  sink: Option<Arc<dyn OutputSink>>,
) -> miette::Result<(String, String)> {
  // Was `.unwrap()`, which is reachable from the GUI and from `serve`.
  let app_settings = load_app_settings()?;
  // Was `PathBuf::from(app_settings.factorio.workspace_path.to_string())` --
  // the raw configured string, never checked for being absolute, joined by
  // `ensure_scripts_dir` against the process's working directory. Resolving
  // first is what `ensure_scripts_dir` now requires (it takes
  // `&paths::ResolvedWorkspace`, not `&Path`), so a relative `workspace_path`
  // is refused here instead of silently reading and writing
  // `<cwd>/<relative>/scripts`.
  let workspace_path =
    factorio_bot_core::paths::resolve_workspace(&app_settings.factorio.workspace_path)?;
  let scripts_root = factorio_bot_core::scripts::ensure_scripts_dir(&workspace_path)?;
  // `run_script_file` reports a typed `RunScriptError` so the HTTP server can
  // tell a missing script (404) from a refused traversal (400). Nothing behind
  // this wrapper -- the GUI command, the CLI, the REPL -- does anything with
  // the error but print it, so it is collapsed back to a `Report` here rather
  // than propagated through three call sites that would only re-format it.
  factorio_bot_scripting_lua::run_script_file(planner, &scripts_root, path, bot_count, sink)
    .await
    .map_err(factorio_bot_scripting_lua::RunScriptError::into_report)
}
