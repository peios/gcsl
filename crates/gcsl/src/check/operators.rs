//! Checking operators: arithmetic, comparison, equality, logic and ranges, on
//! numbers, quantities and strings.
//!
//! # Equality
//!
//! `==` and `!=` are *typed* equality, and `===` and `!==` *total*
//! equality. Both kinds need operands of one identical type. On scalars
//! they are the same. On records, typed equality compares only the fields
//! of the operands' static type, and of the static types of their nested
//! records, so after `let q: {x: Int} = { x = 1; y = 2 }`, `q == { x = 1 }`
//! is `true`; total equality compares whole runtime values, deeply and by
//! name, so `q === { x = 1 }` is `false`. The checker records nothing for
//! either: lowering reads the operands' type from the type table.
//!
//! Functions have no equality: once the operands are found to have one
//! identical type, a type that is a function type, or a record or list type
//! containing one at any depth (see
//! [`TypeTable::contains_fn`](crate::types::TypeTable::contains_fn)), is an
//! error. Nor do they have an order. Lists compare as records do, element by
//! element (see [Lists](crate::check::lists#lists)). A union has no static type
//! to compare typed by: `==` and `!=` of two operands of one union type are
//! "cannot compare `Int | String` with `==`", with the note "narrow it, or
//! compare with `===`", while `===` and `!==` compare their runtime values
//! whole; nor does a union have an order (see
//! [Unions](crate::check::narrowing#unions)).
//!
//! # Quantities
//!
//! `Duration` and `Size` are *quantities*: a duration of whole
//! nanoseconds and a size of whole bytes, each an `i64` of its smallest
//! unit ([`TyId::DURATION`], [`TyId::SIZE`]). A duration literal, `30s` or
//! `1h30m`, is a `Duration`, and a size literal, `2GiB`, a `Size` (see the
//! lexer's literal grammar, `Lexer::quantity`); each is a constant, negated
//! or not, so it may be a parameter's or a field's default. Each is a type
//! of its own: it is a subtype only of itself, mixes with no other type
//! (`Int`, `Float` and the other quantity included), and nothing converts
//! to it, so `0` is no duration and an integer literal never adapts to
//! one. Their names are built-in type names, so a `type` may not be named
//! `Duration` or `Size`.
//!
//! Its operators (`Checker::arithmetic`, `Checker::integer_only`), `Q`
//! being either quantity:
//!
//! | expression                | type   |
//! |---------------------------|--------|
//! | `Q + Q`, `Q - Q`, `-Q`    | `Q`    |
//! | `Q * Int`, `Int * Q`      | `Q`    |
//! | `Q // Int`                | `Q`    |
//! | `Q // Q`                  | `Int`  |
//! | `Q % Q`                   | `Q`    |
//! | `Q < Q` and the others    | `Bool` |
//! | `Q == Q`, `Q != Q`        | `Bool` |
//!
//! An untyped integer beside a quantity is an `Int`. Any other mix is the
//! operator's usual error, naming both operands, as in "cannot add
//! Duration and Int"; `/` is "cannot divide Duration and Int", noted "use
//! `//`; a Duration is a whole number of nanoseconds", and a `//` or `%`
//! that does not fit is noted with what a quantity divides by. Comparing
//! two of one type is as for `Int`, and comparing a quantity with anything
//! else is the mismatch. A quantity is shown as its canonical literal
//! wherever a value is (see [`crate::value::write_quantity`]): in an
//! interpolation, by `show` and in a type's default. Lowering computes on
//! the amounts with the `Int` nodes, giving a quantity's result its unit
//! (see [`crate::ir`], "Quantities").
//!
//! # Ranges
//!
//! `a..b` is the list of the integers from `a` up to but not including
//! `b`, and `a..=b` up to and including `b`. Both bounds are measured, as
//! the operands of any measuring operator are (`Checker::measure`), and
//! must be `Int`, an untyped integer being one, which is coerced: "a
//! range's bounds must be Int, found T", at the bound. The range is a
//! present `List<Int>`, and pure. That a range used as a value may not be
//! too long is a runtime matter (see [`crate::eval`]).

use crate::ast::{BinaryOp, ExprId, UnaryOp};
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::{TyId, TyKind};
use crate::value::Quantity;

use super::{Checker, is_int, is_numeric, is_quantity};

impl Checker<'_> {
    /// Types the binary operator `op` of expression `id` applied to `lhs`
    /// and `rhs`, of types `lhs_ty` and `rhs_ty`: each operand is awaited
    /// at the top if it may be a future, or deeply for an equality, and the
    /// operator typed on what they are once awaited.
    pub(super) fn binary(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if let Some(found) = self.secret_binary(id, op, (lhs, lhs_ty), (rhs, rhs_ty)) {
            return found;
        }
        self.plain_binary(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
    }

    /// [`Checker::binary`], where neither operand is a secret.
    pub(super) fn plain_binary(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if op.is_equality() {
            return self.equality(id, op, (lhs, lhs_ty), (rhs, rhs_ty));
        }
        let lhs_ty = self.measure(lhs, lhs_ty);
        let rhs_ty = self.measure(rhs, rhs_ty);
        // The right operand of `and` and `or` may not run, so only the left
        // makes them `Never`.
        let logic = matches!(op, BinaryOp::And | BinaryOp::Or);
        if lhs_ty == self.never || (rhs_ty == self.never && !logic) {
            return self.unreached_operands(lhs, rhs);
        }
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                self.arithmetic(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
            }
            BinaryOp::Div => self.division(id, (lhs, lhs_ty), (rhs, rhs_ty)),
            BinaryOp::FloorDiv | BinaryOp::Rem => {
                self.integer_only(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
            }
            BinaryOp::And | BinaryOp::Or => {
                let expr = self.ast.get(id);
                let op_span = expr.operator_span().unwrap_or(expr.span);
                self.logic_operand(op.verb(), op_span, lhs, lhs_ty);
                self.logic_operand(op.verb(), op_span, rhs, rhs_ty);
                if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
                    TyId::ERROR
                } else {
                    TyId::BOOL
                }
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Same
            | BinaryOp::NotSame
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => self.comparison(id, op, (lhs, lhs_ty), (rhs, rhs_ty)),
            BinaryOp::Range | BinaryOp::RangeInclusive => {
                let fits =
                    [(lhs, lhs_ty), (rhs, rhs_ty)].map(|(bound, ty)| self.range_bound(bound, ty));
                if fits.contains(&false) {
                    TyId::ERROR
                } else {
                    self.types.list(TyId::INT)
                }
            }
        }
    }

    /// Checks `bound`, of type `ty` once measured, a bound of a range, which
    /// must be an `Int`, an untyped integer being one: "a range's bounds
    /// must be Int, found T" otherwise. Returns whether it is one.
    pub(super) fn range_bound(&mut self, bound: ExprId, ty: TyId) -> bool {
        match ty {
            TyId::UNTYPED_INT => {
                self.coerce(bound, TyId::INT);
                true
            }
            TyId::INT => true,
            TyId::ERROR => false,
            _ => {
                let shown = self.in_message(ty);
                let label = format!("this is {}", self.in_label(ty));
                let mut error = Diagnostic::error(
                    format!("a range's bounds must be Int, found {shown}"),
                    self.ast.get(bound).span,
                )
                .with_label(label);
                if let Some(note) = self.union_note(bound, ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
                false
            }
        }
    }

    /// Types the equality `op` of expression `id`: futures have no equality,
    /// so a type with a `Future` anywhere below its top is an error; and
    /// otherwise each operand is awaited deeply if it needs to be, and the
    /// comparison typed on what they are once awaited.
    pub(super) fn equality(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        // A future at the top is awaited as for any operator; one in a field
        // is not, since it is part of the value compared.
        let with_future = [lhs_ty, rhs_ty]
            .into_iter()
            .map(|ty| self.types.strip(ty))
            .find(|&ty| ty != TyId::ERROR && self.types.contains_future(ty));
        if let Some(ty) = with_future {
            self.settle(lhs, lhs_ty);
            self.settle(rhs, rhs_ty);
            let expr = self.ast.get(id);
            let span = expr.operator_span().unwrap_or(expr.span);
            let lhs_label = format!("this is {}", self.in_label(self.types.strip(lhs_ty)));
            let rhs_label = format!("this is {}", self.in_label(self.types.strip(rhs_ty)));
            let message = format!(
                "cannot compare {}: it contains a future",
                self.in_message(ty)
            );
            self.errors.push(
                Diagnostic::error(message, span)
                    .with_context(self.ast.get(lhs).span, lhs_label)
                    .with_context(self.ast.get(rhs).span, rhs_label)
                    .with_note("futures have no equality; await the future and compare its value"),
            );
            return TyId::ERROR;
        }
        let lhs_ty = self.measure_deep(lhs, lhs_ty);
        let rhs_ty = self.measure_deep(rhs, rhs_ty);
        if lhs_ty == self.never || rhs_ty == self.never {
            return self.unreached_operands(lhs, rhs);
        }
        self.comparison(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
    }

    /// The type of a binary operator one of whose operands, `lhs` and
    /// `rhs`, which always runs, is of type `Never`: `Never`, the operator
    /// never being applied, so asking nothing of either operand; an untyped
    /// integer among them settles to `Int`.
    pub(super) fn unreached_operands(&mut self, lhs: ExprId, rhs: ExprId) -> TyId {
        for operand in [lhs, rhs] {
            let ty = self.expr_types[operand.index()];
            self.settle(operand, ty);
        }
        self.never
    }

    /// Types the prefix operator `op`, at `op_span`, applied to `operand`
    /// of type `operand_ty`. Negation accepts every numeric type and
    /// preserves it; `not` needs a `Bool`; both await an operand that may be
    /// a future first. `await` awaits its operand fully, and its type is
    /// the operand's stripped of every future layer at its top; on an
    /// operand that is present it does nothing.
    pub(super) fn unary(
        &mut self,
        (id, op): (ExprId, UnaryOp),
        op_span: Span,
        (operand, operand_ty): (ExprId, TyId),
    ) -> TyId {
        // An operand of type `Never` never gives the operator a value.
        if operand_ty == TyId::ERROR || operand_ty == self.never {
            return operand_ty;
        }
        if op == UnaryOp::Await {
            return self.explicit_await(op_span, operand, operand_ty);
        }
        let operand_ty = self.measure(operand, operand_ty);
        // A secret's negation is a secret (see [Secrets](crate::check::secrets#secrets)).
        if let Some(value) = self.types.secret_value(operand_ty) {
            let found = self.unary((id, op), op_span, (operand, value));
            return self.secret_result(id, found, [(operand, Some(value))]);
        }
        match op {
            UnaryOp::Neg if is_numeric(operand_ty) || is_quantity(operand_ty) => operand_ty,
            UnaryOp::Neg => {
                let message = format!("cannot {} {}", op.verb(), self.in_message(operand_ty));
                let label = self.in_label(operand_ty);
                let mut error = Diagnostic::error(message, op_span)
                    .with_context(self.ast.get(operand).span, label);
                if let Some(note) = self.union_note(operand, operand_ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
                TyId::ERROR
            }
            UnaryOp::Not => {
                self.logic_operand(op.verb(), op_span, operand, operand_ty);
                TyId::BOOL
            }
            // Handled above.
            UnaryOp::Await => operand_ty,
        }
    }

    /// Types `+`, `-` and `*`: both operands must end up the same numeric
    /// type, with untyped operands adapting to a concrete one, except that
    /// `+` also concatenates two strings, and that a quantity is added to and
    /// subtracted from one of its type and multiplied by an `Int` on either
    /// side, giving its type (see [Quantities](#quantities)).
    pub(super) fn arithmetic(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if op == BinaryOp::Add && lhs_ty == TyId::STRING && rhs_ty == TyId::STRING {
            return TyId::STRING;
        }
        match (op, lhs_ty, rhs_ty) {
            (BinaryOp::Add | BinaryOp::Sub, quantity, other)
                if is_quantity(quantity) && quantity == other =>
            {
                return quantity;
            }
            (BinaryOp::Mul, quantity, count) if is_quantity(quantity) && is_int(count) => {
                self.coerce(rhs, TyId::INT);
                return quantity;
            }
            (BinaryOp::Mul, count, quantity) if is_int(count) && is_quantity(quantity) => {
                self.coerce(lhs, TyId::INT);
                return quantity;
            }
            _ => {}
        }
        if let Some(ty) = self.numeric((lhs, lhs_ty), (rhs, rhs_ty)) {
            return ty;
        }
        let message = format!(
            "cannot {} {} and {}",
            op.verb(),
            self.in_message(lhs_ty),
            self.in_message(rhs_ty)
        );
        self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
        TyId::ERROR
    }

    /// Decides the common numeric type of two expressions that must end up
    /// the same numeric type, as the operands of arithmetic must: untyped
    /// ones adapt to a concrete `Int` or `Float`, and two untyped ones stay
    /// untyped. Returns `Some(TyId::ERROR)` without a report if either is
    /// already an error, and `None` if they cannot agree, having settled any
    /// untyped one to `Int` for the caller's report.
    pub(super) fn numeric(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        match (lhs_ty, rhs_ty) {
            // An erroneous operand has been reported already. The other
            // operand still needs a concrete type in case it is untyped.
            (TyId::ERROR, _) => {
                self.coerce(rhs, TyId::INT);
                Some(TyId::ERROR)
            }
            (_, TyId::ERROR) => {
                self.coerce(lhs, TyId::INT);
                Some(TyId::ERROR)
            }
            (TyId::UNTYPED_INT, TyId::UNTYPED_INT) => Some(TyId::UNTYPED_INT),
            (TyId::UNTYPED_INT, concrete @ (TyId::INT | TyId::FLOAT)) => {
                self.coerce(lhs, concrete);
                Some(concrete)
            }
            (concrete @ (TyId::INT | TyId::FLOAT), TyId::UNTYPED_INT) => {
                self.coerce(rhs, concrete);
                Some(concrete)
            }
            (TyId::INT, TyId::INT) => Some(TyId::INT),
            (TyId::FLOAT, TyId::FLOAT) => Some(TyId::FLOAT),
            _ => {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                None
            }
        }
    }

    /// Types a comparison. Both operands must be the same type, untyped
    /// operands adapting as for arithmetic and two untyped operands
    /// settling to `Int`: any type without functions in it for `===` and
    /// `!==` (two records, two lists, or two unions, must be exactly the same
    /// type), and any such type but a union for `==` and `!=`, a union being
    /// "cannot compare `Int | String` with `==`", and `Int`, `Float` or
    /// `String` for ordering. The result is `Bool`.
    pub(super) fn comparison(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        let ordering = !op.is_equality();
        if ordering && lhs_ty != TyId::ERROR && rhs_ty != TyId::ERROR {
            // An operand of a type with no order is reported as such, before
            // any mismatch.
            let unordered = [lhs_ty, rhs_ty].into_iter().find(|&ty| {
                ty == TyId::BOOL
                    || ty == TyId::FILE
                    || ty == TyId::DIR
                    || self.types.is_record(ty)
                    || self.types.list_element(ty).is_some()
                    || self.types.fn_sig(ty).is_some()
                    || self.types.union_members(ty).is_some()
            });
            if let Some(unordered) = unordered {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                let message = format!("cannot order {}", self.in_message(unordered));
                self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
                return TyId::ERROR;
            }
        }
        match self.agree((lhs, lhs_ty), (rhs, rhs_ty)) {
            Some(TyId::ERROR) => TyId::ERROR,
            // Two untyped literals compare as Int.
            Some(TyId::UNTYPED_INT) => {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                TyId::BOOL
            }
            // Functions have no equality, even inside records.
            Some(ty) if self.types.contains_fn(ty) => {
                let unbounded = matches!(self.types.kind(ty), &TyKind::Enum { key, .. }
                    if self.types.enum_bound(key).is_none());
                let message = if unbounded {
                    format!(
                        "cannot compare {}: its variants' payloads are not bounded, so they may \
                         hold functions",
                        self.in_message(ty)
                    )
                } else if self.types.fn_sig(ty).is_some() {
                    "cannot compare functions".to_owned()
                } else {
                    format!(
                        "cannot compare {}: it contains a function",
                        self.in_message(ty)
                    )
                };
                let expr = self.ast.get(id);
                let span = expr.operator_span().unwrap_or(expr.span);
                let label = format!("this is {}", self.in_label(ty));
                self.errors.push(
                    Diagnostic::error(message, span)
                        .with_context(self.ast.get(lhs).span, label.as_str())
                        .with_context(self.ast.get(rhs).span, label),
                );
                TyId::ERROR
            }
            // Typed equality compares by the static type, which a union's
            // value does not have.
            Some(ty)
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
                    && self.types.union_members(ty).is_some() =>
            {
                let (symbol, total) = if op == BinaryOp::Eq {
                    ("==", "===")
                } else {
                    ("!=", "!==")
                };
                let message = format!("cannot compare {} with `{symbol}`", self.in_message(ty));
                let label = format!("this is {}", self.in_label(ty));
                let expr = self.ast.get(id);
                let span = expr.operator_span().unwrap_or(expr.span);
                self.errors.push(
                    Diagnostic::error(message, span)
                        .with_context(self.ast.get(lhs).span, label.as_str())
                        .with_context(self.ast.get(rhs).span, label)
                        .with_note(format!("narrow it, or compare with `{total}`")),
                );
                TyId::ERROR
            }
            Some(_) => TyId::BOOL,
            None => {
                let message = format!(
                    "cannot {} {} and {}",
                    op.verb(),
                    self.in_message(lhs_ty),
                    self.in_message(rhs_ty)
                );
                self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
                TyId::ERROR
            }
        }
    }

    /// Decides the common type of two expressions that must have the same
    /// type: `Bool`, `String`, `Duration`, `Size`, record, list and function
    /// types and unions must match exactly, and numeric types agree as for
    /// [`Checker::numeric`], whose results this returns.
    pub(super) fn agree(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        // Two types that differ only in their fields' defaults are one (see
        // "Field defaults" under [Functions](crate::check::functions#functions)).
        if self.types.same(lhs_ty, rhs_ty)
            && (lhs_ty == TyId::BOOL
                || lhs_ty == TyId::STRING
                || lhs_ty == TyId::FILE
                || lhs_ty == TyId::DIR
                || is_quantity(lhs_ty)
                || self.types.is_record(lhs_ty)
                || self.types.list_element(lhs_ty).is_some()
                || self.types.fn_sig(lhs_ty).is_some()
                || self.types.union_members(lhs_ty).is_some()
                || matches!(self.types.kind(lhs_ty), TyKind::Enum { .. }))
        {
            Some(lhs_ty)
        } else {
            self.numeric((lhs, lhs_ty), (rhs, rhs_ty))
        }
    }

    /// Checks that `operand`, of type `ty`, of the logic operator `verb` at
    /// `op_span`, is a `Bool`.
    pub(super) fn logic_operand(&mut self, verb: &str, op_span: Span, operand: ExprId, ty: TyId) {
        let why = format!("`{verb}` requires Bool operands");
        self.expect(operand, ty, TyId::BOOL, op_span, &why);
    }

    /// Types `/`: it accepts any mix of numeric operands and always yields
    /// `Float`. Untyped operands become `Float` literals; concrete `Int`
    /// operands are converted by lowering.
    pub(super) fn division(
        &mut self,
        id: ExprId,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        self.coerce(lhs, TyId::FLOAT);
        self.coerce(rhs, TyId::FLOAT);
        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        if !is_numeric(lhs_ty) || !is_numeric(rhs_ty) {
            let message = format!(
                "cannot {} {} and {}",
                BinaryOp::Div.verb(),
                self.in_message(lhs_ty),
                self.in_message(rhs_ty)
            );
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            // A quantity is whole: it divides only with `//`.
            if let Some(quantity) = [lhs_ty, rhs_ty].into_iter().find(|&ty| is_quantity(ty))
                && let Some(error) = self.errors.last_mut()
            {
                let quantity = quantity.quantity().unwrap_or(Quantity::Duration);
                error.notes.push(format!(
                    "use `//`; a {} is a whole number of {}",
                    quantity.type_name(),
                    quantity.smallest()
                ));
            }
            return TyId::ERROR;
        }
        TyId::FLOAT
    }

    /// Reports an operator rejecting its operands: the message points at the
    /// operator, and each operand is labelled with its type; an operand that
    /// is a union adds the note saying to tell its members apart first (see
    /// [`Checker::union_note`]).
    pub(super) fn operand_error(
        &mut self,
        id: ExprId,
        message: String,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) {
        let expr = self.ast.get(id);
        let span = expr.operator_span().unwrap_or(expr.span);
        let (lhs_label, rhs_label) = (self.in_label(lhs_ty), self.in_label(rhs_ty));
        let mut error = Diagnostic::error(message, span)
            .with_context(self.ast.get(lhs).span, lhs_label)
            .with_context(self.ast.get(rhs).span, rhs_label);
        for (operand, ty) in [(lhs, lhs_ty), (rhs, rhs_ty)] {
            if let Some(note) = self.union_note(operand, ty)
                && !error.notes.contains(&note)
            {
                error = error.with_note(note);
            }
        }
        self.errors.push(error);
    }

    /// Types `//` and `%`: both operands must be integers, and the result is
    /// a concrete `Int`. Untyped operands are coerced to `Int`; they never
    /// adapt to `Float`, since these operators do not accept it. A quantity
    /// divided by an `Int` with `//` is of its type, one divided by one of
    /// its type with `//` an `Int`, and the remainder of two of a type with
    /// `%` of their type (see [Quantities](#quantities)).
    pub(super) fn integer_only(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if is_quantity(lhs_ty) {
            match op {
                BinaryOp::FloorDiv if is_int(rhs_ty) => {
                    self.coerce(rhs, TyId::INT);
                    return lhs_ty;
                }
                BinaryOp::FloorDiv if rhs_ty == lhs_ty => return TyId::INT,
                BinaryOp::Rem if rhs_ty == lhs_ty => return lhs_ty,
                _ => {}
            }
        }
        // A quantity on either side is no `Int` to reject alone: what it
        // takes is said.
        if let Some(quantity) = [lhs_ty, rhs_ty].into_iter().find_map(TyId::quantity) {
            self.coerce(lhs, TyId::INT);
            self.coerce(rhs, TyId::INT);
            if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
                return TyId::ERROR;
            }
            let message = format!(
                "cannot {} {} and {}",
                op.verb(),
                self.in_message(lhs_ty),
                self.in_message(rhs_ty)
            );
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            let name = quantity.type_name();
            if let Some(error) = self.errors.last_mut() {
                error.notes.push(format!(
                    "a {name} divides with `//` by an Int, giving a {name}, or by a {name}, \
                     giving an Int, and `%` gives the remainder of one {name} by another"
                ));
            }
            return TyId::ERROR;
        }
        // Whatever the outcome, untyped operands settle to Int: these
        // operators never accept Float, so nothing else could decide them.
        self.coerce(lhs, TyId::INT);
        self.coerce(rhs, TyId::INT);
        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        let rejected = [lhs_ty, rhs_ty]
            .into_iter()
            .find(|&ty| !matches!(ty, TyId::INT | TyId::UNTYPED_INT));
        if let Some(rejected) = rejected {
            let message = format!("cannot {} {}", op.verb(), self.in_message(rejected));
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            return TyId::ERROR;
        }
        TyId::INT
    }
}
