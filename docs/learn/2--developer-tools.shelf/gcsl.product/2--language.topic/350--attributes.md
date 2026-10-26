---
title: Attributes
type: concept
description: Tell the program that runs a function how to run it.
---

An attribute goes before a function and says how to run it, for example
whether a build step can be cached or how long it can take. GCSL checks
every attribute. Most are read by the tool that runs the program, such as
a build system, and GCSL itself acts on its own three, `retry`, `timeout`
and `track_caller`. The prelude declares two for builds: `hermetic`,
which lets a build tool such as pekit run a function's calls as cached
steps, and `scratch`, which gives such a step a directory kept between its
runs. Plain GCSL runs those calls at once, as any call.

```gcsl
#[hermetic]
fn compile(source: String) -> String { "compiled {source}" }

#[hermetic(false)] #[timeout(10m)]
fn deploy(target: String) -> String { "deployed {target}" }

result = deploy(compile("main.c"))
```

## Declaring an attribute

An attribute's arguments form a record, so you declare an attribute as a
record type marked `#[attribute]`. Each field is an argument, and a field
with a default is an argument you can leave out:

```gcsl
#[attribute] type device = {name: String, count: Int = 1}

#[device(name = "kvm")]
fn boot() -> String { "booted" }

booted = boot()
```

Arguments must be constants: literals, or lists and records of them. A
field can be an `Int`, `Float`, `Bool`, `String`, `Duration` or `Size`, or
a list or record of those. An attribute is declared, imported and named in
the same way as any other type, so a tool usually provides its attributes
in a module that you import.

## Using an attribute

Write `#[NAME]`, or `#[NAME(ARGUMENTS)]`, on the line before a function: a
`fn`, a `def`, or a definition made with a definition type. An attribute
with one field can take its argument by position, as in `#[timeout(10m)]`;
one with more fields takes its arguments by name. A misspelt attribute is
an error, so a mistake isn't silently ignored.

GCSL provides three attributes of its own:

| Attribute | Argument | Meaning |
|---|---|---|
| `#[retry(times)]` | `times: Int` | If the call fails, run it again, up to this many more times. |
| `#[timeout(limit)]` | `limit: Duration` | Fail the call if it runs for longer than this. |
| `#[track_caller]` | none | Report an error raised in the function's body at its call. |

A retried call runs its whole body again with the same arguments, so any
effects it has happen again: use `retry` only where that's safe. Retrying a
pure function can't change its result, so GCSL warns about it. When a call
times out, a program it's running is stopped. A function's own timeout
counts as a failure that its `retry` can retry, and each attempt gets the
full time. A timeout of an enclosing call is never retried by a call inside
it.

```gcsl
#[retry(2)] #[timeout(30s)]
fn fetch(url: String) -> String { exec(["echo", "fetched {url}"]).stdout }

page = fetch("https://example.com")
```

### Reporting errors at the call

A library function that checks its arguments, or wraps a command, fails
inside the library, where the reader of the error can do nothing about it.
Mark it `#[track_caller]`, and an error raised in its body is reported at
the call instead:

```gcsl
#[track_caller]
fn positive(n: Int): Int {
    if n <= 0 { panic("expected a positive number, got {n}") }
    n
}

size = positive(4)
```

A call of `positive(0)` would fail at that call, not at the `panic`. The
standard library marks `exec`, `exec_live`, `sh`, `sh_live`, and the
methods of `File`, `Dir` and `Sandbox` this way.

Only errors raised in the marked function's own body move. When it calls
another marked function, that call's error moves to the call in the body,
and then on to the outer call. An error from a function it calls that
isn't marked, or from a function value it was given, stays where it was
raised. A `fn` written inside the marked body counts as part of it.

You don't need the attribute for an error that leaves a library, or the
standard library, into your own code: that is always reported at your
call into it, with the line where it was raised shown beneath, marked
"failed here, in library code". This holds however deep in libraries the
error was raised, for a library's parameter defaults, and for a build
step a library makes that fails later. A function of your own that a
library calls back, and that fails, is reported where it failed.

## Attributes on a definition type

Attributes on a definition type apply to every definition made with it.
A definition's own attribute of the same name replaces the definition
type's:

```gcsl
#[attribute] type cached = {on: Bool = true}

#[def_type] #[cached]
def check(name: String, f: fn(): Bool): fn(): Bool { f }

def check fast() { true }
#[cached(false)] def check online() { true }

results = [(check fast)()(), (check online)()()]
```

Here `fast` is cached and `online` is not.

A deferring attribute (below) on a definition type works the same way:
it defers each definition, not the definition type itself. The
definition type's body calls the definition, and a host can run that call
as a step, while the body stays outside it. So the body can await the
step's result and recover its failure.

## Attributes that defer calls

A host such as pekit can run some calls as separate steps, possibly in
parallel or from a cache, and give their results later. An attribute that
asks for that is declared with `#[defers]` as well as `#[attribute]`, as
the prelude declares `hermetic`, `#[attribute] #[defers] type hermetic =
{on: Bool = true}`:

```gcsl
#[hermetic]
fn compile(source: String) -> String { "{source}.o" }

fn link(objects: List<String>): String { "app from {len(objects)} objects" }

app = link([compile("main.c"), compile("util.c")])
```

A call of a function with a deferring attribute can give a future instead
of its value. GCSL waits for it wherever the value is used, so nothing
about the code changes: plain GCSL runs each call at once, and under a host
that defers it, only code that uses the result waits. The function's type
shows it: `compile` is a `fn(source: String) -> String | Future<String>`.

Only an impure function can defer, and its result type must be able to be
a future, so `Present<String>` isn't allowed. For a definition type that
defers, its invocations are the deferred calls: a definition keeps the
definition type's function type.
