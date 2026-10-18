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
