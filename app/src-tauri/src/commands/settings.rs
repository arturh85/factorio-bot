#![allow(clippy::module_name_repetitions)]
use crate::commands::ERR_TO_STRING;
use crate::constants::app_settings_path;
use async_std::sync::RwLock;
use factorio_bot_core::settings::AppSettings;
use tauri::State;

#[tauri::command]

pub async fn load_settings(
  app_settings: State<'_, RwLock<AppSettings>>,
) -> Result<AppSettings, String> {
  let app_settings = app_settings.read().await;
  Ok(app_settings.clone())
}

#[tauri::command]
pub async fn save_settings(app_settings: State<'_, RwLock<AppSettings>>) -> Result<(), String> {
  let app_settings = app_settings.write().await;
  AppSettings::save(app_settings_path(), &*app_settings).map_err(ERR_TO_STRING)
}

#[tauri::command]
pub async fn update_settings(
  app_settings: State<'_, RwLock<AppSettings>>,
  settings: AppSettings,
) -> Result<(), String> {
  let mut app_settings = app_settings.write().await;
  *app_settings = settings;
  AppSettings::save(app_settings_path(), &*app_settings).map_err(ERR_TO_STRING)
}
