use crate::app_settings::AppSettings;
// use tauri::State;

#[tauri::command]
pub async fn load_config() -> Result<AppSettings, String> {
  AppSettings::load().map_err(|e| String::from("failed to load config: ") + &e.to_string())
}

#[tauri::command]
pub async fn save_config(_app_handle: tauri::AppHandle<tauri::Wry>) -> Result<(), String> {
  let settings =
    AppSettings::load().map_err(|e| String::from("failed to load config: ") + &e.to_string())?;
  AppSettings::save(settings).map_err(|e| String::from("failed to save config: ") + &e.to_string())
}
