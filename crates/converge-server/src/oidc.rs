//! Identity-provider sign-in: **generic OIDC**, with GitHub as the one
//! built-in adapter.
//!
//! Generic means any issuer with standard discovery — Keycloak, Authentik,
//! Dex, GitLab, Forgejo — resolved lazily from
//! `{issuer}/.well-known/openid-configuration` and cached for the process
//! lifetime (lazy so the server boots even when the IdP is briefly down —
//! closed contours have boot-order problems). GitHub speaks OAuth2 but not
//! OIDC (no discovery, no `id_token`), so its endpoints and identity
//! mapping are built in.
//!
//! The flow is authorization-code + PKCE, with a double-submit `state`
//! cookie against CSRF. Identity comes from the **userinfo** endpoint over
//! TLS rather than `id_token` validation — the token response is already
//! authenticated by the code exchange, and skipping JWT validation means
//! no JWKS machinery. The whole module is optional: without `[auth.oidc]`
//! config nothing here runs and the server needs no egress.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use converge_storage::Identity;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

/// `[auth.oidc]` — the operator's provider description.
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    /// Identity namespace and adapter selector: `github`, or any name you
    /// choose for a generic issuer (`keycloak`, `corp`, …). Stored into
    /// each user's `provider` — pick once and keep it.
    pub provider: String,
    /// Issuer URL for discovery. Required unless `provider = "github"`.
    #[serde(default)]
    pub issuer: Option<String>,
    pub client_id: String,
    pub client_secret: String,
    /// This deployment's external origin; the redirect URI is
    /// `{public_url}/auth/callback` — register it with the provider.
    pub public_url: String,
    /// Handles allowed to sign in. Absent → every identity the provider
    /// asserts is welcome (gate at the IdP); present → only these.
    #[serde(default)]
    pub allowed: Option<Vec<String>>,
}

/// A ready sign-in provider (config + HTTP client + endpoint cache).
pub struct Oidc {
    settings: Settings,
    http: reqwest::Client,
    endpoints: OnceCell<Endpoints>,
}

/// The three endpoints a code flow needs.
#[derive(Debug, Clone, Deserialize)]
struct Endpoints {
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
}

/// The per-attempt secrets that must survive the IdP round trip (in an
/// HttpOnly cookie): the CSRF `state` and the PKCE verifier.
#[derive(Debug)]
pub struct Flow {
    pub state: String,
    pub verifier: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// GitHub's `/user` shape (the adapter's userinfo).
#[derive(Deserialize)]
struct GithubUser {
    id: u64,
    login: String,
    name: Option<String>,
}

/// Standard OIDC userinfo claims (the ones identity mapping needs).
#[derive(Deserialize)]
struct Userinfo {
    sub: String,
    preferred_username: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

impl Oidc {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            // GitHub's API refuses requests without a User-Agent.
            http: reqwest::Client::builder()
                .user_agent("converge-server")
                .build()
                .expect("reqwest client construction cannot fail"),
            endpoints: OnceCell::new(),
        }
    }

    /// Display name for the login button ("Sign in with …").
    pub fn label(&self) -> String {
        match self.settings.provider.as_str() {
            "github" => "GitHub".into(),
            other => other.to_string(),
        }
    }

    /// Is this handle welcome? Absent allowlist delegates to the IdP.
    pub fn allowed(&self, handle: &str) -> bool {
        match &self.settings.allowed {
            None => true,
            Some(allowed) => allowed.iter().any(|a| a == handle),
        }
    }

    fn redirect_uri(&self) -> String {
        format!(
            "{}/auth/callback",
            self.settings.public_url.trim_end_matches('/')
        )
    }

    async fn endpoints(&self) -> Result<&Endpoints, String> {
        self.endpoints
            .get_or_try_init(|| async {
                match (self.settings.provider.as_str(), &self.settings.issuer) {
                    ("github", _) => Ok(Endpoints {
                        authorization_endpoint: "https://github.com/login/oauth/authorize".into(),
                        token_endpoint: "https://github.com/login/oauth/access_token".into(),
                        userinfo_endpoint: "https://api.github.com/user".into(),
                    }),
                    (_, Some(issuer)) => {
                        let url = format!(
                            "{}/.well-known/openid-configuration",
                            issuer.trim_end_matches('/')
                        );
                        self.http
                            .get(&url)
                            .send()
                            .await
                            .map_err(|e| format!("issuer discovery: {e}"))?
                            .error_for_status()
                            .map_err(|e| format!("issuer discovery: {e}"))?
                            .json()
                            .await
                            .map_err(|e| format!("issuer discovery: malformed document: {e}"))
                    }
                    (provider, None) => Err(format!(
                        "auth.oidc.issuer is required for provider `{provider}`"
                    )),
                }
            })
            .await
    }

    /// Begin a sign-in: the authorization URL to redirect to, plus the
    /// per-attempt secrets to stash in the flow cookie.
    pub async fn authorize(&self) -> Result<(String, Flow), String> {
        let endpoints = self.endpoints().await?;
        let flow = Flow {
            state: random(),
            verifier: random(),
        };
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(flow.verifier.as_bytes()));
        let scope = match self.settings.provider.as_str() {
            "github" => "read:user",
            _ => "openid profile email",
        };
        let url = url::Url::parse_with_params(
            &endpoints.authorization_endpoint,
            &[
                ("response_type", "code"),
                ("client_id", &self.settings.client_id),
                ("redirect_uri", &self.redirect_uri()),
                ("scope", scope),
                ("state", &flow.state),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
            ],
        )
        .map_err(|e| format!("authorization endpoint is not a URL: {e}"))?;
        Ok((url.into(), flow))
    }

    /// Finish a sign-in: exchange the code, read userinfo, map to an
    /// [`Identity`] under this provider's namespace.
    pub async fn exchange(&self, code: &str, verifier: &str) -> Result<Identity, String> {
        let endpoints = self.endpoints().await?;
        let token: TokenResponse = self
            .http
            .post(&endpoints.token_endpoint)
            // GitHub answers form-encoded unless asked for JSON.
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri()),
                ("client_id", &self.settings.client_id),
                ("client_secret", &self.settings.client_secret),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|e| format!("code exchange: {e}"))?
            .error_for_status()
            .map_err(|e| format!("code exchange: {e}"))?
            .json()
            .await
            .map_err(|e| format!("code exchange: malformed response: {e}"))?;

        let userinfo = self
            .http
            .get(&endpoints.userinfo_endpoint)
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(|e| format!("userinfo: {e}"))?
            .error_for_status()
            .map_err(|e| format!("userinfo: {e}"))?;

        let provider = self.settings.provider.clone();
        if provider == "github" {
            let user: GithubUser = userinfo
                .json()
                .await
                .map_err(|e| format!("userinfo: malformed response: {e}"))?;
            Ok(Identity {
                provider,
                subject: user.id.to_string(),
                name: user.name.unwrap_or_else(|| user.login.clone()),
                handle: user.login,
            })
        } else {
            let user: Userinfo = userinfo
                .json()
                .await
                .map_err(|e| format!("userinfo: malformed response: {e}"))?;
            let handle = user
                .preferred_username
                .or_else(|| {
                    user.email
                        .as_deref()
                        .and_then(|e| e.split('@').next())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| user.sub.clone());
            Ok(Identity {
                provider,
                subject: user.sub,
                name: user.name.unwrap_or_else(|| handle.clone()),
                handle,
            })
        }
    }
}

/// 128 bits of hex — state and verifier material.
fn random() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oidc(provider: &str, allowed: Option<Vec<String>>) -> Oidc {
        Oidc::new(Settings {
            provider: provider.into(),
            issuer: None,
            client_id: "cid".into(),
            client_secret: "secret".into(),
            public_url: "https://converge.example.com/".into(),
            allowed,
        })
    }

    #[tokio::test]
    async fn github_authorize_url_carries_the_flow() {
        let (url, flow) = oidc("github", None).authorize().await.unwrap();
        assert!(url.starts_with("https://github.com/login/oauth/authorize?"));
        assert!(url.contains(&format!("state={}", flow.state)));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fconverge.example.com%2Fauth%2Fcallback"));
        assert!(!url.contains(&flow.verifier), "verifier must never leave");
    }

    #[tokio::test]
    async fn generic_without_issuer_is_a_config_error() {
        let err = oidc("keycloak", None).authorize().await.unwrap_err();
        assert!(err.contains("issuer is required"), "{err}");
    }

    #[test]
    fn allowlist_gates_and_absence_delegates() {
        let open = oidc("github", None);
        assert!(open.allowed("anyone"));
        let gated = oidc("github", Some(vec!["singulared".into()]));
        assert!(gated.allowed("singulared"));
        assert!(!gated.allowed("intruder"));
        assert_eq!(gated.label(), "GitHub");
        assert_eq!(oidc("keycloak", None).label(), "keycloak");
    }
}
