mod factorio;
mod quit;
mod rcon;
mod run;

use crate::{constants, APP_ABOUT, APP_NAME};
use async_trait::async_trait;
use clap::{ArgMatches, Command};
use factorio_bot_core::process::process_control::FactorioInstance;
use miette::{IntoDiagnostic, Result};
use parking_lot::RwLock;
use reedline_repl_rs::Repl;
use std::sync::Arc;
use tokio::runtime::{Handle, Runtime};

const PROMPT: &str = "repl";

pub fn subcommands() -> Vec<Box<dyn ExecutableReplCommand>> {
  vec![
    factorio::build(),
    run::build(),
    rcon::build(),
    quit::build(),
  ]
}

pub struct Context {
  pub instance_state: Arc<RwLock<Option<FactorioInstance>>>,
  pub handle: Handle,
}
pub fn start() -> Result<()> {
  let (handle, _) = get_runtime_handle();
  let instance_state: Arc<RwLock<Option<FactorioInstance>>> = Arc::new(RwLock::new(None));
  let context = Context {
    instance_state: instance_state.clone(),
    handle,
  };
  let mut repl: Repl<Context, reedline_repl_rs::Error> = Repl::new(context)
    .with_name(APP_NAME)
    .with_prompt(&PROMPT)
    .with_history(constants::app_data_dir().join("repl_history"))
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_description(APP_ABOUT);
  for subcommand in subcommands() {
    let command = subcommand.build_command();
    let callback = subcommand.build_callback();
    repl = repl.add_command(command, callback);
  }
  repl.run().into_diagnostic()?;
  let mut instance_state = instance_state.write();
  if let Some(instance_state) = instance_state.take() {
    instance_state.stop()?;
  }
  Ok(())
}
fn get_runtime_handle() -> (Handle, Option<Runtime>) {
  if let Ok(h) = Handle::try_current() {
    (h, None)
  } else {
    let rt = Runtime::new().unwrap();
    (rt.handle().clone(), Some(rt))
  }
}

#[async_trait]
pub trait ExecutableReplCommand {
  fn name(&self) -> &str;
  fn build_command(&self) -> Command<'static>;
  fn build_callback(
    &self,
  ) -> fn(&ArgMatches, &mut Context) -> std::result::Result<Option<String>, reedline_repl_rs::Error>;
}
