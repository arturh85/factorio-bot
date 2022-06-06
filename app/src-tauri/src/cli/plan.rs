use crate::cli::ExecutableCommand;
use crate::settings::load_app_settings;
use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};
use factorio_bot_core::types::PlayerId;
#[cfg(feature = "lua")]
use factorio_bot_scripting_lua::lua_runner::start_factorio_and_plan_graph;
use miette::Result;

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
  fn name(&self) -> &str {
    "plan"
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
        Arg::new("seed")
          .long("seed")
          .value_name("seed")
          .required(false)
          .help("use given seed to recreate level"),
      )
      .arg(
        Arg::new("name")
          .long("name")
          .value_name("name")
          .required(true)
          .help("name of plan without .lua extension"),
      )
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .help("number of clients to plan for"),
      )
      .about("plan graph")
  }

  async fn run(&self, matches: &ArgMatches) -> Result<()> {
    let app_settings = load_app_settings()?;
    let seed = matches
      .value_of("seed")
      .map(std::string::ToString::to_string);
    let name = matches
      .value_of("name")
      .expect("required arg name missing")
      .to_string();
    let map_exchange_string = matches
      .value_of("map")
      .map(std::string::ToString::to_string);
    let bot_count: PlayerId = matches.value_of("clients").unwrap().parse().unwrap();

    #[cfg(feature = "lua")]
    {
      let _graph = start_factorio_and_plan_graph(
        &app_settings.factorio,
        map_exchange_string,
        seed,
        &name,
        bot_count,
      )
      .await;
    }
    Ok(())
  }
}
