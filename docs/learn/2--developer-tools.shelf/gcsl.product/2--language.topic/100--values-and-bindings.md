---
title: Values and Bindings
type: concept
description: Understand block results, records, local names, and value updates.
---

GCSL programs compute values. A block can return one value or construct a
record of named fields, so you can use configuration data inside ordinary
expressions without a separate data format.

## Blocks and records

Newlines or semicolons separate items inside a file or `{ ... }` block.
A block without fields returns its final expression, or an explicit
`= expression` result. A block with fields returns their record instead;
it cannot also have an explicit result. The empty record `{}` represents
an operation with no useful result.

A field is a public part of the resulting record. A `let` binding is local
to its scope. This block keeps the intermediate name private:

```gcsl
let release = "0.1.0"
name = "gcsl"
archive = "{name}-{release}.tar.gz"
```

Top-level fields and named functions can refer to declarations written
later. Inner blocks evaluate their ordinary fields in order. Private
`let` bindings become available after their declaration.

## Updating a binding

`:=` replaces the value associated with a local name. For an ordinary
`let`, this is restricted to the block that declares it. Use `let mut`
when an inner block of the same function needs to update the variable:

```gcsl
let mut total = 0
for cost in [3, 8, 5] {
    total := total + cost
}
total
```

The variable keeps its declared or inferred type. Values themselves do not
change: replacing a record field or list element constructs an updated
value for the binding. Other bindings and previously created closures
keep the values they already held.

A closure captures values when it is created. It cannot update a mutable
variable belonging to its enclosing function. Pass in a value and return
an updated value when you need that communication.

## Names and arithmetic

Names can contain hyphens. Write spaces around subtraction so that
`left - right` is an operation rather than a name. Integer arithmetic uses
signed 64-bit values with checked overflow. Floating-point values use
64-bit precision. `/` produces a floating-point result; `//` and `%` are
integer floor division and modulo.

Integer literals adapt to an expected numeric type. A bound `Int` does
not silently become a `Float`; use `float(value)` when that conversion is
intended. Conditions require `Bool`, not a nonzero integer or a nonempty
string. See `gcsl(5)` for syntax and operator rules.

## Durations and sizes

`Duration` and `Size` hold amounts with units. Write a duration as a
number followed by units from largest to smallest: `30s`, `500ms`,
`1h30m`, `2d`. The units are `d`, `h`, `m`, `s`, `ms`, `us` and `ns`, and a
day is exactly 24 hours. Write a size with one unit: `512B`, the decimal
`KB` to `PB`, or the binary `KiB` to `PiB`, as in `2GiB`. The last part of
a literal can have a fraction if the result is a whole number of
nanoseconds or bytes, so `1.5s` and `0.5KiB` are allowed but `1.5ns` is
not. Units are case-sensitive.

```gcsl
let timeout = 1m30s
retry_after = timeout // 3
budget = 2 * timeout
memory = 2GiB + 512MiB
long = timeout > 1m
message = "giving up after {timeout}"
```

Add or subtract two values of the same kind, multiply one by an `Int`,
and use `//` to divide by an `Int` or by another value of the same kind.
Dividing by the same kind gives an `Int`. `%` gives the remainder of one
value by another of the same kind. Values of the same kind can be
compared. Durations and sizes never mix with each other, with `Int` or
with `Float`, and an integer is never a duration, so write `0s` rather
than `0`. `/` is not available, because the result would not be a whole
number of the smallest unit.

A value is shown as its simplest literal: `retry_after` above is `30s`,
`budget` is `3m`, and `memory` is `2560MiB`.
