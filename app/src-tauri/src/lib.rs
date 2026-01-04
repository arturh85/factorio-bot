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
#[allow(clippy::missing_panics_doc)]
pub fn run() {
  let context = Context::new().expect("failed to create context");

  #[cfg(feature = "cli")]
  {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let app = rt
      .block_on(async { cli::start(context.clone()).await })
      .expect("failed to start cli");

    // If no subcommand was run, app is Some and we should show help (unless GUI/REPL will start)
    #[cfg(all(not(feature = "gui"), not(feature = "repl")))]
    {
      if app.is_none() {
        return;
      }
      app
        .expect("checked before")
        .print_help()
        .expect("failed to print_help");
      return;
    }
    // With GUI or REPL features, continue even after a subcommand runs
    #[cfg(any(feature = "gui", feature = "repl"))]
    {
      if let Some(mut app) = app {
        // No subcommand was run, show help and let GUI/REPL take over
        app.print_help().expect("failed to print_help");
      }
      // If app is None, a subcommand ran - continue to GUI/REPL
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
      let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
      rt.block_on(async { repl::start(context.clone()).await })
        .expect("repl failed");
    }
    #[cfg(all(not(feature = "cli"), not(feature = "repl")))]
    {
      panic!("select at least one feature of cli, repl, gui");
    }
  }
}
