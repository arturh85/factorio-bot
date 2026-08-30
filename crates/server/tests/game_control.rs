use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::process::process_control::FactorioInstance;
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

async fn post(uri: &str, body: &str) -> (StatusCode, String) {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn mutating_routes_report_not_started_without_instance() {
    for (uri, body) in [
        (
            "/api/v1/game/move-player",
            r#"{"player_id":1,"goal":"0,0"}"#,
        ),
        ("/api/v1/game/cheat-all-technologies", "{}"),
        ("/api/v1/game/server-save", "{}"),
        ("/api/v1/game/add-research", r#"{"tech":"automation"}"#),
    ] {
        let (status, response_body) = post(uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(
            response_body.contains("not started"),
            "{uri}: {response_body}"
        );
    }
}

/// direction > 7 hit `Direction::from_u8(...).unwrap()` in the Rocket version,
/// which aborts the process under `panic = "abort"`.
#[tokio::test]
async fn place_entity_rejects_out_of_range_direction() {
    let (status, body) = post(
        "/api/v1/game/place-entity",
        r#"{"player_id":1,"item":"stone-furnace","position":"0,0","direction":99}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("direction"), "got: {body}");
}

#[tokio::test]
async fn move_player_rejects_unparseable_goal() {
    let (status, body) = post(
        "/api/v1/game/move-player",
        r#"{"player_id":1,"goal":"not-a-position"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("goal") || body.contains("position"),
        "got: {body}"
    );
}

#[tokio::test]
async fn get_is_not_allowed_on_mutating_routes() {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .uri("/api/v1/game/server-save")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

/// A player id that is not in the world must be rejected before the rcon call.
/// Reaching rcon with an unknown id hits an unguarded unwrap in
/// player_path's error-recovery branch, which aborts the process in release.
#[tokio::test]
async fn mutating_routes_reject_an_unknown_player_before_calling_rcon() {
    let state = AppState {
        instance: Arc::new(RwLock::new(Some(FactorioInstance {
            world: Some(Arc::new(FactorioWorld::new())),
            rcon: Arc::new(FactorioRcon::new_empty()),
            server_process: None,
            client_processes: Vec::new(),
            silent: Arc::new(factorio_bot_core::parking_lot::RwLock::new(true)),
            server_host: None,
            server_port: None,
            rcon_port: 0,
            client_count: 0,
            map_exchange_string: None,
            seed: None,
        }))),
        settings: AppSettings::default().into_shared(),
    };

    for (uri, body) in [
        (
            "/api/v1/game/move-player",
            r#"{"player_id":42,"goal":"0,0"}"#,
        ),
        (
            "/api/v1/game/place-entity",
            r#"{"player_id":42,"item":"stone-furnace","position":"0,0","direction":0}"#,
        ),
    ] {
        let response = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("player"), "{uri} returned: {text}");
    }
}
