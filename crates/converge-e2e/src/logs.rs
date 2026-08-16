use std::fmt;

use anyhow::{Context, Result};
use testcontainers_modules::testcontainers::{ContainerAsync, Image};

#[derive(Default)]
pub struct ContainerLog {
    pub stdout: String,
    pub stderr: String,
}

impl ContainerLog {
    pub(crate) async fn read<I: Image>(container: &ContainerAsync<I>) -> Result<Self> {
        let stdout = container
            .stdout_to_vec()
            .await
            .context("read a container's stdout")?;
        let stderr = container
            .stderr_to_vec()
            .await
            .context("read a container's stderr")?;

        Ok(Self {
            stdout: plain(&stdout),
            stderr: plain(&stderr),
        })
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.stdout.contains(needle) || self.stderr.contains(needle)
    }
}

impl fmt::Debug for ContainerLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (stream, text) in [("stdout", &self.stdout), ("stderr", &self.stderr)] {
            if text.trim().is_empty() {
                continue;
            }
            writeln!(f, "\n── {stream} ──")?;
            for line in text.lines() {
                writeln!(f, "{line}")?;
            }
        }
        Ok(())
    }
}

fn plain(bytes: &[u8]) -> String {
    let stripped = anstream::adapter::strip_bytes(bytes).into_vec();
    String::from_utf8_lossy(&stripped).into_owned()
}
