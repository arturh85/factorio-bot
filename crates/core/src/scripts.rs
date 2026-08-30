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
        return std::fs::canonicalize(workspace_scripts).into_diagnostic();
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
/// directory is left alone, so this is safe to call on every start. Debug
/// builds deliberately skip the extraction: `include_dir!` bundles this
/// repository's own `scripts/` directory, and a developer checkout already has
/// them.
pub fn ensure_scripts_dir(workspace_path: &Path) -> Result<PathBuf> {
    let workspace_scripts = workspace_path.join("scripts");
    if !workspace_scripts.is_dir() {
        std::fs::create_dir_all(&workspace_scripts).into_diagnostic()?;

        #[cfg(not(debug_assertions))]
        crate::process::instance_setup::PLANS_CONTENT
            .extract(workspace_scripts.clone())
            .map_err(|err| miette!("failed to extract bundled scripts: {err:?}"))?;
    }
    std::fs::canonicalize(&workspace_scripts).into_diagnostic()
}

/// Resolves a client-supplied script path against the scripts root.
///
/// The path may or may not carry a leading `/`. The result is canonicalised and
/// verified to live under `root`, so `..` segments (and symlinks) cannot escape
/// it — this is a network-reachable boundary, not a local convenience.
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

        let scripts = ensure_scripts_dir(&workspace).expect("bootstraps");

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

    /// Called on every server start, so it must be idempotent and must not
    /// disturb scripts already on disk.
    #[test]
    fn ensure_scripts_dir_leaves_an_existing_directory_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(workspace.join("scripts")).expect("mkdir");
        fs::write(workspace.join("scripts").join("mine.lua"), "-- mine").expect("write");

        let scripts = ensure_scripts_dir(&workspace).expect("bootstraps");
        let scripts_again = ensure_scripts_dir(&workspace).expect("bootstraps twice");

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
