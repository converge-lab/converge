//! `#/project/{id}/settings` — the same shape as the group's settings, so the
//! structure is learned once: form on top, destructive actions fenced at the
//! bottom.
//!
//! Name, description and whether whole conversations are kept are real edits
//! (`PATCH /projects/{id}`). Group (moving a project) and retiring a project
//! are drawn but inert — nothing behind either exists yet.

use converge_ui::atoms::{Button, ButtonVariant, Select};
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
    let repository = data::proj_repository(&pid);
    let archives = data::proj_archives(&pid);

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
                {match repository {
                    Some(repo) => view! { " Code at " <span class="cv-mono">{repo}</span> "." }.into_any(),
                    None => view! { " No repository recorded yet — the first bind from a working tree sets it." }.into_any(),
                }}
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
                    <span class="cv-modal__label">"Conversations"</span>
                    // A project-wide call, not a per-machine setting: the
                    // conversations belong to everyone working here. Saved on
                    // change — a policy that needs a second click to take is
                    // a policy someone will leave half-set. The shown value
                    // is the dataset's, which moves only once the server has
                    // agreed; the atom marks the option rather than setting
                    // `value` on the select, which would render blank.
                    <Select
                        options=vec![
                            ("all".to_string(), "Keep the whole conversation".to_string()),
                            ("cited".to_string(), "Keep only the lines decisions cite".to_string()),
                        ]
                        value=(if archives { "all" } else { "cited" }).to_string()
                        on_change={
                            let pid = pid.clone();
                            Callback::new(move |v: String| {
                                mutate::set_project_archives(pid.clone(), v == "all");
                            })
                        }
                    />
                    <span class="cv-setform__hint">
                        "Whole transcripts are kept so they can be read and analysed later. \
                         Turn that off and agents still record the exact lines a decision \
                         cites — the evidence — and nothing else."
                    </span>
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
