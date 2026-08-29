use crate::settings::RestApiSettings;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use miette::{IntoDiagnostic, Result};
use std::sync::Arc;
use tokio::sync::RwLock;

async fn health() -> &'static str {
    "ok"
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .nest("/api/v1/game", crate::game::router())
        .with_state(state)
}

pub async fn start(
    settings: RestApiSettings,
    instance_state: SharedFactorioInstance,
) -> Result<()> {
    let port = settings.port as u16;
    let state = AppState {
        instance: instance_state,
        settings: Arc::new(RwLock::new(settings)),
    };
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .into_diagnostic()?;
    tracing::info!("restapi listening on http://127.0.0.1:{port}");
    axum::serve(listener, app).await.into_diagnostic()?;
    Ok(())
}
