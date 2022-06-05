use crate::cli::ExecutableCommand;
use crate::settings::load_app_settings;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::process::process_control::start_factorio;
use miette::Result;

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
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

  async fn run(&self, matches: &ArgMatches) -> Result<()> {
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

    let instance_state = start_factorio(
      &app_settings.factorio,
      server_host,
      clients,
      recreate,
      map_exchange_string,
      seed,
      // Some(websocket_server.clone()),
      write_logs,
      false,
    )
    .await
    .expect("failed to start factorio");

    #[cfg(feature = "repl")]
    {
      crate::repl::start().unwrap();
    }

    // FIXME: watch children die?

    if let Some(_world) = &instance_state.world {
      // start_webserver(rcon, websocket_server, open_browser, world).await;
    }
    Ok(())
  }
}
