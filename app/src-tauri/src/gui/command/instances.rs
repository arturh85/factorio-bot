#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use crate::paths;
use crate::settings::SharedAppSettings;
use factorio_bot_core::paris::error;
use factorio_bot_core::process::process_control::{
  FactorioInstance, FactorioParams, SharedFactorioInstance,
};
use tauri::{AppHandle, Emitter, State, Wry};

#[tauri::command]
pub async fn start_instances(
  app_handle: AppHandle<Wry>,
  app_settings: State<'_, SharedAppSettings>,
  instance_state: State<'_, SharedFactorioInstance>,
) -> Result<(), String> {
  if instance_state.read().await.is_some() {
    return Result::Err("already started".into());
  }

  let app_settings = app_settings.read().await;
  let workspace_path = paths::workspace_dir();
  std::fs::create_dir_all(&workspace_path).map_err(|e| format!("error: {e}"))?;

  let map_exchange_string = app_settings.factorio.map_exchange_string.to_string();
  let seed = app_settings.factorio.seed.to_string();

  let params = FactorioParams {
    client_count: app_settings.factorio.client_count as u8,
    recreate: app_settings.factorio.recreate,
    seed: if seed.is_empty() { None } else { Some(seed) },
    map_exchange_string: if map_exchange_string.is_empty() {
      None
    } else {
      Some(map_exchange_string)
    },
    ..FactorioParams::default()
  };

  let started_instance_state = FactorioInstance::start(&app_settings.factorio, params).await;

  match started_instance_state {
    Ok(started_instance_state) => {
      let mut instance_state = instance_state.write().await;
      let _copy = (**started_instance_state.world.as_ref().unwrap()).clone();
      *instance_state = Some(started_instance_state);
      app_handle
        .emit("instances_started", true)
        .map_err(|e| format!("error: {e}"))?;
      Ok(())
    }
    Err(err) => {
      error!("failed to start instances: {:?}", err);
      Err(format!("{err:?}"))
    }
  }
}

#[tauri::command]
pub async fn is_instance_started(
  instance_state: State<'_, SharedFactorioInstance>,
) -> Result<bool, String> {
  Ok(instance_state.read().await.is_some())
}

#[tauri::command]
pub async fn stop_instances(
  app_handle: AppHandle<Wry>,
  instance_state: State<'_, SharedFactorioInstance>,
) -> Result<(), String> {
  let mut instance_state = instance_state.write().await;
  if instance_state.is_none() {
    return Err("not started".into());
  }
  instance_state.take().unwrap().stop().unwrap();
  app_handle
    .emit("instances_stopped", true)
    .map_err(|e| format!("error: {e}"))?;
  *instance_state = None;
  Ok(())
}
