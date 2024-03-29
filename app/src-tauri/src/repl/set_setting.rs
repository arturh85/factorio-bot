use crate::context::Context;
use crate::repl::{Error, Subcommand};
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;

#[allow(clippy::unused_async)]
async fn run(matches: ArgMatches, _context: &mut Context) -> Result<Option<String>, Error> {
  let _key = matches
    .value_of("key")
    .expect("Required value validated by clap")
    .to_owned();
  let _value = matches
    .value_of("key")
    .expect("Required value validated by clap")
    .to_owned();

  Ok(None)
}

impl Subcommand for ThisCommand {
  fn name(&self) -> &str {
    "set"
  }
  fn build_command(&self, repl: Repl<Context, Error>) -> Repl<Context, Error> {
    repl.with_command_async(
      Command::new(self.name())
        .about("set setting by key")
        .arg(
          Arg::new("key")
            .required(true)
            .help("dotted path to setting"),
        )
        .arg(Arg::new("value").required(true).help("new value")),
      |args, context| Box::pin(run(args, context)),
    )
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
