//! GitHub's API, as the three questions a provider asks it.
//!
//! Resolve a ref to a commit, list what that commit contains, read one blob. Everything
//! here runs against the [`crate::http::Http`] trait, so every test uses a recorded
//! response and none reaches a network.
//!
//! # What a status means
//!
//! Mapping a status to a refusal is the part worth getting right, because the codes are what
//! a caller branches on and they mean different things to whoever hits them:
//!
//! - **404 is not always "no such repository".** For a request carrying no credential it is
//!   indistinguishable from "private repository you cannot see", and GitHub returns 404 for
//!   the second deliberately so that a probe cannot enumerate private repositories. So an
//!   unauthenticated 404 reports that a credential is required rather than that the source
//!   is gone — the opposite reading sends somebody looking for a typo in a name that is
//!   spelled correctly;
//! - **403 is not always a permissions failure.** GitHub uses it for rate limiting too, and
//!   the two are told apart by the remaining-quota header. Reporting a rate limit as a
//!   rejected credential would send somebody to rotate a token that is working;
//! - **5xx and a transport failure are the same thing to a caller.** The host did not
//!   answer, the link stays declared, and a query returns what it can reach.

use crate::ProviderCode;
use crate::http::{Http, Response};
use crate::locator::GitHubLocator;
use serde_json::Value;

/// The API host. A single constant so a future enterprise host has one place to change.
pub const API_ROOT: &str = "https://api.github.com";

/// The media type that returns a blob's bytes rather than JSON wrapping them.
pub const RAW_MEDIA_TYPE: &str = "application/vnd.github.raw";

/// The media type for everything else.
pub const JSON_MEDIA_TYPE: &str = "application/vnd.github+json";

/// One entry in a resolved tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// The path within the repository.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// The Git blob ID.
    ///
    /// Used to decide what to *avoid downloading*. It is not a digest the Engine trusts:
    /// a blob ID is a hash of the content with a Git header, computed by the host, and an
    /// artifact that is actually opened gets an independent digest first.
    pub blob_id: String,
}

/// A resolved tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    /// The entries, files only.
    pub entries: Vec<TreeEntry>,
}

/// Why an API call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// The code a reply carries.
    pub code: ProviderCode,
    /// A human-readable reason, which carries no credential.
    pub reason: String,
}

impl ApiError {
    fn new(code: ProviderCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.reason)
    }
}

impl std::error::Error for ApiError {}

/// Judges a response, turning a refusal into a code.
///
/// See the module documentation for why 404 and 403 are not read literally.
fn judge(response: &Response, authenticated: bool) -> Result<(), ApiError> {
    match response.status {
        200..=299 => Ok(()),
        401 => Err(ApiError::new(
            ProviderCode::CredentialRejected,
            "the host rejected the credential",
        )),
        403 | 429 if exhausted(response) => Err(ApiError::new(
            ProviderCode::LimitExceeded,
            "the host's rate limit is exhausted",
        )),
        403 => Err(ApiError::new(
            ProviderCode::CredentialRejected,
            "the credential does not permit this",
        )),
        // Without a credential this is indistinguishable from a private repository, and
        // GitHub answers 404 for that case deliberately. Reporting "gone" would send
        // somebody looking for a typo in a name that is spelled correctly.
        404 if !authenticated => Err(ApiError::new(
            ProviderCode::CredentialRequired,
            "not found, which without a credential also means not visible",
        )),
        404 => Err(ApiError::new(
            ProviderCode::SourceUnavailable,
            "the host has no such repository, ref, or path",
        )),
        500..=599 => Err(ApiError::new(
            ProviderCode::SourceUnavailable,
            "the host did not answer",
        )),
        other => Err(ApiError::new(
            ProviderCode::SourceUnavailable,
            format!("the host answered {other}"),
        )),
    }
}

/// Reports whether a refusal was a rate limit rather than a permissions failure.
fn exhausted(response: &Response) -> bool {
    response.header("x-ratelimit-remaining") == Some("0")
        || response.header("retry-after").is_some()
}

fn body_json(response: &Response) -> Result<Value, ApiError> {
    serde_json::from_slice(&response.body).map_err(|error| {
        ApiError::new(
            ProviderCode::SourceUnavailable,
            format!("the host's answer is not JSON: {error}"),
        )
    })
}

/// Resolves a locator's ref to an immutable commit.
///
/// A branch or tag is resolved once, and every later request uses the commit. Resolving per
/// request would let a branch move underneath a build and produce a graph assembled from two
/// different states of one repository.
///
/// # Errors
///
/// Returns an [`ApiError`] carrying the code a refusal reports.
pub fn resolve_commit(
    http: &mut dyn Http,
    locator: &GitHubLocator,
    credential: Option<&str>,
) -> Result<String, ApiError> {
    let url = format!(
        "{API_ROOT}/repos/{}/{}/commits/{}",
        locator.owner(),
        locator.repository(),
        locator.reference()
    );
    let response = http
        .get(&url, JSON_MEDIA_TYPE, credential)
        .map_err(|reason| ApiError::new(ProviderCode::SourceUnavailable, reason))?;
    judge(&response, credential.is_some())?;

    body_json(&response)?
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| !sha.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ApiError::new(
                ProviderCode::SourceUnavailable,
                "the host named no commit for that ref",
            )
        })
}

/// Lists the files one commit contains.
///
/// # Errors
///
/// Returns an [`ApiError`], including when the host truncated the tree. A truncated listing
/// is refused rather than returned: a build over a partial file list would report coverage
/// it does not have, and every file the listing omitted would look like a file the
/// repository does not contain.
pub fn enumerate_tree(
    http: &mut dyn Http,
    locator: &GitHubLocator,
    commit: &str,
    credential: Option<&str>,
) -> Result<Tree, ApiError> {
    let url = format!(
        "{API_ROOT}/repos/{}/{}/git/trees/{commit}?recursive=1",
        locator.owner(),
        locator.repository()
    );
    let response = http
        .get(&url, JSON_MEDIA_TYPE, credential)
        .map_err(|reason| ApiError::new(ProviderCode::SourceUnavailable, reason))?;
    judge(&response, credential.is_some())?;
    let document = body_json(&response)?;

    if document.get("truncated").and_then(Value::as_bool) == Some(true) {
        return Err(ApiError::new(
            ProviderCode::LimitExceeded,
            "the host truncated the tree, so this listing is not the whole repository",
        ));
    }

    let entries = document
        .get("tree")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::new(ProviderCode::SourceUnavailable, "the host listed no tree"))?
        .iter()
        // Directories and submodule gitlinks are not files to read. A gitlink in
        // particular points at another repository entirely.
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("blob"))
        .map(|entry| {
            Ok(TreeEntry {
                path: entry
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ApiError::new(ProviderCode::SourceUnavailable, "an entry states no path")
                    })?
                    .to_owned(),
                bytes: entry.get("size").and_then(Value::as_u64).unwrap_or(0),
                blob_id: entry
                    .get("sha")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ApiError::new(
                            ProviderCode::SourceUnavailable,
                            "an entry states no blob id",
                        )
                    })?
                    .to_owned(),
            })
        })
        .collect::<Result<Vec<TreeEntry>, ApiError>>()?;
    Ok(Tree { entries })
}

/// Reads one blob's bytes.
///
/// Requested with the raw media type, so the bytes arrive as themselves rather than
/// base64-wrapped inside JSON — which would inflate every file by a third and make a large
/// one cost more to decode than to fetch.
///
/// # Errors
///
/// Returns an [`ApiError`] carrying the code a refusal reports.
pub fn read_blob(
    http: &mut dyn Http,
    locator: &GitHubLocator,
    blob_id: &str,
    credential: Option<&str>,
) -> Result<Vec<u8>, ApiError> {
    let url = format!(
        "{API_ROOT}/repos/{}/{}/git/blobs/{blob_id}",
        locator.owner(),
        locator.repository()
    );
    let response = http
        .get(&url, RAW_MEDIA_TYPE, credential)
        .map_err(|reason| ApiError::new(ProviderCode::SourceUnavailable, reason))?;
    judge(&response, credential.is_some())?;
    Ok(response.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// An HTTP client that replays one recorded response and remembers the request.
    struct Recorded {
        response: Response,
        url: String,
        accept: String,
        authenticated: bool,
    }

    impl Recorded {
        fn new(response: Response) -> Self {
            Self {
                response,
                url: String::new(),
                accept: String::new(),
                authenticated: false,
            }
        }
    }

    impl Http for Recorded {
        fn get(
            &mut self,
            url: &str,
            accept: &str,
            credential: Option<&str>,
        ) -> Result<Response, String> {
            self.url = url.to_owned();
            self.accept = accept.to_owned();
            self.authenticated = credential.is_some();
            Ok(self.response.clone())
        }
    }

    fn locator() -> GitHubLocator {
        GitHubLocator::parse("github://example/payments/?ref=main").expect("a locator")
    }

    fn json(status: u16, body: &str) -> Response {
        Response::new(status, body.as_bytes())
    }

    #[test]
    fn a_ref_resolves_to_a_commit() {
        let mut http = Recorded::new(json(200, r#"{"sha":"0f1e2d3c"}"#));
        let commit = resolve_commit(&mut http, &locator(), Some("secret")).expect("resolves");
        assert_eq!(commit, "0f1e2d3c");
        assert!(
            http.url.ends_with("/repos/example/payments/commits/main"),
            "{}",
            http.url
        );
    }

    #[test]
    fn a_tree_lists_files_and_leaves_out_what_is_not_one() {
        // A directory is not a file to read, and a gitlink points at another repository
        // entirely.
        let mut http = Recorded::new(json(
            200,
            r#"{"truncated":false,"tree":[
                {"path":"src","type":"tree","sha":"aaa"},
                {"path":"src/main.rs","type":"blob","size":412,"sha":"bbb"},
                {"path":"vendor/lib","type":"commit","sha":"ccc"}
            ]}"#,
        ));
        let tree = enumerate_tree(&mut http, &locator(), "0f1e", Some("secret")).expect("lists");
        assert_eq!(tree.entries.len(), 1);
        assert_eq!(tree.entries[0].path, "src/main.rs");
        assert_eq!(tree.entries[0].blob_id, "bbb");
        assert!(http.url.contains("recursive=1"), "{}", http.url);
    }

    #[test]
    fn a_truncated_tree_is_refused_rather_than_returned() {
        // A build over a partial listing would report coverage it does not have, and every
        // omitted file would look like one the repository does not contain.
        let mut http = Recorded::new(json(200, r#"{"truncated":true,"tree":[]}"#));
        let refused = enumerate_tree(&mut http, &locator(), "0f1e", Some("s")).unwrap_err();
        assert_eq!(refused.code, ProviderCode::LimitExceeded);
    }

    #[test]
    fn a_blob_is_fetched_raw_rather_than_base64_wrapped() {
        let mut http = Recorded::new(Response::new(200, b"fn main() {}".to_vec()));
        let bytes = read_blob(&mut http, &locator(), "bbb", Some("s")).expect("reads");
        assert_eq!(bytes, b"fn main() {}");
        assert_eq!(http.accept, RAW_MEDIA_TYPE);
    }

    #[test]
    fn an_unauthenticated_404_asks_for_a_credential_rather_than_reporting_it_gone() {
        // GitHub answers 404 for a private repository deliberately, so a probe cannot
        // enumerate them. Reporting "gone" would send somebody looking for a typo in a name
        // that is spelled correctly.
        let mut http = Recorded::new(json(404, "{}"));
        let refused = resolve_commit(&mut http, &locator(), None).unwrap_err();
        assert_eq!(refused.code, ProviderCode::CredentialRequired);
    }

    #[test]
    fn an_authenticated_404_is_a_source_that_is_not_there() {
        let mut http = Recorded::new(json(404, "{}"));
        let refused = resolve_commit(&mut http, &locator(), Some("secret")).unwrap_err();
        assert_eq!(refused.code, ProviderCode::SourceUnavailable);
    }

    #[test]
    fn a_rate_limited_403_is_not_reported_as_a_rejected_credential() {
        // Sending somebody to rotate a token that is working is worse than saying nothing.
        let mut http = Recorded::new(json(403, "{}").with_header("X-RateLimit-Remaining", "0"));
        let refused = resolve_commit(&mut http, &locator(), Some("s")).unwrap_err();
        assert_eq!(refused.code, ProviderCode::LimitExceeded);

        let mut plain = Recorded::new(json(403, "{}"));
        let refused = resolve_commit(&mut plain, &locator(), Some("s")).unwrap_err();
        assert_eq!(refused.code, ProviderCode::CredentialRejected);
    }

    #[test]
    fn a_429_with_a_retry_after_is_a_rate_limit() {
        let mut http = Recorded::new(json(429, "{}").with_header("Retry-After", "60"));
        let refused = resolve_commit(&mut http, &locator(), Some("s")).unwrap_err();
        assert_eq!(refused.code, ProviderCode::LimitExceeded);
    }

    #[test]
    fn a_401_is_always_a_rejected_credential() {
        let mut http = Recorded::new(json(401, "{}"));
        assert_eq!(
            resolve_commit(&mut http, &locator(), Some("s"))
                .unwrap_err()
                .code,
            ProviderCode::CredentialRejected
        );
    }

    #[test]
    fn a_host_that_did_not_answer_leaves_the_source_unavailable() {
        // 5xx and a transport failure are the same thing to a caller: the link stays
        // declared and a query returns what it can reach.
        for response in [json(500, ""), json(503, "")] {
            let mut http = Recorded::new(response);
            assert_eq!(
                resolve_commit(&mut http, &locator(), Some("s"))
                    .unwrap_err()
                    .code,
                ProviderCode::SourceUnavailable
            );
        }

        struct Broken;
        impl Http for Broken {
            fn get(&mut self, _: &str, _: &str, _: Option<&str>) -> Result<Response, String> {
                Err("connection refused".to_owned())
            }
        }
        assert_eq!(
            resolve_commit(&mut Broken, &locator(), Some("s"))
                .unwrap_err()
                .code,
            ProviderCode::SourceUnavailable
        );
    }

    #[test]
    fn a_ref_the_host_answers_without_a_commit_is_not_treated_as_resolved() {
        let mut http = Recorded::new(json(200, r#"{"sha":""}"#));
        assert_eq!(
            resolve_commit(&mut http, &locator(), Some("s"))
                .unwrap_err()
                .code,
            ProviderCode::SourceUnavailable
        );
    }

    #[test]
    fn a_credential_never_appears_in_a_reason() {
        // The one place a secret exists in this crate is the argument to `Http::get`. A
        // reason that echoed it would put it in a diagnostic, which the contract forbids.
        let mut http = Recorded::new(json(403, "{}"));
        let refused = resolve_commit(&mut http, &locator(), Some("ghp_secret_value")).unwrap_err();
        assert!(
            !refused.reason.contains("ghp_secret_value"),
            "{}",
            refused.reason
        );
        assert!(!refused.to_string().contains("ghp_secret_value"));
    }

    #[test]
    fn a_header_lookup_does_not_depend_on_case() {
        let response = Response::new(200, "").with_header("X-RateLimit-Remaining", "0");
        assert_eq!(response.header("x-ratelimit-remaining"), Some("0"));
        assert_eq!(response.header("X-RATELIMIT-REMAINING"), Some("0"));
        let _: BTreeMap<String, String> = response.headers;
    }
}
