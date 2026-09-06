use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::parking_lot;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use factorio_bot_server::webserver::start_with_shutdown;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, oneshot};

/// Grace period injected into these tests. `start_with_shutdown` no longer
/// hardcodes ten seconds — the caller passes it in, production via
/// `webserver::SHUTDOWN_GRACE_PERIOD` and tests via a value small enough
/// that a test which needs to wait the grace period out (or well past it)
/// does not have to sleep for ten real seconds to do so.
const TEST_GRACE_PERIOD: Duration = Duration::from_millis(300);

#[tokio::test]
async fn server_returns_when_the_shutdown_future_resolves() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let (tx, rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        start_with_shutdown(
            settings,
            factorio_bot_core::paths::settings_file(),
            instance,
            bind,
            async {
                let _ = rx.await;
            },
            TEST_GRACE_PERIOD,
        )
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
        world: Some(Arc::new(FactorioSurface::new())),
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
        start_with_shutdown(
            settings,
            factorio_bot_core::paths::settings_file(),
            instance_state,
            bind,
            async {
                let _ = rx.await;
            },
            TEST_GRACE_PERIOD,
        )
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
        start_with_shutdown(
            settings,
            factorio_bot_core::paths::settings_file(),
            instance,
            bind,
            async {
                let _ = rx.await;
            },
            TEST_GRACE_PERIOD,
        )
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

    // Grace period plus a generous margin. If the shutdown wait is
    // unbounded, this join times out and the test fails — that is the
    // guard.
    let test_timeout = TEST_GRACE_PERIOD + Duration::from_secs(5);
    let result = tokio::time::timeout(test_timeout, server)
        .await
        .expect("start_with_shutdown did not return within the grace period plus a margin")
        .expect("task did not panic");
    let elapsed = shutdown_started.elapsed();

    assert!(
        result.is_ok(),
        "shutdown should be a clean exit: {result:?}"
    );
    assert!(
        elapsed < TEST_GRACE_PERIOD + Duration::from_secs(2),
        "shutdown took {elapsed:?}, expected it bounded well within the {TEST_GRACE_PERIOD:?} grace period plus a margin"
    );

    // Keep the connection (and its stuck request) alive across the whole
    // shutdown wait; only drop it now that the assertions above are done.
    drop(stream);
}

/// Guards against the exact bug caught during this task's end-to-end check
/// and reported as a coverage gap: an earlier draft wrapped the *entire*
/// `serve().with_graceful_shutdown()` future in a fixed timeout, so the
/// server silently exited once the grace period elapsed *from process
/// start* — even though `shutdown` never resolved and nothing ever asked it
/// to stop. No other test in this file would have caught that: the other
/// three all send a shutdown signal quickly and only check what happens
/// after that.
///
/// This starts the server with a `shutdown` future that never resolves,
/// waits five times the (injected, short) grace period, and asserts the
/// server task is still running. Under the correct implementation — the
/// grace-period timer only starts once `shutdown` actually fires — this
/// passes easily. Under the naive whole-future-timeout shape, the server
/// would already have exited well before this assertion runs.
#[tokio::test]
async fn server_survives_past_the_grace_period_with_no_shutdown_signal() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    let bind = free_addr().await;

    let server = tokio::spawn(async move {
        start_with_shutdown(
            settings,
            factorio_bot_core::paths::settings_file(),
            instance,
            bind,
            std::future::pending(),
            TEST_GRACE_PERIOD,
        )
        .await
    });

    tokio::time::sleep(TEST_GRACE_PERIOD * 5).await;

    assert!(
        !server.is_finished(),
        "server exited on its own after the grace period elapsed, even though shutdown was never signalled — the grace period must only start counting once a shutdown signal actually fires"
    );

    server.abort();
}

/// Grace period for the SSE test below, deliberately *longer* than
/// `TEST_GRACE_PERIOD`.
///
/// The grace period is a backstop, not the mechanism: without any bound on the
/// streams themselves, `start_with_state` still returns — once the grace period
/// has fully elapsed. So a test that only asserted "returns within the grace
/// period plus a margin" would pass with the stream bound deleted, and would be
/// documentation rather than coverage. What separates the two is *how long* it
/// takes: bounded streams end the moment the signal fires and the drain
/// finishes in milliseconds, while an unbounded one burns the whole grace
/// period. A long grace period makes that gap wide enough to assert on without
/// the assertion being a stopwatch race on a loaded machine.
#[cfg(feature = "lua")]
const STREAMING_GRACE_PERIOD: Duration = Duration::from_secs(3);

/// `webserver.rs`'s own comment names this as the hazard: axum's graceful
/// drain waits for in-flight requests without bound, and an SSE stream for a
/// job that never finishes never ends on its own.
///
/// Asserts on elapsed time, not on a status: a hang is the failure mode, and
/// the difference between "bounded" and "saved only by the backstop" is
/// visible in nothing else.
#[cfg(feature = "lua")]
#[tokio::test]
async fn an_open_event_stream_does_not_hold_shutdown_past_the_grace_period() {
    use factorio_bot_scripting::{OutputSink, Stream};
    use factorio_bot_server::state::AppState;
    use factorio_bot_server::webserver::start_with_state;
    use tokio::io::AsyncReadExt;

    let settings = AppSettings::default().into_shared();
    let instance_state: SharedFactorioInstance =
        Arc::new(RwLock::new(Some(empty_factorio_instance())));
    let state = AppState::new(
        instance_state,
        settings,
        factorio_bot_core::paths::settings_file(),
    );

    // A job that never finishes: this handle is held for the whole test, so
    // nothing completes it and its broadcast channel stays open. (Dropping it
    // would complete the job as failed and close the stream for a reason that
    // has nothing to do with shutdown.)
    let running = state
        .jobs
        .try_start(Some("forever.lua".into()))
        .expect("the slot is free");
    let id = running.id();

    let bind = free_addr().await;
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        start_with_state(
            state,
            bind,
            async {
                let _ = rx.await;
            },
            STREAMING_GRACE_PERIOD,
        )
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
    let request = format!(
        "GET /api/v1/jobs/{id}/events HTTP/1.1\r\nHost: {bind}\r\nAccept: text/event-stream\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("request written");

    // Read until a real event has arrived. Waiting for the response *head*
    // alone would not prove the body is streaming; an event does, and it is
    // what makes this an in-flight request rather than an idle connection —
    // hyper tears idle connections down promptly on its own, so a test built
    // on one could not fail.
    let mut received = Vec::new();
    let read_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        running.line(Stream::Stdout, "tick");
        let mut buffer = [0_u8; 4096];
        match tokio::time::timeout(Duration::from_millis(100), stream.read(&mut buffer)).await {
            Ok(Ok(0)) => panic!("the server closed the stream before shutdown was signalled"),
            Ok(Ok(read)) => received.extend_from_slice(&buffer[..read]),
            Ok(Err(err)) => panic!("reading the event stream failed: {err}"),
            Err(_elapsed) => {}
        }
        if String::from_utf8_lossy(&received).contains("event: output") {
            break;
        }
        assert!(
            Instant::now() < read_deadline,
            "no event ever arrived on the stream: {:?}",
            String::from_utf8_lossy(&received)
        );
    }

    tx.send(()).expect("receiver alive");
    let shutdown_started = Instant::now();

    let result = tokio::time::timeout(STREAMING_GRACE_PERIOD + Duration::from_secs(5), server)
        .await
        .expect("start_with_state did not return at all")
        .expect("task did not panic");
    let elapsed = shutdown_started.elapsed();

    assert!(
        result.is_ok(),
        "shutdown should be a clean exit: {result:?}"
    );
    assert!(
        elapsed < STREAMING_GRACE_PERIOD / 2,
        "shutdown took {elapsed:?}: an open event stream must end on the shutdown signal, not sit \
         there until the {STREAMING_GRACE_PERIOD:?} grace period expires"
    );

    // Both held to the very end on purpose: dropping either earlier would end
    // the stream for a reason other than the one under test.
    drop(stream);
    drop(running);
}
