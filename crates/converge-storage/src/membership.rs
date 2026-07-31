//! Membership — who can see a group.
//!
//! The ACL model: every group has exactly one owner (a column on the
//! group — structural, not a role row) and flat members; visibility is
//! `owner OR member`, and everything inside a group (projects,
//! decisions, sessions, signals) inherits it. Members read and write;
//! only the owner manages membership. Members are added directly, by
//! existing user — signing in is what creates a user, so "sign in
//! first, then be added" is the whole onboarding story. Pending
//! invitations for people who have never signed in are a future SaaS
//! onboarding step (alongside owner transfer), not v1.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{GroupId, UserId};
use crate::{Scope, StoreError};

/// A group member, joined with display identity for listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: UserId,
    pub handle: String,
    pub name: String,
    /// Who brought them in (the owner, today).
    pub invited_by: UserId,
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
}

/// Storage operations on memberships.
///
/// Owner-only operations enforce against `scope`: a caller who cannot
/// see the group gets `NotFound`; one who sees it but lacks the right
/// gets `Invalid`.
pub trait Memberships {
    /// Add a member (owner-only; `System` for bootstrap). Idempotent:
    /// re-adding is a no-op. Adding the owner is `Invalid` — ownership
    /// already contains membership.
    fn member_add(
        &self,
        scope: Scope,
        group: GroupId,
        user: UserId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The group's members (the owner is implicit and NOT listed here),
    /// join-ordered. Visible to any member.
    fn member_list(
        &self,
        scope: Scope,
        group: GroupId,
    ) -> impl Future<Output = Result<Vec<Member>, StoreError>> + Send;

    /// Remove a member: the owner removes anyone, a member removes
    /// themself (leaving). The owner has no membership row, so removing
    /// them is `NotFound` by construction — transfer is a future
    /// operation.
    fn member_remove(
        &self,
        scope: Scope,
        group: GroupId,
        user: UserId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Adopt ownerless groups (the pre-ACL backfill): every group with
    /// no owner becomes `owner`'s. Runs at boot with the deployment
    /// user; idempotent. Returns how many were adopted.
    fn adopt(&self, owner: UserId) -> impl Future<Output = Result<u64, StoreError>> + Send;
}
