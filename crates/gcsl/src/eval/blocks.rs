//! Evaluating blocks: the trailing block a call gives its callee's block
//! parameter, and the calls of it.
//!
//! # Blocks
//!
//! A call whose callee takes a block (see the checker's
//! [Blocks](crate::check::blocks#blocks)) does not evaluate its trailing
//! block where it is: its argument, an
//! [`ArgValue::Block`](crate::ir::ArgValue::Block), gives the parameter a
//! [`Block`], the body's node with the frame of the call's caller, as the
//! evaluator of that frame holds it: its code, where it starts on the value
//! stack, its closure and its caller (`Evaluator::make_block`). The block
//! is no closure: it captures nothing, and holds no value of the frame.
//!
//! A call of the block, an [`IrKind::CallBlock`](crate::ir::IrKind::CallBlock)
//! (`Evaluator::call_block`), evaluates the body in that frame, as an
//! evaluator of it would, while the callee's own frame, and any above it,
//! stay where they are on the stack: so the body reads and writes the
//! caller's slots, its own variables among them, which is how a block
//! rebinds its caller's mutable variables. The caller's frame is there for
//! as long as the block is, since a block is only ever in its parameter's
//! slot, which goes when the call returns. The body runs in the context
//! the call of the block is made in, the callee's `with`s around it
//! included, and under the deadlines running there. A call of a block
//! counts toward the nesting budget as a call does, its body's height
//! being the block's (see [`crate::MAX_DEPTH`]).
//!
//! A failure in the body comes out of the call of the block as any call's
//! failure does, marked with the input of the block's code if that is
//! another than the callee's, so that the callee can `recover` it, and one
//! it lets through is reported where it was raised. No `break`, `continue`
//! or `return` leaves a block: the checker sees to it.

use std::rc::Rc;

use crate::ir::{IrId, IrNode};
use crate::value::{Block, Value};
use crate::{CALL_LEVELS, MAX_DEPTH};

use super::{Evaluator, Flow, Unwind, internal, mistyped, nested_too_deeply};

impl Evaluator<'_> {
    /// The block whose body is `body`, of the height `height`, given by a
    /// call in this frame (see [Blocks](#blocks)).
    pub(super) fn make_block(&self, body: IrId, height: u16) -> Value {
        // A `lazy`'s body runs with the captures taken out of its future.
        let captures =
            (self.callee.is_none() && !self.captures.is_empty()).then(|| Rc::from(self.captures));
        Value::Block(Rc::new(Block {
            code: Rc::clone(self.ir),
            body,
            height,
            base: self.base,
            callee: self.callee.cloned(),
            captures,
            caller: self.caller,
        }))
    }

    /// Evaluates the call `node`, at depth `depth` in this frame's body, of
    /// the block that `block`, a read of this frame's block parameter,
    /// gives (see [Blocks](#blocks)): its body, in the frame of the call
    /// that gave it.
    #[inline(never)]
    pub(super) fn call_block(&mut self, (block, depth): (IrId, u16), node: &IrNode) -> Flow<Value> {
        let Value::Block(block) = self.value(block)? else {
            return mistyped(node, "a call of a block of a value that is none");
        };
        if self.past_deadline() {
            return self.timed_out(node);
        }
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + usize::from(block.height) > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        if block.base > self.base {
            return mistyped(node, "a block of a frame above its call");
        }
        let captures: &[Value] = match (&block.captures, &block.callee) {
            (Some(captures), _) => captures,
            (None, Some(closure)) => &closure.captures,
            (None, None) => &[],
        };
        let result = Evaluator {
            ir: &block.code,
            stack: self.stack,
            base: block.base,
            captures,
            callee: block.callee.as_ref(),
            memo: self.memo,
            shared: self.shared,
            caller: block.caller,
            levels: entering,
        }
        .value(block.body);
        match result {
            Ok(value) => Ok(value),
            Err(Unwind::Error(error)) => Err(self.marked(error, &block.code).into()),
            Err(Unwind::Stop(_)) => Err(internal("a jump left a block", node.span).into()),
        }
    }
}
