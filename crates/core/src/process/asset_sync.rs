//! Detects when an on-disk copy of an `include_dir!` embed has drifted from
//! the embedded snapshot, and provides an explicit, opt-in way to refresh it.
//!
//! Release builds bake `mods/` and `scripts/` into the binary at compile time
//! (see the module docs on `instance_setup`) and extract them into the
//! workspace only once -- the first time the target directory does not
//! exist. After that, nothing keeps the two in sync: editing the repo
//! checkout, or rebuilding the binary, has no effect on a workspace that was
//! already populated. Silently. This module makes that divergence visible
//! instead of leaving it to be discovered as "I edited the file and nothing
//! changed", which has already cost real debugging time on this project.
//!
//! Comparison is a direct byte-for-byte read-and-compare against the
//! in-memory embed, not a persisted hash or mtime sentinel: the embed *is*
//! the only source of truth available at runtime in a release binary (there
//! is no repo checkout to stat), a sentinel file could itself predate this
//! feature or go stale in its own right, and these are small text assets
//! (mod Lua/JSON, script `.lua` files) where reading every file back on
//! startup costs nothing worth optimizing away.
use include_dir::Dir;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Relative paths (as embedded) whose on-disk copy under `extracted_root` is
/// missing or differs from the embedded snapshot.
///
/// Files that exist on disk but are not part of the embed (a user's own extra
/// script, say) are not reported: this answers "has the embedded content
/// drifted from what was extracted", not "is the directory pristine".
pub fn stale_paths(embedded: &Dir, extracted_root: &Path) -> BTreeSet<PathBuf> {
    let mut stale = BTreeSet::new();
    collect_stale(embedded, extracted_root, &mut stale);
    stale
}

fn collect_stale(dir: &Dir, extracted_root: &Path, out: &mut BTreeSet<PathBuf>) {
    for file in dir.files() {
        let relative = file.path();
        match std::fs::read(extracted_root.join(relative)) {
            Ok(on_disk) if on_disk == file.contents() => {}
            _ => {
                out.insert(relative.to_path_buf());
            }
        }
    }
    for subdir in dir.dirs() {
        collect_stale(subdir, extracted_root, out);
    }
}

/// Logs a warning naming the differing files and how to refresh, if any of
/// the embedded snapshot's files differ from what is on disk. A no-op when
/// the copy is already up to date.
pub fn warn_if_stale(embedded: &Dir, extracted_root: &Path, label: &str, refresh_env: &str) {
    let stale = stale_paths(embedded, extracted_root);
    if stale.is_empty() {
        return;
    }
    warn!(
        "<bright-blue>{label}</> workspace copy at <bright-blue>{:?}</> is STALE: {} file(s) differ from the copy baked into this binary: {:?}. Edits to the repo will not take effect until refreshed -- set <bright-blue>{refresh_env}=1</> on the next run to overwrite just the stale files, or delete <bright-blue>{:?}</> to re-extract from scratch.",
        extracted_root,
        stale.len(),
        stale,
        extracted_root
    );
}

/// Re-extracts `embedded` over `extracted_root` when `refresh_env` is set in
/// the environment (to any value), overwriting every file the embed tracks.
/// Extra files the workspace copy has grown on its own are left alone:
/// `include_dir::Dir::extract` only ever writes the paths it embeds.
///
/// Returns whether a refresh happened, so callers can skip the staleness
/// warning when it did.
///
/// Opt-in and env-var-gated rather than automatic: a workspace copy may have
/// been edited on purpose to unblock a run (it happened during the session
/// that motivated this module), and overwriting it on every start would
/// throw that away with no way back.
pub fn refresh_if_requested(
    embedded: &Dir,
    extracted_root: &Path,
    refresh_env: &str,
) -> std::io::Result<bool> {
    if std::env::var_os(refresh_env).is_none() {
        return Ok(false);
    }
    embedded.extract(extracted_root)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal embed to exercise the walk without depending on the real
    /// `mods`/`scripts` trees. Built with the same macro the production code
    /// uses, pointed at a fixture directory checked in for this purpose would
    /// be overkill; `include_dir!` requires a path known at compile time, so
    /// instead these tests embed this very source file's directory and reason
    /// about paths relative to it.
    fn embed() -> Dir<'static> {
        include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/process/fixtures/asset_sync_test")
    }

    #[test]
    fn fixture_has_the_two_files_the_rest_of_this_module_assumes() {
        // Guards the other tests: if the fixture drifts, they should fail with
        // a clear message here rather than a confusing mismatch below. `Dir`
        // has no recursive file listing of its own -- `files()` is top-level
        // only -- so this checks both known paths directly instead of
        // re-implementing the walk `collect_stale` already does.
        let embedded = embed();
        assert!(embedded.get_file("a.txt").is_some(), "missing a.txt");
        assert!(
            embedded.get_file("sub/b.txt").is_some(),
            "missing sub/b.txt"
        );
    }

    #[test]
    fn an_untouched_extraction_has_no_stale_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");

        assert!(stale_paths(&embed(), dir.path()).is_empty());
    }

    #[test]
    fn an_edited_extracted_file_is_reported_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");
        std::fs::write(dir.path().join("a.txt"), b"edited by a user").expect("edit");

        let stale = stale_paths(&embed(), dir.path());
        assert_eq!(stale, BTreeSet::from([PathBuf::from("a.txt")]));
    }

    #[test]
    fn a_missing_extracted_file_is_reported_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");
        std::fs::remove_file(dir.path().join("sub/b.txt")).expect("remove");

        let stale = stale_paths(&embed(), dir.path());
        assert_eq!(stale, BTreeSet::from([PathBuf::from("sub/b.txt")]));
    }

    #[test]
    fn extra_files_not_in_the_embed_are_not_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");
        std::fs::write(dir.path().join("users_own_script.lua"), b"-- mine").expect("write");

        assert!(stale_paths(&embed(), dir.path()).is_empty());
    }

    #[test]
    fn without_the_env_var_refresh_is_a_noop_and_leaves_the_edit_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");
        std::fs::write(dir.path().join("a.txt"), b"edited by a user").expect("edit");

        let refreshed = refresh_if_requested(
            &embed(),
            dir.path(),
            "FACTORIO_BOT_ASSET_SYNC_TEST_UNSET_VAR",
        )
        .expect("refresh_if_requested does not error when unset");

        assert!(!refreshed);
        assert_eq!(
            std::fs::read(dir.path().join("a.txt")).expect("read"),
            b"edited by a user"
        );
    }

    #[test]
    fn with_the_env_var_refresh_overwrites_the_stale_file_and_clears_staleness() {
        // Serialized by nothing but the fact that this is the only test in the
        // module that mutates the environment; std::env is process-global, but
        // `cargo test`'s default per-test-thread model makes this a real risk
        // for env vars shared across tests. This name is unique to this test
        // so no other test can race it.
        let var = "FACTORIO_BOT_ASSET_SYNC_TEST_REFRESH_VAR";
        // SAFETY: edition 2024 made these unsafe because `std::env` is
        // process-global and another thread reading it concurrently is UB.
        // The unique var name above stops another *test* from racing this
        // one for this key, which is the hazard we can actually rule out; it
        // does not rule out a concurrent read of some other var elsewhere in
        // the process. Confined to a test binary, and the alternative is a
        // process-wide env lock these two tests do not earn.
        unsafe { std::env::set_var(var, "1") };

        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");
        std::fs::write(dir.path().join("a.txt"), b"edited by a user").expect("edit");
        assert!(!stale_paths(&embed(), dir.path()).is_empty(), "fixture bug");

        let refreshed =
            refresh_if_requested(&embed(), dir.path(), var).expect("refresh_if_requested");
        // SAFETY: as above -- same key, same test, same reasoning.
        unsafe { std::env::remove_var(var) };

        assert!(refreshed);
        assert!(stale_paths(&embed(), dir.path()).is_empty());
        assert_eq!(
            std::fs::read(dir.path().join("a.txt")).expect("read"),
            embed()
                .get_file("a.txt")
                .expect("fixture has a.txt")
                .contents()
        );
    }

    #[test]
    fn an_extra_workspace_file_survives_a_refresh() {
        let dir = tempfile::tempdir().expect("tempdir");
        embed().extract(dir.path()).expect("extract");
        std::fs::write(dir.path().join("users_own_script.lua"), b"-- mine").expect("write");

        let var = "FACTORIO_BOT_ASSET_SYNC_TEST_REFRESH_KEEPS_EXTRAS";
        // SAFETY: edition 2024 made these unsafe because `std::env` is
        // process-global and another thread reading it concurrently is UB.
        // The unique var name above stops another *test* from racing this
        // one for this key, which is the hazard we can actually rule out; it
        // does not rule out a concurrent read of some other var elsewhere in
        // the process. Confined to a test binary, and the alternative is a
        // process-wide env lock these two tests do not earn.
        unsafe { std::env::set_var(var, "1") };
        refresh_if_requested(&embed(), dir.path(), var).expect("refresh_if_requested");
        // SAFETY: as above -- same key, same test, same reasoning.
        unsafe { std::env::remove_var(var) };

        assert_eq!(
            std::fs::read(dir.path().join("users_own_script.lua")).expect("read"),
            b"-- mine"
        );
    }
}
