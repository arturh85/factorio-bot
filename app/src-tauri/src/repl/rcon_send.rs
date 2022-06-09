use crate::context::Context;
use crate::repl::{Error, Subcommand};
use crate::settings::load_app_settings;
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "rcon"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name())
        .about("send rcon command")
        .arg(Arg::new("rcon-command").required(true).index(1)),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

async fn run(matches: ArgMatches, context: &mut Context) -> Result<Option<String>, Error> {
  let command = matches
    .value_of("rcon-command")
    .expect("Required arg validated by clap")
    .to_string();
  let instance_state = context.instance_state.read().await;
  if instance_state.is_some() {
    let instance_state = context.instance_state.clone();
    let instance_state = instance_state.read().await;
    if let Some(_instance_state) = instance_state.as_ref() {
      let app_settings = load_app_settings().unwrap();
      let rcon_settings = RconSettings::new(
        app_settings.factorio.rcon_port as u16,
        &app_settings.factorio.rcon_pass,
        None,
      );
      let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
      rcon.send(&command).await?;
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
