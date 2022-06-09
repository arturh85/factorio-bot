use factorio_bot_core::constants::WORKSPACE_FOLDERNAME;
use std::path::PathBuf;

#[allow(dead_code)]
pub const APP_SETTINGS_FILENAME: &str = "AppSettings.toml";

pub fn data_local_dir() -> PathBuf {
  dirs_next::data_local_dir()
    .expect("no local data directory available")
    .join(format!(
      "{}{}",
      env!("CARGO_PKG_NAME"),
      if cfg!(debug_assertions) { "-dev" } else { "" }
    ))
}

#[allow(dead_code)]
pub fn settings_file() -> PathBuf {
  data_local_dir().join(APP_SETTINGS_FILENAME)
}
pub fn workspace_dir() -> PathBuf {
  data_local_dir().join(WORKSPACE_FOLDERNAME)
}

// pub fn app_mods_path() -> PathBuf {
//   default_app_dir().join(MODS_FOLDERNAME)
// }
