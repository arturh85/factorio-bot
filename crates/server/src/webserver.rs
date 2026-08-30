use crate::state::AppState;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use miette::{IntoDiagnostic, Result};
use std::net::SocketAddr;
use std::time::Duration;
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
    let state = AppState::new(instance_state.clone(), settings);
    let app = build_router(state, web_root.as_deref());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .into_diagnostic()?;
    tracing::info!("listening on http://{bind}");

    // `with_graceful_shutdown` waits for every in-flight request to finish
    // on its own, with no bound, once `shutdown` resolves. That is harmless
    // today, but plan 4 adds SSE streams that never end by themselves, at
    // which point this would wait forever after a shutdown signal — and a
    // second Ctrl-C cannot help, because `tokio::signal` has already
    // replaced the process's default signal disposition for its lifetime.
    //
    // The grace period has to start counting from the moment `shutdown`
    // resolves, not from process start: naively wrapping the whole
    // `serve().with_graceful_shutdown()` future in a fixed timeout would
    // silently kill the server after the grace period elapses even when
    // nothing ever asked it to shut down (this was caught by manually
    // running `serve` and watching it exit on its own after 10s with no
    // signal sent). So `shutdown` is fanned out through a `watch` channel to
    // two consumers: the signal axum's graceful drain waits on, and a
    // second branch whose grace-period sleep only starts once that same
    // signal has actually fired.
    const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(10);
    let (fired_tx, fired_rx) = tokio::sync::watch::channel(false);
    let mut fired_rx_for_grace = fired_rx.clone();
    let mut fired_rx_for_axum = fired_rx;
    tokio::spawn(async move {
        shutdown.await;
        // The receivers are always alive for the lifetime of this function,
        // so the only way `send` fails is if this task outlives the
        // function — harmless either way.
        let _ = fired_tx.send(true);
    });

    let axum_shutdown_signal = async move {
        let _ = fired_rx_for_axum.changed().await;
    };
    let serve_future = axum::serve(listener, app).with_graceful_shutdown(axum_shutdown_signal);

    let serve_result = tokio::select! {
        result = serve_future => result.into_diagnostic(),
        () = async {
            let _ = fired_rx_for_grace.changed().await;
            tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;
        } => {
            tracing::warn!(
                "graceful shutdown grace period ({SHUTDOWN_GRACE_PERIOD:?}) elapsed; abandoning in-flight requests"
            );
            Ok(())
        }
    };

    // FactorioInstance has no Drop impl, so a server that is signalled — or
    // one whose accept loop returns an error — would otherwise leave the
    // Factorio server and every client process orphaned. Stop it whether
    // `serve_result` is Ok, Err, or timed out (never skip this block — that
    // exact bug was already fixed once on the error path).
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
