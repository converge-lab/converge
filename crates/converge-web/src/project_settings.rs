//! `#/project/{id}/settings` — the same shape as the group's settings, so the
//! structure is learned once: form on top, destructive actions fenced at the
//! bottom.
//!
//! Name and description are real edits (`PATCH /projects/{id}`). Group (moving
//! a project) and archive/delete are drawn but inert — `ProjectEdit` carries
//! only name and description, and nothing behind the other two exists yet.

use converge_ui::atoms::{Button, ButtonVariant};
use converge_ui::domain::Tone;
use leptos::prelude::*;

use crate::modals::{self, ModalKind};
use crate::{data, mutate};

#[component]
pub fn ProjectSettings(pid: String) -> impl IntoView {
    let (name, set_name) = signal(data::proj_name(&pid));
    let (desc, set_desc) = signal(data::proj_desc(&pid));
    let (flash, set_flash) = signal(None::<String>);

    let decisions = data::project_decisions(&pid).len();
    // The project's owning group — not the active one; a deep link can open
    // a project from a group that isn't currently selected.
    let group = data::proj_group_name(&pid);
    let title = data::proj_name(&pid);

    let save = {
        let pid = pid.clone();
        Callback::new(move |()| {
            let n = name.get_untracked().trim().to_string();
            if n.is_empty() {
                return;
            }
            // As on the group's settings: the write re-creates this screen, so
            // a local confirmation line would never survive to be read.
            mutate::edit_project(pid.clone(), n, desc.get_untracked());
        })
    };

    view! {
        <div class="cv-page cv-page--form">
            <h1 class="cv-heading cv-fs-4xl cv-mb-6">"Project settings"</h1>
            <p class="cv-fg-muted cv-fs-md cv-mb-22">
                "Settings for " <span class="cv-mono">{title.clone()}</span>
                {format!(
                    " — {decisions} {} in {group}.",
                    if decisions == 1 { "decision" } else { "decisions" },
                )}
            </p>

            <div class="cv-setform">
                <div class="cv-col cv-gap-6">
                    <span class="cv-modal__label">"Name"</span>
                    <div class="cv-input">
                        <input
                            class="cv-input__field cv-mono"
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
                        "Lowercase and hyphens — this is the name agents address the project by."
                    </span>
                </div>

                <div class="cv-col cv-gap-6">
                    <span class="cv-modal__label">"Description"</span>
                    <div class="cv-input">
                        <input
                            class="cv-input__field"
                            placeholder="What this service is responsible for — one line."
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
                    <span class="cv-modal__label">"Group"</span>
                    // Inert: a project's group is fixed today — the API has no
                    // move. Shown anyway, because "which group is this in?" is
                    // a question the settings screen should answer.
                    <div class="cv-setform__locked">
                        <select class="cv-select cv-w-full">
                            <option>{group.clone()}</option>
                        </select>
                    </div>
                    <span class="cv-setform__hint">
                        "Moving a project to another group isn't possible yet; its decisions would travel with it."
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

            <div class="cv-danger">
                <div class="cv-danger__title">"Danger zone"</div>
                <div class="cv-danger__row">
                    <div class="cv-grow">
                        <div class="cv-danger__what">"Archive project"</div>
                        <div class="cv-danger__why">
                            "Leaves the sidebar. Its decisions stay reachable by link and in search."
                        </div>
                    </div>
                    <Button
                        label="Archive"
                        variant=ButtonVariant::Outline
                        on_click=Callback::new(move |()| {
                            set_flash.set(Some("Archiving a project isn't wired up yet.".into()))
                        })
                    />
                </div>
                <div class="cv-danger__row">
                    <div class="cv-grow">
                        <div class="cv-danger__what">"Delete project"</div>
                        <div class="cv-danger__why">
                            {format!(
                                "Removes {decisions} {}. This cannot be undone.",
                                if decisions == 1 { "decision" } else { "decisions" },
                            )}
                        </div>
                    </div>
                    <Button
                        label="Delete"
                        variant=ButtonVariant::Outline
                        tone=Tone::Danger
                        on_click={
                            let pid = pid.clone();
                            let name = title.clone();
                            Callback::new(move |()| {
                                modals::open(ModalKind::DeleteProject {
                                    id: pid.clone(),
                                    name: name.clone(),
                                })
                            })
                        }
                    />
                </div>
            </div>

            {move || flash.get().map(|msg| view! { <div class="cv-flash">{msg}</div> })}
        </div>
    }
}
