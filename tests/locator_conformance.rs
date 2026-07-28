//! Conformance against the `nostdb-spec` locator fixtures.
//!
//! The specification publishes twelve locators and, for each accepted one, the canonical
//! form it must normalize to. Checking the canonical form rather than only "it parsed" is
//! the point: a locator is a link's identity, and an implementation that accepts a browser
//! URL without normalizing it would pass a parse-only suite while giving one repository two
//! identities.

use nostdb_provider_github::locator::GitHubLocator;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn fixture_root() -> Option<PathBuf> {
    let raw = std::env::var("NOSTDB_SPEC_FIXTURES").ok()?;
    let directory = PathBuf::from(raw).join("provider").join("locator");
    directory.is_dir().then_some(directory)
}

fn expectations(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path.with_extension("expected")).unwrap_or_else(|error| {
        panic!(
            "cannot read the expectation for {}: {error}",
            path.display()
        )
    });
    text.lines()
        .filter_map(|line| line.split_once(" = "))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn locators(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("txt"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn every_accepted_locator_parses_and_normalizes_to_its_declared_form() {
    let Some(root) = fixture_root() else {
        println!("locator conformance: skipped, NOSTDB_SPEC_FIXTURES is unset");
        return;
    };
    let paths = locators(&root.join("valid"));
    assert!(!paths.is_empty(), "no accepted locators were found");
    for path in &paths {
        let expected = expectations(path);
        assert_eq!(expected.get("outcome").map(String::as_str), Some("accept"));
        let text = std::fs::read_to_string(path).expect("fixture is UTF-8");
        let parsed = GitHubLocator::parse(text.trim()).unwrap_or_else(|error| {
            panic!(
                "{} is accepted by the specification and refused here: {error}",
                path.display()
            )
        });
        let declared = expected
            .get("canonical")
            .unwrap_or_else(|| panic!("{} declares no canonical form", path.display()));
        assert_eq!(
            &parsed.to_string(),
            declared,
            "{} normalized to the wrong identity",
            path.display()
        );
    }
    println!(
        "locator conformance: {} accepted locators verified",
        paths.len()
    );
}

#[test]
fn every_rejected_locator_is_refused_with_the_declared_code() {
    let Some(root) = fixture_root() else {
        println!("locator conformance: skipped, NOSTDB_SPEC_FIXTURES is unset");
        return;
    };
    let paths = locators(&root.join("invalid"));
    assert!(!paths.is_empty(), "no rejected locators were found");
    for path in &paths {
        let expected = expectations(path);
        assert_eq!(expected.get("outcome").map(String::as_str), Some("reject"));
        let declared = expected
            .get("code")
            .unwrap_or_else(|| panic!("{} declares no code", path.display()));

        let text = std::fs::read_to_string(path).expect("fixture is UTF-8");
        let Err(error) = GitHubLocator::parse(text.trim()) else {
            panic!(
                "{} is rejected by the specification and accepted here",
                path.display()
            );
        };
        assert_eq!(
            error.code().as_str(),
            declared,
            "{} was refused with the wrong code",
            path.display()
        );
    }
    println!(
        "locator conformance: {} rejected locators verified",
        paths.len()
    );
}

#[test]
fn normalizing_is_idempotent() {
    // A canonical form that does not survive being parsed again would make equality depend
    // on how many times a locator had been through the parser.
    let Some(root) = fixture_root() else {
        println!("locator conformance: skipped, NOSTDB_SPEC_FIXTURES is unset");
        return;
    };
    for path in locators(&root.join("valid")) {
        let text = std::fs::read_to_string(&path).expect("fixture is UTF-8");
        let once = GitHubLocator::parse(text.trim()).expect("it parses");
        let twice = GitHubLocator::parse(&once.to_string()).expect("the canonical form parses");
        assert_eq!(once, twice, "{}", path.display());
    }
}
