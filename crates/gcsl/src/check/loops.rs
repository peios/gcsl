//! Checking loops and jumps: `for` and `while`, labels, `break`, `continue` and
//! `return`, and the values loops and blocks leave with.
//!
//! # For
//!
//! `for x in xs if GUARD { BODY }` is the list of the body's values for the
//! elements of `xs`, in order, that pass the guard (`Checker::check_for`).
//! The iterable is checked and measured at the top, as the list of an
//! index is: awaited if it may be a future, subject to the purity rule, and
//! a pure function's bare parameter measured. It must then be a list:
//! "cannot iterate over a value of type T: it is not a list", at the
//! iterable. Its elements are not measured.
//!
//! The variable is declared in a scope of its own, which holds the guard and
//! the body and nothing else, as a block's scope holds its items: it takes the
//! next free slot of the enclosing function's frame (see
//! [`Resolution::for_slot`](super::Resolution::for_slot)), which the `for`
//! leaves when it ends, and has the element type as it is, which may be the
//! either shape or a future. It shadows as a `let` does, and since the body is
//! a block of its own, `:=` of it there is "declared in an enclosing block". A
//! `for` is not a function: the guard and the body are checked in the enclosing
//! function, whose purity governs their awaits and calls and whose captures
//! they use, so a closure or a `lazy` made in the body captures the variable as
//! it would any variable of the enclosing function, by value, which is its
//! value in that iteration. The guard is checked as an `if`'s condition is:
//! measured, and `Bool`. Being blocks of the enclosing function, the guard and
//! the body may rebind its mutable variables (see [Mutable
//! variables](crate::check::mutation#mutable-variables)), and the guard for
//! each element reads what the body wrote for the elements before it.
//!
//! Every loop, of any form (see [Loops](#loops)), is the list of what its
//! iterations contribute, `List<E>`, `E` being its *element type*
//! (`Checker::element_type`): the join of what can contribute, the body's
//! type and the type of the value of every `break E` and `continue E` for
//! it (see [`break` and `continue`](#break-and-continue)), treated exactly
//! as the elements of a list literal are (see [Lists](crate::check::lists#lists) and
//! [Unions](crate::check::narrowing#unions)), in the order they are checked, the body last. Where
//! no list type is expected, they are joined, untyped integers adapting to
//! each other and settling to `Int` as a list literal's elements do, and a
//! widened join is a warning, "the values this loop collects have
//! different types", at the loop's keyword, labelled at each contributing
//! site with its type (the body at its value, through blocks, as an `if`'s
//! branch is shown), with a note naming the loop's type as a list's does.
//! Where a list type `List<X>`, or its either shape, is expected, the
//! expectation reaches the loop as it reaches a list literal, and the body
//! and each value are checked against `X` as a literal's element is
//! (`Checker::fit`): an untyped integer adapts, a value that fits once
//! awaited is awaited, and a record block is checked field by field; where
//! `X` is a record type with defaults, a record literal that a body or a
//! value that is a block results in, or an `if`'s branches do, is given
//! them (see "Field defaults" under [Functions](crate::check::functions#functions)), as is one a
//! spread `for`'s body results in where the list's elements are expected
//! to be of such a type, or the body of a `for` that a spread block or `if`
//! results in there (see [Lists](crate::check::lists#lists)). `E` is
//! then the join of what they are once those awaits have run, the type of
//! every value the evaluator collects, or `X` if that is not a subtype of
//! `X`, as for a literal, and is no warning. A bare `break` or `continue`
//! contributes nothing, and a contribution of type `Never` joins as
//! nothing (see [Never](crate::check::narrowing#never)), so `for x in xs { break x }` is
//! `List<Int>`. If nothing can contribute, as in `for { break }`, `E` is
//! the expected element type, or `{}` without one, so that such a loop is
//! unit-like as a statement. A body that can fall off its end contributes
//! its own value: `for x in xs { if x > 5 { break x } }` is `List<Int |
//! {}>`, with the warning, which a final `continue` removes. A body that
//! is a record block makes a list of records. That list is the loop's
//! *ordinary* type, what it is when it ends: a loop that cannot end is
//! `Never` instead (see [Never-ending loops](#never-ending-loops)), and a
//! loop a `return` leaves is the join of its ordinary type and the values
//! returned (see [Loops that return](#loops-that-return)).
//!
//! # Loops
//!
//! Besides the iteration, a loop is a condition loop, `for COND { ... }`,
//! with a header, `for BEFORE; COND; AFTER { ... }`, or without a condition,
//! `for { ... }`, the loop whose condition is `true`; or a `do` loop, `do {
//! ... } for COND`, whose body runs before its condition. Each is an
//! expression of the type above, with a scope of its own, as an
//! iteration's: the header's items are declared in it, so they are visible
//! in the later parts of the header and in the body, an inner block, and not
//! after the loop. A condition is checked as an `if`'s is, measured and
//! `Bool` ("`for` conditions are Bool"), once, and narrows nothing; a
//! `do`'s is checked after the body, outside the body's block, whose
//! variables it cannot see. The enclosing function's purity governs the
//! awaits and calls of every part, and a loop is allowed in a pure
//! function.
//!
//! A header's items are checked as a block's (`Checker::header_item`):
//! before the condition, `let`s, of any form a block allows, and `:=`s;
//! after it, `:=`s only. A plain `let` there declares a variable checked as
//! a `let mut`'s is, one slot for its life and its type fixed by its
//! initialiser or annotation, a dotted or indexed `:=` of it having to keep
//! that type, but of kind `BindingKind::Header`, which only a `:=` of the
//! header itself, its own block, may rebind, in its one slot
//! (`Checker::header_rebind`); one from the body, an inner block, is
//! "cannot rebind `i` here: it is declared in the header of a `for`",
//! labelled at the declaration, with a note to write `let mut` in the
//! header, and one from a function nested in the loop is the error of any
//! variable declared in an enclosing block. A `let mut` in the header is a
//! mutable variable, which the body may rebind too (see [Mutable
//! variables](crate::check::mutation#mutable-variables)). A `:=` in the header of a variable from
//! outside the loop follows the rules of any `:=`. Each iteration sees the
//! variable's slot as the after-items left it, and a closure made in the
//! body copies it, so it captures that iteration's value.
//!
//! # `break` and `continue`
//!
//! `break`, `continue`, each with an optional value and label, are items of a
//! block, never expressions (see [`crate::parser`]). A block whose last item is
//! one is of type `Never`. Each is for a *target loop* (`Checker::jump_target`,
//! recorded for lowering in
//! [`Resolution::jump_target`](super::Resolution::jump_target)): without a
//! label, the innermost loop of the function being checked whose *body* holds
//! it, a loop's iterable, guard, condition and header items being outside its
//! body; with one, the loop of that label whose body holds it. To find it, the
//! checker keeps the loops whose bodies are being checked on a stack
//! (`LoopFrame`), with the labelled blocks whose items are (see
//! [Labels](#labels)), each with the number of functions being checked when its
//! body began, so that a loop outside the innermost function or `lazy` is not a
//! target. A `break` for a loop is recorded on its frame, for [Never-ending
//! loops](#never-ending-loops). Errors, each at the keyword and its label:
//!
//! - no loop holds it: "`break` outside a loop" (or "`continue`");
//! - only loops outside the innermost function or `lazy` hold it, or have
//!   its label: "`break` cannot leave a function" (or "a `lazy`"),
//!   labelled "inside a function here" at the function's keyword, and, for
//!   a label, "the loop it names is outside it" at the loop's label, with a
//!   note that the body of a function runs when it is called (a `lazy`'s
//!   when it is awaited), which may be after the loop has ended;
//! - no loop it could be for has its label: "no enclosing loop is labelled
//!   `'L`", at the label;
//! - it is an item of a record block: "`break` cannot be an item of a
//!   record block".
//!
//! Its value, if any, is what it contributes to its target's list (see
//! [For](#for)), checked against the target's expected element type if it
//! has one; where there is no target, it is checked on its own.
//!
//! # Labels
//!
//! A loop's label, `for'L` or `do'L`, names it for the `break'L`,
//! `continue'L` and `return'L` in its body, and a block's, `{'L ... }`,
//! names it for the `return'L` in it (see [Labelled
//! blocks](#labelled-blocks)). Loop labels and block labels are one
//! namespace, kept on one stack, [`Checker::loops`], of the loops whose
//! bodies and the labelled blocks whose items are being checked
//! (`LoopFrame`; an unlabelled block has none, and costs nothing). A loop
//! or labelled block labelled `L` inside the body of one labelled `L` of the
//! same function is "label `'L` is already in use", at the inner label,
//! labelled "the loop around this one is labelled so" (or "the block") at
//! the outer, with the note "give the inner loop another label" (or
//! "block"); a function or `lazy` nested in a loop or block starts with no
//! labels, and an unused label is fine. A labelled `break` or `continue`
//! leaves every loop between it and its target, and its value joins the
//! target's element type only. An unlabelled `break` or `continue` is for
//! the innermost loop, whatever labelled blocks lie between; a labelled one
//! whose label names a block is "`'L` labels a block, not a loop", at the
//! keyword and its label, labelled "the block is labelled here" at the
//! block's label, with the note "use `return'L` to leave a block".
//!
//! # `return`
//!
//! `return`, `return EXPR`, `return'L` and `return'L EXPR` are items, as
//! `break` is (`Checker::check_return`). Each is for a *target*, recorded
//! for lowering in [`Resolution::target`](super::Resolution::target):
//!
//! - unlabelled, the innermost function or `lazy` being checked, however
//!   many blocks, labelled blocks and loops of it lie between; at the top
//!   level, outside any, it is "`return` outside a function";
//! - labelled `L`, the innermost loop or labelled block labelled `L`
//!   holding it, of the same function or `lazy`, found as a `break`'s loop
//!   is ([`Checker::jump_target`]): none at all is "no enclosing block or
//!   loop is labelled `'L`", at the label, and one only outside the
//!   innermost function or `lazy` is "`return` cannot leave a function" (or
//!   "a `lazy`"), as for `break`, labelled "the block it names is outside
//!   it" (or "the loop") at its label. A loop's iterable, guard, condition
//!   and header items are not in its body, so a `return'L` there is not for
//!   it.
//!
//! As an item of a record block it is "`return` cannot be an item of a
//! record block". A block whose last item is a `return` is `Never`, and a
//! `return` never completes, for the unreachable warning, unless it is in
//! error. A `return` without a value returns `{}`.
//!
//! Each target has *result sites*: its own result, and the value of every
//! `return` for it (a `Site`, gathered on its frame as it is checked). How
//! they combine depends on the target:
//!
//! - **A function.** Every value returned is treated exactly as the body's
//!   result is, at every place the checker treats that result specially:
//!   it is checked against the declared result type, as the body's result
//!   is, with [`Checker::check_expr_expecting`] (a mismatch is "mismatched
//!   types" at the value, labelled "the return type is declared here" at
//!   the type, and a `return` without a value that `{}` does not fit is the
//!   same at the `return`, with a note that it returns `{}`), so that an
//!   untyped literal adapts, a value that fits once awaited is awaited, a
//!   record block is checked field by field and a list literal, a block
//!   with a result or an `if` against a list type or a union, as the body
//!   is, and a record literal such a block or `if` results in is given the
//!   defaults of a record result type ("Field defaults" under
//!   [Functions](crate::check::functions#functions)); the pure function's parameters it holds are
//!   measured where the declared type is present ([Inferred
//!   parameters](crate::check::inference#inferred-parameters), through `Checker::expect`); a
//!   callback parameter returned is used as a value, and a call of one in
//!   it records its argument ([Narrowed
//!   callbacks](crate::check::inference#narrowed-callbacks)), as anywhere in the body; and an
//!   impure or memoised function's bare result is inferred from every
//!   site, each as the body's result alone would be, dotted fields
//!   included (see [Inferred results](crate::check::inference#inferred-results)).
//! - **A `lazy`.** A `return` leaves the `lazy`, whatever lies between,
//!   and its value is the future's: it does not leave the function around
//!   the `lazy`, which may have finished, and is no result site of it. The
//!   `lazy` is `Future<T>`, `T` the join of its operand's type and every
//!   value returned, as an `if`'s branches are joined, untyped integers
//!   adapting to each other and then settling to `Int`; a join that widens
//!   is a warning, "the values this `lazy` returns have different types",
//!   at its keyword, labelled at each site, with the union note. Nothing
//!   is expected of a `lazy`'s operand, so nothing is of its `return`s: a
//!   type expected of the `lazy` is fitted by the `Future` as a whole, as
//!   before (`Checker::returned_lazy`).
//! - **A labelled block or a loop**: see [Labelled
//!   blocks](#labelled-blocks) and [Loops that return](#loops-that-return).
//!
//! # Labelled blocks
//!
//! A block labelled `L`, `{'L ... }`, any block that is not a record block
//! (which the parser rejects), evaluates to the value of a `return'L` in
//! it at once. It is checked as any block is, with a frame on
//! [`Checker::loops`] around its items (`Checker::labelled_block`, and
//! for a function's body, which is a block like any other,
//! `Checker::check_fn_body`); its type is the join of its own result and
//! every value returned to it (`Checker::leave_block`), as an `if`'s
//! branches are joined: untyped integers adapt to each other and to the
//! numeric types beside them, and stay untyped if all are, a later
//! [`Checker::coerce`] reaching the values returned as it reaches the
//! block's result. Where a list type or a union is expected of the block
//! (as it reaches a block's result: see [Lists](crate::check::lists#lists)), each value
//! returned is checked against it, as the result is, and the join is no
//! warning; a record type with defaults, which reaches a record literal
//! its result is for its defaults alone (see "Field defaults" under
//! [Functions](crate::check::functions#functions)), reaches no value returned, which is checked
//! as ever; otherwise a join that widens is a warning, "the values this
//! block returns have different types", at its label, labelled at each
//! site, with the union note. A block every way through which returns is
//! of the values' type; a block that returns on no path is `Never` if its
//! result is. A labelled loop body is a labelled block like any other: in
//! `for x in xs {'it if x < 0 { return'it 0 }; x }`, `0` is what the
//! iteration contributes, as `continue 0` would make it.
//!
//! # Loops that return
//!
//! A `return'L` whose `L` labels a loop makes the whole loop evaluate to
//! its value at once, not to a list. The loop's type is the join of its
//! ordinary type, its list or `Never` (see [Never-ending
//! loops](#never-ending-loops)), and every value returned to it
//! ([`Checker::returned_loop`]). Without an expected type, untyped values
//! settle against the list, and a join that widens, as `List<Int> | Int`
//! does, is a warning, "this loop is its list or a value returned to it, of
//! different types", at the loop's keyword, labelled "its list is
//! List<Int>" there and with its type at each `return`, with the union
//! note. Where a type `T` is expected of it, which reaches a loop directly,
//! through parentheses, wherever an expected type reaches an expression,
//! and through a block's result or an `if`'s branches where a list type or
//! a union is expected ([`Checker::check_loop_expecting`]): every value
//! returned is checked against `T`; its body is checked against the
//! element type of `T`'s list type, when `T` is one, or a union with
//! exactly one, as before, and against nothing otherwise; and a loop that
//! can end must fit `T` as a list too, or it is "this loop may end as a
//! list, `List<{}>`, which is not Int", at its keyword, with the note
//! "annotate it as `Int | List<{}>`, or return from every way the loop can
//! end". A loop no `return` leaves keeps the generic mismatch where its
//! list does not fit. Which loops a `return` leaves is recorded for
//! lowering in [`Resolution::loop_of`](super::Resolution::loop_of).
//!
//! # Never-ending loops
//!
//! A `for BLOCK` loop, written with no condition (not `for true`, nor a `do`
//! loop), that no `break` is for, its own or a labelled one from a loop inside
//! it, can never end normally: its ordinary type is `Never`, not a list,
//! whatever its body's type, and it builds no list
//! ([`Resolution::loop_of`](super::Resolution::loop_of)). So `let v: Int =
//! for'spin { ... return'spin r }` is an `Int` with no union and no warning, a
//! bare `for { ... }` statement with no `break` is `Never`, and what follows it
//! in its block gets the unreachable warning; a function whose only result site
//! is such a loop has a body of type `Never`, from which its result is inferred
//! (a present `T`, honestly: it never returns a future). Its body and its
//! `continue`s still make an element type, as any loop's, with the same
//! warning, though no list is ever built.

use crate::ast::{ExprId, ForHead, ForId, Item, ItemId, ItemKind, ItemRange, Jump, Label, LabelId};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;
use crate::value::ValueKind;

use super::{
    BindingKind, Checker, Expecting, JumpTarget, LoopFrame, RESULT_WHY, Site, Stated, jump_span,
};

impl Checker<'_> {
    /// Types the loop `id`, whose details are `for_id` (see the module
    /// documentation, [For](#for) and [Loops](#loops)). The loop has a
    /// scope of its own, as a block has, which holds an iteration's
    /// variable, or a header's items, and the body; a `do` loop's condition
    /// is outside its body's block. The body is checked with the loop on
    /// [`Checker::loops`], so that the `break`s and `continue`s in it find
    /// it, and what they and the body contribute make the loop's element
    /// type (see [`Checker::element_type`]). With `expected`, the type
    /// expected of the whole loop, with where that is stated and its label,
    /// every `return` for the loop is checked against it, and if it has a
    /// list type (see [`Checker::expected_member`]), the body and each
    /// value against that list's element type, as a list literal's elements
    /// are (see [`Checker::fit`]). Without one, but with `fill`, a record
    /// type with defaults that the loop's elements are expected to be of,
    /// with where that is stated and its label, as a spread's in a list
    /// literal are, the body is checked for a record literal it results in
    /// alone (see [`Checker::fill_result`]), and nothing else of the loop
    /// is. The loop's ordinary type is the list of
    /// its element type, or `Never` if what always runs first, an iterable
    /// or a condition before the first iteration, is, or if it cannot end
    /// (see [Never-ending loops](#never-ending-loops)); `Error` if the
    /// iterable, the body or a value is erroneous. A loop that a `return`
    /// leaves is the join of that and the values returned (see
    /// [`Checker::returned_loop`]).
    pub(super) fn check_for(
        &mut self,
        id: ExprId,
        for_id: ForId,
        (expected, fill): (Option<Stated<'_>>, Option<Stated<'_>>),
    ) -> TyId {
        let element = expected.and_then(|(whole, context)| {
            let list = self.expected_member(whole, ValueKind::List)?;
            Some((
                self.types.list_element(list).unwrap_or(TyId::ERROR),
                context,
            ))
        });
        let decl = *self.ast.for_decl(for_id);
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let outer_spreads = std::mem::take(&mut self.spreads);
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        let head = match decl.head {
            ForHead::Each {
                name,
                name_span,
                iterable,
                guard,
            } => self.each_head(id, for_id, (name, name_span), (iterable, guard)),
            ForHead::While {
                before,
                cond,
                after,
            } => self.while_head(id, before, cond, after),
            ForHead::Do { .. } => TyId::EMPTY,
        };
        let owned = |(ty, (context, why)): (TyId, (Span, &str))| (ty, context, why.to_owned());
        let frame = LoopFrame::new(
            (Some(for_id), id),
            decl.label,
            self.functions.len(),
            element.map(owned),
            expected.map(owned),
        );
        self.enter_target(frame);
        let body = match (element, fill) {
            (Some((element, context)), _) => self.fit(decl.body, element, context),
            (None, Some((record, context))) => self.fill_result(decl.body, record, context),
            (None, None) => self.check_expr(decl.body),
        };
        let frame = self.loops.pop();
        if let ForHead::Do { cond } = decl.head {
            // Outside the body's block, and asked only after the body: it
            // makes the loop `Never` in no case.
            self.loop_condition(id, cond);
        }
        self.bindings.truncate(self.block_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        self.spreads = outer_spreads;
        let (values, broken, returns) = frame
            .map(|frame| (frame.values, frame.broken, frame.returns))
            .unwrap_or_default();
        let collected = self.element_type(id, (decl.body, body), values, element);
        // A loop with no condition ends only by a `break`.
        let endless = matches!(decl.head, ForHead::While { cond: None, .. }) && !broken;
        if let Some(resolution) = self.loop_resolutions.get_mut(for_id.index()) {
            resolution.endless = endless;
        }
        let ordinary = if head == TyId::ERROR || collected == TyId::ERROR {
            TyId::ERROR
        } else if head == self.never || endless {
            self.never
        } else {
            self.types.list(collected)
        };
        if returns.is_empty() {
            return ordinary;
        }
        self.returned_loop(id, ordinary, &returns, expected)
    }

    /// The type of the loop `id`, whose ordinary type, what it is when it
    /// ends, is `ordinary`, and which the `return`s `returns` leave, with
    /// `expected`, the type expected of the whole loop, if one is, with
    /// where that is stated and its label: the join of its ordinary type
    /// and the values returned (see [`return`](#return)). Under an expected
    /// type each value fits it already, and a loop that can end must fit it
    /// as a list too: otherwise it is "this loop may end as a list,
    /// `List<{}>`, which is not Int", at its keyword, and `Error`. Without
    /// one, a join that widens is a warning, "this loop is its list or a
    /// value returned to it, of different types".
    pub(super) fn returned_loop(
        &mut self,
        id: ExprId,
        ordinary: TyId,
        returns: &[Site],
        expected: Option<(TyId, (Span, &str))>,
    ) -> TyId {
        if ordinary == TyId::ERROR {
            return TyId::ERROR;
        }
        let keyword = self.ast.loop_keyword(id);
        let mut sites = Vec::with_capacity(returns.len() + 1);
        sites.push(Site {
            value: None,
            at: keyword,
            ty: ordinary,
        });
        sites.extend_from_slice(returns);
        let Some((whole, _)) = expected else {
            let (joined, widened) = self.join_sites(id, &sites, true);
            if widened {
                let note = self.union_note_of("loop", joined);
                let list = format!("its list is {}", self.in_label(ordinary));
                let mut warning = Diagnostic::warning(
                    "this loop is its list or a value returned to it, of different types",
                    keyword,
                )
                .with_label(list);
                let never = self.never;
                for site in returns.iter().filter(|site| site.ty != never) {
                    let label = format!("this is {}", self.in_label(self.site_type(site)));
                    warning = warning.with_context(site.at, label);
                }
                self.warnings.push(warning.with_note(note));
            }
            return joined;
        };
        // An expected type in error has been reported.
        if whole == TyId::ERROR {
            return TyId::ERROR;
        }
        if ordinary != self.never && !self.types.is_subtype(ordinary, whole) {
            let (list, shown) = (self.in_message(ordinary), self.in_message(whole));
            let (joined, _) = self.types.join(whole, ordinary);
            let annotation = self.types.display(joined, self.ast.names()).to_string();
            self.errors.push(
                Diagnostic::error(
                    format!("this loop may end as a list, {list}, which is not {shown}"),
                    keyword,
                )
                .with_note(format!(
                    "annotate it as `{annotation}`, or return from every way the loop can end"
                )),
            );
            return TyId::ERROR;
        }
        // Each value fits what is expected, so the join is no warning; and
        // what is expected describes them all should it not be a subtype.
        let (joined, _) = self.join_sites(id, &sites, true);
        if joined == TyId::ERROR || self.types.is_subtype(joined, whole) {
            joined
        } else {
            whole
        }
    }

    /// Checks the head of the iteration `id`, whose details are `for_id`,
    /// in the loop's scope: the iterable, outside it, measured at the top,
    /// which must be a list, "cannot iterate over a value of type T: it is
    /// not a list", its elements not measured; the variable `name`, written
    /// at `name_span`, declared with the element type as it is in the next
    /// free slot of the current frame, which the loop leaves when it ends;
    /// and the guard, if any, checked as an `if`'s condition is. Returns
    /// `Error` if the iterable is erroneous, `Never` if it is of that type,
    /// the variable then being `Never` too, and `{}` otherwise.
    pub(super) fn each_head(
        &mut self,
        id: ExprId,
        for_id: ForId,
        (name, name_span): (Name, Span),
        (iterable, guard): (ExprId, Option<ExprId>),
    ) -> TyId {
        let ty = self.check_expr(iterable);
        let ty = self.settle(iterable, ty);
        let ty = self.measure(iterable, ty);
        // An iterable of type `Never` is never there to iterate over.
        let item = if ty == self.never {
            self.never
        } else {
            self.iterated(iterable, ty)
        };
        let slot = self.declare_binding(name, item, Some(name_span), BindingKind::Variable);
        self.for_slots[for_id.index()] = slot;
        if let Some(guard) = guard {
            let keyword = self.ast.loop_keyword(id);
            let ty = self.check_expr(guard);
            self.expect(guard, ty, TyId::BOOL, keyword, "`for` guards are Bool");
        }
        match item {
            TyId::ERROR => TyId::ERROR,
            item if item == self.never => item,
            _ => TyId::EMPTY,
        }
    }

    /// Checks the header of the condition loop `id`, in the loop's scope:
    /// the items `before` its condition, in order, the condition `cond`, if
    /// it has one (see [`Checker::loop_condition`]), and the items `after`
    /// it. A plain `let` there declares a variable that only the header may
    /// rebind (see [`Checker::header_item`]). Returns `Never` if the
    /// condition, which runs before the first iteration, is of that type,
    /// and `{}` otherwise.
    pub(super) fn while_head(
        &mut self,
        id: ExprId,
        before: ItemRange,
        cond: Option<ExprId>,
        after: ItemRange,
    ) -> TyId {
        for &item in self.ast.items(before) {
            self.header_item(item);
        }
        let cond = cond.map(|cond| self.loop_condition(id, cond));
        for &item in self.ast.items(after) {
            self.header_item(item);
        }
        match cond {
            Some(cond) if cond == self.never => cond,
            _ => TyId::EMPTY,
        }
    }

    /// Checks the condition `cond` of the loop `id`, which is checked once
    /// and evaluated before every iteration, or after each for a `do` loop:
    /// as an `if`'s condition is, measured, and `Bool`, but narrowing
    /// nothing. Returns its type.
    pub(super) fn loop_condition(&mut self, id: ExprId, cond: ExprId) -> TyId {
        let keyword = self.ast.loop_keyword(id);
        let ty = self.check_expr(cond);
        self.expect(cond, ty, TyId::BOOL, keyword, "`for` conditions are Bool");
        ty
    }

    /// Checks the item `id` of a `for` header: a `let`, checked as in a
    /// block, whose variable, unless it is a `let mut`, is one that only a
    /// `:=` of the header may rebind ([`BindingKind::Header`]), keeping its
    /// one slot and its type; or a `:=`, checked as in a block, so that it
    /// rebinds such a variable, or a `let mut`, in its slot.
    pub(super) fn header_item(&mut self, id: ItemId) {
        match self.ast.item(id).kind {
            ItemKind::Let {
                path,
                ty,
                value,
                mutable,
                ..
            } => {
                let before = self.bindings.len();
                self.check_let(id, path, ty, (value, mutable));
                if self.bindings.len() > before
                    && let Some(binding) = self.bindings.last_mut()
                    && binding.kind == BindingKind::Variable
                {
                    binding.kind = BindingKind::Header;
                }
            }
            ItemKind::Rebind { path, value } => self.check_rebind(id, path, value),
            // The parser allows no other item in a header.
            ItemKind::Field { .. }
            | ItemKind::Spread(_)
            | ItemKind::Result(_)
            | ItemKind::Expr(_)
            | ItemKind::Break(_)
            | ItemKind::Continue(_)
            | ItemKind::Return(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::Enum { .. }
            | ItemKind::Variant { .. }
            | ItemKind::Context { .. }
            | ItemKind::DefTypeDef { .. }
            | ItemKind::DefMeta { .. }
            | ItemKind::Error => {}
        }
    }

    /// Puts `frame`, a loop's or a labelled block's, on [`Checker::loops`],
    /// for its body. Loop and block labels are one namespace: a label that
    /// a loop or labelled block of the same function holding this one has
    /// already is "label `'L` is already in use", at the label, labelled at
    /// the other; a function starts with no labels.
    pub(super) fn enter_target(&mut self, frame: LoopFrame) {
        if let Some(label) = frame.label
            && let Some((outer, noun)) = self
                .loops
                .iter()
                .rev()
                .take_while(|outer| outer.depth == frame.depth)
                .find_map(|outer| {
                    let found = outer.label.filter(|outer| outer.name == label.name)?;
                    Some((found, outer.noun()))
                })
        {
            let text = self.ast.names().get(label.name);
            self.errors.push(
                Diagnostic::error(format!("label `'{text}` is already in use"), label.span)
                    .with_context(
                        outer.span,
                        format!("the {noun} around this one is labelled so"),
                    )
                    .with_note(format!("give the inner {} another label", frame.noun())),
            );
        }
        self.loops.push(frame);
    }

    /// Checks the block `id`, labelled `label`, whose items are `items`, as
    /// [`Checker::check_block`] does, with `expecting` as for it, and with
    /// the block on [`Checker::loops`], so that the `return`s that name it
    /// find it: its type is the join of its result and of the values they
    /// return (see [`Checker::leave_block`]).
    pub(super) fn labelled_block(
        &mut self,
        id: ExprId,
        label: LabelId,
        items: ItemRange,
        expecting: Option<Expecting<'_>>,
    ) -> TyId {
        self.enter_block(id, self.ast.label(label), expecting);
        let ty = self.check_block(items, expecting);
        self.leave_block(id, ty, expecting)
    }

    /// Puts the block `id`, labelled `label`, on [`Checker::loops`], for
    /// its items: the `return`s that name it are checked against what
    /// `expecting` says is expected of it, if anything is.
    pub(super) fn enter_block(
        &mut self,
        id: ExprId,
        label: Label,
        expecting: Option<Expecting<'_>>,
    ) {
        let expected =
            expecting.map(|expecting| (expecting.ty, expecting.context, expecting.why.to_owned()));
        let frame = LoopFrame::new(
            (None, id),
            Some(label),
            self.functions.len(),
            None,
            expected,
        );
        self.enter_target(frame);
    }

    /// Takes the labelled block `id`, whose result is of type `result`, off
    /// [`Checker::loops`], and returns its type: the join of its result and
    /// of the values the `return`s that name it return, as an `if`'s
    /// branches are joined, untyped integers adapting to each other and
    /// staying untyped if all are (see [`Checker::join_sites`]). With
    /// `expecting`, each fits what it expects already, and the join is no
    /// warning, but is the type expected should it not be a subtype of it;
    /// without, a join that widens is a warning, "the values this block
    /// returns have different types", at its label, labelled at each.
    pub(super) fn leave_block(
        &mut self,
        id: ExprId,
        result: TyId,
        expecting: Option<Expecting<'_>>,
    ) -> TyId {
        let Some(frame) = self.loops.pop() else {
            return result;
        };
        if frame.returns.is_empty() {
            return result;
        }
        let values: Vec<ExprId> = frame.returns.iter().filter_map(|site| site.value).collect();
        if !values.is_empty() {
            self.block_returns.insert(id, values);
        }
        // The result site is the block itself, which an untyped integer is
        // pushed into as into its result.
        self.expr_types[id.index()] = result;
        let at = self.ast.get(self.branch_value(id)).span;
        let mut sites = Vec::with_capacity(frame.returns.len() + 1);
        sites.push(Site {
            value: Some(id),
            at,
            ty: result,
        });
        sites.extend_from_slice(&frame.returns);
        let (joined, widened) = self.join_sites(id, &sites, false);
        match expecting {
            Some(expecting)
                if joined == TyId::ERROR || self.types.is_subtype(joined, expecting.ty) =>
            {
                joined
            }
            Some(expecting) => expecting.ty,
            None => {
                if widened && let Some(label) = frame.label {
                    self.sites_widened(
                        (
                            "the values this block returns have different types",
                            label.span,
                        ),
                        &sites,
                        ("block", joined),
                    );
                }
                joined
            }
        }
    }

    /// The join of the types of `sites`, the result sites of the block,
    /// loop or `lazy` `anchor`, in order, as the branches of an `if` are
    /// joined (see [`Checker::join_element`]): a site of type `Never` adds
    /// nothing, untyped integers adapt to each other and to the numeric
    /// types beside them, and staying untyped if all are, unless `settle`,
    /// when they settle to `Int`; and whether it widened. `Never` if every
    /// site is, and `Error` if any is erroneous. A site without a value, a
    /// `return` without one or a loop's list, is pushed nothing into; the
    /// parameters measured where the types differ are measured at a site's
    /// value, or at `anchor` for one without.
    pub(super) fn join_sites(
        &mut self,
        anchor: ExprId,
        sites: &[Site],
        settle: bool,
    ) -> (TyId, bool) {
        let live: Vec<(ExprId, TyId)> = sites
            .iter()
            .filter(|site| site.ty != self.never)
            .map(|site| (site.value.unwrap_or(anchor), site.ty))
            .collect();
        if live.iter().any(|&(_, ty)| ty == TyId::ERROR) {
            return (TyId::ERROR, false);
        }
        let mut joined = None;
        let mut widened = false;
        for (index, &(value, ty)) in live.iter().enumerate() {
            let Some(so_far) = joined else {
                joined = Some(ty);
                continue;
            };
            // A site without a value of its own is never untyped, so
            // nothing is pushed into `anchor` for it.
            let earlier: Vec<ExprId> = live.iter().take(index).map(|&(at, _)| at).collect();
            let (ty, widens, _) = self.join_element((earlier.into_iter(), value), so_far, ty);
            joined = Some(ty);
            widened |= widens;
        }
        let joined = match joined {
            Some(TyId::UNTYPED_INT) if settle => {
                for &(value, _) in &live {
                    let ty = self.expr_types[value.index()];
                    self.settle(value, ty);
                }
                TyId::INT
            }
            Some(joined) => joined,
            None => self.never,
        };
        (joined, widened)
    }

    /// The type of `site` as a warning about it shows it: that of its value
    /// as it now is, an untyped integer having adapted, or the site's own.
    pub(super) fn site_type(&self, site: &Site) -> TyId {
        site.value
            .map_or(site.ty, |value| self.expr_types[value.index()])
    }

    /// Warns, `message` at `at`, that `sites`, the result sites of a
    /// block, a loop or a `lazy`, have different types, which `joined`, the
    /// type of `what`, widened to a union of: labelled at each site with its
    /// type, with slice 20's note naming the union.
    pub(super) fn sites_widened(
        &mut self,
        (message, at): (&str, Span),
        sites: &[Site],
        (what, joined): (&str, TyId),
    ) {
        let note = self.union_note_of(what, joined);
        let mut warning = Diagnostic::warning(message, at);
        let never = self.never;
        for site in sites.iter().filter(|site| site.ty != never) {
            let label = format!("this is {}", self.in_label(self.site_type(site)));
            warning = warning.with_context(site.at, label);
        }
        self.warnings.push(warning.with_note(note));
    }

    /// The note on a warning that the values a `what` may have widened to
    /// the union `joined`: "the `what` is `Int | String`; annotate it with
    /// that type if all are meant, or tell them apart with `is`".
    pub(super) fn union_note_of(&mut self, what: &str, joined: TyId) -> String {
        format!(
            "the {what} is {}; annotate it with that type if all are meant, or tell them apart \
             with `is`",
            self.in_message(joined)
        )
    }

    /// The element type of the loop `id`, whose body, `body`, is of type
    /// `body_ty`, and for which the `break`s and `continue`s with a value
    /// gave `values`: each value with its type (see [`LoopFrame::values`]).
    /// These are joined as the elements of a list literal are (see
    /// [`Checker::check_list`]), in order, the body last, a `Never` among
    /// them adding nothing: with `element`, the element type expected, each
    /// fits it already, and the join is no warning, but is `element` should
    /// it not be a subtype of it; without, untyped integers adapt to each
    /// other and settle to `Int`, and a join that widens is a warning (see
    /// [`Checker::values_widened`]). If nothing contributes, or only
    /// `Never`, it is `element`, or `{}` without one. `Error` if anything
    /// contributing is.
    pub(super) fn element_type(
        &mut self,
        id: ExprId,
        (body, body_ty): (ExprId, TyId),
        values: Vec<(ExprId, TyId)>,
        element: Option<(TyId, (Span, &str))>,
    ) -> TyId {
        let mut sites = values;
        sites.push((body, body_ty));
        sites.retain(|&(_, ty)| ty != self.never);
        if let Some((element, _)) = element {
            let mut joined = None;
            for &(_, ty) in &sites {
                joined = Some(match joined {
                    None => ty,
                    Some(so_far) => self.types.join(so_far, ty).0,
                });
            }
            return match joined {
                Some(TyId::ERROR) => TyId::ERROR,
                Some(joined) if self.types.is_subtype(joined, element) => joined,
                _ => element,
            };
        }
        let mut joined = None;
        let mut widened = false;
        for index in 0..sites.len() {
            let Some(&(value, ty)) = sites.get(index) else {
                continue;
            };
            let Some(so_far) = joined else {
                joined = Some(ty);
                continue;
            };
            let earlier: Vec<ExprId> = sites.iter().take(index).map(|&(at, _)| at).collect();
            let (ty, widens, _) = self.join_element((earlier.into_iter(), value), so_far, ty);
            joined = Some(ty);
            widened |= widens;
        }
        // Whatever has not adapted to a concrete type is an `Int`.
        for &(value, _) in &sites {
            let ty = self.expr_types[value.index()];
            self.settle(value, ty);
        }
        let element = match joined {
            _ if sites.iter().any(|&(_, ty)| ty == TyId::ERROR) => return TyId::ERROR,
            Some(TyId::UNTYPED_INT) => TyId::INT,
            Some(joined) => joined,
            None => TyId::EMPTY,
        };
        if widened {
            self.values_widened(id, &sites, element);
        }
        element
    }

    /// Warns that the values the loop `id` collects, at `sites`, its
    /// `break`s' and `continue`s' values and its body last, each with its
    /// type, have different types, which `element`, its element type,
    /// widened to a union of: "the values this loop collects have different
    /// types", at the loop's keyword, labelled at each site with its type,
    /// the body at its value (see [`Checker::branch_value`]), and a note
    /// naming the loop's type.
    pub(super) fn values_widened(&mut self, id: ExprId, sites: &[(ExprId, TyId)], element: TyId) {
        let list = self.types.list(element);
        let note = format!(
            "the loop is {}; annotate it with that type if all are meant, or tell them apart \
             with `is`",
            self.in_message(list)
        );
        let mut warning = Diagnostic::warning(
            "the values this loop collects have different types",
            self.ast.loop_keyword(id),
        );
        for &(value, _) in sites {
            let ty = self.expr_types[value.index()];
            let label = format!("this is {}", self.in_label(ty));
            let at = self.branch_value(value);
            warning = warning.with_context(self.ast.get(at).span, label);
        }
        self.warnings.push(warning.with_note(note));
    }

    /// Checks the `break` or `continue` item `id`, `item`, whose label and
    /// value are `jump`, in a block whose first field, if it has one, is
    /// `first_field` (see the module documentation): as an item of a record
    /// block, it is "`break` cannot be an item of a record block";
    /// otherwise it is for the loop [`Checker::jump_target`] finds, which
    /// is recorded for lowering, and its value, if it has one, is one of
    /// what that loop collects (see [`Checker::element_type`]), checked
    /// against the loop's expected element type if it has one. Where there
    /// is no loop for it, its value is checked on its own.
    pub(super) fn check_jump(
        &mut self,
        (id, item): (ItemId, Item),
        jump: Jump,
        first_field: Option<ItemId>,
    ) {
        let keyword = match item.kind {
            ItemKind::Break(_) => "break",
            _ => "continue",
        };
        let at = jump_span(keyword, item.span, jump.label);
        let target = if first_field.is_some() {
            self.in_record_error(keyword, at);
            None
        } else {
            self.jump_target(keyword, at, jump.label)
        };
        let Some(frame) = target else {
            if let Some(value) = jump.value {
                self.check_alone(value);
            }
            return;
        };
        let Some((target, expected)) = self.loops.get_mut(frame).map(|frame| {
            frame.broken |= keyword == "break";
            (frame.target(), frame.expected.clone())
        }) else {
            return;
        };
        self.jumps[id.index()] = Some(target);
        let Some(value) = jump.value else {
            return;
        };
        let ty = match expected {
            Some((element, context, why)) => self.fit(value, element, (context, &why)),
            None => self.check_expr(value),
        };
        if let Some(frame) = self.loops.get_mut(frame) {
            frame.values.push((value, ty));
        }
    }

    /// Reports the `break`, `continue` or `return`, as `keyword` says,
    /// written at `at`, as an item of a record block, which nothing can
    /// leave.
    pub(super) fn in_record_error(&mut self, keyword: &str, at: Span) {
        let leave = if keyword == "return" {
            "return"
        } else {
            "leave the loop"
        };
        self.errors.push(
            Diagnostic::error(
                format!("`{keyword}` cannot be an item of a record block"),
                at,
            )
            .with_note(format!(
                "a record block's value is the record of its fields; {leave} from a block that \
                 is not one, such as a field's value"
            )),
        );
    }

    /// The index in [`Checker::loops`] of the loop that the `break` or
    /// `continue`, as `keyword` says, written at `at`, with `label`, if it
    /// has one, is for: the innermost loop of the function being checked
    /// whose body holds it, labelled `label` if it has one; or for a
    /// `return`, which always has a label, the innermost loop or labelled
    /// block of that label holding it. Where there is none, reports why and
    /// returns `None`: "`break` outside a loop", if no loop holds it; "no
    /// enclosing loop is labelled `'L`" (for a `return`, "no enclosing
    /// block or loop is labelled `'L`"), at the label, if nothing it could
    /// be for is labelled so; "`'L` labels a block, not a loop" if a
    /// `break` or `continue` names a block, labelled at the block's label;
    /// and if the only such loops or blocks are outside the function or
    /// `lazy` being checked, "`break` cannot leave a function" (or "a
    /// `lazy`"), labelled at the function's keyword, and at the loop's or
    /// block's label for a label, with a note saying when the body runs.
    pub(super) fn jump_target(
        &mut self,
        keyword: &str,
        at: Span,
        label: Option<Label>,
    ) -> Option<usize> {
        let depth = self.functions.len();
        let returning = keyword == "return";
        let names = |frame: &LoopFrame| match label {
            Some(label) => frame.label.is_some_and(|own| own.name == label.name),
            // An unlabelled `break` or `continue` is for a loop.
            None => frame.id.is_some(),
        };
        let found = self.loops.iter().rposition(names);
        let found = match found.and_then(|index| Some((index, self.loops.get(index)?))) {
            Some((index, frame)) if frame.depth == depth => {
                if returning || frame.id.is_some() {
                    return Some(index);
                }
                let error = self.not_a_loop(at, frame.label);
                self.errors.push(error);
                return None;
            }
            found => found.map(|(_, frame)| frame),
        };
        let error = match (found, label) {
            (Some(outer), _) => self.leave_function_error(keyword, at, (label, outer)),
            (None, None) => Diagnostic::error(format!("`{keyword}` outside a loop"), at),
            (None, Some(label)) => {
                let text = self.ast.names().get(label.name);
                let what = if returning { "block or loop" } else { "loop" };
                Diagnostic::error(
                    format!("no enclosing {what} is labelled `'{text}`"),
                    label.span,
                )
            }
        };
        self.errors.push(error);
        None
    }

    /// The error for a `break` or `continue` written at `at` whose label
    /// names a block, labelled `label`: "`'L` labels a block, not a loop",
    /// labelled at the block's label, with a note saying to `return`.
    pub(super) fn not_a_loop(&self, at: Span, label: Option<Label>) -> Diagnostic {
        let Some(label) = label else {
            return InternalError {
                message: "a block without a label is a target",
                span: at,
            }
            .into();
        };
        let text = self.ast.names().get(label.name);
        Diagnostic::error(format!("`'{text}` labels a block, not a loop"), at)
            .with_context(label.span, "the block is labelled here")
            .with_note(format!("use `return'{text}` to leave a block"))
    }

    /// The error for the `break`, `continue` or `return`, as `keyword`
    /// says, written at `at`, with `label`, if it has one, whose only
    /// target, `outer`, is outside the function or `lazy` being checked:
    /// "`break` cannot leave a function" (or "a `lazy`"), labelled at the
    /// function's keyword, and at the target's label for a label, with a
    /// note saying when the body runs.
    pub(super) fn leave_function_error(
        &self,
        keyword: &str,
        at: Span,
        (label, outer): (Option<Label>, &LoopFrame),
    ) -> Diagnostic {
        let noun = outer.noun();
        let function = self.functions.last();
        let (kind, when) = match function.map(|function| self.ast.fn_decl(function.fn_id)) {
            Some(decl) if decl.ret.is_none() => ("a `lazy`", "awaited"),
            _ => ("a function", "called"),
        };
        let mut error = Diagnostic::error(format!("`{keyword}` cannot leave {kind}"), at)
            .with_note(format!(
                "the body of {kind} runs when it is {when}, which may be after the {noun} has \
                 ended"
            ));
        if let Some(function) = function {
            error = error.with_context(function.keyword, format!("inside {kind} here"));
        }
        match (label, outer.label) {
            (Some(_), Some(outer)) => {
                error.with_context(outer.span, format!("the {noun} it names is outside it"))
            }
            _ => error,
        }
    }

    /// Checks the `return` item `id`, `item`, whose label and value are
    /// `jump`, in a block whose first field, if it has one, is
    /// `first_field` (see [`return`](#return)): as an item of a record
    /// block, it is "`return` cannot be an item of a record block";
    /// without a label it is for the innermost function or `lazy`, and at
    /// the top level, outside any, "`return` outside a function"; with
    /// one, for the loop or labelled block [`Checker::jump_target`] finds.
    /// What it is for is recorded for lowering, and its value, `{}` without
    /// one, is one of that target's result sites (see
    /// [`Checker::return_site`]). Where there is no target, its value is
    /// checked on its own.
    pub(super) fn check_return(
        &mut self,
        (id, item): (ItemId, Item),
        jump: Jump,
        first_field: Option<ItemId>,
    ) {
        let at = jump_span("return", item.span, jump.label);
        let target = if first_field.is_some() {
            self.in_record_error("return", at);
            None
        } else {
            match jump.label {
                None if self.functions.is_empty() => {
                    self.errors.push(
                        Diagnostic::error("`return` outside a function", at).with_note(
                            "only a function or a `lazy` can be returned from without a label; a \
                         block or a loop labelled `'L` is left with `return'L`",
                        ),
                    );
                    None
                }
                None => Some(None),
                Some(label) => self.jump_target("return", at, Some(label)).map(Some),
            }
        };
        let Some(frame) = target else {
            if let Some(value) = jump.value {
                self.check_alone(value);
            }
            return;
        };
        let target = match frame.and_then(|frame| self.loops.get(frame)) {
            Some(frame) => frame.target(),
            None => JumpTarget::Function,
        };
        self.jumps[id.index()] = Some(target);
        match target {
            JumpTarget::Loop(loop_id) => {
                if let Some(resolution) = self.loop_resolutions.get_mut(loop_id.index()) {
                    resolution.returned = true;
                }
            }
            JumpTarget::Block(block) => {
                self.returned_blocks.insert(block);
            }
            JumpTarget::Function => {}
        }
        let site = self.return_site(at, jump.value, frame);
        let returns = match frame {
            Some(frame) => self.loops.get_mut(frame).map(|frame| &mut frame.returns),
            None => self
                .functions
                .last_mut()
                .map(|function| &mut function.returns),
        };
        if let Some(returns) = returns {
            returns.push(site);
        }
    }

    /// Checks the value of the `return` written at `at`, if it has one, for
    /// the loop or labelled block `loops[frame]`, or with no frame for the
    /// innermost function or `lazy`, and returns it as one of the target's
    /// result sites. For a function, it is checked against the declared
    /// result type as the body's result is (see [`return`](#return)), and
    /// for a loop or block, against the type expected of the whole, if one
    /// is; otherwise on its own. A `return` without a value gives `{}`,
    /// which must fit the same.
    pub(super) fn return_site(
        &mut self,
        at: Span,
        value: Option<ExprId>,
        frame: Option<usize>,
    ) -> Site {
        let expected = match frame {
            Some(frame) => self
                .loops
                .get(frame)
                .and_then(|frame| frame.returns_expected.clone()),
            None => self
                .functions
                .last()
                .and_then(|function| function.result)
                .map(|(ret, context)| (ret, context, RESULT_WHY.to_owned())),
        };
        let Some(value) = value else {
            if let Some((expected, context, why)) = expected
                && expected != TyId::ERROR
                && !self.types.is_subtype(TyId::EMPTY, expected)
            {
                let label = format!("expected {}, found {{}}", self.in_label(expected));
                self.errors.push(
                    Diagnostic::error("mismatched types", at)
                        .with_label(label)
                        .with_context(context, why)
                        .with_note("a `return` without a value returns `{}`"),
                );
            }
            return Site {
                value: None,
                at,
                ty: TyId::EMPTY,
            };
        };
        let ty = match (expected, frame) {
            // As the body's result is, whose type is its own.
            (Some((ret, context, why)), None) => {
                self.check_expr_expecting(value, ret, context, &why)
            }
            (Some((expected, context, why)), Some(_)) => self.fit(value, expected, (context, &why)),
            (None, _) => self.check_expr(value),
        };
        Site {
            value: Some(value),
            at: self.ast.get(value).span,
            ty,
        }
    }

    /// The element type of `ty`, the type of the iterable `iterable` of a
    /// `for` once measured, a list or a union of lists (see
    /// [`Checker::element_of`]), or `Error`, having reported "cannot iterate
    /// over a value of type T: it is not a list" if it is neither, or if it
    /// is erroneous.
    pub(super) fn iterated(&mut self, iterable: ExprId, ty: TyId) -> TyId {
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        if let Some(element) = self.element_of(ty) {
            return element;
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        let mut error = Diagnostic::error(
            format!("cannot iterate over a value of type {shown}: it is not a list"),
            self.ast.get(iterable).span,
        )
        .with_label(label);
        if let Some(note) = self.union_note(iterable, ty) {
            error = error.with_note(note);
        }
        self.errors.push(error);
        TyId::ERROR
    }
}
