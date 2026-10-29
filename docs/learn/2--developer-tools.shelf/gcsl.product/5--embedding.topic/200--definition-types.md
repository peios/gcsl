---
title: Definition Types and Metadata
type: concept
description: Build domain-specific declarations without changing the GCSL parser.
---

A definition type lets a host or library describe a family of named
functions. The language checks each definition against a shared signature,
while the host can discover and invoke definitions by name. This is the
mechanism used by integrations such as Pekit.

## A family of definitions

Mark a public function with `#[def_type]`. Its first parameter receives the
definition's path as a string; its second receives the definition function.
Further parameters become invocation arguments. This example wraps a
pure string-producing definition:

```gcsl
#[def_type]
def describe(name: String, body: fn(): String): String {
    "{name}: {body()}"
}
def describe dev.peios.gcsl() { "language tools" }
result = describe dev.peios.gcsl()
```

Declaring the definition does not execute its body. Calling its reference
does. Definitions live in the definition type's namespace, separate from
ordinary values of the same path. Their public paths merge through
workspace directories and imports like other public declarations.

## Parameters of a definition's own

A definition's parameters are written by name alone: their types come
from the definition type's function parameter. After them, a definition
can declare parameters of its own, each with a type and a default, since
the definition type calls it with none:

```gcsl
#[def_type]
def describe(name: String, body: fn(): String): String {
    "{name}: {body()}"
}
def describe dev.peios.gcsl(version: String = "1.0") { "language tools {version}" }
result = describe dev.peios.gcsl()
```

The defaults are evaluated where the definition type calls the
definition, and the body is then run with what they came to. So a
deferring attribute on the definition, its own or its definition type's,
makes the body the step and gives it those values: a test that is given
its program, `def test runs(prog: File = build app())`, is a step given
the built program, and reruns only when the program changes.

## Importing every definition of a type

A definition is visible in its own directory and the directories below
it. To use many definitions from elsewhere without one import each, import
every definition of one type from a set of directories:

```text
workspace.gcsl          # declares the definition type `package`
pkgs/net.busybox.busybox/build.gcsl   # def package net.busybox.busybox()
pkgs/dev.peios.kernel/build.gcsl      # def package dev.peios.kernel()
image/build.gcsl        # import package * from pkgs/*
```

`import package * from pkgs/*` brings in every `package` definition from
each directory directly inside `pkgs/`, so `image/build.gcsl` can refer to
`package dev.peios.kernel` and `package net.busybox.busybox` by name. The
end of the path says which directories it reads:

| Path | Directories read |
| --- | --- |
| `pkgs` | `pkgs/` itself, as `import * from pkgs` would |
| `pkgs/*` | every directory directly inside `pkgs/` |
| `pkgs/**` | every directory at any depth below `pkgs/` |
| `*` or `**` | the same, below the workspace's root |

Only the definitions of that type are imported, nothing else from the
files. Hidden directories, whose names start with `.`, are skipped, and so
is everything below them. If two of the directories define the same name,
that's an error at the import, naming both definitions, so a new package
can never silently replace another. Finding no definitions at all isn't
an error.

## Definitions made with a variant

A definition type can take a variant of an [enum](~gcsl/records-and-types)
after the name, and the definition after that. Each definition then
names a variant, and each variant has its own definitions. When the
definition type is generic, the variant's payload type fills in its type
parameter, so each definition's body is checked against its own
variant's payload:

```gcsl
enum Tool extensible<{*: String}>
enum Tool::C<{cc: String}>
enum Tool::Rust<{rustc: String}>
#[def_type]
def Tool<T>(name: String, kind: Variant<Tool, T>, body: fn(): T): Tool {
    kind(body())
}
def Tool::C gcc() { cc = "gcc" }
def Tool::Rust gcc() { rustc = "rustc" }
c = Tool::C gcc()
rust = Tool::Rust gcc()
```

`Tool::C gcc` and `Tool::Rust gcc` are separate definitions. A reference
names the variant in the same way as the definition, and the definition
type receives the variant as its `kind` argument. Hosts don't invoke these
definitions by name: programs reach them through references, for example
in a list of registered toolchains.

To import such definitions, name the variant, or import every variant's:

```text
import Tool::C * from "github.com/example/tools"
import Tool::* from "github.com/example/tools"
```

The first brings in only the `Tool::C` definitions, the second every
`Tool::C`, `Tool::Rust` and any other variant's. A variant the library
declares itself, such as a `Tool::Go` of its own, comes with them, so
you can write `Tool::Go gc` after importing it. Without a variant,
`import Tool * from ...` would find nothing, so it's an error, as is a
variant for a definition type that takes none.

## Importing one definition

Name a definition in place of the `*` to import it and nothing else:

```text
import build greet from app/greet
import Tool::C gcc from "github.com/example/tools"
```

The file then sees `build greet` and `Tool::C gcc`, but no other
definition of those types from there. A path that ends in a glob imports
the definition from whichever directory defines it, and finding it in
none is an error.

## Definitions of any result

A definition type can be generic without a variant. Each definition's
body then fills in the type parameter, so definitions of one type can
give different results:

```gcsl
#[def_type]
def made<T>(name: String, f: fn(): T): {name: String, made: T} {
    name = name
    made = f()
}
def made count() { 3 }
def made label() { "three" }
n: Int = (made count)().made
s: String = (made label)().made
```

The type parameters must appear in the definition parameter's result,
`fn(): T` here, since that's the only thing that binds them.

The prelude's `build` is a definition type of this kind,
`def build<T>(name: String, f: fn() -> T) -> T`, so every file can write
`def build hello() { ... }` with no import, and give whatever it builds.
A workspace that declares its own `build` uses its own.

The prelude's `package` is one too, for packages of any format. Its body
gives a version and files, each at its path under `/usr`, placed as
`fs.compose_dir` places them, and the definition's path is the package's
name:

```text
def package dev.example.greet() {
    version = "1.0-1"
    files = {"bin/greet" = build app()}
}
```

`package dev.example.greet()` is then a `Package` record of the name, the
version, and the files, which a packager library, such as gcslib's `ppk`,
packs.

The prelude's `test` is another. A test passes if its body succeeds and
fails if it fails, by an `assert`, a program exiting with an error, or
any other failure:

```gcsl
def test adds() {
    assert 1 + 1 == 2
}
result = test adds()
```

`test adds()` is a `TestResult`, `{name = "adds"; passed = true; message
= ""}`, and never itself a failure. A failing test's message holds the
failure's message, then each of its notes on a line of its own, such as a
failing program's output. `test` is `#[hermetic]`, so a build
tool such as pekit runs each test's body as a step, given what its
parameters' defaults build, and doesn't run a test that passed again for
the same inputs.

The prelude's `script` is for tasks a person runs, such as deploying or
booting what was built, whose effects change the world outside the build:

```text
def script deploy() {
    let ran = exec(["rsync", "-a", (build site()).path, "host:/srv"])
}
```

Its function parameter is `fn(^script_session) -> T`, and
`script_session` has no default, so a script can read `^script_session`
and call other scripts. Only the tool that runs a script binds the
context, for example `pekit run deploy`, so nothing a build, a test or a
package calls can call a script: that fails its check. A script builds
what it calls as any build does. `^script_session.args` holds the words
given after the script's name.

## Metadata

Function types can carry metadata with `META:`, `META->`, or `META->>`.
These distinguish pure reads, impure reads, and memoized reads. A
`defmeta` declaration supplies the metadata of a definition in its
directory. Reading a field through the function reference reads that
metadata according to its effect.

Metadata is not a separate package-format contract. The definition type
chooses its shape, and the host integration owns what those fields mean.
Use the host's documentation for package or build semantics.

## Host invocation

A checked workspace exposes definition types and their definitions. Host
APIs can invoke a selected definition with named textual arguments parsed
against its declared parameter types. When a value the host gets back holds
a reference, such as `build = build busybox`, the host can ask which
definition it names and then invoke it, or read its metadata, by name. The
host can also invoke a definition in a [context](~gcsl/context) of its
choosing, such as the platform a build step runs on, by giving each
context a value of its declared type, for example one it got back from
an earlier run. Only that invocation sees those values: what each file
computes as it runs uses the declared defaults. The
`gcsl` executable itself exposes
`run`, `check`, `fmt`, and `lsp`; it does not add a general command for each
definition type.

Keep protocol or package policy in the integration repository. Keep
language syntax, type checking, and discovery semantics in GCSL. This
separation lets other hosts use the same language without inheriting
Pekit's packaging model.
