#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
mod app_settings;
mod commands;
mod constants;

use crate::app_settings::AppSettings;
use crate::constants::default_app_dir;

#[async_std::main]
async fn main() -> anyhow::Result<()> {
  std::fs::create_dir_all(default_app_dir())?;
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      crate::commands::test::my_custom_command,
      crate::commands::config::load_config,
      crate::commands::config::save_config
    ])
    .manage(AppSettings::load()?)
    .run(tauri::generate_context!())
    .expect("failed to run app");
  Ok(())
}
