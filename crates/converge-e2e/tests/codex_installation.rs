pub mod support;

use anyhow::Result;
use serde_json::Value;
use support::{CODEX_VERSION, CodexCli, Server, TestWorld};

#[tokio::test]
async fn codex_init_installs_hooks_into_clean_home() -> Result<()> {
    let world = TestWorld::builder(CodexCli)
        .with_server(Server::default())
        .start()
        .await?;
    let server = world.server().expect("server configured in the builder");

    let version = world.exec(&["codex", "--version"]).await?;
    assert_eq!(
        version.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        version.stdout, version.stderr
    );
    assert!(
        version.stdout.contains(CODEX_VERSION) || version.stderr.contains(CODEX_VERSION),
        "expected Codex CLI {CODEX_VERSION}\nstdout:\n{}\nstderr:\n{}",
        version.stdout,
        version.stderr
    );

    let before = world
        .exec(&["sh", "-c", r#"test ! -e "$HOME/.codex/hooks.json""#])
        .await?;
    assert_eq!(
        before.exit_code, 0,
        "the agent HOME already contained codex hooks\nstdout:\n{}\nstderr:\n{}",
        before.stdout, before.stderr
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
    // The trust gate is the one thing a Codex user must do by hand; if
    // this line ever stops printing, the integration silently does
    // nothing on their machine.
    assert!(
        init.stdout.contains("/hooks"),
        "init never mentioned trusting the hooks\nstdout:\n{}\nstderr:\n{}",
        init.stdout,
        init.stderr
    );

    let hooks_file = world
        .exec(&["sh", "-c", r#"cat "$HOME/.codex/hooks.json""#])
        .await?;
    assert_eq!(
        hooks_file.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        hooks_file.stdout, hooks_file.stderr
    );
    let hooks: Value = serde_json::from_str(&hooks_file.stdout)?;

    for (event, matcher, command) in [
        (
            "SessionStart",
            None,
            "/usr/local/bin/converge hook inject --harness codex",
        ),
        (
            "SessionEnd",
            None,
            "/usr/local/bin/converge hook sync --harness codex",
        ),
        (
            "PreToolUse",
            Some("mcp__converge__"),
            "/usr/local/bin/converge hook ctx --harness codex",
        ),
        (
            "PostToolUse",
            Some("mcp__converge__(project_bind|project_dismiss)"),
            "/usr/local/bin/converge hook mark --harness codex",
        ),
        (
            "UserPromptSubmit",
            None,
            "/usr/local/bin/converge hook poll --harness codex",
        ),
    ] {
        let groups = hooks["hooks"][event].as_array().map(Vec::len);
        assert_eq!(groups, Some(1), "{event}: {}", hooks_file.stdout);
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["type"], "command",
            "{event}"
        );
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["command"], command,
            "{event}"
        );
        match matcher {
            Some(matcher) => assert_eq!(hooks["hooks"][event][0]["matcher"], matcher, "{event}"),
            None => assert!(
                hooks["hooks"][event][0]["matcher"].is_null(),
                "{event} should carry no matcher"
            ),
        }
    }

    Ok(())
}

#[tokio::test]
async fn codex_init_registers_mcp_server() -> Result<()> {
    let world = TestWorld::builder(CodexCli)
        .with_server(Server::default())
        .start()
        .await?;
    let server = world.server().expect("server configured in the builder");

    let before = world.exec(&["codex", "mcp", "get", "converge"]).await?;
    assert_ne!(
        before.exit_code, 0,
        "the agent HOME already knew a converge MCP server\nstdout:\n{}\nstderr:\n{}",
        before.stdout, before.stderr
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

    // Read it back through codex itself: this is what proves the TOML we
    // wrote is the TOML codex accepts, rather than that we can read our
    // own file. `bearer_token` parsed fine as a key and was then rejected
    // for streamable HTTP — exactly the class of bug only codex can catch.
    let mcp = world.exec(&["codex", "mcp", "get", "converge"]).await?;
    assert_eq!(
        mcp.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout, mcp.stderr
    );
    assert!(
        mcp.stdout.contains("transport: streamable_http"),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );
    assert!(
        mcp.stdout.contains(&format!("url: {}/mcp", server.url())),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );
    // Codex redacts header values, so the assertion is that the header is
    // known at all — the token itself is checked in the unit tests.
    assert!(
        mcp.stdout.contains("http_headers: Authorization="),
        "stdout:\n{}\nstderr:\n{}",
        mcp.stdout,
        mcp.stderr
    );

    Ok(())
}
