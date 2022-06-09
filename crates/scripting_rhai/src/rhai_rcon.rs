use crate::error::{to_rhai_error, Result};
use factorio_bot_core::factorio::rcon::FactorioRcon;
use rhai::Engine;
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Clone)]
pub struct RhaiRcon {
    rcon: Arc<FactorioRcon>,
}

impl RhaiRcon {
    pub fn new(rcon: Arc<FactorioRcon>) -> Self {
        RhaiRcon { rcon }
    }

    pub fn register(engine: &mut Engine) {
        engine.register_type::<RhaiRcon>();
        // does not work with async :(
        // .register_result_fn("print", Self::print);
    }

    pub async fn _print(&mut self, str: &str) -> Result<()> {
        self.rcon.print(str).await.map_err(to_rhai_error)
    }
}
