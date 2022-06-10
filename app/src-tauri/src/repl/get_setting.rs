use crate::context::Context;
use crate::repl::{Error, Subcommand};
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "get"
  }

  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name()).about("get setting by key").arg(
        Arg::new("key")
          .required(true)
          .help("dotted path to setting"),
      ),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

#[allow(clippy::unused_async)]
async fn run(matches: ArgMatches, _context: &mut Context) -> Result<Option<String>, Error> {
  let _key = matches
    .value_of("key")
    .expect("Required value validated by clap")
    .to_owned();

  Ok(None)
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
