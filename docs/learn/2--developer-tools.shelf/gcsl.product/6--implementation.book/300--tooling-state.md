---
title: Tooling State
description: Understand source discovery, editor snapshots, and formatter boundaries.
---

GCSL tooling shares language analysis while keeping disk access and editor
protocol handling outside the core. This prevents checking and formatting
from depending on an evaluator run.

## Source discovery

`gcsl-disk` finds workspace markers and returns a source collection with
stable path and diagnostic mappings. It excludes hidden names, nested
workspace trees, and paths the root's `.gcslignore` names when reading one
workspace. The CLI and language server
use this shared layer rather than implementing separate import rules.

A check of a directory discovers workspaces. A format of a directory
finds source files, including standalone files. Their traversal results
are therefore intentionally different.

## Editor snapshots

`gcsl-lsp` overlays open document text on disk sources and asks a worker to
check the affected workspace. Queries use an up-to-date semantic index;
they can force a pending check before answering. Diagnostics continue to
refer to actual source even when a query temporarily repairs unmatched
braces to find usable names.

Rename computes proposed edits and checks their effect before offering
them. Standard-library, file, and directory identities are not editable
through that operation. Formatting returns edits from the language's
formatter rather than applying the editor's formatting preferences.

Only local `file:` document URIs with an empty or `localhost` authority
and an absolute decoded path map to disk files. Other URI forms are
checked as standalone documents, not treated as local workspace paths.

## Formatter

The formatter works from tokens, comments, and parsed structure. It does
not need evaluation or successful type inference. Syntax errors prevent a
rewrite of that file. The CLI uses a temporary file and replacement so
readers don't observe a partially written formatted file.

The principal code is in `crates/gcsl-disk/src/lib.rs`,
`crates/gcsl-lsp/src/`, `crates/gcsl/src/index/`, and
`crates/gcsl/src/fmt.rs`. The CLI adapters live in
`crates/gcsl-cli/src/`.
