use factorio_bot_core::constants::WORKSPACE_FOLDERNAME;
use std::path::PathBuf;

pub const APP_SETTINGS_FILENAME: &str = "AppSettings.toml";

pub fn app_data_dir() -> PathBuf {
  dirs_next::data_local_dir()
    .expect("no local data directory available")
    .join(format!(
      "{}{}",
      env!("CARGO_PKG_NAME"),
      if cfg!(debug_assertions) { "-dev" } else { "" }
    ))
}

pub fn app_settings_path() -> PathBuf {
  app_data_dir().join(APP_SETTINGS_FILENAME)
}
pub fn app_workspace_path() -> PathBuf {
  app_data_dir().join(WORKSPACE_FOLDERNAME)
}

// pub fn app_mods_path() -> PathBuf {
//   default_app_dir().join(MODS_FOLDERNAME)
// }
