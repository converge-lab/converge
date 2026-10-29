//! The composition seam for distributions — the web mirror of the
//! server's `Access`: this crate defines the slots, and never knows who
//! fills them (the dependency is strictly one-way). A distribution
//! composes the app by passing its [`Extension`]s to [`crate::run`];
//! the stock binary passes none.
//!
//! Deliberately tiny: a hash-prefix worth of screens and one
//! account-menu entry per extension. Growth pressure on this surface is
//! a design smell — push back before widening it.

use leptos::prelude::*;

/// One extension: a screen family under its own hash prefix, plus the
/// account-menu entry that opens it.
#[derive(Clone, Copy)]
pub struct Extension {
    /// The first hash segment this extension owns (`"admin"` → `#/admin/…`).
    pub prefix: &'static str,
    /// The account-menu entry label.
    pub label: &'static str,
    /// The breadcrumb shown over its screens.
    pub crumb: &'static str,
    /// Render the screen for `#/{prefix}/{rest}` (`rest` may be empty).
    pub screen: fn(String) -> AnyView,
}

/// The composed set, published to context by [`crate::run`].
#[derive(Clone, Default)]
pub struct Extensions(pub Vec<Extension>);

impl Extensions {
    /// The extension owning `path` (a hash path without `#/`), with the
    /// remainder after its prefix.
    pub(crate) fn find(&self, path: &str) -> Option<(Extension, String)> {
        let (prefix, rest) = path.split_once('/').unwrap_or((path, ""));
        self.0
            .iter()
            .find(|e| e.prefix == prefix)
            .map(|e| (*e, rest.to_string()))
    }
}

/// The set the app was composed with (empty when none were passed).
pub(crate) fn extensions() -> Extensions {
    use_context::<Extensions>().unwrap_or_default()
}
