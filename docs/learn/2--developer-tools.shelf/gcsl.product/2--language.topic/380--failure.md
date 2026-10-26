---
title: Failure
type: concept
description: Check what must hold with assert, and handle a failure with recover.
---

A program fails when it hits a runtime error: a `panic`, an index out of
range, a program it runs exiting with an error, or a failed `assert`.
Normally that ends the run. Two constructs work with failure directly:
`assert`, which fails on purpose when something doesn't hold, and
`recover`, which catches a failure and continues.

## Asserting

`assert COND` fails the run if the condition is false. The error shows
the condition as you wrote it, and for a comparison it labels each side
with its value, so a failed check explains itself:

```text
error: assertion failed: `output == "hello"`
 --> app.gcsl:2:8
  |
2 | assert output == "hello"
  |        ------    ------- this is `"hello"`
  |        |
  |        this is `"hullo"`
```

Give your own message after a comma, `assert ok, "the server did not
start"`. Each side of a comparison is evaluated only once, so an
assertion about a program's output doesn't run the program twice.

## Recovering

`recover { BODY } else { FALLBACK }` evaluates the body, and if it fails,
evaluates the fallback instead. Name the failure to read its message:

```gcsl
fn half(n: Int): Int {
    assert n % 2 == 0, "{n} is odd"
    n // 2
}

even = recover { half(10) } else { 0 }
odd = recover { half(7) } else { -1 }
message = recover { "half is {half(7)}" } else (failure) { failure.message }
```

The failure is a record, `{message: String}`. The two blocks join as an
`if`'s branches do, so they normally have one type.

`recover` catches what `#[retry]` would retry, and nothing else. A bug in
GCSL itself is never caught. Neither is a timeout of a call that encloses
the `recover`: that timeout belongs to the enclosing call, which ends as
if the `recover` weren't there. A `break`, `continue` or `return` inside
the body is not a failure, and leaves through it as usual. Anything the
body did before it failed stays done.
