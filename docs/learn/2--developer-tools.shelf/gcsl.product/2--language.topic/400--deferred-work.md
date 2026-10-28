---
title: Deferred Work
type: concept
description: Delay effects with lazy expressions and resolve them with await.
---

A future represents a calculation whose result is needed later. `lazy`
creates that calculation without running it. Awaiting it evaluates the
body and remembers the successful result. Futures are deferred work, not
concurrent tasks.

This program creates one future and then reads its result:

```gcsl
let answer = lazy 21 * 2
await answer
```

## Where work happens

A `lazy` body is an impure context, even when a pure function creates it.
That lets a pure function plan effects without performing them. A pure
function cannot await a future. Resolve the value in an impure caller
before passing it into a pure calculation that needs the value itself.

The checker inserts implicit awaits at uses that need a value's contents,
such as arithmetic or a required present argument. A spread is one of
those uses: spreading a list whose elements might be futures into a list
of present values awaits each element. It applies the same purity
restriction to explicit and implicit awaits. A call to a pure
function never secretly starts deferred work inside that function.

An implicit await of a record or a list awaits only the fields and
elements that the expected type needs present. A field typed
`T | Future<T>` keeps its future, so a list of records can collect
deferred results without waiting for any of them.

## Writing contracts

Use explicit wrappers when the distinction is part of an interface:

| Type | Contract |
|---|---|
| `Present<Int>` | A present integer, equivalent to `Int` in an ordinary annotation |
| `Future<Int>` | Deferred work yielding an integer |
| `Int | Future<Int>` | Either a present or a deferred integer |

A present value does not satisfy an explicitly required `Future<T>`.
Nested future layers collapse: awaiting a future resolves it fully.
Awaiting an already present value is a silent no-op.

Bare types in impure parameters accept present or deferred values,
including bare positions inside records and lists. Pure parameters infer
whether their bodies need the value's contents. Impure and memoized
results infer their present-or-future shape from the body. Explicit
wrappers keep the contract you wrote.

## Avoiding surprising inference

A pure function with result `Int` requires a present result, even if it
only returns its argument. It does not preserve a future's shape
polymorphically. Recursive result inference also retains conservative
parts of the declared signature. Use `Present<T>` or an explicit future
contract where an API needs a stable distinction, and consult
[Inference Limits](~gcsl/inference-limits) when an inferred type is wider
than expected.
