//! Data and operators in the machine: a node whose operands are not all
//! simple has them evaluated here, in order, and is then applied to their
//! values as the walker of its kind would apply it ([`apply`]). `and` and
//! `or`, which evaluate their right operand only if the left one does not
//! decide, have a frame of their own.

use std::fmt::Write as _;
use std::rc::Rc;

use crate::ir::{IrId, IrKind, Operands};
use crate::value::{List, Record, Value, int_to_float};

use super::super::{Evaluator, Flow, INTERPOLATED_VALUE_ESTIMATE, internal, mistyped, out_of_range};
use super::{Frame, Machine, Next, Outcome, fail, give};

/// A node whose operands the machine evaluates, in order, before applying
/// it.
pub(super) struct OperandsState {
    /// The node.
    node: IrId,
    /// Its operands, in order.
    operands: Vec<IrId>,
    /// Their values so far.
    values: Vec<Value>,
}

impl Machine<'_> {
    /// Whether the machine applies node `kind` itself, its operands
    /// evaluated here ([`apply`]).
    pub(super) const fn applies(kind: IrKind) -> bool {
        matches!(
            kind,
            IrKind::MakeRecord { .. }
                | IrKind::MakeList { spreads: false, .. }
                | IrKind::Field { .. }
                | IrKind::Index { .. }
                | IrKind::ListLen(_)
                | IrKind::Interp(_)
                | IrKind::StrConcat(_)
                | IrKind::IntCmp(..)
                | IrKind::FloatCmp(..)
                | IrKind::StrCmp(..)
                | IrKind::BoolEq(..)
                | IrKind::RecordEq(_)
                | IrKind::RecordNe(_)
                | IrKind::RecordSame(_)
                | IrKind::RecordNotSame(_)
                | IrKind::ListEq(_)
                | IrKind::ListNe(_)
                | IrKind::ListSame(_)
                | IrKind::ListNotSame(_)
                | IrKind::ValueSame(_)
                | IrKind::ValueNotSame(_)
                | IrKind::Not(_)
                | IrKind::FloatAdd(_)
                | IrKind::FloatSub(_)
                | IrKind::FloatMul(_)
                | IrKind::FloatDiv(_)
                | IrKind::FloatNeg(_)
                | IrKind::IntToFloat(_)
        )
    }

    /// Evaluates node `id`, one the machine applies itself: its operands,
    /// in order, the simple ones at once, and then the node.
    pub(super) fn operands(&mut self, id: IrId) -> Next {
        let operands = self.running().ir.children(id).collect();
        self.next_operand(Box::new(OperandsState {
            node: id,
            operands,
            values: Vec::new(),
        }))
    }

    /// Evaluates the operands of `state` from the next on, and then applies
    /// its node.
    fn next_operand(&mut self, mut state: Box<OperandsState>) -> Next {
        loop {
            let Some(&operand) = state.operands.get(state.values.len()) else {
                let OperandsState { node, values, .. } = *state;
                return give(self.walker(|walker| apply(walker, node, values)));
            };
            if !self.running().ir.is_simple(operand) {
                self.frames.push(Frame::Operands(state));
                return Next::Eval(operand);
            }
            match self.walker(|walker| walker.value(operand)) {
                Ok(value) => state.values.push(value),
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up `state` with `result`, the value of its next operand.
    pub(super) fn operand(&mut self, mut state: Box<OperandsState>, result: Outcome) -> Next {
        match result {
            Ok(value) => {
                state.values.push(value);
                self.next_operand(state)
            }
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Evaluates the `and` or `or` node of `operands`: its left operand,
    /// and then, unless that is `decisive`, its right one, whose value is
    /// the node's.
    pub(super) fn logic(&mut self, operands: Operands, decisive: bool) -> Next {
        if self.running().ir.is_simple(operands.lhs) {
            return match self.walker(|walker| walker.bool(operands.lhs)) {
                Ok(lhs) if lhs == decisive => Next::Give(Ok(Value::Bool(lhs))),
                Ok(_) => Next::Eval(operands.rhs),
                Err(unwind) => fail(unwind),
            };
        }
        self.frames.push(Frame::Logic(operands, decisive));
        Next::Eval(operands.lhs)
    }

    /// Takes up the `and` or `or` of `operands` with `result`, the value of
    /// its left operand.
    pub(super) fn logic_lhs(
        &mut self,
        operands: Operands,
        decisive: bool,
        result: Outcome,
    ) -> Next {
        match result {
            Ok(Value::Bool(lhs)) if lhs == decisive => Next::Give(Ok(Value::Bool(lhs))),
            Ok(Value::Bool(_)) => Next::Eval(operands.rhs),
            Ok(_) => self.mistyped(
                operands.lhs,
                "an operand of `and` or `or` that is not a Bool",
            ),
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }
}

/// Applies node `id` of `walker`'s code to `values`, its operands' values,
/// in order, as the walker of its kind would.
fn apply(walker: &mut Evaluator<'_>, id: IrId, values: Vec<Value>) -> Flow<Value> {
    let node = walker.ir.get(id);
    match (node.kind, &values[..]) {
        (IrKind::MakeRecord { layout, .. }, _) => Ok(Value::Record(Rc::new(Record {
            layout: Rc::clone(walker.ir.layout(layout)),
            values: values.into_boxed_slice(),
        }))),
        (IrKind::MakeList { .. }, _) => Ok(Value::List(List::from_vec(values))),
        (IrKind::Field { name, .. }, [record]) => match record {
            Value::Record(fields) => match fields.get(name) {
                Some(value) => Ok(value.clone()),
                None => mistyped(node, "a record lacks a field the checker found"),
            },
            _ => mistyped(node, "a field of the empty record"),
        },
        (IrKind::Index { .. }, [Value::List(list), Value::Int(at)]) => {
            match usize::try_from(*at)
                .ok()
                .and_then(|position| list.get(position))
            {
                Some(value) => Ok(value.clone()),
                None => Err(out_of_range(node.span, *at, list.len()).into()),
            }
        }
        (IrKind::ListLen(_), [Value::List(list)]) => {
            Ok(Value::Int(i64::try_from(list.len()).unwrap_or(i64::MAX)))
        }
        (IrKind::Interp(items), _) => {
            interpolated(walker, walker.ir.items(items), &values, node.span)
        }
        (IrKind::StrConcat(_), [Value::Str(lhs), Value::Str(rhs)]) => {
            let mut text = String::with_capacity(lhs.len() + rhs.len());
            text.push_str(lhs);
            text.push_str(rhs);
            Ok(Value::Str(Rc::from(text)))
        }
        (IrKind::Not(_), [Value::Bool(operand)]) => Ok(Value::Bool(!operand)),
        (IrKind::FloatAdd(_), [Value::Float(lhs), Value::Float(rhs)]) => {
            Ok(Value::Float(lhs + rhs))
        }
        (IrKind::FloatSub(_), [Value::Float(lhs), Value::Float(rhs)]) => {
            Ok(Value::Float(lhs - rhs))
        }
        (IrKind::FloatMul(_), [Value::Float(lhs), Value::Float(rhs)]) => {
            Ok(Value::Float(lhs * rhs))
        }
        (IrKind::FloatDiv(_), [Value::Float(lhs), Value::Float(rhs)]) => {
            Ok(Value::Float(lhs / rhs))
        }
        (IrKind::FloatNeg(_), [Value::Float(operand)]) => Ok(Value::Float(-operand)),
        (IrKind::IntToFloat(_), [Value::Int(operand)]) => Ok(Value::Float(int_to_float(*operand))),
        (
            IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::ListEq(_)
            | IrKind::ListNe(_)
            | IrKind::ListSame(_)
            | IrKind::ListNotSame(_)
            | IrKind::ValueSame(_)
            | IrKind::ValueNotSame(_),
            [lhs, rhs],
        ) => walker.compared(node, (lhs, rhs)).map(Value::Bool),
        _ => mistyped(node, "operands of another kind than their node's"),
    }
}

/// The text of an interpolation spanning `span` of `items`, whose values
/// are `values`: a `StrConst` is literal text, and any other item a value
/// shown with its `Display`, as
/// [`Evaluator::interpolate`](super::Evaluator::interpolate) builds it.
fn interpolated(
    walker: &Evaluator<'_>,
    items: &[IrId],
    values: &[Value],
    span: crate::source::Span,
) -> Flow<Value> {
    let estimate = items.iter().fold(0_usize, |total, &item| {
        let len = match walker.ir.get(item).kind {
            IrKind::StrConst(constant) => walker.ir.string(constant).len(),
            _ => INTERPOLATED_VALUE_ESTIMATE,
        };
        total.saturating_add(len)
    });
    let mut text = String::with_capacity(estimate);
    for (&item, value) in items.iter().zip(values) {
        if let IrKind::StrConst(constant) = walker.ir.get(item).kind {
            text.push_str(walker.ir.string(constant));
        } else {
            write!(text, "{value}")
                .map_err(|_| internal("showing a value as text failed", span))?;
        }
    }
    Ok(Value::Str(Rc::from(text)))
}
