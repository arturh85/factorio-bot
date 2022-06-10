use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::miette::Result;
use factorio_bot_core::settings::FactorioSettings;

impl Subcommand for ThisCommand {
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

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: ArgMatches, _context: &mut Context) -> Result<()> {
  let command = matches.value_of("command").unwrap();
  let server_host = matches
    .value_of("server")
    .map(std::borrow::ToOwned::to_owned);
  let rcon_settings = RconSettings::new_from_config(&FactorioSettings::default(), server_host);
  let rcon = FactorioRcon::new(&rcon_settings, false).await.unwrap();
  rcon.send(command).await.unwrap();
  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
