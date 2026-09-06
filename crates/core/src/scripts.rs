use miette::{Diagnostic, IntoDiagnostic, Result, miette};
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

/// The repo's `scripts/` directory as a compile-time path -- the seed a debug
/// build copies into a new workspace, and the reference its staleness check
/// compares against.
///
/// Resolved from `CARGO_MANIFEST_DIR`, never from the process's working
/// directory, for the same reason `repo_mods_path!` in `instance_setup.rs` is:
/// a script name must mean the same file whatever directory the binary was
/// started from. The predecessor of this constant (`scripts_dir`, deleted)
/// probed `./scripts` and `../../scripts` relative to the CWD *ahead of* the
/// workspace, so from the repo root every workspace ran the checkout's
/// scripts and from anywhere else a new one had none. Nothing called it any
/// more by the time it was deleted, but its CWD probe had survived in the
/// staleness check.
///
/// A compile-time fact: a debug binary carried away from its source tree finds
/// nothing here, which `bootstrap_scripts_dir` reports rather than fails on.
#[cfg(debug_assertions)]
const REPO_SCRIPTS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts");

/// Makes `<workspace>/scripts` exist and hold the shipped scripts, logs one
/// line naming the directory and how it came to be, and returns its canonical
/// path.
///
/// **This is the only resolution of a script name there is.** Every caller --
/// the CLI's `lua` subcommand, the REPL, `serve`, and the HTTP script routes
/// -- resolves `<workspace>/scripts` and nothing else, so a request can never
/// be redirected to whatever `./scripts` happens to be relative to the
/// process's working directory. That used to be true of the server only; the
/// CLI went through a CWD-probing lookup until it was made to share this one.
///
/// What "hold the shipped scripts" means depends on the build, mirroring how
/// `instance_setup::resolve_workspace_mods` treats `mods/`:
///
/// - **debug**: a missing *or empty* directory is seeded by **copying** the
///   checkout's `scripts/` ([`REPO_SCRIPTS_PATH`]). A copy, not a symlink like
///   `BotBridge` gets, because scripts write: `file_write`, `world.draw` and
///   `world.dump` all land under the scripts root, and through a symlink an
///   864 MB `map.json` would land in the checkout. The price of a copy is that
///   an edit in the checkout does not run until copied over, which is exactly
///   what the staleness warning below is for. An empty directory counts as
///   missing because every debug build before this one created it empty, so
///   that is the state a workspace made by one is in.
/// - **release**: a missing or empty directory is extracted from the snapshot
///   `include_dir!` baked into the binary (`SCRIPTS_CONTENT`); a release
///   binary has no checkout to copy from.
///
/// An already-populated directory is **left alone** in both builds: a script
/// under the workspace may have been edited on purpose, and this runs on every
/// server request that touches scripts. It is checked for drift instead, once
/// per process -- against the embedded snapshot in a release build (naming
/// [`REFRESH_SCRIPTS_ENV`] as the way to refresh it) and against the checkout
/// on disk in a debug build. That check exists because `run-1788449752-46541`
/// spent an hour looking for an enclosure report that could not appear:
/// `workspace/scripts/factory_stage2.lua` predated the `record.enclosures()`
/// call by five hours, and the run said nothing about it.
///
/// The `Using scripts directory` line is printed once per process, through
/// `paris` (stdout) and deliberately **not** gated on any `silent` flag: it
/// is the counterpart of the `Using mods directory` line, which printed on no
/// run at all while it was so gated, and it is the authoritative answer to
/// "which file did my script name resolve to".
///
/// Takes [`crate::paths::ResolvedWorkspace`], not a bare `&Path`: this
/// function joins `workspace_path` straight onto `scripts`, so an unresolved
/// -- possibly relative -- `workspace_path` reaching here would silently
/// create and populate `<process cwd>/<relative>/scripts`. Requiring the type
/// that only [`crate::paths::resolve_workspace`] can mint makes that
/// unreachable rather than merely undocumented.
pub fn ensure_scripts_dir(workspace_path: &crate::paths::ResolvedWorkspace) -> Result<PathBuf> {
    use std::sync::OnceLock;
    static LOGGED: OnceLock<()> = OnceLock::new();
    let (scripts, source) = bootstrap_scripts_dir(workspace_path)?;
    if LOGGED.set(()).is_ok() {
        info!(
            "Using scripts directory <bright-blue>{:?}</> ({})",
            &scripts, source
        );
    }
    Ok(scripts)
}

/// [`ensure_scripts_dir`] without the log line: the canonical scripts root
/// and the sentence explaining how it came to hold what it holds. Separate so
/// the sentence is testable, since the line itself goes to stdout once.
pub(crate) fn bootstrap_scripts_dir(
    workspace_path: &crate::paths::ResolvedWorkspace,
) -> Result<(PathBuf, String)> {
    let workspace_path = workspace_path.as_path();
    let workspace_scripts = workspace_path.join("scripts");
    let source = if is_missing_or_empty(&workspace_scripts) {
        seed_scripts_dir(workspace_path, &workspace_scripts)?
    } else {
        check_scripts_staleness_once(&workspace_scripts)?;
        pre_existing_source()
    };
    let scripts = std::fs::canonicalize(&workspace_scripts).into_diagnostic()?;
    Ok((scripts, source))
}

/// True when `dir` is absent, or is a directory with nothing in it. Not a
/// directory at all (a stray file at that name) reads as missing too, and the
/// seeding step then fails on it loudly rather than resolving scripts against
/// a file.
fn is_missing_or_empty(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true,
    }
}

/// Replaces the missing-or-empty `workspace_scripts` with `staging`, which
/// the caller has just finished populating. Staging under a sibling name and
/// renaming in one step means a crash or a full disk mid-copy leaves no
/// half-populated `scripts/` that the next start would mistake for a finished
/// one. An existing empty directory is removed first, since `rename` onto a
/// directory is not portable.
fn install_staged(staging: &Path, workspace_scripts: &Path) -> Result<()> {
    if workspace_scripts.is_dir() {
        std::fs::remove_dir(workspace_scripts).into_diagnostic()?;
    }
    std::fs::rename(staging, workspace_scripts).into_diagnostic()
}

/// Debug seeding: copy the checkout's `scripts/` in, or create the directory
/// empty and say why when this binary's checkout is gone.
#[cfg(debug_assertions)]
fn seed_scripts_dir(workspace_path: &Path, workspace_scripts: &Path) -> Result<String> {
    let Some(checkout) = repo_scripts_checkout() else {
        std::fs::create_dir_all(workspace_scripts).into_diagnostic()?;
        return Ok(format!(
            "debug build; created empty: this binary was compiled against {REPO_SCRIPTS_PATH:?}, \
             and there is no directory there now, so there was nothing to seed it from"
        ));
    };
    let staging = workspace_path.join(".scripts-partial");
    let _ = std::fs::remove_dir_all(&staging);
    crate::process::instance_setup::copy_dir_recursive(&checkout, &staging)
        .map_err(|err| miette!("failed to copy {checkout:?} into {staging:?}: {err}"))?;
    install_staged(&staging, workspace_scripts)?;
    Ok(format!(
        "debug build; seeded by copying {checkout:?} -- a copy, so a script's file_write and \
         world.dump land here and not in the checkout, and an edit there does NOT run until \
         copied over"
    ))
}

/// Release seeding: extract the snapshot embedded in this binary.
#[cfg(not(debug_assertions))]
fn seed_scripts_dir(workspace_path: &Path, workspace_scripts: &Path) -> Result<String> {
    let staging = workspace_path.join(".scripts-partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).into_diagnostic()?;
    crate::process::instance_setup::SCRIPTS_CONTENT
        .extract(staging.clone())
        .map_err(|err| miette!("failed to extract bundled scripts: {err:?}"))?;
    install_staged(&staging, workspace_scripts)?;
    Ok(String::from(
        "release build; extracted from the compile-time snapshot embedded in this binary; \
         edits to scripts/ need a rebuild",
    ))
}

#[cfg(debug_assertions)]
fn pre_existing_source() -> String {
    String::from(
        "debug build; pre-existing workspace copy, left alone -- an edit in the checkout's \
         scripts/ does NOT run until copied over; see the staleness warning above, if any",
    )
}

#[cfg(not(debug_assertions))]
fn pre_existing_source() -> String {
    format!(
        "release build; pre-existing workspace copy, left alone -- editing scripts/ does NOT \
         update it; see the staleness warning above, or set {REFRESH_SCRIPTS_ENV}=1 to refresh it"
    )
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
        &crate::process::instance_setup::SCRIPTS_CONTENT,
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
            &crate::process::instance_setup::SCRIPTS_CONTENT,
            workspace_scripts,
            "scripts",
            REFRESH_SCRIPTS_ENV,
        );
    }
    Ok(())
}

/// The debug-build counterpart of the release staleness check above, and the
/// reason it exists at all.
///
/// A debug build embeds nothing, so there is no snapshot to compare against --
/// which is why this check used to be compiled out entirely, on the reasoning
/// that a developer checkout "already has" the scripts. It does, and that is
/// beside the point: **the CLI resolves a script by bare name against
/// `<workspace>/scripts`, never against the repo.** Unlike `mods/`, which a
/// debug build symlinks to the checkout, `workspace/scripts/` is a separate
/// copy with no fallback and no refresh path, ordinarily left behind by an
/// earlier release build. Editing `scripts/foo.lua` in the repo and running
/// `factorio-bot lua foo.lua` runs the *other* file.
///
/// `run-1788449752-46541` is what that costs. `scripts/factory_stage2.lua` had
/// gained a `record.enclosures()` call five hours before the run; the workspace
/// copy had not, so the enclosure detector -- landed, wired and unit-tested the
/// same afternoon -- could not produce a single line, and the run was read as
/// evidence that the detector did not work.
///
/// So the comparison here is against the checkout on disk rather than an
/// embed, which is both cheaper (no `include_dir!` in a debug build, and so no
/// core rebuild every time a `.lua` changes) and more truthful: in a debug
/// build the checkout *is* the reference, exactly as it is for `mods/`.
/// Silent when no checkout can be found -- an installed debug binary has
/// nothing to compare against, and inventing a complaint would be noise.
#[cfg(debug_assertions)]
fn check_scripts_staleness_once(workspace_scripts: &Path) -> Result<()> {
    use std::sync::OnceLock;
    static CHECKED: OnceLock<()> = OnceLock::new();
    if CHECKED.set(()).is_err() {
        return Ok(());
    }
    let Some(checkout) = repo_scripts_checkout() else {
        return Ok(());
    };
    // A canonical `workspace/scripts` that *is* the checkout (someone pointed
    // the workspace at the repo) can never be stale against itself.
    if checkout.as_path() == workspace_scripts {
        return Ok(());
    }
    let stale = stale_against_checkout(&checkout, workspace_scripts);
    if stale.is_empty() {
        return Ok(());
    }
    warn!(
        "<bright-blue>scripts</> workspace copy at <bright-blue>{:?}</> is STALE: {} file(s) differ from <bright-blue>{:?}</>: {:?}. A script is resolved by bare name against the WORKSPACE copy, so edits in the checkout will not run until you copy them over.",
        workspace_scripts,
        stale.len(),
        checkout,
        stale
    );
    Ok(())
}

/// The repository's own `scripts/` directory -- [`REPO_SCRIPTS_PATH`],
/// canonicalized -- or `None` when this binary's checkout is no longer there.
/// Canonical so the log line does not carry `../..`, and so the comparison
/// against a canonical `workspace/scripts` in the staleness check holds.
#[cfg(debug_assertions)]
fn repo_scripts_checkout() -> Option<PathBuf> {
    std::fs::canonicalize(REPO_SCRIPTS_PATH)
        .ok()
        .filter(|path| path.is_dir())
}

/// File names present in `checkout` whose copy under `workspace_scripts` is
/// missing or differs, sorted.
///
/// Top-level files only, which is the whole of `scripts/` today, and extra
/// files the workspace has grown on its own are not reported -- both matching
/// [`crate::process::asset_sync::stale_paths`], whose question this is asking
/// with a directory in place of an embed.
#[cfg(debug_assertions)]
fn stale_against_checkout(checkout: &Path, workspace_scripts: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(checkout) else {
        return Vec::new();
    };
    let mut stale: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter(|entry| {
            let reference = std::fs::read(entry.path());
            let copied = std::fs::read(workspace_scripts.join(entry.file_name()));
            !matches!((reference, copied), (Ok(a), Ok(b)) if a == b)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    stale.sort();
    stale
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
/// [`ensure_scripts_dir`], which canonicalizes. A
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
/// [`ensure_scripts_dir`], which canonicalizes. A
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

    // There is deliberately NO runtime test that "a relative workspace_path
    // cannot reach ensure_scripts_dir". After a43e19b2 that is a COMPILE-TIME
    // guarantee -- `ensure_scripts_dir` takes a `ResolvedWorkspace`, which only
    // `paths::resolve_workspace` can mint -- so the hazardous call shape does
    // not compile and cannot be exercised from a test.
    //
    // A test was written here and removed. It read
    // `resolve_workspace(relative).map(|ws| ensure_scripts_dir(&ws))`, and
    // since `Result::map` does not run its closure on `Err`, it never called
    // `ensure_scripts_dir` at all: it re-tested `resolve_workspace`, which
    // `paths::tests::a_relative_workspace_is_refused_rather_than_joined_to_the_cwd`
    // already covers. It could not fail for the reason its name claimed, and a
    // test that cannot fail is worse than no test because it reads as coverage.
    //
    // If the newtype ever grows a public constructor, this becomes testable
    // again -- and that is the change to refuse, not the test to restore.

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
        // "Left alone" means no seeding either: a populated workspace does
        // not grow the shipped scripts, however stale or sparse it is. The
        // staleness check may warn; it must not write.
        assert!(
            !scripts.join("lib.lua").exists(),
            "a populated scripts/ was seeded on top of"
        );
        let (_, source) = bootstrap_scripts_dir(&resolved(&workspace)).expect("bootstraps");
        assert!(
            source.contains("pre-existing workspace copy, left alone"),
            "unexpected source sentence: {source}"
        );
    }

    /// The file a fresh workspace must be able to run by bare name. Every
    /// script in the repo `include`s it, so its absence is the "path not
    /// found" a researcher's brand-new `--settings` workspace used to hit.
    const SHIPPED_SCRIPT: &str = "lib.lua";

    /// A brand-new workspace -- the shape a researcher's `--settings <file>`
    /// pointing at its own `workspace_path` produces -- gets the shipped
    /// scripts, not an empty directory. In a debug build they come from the
    /// checkout (byte-identical); in a release build from the embedded
    /// snapshot, which is that same checkout at compile time.
    #[test]
    fn a_new_workspace_is_seeded_with_the_shipped_scripts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("mkdir");

        let (scripts, source) = bootstrap_scripts_dir(&resolved(&workspace)).expect("bootstraps");

        let seeded = scripts.join(SHIPPED_SCRIPT);
        assert!(
            seeded.is_file(),
            "{seeded:?} was not seeded; source: {source}"
        );
        assert!(
            !workspace.join(".scripts-partial").exists(),
            "staging directory left behind"
        );
        #[cfg(debug_assertions)]
        {
            let checkout = repo_scripts_checkout().expect("this test runs from a checkout");
            assert_eq!(
                fs::read(&seeded).expect("read seeded"),
                fs::read(checkout.join(SHIPPED_SCRIPT)).expect("read checkout"),
                "seeded copy differs from the checkout"
            );
            // A copy, not a symlink: a script's `file_write` must land in the
            // workspace, never in the checkout.
            assert!(
                !fs::symlink_metadata(&scripts)
                    .expect("metadata")
                    .file_type()
                    .is_symlink(),
                "{scripts:?} is a symlink into the checkout"
            );
            assert!(
                source.contains("seeded by copying"),
                "unexpected source sentence: {source}"
            );
            // The source names the checkout it copied from -- the answer to
            // "which scripts/ did this run get".
            assert!(
                source.contains(&format!("{checkout:?}")),
                "source does not name the checkout: {source}"
            );
        }
        #[cfg(not(debug_assertions))]
        assert!(
            source.contains("extracted from the compile-time snapshot"),
            "unexpected source sentence: {source}"
        );
    }

    /// Every debug build before seeding existed created `workspace/scripts`
    /// *empty*, so that is the state a workspace made by one is in. It is
    /// indistinguishable from "missing" for every purpose that matters and is
    /// treated as such.
    #[test]
    fn an_empty_scripts_directory_is_seeded_like_a_missing_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(workspace.join("scripts")).expect("mkdir");

        let (scripts, _) = bootstrap_scripts_dir(&resolved(&workspace)).expect("bootstraps");

        assert!(
            scripts.join(SHIPPED_SCRIPT).is_file(),
            "an empty scripts/ was left empty"
        );
    }

    /// The scripts root is `<workspace>/scripts`, full stop. The deleted
    /// `scripts_dir` probed `./scripts` and `../../scripts` relative to the
    /// process CWD *first*, so from the repo root every workspace silently ran
    /// the checkout's scripts. `cargo test -p factorio-bot-core` runs with
    /// `crates/core` as its CWD, where that second probe would match -- so
    /// this test's precondition is that the decoy exists, and its assertion is
    /// that the decoy lost.
    #[test]
    fn resolution_ignores_the_working_directory() {
        let cwd = std::env::current_dir().expect("cwd");
        let decoy = cwd.join("../../scripts");
        assert!(
            decoy.is_dir(),
            "test precondition: {decoy:?} (the old CWD probe's match) must exist"
        );
        let decoy = fs::canonicalize(&decoy).expect("canonicalize decoy");

        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("mkdir");

        let scripts = ensure_scripts_dir(&resolved(&workspace)).expect("bootstraps");

        assert_ne!(scripts, decoy, "resolved to the CWD-relative checkout");
        assert_eq!(
            scripts,
            fs::canonicalize(workspace.join("scripts")).expect("canonicalize")
        );
        assert!(
            scripts.starts_with(fs::canonicalize(dir.path()).expect("canonicalize tempdir")),
            "{scripts:?} is not under the workspace"
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

    /// Renamed from `a_write_path_may_not_climb_out_and_back_in` (doclint-allow:
    /// the retired name is the point of the sentence), which said
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

    /// The defect this whole check exists for, in miniature.
    ///
    /// `run-1788449752-46541` ran a `factory_stage2.lua` five hours older than
    /// the checkout's, which was missing the `record.enclosures()` call the
    /// run was being watched for. Nothing said so.
    #[cfg(debug_assertions)]
    #[test]
    fn a_workspace_copy_older_than_the_checkout_is_reported_by_name() {
        let checkout = tempfile::tempdir().expect("tempdir");
        let workspace = tempfile::tempdir().expect("tempdir");
        fs::write(checkout.path().join("lib.lua"), "-- shared").expect("write");
        fs::write(workspace.path().join("lib.lua"), "-- shared").expect("write");
        fs::write(
            checkout.path().join("factory_stage2.lua"),
            "record.enclosures()",
        )
        .expect("write");
        fs::write(workspace.path().join("factory_stage2.lua"), "").expect("write");
        // A script only the workspace has is the developer's own, not drift.
        fs::write(workspace.path().join("mine.lua"), "-- local").expect("write");

        assert_eq!(
            super::stale_against_checkout(checkout.path(), workspace.path()),
            vec!["factory_stage2.lua".to_string()]
        );
    }

    /// A script the checkout has and the workspace does not is stale too: in a
    /// debug build `workspace/scripts` starts empty, so "missing" is the
    /// ordinary first state and the one most likely to be mistaken for "the
    /// feature does not work".
    #[cfg(debug_assertions)]
    #[test]
    fn a_script_the_workspace_never_received_is_stale() {
        let checkout = tempfile::tempdir().expect("tempdir");
        let workspace = tempfile::tempdir().expect("tempdir");
        fs::write(checkout.path().join("new_thing.lua"), "-- new").expect("write");

        assert_eq!(
            super::stale_against_checkout(checkout.path(), workspace.path()),
            vec!["new_thing.lua".to_string()]
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn an_identical_copy_is_not_reported() {
        let checkout = tempfile::tempdir().expect("tempdir");
        let workspace = tempfile::tempdir().expect("tempdir");
        for dir in [checkout.path(), workspace.path()] {
            fs::write(dir.join("lib.lua"), "-- shared").expect("write");
        }

        assert!(super::stale_against_checkout(checkout.path(), workspace.path()).is_empty());
    }
}
