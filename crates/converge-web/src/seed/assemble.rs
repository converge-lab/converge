//! Rows → wire objects. `assemble()` builds the read-model the API serves:
//! groups with their membership `project_ids`, decisions with embedded authors
//! and *both* edge directions (reverse edges derived here, never stored — D5),
//! everything sorted the way the API returns it (`captured_at` desc, id asc;
//! ISO-UTC strings sort lexicographically, so plain string `Ord` is correct).

use crate::seed::rows::Seed;
use crate::seed::wire;
use std::collections::HashMap;

/// The assembled read-model: what the API serves and the app consumes.
pub struct Assembled {
    pub groups: Vec<wire::Group>,
    pub projects: Vec<wire::Project>,
    pub users: Vec<wire::User>,
    pub agents: Vec<wire::Agent>,
    /// Sorted `captured_at` desc, id asc.
    pub decisions: Vec<wire::Decision>,
    pub me: wire::mock::Me,
    pub user_colors: HashMap<String, String>,
    pub signals: Vec<wire::mock::Signal>,
    pub decision_extras: HashMap<String, wire::mock::Extras>,
    pub unread: Vec<String>,
    pub agent_context: HashMap<String, Vec<String>>,
}

/// Assemble the seed into wire objects.
pub fn assemble(seed: &Seed) -> Assembled {
    // Group membership read-model, in seed order (stable).
    let groups = seed
        .groups
        .iter()
        .map(|g| wire::Group {
            id: g.id.clone(),
            name: g.name.clone(),
            description: g.description.clone(),
            kind: g.kind,
            created_at: g.created_at.clone(),
            project_ids: seed
                .group_projects
                .iter()
                .filter(|gp| gp.group_id == g.id)
                .map(|gp| gp.project_id.clone())
                .collect(),
        })
        .collect();

    let projects = seed
        .projects
        .iter()
        .map(|p| wire::Project {
            id: p.id.clone(),
            group_id: p.group_id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            created_at: p.created_at.clone(),
        })
        .collect();

    let users: Vec<wire::User> = seed
        .users
        .iter()
        .map(|u| wire::User {
            id: u.id.clone(),
            handle: u.handle.clone(),
            name: u.name.clone(),
        })
        .collect();
    let agents = seed
        .agents
        .iter()
        .map(|a| wire::Agent {
            id: a.id.clone(),
            kind: a.kind,
            name: a.name.clone(),
        })
        .collect();

    // Relation lookups, preserving seed order per decision.
    let mut authors: HashMap<&str, Vec<wire::AuthorRef>> = HashMap::new();
    for a in &seed.decision_author {
        authors
            .entry(a.decision_id.as_str())
            .or_default()
            .push(wire::AuthorRef {
                user_id: a.user_id.clone(),
                agent_id: a.agent_id.clone(),
            });
    }
    let mut supersedes: HashMap<&str, Vec<String>> = HashMap::new();
    let mut superseded_by: HashMap<&str, Vec<String>> = HashMap::new();
    for e in &seed.decision_supersedes {
        supersedes
            .entry(e.decision_id.as_str())
            .or_default()
            .push(e.supersedes_id.clone());
        superseded_by
            .entry(e.supersedes_id.as_str())
            .or_default()
            .push(e.decision_id.clone());
    }
    // Cross-ref edges, both directions (upstream naming: `related_to` =
    // outgoing, `related_by` = incoming).
    let mut related_to: HashMap<&str, Vec<wire::RelatedRef>> = HashMap::new();
    let mut related_by: HashMap<&str, Vec<wire::RelatedRef>> = HashMap::new();
    for e in &seed.decision_related {
        related_to
            .entry(e.decision_id.as_str())
            .or_default()
            .push(wire::RelatedRef {
                id: e.ref_id.clone(),
                why: e.why.clone(),
            });
        related_by
            .entry(e.ref_id.as_str())
            .or_default()
            .push(wire::RelatedRef {
                id: e.decision_id.clone(),
                why: e.why.clone(),
            });
    }

    let mut decisions: Vec<wire::Decision> = seed
        .decisions
        .iter()
        .map(|d| {
            let superseded_by = superseded_by.remove(d.id.as_str()).unwrap_or_default();
            // Derived status (upstream semantics): any inbound supersedes-edge
            // makes the decision read as superseded; the stored status is
            // never `superseded` (validate() enforces that).
            let status = if superseded_by.is_empty() {
                d.status
            } else {
                crate::seed::enums::Status::Superseded
            };
            wire::Decision {
                id: d.id.clone(),
                project_id: d.project_id.clone(),
                status,
                title: d.title.clone(),
                summary: d.summary.clone(),
                context: d.context.clone(),
                consequences: d.consequences.clone(),
                alternatives: d.alternatives.clone(),
                authors: authors.remove(d.id.as_str()).unwrap_or_default(),
                supersedes: supersedes.remove(d.id.as_str()).unwrap_or_default(),
                superseded_by,
                related_to: related_to.remove(d.id.as_str()).unwrap_or_default(),
                related_by: related_by.remove(d.id.as_str()).unwrap_or_default(),
                captured_at: d.captured_at.clone(),
            }
        })
        .collect();
    decisions.sort_by(|a, b| {
        b.captured_at
            .cmp(&a.captured_at)
            .then_with(|| a.id.cmp(&b.id))
    });

    // /me: resolve name + color from users / user_colors.
    let me_seed = &seed.mock.me;
    let name = seed
        .users
        .iter()
        .find(|u| u.id == me_seed.user_id)
        .map(|u| u.name.clone())
        .unwrap_or_default();
    let color = seed
        .mock
        .user_colors
        .get(&me_seed.user_id)
        .cloned()
        .unwrap_or_default();
    let me = wire::mock::Me {
        user_id: me_seed.user_id.clone(),
        name,
        initial: me_seed.initial.clone(),
        role: me_seed.role.clone(),
        email: me_seed.email.clone(),
        color,
    };

    Assembled {
        groups,
        projects,
        users,
        agents,
        decisions,
        me,
        user_colors: seed.mock.user_colors.clone(),
        signals: seed.mock.signals.clone(),
        decision_extras: seed.mock.decision_extras.clone(),
        unread: seed.mock.unread.clone(),
        agent_context: seed.mock.agent_context.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::rows::{EMBEDDED, Seed};
    use crate::seed::validate::validate;

    fn embedded() -> Seed {
        Seed::parse(EMBEDDED).expect("embedded seed parses")
    }

    /// The shipped seed passes every validation rule.
    #[test]
    fn embedded_seed_validates() {
        let seed = embedded();
        if let Err(errs) = validate(&seed) {
            panic!("embedded seed invalid:\n{}", errs.join("\n"));
        }
        assert_eq!(seed.decisions.len(), 19);
        assert_eq!(seed.mock.signals.len(), 3);
        assert_eq!(seed.groups.len(), 3);
    }

    /// Each validation rule fires on a hand-broken seed.
    #[test]
    fn validation_rules_fire() {
        let base = embedded();

        // Unknown FK: decision → project.
        let mut s = base.clone();
        s.decisions[0].project_id = "nope".into();
        assert_violation(&s, "unknown project 'nope'");

        // Duplicate id.
        let mut s = base.clone();
        let dup = s.decisions[0].clone();
        s.decisions.push(dup);
        assert_violation(&s, "duplicate id");

        // Author row with neither user nor agent.
        let mut s = base.clone();
        s.decision_author[0].user_id = None;
        s.decision_author[0].agent_id = None;
        assert_violation(&s, "neither user nor agent");

        // Decision with no authors.
        let mut s = base.clone();
        let victim = s.decisions[0].id.clone();
        s.decision_author.retain(|a| a.decision_id != victim);
        assert_violation(&s, "has no authors");

        // Self-loop edge.
        let mut s = base.clone();
        s.decision_supersedes
            .push(crate::seed::rows::DecisionSupersedesRow {
                decision_id: "status-field".into(),
                supersedes_id: "status-field".into(),
            });
        assert_violation(&s, "supersedes itself");

        // Cycle: close the 3-node chain into a loop.
        let mut s = base.clone();
        s.decision_supersedes
            .push(crate::seed::rows::DecisionSupersedesRow {
                decision_id: "status-http".into(),
                supersedes_id: "status-field".into(),
            });
        assert_violation(&s, "cycle");

        // Storing `superseded` is invalid — it's derived from edges.
        let mut s = base.clone();
        s.decisions
            .iter_mut()
            .find(|d| d.id == "gateway-only")
            .unwrap()
            .status = crate::seed::enums::Status::Superseded;
        assert_violation(&s, "must not be stored");

        // Signal from unknown project.
        let mut s = base.clone();
        s.mock.signals[0].from = "nowhere".into();
        assert_violation(&s, "from unknown project");

        // Unread referencing a ghost decision.
        let mut s = base.clone();
        s.mock.unread.push("ghost".into());
        assert_violation(&s, "unread references unknown decision");

        // Project with no membership.
        let mut s = base.clone();
        s.group_projects.retain(|gp| gp.project_id != "scratch");
        assert_violation(&s, "no group membership");
    }

    fn assert_violation(seed: &Seed, needle: &str) {
        let errs = validate(seed).expect_err("seed should be invalid");
        assert!(
            errs.iter().any(|e| e.contains(needle)),
            "expected violation containing {needle:?}, got:\n{}",
            errs.join("\n")
        );
    }

    /// Reverse edges and the derived `superseded` status are correct on the
    /// 3-node chain.
    #[test]
    fn reverse_edges_on_chain() {
        let a = assemble(&embedded());
        let by_id = |id: &str| a.decisions.iter().find(|d| d.id == id).unwrap();

        let field = by_id("status-field");
        assert_eq!(field.supersedes, vec!["status-text"]);
        assert!(field.superseded_by.is_empty());
        assert_eq!(field.status, crate::seed::enums::Status::Accepted);

        // Stored as accepted, *reads* as superseded via the inbound edge.
        let text = by_id("status-text");
        assert_eq!(text.supersedes, vec!["status-http"]);
        assert_eq!(text.superseded_by, vec!["status-field"]);
        assert_eq!(text.status, crate::seed::enums::Status::Superseded);

        let http = by_id("status-http");
        assert!(http.supersedes.is_empty());
        assert_eq!(http.superseded_by, vec!["status-text"]);
        assert_eq!(http.status, crate::seed::enums::Status::Superseded);

        // related_to / related_by: status-field → retry-backoff.
        assert!(field.related_to.iter().any(|r| r.id == "retry-backoff"));
        let retry = by_id("retry-backoff");
        assert!(retry.related_by.iter().any(|r| r.id == "status-field"));
    }

    /// Assembled decisions are sorted `captured_at` desc, id asc; groups carry
    /// their membership.
    #[test]
    fn assembled_order_and_membership() {
        let a = assemble(&embedded());
        for w in a.decisions.windows(2) {
            let ord = w[0]
                .captured_at
                .cmp(&w[1].captured_at)
                .then_with(|| w[1].id.cmp(&w[0].id));
            assert!(
                ord.is_ge(),
                "order violated: {} before {}",
                w[0].id,
                w[1].id
            );
        }
        assert_eq!(a.decisions[0].id, "scratch-llm-cache");

        let platform = a.groups.iter().find(|g| g.id == "platform").unwrap();
        assert_eq!(platform.project_ids.len(), 6);
        let payments = a.groups.iter().find(|g| g.id == "payments").unwrap();
        assert_eq!(payments.project_ids.len(), 3);

        assert_eq!(a.me.name, "Marco Reyes");
        assert_eq!(a.me.color, "#4a7fb5");
    }
}
