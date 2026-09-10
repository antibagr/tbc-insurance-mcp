//! A bounded, local-only vault for one opaque TBC session record.
//!
//! The vault has no network or insurer operations. Callers hold their existing
//! cross-process workflow lock and create one client per lock guard. A failed or
//! cancelled request permanently invalidates that client; obsolete writes must
//! never be retried or rebased. Production peers require a compile-time signing
//! certificate pin and fixed application identifiers.
//!
//! Keep dependency tracing disabled: the XPC dependency contains payload-level
//! trace events. Its native decoder allocates before this crate checks frame
//! lengths, so the hard protocol bounds apply to cooperating pinned peers.
//! Returned records and owned protocol buffers are zeroized; native XPC copies
//! and the dependency's temporary message buffers have their own lifetimes.

use std::fmt;
mod client;
#[cfg(target_os = "macos")]
mod native;
mod protocol;
mod state;

pub use client::Client;
#[cfg(target_os = "macos")]
pub use native::{authorize_session, run};

/// Maximum opaque credential-record length, in bytes.
pub const MAX_RECORD_BYTES: usize = 20_480;

/// A sanitized vault failure. Formatting never includes credential data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultError {
    /// This platform or signing configuration cannot use the vault.
    Unavailable,
    /// The bounded wire protocol was violated.
    InvalidRequest,
    /// The previously loaded record or broker generation changed.
    Conflict,
    /// The local credential operation failed.
    Storage,
    /// A request failed or timed out; its write outcome may be unknown.
    Transport,
    /// This client cannot be reused after an interrupted or failed operation.
    Invalidated,
}

impl fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "vault_unavailable",
            Self::InvalidRequest => "vault_invalid_request",
            Self::Conflict => "vault_conflict",
            Self::Storage => "vault_storage_failed",
            Self::Transport => "vault_outcome_unknown",
            Self::Invalidated => "vault_client_invalidated",
        })
    }
}

impl std::error::Error for VaultError {}
