//! The provider executable.
//!
//! Reads one request per line from standard input and writes one reply per line to standard
//! output. Standard error carries diagnostics and never a reply, so a caller reading replies
//! cannot be confused by one.
//!
//! # The credential is read once, from the environment, and never written down
//!
//! The Engine passes a credential *name*; resolving it is this process's job. This build
//! resolves it from an environment variable, which the product contract lists first among
//! the permitted resolvers. It is read into memory at start and never logged, echoed, or
//! written anywhere — the repository verifier rejects a token literal in this repository at
//! all, and every diagnostic path is written to carry none.

use nostdb_provider_github::client::UreqClient;
use nostdb_provider_github::serve::{Outcome, Session};
use std::io::{BufRead as _, Write as _};

/// Where a credential is read from.
///
/// One variable rather than one per credential name. A provider process serves one source
/// at a time, and the Engine chose which credential applies before it started this process.
const CREDENTIAL_VARIABLE: &str = "NOSTDB_GITHUB_TOKEN";

fn main() -> std::process::ExitCode {
    let credential = std::env::var(CREDENTIAL_VARIABLE)
        .ok()
        .filter(|value| !value.is_empty());
    let mut http = UreqClient::new();
    let mut session = Session::new();

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            eprintln!("the request stream could not be read");
            return std::process::ExitCode::from(9);
        };
        match session.handle(&line, &mut http, credential.as_deref()) {
            Outcome::Done => break,
            Outcome::Reply { line, content } => {
                // The line and its content are written together and flushed once. A reply
                // that reached the Engine without its bytes would leave the stream framed
                // wrong, and a stream framed wrong cannot report that it is.
                if writeln!(stdout, "{line}")
                    .and_then(|()| stdout.write_all(&content))
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    // The Engine has gone. There is nobody to tell.
                    return std::process::ExitCode::from(9);
                }
            }
        }
    }
    std::process::ExitCode::SUCCESS
}
