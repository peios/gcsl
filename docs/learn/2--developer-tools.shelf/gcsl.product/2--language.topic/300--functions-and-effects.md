---
title: Functions and Effects
type: concept
description: Choose pure, impure, or memoized functions and understand captures.
---

A function's signature says what it accepts, what it returns, and whether
calling it can perform effects. This makes executable configuration
reviewable: a pure calculation cannot silently run a process or read a
file.

## Choosing a function form

Use `fn` for a private function and `def` for a function exposed as a
field. Both forms declare parameter and result types. A function
expression can also be assigned to a variable or passed to another
function.

The marker before the result type selects the effect:

| Marker | Meaning |
|---|---|
| `:` | Pure computation; no effects or awaiting |
| `->` | Impure computation; effects and awaiting are allowed |
| `->>` | Impure computation memoized by its arguments |

This private pure function accepts a named argument and a default:

```gcsl
fn scale(value: Int, factor: Int = 2): Int { value * factor }
scale(value = 21)
```

Defaults are constants. Positional and named arguments are evaluated in
source order. A trailing block supplies the last parameter, which is
useful for functions that accept a configuration record.

After a named argument, a name on its own is shorthand for passing the
variable of that name to the parameter of that name. These two calls are
the same:

```gcsl
fn build(source: String, output: String, compiler: String): String {
    "{compiler} {source} -o {output}"
}
let compiler = "cc"
short = build("main.c", output = "main.o", compiler)
long = build("main.c", output = "main.o", compiler = compiler)
```

Before any named argument, a name on its own is an ordinary positional
argument.

## Calling for effects

A call to an impure or memoized function can stand alone as a statement,
outside a pure function, to run it for its effects. Its value is
discarded, whatever it is, and anything it defers is awaited first, so no
effect is lost:

```gcsl
fn build(source: String) -> String {
    sh("make -C {source}")
    sh("make -C {source} install")
    = source
}
```

Any other value standing alone is an error, because discarding it would
do nothing: a pure expression has no effects to keep. Make it the block's
result with `= ...`, bind it with `let`, or remove it.

## Captures and recursion

Closures hold copies of the values they capture. Rebinding a variable
later doesn't change an existing closure. An inner function cannot rebind
an outer function's variable.

Top-level named functions are available regardless of textual order and
can refer to each other within the file. Workspace files still need an
acyclic dependency order: functions in two files cannot make those files
need each other. Put mutually dependent definitions together or extract
their shared dependencies.

## Memoization

A memoized function reuses a successful result for the same arguments in
the same execution context. The cache belongs to the closure, so two
closures created from one function expression have separate entries.
Memoization also accounts for supported ambient inputs such as a caller
path when the function reads it.

This is not a persistent build cache. External changes to files or a
command's environment do not invalidate a stored result. Use `->>` only
when reusing the first result within that run is correct. Cached keys and
results consume memory for the lifetime of their execution context.

Read [Deferred Work](~gcsl/deferred-work) before using futures in function
signatures; a bare type's present-or-future meaning depends on its
position and the function's effect.
