//! `/api/v1/expert` — grounded chat over the group's decision memory,
//! streamed as SSE.
//!
//! One stateless POST: the question and the client-held history in, an
//! event stream out — `context` (what the answer grounds in) first,
//! `delta` text chunks, then `done`; a mid-stream model failure arrives
//! as an `error` event (the HTTP status is already committed by then).
//! Setup failures (invisible group, no `ask` job configured) fail the
//! request itself through the normal error envelope.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::post;
use axum::{Extension, Json, Router};
use converge_expert::Turn;
use converge_storage::{GroupId, Scope, Storage};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;

use super::error::Result;
use crate::auth::Caller;
use crate::expert::Expert;

pub fn routes<S: Storage + 'static>() -> Router<(S, Expert<S>)> {
    Router::new().route("/api/v1/expert/ask", post(ask::<S>))
}

#[derive(Deserialize)]
struct Ask {
    /// The group whose memory answers.
    group_id: GroupId,
    question: String,
    /// Prior turns, client-held (the server keeps no conversations).
    #[serde(default)]
    history: Vec<WireTurn>,
}

#[derive(Deserialize)]
struct WireTurn {
    /// `true` = the asking user, `false` = the expert's earlier answer.
    user: bool,
    text: String,
}

async fn ask<S: Storage + 'static>(
    State((_store, expert)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Json(req): Json<Ask>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let history = req
        .history
        .into_iter()
        .map(|t| Turn {
            user: t.user,
            text: t.text,
        })
        .collect();
    let (briefing, answer) = expert
        .ask(
            Scope::User(caller.user),
            req.group_id,
            history,
            &req.question,
        )
        .await?;

    let head = futures::stream::once(async move {
        Ok(Event::default().event("context").data(
            json!({
                "decisions": briefing.decisions,
                "signals": briefing.signals,
            })
            .to_string(),
        ))
    });
    let deltas = answer.map(|chunk| {
        Ok(match chunk {
            // JSON-wrapped so newlines survive the SSE framing.
            Ok(text) => Event::default()
                .event("delta")
                .data(json!({ "text": text }).to_string()),
            Err(e) => Event::default()
                .event("error")
                .data(json!({ "message": e.to_string() }).to_string()),
        })
    });
    let done = futures::stream::once(async { Ok(Event::default().event("done").data("{}")) });

    Ok(Sse::new(head.chain(deltas).chain(done)).keep_alive(KeepAlive::default()))
}
