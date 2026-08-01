pub mod support;

use anyhow::Result;
use serde_json::Value;
use support::{CLAUDE_CODE_VERSION, ClaudeCode, Server, TestWorld};

#[tokio::test]
async fn claude_init_installs_hooks_into_clean_home() -> Result<()> {
    let world = TestWorld::builder(ClaudeCode)
        .with_server(Server::default())
        .start()
        .await?;
    let server = world.server().expect("server configured in the builder");

    let claude_version = world.exec(&["claude", "--version"]).await?;
    assert_eq!(
        claude_version.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        claude_version.stdout, claude_version.stderr
    );
    assert!(
        claude_version.stdout.contains(CLAUDE_CODE_VERSION)
            || claude_version.stderr.contains(CLAUDE_CODE_VERSION),
        "expected Claude Code {CLAUDE_CODE_VERSION}\nstdout:\n{}\nstderr:\n{}",
        claude_version.stdout,
        claude_version.stderr
    );

    let settings_before = world
        .exec(&["sh", "-c", r#"test ! -e "$HOME/.claude/settings.json""#])
        .await?;
    assert_eq!(
        settings_before.exit_code, 0,
        "the agent HOME was not clean before converge init\nstdout:\n{}\nstderr:\n{}",
        settings_before.stdout, settings_before.stderr
    );

    let init = world
        .exec_with_env(
            &["sh", "-c", "printf 'n\\n' | converge init"],
            &[
                ("CONVERGE_SERVER", server.url()),
                ("CONVERGE_TOKEN", server.token()),
            ],
        )
        .await?;
    assert_eq!(
        init.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        init.stdout, init.stderr
    );
    assert!(
        init.stdout.contains("hooks: installed"),
        "stdout:\n{}\nstderr:\n{}",
        init.stdout,
        init.stderr
    );
    assert!(
        init.stdout.contains("skipped — register later with:"),
        "stdout:\n{}\nstderr:\n{}",
        init.stdout,
        init.stderr
    );

    let settings_file = world
        .exec(&["sh", "-c", r#"cat "$HOME/.claude/settings.json""#])
        .await?;
    assert_eq!(
        settings_file.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        settings_file.stdout, settings_file.stderr
    );

    let settings: Value = serde_json::from_str(&settings_file.stdout)?;

    assert_eq!(
        settings["hooks"]["SessionStart"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["type"],
        "command"
    );
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "/usr/local/bin/converge hook inject"
    );

    assert_eq!(
        settings["hooks"]["PreToolUse"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["matcher"],
        "mcp__converge__"
    );
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["type"],
        "command"
    );
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "/usr/local/bin/converge hook ctx"
    );

    assert_eq!(
        settings["hooks"]["PostToolUse"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        settings["hooks"]["PostToolUse"][0]["matcher"],
        "mcp__converge__(project_bind|project_dismiss)"
    );
    assert_eq!(
        settings["hooks"]["PostToolUse"][0]["hooks"][0]["type"],
        "command"
    );
    assert_eq!(
        settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
        "/usr/local/bin/converge hook mark"
    );

    assert_eq!(
        settings["hooks"]["SessionEnd"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        settings["hooks"]["SessionEnd"][0]["hooks"][0]["type"],
        "command"
    );
    assert_eq!(
        settings["hooks"]["SessionEnd"][0]["hooks"][0]["command"],
        "/usr/local/bin/converge hook sync"
    );

    Ok(())
}

#[tokio::test]
async fn claude_init_registers_mcp_server() -> Result<()> {
    let world = TestWorld::builder(ClaudeCode)
        .with_server(Server::default())
        .start()
        .await?;
    let server = world.server().expect("server configured in the builder");

    let mcp_before = world.exec(&["claude", "mcp", "get", "converge"]).await?;
    assert_ne!(
        mcp_before.exit_code, 0,
        "the agent HOME already contained a converge MCP server\nstdout:\n{}\nstderr:\n{}",
        mcp_before.stdout, mcp_before.stderr
    );

    let init = world
        .exec_with_env(
            &["sh", "-c", "printf '\\n' | converge init"],
            &[
                ("CONVERGE_SERVER", server.url()),
                ("CONVERGE_TOKEN", server.token()),
            ],
        )
        .await?;
    assert_eq!(
        init.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        init.stdout, init.stderr
    );
    assert!(
        init.stdout.contains("mcp: registered"),
        "stdout:\n{}\nstderr:\n{}",
        init.stdout,
        init.stderr
    );

    let mcp = world.exec(&["claude", "mcp", "get", "converge"]).await?;
    assert_eq!(
        mcp.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout, mcp.stderr
    );
    assert!(
        mcp.stdout.contains("Scope: User config"),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );
    assert!(
        mcp.stdout.contains("Status: ✔ Connected"),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );
    assert!(
        mcp.stdout.contains("Type: http"),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );
    assert!(
        mcp.stdout.contains(&format!("URL: {}/mcp", server.url())),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );

    let authorization = format!("Authorization: Bearer {}", server.token());
    assert!(
        mcp.stdout.lines().any(|line| line.trim() == authorization),
        "Claude MCP config did not contain the expected Authorization header"
    );

    Ok(())
}
