use crate::settings::load_app_settings;
use factorio_bot_core::plan::planner::Planner;
#[cfg(feature = "lua")]
use factorio_bot_scripting_lua::run_lua;
#[cfg(feature = "rhai")]
use factorio_bot_scripting_rhai::run_rhai;
#[cfg(feature = "rune")]
use factorio_bot_scripting_rune::run_rune;
use miette::{miette, IntoDiagnostic};
use std::fs;
use std::path::{Path, PathBuf};

pub async fn run_script(
  planner: &mut Planner,
  language: &str,
  code: &str,
  filename: Option<&str>,
  bot_count: u8,
  redirect: bool,
) -> miette::Result<(String, String)> {
  match language {
    #[cfg(feature = "lua")]
    "lua" => run_lua(planner, code, filename, bot_count, redirect).await,
    #[cfg(feature = "rune")]
    "rune" => run_rune(planner, code, filename, bot_count, redirect).await,
    #[cfg(feature = "rhai")]
    "rhai" => run_rhai(planner, code, filename, bot_count, redirect)
      .await
      .map(|n| n.0),
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
  let workspace_plans_path = prepare_workspace_scripts(workspace_path).unwrap();
  if path.contains("..") {
    return Err(miette!("invalid path"));
  }

  let language = language_by_filename(path);
  if language.is_none() {
    return Err(miette!("unknown scripting file extension"));
  }
  let pathbuf = PathBuf::from(path);
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
  let code = fs::read_to_string(file_path).into_diagnostic()?;
  run_script(
    planner,
    language.unwrap(),
    &code,
    Some(path),
    bot_count,
    redirect,
  )
  .await
}

pub fn language_by_filename(filename: &str) -> Option<&'static str> {
  match Path::new(filename).extension()?.to_str()? {
    "lua" => Some("lua"),
    "rhai" => Some("rhai"),
    "rn" => Some("rune"),
    _ => None,
  }
}

pub fn prepare_workspace_scripts(workspace_path: &Path) -> Result<PathBuf, String> {
  #[allow(unused_mut)]
  let mut workspace_plans_path = workspace_path.join(PathBuf::from("scripts"));
  if !workspace_plans_path.exists() {
    #[cfg(debug_assertions)]
    {
      workspace_plans_path = PathBuf::from("../../scripts");
      if !workspace_plans_path.exists() {
        workspace_plans_path = PathBuf::from("./scripts");
      }
    }
    #[cfg(not(debug_assertions))]
    {
      std::fs::create_dir_all(&workspace_plans_path).map_err(|e| format!("error: {}", e))?;
      if let Err(err) = factorio_bot_core::process::instance_setup::PLANS_CONTENT
        .extract(workspace_plans_path.clone())
      {
        error!("failed to extract static mods content: {:?}", err);
        return Err("failed to extract mods content to workspace".into());
      }
    }
    if !workspace_plans_path.exists() {
      return Err(format!(
        "Missing scripts/ folder from working directory: {:?}",
        workspace_plans_path
      ));
    }
  }
  Ok(fs::canonicalize(workspace_plans_path).expect("Failed to canonicalize workspace_plans_path"))
}
