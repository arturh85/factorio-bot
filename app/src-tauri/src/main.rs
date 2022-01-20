#![warn(clippy::all, clippy::pedantic)]
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
#[macro_use]
extern crate paris;

mod commands;
mod constants;

use async_std::sync::{Arc, RwLock};
use async_std::task::JoinHandle;
use factorio_bot::cli::handle_cli;
use factorio_bot_core::process::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use miette::{Result, IntoDiagnostic};
use std::borrow::Cow;

fn app_settings() -> Result<AppSettings> {
  let mut app_settings = AppSettings::load(constants::app_settings_path())?;
  if app_settings.workspace_path == "" {
    let s: String = constants::app_workspace_path().to_str().unwrap().into();
    app_settings.workspace_path = Cow::from(s);
  }
  Ok(app_settings)
}

#[async_std::main]
async fn main() -> Result<()> {
  color_eyre::install().unwrap();
  handle_cli().await;
  std::fs::create_dir_all(constants::app_data_dir())
    .into_diagnostic()?;
  std::fs::create_dir_all(constants::app_workspace_path())
    .into_diagnostic()?;
  info!("factorio-bot started");
  let instance_state: Option<InstanceState> = None;
  let restapi_handle: Option<JoinHandle<Result<()>>> = None;

  #[allow(clippy::items_after_statements)]
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      crate::commands::is_restapi_started,
      crate::commands::is_instance_started,
      crate::commands::is_port_available,
      crate::commands::load_script,
      crate::commands::load_scripts_in_directory,
      crate::commands::execute_rcon,
      crate::commands::execute_script,
      crate::commands::update_settings,
      crate::commands::load_settings,
      crate::commands::save_settings,
      crate::commands::start_instances,
      crate::commands::stop_instances,
      crate::commands::start_restapi,
      crate::commands::stop_restapi,
      crate::commands::maximize_window,
      crate::commands::file_exists,
      crate::commands::open_in_browser,
    ])
    .manage(Arc::new(RwLock::new(app_settings()?)))
    .manage(Arc::new(RwLock::new(instance_state)))
    .manage(RwLock::new(restapi_handle))
    .run(tauri::generate_context!())
    .expect("failed to run app");
  Ok(())
}
