use crate::repl::{Context, ExecutableReplCommand};
use async_trait::async_trait;
use miette::Result;

pub struct ThisCommand {}

#[allow(dead_code)]
pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn commands(&self) -> Vec<String> {
    vec!["exit".to_string(), "quit".to_string()]
  }
  fn run(&self, _args: Vec<&str>, _context: &Context) -> Result<()> {
    std::process::exit(0);
  }
}
