use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::parking_lot;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use factorio_bot_server::webserver::start_with_shutdown;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, RwLock};

#[tokio::test]
async fn server_returns_when_the_shutdown_future_resolves() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let (tx, rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        start_with_shutdown(settings, instance, bind, async {
            let _ = rx.await;
        })
        .await
    });

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!server.is_finished(), "server exited before shutdown");

    tx.send(()).expect("receiver alive");

    let result = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server shut down within 5s")
        .expect("task did not panic");
    assert!(
        result.is_ok(),
        "shutdown should be a clean exit: {result:?}"
    );
}

/// A `FactorioInstance` that owns no real child processes: no server or
/// client `InteractiveProcess`, so `FactorioInstance::stop` has nothing to
/// kill and cannot hang or fail. This is what makes the instance
/// constructible in a unit test without spawning Factorio.
fn empty_factorio_instance() -> FactorioInstance {
    FactorioInstance {
        world: Some(Arc::new(FactorioWorld::new())),
        rcon: Arc::new(FactorioRcon::new_empty()),
        server_process: None,
        client_processes: Vec::new(),
        silent: Arc::new(parking_lot::RwLock::new(true)),
        server_host: None,
        server_port: None,
        rcon_port: 0,
        client_count: 0,
        map_exchange_string: None,
        seed: None,
    }
}

#[tokio::test]
async fn shutdown_takes_and_stops_the_factorio_instance() {
    let settings = AppSettings::default().into_shared();
    let instance_state: SharedFactorioInstance =
        Arc::new(RwLock::new(Some(empty_factorio_instance())));
    let instance_state_for_assertion = instance_state.clone();

    let bind: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let (tx, rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        start_with_shutdown(settings, instance_state, bind, async {
            let _ = rx.await;
        })
        .await
    });

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        instance_state_for_assertion.read().await.is_some(),
        "instance should still be running before shutdown"
    );

    tx.send(()).expect("receiver alive");

    let result = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server shut down within 5s")
        .expect("task did not panic");
    assert!(
        result.is_ok(),
        "shutdown should be a clean exit: {result:?}"
    );

    assert!(
        instance_state_for_assertion.read().await.is_none(),
        "shutdown should take and stop the factorio instance"
    );
}

/// Asks the OS for a free port, then gives it straight back so `start_with_shutdown`
/// can bind it — mirrors `bind.rs`'s helper of the same name (each integration
/// test file is its own crate, so it cannot be shared directly).
async fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port is available");
    let addr = listener.local_addr().expect("listener has a local address");
    drop(listener);
    addr
}

/// A request that never completes must not hold shutdown open forever — plan 4
/// adds SSE streams that never end on their own.
///
/// This drives a genuinely *stuck* request rather than falling back to an
/// idle keep-alive connection: an idle connection (one where the client has
/// received its response and is merely holding the socket open) was tried
/// first and did *not* reproduce a hang — hyper's graceful shutdown already
/// tears down idle-but-unused connections promptly, so a test built on that
/// shape passed even with no bound in place at all, i.e. it could not fail
/// and would not have been a real guard.
///
/// Instead: open a connection and send a `POST /api/v1/rcon` whose
/// `Content-Length` header is much larger than the body bytes actually
/// written, then never send the rest and never close the connection. The
/// `Json` body extractor blocks awaiting the remaining declared bytes, so
/// axum considers this request genuinely *in-flight* — not idle — for as
/// long as the socket stays open. `with_graceful_shutdown` is documented to
/// wait for exactly this kind of in-flight work to finish, which it never
/// will here, so this is the real shape the plan-4 SSE streams will take.
#[tokio::test]
async fn shutdown_does_not_wait_forever_for_an_in_flight_request() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    let bind = free_addr().await;
    let (tx, rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        start_with_shutdown(settings, instance, bind, async {
            let _ = rx.await;
        })
        .await
    });

    let connect_deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match TcpStream::connect(bind).await {
            Ok(stream) => break stream,
            Err(err) => {
                assert!(
                    Instant::now() < connect_deadline,
                    "nothing accepted a connection on {bind}: {err}"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    };
    let headers = "POST /api/v1/rcon HTTP/1.1\r\n\
        Host: placeholder\r\n\
        Content-Type: application/json\r\n\
        Content-Length: 1000000\r\n\r\n"
        .replace("placeholder", &bind.to_string());
    stream
        .write_all(headers.as_bytes())
        .await
        .expect("headers written");
    // A partial body: well short of the declared Content-Length, and never
    // followed by the rest.
    stream
        .write_all(b"{\"command\":\"")
        .await
        .expect("partial body written");

    // Give the server a moment to accept the connection and start (and
    // block on) reading the body before we signal shutdown.
    tokio::time::sleep(Duration::from_millis(200)).await;

    tx.send(()).expect("receiver alive");
    let shutdown_started = Instant::now();

    // Grace period (10s) plus a margin. If the shutdown wait is unbounded,
    // this join times out and the test fails — that is the guard.
    let result = tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .expect("start_with_shutdown did not return within the grace period plus a margin")
        .expect("task did not panic");
    let elapsed = shutdown_started.elapsed();

    assert!(
        result.is_ok(),
        "shutdown should be a clean exit: {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(13),
        "shutdown took {elapsed:?}, expected it bounded well within the 10s grace period plus a margin"
    );

    // Keep the connection (and its stuck request) alive across the whole
    // shutdown wait; only drop it now that the assertions above are done.
    drop(stream);
}
