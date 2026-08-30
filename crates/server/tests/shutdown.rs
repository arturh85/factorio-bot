use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::parking_lot;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use factorio_bot_server::webserver::start_with_shutdown;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
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
