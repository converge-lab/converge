//! The onboarding empty-group state — a *state of the dashboard*, shown when
//! the current group has no projects yet. Offers three ways forward: a new
//! group, a new project, or connecting an agent over MCP.

use converge_ui::atoms::{Glyph, Logo, LogoVariant};
use converge_ui::molecules::OnboardCard;
use leptos::prelude::*;

use crate::command_snippet::{CommandSnippet, mcp_command};
use crate::data;
use crate::modals::{ModalKind, open};

#[component]
pub fn Onboarding() -> impl IntoView {
    // Zero groups (a fresh user or deployment) vs an empty group: the
    // same guide, but "new project" only makes sense inside a group.
    let fresh = data::groups().is_empty();
    let group = data::group_name();
    let (title, sub) = if fresh {
        (
            "Welcome to converge",
            "Decision memory starts with a group — a team's shared space. \
             Create one, or have your agent do it."
                .to_string(),
        )
    } else {
        (
            "Nothing here yet",
            format!(
                "{group} is empty. Create a home for your team's decisions — or have your agent do it.",
            ),
        )
    };
    view! {
        <div class="cv-onboard-wrap">
            <div class="cv-onboard">
                <Logo variant=LogoVariant::Mark class="cv-onboard__mark" />
                <h1 class="cv-heading cv-fs-3xl">{title}</h1>
                <p class="cv-onboard__sub">{sub}</p>
                <div class="cv-onboard__cards">
                    <OnboardCard
                        glyph=Glyph::Shared
                        title="New group"
                        desc="Shared decision memory for a team of services."
                        on_click=Callback::new(|_| open(ModalKind::NewGroup))
                    />
                    {(!fresh)
                        .then(|| {
                            view! {
                                <OnboardCard
                                    glyph=Glyph::Dashboard
                                    title="New project"
                                    desc=format!("A single service's decision log, in {group}.")
                                    on_click=Callback::new(|_| open(ModalKind::NewProject))
                                />
                            }
                        })}
                </div>
                <div class="cv-onboard__or">"or connect your agent"</div>
                <div class="cv-onboard__agent">
                    <CommandSnippet command=mcp_command() />
                    <div class="cv-onboard__ask">
                        "Then ask for what you need in plain words — "
                        <em>
                            "\u{201C}create a group for the platform team, add api-gateway, and record our first decision.\u{201D}"
                        </em>
                        " Groups, projects and decisions appear here as the agent writes them."
                    </div>
                </div>
            </div>
        </div>
    }
}
