//! `#/members` — who can read and write the active group's decision memory.
//!
//! Reached from the dashboard's "⋯" menu. The roster and adding a teammate
//! land here next; the server already carries both (membership is owner-only,
//! and a member is added by handle — signing in is what creates a user, so
//! there is no pending-invite state to model).

use converge_ui::atoms::{Callout, Glyph};
use converge_ui::domain::Tone;
use leptos::prelude::*;

use crate::data;

#[component]
pub fn Members() -> impl IntoView {
    let group = data::group_name();
    view! {
        <div class="cv-page">
            <div class="cv-settings">
                <h1 class="cv-heading cv-fs-3xl cv-mb-22">"Members"</h1>
                <p class="cv-settings__desc">
                    "Who can read and write " <b>{group}</b>
                    "'s decision memory. Everything recorded in the group — decisions, their
                     sources, the signals between them — is visible to every member."
                </p>
                <Callout tone=Tone::Neutral icon=Glyph::Shared title="Adding a teammate">
                    "There are no invite emails to chase: signing in is what creates a user, so
                     the group's owner adds someone by their handle once they've signed in. The
                     roster and that form arrive on this screen next."
                </Callout>
            </div>
        </div>
    }
}
