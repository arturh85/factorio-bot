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
use factorio_bot_core::miette::Result;
pub const APP_NAME: &str = env!("CARGO_BIN_NAME");
pub const APP_AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const APP_ABOUT: &str = env!("CARGO_PKG_DESCRIPTION");

#[allow(unreachable_code, unused_variables)]
#[tokio::main]
async fn main() -> Result<()> {
  let context = Context::new()?;

  #[cfg(feature = "cli")]
  {
    let app = cli::start(context.clone()).await?;
    if app.is_none() {
      // happens when subcommand successfully executes
      return Ok(());
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
    gui::start(context.clone())?;
  }
  #[cfg(not(feature = "gui"))]
  {
    #[cfg(feature = "repl")]
    {
      repl::start(context.clone()).await?;
    }
    #[cfg(all(not(feature = "cli"), not(feature = "repl")))]
    {
      return Err(factorio_bot_core::miette::miette!(
        "select at least one feature of cli, repl, gui"
      ));
    }
  }
  Ok(())
}
