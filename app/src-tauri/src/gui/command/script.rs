#![allow(clippy::module_name_repetitions)]
#[cfg(any(feature = "rhai", feature = "lua"))]
use crate::scripting::{prepare_workspace_scripts, run_script, run_script_file};
use crate::settings::SharedAppSettings;
use factorio_bot_core::paris::warn;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use factorio_bot_core::types::PrimeVueTreeNode;
use tauri::State;

#[tauri::command]
#[allow(
  unused_variables,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
pub async fn execute_script(
  app_settings: State<'_, SharedAppSettings>,
  instance_state: State<'_, SharedFactorioInstance>,
  path: String,
) -> Result<(String, String), String> {
  if let Some(instance_state) = &*instance_state.read().await {
    if let Some(world) = &instance_state.world {
      #[cfg(any(feature = "rhai", feature = "lua"))]
      {
        use factorio_bot_core::plan::planner::Planner;
        world.entity_graph.connect().unwrap();
        let world = world.clone();
        let rcon = instance_state.rcon.clone();
        let mut planner = Planner::new(world, Some(rcon));
        let app_settings = &app_settings.read().await;
        let bot_count = app_settings.factorio.client_count;
        let (stdout, stderr) = run_script_file(&mut planner, &path[1..], bot_count, true)
          .await
          .map_err(|e| format!("error: {:?}", e))?;
        return Ok((stdout, stderr));
      }
      #[cfg(not(any(feature = "rhai", feature = "lua")))]
      {
        return Ok((String::new(), String::new()));
      }
    }
  }
  warn!("execute_script called without running instance");
  Err("execute_script called without world instance".into())
}

#[tauri::command]
#[allow(
  unused_variables,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
pub async fn execute_code(
  app_settings: State<'_, SharedAppSettings>,
  instance_state: State<'_, SharedFactorioInstance>,
  language: String,
  code: String,
) -> Result<(String, String), String> {
  if let Some(instance_state) = &*instance_state.read().await {
    if let Some(world) = &instance_state.world {
      #[cfg(any(feature = "rhai", feature = "lua"))]
      {
        use factorio_bot_core::plan::planner::Planner;
        world.entity_graph.connect().unwrap();
        let world = world.clone();
        let rcon = instance_state.rcon.clone();
        let mut planner = Planner::new(world, Some(rcon));
        let bot_count = app_settings.read().await.factorio.client_count;
        let (stdout, stderr) = run_script(&mut planner, &language, &code, None, bot_count, true)
          .await
          .map_err(|e| format!("error: {:?}", e))?;
        return Ok((stdout, stderr));
      }
      #[cfg(not(any(feature = "rhai", feature = "lua")))]
      {
        return Ok((String::new(), String::new()));
      }
    }
  }
  warn!("execute_script called without running instance");
  Err("execute_script called without world instance".into())
}

#[allow(unused_variables)]
#[tauri::command]
pub async fn load_scripts_in_directory(
  app_settings: State<'_, SharedAppSettings>,
  path: String,
) -> Result<Vec<PrimeVueTreeNode>, String> {
  #[cfg(any(feature = "rhai", feature = "lua"))]
  {
    use std::path::{Path, PathBuf};
    let app_settings = &app_settings.read().await;
    let workspace_path = app_settings.factorio.workspace_path.to_string();
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

    let readdir = dir_path.read_dir().map_err(|e| format!("error: {}", e))?;

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
            .replace('\\', "/"),
          label: entry.file_name().to_str().unwrap().to_owned(),
          leaf: !file_type.is_dir(),
          children: vec![],
        }
      })
      .collect();
    Ok(result)
  }
  #[cfg(not(any(feature = "rhai", feature = "lua")))]
  {
    Ok(Vec::new())
  }
}

#[allow(unused_variables)]
#[tauri::command]
pub async fn load_script(
  app_settings: State<'_, SharedAppSettings>,
  path: String,
) -> Result<String, String> {
  #[cfg(any(feature = "rhai", feature = "lua"))]
  {
    use std::path::{Path, PathBuf};
    let app_settings = &app_settings.read().await;
    let workspace_path = app_settings.factorio.workspace_path.to_string();
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
    std::fs::read_to_string(dir_path).map_err(|e| format!("error: {}", e))
  }
  #[cfg(not(any(feature = "rhai", feature = "lua")))]
  {
    Ok(String::new())
  }
}

#[allow(unused_variables)]
#[tauri::command]
pub async fn save_script(
  app_settings: State<'_, SharedAppSettings>,
  path: String,
  code: String,
) -> Result<(), String> {
  #[cfg(any(feature = "rhai", feature = "lua"))]
  {
    use std::path::{Path, PathBuf};
    let app_settings = &app_settings.read().await;
    let workspace_path = app_settings.factorio.workspace_path.to_string();
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
    std::fs::write(dir_path, code).map_err(|e| format!("error: {}", e))
  }
  #[cfg(not(any(feature = "rhai", feature = "lua")))]
  {
    Ok(String::new())
  }
}
