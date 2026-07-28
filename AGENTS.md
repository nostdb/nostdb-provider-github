# nostdb-provider-github Agent Instructions

## Inheritance

This repository is a child of the NostDB root superproject. The root `AGENTS.md`
at <https://github.com/nostdb/nostdb> is the governing contract.

This file only narrows the root rules for the provider boundary. It must not weaken any
root product, safety, or ownership boundary. If this file and the root contract appear to
conflict, the root contract wins, the current valid behavior stays unchanged, and the exact
conflict is recorded in the root `IMPLEMENTATION_PROGRESS.md`.

## Language policy

Write everything in this repository in English only.

This covers documentation, source code, identifiers, comments, rustdoc, test names, commit
messages, branch names, pull request titles and bodies, issue text, diagnostics, error
messages, log records, configuration, and fixtures.

This rule holds regardless of the language a request is written in.

## Ownership boundary

`nostdb-provider-github` retrieves bytes and metadata from GitHub. It interprets none of
them.

Permitted:

- the `github://` locator, its parsing, and browser-URL normalization;
- resolving a branch or tag to one immutable commit;
- enumerating a snapshot's entries and reading one entry's bytes;
- materializing a read-only graph artifact for federation;
- the provider protocol conversation over standard input and output;
- credential resolution by name, through an environment variable, an OS credential store,
  a protected key path, or a process-memory-only prompt.

Prohibited:

- a dependency on `nostdb-core`, `nostdb-cli`, or `nostdb-server`;
- any parser for `.nost` or `.nostdb`, and any writer for either;
- opening a database, or deciding what a retrieved file means;
- a second copy of the grammar or the conformance fixtures;
- a copy of the root PRD;
- a general action plugin surface. This is a provider, not a plugin manager;
- persisting a credential anywhere, in any form.

If something here appears to need one of the prohibited items, it needs a protocol message
instead. Add it to `provider_protocol_version` in `nostdb-spec` and implement that.

## Invariants this repository must never break

- **A provider returns bytes; only the Engine interprets them.** A provider that parsed
  `.nost` would be a second parser of a format that has exactly one.
- **A branch or tag resolves to one immutable commit before anything is enumerated or
  read**, and every request in one build or query uses that snapshot.
- **A query never silently advances a branch.** Only an explicit refresh records a newer
  commit.
- **The configured locator is the link's identity.** The resolved commit and content digest
  are operational metadata, not a target identity.
- **A credential travels as a name.** A raw credential never reaches a log record, a
  diagnostic, standard output, a cache, a file, or a locator.
- **A refusal is a reply, never an exit.** A caller that gets no reply cannot tell a version
  mismatch from a crash.
- **An unavailable source leaves the link declared.** It is not a build failure.
- **A cached snapshot that serves a request is reported as cached.**
- **`content_id` is the host's identifier and not a digest the Engine may trust.** Every
  downloaded artifact receives an independent cryptographic digest before it is opened.

## Rust standards

Rust stable and Edition 2024. Public APIs require explicit error types and rustdoc. Use
`#![forbid(unsafe_code)]` where practical; required `unsafe` code needs a separate ADR with
documented safety invariants and a Miri or equivalent verification plan before
implementation.

The library uses log records and never writes to stdout. The binary's stdout *is* the
protocol — one reply per line — which is exactly why nothing else may write there.

Every change must pass:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Do not add a dependency without documenting its purpose, maintenance status, and license.
The first HTTP client this crate takes is a decision worth arguing in the manifest.

## Repository verification

Run before every commit:

```bash
./scripts/verify-repository.sh
```

The verifier is non-mutating. Extend it as the provider lands rather than replacing it with
a manual checklist.

## Testing expectations

**No test may reach a network.** Every one runs against recorded fixtures and a fake
transport. This is not a limitation to be lifted later: a provider whose correctness can
only be shown by reaching a live third-party service is one nobody can verify in CI, and the
product contract requires behavior on a *cached* snapshot that a live-only test could never
exercise.

Each boundary carries its own coverage:

- the locator: canonical and browser forms, case rules, percent-encoding, a missing ref, and
  a credential embedded in a locator;
- the protocol: each request and reply, an unsupported version, a malformed request, and a
  reply whose declared length does not match what follows;
- snapshots: a branch resolving to a commit, a ref that does not exist, and a cached
  snapshot serving a request while the host is unreachable;
- credentials: a name that resolves, one that does not, and a host refusal. A test must
  never contain a real token, and the verifier rejects one that does.

## Safety and external actions

- Never execute retrieved content.
- Never place a credential, token, private key, or PEM content in a file, fixture,
  diagnostic, log record, or output.
- Do not create remote repositories, add remotes, push to a new remote, publish packages,
  create releases, or modify registries without explicit user authorization.
- Do not use destructive Git commands or broad deletion.
- Treat every retrieved byte as untrusted input. Bound allocation, file size, and entry
  count before reading anything a host supplied.

## Stage workflow

Implementation sequencing is tracked in the root `IMPLEMENTATION_PROGRESS.md`, not here. Do
not begin a later Stage during a setup-only request, and do not mark a Stage `DONE` until
every Acceptance Criterion passes.
