use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::webserver::start_with_shutdown;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::oneshot;

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
