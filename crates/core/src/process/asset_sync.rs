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

/// The outcome of comparing a directory on disk against a *reference*
/// directory, also on disk.
///
/// The three cases are kept apart on purpose. "the copy differs from the
/// reference" and "there was nothing to compare it against" are different
/// facts, and a reader must never have to guess which one a message is
/// reporting: an absent or empty reference is an environmental condition,
/// while a difference is a real drift someone has to act on.
#[derive(Debug, PartialEq, Eq)]
pub enum DirComparison {
    /// No directory at the reference path at all -- nothing was compared.
    /// Not a verdict about the copy.
    NoReference,
    /// The reference directory exists but holds no files, so again nothing
    /// was compared. Distinctly *not* "the two agree".
    NothingToCompare,
    /// `examined` counts every regular file found under the reference and
    /// compared -- the ones that matched as well as the ones that did not --
    /// so the count answers "how much was actually looked at", and stays put
    /// when a difference appears. `differing` names the subset whose copy is
    /// missing or holds other bytes.
    Compared {
        examined: usize,
        differing: BTreeSet<PathBuf>,
    },
}

/// Compares every regular file under `reference` against the file at the same
/// relative path under `copy`, byte for byte.
///
/// Files the copy has that the reference does not are ignored, matching
/// [`stale_paths`]: this answers "has the copy drifted from the reference",
/// not "is the copy pristine". A file that cannot be read on either side is
/// reported as differing, for the same reason `collect_stale` does: we cannot
/// show it is the same, and the run is about to load it either way.
/// Symlinks in the reference are not followed and not counted; the reference
/// is a source checkout of small text assets.
pub fn compare_dirs(reference: &Path, copy: &Path) -> DirComparison {
    if !reference.is_dir() {
        return DirComparison::NoReference;
    }
    let mut files = Vec::new();
    collect_reference_files(reference, Path::new(""), &mut files);
    if files.is_empty() {
        return DirComparison::NothingToCompare;
    }
    let mut differing = BTreeSet::new();
    for relative in &files {
        let same = match (
            std::fs::read(reference.join(relative)),
            std::fs::read(copy.join(relative)),
        ) {
            (Ok(reference_bytes), Ok(copy_bytes)) => reference_bytes == copy_bytes,
            _ => false,
        };
        if !same {
            differing.insert(relative.clone());
        }
    }
    DirComparison::Compared {
        examined: files.len(),
        differing,
    }
}

fn collect_reference_files(dir: &Path, prefix: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let relative = prefix.join(entry.file_name());
        if file_type.is_dir() {
            collect_reference_files(&entry.path(), &relative, out);
        } else if file_type.is_file() {
            out.push(relative);
        }
    }
}

/// One sentence describing a comparison, for the log line that names which
/// directory a run is about to use.
///
/// Every wording states what was examined rather than what passed, and the
/// two "nothing to compare" cases say so in those words -- they never borrow
/// the vocabulary of agreement ("matches") or of drift ("differs"), so the
/// reader can tell an environmental gap from a real difference without
/// knowing which branch produced the line. `remedy` is appended only when
/// there is something to act on.
pub fn describe(comparison: &DirComparison, reference: &Path, remedy: &str) -> String {
    match comparison {
        DirComparison::NoReference => format!(
            "not compared against a reference: there is no directory at {reference:?} to compare it with"
        ),
        DirComparison::NothingToCompare => format!(
            "not compared against a reference: the directory at {reference:?} holds no files, so there was nothing to compare"
        ),
        DirComparison::Compared {
            examined,
            differing,
        } if differing.is_empty() => format!(
            "identical to {reference:?}: all {examined} file(s) examined hold the same bytes"
        ),
        DirComparison::Compared {
            examined,
            differing,
        } => format!(
            "DIFFERS from {reference:?}: {} of {examined} file(s) examined differ: {:?} -- {remedy}",
            differing.len(),
            differing
        ),
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

/// Comparing an on-disk copy against an on-disk *reference* directory -- the
/// debug-build counterpart to the embed comparison above, where the reference
/// is the repo checkout rather than a compile-time embed.
///
/// The discrimination these tests have to prove is not just "a difference is
/// found". It is that the three outcomes stay apart: a difference, an absent
/// reference and an empty reference must never be reported in each other's
/// words, and the file count must report what was *examined*, so introducing
/// a difference cannot shrink it.
#[cfg(test)]
mod dir_comparison_tests {
    use super::*;

    const REMEDY: &str = "delete the copy and re-run";

    /// A reference holding two files, one of them nested, and an identical
    /// copy of it. Returned as (tempdir, reference, copy); the tempdir is
    /// returned so the caller keeps it alive.
    fn reference_and_copy() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let reference = dir.path().join("reference");
        let copy = dir.path().join("copy");
        for root in [&reference, &copy] {
            std::fs::create_dir_all(root.join("sub")).expect("create dirs");
            std::fs::write(root.join("a.txt"), b"aaa").expect("write a");
            std::fs::write(root.join("sub/b.txt"), b"bbb").expect("write b");
        }
        (dir, reference, copy)
    }

    #[test]
    fn an_identical_copy_is_reported_as_such_with_every_file_counted() {
        let (_dir, reference, copy) = reference_and_copy();

        assert_eq!(
            compare_dirs(&reference, &copy),
            DirComparison::Compared {
                examined: 2,
                differing: BTreeSet::new()
            }
        );
    }

    #[test]
    fn an_edited_copy_names_the_file_and_still_counts_both_as_examined() {
        let (_dir, reference, copy) = reference_and_copy();
        std::fs::write(copy.join("sub/b.txt"), b"edited").expect("edit");

        // `examined` stays 2. A count of what *passed* would drop to 1 here,
        // which would make introducing the very defect this exists to catch
        // look like less work was done rather than like a difference.
        assert_eq!(
            compare_dirs(&reference, &copy),
            DirComparison::Compared {
                examined: 2,
                differing: BTreeSet::from([PathBuf::from("sub/b.txt")])
            }
        );
    }

    #[test]
    fn a_file_the_copy_never_had_differs() {
        let (_dir, reference, copy) = reference_and_copy();
        std::fs::remove_file(copy.join("a.txt")).expect("remove");

        assert_eq!(
            compare_dirs(&reference, &copy),
            DirComparison::Compared {
                examined: 2,
                differing: BTreeSet::from([PathBuf::from("a.txt")])
            }
        );
    }

    #[test]
    fn a_file_only_the_copy_has_is_not_a_difference() {
        let (_dir, reference, copy) = reference_and_copy();
        std::fs::write(copy.join("extra.txt"), b"mine").expect("write extra");

        assert_eq!(
            compare_dirs(&reference, &copy),
            DirComparison::Compared {
                examined: 2,
                differing: BTreeSet::new()
            }
        );
    }

    #[test]
    fn an_absent_reference_is_not_a_verdict_about_the_copy() {
        let (_dir, reference, copy) = reference_and_copy();
        std::fs::remove_dir_all(&reference).expect("remove reference");

        assert_eq!(compare_dirs(&reference, &copy), DirComparison::NoReference);
    }

    #[test]
    fn a_reference_holding_no_files_is_not_agreement() {
        let (_dir, reference, copy) = reference_and_copy();
        std::fs::remove_dir_all(&reference).expect("remove reference");
        std::fs::create_dir_all(reference.join("empty-subdir")).expect("recreate empty");

        // The subdirectory is deliberate: "no files" has to mean no files
        // anywhere under the reference, not just none at the top level.
        assert_eq!(
            compare_dirs(&reference, &copy),
            DirComparison::NothingToCompare
        );
    }

    #[test]
    fn the_two_nothing_to_compare_messages_claim_neither_agreement_nor_drift() {
        let reference = Path::new("/nonexistent/reference");
        for comparison in [DirComparison::NoReference, DirComparison::NothingToCompare] {
            let message = describe(&comparison, reference, REMEDY);
            assert!(
                message.contains("nothing to compare") || message.contains("no directory at"),
                "{comparison:?} must say what is missing, got: {message}"
            );
            assert!(
                !message.to_lowercase().contains("differ"),
                "{comparison:?} must not read as a difference, got: {message}"
            );
            assert!(
                !message.contains("identical"),
                "{comparison:?} must not read as agreement, got: {message}"
            );
            assert!(
                !message.contains(REMEDY),
                "{comparison:?} offers a remedy for a difference that was never found, got: {message}"
            );
        }
    }

    #[test]
    fn a_difference_reads_as_one_and_carries_the_remedy() {
        let (_dir, reference, copy) = reference_and_copy();
        std::fs::write(copy.join("a.txt"), b"edited").expect("edit");

        let message = describe(&compare_dirs(&reference, &copy), &reference, REMEDY);

        assert!(message.contains("DIFFERS"), "got: {message}");
        assert!(
            message.contains("a.txt"),
            "must name the file, got: {message}"
        );
        assert!(
            message.contains("1 of 2"),
            "must report the difference against everything examined, got: {message}"
        );
        assert!(message.contains(REMEDY), "got: {message}");
    }

    #[test]
    fn agreement_reads_as_agreement_and_offers_no_remedy() {
        let (_dir, reference, copy) = reference_and_copy();

        let message = describe(&compare_dirs(&reference, &copy), &reference, REMEDY);

        assert!(message.contains("identical"), "got: {message}");
        assert!(
            message.contains('2'),
            "must say how much it looked at, got: {message}"
        );
        assert!(!message.to_lowercase().contains("differ"), "got: {message}");
        assert!(!message.contains(REMEDY), "got: {message}");
    }
}
