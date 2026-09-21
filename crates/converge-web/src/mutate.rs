//! Create / edit mutations for groups and projects.
//!
//! On the `api` build each calls the live server through `converge-client`,
//! then reflects the confirmed result in the store — an optimistic-feeling
//! update with no full reload (see `data::*_local`). The embedded build has no
//! server, so it applies locally with a generated slug id, enough for the
//! offline demo. The dataset is only touched on success, so the UI never
//! shows a create the server rejected. Forms stay open until success, retain
//! their input on failure, and hand late failures to the shell.

use converge_ui::domain::GroupKind;
use leptos::prelude::*;

use crate::data;
use crate::feedback::ActionState;
use crate::route::{Route, navigate};
use crate::store::{AppStateStoreFields, Notice, push_notice, use_store};
#[cfg(feature = "api")]
use converge_client::StoreError;

/// Confirm a write that changed nothing visible on screen — an edited
/// description, say. The toast lives above the router, so unlike anything the
/// screen itself could set, it survives the screen being re-created.
fn done(store: crate::store::AppStore, message: &str) {
    push_notice(store, Notice::Ok(message.to_string()));
}

/// Create a group, switch to it, and land on its (empty) dashboard.
pub fn create_group(name: String, kind: GroupKind, action: ActionState, close: Callback<()>) {
    let store = use_store();
    if !action.begin() {
        return;
    }
    #[cfg(feature = "api")]
    {
        use converge_client::{GroupKind as Ck, NewGroup};
        let ck = match kind {
            GroupKind::Shared => Ck::Shared,
            GroupKind::Personal => Ck::Personal,
        };
        leptos::task::spawn_local(async move {
            let new = NewGroup {
                name: name.clone(),
                description: None,
                kind: ck,
            };
            match crate::store::client().group_add(&new).await {
                Ok(id) => {
                    let active = action.finish();
                    if active {
                        close.run(());
                    }
                    let idx = data::add_group_local(store, id.to_string(), name, kind);
                    if active {
                        store.group().set(idx);
                        navigate(&Route::Dashboard);
                    }
                    done(store, "Group created.");
                }
                Err(e) => action.fail("Couldn't create group", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        close.run(());
        let idx = data::add_group_local(store, slug(&name), name, kind);
        store.group().set(idx);
        navigate(&Route::Dashboard);
    }
}

/// Create a project in the current group; stay in place (the sidebar unlocks
/// its full layout once the group is no longer empty).
pub fn create_project(name: String, action: ActionState, close: Callback<()>) {
    let store = use_store();
    if !action.begin() {
        return;
    }
    let group_id = data::cur_group().id;
    #[cfg(feature = "api")]
    {
        use converge_client::{GroupId, NewProject};
        let Ok(gid) = group_id.parse::<GroupId>() else {
            action.fail(
                "Couldn't create project",
                &StoreError::Backend("invalid group id".into()),
            );
            return;
        };
        leptos::task::spawn_local(async move {
            let new = NewProject {
                group_id: gid,
                name: name.clone(),
                description: None,
                repository: None,
            };
            match crate::store::client().project_add(&new).await {
                Ok(id) => {
                    if action.finish() {
                        close.run(());
                    }
                    data::add_project_local(store, &group_id, id.to_string(), name, None);
                    done(store, "Project created.");
                }
                Err(e) => action.fail("Couldn't create project", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        close.run(());
        let id = slug(&name);
        data::add_project_local(store, &group_id, id, name, None);
    }
}

/// Edit a group's name and description. The id stays fixed, so membership, its
/// projects and every decision recorded under it are untouched. `kind` is not
/// here on purpose: it is fixed at creation, and turning a personal space into
/// a shared one is a separate operation the server doesn't offer yet.
pub fn edit_group(id: String, name: String, desc: String, action: ActionState) {
    let store = use_store();
    if !action.begin() {
        return;
    }
    let description = (!desc.trim().is_empty()).then(|| desc.clone());
    #[cfg(feature = "api")]
    {
        use converge_client::{GroupEdit, GroupId};
        let Ok(gid) = id.parse::<GroupId>() else {
            // "Impossible" (ids come from the dataset), but a silent Save is
            // worse than a strange toast if it ever happens.
            action.fail(
                "Couldn't save group",
                &StoreError::Backend("invalid group id".into()),
            );
            return;
        };
        let edits = vec![
            GroupEdit::SetName(name.clone()),
            GroupEdit::SetDescription(description.clone()),
        ];
        leptos::task::spawn_local(async move {
            match crate::store::client().group_edit(gid, &edits).await {
                Ok(()) => {
                    action.finish();
                    data::edit_group_local(store, &id, name, description);
                    done(store, "Group saved.");
                }
                // Editing is owner-only server-side; a member's attempt comes
                // back as a refusal beside the Save action.
                Err(e) => action.fail("Couldn't save group", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        data::edit_group_local(store, &id, name, description);
        done(store, "Group saved.");
    }
}

/// Edit a project's display name and description; the id stays fixed, so every
/// reference and decision link is untouched.
pub fn edit_project(id: String, name: String, desc: String, action: ActionState) {
    let store = use_store();
    if !action.begin() {
        return;
    }
    let description = (!desc.trim().is_empty()).then(|| desc.clone());
    #[cfg(feature = "api")]
    {
        use converge_client::{ProjectEdit, ProjectId};
        let Ok(pid) = id.parse::<ProjectId>() else {
            action.fail(
                "Couldn't save project",
                &StoreError::Backend("invalid project id".into()),
            );
            return;
        };
        let edits = vec![
            ProjectEdit::SetName(name.clone()),
            ProjectEdit::SetDescription(description.clone()),
        ];
        leptos::task::spawn_local(async move {
            match crate::store::client().project_edit(pid, &edits).await {
                Ok(()) => {
                    action.finish();
                    data::edit_project_local(store, &id, name, description);
                    done(store, "Project saved.");
                }
                Err(e) => action.fail("Couldn't save project", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        data::edit_project_local(store, &id, name, description);
        done(store, "Project saved.");
    }
}

/// Keep whole conversations for this project, or only the turns its
/// decisions cite. A project-wide call: the conversations belong to
/// everyone working in it, not to the machine that recorded them.
pub fn set_project_archives(id: String, keep: bool, action: ActionState) {
    let store = use_store();
    if !action.begin() {
        return;
    }
    let said = |keep: bool| match keep {
        true => "Whole conversations are kept for this project.",
        false => "Only the lines decisions cite are kept now.",
    };
    #[cfg(feature = "api")]
    {
        use converge_client::{ProjectEdit, ProjectId};
        let Ok(pid) = id.parse::<ProjectId>() else {
            action.fail(
                "Couldn't change what this project keeps",
                &StoreError::Backend("invalid project id".into()),
            );
            return;
        };
        let edits = vec![ProjectEdit::SetArchiveTranscripts(keep)];
        leptos::task::spawn_local(async move {
            match crate::store::client().project_edit(pid, &edits).await {
                Ok(()) => {
                    action.finish();
                    data::set_proj_archives_local(store, &id, keep);
                    done(store, said(keep));
                }
                Err(e) => action.fail("Couldn't change what this project keeps", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        data::set_proj_archives_local(store, &id, keep);
        done(store, said(keep));
    }
}

/// Delete a project permanently — decisions, sessions, the lot. The
/// dataset flips only on server success; until then the confirmation
/// stays open, and a refusal leaves both the screen and typed name intact.
pub fn project_delete(id: String, name: String, action: ActionState, close: Callback<()>) {
    let store = use_store();
    let go = crate::route::navigation();
    if !action.begin() {
        return;
    }
    #[cfg(feature = "api")]
    {
        use converge_client::ProjectId;
        let Ok(pid) = id.parse::<ProjectId>() else {
            action.fail(
                "Couldn't delete project",
                &StoreError::Backend("invalid project id".into()),
            );
            return;
        };
        leptos::task::spawn_local(async move {
            match crate::store::client().project_delete(pid).await {
                Ok(()) => {
                    if action.finish() {
                        close.run(());
                    }
                    if crate::route::current_route().project_target() == Some(id.as_str()) {
                        go.run(Route::Dashboard);
                    }
                    data::drop_project_local(store, &id);
                    done(store, &format!("Project “{name}” deleted."));
                }
                // The evidence-pinning refusal (409) lands here verbatim.
                Err(e) => action.fail("Couldn't delete project", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        close.run(());
        if crate::route::current_route().project_target() == Some(id.as_str()) {
            go.run(Route::Dashboard);
        }
        data::drop_project_local(store, &id);
        done(store, &format!("Project “{name}” deleted."));
    }
}

/// Delete a group and everything under it. Preserve a different active group;
/// if the deleted group was selected, fall back to the first remaining one.
pub fn group_delete(id: String, name: String, action: ActionState, close: Callback<()>) {
    let store = use_store();
    let go = crate::route::navigation();
    if !action.begin() {
        return;
    }
    #[cfg(feature = "api")]
    {
        use converge_client::GroupId;
        let Ok(gid) = id.parse::<GroupId>() else {
            action.fail(
                "Couldn't delete group",
                &StoreError::Backend("invalid group id".into()),
            );
            return;
        };
        leptos::task::spawn_local(async move {
            match crate::store::client().group_delete(gid).await {
                Ok(()) => {
                    if action.finish() {
                        close.run(());
                    }
                    if crate::route::current_route() == Route::GroupSettings
                        && store.dataset().get_untracked().is_some_and(|data| {
                            data.groups
                                .get(store.group().get_untracked())
                                .is_some_and(|group| group.id == id)
                        })
                    {
                        go.run(Route::Dashboard);
                    }
                    data::drop_group_local(store, &id);
                    done(store, &format!("Group “{name}” deleted."));
                }
                Err(e) => action.fail("Couldn't delete group", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        close.run(());
        if crate::route::current_route() == Route::GroupSettings {
            go.run(Route::Dashboard);
        }
        data::drop_group_local(store, &id);
        done(store, &format!("Group “{name}” deleted."));
    }
}

/// Resolve a signal with the user's verdict: confirm (it holds) or
/// dismiss (it will not be raised again). The dataset flips only on
/// server success; dismissal drops the signal from every list.
pub fn resolve_signal(id: String, confirm: bool, action: ActionState) {
    use crate::seed::SignalStatus;
    let store = use_store();
    if !action.begin() {
        return;
    }
    let status = if confirm {
        SignalStatus::Confirmed
    } else {
        SignalStatus::Dismissed
    };
    #[cfg(feature = "api")]
    {
        use converge_client::{Author, SignalId, SignalStatus as Ws, UserId};
        let Ok(sid) = id.parse::<SignalId>() else {
            action.fail(
                "Couldn't resolve the signal",
                &StoreError::Backend("invalid signal id".into()),
            );
            return;
        };
        let me = data::account().user_id;
        let Ok(uid) = me.parse::<UserId>() else {
            action.fail(
                "Couldn't resolve the signal",
                &StoreError::Backend("invalid user id".into()),
            );
            return;
        };
        let ws = if confirm {
            Ws::Confirmed
        } else {
            Ws::Dismissed
        };
        leptos::task::spawn_local(async move {
            match crate::store::client()
                .signal_resolve(sid, ws, &Author::User(uid))
                .await
            {
                Ok(()) => {
                    if action.finish() && !confirm {
                        navigate(&Route::Signals);
                    }
                    data::resolve_signal_local(store, &id, status);
                    done(
                        store,
                        if confirm {
                            "Signal confirmed."
                        } else {
                            "Signal dismissed."
                        },
                    );
                }
                Err(e) => action.fail("Couldn't resolve the signal", &e),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        action.finish();
        if !confirm {
            navigate(&Route::Signals);
        }
        data::resolve_signal_local(store, &id, status);
    }
}

/// Slug an entered name into a unique id — embedded build only (the API mints
/// ULIDs). Lowercase, non-alphanumeric runs collapse to `-`, trimmed; empty →
/// `untitled`; a collision gets `-2`, `-3`, ….
#[cfg(not(feature = "api"))]
fn slug(name: &str) -> String {
    let mut base = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !base.is_empty() {
            base.push('-');
            dash = true;
        }
    }
    let base = base.trim_end_matches('-').to_string();
    let base = if base.is_empty() {
        "untitled".to_string()
    } else {
        base
    };
    let ds = use_store().dataset().get_untracked();
    let taken = |id: &str| {
        ds.as_ref().is_some_and(|d| {
            d.groups.iter().any(|g| g.id == id) || d.projects.iter().any(|p| p.id == id)
        })
    };
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base}-{i}"))
        .find(|c| !taken(c))
        .unwrap()
}
