---
title: Providing Host Operations
type: how-to
description: Give GCSL programs operations of your own, read their arguments and handles as plain data, answer them in graph mode or through Effects, and report their failures.
related:
  - gcsl/host-operations
  - gcsl/graph-mode
---

A program declares the operations it expects from its host with `extern`
(see [Host operations](~gcsl/host-operations)). This page shows how your
host answers them, so that a call such as `boot("a")` in a GCSL program
reaches your code and gets a value back.

## Before you start

- Your host evaluates a workspace in graph mode (see [Running steps for
  a host](~gcsl/graph-mode)). Graph mode is where a host answers
  operations: a unit stops at each one and carries on when you give its
  result.
- Your programs can import the `extern` declarations of your operations.
  Ship them in a file of your own, or generate them from the list of
  operations your host has.

## Declare your operations

Write one `extern` declaration for each operation and one `extern type`
for each kind of handle. Programs written for a test harness that runs
virtual machines might import these:

```gcsl
extern type Vm = "vm.machine"

extern def boot(name: String, cpus: Int = 1) -> Vm = "vm.boot"
extern def (m: Vm) run(...argv: String) -> {status: Int, stdout: String} = "vm.run"
extern def (m: Vm) halt() -> {}
```

The string after `=` is the only name your host sees. Give each kind and
each operation a name that stays the same when someone renames the GCSL
function. An operation declared without one is known by its function's
name, which the language server then refuses to rename.

## Answer an operation in graph mode

An operation is an effect, so in graph mode a unit that calls one stops
with `Progress::Paused(nodes)`, as it does at any effect. For each node:

1. Call `Workspace::operation` with the graph and the node. It returns
   an `Operation` if the node is an operation's, and `None` for any
   other node.
2. Do the work. The operation's `name` field is the name from the
   declaration, and its `args` field holds the arguments as plain data
   (`Plain`), in parameter order. A method's receiver comes first, a
   default the call left out is filled in, and a rest parameter is one
   list.
3. Turn your result into a value with `Workspace::from_plain`, and give
   it to the node with `Graph::resolve`.
4. Run the unit again. It carries on from the call.

This function answers one node. `perform` stands for your host's own
dispatch from a name and arguments to a result:

```rust
fn answer(workspace: &mut Workspace, graph: &mut Graph, node: NodeId) {
    let operation = workspace.operation(graph, node).expect("an operation");
    let outcome = match perform(&operation.name, &operation.args) {
        Ok(plain) => Ok(workspace.from_plain(&plain)),
        Err(failure) => {
            let mut error = Diagnostic::error(failure.message, Span::new(0, 0));
            error.notes = failure.notes;
            Err(error)
        }
    };
    graph.resolve(node, outcome);
}
```

The operation's `input` and `span` fields say where the program called
it: the input number of the file and the span of the call, such as
`m.run("hostname")`. Use them in your logs and reports.

Operations reach you in program order, one at a time for each unit,
because GCSL doesn't know whether their order matters. Several units,
such as the tests of a suite, each stop at their own operation, so you
can work on one operation for each unit at the same time. Don't answer
`true` from `Effects::isolates` for `Builtin::Operation` unless no order
among a unit's operations can matter.

## Read and make handles

A handle is plain data that only your host makes. Make one with
`Plain::handle`, from the kind in the `extern type` declaration and a
number you choose. Read one that a program passes back with
`Plain::as_handle`, which returns the kind and the number, or `None` for
any other value:

```rust
fn perform(name: &str, args: &[Plain]) -> Result<Plain, OperationFailure> {
    match (name, args) {
        ("vm.boot", [Plain::Str(machine), Plain::Int(cpus)]) => {
            let id = start_machine(machine, *cpus)?;
            Ok(Plain::handle("vm.machine", id))
        }
        ("halt", [machine]) => {
            let Some(("vm.machine", id)) = machine.as_handle() else {
                return Err(OperationFailure::new("`halt` needs a machine"));
            };
            stop_machine(id);
            Ok(Plain::Empty)
        }
        _ => Err(OperationFailure::new(format!(
            "this host has no operation `{name}`"
        ))),
    }
}
```

`start_machine` and `stop_machine` stand for your host's own code.

Two handles are equal in a program when their kinds and numbers are
equal, so give each thing you track one number for as long as programs
can hold its handle. Answer an operation declared to return `{}` with
`Plain::Empty`.

A handle means something only in the run that made it, so it can't be an
input of a step. If a program gives one to a deferred call, as an
argument, a captured variable, or a context value, `step_inputs` and
`step_identity` return the error "a step's input must be plain data: it
holds a handle of the kind `vm.machine`, which is not data". To check a
value yourself before you record it, `Plain::handle_within` returns the
kind of a handle it holds at any depth, or `None`.

GCSL checks every result against the operation's declared return type.
If your host returns the wrong shape, the program's call fails with an
error that names the operation, for example "the result of operation
`vm.boot` is not a Vm". The checker has already checked the arguments.

## Report a failure

To fail an operation, give its node an error instead of a value:
`graph.resolve(node, Err(diagnostic))`. The program sees the
diagnostic's message and notes. GCSL sets the place itself, to the
program's call of the operation, so the span you build the diagnostic
with doesn't matter.

The failure is raised in the program like any other. A `recover` or a
`try` around the call catches it, and your `Effects::caught` is told of
each failure a program catches, so that your reports can show a failure
the program went on from.

## Answer operations that run as plain GCSL

GCSL calls `Effects::operation` wherever an operation runs as plain
GCSL, outside graph mode:

- A step that you run whole with `Workspace::evaluate` calls it for each
  operation in the step's body.
- An operation's node that you run as a job, with
  `Workspace::effect_job` and `EffectJob::run`, calls it on the thread
  that runs the job.

Implement it to answer both with the dispatch you already have:

```rust
impl Effects for Machines {
    fn operation(&self, name: &str, args: &[Plain]) -> Result<Plain, OperationFailure> {
        perform(name, args)
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}
```

An `OperationFailure` holds a message and notes, which you add with
`OperationFailure::with_note`. The default implementation has no
operations and fails each one with "this host has no operation
`vm.boot`". That is what `gcsl run` and `Workspace::run` do, since they
run with GCSL's own `Direct` effects.

## Check the result

Run a program that calls one of your operations and check three things:

- Your host receives the operation with the name and arguments you
  expect.
- The program gets your result back, and a handle you returned comes
  back to you in a later operation with the same kind and number.
- A failure you report reaches the program at its call, and a `recover`
  around the call catches it.
