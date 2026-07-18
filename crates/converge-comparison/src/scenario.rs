//! The scored scenarios and their deterministic oracle.
//!
//! Scenarios are data — `scenarios/scenarios.toml` beside this crate's
//! `src/` — and one
//! generic rule judges all of them: the response must contain exactly the
//! expected signals (matched by source, target set, and risk) and nothing
//! else; an empty expectation means the correct answer is silence. Titles
//! and texts are model wording and deliberately unscored.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use converge_expert::signals::Request;
use converge_storage::{DecisionId, Risk, Signal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct ScenariosFile {
    scenarios: Vec<Scenario>,
}

/// One scored scenario.
#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    /// Display name and the `--scenarios` filter key.
    pub name: String,
    /// File name under the crate's `fixtures/`.
    pub fixture: String,
    /// The exact signals the model must return; empty = silence is correct.
    #[serde(default)]
    pub expected: Vec<ExpectedSignal>,
}

/// One required signal. Ids and risk are typed — a typo in the scenario
/// file fails at parse time, not during scoring.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedSignal {
    pub source: DecisionId,
    pub targets: Vec<DecisionId>,
    pub risk: Risk,
}

/// Parse the scenario list.
pub fn load(path: &Path) -> anyhow::Result<Vec<Scenario>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read scenario list {}", path.display()))?;
    let file: ScenariosFile =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(file.scenarios)
}

/// The deterministic verdict for one response.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Score {
    /// The cell verdict.
    pub ok: bool,
    /// Every expected signal is present (vacuously true for silence).
    pub expected_found: bool,
    /// Every matched signal carries the expected risk.
    pub risk_matches: bool,
    /// Signals beyond the expectation.
    pub extra_signals: u32,
    pub signals_total: u32,
}

impl Scenario {
    /// Load and validate this scenario's request fixture.
    pub fn request(&self, fixtures: &Path) -> anyhow::Result<Request> {
        let path = fixtures.join(&self.fixture);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read fixture {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    /// Judge a response against the expectation.
    pub fn score(&self, signals: &[Signal]) -> Score {
        let mut matched = vec![false; signals.len()];
        let mut expected_found = true;
        let mut risk_matches = true;
        for expected in &self.expected {
            let want: HashSet<DecisionId> = expected.targets.iter().copied().collect();
            let hit = signals.iter().enumerate().find(|(index, signal)| {
                !matched[*index]
                    && signal.source == expected.source
                    && signal.targets.iter().copied().collect::<HashSet<_>>() == want
            });
            match hit {
                Some((index, signal)) => {
                    matched[index] = true;
                    if signal.risk != expected.risk {
                        risk_matches = false;
                    }
                }
                None => {
                    expected_found = false;
                    risk_matches = false;
                }
            }
        }
        let extra_signals = matched.iter().filter(|hit| !**hit).count() as u32;
        Score {
            ok: expected_found && risk_matches && extra_signals == 0,
            expected_found,
            risk_matches,
            extra_signals,
            signals_total: signals.len() as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use converge_storage::{Author, SignalId, SignalStatus};
    use time::OffsetDateTime;

    use super::*;
    use crate::data_dir;

    const SOURCE: &str = "01J00000000000000000000203";
    const TARGET: &str = "01J00000000000000000000104";
    const DECOY: &str = "01J00000000000000000000102";

    fn signal(source: &str, target: &str, risk: Risk) -> Signal {
        Signal {
            id: SignalId::new(),
            source: source.parse().unwrap(),
            targets: vec![target.parse().unwrap()],
            risk,
            kind: "k".into(),
            status: SignalStatus::Proposed,
            title: "t".into(),
            text: "x".into(),
            consequence: "c".into(),
            recommendation: "r".into(),
            produced_by: Author::Agent("01J00000000000000000000006".parse().unwrap()),
            validated_by: None,
            created_at: OffsetDateTime::now_utc(),
        }
    }

    fn scenarios() -> Vec<Scenario> {
        load(&data_dir().join("scenarios/scenarios.toml")).unwrap()
    }

    fn scenario(name: &str) -> Scenario {
        scenarios().into_iter().find(|s| s.name == name).unwrap()
    }

    #[test]
    fn find_signal_requires_the_exact_contradiction() {
        let find = scenario("find_signal");
        let good = signal(SOURCE, TARGET, Risk::WillBreak);
        assert!(find.score(std::slice::from_ref(&good)).ok);

        let wrong_risk = find.score(&[signal(SOURCE, TARGET, Risk::Watch)]);
        assert!(wrong_risk.expected_found && !wrong_risk.risk_matches && !wrong_risk.ok);

        let noisy = find.score(&[good, signal(SOURCE, DECOY, Risk::Watch)]);
        assert!(noisy.expected_found && noisy.risk_matches);
        assert_eq!(noisy.extra_signals, 1);
        assert!(!noisy.ok);

        assert!(!find.score(&[]).ok);
    }

    #[test]
    fn stay_silent_accepts_only_silence() {
        let silent = scenario("stay_silent");
        assert!(silent.score(&[]).ok);
        let noisy = silent.score(&[signal(SOURCE, DECOY, Risk::Watch)]);
        assert!(!noisy.ok);
        assert_eq!(noisy.extra_signals, 1);
    }

    /// The committed scenario list and fixtures must stay coherent: every
    /// fixture deserializes as a real request, and every expected id exists
    /// in its scenario's universe (state or batch).
    #[test]
    fn scenario_list_and_fixtures_are_coherent() {
        let fixtures = data_dir().join("fixtures");
        let scenarios = scenarios();
        assert_eq!(scenarios.len(), 3);
        for scenario in &scenarios {
            let request = scenario.request(&fixtures).unwrap();
            assert!(!request.decisions.is_empty(), "{}", scenario.name);
            let known: HashSet<DecisionId> = request
                .state
                .decisions
                .iter()
                .chain(&request.decisions)
                .map(|d| d.decision.id)
                .collect();
            for expected in &scenario.expected {
                assert!(known.contains(&expected.source), "{}", scenario.name);
                for target in &expected.targets {
                    assert!(known.contains(target), "{}", scenario.name);
                }
            }
        }
    }
}
