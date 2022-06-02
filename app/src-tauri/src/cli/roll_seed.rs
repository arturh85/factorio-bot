use crate::cli::ExecutableCommand;
use crate::settings::load_app_settings;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};

#[cfg(feature = "lua")]
use factorio_bot_scripting_lua::roll_best_seed::{roll_seed, RollSeedLimit};
use miette::{IntoDiagnostic, Result};

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
  fn name(&self) -> &str {
    "roll-seed"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name())
      .arg(
        Arg::new("map")
          .long("map")
          .value_name("map")
          .required(true)
          .help("use given map exchange string"),
      )
      .arg(
        Arg::new("seconds")
          .short('s')
          .long("seconds")
          .value_name("seconds")
          .default_value("360")
          .help("limits how long to roll seeds"),
      )
      .arg(
        Arg::new("parallel")
          .short('p')
          .long("parallel")
          .value_name("parallel")
          .default_value("4")
          .help("how many rolling servers to run in parallel"),
      )
      .arg(
        Arg::new("name")
          .long("name")
          .value_name("name")
          .required(true)
          .help("name of plan without .lua extension"),
      )
      .arg(
        Arg::new("rolls")
          .short('r')
          .long("rolls")
          .value_name("rolls")
          .help("how many seeds to roll"),
      )
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .help("number of clients to plan for"),
      )
      .about("roll good seed for given map-exchange-string based on heuristics")
  }
  async fn run(&self, matches: &ArgMatches) -> Result<()> {
    let app_settings = load_app_settings()?;
    if let Some((seed, score)) = roll_seed(
      &app_settings.factorio,
      matches.value_of("map").expect("map required!").into(),
      match matches.value_of("rolls") {
        Some(s) => RollSeedLimit::Rolls(s.parse().into_diagnostic()?),
        None => RollSeedLimit::Seconds(
          matches
            .value_of("seconds")
            .unwrap()
            .parse()
            .into_diagnostic()?,
        ),
      },
      matches
        .value_of("parallel")
        .unwrap()
        .parse()
        .into_diagnostic()?,
      matches.value_of("name").unwrap().into(),
      matches
        .value_of("clients")
        .unwrap()
        .parse()
        .into_diagnostic()?,
    )
    .await?
    {
      println!("Best Seed: {} with Score {}", seed, score);
    } else {
      eprintln!("no seed found");
    }
    Ok(())
  }
}
