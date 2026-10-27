//! Loops and stops in the machine: `for`, condition loops, `break`,
//! `continue`, `return` and what catches one, as the walkers evaluate them
//! (see [Stops](crate::eval::loops#stops)).

use crate::check::Slot;
use crate::ir::{IrId, IrKind, IrRange, Operands};
use std::cell::RefCell;
use std::rc::Rc;

use crate::diagnostic::Mark as ErrorMark;
use crate::value::{List, Value};

use super::super::{Elements, FOR_RESERVE, Flow, Stop, StopKind, Unwind, caught};
use super::pieces::is_cancelled;
use super::tasks::{Begun, Join, Marks, Status, path_of};
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
    /// The values collected so far, each split-off iteration's place held
    /// by `{}` until it delivers.
    values: Vec<Value>,
    /// How many elements it has taken: the iteration running is the last.
    index: usize,
    /// Whether its iterations may be split off (see
    /// [Tasks](super::tasks#tasks)): its node is forkable, in graph mode,
    /// in a function's body.
    forkable: bool,
    /// Where the iteration running began, if it may be split off.
    begun: Begun,
    /// Its iterations split off, once one is.
    join: Option<Rc<RefCell<Join>>>,
    /// Its own failure, once iterations are split off, by its iteration.
    failure: Option<(usize, Box<Unwind>)>,
}

impl ForState {
    /// The iteration it is at, as part of a task's path, if its iterations
    /// may be split off, whether or not any has been (see
    /// [Tasks](super::tasks#tasks)).
    pub(super) fn split_at(&self) -> Option<u32> {
        self.forkable
            .then(|| u32::try_from(self.index.saturating_sub(1)).unwrap_or(u32::MAX))
    }

    /// Whether the iteration running may be split off: where it began is
    /// marked.
    pub(super) const fn marked(&self) -> bool {
        self.begun.is_marked()
    }

    /// Where the iteration running began, if it may be split off.
    pub(super) fn marks_mut(&mut self) -> Option<&mut Marks> {
        self.begun.marks_mut()
    }

    /// Its iterations split off, once one is.
    pub(super) const fn join(&self) -> Option<&Rc<RefCell<Join>>> {
        self.join.as_ref()
    }

    /// Whether an iteration split off has failed, so that what the loop
    /// does after it would not be done in program order.
    pub(super) fn failed(&self) -> bool {
        self.join
            .as_ref()
            .is_some_and(|join| join.borrow().failed.is_some())
    }
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
        let running = self.running();
        let forkable = matches!(ir.get(id).kind, IrKind::For { forkable: true, .. })
            && self.shared.graph.is_some()
            && (running.callee.is_some() || running.owned.is_some());
        let mut state = Box::new(ForState {
            node: id,
            slot,
            guard,
            body,
            collect,
            elements: None,
            values: Vec::new(),
            index: 0,
            forkable,
            begun: Begun::default(),
            join: None,
            failure: None,
        });
        if let IrKind::Range(inclusive, operands) = ir.get(iterable).kind
            && !ir.is_simple(iterable)
        {
            // Counted, not built, as the walker counts it, its bounds here.
            self.frames
                .push(Frame::ForRange(state, inclusive, operands, None));
            return Next::Eval(operands.lhs);
        }
        if ir.is_simple(iterable) {
            return match self.walker(|walker| walker.elements(iterable)) {
                Ok(elements) => self.for_begin(state, elements),
                Err(unwind) => fail(unwind),
            };
        }
        state.elements = None;
        self.frames.push(Frame::ForIterable(state));
        Next::Eval(iterable)
    }

    /// Takes up the `for` `state` over a range, included if `inclusive`, of
    /// `operands`, with `result`, the value of its left bound, or, if
    /// `start` is that, of its right one.
    pub(super) fn for_range(
        &mut self,
        state: Box<ForState>,
        (inclusive, operands, start): (bool, Operands, Option<i64>),
        result: Outcome,
    ) -> Next {
        let bound = match result {
            Ok(Value::Int(bound)) => bound,
            Ok(_) => return self.mistyped(state.node, "a range's bound that is not an Int"),
            Err(unwind) => return Next::Give(Err(unwind)),
        };
        match start {
            None => {
                self.frames
                    .push(Frame::ForRange(state, inclusive, operands, Some(bound)));
                Next::Eval(operands.rhs)
            }
            Some(start) => self.for_begin(state, Elements::range(start, bound, inclusive)),
        }
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
            // Once an iteration split off has failed, no later one runs.
            if state
                .join
                .as_ref()
                .is_some_and(|join| join.borrow().failed.is_some())
            {
                return self.for_end(state);
            }
            let Some(element) = state.elements.as_mut().and_then(Iterator::next) else {
                return self.for_end(state);
            };
            state.index += 1;
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
                return self.for_body_begin(state);
            }
            let body = state.body;
            let result = self.walker(|walker| walker.value(body)).map_err(Box::new);
            match collected(&mut state, result) {
                Ok(true) => return self.for_end(state),
                Ok(false) => {}
                Err(unwind) if state.join.is_some() => {
                    state.failure = Some((state.index.saturating_sub(1), Box::new(unwind)));
                    return self.for_end(state);
                }
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up the `for` `state` with `result`, the value of its guard.
    pub(super) fn for_guard(&mut self, state: Box<ForState>, result: Outcome) -> Next {
        match result {
            Ok(Value::Bool(true)) => self.for_body_begin(state),
            Ok(Value::Bool(false)) => self.for_next(state),
            Ok(_) => self.mistyped(state.node, "a guard that is not a Bool"),
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Takes up the `for` `state` with `result`, what its body came to.
    pub(super) fn for_body(&mut self, mut state: Box<ForState>, result: Outcome) -> Next {
        if state.begun.end() {
            self.walker(|walker| walker.leave_scope(true));
        }
        match collected(&mut state, result) {
            Ok(true) => self.for_end(state),
            Ok(false) => self.for_next(state),
            Err(unwind) if state.join.is_some() => {
                state.failure = Some((state.index.saturating_sub(1), Box::new(unwind)));
                self.for_end(state)
            }
            Err(unwind) => fail(unwind),
        }
    }

    /// Begins the body of the iteration running of `state`: if its
    /// iterations may be split off, where it began is marked, and it runs
    /// in a graph-mode scope of its own, by its number.
    fn for_body_begin(&mut self, state: Box<ForState>) -> Next {
        let body = state.body;
        if !state.forkable {
            self.frames.push(Frame::ForBody(state));
            return Next::Eval(body);
        }
        let key = (
            Value::Int(i64::from(self.running().ir.get(state.node).span.start)),
            Value::Int(i64::try_from(state.index).unwrap_or(i64::MAX)),
        );
        self.frames.push(Frame::ForBody(state));
        let marks = self.marks();
        self.walker(|walker| walker.enter_scope(&[&key.0, &key.1]));
        if let Some(Frame::ForBody(state)) = self.frames.last_mut() {
            state.begun.mark(marks);
        }
        Next::Eval(body)
    }

    /// Ends `state`, its elements taken: its value, or, if iterations were
    /// split off, once they have all delivered, theirs and its own in
    /// iteration order (see [Tasks](super::tasks#tasks)).
    fn for_end(&mut self, state: Box<ForState>) -> Next {
        let Some(join) = state.join.clone() else {
            return Next::Give(Ok(loop_result(state.collect, state.values)));
        };
        if let Some((failed, unwind)) = &state.failure {
            // A cancellation of the loop's own cancels every iteration.
            let every = is_cancelled(unwind);
            let later: Vec<u32> = join
                .borrow()
                .children
                .iter()
                .filter(|child| (every || child.0 > *failed) && child.3.is_none())
                .map(|child| child.2)
                .collect();
            self.cancel(&later);
        }
        if join.borrow().live > 0 {
            self.frames.push(Frame::ForJoin(state));
            return Next::Park(Box::new(Status::Joining));
        }
        Self::join_result(*state)
    }

    /// Takes up `state`, waiting for its children, with `result`: they have
    /// all delivered, or the task is being cancelled, which cancels them.
    pub(super) fn joined(&mut self, state: ForState, result: Outcome) -> Next {
        match result {
            Ok(_) => Self::join_result(state),
            Err(unwind) => {
                let live: Vec<u32> = state
                    .join
                    .as_ref()
                    .map(|join| {
                        join.borrow()
                            .children
                            .iter()
                            .filter(|child| child.3.is_none())
                            .map(|child| child.2)
                            .collect()
                    })
                    .unwrap_or_default();
                self.cancel(&live);
                Next::Give(Err(unwind))
            }
        }
    }

    /// The value of `state`, its children all delivered: the first failure
    /// in iteration order, its own or a child's, or else its values, each
    /// child's in its place, as a loop run in order would give them.
    fn join_result(mut state: ForState) -> Next {
        let children = state
            .join
            .take()
            .map(|join| std::mem::take(&mut join.borrow_mut().children))
            .unwrap_or_default();
        let mut first = state.failure.take();
        let mut dropped = Vec::new();
        for (iteration, position, _, outcome) in children {
            match outcome.map(|outcome| outcome.map_err(|unwind| *unwind)) {
                Some(Ok(value)) => {
                    if let Some(place) = state.values.get_mut(position) {
                        *place = value;
                    }
                }
                Some(Err(Unwind::Stop(Stop {
                    to: 0,
                    kind: StopKind::Continue,
                    value,
                }))) => match (value, state.values.get_mut(position)) {
                    (Some(value), Some(place)) => *place = value,
                    _ => dropped.push(position),
                },
                Some(Err(Unwind::Error(error))) if error.mark == ErrorMark::CANCELLED => {}
                Some(Err(unwind)) => {
                    if first.as_ref().is_none_or(|(at, _)| iteration < *at) {
                        first = Some((iteration, Box::new(unwind)));
                    }
                }
                None => dropped.push(position),
            }
        }
        if let Some((_, unwind)) = first {
            return Next::Give(Err(unwind));
        }
        if state.collect {
            dropped.sort_unstable();
            for position in dropped.into_iter().rev() {
                if position < state.values.len() {
                    state.values.remove(position);
                }
            }
        }
        Next::Give(Ok(loop_result(state.collect, state.values)))
    }

    /// Splits the innermost iteration on the task's frames that may be
    /// split off, which stops for `wait`, to take up after `resume`, into a
    /// child, and goes on with its loop's next iteration; or, if there is
    /// none, gives `wait` and `resume` back (see [Tasks](super::tasks#tasks)).
    pub(super) fn fork_for(&mut self, at: usize, status: Status) -> Result<Next, Status> {
        let (parent, child) = (self.meta.id, self.next_task);
        let Some(Frame::ForBody(state)) = self.frames.get_mut(at) else {
            return Err(status);
        };
        let join = Rc::clone(
            state
                .join
                .get_or_insert_with(|| Rc::new(RefCell::new(Join::new(parent)))),
        );
        let position = if state.collect {
            state.values.push(Value::Empty);
            state.values.len() - 1
        } else {
            0
        };
        let number = {
            let mut joined = join.borrow_mut();
            joined
                .children
                .push((state.index.saturating_sub(1), position, child, None));
            joined.live += 1;
            joined.children.len() - 1
        };
        let Some(marks) = state.begun.take() else {
            return Err(status);
        };
        let base = path_of(
            &self.meta,
            self.frames.get(..=at).unwrap_or_default(),
            &self.calls,
        );
        self.split(&marks, (join, number), status, base);
        match self.frames.pop() {
            Some(Frame::ForBody(state)) => Ok(self.for_next(state)),
            _ => Ok(self.lost_call()),
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
