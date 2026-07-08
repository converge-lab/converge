//! Create / edit mutations for groups and projects.
//!
//! On the `api` build each calls the live server through `converge-client`,
//! then reflects the confirmed result in the store — an optimistic-feeling
//! update with no full reload (see `data::*_local`). The embedded build has no
//! server, so it applies locally with a generated slug id, enough for the
//! offline demo. Failures are logged; the store is only touched on success, so
//! the UI never shows a create the server rejected.

use converge_ui::domain::GroupKind;
use leptos::prelude::*;

use crate::data;
use crate::route::{Route, navigate};
use crate::store::{AppStateStoreFields, use_store};

/// Create a group, switch to it, and land on its (empty) dashboard.
pub fn create_group(name: String, kind: GroupKind) {
    let store = use_store();
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
                    let idx = data::add_group_local(store, id.to_string(), name, kind);
                    store.group().set(idx);
                    navigate(&Route::Dashboard);
                }
                Err(e) => leptos::logging::error!("create group failed: {e}"),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        let idx = data::add_group_local(store, slug(&name), name, kind);
        store.group().set(idx);
        navigate(&Route::Dashboard);
    }
}

/// Create a project in the current group; stay in place (the sidebar unlocks
/// its full layout once the group is no longer empty).
pub fn create_project(name: String) {
    let store = use_store();
    let group_id = data::cur_group().id;
    #[cfg(feature = "api")]
    {
        use converge_client::{GroupId, NewProject};
        let Ok(gid) = group_id.parse::<GroupId>() else {
            leptos::logging::error!("current group id is not a ULID: {group_id}");
            return;
        };
        leptos::task::spawn_local(async move {
            let new = NewProject {
                group_id: gid,
                name: name.clone(),
                description: None,
            };
            match crate::store::client().project_add(&new).await {
                Ok(id) => data::add_project_local(store, &group_id, id.to_string(), name, None),
                Err(e) => leptos::logging::error!("create project failed: {e}"),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        let id = slug(&name);
        data::add_project_local(store, &group_id, id, name, None);
    }
}

/// Edit a project's display name and description; the id stays fixed, so every
/// reference and decision link is untouched.
pub fn edit_project(id: String, name: String, desc: String) {
    let store = use_store();
    let description = (!desc.trim().is_empty()).then(|| desc.clone());
    #[cfg(feature = "api")]
    {
        use converge_client::{ProjectEdit, ProjectId};
        let Ok(pid) = id.parse::<ProjectId>() else {
            leptos::logging::error!("project id is not a ULID: {id}");
            return;
        };
        let edits = vec![
            ProjectEdit::SetName(name.clone()),
            ProjectEdit::SetDescription(description.clone()),
        ];
        leptos::task::spawn_local(async move {
            match crate::store::client().project_edit(pid, &edits).await {
                Ok(()) => data::edit_project_local(store, &id, name, description),
                Err(e) => leptos::logging::error!("edit project failed: {e}"),
            }
        });
    }
    #[cfg(not(feature = "api"))]
    {
        data::edit_project_local(store, &id, name, description);
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
