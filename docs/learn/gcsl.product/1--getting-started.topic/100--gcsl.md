---
title: GCSL
type: concept
description: A language for typed configuration values and executable scripts.
---

GCSL is a statically typed scripting language. A program can describe a
configuration record, compute a value, or run commands. You use the same
expressions for all three, with function types separating pure computation
from operations that can change the outside world.

The `gcsl` command runs programs, checks them without execution, formats
source files, and serves editor requests. Starting it without a command
opens an interactive read-evaluate-print loop (REPL).

## Values first

A file is a block of declarations and expressions. Named fields form its
resulting record. This configuration computes one field from another:

```gcsl
name = "gcsl"
version = "0.1.0"
archive = "{name}-{version}.tar.gz"
```

A file can instead return one expression. Use `let` for private values that
don't become fields of a record. The checker catches incompatible types and
unresolved names before any part of the program runs.

## Execution and trust

Pure functions use `:` before their result type. Impure functions use
`->`; they can run processes, access files, and await deferred work.
Memoized functions use `->>` and reuse results within an execution context.
A `lazy` expression delays work until it is awaited. It doesn't start a
background task.

GCSL's type system is not a sandbox. Running a script gives its process and
file operations the permissions of the process running GCSL. Check an
unfamiliar program with `gcsl check` before deciding whether to run it.
Checking does not establish that executing it is safe.

## Where to go next

Start with [Running a First Program](~gcsl/running-a-first-program), then
read [Values and Bindings](~gcsl/values-and-bindings). Use `gcsl(1)` for the
command interface, `gcsl(5)` for language rules, and `gcsl-stdlib(3)` for
standard functions.

Pekit embeds GCSL to provide its own build interface. Its definitions and
commands belong to Pekit. The separate `tree-sitter-gcsl` repository owns
its editor grammar and queries; the compiler parser in this repository is
the authority for whether a GCSL program is accepted.
