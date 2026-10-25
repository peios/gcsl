//! Evaluating context: reads of a context, and `with`.
//!
//! # Context
//!
//! The run's *current context* (see the checker's "Context") is a chain of
//! frames (`ContextFrame`), innermost first, `None` at the
//! root, kept where every frame of the run shares it, so a call inherits
//! its caller's and costs nothing for it. A `with` evaluates its bindings'
//! values in the current context, then its body in a new frame binding
//! them, each by its declaration's key, over the current one, which it
//! restores after, whatever the body does. A frame has an identity, new for
//! each `with` evaluated. A future captures the current context as it is
//! made, and its body runs with that context, whoever awaits it.
//!
//! A read (`Evaluator::context_read`) takes the nearest frame's binding of
//! its key; with none, it calls its default's function, as a call of no
//! arguments at its depth, in the current context, so that a default
//! reading another context follows it; with no default either, it is
//! "context `arch` has no value here", which the checker's needs make
//! unreachable but for a host's own calls (see
//! [Needs](crate::check::needs#needs)). A read is an ambient input of the
//! memoised calls running (see [Memoised functions](crate::eval::memo#memoised-functions)):
//! it is recorded on each running call whose own context's nearest binding
//! of the key is the read's, the same frame's or none, with the value read,
//! or none for the default, whose value depends only on what it reads in
//! turn, recorded as it runs (`MemoTables::depend_context`). A call whose
//! context gives every such key the same value finds the entry. A cache hit
//! passes the entry's context dependencies on, as reads here would. Making
//! a future in a memoised call records every context the call's own
//! context binds, flattened, as one dependency (`MemoTables::depend_contexts`):
//! the future may read any of them after the call has returned.

use std::rc::Rc;

use crate::{CALL_LEVELS, MAX_DEPTH};
use crate::diagnostic::Diagnostic;
use crate::ir::{IrId, IrKind, IrNode, IrRange};
use crate::names::Name;
use crate::value::{ContextFrame, Value};

use super::{Callee, Evaluator, Flow, mistyped, nested_too_deeply};

impl Evaluator<'_> {
    /// The value of the read `node` of the context of the declaration key
    /// `key` (see [Context](#context)): its nearest binding's in the running
    /// context; with none, the value its default's function, `default`,
    /// returns, called with no arguments as a call at depth `depth` would
    /// be; with no default either, "context `arch` has no value here".
    /// Recorded as a dependency of the running memoised calls it can affect
    /// (see [Memoised functions](crate::eval::memo#memoised-functions), "Ambient inputs").
    #[inline(never)]
    pub(super) fn context_read(
        &mut self,
        key: Name,
        (default, depth): (IrId, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let context = self.shared.context.borrow().clone();
        self.memo.depend_context(key, context.as_ref());
        self.note_step_read(key, context.as_ref());
        if let Some((_, value)) = ContextFrame::find(context.as_ref(), key) {
            return Ok(value.clone());
        }
        if self.ir.get(default).kind == IrKind::Empty {
            let name = self
                .ir
                .name_text(key)
                .and_then(crate::names::key_context)
                .unwrap_or_default();
            return Err(Diagnostic::error(
                format!("context `{name}` has no value here"),
                node.span,
            )
            .with_note("no `with` sets it, and it declares no default")
            .into());
        }
        let Callee::Closure(closure) = self.function(default)? else {
            return mistyped(node, "a context's default that is no closure");
        };
        let Some(code) = closure.code.find_function(closure.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        let passed = self.callee_caller();
        let result = Evaluator {
            ir: &closure.code,
            stack: self.stack,
            base,
            captures: &closure.captures,
            callee: Some(&closure),
            memo: self.memo,
            shared: self.shared,
            caller: passed,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Err(error) => Err(self.call_error(error, &closure, node).into()),
            Ok(value) => Ok(value),
        }
    }

    /// Notes a read of the context of the key `key` in the context `context`
    /// for the step whose unit the run is of, if it is one's, telling the
    /// host of the first of the run (see [Graph mode](crate::eval::graph#graph-mode),
    /// "Reads").
    pub(super) fn note_step_read(&self, key: Name, context: Option<&Rc<ContextFrame>>) {
        if let Some(run) = self.shared.graph
            && let Some(text) = run.note_read(key, context)
        {
            self.shared.effects.context_read(text);
        }
    }

    /// Notes that the step whose unit the run is of read every context its
    /// context binds, telling the host of each it had not read.
    pub(super) fn note_step_reads_all(&self) {
        if let Some(run) = self.shared.graph {
            for text in run.note_all() {
                self.shared.effects.context_read(text);
            }
        }
    }

    /// Evaluates the `with` `node`, of the bindings and then the body
    /// `parts` (see [Context](#context)): each binding's value, in order, in
    /// the running context, and then the body, as a value, in a new frame of
    /// the context binding them over the running one, which is restored
    /// after, whatever the body does.
    #[inline(never)]
    pub(super) fn with(&mut self, parts: IrRange, node: &IrNode) -> Flow<Value> {
        let Some((&body, binds)) = self.ir.items(parts).split_last() else {
            return mistyped(node, "a `with` with no body");
        };
        let mut bound = Vec::with_capacity(binds.len());
        for &item in binds {
            let IrKind::ContextBind { key, value } = self.ir.get(item).kind else {
                return mistyped(node, "a `with` binding that is none");
            };
            bound.push((key, self.value(value)?));
        }
        let frame = ContextFrame {
            id: self.memo.new_context_frame(),
            binds: bound.into_boxed_slice(),
            outer: self.shared.context.borrow().clone(),
        };
        let outer = self.shared.context.replace(Some(Rc::new(frame)));
        let result = self.value(body);
        self.shared.context.replace(outer);
        result
    }
}
