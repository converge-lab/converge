mod commands;

use anyhow::{Context, Result};
use serde_json::Value;
use staw_e2e::agent::Agent;
use staw_e2e::model::{Model, Reply};
use staw_e2e::server::{Database, Server};
use staw_e2e::world::TestWorld;

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

    let staw = world.run(&commands::staw::version()).await?;
    assert!(staw.succeeded(), "{staw:?}");

    let server = world
        .server()
        .context("the world was built with a server")?;
    let init = world
        .run(&commands::staw::init(server.url(), server.token()))
        .await?;
    assert!(init.succeeded());

    let mcp = world.run(&commands::claude::get_mcp("staw")).await?;
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
        .find(|turn| offers_staw_tools(turn))
        .context("no turn offered Staw's tools — MCP never reached the model")?;

    assert!(
        conversation.to_string().contains("SessionStart hook"),
        "the SessionStart hook's context never reached the model"
    );

    world.stop().await?;
    Ok(())
}

fn offers_staw_tools(turn: &Value) -> bool {
    turn["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str())
        .any(|name| name.starts_with("mcp__staw__"))
}
