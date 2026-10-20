---
title: The Language Pipeline
description: Follow source through checking and type-specialized evaluation.
---

GCSL is a tree-walking interpreter with a typed intermediate
representation. The stages are separate so a formatter or editor can stop
before execution while using the same syntax and type rules as the CLI.

The main pipeline is:

```text
SourceFile
    -> lexer and interned names/strings
    -> parser and arena-based syntax tree
    -> workspace resolution and type-check side tables
    -> type-specialized intermediate representation
    -> evaluation and a value with warnings
```

The lexer and parser can recover enough structure to report multiple
errors. A failed check is never lowered into executable code. Recovery
supports diagnostics and editor queries; it does not make a partially
valid program executable.

The checker resolves names and types expressions, recording captures,
slots, argument destinations, and required awaits in side tables.
Lowering makes those awaits explicit. Evaluation dispatches through
specialized operations whose types have already been checked, rather than
repeating the type system at each operation.

## Workspace scheduling

The workspace scanner merges public field paths, resolves imports, and
orders source files by dependencies. Each public leaf has an owner and a
cell. Reading an interior record can require all its contributing owners.
A cycle is a diagnostic, not an order selected at runtime.

Top-level field scheduling differs from ordinary block order. It supports
forward references while ensuring dependencies are initialized before a
read. Shadowed declarations are still checked even when they do not
contribute a runtime value.

## Sources

The principal implementation is in `crates/gcsl/src/lexer.rs`,
`parser.rs`, `workspace.rs`, `check.rs`, `lower.rs`, and `eval.rs`.
Each module documents its internal invariants. The implementation here is
not the separate Tree-sitter grammar used for highlighting.
