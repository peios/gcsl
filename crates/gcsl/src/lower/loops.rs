//! Lowering loops: `for`, `while` and their bodies.

use crate::ast::{ExprId, ExprKind, ForHead, ForId, symbol_span};
use crate::check::{AwaitMark, JumpTarget, Slot};
use crate::diagnostic::InternalError;
use crate::ir::{IrId, IrKind};
use crate::source::Span;
use crate::value::ValueKind;

use super::Lowerer;

impl Lowerer<'_> {
    /// Lowers the loop `for_id`, spanning `span`, which collects what its
    /// iterations contribute into a list if `collect`, and discards it
    /// otherwise. An iteration lowers to a `For` on its variable's slot,
    /// whose parts are its iterable, a list, as it is used, its guard, if
    /// it has one, a `Bool`, and its body, as it is used, which may be
    /// awaited to fit the element type expected of the loop. A condition
    /// loop lowers to a `Loop` whose parts are its condition, a `Bool`, or
    /// `true` for `for { ... }`, its body and the `Store`s of the items
    /// after its condition, in a `Block` whose items are the `Store`s of
    /// those before it, if it has any; a `do` loop to a `Loop` of its
    /// condition and its body that does not test first. Each body is
    /// lowered with its loop on [`Lowerer::loops`].
    ///
    /// A loop that cannot end (see [`crate::check::LoopResolution`]) never
    /// collects, whatever `collect` says: nothing could read its list. A
    /// loop that a `return` leaves is wrapped in a `Catch` of kind `kind`,
    /// the whole loop's, its body lowered with the catch on
    /// [`Lowerer::catches`].
    pub(super) fn lower_for(
        &mut self,
        for_id: ForId,
        (kind, collect): (ValueKind, bool),
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let found = self.resolution.loop_of(for_id);
        let collect = collect && !found.endless;
        if !found.returned {
            return self.lower_loop_node(for_id, collect, span);
        }
        let catch = self.start_catch(JumpTarget::Loop(for_id));
        let looped = self.lower_loop_node(for_id, collect, span)?;
        Ok(self.end_catch(catch, (looped, span), kind))
    }

    /// Lowers the loop `for_id`, spanning `span`, as [`Lowerer::lower_for`]
    /// says, without the catch around it.
    pub(super) fn lower_loop_node(
        &mut self,
        for_id: ForId,
        collect: bool,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let decl = *self.ast.for_decl(for_id);
        let (before, cond, after) = match decl.head {
            ForHead::Each {
                iterable, guard, ..
            } => return self.lower_each(for_id, (iterable, guard, decl.body), collect),
            ForHead::While {
                before,
                cond,
                after,
            } => (Some(before), cond, Some(after)),
            ForHead::Do { cond } => (None, Some(cond), None),
        };
        let ast = self.ast;
        let base = self.item_stack.len();
        for &item in before.map_or(&[][..], |before| ast.items(before)) {
            let store = self.lower_store(item)?;
            self.item_stack.push(store);
        }
        let stores = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let cond = if let Some(cond) = cond {
            self.lower_operand(cond, ValueKind::Bool)?
        } else {
            let keyword = symbol_span("for", span.start);
            self.builder.alloc(IrKind::BoolConst(true), keyword)
        };
        self.item_stack.push(cond);
        let body = self.lower_loop_body(for_id, decl.body)?;
        self.item_stack.push(body);
        for &item in after.map_or(&[][..], |after| ast.items(after)) {
            let store = self.lower_store(item)?;
            self.item_stack.push(store);
        }
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let looped = IrKind::Loop {
            parts,
            collect,
            test_first: before.is_some(),
        };
        if before.is_none_or(|before| ast.items(before).is_empty()) {
            return Ok(looped);
        }
        let result = self.builder.alloc(looped, span);
        Ok(IrKind::Block {
            items: stores,
            result,
        })
    }

    /// Lowers the iteration `for_id`, whose iterable, guard, if any, and
    /// body are `iterable`, `guard` and `body`, to a `For` (see
    /// [`Lowerer::lower_for`]).
    pub(super) fn lower_each(
        &mut self,
        for_id: ForId,
        (iterable, guard, body): (ExprId, Option<ExprId>, ExprId),
        collect: bool,
    ) -> Result<IrKind, InternalError> {
        let slot = self.resolution.for_slot(for_id);
        if slot == Slot::NONE {
            return Err(InternalError {
                message: "a `for` has no slot for its variable",
                span: self.ast.get(body).span,
            });
        }
        let base = self.item_stack.len();
        let iterable = self.lower_operand(iterable, ValueKind::List)?;
        self.item_stack.push(iterable);
        self.forks.push((for_id, slot, true));
        let lowered = self.lower_each_parts(for_id, guard, body);
        let forkable = self.forks.pop().is_some_and(|(_, _, forkable)| forkable);
        lowered?;
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::For {
            slot,
            parts,
            collect,
            forkable,
        })
    }

    /// Lowers the guard, if any, and the body of the iteration `for_id`
    /// onto [`Lowerer::item_stack`], the iteration on
    /// [`Lowerer::forks`].
    fn lower_each_parts(
        &mut self,
        for_id: ForId,
        guard: Option<ExprId>,
        body: ExprId,
    ) -> Result<(), InternalError> {
        if let Some(guard) = guard {
            let guard = self.lower_operand(guard, ValueKind::Bool)?;
            self.item_stack.push(guard);
        }
        let body = self.lower_loop_body(for_id, body)?;
        self.item_stack.push(body);
        Ok(())
    }

    /// Notes that what is being lowered assigns the variable in `slot`:
    /// every iteration around it that the variable was declared outside of
    /// is not forkable.
    pub(super) fn assigns(&mut self, slot: Slot) {
        for (_, floor, forkable) in &mut self.forks {
            if slot.index() < floor.index() {
                *forkable = false;
            }
        }
    }

    /// Notes that what is being lowered leaves the loops after the
    /// `at`th on [`Lowerer::loops`], and the `at`th itself if it ends it,
    /// `ends`: none of them is forkable.
    pub(super) fn leaves(&mut self, at: usize, ends: bool) {
        let first = if ends { at } else { at + 1 };
        let left: Vec<ForId> = self.loops.get(first..).unwrap_or_default().to_vec();
        for (for_id, _, forkable) in &mut self.forks {
            if left.contains(for_id) {
                *forkable = false;
            }
        }
    }

    /// Lowers `body`, the body of the loop `for_id`, as it is used, with
    /// the loop on [`Lowerer::loops`], so that the `break`s and `continue`s
    /// in it count the loops they leave.
    pub(super) fn lower_loop_body(
        &mut self,
        for_id: ForId,
        body: ExprId,
    ) -> Result<IrId, InternalError> {
        self.loops.push(for_id);
        let body = self.lower_expr(body);
        self.loops.pop();
        body
    }

    /// The `for` expression that the statement `value` is, through any
    /// parentheses, with its span, if its type is unit-like as it is, so
    /// that nothing awaits it: its list would be dropped as soon as it was
    /// built, so it is not built.
    pub(super) fn discarded_for(&self, value: ExprId) -> Option<(ForId, Span)> {
        if self.resolution.await_of(value) != AwaitMark::None
            || !self.table.is_unit_like(self.expr_types.get(value))
        {
            return None;
        }
        let mut node = value;
        loop {
            let expr = self.ast.get(node);
            match expr.kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::For(for_id) => return Some((for_id, expr.span)),
                _ => return None,
            }
        }
    }
}
