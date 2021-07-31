#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
mod commands;
mod constants;
mod settings;

use crate::constants::default_app_dir;
use crate::settings::AppSettings;
use async_std::sync::Mutex;

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
    .manage(Mutex::new(AppSettings::load()?))
    .run(tauri::generate_context!())
    .expect("failed to run app");
  Ok(())
}
