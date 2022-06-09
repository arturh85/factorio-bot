use crate::context::Context;
use crate::repl::{Error, Subcommand};
use reedline_repl_rs::clap::{ArgMatches, Command};
use reedline_repl_rs::Repl;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "gui"
  }

  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name()).about("switch to gui"),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

#[allow(clippy::unused_async)]
async fn run(_matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  crate::gui::start(context.clone())?;
  Ok(None)
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
