//! Inferring parameters and results: the types a function's unannotated
//! parameters and result are inferred to, and the callbacks narrowed by them.
//!
//! # Inferred parameters
//!
//! A pure function's bare parameter, one whose type is written as a name, a
//! record type, a list type or a union with no `Future` member (not
//! `Present<...>`, `Future<...>`, a union with a `Future` member or a
//! function type, which say what they are), is present if the function's
//! body
//! *measures* it, and the either shape if it never does, so that a pure
//! function can pass a future through without declaring it: `fn double(x:
//! Int): Int { x * 2 }` is `fn(x: Int): Int`, and `fn keep(x: Int): {v: Int
//! | Future<Int>} { v = x }` is `fn(x: Int | Future<Int>): {v: Int |
//! Future<Int>}`. The result is not inferred: a pure function's bare result
//! is present, so `fn id(x: Int): Int { x }` measures `x` through it. (An
//! impure function's is inferred, and its parameters are not: see [Inferred
//! results](#inferred-results).)
//!
//! While the body is checked, such a parameter has a provisional type,
//! `provisional(function, index, T)` (see
//! [`TypeTable::provisional`](crate::types::TypeTable::provisional)), in the
//! function's type as first interned, in the callee binding and in the
//! parameter's binding, and the function's entry on the stack
//! (`Function::demanded`) records, for each parameter, the first place in
//! source order where it is measured. A value keeps the provisional type
//! through *blind assignment*, with no code of its own: a `let` without an
//! annotation binds it, a record field holds it and gives it back when read,
//! and an `if` both of whose branches have it has it. The parameter is measured
//! where a value of its provisional type is used as its bits, which is exactly
//! where a value that may be a future would be awaited:
//!
//! - `Checker::measure` (the operands of the measuring operators, the
//!   callee of a call, the record of a field access, the list of an index,
//!   a spread or a `for`, a statement that is `{}` once awaited) measures
//!   a provisional type at the top, and gives its present type, with no
//!   await and no purity error;
//! - `Checker::measure_deep` (an interpolated expression, an operand of an
//!   equality, a statement that is unit-like once awaited deeply) measures
//!   each provisional type in the value's type, at the top or in a field,
//!   before deciding the await for the rest;
//! - `Checker::expect`, where the value is not a subtype of what is
//!   expected as it is, measures each parameter whose provisional type sits
//!   where the expected type is present (a present parameter, annotation,
//!   ascription, `:=` target, field type or return type, or the `Bool` of a
//!   condition or a logic operand) and tries again with them present; a
//!   provisional type is a subtype of the either shape as it is, so passing
//!   it to an either parameter, storing it in an either field and returning
//!   it against an either return type measure nothing;
//! - an explicit `await` of it (an error in any case, the function being
//!   pure) and an update of the record it is, whether it is the updated
//!   variable or a record further along the path (`let p.x = ...`, or
//!   `let q.p.x = ...` where `q` holds it in its field `p`), measure it;
//! - a recursive call through the function's own name measures every
//!   argument whose type is a provisional type of the function,
//!   conservatively;
//! - an `if` whose branches have different types measures every parameter
//!   whose provisional type is in either branch, conservatively, and the
//!   branches then agree as they would with those parameters present: a
//!   parameter and a present value join to the present type, and a
//!   parameter and a future to the either shape. (Joining a parameter with
//!   a present value as the either shape instead would leave the `if` an
//!   either, which the pure function could then not measure, and would
//!   reject `if n < 2 { n } else { ... }` against an `Int` result.)
//!
//! Capturing a parameter in a closure or a `lazy`, and binding it with a
//! `let` without an annotation, measure nothing.
//!
//! When the body's check ends, each parameter is decided: present if it was
//! measured. [`TypeTable::settle`](crate::types::TypeTable::settle) then
//! replaces the function's provisional types in the type of every expression of
//! the function, the ids of its subtree (which the parser allocates
//! contiguously: see [`Ast`](crate::ast::Ast)), and in what every function and
//! `lazy` nested in it captures, and the function's own type is its settled
//! type; which parameters were measured, and where, is recorded in
//! [`FnResolution::param_demands`](super::FnResolution::param_demands). A
//! provisional type reaches nothing outside the function but through the
//! function's type, which is settled with the rest, so this one pass is enough,
//! and a successful check leaves none (the final invariant check says so). A
//! nested function is settled when it ends, for its own parameters, and again
//! when the function enclosing it ends, for that one's, which is why settling
//! names the function.
//!
//! **Nested functions.** A provisional type of an *enclosing* function,
//! seen in a nested body through a capture, is measured for the function it
//! belongs to when every function between, the nested one included, is
//! pure (`Checker::claimable`): a pure closure can only ever see what the
//! function around it was given, so measuring the parameter there makes it
//! present, and the caller awaits it before the enclosing function runs, as
//! for a measurement in the enclosing body itself; `fn make(n: Int): fn():
//! Int { fn(): Int { n } }` therefore takes a present `n`. Through an impure
//! or memoised function or a `lazy`, which may await, the provisional type
//! is instead treated as the either shape it may become: measuring it there
//! is an await, which measures nothing for the function it belongs to, and
//! inside a pure function nested in that impure one it is the purity
//! error. This is conservative and correct: the enclosing function's
//! decision is not known while the nested body is checked, and if nothing
//! measures the parameter a future may indeed arrive, which the `lazy` or
//! impure function awaits when it runs. (An await of a parameter that turns
//! out present finds a present value, and does nothing.)
//!
//! **Where a provisional type is shown.** A diagnostic shows a provisional type
//! as what it stands for at that point (`Checker::shown`): one of a function
//! the body being checked may measure (`Checker::claimable`) as present if its
//! parameter has been measured so far and the either shape otherwise, and any
//! other enclosing function's as the either shape.
//! [`TypeTable::display`](crate::types::TypeTable::display) shows one as
//! `Int?`, which nothing the checker returns holds.
//!
//! **The call-site error.** An argument that is awaited at the top, in a
//! pure caller, because the parameter it fills is present by inference, is
//! the error "argument `x` would be awaited inside `f`, which is pure",
//! labelled with where `f`'s body first measures the parameter when `f` is
//! known (`Checker::argument_await_error`). A callee is known when it is a
//! plain name whose binding, not a mutable variable's, was initialised
//! directly with a function expression, of this input
//! ([`FnResolution::param_demands`](super::FnResolution::param_demands) says which
//! parameters were measured, and where) or of an earlier session input,
//! whose [`Scope`](super::Scope) keeps which parameters its functions' bodies measure
//! ([`KnownParams`](super::KnownParams)), but
//! not where, since that input's source is gone: the error then has no label
//! on the measuring use. A parameter written present, or of a callee that is
//! not known, gives the generic implicit-await error.
//!
//! # Inferred results
//!
//! An impure or memoised function's bare result position, one written as a
//! name, a record type, a list type or a union with no `Future` member, is
//! the either shape while the body is checked,
//! as the bare type rule says, and then takes the laziness of what the body
//! returns there: `T` if it is present, `Future<T>` if it is a future, and
//! the either shape, as written, if it may be either or has a provisional
//! type (see below), `T` being the type written there, with its own bare
//! positions inferred in the same way if it is a record type, each field
//! against the field of the same name of what the body returns there once
//! stripped (the body's other fields play no part), and if it is a list
//! type, its element type against the element type of what the body
//! returns there once stripped. So `fn get() -> {a: Int,
//! b: String} { a = 1; b = "x" }` is `fn() -> {a: Int, b: String}`, `fn
//! later() -> Int { lazy 5 }` is `fn() -> Future<Int>`, and `fn pass(x: Int)
//! -> Int { x }` is `fn(x: Int | Future<Int>) -> Int | Future<Int>`, its
//! parameter being the either shape, while `fn add(x: Int) -> Int { x + 1 }`
//! is `fn(x: Int | Future<Int>) -> Int`, and `fn l() -> List<Int> { [1,
//! lazy 2] }` is `fn() -> List<Int | Future<Int>>` (see
//! [Lists](crate::check::lists#lists)). A bare union is inferred at its top only, as a name
//! is: `fn f(c: Bool) -> Int | String { if c { 1 } else { "a" } }` is
//! `fn(c: Bool | Future<Bool>) -> Int | String`, the inside of each member
//! staying as declared. A position that is not bare keeps
//! what is written, with everything inside it: `Present<...>`,
//! `Future<...>`, a union with a `Future` member and a function type say
//! what they are. A
//! parenthesised type is what is inside it. The parameters are never
//! inferred, nor is anything of a pure function's, though a callback
//! parameter may be narrowed, in the same step (see [Narrowed
//! callbacks](#narrowed-callbacks)). A memoised function's
//! result is inferred in the same way, and its parameters are present:
//! `fn fetch(x: Int) ->> Int { x }` is `fn(x: Int) ->> Int`, and `fn
//! later(x: Int) ->> Int { lazy x }` is `fn(x: Int) ->> Future<Int>`.
//!
//! What the body returns is the value it has once every await the checker
//! marked for fitting it to the declared result has run, since the typed
//! walkers of the evaluator trust the inferred type (`Checker::value_of`): a
//! body, or a field's value, awaited at the top has its type stripped, and one
//! awaited deeply its type made present (see
//! [`TypeTable::present`](crate::types::TypeTable::present)), which is exactly
//! what the deep await's lazy shape resolves; a record block checked field by
//! field against the declared record type, whose type is the record of the
//! declared fields' types, is instead the record of what its fields' values
//! are, each found in the same way; an untyped literal has been coerced. An
//! open field, one built by dotted paths, is a present record, built by the
//! block, of what its segments are: a segment that further paths extend is such
//! a record in turn, and a leaf, `a.b.c = e`, is what `e` is, found as a
//! field's value is, since it was checked against the declared type at its path
//! as a field's value is against its field's type, and lowering stores exactly
//! `e`, with any await marked on it, into the record (see [Dotted
//! fields](crate::check::records#dotted-fields)). So `fn n() -> {a: {b: Int}} {
//! a.b = 1 }` is `fn() -> {a: {b: Int}}`, as with `a = { b = 1 }`. A soft
//! field, one a spread defines (see [Spreads and soft
//! assignment](crate::check::records#spreads-and-soft-assignment)), is what the
//! type its spread's operand gives it says, as it is: the spread stores exactly
//! the value it reads from the operand, once awaited if marked, at that field,
//! never awaiting it, and the static type is the truth, so the record really
//! holds a value of that type there. Only the definition of a field that stands
//! is inferred, a soft one overridden later playing no part. The block's items
//! are passed over once, rebuilding the segments of its open fields as a tree
//! (`Built`), which is then inferred against the declared type level by level;
//! a segment the declared type lacks plays no part, and a declared field the
//! block does not define, which a successful check rules out, keeps its
//! declared type. A position whose value's type is not a subtype of what would
//! be inferred keeps its declared type. The inferred result is a subtype of the
//! declared one, so the function's type is one too, by ordinary result
//! covariance; a written function type is not inferred, so `fn() -> Int`
//! written means the either shape as before, and accepts `fn() -> Int { 5 }`,
//! as `fn() -> Present<Int>` now does too.
//!
//! What the body returns is decided from all its *result sites* (see
//! [`return`](crate::check::loops#return)): its own result and the value of every `return`
//! for the function, as if they were the branches of one `if`
//! (`Checker::inferred_result`). Each site is inferred as the body's
//! result alone would be, and the result is the join of what they give,
//! or the declared type should that join not be a subtype of it; a site
//! of type `Never`, as a body ending in a `return` is, plays no part, and
//! a function every site of which is `Never` is inferred from `Never`, a
//! present type. Inferring each site and joining is never less precise
//! than joining the sites' types first, which would see a record block
//! checked field by field as the record of the declared fields, and is the
//! same where the sites are names, futures or eithers: `fn f(c: Bool) ->
//! Int { if c { return 1 }; 2 }` is `fn(...) -> Int`, with `lazy 2` there
//! `-> Int | Future<Int>`, and with both lazy `-> Future<Int>`.
//!
//! The result is inferred once, when the body's check ends
//! (`Checker::inferred`), and only if no error was reported while the body
//! was checked, and the declared parameter and result types have none: an
//! erroneous function keeps its declared type, so that its calls do not
//! cascade. While the body is checked, the function's own name has the
//! declared type, so a recursive call's result is the either shape at the
//! bare positions, and nothing is iterated: `fn count(n: Int) -> Int { if n
//! == 0 { 0 } else { count(n - 1) } }` is `fn(n: Int | Future<Int>) -> Int |
//! Future<Int>`, but with `sum(n - 1) + n` in the `else` branch, which
//! measures the recursive call, the result is present. Everything that sees
//! the function's value sees the inferred type (the function expression's
//! type, the binding it initialises, a session's [`Scope`](super::Scope), a closure's
//! display, and captures); a use of its own name in its body keeps the
//! declared type, a supertype. A provisional type of an enclosing pure
//! function in what the body returns counts as the either shape, so the
//! inferred type holds no provisional type, and the enclosing function's
//! settling leaves it as it is.
//!
//! # Narrowed callbacks
//!
//! In an impure or memoised function expression, a *callback parameter* is
//! one whose type is written directly as an impure function type, `fn(...)
//! -> T` (not inside `Future<...>`, a union, a record type or another
//! function type), and each parameter position of that type written bare,
//! as a name, a record type, a list type or a union with no `Future`
//! member, is *narrowable*. As written,
//! such a position
//! is the either shape: the function says it may pass the callback a
//! future there, so a pure callback that measures its parameter does not
//! fit. A body that only ever passes present values there is narrowed to
//! say so, and then it does:
//!
//! ```text
//! fn double(x: Int): Int { x * 2 }
//! fn run(f: fn(Int) -> Int) -> Int { f(1) }
//! # run is fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>
//! run(double)      # 2
//! ```
//!
//! Once the body is checked, each narrowable position becomes its *present
//! type*, the type written there with every bare type in it present (`Int`
//! for `Int`, `{a: Int}` for `{a: Int}`, and `{a: Future<Int>}` for `{a:
//! Future<Int>}`), if the body never uses the callback as a value, and
//! every call of it gives the position an argument that fits the present
//! type; otherwise it stays as written. A use as a value is any appearance
//! of the name other than as exactly the callee of a call: `let g = f`,
//! `other(f)`, `(f)(1)`, returning it, interpolating it. Each position is
//! decided on its own, all or nothing, and a callback that is never called
//! nor used has every narrowable position narrowed. An argument fits if
//! what it is where it is passed, found as an inferred result's value is
//! and inferred against the position as a bare result position is (see
//! [Inferred results](#inferred-results): once the awaits marked on it have
//! run, a record block by what its fields are, an untyped literal
//! coerced), is a subtype of the present type. So `f({ a = 1 })` fits `{a:
//! Int}`, while `f(j)`, `j` being a parameter of the function, which may be
//! a future, does not fit `Int`, and nor does a parameter of an enclosing
//! pure function, whose type is provisional. Positions not written bare,
//! `Present<T>`, `Future<T>`, `T | Future<T>` and function types, are never
//! narrowed, nor is the callback's result; a parameter written as a
//! memoised or pure function type is not a callback parameter, its bare
//! parameters being present already; and nothing of a pure function's is
//! narrowed, since it cannot call an impure callback.
//!
//! Calls count wherever they are in the body, in a nested function or
//! `lazy` too, as long as the name resolves to the parameter, not to
//! something shadowing it. While the body is checked, the parameter's
//! binding holds the index of its entry in the stack of the callback
//! parameters of the functions being checked (`Callback`, with one
//! `CallbackPosition` for each of its positions), which records the first
//! use as a value (`Checker::check_name`) and, for each position, the first
//! argument that does not fit (`Checker::check_callee`,
//! `Checker::record_argument`), each in constant time but for finding what
//! the argument is and the subtype test. When the body ends, the positions
//! are decided (`Checker::narrow_callbacks`), and the function's type is
//! interned again with the narrowed parameter types, in the same step as
//! its inferred result and on the same conditions: no error was reported
//! while the body was checked, and the declared parameter and result types
//! have none. Everything that sees the function's value sees that type.
//! Nothing inside the body changes: the callback has its written type
//! there, every expression keeps its type, and no await is added or
//! removed; the function's own name in its body has the declared type.
//!
//! Narrowing a position makes the callback's type a supertype of the
//! written one, since parameters are contravariant, so the function accepts
//! more callbacks. It is sound because the only code that can call the
//! callback is the body, which was checked against the written type and is
//! known to pass nothing at a narrowed position that does not fit its
//! present type: the value can leave the body only through a use of its
//! name as a value, which keeps every position as written.
//!
//! **The call-site label.** When an argument does not fit a callback parameter
//! of a known callee of this input (see "The call-site error" under [Inferred
//! parameters](#inferred-parameters)) that was not narrowed at some position,
//! the mismatch has a label at what kept a position the argument needs as
//! written, the first in source order: "`f` is given a value that may be a
//! future here" at the argument of a call, and "`f` is used as a value here, so
//! it may be given a future" at a use as a value. A position is needed if the
//! argument's function type does not take the type the position is written as.
//! The label is added only if the argument would have fitted, as it is or once
//! awaited, had every narrowable position been narrowed, so that it never
//! misleads about a mismatch with another cause
//! ([`FnResolution::unnarrowed`](super::FnResolution::unnarrowed) records what
//! it needs). A callee of an earlier session input has no label, its source
//! being gone.

use crate::ast::{
    ExprId, FieldTy, FieldTyRange, ItemKind, ItemRange, PathSegment, TypeExprId, TypeExprKind,
};
use crate::names::Name;
use crate::types::{Laziness, TyId};

use super::{
    AwaitMark, BodyValue, Built, CallbackPosition, Checker, Definition, NO_NODE, NarrowingBlock,
    Site, Unnarrowed, position,
};

impl Checker<'_> {
    /// Narrows the callback parameters `callbacks[start..]` of the impure or
    /// memoised function whose parameters' types are `fn_params[base..]`,
    /// whose body has just been checked without an error (see the module
    /// documentation): each narrowable position of a callback parameter
    /// that the body never uses as a value, and to which every call of it
    /// gives an argument that fits the position's present type, becomes
    /// that present type, in `fn_params`. Returns whether any did. For each
    /// callback parameter left unnarrowed at some narrowable position, what
    /// kept it so is recorded in `unnarrowed`, at the parameter's index,
    /// the vector being filled with `None` first.
    pub(super) fn narrow_callbacks(
        &mut self,
        base: usize,
        start: usize,
        unnarrowed: &mut Vec<Option<Unnarrowed>>,
    ) -> bool {
        let count = self.fn_params.len().saturating_sub(base);
        let mut narrowed = false;
        for index in start..self.callbacks.len() {
            let Some(callback) = self.callbacks.get(index).copied() else {
                continue;
            };
            let Some(&(_, declared)) = self.fn_params.get(base + callback.param) else {
                continue;
            };
            let Some(sig) = self.types.fn_sig(declared).cloned() else {
                continue;
            };
            let positions = self
                .callback_positions
                .get(callback.positions..callback.positions + callback.count)
                .unwrap_or_default();
            // The parameters as narrowed, and as they would be had every
            // narrowable position been narrowed.
            let mut params = Vec::with_capacity(sig.params.len());
            let mut every = Vec::with_capacity(sig.params.len());
            let mut blocks = Vec::with_capacity(sig.params.len());
            for (at, &(name, written)) in sig.params.iter().enumerate() {
                let Some((present, given)) = positions.get(at).and_then(|position| {
                    position
                        .narrowable
                        .map(|(_, present)| (present, position.given))
                }) else {
                    params.push((name, written));
                    every.push((name, written));
                    blocks.push(None);
                    continue;
                };
                // The first in source order of what keeps it as written.
                let block = [
                    given.map(NarrowingBlock::Given),
                    callback.used.map(NarrowingBlock::Used),
                ]
                .into_iter()
                .flatten()
                .min_by_key(|block| block.span().start);
                params.push((name, if block.is_none() { present } else { written }));
                every.push((name, present));
                blocks.push(block);
            }
            // A meta is kept as declared: only parameters are narrowed.
            let ty =
                self.types
                    .fn_type_with_rest(params, (sig.ret, sig.effect), sig.meta, sig.rest);
            let ty = self.types.with_need_set(ty, sig.needs);
            if ty != declared {
                if let Some(entry) = self.fn_params.get_mut(base + callback.param) {
                    entry.1 = ty;
                }
                narrowed = true;
            }
            if blocks.iter().any(Option::is_some) {
                let every =
                    self.types
                        .fn_type_with_rest(every, (sig.ret, sig.effect), sig.meta, sig.rest);
                let every = self.types.with_need_set(every, sig.needs);
                if unnarrowed.is_empty() {
                    unnarrowed.resize(count, None);
                }
                if let Some(entry) = unnarrowed.get_mut(callback.param) {
                    *entry = Some(Unnarrowed {
                        narrowed: every,
                        blocks: blocks.into(),
                    });
                }
            }
        }
        narrowed
    }

    /// Records that the argument `arg`, checked against `param_ty`, fills
    /// the parameter number `param` of the callback parameter
    /// `callbacks[callback]` in a call of it: if the position is
    /// narrowable and what the argument is once every await marked on it
    /// has run (see [`Checker::value_of`], [`Checker::inferred`]) does not
    /// fit its present type, the argument keeps the position as written.
    pub(super) fn record_argument(
        &mut self,
        callback: usize,
        param: usize,
        arg: ExprId,
        param_ty: TyId,
    ) {
        let Some(at) = self
            .callbacks
            .get(callback)
            .filter(|callback| param < callback.count)
            .map(|callback| callback.positions + param)
        else {
            return;
        };
        let Some(&CallbackPosition {
            narrowable: Some((written, present)),
            given,
        }) = self.callback_positions.get(at)
        else {
            return;
        };
        let span = self.ast.get(arg).span;
        // An earlier argument keeps it as written already.
        if given.is_some_and(|given| given.start <= span.start) {
            return;
        }
        let value = self.value_of(arg, param_ty);
        let found = self.inferred(written, param_ty, value);
        if !self.types.is_subtype(found, present)
            && let Some(position) = self.callback_positions.get_mut(at)
        {
            position.given = Some(span);
        }
    }

    /// The inferred result of an impure or memoised function whose result
    /// type is written as `ty`, declared `declared`, whose body is `body`
    /// and whose `return`s are `returns` (see [Inferred
    /// results](#inferred-results)): decided from all its result sites, the
    /// body's own result and every value returned, as if they were the
    /// branches of one `if`. Each site is inferred against the declared
    /// type as the body alone would be (see [`Checker::inferred`]), a site
    /// of type `Never` playing no part, and the result is the join of what
    /// they give; the declared type if that is not a subtype of it. A
    /// function every site of which is `Never` is inferred from `Never`.
    pub(super) fn inferred_result(
        &mut self,
        (ty, declared): (TypeExprId, TyId),
        body: ExprId,
        returns: &[Site],
    ) -> TyId {
        if returns.is_empty() {
            let value = self.value_of(body, declared);
            return self.inferred(ty, declared, value);
        }
        let mut joined = None;
        let sites = std::iter::once(Some(body)).chain(returns.iter().map(|site| site.value));
        for value in sites {
            let found = match value {
                Some(value) if self.awaited(value) == self.never => continue,
                Some(value) => {
                    let value = self.value_of(value, declared);
                    self.inferred(ty, declared, value)
                }
                // A `return` without a value returns `{}`.
                None => self.inferred(ty, declared, BodyValue::Typed(TyId::EMPTY)),
            };
            joined = Some(match joined {
                None => found,
                Some(so_far) => self.types.join(so_far, found).0,
            });
        }
        match joined {
            None => {
                let never = self.never;
                self.inferred(ty, declared, BodyValue::Typed(never))
            }
            Some(joined) if self.types.is_subtype(joined, declared) => joined,
            Some(_) => declared,
        }
    }

    /// Whether a pure function's parameter whose type is written as `ty` is
    /// inferred: present if the body measures it, the either shape if not.
    /// So is one written bare, as a name, a record type, a list type or a
    /// union with no `Future` member (see [`Checker::is_bare`]); not one
    /// written as `Present<...>` or `Future<...>` or a union with a
    /// `Future` member, which says what it is, nor as a function type,
    /// which is never made the either shape at its own position (see the
    /// module documentation).
    pub(super) fn is_inferred(&self, ty: TypeExprId) -> bool {
        self.is_bare(ty)
    }

    /// The inferred result of an impure function at the position of its
    /// result type written as `ty`, whose declared type is `declared`, where
    /// the body returns `value` (see the module documentation). A position
    /// written bare, as a name, a record type, a list type or a union with
    /// no `Future` member, whose declared type is therefore the either
    /// shape of a `T`, is `T` if the value is
    /// present, `Future<T>` if it is a future, and the either shape, as
    /// declared, if it may be either or is a provisional type; for a record
    /// type, `T` is the record of its declared fields, each inferred in turn
    /// against the field of the same name of the value once stripped, and
    /// for a list type, the list of its declared element type inferred in
    /// turn against the element type of the value once stripped. Every other
    /// position keeps its declared type, and so does a bare one whose value
    /// is not known well enough: a record block, or a record its dotted
    /// paths build, where a name or a list type is declared, or a value
    /// whose type is not a subtype of what would be inferred.
    ///
    /// Recurses once per level of record or list type nesting in `ty`.
    pub(super) fn inferred(&mut self, ty: TypeExprId, declared: TyId, value: BodyValue) -> TyId {
        // The fields of a record type, or the element type of a list type.
        let (fields, element) = match self.ast.type_expr(ty).kind {
            // A bare union is inferred at its top only, as a name is, and
            // an enum.
            TypeExprKind::Named(_) | TypeExprKind::Enum { .. } => (None, None),
            TypeExprKind::Union { .. } if self.is_bare(ty) => (None, None),
            TypeExprKind::Record(fields) => (Some(fields), None),
            TypeExprKind::List(element) => (None, Some(element)),
            TypeExprKind::Applied { name, args, .. }
                if Some(name) == self.constructors.list
                    && let &[element] = self.ast.type_args(args) =>
            {
                (None, Some(element))
            }
            // They say what they are, a function type at its own position
            // included; and a type that names nothing is erroneous.
            TypeExprKind::Applied { .. }
            | TypeExprKind::Union { .. }
            | TypeExprKind::Fn { .. }
            | TypeExprKind::Path(_)
            | TypeExprKind::PathApplied { .. }
            | TypeExprKind::Ref(_)
            | TypeExprKind::Instance { .. }
            | TypeExprKind::Param(_)
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => {
                return declared;
            }
        };
        if self.types.laziness(declared) != Laziness::Either {
            return declared;
        }
        let present = self.types.strip(declared);
        let (laziness, stripped) = match value {
            // A record block is present, and so is a record its dotted
            // paths build.
            BodyValue::Fields(_) | BodyValue::Open(_) => (Laziness::Present, value),
            BodyValue::Typed(found) => (
                self.types.laziness(found),
                BodyValue::Typed(self.types.strip(found)),
            ),
        };
        let shape = match (fields, element, stripped) {
            (Some(fields), _, stripped) => match self.inferred_record(fields, present, stripped) {
                Some(record) => record,
                None => return declared,
            },
            (None, Some(element), BodyValue::Typed(found)) => {
                let (Some(declared_element), Some(found_element)) = (
                    self.types.list_element(present),
                    self.types.list_element(found),
                ) else {
                    return declared;
                };
                let element =
                    self.inferred(element, declared_element, BodyValue::Typed(found_element));
                self.types.list(element)
            }
            (None, None, BodyValue::Typed(_)) => present,
            (None, _, BodyValue::Fields(_) | BodyValue::Open(_)) => return declared,
        };
        let inferred = match laziness {
            Laziness::Present => shape,
            Laziness::Future => self.types.future(shape),
            Laziness::Either | Laziness::Provisional => self.types.deferrable(shape),
        };
        match value {
            BodyValue::Typed(found) if !self.types.is_subtype(found, inferred) => declared,
            _ => inferred,
        }
    }

    /// The present member of the inferred result at a record type written
    /// with the fields `fields`, whose declared present member is the record
    /// type `present`, where the body returns `value`, stripped: the record
    /// of the declared fields, each inferred against the value's field of
    /// the same name (see [`Checker::inferred`]), or keeping its declared
    /// type if the value has no such field. A field of a record block, or
    /// of a record its dotted paths build, that dotted paths build is
    /// inferred as such a record in turn. The value's other fields play no
    /// part. `None` if `present` lacks a field written, which a record type
    /// resolved from `fields` never does.
    ///
    /// The declared fields are kept in `sorted_fields`, by name, each with
    /// its position in `fields` and its type so far, so that a record
    /// block's fields, and the children of a segment its dotted paths
    /// build, are found by binary search, in one pass over its items (see
    /// [`Checker::infer_block_fields`]) or over the segment's children.
    pub(super) fn inferred_record(
        &mut self,
        fields: FieldTyRange,
        present: TyId,
        value: BodyValue,
    ) -> Option<TyId> {
        let ast = self.ast;
        let written = ast.field_tys(fields);
        let base = self.sorted_fields.len();
        for (index, field) in written.iter().enumerate() {
            // The expansion of the file's types leaves no spread: a type
            // with one resolves to `Error`, which is not inferred. A
            // rest-field is kept as declared.
            let Some(name) = field.name() else {
                continue;
            };
            let Some(declared) = self.types.field(present, name) else {
                self.sorted_fields.truncate(base);
                return None;
            };
            self.sorted_fields.push((name, position(index), declared));
        }
        // Inferring a nested record type uses `sorted_fields` above `end` and
        // truncates back.
        let end = self.sorted_fields.len();
        self.sorted_fields
            .get_mut(base..end)
            .unwrap_or_default()
            .sort_unstable_by_key(|&(name, _, _)| name);
        match value {
            BodyValue::Typed(found) => {
                for at in base..end {
                    let Some(&(name, index, declared)) = self.sorted_fields.get(at) else {
                        continue;
                    };
                    let (Some(field), Some(found)) =
                        (written.get(index as usize), self.types.field(found, name))
                    else {
                        continue;
                    };
                    let ty = self.inferred(field.ty, declared, BodyValue::Typed(found));
                    if let Some(entry) = self.sorted_fields.get_mut(at) {
                        entry.2 = ty;
                    }
                }
            }
            BodyValue::Fields(items) => self.infer_block_fields(written, (base, end), items),
            BodyValue::Open(node) => {
                let mut child = self
                    .built
                    .get(node as usize)
                    .map_or(NO_NODE, |entry| entry.first_child);
                while let Some(&entry) = self.built.get(child as usize) {
                    let definition = match entry.leaf {
                        Some(value) => Definition::Whole(value),
                        None => Definition::Dotted(child),
                    };
                    if let Some(at) = self.sorted_at((base, end), entry.name) {
                        self.infer_field(written, at, definition);
                    }
                    child = entry.next_sibling;
                }
            }
        }
        let sorted: Vec<(Name, TyId)> = self
            .sorted_fields
            .get(base..end)
            .unwrap_or_default()
            .iter()
            .map(|&(name, _, ty)| (name, ty))
            .collect();
        // Sorted by name, so interning does not sort it again; the declared
        // fields keep their defaults (see "Field defaults" under
        // [Functions](crate::check::functions#functions)).
        let rest = self.types.record_rest(present);
        let record = self.types.record_like(present, sorted, rest);
        self.sorted_fields.truncate(base);
        Some(record)
    }

    /// Infers each of the declared fields `sorted_fields[base..end]`,
    /// written as `written`, that the record block of `items` defines, in
    /// one pass over its items: a field defined whole from its value, a
    /// soft field from the type its spread's operand gives it, and an open
    /// field from the leaves of the dotted paths that build it, which the
    /// pass rebuilds in `built`, as a tree of segments under one root for
    /// each declared field, to be inferred once it ends. A field, or a
    /// segment, that the declared type lacks plays no part.
    ///
    /// Only the definition of a field that stands is inferred: a soft one
    /// may be overridden by a later spread or field item. So from the
    /// block's first spread on, each definition is recorded in `standing`,
    /// a later one replacing an earlier one, and inferred once the pass
    /// ends; before it, a field item cannot be overridden, the block having
    /// checked without an error, and is inferred at once.
    pub(super) fn infer_block_fields(
        &mut self,
        written: &[FieldTy],
        (base, end): (usize, usize),
        items: ItemRange,
    ) {
        let ast = self.ast;
        // The root for `sorted_fields[at]` is at `roots + (at - base)`.
        // Inferring a value pushes onto `built` above what is there and
        // truncates back.
        let roots = self.built.len();
        self.built.extend(
            self.sorted_fields
                .get(base..end)
                .unwrap_or_default()
                .iter()
                .map(|&(name, _, _)| Built {
                    name,
                    leaf: None,
                    first_child: NO_NODE,
                    next_sibling: NO_NODE,
                }),
        );
        // The definition that stands for `sorted_fields[at]` is at
        // `standing + (at - base)`, once the block has had a spread.
        let standing = self.standing.len();
        let mut spread = false;
        for &item in ast.items(items) {
            let (path, value) = match ast.item(item).kind {
                ItemKind::Field { path, value, .. } => (path, value),
                ItemKind::Spread(operand) => {
                    if !spread {
                        spread = true;
                        self.standing.resize(standing + (end - base), None);
                    }
                    self.stand_spread(operand, (base, end), standing);
                    continue;
                }
                _ => continue,
            };
            let Some((first, rest)) = ast.path(path).split_first() else {
                continue;
            };
            let Some(at) = first
                .name()
                .and_then(|name| self.sorted_at((base, end), name))
            else {
                continue;
            };
            if !rest.is_empty() {
                self.build_path(position(roots + (at - base)), rest, value);
            } else if spread {
                if let Some(entry) = self.standing.get_mut(standing + (at - base)) {
                    *entry = Some(Definition::Whole(value));
                }
            } else {
                self.infer_field(written, at, Definition::Whole(value));
            }
        }
        if spread {
            for at in base..end {
                if let Some(Some(definition)) = self.standing.get(standing + (at - base)).copied() {
                    self.infer_field(written, at, definition);
                }
            }
            self.standing.truncate(standing);
        }
        for at in base..end {
            let root = position(roots + (at - base));
            // A field no dotted path builds.
            if self
                .built
                .get(root as usize)
                .is_none_or(|entry| entry.first_child == NO_NODE)
            {
                continue;
            }
            self.infer_field(written, at, Definition::Dotted(root));
        }
        self.built.truncate(roots);
    }

    /// Records, in `standing` from `standing` on, that the spread whose
    /// operand is `operand` defines each of the declared fields
    /// `sorted_fields[base..end]` that its operand's type has, once awaited,
    /// with the type it gives the field, which is what the spread reads
    /// from the operand and stores, never awaited.
    pub(super) fn stand_spread(
        &mut self,
        operand: ExprId,
        (base, end): (usize, usize),
        standing: usize,
    ) {
        let awaited = self.awaited(operand);
        let record = self.types.strip(awaited);
        for &(name, ty) in self.types.record_fields(record).unwrap_or_default() {
            let Some(offset) = self
                .sorted_fields
                .get(base..end)
                .unwrap_or_default()
                .binary_search_by_key(&name, |&(name, _, _)| name)
                .ok()
            else {
                continue;
            };
            if let Some(entry) = self.standing.get_mut(standing + offset) {
                *entry = Some(Definition::Soft(ty));
            }
        }
    }

    /// Adds the dotted path whose segments after the first are `rest`,
    /// ending in `value`, to the tree of segments in `built` whose root, the
    /// path's first segment, is `root`: each segment that is not yet a child
    /// of the one before it is added as its newest child, and the last is
    /// the leaf holding `value`.
    pub(super) fn build_path(&mut self, root: u32, rest: &[PathSegment], value: ExprId) {
        let mut node = root;
        // A field's path's segments are all names.
        for name in rest.iter().filter_map(PathSegment::name) {
            if let Some(child) = self.built_child(node, name) {
                node = child;
                continue;
            }
            let index = position(self.built.len());
            let next_sibling = match self.built.get_mut(node as usize) {
                Some(parent) => std::mem::replace(&mut parent.first_child, index),
                None => NO_NODE,
            };
            self.built.push(Built {
                name,
                leaf: None,
                first_child: NO_NODE,
                next_sibling,
            });
            node = index;
        }
        if let Some(leaf) = self.built.get_mut(node as usize) {
            leaf.leaf = Some(value);
        }
    }

    /// The child of the segment `node` of `built` named `name`, if it has
    /// one.
    pub(super) fn built_child(&self, node: u32, name: Name) -> Option<u32> {
        let mut child = self.built.get(node as usize)?.first_child;
        while let Some(entry) = self.built.get(child as usize) {
            if entry.name == name {
                return Some(child);
            }
            child = entry.next_sibling;
        }
        None
    }

    /// The index in `sorted_fields` of the entry named `name` among those
    /// in `base..end`, which are sorted by name, if there is one.
    pub(super) fn sorted_at(&self, (base, end): (usize, usize), name: Name) -> Option<usize> {
        self.sorted_fields
            .get(base..end)
            .unwrap_or_default()
            .binary_search_by_key(&name, |&(name, _, _)| name)
            .ok()
            .map(|offset| base + offset)
    }

    /// Infers the declared field `sorted_fields[at]`, written as one of
    /// `written`, which the record the body returns defines as
    /// `definition`, and records its inferred type there (see
    /// [`Checker::inferred`]): a value defined whole as it is once every
    /// await marked for fitting it to the field's declared type has run
    /// (see [`Checker::value_of`]), whether it is a field's or the leaf's
    /// of a dotted path, which is checked against the declared type at its
    /// path as a field's value is against its field's; a soft field as the
    /// type its spread's operand gives it; and a record dotted paths build
    /// as present, its fields inferred in turn.
    pub(super) fn infer_field(&mut self, written: &[FieldTy], at: usize, definition: Definition) {
        let Some(&(_, index, declared)) = self.sorted_fields.get(at) else {
            return;
        };
        let Some(field) = written.get(index as usize) else {
            return;
        };
        let found = match definition {
            Definition::Whole(value) => self.value_of(value, declared),
            Definition::Soft(ty) => BodyValue::Typed(ty),
            Definition::Dotted(node) => BodyValue::Open(node),
        };
        let ty = self.inferred(field.ty, declared, found);
        if let Some(entry) = self.sorted_fields.get_mut(at) {
            entry.2 = ty;
        }
    }

    /// What the expression `expr`, checked where a value of type `expected`
    /// was expected, is known to yield where it is used: the record of its
    /// fields if it is a record block that was checked field by field
    /// against `expected` (on the condition [`Checker::check_expr_expecting`]
    /// and [`Checker::check_fn_body`] have), whose type is then the record
    /// of the expected fields' types rather than of what its fields hold;
    /// and otherwise its type once the await marked on it has run: as it
    /// is if none is, stripped for an await at the top, and made present
    /// for a deep one, which awaits every field of its type's record whose
    /// type is lazy, at any depth, as its lazy shape lists them (see
    /// [`TypeTable::present`](crate::types::TypeTable::present)).
    pub(super) fn value_of(&mut self, expr: ExprId, expected: TyId) -> BodyValue {
        if self.types.laziness(expected) != Laziness::Future
            && self.types.is_record(self.types.strip(expected))
            && let Some((_, items)) = self.record_block_within(expr)
        {
            return BodyValue::Fields(items);
        }
        BodyValue::Typed(self.awaited(expr))
    }

    /// The type of what `expr` yields where it is used, once the await
    /// marked on it has run: its type as it is if none is, stripped for an
    /// await at the top, and made present for a deep one (see
    /// [`Checker::value_of`]).
    pub(super) fn awaited(&mut self, expr: ExprId) -> TyId {
        let ty = self.expr_types[expr.index()];
        match self.awaits[expr.index()] {
            AwaitMark::None => ty,
            AwaitMark::Top => self.types.strip(ty),
            AwaitMark::Deep => self.types.present(ty),
        }
    }
}
