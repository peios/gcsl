//! The built-in functions of secrets, enums and partial records.

use std::rc::Rc;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{Quantity, Secret, Value};

use super::{Flow, mistyped};

/// The call `node` of `builtin`, `secret`, `secret_value` or
/// `secret_reveal`, with the argument `value` and, for `secret`, the
/// identity `identity` (see [Secrets](crate::eval::secrets#secrets)):
/// `secret` of a secret is its value as a secret of the new identity, not
/// revealed; `secret_reveal` is the secret revealed, of its identity; and
/// each of the other two of a plain value, a plain `T` where a `Secret<T>`
/// is expected, is the value.
pub(super) fn secret_builtin(
    builtin: Builtin,
    value: Value,
    identity: Option<Value>,
    node: &IrNode,
) -> Flow<Value> {
    if builtin == Builtin::SecretReveal {
        return match value {
            Value::Secret(secret) => Ok(Value::Secret(Rc::new(Secret {
                value: secret.value.clone(),
                identity: Rc::clone(&secret.identity),
                revealed: true,
            }))),
            value => Ok(value),
        };
    }
    let value = match value {
        Value::Secret(secret) => secret.value.clone(),
        value => value,
    };
    match (builtin, identity) {
        (Builtin::Secret, Some(Value::Str(identity))) => Ok(Value::Secret(Rc::new(Secret {
            value,
            identity,
            revealed: false,
        }))),
        (Builtin::SecretValue, None) => Ok(value),
        _ => mistyped(node, "a secret's built-in with an argument of another kind"),
    }
}

/// The call `node` of `builtin`, `variant_matches`, `variant_payload` or
/// `partial_matches`, with the arguments `first` and `second` (see the
/// checker's "Enums" and "Partial records"): `variant_payload` of a value
/// of another variant than its kind is a runtime error, "the value is
/// `Tool::Rust`, not `Tool::C`".
pub(super) fn enum_builtin(
    builtin: Builtin,
    (first, second): (&Value, &Value),
    node: &IrNode,
) -> Flow<Value> {
    match (builtin, first, second) {
        (Builtin::VariantMatches, Value::Tag(kind), Value::Variant(value)) => {
            Ok(Value::Bool(value.tag.key == kind.key))
        }
        (Builtin::VariantPayload, Value::Tag(kind), Value::Variant(value)) => {
            if value.tag.key == kind.key {
                Ok(value.payload.clone())
            } else {
                Err(Diagnostic::error(
                    format!("the value is `{}`, not `{}`", value.tag.shown, kind.shown),
                    node.span,
                )
                .into())
            }
        }
        (Builtin::PartialMatches, partial, full) => Ok(Value::Bool(partial_matches(partial, full))),
        _ => mistyped(
            node,
            "an enum's built-in called with an argument of another kind",
        ),
    }
}

/// Whether every field `partial`, a record, holds equals the field of its
/// name of `full`, a record, as `===` compares them: what
/// `partial.matches(full)` is (see the checker's "Partial records"). A
/// field `full` lacks matches nothing.
pub(super) fn partial_matches(partial: &Value, full: &Value) -> bool {
    let Value::Record(partial) = partial else {
        // `{}` holds no field, so matches every record.
        return true;
    };
    let Value::Record(full) = full else {
        return false;
    };
    partial
        .layout
        .names
        .iter()
        .zip(&partial.values)
        .all(|(name, value)| {
            full.layout
                .names
                .iter()
                .position(|own| own == name)
                .and_then(|at| full.values.get(at))
                .is_some_and(|own| own == value)
        })
}

/// What `value` is, after its indefinite article, as a failed test's label
/// says it: "an Int", "a Float", "a Duration", "a Size", "a Bool", "a
/// String", "a record", "a list", "a function" or "a future".
pub(super) const fn described(value: &Value) -> &'static str {
    match value {
        Value::Int(_) => "an Int",
        Value::Float(_) => "a Float",
        Value::Quantity(Quantity::Duration, _) => "a Duration",
        Value::Quantity(Quantity::Size, _) => "a Size",
        Value::Bool(_) => "a Bool",
        Value::Str(_) => "a String",
        Value::Empty | Value::Record(_) => "a record",
        Value::List(_) => "a list",
        Value::Fn(_) | Value::Builtin(_) => "a function",
        Value::Tag(_) => "a variant",
        Value::Variant(_) => "a value of an enum",
        Value::Secret(_) => "a secret",
        Value::Future(_) => "a future",
    }
}
