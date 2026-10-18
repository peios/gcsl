---
title: Inference Limits
type: reference
description: Recognize supported inference and use explicit types at its boundaries.
---

GCSL infers local values and selected present-or-future positions. It does
not infer every relationship between types. Explicit annotations help you
state an intended interface when a program reaches one of these limits.

The following boundaries affect ordinary programs:

| Boundary | What to write |
|---|---|
| An empty list has no independent element type | Give it an annotation or an expected `List<T>` type |
| A pure function does not preserve arbitrary future shapes | Write the present or future contract explicitly |
| Recursive impure results can retain a deferred possibility | Use `Present<T>` when the result must be present |
| General type parameters and intersections are unsupported | Write concrete structural types |
| There is no `None` value or optional-type shorthand | Represent alternatives using supported union members |
| Narrowing recognizes a plain name tested directly by `if` | Bind a field to a name before testing it |
| Narrowing does not establish facts after an early return | Keep uses inside the narrowed branch |
| A narrowed mutable name is a branch-local copy | Don't try to update the original through that copy |
| `len` and `show` use special call rules | Call them directly or through supported aliases, rather than storing them as ordinary function values |

Expected types reach record fields, call arguments, and other supported
literal contexts. An unrelated sibling expression is not a general
constraint-solving mechanism. For example, annotate an empty list before
combining it with independently inferred values.

Warnings do not prevent execution. A union inferred from unlike branch
results can produce a warning while the program remains valid. Use
`gcsl check` to inspect the full set of errors and warnings, and
`gcsl run --dump-types FILE` when the inferred shape is unclear.

These are supported-language boundaries, not promises about a future
inference engine. The checker implementation and its regression tests
are the source for the behavior described here.
