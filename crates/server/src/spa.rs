use axum::routing::{MethodRouter, get_service};
use std::path::Path;
use tower_http::services::{ServeDir, ServeFile};

/// Builds a static-file service for the SPA. Returns `None` when no web root is
/// configured or the directory does not exist, in which case the API is served
/// without a frontend.
pub fn service(web_root: Option<&str>) -> Option<MethodRouter> {
    let root = web_root?;
    let path = Path::new(root);
    if !path.is_dir() {
        tracing::warn!("web root {root} does not exist, serving API only");
        return None;
    }
    let index = path.join("index.html");
    Some(get_service(
        ServeDir::new(path).fallback(ServeFile::new(index)),
    ))
}
