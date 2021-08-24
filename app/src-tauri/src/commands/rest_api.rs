#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use async_std::sync::{Arc, RwLock};
use async_std::task::JoinHandle;
use factorio_bot_core::factorio::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use factorio_bot_restapi::server::start_webserver;
use tauri::State;

#[tauri::command]
pub async fn start_restapi(
  app_settings: State<'_, RwLock<AppSettings>>,
  instance_state: State<'_, Arc<RwLock<Option<InstanceState>>>>,
  restapi_handle: State<'_, RwLock<Option<JoinHandle<anyhow::Result<()>>>>>,
) -> Result<(), String> {
  if restapi_handle.read().await.is_some() {
    return Result::Err("already started".into());
  }
  let app_settings = app_settings.read().await.clone();
  let instance_state = instance_state.inner().clone();
  let handle = async_std::task::spawn(start_webserver(app_settings, instance_state));
  let mut restapi_handle = restapi_handle.write().await;
  *restapi_handle = Some(handle);
  Ok(())
}

#[tauri::command]
pub async fn stop_restapi(
  restapi_handle: State<'_, RwLock<Option<JoinHandle<anyhow::Result<()>>>>>,
) -> Result<(), String> {
  if restapi_handle.read().await.is_none() {
    return Result::Err("not started".into());
  }
  let mut restapi_handle = restapi_handle.write().await;
  if let Some(handle) = restapi_handle.take() {
    handle.cancel().await;
  }
  Ok(())
}

#[tauri::command]
pub async fn is_port_available(port: u16) -> bool {
  port_scanner::local_port_available(port)
}
