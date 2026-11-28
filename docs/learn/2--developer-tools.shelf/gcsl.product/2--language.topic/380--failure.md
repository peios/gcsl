---
title: Failure
type: concept
description: Check what must hold with assert, handle a failure with recover, go on past one with try, and pass one on with raise.
---

A program fails when it hits a runtime error: a `panic`, an index out of
range, a program it runs exiting with an error, or a failed `assert`.
Normally that ends the run. Four constructs work with failure directly:
`assert` fails on purpose when something doesn't hold, `recover` catches
a failure and continues, `try` goes on past a failure without a fallback,
and `raise` passes a caught failure on.

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

The failure is a record, `{message: String, notes: List<String>}`. The
notes add detail to the message. A program that fails, for example, notes
its standard output and standard error. The two blocks join as an `if`'s
branches do, so they normally have one type.

`recover` catches what `#[retry]` would retry, and nothing else. A bug in
GCSL itself is never caught. Neither is a timeout of a call that encloses
the `recover`: that timeout belongs to the enclosing call, which ends as
if the `recover` weren't there. A `break`, `continue` or `return` inside
the body is not a failure, and leaves through it as usual. Anything the
body did before it failed stays done.

## Going on past a failure

`try EXPR` gives the value of the expression, or `{}` if it fails. It is
short for `recover { EXPR } else { {} }`, and catches the same failures.
Its main use is a call made for its effects, where a failure shouldn't
stop what comes after:

```gcsl
fn half(n: Int): Int {
    assert n % 2 == 0, "{n} is odd"
    n // 2
}

even = try half(10)
odd = try half(7)
sum = try half(10) + half(7)
size = if odd is Int { odd } else { 0 }
```

Here `even` is `5`, and `odd` is `{}`. `try` takes the longest expression
after it, so `sum` is the `try` of the whole sum and is `{}` too. The type
of `try half(7)` is `Int | {}`, so test the value with `is` before you
use it as a number, as `size` does.

For a record, `try` can't tell a failure from a value. A record joined
with `{}` has no field left to read, so the type of `try` on any record is
`{}`. If you need the fields, use `recover` and say what a failure gives:

```gcsl
fn probe() -> {status: Int} { exec(["sh", "-c", "exit 3"]) }
status = recover { probe().status } else { -1 }
```

`try` awaits its expression where it stands, including anything the
expression defers, so deferred work fails inside the `try` and not later.

`try` isn't a reserved word. It means this form only when the start of an
expression follows it on the same line: a name, a literal, `{`, or a word
such as `not`, `if` or `lazy`. Before `(`, `[` or `-`, and anywhere else,
`try` is an ordinary name, so `try(x)` is still a call of a function
named `try`. To use `try` on an expression in parentheses, write
`try { (x) }`.

## Recovering a caller's block

A function that takes a [block parameter](~gcsl/block-parameters) can put
its call of the block in a `recover`. A failure in the block comes out of
that call like the failure of any call, so the function decides what a
failure of its caller's code means:

```gcsl
fn attempt(body: block -> {}) -> String {
    recover { body(); = "passed" } else (failure) { "failed: {failure.message}" }
}

let mut reached = ""
first = attempt {
    reached := "boot"
    assert sh("echo down").trim() == "up", "the service is down"
    reached := "check"
}
second = attempt { reached := "{reached}, then cleanup" }
progress = reached
```

Here `first` is `"failed: assertion failed: the service is down"` and
`second` is `"passed"`. The block's assignment before the failure stays
done, so `progress` is `"boot, then cleanup"`. A failure that the function
doesn't recover ends the run, reported where it happened in the block.

## Raising a failure again

Inside the fallback of a `recover`, `raise` fails again with the failure
that the `recover` caught. The failure is passed on as it was: the same
message, the same place, the same notes. Whatever reports or catches it
next sees the original failure, not the `raise`.

Use `raise` when a function needs to act on a failure without deciding
its outcome. This `step` records that its block failed and then lets the
failure go on. Its caller uses `try` to continue with the next step:

```gcsl
fn step(name: String, body: block -> {}) -> {} {
    recover { body() } else (failure) {
        sh("echo 'step {name} failed: {failure.message}'")
        raise
    }
}

let mut passed: List<String> = []
try step("boot") {
    assert sh("echo down").trim() == "up", "the service is down"
    passed := [...passed, "boot"]
}
try step("report") {
    passed := [...passed, "report"]
}
passed
```

The first step fails, so its `echo` runs and `try` goes on to the second.
The program's value is `["report"]`. Without the `try`, the run ends with
"assertion failed: the service is down", reported at the `assert` in the
block. The `echo` stands in for whatever a real `step` records.

`raise` has the type `Never`, so it can end a fallback whatever type the
`recover` has. It belongs to the innermost `recover` whose fallback it is
written in. Writing it anywhere else is an error, and so is writing it in
a `fn` or a `lazy` inside the fallback, which could run after the fallback
has ended. `raise` isn't a reserved word: if a variable named `raise` is
in scope, `raise` reads the variable.
