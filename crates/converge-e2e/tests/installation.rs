//! `converge init` on a clean home, per agent tool: the hooks or plugin
//! it writes, read back through the tool where the tool can, and the
//! MCP registration the tool itself accepts.

mod commands;

use anyhow::{Context, Result};
use converge_e2e::agent::Agent;
use converge_e2e::server::{Database, Server};
use converge_e2e::world::{RunningWorld, TestWorld};
use serde_json::Value;

const CLAUDE_CODE_VERSION: &str = "2.1.220";
const CODEX_VERSION: &str = "0.154.0";
const OPENCODE_VERSION: &str = "1.18.31";

/// The five registrations, per event: the matcher and the subcommand.
const HOOKS: [(&str, Option<&str>, &str); 5] = [
    ("SessionStart", None, "inject"),
    ("SessionEnd", None, "sync"),
    ("PreToolUse", Some("mcp__converge__"), "ctx"),
    (
        "PostToolUse",
        Some("mcp__converge__(project_bind|project_dismiss)"),
        "mark",
    ),
    ("UserPromptSubmit", None, "poll"),
];

async fn world(agent: Agent) -> Result<RunningWorld> {
    TestWorld::new(agent)
        .with_server(Server::new(Database))
        .start()
        .await
}

/// `converge init` on the pipe: credentials answered, the MCP question
/// taken at its default, every tool found gets wired.
async fn init(world: &RunningWorld) -> Result<String> {
    let server = world
        .server()
        .context("the world was built with a server")?;
    let init = world
        .run(&commands::converge::init(server.url(), server.token()))
        .await?;
    assert!(init.succeeded(), "{init:?}");
    Ok(init.stdout)
}

async fn json_file(world: &RunningWorld, path: &str) -> Result<Value> {
    let read = world
        .run(&commands::sh(&format!(r#"cat "{path}""#)))
        .await?;
    assert!(read.succeeded(), "{path}: {read:?}");
    serde_json::from_str(&read.stdout).with_context(|| format!("{path} is not JSON"))
}

/// One group per event, ours, with the matcher where wanted.
fn assert_hooks(file: &Value, suffix: &str) {
    for (event, matcher, sub) in HOOKS {
        let groups = file["hooks"][event].as_array().map(Vec::len);
        assert_eq!(groups, Some(1), "{event}: {file}");
        let group = &file["hooks"][event][0];
        assert_eq!(group["hooks"][0]["type"], "command", "{event}");
        assert_eq!(
            group["hooks"][0]["command"],
            format!("{} hook {sub}{suffix}", commands::converge::BIN),
            "{event}"
        );
        match matcher {
            Some(matcher) => assert_eq!(group["matcher"], matcher, "{event}"),
            None => assert!(group["matcher"].is_null(), "{event} carries no matcher"),
        }
    }
}

#[tokio::test]
async fn claude_code_gets_the_five_hooks() -> Result<()> {
    let world = world(Agent::claude_code(CLAUDE_CODE_VERSION)).await?;
    init(&world).await?;
    let settings = json_file(&world, "$HOME/.claude/settings.json").await?;
    assert_hooks(&settings, "");
    world.stop().await?;
    Ok(())
}

#[tokio::test]
async fn codex_cli_gets_the_five_hooks_and_an_mcp_server_it_accepts() -> Result<()> {
    let world = world(Agent::codex_cli(CODEX_VERSION)).await?;

    let version = world.run(&commands::codex::version()).await?;
    assert!(
        version.succeeded()
            && (version.stdout.contains(CODEX_VERSION) || version.stderr.contains(CODEX_VERSION)),
        "expected Codex CLI {CODEX_VERSION}: {version:?}"
    );
    let before = world.run(&commands::codex::get_mcp("converge")).await?;
    assert!(
        !before.succeeded(),
        "the agent home already knew a converge MCP server: {before:?}"
    );

    let stdout = init(&world).await?;
    // The trust gate is the one thing a Codex user must do by hand; if
    // this line ever stops printing, the integration silently does
    // nothing on their machine.
    assert!(
        stdout.contains("/hooks"),
        "init never mentioned trusting the hooks:\n{stdout}"
    );

    let hooks = json_file(&world, "$HOME/.codex/hooks.json").await?;
    assert_hooks(&hooks, " --harness codex");

    // Read back through codex itself: this proves the TOML we wrote is
    // the TOML codex accepts, not that we can read our own file.
    let mcp = world.run(&commands::codex::get_mcp("converge")).await?;
    assert!(mcp.succeeded(), "{mcp:?}");
    assert!(mcp.stdout.contains("transport: streamable_http"), "{mcp:?}");

    world.stop().await?;
    Ok(())
}

#[tokio::test]
async fn opencode_gets_the_plugin_and_an_mcp_server() -> Result<()> {
    let world = world(Agent::opencode(OPENCODE_VERSION)).await?;

    let version = world.run(&commands::opencode::version()).await?;
    assert!(
        version.succeeded()
            && (version.stdout.contains(OPENCODE_VERSION)
                || version.stderr.contains(OPENCODE_VERSION)),
        "expected opencode {OPENCODE_VERSION}: {version:?}"
    );

    let stdout = init(&world).await?;
    // "plugin", not "hooks": the noun is the harness's own.
    assert!(stdout.contains("plugin: installed"), "{stdout}");

    let shim = world
        .run(&commands::sh(
            r#"cat "$HOME/.config/opencode/plugin/converge.js""#,
        ))
        .await?;
    assert!(shim.succeeded(), "{shim:?}");
    // The absolute path is baked in at install time; a leftover
    // placeholder would make every hook call fail silently.
    assert!(
        shim.stdout.contains(&format!(
            r#"const CONVERGE = "{}""#,
            commands::converge::BIN
        )),
        "the shim kept its placeholder:\n{}",
        shim.stdout
    );
    for callback in [
        "experimental.chat.system.transform",
        "chat.message",
        "tool.execute.before",
        "tool.execute.after",
    ] {
        assert!(shim.stdout.contains(callback), "shim lost {callback}");
    }
    let loads = world.run(&commands::opencode::load_plugin()).await?;
    assert!(
        loads.succeeded(),
        "the installed plugin does not load: {loads:?}"
    );

    let server = world
        .server()
        .context("the world was built with a server")?;
    let config = json_file(&world, "$HOME/.config/opencode/opencode.json").await?;
    assert_eq!(config["mcp"]["converge"]["type"], "remote", "{config}");
    assert_eq!(
        config["mcp"]["converge"]["url"],
        format!("{}/mcp", server.url()),
        "{config}"
    );
    assert_eq!(config["mcp"]["converge"]["enabled"], true, "{config}");

    world.stop().await?;
    Ok(())
}
