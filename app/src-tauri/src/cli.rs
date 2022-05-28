use clap::{Arg, Command};
use factorio_bot_core::process::process_control::start_factorio;
use factorio_bot_core::settings::FACTORIO_SETTINGS_DEFAULT;

#[allow(clippy::module_name_repetitions, clippy::too_many_lines)]
pub async fn handle_cli() {
  let matches = Command::new("factorio-bot")
    .version(env!("CARGO_PKG_VERSION"))
    .author("Artur Hallmann <arturh@arturh.de>")
    .about("Bot for Factorio")
    .subcommand(
      Command::new("rcon")
        .arg(Arg::new("command").required(true).last(true))
        .arg(
          Arg::new("server")
            .short('s')
            .long("server")
            .value_name("server")
            .required(false)
            .help("connect to server instead of starting a server"),
        )
        .about("send given rcon command"),
    )
    .subcommand(
      Command::new("roll-seed")
        .arg(
          Arg::new("map")
            .long("map")
            .value_name("map")
            .required(true)
            .help("use given map exchange string"),
        )
        .arg(
          Arg::new("seconds")
            .short('s')
            .long("seconds")
            .value_name("seconds")
            .default_value("360")
            .help("limits how long to roll seeds"),
        )
        .arg(
          Arg::new("parallel")
            .short('p')
            .long("parallel")
            .value_name("parallel")
            .default_value("4")
            .help("how many rolling servers to run in parallel"),
        )
        .arg(
          Arg::new("name")
            .long("name")
            .value_name("name")
            .required(true)
            .help("name of plan without .lua extension"),
        )
        .arg(
          Arg::new("rolls")
            .short('r')
            .long("rolls")
            .value_name("rolls")
            .help("how many seeds to roll"),
        )
        .arg(
          Arg::new("clients")
            .short('c')
            .long("clients")
            .default_value("1")
            .help("number of clients to plan for"),
        )
        .about("roll good seed for given map-exchange-string based on heuristics"),
    )
    .subcommand(
      Command::new("plan")
        .arg(
          Arg::new("map")
            .long("map")
            .value_name("map")
            .required(true)
            .help("use given map exchange string"),
        )
        .arg(
          Arg::new("seed")
            .long("seed")
            .value_name("seed")
            .required(false)
            .help("use given seed to recreate level"),
        )
        .arg(
          Arg::new("name")
            .long("name")
            .value_name("name")
            .required(true)
            .help("name of plan without .lua extension"),
        )
        .arg(
          Arg::new("clients")
            .short('c')
            .long("clients")
            .default_value("1")
            .help("number of clients to plan for"),
        )
        .about("plan graph"),
    )
    .subcommand(
      Command::new("start")
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
      &FACTORIO_SETTINGS_DEFAULT,
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
    // } else if let Some(_matches) = matches.subcommand_matches("rcon") {
    // let command = matches.value_of("command").unwrap();
    // let server_host = matches.value_of("server");
    // let rcon_settings = RconSettings::new_from_config(&settings, server_host);
    // let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
    // rcon.send(command).await.unwrap();
    // } else if let Some(_matches) = matches.subcommand_matches("roll-seed") {
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
    // } else if let Some(_matches) = matches.subcommand_matches("plan") {
    // let seed = matches.value_of("seed").map(|s| s.to_string());
    // let name = matches.value_of("name").unwrap().to_string();
    // let map_exchange_string = matches.value_of("map").map(|s| s.to_string());
    // let bot_count = matches.value_of("clients").unwrap().parse().unwrap();
    // let _graph =
    //   start_factorio_and_plan_graph(settings, map_exchange_string, seed, &name, bot_count).await;
  }
}
