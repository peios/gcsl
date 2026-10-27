//! Loops and stops in the machine: `for`, condition loops, `break`,
//! `continue`, `return` and what catches one, as the walkers evaluate them
//! (see [Stops](crate::eval::loops#stops)).

use crate::check::Slot;
use crate::ir::{IrId, IrKind, IrRange};
use crate::value::{List, Value};

use super::super::{Elements, FOR_RESERVE, Flow, Stop, StopKind, Unwind, caught};
use super::{Frame, Machine, Next, Outcome, fail, give};

/// A `for` running in the machine.
pub(super) struct ForState {
    /// The `For` node.
    node: IrId,
    /// The slot of its variable.
    slot: Slot,
    /// Its guard, if it has one.
    guard: Option<IrId>,
    /// Its body.
    body: IrId,
    /// Whether it collects its body's values.
    collect: bool,
    /// The elements left; none until its iterable is evaluated.
    elements: Option<Elements>,
    /// The values collected so far.
    values: Vec<Value>,
}

/// A condition loop running in the machine.
pub(super) struct LoopState {
    /// The `Loop` node.
    node: IrId,
    /// Its parts: its condition, its body and the items after it.
    parts: IrRange,
    /// Whether it collects what its iterations contribute.
    collect: bool,
    /// Whether its condition is tested before the next iteration.
    test: bool,
    /// The values collected so far.
    values: Vec<Value>,
}

/// Where a condition loop is, for the frame that waits on a part of it.
#[derive(Clone, Copy)]
pub(super) enum LoopAt {
    /// Its condition.
    Condition,
    /// Its body.
    Body,
    /// The item after its body at this index of its parts.
    After(u32),
}

impl Machine<'_> {
    /// Evaluates the `For` node `id`, whose variable is in `slot` and whose
    /// parts are `parts`, as [`Evaluator::for_loop`](super::Evaluator::for_loop)
    /// does. An iterable that is a range is counted by a walker, as is any
    /// simple iterable; any other is evaluated here, and must be a list.
    pub(super) fn for_loop(
        &mut self,
        id: IrId,
        (slot, parts, collect): (Slot, IrRange, bool),
    ) -> Next {
        let ir = &self.running().ir;
        let (iterable, guard, body) = match *ir.items(parts) {
            [iterable, body] => (iterable, None, body),
            [iterable, guard, body] => (iterable, Some(guard), body),
            _ => return self.mistyped(id, "a `for` without its iterable and body"),
        };
        let mut state = Box::new(ForState {
            node: id,
            slot,
            guard,
            body,
            collect,
            elements: None,
            values: Vec::new(),
        });
        let counted = matches!(ir.get(iterable).kind, IrKind::Range(..));
        if counted || ir.is_simple(iterable) {
            return match self.walker(|walker| walker.elements(iterable)) {
                Ok(elements) => self.for_begin(state, elements),
                Err(unwind) => fail(unwind),
            };
        }
        state.elements = None;
        self.frames.push(Frame::ForIterable(state));
        Next::Eval(iterable)
    }

    /// Takes up the `for` `state` with `result`, its iterable's value.
    pub(super) fn for_iterable(&mut self, state: Box<ForState>, result: Outcome) -> Next {
        match result {
            Ok(Value::List(list)) => self.for_begin(state, Elements::List(list, 0)),
            Ok(_) => self.mistyped(state.node, "a `for` over a value that is not a list"),
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Begins the iterations of the `for` `state` over `elements`.
    fn for_begin(&mut self, mut state: Box<ForState>, elements: Elements) -> Next {
        if state.collect && state.guard.is_none() {
            // As the walker reserves: no more than this up front.
            state
                .values
                .reserve_exact(elements.remaining().min(FOR_RESERVE));
        }
        state.elements = Some(elements);
        self.for_next(state)
    }

    /// Runs the iterations of the `for` `state` from its next element on,
    /// the simple guards and bodies at once, until one needs a frame or
    /// the elements run out.
    pub(super) fn for_next(&mut self, mut state: Box<ForState>) -> Next {
        loop {
            let deadline = self.walker(|walker| {
                if walker.past_deadline() {
                    walker.timed_out::<()>(walker.ir.get(state.node))
                } else {
                    Ok(())
                }
            });
            if let Err(unwind) = deadline {
                return fail(unwind);
            }
            let Some(element) = state.elements.as_mut().and_then(Iterator::next) else {
                return Next::Give(Ok(loop_result(state.collect, state.values)));
            };
            let at = self.running().base + state.slot.index();
            let Some(variable) = self.stack.get_mut(at) else {
                let span = self.running().ir.get(state.node).span;
                let error = super::super::internal("a `for`'s variable is outside the frame", span);
                return fail(error);
            };
            *variable = element;
            if let Some(guard) = state.guard {
                if !self.running().ir.is_simple(guard) {
                    self.frames.push(Frame::ForGuard(state));
                    return Next::Eval(guard);
                }
                // A stop out of the guard is for a loop around this one.
                match self.walker(|walker| walker.bool(guard)) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(unwind) => return fail(unwind),
                }
            }
            if !self.running().ir.is_simple(state.body) {
                let body = state.body;
                self.frames.push(Frame::ForBody(state));
                return Next::Eval(body);
            }
            let body = state.body;
            let result = self.walker(|walker| walker.value(body)).map_err(Box::new);
            match collected(&mut state, result) {
                Ok(true) => return Next::Give(Ok(loop_result(state.collect, state.values))),
                Ok(false) => {}
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up the `for` `state` with `result`, the value of its guard.
    pub(super) fn for_guard(&mut self, state: Box<ForState>, result: Outcome) -> Next {
        match result {
            Ok(Value::Bool(true)) => {
                let body = state.body;
                self.frames.push(Frame::ForBody(state));
                Next::Eval(body)
            }
            Ok(Value::Bool(false)) => self.for_next(state),
            Ok(_) => self.mistyped(state.node, "a guard that is not a Bool"),
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Takes up the `for` `state` with `result`, what its body came to.
    pub(super) fn for_body(&mut self, mut state: Box<ForState>, result: Outcome) -> Next {
        match collected(&mut state, result) {
            Ok(true) => Next::Give(Ok(loop_result(state.collect, state.values))),
            Ok(false) => self.for_next(state),
            Err(unwind) => fail(unwind),
        }
    }

    /// Evaluates the `Loop` node `id`, whose parts are `parts`, as
    /// [`Evaluator::condition_loop`](super::Evaluator::condition_loop) does.
    pub(super) fn condition_loop(
        &mut self,
        id: IrId,
        (parts, collect, test_first): (IrRange, bool, bool),
    ) -> Next {
        if self.running().ir.items(parts).len() < 2 {
            return self.mistyped(id, "a loop without its condition and body");
        }
        self.loop_iteration(Box::new(LoopState {
            node: id,
            parts,
            collect,
            test: test_first,
            values: Vec::new(),
        }))
    }

    /// Begins an iteration of the condition loop `state`: its deadline, and
    /// its condition, if it is tested, or else its body.
    fn loop_iteration(&mut self, mut state: Box<LoopState>) -> Next {
        let deadline = self.walker(|walker| {
            if walker.past_deadline() {
                walker.timed_out::<()>(walker.ir.get(state.node))
            } else {
                Ok(())
            }
        });
        if let Err(unwind) = deadline {
            return fail(unwind);
        }
        if !state.test {
            state.test = true;
            return self.loop_part(state, LoopAt::Body);
        }
        self.loop_part(state, LoopAt::Condition)
    }

    /// Evaluates the part `at` of the condition loop `state`.
    fn loop_part(&mut self, state: Box<LoopState>, at: LoopAt) -> Next {
        let index = match at {
            LoopAt::Condition => 0,
            LoopAt::Body => 1,
            LoopAt::After(index) => index as usize,
        };
        let Some(&part) = self.running().ir.items(state.parts).get(index) else {
            return self.loop_iteration(state);
        };
        self.frames.push(Frame::Loop(state, at));
        Next::Eval(part)
    }

    /// Takes up the condition loop `state` with `result`, what its part `at`
    /// came to.
    pub(super) fn loop_resume(
        &mut self,
        mut state: Box<LoopState>,
        at: LoopAt,
        result: Outcome,
    ) -> Next {
        match at {
            LoopAt::Condition => match result {
                Ok(Value::Bool(true)) => self.loop_part(state, LoopAt::Body),
                Ok(Value::Bool(false)) => Next::Give(Ok(loop_result(state.collect, state.values))),
                Ok(_) => self.mistyped(state.node, "a loop's condition that is not a Bool"),
                Err(unwind) => Next::Give(Err(unwind)),
            },
            LoopAt::Body => {
                let value = match result {
                    Ok(value) => {
                        if state.collect {
                            state.values.push(value);
                        }
                        false
                    }
                    Err(unwind) => match caught(*unwind, &mut state.values, state.collect) {
                        Ok(stopped) => stopped,
                        Err(unwind) => return fail(unwind),
                    },
                };
                if value {
                    return Next::Give(Ok(loop_result(state.collect, state.values)));
                }
                state.test = true;
                self.loop_part(state, LoopAt::After(2))
            }
            LoopAt::After(index) => match result {
                Ok(_) => self.loop_part(state, LoopAt::After(index + 1)),
                Err(unwind) => Next::Give(Err(unwind)),
            },
        }
    }

    /// Evaluates the `Break`, `Continue` or `Return` node `id`: its value,
    /// if it is not simple, here, and the stop by a walker otherwise.
    pub(super) fn leave(&mut self, id: IrId) -> Next {
        let value = match self.running().ir.get(id).kind {
            IrKind::Break { value, .. }
            | IrKind::Continue { value, .. }
            | IrKind::Return { value, .. } => value,
            _ => None,
        };
        match value {
            Some(value) if !self.running().ir.is_simple(value) => {
                self.frames.push(Frame::Leave(id));
                Next::Eval(value)
            }
            _ => give(self.walker(|walker| walker.leave(walker.ir.get(id)))),
        }
    }

    /// Takes up the stop `id` with `result`, its value: the stop that
    /// carries it, as [`Evaluator::leave`](super::Evaluator::leave) makes it.
    pub(super) fn left(&mut self, id: IrId, result: Outcome) -> Next {
        let value = match result {
            Ok(value) => value,
            Err(unwind) => return Next::Give(Err(unwind)),
        };
        let (kind, to) = match self.running().ir.get(id).kind {
            IrKind::Break { loops, .. } => (StopKind::Break, loops),
            IrKind::Continue { loops, .. } => (StopKind::Continue, loops),
            IrKind::Return { to, .. } => (StopKind::Return, to.number()),
            _ => {
                return self.mistyped(
                    id,
                    "a node that is not a `break`, `continue` or `return` left a loop",
                );
            }
        };
        fail(Unwind::Stop(Stop {
            to,
            kind,
            value: Some(value),
        }))
    }

    /// Evaluates the `Catch` node `id`, of `body`, whose `return`s are
    /// for `catch`.
    pub(super) fn catch(&mut self, body: IrId, catch: u32) -> Next {
        self.frames.push(Frame::Catch(catch));
        Next::Eval(body)
    }
}

/// Takes `result`, what an iteration's body came to, into the `for`
/// `state`: its value collected, or a stop caught as [`caught`] says.
/// Whether the loop ends.
fn collected(state: &mut ForState, result: Outcome) -> Flow<bool> {
    match result {
        Ok(value) => {
            if state.collect {
                state.values.push(value);
            }
            Ok(false)
        }
        Err(unwind) => caught(*unwind, &mut state.values, state.collect),
    }
}

/// The value of a loop once it ends that collected `values` if it
/// `collect`s, as the walkers give it.
fn loop_result(collect: bool, values: Vec<Value>) -> Value {
    if collect {
        Value::List(List::from_vec(values))
    } else {
        Value::Empty
    }
}

/// What the `Catch` for `catch` makes of `result`, what its body came to:
/// the value of a `return` for it, `{}` for one without; anything else as
/// it is.
pub(super) fn caught_return(catch: u32, result: Outcome) -> Outcome {
    match result {
        Err(unwind) => match *unwind {
            Unwind::Stop(Stop {
                kind: StopKind::Return,
                to,
                value,
            }) if to == catch => Ok(value.unwrap_or(Value::Empty)),
            unwind => Err(Box::new(unwind)),
        },
        ok => ok,
    }
}
