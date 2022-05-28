#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use crate::settings::AppSettings;
use factorio_bot_core::process::process_control::InstanceState;
use factorio_bot_restapi::webserver::start;
use miette::Result;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

#[tauri::command]
pub async fn start_restapi(
  app_settings: State<'_, Arc<RwLock<AppSettings>>>,
  instance_state: State<'_, Arc<RwLock<Option<InstanceState>>>>,
  restapi_handle: State<'_, RwLock<Option<JoinHandle<Result<()>>>>>,
) -> Result<(), String> {
  println!("starting restapi 2");
  if restapi_handle.read().await.is_some() {
    return Result::Err("already started".into());
  }
  let app_settings = app_settings.inner().clone();
  let instance_state = instance_state.inner().clone();
  let app_settings = app_settings.read().await;
  let webserver = start(app_settings.restapi.clone(), instance_state);
  let handle = tokio::task::spawn(webserver);
  let mut restapi_handle = restapi_handle.write().await;
  *restapi_handle = Some(handle);
  Ok(())
}

#[tauri::command]
pub async fn stop_restapi(
  restapi_handle: State<'_, RwLock<Option<JoinHandle<Result<()>>>>>,
) -> Result<(), String> {
  if restapi_handle.read().await.is_none() {
    return Result::Err("not started".into());
  }
  let mut restapi_handle = restapi_handle.write().await;
  if let Some(handle) = restapi_handle.take() {
    handle.abort();
  }
  Ok(())
}

#[tauri::command]
pub async fn is_restapi_started(
  restapi_handle: State<'_, RwLock<Option<JoinHandle<Result<()>>>>>,
) -> Result<bool, String> {
  Ok(restapi_handle.read().await.is_some())
}
