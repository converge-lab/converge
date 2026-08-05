//! `#/pair` — the browser half of the device grant (RFC 8628): a
//! signed-in user approves (or denies) a CLI asking to act as them.
//!
//! The code usually rides the URL (`#/pair/XXXX-XXXX` from the link the
//! CLI prints), so the common path is one glance and one click; the bare
//! `#/pair` form is the type-it-in fallback from `verification_uri`. The
//! decision is terminal — the screen ends in a "return to the terminal"
//! state, never back at the form.

use converge_ui::atoms::{Button, Glyph, Input, SectionLabel};
use converge_ui::domain::Tone;
use leptos::prelude::*;

/// What the screen knows about the pending grant.
#[derive(Clone, PartialEq)]
enum Stage {
    /// Waiting for a code (bare `#/pair`, or a lookup that found nothing).
    Ask,
    /// Grant found — show who is asking, offer the verdict.
    Found { client_name: String },
    /// Approved: the CLI's next poll signs it in.
    Approved,
    /// Denied: the CLI's next poll is told no.
    Denied,
}

#[component]
pub fn Pair(code: Option<String>) -> impl IntoView {
    let (input, set_input) = signal(code.clone().unwrap_or_default());
    let (stage, set_stage) = signal(Stage::Ask);
    let (notice, set_notice) = signal(None::<String>);

    #[cfg(feature = "api")]
    let lookup = move || {
        let code = input.get_untracked().trim().to_string();
        if code.is_empty() {
            return;
        }
        leptos::task::spawn_local(async move {
            match crate::store::client().device_get(&code).await {
                Ok(Some(grant)) => {
                    set_notice.set(None);
                    set_stage.set(Stage::Found {
                        client_name: grant.client_name,
                    });
                }
                Ok(None) => {
                    set_notice.set(Some(
                        "No pending request for that code — it may have expired. \
                         Re-run the command and try again."
                            .into(),
                    ));
                }
                Err(e) => set_notice.set(Some(format!("look up code: {e}"))),
            }
        });
    };
    // The embedded (no-API) build walks the stages with fixture data,
    // like every other demo screen — nothing real is paired.
    #[cfg(not(feature = "api"))]
    let lookup = move || {
        let _ = input.get_untracked();
        set_notice.set(None);
        set_stage.set(Stage::Found {
            client_name: "converge-cli @ demo".into(),
        });
    };

    #[cfg(feature = "api")]
    let decide = move |approve: bool| {
        let code = input.get_untracked().trim().to_string();
        leptos::task::spawn_local(async move {
            match crate::store::client().device_decide(&code, approve).await {
                Ok(()) => set_stage.set(match approve {
                    true => Stage::Approved,
                    false => Stage::Denied,
                }),
                Err(e) => set_notice.set(Some(format!("submit decision: {e}"))),
            }
        });
    };
    #[cfg(not(feature = "api"))]
    let decide = move |approve: bool| {
        set_stage.set(match approve {
            true => Stage::Approved,
            false => Stage::Denied,
        });
    };

    // A code in the URL looks itself up — the printed link is one click.
    if code.is_some() {
        lookup();
    }

    view! {
        <div class="cv-page">
            <div class="cv-settings">
                <h1 class="cv-heading cv-fs-3xl cv-mb-22">"Pair a device"</h1>

                <div class="cv-mb-12">
                    <SectionLabel text="device pairing" />
                </div>

                {move || {
                    match stage.get() {
                        Stage::Ask => {
                            view! {
                                <p class="cv-settings__desc">
                                    "A CLI or agent waiting on " <span class="cv-mono">"converge init"</span>
                                    " shows a pairing code. Enter it here to connect that device "
                                    "to your account."
                                </p>
                                <div class="cv-tokenform">
                                    <Input
                                        placeholder="XXXX-XXXX"
                                        value=input
                                        on_input=Callback::new(move |v: String| set_input.set(v))
                                        on_keydown=Callback::new(move |ev: leptos::ev::KeyboardEvent| {
                                            if ev.key() == "Enter" {
                                                lookup();
                                            }
                                        })
                                    />
                                    <Button
                                        label="Look up"
                                        tone=Tone::Primary
                                        disabled=Signal::derive(move || {
                                            input.get().trim().is_empty()
                                        })
                                        on_click=Callback::new(move |()| lookup())
                                    />
                                </div>
                            }
                                .into_any()
                        }
                        Stage::Found { client_name } => {
                            view! {
                                <p class="cv-settings__desc">
                                    <b>{client_name}</b>
                                    " is asking to connect to your Converge account. Approving "
                                    "signs that device in " <b>"as you"</b>
                                    ": it reads and records decisions under your name. Its "
                                    "credential appears with your API tokens and can be revoked "
                                    "there at any time."
                                </p>
                                <div class="cv-pair__code cv-mono cv-fs-xl cv-mb-16">
                                    {input.get_untracked()}
                                </div>
                                <div class="cv-row cv-gap-10">
                                    <Button
                                        label="Deny"
                                        variant=converge_ui::atoms::ButtonVariant::Ghost
                                        on_click=Callback::new(move |()| decide(false))
                                    />
                                    <Button
                                        label="Approve"
                                        tone=Tone::Primary
                                        on_click=Callback::new(move |()| decide(true))
                                    />
                                </div>
                            }
                                .into_any()
                        }
                        Stage::Approved => {
                            view! {
                                <div class="cv-tokennew">
                                    <div class="cv-tokennew__head">
                                        {Glyph::Verified.glyph()}
                                        " Device connected"
                                    </div>
                                    <div class="cv-tokennew__note">
                                        "You can return to the terminal — it signs in on its "
                                        "next poll (a few seconds)."
                                    </div>
                                </div>
                            }
                                .into_any()
                        }
                        Stage::Denied => {
                            view! {
                                <p class="cv-settings__desc">
                                    "Request denied. The waiting command will report the "
                                    "refusal and exit — nothing was connected."
                                </p>
                            }
                                .into_any()
                        }
                    }
                }}
                {move || {
                    notice
                        .get()
                        .map(|msg| {
                            view! { <div class="cv-fs-sm cv-fg-danger cv-mb-16">{msg}</div> }
                        })
                }}
            </div>
        </div>
    }
}
