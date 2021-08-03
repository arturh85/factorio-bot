#![warn(clippy::all, clippy::pedantic)]
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
#[macro_use]
extern crate paris;

mod commands;
mod constants;

use async_std::sync::RwLock;
use clap::{App, Arg};
use factorio_bot_backend::factorio::process_control::{start_factorio, InstanceState};
use factorio_bot_backend::settings::AppSettings;
use factorio_bot_backend::settings::APP_SETTINGS_DEFAULT;
use std::borrow::Cow;

fn app_settings() -> anyhow::Result<AppSettings> {
  let mut app_settings = AppSettings::load(constants::app_settings_path())?;
  if app_settings.workspace_path == "" {
    let s: String = constants::app_workspace_path().to_str().unwrap().into();
    app_settings.workspace_path = Cow::from(s);
  }
  Ok(app_settings)
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
  color_eyre::install().unwrap();
  info!("started");
  std::fs::create_dir_all(constants::default_app_dir())?;
  std::fs::create_dir_all(constants::app_workspace_path())?;
  let matches = App::new("factorio-bot")
    .version(env!("CARGO_PKG_VERSION"))
    .author("Artur Hallmann <arturh@arturh.de>")
    .about("Bot for Factorio")
    .subcommand(
      App::new("rcon")
        .arg(Arg::with_name("command").required(true).last(true))
        .arg(
          Arg::with_name("server")
            .short("server")
            .long("server")
            .value_name("server")
            .required(false)
            .help("connect to server instead of starting a server"),
        )
        .about("send given rcon command"),
    )
    .subcommand(
      App::new("roll-seed")
        .arg(
          Arg::with_name("map")
            .long("map")
            .value_name("map")
            .required(true)
            .help("use given map exchange string"),
        )
        .arg(
          Arg::with_name("seconds")
            .short("s")
            .long("seconds")
            .value_name("seconds")
            .default_value("360")
            .help("limits how long to roll seeds"),
        )
        .arg(
          Arg::with_name("parallel")
            .short("p")
            .long("parallel")
            .value_name("parallel")
            .default_value("4")
            .help("how many rolling servers to run in parallel"),
        )
        .arg(
          Arg::with_name("name")
            .long("name")
            .value_name("name")
            .required(true)
            .help("name of plan without .lua extension"),
        )
        .arg(
          Arg::with_name("rolls")
            .short("r")
            .long("rolls")
            .value_name("rolls")
            .help("how many seeds to roll"),
        )
        .arg(
          Arg::with_name("clients")
            .short("c")
            .long("clients")
            .default_value("1")
            .help("number of clients to plan for"),
        )
        .about("roll good seed for given map-exchange-string based on heuristics"),
    )
    .subcommand(
      App::new("plan")
        .arg(
          Arg::with_name("map")
            .long("map")
            .value_name("map")
            .required(true)
            .help("use given map exchange string"),
        )
        .arg(
          Arg::with_name("seed")
            .long("seed")
            .value_name("seed")
            .required(false)
            .help("use given seed to recreate level"),
        )
        .arg(
          Arg::with_name("name")
            .long("name")
            .value_name("name")
            .required(true)
            .help("name of plan without .lua extension"),
        )
        .arg(
          Arg::with_name("clients")
            .short("c")
            .long("clients")
            .default_value("1")
            .help("number of clients to plan for"),
        )
        .about("plan graph"),
    )
    .subcommand(
      App::new("start")
        .about("start the factorio server and clients + web server")
        .arg(
          Arg::with_name("clients")
            .short("c")
            .long("clients")
            .default_value("1")
            .help("number of clients to start in addition to the server"),
        )
        .arg(
          Arg::with_name("server")
            .short("server")
            .long("server")
            .value_name("server")
            .required(false)
            .help("connect to server instead of starting a server"),
        )
        .arg(
          Arg::with_name("seed")
            .long("seed")
            .value_name("seed")
            .required(false)
            .help("use given seed to recreate level"),
        )
        .arg(
          Arg::with_name("map")
            .long("map")
            .value_name("map")
            .required(false)
            .help("use given map exchange string"),
        )
        .arg(
          Arg::with_name("new")
            .long("new")
            .short("n")
            .help("recreate level by deleting server map if exists"),
        )
        .arg(
          Arg::with_name("logs")
            .short("l")
            .long("logs")
            .help("enabled writing server & client logs to workspace"),
        )
        .about("start given number of clients after server start"),
    )
    .get_matches();

  #[allow(clippy::redundant_closure_for_method_calls)]
  if let Some(matches) = matches.subcommand_matches("start") {
    let clients: u8 = matches.value_of("clients").unwrap().parse().unwrap();
    let write_logs: bool = matches.is_present("logs");
    let seed = matches.value_of("seed").map(|s| s.to_string());
    let map_exchange_string = matches.value_of("map").map(|s| s.to_string());
    let recreate = matches.is_present("new");
    let server_host = matches.value_of("server");
    // let websocket_server = FactorioWebSocketServer { listeners: vec![] }.start();

    let instance_state = start_factorio(
      &APP_SETTINGS_DEFAULT,
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

    // FIXME: watch children die?

    if let Some(_world) = &instance_state.world {
      // start_webserver(rcon, websocket_server, open_browser, world).await;
    }
  } else if let Some(_matches) = matches.subcommand_matches("rcon") {
    // let command = matches.value_of("command").unwrap();
    // let server_host = matches.value_of("server");
    // let rcon_settings = RconSettings::new_from_config(&settings, server_host);
    // let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
    // rcon.send(command).await.unwrap();
  } else if let Some(_matches) = matches.subcommand_matches("roll-seed") {
    // if let Some((seed, score)) = roll_seed(
    //   settings,
    //   matches.value_of("map").expect("map required!").into(),
    //   match matches.value_of("rolls") {
    //     Some(s) => RollSeedLimit::Rolls(s.parse()?),
    //     None => RollSeedLimit::Seconds(matches.value_of("seconds").unwrap().parse()?),
    //   },
    //   matches.value_of("parallel").unwrap().parse()?,
    //   matches.value_of("name").unwrap().into(),
    //   matches.value_of("clients").unwrap().parse()?,
    // )
    // .await?
    // {
    //   println!("Best Seed: {} with Score {}", seed, score);
    // } else {
    //   eprintln!("no seed found");
    // }
  } else if let Some(_matches) = matches.subcommand_matches("plan") {
    // let seed = matches.value_of("seed").map(|s| s.to_string());
    // let name = matches.value_of("name").unwrap().to_string();
    // let map_exchange_string = matches.value_of("map").map(|s| s.to_string());
    // let bot_count = matches.value_of("clients").unwrap().parse().unwrap();
    // let _graph =
    //   start_factorio_and_plan_graph(settings, map_exchange_string, seed, &name, bot_count).await;
  }
  let instance_state: Option<InstanceState> = None;
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      crate::commands::my_custom_command,
      crate::commands::update_settings,
      crate::commands::load_settings,
      crate::commands::save_settings,
      crate::commands::start_instances,
      crate::commands::stop_instances,
      crate::commands::maximize_window,
    ])
    .manage(RwLock::new(app_settings()?))
    .manage(RwLock::new(instance_state))
    .run(tauri::generate_context!())
    .expect("failed to run app");
  Ok(())
}
