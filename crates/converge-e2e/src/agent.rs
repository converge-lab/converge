use std::path::Path;

use anyhow::{Context, Result};
use testcontainers_modules::testcontainers::core::BuildImageOptions;
use testcontainers_modules::testcontainers::runners::{AsyncBuilder, AsyncRunner};
use testcontainers_modules::testcontainers::{
    ContainerAsync, GenericBuildableImage, GenericImage, ImageExt,
};

const IMAGE: &str = "converge-e2e-agent";
const DOCKERFILE: &str = "crates/converge-e2e/docker/agent.Dockerfile";

pub struct Agent {
    tag: String,
    base_image: String,
    install: String,
    env: Vec<(String, String)>,
}

impl Agent {
    pub fn claude_code(version: &str) -> Self {
        Self {
            tag: format!("claude-{version}"),
            base_image: "node:22-bookworm-slim".to_owned(),
            install: format!(
                "npm install --global @anthropic-ai/claude-code@{version} \
                 && npm cache clean --force"
            ),
            env: vec![("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned())],
        }
    }

    pub fn pointed_at(mut self, endpoint: &str) -> Self {
        self.env
            .push(("ANTHROPIC_BASE_URL".to_owned(), endpoint.to_owned()));
        self.env
            .push(("ANTHROPIC_API_KEY".to_owned(), "e2e".to_owned()));
        self
    }

    pub(crate) async fn start(
        &self,
        workspace_root: &Path,
        network: &str,
        name: &str,
    ) -> Result<ContainerAsync<GenericImage>> {
        let build_options = BuildImageOptions::new()
            .with_build_arg("BASE_IMAGE", &self.base_image)
            .with_build_arg("AGENT_INSTALL", &self.install);

        let image = GenericBuildableImage::new(IMAGE, &self.tag)
            .with_dockerfile(workspace_root.join(DOCKERFILE))
            .build_image_with(build_options)
            .await
            .context("build the agent image")?;

        let mut request = image
            .with_cmd(["sleep", "infinity"])
            .with_container_name(name.to_owned())
            .with_network(network.to_owned());
        for (name, value) in &self.env {
            request = request.with_env_var(name, value);
        }

        request.start().await.context("start the agent container")
    }
}
