---
title: Block Parameters
type: concept
description: Write functions that run a block of their caller's code, such as a named step, a retry, or a wait, and see how a block differs from a function value.
related:
  - gcsl/functions-and-effects
  - gcsl/failure
  - gcsl/context
---

A block parameter lets a function take a piece of its caller's code and run
it when it chooses, as often as it chooses. The block runs in the caller's
scope, so it can update the caller's variables, which a function value
can't. That makes blocks the way to write functions that wrap other code: a
named step of a test, a retry, a measurement, or a wait for a condition.

## Declaring and giving a block

Declare a block as a function's last parameter, with `block` and the type
of its result in place of an ordinary type. Inside the function, call the
block like a function with no arguments. A call gives the block in braces
after its other arguments:

```gcsl
fn step(name: String, body: block -> {}) -> {} {
    sh("echo 'begin {name}'")
    body()
    sh("echo 'end {name}'")
    = {}
}

let mut address = ""
step("boot") {
    address := sh("echo 192.0.2.7").trim()
}
step("ping") {
    sh("echo ping {address}")
}
address
```

Each `step` runs its block between its two messages, and the program's
value is `192.0.2.7`. The `echo` commands stand in for whatever a real
`step` reports to.

A block is always given this way, as the call's trailing block. Passing it
inside the parentheses, by position or by name, is an error, and so is
leaving it out: "missing the block `body`". When the block is the only
argument, the parentheses can go too, as in `retrying { ... }`.

## Updating the caller's variables

The block in `step("boot") { ... }` rebinds `address`, a variable of the
code around the call, and the second step reads what the first one set.
A block can do that because it isn't a separate function. It runs where it
is written, each time the function calls it: it reads the variables in
scope there, and it rebinds those declared with `let mut`. A variable that
the block declares itself is the block's own, and is made afresh by each
call.

A function expression can't do the same. A closure holds copies of the
values it captures, so `let bump = fn() -> {} { count := count + 1 }` is
the error "cannot rebind `count` here: it is declared outside this
function". Use a function value when the code has to be stored or passed
around, and a block when it has to share state with the code around it.

## Using the block's value

A call of the block gives the value the block ended with, so a function
can act on it or return it. This `wait_until` calls its block until the
block gives `true`:

```gcsl
fn wait_until(what: String, attempts: Int, ready: block -> Bool) -> {} {
    let mut left = attempts
    for left > 0 and not ready() { left := left - 1 }
    assert left > 0, "{what} did not happen in {attempts} attempts"
}

let mut polls = 0
wait_until("the service is up", 10) {
    polls := polls + 1
    = sh("echo {polls}").trim() == "3"
}
polls
```

The block runs three times, so `polls` is `3`.

A generic function can return whatever its block gives. Each call works out
`T` from its block:

```gcsl
fn measure<T>(name: String, body: block -> T) -> T {
    let start = sh("date +%s").trim()
    let value = body()
    sh("echo \"{name}: $(( $(date +%s) - {start} )) seconds\"")
    = value
}

kernel = measure("uname") { sh("uname -s").trim() }
sum = measure("sum") { 40 + 2 }
```

Here `kernel` is a `String` and `sum` is an `Int`.

A method can take a block as well, after its other parameters:

```gcsl
fn (n: Int) times(body: block -> {}) -> {} {
    for _ in 0..n { body() }
    = {}
}

let mut dots = ""
3.times { dots := dots + "." }
dots
```

## What a block can't be used for

A block isn't a value. The only thing a function can do with its block
parameter is call it. Binding it with `let`, putting it in a record or a
list, returning it, and passing it as an argument are all the error "a
block can only be called". Using it inside a `fn` or a `lazy` written in
the function is an error too.

The reason is the scope the block runs in. The block uses the caller's
variables directly, and those exist only until the caller's own code
ends. A value could outlive them: a stored block, or one captured by a
closure, could be called after the call that gave it has returned. Because
a block can only be called, and only by the function it was given to, it
can never run after its scope has gone.

To pass a block on to another function, call it inside that call's own
block:

```gcsl
fn step(name: String, body: block -> {}) -> {} {
    sh("echo 'begin {name}'")
    body()
    sh("echo 'end {name}'")
    = {}
}
fn measure<T>(name: String, body: block -> T) -> T {
    let start = sh("date +%s").trim()
    let value = body()
    sh("echo \"{name}: $(( $(date +%s) - {start} )) seconds\"")
    = value
}
fn timed_step(name: String, body: block -> {}) -> {} {
    measure(name) { step(name) { body() } }
}

let mut built = false
timed_step("build") { built := true }
built
```

The block given to `timed_step` still runs in the scope it was written in,
two calls further down, and sets `built` there.

A function that takes a block is itself an ordinary value. Its type is
written with the block in it, `fn(String, block -> {}) -> {}`, and a call
through a variable or a parameter of that type gives a trailing block in
the same way.

## Pure and impure blocks

The marker after `block` says what the block can do, as the marker before a
function's result type does:

| Declaration | Meaning |
|---|---|
| `block -> T` | The block can perform effects and await |
| `block: T` | The block is pure |

A pure function can take only a pure block, because calling an impure one
would perform effects. The body of a pure block is pure code wherever it is
written, so it can't call an impure function:

```gcsl
fn bracketed(body: block: String): String { "[{body()}]" }
tag = bracketed { "release" }
```

A pure block can still rebind the variables of the code around it.

## Ending a block with a call or a loop

A block that gives `{}`, such as the block of a `step`, can end with
anything that can stand alone as a statement. A call of an impure function
is one: its value is discarded, as it is when the call is a statement, so
the block doesn't need a `{}` after it:

```gcsl
fn step(name: String, body: block -> {}) -> {} {
    body()
}

step("reload") { sh("echo reload") }
```

A loop whose body gives `{}` is another, since its list holds nothing:

```gcsl
fn step(name: String, body: block -> {}) -> {} {
    body()
}

let mut total = 0
step("count") {
    for n in [1, 2, 3] { total := total + n }
}
```

A block that ends with any other value, such as a number or the result of
a pure function, doesn't fit `{}`, as an unused value is an error in a
statement. A function's own body never discards its last expression, which
is why the `step` in
[Declaring and giving a block](#declaring-and-giving-a-block) ends with
`= {}`.

## Leaving a block

A block can't be left with a jump. The function that calls the block goes
on from that call, so `return` inside a block is the error "`return`
cannot leave a block", and so is a `break` or `continue` for a loop that is
outside the block. To end early, finish the block with a value that the
function can act on, as the block of `wait_until` does. A loop written
inside the block is left with `break` and `continue` as usual.

## Failure in a block

A failure inside a block is a failure of the function's call of the block.
If the function doesn't handle it, the run ends with the error reported
where it happened, inside the block. The function can also catch it with
`recover`, which is how to write a retry. In the fallback, `raise` passes
the block's failure on unchanged, so the last attempt's failure is the one
reported:

```gcsl
fn retrying(times: Int, body: block -> {}) -> {} {
    let mut left = times
    for left > 0 {
        left := left - 1
        recover { body(); left := 0 } else { if left == 0 { raise } }
    }
    = {}
}

let mut attempts = 0
retrying(5) {
    attempts := attempts + 1
    assert attempts == 3, "attempt {attempts} failed"
}
attempts
```

The block fails twice and succeeds the third time, so `attempts` is `3`.
Whatever the block did before it failed stays done, including its updates
of the caller's variables. [Failure](~gcsl/failure) describes `recover`
and `raise`, and has a `step` that records a failure and raises it again.

## Limits on functions that take blocks

A function takes at most one block, as its last parameter. A block
parameter has no default, and it can't be a rest parameter or a method's
receiver. A block takes no arguments.

Three kinds of function can't take a block:

- A memoized function, declared with `->>`, because a memoized call is
  found by its arguments and a block isn't a value to find it by
- A function with a deferring attribute, because a host can run such a
  call after the scope of its block has ended
- A definition type, because a host invokes it and gives its arguments by
  name

`block` isn't a reserved word. It means a block only at the start of a
parameter's type, directly before `->` or `:`, so a variable, a field, or a
type can still be named `block`.

A block runs in the context that is in force where the function calls it;
see [Context](~gcsl/context#context-and-blocks).
