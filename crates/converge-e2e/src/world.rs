use std::path::PathBuf;

use anyhow::{Context, Result};
use testcontainers_modules::testcontainers::core::{CmdWaitFor, ExecCommand};
use testcontainers_modules::testcontainers::{ContainerAsync, GenericImage};
use uuid::Uuid;

use crate::agent::Agent;
use crate::command::{Command, CommandResult};
use crate::logs::ContainerLog;
use crate::model::{Model, RunningModel};
use crate::server::{RunningServer, Server};

pub struct TestWorld {
    agent: Agent,
    server: Option<Server>,
    model: Option<Model>,
}

impl TestWorld {
    pub fn new(agent: Agent) -> Self {
        Self {
            agent,
            server: None,
            model: None,
        }
    }

    pub fn with_server(mut self, server: Server) -> Self {
        self.server = Some(server);
        self
    }

    pub fn with_model(mut self, model: Model) -> Self {
        self.model = Some(model);
        self
    }

    pub async fn start(self) -> Result<RunningWorld> {
        let workspace_root = workspace_root()?;

        let suffix = Uuid::new_v4().simple().to_string();
        let network = format!("converge-e2e-{suffix}");
        let agent_name = format!("converge-e2e-agent-{suffix}");
        let database_name = format!("converge-e2e-database-{suffix}");
        let server_name = format!("converge-e2e-server-{suffix}");
        let model_name = format!("converge-e2e-model-{suffix}");

        let model = match self.model {
            Some(model) => Some(model.start(&network, &model_name).await?),
            None => None,
        };

        let agent = match &model {
            Some(running) => self.agent.pointed_at(running.agent_url()),
            None => self.agent,
        };
        let agent = agent.start(&workspace_root, &network, &agent_name).await?;

        let server = match self.server {
            Some(server) => Some(
                server
                    .start(&workspace_root, &network, &database_name, &server_name)
                    .await?,
            ),
            None => None,
        };

        Ok(RunningWorld {
            server,
            agent,
            model,
        })
    }
}

pub struct RunningWorld {
    server: Option<RunningServer>,
    agent: ContainerAsync<GenericImage>,
    model: Option<RunningModel>,
}

impl RunningWorld {
    pub fn server(&self) -> Option<&RunningServer> {
        self.server.as_ref()
    }

    pub fn model(&self) -> Option<&RunningModel> {
        self.model.as_ref()
    }

    pub async fn run(&self, command: &Command) -> Result<CommandResult> {
        let what = command.program();
        let argv = command.argv();

        let mut result = self
            .agent
            .exec(
                ExecCommand::new(argv.iter().map(String::as_str))
                    .with_cmd_ready_condition(CmdWaitFor::exit()),
            )
            .await
            .with_context(|| format!("execute in the agent: {what}"))?;

        let exit_code = result
            .exit_code()
            .await
            .with_context(|| format!("read the exit code of {what}"))?
            .with_context(|| format!("{what} never exited"))?;

        let stdout = read(result.stdout_to_vec().await, what, "stdout")?;
        let stderr = read(result.stderr_to_vec().await, what, "stderr")?;

        Ok(CommandResult {
            outcome: exit_code.into(),
            stdout,
            stderr,
        })
    }

    pub async fn stop(self) -> Result<WorldLogs> {
        let agent = ContainerLog::read(&self.agent)
            .await
            .context("read the agent's log")?;
        let (server, database) = match &self.server {
            Some(running) => {
                let (server, database) = running.logs().await?;
                (Some(server), Some(database))
            }
            None => (None, None),
        };

        if let Some(running) = &self.server {
            running.stop().await?;
        }
        self.agent
            .stop_with_timeout(None)
            .await
            .context("stop the agent")?;

        Ok(WorldLogs {
            agent,
            server,
            database,
        })
    }
}

pub struct WorldLogs {
    pub agent: ContainerLog,
    pub server: Option<ContainerLog>,
    pub database: Option<ContainerLog>,
}

fn read(
    bytes: Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static>,
    what: &str,
    stream: &str,
) -> Result<String> {
    let bytes = bytes.with_context(|| format!("read the {stream} of {what}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn workspace_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("locate the Converge workspace root")
}
