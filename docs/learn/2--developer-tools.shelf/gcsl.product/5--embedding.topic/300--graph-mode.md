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
   `Progress::Paused(nodes)` if the unit needs results it doesn't have
   yet (see [Code that waits](#code-that-waits)),
   `Progress::Waiting(unit)` if it needs a result that another unit is
   computing (see [Units that share work](#units-that-share-work)), or
   `Progress::Done(value)` once it finishes.
2. Find what to run. `graph.holes(&value)` lists the results a finished
   value still waits for, `graph.needed(&demands)` adds every step they
   depend on, and `graph.ready(&needed)` keeps those whose own inputs are
   all known. Steps that nothing needs never run.
3. For each ready step, either give its result from your cache, or run it
   with `Workspace::evaluate`, then pass the outcome to `graph.resolve`.
4. Run the unit again. It carries on from the point where it stopped:
   nothing it did before runs again.

A target's call can set root contexts as it starts, from values you give
or from a top-level field of a file, `Workspace::prepare_call_with`. With
`Workspace::prepare_call_wrapped` it can also give the target's value to
a top-level function of a file, followed by literal arguments of your
own, as if the call were written `FUNCTION(DT PATH(), ARGS...)`. That's
how a host runs code of its own over a target without calling a function
value itself: pekit packs a package as `packaging.pack(package NAME())`.
Several prepared calls are units of their own, and can be driven together
against one graph, sharing the steps they both reach.

## Code that waits

When an iteration of a `for` loop in a function stops to wait, the rest
of the loop doesn't wait with it: the iteration is set aside, and the
next one runs. So a loop that fetches a hundred files stops once, with
all hundred fetches in `Progress::Paused(nodes)`, rather than a hundred
times. The loop's value is the same as if its iterations had run in
order, and if several fail, the earliest iteration's failure is the one
reported.

A loop is only split like this if its iterations are independent: none
assigns a variable declared outside the loop with `:=`, and none leaves
it with `break` or `return`. Any other loop runs its iterations in
order.

The same goes for the elements of a list, the pieces of a string with
`{}` in it, and the arguments of a call. In
`link(compile(a), compile(b))`, both compiles are met before the unit
stops. A call's body runs, and a parameter's default is evaluated, only
once every argument before it is there. An element or argument is set
aside only if it assigns no variable with `:=` and has no `break`,
`continue` or `return` in it.

An iteration set aside can still reach effects that must happen in
program order, such as programs that write files the next one reads.
Your `Effects` says which effects it isolates, through
`Effects::isolates`: an effect you don't isolate is only made once
nothing earlier in the program is still waiting, so you still see such
effects one at a time, in program order. Once something set aside has
failed, nothing after it that you don't isolate is made at all, as the
program would have stopped at the failure. By default nothing is
isolated. Isolate an effect only if its order can't matter, such as a
fetch pinned by its digest.

## Units that share work

A unit that stops keeps everything it was doing, including the memoised
calls it is in the middle of. If another unit makes the same memoised
call with the same arguments, or awaits the same `lazy` value, it
doesn't compute it again. It stops with `Progress::Waiting(unit)`,
naming the unit that is computing it. Run that unit, or anything else
that can make progress, and then run the waiting unit again. It looks
the call up again, and finds the result once the other unit has
finished it.

A unit that waits on another never makes a node of its own for the
shared work, so the work is done once, by whichever unit reached it
first. If every unit you have is waiting on another, they are waiting
on each other: report that as an error rather than running them again.

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
workers. A step's `exec`s run in the order its code makes them. A
step's own `#[retry]` and `#[timeout]` are left to you, since
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

`Workspace::run_step` also gives what the step's run read of the context
it was made in: each context whose binding is the step's own, with
whether it was bound or its default read. A step keeps what it read while
it is stopped, so the reads a run that ends gives are everything the step
read, and you can key the step by the values of those contexts alone,
rather than by every context it could read. A memoised call found in its
table counts the reads it made, so a cache hit hides none. Each context
is also told to your `Effects::context_read` the first time a run reads
it, before the step does anything with it, so you can give the step that
context's files only then.

A deferred call is one node however many times, and wherever, it is
made: by its function, its arguments and the context it was made in, but
not the definition whose body made it, which a step can't read. So two
targets that make the same call share one step, and a graph records where
each unit met each node, which `Graph::met_at` gives you, for messages
about a unit's own code.

Name a step's outputs by what it is, so that they're at the same path in
every build that makes it, whatever else you build.
`Workspace::step_identity` gives the step's inputs as
`Workspace::step_inputs` does, its function among them, but a result of
another step as `Plain::Step` with that step's node, whether or not it's
been given. Name each step after the steps whose results it's given, and
you can name every step as soon as it's made, before any runs.

`Workspace::step_definition` tells you which definition made a step: the
one whose body was running where it was first made, or else the innermost
`DT PATH()` around it.

`Plain::files` lists the files and directories in a value, which is what
a sandbox needs to contain. Results go back with `Workspace::from_plain`,
and `Workspace::to_plain` refuses a result that holds a function or a
result not given yet.

`Workspace::evaluate` takes an `Effects` value, which decides how the
step's effects reach the system. `Direct` uses the system as it is. Your
own implementation can run each program in a sandbox, through
`Effects::command`, and root file paths there, through
`Effects::host_path`; GCSL's values keep the paths the code wrote.
`Effects::working_dir` is the run's working directory: where a program
given no `cwd` runs, and what `working_dir()` returns. Relative paths
don't resolve against it; they resolve against the directory of the file
they're written in. Give a library's files the directory you found it in
with `SourceFile::with_dir`, so that its relative paths point at its own
files.

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

## Seeing where evaluation spends its time

Call `Workspace::start_profiling` before you run anything, and
`Workspace::profile_report` once you're done. Every run of the
workspace's code is profiled from then on: its files, its prepared calls
and its steps, through every pause. The report has a row for each
function and built-in that ran:

- `calls`, how many calls of it ran, and `found`, how many of those were
  memoised calls that found their entry;
- `own`, the time spent in its body, less the calls made inside it;
- `total`, the time of its calls including the calls inside them,
  counted once for a function that calls itself.

A call's time starts once its arguments are evaluated, so evaluating an
argument counts toward the caller. `outermost` is the time spent in
calls that no other call was running around. If you time your runs, the
difference is the time spent at the top level of the files, outside any
function. A profiled run costs a little more than one that isn't, and
nothing changes for a workspace that never starts profiling.

A target's unit carries on from where it stopped after each pause, so a
profile counts each call it makes once, however often it stops. Time a
unit spends stopped isn't counted. When the same expensive work, such as
parsing a lock file, shows up thousands of times, it's being done by
that many calls: memoise the function that does it with `->>`, and
later calls find its entry instead of doing the work again.
