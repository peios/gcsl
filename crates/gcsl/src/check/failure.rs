//! Checking `assert` and `recover`.
//!
//! # Failure
//!
//! `assert COND, MESSAGE` (see the parser's [Failure](crate::parser#failure))
//! is `{}`: its condition must be a `Bool`, "an assertion's condition is a
//! Bool", and its message a `String` (`Checker::check_assert`). `recover { BODY
//! } else (e) { FALLBACK }` (`Checker::check_recover`) checks its body, then
//! its fallback in a scope of its own, where the failure, if it is named, is a
//! variable of type `{message: String}` in a slot of its own, recorded for
//! lowering ([`Resolution::recover_slot`](super::Resolution::recover_slot));
//! its type is the join of the two, as an `if`'s branches', warned of as theirs
//! are when it widens to a union, "the branches of this `recover` have
//! different types". Its effect is its blocks': a `recover` catches a failure,
//! it does nothing of its own.

use crate::ast::{ExprId, RecoverId};
use crate::source::Span;
use crate::types::TyId;

use super::{BindingKind, Checker, FAILURE_MESSAGE};

impl Checker<'_> {
    /// Types `id`, `assert COND` or `assert COND, MESSAGE` (see
    /// [Failure](#failure)): the condition a `Bool`, the message a `String`;
    /// the statement `{}`.
    pub(super) fn check_assert(&mut self, id: ExprId, cond: ExprId, message: ExprId) -> TyId {
        let keyword = Span::new(self.ast.get(id).span.start, self.ast.get(id).span.start + 6);
        let cond_ty = self.check_expr(cond);
        self.expect(
            cond,
            cond_ty,
            TyId::BOOL,
            keyword,
            "an assertion's condition is a Bool",
        );
        let ty = self.check_expr(message);
        self.expect(
            message,
            ty,
            TyId::STRING,
            keyword,
            "an assertion's message is a String",
        );
        if cond_ty == TyId::ERROR {
            TyId::ERROR
        } else {
            TyId::EMPTY
        }
    }

    /// Types `id`, the `recover` expression `recover` (see
    /// [Failure](#failure)): its body, then its fallback, in a scope where
    /// the failure, if it is named, is a variable of type `{message:
    /// String}` in a slot of its own, recorded for lowering. Its type is
    /// the join of the body's and the fallback's, as an `if`'s branches'.
    pub(super) fn check_recover(&mut self, recover: RecoverId) -> TyId {
        let decl = *self.ast.recover_decl(recover);
        let body = self.check_expr(decl.body);
        let outer = (self.block_start, self.next_slot);
        self.block_start = self.bindings.len();
        if let Some((name, span)) = decl.error {
            // Interned by the parser for a named failure.
            let message = self.ast.names().lookup(FAILURE_MESSAGE);
            let failure = message.map_or(TyId::ERROR, |message| {
                self.types.record([(message, TyId::STRING)])
            });
            let slot = self.declare_binding(name, failure, Some(span), BindingKind::Variable);
            self.lowering.recover_slots.insert(recover, slot);
        }
        let fallback = self.check_expr(decl.fallback);
        self.bindings.truncate(self.block_start);
        (self.block_start, self.next_slot) = outer;
        self.branches((decl.body, body), (decl.fallback, fallback), "`recover`")
    }
}
