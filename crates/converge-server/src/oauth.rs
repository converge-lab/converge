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
use converge_storage::{DeviceClaim, Devices, NewDeviceGrant, StoreError, Tokens, UserId};
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use crate::auth::{self, ACCESS_TTL, Sessions};

/// Authorization codes: one browser redirect long.
const CODE_TTL: Duration = Duration::minutes(10);

/// Registered clients: effectively forever (re-registering is cheap).
const CLIENT_TTL: Duration = Duration::days(3650);

/// Device grants: long enough to walk to a browser (RFC 8628 §3.2).
const DEVICE_TTL: Duration = Duration::minutes(15);

/// The minimum seconds between device polls (RFC 8628 `interval`).
pub const DEVICE_POLL_INTERVAL: i64 = 5;

/// The device grant's `grant_type` URN (RFC 8628 §3.4).
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// User-code alphabet (RFC 8628 §6.1): no vowels, no look-alikes —
/// mistype-resistant and never spells anything. 20^8 codes.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

/// A registration request (RFC 7591) — the fields we honor.
#[derive(Deserialize)]
pub struct Registration {
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    /// RFC 7591 `grant_types`; absent means `authorization_code`. A
    /// device-only client is the one shape that needs no redirect URIs.
    #[serde(default)]
    pub grant_types: Vec<String>,
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
        let device_only = !registration.grant_types.is_empty()
            && registration
                .grant_types
                .iter()
                .all(|g| g == DEVICE_GRANT || g == "refresh_token");
        if registration.redirect_uris.is_empty() && !device_only {
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

    /// Open a device grant (RFC 8628 §3.1–3.2): the client gets the
    /// high-entropy `device_code` to poll with and the short `user_code`
    /// a signed-in browser approves. The HTTP layer adds the
    /// verification URIs (they need the public base URL).
    pub async fn device_start<S: Devices>(
        &self,
        store: &S,
        client_id: &str,
    ) -> Result<DeviceAuth, Refused> {
        let client = self.client(client_id).ok_or_else(|| {
            Refused(
                "invalid_client",
                "unknown client_id (register first)".into(),
            )
        })?;
        let device_code = auth::mint();
        let expires_at = OffsetDateTime::now_utc() + DEVICE_TTL;
        // A colliding user code (20^-8 per pending grant) is regenerated.
        for _ in 0..3 {
            let user_code = user_code();
            let new = NewDeviceGrant {
                device_hash: auth::hash(&device_code),
                client_hash: hex(&Sha256::digest(client_id.as_bytes())),
                user_code: user_code.clone(),
                client_name: client.name.clone(),
                expires_at,
            };
            match store.device_start(new).await {
                Ok(()) => {
                    return Ok(DeviceAuth {
                        device_code,
                        user_code,
                        expires_in: DEVICE_TTL.whole_seconds(),
                        interval: DEVICE_POLL_INTERVAL,
                    });
                }
                Err(StoreError::Conflict(_)) => continue,
                Err(e) => return Err(unavailable(e)),
            }
        }
        Err(Refused(
            "temporarily_unavailable",
            "could not allocate a user code".into(),
        ))
    }

    /// The device grant's token-endpoint arm (RFC 8628 §3.4–3.5): poll
    /// until the browser decides, then issue the same access + revocable
    /// refresh pair as `authorization_code`.
    pub async fn device_poll<S: Devices + Tokens>(
        &self,
        store: &S,
        device_code: &str,
        client_id: &str,
    ) -> Result<Grant, Refused> {
        let claim = store
            .device_claim(
                &auth::hash(device_code),
                &hex(&Sha256::digest(client_id.as_bytes())),
            )
            .await
            .map_err(unavailable)?;
        match claim {
            DeviceClaim::Pending => Err(Refused(
                "authorization_pending",
                "the user has not decided yet".into(),
            )),
            DeviceClaim::Denied => Err(Refused(
                "access_denied",
                "the user denied the request".into(),
            )),
            DeviceClaim::Gone => Err(Refused(
                "expired_token",
                "unknown or expired device_code".into(),
            )),
            DeviceClaim::Approved(user) => {
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
        }
    }
}

/// `POST /oauth/device_authorization` success, minus the verification
/// URIs the HTTP layer attaches.
pub struct DeviceAuth {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: i64,
}

/// A fresh canonical user code: `XXXX-XXXX` over the reduced alphabet.
fn user_code() -> String {
    let mut rng = rand::rng();
    let mut code: Vec<u8> = (0..8)
        .map(|_| USER_CODE_ALPHABET[rng.random_range(0..USER_CODE_ALPHABET.len())])
        .collect();
    code.insert(4, b'-');
    String::from_utf8(code).expect("the alphabet is ASCII")
}

/// Normalize a typed user code to the canonical `XXXX-XXXX`: uppercase,
/// separators dropped and re-inserted — forgiving of lowercase and
/// hyphenless paste. Codes of unexpected length pass through (they
/// simply won't match anything).
pub fn normalize_user_code(input: &str) -> String {
    let chars: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    match chars.len() {
        8 => format!("{}-{}", &chars[..4], &chars[4..]),
        _ => chars,
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
                grant_types: vec![],
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
                    grant_types: vec![],
                })
                .is_err()
        );
        // …except for a device-only client, which never redirects.
        assert!(
            oauth()
                .register(&Registration {
                    redirect_uris: vec![],
                    client_name: Some("converge-cli".into()),
                    grant_types: vec![DEVICE_GRANT.into(), "refresh_token".into()],
                })
                .is_ok()
        );
        // Mixed grants still redirect somewhere — URIs stay required.
        assert!(
            oauth()
                .register(&Registration {
                    redirect_uris: vec![],
                    client_name: None,
                    grant_types: vec!["authorization_code".into(), DEVICE_GRANT.into()],
                })
                .is_err()
        );
    }

    #[test]
    fn user_codes_normalize_to_canonical() {
        assert_eq!(normalize_user_code("bcdf-ghjk"), "BCDF-GHJK");
        assert_eq!(normalize_user_code("bcdfghjk"), "BCDF-GHJK");
        assert_eq!(normalize_user_code(" BCDF GHJK "), "BCDF-GHJK");
        assert_eq!(normalize_user_code("short"), "SHORT");
        let code = user_code();
        assert_eq!(normalize_user_code(&code), code);
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
