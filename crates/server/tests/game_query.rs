use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_server::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings::default())),
    }
}

async fn get(uri: &str) -> (StatusCode, String) {
    let response = build_router(test_state())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Rocket mapped absent optional params to None; axum needs #[serde(default)]
/// or this returns "missing field" instead of reaching the handler.
#[tokio::test]
async fn find_entities_without_any_params_reports_missing_area() {
    let (status, body) = get("/api/v1/game/find-entities").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("area or position"),
        "expected the handler's own error, got: {body}"
    );
}

#[tokio::test]
async fn find_entities_with_unparseable_area_does_not_panic() {
    let (status, body) = get("/api/v1/game/find-entities?area=garbage").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("area"), "got: {body}");
}

#[tokio::test]
async fn find_entities_without_running_instance_reports_not_started() {
    let (status, body) = get("/api/v1/game/find-entities?position=0,0&radius=10").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("not started"), "got: {body}");
}

#[tokio::test]
async fn read_routes_report_not_started_without_instance() {
    for uri in [
        "/api/v1/game/find-tiles?position=0,0&radius=10",
        "/api/v1/game/player-info?player_id=1",
        "/api/v1/game/all-players",
        "/api/v1/game/item-prototypes",
        "/api/v1/game/entity-prototypes",
    ] {
        let (status, body) = get(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(body.contains("not started"), "{uri} returned: {body}");
    }
}

#[tokio::test]
async fn inventory_contents_at_rejects_query_without_separator() {
    let (status, body) = get("/api/v1/game/inventory-contents-at?query=nonsense").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.is_empty());
}

#[tokio::test]
async fn player_info_rejects_out_of_range_player_id() {
    let (status, _body) = get("/api/v1/game/player-info?player_id=300").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
