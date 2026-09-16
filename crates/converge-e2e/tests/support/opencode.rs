use std::path::{Path, PathBuf};

use super::world::Agent;
use testcontainers_modules::testcontainers::GenericImage;
use tokio::sync::OnceCell;

pub const OPENCODE_VERSION: &str = "1.18.31";
static IMAGE: OnceCell<GenericImage> = OnceCell::const_new();

#[derive(Debug, Clone, Copy)]
pub struct OpenCode;

impl Agent for OpenCode {
    fn image_name(&self) -> &'static str {
        "converge-e2e-opencode"
    }

    fn image_tag(&self) -> &'static str {
        OPENCODE_VERSION
    }

    fn dockerfile(&self, workspace_root: &Path) -> PathBuf {
        workspace_root.join("crates/converge-e2e/docker/opencode.Dockerfile")
    }

    fn build_arguments(&self) -> Vec<(&'static str, &'static str)> {
        vec![("OPENCODE_VERSION", OPENCODE_VERSION)]
    }

    fn image_cache(&self) -> &'static OnceCell<GenericImage> {
        &IMAGE
    }
}
