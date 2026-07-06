use crate::atoms::Glyph;
use crate::domain::ChatRole;
use leptos::prelude::*;

/// An Expert-chat message. User messages sit right; expert messages sit left
/// behind a ✦ avatar and may carry the decisions forwarded to the local agent.
#[component]
pub fn ChatBubble(
    role: ChatRole,
    #[prop(into)] text: String,
    /// (project, title) of each forwarded decision.
    #[prop(optional)]
    forwarded: Vec<(String, String)>,
) -> impl IntoView {
    match role {
        ChatRole::User => view! {
            <div class="cv-bubble--user-row">
                <div class="cv-bubble__user">{text}</div>
            </div>
        }
        .into_any(),
        ChatRole::Expert => {
            let has_fwd = !forwarded.is_empty();
            view! {
                <div class="cv-bubble--expert-row">
                    <div class="cv-bubble__avatar">{Glyph::Expert.glyph()}</div>
                    <div class="cv-grow">
                        <div class="cv-bubble__expert">{text}</div>
                        {has_fwd
                            .then(move || {
                                view! {
                                    <div class="cv-fwd">
                                        <div class="cv-fwd__label">"→ forwarded to local agent"</div>
                                        <div class="cv-fwd__list">
                                            {forwarded
                                                .into_iter()
                                                .map(|(proj, title)| {
                                                    view! {
                                                        <div class="cv-fwd__card">
                                                            <span class="cv-fwd__proj cv-mono">{proj}</span>
                                                            <span class="cv-fwd__title">{title}</span>
                                                        </div>
                                                    }
                                                })
                                                .collect_view()}
                                        </div>
                                    </div>
                                }
                            })}
                    </div>
                </div>
            }
            .into_any()
        }
    }
}
