#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use async_std::sync::{Arc, RwLock};
use async_std::task::JoinHandle;
use factorio_bot_core::process::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use factorio_bot_restapi::webserver::start;
use miette::DiagnosticResult;
use tauri::State;

#[tauri::command]
pub async fn start_restapi(
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  instance_state: State<'_, Arc<RwLock<Option<InstanceState>>>>,
  restapi_handle: State<'_, RwLock<Option<JoinHandle<DiagnosticResult<()>>>>>,
) -> Result<(), String> {
  if restapi_handle.read().await.is_some() {
    return Result::Err("already started".into());
  }
  let app_settings = app_settings.inner().clone();
  let instance_state = instance_state.inner().clone();
  let webserver = start(app_settings, instance_state);
  let handle = async_std::task::spawn(webserver);
  let mut restapi_handle = restapi_handle.write().await;
  *restapi_handle = Some(handle);
  Ok(())
}

#[tauri::command]
pub async fn stop_restapi(
  restapi_handle: State<'_, RwLock<Option<JoinHandle<DiagnosticResult<()>>>>>,
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
pub async fn is_restapi_started(
  restapi_handle: State<'_, RwLock<Option<JoinHandle<DiagnosticResult<()>>>>>,
) -> Result<bool, String> {
  Ok(restapi_handle.read().await.is_some())
}
