mod commands;

use anyhow::{Context, Result};
use converge_e2e::agent::Agent;
use converge_e2e::command::Outcome;
use converge_e2e::model::{Model, Reply};
use converge_e2e::server::{self, Database, Server};
use converge_e2e::world::TestWorld;

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

    let before = world.run(&commands::converge::version()).await?;
    assert_eq!(before.outcome, Outcome::NotFound);

    let install = world.run(&commands::converge::install()).await?;
    assert!(install.succeeded());

    let after = world.run(&commands::converge::version()).await?;
    assert!(after.succeeded());

    let server = world
        .server()
        .context("the world was built with a server")?;
    let init = world
        .run(&commands::converge::init(server.url(), server::TOKEN))
        .await?;
    assert!(init.succeeded());

    let mcp = world.run(&commands::claude::get_mcp("converge")).await?;
    dbg!(&mcp);

    let model = world.model().context("the world was built with a model")?;
    model.always(Reply::Text("done".to_owned())).await?;

    let session = world
        .run(&commands::claude::session("/workspace", "hello", &[]))
        .await?;
    assert!(session.succeeded());
    assert!(session.stdout.contains("done"));

    let turns = model.turns().await?;
    dbg!(&turns);
    assert!(!turns.is_empty(), "the agent never reached the model");

    world.stop().await?;
    Ok(())
}
