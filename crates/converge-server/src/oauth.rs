//! The OAuth 2.1 authorization server for MCP connectors (claude.ai and
//! any OAuth-capable MCP client) — the flow-capable client class from the
//! two-credential-families design.
//!
//! Ported from prod's shape (RFC 8414 metadata, RFC 7591 dynamic client
//! registration, authorization-code + mandatory PKCE), with two deliberate
//! divergences:
//!
//! - **Clients and codes are stateless**: signed JWTs (this deployment's
//!   session key), not table rows. A registered `client_id` *is* its own
//!   record — nothing to store, nothing to sweep. Codes are single-flight
//!   by expiry + PKCE rather than by a server-side used-bit: replaying a
//!   code needs the verifier, which only ever travels alongside it over
//!   TLS. Because clients must survive restarts, connectors require a
//!   configured `auth.session_secret` (a random per-boot key would orphan
//!   every registered client on restart).
//! - **Refresh tokens are opaque and revocable**: ordinary `tokens`-table
//!   rows labeled `connector:<name>` — they show up in the settings UI
//!   like any other token, and revoking one cuts the connector off at its
//!   next refresh. Access tokens are 1-hour JWTs verified by the same
//!   middleware as session cookies.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use converge_storage::{StoreError, Tokens, UserId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use crate::auth::{self, ACCESS_TTL, Sessions};

/// Authorization codes: one browser redirect long.
const CODE_TTL: Duration = Duration::minutes(10);

/// Registered clients: effectively forever (re-registering is cheap).
const CLIENT_TTL: Duration = Duration::days(3650);

/// A registration request (RFC 7591) — the fields we honor.
#[derive(Deserialize)]
pub struct Registration {
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub client_name: Option<String>,
}

/// What a `client_id` decodes to: the registration, signed.
#[derive(Serialize, Deserialize)]
struct Client {
    typ: String,
    redirect_uris: Vec<String>,
    name: String,
    exp: i64,
}

/// What a `code` decodes to: who approved what, PKCE-bound.
#[derive(Serialize, Deserialize)]
struct Code {
    typ: String,
    sub: String,
    /// SHA-256 of the `client_id` string — binds the code to one client
    /// without nesting the whole client JWT.
    client: String,
    redirect_uri: String,
    challenge: String,
    exp: i64,
}

/// `POST /oauth/token` success (RFC 6749 §5.1).
#[derive(Serialize)]
pub struct Grant {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// A token-endpoint failure (RFC 6749 §5.2): the `error` code plus a
/// human-readable line.
pub struct Refused(pub &'static str, pub String);

/// The authorization server: stateless over the session signer.
#[derive(Clone)]
pub struct Oauth {
    sessions: Sessions,
}

impl Oauth {
    pub fn new(sessions: Sessions) -> Self {
        Self { sessions }
    }

    /// Register a client: the signed registration *is* the `client_id`.
    pub fn register(&self, registration: &Registration) -> Result<String, Refused> {
        if registration.redirect_uris.is_empty() {
            return Err(Refused(
                "invalid_client_metadata",
                "redirect_uris is required".into(),
            ));
        }
        Ok(self.sessions.sign(&Client {
            typ: "r".into(),
            redirect_uris: registration.redirect_uris.clone(),
            name: registration
                .client_name
                .clone()
                .unwrap_or_else(|| "connector".into()),
            exp: (OffsetDateTime::now_utc() + CLIENT_TTL).unix_timestamp(),
        }))
    }

    fn client(&self, client_id: &str) -> Option<Client> {
        let client: Client = self.sessions.open(client_id)?;
        (client.typ == "r").then_some(client)
    }

    /// Validate an authorization request and mint the code for `user`
    /// (the already-authenticated browser). Returns the full redirect URL.
    pub fn authorize(
        &self,
        client_id: &str,
        redirect_uri: &str,
        challenge: &str,
        state: Option<&str>,
        user: UserId,
    ) -> Result<String, String> {
        let client = self
            .client(client_id)
            .ok_or("unknown client_id (register first)")?;
        if !client.redirect_uris.iter().any(|u| u == redirect_uri) {
            return Err("redirect_uri is not registered for this client".into());
        }
        // RFC 7636: 43–128 chars of base64url material.
        let ok = (43..=128).contains(&challenge.len())
            && challenge
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err("a valid S256 code_challenge is required".into());
        }
        let code = self.sessions.sign(&Code {
            typ: "c".into(),
            sub: user.to_string(),
            client: hex(&Sha256::digest(client_id.as_bytes())),
            redirect_uri: redirect_uri.into(),
            challenge: challenge.into(),
            exp: (OffsetDateTime::now_utc() + CODE_TTL).unix_timestamp(),
        });
        let mut url = format!("{redirect_uri}?code={code}");
        if let Some(state) = state {
            url.push_str("&state=");
            url.push_str(&query_encode(state));
        }
        Ok(url)
    }

    /// `authorization_code` grant: verify the code + PKCE, issue an access
    /// JWT and a revocable refresh token.
    pub async fn exchange<S: Tokens>(
        &self,
        store: &S,
        code: &str,
        verifier: &str,
        client_id: &str,
        redirect_uri: &str,
    ) -> Result<Grant, Refused> {
        let invalid = |m: &str| Refused("invalid_grant", m.into());
        let opened: Code = self
            .sessions
            .open(code)
            .ok_or_else(|| invalid("invalid or expired code"))?;
        if opened.typ != "c" {
            return Err(invalid("invalid or expired code"));
        }
        if opened.client != hex(&Sha256::digest(client_id.as_bytes())) {
            return Err(invalid("the code belongs to a different client"));
        }
        if opened.redirect_uri != redirect_uri {
            return Err(invalid("redirect_uri does not match the authorization"));
        }
        if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != opened.challenge {
            return Err(invalid("PKCE verification failed"));
        }
        let user: UserId = opened
            .sub
            .parse()
            .map_err(|_| invalid("invalid or expired code"))?;

        let name = self
            .client(client_id)
            .map(|c| c.name)
            .unwrap_or_else(|| "connector".into());
        let refresh = auth::mint();
        store
            .token_add(user, format!("connector:{name}"), auth::hash(&refresh))
            .await
            .map_err(unavailable)?;
        Ok(Grant {
            access_token: self.sessions.access(user),
            token_type: "bearer",
            expires_in: ACCESS_TTL.whole_seconds(),
            refresh_token: Some(refresh),
        })
    }

    /// `refresh_token` grant: the opaque token resolves through the same
    /// lookup as any bearer; revoking it (settings UI, `token revoke`)
    /// ends the connector's line of credit.
    pub async fn refresh<S: Tokens>(&self, store: &S, refresh: &str) -> Result<Grant, Refused> {
        let user = store
            .token_user(&auth::hash(refresh))
            .await
            .map_err(unavailable)?
            .ok_or_else(|| Refused("invalid_grant", "unknown or revoked refresh_token".into()))?;
        Ok(Grant {
            access_token: self.sessions.access(user),
            token_type: "bearer",
            expires_in: ACCESS_TTL.whole_seconds(),
            refresh_token: None,
        })
    }
}

fn unavailable(e: StoreError) -> Refused {
    tracing::error!(error = %e, "storage failure in the token endpoint");
    Refused("temporarily_unavailable", "storage unavailable".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Query-component percent-encoding (the `state` echo, the `next` param).
pub fn query_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth() -> Oauth {
        Oauth::new(Sessions::new(Some("test")))
    }

    fn registered(oauth: &Oauth) -> String {
        oauth
            .register(&Registration {
                redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".into()],
                client_name: Some("claude.ai".into()),
            })
            .unwrap_or_else(|e| panic!("{}", e.1))
    }

    #[test]
    fn registration_requires_redirect_uris() {
        assert!(
            oauth()
                .register(&Registration {
                    redirect_uris: vec![],
                    client_name: None,
                })
                .is_err()
        );
    }

    #[test]
    fn authorize_binds_client_uri_and_challenge() {
        let oauth = oauth();
        let client = registered(&oauth);
        let user = UserId::new();
        let challenge = "a".repeat(43);

        let url = oauth
            .authorize(
                &client,
                "https://claude.ai/api/mcp/auth_callback",
                &challenge,
                Some("st/ate"),
                user,
            )
            .unwrap();
        assert!(url.starts_with("https://claude.ai/api/mcp/auth_callback?code="));
        assert!(url.ends_with("&state=st%2Fate"));

        // Unregistered redirect target, bad challenge, bogus client: no code.
        assert!(
            oauth
                .authorize(&client, "https://evil.example/cb", &challenge, None, user)
                .is_err()
        );
        assert!(
            oauth
                .authorize(
                    &client,
                    "https://claude.ai/api/mcp/auth_callback",
                    "short",
                    None,
                    user
                )
                .is_err()
        );
        assert!(
            oauth
                .authorize(
                    "not-a-client",
                    "https://claude.ai/api/mcp/auth_callback",
                    &challenge,
                    None,
                    user
                )
                .is_err()
        );
    }
}
