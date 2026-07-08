//! The authorization-server surface for MCP connectors: RFC 8414/9728
//! discovery, RFC 7591 registration, and the authorize/token pair over
//! [`crate::oauth`]. All open paths — this *is* the connectors' entrance.
//!
//! The issuer base is `auth.public_url` when configured, else derived from
//! the request's `Host` (which keeps `xtask dev` + local MCP clients
//! working with zero config). Authorization needs a signed-in browser: an
//! unauthenticated visit bounces to sign-in with `?next=` pointing back
//! here, so the flow resumes after login.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use axum_extra::extract::CookieJar;
use converge_storage::Storage;
use serde::Deserialize;
use serde_json::json;

use crate::auth::{COOKIE, Sessions};
use crate::oauth::{Oauth, Refused, Registration};

/// The state these routes share. `signin` says whether an IdP is
/// configured — it picks the login screen unauthenticated browsers bounce
/// to.
#[derive(Clone)]
pub struct Issuer<S> {
    pub store: S,
    pub sessions: Sessions,
    pub oauth: Oauth,
    pub public: Option<String>,
    pub signin: bool,
}

impl<S> Issuer<S> {
    /// The externally visible origin: configured, or the request's Host.
    fn base(&self, headers: &HeaderMap) -> String {
        match &self.public {
            Some(public) => public.trim_end_matches('/').to_string(),
            None => {
                let host = headers
                    .get(header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("127.0.0.1");
                format!("http://{host}")
            }
        }
    }
}

pub fn routes<S: Storage + 'static>() -> Router<Issuer<S>> {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(server_metadata::<S>),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata::<S>),
        )
        // Path-scoped variant some clients probe for the /mcp resource.
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_metadata::<S>),
        )
        .route("/oauth/register", post(register::<S>))
        .route("/oauth/authorize", get(authorize::<S>))
        .route("/oauth/token", post(token::<S>))
}

async fn server_metadata<S: Storage>(
    State(issuer): State<Issuer<S>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let base = issuer.base(&headers);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

async fn resource_metadata<S: Storage>(
    State(issuer): State<Issuer<S>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let base = issuer.base(&headers);
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
    }))
}

async fn register<S: Storage>(
    State(issuer): State<Issuer<S>>,
    Json(registration): Json<Registration>,
) -> Response {
    match issuer.oauth.register(&registration) {
        Ok(client_id) => (
            StatusCode::CREATED,
            Json(json!({
                "client_id": client_id,
                "client_id_issued_at": time::OffsetDateTime::now_utc().unix_timestamp(),
                "redirect_uris": registration.redirect_uris,
                "client_name": registration.client_name,
                "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
            })),
        )
            .into_response(),
        Err(refused) => refuse(refused),
    }
}

#[derive(Deserialize)]
struct Authorize {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code_challenge: Option<String>,
    #[serde(default)]
    code_challenge_method: Option<String>,
}

async fn authorize<S: Storage>(
    State(issuer): State<Issuer<S>>,
    jar: CookieJar,
    uri: Uri,
    Query(request): Query<Authorize>,
) -> Response {
    // Who is approving? An unauthenticated browser goes to sign in and
    // comes back here (`next` is this exact authorize URL).
    let user = jar
        .get(COOKIE)
        .and_then(|cookie| issuer.sessions.verify(cookie.value()));
    let Some(user) = user else {
        let next = crate::oauth::query_encode(&uri.to_string());
        let login = if issuer.signin {
            format!("/auth/login?next={next}")
        } else {
            format!("/?next={next}")
        };
        return Redirect::to(&login).into_response();
    };

    if request.response_type != "code" {
        return (StatusCode::BAD_REQUEST, "response_type must be `code`").into_response();
    }
    if request.code_challenge_method.as_deref() != Some("S256") {
        return (StatusCode::BAD_REQUEST, "PKCE with S256 is required").into_response();
    }
    match issuer.oauth.authorize(
        &request.client_id,
        &request.redirect_uri,
        request.code_challenge.as_deref().unwrap_or_default(),
        request.state.as_deref(),
        user,
    ) {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn token<S: Storage>(
    State(issuer): State<Issuer<S>>,
    Form(request): Form<TokenRequest>,
) -> Response {
    let granted = match request.grant_type.as_str() {
        "authorization_code" => {
            issuer
                .oauth
                .exchange(
                    &issuer.store,
                    request.code.as_deref().unwrap_or_default(),
                    request.code_verifier.as_deref().unwrap_or_default(),
                    request.client_id.as_deref().unwrap_or_default(),
                    request.redirect_uri.as_deref().unwrap_or_default(),
                )
                .await
        }
        "refresh_token" => {
            issuer
                .oauth
                .refresh(
                    &issuer.store,
                    request.refresh_token.as_deref().unwrap_or_default(),
                )
                .await
        }
        other => Err(Refused(
            "unsupported_grant_type",
            format!("unsupported grant_type `{other}`"),
        )),
    };
    match granted {
        Ok(grant) => Json(grant).into_response(),
        Err(refused) => refuse(refused),
    }
}

/// RFC 6749 §5.2 error body.
fn refuse(Refused(error, description): Refused) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}
