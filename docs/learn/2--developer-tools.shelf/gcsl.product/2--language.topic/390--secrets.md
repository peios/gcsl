---
title: Secrets
type: concept
description: Keep a password or token from leaking with Secret<T>.
---

A secret is a value that never leaks: a password, a token, or a key. Its
type is `Secret<T>`, and wherever it's shown, it's `***`. Make one with
`secret`, giving the value and an identity, a public name for it:

```gcsl
let password = secret("hunter2", "database password")
let user = "admin"

shown = "{password}"
login = "{user}:{password}"
```

Here `shown` and `login` are both secrets too, and are shown as `***`.

## What a secret is computed into

Anything computed from a secret is a secret. That covers `+` and the
other operators, interpolating it into a string, reading a field of a
secret record and indexing a secret list. A comparison is too:
`password == "hunter2"` is a `Secret<Bool>`, not a `Bool`, since its
answer would tell you about the password. The same goes for comparing
any values that hold a secret, such as two records with a secret field.

A secret computed from others is identified by how it was computed: the
operation, the identities of the secrets it used, and the plain values it
used, which aren't secret. `login` above is identified as
`interp("admin", ":", $(database password))`, so equal computations have
equal identities and different ones different.

Give each secret an identity of its own. Where GCSL compares values
without making the result a secret, it compares secrets by identity
alone. That covers `===`, which compares even fields a value's type
doesn't name, and the arguments of a memoised call. Two secrets with one
identity are one secret to them, whatever their values, so neither can
tell anything about a value.

## A secret decides nothing

What a program does never depends on a secret, because whoever can see
what ran would learn something about it. So a secret can't be the
condition of an `if`, the condition of an `assert` or an operand of
`and` and `or`. You can't test it with `is` or `as`, or loop over it with
`for`. A secret isn't its value either: you can't pass a `Secret<String>`
where a `String` is expected.

If an operation on a secret fails, the error says only that "an
operation on a secret failed", and where. Such a failure might depend on
the secret, so `recover` doesn't catch it and `#[retry]` doesn't retry
it. A failure of anything else in the expression is unchanged: in
`password + panic("boom")`, the `panic` fails as it always does.

## Giving a secret to a program

A program that needs a secret gets it revealed: `.reveal()` turns a
`Secret<T>` into a `Revealed<T>`, which `exec` accepts as an argument.

```gcsl
let token = secret("abc123", "api token")

# `test -n` exits 0 if the token it is given isn't empty.
status = exec(["test", "-n", token.reveal()]).status
```

The program receives the value, but GCSL still shows a revealed secret as
`***`, so it doesn't appear in error messages or anywhere GCSL shows the
command line. A revealed secret is contagious too:
`"--token={token.reveal()}"` is a `Revealed<String>`, while mixing in a
secret that isn't revealed gives a plain `Secret`, which `exec` refuses.

Only `exec` takes a revealed secret, so searching for `.reveal()` finds
every program that ever receives one. What the program does with it is up
to the program: if it prints the secret, the output it returns has it in
plain text.

## Taking the value

When you really do need the value inside GCSL, take it with
`.unsafe_value()`:

```gcsl
let token = secret("abc123", "api token")

header = "Authorization: Bearer {token.unsafe_value()}"
```

`header` is a plain `String`, and is shown in full. Use it only when
nothing else will do, and nowhere it might be shown. To give a secret to a
program, reveal it instead.
