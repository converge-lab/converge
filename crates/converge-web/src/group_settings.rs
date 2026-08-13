//! `#/group/settings` — everything about the active group in one place: what
//! it's called, who's in it, and how to get rid of it.
//!
//! Reached from the "⋯" beside the group's name on the dashboard. The order is
//! by frequency: name and description on top, members in the middle, the
//! destructive actions fenced off at the bottom.
//!
//! Name, description, membership and deletion are all real (`PATCH
//! /groups/{id}`, the members endpoints, `DELETE /groups/{id}` behind
//! the retype-to-confirm modal). Two things stay drawn-but-inert, and
//! say so where they stand: visibility (a group's kind is fixed at
//! creation) and archive (no domain support yet). The embedded build
//! keeps a local member fixture; see `mock_members`.

use converge_ui::atoms::{Avatar, Button, ButtonVariant, Glyph, Modal};
#[cfg(not(feature = "api"))]
use converge_ui::domain::AuthorKind;
use converge_ui::domain::{GroupKind, Tone, initials};
use leptos::html;
use leptos::prelude::*;

use crate::modals::{self, ModalKind};
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
    /// The server's user id (`None` on the embedded fixture).
    user_id: Option<String>,
}

/// A handle from a display name: the first word, lowercased.
#[cfg(not(feature = "api"))]
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
#[cfg(not(feature = "api"))]
fn mock_members() -> Vec<Member> {
    let me = data::account();
    let mut rows = vec![Member {
        initial: me.initial.clone(),
        color: me.color.clone(),
        name: me.name.clone(),
        handle: handle_of(&me.name),
        owner: true,
        user_id: None,
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
                user_id: None,
            });
        }
    }
    rows
}

/// The real roster (owner first, marked — the server's contract).
#[cfg(feature = "api")]
fn load_members(
    gid: String,
    members: RwSignal<Vec<Member>>,
    set_flash: WriteSignal<Option<String>>,
) {
    use converge_client::GroupId;
    let Ok(id) = gid.parse::<GroupId>() else {
        return;
    };
    leptos::task::spawn_local(async move {
        match crate::store::client().member_list(id).await {
            Ok(list) => members.set(
                list.into_iter()
                    .map(|m| {
                        let initial = initials(&m.name);
                        let color =
                            converge_ui::domain::Author::human_named(&initial, &m.name).color();
                        Member {
                            initial,
                            color,
                            name: m.name,
                            handle: m.handle,
                            owner: m.owner,
                            user_id: Some(m.user_id.to_string()),
                        }
                    })
                    .collect(),
            ),
            Err(e) => set_flash.set(Some(format!("Couldn't load members — {e}"))),
        }
    });
}

#[component]
pub fn GroupSettings() -> impl IntoView {
    let group = data::cur_group();
    // A `Copy` handle: the id is read from many `move` closures (save,
    // reload, rows, invite, delete) without a clone-per-closure dance.
    let gid = StoredValue::new(group.id.clone());
    let personal = group.kind == GroupKind::Personal;

    let (name, set_name) = signal(group.name.clone());
    let (desc, set_desc) = signal(group.description.clone().unwrap_or_default());
    let (inviting, set_inviting) = signal(false);
    let (flash, set_flash) = signal(None::<String>);
    let members = RwSignal::new(Vec::<Member>::new());
    #[cfg(feature = "api")]
    let reload = move || load_members(gid.get_value(), members, set_flash);
    #[cfg(not(feature = "api"))]
    let reload = move || members.set(mock_members());
    reload();
    // Membership is managed by the owner; everyone else reads. On the
    // fixture the account owns everything.
    let account_id = data::account().user_id;
    let mine = {
        let account_id = account_id.clone();
        Signal::derive(move || {
            members.with(|ms| {
                ms.iter().any(|m| {
                    m.owner
                        && (m.user_id.is_none()
                            || m.user_id.as_deref() == Some(account_id.as_str()))
                })
            })
        })
    };

    let projects = group.project_ids.len();
    let decisions = data::group_decisions().len();

    let save = Callback::new(move |()| {
        let n = name.get_untracked().trim().to_string();
        if n.is_empty() {
            return;
        }
        // No "saved" line: a dataset write re-creates the active screen
        // (see the router's `track_data`), so any local state set here dies
        // on the same tick. The sidebar and breadcrumb updating is the
        // feedback.
        mutate::edit_group(gid.get_value(), n, desc.get_untracked());
    });

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

            {if personal {
                // A personal space has no membership to manage. Inviting
                // someone is precisely what would turn it into a shared group,
                // and that conversion doesn't exist yet — so this says what
                // would happen rather than offering a list of one.
                view! {
                    <div class="cv-col cv-gap-8 cv-mb-32">
                        <div class="cv-fs-lg cv-fw-medium">"Members"</div>
                        <p class="cv-fg-muted cv-fs-md cv-lh-normal cv-measure">
                            "This is your personal workspace — the decisions here are yours alone. Inviting someone turns it into a shared group, and every project in it becomes visible to them."
                        </p>
                        <div>
                            <Button
                                label="Make it a shared group…"
                                variant=ButtonVariant::Outline
                                tone=Tone::Primary
                                on_click=Callback::new(move |()| {
                                    set_flash
                                        .set(
                                            Some(
                                                "Turning a personal space into a shared group isn't wired up yet."
                                                    .into(),
                                            ),
                                        )
                                })
                            />
                        </div>
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <div class="cv-col cv-gap-8 cv-mb-32">
                <div class="cv-row" style="align-items:flex-start">
                    <div class="cv-grow">
                        <div class="cv-fs-lg cv-fw-medium">"Members"</div>
                        <div class="cv-fs-xs cv-fg-faint cv-mt-4">
                            "Who can read and add to this group's decisions."
                        </div>
                    </div>
                    // Membership is the owner's to manage.
                    {move || {
                        mine.get()
                            .then(|| {
                                view! {
                                    <Button
                                        label="Invite"
                                        icon=Glyph::Plus
                                        variant=ButtonVariant::Outline
                                        tone=Tone::Primary
                                        on_click=Callback::new(move |()| set_inviting.set(true))
                                    />
                                }
                            })
                    }}
                </div>
                <div class="cv-log">
                    {move || {
                        let account_id = account_id.clone();
                        #[cfg(feature = "api")]
                        let gid = gid.get_value();
                        members
                            .get()
                            .into_iter()
                            .map(move |m| {
                                let handle = m.handle.clone();
                                let name = m.name.clone();
                                let you = match &m.user_id {
                                    Some(uid) => *uid == account_id,
                                    None => m.owner,
                                };
                                #[cfg(feature = "api")]
                                let remove: Callback<()> = {
                                    let gid = gid.clone();
                                    let handle = handle.clone();
                                    let uid = m.user_id.clone();
                                    Callback::new(move |()| {
                                        use converge_client::{GroupId, UserId};
                                        let (Ok(g), Some(Ok(u))) = (
                                            gid.parse::<GroupId>(),
                                            uid.as_deref().map(str::parse::<UserId>),
                                        ) else {
                                            return;
                                        };
                                        let handle = handle.clone();
                                        leptos::task::spawn_local(async move {
                                            match crate::store::client().member_remove(g, u).await {
                                                Ok(()) => {
                                                    members.update(|ms| {
                                                        ms.retain(|x| {
                                                            x.user_id.as_deref()
                                                                != Some(&u.to_string())
                                                        })
                                                    });
                                                    set_flash.set(Some(format!(
                                                        "@{handle} removed — their decisions stay, authored and searchable.",
                                                    )));
                                                }
                                                Err(e) => set_flash.set(Some(format!(
                                                    "Couldn't remove @{handle} — {e}"
                                                ))),
                                            }
                                        });
                                    })
                                };
                                #[cfg(not(feature = "api"))]
                                let remove: Callback<()> = {
                                    let handle = handle.clone();
                                    let name = name.clone();
                                    Callback::new(move |()| {
                                        // Keyed by name: handles are derived
                                        // (first word) and collide across
                                        // homonyms; names are unique here.
                                        let name = name.clone();
                                        members.update(|ms| ms.retain(|x| x.name != name));
                                        set_flash.set(Some(format!(
                                            "@{handle} removed — their decisions stay, authored and searchable.",
                                        )));
                                    })
                                };
                                view! {
                                    <div class="cv-memberrow">
                                        <Avatar initial=m.initial color=m.color size=28 />
                                        <div class="cv-minw-0 cv-grow">
                                            <div class="cv-memberrow__name">
                                                {name.clone()}
                                                {you
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
                                        } else if mine.get() {
                                            view! {
                                                <button
                                                    type="button"
                                                    class="cv-memberrow__x"
                                                    aria-label=format!("Remove {name}")
                                                    on:click=move |_| remove.run(())
                                                >
                                                    {Glyph::Close.glyph()}
                                                </button>
                                            }
                                                .into_any()
                                        } else {
                                            ().into_any()
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
                }
                    .into_any()
            }}

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
                        on_click={
                            let name = group_name.clone();
                            Callback::new(move |()| {
                                modals::open(ModalKind::DeleteGroup {
                                    id: gid.get_value(),
                                    name: name.clone(),
                                })
                            })
                        }
                    />
                </div>
            </div>

            {move || flash.get().map(|msg| view! { <div class="cv-flash">{msg}</div> })}
        </div>

        {move || {
            inviting
                .get()
                .then(|| {
                    #[cfg(feature = "api")]
                    let add: Callback<String> = {
                        Callback::new(move |handle: String| {
                            use converge_client::GroupId;
                            set_inviting.set(false);
                            let Ok(g) = gid.get_value().parse::<GroupId>() else {
                                return;
                            };
                            leptos::task::spawn_local(async move {
                                match crate::store::client().member_add(g, &handle).await {
                                    Ok(()) => {
                                        set_flash.set(Some(format!("@{handle} added.")));
                                        reload();
                                    }
                                    // "No user with that handle" / "ambiguous"
                                    // come back worded for humans — verbatim.
                                    Err(e) => set_flash
                                        .set(Some(format!("Couldn't add @{handle} — {e}"))),
                                }
                            });
                        })
                    };
                    #[cfg(not(feature = "api"))]
                    let add: Callback<String> = Callback::new(move |handle: String| {
                        set_inviting.set(false);
                        let known = members
                            .with_untracked(|ms| ms.iter().any(|x| x.handle == handle));
                        if known {
                            set_flash.set(Some(format!("@{handle} is already a member.")));
                            return;
                        }
                        let name = {
                            let mut c = handle.chars();
                            match c.next() {
                                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                                None => handle.clone(),
                            }
                        };
                        let initial = initials(&name);
                        let color =
                            converge_ui::domain::Author::human_named(&initial, &name).color();
                        members.update(|ms| {
                            ms.push(Member {
                                initial,
                                color,
                                name,
                                handle: handle.clone(),
                                owner: false,
                                user_id: None,
                            })
                        });
                        set_flash.set(Some(format!("@{handle} added.")));
                    });
                    view! {
                        <InviteModal
                            on_close=Callback::new(move |()| set_inviting.set(false))
                            on_invite=add
                        />
                    }
                })
        }}
    }
}

/// Add a member by handle. Signing in is what creates a user, so there is no
/// invite to send and nothing pending — you add someone who already exists.
#[component]
fn InviteModal(on_close: Callback<()>, on_invite: Callback<String>) -> impl IntoView {
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
        on_invite.run(h);
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
