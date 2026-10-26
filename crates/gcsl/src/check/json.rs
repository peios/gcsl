//! Checking `json.decode`.
//!
//! # JSON decoding
//!
//! `json.decode(text)` decodes the JSON document `text` as a value of the
//! type expected of the call, which must be expected of it where it stands:
//! `let graph: Graph = json.decode(text)`, `json.decode(text): Graph`, an
//! argument of a parameter of that type, or a function's result. With no
//! type expected, the call is the error "`json.decode` needs to know the
//! type to decode as". The type must be one JSON can be decoded as (see
//! [Decoding plans](crate::types::decode#decoding-plans)), or the call is the
//! error "cannot decode JSON as a File", with the reason.
//!
//! The checker plans the decode once, from the type, and records the plan
//! for the call ([`Resolution::decode_of`](super::Resolution::decode_of)); the
//! call lowers to a node of its own, as `show`'s does, and the evaluator
//! follows the plan. The expected type reaches the call through
//! parentheses, a block's result and the branches of an `if` with `else`,
//! as it reaches a list literal, so a function's body can be the call.
//!
//! The builtin is pure: the same text decodes to the same value, and
//! failing to, a document that is not JSON or does not fit the type, is a
//! runtime error at the call, as a failed `as` is.

use std::collections::HashMap;

use crate::ast::{ExprId, ExprKind, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::types::{Decode, TyId, Undecodable};

use super::{Builtin, Checker};

/// What the checker knows of the calls of `json.decode` (see [JSON
/// decoding](#json-decoding)).
#[derive(Default)]
pub(super) struct Decoding {
    /// The type expected of each call being checked that may be one of
    /// `json.decode`, by its id.
    targets: HashMap<ExprId, TyId>,
    /// The plan of each call of `json.decode` checked, by its id: see
    /// [`Resolution::decode_of`](super::Resolution::decode_of).
    pub(super) plans: HashMap<ExprId, Decode>,
}

impl Checker<'_> {
    /// Notes that `expected` is the type expected of `id` while `check`
    /// checks it, for each call that `id` results in to decode as, should it
    /// be one of `json.decode` (see [JSON decoding](#json-decoding)), and
    /// returns what `check` does. `id` results in itself if it is a call,
    /// in what a parenthesis holds, in a block's result and in each branch
    /// of an `if` with `else`, as a list literal's expected type reaches it.
    pub(super) fn expecting_decode(
        &mut self,
        id: ExprId,
        expected: TyId,
        check: impl FnOnce(&mut Self) -> TyId,
    ) -> TyId {
        let calls = self.resulting_calls(id);
        for &call in &calls {
            self.decoding.targets.insert(call, expected);
        }
        let found = check(self);
        for call in calls {
            self.decoding.targets.remove(&call);
        }
        found
    }

    /// The calls `id` results in (see [`Checker::expecting_decode`]).
    fn resulting_calls(&self, id: ExprId) -> Vec<ExprId> {
        let mut calls = Vec::new();
        let mut pending = vec![id];
        while let Some(node) = pending.pop() {
            match self.ast.get(node).kind {
                ExprKind::Call { .. } => calls.push(node),
                ExprKind::Paren(inner) => pending.push(inner),
                ExprKind::Block { items, .. } if self.has_result(items) => {
                    if let Some(&last) = self.ast.items(items).last()
                        && let ItemKind::Result(value) | ItemKind::Expr(value) =
                            self.ast.item(last).kind
                    {
                        pending.push(value);
                    }
                }
                ExprKind::If {
                    then,
                    else_,
                    has_else: true,
                    ..
                } => pending.extend([then, else_]),
                _ => {}
            }
        }
        calls
    }

    /// Types the call `id` of `json.decode`, the callee `callee`, whose
    /// argument is `text` (see [JSON decoding](#json-decoding)): `text` must
    /// be a `String`, and the call is the type expected of it, made present,
    /// whose decoding plan is recorded for the call; `Error` if the argument
    /// is erroneous, no type is expected, or the type has no plan.
    pub(super) fn check_json_decode(&mut self, id: ExprId, callee: ExprId, text: ExprId) -> TyId {
        let callee_span = self.ast.get(callee).span;
        let found = self.check_expr_expecting(
            text,
            TyId::STRING,
            callee_span,
            "`json.decode` decodes the text of a document",
        );
        let span = self.ast.get(id).span;
        let Some(expected) = self.decoding.targets.remove(&id) else {
            self.errors.push(
                Diagnostic::error("`json.decode` needs to know the type to decode as", span)
                    .with_note(
                        "give it where the call stands: `let graph: Graph = json.decode(text)`, \
                         or `json.decode(text): Graph`",
                    ),
            );
            return TyId::ERROR;
        };
        if found == TyId::ERROR || expected == TyId::ERROR {
            return TyId::ERROR;
        }
        let target = self.types.present(expected);
        match self.types.decode_plan(target, self.ast.names()) {
            Ok(plan) => {
                self.decoding.plans.insert(id, plan);
                self.expr_types[callee.index()] = self.types.fn_type(
                    [(None, TyId::STRING)],
                    target,
                    Builtin::JsonDecode.effect(),
                );
                target
            }
            Err(Undecodable { ty, why }) => {
                let shown = self.with_article(ty);
                let mut error = Diagnostic::error(format!("cannot decode JSON as {shown}"), span)
                    .with_note(why);
                if ty != target {
                    error = error.with_note(format!(
                        "it is part of {}, the type expected here",
                        self.with_article(target)
                    ));
                }
                self.errors.push(error);
                TyId::ERROR
            }
        }
    }
}
