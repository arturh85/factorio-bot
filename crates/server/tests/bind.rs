use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_core::settings::RestApiSettings;
use factorio_bot_server::webserver::{BindFailed, start};
use miette::Result;
use std::future::Future;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Asks the OS for a free port, then gives it straight back so the server can
/// take it. Passing `127.0.0.1:0` to `start()` instead would tell us nothing:
/// the address it actually bound would stay invisible to the test, so an
/// implementation that ignored `bind` entirely would still pass.
///
/// The port is owned by nobody between the `drop` here and the server's own
/// `bind`, so anything on the machine can take it in that window -- including
/// this binary's own sibling tests, which run concurrently and each call this.
/// Measured on this box: with only a `tokio::spawn` hop in the window, 1-2 of
/// 8,000 handouts were lost to an unrelated taker; the real window is wider,
/// because `start()` reads the shared settings and builds the whole router
/// before it binds. Losing it makes the server fail with `Address already in
/// use`, after which the test either sees `Connection refused` for its whole
/// deadline or -- if the taker listens but never answers -- hangs forever.
/// Both were reproduced deliberately.
///
/// So this is never called directly by a test: [`serving`] wraps it and
/// retries, which is what closes the flake without weakening what the tests
/// prove. Each attempt still hands the server one concrete address and
/// demands an answer on exactly that address.
async fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port is available");
    let addr = listener.local_addr().expect("listener has a local address");
    drop(listener);
    addr
}

/// How many ports we are willing to lose before calling it a defect rather
/// than bad luck. At the measured loss rate this is never reached; it is a
/// bound so that a genuinely unbindable environment fails the test instead of
/// spinning.
const ACQUISITION_ATTEMPTS: usize = 8;

/// Starts a server on a free address and returns once something is listening
/// there, retrying when the address was taken out from under us.
///
/// `launch` is handed the address the server must bind, and is called afresh
/// for each attempt.
///
/// **Losing the race and a broken server must not be the same answer**, and
/// the difference is taken from the server's own error: [`BindFailed`] with
/// an `ErrorKind::AddrInUse` source means another socket held the address, so
/// try a different port; anything else fails the test with that error. Every
/// other failure -- the launch returning `Ok`, or an error that is not a bind
/// failure -- is reported, never retried away.
///
/// Two cheaper-looking distinctions were tried and are wrong, so do not go
/// back to them. Matching the error's *message* is not a fact: an OS error
/// string is neither stable nor locale-independent, and before `BindFailed`
/// existed it was all there was, because `into_diagnostic()` erases the
/// concrete type and `downcast_ref::<std::io::Error>()` on such a report is
/// `None`. Re-binding the address *after* the failure to see whether it is
/// still taken is not a fact either, and that one is worse because it looks
/// like one: it was measured wrong 7 times in 60 runs, every time on a
/// genuine lost race whose taker had released the port again by the time the
/// probe ran, reported as "the server failed while the address was free".
///
/// The one thing this cannot make deterministic: a taker that both listens
/// and answers could satisfy the connect probe below while our own launch is
/// still on its way to failing. That is checked rather than assumed away --
/// the task is asked again after a successful connect, and a lost bind
/// resolves it long before a connect round trip -- and if it ever did slip
/// through, each caller's own assertions (the response body, and
/// `!server.is_finished()`) reject an impostor. It can therefore cost a loud
/// failure, never a silent pass.
async fn serving<L, F>(mut launch: L) -> (SocketAddr, JoinHandle<Result<()>>)
where
    L: FnMut(SocketAddr) -> F,
    F: Future<Output = Result<()>> + Send + 'static,
{
    let mut lost = Vec::new();
    for attempt in 1..=ACQUISITION_ATTEMPTS {
        let addr = free_addr().await;
        let mut server = tokio::spawn(launch(addr));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if server.is_finished() {
                let outcome = (&mut server).await.expect("launch task did not panic");
                let taken = outcome.as_ref().err().is_some_and(|report| {
                    report
                        .downcast_ref::<BindFailed>()
                        .is_some_and(|failed| failed.source.kind() == ErrorKind::AddrInUse)
                });
                assert!(
                    taken,
                    "the server on {addr} stopped for a reason that is not a lost port, so \
                     retrying would only hide it: {outcome:?}"
                );
                lost.push(format!("attempt {attempt}: {addr} was already taken"));
                break;
            }
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    drop(stream);
                    // Something is listening -- but is it ours? A taker that
                    // won the race *and* listens answers this connect just as
                    // our own server would, while our launch is off failing.
                    // So ask the task again instead of assuming: a lost bind
                    // resolves it long before this connect finished its round
                    // trip, and if it has, the next pass classifies it as a
                    // lost race and tries another port. Without this recheck
                    // the helper hands the caller an impostor's address and
                    // the failure surfaces later, somewhere else -- which is
                    // exactly what a deliberate listening squatter produced
                    // while this helper was being written.
                    if !server.is_finished() {
                        return (addr, server);
                    }
                }
                Err(err) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "no server ever came up on {addr} (attempt {attempt}): {err}"
                    );
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
        }
    }
    panic!("lost the port {ACQUISITION_ATTEMPTS} times running: {lost:#?}");
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

    let (bind, server) = serving(move |addr| {
        let settings = settings.clone();
        let instance = instance.clone();
        async move {
            start(
                settings,
                factorio_bot_core::paths::settings_file(),
                instance,
                addr,
            )
            .await
        }
    })
    .await;

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

    let (bind, server) = serving(move |addr| {
        let settings = settings.clone();
        let instance = instance.clone();
        async move {
            start(
                settings,
                factorio_bot_core::paths::settings_file(),
                instance,
                addr,
            )
            .await
        }
    })
    .await;

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

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        start(
            settings,
            factorio_bot_core::paths::settings_file(),
            instance,
            bind,
        ),
    )
    .await
    .expect("start() returned instead of serving the occupied port");

    assert!(result.is_err(), "expected a bind error for {bind}");
    drop(occupied);
}
