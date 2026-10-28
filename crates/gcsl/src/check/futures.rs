//! Checking futures: measuring an expression's laziness, awaiting it implicitly
//! or explicitly, and the purity rule of awaits.
//!
//! # Futures
//!
//! `lazy e` is checked as the body of an anonymous, parameterless function (a
//! `Function` of its own on the stack, with a frame of its own, capturing as a
//! closure does), which is always **impure**, wherever it is written; its type
//! is `Future<T>`, `T` being the body's type, an untyped integer settled to
//! `Int`, with every future layer at its top stripped, since awaiting resolves
//! fully (so `lazy lazy 1` is `Future<Int>`, and so is `lazy f()` where `f()`
//! is `Int | Future<Int>`; see
//! [`TypeTable::future`](crate::types::TypeTable::future)). The name a `let` or
//! a field binds is not in scope in its own `lazy` initialiser, as for any
//! initialiser.
//!
//! **Bare types.** In a type expression, a type written as a name, a
//! record type, a list type (`List<T>` or `T[]`) or a union with no member
//! written `Future<...>` means what the position it is written in says
//! (`Bareness`):
//! the either shape, `T | Future<T>`, in the parameters and the return type
//! of an impure function expression and of a written impure function type,
//! and in the return type of a memoised one, at that position and at every
//! bare position inside it, through record and list types (so `{a: Int}`
//! there is `{a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>`, and
//! `List<Int>` is `List<Int | Future<Int>> | Future<List<Int | Future<Int>>>`)
//! but not into a
//! nested function type, which has its own rule; and a present `T`
//! everywhere else: annotations, ascriptions, the return types of pure
//! functions, the parameters and return types of written pure function
//! types, and the parameters of memoised function expressions and written
//! memoised function types (see `bareness_of`). A memoised function is
//! strict: its arguments are the key its results are memoised by, so each
//! is present, at its position and at every position inside it, and none is
//! inferred; `Future<T>`, `T | Future<T>` and `Present<T>` may still be
//! written there and mean what they say. A caller passing a value that may
//! be a future to such a parameter awaits it at the call, deeply where the
//! parameter's type needs it, as for any present parameter. (A pure
//! function expression's bare parameters are inferred: see [Inferred
//! parameters](crate::check::inference#inferred-parameters); and so are an impure or memoised
//! function expression's bare result positions, from what its body
//! returns, once its body is checked against the either shapes written:
//! see [Inferred
//! results](crate::check::inference#inferred-results).) A function type is the one bare type
//! that is never the
//! either shape at its own position: its interior follows its own purity
//! marker, and a function value that may be a future must be written so,
//! as `Future<fn(): Int>` or `(fn(): Int) | Future<fn(): Int>` (in
//! parentheses, since a function type's return type would take the union).
//! A bare union is what the rule makes of each member, as it is written
//! (each record member's fields and list member's elements as the rule
//! says, a function type member following its own marker), the union of
//! them at its own position, and the either shape of that union where a
//! bare name would be: `Int | String` in an impure function's parameter is
//! `Int | String | Future<Int | String>`, so widening a parameter from
//! `Int` to `Int | String` does not stop it taking a future
//! (`Checker::resolve_union`). An applied type other than `List` and a
//! union with a member written `Future<...>` are explicit, and so never
//! wrapped. `Future<T>` and `Present<T>` override that at their position,
//! their own interiors being present, and a union with a `Future` member
//! may be written if it is the either shape, `T | Future<T>`: exactly one
//! member a `Future`, of the union of the others, as in `Int | String |
//! Future<Int | String>`; any other union with a `Future` member ("a union
//! with a `Future` member must be `T | Future<T>`", at its last `|`), any
//! other name applied to a type, and `Future` or `Present` without a type
//! argument are errors. So an impure function takes, and says it returns,
//! values that may be futures, since its body awaits them on demand, while
//! a pure function takes present values where its body needs them, since
//! its body cannot
//! await, and a memoised function takes present values, the key it looks
//! its results up by; and a pure `fn(Int): Int` is not a subtype of the
//! impure `fn(Int) -> Int`, which promises to pass futures (parameters are
//! contravariant), though it is of `fn(Present<Int>) -> Int`, and neither
//! is a memoised `fn(Int) ->> Int`, though it too is a subtype of
//! `fn(Present<Int>) -> Int` (see [`crate::types`] for the effect
//! lattice). An impure or memoised function's parameter written `fn(Int)
//! -> Int` is narrowed to `fn(Present<Int>) -> Int` if its body only ever
//! passes it present values: see [Narrowed
//! callbacks](crate::check::inference#narrowed-callbacks).
//!
//! **Awaits.** Every place a value is used as its bits decides whether it is
//! awaited there, and records it in
//! [`Resolution::await_of`](super::Resolution::await_of) (or, for a record an
//! update reads, [`Resolution::path_awaits`](super::Resolution::path_awaits))
//! for lowering, which inserts the await; nothing is decided at runtime. A
//! measuring operator (arithmetic, ordering, `and`, `or`, `not`, negation), a
//! call of a value, a field access into one and a spread of one await it at the
//! top if its type is a `Future` or an either (`Checker::measure`), and type
//! the operation on the type it has once awaited, stripped of every future
//! layer at its top (see [`TypeTable::strip`](crate::types::TypeTable::strip));
//! so does a dotted `let` or `:=` each record, or list, along its path that
//! exists (see "Dotted `let` and `:=`" above). An interpolated expression and
//! an operand of an equality are awaited deeply (`Checker::measure_deep`): at
//! the top and in every lazy field of the record it is, or element of the list
//! it is, at any depth (see
//! [`TypeTable::present`](crate::types::TypeTable::present)). Where a type is
//! expected, as for an argument, an annotated initialiser, a function's body or
//! an `if` condition, a value that is not a subtype of it is awaited if it is
//! one once awaited, at the top or deeply (see
//! [`TypeTable::needs_await`](crate::types::TypeTable::needs_await)); otherwise
//! the mismatch is reported, with a note when a future is expected and a value
//! that is not one is found. `await e` awaits `e` fully and has its type
//! stripped; on a present value it does nothing, silently, since whether a
//! value is present may be inferred from code elsewhere. A bare `await e` item
//! is never an unused value. A statement, an expression item that is not its
//! block's result, is a position expecting a *unit-like* value: `{}`, a list
//! whose element type is unit-like, or a union all of whose members are, as the
//! `List<{}> | {}` of a statement loop a bare `return` may leave is (see
//! [`TypeTable::is_unit_like`](crate::types::TypeTable::is_unit_like)), which
//! holds nothing a program could use. A statement that is unit-like as it is,
//! as `{}` is, or the `List<{}>` of a `for` whose body is `{}`, is fine as it
//! is; one whose value is unit-like once awaited, at the top (a `Future<{}>`,
//! or the `{}` an impure function returns, which may be one) or deeply (a
//! `List<Future<{}>>`, as `for s in steps { lazy run(s) }` is), is awaited
//! there, at the top or deeply as its type needs (`Checker::measure`,
//! `Checker::measure_deep`), subject to the purity rule, so that its effects
//! are not lost. A call of a function or built-in that is not pure, outside a
//! pure function, is run for its effects as a bare `await` is: awaited deeply,
//! and its value, whatever it is, discarded
//! ([`Resolution::is_discarded`](super::Resolution::is_discarded)). Any other
//! statement is an unused value. A statement that is directly the empty list
//! literal, `[]`, is a `List<{}>`, whose element type needs no inferring.
//!
//! Futures have no equality: an operand of `==`, `!=`, `===` or `!==` whose
//! type, once awaited at the top, has a `Future` in a field or as a list's
//! element type at any depth is an error (a field or an element of the
//! either shape is not: the deep await resolves it). Two `if` branches of which either may be a
//! future have their join
//! (see [`TypeTable::join`](crate::types::TypeTable::join)).
//!
//! **Purity.** Awaiting runs a deferred expression, which may have effects,
//! so it is impure: an await in the body of a pure function, explicit or
//! implicit, is an error, reported where the await is decided, entirely
//! statically. A pure function may still build, hold, store, pass on and
//! return futures, and build them with `lazy`, whose body is impure. A
//! pure function measuring its own bare parameter is never an error: it
//! makes the parameter present (see below), so the await happens in the
//! caller, where an argument that may be a future is awaited at the call.
//!
//! The implicit-await error's note suggests declaring the function with
//! `->`. Where the value is a variable, through any parentheses and as the
//! result of any block (as in the body `{ n }`), or the variable a dotted
//! `let` or `:=` updates, the record at its path's first segment, that the
//! pure function captures, and that code outside it could await before
//! creating it, the note instead says to do that, as `let n = await n`
//! (`Checker::awaitable_capture`, `Checker::awaitable_outside`): a variable
//! of the top level, which is impure, or one whose home function (the
//! function whose body declares it) or a function between that one and the
//! pure function is impure, as a `lazy` and a memoised function are. A
//! capture whose home and every function between are pure could be awaited
//! there no more than in the pure function, and keeps the usual note, as
//! does a record further along a dotted `let`'s or `:=`'s path, which is
//! not a variable.

use crate::ast::{ArgKind, Effect, ExprId, ExprKind, FnId, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::{AwaitKind, Laziness, TyId};

use super::{
    AwaitMark, AwaitSite, CallSite, Checker, Function, PURE_CONTEXT, Site, VarRef,
    explicit_await_error,
};

impl Checker<'_> {
    /// Where `expr`, of type `ty`, is used as its bits at the top, by a
    /// measuring operator, a call of it or a field access into it: if it
    /// is a parameter of the innermost function whose type is provisional,
    /// the parameter is measured, which makes it present, and its present
    /// type returned; if it may be a future, it is marked to be awaited at
    /// the top, subject to the purity rule, and the type it has once
    /// awaited returned; otherwise, or if it is erroneous, `ty` is.
    pub(super) fn measure(&mut self, expr: ExprId, ty: TyId) -> TyId {
        if ty == TyId::ERROR || self.types.laziness(ty) == Laziness::Present {
            return ty;
        }
        let claimed = self.claim_top(self.ast.get(expr).span, ty);
        if claimed != ty {
            return claimed;
        }
        // A future, an either, or a provisional type of an enclosing
        // function, which may be a future for all this body knows.
        self.await_here(expr, ty, AwaitKind::Top, AwaitSite::Implicit);
        self.types.strip(ty)
    }

    /// Where `expr`, of type `ty`, is used as its bits all the way down, as
    /// an interpolated expression or an operand of an equality, or is
    /// awaited as a statement unit-like once awaited deeply: every
    /// parameter of the innermost function whose provisional type is in it,
    /// at the top or in a field, is measured; then if it, or a field of the
    /// record it is at any depth, may be a future, it is marked to be
    /// awaited (deeply, if the fields need it), subject to the purity rule,
    /// and the type it has once awaited returned; otherwise, or if it is
    /// erroneous, its type with the measured parameters present is.
    pub(super) fn measure_deep(&mut self, expr: ExprId, ty: TyId) -> TyId {
        if ty == TyId::ERROR {
            return ty;
        }
        let ty = self.claim(expr, ty);
        let present = self.types.present(ty);
        if present == ty {
            return ty;
        }
        let kind = if present == self.types.strip(ty) {
            AwaitKind::Top
        } else {
            AwaitKind::Deep
        };
        self.await_here(expr, ty, kind, AwaitSite::Implicit);
        present
    }

    /// Marks `expr`, of type `found`, which fits `expected` once awaited
    /// deeply, to be awaited toward it, `site` as [`Checker::await_here`]
    /// takes it: the type it has once awaited, which leaves lazy each field
    /// `expected` admits lazy (see
    /// [`TypeTable::present_toward`](crate::types::TypeTable::present_toward)).
    pub(super) fn await_toward(
        &mut self,
        expr: ExprId,
        (found, expected): (TyId, TyId),
        site: AwaitSite,
    ) -> TyId {
        self.await_here(expr, found, AwaitKind::Deep, site);
        let awaited = self.types.present_toward(found, expected);
        self.awaits[expr.index()] = AwaitMark::Toward(awaited);
        awaited
    }

    /// Marks `expr`, of type `ty`, to be awaited as `kind` says, and applies
    /// the purity rule: awaiting is impure, so inside a pure function it is
    /// an error, "cannot await in a pure function" at the `await` keyword
    /// for an explicit await ([`AwaitSite::Explicit`]), and "cannot use a
    /// value that may be a future in a pure function" at `expr` for an
    /// implicit one (see [`Checker::implicit_await_error`]), or its
    /// call-site form for an argument that fills a parameter the callee's
    /// body measures ([`AwaitSite::Argument`]; see
    /// [`Checker::argument_await_error`]). The mark is recorded regardless,
    /// so that the error does not cascade. The body of a `lazy` is impure,
    /// wherever it is written.
    pub(super) fn await_here(&mut self, expr: ExprId, ty: TyId, kind: AwaitKind, site: AwaitSite) {
        self.awaits[expr.index()] = kind.into();
        let Some(marker) = self.pure_marker() else {
            return;
        };
        let error = match site {
            AwaitSite::Explicit(keyword) => explicit_await_error(keyword, marker),
            AwaitSite::Argument(call_site) => {
                self.argument_await_error(expr, ty, marker, call_site)
            }
            AwaitSite::Implicit => {
                let capture = self.awaitable_capture(expr);
                self.implicit_await_error(self.ast.get(expr).span, ty, marker, capture)
            }
        };
        self.errors.push(error);
    }

    /// The implicit-await error, "cannot use a value that may be a future
    /// in a pure function", at `span`, where a value of type `ty` is used
    /// as its bits in a pure function whose marker is at `marker`: labelled
    /// with the type and the marker. Its note says how to await `capture`
    /// before the function is created, if it is a variable the function
    /// captures that can be awaited there (see
    /// [`Checker::awaitable_capture`]), and otherwise that the function
    /// could be made impure. For a use of a value by an expression and for
    /// a record read by a dotted `let` or `:=` alike.
    pub(super) fn implicit_await_error(
        &mut self,
        span: Span,
        ty: TyId,
        marker: Span,
        capture: Option<Name>,
    ) -> Diagnostic {
        let note = match capture {
            Some(name) => {
                let text = self.ast.names().get(name);
                format!(
                    "a pure function cannot await, and `{text}` may be a future; await it before \
                     the function is created, in the enclosing impure code: `let {text} = await \
                     {text}`"
                )
            }
            None => "using the value here would await it, which may run effects; a pure \
                     function may hold and pass on a future but not await it. Declare the \
                     function with `->` to make it impure"
                .to_owned(),
        };
        Diagnostic::error(
            "cannot use a value that may be a future in a pure function",
            span,
        )
        .with_label(format!("this is {}", self.in_label(ty)))
        .with_context(marker, PURE_CONTEXT)
        .with_note(note)
    }

    /// The name of the variable that `expr` is, through any parentheses and
    /// as the result of any block, if the innermost function captures it and
    /// it can be awaited before that function is created (see
    /// [`Checker::awaitable_outside`]). `None` for any other expression,
    /// including a capture whose home and every function between are pure,
    /// which can await it no more than the innermost function can.
    pub(super) fn awaitable_capture(&self, expr: ExprId) -> Option<Name> {
        let mut node = expr;
        let name = loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                // A block is awaited for its result, as a function's body
                // `{ n }` is against the result type.
                ExprKind::Block { items, .. } => {
                    let last = self.ast.items(items).last()?;
                    let (ItemKind::Result(result) | ItemKind::Expr(result)) =
                        self.ast.item(*last).kind
                    else {
                        return None;
                    };
                    node = result;
                }
                ExprKind::Name(name) => break name,
                _ => return None,
            }
        };
        let VarRef::Capture(capture) = self.expr_vars[node.index()] else {
            return None;
        };
        let binding = self
            .functions
            .last()?
            .captures
            .get(capture as usize)?
            .binding;
        self.awaitable_outside(binding).then_some(name)
    }

    /// Whether the binding at `binding` in `bindings` is one the innermost
    /// function captures, lying below its first binding, that can be awaited
    /// before that function is created: one of the top level, which is
    /// impure, or whose home function (the function whose body declares it)
    /// or a function between that one and the innermost is impure, as a
    /// `lazy` and a memoised function are. Not a binding of the innermost
    /// function itself, nor a capture whose home and every function between
    /// are pure. The condition of [`Checker::awaitable_capture`], and of the
    /// same hint for the variable a dotted `let` or `:=` updates.
    pub(super) fn awaitable_outside(&self, binding: usize) -> bool {
        let Some((_, enclosing)) = self.functions.split_last() else {
            return false;
        };
        // The outermost function the binding is not in; its home is the
        // function before it, or the top level if there is none.
        let Some(outside) = self
            .functions
            .iter()
            .position(|function| function.bindings_start > binding)
        else {
            return false;
        };
        match outside.checked_sub(1) {
            None => true,
            Some(home) => enclosing.get(home..).is_some_and(|between| {
                // A memoised function may await, as an impure one may.
                between
                    .iter()
                    .any(|function| function.effect != Effect::Pure)
            }),
        }
    }

    /// The marker of the innermost function, if it is pure (a memoised
    /// function is not): where the purity rule's errors say the function is
    /// pure.
    pub(super) fn pure_marker(&self) -> Option<Span> {
        self.functions
            .last()
            .filter(|function| function.effect == Effect::Pure)
            .map(|function| function.marker)
    }

    /// The call-site form of the implicit-await error, in a pure function
    /// whose marker is at `marker`, for the argument `arg`, of type `ty`,
    /// which fills a parameter that is present because the callee's body
    /// measures it, as `site` describes: "argument `x` would be awaited
    /// inside `f`, which is pure", the argument and the callee named by
    /// their source text if each is a plain name or a dotted path ("this
    /// argument" and "the function" otherwise); labelled with the
    /// argument's type, with where the callee's body measures the
    /// parameter if its function is known, and with the pure function's
    /// marker; and a note saying how to await the argument first, with the
    /// call written so if every argument is a name or a dotted path.
    pub(super) fn argument_await_error(
        &mut self,
        arg: ExprId,
        ty: TyId,
        marker: Span,
        site: CallSite,
    ) -> Diagnostic {
        let inside = self
            .path_text(site.callee)
            .map_or_else(|| "the function".to_owned(), |text| format!("`{text}`"));
        let message = match self.path_text(arg) {
            Some(text) => {
                format!("argument `{text}` would be awaited inside {inside}, which is pure")
            }
            None => format!("this argument would be awaited inside {inside}, which is pure"),
        };
        let label = format!("this is {}", self.in_label(ty));
        let mut error = Diagnostic::error(message, self.ast.get(arg).span).with_label(label);
        if let Some((span, param)) = site.measured {
            let text = self.ast.names().get(param);
            error = error.with_context(span, format!("`{text}` is measured here"));
        }
        let mut note = "a pure function cannot await; await the argument before the call, in an \
                        impure function"
            .to_owned();
        if let Some(call) = self.awaited_call_text(site.call, arg) {
            note = format!("{note}: `{call}`");
        }
        error.with_context(marker, PURE_CONTEXT).with_note(note)
    }

    /// The source text of `expr` if it is a plain name or a dotted path, a
    /// chain of field accesses on a name, as in `server.port`; `None`
    /// otherwise.
    pub(super) fn path_text(&self, expr: ExprId) -> Option<String> {
        let names = self.ast.names();
        let mut fields = Vec::new();
        let mut node = expr;
        let root = loop {
            match self.ast.get(node).kind {
                ExprKind::Name(name) => break name,
                ExprKind::Field { expr, name } => {
                    fields.push(name);
                    node = expr;
                }
                _ => return None,
            }
        };
        let mut text = names.get(root).to_owned();
        for &field in fields.iter().rev() {
            text.push('.');
            text.push_str(&FieldName(names.get(field)).to_string());
        }
        Some(text)
    }

    /// The call `call` written with `await` before its argument `awaited`,
    /// as in `f(await x)`, if its callee and every argument, named ones by
    /// their values, is a plain name or a dotted path, so that it can be
    /// written from them; `None` otherwise.
    pub(super) fn awaited_call_text(&self, call: ExprId, awaited: ExprId) -> Option<String> {
        let ExprKind::Call { callee, args } = self.ast.get(call).kind else {
            return None;
        };
        let mut text = self.path_text(callee)?;
        text.push('(');
        for (index, arg) in self.ast.args(args).iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            if let ArgKind::Named { name, .. } = arg.kind {
                text.push_str(self.ast.names().get(name));
                text.push_str(" = ");
            }
            if arg.value == awaited {
                text.push_str("await ");
            }
            text.push_str(&self.path_text(arg.value)?);
        }
        text.push(')');
        Some(text)
    }

    /// Types a `lazy` expression, whose body is function `function`: the body
    /// is checked as that of an anonymous, parameterless, impure function, in a
    /// frame of its own, capturing what it uses of the enclosing scopes as a
    /// closure does, and the expression's type is `Future<T>`, `T` being the
    /// body's type with an untyped integer settled to `Int`, stripped (see
    /// [`TypeTable::future`](crate::types::TypeTable::future): `lazy lazy 1` is
    /// `Future<Int>`); or `Error` if the body is erroneous. The `lazy` keyword
    /// is at `keyword`.
    pub(super) fn check_lazy(&mut self, function: FnId, keyword: Span) -> TyId {
        // A function in the body must not see a name meant for another.
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: Effect::Impure,
            marker: decl.marker,
            keyword,
            demanded: Vec::new(),
            result: None,
            returns: Vec::new(),
            with_depth: self.contexts.bound.len(),
            needs: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let found = self.check_expr(decl.body);
        let returns = self
            .functions
            .last_mut()
            .map(|function| std::mem::take(&mut function.returns))
            .unwrap_or_default();
        let body_ty = if returns.is_empty() {
            self.settle(decl.body, found)
        } else {
            self.returned_lazy(keyword, (decl.body, found), &returns)
        };
        self.finish_function(0, None);
        if body_ty == TyId::ERROR {
            TyId::ERROR
        } else {
            self.types.future(body_ty)
        }
    }

    /// What the `lazy` whose keyword is at `keyword`, whose operand is
    /// `body`, of type `found`, and which the `return`s `returns` leave,
    /// yields: the join of its operand's type and the values returned, as
    /// an `if`'s branches are joined, untyped integers adapting to each
    /// other and then settling to `Int`. A join that widens is a warning,
    /// "the values this `lazy` returns have different types", at the
    /// keyword, labelled at each. (Nothing reaches a `lazy` from a type
    /// expected of it: the `Future` it is must fit that as a whole.)
    pub(super) fn returned_lazy(
        &mut self,
        keyword: Span,
        (body, found): (ExprId, TyId),
        returns: &[Site],
    ) -> TyId {
        let mut sites = Vec::with_capacity(returns.len() + 1);
        sites.push(Site {
            value: Some(body),
            at: self.ast.get(self.branch_value(body)).span,
            ty: found,
        });
        sites.extend_from_slice(returns);
        let (joined, widened) = self.join_sites(body, &sites, true);
        if widened {
            let future = self.types.future(joined);
            self.sites_widened(
                (
                    "the values this `lazy` returns have different types",
                    keyword,
                ),
                &sites,
                ("`lazy`", future),
            );
        }
        joined
    }

    /// Types `await OPERAND`, whose keyword is at `keyword`, `operand` being
    /// of type `ty`, which is not erroneous (see [`Checker::unary`]).
    pub(super) fn explicit_await(&mut self, keyword: Span, operand: ExprId, ty: TyId) -> TyId {
        // Whether a value is present may be inferred, as a call's result is
        // from its callee's body, so awaiting one is not worth a warning.
        if self.types.laziness(ty) == Laziness::Present {
            return ty;
        }
        let claimed = self.claim_top(self.ast.get(operand).span, ty);
        if claimed != ty {
            // A parameter of this function, which is pure (only a pure
            // function's parameters are provisional), so the `await` is an
            // error before anything else; the parameter is measured.
            if let Some(marker) = self.pure_marker() {
                self.errors.push(explicit_await_error(keyword, marker));
            }
            return claimed;
        }
        self.await_here(operand, ty, AwaitKind::Top, AwaitSite::Explicit(keyword));
        self.types.strip(ty)
    }
}
