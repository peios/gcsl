//! Awaits and futures in the machine, as the walkers evaluate them (see
//! [Futures](crate::eval::futures#futures)): a `lazy`'s body runs in an
//! activation of its own, which owns the captures it took out of the
//! future while it runs.

use std::rc::Rc;

use crate::CALL_LEVELS;
use crate::MAX_DEPTH;
use crate::ir::{IrId, IrKind, LazyShapeId};
use crate::value::{Begin, Future, FutureState, Value};

use super::super::graph::NodeId;
use super::super::kinds::of_kind;
use super::super::{depends_on_itself, nested_too_deeply};
use super::{Activation, Frame, Machine, Next, Outcome, fail, give};

/// An await running in the machine.
#[derive(Clone, Copy)]
pub(super) struct AwaitState {
    /// The `Await` node.
    node: IrId,
}

/// A `lazy`'s body running in the machine for an await.
pub(super) struct FutureRun {
    /// The await.
    state: AwaitState,
    /// The future.
    future: Rc<Future>,
    /// Where the body's frame starts on the value stack.
    base: usize,
    /// The context the await was evaluated in, restored once the body
    /// ends.
    outer: Option<Rc<crate::value::ContextFrame>>,
    /// Whether the body is a graph-mode scope of its own.
    scoped: bool,
}

impl Machine<'_> {
    /// Evaluates the `Await` node `id` of `value`: its value, here unless
    /// it is simple, and then the await of it.
    pub(super) fn await_node(&mut self, id: IrId, value: IrId) -> Next {
        let state = AwaitState { node: id };
        if self.running().ir.is_simple(value) {
            return match self.walker(|walker| walker.value(value)) {
                Ok(value) => self.await_fully(state, value),
                Err(unwind) => fail(unwind),
            };
        }
        self.frames.push(Frame::Await(state));
        Next::Eval(value)
    }

    /// Takes up the await `state` with `result`, the value it awaits.
    pub(super) fn awaited(&mut self, state: AwaitState, result: Outcome) -> Next {
        match result {
            Ok(value) => self.await_fully(state, value),
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Awaits `value` fully for the await `state`, as
    /// [`Evaluator::await_fully`](super::Evaluator::await_fully) does:
    /// while it is a future, runs it, the body here; and then awaits the
    /// result deeply, if the await has a lazy shape, by a walker.
    fn await_fully(&mut self, state: AwaitState, value: Value) -> Next {
        let mut value = value;
        loop {
            let future = match value {
                Value::Future(future) => future,
                present => return self.await_deep(state, present),
            };
            match self.begin_future(state, future) {
                Ok(present) => value = present,
                Err(next) => return next,
            }
        }
    }

    /// Begins running `future` for the await `state`, as
    /// [`Evaluator::run_future`](super::Evaluator::run_future) does: the
    /// value it recorded, if it has run; or its body, pushed, in an
    /// activation of its own; or the error of an await of a running future,
    /// a step's failure, or the stop for a step's result not given yet,
    /// each as what the machine does next.
    fn begin_future(&mut self, state: AwaitState, future: Rc<Future>) -> Result<Value, Next> {
        let Some(code) = future.code.find_function(future.function) else {
            return Err(self.mistyped(state.node, "a future of a function its IR lacks"));
        };
        let (height, frame_size, body) =
            (code.height as usize, code.frame_size as usize, code.body);
        let depth = self.await_depth(state.node);
        let entering = self.running().levels + usize::from(depth) + CALL_LEVELS;
        if entering + height > MAX_DEPTH {
            return Err(fail(nested_too_deeply(self.running().ir.get(state.node))));
        }
        let captures = match future.begin() {
            Some(Begin::Done(value)) => return Ok(value),
            Some(Begin::Running) => {
                return Err(fail(depends_on_itself(self.running().ir.get(state.node))));
            }
            Some(Begin::Pending(captures)) => captures,
            Some(Begin::Host(waiting)) => {
                let node = state.node;
                return Err(give(self.walker(|walker| {
                    walker.pause(NodeId::of(waiting), walker.ir.get(node))
                })));
            }
            Some(Begin::Failed(error)) => return Err(fail(error)),
            None => return Err(self.mistyped(state.node, "a future's state is in use")),
        };
        let base = self.stack.len();
        self.stack.resize(base + frame_size, Value::Empty);
        let outer = self.shared.context.replace(future.context.clone());
        let scoped = self.walker(|walker| {
            let scoped = walker.enter_scope_of(&future.code, future.function, &captures);
            walker.profile_begin((&future.code, future.function, code), true);
            scoped
        });
        self.activations.push(Activation {
            ir: Rc::clone(&future.code),
            base,
            callee: None,
            owned: Some(captures),
            caller: future.made,
            levels: entering,
        });
        self.frames.push(Frame::Future(Box::new(FutureRun {
            state,
            future,
            base,
            outer,
            scoped,
        })));
        Err(Next::Eval(body))
    }

    /// Takes up the future `run` with `result`, what its body came to: the
    /// future records the value, or is left unrun with its captures, and
    /// the await goes on with the value.
    pub(super) fn future_done(&mut self, run: FutureRun, result: Outcome) -> Next {
        let captures = self
            .activations
            .pop()
            .and_then(|activation| activation.owned)
            .unwrap_or_default();
        let FutureRun {
            state,
            future,
            base,
            outer,
            scoped,
        } = run;
        self.walker(|walker| {
            walker.profile_end();
            walker.leave_scope(scoped);
        });
        self.shared.context.replace(outer);
        self.stack.truncate(base);
        let span = self.running().ir.get(state.node).span;
        match result.or_else(|unwind| unwind.returned(span)) {
            Ok(value) => {
                // The captures are dropped: a future that has run holds
                // only its value.
                if future.finish(FutureState::Done(value.clone())) {
                    self.await_fully(state, value)
                } else {
                    self.mistyped(state.node, "a future's state is in use")
                }
            }
            Err(error) => {
                // Left unrun, so that a later await runs it again.
                if future.finish(FutureState::Pending(captures)) {
                    let node = state.node;
                    fail(
                        self.walker(|walker| {
                            walker.awaited_error(error, &future, walker.ir.get(node))
                        }),
                    )
                } else {
                    self.mistyped(state.node, "a future's state is in use")
                }
            }
        }
    }

    /// Ends the await `state` of `value`, awaited fully: awaited deeply,
    /// by a walker, if the await has a lazy shape, and checked to be of the
    /// kind the node states, as
    /// [`Evaluator::await_node`](super::Evaluator::await_node) checks it.
    fn await_deep(&mut self, state: AwaitState, value: Value) -> Next {
        let node = state.node;
        give(self.walker(|walker| {
            let node = walker.ir.get(node);
            let IrKind::Await {
                shape, depth, kind, ..
            } = node.kind
            else {
                return super::super::mistyped(node, "a node that is not an await awaited");
            };
            let value = if shape == LazyShapeId::NONE {
                value
            } else {
                walker.await_deep(value, shape, (depth, 1, false), node)?
            };
            if of_kind(&value, kind) {
                Ok(value)
            } else {
                super::super::mistyped(
                    node,
                    "an await produced a value of another kind than its node",
                )
            }
        }))
    }

    /// The depth of the await node `id` in its frame's body.
    fn await_depth(&self, id: IrId) -> u16 {
        match self.running().ir.get(id).kind {
            IrKind::Await { depth, .. } => depth,
            _ => 0,
        }
    }
}
