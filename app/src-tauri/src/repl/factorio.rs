use crate::repl::{Context, ExecutableReplCommand};
use crate::settings::load_app_settings;
use async_trait::async_trait;
use factorio_bot_core::process::process_control::{start_factorio, FactorioInstance};
use miette::Result;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct ThisCommand {}

#[allow(dead_code)]
pub fn build() -> Box<dyn ExecutableReplCommand> {
  Box::new(ThisCommand {})
}

#[async_trait]
impl ExecutableReplCommand for ThisCommand {
  fn commands(&self) -> Vec<String> {
    vec!["factorio start".to_string(), "factorio stop".to_string()]
  }
  fn run(&self, args: Vec<&str>, context: &Context) -> Result<()> {
    match *args.get(1).unwrap_or(&"") {
      "start" => {
        {
          let instance_state = context.instance_state.read();
          if instance_state.is_some() {
            error!("failed: already started");
            return Ok(());
          }
          drop(instance_state);
        }
        let instance_state = context.instance_state.clone();
        std::thread::spawn(move || {
          let rt = Runtime::new().unwrap();
          rt.block_on(do_start_factorio(instance_state)).unwrap();
        });
      }
      "stop" => {
        {
          let instance_state = context.instance_state.read();
          if instance_state.is_none() {
            error!("failed: not started");
            return Ok(());
          }
          drop(instance_state);
        }
        let mut instance_state = context.instance_state.write();
        instance_state.as_mut().unwrap().stop().unwrap();
        *instance_state = None;
        info!("stopped");
      }
      _ => error!("invalid command, use one of start, stop"),
    };
    Ok(())
  }
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
      Ok(None)
    }
    Err(_err) => todo!(),
  }
  // Ok(Some("successfully started!".into()))
}
