//! The NostDB GitHub provider.
//!
//! An out-of-process executable that answers the versioned provider protocol: resolve a
//! `github://` locator to an immutable commit, enumerate what that snapshot contains, and
//! read one entry's bytes.
//!
//! # What this crate does not do
//!
//! It does not interpret what it retrieves. A provider returns bytes and metadata; only
//! `nostdb-core` reads `.nost` or `.nostdb`. This crate therefore does not depend on the
//! Engine, and must not: linking it would make parsing either format possible by accident,
//! and a second parser of a format that has exactly one is how two answers to one question
//! start.
//!
//! It also does not decide what is interesting. It has no view on which files are worth
//! analyzing, no analyzer, and no graph.
//!
//! # Why it is a separate process
//!
//! Not for performance. This is the component that holds a credential and talks to a
//! network, and the one most likely to be replaced by something a third party wrote. Behind
//! a process boundary it cannot read the Engine's memory, cannot reach a database handle,
//! and cannot outlive the request it was started for.
//!
//! # Scaffolding
//!
//! This crate is scaffolding. The protocol it will speak is specified in `nostdb-spec` at
//! `docs/PROVIDER_PROTOCOL.md`, and the root `IMPLEMENTATION_PROGRESS.md` records which
//! increment builds which part of it.

#![forbid(unsafe_code)]

pub mod api;
pub mod http;
pub mod locator;
pub mod serve;

/// The provider protocol version this build speaks.
///
/// Stated here rather than inferred, so a build that has not implemented a version cannot
/// claim it by being compiled against the contract that describes it.
pub const PROVIDER_PROTOCOL_VERSION: u32 = 1;

/// The name this provider reports in a handshake.
pub const PROVIDER_NAME: &str = "github";

/// The locator scheme this provider reads.
pub const LOCATOR_SCHEME: &str = "github://";

/// A refusal this provider can reply with.
///
/// Declared here even though no request is served yet. These codes are part of the protocol
/// this crate speaks, so the crate owning them is a fact about the contract rather than
/// about how much of it is built — and the workspace verifier checks that the registry's
/// owner and the source that declares them agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderCode {
    /// The request's protocol version is not implemented.
    ProtocolUnsupported,
    /// The request is malformed or names a kind this version does not define.
    RequestInvalid,
    /// The locator is not one this provider reads.
    LocatorInvalid,
    /// The source needs a credential that was not supplied.
    ///
    /// Never downgraded to an anonymous request: that turns a permissions problem into a
    /// "repository not found" and sends whoever hits it looking in the wrong place.
    CredentialRequired,
    /// The host refused the credential.
    CredentialRejected,
    /// The host could not be reached, or has no such snapshot.
    ///
    /// Not a build failure. The link remains declared, and a query returns what it can
    /// reach.
    SourceUnavailable,
    /// A host quota or rate limit was reached.
    LimitExceeded,
}

impl ProviderCode {
    /// Every code, so a test can walk them.
    pub const ALL: [Self; 7] = [
        Self::ProtocolUnsupported,
        Self::RequestInvalid,
        Self::LocatorInvalid,
        Self::CredentialRequired,
        Self::CredentialRejected,
        Self::SourceUnavailable,
        Self::LimitExceeded,
    ];

    /// The symbolic name a reply carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProtocolUnsupported => "PROVIDER_PROTOCOL_UNSUPPORTED",
            Self::RequestInvalid => "PROVIDER_REQUEST_INVALID",
            Self::LocatorInvalid => "PROVIDER_LOCATOR_INVALID",
            Self::CredentialRequired => "PROVIDER_CREDENTIAL_REQUIRED",
            Self::CredentialRejected => "PROVIDER_CREDENTIAL_REJECTED",
            Self::SourceUnavailable => "PROVIDER_SOURCE_UNAVAILABLE",
            Self::LimitExceeded => "PROVIDER_LIMIT_EXCEEDED",
        }
    }
}

impl std::fmt::Display for ProviderCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declared_protocol_version_is_one_the_specification_defines() {
        // A guard against the version drifting ahead of what this build implements. The
        // specification is the authority; this constant only says which of its versions is
        // spoken here.
        assert_eq!(PROVIDER_PROTOCOL_VERSION, 1);
    }

    #[test]
    fn every_code_is_distinct_and_carries_the_registry_prefix() {
        let names: std::collections::BTreeSet<&str> =
            ProviderCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(names.len(), ProviderCode::ALL.len());
        assert!(names.iter().all(|name| name.starts_with("PROVIDER_")));
    }

    #[test]
    fn the_scheme_is_the_canonical_one() {
        assert!(LOCATOR_SCHEME.starts_with(PROVIDER_NAME));
        assert!(LOCATOR_SCHEME.ends_with("://"));
    }
}
