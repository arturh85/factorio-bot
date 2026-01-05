use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use crate::scripting::run_script_file;
use crate::settings::load_app_settings;
use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::miette::Result;
use factorio_bot_core::paris::info;
use factorio_bot_core::parking_lot::RwLock;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::process::process_control::{
  FactorioInstance, FactorioParams, FactorioStartCondition,
};
use std::sync::Arc;

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "lua"
  }
  fn build_command(&self) -> Command {
    Command::new("lua")
            .about("Start Factorio and run a Lua script")
            .arg(
                Arg::new("script")
                    .help("Path to the Lua script to run (relative to scripts/ folder)")
                    .required(true)
                    .value_parser(value_parser!(String)),
            )
            .arg(
                Arg::new("clients")
                    .short('c')
                    .long("clients")
                    .default_value("1")
                    .value_parser(value_parser!(u8))
                    .help("number of clients to start"),
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
            .arg(
                Arg::new("connect")
                    .long("connect")
                    .action(ArgAction::SetTrue)
                    .help("Connect to already-running Factorio (fast iteration mode - world.* functions won't work, only rcon.*)"),
            )
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, _context: &mut Context) -> Result<()> {
  let app_settings = load_app_settings()?;
  let script_path = matches
    .get_one::<String>("script")
    .expect("required by clap");
  let clients = *matches.get_one::<u8>("clients").expect("defaulted by clap");
  let connect_mode = matches.get_flag("connect");
  let server_host = matches.get_one::<String>("server").cloned();

  if connect_mode {
    // Fast iteration mode: connect to already-running Factorio
    info!(
      "Connecting to running Factorio to run script: {}",
      script_path
    );

    let rcon_settings = RconSettings::new(
      app_settings.factorio.rcon_port,
      &app_settings.factorio.rcon_pass,
      server_host,
    );
    let rcon = FactorioRcon::new(&rcon_settings, Arc::new(RwLock::new(false)))
      .await
      .expect("failed to connect to RCON - is Factorio running?");

    // Create empty world for connect mode
    let world = Arc::new(FactorioWorld::new());
    let mut planner = Planner::new(world, Some(Arc::new(rcon)));

    let (stdout, stderr) = run_script_file(&mut planner, script_path, clients, false).await?;

    if !stdout.is_empty() {
      print!("{stdout}");
    }
    if !stderr.is_empty() {
      eprint!("{stderr}");
    }

    info!("Script completed");
  } else {
    // Full mode: start Factorio server + clients
    let write_logs = matches.get_flag("logs");
    let verbose = matches.get_flag("verbose");
    let seed = matches.get_one::<String>("seed").cloned();
    let map_exchange_string = matches.get_one::<String>("map").cloned();
    let recreate = matches.get_flag("new");

    info!("Starting Factorio to run script: {}", script_path);

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

    if let Some(world) = instance_state.world.as_ref() {
      let rcon = &instance_state.rcon;
      info!("Factorio started, running script...");

      let mut planner = Planner::new(world.clone(), Some(rcon.clone()));

      let (stdout, stderr) = run_script_file(&mut planner, script_path, clients, false).await?;

      if !stdout.is_empty() {
        print!("{stdout}");
      }
      if !stderr.is_empty() {
        eprint!("{stderr}");
      }

      info!("Script completed");

      // Clean up Factorio processes (clients first, then server)
      instance_state.stop().expect("failed to stop factorio");
    } else {
      factorio_bot_core::paris::error!("Failed to start Factorio (no world/rcon available)");
    }
  }

  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
