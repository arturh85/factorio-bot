#[cfg(feature = "lua")]
pub mod execute;
pub mod frames;
pub mod fs;
pub mod instance;
pub mod rcon;
pub mod scripts;
pub mod settings;

use crate::state::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn router() -> OpenApiRouter<AppState> {
    let router = OpenApiRouter::new()
        .routes(routes!(settings::get_settings))
        .routes(routes!(settings::put_settings))
        .routes(routes!(instance::get_instance))
        .routes(routes!(instance::start_instance))
        .routes(routes!(instance::stop_instance))
        .routes(routes!(rcon::send_rcon))
        .routes(routes!(scripts::list_scripts))
        .routes(routes!(scripts::read_script))
        .routes(routes!(scripts::write_script))
        .routes(routes!(scripts::create_script))
        .routes(routes!(scripts::delete_script))
        .routes(routes!(fs::exists))
        .routes(routes!(frames::list_frames))
        .routes(routes!(frames::get_frame));

    // Script execution and the job history it produces exist only in a build
    // that has an interpreter: `factorio-bot-scripting-lua` is an optional
    // dependency here, and `cargo build --no-default-features` (a precommit
    // gate) leaves it out entirely. Registering the routes anyway would publish
    // operations in the OpenAPI document that can answer nothing but an error.
    #[cfg(feature = "lua")]
    let router = router
        .routes(routes!(execute::post_execute))
        .routes(routes!(execute::list_jobs))
        .routes(routes!(execute::get_job))
        .routes(routes!(execute::job_events));

    router
}
