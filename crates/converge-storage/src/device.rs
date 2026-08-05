//! Device-authorization grants (RFC 8628) — the CLI pairing rendezvous.
//!
//! The one deliberately *stateful* corner of the OAuth surface: a pending
//! grant is a meeting point between a polling client (holding the
//! high-entropy `device_code`, stored hashed like every bearer) and an
//! authenticated browser (holding the short human `user_code`). Rows are
//! ephemeral — approved, denied, or expired, they are consumed on claim.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::StoreError;
use crate::ids::UserId;

/// A pending grant, as the approval screen sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGrant {
    /// The human-facing code (`XXXX-XXXX`), stored normalized.
    pub user_code: String,
    /// The requesting client's display name ("converge-cli @ host").
    pub client_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// The fields required to open a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDeviceGrant {
    /// SHA-256 of the `device_code` the client polls with.
    pub device_hash: String,
    /// SHA-256 of the `client_id` — the poll must come from the same
    /// client that opened the grant.
    pub client_hash: String,
    pub user_code: String,
    pub client_name: String,
    pub expires_at: OffsetDateTime,
}

/// What a poll finds. Every terminal state consumes the row — a device
/// code never yields twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClaim {
    /// Still waiting for the browser.
    Pending,
    /// Approved by this user; the row is gone.
    Approved(UserId),
    /// Denied; the row is gone.
    Denied,
    /// Expired or never existed (indistinguishable on purpose).
    Gone,
}

/// Storage operations on device grants. All `Scope`-free: the rows are
/// pre-authentication protocol state, keyed by unguessable hashes.
pub trait Devices {
    /// Open a grant. A colliding `user_code` is `Conflict` — the caller
    /// regenerates and retries.
    fn device_start(
        &self,
        new: NewDeviceGrant,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// A pending, unexpired grant by user code — the approval screen's
    /// read. Terminal or expired grants read as absent.
    fn device_get(
        &self,
        user_code: &str,
    ) -> impl Future<Output = Result<Option<DeviceGrant>, StoreError>> + Send;

    /// The browser's verdict. Only a pending, unexpired grant can be
    /// decided — anything else is `NotFound`.
    fn device_decide(
        &self,
        user_code: &str,
        user: UserId,
        approve: bool,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The client's poll: pending grants stay, terminal ones are
    /// consumed (deleted) as they are reported. A wrong `client_hash`
    /// reads as `Gone` — a device code is bound to its opener.
    fn device_claim(
        &self,
        device_hash: &str,
        client_hash: &str,
    ) -> impl Future<Output = Result<DeviceClaim, StoreError>> + Send;
}
