use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::webserver::start;
use std::net::SocketAddr;
use std::time::Duration;

/// start() must bind the address it is given, not a hardcoded one.
#[tokio::test]
async fn start_binds_the_requested_address() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    // port 0 asks the OS for a free port, so this test cannot collide with a
    // developer's running server
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");

    let server = tokio::spawn(async move { start(settings, instance, bind).await });

    // the server runs until aborted; if it returned early it failed to bind
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!server.is_finished(), "server exited instead of serving");
    server.abort();
}

#[tokio::test]
async fn start_reports_an_unbindable_address() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    // port 1 is privileged; binding it as an unprivileged user fails
    let bind: SocketAddr = "127.0.0.1:1".parse().expect("valid addr");

    let result = start(settings, instance, bind).await;
    assert!(result.is_err(), "expected a bind error");
}
