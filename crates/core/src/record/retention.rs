//! Keeping the run archive from growing without bound.
//!
//! Video dominates: a 45-minute run archived 290MB of it. Left alone the
//! archive is a slow disk leak.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// How many runs to keep when nothing says otherwise.
pub const DEFAULT_KEEP: usize = 20;

/// A run carrying this file is never reaped, however old it gets.
///
/// A file rather than a manifest field so it can be set with `touch` and
/// survives a run that never wrote a manifest -- which is exactly the run
/// someone is most likely to want to keep.
pub const KEEP_MARKER: &str = ".keep";

/// What a reap did. Returned rather than logged, so a caller cannot fail to
/// notice that data went away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reaped {
    /// Run ids deleted, oldest first.
    pub deleted: Vec<String>,
    /// Runs kept because they carry [`KEEP_MARKER`], however old.
    pub protected: Vec<String>,
}

/// One archived run, and when it began -- for ordering only.
fn started_at(dir: &Path, id: &str) -> u64 {
    // The manifest when there is one, otherwise the timestamp in the id, which
    // this crate minted as `run-<unix seconds>-<sub-second>`. Same fallback the
    // listing endpoint uses, and for the same reason: a run that never finished
    // still has to sort somewhere, and putting it last would reap the crashed
    // runs first.
    if let Ok(bytes) = fs::read(dir.join("manifest.json"))
        && let Ok(manifest) = serde_json::from_slice::<super::Manifest>(&bytes)
    {
        return manifest.started_unix;
    }
    id.strip_prefix("run-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|secs| secs.parse().ok())
        .unwrap_or(0)
}

/// Deletes all but the `keep` newest runs.
///
/// **Only directories that look like runs are considered.** A directory with no
/// `events.jsonl` is left completely alone -- this function deletes whole trees,
/// and the cost of being wrong about what is a run is somebody's unrelated
/// data.
///
/// Runs marked with [`KEEP_MARKER`] are skipped entirely and do not count
/// against the budget, so marking one does not push an unmarked one out.
///
/// Called at archive time rather than on a timer: a background reaper deleting
/// a directory while a viewer reads it is a race nobody needs, and reaping when
/// a run is added is exactly when the archive grows.
pub fn reap(runs_root: &Path, keep: usize) -> io::Result<Reaped> {
    let Ok(entries) = fs::read_dir(runs_root) else {
        return Ok(Reaped {
            deleted: Vec::new(),
            protected: Vec::new(),
        });
    };

    let mut runs: Vec<(u64, String, PathBuf)> = Vec::new();
    let mut protected: Vec<String> = Vec::new();

    for entry in entries.filter_map(Result::ok) {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        // The safety gate: no event log, not a run, not ours to delete.
        if !path.join("events.jsonl").is_file() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if path.join(KEEP_MARKER).exists() {
            protected.push(id);
            continue;
        }
        runs.push((started_at(&path, &id), id, path));
    }

    // Newest first, so everything past `keep` is the tail.
    runs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    protected.sort();

    let mut deleted = Vec::new();
    for (_, id, path) in runs.into_iter().skip(keep) {
        fs::remove_dir_all(&path)?;
        deleted.push(id);
    }
    deleted.reverse(); // oldest first, which is the order they went
    Ok(Reaped { deleted, protected })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-reap-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(root: &Path, id: &str) -> PathBuf {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("events.jsonl"), b"{}\n").unwrap();
        dir
    }

    #[test]
    fn the_newest_runs_are_kept_and_the_rest_go() {
        let root = root("basic");
        for id in ["run-1000-1", "run-2000-1", "run-3000-1", "run-4000-1"] {
            seed(&root, id);
        }
        let reaped = reap(&root, 2).unwrap();
        assert_eq!(reaped.deleted, vec!["run-1000-1", "run-2000-1"]);
        assert!(root.join("run-4000-1").exists());
        assert!(!root.join("run-1000-1").exists());
    }

    #[test]
    fn a_marked_run_survives_however_old_it_is() {
        let root = root("marked");
        let old = seed(&root, "run-1000-1");
        fs::write(old.join(KEEP_MARKER), b"").unwrap();
        for id in ["run-2000-1", "run-3000-1"] {
            seed(&root, id);
        }
        let reaped = reap(&root, 1).unwrap();
        assert!(old.exists(), "a marked run is never reaped");
        assert_eq!(reaped.protected, vec!["run-1000-1"]);
        // And it did not consume the budget: the newest unmarked run stays too.
        assert!(root.join("run-3000-1").exists());
        assert_eq!(reaped.deleted, vec!["run-2000-1"]);
    }

    #[test]
    fn a_directory_that_is_not_a_run_is_never_touched() {
        // This function deletes whole trees. Being wrong about what a run is
        // costs somebody their unrelated data.
        let root = root("notarun");
        let stranger = root.join("my-important-notes");
        fs::create_dir_all(&stranger).unwrap();
        fs::write(stranger.join("thoughts.txt"), b"keep me").unwrap();
        seed(&root, "run-1000-1");
        seed(&root, "run-2000-1");

        let reaped = reap(&root, 1).unwrap();
        assert!(stranger.join("thoughts.txt").exists());
        assert_eq!(reaped.deleted, vec!["run-1000-1"]);
    }

    #[test]
    fn deletions_are_reported_rather_than_silent() {
        let root = root("reported");
        seed(&root, "run-1000-1");
        seed(&root, "run-2000-1");
        let reaped = reap(&root, 1).unwrap();
        assert_eq!(
            reaped.deleted.len(),
            1,
            "a caller must be able to see that data went away"
        );
    }

    #[test]
    fn keeping_more_than_there_are_deletes_nothing() {
        let root = root("under");
        seed(&root, "run-1000-1");
        assert_eq!(reap(&root, 20).unwrap().deleted, Vec::<String>::new());
        assert!(root.join("run-1000-1").exists());
    }

    #[test]
    fn a_missing_runs_directory_is_not_an_error() {
        let root = root("missing");
        let reaped = reap(&root.join("never-created"), 5).unwrap();
        assert!(reaped.deleted.is_empty());
    }

    #[test]
    fn an_unfinished_run_sorts_by_its_id_not_to_the_oldest_slot() {
        // A crashed run has no manifest. Treating it as timestamp zero would
        // reap exactly the runs worth keeping.
        let root = root("unfinished");
        let finished = seed(&root, "run-1000-1");
        fs::write(
            finished.join("manifest.json"),
            br#"{"run_id":"run-1000-1","started_unix":1000,"finished_unix":1100,
                 "outcome":"done","elapsed_ticks":1,"events":1,"splits":0}"#,
        )
        .unwrap();
        seed(&root, "run-9000-1"); // newer, crashed, no manifest

        let reaped = reap(&root, 1).unwrap();
        assert_eq!(reaped.deleted, vec!["run-1000-1"]);
        assert!(root.join("run-9000-1").exists());
    }
}
