use crate::paths;
use crate::settings::{
  SettingsOverrides, SharedAppSettings, load_app_settings_with, resolved_settings_path,
};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

pub type SharedJoinShandle<T> = Arc<RwLock<Option<JoinHandle<T>>>>;
pub type SharedRestApiHandle = SharedJoinShandle<Result<()>>;

#[derive(Clone)]
pub struct Context {
  pub instance_state: SharedFactorioInstance,
  pub app_settings: SharedAppSettings,
  pub restapi_handle: SharedRestApiHandle,
  /// The file `app_settings` was loaded from -- `overrides.settings_path` if
  /// one was given, otherwise the data-dir default. Every command that turns
  /// around and persists settings (currently just `PUT /api/v1/settings` via
  /// `serve`) must write back to this path, not re-derive its own: that is
  /// exactly the bug this field exists to close, where the server read the
  /// named `--settings` file but saved to the hardcoded default.
  pub settings_path: PathBuf,
}

impl Context {
  pub fn new(overrides: &SettingsOverrides) -> Result<Self> {
    color_eyre::install().expect("failed to colorize panics");
    // A `tracing` event goes nowhere unless a subscriber is installed, and
    // silently: the macro still compiles and still runs. Until this existed
    // the only installer was `console_subscriber` behind the non-default
    // `tokio-console` feature, so in every ordinary build all seven
    // `tracing::` call sites in `crates/server` -- including two error paths
    // and the "listening on" line that names the bind address -- printed
    // nothing at all. `serve.rs` prints its own "serving http://..." through
    // `paris`, which is how the silence stayed invisible.
    //
    // stderr, not stdout: `paris` writes the user-facing narration to stdout
    // and two tests capture it, so diagnostics go to the other stream rather
    // than interleaving with output someone is parsing.
    //
    // `try_init` rather than `init` because a second call must not abort the
    // process -- only one global subscriber can be set, and a test harness
    // that builds two `Context`s is not an error.
    #[cfg(feature = "tokio-console")]
    {
      // `console_subscriber::init()` binds a fixed TCP port on a background
      // thread and, if the bind fails (`AddrInUse` -- the exact case where a
      // viewer build and a game-run build of this same binary are both
      // started with this feature on), `.expect()`s. This workspace's
      // `[profile.release]` sets `panic = "abort"`, so that `.expect` does
      // not just kill its own thread -- it takes down the whole process,
      // silently as far as anything watching the game is concerned, with a
      // message naming `console-subscriber` and nothing about Factorio.
      // Debug tooling must not be able to kill the thing it observes, so the
      // port is probed here first and a failure degrades to a warning plus
      // the same plain `tracing` setup the non-console build uses, instead
      // of reaching console_subscriber's own `.expect` at all.
      //
      // The probe resolves the address exactly as `console_subscriber`'s own
      // `Builder::with_default_env` does -- `TOKIO_CONSOLE_BIND` (a
      // `HOST:PORT` string) if set, otherwise `127.0.0.1:6669` -- so setting
      // `TOKIO_CONSOLE_BIND=host:port` before running a second build is the
      // documented way to let both keep tokio-console instead of one losing
      // it. There is an inherent, unavoidable race between releasing the
      // probe socket and console_subscriber binding its own: that only
      // matters if a *third* party grabs the exact same port in that gap,
      // not for the two-builds-of-this-binary case this exists to fix.
      use console_subscriber::Server;
      use std::net::{SocketAddr, TcpListener, ToSocketAddrs};

      let addr = std::env::var("TOKIO_CONSOLE_BIND")
        .ok()
        .and_then(|bind| {
          bind
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
        })
        .unwrap_or_else(|| SocketAddr::new(Server::DEFAULT_IP, Server::DEFAULT_PORT));

      match TcpListener::bind(addr) {
        Ok(probe) => {
          drop(probe);
          console_subscriber::init();
        }
        Err(err) => {
          eprintln!(
            "warning: tokio-console could not bind {addr} ({err}); continuing without it. \
             Set TOKIO_CONSOLE_BIND=host:port to run two instrumented builds at once, \
             or stop whichever process already holds that port."
          );
          init_plain_tracing();
        }
      }
    }
    #[cfg(not(feature = "tokio-console"))]
    {
      init_plain_tracing();
    }

    create_dir_all(paths::data_local_dir()).into_diagnostic()?;
    create_dir_all(paths::workspace_dir()).into_diagnostic()?;

    let (app_settings, settings_path) = Self::resolve(overrides)?;

    let context = Context {
      instance_state: FactorioInstance::new_shared(),
      restapi_handle: Arc::new(RwLock::new(None)),
      app_settings,
      settings_path,
    };

    Ok(context)
  }

  /// The settings-loading half of [`Context::new`], split out so it can be
  /// exercised without `new`'s `create_dir_all` calls -- which always touch
  /// the real data directory, even when `--settings` points elsewhere --
  /// ever running in a test.
  fn resolve(overrides: &SettingsOverrides) -> Result<(SharedAppSettings, PathBuf)> {
    let settings_path = resolved_settings_path(overrides);
    let app_settings = load_app_settings_with(overrides)?.into_shared();
    Ok((app_settings, settings_path))
  }
}

/// The `tracing` diagnostics subscriber used whenever tokio-console is not
/// running the show: on every build without the `tokio-console` feature, and
/// as the fallback when that feature is on but its port could not be bound.
/// stderr, not stdout -- see [`Context::new`]'s doc comment on why the two
/// logging systems are kept apart. `try_init` rather than `init` because a
/// second call must not abort the process -- only one global subscriber can
/// be set, and a test harness that builds two `Context`s is not an error.
fn init_plain_tracing() {
  use tracing_subscriber::EnvFilter;
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
  let _ = tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_writer(std::io::stderr)
    .try_init();
}

#[cfg(all(test, feature = "restapi"))]
mod tests {
  use super::*;
  use axum::body::Body;
  use axum::http::{Request, StatusCode, header};
  use factorio_bot_core::app_settings::AppSettings;
  use factorio_bot_server::state::AppState;
  use factorio_bot_server::webserver::build_router;
  use tower::ServiceExt;

  /// Builds exactly what `serve.rs` builds in production: `Context`'s
  /// settings-loading half feeding `AppState::new` the same `app_settings`
  /// and `settings_path`. Deliberately does not call `Context::new` --
  /// its `create_dir_all` calls touch the real data directory regardless of
  /// `--settings`, which a test must never do.
  fn app_state_for(overrides: &SettingsOverrides) -> AppState {
    let (app_settings, settings_path) = Context::resolve(overrides).expect("settings resolve");
    AppState::new(FactorioInstance::new_shared(), app_settings, settings_path)
  }

  /// Test 1: `GET /api/v1/settings` with an explicit `--settings` path
  /// reports that file's values, not the data directory's.
  #[tokio::test]
  async fn get_settings_with_an_explicit_path_reports_that_files_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("AppSettings.toml");
    std::fs::write(&path, "[factorio]\nrcon_port = 55155\n").expect("writes settings file");

    let overrides = SettingsOverrides {
      settings_path: Some(path),
      ..SettingsOverrides::default()
    };
    let state = app_state_for(&overrides);

    let response = build_router(state, None)
      .oneshot(
        Request::builder()
          .uri("/api/v1/settings")
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
      .await
      .unwrap();
    let settings: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
      settings["factorio"]["rcon_port"], 55155,
      "expected the explicit file's value, got {settings}"
    );
  }

  /// Test 2: `PUT /api/v1/settings` with an explicit `--settings` path
  /// writes to that file, and a stand-in for the data-dir file is left
  /// completely untouched. Both halves matter -- "wrote the right file" and
  /// "did not write the wrong one" -- because the second is the bug: today
  /// `AppState::new` hardcodes the data-dir path regardless of what was
  /// loaded.
  ///
  /// The "data-dir file" is a second, never-referenced tempfile path rather
  /// than the real `paths::settings_file()`: this suite must never write to
  /// (or even risk creating) a developer's actual settings file.
  #[tokio::test]
  async fn put_settings_with_an_explicit_path_writes_only_that_file() {
    let explicit_dir = tempfile::tempdir().expect("tempdir");
    let explicit_path = explicit_dir.path().join("AppSettings.toml");
    std::fs::write(&explicit_path, "[factorio]\nrcon_port = 1111\n").expect("writes");

    let decoy_dir = tempfile::tempdir().expect("tempdir");
    let decoy_path = decoy_dir.path().join("AppSettings.toml");

    let overrides = SettingsOverrides {
      settings_path: Some(explicit_path.clone()),
      ..SettingsOverrides::default()
    };
    let state = app_state_for(&overrides);
    assert_eq!(
      state.settings_path, explicit_path,
      "AppState must be wired to the resolved --settings path, not a hardcoded default"
    );

    let mut updated = AppSettings::default();
    updated.factorio.client_count = 7;
    let body = serde_json::to_string(&updated).unwrap();

    let response = build_router(state, None)
      .oneshot(
        Request::builder()
          .method("PUT")
          .uri("/api/v1/settings")
          .header(header::CONTENT_TYPE, "application/json")
          .body(Body::from(body))
          .unwrap(),
      )
      .await
      .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());

    let persisted = AppSettings::load(explicit_path).expect("wrote the explicit file");
    assert_eq!(persisted.factorio.client_count, 7);
    assert!(
      !decoy_path.exists(),
      "PUT must never write anywhere but the resolved --settings path"
    );
  }

  /// Test 3: with no `--settings`, the resolved path -- what both routes end
  /// up wired to -- is still the data-dir default. Asserted on the pure
  /// resolution rule rather than by loading it, so this never touches the
  /// real data directory.
  #[test]
  fn with_no_settings_flag_the_resolved_path_is_the_data_dir_default() {
    assert_eq!(
      resolved_settings_path(&SettingsOverrides::default()),
      factorio_bot_core::paths::settings_file()
    );
  }

  /// Test 4: a `--settings` naming a missing file fails with a legible
  /// error -- not a panic -- since this runs behind `lib.rs`'s `Context::new`
  /// call, before clap has even chosen a subcommand, and release builds are
  /// `panic = "abort"`.
  #[test]
  fn a_missing_explicit_settings_file_is_a_legible_error_not_a_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let overrides = SettingsOverrides {
      settings_path: Some(dir.path().join("nope.toml")),
      ..SettingsOverrides::default()
    };

    let report = Context::resolve(&overrides).expect_err("missing file must error, not panic");
    assert!(
      report.to_string().contains("settings file not found"),
      "unexpected error: {report}"
    );
  }
}
