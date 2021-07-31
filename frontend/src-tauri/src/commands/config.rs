use crate::commands::ERR_TO_STRING;
use crate::settings::AppSettings;
use async_std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub async fn load_config(
  app_settings: State<'_, Mutex<AppSettings>>,
) -> Result<AppSettings, String> {
  let app_settings = app_settings.lock().await;
  Ok(app_settings.clone())
}

#[tauri::command]
pub async fn save_config(app_settings: State<'_, Mutex<AppSettings>>) -> Result<(), String> {
  let app_settings = app_settings.lock().await;
  AppSettings::save(&*app_settings).map_err(ERR_TO_STRING)
}

#[tauri::command]
pub async fn update_config(
  app_settings: State<'_, Mutex<AppSettings>>,
  new_settings: AppSettings,
) -> Result<(), String> {
  let mut app_settings = app_settings.lock().await;
  *app_settings = new_settings;
  Ok(())
}
