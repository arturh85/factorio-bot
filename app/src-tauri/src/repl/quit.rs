use crate::repl::{Context, ExecutableReplCommand};
use async_trait::async_trait;
use miette::Result;
use repl_rs::{Command, Value};
use std::collections::HashMap;

pub struct Quit {}

#[allow(dead_code)]
pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(Quit {})
}

#[async_trait]
impl ExecutableReplCommand for Quit {
  fn build_command(&self) -> Result<Command<Context, repl_rs::Error>> {
    let command = Command::new("q", run).with_help("quit");
    Ok(command)
  }
}

#[allow(clippy::needless_pass_by_value)]
fn run(_args: HashMap<String, Value>, _context: &mut Context) -> repl_rs::Result<Option<String>> {
  std::process::exit(0);
}
