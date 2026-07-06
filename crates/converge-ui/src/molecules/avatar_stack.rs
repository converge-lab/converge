use crate::atoms::Avatar;
use crate::domain::Author;
use leptos::prelude::*;

/// Overlapping avatars with a `+N` overflow chip. `Avatar` × overflow.
/// Colours are derived per author by the library, not passed in.
#[component]
pub fn AvatarStack(
    authors: Vec<Author>,
    #[prop(default = 24)] size: u32,
    #[prop(default = 3)] max: usize,
) -> impl IntoView {
    let total = authors.len();
    let shown: Vec<Author> = authors.into_iter().take(max).collect();
    let overflow = total.saturating_sub(max);
    let overlap = (size as f32 * 0.28) as i32;
    view! {
        <div class="cv-avatarstack">
            {shown
                .into_iter()
                .enumerate()
                .map(move |(i, a)| {
                    let color = a.color();
                    let ml = if i == 0 {
                        "0".to_string()
                    } else {
                        format!("-{}rem", overlap as f32 / 16.0)
                    };
                    view! {
                        <span style=format!("margin-left:{ml}")>
                            <Avatar initial=a.initial color=color size=size ring=true />
                        </span>
                    }
                })
                .collect_view()}
            {(overflow > 0)
                .then(move || {
                    view! {
                        <span
                            class="cv-avatar cv-avatar--ring cv-avatar--more"
                            style=format!(
                                "--cv-avatar-size:{}rem;margin-left:-{}rem",
                                size as f64 / 16.0,
                                overlap as f32 / 16.0,
                            )
                        >
                            {format!("+{overflow}")}
                        </span>
                    }
                })}
        </div>
    }
}
