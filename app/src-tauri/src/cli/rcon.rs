use crate::cli::ExecutableCommand;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::settings::FACTORIO_SETTINGS_DEFAULT;
use miette::Result;

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
  fn name(&self) -> &str {
    "rcon"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name())
      .arg(Arg::new("command").required(true).last(true))
      .arg(
        Arg::new("server")
          .short('s')
          .long("server")
          .value_name("server")
          .required(false)
          .help("connect to server instead of starting a server"),
      )
      .about("send given rcon command")
  }

  async fn run(&self, matches: &ArgMatches) -> Result<()> {
    let command = matches.value_of("command").unwrap();
    let server_host = matches.value_of("server");
    let rcon_settings = RconSettings::new_from_config(&FACTORIO_SETTINGS_DEFAULT, server_host);
    let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
    rcon.send(command).await.unwrap();
    Ok(())
  }
}
