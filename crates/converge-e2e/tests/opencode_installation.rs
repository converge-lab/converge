pub mod support;

use anyhow::Result;
use serde_json::Value;
use support::{OPENCODE_VERSION, OpenCode, Server, TestWorld};

#[tokio::test]
async fn opencode_init_installs_the_plugin_into_a_clean_home() -> Result<()> {
    let world = TestWorld::builder(OpenCode)
        .with_server(Server::default())
        .start()
        .await?;
    let server = world.server().expect("server configured in the builder");

    let version = world.exec(&["opencode", "--version"]).await?;
    assert_eq!(
        version.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        version.stdout, version.stderr
    );
    assert!(
        version.stdout.contains(OPENCODE_VERSION) || version.stderr.contains(OPENCODE_VERSION),
        "expected opencode {OPENCODE_VERSION}\nstdout:\n{}\nstderr:\n{}",
        version.stdout,
        version.stderr
    );

    let before = world
        .exec(&[
            "sh",
            "-c",
            r#"test ! -e "$HOME/.config/opencode/plugin/converge.js""#,
        ])
        .await?;
    assert_eq!(
        before.exit_code, 0,
        "the agent HOME already had a converge plugin\nstdout:\n{}\nstderr:\n{}",
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
    // "plugin", not "hooks" — the noun is the harness's, and this is the
    // one that proves `install` stopped meaning "write a hooks file".
    assert!(
        init.stdout.contains("plugin: installed"),
        "stdout:\n{}\nstderr:\n{}",
        init.stdout,
        init.stderr
    );

    let shim = world
        .exec(&[
            "sh",
            "-c",
            r#"cat "$HOME/.config/opencode/plugin/converge.js""#,
        ])
        .await?;
    assert_eq!(
        shim.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        shim.stdout, shim.stderr
    );
    // The absolute path is baked in at install time; a leftover
    // placeholder would make every hook call fail silently.
    assert!(
        shim.stdout
            .contains(r#"const CONVERGE = "/usr/local/bin/converge""#),
        "the shim kept its placeholder:\n{}",
        shim.stdout
    );
    assert!(!shim.stdout.contains("__CONVERGE_BIN__"));
    for callback in [
        "experimental.chat.system.transform",
        "tool.execute.before",
        "tool.execute.after",
    ] {
        assert!(shim.stdout.contains(callback), "shim lost {callback}");
    }

    // opencode has to be able to load it: a syntax error here would only
    // ever surface as "converge silently does nothing".
    let parses = world
        .exec(&[
            "node",
            "--input-type=module",
            "-e",
            "import(process.env.HOME + '/.config/opencode/plugin/converge.js')\
             .then(m => { if (typeof m.server !== 'function') { process.exit(3) } })\
             .catch(e => { console.error(e); process.exit(4) })",
        ])
        .await?;
    assert_eq!(
        parses.exit_code, 0,
        "the installed plugin does not load\nstdout:\n{}\nstderr:\n{}",
        parses.stdout, parses.stderr
    );

    Ok(())
}

#[tokio::test]
async fn opencode_init_registers_mcp_server() -> Result<()> {
    let world = TestWorld::builder(OpenCode)
        .with_server(Server::default())
        .start()
        .await?;
    let server = world.server().expect("server configured in the builder");

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

    let config = world
        .exec(&["sh", "-c", r#"cat "$HOME/.config/opencode/opencode.json""#])
        .await?;
    assert_eq!(
        config.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        config.stdout, config.stderr
    );
    let doc: Value = serde_json::from_str(&config.stdout)?;
    assert_eq!(doc["mcp"]["converge"]["type"], "remote");
    assert_eq!(
        doc["mcp"]["converge"]["url"],
        format!("{}/mcp", server.url())
    );
    assert_eq!(doc["mcp"]["converge"]["enabled"], true);
    assert_eq!(
        doc["mcp"]["converge"]["headers"]["Authorization"],
        format!("Bearer {}", server.token())
    );

    // And opencode agrees it is a server it can reach — the assertion
    // that tests opencode's schema rather than our own round trip.
    let listed = world.exec(&["opencode", "mcp", "list"]).await?;
    assert_eq!(
        listed.exit_code, 0,
        "stdout:\n{}\nstderr:\n{}",
        listed.stdout, listed.stderr
    );
    assert!(
        listed.stdout.contains("converge"),
        "stdout:\n{}\nstderr:\n{}",
        listed.stdout,
        listed.stderr
    );

    Ok(())
}
