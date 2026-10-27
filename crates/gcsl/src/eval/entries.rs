//! Evaluating `record`: a record of a field for each entry of a list, named
//! as the program runs (see [Records of any
//! names](crate::check::entries#records-of-any-names)).
//!
//! # Records of any names
//!
//! Each entry's key is interned in the names of the run's owner, a
//! workspace or a session, which the run is lent ([`Shared`](super::Shared)),
//! so that a field it names is the name the program's own code gives that
//! text, and a record it makes equals one written with the same fields. A
//! key two entries give is the runtime error "the key `x` is in the
//! entries twice", at the call. The fields are in the entries' order.

use std::rc::Rc;

use crate::diagnostic::Diagnostic;
use crate::ir::{IrKind, IrNode};
use crate::names::Name;
use crate::value::{Layout, Record, Value};

use super::{Evaluator, Flow, internal, mistyped};

impl Evaluator<'_> {
    /// Evaluates the `RecordOf` `node`: the record of its entries.
    pub(super) fn record_of(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::RecordOf {
            entries,
            key,
            value,
        } = node.kind
        else {
            return mistyped(node, "a node that is not a `record` made a record");
        };
        let entries = self.list(entries)?;
        let mut names: Vec<Name> = Vec::with_capacity(entries.values().len());
        let mut texts: Vec<Box<str>> = Vec::with_capacity(names.capacity());
        let mut values: Vec<Value> = Vec::with_capacity(names.capacity());
        for entry in entries.values() {
            let Value::Record(entry) = entry else {
                return mistyped(node, "an entry of `record` that is no record");
            };
            let (Some(Value::Str(text)), Some(given)) = (entry.get(key), entry.get(value)) else {
                return mistyped(node, "an entry of `record` with no `key` or `value`");
            };
            if texts.iter().any(|seen| **seen == **text) {
                return Err(Diagnostic::error(
                    format!("the key `{text}` is in the entries twice"),
                    node.span,
                )
                .with_note("a record has a field of each name once")
                .into());
            }
            names.push(self.name_of(text, node)?);
            texts.push(Box::from(&**text));
            values.push(given.clone());
        }
        if values.is_empty() {
            return Ok(Value::Empty);
        }
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: names.into_boxed_slice(),
                texts: texts.into_boxed_slice(),
            }),
            values: values.into_boxed_slice(),
        })))
    }

    /// The name of `text` in the names of the run's owner, interned there
    /// if it is new: an internal error, at `node`, in a run with no owner's
    /// names to intern it in.
    pub(super) fn name_of(&self, text: &str, node: &IrNode) -> Flow<Name> {
        let Some(names) = self.shared.names else {
            return Err(internal(
                "a name made in a run with no names to make it in",
                node.span,
            )
            .into());
        };
        let mut names = names
            .try_borrow_mut()
            .map_err(|_| internal("the run's names are already borrowed", node.span))?;
        Ok(names.intern(text))
    }
}

#[cfg(test)]
mod tests;
