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
/// verified to live under `root`, so `..` segments cannot escape it — this is a
/// network-reachable boundary, not a local convenience.
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
    /// is multi-byte.
    #[test]
    fn rejects_a_multibyte_path_without_panicking() {
        let (_dir, root) = root();
        let result = resolve_script_path(&root, "ä.lua");
        assert!(result.is_err(), "expected a miss, got {result:?}");
    }

    #[test]
    fn refuses_to_escape_the_root() {
        let (_dir, root) = root();
        for attempt in ["../outside.lua", "/../outside.lua", "sub/../../outside.lua"] {
            assert!(
                resolve_script_path(&root, attempt).is_err(),
                "{attempt} should not resolve"
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
