use crate::cli::ExecutableCommand;
use async_trait::async_trait;
use clap::{ArgMatches, Command};
use crossterm::{
  event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
  execute,
  terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use miette::{IntoDiagnostic, Result};
use std::io;
use tui::widgets::{Block, Borders};
use tui::{
  backend::{Backend, CrosstermBackend},
  layout::{Constraint, Direction, Layout},
  Frame, Terminal,
};

pub fn build() -> Box<dyn ExecutableCommand> {
  Box::new(ThisCommand {})
}
struct ThisCommand {}

#[async_trait]
impl ExecutableCommand for ThisCommand {
  fn name(&self) -> &str {
    "tui"
  }
  fn build_command(&self) -> Command<'static> {
    Command::new(self.name()).about("starts interactive terminal user interface")
  }

  async fn run(&self, _matches: &ArgMatches) -> Result<()> {
    // setup terminal
    enable_raw_mode().into_diagnostic()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).into_diagnostic()?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).into_diagnostic()?;

    // create app and run it
    let _res = run_app(&mut terminal);

    // restore terminal
    disable_raw_mode().into_diagnostic()?;
    execute!(
      terminal.backend_mut(),
      LeaveAlternateScreen,
      DisableMouseCapture
    )
    .into_diagnostic()?;
    terminal.show_cursor().into_diagnostic()?;
    Ok(())
  }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>) -> io::Result<()> {
  loop {
    terminal.draw(|f| ui(f))?;

    if let Event::Key(key) = event::read()? {
      if let KeyCode::Char('q') = key.code {
        return Ok(());
      }
    }
  }
}

fn ui<B: Backend>(f: &mut Frame<B>) {
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints(
      [
        Constraint::Percentage(20),
        Constraint::Percentage(60),
        Constraint::Percentage(20),
      ]
      .as_ref(),
    )
    .split(f.size());

  let block = Block::default().title("Block").borders(Borders::ALL);
  f.render_widget(block, chunks[0]);
  let block = Block::default().title("Block 2").borders(Borders::ALL);
  f.render_widget(block, chunks[2]);
}
