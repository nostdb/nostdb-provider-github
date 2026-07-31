#!/usr/bin/env bash

# Non-mutating verification for nostdb-provider-github.
#
# Covers the repository shape, the ownership boundaries in AGENTS.md, the local-only transport
# invariant, the library's stdout boundary, and the Rust command set.

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

cd "$repository_root"

required_files="
AGENTS.md
CLAUDE.md
README.md
LICENSE
.gitignore
.editorconfig
.github/workflows/verify.yml
Cargo.toml
Cargo.lock
rust-toolchain.toml
src/lib.rs
src/main.rs
"

for required_file in $required_files; do
  if [ ! -e "$required_file" ]; then
    echo "missing required file: $required_file" >&2
    exit 1
  fi
done

# LICENSE is verbatim upstream text and is intentionally not whitespace-scanned.
checked_text_files="
AGENTS.md
README.md
.gitignore
.editorconfig
.github/workflows/verify.yml
Cargo.toml
rust-toolchain.toml
scripts/verify-repository.sh
"

for checked_file in $checked_text_files; do
  if grep -nE '[[:blank:]]+$' "$checked_file"; then
    echo "trailing whitespace found in: $checked_file" >&2
    exit 1
  fi
done

if [ ! -L CLAUDE.md ] || [ "$(readlink CLAUDE.md)" != "AGENTS.md" ]; then
  echo "CLAUDE.md must be a symlink to AGENTS.md" >&2
  exit 1
fi

# The provider is Apache-2.0, not SSPL. The root contract puts providers and extension
# schemas in the permissive tier deliberately: a provider is the component a third party is
# most likely to write, and a copyleft licence on the protocol side would discourage exactly
# the implementations the product wants to exist.
if ! grep -q '^ *Apache License$' LICENSE; then
  echo "LICENSE must be the Apache License, which the provider tier requires" >&2
  exit 1
fi

if ! grep -q '^ *Version 2.0, January 2004$' LICENSE; then
  echo "LICENSE must be Apache License version 2.0" >&2
  exit 1
fi

# A provider retrieves bytes and metadata; only nostdb-core interprets .nost or .nostdb.
# Linking the Engine would make parsing either possible by accident, and a second parser of
# a format that has exactly one is how two answers to one question start. This runs now
# rather than with the first real code, so the dependency cannot arrive quietly.
if [ -f Cargo.toml ] && grep -nE '^nostdb[-_](core|cli|server) *=' Cargo.toml; then
  echo "a provider must not link the Engine; it returns bytes and Core interprets them" >&2
  exit 1
fi

# The same boundary from the other side. A provider that opened a database, or wrote one,
# would be doing the Engine's job with none of the Engine's invariants.
#
# The pattern matches Engine API calls, not paths. An earlier version also rejected any
# mention of a `.nostdb` path and fired on a test fixture — which was not merely noisy but
# wrong: naming a `.nostdb` file is exactly what a graph locator does, and it is this
# provider's job in the graph_store role. A check that forbids the thing the component is
# for is one people learn to work around.
if [ -d src ] && grep -rnE '\b(open_database|commit_graph|read_graph|Database::)' src; then
  echo "a provider must not open or write a database" >&2
  exit 1
fi

# Section 15.3 forbids a raw credential reaching any stored or printed place. A provider is
# the component that holds one, so this is the repository where an accidental literal is
# most likely and most costly. The pattern catches the shapes GitHub tokens actually take.
if [ -d src ] || [ -d tests ]; then
  leaked=$(
    grep -rnE '\b(gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,})' src tests 2>/dev/null || true
  )
  if [ -n "$leaked" ]; then
    echo "a credential must never appear in this repository" >&2
    printf '%s\n' "$leaked" >&2
    exit 1
  fi
fi

# AGENTS.md requires the library to use log records rather than writing diagnostics to stdout.
# The binary legitimately writes: its stdout *is* the protocol, one reply per line. The library is what must stay quiet, because a caller parsing the binary's stdout must
# not have library chatter interleaved into it.
#
# main.rs is excluded by name rather than by directory, so a second binary added later is
# excluded deliberately instead of by accident.
if [ -d src ]; then
  noisy=$(
    find src -name '*.rs' ! -name 'main.rs' -exec grep -nE '\b(println!|print!)' {} + || true
  )
  if [ -n "$noisy" ]; then
    echo "the library must not write to stdout; use a log record instead" >&2
    printf '%s\n' "$noisy" >&2
    exit 1
  fi
fi

# A parser, storage engine, or query engine here would be a second implementation of
# behavior the product contract defines once.
if [ -e grammar ] || [ -e fixtures ]; then
  echo "the grammar and the conformance fixtures belong to nostdb-spec" >&2
  exit 1
fi

if [ -e docs/PRD.md ]; then
  echo "the PRD lives once, in the root superproject" >&2
  exit 1
fi

# There is deliberately no "the Engine dependency is pinned" check here, which every
# sibling repository has. This one forbids the dependency outright above, and a pinning rule
# beside a prohibition would read as permission to add it as long as the pin is right.
# The Rust command set the root contract requires of every Rust repository, which this one did not run.
#
# `--locked` is the point of adding it now. The release workflow builds this provider with `--locked`, and
# nothing here built at all — so a `Cargo.toml` version bumped without its `Cargo.lock` passed every local
# gate and failed all four targets of a release, which is the most expensive place to learn it.
if [ -f Cargo.toml ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to verify the provider" >&2
    exit 1
  fi
  cargo fmt --check
  cargo check --locked --all-targets --all-features
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-targets --all-features
fi

echo "nostdb-provider-github verification passed"
