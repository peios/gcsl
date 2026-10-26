//! The built-ins that compare a list's elements: `unique` and `contains`.
//!
//! # Comparing elements
//!
//! `xs.unique()`, the built-in `list_unique`, is `xs` with every element
//! that equals an earlier one left out, the first of each kept where it
//! was; `xs.contains(x)`, the built-in `list_contains`, whether an element
//! equals `x`. Equality is total equality, `===`'s, [`Value`]'s own: of a
//! record, every field it has, by name, whatever its type names; of a
//! secret, its identity (see [Secrets](crate::eval::secrets#secrets)), so
//! that the answer tells nothing of a value and is no secret. Typed
//! equality, `==`'s, compares through a static type, which a generic
//! built-in does not have. Elements are first grouped by a hash of them,
//! the memoisation's ([`hash_value`]), so that `unique` compares each with
//! the few of its hash, not with every other. A generic method cannot
//! require its elements to have an equality, so what has none, which the
//! checker refuses to compare, is found as it is met: an element, or the
//! `x` looked for, that holds a function or a future, in a field, an
//! element or a payload at any depth, is "cannot compare functions:
//! `unique` compares the elements of its list", or futures.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{List, Value};

use super::{Flow, mistyped};
use super::memo::hash_value;

/// How deep [`hash_value`] looks into an element: past that, elements of
/// one hash are told apart by equality alone.
const HASH_DEPTH: usize = 8;

/// Applies `builtin`, `contains`, `list_unique` or `list_contains`, for the
/// call `node`, to `first` and `second`, if it takes two: whether a string
/// holds another, byte-wise, or as [`compare_elements`] says.
pub(super) fn element_builtin(
    builtin: Builtin,
    first: &Value,
    second: Option<&Value>,
    node: &IrNode,
) -> Flow<Value> {
    match (builtin, first, second) {
        (Builtin::Contains, Value::Str(s), Some(Value::Str(part))) => {
            Ok(Value::Bool(s.contains(&**part)))
        }
        (Builtin::ListUnique | Builtin::ListContains, Value::List(xs), x) => {
            compare_elements(builtin, xs, x, node)
        }
        _ => mistyped(
            node,
            "a built-in of elements called with arguments of other kinds",
        ),
    }
}

/// Applies `builtin`, `list_unique` or `list_contains`, for the call
/// `node`, to the list `xs` and, for `list_contains`, the value `x` (see
/// [Comparing elements](#comparing-elements)).
fn compare_elements(builtin: Builtin, xs: &List, x: Option<&Value>, node: &IrNode) -> Flow<Value> {
    let values = xs.values();
    if let Some(what) = values.iter().chain(x).find_map(incomparable) {
        let name = if builtin == Builtin::ListUnique {
            "unique"
        } else {
            "contains"
        };
        let message = format!("cannot compare {what}: `{name}` compares the elements of its list");
        return Err(Diagnostic::error(message, node.span).into());
    }
    if let Some(x) = x {
        return Ok(Value::Bool(values.iter().any(|value| value == x)));
    }
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut kept: Vec<Value> = Vec::with_capacity(values.len());
    for value in values {
        let mut state = DefaultHasher::new();
        hash_value(value, &mut state, HASH_DEPTH);
        let alike = seen.entry(state.finish()).or_default();
        if alike.iter().any(|&at| kept.get(at) == Some(value)) {
            continue;
        }
        alike.push(kept.len());
        kept.push(value.clone());
    }
    Ok(Value::List(List::from_vec(kept)))
}

/// What `value` holds that has no equality, in a field, an element or a
/// payload at any depth: `"functions"` for a closure or a built-in,
/// `"futures"` for a future; `None` if it holds neither. A variant as a
/// value compares as data.
fn incomparable(value: &Value) -> Option<&'static str> {
    let mut pending: Vec<&Value> = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Fn(_) | Value::Builtin(_) => return Some("functions"),
            Value::Future(_) => return Some("futures"),
            Value::Record(record) => pending.extend(record.values.iter()),
            Value::List(list) => pending.extend(list.values().iter()),
            Value::Variant(variant) => pending.push(&variant.payload),
            _ => {}
        }
    }
    None
}
