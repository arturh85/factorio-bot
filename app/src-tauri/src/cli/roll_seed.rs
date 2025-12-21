use crate::cli::{Subcommand, SubcommandCallback};
use crate::settings::load_app_settings;
use clap::{value_parser, Arg, ArgMatches, Command};

use crate::context::Context;
use factorio_bot_core::miette::Result;
#[cfg(feature = "lua")]
use factorio_bot_scripting_lua::roll_best_seed::{roll_seed, RollSeedLimit};

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "roll-seed"
  }
  fn build_command(&self) -> Command {
    Command::new("roll-seed")
      .arg(
        Arg::new("map")
          .long("map")
          .value_name("map")
          .required(true)
          .value_parser(value_parser!(String))
          .help("use given map exchange string"),
      )
      .arg(
        Arg::new("seconds")
          .short('s')
          .long("seconds")
          .value_name("seconds")
          .default_value("360")
          .value_parser(value_parser!(u64))
          .help("limits how long to roll seeds"),
      )
      .arg(
        Arg::new("parallel")
          .short('p')
          .long("parallel")
          .value_name("parallel")
          .default_value("4")
          .value_parser(value_parser!(u8))
          .help("how many rolling servers to run in parallel"),
      )
      .arg(
        Arg::new("name")
          .long("name")
          .value_name("name")
          .required(true)
          .value_parser(value_parser!(String))
          .help("name of plan without .lua extension"),
      )
      .arg(
        Arg::new("rolls")
          .short('r')
          .long("rolls")
          .value_name("rolls")
          .value_parser(value_parser!(u64))
          .help("how many seeds to roll"),
      )
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .value_parser(value_parser!(u8))
          .help("number of clients to plan for"),
      )
      .about("roll good seed for given map-exchange-string based on heuristics")
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, _context: &mut Context) -> Result<()> {
  let app_settings = load_app_settings()?;
  if let Some((seed, score)) = roll_seed(
    app_settings.factorio.clone(),
    matches
      .get_one::<String>("map")
      .expect("required by clap")
      .to_owned(),
    match matches.get_one::<u64>("rolls") {
      Some(rolls) => RollSeedLimit::Rolls(*rolls),
      None => RollSeedLimit::Seconds(
        *matches
          .get_one::<u64>("seconds")
          .expect("defaulted by clap"),
      ),
    },
    *matches
      .get_one::<u8>("parallel")
      .expect("defaulted by clap"),
    matches
      .get_one::<String>("name")
      .expect("required by clap")
      .to_owned(),
    *matches.get_one::<u8>("clients").expect("defaulted by clap"),
  )
  .await?
  {
    println!("Best Seed: {seed} with Score {score}");
  } else {
    eprintln!("no seed found");
  }
  Ok(())
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
