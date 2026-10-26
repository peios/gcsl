---
title: Evaluation State
description: Understand values, futures, memo tables, and failure cleanup.
---

GCSL evaluation uses reference-counted values and explicit execution
contexts. A closure captures values; a frame holds the current call's
slots. This supports value semantics without exposing shared mutable
variables to programs.

## Futures and memo tables

A future begins pending, becomes running during evaluation, and records a
successful result. Reentering running work is an error rather than an
infinite cycle through the same pending computation. A failed evaluation
does not become a cached success.

Memo tables belong to the execution context and are keyed by closure
identity and argument values, with supported ambient inputs included when
used. Keeping tables outside closures avoids introducing reference cycles
through cached keys and results. Successful entries for live closures
remain until their context ends; they are not an eviction-based cache.

## Exits and cleanup

Runtime errors and language control flow share an unwinding path through
the evaluator. A loop handles its own `break` or `continue`; a function,
future, or labeled block handles a matching `return`. Intermediate
operations restore stack and evaluation state before propagating an exit.

A failed REPL input does not commit its new bindings or memo-table changes
to the session. Effects already performed outside the interpreter are not
rolled back. A file written or a subprocess started before an error still
requires the host's ordinary handling.

A workspace remembers the first runtime failure of each file. A later
request that needs that file receives the original diagnostic without
rerunning its effects or starting dependent files. Successful prior files
keep their values, and unrelated files can still run. Create a new
workspace object when you intentionally need to retry evaluation.

## Resource limits

The parser's maximum expression depth is `gcsl::MAX_DEPTH`, 10,000 levels.
Runtime expressions, calls, and awaits share a nesting budget; calls cost
additional levels. Iterations reuse their body's depth rather than adding
one level per iteration. Tail calls are not eliminated.

These limits protect the recursive implementation, not total computation.
A program can still loop indefinitely, allocate large values, capture
large process output, or retain many memoized results. The embedding host
owns any execution timeout and memory limit.

The main implementation lives in `eval.rs` and the modules under `eval/`,
`value.rs`, and `session.rs`.
