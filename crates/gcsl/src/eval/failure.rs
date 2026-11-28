//! Evaluating `assert` and `recover`, and the failure a fallback reads.
//!
//! # Failure
//!
//! An assertion (`Evaluator::assert`) of a comparison evaluates its
//! operands, each once, as values, and compares them as the comparison's
//! own walker would (`Evaluator::compared`), so that a failed one labels
//! each operand with its value, as `show` writes it, without evaluating
//! anything twice; any other condition is evaluated as a `Bool`. A false
//! one is "assertion failed: MESSAGE", at the condition, its message
//! evaluated only then. A `recover` (`Evaluator::recover`) evaluates its
//! body; a failure of it that `#[retry]` would retry, an error with no
//! mark, being neither a bug in GCSL nor the passing of an enclosing call's
//! deadline (see [Retry and timeout](crate::eval::retry#retry-and-timeout)), is caught: its
//! message and its notes, a failing program's output say, are pushed onto
//! a stack in the run's shared state ([`Caught`]), which the fallback's
//! `Failure` reads as the record `{message, notes}`, and popped once the
//! fallback has run. A deadline a call inside the body set is cleared by
//! that call, its own failure, so it is caught; `break`, `continue` and
//! `return` are no failures, and pass through. What the body ran stays run.
//!
//! The host is told of every failure caught, as it is caught
//! (`Evaluator::caught`, [`Effects::caught`](super::Effects::caught)): the
//! failure whole, as it would have been reported, with the input its spans
//! are of, so that a host can report what the program went on from. A
//! fallback's `raise` (`FailureOp::Raise`) fails with the failure its
//! `recover` caught, as it was: its message, where it was raised, its
//! labels and its notes, so that what catches or reports it next sees the
//! failure, not the `raise`. The failure is found by its number among
//! those of the `recover`s running their fallbacks, which the fallback
//! stored in a slot of its frame as it began (`FailureOp::At`), since
//! another fallback may be running by the time the `raise` is reached, in
//! a block the fallback gives a call that recovers too. A failure raised
//! again is marked so, which the host is told when it is caught again.

use std::rc::Rc;

use crate::diagnostic::{Diagnostic, Mark as ErrorMark};
use crate::ir::{FailureOp, IrId, IrKind, IrNode};
use crate::value::{List, Record, Value};

use super::{
    CaughtFailure, Evaluator, Flow, Unwind, comparison_operands, mistyped, record_eq_shaped,
    source_of,
};

impl Evaluator<'_> {
    /// Evaluates the assertion `node`, of the condition `cond` and the
    /// message `message` (see [Failure](#failure)): a comparison's operands
    /// each once, as values, compared as the comparison would compare them
    /// ([`Evaluator::compared`]), and any other condition as a `Bool`; if it
    /// is false, "assertion failed: MESSAGE", at the condition, each operand
    /// of a comparison labelled with its value, as `show` writes it.
    #[inline(never)]
    pub(super) fn assert(&mut self, (cond, message): (IrId, IrId), node: &IrNode) -> Flow<Value> {
        let condition = self.ir.get(cond);
        let (holds, shown) = match comparison_operands(condition.kind) {
            Some((lhs, rhs)) => {
                let (left, right) = (self.value(lhs)?, self.value(rhs)?);
                let holds = self.compared(condition, (&left, &right))?;
                (holds, Some([(lhs, left), (rhs, right)]))
            }
            None => (self.bool(cond)?, None),
        };
        if holds {
            return Ok(Value::Empty);
        }
        let message = self.string(message)?;
        Err(self.assertion_failed((cond, message), shown, node).into())
    }

    /// The failure of the assertion `node` whose condition `cond` did not
    /// hold, with the message `message`, its condition's operands' values
    /// `shown` if it is a comparison, as [`Evaluator::assert`] reports it.
    pub(super) fn assertion_failed(
        &self,
        (cond, message): (IrId, Rc<str>),
        shown: Option<[(IrId, Value); 2]>,
        node: &IrNode,
    ) -> Diagnostic {
        let condition = self.ir.get(cond);
        let mut error = Diagnostic::error(format!("assertion failed: {message}"), condition.span);
        for (operand, value) in shown.into_iter().flatten() {
            let text =
                source_of(&value, node).map_or_else(|_| value.to_string(), |text| text.to_string());
            error = error.with_context(self.ir.get(operand).span, format!("this is `{text}`"));
        }
        error
    }

    /// Whether the comparison `node` holds of `lhs` and `rhs`, its operands'
    /// values, as the walker of its kind would compare them.
    pub(super) fn compared(&self, node: &IrNode, (lhs, rhs): (&Value, &Value)) -> Flow<bool> {
        let amount = |value: &Value| match *value {
            Value::Int(amount) | Value::Quantity(_, amount) => Some(amount),
            _ => None,
        };
        let held = match (node.kind, lhs, rhs) {
            (IrKind::IntCmp(cmp, _), _, _) => {
                amount(lhs).zip(amount(rhs)).map(|(a, b)| cmp.test(&a, &b))
            }
            (IrKind::FloatCmp(cmp, _), Value::Float(a), Value::Float(b)) => Some(cmp.test(a, b)),
            (IrKind::StrCmp(cmp, _), Value::Str(a), Value::Str(b)) => Some(cmp.test(&**a, &**b)),
            (IrKind::BoolEq(equal, _), Value::Bool(a), Value::Bool(b)) => Some((a == b) == equal),
            (IrKind::RecordEq(o) | IrKind::ListEq(o), _, _) => {
                record_eq_shaped(self.ir, o.shape, lhs, rhs)
            }
            (IrKind::RecordNe(o) | IrKind::ListNe(o), _, _) => {
                record_eq_shaped(self.ir, o.shape, lhs, rhs).map(|same| !same)
            }
            (IrKind::RecordSame(_) | IrKind::ListSame(_) | IrKind::ValueSame(_), _, _) => {
                Some(lhs == rhs)
            }
            (IrKind::RecordNotSame(_) | IrKind::ListNotSame(_) | IrKind::ValueNotSame(_), _, _) => {
                Some(lhs != rhs)
            }
            _ => None,
        };
        held.map_or_else(
            || mistyped(node, "a comparison of values of another kind"),
            Ok,
        )
    }

    /// Evaluates `body`, the body of a `recover` (see [Failure](#failure)),
    /// and, if it fails as `#[retry]` would retry, with no mark, `fallback`
    /// instead, with its failure the innermost a `Failure` reads.
    #[inline(never)]
    pub(super) fn recover(&mut self, body: IrId, fallback: IrId) -> Flow<Value> {
        match self.value(body) {
            Err(Unwind::Error(error)) if error.mark == ErrorMark::NONE => {
                let caught = self.caught(error);
                self.shared.failures.borrow_mut().push(caught);
                let result = self.value(fallback);
                self.shared.failures.borrow_mut().pop();
                result
            }
            result => result,
        }
    }

    /// The failure `error`, which a `recover` of this frame catches, as
    /// its fallback holds it, the host told of it
    /// ([`Effects::caught`](super::Effects::caught)): located in this
    /// frame's input if it names none, since a failure that reaches a frame
    /// with none was raised in that frame's code.
    pub(super) fn caught(&self, mut error: Diagnostic) -> Rc<Caught> {
        error.input.get_or_insert(self.ir.input());
        self.shared.effects.caught(&CaughtFailure {
            failure: &error,
            again: error.raised_again,
        });
        Caught::of(error)
    }

    /// What the `Failure` `node` does, as `op` says (see
    /// [Failure](#failure)): the failure the innermost `recover` running
    /// its fallback caught, as a record of its message and its notes; which
    /// of the failures caught that one is; or the failure of that number,
    /// raised again.
    pub(super) fn failure(&mut self, op: FailureOp, node: &IrNode) -> Flow<Value> {
        let caught = match op {
            FailureOp::Read(_) | FailureOp::At => self.shared.failures.borrow().last().cloned(),
            FailureOp::Raise(at) => {
                let at = self.int(at)?;
                let failures = self.shared.failures.borrow();
                usize::try_from(at)
                    .ok()
                    .and_then(|at| failures.get(at))
                    .cloned()
            }
        };
        let Some(caught) = caught else {
            return mistyped(node, "a failure read outside a `recover`'s fallback");
        };
        match op {
            FailureOp::Read(layout) => Ok(Value::Record(Rc::new(Record {
                layout: Rc::clone(self.ir.layout(layout)),
                values: Box::new([
                    Value::Str(Rc::clone(&caught.message)),
                    Value::List(caught.notes.clone()),
                ]),
            }))),
            FailureOp::At => {
                let at = self.shared.failures.borrow().len().saturating_sub(1);
                Ok(Value::Int(i64::try_from(at).unwrap_or(i64::MAX)))
            }
            FailureOp::Raise(_) => Err(self.raised(&caught).into()),
        }
    }

    /// The failure `caught`, raised again from this frame: as it was, but
    /// marked so, and naming no input if its input is this frame's, as a
    /// failure raised here does.
    fn raised(&self, caught: &Caught) -> Diagnostic {
        let mut error = caught.error.clone();
        error.raised_again = true;
        if error.input == Some(self.ir.input()) {
            error.input = None;
        }
        error
    }
}

/// A failure a `recover` caught, as its fallback reads it (see
/// [Failure](#failure)): its message, and its notes, each a `String`; and
/// the failure whole, which `raise` raises again.
#[derive(Debug)]
pub(super) struct Caught {
    message: Rc<str>,
    notes: List,
    error: Diagnostic,
}

impl Caught {
    /// The failure `error` caught.
    pub(super) fn of(error: Diagnostic) -> Rc<Self> {
        let notes = error
            .notes
            .iter()
            .map(|note| Value::Str(Rc::from(note.as_str())))
            .collect();
        Rc::new(Self {
            message: Rc::from(error.message.as_str()),
            notes: List::from_vec(notes),
            error,
        })
    }
}
