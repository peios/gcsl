//! Testing a value against a type at run time, for `is` and `as`, and the
//! labels of a failed test.
//!
//! # Unions
//!
//! A node of a union of kinds, of [`crate::value::ValueKind::Mixed`], is
//! evaluated by the general walker only, as a node that may be a future is:
//! its value is whatever its member's kind's is, built as a [`Value`]. The
//! typed walkers read loads, captures, field accesses, indexes, calls,
//! awaits and `as`es through helpers, each of which treats a `Mixed` node
//! as mistyped (`mixed`), so the one check costs a node of any other kind
//! one comparison, and no typed walker grows. An `await` of kind `Mixed`
//! yields a present value of any kind.
//!
//! An `Is` evaluates its value through `value`, writes it to its slot if it
//! has one, for the narrowed bindings of the branches of the `if` it is the
//! condition of, and runs its test: the plan the checker made, which looks
//! at the value's kind and, as far as the plan says, at its fields and
//! elements, by name and in order (`passes`), recursing once per level of
//! the plan, which is no deeper than the type tested for. An `As` runs the
//! test the same way and yields the value if it passes, and otherwise
//! fails with "`x` is not an Int", at the `as`, labelled with what the value
//! is, and for a record or a list that fails inside, one level down: "it
//! is a String", "it is a record, and its field `a` is a String", "it is a
//! record without a field `a`" or "it is a list, and its element 3 is a
//! String" (`failure`). Nothing is changed, so a failed `as` leaves
//! everything as it was but what the input ran before it.
//!
//! A deep await of a union applies the lazy shape of each of its members
//! that has one, in turn, *tolerantly* (see [`Ir::lazy_union`]): which
//! member the value is of is not known, so a field a member's shape lists
//! that the record lacks, and a shape of another kind than the value it
//! meets, are passed over, at every level below, where for any other
//! shape they are internal errors. Awaiting a value that is present does
//! nothing, so applying several shapes awaits nothing twice; but a future
//! in a field that the value's own type hides, which another member's
//! shape lists, is awaited too.

use super::{ValueTest, Value, Ir, FieldName, described, Name, Record};

/// Whether `value` passes the test `test` (see [`ValueTest`]): by its kind,
/// and as far as the test looks, its fields and elements.
///
/// Recurses once per level of record, list or union nesting in the test,
/// which is no deeper than the type it tests for.
pub(super) fn passes(test: &ValueTest, value: &Value) -> bool {
    match test {
        ValueTest::Pass => true,
        ValueTest::Kind(kind) => value.kind() == *kind,
        ValueTest::Record(fields) => match value {
            Value::Record(record) => fields
                .iter()
                .all(|(name, field)| record.get(*name).is_some_and(|value| passes(field, value))),
            _ => false,
        },
        ValueTest::RecordRest {
            fields,
            named,
            rest,
        } => match value {
            Value::Record(record) => {
                fields.iter().all(|(name, field)| {
                    record.get(*name).is_some_and(|value| passes(field, value))
                }) && other_fields(record, named).all(|(_, _, value)| passes(rest, value))
            }
            // The empty record has no field, so no other one: it passes if
            // there is no named field to test.
            Value::Empty => fields.is_empty(),
            _ => false,
        },
        ValueTest::List(element) => match value {
            Value::List(list) => list.values().iter().all(|value| passes(element, value)),
            _ => false,
        },
        ValueTest::Any(alternatives) => alternatives
            .iter()
            .any(|alternative| passes(alternative, value)),
        ValueTest::Enum(key) => matches!(value, Value::Variant(variant) if variant.tag.of == *key),
    }
}

/// The label on the failure of `value` to pass `test`, an `as`'s, saying
/// what it is: "it is a String"; and for a record or a list that a test of
/// its fields or elements fails inside, one level down, "it is a record,
/// and its field `a` is a String", "it is a record without a field `a`" or
/// "it is a list, and its element 3 is a String". A union's test fails
/// inside as the first of its alternatives that looks inside a value of
/// this one's kind does. Field names are taken from `ir`.
pub(super) fn failure(ir: &Ir, test: &ValueTest, value: &Value) -> String {
    let inside = match test {
        ValueTest::Any(alternatives) => alternatives.iter().find(|alternative| {
            matches!(
                (alternative, value),
                (
                    ValueTest::Record(_) | ValueTest::RecordRest { .. },
                    Value::Record(_)
                ) | (ValueTest::List(_), Value::List(_))
            )
        }),
        _ => Some(test),
    };
    match (inside, value) {
        (Some(ValueTest::Record(fields)), Value::Record(record)) => {
            if let Some(label) = named_failure(ir, fields, record) {
                return label;
            }
        }
        (
            Some(ValueTest::RecordRest {
                fields,
                named,
                rest,
            }),
            Value::Record(record),
        ) => {
            if let Some(label) = named_failure(ir, fields, record) {
                return label;
            }
            if let Some((_, text, found)) =
                other_fields(record, named).find(|(_, _, found)| !passes(rest, found))
            {
                return format!(
                    "it is a record, and its field `{}` is {}",
                    FieldName(text),
                    described(found)
                );
            }
        }
        (Some(ValueTest::List(element)), Value::List(list)) => {
            if let Some((index, found)) = list
                .values()
                .iter()
                .enumerate()
                .find(|(_, found)| !passes(element, found))
            {
                return format!(
                    "it is a list, and its element {index} is {}",
                    described(found)
                );
            }
        }
        _ => {}
    }
    format!("it is {}", described(value))
}

/// The label on the failure of `record` to pass the tests of its named
/// fields `fields`, a [`ValueTest::Record`]'s or a
/// [`ValueTest::RecordRest`]'s, as [`failure`] says it, or `None` if it
/// passes them.
pub(super) fn named_failure(
    ir: &Ir,
    fields: &[(Name, ValueTest)],
    record: &Record,
) -> Option<String> {
    for (name, field) in fields {
        let shown = FieldName(ir.name_text(*name).unwrap_or("?"));
        match record.get(*name) {
            None => return Some(format!("it is a record without a field `{shown}`")),
            Some(found) if !passes(field, found) => {
                return Some(format!(
                    "it is a record, and its field `{shown}` is {}",
                    described(found)
                ));
            }
            Some(_) => {}
        }
    }
    None
}

/// The fields of `record` that `named`, sorted, does not name, each as its
/// name, text and value, in the record's order: those a record type's
/// rest-field covers, which a [`ValueTest::RecordRest`] tests, typed
/// equality compares and a deep await awaits as the rest-field says.
pub(super) fn other_fields<'r, 'n>(
    record: &'r Record,
    named: &'n [Name],
) -> impl Iterator<Item = (Name, &'r str, &'r Value)> + use<'r, 'n> {
    record
        .layout
        .names
        .iter()
        .zip(&record.layout.texts)
        .zip(&record.values)
        .filter(|((name, _), _)| named.binary_search(name).is_err())
        .map(|((&name, text), value)| (name, &**text, value))
}

/// The record `value` is, if it is one with fields: `None` for the empty
/// record and any other value.
pub(super) fn with_fields(value: &Value) -> Option<&Record> {
    match value {
        Value::Record(record) => Some(record),
        _ => None,
    }
}
