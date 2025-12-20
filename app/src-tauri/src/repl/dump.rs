use crate::context::Context;
use crate::repl::{Error, Subcommand};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::error;
use reedline_repl_rs::clap::{builder::PossibleValuesParser, Arg, ArgMatches, Command};
use reedline_repl_rs::clap::builder::PossibleValue;
use reedline_repl_rs::Repl;
use std::str::FromStr;
use strum::{EnumIter, EnumMessage, EnumString, IntoEnumIterator, IntoStaticStr};

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let command = DumpType::from_str(
    matches
      .get_one::<String>("type")
      .map(|s| s.as_str())
      .expect("Has default value"),
  )
  .into_diagnostic()?;
  let save_path = match matches
    .get_one::<String>("save")
    .map(|s| s.as_str())
    .expect("Has default value")
  {
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
      DumpType::EntityPrototypes => {
        if let Some(world) = instance_state.world.as_ref() {
          world.dump_entitiy_prototypes(save_path)?;
        } else {
          error!("no factorio world found??");
        }
      }
      DumpType::ItemPrototypes => {
        if let Some(world) = instance_state.world.as_ref() {
          world.dump_item_prototypes(save_path)?;
        } else {
          error!("no factorio world found??");
        }
      }
      DumpType::Recipes => {
        if let Some(world) = instance_state.world.as_ref() {
          world.dump_recipes(save_path)?;
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
  #[strum(message = "dump complete internal factorio world representation (very big)")]
  World,
  #[strum(message = "dump entity prototypes")]
  EntityPrototypes,
  #[strum(message = "dump item prototypes")]
  ItemPrototypes,
  #[strum(message = "dump recipes")]
  Recipes,
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "dump"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name())
        .about("dump information")
        .arg(
          Arg::new("type")
            .required(true)
            .value_parser(PossibleValuesParser::new(DumpType::iter().map(|action| {
              let message = action.get_message().unwrap();
              PossibleValue::new(Into::<&str>::into(action)).help(message)
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
