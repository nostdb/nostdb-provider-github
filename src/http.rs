//! What this provider needs from an HTTP client, and nothing more.
//!
//! # Why a trait rather than a client
//!
//! Everything this crate does with HTTP — building a request, reading a response, deciding
//! what a status means — is expressed here and tested against recorded responses. No test
//! reaches a network.
//!
//! That is not only about testing. It means choosing an HTTP client is a decision about
//! *one implementation of this trait* rather than a decision the whole crate is built on
//! top of, so it can be argued on its own merits when it is made rather than inherited from
//! whatever was convenient on the first day.
//!
//! It is also the only way to exercise what the product contract actually requires. Section
//! 16.3 asks for behavior when GitHub is *unreachable* and when a rate limit is *reached* —
//! neither of which a live test can produce on demand, and both of which a recorded
//! response produces exactly.

use std::collections::BTreeMap;

/// A response, reduced to what a decision here depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// Headers, lower-cased by the implementation so a lookup need not guess at case.
    pub headers: BTreeMap<String, String>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// A response with no headers, for a caller building one by hand.
    #[must_use]
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body: body.into(),
        }
    }

    /// Adds a header, lower-casing its name.
    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .insert(name.to_ascii_lowercase(), value.to_owned());
        self
    }

    /// One header's value.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// The one thing this provider asks of an HTTP client.
pub trait Http {
    /// Performs a GET.
    ///
    /// `accept` names the media type, which is how a blob is fetched raw rather than
    /// base64-wrapped inside JSON.
    ///
    /// `credential` is the secret itself, resolved by name before it reaches here. It is
    /// the only place in this crate that holds one, and it is never stored, logged, or
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns a reason when the request could not be completed at all. A response the
    /// server sent — including a refusal — is a success here and is judged by the caller.
    fn get(
        &mut self,
        url: &str,
        accept: &str,
        credential: Option<&str>,
    ) -> Result<Response, String>;
}
