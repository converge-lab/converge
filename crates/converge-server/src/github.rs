//! Reading a repository, so an anchor can be checked against it.
//!
//! One client, two ways of proving who we are. A GitHub App signs a
//! short JWT with its private key, trades it for an installation token
//! scoped to one repository, and can later be written to; a personal
//! token reads and nothing more. Everything phase one needs is a read,
//! so both work, and a deployment with neither simply never asks.
//!
//! What this does *not* do is decide anything about an anchor. It
//! fetches a blob at a commit; the comparison lives in [`verdict`],
//! where it can be tested without a network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use converge_storage::{CodeAnchor, Repository, Scope, Storage};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::StatusCode;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::config;

/// How long the App's own JWT is good for. GitHub allows ten minutes
/// and rejects a clock ahead of its own, so this asks for less.
const JWT_TTL: Duration = Duration::from_secs(480);
/// An installation token lasts an hour; it is dropped early so a call
/// is never made with one about to expire.
const TOKEN_TTL: Duration = Duration::from_secs(45 * 60);

enum Auth {
    /// The App: an id and the PEM that signs for it.
    App { id: u64, key: Box<EncodingKey> },
    /// A personal token — reads only, and no webhooks.
    Token(String),
}

pub struct Github {
    http: reqwest::Client,
    api: String,
    auth: Auth,
    /// Installation tokens, per repository, with the moment they were
    /// minted. Cheap to lose: the worst case is minting another.
    tokens: Mutex<HashMap<String, (String, Instant)>>,
}

impl Github {
    /// A client, or `None` when this deployment has nothing to prove
    /// who it is with — which is not an error, just an integration that
    /// is off.
    pub fn new(cfg: &config::Github) -> Option<Self> {
        let auth = match (cfg.app_id, cfg.private_key.as_deref(), cfg.token.as_deref()) {
            (Some(id), Some(key), _) => Auth::App {
                id,
                key: Box::new(signing_key(key)?),
            },
            (_, _, Some(token)) => Auth::Token(token.to_owned()),
            _ => return None,
        };
        Some(Self {
            http: reqwest::Client::builder()
                .user_agent("converge")
                .build()
                .ok()?,
            api: cfg.api.trim_end_matches('/').to_owned(),
            auth,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    /// The file at that commit, as text. `None` means the repository
    /// answered that it is not there — a gone commit, a gone file, or
    /// a path this installation cannot see; the caller records that as
    /// a mismatch rather than an outage.
    pub async fn blob(
        &self,
        owner: &str,
        name: &str,
        commit: &str,
        path: &str,
    ) -> Result<Option<String>, Error> {
        let token = self.token(owner, name).await?;
        let url = format!(
            "{}/repos/{owner}/{name}/contents/{path}?ref={commit}",
            self.api
        );
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            // Raw, so nothing has to be un-base64'd and a large file
            // does not arrive wrapped in JSON.
            .header("Accept", "application/vnd.github.raw+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        match response.status() {
            StatusCode::OK => response
                .text()
                .await
                .map(Some)
                .map_err(|e| Error::Unreachable(e.to_string())),
            StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => Ok(None),
            other => Err(Error::Refused(other.as_u16())),
        }
    }

    /// The bearer for one repository: the token itself when that is all
    /// we have, otherwise an installation token, minted and kept until
    /// it is nearly old.
    async fn token(&self, owner: &str, name: &str) -> Result<String, Error> {
        let (id, key) = match &self.auth {
            Auth::Token(token) => return Ok(token.clone()),
            Auth::App { id, key } => (*id, key),
        };
        let repo = format!("{owner}/{name}");
        if let Ok(cache) = self.tokens.lock()
            && let Some((token, minted)) = cache.get(&repo)
            && minted.elapsed() < TOKEN_TTL
        {
            return Ok(token.clone());
        }
        let jwt = app_jwt(id, key)?;
        let installation = self.installation(&repo, &jwt).await?;
        let minted = self.mint(installation, &jwt).await?;
        if let Ok(mut cache) = self.tokens.lock() {
            cache.insert(repo, (minted.clone(), Instant::now()));
        }
        Ok(minted)
    }

    /// Which installation covers this repository. `Refused(404)` is the
    /// honest answer when the App is not installed on it.
    async fn installation(&self, repo: &str, jwt: &str) -> Result<u64, Error> {
        #[derive(Deserialize)]
        struct Installation {
            id: u64,
        }
        let response = self
            .http
            .get(format!("{}/repos/{repo}/installation", self.api))
            .bearer_auth(jwt)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(Error::Refused(response.status().as_u16()));
        }
        response
            .json::<Installation>()
            .await
            .map(|i| i.id)
            .map_err(|e| Error::Unreachable(e.to_string()))
    }

    async fn mint(&self, installation: u64, jwt: &str) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct Minted {
            token: String,
        }
        let response = self
            .http
            .post(format!(
                "{}/app/installations/{installation}/access_tokens",
                self.api
            ))
            .bearer_auth(jwt)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(Error::Refused(response.status().as_u16()));
        }
        response
            .json::<Minted>()
            .await
            .map(|m| m.token)
            .map_err(|e| Error::Unreachable(e.to_string()))
    }
}

/// What can go wrong on the way to a blob. Neither is a verdict on the
/// anchor: an anchor is only wrong when the repository answers and
/// disagrees.
#[derive(Debug)]
pub enum Error {
    /// The network, or a reply we could not read.
    Unreachable(String),
    /// GitHub answered, and said no.
    Refused(u16),
    /// The key this deployment carries is not a key.
    BadKey,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable(why) => write!(f, "github unreachable: {why}"),
            Error::Refused(code) => write!(f, "github refused with {code}"),
            Error::BadKey => write!(f, "the configured github private key is not a PEM"),
        }
    }
}

/// The key as configured: the PEM itself, or a path to it. A path is
/// what an operator wants on a host; the PEM itself is what an
/// environment variable can carry.
fn signing_key(configured: &str) -> Option<EncodingKey> {
    let pem = if configured.trim_start().starts_with("-----BEGIN") {
        configured.to_owned()
    } else {
        std::fs::read_to_string(configured.trim()).ok()?
    };
    EncodingKey::from_rsa_pem(pem.as_bytes()).ok()
}

/// The App proving it is itself: RS256 over its id, backdated a minute
/// against clock skew, as GitHub's own documentation advises.
fn app_jwt(id: u64, key: &EncodingKey) -> Result<String, Error> {
    #[derive(serde::Serialize)]
    struct Claims {
        iat: u64,
        exp: u64,
        iss: String,
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Error::Unreachable(e.to_string()))?
        .as_secs();
    let claims = Claims {
        iat: now - 60,
        exp: now + JWT_TTL.as_secs(),
        iss: id.to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).map_err(|_| Error::BadKey)
}

/// The cited lines, normalized the way an excerpt is: joined with `\n`
/// and ending with one, whatever the file's own line endings are. This
/// is the convention `CodeAnchor` documents, and the reason a digest
/// computed on a clone matches one computed here.
///
/// `None` when the range runs past the end of the file — a citation
/// that no longer fits is a disagreement, not an error.
pub fn cited(blob: &str, lines: (u32, u32)) -> Option<String> {
    let (start, end) = lines;
    if start == 0 || end < start {
        return None;
    }
    let all: Vec<&str> = blob.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    // A file ending in a newline splits to a trailing empty piece that
    // is not a line.
    let count = match all.last() {
        Some(&"") => all.len() - 1,
        _ => all.len(),
    };
    if end as usize > count {
        return None;
    }
    let mut cited = all[(start - 1) as usize..end as usize].join("\n");
    cited.push('\n');
    Some(cited)
}

/// What the repository says about one anchor: `Ok(())` when the lines
/// at that commit still hash to what was cited, `Err(why)` in the
/// reader's words when they do not.
pub fn verdict(anchor: &CodeAnchor, blob: Option<&str>) -> Result<(), String> {
    let Some(blob) = blob else {
        return Err("the file is not at that commit".into());
    };
    let Some(cited) = cited(blob, anchor.lines) else {
        let (start, end) = anchor.lines;
        return Err(format!("lines {start}–{end} are past the end of the file"));
    };
    if CodeAnchor::digest_of(&cited) == anchor.digest {
        Ok(())
    } else {
        Err("the lines at that commit are not the ones cited".into())
    }
}

/// How many anchors one pass asks about. Small: each is a request to
/// GitHub, and a backlog drains over several passes rather than in a
/// burst that spends a rate limit.
const ANCHORS_PER_SWEEP: u32 = 25;

/// Start checking anchors against their repositories, if this
/// deployment can read one. Returns at once; the loop is a background
/// task and nothing waits on it.
///
/// Called from [`crate::app`] rather than from a binary's `main`,
/// because there is more than one binary — the open server and the
/// hosted one — and the hosted one silently shipped without this when
/// it lived in a `main`.
pub fn start<S: Storage + Clone + Send + Sync + 'static>(store: S, cfg: &config::Github) {
    let Some(github) = Github::new(cfg) else {
        info!("github not configured — code anchors stay unchecked");
        return;
    };
    info!("github configured — anchors will be checked against their repositories");
    let every = Duration::from_secs(cfg.sweep_secs.max(5));
    tokio::spawn(async move {
        loop {
            let swept = sweep(&store, &github, ANCHORS_PER_SWEEP).await;
            if swept.asked() > 0 {
                info!(
                    matched = swept.matched,
                    mismatched = swept.mismatched,
                    "checked code anchors"
                );
            }
            tokio::time::sleep(every).await;
        }
    });
}

/// Ask the repository about anchors nobody has asked about yet.
///
/// Answers are written one at a time, so a pass that dies halfway has
/// still done its work. An outage writes nothing: a mismatch is a
/// verdict, and we only have one when GitHub answered. An anchor whose
/// project records no repository, or one whose host we cannot read, is
/// left alone rather than marked wrong — nobody asked, and the anchor
/// is still checkable by hand from any clone.
pub async fn sweep<S: Storage>(store: &S, github: &Github, limit: u32) -> Swept {
    let mut swept = Swept::default();
    let Ok(waiting) = store.code_anchors_unchecked(limit).await else {
        return swept;
    };
    for unchecked in waiting {
        let Some(Repository::Github { owner, name }) = unchecked.repository else {
            swept.unreachable += 1;
            continue;
        };
        let anchor = unchecked.anchor;
        let blob = match github
            .blob(&owner, &name, &anchor.commit, &anchor.path)
            .await
        {
            Ok(blob) => blob,
            Err(why) => {
                debug!(%owner, %name, path = %anchor.path, %why, "anchor left unchecked");
                swept.unreachable += 1;
                continue;
            }
        };
        let outcome = verdict(&anchor, blob.as_deref());
        match &outcome {
            Ok(()) => swept.matched += 1,
            Err(_) => swept.mismatched += 1,
        }
        if let Err(e) = store
            .decision_anchor_checked(
                Scope::System,
                unchecked.decision,
                &anchor.commit,
                &anchor.path,
                anchor.lines,
                outcome,
            )
            .await
        {
            warn!(decision = %unchecked.decision, error = %e, "recording an anchor check failed");
        }
    }
    swept
}

/// What one pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// The repository still holds the cited lines.
    pub matched: usize,
    /// It answered, and disagreed.
    pub mismatched: usize,
    /// Nobody could be asked: no repository on the project, a host we
    /// do not read, or GitHub was unreachable.
    pub unreachable: usize,
}

impl Swept {
    pub fn asked(&self) -> usize {
        self.matched + self.mismatched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cited_lines_normalize_the_way_an_excerpt_does() {
        let file = "one\ntwo\nthree\n";
        assert_eq!(cited(file, (1, 1)).as_deref(), Some("one\n"));
        assert_eq!(cited(file, (2, 3)).as_deref(), Some("two\nthree\n"));
        // Windows endings normalize away, which is what makes a digest
        // computed on a clone match one computed here.
        assert_eq!(
            cited("one\r\ntwo\r\n", (1, 2)).as_deref(),
            Some("one\ntwo\n")
        );
        // A file with no trailing newline still cites its last line.
        assert_eq!(cited("one\ntwo", (2, 2)).as_deref(), Some("two\n"));
        // Past the end, and nonsense ranges.
        assert_eq!(cited(file, (3, 4)), None);
        assert_eq!(cited(file, (0, 1)), None);
        assert_eq!(cited(file, (2, 1)), None);
    }

    #[test]
    fn a_verdict_is_about_the_lines_not_the_file() {
        let excerpt = "two\nthree\n".to_string();
        let anchor = CodeAnchor {
            commit: "c".repeat(40),
            path: "a.rs".into(),
            lines: (2, 3),
            digest: CodeAnchor::digest_of(&excerpt),
            excerpt,
            ..Default::default()
        };
        // The same lines, wherever else the file changed.
        assert!(verdict(&anchor, Some("one\ntwo\nthree\nfour\n")).is_ok());
        assert!(verdict(&anchor, Some("ONE\ntwo\nthree\n")).is_ok());
        // Moved, gone, or shorter than the citation.
        assert!(verdict(&anchor, Some("one\ntwo\nTHREE\n")).is_err());
        assert!(verdict(&anchor, Some("one\n")).is_err());
        assert_eq!(
            verdict(&anchor, None).unwrap_err(),
            "the file is not at that commit"
        );
    }

    #[test]
    fn a_deployment_with_nothing_to_prove_itself_with_has_no_client() {
        assert!(Github::new(&config::Github::default()).is_none());
        // Half an App is no App; the token alone still reads.
        assert!(
            Github::new(&config::Github {
                app_id: Some(1),
                ..Default::default()
            })
            .is_none()
        );
        assert!(
            Github::new(&config::Github {
                token: Some("ghp_x".into()),
                ..Default::default()
            })
            .is_some()
        );
    }
}
