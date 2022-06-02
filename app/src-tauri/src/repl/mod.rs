mod factorio;
mod quit;

use crate::{APP_ABOUT, APP_NAME};
use async_trait::async_trait;
use factorio_bot_core::process::process_control::FactorioInstance;
use miette::{IntoDiagnostic, Result};
use parking_lot::RwLock;
use repl_rs::Command;
use repl_rs::Repl;
use std::sync::Arc;

const PROMPT: &str = "repl> ";

pub fn start() -> Result<()> {
  let mut repl = Repl::new(Context {
    instance_state: Arc::new(RwLock::new(None)),
  })
  .with_name(APP_NAME)
  .with_prompt(&PROMPT)
  .with_version(env!("CARGO_PKG_VERSION"))
  .with_description(APP_ABOUT)
  .use_completion(true);
  for subcommand in subcommands() {
    repl = repl.add_command(subcommand.build_command()?);
  }
  repl.run().into_diagnostic()
}

pub struct Context {
  pub instance_state: Arc<RwLock<Option<FactorioInstance>>>,
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

#[async_trait]
pub trait ExecutableReplCommand {
  fn build_command(&self) -> Result<Command<Context, repl_rs::Error>>;
}

fn subcommands() -> Vec<Box<dyn ExecutableReplCommand>> {
  vec![factorio::build(), quit::build()]
}
