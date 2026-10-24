//! Checking lists and strings: list literals and their elements, spreads into
//! lists, indexes and interpolation.
//!
//! # Lists
//!
//! `List<T>`, also written `T[]`, is a container, like a record type (see
//! [`crate::types`]): every rule above that names a record type names a
//! list type too, with its element type a position inside it. So in an
//! impure function's signature `List<Int>` is `List<Int | Future<Int>> |
//! Future<List<Int | Future<Int>>>`; a pure function's parameter written as
//! a list type is inferred, a bare list result position is inferred from
//! the body, its element type from the element type of what the body
//! returns, and a callback's parameter position written as a list type is
//! narrowable; and a memoised function's list parameter is present
//! throughout, so an argument whose elements may be futures is awaited
//! deeply at the call. What looks through a record's fields for futures
//! and provisional types looks through a list's element type, so an
//! interpolated list and an operand of an equality are awaited deeply,
//! element by element.
//!
//! **Literals.** A list literal with no expected type is the list of the join
//! of its elements' types (see
//! [`TypeTable::join`](crate::types::TypeTable::join)), in order, a spread
//! contributing its list's element type; untyped integers adapt as an `if`'s
//! branches do, so `[1, 2.5]` is `List<Float>` and `[1, 2]` `List<Int>`, and
//! two element types that differ measure the provisional parameters in them, as
//! an `if`'s branches do (see [Inferred
//! parameters](crate::check::inference#inferred-parameters)). Elements that do
//! not agree make a union, which is the warning "the elements of this list have
//! different types", at the first element that widens the join of those before
//! it (see [Unions](crate::check::narrowing#unions)); `[]` has nothing to join,
//! and is "cannot infer the element type of `[]`". Nothing is awaited: `[1,
//! lazy 2]` is `List<Int | Future<Int>>`.
//!
//! A literal where a list type `List<X>` (or its either shape) is expected
//! has each element checked against `X` as an argument is against its
//! parameter: an untyped integer adapts, an element that fits `X` once
//! awaited is awaited, subject to the purity rule, and anything else is a
//! mismatch. The literal's type is then the list of the join of what its
//! elements are once those awaits have run (`Checker::fit`: what the
//! evaluator's typed walkers will find in it), a subtype of `List<X>`, so
//! that an inferred result sees what the list really holds; `[]` there is
//! the list of `X` made present, where that fits `X`, since no element of it
//! is a future. The expected type reaches a literal where it reaches a
//! record block (an annotation, an argument, a function's body, a field of
//! an expected record type, a `:=`, an ascription) through parentheses, and,
//! for a list type or a union only, also through a block's result and the
//! branches of an `if` with `else`, so that a function's body, a block,
//! reaches its literal; it reaches a `for` in the same way (see
//! [For](crate::check::loops#for)); such a block or `if` has the type of what its result, or
//! the join of what its branches, are once awaited, or the expected type
//! if that join is not a subtype of it. Under an expected type, a join is
//! never a warning, since each part fits what is expected. Where a union
//! is expected, a list literal or a `for` is checked against its one list
//! member, and `[]` against any (`Checker::expected_member`). It never
//! flows from one branch of an `if` to the other. A record type expected
//! reaches neither a block's result nor an `if`'s branches, but for a
//! record type with defaults, which reaches a record literal there for its
//! defaults alone (see "Field defaults" under [Functions](crate::check::functions#functions)).
//!
//! A spread's operand is used as its bits at the top (`Checker::measure`),
//! and must be a list: "cannot spread a value of type T: it is not a list".
//! It is checked with nothing expected, but for a `for` where the elements
//! are expected to be of a record type with defaults, whose body has a
//! record literal it results in given them, as a `for`'s body has where a
//! list of the type is expected of it (`Checker::check_spread`; see "Field
//! defaults"), the rest of the loop checked as ever; and there for a block
//! with a result or an `if` with `else`, whose list literal results, and
//! `for`s, have the record literals among their elements, and their
//! spreads, given them in the same way, the rest of the block, the `if` and
//! the list checked as ever (`Checker::fill_elements`).
//! Its elements are not measured: where an element type is expected, they
//! must fit it as they are, since there is no expression to await for each;
//! a list whose elements would fit only once awaited is a mismatch, with a
//! note saying why.
//!
//! **Indexes.** `e[i]` measures `e` at the top, which must be a list
//! ("cannot index a value of type T: it is not a list"), and `i`, which
//! must be an `Int` ("a list index must be an Int, found T"). Its type is
//! the element type as it is, so the element, not the index, is awaited
//! where it is used as its bits. A position outside the list is a runtime
//! error (see [`crate::eval`]).
//!
//! **Updating an element.** The path of a `:=` may have index segments,
//! `[EXPR]`, among its names, in any number and order (see
//! [`crate::ast::SegmentKind::Index`]):
//!
//! ```text
//! let mut xs = [1, 2, 3]
//! xs[0] := 10                         # [10, 2, 3]
//! let mut conf = { hosts = [{ name = "a"; up = false }] }
//! conf.hosts[0].up := true
//! grid[y][x] := 1
//! ```
//!
//! Which variable a `:=` may rebind is as for any `:=` (see [Mutable
//! variables](crate::check::mutation#mutable-variables)). The walk along the path (see "Dotted
//! `let` and `:=`" above, and `Checker::walk_path`) has one case more: at
//! an index segment `P[EXPR]`, the value at `P` is measured at the top, as
//! a record along a path is (awaited if it may be a future, subject to the
//! purity rule, reported at `P`; a pure function's bare parameter
//! measured), and must then be a list: otherwise "cannot update `xs[0]`:
//! `xs` is Int, not a list", showing the type it had before stripping, the
//! purity error being reported only for a value that is a list once
//! awaited. `EXPR` is then checked and measured, as an index's position is
//! (`Checker::check_position`), an expression of the enclosing function
//! like any other, its purity, captures and awaits as usual: an `Int`, an
//! untyped integer being one, or "a list index must be an Int, found T".
//! The segment's type is the list's element type as it is; a segment after
//! it walks into the element, which is measured in turn, as a record for a
//! name or a list for another index. A name segment on a list, or an index
//! segment on a record, is the matching "not a record" or "not a list"
//! error. Each position is checked exactly once: where the walk reaches it,
//! or on its own if the walk stops before, or the `:=` is rejected, so that
//! its mistakes are still reported.
//!
//! If the last segment is an index, the value must fit the list's element
//! type, as a field's leaf must fit the field's: a subtype, an adapting
//! literal, or an await to fit where one is allowed, labelled "`:=` keeps
//! the element's type" at the path. A `let` path cannot index a list, so a
//! field missing after an index segment, "cannot rebind `xs[0].b`: `xs[0]`
//! has no field `b`", has no note suggesting one.
//!
//! A list's type never changes. The updated type is built from the leaf
//! outwards (`Checker::record_update`), each record along the path present
//! with its segment set, and each list along it present, keeping its
//! element type, which the updated element must be a subtype of
//! (`Checker::updated_list`). It is whenever the element type is present or
//! the either shape; otherwise, as for elements declared exactly
//! `Future<{a: Int}>` of a path continuing through one, it is "cannot
//! rebind `xs[0].a`: the elements of `xs` are `Future<{a: Int}>`, and the
//! update makes one `{a: Int}`", at the path, with a note that a list keeps
//! its element type. A mutable variable keeps its declared type, which the
//! updated type must then fit (`Checker::kept_type`); a variable that is
//! not mutable is rebound with the updated type. `:=` never adds: an index
//! segment does not create an element, and its position is not checked
//! statically. A rejected `:=` with an index segment declares its name as
//! `Error`, rather than walking its path to find an updated type, which
//! would check its positions again.
//!
//! **Equality.** Lists compare as records do (see [Equality](crate::check::operators#equality)),
//! when their types are identical, contain no function and no `Future`:
//! element by element, typed equality comparing records inside by their
//! static type. They have no order, and no arithmetic: `+` does not
//! concatenate them, a spread does.

use crate::ast::{Element, ElementRange, ExprId, PartRange, StrPart};
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::TyId;

use super::{Checker, Stated};

impl Checker<'_> {
    /// Types an interpolated string: each interpolated expression is
    /// checked, an untyped one settles to `Int`, and one that may be a
    /// future, or hold one in a field, is awaited deeply. The result is a
    /// `String`, or an error, without a further report, if a part is one.
    pub(super) fn check_interpolation(&mut self, id: ExprId, parts: PartRange) -> TyId {
        let mut ty = TyId::STRING;
        // The expressions interpolated, each secret with its value's type:
        // if one is a secret, the text is (see [Secrets](crate::check::secrets#secrets)).
        let mut operands: Vec<(ExprId, Option<(TyId, bool)>)> = Vec::new();
        for &part in self.ast.parts(parts) {
            let StrPart::Expr(expr) = part else {
                continue;
            };
            let found = self.check_expr(expr);
            // A value of every type can be shown as text, a record in its
            // `repr` form, but one of a type parameter, which may be a
            // future that nothing awaits (see [Generics](crate::check::methods#generics)).
            let found = self.settle(expr, found);
            if self.types.has_param(found) {
                let shown = self.in_message(found);
                self.errors.push(Diagnostic::error(
                    format!("cannot interpolate a value of type {shown}, of a type parameter"),
                    self.ast.get(expr).span,
                ));
                ty = TyId::ERROR;
                continue;
            }
            match self.measure_deep(expr, found) {
                TyId::ERROR => ty = TyId::ERROR,
                // A value of type `Never` is never there to show.
                found if found == self.never && ty != TyId::ERROR => ty = self.never,
                found => {
                    let value = self.types.secret_value(found);
                    operands.push((expr, value.map(|value| (value, false))));
                }
            }
        }
        if ty != TyId::STRING || operands.iter().all(|&(_, value)| value.is_none()) {
            return ty;
        }
        self.secret_operation(id, TyId::STRING, &operands)
    }

    /// Types the list literal `id`, whose elements are `elements`, with no
    /// expected type (see the module documentation): each element is
    /// checked, a spread's operand being measured and having to be a list,
    /// and the literal is the list of the join of the elements' types, in
    /// order, a spread contributing its list's element type, untyped
    /// integers adapting as an `if`'s branches do. A join that is widened,
    /// a union of element types that do not agree, is a warning, at the
    /// first element whose type widens the join of those before it (see
    /// [`Checker::elements_widened`]). An empty literal has nothing to join,
    /// "cannot infer the element type of `[]`". `Error` for that, and for a
    /// literal with an erroneous element. With `fill`, a record type with
    /// defaults that the elements are expected to be of, with where that is
    /// stated and its label, each element that is no spread is checked for
    /// the defaults alone ([`Checker::fill_result`]), and each spread's
    /// operand as [`Checker::check_spread`] says with `fill`; nothing else
    /// of the literal changes (see [`Checker::fill_elements`]).
    pub(super) fn check_list(
        &mut self,
        id: ExprId,
        elements: ElementRange,
        fill: Option<Stated<'_>>,
    ) -> TyId {
        let ast = self.ast;
        let list = ast.elements(elements);
        if list.is_empty() {
            // Against an erroneous expected type, which would have typed it,
            // an empty list is no error of its own.
            if self.quiet.expecting_error > 0 {
                self.quiet.failed = true;
            } else {
                self.errors.push(
                    Diagnostic::error("cannot infer the element type of `[]`", ast.get(id).span)
                        .with_note("annotate it, as in `let xs: List<Int> = []`"),
                );
            }
            return TyId::ERROR;
        }
        // Every element is checked, in order, before any is joined.
        let base = self.element_types.len();
        for element in list {
            let ty = match (element.spread, fill) {
                (Some(spread), _) => self
                    .check_spread(element.value, spread, fill)
                    .and_then(|list| self.types.list_element(list))
                    .unwrap_or(TyId::ERROR),
                (None, Some((record, context))) => self.fill_result(element.value, record, context),
                (None, None) => self.check_expr(element.value),
            };
            self.element_types.push(ty);
        }
        let mut joined = None;
        let mut failed = false;
        // The first element that widens the join, with the two types it
        // joined.
        let mut widened = None;
        for (index, element) in list.iter().enumerate() {
            let ty = self
                .element_types
                .get(base + index)
                .copied()
                .unwrap_or(TyId::ERROR);
            if ty == TyId::ERROR {
                failed = true;
            }
            if failed {
                continue;
            }
            let Some(so_far) = joined else {
                joined = Some(ty);
                continue;
            };
            let earlier = list
                .get(..index)
                .unwrap_or_default()
                .iter()
                .filter(|earlier| earlier.spread.is_none())
                .map(|earlier| earlier.value);
            let (ty, widens, sides) = self.join_element((earlier, element.value), so_far, ty);
            joined = Some(ty);
            if widens && widened.is_none() {
                widened = Some((index, sides));
            }
        }
        self.element_types.truncate(base);
        // Whatever has not adapted to a concrete type is an `Int`.
        for element in list.iter().filter(|element| element.spread.is_none()) {
            let ty = self.expr_types[element.value.index()];
            self.settle(element.value, ty);
        }
        let ty = match joined {
            _ if failed => return TyId::ERROR,
            Some(TyId::UNTYPED_INT) => self.types.list(TyId::INT),
            Some(element) => self.types.list(element),
            None => return TyId::ERROR,
        };
        if let Some((index, sides)) = widened {
            self.elements_widened(list, index, sides, ty);
        }
        ty
    }

    /// The join of `so_far`, the join of the types of the expressions
    /// `earlier`, the elements of a list literal but its spreads or the
    /// values a loop collects, with `ty`, the type of the one after them,
    /// whose expression is `value`, a spread's being its list's element
    /// type: the join, whether it is widened, and the two types as they
    /// were joined. As for an `if`'s branches, an untyped integer, or the
    /// untyped values so far, adapt to the other side's `Int` or `Float`
    /// once stripped, or to `Int`; and if the two types differ, each
    /// parameter of a claimable function (see [`Checker::claimable`]) whose
    /// provisional type is in either is measured, conservatively, at the
    /// value.
    pub(super) fn join_element(
        &mut self,
        (earlier, value): (impl Iterator<Item = ExprId>, ExprId),
        so_far: TyId,
        ty: TyId,
    ) -> (TyId, bool, (TyId, TyId)) {
        let (mut so_far, mut ty) = (so_far, ty);
        if so_far == TyId::UNTYPED_INT && ty == TyId::UNTYPED_INT {
            return (so_far, false, (so_far, ty));
        }
        if so_far == TyId::UNTYPED_INT {
            so_far = self.numeric_target(ty);
            for earlier in earlier {
                self.coerce(earlier, so_far);
            }
        }
        if ty == TyId::UNTYPED_INT {
            ty = self.numeric_target(so_far);
            self.coerce(value, ty);
        }
        if so_far == ty {
            return (ty, false, (so_far, ty));
        }
        let so_far = self.claim(value, so_far);
        let ty = self.claim(value, ty);
        let (joined, widened) = self.types.join(so_far, ty);
        (joined, widened, (so_far, ty))
    }

    /// The type an untyped integer adapts to where it must agree with a
    /// value of type `other`: `other`'s `Int` or `Float`, once stripped, or
    /// its union's (`Int` if it has both), or `Int`.
    pub(super) fn numeric_target(&self, other: TyId) -> TyId {
        self.numeric_member(self.types.strip(other))
            .unwrap_or(TyId::INT)
    }

    /// The numeric type of `ty`, a present type: `ty` itself if it is `Int`
    /// or `Float`, and for a union its `Int` member if it has one and its
    /// `Float` member otherwise; `None` if it has neither.
    pub(super) fn numeric_member(&self, ty: TyId) -> Option<TyId> {
        match ty {
            TyId::INT | TyId::FLOAT => Some(ty),
            _ => {
                let members = self.types.union_members(ty)?;
                [TyId::INT, TyId::FLOAT]
                    .into_iter()
                    .find(|numeric| members.contains(numeric))
            }
        }
    }

    /// Warns that element number `index` of `list`, whose type (or, for a
    /// spread, whose list's element type) is `ty`, does not agree with the
    /// type `so_far` of the elements before it, the join of the two being
    /// widened to a union, and the literal being of type `list_ty`: at the
    /// element, labelled with its type, with a label of the elements' so
    /// far on the first, and a note naming the list's type.
    pub(super) fn elements_widened(
        &mut self,
        list: &[Element],
        index: usize,
        (so_far, ty): (TyId, TyId),
        list_ty: TyId,
    ) {
        let (Some(first), Some(element)) = (list.first(), list.get(index)) else {
            return;
        };
        let label = match element.spread {
            Some(_) => format!("its elements are {}", self.in_label(ty)),
            None => format!("this is {}", self.in_label(ty)),
        };
        let context = format!("the elements so far are {}", self.in_label(so_far));
        let note = format!(
            "the list is {}; annotate it with that type if both are meant, or tell them apart \
             with `is`",
            self.in_message(list_ty)
        );
        self.warnings.push(
            Diagnostic::warning(
                "the elements of this list have different types",
                element.span(self.ast),
            )
            .with_label(label)
            .with_context(first.span(self.ast), context)
            .with_note(note),
        );
    }

    /// Checks the operand `value` of the spread whose `...` is at `spread`,
    /// which is used as its bits at the top (see [`Checker::measure`]):
    /// returns the list type it has once measured, or `None`, having
    /// reported "cannot spread a value of type T: it is not a list" for
    /// anything else, or if it is erroneous. With `fill`, a record type with
    /// defaults that the list's elements are expected to be of, with where
    /// that is stated and its label, an operand that is, through any
    /// parentheses, a `for` has the record literal its body results in
    /// given the defaults it lacks ([`Checker::check_for`]), and one that
    /// is a block with a result or an `if` with `else` has the elements of
    /// a list literal it results in, or the body of a `for` it does, checked
    /// so ([`Checker::fill_elements`]); the operand is otherwise checked as
    /// it is with nothing expected, a list literal included.
    pub(super) fn check_spread(
        &mut self,
        value: ExprId,
        spread: Span,
        fill: Option<Stated<'_>>,
    ) -> Option<TyId> {
        let found = match fill {
            Some((record, context))
                if self.loop_within(value).is_some() || self.reaches_result(value) =>
            {
                self.fill_elements(value, record, context)
            }
            _ => self.check_expr(value),
        };
        let found = self.settle(value, found);
        let ty = self.measure(value, found);
        if ty == TyId::ERROR {
            return None;
        }
        if self.types.list_element(ty).is_some() {
            return Some(ty);
        }
        // A list of type `Never` is never there to spread: as a list of
        // `Never`, it adds nothing to the join of the elements.
        if ty == self.never {
            return Some(self.types.list(ty));
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        let span = self.ast.get(value).span;
        let mut error = Diagnostic::error(
            format!("cannot spread a value of type {shown}: it is not a list"),
            Span::join(spread, span),
        )
        .with_context(span, label);
        if let Some(note) = self.union_note(value, ty) {
            error = error.with_note(note);
        }
        self.errors.push(error);
        None
    }

    /// Types a list literal whose elements are `elements` where a value of
    /// type `expected` is expected, a list type `List<X>` once stripped of
    /// its either shape, as stated at `context`, labelled `why` (see the
    /// module documentation): each element is checked against `X` as an
    /// argument is against its parameter, and a spread's list's elements
    /// must fit `X` as they are. The literal is the list of the join of what
    /// its elements are once the awaits marked on them have run (see
    /// [`Checker::fit`]), a subtype of `List<X>`; `List<X>` itself should
    /// they have no join that is one. An empty literal is the list of `X`
    /// made present, where that fits `X`, since it has no element to be a
    /// future, and of `X` otherwise. `Error` if an element does not fit.
    pub(super) fn check_list_expecting(
        &mut self,
        elements: ElementRange,
        expected: TyId,
        (context, why): (Span, &str),
    ) -> TyId {
        let stripped = self.types.strip(expected);
        let Some(element) = self.types.list_element(stripped) else {
            return TyId::ERROR;
        };
        let list = self.ast.elements(elements);
        if list.is_empty() {
            let present = self.types.present(element);
            let element = if self.types.is_subtype(present, element) {
                present
            } else {
                element
            };
            return self.types.list(element);
        }
        let base = self.element_types.len();
        for item in list {
            let ty = match item.spread {
                Some(spread) => self.fit_spread(item.value, spread, element, (context, why)),
                None => self.fit(item.value, element, (context, why)),
            };
            self.element_types.push(ty);
        }
        let end = self.element_types.len();
        // Each element fits what is expected, so a widened join is no
        // warning.
        let mut joined = self.element_types.get(base).copied();
        for at in base + 1..end {
            let value = self.element_types.get(at).copied().unwrap_or(TyId::ERROR);
            joined = joined.map(|so_far| self.types.join(so_far, value).0);
        }
        let failed = self
            .element_types
            .get(base..)
            .unwrap_or_default()
            .contains(&TyId::ERROR);
        self.element_types.truncate(base);
        if failed {
            return TyId::ERROR;
        }
        let joined = joined
            .filter(|&joined| self.types.is_subtype(joined, element))
            .unwrap_or(element);
        self.types.list(joined)
    }

    /// Checks the operand `value` of the spread whose `...` is at `spread`
    /// in a list literal whose elements are expected to be of type
    /// `element`, as stated at `context`, labelled `why`: the operand is
    /// measured at the top and must be a list (see [`Checker::check_spread`])
    /// whose element type fits `element` as it is, once a provisional type
    /// in it where `element` is present is measured. There is no expression
    /// to await for each element, so a list whose elements would fit only
    /// once awaited is a mismatch, whose note says so. Where `element` is a
    /// record type with defaults itself, an operand that is a `for` has the
    /// record literal its body results in given them first, and one that is
    /// a block with a result or an `if` with `else` the record literals
    /// among the elements of a list literal it results in (see
    /// [`Checker::check_spread`]). Returns the list's element type, or
    /// `Error` if it does not fit.
    pub(super) fn fit_spread(
        &mut self,
        value: ExprId,
        spread: Span,
        element: TyId,
        (context, why): (Span, &str),
    ) -> TyId {
        let fill = self
            .defaulted_record(element)
            .map(|record| (record, (context, why)));
        let Some(list) = self.check_spread(value, spread, fill) else {
            return TyId::ERROR;
        };
        let expected = self.types.list(element);
        let list = if self.types.is_subtype(list, expected) {
            list
        } else {
            match self.claim_against(value, list, expected) {
                Some(claimed) if self.types.is_subtype(claimed, expected) => claimed,
                _ => {
                    let found = self.types.list_element(list).unwrap_or(TyId::ERROR);
                    let label = format!(
                        "expected {}, found {}",
                        self.in_label(expected),
                        self.in_label(list)
                    );
                    let mut error = Diagnostic::error("mismatched types", self.ast.get(value).span)
                        .with_label(label)
                        .with_context(context, why);
                    if self.types.needs_await(found, element).is_some() {
                        error = error.with_note(
                            "its elements would fit once awaited, but a spread cannot await them: \
                             there is no expression to await; spread a list of their awaited \
                             values instead",
                        );
                    }
                    self.errors.push(error);
                    return TyId::ERROR;
                }
            }
        };
        self.types.list_element(list).unwrap_or(TyId::ERROR)
    }

    /// Types the index `id`, reading the element of `list` at `index` (see
    /// the module documentation): the list is measured at the top, and must
    /// be a list, or a union of lists (see [`Checker::element_of`]),
    /// "cannot index a value of type T: it is not a list", reported at the
    /// `[INDEX]`; the position is measured, and must be an
    /// `Int`, an untyped integer being one, "a list index must be an Int,
    /// found T". The index's type is the list's element type as it is, which
    /// may be a future or the either shape; `Error` if either is wrong.
    pub(super) fn check_index(&mut self, id: ExprId, list: ExprId, index: ExprId) -> TyId {
        let list_ty = self.check_expr(list);
        let list_ty = self.settle(list, list_ty);
        let list_ty = self.measure(list, list_ty);
        let index_fits = self.check_position(index);
        // A list or a position of type `Never` is never there to read.
        if list_ty == TyId::ERROR || list_ty == self.never {
            return list_ty;
        }
        if self.expr_types[index.index()] == self.never {
            return self.never;
        }
        // A secret list's element is a secret (see [Secrets](crate::check::secrets#secrets)).
        if let Some(value) = self.types.secret_value(list_ty)
            && let Some(element) = self.element_of(value)
        {
            let found = if index_fits { element } else { TyId::ERROR };
            return self.secret_result(id, found, [(list, Some(value)), (index, None)]);
        }
        let Some(element) = self.element_of(list_ty) else {
            let expr = self.ast.get(id);
            let brackets = expr.bracket_span(self.ast).unwrap_or(expr.span);
            let shown = self.in_message(list_ty);
            let label = format!("this is {}", self.in_label(list_ty));
            let mut error = Diagnostic::error(
                format!("cannot index a value of type {shown}: it is not a list"),
                brackets,
            )
            .with_context(self.ast.get(list).span, label);
            if let Some(note) = self.union_note(list, list_ty) {
                error = error.with_note(note);
            }
            self.errors.push(error);
            return TyId::ERROR;
        };
        if index_fits { element } else { TyId::ERROR }
    }

    /// Checks `index`, the position of an index or of an index segment of a
    /// `:=` path, an expression of the enclosing function like any other,
    /// and measures it at the top: it must be an `Int`, an untyped integer
    /// being one, which is coerced, and is otherwise reported, "a list index
    /// must be an Int, found T", at the position. Returns whether it is one;
    /// an erroneous position has been reported already.
    pub(super) fn check_position(&mut self, index: ExprId) -> bool {
        let index_ty = self.check_expr(index);
        let index_ty = self.measure(index, index_ty);
        match index_ty {
            TyId::UNTYPED_INT => {
                self.coerce(index, TyId::INT);
                true
            }
            TyId::INT => true,
            TyId::ERROR => false,
            // A position of type `Never` is never there to use.
            ty if ty == self.never => true,
            _ => {
                let shown = self.in_message(index_ty);
                let label = format!("this is {}", self.in_label(index_ty));
                let mut error = Diagnostic::error(
                    format!("a list index must be an Int, found {shown}"),
                    self.ast.get(index).span,
                )
                .with_label(label);
                if let Some(note) = self.union_note(index, index_ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
                false
            }
        }
    }

    /// The element type of `ty`, a type once measured, if it is a list type,
    /// or a union whose members are all list types: the join of their
    /// element types, which is no warning, since the union says the value
    /// is any of them. `None` for any other type.
    pub(super) fn element_of(&mut self, ty: TyId) -> Option<TyId> {
        if let Some(element) = self.types.list_element(ty) {
            return Some(element);
        }
        let members = self.types.union_members(ty)?.to_vec();
        let mut joined: Option<TyId> = None;
        for member in members {
            let element = self.types.list_element(member)?;
            joined = Some(match joined {
                Some(so_far) => self.types.join(so_far, element).0,
                None => element,
            });
        }
        joined
    }
}
