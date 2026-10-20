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
against its declared parameter types. The `gcsl` executable itself exposes
`run`, `check`, `fmt`, and `lsp`; it does not add a general command for each
definition type.

Keep protocol or package policy in the integration repository. Keep
language syntax, type checking, and discovery semantics in GCSL. This
separation lets other hosts use the same language without inheriting
Pekit's packaging model.
