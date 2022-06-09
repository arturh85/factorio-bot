#![allow(
  clippy::module_name_repetitions,
  clippy::cast_possible_truncation,
  clippy::cast_sign_loss
)]
use crate::context::SharedJoinShandle;
use crate::settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
#[cfg(feature = "restapi")]
use factorio_bot_restapi::webserver::start;
use miette::Result;
use tauri::State;

#[allow(unused_variables)]
#[allow(clippy::unused_async)]
#[tauri::command]
pub async fn start_restapi(
  app_settings: State<'_, SharedAppSettings>,
  instance_state: State<'_, SharedFactorioInstance>,
  restapi_handle: State<'_, SharedJoinShandle<Result<()>>>,
) -> Result<(), String> {
  #[cfg(feature = "restapi")]
  {
    if restapi_handle.read().await.is_some() {
      return Err("already started".into());
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
  #[cfg(not(feature = "restapi"))]
  {
    Err("restapi unavailable".into())
  }
}

#[tauri::command]
pub async fn stop_restapi(
  restapi_handle: State<'_, SharedJoinShandle<Result<()>>>,
) -> Result<(), String> {
  if restapi_handle.read().await.is_none() {
    return Err("not started".into());
  }
  let mut restapi_handle = restapi_handle.write().await;
  if let Some(handle) = restapi_handle.take() {
    handle.abort();
  }
  Ok(())
}

#[tauri::command]
pub async fn is_restapi_started(
  restapi_handle: State<'_, SharedJoinShandle<Result<()>>>,
) -> Result<bool, String> {
  Ok(restapi_handle.read().await.is_some())
}
