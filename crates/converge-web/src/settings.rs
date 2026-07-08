//! `#/settings` — the caller's API tokens: mint, list, revoke.
//!
//! Tokens are how the caller's *agents* (Claude Code over MCP, the CLI,
//! scripts) authenticate as them. The secret is shown exactly once, right
//! after minting; the list never carries secrets. All calls go through the
//! live API — this screen has no seed residue, and in the embedded build
//! its actions only explain themselves away.

use converge_ui::atoms::{Button, Callout, Glyph, Input};
use converge_ui::domain::Tone;
use leptos::prelude::*;

/// A token row, decoupled from the client types so the module compiles in
/// the embedded (no-API) build too.
#[derive(Clone, PartialEq)]
struct Row {
    id: String,
    label: String,
    created: String,
}

#[component]
pub fn Settings() -> impl IntoView {
    let (rows, set_rows) = signal(Vec::<Row>::new());
    let (label, set_label) = signal(String::new());
    // The shown-once secret of the most recent mint.
    let (minted, set_minted) = signal(None::<String>);
    let (notice, set_notice) = signal(None::<String>);

    #[cfg(feature = "api")]
    let refresh = move || {
        leptos::task::spawn_local(async move {
            use converge_client::Pagination;
            use time::format_description::well_known::Rfc3339;
            match crate::store::client()
                .token_list(&Pagination::default())
                .await
            {
                Ok(page) => set_rows.set(
                    page.items
                        .into_iter()
                        .map(|t| Row {
                            id: t.id.to_string(),
                            label: t.label,
                            created: t
                                .created_at
                                .format(&Rfc3339)
                                .map(|iso| crate::when::when(&iso))
                                .unwrap_or_default(),
                        })
                        .collect(),
                ),
                Err(e) => set_notice.set(Some(format!("load tokens: {e}"))),
            }
        });
    };
    #[cfg(not(feature = "api"))]
    let refresh = move || {
        set_notice.set(Some("This build has no API to manage tokens on.".into()));
        let _ = set_rows;
    };
    refresh();

    #[cfg(feature = "api")]
    let mint = move || {
        let name = label.get_untracked().trim().to_string();
        if name.is_empty() {
            return;
        }
        leptos::task::spawn_local(async move {
            use converge_client::NewToken;
            match crate::store::client()
                .token_add(&NewToken { label: name })
                .await
            {
                Ok(minted) => {
                    set_minted.set(Some(minted.token));
                    set_label.set(String::new());
                    set_notice.set(None);
                    refresh();
                }
                Err(e) => set_notice.set(Some(format!("create token: {e}"))),
            }
        });
    };
    #[cfg(not(feature = "api"))]
    let mint = move || {
        let _ = label.get_untracked();
        set_minted.set(None);
        refresh();
    };

    #[cfg(feature = "api")]
    let revoke = move |id: String| {
        leptos::task::spawn_local(async move {
            let Ok(id) = id.parse::<converge_client::TokenId>() else {
                return;
            };
            match crate::store::client().token_revoke(id).await {
                Ok(()) => refresh(),
                Err(e) => set_notice.set(Some(format!("revoke token: {e}"))),
            }
        });
    };
    #[cfg(not(feature = "api"))]
    let revoke = move |_id: String| refresh();

    view! {
        <div class="cv-page">
            <div class="cv-fs-xl cv-fw-semibold cv-mb-8">"API tokens"</div>
            <div class="cv-fs-md cv-fg-muted cv-lh-normal cv-mb-20 cv-measure">
                "Your agents — Claude Code over MCP, the CLI, scripts — authenticate "
                "with bearer tokens. A token acts as you: it can read and record "
                "decisions under your name. Revoking one kills that credential "
                "immediately."
            </div>

            <div class="cv-row cv-gap-10 cv-mb-16" style="max-width: 480px;">
                <Input
                    placeholder="What's this token for? (laptop, ci, …)"
                    value=label.get_untracked()
                    on_input=Callback::new(move |v: String| set_label.set(v))
                />
                <Button label="Create token" on_click=Callback::new(move |()| mint()) />
            </div>

            {move || {
                minted
                    .get()
                    .map(|secret| {
                        view! {
                            <div class="cv-mb-16 cv-measure">
                                <Callout tone=Tone::Primary icon=Glyph::Verified title="Token created — save it now">
                                    <div class="cv-mono cv-fs-sm" style="word-break: break-all; user-select: all;">
                                        {secret}
                                    </div>
                                    <div class="cv-fs-xs cv-fg-muted cv-mt-8">
                                        "This is the only time the secret is shown."
                                    </div>
                                </Callout>
                            </div>
                        }
                    })
            }}
            {move || {
                notice
                    .get()
                    .map(|msg| {
                        view! { <div class="cv-fs-sm cv-fg-danger cv-mb-16">{msg}</div> }
                    })
            }}

            <div class="cv-filterlabel cv-mb-10 cv-ls-wider">"Active tokens"</div>
            <div class="cv-col cv-gap-8" style="max-width: 640px;">
                {move || {
                    let items = rows.get();
                    if items.is_empty() {
                        return view! {
                            <div class="cv-fs-sm cv-fg-faint">"No tokens yet."</div>
                        }
                            .into_any();
                    }
                    items
                        .into_iter()
                        .map(|row| {
                            let id = row.id.clone();
                            view! {
                                <div class="cv-card cv-row cv-gap-12" style="padding: 10px 14px; align-items: center;">
                                    <div class="cv-col cv-gap-2" style="min-width: 0; flex: 1;">
                                        <div class="cv-fs-md cv-fw-medium">{row.label}</div>
                                        <div class="cv-fs-xs cv-fg-faint cv-mono">{row.id}</div>
                                    </div>
                                    <div class="cv-fs-xs cv-fg-muted">{row.created}</div>
                                    <Button
                                        label="Revoke"
                                        variant=converge_ui::atoms::ButtonVariant::Ghost
                                        tone=Tone::Danger
                                        on_click=Callback::new(move |()| revoke(id.clone()))
                                    />
                                </div>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </div>
    }
}
