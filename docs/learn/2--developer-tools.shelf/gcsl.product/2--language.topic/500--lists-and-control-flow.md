---
title: Lists and Control Flow
type: concept
description: Build lists, repeat expressions, and leave loops or functions.
---

Lists hold ordered values of one element type. GCSL's loops are
expressions, so a loop can collect results into a list or run only for its
effects. This avoids a separate collection-building language.

## Lists and ranges

`List<Int>` and `Int[]` name the same type. Indexes start at zero; a
negative or out-of-range index is a runtime error. A spread inserts
another list's elements. An empty list needs an expected element type:

```gcsl
let empty: List<Int> = []
let values = [...empty, 3, 8, 5]
values[1]
```

A spread shares the other list's elements instead of copying them, so
building a list from others, such as a package's files from those of its
dependencies, costs time in proportion to what is added, not to everything
spread. Reading a list's length is immediate, and an index or an update of
one element takes time in proportion to the logarithm of its length.

`0..3` produces `[0, 1, 2]`; `0..=3` includes `3`. Bounds are integers and
ranges run upward. A range used directly as a loop's iterable is counted
without first allocating the list of all its integers.

## Collecting and repeating

A loop used as a value collects each iteration's result. A guard skips
iterations that don't match:

```gcsl
let doubled = for n in [1, 2, 3] if n > 1 { n * 2 }
doubled
```

A loop used as a statement discards its results. Use `for condition` for
repetition while a condition holds, `for { ... }` without a condition,
or `do { ... } for condition` to test after the body. A header can contain
initialization and an update:

```gcsl
let values = for let i = 0; i < 3; i := i + 1 { i * 2 }
values
```

A plain header variable can be rebound by the header. Write `let mut` if
the body must also update it.

## Leaving a computation

`break` ends the enclosing loop and `continue` skips to its next
iteration. Either can supply a value to the collecting loop before
leaving that iteration. An unlabeled `return` returns from the enclosing
function or `lazy` body.

Labels identify an outer loop or a block. A labeled `return` leaves that
target with one value; it does not append to a loop's accumulated list.
Control flow cannot jump out through a function boundary. See `gcsl(5)`
for label syntax and the distinction between collected and returned
values.
