//! Lowering blocks, and what is never reached.

use crate::ast::{ExprId, ExprKind, ForHead, ForId, ItemRange, LabelId, StrPart, UnaryOp};
use crate::check::{JumpTarget, VarRef};
use crate::diagnostic::InternalError;
use crate::ir::IrKind;
use crate::value::ValueKind;

use super::Lowerer;

impl Lowerer<'_> {
    /// Lowers the block `id`, whose items are `items` and whose label, if
    /// it has one, is `label`, of kind `kind`, to a `Block`; or, if it is a
    /// labelled block that some `return` leaves, to a `Catch` of kind
    /// `kind` of the `Block`, which is of its result's own kind, its items
    /// lowered with the catch on [`Lowerer::catches`]. Apart from
    /// `lower_plain`, which is on the recursion path of every expression.
    pub(super) fn lower_block(
        &mut self,
        id: ExprId,
        (items, label): (ItemRange, Option<LabelId>),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        if label.is_none() || !self.resolution.is_returned_block(id) {
            return self.lower_body((Some(id), items), span, kind);
        }
        let catch = self.start_catch(JumpTarget::Block(id));
        let own = self.body_kind(items)?;
        let block = self.lower_body((Some(id), items), span, own)?;
        Ok(self.end_catch(catch, (block, span), kind))
    }

    /// Lowers the expression `id`, of type `Never` because what it asks of
    /// an operand that always runs first is `Never` (see [`crate::check`]),
    /// to a `Block` whose items are its operands that run before that one,
    /// in order, each as it is used, for their effects, and whose result is
    /// that operand, which never completes; the operands after it never run,
    /// and are lowered only for the functions in them, their nodes left
    /// out. A loop whose iterable or first condition is `Never` is lowered
    /// so too, by [`Lowerer::lower_unreached_loop`]. `None` for any other
    /// expression, and for one of type `Never` only because of the value it
    /// reads, a field of a record type's that is `Never`, say, which no
    /// value has: it is lowered as it is otherwise.
    pub(super) fn lower_unreached(&mut self, id: ExprId) -> Result<Option<IrKind>, InternalError> {
        let expr = *self.ast.get(id);
        let children: Vec<ExprId> = match expr.kind {
            ExprKind::Unary { op, operand } if op != UnaryOp::Await => vec![operand],
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Index {
                expr: lhs,
                index: rhs,
            } => vec![lhs, rhs],
            ExprKind::Field { expr, .. }
            | ExprKind::Is { expr, .. }
            | ExprKind::As { expr, .. } => {
                vec![expr]
            }
            ExprKind::Call { callee, args } => {
                let values = self.ast.args(args).iter().map(|arg| arg.value);
                // A method's callee is a read of its binding, after its
                // receiver.
                if let Some(receiver) = self.resolution.method_receiver(id) {
                    std::iter::once(receiver).chain(values).collect()
                } else {
                    match self.resolution.expr_var(callee) {
                        VarRef::Builtin(_) | VarRef::Fileroot(_) => values.collect(),
                        _ => std::iter::once(callee).chain(values).collect(),
                    }
                }
            }
            ExprKind::Interp(parts) => self
                .ast
                .parts(parts)
                .iter()
                .filter_map(|&part| match part {
                    StrPart::Expr(expr) => Some(expr),
                    StrPart::Lit(_) => None,
                })
                .collect(),
            ExprKind::If {
                cond, then, else_, ..
            } if self.kind(cond)? == ValueKind::Never => vec![cond, then, else_],
            ExprKind::For(for_id) => return self.lower_unreached_loop(for_id),
            _ => return Ok(None),
        };
        let mut first = None;
        for (index, &child) in children.iter().enumerate() {
            if self.kind(child)? == ValueKind::Never {
                first = Some(index);
                break;
            }
        }
        let Some(first) = first else {
            return Ok(None);
        };
        let base = self.item_stack.len();
        let mut result = None;
        for (index, &child) in children.iter().enumerate() {
            let node = self.lower_expr(child)?;
            match index.cmp(&first) {
                std::cmp::Ordering::Less => self.item_stack.push(node),
                std::cmp::Ordering::Equal => result = Some(node),
                // Never run.
                std::cmp::Ordering::Greater => {}
            }
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(result.map(|result| IrKind::Block { items, result }))
    }

    /// Lowers the loop `for_id`, of type `Never` because its iterable, or
    /// its condition, which runs before the first iteration, is: to a
    /// `Block` of the `Store`s of the items before a condition, which run
    /// first, whose result is the iterable or the condition. Its guard, its
    /// items after the condition and its body never run, and are lowered
    /// only for the functions in them. `None` for any other loop.
    pub(super) fn lower_unreached_loop(
        &mut self,
        for_id: ForId,
    ) -> Result<Option<IrKind>, InternalError> {
        let decl = *self.ast.for_decl(for_id);
        let (before, first, after, guard) = match decl.head {
            ForHead::Each {
                iterable, guard, ..
            } if self.kind(iterable)? == ValueKind::Never => (None, iterable, None, guard),
            ForHead::While {
                before,
                cond: Some(cond),
                after,
            } if self.kind(cond)? == ValueKind::Never => (Some(before), cond, Some(after), None),
            _ => return Ok(None),
        };
        let ast = self.ast;
        let base = self.item_stack.len();
        for &item in before.map_or(&[][..], |before| ast.items(before)) {
            let store = self.lower_store(item)?;
            self.item_stack.push(store);
        }
        let result = self.lower_expr(first)?;
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        // Never run.
        if let Some(guard) = guard {
            self.lower_expr(guard)?;
        }
        for &item in after.map_or(&[][..], |after| ast.items(after)) {
            self.lower_store(item)?;
        }
        // The `return`s for the loop need its catch, though it is never
        // made.
        let returned = self.resolution.loop_of(for_id).returned;
        if returned {
            self.start_catch(JumpTarget::Loop(for_id));
        }
        self.lower_loop_body(for_id, decl.body)?;
        if returned {
            self.catches.pop();
        }
        Ok(Some(IrKind::Block { items, result }))
    }
}
