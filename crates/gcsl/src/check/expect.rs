//! Checking against an expected type: an expression's fit to the type expected
//! of it, the claims and coercions that makes, and the errors it reports.

use crate::ast::{Effect, ExprId, ExprKind, FnId, ForId, ItemKind, ItemRange, LabelId, PathSegment};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::types::{AwaitKind, Laziness, TyId, TyKind};
use crate::value::ValueKind;

use super::{AwaitSite, BodyValue, Branch, Checker, Expecting, NarrowingBlock, Reach};

impl Checker<'_> {
    /// Checks that `expr`, of type `found`, can have type `expected`: an
    /// untyped integer is coerced to `Int` or `Float`, the type of the `Int` or
    /// `Float` expected or of the either shape of one, and any other type must
    /// be a subtype of `expected`, or become one once awaited, in which case
    /// `expr` is marked to be awaited (see
    /// [`TypeTable::needs_await`](crate::types::TypeTable::needs_await)),
    /// subject to the purity rule. A mismatch is reported at `expr` with a
    /// context label on `context`, the source of the expectation.
    ///
    /// Where `found` has a provisional type of the innermost function's at a
    /// position where `expected` is present, that parameter is measured
    /// here, and the test is retried with every provisional type of the
    /// function in `found` made present.
    pub(super) fn expect(
        &mut self,
        expr: ExprId,
        found: TyId,
        expected: TyId,
        context: Span,
        why: &str,
    ) {
        let site = (AwaitSite::Implicit, None);
        self.expect_at(expr, found, expected, (context, why), site);
    }

    /// As [`Checker::expect`], with `site` saying how an await at the top is
    /// reported in a pure function (see [`Checker::await_here`]), and
    /// `filling`, for an argument of a known callee of this input, the
    /// callee's function and the parameter the argument fills, for the
    /// label on a mismatch (see [`Checker::narrowing_label`]).
    pub(super) fn expect_at(
        &mut self,
        expr: ExprId,
        found: TyId,
        expected: TyId,
        (context, why): (Span, &str),
        (site, filling): (AwaitSite, Option<(FnId, usize)>),
    ) {
        match (found, expected) {
            (TyId::ERROR, _) => {}
            // The expected type is unknown and has been reported.
            (_, TyId::ERROR) => {
                self.settle(expr, found);
            }
            (TyId::UNTYPED_INT, TyId::INT | TyId::FLOAT) => self.coerce(expr, expected),
            (TyId::UNTYPED_INT, _) if self.untyped_fits(expected) => {
                let present = self.numeric_target(expected);
                self.coerce(expr, present);
            }
            _ if self.types.is_subtype(found, expected) => {}
            _ => {
                let found = match self.claim_against(expr, found, expected) {
                    Some(claimed) if self.types.is_subtype(claimed, expected) => return,
                    Some(claimed) => claimed,
                    None => found,
                };
                match self.types.needs_await(found, expected) {
                    Some(kind) => {
                        // Only an await at the top is the parameter's doing.
                        let site = match (kind, site) {
                            (AwaitKind::Top, site) => site,
                            (AwaitKind::Deep, _) => AwaitSite::Implicit,
                        };
                        self.await_here(expr, found, kind, site);
                    }
                    None => self.mismatch(expr, (found, expected), (context, why), filling),
                }
            }
        }
    }

    /// The functions, as indices into the stack of functions being checked,
    /// whose parameters a use in the innermost body may measure: the
    /// innermost function, and each function enclosing it that is reached
    /// through pure functions only (see the module documentation), a
    /// memoised function counting as the impure function it is. Empty at
    /// the top level.
    pub(super) fn claimable(&self) -> std::ops::Range<usize> {
        let end = self.functions.len();
        let mut start = end.saturating_sub(1);
        while start > 0
            && self
                .functions
                .get(start)
                .is_some_and(|function| function.effect == Effect::Pure)
        {
            start -= 1;
        }
        start..end
    }

    /// Where `expr`, of type `found`, is expected to have type `expected`, and
    /// does not as it is: measures each parameter of a claimable function (see
    /// [`Checker::claimable`]) whose provisional type sits in `found` where
    /// `expected` is present (see
    /// [`TypeTable::provisionals_at_present`](crate::types::TypeTable::provisionals_at_present)),
    /// and returns `found` with every provisional type of those functions made
    /// present; or `None` if there is no such parameter.
    pub(super) fn claim_against(
        &mut self,
        expr: ExprId,
        found: TyId,
        expected: TyId,
    ) -> Option<TyId> {
        self.claim_against_at(self.ast.get(expr).span, found, expected)
    }

    /// As [`Checker::claim_against`], for a value at `span`, which is where
    /// each parameter is measured: a spread's operand, for a soft field,
    /// which is not an expression of its own.
    pub(super) fn claim_against_at(
        &mut self,
        span: Span,
        found: TyId,
        expected: TyId,
    ) -> Option<TyId> {
        let mut found = found;
        let mut claimed = false;
        for index in self.claimable().rev() {
            let Some(function) = self.functions.get(index).map(|function| function.fn_id) else {
                continue;
            };
            if !self.types.contains_provisional(found, Some(function)) {
                continue;
            }
            let mut params = std::mem::take(&mut self.claimed);
            params.clear();
            self.types
                .provisionals_at_present(found, expected, function, &mut params);
            if !params.is_empty() {
                found = self.make_present(index, span, &params, found);
                claimed = true;
            }
            self.claimed = params;
        }
        claimed.then_some(found)
    }

    /// Where `expr`, of type `ty`, is used as its bits, or must agree with
    /// another type: measures every parameter of a claimable function (see
    /// [`Checker::claimable`]) whose provisional type is in `ty`, at any
    /// depth, and returns `ty` with all of them present. `ty` itself if it
    /// has none.
    pub(super) fn claim(&mut self, expr: ExprId, ty: TyId) -> TyId {
        let mut ty = ty;
        for index in self.claimable().rev() {
            let Some(function) = self.functions.get(index).map(|function| function.fn_id) else {
                continue;
            };
            if !self.types.contains_provisional(ty, Some(function)) {
                continue;
            }
            let mut params = std::mem::take(&mut self.claimed);
            params.clear();
            self.types.provisional_params(ty, function, &mut params);
            let span = self.ast.get(expr).span;
            ty = self.make_present(index, span, &params, ty);
            self.claimed = params;
        }
        ty
    }

    /// Where the value at `span`, of type `ty`, is used as its bits at the
    /// top, as a record is by an update: if `ty` is a provisional type of a
    /// claimable function (see [`Checker::claimable`]), measures its
    /// parameter and returns its present type; otherwise returns `ty`.
    pub(super) fn claim_top(&mut self, span: Span, ty: TyId) -> TyId {
        let TyKind::Provisional {
            function, param, ..
        } = *self.types.kind(ty)
        else {
            return ty;
        };
        let index = self
            .claimable()
            .find(|&index| self.functions.get(index).map(|f| f.fn_id) == Some(function));
        match index {
            Some(index) => self.make_present(index, span, &[param], ty),
            None => ty,
        }
    }

    /// Records that each of `params`, parameters of the function at `index`
    /// of the stack of functions being checked, is measured at `span`, and
    /// returns `ty` with every provisional type of that function in it made
    /// present.
    pub(super) fn make_present(
        &mut self,
        index: usize,
        span: Span,
        params: &[u32],
        ty: TyId,
    ) -> TyId {
        let Some(function) = self.functions.get_mut(index) else {
            return ty;
        };
        for &param in params {
            if let Some(entry) = function.demanded.get_mut(param as usize) {
                // The first measuring use in source order.
                if entry.is_none_or(|first| span.start < first.start) {
                    *entry = Some(span);
                }
            }
        }
        let (fn_id, count) = (function.fn_id, function.demanded.len());
        self.types.settle(ty, fn_id, &vec![true; count])
    }

    /// Reports that `expr`, of type `found`, does not fit the type
    /// `expected` stated at `context`, labelled `why`; for an argument of a
    /// known callee of this input, `filling` being the callee's function
    /// and the parameter the argument fills, with a label at what kept the
    /// parameter from being narrowed, if that is the cause (see
    /// [`Checker::narrowing_label`]); with a note when a future is expected
    /// and what is found is not one; with the note saying to tell a
    /// union's members apart when what is found is one (see
    /// [`Checker::union_note`]); and with the note saying that a default
    /// fills only a record literal when what is found lacks a defaulted
    /// field (see [`Checker::default_note`]).
    pub(super) fn mismatch(
        &mut self,
        expr: ExprId,
        (found, expected): (TyId, TyId),
        (context, why): (Span, &str),
        filling: Option<(FnId, usize)>,
    ) {
        let narrowing = filling.and_then(|filling| self.narrowing_label(filling, found, expected));
        let found = self.settle(expr, found);
        let label = format!(
            "expected {}, found {}",
            self.in_label(expected),
            self.in_label(found)
        );
        let mut error = Diagnostic::error("mismatched types", self.ast.get(expr).span)
            .with_label(label)
            .with_context(context, why);
        if let Some((span, label)) = narrowing {
            error = error.with_context(span, label);
        }
        if let Some(note) = self.union_note(expr, found) {
            error = error.with_note(note);
        }
        if let Some(note) = self.default_note(found, expected) {
            error = error.with_note(note);
        }
        if let Some(note) = self.secret_note(found, expected) {
            error = error.with_note(note);
        }
        if self.types.laziness(expected) == Laziness::Future {
            let shown = self.in_message(expected);
            let found = self.shown(found);
            match self.types.laziness(found) {
                Laziness::Present => {
                    error = error.with_note(format!(
                        "a present value does not satisfy {shown}; write `lazy` before the \
                         expression to defer it"
                    ));
                }
                // A provisional type is shown as what it may be, but none is
                // left once shown.
                Laziness::Either | Laziness::Provisional => {
                    error = error.with_note(format!(
                        "this value may already be present; a {shown} must be deferred"
                    ));
                }
                Laziness::Future => {}
            }
        }
        self.errors.push(error);
    }

    /// The note on a mismatch of a value of type `found` with `expected` when
    /// `found` lacks a defaulted field of a record type of `expected` (see
    /// "Field defaults" under [Functions](crate::check::functions#functions)),
    /// which no default fills, the value being no record literal written where
    /// the record type is expected: "a default fills only a record literal
    /// written where `T` is expected", `T` that record type
    /// ([`Checker::lacked_default`]). `None` otherwise.
    pub(super) fn default_note(&mut self, found: TyId, expected: TyId) -> Option<String> {
        let record = self.lacked_default(found, expected)?;
        let shown = self.in_message(record);
        Some(format!(
            "a default fills only a record literal written where {shown} is expected"
        ))
    }

    /// The record type, of `expected` or of a type in it, that has a
    /// defaulted field that `found` lacks where the two types meet: each
    /// stripped of its future layers, at the top, and then through the
    /// element types of two list types and through the fields that two
    /// record types share, outermost first. `None` if there is none, and
    /// where `expected` is a `Future`.
    ///
    /// Recurses once per level of list and record nesting shared by the two
    /// types.
    pub(super) fn lacked_default(&self, found: TyId, expected: TyId) -> Option<TyId> {
        if self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let (found, expected) = (self.types.strip(found), self.types.strip(expected));
        if let (Some(found), Some(expected)) = (
            self.types.list_element(found),
            self.types.list_element(expected),
        ) {
            return self.lacked_default(found, expected);
        }
        let (Some(_), Some(fields)) = (
            self.types.record_fields(found),
            self.types.record_fields(expected),
        ) else {
            return None;
        };
        let lacked = self
            .types
            .defaults(expected)
            .iter()
            .any(|&(name, _)| self.types.field(found, name).is_none());
        if lacked {
            return Some(expected);
        }
        fields
            .iter()
            .find_map(|&(name, ty)| self.lacked_default(self.types.field(found, name)?, ty))
    }

    /// The context label on the mismatch of an argument of type `found`
    /// with `expected`, the type of the parameter number `param` of the
    /// function expression `function` of this input, which the argument
    /// fills, if that parameter is a callback parameter that was not
    /// narrowed at some position and the argument would have fitted had
    /// every narrowable position been narrowed, as it is or once awaited:
    /// the label is at what first kept one of the positions the argument
    /// needs narrowed as written, in source order, a position being needed
    /// if the argument's function type does not take what the position is
    /// written as (see the module documentation). `None` otherwise, when
    /// the mismatch has another cause.
    pub(super) fn narrowing_label(
        &mut self,
        (function, param): (FnId, usize),
        found: TyId,
        expected: TyId,
    ) -> Option<(Span, String)> {
        let unnarrowed = self
            .fn_resolutions
            .get(function.index())?
            .unnarrowed
            .get(param)?
            .clone()?;
        if !self.types.is_subtype(found, unnarrowed.narrowed)
            && self.types.needs_await(found, unnarrowed.narrowed).is_none()
        {
            return None;
        }
        let offered = self.types.fn_sig(self.types.strip(found))?;
        let written = self.types.fn_sig(expected)?;
        let block = unnarrowed
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(at, &block)| {
                let (&(_, written), &(_, accepted)) =
                    (written.params.get(at)?, offered.params.get(at)?);
                block.filter(|_| !self.types.is_subtype(written, accepted))
            })
            .min_by_key(|block| block.span().start)?;
        let decl = self.ast.fn_decl(function);
        let name = self
            .ast
            .names()
            .get(self.ast.params(decl.params).get(param)?.name);
        let label = match block {
            NarrowingBlock::Given(_) => {
                format!("`{name}` is given a value that may be a future here")
            }
            NarrowingBlock::Used(_) => {
                format!("`{name}` is used as a value here, so it may be given a future")
            }
        };
        Some((block.span(), label))
    }

    /// Checks `id` where a value of type `expected` is expected, as stated
    /// at `context`, labelled `why`, and returns the type it has.
    ///
    /// If `expected` is a record type, or the either shape of one, and `id`
    /// is, through any parentheses, a record block, the block is checked
    /// against the record type field by field (see the module
    /// documentation) and its type, and that of each parenthesis around it,
    /// is the record of its fields, a subtype of `expected`, or `Error` if a
    /// field is missing. If `expected` is a list type, or the either shape
    /// of one, and `id` is, through any parentheses, a list literal, a block
    /// with a result or an `if` with `else`, it is checked against the list
    /// type as [`Checker::check_list_expected`] says. Otherwise `id` is
    /// checked bottom-up and then must be a subtype of `expected`, as
    /// [`Checker::expect`] says; but if `expected` is a record type with
    /// defaults, or the either shape of one, and `id` is, through any
    /// parentheses, a block with a result or an `if` with `else`, a record
    /// literal it results in is checked against the record type first, for
    /// the defaults alone, as [`Checker::fill_result`] says.
    pub(super) fn check_expr_expecting(
        &mut self,
        id: ExprId,
        expected: TyId,
        context: Span,
        why: &str,
    ) -> TyId {
        self.check_expr_expecting_at(id, expected, (context, why), (AwaitSite::Implicit, None))
    }

    /// As [`Checker::check_expr_expecting`], with `site` saying how an
    /// await of `id` at the top is reported in a pure function (see
    /// [`Checker::await_here`]), and `filling`, for an argument of a known
    /// callee of this input, the callee's function and the parameter the
    /// argument fills, for the label on a mismatch (see
    /// [`Checker::narrowing_label`]).
    pub(super) fn check_expr_expecting_at(
        &mut self,
        id: ExprId,
        expected: TyId,
        stated: (Span, &str),
        how: (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        // An erroneous expected type, its error reported elsewhere, types
        // nothing in the value: what it would have typed is no error.
        if expected == TyId::ERROR {
            self.quiet.expecting_error = self.quiet.expecting_error.saturating_add(1);
            let found = self.check_expected(id, expected, stated, how);
            self.quiet.expecting_error = self.quiet.expecting_error.saturating_sub(1);
            return found;
        }
        self.check_expected(id, expected, stated, how)
    }

    /// [`Checker::check_expr_expecting_at`], whatever `expected` is.
    pub(super) fn check_expected(
        &mut self,
        id: ExprId,
        expected: TyId,
        (context, why): (Span, &str),
        (site, filling): (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        // Checked already, to infer a call's type parameters.
        if let Some(found) = self.calls.prechecked.remove(&id) {
            self.expect_at(id, found, expected, (context, why), (site, filling));
            return found;
        }
        if let Some(found) = self.loop_within(id) {
            return self.check_loop_expecting(id, found, expected, (context, why), (site, filling));
        }
        // A record block is present, so it is checked against the present
        // member of an either shape (or of a provisional type), or the one
        // record member of a union.
        if let Some(record) = self.expected_member(expected, ValueKind::Record)
            && let Some((block, body)) = self.record_block_within(id)
        {
            let span = self.ast.get(block).span;
            // A union of record types fills nothing.
            let itself = record == self.types.strip(expected);
            let expecting = Expecting {
                ty: record,
                context,
                why,
                close: Span::new(span.end.saturating_sub(1), span.end),
                fill: itself.then_some(block),
                reach: Reach::Whole,
            };
            let ty = self.check_block(body, Some(expecting));
            let mut node = id;
            loop {
                self.expr_types[node.index()] = ty;
                match self.ast.get(node).kind {
                    ExprKind::Paren(inner) => node = inner,
                    _ => break,
                }
            }
            return ty;
        }
        // `{}`, the empty record's literal, is given the defaults of a
        // record type with defaults expected itself.
        if let Some(block) = self.empty_block_within(id)
            && let Some(record) = self.defaulted_record(expected)
        {
            let span = self.ast.get(block).span;
            let expecting = Expecting {
                ty: record,
                context,
                why,
                close: Span::new(span.end.saturating_sub(1), span.end),
                fill: Some(block),
                reach: Reach::Whole,
            };
            let ty = self.empty_record(expecting);
            self.set_through_parens(id, ty);
            return ty;
        }
        if self.expects_list(expected)
            && let Some(ty) = self.check_list_expected(id, expected, (context, why))
        {
            return ty;
        }
        // A record type with defaults expected itself reaches a record
        // literal that a block results in, or an `if`'s branch does, to give
        // it the defaults it lacks; what the block or `if` is then must fit
        // as ever.
        let found = match self.defaulted_record(expected) {
            Some(record) if self.reaches_result(id) => self.fill_result(id, record, (context, why)),
            _ => self.check_expr(id),
        };
        self.expect_at(id, found, expected, (context, why), (site, filling));
        found
    }

    /// Whether `id` is, through any parentheses, a block with a result or an
    /// `if` with `else`: what a record type with defaults expected of it
    /// reaches, for a record literal it results in (see
    /// [`Checker::fill_result`]).
    pub(super) fn reaches_result(&self, id: ExprId) -> bool {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items, .. } => return self.has_result(items),
                ExprKind::If { has_else, .. } => return has_else,
                _ => return false,
            }
        }
    }

    /// Checks `id`, of which the record type with defaults `record` is
    /// expected, as stated at `context`, labelled `why`, for the defaults alone
    /// (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)), and returns its type,
    /// recorded for `id` and every parenthesis around it. A record literal, a
    /// record block or `{}`, through any parentheses, is checked against
    /// `record` as any literal is where it is expected
    /// ([`Checker::check_expr_expecting`]), which gives it the defaults it
    /// lacks. A block with a result has its result checked so in turn
    /// ([`Expecting::literal_only`]), and a labelled one's `return`s as ever;
    /// an `if` with `else` has each branch checked so, and is their join, as
    /// ever ([`Checker::check_if`]). Anything else is checked as it is with
    /// nothing expected, and nothing else of the block or `if` changes: what it
    /// is must then fit what is expected where it stands, as ever.
    ///
    /// Recurses once per level of blocks and branches nested in each other.
    pub(super) fn fill_result(
        &mut self,
        id: ExprId,
        record: TyId,
        (context, why): (Span, &str),
    ) -> TyId {
        let mut node = id;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let expr = *self.ast.get(node);
        let ty = match expr.kind {
            ExprKind::Block { items, label }
                if self.is_record_body(items)
                    || (label.is_none() && self.ast.items(items).is_empty()) =>
            {
                return if self.lacks_default(items, record) {
                    self.check_expr_expecting(id, record, context, why)
                } else {
                    self.check_expr(id)
                };
            }
            ExprKind::Block { items, label } if self.has_result(items) => {
                let expecting = Expecting::literal_only(record, (context, why), expr.span);
                self.reach_block(node, (items, label), expecting)
            }
            ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                let fills = Branch::Fills(record, (context, why));
                self.check_if(keyword, cond, (then, else_), (true, fills))
            }
            _ => return self.check_expr(id),
        };
        self.set_through_parens(id, ty);
        ty
    }

    /// Checks `id`, a list whose elements the record type with defaults
    /// `record` is expected to be of, as stated at `context`, labelled `why`,
    /// for the defaults alone (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)), and returns its type,
    /// recorded for `id` and every parenthesis around it. A list literal,
    /// through any parentheses, is checked as it is with nothing expected but
    /// for its elements: each that is no spread as [`Checker::fill_result`]
    /// says, so that a record literal lacking a defaulted field is given it,
    /// and each spread's operand as [`Checker::check_spread`] says with
    /// `record` ([`Checker::check_list`]). A `for` has the record literal its
    /// body results in given the defaults it lacks, as a spread `for` has
    /// ([`Checker::check_for`]). A block with a result has its result checked
    /// so in turn ([`Expecting::elements_only`]), and a labelled one's
    /// `return`s as ever; an `if` with `else` has each branch checked so, and
    /// is their join, as ever ([`Checker::check_if`]). Anything else is checked
    /// as it is with nothing expected, and nothing else of the list, the loop,
    /// the block or the `if` changes: what it is must then fit what is expected
    /// where it stands, as ever.
    ///
    /// Recurses once per level of blocks, branches, list literals and
    /// spreads nested in each other.
    pub(super) fn fill_elements(
        &mut self,
        id: ExprId,
        record: TyId,
        (context, why): (Span, &str),
    ) -> TyId {
        let mut node = id;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let expr = *self.ast.get(node);
        let fill = (record, (context, why));
        let ty = match expr.kind {
            ExprKind::List(elements) => self.check_list(node, elements, Some(fill)),
            ExprKind::For(for_id) => self.check_for(node, for_id, (None, Some(fill))),
            ExprKind::Block { items, label } if self.has_result(items) => {
                let expecting = Expecting::elements_only(record, (context, why), expr.span);
                self.reach_block(node, (items, label), expecting)
            }
            ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                let fills = Branch::FillsElements(record, (context, why));
                self.check_if(keyword, cond, (then, else_), (true, fills))
            }
            _ => return self.check_expr(id),
        };
        self.set_through_parens(id, ty);
        ty
    }

    /// Checks the block `node`, with a result, whose items are `items`,
    /// labelled `label` if it is, with `expecting`, which reaches only part
    /// of its result ([`Expecting::literal_only`],
    /// [`Expecting::elements_only`]), and returns its type: a labelled one
    /// has its `return`s checked as ever, and joined with its result.
    pub(super) fn reach_block(
        &mut self,
        node: ExprId,
        (items, label): (ItemRange, Option<LabelId>),
        expecting: Expecting<'_>,
    ) -> TyId {
        match label {
            Some(label) => {
                self.enter_block(node, self.ast.label(label), None);
                let ty = self.check_block(items, Some(expecting));
                self.leave_block(node, ty, None)
            }
            None => self.check_block(items, Some(expecting)),
        }
    }

    /// Whether the record literal whose items are `items`, a record block's
    /// or `{}`'s, has no spread and lacks a defaulted field of the record
    /// type `record`, giving it by none of its field items, or `record` has
    /// a rest-field: whether, where `record` reaches it only for its
    /// defaults and its rest-field ([`Checker::fill_result`]), it is given
    /// one, and so checked against `record`. One that gives every defaulted
    /// field, or that has a spread, which may give it one, is checked there
    /// as it is with nothing expected, unless `record` has a rest-field,
    /// which the literal has only once checked against it.
    pub(super) fn lacks_default(&self, items: ItemRange, record: TyId) -> bool {
        if self.types.record_rest(record).is_some() {
            return true;
        }
        let items = self.ast.items(items);
        let ast = self.ast;
        if items
            .iter()
            .any(|&item| matches!(ast.item(item).kind, ItemKind::Spread(_)))
        {
            return false;
        }
        self.types.defaults(record).iter().any(|&(name, _)| {
            !items.iter().any(|&item| match ast.item(item).kind {
                ItemKind::Field { path, .. } => {
                    ast.path(path).first().and_then(PathSegment::name) == Some(name)
                }
                _ => false,
            })
        })
    }

    /// Whether a value where `expected` is expected is checked against it
    /// as [`Checker::check_list_expected`] says: whether `expected` is a
    /// list type or a union, or the either shape of one (or a provisional
    /// one), but not a future, which no list literal, block or `if` is.
    pub(super) fn expects_list(&self, expected: TyId) -> bool {
        if self.types.laziness(expected) == Laziness::Future {
            return false;
        }
        let stripped = self.types.strip(expected);
        self.types.list_element(stripped).is_some() || self.types.union_members(stripped).is_some()
    }

    /// The type a record block, for `kind` [`ValueKind::Record`], or a list
    /// literal or a `for`, for [`ValueKind::List`], is checked against
    /// where `expected` is expected: `expected` once stripped, if it is of
    /// that kind, or the one member of that kind of the union it is once
    /// stripped, if it has exactly one; `None` otherwise, and where
    /// `expected` is a future, which no such value is.
    pub(super) fn expected_member(&self, expected: TyId, kind: ValueKind) -> Option<TyId> {
        if self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let stripped = self.types.strip(expected);
        let of_kind = |ty: TyId| match kind {
            ValueKind::Record => self.types.is_record(ty),
            ValueKind::List => self.types.list_element(ty).is_some(),
            _ => false,
        };
        if of_kind(stripped) {
            return Some(stripped);
        }
        let mut members = self
            .types
            .union_members(stripped)?
            .iter()
            .copied()
            .filter(|&member| of_kind(member));
        match (members.next(), members.next()) {
            (Some(member), None) => Some(member),
            _ => None,
        }
    }

    /// Whether `body`, a block's, has a result: whether it is not a record
    /// block and its last item is a `= EXPR` or an expression statement.
    pub(super) fn has_result(&self, body: ItemRange) -> bool {
        !self.is_record_body(body)
            && self.ast.items(body).last().is_some_and(|&last| {
                matches!(
                    self.ast.item(last).kind,
                    ItemKind::Result(_) | ItemKind::Expr(_)
                )
            })
    }

    /// Where a value of type `expected`, a list type or a union, or the
    /// either shape of one, is expected, as stated at `context`, labelled
    /// `why`: checks `id` against it and returns the type it has, if `id`
    /// is, through any parentheses, a list literal and `expected` has a
    /// list type to check it against (see [`Checker::expected_member`]), a
    /// block with a result or an `if` with `else`, and `None`, having
    /// checked nothing, otherwise. (A loop is checked against what is
    /// expected of it by [`Checker::check_loop_expecting`].) The type of a
    /// list literal is found as [`Checker::check_list_expecting`] says, and
    /// that of a block or an `if` is what its result, or each branch, checked against
    /// `expected` in turn, is once any await marked on it has run (see
    /// [`Checker::fit`]); it is recorded for `id` and every parenthesis
    /// around it. So the expected type reaches a list literal that is a
    /// function's body's result, a branch's, or either nested in the other,
    /// and each branch of an `if` that a union is expected of, so that
    /// branches of different types make no join, and no warning.
    pub(super) fn check_list_expected(
        &mut self,
        id: ExprId,
        expected: TyId,
        (context, why): (Span, &str),
    ) -> Option<TyId> {
        let mut node = id;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let expr = *self.ast.get(node);
        let ty = match expr.kind {
            ExprKind::List(elements) => {
                // `[]` is a list of every type, so any of a union's lists.
                let list = self
                    .expected_member(expected, ValueKind::List)
                    .or_else(|| {
                        let members = self.types.union_members(self.types.strip(expected))?;
                        let empty = self.ast.elements(elements).is_empty();
                        members
                            .iter()
                            .copied()
                            .find(|&member| empty && self.types.list_element(member).is_some())
                    })?;
                self.check_list_expecting(elements, list, (context, why))
            }
            ExprKind::Block { items, label } if self.has_result(items) => {
                let expecting = Expecting::new(expected, (context, why), expr.span);
                match label {
                    Some(label) => self.labelled_block(node, label, items, Some(expecting)),
                    None => self.check_block(items, Some(expecting)),
                }
            }
            ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                let narrowing = self.if_condition(keyword, cond);
                let expecting = Branch::Fits(expected, (context, why));
                let then_ty = self.check_branch(then, narrowing.then, expecting);
                let else_ty = self.check_branch(else_, narrowing.else_, expecting);
                if let Some(next_slot) = narrowing.next_slot {
                    self.next_slot = next_slot;
                }
                if then_ty == TyId::ERROR || else_ty == TyId::ERROR {
                    TyId::ERROR
                } else if self.expr_types[cond.index()] == self.never {
                    // A condition of type `Never` never picks a branch.
                    self.never
                } else {
                    // What the branches hold has a join that fits, or the
                    // expected type describes both. Each branch fits what
                    // is expected, so a widened join is no warning.
                    let (joined, _) = self.types.join(then_ty, else_ty);
                    if self.types.is_subtype(joined, expected) {
                        joined
                    } else {
                        expected
                    }
                }
            }
            // A loop is checked against whatever is expected of it, by
            // `check_loop_expecting`, before this is asked.
            _ => return None,
        };
        self.set_through_parens(id, ty);
        Some(ty)
    }

    /// Records `ty` as the type of `id` and of every parenthesis inside it,
    /// down to what they hold.
    pub(super) fn set_through_parens(&mut self, id: ExprId, ty: TyId) {
        let mut node = id;
        loop {
            self.expr_types[node.index()] = ty;
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                _ => break,
            }
        }
    }

    /// The loop that `id` is, through any parentheses, with its details,
    /// or `None` if it is not one.
    pub(super) fn loop_within(&self, id: ExprId) -> Option<(ExprId, ForId)> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::For(for_id) => return Some((node, for_id)),
                _ => return None,
            }
        }
    }

    /// Checks `id`, which is, through any parentheses, the loop `node`,
    /// whose details are `for_id`, where a value of type `expected` is
    /// expected, as stated at `context`, labelled `why`, with `site` and
    /// `filling` as for [`Checker::check_expr_expecting_at`] (see
    /// [`Checker::check_for`]): every `return` for the loop is checked
    /// against `expected`, and its body against the element type of
    /// `expected`'s list type, if it has one, as a list literal is. A loop
    /// that no `return` leaves, and whose list type is not checked so, must
    /// then fit `expected` as any value must (see [`Checker::expect`]).
    /// Returns its type, recorded for `id` and every parenthesis inside it.
    pub(super) fn check_loop_expecting(
        &mut self,
        id: ExprId,
        (node, for_id): (ExprId, ForId),
        expected: TyId,
        (context, why): (Span, &str),
        (site, filling): (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        let ty = self.check_for(node, for_id, (Some((expected, (context, why))), None));
        self.set_through_parens(id, ty);
        let listed = self.expected_member(expected, ValueKind::List).is_some();
        if !listed && !self.loop_resolutions[for_id.index()].returned {
            self.expect_at(id, ty, expected, (context, why), (site, filling));
        }
        ty
    }

    /// Checks `id` where a value of type `expected` is expected, as stated
    /// at `context`, labelled `why` (see [`Checker::check_expr_expecting`]),
    /// and returns what it is once any await marked on it has run (see
    /// [`Checker::value_of`]), or `Error` if checking it reported an error,
    /// a mismatch included. A record block checked field by field against a
    /// record type is the record of what its fields are (see
    /// [`Checker::held_record`]).
    pub(super) fn fit(&mut self, id: ExprId, expected: TyId, (context, why): (Span, &str)) -> TyId {
        let errors = self.errors.len();
        let found =
            self.check_expr_expecting_at(id, expected, (context, why), (AwaitSite::Implicit, None));
        if found == TyId::ERROR || self.errors.len() > errors {
            return TyId::ERROR;
        }
        match self.value_of(id, expected) {
            BodyValue::Typed(ty) => ty,
            BodyValue::Fields(items) => {
                let record = self.types.strip(expected);
                self.held_record(items, record, found)
            }
            BodyValue::Open(_) => found,
        }
    }

    /// What the record block of `items`, checked without an error field by
    /// field against the record type `record`, which gave it the type `found`,
    /// holds once the awaits marked on its fields' values have run: the record
    /// of what each field's value is, found as [`Checker::value_of`] finds it,
    /// a record block in turn as this finds it, a field that `record` lacks as
    /// its type says, and a defaulted field of `record` that the block lacks,
    /// which it was given (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)), as `record` has it. A
    /// block with a dotted field or a spread keeps `found`, the record of the
    /// expected fields' types, and of what the soft fields the expected type
    /// lacks hold, a supertype of what it holds.
    ///
    /// Recurses once per level of record blocks nested in each other.
    pub(super) fn held_record(&mut self, items: ItemRange, record: TyId, found: TyId) -> TyId {
        let ast = self.ast;
        let base = self.sorted_fields.len();
        for &item in ast.items(items) {
            let (path, value) = match ast.item(item).kind {
                ItemKind::Field { path, value, .. } => (path, value),
                ItemKind::Spread(_) => {
                    self.sorted_fields.truncate(base);
                    return found;
                }
                _ => continue,
            };
            let Some(name) = (match ast.path(path) {
                [field] => field.name(),
                _ => None,
            }) else {
                self.sorted_fields.truncate(base);
                return found;
            };
            let ty = match self.types.field(record, name) {
                Some(expected) => match self.value_of(value, expected) {
                    BodyValue::Typed(ty) => ty,
                    BodyValue::Fields(inner) => {
                        let (inner_record, inner_found) =
                            (self.types.strip(expected), self.expr_types[value.index()]);
                        self.held_record(inner, inner_record, inner_found)
                    }
                    BodyValue::Open(_) => self.expr_types[value.index()],
                },
                None => self.expr_types[value.index()],
            };
            self.sorted_fields.push((name, 0, ty));
        }
        // The defaulted fields the block lacks, which it was given (see "Field
        // defaults" under [Functions](crate::check::functions#functions)), as
        // the type has them.
        for &(name, _) in self.types.defaults(record) {
            let written = self
                .sorted_fields
                .get(base..)
                .unwrap_or_default()
                .iter()
                .any(|&(field, _, _)| field == name);
            if !written && let Some(ty) = self.types.field(record, name) {
                self.sorted_fields.push((name, 0, ty));
            }
        }
        let fields = self.sorted_fields.get(base..).unwrap_or_default();
        let held = self
            .types
            .record(fields.iter().map(|&(name, _, ty)| (name, ty)));
        self.sorted_fields.truncate(base);
        held
    }

    /// The empty block `{}`, unlabelled, that `id` is, through any
    /// parentheses: the empty record's literal; `None` if it is not one.
    pub(super) fn empty_block_within(&self, id: ExprId) -> Option<ExprId> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items, label: None } if self.ast.items(items).is_empty() => {
                    return Some(node);
                }
                _ => return None,
            }
        }
    }

    /// The record type that `expected` is once stripped, if it is not a
    /// future and the record type has defaults or a rest-field: what `{}`
    /// is given the defaults of where `expected` is expected (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)), and its rest-field, which a record
    /// literal written there has too (see [Records](crate::check::records#records)). `None`
    /// otherwise, a union of record types included, where `{}` is checked
    /// as ever.
    pub(super) fn defaulted_record(&self, expected: TyId) -> Option<TyId> {
        if self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let record = self.types.strip(expected);
        (self.types.is_record(record)
            && (!self.types.defaults(record).is_empty()
                || self.types.record_rest(record).is_some()))
        .then_some(record)
    }

    /// The type of `{}`, the empty record's literal, checked against
    /// `expecting`'s record type, which has defaults or a rest-field, as a
    /// record block is ([`Checker::report_missing_fields`]): each defaulted
    /// field is given it, and the record of those fields, with the
    /// rest-field, is its type; each other field is missing, reported, and
    /// its type `Error`.
    pub(super) fn empty_record(&mut self, expecting: Expecting<'_>) -> TyId {
        let base = self.sorted_fields.len();
        let missing = self.report_missing_fields(base, expecting);
        let fields: Vec<(Name, TyId)> = self
            .sorted_fields
            .get(base..)
            .unwrap_or_default()
            .iter()
            .map(|&(name, _, ty)| (name, ty))
            .collect();
        self.sorted_fields.truncate(base);
        if missing {
            TyId::ERROR
        } else {
            let rest = self.types.record_rest(expecting.ty);
            self.types.record_with_rest(fields, rest, Vec::new())
        }
    }

    /// The type of the function body `body`, if it is `{}` and the return
    /// type `ret`, stated at `context`, labelled `why`, is a record type
    /// with defaults: `{}` given them, as any `{}` is where such a type is
    /// expected ([`Checker::empty_record`]). `None` otherwise, the body
    /// being checked as ever.
    pub(super) fn empty_body(
        &mut self,
        body: ExprId,
        ret: TyId,
        (context, why): (Span, &str),
    ) -> Option<TyId> {
        let block = self.empty_block_within(body)?;
        let record = self.defaulted_record(ret)?;
        let span = self.ast.get(block).span;
        Some(self.empty_record(Expecting {
            ty: record,
            context,
            why,
            close: Span::new(span.end.saturating_sub(1), span.end),
            fill: Some(block),
            reach: Reach::Whole,
        }))
    }

    /// The record block that `id` is, through any parentheses, with its
    /// body, or `None` if it is not one.
    pub(super) fn record_block_within(&self, id: ExprId) -> Option<(ExprId, ItemRange)> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items, .. } if self.is_record_body(items) => {
                    return Some((node, items));
                }
                _ => return None,
            }
        }
    }

    /// Whether `body` has a field or spread item, which makes it a record
    /// block.
    pub(super) fn is_record_body(&self, body: ItemRange) -> bool {
        self.ast.items(body).iter().any(|&id| {
            matches!(
                self.ast.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        })
    }
}
