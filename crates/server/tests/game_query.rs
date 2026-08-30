use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState::new(
        Arc::new(RwLock::new(None)),
        AppSettings::default().into_shared(),
    )
}

async fn get(uri: &str) -> (StatusCode, String) {
    let response = build_router(test_state(), None)
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

/// `player_id` is required: with `#[serde(default)]` a missing key silently
/// became player 0 and returned somebody else's data.
#[tokio::test]
async fn player_info_without_player_id_is_rejected() {
    let (status, body) = get("/api/v1/game/player-info").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !body.contains("not started"),
        "expected the missing-parameter rejection, got: {body}"
    );
}

/// `query` is required too, for the same reason.
#[tokio::test]
async fn inventory_contents_at_without_query_is_rejected() {
    let (status, body) = get("/api/v1/game/inventory-contents-at").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !body.contains("not started"),
        "expected the missing-parameter rejection, got: {body}"
    );
}

#[tokio::test]
async fn player_info_rejects_out_of_range_player_id() {
    let (status, _body) = get("/api/v1/game/player-info?player_id=300").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

const PLAN_PATH_PARAMS: &str = "entity_name=transport-belt&entity_type=transport-belt&underground_entity_name=underground-belt&underground_entity_type=underground-belt&underground_max=4&from_position=0,0&to_position=10,10";

/// to_direction > 7 hit `Direction::from_u8(...).unwrap()` in the Rocket version,
/// which aborts the process under `panic = "abort"`.
#[tokio::test]
async fn plan_path_rejects_out_of_range_direction() {
    let (status, body) = get(&format!(
        "/api/v1/game/plan-path?{PLAN_PATH_PARAMS}&to_direction=99"
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("direction"), "got: {body}");
}

#[tokio::test]
async fn plan_path_rejects_unparseable_position() {
    let (status, body) = get(
        "/api/v1/game/plan-path?entity_name=transport-belt&entity_type=transport-belt&\
         underground_entity_name=underground-belt&underground_entity_type=underground-belt&\
         underground_max=4&from_position=not-a-position&to_position=10,10&to_direction=0",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("from_position"), "got: {body}");
}

#[tokio::test]
async fn plan_path_without_running_instance_reports_not_started() {
    let (status, body) = get(&format!(
        "/api/v1/game/plan-path?{PLAN_PATH_PARAMS}&to_direction=0"
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("not started"), "got: {body}");
}

#[tokio::test]
async fn malformed_query_parameters_return_json() {
    let response = build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .uri("/api/v1/game/find-entities?position=0,0&radius=notanumber")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("has a content type")
        .to_str()
        .expect("utf-8");
    assert!(
        content_type.starts_with("application/json"),
        "expected JSON, got {content_type}"
    );
}
