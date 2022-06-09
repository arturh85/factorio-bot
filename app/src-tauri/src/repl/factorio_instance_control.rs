use crate::context::Context;
use crate::repl::{Error, Subcommand};
use crate::settings::load_app_settings;
use factorio_bot_core::process::process_control::start_factorio;
use miette::{IntoDiagnostic, Result};
use reedline_repl_rs::clap::{Arg, ArgMatches, Command, PossibleValue};
use reedline_repl_rs::Repl;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "factorio"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name())
        .about("start/stop factorio")
        .arg(
          Arg::new("action")
            .default_value("start")
            .possible_values(vec![
              PossibleValue::new("start").help("starts factorio"),
              PossibleValue::new("stop").help("stops factorio"),
            ])
            .help("either start or stop factorio server"),
        )
        .arg(
          Arg::new("clients")
            .short('c')
            .long("clients")
            .default_value("1")
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
            .help("recreate level by deleting server map if exists"),
        )
        .arg(
          Arg::new("logs")
            .short('l')
            .long("logs")
            .help("enabled writing server & client logs to workspace"),
        )
        .arg(
          Arg::new("verbose")
            .short('v')
            .long("verbose")
            .help("log server output to console"),
        ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

/// Print hint for Windows users because their cursor input is lost
fn print_hint() {
  #[cfg(windows)]
  {
    info!("Hint: press CTRL+Z if you loose the ability to type");
  }
}

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let action: &str = matches.value_of("action").expect("Has default value");
  match action {
    "start" => {
      let app_settings = context.app_settings.read().await;
      let clients: u8 = if let Some(clients) = matches.value_of("clients") {
        clients.parse().into_diagnostic()?
      } else {
        app_settings.factorio.client_count
      };
      let write_logs: bool = matches.is_present("logs");
      let verbose: bool = matches.is_present("verbose");
      let seed = if let Some(seed) = matches.value_of("seed") {
        Some(seed.to_string())
      } else if app_settings.factorio.seed == "" {
        None
      } else {
        Some(app_settings.factorio.seed.to_string())
      };
      let map_exchange_string = if let Some(map_exchange_string) = matches.value_of("map") {
        Some(map_exchange_string.to_string())
      } else if app_settings.factorio.map_exchange_string == "" {
        None
      } else {
        Some(app_settings.factorio.map_exchange_string.to_string())
      };
      let recreate = matches.is_present("new");
      {
        let instance_state = context.instance_state.read().await;
        if instance_state.is_some() {
          error!("failed: already started");
          return Ok(None);
        }
        drop(instance_state);
      }
      print_hint();
      let instance_state = context.instance_state.clone();
      let app_settings = load_app_settings().unwrap();
      match start_factorio(
        &app_settings.factorio,
        None,
        clients,
        recreate,
        map_exchange_string,
        seed,
        write_logs,
        !verbose,
      )
      .await
      {
        Ok(new_instance_state) => {
          let mut instance_state = instance_state.write().await;
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
    }
    "stop" => {
      {
        let instance_state = context.instance_state.read().await;
        if instance_state.is_none() {
          error!("failed: not started");
          return Ok(None);
        }
        drop(instance_state);
      }
      let mut instance_state = context.instance_state.write().await;
      instance_state.take().expect("Already checked").stop()?;
      *instance_state = None;
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
