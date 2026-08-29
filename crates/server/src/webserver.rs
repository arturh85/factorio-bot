use crate::settings::RestApiSettings;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use miette::{IntoDiagnostic, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

async fn health() -> &'static str {
    "ok"
}

pub fn build_router(state: AppState) -> Router {
    let (router, api) = OpenApiRouter::with_openapi(crate::openapi::ApiDoc::openapi())
        .route("/api/v1/health", get(health))
        .merge(crate::game::router())
        .with_state(state)
        .split_for_parts();

    router.merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", api))
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
