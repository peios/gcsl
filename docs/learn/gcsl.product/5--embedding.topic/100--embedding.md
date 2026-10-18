---
title: Embedding GCSL
type: concept
description: Choose a language-library entry point and preserve its execution requirements.
---

The `gcsl` Rust crate exposes the language pipeline without requiring the
command-line program. You can evaluate one source, retain an interactive
session, or load a workspace supplied by your application.

## Choosing the boundary

The repository separates four responsibilities:

| Crate | Responsibility |
|---|---|
| `gcsl` | Syntax, types, evaluation, formatting, and semantic indexing |
| `gcsl-disk` | Finding workspace roots and reading source trees |
| `gcsl-lsp` | Editor protocol, document state, and request handling |
| `gcsl-cli` | Command arguments, terminal interaction, and output |

`gcsl::run` evaluates a `SourceFile` as a standalone program. A `Session`
retains names and values across inputs. `workspace::Workspace` evaluates
selected files after a workspace check. The lower-level lexer, parser,
checker, and index are available when you only need a particular stage.
Consult the Rust API documentation for exact signatures and ownership.

## Stack and lifetime

Run the language pipeline on a thread with at least `gcsl::STACK_SIZE`
bytes of stack, as the CLI does. Its configured size is 64 MiB. Parsing,
checking, lowering, and evaluation share documented nesting limits; a host
thread's default stack is not a substitute for the required reservation.

Keep the source collection and diagnostic input numbering together.
Standard-library files participate in the source list, so diagnostics can
point into a helper as well as into user text. The library returns
warnings with successful results instead of printing them.

A workspace or session owns memoization state and temporary directories.
Dropping it performs normal temporary-directory cleanup. Do not return a
path to such a directory and then destroy its owner before a consumer has
used it.

## Effects are host effects

The standard library is embedded in the binary. Its process and file
operations still run with the embedding process's permissions and
resources. The library does not impose a wall-clock limit or turn pure
checking into an execution sandbox. A host that accepts untrusted programs
needs an external isolation and resource policy.

Use `gcsl-disk` when you want the CLI's workspace-discovery behavior.
Supply sources yourself when your host has a different storage model;
keep the language's dependency and scope rules intact.
