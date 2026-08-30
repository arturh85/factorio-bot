pub mod fs;
pub mod instance;
pub mod rcon;
pub mod scripts;
pub mod settings;

use crate::state::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(settings::get_settings))
        .routes(routes!(settings::put_settings))
        .routes(routes!(instance::get_instance))
        .routes(routes!(instance::stop_instance))
        .routes(routes!(rcon::send_rcon))
        .routes(routes!(scripts::list_scripts))
        .routes(routes!(scripts::read_script))
        .routes(routes!(scripts::write_script))
        .routes(routes!(scripts::create_script))
        .routes(routes!(scripts::delete_script))
        .routes(routes!(fs::exists))
}
