//! Checking secrets, `Secret<T>`: its type, what is computed on one, and what
//! one may not decide.
//!
//! # Secrets
//!
//! `Secret<T>` (see the types' [Secrets](crate::types#secrets)) is a value
//! of the present type `T` that never leaks: shown, it is `***`, and
//! nothing that runs or fails tells anything of it. `T` must be present, "a
//! secret's value is present, not `Future<Int>`". `secret(value, identity)`
//! makes one, of a value and a public identity, what a host's receipts
//! record of it; `.unsafe_value()` takes its value, deliberately. Both are
//! the prelude's, over the built-ins `secret` and `secret_value`.
//!
//! What is computed on a secret is a secret. An operator with a secret operand,
//! `s + "!"` or `-n` (`Checker::secret_binary`), a secret interpolated,
//! `"--token={s}"` (`Checker::check_interpolation`), a field of a secret record
//! and an element of a secret list, by a plain index, are the operation on the
//! values, its type the secret of its result's (`Checker::secret_operation`):
//! the operation is recorded for lowering with the type of what it computes
//! ([`Resolution::secret_op`](super::Resolution::secret_op)), and its operands,
//! each secret with the type of its value, unwrapped
//! ([`Resolution::secret_operand`](super::Resolution::secret_operand)), and
//! each other one marked
//! ([`Resolution::is_plain_operand`](super::Resolution::is_plain_operand)), so
//! that a failure of it is told from the operation's own. A refused operation's
//! errors are of the types as written, "cannot add `Secret<String>` and Int"
//! (`Checker::secret_plain`).
//!
//! A comparison of values that hold secrets, in a field, an element, a
//! member or a payload at any depth, a record of a secret included, is the
//! comparison of what they hold, every secret its value
//! ([`TypeTable::without_secrets`]), and a secret, its operands that hold
//! one unwrapped deeply: whether two values are equal tells of the secrets
//! in them just as a comparison of the secrets would.
//!
//! A secret decides nothing: what runs never depends on one. `and` and
//! `or`, which decide whether their right side runs, and `is` and `as`, of a
//! value whose members include a secret, refuse one, "a secret cannot
//! decide what runs" (`Checker::no_branching`); a condition of `if`, a
//! guard and an assertion's condition are `Bool`s, which a secret is not,
//! and a `for` iterates over a list, which a secret is not either. A
//! secret where its value's type is expected is "mismatched types", noted
//! that a secret is not its value and that `.unsafe_value()` takes it.

use crate::ast::{BinaryOp, ExprId};
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::{Laziness, TyId, TypeTable};

use super::Checker;

impl Checker<'_> {
    /// `Secret<T>` of `value`, `T` resolved present, written at `span` (see
    /// [Secrets](#secrets)): `Error` for an erroneous one, and (reported)
    /// for one that may be a future, "a secret's value is present, not
    /// `Future<String>`".
    pub(super) fn secret_type(&mut self, value: TyId, span: Span) -> TyId {
        if value == TyId::ERROR {
            return TyId::ERROR;
        }
        if self.types.laziness(value) != Laziness::Present {
            let shown = self.in_message(value);
            self.errors.push(Diagnostic::error(
                format!("a secret's value is present, not {shown}"),
                span,
            ));
            return TyId::ERROR;
        }
        self.types.secret(value)
    }

    /// Types the binary operator `op` of `id`, `lhs` of type `lhs_ty` and
    /// `rhs` of type `rhs_ty`, if either is a secret, or, for a comparison,
    /// holds one (see [Secrets](#secrets)): the operator on the values, its
    /// result a secret, the operation and its secret operands recorded for
    /// lowering. A comparison compares what its operands hold with every
    /// secret its value, so an operand holding one is unwrapped deeply.
    /// `&&` and `||`, which decide whether their right side runs, refuse
    /// one. `None` if neither is a secret, nor for a comparison holds one.
    pub(super) fn secret_binary(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        let compares = op.is_equality() || op.is_comparison();
        let holds = |types: &TypeTable, ty: TyId| {
            types.secret_value(types.strip(ty)).is_some()
                || (compares && types.holds_secret(types.strip(ty)))
        };
        if !holds(self.types, lhs_ty) && !holds(self.types, rhs_ty) {
            return None;
        }
        let (lhs_ty, rhs_ty) = (self.measure(lhs, lhs_ty), self.measure(rhs, rhs_ty));
        let values = (
            self.types.secret_value(lhs_ty),
            self.types.secret_value(rhs_ty),
        );
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let secret = if values.0.is_some() { lhs } else { rhs };
            self.no_branching(secret, &format!("`{}`", op.verb()));
            return Some(TyId::ERROR);
        }
        if !compares {
            let unwrapped = (values.0.unwrap_or(lhs_ty), values.1.unwrap_or(rhs_ty));
            let found = self.secret_plain(
                id,
                op,
                (lhs, lhs_ty, unwrapped.0),
                (rhs, rhs_ty, unwrapped.1),
            );
            return Some(self.secret_result(id, found, [(lhs, values.0), (rhs, values.1)]));
        }
        // Values holding secrets compare as the values they hold.
        let [lhs_value, rhs_value] = [lhs_ty, rhs_ty].map(|ty| self.types.without_secrets(ty));
        let found = self.secret_plain(id, op, (lhs, lhs_ty, lhs_value), (rhs, rhs_ty, rhs_value));
        let held = |ty: TyId, value: TyId| self.types.holds_secret(ty).then_some(value);
        let operands = [
            (lhs, held(lhs_ty, lhs_value)),
            (rhs, held(rhs_ty, rhs_value)),
        ];
        Some(self.secret_result_deep(id, found, operands))
    }

    /// [`Checker::plain_binary`] of `id`'s `op` on what `lhs` and `rhs`,
    /// each with its type and the type it is unwrapped to, hold once
    /// unwrapped; if that is refused, its errors are of the types as
    /// written, `Secret<String>` where a secret is, rather than of what
    /// they hold, as long as those are refused too.
    pub(super) fn secret_plain(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty, lhs_value): (ExprId, TyId, TyId),
        (rhs, rhs_ty, rhs_value): (ExprId, TyId, TyId),
    ) -> TyId {
        let before = self.errors.len();
        let found = self.plain_binary(id, op, (lhs, lhs_value), (rhs, rhs_value));
        if found != TyId::ERROR || self.errors.len() == before {
            return found;
        }
        let unwrapped = self.errors.split_off(before);
        if self.plain_binary(id, op, (lhs, lhs_ty), (rhs, rhs_ty)) != TyId::ERROR {
            self.errors.truncate(before);
            self.errors.extend(unwrapped);
        }
        TyId::ERROR
    }

    /// The type of `id`, an operation on secrets whose value is of type
    /// `found` (see [Secrets](#secrets)): its secret, recorded for lowering
    /// with its operands that are secrets, each with its value's type;
    /// `found` itself if it is erroneous or `Never`.
    pub(super) fn secret_result<const N: usize>(
        &mut self,
        id: ExprId,
        found: TyId,
        operands: [(ExprId, Option<TyId>); N],
    ) -> TyId {
        let operands =
            operands.map(|(operand, value)| (operand, value.map(|value| (value, false))));
        self.secret_operation(id, found, &operands)
    }

    /// [`Checker::secret_result`], for a comparison: each operand that is
    /// or holds a secret, with the type of what it holds once every secret
    /// in it is its value, unwrapped deeply.
    pub(super) fn secret_result_deep(
        &mut self,
        id: ExprId,
        found: TyId,
        operands: [(ExprId, Option<TyId>); 2],
    ) -> TyId {
        let operands = operands.map(|(operand, value)| (operand, value.map(|value| (value, true))));
        self.secret_operation(id, found, &operands)
    }

    /// Records `id`, an operation on secrets whose value is of type
    /// `found`, and its `operands`, each that is or holds a secret with the
    /// type it is unwrapped to and whether deeply, for lowering; its type,
    /// the secret of `found`, or `found` itself if it is erroneous or
    /// `Never` (see [Secrets](#secrets)).
    pub(super) fn secret_operation(
        &mut self,
        id: ExprId,
        found: TyId,
        operands: &[(ExprId, Option<(TyId, bool)>)],
    ) -> TyId {
        if found == TyId::ERROR || found == self.never {
            return found;
        }
        self.lowering.secret_ops.insert(id, found);
        for &(operand, unwrapped) in operands {
            if let Some(unwrapped) = unwrapped {
                self.lowering.secret_operands.insert(operand, unwrapped);
            } else {
                self.lowering.plain_operands.insert(operand);
            }
        }
        self.types.secret(found)
    }

    /// Reports `operand`, a secret, where it would decide what runs, by
    /// `what`: "a secret cannot decide what runs" (see [Secrets](#secrets)).
    pub(super) fn no_branching(&mut self, operand: ExprId, what: &str) {
        self.errors.push(
            Diagnostic::error(
                "a secret cannot decide what runs",
                self.ast.get(operand).span,
            )
            .with_note(format!(
                "what {what} does would tell the secret to whoever sees what runs"
            )),
        );
    }
}
