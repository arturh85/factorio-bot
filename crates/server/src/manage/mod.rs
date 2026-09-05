#[cfg(feature = "lua")]
pub mod execute;
pub mod fs;
pub mod instance;
pub mod rcon;
pub mod scripts;
pub mod settings;
pub mod video;

use crate::error::ErrorResponse;
use crate::state::AppState;
use std::path::PathBuf;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Resolves the workspace root this request's artefacts live under.
///
/// Mirrors `manage::scripts::scripts_root_path`: reads `workspace_path`
/// straight out of settings rather than going through
/// any CWD-relative fallback (the deleted `scripts::scripts_dir` had one), for the
/// same reason -- a request must be bound to the *configured* workspace, not
/// to wherever the server process's working directory happens to be.
///
/// Deliberately does not require the directory to exist, and does not
/// canonicalize it: "no workspace yet" is a state the callers represent
/// themselves, not an error, and canonicalizing a missing path would fail.
pub(crate) async fn workspace_root(state: &AppState) -> Result<PathBuf, ErrorResponse> {
    let workspace_path = state.settings.read().await.factorio.workspace_path.clone();
    factorio_bot_core::paths::resolve_workspace(workspace_path.as_ref())
        .map(|resolved| resolved.as_path().to_path_buf())
        .map_err(|err| ErrorResponse::bad_request(err.to_string()))
}

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
        .routes(routes!(video::get_video))
        .routes(routes!(video::get_video_ticks))
        .routes(routes!(video::get_video_file));

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
