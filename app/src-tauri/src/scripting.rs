use crate::settings::load_app_settings;
use factorio_bot_core::miette;
use factorio_bot_core::miette::{miette, IntoDiagnostic};
use factorio_bot_core::plan::planner::Planner;
#[cfg(feature = "lua")]
use factorio_bot_scripting_lua::run_lua;
// #[cfg(feature = "rhai")]
// use factorio_bot_scripting_rhai::run_rhai;
// #[cfg(feature = "rune")]
// use factorio_bot_scripting_rune::run_rune;
use std::fs;
use std::path::{Path, PathBuf};

/// `scripts_root` is the sandbox boundary handed to the interpreter: every
/// filesystem operation the script can reach is bounded by it.
#[allow(unused_variables)]
pub async fn run_script(
  planner: &mut Planner,
  language: &str,
  code: &str,
  filename: Option<&str>,
  scripts_root: &Path,
  bot_count: u8,
  redirect: bool,
) -> miette::Result<(String, String)> {
  match language {
    #[cfg(feature = "lua")]
    "lua" => run_lua(planner, code, filename, scripts_root, bot_count, redirect)
      .await
      .map(|n| n.1),
    // #[cfg(feature = "rune")]
    // "rune" => run_rune(planner, code, filename, bot_count, redirect).await,
    // #[cfg(feature = "rhai")]
    // "rhai" => run_rhai(planner, code, filename, bot_count, redirect)
    //   .await
    //   .map(|n| n.0),
    _ => Err(miette!(format!("unknown language: \"{}\"", language))),
  }
}

pub async fn run_script_file(
  planner: &mut Planner,
  path: &str,
  bot_count: u8,
  redirect: bool,
) -> miette::Result<(String, String)> {
  let app_settings = load_app_settings().unwrap();
  let workspace_path = app_settings.factorio.workspace_path.to_string();
  let workspace_path = Path::new(&workspace_path);
  let workspace_plans_path = factorio_bot_core::scripts::scripts_dir(workspace_path)?;
  if path.contains("..") {
    return Err(miette!("invalid path"));
  }

  // Normalize path: strip leading "/" or "scripts/" prefix
  let normalized_path = path.trim_start_matches('/').trim_start_matches("scripts/");

  let language = language_by_filename(normalized_path);
  if language.is_none() {
    return Err(miette!("unknown scripting file extension"));
  }
  let pathbuf = PathBuf::from(normalized_path);
  let file_path = workspace_plans_path.join(&pathbuf);
  if !file_path.exists() {
    return Err(miette!(format!(
      "path not found: {}",
      file_path.as_os_str().to_str().unwrap().replace('\\', "/")
    )));
  }
  if !file_path.is_file() {
    return Err(miette!("path not a file"));
  }
  let code = fs::read_to_string(&file_path).into_diagnostic()?;
  let full_path = file_path.to_str().expect("invalid path");
  run_script(
    planner,
    language.unwrap(),
    &code,
    Some(full_path),
    &workspace_plans_path,
    bot_count,
    redirect,
  )
  .await
}

pub fn language_by_filename(filename: &str) -> Option<&'static str> {
  match Path::new(filename).extension()?.to_str()? {
    "lua" => Some("lua"),
    // "rhai" => Some("rhai"),
    // "rn" => Some("rune"),
    _ => None,
  }
}
