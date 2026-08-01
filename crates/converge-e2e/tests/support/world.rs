use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::core::{
    BuildImageOptions, CmdWaitFor, ExecCommand, WaitFor,
};
use testcontainers_modules::testcontainers::runners::{AsyncBuilder, AsyncRunner};
use testcontainers_modules::testcontainers::{
    ContainerAsync, GenericBuildableImage, GenericImage, ImageExt,
};
use tokio::sync::OnceCell;
use uuid::Uuid;

pub trait Agent {
    fn image_name(&self) -> &'static str;
    fn image_tag(&self) -> &'static str;
    fn dockerfile(&self, workspace_root: &Path) -> PathBuf;
    fn build_arguments(&self) -> Vec<(&'static str, &'static str)>;
    fn image_cache(&self) -> &'static OnceCell<GenericImage>;
}

#[derive(Debug, Clone)]
pub struct Server {
    log_filter: Option<String>,
    public_url: Option<String>,
    session_secret: Option<String>,
    user_handle: String,
    user_name: String,
    token_label: String,
}

impl Server {
    pub fn with_log_filter(mut self, filter: impl Into<String>) -> Self {
        self.log_filter = Some(filter.into());
        self
    }

    pub fn with_public_url(mut self, public_url: impl Into<String>) -> Self {
        self.public_url = Some(public_url.into());
        self
    }

    pub fn with_session_secret(mut self, session_secret: impl Into<String>) -> Self {
        self.session_secret = Some(session_secret.into());
        self
    }

    pub fn with_user(mut self, handle: impl Into<String>, name: impl Into<String>) -> Self {
        self.user_handle = handle.into();
        self.user_name = name.into();
        self
    }

    pub fn with_token_label(mut self, token_label: impl Into<String>) -> Self {
        self.token_label = token_label.into();
        self
    }
}

impl Default for Server {
    fn default() -> Self {
        Self {
            log_filter: None,
            public_url: None,
            session_secret: None,
            user_handle: "admin".to_owned(),
            user_name: "Admin".to_owned(),
            token_label: "e2e".to_owned(),
        }
    }
}

pub struct CommandOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

pub struct RunningServer {
    _server_container: ContainerAsync<GenericImage>,
    _postgres_container: ContainerAsync<Postgres>,
    url: String,
    token: String,
}

impl RunningServer {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

pub struct TestWorld<A> {
    // Drop the server before the agent whose network namespace it uses.
    server: Option<RunningServer>,
    agent_container: ContainerAsync<GenericImage>,
    _agent: A,
}

pub struct TestWorldBuilder<A> {
    agent: A,
    server: Option<Server>,
}

impl<A> TestWorld<A> {
    pub fn builder(agent: A) -> TestWorldBuilder<A> {
        TestWorldBuilder {
            agent,
            server: None,
        }
    }

    pub fn server(&self) -> Option<&RunningServer> {
        self.server.as_ref()
    }

    pub async fn exec(&self, command: &[&str]) -> Result<CommandOutput> {
        command_output(&self.agent_container, command, &[]).await
    }

    pub async fn exec_with_env(
        &self,
        command: &[&str],
        env: &[(&str, &str)],
    ) -> Result<CommandOutput> {
        command_output(&self.agent_container, command, env).await
    }
}

impl<A> TestWorldBuilder<A> {
    pub fn with_server(mut self, server: Server) -> Self {
        self.server = Some(server);
        self
    }
}

impl<A: Agent> TestWorldBuilder<A> {
    pub async fn start(self) -> Result<TestWorld<A>> {
        let Self { agent, server } = self;
        let workspace_root = workspace_root()?;

        let mut build_options = BuildImageOptions::new();
        for (name, value) in agent.build_arguments() {
            build_options = build_options.with_build_arg(name, value);
        }

        let image = agent
            .image_cache()
            .get_or_try_init(|| async {
                GenericBuildableImage::new(agent.image_name(), agent.image_tag())
                    .with_dockerfile(agent.dockerfile(&workspace_root))
                    .with_file(workspace_root.join("Cargo.toml"), "Cargo.toml")
                    .with_file(workspace_root.join("Cargo.lock"), "Cargo.lock")
                    .with_file(workspace_root.join(".sqlx"), ".sqlx")
                    .with_file(workspace_root.join("crates"), "crates")
                    .build_image_with(build_options)
                    .await
                    .context("build the agent E2E image")
            })
            .await?
            .clone();

        let suffix = Uuid::new_v4().simple().to_string();
        let network = format!("converge-e2e-{suffix}");
        let postgres_name = format!("converge-e2e-postgres-{suffix}");
        let server_name = format!("converge-e2e-server-{suffix}");
        let agent_name = format!("converge-e2e-agent-{suffix}");

        let postgres_container = if server.is_some() {
            Some(
                Postgres::default()
                    .with_tag("16-alpine")
                    .with_container_name(postgres_name.clone())
                    .with_network(network.clone())
                    .start()
                    .await
                    .context("start the E2E Postgres container")?,
            )
        } else {
            None
        };

        let agent_container = image
            .clone()
            .with_cmd(["sleep", "infinity"])
            .with_container_name(agent_name.clone())
            .with_network(network)
            .start()
            .await
            .context("start the E2E agent container")?;

        let running_server = match (server, postgres_container) {
            (Some(server), Some(postgres_container)) => {
                let database_url =
                    format!("postgres://postgres:postgres@{postgres_name}:5432/postgres");
                let mut request = image
                    .with_wait_for(WaitFor::message_on_stdout("converge-server listening"))
                    .with_cmd(["converge-server"])
                    .with_container_name(server_name)
                    .with_network(format!("container:{agent_name}"))
                    .with_env_var("CONVERGE_DATABASE_URL", database_url)
                    .with_env_var("CONVERGE_LISTEN", "0.0.0.0:8080")
                    .with_env_var("CONVERGE_USER__HANDLE", &server.user_handle)
                    .with_env_var("CONVERGE_USER__NAME", &server.user_name);

                if let Some(filter) = &server.log_filter {
                    request = request.with_env_var("CONVERGE_LOG__FILTER", filter);
                }
                if let Some(public_url) = &server.public_url {
                    request = request.with_env_var("CONVERGE_AUTH__PUBLIC_URL", public_url);
                }
                if let Some(session_secret) = &server.session_secret {
                    request = request.with_env_var("CONVERGE_AUTH__SESSION_SECRET", session_secret);
                }

                let server_container = request
                    .start()
                    .await
                    .context("start the E2E Converge server container")?;

                let minted = command_output(
                    &server_container,
                    &["converge-server", "token", "mint", &server.token_label],
                    &[],
                )
                .await
                .context("mint the E2E Converge token")?;
                if minted.exit_code != 0 {
                    bail!(
                        "mint the E2E Converge token: exit {}\nstdout:\n{}\nstderr:\n{}",
                        minted.exit_code,
                        minted.stdout,
                        minted.stderr
                    );
                }
                let token = minted.stdout.trim().to_owned();
                if token.is_empty() {
                    bail!("mint the E2E Converge token: command printed an empty token");
                }

                Some(RunningServer {
                    _server_container: server_container,
                    _postgres_container: postgres_container,
                    url: "http://localhost:8080".to_owned(),
                    token,
                })
            }
            (None, None) => None,
            _ => unreachable!("Postgres is created exactly when a server is configured"),
        };

        Ok(TestWorld {
            server: running_server,
            agent_container,
            _agent: agent,
        })
    }
}

async fn command_output<I>(
    container: &ContainerAsync<I>,
    command: &[&str],
    env: &[(&str, &str)],
) -> Result<CommandOutput>
where
    I: testcontainers_modules::testcontainers::Image,
{
    let mut result = container
        .exec(
            ExecCommand::new(command.iter().copied())
                .with_cmd_ready_condition(CmdWaitFor::exit())
                .with_env_vars(env.iter().copied()),
        )
        .await
        .with_context(|| format!("execute in container: {command:?}"))?;

    let exit_code = result
        .exit_code()
        .await
        .with_context(|| format!("read exit code for: {command:?}"))?
        .with_context(|| format!("command did not exit: {command:?}"))?;
    let stdout = String::from_utf8(
        result
            .stdout_to_vec()
            .await
            .with_context(|| format!("read stdout for: {command:?}"))?,
    )
    .with_context(|| format!("stdout was not UTF-8 for: {command:?}"))?;
    let stderr = String::from_utf8(
        result
            .stderr_to_vec()
            .await
            .with_context(|| format!("read stderr for: {command:?}"))?,
    )
    .with_context(|| format!("stderr was not UTF-8 for: {command:?}"))?;

    Ok(CommandOutput {
        exit_code,
        stdout,
        stderr,
    })
}

fn workspace_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("locate the Converge workspace root")
}
