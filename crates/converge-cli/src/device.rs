//! Device pairing (RFC 8628) — `converge init`'s zero-paste path.
//!
//! Discovery-driven: if the server's RFC 8414 metadata offers the device
//! grant, `init` registers an ephemeral device-only client, prints (and
//! opens) the approval link, and polls the token endpoint until the
//! signed-in browser decides. The grant's `refresh_token` **is** the
//! durable credential — server-side it is an ordinary revocable bearer
//! row (`connector:<name>` in the settings UI), so the config file
//! format and every later request are unchanged. Servers without the
//! grant (or piped stdin) keep the paste-a-token flow.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The device grant's `grant_type` URN (RFC 8628 §3.4).
const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 8414 metadata — the fields the flow needs.
#[derive(Deserialize)]
struct Discovery {
    registration_endpoint: Option<String>,
    token_endpoint: Option<String>,
    device_authorization_endpoint: Option<String>,
    #[serde(default)]
    grant_types_supported: Vec<String>,
}

/// A server that offers the device grant: the three endpoints to run it.
pub struct Offer {
    register: String,
    device: String,
    token: String,
}

/// Probe `server`'s authorization-server metadata. `None` (offline, older
/// server, grant not offered) sends the caller to the paste flow.
pub async fn probe(server: &str) -> Option<Offer> {
    let url = format!("{server}/.well-known/oauth-authorization-server");
    let discovery: Discovery = reqwest::get(&url)
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    if !discovery.grant_types_supported.iter().any(|g| g == GRANT) {
        return None;
    }
    Some(Offer {
        register: discovery.registration_endpoint?,
        device: discovery.device_authorization_endpoint?,
        token: discovery.token_endpoint?,
    })
}

#[derive(Deserialize)]
struct Registered {
    client_id: String,
}

/// `POST device_authorization` success (RFC 8628 §3.2).
#[derive(Deserialize)]
struct Opened {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: i64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Deserialize)]
struct Token {
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct Oops {
    error: String,
}

/// Run the pairing dance against an [`Offer`]; returns the credential to
/// store (the grant's refresh token).
pub async fn pair(offer: &Offer) -> Result<String> {
    let http = reqwest::Client::new();
    let registered: Registered = http
        .post(&offer.register)
        .json(&serde_json::json!({
            "client_name": format!("converge-cli @ {}", host()),
            "grant_types": [GRANT, "refresh_token"],
        }))
        .send()
        .await
        .context("register a client")?
        .error_for_status()
        .context("register a client")?
        .json()
        .await
        .context("register a client")?;
    let opened: Opened = http
        .post(&offer.device)
        .form(&[("client_id", registered.client_id.as_str())])
        .send()
        .await
        .context("open the pairing request")?
        .error_for_status()
        .context("open the pairing request")?
        .json()
        .await
        .context("open the pairing request")?;

    let link = opened
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&opened.verification_uri);
    println!("\nsign in via your browser and approve pairing code\n");
    println!("    {}\n", opened.user_code);
    println!("at {link}");
    if browse(link) {
        println!("(opened in your browser)");
    }

    let deadline = Instant::now() + Duration::from_secs(opened.expires_in.max(0) as u64);
    let mut interval = opened.interval.max(1);
    loop {
        if Instant::now() >= deadline {
            bail!("the pairing code expired — run `converge init` again");
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let response = http
            .post(&offer.token)
            .form(&[
                ("grant_type", GRANT),
                ("device_code", &opened.device_code),
                ("client_id", &registered.client_id),
            ])
            .send()
            .await
            .context("poll the token endpoint")?;
        if response.status().is_success() {
            let token: Token = response.json().await.context("read the token grant")?;
            return token
                .refresh_token
                .context("the server issued no refresh_token");
        }
        let oops: Oops = response.json().await.context("read the token refusal")?;
        match oops.error.as_str() {
            "authorization_pending" => continue,
            "slow_down" => interval += 5,
            "access_denied" => bail!("pairing was denied in the browser"),
            "expired_token" => bail!("the pairing code expired — run `converge init` again"),
            other => bail!("the token endpoint refused: {other}"),
        }
    }
}

/// Best-effort browser launch; the printed link is the contract.
fn browse(url: &str) -> bool {
    let opener = match cfg!(target_os = "macos") {
        true => "open",
        false => "xdg-open",
    };
    Command::new(opener)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// This machine's name, for the client label the approval screen (and
/// later the settings token list) shows.
fn host() -> String {
    Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown host".into())
}
