use crate::repl::{Context, ExecutableReplCommand};
use crate::scripting::run_script_file;
use crate::settings::load_app_settings;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::types::PlayerId;
use reedline_repl_rs::Callback;
use std::sync::Arc;

pub struct ThisCommand {}

pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn name(&self) -> &str {
    "run"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name())
      .about("run script")
      .arg(Arg::new("filename").required(true).index(1))
      .arg(
        Arg::new("bots")
          .short('b')
          .long("bots")
          .default_value("1")
          .help("number of bots to use for running the script"),
      )
  }

  fn build_callback(&self) -> Callback<Context, reedline_repl_rs::Error> {
    |matches: &ArgMatches, context: &mut Context| {
      let filename = matches.value_of("filename").unwrap().to_string();
      let bot_count: PlayerId = matches.value_of("bots").unwrap().parse()?;
      let instance_state = context.instance_state.read();
      if instance_state.is_some() {
        let instance_state = context.instance_state.clone();
        let handle = context.handle.clone();
        if let Err(err) = std::thread::spawn(move || {
          let instance_state = instance_state.read();
          if let Some(instance_state) = instance_state.as_ref() {
            let app_settings = load_app_settings().unwrap();

            let rcon_settings = RconSettings::new(
              app_settings.factorio.rcon_port as u16,
              &app_settings.factorio.rcon_pass,
              None,
            );
            if let Err(err) = handle.block_on(async {
              let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
              let mut planner =
                Planner::new(instance_state.world.clone().unwrap(), Some(Arc::new(rcon)));
              run_script_file(&mut planner, &filename, bot_count)
            }) {
              error!("failed to execute {}: {:?}", filename, err);
            }
          }
        })
        .join()
        {
          error!("failed: {:?}", err);
        }
      } else {
        error!("failed: not started");
      }
      Ok(None)
    }
  }
}
