//! Checking `assert` and `recover`.
//!
//! # Failure
//!
//! `assert COND, MESSAGE` (see the parser's [Failure](crate::parser::failure#failure))
//! is `{}`: its condition must be a `Bool`, "an assertion's condition is a
//! Bool", and its message a `String` (`Checker::check_assert`). `recover { BODY
//! } else (e) { FALLBACK }` (`Checker::check_recover`) checks its body, then
//! its fallback in a scope of its own, where the failure, if it is named, is a
//! variable of type `{message: String, notes: List<String>}` in a slot of
//! its own, recorded for
//! lowering ([`Resolution::recover_slot`](super::Resolution::recover_slot));
//! its type is the join of the two, as an `if`'s branches', warned of as theirs
//! are when it widens to a union, "the branches of this `recover` have
//! different types". Its effect is its blocks': a `recover` catches a failure,
//! it does nothing of its own.
//!
//! `try EXPR` (`Checker::check_try`) is `recover { EXPR } else { {} }`,
//! with nothing said of the two types differing: its type is the join of
//! its operand's and `{}`, so `{}` for an operand that is `{}`, as a call
//! made for its effects is, `Int | {}` for an `Int`, and `{}` for any
//! record, whose fields the join with `{}` leaves none of; a program that
//! wants a failed expression's value told from a record's uses `recover`.
//! Its operand is awaited where it is, deeply, as a statement's call is,
//! so that what it defers fails inside the `try`; in a pure function an
//! operand that may be a future is the usual error.
//!
//! `raise`, in a fallback, raises again the failure its `recover` caught,
//! as it was (`Checker::check_raise`). It is no keyword: a name `raise`
//! that names no variable is the form, of type `Never`, so a variable or a
//! field named `raise` is read as it always was. It is for the innermost
//! `recover` whose fallback it is written in, in the function that
//! `recover` is in, a block it gives a call included; anywhere else, in a
//! function or a `lazy` written in the fallback too, it is "`raise` is
//! only for the fallback of a `recover`". Each fallback has a variable no
//! program can name, an `Int`, in which it keeps which of the failures
//! caught is its own; the slot is recorded for each `raise`
//! ([`Resolution::raise_of`](super::Resolution::raise_of)), and for each
//! `recover` whose fallback has one
//! ([`Resolution::raise_slot`](super::Resolution::raise_slot)), which
//! lowering stores it for.

use crate::ast::{ExprId, RecoverId};
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::TyId;

use super::{BindingKind, Checker, FAILURE_AT, FAILURE_MESSAGE, FAILURE_NOTES, Slot};

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
    /// String, notes: List<String>}` in a slot of its own, recorded for
    /// lowering. Its type is
    /// the join of the body's and the fallback's, as an `if`'s branches'.
    pub(super) fn check_recover(&mut self, recover: RecoverId) -> TyId {
        let decl = *self.ast.recover_decl(recover);
        let body = self.check_expr(decl.body);
        let outer = (self.block_start, self.next_slot);
        self.block_start = self.bindings.len();
        // Interned by the parser for every `recover`.
        let at = self
            .ast
            .names()
            .lookup(FAILURE_AT)
            .map(|name| self.declare_binding(name, TyId::INT, None, BindingKind::Variable));
        self.calls.fallbacks.push(Fallback {
            depth: self.functions.len(),
            at,
            raised: false,
        });
        if let Some((name, span)) = decl.error {
            // Interned by the parser for a named failure.
            let names = self.ast.names();
            let fields = names
                .lookup(FAILURE_MESSAGE)
                .zip(names.lookup(FAILURE_NOTES));
            let failure = fields.map_or(TyId::ERROR, |(message, notes)| {
                let notes_ty = self.types.list(TyId::STRING);
                self.types
                    .record([(message, TyId::STRING), (notes, notes_ty)])
            });
            let slot = self.declare_binding(name, failure, Some(span), BindingKind::Variable);
            self.lowering.recover_slots.insert(recover, slot);
        }
        let fallback = self.check_expr(decl.fallback);
        if let Some(Fallback {
            at: Some(at),
            raised: true,
            ..
        }) = self.calls.fallbacks.pop()
        {
            self.lowering.raise_slots.insert(recover, at);
        }
        self.bindings.truncate(self.block_start);
        (self.block_start, self.next_slot) = outer;
        self.branches((decl.body, body), (decl.fallback, fallback), "`recover`")
    }

    /// Types `try OPERAND` (see [Failure](#failure)): its operand, awaited
    /// deeply where it is, so that nothing it defers fails once the `try`
    /// has let it through, and then the join of its type and `{}`.
    pub(super) fn check_try(&mut self, operand: ExprId) -> TyId {
        let found = self.check_expr(operand);
        let found = self.settle(operand, found);
        let found = self.measure_deep(operand, found);
        self.types.join(found, TyId::EMPTY).0
    }

    /// Types `id`, a name `raise` written at `span` that names no variable
    /// (see [Failure](#failure)): in the fallback of a `recover` of the
    /// function being checked, the raising again of the failure that
    /// `recover` caught, recorded for lowering, of type `Never`; anywhere
    /// else, "`raise` is only for the fallback of a `recover`".
    pub(super) fn check_raise(&mut self, id: ExprId, span: Span) -> TyId {
        let depth = self.functions.len();
        let fallback = self
            .calls
            .fallbacks
            .last_mut()
            .filter(|fallback| fallback.depth == depth);
        if let Some(fallback) = fallback
            && let Some(at) = fallback.at
        {
            fallback.raised = true;
            self.lowering.raises.insert(id, at);
            return self.never;
        }
        let mut error = Diagnostic::error("`raise` is only for the fallback of a `recover`", span)
            .with_note(
                "`raise` raises again the failure a `recover` caught: write it in the block \
                 after that `recover`'s `else`",
            );
        if let (Some(_), Some(function)) = (self.calls.fallbacks.last(), self.functions.last()) {
            let when = if self.ast.fn_decl(function.fn_id).ret.is_none() {
                "awaited"
            } else {
                "called"
            };
            error = error.with_context(
                function.keyword,
                format!("this runs when it is {when}, which may be after the fallback has ended"),
            );
        }
        self.errors.push(error);
        TyId::ERROR
    }
}

/// A `recover` whose fallback is being checked (see [Failure](self#failure)).
#[derive(Clone, Copy, Debug)]
pub(super) struct Fallback {
    /// How many functions the checker was inside where it is written.
    depth: usize,
    /// The slot in which the fallback keeps which of the failures caught is
    /// its own.
    at: Option<Slot>,
    /// Whether a `raise` in the fallback is for it.
    raised: bool,
}
