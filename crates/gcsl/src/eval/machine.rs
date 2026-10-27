//! The machine: evaluation whose place is held as data, not on Rust's
//! call stack, so that it can stop where it waits and carry on later
//! (pekit2's `design/suspension.md`).
//!
//! # Suspension
//!
//! The recursive walkers (see the module documentation of
//! [`crate::eval`]) keep where evaluation is in Rust's own call stack, one
//! Rust frame per node being evaluated. That stack cannot be set aside and
//! taken up again, so a unit in graph mode that waits for a result can only
//! give up and run again from its start. The machine keeps the same place
//! as data instead: a stack of [`Frame`]s, each saying what to do with the
//! value of the node evaluated above it, and a stack of [`Activation`]s,
//! the calls running, each the code, the frame's base on the value stack,
//! the closure and the caller a walker would hold.
//!
//! Graph mode runs on the machine. Plain evaluation never waits, and the
//! walkers run it faster, so it runs on them, unless the crate is built
//! with the feature `machine-everywhere`, with which the test suite checks
//! the machine against them.
//!
//! The machine evaluates control nodes, those that a simple subtree cannot
//! hold ([`crate::ir::Ir::is_simple`]), with frames of its own. A simple
//! node, which can wait for nothing, it hands to the walkers, which
//! evaluate it by recursion as they always have, through an [`Evaluator`]
//! made for the activation running ([`Machine::walker`]); so is, for now,
//! every control node the machine does not evaluate itself, its
//! subtree with it. Whatever a walker changes of its activation, the
//! frame's caller, is written back when it returns.
//!
//! What the machine evaluates itself, so far:
//!
//! - a block, its items in order and then its result;
//! - an `if`, its condition and then the branch it selects;
//! - a store, its value and then the write to its slot;
//! - `Int` arithmetic, `+`, `-`, `*`, `//` and `%`, its operands in order
//!   and then the operator;
//! - a call of a closure of a plain function, one neither memoised,
//!   deferred, retried nor timed, in a run that is not profiled: its
//!   callee, its arguments in order, each in the calling frame, and then
//!   its body, in a frame of its own, exactly as
//!   [`Evaluator::run_closure`] runs it. Any other call is handed to the
//!   walkers once its callee is evaluated ([`Evaluator::call_resolved`]).
//!
//! Every error, stop and cleanup is the walkers': a frame that an
//! unwinding passes through does what the Rust function it stands for does
//! on its way out, a call's frame truncating the value stack and reporting
//! the error as the call does.

mod builtins;
mod control;
mod data;
mod futures;
mod loops;
mod suspend;

use std::rc::Rc;

use crate::diagnostic::Mark as ErrorMark;
use crate::ir::{ArgValue, FnCode, FnKind, Ir, IrId, IrKind, IrNode, IrRange, Operands};
use crate::names::Name;
use crate::value::{Caller, Closure, ContextFrame, Value};
use crate::{CALL_LEVELS, MAX_DEPTH};

pub(crate) use suspend::Suspended;
pub(super) use suspend::run_machine;

use builtins::BuiltinCall;
use suspend::Resume;
use control::{DefaultRun, WithState};
use data::OperandsState;
use futures::{AwaitState, FutureRun};
use loops::{ForState, LoopAt, LoopState};

use super::kinds::into_callee;
use super::memo::{Mark, MemoBegin, MemoTables};
use super::graph::Wait;
use super::{
    Callee, Evaluator, Flow, GUARD_LEVELS, Shared, Unwind, mistyped, nested_too_deeply,
    shown_duration,
};

/// A call running in the machine: what an [`Evaluator`] of its frame
/// holds, owned.
pub(super) struct Activation {
    /// The code running.
    ir: Rc<Ir>,
    /// Where its frame starts on the value stack.
    base: usize,
    /// The closure running; `None` at the top level and in a `lazy`'s
    /// body.
    callee: Option<Rc<Closure>>,
    /// The captured values of the `lazy` whose body is running, taken out
    /// of its future while it runs; `None` for any other frame.
    owned: Option<Box<[Value]>>,
    /// Its caller, its helper flag and the caller's origin.
    caller: Caller,
    /// The levels of nesting the enclosing frames have consumed.
    levels: usize,
}

impl Activation {
    /// The top level's activation, of `ir`, with the caller `caller`.
    pub(super) fn top(ir: &Rc<Ir>, caller: Caller) -> Self {
        Self {
            ir: Rc::clone(ir),
            base: 0,
            callee: None,
            owned: None,
            caller,
            levels: 0,
        }
    }
}

/// What to do with the value of the node evaluated above a frame.
enum Frame {
    /// The items of a block from `next` on, and then its result.
    Items {
        items: IrRange,
        next: u32,
        result: IrId,
    },
    /// The branches of the `if` `node`, its condition evaluated above.
    Branch { node: IrId, then: IrId, else_: IrId },
    /// The write of the value evaluated above to `slot`, for the `Store`
    /// `node`.
    Store {
        slot: crate::check::Slot,
        node: IrId,
    },
    /// An `Int` operator `node` of `operands`, the left one evaluated
    /// above, or, with `lhs`, the right one.
    Arithmetic {
        node: IrId,
        operands: Operands,
        lhs: Option<i64>,
    },
    /// The arguments of the innermost of the machine's calls, from `next`
    /// on, the one before evaluated above.
    Arguments,
    /// The body of the innermost of the machine's calls, evaluated above in
    /// an activation of its own.
    Body,
    /// A `for`'s iterable, evaluated above.
    ForIterable(Box<ForState>),
    /// A `for`'s guard, evaluated above.
    ForGuard(Box<ForState>),
    /// A `for`'s body, evaluated above.
    ForBody(Box<ForState>),
    /// A part of a condition loop, evaluated above.
    Loop(Box<LoopState>, LoopAt),
    /// The value of a `break`, `continue` or `return`, evaluated above.
    Leave(IrId),
    /// The body of a catch for the `return`s of this number, evaluated
    /// above.
    Catch(u32),
    /// The value of an await, evaluated above.
    Await(AwaitState),
    /// The body of a `lazy`, run above for an await.
    Future(Box<FutureRun>),
    /// An argument of a call of a built-in, evaluated above.
    BuiltinArgument(Box<BuiltinCall>),
    /// The callee of the call node, evaluated above.
    Callee(IrId),
    /// An operand of a node the machine applies, evaluated above.
    Operands(Box<OperandsState>),
    /// The left operand of an `and`, if `false`, or an `or`, evaluated
    /// above.
    Logic(Operands, bool),
    /// A binding of a `with`, of the key, evaluated above.
    WithBinding(Box<WithState>, Name),
    /// The body of a `with`, evaluated above, and the context to restore.
    WithBody(Option<Rc<ContextFrame>>),
    /// The body of a `recover` with this fallback, evaluated above.
    Recover(IrId),
    /// The fallback of a `recover`, evaluated above.
    Fallback,
    /// The default of a context read, run above as a call.
    Default(Box<DefaultRun>),
}

/// A call of a closure that the machine runs.
struct CallState {
    /// The closure called.
    closure: Rc<Closure>,
    /// The call node.
    node: IrId,
    /// Its arguments.
    args: IrRange,
    /// The argument evaluated next.
    next: u32,
    /// Its depth in the calling frame's body.
    depth: u16,
    /// Where the callee's frame starts on the value stack.
    base: usize,
    /// The levels of nesting at the callee's body.
    entering: usize,
    /// Whether the call goes from the workspace's own code into foreign
    /// code, and so is on the run's list of sites.
    crossing: bool,
    /// For a memoised call, its entry, once its lookup has started one, and
    /// whether its body is a graph-mode scope of its own.
    memo: Option<(Option<Mark>, bool)>,
    /// For a call of a function with `#[retry]` or `#[timeout]`, its
    /// attempts, once its body runs.
    guard: Option<Guard>,
    /// Whether the host takes the call as a step, in graph mode: its
    /// arguments are evaluated, and it becomes a node.
    deferred: bool,
}

/// The attempts of a call whose function has `#[retry]` or `#[timeout]`,
/// as [`Evaluator::guarded_body`] makes them.
struct Guard {
    /// How many attempts have begun.
    tried: u32,
    /// The running attempt's deadline, if the function has a timeout.
    deadline: Option<u32>,
    /// The caller the body runs with.
    caller: Caller,
    /// The levels of nesting at the body, the guard's own included.
    levels: usize,
}

/// What evaluating a node came to, in the machine: a [`Flow`] whose
/// unwinding is boxed, so that what the machine moves at every step is a
/// value's size, not an error's.
type Outcome = Result<Value, Box<Unwind>>;

/// What the machine does next.
enum Next {
    /// Evaluate a node in the activation running.
    Eval(IrId),
    /// Give the frame on top what the node above it came to.
    Give(Outcome),
    /// Stop for `Wait`, to take up where it stopped, doing `Resume`, once
    /// what it waits for is there (see [Suspension](#suspension)).
    Block(Wait, Resume),
}

/// How a run of the machine ends.
pub(super) enum Ran {
    /// With what the root came to.
    Done(Flow<Value>),
    /// Stopped for something it waits for, its frames kept, to take up
    /// doing `Resume` (see [Suspension](#suspension)).
    Blocked(Wait, Resume),
}

/// What a machine that stopped keeps of itself: its frames, activations
/// and calls (see [Suspension](#suspension)).
pub(super) struct Parked {
    /// The calls running, innermost last.
    activations: Vec<Activation>,
    /// The frames, innermost last.
    frames: Vec<Frame>,
    /// The calls the machine runs, innermost last.
    calls: Vec<CallState>,
}

/// Gives the frame on top `flow`, a walker's result.
fn give(flow: Flow<Value>) -> Next {
    Next::Give(flow.map_err(Box::new))
}

/// Gives the frame on top the unwinding `unwind`.
fn fail(unwind: impl Into<Unwind>) -> Next {
    Next::Give(Err(Box::new(unwind.into())))
}

/// A run of the machine over one value stack.
pub(super) struct Machine<'r> {
    /// The value stack, holding every frame.
    stack: &'r mut Vec<Value>,
    /// The memo tables of the execution context.
    memo: &'r mut MemoTables,
    /// What every frame of the run shares.
    shared: &'r Shared<'r>,
    /// The calls running, innermost last.
    activations: Vec<Activation>,
    /// The frames, innermost last.
    frames: Vec<Frame>,
    /// The calls the machine runs whose arguments or body are being
    /// evaluated, innermost last: one for each [`Frame::Arguments`] and
    /// [`Frame::Body`], kept here so that a call allocates nothing.
    calls: Vec<CallState>,
}

impl<'r> Machine<'r> {
    /// A machine over `stack` and `memo`, sharing `shared`, running
    /// `activation`.
    pub(super) fn new(
        (stack, memo): (&'r mut Vec<Value>, &'r mut MemoTables),
        shared: &'r Shared<'r>,
        activation: Activation,
    ) -> Self {
        Self {
            stack,
            memo,
            shared,
            activations: vec![activation],
            frames: Vec::new(),
            calls: Vec::new(),
        }
    }

    /// A machine over `stack` and `memo`, sharing `shared`, taking up
    /// `parked`, where an earlier one stopped.
    pub(super) fn parked(
        (stack, memo): (&'r mut Vec<Value>, &'r mut MemoTables),
        shared: &'r Shared<'r>,
        parked: Parked,
    ) -> Self {
        Self {
            stack,
            memo,
            shared,
            activations: parked.activations,
            frames: parked.frames,
            calls: parked.calls,
        }
    }

    /// What the machine keeps of itself once it has stopped.
    pub(super) fn into_parked(self) -> Parked {
        Parked {
            activations: self.activations,
            frames: self.frames,
            calls: self.calls,
        }
    }

    /// Evaluates `root` in the activation the machine was made with.
    pub(super) fn run(&mut self, root: IrId) -> Ran {
        self.run_from(Next::Eval(root))
    }

    /// Runs from `next` on, until the root is evaluated or the machine
    /// stops.
    fn run_from(&mut self, next: Next) -> Ran {
        let mut next = next;
        loop {
            next = match next {
                Next::Eval(id) => self.eval(id),
                Next::Give(result) => match self.frames.pop() {
                    Some(frame) => self.resume(frame, result),
                    None => return Ran::Done(result.map_err(|unwind| *unwind)),
                },
                Next::Block(wait, resume) => return Ran::Blocked(wait, resume),
            };
        }
    }

    /// The activation running.
    fn running(&self) -> &Activation {
        // The machine is made with one, and only a call's body frame, which
        // pops its own, ever pops one.
        let at = self.activations.len().saturating_sub(1);
        &self.activations[at]
    }

    /// Runs `walk` on a walker of the activation running, writing back
    /// what it changes of the activation: its caller.
    fn walker<T>(&mut self, walk: impl FnOnce(&mut Evaluator<'_>) -> T) -> T {
        let at = self.activations.len().saturating_sub(1);
        let activation = &mut self.activations[at];
        let mut evaluator = Evaluator {
            ir: &activation.ir,
            stack: self.stack,
            base: activation.base,
            captures: match (&activation.owned, &activation.callee) {
                (Some(owned), _) => owned,
                (None, Some(closure)) => &closure.captures,
                (None, None) => &[],
            },
            callee: activation.callee.as_ref(),
            memo: self.memo,
            shared: self.shared,
            caller: activation.caller,
            levels: activation.levels,
        };
        let outcome = walk(&mut evaluator);
        let caller = evaluator.caller;
        activation.caller = caller;
        outcome
    }

    /// Evaluates node `id` of the activation running: a simple node, or one
    /// the machine does not evaluate itself, by a walker, and any other by
    /// pushing the frame for it.
    fn eval(&mut self, id: IrId) -> Next {
        let ir = &self.running().ir;
        let kind = ir.get(id).kind;
        if ir.is_simple(id) {
            return give(self.walker(|walker| walker.value(id)));
        }
        match kind {
            IrKind::Block { items, result } => self.items(items, 0, result),
            IrKind::If { cond, then, else_ } => self.branch(id, cond, (then, else_)),
            IrKind::Store(slot, value) => {
                self.frames.push(Frame::Store { slot, node: id });
                Next::Eval(value)
            }
            IrKind::IntAdd(operands)
            | IrKind::IntSub(operands)
            | IrKind::IntMul(operands)
            | IrKind::IntFloorDiv(operands)
            | IrKind::IntRem(operands) => {
                self.frames.push(Frame::Arithmetic {
                    node: id,
                    operands,
                    lhs: None,
                });
                Next::Eval(operands.lhs)
            }
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call(id, (callee, args, depth)),
            IrKind::For {
                slot,
                parts,
                collect,
            } => self.for_loop(id, (slot, parts, collect)),
            IrKind::Loop {
                parts,
                collect,
                test_first,
            } => self.condition_loop(id, (parts, collect, test_first)),
            IrKind::Break { .. } | IrKind::Continue { .. } | IrKind::Return { .. } => {
                self.leave(id)
            }
            IrKind::Catch {
                body, id: catch, ..
            } => self.catch(body, catch.number()),
            IrKind::Await { value, .. } => self.await_node(id, value),
            IrKind::And(operands) => self.logic(operands, false),
            IrKind::Or(operands) => self.logic(operands, true),
            IrKind::With { parts, .. } => self.with(id, parts),
            IrKind::Recover { body, fallback, .. } => self.recover(body, fallback),
            IrKind::ContextRead {
                key,
                default,
                depth,
                ..
            } => self.context_read(id, key, (default, depth)),
            kind if Self::applies(kind) => self.operands(id),
            _ => give(self.walker(|walker| walker.value(id))),
        }
    }

    /// Takes up `frame` with `result`, what the node evaluated above it
    /// came to.
    fn resume(&mut self, frame: Frame, result: Outcome) -> Next {
        match frame {
            Frame::Items {
                items,
                next,
                result: last,
            } => match result {
                Ok(_) => self.items(items, next, last),
                Err(unwind) => Next::Give(Err(unwind)),
            },
            Frame::Branch { node, then, else_ } => match result {
                Ok(Value::Bool(cond)) => Next::Eval(if cond { then } else { else_ }),
                Ok(_) => self.mistyped(node, "a condition that is not a Bool"),
                Err(unwind) => Next::Give(Err(unwind)),
            },
            Frame::Store { slot, node } => match result {
                Ok(value) => self.store(slot, value, node),
                Err(unwind) => Next::Give(Err(unwind)),
            },
            Frame::Arithmetic {
                node,
                operands,
                lhs,
            } => self.arithmetic(node, operands, lhs, result),
            Frame::Arguments => match self.calls.pop() {
                Some(state) => self.argument_done(state, result),
                None => self.lost_call(),
            },
            Frame::Body => match self.calls.pop() {
                Some(state) => self.body_done(state, result),
                None => self.lost_call(),
            },
            Frame::ForIterable(state) => self.for_iterable(state, result),
            Frame::ForGuard(state) => self.for_guard(state, result),
            Frame::ForBody(state) => self.for_body(state, result),
            Frame::Loop(state, at) => self.loop_resume(state, at, result),
            Frame::Leave(id) => self.left(id, result),
            Frame::Catch(catch) => Next::Give(loops::caught_return(catch, result)),
            Frame::Await(state) => self.awaited(state, result),
            Frame::Future(run) => self.future_done(*run, result),
            Frame::BuiltinArgument(call) => self.builtin_argument(call, result),
            Frame::Callee(id) => self.callee_done(id, result),
            Frame::Operands(state) => self.operand(state, result),
            Frame::Logic(operands, decisive) => self.logic_lhs(operands, decisive, result),
            Frame::WithBinding(state, key) => self.with_binding(state, key, result),
            Frame::WithBody(outer) => self.with_done(outer, result),
            Frame::Recover(fallback) => self.recovered(fallback, result),
            Frame::Fallback => self.fallback_done(result),
            Frame::Default(run) => self.default_done(&run, result),
        }
    }

    /// Evaluates the items of a block from `next` on, the simple ones at
    /// once, and then its result.
    fn items(&mut self, items: IrRange, mut next: u32, result: IrId) -> Next {
        loop {
            let ir = &self.running().ir;
            let Some(&item) = ir.items(items).get(next as usize) else {
                return Next::Eval(result);
            };
            next += 1;
            if !ir.is_simple(item) {
                self.frames.push(Frame::Items {
                    items,
                    next,
                    result,
                });
                return Next::Eval(item);
            }
            if let Err(unwind) = self.walker(|walker| walker.value(item)) {
                return fail(unwind);
            }
        }
    }

    /// Evaluates an `if`: its condition, at once if it is simple, and then
    /// the branch it selects.
    fn branch(&mut self, node: IrId, cond: IrId, (then, else_): (IrId, IrId)) -> Next {
        if self.running().ir.is_simple(cond) {
            return match self.walker(|walker| walker.bool(cond)) {
                Ok(cond) => Next::Eval(if cond { then } else { else_ }),
                Err(unwind) => fail(unwind),
            };
        }
        self.frames.push(Frame::Branch { node, then, else_ });
        Next::Eval(cond)
    }

    /// Writes `value` to `slot` of the running frame, for the `Store` node
    /// `node`.
    fn store(&mut self, slot: crate::check::Slot, value: Value, node: IrId) -> Next {
        let at = self.running().base + slot.index();
        if let Some(target) = self.stack.get_mut(at) {
            *target = value;
            return Next::Give(Ok(Value::Empty));
        }
        let span = self.running().ir.get(node).span;
        fail(super::internal("store to a slot outside the frame", span))
    }

    /// Takes up the `Int` operator `node` of `operands` with `result`, the
    /// value of its left operand, or, if `lhs` is that, of its right one.
    fn arithmetic(
        &mut self,
        node: IrId,
        operands: Operands,
        lhs: Option<i64>,
        result: Outcome,
    ) -> Next {
        let value = match result {
            Ok(Value::Int(value)) => value,
            Ok(_) => return self.mistyped(node, "a non-Int operand of an Int operator"),
            Err(unwind) => return Next::Give(Err(unwind)),
        };
        let Some(lhs) = lhs else {
            self.frames.push(Frame::Arithmetic {
                node,
                operands,
                lhs: Some(value),
            });
            return Next::Eval(operands.rhs);
        };
        let pair = (lhs, value);
        give(
            self.walker(|walker| {
                let node = walker.ir.get(node);
                match node.kind {
                    IrKind::IntAdd(_) => {
                        walker.int_arithmetic_of(node, operands, i64::checked_add, pair)
                    }
                    IrKind::IntSub(_) => {
                        walker.int_arithmetic_of(node, operands, i64::checked_sub, pair)
                    }
                    IrKind::IntMul(_) => {
                        walker.int_arithmetic_of(node, operands, i64::checked_mul, pair)
                    }
                    IrKind::IntFloorDiv(_) => {
                        walker.int_division_of(node, operands, super::floor_div, pair)
                    }
                    IrKind::IntRem(_) => walker.int_division_of(
                        node,
                        operands,
                        |lhs, rhs| Some(super::floor_mod(lhs, rhs)),
                        pair,
                    ),
                    _ => mistyped(node, "a node that is not an Int operator"),
                }
            })
            .map(Value::Int),
        )
    }

    /// Evaluates the call node `id` of `callee` with the arguments `args`,
    /// at depth `depth`: its callee, and then, for a closure the host does
    /// not take as a step, its arguments and body, as
    /// [`Evaluator::call_closure`] runs them; any other callee's call is a
    /// walker's ([`Evaluator::call_resolved`]).
    fn call(&mut self, id: IrId, (callee, args, depth): (IrId, IrRange, u16)) -> Next {
        if !self.running().ir.is_simple(callee) {
            self.frames.push(Frame::Callee(id));
            return Next::Eval(callee);
        }
        match self.walker(|walker| walker.function(callee)) {
            Ok(callee) => self.call_resolved(id, callee, (args, depth)),
            Err(unwind) => fail(unwind),
        }
    }

    /// Takes up the call node `id` with `result`, the value of its callee.
    fn callee_done(&mut self, id: IrId, result: Outcome) -> Next {
        let IrKind::Call { args, depth, .. } = self.running().ir.get(id).kind else {
            return self.mistyped(id, "a callee of a node that is not a call");
        };
        match result.map(into_callee) {
            Ok(Some(callee)) => self.call_resolved(id, callee, (args, depth)),
            Ok(None) => self.mistyped(id, "a node of another kind where a function was expected"),
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Goes on with the call node `id` of `callee`, evaluated already, with
    /// the arguments `args`, at depth `depth`.
    fn call_resolved(&mut self, id: IrId, callee: Callee, (args, depth): (IrRange, u16)) -> Next {
        let closure = match callee {
            Callee::Builtin(builtin) => return self.builtin_call(id, builtin, args),
            Callee::Closure(closure) => closure,
            callee @ Callee::Tag(_) => {
                return give(self.walker(|walker| {
                    let node = walker.ir.get(id);
                    walker.call_resolved(callee, (args, depth), node)
                }));
            }
        };
        let Some(code) = closure.code.find_function(closure.function) else {
            return self.mistyped(id, "a closure of a function its IR lacks");
        };
        let deferred = self.deferred(&closure);
        let memo = code.kind == FnKind::Memo && !deferred;
        let height = code.height as usize;
        let running = self.running();
        let crossing = closure.code.is_foreign() && !running.ir.is_foreign();
        let entering = running.levels + usize::from(depth) + CALL_LEVELS;
        if crossing {
            let at = (running.ir.input(), running.ir.get(id).span);
            self.shared.sites.borrow_mut().push(at);
        }
        if let Err(unwind) = self.walker(|walker| {
            if walker.past_deadline() {
                return walker.timed_out(walker.ir.get(id));
            }
            // A step's call is not profiled, and runs no body here.
            if deferred {
                return Ok(());
            }
            walker.profile_begin((&closure.code, closure.function, code), false);
            if !memo && entering + height > MAX_DEPTH {
                walker.profile_end();
                return Err(nested_too_deeply(walker.ir.get(id)).into());
            }
            Ok(())
        }) {
            self.leave_call(crossing);
            return fail(unwind);
        }
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        self.next_argument(CallState {
            closure,
            node: id,
            args,
            next: 0,
            depth,
            base,
            entering,
            crossing,
            memo: memo.then_some((None, false)),
            guard: None,
            deferred,
        })
    }

    /// Whether the host takes a call of `closure` as a step, in graph mode.
    fn deferred(&mut self, closure: &Rc<Closure>) -> bool {
        let Some(code) = closure.code.find_function(closure.function) else {
            return false;
        };
        self.walker(|walker| walker.defers(code))
    }

    /// Evaluates the call's arguments from `state.next` on, in the calling
    /// frame, the simple ones and defaults at once, and then its body.
    fn next_argument(&mut self, mut state: CallState) -> Next {
        loop {
            let ir = &self.running().ir;
            let Some(&arg) = ir.items(state.args).get(state.next as usize) else {
                return self.enter_body(state);
            };
            if let IrKind::Arg {
                value: ArgValue::Node(value),
                ..
            } = ir.get(arg).kind
                && !ir.is_simple(value)
            {
                self.calls.push(state);
                self.frames.push(Frame::Arguments);
                return Next::Eval(value);
            }
            let written = self
                .walker(|walker| walker.argument(arg, state.base, (&state.closure, state.depth)));
            if let Err(unwind) = written {
                return self.argument_failed(&state, unwind);
            }
            state.next += 1;
        }
    }

    /// Takes up a call's arguments with `result`, the value of argument
    /// `state.next`.
    fn argument_done(&mut self, mut state: CallState, result: Outcome) -> Next {
        let value = match result {
            Ok(value) => value,
            Err(unwind) => return self.argument_failed(&state, *unwind),
        };
        let ir = &self.running().ir;
        let param = match ir.items(state.args).get(state.next as usize) {
            Some(&arg) => match ir.get(arg).kind {
                IrKind::Arg { param, .. } => Some(param),
                _ => None,
            },
            None => None,
        };
        let slot = param.and_then(|param| self.stack.get_mut(state.base + param as usize));
        let Some(slot) = slot else {
            let unwind =
                self.mistyped_unwind(state.node, "an argument for a parameter outside the frame");
            return self.argument_failed(&state, unwind);
        };
        *slot = value;
        state.next += 1;
        self.next_argument(state)
    }

    /// Ends a call whose argument failed or stopped with `unwind`: its frame
    /// goes, and the call reports an error as
    /// [`Evaluator::run_closure`] does.
    fn argument_failed(&mut self, state: &CallState, unwind: Unwind) -> Next {
        self.stack.truncate(state.base);
        let unwind = self.walker(|walker| {
            if !state.deferred {
                walker.profile_end();
            }
            walker.argument_failed(unwind, walker.ir.get(state.node))
        });
        self.leave_call(state.crossing);
        fail(unwind)
    }

    /// Goes on with the call `state`, its arguments evaluated: a memoised
    /// call's lookup, and then its body, in an activation of its own.
    fn enter_body(&mut self, mut state: CallState) -> Next {
        let Some(code) = state.closure.code.find_function(state.closure.function) else {
            let unwind = self.mistyped_unwind(state.node, "a closure of a function its IR lacks");
            return self.argument_failed(&state, unwind);
        };
        // A step's call becomes its node, and gives the future of its
        // result; its frame goes.
        if state.deferred {
            let made = self.walker(|walker| {
                let node = walker.ir.get(state.node);
                walker
                    .defer_made((&state.closure, code), state.base, node)
                    .map_err(|unwind| walker.argument_failed(unwind, node))
            });
            self.leave_call(state.crossing);
            return give(made);
        }
        let (caller, levels) = if state.memo.is_some() {
            let begun = self.walker(|walker| {
                let node = walker.ir.get(state.node);
                walker.memo_lookup((&state.closure, code), (state.base, state.depth), node)
            });
            match begun {
                Err(unwind) => return self.argument_failed(&state, unwind),
                Ok(MemoBegin::Found(value)) => return self.found(&state, value),
                // Looked up again, its arguments still in its frame, once the
                // unit that runs it has run.
                Ok(MemoBegin::Elsewhere(owner)) => {
                    self.calls.push(state);
                    return Next::Block(Wait::Unit(owner), Resume::Lookup);
                }
                Ok(MemoBegin::Started(mark)) => {
                    let scoped = self.memo_scope(&state);
                    state.memo = Some((Some(mark), scoped));
                    (mark.caller, mark.entering)
                }
                Ok(MemoBegin::Unkeyed(entering, caller)) => {
                    let scoped = self.memo_scope(&state);
                    state.memo = Some((None, scoped));
                    (caller, entering)
                }
            }
        } else {
            let caller = self.walker(|walker| {
                walker.arguments_done();
                walker.callee_caller()
            });
            (caller, state.entering)
        };
        if code.retry > 0 || code.timeout.is_some() {
            let levels = levels + GUARD_LEVELS;
            if levels + code.height as usize > MAX_DEPTH {
                let error = nested_too_deeply(self.running().ir.get(state.node));
                return self.finish(&state, Err(Box::new(error.into())));
            }
            state.guard = Some(Guard {
                tried: 0,
                deadline: None,
                caller,
                levels,
            });
            return self.attempt(state);
        }
        self.push_body(state, caller, levels)
    }

    /// Whether the body of the memoised call `state` is a scope of its own,
    /// in graph mode, keyed by its callee and arguments, entering it if so
    /// (see [Graph mode](crate::eval::graph#graph-mode)).
    fn memo_scope(&mut self, state: &CallState) -> bool {
        if self.shared.graph.is_none() {
            return false;
        }
        let Some(code) = state.closure.code.find_function(state.closure.function) else {
            return false;
        };
        let params = state.base..state.base + code.param_count as usize;
        let given: Vec<Value> = self.stack.get(params).unwrap_or_default().to_vec();
        let function = Value::Fn(Rc::clone(&state.closure));
        let mut parts: Vec<&Value> = vec![&function];
        parts.extend(given.iter());
        self.walker(|walker| walker.enter_scope(&parts))
    }

    /// Ends the memoised call `state`, whose lookup found `value`.
    fn found(&mut self, state: &CallState, value: Value) -> Next {
        self.walker(|walker| walker.profile_end());
        self.leave_call(state.crossing);
        Next::Give(Ok(value))
    }

    /// Runs the body of the call `state` with the caller `caller` at
    /// `levels` of nesting, in an activation of its own.
    fn push_body(&mut self, state: CallState, caller: Caller, levels: usize) -> Next {
        let Some(code) = state.closure.code.find_function(state.closure.function) else {
            let unwind = self.mistyped_unwind(state.node, "a closure of a function its IR lacks");
            return self.finish(&state, Err(Box::new(unwind)));
        };
        let body = code.body;
        self.activations.push(Activation {
            ir: Rc::clone(&state.closure.code),
            base: state.base,
            callee: Some(Rc::clone(&state.closure)),
            owned: None,
            caller,
            levels,
        });
        self.calls.push(state);
        self.frames.push(Frame::Body);
        Next::Eval(body)
    }

    /// Begins another attempt of the guarded call `state`, as
    /// [`Evaluator::guarded_body`] does: its frame's variables emptied
    /// after the first, and under its own deadline, if it has a timeout.
    fn attempt(&mut self, mut state: CallState) -> Next {
        let Some(code) = state.closure.code.find_function(state.closure.function) else {
            let unwind = self.mistyped_unwind(state.node, "a closure of a function its IR lacks");
            return self.finish(&state, Err(Box::new(unwind)));
        };
        let Some(guard) = state.guard.as_mut() else {
            let unwind = self.mistyped_unwind(state.node, "an attempt of an unguarded call");
            return self.finish(&state, Err(Box::new(unwind)));
        };
        guard.tried = guard.tried.saturating_add(1);
        if guard.tried > 1 {
            let locals =
                state.base + code.param_count as usize..state.base + code.frame_size as usize;
            for slot in self.stack.get_mut(locals).into_iter().flatten() {
                *slot = Value::Empty;
            }
        }
        guard.deadline = code.timeout.map(|limit| {
            let message = format!("{} timed out after {}", shown(code), shown_duration(limit));
            self.walker(|walker| walker.push_deadline(limit, &message))
        });
        let (caller, levels) = (guard.caller, guard.levels);
        self.push_body(state, caller, levels)
    }

    /// Takes up the call `state` with `result`, what its body came to: a
    /// guarded call's attempt ends, and may begin another; and then the
    /// call ends.
    fn body_done(&mut self, state: CallState, result: Outcome) -> Next {
        self.activations.pop();
        let Some(guard) = state.guard.as_ref() else {
            return self.finish(&state, result);
        };
        if let Some(number) = guard.deadline {
            self.walker(|walker| walker.pop_deadline(number));
        }
        let span = self.running().ir.get(state.node).span;
        let mut error = match result.or_else(|unwind| unwind.returned(span)) {
            Ok(value) => return self.finish(&state, Ok(value)),
            Err(error) => error,
        };
        let Some(code) = state.closure.code.find_function(state.closure.function) else {
            return self.finish(&state, Err(Box::new(error.into())));
        };
        // Its own deadline's passing is its own error.
        if guard.deadline.is_some() && error.mark.deadline_number() == guard.deadline {
            error.mark = ErrorMark::NONE;
        }
        if error.mark == ErrorMark::NONE && guard.tried <= code.retry {
            return self.attempt(state);
        }
        if guard.tried > 1 && error.mark == ErrorMark::NONE {
            error
                .notes
                .push(format!("{} was tried {} times", shown(code), guard.tried));
        }
        self.finish(&state, Err(Box::new(error.into())))
    }

    /// Ends the call `state` with `result`, what its body came to, as
    /// [`Evaluator::run_closure`], [`Evaluator::guarded_call`] and
    /// [`Evaluator::memo_call`] do: its frame goes, a `return` for the
    /// function is its value, a memoised call's entry records it, and an
    /// error is reported as the calling frame reports it.
    fn finish(&mut self, state: &CallState, result: Outcome) -> Next {
        let span = self.running().ir.get(state.node).span;
        let outcome = if let Some((mark, scoped)) = state.memo {
            self.walker(|walker| walker.leave_scope(scoped));
            self.stack.truncate(state.base);
            if mark.is_some() {
                self.memo.running.pop();
            }
            let result = result.or_else(|unwind| unwind.returned(span));
            self.walker(|walker| {
                let node = walker.ir.get(state.node);
                let ended = match mark {
                    Some(mark) => walker.memo_end(&state.closure, mark, result, node),
                    None => result.map_err(|error| {
                        walker.memo_error(error, &state.closure, false, node).into()
                    }),
                };
                ended.map_err(|unwind| walker.argument_failed(unwind, node))
            })
        } else {
            self.stack.truncate(state.base);
            result
                .or_else(|unwind| unwind.returned(span))
                .map_err(|error| {
                    self.walker(|walker| {
                        let node = walker.ir.get(state.node);
                        walker.call_error(error, &state.closure, node).into()
                    })
                })
        };
        self.walker(|walker| walker.profile_end());
        self.leave_call(state.crossing);
        give(outcome)
    }

    /// The internal error of a call's frame without its call.
    fn lost_call(&self) -> Next {
        let span = self.running().ir.get(self.running().ir.root()).span;
        fail(super::internal("a call's frame without its call", span))
    }

    /// Leaves a call, taking it off the run's list of sites if it is
    /// `crossing` onto it.
    fn leave_call(&self, crossing: bool) {
        if crossing {
            self.shared.sites.borrow_mut().pop();
        }
    }

    /// The internal error of the mistyped node `id`, saying `what`.
    fn mistyped(&self, id: IrId, what: &'static str) -> Next {
        fail(self.mistyped_unwind(id, what))
    }

    /// The internal error of the mistyped node `id`, saying `what`.
    fn mistyped_unwind(&self, id: IrId, what: &'static str) -> Unwind {
        let node: &IrNode = self.running().ir.get(id);
        match mistyped::<()>(node, what) {
            Err(unwind) => unwind,
            Ok(()) => super::internal(what, node.span).into(),
        }
    }
}

/// How the function of `code` is named in its errors: "`name`", or "the
/// call" for one without a name, as [`Evaluator::guarded_body`] names it.
fn shown(code: &FnCode) -> String {
    code.name_text
        .as_deref()
        .map_or_else(|| "the call".to_owned(), |name| format!("`{name}`"))
}
