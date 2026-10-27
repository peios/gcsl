//! The walkers of scalars and strings: integers, quantities, floats, booleans
//! and strings, their operators, interpolation and `show`.

use std::fmt::Write;
use std::num::NonZeroI64;
use std::rc::Rc;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::{Cmp, IrId, IrKind, IrNode, IrRange, Operands};
use crate::source::Span;
use crate::value::{Quantity, Value, ValueKind, int_to_float};

use super::{
    CONTEXT_BIND, DIVISION_BY_ZERO, Evaluator, Flow, INTERPOLATED_VALUE_ESTIMATE, OVERFLOW, Unwind,
    floor_div, floor_mod, internal, mistyped, source_of,
};
use super::kinds::{BOOL, FLOAT, STRING, amount_taken, into_int};

impl Evaluator<'_> {
    /// Evaluates an `Int`-typed node.
    pub(super) fn int(&mut self, id: IrId) -> Flow<i64> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::IntConst(value) => Ok(value),
            IrKind::Quantity(quantity, amount) => self.quantity_amount(quantity, amount),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::JsonDecode { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..)
            | IrKind::MakeFuture { .. } => self.shared_amount(id, node),
            IrKind::ListLen(list) => self.list_len(list),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.int(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.int(branch)
            }
            IrKind::IntNeg(operand) => self.int_neg(node, operand),
            IrKind::IntAdd(operands) => self.int_arithmetic(node, operands, i64::checked_add),
            IrKind::IntSub(operands) => self.int_arithmetic(node, operands, i64::checked_sub),
            IrKind::IntMul(operands) => self.int_arithmetic(node, operands, i64::checked_mul),
            IrKind::IntFloorDiv(operands) => self.int_division(node, operands, floor_div),
            IrKind::IntRem(operands) => {
                self.int_division(node, operands, |lhs, rhs| Some(floor_mod(lhs, rhs)))
            }
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where an Int was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where an Int was expected"),
            IrKind::StrConst(_)
            | IrKind::StrConcat(_)
            | IrKind::Interp(_)
            | IrKind::Caller
            | IrKind::FilePath(_) => mistyped(node, "String node where an Int was expected"),
            IrKind::Show(_) => mistyped(node, "`show` node where an Int was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::RecordOf { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where an Int was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where an Int was expected"),
            // A loop that discards its values is `{}`, but only ever a
            // statement, or a loop that cannot end, which `stopping` runs.
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, into_int),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where an Int was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where an Int was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// The value of the quantity node of `quantity` whose `Int` node is
    /// `amount` ([`Evaluator::quantity_amount`]). Out of line, as the walkers'
    /// helpers are, so that the general walker's frame holds none of its
    /// temporaries.
    #[inline(never)]
    pub(super) fn quantity_value(&mut self, quantity: Quantity, amount: IrId) -> Flow<Value> {
        let amount = self.quantity_amount(quantity, amount)?;
        Ok(Value::Quantity(quantity, amount))
    }

    /// Evaluates `node`, the node `id` of a kind any walker shares, for the
    /// `Int` walker: an `Int`, or the amount of a duration or a size, by the
    /// node's kind. Out of line, so that the walker's frame holds none of its
    /// temporaries.
    #[inline(never)]
    pub(super) fn shared_amount(&mut self, id: IrId, node: &IrNode) -> Flow<i64> {
        self.shared_as(node, amount_taken(self.ir.kind_of(id)))
    }

    /// The amount of the quantity node of `quantity` whose `Int` node is
    /// `amount`, of its smallest unit: an overflow in `amount`'s own
    /// operator is "duration overflow" or "size overflow", saying what
    /// overflowed, rather than the `Int` operator's "integer overflow".
    pub(super) fn quantity_amount(&mut self, quantity: Quantity, amount: IrId) -> Flow<i64> {
        let result = self.int(amount);
        result.map_err(|unwind| self.quantity_overflow(quantity, amount, unwind))
    }

    /// `unwind`, out of the evaluation of `amount`, the `Int` node of a
    /// quantity node of `quantity`: an overflow in its own operator says
    /// what overflowed (see [`Evaluator::quantity_amount`]).
    pub(super) fn quantity_overflow(
        &self,
        quantity: Quantity,
        amount: IrId,
        unwind: Unwind,
    ) -> Unwind {
        let inner = self.ir.get(amount);
        let at = inner.operator_span().unwrap_or(inner.span);
        match unwind {
            Unwind::Error(mut error) if error.message == OVERFLOW && error.span == at => {
                error.message = format!("{} overflow", quantity.name());
                Unwind::Error(error)
            }
            unwind => unwind,
        }
    }

    /// The text an operator's error labels its operand `operand` with, whose
    /// value is `value`: as the quantity it is, `30s`, or as the `Int`.
    pub(super) fn operand_text(&self, operand: IrId, value: i64) -> String {
        match self.ir.kind_of(operand) {
            ValueKind::Quantity(quantity) => Value::Quantity(quantity, value).to_string(),
            _ => value.to_string(),
        }
    }

    /// Evaluates both `Int` operands, left to right.
    pub(super) fn ints(&mut self, operands: Operands) -> Flow<(i64, i64)> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        Ok((lhs, rhs))
    }

    /// Evaluates the `IntNeg` `node` of `operand`.
    pub(super) fn int_neg(&mut self, node: &IrNode, operand: IrId) -> Flow<i64> {
        let value = self.int(operand)?;
        self.int_neg_of(node, operand, value)
    }

    /// The `IntNeg` `node` of `operand`, whose value is `value`, evaluated
    /// already.
    pub(super) fn int_neg_of(&self, node: &IrNode, operand: IrId, value: i64) -> Flow<i64> {
        value
            .checked_neg()
            .ok_or_else(|| self.unary_error(node, operand, OVERFLOW, value).into())
    }

    /// Evaluates the `+`, `-` or `*` `node`, which `op` computes, `None`
    /// meaning overflow.
    pub(super) fn int_arithmetic(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, i64) -> Option<i64>,
    ) -> Flow<i64> {
        // Not through `ints`: this is on the recursion path of every
        // arithmetic chain, so it saves that frame.
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        self.int_arithmetic_of(node, operands, op, (lhs, rhs))
    }

    /// The `+`, `-` or `*` `node`, which `op` computes, of its operands'
    /// values `lhs` and `rhs`, evaluated already.
    pub(super) fn int_arithmetic_of(
        &self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, i64) -> Option<i64>,
        (lhs, rhs): (i64, i64),
    ) -> Flow<i64> {
        op(lhs, rhs).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs).into())
    }

    /// Evaluates the `//` or `%` `node`, which `op` computes for a nonzero
    /// divisor, `None` meaning overflow.
    pub(super) fn int_division(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, NonZeroI64) -> Option<i64>,
    ) -> Flow<i64> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        self.int_division_of(node, operands, op, (lhs, rhs))
    }

    /// The `//` or `%` `node`, which `op` computes, of its operands' values
    /// `lhs` and `rhs`, evaluated already.
    pub(super) fn int_division_of(
        &self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, NonZeroI64) -> Option<i64>,
        (lhs, rhs): (i64, i64),
    ) -> Flow<i64> {
        let Some(divisor) = NonZeroI64::new(rhs) else {
            return Err(self
                .binary_error(node, operands, DIVISION_BY_ZERO, lhs, rhs)
                .into());
        };
        op(lhs, divisor).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs).into())
    }

    /// Evaluates a `Float`-typed node. IEEE 754 semantics; never fails except
    /// through an `Int` operand of `IntToFloat`, an item of a block or the
    /// condition of an `if`.
    pub(super) fn float(&mut self, id: IrId) -> Flow<f64> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::FloatConst(value) => Ok(value),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::JsonDecode { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &FLOAT),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a Float was expected"),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.float(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.float(branch)
            }
            IrKind::FloatNeg(operand) => self.float_neg(operand),
            IrKind::FloatAdd(o) => self.float_arithmetic(o, |lhs, rhs| lhs + rhs),
            IrKind::FloatSub(o) => self.float_arithmetic(o, |lhs, rhs| lhs - rhs),
            IrKind::FloatMul(o) => self.float_arithmetic(o, |lhs, rhs| lhs * rhs),
            IrKind::FloatDiv(o) => self.float_arithmetic(o, |lhs, rhs| lhs / rhs),
            IrKind::IntToFloat(operand) => self.int(operand).map(int_to_float),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a Float was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where a Float was expected"),
            IrKind::StrConst(_)
            | IrKind::StrConcat(_)
            | IrKind::Interp(_)
            | IrKind::Caller
            | IrKind::FilePath(_) => mistyped(node, "String node where a Float was expected"),
            IrKind::Show(_) => mistyped(node, "`show` node where a Float was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::RecordOf { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where a Float was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where a Float was expected"),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, Value::into_float),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where a Float was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a Float was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates a `FloatNeg` of `operand`.
    pub(super) fn float_neg(&mut self, operand: IrId) -> Flow<f64> {
        Ok(-self.float(operand)?)
    }

    /// Evaluates a `Float` operator that `op` computes.
    pub(super) fn float_arithmetic(
        &mut self,
        operands: Operands,
        op: fn(f64, f64) -> f64,
    ) -> Flow<f64> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(op(lhs, rhs))
    }

    /// Evaluates a `Bool`-typed node. `and` and `or` evaluate their right
    /// operand only when the left does not decide the result.
    pub(super) fn bool(&mut self, id: IrId) -> Flow<bool> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::BoolConst(value) => Ok(value),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::JsonDecode { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &BOOL),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a Bool was expected"),
            IrKind::Is { .. } => self.is_test(node),
            IrKind::ValueSame(o) => self.value_same(o, true),
            IrKind::ValueNotSame(o) => self.value_same(o, false),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.bool(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.bool(branch)
            }
            IrKind::IntCmp(cmp, o) => self.int_cmp(cmp, o),
            IrKind::FloatCmp(cmp, o) => self.float_cmp(cmp, o),
            IrKind::StrCmp(cmp, o) => self.str_cmp(cmp, o),
            IrKind::BoolEq(equal, o) => self.bool_eq(equal, o),
            IrKind::RecordEq(o) => self.record_eq(o, true, node),
            IrKind::RecordNe(o) => self.record_eq(o, false, node),
            IrKind::RecordSame(o) => self.record_same(o, true),
            IrKind::RecordNotSame(o) => self.record_same(o, false),
            IrKind::ListEq(o) => self.list_eq(o, true, node),
            IrKind::ListNe(o) => self.list_eq(o, false, node),
            IrKind::ListSame(o) => self.list_same(o, true),
            IrKind::ListNotSame(o) => self.list_same(o, false),
            IrKind::And(o) => self.logic(o, false),
            IrKind::Or(o) => self.logic(o, true),
            IrKind::Not(operand) => self.not(operand),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a Bool was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a Bool was expected"),
            IrKind::StrConst(_)
            | IrKind::StrConcat(_)
            | IrKind::Interp(_)
            | IrKind::Caller
            | IrKind::FilePath(_) => mistyped(node, "String node where a Bool was expected"),
            IrKind::Show(_) => mistyped(node, "`show` node where a Bool was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::RecordOf { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where a Bool was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where a Bool was expected"),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, Value::into_bool),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where a Bool was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a Bool was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates an `IntCmp` testing `cmp`.
    pub(super) fn int_cmp(&mut self, cmp: Cmp, operands: Operands) -> Flow<bool> {
        let (lhs, rhs) = self.ints(operands)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `FloatCmp` testing `cmp`.
    pub(super) fn float_cmp(&mut self, cmp: Cmp, operands: Operands) -> Flow<bool> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `StrCmp` testing `cmp`.
    pub(super) fn str_cmp(&mut self, cmp: Cmp, operands: Operands) -> Flow<bool> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        Ok(cmp.test(&*lhs, &*rhs))
    }

    /// Evaluates a `BoolEq`: `==` if `equal`, `!=` otherwise.
    pub(super) fn bool_eq(&mut self, equal: bool, operands: Operands) -> Flow<bool> {
        let lhs = self.bool(operands.lhs)?;
        let rhs = self.bool(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates `and`, which a `false` left operand decides, or `or`, which
    /// a `true` one decides, as `decisive` says. The right operand is only
    /// evaluated when the left does not decide.
    pub(super) fn logic(&mut self, operands: Operands, decisive: bool) -> Flow<bool> {
        let lhs = self.bool(operands.lhs)?;
        if lhs == decisive {
            Ok(lhs)
        } else {
            self.bool(operands.rhs)
        }
    }

    /// Evaluates a `Not` of `operand`.
    pub(super) fn not(&mut self, operand: IrId) -> Flow<bool> {
        Ok(!self.bool(operand)?)
    }

    /// Evaluates a `String`-typed node.
    pub(super) fn string(&mut self, id: IrId) -> Flow<Rc<str>> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::StrConst(constant) => Ok(Rc::clone(self.ir.string(constant))),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::JsonDecode { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &STRING),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a String was expected"),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.string(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.string(branch)
            }
            IrKind::StrConcat(operands) => self.concat(operands),
            IrKind::Interp(items) => self.interpolate(items, node.span),
            IrKind::Show(_) | IrKind::Caller | IrKind::FilePath(_) => self.computed_string(node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a String was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a String was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where a String was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::RecordOf { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where a String was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where a String was expected"),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, Value::into_string),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where a String was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a String was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates a `StrConcat`, allocating exactly the length the result
    /// needs.
    pub(super) fn concat(&mut self, operands: Operands) -> Flow<Rc<str>> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        let mut text = String::with_capacity(lhs.len() + rhs.len());
        text.push_str(&lhs);
        text.push_str(&rhs);
        Ok(Rc::from(text))
    }

    /// Builds the text of an interpolation, spanning `span`, from its
    /// `items`: a `StrConst` is literal text, pushed as it is, and any other
    /// item is a value shown with its `Display`. The text is built in one
    /// buffer, reserved for the literal pieces and a little per value.
    pub(super) fn interpolate(&mut self, items: IrRange, span: Span) -> Flow<Rc<str>> {
        let items = self.ir.items(items);
        let estimate = items.iter().fold(0_usize, |total, &item| {
            let len = match self.ir.get(item).kind {
                IrKind::StrConst(constant) => self.ir.string(constant).len(),
                _ => INTERPOLATED_VALUE_ESTIMATE,
            };
            total.saturating_add(len)
        });
        let mut text = String::with_capacity(estimate);
        for &item in items {
            if let IrKind::StrConst(constant) = self.ir.get(item).kind {
                text.push_str(self.ir.string(constant));
            } else {
                let value = self.value(item)?;
                write!(text, "{value}")
                    .map_err(|_| internal("showing a value as text failed", span))?;
            }
        }
        Ok(Rc::from(text))
    }

    /// Evaluates `node`, a `Show` or a `Caller`, the `String` nodes whose
    /// text is made of what the run holds: a value's source, or the running
    /// frame's caller. Apart from [`Evaluator::string`], whose match it keeps
    /// short; any other node is an internal error.
    pub(super) fn computed_string(&mut self, node: &IrNode) -> Flow<Rc<str>> {
        match node.kind {
            IrKind::Show(operand) => self.show(operand, node),
            IrKind::Caller => self.caller_text(node),
            IrKind::FilePath(file) => match self.value(file)? {
                Value::File(file) | Value::Dir(file) => Ok(Rc::clone(&file.path)),
                _ => mistyped(
                    node,
                    "the path of a value that is neither a file nor a directory",
                ),
            },
            _ => mistyped(node, "a node whose text is not computed"),
        }
    }

    /// Evaluates a `Show`, the call `node` of `show`, of `operand`: its
    /// value's GCSL source (see [`source_of`]).
    pub(super) fn show(&mut self, operand: IrId, node: &IrNode) -> Flow<Rc<str>> {
        let value = self.value(operand)?;
        self.profiled_builtin(Builtin::Show, |_| source_of(&value, node))
    }

    /// A runtime error in a binary operator: the message points at the
    /// operator, and each operand is labelled with its value.
    pub(super) fn binary_error(
        &self,
        node: &IrNode,
        operands: Operands,
        message: &'static str,
        lhs: i64,
        rhs: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(
                self.ir.get(operands.lhs).span,
                self.operand_text(operands.lhs, lhs),
            )
            .with_context(
                self.ir.get(operands.rhs).span,
                self.operand_text(operands.rhs, rhs),
            )
    }

    /// A runtime error in a unary operator, labelling the operand with its
    /// value.
    pub(super) fn unary_error(
        &self,
        node: &IrNode,
        operand: IrId,
        message: &'static str,
        value: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operand).span, self.operand_text(operand, value))
    }
}
