use crate::context::Context;
use crate::repl::{Error, Subcommand};
use factorio_bot_core::paris::error;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let command = matches
    .value_of("rcon-command")
    .expect("Required arg validated by clap")
    .to_owned();
  let instance_state = context.instance_state.read().await;
  if instance_state.is_some() {
    let instance_state = context.instance_state.clone();
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = instance_state.as_ref() {
      let rcon = instance_state.rcon.clone();
      rcon.send(&command).await?;
    }
  } else {
    error!("failed: not started");
  }
  Ok(None)
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "rcon"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name())
        .about("send rcon command")
        .arg(Arg::new("rcon-command").required(true).index(1)),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
