//! The app's internal dataset + the query layer over it.
//!
//! Read-model shapes come from `crate::seed`; [`build_dataset`] converts an
//! [`Assembled`] read-model (either assembled in-process from the embedded
//! seed, or fetched over HTTP) into the internal [`Dataset`]. Both sources go
//! through this one function, so the embedded and API paths cannot drift.
//!
//! Internally decisions carry *resolved* `converge_ui::domain` values (authors
//! with tints, domain enums) so screens render without further lookups. Query
//! functions read the current snapshot out of the reactive store and hand back
//! `Rc` clones; the pure `q_*` cores take `&Dataset` so they can be unit-tested
//! without a reactive context.

use crate::seed::{Assembled, wire};
use crate::store::{AppStateStoreFields, use_store};
use crate::when::when;
use converge_ui::domain::{
    self, Author, ConvLine, Decision, DecisionRef, GroupKind, Signal, SignalDetail, Source,
    SourceView, Status, initials,
};
use leptos::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;

// ---- internal dataset types ----------------------------------------------

/// One line of an anchored source conversation.
pub struct Line {
    pub speaker: String,
    pub text: String,
    pub hl: bool,
}

/// Anchored evidence a decision was derived from.
pub struct Src {
    pub kind: domain::SourceKind,
    pub label: String,
    pub when: String,
    pub lines: Vec<Line>,
}

/// A rejected alternative + why it lost.
pub struct Alt {
    pub option: String,
    pub why_rejected: String,
}

/// A related-decision reference (either direction) with the reason.
pub struct Related {
    pub id: String,
    pub why: Option<String>,
}

/// A decision, fully resolved for rendering: domain enums, authors with tints,
/// extras (description/session/tags/sources) merged in.
pub struct Dec {
    pub id: String,
    pub project_id: String,
    pub status: Status,
    pub title: String,
    pub summary: String,
    /// Long markdown body; empty when the decision has none.
    pub description: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    /// ISO-8601 UTC; rendered via `when()`.
    pub captured_at: String,
    pub authors: Vec<Author>,
    #[allow(dead_code)] // filters use them via ProjectLog's RowData
    pub tags: Vec<String>,
    pub alts: Vec<Alt>,
    /// Outgoing references to other decisions (with why).
    pub related_to: Vec<Related>,
    /// Derived incoming references (who references this decision).
    #[allow(dead_code)] // not rendered yet; carried from the wire for parity
    pub related_by: Vec<Related>,
    pub supersedes: Vec<String>,
    pub superseded_by: Vec<String>,
    /// Session provenance (e.g. "claude-session · a3f9"); empty when absent.
    pub session: String,
    pub sources: Vec<Src>,
}

/// A cross-project signal (mock-backed for now).
pub struct Sig {
    pub id: String,
    pub from: String,
    pub to: String,
    pub dec_id: String,
    pub title: String,
    pub text: String,
    pub consequence: String,
    pub recommended: String,
    pub risk: domain::Risk,
    pub sources: Vec<String>,
}

/// A group with its membership (D3 read-model).
#[derive(Clone)]
pub struct GroupDef {
    #[allow(dead_code)] // reserved as the routing/persistence id
    pub id: String,
    pub name: String,
    pub kind: GroupKind,
    pub project_ids: Vec<String>,
}

/// A project row, as the app needs it.
pub struct ProjectInfo {
    pub id: String,
    #[allow(dead_code)] // owning group; not displayed yet
    pub group_id: String,
    #[allow(dead_code)] // display name == id today; screens render the id
    pub name: String,
    pub description: Option<String>,
}

/// The signed-in account.
#[derive(Clone)]
pub struct Account {
    pub initial: String,
    pub name: String,
    pub role: String,
    pub color: String,
    pub email: String,
}

/// Everything the app renders.
pub struct Dataset {
    pub groups: Vec<GroupDef>,
    pub projects: Vec<ProjectInfo>,
    /// Sorted `captured_at` desc (the wire order).
    pub decisions: Vec<Rc<Dec>>,
    pub signals: Vec<Rc<Sig>>,
    pub agent_context: HashMap<String, Vec<String>>,
    pub unread: Vec<String>,
    pub account: Account,
}

// ---- wire → domain conversion ---------------------------------------------

fn status_of(s: crate::seed::Status) -> Status {
    use crate::seed::Status as W;
    match s {
        W::Accepted => Status::Accepted,
        W::Draft => Status::Draft,
        W::Proposed => Status::Proposed,
        W::Superseded => Status::Superseded,
        W::Rejected => Status::Rejected,
    }
}

fn risk_of(r: crate::seed::Risk) -> domain::Risk {
    use crate::seed::Risk as W;
    match r {
        W::WillBreak => domain::Risk::WillBreak,
        W::Coordinate => domain::Risk::Coordinate,
        W::Watch => domain::Risk::Watch,
    }
}

fn source_kind_of(k: crate::seed::SourceKind) -> domain::SourceKind {
    use crate::seed::SourceKind as W;
    match k {
        W::Transcript => domain::SourceKind::Transcript,
        W::Slack => domain::SourceKind::Slack,
        W::Pr => domain::SourceKind::Pr,
        W::Incident => domain::SourceKind::Incident,
    }
}

fn group_kind_of(k: crate::seed::GroupKind) -> GroupKind {
    use crate::seed::GroupKind as W;
    match k {
        W::Shared => GroupKind::Shared,
        W::Personal => GroupKind::Personal,
    }
}

/// Resolve an author reference into a renderable `domain::Author`: users get
/// their pinned color as `tint` (FNV fallback when absent); agents render as
/// the expert-purple "✦" chip (D9). A ref with *both* ids set is upstream's
/// `UserViaAgent` (a person working through an agent) — rendered as the
/// person, since they own the decision.
fn resolve_author(
    r: &wire::AuthorRef,
    users: &HashMap<&str, &str>,
    colors: &HashMap<String, String>,
    agents: &HashMap<&str, &str>,
) -> Author {
    if let Some(uid) = &r.user_id {
        let name = users.get(uid.as_str()).copied().unwrap_or(uid.as_str());
        return match colors.get(uid) {
            Some(c) => Author::person(name, c),
            None => Author::human_named(&initials(name), name),
        };
    }
    let aid = r.agent_id.as_deref().unwrap_or("");
    let name = agents.get(aid).copied().unwrap_or(aid);
    let mut a = Author::agent_named("✦", name);
    a.tint = Some("var(--cv-expert)".into());
    a
}

/// Convert the assembled read-model into the app's dataset. The one conversion
/// both `EmbeddedSource` and `ApiSource` share.
pub fn build_dataset(a: Assembled) -> Dataset {
    let users: HashMap<&str, &str> = a
        .users
        .iter()
        .map(|u| (u.id.as_str(), u.name.as_str()))
        .collect();
    let agents: HashMap<&str, &str> = a
        .agents
        .iter()
        .map(|ag| (ag.id.as_str(), ag.name.as_str()))
        .collect();

    let mut extras = a.decision_extras;
    let decisions: Vec<Rc<Dec>> = a
        .decisions
        .iter()
        .map(|d| {
            let ex = extras.remove(&d.id).unwrap_or(wire::mock::Extras {
                description: None,
                session: None,
                tags: Vec::new(),
                sources: Vec::new(),
            });
            Rc::new(Dec {
                id: d.id.clone(),
                project_id: d.project_id.clone(),
                status: status_of(d.status),
                title: d.title.clone(),
                summary: d.summary.clone(),
                description: ex.description.unwrap_or_default(),
                context: d.context.clone(),
                consequences: d.consequences.clone(),
                captured_at: d.captured_at.clone(),
                authors: d
                    .authors
                    .iter()
                    .map(|r| resolve_author(r, &users, &a.user_colors, &agents))
                    .collect(),
                tags: ex.tags,
                alts: d
                    .alternatives
                    .iter()
                    .map(|alt| Alt {
                        option: alt.option.clone(),
                        why_rejected: alt.why_rejected.clone(),
                    })
                    .collect(),
                related_to: d
                    .related_to
                    .iter()
                    .map(|r| Related {
                        id: r.id.clone(),
                        why: r.why.clone(),
                    })
                    .collect(),
                related_by: d
                    .related_by
                    .iter()
                    .map(|r| Related {
                        id: r.id.clone(),
                        why: r.why.clone(),
                    })
                    .collect(),
                supersedes: d.supersedes.clone(),
                superseded_by: d.superseded_by.clone(),
                session: ex.session.unwrap_or_default(),
                sources: ex
                    .sources
                    .into_iter()
                    .map(|s| Src {
                        kind: source_kind_of(s.kind),
                        label: s.label,
                        when: s.when,
                        lines: s
                            .lines
                            .into_iter()
                            .map(|l| Line {
                                speaker: l.speaker,
                                text: l.text,
                                hl: l.hl,
                            })
                            .collect(),
                    })
                    .collect(),
            })
        })
        .collect();

    let signals = a
        .signals
        .iter()
        .map(|s| {
            Rc::new(Sig {
                id: s.id.clone(),
                from: s.from.clone(),
                to: s.to.clone(),
                dec_id: s.dec_id.clone(),
                title: s.title.clone(),
                text: s.text.clone(),
                consequence: s.consequence.clone(),
                recommended: s.recommended.clone(),
                risk: risk_of(s.risk),
                sources: s.sources.clone(),
            })
        })
        .collect();

    Dataset {
        groups: a
            .groups
            .iter()
            .map(|g| GroupDef {
                id: g.id.clone(),
                name: g.name.clone(),
                kind: group_kind_of(g.kind),
                project_ids: g.project_ids.clone(),
            })
            .collect(),
        projects: a
            .projects
            .iter()
            .map(|p| ProjectInfo {
                id: p.id.clone(),
                group_id: p.group_id.clone(),
                name: p.name.clone(),
                description: p.description.clone(),
            })
            .collect(),
        decisions,
        signals,
        agent_context: a.agent_context,
        unread: a.unread,
        account: Account {
            initial: a.me.initial.clone(),
            name: a.me.name.clone(),
            role: a.me.role.clone(),
            color: a.me.color.clone(),
            email: a.me.email.clone(),
        },
    }
}

// ---- store access ---------------------------------------------------------

/// The current dataset snapshot. Reads are untracked: screens are re-created by
/// the router when the group changes, and the dataset itself is immutable in
/// this phase, so query functions don't need to subscribe.
fn ds() -> Rc<Dataset> {
    use_store()
        .dataset()
        .get_untracked()
        .expect("dataset must be loaded before queries run")
}

fn group_idx() -> usize {
    use_store().group().get_untracked()
}

// ---- pure cores (unit-testable, no reactive context) -----------------------

fn q_by_id<'a>(ds: &'a Dataset, id: &str) -> Option<&'a Rc<Dec>> {
    ds.decisions.iter().find(|d| d.id == id)
}

fn q_sig_by_id<'a>(ds: &'a Dataset, id: &str) -> Option<&'a Rc<Sig>> {
    ds.signals.iter().find(|s| s.id == id)
}

fn q_group(ds: &Dataset, idx: usize) -> &GroupDef {
    &ds.groups[idx.min(ds.groups.len().saturating_sub(1))]
}

fn q_group_decisions(ds: &Dataset, idx: usize) -> Vec<Rc<Dec>> {
    let projs = &q_group(ds, idx).project_ids;
    ds.decisions
        .iter()
        .filter(|d| projs.contains(&d.project_id))
        .cloned()
        .collect()
}

/// Feed = the group's decisions, newest first (the dataset is already sorted
/// `captured_at` desc), capped at 7.
fn q_feed(ds: &Dataset, idx: usize) -> Vec<Rc<Dec>> {
    q_group_decisions(ds, idx).into_iter().take(7).collect()
}

/// The supersession chain through `id`, oldest → newest. Edges are `Vec`s on
/// the wire; the walk follows the *first* entry each step (the demo data is a
/// single linear chain), cycle-guarded.
fn q_chain(ds: &Dataset, id: &str) -> Vec<Rc<Dec>> {
    let Some(d) = q_by_id(ds, id) else {
        return Vec::new();
    };
    let mut back: Vec<Rc<Dec>> = Vec::new();
    let mut cur = d;
    while let Some(prev_id) = cur.supersedes.first() {
        match q_by_id(ds, prev_id) {
            Some(p) if !back.iter().any(|x| x.id == p.id) && p.id != d.id => {
                back.insert(0, p.clone());
                cur = p;
            }
            _ => break,
        }
    }
    let mut fwd: Vec<Rc<Dec>> = Vec::new();
    cur = d;
    while let Some(next_id) = cur.superseded_by.first() {
        match q_by_id(ds, next_id) {
            Some(n) if !fwd.iter().any(|x| x.id == n.id) && n.id != d.id => {
                fwd.push(n.clone());
                cur = n;
            }
            _ => break,
        }
    }
    let mut out = back;
    out.push(d.clone());
    out.extend(fwd);
    out
}

// ---- groups -----------------------------------------------------------------

pub fn cur_group() -> GroupDef {
    let d = ds();
    q_group(&d, group_idx()).clone()
}

pub fn groups() -> Vec<GroupDef> {
    ds().groups.clone()
}

pub fn cur_group_projects() -> Vec<String> {
    cur_group().project_ids
}

pub fn group_name() -> String {
    cur_group().name
}

pub fn group_meta() -> String {
    let g = cur_group();
    let n = g.project_ids.len();
    let kind = match g.kind {
        GroupKind::Shared => "Shared",
        GroupKind::Personal => "Personal",
    };
    format!(
        "{kind} · {n} {}",
        if n == 1 { "project" } else { "projects" }
    )
}

pub fn group_tagline() -> String {
    let g = cur_group();
    let n = g.project_ids.len();
    match g.kind {
        GroupKind::Personal => format!(
            "Your personal decision memory across {n} {}.",
            if n == 1 { "project" } else { "projects" }
        ),
        GroupKind::Shared => format!("Shared decision memory across {n} services."),
    }
}

// ---- account + lookups -------------------------------------------------------

pub fn account() -> Account {
    ds().account.clone()
}

pub fn proj_desc(pid: &str) -> String {
    ds().projects
        .iter()
        .find(|p| p.id == pid)
        .and_then(|p| p.description.clone())
        .unwrap_or_default()
}

// ---- decision / signal queries ------------------------------------------------

pub fn by_id(id: &str) -> Option<Rc<Dec>> {
    q_by_id(&ds(), id).cloned()
}

pub fn sig_by_id(id: &str) -> Option<Rc<Sig>> {
    q_sig_by_id(&ds(), id).cloned()
}

pub fn group_decisions() -> Vec<Rc<Dec>> {
    q_group_decisions(&ds(), group_idx())
}

pub fn group_signals() -> Vec<Rc<Sig>> {
    let d = ds();
    let projs = &q_group(&d, group_idx()).project_ids;
    d.signals
        .iter()
        .filter(|s| projs.contains(&s.from))
        .cloned()
        .collect()
}

pub fn feed() -> Vec<Rc<Dec>> {
    q_feed(&ds(), group_idx())
}

pub fn project_decisions(pid: &str) -> Vec<Rc<Dec>> {
    // Dataset order is already `captured_at` desc.
    ds().decisions
        .iter()
        .filter(|d| d.project_id == pid)
        .cloned()
        .collect()
}

pub fn is_unread(id: &str) -> bool {
    ds().unread.iter().any(|x| x == id)
}

pub fn unread_count(pid: &str) -> u32 {
    project_decisions(pid)
        .iter()
        .filter(|d| is_unread(&d.id))
        .count() as u32
}

pub fn chain_of(id: &str) -> Vec<Rc<Dec>> {
    q_chain(&ds(), id)
}

pub fn signals_for(dec_id: &str) -> Vec<Rc<Sig>> {
    ds().signals
        .iter()
        .filter(|s| s.dec_id == dec_id || s.sources.iter().any(|x| x == dec_id))
        .cloned()
        .collect()
}

/// True if a decision sits in its own project's forwarded expert context.
pub fn in_agent_context(id: &str, proj: &str) -> bool {
    ds().agent_context
        .get(proj)
        .map(|ids| ids.iter().any(|x| x == id))
        .unwrap_or(false)
}

// ---- conversions to UI view models ---------------------------------------------

/// "Priya Nair", "A & B", "A +2" — from resolved authors.
pub fn authors_label(authors: &[Author]) -> String {
    match authors.len() {
        0 => String::new(),
        1 => authors[0].name.clone(),
        2 => format!("{} & {}", authors[0].name, authors[1].name),
        _ => format!("{} +{}", authors[0].name, authors.len() - 1),
    }
}

/// Session provenance when present, else the first source's label.
pub fn provenance_from(d: &Dec) -> String {
    if !d.session.is_empty() {
        d.session.clone()
    } else if let Some(s) = d.sources.first() {
        s.label.clone()
    } else {
        String::new()
    }
}

pub fn to_card(d: &Dec) -> Decision {
    Decision {
        authors: d.authors.clone(),
        project: d.project_id.clone(),
        status: d.status,
        date: when(&d.captured_at),
        title: d.title.clone(),
        provenance: provenance_from(d),
        authors_label: authors_label(&d.authors),
    }
}

pub fn to_ref(d: &Dec) -> DecisionRef {
    DecisionRef {
        authors: d.authors.clone(),
        project: d.project_id.clone(),
        status: d.status,
        title: d.title.clone(),
        summary: d.summary.clone(),
    }
}

pub fn to_source(s: &Src) -> Source {
    Source {
        kind: s.kind,
        label: s.label.clone(),
        when: s.when.clone(),
    }
}

pub fn to_source_view(s: &Src) -> SourceView {
    SourceView {
        kind: s.kind,
        label: s.label.clone(),
        when: s.when.clone(),
        lines: s
            .lines
            .iter()
            .map(|l| ConvLine {
                speaker: l.speaker.clone(),
                text: l.text.clone(),
                extracted: l.hl,
            })
            .collect(),
    }
}

pub fn to_signal(s: &Sig) -> Signal {
    Signal {
        from: s.from.clone(),
        to: s.to.clone(),
        risk: s.risk,
        title: s.title.clone(),
        text: s.text.clone(),
    }
}

pub fn to_signal_detail(s: &Sig) -> SignalDetail {
    SignalDetail {
        from: s.from.clone(),
        to: s.to.clone(),
        risk: s.risk,
        title: s.title.clone(),
        consequence: s.consequence.clone(),
        recommended: s.recommended.clone(),
        sources: s
            .sources
            .iter()
            .filter_map(|id| by_id(id))
            .map(|d| to_ref(&d))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::{EMBEDDED, Seed, assemble, validate};

    fn dataset() -> Dataset {
        let seed = Seed::parse(EMBEDDED).expect("embedded seed parses");
        validate(&seed).expect("embedded seed validates");
        build_dataset(assemble(&seed))
    }

    /// The dataset builds from the embedded seed with the expected inventory.
    #[test]
    fn dataset_builds_from_embedded_seed() {
        let ds = dataset();
        assert_eq!(ds.decisions.len(), 19, "decision count");
        assert_eq!(ds.signals.len(), 3, "signal count");
        assert_eq!(ds.groups.len(), 3, "group count");
        assert_eq!(ds.projects.len(), 7, "project count");
        assert_eq!(ds.account.name, "Marco Reyes");

        // Statuses round-tripped through the wire enums; `superseded` is
        // derived from inbound edges (stored as accepted in the seed).
        assert!(q_by_id(&ds, "status-field").unwrap().status == Status::Accepted);
        assert!(q_by_id(&ds, "scratch-llm-cache").unwrap().status == Status::Rejected);
        assert!(q_by_id(&ds, "status-text").unwrap().status == Status::Superseded);
        assert!(q_by_id(&ds, "status-http").unwrap().status == Status::Superseded);

        // Extras merged in: description on exactly the two known decisions.
        let with_desc: Vec<&str> = ds
            .decisions
            .iter()
            .filter(|d| !d.description.is_empty())
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(with_desc.len(), 2);
        assert!(with_desc.contains(&"status-field") && with_desc.contains(&"shared-oidc"));

        // D9: the two scratch decisions gained the agent co-author.
        let sse = q_by_id(&ds, "scratch-sse").unwrap();
        assert_eq!(sse.authors.len(), 2);
        assert_eq!(sse.authors[1].name, "Claude");
        assert_eq!(sse.authors[1].initial, "✦");
    }

    /// The chain over Vec-edges is still length 3, oldest → newest.
    #[test]
    fn chain_over_edges() {
        let ds = dataset();
        let chain: Vec<String> = q_chain(&ds, "status-text")
            .iter()
            .map(|d| d.id.clone())
            .collect();
        assert_eq!(chain, ["status-http", "status-text", "status-field"]);
        // Same chain regardless of the entry point.
        let via_head: Vec<String> = q_chain(&ds, "status-field")
            .iter()
            .map(|d| d.id.clone())
            .collect();
        assert_eq!(via_head, chain);
    }

    /// feed() for group 0 (platform): the exact id sequence per the timestamp
    /// table in the plan (§6.3), newest first, capped at 7.
    #[test]
    fn feed_for_platform_group() {
        let ds = dataset();
        let ids: Vec<String> = q_feed(&ds, 0).iter().map(|d| d.id.clone()).collect();
        assert_eq!(
            ids,
            [
                "status-field",
                "ratelimit-token",
                "retry-backoff",
                "webhook-idempotent",
                "forward-only-migrations",
                "queue-fanout",
                "s2s-oidc-short",
            ]
        );
    }
}
