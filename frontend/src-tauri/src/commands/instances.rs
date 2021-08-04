#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use crate::constants;
use async_std::sync::RwLock;
use factorio_bot_backend::factorio::process_control::{start_factorio, InstanceState};
use factorio_bot_backend::settings::AppSettings;
use tauri::{AppHandle, Manager, State, Wry};

#[tauri::command]
pub async fn start_instances(
  app_handle: AppHandle<Wry>,
  app_settings: State<'_, RwLock<AppSettings>>,
  instance_state: State<'_, RwLock<Option<InstanceState>>>,
) -> Result<(), String> {
  if instance_state.read().await.is_some() {
    return Result::Err("already started".into());
  }

  let app_settings = app_settings.read().await;
  let workspace_path = constants::app_workspace_path();
  std::fs::create_dir_all(&workspace_path).map_err(|e| String::from("error: ") + &e.to_string())?;

  let map_exchange_string = app_settings.map_exchange_string.to_string();
  let seed = app_settings.seed.to_string();

  let started_instance_state = start_factorio(
    &app_settings,
    None,
    app_settings.client_count as u8,
    app_settings.recreate,
    if map_exchange_string.is_empty() {
      None
    } else {
      Some(map_exchange_string)
    },
    if seed.is_empty() { None } else { Some(seed) },
    true,
    false,
  )
  .await;

  match started_instance_state {
    Ok(started_instance_state) => {
      let mut instance_state = instance_state.write().await;
      *instance_state = Some(started_instance_state);
      app_handle
        .emit_all("instances_started", true)
        .map_err(|e| String::from("error: ") + &e.to_string())?;
      Ok(())
    }
    Err(err) => {
      error!("failed to start instances: {:?}", err);
      Err(err.to_string())
    }
  }
}

#[tauri::command]
pub async fn stop_instances(
  app_handle: AppHandle<Wry>,
  instance_state: State<'_, RwLock<Option<InstanceState>>>,
) -> Result<(), String> {
  let mut instance_state = instance_state.write().await;
  let result: Result<(), String> = match instance_state.as_mut() {
    None => Result::Err("not started".into()),
    Some(instance_state) => {
      for child in &mut instance_state.client_processes {
        if child.kill().is_err() {
          error!("failed to kill client");
        }
      }
      if let Some(server) = instance_state.server_process.as_mut() {
        if server.kill().is_err() {
          error!("failed to kill server");
        }
      }
      app_handle
        .emit_all("instances_stopped", true)
        .map_err(|e| String::from("error: ") + &e.to_string())?;
      Ok(())
    }
  };
  result?;
  *instance_state = None;
  Ok(())
}
