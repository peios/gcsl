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
`with` that sets it, and reading it without one is an error.

A default is an expression, and it is evaluated where the context is
read, in the context of the read. So one context can default to another,
and follow it:

```gcsl
type Platform = {arch: String, os: String}
context host_platform: Platform = {arch = "x86_64"; os = "linux"}
context platform: Platform = ^host_platform

target = ^platform.arch with { host_platform = {arch = "aarch64"; os = "linux"} }
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
type Platform = {arch: String, os: String}
context platform: Platform = {arch = "x86_64"; os = "linux"}

fn triple(): String { "{^platform.arch}-{^platform.os}" }
riscv = triple() with { platform.arch = "riscv64" }
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

## Context and deferred work

A `lazy` keeps the context it was made in. Wherever it is later
awaited, it reads the values that were set where it was written. A
top-level field is computed when its file runs, so it reads the default
values.

Memoised functions stay correct: a memoised call's result is reused only
for a call made in a context that gives every context the call read the
same value.
