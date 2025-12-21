#![warn(clippy::all, clippy::pedantic)]
// Removed because of CLI/REPL features
// // Remove console window opening on windows
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "console"
)]
#[cfg(feature = "cli")]
mod cli;
mod context;
#[cfg(feature = "gui")]
mod gui;
mod paths;
#[cfg(feature = "repl")]
mod repl;
#[cfg(feature = "lua")]
mod scripting;
mod settings;

use context::Context;
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const APP_ABOUT: &str = env!("CARGO_PKG_DESCRIPTION");

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let context = Context::new().expect("failed to create context");

  #[cfg(feature = "cli")]
  {
    let app = tauri::async_runtime::block_on(async {
      cli::start(context.clone()).await
    }).expect("failed to start cli");

    if app.is_none() {
      return ();
    }
    #[cfg(all(not(feature = "gui"), not(feature = "repl")))]
    {
      app
        .expect("checked before")
        .print_help()
        .expect("failed to print_help");
      return Err(factorio_bot_core::miette::miette!("missing subcommand"));
    }
  }
  #[cfg(feature = "gui")]
  {
    gui::start(context.clone()).expect("failed to start gui");
  }
  #[cfg(not(feature = "gui"))]
  {
    #[cfg(feature = "repl")]
    {
      tauri::async_runtime::block_on(async {
        repl::start(context.clone()).await
      })?;
    }
    #[cfg(all(not(feature = "cli"), not(feature = "repl")))]
    {
      return Err(factorio_bot_core::miette::miette!(
        "select at least one feature of cli, repl, gui"
      ));
    }
  }
}
