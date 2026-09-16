use std::path::{Path, PathBuf};

use super::world::Agent;
use testcontainers_modules::testcontainers::GenericImage;
use tokio::sync::OnceCell;

pub const CODEX_VERSION: &str = "0.154.0";
static IMAGE: OnceCell<GenericImage> = OnceCell::const_new();

#[derive(Debug, Clone, Copy)]
pub struct CodexCli;

impl Agent for CodexCli {
    fn image_name(&self) -> &'static str {
        "converge-e2e-codex"
    }

    fn image_tag(&self) -> &'static str {
        CODEX_VERSION
    }

    fn dockerfile(&self, workspace_root: &Path) -> PathBuf {
        workspace_root.join("crates/converge-e2e/docker/codex.Dockerfile")
    }

    fn build_arguments(&self) -> Vec<(&'static str, &'static str)> {
        vec![("CODEX_VERSION", CODEX_VERSION)]
    }

    fn image_cache(&self) -> &'static OnceCell<GenericImage> {
        &IMAGE
    }
}
