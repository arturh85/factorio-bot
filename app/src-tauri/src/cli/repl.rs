use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use crate::repl;
use clap::{ArgMatches, Command};
use factorio_bot_core::miette::Result;

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "repl"
  }
  fn build_command(&self) -> Command {
    Command::new("repl").about("repl")
  }
  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(_matches: &ArgMatches, context: &mut Context) -> Result<()> {
  repl::start(context.clone()).await
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
