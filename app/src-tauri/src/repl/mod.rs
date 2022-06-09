mod dump;
mod factorio_instance_control;
mod get_setting;
#[cfg(all(debug_assertions, feature = "gui"))]
mod gui;
mod quit;
mod rcon_send;
#[cfg(any(feature = "lua", feature = "rhai"))]
mod run_script;
mod set_setting;

use crate::context::Context;
use crate::{paths, APP_ABOUT, APP_NAME};
use miette::{miette, IntoDiagnostic};
use reedline_repl_rs::{yansi::Paint, Repl};
use std::fmt;

fn subcommands() -> Vec<Box<dyn Subcommand>> {
  vec![
    factorio_instance_control::build(),
    #[cfg(all(debug_assertions, feature = "gui"))]
    gui::build(),
    #[cfg(any(feature = "lua", feature = "rhai"))]
    run_script::build(),
    rcon_send::build(),
    set_setting::build(),
    get_setting::build(),
    quit::build(),
    dump::build(),
  ]
}

pub async fn start(context: Context) -> miette::Result<()> {
  let instance_state = context.instance_state.clone();
  let mut repl: Repl<Context, Error> = Repl::new(context)
    .with_name(APP_NAME)
    .with_description(APP_ABOUT)
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_prompt("repl")
    .with_history(paths::data_local_dir().join("repl_history"), 50)
    .with_on_after_command_async(|context| Box::pin(update_prompt(context)));
  for subcommand in subcommands() {
    repl = subcommand.build_command(repl);
  }
  repl.run_async().await.into_diagnostic()?;
  let mut instance_state = instance_state.write().await;
  if let Some(instance_state) = instance_state.take() {
    instance_state.stop()?;
  }
  Ok(())
}

async fn update_prompt(context: &mut Context) -> Result<Option<String>> {
  let instance_state = context.instance_state.read().await;
  let mut prompt = "repl".to_string();
  if instance_state.is_some() {
    prompt += &Box::new(Paint::blue(" [running]").bold()).to_string();
  };
  Ok(Some(prompt))
}

pub trait Subcommand {
  fn name(&self) -> &str;
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error>;
}

pub struct Error(miette::Error);
pub type Result<T> = std::result::Result<T, Error>;

impl From<reedline_repl_rs::Error> for Error {
  fn from(e: reedline_repl_rs::Error) -> Self {
    Self(miette!(e.to_string()))
  }
}
impl From<miette::Error> for Error {
  fn from(e: miette::Error) -> Self {
    Self(e)
  }
}
impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}", self.0)
  }
}
impl fmt::Debug for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}", self.0)
  }
}
