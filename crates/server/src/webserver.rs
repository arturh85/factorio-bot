use crate::settings::RestApiSettings;
use crate::state::AppState;
use axum::response::Redirect;
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
    let web_root = state
        .settings
        .try_read()
        .ok()
        .and_then(|settings| settings.web_root.clone());

    let (router, api) = OpenApiRouter::with_openapi(crate::openapi::ApiDoc::openapi())
        .route("/api/v1/health", get(health))
        .merge(crate::game::router())
        .with_state(state)
        .split_for_parts();

    let router = router
        .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", api))
        // Unmatched API paths always answer with the JSON error body, whether or
        // not a frontend is deployed.
        .route("/api/v1/{*rest}", axum::routing::any(api_not_found));

    match crate::spa::service(web_root.as_deref()) {
        Some(spa) => router.fallback_service(spa),
        // Without a frontend `/` has nothing to serve, so point it at the docs.
        None => router.route("/", get(|| async { Redirect::temporary("/swagger-ui/") })),
    }
}

async fn api_not_found() -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = crate::error::ErrorResponse::new("not found".into(), 404);
    (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response()
}

pub async fn start(
    settings: RestApiSettings,
    instance_state: SharedFactorioInstance,
) -> Result<()> {
    let port = u16::try_from(settings.port).map_err(|_| {
        miette::miette!(
            "invalid restapi port {}: must be between 0 and 65535",
            settings.port
        )
    })?;
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
