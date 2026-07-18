//! The comparison runner: scenarios × models × prompts through the
//! production [`Expert`], results saved to disk.
//!
//! ```sh
//! cargo run -p converge-comparison -- [--models a,b] [--scenarios x] \
//!     [--prompts p] [--reps N] [--concurrency N] [--out DIR]
//! ```
//!
//! Participants come from `models/models.toml`, scenarios from
//! `scenarios/scenarios.toml`, prompt variants from `prompts/prompts.toml`
//! (all beside this crate's `src/`). Each run writes a timestamped
//! directory with `results.jsonl` (one line per call, appended as results
//! arrive), `summary.md`, `summary.json`, and `meta.json`. Model failures
//! are recorded, never abort the run.

mod models;
mod prompts;
mod report;
mod run;
mod scenario;

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Parser;
use converge_expert::Expert;
use converge_expert::signals::Request;
use futures::StreamExt;
use tabled::settings::Style;

use models::Model;
use prompts::Prompt;
use run::{Job, Record};
use scenario::Scenario;

/// The crate's own data directory — the comparison is self-contained.
pub(crate) fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[derive(Parser)]
#[command(about = "Compare models on the signal expert's scenarios")]
struct Cli {
    /// Run only these model labels.
    #[arg(long, value_delimiter = ',')]
    models: Vec<String>,
    /// Run only these scenario names.
    #[arg(long, value_delimiter = ',')]
    scenarios: Vec<String>,
    /// Run only these prompt names.
    #[arg(long, value_delimiter = ',')]
    prompts: Vec<String>,
    /// Repetitions per (model, scenario, prompt) cell.
    #[arg(long, default_value_t = 1)]
    reps: u32,
    /// Concurrent model calls.
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    /// Output directory; each run gets a timestamped subdirectory.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = data_dir();

    let mut models = models::load(&dir.join("models/models.toml"))?;
    if !cli.models.is_empty() {
        models.retain(|model| cli.models.contains(&model.label));
    }
    let mut scenarios = scenario::load(&dir.join("scenarios/scenarios.toml"))?;
    if !cli.scenarios.is_empty() {
        scenarios.retain(|scenario| cli.scenarios.contains(&scenario.name));
    }
    let mut prompts = prompts::load(&dir.join("prompts/prompts.toml"))?;
    if !cli.prompts.is_empty() {
        prompts.retain(|prompt| cli.prompts.contains(&prompt.name));
    }
    if models.is_empty() || scenarios.is_empty() || prompts.is_empty() {
        bail!("nothing to run after filters — check --models/--scenarios/--prompts");
    }

    let models: Vec<Arc<Model>> = models.into_iter().map(Arc::new).collect();
    let prompts: Vec<Arc<Prompt>> = prompts.into_iter().map(Arc::new).collect();
    let fixtures = dir.join("fixtures");
    let scenarios: Vec<(Arc<Scenario>, Arc<Request>)> = scenarios
        .into_iter()
        .map(|scenario| {
            let request = scenario.request(&fixtures)?;
            Ok((Arc::new(scenario), Arc::new(request)))
        })
        .collect::<anyhow::Result<_>>()?;

    // One expert per (model × prompt) — the exact configuration production
    // would ship with that pair.
    let mut experts = Vec::new();
    for model in &models {
        for prompt in &prompts {
            let expert = Expert::new(model.config(prompt.text.clone())).with_context(|| {
                format!("configure the expert for {} × {}", model.label, prompt.name)
            })?;
            experts.push((Arc::clone(model), Arc::clone(prompt), expert));
        }
    }

    let mut jobs = Vec::new();
    for (model, prompt, expert) in &experts {
        for (scenario, request) in &scenarios {
            for rep in 1..=cli.reps {
                jobs.push(Job {
                    model: Arc::clone(model),
                    expert: expert.clone(),
                    scenario: Arc::clone(scenario),
                    prompt: Arc::clone(prompt),
                    request: Arc::clone(request),
                    rep,
                });
            }
        }
    }

    let out = cli.out.unwrap_or_else(|| dir.join("results"));
    let run_dir = out.join(run::now_rfc3339().replace(['-', ':'], ""));
    fs::create_dir_all(&run_dir).with_context(|| format!("create {}", run_dir.display()))?;
    fs::write(
        run_dir.join("meta.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "started_at": run::now_rfc3339(),
            "git_commit": git_commit(),
            "models": models.iter().map(|m| m.label.clone()).collect::<Vec<_>>(),
            "scenarios": scenarios.iter().map(|(s, _)| s.name.clone()).collect::<Vec<_>>(),
            "prompts": prompts.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            "reps": cli.reps,
            "concurrency": cli.concurrency,
        }))?,
    )?;

    let total = jobs.len();
    println!(
        "{total} runs ({} models × {} scenarios × {} prompts × {} reps) → {}",
        models.len(),
        scenarios.len(),
        prompts.len(),
        cli.reps,
        run_dir.display()
    );

    let mut results = fs::File::create(run_dir.join("results.jsonl"))?;
    let mut records: Vec<Record> = Vec::with_capacity(total);
    let mut stream = futures::stream::iter(jobs.into_iter().map(run::execute))
        .buffer_unordered(cli.concurrency.max(1));
    while let Some(record) = stream.next().await {
        writeln!(results, "{}", serde_json::to_string(&record)?)?;
        results.flush()?;
        println!(
            "[{}/{total}] {} × {} × {} → {}",
            records.len() + 1,
            record.model,
            record.scenario,
            record.prompt,
            verdict(&record)
        );
        records.push(record);
    }

    let scenario_list: Vec<Arc<Scenario>> = scenarios.iter().map(|(s, _)| Arc::clone(s)).collect();
    let summary = report::Summary::build(&models, &prompts, &scenario_list, &records);
    fs::write(
        run_dir.join("summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )?;
    let mut markdown = summary.table();
    markdown.with(Style::markdown());
    fs::write(
        run_dir.join("summary.md"),
        format!(
            "# Signal expert comparison\n\n{} runs, {} errors\n\n{markdown}\n",
            summary.runs, summary.errors
        ),
    )?;
    let mut terminal = summary.table();
    terminal.with(Style::sharp());
    println!("\n{terminal}");
    println!("results: {}", run_dir.display());
    Ok(())
}

fn verdict(record: &Record) -> String {
    match (&record.score, &record.error) {
        (Some(score), _) if score.ok => "ok".into(),
        (Some(score), _) => format!(
            "FAIL (found={} risk={} extra={})",
            score.expected_found, score.risk_matches, score.extra_signals
        ),
        (None, Some(error)) => {
            let short: String = error.chars().take(140).collect();
            let ellipsis = if short.len() < error.len() { "…" } else { "" };
            format!("ERROR {short}{ellipsis}")
        }
        (None, None) => "ERROR empty".into(),
    }
}

fn git_commit() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
