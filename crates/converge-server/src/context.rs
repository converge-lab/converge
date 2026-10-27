//! What a harness session is shown: the session-start block and the
//! per-prompt signal frame. Rendered here, not in the CLI, so the
//! wording ships with a deploy and the preview shows the exact bytes a
//! session gets.
//!
//! The wording lives in `templates/`; this module decides what goes in
//! and in which order. The wording is contract-like — agents act on it —
//! so it changes carefully.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::sync::OnceLock;

use converge_storage::{
    DecisionFilter, DecisionId, Pagination, ProjectId, Scope, Signal, SignalFilter, SignalId,
    SignalStatus, Storage, StoreError, Tier, UserId,
};
use minijinja::{Environment, context};
use serde::Serialize;

/// How much of the index one session start carries.
const DECISIONS: u32 = 30;
const SIGNALS: u32 = 10;

fn templates() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.add_template("session.md", include_str!("../templates/session.md"))
            .expect("session template parses");
        env.add_template("signals.md", include_str!("../templates/signals.md"))
            .expect("signals template parses");
        env
    })
}

fn render(name: &str, ctx: minijinja::Value) -> String {
    templates()
        .get_template(name)
        .and_then(|t| t.render(ctx))
        .expect("templates render: their inputs are fixed shapes")
        .trim_end()
        .to_string()
}

/// The session-start block, its visible line, and what it listed — the
/// ids a session receipts once the block is in front of the model.
#[derive(Debug, Serialize)]
pub struct Session {
    pub context: String,
    pub line: String,
    pub decisions: Vec<DecisionId>,
    pub signals: Vec<SignalId>,
}

/// A decision as the block lists it.
#[derive(Debug, Clone, Serialize)]
struct Recorded {
    id: DecisionId,
    title: String,
    status: String,
    /// Never shown to this user before, in any session on any machine.
    new: bool,
}

/// An open signal as the block shows it.
#[derive(Debug, Clone, Serialize)]
struct Open {
    id: SignalId,
    title: String,
    /// The server holds no receipt for it from this user.
    new: bool,
}

fn page<Id>(limit: u32) -> Pagination<Id> {
    Pagination {
        limit: Some(limit),
        cursor: None,
    }
}

/// The block for `user` in `project`, or `None` when the project is not
/// visible to them — which answers exactly like a missing one.
pub async fn session<S: Storage>(
    store: &S,
    user: UserId,
    project: ProjectId,
) -> Result<Option<Session>, StoreError> {
    let scope = Scope::User(user);
    let Some(found) = store.project_get(scope, project).await? else {
        return Ok(None);
    };
    // Both lists, and for each the half this user has never been shown:
    // the marks fall out of the difference, so they are the same on
    // every machine.
    let recorded = DecisionFilter {
        project: Some(project),
        ..Default::default()
    };
    // Conflicts only: the expert raises nothing else, and what it raised
    // below that before is no longer worth a session's attention.
    let open = SignalFilter {
        project: Some(project),
        status: Some(SignalStatus::Proposed),
        tier: Some(Tier::Conflict),
        ..Default::default()
    };
    let (all, unseen, listed, fresh) = tokio::try_join!(
        store.decision_list(scope, recorded.clone(), page(DECISIONS)),
        store.decision_list(
            scope,
            DecisionFilter {
                unseen: true,
                ..recorded
            },
            page(DECISIONS)
        ),
        store.signal_list(scope, open.clone(), page(SIGNALS)),
        store.signal_list(
            scope,
            SignalFilter {
                unseen: true,
                ..open
            },
            page(SIGNALS)
        ),
    )?;
    let unseen: BTreeSet<DecisionId> = unseen.iter().map(|d| d.id).collect();
    let fresh: BTreeSet<SignalId> = fresh.iter().map(|s| s.id).collect();
    let decisions: Vec<Recorded> = all
        .into_iter()
        .map(|d| Recorded {
            new: unseen.contains(&d.id),
            id: d.id,
            title: d.title,
            status: format!("{:?}", d.status).to_lowercase(),
        })
        .collect();
    let signals: Vec<Open> = listed
        .into_iter()
        .map(|s| Open {
            new: fresh.contains(&s.id),
            id: s.id,
            title: s.title,
        })
        .collect();
    let (context, line) = session_block(project, &found.name, decisions.clone(), signals.clone());
    Ok(Some(Session {
        context,
        line,
        decisions: decisions.iter().map(|d| d.id).collect(),
        signals: signals.iter().map(|s| s.id).collect(),
    }))
}

/// How many signals one prompt is handed — the poll's own cap.
const SAMPLE: u32 = 3;

/// The per-prompt frame as it would read now, built from the project's
/// newest open signals and shown oldest first, as a claim hands them
/// over. Nothing is claimed or receipted.
pub async fn sample<S: Storage>(
    store: &S,
    user: UserId,
    project: ProjectId,
) -> Result<Option<(String, String)>, StoreError> {
    let open = SignalFilter {
        project: Some(project),
        status: Some(SignalStatus::Proposed),
        tier: Some(Tier::Conflict),
        ..Default::default()
    };
    let mut newest = store
        .signal_list(Scope::User(user), open, page(SAMPLE))
        .await?;
    newest.reverse();
    Ok(signals(&newest))
}

/// The block and its visible line. This listing is deliberately
/// unfiltered by receipts: it is the one place every open conflict shows,
/// whatever a poll handed out or dropped.
fn session_block(
    project: ProjectId,
    name: &str,
    mut decisions: Vec<Recorded>,
    mut signals: Vec<Open>,
) -> (String, String) {
    let new_decisions = decisions.iter().filter(|d| d.new).count();
    let new_signals = signals.iter().filter(|s| s.new).count();
    // The list limits cap what we can count; say "N+" at the cap
    // instead of understating a bigger corpus as exactly N.
    let counted = |n: usize, cap: u32| {
        if n >= cap as usize {
            format!("{n}+")
        } else {
            n.to_string()
        }
    };
    let fresh = |n: usize| match n {
        0 => String::new(),
        n => format!(" ({n} new)"),
    };
    let line = format!(
        "Converge: \"{name}\" — {} decision(s){}, {} open conflict(s){} ✓",
        counted(decisions.len(), DECISIONS),
        fresh(new_decisions),
        counted(signals.len(), SIGNALS),
        fresh(new_signals),
    );
    // What is new to this reader leads, newest first within each half.
    decisions.sort_by_key(|d| (Reverse(d.new), Reverse(d.id)));
    signals.sort_by_key(|s| (Reverse(s.new), Reverse(s.id)));
    let context = render(
        "session.md",
        context! {
            project => context! { id => project, name },
            decisions,
            signals,
            new_decisions,
            new_signals,
        },
    );
    (context, line)
}

/// A signal as the per-prompt frame shows it.
#[derive(Serialize)]
struct Arrived<'a> {
    id: SignalId,
    source: DecisionId,
    targets: &'a [DecisionId],
    title: &'a str,
    text: &'a str,
    consequence: Option<&'a str>,
    recommendation: Option<&'a str>,
}

/// An optional field as the frame shows it: trimmed, and absent if blank.
fn note(field: &Option<String>) -> Option<&str> {
    field.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// The per-prompt frame: what arrived, framed so the model reads it as
/// information from Converge — never as the user's words, and never as
/// an instruction to act on by itself. The wording is factual on
/// purpose: text shaped like an out-of-band command trips a model's
/// injection defences and gets shown to the user as suspicious instead
/// of read. `None` when nothing arrived.
pub fn signals(signals: &[Signal]) -> Option<(String, String)> {
    if signals.is_empty() {
        return None;
    }
    let n = signals.len();
    let line = format!(
        "Converge: {n} new conflict{}",
        if n == 1 { "" } else { "s" }
    );
    let arrived: Vec<Arrived> = signals
        .iter()
        .map(|s| Arrived {
            id: s.id,
            source: s.source,
            targets: &s.targets,
            title: &s.title,
            text: s.text.trim(),
            consequence: note(&s.consequence),
            recommendation: note(&s.recommendation),
        })
        .collect();
    let context = render("signals.md", context! { signals => arrived });
    Some((context, line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use converge_storage::Author;
    use time::OffsetDateTime;

    fn open(id: &str, title: &str, new: bool) -> Open {
        Open {
            id: id.parse().unwrap(),
            title: title.into(),
            new,
        }
    }

    fn recorded(id: &str, title: &str, new: bool) -> Recorded {
        Recorded {
            id: id.parse().unwrap(),
            title: title.into(),
            status: "accepted".into(),
            new,
        }
    }

    fn project() -> ProjectId {
        "01J00000000000000000000000".parse().unwrap()
    }

    /// The listed conflicts, in order: the lines under their header.
    fn conflict_lines(block: &str) -> Vec<&str> {
        block
            .split_once("Open conflicts")
            .map_or("", |(_, rest)| rest)
            .lines()
            .filter(|l| l.starts_with("- "))
            .collect()
    }

    #[test]
    fn an_empty_project_says_how_to_start() {
        let (block, line) = session_block(project(), "p", vec![], vec![]);
        assert_eq!(
            block,
            format!(
                "## Converge memory — project \"p\" ({p})\n\
                 This working tree is bound to converge project `{p}`. No decisions are \
                 recorded yet; when the user settles something others will need to know, \
                 `decision_add` keeps it for the team. A hook attaches the exchange that \
                 decided it, and completes a bare `path:lines` into a code citation.",
                p = project()
            )
        );
        assert_eq!(
            line,
            "Converge: \"p\" — 0 decision(s), 0 open conflict(s) ✓"
        );
    }

    #[test]
    fn the_block_is_byte_for_byte() {
        let decisions = vec![
            recorded("01J0000000000000000000000A", "settled last week", false),
            recorded("01J0000000000000000000000B", "settled today", true),
        ];
        let signals = vec![open("01J00000000000000000000003", "hi", true)];
        let (block, _) = session_block(project(), "p", decisions, signals);
        let p = project();
        assert_eq!(
            block,
            format!(
                "## Converge memory — project \"p\" ({p})\n\
                 This working tree is bound to converge project `{p}`. The decisions below \
                 are the team's answers so far: `decision_get` shows why one was made, worth \
                 reading before going against it, and `decision_add` (with `supersedes` if \
                 it replaces one) keeps what the user settles next. A hook attaches the \
                 exchange that decided it, and completes a bare `path:lines` into a code \
                 citation.\n\
                 \n\
                 Decisions (← NEW = not shown to you before, in any session):\n\
                 - settled today [accepted] (01J0000000000000000000000B) ← NEW\n\
                 - settled last week [accepted] (01J0000000000000000000000A)\n\
                 \n\
                 Open conflicts (nobody has judged them yet; ← NEW = not shown to you \
                 before, in any session). In a conflict, a decision breaks a contract that \
                 earlier decisions created, and the decisions cannot both apply. It is worth \
                 raising with the user before building on either. Whether a conflict is real \
                 is their call, recorded with `signal_resolve`; `signal_list` has the full \
                 text:\n\
                 - hi (01J00000000000000000000003) ← NEW"
            )
        );
    }

    #[test]
    fn what_the_user_was_never_shown_leads() {
        let decisions = vec![
            recorded("01J0000000000000000000000A", "settled last week", false),
            recorded("01J0000000000000000000000B", "settled today", true),
        ];
        let (_, line) = session_block(project(), "p", decisions.clone(), vec![]);
        assert_eq!(
            line,
            "Converge: \"p\" — 2 decision(s) (1 new), 0 open conflict(s) ✓"
        );
        // Nothing new: no mark, no legend, no parenthetical.
        let seen = decisions
            .into_iter()
            .map(|d| Recorded { new: false, ..d })
            .collect();
        let (block, line) = session_block(project(), "p", seen, vec![]);
        assert!(!block.contains("NEW"), "{block}");
        assert_eq!(
            line,
            "Converge: \"p\" — 2 decision(s), 0 open conflict(s) ✓"
        );

        // The oldest conflict is the one never shown: it leads anyway.
        let lo = open("01J00000000000000000000001", "lo", true);
        let mid = open("01J00000000000000000000002", "mid", false);
        let hi = open("01J00000000000000000000003", "hi", false);
        let one = vec![recorded("01J0000000000000000000000A", "one", false)];
        let (block, line) = session_block(
            project(),
            "p",
            one.clone(),
            vec![lo.clone(), mid.clone(), hi.clone()],
        );
        assert_eq!(
            conflict_lines(&block),
            [
                format!("- lo ({}) ← NEW", lo.id),
                format!("- hi ({})", hi.id),
                format!("- mid ({})", mid.id),
            ]
        );
        assert_eq!(
            line,
            "Converge: \"p\" — 1 decision(s), 3 open conflict(s) (1 new) ✓"
        );
        // Everything receipted: newest first.
        let seen = [lo, mid, hi]
            .into_iter()
            .map(|s| Open { new: false, ..s })
            .collect();
        let (block, _) = session_block(project(), "p", one, seen);
        assert!(!block.contains("NEW"), "{block}");
        let lines = conflict_lines(&block);
        assert!(
            lines[0].contains(" hi ") && lines[1].contains(" mid ") && lines[2].contains(" lo ")
        );
    }

    /// Past 10,000 characters Claude Code 2.1.288 stops inlining a hook's
    /// context — it saves it to a file and shows a 2 KB preview — and Codex
    /// 0.155.1 does the same past 2,500 estimated tokens (bytes / 4). A
    /// full index with long titles must stay well inside both.
    #[test]
    fn a_full_block_stays_inline() {
        let title = "x".repeat(100);
        let decisions = (0..DECISIONS)
            .map(|_| Recorded {
                id: DecisionId::new(),
                title: title.clone(),
                status: "superseded".into(),
                new: true,
            })
            .collect();
        let signals = (0..SIGNALS)
            .map(|_| Open {
                id: SignalId::new(),
                title: title.clone(),
                new: true,
            })
            .collect();
        let (block, _) = session_block(project(), &title, decisions, signals);
        assert!(
            block.len() <= 8_000,
            "a full block is {} bytes",
            block.len()
        );
    }

    fn arrived(
        id: &str,
        title: &str,
        consequence: Option<&str>,
        recommendation: Option<&str>,
    ) -> Signal {
        let decision = |s: &str| s.parse::<DecisionId>().unwrap();
        Signal {
            id: id.parse().unwrap(),
            source: decision("01J00000000000000000000000"),
            targets: vec![decision("01J00000000000000000000001")],
            kind: "broken_contract".into(),
            tier: Tier::Conflict,
            status: SignalStatus::Proposed,
            title: title.into(),
            text: format!("{title} bears on the other one.\n"),
            consequence: consequence.map(str::to_owned),
            recommendation: recommendation.map(str::to_owned),
            produced_by: Author::User("01J00000000000000000000002".parse().unwrap()),
            resolved_by: None,
            captured_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_frame_says_what_arrived_and_whose_words_they_are() {
        assert!(signals(&[]).is_none());
        let one = arrived("01J00000000000000000000003", "one", None, None);
        let (context, line) = signals(std::slice::from_ref(&one)).unwrap();
        assert_eq!(line, "Converge: 1 new conflict");
        assert_eq!(
            context,
            "Converge: 1 new conflict since your last prompt. In a conflict, a decision \
             breaks a contract that earlier decisions created, and the decisions cannot \
             both apply. Converge Expert found these conflicts, and some of the decisions \
             can come from other people's sessions. The user decides whether each conflict \
             is real, so it is worth raising with them before building on either \
             decision.\n\
             - one (01J00000000000000000000003)\n  \
             one bears on the other one.\n  \
             Decision 01J00000000000000000000000 breaks the contract of \
             01J00000000000000000000001.\n\
             `decision_get` and `signal_list` hold the full record, and `signal_resolve` \
             records the user's verdict once they give it."
        );

        let two = arrived(
            "01J00000000000000000000004",
            "two",
            Some(" billing loses the currency "),
            Some(" talk to billing "),
        );
        let (context, line) = signals(&[one, two]).unwrap();
        assert_eq!(line, "Converge: 2 new conflicts");
        assert!(
            context.starts_with("Converge: 2 new conflicts since"),
            "{context}"
        );
        assert!(
            context.contains("\n  Consequence: billing loses the currency\n"),
            "{context}"
        );
        assert!(
            context.contains("\n  Recommendation: talk to billing\n"),
            "{context}"
        );
    }
}
