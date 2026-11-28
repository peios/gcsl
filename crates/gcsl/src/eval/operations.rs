//! Evaluating host operations: asking the host, and what it gives back.
//!
//! # Host operations
//!
//! A call of an operation (see the checker's [Host
//! operations](crate::check::operations#host-operations)) runs its
//! function's body, which is one call of the built-in
//! [`Builtin::Operation`](crate::check::Builtin::Operation) with the
//! operation's name and the list of its arguments, in parameter order, a
//! method's receiver first, the defaults the call gave among them, a rest
//! parameter's arguments as the one list they are. Each argument has been
//! awaited deeply, so the list holds no future.
//!
//! Plain GCSL asks the host through the run's effect context:
//! [`Effects::operation`](super::Effects::operation), given the name and
//! the arguments as plain data (see [Plain
//! values](crate::eval::plain#plain-values)), which answers with a plain
//! value or an [`OperationFailure`](super::OperationFailure), a message and
//! its notes (`Evaluator::ask_host`). The answer is made a value in the
//! names of the run's owner. A context that knows no operations, as
//! [`Direct`](super::Direct) is, fails every one: "this host has no
//! operation `vm.boot`". So `gcsl run` fails cleanly at the first operation
//! a program calls. In graph mode the call is an effect's node, which the
//! unit stops at and the host answers (see [Graph
//! mode](crate::eval::graph#graph-mode)); a host is given what the node
//! asks by [`Workspace::operation`](crate::workspace::Workspace::operation).
//!
//! Either way, what the host gave is then tested against the operation's
//! declared result, by the `As` node lowering put around the call: one of
//! another shape is the failure "the result of operation `vm.boot` is not a
//! Vm", labelled with what the value is. An operation declared to give `{}`
//! is answered with [`Plain::Empty`](super::plain::Plain::Empty).
//!
//! A failure, the host's or the test's, is raised at the body's call, and
//! the operation's function reports what is raised in its body at its own
//! call, as a function marked `#[track_caller]` does (see [Located at the
//! call](crate::eval::calls#located-at-the-call)): so it is reported where
//! the program called the operation, with the host's message and notes. In
//! graph mode a failure the host gives a node is raised with the place the
//! host gave it replaced by the call's and its labels dropped, since a host
//! knows no place in the program but the one it was told
//! (`operation_failure`). It is a failure like any other: `recover` and
//! `try` catch it, the host is told of it
//! ([`Effects::caught`](super::Effects::caught)), and `raise` raises it
//! again as it was. An operation is impure, so a memoised function that
//! calls one runs it once for each distinct list of its own arguments, and
//! `#[retry]` and `#[timeout]` on a function around one apply as to any
//! effect.
//!
//! # Handles
//!
//! A handle (see the checker's [Handle
//! types](crate::check::operations#handle-types)) is a
//! [`Value::Variant`] whose tag's enum is its handle type's key
//! ([`crate::names::extern_key`]), whose tag's key is its kind's
//! ([`crate::names::handle_key`]) and whose payload is the `Int` the host
//! chose. A host makes one with
//! [`Plain::handle`](super::plain::Plain::handle) and reads one with
//! [`Plain::as_handle`](super::plain::Plain::as_handle), and never sees the
//! encoding. Two handles are equal if they are of one kind and one number;
//! a handle is shown as `<vm.machine 3>`, its kind and its number in angle
//! brackets, which is no expression, as a sandbox's display is none.
//!
//! A handle is data to the program and to the host that made it, but it
//! means something only in the run it was made in. So a deferred call's
//! inputs, which a host keys and records the step by
//! ([`Workspace::step_inputs`](crate::workspace::Workspace::step_inputs)),
//! refuse one: "a step's input must be plain data: it holds a handle of the
//! kind `vm.machine`, which is not data"
//! ([`Plain::handle_within`](super::plain::Plain::handle_within)). A host
//! with no operations never meets one, since only an operation's answer
//! makes a handle and each of its operations fails at its call.

use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{List, Value};

use super::plain::{from_plain, to_plain};
use super::{Evaluator, Flow, internal};

impl Evaluator<'_> {
    /// Asks the run's host for its operation `name`, given `args`, for the
    /// call `node` of the built-in (see [Host
    /// operations](#host-operations)): the arguments as plain data, through
    /// the effect context, and its answer as a value, in the names of the
    /// run's owner. A failure the host gives is raised at `node`, with its
    /// notes; an argument that is no data, which the checker lets no
    /// program give, is one too. An internal error in a run with no
    /// owner's names.
    #[inline(never)]
    pub(super) fn ask_host(&self, name: &str, args: &List, node: &IrNode) -> Flow<Value> {
        let Some(names) = self.shared.names else {
            return Err(internal(
                "an operation called in a run with no names to make its result in",
                node.span,
            )
            .into());
        };
        let given = {
            let names = names
                .try_borrow()
                .map_err(|_| internal("the run's names are already borrowed", node.span))?;
            args.values()
                .iter()
                .map(|value| to_plain(value, &names))
                .collect::<Result<Vec<_>, _>>()
        };
        let given = given.map_err(|what| {
            Diagnostic::error(
                format!("an argument of operation `{name}` is not plain data"),
                node.span,
            )
            .with_note(what)
        })?;
        match self.shared.effects.operation(name, &given) {
            Ok(answer) => {
                let mut names = names
                    .try_borrow_mut()
                    .map_err(|_| internal("the run's names are already borrowed", node.span))?;
                Ok(from_plain(&answer, &mut names))
            }
            Err(failure) => {
                let mut error = Diagnostic::error(failure.message, node.span);
                error.notes = failure.notes;
                Err(error.into())
            }
        }
    }
}

/// `error`, the failure a host gave the node of an operation, as it is
/// raised at the operation's call, which spans `at` (see [Host
/// operations](#host-operations)): its message, its notes and its mark
/// kept, its place the call's and its labels dropped.
pub(super) fn operation_failure(mut error: Diagnostic, at: crate::source::Span) -> Diagnostic {
    error.span = at;
    error.input = None;
    error.foreign = false;
    error.label = None;
    error.labels.clear();
    error.suggestion = None;
    error
}
