---
title: Running Steps for a Host
type: concept
description: Evaluate a workspace as an action graph, so your host can run, cache and parallelise its steps.
---

A build tool wants to run independent pieces of work at the same time,
reuse results it already has, and isolate each piece. GCSL supports that
through *graph mode*: instead of running every call as it reaches it,
GCSL hands your host the calls it should run as *steps*, and carries on
with the rest of the program until it needs one of their results.

## Marking the calls that are steps

A step is a call of a function whose attribute is declared deferring,
with `#[defers]` as well as `#[attribute]`:

```gcsl
#[attribute] #[defers] type step = {}

#[step]
fn compile(source: String) -> String { "{source}.o" }

#[step]
fn link(objects: List<String>) -> String { "app from {len(objects)}" }

app = link([compile("main.c"), compile("util.c")])
```

Plain GCSL runs every call at once, so the program means the same under
any host. Under a host in graph mode, each call gives a future for its
result instead, and code waits only where it uses one. Here neither
compile waits for the other, and the link is a step whose inputs are the
two compiles' results.

## The loop a host runs

The host keeps a `Graph` for the whole evaluation and drives it:

1. Run a unit, a file with `Workspace::run_graph` or a target with
   `Workspace::prepare_call` and `Workspace::run_prepared`. You get
   `Progress::Paused(node)` if the unit needs a result it doesn't have
   yet, or `Progress::Done(value)` once it finishes.
2. Find what to run. `graph.holes(&value)` lists the results a finished
   value still waits for, `graph.needed(&demands)` adds every step they
   depend on, and `graph.ready(&needed)` keeps those whose own inputs are
   all known. Steps that nothing needs never run.
3. For each ready step, either give its result from your cache, or run it
   with `Workspace::evaluate`, then pass the outcome to `graph.resolve`.
4. Run the unit again. It starts from the beginning, finds the steps it
   made before, and continues past the point where it stopped.

A target's call can set root contexts as it starts, from values you give
or from a top-level field of a file, `Workspace::prepare_call_with`. With
`Workspace::prepare_call_wrapped` it can also give the target's value to
a top-level function of a file, followed by literal arguments of your
own, as if the call were written `FUNCTION(DT PATH(), ARGS...)`. That's
how a host runs code of its own over a target without calling a function
value itself: pekit packs a package as `packaging.pack(package NAME())`.
Several prepared calls are units of their own, and can be driven together
against one graph, sharing the steps they both reach.

A step's failure is raised where its result is used, so a `recover`
around that use catches it, as with any failure. A finished value can
also hold a failed step's result that no code used. Once
`graph.holes(&value)` is empty, `graph.failures(&value)` lists those
failures so that you can report them.

Effects that aren't inside a step are steps too: in graph mode, a unit
never runs a program or touches a file itself. Each such effect becomes a
step that the unit needs at once, which keeps it in program order.

Every run in graph mode takes a `GraphHost`: which deferred calls the
host takes as steps, and the `Effects` the unit's own effects use, such
as the working directory that `file("hello.c")` resolves against.

## Running many steps at once

GCSL evaluates on one thread, but the programs steps run can run in
parallel. Run a step's body as a unit of its own with
`Workspace::run_step`: each `exec` in it becomes an effect step of that
unit, and the unit stops at it. `Workspace::effect_job` turns that effect
into an `EffectJob`, plain data you can send to a worker thread, where
`EffectJob::run` runs it under your `Effects`, with a deadline if you want
one. Give the result back with `Workspace::from_plain` and
`graph.resolve`, and run the step again; it carries on to its next
effect. One GCSL thread can drive as many running programs as you have
workers. A step's own `#[retry]` and `#[timeout]` are left to you, since
you see every attempt: read them from its attributes with
`check::retry_of` and `check::timeout_of`, as GCSL itself does. To run a
failed step again from its start, call `graph.forget_unit` with its unit
first, so that its effects are made again rather than found failed.

## Keys, inputs, and isolation

`Workspace::step_inputs` gives what you need to key and stage a step:

- its arguments, its function's captured values and the context it can
  read, as plain data (`Plain`);
- its function's attributes;
- the files whose sources make up its code, which you hash to identify
  the code.

The context a step can read is every context bound where it was made
that has a default, and each without one that its function needs (see
[What a function needs](~gcsl/context#what-a-function-needs)). GCSL
works out what a function needs through every call it makes, so a step
can't read a context with no default that its function doesn't need. A
context that only your host binds for some runs, such as pekit's script
session, then doesn't change the key of a step that never reads it.

`Plain::files` lists the files and directories in a value, which is what
a sandbox needs to contain. Results go back with `Workspace::from_plain`,
and `Workspace::to_plain` refuses a result that holds a function or a
result not given yet.

`Workspace::evaluate` takes an `Effects` value, which decides how the
step's effects reach the system. `Direct` uses the system as it is. Your
own implementation can run each program in a sandbox, through
`Effects::command`, and root file paths there, through
`Effects::host_path`; GCSL's values keep the paths the code wrote.

If your sandboxes use bubblewrap, also implement
`Effects::sandbox_command`, which runs the commands of a GCSL
`Dir.sandbox` with the sandbox's copy as their root. Without it, GCSL
starts its own bubblewrap inside yours, which some systems, such as
Ubuntu, forbid.

Three more hooks let you avoid copying. `Effects::place_file` places each
file of an `fs.compose_dir`, a copy by default, so you can hard-link a
file nothing can change. `Effects::composed` tells you when a composition
is finished, so you can make it read-only. `Effects::make_sandbox` lets
you make a sandbox yourself, as an overlay over its directory, say; you
then answer `Effects::host_path` for paths in it, upper layer over lower.

A host decides, per call, whether to take a deferred call as a step, from
the attribute's arguments. An attribute declared as
`type step = {on: Bool = true}` lets a function written `#[step(false)]`
run inline. The host must decide the same way for the same call
throughout one evaluation.
