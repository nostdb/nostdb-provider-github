//! The request loop.
//!
//! One request per line on standard input, one reply per line on standard output. Standard
//! error carries diagnostics and never a reply, so a caller reading replies cannot be
//! confused by one.
//!
//! # Refusing is answering
//!
//! Every failure here becomes a reply. A provider that exited to signal one would leave the
//! Engine unable to tell a version mismatch from a crash, and those need different things
//! from whoever hits them.
//!
//! # The snapshot is remembered, not re-resolved
//!
//! `resolve` records the commit a ref pointed at, and every later request in the same
//! session uses it. Re-resolving per request would let a branch move underneath a build and
//! produce a graph assembled from two different states of one repository.

use crate::ProviderCode;
use crate::api;
use crate::http::Http;
use crate::locator::GitHubLocator;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// The protocol version this build speaks.
pub const PROTOCOL_VERSION: u32 = crate::PROVIDER_PROTOCOL_VERSION;

/// What a request produced.
pub enum Outcome {
    /// A reply line, and content to follow it.
    Reply {
        /// The line.
        line: String,
        /// Bytes that follow it, for a `read` or a `materialize`.
        content: Vec<u8>,
    },
    /// The stream ended.
    Done,
}

/// One session's state.
///
/// A session remembers which commit each snapshot identifier refers to, and which locator it
/// came from, because a later request names only the snapshot.
#[derive(Default)]
pub struct Session {
    resolved: BTreeMap<String, GitHubLocator>,
    handshaken: bool,
}

impl Session {
    /// A session that has agreed nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers one request line.
    ///
    /// Never returns an error: every failure is a reply, which is what lets a caller tell a
    /// refusal from a crash.
    pub fn handle(&mut self, line: &str, http: &mut dyn Http, credential: Option<&str>) -> Outcome {
        let line = line.trim();
        if line.is_empty() {
            return Outcome::Done;
        }
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            return refuse(ProviderCode::RequestInvalid, "the request is not JSON");
        };

        match request
            .get("provider_protocol_version")
            .and_then(Value::as_u64)
        {
            Some(found) if found == u64::from(PROTOCOL_VERSION) => {}
            Some(found) => {
                return refuse(
                    ProviderCode::ProtocolUnsupported,
                    &format!(
                        "this provider speaks {PROTOCOL_VERSION} and the request states {found}"
                    ),
                );
            }
            None => {
                return refuse(
                    ProviderCode::RequestInvalid,
                    "the request states no protocol version",
                );
            }
        }

        let kind = request.get("request").and_then(Value::as_str);
        if kind != Some("handshake") && !self.handshaken {
            // Answering before a version is agreed would mean guessing what the request
            // meant, which is the thing the handshake exists to prevent.
            return refuse(
                ProviderCode::RequestInvalid,
                "no handshake has been exchanged",
            );
        }

        match kind {
            Some("handshake") => {
                self.handshaken = true;
                reply(json!({
                    "provider_protocol_version": PROTOCOL_VERSION,
                    "reply": "handshake",
                    "provider": crate::PROVIDER_NAME,
                    "provider_version": env!("CARGO_PKG_VERSION"),
                    // Both roles: GitHub can be analyzed as source and can hold a published
                    // graph, which section 15.2 requires of this provider specifically.
                    "roles": ["source", "graph_store"],
                }))
            }
            Some("resolve") => self.resolve(&request, http, credential),
            Some("enumerate") => self.enumerate(&request, http, credential),
            Some("read") => self.read(&request, http, credential),
            Some("materialize") => self.materialize(&request, http, credential),
            Some(other) => refuse(
                ProviderCode::RequestInvalid,
                &format!("`{other}` is not a request this version defines"),
            ),
            None => refuse(ProviderCode::RequestInvalid, "the request names no kind"),
        }
    }

    fn resolve(
        &mut self,
        request: &Value,
        http: &mut dyn Http,
        credential: Option<&str>,
    ) -> Outcome {
        let Some(text) = request.get("locator").and_then(Value::as_str) else {
            return refuse(ProviderCode::RequestInvalid, "the request names no locator");
        };
        // A secret in a request is refused rather than used. The protocol passes a name, and
        // a provider that accepted a secret would make the Engine a place one had been.
        if request
            .get("credential")
            .and_then(Value::as_object)
            .is_some_and(|object| !object.contains_key("ref"))
        {
            return refuse(
                ProviderCode::RequestInvalid,
                "a credential travels as a name, and this request carries something else",
            );
        }

        let locator = match GitHubLocator::parse(text) {
            Ok(locator) => locator,
            Err(error) => return refuse(ProviderCode::LocatorInvalid, &error.to_string()),
        };
        let commit = match api::resolve_commit(http, &locator, credential) {
            Ok(commit) => commit,
            Err(error) => return refuse(error.code, &error.reason),
        };

        let canonical = locator.to_string();
        self.resolved.insert(commit.clone(), locator);
        reply(json!({
            "provider_protocol_version": PROTOCOL_VERSION,
            "reply": "resolve",
            "snapshot": commit,
            "canonical_locator": canonical,
            // This build has no cache yet, and says so rather than omitting the member. An
            // absent `cached` is refused by the Engine, which is the correct reading: a
            // provider that did not say must not be recorded as having confirmed anything.
            "cached": false,
        }))
    }

    fn enumerate(
        &mut self,
        request: &Value,
        http: &mut dyn Http,
        credential: Option<&str>,
    ) -> Outcome {
        let Some((snapshot, locator)) = self.snapshot(request) else {
            return refuse(
                ProviderCode::RequestInvalid,
                "that snapshot was not resolved in this session",
            );
        };
        match api::enumerate_tree(http, &locator, &snapshot, credential) {
            Err(error) => refuse(error.code, &error.reason),
            Ok(tree) => reply(json!({
                "provider_protocol_version": PROTOCOL_VERSION,
                "reply": "enumerate",
                "entries": tree.entries.iter().map(|entry| json!({
                    "path": entry.path,
                    "bytes": entry.bytes,
                    "content_id": entry.blob_id,
                })).collect::<Vec<_>>(),
            })),
        }
    }

    fn read(&mut self, request: &Value, http: &mut dyn Http, credential: Option<&str>) -> Outcome {
        let Some((snapshot, locator)) = self.snapshot(request) else {
            return refuse(
                ProviderCode::RequestInvalid,
                "that snapshot was not resolved in this session",
            );
        };
        let Some(path) = request.get("path").and_then(Value::as_str) else {
            return refuse(ProviderCode::RequestInvalid, "the request names no path");
        };

        // The tree is listed again to turn a path into a blob id. A session cache belongs
        // here and is increment 5's remaining work; correctness first, and a second request
        // is the honest cost of not having it yet.
        let tree = match api::enumerate_tree(http, &locator, &snapshot, credential) {
            Ok(tree) => tree,
            Err(error) => return refuse(error.code, &error.reason),
        };
        let Some(entry) = tree.entries.iter().find(|entry| entry.path == path) else {
            return refuse(
                ProviderCode::SourceUnavailable,
                "that snapshot contains no such path",
            );
        };
        match api::read_blob(http, &locator, &entry.blob_id, credential) {
            Err(error) => refuse(error.code, &error.reason),
            Ok(content) => Outcome::Reply {
                line: json!({
                    "provider_protocol_version": PROTOCOL_VERSION,
                    "reply": "read",
                    "bytes": content.len(),
                })
                .to_string(),
                content,
            },
        }
    }

    fn materialize(
        &mut self,
        request: &Value,
        http: &mut dyn Http,
        credential: Option<&str>,
    ) -> Outcome {
        let Some((snapshot, locator)) = self.snapshot(request) else {
            return refuse(
                ProviderCode::RequestInvalid,
                "that snapshot was not resolved in this session",
            );
        };
        if locator.is_root() {
            // A graph artifact is a file. A locator naming a repository root names no
            // artifact, and guessing at `.nostdb/root.nostdb` would be inventing a path the
            // user did not write.
            return refuse(
                ProviderCode::LocatorInvalid,
                "a graph locator names a file, and this one names a repository root",
            );
        }

        let tree = match api::enumerate_tree(http, &locator, &snapshot, credential) {
            Ok(tree) => tree,
            Err(error) => return refuse(error.code, &error.reason),
        };
        let Some(entry) = tree
            .entries
            .iter()
            .find(|entry| entry.path == locator.path())
        else {
            return refuse(
                ProviderCode::SourceUnavailable,
                "that snapshot contains no such artifact",
            );
        };
        match api::read_blob(http, &locator, &entry.blob_id, credential) {
            Err(error) => refuse(error.code, &error.reason),
            Ok(content) => Outcome::Reply {
                line: json!({
                    "provider_protocol_version": PROTOCOL_VERSION,
                    "reply": "materialize",
                    "bytes": content.len(),
                    // The Engine checks this rather than accepting it, which is why it is
                    // computed over the bytes actually being sent rather than copied from
                    // anything the host said.
                    "content_digest": digest(&content),
                })
                .to_string(),
                content,
            },
        }
    }

    /// The locator a snapshot identifier refers to, when this session resolved it.
    fn snapshot(&self, request: &Value) -> Option<(String, GitHubLocator)> {
        let snapshot = request.get("snapshot").and_then(Value::as_str)?;
        let locator = self.resolved.get(snapshot)?;
        Some((snapshot.to_owned(), locator.clone()))
    }
}

/// A SHA-256 digest in the `algorithm:hex` form the contracts use.
///
/// The provider computes this so the Engine has something to check against. The Engine's own
/// digest over the bytes it received is the authoritative one, so a disagreement surfaces as
/// a refusal rather than as a wrong answer — which is the whole reason both exist.
fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut text = String::from("sha256:");
    for byte in hasher.finalize() {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn reply(document: Value) -> Outcome {
    Outcome::Reply {
        line: document.to_string(),
        content: Vec::new(),
    }
}

fn refuse(code: ProviderCode, message: &str) -> Outcome {
    reply(json!({
        "provider_protocol_version": PROTOCOL_VERSION,
        "reply": "error",
        "code": code.as_str(),
        "message": message,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Response;

    /// An HTTP client that answers each URL from a table.
    struct Canned(Vec<(&'static str, Response)>);

    impl Http for Canned {
        fn get(&mut self, url: &str, _: &str, _: Option<&str>) -> Result<Response, String> {
            self.0
                .iter()
                .find(|(fragment, _)| url.contains(fragment))
                .map(|(_, response)| response.clone())
                .ok_or_else(|| format!("nothing recorded for {url}"))
        }
    }

    fn json(body: &str) -> Response {
        Response::new(200, body.as_bytes())
    }

    fn canned() -> Canned {
        Canned(vec![
            ("/commits/main", json(r#"{"sha":"0f1e2d3c"}"#)),
            (
                "/git/trees/",
                json(
                    r#"{"truncated":false,"tree":[
                        {"path":"src/main.rs","type":"blob","size":12,"sha":"bbb"},
                        {"path":"root.nostdb","type":"blob","size":4,"sha":"ccc"}
                    ]}"#,
                ),
            ),
            (
                "/git/blobs/bbb",
                Response::new(200, b"fn main() {}".to_vec()),
            ),
            ("/git/blobs/ccc", Response::new(200, b"NDB1".to_vec())),
        ])
    }

    fn line(outcome: &Outcome) -> Value {
        match outcome {
            Outcome::Reply { line, .. } => serde_json::from_str(line).expect("a reply is JSON"),
            Outcome::Done => panic!("the stream ended"),
        }
    }

    fn content(outcome: &Outcome) -> &[u8] {
        match outcome {
            Outcome::Reply { content, .. } => content,
            Outcome::Done => panic!("the stream ended"),
        }
    }

    fn shaken() -> (Session, Canned) {
        let (mut session, mut http) = (Session::new(), canned());
        let reply = session.handle(
            r#"{"provider_protocol_version":1,"request":"handshake"}"#,
            &mut http,
            None,
        );
        assert_eq!(line(&reply)["reply"], "handshake");
        (session, http)
    }

    fn resolved() -> (Session, Canned) {
        let (mut session, mut http) = shaken();
        let reply = session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"github://Example/Payments/src/main.rs?ref=main"}"#,
            &mut http,
            Some("secret"),
        );
        assert_eq!(line(&reply)["snapshot"], "0f1e2d3c");
        (session, http)
    }

    #[test]
    fn a_handshake_declares_both_roles() {
        // Section 15.2 requires this provider to serve both: analyzing repository source
        // and retrieving a published graph.
        let (mut session, mut http) = (Session::new(), canned());
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"handshake"}"#,
            &mut http,
            None,
        ));
        assert_eq!(reply["provider"], "github");
        assert_eq!(reply["roles"], json!(["source", "graph_store"]));
    }

    #[test]
    fn nothing_is_answered_before_a_handshake() {
        let (mut session, mut http) = (Session::new(), canned());
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"github://a/b/?ref=main"}"#,
            &mut http,
            None,
        ));
        assert_eq!(reply["code"], "PROVIDER_REQUEST_INVALID");
    }

    #[test]
    fn an_unreadable_version_is_refused_with_a_reply_rather_than_an_exit() {
        // An Engine that gets no reply cannot tell a version mismatch from a crash.
        let (mut session, mut http) = (Session::new(), canned());
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":99,"request":"handshake"}"#,
            &mut http,
            None,
        ));
        assert_eq!(reply["reply"], "error");
        assert_eq!(reply["code"], "PROVIDER_PROTOCOL_UNSUPPORTED");
    }

    #[test]
    fn a_resolve_returns_the_canonical_locator_and_says_it_is_not_cached() {
        let (mut session, mut http) = shaken();
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"https://github.com/Example/Payments/tree/main/src"}"#,
            &mut http,
            Some("secret"),
        ));
        assert_eq!(
            reply["canonical_locator"],
            "github://example/payments/src?ref=main"
        );
        assert_eq!(
            reply["cached"], false,
            "absent would be refused by the Engine"
        );
    }

    #[test]
    fn a_request_carrying_a_secret_rather_than_a_name_is_refused() {
        // A provider that accepted one would make the Engine a place a credential had been.
        let (mut session, mut http) = shaken();
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"github://a/b/?ref=main","credential":{"token":"ghp_x"}}"#,
            &mut http,
            None,
        ));
        assert_eq!(reply["code"], "PROVIDER_REQUEST_INVALID");
    }

    #[test]
    fn a_snapshot_this_session_did_not_resolve_is_refused() {
        // Re-resolving a ref per request would let a branch move underneath a build and
        // produce a graph assembled from two states of one repository.
        let (mut session, mut http) = shaken();
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"enumerate","snapshot":"deadbeef"}"#,
            &mut http,
            Some("s"),
        ));
        assert_eq!(reply["code"], "PROVIDER_REQUEST_INVALID");
    }

    #[test]
    fn enumeration_reports_each_file_with_the_hosts_own_identifier() {
        let (mut session, mut http) = resolved();
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"enumerate","snapshot":"0f1e2d3c"}"#,
            &mut http,
            Some("s"),
        ));
        assert_eq!(reply["entries"][0]["path"], "src/main.rs");
        assert_eq!(reply["entries"][0]["content_id"], "bbb");
    }

    #[test]
    fn a_read_declares_its_length_and_the_bytes_follow() {
        let (mut session, mut http) = resolved();
        let outcome = session.handle(
            r#"{"provider_protocol_version":1,"request":"read","snapshot":"0f1e2d3c","path":"src/main.rs"}"#,
            &mut http,
            Some("s"),
        );
        assert_eq!(line(&outcome)["bytes"], 12);
        assert_eq!(content(&outcome), b"fn main() {}");
    }

    #[test]
    fn a_path_the_snapshot_does_not_contain_leaves_the_source_unavailable() {
        let (mut session, mut http) = resolved();
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"read","snapshot":"0f1e2d3c","path":"absent.rs"}"#,
            &mut http,
            Some("s"),
        ));
        assert_eq!(reply["code"], "PROVIDER_SOURCE_UNAVAILABLE");
    }

    #[test]
    fn a_materialize_carries_a_digest_over_the_bytes_it_is_sending() {
        let (mut session, mut http) = shaken();
        session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"github://example/payments/root.nostdb?ref=main"}"#,
            &mut http,
            Some("s"),
        );
        let outcome = session.handle(
            r#"{"provider_protocol_version":1,"request":"materialize","snapshot":"0f1e2d3c"}"#,
            &mut http,
            Some("s"),
        );
        assert_eq!(content(&outcome), b"NDB1");
        assert_eq!(line(&outcome)["content_digest"], digest(b"NDB1"));
    }

    #[test]
    fn a_graph_locator_naming_a_repository_root_names_no_artifact() {
        // Guessing at `.nostdb/root.nostdb` would be inventing a path the user did not
        // write.
        let (mut session, mut http) = resolved_root();
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"materialize","snapshot":"0f1e2d3c"}"#,
            &mut http,
            Some("s"),
        ));
        assert_eq!(reply["code"], "PROVIDER_LOCATOR_INVALID");
    }

    fn resolved_root() -> (Session, Canned) {
        let (mut session, mut http) = shaken();
        session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"github://example/payments/?ref=main"}"#,
            &mut http,
            Some("s"),
        );
        (session, http)
    }

    #[test]
    fn a_malformed_or_unknown_request_is_refused_rather_than_guessed_at() {
        let (mut session, mut http) = shaken();
        for text in [
            "not json at all",
            r#"{"request":"handshake"}"#,
            r#"{"provider_protocol_version":1,"request":"frobnicate"}"#,
            r#"{"provider_protocol_version":1}"#,
        ] {
            let reply = line(&session.handle(text, &mut http, None));
            assert_eq!(reply["reply"], "error", "{text}");
        }
    }

    #[test]
    fn an_empty_line_ends_the_stream() {
        let (mut session, mut http) = shaken();
        assert!(matches!(session.handle("", &mut http, None), Outcome::Done));
    }

    #[test]
    fn a_refusal_never_echoes_a_credential() {
        let (mut session, mut http) = (Session::new(), Canned(Vec::new()));
        let reply = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"handshake"}"#,
            &mut http,
            Some("ghp_secret_value"),
        ));
        assert!(!reply.to_string().contains("ghp_secret_value"));

        let refused = line(&session.handle(
            r#"{"provider_protocol_version":1,"request":"resolve","locator":"github://a/b/?ref=main"}"#,
            &mut http,
            Some("ghp_secret_value"),
        ));
        assert!(
            !refused.to_string().contains("ghp_secret_value"),
            "{refused}"
        );
    }
}
