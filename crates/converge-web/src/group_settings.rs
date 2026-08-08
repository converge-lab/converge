//! `#/group/settings` — everything about the active group in one place: what
//! it's called, who's in it, and how to get rid of it.
//!
//! Reached from the "⋯" beside the group's name on the dashboard. The order is
//! by frequency: name and description on top, members in the middle, the
//! destructive actions fenced off at the bottom.
//!
//! Name and description are real edits (`PATCH /groups/{id}`). Three things on
//! this screen are drawn but not wired, and each says so where it stands:
//! visibility (a group's kind is fixed at creation — turning a personal space
//! into a shared one is a separate operation the server doesn't offer),
//! membership (the API carries it, `converge-client` doesn't yet), and
//! archive/delete (nothing behind them at all). The member rows are a local
//! fixture; see [`mock_members`].

use converge_ui::atoms::{Avatar, Button, ButtonVariant, Glyph, Modal};
use converge_ui::domain::{AuthorKind, GroupKind, Tone, initials};
use leptos::html;
use leptos::prelude::*;

use crate::{data, mutate};

/// A member row — all-owned so the reactive closures stay `Send`.
#[derive(Clone, PartialEq)]
struct Member {
    initial: String,
    color: String,
    name: String,
    handle: String,
    /// The group's single owner (the ACL has no roles beyond this).
    owner: bool,
}

/// A handle from a display name: the first word, lowercased.
fn handle_of(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or(name)
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Stand-in for the membership the server would return: the account as owner,
/// then whoever authored decisions in this group. Deriving the people from the
/// group's own decisions keeps the faces and names consistent with the feed —
/// only the fact that they're *members* is invented.
fn mock_members() -> Vec<Member> {
    let me = data::account();
    let mut rows = vec![Member {
        initial: me.initial.clone(),
        color: me.color.clone(),
        name: me.name.clone(),
        handle: handle_of(&me.name),
        owner: true,
    }];
    for dec in data::group_decisions() {
        for a in &dec.authors {
            if a.kind == AuthorKind::Agent || a.name == me.name {
                continue;
            }
            if rows.iter().any(|r| r.name == a.name) {
                continue;
            }
            rows.push(Member {
                initial: a.initial.clone(),
                color: a.color(),
                name: a.name.clone(),
                handle: handle_of(&a.name),
                owner: false,
            });
        }
    }
    rows
}

#[component]
pub fn GroupSettings() -> impl IntoView {
    let group = data::cur_group();
    let gid = group.id.clone();
    let personal = group.kind == GroupKind::Personal;

    let (name, set_name) = signal(group.name.clone());
    let (desc, set_desc) = signal(group.description.clone().unwrap_or_default());
    let members = RwSignal::new(mock_members());
    let (inviting, set_inviting) = signal(false);
    let (flash, set_flash) = signal(None::<String>);

    let projects = group.project_ids.len();
    let decisions = data::group_decisions().len();

    let save = {
        let gid = gid.clone();
        Callback::new(move |()| {
            let n = name.get_untracked().trim().to_string();
            if n.is_empty() {
                return;
            }
            // No "saved" line: a dataset write re-creates the active screen
            // (see the router's `track_data`), so any local state set here dies
            // on the same tick. The sidebar and breadcrumb updating is the
            // feedback.
            mutate::edit_group(gid.clone(), n, desc.get_untracked());
        })
    };

    let group_name = group.name.clone();
    view! {
        <div class="cv-page cv-page--form">
            <h1 class="cv-heading cv-fs-4xl cv-mb-6">"Group settings"</h1>
            <p class="cv-fg-muted cv-fs-md cv-mb-22">
                "Settings for " <span class="cv-fw-medium">{group_name.clone()}</span>
                {format!(
                    " — {projects} {}, {decisions} {}.",
                    if projects == 1 { "project" } else { "projects" },
                    if decisions == 1 { "decision" } else { "decisions" },
                )}
            </p>

            <div class="cv-setform">
                <div class="cv-col cv-gap-6">
                    <span class="cv-modal__label">"Name"</span>
                    <div class="cv-input">
                        <input
                            class="cv-input__field"
                            prop:value=name
                            on:input=move |ev| set_name.set(event_target_value(&ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    save.run(());
                                }
                            }
                        />
                    </div>
                    <span class="cv-setform__hint">
                        "Renaming keeps the group id and every reference to it intact."
                    </span>
                </div>

                <div class="cv-col cv-gap-6">
                    <span class="cv-modal__label">"Description"</span>
                    <div class="cv-input">
                        <input
                            class="cv-input__field"
                            placeholder="Why this group exists — one line for whoever arrives later."
                            prop:value=desc
                            on:input=move |ev| set_desc.set(event_target_value(&ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    save.run(());
                                }
                            }
                        />
                    </div>
                </div>

                <div class="cv-col cv-gap-6">
                    <span class="cv-modal__label">"Visibility"</span>
                    // Kind is fixed at creation, so these show the answer
                    // rather than ask it — inert, and the line below says why.
                    <div class="cv-row cv-gap-7 cv-setform__locked">
                        <span class=if personal { "cv-projchip" } else { "cv-projchip cv-projchip--on" }>
                            {format!("{} shared", Glyph::Shared.glyph())}
                        </span>
                        <span class=if personal { "cv-projchip cv-projchip--on" } else { "cv-projchip" }>
                            {format!("{} personal", Glyph::Personal.glyph())}
                        </span>
                    </div>
                    <span class="cv-setform__hint">
                        {if personal {
                            "A personal space stays yours. Turning it into a shared group isn't possible yet."
                        } else {
                            "Shared groups are visible to every member. A group's kind is fixed when it's created."
                        }}
                    </span>
                </div>

                <div>
                    <Button
                        label="Save changes"
                        tone=Tone::Primary
                        disabled=Signal::derive(move || name.get().trim().is_empty())
                        on_click=save
                    />
                </div>
            </div>

            <div class="cv-setsep"></div>

            <div class="cv-col cv-gap-8 cv-mb-32">
                <div class="cv-row" style="align-items:flex-start">
                    <div class="cv-grow">
                        <div class="cv-fs-lg cv-fw-medium">"Members"</div>
                        <div class="cv-fs-xs cv-fg-faint cv-mt-4">
                            "Who can read and add to this group's decisions."
                        </div>
                    </div>
                    <Button
                        label="Invite"
                        icon=Glyph::Plus
                        variant=ButtonVariant::Outline
                        tone=Tone::Primary
                        on_click=Callback::new(move |()| set_inviting.set(true))
                    />
                </div>
                <div class="cv-log">
                    {move || {
                        members
                            .get()
                            .into_iter()
                            .map(|m| {
                                let handle = m.handle.clone();
                                let name = m.name.clone();
                                view! {
                                    <div class="cv-memberrow">
                                        <Avatar initial=m.initial color=m.color size=28 />
                                        <div class="cv-minw-0 cv-grow">
                                            <div class="cv-memberrow__name">
                                                {name.clone()}
                                                {m
                                                    .owner
                                                    .then(|| {
                                                        view! {
                                                            <span class="cv-fs-2xs cv-fg-faint">" — you"</span>
                                                        }
                                                    })}
                                            </div>
                                            <div class="cv-memberrow__handle">
                                                {format!("@{handle}")}
                                            </div>
                                        </div>
                                        {if m.owner {
                                            view! { <span class="cv-memberrow__role">"Owner"</span> }
                                                .into_any()
                                        } else {
                                            view! {
                                                <button
                                                    type="button"
                                                    class="cv-memberrow__x"
                                                    aria-label=format!("Remove {name}")
                                                    on:click=move |_| {
                                                        members
                                                            .update(|ms| { ms.retain(|x| x.handle != handle) });
                                                        set_flash
                                                            .set(
                                                                Some(
                                                                    format!(
                                                                        "{name} removed — their decisions stay, authored and searchable.",
                                                                    ),
                                                                ),
                                                            );
                                                    }
                                                >
                                                    {Glyph::Close.glyph()}
                                                </button>
                                            }
                                                .into_any()
                                        }}
                                    </div>
                                }
                            })
                            .collect_view()
                    }}
                </div>
                <span class="cv-setform__hint">
                    {move || {
                        if members.get().len() == 1 {
                            "It's just you so far. Invite whoever should read and add to these decisions."
                                .to_string()
                        } else {
                            "Everyone here sees every project in the group.".to_string()
                        }
                    }}
                </span>
            </div>

            <div class="cv-danger">
                <div class="cv-danger__title">"Danger zone"</div>
                <div class="cv-danger__row">
                    <div class="cv-grow">
                        <div class="cv-danger__what">{format!("Archive {group_name}")}</div>
                        <div class="cv-danger__why">
                            "The group leaves the sidebar. Its decisions stay searchable and known to the expert model."
                        </div>
                    </div>
                    <Button
                        label="Archive"
                        variant=ButtonVariant::Outline
                        on_click=Callback::new(move |()| {
                            set_flash.set(Some("Archiving a group isn't wired up yet.".into()))
                        })
                    />
                </div>
                <div class="cv-danger__row">
                    <div class="cv-grow">
                        <div class="cv-danger__what">{format!("Delete {group_name}")}</div>
                        <div class="cv-danger__why">
                            {format!(
                                "Removes {projects} {} and {decisions} {} permanently. Cross-references from other groups will break.",
                                if projects == 1 { "project" } else { "projects" },
                                if decisions == 1 { "decision" } else { "decisions" },
                            )}
                        </div>
                    </div>
                    <Button
                        label="Delete"
                        variant=ButtonVariant::Outline
                        tone=Tone::Danger
                        on_click=Callback::new(move |()| {
                            set_flash.set(Some("Deleting a group isn't wired up yet.".into()))
                        })
                    />
                </div>
            </div>

            {move || flash.get().map(|msg| view! { <div class="cv-flash">{msg}</div> })}

            <div class="cv-flash">
                "Preview — the member list is a local fixture, not the server's membership."
            </div>
        </div>

        {move || {
            inviting
                .get()
                .then(|| {
                    view! {
                        <InviteModal
                            on_close=Callback::new(move |()| set_inviting.set(false))
                            on_invite=Callback::new(move |m: Member| {
                                let handle = m.handle.clone();
                                members.update(|ms| ms.push(m));
                                set_inviting.set(false);
                                set_flash.set(Some(format!("@{handle} added.")));
                            })
                        />
                    }
                })
        }}
    }
}

/// Add a member by handle. Signing in is what creates a user, so there is no
/// invite to send and nothing pending — you add someone who already exists.
#[component]
fn InviteModal(on_close: Callback<()>, on_invite: Callback<Member>) -> impl IntoView {
    let (handle, set_handle) = signal(String::new());
    let input_ref = NodeRef::<html::Input>::new();
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            let _ = el.focus();
        }
    });

    let submit = Callback::new(move |()| {
        let h: String = handle
            .get_untracked()
            .trim()
            .trim_start_matches('@')
            .to_lowercase();
        if h.is_empty() {
            return;
        }
        // Without a directory to resolve against, the handle is the name.
        let name = {
            let mut c = h.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => h.clone(),
            }
        };
        let initial = initials(&name);
        let color = converge_ui::domain::Author::human_named(&initial, &name).color();
        on_invite.run(Member {
            initial,
            color,
            name,
            handle: h,
            owner: false,
        });
    });

    view! {
        <Modal
            title="Add a teammate"
            subtitle="By their Converge handle — signing in is what creates a user, so there are no invites to chase."
            on_close=on_close
        >
            <div class="cv-input">
                <span class="cv-input__lead">"@"</span>
                <input
                    node_ref=input_ref
                    class="cv-input__field"
                    placeholder="handle"
                    prop:value=handle
                    on:input=move |ev| set_handle.set(event_target_value(&ev))
                    on:keydown=move |ev| match ev.key().as_str() {
                        "Enter" => {
                            ev.prevent_default();
                            submit.run(());
                        }
                        "Escape" => on_close.run(()),
                        _ => {}
                    }
                />
            </div>
            <div class="cv-modal__foot">
                <Button label="Cancel" variant=ButtonVariant::Ghost on_click=on_close />
                <Button
                    label="Add"
                    tone=Tone::Primary
                    disabled=Signal::derive(move || handle.get().trim().is_empty())
                    on_click=submit
                />
            </div>
        </Modal>
    }
}
