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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use converge_expert::signals::{Entry, Request, discover};
use converge_expert::{Registry, Turn};
use converge_storage::{
    AgentId, Author, Decision, DecisionFilter, DecisionId, GroupId, NewSignal, Pagination, Scope,
    Signal, SignalFilter, SignalStatus, Storage, StoreError,
};
use futures::Stream;
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

/// The context-policy budgets for [`Expert::ask`] (chars ≈ tokens × 4).
/// The expert is *at home* in its window — these are generous by design
/// (the inject's compactness rule explicitly does not apply here).
mod budget {
    /// The always-present index: one line per decision. Groups whose
    /// index outgrows this degrade to the newest lines that fit.
    pub const INDEX: usize = 120_000;
    /// Tier-2 full bodies, total.
    pub const BODIES: usize = 240_000;
    /// Tier-2 selection width: search hits by the question.
    pub const HITS: usize = 12;
    /// Tier-2 selection width: open-signal endpoint decisions.
    pub const ENDPOINTS: usize = 8;
    /// Tier-2 selection width: newest decisions regardless of match.
    pub const RECENT: usize = 3;
    /// How long a group's assembled index stays warm. A short TTL keeps
    /// invalidation honest until writes carry a corpus revision.
    pub const TTL_SECS: u64 = 60;
}

/// A group's assembled tier-1 context, memoized.
struct Index {
    built: Instant,
    text: Arc<String>,
    decisions: usize,
    signals: usize,
}

/// What an ask grounds in — counts for the UI, the prompt for the model.
pub struct Briefing {
    pub system: String,
    pub decisions: usize,
    pub signals: usize,
}

/// The service. Cheap to clone (the registry shares clients, storage
/// shares its pool, the index cache is shared).
#[derive(Clone)]
pub struct Expert<S> {
    store: S,
    registry: Registry,
    /// The `expert` agent — `produced_by` on everything this writes.
    agent: AgentId,
    /// Per-group memoized index (the stable prompt prefix).
    indexes: Arc<Mutex<HashMap<GroupId, Index>>>,
}

impl<S: Storage + 'static> Expert<S> {
    pub fn new(store: S, registry: Registry, agent: AgentId) -> Self {
        Self {
            store,
            registry,
            agent,
            indexes: Arc::new(Mutex::new(HashMap::new())),
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

    /// Grounded chat over the group's decision memory: assemble the
    /// briefing (rich index always + selected full bodies — the context
    /// policy), then stream the model's answer. Everything is read
    /// under the **caller's scope**: the expert can never see past the
    /// asker. `history` is the prior turns, client-held (stateless
    /// server, like every surface here).
    pub async fn ask(
        &self,
        scope: Scope,
        group: GroupId,
        history: Vec<Turn>,
        question: &str,
    ) -> Result<
        (
            Briefing,
            impl Stream<Item = Result<String, converge_expert::Error>> + Send + use<S>,
        ),
        StoreError,
    > {
        // Visibility refuses first (an invisible group is NotFound,
        // like every surface); only then deployment readiness.
        self.store
            .group_get(scope, group)
            .await?
            .ok_or(StoreError::NotFound)?;
        let Some(client) = self.registry.job("ask") else {
            return Err(StoreError::Unavailable(
                "no `ask` model is configured ([expert.jobs] ask = …)".into(),
            ));
        };

        let briefing = self.briefing(scope, group, question).await?;
        let mut turns = history;
        turns.push(Turn {
            user: true,
            text: question.to_string(),
        });
        let stream = client
            .converse(&briefing.system, turns)
            .await
            .map_err(|e| StoreError::Backend(format!("ask job: {e}")))?;
        Ok((briefing, stream))
    }

    /// The briefing per the context policy: memoized index (tier 1),
    /// question-selected bodies (tier 2), stable-first ordering so the
    /// provider prompt cache pays for the prefix.
    async fn briefing(
        &self,
        scope: Scope,
        group: GroupId,
        question: &str,
    ) -> Result<Briefing, StoreError> {
        let cached = {
            let indexes = self.indexes.lock().expect("index cache lock");
            indexes.get(&group).and_then(|i| {
                (i.built.elapsed() < Duration::from_secs(budget::TTL_SECS))
                    .then(|| (i.text.clone(), i.decisions, i.signals))
            })
        };
        let (index, decisions, signals) = match cached {
            Some(warm) => warm,
            None => {
                let (text, decisions, signals) = self.index(scope, group).await?;
                let text = Arc::new(text);
                self.indexes.lock().expect("index cache lock").insert(
                    group,
                    Index {
                        built: Instant::now(),
                        text: text.clone(),
                        decisions,
                        signals,
                    },
                );
                (text, decisions, signals)
            }
        };
        let bodies = self.bodies(scope, group, question).await?;
        let system = format!(
            "You are the Converge expert: this group's decision memory, made \
             conversational. Answer from the records below — the index is \
             everything that exists, the full records are the ones most \
             relevant right now. Cite decisions inline by id in square \
             brackets, e.g. [01ABCDEFGHJKMNPQRSTVWXYZ01]. When the memory \
             doesn't answer the question, say so plainly instead of \
             guessing; point at the nearest recorded decision when one is \
             adjacent. Never invent ids, titles, or history.\n\n{index}\n{bodies}",
        );
        Ok(Briefing {
            system,
            decisions,
            signals,
        })
    }

    /// Tier 1: every visible decision as one line, plus open signals.
    /// Degrades newest-first when a giant corpus outgrows the budget.
    async fn index(
        &self,
        scope: Scope,
        group: GroupId,
    ) -> Result<(String, usize, usize), StoreError> {
        let all = self
            .store
            .decision_list(
                scope,
                DecisionFilter {
                    group: Some(group),
                    ..Default::default()
                },
                Pagination::default(),
            )
            .await?;
        let open = {
            let mut signals = self
                .store
                .signal_list(
                    scope,
                    SignalFilter {
                        status: Some(SignalStatus::Proposed),
                        ..Default::default()
                    },
                    Pagination::default(),
                )
                .await?;
            // The unfiltered list spans the caller's world; keep the
            // group's own (either end touching one of its decisions).
            let ids: std::collections::HashSet<_> = all.iter().map(|d| d.id).collect();
            signals
                .retain(|s| ids.contains(&s.source) || s.targets.iter().any(|t| ids.contains(t)));
            signals
        };
        let counts = (all.len(), open.len());
        let mut text = String::from("## Decision index (newest first)\n");
        for d in &all {
            let line = format!(
                "- [{}] ({:?}, {}) {} — {}\n",
                d.id, d.status, d.project_id, d.title, d.summary
            );
            if text.len() + line.len() > budget::INDEX {
                text.push_str("- … (index truncated: oldest decisions omitted)\n");
                break;
            }
            text.push_str(&line);
        }
        if !open.is_empty() {
            text.push_str("\n## Open signals (awaiting judgment)\n");
            for s in &open {
                text.push_str(&format!(
                    "- [{}] {:?}/{} {} — affects {}\n",
                    s.source,
                    s.tier,
                    s.kind,
                    s.title,
                    s.targets
                        .iter()
                        .map(|t| format!("[{t}]"))
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
            }
        }
        Ok((text, counts.0, counts.1))
    }

    /// Tier 2: full records for what matters *now* — question-relevant
    /// search hits ∪ open-signal endpoints ∪ the newest few, deduped,
    /// budget-capped.
    async fn bodies(
        &self,
        scope: Scope,
        group: GroupId,
        question: &str,
    ) -> Result<String, StoreError> {
        let mut picked: Vec<Decision> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let text = question.to_lowercase();
        let mut words: Vec<&str> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3 && !STOP.contains(w))
            .collect();
        words.sort_unstable();
        words.dedup();
        if !words.is_empty() {
            let hits = self
                .store
                .decision_search(
                    scope,
                    &words.join(" or "),
                    DecisionFilter {
                        group: Some(group),
                        ..Default::default()
                    },
                    Some(budget::HITS as u32),
                )
                .await?;
            for d in hits {
                if seen.insert(d.id) {
                    picked.push(d);
                }
            }
        }

        // Open-signal endpoints: what's contested is always relevant.
        let open = self
            .store
            .signal_list(
                scope,
                SignalFilter {
                    status: Some(SignalStatus::Proposed),
                    ..Default::default()
                },
                Pagination::default(),
            )
            .await?;
        let endpoints: Vec<DecisionId> = open
            .iter()
            .flat_map(|s| std::iter::once(s.source).chain(s.targets.iter().copied()))
            .take(budget::ENDPOINTS)
            .collect();
        for id in endpoints {
            if seen.contains(&id) {
                continue;
            }
            if let Some(d) = self.store.decision_get(scope, id).await?
                && seen.insert(d.id)
            {
                picked.push(d);
            }
        }

        let newest = self
            .store
            .decision_list(
                scope,
                DecisionFilter {
                    group: Some(group),
                    ..Default::default()
                },
                Pagination {
                    limit: Some(budget::RECENT as u32),
                    cursor: None,
                },
            )
            .await?;
        for d in newest {
            if seen.insert(d.id) {
                picked.push(d);
            }
        }

        let mut text = String::from("\n## Selected decisions (full records)\n");
        for d in picked {
            let record = render(&d);
            if text.len() + record.len() > budget::BODIES {
                break;
            }
            text.push_str(&record);
        }
        Ok(text)
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

/// A decision as the ask's tier-2 record: the full ADR, plain text.
fn render(d: &Decision) -> String {
    let mut out = format!(
        "\n### [{}] {} ({:?}, project {})\n{}\n",
        d.id, d.title, d.status, d.project_id, d.summary
    );
    if let Some(context) = &d.context {
        out.push_str(&format!("Context: {context}\n"));
    }
    if let Some(consequences) = &d.consequences {
        out.push_str(&format!("Consequences: {consequences}\n"));
    }
    for alt in &d.alternatives {
        out.push_str(&format!(
            "Rejected: {} — {}\n",
            alt.option, alt.why_rejected
        ));
    }
    out
}
