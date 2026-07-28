# nostdb-provider-github

The NostDB GitHub source and graph-store provider: an out-of-process executable that
retrieves bytes and metadata from GitHub over the versioned provider protocol.

**Status: scaffolding.** The protocol is specified; no request is implemented yet. See the
root [`IMPLEMENTATION_PROGRESS.md`](https://github.com/nostdb/nostdb/blob/main/IMPLEMENTATION_PROGRESS.md)
for which increment builds which part of it.

## What it does

Three questions, and no others:

- what does this `github://` locator resolve to?
- what does that snapshot contain?
- what are the bytes of one entry?

A branch or tag is resolved to one immutable commit before anything is enumerated or read,
so every request in a build or query sees one consistent snapshot.

## What it does not do

It does not interpret what it retrieves. Only `nostdb-core` reads `.nost` or `.nostdb`, and
this crate does not depend on the Engine — the repository verifier rejects that dependency.

It does not decide what is interesting, does not analyze, and reports no graph.

## Why it is a separate process

Not for performance. This is the component that holds a credential and talks to a network,
and the one most likely to be replaced by something a third party wrote. Behind a process
boundary it cannot read the Engine's memory, cannot reach a database handle, and cannot
outlive the request it was started for.

## Credentials

A request carries a credential *name*, never a secret. The provider resolves the name
through an environment variable, an OS credential store, a protected key path, or a
process-memory-only prompt.

The Engine never holds the secret, so it cannot leak one it never had. A raw credential must
not reach a log record, a diagnostic, standard output, a cache, a file, or a locator, and
the repository verifier rejects a token literal committed here.

## The protocol

Line-delimited JSON over standard input and output, specified in
[`nostdb-spec/docs/PROVIDER_PROTOCOL.md`](https://github.com/nostdb/nostdb-spec/blob/main/docs/PROVIDER_PROTOCOL.md)
under `provider_protocol_version`. Binary content does not travel inside JSON: a `read`
reply names a length and the bytes follow the newline.

## Verification

```bash
./scripts/verify-repository.sh
```

No test reaches a network. Every one runs against recorded fixtures and a fake transport,
which is what makes the suite runnable in CI and is the only way to exercise the
cached-snapshot behavior the product contract requires.

## Licence

Apache-2.0. The provider tier is permissive deliberately: a provider is the component a
third party is most likely to write.
