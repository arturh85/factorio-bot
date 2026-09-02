//! `GET /api/v1/video`, `/video/file` and `/video/ticks` -- serving the
//! host-side recording of the current run.
//!
//! Three routes because they answer three different questions and a browser
//! fetches them differently:
//!
//! - the **manifest** says whether there is a recording, what it is, and how it
//!   ended. It is derived from a directory listing on every request and stores
//!   nothing of its own, exactly like `GET /api/v1/frames`.
//! - the **file** is bytes for a `<video>` element, and therefore must support
//!   `Range` requests.
//! - the **clock** is the `(tick, wall_ms)` table the viewer interpolates. It is
//!   separate from the manifest because it is thousands of lines and a caller
//!   sizing an axis only needs `VideoManifest::tick_range`.
//!
//! # Why the file route is not `get_frame`
//!
//! `get_frame` does `std::fs::read` into a `Vec<u8>` and answers a plain 200.
//! For a JPEG that is right; for a video it is two separate failures. A
//! `<video>` served that way **cannot seek at all** -- the browser needs
//! `Accept-Ranges`/206 to fetch the byte range around a timestamp -- and every
//! request loads the whole recording into server memory. So this route
//! delegates to [`tower_http::services::ServeFile`], which is already a
//! dependency (`crates/server/src/spa.rs` uses it) and handles conditional and
//! range requests.
//!
//! **`resolve_script_path` still runs first, before `ServeFile` is
//! constructed**, exactly as it does for frames. These endpoints are
//! unauthenticated, so that guard is the only thing between an HTTP caller and
//! the filesystem, and nothing about delegating the read may route around it.

use crate::error::ErrorResponse;
use crate::manage::frames::workspace_root;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use factorio_bot_core::record::TickSample;
#[cfg(test)]
use factorio_bot_core::record::parse_tick_samples;
use factorio_bot_core::record::video::{
    TICKS_FILE, VIDEO_DIR, VideoManifest, clock::read_tick_samples, read_video_dir,
};
use factorio_bot_core::scripts::resolve_script_path;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tower::util::ServiceExt;
use tower_http::services::ServeFile;
use utoipa::ToSchema;

/// `GET /api/v1/video/ticks` response.
///
/// The whole clock, in file order. The viewer interpolates between the two
/// nearest samples and **refuses to interpolate across a gap** -- which is why
/// the gap lines are in this list rather than filtered out of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VideoTicksResponse {
    pub samples: Vec<TickSample>,
    /// Lines that did not parse -- in practice the torn last line of a recorder
    /// that was killed. Reported rather than swallowed, matching
    /// `EventsResponse.skipped`.
    pub skipped: usize,
}

/// `<workspace>/video/`.
async fn video_root(state: &AppState) -> Result<PathBuf, ErrorResponse> {
    Ok(workspace_root(state).await?.join(VIDEO_DIR))
}

/// The manifest of the current run's recording.
///
/// Answers a manifest describing *nothing* rather than a 404 when no video was
/// recorded: "this run had no video" is the ordinary state -- video is opt-in
/// -- and a 404 would make a caller unable to tell it from a broken route.
#[utoipa::path(
    get,
    path = "/api/v1/video",
    tag = "Admin",
    responses(
        (status = 200, body = VideoManifest),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_video(
    State(state): State<AppState>,
) -> Result<Json<VideoManifest>, ErrorResponse> {
    Ok(Json(read_video_dir(&video_root(&state).await?)))
}

/// The clock: every `(tick, wall_ms)` pair the recorder sampled.
#[utoipa::path(
    get,
    path = "/api/v1/video/ticks",
    tag = "Admin",
    responses(
        (status = 200, body = VideoTicksResponse),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_video_ticks(
    State(state): State<AppState>,
) -> Result<Json<VideoTicksResponse>, ErrorResponse> {
    let read = read_tick_samples(&video_root(&state).await?.join(TICKS_FILE))
        .map_err(|err| ErrorResponse::internal(format!("failed to read the video clock: {err}")))?;
    Ok(Json(VideoTicksResponse {
        samples: read.samples,
        skipped: read.skipped,
    }))
}

/// Serves the recording's bytes, with range support.
///
/// **`Cache-Control: no-cache`, deliberately unlike a frame.** A frame is
/// immutable once written -- a tick never recurs -- but
/// `<workspace>/video/video.mp4` is *overwritten by the next run*, at the same
/// URL. Serving it `immutable` would show a browser the previous run's
/// recording beside this run's timeline, and nothing would say so. The archived
/// route (`/api/v1/runs/{id}/video/file`) is the one whose bytes never change,
/// and it is cached accordingly.
#[utoipa::path(
    get,
    path = "/api/v1/video/file",
    tag = "Admin",
    responses(
        (status = 200, content_type = "video/mp4", description = "the whole recording"),
        (status = 206, content_type = "video/mp4", description = "a byte range, for seeking"),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_video_file(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, ErrorResponse> {
    let root = video_root(&state).await?;
    let manifest = read_video_dir(&root);
    let name = manifest
        .video
        .as_ref()
        .map(|record| record.file.clone())
        .ok_or_else(|| ErrorResponse::not_found("no video was recorded".to_string()))?;
    serve_video_file(&root, &name, request, CacheFor::ThisRunOnly).await
}

/// How long the bytes at this URL stay the bytes at this URL.
pub(crate) enum CacheFor {
    /// The live recording, overwritten by the next run.
    ThisRunOnly,
    /// An archived recording: its run is over, so the bytes never change.
    Ever,
}

/// The shared body of both file routes.
///
/// The traversal guard runs against the *canonicalised* directory and **before**
/// `ServeFile` exists, so no path the guard would reject is ever handed to it.
pub(crate) async fn serve_video_file(
    dir: &Path,
    name: &str,
    request: Request,
    cache: CacheFor,
) -> Result<Response, ErrorResponse> {
    let root = std::fs::canonicalize(dir)
        .map_err(|_| ErrorResponse::not_found("no video was recorded".to_string()))?;
    let resolved = resolve_script_path(&root, name).map_err(ErrorResponse::from)?;
    if !resolved.is_file() {
        return Err(ErrorResponse::not_found(format!("no such video: {name}")));
    }

    let mut response = ServeFile::new(&resolved)
        .oneshot(request)
        .await
        .map_err(|err| ErrorResponse::internal(format!("failed to serve video: {err}")))?
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        match cache {
            CacheFor::ThisRunOnly => HeaderValue::from_static("no-cache"),
            CacheFor::Ever => HeaderValue::from_static("public, max-age=31536000, immutable"),
        },
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The response is a straight forwarding of core's parser -- there is no
    /// second reader of this format on the server side, deliberately: two
    /// parsers for one file is how two halves of a system come to disagree
    /// about what a sample is.
    fn parse_ticks(text: &str) -> VideoTicksResponse {
        let read = parse_tick_samples(text);
        VideoTicksResponse {
            samples: read.samples,
            skipped: read.skipped,
        }
    }

    #[test]
    fn a_torn_clock_reports_what_it_dropped() {
        let parsed = parse_ticks("{\"t\":1,\"w\":0}\n{\"t\":2,\"w\"");
        assert_eq!(parsed.samples.len(), 1);
        assert_eq!(parsed.skipped, 1);
    }

    /// Gap lines are *in* the response. Filtering them would leave the viewer
    /// with two ordinary samples straddling a stall and no way to know it
    /// happened -- which is the one thing this whole clock exists to prevent.
    #[test]
    fn gap_lines_reach_the_client() {
        let parsed = parse_ticks(
            "{\"t\":1,\"w\":0}\n{\"w\":5000,\"k\":\"gap\",\"reason\":\"stalled\"}\n{\"t\":2,\"w\":9000}\n",
        );
        assert_eq!(parsed.samples.len(), 3);
        assert_eq!(parsed.samples[1].tick, None);
        assert_eq!(
            parsed.samples[1].kind,
            factorio_bot_core::record::video::TickKind::Gap
        );
    }
}
