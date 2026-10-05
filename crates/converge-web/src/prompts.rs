//! `#/project/{id}/prompts` — what a model working in this project is
//! handed, rendered by the server for the person looking: the
//! session-start block, the per-prompt signal frame, the signal expert's
//! input for a chosen decision, and what every MCP client is told.
//! Looking changes nothing: no signal is claimed, nothing is marked read.

use leptos::prelude::*;

use crate::data;

#[component]
pub fn ProjectPrompts(pid: String) -> impl IntoView {
    let title = data::proj_name(&pid);
    view! {
        <div class="cv-page">
            <h1 class="cv-heading cv-fs-4xl cv-mb-6">"What agents see"</h1>
            <p class="cv-fg-muted cv-fs-md cv-mb-22">
                "Rendered for you, exactly as a session in "
                <span class="cv-mono">{title}</span>
                " receives it. Looking changes nothing: no signal is claimed and nothing is marked read."
            </p>
            {body(pid)}
        </div>
    }
}

/// One rendered text, as the model gets it.
#[cfg(feature = "api")]
fn block(text: String) -> impl IntoView {
    view! { <pre class="cv-md-pre cv-prompt"><code>{text}</code></pre> }
}

/// A section: what it is, when it is sent, and its content.
#[cfg(feature = "api")]
fn section(label: &'static str, hint: &'static str, content: AnyView) -> impl IntoView {
    view! {
        <section class="cv-col cv-gap-6 cv-mb-28">
            <h2 class="cv-heading cv-fs-xl">{label}</h2>
            <span class="cv-setform__hint">{hint}</span>
            {content}
        </section>
    }
}

/// The line the person sees beside what the model gets.
#[cfg(feature = "api")]
fn line(text: String) -> impl IntoView {
    view! {
        <div class="cv-fs-sm cv-fg-muted">
            "Shown to the person: " <span class="cv-mono">{text}</span>
        </div>
    }
}

#[cfg(not(feature = "api"))]
fn body(_pid: String) -> impl IntoView {
    view! {
        <p class="cv-fg-muted">
            "This demo build has no server behind it; the preview renders on a live one."
        </p>
    }
}

#[cfg(feature = "api")]
fn body(pid: String) -> impl IntoView {
    use converge_client::{DecisionId, ExpertPrompt, ProjectId, Prompts, StoreError};
    use converge_ui::atoms::Select;

    let prompts = RwSignal::new(None::<Result<Prompts, String>>);
    let expert = RwSignal::new(None::<Result<ExpertPrompt, String>>);
    match pid.parse::<ProjectId>() {
        Ok(id) => leptos::task::spawn_local(async move {
            let got = match crate::store::client().project_prompts(id).await {
                Ok(Some(p)) => Ok(p),
                Ok(None) => Err("This project isn't visible to you.".to_string()),
                Err(e) => Err(crate::feedback::message("Couldn't load the preview", &e)),
            };
            prompts.try_set(Some(got));
        }),
        Err(_) => prompts.set(Some(Err("Not a project id.".into()))),
    }

    let load_expert = move |decision: String| {
        let Ok(id) = decision.parse::<DecisionId>() else {
            return;
        };
        expert.set(None);
        leptos::task::spawn_local(async move {
            let got = match crate::store::client().decision_prompt(id).await {
                Ok(Some(p)) => Ok(p),
                Ok(None) => Err(crate::feedback::message(
                    "Couldn't load the expert's input",
                    &StoreError::NotFound,
                )),
                Err(e) => Err(crate::feedback::message(
                    "Couldn't load the expert's input",
                    &e,
                )),
            };
            expert.try_set(Some(got));
        });
    };
    let decisions: Vec<(String, String)> = data::project_decisions(&pid)
        .iter()
        .map(|d| (d.id.clone(), d.title.clone()))
        .collect();
    let first = decisions.first().map(|(id, _)| id.clone());
    if let Some(first) = first.clone() {
        load_expert(first);
    }

    let pretty = |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_default();

    let expert_view = match first {
        None => view! {
            <p class="cv-fg-muted">"No decisions yet — the expert runs when one is recorded."</p>
        }
        .into_any(),
        Some(first) => view! {
            <Select
                options=decisions
                value=first
                on_change=Callback::new(load_expert)
            />
            {move || match expert.get() {
                None => view! { <p class="cv-fg-muted">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="cv-flash">{e}</div> }.into_any(),
                Some(Ok(p)) => view! {
                    <span class="cv-modal__label">"System prompt"</span>
                    {block(p.system)}
                    <span class="cv-modal__label">"Input"</span>
                    {match p.user {
                        Some(user) => block(pretty(&user)).into_any(),
                        None => view! {
                            <p class="cv-fg-muted">
                                "Nothing related was retrieved for this decision, so no pass would run."
                            </p>
                        }
                        .into_any(),
                    }}
                }
                .into_any(),
            }}
        }
        .into_any(),
    };

    view! {
        {move || match prompts.get() {
            None => view! { <p class="cv-fg-muted">"Rendering…"</p> }.into_any(),
            Some(Err(e)) => view! { <div class="cv-flash">{e}</div> }.into_any(),
            Some(Ok(p)) => view! {
                {section(
                    "Always",
                    "What every agent is told when it connects, hooks or not: what Converge is and how to work with it.",
                    block(p.mcp.instructions).into_any(),
                )}
                {section(
                    "Session start",
                    "Injected once when a session opens in a working tree bound to this project.",
                    view! { {line(p.session.line)} {block(p.session.context)} }.into_any(),
                )}
                {section(
                    "Each prompt",
                    "Added to a prompt when signals arrived since the last one; at most three, oldest first.",
                    match p.signals {
                        Some(frame) => view! { {line(frame.line)} {block(frame.context)} }.into_any(),
                        None => view! {
                            <p class="cv-fg-muted">"No open signals — nothing would be added right now."</p>
                        }
                        .into_any(),
                    },
                )}
            }
            .into_any(),
        }}
        {section(
            "Signal expert",
            "What the expert is handed when a decision is recorded, for the decision you pick. The model is not called.",
            expert_view,
        )}
        // Last: the longest, and the same for every project.
        {move || prompts.get().and_then(Result::ok).map(|p| section(
            "Tools",
            "Each tool's description and input, as the model reads them.",
            view! {
                {p.mcp.tools.into_iter().map(|t| view! {
                    <details class="cv-col cv-gap-6">
                        <summary class="cv-mono cv-pointer">{t.name}</summary>
                        {block(t.description.unwrap_or_default())}
                        {block(pretty(&t.input_schema))}
                    </details>
                }).collect_view()}
            }
            .into_any(),
        ))}
    }
}
