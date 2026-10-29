//! Lowering `recover`.

use crate::ast::RecoverId;
use crate::check::{FAILURE_MESSAGE, FAILURE_NOTES};
use crate::diagnostic::InternalError;
use crate::ir::IrKind;
use crate::value::ValueKind;

use super::Lowerer;

impl Lowerer<'_> {
    /// Lowers the def-type reference `id`, `DT PATH`, of `def` (see the
    /// checker's "Def-types"), to the `MakeClosure` of a function of its own,
    /// made here, past the tree's ([`FnId::lowered`](super::FnId::lowered)),
    /// which captures the def-type and the definition, read where the reference
    /// is, and whose body, run with its parameters, those of the def-type after
    /// its definition's, sets its helper flag, so that the def-type is called
    /// with the caller the closure is called with, and calls the def-type with
    /// the definition's path, as written, the variant the reference names, a
    /// constant tag, if the def-type's definitions are made with one, the
    /// definition and its own parameters, in order: so a closure is made each
    /// time the reference is evaluated. If the reference carries its
    /// definition's meta, the function's meta is recorded as its capture of the
    /// definition's ([`MetaAt::Capture`](super::MetaAt::Capture)). It is of
    /// kind `kind`, which can only be a function's. Lowers the `recover`
    /// expression `recover`, of kind `kind` (see the checker's "Failure"): its
    /// body, and its fallback, after storing the failure, if the fallback names
    /// it, in the slot the check gave it.
    pub(super) fn lower_recover(
        &mut self,
        recover: RecoverId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let decl = *self.ast.recover_decl(recover);
        let body = self.lower_operand(decl.body, kind)?;
        let mut fallback = self.lower_operand(decl.fallback, kind)?;
        if let Some(slot) = self.resolution.recover_slot(recover) {
            let span = self.ast.get(decl.fallback).span;
            let names = self.ast.names();
            let fields = names
                .lookup(FAILURE_MESSAGE)
                .zip(names.lookup(FAILURE_NOTES))
                .ok_or(InternalError {
                    message: "a failure's fields are not interned",
                    span,
                })?;
            let layout = self.layout_of(&[fields.0, fields.1]);
            let failure = self.builder.alloc(IrKind::Failure(layout), span);
            let store = self.builder.alloc(IrKind::Store(slot, failure), span);
            let items = self.builder.push_item_list(&[store]);
            fallback = self.builder.alloc(
                IrKind::Block {
                    items,
                    result: fallback,
                },
                span,
            );
        }
        Ok(IrKind::Recover {
            body,
            fallback,
            kind,
        })
    }
}
