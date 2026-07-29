use std::path::{Path, PathBuf};

use super::world::Agent;
use testcontainers_modules::testcontainers::GenericImage;
use tokio::sync::OnceCell;

pub const CLAUDE_CODE_VERSION: &str = "2.1.220";
static IMAGE: OnceCell<GenericImage> = OnceCell::const_new();

#[derive(Debug, Clone, Copy)]
pub struct ClaudeCode;

impl Agent for ClaudeCode {
    fn image_name(&self) -> &'static str {
        "converge-e2e-claude"
    }

    fn image_tag(&self) -> &'static str {
        CLAUDE_CODE_VERSION
    }

    fn dockerfile(&self, workspace_root: &Path) -> PathBuf {
        workspace_root.join("crates/converge-e2e/docker/Dockerfile")
    }

    fn build_arguments(&self) -> Vec<(&'static str, &'static str)> {
        vec![("CLAUDE_CODE_VERSION", CLAUDE_CODE_VERSION)]
    }

    fn image_cache(&self) -> &'static OnceCell<GenericImage> {
        &IMAGE
    }
}
