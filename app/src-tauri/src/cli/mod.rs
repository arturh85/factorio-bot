#[cfg(feature = "lua")]
mod plan;
#[cfg(debug_assertions)]
mod playground;
mod rcon;
#[cfg(feature = "repl")]
mod repl;
#[cfg(feature = "lua")]
mod roll_seed;
mod start;

use crate::context::Context;
use crate::{APP_ABOUT, APP_AUTHOR, APP_NAME};
use clap::{ArgMatches, Command};
use miette::Result;
use std::future::Future;
use std::pin::Pin;

pub fn subcommands() -> Vec<Box<dyn Subcommand>> {
  vec![
    rcon::build(),
    #[cfg(feature = "lua")]
    plan::build(),
    #[cfg(feature = "lua")]
    roll_seed::build(),
    #[cfg(debug_assertions)]
    playground::build(),
    #[cfg(feature = "repl")]
    repl::build(),
    start::build(),
  ]
}

pub async fn start(mut context: Context) -> Result<Option<Command<'static>>> {
  let mut app = Command::new(APP_NAME)
    .version(env!("CARGO_PKG_VERSION"))
    .author(APP_AUTHOR)
    .about(APP_ABOUT);
  let subcommands = subcommands();
  for subcommand in &subcommands {
    app = app.subcommand(subcommand.build_command());
  }
  let matches = app.clone().get_matches();
  for subcommand in &subcommands {
    if let Some(matches) = matches.subcommand_matches(&subcommand.name()) {
      let callback = subcommand.build_callback();
      callback(matches.clone(), &mut context).await?;
      return Ok(None);
    }
  }
  Ok(Some(app))
}

pub trait Subcommand {
  fn name(&self) -> &str;
  fn build_command(&self) -> Command<'static>;
  fn build_callback(&self) -> SubcommandCallback;
}

pub type SubcommandCallback =
  fn(ArgMatches, &'_ mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + '_>>;
