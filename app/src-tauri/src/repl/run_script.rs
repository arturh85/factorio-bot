use crate::context::Context;
use crate::repl::{Error, Subcommand};
use crate::scripting::{prepare_workspace_scripts, run_script_file};
use crate::settings::load_app_settings;
use factorio_bot_core::miette::IntoDiagnostic;
use factorio_bot_core::paris::error;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::types::PlayerId;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;
use std::path::Path;

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let filename = matches.value_of("filename").unwrap().to_owned();
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
      let mut planner = Planner::new(
        instance_state.world.clone().unwrap(),
        Some(instance_state.rcon.clone()),
      );
      if let Err(err) = run_script_file(&mut planner, &filename, bot_count, false).await {
        error!("failed to execute: {:?}", err);
      }
    }
  } else {
    error!("failed: not started");
  }
  Ok(None)
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "run"
  }

  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    // let app_settings = context.app_settings.read().await;
    let app_settings = load_app_settings().unwrap();
    let workspace_path = app_settings.factorio.workspace_path.to_string();
    let workspace_path = Path::new(&workspace_path);
    let dir =
      std::fs::read_dir(prepare_workspace_scripts(workspace_path).expect("failed to prepare"))
        .expect("failed to read script dir");
    let _entries: Vec<String> = dir
      .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
      .collect();
    repl.with_command_async(
      Command::new(self.name())
        .about("run script")
        .arg(
          Arg::new("filename")
            // use clap::builder::PossibleValuesParser;
            // use clap::PossibleValue;
            // .value_parser(PossibleValuesParser::new(
            //   entries.into_iter().map(|entry| PossibleValue::new(entry)),
            // ))
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

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
