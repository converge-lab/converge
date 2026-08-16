mod commands;

use anyhow::{Context, Result};
use converge_e2e::agent::Agent;
use converge_e2e::model::{Model, Reply};
use converge_e2e::server::{self, Database, Server};
use converge_e2e::world::TestWorld;
use serde_json::Value;

const CLAUDE_CODE_VERSION: &str = "2.1.220";

#[tokio::test]
async fn the_happy_path() -> Result<()> {
    let world = TestWorld::new(Agent::claude_code(CLAUDE_CODE_VERSION))
        .with_server(Server::new(Database))
        .with_model(Model)
        .start()
        .await?;

    let claude = world.run(&commands::claude::version()).await?;
    assert!(claude.succeeded());

    let converge = world.run(&commands::converge::version()).await?;
    assert!(converge.succeeded(), "{converge:?}");

    let server = world
        .server()
        .context("the world was built with a server")?;
    let init = world
        .run(&commands::converge::init(server.url(), server::TOKEN))
        .await?;
    assert!(init.succeeded());

    let mcp = world.run(&commands::claude::get_mcp("converge")).await?;
    assert!(mcp.succeeded(), "{mcp:?}");
    assert!(mcp.stdout.contains("✔ Connected"), "{mcp:?}");

    let model = world.model().context("the world was built with a model")?;
    model.always(Reply::Text("done".to_owned())).await?;

    let session = world
        .run(&commands::claude::session("/workspace", "hello", &[]))
        .await?;
    assert!(session.succeeded());
    assert!(session.stdout.contains("done"));

    let turns = model.turns().await?;
    let conversation = turns
        .iter()
        .find(|turn| offers_converge_tools(turn))
        .context("no turn offered Converge's tools — MCP never reached the model")?;

    assert!(
        conversation.to_string().contains("SessionStart hook"),
        "the SessionStart hook's context never reached the model"
    );

    world.stop().await?;
    Ok(())
}

fn offers_converge_tools(turn: &Value) -> bool {
    turn["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str())
        .any(|name| name.starts_with("mcp__converge__"))
}
