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

/// The provider protocol version this build speaks.
///
/// Stated here rather than inferred, so a build that has not implemented a version cannot
/// claim it by being compiled against the contract that describes it.
pub const PROVIDER_PROTOCOL_VERSION: u32 = 1;

/// The name this provider reports in a handshake.
pub const PROVIDER_NAME: &str = "github";

/// The locator scheme this provider reads.
pub const LOCATOR_SCHEME: &str = "github://";

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
    fn the_scheme_is_the_canonical_one() {
        assert!(LOCATOR_SCHEME.starts_with(PROVIDER_NAME));
        assert!(LOCATOR_SCHEME.ends_with("://"));
    }
}
