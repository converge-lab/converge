//! The Expert screen — the chat with the server-side "Converge Expert" that
//! holds the group's whole decision memory and forwards the relevant slice into
//! a local agent's context. The live LLM is deferred; this opens on an empty
//! state and reveals a mock conversation once you ask something.

use converge_ui::atoms::Glyph;
use converge_ui::domain::ChatRole;
use converge_ui::molecules::{ChatBubble, ChatComposer, ChatListItem};
use leptos::prelude::*;

const SUGGESTIONS: [&str; 3] = [
    "What must the local agent know before changing the api-gateway error format?",
    "Is it safe to change the OIDC token TTL?",
    "Why is infrastructure deployed separately from services?",
];

// The prototype opens with a single fresh chat; more accumulate as you talk.
const CHATS: [&str; 1] = ["New chat"];

#[component]
pub fn Expert() -> impl IntoView {
    let (started, set_started) = signal(false);
    // Which mock chat is selected, and which group projects are in scope.
    let (active_chat, set_active_chat) = signal(0usize);
    // Default the in-scope project to api-gateway when the active group has it,
    // otherwise the group's first project (so personal groups select correctly).
    let (selected, set_selected) = signal::<Vec<String>>({
        let projs = crate::data::cur_group_projects();
        let first = if projs.iter().any(|p| p.as_str() == "api-gateway") {
            "api-gateway"
        } else {
            projs.first().map(|s| s.as_str()).unwrap_or("")
        };
        if first.is_empty() {
            vec![]
        } else {
            vec![first.to_string()]
        }
    });

    view! {
        <div class="cv-expert">
            <div class="cv-expert__chats">
                <div class="cv-expert__newchat" on:click=move |_| set_started.set(false)>
                    <span class="cv-fg-expert">"＋"</span>
                    " New chat"
                </div>
                <div class="cv-expert__chatslabel">"Chats"</div>
                {move || {
                    let cur = active_chat.get();
                    CHATS
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            view! {
                                <ChatListItem
                                    title=*t
                                    active=i == cur
                                    on_click=Callback::new(move |_| set_active_chat.set(i))
                                />
                            }
                        })
                        .collect_view()
                }}
            </div>

            <div class="cv-expert__area">
                <div class="cv-row cv-gap-9 cv-mb-12">
                    <span class="cv-fg-expert cv-fs-2xl">{Glyph::Expert.glyph()}</span>
                    <h1 class="cv-heading cv-fs-2xl">
                        "Expert model"
                    </h1>
                </div>
                <div class="cv-expert__projs">
                    {move || {
                        let sel = selected.get();
                        crate::data::cur_group_projects()
                            .iter()
                            .map(|p| {
                                let pid = p.to_string();
                                let on = sel.iter().any(|x| x == &pid);
                                let cls = if on {
                                    "cv-projchip cv-projchip--on"
                                } else {
                                    "cv-projchip"
                                };
                                view! {
                                    <span
                                        class=cls
                                        on:click=move |_| {
                                            let pid = pid.clone();
                                            set_selected
                                                .update(|s| {
                                                    if let Some(idx) = s.iter().position(|x| x == &pid) {
                                                        // keep at least one project selected
                                                        if s.len() > 1 { s.remove(idx); }
                                                    } else {
                                                        s.push(pid);
                                                    }
                                                });
                                        }
                                    >
                                        {p.clone()}
                                    </span>
                                }
                            })
                            .collect_view()
                    }}
                </div>

                {move || {
                    if started.get() {
                        started_thread().into_any()
                    } else {
                        empty_state(set_started).into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// Empty state — hero, composer, and three suggestion chips. Any of them starts
/// the (mock) conversation.
fn empty_state(set_started: WriteSignal<bool>) -> impl IntoView {
    view! {
        <div class="cv-expert__empty">
            <div class="cv-text-center cv-expert__lead">
                <div class="cv-fs-5xl cv-fg-expert cv-mb-8">
                    {Glyph::Expert.glyph()}
                </div>
                <h2 class="cv-heading cv-fs-4xl cv-mb-9">
                    "Ask the expert"
                </h2>
                <p class="cv-fs-lg cv-fg-muted cv-lh-relaxed">
                    "It holds all "
                    <span class="cv-fg-secondary">
                        {crate::data::group_decisions().len()}
                    </span>
                    " decisions for "
                    <span class="cv-mono cv-fg-secondary">
                        {crate::data::group_name()}
                    </span>
                    " and decides which to forward into your local agent's context."
                </p>
            </div>
            <div class="cv-w-full cv-measure">
                <ChatComposer
                    placeholder="Ask the expert…"
                    on_send=Callback::new(move |_: String| set_started.set(true))
                />
            </div>
            <div class="cv-w-full cv-measure cv-col cv-gap-7">
                {SUGGESTIONS
                    .iter()
                    .map(|s| {
                        view! {
                            <div class="cv-suggest" on:click=move |_| set_started.set(true)>
                                {*s}
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}

/// The started conversation — a mock exchange plus the docked composer.
fn started_thread() -> impl IntoView {
    view! {
        <div class="cv-expert__thread">
            <ChatBubble
                role=ChatRole::User
                text="we're about to rename the error status field in api-gateway. anything that'll break?"
            />
            <ChatBubble
                role=ChatRole::Expert
                text="Two things. web-app still parses the error text to choose toast copy, so changing the field breaks it until it moves to the enum — that's the open \"will break\" signal. And the error code stays pinned by a contract with the legacy billing stack, so don't touch that. Ship behind a flag and let web-app cut over first."
                forwarded=vec![
                    ("api-gateway".to_string(), "Error responses include a structured `status` field".to_string()),
                    ("web-app".to_string(), "Error toasts map status → copy".to_string()),
                ]
            />
        </div>

        <div class="cv-pt-16">
            <ChatComposer placeholder="Ask the expert…" />
        </div>
    }
}
