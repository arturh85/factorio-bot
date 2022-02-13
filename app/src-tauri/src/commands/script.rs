#![allow(clippy::module_name_repetitions)]
use factorio_bot_core::process::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use factorio_bot_core::types::PrimeVueTreeNode;
use factorio_bot_lua::planner::Planner;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

#[tauri::command]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub async fn execute_script(
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  instance_state: State<'_, Arc<RwLock<Option<InstanceState>>>>,
  path: String,
) -> Result<(String, String), String> {
  if let Some(instance_state) = &*instance_state.read().await {
    if let Some(world) = &instance_state.world {
      world.entity_graph.connect().unwrap();
      let world = world.clone();
      let rcon = instance_state.rcon.clone();
      let mut planner = Planner::new(world, Some(rcon));
      let bot_count = app_settings.read().await.client_count as u32;

      let app_settings = &app_settings.read().await;
      let workspace_path = app_settings.workspace_path.to_string();
      let workspace_path = Path::new(&workspace_path);
      let workspace_plans_path = prepare_workspace_scripts(workspace_path)?;
      if path.contains("..") {
        return Err("invalid path".into());
      }

      let path = PathBuf::from(&path[1..]);
      let dir_path = workspace_plans_path.join(&path);
      if !dir_path.exists() {
        return Err("path not found".into());
      }
      if !dir_path.is_file() {
        return Err("path not directory".into());
      }
      let lua_code =
        std::fs::read_to_string(dir_path).map_err(|e| String::from("error: ") + &e.to_string())?;
      let (stdout, stderr) = std::thread::spawn(move || planner.plan(lua_code, bot_count))
        .join()
        .unwrap()
        .unwrap();
      return Ok((stdout, stderr));
    }
  }
  warn!("execute_script called without running instance");
  Err("execute_script called without world instance".into())
}

#[tauri::command]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub async fn execute_code(
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  instance_state: State<'_, Arc<RwLock<Option<InstanceState>>>>,
  lua_code: String,
) -> Result<(String, String), String> {
  if let Some(instance_state) = &*instance_state.read().await {
    if let Some(world) = &instance_state.world {
      world.entity_graph.connect().unwrap();
      let world = world.clone();
      let rcon = instance_state.rcon.clone();
      let mut planner = Planner::new(world, Some(rcon));
      let bot_count = app_settings.read().await.client_count as u32;
      let (stdout, stderr) = std::thread::spawn(move || planner.plan(lua_code, bot_count))
        .join()
        .unwrap()
        .unwrap();
      return Ok((stdout, stderr));
    }
  }
  warn!("execute_script called without running instance");
  Err("execute_script called without world instance".into())
}

#[tauri::command]
pub async fn load_scripts_in_directory(
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  path: String,
) -> Result<Vec<PrimeVueTreeNode>, String> {
  let app_settings = &app_settings.read().await;
  let workspace_path = app_settings.workspace_path.to_string();
  let workspace_path = Path::new(&workspace_path);
  let workspace_plans_path = prepare_workspace_scripts(workspace_path)?;

  if path.contains("..") {
    return Err("invalid path".into());
  }

  let dir_path = workspace_plans_path.join(PathBuf::from(&path[1..]));
  let path = PathBuf::from(&path);
  if !dir_path.exists() {
    return Err("path not found".into());
  }
  if !dir_path.is_dir() {
    return Err("path not directory".into());
  }

  let readdir = dir_path
    .read_dir()
    .map_err(|e| String::from("error: ") + &e.to_string())?;

  let result = readdir
    .filter(|entry| entry.is_ok() && entry.as_ref().unwrap().file_type().is_ok())
    .map(|entry| {
      let entry = entry.unwrap();
      let file_type = entry.file_type().unwrap();
      PrimeVueTreeNode {
        key: path
          .join(entry.file_name().to_str().unwrap())
          .to_str()
          .unwrap()
          .to_string()
          .replace("\\", "/"),
        label: entry.file_name().to_str().unwrap().to_string(),
        leaf: !file_type.is_dir(),
        children: vec![],
      }
    })
    .collect();
  Ok(result)
}

#[tauri::command]
pub async fn load_script(
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  path: String,
) -> Result<String, String> {
  let app_settings = &app_settings.read().await;
  let workspace_path = app_settings.workspace_path.to_string();
  let workspace_path = Path::new(&workspace_path);
  let workspace_plans_path = prepare_workspace_scripts(workspace_path)?;
  if path.contains("..") {
    return Err("invalid path".into());
  }

  let path = PathBuf::from(&path[1..]);
  let dir_path = workspace_plans_path.join(&path);
  if !dir_path.exists() {
    return Err("path not found".into());
  }
  if !dir_path.is_file() {
    return Err("path not directory".into());
  }
  std::fs::read_to_string(dir_path).map_err(|e| String::from("error: ") + &e.to_string())
}

fn prepare_workspace_scripts(workspace_path: &Path) -> Result<PathBuf, String> {
  #[allow(unused_mut)]
  let mut workspace_plans_path = workspace_path.join(PathBuf::from("scripts"));
  if !workspace_plans_path.exists() {
    #[cfg(debug_assertions)]
    {
      workspace_plans_path = PathBuf::from("../../scripts");
    }
    #[cfg(not(debug_assertions))]
    {
      std::fs::create_dir_all(&workspace_plans_path)
        .map_err(|e| String::from("error: ") + &e.to_string())?;
      if let Err(err) = factorio_bot_core::process::instance_setup::PLANS_CONTENT
        .extract(workspace_plans_path.clone())
      {
        error!("failed to extract static mods content: {:?}", err);
        return Err("failed to extract mods content to workspace".into());
      }
    }
    if !workspace_plans_path.exists() {
      return Err(format!(
        "missing scripts/ folder from working directory: {:?}",
        workspace_plans_path
      ));
    }
  }
  Ok(workspace_plans_path)
}
