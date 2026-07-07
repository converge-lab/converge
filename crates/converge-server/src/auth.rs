//! Authentication — **always on**, no fallback caller.
//!
//! Two credential families resolve to the same [`Caller`]:
//!
//! - **Opaque bearer tokens** (`cvg_<64 hex>`) for agents, the CLI, and
//!   anything header-configured: long-lived, listable, revocable. Storage
//!   holds only their SHA-256 (high-entropy secrets need no salt — the
//!   GitHub construction). Minted by `converge-server token mint` on the
//!   host — never written to logs, where collectors would keep them.
//! - **Session JWTs** for browsers: short-lived, carried in an `HttpOnly`
//!   cookie so the secret never touches JavaScript, signed with
//!   [`Sessions`]' key and self-expiring — the ephemeral sibling of the
//!   opaque tokens (expiry stands in for revocation).
//!
//! GitHub OIDC (the team path) and the MCP OAuth server land in the next
//! slices; `healthz`, the session endpoint, and the static web assets are
//! the only open paths (the app must load to show its login screen).

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use converge_storage::{Identity, Pagination, Storage, StoreError, UserId};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

/// The session cookie's name.
pub const COOKIE: &str = "converge_session";

/// How long a browser session lives. Expiry is the session's *only* end
/// of life (no server-side revocation list) — keep it short-ish.
pub const SESSION_TTL: Duration = Duration::days(7);

/// The authenticated principal, injected request-wide by [`require`].
#[derive(Debug, Clone, Copy)]
pub struct Caller {
    pub user: UserId,
}

/// Signs and verifies browser-session JWTs (HS256).
#[derive(Clone)]
pub struct Sessions {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    /// The user id (ULID string).
    sub: String,
    /// Expiry, seconds since epoch (verified by the JWT library).
    exp: i64,
}

impl Sessions {
    /// A signer from the configured secret. `None` generates a random
    /// per-boot key: everything works, but sessions reset on restart —
    /// set `[auth] session_secret` to persist them across deploys.
    pub fn new(secret: Option<&str>) -> Self {
        let key = match secret {
            Some(secret) => secret.as_bytes().to_vec(),
            None => {
                tracing::info!(
                    "auth.session_secret is not set — browser sessions will reset on restart"
                );
                let mut bytes = vec![0u8; 32];
                rand::rng().fill_bytes(&mut bytes);
                bytes
            }
        };
        Self {
            encoding: EncodingKey::from_secret(&key),
            decoding: DecodingKey::from_secret(&key),
        }
    }

    /// Issue a session JWT for `user`, expiring in [`SESSION_TTL`].
    pub fn issue(&self, user: UserId) -> String {
        let claims = Claims {
            sub: user.to_string(),
            exp: (OffsetDateTime::now_utc() + SESSION_TTL).unix_timestamp(),
        };
        jsonwebtoken::encode(&Header::default(), &claims, &self.encoding)
            .expect("HS256 encoding of plain claims cannot fail")
    }

    /// Verify a session JWT (signature + expiry) down to its user.
    pub fn verify(&self, jwt: &str) -> Option<UserId> {
        let data =
            jsonwebtoken::decode::<Claims>(jwt, &self.decoding, &Validation::default()).ok()?;
        data.claims.sub.parse().ok()
    }
}

/// The session cookie, ready to set: `HttpOnly` (never visible to JS),
/// `SameSite=Strict`, whole-site path, living [`SESSION_TTL`]. Shared by
/// the token exchange and the OIDC callback so the two entrances can't
/// drift.
pub fn cookie(jwt: String) -> axum_extra::extract::cookie::Cookie<'static> {
    use axum_extra::extract::cookie::{Cookie, SameSite};
    Cookie::build((COOKIE, jwt))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(SESSION_TTL)
        .build()
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

/// Middleware: everything behind it requires a credential — a bearer
/// token (agents, CLI) or the session cookie (browsers); the resolved
/// [`Caller`] rides the request extensions. Bearer wins when both are
/// present (it's the more explicit assertion).
pub async fn require<S: Storage>(
    State((store, sessions)): State<(S, Sessions)>,
    mut request: Request,
    next: Next,
) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let user = match bearer {
        Some(secret) => match store.token_user(&hash(secret)).await {
            Ok(user) => user,
            Err(e) => {
                tracing::error!(error = %e, "authentication lookup failed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": { "code": "unavailable", "message": "storage unavailable" }
                    })),
                )
                    .into_response();
            }
        },
        None => CookieJar::from_headers(request.headers())
            .get(COOKIE)
            .and_then(|cookie| sessions.verify(cookie.value())),
    };
    match user {
        Some(user) => {
            request.extensions_mut().insert(Caller { user });
            next.run(request).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": { "code": "unauthorized", "message": "authentication required" }
            })),
        )
            .into_response(),
    }
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

    #[test]
    fn sessions_round_trip_and_reject_foreign_signatures() {
        let user = UserId::new();
        let sessions = Sessions::new(Some("secret"));
        assert_eq!(sessions.verify(&sessions.issue(user)), Some(user));
        // A different key (or a random per-boot key) verifies nothing.
        let other = Sessions::new(Some("other"));
        assert_eq!(other.verify(&sessions.issue(user)), None);
        assert_eq!(Sessions::new(None).verify(&sessions.issue(user)), None);
        assert_eq!(sessions.verify("not-a-jwt"), None);
    }
}
