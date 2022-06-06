#![warn(clippy::all, clippy::pedantic)]
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
#[allow(unused_imports)]
#[macro_use]
extern crate paris;

mod cli;
#[cfg(feature = "gui")]
mod commands;
mod constants;
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "repl")]
mod repl;
#[cfg(any(feature = "rhai", feature = "lua"))]
mod scripting;
mod settings;

use clap::Command;
use miette::{IntoDiagnostic, Result};

pub const APP_NAME: &str = "factorio-bot";
pub const APP_AUTHOR: &str = "Artur Hallmann <arturh@arturh.de>";
pub const APP_ABOUT: &str = "Bot for Factorio";

#[allow(unreachable_code, clippy::needless_return)]
#[tokio::main]
async fn main() -> Result<()> {
  color_eyre::install().unwrap(); // colored panics
  console_subscriber::init(); // needed for tokio console: https://github.com/tokio-rs/console
  std::fs::create_dir_all(constants::app_data_dir()).into_diagnostic()?;
  std::fs::create_dir_all(constants::app_workspace_path()).into_diagnostic()?;
  let mut app = Command::new(APP_NAME)
    .version(env!("CARGO_PKG_VERSION"))
    .author(APP_AUTHOR)
    .about(APP_ABOUT);
  let subcommands = cli::subcommands();
  for subcommand in &subcommands {
    app = app.subcommand(subcommand.build_command());
  }
  let matches = app.get_matches();
  for subcommand in &subcommands {
    if let Some(matches) = matches.subcommand_matches(&subcommand.name()) {
      subcommand.run(matches).await?;
      return Ok(());
    }
  }
  #[cfg(feature = "gui")]
  {
    return gui::start().await;
  }
  #[cfg(not(feature = "gui"))]
  {
    #[cfg(feature = "repl")]
    {
      return repl::start();
    }
    #[cfg(not(feature = "repl"))]
    {
      use miette::miette;
      info!("you can use -h/--help to list all possible commands");
      return Err(miette!("missing subcommand"));
    }
  }
}
