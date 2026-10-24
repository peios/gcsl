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

A secret computed from other secrets is identified by theirs: `login`
above is identified as `database password`.

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

## Taking the value

When you really do need the value, take it with `.unsafe_value()`:

```gcsl
let token = secret("abc123", "api token")

header = "Authorization: Bearer {token.unsafe_value()}"
```

`header` is a plain `String`, and is shown in full. Use it only where the
value has to leave GCSL, and nowhere it might be shown.
