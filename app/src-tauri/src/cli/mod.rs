#[cfg(feature = "lua")]
mod plan;
#[cfg(debug_assertions)]
mod playground;
mod rcon;
#[cfg(feature = "repl")]
mod repl;
#[cfg(feature = "lua")]
mod roll_seed;
mod start;
#[cfg(feature = "tui")]
mod tui;

use async_trait::async_trait;
use clap::{ArgMatches, Command};
use miette::Result;

#[async_trait]
pub trait ExecutableCommand {
  fn name(&self) -> &str;
  fn build_command(&self) -> Command<'static>;
  async fn run(&self, matches: &ArgMatches) -> Result<()>;
}

pub fn subcommands() -> Vec<Box<dyn ExecutableCommand>> {
  vec![
    rcon::build(),
    #[cfg(feature = "lua")]
    plan::build(),
    #[cfg(feature = "lua")]
    roll_seed::build(),
    #[cfg(debug_assertions)]
    playground::build(),
    #[cfg(feature = "tui")]
    tui::build(),
    #[cfg(feature = "repl")]
    repl::build(),
    start::build(),
  ]
}
