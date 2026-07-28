//! The provider executable.
//!
//! Reads one request per line from standard input and writes one reply per line to standard
//! output. Standard error carries diagnostics and never a reply, so a caller reading replies
//! cannot be confused by one.
//!
//! This is scaffolding: the request loop lands in a later increment. It exits reporting that
//! it implements nothing yet rather than pretending to hand back a handshake, because a
//! caller that got a handshake would go on to send a `resolve` this build cannot answer.

fn main() -> std::process::ExitCode {
    eprintln!(
        "nostdb-provider-github {} speaks provider protocol {} and implements no request yet",
        env!("CARGO_PKG_VERSION"),
        nostdb_provider_github::PROVIDER_PROTOCOL_VERSION
    );
    // Not zero: a caller must not read "the provider ran" as "the provider served".
    std::process::ExitCode::from(10)
}
