use axum::routing::{MethodRouter, get_service};
use std::path::Path;
use tower_http::services::{ServeDir, ServeFile};

/// The two static services the SPA needs, which differ in one respect: what
/// they do when the file is not there.
pub struct SpaServices {
    /// Mounted at `/assets`, with **no** fallback.
    pub assets: MethodRouter,
    /// Everything else, falling back to `index.html` so client-side routes work.
    pub fallback: MethodRouter,
}

/// Builds the static-file services for the SPA. Returns `None` when no web root
/// is configured or the directory does not exist, in which case the API is
/// served without a frontend.
///
/// # Why `/assets` is separate
///
/// A path-based fallback cannot tell a client-side route from an asset that
/// ought to exist -- both are just paths that matched no file -- so serving
/// `index.html` for everything is the only choice it can make on its own. The
/// alternative, a hardcoded list of client routes, would be a second source of
/// truth against the router and would go stale the first time a route was
/// added.
///
/// `/assets` escapes that bind because the build content-hashes everything it
/// puts there. A request for a hashed name that is absent is never a route; it
/// is a stale `index.html` pointing at a bundle that no longer exists, or a
/// half-finished deploy. So this one prefix can answer honestly, and it needs
/// to: a 200 carrying HTML where JavaScript was asked for makes the browser
/// report a MIME type error, which names neither the file nor the fact that it
/// is missing.
pub fn services(web_root: Option<&str>) -> Option<SpaServices> {
    let root = web_root?;
    let path = Path::new(root);
    if !path.is_dir() {
        tracing::warn!("web root {root} does not exist, serving API only");
        return None;
    }
    let index = path.join("index.html");
    Some(SpaServices {
        assets: get_service(ServeDir::new(path.join("assets"))),
        fallback: get_service(ServeDir::new(path).fallback(ServeFile::new(index))),
    })
}
