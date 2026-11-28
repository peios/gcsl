---
title: Host Operations
type: concept
description: Declare the operations and handle types a program's host supplies with extern, call them like functions, and handle their failures.
related:
  - gcsl/functions-and-effects
  - gcsl/failure
  - gcsl/answering-operations
---

A GCSL program always runs inside a *host*: `gcsl run`, or a tool that
embeds GCSL, such as a build tool or a test harness. A host can offer work
of its own that no built-in function does, for example starting a virtual
machine. An `extern` declaration tells GCSL about one such *operation*, so
that your program calls it like any function and the checker types the
call.

## Declaring an operation

An operation is a function with a signature and no body. Write `extern`
before `fn` or `def`, and end the declaration at its return type:

```gcsl
extern type Vm = "vm.machine"

extern fn boot(name: String, cpus: Int = 1) -> Vm = "vm.boot"
extern fn (m: Vm) run(...argv: String) -> {status: Int, stdout: String} = "vm.run"
extern fn (m: Vm) halt() -> {}

fn hostname(name: String) -> String {
    let m = boot(name)
    let ran = m.run("hostname")
    m.halt()
    = ran.stdout
}
```

The string after `=` is the name the host knows the operation by. Leave
it out, as `halt` does, and the name is the function's own. The rest is
an ordinary signature: parameters with defaults, a rest parameter, the
method form with a receiver, and `def` to make the operation visible to
other files. A tool that hosts GCSL usually ships these declarations in
a file that you import.

An operation is declared with `->`. The host does the work on every
call, so an operation is never pure and never memoised. It also has no
type parameters and takes no block.

Everything an operation takes and returns must be *data*: numbers,
strings, durations, sizes, files, directories, handles, and records,
lists, and unions of those. The host can't be given a function, a
secret, or a sandbox, and the checker reports a parameter or a result
that holds one. An argument that is a future is awaited before the host
is asked, so the host only ever sees values.

## Handle types

Some of what a host works with stays on the host: a running machine, a
network, an open connection. An `extern type` declares a *handle type*
for values like that:

```gcsl
extern type Vm = "vm.machine"
extern type Network
```

The string is the *kind*, the name the host knows the type by. Left out,
the kind is the type's own name. A handle is a value of that type, and
only the host makes one. Your program can't write a handle, read
anything out of one, or turn a number into one. It can do what it does
with any data:

- Store a handle in a variable, a record, or a list.
- Pass a handle to an operation or to any function.
- Compare two handles with `==`. They are equal when they are the same
  kind and the host gave them the same number.
- Test what a value is with `is` and `as`, in a union such as
  `Vm | Network`.

Interpolating a handle into a string, or passing it to `show`, gives its
kind and its number, such as `<vm.machine 3>`. That text is for messages.
It isn't GCSL source, and nothing reads it back.

Declare each kind in one file of a workspace and import the type where
you need it. Two declarations of one kind would be one type under two
names, so the checker reports the second: "handle kind `vm.machine` is
declared twice".

## Calling an operation

Call an operation as you call any impure function. A call can name its
arguments, leave out the ones with defaults, and spread a list into a
rest parameter. The function is a value too, so you can pass it to
another function or keep it in a record.

Because an operation is impure, a pure function, declared with `:`,
can't call one. To share one call among several callers, wrap it in a
memoised function. Every caller of `shared()` gets the same machine, and
the host boots it once:

```gcsl
extern type Vm = "vm.machine"
extern fn boot(name: String, cpus: Int = 1) -> Vm = "vm.boot"

fn shared() ->> Vm { boot("fixture", cpus = 4) }
```

GCSL checks what the host returns against the declared return type as
it arrives. A value of another type fails the call with an error such as
"the result of operation `vm.boot` is not a Vm", so a host's mistake
never becomes a value of the wrong type further on in your program.

## When an operation fails

A host reports a failure with a message, and sometimes notes. The
failure is raised at your call of the operation, and it is a failure
like any other: `recover` and `try` catch it, and `#[retry]` on a
function around the call runs the call again (see
[Failure](~gcsl/failure)).

`gcsl run` is a host with no operations, so under it every operation
fails. This program runs anywhere, and shows what a failure looks like
to the code that catches it:

```gcsl
extern fn boot(name: String) -> Int = "vm.boot"

outcome = recover { "booted {boot("a")}" } else (failure) { failure.message }
```

Run with `gcsl run`, it prints the host's message:

```text
{outcome = "this host has no operation `vm.boot`"}
```

Without the `recover`, the run ends with that error, pointing at the
call. `gcsl check` checks a program that declares operations as it
checks any other, whatever host the program is written for.
