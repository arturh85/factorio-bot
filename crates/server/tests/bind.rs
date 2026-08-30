use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_core::settings::RestApiSettings;
use factorio_bot_server::webserver::start;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Asks the OS for a free port, then gives it straight back so `start()` can
/// take it. Passing `127.0.0.1:0` to `start()` instead would tell us nothing:
/// the address it actually bound would stay invisible to the test, so an
/// implementation that ignored `bind` entirely would still pass.
async fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port is available");
    let addr = listener.local_addr().expect("listener has a local address");
    drop(listener);
    addr
}

/// Speaks just enough HTTP/1.1 to prove the thing answering on `addr` is our
/// router, retrying while the spawned server is still coming up.
async fn get(addr: SocketAddr, path: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(addr).await {
            Ok(mut stream) => {
                let request =
                    format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
                stream
                    .write_all(request.as_bytes())
                    .await
                    .expect("request written");
                let mut response = Vec::new();
                stream
                    .read_to_end(&mut response)
                    .await
                    .expect("response read");
                return String::from_utf8_lossy(&response).into_owned();
            }
            Err(err) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "nothing accepted a connection on {addr}: {err}"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
}

/// start() must bind the address it is given, not a hardcoded one.
#[tokio::test]
async fn start_binds_the_requested_address() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    let bind = free_addr().await;

    let server = tokio::spawn(async move { start(settings, instance, bind).await });

    let response = get(bind, "/api/v1/health").await;
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "expected our router on {bind}, got: {response}"
    );
    assert!(
        response.ends_with("ok"),
        "expected the health body on {bind}, got: {response}"
    );

    assert!(!server.is_finished(), "server exited instead of serving");
    server.abort();
}

/// The web root reaches the router through `start()`, not only through the
/// directly-constructed router the spa tests use.
#[tokio::test]
async fn start_serves_the_web_root_from_the_shared_settings() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").expect("index written");
    let settings = AppSettings {
        restapi: RestApiSettings {
            port: 7492,
            web_root: Some(dir.path().to_string_lossy().into_owned()),
        },
        ..Default::default()
    }
    .into_shared();
    let instance = FactorioInstance::new_shared();
    let bind = free_addr().await;

    let server = tokio::spawn(async move { start(settings, instance, bind).await });

    let response = get(bind, "/").await;
    assert!(
        response.contains("<html>spa</html>"),
        "expected the spa index, got: {response}"
    );

    server.abort();
}

#[tokio::test]
async fn start_reports_an_unbindable_address() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    // Hold a listener open on the port and hand `start()` the same address.
    // Unlike a privileged port this also fails for root, so the test cannot
    // silently turn into a hang: `start()` delegates to
    // `start_with_shutdown(.., std::future::pending())`, so a successful bind
    // here would never return.
    let occupied = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port is available");
    let bind = occupied.local_addr().expect("listener has a local address");

    let result = tokio::time::timeout(Duration::from_secs(5), start(settings, instance, bind))
        .await
        .expect("start() returned instead of serving the occupied port");

    assert!(result.is_err(), "expected a bind error for {bind}");
    drop(occupied);
}
