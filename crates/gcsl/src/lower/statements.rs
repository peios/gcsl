//! Lowering stores, jumps and returns, and the catches that returns unwind to.

use crate::ast::{ExprId, ItemId, ItemKind, Jump, PathRange};
use crate::check::{JumpTarget, Slot};
use crate::diagnostic::InternalError;
use crate::ir::{CatchId, IrId, IrKind};
use crate::source::Span;
use crate::value::ValueKind;

use super::Lowerer;

impl Lowerer<'_> {
    /// Lowers the declaration `id`, a `let`, a field or a `:=`, to the
    /// `Store` of its value (see [`Lowerer::lower_declaration`]) into the
    /// slot the checker gave it.
    pub(super) fn lower_store(&mut self, id: ItemId) -> Result<IrId, InternalError> {
        let item = self.ast.item(id);
        // A definition made with a def-type, or a definition's meta, has no
        // path of its own to update: its closure is stored in its leaf's
        // cell.
        let (path, value) = match item.kind {
            ItemKind::Let { path, value, .. }
            | ItemKind::Rebind { path, value }
            | ItemKind::Field { path, value, .. } => (Some(path), value),
            ItemKind::DefTypeDef { value, .. } | ItemKind::DefMeta { value, .. } => (None, value),
            _ => {
                return Err(InternalError {
                    message: "an item that declares nothing lowered as a declaration",
                    span: item.span,
                });
            }
        };
        let slot = self.resolution.item_slot(id);
        if slot == Slot::NONE {
            return Err(InternalError {
                message: "a declaration has no slot",
                span: item.span,
            });
        }
        if let ItemKind::Rebind { .. } = item.kind {
            self.assigns(slot);
        }
        let value = match path {
            Some(path) => self.lower_declaration(id, path, value)?,
            None => self.lower_expr(value)?,
        };
        Ok(self.builder.alloc(IrKind::Store(slot, value), item.span))
    }

    /// Lowers the `break` or `continue` item `id`, whose value, if any, is
    /// `jump`'s, to a `Break` or a `Continue` of its value as it is used,
    /// counting the loops it leaves before the one the checker found it is for
    /// (see [`Resolution::jump_target`](super::Resolution::jump_target)), which
    /// is among those whose bodies are being lowered.
    pub(super) fn lower_jump(&mut self, id: ItemId, jump: Jump) -> Result<IrId, InternalError> {
        let item = self.ast.item(id);
        if let ItemKind::Return(_) = item.kind {
            return self.lower_return(id, jump);
        }
        let target = self.resolution.jump_target(id);
        let Some(at) = self
            .loops
            .iter()
            .rposition(|&loop_id| Some(loop_id) == target)
        else {
            return Err(InternalError {
                message: "a `break` or `continue` is not in the body of its loop",
                span: item.span,
            });
        };
        self.leaves(at, matches!(item.kind, ItemKind::Break(_)));
        let loops = u32::try_from(self.loops.len() - 1 - at).unwrap_or(u32::MAX);
        let value = jump.value.map(|value| self.lower_expr(value)).transpose()?;
        let kind = match item.kind {
            ItemKind::Break(_) => IrKind::Break { loops, value },
            _ => IrKind::Continue { loops, value },
        };
        Ok(self.builder.alloc(kind, item.span))
    }

    /// Lowers the `return` item `id`, whose value, if any, is `jump`'s, to a
    /// `Return` of its value as it is used (awaited where the checker marked
    /// it, to fit its target), naming what the checker found it leaves (see
    /// [`Resolution::target`](super::Resolution::target)):
    /// [`CatchId::FUNCTION`] for the function or `lazy` around it, or the
    /// number of the catch of the labelled block or loop it names, which is
    /// among those being lowered, on [`Lowerer::catches`].
    pub(super) fn lower_return(&mut self, id: ItemId, jump: Jump) -> Result<IrId, InternalError> {
        let item = self.ast.item(id);
        let to = match self.resolution.target(id) {
            Some(JumpTarget::Function) => CatchId::FUNCTION,
            Some(target) => self
                .catches
                .iter()
                .rev()
                .find(|&&(around, _)| around == target)
                .map(|&(_, catch)| catch)
                .ok_or(InternalError {
                    message: "a `return` is not in the block or loop it leaves",
                    span: item.span,
                })?,
            None => {
                return Err(InternalError {
                    message: "a `return` has nothing to leave",
                    span: item.span,
                });
            }
        };
        // It leaves every loop it is in, conservatively.
        self.leaves(0, true);
        let value = jump.value.map(|value| self.lower_expr(value)).transpose()?;
        Ok(self.builder.alloc(IrKind::Return { to, value }, item.span))
    }

    /// Starts a catch for `target`, a labelled block or a loop that some
    /// `return` leaves: numbers it, the next number of the program, and
    /// puts it on [`Lowerer::catches`] for its block's items or its loop's
    /// body, which the caller lowers and then ends it with
    /// [`Lowerer::end_catch`].
    pub(super) fn start_catch(&mut self, target: JumpTarget) -> CatchId {
        let id = CatchId::new(self.next_catch);
        self.next_catch = self.next_catch.saturating_add(1);
        self.catches.push((target, id));
        id
    }

    /// Ends the catch `id`, started by [`Lowerer::start_catch`], whose
    /// block or loop lowered to `body`, spanning `span`: an
    /// [`IrKind::Catch`] of it, of kind `kind`, the whole's.
    pub(super) fn end_catch(
        &mut self,
        id: CatchId,
        (body, span): (IrKind, Span),
        kind: ValueKind,
    ) -> IrKind {
        self.catches.pop();
        let body = self.builder.alloc(body, span);
        IrKind::Catch { body, id, kind }
    }

    /// Lowers the value that the declaration `id`, whose path is `path`,
    /// stores: its expression `value` for a plain path, for a dotted field
    /// the chain of field updates that sets the path to it, and for a dotted
    /// `let` or `:=` the update that does.
    pub(super) fn lower_declaration(
        &mut self,
        id: ItemId,
        path: PathRange,
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        match (self.ast.item(id).kind, self.ast.path(path)) {
            (_, [_]) => self.lower_expr(value),
            // A top-level function named by a dotted path: its closure,
            // stored in its leaf's cell.
            (ItemKind::Field { function: true, .. }, _) if self.is_top_function(id) => {
                self.lower_expr(value)
            }
            (ItemKind::Field { .. }, segments) => self.lower_field_update(id, segments, value),
            (_, segments) => self.lower_update(id, segments, value),
        }
    }

    /// Whether the item `id` is one of the input's top-level functions.
    pub(super) fn is_top_function(&self, id: ItemId) -> bool {
        self.resolution
            .top_functions()
            .iter()
            .any(|&(item, _)| item == id)
    }
}
