//! Prometheus metrics: what a scraper on the host can ask this server.
//!
//! Call sites use the `metrics` facade (`histogram!`, `counter!`) and
//! never name Prometheus; the exporter is installed once at startup and
//! serves `/metrics` on its own listener, so the app's router and the
//! reverse proxy never see it. Two rules hold everywhere:
//!
//! - **No id is ever a label.** Project, decision, user, session ids go in
//!   the structured logs. Labels are closed sets this module owns: tool
//!   names from a list the router is tested against, route *templates*
//!   from the router, HTTP methods from a list, outcomes, tiers and
//!   sources from enums. A client cannot mint a label by sending a novel
//!   string.
//! - **Durations are seconds**, on one bucket ladder, so every
//!   `_seconds` histogram answers the same `histogram_quantile`.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use futures::Stream;
use metrics::{
    Unit, counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram,
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use tracing::warn;

/// The bucket ladder for every `_seconds` histogram. A millisecond floor
/// so tool calls and HTTP requests resolve; dense between one and
/// sixteen seconds, where a model answers and where a delivery deadline
/// would sit — 8 s is an edge on purpose, so "how often does detection
/// overrun it" is one bucket read; sparse above.
pub const SECONDS_BUCKETS: [f64; 21] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0,
    8.0, 12.0, 16.0, 32.0, 64.0,
];

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the Prometheus exporter with its own listener on `listen`.
/// Once per process — a second call is a no-op, not a second socket —
/// and inside the Tokio runtime. The listener carries no authentication,
/// so anything but loopback is loudly noted.
pub fn install(listen: SocketAddr) -> anyhow::Result<()> {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    if !listen.ip().is_loopback() {
        warn!(%listen, "metrics listener is not on loopback and has no authentication");
    }
    PrometheusBuilder::new()
        .with_http_listener(listen)
        .set_buckets_for_metric(Matcher::Suffix("_seconds".into()), &SECONDS_BUCKETS)?
        .install()?;
    describe();
    Ok(())
}

/// Help text and units for every metric, and the series that should
/// exist from boot: a counter that has never been incremented is absent
/// from a scrape, and an absent series looks like a broken exporter to a
/// dashboard. Also called by the test recorder.
pub fn describe() {
    describe_histogram!(
        "converge_signal_detection_seconds",
        Unit::Seconds,
        "One signal-detection pass: phase=total is the whole pass, phase=model the model call alone; source=live for decisions as they land, backfill for the day-one sweep"
    );
    describe_counter!(
        "converge_signal_drafts_total",
        "Signal drafts the expert produced, by tier and what became of them: written, duplicate (the re-raise ban), rejected (storage refused it)"
    );
    describe_histogram!(
        "converge_mcp_tool_seconds",
        Unit::Seconds,
        "One MCP tool call, by tool; outcome=error covers both transport errors and results the tool marked as errors"
    );
    describe_counter!(
        "converge_decisions_recorded_total",
        "Decisions recorded, by the door they came in through"
    );
    describe_counter!(
        "converge_evidence_messages_total",
        "Evidence messages accepted, by the door they came in through"
    );
    describe_histogram!(
        "converge_expert_ask_seconds",
        Unit::Seconds,
        "An expert question, from the request: phase=prepare until the briefing is built and the model request dispatched, first_token until the first streamed chunk, total until the stream ends; outcome=aborted when the client left first"
    );
    describe_histogram!(
        "converge_http_request_seconds",
        Unit::Seconds,
        "One HTTP request, by method, matched route template and status"
    );
    describe_gauge!(
        "converge_build_info",
        "Always 1; the version label says which build is serving"
    );
    describe_gauge!(
        "converge_process_start_time_seconds",
        Unit::Seconds,
        "When this process started, as unix seconds"
    );
    for via in ["mcp", "rest"] {
        counter!("converge_decisions_recorded_total", "via" => via).increment(0);
        counter!("converge_evidence_messages_total", "via" => via).increment(0);
    }
    gauge!("converge_build_info", "version" => env!("CARGO_PKG_VERSION")).set(1.0);
    gauge!("converge_process_start_time_seconds").set(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0),
    );
}

/// One histogram per HTTP request, labelled by method, the route
/// *template* the router matched (`/api/v1/decisions/{id}`, never the
/// path with the id in it) and the status. Under the one nested service,
/// `/mcp`, axum does not set `MatchedPath`; those requests are labelled
/// by the prefix. Requests nothing matched — the SPA fallback, 404s —
/// share one `unmatched` route.
pub async fn http(req: Request, next: Next) -> Response {
    let method = method_label(req.method().as_str());
    let route = match req.extensions().get::<MatchedPath>() {
        Some(matched) => matched.as_str().to_owned(),
        None if req.uri().path().starts_with("/mcp") => "/mcp".to_owned(),
        None => "unmatched".to_owned(),
    };
    let started = Instant::now();
    let response = next.run(req).await;
    histogram!(
        "converge_http_request_seconds",
        "method" => method,
        "route" => route,
        "status" => response.status().as_u16().to_string(),
    )
    .record(started.elapsed().as_secs_f64());
    response
}

/// HTTP methods are client-controlled text; only the ones we route on
/// become labels.
fn method_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        "HEAD" => "HEAD",
        _ => "other",
    }
}

/// The MCP tools this server has. A client naming anything else is
/// timed under `other`, so the label set is ours, not the client's. A
/// test in `mcp` walks the real router so a tool added there without an
/// arm here fails the build, not the dashboard.
pub fn tool_label(name: &str) -> &'static str {
    match name {
        "project_list" => "project_list",
        "project_match" => "project_match",
        "project_bind" => "project_bind",
        "project_dismiss" => "project_dismiss",
        "group_add" => "group_add",
        "session_ensure" => "session_ensure",
        "message_add" => "message_add",
        "decision_add" => "decision_add",
        "decision_get" => "decision_get",
        "decision_list" => "decision_list",
        "decision_search" => "decision_search",
        "signal_list" => "signal_list",
        "signal_resolve" => "signal_resolve",
        _ => "other",
    }
}

/// One MCP tool call, timed. The agent's view of the server: this is
/// where a deliberately slow `decision_add` will show up the day it ships.
pub fn tool_call(tool: &'static str, ok: bool, started: Instant) {
    histogram!(
        "converge_mcp_tool_seconds",
        "tool" => tool,
        "outcome" => if ok { "ok" } else { "error" },
    )
    .record(started.elapsed().as_secs_f64());
}

/// A decision recorded, by the door it came in through.
pub fn decision_recorded(via: &'static str) {
    counter!("converge_decisions_recorded_total", "via" => via).increment(1);
}

/// Evidence messages accepted, by the door.
pub fn evidence_messages(via: &'static str, count: usize) {
    counter!("converge_evidence_messages_total", "via" => via).increment(count as u64);
}

/// One signal-detection pass: the whole pass and the model call alone,
/// as two phases of one histogram. `source` keeps the day-one backfill
/// out of the number a deadline is set from.
pub fn detection(
    total_secs: f64,
    model_secs: Option<f64>,
    outcome: &'static str,
    source: &'static str,
) {
    histogram!(
        "converge_signal_detection_seconds",
        "phase" => "total",
        "outcome" => outcome,
        "source" => source,
    )
    .record(total_secs);
    if let Some(model) = model_secs {
        histogram!(
            "converge_signal_detection_seconds",
            "phase" => "model",
            "outcome" => outcome,
            "source" => source,
        )
        .record(model);
    }
}

/// A signal draft the expert produced, by tier and what became of it.
pub fn signal_draft(tier: &'static str, outcome: &'static str) {
    counter!("converge_signal_drafts_total", "tier" => tier, "outcome" => outcome).increment(1);
}

/// One phase of an expert question, measured from the request.
pub fn expert_ask(phase: &'static str, outcome: &'static str, started: Instant) {
    histogram!(
        "converge_expert_ask_seconds",
        "phase" => phase,
        "outcome" => outcome,
    )
    .record(started.elapsed().as_secs_f64());
}

/// An expert answer stream, observed: the time to the first chunk and to
/// the end, with an outcome a mid-stream failure cannot hide behind the
/// 200 that was already sent. The model request is only dispatched when
/// this is first polled, which is why `prepare` alone says nothing about
/// the model.
pub struct AskStream<S> {
    inner: S,
    started: Instant,
    first: bool,
    errored: bool,
    done: bool,
}

impl<S> AskStream<S> {
    pub fn new(inner: S, started: Instant) -> Self {
        Self {
            inner,
            started,
            first: true,
            errored: false,
            done: false,
        }
    }
}

impl<S, T, E> Stream for AskStream<S>
where
    S: Stream<Item = Result<T, E>> + Unpin,
{
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(item)) => {
                if this.first {
                    this.first = false;
                    expert_ask(
                        "first_token",
                        if item.is_ok() { "ok" } else { "error" },
                        this.started,
                    );
                }
                if item.is_err() {
                    this.errored = true;
                }
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => {
                if !this.done {
                    this.done = true;
                    expert_ask(
                        "total",
                        if this.errored { "error" } else { "ok" },
                        this.started,
                    );
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for AskStream<S> {
    fn drop(&mut self) {
        // Dropped before the end: the client went away mid-answer.
        if !self.done {
            self.done = true;
            expert_ask("total", "aborted", self.started);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_closed_sets_and_the_ladder_is_sound() {
        assert_eq!(tool_label("decision_add"), "decision_add");
        assert_eq!(tool_label("drop_table"), "other");
        assert_eq!(method_label("GET"), "GET");
        assert_eq!(method_label("BREW"), "other");
        // The deadline is a bucket edge, so an overrun is one bucket read;
        // the floor resolves a tool call; the ladder is strictly rising.
        let ladder = SECONDS_BUCKETS.to_vec();
        assert!(ladder.contains(&8.0));
        assert!(ladder.first().is_some_and(|floor| *floor <= 0.001));
        assert!(ladder.windows(2).all(|w| w[0] < w[1]));
    }
}
