use crate::paths;
use crate::settings::{load_app_settings, SharedAppSettings};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use std::fs::create_dir_all;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

pub type SharedJoinShandle<T> = Arc<RwLock<Option<JoinHandle<T>>>>;
pub type SharedRestApiHandle = SharedJoinShandle<Result<()>>;

#[derive(Clone)]
pub struct Context {
  pub instance_state: SharedFactorioInstance,
  pub app_settings: SharedAppSettings,
  pub restapi_handle: SharedRestApiHandle,
}

impl Context {
  pub fn new() -> Result<Self> {
    color_eyre::install().expect("failed to colorize panics");
    #[cfg(feature = "tokio-console")]
    {
      console_subscriber::init();
    }

    create_dir_all(paths::data_local_dir()).into_diagnostic()?;
    create_dir_all(paths::workspace_dir()).into_diagnostic()?;

    let context = Context {
      instance_state: FactorioInstance::new_shared(),
      restapi_handle: Arc::new(RwLock::new(None)),
      app_settings: load_app_settings()?.into_shared(),
    };

    Ok(context)
  }
}
