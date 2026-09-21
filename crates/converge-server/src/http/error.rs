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
            // Handlers sit behind the auth gate and never see this; kept
            // for completeness (the middleware and session endpoint build
            // their 401s directly).
            StoreError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "unauthorized", self.0.to_string())
            }
            // Backend-specific diagnostics are sanitized at their source.
            // These strings may still come from other implementations, so
            // never log a raw cause or leak it into an HTTP response here.
            StoreError::Unavailable(_) => {
                error!(category = "unavailable", "storage request failed");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "unavailable",
                    "storage unavailable".into(),
                )
            }
            StoreError::Backend(_) => {
                error!(category = "backend", "storage request failed");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn internal_causes_from_other_backends_reach_neither_response_nor_logs() {
        let sensitive = "postgres://user:secret@host/private@example.test";
        for (error, status) in [
            (
                StoreError::Backend(sensitive.into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                StoreError::Unavailable(sensitive.into()),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let bytes = Arc::new(Mutex::new(Vec::new()));
            let capture = Capture(bytes.clone());
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(move || capture.clone())
                .finish();
            let response = tracing::subscriber::with_default(subscriber, || {
                Error::from(error).into_response()
            });
            assert_eq!(response.status(), status);
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let body = std::str::from_utf8(&body).unwrap();
            let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
            assert!(logs.contains("storage request failed"));
            for forbidden in ["postgres://", "secret", "@example.test", "@host"] {
                assert!(!body.contains(forbidden));
                assert!(!logs.contains(forbidden));
            }
        }
    }
}
