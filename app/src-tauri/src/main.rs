#![warn(clippy::all, clippy::pedantic)]
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
#[macro_use]
extern crate paris;

mod cli;
#[cfg(feature = "gui")]
mod commands;
mod constants;
mod settings;

use clap::Command;
use miette::{IntoDiagnostic, Result};

#[cfg(feature = "gui")]
#[allow(clippy::items_after_statements)]
async fn start_gui() -> Result<()> {
  use crate::settings::load_app_settings;
  use factorio_bot_core::process::process_control::InstanceState;
  use std::sync::Arc;
  use tokio::sync::RwLock;
  use tokio::task::JoinHandle;

  let instance_state: Option<InstanceState> = None;
  let restapi_handle: Option<JoinHandle<Result<()>>> = None;
  let app_settings = load_app_settings()?;
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      crate::commands::is_restapi_started,
      crate::commands::is_instance_started,
      crate::commands::is_port_available,
      crate::commands::load_script,
      crate::commands::load_scripts_in_directory,
      crate::commands::execute_rcon,
      crate::commands::execute_script,
      crate::commands::execute_code,
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
    .manage(Arc::new(RwLock::new(app_settings)))
    .manage(Arc::new(RwLock::new(instance_state)))
    .manage(RwLock::new(restapi_handle))
    .run(tauri::generate_context!())
    .into_diagnostic()
}

#[tokio::main]
async fn main() -> Result<()> {
  color_eyre::install().unwrap();
  console_subscriber::init();
  std::fs::create_dir_all(constants::app_data_dir()).into_diagnostic()?;
  std::fs::create_dir_all(constants::app_workspace_path()).into_diagnostic()?;
  let mut command = Command::new("factorio-bot")
    .version(env!("CARGO_PKG_VERSION"))
    .author("Artur Hallmann <arturh@arturh.de>")
    .about("Bot for Factorio");
  let subcommands = cli::subcommands();
  for subcommand in &subcommands {
    command = command.subcommand(subcommand.build_command());
  }
  let matches = command.get_matches();
  for subcommand in subcommands {
    if let Some(matches) = matches.subcommand_matches(&subcommand.name()) {
      subcommand.run(matches).await?;
      return Ok(());
    }
  }
  #[cfg(feature = "gui")]
  {
    start_gui().await.expect("failed to start gui");
    Ok(())
  }
  #[cfg(not(feature = "gui"))]
  {
    info!("you can use -h/--help to list all possible commands");
    return Err(miette::Error::msg("missing subcommand"));
  }
}
