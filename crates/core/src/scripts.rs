use miette::{miette, Diagnostic, IntoDiagnostic, Result};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Why [`resolve_script_path`] refused a client-supplied path.
///
/// The two cases carry different meaning to an HTTP caller — a missing script
/// is a 404, a path that tries to leave the scripts root is a 400 — so they are
/// distinguishable in the type rather than only in a formatted message.
// False positive from the thiserror/miette derives using struct fields in
// format strings, same as `crate::errors`.
#[allow(unused_assignments)]
#[derive(Error, Debug, Diagnostic)]
pub enum ScriptPathError {
    #[error("path not found: {requested}")]
    #[diagnostic(code(factorio::scripts::not_found), help("check the script path"))]
    NotFound { requested: String },

    #[error("path escapes the scripts directory: {requested}")]
    #[diagnostic(
        code(factorio::scripts::escapes_root),
        help("scripts must live under the workspace scripts directory")
    )]
    EscapesRoot { requested: String },
}

/// Locates the directory holding user Lua scripts.
///
/// Development checkouts keep them at the repository root; an installed copy
/// keeps them under the workspace. The development paths are resolved against
/// the current working directory, which is why a server started from an
/// arbitrary directory falls back to the workspace copy.
pub fn scripts_dir(workspace_path: &Path) -> Result<PathBuf> {
    for candidate in [PathBuf::from("./scripts"), PathBuf::from("../../scripts")] {
        if candidate.exists() {
            return std::fs::canonicalize(candidate).into_diagnostic();
        }
    }

    let workspace_scripts = workspace_path.join("scripts");
    if workspace_scripts.exists() {
        return std::fs::canonicalize(workspace_scripts).into_diagnostic();
    }

    #[cfg(not(debug_assertions))]
    {
        std::fs::create_dir_all(&workspace_scripts).into_diagnostic()?;
        crate::process::instance_setup::PLANS_CONTENT
            .extract(workspace_scripts.clone())
            .map_err(|err| miette!("failed to extract bundled scripts: {err:?}"))?;
        std::fs::canonicalize(workspace_scripts).into_diagnostic()
    }

    #[cfg(debug_assertions)]
    Err(miette!(
        "missing scripts/ directory: {}",
        workspace_scripts.display()
    ))
}

/// Creates `workspace_path/scripts` if it is not there yet, and returns its
/// canonical path.
///
/// This is the bootstrap half of [`scripts_dir`] without the CWD-relative
/// lookup. The HTTP server resolves the scripts root straight from
/// `workspace_path` (see `scripts_root` in `crates/server/src/manage/scripts.rs`)
/// precisely so a request can never be redirected to whatever `./scripts`
/// happens to be relative to the server process's working directory — but that
/// also means it never runs [`scripts_dir`]'s directory creation, so on a fresh
/// install every `/api/v1/scripts*` route answered "missing scripts directory"
/// with no way to create a first script from a browser. Callers that start a
/// long-running server call this once at startup instead.
///
/// In release builds a *newly created* directory is seeded with the bundled
/// scripts (`PLANS_CONTENT`), matching [`scripts_dir`]. An already-populated
/// directory is left alone -- editing `scripts/*.lua` in the repo has no
/// effect on it -- so this is safe to call on every start; when it is already
/// populated this also checks it for drift from the embedded snapshot (once
/// per process, since callers such as the HTTP script routes call this on
/// every request) and warns if any script is stale, naming
/// [`REFRESH_SCRIPTS_ENV`]
/// as the way to refresh it. Debug builds deliberately skip the extraction
/// and the check: `include_dir!` bundles this repository's own `scripts/`
/// directory, and a developer checkout already has them.
///
/// Takes [`crate::paths::ResolvedWorkspace`], not a bare `&Path`: this
/// function joins `workspace_path` straight onto `scripts` with no
/// CWD-relative fallback (unlike [`scripts_dir`]), so an unresolved --
/// possibly relative -- `workspace_path` reaching here would silently create
/// and populate `<process cwd>/<relative>/scripts`. Requiring the type that
/// only [`crate::paths::resolve_workspace`] can mint makes that unreachable
/// rather than merely undocumented.
pub fn ensure_scripts_dir(workspace_path: &crate::paths::ResolvedWorkspace) -> Result<PathBuf> {
    let workspace_path = workspace_path.as_path();
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
    } else {
        #[cfg(not(debug_assertions))]
        check_scripts_staleness_once(&workspace_scripts)?;
    }
    std::fs::canonicalize(&workspace_scripts).into_diagnostic()
}

/// Set to any value to overwrite stale files under `<workspace>/scripts` with
/// the snapshot embedded in this binary. Not read automatically, for the same
/// reason as `FACTORIO_BOT_REFRESH_MODS`: a script under the workspace may
/// have been edited on purpose to unblock a run.
#[cfg(not(debug_assertions))]
pub const REFRESH_SCRIPTS_ENV: &str = "FACTORIO_BOT_REFRESH_SCRIPTS";

/// `ensure_scripts_dir` runs on every server request that touches scripts, so
/// checking staleness unconditionally would print
/// the same warning over and over for the life of the process. This runs the
/// check (and an env-gated refresh) exactly once per process instead.
#[cfg(not(debug_assertions))]
fn check_scripts_staleness_once(workspace_scripts: &Path) -> Result<()> {
    use std::sync::OnceLock;
    static CHECKED: OnceLock<()> = OnceLock::new();
    if CHECKED.set(()).is_err() {
        return Ok(());
    }
    if crate::process::asset_sync::refresh_if_requested(
        &crate::process::instance_setup::PLANS_CONTENT,
        workspace_scripts,
        REFRESH_SCRIPTS_ENV,
    )
    .into_diagnostic()?
    {
        info!(
            "Refreshed <bright-blue>{:?}</> from the embedded snapshot ({}=1 was set)",
            workspace_scripts, REFRESH_SCRIPTS_ENV
        );
    } else {
        crate::process::asset_sync::warn_if_stale(
            &crate::process::instance_setup::PLANS_CONTENT,
            workspace_scripts,
            "scripts",
            REFRESH_SCRIPTS_ENV,
        );
    }
    Ok(())
}

/// Resolves a client-supplied script path against the scripts root.
///
/// The path may or may not carry a leading `/`. The result is canonicalised and
/// verified to live under `root`, so `..` segments (and symlinks) cannot escape
/// it — this is a network-reachable boundary, not a local convenience. Unlike
/// [`resolve_write_path`] this canonicalizes the whole target, so a symlink at
/// the final component is resolved and bounds-checked like any other.
///
/// `root` must already be canonical — every caller obtains it from
/// [`scripts_dir`] or [`ensure_scripts_dir`], both of which canonicalize. A
/// non-canonical `root` fails closed: the canonical result will not
/// `starts_with` it, so everything is denied rather than let through.
///
/// This function only answers "is this path inside the scripts root, and
/// where does it land" — it does not decide whether the result must be a file
/// or a directory; callers check that themselves after resolving, same as the
/// original handlers did. In particular, `""` and `"/"` both mean "the root"
/// once the leading slash is stripped, so both resolve to `root` itself
/// (rather than one of them erroring while the other doesn't) — this is what
/// lets a directory-listing endpoint pass either straight through to list the
/// scripts root.
pub fn resolve_script_path(
    root: &Path,
    requested: &str,
) -> std::result::Result<PathBuf, ScriptPathError> {
    let relative = requested.trim_start_matches('/');

    let joined = root.join(relative);
    // canonicalize resolves `..` and symlinks, and fails if the target is absent
    let canonical = std::fs::canonicalize(&joined).map_err(|_| ScriptPathError::NotFound {
        requested: requested.to_owned(),
    })?;

    if !canonical.starts_with(root) {
        return Err(ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        });
    }
    Ok(canonical)
}

/// Resolves a path that is allowed not to exist yet, bounding it to `root`.
///
/// [`resolve_script_path`] canonicalizes the *target*, so it can only resolve
/// paths that already exist — right for reads, useless for `file_write` or
/// `world.draw`, whose whole point is creating something new. This resolves
/// the deepest existing ancestor instead and re-attaches the remainder.
///
/// That gets the parent chain checked the way `canonicalize` would — `..` and
/// symlinks above the destination are resolved by the OS, not by us — but it
/// deliberately does *not* resolve the final component, because the whole
/// point is that it may not exist. A name that does exist and is a symlink is
/// therefore rejected outright: following it would write through to wherever
/// it points, which is how a script planted `evil.txt -> /outside/file` and
/// then overwrote the target through a path that passed every bounds check.
///
/// `root` must already be canonical — every caller obtains it from
/// [`scripts_dir`] or [`ensure_scripts_dir`], both of which canonicalize. A
/// non-canonical `root` fails closed: no canonicalized parent will
/// `starts_with` it, so everything is denied rather than let through.
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
        return Err(ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        });
    }

    let joined = root.join(requested_path);
    let parent = joined
        .parent()
        .ok_or_else(|| ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        })?;
    let file_name = joined
        .file_name()
        .ok_or_else(|| ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        })?;

    // The parent must exist; canonicalizing it is what resolves `..` segments
    // and symlinks before the bounds check sees the path.
    let parent = std::fs::canonicalize(parent).map_err(|_| ScriptPathError::NotFound {
        requested: requested.to_owned(),
    })?;
    if !parent.starts_with(root) {
        return Err(ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        });
    }

    let target = parent.join(file_name);
    // Belt and braces: shadowed by the `parent` check above, which rejects
    // every input that could reach here with `target` outside the root. It is
    // kept because `resolve_new_script_path` in the server learned the hard
    // way (plan 3, finding I4) that a component can replace a path on Windows,
    // and a bounds check that is cheap and unconditional outlives the
    // reasoning that made it redundant. Being shadowed is why no test can kill
    // it on its own — that is a property of the code, not a gap in the tests.
    if !target.starts_with(root) {
        return Err(ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        });
    }

    // The parent chain was canonicalized; the leaf was not, because it is
    // allowed not to exist. If it does exist and is a symlink, writing to it
    // writes through to wherever it points — which needs no `..` and no
    // symlinked directory, so nothing above catches it. `symlink_metadata`
    // does not follow the link; an absent target simply has no metadata and
    // is fine.
    if std::fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ScriptPathError::EscapesRoot {
            requested: requested.to_owned(),
        });
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("scripts");
        fs::create_dir_all(root.join("sub")).expect("mkdir");
        fs::write(root.join("a.lua"), "-- a").expect("write");
        fs::write(root.join("sub").join("b.lua"), "-- b").expect("write");
        // A real file that lives *outside* the scripts root, as a sibling of
        // `scripts/`. Escape attempts below resolve to this file, so
        // canonicalize succeeds and the traversal guard is what rejects them —
        // not a coincidental "file not found".
        fs::write(dir.path().join("outside.lua"), "-- outside").expect("write");
        let canonical = fs::canonicalize(&root).expect("canonicalize");
        (dir, canonical)
    }

    /// Test workspace roots here are always absolute tempdir paths, so
    /// routing them through `resolve_workspace` -- the same thing every real
    /// caller does -- always succeeds; there is no need for a test-only
    /// constructor.
    fn resolved(workspace_path: &Path) -> crate::paths::ResolvedWorkspace {
        crate::paths::resolve_workspace(&workspace_path.to_string_lossy())
            .expect("test workspace paths are absolute")
    }

    /// A fresh install has a `workspace/` directory but no `workspace/scripts`
    /// — `Context::new` creates only the former. Without this bootstrap every
    /// `/api/v1/scripts*` route answered "missing scripts directory" and there
    /// was no way to create a first script from a browser.
    #[test]
    fn ensure_scripts_dir_creates_a_missing_scripts_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("mkdir");
        assert!(
            !workspace.join("scripts").exists(),
            "fixture bug: scripts/ already exists"
        );

        let scripts = ensure_scripts_dir(&resolved(&workspace)).expect("bootstraps");

        assert!(scripts.is_dir(), "{scripts:?} is not a directory");
        // The server canonicalises the root and requires it to exist; proving
        // the returned path is canonical and resolvable is what "usable" means
        // for `GET /api/v1/scripts?path=/`.
        assert_eq!(
            scripts,
            fs::canonicalize(workspace.join("scripts")).expect("canonicalize")
        );
        assert_eq!(
            resolve_script_path(&scripts, "/").expect("root resolves"),
            scripts
        );
    }

    /// The regression task 3c closes: `scripting.rs` and `run_script.rs` used
    /// to hand `ensure_scripts_dir` the raw, unresolved `workspace_path`
    /// string (as a bare `&Path`), so a relative configured value quietly
    /// created `<process cwd>/<relative>/scripts` instead of erroring.
    ///
    /// `ensure_scripts_dir` now takes `&paths::ResolvedWorkspace`, which only
    /// `resolve_workspace` can mint, so that call shape can no longer be
    /// written at all -- confirmed here by going through the same two steps
    /// every real caller now must: `resolve_workspace` first, which refuses a
    /// relative path outright, so `ensure_scripts_dir` (and its directory
    /// creation) is never reached.
    ///
    /// Asserts the directory itself is absent, not merely that an `Err` came
    /// back: an `Err` returned after the directory was already created would
    /// still leave the mess behind. Before this fix (`ensure_scripts_dir`
    /// taking a bare `&Path`), the equivalent call sequence created the
    /// directory and this assertion failed.
    #[test]
    fn a_relative_workspace_path_never_reaches_ensure_scripts_dir() {
        let relative = "relative-workspace-hazard-repro-3c";
        let hazard_dir = std::env::current_dir().expect("cwd").join(relative);
        let _ = fs::remove_dir_all(&hazard_dir);

        let result = crate::paths::resolve_workspace(relative).map(|ws| ensure_scripts_dir(&ws));

        assert!(
            result.is_err(),
            "a relative workspace_path must be refused before it reaches ensure_scripts_dir"
        );
        let still_absent = !hazard_dir.join("scripts").exists();
        let _ = fs::remove_dir_all(&hazard_dir);
        assert!(
            still_absent,
            "ensure_scripts_dir must not create a scripts dir under an unresolved relative workspace"
        );
    }

    /// Called on every server start, so it must be idempotent and must not
    /// disturb scripts already on disk.
    #[test]
    fn ensure_scripts_dir_leaves_an_existing_directory_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(workspace.join("scripts")).expect("mkdir");
        fs::write(workspace.join("scripts").join("mine.lua"), "-- mine").expect("write");

        let scripts = ensure_scripts_dir(&resolved(&workspace)).expect("bootstraps");
        let scripts_again = ensure_scripts_dir(&resolved(&workspace)).expect("bootstraps twice");

        assert_eq!(scripts, scripts_again);
        assert_eq!(
            fs::read_to_string(scripts.join("mine.lua")).expect("read"),
            "-- mine"
        );
    }

    #[test]
    fn resolves_a_path_with_a_leading_slash() {
        let (_dir, root) = root();
        let resolved = resolve_script_path(&root, "/a.lua").expect("resolves");
        assert_eq!(resolved, root.join("a.lua"));
    }

    #[test]
    fn resolves_a_nested_path() {
        let (_dir, root) = root();
        let resolved = resolve_script_path(&root, "/sub/b.lua").expect("resolves");
        assert_eq!(resolved, root.join("sub").join("b.lua"));
    }

    #[test]
    fn resolves_a_path_without_a_leading_slash() {
        let (_dir, root) = root();
        let resolved = resolve_script_path(&root, "a.lua").expect("resolves");
        assert_eq!(resolved, root.join("a.lua"));
    }

    /// The old caller did `&path[1..]`, which panics on an empty string. Both
    /// `""` and `"/"` reduce to the same string once the leading slash is
    /// stripped, so both deliberately resolve to `root` itself — this is the
    /// guarantee a directory-listing endpoint relies on to list the scripts
    /// root (`GET /api/v1/scripts?path=/`).
    #[test]
    fn resolves_an_empty_or_slash_path_to_the_root() {
        let (_dir, root) = root();
        assert_eq!(resolve_script_path(&root, "").expect("resolves"), root);
        assert_eq!(resolve_script_path(&root, "/").expect("resolves"), root);
    }

    /// The old caller did `&path[1..]`, which panics when the first character
    /// is multi-byte. This only proves absence of a panic on a missing file —
    /// see `resolves_a_multibyte_path` for proof the byte-safe case actually
    /// resolves.
    #[test]
    fn rejects_a_multibyte_path_without_panicking() {
        let (_dir, root) = root();
        let result = resolve_script_path(&root, "ä.lua");
        assert!(result.is_err(), "expected a miss, got {result:?}");
    }

    /// Positive counterpart to `rejects_a_multibyte_path_without_panicking`:
    /// a multi-byte-first-character filename that actually exists resolves
    /// correctly, proving `trim_start_matches` (char-aware) is used instead of
    /// the old byte slice `&path[1..]`.
    #[test]
    fn resolves_a_multibyte_path() {
        let (_dir, root) = root();
        fs::write(root.join("ä.lua"), "-- a with umlaut").expect("write");
        let resolved = resolve_script_path(&root, "/ä.lua").expect("resolves");
        assert_eq!(resolved, root.join("ä.lua"));
    }

    #[test]
    fn refuses_to_escape_the_root() {
        let (dir, root) = root();
        // Sanity check the fixture: the escape attempts below must land on a
        // real file that is genuinely outside root, otherwise this test would
        // pass merely because canonicalize fails with "not found".
        let outside = fs::canonicalize(dir.path().join("outside.lua")).expect("canonicalize");
        assert!(
            !outside.starts_with(&root),
            "fixture bug: outside.lua ended up under root"
        );
        for attempt in ["../outside.lua", "/../outside.lua", "sub/../../outside.lua"] {
            assert!(
                resolve_script_path(&root, attempt).is_err(),
                "{attempt} should not resolve"
            );
        }
    }

    /// A symlink inside the root pointing at a file outside it must not let a
    /// request escape: `canonicalize` follows the symlink, and the resulting
    /// path fails the `starts_with(root)` check.
    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_that_escapes_the_root() {
        use std::os::unix::fs::symlink;

        let (dir, root) = root();
        let outside = dir.path().join("outside.lua");
        let link = root.join("escape.lua");
        symlink(&outside, &link).expect("symlink");

        assert!(
            resolve_script_path(&root, "/escape.lua").is_err(),
            "a symlink escaping the root should not resolve"
        );
    }

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
        assert!(
            matches!(err, ScriptPathError::EscapesRoot { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn a_write_path_may_not_climb_out_with_dotdot() {
        let (_dir, root) = root();
        let err = resolve_write_path(&root, "../outside.txt").expect_err("refused");
        assert!(
            matches!(err, ScriptPathError::EscapesRoot { .. }),
            "got {err:?}"
        );
    }

    /// Renamed from `a_write_path_may_not_climb_out_and_back_in`, which said
    /// "may not" about a path the body asserts is *accepted*.
    #[test]
    fn a_write_path_that_climbs_out_and_back_in_resolves_to_where_it_lands() {
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
        assert!(
            matches!(err, ScriptPathError::NotFound { .. }),
            "got {err:?}"
        );
    }

    /// The parent chain is canonicalized but the destination name is not, so
    /// a symlink planted at the name itself needs no `..` and no symlinked
    /// directory: every bounds check passes and the write goes through to
    /// wherever the link points.
    #[cfg(unix)]
    #[test]
    fn a_write_path_may_not_write_through_a_symlinked_destination() {
        use std::os::unix::fs::symlink;

        let (dir, root) = root();
        let outside = dir.path().join("outside.lua");
        symlink(&outside, root.join("evil.txt")).expect("symlink");

        let err = resolve_write_path(&root, "evil.txt").expect_err("refused");
        assert!(
            matches!(err, ScriptPathError::EscapesRoot { .. }),
            "got {err:?}"
        );
    }

    /// The counterpart that stops the symlink check from degenerating into
    /// "refuse anything that already exists": overwriting a real file in the
    /// root is what `file_write` is for.
    #[test]
    fn a_write_path_may_overwrite_an_existing_regular_file() {
        let (_dir, root) = root();
        let resolved = resolve_write_path(&root, "a.lua").expect("accepted");
        assert_eq!(resolved, root.join("a.lua"));
    }

    /// Pins the `parent` bounds check specifically.
    ///
    /// This is the one input where the two `starts_with` guards disagree:
    /// `target` is `parent.join(file_name)` with a single-component
    /// `file_name`, so `target` can only be inside the root while `parent` is
    /// outside it when the two are the *same* path -- climbing out of the root
    /// and naming the root back. Without the `parent` check this resolves to
    /// the scripts root itself, which is a directory a script has no business
    /// being handed as a write destination.
    #[test]
    fn a_write_path_may_not_name_the_scripts_root_by_climbing_out_and_back() {
        let (_dir, root) = root();
        let name = root.file_name().expect("root has a name").to_string_lossy();
        let err = resolve_write_path(&root, &format!("../{name}")).expect_err("refused");
        assert!(
            matches!(err, ScriptPathError::EscapesRoot { .. }),
            "got {err:?}"
        );
    }

    /// Pins the `is_absolute` check specifically.
    ///
    /// `a_write_path_may_not_be_absolute` uses a path outside the root, so the
    /// bounds check refuses it even with the absolute check gone. An absolute
    /// path that happens to land *inside* the root is what isolates the rule
    /// "the argument is always root-relative" from the rule "the destination
    /// is inside the root".
    #[test]
    fn a_write_path_may_not_be_absolute_even_when_it_points_inside_the_root() {
        let (_dir, root) = root();
        let inside = root.join("new.png");
        let err = resolve_write_path(&root, &inside.to_string_lossy()).expect_err("refused");
        assert!(
            matches!(err, ScriptPathError::EscapesRoot { .. }),
            "got {err:?}"
        );
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
            assert!(
                matches!(err, ScriptPathError::EscapesRoot { .. }),
                "got {err:?}"
            );
        }
    }

    /// A substring check on ".." rejects this legitimate name; a canonicalising
    /// check accepts it.
    #[test]
    fn accepts_a_filename_containing_two_dots() {
        let (_dir, root) = root();
        fs::write(root.join("my..script.lua"), "-- ok").expect("write");
        let resolved = resolve_script_path(&root, "/my..script.lua").expect("resolves");
        assert_eq!(resolved, root.join("my..script.lua"));
    }
}
