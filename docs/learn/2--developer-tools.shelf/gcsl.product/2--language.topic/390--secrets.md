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

## Secrets from a keychain

A build shouldn't hold its secrets in its source. The `keychain` module
gives them by name instead, and the same build file works wherever the
secret is kept:

```text
import keychain

let token = keychain.string("api-token")
let key = keychain.file("module-signing")
```

`keychain.string` gives a `Secret<String>` and `keychain.file` a
`Secret<File>`. Each looks in the keychain's configuration first: the
context `keychain_keys`, a list of entries naming a secret, the profile
it's for, and the file that holds it. A relative path is relative to the
workspace's root:

```text
let key = keychain.file("module-signing") with {
    keychain_keys = [{profile = "dev"; name = "module-signing"; file = "keys/test.pem"}]
}
```

The profile is the context `keychain_profile`, `dev` unless something
sets it, so a release build can use other keys than a developer's. A
string from a file is the file's contents, without a newline at the end.

With nothing configured, a secret comes from the environment: a file from
the path in `$GCSL_SECRET_FILE_<NAME>`, and a string from that file or
the value of `$GCSL_SECRET_<NAME>`. `<NAME>` is the secret's name in
capitals, with `_` for each character that isn't a letter or a digit, so
`api-token` is `$GCSL_SECRET_API_TOKEN`. A secret found in none of these
fails, naming the variable to set.

A keychain's secret is identified by its profile and name, `dev/api-token`,
unless the program running GCSL identifies it otherwise. A build tool
such as pekit adds a keyed hash of the secret's contents, so that a
changed key never reuses what was made with the old one.

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

A program that needs a secret gets it with `exec_secret`, as an
environment variable. Reveal the secret with `.reveal()`, which turns a
`Secret<T>` into a `Revealed<T>`, and pass it in `secrets`:

```gcsl
let token = secret("abc123", "api token")

# `test -n` exits 0 if the variable it is given isn't empty.
let run = exec_secret(["sh", "-c", "test -n \"$TOKEN\""], secrets = {TOKEN = token.reveal()})
status = run.status
```

The program receives the value. Everything it reports is a secret,
because the program might tell of what it was given: `run` is a
`Secret<{status: Int, stdout: String, stderr: String}>`, so `run.status`
and `run.stdout` are secrets too. Files the program writes aren't secrets;
a key that signs a file doesn't make the signed file secret.

A secret is never an argument of the command. Any user on the machine
can read a running program's arguments, so `exec` refuses a revealed
secret anywhere in its arguments or its `env`, and points you to
`exec_secret`. Its arguments and the names of its variables are public.

`exec_secret` fails, as `exec` does, when the program exits with a status
other than 0, unless you pass `panic = false`. The error says only that "a
program given a secret failed", without the status or the output. Unlike
other failures involving a secret, `recover` can catch it and `#[retry]`
can retry it: it tells only that the program failed. `exec_secret` doesn't
copy output as it arrives, or give the program the terminal, since either
would show the output.

A revealed secret is contagious too: `"--token={token.reveal()}"` is a
`Revealed<String>`, while mixing in a secret that isn't revealed gives a
plain `Secret`, which `secrets` refuses. Only `exec_secret` takes a
revealed secret, so searching for `.reveal()` finds every program that
ever receives one.

The result of a run is identified by everything the program was given:
its arguments, its options and the identities of its secrets. Two runs of
one command with the same secrets are one secret wherever secrets compare
by identity, even if their outputs differ.

## What secrets protect against

Secrets keep a value from leaking by accident: into a log, an error
message, a cache key, a build record or the output of a command. They
don't protect a secret from the code that uses it. Anyone who can change
a build file can take the value with `.unsafe_value()`, or give it to a
program that writes it somewhere. Review where `.reveal()` and
`.unsafe_value()` are used, as you'd review any code that handles
credentials.

GCSL makes no promise about timing. How long an operation on a secret
takes can depend on its value, but the code that could measure that can
already read the value directly.

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
