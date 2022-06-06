use crate::repl::{Context, ExecutableReplCommand};
use crate::settings::load_app_settings;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use reedline_repl_rs::Callback;

pub struct ThisCommand {}

pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn name(&self) -> &str {
    "rcon"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name())
      .about("send rcon command")
      .arg(Arg::new("rcon-command").required(true).index(1))
  }

  fn build_callback(&self) -> Callback<Context, reedline_repl_rs::Error> {
    |matches: &ArgMatches, context: &mut Context| {
      let command = matches.value_of("rcon-command").unwrap().to_string();
      let instance_state = context.instance_state.read();
      if instance_state.is_some() {
        let instance_state = context.instance_state.clone();
        let handle = context.handle.clone();
        std::thread::spawn(move || {
          let instance_state = instance_state.read();
          if let Some(_instance_state) = instance_state.as_ref() {
            let app_settings = load_app_settings().unwrap();
            let rcon_settings = RconSettings::new(
              app_settings.factorio.rcon_port as u16,
              &app_settings.factorio.rcon_pass,
              None,
            );

            if let Err(err) = handle.block_on(async {
              let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
              rcon.send(&command).await
            }) {
              error!("failed to send {:?}", err);
            }
          }
        })
        .join()
        .unwrap();
      } else {
        error!("failed: not started");
      }
      Ok(None)
    }
  }
}
