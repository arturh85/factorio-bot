use miette::{miette, IntoDiagnostic, Result};
use std::path::{Path, PathBuf};

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

/// Resolves a client-supplied script path against the scripts root.
///
/// The path may or may not carry a leading `/`. The result is canonicalised and
/// verified to live under `root`, so `..` segments (and symlinks) cannot escape
/// it — this is a network-reachable boundary, not a local convenience.
///
/// The returned path is always strictly inside `root`: a request that
/// resolves to `root` itself (e.g. `"."`) is rejected rather than silently
/// handed back. This keeps the contract unambiguous for callers that build
/// both file-reading and directory-listing endpoints on top of it — "list the
/// root" is the caller's own default, never a value this function returns.
pub fn resolve_script_path(root: &Path, requested: &str) -> Result<PathBuf> {
    let relative = requested.trim_start_matches('/');
    if relative.is_empty() {
        return Err(miette!("empty path"));
    }

    let joined = root.join(relative);
    // canonicalize resolves `..` and symlinks, and fails if the target is absent
    let canonical = std::fs::canonicalize(&joined)
        .into_diagnostic()
        .map_err(|_| miette!("path not found: {requested}"))?;

    if canonical == root {
        return Err(miette!(
            "path resolves to the scripts root itself: {requested}"
        ));
    }

    if !canonical.starts_with(root) {
        return Err(miette!("path escapes the scripts directory: {requested}"));
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

    /// The old caller did `&path[1..]`, which panics on an empty string.
    #[test]
    fn rejects_an_empty_path_without_panicking() {
        let (_dir, root) = root();
        assert!(resolve_script_path(&root, "").is_err());
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

    /// A request that resolves to the scripts root itself (e.g. `"."`) is
    /// rejected rather than silently returning `root`. Callers that want "the
    /// root directory" (a directory-listing endpoint's default) special-case
    /// that themselves instead of relying on this function to hand it back.
    #[test]
    fn rejects_a_path_that_resolves_to_the_root_itself() {
        let (_dir, root) = root();
        let result = resolve_script_path(&root, ".");
        assert!(result.is_err(), "expected a miss, got {result:?}");
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
