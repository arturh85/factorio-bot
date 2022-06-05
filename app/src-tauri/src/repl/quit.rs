use crate::repl::{Context, ExecutableReplCommand};
use async_trait::async_trait;
use miette::Result;
use reedline_repl_rs::{Command, Value};
use std::collections::HashMap;

pub struct ThisCommand {}

#[allow(dead_code)]
pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn build_command(&self) -> Result<Command<Context, reedline_repl_rs::Error>> {
    let command = Command::new("quit", run).with_help("quit");
    Ok(command)
  }
}

#[allow(clippy::needless_pass_by_value)]
fn run(
  _args: HashMap<String, Value>,
  _context: &mut Context,
) -> reedline_repl_rs::Result<Option<String>> {
  std::process::exit(0);
}
