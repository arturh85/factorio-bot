use crate::globals::create_lua_globals;
use crate::globals::goal::create_lua_goal;
use crate::globals::plan::create_lua_plan_builder;
use crate::globals::rcon::create_lua_rcon;
use crate::globals::world::create_lua_world;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::mlua::LuaSerdeExt;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::serde_json;
use factorio_bot_core::tokio::runtime::Runtime;
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use miette::{miette, IntoDiagnostic, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;

/// `scripts_root` bounds every filesystem operation the script can reach.
/// `filename` is only used for error messages and for resolving `include`
/// relative to the script's own directory; a `None` filename (inline code
/// from the editor) simply resolves relative to the root.
pub async fn run_lua(
    planner: &mut Planner,
    lua_code: &str,
    filename: Option<&str>,
    scripts_root: &Path,
    bot_count: u8,
    // Task 2 replaces this with an OutputSink
    redirect: bool,
) -> Result<(Option<serde_json::Value>, (String, String))> {
    let scripts_root = scripts_root.to_path_buf();
    let buffers = redirect_buffers(redirect);
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
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let lua_code = lua_code.to_owned();

    let plan_world = planner.plan_world.clone();
    let graph = planner.graph.clone();
    let real_world = planner.real_world.clone();
    let rcon = planner.rcon.clone();

    let thread_stdout = stdout.clone();
    let thread_stderr = stderr.clone();

    // Every fallible step below returns instead of unwrapping: this crate is
    // built with `panic = "abort"`, so a panic anywhere in here is a remote
    // kill of the whole server process, not a failed script.
    let result = thread::spawn(move || -> Result<Option<serde_json::Value>> {
        let lua = crate::sandbox::new_sandboxed_lua()
            .map_err(|err| miette!("failed to create the lua sandbox: {err}"))?;
        let _code_by_path = code_by_path.clone();
        let setup = (|| -> LuaResult<()> {
            let world = create_lua_world(
                &lua,
                plan_world.clone(),
                scripts_root.clone(),
                script_dir.clone(),
            )?;
            let goal = create_lua_goal(
                &lua,
                plan_world.clone(),
                real_world.clone(),
                rcon.clone(),
                all_bots.clone(),
            )?;
            let plan = create_lua_plan_builder(&lua, graph, plan_world)?;
            create_lua_globals(
                &lua,
                all_bots,
                scripts_root,
                script_dir,
                thread_stdout,
                thread_stderr,
                _code_by_path,
            )?;

            let globals = lua.globals();
            globals.set("world", world)?;
            globals.set("plan", plan)?;
            globals.set("goal", goal)?;
            if let Some(rcon) = rcon.as_ref() {
                let rcon = create_lua_rcon(&lua, rcon.clone(), real_world.clone())?;
                globals.set("rcon", rcon)?;
            }
            Ok(())
        })();
        let to_report = |err: LuaError| {
            let code_by_path = code_by_path.lock().clone();
            crate::error::to_lua_error(err, &code_by_path)
        };
        setup.map_err(to_report)?;

        let rt: Runtime = Runtime::new().into_diagnostic()?;
        rt.block_on(async {
            let chunk = lua.load(&lua_code).set_name(&filename);
            chunk.exec_async().await.map_err(to_report)?;
            // `result` is whatever the script assigned, so it can be a value
            // serde cannot represent (a function, a table with a cycle).
            // Unwrapping here would let a script abort the process.
            match lua.globals().get::<LuaValue>("result") {
                Ok(LuaValue::Nil) | Err(_) => Ok(None),
                Ok(value) => Ok(Some(lua.from_value(value).map_err(to_report)?)),
            }
        })
    })
    .join()
    .map_err(|_| miette!("lua thread panicked"))??;
    let stdout: String = stdout.lock().to_owned();
    let stderr: String = stderr.lock().to_owned();
    let buffers = buffers_to_string(&stdout, &stderr, buffers)?;
    Ok((result, buffers))
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::factorio::rcon::FactorioRcon;
    use factorio_bot_core::serde_json::json;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;
    use tokio::fs;

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
        let outcome = run_lua(&mut planner, code, None, &root, 1, false)
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
        let outcome = run_lua(&mut planner, code, None, &root, 1, false)
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
        let outcome = run_lua(&mut planner, code, None, &root, 1, false)
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
        let result = run_lua(&mut planner, &code, None, &root, 1, false)
            .await
            .map(|_| ());

        assert!(result.is_err(), "the write should have been refused");
        assert!(
            !inside.exists(),
            "the file was created: {}",
            inside.display()
        );
    }

    #[tokio::test]
    async fn an_out_of_range_direction_is_reported_not_panicked() {
        // `Direction::from_u8(9)` is `None`, and unwrapping it aborted.
        let (_dir, result) = sandboxed("direction_clockwise(9)").await;
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
        assert!(!dir
            .path()
            .parent()
            .expect("parent")
            .join("pwned.txt")
            .exists());
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

    #[tokio::test]
    async fn test_script() {
        let world = Arc::new(fixture_world());
        // draw_world(world.clone(), "tests/world_start.png");

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
            let (_result, (stdout, stderr)) = run_lua(
                &mut planner,
                include_str!("../tests/script.lua"),
                Some(relative_path),
                &repo_root(),
                bot_count,
                false,
            )
            .await
            .expect("run_lua failed");

            fs::write(
                format!(
                    "{}/tests/stdout-{}.txt",
                    env!("CARGO_MANIFEST_DIR"),
                    bot_count
                ),
                stdout,
            )
            .await
            .expect("failed to write");

            if !stderr.is_empty() {
                fs::write(
                    format!(
                        "{}/tests/stderr-{}.txt",
                        env!("CARGO_MANIFEST_DIR"),
                        bot_count
                    ),
                    stderr,
                )
                .await
                .expect("failed to write");
            }
        }
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

    async fn result_test(bot_count: u8, code: &str, expected: serde_json::Value) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (result, _) = run_lua(&mut planner, code, None, &root, bot_count, false)
            .await
            .expect("run_lua failed");

        let actual = serde_json::to_string(&result.expect("no result found")).unwrap();
        let expected = serde_json::to_string(&expected).unwrap();

        assert_eq!(actual, expected);
    }
}
