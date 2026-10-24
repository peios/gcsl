---
title: Functions and Effects
type: concept
description: Choose pure, impure, or memoized functions and understand captures.
---

A function's signature says what it accepts, what it returns, and whether
calling it can perform effects. This makes executable configuration
reviewable: a pure calculation cannot silently run a process or read a
file.

## Choosing a function form

Use `fn` for a private function and `def` for a function exposed as a
field. Both forms declare parameter and result types. A function
expression can also be assigned to a variable or passed to another
function.

The marker before the result type selects the effect:

| Marker | Meaning |
|---|---|
| `:` | Pure computation; no effects or awaiting |
| `->` | Impure computation; effects and awaiting are allowed |
| `->>` | Impure computation memoized by its arguments |

This private pure function accepts a named argument and a default:

```gcsl
fn scale(value: Int, factor: Int = 2): Int { value * factor }
scale(value = 21)
```

Positional and named arguments are evaluated in source order. A trailing
block supplies the last parameter, which is useful for functions that
accept a configuration record.

A default can be any expression. It can use the parameters before it and
anything visible where the function is written:

```gcsl
let suffix = ".o"
fn object(source: String, output: String = "{source}{suffix}"): String {
    output
}
main = object("main.c")
lib = object("lib.c", output = "lib.a")
```

A default is evaluated each time a call leaves its parameter out, after
the arguments that are written and before the function's body runs. It
has the function's effect, so a pure function's default cannot call an
impure function. A default cannot use its own parameter or a later one.
Only a call by the function's own name uses its defaults. A call through
a copy, such as `g` after `let g = object`, or through a parameter must
supply every argument.

After a named argument, a name on its own is shorthand for passing the
variable of that name to the parameter of that name. These two calls are
the same:

```gcsl
fn build(source: String, output: String, compiler: String): String {
    "{compiler} {source} -o {output}"
}
let compiler = "cc"
short = build("main.c", output = "main.o", compiler)
long = build("main.c", output = "main.o", compiler = compiler)
```

Before any named argument, a name on its own is an ordinary positional
argument.

## Generic functions

A function can work on values of any type. Name its type parameters in
angle brackets after its name, and use them as types in its signature and
body:

```gcsl
fn first<T>(items: List<T>, fallback: T): T {
    if len(items) > 0 { items[0] } else { fallback }
}
port = first([8080, 9090], 80)
name = first([], "none")
```

Each call works out the type parameters from its arguments: here `T` is
`Int` in the first call and `String` in the second. Inside the function, a
value of type `T` can be passed on, returned, or put in a list or record,
but nothing else, because `T` could be anything. A call where nothing binds
a type parameter, such as one that only returns a `List<T>`, is an error.

## Methods

A method is a function you call on a value with a dot. Write its
receiver in parentheses before its name:

```gcsl
fn (n: Int) twice(): Int { n * 2 }
fn (items: List<T>) first_or<T>(fallback: T): T {
    if len(items) > 0 { items[0] } else { fallback }
}
doubled = 21.twice()
port = [8080].first_or(80)
```

The value before the dot is the receiver. GCSL chooses the method by
the receiver's type, so methods with the same name can serve different
types. When more than one fits, the one for the more specific type is
used: a method for `List<Int>` is chosen over one for `List<T>` when
the receiver is a `List<Int>`. A method's other parameters, defaults and
effects work as a function's do.

A method has no name of its own: `twice` alone is not a variable, and
`x.twice` without parentheses reads a field. A record field and a method
with the same name are an error when both could apply.

Methods are declared at the top level of a file. A `fn` method is
visible only in its own file. A `def` method is also visible to the
other files of its directory and the directories below it, and to every
file that imports its module, whatever the form of the import. In the
REPL, methods are visible to later lines.

## Calling for effects

A call to an impure or memoized function can stand alone as a statement,
outside a pure function, to run it for its effects. Its value is
discarded, whatever it is, and anything it defers is awaited first, so no
effect is lost:

```gcsl
fn build(source: String) -> String {
    sh("make -C {source}")
    sh("make -C {source} install")
    = source
}
```

Any other value standing alone is an error, because discarding it would
do nothing: a pure expression has no effects to keep. Make it the block's
result with `= ...`, bind it with `let`, or remove it.

## Captures and recursion

Closures hold copies of the values they capture. Rebinding a variable
later doesn't change an existing closure. An inner function cannot rebind
an outer function's variable.

Top-level named functions are available regardless of textual order and
can refer to each other within the file. Workspace files still need an
acyclic dependency order: functions in two files cannot make those files
need each other. Put mutually dependent definitions together or extract
their shared dependencies.

## Memoization

A memoized function reuses a successful result for the same arguments in
the same execution context. The cache belongs to the closure, so two
closures created from one function expression have separate entries.
Memoization also accounts for supported ambient inputs such as a caller
path when the function reads it.

This is not a persistent build cache. External changes to files or a
command's environment do not invalidate a stored result. Use `->>` only
when reusing the first result within that run is correct. Cached keys and
results consume memory for the lifetime of their execution context.

Read [Deferred Work](~gcsl/deferred-work) before using futures in function
signatures; a bare type's present-or-future meaning depends on its
position and the function's effect.
