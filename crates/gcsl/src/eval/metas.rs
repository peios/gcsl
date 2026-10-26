//! Evaluating metas: a function's meta, read as its own fields.
//!
//! # Metas
//!
//! A read of a function's meta, `func.versions` (see the checker's "Metas"),
//! is a `Field` of a `Call` of no arguments whose callee is a `Meta` of the
//! function: the function walker evaluates the `Meta` (`Evaluator::meta`)
//! by evaluating the function to its closure and taking the closure that
//! the cell of the top level's frame its code names holds
//! ([`Ir::meta_of`](super::Ir::meta_of)), the closure of the definition's meta, which the
//! meta's file stored there as any top-level function's, before anything
//! that uses the definition ran. The closure of a def-type reference
//! carries its definition's meta: its code names, for its function, the
//! capture that holds the definition ([`MetaAt::Capture`]), whose closure's
//! meta is then taken as above; so a read through a reference and one
//! through the definition, as the def-type reads it, take the one closure of
//! the one meta. Then the call is a call like any other: a
//! `META->` meta's body runs at each read, a `META->>` meta's closure is
//! memoised as any closure of a memoised function is, by the closure, so
//! its body runs once per run however often it is read, and a `META:`
//! meta's runs at each read too, which is invisible; the body runs for the
//! caller the reading frame gives a callee, `fileroot.helper()` applying, and
//! its own code's directory is its `fileroot.this`. A closure whose code
//! names no meta, or a built-in function, is an internal error, which the
//! checker rules out. A function with a meta is a closure like any other:
//! equality, `is` and the keys of memoised calls see nothing of its meta.

use std::rc::Rc;

use crate::ir::{IrId, IrNode, MetaAt};
use crate::value::Value;

use super::{Callee, Evaluator, Flow, internal};

impl Evaluator<'_> {
    /// The closure of the meta of the function that `function` evaluates to,
    /// for the `Meta` `node` (see [Metas](#metas)): the closure of a
    /// definition's meta, which the cell of the top level's frame that the
    /// definition's code names holds ([`Ir::meta_of`](super::Ir::meta_of)); for a def-type
    /// reference's closure, which carries its definition's meta, that of the
    /// definition it captures ([`MetaAt::Capture`]). A built-in function, or
    /// a closure whose code names no meta, which the checker rules out, is
    /// an internal error. Out of line: only a meta's read pays for it.
    #[inline(never)]
    pub(super) fn meta(&mut self, function: IrId, node: &IrNode) -> Flow<Callee> {
        let Callee::Closure(mut closure) = self.function(function)? else {
            return Err(internal("the meta of a built-in function read", node.span).into());
        };
        loop {
            match closure.code.meta_of(closure.function) {
                Some(MetaAt::Cell(slot)) => return self.top_function(slot, node),
                Some(MetaAt::Capture(index)) => {
                    let Some(Value::Fn(captured)) = closure.captures.get(index as usize) else {
                        return Err(internal(
                            "the meta of a reference that captures no definition read",
                            node.span,
                        )
                        .into());
                    };
                    closure = Rc::clone(captured);
                }
                None => {
                    return Err(
                        internal("the meta of a function that has none read", node.span).into(),
                    );
                }
            }
        }
    }

    /// [`Evaluator::meta`] as a value. Out of line, as `meta` is.
    #[inline(never)]
    pub(super) fn meta_value(&mut self, function: IrId, node: &IrNode) -> Flow<Value> {
        match self.meta(function, node)? {
            Callee::Closure(closure) => Ok(Value::Fn(closure)),
            Callee::Builtin(builtin) => Ok(Value::Builtin(builtin)),
            Callee::Tag(tag) => Ok(Value::Tag(tag)),
        }
    }
}
