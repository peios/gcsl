//! Pieces split off: the elements of a list, the fields of a record, the
//! pieces of an interpolation and the arguments of a call, each of which,
//! in graph mode, may wait apart, as an iteration of a `for` may (see
//! [Tasks](super::tasks#tasks)).
//!
//! # Pieces
//!
//! A piece that is not simple, and is independent
//! ([`crate::ir::Ir::is_independent`]), is marked where it begins, in a
//! function's body in graph mode, and runs in a graph-mode scope of its
//! own, by the node it is a piece of and its number, so that the nodes it
//! meets are named the same whether it was split off or not. If it would
//! stop to wait, and no iteration or piece nearer the top of the task's
//! frames may be split off, it is split off into a child, which waits, and
//! the task goes on with the next piece. The child's value has its place
//! held: the element's or field's among the values so far, the
//! parameter's slot of an argument, or the built-in's argument.
//!
//! Before the node is applied, before a call's body runs or any default of
//! its parameters, which reads the parameters before it, the task waits
//! for its children ([`Joining`]), and then takes their outcomes in the
//! pieces' order: the first failure, or else each value in its place. A
//! failure of the task's own, in a later piece, waits behind the children,
//! as it comes after them in program order.

use std::cell::RefCell;
use std::rc::Rc;

use crate::diagnostic::Mark;
use crate::ir::{ArgValue, Ir, IrId, IrKind, IrRange};
use crate::value::Value;

use super::super::Unwind;
use super::builtins::BuiltinCall;
use super::data::OperandsState;
use super::tasks::{Begun, Join, Status, path_of};
use super::{CallState, Frame, Machine, Next, Outcome};

/// The pieces of a node or call, those split off once one is (see
/// [Pieces](#pieces)).
#[derive(Clone, Default)]
pub(in crate::eval) struct Pieces {
    /// The piece running, by its number.
    pub(super) at: u32,
    /// Where it began, if it may be split off.
    pub(super) begun: Begun,
    /// The pieces split off, once one is.
    pub(super) join: Option<Rc<RefCell<Join>>>,
}

impl Pieces {
    /// The piece running, as part of a task's path, if pieces were split
    /// off (see [Tasks](super::tasks#tasks)).
    pub(super) fn split_at(&self) -> Option<u32> {
        self.join.as_ref().map(|_| self.at)
    }

    /// Whether a piece split off has failed, so that what the task does
    /// after it would not be done in program order.
    pub(super) fn failed(&self) -> bool {
        self.join
            .as_ref()
            .is_some_and(|join| join.borrow().failed.is_some())
    }
}

/// What waits for its pieces split off (see [Pieces](#pieces)).
pub(in crate::eval) enum Joining {
    /// A list, record or interpolation, its operands evaluated.
    Operands(Box<OperandsState>),
    /// The innermost of the machine's calls, its arguments up to a
    /// default or their end evaluated.
    Arguments,
    /// A call of a built-in, its arguments evaluated.
    Builtin(Box<BuiltinCall>),
}

/// A node or call waiting for its pieces split off, `join`, with its own
/// failure, by its piece, if it has one.
pub(in crate::eval) struct PiecesJoin {
    /// What waits.
    what: Joining,
    /// Its pieces split off.
    join: Rc<RefCell<Join>>,
    /// Its own failure, by its piece.
    failure: Option<(usize, Box<Unwind>)>,
}

/// Whether `unwind` is a cancelled task's.
pub(super) fn is_cancelled(unwind: &Unwind) -> bool {
    matches!(unwind, Unwind::Error(error) if error.mark == Mark::CANCELLED)
}

/// Whether any of `operands`, those after a piece, is not simple: one that
/// a task could go on with while the piece waits apart. The last piece
/// that is not simple is never split off, which would gain nothing: its
/// task would go on only to wait for it.
pub(super) fn any_later(ir: &Ir, operands: &[IrId]) -> bool {
    operands.iter().any(|&operand| !ir.is_simple(operand))
}

/// As [`any_later`], of the arguments `args`, those after a piece, of
/// which only values count: a default waits for every argument before it.
pub(super) fn any_later_argument(ir: &Ir, args: &[IrId]) -> bool {
    args.iter().any(|&arg| {
        matches!(
            ir.get(arg).kind,
            IrKind::Arg { value: ArgValue::Node(value), .. } if !ir.is_simple(value)
        )
    })
}

/// Whether the pieces of a node of `kind` may be split off: a list's
/// elements, a record's fields or an interpolation's pieces.
pub(super) const fn splits(kind: IrKind) -> bool {
    matches!(
        kind,
        IrKind::MakeList { .. } | IrKind::MakeRecord { .. } | IrKind::Interp(_)
    )
}

impl Frame {
    /// Whether this frame takes one of the machine's calls, the innermost
    /// that no frame above it takes, when it resumes.
    pub(super) const fn takes_call(&self) -> bool {
        match self {
            Self::Arguments | Self::Body | Self::ArgDefault(_) => true,
            Self::PiecesJoin(state) => matches!(state.what, Joining::Arguments),
            _ => false,
        }
    }

    /// The pieces of the node or call of this frame, if it has pieces that
    /// may be split off, `call` the call it takes, if it takes one.
    pub(super) fn pieces<'f>(&'f self, call: Option<&'f CallState>) -> Option<&'f Pieces> {
        match self {
            Self::Operands(state) => Some(&state.pieces),
            Self::Arguments => call.map(|call| &call.pieces),
            Self::BuiltinArgument(call) => Some(&call.pieces),
            _ => None,
        }
    }

    /// As [`Frame::pieces`], to change, but for a call's arguments, whose
    /// pieces are its call's.
    const fn pieces_mut(&mut self) -> Option<&mut Pieces> {
        match self {
            Self::Operands(state) => Some(&mut state.pieces),
            Self::BuiltinArgument(call) => Some(&mut call.pieces),
            _ => None,
        }
    }

    /// Whether the iteration or piece of this frame running may be split
    /// off, `call` the call it takes, if it takes one.
    fn splittable(&self, call: Option<&CallState>) -> bool {
        match self {
            Self::ForBody(state) => state.marked(),
            frame => frame
                .pieces(call)
                .is_some_and(|pieces| pieces.begun.is_marked()),
        }
    }

    /// The iterations or pieces split off from the loop or node of this
    /// frame, if any were; a call's are its call's.
    pub(super) fn join(&self) -> Option<&Rc<RefCell<Join>>> {
        match self {
            Self::ForBody(state) | Self::ForJoin(state) => state.join(),
            Self::PiecesJoin(state) => Some(&state.join),
            frame => frame.pieces(None).and_then(|pieces| pieces.join.as_ref()),
        }
    }

    /// Moves where the iteration or piece of this frame running began by
    /// `delta` slots up the stack, wrapping; a call's are its call's.
    pub(super) fn rebase_marks(&mut self, delta: usize) {
        let marks = match self {
            Self::ForBody(state) => state.marks_mut(),
            frame => frame
                .pieces_mut()
                .and_then(|pieces| pieces.begun.marks_mut()),
        };
        if let Some(marks) = marks {
            marks.rebase(delta);
        }
    }
}

/// Each of `frames` with the call of `calls` it takes, if it takes one
/// (see [`Frame::takes_call`]).
pub(super) fn with_calls<'f>(
    frames: &'f [Frame],
    calls: &'f [CallState],
) -> impl Iterator<Item = (&'f Frame, Option<&'f CallState>)> {
    let mut taken = 0;
    frames.iter().map(move |frame| {
        let call = if frame.takes_call() {
            taken += 1;
            calls.get(taken - 1)
        } else {
            None
        };
        (frame, call)
    })
}

impl Machine<'_> {
    /// Whether `piece`, a piece that is not simple, may be split off: it is
    /// independent, in a function's body, in graph mode.
    pub(super) fn splittable(&self, piece: IrId) -> bool {
        let running = self.running();
        self.shared.graph.is_some()
            && (running.callee.is_some() || running.owned.is_some())
            && running.ir.is_independent(piece)
    }

    /// Begins the piece `at` of the node or call whose frame is on top:
    /// where it began is marked, and it runs in a graph-mode scope of its
    /// own.
    pub(super) fn piece_begin(&mut self, at: u32) {
        let marks = self.marks();
        if let Some(graph) = self.shared.graph {
            graph.enter_piece();
        }
        let pieces = match self.frames.last_mut() {
            Some(Frame::Arguments) => self.calls.last_mut().map(|call| &mut call.pieces),
            Some(frame) => frame.pieces_mut(),
            None => None,
        };
        if let Some(pieces) = pieces {
            pieces.at = at;
            pieces.begun.mark(marks);
        }
    }

    /// Ends the piece running of `pieces`, which came to a value or
    /// failed, if it was marked: its scope is left.
    pub(super) fn piece_end(&mut self, pieces: &mut Pieces) {
        if pieces.begun.end()
            && let Some(graph) = self.shared.graph
        {
            graph.leave();
        }
    }

    /// The innermost of the task's frames whose iteration or piece running
    /// may be split off, with the number of the call it takes, if it takes
    /// one.
    fn innermost_splittable(&self) -> Option<(usize, Option<usize>)> {
        let mut taken = 0;
        let mut found = None;
        for (at, frame) in self.frames.iter().enumerate() {
            let call = frame.takes_call().then(|| {
                taken += 1;
                taken - 1
            });
            if frame.splittable(call.and_then(|call| self.calls.get(call))) {
                found = Some((at, call));
            }
        }
        found
    }

    /// Splits the innermost iteration or piece on the task's frames that
    /// may be split off, which is to wait as `status` says, into a child,
    /// and goes on with the next; or, if there is none, gives `status`
    /// back.
    pub(super) fn fork(&mut self, status: Status) -> Result<Next, Status> {
        let Some((at, call)) = self.innermost_splittable() else {
            return Err(status);
        };
        if matches!(self.frames.get(at), Some(Frame::ForBody(_))) {
            return self.fork_for(at, status);
        }
        let parent = self.meta.id;
        let base = path_of(
            &self.meta,
            self.frames.get(..=at).unwrap_or_default(),
            &self.calls,
        );
        let pieces = match call {
            Some(call) => self.calls.get_mut(call).map(|call| &mut call.pieces),
            None => self.frames.get_mut(at).and_then(Frame::pieces_mut),
        };
        let Some(pieces) = pieces else {
            return Err(status);
        };
        let Some(marks) = pieces.begun.take() else {
            return Err(status);
        };
        let piece = pieces.at;
        let join = Rc::clone(
            pieces
                .join
                .get_or_insert_with(|| Rc::new(RefCell::new(Join::new(parent)))),
        );
        let number = join.borrow().children.len();
        self.split(&marks, (Rc::clone(&join), number), status, base);
        let child = self.next_task - 1;
        let held = |position: usize| {
            let mut joined = join.borrow_mut();
            joined
                .children
                .push((piece as usize, position, child, None));
            joined.live += 1;
        };
        // The frame split at is on top again, the piece's place held.
        Ok(match self.frames.pop() {
            Some(Frame::Operands(mut state)) => {
                held(state.values.len());
                state.values.push(Value::Empty);
                self.next_operand(state)
            }
            Some(Frame::Arguments) => match self.calls.pop() {
                Some(mut state) => {
                    let Some(param) = self.param_of(state.args, piece) else {
                        return Ok(self.lost_call());
                    };
                    held(param);
                    state.next += 1;
                    self.next_argument(state)
                }
                None => self.lost_call(),
            },
            Some(Frame::BuiltinArgument(mut call)) => {
                let Some(param) = self.param_of(call.args, piece) else {
                    return Ok(self.lost_call());
                };
                held(param);
                call.next += 1;
                self.builtin_next(call)
            }
            _ => self.lost_call(),
        })
    }

    /// The parameter of the argument `at` of `args`, by its index.
    fn param_of(&self, args: IrRange, at: u32) -> Option<usize> {
        let ir = &self.running().ir;
        let &arg = ir.items(args).get(at as usize)?;
        match ir.get(arg).kind {
            IrKind::Arg { param, .. } => Some(param as usize),
            _ => None,
        }
    }

    /// Waits for the pieces of `what` split off, `join`, if any has yet to
    /// end, and then goes on with their outcomes and `failure`, its own
    /// failure by its piece, if it has one: a cancellation, which cancels
    /// every piece split off.
    pub(super) fn join_pieces(
        &mut self,
        what: Joining,
        join: Rc<RefCell<Join>>,
        failure: Option<(usize, Box<Unwind>)>,
    ) -> Next {
        if failure
            .as_ref()
            .is_some_and(|(_, unwind)| is_cancelled(unwind))
        {
            self.cancel(&live(&join));
        }
        let state = Box::new(PiecesJoin {
            what,
            join,
            failure,
        });
        if state.join.borrow().live > 0 {
            self.frames.push(Frame::PiecesJoin(state));
            return Next::Park(Box::new(Status::Joining));
        }
        self.pieces_done(*state)
    }

    /// Takes up `state`, waiting for its pieces split off, with `result`:
    /// they have all ended, or the task is being cancelled, which cancels
    /// them.
    pub(super) fn pieces_joined(&mut self, state: PiecesJoin, result: Outcome) -> Next {
        match result {
            Ok(_) => self.pieces_done(state),
            Err(unwind) => {
                self.cancel(&live(&state.join));
                self.pieces_failed(&state.what, unwind)
            }
        }
    }

    /// Goes on with `state`, its pieces split off all ended: the first
    /// failure in the pieces' order, its own or a child's, or else each
    /// child's value in its place.
    fn pieces_done(&mut self, state: PiecesJoin) -> Next {
        let PiecesJoin {
            what,
            join,
            failure,
        } = state;
        let children = std::mem::take(&mut join.borrow_mut().children);
        let mut first = failure.filter(|(_, unwind)| !is_cancelled(unwind));
        let mut cancelled = None;
        let mut values = Vec::with_capacity(children.len());
        for (piece, position, _, outcome) in children {
            match outcome {
                Some(Ok(value)) => values.push((piece, position, value)),
                Some(Err(unwind)) if is_cancelled(&unwind) => cancelled = Some(unwind),
                Some(Err(unwind)) => {
                    if first.as_ref().is_none_or(|(at, _)| piece < *at) {
                        first = Some((piece, unwind));
                    }
                }
                None => {
                    let span = self.running().ir.get(self.running().ir.root()).span;
                    let lost = super::super::internal("a piece joined before it ended", span);
                    return self.pieces_failed(&what, Box::new(lost.into()));
                }
            }
        }
        // A cancellation, the task's or a piece's, comes out only if no
        // piece failed.
        if let Some(unwind) = first.map(|(_, unwind)| unwind).or(cancelled) {
            return self.pieces_failed(&what, unwind);
        }
        match what {
            Joining::Operands(mut state) => {
                for (_, position, value) in values {
                    if let Some(place) = state.values.get_mut(position) {
                        *place = value;
                    }
                }
                self.next_operand(state)
            }
            Joining::Arguments => match self.calls.pop() {
                Some(state) => {
                    for (_, param, value) in values {
                        if let Some(slot) = self.stack.get_mut(state.base + param) {
                            *slot = value;
                        }
                    }
                    self.next_argument(state)
                }
                None => self.lost_call(),
            },
            Joining::Builtin(mut call) => {
                for (piece, param, value) in values {
                    let span = self.argument_span(call.args, piece);
                    if let Some(entry) = call.arguments.get_mut(param) {
                        *entry = Some((value, span));
                    }
                }
                self.builtin_next(call)
            }
        }
    }

    /// The span of the value of the argument `at` of `args`.
    fn argument_span(&self, args: IrRange, at: usize) -> crate::source::Span {
        let ir = &self.running().ir;
        let arg = ir.items(args).get(at).copied();
        let value = arg.and_then(|arg| match ir.get(arg).kind {
            IrKind::Arg {
                value: ArgValue::Node(value),
                ..
            } => Some(value),
            _ => None,
        });
        ir.get(value.or(arg).unwrap_or_else(|| ir.root())).span
    }

    /// Ends `what`, whose pieces came to `unwind`, as its own piece's
    /// failure would end it.
    fn pieces_failed(&mut self, what: &Joining, unwind: Box<Unwind>) -> Next {
        match what {
            Joining::Operands(_) | Joining::Builtin(_) => Next::Give(Err(unwind)),
            Joining::Arguments => match self.calls.pop() {
                Some(state) => self.argument_failed(&state, *unwind),
                None => self.lost_call(),
            },
        }
    }

    /// Whether an iteration or piece split off from one the task is in
    /// has failed, so that what the task does now would not be done in
    /// program order.
    pub(super) fn doomed(&self) -> bool {
        with_calls(&self.frames, &self.calls).any(|(frame, call)| match frame {
            Frame::ForBody(state) => state.failed(),
            frame => frame.pieces(call).is_some_and(Pieces::failed),
        })
    }
}

/// The tasks of `join` that have yet to end.
fn live(join: &Rc<RefCell<Join>>) -> Vec<u32> {
    join.borrow()
        .children
        .iter()
        .filter(|child| child.3.is_none())
        .map(|child| child.2)
        .collect()
}
