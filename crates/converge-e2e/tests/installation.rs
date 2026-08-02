pub mod support;

use std::time::Duration;

use anyhow::{Context, Result};
use support::{ClaudeCode, Server, TestWorld};

const INSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const INIT_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/converge-lab/converge/main/install.sh | sh";
const CONVERGE_BIN: &str = "/home/e2e/.converge/bin/converge";
const CONVERGE_LINK: &str = "/home/e2e/.local/bin/converge";

#[tokio::test]
async fn install_and_init_happy_path() -> Result<()> {
    let world = TestWorld::builder(ClaudeCode)
        .with_server(Server::default())
        .start()
        .await?;

    assert!(!world.exists(CONVERGE_BIN).await?);
    assert!(!world.exists(CONVERGE_LINK).await?);

    let install = tokio::time::timeout(INSTALL_TIMEOUT, world.exec(&["sh", "-c", INSTALL_COMMAND]))
        .await
        .context("the installer did not finish within five minutes")??;
    assert_eq!(install.exit_code, 0);
    assert_eq!(world.read_link(CONVERGE_LINK).await?, CONVERGE_BIN);

    let version = world.exec(&[CONVERGE_LINK, "--version"]).await?;
    assert_eq!(version.exit_code, 0);
    assert!(!version.stdout.trim().is_empty());

    let (server_url, token) = {
        let server = world
            .server()
            .context("the Converge server is not running")?;
        (server.url().to_owned(), server.token().to_owned())
    };
    let init = tokio::time::timeout(
        INIT_TIMEOUT,
        world.pipe(&[
            &["printf", "%s\n%s\n\n", &server_url, &token],
            &[CONVERGE_LINK, "init"],
        ]),
    )
    .await
    .context("converge init did not finish within one minute")??;
    assert_eq!(init.exit_code, 0);

    let registration = world.exec(&["claude", "mcp", "get", "converge"]).await?;
    assert_eq!(registration.exit_code, 0);

    Ok(())
}
