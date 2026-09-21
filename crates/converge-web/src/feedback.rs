//! User-facing feedback only. Browser diagnostics are deliberately disabled
//! until a collection backend and its privacy policy have been agreed. Do not
//! send errors to the console or attach raw errors to rendered messages.

#[cfg(feature = "api")]
use converge_client::StoreError;
use leptos::prelude::*;

use crate::store::{AppStore, NoticeOrigin, push_failure, use_store};

/// State owned by the form or section that starts an operation. If that view
/// disappears before a failure arrives, the shell retains the explanation.
#[derive(Clone, Copy)]
pub struct ActionState {
    pub pending: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    store: AppStore,
}

impl ActionState {
    pub fn new() -> Self {
        Self {
            pending: RwSignal::new(false),
            error: RwSignal::new(None),
            store: use_store(),
        }
    }

    /// Ignore repeated submissions until the first request has an outcome.
    pub fn begin(self) -> bool {
        if self.pending.try_get_untracked() != Some(false) {
            return false;
        }
        self.error.set(None);
        self.pending.set(true);
        true
    }

    /// Returns whether the original view is still mounted.
    pub fn finish(self) -> bool {
        self.pending.try_set(false).is_none()
    }

    pub fn fail_message(self, message: String) {
        self.finish();
        if let Some(Some(message)) = self.error.try_set(Some(message)) {
            push_failure(self.store, NoticeOrigin::Action(self.pending), message);
        }
    }

    #[cfg(feature = "api")]
    pub fn fail(self, action: &'static str, error: &StoreError) {
        self.fail_message(message(action, error));
    }
}

/// Keep feedback adjacent to its action and announce changes to assistive
/// technology. The error remains until another attempt or view dismissal.
#[component]
pub fn ActionStatus(state: ActionState, pending_text: &'static str) -> impl IntoView {
    view! {
        {move || state.pending.get().then(|| view! {
            <p role="status" class="cv-fs-sm cv-fg-muted">{pending_text}</p>
        })}
        <div role="alert" class="cv-fs-sm cv-fg-danger">
            {move || state.error.get()}
        </div>
    }
}

/// Format an explanation without logging or otherwise reporting the error.
#[cfg(feature = "api")]
pub fn message(action: &'static str, error: &StoreError) -> String {
    format!("{action} — {}", explanation(error))
}

/// A failure that must survive navigation or the form being dismissed.
#[cfg(feature = "api")]
pub fn notify(store: AppStore, action: &'static str, resource: &str, error: &StoreError) {
    push_failure(
        store,
        NoticeOrigin::Resource(action, resource.into()),
        message(action, error),
    );
}

#[cfg(feature = "api")]
fn explanation(error: &StoreError) -> &str {
    match error {
        StoreError::Invalid(message) | StoreError::Conflict(message) => message,
        StoreError::Unauthorized => "Your session has expired. Sign in again and retry.",
        StoreError::NotFound => "This item no longer exists or you no longer have access to it.",
        StoreError::Unavailable(_) => {
            "Couldn't confirm the result. Check your connection and try again."
        }
        StoreError::Backend(_) => "Something went wrong. Please try again.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AppState, AppStateStoreFields, Notice, dismiss_notice, push_notice};

    #[test]
    fn failures_survive_unrelated_success_and_individual_dismissals() {
        let owner = Owner::new();
        owner.with(|| {
            let store = AppStore::new_local(AppState::default());
            push_notice(store, Notice::Failed("First failure".into()));
            push_notice(store, Notice::Ok("Unrelated success".into()));
            push_notice(store, Notice::Failed("Second failure".into()));
            let notices = store.notices().get_untracked();
            dismiss_notice(store, notices[1].id);
            assert_eq!(
                store.notices().get_untracked(),
                vec![notices[0].clone(), notices[2].clone()]
            );
            dismiss_notice(store, notices[0].id);
            // Replaying an older timer must not dismiss a later receipt.
            push_notice(store, Notice::Ok("Later success".into()));
            dismiss_notice(store, notices[1].id);
            assert_eq!(store.notices().get_untracked().len(), 2);
        });
        owner.cleanup();
    }

    #[test]
    fn failures_after_form_disposal_move_to_the_shell() {
        let owner = Owner::new();
        owner.with(|| {
            let store = AppStore::new_local(AppState::default());
            provide_context(store);
            let form_owner = Owner::new();
            let action = form_owner.with(ActionState::new);
            assert!(action.begin());
            assert!(!action.begin());
            form_owner.cleanup();
            action.fail_message("Couldn't save the form".into());
            let notices = store.notices().get_untracked();
            assert_eq!(notices.len(), 1);
            assert_eq!(
                notices[0].notice,
                Notice::Failed("Couldn't save the form".into())
            );
        });
        owner.cleanup();
    }

    #[test]
    fn repeated_failures_are_counted_without_merging_different_resources() {
        let owner = Owner::new();
        owner.with(|| {
            let store = AppStore::new_local(AppState::default());
            let key = |id: &str| NoticeOrigin::Resource("delete project", id.into());
            for _ in 0..5 {
                push_failure(store, key("project-a"), "Couldn't delete project".into());
            }
            push_failure(store, key("project-b"), "Couldn't delete project".into());
            let notices = store.notices().get_untracked();
            assert_eq!(notices.len(), 2);
            assert_eq!(notices[0].occurrences, 5);
            assert_eq!(notices[1].occurrences, 1);
            push_failure(store, key("project-a"), "Couldn't delete project".into());
            let reordered = store.notices().get_untracked();
            assert_eq!(reordered[1].id, notices[0].id);
            assert_eq!(reordered[1].occurrences, 6);
            dismiss_notice(store, notices[0].id);
            push_failure(store, key("project-a"), "Couldn't delete project".into());
            let next = store.notices().get_untracked();
            assert_eq!(next[1].occurrences, 1);
            assert_ne!(next[1].id, notices[0].id);
        });
        owner.cleanup();
    }

    #[cfg(feature = "api")]
    #[test]
    fn domain_errors_explain_what_the_user_can_correct() {
        let reason = "No user with that handle. They need to sign in first.";
        assert_eq!(explanation(&StoreError::Invalid(reason.into())), reason);
        assert!(explanation(&StoreError::Unauthorized).contains("Sign in again"));
    }

    #[cfg(feature = "api")]
    #[test]
    fn operational_details_never_reach_the_ui() {
        for error in [
            StoreError::Backend("SQL with private content".into()),
            StoreError::Unavailable("https://host/?token=secret".into()),
        ] {
            let text = message("Couldn't load members", &error);
            assert!(!text.contains("private"));
            assert!(!text.contains("secret"));
            assert!(!text.contains("SQL"));
        }
    }
}
