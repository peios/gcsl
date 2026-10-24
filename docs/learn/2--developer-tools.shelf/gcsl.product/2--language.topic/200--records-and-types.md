---
title: Records and Types
type: concept
description: Compose structural records and apply annotations and defaults.
---

A record type describes the fields a value supplies. GCSL uses structural
types: two declarations with the same structure describe the same type,
and a record can be used where fewer of its fields are required.

## Declaring a shape

Use `type` to give a structure a reusable name. An annotation checks a
value against that structure. Defaults supply omitted fields in a record
literal checked against the type:

```gcsl
type Install = {from: String, exclude: List<String> = []}
let item: Install = {from = "src"}
item.exclude
```

Defaults are constant expressions, not arbitrary function calls. They are
applied where a record literal has the expected type; they don't add
fields to an already constructed variable with a smaller type.

An annotation also determines what fields later code can see. Extra
fields in a record literal produce a warning when an expected type hides
them. A structural alias does not introduce a distinct runtime identity.

## Generic types

A type can take type parameters, named in angle brackets after its name.
Each use names the types to put in their place:

```gcsl
type Pair<A, B> = {first: A, second: B}
let entry: Pair<String, Int> = {first = "port"; second = 8080}
entry.second
```

A use must give as many type arguments as the type has parameters, and a
type without parameters takes none. Because types are structural,
`Pair<String, Int>` is the same type as `{first: String, second: Int}`. A
generic type can be built from another, as in
`type Named<T> = Pair<String, T>`. It can't refer to itself, and it can't
spread one of its parameters, because the parameter might not be a record.

## Records of any fields

A rest-field, `*: T`, gives the type of every field that a record type
doesn't name. Use it when the field names are data, such as paths. A
`{*: String}` holds any fields, each a `String`. Named fields keep their
own types, so `{to: String, *: Int}` holds a `String` field `to`, and any
other fields are `Int`s:

```gcsl
let files: {*: String} = {"bin/*" = "bin/*"; "sbin/*" = "sbin/*"}
files
```

Write the record literal where the type is expected: in an annotation,
an argument or a definition's result. There, each field that the type
doesn't name is checked against the rest-field. A value whose own type
has no rest-field doesn't fit one, because a record keeps fields that its
static type hides, and they could be of any type.

Read a named field as usual. A field that only the rest-field covers
can't be read by name, and a record whose type has a rest-field can't be
spread. Typed equality compares every field, and `is` and `as` test each
field that the rest-field covers.

## Combining fields

Dotted definitions construct nested records. Compatible dotted paths
merge, including across files in a workspace:

```gcsl
server.host = "localhost"
server.port = 8080
server.tls.enabled = true
```

A whole-field definition such as `server = {...}` closes that field to
further dotted extensions. Define the individual dotted leaves when you
want several files to contribute to a shared structure.

A spread starts from another record's visible fields. Inside an ordinary
record block, put defaults first and explicit overrides afterward:

```gcsl
let defaults = {host = "localhost"; port = 80}
let config = {
    ...defaults
    port = 8080
}
config.port
```

A spread copies only fields visible through its operand's static type.
It cannot replace an earlier explicit field inside an ordinary block.
Top-level declarations are resolved across the whole input and take
precedence over a spread of the same name regardless of textual order.

## Unions and narrowing

A union, such as `Int | String`, allows more than one shape. Use `is` to
test a value and `as` for a checked conversion that fails at runtime if
the value doesn't fit. In `if name is Type`, the branches see narrowed
copies of the named value. This does not perform general flow analysis of
later statements or dotted field paths.

Typed equality `==` compares operands of the same static type. Total
equality `===` compares complete runtime records, including fields hidden by
their shared static type. Both require the same static operand type. The
language's
restrictions on unions, functions, and futures are described in
`gcsl(5)` and [Inference Limits](~gcsl/inference-limits).
