//! Bearer authentication — **always on**, no fallback caller.
//!
//! Tokens are `cvg_<64 hex>` secrets; storage holds only their SHA-256
//! (high-entropy random secrets need no salt — the GitHub construction).
//! The bootstrap admin token is minted on first boot and logged **once** —
//! the Jupyter/Grafana pattern: one copy-paste to get in, no provider
//! setup. Real providers (GitHub OIDC) and browser sessions layer on in
//! the next slices; `healthz` and the static web assets are the only open
//! paths (the app must load to show a login screen).

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use converge_storage::{Identity, Pagination, Storage, StoreError, UserId};
use rand::RngCore;
use serde_json::json;
use sha2::{Digest, Sha256};

/// The authenticated principal, injected request-wide by [`require`].
#[derive(Debug, Clone, Copy)]
pub struct Caller {
    pub user: UserId,
}

/// The stored form of a token secret.
pub fn hash(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// Mint a fresh token secret (256 bits, `cvg_`-prefixed for greppability).
pub fn mint() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("cvg_{}", hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Boot-time setup: log the deployment user in, and when they hold no
/// token yet, say how to mint one. Secrets are **never** written to the
/// service log — logs get shipped to collectors and retained; minting
/// happens through `converge-server token mint`, which prints to the
/// operator's terminal (host access is the trust boundary).
pub async fn hint<S: Storage>(store: &S, me: Identity) -> Result<(), StoreError> {
    let user = store.user_login(me).await?;
    let existing = store
        .token_list(
            user,
            Pagination {
                limit: Some(1),
                cursor: None,
            },
        )
        .await?;
    if existing.is_empty() {
        tracing::warn!(
            "no tokens exist for the deployment user — mint one with \
             `converge-server token mint` (docker: `docker compose exec \
             converge converge-server token mint`)"
        );
    }
    Ok(())
}

/// Middleware: everything behind it requires `Authorization: Bearer` with
/// a known token; the resolved [`Caller`] rides the request extensions.
pub async fn require<S: Storage>(
    State(store): State<S>,
    mut request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(secret) = presented else {
        return unauthorized("missing bearer token");
    };
    match store.token_user(&hash(secret)).await {
        Ok(Some(user)) => {
            request.extensions_mut().insert(Caller { user });
            next.run(request).await
        }
        Ok(None) => unauthorized("unknown token"),
        Err(e) => {
            tracing::error!(error = %e, "authentication lookup failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(
                    json!({ "error": { "code": "unavailable", "message": "storage unavailable" } }),
                ),
            )
                .into_response()
        }
    }
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": { "code": "unauthorized", "message": message } })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_prefixed_and_hashes_are_stable() {
        let secret = mint();
        assert!(secret.starts_with("cvg_"));
        assert_eq!(secret.len(), 4 + 64);
        assert_ne!(secret, mint());
        assert_eq!(hash("cvg_x"), hash("cvg_x"));
        assert_ne!(hash("cvg_x"), hash("cvg_y"));
    }
}
