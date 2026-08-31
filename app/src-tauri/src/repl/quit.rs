use crate::context::Context;
use crate::repl::{Error, Subcommand};
use reedline_repl_rs::Repl;
use reedline_repl_rs::clap::{ArgMatches, Command};

#[allow(clippy::unused_async)]
async fn run(_matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let mut instance_state = context.instance_state.write().await;
  if let Some(instance_state) = instance_state.take() {
    instance_state.stop().expect("failed to stop");
  }
  std::process::exit(0);
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "quit"
  }

  fn build_command(
    &self,
    repl: Repl<Context, Error>,
  ) -> factorio_bot_core::miette::Result<Repl<Context, Error>> {
    Ok(repl.with_command_async(
      Command::new(self.name()).about("stop all running instances and quit"),
      |args, context| Box::pin(run(args, context)),
    ))
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
