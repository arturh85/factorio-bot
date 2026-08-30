use crate::state::AppState;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use miette::{IntoDiagnostic, Result};
use std::net::SocketAddr;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

async fn health() -> &'static str {
    "ok"
}

/// Builds the router. `web_root` is passed in rather than read back out of
/// `state.settings`: that lock is the process-wide `SharedAppSettings` that
/// `update_settings` also writes to, so a `try_read()` here could lose the race
/// and silently build a router with no SPA. Callers read it once, `await`ing
/// the lock properly, and hand the value over.
pub fn build_router(state: AppState, web_root: Option<&str>) -> Router {
    let (router, api) = OpenApiRouter::with_openapi(crate::openapi::ApiDoc::openapi())
        .route("/api/v1/health", get(health))
        .merge(crate::game::router())
        .merge(crate::manage::router())
        .with_state(state)
        .split_for_parts();

    let router = router
        .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", api))
        // Unmatched API paths always answer with the JSON error body, whether or
        // not a frontend is deployed.
        .route("/api/v1/{*rest}", axum::routing::any(api_not_found));

    match crate::spa::service(web_root) {
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

pub async fn start_with_shutdown(
    settings: SharedAppSettings,
    instance_state: SharedFactorioInstance,
    bind: SocketAddr,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let web_root = settings.read().await.restapi.web_root.clone();
    let state = AppState {
        instance: instance_state.clone(),
        settings,
    };
    let app = build_router(state, web_root.as_deref());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .into_diagnostic()?;
    tracing::info!("listening on http://{bind}");
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .into_diagnostic();

    // FactorioInstance has no Drop impl, so a server that is signalled — or
    // one whose accept loop returns an error — would otherwise leave the
    // Factorio server and every client process orphaned. Stop it whether
    // `serve_result` is Ok or Err, then propagate the original error first.
    let stop_result = if let Some(instance) = instance_state.write().await.take() {
        tracing::info!("stopping factorio instance");
        instance.stop()
    } else {
        Ok(())
    };

    serve_result?;
    stop_result
}

pub async fn start(
    settings: SharedAppSettings,
    instance_state: SharedFactorioInstance,
    bind: SocketAddr,
) -> Result<()> {
    start_with_shutdown(settings, instance_state, bind, std::future::pending()).await
}
