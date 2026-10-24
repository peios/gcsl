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

A step's failure is raised where its result is used, so a `recover`
around that use catches it, as with any failure.

Effects that aren't inside a step are steps too: in graph mode, a unit
never runs a program or touches a file itself. Each such effect becomes a
step that the unit needs at once, which keeps it in program order.

## Keys, inputs, and isolation

`Workspace::step_inputs` gives what you need to key and stage a step:

- its arguments, its function's captured values and the context it runs
  in, as plain data (`Plain`);
- its function's attributes;
- the files whose sources make up its code, which you hash to identify
  the code.

`Plain::files` lists the files and directories in a value, which is what
a sandbox needs to contain. Results go back with `Workspace::from_plain`,
and `Workspace::to_plain` refuses a result that holds a function or a
result not given yet.

`Workspace::evaluate` takes an `Effects` value, which decides how the
step's effects reach the system. `Direct` uses the system as it is. Your
own implementation can run each program in a sandbox, through
`Effects::command`, and root file paths there, through
`Effects::host_path`; GCSL's values keep the paths the code wrote.

A host decides, per call, whether to take a deferred call as a step, from
the attribute's arguments. An attribute declared as
`type step = {on: Bool = true}` lets a function written `#[step(false)]`
run inline. The host must decide the same way for the same call
throughout one evaluation.
