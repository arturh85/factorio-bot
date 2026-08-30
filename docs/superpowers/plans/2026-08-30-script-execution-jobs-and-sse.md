# Script Execution, Job Registry and SSE Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a browser start a Lua script over HTTP, watch its output stream in live, and see the result — without the script being able to read or write files outside the scripts directory, and without one script's run killing the server for everyone.

**Architecture:** Three layers, bottom-up. (1) The Lua host bindings get a filesystem sandbox rooted at the scripts directory and stop panicking on bad input. (2) `run_lua` loses the process-global `gag` stdout redirect in favour of an `OutputSink` trait, so output belongs to one run instead of to the process, and moves onto `spawn_blocking` so it stops parking a tokio worker for the length of a script. (3) The server grows a job registry — one script at a time, 409 with the running job's id otherwise — exposed as `POST /api/v1/scripts/execute`, `GET /api/v1/jobs`, `GET /api/v1/jobs/{id}` and an SSE stream at `GET /api/v1/jobs/{id}/events`.

**Tech Stack:** Rust, axum 0.8, utoipa 5.5 + utoipa-axum 0.2, tokio (broadcast + spawn_blocking), mlua 0.12, miette 7.

**Spec:** `docs/superpowers/specs/2026-08-29-webserver-replaces-tauri-gui-design.md`

**Predecessor:** `docs/superpowers/plans/2026-08-30-management-api-reads-and-mutations.md` (plan 3, complete). Its ledger is at `.superpowers/sdd/2026-08-30-management-api-reads-and-mutations/progress.md`; finding **I3** there is carried into this plan as Task 3.

---

## Global Constraints

- **`panic = "abort"` is set for the release profile** (root `Cargo.toml`). Any `.unwrap()`, `.expect()` or arithmetic overflow reachable from a request or from user Lua is a remote kill of the whole server, taking the Factorio instance and every other user's session with it. In code reachable from Lua or from an HTTP handler: no `unwrap`, no `expect`, no indexing that can be out of range. Return `LuaError::RuntimeError` / `ErrorResponse` instead.
- **The HTTP API has no authentication by design.** The user decided this; the bind address (default `127.0.0.1`) is the only control. This does *not* make the Lua sandbox pointless — it is what keeps a script written by someone else, pasted by the operator, from reading `~/.ssh/id_ed25519`. Never argue "there's no auth anyway" to skip a bounds check.
- **The scripts root is `ensure_scripts_dir(workspace_path)`** — `workspace_path/scripts`, canonicalized. It is never `./scripts` relative to the process's working directory. `factorio_bot_core::scripts::scripts_dir` is the old CWD-first resolver and must not gain new callers.
- **Every path check is canonicalize-then-`starts_with(root)`.** Never `contains("..")`, never a substring test.
- **Every new route is registered in `crates/server/src/manage/mod.rs`'s `router()` via `routes!`** and asserted by `crates/server/tests/openapi.rs`. Query-parameter structs carry `#[into_params(parameter_in = Query)]` — without it utoipa publishes them as *path* parameters (this was finding I1 of plan 3).
- **Mutation checks name an input class, not a line.** A spec that says "delete guard X and test Y goes red" is only valid when X is the *sole* gate on Y's input. Task 1 proved the failure mode: `resolve_write_path`'s three guards are mutually redundant, so deleting any one of them killed zero tests while the tests themselves were perfectly sound — an absolute path is caught by the `is_absolute` guard, a `..` path that escapes is caught by the `target` bound check even with the `parent` check gone, and each masks the other. Before writing a mutation step, ask which *input* only that guard rejects. If no such input exists, the guard cannot be defended by a test and the honest move is to say so in the report rather than manufacture a passing mutation.
- **A mutation must be observed to land.** Task 1's first sweep reported all-green and was wrong: the loop ran under zsh, which does not word-split unquoted variables, so the edit silently never applied. Assert the file actually changed before trusting the result.
- **A mutation that makes a test *hang* is not a signal.** Bound every test that consumes a stream with an explicit timeout, so the mutated behaviour is a failure with a name rather than a suite that never returns.
- **`git diff` in this repo lies to greps.** `diff.external` is set to difftastic, so `git diff` emits no `+`/`-` prefixed lines and any `grep "^+"` over it returns 0 — indistinguishable from "found nothing". Measured on a known one-line removal: `git diff … | grep -c "^-rusttype"` gives **0**, `git diff --no-ext-diff … | grep -c` gives **1**. `git show` and `git log` are *not* affected (they disable the external driver unless `--ext-diff` is passed), which is why review packages built with `git show` are genuine unified diffs. Pass `--no-ext-diff` to every `git diff` regardless — a verification command that cannot fail is worse than a wrong answer, because a wrong answer gets challenged and a false green does not.
- **Cargo runs inside the Nix devShell:** `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`.
- **Test names state the behaviour**, not the function under test: `a_script_cannot_write_outside_the_scripts_root`, not `test_file_write`.
- Every task ends green on `cargo clippy --workspace --all-features --all-targets -- --deny warnings` and `cargo fmt --all -- --check`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/scripts.rs` (modify) | Gains `resolve_write_path` — the bounded resolver for paths that need not exist yet. Also fixes `ensure_scripts_dir`'s partial-extraction hole. |
| `crates/scripting_lua/src/globals/globals.rs` (modify) | `include` / `file_read` / `file_write` bounded and non-panicking. |
| `crates/scripting_lua/src/globals/world.rs` (modify) | `world.draw` bounded and non-panicking. |
| `crates/core/src/test_utils.rs` (modify) | `draw_world` returns `Result` instead of `.unwrap()`ing the image save. |
| `crates/scripting/src/lib.rs` (modify) | `gag` gone. Gains the `OutputSink` trait and `Stream` enum. |
| `crates/scripting_lua/src/lua_runner.rs` (modify) | Takes `scripts_root` and an `OutputSink`; drops `redirect: bool`; runs on `spawn_blocking`. |
| `crates/scripting_lua/src/run_script.rs` (create) | `run_script_file` / `run_script` / `language_by_filename`, moved out of `app/src-tauri`, taking `scripts_root` explicitly. This is the I3 unification. |
| `app/src-tauri/src/scripting.rs` (modify) | Thin wrapper: read settings → call `factorio_bot_scripting_lua::run_script_file`. |
| `crates/server/src/jobs.rs` (create) | `JobRegistry`, `Job`, `JobStatus`, `JobEvent`; single-slot execution; broadcast fan-out. |
| `crates/server/src/manage/execute.rs` (create) | `POST /scripts/execute`, `GET /jobs`, `GET /jobs/{id}`, `GET /jobs/{id}/events`. |
| `crates/server/src/state.rs` (modify) | `AppState` gains `jobs: Arc<JobRegistry>`. |
| `crates/server/tests/manage_execute.rs` (create) | HTTP-level tests for the four routes. |

---

## Task 0: Security gate — verify the Lua filesystem escape is still open

This task produces no commit. It exists because Task 1 must not be written against a stale reading of the code, and because a second agent works in this checkout and may have touched these files.

**Files:**
- Read: `crates/scripting_lua/src/globals/world.rs:195-205`
- Read: `crates/scripting_lua/src/globals/globals.rs:47-102`
- Read: `crates/core/src/test_utils.rs:178-233`

- [ ] **Step 1: Confirm the four unbounded bindings still exist**

Each of these joins a caller-supplied string onto a root and hands it straight to the filesystem:

| Binding | Site | Sink |
|---|---|---|
| `world.draw(save_path)` | `world.rs:200` → `test_utils.rs:232` | `buffer.save(cwd.join(save_path)).unwrap()` |
| `globals.include(source_path)` | `globals.rs:52` | `fs::read_to_string(_cwd.join(&source_path)…)` then `chunk.exec()` |
| `globals.file_read(source_path)` | `globals.rs:78` | `fs::read_to_string(_cwd.join(source_path)…)` |
| `globals.file_write(target_path, contents)` | `globals.rs:99` | `fs::write(_cwd.join(target_path)…)` |

Record in the report whether all four are still in this shape. If another agent has already fixed one, say so and adapt Task 1 rather than reverting their fix.

- [ ] **Step 2: Confirm the escape is real, not theoretical**

`Path::join` replaces the whole path when its argument is absolute. Verified on this platform:

```
cwd = /home/user/workspace/scripts
cwd.join("/tmp/pwned-abc.png")   -> "/tmp/pwned-abc.png"
cwd.join("../../../pwned.png")   -> "/home/user/workspace/scripts/../../../pwned.png"
```

The first escapes outright. The second is never normalized before it reaches `fs::write` / `image::save`, and the OS resolves the `..` segments — so it escapes too.

- [ ] **Step 3: Confirm the third defect — a filename-less run panics**

`run_lua` computes `cwd` as:

```rust
let cwd = Path::new(&filename).parent().expect("failed to find cwd")
    .canonicalize().expect("failed to canonicalize");
```

with `filename` defaulting to `"unknown.lua"`. Verified: `Path::new("unknown.lua").parent()` is `Some("")`, and `Path::new("").canonicalize()` is an error. So **every inline-code run panics before executing a line** — and under `panic = "abort"` that is process death, reached by `POST /scripts/execute` with a `code` body. Task 1 fixes this by rooting `cwd` at `scripts_root` instead of deriving it from the filename.

**Gate:** do not begin Task 5 (job registry) or Task 6 (execute endpoints) until Task 1 has landed and its tests pass. Exposing `run_lua` over HTTP before the sandbox exists turns a local footgun into a network-reachable arbitrary file read/write.

---

## Task 1: Sandbox the Lua filesystem surface

**Files:**
- Modify: `crates/core/src/scripts.rs` (add `resolve_write_path`; fix `ensure_scripts_dir`)
- Modify: `crates/scripting_lua/src/globals/globals.rs:47-102`
- Modify: `crates/scripting_lua/src/globals/world.rs:11-14, 195-205`
- Modify: `crates/core/src/test_utils.rs:178, 232`
- Modify: `crates/scripting_lua/src/lua_runner.rs:18-60`
- Test: `crates/core/src/scripts.rs` (unit), `crates/scripting_lua/src/lua_runner.rs` (integration through Lua)

**Interfaces:**
- Consumes: `factorio_bot_core::scripts::{resolve_script_path, ScriptPathError}` (exists).
- Produces:
  - `factorio_bot_core::scripts::resolve_write_path(root: &Path, requested: &str) -> Result<PathBuf, ScriptPathError>`
  - `factorio_bot_core::test_utils::draw_world(world: Arc<FactorioWorld>, save_path: &Path) -> miette::Result<()>` — note the changed signature: it no longer takes a `cwd` and no longer joins; the caller passes an already-resolved absolute path.
  - `create_lua_globals(lua, all_bots, scripts_root: PathBuf, script_dir: PathBuf, stdout, stderr, code_by_path)` — `scripts_root` is the sandbox boundary, `script_dir` is what relative paths resolve against.
  - `create_lua_world(lua, world, scripts_root: PathBuf, script_dir: PathBuf)` — same pair.

### Why two roots

`include("lib.lua")` from `scripts/sub/foo.lua` must keep finding `scripts/sub/lib.lua`, so relative resolution stays anchored at the script's own directory. The *bound* is a separate question and is always the scripts root. Collapsing them into one value breaks either nested scripts or the boundary.

- [ ] **Step 1: Write the failing tests for `resolve_write_path`**

In `crates/core/src/scripts.rs`'s `mod tests`. Reuse the existing `root()` helper, which builds `scripts/` with `a.lua`, `sub/b.lua` and an `outside.txt` sibling of `scripts/`.

```rust
#[test]
fn a_write_path_may_name_a_file_that_does_not_exist_yet() {
    let (_dir, root) = root();
    let resolved = resolve_write_path(&root, "new.png").expect("accepted");
    assert_eq!(resolved, root.join("new.png"));
}

#[test]
fn a_write_path_may_name_a_file_in_an_existing_subdirectory() {
    let (_dir, root) = root();
    let resolved = resolve_write_path(&root, "sub/new.png").expect("accepted");
    assert_eq!(resolved, root.join("sub").join("new.png"));
}

#[test]
fn a_write_path_may_not_be_absolute() {
    let (_dir, root) = root();
    let err = resolve_write_path(&root, "/tmp/pwned.png").expect_err("refused");
    assert!(matches!(err, ScriptPathError::EscapesRoot { .. }), "got {err:?}");
}

#[test]
fn a_write_path_may_not_climb_out_with_dotdot() {
    let (_dir, root) = root();
    let err = resolve_write_path(&root, "../outside.txt").expect_err("refused");
    assert!(matches!(err, ScriptPathError::EscapesRoot { .. }), "got {err:?}");
}

#[test]
fn a_write_path_may_not_climb_out_and_back_in() {
    // The interesting case: this one *does* land inside the root, but only
    // after leaving it. `canonicalize` on the parent chain is what makes the
    // difference between checking the string and checking the destination.
    let (_dir, root) = root();
    let resolved = resolve_write_path(&root, "sub/../new.png").expect("accepted");
    assert_eq!(resolved, root.join("new.png"));
}

#[test]
fn a_write_path_may_not_target_a_directory_that_does_not_exist() {
    let (_dir, root) = root();
    let err = resolve_write_path(&root, "nope/new.png").expect_err("refused");
    assert!(matches!(err, ScriptPathError::NotFound { .. }), "got {err:?}");
}

#[test]
fn a_write_path_may_not_escape_through_a_symlinked_parent() {
    let (dir, root) = root();
    let outside_dir = dir.path().join("elsewhere");
    std::fs::create_dir_all(&outside_dir).expect("mkdir");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_dir, root.join("link")).expect("symlink");
    #[cfg(unix)]
    {
        let err = resolve_write_path(&root, "link/new.png").expect_err("refused");
        assert!(matches!(err, ScriptPathError::EscapesRoot { .. }), "got {err:?}");
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core scripts::tests'`
Expected: FAIL, `cannot find function resolve_write_path`.

- [ ] **Step 3: Implement `resolve_write_path`**

```rust
/// Resolves a path that is allowed not to exist yet, bounding it to `root`.
///
/// [`resolve_script_path`] canonicalizes the *target*, so it can only resolve
/// paths that already exist — right for reads, useless for `file_write` or
/// `world.draw`, whose whole point is creating something new. This resolves
/// the deepest existing ancestor instead and re-attaches the remainder, which
/// gives the same guarantee (`..` and symlinks are resolved by the OS, not by
/// us) for a destination that is not there yet.
///
/// The parent directory must already exist: creating intermediate directories
/// on a script's behalf would let a script build a tree of its own choosing,
/// and no caller needs it.
pub fn resolve_write_path(
    root: &Path,
    requested: &str,
) -> std::result::Result<PathBuf, ScriptPathError> {
    let requested_path = Path::new(requested);
    // An absolute argument makes `Path::join` discard `root` entirely, so it
    // must be refused before any joining happens.
    if requested_path.is_absolute() {
        return Err(ScriptPathError::EscapesRoot { requested: requested.to_owned() });
    }

    let joined = root.join(requested_path);
    let parent = joined.parent().ok_or_else(|| ScriptPathError::EscapesRoot {
        requested: requested.to_owned(),
    })?;
    let file_name = joined
        .file_name()
        .ok_or_else(|| ScriptPathError::EscapesRoot { requested: requested.to_owned() })?;

    // The parent must exist; canonicalizing it is what resolves `..` segments
    // and symlinks before the bounds check sees the path.
    let parent = std::fs::canonicalize(parent)
        .map_err(|_| ScriptPathError::NotFound { requested: requested.to_owned() })?;
    if !parent.starts_with(root) {
        return Err(ScriptPathError::EscapesRoot { requested: requested.to_owned() });
    }

    let target = parent.join(file_name);
    // Belt and braces: `file_name` is a single component by construction, so
    // this cannot fail today. It is here because `resolve_new_script_path` in
    // the server learned the hard way (plan 3, finding I4) that a component
    // can replace a path on Windows, and a bounds check that is cheap and
    // unconditional outlives the reasoning that made it redundant.
    if !target.starts_with(root) {
        return Err(ScriptPathError::EscapesRoot { requested: requested.to_owned() });
    }
    Ok(target)
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Same command. Expected: PASS, 7 new tests.

- [ ] **Step 5: Fix `ensure_scripts_dir`'s partial-extraction hole**

Carried from the plan-3 re-review. Today `ensure_scripts_dir` creates the directory and *then* extracts `PLANS_CONTENT`; if extraction fails partway, the now-existing directory reads as "already bootstrapped" forever and is never retried. Extract into a temporary sibling and rename into place, so the directory only appears once it is complete:

```rust
pub fn ensure_scripts_dir(workspace_path: &Path) -> Result<PathBuf> {
    let workspace_scripts = workspace_path.join("scripts");
    if !workspace_scripts.is_dir() {
        #[cfg(not(debug_assertions))]
        {
            // Build under a sibling name and rename in one step: a crash or a
            // full disk mid-extraction leaves no half-populated `scripts/`
            // that the next start would mistake for a finished one.
            let staging = workspace_path.join(".scripts-partial");
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).into_diagnostic()?;
            crate::process::instance_setup::PLANS_CONTENT
                .extract(staging.clone())
                .map_err(|err| miette!("failed to extract bundled scripts: {err:?}"))?;
            std::fs::rename(&staging, &workspace_scripts).into_diagnostic()?;
        }
        #[cfg(debug_assertions)]
        std::fs::create_dir_all(&workspace_scripts).into_diagnostic()?;
    }
    std::fs::canonicalize(&workspace_scripts).into_diagnostic()
}
```

The two existing tests (`ensure_scripts_dir_creates_a_missing_scripts_directory`, `ensure_scripts_dir_leaves_an_existing_directory_alone`) must still pass unchanged — do not edit them to accommodate this.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/scripts.rs
git commit -m "feat(core): add bounded resolve_write_path and make script bootstrap atomic"
```

- [ ] **Step 7: Write the failing Lua-level escape tests**

In `crates/scripting_lua/src/lua_runner.rs`'s `mod tests`. These run real Lua through the real binding, which is the only level at which the sandbox is actually proven.

```rust
/// Runs `code` with the sandbox rooted at a fresh temp directory and returns
/// the run's error, if any. The temp dir is returned so the caller can assert
/// on what did and did not appear on disk.
async fn sandboxed(code: &str) -> (tempfile::TempDir, Result<()>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
    let world = Arc::new(fixture_world());
    let mut planner = Planner::new(world, None);
    let outcome = run_lua(&mut planner, code, None, &root, 1, None).await.map(|_| ());
    (dir, outcome)
}

#[tokio::test]
async fn a_script_cannot_write_outside_the_scripts_root() {
    let victim = std::env::temp_dir().join(format!("pwned-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&victim);
    let code = format!("file_write({:?}, \"owned\")", victim.to_str().expect("utf8"));
    let (_dir, result) = sandboxed(&code).await;
    assert!(result.is_err(), "the write should have been refused");
    assert!(!victim.exists(), "the file was created outside the root: {}", victim.display());
}

#[tokio::test]
async fn a_script_cannot_climb_out_of_the_scripts_root() {
    let (dir, result) = sandboxed("file_write(\"../pwned.txt\", \"owned\")").await;
    assert!(result.is_err(), "the write should have been refused");
    assert!(!dir.path().parent().expect("parent").join("pwned.txt").exists());
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
    assert!(!victim.exists(), "the image was written outside the root: {}", victim.display());
}

#[tokio::test]
async fn a_script_may_write_inside_the_scripts_root() {
    // The counterpart that keeps the five tests above honest: if the bindings
    // were simply broken rather than bounded, they would all still pass.
    let (dir, result) = sandboxed("file_write(\"ok.txt\", \"fine\")").await;
    assert!(result.is_ok(), "an in-root write must still work: {result:?}");
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
```

- [ ] **Step 8: Run them and watch them fail**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-scripting-lua'`
Expected: compile failure first (`run_lua` does not yet take `scripts_root`), which is the correct kind of red for a signature change.

- [ ] **Step 9: Change `draw_world` to take a resolved path and return `Result`**

`crates/core/src/test_utils.rs`. Signature becomes:

```rust
pub fn draw_world(world: Arc<FactorioWorld>, save_path: &Path) -> miette::Result<()> {
```

and the last line becomes:

```rust
    buffer.save(save_path).into_diagnostic()
```

The `cwd: PathBuf` parameter is removed — joining is now the caller's job, and the caller is the only place that knows the sandbox root. Fix the other call sites the compiler points at.

- [ ] **Step 10: Bound the three `globals.rs` bindings**

Change `create_lua_globals`'s signature to take `scripts_root: PathBuf` and `script_dir: PathBuf` in place of the single `cwd`, then rewrite the three bindings. `include`:

```rust
let root = scripts_root.clone();
let dir = script_dir.clone();
map_table.set(
    "include",
    lua.create_function(move |lua, source_path: String| {
        let resolved = resolve_script_path(&root, &relative_to(&root, &dir, &source_path))
            .map_err(|err| LuaError::RuntimeError(err.to_string()))?;
        let content = fs::read_to_string(&resolved)
            .map_err(|err| LuaError::RuntimeError(format!("{}: {err}", source_path)))?;
        let mut code_by_path_lock = code_by_path.lock();
        code_by_path_lock.insert(source_path.clone(), content.clone());
        drop(code_by_path_lock);
        lua.load(&content).set_name(&source_path).exec()
    })?,
)?;
```

`file_read` is the same shape without the `exec`. `file_write` uses `resolve_write_path` instead of `resolve_script_path`:

```rust
let root = scripts_root.clone();
let dir = script_dir.clone();
map_table.set(
    "file_write",
    lua.create_function(move |_lua, (target_path, contents): (String, String)| {
        let resolved = resolve_write_path(&root, &relative_to(&root, &dir, &target_path))
            .map_err(|err| LuaError::RuntimeError(err.to_string()))?;
        fs::write(&resolved, contents)
            .map_err(|err| LuaError::RuntimeError(format!("{}: {err}", target_path)))
    })?,
)?;
```

with this helper, private to the module — it turns a script-relative request into a root-relative one so both resolvers can do their bounds check against the root:

```rust
/// Re-expresses a script-relative path as a path relative to the sandbox root.
///
/// Relative paths in a script mean "next to this script", so they are joined
/// onto `script_dir` first. The resolvers bound against `root`, so the result
/// is handed back as a root-relative string. An argument that is absolute, or
/// one that lands outside the root, is passed through untouched — the
/// resolvers refuse it, and refusing it *there* keeps one rejection path
/// instead of two.
fn relative_to(root: &Path, script_dir: &Path, requested: &str) -> String {
    let joined = script_dir.join(requested);
    match joined.strip_prefix(root) {
        Ok(rest) => rest.to_string_lossy().into_owned(),
        Err(_) => requested.to_owned(),
    }
}
```

Note the three `.to_str().unwrap()` calls at `globals.rs:52,78,99` disappear with this rewrite: they panic on any non-UTF-8 path, and passing the `Path` straight to `fs::read_to_string` / `fs::write` never needed the string anyway.

- [ ] **Step 11: Bound `world.draw`**

`world.rs`. `create_lua_world` takes `scripts_root` and `script_dir` in place of `cwd`, and the binding becomes:

```rust
let root = scripts_root.clone();
let dir = script_dir.clone();
map_table.set(
    "draw",
    lua.create_function(move |_lua, save_path: String| {
        let resolved = resolve_write_path(&root, &relative_to(&root, &dir, &save_path))
            .map_err(|err| LuaError::RuntimeError(err.to_string()))?;
        draw_world(world.clone(), &resolved)
            .map_err(|err| LuaError::RuntimeError(format!("{err}")))
    })?,
)?;
```

`relative_to` is needed in both modules; put it in `crates/scripting_lua/src/globals/mod.rs` as `pub(crate) fn relative_to` and use it from both rather than copying it.

- [ ] **Step 12: Root `cwd` at the scripts root in `run_lua`**

`lua_runner.rs`. Replace the panicking derivation:

```rust
    let cwd = Path::new(&filename).parent().expect(…).canonicalize().expect(…);
```

with a `scripts_root: &Path` parameter and a script directory derived without panicking:

```rust
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
    sink: Option<Arc<dyn OutputSink>>,
) -> Result<(Option<serde_json::Value>, (String, String))> {
    let scripts_root = scripts_root.to_path_buf();
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
```

The `redirect: bool` parameter is replaced by `sink` in Task 2; write the signature with `sink` now and pass `None` through until Task 2 gives it a body. Update the existing tests in this file for the new signature (they pass a temp root, not `"./test.lua"`).

- [ ] **Step 13: Run the Lua sandbox tests and watch them pass**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-scripting-lua'`
Expected: PASS, including `a_script_may_write_inside_the_scripts_root` and `a_run_without_a_filename_does_not_panic`.

- [ ] **Step 14: Prove the guard is load-bearing (mutation)**

Delete the `if !parent.starts_with(root)` check in `resolve_write_path` and re-run. `a_script_cannot_write_outside_the_scripts_root`, `a_script_cannot_climb_out_of_the_scripts_root` and `a_script_cannot_draw_the_world_outside_the_scripts_root` must all go red. Record the failure set in the report — pass/fail counts alone are not evidence. Restore the check and confirm green.

- [ ] **Step 15: Commit**

```bash
git add crates/scripting_lua crates/core/src/test_utils.rs
git commit -m "fix(lua): bound every filesystem binding to the scripts root

world.draw, include, file_read and file_write all joined a caller-supplied
string onto a root and handed it to the filesystem. An absolute argument made
Path::join discard the root entirely, so any script could read or write
anywhere the server process could. All four now resolve through the bounded
resolvers and return a Lua error instead of panicking -- which, under
panic=abort, was killing the process."
```

---

## Task 1b: Harden the `rcon.*` Lua bindings (GATE for Task 6)

Task 1's review scope stopped at `globals.rs` and `world.rs`. `crates/scripting_lua/src/globals/rcon.rs` carries **26** `unwrap`/`expect` calls of the same class, and `create_lua_rcon` is registered into the interpreter at `lua_runner.rs:89` whenever an RCON handle exists — which is exactly the configuration the HTTP execute endpoint runs in.

**Files:**
- Modify: `crates/scripting_lua/src/globals/rcon.rs`
- Test: `crates/scripting_lua/src/lua_runner.rs`

### Two distinct classes, both real, verified by counting

**10 are attacker input.** `position.get("x").unwrap()` at `:239`, `:291`, `:327` and the `search_center` pair at `:57-58` read fields off a Lua table the script supplies. `rcon.player_mine({}, ...)` — an empty table — panics. Under `panic = "abort"` that is one line of Lua killing the server.

**16 are availability.** `_rcon.as_ref().print(..).await.unwrap()` at `:90`, `:166`, and thirteen more, panic when the *RCON call itself* fails. No attacker needed: if the Factorio server drops its connection mid-script, or the game is stopped while a script runs, the bot server dies with it. This class is not a security bug and would not have been found by looking for one — it is a crash on the ordinary unhappy path.

- [ ] **Step 1: Write the failing tests**

In `lua_runner.rs`'s `mod tests`, reusing the `sandboxed` helper. These run with no RCON handle, so they exercise the argument parsing that happens *before* any call:

```rust
#[tokio::test]
async fn an_rcon_binding_given_a_table_without_coordinates_reports_an_error() {
    // rcon.* is only registered when a handle exists, so build the planner
    // with one; the call must fail on the missing field, not on the socket.
    let (_dir, result) = sandboxed_with_rcon("rcon.player_mine(1, {}, \"iron-ore\", 1)").await;
    assert!(result.is_err(), "a table with no x/y must be refused");
    assert_reported_not_panicked(&result);
}

#[tokio::test]
async fn an_rcon_binding_given_a_non_numeric_coordinate_reports_an_error() {
    let (_dir, result) =
        sandboxed_with_rcon("rcon.player_mine(1, {x=\"north\", y=0}, \"iron-ore\", 1)").await;
    assert!(result.is_err(), "a non-numeric coordinate must be refused");
    assert_reported_not_panicked(&result);
}
```

`assert_reported_not_panicked` already exists from commit `05775a5` — it checks the error is not the `"lua thread panicked"` message produced by the join handler, which is the distinction that matters. A plain `is_err()` passes against the vulnerable code.

- [ ] **Step 2: Run them and watch them fail with a panic, not an error**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-scripting-lua'`
Expected: the interpreter thread panics at `rcon.rs:291` and the assertion reports it.

- [ ] **Step 3: Replace both classes**

Field reads become a helper, since the same pair appears five times:

```rust
/// Reads an `{x=, y=}` table from Lua, refusing anything else.
///
/// The previous `table.get("x").unwrap()` panicked on a missing or
/// non-numeric field, and under `panic = "abort"` that ends the process
/// rather than the script.
fn position_from_lua(table: &LuaTable, argument: &str) -> LuaResult<Position> {
    let x: f64 = table
        .get("x")
        .map_err(|_| LuaError::RuntimeError(format!("{argument}: expected a number at `x`")))?;
    let y: f64 = table
        .get("y")
        .map_err(|_| LuaError::RuntimeError(format!("{argument}: expected a number at `y`")))?;
    Ok(Position::new(x, y))
}
```

RCON results become `.map_err(|err| LuaError::RuntimeError(format!("rcon: {err}")))?`.

- [ ] **Step 4: Run and watch them pass**

- [ ] **Step 5: Lock the class out**

Add `#![deny(clippy::unwrap_used, clippy::expect_used)]` at the top of `rcon.rs`, matching what Task 1's fix round put on `globals.rs` and `world.rs`. Do **not** put it on `globals/mod.rs` — that propagates into `plan.rs`, which another session is deleting.

- [ ] **Step 6: Mutation**

Restore one `position.get("x").unwrap()`. `an_rcon_binding_given_a_table_without_coordinates_reports_an_error` must fail *and* report a thread panic. Assert the edit landed before trusting the result. Then re-introduce one `unwrap` anywhere in the file and confirm the `deny` lint fails the build. Restore both.

- [ ] **Step 7: Commit**

```bash
git add crates/scripting_lua/src/globals/rcon.rs crates/scripting_lua/src/lua_runner.rs
git commit -m "fix(lua): stop rcon bindings panicking on bad input and rcon failure"
```

**Do not touch `plan.rs`.** Its 10 panics go with the file when the other session deletes it.

---

## Task 2: Replace the process-global stdout redirect with an output sink

**Files:**
- Modify: `crates/scripting/src/lib.rs`
- Modify: `crates/scripting/Cargo.toml` (drop `gag`)
- Modify: `crates/scripting_lua/src/lua_runner.rs`
- Modify: `crates/scripting_lua/src/globals/globals.rs` (the `print` bindings)
- Test: `crates/scripting_lua/src/lua_runner.rs`

**Interfaces:**
- Produces: `factorio_bot_scripting::{OutputSink, Stream}`. Removes `redirect_buffers` and `buffers_to_string`.

### Why `gag` has to go

`gag::BufferRedirect` replaces the *process's* file descriptors 1 and 2. With a single desktop app running one script at a time that was merely untidy. In a server it is wrong three ways: it swallows the server's own `tracing` output for the duration of a run, it cannot attribute a line to a job, and it hands back one string at the end when SSE needs lines as they happen.

- [ ] **Step 1: Write the failing test**

In `crates/scripting_lua/src/lua_runner.rs`'s `mod tests`:

```rust
#[derive(Default)]
struct RecordingSink {
    lines: Mutex<Vec<(Stream, String)>>,
}

impl OutputSink for RecordingSink {
    fn line(&self, stream: Stream, text: &str) {
        self.lines.lock().push((stream, text.to_owned()));
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
```

- [ ] **Step 2: Run it and watch it fail**

Expected: FAIL, `OutputSink` not found.

- [ ] **Step 3: Define the sink in `crates/scripting`**

Replace `redirect_buffers` and `buffers_to_string` (delete both, and the `use gag::BufferRedirect;` / `use std::io::Read;` lines) with:

```rust
/// Which of a script's two output streams a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Receives a script's output one line at a time, as it is produced.
///
/// This exists because the previous implementation redirected the *process's*
/// file descriptors with `gag`, which is unusable in a server: it captures the
/// server's own logging along with the script's, it cannot say which job a
/// line belongs to, and it only yields anything once the run is over. An
/// implementation must be cheap and must not block — it is called from inside
/// the Lua interpreter, with the interpreter's lock held.
pub trait OutputSink: Send + Sync {
    fn line(&self, stream: Stream, text: &str);
}
```

Then remove `gag = "^1.0"` from `crates/scripting/Cargo.toml`.

- [ ] **Step 4: Feed the sink from the `print` bindings**

`globals.rs` already accumulates into `Arc<Mutex<String>>`. Keep that (the full transcript is still returned at the end) and add the sink call beside it. `create_lua_globals` takes `sink: Option<Arc<dyn OutputSink>>`.

The three bindings are **`print` (`:163`), `print_err` (`:185`) and `print_warn` (`:208`)** — not `print`/`warn`/`error`, which is what an earlier draft of this task claimed and which do not exist. Verified against the registered names.

**Their current stream assignment is not what you would guess, and you must preserve it:**

| binding | accumulates into | prefix |
|---|---|---|
| `print` | `stdout` | none |
| `print_err` | `stderr` | `"ERROR: "` |
| `print_warn` | **`stdout`** | `"WARN: "` |

`print_warn` writes to **stdout**, not stderr (`:211` takes `stdout_lock`). Map it to `Stream::Stdout`. Moving it to stderr would be a silent behaviour change smuggled inside a refactor — the SSE consumer in plan 5 splits on stream, so a script's warnings would move panes. If that assignment is wrong it is wrong today and should be changed deliberately, in its own commit, not here.

Each binding does:

```rust
let text = strings.iter().join(" ");
info!("<cyan>lua</>   ⮞ {text}");
if let Some(sink) = sink.as_ref() {
    sink.line(Stream::Stdout, &text);
}
let mut stdout_lock = _stdout.lock();
```

`print_err` uses `Stream::Stderr`; `print` and `print_warn` use `Stream::Stdout`, matching the table above. The sink call goes *beside* the existing accumulation, reading from the same values — do not refactor the accumulation into the sink, because the returned transcript and the streamed lines must stay identical.

- [ ] **Step 5: Drop `redirect` from `run_lua`**

Remove the `buffers` local and the `buffers_to_string` call; return the two accumulated strings directly. `run_lua`'s signature is the one written in Task 1 Step 12 — this step gives `sink` its meaning.

- [ ] **Step 6: Run the tests and watch them pass**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-scripting-lua -p factorio-bot-scripting'`

- [ ] **Step 7: Confirm `gag` is gone from the lock file**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo tree -p factorio-bot-scripting'
```
Expected: no `gag` in the output. If `Cargo.lock` still lists it, some other crate pulls it in — find out which before proceeding.

- [ ] **Step 8: Commit**

```bash
git add crates/scripting crates/scripting_lua
git commit -m "refactor(scripting): replace the gag stdout redirect with an OutputSink

gag replaced the process's fd 1 and 2, so a running script swallowed the
server's own logging, no line could be attributed to a job, and nothing was
available until the run finished. A sink trait gives per-run, per-stream,
line-at-a-time output, which is what the SSE endpoint needs."
```

---

## Task 3: Unify script path resolution (plan 3, finding I3)

**Files:**
- Create: `crates/scripting_lua/src/run_script.rs`
- Modify: `crates/scripting_lua/src/lib.rs` (export it)
- Modify: `app/src-tauri/src/scripting.rs` (becomes a wrapper)
- Modify: `app/src-tauri/src/gui/command/script.rs:32,69`
- Modify: `app/src-tauri/src/repl/run_script.rs:29,50`
- Modify: `app/src-tauri/src/cli/lua.rs:126,168`
- Test: `crates/scripting_lua/src/run_script.rs`

**Interfaces:**
- Consumes: `factorio_bot_core::scripts::{ensure_scripts_dir, resolve_script_path}`; `run_lua` from Tasks 1-2.
- Produces:
  - `factorio_bot_scripting_lua::run_script_file(planner: &mut Planner, scripts_root: &Path, requested: &str, bot_count: u8, sink: Option<Arc<dyn OutputSink>>) -> Result<(String, String)>`
  - `factorio_bot_scripting_lua::run_script(planner: &mut Planner, language: &str, code: &str, scripts_root: &Path, bot_count: u8, sink: Option<Arc<dyn OutputSink>>) -> Result<(String, String)>`
  - `factorio_bot_scripting_lua::language_by_filename(filename: &str) -> Option<&'static str>`

### The defect being fixed

`app/src-tauri/src/scripting.rs:46` resolves through `scripts_dir`, which tries `./scripts` and `../../scripts` relative to the process's working directory *before* the workspace. The HTTP handlers resolve through `scripts_root`, which is workspace-only. So from a repository checkout, `PUT /api/v1/scripts/foo.lua` writes `workspace/scripts/foo.lua` while `execute` would run `./scripts/foo.lua` — the editor and the executor disagree about which file a name means. It also still uses `path.contains("..")`, the substring check the codebase rejected everywhere else.

- [ ] **Step 1: Write the failing test**

In `crates/scripting_lua/src/run_script.rs`:

```rust
#[tokio::test]
async fn a_script_is_read_from_the_given_root_not_from_the_working_directory() {
    // The regression this file exists to prevent: `scripts_dir` preferred
    // `./scripts` relative to the process CWD over the workspace, so the
    // editor and the executor resolved the same name to different files.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
    std::fs::write(root.join("hello.lua"), "print(\"from the root\")").expect("write");

    // A decoy with the same name under the process's working directory. If
    // resolution ever consults the CWD again, this is what would run.
    let cwd_scripts = std::env::current_dir().expect("cwd").join("scripts");
    let decoy_exists = cwd_scripts.join("hello.lua").exists();
    assert!(!decoy_exists, "test precondition: no ./scripts/hello.lua in the checkout");

    let world = Arc::new(fixture_world());
    let mut planner = Planner::new(world, None);
    let (stdout, _stderr) = run_script_file(&mut planner, &root, "/hello.lua", 1, None)
        .await
        .expect("run_script_file failed");
    assert!(stdout.contains("from the root"), "stdout was {stdout:?}");
}

#[tokio::test]
async fn a_script_outside_the_root_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
    let world = Arc::new(fixture_world());
    let mut planner = Planner::new(world, None);
    let err = run_script_file(&mut planner, &root, "../../etc/hostname", 1, None)
        .await
        .expect_err("should be refused");
    assert!(format!("{err}").contains("escapes"), "error was {err}");
}

#[tokio::test]
async fn a_non_script_extension_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
    std::fs::write(root.join("notes.txt"), "hello").expect("write");
    let world = Arc::new(fixture_world());
    let mut planner = Planner::new(world, None);
    let err = run_script_file(&mut planner, &root, "notes.txt", 1, None)
        .await
        .expect_err("should be refused");
    assert!(format!("{err}").contains("extension"), "error was {err}");
}
```

- [ ] **Step 2: Run and watch it fail**

Expected: FAIL, module does not exist.

- [ ] **Step 3: Implement `run_script.rs`**

```rust
use crate::run_lua;
use factorio_bot_core::miette::{miette, IntoDiagnostic, Result};
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::scripts::resolve_script_path;
use factorio_bot_scripting::OutputSink;
use std::path::Path;
use std::sync::Arc;

/// Maps a filename to the scripting language that runs it.
pub fn language_by_filename(filename: &str) -> Option<&'static str> {
    match Path::new(filename).extension()?.to_str()? {
        "lua" => Some("lua"),
        _ => None,
    }
}

/// Runs a script named relative to `scripts_root`.
///
/// `scripts_root` is passed in rather than looked up: the old version read
/// global settings and then consulted `scripts_dir`, which prefers `./scripts`
/// relative to the process's working directory. That made the same name mean
/// different files to the editor and to the executor (plan 3, finding I3).
/// There is now exactly one resolution, and its root is the caller's to state.
pub async fn run_script_file(
    planner: &mut Planner,
    scripts_root: &Path,
    requested: &str,
    bot_count: u8,
    sink: Option<Arc<dyn OutputSink>>,
) -> Result<(String, String)> {
    let resolved = resolve_script_path(scripts_root, requested).map_err(|err| miette!("{err}"))?;
    if !resolved.is_file() {
        return Err(miette!("path is not a file: {requested}"));
    }
    let language = language_by_filename(requested)
        .ok_or_else(|| miette!("unknown scripting file extension: {requested}"))?;
    let code = std::fs::read_to_string(&resolved).into_diagnostic()?;
    // The resolved absolute path, not the request: `include` resolves relative
    // to the script's own directory, and errors should name the real file.
    let filename = resolved.to_string_lossy().into_owned();
    match language {
        "lua" => run_lua(planner, &code, Some(&filename), scripts_root, bot_count, sink)
            .await
            .map(|outcome| outcome.1),
        other => Err(miette!("unknown language: \"{other}\"")),
    }
}

/// Runs code that has no file behind it (the editor's "run selection").
pub async fn run_script(
    planner: &mut Planner,
    language: &str,
    code: &str,
    scripts_root: &Path,
    bot_count: u8,
    sink: Option<Arc<dyn OutputSink>>,
) -> Result<(String, String)> {
    match language {
        "lua" => run_lua(planner, code, None, scripts_root, bot_count, sink)
            .await
            .map(|outcome| outcome.1),
        other => Err(miette!("unknown language: \"{other}\"")),
    }
}
```

- [ ] **Step 4: Run and watch it pass**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-scripting-lua'`

- [ ] **Step 5: Rewire the four callers**

`app/src-tauri/src/scripting.rs` shrinks to a settings-reading wrapper:

```rust
/// Resolves the workspace scripts root from settings, then delegates.
/// The resolution itself lives in `factorio_bot_scripting_lua` so the HTTP
/// server and the CLI cannot drift apart about what a script name means.
pub async fn run_script_file(
  planner: &mut Planner,
  path: &str,
  bot_count: u8,
) -> miette::Result<(String, String)> {
  let app_settings = load_app_settings()?;
  let workspace_path = PathBuf::from(app_settings.factorio.workspace_path.to_string());
  let scripts_root = factorio_bot_core::scripts::ensure_scripts_dir(&workspace_path)?;
  factorio_bot_scripting_lua::run_script_file(planner, &scripts_root, path, bot_count, None).await
}
```

Note `load_app_settings().unwrap()` becomes `?` — that `unwrap` is reachable from the GUI and from `serve`.

Then update the call sites for the dropped `redirect` argument:
- `gui/command/script.rs:32` — `run_script_file(&mut planner, &path[1..], bot_count)`; also drop the `&path[1..]` slicing, which panics on an empty `path` — `resolve_script_path` strips the leading `/` itself.
- `gui/command/script.rs:69` — `run_script(&mut planner, &language, &code, &scripts_root, bot_count, None)`.
- `repl/run_script.rs:29` and `cli/lua.rs:126,168` — drop the trailing `false`.

- [ ] **Step 6: Full build**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --workspace --all-features && cargo clippy --workspace --all-features --all-targets -- --deny warnings'`

- [ ] **Step 7: Commit**

```bash
git add crates/scripting_lua app/src-tauri/src
git commit -m "fix(scripting): resolve script names against one root, not two

run_script_file went through scripts_dir, which prefers ./scripts relative to
the process CWD, while the HTTP handlers resolve workspace-only. From a
checkout the editor wrote workspace/scripts/foo.lua and the executor ran
./scripts/foo.lua. Resolution now lives in one place and takes its root as an
argument. Closes finding I3 of the management-API plan."
```

---

## Task 4: Run scripts off the async executor

**Files:**
- Modify: `crates/scripting_lua/src/lua_runner.rs:80-95`
- Test: `crates/scripting_lua/src/lua_runner.rs`

### The defect

`run_lua` is `async`, but its body is `thread::spawn(…).join().unwrap()` — a synchronous block for the entire duration of the script. Awaiting it parks a tokio worker thread until the script finishes. With axum's default worker count, a handful of concurrent long scripts starve the whole server, including the health endpoint. The `.join().unwrap()` also re-panics on the calling thread, which under `panic = "abort"` kills the process rather than failing the request.

- [ ] **Step 1: Write the failing test**

```rust
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

    run_lua(&mut planner, code, None, &root, 1, None).await.expect("run_lua failed");
    ticker.await.expect("ticker panicked");
    assert!(
        ticks.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "the runtime made no progress on other tasks while the script ran"
    );
}
```

- [ ] **Step 2: Run it**

On two worker threads this may pass by luck even before the fix — the test's value is as a regression guard once `spawn_blocking` is in. Record whichever it does; do not weaken it to force red.

### Runtime ownership — decided here, because Task 5 inherits it

`run_lua` builds its own runtime inside the spawned thread (`lua_runner.rs:109`) and drops it when `block_on` returns. Dropping a tokio runtime aborts every task spawned onto it that has not finished — **silently**, with no error and no log line. So any asynchronous work a binding starts and the script does not explicitly wait for is killed the moment the script returns.

That is invisible today because every binding awaits its own work inline. It stops being invisible the moment a binding hands the script a handle and lets it walk away, which is the shape the other session's `goal.execute` takes: `goal.execute(..)` returns a run handle and `goal.wait(h)` blocks. A script that calls the first and not the second gets its bots stopped mid-plan with no diagnostic.

This is a job-lifetime question, not a binding question, which is why it is settled here rather than left to whoever writes the next binding. Two rules:

1. **A job is one `run_lua` call, and it is not finished while work it started is still running.** Do not redefine a job as "the script returned". Plan 5's UI polls job status; a job that reports `succeeded` while bots are still moving is lying to the operator.
2. **Outstanding work is awaited, never silently dropped.** `run_lua` gains a handle registry that bindings register spawned work into, and awaits everything in it after the chunk finishes and before returning.

The registry is the seam: this task provides it, bindings opt in. `goal.*` is another session's and will register into it; nothing in this repository does today, so the registry starts empty and the behaviour is unchanged until someone uses it.

```rust
/// Work a binding spawned that must finish before the run is considered over.
///
/// The runtime that `run_lua` builds dies with the call. Anything spawned onto
/// it and not awaited is aborted with no error and no log line, so a binding
/// that hands a script a handle and lets it walk away would have its work
/// killed the moment the script returned. Registering here makes the run wait.
#[derive(Default, Clone)]
pub struct PendingWork(Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>);

impl PendingWork {
    pub fn register(&self, handle: tokio::task::JoinHandle<()>) {
        self.0.lock().push(handle);
    }

    /// Awaits everything registered. A panicking task is reported, not
    /// propagated: one background task dying must not abort the process.
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
```

`run_lua` calls `pending.drain().await` inside `block_on`, after `chunk.exec_async()` and before the runtime is dropped, and folds any returned failures into the run's stderr.

**Do not** try to solve this by moving the runtime up to the job registry and sharing it across runs. That couples every script's lifetime to every other's and makes one runaway script's tasks outlive the job that owns them — the opposite of what the registry is for.

- [ ] **Step 3: Move the work onto `spawn_blocking`**

Replace `thread::spawn(move || { … }).join().unwrap()?` with:

```rust
    // `spawn_blocking`, not `thread::spawn().join()`: the old form blocked the
    // calling worker for the whole script, so a few concurrent runs starved
    // every other task including /api/v1/health. It also re-panicked the
    // child's panic on the caller's thread, which under panic=abort takes the
    // process down instead of failing one request.
    let result = tokio::task::spawn_blocking(move || {
        // … unchanged body …
    })
    .await
    .map_err(|err| miette!("script task failed: {err}"))??;
```

`JoinError` covers both a panic inside the script host and cancellation, and both become an error on this request rather than a process abort.

- [ ] **Step 4: Run the whole crate's tests**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-scripting-lua'`

- [ ] **Step 5: Commit**

```bash
git add crates/scripting_lua/src/lua_runner.rs
git commit -m "perf(lua): run scripts on spawn_blocking instead of blocking a worker"
```

---

## Task 5: Job registry

**Files:**
- Create: `crates/server/src/jobs.rs`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/state.rs`
- Modify: `crates/server/Cargo.toml`
- Test: `crates/server/src/jobs.rs` (unit)

**Interfaces:**
- Produces:
  - `JobId(u64)` — `Display`s as the decimal number; `serde` as a string, so the JSON contract does not depend on a number type.
  - `JobStatus { Running, Succeeded, Failed }`
  - `JobEvent { Output { stream, text }, Finished { status } }`
  - `Job { id, script: Option<String>, status, started_at_ms, finished_at_ms: Option<u64>, stdout, stderr, error: Option<String> }`
  - `JobRegistry::new(history_limit: usize)`
  - `JobRegistry::try_start(&self, script: Option<String>) -> Result<JobHandle, JobId>` — `Err(running_id)` when one is already running
  - `JobRegistry::subscribe(&self, id: JobId) -> Option<broadcast::Receiver<JobEvent>>`
  - `JobRegistry::get(&self, id: JobId) -> Option<Job>`, `JobRegistry::list(&self) -> Vec<Job>`
  - `JobHandle` implements `OutputSink`; `JobHandle::finish(self, outcome: Result<(String, String)>)`

### Design notes for the implementer

- **One at a time.** The user's decision: script execution is serialized.
- **A job is not finished when the script returns — it is finished when `run_lua` returns**, which Task 4 makes wait for work the script spawned and did not await. Do not shortcut this by completing the job on chunk exit. `try_start` takes the single slot or returns the occupant's id, atomically under one lock. Do not implement a queue.
- **Ids are a `u64` counter, not UUIDs.** The registry is per-process and dies with it, so a counter is sufficient and adds no dependency. Serialize as a string so the frontend never sees a JavaScript number large enough to lose precision.
- **`broadcast`, not `mpsc`.** Several browser tabs may watch the same job. Capacity 256; a slow subscriber that lags gets `RecvError::Lagged`, which the SSE handler turns into a visible gap rather than dropping the connection.
- **History is capped** at `history_limit` finished jobs, oldest evicted. Without a cap a long-lived server accumulates every script's full stdout forever.
- **Timestamps** come from `SystemTime::now().duration_since(UNIX_EPOCH)`. That call returns a `Result`; a clock before 1970 yields `0`, never a panic.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_second_start_is_refused_while_one_is_running() {
    let registry = JobRegistry::new(8);
    let first = registry.try_start(Some("a.lua".into())).expect("first start");
    let refused = registry.try_start(Some("b.lua".into())).expect_err("second refused");
    assert_eq!(refused, first.id(), "the refusal must name the job that holds the slot");
}

#[test]
fn the_slot_frees_when_a_job_finishes() {
    let registry = JobRegistry::new(8);
    let first = registry.try_start(Some("a.lua".into())).expect("first start");
    first.finish(Ok(("out".into(), String::new())));
    registry.try_start(Some("b.lua".into())).expect("slot is free again");
}

#[test]
fn the_slot_frees_even_when_a_job_fails() {
    // The failure mode that would wedge the server permanently: an error path
    // that returns without releasing the slot leaves every later request 409.
    let registry = JobRegistry::new(8);
    let first = registry.try_start(Some("a.lua".into())).expect("first start");
    first.finish(Err(miette!("boom")));
    registry.try_start(Some("b.lua".into())).expect("slot is free after a failure");
}

#[test]
fn a_finished_job_records_its_output_and_status() {
    let registry = JobRegistry::new(8);
    let handle = registry.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    handle.finish(Ok(("hello".into(), String::new())));
    let job = registry.get(id).expect("job is retained");
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.stdout, "hello");
    assert!(job.finished_at_ms.is_some());
}

#[test]
fn a_failed_job_records_the_error_message() {
    let registry = JobRegistry::new(8);
    let handle = registry.try_start(None).expect("start");
    let id = handle.id();
    handle.finish(Err(miette!("script exploded")));
    let job = registry.get(id).expect("job is retained");
    assert_eq!(job.status, JobStatus::Failed);
    assert!(job.error.as_deref().unwrap_or_default().contains("script exploded"));
}

#[test]
fn subscribers_see_output_lines_and_the_terminal_event() {
    let registry = JobRegistry::new(8);
    let handle = registry.try_start(Some("a.lua".into())).expect("start");
    let mut rx = registry.subscribe(handle.id()).expect("subscribed");
    handle.line(Stream::Stdout, "first");
    handle.finish(Ok(("first".into(), String::new())));

    assert!(matches!(
        rx.try_recv().expect("an output event"),
        JobEvent::Output { stream: Stream::Stdout, ref text } if text == "first"
    ));
    assert!(matches!(
        rx.try_recv().expect("a terminal event"),
        JobEvent::Finished { status: JobStatus::Succeeded }
    ));
}

#[test]
fn history_is_capped_and_evicts_the_oldest() {
    let registry = JobRegistry::new(2);
    let mut ids = Vec::new();
    for name in ["a.lua", "b.lua", "c.lua"] {
        let handle = registry.try_start(Some(name.into())).expect("start");
        ids.push(handle.id());
        handle.finish(Ok((String::new(), String::new())));
    }
    assert!(registry.get(ids[0]).is_none(), "the oldest job should have been evicted");
    assert!(registry.get(ids[2]).is_some(), "the newest job should be retained");
    assert_eq!(registry.list().len(), 2);
}
```

- [ ] **Step 2: Run and watch them fail**

`nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server jobs'`
Expected: FAIL, module does not exist.

- [ ] **Step 3: Implement `jobs.rs`**

Use `std::sync::Mutex` around a single `RegistryInner { next_id: u64, running: Option<JobId>, history: VecDeque<Job> }`, plus a `HashMap<JobId, broadcast::Sender<JobEvent>>` for live subscriptions, pruned when a job finishes. Every lock acquisition uses `.lock().unwrap_or_else(PoisonError::into_inner)` — a poisoned mutex must degrade, not abort. `JobHandle` holds an `Arc<JobRegistry>` and its `JobId`; its `OutputSink::line` appends to the job's buffer *and* publishes a `JobEvent::Output`.

Add to `crates/server/Cargo.toml`:

```toml
factorio-bot-scripting = { path = "../scripting" }
factorio-bot-scripting-lua = { path = "../scripting_lua", optional = true }

[features]
default = ["lua"]
lua = ["dep:factorio-bot-scripting-lua"]
```

`jobs.rs` itself needs only `factorio-bot-scripting` (for `OutputSink`/`Stream`), so it is not feature-gated; only the execute *handler* in Task 6 is.

- [ ] **Step 4: Run and watch them pass**

- [ ] **Step 5: Wire the registry into `AppState`**

`state.rs` gains `pub jobs: Arc<JobRegistry>`, built in `AppState::new` with a history limit of `50`. Tests that construct `AppState` directly get the same.

- [ ] **Step 6: Prove the slot guard is load-bearing (mutation)**

Make `try_start` always take the slot (delete the `if let Some(running) = inner.running { return Err(running) }` branch). `a_second_start_is_refused_while_one_is_running` must fail, and so must `a_second_execution_is_refused_with_the_running_job_id` once Task 6 lands.

This mutation *is* valid under the rule in Global Constraints: that branch is the sole gate on "a second start while one is running", and no other check rejects that input. Record the full failure set — every failing test name, not a count. Restore.

- [ ] **Step 7: Commit**

```bash
git add crates/server/src/jobs.rs crates/server/src/lib.rs crates/server/src/state.rs crates/server/Cargo.toml
git commit -m "feat(server): add a single-slot job registry with broadcast output"
```

---

## Task 6: Execute and job-inspection endpoints

> **GATE:** do not start this task until Task 1b has landed. This is the task that makes the Lua interpreter reachable over an unauthenticated network API; shipping it while `rcon.*` still panics on a malformed table means one line of Lua kills the server for everyone.

**Files:**
- Create: `crates/server/src/manage/execute.rs`
- Modify: `crates/server/src/manage/mod.rs`
- Create: `crates/server/tests/manage_execute.rs`
- Modify: `crates/server/tests/openapi.rs`

**Interfaces:**
- Consumes: `JobRegistry` (Task 5), `factorio_bot_scripting_lua::{run_script_file, run_script}` (Task 3), `factorio_bot_core::scripts::ensure_scripts_dir`.

### Routes

| Method | Path | Success | Errors |
|---|---|---|---|
| POST | `/api/v1/scripts/execute` | `202` `{ "job_id": "3" }` | `400` bad body / unknown extension; `404` script not found; `409` `{ "code": 409, "error": "…", "running_job_id": "2" }`; `503` no running Factorio instance |
| GET | `/api/v1/jobs` | `200` `[Job]`, newest first | — |
| GET | `/api/v1/jobs/{id}` | `200` `Job` | `404` |
| GET | `/api/v1/jobs/{id}/events` | `200` `text/event-stream` | `404` (Task 7) |

Request body (exactly one of `path` or `code` must be present):

```rust
#[derive(Deserialize, ToSchema)]
pub struct ExecuteRequest {
    /// Script to run, relative to the scripts root. Mutually exclusive with `code`.
    pub path: Option<String>,
    /// Inline code to run. Mutually exclusive with `path`.
    pub code: Option<String>,
    /// Defaults to `"lua"` when `code` is given; ignored when `path` is.
    pub language: Option<String>,
    /// Defaults to the configured `factorio.client_count`.
    pub bot_count: Option<u8>,
}
```

- [ ] **Step 1: Write the failing HTTP tests**

`crates/server/tests/manage_execute.rs`. Follow the existing helper shape in `tests/manage_scripts.rs` (temp workspace + `AppState` built directly + `tower::ServiceExt::oneshot`).

```rust
#[tokio::test]
async fn executing_without_a_running_instance_is_service_unavailable() {
    // The honest status for "the server is fine, the game is not running".
    let (_dir, state) = test_state().await;
    let response = post_execute(&state, serde_json::json!({ "path": "/hello.lua" })).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn executing_with_neither_path_nor_code_is_a_bad_request() {
    let (_dir, state) = test_state().await;
    let response = post_execute(&state, serde_json::json!({})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn executing_with_both_path_and_code_is_a_bad_request() {
    let (_dir, state) = test_state().await;
    let response =
        post_execute(&state, serde_json::json!({ "path": "/a.lua", "code": "print(1)" })).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_second_execution_is_refused_with_the_running_job_id() {
    // Occupy the slot directly rather than racing two real scripts: the
    // contract under test is the 409 body, not the scheduler's timing.
    let (_dir, state) = test_state().await;
    let holder = state.jobs.try_start(Some("busy.lua".into())).expect("slot taken");
    let response = post_execute(&state, serde_json::json!({ "path": "/hello.lua" })).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: serde_json::Value = body_json(response).await;
    assert_eq!(body["running_job_id"], serde_json::json!(holder.id().to_string()));
}

#[tokio::test]
async fn listing_jobs_returns_them_newest_first() {
    let (_dir, state) = test_state().await;
    for name in ["a.lua", "b.lua"] {
        let handle = state.jobs.try_start(Some(name.into())).expect("start");
        handle.finish(Ok((String::new(), String::new())));
    }
    let response = get(&state, "/api/v1/jobs").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = body_json(response).await;
    assert_eq!(body[0]["script"], serde_json::json!("b.lua"));
    assert_eq!(body[1]["script"], serde_json::json!("a.lua"));
}

#[tokio::test]
async fn an_unknown_job_is_not_found() {
    let (_dir, state) = test_state().await;
    let response = get(&state, "/api/v1/jobs/9999").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_finished_job_reports_its_output() {
    let (_dir, state) = test_state().await;
    let handle = state.jobs.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    handle.finish(Ok(("printed".into(), String::new())));
    let response = get(&state, &format!("/api/v1/jobs/{id}")).await;
    let body: serde_json::Value = body_json(response).await;
    assert_eq!(body["status"], serde_json::json!("succeeded"));
    assert_eq!(body["stdout"], serde_json::json!("printed"));
}
```

- [ ] **Step 2: Run and watch them fail**

- [ ] **Step 3: Implement the handlers**

`post_execute` in outline — validate, resolve, take the slot, then `tokio::spawn` the run and return `202` immediately:

```rust
#[utoipa::path(
    post,
    path = "/api/v1/scripts/execute",
    request_body = ExecuteRequest,
    responses(
        (status = 202, description = "Execution started", body = ExecuteAccepted),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 404, description = "Script not found", body = ErrorResponse),
        (status = 409, description = "A script is already running", body = ErrorResponse),
        (status = 503, description = "No running Factorio instance", body = ErrorResponse),
    ),
    tag = "scripts",
)]
pub async fn post_execute(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ExecuteRequest>,
) -> Result<(StatusCode, Json<ExecuteAccepted>), ErrorResponse> { … }
```

Order matters and is part of the contract: validate the body first (400), then check for an instance (503), then resolve the script (404), and only then take the slot (409). Taking the slot before a check that can fail would leave it held by a request that never runs.

`ErrorResponse` gains an optional `running_job_id: Option<String>` field, `#[serde(skip_serializing_if = "Option::is_none")]` so every other error body is unchanged.

The spawned task must release the slot on *every* exit path, including a panic inside the script host. `JobHandle`'s `Drop` marks an unfinished job `Failed` and frees the slot — do not rely on the happy path calling `finish`.

- [ ] **Step 4: Register the routes and assert them**

`manage/mod.rs` gains four `.routes(routes!(…))` lines, and `tests/openapi.rs` gains the four paths to its route list.

`no_operation_publishes_a_path_parameter` needs care. It currently asserts that *nothing* publishes a path parameter, which held because plan 3 added no path-parameterised route. `GET /jobs/{id}` and `GET /jobs/{id}/events` are the first genuine ones. Do **not** relax the assertion to "path parameters are allowed" — that would retire the guard that stops finding I1 from returning. Instead give the test an explicit allow-list of the two operations that legitimately carry `{id}`, so any *third* path parameter still fails:

```rust
const OPERATIONS_WITH_A_PATH_PARAMETER: &[(&str, &str)] = &[
    ("get", "/api/v1/jobs/{id}"),
    ("get", "/api/v1/jobs/{id}/events"),
];
```

- [ ] **Step 5: Run the tests and watch them pass**

- [ ] **Step 6: Commit**

```bash
git add crates/server/src/manage crates/server/tests
git commit -m "feat(server): add script execution and job inspection endpoints"
```

---

## Task 7: SSE stream of a job's output

**Files:**
- Modify: `crates/server/src/manage/execute.rs`
- Modify: `crates/server/src/webserver.rs` (shutdown interaction)
- Modify: `crates/server/tests/manage_execute.rs`
- Modify: `crates/server/tests/shutdown.rs`

### Scope

The user's decision: **the stream carries script output only** — not the Factorio server process's stdout. Events:

| `event:` | `data:` |
|---|---|
| `output` | `{"stream":"stdout","text":"…"}` |
| `finished` | `{"status":"succeeded"}` |
| `lagged` | `{"skipped":12}` — emitted when a slow client misses messages, so a gap is visible rather than silent |

The stream ends after `finished`. A subscriber attaching to an already-finished job gets the buffered output followed by `finished`, then end-of-stream — never an open connection waiting for events that will never come.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn the_event_stream_replays_a_finished_job_and_ends() {
    let (_dir, state) = test_state().await;
    let handle = state.jobs.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    handle.line(Stream::Stdout, "hello");
    handle.finish(Ok(("hello".into(), String::new())));

    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    // The whole body is collectable, which is itself the assertion: a stream
    // that stayed open would hang here.
    let body = collect_body(response).await;
    assert!(body.contains("event: output"), "body was {body:?}");
    assert!(body.contains("hello"), "body was {body:?}");
    assert!(body.contains("event: finished"), "body was {body:?}");
}

#[tokio::test]
async fn the_event_stream_delivers_output_produced_after_subscribing() {
    let (_dir, state) = test_state().await;
    let handle = state.jobs.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;

    tokio::spawn(async move {
        handle.line(Stream::Stdout, "late");
        handle.finish(Ok(("late".into(), String::new())));
    });

    let body = tokio::time::timeout(Duration::from_secs(5), collect_body(response))
        .await
        .expect("the stream must end when the job finishes");
    assert!(body.contains("late"), "body was {body:?}");
}

#[tokio::test]
async fn the_event_stream_of_an_unknown_job_is_not_found() {
    let (_dir, state) = test_state().await;
    let response = get(&state, "/api/v1/jobs/9999/events").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

And in `tests/shutdown.rs`:

```rust
#[tokio::test]
async fn an_open_event_stream_does_not_hold_shutdown_past_the_grace_period() {
    // webserver.rs's own comment names this as the hazard: axum's graceful
    // drain waits for in-flight requests without bound, and an SSE stream for
    // a job that never finishes never ends on its own.
    // Asserts on elapsed time, not on a status: a hang is the failure mode.
}
```

Write that test's body in full using the existing `SHORT_GRACE_PERIOD` constant: start a server with a job running and a subscriber attached, fire the shutdown signal, and assert the `start_with_shutdown` future resolves within roughly the grace period rather than never.

- [ ] **Step 2: Run and watch them fail**

- [ ] **Step 3: Implement the SSE handler**

Use `axum::response::sse::{Event, KeepAlive, Sse}` over a `tokio_stream::wrappers::BroadcastStream`, prefixed with the job's buffered output so a late subscriber sees the whole run. Terminate the stream on `JobEvent::Finished`. Add `KeepAlive::default()` so proxies do not drop an idle connection.

`tokio-stream` (with the `sync` feature) and `futures-util` are new dependencies of `crates/server`.

- [ ] **Step 4: Bound the stream by shutdown**

The registry gets a process-wide `shutdown: watch::Receiver<bool>` (or a `CancellationToken`); every SSE stream selects on it and ends. Without this, axum's unbounded graceful drain waits for streams that never end, and only the grace-period timer saves the process — which is a fallback, not a design.

- [ ] **Step 5: Run and watch them pass**

- [ ] **Step 6: Prove the terminator is load-bearing (mutation)**

Remove the `JobEvent::Finished` stream terminator (let the stream continue after the terminal event).

Wrap the body collection in `the_event_stream_replays_a_finished_job_and_ends` in `tokio::time::timeout(Duration::from_secs(5), ...)` with an `.expect("the stream must end once the job has finished")` **before** running the mutation, so the mutated build fails on a named assertion in five seconds instead of hanging the suite. A hang is not a test result — CI reports it as a timeout with no attribution, and a developer running the suite locally cannot tell it from a deadlock elsewhere.

Confirm that test, and only that test, goes red. Record the full failure set. Restore.

- [ ] **Step 7: Commit**

```bash
git add crates/server
git commit -m "feat(server): stream job output over SSE, bounded by shutdown"
```

---

## Task 8: Documentation

**Files:**
- Modify: `docs/userguide/src/` (the scripting page)
- Modify: `CLAUDE.md`

- [ ] **Step 1: Document the sandbox in the Lua API docs**

The `__doc_entry_*` strings in `globals.rs` and `world.rs` are the source of the published Lua API docs. Update the four bounded bindings to say that paths are relative to the scripts directory and that leaving it is refused, and say that `io`, `os`, `package`, `require`, `dofile` and `loadfile` are not available.

**Verified rather than assumed** (this claim is why the task exists, so it was checked before an implementer acted on it): `write_lua_docs` at `crates/scripting_lua/src/lua_docs.rs:16` walks each table for keys prefixed `__doc_entry_` and writes `globals.lua`, `world.lua`, `plan.lua`, `goal.lua` and `rcon.lua`; `app/src-tauri/build.rs:39` invokes it into `docs/lua/src/`; and `docs/lua/src/.gitignore` ignores `*.lua`, so those five files are untracked build artifacts. Editing the strings really is the only place to change them.

**One exception, and it is a hand-maintained copy of a derived thing.** `docs/lua/src/types.lua` *is* tracked, is not generated from any `__doc_entry_*`, and documents `crates/core/src/types.rs`'s structs (`FactorioTile`, `Position`, …) by hand. It carries the same drift risk as any hand-written mirror: nothing regenerates it and nothing checks it. Do not try to fix that here — it is documentation-only and out of this plan's scope — but note it in the report so it does not stay invisible.

- [ ] **Step 2: Note the execution model in `CLAUDE.md`**

One short subsection under the existing architecture notes: one script at a time, 409 with the running job's id, output over SSE at `/api/v1/jobs/{id}/events`, scripts sandboxed to the scripts directory.

- [ ] **Step 3: Full check**

`just test`

- [ ] **Step 4: Commit**

```bash
git add docs CLAUDE.md crates/scripting_lua
git commit -m "docs: describe the script sandbox and the execution model"
```

---

## Self-Review Notes

**Spec coverage.** The spec's script-execution section asks for: run a script from the browser (Task 6), see its output live (Task 7), one at a time with a 409 naming the running job (Tasks 5-6), and no Tauri dependency in the path (Task 3 moves execution into a workspace crate). The SSE-scope decision — script output only, not Factorio process stdout — is stated in Task 7.

**Type consistency.** `run_lua`'s signature is introduced in Task 1 Step 12 and used unchanged in Tasks 2, 3 and 4. `OutputSink`/`Stream` are defined in Task 2 Step 3 and consumed in Tasks 3, 5, 6, 7. `JobId` serializes as a string in Task 5 and is asserted as a string in Task 6's 409 test.

**Known ordering constraint.** Task 1 is a hard gate for Tasks 5-7: the execute endpoint must not exist before the sandbox does.

**Deliberately not in this plan.** `extract_archive` blocking the executor: real, but only reachable from instance *start*, and there is no start endpoint yet — it belongs with whichever plan adds one. The `helpers.table_to_json` question from the plan-3 hand-off is closed: `crates/executor/src/rcon_actuator.rs:38` already uses `helpers.table_to_json` and asserts against the removed `game.table_to_json` at line 313; a repository-wide grep finds no other occurrence.
