use crate::repl::{Context, ExecutableReplCommand};
use async_trait::async_trait;
use clap::{ArgMatches, Command};
use reedline_repl_rs::Callback;

pub struct ThisCommand {}

pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn name(&self) -> &str {
    "quit"
  }

  fn build_command(&self) -> Command<'static> {
    Command::new(self.name()).about("stop all running instances and quit")
  }

  fn build_callback(&self) -> Callback<Context, reedline_repl_rs::Error> {
    |_matches: &ArgMatches, context: &mut Context| {
      let mut instance_state = context.instance_state.write();
      if let Some(instance_state) = instance_state.take() {
        instance_state.stop().expect("failed to stop");
      }
      std::process::exit(0);
    }
  }
}
