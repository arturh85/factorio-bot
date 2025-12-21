use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{value_parser, Arg, ArgMatches, Command};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::miette::Result;
use factorio_bot_core::parking_lot::RwLock;
use factorio_bot_core::settings::FactorioSettings;
use std::sync::Arc;

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "rcon"
  }
  fn build_command(&self) -> Command {
    Command::new("rcon")
      .arg(Arg::new("command").required(true).last(true))
      .arg(
        Arg::new("server")
          .short('s')
          .long("server")
          .value_name("server")
          .required(false)
          .value_parser(value_parser!(String))
          .help("connect to server instead of starting a server"),
      )
      .about("send given rcon command")
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, _context: &mut Context) -> Result<()> {
  let command = matches
    .get_one::<String>("command")
    .expect("required by clap")
    .as_str();
  let server_host = matches.get_one::<String>("server").cloned();
  let rcon_settings = RconSettings::new_from_config(&FactorioSettings::default(), server_host);
  let rcon = FactorioRcon::new(&rcon_settings, Arc::new(RwLock::new(false)))
    .await
    .unwrap();
  rcon.send(command).await.unwrap();
  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
