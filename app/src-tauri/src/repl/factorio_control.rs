use crate::context::Context;
use crate::repl::{Error, Subcommand};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::{error, info};
use factorio_bot_core::process::process_control::{
  FactorioInstance, FactorioParams, FactorioStartCondition,
};
use reedline_repl_rs::clap::builder::PossibleValue;
use reedline_repl_rs::clap::{builder::PossibleValuesParser, Arg, ArgAction, ArgMatches, Command};
use reedline_repl_rs::crossterm::event::{KeyCode, KeyModifiers};
use reedline_repl_rs::reedline::ReedlineEvent;
use reedline_repl_rs::Repl;
use std::str::FromStr;
use strum::{EnumIter, EnumMessage, EnumString, IntoEnumIterator, IntoStaticStr};

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let action = Action::from_str(
    matches
      .get_one::<String>("action")
      .map(std::string::String::as_str)
      .expect("Has default value"),
  )
  .into_diagnostic()?;
  match action {
    Action::Start => {
      let app_settings = context.app_settings.read().await;
      let client_count: u8 = match matches
        .get_one::<String>("clients")
        .map(std::string::String::as_str)
        .expect("Has default value")
      {
        "" => app_settings.factorio.client_count,
        clients => clients.parse().into_diagnostic()?,
      };
      let write_logs: bool = matches.get_flag("logs");
      let verbose: bool = matches.get_flag("verbose");
      let seed = config_fallback(
        matches
          .get_one::<String>("seed")
          .map(std::string::String::as_str),
        &app_settings.factorio.seed,
      );
      let map_exchange_string = config_fallback(
        matches
          .get_one::<String>("map")
          .map(std::string::String::as_str),
        &app_settings.factorio.map_exchange_string,
      );
      let wait_until_finished = matches.get_flag("wait_until_finished");
      let recreate = matches.get_flag("new");
      drop(app_settings);
      subcommand_start(
        context,
        client_count,
        write_logs,
        verbose,
        seed,
        map_exchange_string,
        wait_until_finished,
        recreate,
      )
      .await?
    }
    Action::Status => subcommand_status(context).await?,
    Action::ToggleVerbose => subcommand_toggle_verbose(context).await?,
    Action::Add => subcommand_add(context).await?,
    Action::Stop => subcommand_stop(context).await?,
  };
  Ok(None)
}

#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
async fn subcommand_start(
  context: &mut Context,
  client_count: u8,
  write_logs: bool,
  verbose: bool,
  seed: Option<String>,
  map_exchange_string: Option<String>,
  wait_until_finished: bool,
  recreate: bool,
) -> Result<Option<String>, Error> {
  {
    let instance_state = context.instance_state.read().await;
    if instance_state.is_some() {
      error!("failed: already started");
      return Ok(None);
    }
  }
  print_hint();
  let app_settings = context.app_settings.read().await;
  let params = FactorioParams {
    client_count,
    recreate,
    write_logs,
    seed,
    map_exchange_string,
    silent: !verbose,
    wait_until: if wait_until_finished {
      FactorioStartCondition::DiscoveryComplete
    } else {
      FactorioStartCondition::Initialized
    },
    ..FactorioParams::default()
  };

  match FactorioInstance::start(&app_settings.factorio, params).await {
    Ok(new_instance_state) => {
      let mut instance_state = context.instance_state.write().await;
      *instance_state = Some(new_instance_state);
      drop(instance_state);
    }
    Err(err) => {
      error!("failed to start factorio: {:?}", err);
    }
  }
  // repeat hint because beginning of output might not be
  // visible in verbose mode any more
  if verbose {
    print_hint();
  }
  Ok(None)
}

async fn subcommand_stop(context: &mut Context) -> Result<Option<String>, Error> {
  let mut instance_state = context.instance_state.write().await;
  if instance_state.is_none() {
    error!("failed: not started");
    return Ok(None);
  }
  instance_state.take().expect("Already checked").stop()?;
  info!("successfully stopped");
  Ok(None)
}

async fn subcommand_status(context: &mut Context) -> Result<Option<String>, Error> {
  let instance_state = context.instance_state.read().await;
  if let Some(instance_state) = instance_state.as_ref() {
    info!(
      "started {} with {} clients @ Port {} with RCON {}",
      if *instance_state.silent.read() {
        "silently"
      } else {
        "verbosely"
      },
      instance_state.client_count,
      instance_state.server_port.unwrap_or(0),
      instance_state.rcon_port
    );
  } else {
    info!("factorio not started");
    return Ok(None);
  }
  Ok(None)
}

async fn subcommand_add(context: &mut Context) -> Result<Option<String>, Error> {
  let instance_state = context.instance_state.write().await;
  if instance_state.is_none() {
    error!("failed: not started");
    return Ok(None);
  }
  error!("not implemented");
  Ok(None)
}

async fn subcommand_toggle_verbose(context: &mut Context) -> Result<Option<String>, Error> {
  let instance_state = context.instance_state.read().await;
  if let Some(instance_state) = instance_state.as_ref() {
    let mut silent = instance_state.silent.write();
    *silent = !*silent;
    if *silent {
      info!("verbose mode enabled");
    } else {
      info!("silent mode enabled");
    }
  } else {
    error!("failed: not started");
    return Ok(None);
  }
  Ok(None)
}

fn config_fallback(value: Option<&str>, config: &str) -> Option<String> {
  if let Some(str) = value {
    Some(str.to_owned())
  } else if config.is_empty() {
    None
  } else {
    Some(config.to_owned())
  }
}

/// Print hint for Windows users because their cursor input is lost
fn print_hint() {
  #[cfg(windows)]
  {
    info!("Hint: press CTRL+Z if you loose the ability to type");
  }
}

#[derive(EnumString, EnumMessage, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
enum Action {
  #[strum(message = "starts factorio")]
  Start,
  #[strum(message = "stops factorio")]
  Stop,
  #[strum(message = "show status of factorio processes")]
  Status,
  #[strum(message = "toggle verbosity of factorio process")]
  ToggleVerbose,
  #[strum(message = "start additional clients")]
  Add,
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "factorio"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl
      .with_keybinding(
        KeyModifiers::CONTROL,
        KeyCode::Char('s'),
        ReedlineEvent::ExecuteHostCommand("factorio status".to_owned()),
      )
      .with_keybinding(
        KeyModifiers::CONTROL,
        KeyCode::Char('k'),
        ReedlineEvent::ExecuteHostCommand("factorio toggle-verbose".to_owned()),
      )
      .with_command_async(
        Command::new(self.name())
          .about("control factorio instances")
          .arg(
            Arg::new("action")
              .default_value(Into::<&str>::into(Action::Start))
              .value_parser(PossibleValuesParser::new(Action::iter().map(|action| {
                let message = action.get_message().unwrap();
                PossibleValue::new(Into::<&str>::into(action)).help(message)
              })))
              .help("what action to take"),
          )
          .arg(
            Arg::new("clients")
              .short('c')
              .default_value("")
              .long("clients")
              .help("number of clients to start in addition to the server"),
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
              .help("log server output to console"),
          )
          .arg(
            Arg::new("wait_until_finished")
              .short('w')
              .long("wait")
              .action(ArgAction::SetTrue)
              .help("wait until world discovery is done"),
          ),
        |args, context| Box::pin(run(args, context)),
      )
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
