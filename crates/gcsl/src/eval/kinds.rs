//! Taking values of the kind a node says out of the walkers: each kind's
//! extractor, and the casts of a value to a kind.

use std::rc::Rc;

use crate::ir::{IrKind, IrNode};
use crate::value::{Quantity, Value, ValueKind};

use super::{
    Callee, DEFERRED_BOOL, DEFERRED_FLOAT, DEFERRED_INT, DEFERRED_RECORD, DEFERRED_STRING, Flow,
    mistyped,
};

/// Extracts a value of one kind from a slot or capture, for the node
/// reading it: [`as_int`], [`as_float`], and so on.
pub(super) type Extract<T> = fn(&Value, &IrNode) -> Flow<T>;

/// How a typed walker takes values of its kind, `T`, from the nodes whose
/// values may be of any kind, which it evaluates through
/// [`Evaluator::shared_as`](super::Evaluator::shared_as).
pub(super) struct Taken<T> {
    /// From a slot or a capture.
    pub(super) borrowed: Extract<T>,
    /// From an element read by an index, an awaited or narrowed value, a
    /// field or a call's result.
    pub(super) owned: fn(Value) -> Option<T>,
    /// The internal error of meeting such a node whose value may be a
    /// future, which only the general walker evaluates.
    pub(super) deferred: &'static str,
}

/// How the `Int` walker takes its values.
pub(super) const INT: Taken<i64> = Taken {
    borrowed: as_int,
    owned: into_int,
    deferred: DEFERRED_INT,
};

/// How the `Int` walker takes a duration's amount, for an integer operator
/// computing on durations.
pub(super) const DURATION_AMOUNT: Taken<i64> = Taken {
    borrowed: as_duration,
    owned: Value::into_duration,
    deferred: DEFERRED_INT,
};

/// How the `Int` walker takes a size's amount, as [`DURATION_AMOUNT`] a
/// duration's.
pub(super) const SIZE_AMOUNT: Taken<i64> = Taken {
    borrowed: as_size,
    owned: Value::into_size,
    deferred: DEFERRED_INT,
};

/// How the `Int` walker takes the value of a node of any kind, of kind
/// `kind`: an `Int`, or a duration's or a size's amount.
pub(super) const fn amount_taken(kind: ValueKind) -> &'static Taken<i64> {
    match kind {
        ValueKind::Quantity(Quantity::Duration) => &DURATION_AMOUNT,
        ValueKind::Quantity(Quantity::Size) => &SIZE_AMOUNT,
        _ => &INT,
    }
}

/// How the `Float` walker takes its values.
pub(super) const FLOAT: Taken<f64> = Taken {
    borrowed: as_float,
    owned: Value::into_float,
    deferred: DEFERRED_FLOAT,
};

/// How the `Bool` walker takes its values.
pub(super) const BOOL: Taken<bool> = Taken {
    borrowed: as_bool,
    owned: Value::into_bool,
    deferred: DEFERRED_BOOL,
};

/// How the `String` walker takes its values.
pub(super) const STRING: Taken<Rc<str>> = Taken {
    borrowed: as_string,
    owned: Value::into_string,
    deferred: DEFERRED_STRING,
};

/// How the record walker takes its values.
pub(super) const RECORD: Taken<Value> = Taken {
    borrowed: as_record,
    owned: into_record,
    deferred: DEFERRED_RECORD,
};

/// Whether `node`, a load, a capture, a field access, a call, an index, an
/// await or an `as`, which a typed walker reads through a helper, is of
/// kind [`ValueKind::Mixed`], which only the general walker evaluates: the
/// one check the typed walkers make of a union's node, in the helpers, so
/// that a node of any other kind costs them one comparison.
pub(super) const fn mixed(node: &IrNode) -> bool {
    matches!(
        node.kind,
        IrKind::Load(_, ValueKind::Mixed)
            | IrKind::LoadCapture(_, ValueKind::Mixed)
            | IrKind::Top(_, ValueKind::Mixed)
            | IrKind::Field {
                kind: ValueKind::Mixed,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Mixed,
                ..
            }
            | IrKind::Index {
                kind: ValueKind::Mixed,
                ..
            }
            | IrKind::Await {
                kind: ValueKind::Mixed,
                ..
            }
            | IrKind::As {
                kind: ValueKind::Mixed,
                ..
            }
            | IrKind::Catch {
                kind: ValueKind::Mixed,
                ..
            }
    )
}

/// Whether `value` is of kind `kind`, where a node of that kind yields it:
/// of that very kind, or, for [`ValueKind::Mixed`], of any kind but a
/// future's.
pub(super) fn of_kind(value: &Value, kind: ValueKind) -> bool {
    match kind {
        ValueKind::Mixed => value.kind() != ValueKind::Deferred,
        kind => value.kind() == kind,
    }
}

/// The `Int` in `value`, read by the load or capture read `node`.
pub(super) fn as_int(value: &Value, node: &IrNode) -> Flow<i64> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => mistyped(node, "Int load from a slot holding another type"),
    }
}

/// The `Float` in `value`, read by the load or capture read `node`.
pub(super) fn as_float(value: &Value, node: &IrNode) -> Flow<f64> {
    match value {
        Value::Float(value) => Ok(*value),
        _ => mistyped(node, "Float load from a slot holding another type"),
    }
}

/// The `Bool` in `value`, read by the load or capture read `node`.
pub(super) fn as_bool(value: &Value, node: &IrNode) -> Flow<bool> {
    match value {
        Value::Bool(value) => Ok(*value),
        _ => mistyped(node, "Bool load from a slot holding another type"),
    }
}

/// A new reference to the `String` in `value`, read by the load or capture
/// read `node`: not a copy.
pub(super) fn as_string(value: &Value, node: &IrNode) -> Flow<Rc<str>> {
    match value {
        Value::Str(text) => Ok(Rc::clone(text)),
        _ => mistyped(node, "String load from a slot holding another type"),
    }
}

/// A new reference to the record in `value`, read by the load or capture
/// read `node`: not a copy.
pub(super) fn as_record(value: &Value, node: &IrNode) -> Flow<Value> {
    match value {
        Value::Empty | Value::Record(_) => Ok(value.clone()),
        _ => mistyped(node, "record load from a slot holding another type"),
    }
}

/// The amount of the duration in `value`, read by the load or capture read
/// `node`.
pub(super) fn as_duration(value: &Value, node: &IrNode) -> Flow<i64> {
    match value {
        &Value::Quantity(Quantity::Duration, amount) => Ok(amount),
        _ => mistyped(node, "Duration load from a slot holding another type"),
    }
}

/// The amount of the size in `value`, read by the load or capture read
/// `node`.
pub(super) fn as_size(value: &Value, node: &IrNode) -> Flow<i64> {
    match value {
        &Value::Quantity(Quantity::Size, amount) => Ok(amount),
        _ => mistyped(node, "Size load from a slot holding another type"),
    }
}

/// `value`'s `Int` if it is one: what the Int walker extracts from an
/// element read by an index, an awaited value or a field, as
/// [`into_record`] is the record walker's.
pub(super) fn into_int(value: Value) -> Option<i64> {
    value.into_int()
}

/// `value` if it is a record: what the record walker extracts from an
/// element read by an index, an awaited value or a field.
pub(super) fn into_record(value: Value) -> Option<Value> {
    match value {
        Value::Empty | Value::Record(_) => Some(value),
        _ => None,
    }
}

/// What a call of `value`, read by the load or capture read `node`, calls:
/// a new reference to the closure it is, or the built-in function.
pub(super) fn as_function(value: &Value, node: &IrNode) -> Flow<Callee> {
    match value {
        Value::Fn(closure) => Ok(Callee::Closure(Rc::clone(closure))),
        &Value::Builtin(builtin) => Ok(Callee::Builtin(builtin)),
        Value::Tag(tag) => Ok(Callee::Tag(Rc::clone(tag))),
        _ => mistyped(node, "function load from a slot holding another type"),
    }
}

/// What a call of `value` calls, if it is a function: what the function
/// walker extracts from an element read by an index, an awaited value, a
/// field or a call's result.
pub(super) fn into_callee(value: Value) -> Option<Callee> {
    match value {
        Value::Fn(closure) => Some(Callee::Closure(closure)),
        Value::Builtin(builtin) => Some(Callee::Builtin(builtin)),
        Value::Tag(tag) => Some(Callee::Tag(tag)),
        _ => None,
    }
}

/// `x` truncated toward zero, the value of `int(x)`; `None` when that is not
/// an `Int`: `x` is NaN, infinite, or beyond the range of `i64`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the truncated value is checked to be within the range of i64 first"
)]
pub(super) fn float_to_int(x: f64) -> Option<i64> {
    // -2^63 and 2^63, both exactly representable.
    const LOW: f64 = -9_223_372_036_854_775_808.0;
    const HIGH: f64 = 9_223_372_036_854_775_808.0;
    let truncated = x.trunc();
    (LOW..HIGH).contains(&truncated).then_some(truncated as i64)
}
