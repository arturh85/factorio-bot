use crate::repl::{Context, ExecutableReplCommand};
use crate::settings::load_app_settings;
use async_trait::async_trait;
use factorio_bot_core::process::process_control::{start_factorio, FactorioInstance};
use miette::{IntoDiagnostic, Result};
use parking_lot::RwLock;
use reedline_repl_rs::Convert;
use reedline_repl_rs::{Command, Parameter, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct ThisCommand {}

#[allow(dead_code)]
pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn build_command(&self) -> Result<Command<Context, reedline_repl_rs::Error>> {
    let command = Command::new("factorio", run)
      .with_parameter(
        Parameter::new("action")
          .add_allowed_value("start", Some("start factorio"))
          .add_allowed_value("stop", Some("stop factorio"))
          .with_help("control factorio instances")
          .set_required(true)
          .into_diagnostic()?,
      )
      .into_diagnostic()?
      .with_help("control factorio instances");
    Ok(command)
  }
}

#[allow(clippy::needless_pass_by_value)]
fn run(
  args: HashMap<String, Value>,
  context: &mut Context,
) -> reedline_repl_rs::Result<Option<String>> {
  let action: String = args["action"].convert()?;
  match action.as_str() {
    "start" => {
      {
        let instance_state = context.instance_state.read();
        if instance_state.is_some() {
          error!("failed: already started");
          return Ok(None);
        }
        drop(instance_state);
      }
      let instance_state = context.instance_state.clone();
      std::thread::spawn(move || {
        let rt = Runtime::new().unwrap();
        rt.block_on(do_start_factorio(instance_state)).unwrap();
        info!("block on finished");
      })
      .join()
      .unwrap();
      info!("join finished");
    }
    "stop" => {
      {
        let instance_state = context.instance_state.read();
        if instance_state.is_none() {
          error!("failed: not started");
          return Ok(None);
        }
        drop(instance_state);
      }
      let mut instance_state = context.instance_state.write();
      instance_state.take().unwrap().stop().unwrap();
      *instance_state = None;
      info!("stopped");
    }
    _ => error!("invalid command, use one of start, stop"),
  };
  Ok(None)
}

async fn do_start_factorio(
  instance_state: Arc<RwLock<Option<FactorioInstance>>>,
) -> Result<Option<String>> {
  let app_settings = load_app_settings().unwrap();
  match start_factorio(
    &app_settings.factorio,
    None,
    // app_settings.factorio.client_count as u8,
    0,
    app_settings.factorio.recreate,
    if app_settings.factorio.map_exchange_string == "" {
      None
    } else {
      Some(app_settings.factorio.map_exchange_string.to_string())
    },
    if app_settings.factorio.seed == "" {
      None
    } else {
      Some(app_settings.factorio.seed.to_string())
    },
    false,
    true,
  )
  .await
  {
    Ok(new_instance_state) => {
      info!("factorio started");
      let mut instance_state = instance_state.write();
      *instance_state = Some(new_instance_state);
      drop(instance_state);
      info!("factorio instance state written");
      Ok(None)
    }
    Err(_err) => todo!(),
  }
  // Ok(Some("successfully started!".into()))
}
