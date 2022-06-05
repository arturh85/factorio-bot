mod factorio;
mod quit;

use crate::{constants, APP_ABOUT, APP_NAME};
use async_trait::async_trait;
use factorio_bot_core::process::process_control::FactorioInstance;
use miette::{IntoDiagnostic, Result};
use parking_lot::RwLock;
use reedline_repl_rs::{Command, Repl};
use std::sync::Arc;

const PROMPT: &str = "repl";

pub fn start() -> Result<()> {
  let mut repl = Repl::new(Context {
    instance_state: Arc::new(RwLock::new(None)),
  })
  .with_name(APP_NAME)
  .with_prompt(&PROMPT)
  .with_history(constants::app_data_dir().join("repl_history"))
  .with_version(env!("CARGO_PKG_VERSION"))
  .with_description(APP_ABOUT);
  for subcommand in subcommands() {
    repl = repl.add_command(subcommand.build_command()?);
  }
  repl.run().into_diagnostic()
}

pub struct Context {
  pub instance_state: Arc<RwLock<Option<FactorioInstance>>>,
}

pub fn subcommands() -> Vec<Box<dyn ExecutableReplCommand>> {
  vec![factorio::build(), quit::build()]
}

#[async_trait]
pub trait ExecutableReplCommand {
  fn build_command(&self) -> Result<Command<Context, reedline_repl_rs::Error>>;
}
