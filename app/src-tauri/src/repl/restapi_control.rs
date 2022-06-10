use crate::context::Context;
use crate::repl::{Error, Subcommand};
use factorio_bot_core::miette::Result;
use factorio_bot_core::paris::{error, info};
use factorio_bot_restapi::webserver;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command, PossibleValue};
use reedline_repl_rs::Repl;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "restapi"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name()).about("start/stop restapi").arg(
        Arg::new("action")
          .default_value("start")
          .possible_values(vec![
            PossibleValue::new("start").help("starts restapi"),
            PossibleValue::new("stop").help("stops restapi"),
          ])
          .help("either start or stop restapi server"),
      ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let action: &str = matches.value_of("action").expect("Has default value");
  match action {
    "start" => {
      let app_settings = context.app_settings.read().await;
      let instance_state = context.instance_state.clone();
      let webserver = webserver::start(app_settings.restapi.clone(), instance_state);
      let handle = tokio::task::spawn(webserver);
      let mut restapi_handle = context.restapi_handle.write().await;
      *restapi_handle = Some(handle);
    }
    "stop" => {
      let mut handle = context.restapi_handle.write().await;
      if handle.is_none() {
        error!("failed: not started");
        return Ok(None);
      }
      handle.take().expect("Already checked").abort();
      info!("successfully stopped");
    }
    _ => error!("invalid command, use one of start, stop"),
  };
  Ok(None)
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
