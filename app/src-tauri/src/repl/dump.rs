use crate::context::Context;
use crate::repl::{Error, Subcommand};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::error;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command, PossibleValue};
use reedline_repl_rs::Repl;
use std::str::FromStr;
use strum::EnumMessage;
use strum::IntoEnumIterator;

#[derive(EnumString, EnumMessage, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
enum ToDump {
  #[strum(message = "dump internal factorio world representation")]
  World,
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "dump"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name())
        .about("dump information")
        .arg(
          Arg::new("type")
            .default_value(ToDump::World.into())
            .possible_values(ToDump::iter().map(|action| {
              let message = action.get_message().unwrap();
              PossibleValue::new(action.into()).help(message)
            }))
            .help("type of information to dump"),
        )
        .arg(
          Arg::new("save")
            .long("save")
            .required(false)
            .help("path to save at"),
        ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

#[allow(clippy::unused_async)]
async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let command =
    ToDump::from_str(matches.value_of("type").expect("Has default value")).into_diagnostic()?;
  let save_path = matches.value_of("save");

  let instance_state = context.instance_state.read().await;
  if let Some(instance_state) = instance_state.as_ref() {
    match command {
      ToDump::World => {
        if let Some(world) = instance_state.world.as_ref() {
          world.dump(save_path)?;
        } else {
          error!("no factorio world found??");
        }
      }
    }
  } else {
    error!("no factorio instance running");
  }
  Ok(None)
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
