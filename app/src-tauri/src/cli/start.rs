use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use crate::settings::load_app_settings;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::miette::Result;
use factorio_bot_core::process::process_control::{FactorioInstance, FactorioParams};

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "start"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name())
      .about("start the factorio server and clients + web server")
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .help("number of clients to start in addition to the server"),
      )
      .arg(
        Arg::new("server")
          .short('s')
          .long("server")
          .value_name("server")
          .required(false)
          .help("connect to server instead of starting a server"),
      )
      .arg(
        Arg::new("seed")
          .long("seed")
          .value_name("seed")
          .required(false)
          .help("use given seed to recreate level"),
      )
      .arg(
        Arg::new("map")
          .long("map")
          .value_name("map")
          .required(false)
          .help("use given map exchange string"),
      )
      .arg(
        Arg::new("new")
          .long("new")
          .short('n')
          .help("recreate level by deleting server map if exists"),
      )
      .arg(
        Arg::new("logs")
          .short('l')
          .long("logs")
          .help("enabled writing server & client logs to workspace"),
      )
      .about("start given number of clients after server start")
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: ArgMatches, _context: &mut Context) -> Result<()> {
  let app_settings = load_app_settings()?;
  let clients: u8 = matches.value_of("clients").unwrap().parse().unwrap();
  let write_logs: bool = matches.is_present("logs");
  let seed = matches
    .value_of("seed")
    .map(std::string::ToString::to_string);
  let map_exchange_string = matches
    .value_of("map")
    .map(std::string::ToString::to_string);
  let recreate = matches.is_present("new");
  let server_host = matches.value_of("server");
  // let websocket_server = FactorioWebSocketServer { listeners: vec![] }.start();

  let params = FactorioParams {
    seed,
    server_host: server_host.map(std::borrow::ToOwned::to_owned),
    client_count: clients,
    recreate,
    write_logs,
    map_exchange_string,
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
    // start_webserver(rcon, websocket_server, open_browser, world).await;
  }
  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
