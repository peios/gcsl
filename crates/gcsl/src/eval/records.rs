//! Evaluating records: record literals, field reads and comparisons of records.

use std::rc::Rc;

use crate::ir::{IrId, IrKind, IrNode, IrRange, LayoutId, Operands, ShapedOperands};
use crate::names::Name;
use crate::value::{Record, Value, ValueKind};

use super::{CONTEXT_BIND, Evaluator, Flow, MIXED, mistyped, record_eq_shaped};
use super::kinds::{RECORD, into_record, mixed};

impl Evaluator<'_> {
    /// Builds a record of the values of `values`, in the order of `layout`'s
    /// names: the one allocation for its values, sized exactly, and one for
    /// the record, which shares the layout.
    pub(super) fn make_record(&mut self, layout: LayoutId, values: IrRange) -> Flow<Value> {
        let items = self.ir.items(values);
        let mut fields = Vec::with_capacity(items.len());
        for &item in items {
            fields.push(self.value(item)?);
        }
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::clone(self.ir.layout(layout)),
            values: fields.into_boxed_slice(),
        })))
    }

    /// Evaluates `record` and returns its field `name`, for the `Field`
    /// `node`. The checker guarantees the field is there, so a missing
    /// one, or an empty record, is an internal error.
    pub(super) fn field(&mut self, record: IrId, name: Name, node: &IrNode) -> Flow<Value> {
        match self.record(record)? {
            Value::Record(fields) => match fields.get(name) {
                Some(value) => Ok(value.clone()),
                None => mistyped(node, "a record lacks a field the checker found"),
            },
            _ => mistyped(node, "a field of the empty record"),
        }
    }

    /// Evaluates the `Field` `node`, reading field `name` of `record`, as a
    /// value that `extract` accepts: a field of the kind the node states.
    pub(super) fn field_as<T>(
        &mut self,
        (record, name): (IrId, Name),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.field(record, name, node)?) {
            Some(value) => Ok(value),
            None => mistyped(node, "a field holds a value of another kind than its node"),
        }
    }

    /// Evaluates the typed record comparison `node`, a `RecordEq` if `equal`
    /// and a `RecordNe` otherwise: the operands compare over their shape
    /// (see [`record_eq_shaped`]).
    pub(super) fn record_eq(
        &mut self,
        operands: ShapedOperands,
        equal: bool,
        node: &IrNode,
    ) -> Flow<bool> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        match record_eq_shaped(self.ir, operands.shape, &lhs, &rhs) {
            Some(same) => Ok(same == equal),
            None => mistyped(node, "a record lacks a field of its shape"),
        }
    }

    /// Evaluates a total record comparison, a `RecordSame` if `equal` and a
    /// `RecordNotSame` otherwise: records compare whole, deeply and by field
    /// name; empty records are all equal.
    pub(super) fn record_same(&mut self, operands: Operands, equal: bool) -> Flow<bool> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates a record node: [`Value::Empty`] or a [`Value::Record`].
    pub(super) fn record(&mut self, id: IrId) -> Flow<Value> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty | IrKind::Helper => Ok(self.empty(node)),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &RECORD),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a record was expected"),
            IrKind::Store(slot, value) => self.store(slot, value, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.record(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.record(branch)
            }
            IrKind::MakeRecord { layout, values } => self.make_record(layout, values),
            IrKind::UpdateField { .. } => self.update_field(node),
            IrKind::Update {
                kind: ValueKind::Record,
                ..
            } => self.update(node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a record was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a record was expected"),
            IrKind::BoolConst(_)
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
            | IrKind::Is { .. }
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a record was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) | IrKind::Caller => {
                mistyped(node, "String node where a record was expected")
            }
            IrKind::Show(_) => mistyped(node, "`show` node where a record was expected"),
            IrKind::For { .. } | IrKind::Loop { .. } => self.record_loop(node),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. } => self.stopping(node, into_record),
            IrKind::MakeList { .. }
            | IrKind::Spread(_)
            | IrKind::Range(..)
            | IrKind::Update { .. } => mistyped(node, "list node where a record was expected"),
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a record was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }
}
