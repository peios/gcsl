//! Evaluating narrowing: the tests of `is` and `as`, and the narrowed values
//! they give.

use crate::check::Slot;
use crate::diagnostic::Diagnostic;
use crate::ir::{IrKind, IrNode, Operands};
use crate::value::Value;

use super::{Evaluator, Flow, MIXED, internal, mistyped};
use super::kinds::{mixed, of_kind};
use super::unions::{failure, passes};

impl Evaluator<'_> {
    /// Evaluates the `As` `node` (see the module documentation) as a value,
    /// which must be of the kind the node states, or, for
    /// [`ValueKind::Mixed`](super::ValueKind::Mixed), of any kind but a future's.
    pub(super) fn narrow_node(&mut self, node: &IrNode) -> Flow<Value> {
        let value = self.narrowed(node)?;
        match node.kind {
            IrKind::As { kind, .. } if of_kind(&value, kind) => Ok(value),
            _ => mistyped(
                node,
                "an `as` produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `As` `node` as a value that `extract` accepts: one of
    /// the kind the node states.
    pub(super) fn narrow_as<T>(
        &mut self,
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.narrowed(node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "an `as` produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `As` `node`: its value, through the general walker,
    /// which it yields if it passes the node's test, and otherwise "`x` is
    /// not an Int", at the node, labelled with what the value is (see
    /// [`failure`]).
    pub(super) fn narrowed(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::As { value, test, .. } = node.kind else {
            return mistyped(node, "a node that is not an `as` narrowed");
        };
        let value = self.value(value)?;
        let ir = self.ir;
        let Some(code) = ir.test(test) else {
            return mistyped(node, "an `as` of a test its IR lacks");
        };
        if passes(&code.test, &value) {
            return Ok(value);
        }
        Err(Diagnostic::error(
            format!("{} is not {}", code.subject, code.target),
            node.span,
        )
        .with_label(failure(ir, &code.test, &value))
        .into())
    }

    /// Evaluates the `Is` `node`: its value, through the general walker,
    /// written to the node's slot if it has one, and whether it passes the
    /// node's test.
    pub(super) fn is_test(&mut self, node: &IrNode) -> Flow<bool> {
        let IrKind::Is { value, test, slot } = node.kind else {
            return mistyped(node, "a node that is not an `is` tested");
        };
        let value = self.value(value)?;
        let Some(code) = self.ir.test(test) else {
            return mistyped(node, "an `is` of a test its IR lacks");
        };
        let passed = passes(&code.test, &value);
        if slot != Slot::NONE {
            let target = self
                .stack
                .get_mut(self.base + slot.index())
                .ok_or_else(|| internal("store to a slot outside the frame", node.span))?;
            *target = value;
        }
        Ok(passed)
    }

    /// Evaluates a total comparison of two values of a union of kinds, a
    /// `ValueSame` if `equal` and a `ValueNotSame` otherwise: the values
    /// compare whole, deeply, as [`Value`]'s equality does.
    pub(super) fn value_same(&mut self, operands: Operands, equal: bool) -> Flow<bool> {
        let lhs = self.value(operands.lhs)?;
        let rhs = self.value(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }
}
