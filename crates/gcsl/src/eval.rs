//! The evaluator: a recursive walk over [`Ir`].
//!
//! There is one walker per kind of value (see [`crate::value::ValueKind`]):
//! `Evaluator::int`, `Evaluator::float`, `Evaluator::bool` and
//! `Evaluator::string`, returning an `i64`, an `f64`, a `bool` and an
//! `Rc<str>`, and `Evaluator::record`, returning a [`Value`] that is
//! [`Value::Empty`] or [`Value::Record`], since both are records. Every IR
//! node states its kind, so the walker for a node is always known from its
//! parent and no runtime value is ever inspected to choose an operation.
//! Each typed walker handles the nodes of its kind and also loads, blocks,
//! `if`s and field accesses, which can be of any kind: an `if` evaluates its
//! condition through `bool` and then exactly one branch through the walker
//! it was called with, and a field access evaluates its record through
//! `record` and extracts the field's value. A general walker,
//! `Evaluator::value`, handles every node, and hands the typed nodes to the
//! typed walkers, building a [`Value`] only where a value of statically
//! unknown kind is needed: at the root, in a slot, in a record, as an
//! interpolated value, or as a block's result evaluated through `value`.
//!
//! A record is built from its record block's shared [`crate::value::Layout`]
//! and its field values in layout order, the one allocation for the values
//! and one for the reference-counted record. The empty record is unboxed,
//! so the statements that produce it allocate nothing. Reading a field finds
//! it by comparing interned names against the layout's.
//!
//! An `UpdateField` builds a new record and never changes the old one. It
//! clones the old record's values into a new boxed slice, one allocation,
//! with the updated field's value replaced or appended. A replacement
//! shares the old record's layout. An addition needs a new layout: the old
//! one's names and texts with the added name after them, whose text the
//! evaluator takes from [`Ir::name_text`], since it has no names table and
//! must never need one. Every addition of a name to a record of a given
//! layout makes an equal layout anew; caching the result per old layout and
//! name is the optimisation for a bytecode VM, which will have a place to
//! keep such a cache at each update site. Updating the empty record makes a
//! record of the one field.
//!
//! Typed equality, `RecordEq` and `RecordNe`, compares two records over the
//! [`crate::ir::Shape`] of their static type: for each field of the shape,
//! it looks the field up in both records by name, compares a record field
//! through its own shape and any other field with `==` on [`Value`], and
//! ignores every field the shape lacks. Nested records are compared through
//! a work list, as total equality compares them (see
//! [`crate::value::Record`]), so nesting does not recurse and comparing flat
//! records allocates nothing. Total equality, `RecordSame` and
//! `RecordNotSame`, is [`Value`]'s `PartialEq`.
//!
//! `and` and `or` short-circuit: the right operand is evaluated only when
//! the left does not decide the result. A string constant is a reference
//! to the IR's copy, so producing one increments a reference count and
//! copies nothing. Concatenation allocates exactly the length it needs, and
//! an interpolation builds its text in one buffer sized from its literal
//! pieces before sharing it.
//!
//! Variables live in a frame: one [`Value`] per slot, allocated once before
//! evaluation starts, sized by the checker's slot assignment. A `Load` in a
//! typed context reads its slot and extracts the unboxed value, cloning a
//! string's reference; a slot holding a value of another type there is an
//! internal error, since lowering typed the load. Storing values in
//! separate typed slots (`Vec<i64>`, `Vec<f64>`) would avoid the tag check;
//! that is a later optimisation for a bytecode VM, not something this tree
//! walker needs.
//!
//! Operands are evaluated left to right, so when both would fail, the left
//! one's error is reported. Items are evaluated in order. Evaluation holds
//! no per-node state and allocates nothing beyond the frame, except the
//! strings and records the program builds and the diagnostic when it
//! fails. A runtime
//! error points at the operator and labels each operand with the value it
//! had, so the user sees what actually overflowed or divided by zero.

use std::fmt::Write as _;
use std::num::NonZeroI64;
use std::rc::Rc;

use crate::check::Slot;
use crate::diagnostic::Diagnostic;
use crate::ir::{
    Cmp, Ir, IrId, IrKind, IrNode, IrRange, LayoutId, Operands, ShapeId, ShapedOperands,
};
use crate::names::Name;
use crate::source::Span;
use crate::value::{Layout, Record, Value, int_to_float};

/// The room an interpolation reserves for each interpolated value's text,
/// on top of its literal pieces' lengths.
const INTERPOLATED_VALUE_ESTIMATE: usize = 8;

/// Evaluates `ir` to a value, in a fresh frame.
///
/// # Errors
///
/// Returns a runtime error at the failing operator's span: "integer
/// overflow" for `Int` arithmetic that leaves the range of `i64`, and
/// "division by zero" for `//` or `%` by zero. `Float` arithmetic never
/// fails.
pub fn eval(ir: &Ir) -> Result<Value, Diagnostic> {
    // The one allocation of an evaluation, besides the values it builds.
    let mut frame = vec![Value::Empty; ir.frame_size() as usize];
    eval_in(ir, &mut frame)
}

/// Evaluates `ir` to a value in an existing `frame`, which must have at
/// least [`Ir::frame_size`] slots. Slots the program does not write keep
/// their values, which is how a [`crate::session::Session`] keeps its
/// variables from one input to the next.
///
/// # Errors
///
/// As [`eval`]; and an internal error if `frame` is too small.
pub fn eval_in(ir: &Ir, frame: &mut [Value]) -> Result<Value, Diagnostic> {
    let root = ir.root();
    if frame.len() < ir.frame_size() as usize {
        return Err(internal(
            "frame is smaller than the IR needs",
            ir.get(root).span,
        ));
    }
    Evaluator { ir, frame }.value(root)
}

struct Evaluator<'a> {
    ir: &'a Ir,
    frame: &'a mut [Value],
}

// The walkers `value`, `int`, `float`, `bool`, `string` and `record` are
// the recursion of evaluation: an expression nested N deep puts about N of
// their frames on the stack. Each is therefore only a dispatcher. The work
// of each operator is in a helper of its own, and every error is returned
// by a call in tail position, so a walker's frame does not hold the
// temporaries of every operator; an unoptimised build gives each of those
// a stack slot of its own, which would make the walkers' frames several
// kilobytes and `crate::STACK_SIZE` too small at `crate::MAX_DEPTH`.
impl Evaluator<'_> {
    /// Evaluates a node of any kind.
    fn value(&mut self, id: IrId) -> Result<Value, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty => Ok(Value::Empty),
            IrKind::Load(slot, _) => self.load(slot, node).cloned(),
            IrKind::Store(slot, value) => self.store(slot, value, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.value(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.value(branch)
            }
            IrKind::MakeRecord { layout, values } => self.make_record(layout, values),
            IrKind::Field { record, name, .. } => self.field(record, name, node),
            IrKind::UpdateField {
                record,
                name,
                value,
            } => self.update_field((record, name, value), node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => self.int(id).map(Value::Int),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => self.float(id).map(Value::Float),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => self.bool(id).map(Value::Bool),
            IrKind::StrConst(_) | IrKind::StrConcat(_) => self.string(id).map(Value::Str),
            // Directly rather than through `string`: an interpolation
            // evaluates its parts through `value`, so interpolations nested
            // in each other then recurse two frames per level, not three.
            IrKind::Interp(items) => self.interpolate(items, node.span).map(Value::Str),
        }
    }

    /// Evaluates a block's items, in order, for their effects.
    fn items(&mut self, items: IrRange) -> Result<(), Diagnostic> {
        for &item in self.ir.items(items) {
            self.value(item)?;
        }
        Ok(())
    }

    /// Evaluates the condition of an `if` and returns the branch it selects.
    fn branch(&mut self, cond: IrId, then: IrId, else_: IrId) -> Result<IrId, Diagnostic> {
        Ok(if self.bool(cond)? { then } else { else_ })
    }

    /// Evaluates `value` and writes it to `slot`, for the `Store` `node`.
    fn store(&mut self, slot: Slot, value: IrId, node: &IrNode) -> Result<Value, Diagnostic> {
        let value = self.value(value)?;
        let target = self
            .frame
            .get_mut(slot.index())
            .ok_or_else(|| internal("store to a slot outside the frame", node.span))?;
        *target = value;
        Ok(Value::Empty)
    }

    /// Builds a record of the values of `values`, in the order of `layout`'s
    /// names: the one allocation for its values, sized exactly, and one for
    /// the record, which shares the layout.
    fn make_record(&mut self, layout: LayoutId, values: IrRange) -> Result<Value, Diagnostic> {
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
    fn field(&mut self, record: IrId, name: Name, node: &IrNode) -> Result<Value, Diagnostic> {
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
    fn field_as<T>(
        &mut self,
        (record, name): (IrId, Name),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Result<T, Diagnostic> {
        match extract(self.field(record, name, node)?) {
            Some(value) => Ok(value),
            None => mistyped(node, "a field holds a value of another kind than its node"),
        }
    }

    /// The value in `slot`.
    fn load(&self, slot: Slot, node: &IrNode) -> Result<&Value, Diagnostic> {
        self.frame
            .get(slot.index())
            .ok_or_else(|| internal("load from a slot outside the frame", node.span))
    }

    /// Evaluates an `Int`-typed node.
    fn int(&mut self, id: IrId) -> Result<i64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::IntConst(value) => Ok(value),
            IrKind::Load(slot, _) => self.load_int(slot, node),
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
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Int(value) => Some(value),
                    _ => None,
                })
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
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where an Int was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where an Int was expected")
            }
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => mistyped(node, "record node where an Int was expected"),
        }
    }

    /// Reads the `Int` in `slot`, for the `Load` `node`.
    fn load_int(&self, slot: Slot, node: &IrNode) -> Result<i64, Diagnostic> {
        match self.load(slot, node)? {
            Value::Int(value) => Ok(*value),
            Value::Float(_) | Value::Bool(_) | Value::Str(_) | Value::Empty | Value::Record(_) => {
                mistyped(node, "Int load from a slot holding another type")
            }
        }
    }

    /// Evaluates both `Int` operands, left to right.
    fn ints(&mut self, operands: Operands) -> Result<(i64, i64), Diagnostic> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        Ok((lhs, rhs))
    }

    /// Evaluates the `IntNeg` `node` of `operand`.
    fn int_neg(&mut self, node: &IrNode, operand: IrId) -> Result<i64, Diagnostic> {
        let value = self.int(operand)?;
        value
            .checked_neg()
            .ok_or_else(|| self.unary_error(node, operand, OVERFLOW, value))
    }

    /// Evaluates the `+`, `-` or `*` `node`, which `op` computes, `None`
    /// meaning overflow.
    fn int_arithmetic(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, i64) -> Option<i64>,
    ) -> Result<i64, Diagnostic> {
        // Not through `ints`: this is on the recursion path of every
        // arithmetic chain, so it saves that frame.
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        op(lhs, rhs).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
    }

    /// Evaluates the `//` or `%` `node`, which `op` computes for a nonzero
    /// divisor, `None` meaning overflow.
    fn int_division(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, NonZeroI64) -> Option<i64>,
    ) -> Result<i64, Diagnostic> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        let Some(divisor) = NonZeroI64::new(rhs) else {
            return Err(self.binary_error(node, operands, DIVISION_BY_ZERO, lhs, rhs));
        };
        op(lhs, divisor).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
    }

    /// Evaluates a `Float`-typed node. IEEE 754 semantics; never fails except
    /// through an `Int` operand of `IntToFloat`, an item of a block or the
    /// condition of an `if`.
    fn float(&mut self, id: IrId) -> Result<f64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::FloatConst(value) => Ok(value),
            IrKind::Load(slot, _) => self.load_float(slot, node),
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
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Float(value) => Some(value),
                    _ => None,
                })
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a Float was expected"),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a Float was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where a Float was expected")
            }
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => {
                mistyped(node, "record node where a Float was expected")
            }
        }
    }

    /// Reads the `Float` in `slot`, for the `Load` `node`.
    fn load_float(&self, slot: Slot, node: &IrNode) -> Result<f64, Diagnostic> {
        match self.load(slot, node)? {
            Value::Float(value) => Ok(*value),
            Value::Int(_) | Value::Bool(_) | Value::Str(_) | Value::Empty | Value::Record(_) => {
                mistyped(node, "Float load from a slot holding another type")
            }
        }
    }

    /// Evaluates a `FloatNeg` of `operand`.
    fn float_neg(&mut self, operand: IrId) -> Result<f64, Diagnostic> {
        Ok(-self.float(operand)?)
    }

    /// Evaluates a `Float` operator that `op` computes.
    fn float_arithmetic(
        &mut self,
        operands: Operands,
        op: fn(f64, f64) -> f64,
    ) -> Result<f64, Diagnostic> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(op(lhs, rhs))
    }

    /// Evaluates a `Bool`-typed node. `and` and `or` evaluate their right
    /// operand only when the left does not decide the result.
    fn bool(&mut self, id: IrId) -> Result<bool, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::BoolConst(value) => Ok(value),
            IrKind::Load(slot, _) => self.load_bool(slot, node),
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
            IrKind::And(o) => self.logic(o, false),
            IrKind::Or(o) => self.logic(o, true),
            IrKind::Not(operand) => self.not(operand),
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Bool(value) => Some(value),
                    _ => None,
                })
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a Bool was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a Bool was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where a Bool was expected")
            }
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => mistyped(node, "record node where a Bool was expected"),
        }
    }

    /// Reads the `Bool` in `slot`, for the `Load` `node`.
    fn load_bool(&self, slot: Slot, node: &IrNode) -> Result<bool, Diagnostic> {
        match self.load(slot, node)? {
            Value::Bool(value) => Ok(*value),
            Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Empty | Value::Record(_) => {
                mistyped(node, "Bool load from a slot holding another type")
            }
        }
    }

    /// Evaluates an `IntCmp` testing `cmp`.
    fn int_cmp(&mut self, cmp: Cmp, operands: Operands) -> Result<bool, Diagnostic> {
        let (lhs, rhs) = self.ints(operands)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `FloatCmp` testing `cmp`.
    fn float_cmp(&mut self, cmp: Cmp, operands: Operands) -> Result<bool, Diagnostic> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `StrCmp` testing `cmp`.
    fn str_cmp(&mut self, cmp: Cmp, operands: Operands) -> Result<bool, Diagnostic> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        Ok(cmp.test(&*lhs, &*rhs))
    }

    /// Evaluates a `BoolEq`: `==` if `equal`, `!=` otherwise.
    fn bool_eq(&mut self, equal: bool, operands: Operands) -> Result<bool, Diagnostic> {
        let lhs = self.bool(operands.lhs)?;
        let rhs = self.bool(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates the typed record comparison `node`, a `RecordEq` if `equal`
    /// and a `RecordNe` otherwise: the operands compare over their shape
    /// (see [`record_eq_shaped`]).
    fn record_eq(
        &mut self,
        operands: ShapedOperands,
        equal: bool,
        node: &IrNode,
    ) -> Result<bool, Diagnostic> {
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
    fn record_same(&mut self, operands: Operands, equal: bool) -> Result<bool, Diagnostic> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates `and`, which a `false` left operand decides, or `or`, which
    /// a `true` one decides, as `decisive` says. The right operand is only
    /// evaluated when the left does not decide.
    fn logic(&mut self, operands: Operands, decisive: bool) -> Result<bool, Diagnostic> {
        let lhs = self.bool(operands.lhs)?;
        if lhs == decisive {
            Ok(lhs)
        } else {
            self.bool(operands.rhs)
        }
    }

    /// Evaluates a `Not` of `operand`.
    fn not(&mut self, operand: IrId) -> Result<bool, Diagnostic> {
        Ok(!self.bool(operand)?)
    }

    /// Evaluates a `String`-typed node.
    fn string(&mut self, id: IrId) -> Result<Rc<str>, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::StrConst(constant) => Ok(Rc::clone(self.ir.string(constant))),
            IrKind::Load(slot, _) => self.load_string(slot, node),
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
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Str(text) => Some(text),
                    _ => None,
                })
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a String was expected"),
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
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a String was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => {
                mistyped(node, "record node where a String was expected")
            }
        }
    }

    /// Reads the `String` in `slot`, for the `Load` `node`: a new reference
    /// to it, not a copy.
    fn load_string(&self, slot: Slot, node: &IrNode) -> Result<Rc<str>, Diagnostic> {
        match self.load(slot, node)? {
            Value::Str(text) => Ok(Rc::clone(text)),
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Empty | Value::Record(_) => {
                mistyped(node, "String load from a slot holding another type")
            }
        }
    }

    /// Evaluates a record node: [`Value::Empty`] or a [`Value::Record`].
    fn record(&mut self, id: IrId) -> Result<Value, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty => Ok(Value::Empty),
            IrKind::Load(slot, _) => self.load_record(slot, node),
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
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Empty | Value::Record(_) => Some(value),
                    _ => None,
                })
            }
            IrKind::UpdateField {
                record,
                name,
                value,
            } => self.update_field((record, name, value), node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a record was expected"),
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
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a record was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where a record was expected")
            }
        }
    }

    /// Evaluates the `UpdateField` `node`, setting field `name` of `record`
    /// to `value`: a new record, the old one unchanged. See the module
    /// documentation.
    fn update_field(
        &mut self,
        (record, name, value): (IrId, Name, IrId),
        node: &IrNode,
    ) -> Result<Value, Diagnostic> {
        let old = self.record(record)?;
        let value = self.value(value)?;
        let old = match old {
            Value::Record(old) => old,
            Value::Empty => {
                let Some(text) = self.ir.name_text(name) else {
                    return mistyped(node, "an update's name has no text");
                };
                return Ok(Value::Record(Rc::new(Record {
                    layout: Rc::new(Layout {
                        names: Box::new([name]),
                        texts: Box::new([Box::from(text)]),
                    }),
                    values: Box::new([value]),
                })));
            }
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) => {
                return mistyped(node, "an update of a value that is not a record");
            }
        };
        if let Some(index) = old.layout.names.iter().position(|&field| field == name) {
            // The same fields: the layout is shared.
            let mut values = old.values.clone();
            if let Some(slot) = values.get_mut(index) {
                *slot = value;
            }
            return Ok(Value::Record(Rc::new(Record {
                layout: Rc::clone(&old.layout),
                values,
            })));
        }
        let Some(text) = self.ir.name_text(name) else {
            return mistyped(node, "an update's name has no text");
        };
        let len = old.values.len() + 1;
        let mut names = Vec::with_capacity(len);
        names.extend_from_slice(&old.layout.names);
        names.push(name);
        let mut texts = Vec::with_capacity(len);
        texts.extend(old.layout.texts.iter().cloned());
        texts.push(Box::from(text));
        let mut values = Vec::with_capacity(len);
        values.extend(old.values.iter().cloned());
        values.push(value);
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: names.into_boxed_slice(),
                texts: texts.into_boxed_slice(),
            }),
            values: values.into_boxed_slice(),
        })))
    }

    /// Reads the record in `slot`, for the `Load` `node`: a new reference to
    /// it, not a copy.
    fn load_record(&self, slot: Slot, node: &IrNode) -> Result<Value, Diagnostic> {
        match self.load(slot, node)? {
            value @ (Value::Empty | Value::Record(_)) => Ok(value.clone()),
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) => {
                mistyped(node, "record load from a slot holding another type")
            }
        }
    }

    /// Evaluates a `StrConcat`, allocating exactly the length the result
    /// needs.
    fn concat(&mut self, operands: Operands) -> Result<Rc<str>, Diagnostic> {
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
    fn interpolate(&mut self, items: IrRange, span: Span) -> Result<Rc<str>, Diagnostic> {
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

    /// A runtime error in a binary operator: the message points at the
    /// operator, and each operand is labelled with its value.
    fn binary_error(
        &self,
        node: &IrNode,
        operands: Operands,
        message: &'static str,
        lhs: i64,
        rhs: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operands.lhs).span, lhs.to_string())
            .with_context(self.ir.get(operands.rhs).span, rhs.to_string())
    }

    /// A runtime error in a unary operator, labelling the operand with its
    /// value.
    fn unary_error(
        &self,
        node: &IrNode,
        operand: IrId,
        message: &'static str,
        value: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operand).span, value.to_string())
    }
}

const OVERFLOW: &str = "integer overflow";
const DIVISION_BY_ZERO: &str = "division by zero";

/// Floor division: the quotient rounded toward negative infinity. Returns
/// `None` on overflow, which only `i64::MIN // -1` causes.
///
/// Derivation. Rust's `/` truncates toward zero and `%` takes the sign of
/// the dividend, so `a == b * q + r` with `q = a / b`, `r = a % b` and
/// `|r| < |b|`. If `r == 0` the division is exact and `q` is already the
/// floor. Otherwise the exact quotient `a / b` lies strictly between `q` and
/// the next integer away from zero, and it is negative exactly when `r` and
/// `b` have opposite signs. In that case truncation rounded *up*, so the
/// floor is `q - 1`. When the signs agree the exact quotient is positive,
/// truncation rounded down, and `q` is the floor.
///
/// `q - 1` cannot overflow: it only happens when the exact quotient is
/// negative and not whole, so `q <= 0` and `q > i64::MIN`.
///
/// This is not `i64::div_euclid`, which rounds so that the remainder is
/// never negative, and so differs from floor division when `b < 0`
/// (`7.div_euclid(-2) == -3`, but `7 // -2 == -4`).
fn floor_div(a: i64, b: NonZeroI64) -> Option<i64> {
    let b = b.get();
    let q = a.checked_div(b)?;
    // Cannot wrap: `checked_div` has already rejected `i64::MIN / -1`.
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        Some(q - 1)
    } else {
        Some(q)
    }
}

/// The remainder matching [`floor_div`]: `a - b * floor_div(a, b)`, which
/// has the sign of the divisor `b` (or is zero).
///
/// Derivation. With `q`, `r` the truncating quotient and remainder as in
/// [`floor_div`], the floored quotient is `q - 1` exactly when `r != 0` and
/// `r`, `b` have opposite signs. Then the floored remainder is
/// `a - b * (q - 1) = (a - b * q) + b = r + b`, which has the sign of `b`
/// because `|r| < |b|`. Otherwise it is `r`, which is zero or already has
/// the sign of `b`. `r + b` cannot overflow, since `r` and `b` have opposite
/// signs.
///
/// Never overflows: `i64::MIN % -1` is mathematically `0`, which is what
/// `wrapping_rem` returns for it.
fn floor_mod(a: i64, b: NonZeroI64) -> i64 {
    let b = b.get();
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    }
}

/// Typed equality: whether records `a` and `b` are equal over shape
/// `shape`, comparing, for each field of the shape, the two records' fields
/// of that name, a record field through its own shape and any other with
/// `==` on [`Value`]. Fields the shape lacks are ignored, so a record with
/// no fields in its shape is equal to any other.
///
/// Nested records are compared through a work list rather than recursion,
/// as [`Record`]'s total equality does, and the list only grows when a
/// shape has a record field, so comparing flat records allocates nothing.
///
/// `None` if a record lacks a field of its shape, or holds something other
/// than a record in a field the shape has as a record: the checker
/// guarantees neither happens, so either is an internal error.
fn record_eq_shaped(ir: &Ir, shape: ShapeId, a: &Value, b: &Value) -> Option<bool> {
    let mut pending: Vec<(ShapeId, &Value, &Value)> = Vec::new();
    let (mut shape, mut a, mut b) = (shape, a, b);
    loop {
        let fields = &ir.shape(shape).fields;
        if !fields.is_empty() {
            let (Value::Record(a), Value::Record(b)) = (a, b) else {
                return None;
            };
            for &(name, nested) in fields {
                let (mine, theirs) = (a.get(name)?, b.get(name)?);
                match nested {
                    Some(nested) => pending.push((nested, mine, theirs)),
                    None if mine == theirs => {}
                    None => return Some(false),
                }
            }
        }
        let Some(next) = pending.pop() else {
            return Some(true);
        };
        (shape, a, b) = next;
    }
}

fn internal(message: &'static str, span: Span) -> Diagnostic {
    crate::diagnostic::InternalError { message, span }.into()
}

/// Fails with the internal error `message` at `node`: a node of a type its
/// walker does not handle. Returned directly by the walkers' error arms, so
/// that they hold no temporary for it (see the note on `Evaluator`).
fn mistyped<T>(node: &IrNode, message: &'static str) -> Result<T, Diagnostic> {
    Err(internal(message, node.span))
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::lower::lower;
    use crate::parser::parse;
    use crate::source::SourceFile;
    use crate::types::TypeTable;
    use crate::value::ValueKind;

    fn compile(text: &str) -> Ir {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, strings, &source).unwrap();
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        lower(&ast, &checked, &types).unwrap()
    }

    fn boolean(text: &str) -> bool {
        match run(text).unwrap() {
            Value::Bool(value) => value,
            other => panic!("{text}: expected a Bool, got {other:?}"),
        }
    }

    fn string(text: &str) -> String {
        match run(text) {
            Ok(Value::Str(value)) => value.to_string(),
            other => panic!("{text}: expected a String, got {other:?}"),
        }
    }

    /// Checks that each program evaluates to the Bool given.
    fn booleans(cases: &[(&str, bool)]) {
        for &(text, expected) in cases {
            assert_eq!(boolean(text), expected, "{text}");
        }
    }

    fn run(text: &str) -> Result<Value, Diagnostic> {
        eval(&compile(text))
    }

    fn int(text: &str) -> i64 {
        match run(text).unwrap() {
            Value::Int(value) => value,
            other => panic!("{text}: expected an Int, got {other:?}"),
        }
    }

    fn float(text: &str) -> f64 {
        match run(text).unwrap() {
            Value::Float(value) => value,
            other => panic!("{text}: expected a Float, got {other:?}"),
        }
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = run(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
    }

    /// `i64::MIN`, which cannot be written as a literal.
    const MIN: &str = "(-9223372036854775807 - 1)";
    const MAX: &str = "9223372036854775807";

    fn nz(value: i64) -> NonZeroI64 {
        NonZeroI64::new(value).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(int("1 + 2 * 3"), 7);
        assert_eq!(int("8 - 3 - 2"), 3);
        assert_eq!(int("2 * 3 // 4"), 1);
        assert_eq!(int("-2 * 3"), -6);
        assert_eq!(int("-(-2)"), 2);
        assert_eq!(int("2 * -3"), -6);
        assert_eq!(float("7 / 2"), 3.5);
        assert_eq!(float("7 / 2.0"), 3.5);
        assert_eq!(float("(1 + 2) * 3.0"), 9.0);
        assert_eq!(float("1.5 - 2"), -0.5);
        assert_eq!(float("-1.5"), -1.5);
        assert_eq!(float("(7 // 2) / 2"), 1.5);
    }

    #[test]
    fn floor_division_and_modulo_in_all_sign_combinations() {
        assert_eq!((int("7 // 2"), int("7 % 2")), (3, 1));
        assert_eq!((int("-7 // 2"), int("-7 % 2")), (-4, 1));
        assert_eq!((int("7 // -2"), int("7 % -2")), (-4, -1));
        assert_eq!((int("-7 // -2"), int("-7 % -2")), (3, -1));
    }

    #[test]
    fn exact_division_needs_no_adjustment() {
        assert_eq!((int("6 // 2"), int("6 % 2")), (3, 0));
        assert_eq!((int("-6 // 2"), int("-6 % 2")), (-3, 0));
        assert_eq!((int("6 // -2"), int("6 % -2")), (-3, 0));
        assert_eq!((int("-6 // -2"), int("-6 % -2")), (3, 0));
        assert_eq!((int("0 // -5"), int("0 % -5")), (0, 0));
    }

    #[test]
    fn floor_division_identity_holds() {
        for a in [-9, -7, -1, 0, 1, 7, 9, i64::MIN, i64::MAX] {
            for b in [-7, -2, -1, 1, 2, 7, i64::MIN, i64::MAX] {
                let Some(q) = floor_div(a, nz(b)) else {
                    assert_eq!((a, b), (i64::MIN, -1));
                    continue;
                };
                let r = floor_mod(a, nz(b));
                assert_eq!(i128::from(b) * i128::from(q) + i128::from(r), i128::from(a));
                assert!(r == 0 || (r < 0) == (b < 0), "{a} % {b} = {r}");
                assert!(r.unsigned_abs() < b.unsigned_abs());
            }
        }
    }

    #[test]
    fn differs_from_euclidean_division() {
        assert_eq!(7_i64.div_euclid(-2), -3);
        assert_eq!(floor_div(7, nz(-2)), Some(-4));
        assert_eq!(7_i64.rem_euclid(-2), 1);
        assert_eq!(floor_mod(7, nz(-2)), -1);
    }

    #[test]
    fn overflow_on_each_operator() {
        let overflow = "integer overflow".to_owned();
        // The error points at the operator.
        assert_eq!(
            error(&format!("{MAX} + 1")),
            (overflow.clone(), Span::new(20, 21))
        );
        assert_eq!(error(&format!("{MIN} - 1")).0, overflow);
        assert_eq!(error(&format!("{MAX} * 2")).0, overflow);
        assert_eq!(
            error(&format!("-{MIN}")),
            (overflow.clone(), Span::new(0, 1))
        );
        assert_eq!(error(&format!("{MIN} // -1")).0, overflow);
        assert_eq!(error(&format!("{MIN} * -1")).0, overflow);
    }

    #[test]
    fn runtime_errors_label_operands_with_their_values() {
        let diagnostic = run("2 * (9223372036854775807 - -1)").unwrap_err();
        assert_eq!(diagnostic.message, "integer overflow");
        assert_eq!(diagnostic.span, Span::new(25, 26));
        let labels: Vec<(Span, &str)> = diagnostic
            .labels
            .iter()
            .map(|label| (label.span, label.text.as_str()))
            .collect();
        assert_eq!(
            labels,
            vec![
                (Span::new(5, 24), "9223372036854775807"),
                (Span::new(27, 29), "-1")
            ]
        );
        let diagnostic = run("-(-9223372036854775807 - 1)").unwrap_err();
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].text, "-9223372036854775808");
        // The parenthesised operand is labelled without its parentheses.
        assert_eq!(diagnostic.labels[0].span, Span::new(2, 26));
    }

    #[test]
    fn i64_min_cases() {
        assert_eq!(int(MIN), i64::MIN);
        assert_eq!(int(&format!("{MIN} % -1")), 0);
        assert_eq!(int(&format!("{MIN} // 1")), i64::MIN);
        assert_eq!(int(&format!("{MIN} // 2")), i64::MIN / 2);
        assert_eq!(int(&format!("{MIN} // {MAX}")), -2);
        assert_eq!(int(&format!("{MIN} % {MAX}")), i64::MAX - 1);
        assert_eq!(int(&format!("{MIN} % 2")), 0);
        assert_eq!(int(&format!("-({MIN} + 1)")), i64::MAX);
        assert_eq!(float(&format!("{MIN} / -1")), 9_223_372_036_854_775_808.0);
    }

    #[test]
    fn division_by_zero() {
        let message = "division by zero".to_owned();
        assert_eq!(error("7 // 0"), (message.clone(), Span::new(2, 4)));
        assert_eq!(error("7 % 0"), (message.clone(), Span::new(2, 3)));
        assert_eq!(error("1 + 7 % (2 - 2)"), (message, Span::new(6, 7)));
        let diagnostic = run("1 + 7 % (2 - 2)").unwrap_err();
        assert_eq!(diagnostic.labels[1].text, "0");
        assert_eq!(diagnostic.labels[1].span, Span::new(9, 14));
    }

    #[test]
    fn float_division_by_zero_is_ieee() {
        assert_eq!(float("1.0 / 0.0"), f64::INFINITY);
        assert_eq!(float("-1.0 / 0.0"), f64::NEG_INFINITY);
        assert!(float("0.0 / 0.0").is_nan());
        assert_eq!(float("1 / 0"), f64::INFINITY);
        assert!(float("1e308 * 10.0").is_infinite());
        assert!(float("(1.0 / 0.0) - (1.0 / 0.0)").is_nan());
    }

    #[test]
    fn errors_are_reported_left_to_right() {
        assert_eq!(
            error("(1 // 0) + (9223372036854775807 + 1)"),
            ("division by zero".to_owned(), Span::new(3, 5))
        );
        assert_eq!(
            error("(9223372036854775807 + 1) + (1 // 0)"),
            ("integer overflow".to_owned(), Span::new(21, 22))
        );
    }

    #[test]
    fn root_value_type_follows_the_ir() {
        assert_eq!(run("1").unwrap().kind(), ValueKind::Int);
        assert_eq!(run("1 / 1").unwrap().kind(), ValueKind::Float);
        assert_eq!(run("").unwrap().kind(), ValueKind::Record);
        assert_eq!(run("x = 1").unwrap().kind(), ValueKind::Record);
    }

    #[test]
    fn blocks_yield_their_result() {
        assert_eq!(int("let x = { let y = 1; y + 1 }\nx"), 2);
        assert_eq!(int("{ { { 3 } } }"), 3);
        assert_eq!(int("{ 1 } + { 2 } * 3"), 7);
        assert_eq!(int("{ = 4 }"), 4);
        assert_eq!(int("let x = 1\n= { let x = 5\n= x } - x"), 4);
        assert_eq!(float("{ 1.5 } * 2"), 3.0);
        assert_eq!(float("{ let h = 0.5\nh } / { 2 }"), 0.25);
    }

    #[test]
    fn shadowing() {
        assert_eq!(
            int("let x = 1\nlet x = x + 1\n{ let x = x * 10; x } + x"),
            22
        );
        // A shadowing `let` may change the type.
        assert_eq!(float("let x = 1\nlet x = 2.5\nx"), 2.5);
        // The outer variable is intact after an inner block shadows it.
        assert_eq!(int("let x = 1\n{ let x = 2 }\nx"), 1);
    }

    #[test]
    fn rebinding() {
        assert_eq!(int("let x = 1\nx := x + 1\nx := x * 3\nx"), 6);
        // The literal adapts to the variable's type.
        assert_eq!(float("let x: Float = 1\nx := 2\nx"), 2.0);
        assert_eq!(int("{ let n = 1; n := n + 1; n }"), 2);
    }

    #[test]
    fn ascription() {
        assert_eq!(float("1 + 2 : Float"), 3.0);
        assert_eq!(float("-5 : Float"), -5.0);
        assert_eq!(float("let x: Float = 5\nx / 2"), 2.5);
        assert_eq!(float("let x = 5 : Float\nx * 2"), 10.0);
        assert_eq!(float("{ 5 } : Float"), 5.0);
        assert_eq!(int("let x = 3\nx : Int"), 3);
    }

    #[test]
    fn nested_blocks_reuse_slots() {
        let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
        assert_eq!(int(text), 114);
        // Each temporary shares its slot with the variable declared after
        // it; only the innermost `w` needs a slot of its own.
        assert_eq!(compile(text).frame_size(), 4);
    }

    #[test]
    fn empty_record_results() {
        for text in [
            "",
            "{}",
            "let x = 1",
            "{ let x = 1 }",
            "let u = {}\nu",
            "{ {}; {} }",
            "let x = 1\nx := 2",
            "= {}",
            "if true {}",
            "{ e = {} }.e",
        ] {
            assert_eq!(run(text).unwrap(), Value::Empty, "{text}");
        }
    }

    #[test]
    fn items_run_in_order_and_their_errors_are_reported() {
        assert_eq!(
            error("let x = 1 // 0\n5"),
            ("division by zero".to_owned(), Span::new(10, 12))
        );
        assert_eq!(
            error("{ let y = 9223372036854775807 + 1 }\n1"),
            ("integer overflow".to_owned(), Span::new(30, 31))
        );
        // The first failing item wins.
        assert_eq!(
            error("let a = 1 // 0\nlet b = 9223372036854775807 + 1\nb").0,
            "division by zero"
        );
    }

    #[test]
    fn eval_in_uses_the_given_frame() {
        let ir = compile("let x = 7\nlet y = x + 1");
        let mut frame = vec![Value::Empty; 3];
        assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
        assert_eq!(frame, vec![Value::Int(7), Value::Int(8), Value::Empty]);
        let error = eval_in(&ir, &mut frame[..1]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: frame is smaller than the IR needs"
        );
    }

    #[test]
    fn int_comparisons() {
        booleans(&[
            ("1 == 1", true),
            ("1 == 2", false),
            ("1 != 2", true),
            ("1 != 1", false),
            ("1 < 2", true),
            ("2 < 1", false),
            ("1 < 1", false),
            ("1 <= 1", true),
            ("2 <= 1", false),
            ("2 > 1", true),
            ("1 > 1", false),
            ("1 >= 1", true),
            ("1 >= 2", false),
            ("-5 < 3", true),
            ("(-9223372036854775807 - 1) < 9223372036854775807", true),
            ("let x = 7\nx // 2 == 3", true),
        ]);
    }

    #[test]
    fn float_comparisons_follow_ieee_754() {
        booleans(&[
            ("1.5 == 1.5", true),
            ("1.5 != 1.5", false),
            ("1.5 < 2.5", true),
            ("2.5 <= 2.5", true),
            ("2.5 > 1", true),
            ("1 >= 1.5", false),
            ("0.1 + 0.2 == 0.3", false),
            ("-0.0 == 0.0", true),
            ("-0.0 != 0.0", false),
            ("-0.0 < 0.0", false),
            ("-0.0 >= 0.0", true),
            ("1.0 / 0.0 > 1e308", true),
            ("-1.0 / 0.0 < -1e308", true),
        ]);
        // NaN is unequal and unordered to everything, itself included.
        let nan = "let nan = 0.0 / 0.0\n";
        for (comparison, expected) in [
            ("nan == nan", false),
            ("nan != nan", true),
            ("nan < nan", false),
            ("nan <= nan", false),
            ("nan > nan", false),
            ("nan >= nan", false),
            ("nan == 1.0", false),
            ("nan != 1.0", true),
            ("nan < 1.0", false),
            ("1.0 < nan", false),
            ("nan >= 1.0", false),
            ("1.0 >= nan", false),
        ] {
            assert_eq!(
                boolean(&format!("{nan}{comparison}")),
                expected,
                "{comparison}"
            );
        }
    }

    #[test]
    fn string_comparisons_are_bytewise() {
        booleans(&[
            ("\"a\" < \"b\"", true),
            ("\"Z\" < \"a\"", true),
            ("\"\" < \"a\"", true),
            ("\"ab\" < \"b\"", true),
            ("\"b\" < \"ab\"", false),
            ("\"a\" <= \"a\"", true),
            ("\"a\" > \"A\"", true),
            ("\"abc\" >= \"abd\"", false),
            ("\"é\" > \"z\"", true),
            ("\"a\" == \"a\"", true),
            ("\"a\" == \"b\"", false),
            ("\"a\" != \"b\"", true),
            ("\"\" == \"\"", true),
            ("\"a\" == \"a \"", false),
            // Equality compares content, however the strings were built.
            ("\"{1}{2}\" == \"12\"", true),
            ("\"a\" + \"b\" == r\"ab\"", true),
            ("\"\"\"a\"\"\" == \"a\"", true),
        ]);
    }

    #[test]
    fn bool_and_empty_record_equality() {
        booleans(&[
            ("true == true", true),
            ("false == false", true),
            ("true == false", false),
            ("true != false", true),
            ("false != false", false),
            ("(1 < 2) == (2 > 1)", true),
            ("{} == {}", true),
            ("{} != {}", false),
            ("let u = {}\nu == { let x = 1 }", true),
        ]);
        // Empty record operands are still evaluated.
        assert_eq!(
            error("{ let y = 1 // 0 } == {}"),
            ("division by zero".to_owned(), Span::new(12, 14))
        );
    }

    #[test]
    fn records_are_built_and_read() {
        assert_eq!(int("{ x = 1; y = 2 }.y"), 2);
        assert_eq!(float("{ a = { n = 1.5 } }.a.n"), 1.5);
        assert!(boolean("{ \"a b\" = true }.\"a b\""));
        assert_eq!(string("{ s = \"hi\" }.s"), "hi");
        // Fields see earlier fields and private `let`s; later `let`s do not
        // change a field.
        assert_eq!(int("{ x = 1; let t = x * 10; y = t + x }.y"), 11);
        assert_eq!(
            int("let r = { x = 1; let x = 5; y = x }\nr.x * 10 + r.y"),
            15
        );
        // Stored in a variable and read back, whole and field by field.
        assert_eq!(
            run("let p = { x = 1; y = \"a\" }\nlet q = p\nq")
                .unwrap()
                .to_string(),
            "{x = 1; y = \"a\"}"
        );
        assert_eq!(int("let p = { x = 3; y = 4 }\np.x * p.x + p.y * p.y"), 25);
        // A record's field can be a record, read at any depth, and a record
        // can be rebuilt from another's fields.
        assert_eq!(
            run("let p = { a = { b = { c = 7 } } }\n{ c = p.a.b.c; d = p.a }")
                .unwrap()
                .to_string(),
            "{c = 7; d = {b = {c = 7}}}"
        );
        // An empty record as a field.
        assert_eq!(run("{ e = {} }").unwrap().to_string(), "{e = {}}");
        // Checked against an annotation, a literal is built at the field's
        // type.
        assert_eq!(float("let p: {x: Float} = { x = 1 }\np.x / 2"), 0.5);
        // A file of fields is a record.
        assert_eq!(
            run("x = 1\ny = \"a\"").unwrap().to_string(),
            "{x = 1; y = \"a\"}"
        );
    }

    #[test]
    fn records_display_as_gcsl_syntax() {
        let value = run("{ x = 1; y = \"a\\n\"; inner = { n = 2.0 }; \"a b\" = true }").unwrap();
        let shown = "{x = 1; y = \"a\\n\"; inner = {n = 2.0}; \"a b\" = true}";
        assert_eq!(value.to_string(), shown);
        assert_eq!(value.repr().to_string(), shown);
    }

    #[test]
    fn record_equality_is_deep_and_by_name() {
        booleans(&[
            ("{ x = 1; y = \"a\" } == { x = 1; y = \"a\" }", true),
            ("{ x = 1; y = \"a\" } == { x = 1; y = \"b\" }", false),
            ("{ x = 1; y = \"a\" } != { x = 2; y = \"a\" }", true),
            // Fields in a different order, of the same type.
            ("{ x = 1; y = 2 } == { y = 2; x = 1 }", true),
            ("{ x = 1; y = 2 } == { y = 1; x = 2 }", false),
            // Nested.
            ("{ a = { n = 1 } } == { a = { n = 1 } }", true),
            ("{ a = { n = 1 } } == { a = { n = 2 } }", false),
            ("{ e = {} } == { e = {} }", true),
            // Floats compare as IEEE 754 does, so NaN is unequal to itself.
            ("let r = { f = 0.0 / 0.0 }\nr == r", false),
            ("let r = { f = 0.0 / 0.0 }\nr != r", true),
            ("{ f = -0.0 } == { f = 0.0 }", true),
            // Through variables.
            ("let p = { x = 1 }\nlet q = { x = 1 }\np == q", true),
        ]);
        // Operands are evaluated left to right.
        assert_eq!(
            error("{ x = 1 // 0 } == { x = 9223372036854775807 + 1 }"),
            ("division by zero".to_owned(), Span::new(8, 10))
        );
    }

    #[test]
    fn if_selects_records() {
        assert_eq!(
            run("if true { { x = 1; y = 2 } } else { { x = 3 } }")
                .unwrap()
                .to_string(),
            "{x = 1; y = 2}"
        );
        assert_eq!(
            int("let r = if 1 > 2 { { x = 1; y = 2 } } else { { x = 3 } }\nr.x"),
            3
        );
        assert_eq!(
            int("(if false { { x = 1 } } else { { x = 2; z = 0 } }).x"),
            2
        );
    }

    #[test]
    fn records_in_interpolations_show_their_repr() {
        assert_eq!(
            string("let p = { x = 1; s = \"a\" }\n\"p is {p}, {{}} is { {} }\""),
            "p is {x = 1; s = \"a\"}, {} is {}"
        );
    }

    #[test]
    fn logic() {
        booleans(&[
            ("true and true", true),
            ("true and false", false),
            ("false and true", false),
            ("true or false", true),
            ("false or false", false),
            ("not true", false),
            ("not false", true),
            ("not not true", true),
            ("not 1 == 2 and 3 > 2 or false", true),
        ]);
    }

    #[test]
    fn and_and_or_short_circuit() {
        // The right operand would fail, so it is never evaluated.
        booleans(&[
            ("false and 1 // 0 == 0", false),
            ("true or 1 // 0 == 0", true),
            ("not (false and 1 // 0 == 0)", true),
            ("false and (true or 1 // 0 == 0)", false),
        ]);
        // When the left operand does not decide, it is.
        assert_eq!(
            error("true and 1 // 0 == 0"),
            ("division by zero".to_owned(), Span::new(11, 13))
        );
        assert_eq!(
            error("false or 1 // 0 == 0"),
            ("division by zero".to_owned(), Span::new(11, 13))
        );
    }

    #[test]
    fn if_evaluates_exactly_one_branch() {
        assert_eq!(int("if true { 1 } else { 1 // 0 }"), 1);
        assert_eq!(int("if false { 1 // 0 } else { 2 }"), 2);
        assert_eq!(
            int("if 1 > 2 { 1 // 0 } else if 2 > 1 { 3 } else { 1 // 0 }"),
            3
        );
        assert_eq!(float("if true { 1.5 } else { 1.0 / 0.0 }"), 1.5);
        assert!(!boolean("if false { true } else { false }"));
        assert_eq!(string("if 1 < 2 { \"yes\" } else { \"no\" }"), "yes");
        assert_eq!(
            run("if true {} else { let x = 1 // 0 }").unwrap(),
            Value::Empty
        );
        assert_eq!(int("if false { let x = 1 // 0 }\n5"), 5);
        // The branch taken, and the condition, are evaluated.
        assert_eq!(
            error("if true { 1 // 0 } else { 2 }"),
            ("division by zero".to_owned(), Span::new(12, 14))
        );
        assert_eq!(
            error("if 1 // 0 == 0 { 1 } else { 2 }"),
            ("division by zero".to_owned(), Span::new(5, 7))
        );
        assert_eq!(
            error("if true { let y = 1 // 0 }"),
            ("division by zero".to_owned(), Span::new(20, 22))
        );
        // Adapted and defaulted branches.
        assert_eq!(float("if true { 1 } else { 2.5 }"), 1.0);
        assert_eq!(float("(if false { 1 } else { 2 }) * 0.5"), 1.0);
        assert_eq!(int("let x = if true { 7 } else { 8 }\nx + 1"), 8);
    }

    #[test]
    fn interpolating_every_type() {
        assert_eq!(
            string(
                "\"{1} {2.5} {3.0} {true} {\"s\"} { {} } {1 == 2} {1.0 / 0.0} {0.0 / 0.0} {-0.0}\""
            ),
            "1 2.5 3.0 true s {} false inf nan -0.0"
        );
        assert_eq!(
            string("let n = 3\nlet s = \"x\"\nlet b = n > 2\n\"{s}{n}{s} {b}\""),
            "x3x true"
        );
        assert_eq!(string("\"{ { let a = 2; a * 3 } }\""), "6");
        assert_eq!(string("\"{\"\"}\""), "");
        assert_eq!(string("\"say \\\"{\"hi\"}\\\" {{ok}}\""), "say \"hi\" {ok}");
        assert_eq!(
            string("\"status: {if 1 > 2 { \"ok\" } else { \"failed\" }}\""),
            "status: failed"
        );
    }

    #[test]
    fn nested_interpolation() {
        assert_eq!(string("\"a{ \"b{ \"c{1}\" }\" }d\""), "abc1d");
        assert_eq!(string("\"{ \"{ \"{ 1 + 1 }\" }\" }\""), "2");
        assert_eq!(
            string("let who = \"world\"\n\"<{ \"{who} and {\"{who}!\"}\" }>\""),
            "<world and world!>"
        );
    }

    #[test]
    fn interpolation_errors_come_from_its_values() {
        assert_eq!(
            error("\"a{1 // 0}\""),
            ("division by zero".to_owned(), Span::new(5, 7))
        );
    }

    #[test]
    fn concatenation() {
        assert_eq!(string("\"a\" + \"b\""), "ab");
        assert_eq!(string("\"\" + \"\""), "");
        assert_eq!(string("\"x\" + \"{1 + 1}\" + \"y\""), "x2y");
        assert_eq!(string("let s = \"ab\"\ns + s + s"), "ababab");
        assert_eq!(string("\"é\" + \"😀\""), "é😀");
    }

    #[test]
    fn strings_round_trip_through_variables_and_blocks() {
        assert_eq!(
            string("let s = \"hello\"\nlet t = { let u = s; u }\nt"),
            "hello"
        );
        assert_eq!(string("let s = \"a\"\ns := s + \"b\"\n{ s }"), "ab");
        assert!(boolean("let s = \"x\"\n{ s } == s"));
        assert_eq!(string("let s = \"q\"\n= { = { s } }"), "q");
        assert_eq!(run("\"a\"").unwrap().kind(), ValueKind::String);
        assert_eq!(run("true").unwrap().kind(), ValueKind::Bool);
    }

    #[test]
    fn loading_a_string_shares_it() {
        let ir = compile("let s = \"abc\"\nlet t = s\nlet u = s + \"\"");
        let mut frame = vec![Value::Empty; 3];
        assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
        let [Value::Str(s), Value::Str(t), Value::Str(u)] = &frame[..] else {
            panic!("expected strings, got {frame:?}");
        };
        // A load copies the reference, and concatenation makes a new string.
        assert!(Rc::ptr_eq(s, t));
        assert!(!Rc::ptr_eq(s, u));
        assert_eq!(u.as_ref(), "abc");
    }

    #[test]
    fn a_load_of_the_wrong_type_is_an_internal_error() {
        // `x + 1` with `x` loaded as an Int, from a frame holding a Float.
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let load = builder.alloc(IrKind::Load(Slot::new(0), ValueKind::Int), Span::new(0, 1));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
        let add = builder.alloc(
            IrKind::IntAdd(Operands {
                lhs: load,
                rhs: one,
                op_start: 2,
            }),
            Span::new(0, 5),
        );
        let ir = builder.finish(add, 1, Vec::new());
        assert_eq!(eval_in(&ir, &mut [Value::Int(2)]).unwrap(), Value::Int(3));
        let error = eval_in(&ir, &mut [Value::Float(1.5)]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: Int load from a slot holding another type"
        );
        assert_eq!(error.span, Span::new(0, 1));
    }

    /// The record in `value`, which must be one.
    fn record_of(value: &Value) -> &Rc<Record> {
        match value {
            Value::Record(record) => record,
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn dotted_fields_merge_into_records() {
        assert_eq!(
            run("server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true")
                .unwrap()
                .to_string(),
            "{server = {host = \"localhost\"; port = 8080; tls = {enabled = true}}}"
        );
        // Fields in the order of first mention, at every level.
        assert_eq!(
            run("{ a.b.c = 1; d = 2; a.e = 3; a.b.f = 4 }")
                .unwrap()
                .to_string(),
            "{a = {b = {c = 1; f = 4}; e = 3}; d = 2}"
        );
        // Later items see the record built so far.
        assert_eq!(
            run("{ a.x = 1; let p = a; a.y = a.x + 1; q = p }")
                .unwrap()
                .to_string(),
            "{a = {x = 1; y = 2}; q = {x = 1}}"
        );
        assert_eq!(int("{ a.x = 1; a.y = 2 }.a.y"), 2);
    }

    #[test]
    fn an_update_that_replaces_a_field_shares_the_layout() {
        let ir = compile(
            "let a = { x = 1; s = \"text\" }\n\
             let b = a\n\
             let b.x = 5\n\
             let c = a\n\
             let c.z = true",
        );
        let mut frame = vec![Value::Empty; 5];
        eval_in(&ir, &mut frame).unwrap();
        let [a, _, replaced, _, added] = &frame[..] else {
            panic!("expected five slots, got {frame:?}");
        };
        // The old record is intact.
        assert_eq!(a.to_string(), "{x = 1; s = \"text\"}");
        assert_eq!(replaced.to_string(), "{x = 5; s = \"text\"}");
        let (a, replaced) = (record_of(a), record_of(replaced));
        assert!(Rc::ptr_eq(&a.layout, &replaced.layout));
        assert!(!Rc::ptr_eq(a, replaced));
        // An addition makes a new layout, with the name at the end.
        assert_eq!(added.to_string(), "{x = 1; s = \"text\"; z = true}");
        let added = record_of(added);
        assert!(!Rc::ptr_eq(&a.layout, &added.layout));
        assert_eq!(added.layout.names.len(), 3);
        assert_eq!(added.layout.names[..2], a.layout.names[..]);
        assert!(!a.layout.names.contains(&added.layout.names[2]));
        assert_eq!(
            &*added.layout.texts,
            &[Box::from("x"), Box::from("s"), Box::from("z")]
        );
        // The unchanged fields are shared, not copied.
        let shared = |record: &Record| match &record.values[1] {
            Value::Str(text) => Rc::clone(text),
            other => panic!("expected a string, got {other:?}"),
        };
        assert!(Rc::ptr_eq(&shared(a), &shared(replaced)));
        assert!(Rc::ptr_eq(&shared(a), &shared(added)));
    }

    #[test]
    fn updates_nest_and_create_records() {
        assert_eq!(
            run("let a = { b = { c = 1; d = 2 } }\nlet a.b.c = 10\na")
                .unwrap()
                .to_string(),
            "{b = {c = 10; d = 2}}"
        );
        assert_eq!(
            run("let a = { x = 1 }\nlet a.b.c = 2\na")
                .unwrap()
                .to_string(),
            "{x = 1; b = {c = 2}}"
        );
        assert_eq!(
            run("let a.b.c = \"s\"\na").unwrap().to_string(),
            "{b = {c = \"s\"}}"
        );
        assert_eq!(
            run("let a = { x = 1 }\na.x := a.x + 1\nlet a.y.z = false\na.y.z := true\na")
                .unwrap()
                .to_string(),
            "{x = 2; y = {z = true}}"
        );
        // A leaf replaced by a value of another type.
        assert_eq!(
            run("let a = { b = { c = 1 } }\nlet a.b = 2.5\na")
                .unwrap()
                .to_string(),
            "{b = 2.5}"
        );
        // A field the static type has as `{}` may hold the empty record.
        assert_eq!(
            run("let a = { e = {} }\nlet a.e.x = 1\na")
                .unwrap()
                .to_string(),
            "{e = {x = 1}}"
        );
        // A field hidden by subtyping is replaced where the path creates a
        // record, since the static type is the truth.
        assert_eq!(
            run("let p: {x: Int} = { x = 1; y = 2 }\nlet p.y.z = 3\np")
                .unwrap()
                .to_string(),
            "{x = 1; y = {z = 3}}"
        );
        // The value is evaluated after the record, and its errors reported.
        assert_eq!(
            error("let a = { x = 1 }\nlet a.y = 1 // 0").0,
            "division by zero"
        );
    }

    #[test]
    fn an_update_leaves_the_old_value_intact() {
        assert_eq!(
            string("let a = { x = 1; n = { m = 2 } }\nlet old = a\nlet a.n.m = 3\n\"{old} {a}\""),
            "{x = 1; n = {m = 2}} {x = 1; n = {m = 3}}"
        );
        assert_eq!(
            string("let a = { x = 1 }\nlet old = a\na.x := 2\n\"{old} {a}\""),
            "{x = 1} {x = 2}"
        );
        // Through a field: the field's record is extended in its slot, and
        // what was read from it before is unchanged.
        assert_eq!(
            run("{ a.x = 1; let p = a; a.y = 2; q = p }")
                .unwrap()
                .to_string(),
            "{a = {x = 1; y = 2}; q = {x = 1}}"
        );
    }

    #[test]
    fn typed_equality_compares_the_static_fields() {
        let narrowed = "let q: {x: Int} = { x = 1; y = 2 }\n";
        booleans(&[
            (&format!("{narrowed}q == {{ x = 1 }}"), true),
            (&format!("{narrowed}q === {{ x = 1 }}"), false),
            (&format!("{narrowed}q != {{ x = 1 }}"), false),
            (&format!("{narrowed}q !== {{ x = 1 }}"), true),
            (&format!("{narrowed}q == {{ x = 2 }}"), false),
            (&format!("{narrowed}q === q"), true),
            (&format!("{narrowed}q == q"), true),
        ]);
        // Nested records compare over their static fields too.
        let nested = "let q: {a: {n: Int}} = { a = { n = 1; m = 2 }; b = 3 }\n";
        booleans(&[
            (&format!("{nested}q == {{ a = {{ n = 1 }} }}"), true),
            (&format!("{nested}q === {{ a = {{ n = 1 }} }}"), false),
            (&format!("{nested}q == {{ a = {{ n = 2 }} }}"), false),
            (&format!("{nested}q != {{ a = {{ n = 2 }} }}"), true),
            (&format!("{nested}q.a == {{ n = 1 }}"), true),
        ]);
        // A record typed `{}` equals every other.
        booleans(&[
            ("let u: {} = { x = 1 }\nu == {}", true),
            ("let u: {} = { x = 1 }\nu === {}", false),
            ("let u: {} = { x = 1 }\nu != {}", false),
            ("let e: {e: {}} = { e = { x = 1 } }\ne == { e = {} }", true),
        ]);
        // Floats compare as IEEE 754 does either way.
        booleans(&[
            ("let r = { f = 0.0 / 0.0 }\nr == r", false),
            ("let r = { f = 0.0 / 0.0 }\nr === r", false),
            ("let r = { f = 0.0 / 0.0 }\nr != r", true),
            ("{ f = -0.0 } == { f = 0.0 }", true),
            ("{ f = -0.0 } === { f = 0.0 }", true),
        ]);
        // Scalars are the same under both.
        booleans(&[
            ("1 === 1", true),
            ("1 !== 1", false),
            ("\"a\" === \"b\"", false),
            ("true !== false", true),
            ("1.5 === 1.5", true),
        ]);
        // Operands are evaluated left to right.
        assert_eq!(
            error("{ x = 1 // 0 } == { x = 9223372036854775807 + 1 }"),
            ("division by zero".to_owned(), Span::new(8, 10))
        );
        assert_eq!(
            error("{ x = 1 // 0 } === { x = 9223372036854775807 + 1 }"),
            ("division by zero".to_owned(), Span::new(8, 10))
        );
    }

    /// A record nested `depth` deep through the field named `inner`,
    /// ending in `leaf`.
    fn nested_record(inner: Name, depth: usize, leaf: Value) -> Value {
        let layout = Rc::new(Layout {
            names: Box::new([inner]),
            texts: Box::new([Box::from("inner")]),
        });
        let mut value = leaf;
        for _ in 0..depth {
            value = Value::Record(Rc::new(Record {
                layout: Rc::clone(&layout),
                values: Box::new([value]),
            }));
        }
        value
    }

    #[test]
    fn typed_equality_of_deep_records_does_not_recurse() {
        // Far deeper than a test thread's stack could take recursively: a
        // shape for each level, the last comparing the leaf as a value.
        const DEPTH: usize = 1_000_000;
        let mut names = crate::names::Names::new();
        let inner = names.intern("inner");
        let mut builder = crate::ir::IrBuilder::with_capacity(1);
        let first = builder.push_shape(crate::ir::Shape {
            fields: Box::default(),
        });
        let mut shape = first;
        for _ in 1..DEPTH {
            let next = builder.push_shape(crate::ir::Shape {
                fields: Box::default(),
            });
            builder.set_shape(
                shape,
                crate::ir::Shape {
                    fields: Box::new([(inner, Some(next))]),
                },
            );
            shape = next;
        }
        builder.set_shape(
            shape,
            crate::ir::Shape {
                fields: Box::new([(inner, None)]),
            },
        );
        let root = builder.alloc(IrKind::Empty, Span::new(0, 0));
        let ir = builder.finish(root, 0, Vec::new());
        let a = nested_record(inner, DEPTH, Value::Int(0));
        let b = nested_record(inner, DEPTH, Value::Int(0));
        let c = nested_record(inner, DEPTH, Value::Int(1));
        assert_eq!(record_eq_shaped(&ir, first, &a, &b), Some(true));
        assert_eq!(record_eq_shaped(&ir, first, &a, &c), Some(false));
        // Shallower records lack a field the shape has at the bottom.
        let shallow = nested_record(inner, DEPTH - 1, Value::Int(0));
        assert_eq!(record_eq_shaped(&ir, first, &a, &shallow), None);
    }

    #[test]
    fn a_record_lacking_a_field_of_its_shape_is_an_internal_error() {
        // `r == r` built by hand, with `r` holding a record without the
        // shape's `y`.
        let mut names = crate::names::Names::new();
        let (x, y) = (names.intern("x"), names.intern("y"));
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let shape = builder.push_shape(crate::ir::Shape {
            fields: Box::new([(y, None)]),
        });
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Record),
            Span::new(0, 1),
        );
        let eq = builder.alloc(
            IrKind::RecordEq(ShapedOperands {
                shape,
                lhs: load,
                rhs: load,
            }),
            Span::new(0, 6),
        );
        let ir = builder.finish(eq, 1, Vec::new());
        let record = Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: Box::new([x]),
                texts: Box::new([Box::from("x")]),
            }),
            values: Box::new([Value::Int(1)]),
        }));
        let error = eval_in(&ir, &mut [record]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a record lacks a field of its shape"
        );
        assert_eq!(error.span, Span::new(0, 6));
        // So is the empty record, where the shape has fields.
        let error = eval_in(&ir, &mut [Value::Empty]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a record lacks a field of its shape"
        );
    }

    #[test]
    fn an_update_without_its_name_text_is_an_internal_error() {
        let mut names = crate::names::Names::new();
        let x = names.intern("x");
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let nothing = builder.alloc(IrKind::Empty, Span::new(0, 1));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
        let update = builder.alloc(
            IrKind::UpdateField {
                record: nothing,
                name: x,
                value: one,
            },
            Span::new(0, 5),
        );
        let ir = builder.finish(update, 0, Vec::new());
        let error = eval(&ir).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: an update's name has no text"
        );
        assert_eq!(error.span, Span::new(0, 5));
    }

    #[test]
    fn a_missing_field_is_an_internal_error() {
        // `r.y` built by hand, with `r` holding a record without `y`.
        let mut names = crate::names::Names::new();
        let (x, y) = (names.intern("x"), names.intern("y"));
        let mut builder = crate::ir::IrBuilder::with_capacity(2);
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Record),
            Span::new(0, 1),
        );
        let field = builder.alloc(
            IrKind::Field {
                record: load,
                name: y,
                kind: ValueKind::Int,
            },
            Span::new(0, 3),
        );
        let ir = builder.finish(field, 1, Vec::new());
        let record = |value: Value| {
            Value::Record(Rc::new(Record {
                layout: Rc::new(crate::value::Layout {
                    names: Box::new([x]),
                    texts: Box::new([Box::from("x")]),
                }),
                values: Box::new([value]),
            }))
        };
        let error = eval_in(&ir, &mut [record(Value::Int(1))]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a record lacks a field the checker found"
        );
        assert_eq!(error.span, Span::new(0, 3));
        let error = eval_in(&ir, &mut [Value::Empty]).unwrap_err();
        assert_eq!(error.message, "internal error: a field of the empty record");
    }
}
