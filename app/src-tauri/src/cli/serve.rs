use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{value_parser, Arg, ArgMatches, Command};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use std::net::SocketAddr;

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "serve"
  }
  fn build_command(&self) -> Command {
    Command::new("serve")
      .arg(
        Arg::new("bind")
          .short('b')
          .long("bind")
          .value_name("ADDR")
          .required(false)
          .value_parser(value_parser!(String))
          .help("address to bind, defaults to 127.0.0.1 on the configured port"),
      )
      .about("serve the web UI and HTTP API")
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, context: &mut Context) -> Result<()> {
  let bind: SocketAddr = if let Some(raw) = matches.get_one::<String>("bind") {
    raw.parse().into_diagnostic()?
  } else {
    let port = context.app_settings.read().await.restapi.port;
    SocketAddr::from(([127, 0, 0, 1], u16::try_from(port).into_diagnostic()?))
  };

  let shutdown = async {
    let _ = tokio::signal::ctrl_c().await;
    log::info!("shutdown signal received");
  };

  factorio_bot_server::webserver::start_with_shutdown(
    context.app_settings.clone(),
    context.instance_state.clone(),
    bind,
    shutdown,
  )
  .await
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
