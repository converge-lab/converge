//! The expert application service — every model interaction the server
//! initiates lives here, one method per capability, all sharing the same
//! dependencies: the storage seam, the expert [`Registry`] (endpoints +
//! job bindings), and the `expert` agent identity that authorship
//! stamps. Today that is signal detection ([`Expert::detect`]); evidence
//! verification and grounded chat land here as methods when their slices
//! arrive.
//!
//! Detection is the first cross-transport write-path logic (both
//! `decision_add` paths call it post-commit) — exactly where the roadmap
//! parked the application layer. It is E1 by the book: deterministic
//! retrieval feeds the expert job (`converge_expert::signals::discover`),
//! the drafts come back as values, and this service — never the model —
//! stamps provenance and writes. Storage's re-raise ban turns duplicate
//! observations into silent skips, so the pass is idempotent per
//! (source, target, kind).
//!
//! Fail-open throughout: no `signals` job binding → every call is a
//! cheap no-op; any pass failure is a log line, never a caller error —
//! the decision write already succeeded, and enrichment must not
//! retroactively complicate it.

use converge_expert::Registry;
use converge_expert::signals::{Entry, Request, discover};
use converge_storage::{
    AgentId, Author, Decision, DecisionFilter, DecisionId, GroupId, NewSignal, Pagination, Scope,
    Signal, SignalFilter, Storage, StoreError,
};
use tracing::{debug, info, warn};

/// Retrieval breadth: how many cross-project candidates the judgment
/// sees. Bounded by what E1-class models reliably enumerate (live
/// testing: 8 walked cleanly where 25 collapsed); a config knob when a
/// deployment needs to tune it against its triage model.
const CANDIDATES: usize = 8;

/// Words that would otherwise dominate a naive OR-query. The tsvector
/// side stems and drops stopwords on its own; this only keeps the query
/// itself from ballooning.
const STOP: &[&str] = &[
    "the", "and", "with", "that", "this", "from", "into", "over", "only", "not", "are", "for",
    "its", "has", "have", "when", "then", "than", "them", "each", "every", "will", "should",
];

/// What a backfill pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Backfill {
    /// Decisions the pass examined.
    pub examined: usize,
    /// Signals written across all of them.
    pub written: usize,
    /// Decisions whose pass failed (logged, not fatal).
    pub failed: usize,
}

/// The service. Cheap to clone (the registry shares clients, storage
/// shares its pool).
#[derive(Clone)]
pub struct Expert<S> {
    store: S,
    registry: Registry,
    /// The `expert` agent — `produced_by` on everything this writes.
    agent: AgentId,
}

impl<S: Storage + 'static> Expert<S> {
    pub fn new(store: S, registry: Registry, agent: AgentId) -> Self {
        Self {
            store,
            registry,
            agent,
        }
    }

    /// A decision landed — judge its impact, asynchronously. Returns at
    /// once; the caller's write path never waits on a model.
    pub fn detect(&self, decision: DecisionId) {
        if self.registry.job("signals").is_none() {
            return;
        }
        let expert = self.clone();
        tokio::spawn(async move {
            match expert.run(decision).await {
                Ok(written) if written > 0 => {
                    info!(%decision, written, "signal detection finished");
                }
                Ok(_) => debug!(%decision, "signal detection found nothing"),
                // Enrichment is best-effort: the decision is already
                // written, so a failed pass is a log line, not an error.
                Err(error) => warn!(%decision, %error, "signal detection failed"),
            }
        });
    }

    /// Run detection over every existing decision (optionally narrowed)
    /// — the day-one story for a deployment that already has memory.
    /// Sequential (one model call in flight), oldest first, fail-open
    /// per decision: a failed pass is counted and logged, never fatal.
    /// The re-raise ban absorbs re-running over already-judged pairs, so
    /// backfill is idempotent.
    pub async fn backfill(&self, filter: DecisionFilter) -> Result<Backfill, StoreError> {
        let mut stats = Backfill::default();
        if self.registry.job("signals").is_none() {
            return Ok(stats);
        }
        let mut decisions = self
            .store
            .decision_list(Scope::System, filter, Pagination::default())
            .await?;
        // Newest-first from storage; judge in capture order.
        decisions.reverse();
        for decision in decisions {
            stats.examined += 1;
            match self.run(decision.id).await {
                Ok(written) => {
                    stats.written += written;
                    if written > 0 {
                        info!(decision = %decision.id, written, title = %decision.title, "backfill: signals raised");
                    }
                }
                Err(error) => {
                    stats.failed += 1;
                    warn!(decision = %decision.id, %error, "backfill: pass failed — continuing");
                }
            }
        }
        Ok(stats)
    }

    /// The detection pass itself, awaitable — public so tests (and a
    /// future backfill command) can run it to completion.
    pub async fn run(&self, id: DecisionId) -> Result<usize, StoreError> {
        let Some(client) = self.registry.job("signals") else {
            return Ok(0);
        };
        let subject = self
            .store
            .decision_get(Scope::System, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        // Detection never leaves the subject's group — the ACL boundary
        // is also the coordination boundary.
        let group = self
            .store
            .project_get(Scope::System, subject.project_id)
            .await?
            .ok_or(StoreError::NotFound)?
            .group_id;

        let candidates = self.retrieve(&subject, group).await?;
        if candidates.is_empty() {
            return Ok(0);
        }
        let mut signals = self.touching(id).await?;
        for candidate in &candidates {
            let mut more = self.touching(candidate.id).await?;
            more.retain(|s| !signals.iter().any(|k: &Signal| k.id == s.id));
            signals.append(&mut more);
        }

        let request = Request {
            decision: self.entry(subject).await?,
            candidates: {
                let mut entries = Vec::with_capacity(candidates.len());
                for candidate in candidates {
                    entries.push(self.entry(candidate).await?);
                }
                entries
            },
            signals,
        };

        let drafts = discover(&client, &request)
            .await
            .map_err(|e| StoreError::Backend(format!("signals job: {e}")))?;

        let mut written = 0;
        for draft in drafts {
            let new = NewSignal {
                source: id,
                targets: draft.targets,
                kind: draft.kind,
                tier: draft.tier,
                title: draft.title,
                text: draft.text,
                consequence: draft.consequence,
                recommendation: draft.recommendation,
                produced_by: Author::Agent(self.agent),
            };
            match self.store.signal_add(Scope::System, new).await {
                Ok(_) => written += 1,
                // Already observed (possibly dismissed): the re-raise
                // ban working as designed — not an error.
                Err(StoreError::Conflict(_)) => {
                    debug!(source = %id, "draft already observed — skipped")
                }
                Err(error) => warn!(source = %id, %error, "draft rejected by storage"),
            }
        }
        Ok(written)
    }

    /// Deterministic retrieval: an OR-query of the subject's content
    /// words over the full-text index, **within the subject's group**,
    /// minus the subject itself and its own project (the judgment is
    /// cross-project by contract — same-project hits would waste
    /// candidate slots).
    async fn retrieve(
        &self,
        subject: &Decision,
        group: GroupId,
    ) -> Result<Vec<Decision>, StoreError> {
        let text = format!(
            "{} {} {}",
            subject.title,
            subject.summary,
            subject.context.as_deref().unwrap_or_default()
        )
        .to_lowercase();
        let mut words: Vec<&str> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3 && !STOP.contains(w))
            .collect();
        words.sort_unstable();
        words.dedup();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let query = words.join(" or ");
        let hits = self
            .store
            .decision_search(
                Scope::System,
                &query,
                DecisionFilter {
                    group: Some(group),
                    ..Default::default()
                },
                Some((CANDIDATES * 3) as u32),
            )
            .await?;
        Ok(hits
            .into_iter()
            .filter(|d| d.id != subject.id && d.project_id != subject.project_id)
            .take(CANDIDATES)
            .collect())
    }

    /// Signals touching a decision on either end, newest first.
    async fn touching(&self, id: DecisionId) -> Result<Vec<Signal>, StoreError> {
        self.store
            .signal_list(
                Scope::System,
                SignalFilter {
                    decision: Some(id),
                    ..Default::default()
                },
                Pagination::default(),
            )
            .await
    }

    /// A decision as the job sees it: + project name + one-hop edges.
    async fn entry(&self, decision: Decision) -> Result<Entry, StoreError> {
        let project = self
            .store
            .project_get(Scope::System, decision.project_id)
            .await?
            .map(|p| p.name)
            .unwrap_or_else(|| decision.project_id.to_string());
        let edges = self
            .store
            .decision_edges(Scope::System, decision.id)
            .await?
            .unwrap_or_default();
        Ok(Entry {
            decision,
            project,
            edges,
        })
    }
}
