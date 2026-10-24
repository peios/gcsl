//! Evaluating updates: rebinding a field or an element of a variable deep in
//! its value.
//!
//! # Updates
//!
//! An `Update`, a dotted `let` or `:=`, is evaluated by a helper,
//! `Evaluator::update`, which the walkers only call, in the order every
//! dotted update has: its value first, once, through `value`; then the
//! record or list it updates, the variable's, read, and awaited if the
//! checker marked it; and then `Evaluator::walk_update` walks the path down
//! from it, once, without recursing: each step selects a field, or an
//! element at the position its next `Int` child evaluates to, there and
//! then, left to right, and reads it, awaits it, charged to the nesting
//! budget as an await just below the update, or, where a `let`'s path goes
//! beyond the static type, starts from `{}`, holding each record or list
//! it passes through in a vector; the last step selects the leaf. A
//! position outside its list, a negative one included, stops the walk with
//! "index out of range", at the path up to and including its segment,
//! labelled as an index's is, before anything is stored, so the variable is
//! left as it was. Then the new value is built back up, from the leaf
//! outwards: the leaf set to the value, then each record or list passed
//! through with what its step selected replaced by the one built below it.
//! The update's value is that; the `Store` around it writes it. A value
//! already read is held while what follows it runs, so a position that
//! rebinds the variable does not change what the walk updates, and the
//! store then overwrites the rebind.
//!
//! Nothing read is changed: another variable or closure holding an old
//! record or list still sees it as it was. A record is set as an
//! `UpdateField` sets it. A list is copied, in one allocation sized to its
//! length, sharing every element but the one replaced, so updating each
//! element of a list in a loop takes time quadratic in its length, for now:
//! updating in place, when the variable is the only holder of what it
//! updates, is the optimisation that evaluating the value before the
//! variable is read allows. A path of one step allocates nothing for the
//! walk.

use std::rc::Rc;

use crate::ir::{IrId, IrKind, IrNode, StepAt, StepMode, UpdatePathId, UpdateStep};
use crate::names::Name;
use crate::value::{Layout, Record, Value};

use super::{Evaluator, Flow, Selected, mistyped, out_of_range, selected, with_element};

impl Evaluator<'_> {
    /// Evaluates the `UpdateField` `node`, setting its field of its record
    /// to its value: a new record, the old one unchanged. See the module
    /// documentation.
    pub(super) fn update_field(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::UpdateField {
            record,
            name,
            value,
        } = node.kind
        else {
            return mistyped(node, "a node that is not a field's update updated");
        };
        let old = self.record(record)?;
        let value = self.value(value)?;
        self.with_field(old, name, value, node)
    }

    /// Evaluates the `Update` `node` (see the module documentation): its
    /// value, first, then the record or list it updates, and then the walk
    /// of its path (see [`Evaluator::walk_update`]). Apart from the walk,
    /// so that the recursion through the value holds nothing of it.
    pub(super) fn update(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::Update {
            parts, path, depth, ..
        } = node.kind
        else {
            return mistyped(node, "a node that is not an update updated");
        };
        let ir = self.ir;
        let parts = ir.items(parts);
        let (Some(&value), Some(&root)) = (parts.first(), parts.get(1)) else {
            return mistyped(node, "an update without its value or its record");
        };
        let value = self.value(value)?;
        let root = self.value(root)?;
        let positions = parts.get(2..).unwrap_or_default();
        self.walk_update((root, value), (positions, path, depth), node)
    }

    /// Walks the path `path` of the `Update` `node`, at depth `depth` in
    /// this frame's body, down from `root`, the record or list it updates,
    /// one step at a time: each step selects a field, or an element at the
    /// position the next of `positions` evaluates to, there and then, and
    /// reads it, awaits it, as an await at the step's path, or starts from
    /// `{}`, holding each record or list it passes through. The last step
    /// selects the leaf, and the new value is built back up from it: the
    /// leaf set to `value`, then each record or list passed through with
    /// what its step selected replaced by the one built below it. Nothing
    /// read is changed. A position outside its list stops the walk with
    /// "index out of range", at the path up to its segment.
    pub(super) fn walk_update(
        &mut self,
        (root, value): (Value, Value),
        (positions, path, depth): (&[IrId], UpdatePathId, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let ir = self.ir;
        let Some((leaf, steps)) = ir.update_path(path).and_then(<[UpdateStep]>::split_last) else {
            return mistyped(node, "an update of a path its IR lacks");
        };
        let mut positions = positions.iter().copied();
        let mut passed = Vec::with_capacity(steps.len());
        let mut current = root;
        for step in steps {
            let at = self.select(&current, step, &mut positions, node)?;
            let inner = match step.mode {
                StepMode::Read => selected(&current, at, node)?,
                StepMode::Await => {
                    // A failure of the await is reported at the step's path.
                    let site = IrNode {
                        kind: node.kind,
                        span: step.span,
                    };
                    let read = selected(&current, at, node)?;
                    self.await_fully(read, depth.saturating_add(1), 0, &site)?
                }
                StepMode::Create => Value::Empty,
                StepMode::Set => return mistyped(node, "an update sets a value before its leaf"),
            };
            passed.push((current, at));
            current = inner;
        }
        if leaf.mode != StepMode::Set {
            return mistyped(node, "an update's last step does not set its leaf");
        }
        let at = self.select(&current, leaf, &mut positions, node)?;
        let mut new = self.replace(current, at, value, node)?;
        while let Some((container, at)) = passed.pop() {
            new = self.replace(container, at, new, node)?;
        }
        Ok(new)
    }

    /// What `step` of the `Update` `node` selects in `container`, the
    /// record or list its walk is at: a field by name, or, evaluating the
    /// next of `positions`, an element, whose position must lie in the
    /// list, a negative one never does, or the update fails with "index out
    /// of range" at the step's path.
    pub(super) fn select(
        &mut self,
        container: &Value,
        step: &UpdateStep,
        positions: &mut impl Iterator<Item = IrId>,
        node: &IrNode,
    ) -> Flow<Selected> {
        let StepAt::Field(name) = step.at else {
            let Some(position) = positions.next() else {
                return mistyped(node, "an update lacks the position of an element");
            };
            let at = self.int(position)?;
            let Value::List(list) = container else {
                return mistyped(node, "an update selects an element of what is not a list");
            };
            return match usize::try_from(at).ok().filter(|&index| index < list.len()) {
                Some(index) => Ok(Selected::Element(index)),
                None => Err(out_of_range(step.span, at, list.len()).into()),
            };
        };
        Ok(Selected::Field(name))
    }

    /// `container`, a record or list an update's walk passed through, with
    /// what `at` selects in it replaced by `value`, or added for a field
    /// the record lacks, for the `Update` `node`: a new record or list,
    /// `container` unchanged.
    pub(super) fn replace(
        &self,
        container: Value,
        at: Selected,
        value: Value,
        node: &IrNode,
    ) -> Flow<Value> {
        match (container, at) {
            (container, Selected::Field(name)) => self.with_field(container, name, value, node),
            (Value::List(list), Selected::Element(index)) => {
                Ok(Value::List(with_element(&list, index, value)))
            }
            _ => mistyped(node, "an update sets an element of what is not a list"),
        }
    }

    /// `old`, a record, with field `name` set to `value`, for the update
    /// `node`: a new record, the old one unchanged. See the module
    /// documentation.
    pub(super) fn with_field(
        &self,
        old: Value,
        name: Name,
        value: Value,
        node: &IrNode,
    ) -> Flow<Value> {
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
            Value::Int(_)
            | Value::Float(_)
            | Value::Quantity(..)
            | Value::Bool(_)
            | Value::Str(_)
            | Value::List(_)
            | Value::Fn(_)
            | Value::Builtin(_)
            | Value::Tag(_)
            | Value::Variant(_)
            | Value::Secret(_)
            | Value::File(_)
            | Value::Dir(_)
            | Value::Future(_) => {
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
}
