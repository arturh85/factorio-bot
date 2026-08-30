//! Local extractor wrappers that route axum's built-in rejections through
//! [`ErrorResponse`], so a malformed query string or request body answers
//! with the same JSON error shape as every other handler failure instead of
//! axum's default `text/plain` rejection body.
//!
//! This is a thin, local alternative to `axum_extra::extract::WithRejection`:
//! adding `axum-extra` as a dependency for two one-line rejection mappings
//! did not seem worth it, so `ApiQuery`/`ApiJson` delegate to
//! `axum::extract::{Query, Json}` and map the rejection via `ErrorResponse`'s
//! existing `From` impls.
//!
//! Note this collapses status codes: both rejections are mapped through
//! `ErrorResponse::bad_request`, so a rejection that axum would otherwise have
//! answered differently (an oversized body would be 413, a missing/wrong
//! content-type would be 415) answers 400 here. That is a real behavior change
//! from axum's defaults, accepted because a malformed request really is the
//! caller's fault and one status keeps client handling simple. `ErrorResponse`
//! itself does carry a status now (see `error.rs`), so handlers that know
//! better — a missing script, a create over an existing file, a server-side
//! failure — answer 404/409/500 rather than 400.

use crate::error::ErrorResponse;
use axum::extract::{FromRequest, FromRequestParts, Json, Query, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

/// Wraps [`axum::extract::Query`] so a failed query-string extraction
/// answers as JSON.
pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ErrorResponse;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(ErrorResponse::from)?;
        Ok(ApiQuery(value))
    }
}

/// Wraps [`axum::extract::Json`] so a failed body extraction answers as
/// JSON in the same error shape (rather than axum's default `text/plain`).
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ErrorResponse;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(ErrorResponse::from)?;
        Ok(ApiJson(value))
    }
}
