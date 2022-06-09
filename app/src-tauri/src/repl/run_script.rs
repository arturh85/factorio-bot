use crate::context::Context;
use crate::repl::{Error, Subcommand};
use crate::scripting::run_script_file;
use crate::settings::load_app_settings;
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::types::PlayerId;
use miette::IntoDiagnostic;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;
use std::sync::Arc;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "run"
  }

  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    // let app_settings = load_app_settings().expect("failed to load settings");
    // let workspace_path = app_settings.factorio.workspace_path.to_string();
    // let workspace_path = Path::new(&workspace_path);
    // let dir =
    //   std::fs::read_dir(prepare_workspace_scripts(workspace_path).expect("failed to prepare"))
    //     .expect("failed to read script dir");
    // let entries: Vec<String> = dir
    //   .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_string())
    //   .collect();
    repl.with_command_async(
      Command::new(self.name())
        .about("run script")
        .arg(
          Arg::new("filename")
            // NOTE: not possible currently, see https://github.com/clap-rs/clap/issues/1232
            // .possible_values(entries.into_iter().map(|entry| PossibleValue::new(&entry)))
            .required(true)
            .index(1),
        )
        .arg(
          Arg::new("bots")
            .short('b')
            .long("bots")
            .default_value("1")
            .help("number of bots to use for running the script"),
        ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let filename = matches.value_of("filename").unwrap().to_string();
  let bot_count: PlayerId = matches
    .value_of("bots")
    .expect("Has default value")
    .parse()
    .into_diagnostic()?;
  let instance_state = context.instance_state.read().await;
  if instance_state.is_some() {
    let instance_state = context.instance_state.clone();
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = instance_state.as_ref() {
      let app_settings = load_app_settings().unwrap();
      let rcon_settings = RconSettings::new(
        app_settings.factorio.rcon_port as u16,
        &app_settings.factorio.rcon_pass,
        None,
      );
      let world = instance_state.world.clone();
      let rcon = Arc::new(FactorioRcon::new(&rcon_settings, false).await.unwrap());
      let mut planner = Planner::new(world.unwrap(), Some(rcon));
      if let Err(err) = run_script_file(&mut planner, &filename, bot_count, false).await {
        error!("failed to execute: {:?}", err);
      }
    }
  } else {
    error!("failed: not started");
  }
  Ok(None)
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
