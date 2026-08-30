use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: AppSettings::default().into_shared(),
    }
}

#[tokio::test]
async fn openapi_json_lists_every_route() {
    let response = build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let paths = spec["paths"].as_object().expect("paths object");

    for path in [
        "/api/v1/game/find-entities",
        "/api/v1/game/find-tiles",
        "/api/v1/game/inventory-contents-at",
        "/api/v1/game/player-info",
        "/api/v1/game/all-players",
        "/api/v1/game/item-prototypes",
        "/api/v1/game/entity-prototypes",
        "/api/v1/game/plan-path",
        "/api/v1/game/move-player",
        "/api/v1/game/place-entity",
        "/api/v1/game/cheat-item",
        "/api/v1/game/cheat-technology",
        "/api/v1/game/cheat-all-technologies",
        "/api/v1/game/insert-to-inventory",
        "/api/v1/game/remove-from-inventory",
        "/api/v1/game/server-save",
        "/api/v1/game/add-research",
    ] {
        assert!(paths.contains_key(path), "spec is missing {path}");
    }
}

#[tokio::test]
async fn swagger_ui_is_served() {
    let response = build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .uri("/swagger-ui/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success() || response.status().is_redirection(),
        "got {}",
        response.status()
    );
}
