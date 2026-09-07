use crate::context::Context;
use crate::repl::{Error, Subcommand};
use crate::scripting::run_script_file;
use factorio_bot_core::app_settings::load_app_settings;
use factorio_bot_core::miette;
use factorio_bot_core::miette::IntoDiagnostic;
use factorio_bot_core::paris::error;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::types::PlayerId;
use reedline_repl_rs::Repl;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let filename = matches.get_one::<String>("filename").unwrap().to_owned();
  let bot_count: PlayerId = matches
    .get_one::<String>("bots")
    .expect("Has default value")
    .parse()
    .into_diagnostic()?;
  let instance_state = context.instance_state.read().await;
  if instance_state.is_some() {
    let instance_state = context.instance_state.clone();
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = instance_state.as_ref() {
      let mut planner = Planner::new(
        instance_state.surface().cloned().unwrap(),
        Some(instance_state.rcon.clone()),
      );
      // The settings the REPL was started with, override and all -- not a
      // fresh `load_app_settings()`, which would read the default file and
      // resolve scripts against a workspace this session is not using.
      let app_settings = context.app_settings.read().await.clone();
      if let Err(err) =
        run_script_file(&mut planner, &app_settings, &filename, bot_count, None).await
      {
        error!("failed to execute: {:?}", err);
      }
    }
  } else {
    error!("failed: not started");
  }
  Ok(None)
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "run"
  }

  fn build_command(&self, repl: Repl<Context, Error>) -> miette::Result<Repl<Context, Error>> {
    // let app_settings = context.app_settings.read().await;
    // Was `.unwrap()`. This runs inside `start()`, before clap has chosen a
    // subcommand and before there is a `Context` to read settings from
    // asynchronously, so a bare `?` propagating through `build_command`'s
    // `Result` (rather than a panic) is the only available error channel --
    // and release builds use `panic = "abort"`, so the old `.unwrap()` took
    // the whole process down on a malformed settings file.
    let app_settings = load_app_settings()?;
    // Was `Path::new(&app_settings.factorio.workspace_path.to_string())`,
    // handed straight to `ensure_scripts_dir`, which joins it against the
    // process's working directory -- exactly the hazard `resolve_workspace`
    // exists to close. `ensure_scripts_dir` now requires
    // `&paths::ResolvedWorkspace`, so a relative `workspace_path` is refused
    // here instead of silently reading `<cwd>/<relative>/scripts`.
    let workspace_path =
      factorio_bot_core::paths::resolve_workspace(&app_settings.factorio.workspace_path)?;
    // Must be the same root `run_script_file` executes from, or the REPL lists
    // one directory and runs from another. `scripts_dir` prefers `./scripts`
    // relative to the process CWD; `ensure_scripts_dir` is workspace-only.
    let scripts_dir = factorio_bot_core::scripts::ensure_scripts_dir(&workspace_path)?;
    let dir = std::fs::read_dir(scripts_dir).into_diagnostic()?;
    let _entries: Vec<String> = dir
      .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
      .collect();
    Ok(
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
      ),
    )
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
