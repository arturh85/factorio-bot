pub mod instance;
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
}
