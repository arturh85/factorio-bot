use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

/// Points `workspace_path` at a temp dir holding a `scripts/` directory, so
/// these tests never touch the developer's real scripts. `settings_path`
/// points at a throwaway file inside the same temp dir: nothing here
/// exercises `PUT /api/v1/settings`, but `AppState`'s fields are all
/// required, and pointing it at a real settings file would risk clobbering
/// one if a future test here ever did touch persistence.
fn state_with_scripts(dir: &std::path::Path) -> AppState {
    std::fs::create_dir_all(dir.join("scripts").join("sub")).expect("mkdir");
    std::fs::write(dir.join("scripts").join("hello.lua"), "-- hello").expect("write");
    let mut settings = AppSettings::default();
    settings.factorio.workspace_path = dir.to_string_lossy().into_owned().into();
    AppState {
        instance: FactorioInstance::new_shared(),
        settings: settings.into_shared(),
        settings_path: dir.join("AppSettings.toml"),
    }
}

async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
    let response = build_router(state, None)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn lists_the_scripts_directory() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = get(state_with_scripts(dir.path()), "/api/v1/scripts?path=/").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("hello.lua"), "{body}");
    assert!(body.contains("\"leaf\":true"), "{body}");
}

#[tokio::test]
async fn reads_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = get(
        state_with_scripts(dir.path()),
        "/api/v1/scripts/file?path=/hello.lua",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("-- hello"), "{body}");
}

/// The old handler byte-sliced the path with `&path[1..]`, which panics here.
#[tokio::test]
async fn an_empty_path_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let (status, _body) = get(state_with_scripts(dir.path()), "/api/v1/scripts/file?path=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn traversal_outside_the_scripts_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), "top secret").expect("write");
    let (status, body) = get(
        state_with_scripts(dir.path()),
        "/api/v1/scripts/file?path=/../secret.txt",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.contains("top secret"), "leaked file contents: {body}");
}

#[tokio::test]
async fn writes_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/scripts/file?path=/hello.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- replaced"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    let written = std::fs::read_to_string(dir.path().join("scripts").join("hello.lua")).unwrap();
    assert_eq!(written, "-- replaced");
}

#[tokio::test]
async fn creates_a_new_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/scripts/file?path=/fresh.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- fresh"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    let written = std::fs::read_to_string(dir.path().join("scripts").join("fresh.lua")).unwrap();
    assert_eq!(written, "-- fresh");
}

#[tokio::test]
async fn refuses_to_create_over_an_existing_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/scripts/file?path=/hello.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- clobber"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let kept = std::fs::read_to_string(dir.path().join("scripts").join("hello.lua")).unwrap();
    assert_eq!(kept, "-- hello", "existing script must not be overwritten");
}

/// The parent resolves legitimately; the escape is in the final component.
#[tokio::test]
async fn creating_cannot_escape_via_the_final_component() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    for attempt in [
        "/sub/../../escaped.lua",
        "/../escaped.lua",
        "/sub/..%2F..%2Fescaped.lua",
    ] {
        let response = build_router(state.clone(), None)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/scripts/file?path={attempt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"code":"-- escaped"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{attempt}");
    }
    assert!(
        !dir.path().join("escaped.lua").exists(),
        "a file escaped the scripts root"
    );
}

#[tokio::test]
async fn deletes_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/scripts/file?path=/hello.lua")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    assert!(!dir.path().join("scripts").join("hello.lua").exists());
}

#[tokio::test]
async fn deleting_a_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/scripts/file?path=/sub")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(dir.path().join("scripts").join("sub").exists());
}

/// `target.exists()` follows symlinks and reports `false` for a *dangling*
/// one, and `File::create` (what `std::fs::write` uses) also follows
/// symlinks when opening. A dangling symlink planted inside the scripts
/// root, pointing at a path outside it that does not exist yet, would
/// therefore pass an `exists()` guard and then have its target created and
/// written by a naive `fs::write` — landing the new file's contents
/// outside the scripts root through a name that was never itself resolved
/// or bounds-checked. `create_script` must refuse this outright rather
/// than writing through the symlink.
#[cfg(unix)]
#[tokio::test]
async fn creating_over_a_dangling_symlink_does_not_write_through_it() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(
        outside.join("evil.lua"),
        dir.path().join("scripts").join("planted.lua"),
    )
    .unwrap();

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/scripts/file?path=/planted.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- pwned"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        !outside.join("evil.lua").exists(),
        "wrote through a dangling symlink to outside the scripts root"
    );
}
