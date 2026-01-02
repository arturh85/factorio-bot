use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use crate::settings::load_app_settings;
use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use factorio_bot_core::miette::Result;
use factorio_bot_core::paris::info;
use factorio_bot_core::process::process_control::{
  FactorioInstance, FactorioParams, FactorioStartCondition,
};

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "start"
  }
  fn build_command(&self) -> Command {
    Command::new("start")
      .about("start the factorio server and clients + web server")
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .value_parser(value_parser!(u8))
          .help("number of clients to start in addition to the server"),
      )
      .arg(
        Arg::new("server")
          .short('s')
          .long("server")
          .value_name("server")
          .required(false)
          .value_parser(value_parser!(String))
          .help("connect to server instead of starting a server"),
      )
      .arg(
        Arg::new("seed")
          .long("seed")
          .value_name("seed")
          .required(false)
          .value_parser(value_parser!(String))
          .help("use given seed to recreate level"),
      )
      .arg(
        Arg::new("map")
          .long("map")
          .value_name("map")
          .required(false)
          .value_parser(value_parser!(String))
          .help("use given map exchange string"),
      )
      .arg(
        Arg::new("new")
          .long("new")
          .short('n')
          .action(ArgAction::SetTrue)
          .help("recreate level by deleting server map if exists"),
      )
      .arg(
        Arg::new("logs")
          .short('l')
          .long("logs")
          .action(ArgAction::SetTrue)
          .help("enabled writing server & client logs to workspace"),
      )
      .arg(
        Arg::new("verbose")
          .short('v')
          .long("verbose")
          .action(ArgAction::SetTrue)
          .help("Log server output to console"),
      )
      .about("start given number of clients after server start")
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, _context: &mut Context) -> Result<()> {
  let app_settings = load_app_settings()?;
  let clients = *matches.get_one::<u8>("clients").expect("defaulted by clap");
  let write_logs = matches.get_flag("logs");
  let verbose = matches.get_flag("verbose");
  let seed = matches.get_one::<String>("seed").cloned();
  let map_exchange_string = matches.get_one::<String>("map").cloned();
  let recreate = matches.get_flag("new");
  let server_host = matches.get_one::<String>("server").cloned();
  // let websocket_server = FactorioWebSocketServer { listeners: vec![] }.start();

  let params = FactorioParams {
    seed,
    server_host,
    client_count: clients,
    recreate,
    write_logs,
    map_exchange_string,
    wait_until: FactorioStartCondition::DiscoveryComplete,
    silent: !verbose,
    ..FactorioParams::default()
  };
  let instance_state = FactorioInstance::start(&app_settings.factorio, params)
    .await
    .expect("failed to start factorio");

  #[cfg(feature = "repl")]
  {
    // crate::repl::start().await?;
  }

  // FIXME: watch children die?

  if let Some(_world) = &instance_state.world {
    info!("started!");
    // start_webserver(rcon, websocket_server, open_browser, world).await;
  }
  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
