//! `#/settings` — the caller's API tokens: mint, list, revoke.
//!
//! Tokens are how the caller's *agents* (Claude Code over MCP, the CLI,
//! scripts) authenticate as them. The secret is shown exactly once, right
//! after minting (the one-time reveal below the form); the list shows an
//! id-derived hint, never a secret. Revocation confirms inline in the row —
//! no modal. All calls go through the live API; in the embedded build the
//! actions only explain themselves away.

use converge_ui::atoms::{Button, Glyph, Input, SectionLabel};
use converge_ui::domain::Tone;
use leptos::prelude::*;

use crate::command_snippet::CopyButton;

/// A token row, decoupled from the client types so the module compiles in
/// the embedded (no-API) build too.
#[derive(Clone, PartialEq)]
struct Row {
    id: String,
    label: String,
    created: String,
}

/// `cvg_<id>` shortened for display — first 10 chars + "…" + last 5
/// (`cvg_01KX1P…36P3K`). An identifier-shaped hint for telling tokens
/// apart; never the secret (storage only ever holds its hash).
fn token_display(id: &str) -> String {
    let full = format!("cvg_{id}");
    let chars: Vec<char> = full.chars().collect();
    if chars.len() <= 16 {
        return full;
    }
    let head: String = chars[..10].iter().collect();
    let tail: String = chars[chars.len() - 5..].iter().collect();
    format!("{head}…{tail}")
}

#[component]
pub fn Settings() -> impl IntoView {
    let (rows, set_rows) = signal(Vec::<Row>::new());
    let (label, set_label) = signal(String::new());
    // The shown-once secret of the most recent mint.
    let (minted, set_minted) = signal(None::<String>);
    let (notice, set_notice) = signal(None::<String>);
    // Which row is asking "really revoke?" — inline, one at a time.
    let (confirming, set_confirming) = signal(None::<String>);

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
        set_confirming.set(None);
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
    let revoke = move |_id: String| {
        set_confirming.set(None);
        refresh();
    };

    view! {
        <div class="cv-page">
            <div class="cv-settings">
                <h1 class="cv-heading cv-fs-3xl cv-mb-22">"Settings"</h1>

                <div class="cv-mb-12">
                    <SectionLabel text="api tokens" />
                </div>
                <p class="cv-settings__desc">
                    "Agents authenticate with bearer tokens — Claude Code over MCP, "
                    "the CLI, scripts. A token acts as you: it reads and records "
                    "decisions " <b>"under your name"</b>
                    ". Revoking one kills that credential immediately."
                </p>

                <div class="cv-tokenform">
                    <Input
                        placeholder="What's this token for? (laptop, ci, …)"
                        value=label
                        on_input=Callback::new(move |v: String| set_label.set(v))
                        on_keydown=Callback::new(move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                mint();
                            }
                        })
                    />
                    <Button
                        label="Create token"
                        tone=Tone::Primary
                        disabled=Signal::derive(move || label.get().trim().is_empty())
                        on_click=Callback::new(move |()| mint())
                    />
                </div>

                // The one-time reveal: the only place the full secret ever
                // appears. It lives in component state, so any navigation
                // away drops it; the list below never carries secrets.
                {move || {
                    minted
                        .get()
                        .map(|secret| {
                            view! {
                                <div class="cv-tokennew">
                                    <div class="cv-tokennew__head">
                                        {Glyph::Verified.glyph()}
                                        " Token created"
                                    </div>
                                    <div class="cv-tokennew__snippet">
                                        <span class="cv-tokennew__cmd">{secret.clone()}</span>
                                        <CopyButton text=secret />
                                    </div>
                                    <div class="cv-tokennew__note">
                                        "Copy it now — it won't be shown again. Store it as "
                                        <span class="cv-mono">"CONVERGE_TOKEN"</span> "."
                                    </div>
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

                <div class="cv-mb-10">
                    <SectionLabel text="active tokens" />
                </div>
                <div class="cv-tokens">
                    {move || {
                        let items = rows.get();
                        if items.is_empty() {
                            return view! {
                                <div class="cv-tokens__empty">
                                    "No active tokens. Create one above to connect an agent."
                                </div>
                            }
                                .into_any();
                        }
                        items
                            .into_iter()
                            .map(|row| {
                                let ask_id = row.id.clone();
                                let do_id = row.id.clone();
                                let row_id = row.id.clone();
                                view! {
                                    <div class="cv-tokens__row">
                                        <div class="cv-tokens__meta">
                                            <div class="cv-tokens__name">{row.label}</div>
                                            <div class="cv-tokens__id">{token_display(&row.id)}</div>
                                        </div>
                                        <div class="cv-tokens__when">
                                            "created " {row.created}
                                        </div>
                                        {move || {
                                            let confirming_this = confirming.get().as_deref()
                                                == Some(row_id.as_str());
                                            if confirming_this {
                                                let do_id = do_id.clone();
                                                view! {
                                                    <div class="cv-tokens__confirm">
                                                        <span class="cv-tokens__warn">
                                                            "Stops working immediately."
                                                        </span>
                                                        <Button
                                                            label="Cancel"
                                                            variant=converge_ui::atoms::ButtonVariant::Ghost
                                                            on_click=Callback::new(move |()| {
                                                                set_confirming.set(None)
                                                            })
                                                        />
                                                        <Button
                                                            label="Revoke"
                                                            tone=Tone::Danger
                                                            on_click=Callback::new(move |()| {
                                                                revoke(do_id.clone())
                                                            })
                                                        />
                                                    </div>
                                                }
                                                    .into_any()
                                            } else {
                                                let ask_id = ask_id.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        class="cv-tokens__revoke"
                                                        on:click=move |_| {
                                                            set_confirming.set(Some(ask_id.clone()))
                                                        }
                                                    >
                                                        "Revoke"
                                                    </button>
                                                }
                                                    .into_any()
                                            }
                                        }}
                                    </div>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::token_display;

    /// The list hint: `cvg_` + first six of the id, an ellipsis, the last
    /// five — and short ids pass through untouched.
    #[test]
    fn token_display_shortens_ids() {
        assert_eq!(
            token_display("01KX1P5DRS53QJGWMQKNJ36P3K"),
            "cvg_01KX1P…36P3K"
        );
        assert_eq!(token_display("SHORT"), "cvg_SHORT");
    }
}
