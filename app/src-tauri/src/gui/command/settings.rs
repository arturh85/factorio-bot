#![allow(clippy::module_name_repetitions)]
use crate::gui::ERR_TO_STRING;
use crate::paths::settings_file;
use crate::settings::{AppSettings, SharedAppSettings};
use tauri::State;
use tokio::sync::RwLock;

#[tauri::command]
pub async fn load_settings(
  app_settings: State<'_, SharedAppSettings>,
) -> Result<AppSettings, String> {
  let app_settings = app_settings.read().await;
  Ok(app_settings.clone())
}

#[tauri::command]
pub async fn save_settings(app_settings: State<'_, RwLock<AppSettings>>) -> Result<(), String> {
  let app_settings = app_settings.write().await;
  AppSettings::save(settings_file(), &app_settings).map_err(ERR_TO_STRING)
}

#[tauri::command]
pub async fn update_settings(
  app_settings: State<'_, SharedAppSettings>,
  settings: AppSettings,
) -> Result<(), String> {
  let mut app_settings = app_settings.write().await;
  *app_settings = settings;
  AppSettings::save(settings_file(), &app_settings).map_err(ERR_TO_STRING)
}
