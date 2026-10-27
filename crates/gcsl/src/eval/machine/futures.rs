//! Awaits and futures in the machine, as the walkers evaluate them (see
//! [Futures](crate::eval::futures#futures)): a `lazy`'s body runs in an
//! activation of its own, which owns the captures it took out of the
//! future while it runs.

use std::rc::Rc;

use crate::CALL_LEVELS;
use crate::MAX_DEPTH;
use crate::ir::{IrId, IrKind, LazyShapeId};
use crate::source::Span;
use crate::value::{Begin, Future, FutureState, Value};

use super::super::graph::{NodeId, Wait};
use super::super::kinds::of_kind;
use super::super::{depends_on_itself, nested_too_deeply};
use super::tasks::{Status, Wound};
use super::{Activation, Frame, Machine, Next, Outcome, Resume, fail, give};

/// An await running in the machine: of an `Await` node, or of a step of an
/// update's path.
#[derive(Clone, Copy)]
pub(in crate::eval) struct AwaitState {
    /// The node.
    node: IrId,
    /// Its depth in its frame's body, for the nesting budget.
    depth: u16,
    /// Where an error of the future it runs is reported.
    span: Span,
}

impl AwaitState {
    /// The await of the node `node` of `ir`, an `Await`'s.
    fn of(ir: &crate::ir::Ir, node: IrId) -> Self {
        let found = ir.get(node);
        let depth = match found.kind {
            IrKind::Await { depth, .. } => depth,
            _ => 0,
        };
        Self {
            node,
            depth,
            span: found.span,
        }
    }

    /// The await at `site`, its depth and span, for the node `node`.
    pub(super) const fn at(node: IrId, (depth, span): (u16, Span)) -> Self {
        Self { node, depth, span }
    }

    /// The node, as the site where an error of its await is reported.
    fn site(self, ir: &crate::ir::Ir) -> crate::ir::IrNode {
        crate::ir::IrNode {
            kind: ir.get(self.node).kind,
            span: self.span,
        }
    }
}

impl FutureRun {
    /// Moves its frame `delta` slots up the stack, wrapping (see
    /// [Suspension](crate::eval::machine#suspension)).
    pub(super) fn rebase(&mut self, delta: usize) {
        self.base = self.base.wrapping_add(delta);
    }
}

/// What an await does once a future it began running has run.
pub(in crate::eval) enum Then {
    /// Awaits what it yields fully, as the future was the value awaited.
    Await,
    /// Awaits this value deeply again, for this await, the future one of
    /// its fields'.
    Deep(AwaitState, Value),
    /// Applies this node to its operands again, the future met on its
    /// update's path.
    Apply(Box<super::data::OperandsState>),
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
    /// What the await does once the future has run.
    then: Then,
}

impl FutureRun {
    /// Whether it is a run of `future`'s body.
    pub(super) fn runs(&self, future: *const Future) -> bool {
        std::ptr::eq(Rc::as_ptr(&self.future), future)
    }
}

impl Machine<'_> {
    /// Evaluates the `Await` node `id` of `value`: its value, here unless
    /// it is simple, and then the await of it.
    pub(super) fn await_node(&mut self, id: IrId, value: IrId) -> Next {
        let state = AwaitState::of(&self.running().ir, id);
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
    pub(super) fn await_fully(&mut self, state: AwaitState, value: Value) -> Next {
        let mut value = value;
        loop {
            let future = match value {
                Value::Future(future) => future,
                present => return self.await_deep(state, present),
            };
            match self.begin_future(state, future, (0, Then::Await)) {
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
    /// `level` is how many levels of record and list nesting below the
    /// value awaited the future is, and `then` what the await does once it
    /// has run.
    pub(super) fn begin_future(
        &mut self,
        state: AwaitState,
        future: Rc<Future>,
        (level, then): (usize, Then),
    ) -> Result<Value, Next> {
        let Some(code) = future.code.find_function(future.function) else {
            return Err(self.mistyped(state.node, "a future of a function its IR lacks"));
        };
        let (height, frame_size, body) =
            (code.height as usize, code.frame_size as usize, code.body);
        let site = state.site(&self.running().ir);
        let entering = self.running().levels + usize::from(state.depth) + level + CALL_LEVELS;
        if entering + height > MAX_DEPTH {
            return Err(fail(nested_too_deeply(&site)));
        }
        let unit = self.walker(|walker| walker.unit());
        let captures = match future.begin(unit) {
            Some(Begin::Done(value)) => return Ok(value),
            Some(Begin::Running)
                if !self
                    .shared
                    .futures
                    .borrow()
                    .iter()
                    .any(|running| Rc::ptr_eq(running, &future)) =>
            {
                // Another task of the unit runs it: waited for, and awaited
                // again; a later task's run of it is abandoned for this one.
                self.wound(Wound::Future(Rc::as_ptr(&future)));
                let waiting = Status::Waiting(resumed(state, future, then));
                return Err(Next::Park(Box::new(waiting)));
            }
            Some(Begin::Running) => return Err(fail(depends_on_itself(&site))),
            Some(Begin::Pending(captures)) => captures,
            // Awaited again once what it waits for is there.
            Some(Begin::Elsewhere(owner)) => {
                return Err(Next::Block(Wait::Unit(owner), resumed(state, future, then)));
            }
            Some(Begin::Host(waiting)) => {
                let wait = Wait::Node(NodeId::of(waiting));
                return Err(Next::Block(wait, resumed(state, future, then)));
            }
            Some(Begin::Failed(error)) => return Err(fail(error)),
            None => return Err(self.mistyped(state.node, "a future's state is in use")),
        };
        let base = self.stack.len();
        self.stack.resize(base + frame_size, Value::Empty);
        self.shared.futures.borrow_mut().push(Rc::clone(&future));
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
            then,
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
            then,
        } = run;
        self.walker(|walker| {
            walker.profile_end();
            walker.leave_scope(scoped);
        });
        self.shared.futures.borrow_mut().pop();
        self.shared.context.replace(outer);
        self.stack.truncate(base);
        match result.or_else(|unwind| unwind.returned(state.span)) {
            Ok(value) => {
                // The captures are dropped: a future that has run holds
                // only its value.
                if future.finish(FutureState::Done(value.clone())) {
                    match then {
                        Then::Await => self.await_fully(state, value),
                        Then::Deep(awaiting, awaited) => self.await_deep(awaiting, awaited),
                        Then::Apply(operands) => self.apply_again(operands),
                    }
                } else {
                    self.mistyped(state.node, "a future's state is in use")
                }
            }
            // Its run abandoned for an earlier task's (see
            // [Wound-wait](super::tasks#wound-wait)): left unrun, for that
            // task to run, and awaited again.
            Err(error)
                if self
                    .meta
                    .wound
                    .is_some_and(|(wound, _)| wound == Wound::Future(Rc::as_ptr(&future)))
                    && error.mark == crate::diagnostic::Mark::CANCELLED =>
            {
                if !future.finish(FutureState::Pending(captures)) {
                    return self.mistyped(state.node, "a future's state is in use");
                }
                if let Some((_, wounder)) = self.meta.wound.take() {
                    self.wake_task(wounder);
                }
                Next::Park(Box::new(Status::Waiting(resumed(state, future, then))))
            }
            Err(error) => {
                // Left unrun, so that a later await runs it again.
                if future.finish(FutureState::Pending(captures)) {
                    let site = state.site(&self.running().ir);
                    fail(self.walker(|walker| walker.awaited_error(error, &future, &site)))
                } else {
                    self.mistyped(state.node, "a future's state is in use")
                }
            }
        }
    }

    /// Ends the await `state` of `value`, awaited fully: awaited deeply,
    /// if the await has a lazy shape, and checked to be of the kind the
    /// node states, as
    /// [`Evaluator::await_node`](super::Evaluator::await_node) checks it.
    /// A walker walks the value as
    /// [`Evaluator::await_deep`](super::Evaluator::await_deep) does, but
    /// leaves the first future it meets that has not run to the machine,
    /// which runs it, its body here, and then walks the value again: the
    /// futures that have run are passed over, and the walk goes further.
    pub(super) fn await_deep(&mut self, state: AwaitState, value: Value) -> Next {
        let node = state.node;
        self.shared.ahead.awaiting.set(true);
        let walked = self.walker(|walker| {
            let node = walker.ir.get(node);
            let IrKind::Await {
                shape, depth, kind, ..
            } = node.kind
            else {
                return super::super::mistyped(node, "a node that is not an await awaited");
            };
            let awaited = if shape == LazyShapeId::NONE {
                value.clone()
            } else {
                walker.await_deep(value.clone(), shape, (depth, 1, false), node)?
            };
            if of_kind(&awaited, kind) {
                Ok(awaited)
            } else {
                super::super::mistyped(
                    node,
                    "an await produced a value of another kind than its node",
                )
            }
        });
        self.shared.ahead.awaiting.set(false);
        match (walked, self.shared.ahead.met.take()) {
            (
                Err(_),
                Some(super::super::Met {
                    future,
                    depth,
                    level,
                    span,
                }),
            ) => {
                let site = AwaitState::at(state.node, (depth, span));
                let then = Then::Deep(state, value.clone());
                match self.begin_future(site, future, (level, then)) {
                    // It had run after all: the walk goes on past it.
                    Ok(_) => self.await_deep(state, value),
                    Err(next) => next,
                }
            }
            (walked, _) => give(walked),
        }
    }
}

/// What a machine that stopped for `future`, which the await `state` began
/// running, does when it takes up: awaits it again, or, if it was a field's,
/// walks the value awaited deeply again, or, if an update's path met it,
/// applies the update again.
fn resumed(state: AwaitState, future: Rc<Future>, then: Then) -> Resume {
    match then {
        Then::Await => Resume::Await(state, future),
        Then::Deep(awaiting, value) => Resume::Deep(awaiting, value),
        Then::Apply(operands) => Resume::Apply(operands),
    }
}
