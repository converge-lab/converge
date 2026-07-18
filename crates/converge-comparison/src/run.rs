//! One comparison cell: call the production expert, score, record.

use std::sync::Arc;
use std::time::Instant;

use converge_expert::Expert;
use converge_expert::signals::Request;
use serde::Serialize;
use serde_json::Value;

use crate::models::Model;
use crate::prompts::Prompt;
use crate::scenario::{Scenario, Score};

/// One (model × scenario × prompt × repetition) cell.
pub struct Job {
    pub model: Arc<Model>,
    pub expert: Expert,
    pub scenario: Arc<Scenario>,
    pub prompt: Arc<Prompt>,
    pub request: Arc<Request>,
    pub rep: u32,
}

/// Everything recorded about one run — one line of `results.jsonl`.
#[derive(Debug, Serialize)]
pub struct Record {
    pub scenario: String,
    pub model: String,
    pub model_id: String,
    pub tier: String,
    pub prompt: String,
    pub rep: u32,
    pub started_at: String,
    pub duration_ms: u128,
    pub score: Option<Score>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The raw signals for manual review — wording is unscored but telling.
    pub signals: Option<Value>,
    pub error: Option<String>,
}

/// Execute one cell. Model failures land in the record, never abort a run.
pub async fn execute(job: Job) -> Record {
    let started_at = now_rfc3339();
    let started = Instant::now();
    let outcome = job.expert.signals(&job.request).await;
    let mut record = Record {
        scenario: job.scenario.name.clone(),
        model: job.model.label.clone(),
        model_id: job.model.id.clone(),
        tier: job.model.tier.clone(),
        prompt: job.prompt.name.clone(),
        rep: job.rep,
        started_at,
        duration_ms: started.elapsed().as_millis(),
        score: None,
        input_tokens: None,
        output_tokens: None,
        signals: None,
        error: None,
    };
    match outcome {
        Ok(response) => {
            record.score = Some(job.scenario.score(&response.signals));
            record.input_tokens = Some(response.meta.input_tokens);
            record.output_tokens = Some(response.meta.output_tokens);
            record.signals = serde_json::to_value(&response.signals).ok();
        }
        Err(error) => record.error = Some(error_chain(&error)),
    }
    record
}

/// Display an error with its full source chain.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("zero nanoseconds is valid")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("UTC timestamps format as RFC3339")
}
