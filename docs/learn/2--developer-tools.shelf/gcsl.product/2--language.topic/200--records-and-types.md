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
