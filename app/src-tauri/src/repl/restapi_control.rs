use crate::context::Context;
use crate::repl::{Error, Subcommand};
use clap::builder::PossibleValuesParser;
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::{error, info};
use factorio_bot_restapi::webserver;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command, PossibleValue};
use reedline_repl_rs::Repl;
use std::str::FromStr;
use strum::{EnumIter, EnumMessage, EnumString, IntoEnumIterator, IntoStaticStr};

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let action =
    Action::from_str(matches.value_of("action").expect("Has default value")).into_diagnostic()?;
  match action {
    Action::Start => {
      let app_settings = context.app_settings.read().await;
      let instance_state = context.instance_state.clone();
      let webserver = webserver::start(app_settings.restapi.clone(), instance_state);
      let handle = tokio::task::spawn(webserver);
      let mut restapi_handle = context.restapi_handle.write().await;
      *restapi_handle = Some(handle);
    }
    Action::Stop => {
      let mut handle = context.restapi_handle.write().await;
      if handle.is_none() {
        error!("failed: not started");
        return Ok(None);
      }
      handle.take().expect("Already checked").abort();
      info!("successfully stopped");
    }
  };
  Ok(None)
}

#[derive(EnumString, EnumMessage, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
enum Action {
  #[strum(message = "starts restapi")]
  Start,
  #[strum(message = "stops restapi")]
  Stop,
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "restapi"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name()).about("start/stop restapi").arg(
        Arg::new("action")
          .default_value(Action::Start.into())
          .value_parser(PossibleValuesParser::new(Action::iter().map(|action| {
            let message = action.get_message().unwrap();
            PossibleValue::new(action.into()).help(message)
          })))
          .help("either start or stop restapi server"),
      ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
