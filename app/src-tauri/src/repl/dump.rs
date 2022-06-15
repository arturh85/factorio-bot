use crate::context::Context;
use crate::repl::{Error, Subcommand};
use clap::builder::PossibleValuesParser;
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::error;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command, PossibleValue};
use reedline_repl_rs::Repl;
use std::str::FromStr;
use strum::{EnumIter, EnumMessage, EnumString, IntoEnumIterator, IntoStaticStr};

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let command =
    DumpType::from_str(matches.value_of("type").expect("Has default value")).into_diagnostic()?;
  let save_path = match matches.value_of("save").expect("Has default value") {
    "" => None,
    save_path => Some(save_path),
  };

  let instance_state = context.instance_state.read().await;
  if let Some(instance_state) = instance_state.as_ref() {
    match command {
      DumpType::World => {
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

#[derive(EnumString, EnumMessage, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
enum DumpType {
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
            .default_value(DumpType::World.into())
            .value_parser(PossibleValuesParser::new(DumpType::iter().map(|action| {
              let message = action.get_message().unwrap();
              PossibleValue::new(action.into()).help(message)
            })))
            .help("type of information to dump"),
        )
        .arg(
          Arg::new("save")
            .default_value("")
            .long("save")
            .required(false)
            .help("path to save at"),
        ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
