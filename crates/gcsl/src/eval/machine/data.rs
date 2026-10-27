//! Data and operators in the machine: a node whose operands are not all
//! simple has them evaluated here, in order, and is then applied to their
//! values as the walker of its kind would apply it ([`apply`]). `and` and
//! `or`, which evaluate their right operand only if the left one does not
//! decide, have a frame of their own.

use std::fmt::Write as _;
use std::rc::Rc;

use crate::check::Builtin;
use crate::ir::{IrId, IrKind, Operands};
use crate::value::{List, Record, Value, int_to_float};

use super::super::kinds::{into_callee, of_kind};
use super::super::{
    Callee, Evaluator, Flow, INTERPOLATED_VALUE_ESTIMATE, internal, mistyped, out_of_range,
    source_of,
};
use super::futures::{AwaitState, Then};
use super::pieces::{Joining, Pieces, any_later, splits};
use super::{Frame, Machine, Next, Outcome, fail, give};

/// A node whose operands the machine evaluates, in order, before applying
/// it.
#[derive(Clone)]
pub(in crate::eval) struct OperandsState {
    /// The node.
    node: IrId,
    /// Its operands, in order.
    operands: Vec<IrId>,
    /// Their values so far, each split-off operand's place held by `{}`
    /// until it ends.
    pub(super) values: Vec<Value>,
    /// Its operands split off, if it has pieces that may be split off (see
    /// [Pieces](super::pieces#pieces)).
    pub(super) pieces: Pieces,
}

impl Machine<'_> {
    /// Whether the machine applies node `kind` itself, its operands
    /// evaluated here ([`apply`]).
    pub(super) const fn applies(kind: IrKind) -> bool {
        matches!(
            kind,
            IrKind::MakeRecord { .. }
                | IrKind::MakeList { .. }
                | IrKind::UpdateField { .. }
                | IrKind::Update { .. }
                | IrKind::JsonDecode { .. }
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
                | IrKind::Is { .. }
                | IrKind::As { .. }
                | IrKind::RecordOf { .. }
                | IrKind::IntNeg(_)
                | IrKind::Quantity(..)
                | IrKind::Show(_)
                | IrKind::Range(..)
                | IrKind::FilePath(_)
                | IrKind::Meta(_)
        )
    }

    /// Evaluates node `id`, one the machine applies itself: its operands,
    /// in order, the simple ones at once, and then the node.
    pub(super) fn operands(&mut self, id: IrId) -> Next {
        let ir = &self.running().ir;
        let operands = match ir.get(id).kind {
            // Its value and the record or list it updates: the positions
            // of its path are evaluated as its walk reaches them.
            IrKind::Update { parts, .. } => ir.items(parts).iter().copied().take(2).collect(),
            // A spread's list stands for it.
            IrKind::MakeList { items, .. } => ir
                .items(items)
                .iter()
                .map(|&item| match ir.get(item).kind {
                    IrKind::Spread(list) => list,
                    _ => item,
                })
                .collect(),
            _ => ir.children(id).collect(),
        };
        self.next_operand(Box::new(OperandsState {
            node: id,
            operands,
            values: Vec::new(),
            pieces: Pieces::default(),
        }))
    }

    /// Evaluates the operands of `state` from the next on, and then, those
    /// split off ended, applies its node.
    pub(super) fn next_operand(&mut self, mut state: Box<OperandsState>) -> Next {
        loop {
            let Some(&operand) = state.operands.get(state.values.len()) else {
                if let Some(join) = state.pieces.join.take() {
                    return self.join_pieces(Joining::Operands(state), join, None);
                }
                return self.apply_again(state);
            };
            let ir = &self.running().ir;
            if !ir.is_simple(operand) {
                let later = state.operands.get(state.values.len() + 1..);
                let split = splits(ir.get(state.node).kind)
                    && any_later(ir, later.unwrap_or_default())
                    && self.splittable(operand);
                let at = state.values.len();
                self.frames.push(Frame::Operands(state));
                if split {
                    self.piece_begin(u32::try_from(at).unwrap_or(u32::MAX));
                }
                return Next::Eval(operand);
            }
            match self.walker(|walker| walker.value(operand)) {
                Ok(value) => state.values.push(value),
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up `state` with `result`, the value of its next operand: a
    /// quantity's overflow in its amount's own operator says what
    /// overflowed, as
    /// [`Evaluator::quantity_amount`](super::Evaluator::quantity_amount)
    /// says it.
    pub(super) fn operand(&mut self, mut state: Box<OperandsState>, result: Outcome) -> Next {
        self.piece_end(&mut state.pieces);
        match result {
            Ok(value) => {
                state.values.push(value);
                self.next_operand(state)
            }
            // A failure waits behind the operands split off before it.
            Err(unwind) if state.pieces.join.is_some() => {
                let at = state.values.len();
                match state.pieces.join.take() {
                    Some(join) => {
                        self.join_pieces(Joining::Operands(state), join, Some((at, unwind)))
                    }
                    None => Next::Give(Err(unwind)),
                }
            }
            Err(unwind) => match self.running().ir.get(state.node).kind {
                IrKind::Quantity(quantity, amount) => {
                    fail(self.walker(|walker| walker.quantity_overflow(quantity, amount, *unwind)))
                }
                _ => Next::Give(Err(unwind)),
            },
        }
    }

    /// Applies the node of `state` to its operands' values. An update's walk
    /// down its path leaves the first future it meets that has not run to
    /// the machine, as a deep await does, which runs it, its body here, and
    /// then applies the update again.
    pub(super) fn apply_again(&mut self, state: Box<OperandsState>) -> Next {
        let node = state.node;
        if !matches!(self.running().ir.get(node).kind, IrKind::Update { .. }) {
            let OperandsState { values, .. } = *state;
            return give(self.walker(|walker| apply(walker, node, values)));
        }
        let values = state.values.clone();
        self.shared.ahead.awaiting.set(true);
        let applied = self.walker(|walker| apply(walker, node, values));
        self.shared.ahead.awaiting.set(false);
        match (applied, self.shared.ahead.met.take()) {
            (
                Err(_),
                Some(super::super::Met {
                    future,
                    depth,
                    level,
                    span,
                }),
            ) => {
                let site = AwaitState::at(node, (depth, span));
                match self.begin_future(site, future, (level, Then::Apply(state.clone()))) {
                    // It had run after all: the walk goes on past it.
                    Ok(_) => self.apply_again(state),
                    Err(next) => next,
                }
            }
            (applied, _) => give(applied),
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
        (IrKind::MakeList { spreads: false, .. }, _) => Ok(Value::List(List::from_vec(values))),
        (IrKind::MakeList { items, .. }, _) => {
            spread_list(walker, walker.ir.items(items), values, node)
        }
        (IrKind::UpdateField { name, .. }, [record, _]) => {
            let record = record.clone();
            let value = values.into_iter().nth(1).unwrap_or(Value::Empty);
            walker.with_field(record, name, value, node)
        }
        (
            IrKind::Update {
                parts, path, depth, ..
            },
            [_, _],
        ) => {
            let mut values = values.into_iter();
            let (value, root) = (
                values.next().unwrap_or(Value::Empty),
                values.next().unwrap_or(Value::Empty),
            );
            let positions = walker.ir.items(parts).get(2..).unwrap_or_default();
            walker.walk_update((root, value), (positions, path, depth), node)
        }
        (IrKind::JsonDecode { decode, kind, .. }, [Value::Str(text)]) => {
            decoded(walker, text, (decode, kind), node)
        }
        (IrKind::Is { .. } | IrKind::As { .. } | IrKind::RecordOf { .. } | IrKind::Meta(_), _) => {
            narrowed(walker, node, values)
        }
        (IrKind::IntNeg(operand), [Value::Int(value)]) => {
            walker.int_neg_of(node, operand, *value).map(Value::Int)
        }
        (IrKind::Quantity(quantity, _), [Value::Int(amount)]) => {
            Ok(Value::Quantity(quantity, *amount))
        }
        (IrKind::Show(_), [value]) => {
            let value = value.clone();
            walker
                .profiled_builtin(Builtin::Show, |_| source_of(&value, node))
                .map(Value::Str)
        }
        (IrKind::Range(inclusive, _), [Value::Int(start), Value::Int(end)]) => {
            Evaluator::range_of((*start, *end), inclusive, node)
        }
        (IrKind::FilePath(_), [Value::File(file) | Value::Dir(file)]) => {
            Ok(Value::Str(Rc::clone(&file.path)))
        }
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
        _ => operator(walker, node, &values),
    }
}

/// Applies the operator `node` of `walker`'s code to `values`, its
/// operands' values, in order, as the walker of its kind would: what
/// [`apply`] leaves.
fn operator(walker: &Evaluator<'_>, node: &crate::ir::IrNode, values: &[Value]) -> Flow<Value> {
    match (node.kind, values) {
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
            interpolated(walker, walker.ir.items(items), values, node.span)
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
        _ => mistyped(node, "operands of another kind than their node's"),
    }
}

/// What the `Is`, `As`, `RecordOf` or `Meta` `node` comes to of `values`,
/// its one operand's, as the walkers' helpers for them make it.
fn narrowed(
    walker: &mut Evaluator<'_>,
    node: &crate::ir::IrNode,
    values: Vec<Value>,
) -> Flow<Value> {
    let Some(value) = values.into_iter().next() else {
        return mistyped(node, "a node without its operand");
    };
    match node.kind {
        IrKind::Is { .. } => walker.is_test_of(node, value).map(Value::Bool),
        IrKind::As { kind, .. } => {
            let value = walker.narrowed_of(node, value)?;
            if of_kind(&value, kind) {
                Ok(value)
            } else {
                mistyped(
                    node,
                    "an `as` produced a value of another kind than its node",
                )
            }
        }
        IrKind::RecordOf {
            key, value: named, ..
        } => match value {
            Value::List(entries) => walker.record_of_entries(node, &entries, (key, named)),
            _ => mistyped(node, "a `record` of entries that are no list"),
        },
        IrKind::Meta(_) => match into_callee(value) {
            Some(callee) => match walker.meta_of(callee, node)? {
                Callee::Closure(closure) => Ok(Value::Fn(closure)),
                Callee::Builtin(builtin) => Ok(Value::Builtin(builtin)),
                Callee::Tag(tag) => Ok(Value::Tag(tag)),
            },
            None => mistyped(node, "the meta of a value that is no function"),
        },
        _ => mistyped(node, "a node that is not narrowed narrowed"),
    }
}

/// The decode of `text` by the plan `decode` for the `JsonDecode` `node`,
/// of the kind `kind`, as
/// [`Evaluator::decode_node`](super::Evaluator::decode_node) decodes it.
fn decoded(
    walker: &mut Evaluator<'_>,
    text: &str,
    (decode, kind): (crate::ir::DecodeId, crate::value::ValueKind),
    node: &crate::ir::IrNode,
) -> Flow<Value> {
    let value = walker.profiled_builtin(Builtin::JsonDecode, |this| {
        this.decode_text(text, decode, node)
    })?;
    if of_kind(&value, kind) {
        Ok(value)
    } else {
        mistyped(
            node,
            "a decode produced a value of another kind than its node",
        )
    }
}

/// The list of `items`, some of them spreads, whose values are `values`, a
/// spread's that of its list, as
/// [`Evaluator::make_spread_list`](super::Evaluator::make_spread_list)
/// builds it: each run of elements a leaf, and each spread list joined in
/// as it is.
fn spread_list(
    walker: &Evaluator<'_>,
    items: &[IrId],
    values: Vec<Value>,
    node: &crate::ir::IrNode,
) -> Flow<Value> {
    let mut list = List::EMPTY;
    let mut run = Vec::new();
    for (&item, value) in items.iter().zip(values) {
        if let IrKind::Spread(_) = walker.ir.get(item).kind {
            let Value::List(spread) = value else {
                return mistyped(node, "a spread of a value that is not a list");
            };
            if !run.is_empty() {
                list = list.concat(&List::from_vec(std::mem::take(&mut run)));
            }
            list = list.concat(&spread);
        } else {
            run.push(value);
        }
    }
    if !run.is_empty() {
        list = list.concat(&List::from_vec(run));
    }
    Ok(Value::List(list))
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
