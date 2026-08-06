use std::path::Path;

use anyhow::{Context, Result};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::core::WaitFor;
use testcontainers_modules::testcontainers::runners::{AsyncBuilder, AsyncRunner};
use testcontainers_modules::testcontainers::{
    ContainerAsync, GenericBuildableImage, GenericImage, ImageExt,
};

use crate::logs::ContainerLog;

const IMAGE: &str = "converge-e2e-server";
const DOCKERFILE: &str = "crates/converge-e2e/docker/server.Dockerfile";
const PORT: u16 = 8080;

const DATABASE_IMAGE: &str = "converge-e2e-database";
const DATABASE_DOCKERFILE: &str = "crates/converge-e2e/docker/database.Dockerfile";

pub const TOKEN: &str = "cvg_0000000000000000000000000000000000000000000000000000000000000e2e";

pub struct Server {
    database: Database,
    user_handle: String,
    user_name: String,
}

impl Server {
    pub fn new(database: Database) -> Self {
        Self {
            database,
            user_handle: "admin".to_owned(),
            user_name: "Admin".to_owned(),
        }
    }

    pub fn with_user(mut self, handle: impl Into<String>, name: impl Into<String>) -> Self {
        self.user_handle = handle.into();
        self.user_name = name.into();
        self
    }

    pub(crate) async fn start(
        &self,
        workspace_root: &Path,
        network: &str,
        database_name: &str,
        server_name: &str,
    ) -> Result<RunningServer> {
        let database = self
            .database
            .start(workspace_root, network, database_name)
            .await?;

        let image = GenericBuildableImage::new(IMAGE, "latest")
            .with_dockerfile(workspace_root.join(DOCKERFILE))
            .with_file(workspace_root.join("Cargo.toml"), "Cargo.toml")
            .with_file(workspace_root.join("Cargo.lock"), "Cargo.lock")
            .with_file(workspace_root.join(".sqlx"), ".sqlx")
            .with_file(workspace_root.join("crates"), "crates")
            .build_image()
            .await
            .context("build the Converge server image")?;

        let url = format!("http://{server_name}:{PORT}");
        let server = image
            .with_wait_for(WaitFor::message_on_stdout("converge-server listening"))
            .with_container_name(server_name.to_owned())
            .with_network(network.to_owned())
            .with_env_var("CONVERGE_DATABASE_URL", self.database.url(database_name))
            .with_env_var("CONVERGE_LISTEN", format!("0.0.0.0:{PORT}"))
            .with_env_var("CONVERGE_AUTH__PUBLIC_URL", &url)
            .with_env_var("CONVERGE_USER__HANDLE", &self.user_handle)
            .with_env_var("CONVERGE_USER__NAME", &self.user_name)
            .start()
            .await
            .context("start the Converge server container")?;

        Ok(RunningServer {
            server,
            database,
            url,
        })
    }
}

#[derive(Default)]
pub struct Database;

impl Database {
    fn url(&self, name: &str) -> String {
        format!("postgres://postgres:postgres@{name}:5432/postgres")
    }

    async fn start(
        &self,
        workspace_root: &Path,
        network: &str,
        name: &str,
    ) -> Result<ContainerAsync<Postgres>> {
        let _built = GenericBuildableImage::new(DATABASE_IMAGE, "latest")
            .with_dockerfile(workspace_root.join(DATABASE_DOCKERFILE))
            .with_file(
                workspace_root.join("crates/converge-storage-postgres/migrations"),
                "crates/converge-storage-postgres/migrations",
            )
            .with_file(
                workspace_root.join("crates/converge-e2e/docker"),
                "crates/converge-e2e/docker",
            )
            .build_image()
            .await
            .context("build the database image")?;

        Postgres::default()
            .with_name(DATABASE_IMAGE)
            .with_tag("latest")
            .with_container_name(name.to_owned())
            .with_network(network.to_owned())
            .start()
            .await
            .context("start the database container")
    }
}

pub struct RunningServer {
    server: ContainerAsync<GenericImage>,
    database: ContainerAsync<Postgres>,
    url: String,
}

impl RunningServer {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub(crate) async fn logs(&self) -> Result<(ContainerLog, ContainerLog)> {
        let server = ContainerLog::read(&self.server)
            .await
            .context("read the Converge server's log")?;
        let database = ContainerLog::read(&self.database)
            .await
            .context("read the database's log")?;
        Ok((server, database))
    }

    pub(crate) async fn stop(&self) -> Result<()> {
        self.server
            .stop_with_timeout(None)
            .await
            .context("stop the Converge server")?;
        self.database
            .stop_with_timeout(None)
            .await
            .context("stop the database")?;
        Ok(())
    }
}
