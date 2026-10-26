---
title: Context
type: concept
description: Pass settings such as the target platform through every call without threading them by hand.
---

Some values matter deep inside a program without every function in
between caring about them: the platform you are building for, for
example, or the toolchains available. A *context* is a named value of
that kind. You declare it once, read it with `^`, and set it for a call
and everything that call does with `with`:

```gcsl
context arch: String = "x86_64"

fn compiler(): String { "gcc-{^arch}" }
fn build(): String { "built with {compiler()}" }

native = build()
cross = build() with { arch = "aarch64" }
```

`build` never mentions `arch`, yet `cross` is built with `gcc-aarch64`:
the `with` reaches every call made inside it.

## Declaring a context

`context NAME: TYPE = DEFAULT` declares a context and its default, at the
top level of a file. Every file of the workspace can read it without an
import, so a name is declared only once. A context can leave out its
default, `context toolchains: List<Toolchain>`; reading it then needs a
`with` that sets it (see [What a function needs](#what-a-function-needs)).

A default is an expression, and it is evaluated where the context is
read, in the context of the read. So one context can default to another,
and follow it:

```gcsl
context exec_cpu: String = "x86_64"
context target_cpu: String = ^exec_cpu

target = ^target_cpu with { exec_cpu = "aarch64" }
```

## The platform contexts

The prelude declares two contexts for building, which every file can read:

- `host_platform` is where build steps run: in plain GCSL, the machine
  you run it on. A build tool such as pekit sets it to where it runs them.
- `platform` is what you're building for. Its default is
  `host_platform`.

Both are a `Platform`, `{arch: String, os: String}`, named as Rust names
them, such as `{arch = "x86_64"; os = "linux"}`:

```gcsl
fn triple(): String { "{^platform.arch}-{^platform.os}" }

native = triple()
arm = triple() with { platform.arch = "aarch64" }
```

A default must be pure: it can't call an impure function. To use a value
that needs one, compute it first and bind it with `with`.

## Setting a context with `with`

`EXPR with { NAME = VALUE }` evaluates `EXPR` with the context `NAME`
set to `VALUE`. It applies to the call or field access immediately before it,
so `a + f() with {...}` sets the context for `f()` alone. It must be on
the line where the expression ends. To set a context for several
statements, use the block form, `with { NAME = VALUE } { ... }`.

A dotted name sets one field of a record context and keeps the others:

```gcsl
type Board = {cpu: String, ram: Int}
context board: Board = {cpu = "x86_64"; ram = 8}

fn describe(): String { "{^board.cpu} with {^board.ram} GiB" }
small = describe() with { board.ram = 1 }
```

`with { arch }` is short for `with { arch = arch }`, setting the context
from a variable of the same name.

## Reading a context in a parameter

A parameter written `^NAME` takes its default from the context of that
name, so a caller can still pass a value explicitly. `fn f(^arch)` is the
same as `fn f(arch: String = ^arch)`, with the type taken from the
context's declaration:

```gcsl
context arch: String = "x86_64"
fn compiler(^arch): String { "gcc-{arch}" }

usual = compiler()
explicit = compiler(arch = "mips")
```

## What a function needs

A function that reads a context with no default, with no `with` of its
own around the read, *needs* that context: whoever calls it must set it.
GCSL works this out for you, through every function in between, and
checks each call before the program runs:

```gcsl
context arch: String

fn compiler(): String { "gcc-{^arch}" }
fn build(): String { "built with {compiler()}" }

cross = build() with { arch = "aarch64" }
```

Here `build` needs `arch` because `compiler` does. Calling `build()`
where no `with` sets `arch` is an error: "context `arch` has no value
here". A `with` inside a function sets the context for what it calls, so
that function needs nothing.

A function's type shows what it needs after its parameters:
`compiler` is a `fn(^arch): String`. A function passed as a value keeps
its needs, so calling it needs them too. A function type you write needs
only the contexts you write in it, in the same form, after its
parameters. So a function that needs a context can be passed where its
type says so, and calling it there needs the context in turn:

```gcsl
context arch: String

fn twice(f: fn(String, ^arch): String): String { "{f("a")} {f("b")}" }
fn tagged(name: String): String { "{name}-{^arch}" }
both = twice(tagged) with { arch = "x86_64" }
```

`twice` needs `arch` because it calls `f`, so the call of `twice` sets
it. Where a function type doesn't name a context, a function that needs
one can't be passed. Set the context inside the function instead:

```gcsl
context arch: String

fn twice(f: fn(): String): String { "{f()} {f()}" }
both = twice(fn(): String { ^arch with { arch = "x86_64" } })
```

Name only contexts with no default: one with a default needs nothing.
A function that reads a context through another's default, `fn(^host |
^target)`, fits a type that names the context with no default,
`fn(^host)`. A declared `type` can't name a need yet, since a declared
type is resolved in each file that uses it: write the function type
where you use it.

A context with a default needs nothing, but its default can read another
context that has none. Then the function needs that one, unless the
caller sets the context with the default, which is then never evaluated.
Its type shows both: `fn(^host | ^target): String`.

What a call needs is what runs where it is called. A `^NAME` parameter's
default is evaluated by the caller, so `fn compiler(^arch)` needs
nothing itself: `compiler()` needs `arch`, and `compiler(arch = "mips")`
doesn't. A `with` around where a function is written doesn't count,
because the function runs in the context of each call. A `lazy` is the
other way round: it needs what its body needs where it is made.

A definition is called by its def-type, which sets no context, so a
definition needs only what its def-type's function parameter names:
with `def script(name: String, f: fn(^session) -> {}) -> {}`, a script
can read `^session` and call other scripts, and anything that calls a
script needs `session` too. A definition of a def-type whose parameter
names nothing, such as `build`, can't need a context, so a build can't
call a script. A function used inside its own recursion
before GCSL knows what it needs, from a function nested in it or as a
value, is an error if it turns out to need a context there. Call it
directly, or set the context around the call.

## Context and deferred work

A `lazy` keeps the context it was made in. Wherever it is later
awaited, it reads the values that were set where it was written. A
top-level field is computed when its file runs, so it reads the default
values.

Memoised functions stay correct: a memoised call's result is reused only
for a call made in a context that gives every context the call read the
same value.
