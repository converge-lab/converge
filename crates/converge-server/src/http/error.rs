//! The `StoreError` → HTTP mapping, shared by every resource.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use converge_storage::StoreError;
use serde_json::json;
use tracing::error;

/// Handler result — `?` on storage calls maps errors uniformly.
pub type Result<T> = std::result::Result<T, Error>;

/// Newtype over the domain error so it can implement [`IntoResponse`].
pub struct Error(StoreError);

impl From<StoreError> for Error {
    fn from(e: StoreError) -> Self {
        Self(e)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self.0 {
            StoreError::NotFound => (StatusCode::NOT_FOUND, "not_found", self.0.to_string()),
            StoreError::Invalid(m) => (StatusCode::BAD_REQUEST, "invalid", m.clone()),
            StoreError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            // 5xx: log the detail, answer generically — backend internals
            // (connection strings, SQL) don't belong in responses.
            StoreError::Unavailable(_) => {
                error!(error = %self.0, "storage unavailable");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "unavailable",
                    "storage unavailable".into(),
                )
            }
            StoreError::Backend(_) => {
                error!(error = %self.0, "storage failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".into(),
                )
            }
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}
