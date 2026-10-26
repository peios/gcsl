//! Checking unions and narrowing: `if` and its branches, `is` and `as`,
//! `Never`, and the narrowing a test makes.
//!
//! # Never
//!
//! `Never` (see [`crate::types`]) is the type of what never completes: a
//! block whose last item is a `break`, `continue` or `return`, a loop that
//! cannot end (see [Never-ending loops](crate::check::loops#never-ending-loops)), and so
//! anything whose value is such a block's. Everything else follows from the join, `Never`
//! being its identity: an `if` both of whose branches are `Never` is
//! `Never`, `if c { break } else { 5 }` is `Int`, and an `if` without
//! `else` whose branch is `Never` is `{}`.
//!
//! Code that cannot be reached is never an error because of `Never`. Where
//! a rule asks what an operand *is*, a record for a field access, a list to
//! index, iterate, spread or count, a function to call, a number for
//! arithmetic, a `Bool` condition, an operand of `is`, `as`, `await`, a
//! comparison or an interpolation, an operand of type `Never` satisfies it,
//! and the operation is `Never` if that operand always runs first: so `{
//! break } + 1`, `f()({ break })`, `{ break }.a`, `if { break } { 1 } else
//! { 2 }`, a loop whose iterable or first condition is `Never` and `"{ {
//! break } }"` are `Never`. An operand that may not run asks nothing that
//! makes its operation `Never`: the right operand of `and` and `or`, a
//! `for`'s guard and a `do`'s condition, which comes after the body, the
//! operation keeping its own type. Where a type is only compared with an
//! expected one, an argument, an annotated `let`, a list element, a field,
//! subtyping accepts `Never`, so `f({ continue })` is `f`'s result and `[{
//! break }]` a `List<Never>`. A variable, a list element or a field may be
//! of type `Never`, a value that is never there: a `:=` or a dotted `let`
//! of such a variable, or along a path through such a field, is checked on
//! its own and declares `Never`, quietly.
//!
//! **The unreachable warning.** In a block, the first item after one that
//! never completes, a `break`, `continue` or `return` (not one that is an
//! error, in a record block or with nothing to leave), or an expression, result,
//! `let` or `:=` item whose
//! value is `Never`, is warned about once: "this is never run", labelled
//! "this always leaves the block" at the earlier item. The block's type is
//! still its last item's.
//!
//! # Unions
//!
//! `Int | String` is the type of a value that is an `Int` or a `String`; any
//! present types may be members (see [`crate::types`]). The checker does not
//! block where two types merely disagree: it joins the types of the expressions
//! whose values one expression may have, the branches of an `if` and the
//! elements of a list literal (and its spreads' elements), and the join always
//! exists ([`TypeTable::join`](crate::types::TypeTable::join)). Where two types
//! have no common supertype, at the top or inside a record field, a list's
//! element type or a function's return type, the join is their union, and
//! *widened*; a widened join is a warning, not an error, at the expression that
//! does not agree with what came before it:
//!
//! - an `if`'s branches, "the branches of this `if` have different types",
//!   at the `else` branch's value (a block's result, through blocks and
//!   parentheses), labelled with its type, with a label of the `then`
//!   branch's value's type on it, and the note "the `if` is `Int |
//!   String`; annotate it with that type if both are meant, or tell them
//!   apart with `is`" (`Checker::branches_widened`);
//! - a list literal's elements, "the elements of this list have different
//!   types", at the first element (or spread) that widens the join of those
//!   before it, labelled with its type, with a label of the elements' so far
//!   on the first, and the note "the list is `List<Int | String>`; ..."
//!   (`Checker::elements_widened`).
//!
//! There are no other joins of expressions' types that could widen: a
//! field access, an index or an iteration of a union whose members are all
//! records with the field, or all lists, has the join of what the members
//! have there, which is no warning, since the union says the value may be
//! any of them. Nor is there a warning where a type is expected that the
//! union fits: the expected type reaches the branches of an `if`, a
//! block's result and a list's elements (see [Lists](crate::check::lists#lists)), each is
//! checked against it, and the join of what they are is no warning. So
//! `let w: Int | String = if c { 1 } else { "one" }` warns about nothing.
//! An integer literal adapts to a union's `Int` member, or its `Float`
//! member if it has no `Int` one; one that cannot adapt to the other side
//! of a join settles to `Int` first, so `if c { 1 } else { "a" }` is `Int |
//! String`.
//!
//! A union is used by narrowing it (see [`is`](#is), [Narrowing](#narrowing)
//! and [`as`](#as)). Used as it is where one particular type is needed, an
//! operand of an operator, a condition, an index position, a range bound,
//! a callee, the record of a field access, the list of an index, an
//! iteration, `len` or a list's spread, it gives that place's existing
//! error, the union shown in it ("cannot add `Int | String` and Int"),
//! with the note "tell them apart first, as in `if v is Int { ... }`",
//! naming the variable when the operand is one, through parentheses, and
//! `x` otherwise, and the union's first member that `is` can test, as the
//! union shows its members, or, for a union of function types only, which
//! `is` cannot tell apart, a note to annotate the value with one function
//! type (`Checker::union_note`). An expected type the
//! union does not fit gives the mismatch it always has, with the same note.
//! A field access is allowed when every member is a record type with the
//! field, and an index, a `for` and `len` when every member is a list type;
//! a spread needs a list, or a record, as before, and the path of a dotted
//! `let` or `:=` records or lists, as before, without the note.
//! Interpolating, passing, storing,
//! returning and capturing a union need nothing. Awaiting an either of a
//! union gives the union.
//!
//! A union whose members are of different kinds is of
//! [`ValueKind::Mixed`](crate::value::ValueKind::Mixed) at runtime: lowering gives its nodes to the
//! evaluator's general walker only, as it does a value that may be a
//! future (see [`crate::lower`]).
//!
//! # `is`
//!
//! `EXPR is TYPE` is a `Bool`: whether the value is a `TYPE`
//! (`Checker::check_test`). `TYPE` is resolved with every bare type present,
//! and may hold no future, nor an either shape ("`is` cannot test a future"),
//! and no function type ("`is` cannot test a function type": a function's
//! signature is not something a value can be asked for; a function is told
//! apart by the other members, in the `else` branch). The operand is checked,
//! an untyped integer settling to `Int`, and measured at the top, as a
//! measuring operator's operand is: awaited if it may be a future, subject to
//! the purity rule, a pure function's parameter being measured. Its type then,
//! `S`, has *members*: its members if it is a union, and itself otherwise. Each
//! member `M`, made present (a list of futures being asked about as a list of
//! what they yield), *always passes* if it is a subtype of `TYPE`, *may pass*
//! if not but it *overlaps* `TYPE`
//! ([`TypeTable::overlaps`](crate::types::TypeTable::overlaps): a value could
//! be both, as far as their structure says), and otherwise *cannot pass*. If
//! every member always passes, the test is `true`, and a warning, "`x` is
//! always an Int", at the `is`; if none passes or may, it is an error, "`x` is
//! never a String", at `TYPE`, labelled with the operand's type at the operand.
//! `x` is the operand as written when it is a name or a dotted path, and "this
//! value" otherwise.
//!
//! The test at runtime is by the value's structure, as its *plan*
//! ([`TypeTable::test_plan`](crate::types::TypeTable::test_plan)) says, which
//! is recorded for lowering in
//! [`Resolution::test_of`](super::Resolution::test_of): a scalar type by the
//! value's kind; a record type by the record's having each field, each passing
//! its own test, whatever else it has; a list type by each element's passing
//! (the empty list passes every list type); a union by any member's. The plan
//! looks no further than `S` leaves in doubt: if every member of `S` of the
//! kind tested always passes, it tests the value's kind alone (`x is List<Int>`
//! for `x: Int | List<Int>`), and otherwise it looks inside, applying the same
//! rule at each field and element, from what `S`'s members of that kind say of
//! it (or nothing, where one lacks the field). How deeply the operand is
//! awaited follows the plan: at the top for one that tests the kind alone, so
//! that `xs is Int` awaits no element of a list of futures, and deeply, as an
//! operand of an equality is (`Checker::measure_deep`), for one that looks
//! inside, `S` then being made present for everything below. Values are
//! immutable and types structural, so a value that passes the test of a type is
//! of that type, whatever it was built as, which is what makes narrowing on the
//! test sound.
//!
//! The *narrowed type* `N` is the union of each member that always passes,
//! as it is, and `TYPE`, if any member may pass; the *remainder* `R` is the
//! union of the members that do not always pass. "May pass" asks whether
//! `M` and `TYPE` overlap, not merely whether `TYPE` is a subtype of `M`, so
//! that `x is {value: Int}` is a real test for `x: {kind: String, value:
//! Int | String}`, and so that a member that cannot pass is one no value of
//! which passes the test: were it one whose record could have a field its
//! type hides, such a record could pass and be taken for another member's
//! type than its own.
//!
//! # Narrowing
//!
//! `if NAME is TYPE { A } else { B }`, the condition being exactly that
//! through any parentheses, and `NAME`, through any parentheses, a
//! variable, mutable or not, a parameter, a field, a `for`'s variable or a
//! variable of a `for` header,
//! narrows `NAME` in the branches (`Checker::if_condition`). The `is`
//! writes the value it tests, awaited as the test awaited it, to a slot of
//! its own, taken for the `if` and free again after it; in `A`, `NAME` is a
//! new binding of `N` in that slot, in a scope around `A`, as a `for`'s
//! variable is around its body, and in `B` a new binding of `R` there in
//! the same way, unless `R` is the whole of `S` (no member always passes)
//! and the test awaited nothing, when `B` keeps `NAME` as it was. Each is
//! a copy (`BindingKind::Narrowed`), declared, as its diagnostics say, by
//! the `is`. Being a copy, nothing that rebinds the variable it was copied
//! from can change it, which keeps the narrowing sound for a mutable
//! variable. For the same reason `NAME := v` in the branch is an error,
//! "cannot rebind `x` here: it is narrowed by `is`": it would change the
//! copy and leave the variable as it was, silently for a mutable one, so
//! the note says to test a copy under another name, which leaves the
//! variable free to rebind. A `let NAME` in the branch shadows it as any
//! `let` does. When the
//! test always passes (and is warned about), `A` is narrowed and `B` keeps
//! `NAME`. An `else if` is an `if` inside `B`, narrowing in turn. Both
//! paths that check an `if`, with and without an expected type, narrow
//! alike.
//!
//! Nothing else narrows, for now: not `x is Int and x > 5`, not `not (x is
//! Int)`, not a field path (`r.a is Int`), and nothing after the `if` (an
//! early exit from `A` does not make `NAME` the remainder after it). These
//! are flow narrowing, deferred to a later slice.
//!
//! # `as`
//!
//! `EXPR as TYPE` is the operand as a `TYPE`, or a runtime error: a checked
//! narrowing, never a blind promise. The operand and `TYPE` follow the
//! rules of `is`, the same members, the same plan, awaited the same way,
//! the same "never" error and the same rejection of futures and function
//! types; if every member always passes, it is a warning, "`as` changes
//! nothing here: `x` is already an Int", with the note "`: Int` says the
//! same without a test". Its type is `N`, `TYPE` or what is more precise,
//! and its value the operand, awaited as the test awaited it. The test runs
//! at runtime, and a value that fails it is "`x` is not an Int", at the
//! whole expression, labelled with what the value is (see [`crate::eval`]).
//! It does nothing impure, so a pure function may use it on an operand that
//! needs no await.

use crate::ast::{ExprId, ExprKind, ItemKind, TypeExprId};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::types::{Laziness, TyId, ValueTest};

use super::{Bareness, Binding, BindingKind, Branch, Checker, Narrowing, Slot, TestResolution, Tested};

impl Checker<'_> {
    /// Types an `if` whose keyword is at `keyword`: its condition must be
    /// `Bool`, and its branches agree (see [`Checker::branches`]) or, without
    /// `else`, its block is `{}`. A condition that narrows a variable gives
    /// each branch its binding (see [`Checker::if_condition`]). Each branch
    /// is checked as `branch` says: bottom-up, or, with `else`, where a
    /// record type with defaults is expected of the `if` itself, for a
    /// record literal it results in alone ([`Checker::fill_result`]), or of
    /// the elements of the list the `if` is, for those of a list literal it
    /// results in alone ([`Checker::fill_elements`]), the branches agreeing
    /// as ever.
    pub(super) fn check_if(
        &mut self,
        keyword: Span,
        cond: ExprId,
        (then, else_): (ExprId, ExprId),
        (has_else, branch): (bool, Branch<'_>),
    ) -> TyId {
        let narrowing = self.if_condition(keyword, cond);
        let then_ty = self.check_branch(then, narrowing.then, branch);
        let else_ty = self.check_branch(else_, narrowing.else_, branch);
        if let Some(next_slot) = narrowing.next_slot {
            self.next_slot = next_slot;
        }
        // A condition of type `Never` never picks a branch.
        if self.expr_types[cond.index()] == self.never {
            self.settle(then, then_ty);
            self.settle(else_, else_ty);
            return self.never;
        }
        if has_else {
            self.branches((then, then_ty), (else_, else_ty), "`if`")
        } else {
            self.empty_branch(keyword, then, then_ty)
        }
    }

    /// Checks the condition `cond` of the `if` whose keyword is at
    /// `keyword`, which must be `Bool`, and returns the bindings it gives
    /// the branches. A condition that is, through any parentheses, `NAME is
    /// TYPE`, NAME being, through any parentheses, a variable, a field, a
    /// parameter or a `for`'s variable in scope, narrows it (see the module
    /// documentation): the test's value is stored in a slot of its own,
    /// taken here, and the `then` branch has a binding of NAME there of the
    /// narrowed type, and the `else` branch one of the remainder, unless the
    /// test always passes, or the remainder is the whole of the value's
    /// type and the test awaits nothing. Any other condition narrows
    /// nothing.
    pub(super) fn if_condition(&mut self, keyword: Span, cond: ExprId) -> Narrowing {
        let Some((test, operand, ty, name)) = self.narrowing_test(cond) else {
            let cond_ty = self.check_expr(cond);
            self.expect(
                cond,
                cond_ty,
                TyId::BOOL,
                keyword,
                "`if` conditions are Bool",
            );
            return Narrowing::default();
        };
        let tested = self.check_test(test, operand, ty, "is");
        let cond_ty = match tested {
            Some(tested) if tested.unreached => self.never,
            Some(_) => TyId::BOOL,
            None => TyId::ERROR,
        };
        // A test of a value of type `Never` is never run, and narrows
        // nothing.
        let tested = tested.filter(|tested| !tested.unreached);
        let mut node = cond;
        loop {
            self.expr_types[node.index()] = cond_ty;
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                _ => break,
            }
        }
        let Some(tested) = tested else {
            return Narrowing::default();
        };
        let next_slot = self.next_slot;
        let slot = Slot(next_slot);
        self.next_slot = next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        match self.slot_names.get_mut(slot.index()) {
            Some(entry) => *entry = name,
            None => self.slot_names.push(name),
        }
        if let Some(resolution) = self.tests.get_mut(&test) {
            resolution.slot = slot;
        }
        // Declared by the test, where a rebind of it points.
        let binding = |ty: TyId| Binding {
            name,
            slot,
            ty,
            declared: Some(self.ast.get(test).span),
            kind: BindingKind::Narrowed,
            function: None,
            callback: None,
        };
        let else_ = match tested.remainder {
            Some(remainder) if !tested.whole || tested.awaited => Some(binding(remainder)),
            _ => None,
        };
        Narrowing {
            then: Some(binding(tested.narrowed)),
            else_,
            next_slot: Some(next_slot),
        }
    }

    /// If `cond`, an `if`'s condition, narrows a variable (see
    /// [`Checker::if_condition`]): the `is` expression, its operand, its
    /// type and the variable's name. `None` for every other condition.
    pub(super) fn narrowing_test(
        &self,
        cond: ExprId,
    ) -> Option<(ExprId, ExprId, TypeExprId, Name)> {
        let mut test = cond;
        while let ExprKind::Paren(inner) = self.ast.get(test).kind {
            test = inner;
        }
        let ExprKind::Is { expr: operand, ty } = self.ast.get(test).kind else {
            return None;
        };
        let mut node = operand;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let ExprKind::Name(name) = self.ast.get(node).kind else {
            return None;
        };
        let (_, binding) = self.lookup(name)?;
        matches!(
            binding.kind,
            BindingKind::Variable
                | BindingKind::Mutable
                | BindingKind::Header
                | BindingKind::Field
                | BindingKind::Narrowed
        )
        .then_some((test, operand, ty, name))
    }

    /// Checks the branch `branch` of an `if`, with the narrowed binding
    /// `narrowed`, if it has one, bound in a scope around it, as a `for`'s
    /// variable is around its body, so that a `:=` of it in the branch
    /// finds it and is rejected (see [`Checker::check_rebind`]). With
    /// `expected` [`Branch::Fits`], the type the branch is expected to have,
    /// with where that is stated and its label, it is checked against it
    /// and has what it is once any await marked on it has run (see
    /// [`Checker::fit`]); with [`Branch::Fills`], a record type with
    /// defaults reaches only a record literal it results in (see
    /// [`Checker::fill_result`]); with [`Branch::FillsElements`], only the
    /// elements of a list literal it results in, or the body of a `for` it
    /// does (see [`Checker::fill_elements`]); otherwise it is checked
    /// bottom-up.
    pub(super) fn check_branch(
        &mut self,
        branch: ExprId,
        narrowed: Option<Binding>,
        expected: Branch<'_>,
    ) -> TyId {
        let Some(narrowed) = narrowed else {
            return self.branch_type(branch, expected);
        };
        let start = self.bindings.len();
        self.bind(narrowed);
        let ty = self.branch_type(branch, expected);
        self.bindings.truncate(start);
        ty
    }

    /// Checks the branch `branch` of an `if` as [`Checker::check_branch`]
    /// says, its narrowed binding, if any, being in place already.
    pub(super) fn branch_type(&mut self, branch: ExprId, expected: Branch<'_>) -> TyId {
        match expected {
            Branch::Fits(expected, context) => self.fit(branch, expected, context),
            Branch::Fills(record, context) => self.fill_result(branch, record, context),
            Branch::FillsElements(record, context) => self.fill_elements(branch, record, context),
            Branch::Free => self.check_expr(branch),
        }
    }

    /// Types the `is` or `as`, as `keyword` says, expression `id`, testing
    /// whether `operand` is of the type written `ty` (see the module
    /// documentation): the type is resolved with every bare type present, and
    /// may contain no future and no function type; the operand is checked, an
    /// untyped integer settling to `Int`, and measured at the top, or deeply if
    /// the test's plan walks into it (see
    /// [`TypeTable::test_plan`](crate::types::TypeTable::test_plan)), subject
    /// to the purity rule. Each member of the operand's type once measured,
    /// made present, *always passes* if it is a subtype of the type tested,
    /// *may pass* if it does not but overlaps it (see
    /// [`TypeTable::overlaps`](crate::types::TypeTable::overlaps)), and *cannot
    /// pass* otherwise. If every member always passes, the test is `true`,
    /// which is a warning; if none passes or may, it is the error "`x` is never
    /// a String", at the type. Records the plan for lowering (see
    /// [`Resolution::test_of`](super::Resolution::test_of)). `None` if the
    /// type, the operand or the test is erroneous.
    pub(super) fn check_test(
        &mut self,
        id: ExprId,
        operand: ExprId,
        ty: TypeExprId,
        keyword: &str,
    ) -> Option<Tested> {
        let target = self.resolve_type(ty, Bareness::Present);
        let found = self.check_expr(operand);
        let found = self.settle(operand, found);
        if target == TyId::ERROR || found == TyId::ERROR || !self.testable(target, ty, keyword) {
            return None;
        }
        // An operand of type `Never` is never there to test.
        if found == self.never {
            return Some(Tested {
                narrowed: self.never,
                remainder: None,
                whole: false,
                awaited: false,
                unreached: true,
            });
        }
        let present = self.types.present(found);
        let present_members = self.flat_members(present);
        // What a secret is decides nothing (see [Secrets](crate::check::secrets#secrets)).
        if present_members
            .iter()
            .any(|&member| self.types.secret_value(member).is_some())
        {
            self.no_branching(operand, &format!("`{keyword}`"));
            return None;
        }
        let plan = self.types.test_plan(Some(&present_members), target);
        let awaits = self.awaits[operand.index()];
        let used = if plan.walks() {
            self.measure_deep(operand, found)
        } else {
            self.measure(operand, found)
        };
        let awaited = self.awaits[operand.index()] != awaits;
        let (mut always, mut rest, mut may) = (Vec::new(), Vec::new(), false);
        for member in self.flat_members(used) {
            let shown = self.types.present(member);
            if self.types.is_subtype(shown, target) {
                always.push(member);
            } else {
                may |= self.types.overlaps(shown, target);
                rest.push(member);
            }
        }
        if always.is_empty() && !may {
            self.never_error((operand, ty), found, target);
            return None;
        }
        let (all, whole) = (rest.is_empty(), always.is_empty());
        if all {
            self.always_warning(id, operand, target, keyword);
        }
        if may {
            always.push(target);
        }
        let narrowed = self.types.union(always);
        let remainder = (!all).then(|| self.types.union(rest));
        let resolution = TestResolution {
            test: if all { ValueTest::Pass } else { plan },
            slot: Slot::NONE,
            subject: self.subject(operand).into(),
            target: self.with_article(target).into(),
        };
        self.tests.insert(id, resolution);
        Some(Tested {
            narrowed,
            remainder,
            whole,
            awaited,
            unreached: false,
        })
    }

    /// Whether `target`, the type written `ty` that the `is` or `as`, as
    /// `keyword` says, tests for, can be tested: it holds no future, nor an
    /// either shape, which "`is` cannot test a future" reports at the type,
    /// and no function type, which "`is` cannot test a function type"
    /// reports, since a function's signature is not something a value can
    /// be asked for.
    pub(super) fn testable(&mut self, target: TyId, ty: TypeExprId, keyword: &str) -> bool {
        let span = self.ast.type_expr(ty).span;
        if self.types.present(target) != target {
            self.errors.push(
                Diagnostic::error(format!("`{keyword}` cannot test a future"), span)
                    .with_note("await the value and test what it yields"),
            );
            return false;
        }
        if self.types.holds_untestable(target) {
            self.errors.push(
                Diagnostic::error(format!("`{keyword}` cannot test a function type"), span)
                    .with_note(
                        "a function's signature is not something a value can be asked for; tell \
                         a function apart by the other members, in the `else` branch",
                    ),
            );
            return false;
        }
        true
    }

    /// Reports that no member of `found`, the type of `operand`, the
    /// operand of an `is` or `as`, may be a `target`, the type written
    /// `ty`: "`x` is never a String", at the type, labelled with the
    /// operand's type at the operand.
    pub(super) fn never_error(
        &mut self,
        (operand, ty): (ExprId, TypeExprId),
        found: TyId,
        target: TyId,
    ) {
        let message = format!(
            "{} is never {}",
            self.subject(operand),
            self.with_article(target)
        );
        let label = format!("this is {}", self.in_label(found));
        self.errors.push(
            Diagnostic::error(message, self.ast.type_expr(ty).span)
                .with_context(self.ast.get(operand).span, label),
        );
    }

    /// Warns that every member of the operand's type of the `is` or `as`,
    /// as `keyword` says, expression `id` is a `target`: for `is`, "`x` is
    /// always an Int", since the test is `true`; for `as`, "`as` changes
    /// nothing here: `x` is already an Int", with a note that an ascription
    /// says the same without a test.
    pub(super) fn always_warning(
        &mut self,
        id: ExprId,
        operand: ExprId,
        target: TyId,
        keyword: &str,
    ) {
        let (subject, target_text) = (self.subject(operand), self.with_article(target));
        let span = self.ast.get(id).span;
        let warning = if keyword == "as" {
            let written = self.types.display(target, self.ast.names()).to_string();
            Diagnostic::warning(
                format!("`as` changes nothing here: {subject} is already {target_text}"),
                span,
            )
            .with_note(format!("`: {written}` says the same without a test"))
        } else {
            Diagnostic::warning(format!("{subject} is always {target_text}"), span)
        };
        self.warnings.push(warning);
    }

    /// The operand `operand` of an `is` or `as` as its messages name it:
    /// as written, in backticks, if it is a plain name or a dotted path, as
    /// in "`x`", and "this value" otherwise.
    pub(super) fn subject(&self, operand: ExprId) -> String {
        let mut node = operand;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        match self.path_text(node) {
            Some(text) => format!("`{text}`"),
            None => "this value".to_owned(),
        }
    }

    /// Type `ty` as a message shows it (see [`Checker::in_message`]), after
    /// its indefinite article: "an Int", "a String", "a `List<Int>`", "an
    /// `Int | String`".
    pub(super) fn with_article(&mut self, ty: TyId) -> String {
        let shown = self.in_message(ty);
        let first = shown
            .chars()
            .find(char::is_ascii_alphanumeric)
            .unwrap_or('x');
        let starts_type = shown.trim_start_matches('`').starts_with(first);
        let article = if starts_type && "AEIOUaeiou".contains(first) {
            "an"
        } else {
            "a"
        };
        format!("{article} {shown}")
    }

    /// The members of `ty` if it is a union, and `ty` alone otherwise.
    pub(super) fn flat_members(&self, ty: TyId) -> Vec<TyId> {
        match self.types.union_members(ty) {
            Some(members) => members.to_vec(),
            None => vec![ty],
        }
    }

    /// Types the branches of an `if` with `else`: they have their join (see
    /// [`TypeTable::join`](crate::types::TypeTable::join)), an untyped branch
    /// first adapting to the other branch's `Int` or `Float` once stripped, or
    /// settling to `Int`, and two untyped branches leaving the `if` untyped. A
    /// join that is widened, a union of what the branches are where they do not
    /// agree, is a warning (see [`Checker::branches_widened`]).
    ///
    /// Two branches of the same type have it, a provisional type of the
    /// innermost function's included, so a parameter passes through an
    /// `if` unmeasured. Otherwise every parameter of the function whose
    /// provisional type is in either branch is measured, conservatively,
    /// and the branches join with those parameters present.
    pub(super) fn branches(
        &mut self,
        (then, then_ty): (ExprId, TyId),
        (else_, else_ty): (ExprId, TyId),
        what: &str,
    ) -> TyId {
        if then_ty == else_ty {
            return then_ty;
        }
        let (then_ty, else_ty) = (self.claim(then, then_ty), self.claim(else_, else_ty));
        let then_ty = self.adapt_to(then, then_ty, else_ty);
        let else_ty = self.adapt_to(else_, else_ty, then_ty);
        let (joined, widened) = self.types.join(then_ty, else_ty);
        if widened {
            self.branches_widened((then, then_ty), (else_, else_ty), (joined, what));
        }
        joined
    }

    /// Warns that the branches of an `if`, or a `recover`'s body and
    /// fallback, `what` it is, `then` and `else_`, of types `then_ty` and
    /// `else_ty`, have different types, which `joined`, its type, widened
    /// to a union of: at the `else` branch's value (see
    /// [`Checker::branch_value`]), labelled with its type, with a label of
    /// the other branch's on its value, and a note naming the union.
    pub(super) fn branches_widened(
        &mut self,
        (then, then_ty): (ExprId, TyId),
        (else_, else_ty): (ExprId, TyId),
        (joined, what): (TyId, &str),
    ) {
        let then_label = format!("this is {}", self.in_label(then_ty));
        let else_label = format!("this is {}", self.in_label(else_ty));
        let note = format!(
            "the {what} is {}; annotate it with that type if both are meant, or tell them apart \
             with `is`",
            self.in_message(joined)
        );
        let (then_at, else_at) = (self.branch_value(then), self.branch_value(else_));
        self.warnings.push(
            Diagnostic::warning(
                format!("the branches of this {what} have different types"),
                self.ast.get(else_at).span,
            )
            .with_label(else_label)
            .with_context(self.ast.get(then_at).span, then_label)
            .with_note(note),
        );
    }

    /// The expression that gives the value of `branch`, an `if`'s branch:
    /// through any parentheses and blocks, each block's result, if it has
    /// one; `branch` itself if it is not a block with a result.
    pub(super) fn branch_value(&self, branch: ExprId) -> ExprId {
        let mut node = branch;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items: body, .. } if self.has_result(body) => {
                    match self
                        .ast
                        .items(body)
                        .last()
                        .map(|&last| self.ast.item(last).kind)
                    {
                        Some(ItemKind::Result(value) | ItemKind::Expr(value)) => node = value,
                        _ => return node,
                    }
                }
                _ => return node,
            }
        }
    }

    /// Whether an integer literal, of the untyped integer type, fits
    /// `expected`: if it is `Int` or `Float`; and since a literal is present,
    /// the either shape of one, `Int | Future<Int>`, as it would `Int`, and a
    /// provisional one, which stands for its present type as a supertype;
    /// and a union with a numeric member, as that member.
    pub(super) fn untyped_fits(&self, expected: TyId) -> bool {
        matches!(expected, TyId::INT | TyId::FLOAT)
            || (self.types.laziness(expected) != Laziness::Future
                && self.numeric_member(self.types.strip(expected)).is_some())
    }

    /// The type of `expr`, of type `ty`, once an untyped integer in it
    /// adapts to `other`, the type of the expression it must agree with (see
    /// [`Checker::numeric_target`]). Any other type is returned as it is.
    pub(super) fn adapt_to(&mut self, expr: ExprId, ty: TyId, other: TyId) -> TyId {
        if ty != TyId::UNTYPED_INT {
            return ty;
        }
        let target = self.numeric_target(other);
        self.coerce(expr, target);
        target
    }

    /// Types an `if` without `else`, whose keyword is at `keyword`: it is
    /// `{}`, so its block must be too, which measures a parameter of the
    /// innermost function whose provisional type is in the block's type.
    pub(super) fn empty_branch(&mut self, keyword: Span, then: ExprId, then_ty: TyId) -> TyId {
        match self.claim(then, then_ty) {
            TyId::EMPTY => TyId::EMPTY,
            TyId::ERROR => TyId::ERROR,
            // A block that never completes gives no value to drop: the
            // `if` is `{}`, as the `else` it lacks is.
            ty if ty == self.never => TyId::EMPTY,
            _ => {
                let then_ty = self.settle(then, then_ty);
                let label = format!("this branch is {}", self.in_label(then_ty));
                self.errors.push(
                    Diagnostic::error("`if` without `else` must be `{}`", keyword)
                        .with_context(self.ast.get(then).span, label)
                        .with_note("add an `else` branch, or make the block's result `{}`"),
                );
                TyId::ERROR
            }
        }
    }

    /// The note on an error where `operand`, of type `ty`, is used as one
    /// particular type and is a union, once stripped: "tell them apart
    /// first, as in `if x is Int { ... }`", naming the variable when
    /// `operand` is one, through any parentheses, and the union's first
    /// member as it shows it that `is` can test (one that is not a function
    /// type); for a union of function types only, which `is` cannot tell
    /// apart, a note saying to annotate the value with one function type
    /// instead. `None` if `ty` is not a union.
    pub(super) fn union_note(&mut self, operand: ExprId, ty: TyId) -> Option<String> {
        let ty = self.shown(ty);
        let members = self.types.union_members(self.types.strip(ty))?.to_vec();
        let mut node = operand;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let name = match self.ast.get(node).kind {
            ExprKind::Name(name) => self.ast.names().get(name),
            _ => "x",
        };
        let example = self
            .types
            .shown_members(&members, self.ast.names())
            .into_iter()
            .find(|&(member, _)| self.types.fn_sig(member).is_none());
        Some(match example {
            Some((_, shown)) => {
                format!("tell them apart first, as in `if {name} is {shown} {{ ... }}`")
            }
            None => "`is` cannot tell function types apart; annotate the value with one \
                     function type that each of them fits"
                .to_owned(),
        })
    }
}
