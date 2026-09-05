use crate::globals::create_lua_globals;
use crate::globals::goal::create_lua_goal;
use crate::globals::rcon::create_lua_rcon;
use crate::globals::record::create_lua_record;
use crate::globals::world::create_lua_world;
use factorio_bot_core::mlua::LuaSerdeExt;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::serde_json;
use factorio_bot_core::tokio::runtime::Runtime;
use factorio_bot_core::tokio::task::JoinHandle;
use factorio_bot_scripting::{OutputSink, Stream};
use miette::{IntoDiagnostic, Result, miette};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Work a binding spawned that must finish before the run is considered over.
///
/// The runtime that [`run_lua`] builds dies with the call. Dropping a tokio
/// runtime aborts every task spawned onto it that has not finished — silently,
/// with no error and no log line. A binding that hands a script a run value and
/// lets it walk away (`goal.start` without a matching `:wait()`) would
/// therefore have its work killed the moment the script returned, and the run
/// would report success while its bots were stopped mid-plan. Registering the
/// run here makes the run wait for it instead.
///
/// Two rules this encodes, decided at the job level rather than per binding:
///
/// 1. A job is one `run_lua` call, and it is not finished while work it
///    started is still running. "The script returned" is not the end of a job.
/// 2. Outstanding work is awaited, never silently dropped.
///
/// The alternative — one runtime hoisted up to the job registry and shared
/// across runs — is deliberately *not* what this does: it would couple every
/// script's lifetime to every other's and let a runaway script's tasks outlive
/// the job that owns them.
///
/// A binding reaches the running job's registry through the Lua state's app
/// data, which [`run_lua`] populates before executing the chunk:
///
/// ```ignore
/// if let Some(pending) = lua.app_data_ref::<PendingWork>() {
///     pending.register(handle);
/// }
/// ```
#[derive(Default, Clone)]
pub struct PendingWork(Arc<Mutex<Vec<JoinHandle<()>>>>);

/// The run's [`OutputSink`], published into the Lua state's app data so a
/// binding below the Lua seam can reach it.
///
/// The same seam [`PendingWork`] uses, for the same reason: `create_lua_goal`
/// is given a world, an actuator factory and a bot roster, and threading a
/// sink through it as a fifth argument would put the sink in the signature of
/// every binding that has nothing to do with output.
///
/// Unlike `PendingWork`, **absence is an ordinary state, not an error**. A
/// missing `PendingWork` means a run could be silently killed, so `goal.start`
/// refuses; a missing sink means only that nobody is listening — every
/// `run_lua(.., None)` caller, the CLI included, is in exactly that position.
/// A run with no sink emits no replay and is otherwise identical.
#[derive(Clone)]
pub struct ReplaySink(pub Arc<dyn OutputSink>);

impl PendingWork {
    /// Registers a spawned task the run must outlive.
    pub fn register(&self, handle: JoinHandle<()>) {
        self.0.lock().push(handle);
    }

    /// Awaits everything registered, returning one message per task that did
    /// not complete normally.
    ///
    /// A panicking task is reported, not propagated: one background task dying
    /// must not take down the run — and under `panic = "abort"` a re-panic here
    /// would take down the whole server process.
    pub async fn drain(&self) -> Vec<String> {
        let handles: Vec<_> = self.0.lock().drain(..).collect();
        let mut failures = Vec::new();
        for handle in handles {
            if let Err(err) = handle.await {
                failures.push(format!("background task failed: {err}"));
            }
        }
        failures
    }
}

/// `scripts_root` bounds every filesystem operation the script can reach.
/// `filename` is only used for error messages and for resolving `include`
/// relative to the script's own directory; a `None` filename (inline code
/// from the editor) simply resolves relative to the root.
///
/// `sink` receives each printed line as the script produces it. It replaces a
/// `gag` redirect of the *process's* fd 1 and 2, which captured the host's own
/// logging along with the script's, could not say which run a line belonged
/// to, and handed back a single string only once the run had finished. The
/// full transcript is still returned; the sink is the same text, live.
pub async fn run_lua(
    planner: &mut Planner,
    lua_code: &str,
    filename: Option<&str>,
    scripts_root: &Path,
    bot_count: u8,
    sink: Option<Arc<dyn OutputSink>>,
) -> Result<(Option<serde_json::Value>, (String, String))> {
    let scripts_root = scripts_root.to_path_buf();
    let stdout: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let filename = filename.unwrap_or("<inline>").to_owned();
    // Never derive this by canonicalizing `filename`'s parent: for a bare
    // name that parent is `""`, which does not canonicalize, and the old
    // `.expect()` there aborted the process on every inline run.
    let script_dir = Path::new(&filename)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(|parent| std::fs::canonicalize(parent).ok())
        .filter(|parent| parent.starts_with(&scripts_root))
        .unwrap_or_else(|| scripts_root.clone());
    let mut code_by_path: HashMap<String, String> = HashMap::new();
    code_by_path.insert(filename.clone(), lua_code.to_owned());
    let code_by_path: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(code_by_path));
    // The roster is who the game actually has, never `1..=bot_count`. A client
    // that failed to connect used to be seeded into the world at the default
    // position and handed real work; see `Planner::roster`. A planning-only run
    // (`--clients 0`) seeds its own players before this point, so its roster
    // still comes back whole.
    // Read before `planner` is borrowed for anything else: the seam only
    // needs the fact, not the planner.
    let server = planner.server;
    let all_bots = planner.roster(bot_count);
    let lua_code = lua_code.to_owned();

    // One world handle, and it is the live one. The bindings used to get a
    // deep copy taken here and never refreshed, so every `world.*` query
    // answered from a snapshot frozen at script start -- 41.8 tiles and
    // 6,214 ticks out by the end of one run, while `rcon.*` and the
    // executor read the truth. A script checking itself two ways disagreed
    // with itself. See 01948bec.
    let real_world = planner.real_world.clone();
    let rcon = planner.rcon.clone();

    let thread_stdout = stdout.clone();
    let thread_stderr = stderr.clone();
    // A second handle on the same transcript: `thread_stderr` is moved into
    // the print bindings, and the drain below reports into stderr too.
    let failure_stderr = stderr.clone();
    let failure_sink = sink.clone();

    // The registry bindings register unawaited work into, awaited before the
    // runtime that owns it is dropped. See [`PendingWork`].
    let pending = PendingWork::default();
    #[cfg(test)]
    let test_pending = pending.clone();
    #[cfg(test)]
    let test_root = scripts_root.clone();

    // Every fallible step below returns instead of unwrapping: this crate is
    // built with `panic = "abort"`, so a panic anywhere in here is a remote
    // kill of the whole server process, not a failed script.
    //
    // `spawn_blocking`, not `thread::spawn().join()`: the old form ran a
    // synchronous join inside an `async fn`, parking the calling tokio worker
    // for the entire duration of the script. With axum's default worker count
    // a handful of concurrent long scripts starved every other task on the
    // runtime, `/api/v1/health` included. The blocking pool is where a
    // long-running synchronous body belongs, and it is not an async context,
    // so building a `Runtime` inside it below is still permitted.
    let result = factorio_bot_core::tokio::task::spawn_blocking(
        move || -> Result<Option<serde_json::Value>> {
            let lua = crate::sandbox::new_sandboxed_lua()
                .map_err(|err| miette!("failed to create the lua sandbox: {err}"))?;
            // The seam bindings opt into. Set before the chunk runs so a binding
            // called from the very first line can already reach it.
            lua.set_app_data(pending.clone());
            // The same seam for the replay sink. Only when there is one: a
            // `None` sink must leave no app data behind, so `goal.start` reads
            // an honest absence rather than a wrapper around nothing.
            if let Some(sink) = sink.clone() {
                lua.set_app_data(ReplaySink(sink));
            }
            let _code_by_path = code_by_path.clone();
            let setup = (|| -> LuaResult<()> {
                let world = create_lua_world(
                    &lua,
                    real_world.clone(),
                    scripts_root.clone(),
                    script_dir.clone(),
                    rcon.clone(),
                )?;
                let goal = create_lua_goal(
                    &lua,
                    real_world.clone(),
                    real_world.clone(),
                    rcon.clone(),
                    all_bots.clone(),
                    server,
                )?;
                // Cloned before `create_lua_globals` consumes them: recording
                // needs the same roster and the same scripts root.
                let record_bots = all_bots.clone();
                let record_scripts_root = scripts_root.clone();
                create_lua_globals(
                    &lua,
                    all_bots,
                    scripts_root,
                    script_dir,
                    thread_stdout,
                    thread_stderr,
                    _code_by_path,
                    sink,
                )?;

                let globals = lua.globals();
                globals.set("world", world)?;
                globals.set("goal", goal)?;
                if let Some(rcon) = rcon.as_ref() {
                    // `record` is installed only alongside `rcon`, and that is
                    // the honest dependency rather than an omission: every
                    // event is stamped with a tick the game supplies, and the
                    // sampling session is started through the same connection.
                    // With no game there is no clock and nothing to sample,
                    // so a `record` table here would be one whose events all
                    // claimed tick 0.
                    let record = create_lua_record(
                        &lua,
                        rcon.clone(),
                        real_world.clone(),
                        record_scripts_root,
                        record_bots,
                    )?;
                    globals.set("record", record)?;
                    let rcon = create_lua_rcon(&lua, rcon.clone(), real_world.clone())?;
                    globals.set("rcon", rcon)?;
                }
                #[cfg(test)]
                install_unawaited_work_probe(&lua, &globals, test_pending, test_root)?;
                #[cfg(test)]
                install_replay_sink_probe(&lua, &globals)?;
                Ok(())
            })();
            let to_report = |err: LuaError| {
                let code_by_path = code_by_path.lock().clone();
                crate::error::to_lua_error(err, &code_by_path)
            };
            setup.map_err(to_report)?;

            let rt: Runtime = Runtime::new().into_diagnostic()?;
            rt.block_on(async {
                let outcome = async {
                    let chunk = lua.load(&lua_code).set_name(&filename);
                    chunk.exec_async().await.map_err(to_report)?;
                    // `result` is whatever the script assigned, so it can be a
                    // value serde cannot represent (a function, a table with a
                    // cycle). Unwrapping here would let a script abort the
                    // process.
                    match lua.globals().get::<LuaValue>("result") {
                        Ok(LuaValue::Nil) | Err(_) => Ok(None),
                        Ok(value) => Ok(Some(lua.from_value(value).map_err(to_report)?)),
                    }
                }
                .await;
                // Between the chunk finishing and `rt` being dropped is the only
                // window in which work spawned onto `rt` can still be awaited.
                // Unconditional, including on the error path: a chunk that raised
                // halfway through may already have started bots, and abandoning
                // them is exactly the silent abort this registry exists to stop.
                for failure in pending.drain().await {
                    if let Some(sink) = failure_sink.as_ref() {
                        sink.line(Stream::Stderr, &failure);
                    }
                    let mut stderr = failure_stderr.lock();
                    stderr.push_str("ERROR: ");
                    stderr.push_str(&failure);
                    stderr.push('\n');
                }
                outcome
            })
        },
    )
    .await
    // Same message as the old `thread::spawn(..).join()` mapping, deliberately:
    // `assert_reported_not_panicked` distinguishes "the script failed" from
    // "the interpreter thread died" by this exact string, and renaming it would
    // leave that guard asserting nothing. A `JoinError` here is a panic in the
    // script host or a cancelled blocking task; both fail this one run rather
    // than aborting the process.
    .map_err(|err| {
        if err.is_cancelled() {
            miette!("lua task was cancelled")
        } else {
            miette!("lua thread panicked")
        }
    })??;
    let stdout: String = stdout.lock().to_owned();
    let stderr: String = stderr.lock().to_owned();
    Ok((result, (stdout, stderr)))
}

/// Registers the probes the drain tests need.
///
/// The input class the drain exists for is "work a binding spawned onto the
/// run's own runtime that the script never awaited". `goal.start` produces
/// exactly that when a script omits the matching `:wait()`, and it registers
/// into [`PendingWork`] itself (`globals/goal/run.rs`'s `start_impl`) — but
/// driving the drain through it would mean a real actuator and a real plan,
/// more machinery than this module wants for a unit test of `drain()` alone.
/// These probes are the smallest thing that manufactures the class: they
/// spawn onto whatever runtime is current when the script calls them, which
/// inside `rt.block_on` is precisely the runtime that is about to be dropped.
///
/// `register = false` is the negative control: the same task, not registered,
/// must be *lost*. Without it a passing positive test would not distinguish
/// "the drain waited" from "the task happened to finish first".
/// Reaches the [`ReplaySink`] the way `goal.start` reaches it — out of the
/// Lua state's app data — and forwards a string to it, answering whether one
/// was there.
///
/// This is the only way to test the seam from *inside* a real `run_lua`. A
/// replay comes from a run, and a run cannot be started here: `run_lua` builds
/// its actuator from an rcon connection these tests do not have, so
/// `goal.start` is unreachable and the wiring `run_lua` is responsible for —
/// publishing its sink where a binding can find it — would otherwise be
/// covered only by tests that set the app data themselves and so could not
/// notice `run_lua` failing to.
#[cfg(test)]
fn install_replay_sink_probe(lua: &Lua, globals: &LuaTable) -> LuaResult<()> {
    let probe =
        lua.create_function(
            |lua: &Lua, json: String| match lua.app_data_ref::<ReplaySink>() {
                Some(sink) => {
                    sink.0.replay(&json);
                    Ok(true)
                }
                None => Ok(false),
            },
        )?;
    globals.set("__replay_sink_probe", probe)
}

#[cfg(test)]
fn install_unawaited_work_probe(
    lua: &Lua,
    globals: &LuaTable,
    pending: PendingWork,
    root: std::path::PathBuf,
) -> LuaResult<()> {
    let spawn_pending = pending.clone();
    let spawn = lua.create_function(move |_, (marker, register): (String, bool)| {
        let path = root.join(marker);
        let handle = factorio_bot_core::tokio::spawn(async move {
            factorio_bot_core::tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            // Not `.expect`: a panic here would be reported as a drained
            // failure and confuse the test it is meant to serve.
            let _ = std::fs::write(path, "finished");
        });
        if register {
            spawn_pending.register(handle);
        }
        Ok(())
    })?;
    globals.set("__spawn_unawaited_work", spawn)?;

    let panic_pending = pending;
    let spawn_panicking = lua.create_function(move |_, ()| {
        panic_pending.register(factorio_bot_core::tokio::spawn(async {
            panic!("background boom");
        }));
        Ok(())
    })?;
    globals.set("__spawn_panicking_work", spawn_panicking)?;
    Ok(())
}

/// `pub(crate)`: [`RecordingSink`] is the one [`OutputSink`] implementation
/// this crate owns, and `globals::goal::run`'s tests drive it too — a run is
/// the only thing that produces a replay, and a run cannot be started from
/// here (`run_lua` builds its actuator from an rcon connection these tests do
/// not have). Sharing the one implementation is the point: a second recorder
/// written next to the run tests could record a `replay` call the real
/// implementer never grew.
#[cfg(test)]
pub(crate) mod tests {
    use factorio_bot_core::factorio::rcon::FactorioRcon;
    use factorio_bot_core::serde_json::json;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_scripting::Stream;
    use std::sync::Arc;

    use super::*;

    /// The repository checkout is the sandbox root for `test_script`: its
    /// fixture script includes `scripts/lib.lua` from the repository root and
    /// writes its output next to itself under `crates/scripting_lua/tests`.
    fn repo_root() -> std::path::PathBuf {
        std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
            .expect("canonicalize repo root")
    }

    /// Runs `code` with the sandbox rooted at a fresh temp directory and returns
    /// the run's error, if any. The temp dir is returned so the caller can assert
    /// on what did and did not appear on disk.
    async fn sandboxed(code: &str) -> (tempfile::TempDir, Result<()>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let outcome = run_lua(&mut planner, code, None, &root, 1, None)
            .await
            .map(|_| ());
        (dir, outcome)
    }

    /// Like [`sandboxed`], but seeds the root first so a test can exercise the
    /// bindings against files that actually exist.
    async fn sandboxed_with(
        seed: impl FnOnce(&Path),
        code: &str,
    ) -> (tempfile::TempDir, Result<()>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        seed(&root);
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let outcome = run_lua(&mut planner, code, None, &root, 1, None)
            .await
            .map(|_| ());
        (dir, outcome)
    }

    /// Like [`sandboxed`], but with an RCON handle, because `create_lua_rcon`
    /// only registers the `rcon.*` table when one exists — which is exactly
    /// the configuration the HTTP execute endpoint runs in.
    ///
    /// The handle has no connection pool, so any call that reaches the socket
    /// fails. That is deliberate: these tests are about the argument parsing
    /// that happens *before* the call, and a test that needed a live Factorio
    /// server would not run here at all.
    async fn sandboxed_with_rcon(code: &str) -> (tempfile::TempDir, Result<()>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let rcon = Arc::new(FactorioRcon::new_empty());
        let mut planner = Planner::new(world, Some(rcon));
        let outcome = run_lua(&mut planner, code, None, &root, 1, None)
            .await
            .map(|_| ());
        (dir, outcome)
    }

    /// Guards the helper itself: if `rcon.mine` were misspelled the call would
    /// be `attempt to call a nil value`, which is an error and not a panic, so
    /// the two tests below would pass against the vulnerable code without ever
    /// reaching the argument parsing they exist to cover.
    #[tokio::test]
    async fn the_rcon_table_is_registered_when_a_handle_exists() {
        let (_dir, result) = sandboxed_with_rcon(
            r#"
            assert(type(rcon) == "table", "rcon table missing")
            assert(type(rcon.mine) == "function", "rcon.mine missing")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn an_rcon_binding_given_a_table_without_coordinates_reports_an_error() {
        // `rcon.mine(player_id, name, position, count)` -- the position is the
        // third argument. An empty table has no `x`, and the old
        // `position.get("x").unwrap()` aborted the process on it.
        let (_dir, result) = sandboxed_with_rcon(r#"rcon.mine(1, "iron-ore", {}, 1)"#).await;
        assert!(result.is_err(), "a table with no x/y must be refused");
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn an_rcon_binding_given_a_non_numeric_coordinate_reports_an_error() {
        let (_dir, result) =
            sandboxed_with_rcon(r#"rcon.mine(1, "iron-ore", {x="north", y=0}, 1)"#).await;
        assert!(result.is_err(), "a non-numeric coordinate must be refused");
        assert_reported_not_panicked(&result);
    }

    /// The availability half: no attacker involved. With a handle that has no
    /// connection, a well-formed call must report that the game server is
    /// unreachable rather than take the process down -- which is what happens
    /// when Factorio drops its connection mid-script.
    #[tokio::test]
    async fn an_rcon_call_that_cannot_reach_the_game_reports_an_error() {
        let (_dir, result) = sandboxed_with_rcon(r#"rcon.print("hello")"#).await;
        assert!(result.is_err(), "an unreachable rcon call must be refused");
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn an_rcon_call_with_a_valid_position_that_cannot_reach_the_game_reports_an_error() {
        // Proves the position parse *succeeded* and the failure came from the
        // call: the same script with a valid position gets past the argument
        // check that the two tests above stop at.
        let (_dir, result) =
            sandboxed_with_rcon(r#"rcon.mine(1, "iron-ore", {x=0, y=0}, 1)"#).await;
        assert!(result.is_err(), "an unreachable rcon call must be refused");
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn a_script_cannot_reach_the_filesystem_standard_library() {
        // `Lua::new()` loaded `StdLib::ALL_SAFE`, which is memory-safe and not
        // remotely sandboxed: `io.open` wrote anywhere the process could and
        // `os.execute` ran arbitrary commands, right beside the four bindings
        // this task bounded.
        let (_dir, result) = sandboxed(
            r#"
            assert(io == nil, "io is reachable")
            assert(os == nil, "os is reachable")
            assert(package == nil, "package is reachable")
            assert(require == nil, "require is reachable")
            assert(dofile == nil, "dofile is reachable")
            assert(loadfile == nil, "loadfile is reachable")
            -- `load` is deliberately still present, so asserting it is
            -- non-nil would prove nothing. What matters is that it is the
            -- text-only replacement: it must refuse a binary chunk.
            assert(type(load) == "function", "load is missing")
            local refused, message = load(string.dump(function() return 1 end, true), "attack")
            assert(refused == nil, "load accepted a binary chunk")
            assert(message ~= nil, "load refused without a message")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// Lua's `load` defaults to mode `"bt"`, so it accepted *bytecode* as well
    /// as source. PUC Lua 5.4's bytecode loader does not validate untrusted
    /// input, which makes crafted bytecode a route to type confusion and
    /// arbitrary memory access — straight past the `io`/`os` lockdown.
    /// `load(string.dump(f, true))` used to round-trip to a working function.
    #[tokio::test]
    async fn a_script_cannot_load_a_binary_chunk() {
        let (_dir, result) = sandboxed(
            r#"
            local dumped = string.dump(function() return 41 + 1 end, true)
            assert(#dumped > 0, "fixture bug: string.dump produced nothing")

            -- The chunkname is explicit and NUL-free on purpose. Lua derives a
            -- missing chunkname from the source, and bytecode contains NUL
            -- bytes, which fails the CString conversion on the way to the
            -- loader -- so an unnamed binary chunk is refused whether or not
            -- the mode is pinned, and a test that omitted the name would pass
            -- against the vulnerable build. Naming it is what forces the
            -- refusal to come from the mode.
            local loaded, message = load(dumped, "attack")
            -- Assert on the refusal itself, not merely that something came
            -- back: the vulnerable version returned a callable here.
            assert(loaded == nil, "load returned a value for a binary chunk")
            assert(type(message) == "string", "no error message for a binary chunk")

            -- The unnamed path too, so both are covered.
            local unnamed = load(dumped)
            assert(unnamed == nil, "load returned a value for an unnamed binary chunk")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// `string.dump` is not the vulnerability and removing it would fix
    /// nothing — a script can spell the bytecode out as a literal. This pins
    /// that the refusal is about the *chunk*, not about where it came from.
    #[tokio::test]
    async fn a_script_cannot_load_a_binary_chunk_written_as_a_literal() {
        let (_dir, result) = sandboxed(
            r#"
            -- Rebuild the exact same bytes one `string.char` at a time. The
            -- result is a plain string literal as far as `load` is concerned
            -- -- nothing here is a `string.dump` return value -- and it is
            -- still complete, valid bytecode, so the refusal has to come from
            -- the mode rather than from a truncated header.
            local dumped = string.dump(function() return 41 + 1 end, true)
            local rebuilt = {}
            for i = 1, #dumped do
                rebuilt[i] = string.char(dumped:byte(i))
            end
            rebuilt = table.concat(rebuilt)
            assert(rebuilt == dumped, "fixture bug: rebuilt bytes differ")

            local loaded, message = load(rebuilt, "attack")
            assert(loaded == nil, "load returned a value for a binary literal")
            assert(type(message) == "string", "no error message for a binary literal")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn a_script_can_still_load_source_text() {
        // The counterpart that keeps the two tests above honest: a `load` that
        // refused everything would pass them and break the product.
        let (_dir, result) = sandboxed(
            r#"
            local f = load("return 1 + 1")
            assert(type(f) == "function", "load did not return a function")
            assert(f() == 2, "loaded chunk did not run")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn a_syntax_error_in_a_loaded_chunk_returns_nil_and_a_message() {
        // Real `load` reports a compile error by returning `nil, msg` rather
        // than raising, and scripts test that second value. Turning it into a
        // raised error would be a behaviour change beyond the security fix.
        let (_dir, result) = sandboxed(
            r#"
            local f, message = load("this is not lua ==")
            assert(f == nil, "a syntax error produced a function")
            assert(type(message) == "string", "a syntax error produced no message")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn the_replacement_load_keeps_the_rest_of_its_signature() {
        // The reader-function form and the `env` argument are part of `load`.
        // Silently dropping either would trade a security bug for a
        // correctness one.
        let (_dir, result) = sandboxed(
            r#"
            local pieces = {"return ", "7 * 6"}
            local i = 0
            local f = load(function() i = i + 1 return pieces[i] end)
            assert(f ~= nil and f() == 42, "reader-function form broken")

            local env = {answer = 42}
            local g = load("return answer", "=chunk", "t", env)
            assert(g ~= nil and g() == 42, "env argument ignored")

            local named = load("error('x')", "custom-name")
            assert(named ~= nil, "chunkname argument broke loading")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn a_script_can_still_use_the_computation_standard_library() {
        // The lockdown is an allow-list, and an allow-list that removes what
        // scripts actually compute with is a broken product, not a sandbox.
        let (_dir, result) = sandboxed(
            r#"
            local t = {}
            table.insert(t, "b")
            table.insert(t, 1, "a")
            assert(table.concat(t) == "ab")
            assert(string.upper("ab") == "AB")
            assert(math.max(1, 2) == 2)
            assert(type(coroutine.create(function() end)) == "thread")
            assert(tostring(1) == "1" and tonumber("1") == 1)
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn every_host_binding_still_works_after_the_standard_library_lockdown() {
        // The positive counterpart to the escape tests: a sandbox that breaks
        // the product is not a fix. Exercises all four bounded bindings plus
        // `include` in one run.
        let (dir, result) = sandboxed_with(
            |root| {
                std::fs::write(root.join("lib.lua"), "included_value = 7").expect("write");
                std::fs::write(root.join("in.txt"), "contents").expect("write");
            },
            r#"
            include("lib.lua")
            assert(included_value == 7, "include did not run")
            assert(file_read("in.txt") == "contents", "file_read")
            file_write("out.txt", "written")
            world.draw("world.png")
            "#,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.txt")).expect("file_write"),
            "written"
        );
        assert!(dir.path().join("world.png").is_file(), "world.draw");
    }

    /// The parent chain is canonicalized but the destination name is not, so a
    /// symlink planted at the name itself passed every bounds check and the
    /// write went through to whatever it pointed at.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_script_cannot_write_through_a_symlinked_destination() {
        let outside = tempfile::tempdir().expect("tempdir");
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, "original").expect("write");

        let victim_for_seed = victim.clone();
        let (_dir, result) = sandboxed_with(
            move |root| {
                std::os::unix::fs::symlink(&victim_for_seed, root.join("evil.txt"))
                    .expect("symlink");
            },
            r#"file_write("evil.txt", "PWNED")"#,
        )
        .await;

        assert!(result.is_err(), "the write should have been refused");
        // Asserting on the *contents*, not merely that the call errored: the
        // question is whether the outside file survived.
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim still readable"),
            "original",
            "the outside file was overwritten through the symlink"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_script_cannot_draw_the_world_through_a_symlinked_destination() {
        let outside = tempfile::tempdir().expect("tempdir");
        let victim = outside.path().join("victim.png");
        std::fs::write(&victim, "original").expect("write");

        let victim_for_seed = victim.clone();
        let (_dir, result) = sandboxed_with(
            move |root| {
                std::os::unix::fs::symlink(&victim_for_seed, root.join("evil.png"))
                    .expect("symlink");
            },
            r#"world.draw("evil.png")"#,
        )
        .await;

        assert!(result.is_err(), "the draw should have been refused");
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim still readable"),
            "original",
            "the outside file was overwritten through the symlink"
        );
    }

    /// `relative_to` used to hand an absolute in-root path to the resolver
    /// already relativised, so `resolve_write_path` never saw it as absolute
    /// and accepted it -- the system allowed what the unit test on
    /// `resolve_write_path` says it refuses. Rejecting in the binding is what
    /// makes those two agree.
    #[tokio::test]
    async fn a_script_cannot_name_a_path_absolutely_even_inside_the_scripts_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let inside = root.join("absolute.txt");
        let code = format!(
            "file_write({:?}, \"owned\")",
            inside.to_str().expect("utf8")
        );

        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let result = run_lua(&mut planner, &code, None, &root, 1, None)
            .await
            .map(|_| ());

        assert!(result.is_err(), "the write should have been refused");
        assert!(
            !inside.exists(),
            "the file was created: {}",
            inside.display()
        );
    }

    /// The Lua surface of the 2.x widening, checked through the real
    /// interpreter.
    ///
    /// `scripts/api_test.lua` covers the same ground but is a *shipped sample
    /// script*, not a test: it prints its own verdict, so when
    /// `directions_all()` went from 8 to 16 it would have printed `FAIL` at a
    /// correct API, with nothing in CI to contradict it. This is the assertion
    /// that goes red instead.
    #[tokio::test]
    async fn the_lua_direction_table_is_factorio_2_xs_sixteen_values() {
        let (_dir, result) = sandboxed(
            r#"
            local n = 0
            for _ in pairs(Direction) do n = n + 1 end
            assert(n == 16, "Direction must name all sixteen values, got " .. n)

            -- Names keep their meanings; the numbers behind them move.
            assert(Direction.North == 0, "north is 0")
            assert(Direction.East == 4, "east is 4, not 2")
            assert(Direction.South == 8, "south is 8, not 4")
            assert(Direction.West == 12, "west is 12, not 6")
            assert(Direction.NorthEast == 2, "northeast is 2, which used to be east")
            assert(Direction.NorthNorthEast == 1, "the half-diagonals rails use")

            assert(#directions_all() == 16, "directions_all is sixteen now")
            assert(#directions_compass() == 8, "directions_compass is what it used to return")
            assert(#directions_orthogonal() == 4, "still the four cardinals")

            assert(direction_clockwise(Direction.North) == Direction.East, "90 degrees")
            assert(direction_clockwise(Direction.West) == Direction.North, "90 degrees")
            assert(direction_opposite(Direction.North) == Direction.South, "180 degrees")
            assert(direction_opposite(Direction.East) == Direction.West, "180 degrees")
        "#,
        )
        .await;
        result.expect("the direction surface must hold");
    }

    #[tokio::test]
    async fn an_out_of_range_direction_is_reported_not_panicked() {
        // `Direction::from_u8(16)` is `None`, and unwrapping it aborted. This
        // used to pass 9, which was out of range on the Factorio 1.x scale and
        // is `SouthSouthWest` on the 2.x one -- 16 is the first value still
        // outside `defines.direction`.
        let (_dir, result) = sandboxed("direction_clockwise(16)").await;
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn a_position_table_without_coordinates_is_reported_not_panicked() {
        // `near.get("x")` on `{}` is `Nil`, and unwrapping the conversion
        // aborted.
        let (_dir, result) =
            sandboxed(r#"world.find_free_resource_rect("iron-ore", 2, 2, {})"#).await;
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn a_script_cannot_write_outside_the_scripts_root() {
        let victim = std::env::temp_dir().join(format!("pwned-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&victim);
        let code = format!(
            "file_write({:?}, \"owned\")",
            victim.to_str().expect("utf8")
        );
        let (_dir, result) = sandboxed(&code).await;
        assert!(result.is_err(), "the write should have been refused");
        assert!(
            !victim.exists(),
            "the file was created outside the root: {}",
            victim.display()
        );
    }

    #[tokio::test]
    async fn a_script_cannot_climb_out_of_the_scripts_root() {
        let (dir, result) = sandboxed("file_write(\"../pwned.txt\", \"owned\")").await;
        assert!(result.is_err(), "the write should have been refused");
        assert!(
            !dir.path()
                .parent()
                .expect("parent")
                .join("pwned.txt")
                .exists()
        );
    }

    #[tokio::test]
    async fn a_script_cannot_read_outside_the_scripts_root() {
        let (_dir, result) = sandboxed("file_read(\"/etc/hostname\")").await;
        assert!(result.is_err(), "the read should have been refused");
    }

    #[tokio::test]
    async fn a_script_cannot_include_code_from_outside_the_scripts_root() {
        let (_dir, result) = sandboxed("include(\"../../../etc/hostname\")").await;
        assert!(result.is_err(), "the include should have been refused");
    }

    #[tokio::test]
    async fn a_script_cannot_draw_the_world_outside_the_scripts_root() {
        let victim = std::env::temp_dir().join(format!("pwned-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&victim);
        let code = format!("world.draw({:?})", victim.to_str().expect("utf8"));
        let (_dir, result) = sandboxed(&code).await;
        assert!(result.is_err(), "the draw should have been refused");
        assert!(
            !victim.exists(),
            "the image was written outside the root: {}",
            victim.display()
        );
    }

    #[tokio::test]
    async fn a_script_may_write_inside_the_scripts_root() {
        // The counterpart that keeps the five tests above honest: if the bindings
        // were simply broken rather than bounded, they would all still pass.
        let (dir, result) = sandboxed("file_write(\"ok.txt\", \"fine\")").await;
        assert!(
            result.is_ok(),
            "an in-root write must still work: {result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ok.txt")).expect("written"),
            "fine"
        );
    }

    #[tokio::test]
    async fn a_run_without_a_filename_does_not_panic() {
        // Guards the `Path::new("unknown.lua").parent() == Some("")` crash: `""`
        // does not canonicalize, so the old code aborted the process before
        // executing a line. `None` for the filename is what inline code passes.
        let (_dir, result) = sandboxed("result = 1 + 1").await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// A panic inside the interpreter thread surfaces as this exact message,
    /// so asserting on it is what separates "the script failed" (wanted) from
    /// "the process would have died" (a remote kill under `panic = "abort"`).
    fn assert_reported_not_panicked(result: &Result<()>) {
        let err = result.as_ref().expect_err("expected an error");
        assert!(
            !format!("{err}").contains("lua thread panicked"),
            "the interpreter thread panicked instead of reporting: {err:?}"
        );
    }

    #[tokio::test]
    async fn an_error_from_a_chunk_the_script_named_itself_is_reported_not_panicked() {
        // `code_by_path` has no entry for "evil", and the old error formatter
        // unwrapped that lookup -- one line of Lua killed the whole process.
        let (_dir, result) = sandboxed(r#"load("error('boom')", "evil")()"#).await;
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn a_crafted_error_message_with_an_unparsable_line_number_is_reported_not_panicked() {
        // `error(msg, 0)` suppresses Lua's own position prefix, so the whole
        // message -- including the `[string "x"]:N:` the formatter parses -- is
        // exactly what the script chose. A line number this size overflows the
        // `usize` the formatter parsed it into.
        let (_dir, result) =
            sandboxed("error('[string \"<inline>\"]:99999999999999999999999999: boom', 0)").await;
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn a_crafted_error_message_naming_a_line_past_the_end_is_reported_not_panicked() {
        let (_dir, result) = sandboxed("error('[string \"<inline>\"]:9999: boom', 0)").await;
        assert_reported_not_panicked(&result);
    }

    #[tokio::test]
    async fn a_result_that_cannot_be_serialised_is_reported_not_panicked() {
        // `result` is whatever the script assigned; a function has no serde
        // representation, and `lua.from_value(..).unwrap()` was reachable.
        let (_dir, result) = sandboxed("result = function() end").await;
        assert_reported_not_panicked(&result);
    }

    /// The sink is what makes per-job SSE possible: the previous
    /// implementation redirected the *process's* fd 1 and 2 with `gag`, which
    /// captured the server's own logging, could not attribute a line to a job,
    /// and yielded nothing until the run was over.
    #[derive(Default)]
    pub(crate) struct RecordingSink {
        pub(crate) lines: Mutex<Vec<(Stream, String)>>,
        /// Every string handed to [`OutputSink::replay`], verbatim.
        ///
        /// Kept as the raw JSON rather than parsed on arrival: what a
        /// consumer receives is the text, and a recorder that deserialised
        /// would hide a document whose serialisation is wrong from the tests
        /// that exist to catch exactly that.
        pub(crate) replays: Mutex<Vec<String>>,
    }

    impl OutputSink for RecordingSink {
        fn line(&self, stream: Stream, text: &str) {
            self.lines.lock().push((stream, text.to_owned()));
        }

        fn replay(&self, json: &str) {
            self.replays.lock().push(json.to_owned());
        }
    }

    #[tokio::test]
    async fn script_output_reaches_the_sink_as_it_is_printed() {
        let sink = Arc::new(RecordingSink::default());
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        run_lua(
            &mut planner,
            "print(\"first\")\nprint(\"second\")",
            None,
            &root,
            1,
            Some(sink.clone()),
        )
        .await
        .expect("run_lua failed");

        let lines = sink.lines.lock().clone();
        assert_eq!(
            lines,
            vec![
                (Stream::Stdout, "first".to_owned()),
                (Stream::Stdout, "second".to_owned()),
            ],
            "both prints should have reached the sink, in order"
        );
    }

    /// `print_warn` accumulates into *stdout*, which is not what its name
    /// suggests, and `print`/`print_err` split the usual way. The SSE consumer
    /// splits on stream, so each binding must report to the sink the same
    /// stream it writes into the transcript -- and the transcript, prefixes
    /// and all, must survive the sink being added beside it.
    #[tokio::test]
    async fn each_print_binding_reports_the_stream_it_accumulates_into() {
        let sink = Arc::new(RecordingSink::default());
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (_result, (stdout, stderr)) = run_lua(
            &mut planner,
            "print(\"out\")\nprint_err(\"bad\")\nprint_warn(\"careful\")",
            None,
            &root,
            1,
            Some(sink.clone()),
        )
        .await
        .expect("run_lua failed");

        assert_eq!(
            sink.lines.lock().clone(),
            vec![
                (Stream::Stdout, "out".to_owned()),
                (Stream::Stderr, "bad".to_owned()),
                (Stream::Stdout, "careful".to_owned()),
            ],
            "each binding must report the stream it accumulates into"
        );
        assert_eq!(stdout, "out\nWARN: careful\n", "stdout transcript");
        assert_eq!(stderr, "ERROR: bad\n", "stderr transcript");
    }

    /// `run_lua` must publish its sink where a binding below the Lua seam can
    /// reach it, and what reaches the sink must be the string, byte for byte.
    ///
    /// The probe stands in for `goal.start`, which cannot run here — see
    /// [`install_replay_sink_probe`]. What it proves is the half `goal::run`'s
    /// own tests cannot: they install a `ReplaySink` themselves, so they would
    /// pass unchanged against a `run_lua` that published none.
    #[tokio::test]
    async fn a_sink_given_to_run_lua_is_reachable_as_app_data_and_gets_the_string_verbatim() {
        let sink = Arc::new(RecordingSink::default());
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        // Quoted, braced and non-ASCII: a document is JSON, and a transport
        // that re-encoded or truncated it would show up here.
        let payload = r#"{"planned_makespan":250,"steps":[],"note":"a\"b — ü"}"#;
        run_lua(
            &mut planner,
            &format!("result = __replay_sink_probe({payload:?})"),
            None,
            &root,
            1,
            Some(sink.clone()),
        )
        .await
        .expect("run_lua failed");

        assert_eq!(
            sink.replays.lock().clone(),
            vec![payload.to_owned()],
            "the sink must receive exactly the string it was handed"
        );
    }

    /// The negative control. Without it the test above cannot tell "the sink
    /// was published" from "the probe found some sink".
    #[tokio::test]
    async fn a_run_lua_with_no_sink_publishes_no_replay_sink_at_all() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (result, _) = run_lua(
            &mut planner,
            "result = __replay_sink_probe(\"{}\")",
            None,
            &root,
            1,
            None,
        )
        .await
        .expect("run_lua failed");
        assert_eq!(
            result,
            Some(json!(false)),
            "no sink means no app data, not a wrapper around nothing"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_long_script_does_not_block_other_tasks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);

        // A pure-Lua busy loop: long enough to be unmistakable, no sleeping, so
        // it genuinely occupies whatever thread it runs on.
        let code = "local n = 0 for i = 1, 40000000 do n = n + i end result = n";

        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ticker = {
            let ticks = ticks.clone();
            tokio::spawn(async move {
                for _ in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        };

        run_lua(&mut planner, code, None, &root, 1, None)
            .await
            .expect("run_lua failed");
        ticker.await.expect("ticker panicked");
        assert!(
            ticks.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "the runtime made no progress on other tasks while the script ran"
        );
    }

    /// The variant that actually fails against `thread::spawn(..).join()`.
    ///
    /// Three things have to be true for a test to reproduce the production
    /// failure, and the test above has none of them:
    ///
    /// * The script has to run *on a worker*. A `#[tokio::test]` body is driven
    ///   by `block_on` on the calling thread, which is not one of the
    ///   `worker_threads`, so blocking it starves nothing. `run_lua` has to be
    ///   `spawn`ed, the way axum spawns a request handler.
    /// * Every worker has to be occupied. With two workers the blocked one
    ///   leaves a second free and the ticker still runs -- verified: the test
    ///   above passes against `thread::spawn(..).join()`.
    /// * The progress has to be counted *during* the script, not after it.
    ///   Reading a counter once `run_lua` has returned is a race the defect
    ///   wins: the worker is free again by then and drains the ticker's expired
    ///   timers before the main thread is even woken -- which is why the
    ///   obvious `assert!(ticks > 0)` after the await passes either way.
    ///
    /// Hence the flag: the ticker only counts while the script task says it is
    /// inside `run_lua`, and the task clears the flag with no `.await` between,
    /// so the ticker cannot see a stale `true`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_long_script_leaves_the_worker_free_while_it_runs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");

        let running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ticker = {
            let ticks = ticks.clone();
            let running = running.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    if running.load(std::sync::atomic::Ordering::SeqCst) {
                        ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            })
        };
        // Let the ticker reach its first `sleep` on the worker before the
        // script task is queued behind it.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let script = {
            let running = running.clone();
            tokio::spawn(async move {
                let world = Arc::new(fixture_world());
                let mut planner = Planner::new(world, None);
                let code = "local n = 0 for i = 1, 40000000 do n = n + i end result = n";
                running.store(true, std::sync::atomic::Ordering::SeqCst);
                let outcome = run_lua(&mut planner, code, None, &root, 1, None).await;
                running.store(false, std::sync::atomic::Ordering::SeqCst);
                outcome.expect("run_lua failed");
            })
        };

        script.await.expect("script task panicked");
        ticker.abort();
        assert!(
            ticks.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "the only worker stayed inside the script for its whole duration, \
             so nothing else on the runtime ran"
        );
    }

    /// The rule this guards: a job is one `run_lua` call, and it is not
    /// finished while work it started is still running. The script starts work
    /// and returns immediately without waiting for it; `run_lua` must not
    /// return until that work is done.
    #[tokio::test]
    async fn work_a_binding_registered_finishes_before_the_run_returns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);

        run_lua(
            &mut planner,
            r#"__spawn_unawaited_work("registered.txt", true)"#,
            None,
            &root,
            1,
            None,
        )
        .await
        .expect("run_lua failed");

        // Asserting on the task's *effect*, not on `drain` returning: the
        // question is whether the work ran to completion, and a `drain` that
        // dropped its handles would still return.
        assert_eq!(
            std::fs::read_to_string(root.join("registered.txt"))
                .expect("registered work did not finish before run_lua returned"),
            "finished"
        );
    }

    /// The negative control for the test above, and the reason the registry
    /// exists at all: unregistered work spawned onto the run's runtime is
    /// aborted when that runtime is dropped -- silently, with no error on the
    /// run. If this ever passes with the marker present, the positive test
    /// above stopped proving anything, because the task would be finishing on
    /// its own rather than because the drain waited for it.
    #[tokio::test]
    async fn work_a_binding_did_not_register_is_lost_when_the_run_ends() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);

        let (_result, (_stdout, stderr)) = run_lua(
            &mut planner,
            r#"__spawn_unawaited_work("unregistered.txt", false)"#,
            None,
            &root,
            1,
            None,
        )
        .await
        .expect("run_lua failed");

        assert!(
            !root.join("unregistered.txt").exists(),
            "unregistered work survived the run, so the positive test proves nothing"
        );
        assert_eq!(stderr, "", "the abort really is silent");
    }

    /// A background task that panics must fail its own run's transcript, not
    /// the run and not the process: under `panic = "abort"` a re-panic on the
    /// draining thread is a remote kill of the server.
    #[tokio::test]
    async fn a_background_task_that_panics_is_reported_and_does_not_fail_the_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let sink = Arc::new(RecordingSink::default());

        let (_result, (_stdout, stderr)) = run_lua(
            &mut planner,
            "__spawn_panicking_work()",
            None,
            &root,
            1,
            Some(sink.clone()),
        )
        .await
        .expect("a panicking background task must not fail the run");

        assert!(
            stderr.contains("background task failed"),
            "the failure was swallowed instead of reported: {stderr:?}"
        );
        // The SSE consumer reads the sink, not the transcript, so a failure
        // that reached only one of the two would be invisible live.
        assert!(
            sink.lines
                .lock()
                .iter()
                .any(|(stream, text)| *stream == Stream::Stderr
                    && text.contains("background task failed")),
            "the failure never reached the sink"
        );
    }

    /// The fixture script builds a starter iron plan and asserts on the
    /// planner's own output; running it here is what proves the whole Lua
    /// surface it touches -- `world.*`, `include`, `goal.have`, `goal.plan`,
    /// `plan:graphviz()`, `plan:gantt()` -- still composes.
    ///
    /// It used to write the transcript to `tests/stdout-N.txt` and assert
    /// nothing at all, so a script that silently stopped halfway still passed.
    /// The transcript is asserted instead: `end script` is the last line
    /// `main()` prints, and it is only reached after every `assert` inside the
    /// script has held.
    #[tokio::test]
    async fn test_script() {
        let world = Arc::new(fixture_world());

        let tests_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("script.lua");
        let tests_path = tests_path.to_str().unwrap();

        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_str().unwrap().to_owned();
        let relative_path = tests_path.replace(&cwd, "");
        let relative_path = &relative_path[1..];

        for bot_count in 1..=2 {
            let mut planner = Planner::new(world.clone(), None);
            // No Factorio behind this world, so seed the bots the way the
            // planning-only mode does before `roster` reads them -- see
            // `Planner::roster`.
            planner.initiate_missing_players_with_default_inventory(bot_count);
            let (_result, (stdout, stderr)) = run_lua(
                &mut planner,
                include_str!("../tests/script.lua"),
                Some(relative_path),
                &repo_root(),
                bot_count,
                None,
            )
            .await
            .expect("run_lua failed");

            assert!(
                stdout.contains(&format!("start script for {bot_count} bots")),
                "the script must see the roster it was started with; stdout was:\n{stdout}"
            );
            assert!(
                stdout.contains("end script"),
                "the script must run to completion for {bot_count} bot(s); stdout was:\n{stdout}"
            );
            assert!(
                stderr.is_empty(),
                "the fixture script must not report errors; stderr was:\n{stderr}"
            );
        }
    }

    /// `world.draw` was exercised only by the fixture writing
    /// `world_start.png` and `world_end-N.png` into `tests/` -- files nothing
    /// read or compared, which regenerated differently on every run and so read
    /// as snapshot tests without being any. Drawing into a temp root and
    /// checking a real PNG came out is the same coverage with an oracle: a
    /// binding that wrote nothing, or wrote something that is not an image,
    /// fails here instead of silently updating a tracked file.
    #[tokio::test]
    async fn world_draw_writes_a_png() {
        let (dir, result) = sandboxed(r#"world.draw("drawn.png")"#).await;
        result.expect("world.draw failed");

        let drawn = std::fs::read(dir.path().join("drawn.png")).expect("world.draw wrote no file");
        // Checked before slicing to `[..8]`: a `drawn` shorter than 8 bytes
        // would otherwise panic on the index itself, before the assertion
        // message below -- the one meant for a failing developer -- ever
        // gets to print.
        assert!(
            drawn.len() >= 8 && &drawn[..8] == b"\x89PNG\r\n\x1a\n",
            "world.draw must write a PNG; got {} bytes starting {:?}",
            drawn.len(),
            &drawn[..8.min(drawn.len())]
        );
    }

    /// The only place `world.find_entities_in_radius` was exercised was
    /// `mine_rocks` in `scripts/lib.lua`, called from `test_script`'s
    /// `build_starter_mining` -- but `mine_rocks` early-returns whenever
    /// `rcon` is `nil` (see its `SKIP` guard), and every `test_script`
    /// planner is built with `Planner::new(world, None)`, so that call is a
    /// guarded no-op under CI: the test still passes having exercised
    /// nothing. This calls the binding directly against the fixture's
    /// `rock-huge` cluster -- `spawn_rocks` seeds 3 of them around (20, 20)
    /// in a tight spiral, see `test_utils.rs` -- and asserts on how many
    /// come back, so a binding that stops filtering by name or radius fails
    /// here instead of silently losing its only coverage.
    #[tokio::test]
    async fn test_find_entities_in_radius() {
        result_test(
            1,
            r#"
local entities = world.find_entities_in_radius({x=20,y=20}, 10, "rock-huge", nil)
result = #entities
"#,
            json!(3),
        )
        .await
    }

    // The fixture's iron-ore field (see `add_to_rect` in `test_utils.rs`) spans
    // integer tile positions x in [-45,-35], y in [35,45]. Since
    // `EntityGraph::resource_patches`'s flood-fill bug was fixed (45c1e1f), the
    // field is now a single contiguous patch instead of being split into
    // fragments by seed order, so `find_free_resource_rect` searches the whole
    // field for every query origin. Both {x=0,y=0} and {x=0,y=-200} lie north
    // and/or east of the field, so both resolve to the same north-east corner
    // of the patch (the closest corner to either origin): the edge element
    // (-35,35) is the nearest point overall but can't anchor a 2x2 block
    // (x=-35 is the field's east edge), so the nearest valid anchor is
    // (-36,35).
    #[tokio::test]
    async fn test_free_rect_from_center() {
        result_test(
            1,
            r#"
result = world.find_free_resource_rect("iron-ore", 2, 2, {x=0,y=0})
"#,
            json!({
                "left_top": {"x": -36.0, "y": 35.0},
                "right_bottom": {"x": -34.0, "y": 37.0}
            }),
        )
        .await
    }

    #[tokio::test]
    async fn test_free_rect_from_top() {
        result_test(
            1,
            r#"
result = world.find_free_resource_rect("iron-ore", 2, 2, {x=0,y=-200})
"#,
            json!({
                "left_top": {"x": -36.0, "y": 35.0},
                "right_bottom": {"x": -34.0, "y": 37.0}
            }),
        )
        .await
    }

    // A far-southern origin is closest to the field's *southern* edge (high
    // y) rather than the northern one, so it must resolve to a different
    // rect than `test_free_rect_from_center`/`test_free_rect_from_top` above
    // -- this is what restores coverage of the `near` parameter now that
    // those two return the same value. The nearest edge element is (-35,45),
    // which can't anchor a 2x2 block (y=45 is the field's south edge), so the
    // nearest valid anchor is (-36,44).
    #[tokio::test]
    async fn test_free_rect_from_south() {
        result_test(
            1,
            r#"
result = world.find_free_resource_rect("iron-ore", 2, 2, {x=0,y=200})
"#,
            json!({
                "left_top": {"x": -36.0, "y": 44.0},
                "right_bottom": {"x": -34.0, "y": 46.0}
            }),
        )
        .await
    }

    // The no-fit path is the one a caller is most likely to get wrong, so it
    // has to be distinguishable from a hit. It used to answer `Rect::default()`
    // -- an all-zero rectangle at the map origin -- which reads as a perfectly
    // good 0x0 site at (0,0). 500x500 does not fit in the fixture's 10x10 iron
    // field, so every patch is searched and exhausted: this is the "a patch
    // exists but has no room" case, not the "no such ore" one.
    #[tokio::test]
    async fn test_free_rect_absent_when_nothing_fits() {
        result_test(
            1,
            r#"
local r = world.find_free_resource_rect("iron-ore", 500, 500, {x=0,y=0})
result = { found = r ~= nil, is_table = type(r) == "table" }
"#,
            json!({"found": false, "is_table": false}),
        )
        .await
    }

    // An ore the fixture has no patch of at all reaches the same exit with the
    // patch loop never entered, so it must answer the same absence.
    #[tokio::test]
    async fn test_free_rect_absent_when_ore_is_unknown() {
        result_test(
            1,
            r#"
result = world.find_free_resource_rect("uranium-ore", 2, 2, {x=0,y=0}) ~= nil
"#,
            json!(false),
        )
        .await
    }

    // The companion to the two above: proving the function *can* answer nil is
    // worth nothing on its own, because always answering nil would prove it
    // too. A hit must still be a real rectangle with a real extent.
    #[tokio::test]
    async fn test_free_rect_present_when_one_fits() {
        result_test(
            1,
            r#"
local r = world.find_free_resource_rect("iron-ore", 2, 2, {x=0,y=0})
result = {
  found = r ~= nil,
  width = r.right_bottom.x - r.left_top.x,
  height = r.right_bottom.y - r.left_top.y,
}
"#,
            json!({"found": true, "width": 2.0, "height": 2.0}),
        )
        .await
    }

    async fn result_test(bot_count: u8, code: &str, expected: serde_json::Value) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (result, _) = run_lua(&mut planner, code, None, &root, bot_count, None)
            .await
            .expect("run_lua failed");

        let actual = serde_json::to_string(&result.expect("no result found")).unwrap();
        let expected = serde_json::to_string(&expected).unwrap();

        assert_eq!(actual, expected);
    }
}
