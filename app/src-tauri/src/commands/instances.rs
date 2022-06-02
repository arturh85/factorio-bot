#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use crate::constants;
use crate::settings::AppSettings;
use factorio_bot_core::process::process_control::{start_factorio, FactorioInstance};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State, Wry};
use tokio::sync::RwLock;

#[tauri::command]
pub async fn start_instances(
  app_handle: AppHandle<Wry>,
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  instance_state: State<'_, Arc<RwLock<Option<FactorioInstance>>>>,
) -> Result<(), String> {
  if instance_state.read().await.is_some() {
    return Result::Err("already started".into());
  }

  let app_settings = app_settings.read().await;
  let workspace_path = constants::app_workspace_path();
  std::fs::create_dir_all(&workspace_path).map_err(|e| format!("error: {}", e))?;

  let map_exchange_string = app_settings.factorio.map_exchange_string.to_string();
  let seed = app_settings.factorio.seed.to_string();

  let started_instance_state = start_factorio(
    &app_settings.factorio,
    None,
    app_settings.factorio.client_count as u8,
    app_settings.factorio.recreate,
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
      let _copy = (**started_instance_state.world.as_ref().unwrap()).clone();
      *instance_state = Some(started_instance_state);
      app_handle
        .emit_all("instances_started", true)
        .map_err(|e| format!("error: {}", e))?;
      Ok(())
    }
    Err(err) => {
      error!("failed to start instances: {:?}", err);
      Err(format!("{:?}", err))
    }
  }
}

#[tauri::command]
pub async fn is_instance_started(
  instance_state: State<'_, Arc<RwLock<Option<FactorioInstance>>>>,
) -> Result<bool, String> {
  Ok(instance_state.read().await.is_some())
}

#[tauri::command]
pub async fn stop_instances(
  app_handle: AppHandle<Wry>,
  instance_state: State<'_, Arc<RwLock<Option<FactorioInstance>>>>,
) -> Result<(), String> {
  let mut instance_state = instance_state.write().await;
  let result: Result<(), String> = match instance_state.as_mut() {
    None => Err("not started".into()),
    Some(instance_state) => {
      instance_state.stop().unwrap();
      app_handle
        .emit_all("instances_stopped", true)
        .map_err(|e| format!("error: {}", e))?;
      Ok(())
    }
  };
  result?;
  *instance_state = None;
  Ok(())
}
