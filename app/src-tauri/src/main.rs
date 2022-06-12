#![warn(clippy::all, clippy::pedantic)]
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
#[cfg(feature = "cli")]
mod cli;
mod context;
#[cfg(feature = "gui")]
mod gui;
mod paths;
#[cfg(feature = "repl")]
mod repl;
#[cfg(any(feature = "rhai", feature = "lua", feature = "rune"))]
mod scripting;
mod settings;
#[allow(unused_imports)]
#[macro_use]
extern crate strum_macros;

use context::Context;
use factorio_bot_core::miette::Result;

pub const APP_NAME: &str = "factorio-bot";
pub const APP_AUTHOR: &str = "Artur Hallmann <arturh@arturh.de>";
pub const APP_ABOUT: &str = "Bot for Factorio";

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
