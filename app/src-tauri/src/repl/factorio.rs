use crate::repl::{Context, ExecutableReplCommand};
use crate::settings::load_app_settings;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command, PossibleValue};
use factorio_bot_core::process::process_control::{start_factorio, FactorioInstance};
use miette::Result;
use parking_lot::RwLock;
use reedline_repl_rs::Callback;
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct ThisCommand {}

pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn name(&self) -> &str {
    "factorio"
  }
  fn build_command(&self) -> Command<'static> {
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
      )
  }

  fn build_callback(&self) -> Callback<Context, reedline_repl_rs::Error> {
    |matches: &ArgMatches, context: &mut Context| {
      let action: &str = matches.value_of("action").unwrap();
      match action {
        "start" => {
          let app_settings = load_app_settings().unwrap();
          let clients: u8 = if let Some(clients) = matches.value_of("clients") {
            clients.parse().expect("failed to parse clients")
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
            let instance_state = context.instance_state.read();
            if instance_state.is_some() {
              error!("failed: already started");
              return Ok(None);
            }
            drop(instance_state);
          }
          info!("Hint: press CTRL+Z if you loose the ability to type");
          let instance_state = context.instance_state.clone();
          std::thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            rt.block_on(do_start_factorio(
              instance_state,
              clients,
              recreate,
              map_exchange_string,
              seed,
              write_logs,
              !verbose,
            ))
            .unwrap();
          })
          .join()
          .unwrap();
        }
        "stop" => {
          {
            let instance_state = context.instance_state.read();
            if instance_state.is_none() {
              error!("failed: not started");
              return Ok(None);
            }
            drop(instance_state);
          }
          let mut instance_state = context.instance_state.write();
          instance_state.take().unwrap().stop().unwrap();
          *instance_state = None;
          info!("stopped");
        }
        _ => error!("invalid command, use one of start, stop"),
      };
      Ok(None)
    }
  }
}

async fn do_start_factorio(
  instance_state: Arc<RwLock<Option<FactorioInstance>>>,
  client_count: u8,
  recreate: bool,
  map_exchange_string: Option<String>,
  seed: Option<String>,
  write_logs: bool,
  silent: bool,
) -> Result<Option<String>> {
  let app_settings = load_app_settings().unwrap();
  match start_factorio(
    &app_settings.factorio,
    None,
    client_count,
    recreate,
    map_exchange_string,
    seed,
    write_logs,
    silent,
  )
  .await
  {
    Ok(new_instance_state) => {
      let mut instance_state = instance_state.write();
      *instance_state = Some(new_instance_state);
      drop(instance_state);
      Ok(None)
    }
    Err(_err) => todo!(),
  }
  // Ok(Some("successfully started!".into()))
}
