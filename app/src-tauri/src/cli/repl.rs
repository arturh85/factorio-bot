use async_trait::async_trait;
use clap::{ArgMatches, Command};
use miette::Result;

use crate::cli::ExecutableCommand;
use crate::repl;

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
  fn name(&self) -> &str {
    "repl"
  }

  fn build_command(&self) -> Command<'static> {
    Command::new(self.name()).about("repl")
  }

  async fn run(&self, _matches: &ArgMatches) -> Result<()> {
    repl::start()
  }
}
