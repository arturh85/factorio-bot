use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{ArgMatches, Command};
use factorio_bot_core::miette::Result;
use factorio_bot_core::paris::info;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "play"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name()).about("play")
  }
  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

#[allow(clippy::unused_async)]
async fn run(_matches: ArgMatches, _context: &mut Context) -> Result<()> {
  info!("hello world");
  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
