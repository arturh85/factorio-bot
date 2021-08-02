use std::path::PathBuf;

pub const APP_SETTINGS_FILENAME: &str = "AppSettings.toml";
// pub const MODS_FOLDERNAME: &str = "mods";
pub const WORKSPACE_FOLDERNAME: &str = "workspace";

pub fn default_app_dir() -> PathBuf {
  tauri::api::path::local_data_dir()
    .expect("no local data directory available")
    .join(env!("CARGO_PKG_NAME"))
}

pub fn app_settings_path() -> PathBuf {
  default_app_dir().join(APP_SETTINGS_FILENAME)
}
pub fn app_workspace_path() -> PathBuf {
  default_app_dir().join(WORKSPACE_FOLDERNAME)
}

// pub fn app_mods_path() -> PathBuf {
//   default_app_dir().join(MODS_FOLDERNAME)
// }
