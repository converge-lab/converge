//! Schema constraints as code (D13). `validate()` returns *all* violations, not
//! just the first — a broken seed should be fixable in one pass. The mock
//! server runs this at startup and refuses to serve a bad seed; the app runs it
//! as a debug assertion on the embedded path.

use crate::seed::enums::Status;
use crate::seed::rows::Seed;
use std::collections::{HashMap, HashSet};

/// Validate every D13 rule. `Ok(())` or the full list of violations.
pub fn validate(seed: &Seed) -> Result<(), Vec<String>> {
    let mut errs: Vec<String> = Vec::new();

    // Unique ids per table.
    let group_ids = unique_ids(
        seed.groups.iter().map(|g| g.id.as_str()),
        "groups",
        &mut errs,
    );
    let project_ids = unique_ids(
        seed.projects.iter().map(|p| p.id.as_str()),
        "projects",
        &mut errs,
    );
    let user_ids = unique_ids(seed.users.iter().map(|u| u.id.as_str()), "users", &mut errs);
    let agent_ids = unique_ids(
        seed.agents.iter().map(|a| a.id.as_str()),
        "agents",
        &mut errs,
    );
    let decision_ids = unique_ids(
        seed.decisions.iter().map(|d| d.id.as_str()),
        "decisions",
        &mut errs,
    );

    // FK integrity.
    for p in &seed.projects {
        if !group_ids.contains(p.group_id.as_str()) {
            errs.push(format!(
                "project '{}' references unknown group '{}'",
                p.id, p.group_id
            ));
        }
    }
    for d in &seed.decisions {
        if !project_ids.contains(d.project_id.as_str()) {
            errs.push(format!(
                "decision '{}' references unknown project '{}'",
                d.id, d.project_id
            ));
        }
    }
    for gp in &seed.group_projects {
        if !group_ids.contains(gp.group_id.as_str()) {
            errs.push(format!(
                "group_projects references unknown group '{}'",
                gp.group_id
            ));
        }
        if !project_ids.contains(gp.project_id.as_str()) {
            errs.push(format!(
                "group_projects references unknown project '{}'",
                gp.project_id
            ));
        }
    }

    // Membership: every project has ≥1 group; the owning group is a member.
    let mut membership: HashSet<(&str, &str)> = HashSet::new();
    for gp in &seed.group_projects {
        membership.insert((gp.group_id.as_str(), gp.project_id.as_str()));
    }
    for p in &seed.projects {
        if !membership.iter().any(|(_, pid)| *pid == p.id) {
            errs.push(format!("project '{}' has no group membership", p.id));
        }
        if !membership.contains(&(p.group_id.as_str(), p.id.as_str())) {
            errs.push(format!(
                "project '{}' owner group '{}' missing from group_projects",
                p.id, p.group_id
            ));
        }
    }

    // decision_author: refs exist, at-least-one-of, no duplicates, coverage.
    let mut author_rows: HashSet<(&str, Option<&str>, Option<&str>)> = HashSet::new();
    let mut authored: HashSet<&str> = HashSet::new();
    for a in &seed.decision_author {
        if !decision_ids.contains(a.decision_id.as_str()) {
            errs.push(format!(
                "decision_author references unknown decision '{}'",
                a.decision_id
            ));
        }
        if a.user_id.is_none() && a.agent_id.is_none() {
            errs.push(format!(
                "decision_author row for '{}' has neither user nor agent",
                a.decision_id
            ));
        }
        if let Some(u) = &a.user_id
            && !user_ids.contains(u.as_str())
        {
            errs.push(format!("decision_author references unknown user '{u}'"));
        }
        if let Some(ag) = &a.agent_id
            && !agent_ids.contains(ag.as_str())
        {
            errs.push(format!("decision_author references unknown agent '{ag}'"));
        }
        let key = (
            a.decision_id.as_str(),
            a.user_id.as_deref(),
            a.agent_id.as_deref(),
        );
        if !author_rows.insert(key) {
            errs.push(format!("duplicate decision_author row {key:?}"));
        }
        authored.insert(a.decision_id.as_str());
    }
    for d in &seed.decisions {
        if !authored.contains(d.id.as_str()) {
            errs.push(format!("decision '{}' has no authors", d.id));
        }
    }

    // Edges: refs exist, no self-loops.
    for e in &seed.decision_supersedes {
        for id in [&e.decision_id, &e.supersedes_id] {
            if !decision_ids.contains(id.as_str()) {
                errs.push(format!(
                    "decision_supersedes references unknown decision '{id}'"
                ));
            }
        }
        if e.decision_id == e.supersedes_id {
            errs.push(format!("decision '{}' supersedes itself", e.decision_id));
        }
    }
    for e in &seed.decision_related {
        for id in [&e.decision_id, &e.ref_id] {
            if !decision_ids.contains(id.as_str()) {
                errs.push(format!(
                    "decision_related references unknown decision '{id}'"
                ));
            }
        }
        if e.decision_id == e.ref_id {
            errs.push(format!("decision '{}' relates to itself", e.decision_id));
        }
    }

    // Supersedes graph is acyclic (DFS, three-color).
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &seed.decision_supersedes {
        adj.entry(e.decision_id.as_str())
            .or_default()
            .push(e.supersedes_id.as_str());
    }
    if has_cycle(&adj) {
        errs.push("decision_supersedes graph has a cycle".into());
    }

    // `superseded` is derived from inbound supersedes-edges (upstream storage
    // semantics): the *stored* status must never be `superseded`.
    for d in &seed.decisions {
        if d.status == Status::Superseded {
            errs.push(format!(
                "decision '{}' stores status 'superseded' — that status is derived \
                 from supersedes edges and must not be stored",
                d.id
            ));
        }
    }

    // Mock namespace keys reference real rows.
    let m = &seed.mock;
    if !user_ids.contains(m.me.user_id.as_str()) {
        errs.push(format!(
            "mock.me references unknown user '{}'",
            m.me.user_id
        ));
    }
    for uid in m.user_colors.keys() {
        if !user_ids.contains(uid.as_str()) {
            errs.push(format!("mock.user_colors references unknown user '{uid}'"));
        }
    }
    for did in m.decision_extras.keys() {
        if !decision_ids.contains(did.as_str()) {
            errs.push(format!(
                "mock.decision_extras references unknown decision '{did}'"
            ));
        }
    }
    for did in &m.unread {
        if !decision_ids.contains(did.as_str()) {
            errs.push(format!("mock.unread references unknown decision '{did}'"));
        }
    }
    for (pid, dids) in &m.agent_context {
        if !project_ids.contains(pid.as_str()) {
            errs.push(format!(
                "mock.agent_context references unknown project '{pid}'"
            ));
        }
        for did in dids {
            if !decision_ids.contains(did.as_str()) {
                errs.push(format!(
                    "mock.agent_context['{pid}'] references unknown decision '{did}'"
                ));
            }
        }
    }
    // Signals: `from` is a project id; `to` is display text (D12).
    for s in &m.signals {
        if !project_ids.contains(s.from.as_str()) {
            errs.push(format!(
                "signal '{}' from unknown project '{}'",
                s.id, s.from
            ));
        }
        if !decision_ids.contains(s.dec_id.as_str()) {
            errs.push(format!(
                "signal '{}' references unknown decision '{}'",
                s.id, s.dec_id
            ));
        }
        for src in &s.sources {
            if !decision_ids.contains(src.as_str()) {
                errs.push(format!(
                    "signal '{}' source references unknown decision '{src}'",
                    s.id
                ));
            }
        }
    }

    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

/// Collect ids, recording duplicates as violations.
fn unique_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    table: &str,
    errs: &mut Vec<String>,
) -> HashSet<&'a str> {
    let mut set = HashSet::new();
    for id in ids {
        if !set.insert(id) {
            errs.push(format!("duplicate id '{id}' in {table}"));
        }
    }
    set
}

/// Cycle detection over the supersedes edges (iterative three-color DFS).
fn has_cycle(adj: &HashMap<&str, Vec<&str>>) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Open,
        Done,
    }
    let mut state: HashMap<&str, State> = HashMap::new();
    for &start in adj.keys() {
        if state.contains_key(start) {
            continue;
        }
        // stack of (node, next-child-index)
        let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
        state.insert(start, State::Open);
        while let Some(&mut (node, ref mut idx)) = stack.last_mut() {
            let children = adj.get(node).map(Vec::as_slice).unwrap_or(&[]);
            if *idx < children.len() {
                let child = children[*idx];
                *idx += 1;
                match state.get(child) {
                    Some(State::Open) => return true,
                    Some(State::Done) => {}
                    None => {
                        state.insert(child, State::Open);
                        stack.push((child, 0));
                    }
                }
            } else {
                state.insert(node, State::Done);
                stack.pop();
            }
        }
    }
    false
}
