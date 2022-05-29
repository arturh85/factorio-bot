mod plan;
mod rcon;
mod roll_seed;
mod start;

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
    // plan::build(),
    // rcon::build(),
    // roll_seed::build(),
    start::build(),
  ]
}
