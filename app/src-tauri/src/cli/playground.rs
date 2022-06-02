use crate::cli::ExecutableCommand;
use async_trait::async_trait;
use clap::{ArgMatches, Command};
use miette::Result;

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
  fn name(&self) -> &str {
    "play"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name()).about("play")
  }

  async fn run(&self, _matches: &ArgMatches) -> Result<()> {
    println!("hello world");
    Ok(())
  }
}
