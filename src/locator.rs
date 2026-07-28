//! The `github://` locator.
//!
//! A locator is a link's **identity**. Everything here follows from that one fact: two
//! spellings of one repository must produce one locator, or the graph holds two links where
//! the user declared one, and neither of them is wrong enough to be obviously wrong.
//!
//! # What canonicalization does and does not touch
//!
//! GitHub treats an owner and a repository name case-insensitively, so those are lowered. A
//! repository's own paths are case-sensitive and so are Git refs, so those are left exactly
//! as written. Lowering a path would silently rename a file; leaving an owner alone would
//! give one repository two identities.
//!
//! # No default ref
//!
//! `ref` is required. Defaulting it to a branch name would make a locator's meaning depend
//! on the repository's current default branch, which can change — and an identity that
//! changes underneath the thing it identifies is not an identity.

use std::fmt;

/// The canonical scheme.
pub const SCHEME: &str = "github://";

/// The browser host a locator may be written against instead.
const BROWSER_PREFIX: &str = "https://github.com/";

/// A parsed, canonical GitHub locator.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitHubLocator {
    owner: String,
    repository: String,
    path: String,
    reference: String,
}

impl GitHubLocator {
    /// The owner, lowered.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The repository, lowered.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// The path within the repository, with its case preserved. Empty names the root.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The Git ref, with its case preserved.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Reports whether this names the repository root rather than a file within it.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.path.is_empty()
    }

    /// Parses a locator in canonical or browser form, canonicalizing it.
    ///
    /// # Errors
    ///
    /// Returns [`LocatorError`] naming what was wrong. A locator carrying a credential is
    /// refused rather than stripped: somebody who wrote one meant to use it, and quietly
    /// dropping it would turn an authentication mistake into a confusing "not found".
    pub fn parse(text: &str) -> Result<Self, LocatorError> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix(BROWSER_PREFIX) {
            return Self::from_browser(rest);
        }
        let Some(rest) = text.strip_prefix(SCHEME) else {
            return Err(LocatorError::NotGitHub);
        };
        if rest.contains('@') {
            return Err(LocatorError::CredentialPresent);
        }

        let (path_part, query) = rest.split_once('?').ok_or(LocatorError::NoReference)?;
        let reference = query
            .strip_prefix("ref=")
            .ok_or(LocatorError::NoReference)?;
        if reference.is_empty() {
            return Err(LocatorError::EmptyReference);
        }

        let mut segments = path_part.splitn(3, '/');
        let owner = segments.next().unwrap_or_default();
        let repository = segments.next().unwrap_or_default();
        let path = segments.next().unwrap_or_default();
        if owner.is_empty() {
            return Err(LocatorError::NoOwner);
        }
        if repository.is_empty() {
            return Err(LocatorError::NoRepository);
        }
        Self::build(owner, repository, path, reference)
    }

    /// Parses the tail of a `https://github.com/...` URL.
    ///
    /// A browser URL is accepted and normalized rather than rejected, because somebody
    /// pasting one has named a real repository and refusing would be pedantry. It is never
    /// *stored* in that form: that is what would give one repository two identities.
    fn from_browser(rest: &str) -> Result<Self, LocatorError> {
        let (rest, _) = rest.split_once('#').unwrap_or((rest, ""));
        let (rest, _) = rest.split_once('?').unwrap_or((rest, ""));
        let mut segments = rest.split('/');
        let owner = segments.next().unwrap_or_default();
        let repository = segments.next().unwrap_or_default();
        if owner.is_empty() {
            return Err(LocatorError::NoOwner);
        }
        if repository.is_empty() {
            return Err(LocatorError::NoRepository);
        }

        match segments.next() {
            // `/tree/<ref>/<path>` and `/blob/<ref>/<path>` differ only in what the browser
            // renders, and name the same thing.
            Some("tree" | "blob") => {
                let reference = segments.next().unwrap_or_default();
                if reference.is_empty() {
                    return Err(LocatorError::EmptyReference);
                }
                let path: Vec<&str> = segments.collect();
                Self::build(owner, repository, &path.join("/"), reference)
            }
            // A bare repository URL states no ref, and this must not invent one.
            Some(_) | None => Err(LocatorError::NoReference),
        }
    }

    fn build(
        owner: &str,
        repository: &str,
        path: &str,
        reference: &str,
    ) -> Result<Self, LocatorError> {
        for part in [owner, repository, reference] {
            if part.contains('@') {
                return Err(LocatorError::CredentialPresent);
            }
        }
        Ok(Self {
            // Lowered: GitHub treats these case-insensitively, so one repository is one
            // identity however it was typed.
            owner: owner.to_ascii_lowercase(),
            repository: repository.to_ascii_lowercase(),
            // Preserved: a repository's own paths are case-sensitive, and lowering one
            // would silently rename a file.
            path: path.trim_end_matches('/').to_owned(),
            // Preserved: a Git ref is case-sensitive.
            reference: reference.to_owned(),
        })
    }
}

impl fmt::Display for GitHubLocator {
    /// Renders the canonical form.
    ///
    /// The root is written with a trailing slash so a repository and a file within it are
    /// never one character apart in a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{SCHEME}{}/{}/{}?ref={}",
            self.owner, self.repository, self.path, self.reference
        )
    }
}

/// Why a locator is not one this provider reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocatorError {
    /// The scheme is not `github://` or a GitHub browser URL.
    NotGitHub,
    /// No owner was named.
    NoOwner,
    /// No repository was named.
    NoRepository,
    /// No `ref` was stated, and this must not invent one.
    NoReference,
    /// A `ref` was stated and is empty.
    EmptyReference,
    /// The locator carries a credential.
    CredentialPresent,
}

impl LocatorError {
    /// The code a refusal carries. Every one is the same: the locator is not readable.
    #[must_use]
    pub const fn code(self) -> crate::ProviderCode {
        crate::ProviderCode::LocatorInvalid
    }
}

impl fmt::Display for LocatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotGitHub => "not a github:// locator or a GitHub URL",
            Self::NoOwner => "no owner was named",
            Self::NoRepository => "no repository was named",
            Self::NoReference => {
                "no ref was stated, and a default branch can change so one is never assumed"
            }
            Self::EmptyReference => "the ref is empty",
            Self::CredentialPresent => "a credential must never appear in a locator",
        })
    }
}

impl std::error::Error for LocatorError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(text: &str) -> String {
        GitHubLocator::parse(text)
            .unwrap_or_else(|error| panic!("{text} should parse: {error}"))
            .to_string()
    }

    #[test]
    fn a_canonical_locator_round_trips() {
        for text in [
            "github://example/payments/?ref=main",
            "github://example/payments/.nostdb/root.nostdb?ref=v1.2.0",
        ] {
            assert_eq!(canonical(text), text);
        }
    }

    #[test]
    fn owner_and_repository_lower_and_path_and_ref_do_not() {
        // GitHub treats owner and repository case-insensitively, so one repository is one
        // identity however it was typed. Lowering a path would silently rename a file, and
        // a Git ref is case-sensitive.
        assert_eq!(
            canonical("github://Example/Payments/?ref=main"),
            "github://example/payments/?ref=main"
        );
        assert_eq!(
            canonical("github://example/payments/src/Main.rs?ref=Feature-A"),
            "github://example/payments/src/Main.rs?ref=Feature-A"
        );
    }

    #[test]
    fn a_browser_url_normalizes_rather_than_being_refused() {
        // Somebody pasting one has named a real repository. It is never stored in that
        // form, which is what would give one repository two identities.
        assert_eq!(
            canonical("https://github.com/Example/Payments/tree/main/src"),
            "github://example/payments/src?ref=main"
        );
        assert_eq!(
            canonical("https://github.com/example/payments/blob/main/README.md"),
            "github://example/payments/README.md?ref=main"
        );
        assert_eq!(
            canonical("https://github.com/example/payments/tree/main"),
            "github://example/payments/?ref=main"
        );
    }

    #[test]
    fn a_bare_browser_url_states_no_ref_and_none_is_invented() {
        // A default branch can change, and an identity that changes underneath the thing it
        // identifies is not an identity.
        assert_eq!(
            GitHubLocator::parse("https://github.com/example/payments"),
            Err(LocatorError::NoReference)
        );
    }

    #[test]
    fn percent_encoding_is_preserved_rather_than_decoded() {
        // Decoding here would make two distinct paths compare equal.
        assert_eq!(
            canonical("github://example/payments/docs/a%20b.md?ref=main"),
            "github://example/payments/docs/a%20b.md?ref=main"
        );
    }

    #[test]
    fn a_commit_is_a_ref_like_any_other() {
        let text = "github://example/payments/?ref=0f1e2d3c4b5a69788796a5b4c3d2e1f009182736";
        assert_eq!(canonical(text), text);
    }

    #[test]
    fn a_locator_carrying_a_credential_is_refused_rather_than_stripped() {
        // Somebody who wrote one meant to use it, and quietly dropping it would turn an
        // authentication mistake into a confusing "not found".
        assert_eq!(
            GitHubLocator::parse("github://token@example/payments/?ref=main"),
            Err(LocatorError::CredentialPresent)
        );
    }

    #[test]
    fn the_incomplete_forms_are_each_refused_for_their_own_reason() {
        for (text, expected) in [
            ("github://example/payments/", LocatorError::NoReference),
            ("github://example/?ref=main", LocatorError::NoRepository),
            (
                "github://example/payments/?ref=",
                LocatorError::EmptyReference,
            ),
            (
                "https://example.com/payments?ref=main",
                LocatorError::NotGitHub,
            ),
            ("github:///payments/?ref=main", LocatorError::NoOwner),
        ] {
            assert_eq!(GitHubLocator::parse(text), Err(expected), "{text}");
        }
    }

    #[test]
    fn every_refusal_carries_the_locator_code() {
        for error in [
            LocatorError::NotGitHub,
            LocatorError::NoOwner,
            LocatorError::NoRepository,
            LocatorError::NoReference,
            LocatorError::EmptyReference,
            LocatorError::CredentialPresent,
        ] {
            assert_eq!(error.code(), crate::ProviderCode::LocatorInvalid);
        }
    }

    #[test]
    fn the_root_and_a_file_within_it_are_distinguishable() {
        let root = GitHubLocator::parse("github://example/payments/?ref=main").unwrap();
        let file = GitHubLocator::parse("github://example/payments/a?ref=main").unwrap();
        assert!(root.is_root());
        assert!(!file.is_root());
        assert_ne!(root, file);
    }
}
