//! `#/members` — the group's roster: who's in, who's invited, who can do what.
//!
//! Reached from the dashboard's "⋯" menu. The layout is the members-first one:
//! the group name *is* the heading (the "✎" beside it opens the same rename
//! modal the menu does), the roster is a table, and the destructive actions are
//! one quiet line in the footer.
//!
//! **The membership data here is a local fixture, not the server's.** The API
//! carries membership already (list, add by handle, remove) but `converge-client`
//! has no methods for it, and roles and pending invites don't exist in the model
//! at all — the ACL is flat: one owner plus members, added directly, no invite
//! state. So this screen draws the whole target design and backs it with mock
//! rows: the people are the group's own decision authors (so avatars, names and
//! initials match the feed), while roles, join dates, the invited row and the
//! username directory are invented here. Every action mutates component state
//! and nothing else. Swapping in the real thing means replacing [`mock_roster`]
//! and the four action callbacks — the markup stays.

use converge_ui::atoms::{Avatar, Badge, Button, ButtonVariant, Callout, Glyph, Modal, Select};
use converge_ui::domain::{AuthorKind, Tone, initials};
use leptos::prelude::*;

use crate::data;
use crate::modals::{ModalKind, open};

/// What someone may do in the group. `Owner` is structural (the group's
/// creator) and can't be assigned; the other two are the assignable roles.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::Owner => "Owner",
            Role::Admin => "Admin",
            Role::Member => "Member",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    fn from_slug(s: &str) -> Role {
        match s {
            "admin" => Role::Admin,
            "owner" => Role::Owner,
            _ => Role::Member,
        }
    }

    /// The two roles a row's select offers, current one first (the atom shows
    /// the leading option as selected).
    fn options(self) -> Vec<(String, String)> {
        let (a, b) = match self {
            Role::Admin => (Role::Admin, Role::Member),
            _ => (Role::Member, Role::Admin),
        };
        vec![
            (a.slug().to_string(), a.label().to_string()),
            (b.slug().to_string(), b.label().to_string()),
        ]
    }
}

/// One roster row — all-owned so the reactive closures stay `Send` (the
/// dataset's `Rc<Dec>` snapshots never cross into them).
#[derive(Clone, PartialEq)]
struct Row {
    initial: String,
    color: String,
    name: String,
    handle: String,
    role: Role,
    /// Display-only join date; "sent 2d ago" for a pending invite.
    joined: String,
    /// Invited but not yet accepted — no role, and the ✕ revokes instead of
    /// removing.
    invited: bool,
    /// The signed-in account's own row: no role select, no ✕.
    is_me: bool,
}

/// People who exist in "Converge" but aren't in this group — what the invite
/// field resolves against. Mock, obviously; the real lookup is the server's
/// `user_lookup`, which `member_add` already runs on the handle it's given.
/// Deliberately names nobody from the fixture's decisions, so every one of
/// them resolves as invitable rather than "already a member".
const DIRECTORY: [(&str, &str); 4] = [
    ("Sofia Ortiz", "sofia"),
    ("Nadia Rahman", "nadia"),
    ("Tom Becker", "tom"),
    ("Ines Duarte", "ines"),
];

/// Join dates, handed out in order down the roster.
const JOINED: [&str; 6] = ["2 Mar", "4 Mar", "4 Mar", "11 Mar", "18 Mar", "2 Apr"];

/// A handle from a display name: the first word, lowercased (`Priya Nair` →
/// `priya`), matching how the fixture's people are addressed elsewhere.
fn handle_of(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or(name)
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// The mock roster: the account as owner, then the humans who authored
/// decisions in this group, then one pending invite so the state is visible.
fn mock_roster() -> Vec<Row> {
    let me = data::account();
    let mut rows = vec![Row {
        initial: me.initial.clone(),
        color: me.color.clone(),
        name: me.name.clone(),
        handle: handle_of(&me.name),
        role: Role::Owner,
        joined: JOINED[0].to_string(),
        invited: false,
        is_me: true,
    }];

    for dec in data::group_decisions() {
        for a in &dec.authors {
            if a.kind == AuthorKind::Agent || a.name == me.name {
                continue;
            }
            if rows.iter().any(|r| r.name == a.name) {
                continue;
            }
            let n = rows.len();
            rows.push(Row {
                initial: a.initial.clone(),
                color: a.color(),
                name: a.name.clone(),
                handle: handle_of(&a.name),
                // The first teammate is an admin; everyone else a plain member.
                role: if n == 1 { Role::Admin } else { Role::Member },
                joined: JOINED[n.min(JOINED.len() - 1)].to_string(),
                invited: false,
                is_me: false,
            });
        }
    }

    // One pending invite, so the invited row isn't a state you have to create
    // by hand to see. Pick someone from the directory who isn't in yet.
    if let Some((name, handle)) = DIRECTORY
        .iter()
        .find(|(_, h)| !rows.iter().any(|r| r.handle == *h))
    {
        rows.push(invited_row(name, handle));
    }
    rows
}

/// A pending-invite row for `name`/`handle` (avatar tint comes from the shared
/// hashed palette, same as everywhere else).
fn invited_row(name: &str, handle: &str) -> Row {
    let initial = initials(name);
    let color = converge_ui::domain::Author::human_named(&initial, name).color();
    Row {
        initial,
        color,
        name: name.to_string(),
        handle: handle.to_string(),
        role: Role::Member,
        joined: "sent 2d ago".to_string(),
        invited: true,
        is_me: false,
    }
}

/// What the invite field's current text resolves to.
#[derive(Clone, PartialEq)]
enum Lookup {
    /// Nothing typed yet.
    Empty,
    /// A Converge user who isn't in the group.
    Found { name: String, handle: String },
    /// No such user — the fallback is an e-mail invite.
    Missing(String),
    /// Already on the roster (or already invited). Carries the row's own
    /// avatar so the face matches the one in the table below.
    Already {
        name: String,
        note: String,
        initial: String,
        color: String,
    },
}

#[component]
pub fn Members() -> impl IntoView {
    let group = data::group_name();
    let rows = RwSignal::new(mock_roster());
    let (query, set_query) = signal(String::new());
    let (role, set_role) = signal(Role::Member);
    // The member the remove dialog is asking about.
    let (removing, set_removing) = signal(None::<Row>);
    // One-line feedback for the actions that only pretend to do something.
    let (flash, set_flash) = signal(None::<String>);

    // Escape closes the remove dialog — it carries no input of its own to hang
    // the key handler on. Removed by hand, like the project log's menu: leptos
    // never detaches window listeners when the owner is disposed.
    let escape = window_event_listener(leptos::ev::keydown, move |evt| {
        if evt.key() == "Escape" {
            set_removing.set(None);
        }
    });
    on_cleanup(move || escape.remove());

    let projects = data::cur_group_projects().len();
    let decisions = data::group_decisions().len();
    let kind = match data::cur_group().kind {
        converge_ui::domain::GroupKind::Personal => "Personal group",
        converge_ui::domain::GroupKind::Shared => "Shared group",
    };
    let meta = format!(
        "{kind} · {projects} {} · {decisions} {}",
        if projects == 1 { "project" } else { "projects" },
        if decisions == 1 {
            "decision"
        } else {
            "decisions"
        },
    );
    let gid = data::cur_group().id;

    // Resolve the typed handle against the roster first, then the directory.
    let lookup = Signal::derive(move || {
        let q = query.get().trim().trim_start_matches('@').to_lowercase();
        if q.is_empty() {
            return Lookup::Empty;
        }
        if let Some(r) = rows.get().into_iter().find(|r| r.handle == q) {
            let note = if r.invited {
                "already invited · waiting".to_string()
            } else {
                format!("already in {} · {}", data::group_name(), r.role.label())
            };
            return Lookup::Already {
                name: r.name,
                note,
                initial: r.initial,
                color: r.color,
            };
        }
        match DIRECTORY.iter().find(|(_, h)| *h == q) {
            Some((name, handle)) => Lookup::Found {
                name: name.to_string(),
                handle: handle.to_string(),
            },
            None => Lookup::Missing(q),
        }
    });

    let invite = Callback::new(move |()| {
        let Lookup::Found { name, handle } = lookup.get_untracked() else {
            return;
        };
        let mut row = invited_row(&name, &handle);
        row.role = role.get_untracked();
        row.joined = "sent just now".to_string();
        rows.update(|rs| rs.push(row));
        set_query.set(String::new());
        set_flash.set(Some(format!(
            "Invited @{handle} as {}.",
            role.get_untracked().label()
        )));
    });

    let group_for_dialog = group.clone();
    let group_for_hint = group.clone();
    view! {
        <div class="cv-page">
            <div class="cv-members">
                <div class="cv-members__title">
                    <h1 class="cv-heading cv-fs-4xl">{group.clone()}</h1>
                    <button
                        type="button"
                        class="cv-members__pencil"
                        aria-label="Rename group"
                        on:click=move |_| open(ModalKind::RenameGroup(gid.clone()))
                    >
                        {Glyph::Edit.glyph()}
                    </button>
                </div>
                <div class="cv-members__meta">{meta}</div>

                <div class="cv-invite">
                    <div class="cv-invite__head">"Invite someone"</div>
                    <div class="cv-invite__row">
                        <div class="cv-input">
                            <span class="cv-input__lead">"@"</span>
                            <input
                                class="cv-input__field"
                                placeholder="Converge username"
                                prop:value=query
                                on:input=move |ev| set_query.set(event_target_value(&ev))
                                on:keydown=move |ev| {
                                    if ev.key() == "Enter" {
                                        ev.prevent_default();
                                        invite.run(());
                                    }
                                }
                            />
                        </div>
                        <Select
                            options=Role::Member.options()
                            on_change=Callback::new(move |v: String| {
                                set_role.set(Role::from_slug(&v))
                            })
                        />
                        <Button
                            label="Send invite"
                            tone=Tone::Primary
                            disabled=Signal::derive(move || {
                                !matches!(lookup.get(), Lookup::Found { .. })
                            })
                            on_click=invite
                        />
                    </div>
                    {move || match lookup.get() {
                        Lookup::Empty => {
                            view! {
                                <p class="cv-invite__note">
                                    "Members see every project in the group. Admins can also invite, rename, and archive."
                                </p>
                            }
                                .into_any()
                        }
                        Lookup::Found { name, handle } => {
                            let initial = initials(&name);
                            let color = converge_ui::domain::Author::human_named(&initial, &name)
                                .color();
                            view! {
                                <div class="cv-resolve">
                                    <Avatar initial=initial color=color size=30 />
                                    <div class="cv-resolve__body">
                                        <div class="cv-resolve__name">{name}</div>
                                        <div class="cv-resolve__sub">{format!("@{handle}")}</div>
                                    </div>
                                    <Button label="Invite" tone=Tone::Primary on_click=invite />
                                </div>
                            }
                                .into_any()
                        }
                        Lookup::Already {
                            name,
                            note,
                            initial,
                            color,
                        } => {
                            view! {
                                <div class="cv-resolve">
                                    <Avatar initial=initial color=color size=30 />
                                    <div class="cv-resolve__body">
                                        <div class="cv-resolve__name">{name}</div>
                                        <div class="cv-resolve__sub">{note}</div>
                                    </div>
                                </div>
                            }
                                .into_any()
                        }
                        Lookup::Missing(q) => {
                            view! {
                                <div class="cv-resolve cv-resolve--miss">
                                    <div class="cv-resolve__msg">
                                        {format!("No Converge user @{q}. ")}
                                        "They need to sign in to Converge once — then their handle resolves here."
                                    </div>
                                </div>
                            }
                                .into_any()
                        }
                    }}
                </div>

                <div class="cv-roster__head">
                    <span class="cv-grow">"Member"</span>
                    <span class="cv-roster__role">"Role"</span>
                    <span class="cv-roster__joined">"Joined"</span>
                    <span style="width:1.75rem;flex:none"></span>
                </div>
                {move || {
                    rows.get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let row = r.clone();
                            let name = r.name.clone();
                            let handle = r.handle.clone();
                            view! {
                                <div class="cv-roster__row">
                                    <div class="cv-roster__who">
                                        <Avatar initial=r.initial color=r.color size=30 />
                                        <div class="cv-minw-0">
                                            <div class="cv-roster__name">
                                                {name.clone()}
                                                {r
                                                    .is_me
                                                    .then(|| {
                                                        view! { <span class="cv-roster__you">"· you"</span> }
                                                    })}
                                                {r
                                                    .invited
                                                    .then(|| {
                                                        view! { <Badge label="invited" tone=Tone::Signal /> }
                                                    })}
                                            </div>
                                            <div class="cv-roster__handle">
                                                {format!("@{handle}")}
                                            </div>
                                        </div>
                                    </div>
                                    <div class="cv-roster__role">
                                        {if r.role == Role::Owner {
                                            view! { <span class="cv-roster__owner">"Owner"</span> }
                                                .into_any()
                                        } else if r.invited {
                                            view! {
                                                <span class="cv-roster__owner">{r.role.label()}</span>
                                            }
                                                .into_any()
                                        } else {
                                            view! {
                                                <Select
                                                    options=r.role.options()
                                                    on_change=Callback::new(move |v: String| {
                                                        rows.update(|rs| rs[i].role = Role::from_slug(&v));
                                                    })
                                                />
                                            }
                                                .into_any()
                                        }}
                                    </div>
                                    <span class="cv-roster__joined">{r.joined}</span>
                                    {if r.is_me {
                                        view! { <span class="cv-roster__x cv-roster__x--none"></span> }
                                            .into_any()
                                    } else if r.invited {
                                        view! {
                                            <button
                                                type="button"
                                                class="cv-roster__x"
                                                aria-label=format!("Revoke invite to {name}")
                                                on:click=move |_| {
                                                    rows.update(|rs| { rs.retain(|x| x.handle != handle); });
                                                    set_flash.set(Some("Invite revoked.".into()));
                                                }
                                            >
                                                {Glyph::Close.glyph()}
                                            </button>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <button
                                                type="button"
                                                class="cv-roster__x"
                                                aria-label=format!("Remove {name}")
                                                on:click=move |_| set_removing.set(Some(row.clone()))
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

                <div class="cv-agenthint">
                    <span class="cv-agenthint__mark">{Glyph::Expert.glyph()}</span>
                    <span>
                        "Your agent can invite and remove members too — "
                        <em>{format!("“invite @nadia to {group_for_hint} as a member”")}</em> "."
                    </span>
                </div>

                <div class="cv-dangerbar">
                    <span class="cv-grow">
                        {format!(
                            "Archiving keeps all {decisions} decisions searchable. Deleting removes them for everyone.",
                        )}
                    </span>
                    <button
                        type="button"
                        class="cv-dangerbar__act"
                        on:click=move |_| {
                            set_flash.set(Some("Archiving a group isn't wired up yet.".into()))
                        }
                    >
                        "Archive group"
                    </button>
                    <button
                        type="button"
                        class="cv-dangerbar__act cv-dangerbar__act--danger"
                        on:click=move |_| {
                            set_flash.set(Some("Deleting a group isn't wired up yet.".into()))
                        }
                    >
                        "Delete group"
                    </button>
                </div>

                {move || {
                    flash
                        .get()
                        .map(|msg| view! { <div class="cv-flash">{msg}</div> })
                }}

                <div class="cv-members__foot">
                    "Preview — the roster on this screen is a local fixture, not the server's membership."
                </div>
            </div>
        </div>

        // Removing a member is a calm dialog, not a red warning: the point is
        // that their decisions survive them leaving.
        {move || {
            removing
                .get()
                .map(|r| {
                    let close = Callback::new(move |()| set_removing.set(None));
                    let handle = r.handle.clone();
                    let name = r.name.clone();
                    view! {
                        <Modal
                            title=format!("Remove {} from {}?", r.name, group_for_dialog)
                            subtitle=format!(
                                "They lose access to all {projects} {} immediately.",
                                if projects == 1 { "project" } else { "projects" },
                            )
                            on_close=close
                        >
                            <Callout tone=Tone::Neutral>
                                "Their decisions stay — still authored by "
                                <span class="cv-mono">{format!("@{}", r.handle)}</span>
                                ", searchable, quoted in supersession chains, and known to the expert model. Invite them back any time."
                            </Callout>
                            <div class="cv-modal__foot">
                                <Button
                                    label="Cancel"
                                    variant=ButtonVariant::Ghost
                                    on_click=close
                                />
                                <Button
                                    label="Remove member"
                                    tone=Tone::Danger
                                    on_click=Callback::new(move |()| {
                                        let h = handle.clone();
                                        rows.update(|rs| rs.retain(|x| x.handle != h));
                                        set_removing.set(None);
                                        set_flash.set(Some(format!("{name} removed.")));
                                    })
                                />
                            </div>
                        </Modal>
                    }
                })
        }}
    }
}
