//! The expert-model layer — the model-calling substrate under the E0→E3
//! cascade (decision 01KWBXVJ1FF7TNMSA5KG2S57E7).
//!
//! Two ideas, deliberately small:
//!
//! - **Named model endpoints** (`[expert.models.<name>]`): a provider +
//!   model + credentials. Providers today: `anthropic` and `openai` — the
//!   latter is any OpenAI-compatible runtime (OpenAI itself, or ollama /
//!   LM Studio / vLLM / llama.cpp via `base_url`).
//! - **Per-job routing** (`[expert.jobs]`): expert jobs are one-shot
//!   subagents, and each binds to a named model *explicitly* — token-heavy
//!   background verification might run on a local box while a small
//!   latency-sensitive triage call uses a fast hosted model, or the
//!   reverse. There is no universally right assignment, so it is config,
//!   not a default: an unassigned job is *disabled*, and no `[expert]`
//!   section means no model calls at all (the no-egress default).
//!
//! The wire layer is [`genai`] — it owns each provider's protocol (and,
//! at the E2 rung, the tool-calling encodings); this crate owns what
//! genai must not: routing, credential resolution, and the budget. The
//! surface is one-shot by design — `system + user → text` — no
//! streaming, no tools, no conversation state; those belong to E2 when
//! it exists. Every call carries the endpoint's hard timeout, and errors
//! are values, never panics: expert work is best-effort enrichment, and
//! the paths that call it must stay fail-open.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat, JsonSpec};
use genai::resolver::{AuthData, ServiceTargetResolver};
use genai::{ModelIden, ServiceTarget};
use serde::Deserialize;

pub mod signals;

/// The `[expert]` configuration table.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    /// Named model endpoints (`[expert.models.<name>]`).
    #[serde(default)]
    pub models: BTreeMap<String, Endpoint>,
    /// Job → model-name bindings (`[expert.jobs]`). Jobs absent here are
    /// disabled.
    #[serde(default)]
    pub jobs: BTreeMap<String, String>,
}

/// One named model endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct Endpoint {
    pub provider: Provider,
    /// The model to request ("claude-haiku-4-5", "qwen3:8b", …).
    pub model: String,
    /// Origin override. Defaults to the provider's hosted API; point it
    /// at a local runtime's OpenAI-compatible root
    /// (`http://127.0.0.1:11434/v1/`) instead.
    #[serde(default)]
    pub base_url: Option<String>,
    /// The API key, plaintext. Prefer `api_key_cmd`.
    #[serde(default)]
    pub api_key: Option<String>,
    /// A command that *prints* the key (a password-manager call); wins
    /// over `api_key` when both are set. Local runtimes need neither.
    #[serde(default)]
    pub api_key_cmd: Option<String>,
    /// Hard per-call budget, seconds. Expert calls are best-effort
    /// enrichment — callers rely on this bound to stay fail-open.
    #[serde(default = "timeout")]
    pub timeout_secs: f64,
    /// Reply size cap, tokens.
    #[serde(default = "max_tokens")]
    pub max_tokens: u32,
}

fn timeout() -> f64 {
    30.0
}

fn max_tokens() -> u32 {
    1024
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// The Anthropic Messages API.
    Anthropic,
    /// The OpenAI API — or any compatible local runtime via `base_url`.
    Openai,
}

impl Provider {
    fn adapter(self) -> AdapterKind {
        match self {
            Provider::Anthropic => AdapterKind::Anthropic,
            Provider::Openai => AdapterKind::OpenAI,
        }
    }

    /// The hosted default when `base_url` is not set. Both adapters
    /// append the method path ("messages", "chat/completions") to a
    /// `…/v1/` root.
    fn origin(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com/v1/",
            Provider::Openai => "https://api.openai.com/v1/",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::Openai => "openai",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("expert config: {0}")]
    Config(String),
    #[error("expert model call: {0}")]
    Model(#[from] genai::Error),
    #[error("expert call exceeded its {0:?} budget")]
    Timeout(Duration),
    #[error("expert reply unusable: {0}")]
    Shape(String),
}

/// A ready-to-call model endpoint: a genai client pinned to one adapter,
/// origin, and credential by a [`ServiceTargetResolver`], plus the budget
/// genai doesn't own.
#[derive(Debug, Clone)]
pub struct Client {
    genai: genai::Client,
    model: String,
    timeout: Duration,
    options: ChatOptions,
    describe: String,
}

impl Client {
    /// One-shot exchange: a system prompt and a user turn in, the reply
    /// text out. Bounded by the endpoint's timeout.
    pub async fn reply(&self, system: &str, user: &str) -> Result<String, Error> {
        let request = ChatRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)]);
        let response = tokio::time::timeout(
            self.timeout,
            self.genai
                .exec_chat(&self.model, request, Some(&self.options)),
        )
        .await
        .map_err(|_| Error::Timeout(self.timeout))??;
        let text = response
            .first_text()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::Shape("no text in the reply".into()))?;
        Ok(text.to_string())
    }

    /// One-shot **schema-constrained** exchange: the reply must be a JSON
    /// document matching `schema` (enforced provider-side via structured
    /// output, verified here by parsing). Deterministic by construction —
    /// temperature 0. Providers that can't enforce a schema surface a
    /// call-time error, which `expert check` on the bound job exposes.
    pub async fn extract(
        &self,
        system: &str,
        user: &str,
        name: &str,
        schema: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        let request = ChatRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)]);
        let options = self
            .options
            .clone()
            .with_temperature(0.0)
            .with_response_format(ChatResponseFormat::JsonSpec(JsonSpec::new(name, schema)));
        let response = tokio::time::timeout(
            self.timeout,
            self.genai.exec_chat(&self.model, request, Some(&options)),
        )
        .await
        .map_err(|_| Error::Timeout(self.timeout))??;
        let text = response
            .first_text()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::Shape("no text in the reply".into()))?;
        serde_json::from_str(text)
            .map_err(|e| Error::Shape(format!("the reply is not the requested JSON: {e}")))
    }

    /// "provider model @ origin" — for logs and `expert check` output.
    pub fn describe(&self) -> &str {
        &self.describe
    }
}

impl Endpoint {
    /// Build the client: resolve the key, pin the service target, apply
    /// the budget. Fails on a missing required key or a key command that
    /// doesn't produce one.
    pub fn client(&self) -> Result<Client, Error> {
        let key = self.key()?;
        if self.provider == Provider::Anthropic && key.is_none() {
            return Err(Error::Config(format!(
                "model \"{}\": the anthropic provider needs api_key or api_key_cmd",
                self.model
            )));
        }
        let origin = self
            .base_url
            .clone()
            .unwrap_or_else(|| self.provider.origin().to_string());

        // Pin everything from config — never from genai's model-name
        // inference and never from provider env vars (a deployment must
        // not silently pick up an operator's OPENAI_API_KEY).
        let adapter = self.provider.adapter();
        let endpoint = genai::resolver::Endpoint::from_owned(origin.clone());
        // Keyless is only ever a local runtime; the header value is
        // arbitrary there and never checked.
        let auth = AuthData::from_single(key.unwrap_or_else(|| "unused".into()));
        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                Ok(ServiceTarget {
                    endpoint: endpoint.clone(),
                    auth: auth.clone(),
                    model: ModelIden::new(adapter, target.model.model_name.clone()),
                })
            },
        );

        Ok(Client {
            genai: genai::Client::builder()
                .with_service_target_resolver(resolver)
                .build(),
            model: self.model.clone(),
            timeout: Duration::from_secs_f64(self.timeout_secs),
            options: ChatOptions::default().with_max_tokens(self.max_tokens),
            describe: format!("{} {} @ {}", self.provider.name(), self.model, origin),
        })
    }

    fn key(&self) -> Result<Option<String>, Error> {
        match (&self.api_key_cmd, &self.api_key) {
            (Some(cmd), _) => run(cmd).map(Some),
            (None, Some(key)) => Ok(Some(key.clone())),
            (None, None) => Ok(None),
        }
    }
}

/// Resolve an `api_key_cmd`: run it, take stdout. The secret stays out of
/// config files and out of error messages.
fn run(cmd: &str) -> Result<String, Error> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| Error::Config(format!("api_key_cmd `{cmd}`: {e}")))?;
    if !output.status.success() {
        return Err(Error::Config(format!(
            "api_key_cmd `{cmd}` failed with {}",
            output.status
        )));
    }
    let key = String::from_utf8(output.stdout)
        .map_err(|_| Error::Config(format!("api_key_cmd `{cmd}` printed non-UTF-8")))?
        .trim()
        .to_string();
    if key.is_empty() {
        return Err(Error::Config(format!(
            "api_key_cmd `{cmd}` printed nothing"
        )));
    }
    Ok(key)
}

/// The resolved job table: every configured job, bound to its built
/// client. Constructed once at startup so a bad binding fails fast, not
/// on the first signal.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    jobs: BTreeMap<String, Arc<Client>>,
}

impl Registry {
    pub fn new(config: &Config) -> Result<Self, Error> {
        // Build each referenced endpoint once; jobs sharing a model share
        // the client (and its connection pool).
        let mut clients: BTreeMap<&str, Arc<Client>> = BTreeMap::new();
        let mut jobs = BTreeMap::new();
        for (job, model) in &config.jobs {
            let client = match clients.get(model.as_str()) {
                Some(client) => Arc::clone(client),
                None => {
                    let endpoint = config.models.get(model).ok_or_else(|| {
                        Error::Config(format!(
                            "job \"{job}\" is bound to model \"{model}\", which is not \
                             defined under [expert.models]"
                        ))
                    })?;
                    let client = Arc::new(endpoint.client()?);
                    clients.insert(model, Arc::clone(&client));
                    client
                }
            };
            jobs.insert(job.clone(), client);
        }
        Ok(Self { jobs })
    }

    /// The client bound to `job` — `None` means the job is disabled and
    /// the caller skips the enrichment entirely.
    pub fn job(&self, name: &str) -> Option<Arc<Client>> {
        self.jobs.get(name).cloned()
    }

    /// All configured jobs, for startup logs and `expert check`.
    pub fn jobs(&self) -> impl Iterator<Item = (&str, &Client)> {
        self.jobs.iter().map(|(name, c)| (name.as_str(), &**c))
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(value: serde_json::Value) -> Config {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn jobs_bind_to_defined_models_only() {
        let cfg = config(serde_json::json!({
            "models": { "local": { "provider": "openai", "model": "qwen3:8b" } },
            "jobs": { "triage": "cloud" },
        }));
        let err = Registry::new(&cfg).unwrap_err();
        assert!(matches!(err, Error::Config(m) if m.contains("triage") && m.contains("cloud")));
    }

    #[test]
    fn anthropic_requires_a_key() {
        let cfg = config(serde_json::json!({
            "models": { "claude": { "provider": "anthropic", "model": "claude-haiku-4-5" } },
            "jobs": { "triage": "claude" },
        }));
        assert!(matches!(Registry::new(&cfg), Err(Error::Config(_))));
    }

    #[test]
    fn key_cmd_wins_and_is_trimmed() {
        let endpoint: Endpoint = serde_json::from_value(serde_json::json!({
            "provider": "openai", "model": "m",
            "api_key": "plain", "api_key_cmd": "echo ' from-cmd '",
        }))
        .unwrap();
        assert_eq!(endpoint.key().unwrap().unwrap(), "from-cmd");
    }

    #[test]
    fn no_expert_section_is_an_empty_registry() {
        let registry = Registry::new(&Config::default()).unwrap();
        assert!(registry.is_empty());
        assert!(registry.job("triage").is_none());
    }

    #[test]
    fn jobs_sharing_a_model_share_the_client() {
        let cfg = config(serde_json::json!({
            "models": { "local": { "provider": "openai", "model": "m" } },
            "jobs": { "triage": "local", "verify": "local" },
        }));
        let registry = Registry::new(&cfg).unwrap();
        let a = registry.job("triage").unwrap();
        let b = registry.job("verify").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
