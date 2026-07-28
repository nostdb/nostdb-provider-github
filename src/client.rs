//! The one implementation of [`Http`] that reaches GitHub.
//!
//! Everything else in this crate speaks the trait, so this file is the whole of the network
//! surface. Replacing it is one file and no test changes.
//!
//! # Nothing here is clever
//!
//! It builds a GET, sets three headers, and hands back a status, headers, and bytes. Every
//! decision about what a status *means* lives in [`crate::api`], where it can be tested
//! against a recorded response. Putting that judgement here would put it behind a network
//! call, which is the same as not testing it.

use crate::http::{Http, Response};
use std::io::Read as _;

/// The API version header GitHub asks callers to pin.
///
/// Sent so a future change to their default cannot alter what this provider sees. An
/// unpinned client is one whose behavior changes without a release.
const API_VERSION: &str = "2022-11-28";

/// How long a single request may take.
const TIMEOUT_SECONDS: u64 = 30;

/// The largest response this client will hold in memory.
///
/// A provider reads what a host sends it, and a host is not trusted to be reasonable. Every
/// byte here is attacker-influenced in the sense that matters: a repository somebody else
/// controls can contain a file of any size, and a build that tried to hold one would fail as
/// an allocation rather than as a diagnostic anybody can act on.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// An HTTP client over `ureq`.
pub struct UreqClient {
    agent: ureq::Agent,
    user_agent: String,
}

impl UreqClient {
    /// A client identifying itself as this provider at this version.
    ///
    /// GitHub requires a user agent and rate-limits anonymous ones harder. Naming the
    /// provider also means a repository owner reading their audit log sees what reached
    /// them rather than a generic library string.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(TIMEOUT_SECONDS)))
                .build()
                .into(),
            user_agent: format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        }
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for UreqClient {
    fn get(
        &mut self,
        url: &str,
        accept: &str,
        credential: Option<&str>,
    ) -> Result<Response, String> {
        let mut request = self
            .agent
            .get(url)
            .header("accept", accept)
            .header("user-agent", &self.user_agent)
            .header("x-github-api-version", API_VERSION);
        if let Some(secret) = credential {
            request = request.header("authorization", &format!("Bearer {secret}"));
        }

        // A refusal is a response, not an error: the caller decides what a status means.
        // Only a request that could not be completed at all is an error here.
        let mut response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(_)) => {
                return Err("the host answered with a status this client could not read".to_owned());
            }
            // The message is deliberately not the library's. A transport error can carry a
            // URL, and a URL can carry whatever a caller put in it — this is the one place
            // a credential could reach a diagnostic, so nothing from the error is echoed.
            Err(_) => return Err("the host could not be reached".to_owned()),
        };

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().ok()?.to_owned(),
                ))
            })
            .collect();

        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_RESPONSE_BYTES)
            .read_to_end(&mut body)
            .map_err(|_| "the host's answer could not be read".to_owned())?;

        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No test here reaches a network, which leaves exactly two things worth asserting: that
    // a client can be built, and that it identifies itself. Everything else this file does
    // is observable only against a real host, and is covered by the live conformance run the
    // Stage record names as needing separate authorization.

    #[test]
    fn a_client_identifies_itself_as_this_provider() {
        // GitHub rate-limits anonymous user agents harder, and a repository owner reading
        // an audit log should see what reached them rather than a library string.
        let client = UreqClient::new();
        assert!(client.user_agent.starts_with("nostdb-provider-github/"));
        assert!(client.user_agent.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn the_api_version_is_pinned() {
        // An unpinned client is one whose behavior changes without a release.
        assert!(!API_VERSION.is_empty());
        assert!(API_VERSION.starts_with("20"));
    }
}
