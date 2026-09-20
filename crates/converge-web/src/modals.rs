//! The create / edit modals and the little controller that opens them.
//!
//! One `RwSignal<Option<ModalKind>>` lives in context (provided at the app
//! root); any trigger — an onboarding card, a sidebar section's "＋" — opens a
//! modal by setting it, and [`ModalHost`]
//! (mounted once at the root) renders the active one. Submits go through
//! [`crate::mutate`], bound to the live API.

use converge_ui::atoms::{Button, ButtonVariant, Glyph, Modal};
use converge_ui::domain::{GroupKind, Tone};
use leptos::html;
use leptos::prelude::*;

use crate::feedback::{ActionState, ActionStatus};
use crate::{data, mutate};

/// Which modal is open.
#[derive(Clone, PartialEq)]
pub enum ModalKind {
    NewGroup,
    NewProject,
    /// Permanent project deletion; the name is retyped to confirm.
    DeleteProject {
        id: String,
        name: String,
    },
    /// Permanent group deletion — same ceremony, bigger blast radius.
    DeleteGroup {
        id: String,
        name: String,
    },
}

/// The context-shared open-modal signal.
type ModalSignal = RwSignal<Option<ModalKind>>;

/// Publish the modal controller at the app root. Call once, above [`ModalHost`].
pub fn provide_modal_ctl() {
    provide_context(RwSignal::new(None::<ModalKind>));
}

/// The modal controller for the current owner.
pub fn use_modal() -> ModalSignal {
    expect_context::<ModalSignal>()
}

/// Open a modal (from any trigger).
pub fn open(kind: ModalKind) {
    use_modal().set(Some(kind));
}

/// Focus an input the moment it mounts — dynamic inserts don't honor the HTML
/// `autofocus` attribute, and there is no DOM on native (compile-only) builds.
fn autofocus(input_ref: NodeRef<html::Input>) {
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            let _ = el.focus();
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = input_ref;
}

/// Renders the active modal (or nothing). Mount once at the app root.
#[component]
pub fn ModalHost() -> impl IntoView {
    let modal = use_modal();
    move || {
        modal.get().map(|kind| match kind {
            ModalKind::NewGroup => view! { <NewGroupModal /> }.into_any(),
            ModalKind::NewProject => view! { <NewProjectModal /> }.into_any(),
            ModalKind::DeleteProject { id, name } => {
                let run = Callback::new(move |(id, name, action, close): (String, String, ActionState, Callback<()>)| {
                    mutate::project_delete(id, name, action, close);
                });
                view! {
                    <DeleteModal
                        id=id
                        name=name
                        what="project"
                        why="Its decisions and recorded sessions go with it. This cannot be undone."
                        run=run
                    />
                }
                    .into_any()
            }
            ModalKind::DeleteGroup { id, name } => {
                let run = Callback::new(move |(id, name, action, close): (String, String, ActionState, Callback<()>)| {
                    mutate::group_delete(id, name, action, close);
                });
                view! {
                    <DeleteModal
                        id=id
                        name=name
                        what="group"
                        why="Every project in it — decisions, sessions, membership — goes with it. This cannot be undone."
                        run=run
                    />
                }
                    .into_any()
            }
        })
    }
}

/// The shared destruction ceremony: retype the name, then the danger
/// button arms. Keep the confirmation and its input until the API succeeds;
/// a refusal is shown inside the modal, and navigation follows success.
#[component]
fn DeleteModal(
    id: String,
    name: String,
    what: &'static str,
    why: &'static str,
    run: Callback<(String, String, ActionState, Callback<()>)>,
) -> impl IntoView {
    let modal = use_modal();
    let action = ActionState::new();
    let (typed, set_typed) = signal(String::new());
    let input_ref = NodeRef::<html::Input>::new();
    autofocus(input_ref);
    let expected = name.clone();
    let armed = Signal::derive(move || typed.get().trim() == expected);
    let submit = {
        let id = id.clone();
        let name = name.clone();
        Callback::new(move |()| {
            if !armed.get_untracked() || action.pending.get_untracked() {
                return;
            }
            run.run((
                id.clone(),
                name.clone(),
                action,
                Callback::new(move |()| modal.set(None)),
            ));
        })
    };
    view! {
        <Modal
            title=format!("Delete {what} “{name}”?")
            subtitle=why
            on_close=Callback::new(move |()| modal.set(None))
        >
            <div class="cv-col cv-gap-6">
                <span class="cv-modal__label">{format!("Type the {what}'s name to confirm")}</span>
                <div class="cv-input">
                    <input
                        node_ref=input_ref
                        class="cv-input__field cv-mono"
                        placeholder=name.clone()
                        disabled=action.pending
                        prop:value=typed
                        on:input=move |ev| set_typed.set(event_target_value(&ev))
                        on:keydown=move |ev| {
                            if ev.key() == "Enter" {
                                ev.prevent_default();
                                submit.run(());
                            }
                        }
                    />
                </div>
            </div>
            <ActionStatus state=action pending_text="Working…" />
            <div class="cv-modal__foot">
                <Button
                    label="Cancel"
                    variant=ButtonVariant::Ghost
                    on_click=Callback::new(move |()| modal.set(None))
                />
                <Button
                    label=format!("Delete {what}")
                    tone=Tone::Danger
                    disabled=Signal::derive(move || action.pending.get() || !armed.get())
                    on_click=submit
                />
            </div>
        </Modal>
    }
}

#[component]
fn NewGroupModal() -> impl IntoView {
    let modal = use_modal();
    let action = ActionState::new();
    let (name, set_name) = signal(String::new());
    let (kind, set_kind) = signal(GroupKind::Shared);
    let name_ref = NodeRef::<html::Input>::new();
    autofocus(name_ref);

    let close = Callback::new(move |()| modal.set(None));
    let submit = Callback::new(move |()| {
        let n = name.get_untracked().trim().to_string();
        if n.is_empty() {
            return;
        }
        mutate::create_group(n, kind.get_untracked(), action, close);
    });

    view! {
        <Modal
            title="New group"
            subtitle="Shared memory across services — or a personal space."
            on_close=close
        >
            <div class="cv-input">
                <input
                    node_ref=name_ref
                    class="cv-input__field"
                    placeholder="platform-team"
                    disabled=action.pending
                    prop:value=name
                    on:input=move |ev| set_name.set(event_target_value(&ev))
                    on:keydown=move |ev| match ev.key().as_str() {
                        "Enter" => {
                            ev.prevent_default();
                            submit.run(());
                        }
                        "Escape" => close.run(()),
                        _ => {}
                    }
                />
            </div>
            <div class="cv-row cv-gap-7">
                <span
                    class=move || {
                        if kind.get() == GroupKind::Shared {
                            "cv-projchip cv-projchip--on"
                        } else {
                            "cv-projchip"
                        }
                    }
                    on:click=move |_| { if !action.pending.get_untracked() { set_kind.set(GroupKind::Shared); } }
                >
                    {format!("{} shared", Glyph::Shared.glyph())}
                </span>
                <span
                    class=move || {
                        if kind.get() == GroupKind::Personal {
                            "cv-projchip cv-projchip--on"
                        } else {
                            "cv-projchip"
                        }
                    }
                    on:click=move |_| { if !action.pending.get_untracked() { set_kind.set(GroupKind::Personal); } }
                >
                    {format!("{} personal", Glyph::Personal.glyph())}
                </span>
            </div>
            <ActionStatus state=action pending_text="Working…" />
            <div class="cv-modal__foot">
                <Button label="Cancel" variant=ButtonVariant::Ghost on_click=close />
                <Button
                    label="Create group"
                    tone=Tone::Primary
                    disabled=Signal::derive(move || action.pending.get() || name.get().trim().is_empty())
                    on_click=submit
                />
            </div>
        </Modal>
    }
}

#[component]
fn NewProjectModal() -> impl IntoView {
    let modal = use_modal();
    let action = ActionState::new();
    let (name, set_name) = signal(String::new());
    let name_ref = NodeRef::<html::Input>::new();
    autofocus(name_ref);

    let close = Callback::new(move |()| modal.set(None));
    let submit = Callback::new(move |()| {
        let n = name.get_untracked().trim().to_string();
        if n.is_empty() {
            return;
        }
        mutate::create_project(n, action, close);
    });

    view! {
        <Modal
            title="New project"
            subtitle=format!("Its decision log starts empty in {}.", data::group_name())
            on_close=close
        >
            <div class="cv-input">
                <input
                    node_ref=name_ref
                    class="cv-input__field"
                    placeholder="api-gateway"
                    disabled=action.pending
                    prop:value=name
                    on:input=move |ev| set_name.set(event_target_value(&ev))
                    on:keydown=move |ev| match ev.key().as_str() {
                        "Enter" => {
                            ev.prevent_default();
                            submit.run(());
                        }
                        "Escape" => close.run(()),
                        _ => {}
                    }
                />
            </div>
            <ActionStatus state=action pending_text="Working…" />
            <div class="cv-modal__foot">
                <Button label="Cancel" variant=ButtonVariant::Ghost on_click=close />
                <Button
                    label="Create project"
                    tone=Tone::Primary
                    disabled=Signal::derive(move || action.pending.get() || name.get().trim().is_empty())
                    on_click=submit
                />
            </div>
        </Modal>
    }
}
