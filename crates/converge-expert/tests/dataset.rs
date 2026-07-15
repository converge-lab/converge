#![cfg(feature = "private-fixtures")]

use std::collections::{HashMap, HashSet};

use converge_expert::signals::{Request, State};
use converge_storage::{Decision, DecisionId, Group, Project, ProjectId, Risk};
use serde::Deserialize;

const MEMORY_JSON: &str = include_str!("fixtures/signals_memory.json");
const CASES_JSON: &str = include_str!("fixtures/signals_cases.json");

#[derive(Debug, Deserialize)]
struct Memory {
    schema_version: u32,
    group: Group,
    projects: Vec<Project>,
    decisions: Vec<Decision>,
}

#[derive(Debug, Deserialize)]
struct Dataset {
    schema_version: u32,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    key: String,
    description: String,
    new_decision: DecisionId,
    expected_signals: Vec<ExpectedSignal>,
    forbidden_targets: Vec<DecisionId>,
    notes: String,
}

#[derive(Debug, Deserialize)]
struct ExpectedSignal {
    targets: Vec<DecisionId>,
    risk: Risk,
    acceptable_kinds: Vec<String>,
    rationale: String,
}

fn load_memory() -> Memory {
    serde_json::from_str(MEMORY_JSON).expect("signals_memory.json must match storage types")
}

fn load_dataset() -> Dataset {
    serde_json::from_str(CASES_JSON).expect("signals_cases.json must match the rubric schema")
}

#[test]
fn memory_is_a_valid_three_project_state() {
    let memory = load_memory();

    assert_eq!(memory.schema_version, 1);
    assert_eq!(memory.projects.len(), 3);
    assert_eq!(memory.decisions.len(), 18);

    let project_names = memory
        .projects
        .iter()
        .map(|project| project.name.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(project_names, HashSet::from(["server", "mcp", "webui"]));

    let projects = memory
        .projects
        .iter()
        .map(|project| {
            assert_eq!(project.group_id, memory.group.id);
            (project.id, project)
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(
        projects.len(),
        memory.projects.len(),
        "duplicate project id"
    );

    let mut decision_ids = HashSet::new();
    let mut decisions_per_project = HashMap::<ProjectId, usize>::new();
    for decision in &memory.decisions {
        assert!(
            decision_ids.insert(decision.id),
            "duplicate decision id {}",
            decision.id
        );
        assert!(
            projects.contains_key(&decision.project_id),
            "decision {} references an unknown project",
            decision.id
        );
        *decisions_per_project
            .entry(decision.project_id)
            .or_default() += 1;
    }

    for project in &memory.projects {
        assert!(
            decisions_per_project.get(&project.id).copied().unwrap_or(0) >= 2,
            "project {} needs enough context for evaluation",
            project.name
        );
    }

    assert!(
        memory
            .decisions
            .windows(2)
            .all(|pair| pair[0].captured_at < pair[1].captured_at)
    );
}

#[test]
fn cases_build_valid_signals_requests() {
    let memory = load_memory();
    let dataset = load_dataset();

    assert_eq!(dataset.schema_version, 1);
    assert_eq!(dataset.cases.len(), 7);

    let decisions = memory
        .decisions
        .iter()
        .map(|decision| (decision.id, decision))
        .collect::<HashMap<_, _>>();
    let mut keys = HashSet::new();
    let mut has_empty_case = false;
    let mut risk_coverage = [false; 3];

    for case in &dataset.cases {
        assert!(keys.insert(case.key.as_str()), "duplicate case key");
        assert!(!case.description.trim().is_empty());
        assert!(!case.notes.trim().is_empty());

        let source = decisions
            .get(&case.new_decision)
            .unwrap_or_else(|| panic!("case {} has an unknown source decision", case.key));
        let state_decisions = memory
            .decisions
            .iter()
            .filter(|decision| decision.captured_at < source.captured_at)
            .cloned()
            .collect::<Vec<_>>();
        let state_ids = state_decisions
            .iter()
            .map(|decision| decision.id)
            .collect::<HashSet<_>>();

        let request = Request {
            state: State {
                group: memory.group.clone(),
                projects: memory.projects.clone(),
                decisions: state_decisions,
                signals: Vec::new(),
            },
            decision: (*source).clone(),
        };
        assert!(!request.state.decisions.contains(source));

        let encoded = serde_json::to_string(&request).expect("request must serialize");
        let decoded: Request = serde_json::from_str(&encoded).expect("request must deserialize");
        assert_eq!(decoded, request);

        has_empty_case |= case.expected_signals.is_empty();
        let mut expected_targets = HashSet::new();
        for expected in &case.expected_signals {
            assert!(!expected.targets.is_empty());
            assert!(!expected.rationale.trim().is_empty());
            assert!(!expected.acceptable_kinds.is_empty());
            assert!(
                expected
                    .acceptable_kinds
                    .iter()
                    .all(|kind| is_snake_case(kind))
            );

            match expected.risk {
                Risk::Watch => risk_coverage[0] = true,
                Risk::Coordinate => risk_coverage[1] = true,
                Risk::WillBreak => risk_coverage[2] = true,
            }

            for target in &expected.targets {
                assert!(
                    expected_targets.insert(*target),
                    "case {} labels target {} more than once",
                    case.key,
                    target
                );
                assert!(
                    state_ids.contains(target),
                    "case {} target {} is not in the preceding state",
                    case.key,
                    target
                );
                assert_ne!(
                    decisions[target].project_id, source.project_id,
                    "case {} expected signal must cross a project boundary",
                    case.key
                );
            }
        }

        let mut forbidden_targets = HashSet::new();
        for target in &case.forbidden_targets {
            assert!(
                forbidden_targets.insert(*target),
                "case {} repeats forbidden target {}",
                case.key,
                target
            );
            assert!(
                state_ids.contains(target),
                "case {} forbidden target {} is not in the preceding state",
                case.key,
                target
            );
            assert!(
                !expected_targets.contains(target),
                "case {} marks target {} as both expected and forbidden",
                case.key,
                target
            );
        }
    }

    assert!(has_empty_case, "dataset needs a no-signal example");
    assert_eq!(risk_coverage, [true, true, true]);
}

fn is_snake_case(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('_')
        && !value.ends_with('_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
