mod factorio;
mod quit;

use crate::constants;
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use factorio_bot_core::process::process_control::FactorioInstance;
use miette::{IntoDiagnostic, Result};
use nu_ansi_term::{Color, Style};
use parking_lot::RwLock;
use reedline::{
  default_emacs_keybindings, ColumnarMenu, DefaultCompleter, DefaultHinter, DefaultPrompt,
  DefaultValidator, Emacs, ExampleHighlighter, FileBackedHistory, Prompt, PromptEditMode,
  PromptHistorySearch, Reedline, ReedlineEvent, ReedlineMenu, Signal,
};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

pub fn start() -> Result<()> {
  enable_virtual_terminal_processing();
  let prompt = SimplePrompt::new("repl");
  let mut commands = vec!["help".to_string()];
  for subcommand in subcommands() {
    for command in subcommand.commands() {
      commands.push(command);
    }
  }
  let completer = Box::new(DefaultCompleter::new_with_wordlen(commands.clone(), 0));
  let completion_menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));
  let repl_history: PathBuf = constants::app_data_dir().join("repl_history");
  let history = FileBackedHistory::with_file(25, repl_history).unwrap();

  let mut keybindings = default_emacs_keybindings();
  keybindings.add_binding(
    KeyModifiers::NONE,
    KeyCode::Tab,
    ReedlineEvent::Menu("completion_menu".to_string()),
  );

  let validator = Box::new(DefaultValidator);
  let mut line_editor = Reedline::create()
    .with_edit_mode(Box::new(Emacs::new(keybindings)))
    .with_completer(completer)
    .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
    .with_hinter(Box::new(
      DefaultHinter::default().with_style(Style::new().italic().fg(Color::LightGray)),
    ))
    .with_history(Box::new(history))
    .with_highlighter(Box::new(ExampleHighlighter::new(commands.clone())))
    .with_validator(validator)
    .with_partial_completions(true)
    .with_quick_completions(false);

  let context = Context {
    instance_state: Arc::new(RwLock::new(None)),
  };
  loop {
    let sig = line_editor.read_line(&prompt).into_diagnostic()?;
    match sig {
      Signal::Success(line) => {
        if line.trim() != "" {
          let line_words: Vec<&str> = line.split(' ').collect();
          if line_words[0] == "help" {
            println!("\navailable commands:");
            println!("-------------------");
            for command in &commands {
              println!("- {}", command);
            }
            continue;
          }
          let mut found: Option<Box<dyn ExecutableReplCommand>> = None;
          for subcommand in subcommands() {
            for command in &subcommand.commands() {
              let command_words: Vec<&str> = command.split(' ').collect();
              if command_words[0] == line_words[0] {
                found = Some(subcommand);
                break;
              }
            }
            if found.is_some() {
              break;
            }
          }
          if let Some(subcommand) = found {
            subcommand.run(line_words.clone(), &context)?;
          } else {
            warn!("invalid command: {}", line);
          }
        }
      }
      Signal::CtrlD | Signal::CtrlC => {
        println!("\nquitting...");
        break;
      }
    }
  }
  disable_virtual_terminal_processing();
  Ok(())
}

pub struct Context {
  pub instance_state: Arc<RwLock<Option<FactorioInstance>>>,
}

pub fn subcommands() -> Vec<Box<dyn ExecutableReplCommand>> {
  vec![factorio::build(), quit::build()]
}

#[async_trait]
pub trait ExecutableReplCommand {
  fn commands(&self) -> Vec<String>;
  fn run(&self, args: Vec<&str>, context: &Context) -> Result<()>;
}

#[derive(Clone)]
pub struct SimplePrompt {
  default: DefaultPrompt,
  prefix: String,
}

impl Prompt for SimplePrompt {
  fn render_prompt_left(&self) -> Cow<str> {
    {
      Cow::Borrowed(&self.prefix)
    }
  }

  fn render_prompt_right(&self) -> Cow<str> {
    self.default.render_prompt_right()
  }

  fn render_prompt_indicator(&self, edit_mode: PromptEditMode) -> Cow<str> {
    self.default.render_prompt_indicator(edit_mode)
  }

  fn render_prompt_multiline_indicator(&self) -> Cow<str> {
    self.default.render_prompt_multiline_indicator()
  }

  fn render_prompt_history_search_indicator(
    &self,
    history_search: PromptHistorySearch,
  ) -> Cow<str> {
    self
      .default
      .render_prompt_history_search_indicator(history_search)
  }
}

impl Default for SimplePrompt {
  fn default() -> Self {
    SimplePrompt::new("repl")
  }
}

impl SimplePrompt {
  /// Constructor for the default prompt, which takes the amount of spaces required between the left and right-hand sides of the prompt
  pub fn new(left_prompt: &str) -> SimplePrompt {
    SimplePrompt {
      prefix: left_prompt.to_string(),
      default: DefaultPrompt::default(),
    }
  }
}

#[cfg(windows)]
pub fn enable_virtual_terminal_processing() {
  use winapi_util::console::Console;
  if let Ok(mut term) = Console::stdout() {
    let _guard = term.set_virtual_terminal_processing(true);
  }
  if let Ok(mut term) = Console::stderr() {
    let _guard = term.set_virtual_terminal_processing(true);
  }
}

#[cfg(windows)]
pub fn disable_virtual_terminal_processing() {
  use winapi_util::console::Console;
  if let Ok(mut term) = Console::stdout() {
    let _guard = term.set_virtual_terminal_processing(false);
  }
  if let Ok(mut term) = Console::stderr() {
    let _guard = term.set_virtual_terminal_processing(false);
  }
}

#[cfg(not(windows))]
pub fn enable_virtual_terminal_processing() {
  // no-op
}

#[cfg(not(windows))]
pub fn disable_virtual_terminal_processing() {
  // no-op
}
