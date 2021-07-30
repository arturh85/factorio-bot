use crate::app_settings::AppSettings;

#[tauri::command]
pub async fn load_config() -> AppSettings {
  // app_settings.inner().clone()
  AppSettings::load().expect("aa")
}
#[tauri::command]
pub async fn save_config(_app_handle: tauri::AppHandle<tauri::Wry>) {
  AppSettings::save(AppSettings::load().expect("config failed to load"))
    .expect("config failed to save");
}
