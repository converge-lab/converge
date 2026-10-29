use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Value, json};
use testcontainers_modules::testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, GenericImage, ImageExt};

const IMAGE: &str = "mockserver/mockserver";
const TAG: &str = "7.5.0";
const PORT: u16 = 1080;
const ENDPOINT: &str = "/v1/messages";

pub enum Reply {
    Text(String),
    ToolCall { name: String, input: Value },
}

pub enum When {
    Anything,
    ToolOffered(String),
}

#[derive(Default)]
pub struct Model;

impl Model {
    pub(crate) async fn start(&self, network: &str, name: &str) -> Result<RunningModel> {
        let container = GenericImage::new(IMAGE, TAG)
            .with_wait_for(WaitFor::healthcheck())
            .with_exposed_port(PORT.tcp())
            .with_container_name(name.to_owned())
            .with_network(network.to_owned())
            .with_env_var("SERVER_PORT", PORT.to_string())
            .start()
            .await
            .context("start the model container")?;

        let host = container
            .get_host()
            .await
            .context("resolve the Docker host")?;
        let port = container
            .get_host_port_ipv4(PORT.tcp())
            .await
            .context("publish the model's control port")?;

        Ok(RunningModel {
            _container: container,
            agent_url: format!("http://{name}:{PORT}"),
            control_url: format!("http://{host}:{port}"),
            http: Client::new(),
        })
    }
}

pub struct RunningModel {
    _container: ContainerAsync<GenericImage>,
    agent_url: String,
    control_url: String,
    http: Client,
}

impl RunningModel {
    pub fn agent_url(&self) -> &str {
        &self.agent_url
    }

    pub async fn always(&self, reply: Reply) -> Result<()> {
        self.expect(When::Anything, reply, None).await
    }

    pub async fn once(&self, when: When, reply: Reply) -> Result<()> {
        self.expect(when, reply, Some(1)).await
    }

    pub async fn turns(&self) -> Result<Vec<Value>> {
        let recorded = self
            .control("/mockserver/retrieve?type=REQUESTS&format=JSON", None)
            .await?;
        let recorded: Vec<Value> =
            serde_json::from_str(&recorded).context("parse the recorded model turns")?;

        Ok(recorded
            .into_iter()
            .filter(|request| request["path"] == ENDPOINT)
            .filter_map(|request| request["body"].get("json").cloned())
            .collect())
    }

    async fn expect(&self, when: When, reply: Reply, times: Option<u32>) -> Result<()> {
        let mut request = json!({ "method": "POST", "path": ENDPOINT });
        let priority = match &when {
            When::Anything => 0,
            When::ToolOffered(tool) => {
                request["body"] = json!({
                    "type": "JSON_PATH",
                    "jsonPath": format!("$.tools[?(@.name=='{tool}')]"),
                });
                10
            }
        };

        let completion = match &reply {
            Reply::Text(text) => json!({
                "text": text,
                "stopReason": "end_turn",
                "streaming": true,
            }),
            Reply::ToolCall { name, input } => json!({
                "toolCalls": [{ "name": name, "arguments": input.to_string() }],
                "stopReason": "tool_use",
                "streaming": true,
            }),
        };

        let mut expectation = json!({
            "priority": priority,
            "httpRequest": request,
            "httpLlmResponse": { "provider": "ANTHROPIC", "completion": completion },
        });
        if let Some(times) = times {
            expectation["times"] = json!({ "remainingTimes": times });
        }

        self.control("/mockserver/expectation", Some(&expectation))
            .await?;
        Ok(())
    }

    async fn control(&self, path: &str, body: Option<&Value>) -> Result<String> {
        let url = format!("{}{path}", self.control_url);
        let mut request = self.http.put(&url);
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("reach the model at {url}"))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("model {path}: {status}\n{body}");
        }
        Ok(body)
    }
}
