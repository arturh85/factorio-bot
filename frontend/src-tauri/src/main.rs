#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
mod commands;
mod constants;

use crate::constants::default_app_dir;
use async_std::sync::Mutex;
use factorio_bot_backend::settings::AppSettings;
use std::borrow::Cow;

fn app_settings() -> anyhow::Result<AppSettings> {
  let mut app_settings = AppSettings::load(constants::app_settings_path())?;
  if app_settings.workspace_path == "" {
    let s: String = constants::app_workspace_path().to_str().unwrap().into();
    app_settings.workspace_path = Cow::from(s);
  }
  Ok(app_settings)
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
  std::fs::create_dir_all(default_app_dir())?;
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      crate::commands::my_custom_command,
      crate::commands::update_settings,
      crate::commands::load_settings,
      crate::commands::save_settings,
      crate::commands::start_instances,
    ])
    .manage(Mutex::new(app_settings()?))
    .run(tauri::generate_context!())
    .expect("failed to run app");
  Ok(())
}
