#![warn(clippy::all, clippy::pedantic)]

use clap::{App, Arg};

use factorio_bot_backend::factorio::planner::start_factorio_and_plan_graph;
use factorio_bot_backend::factorio::process_control::start_factorio;
use factorio_bot_backend::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_backend::factorio::roll_best_seed::{roll_seed, RollSeedLimit};
// use factorio_bot_backend::factorio::ws::FactorioWebSocketServer;
// use factorio_bot_backend::web::server::start_webserver;

// #[tokio::main(core_threads = 4, max_threads = 8)]
// #[actix_rt::main]
#[async_std::main]
async fn main() -> anyhow::Result<()> {
    color_eyre::install().unwrap();
    let matches = App::new("factorio-bot-rs")
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
                    Arg::with_name("open")
                        .long("open")
                        .short("o")
                        .help("open web interface with default browser"),
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

    let mut settings = config::Config::default();
    settings
        .merge(config::File::with_name("Settings"))?
        .merge(config::Environment::with_prefix("APP"))?;

    #[allow(clippy::redundant_closure_for_method_calls)]
    if let Some(matches) = matches.subcommand_matches("start") {
        let clients: u8 = matches.value_of("clients").unwrap().parse().unwrap();
        let write_logs: bool = matches.is_present("logs");
        let seed = matches.value_of("seed").map(|s| s.to_string());
        let map_exchange_string = matches.value_of("map").map(|s| s.to_string());
        let recreate = matches.is_present("new");
        let _open_browser = matches.is_present("open");
        let server_host = matches.value_of("server");
        // let websocket_server = FactorioWebSocketServer { listeners: vec![] }.start();
        let (world, _rcon) = start_factorio(
            &settings,
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

        if let Some(_world) = world {
            // start_webserver(rcon, websocket_server, open_browser, world).await;
        }
    } else if let Some(matches) = matches.subcommand_matches("rcon") {
        let command = matches.value_of("command").unwrap();
        let server_host = matches.value_of("server");
        let rcon_settings = RconSettings::new_from_config(&settings, server_host);
        let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
        rcon.send(command).await.unwrap();
    } else if let Some(matches) = matches.subcommand_matches("roll-seed") {
        if let Some((seed, score)) = roll_seed(
            settings,
            matches.value_of("map").expect("map required!").into(),
            match matches.value_of("rolls") {
                Some(s) => RollSeedLimit::Rolls(s.parse()?),
                None => RollSeedLimit::Seconds(matches.value_of("seconds").unwrap().parse()?),
            },
            matches.value_of("parallel").unwrap().parse()?,
            matches.value_of("name").unwrap().into(),
            matches.value_of("clients").unwrap().parse()?,
        )
        .await?
        {
            println!("Best Seed: {} with Score {}", seed, score);
        } else {
            eprintln!("no seed found");
        }
    } else if let Some(matches) = matches.subcommand_matches("plan") {
        let seed = matches.value_of("seed").map(|s| s.to_string());
        let name = matches.value_of("name").unwrap().to_string();
        let map_exchange_string = matches.value_of("map").map(|s| s.to_string());
        let bot_count = matches.value_of("clients").unwrap().parse().unwrap();
        let _graph =
            start_factorio_and_plan_graph(settings, map_exchange_string, seed, &name, bot_count)
                .await;
    } else {
        eprintln!("Missing required Sub Command!");
        std::process::exit(1);
    }
    Ok::<(), anyhow::Error>(())
}
