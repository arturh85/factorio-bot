//! `GET /api/v1/frames` and `GET /api/v1/frames/{name}` -- serving the
//! tick-cadence screenshots the mod captures.
//!
//! **A frame's filename is a measurement, not a claim.** The mod writes the
//! tick from `game.tick` *inside the game*, so the name on disk is the fact.
//! The manifest this module answers with is therefore *derived from a
//! directory listing* every time it is asked for, and stores nothing of its
//! own. Two consequences follow, and both are requirements rather than
//! preferences:
//!
//! - **Never renumber, backfill, interpolate or smooth a gap.** `force_render`
//!   is not honoured on multiplayer clients catching up, so frames genuinely
//!   drop. A manifest computed from a start tick and a stride would be
//!   contiguous, plausible, and wrong -- and it cannot fail to look right,
//!   which is exactly what makes that shortcut dangerous. A gap in the tick
//!   sequence is the truth and must survive to the client.
//! - **A file whose name does not parse is reported with a null tick, not
//!   skipped.** Skipping would hide a file that exists on disk.
//!
//! Frames are captured per client (`by_player`), so with N clients there are N
//! `<workspace>/client<N>/script-output/frames/` directories. This
//! enumerates `client*` rather than hardcoding `client1`, so adding clients
//! needs no change here.

use crate::error::ErrorResponse;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use factorio_bot_core::scripts::resolve_script_path;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use utoipa::ToSchema;

/// One frame file exactly as it exists on disk. One entry per file found
/// under a client's frames directory -- nothing is computed, nothing is
/// inferred about files that are not there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FrameEntry {
    /// Which `client<N>` directory this frame was captured by.
    pub client: u8,
    /// `game.tick` at capture, parsed from the filename, or `null` when the
    /// name does not fit the pattern. Always present as a key -- `null` says
    /// "we looked and there is nothing to report", not "we didn't ask".
    pub tick: Option<u64>,
    /// The camera id, parsed from the filename, or `null` alongside `tick`
    /// when the name does not parse.
    pub camera: Option<String>,
    /// The filename exactly as it appears on disk.
    pub name: String,
    pub bytes: u64,
}

/// `GET /api/v1/frames` response.
///
/// `clients` and `frames` together distinguish the three states a caller must
/// not conflate:
/// - no workspace, or no `client<N>` directory at all: `clients` is empty --
///   the run has never happened.
/// - a `client<N>` directory exists but has produced no frames yet: `clients`
///   names it, `frames` is empty -- capture has not run.
/// - frames are present: both are non-empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FramesManifest {
    /// Every `client<N>` directory discovered under the workspace, regardless
    /// of whether it has produced a frame yet.
    pub clients: Vec<u8>,
    /// One entry per file actually on disk, sorted by `(client, tick,
    /// camera)` with unparsed entries last, so iteration order is contractual
    /// rather than filesystem order.
    pub frames: Vec<FrameEntry>,
}

/// Parses `tick-<digits>-<camera>.jpg`, or reports that a name does not fit
/// the pattern by answering `(None, None)`.
///
/// Deliberately does **not** assume the camera id is hyphen-free: it splits on
/// the *first* hyphen after the digit run and takes everything up to `.jpg` as
/// the camera id, however many hyphens that contains. `tick-0001800-bot-1.jpg`
/// must yield tick `1800`, camera `bot-1` -- a last-hyphen split would instead
/// read camera `1` and silently fold `bot` into a mis-parsed tick component,
/// which is a plausible-looking wrong answer, exactly the failure mode this
/// module exists to avoid. Splitting from the correct end (immediately after
/// the numeric tick, which cannot itself contain a hyphen) removes any
/// dependency on the producer's camera-naming scheme.
fn parse_frame_name(name: &str) -> (Option<u64>, Option<String>) {
    let Some(rest) = name.strip_prefix("tick-") else {
        return (None, None);
    };
    // The digit run cannot contain a hyphen, so the first hyphen in `rest`
    // (if any) is exactly the separator between the tick and the camera id.
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
    // `remainder` still carries the separating hyphen itself.
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

/// One `client<N>` directory discovered under the workspace, and where its
/// frames would live.
struct ClientDir {
    client: u8,
    frames_dir: PathBuf,
}

/// Enumerates `<workspace>/client<N>/` directories, sorted by client number.
///
/// Returns an empty list when `workspace` does not exist, or exists but holds
/// no `client<N>` directory -- exactly the "the run has never happened" state
/// `FramesManifest::clients` reports as empty. A `client<N>` directory is
/// listed here even when its `script-output/frames` subdirectory does not
/// exist yet: that is the "capture has not run" state, distinct from "no run
/// at all", and it is [`list_frame_entries`] below (not this function) that
/// treats a missing frames directory as zero frames.
fn discover_client_dirs(workspace: &std::path::Path) -> Vec<ClientDir> {
    let Ok(read_dir) = std::fs::read_dir(workspace) else {
        return Vec::new();
    };
    let mut dirs: Vec<ClientDir> = read_dir
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let file_name = entry.file_name();
            let name = file_name.to_str()?;
            let suffix = name.strip_prefix("client")?;
            let client: u8 = suffix.parse().ok()?;
            Some(ClientDir {
                client,
                frames_dir: entry.path().join("script-output").join("frames"),
            })
        })
        .collect();
    dirs.sort_by_key(|dir| dir.client);
    dirs
}

/// Lists the frame files under one client's frames directory.
///
/// A frames directory that does not exist yet (capture has not run) answers
/// an empty list rather than an error -- the same treatment
/// `list_scripts`/`read_script` do not need, because a script route always
/// requires its target to exist first; here "not there yet" is one of the
/// states the manifest must represent, not a failure.
///
/// Every file is reported, whatever its name -- there is no extension or
/// pattern filter. Skipping a file that does not look like a frame would be
/// exactly the "skip what does not parse" mistake the manifest exists to
/// avoid; a stray file lands in the manifest with a null tick instead.
fn list_frame_entries(dir: &ClientDir) -> Vec<FrameEntry> {
    let Ok(read_dir) = std::fs::read_dir(&dir.frames_dir) else {
        return Vec::new();
    };
    read_dir
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = entry.metadata().ok()?.len();
            let (tick, camera) = parse_frame_name(&name);
            Some(FrameEntry {
                client: dir.client,
                tick,
                camera,
                name,
                bytes,
            })
        })
        .collect()
}

/// Orders entries by `(client, tick, camera)` with entries whose name did not
/// parse (`tick: None`) sorted last, so a manifest's iteration order is part
/// of the contract rather than an accident of the filesystem's own order.
fn sort_frame_entries(frames: &mut [FrameEntry]) {
    frames.sort_by(|a, b| {
        a.client
            .cmp(&b.client)
            .then_with(|| match (a.tick, b.tick) {
                (Some(a_tick), Some(b_tick)) => a_tick.cmp(&b_tick),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.camera.cmp(&b.camera))
            // Final tiebreak so the order is fully deterministic even between
            // two unparsed entries on the same client.
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Resolves the workspace root this request's frames live under.
///
/// Mirrors `manage::scripts::scripts_root_path`: reads `workspace_path`
/// straight out of settings rather than going through
/// `factorio_bot_core::scripts::scripts_dir`'s CWD-relative fallback, for the
/// same reason -- a request must be bound to the *configured* workspace, not
/// to wherever the server process's working directory happens to be.
///
/// Deliberately does not require the directory to exist, and does not
/// canonicalize it: "no workspace yet" is one of the three states
/// `GET /api/v1/frames` must represent, not an error, and canonicalizing a
/// missing path would fail.
async fn workspace_root(state: &AppState) -> Result<PathBuf, ErrorResponse> {
    let workspace_path = state.settings.read().await.factorio.workspace_path.clone();
    factorio_bot_core::paths::resolve_workspace(workspace_path.as_ref())
        .map(|resolved| resolved.as_path().to_path_buf())
        .map_err(|err| ErrorResponse::bad_request(err.to_string()))
}

/// Builds the manifest from the current state of the filesystem. Called fresh
/// on every request -- see the module docs for why nothing is cached or
/// computed from a previous answer.
async fn build_manifest(state: &AppState) -> Result<FramesManifest, ErrorResponse> {
    let workspace = workspace_root(state).await?;
    let client_dirs = discover_client_dirs(&workspace);
    let clients: Vec<u8> = client_dirs.iter().map(|dir| dir.client).collect();

    let mut frames = Vec::new();
    for dir in &client_dirs {
        frames.extend(list_frame_entries(dir));
    }
    sort_frame_entries(&mut frames);

    Ok(FramesManifest { clients, frames })
}

/// Returns the manifest of frames captured so far.
///
/// Derived from a directory listing on every call -- see the module docs for
/// why this is not computed from a start tick and a stride.
#[utoipa::path(
    get,
    path = "/api/v1/frames",
    tag = "Admin",
    responses(
        (status = 200, body = FramesManifest),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn list_frames(
    State(state): State<AppState>,
) -> Result<Json<FramesManifest>, ErrorResponse> {
    Ok(Json(build_manifest(&state).await?))
}

/// Serves one frame's JPEG bytes.
///
/// The route carries no client segment, so `name` is tried against each
/// discovered `client<N>` directory in turn (lowest client number first),
/// answering the first match. Each directory is bounds-checked with
/// [`resolve_script_path`] -- the same traversal guard
/// `manage::scripts` uses -- rather than a second, independent
/// implementation of it: this endpoint is unauthenticated, so that guard is
/// what stands between an HTTP caller and the filesystem.
///
/// **Addressed by `(client, name)`, not by `name` alone.** Per-bot cameras mean
/// `client1` and `client2` both capture at tick 300 and both files are correct
/// and different; they are distinguished by the directory they are in, which
/// is exactly what [`FrameEntry::client`] reports. An earlier version of this
/// route searched the client directories in order and returned the first
/// match, which made the manifest honest and the bytes not: two entries with
/// the same `name` resolved to one image, and the higher-numbered client's
/// frame was unreachable. A scrubber showing two bots would have shown the
/// same picture twice and nothing would have said so.
///
/// Still no uniquifier: nothing here renames a colliding file so both survive.
/// That would let a genuine same-client double-write live on disk
/// indistinguishably from two legitimate frames. The two cases are different —
/// two clients at one tick is expected and addressable; one client twice at
/// one tick is a defect and must not be given a home.
#[utoipa::path(
    get,
    path = "/api/v1/frames/{client}/{name}",
    tag = "Admin",
    params(
        ("client" = u8, Path, description = "the client number, as reported by FrameEntry.client"),
        ("name" = String, Path, description = "a frame's filename, as reported by GET /api/v1/frames"),
    ),
    responses(
        (status = 200, content_type = "image/jpeg", description = "the frame's JPEG bytes"),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_frame(
    State(state): State<AppState>,
    Path((client, name)): Path<(u8, String)>,
) -> Result<Response, ErrorResponse> {
    let workspace = workspace_root(&state).await?;
    let client_dirs = discover_client_dirs(&workspace);

    // Exactly the one the caller named. No fallback to another client: a
    // fallback is how the previous version returned a different client's
    // frame under the requested name, which is worse than a 404 because the
    // caller cannot tell it happened.
    let dir = client_dirs
        .iter()
        .find(|dir| dir.client == client)
        .ok_or_else(|| ErrorResponse::not_found(format!("no such client: {client}")))?;

    let frames_root = std::fs::canonicalize(&dir.frames_dir)
        .map_err(|_| ErrorResponse::not_found(format!("no frames for client {client}")))?;
    let resolved = resolve_script_path(&frames_root, &name).map_err(ErrorResponse::from)?;
    if !resolved.is_file() {
        return Err(ErrorResponse::not_found(format!("no such frame: {name}")));
    }
    let bytes = std::fs::read(&resolved)
        .map_err(|err| ErrorResponse::internal(format!("failed to read frame: {err}")))?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
            (
                header::CACHE_CONTROL,
                // A tick never recurs, so a frame is immutable once written --
                // a scrubber re-requests the same frames constantly, and this
                // header is the difference between a usable one and a slow one.
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parseable_name_yields_its_tick_and_camera() {
        let (tick, camera) = parse_frame_name("tick-0000000300-front.jpg");
        assert_eq!(tick, Some(300));
        assert_eq!(camera.as_deref(), Some("front"));
    }

    /// The camera id is not assumed hyphen-free: everything after the tick's
    /// separating hyphen and before `.jpg` belongs to it, however many
    /// hyphens it contains. A last-hyphen split would instead answer camera
    /// `1` here, silently -- exactly the failure this test pins against.
    #[test]
    fn a_hyphenated_camera_id_survives_whole() {
        let (tick, camera) = parse_frame_name("tick-0001800-bot-1.jpg");
        assert_eq!(tick, Some(1800));
        assert_eq!(camera.as_deref(), Some("bot-1"));
    }

    #[test]
    fn an_unparseable_name_yields_no_tick_and_no_camera() {
        for name in [
            "not-a-frame.jpg",
            "tick-abc-front.jpg",
            "tick-300.jpg",
            "tick-300-.jpg",
            "tick-.jpg",
            "",
        ] {
            let (tick, camera) = parse_frame_name(name);
            assert_eq!(tick, None, "{name} should not yield a tick");
            assert_eq!(camera, None, "{name} should not yield a camera");
        }
    }

    /// Pins the sort key itself, independent of any filesystem/HTTP plumbing:
    /// `(client, tick, camera)`, unparsed (`tick: None`) entries last.
    #[test]
    fn entries_sort_by_client_then_tick_with_unparsed_entries_last() {
        let mut frames = vec![
            FrameEntry {
                client: 1,
                tick: None,
                camera: None,
                name: "junk.txt".into(),
                bytes: 0,
            },
            FrameEntry {
                client: 1,
                tick: Some(900),
                camera: Some("front".into()),
                name: "tick-0000000900-front.jpg".into(),
                bytes: 1,
            },
            FrameEntry {
                client: 1,
                tick: Some(300),
                camera: Some("front".into()),
                name: "tick-0000000300-front.jpg".into(),
                bytes: 1,
            },
        ];
        sort_frame_entries(&mut frames);
        let names: Vec<&str> = frames.iter().map(|frame| frame.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "tick-0000000300-front.jpg",
                "tick-0000000900-front.jpg",
                "junk.txt",
            ]
        );
    }
}
