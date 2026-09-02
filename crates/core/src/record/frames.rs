//! Archiving a run's frames out of the transient workspace.
//!
//! `<workspace>/client<N>/script-output/frames/` is wiped by the next run, so a
//! run that is to survive must take its frames with it. The sidecar
//! `frames/run.json` says which run the files on disk belong to, and that id --
//! never a tick range -- is what decides whether they are ours.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The sidecar naming the run a frames directory belongs to.
const RUN_SIDECAR: &str = "run.json";

/// One archived frame, as recorded in `frames/index.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ArchivedFrame {
    /// The `client<N>` the frame came from -- the **client** number, despite
    /// this field's name.
    ///
    /// It is not a bot id, and clients and bots are not 1:1. A client writes
    /// whichever cameras it was asked for, and the assignment is neither fixed
    /// nor per-bot: on the run archived here, `client3` holds `follow`,
    /// `bot-1` and `area` while `client1` holds `bot-4`. Which bot a frame is
    /// *of* is in the `camera` field, and only when the camera names one --
    /// `follow` and `area` name none.
    pub bot: u8,
    /// `game.tick` at capture, or `null` when the name does not parse.
    pub tick: Option<u64>,
    /// The camera id, or `null` alongside `tick`.
    pub camera: Option<String>,
    /// Path relative to the run directory.
    pub file: String,
}

/// Parses `tick-<digits>-<camera>.jpg`, or reports that a name does not fit
/// the pattern by answering `(None, None)`.
///
/// Deliberately does **not** assume the camera id is hyphen-free: it splits on
/// the *first* hyphen after the digit run and takes everything up to `.jpg` as
/// the camera id, however many hyphens that contains. `tick-0001800-bot-1.jpg`
/// must yield tick `1800`, camera `bot-1` -- a last-hyphen split would instead
/// read camera `1` and silently fold `bot` into a mis-parsed tick component,
/// which is a plausible-looking wrong answer. Splitting from the correct end
/// (immediately after the numeric tick, which cannot itself contain a hyphen)
/// removes any dependency on the producer's camera-naming scheme.
///
/// Lives in `core` rather than beside the HTTP handler that first needed it,
/// because the archive needs the same answer. Two parsers for one filename
/// format is how two halves of a system come to disagree about what a file is.
pub fn parse_frame_name(name: &str) -> (Option<u64>, Option<String>) {
    let Some(rest) = name.strip_prefix("tick-") else {
        return (None, None);
    };
    let separator = match rest.find('-') {
        Some(index) => index,
        None => return (None, None),
    };
    let (tick_str, remainder) = rest.split_at(separator);
    if tick_str.is_empty() || !tick_str.bytes().all(|byte| byte.is_ascii_digit()) {
        return (None, None);
    }
    let Ok(tick) = tick_str.parse::<u64>() else {
        return (None, None);
    };
    let Some(camera_and_extension) = remainder.strip_prefix('-') else {
        return (None, None);
    };
    let Some(camera) = camera_and_extension.strip_suffix(".jpg") else {
        return (None, None);
    };
    if camera.is_empty() {
        return (None, None);
    }
    (Some(tick), Some(camera.to_string()))
}

/// Reads the run id a frames directory claims, or `None` for any reason at all.
fn read_run_id(frames_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(frames_dir.join(RUN_SIDECAR)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("run")?.as_str().map(str::to_owned)
}

/// Every `client<N>` directory under the workspace, with its frames directory.
fn client_dirs(workspace: &Path) -> Vec<(u8, PathBuf)> {
    let Ok(read_dir) = fs::read_dir(workspace) else {
        return Vec::new();
    };
    let mut dirs: Vec<(u8, PathBuf)> = read_dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name();
            let client: u8 = name.to_str()?.strip_prefix("client")?.parse().ok()?;
            Some((client, entry.path().join("script-output").join("frames")))
        })
        .collect();
    dirs.sort_by_key(|(client, _)| *client);
    dirs
}

/// Copies this run's frames into `<run_dir>/frames/<bot>/` and returns the
/// index.
///
/// **Only directories whose `run.json` names `run_id` are copied.** A frames
/// directory left behind by an earlier run is skipped whole. Matching on a tick
/// range instead would copy it: every run starts near tick 0, so two runs'
/// ranges overlap almost entirely.
///
/// Filenames are preserved rather than rewritten into a `<camera>/<tick>.jpg`
/// tree. The producer's name is the fact -- it carries the tick from inside the
/// game -- and re-deriving a path from a parse means a file whose name did not
/// parse has nowhere to go. Here it is copied like any other and appears in the
/// index with a null tick, which is the honest report.
pub fn archive_frames(
    workspace: &Path,
    run_dir: &Path,
    run_id: &str,
) -> io::Result<Vec<ArchivedFrame>> {
    let mut archived = Vec::new();

    for (bot, frames_dir) in client_dirs(workspace) {
        if read_run_id(&frames_dir).as_deref() != Some(run_id) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&frames_dir) else {
            continue;
        };
        let target = run_dir.join("frames").join(bot.to_string());
        fs::create_dir_all(&target)?;

        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|name| name != RUN_SIDECAR)
            .collect();
        names.sort();

        for name in names {
            fs::copy(frames_dir.join(&name), target.join(&name))?;
            let (tick, camera) = parse_frame_name(&name);
            archived.push(ArchivedFrame {
                bot,
                tick,
                camera,
                file: format!("frames/{bot}/{name}"),
            });
        }
    }

    // Sorted by (bot, tick, camera) with unparsed entries last, so iteration
    // order is contractual rather than filesystem order.
    archived.sort_by(|a, b| {
        (a.bot, a.tick.is_none(), a.tick, a.camera.clone()).cmp(&(
            b.bot,
            b.tick.is_none(),
            b.tick,
            b.camera.clone(),
        ))
    });

    let index = run_dir.join("frames").join("index.json");
    if let Some(parent) = index.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &index,
        serde_json::to_vec_pretty(&archived).map_err(io::Error::other)?,
    )?;
    Ok(archived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-frames-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_client(ws: &Path, client: u8, run: &str, names: &[&str]) {
        let frames = ws
            .join(format!("client{client}"))
            .join("script-output")
            .join("frames");
        fs::create_dir_all(&frames).unwrap();
        fs::write(frames.join(RUN_SIDECAR), format!(r#"{{"run":"{run}"}}"#)).unwrap();
        for name in names {
            fs::write(frames.join(name), b"jpeg").unwrap();
        }
    }

    #[test]
    fn a_camera_id_may_contain_hyphens() {
        assert_eq!(
            parse_frame_name("tick-0001800-bot-1.jpg"),
            (Some(1800), Some("bot-1".to_string()))
        );
    }

    #[test]
    fn a_name_that_does_not_fit_parses_to_nulls_rather_than_guessing() {
        for name in ["run.json", "tick-.jpg", "tick-abc-front.jpg", "frame.jpg"] {
            assert_eq!(parse_frame_name(name), (None, None), "for {name}");
        }
    }

    #[test]
    fn frames_from_another_run_are_not_archived() {
        // The decoy case: a stale directory left by an earlier run. This is the
        // bug that shipped on 2026-08-30 and it stays covered.
        let ws = workspace("decoy");
        seed_client(&ws, 1, "ours", &["tick-0000300-front.jpg"]);
        seed_client(
            &ws,
            2,
            "STALE-RUN-MUST-NOT-SURVIVE",
            &["tick-0000300-front.jpg"],
        );
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        let archived = archive_frames(&ws, &run_dir, "ours").unwrap();
        assert_eq!(archived.len(), 1, "only our own frames may be archived");
        assert_eq!(archived[0].bot, 1);
        assert!(
            !run_dir.join("frames").join("2").exists(),
            "the stale client must not even get a directory"
        );
    }

    #[test]
    fn a_directory_with_no_sidecar_is_skipped() {
        // No sidecar means nothing claims these frames. Copying them would
        // attribute another run's images to this one.
        let ws = workspace("nosidecar");
        let frames = ws.join("client1").join("script-output").join("frames");
        fs::create_dir_all(&frames).unwrap();
        fs::write(frames.join("tick-0000300-front.jpg"), b"jpeg").unwrap();
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        assert!(archive_frames(&ws, &run_dir, "ours").unwrap().is_empty());
    }

    #[test]
    fn an_unparsable_name_is_copied_and_reported_with_a_null_tick() {
        let ws = workspace("unparsable");
        seed_client(&ws, 1, "ours", &["tick-0000300-front.jpg", "notes.txt"]);
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        let archived = archive_frames(&ws, &run_dir, "ours").unwrap();
        assert_eq!(archived.len(), 2, "an odd file is copied, not dropped");
        assert_eq!(archived[1].tick, None, "unparsed entries sort last");
        assert!(run_dir.join("frames/1/notes.txt").exists());
    }

    /// **A run that captured nothing must read as "none were captured".**
    ///
    /// The normal case since screenshot cameras were retired (2026-09-02):
    /// the mod starts a capture session, claims the directory with
    /// `run.json`, and registers no camera. The archive then has to produce
    /// an `index.json` that says `[]` -- a missing file is a viewer tripping
    /// over an absence, and `manifest.frames` reading 0 beside no index at
    /// all is indistinguishable from a capture that failed.
    #[test]
    fn a_run_that_captured_no_frames_reports_an_empty_index_not_a_missing_file() {
        let ws = workspace("noframes");
        seed_client(&ws, 1, "ours", &[]);
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        let archived = archive_frames(&ws, &run_dir, "ours").unwrap();
        assert!(
            archived.is_empty(),
            "nothing was captured, so nothing is archived"
        );

        let index = run_dir.join("frames").join("index.json");
        let text = fs::read_to_string(&index)
            .unwrap_or_else(|err| panic!("index.json must exist at {index:?}: {err}"));
        let from_disk: Vec<ArchivedFrame> = serde_json::from_str(&text).unwrap();
        assert!(
            from_disk.is_empty(),
            "an empty list is the report; no file at all is a shrug"
        );
    }

    /// The same answer for a run with no `client<N>` directory at all -- a
    /// planning-only run, or `--clients 0`. It is still "none were captured".
    #[test]
    fn a_workspace_with_no_clients_still_writes_the_empty_index() {
        let ws = workspace("noclients");
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        assert!(archive_frames(&ws, &run_dir, "ours").unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(run_dir.join("frames").join("index.json")).unwrap(),
            "[]"
        );
    }

    #[test]
    fn the_index_is_written_and_matches_what_was_copied() {
        let ws = workspace("index");
        seed_client(
            &ws,
            1,
            "ours",
            &["tick-0000600-front.jpg", "tick-0000300-front.jpg"],
        );
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        let archived = archive_frames(&ws, &run_dir, "ours").unwrap();
        let text = fs::read_to_string(run_dir.join("frames").join("index.json")).unwrap();
        let from_disk: Vec<ArchivedFrame> = serde_json::from_str(&text).unwrap();
        assert_eq!(from_disk, archived);
        assert_eq!(
            from_disk.iter().map(|f| f.tick).collect::<Vec<_>>(),
            vec![Some(300), Some(600)],
            "sorted by tick, not by filesystem order"
        );
    }
}
