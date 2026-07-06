//! `/api/v1/agents` — list only. Agents come into existence through
//! `ensure` on write paths (authorship stamping, the future MCP), never
//! through a REST create.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use converge_storage::{Agent, AgentId, Page, Pagination, Storage};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new().route("/api/v1/agents", get(list::<S>))
}

async fn list<S: Storage>(
    State(store): State<S>,
    Query(page): Query<Pagination<AgentId>>,
) -> Result<Json<Page<Agent>>> {
    let items = store.agent_list(page.clone()).await?;
    Ok(Json(Page::new(items, &page, |a| a.id.to_string())))
}
