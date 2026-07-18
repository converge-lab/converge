//! The scoreboard: recorded runs aggregated into one [`Summary`] — a plain
//! data structure rendered by `tabled`, never assembled by hand. The same
//! structure serializes to `summary.json` for machine comparison between
//! runs.

use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use tabled::Table;
use tabled::builder::Builder;

use crate::models::Model;
use crate::prompts::Prompt;
use crate::run::Record;
use crate::scenario::Scenario;

/// The whole scoreboard, participant order preserved.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub runs: usize,
    pub errors: usize,
    /// Scenario names, in column order.
    pub scenarios: Vec<String>,
    pub rows: Vec<SummaryRow>,
}

/// One (participant × prompt) line.
#[derive(Debug, Serialize)]
pub struct SummaryRow {
    pub tier: String,
    pub model: String,
    pub prompt: String,
    /// Ordered as [`Summary::scenarios`].
    pub scenarios: Vec<CellOutcome>,
    /// Average over runs that answered (errors excluded).
    pub avg_output_tokens: u64,
    /// Average over all runs, errors included.
    pub avg_seconds: f64,
}

/// One (line × scenario) cell.
#[derive(Debug, Serialize)]
pub struct CellOutcome {
    pub name: String,
    pub ok: u32,
    pub total: u32,
    pub errors: u32,
}

impl fmt::Display for CellOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.ok, self.total)?;
        if self.errors > 0 {
            write!(f, " e{}", self.errors)?;
        }
        Ok(())
    }
}

impl Summary {
    /// Aggregate records into the scoreboard.
    pub fn build(
        models: &[Arc<Model>],
        prompts: &[Arc<Prompt>],
        scenarios: &[Arc<Scenario>],
        records: &[Record],
    ) -> Self {
        let mut rows = Vec::new();
        for model in models {
            for prompt in prompts {
                let mine: Vec<&Record> = records
                    .iter()
                    .filter(|r| r.model == model.label && r.prompt == prompt.name)
                    .collect();
                if mine.is_empty() {
                    continue;
                }
                let cells = scenarios
                    .iter()
                    .map(|scenario| {
                        let cell: Vec<&&Record> = mine
                            .iter()
                            .filter(|r| r.scenario == scenario.name)
                            .collect();
                        CellOutcome {
                            name: scenario.name.clone(),
                            ok: cell
                                .iter()
                                .filter(|r| r.score.is_some_and(|s| s.ok))
                                .count() as u32,
                            total: cell.len() as u32,
                            errors: cell.iter().filter(|r| r.error.is_some()).count() as u32,
                        }
                    })
                    .collect();
                let answered: Vec<&&Record> = mine.iter().filter(|r| r.error.is_none()).collect();
                let avg_output_tokens =
                    answered.iter().filter_map(|r| r.output_tokens).sum::<u64>()
                        / answered.len().max(1) as u64;
                let avg_seconds = mine.iter().map(|r| r.duration_ms as f64).sum::<f64>()
                    / 1000.0
                    / mine.len() as f64;
                rows.push(SummaryRow {
                    tier: model.tier.clone(),
                    model: model.label.clone(),
                    prompt: prompt.name.clone(),
                    scenarios: cells,
                    avg_output_tokens,
                    avg_seconds,
                });
            }
        }
        Summary {
            runs: records.len(),
            errors: records.iter().filter(|r| r.error.is_some()).count(),
            scenarios: scenarios.iter().map(|s| s.name.clone()).collect(),
            rows,
        }
    }

    /// The scoreboard as a table, ready for a `tabled` style.
    pub fn table(&self) -> Table {
        let mut builder = Builder::default();
        let mut header = vec![
            "tier".to_string(),
            "model".to_string(),
            "prompt".to_string(),
        ];
        header.extend(self.scenarios.iter().cloned());
        header.extend(["out tok".to_string(), "s/run".to_string()]);
        builder.push_record(header);
        for row in &self.rows {
            let mut record = vec![row.tier.clone(), row.model.clone(), row.prompt.clone()];
            record.extend(row.scenarios.iter().map(ToString::to_string));
            record.push(row.avg_output_tokens.to_string());
            record.push(format!("{:.1}", row.avg_seconds));
            builder.push_record(record);
        }
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Record;
    use crate::scenario::Score;

    fn record(model: &str, scenario: &str, ok: Option<bool>, tokens: u64) -> Record {
        Record {
            scenario: scenario.into(),
            model: model.into(),
            model_id: model.into(),
            tier: "cheap".into(),
            prompt: "baseline_v1".into(),
            rep: 1,
            started_at: String::new(),
            duration_ms: 2000,
            score: ok.map(|ok| Score {
                ok,
                expected_found: ok,
                risk_matches: ok,
                extra_signals: 0,
                signals_total: 0,
            }),
            input_tokens: Some(1),
            output_tokens: Some(tokens),
            signals: None,
            error: ok.is_none().then(|| "boom".into()),
        }
    }

    #[test]
    fn aggregates_cells_errors_and_averages() {
        let models = vec![Arc::new(
            toml::from_str::<Model>(
                r#"label = "m1"
id = "x/m1"
tier = "cheap""#,
            )
            .unwrap(),
        )];
        let prompts = vec![Arc::new(Prompt {
            name: "baseline_v1".into(),
            text: "p".into(),
        })];
        let scenarios: Vec<Arc<Scenario>> = ["find_signal", "stay_silent"]
            .iter()
            .map(|name| {
                Arc::new(
                    toml::from_str::<Scenario>(&format!("name = \"{name}\"\nfixture = \"f.json\""))
                        .unwrap(),
                )
            })
            .collect();
        let records = vec![
            record("m1", "find_signal", Some(true), 100),
            record("m1", "find_signal", Some(false), 300),
            record("m1", "stay_silent", None, 0),
        ];
        let summary = Summary::build(&models, &prompts, &scenarios, &records);
        assert_eq!((summary.runs, summary.errors), (3, 1));
        let row = &summary.rows[0];
        assert_eq!(row.prompt, "baseline_v1");
        assert_eq!(row.scenarios[0].to_string(), "1/2");
        assert_eq!(row.scenarios[1].to_string(), "0/1 e1");
        assert_eq!(row.avg_output_tokens, 200);

        let table = summary.table().to_string();
        assert!(table.contains("find_signal") && table.contains("m1") && table.contains("prompt"));
    }
}
