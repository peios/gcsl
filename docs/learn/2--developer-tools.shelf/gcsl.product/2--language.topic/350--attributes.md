---
title: Attributes
type: concept
description: Tell the program that runs a function how to run it.
---

An attribute goes before a function and says how to run it, for example
whether a build step may be cached or how long it may take. GCSL checks
attributes but doesn't act on them itself: they're read by the tool that
runs the program, such as a build system.

```gcsl
#[attribute] type hermetic = {on: Bool = true}

#[hermetic]
fn compile(source: String) -> String { "compiled {source}" }

#[hermetic(false)] #[timeout(10m)]
fn deploy(target: String) -> String { "deployed {target}" }

result = deploy(compile("main.c"))
```

## Declaring an attribute

An attribute's arguments form a record, so you declare an attribute as a
record type marked `#[attribute]`. Each field is an argument, and a field
with a default is an argument you can leave out:

```gcsl
#[attribute] type device = {name: String, count: Int = 1}

#[device(name = "kvm")]
fn boot() -> String { "booted" }

booted = boot()
```

Arguments must be constants: literals, or lists and records of them. A
field may be an `Int`, `Float`, `Bool`, `String`, `Duration` or `Size`, or
a list or record of those. An attribute is declared, imported and named in
the same way as any other type, so a tool usually provides its attributes
in a module that you import.

## Using an attribute

Write `#[NAME]`, or `#[NAME(ARGUMENTS)]`, on the line before a function: a
`fn`, a `def`, or a definition made with a definition type. An attribute
with one field can take its argument by position, as in `#[timeout(10m)]`;
one with more fields takes its arguments by name. A misspelt attribute is
an error, so a mistake isn't silently ignored.

GCSL provides two attributes of its own, `retry` and `timeout`:

| Attribute | Argument | Meaning |
|---|---|---|
| `#[retry(times)]` | `times: Int` | The call may be run again this many times if it fails. |
| `#[timeout(limit)]` | `limit: Duration` | The call fails if it runs for longer than this. |

GCSL checks and records them, and doesn't act on them yet.

## Attributes on a definition type

Attributes on a definition type apply to every definition made with it.
A definition's own attribute of the same name replaces the definition
type's:

```gcsl
#[attribute] type hermetic = {on: Bool = true}

#[def_type] #[hermetic]
def check(name: String, f: fn(): Bool): fn(): Bool { f }

def check fast() { true }
#[hermetic(false)] def check online() { true }

results = [(check fast)()(), (check online)()()]
```

Here `fast` is hermetic and `online` is not.
