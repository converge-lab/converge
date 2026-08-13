//! The Expert screen — grounded chat with the server-side expert over
//! the group's decision memory. On the `api` build the conversation is
//! real: `POST /api/v1/expert/ask` streams the answer (SSE), grounded
//! per the context policy (rich index + selected records), and the
//! transcript lives client-side — the server keeps no conversations.
//! The embedded build keeps a canned exchange, enough to show the shape.

use converge_ui::atoms::Glyph;
use converge_ui::domain::ChatRole;
use converge_ui::molecules::{ChatBubble, ChatComposer, ChatListItem};
use leptos::prelude::*;

use crate::store::AppStateStoreFields;

const SUGGESTIONS: [&str; 3] = [
    "What must an agent know before touching authentication?",
    "Which decisions are still proposed, and what's blocking them?",
    "Summarize what this group has settled about deployment.",
];

/// One transcript turn.
#[derive(Clone, PartialEq)]
pub struct Msg {
    pub user: bool,
    pub text: String,
}

/// Expert-chat state, held in context above the router: the active screen is
/// re-created on every route change *and* on every dataset write (a sidebar
/// "New project" mid-chat), and an in-progress chat must survive both. It is
/// scoped to a group and reset lazily when the active group changes.
#[derive(Clone, Copy)]
pub struct ExpertState {
    thread: RwSignal<Vec<Msg>>,
    busy: RwSignal<bool>,
    /// What the last answer grounded in: (decisions, signals).
    grounded: RwSignal<Option<(u64, u64)>>,
    /// The group index the state belongs to; `None` until the first visit.
    for_group: RwSignal<Option<usize>>,
}

/// Provide the chat state at the app root (once, above every screen).
pub fn provide_expert_state() {
    provide_context(ExpertState {
        thread: RwSignal::new(Vec::new()),
        busy: RwSignal::new(false),
        grounded: RwSignal::new(None),
        for_group: RwSignal::new(None),
    });
}

/// Ask over the live API: append the question, stream the answer into a
/// growing expert turn.
#[cfg(feature = "api")]
fn send(state: ExpertState, question: String) {
    use converge_client::AskEvent;
    use futures::StreamExt;

    if state.busy.get_untracked() || question.trim().is_empty() {
        return;
    }
    let Ok(group) = crate::data::cur_group()
        .id
        .parse::<converge_client::GroupId>()
    else {
        return;
    };
    let history: Vec<(bool, String)> = state
        .thread
        .get_untracked()
        .into_iter()
        .map(|m| (m.user, m.text))
        .collect();
    state.thread.update(|t| {
        t.push(Msg {
            user: true,
            text: question.clone(),
        });
        t.push(Msg {
            user: false,
            text: String::new(),
        });
    });
    state.busy.set(true);
    leptos::task::spawn_local(async move {
        let outcome = async {
            // Pinned: the SSE reader is an unfold (not Unpin).
            let mut stream = Box::pin(
                crate::store::client()
                    .expert_ask(group, &question, &history)
                    .await?,
            );
            while let Some(event) = stream.next().await {
                match event? {
                    AskEvent::Context { decisions, signals } => {
                        state.grounded.set(Some((decisions, signals)));
                    }
                    AskEvent::Delta(text) => state.thread.update(|t| {
                        if let Some(last) = t.last_mut() {
                            last.text.push_str(&text);
                        }
                    }),
                    AskEvent::Done => break,
                }
            }
            Ok::<_, converge_client::StoreError>(())
        }
        .await;
        if let Err(e) = outcome {
            state.thread.update(|t| {
                if let Some(last) = t.last_mut() {
                    if last.text.is_empty() {
                        last.text = format!("The expert couldn't answer — {e}");
                    } else {
                        last.text
                            .push_str(&format!("\n\n(stream interrupted — {e})"));
                    }
                }
            });
        }
        state.busy.set(false);
    });
}

/// The embedded demo: a canned reply, immediately.
#[cfg(not(feature = "api"))]
fn send(state: ExpertState, question: String) {
    if question.trim().is_empty() {
        return;
    }
    state.thread.update(|t| {
        t.push(Msg {
            user: true,
            text: question,
        });
        t.push(Msg {
            user: false,
            text: "This demo build has no model behind it — the hosted expert \
                   answers from the group's decision memory, streaming, with \
                   the decisions it leaned on cited inline."
                .into(),
        });
    });
}

#[component]
pub fn Expert() -> impl IntoView {
    let state = expect_context::<ExpertState>();
    // (Re)scope the chat to the active group: the first visit — or a group
    // switch — resets the thread.
    let group = crate::store::use_store().group().get_untracked();
    if state.for_group.get_untracked() != Some(group) {
        state.for_group.set(Some(group));
        state.thread.set(Vec::new());
        state.grounded.set(None);
    }
    let thread = state.thread;
    let grounded = state.grounded;

    view! {
        <div class="cv-expert">
            <div class="cv-expert__chats">
                <div
                    class="cv-expert__newchat"
                    on:click=move |_| {
                        thread.set(Vec::new());
                        grounded.set(None);
                    }
                >
                    <span class="cv-fg-expert">"＋"</span>
                    " New chat"
                </div>
                <div class="cv-expert__chatslabel">"Chats"</div>
                <ChatListItem title="New chat" active=true on_click=Callback::new(move |_| {}) />
            </div>

            <div class="cv-expert__area">
                <div class="cv-row cv-gap-9 cv-mb-12">
                    <span class="cv-fg-expert cv-fs-2xl">{Glyph::Expert.glyph()}</span>
                    <h1 class="cv-heading cv-fs-2xl">"Expert model"</h1>
                    <span class="cv-spacer"></span>
                    {move || {
                        grounded
                            .get()
                            .map(|(d, s)| {
                                view! {
                                    <span class="cv-fs-xs cv-fg-faint">
                                        {format!(
                                            "grounded in {d} decision{} · {s} open signal{}",
                                            if d == 1 { "" } else { "s" },
                                            if s == 1 { "" } else { "s" },
                                        )}
                                    </span>
                                }
                            })
                    }}
                </div>

                {move || {
                    if thread.get().is_empty() {
                        empty_state(state).into_any()
                    } else {
                        thread_view(state).into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// Empty state — hero, composer, and three suggestion chips.
fn empty_state(state: ExpertState) -> impl IntoView {
    view! {
        <div class="cv-expert__empty">
            <div class="cv-text-center cv-expert__lead">
                <div class="cv-fs-5xl cv-fg-expert cv-mb-8">{Glyph::Expert.glyph()}</div>
                <h2 class="cv-heading cv-fs-4xl cv-mb-9">"Ask the expert"</h2>
                <p class="cv-fs-lg cv-fg-muted cv-lh-relaxed">
                    "It holds all "
                    <span class="cv-fg-secondary">{crate::data::group_decisions().len()}</span>
                    " decisions for "
                    <span class="cv-mono cv-fg-secondary">{crate::data::group_name()}</span>
                    " and answers from them — citing what it leaned on."
                </p>
            </div>
            <div class="cv-w-full cv-measure">
                <ChatComposer
                    placeholder="Ask the expert…"
                    on_send=Callback::new(move |q: String| send(state, q))
                />
            </div>
            <div class="cv-w-full cv-measure cv-col cv-gap-7">
                {SUGGESTIONS
                    .iter()
                    .map(|s| {
                        let q = *s;
                        view! {
                            <div class="cv-suggest" on:click=move |_| send(state, q.to_string())>
                                {q}
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}

/// The live conversation plus the docked composer.
fn thread_view(state: ExpertState) -> impl IntoView {
    let thread = state.thread;
    let busy = state.busy;
    view! {
        <div class="cv-expert__thread">
            {move || {
                thread
                    .get()
                    .into_iter()
                    .map(|m| {
                        let role = if m.user { ChatRole::User } else { ChatRole::Expert };
                        let text = if m.text.is_empty() { "…".to_string() } else { m.text };
                        view! { <ChatBubble role=role text=text /> }
                    })
                    .collect_view()
            }}
        </div>

        <div class="cv-pt-16">
            {move || {
                let waiting = busy.get();
                view! {
                    <ChatComposer
                        placeholder=if waiting { "Answering…" } else { "Ask the expert…" }
                        on_send=Callback::new(move |q: String| send(state, q))
                    />
                }
            }}
        </div>
    }
}
