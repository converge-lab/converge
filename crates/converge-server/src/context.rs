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
    tier: Tier,
    kind: String,
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
    let open = SignalFilter {
        project: Some(project),
        status: Some(SignalStatus::Proposed),
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
            tier: s.tier,
            kind: s.kind,
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
        ..Default::default()
    };
    let mut newest = store
        .signal_list(Scope::User(user), open, page(SAMPLE))
        .await?;
    newest.reverse();
    Ok(signals(&newest))
}

/// The block and its visible line. This listing is deliberately
/// unfiltered by receipts: it is the one place every open signal shows,
/// whatever a poll handed out or dropped.
fn session_block(
    project: ProjectId,
    name: &str,
    mut decisions: Vec<Recorded>,
    mut signals: Vec<Open>,
) -> (String, String) {
    let new_decisions = decisions.iter().filter(|d| d.new).count();
    let new_signals = signals.iter().filter(|s| s.new).count();
    let conflicts = signals.iter().filter(|s| s.tier == Tier::Conflict).count();
    // The list limits cap what we can count; say "N+" at the cap
    // instead of understating a bigger corpus as exactly N.
    let counted = |n: usize, cap: u32| {
        if n >= cap as usize {
            format!("{n}+")
        } else {
            n.to_string()
        }
    };
    let mut detail = Vec::new();
    if new_signals > 0 {
        detail.push(format!("{new_signals} new"));
    }
    if conflicts > 0 {
        detail.push(format!("{conflicts} conflict"));
    }
    let line = format!(
        "Converge: \"{name}\" — {} decision(s){}, {} open signal(s){} ✓",
        counted(decisions.len(), DECISIONS),
        match new_decisions {
            0 => String::new(),
            n => format!(" ({n} new)"),
        },
        counted(signals.len(), SIGNALS),
        match detail.is_empty() {
            true => String::new(),
            false => format!(" ({})", detail.join(", ")),
        },
    );
    // What is new to this reader leads, newest first within each half;
    // among signals a conflict leads within that.
    decisions.sort_by_key(|d| (Reverse(d.new), Reverse(d.id)));
    signals.sort_by_key(|s| (Reverse(s.new), Reverse(s.tier), Reverse(s.id)));
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
    tier: Tier,
    kind: &'a str,
    title: &'a str,
    text: &'a str,
    recommendation: Option<&'a str>,
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
    let conflicts = signals.iter().filter(|s| s.tier == Tier::Conflict).count();
    let line = format!(
        "Converge: {n} new signal{}{}",
        if n == 1 { "" } else { "s" },
        match conflicts {
            0 => String::new(),
            c => format!(" ({c} conflict)"),
        }
    );
    let arrived: Vec<Arrived> = signals
        .iter()
        .map(|s| Arrived {
            id: s.id,
            tier: s.tier,
            kind: &s.kind,
            title: &s.title,
            text: s.text.trim(),
            recommendation: s
                .recommendation
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty()),
        })
        .collect();
    let context = render("signals.md", context! { signals => arrived, conflicts });
    Some((context, line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use converge_storage::Author;
    use time::OffsetDateTime;

    fn open(id: &str, tier: Tier, title: &str, new: bool) -> Open {
        Open {
            id: id.parse().unwrap(),
            tier,
            kind: "dependency".into(),
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
        assert_eq!(line, "Converge: \"p\" — 0 decision(s), 0 open signal(s) ✓");
    }

    #[test]
    fn the_block_is_byte_for_byte() {
        let decisions = vec![
            recorded("01J0000000000000000000000A", "settled last week", false),
            recorded("01J0000000000000000000000B", "settled today", true),
        ];
        let signals = vec![open(
            "01J00000000000000000000003",
            Tier::Conflict,
            "hi",
            true,
        )];
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
                 - settled today [accepted] ← NEW\n\
                 - settled last week [accepted]\n\
                 \n\
                 Proposed signals (observations about this project's decisions that nobody \
                 has judged yet; ← NEW = not shown to you before, in any session). A \
                 conflict means two decisions can't both stand, so it is worth raising with \
                 the user before building on either. Whether a signal holds is their call, \
                 recorded with `signal_resolve`; `signal_list` has the full text:\n\
                 - [conflict/dependency] hi (01J00000000000000000000003) ← NEW"
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
            "Converge: \"p\" — 2 decision(s) (1 new), 0 open signal(s) ✓"
        );
        // Nothing new: no mark, no legend, no parenthetical.
        let seen = decisions
            .into_iter()
            .map(|d| Recorded { new: false, ..d })
            .collect();
        let (block, line) = session_block(project(), "p", seen, vec![]);
        assert!(!block.contains("NEW"), "{block}");
        assert_eq!(line, "Converge: \"p\" — 2 decision(s), 0 open signal(s) ✓");

        let lo = open("01J00000000000000000000001", Tier::Conflict, "lo", false);
        let mid = open("01J00000000000000000000002", Tier::Watch, "mid", false);
        let hi = open("01J00000000000000000000003", Tier::Coordinate, "hi", true);
        let one = vec![recorded("01J0000000000000000000000A", "one", false)];
        let (block, line) = session_block(
            project(),
            "p",
            one.clone(),
            vec![lo.clone(), mid.clone(), hi.clone()],
        );
        let lines: Vec<&str> = block.lines().filter(|l| l.starts_with("- [")).collect();
        assert_eq!(
            lines,
            [
                format!("- [coordinate/dependency] hi ({}) ← NEW", hi.id),
                format!("- [conflict/dependency] lo ({})", lo.id),
                format!("- [watch/dependency] mid ({})", mid.id),
            ]
        );
        assert_eq!(
            line,
            "Converge: \"p\" — 1 decision(s), 3 open signal(s) (1 new, 1 conflict) ✓"
        );
        // Everything receipted: the order is tier, then newest.
        let seen = [lo, mid, hi]
            .into_iter()
            .map(|s| Open { new: false, ..s })
            .collect();
        let (block, _) = session_block(project(), "p", one, seen);
        assert!(!block.contains("NEW"), "{block}");
        let lines: Vec<&str> = block.lines().filter(|l| l.starts_with("- [")).collect();
        assert!(
            lines[0].contains(" lo ") && lines[1].contains(" hi ") && lines[2].contains(" mid ")
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
                tier: Tier::Coordinate,
                kind: "scope_narrowing".into(),
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

    fn arrived(id: &str, tier: Tier, title: &str, recommendation: Option<&str>) -> Signal {
        let decision = |s: &str| s.parse::<DecisionId>().unwrap();
        Signal {
            id: id.parse().unwrap(),
            source: decision("01J00000000000000000000000"),
            targets: vec![decision("01J00000000000000000000001")],
            kind: "dependency".into(),
            tier,
            status: SignalStatus::Proposed,
            title: title.into(),
            text: format!("{title} bears on the other one.\n"),
            consequence: None,
            recommendation: recommendation.map(str::to_owned),
            produced_by: Author::User("01J00000000000000000000002".parse().unwrap()),
            resolved_by: None,
            captured_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_frame_says_what_arrived_and_whose_words_they_are() {
        assert!(signals(&[]).is_none());
        let one = arrived("01J00000000000000000000003", Tier::Coordinate, "one", None);
        let (context, line) = signals(std::slice::from_ref(&one)).unwrap();
        assert_eq!(line, "Converge: 1 new signal");
        assert_eq!(
            context,
            "Converge: 1 signal raised since your last prompt — an expert model's \
             observations about decisions in this project, some possibly from other \
             people's sessions. They are information for the user, who decides whether \
             each holds.\n\
             - [coordinate/dependency] one (01J00000000000000000000003): one bears on the \
             other one.\n\
             The user will want to hear about these; `decision_get` and `signal_list` \
             hold the full record, and `signal_resolve` records the user's verdict once \
             they give it."
        );

        let two = arrived(
            "01J00000000000000000000004",
            Tier::Conflict,
            "two",
            Some(" talk to billing "),
        );
        let (context, line) = signals(&[one, two]).unwrap();
        assert_eq!(line, "Converge: 2 new signals (1 conflict)");
        assert!(
            context.starts_with("Converge: 2 signals raised"),
            "{context}"
        );
        assert!(
            context.contains("worth raising before building on either"),
            "{context}"
        );
        assert!(
            context.contains("\n  Recommendation: talk to billing\n"),
            "{context}"
        );
    }
}
