//! Checking functions: their bodies, results, defaults and effects, and the
//! calls of them.
//!
//! # Functions
//!
//! A function expression's type is known before its body is checked, since
//! every parameter and the return type are declared: its parameter and
//! return types are resolved and its function type interned first, and then
//! the body is checked as a block whose scope starts with the parameters,
//! against the return type as an annotated initialiser is checked against
//! its annotation. The parameters are bindings like any other, so a `let`
//! may shadow one and a `:=` rebind one, in the body's own block, since a
//! parameter is not a mutable variable. (The three things the body decides
//! are whether a pure function's bare parameters may be futures, whether
//! an impure function's bare result is, and whether an impure function's
//! callback parameter may be given futures: see [Inferred
//! parameters](crate::check::inference#inferred-parameters), [Inferred
//! results](crate::check::inference#inferred-results) and [Narrowed
//! callbacks](crate::check::inference#narrowed-callbacks).)
//!
//! A named function item, `fn f(...) ...`, is checked as the `let f =
//! fn(...) ...` it is parsed as, and a public function item, `def f(...)
//! ...`, as the field `f = fn(...) ...`: in every rule about fields (whether
//! a block has fields and so cannot have a result, the record's fields and
//! their order, a field defined twice, a soft field overridden) a `def` is
//! that field. The tree marks both as written, which only the printer
//! reads here.
//!
//! **Frames.** Each function runs in a frame of its own. The checker keeps a
//! stack of the functions it is inside (`Function`); entering one saves the
//! enclosing frame's slot allocation and starts a fresh one at slot 0, in
//! which the parameters take the first slots, and leaving it records the
//! function's frame size and restores the enclosing allocation. The
//! top-level body is the outermost frame. What the checker learns about each
//! function is recorded in [`Resolution`](super::Resolution) as a [`FnResolution`], indexed by
//! its [`FnId`].
//!
//! **Captures.** A use of a name resolves to a [`VarRef`](super::VarRef): a slot of the
//! current frame, a captured value, or the function being called. `lookup`
//! finds the binding as for any variable; if it lies below the innermost
//! function's first binding, it belongs to an enclosing function (or the
//! top level), and the use is a capture. Every function between the
//! binding's home and the use captures it: the outermost of them copies it
//! from its home frame (its slot, or the home function's callee), and each
//! function nested in that one copies it from the captures of the function
//! enclosing it, so the use reads its own function's capture. This is the
//! upvalue chain of Lua and the free-variable cells of Python, except that
//! there are no cells: each closure holds copies, made when its function
//! expression is evaluated. For a variable that is not mutable, `:=`
//! declares a new binding rather than assigning to the old one: after `let
//! x = 1; let f = fn(): Int { x }; x := 2`, the `x` that `f` uses is the
//! first binding, whose value is 1 whenever it is read, so `f()` is 1. A
//! mutable variable is one binding, whose slot each `:=` writes (see
//! [Mutable variables](crate::check::mutation#mutable-variables)), and capturing by value is what
//! keeps it from being shared: after `let mut x = 1; let f = fn(): Int { x
//! }; x := 2`, `f()` is 1 too, since `f` copied `x` when it was made, and
//! no function can rebind the variable it copied, which `:=` from inside
//! one is rejected for. So no value is ever shared and changed. A capture
//! list is searched linearly, since it is short.
//!
//! **Self-reference.** A function expression that is the direct initialiser of
//! a single-name `let`, field, named function item or `def` (not through
//! parentheses) sees that name throughout its body as the function being
//! called: a binding declared before the parameters, so a parameter of the same
//! name shadows it, of kind `BindingKind::Callee`, with no slot. It resolves to
//! [`VarRef::Callee`](super::VarRef::Callee), which the evaluator serves from
//! the closure being run, so a recursive closure never holds a reference to
//! itself and reference counts stay free of cycles. It cannot be rebound with
//! `:=`, and never reaches a session's [`Scope`](super::Scope). A `def` named
//! by a dotted path has no such name: it calls itself by its path, as any other
//! top-level function (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)). A function is
//! visible from the item after its own, as any binding is, except a top-level
//! function, whose name is in scope in its whole input, so that top-level
//! functions may be used above their declarations and call each other in any
//! order (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)). One top-level
//! function's use of another is not self-reference and not a capture: it reads
//! the other's slot of the top level's frame
//! ([`VarRef::Fn`](super::VarRef::Fn)). Mutual recursion between functions that
//! are not top-level does not exist: an inner function sees only what is above
//! it.
//!
//! **Calls.** The callee must have a function type. Positional arguments fill
//! its parameters left to right, the trailing block fills the last, and named
//! arguments fill the parameters of their names; each argument is then checked
//! against its parameter's type as an annotated initialiser is. The parameter
//! each argument fills is recorded for lowering in
//! [`Resolution::arg_params`](super::Resolution::arg_params). Too many
//! positional arguments, an unknown or unusable name, a parameter filled twice
//! and parameters left unfilled are each reported, and make the call erroneous.
//! A binding initialised directly with a function expression remembers which,
//! so that a call through its name knows the function's [`FnResolution`],
//! unless it is a mutable variable, which may come to hold another. A call of a
//! built-in function is typed apart (see [Built-in
//! functions](crate::check::calls#built-in-functions)).
//!
//! **Rest parameters.** A function's rest parameter, `...words: T`, at most
//! one, "a function has one rest parameter at most" for a second, is part
//! of its type ([`FnTy::rest`](crate::types::FnTy::rest)), whose parameter
//! type is its elements', `T`; the body declares it a `List<T>`. A call
//! gives it every positional argument from its position on, each checked
//! against `T`, and every spread argument, `...xs`, checked against
//! `List<T>`; they are written together, "a rest parameter's arguments are
//! written together" otherwise, so that they are evaluated in their order.
//! It is never named, "a rest parameter is given by position", and never
//! unfilled: given nothing it is the empty list. A spread where the callee
//! has no rest parameter is "a spread argument fills a rest parameter".
//! Each call of a callee with one is recorded with its number
//! ([`Resolution::call_rest`](super::Resolution::call_rest)), and lowering
//! gives it a list of its arguments, where the first is written.
//!
//! **Defaults.** A parameter of a function expression may have a default,
//! `greeting: String = "hello"`, in any position, before, between or after
//! parameters without one. A default is any expression. One that is a
//! *constant*, a literal (a string without interpolation, raw or not, an
//! integer, a float, `true` or `false`), `-` applied directly to an integer
//! or float literal, or a list literal without spreads or a record block of
//! fields `NAME = CONSTANT` (of one name each, not annotated; `{}` included)
//! whose elements or fields are constants, nested freely
//! ([`crate::ast::is_constant`]), is checked against its parameter's type,
//! as resolved for the function before any inference (the either shape of
//! an impure function's bare parameter, which a constant fits as the
//! present value it is), as an annotated initialiser is against its
//! annotation, in the function's own frame before its parameters are
//! declared (`Checker::check_defaults`): so `[]` is any list, an integer is
//! a `Float` where one is expected, and a mismatch is the usual error, at
//! the default, "expected because of this annotation" at the parameter's
//! type. What it is once checked is kept as a [`Constant`], an owned value
//! that names no node of any tree, in [`FnResolution::defaults`], as
//! [`ParamDefault::Constant`](super::ParamDefault::Constant).
//!
//! Any other default is its parameter's *default function*, which the parser
//! makes (see the parser's [Functions and
//! calls](crate::parser::functions#functions-and-calls)): a function of the parameters
//! before it, as they are written, whose result type is the parameter's, whose
//! effect is the function's (a memoised function's is impure), whose marker is
//! the function's, and whose body is the default. It is checked
//! (`Checker::check_default_fn`) where the function expression is, before the
//! function's own frame is entered, as a function written there would be: so it
//! sees the function's lexical scope, what is declared where the function is
//! written, and captures from it, and the parameters before its own, with the
//! types the function gives them. It cannot see its own parameter or a later
//! one: a read of one is "undefined variable `b`", labelled "`b` is a parameter
//! from here on" at the first of them and noted "a parameter's default sees
//! only the parameters before it". Its body is checked against the parameter's
//! type as a constant default is, and with its effect, so a pure function's
//! default is pure code: calling an impure function in it, or awaiting, is the
//! usual purity error, labelled at the function's `:`. A pure function's
//! default's bare parameters are inferred as any pure function's are (see
//! [Inferred parameters](crate::check::inference#inferred-parameters)), and
//! what it measures its function measures, as if its body did, before the body:
//! the default runs on the arguments before the body does, so `fn f(a: Int, b:
//! Int = a * 2): {v: Int} { v = b }` takes a present `a`, and the call-site
//! error for a future there is labelled at the default's measuring use. Its
//! closure is made where the function's is, and kept in the function's closure
//! after the function's own captures: [`FnResolution::defaults`] has, for its
//! parameter, [`ParamDefault::Function`](super::ParamDefault::Function) with
//! the index of that capture. A top-level function's default's copies are the
//! function's own, which decide where its closure is made (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)), and what a default
//! mentions the function mentions (see [`crate::mentions`]). The function's
//! type says nothing of defaults: `fn(name: String, greeting: String =
//! "hello")` is `fn(name: String, greeting: String)`, wherever it is shown.
//!
//! Only a call that knows the function gives defaults: one of a *known callee*
//! (`KnownCallee`, `Checker::known_callee`), whose callee is the name of a
//! binding initialised directly with the function expression (a `let`, a named
//! `fn` item, a field or a `def`, but not a mutable variable), a `def` named by
//! a dotted path called by its path, a function of an earlier session input
//! called by its name, or a `def` of another file reached through an import or
//! by its path, or by its name from another file of its directory chain.
//! Positional arguments, the trailing block and named arguments fill parameters
//! as above; then each parameter left unfilled that has a default is given it
//! (`Checker::fill_defaults`), and only the parameters left unfilled after that
//! are reported, as before, so `pad(1)`, with `def pad(n: Int = 0, s: String)`,
//! fills `n` and is "expected 2 arguments, found 1". The parameters a call
//! gives their defaults, with their defaults, are recorded by the call's id for
//! lowering ([`Resolution::call_defaults`](super::Resolution::call_defaults)),
//! which passes each as an argument, so the callee sees an ordinary full list
//! of arguments: a constant as its value, and a default function's result,
//! which the call computes on its own side once the arguments before it are
//! known, before the callee's body starts, each time it leaves the parameter
//! out (see [`crate::eval`], "Calls"). A call through any other function value
//! (a parameter, `let g = greet`, an element of a list, a mutable variable, a
//! def-type reference, `DT PATH`, whose function is not the def-type's) fills
//! every parameter, as ever. Nor does a recursive call, through the function's
//! own name in its body or through a member of the group being checked (see
//! [Top-level functions](crate::check::toplevel#top-level-functions)), which
//! has no known callee. A function of an earlier input, or a `def` of another
//! file, has its defaults carried, beside which parameters its body measures,
//! as a [`KnownParams`](super::KnownParams), in the session's
//! [`Scope`](super::Scope) or the workspace's cell of the `def`.
//!
//! **Field defaults.** A field of a record type may have a default too,
//! `type Install = {from: String, exclude: List<String> = []}`, wherever a
//! record type is written: a `type`, an annotation, a parameter's or a
//! return type, a list, function or union type, another record type. It
//! is a constant by the rule above (`Checker::is_constant`); anything else
//! is "a field's default must be a constant", at it, with the same note.
//! It is checked against its field's type as a parameter's default is
//! against its parameter's, in the mode the type is resolved in (the
//! either shape of an impure function's bare parameter taking it as the
//! present value it is), the usual mismatch at it if it does not fit
//! (`Checker::field_default`), each time the type it is written in is
//! resolved, as the rest of the type is (a top-level function's signature,
//! resolved first in silence, reports it once). What it
//! is once checked, a [`Constant`], is part of the record type
//! ([`crate::types::RecordTy::defaults`]); a record type with an erroneous
//! default is erroneous. A declared type's definition is copied into each
//! tree using it apart from its own (see [Type
//! declarations](crate::check::resolve#type-declarations)), its defaults as the constants they
//! are written as, with no type known (`written_constant`): there each is
//! fitted to its field's type as `check_expr_expecting` would check it
//! (`Checker::fit_constant`: an integer a `Float` where one is expected, a
//! record given the defaults of its own type), with nothing reported, the
//! declaration's own file reporting what is wrong; one that is no
//! constant or does not fit fails the check quietly.
//!
//! A record literal (a record block, or `{}`, the empty record's, through any
//! parentheses) checked against an expected type that is, once stripped of the
//! either shape of an impure function's bare parameter or result, a record type
//! with defaults, which it lacks fields of, by a field item or a spread whose
//! type has the field, is *given* each such field: the field is not missing,
//! the literal's type has it, of the expected type's type of it, so it is read
//! with no `is`, and the field with its default is recorded for lowering by the
//! literal's id
//! ([`Resolution::record_fills`](super::Resolution::record_fills)), which
//! builds the record with it, so at run time the value simply has the field
//! (`Checker::report_missing_fields`). So `{}` where such a type is expected is
//! the record of its defaults if every field of it has one, and otherwise each
//! other field is missing, as for any record literal (`Checker::empty_record`);
//! where no record type with defaults is expected itself, `{}` is checked as
//! ever. That is wherever a literal is checked against an expected type: an
//! annotated `let` or field, an ascription, an argument, a list's element, a
//! field of an enclosing literal, `:=`, a function's body and a `return`'s
//! value against its return type, a meta's body, and a parameter's or a field's
//! default that is a record literal, whose constant then has the fields given
//! it. A union of record types expected gives nothing, its one record member's
//! missing fields being missing as ever, nor does a dotted field's segment (an
//! open field).
//!
//! Where such a type is expected itself, it also reaches, for its defaults
//! alone, a literal that a block results in, at any depth of blocks,
//! labelled or not, through parentheses, or that a branch of an `if` with
//! `else` does, every `else if` included (`Checker::fill_result`): so a
//! function's body or a meta's whose result is a literal, as `defmeta DT
//! PATH = { ... }`'s is, a block or `if` written where the type is
//! expected, a `for`'s body where a list of the type is expected of the
//! loop (see [For](crate::check::loops#for)), a `break`'s value for such a loop, and the body
//! of a `for` spread in a list literal whose elements are expected to be of
//! the type, `[...for x in xs { { ... } }]` (see [Lists](crate::check::lists#lists)). Where the
//! elements of such a list literal are expected to be of the type, a spread
//! of a block with a result or of an `if` with `else` reaches, through the
//! same blocks and branches (`Checker::fill_elements`), a list literal they
//! result in, whose elements are then reached as the enclosing literal's
//! are, each an element that is no spread, as above, and each spread of a
//! `for`, a block or an `if` in turn, and a `for` they result in, whose
//! body is reached as a spread `for`'s is: `[...if c { xs } else { [{ ...
//! }] }]`, `[...{ let y = 1; [{ ... }] }]`. A
//! literal reached so that lacks a defaulted field, by its field items, and
//! has no spread is checked against the type as any literal is where it is
//! expected, field by field, and given the defaults it lacks; one that
//! gives every defaulted field, or has a spread, is checked as it is with
//! nothing expected, as ever. Nothing else of the block, the `if`, the
//! loop or the list literal changes: a result or a branch that is no
//! literal, a name or a call, is checked as with nothing expected, `[]`
//! included, the branches are joined, a list literal's elements are
//! joined, a labelled block's `return`s are checked and joined with its
//! result, and the whole must then fit the type where it stands, as ever, a
//! mismatch being reported there. A record type without defaults reaches
//! none of these, nor does one that is a union's member; nor is the result
//! of an `if` without `else`, or a spread of anything else, a list literal
//! itself included, reached. Nothing else is
//! given anything: a value that is not such a literal, a variable, a
//! call's result, a field read, a literal checked with no expected type,
//! that lacks a defaulted field is the usual mismatch where it meets the
//! type, noted "a default fills only a record literal written where `T` is
//! expected", `T` the record type, shown as types are
//! (`Checker::default_note`); nothing is converted at run time.
//!
//! Defaults do not change which values a type admits: subtyping and the join
//! ignore them (see [`crate::types`], "Defaults"), and where two types must be
//! one, as the operands of `==` must, a record type with defaults is one with
//! the record type without them
//! ([`TypeTable::same`](crate::types::TypeTable::same)). Every type is shown
//! with its defaults, after their fields' types, `{exclude: List<String> = [],
//! from: String}`, in errors, hovers and the dumps; a function's type shows
//! those of its parameters' record types, which are part of them, unlike its
//! own parameters' defaults.
//!
//! **Purity.** A function declared with `:` is pure, one declared with `->`
//! impure, and one declared with `->>` impure and memoised: its [`Effect`]
//! is one of `Pure < Memo < Impure`, and only `Pure` is pure. A memoised
//! function's body is an impure context exactly as an impure function's
//! is; memoisation is the evaluator's business (see [`crate::eval`]), and
//! what the checker does differently for it is only what its bare types
//! mean (see [Futures](crate::check::futures#futures)). The top level is impure, and a block has
//! the purity of the function it is in. Calling a function that is not
//! pure, an impure or a memoised one, from a pure one is an error, and so
//! is awaiting (see [Futures](crate::check::futures#futures)). Those are the only gated
//! operations: a pure function may create, pass, store and return impure
//! and memoised functions and futures.

use crate::ast::{
    ArgRange, Effect, ExprId, ExprKind, FnDecl, FnId, ItemRange, Param, TypeExprId, symbol_span,
};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::names::Name;
use crate::source::Span;
use crate::types::{FnTy, Laziness, TyId, TyKind};

use super::{
    Binding, BindingKind, Checker, Constant, Expecting, FnResolution, Function, PURE_CONTEXT,
    RESULT_WHY, Reach, Slot, bareness_of, merged_defaults, position,
};

impl Checker<'_> {
    /// Pops the innermost function off the stack of functions being
    /// checked, recording what was found for it, with `param_count`
    /// parameters and the callee name `callee`, and restores the enclosing
    /// frame's slot allocation. Returns which of its parameters its body
    /// measures: where it first does, for each.
    pub(super) fn finish_function(
        &mut self,
        param_count: u32,
        callee: Option<Name>,
    ) -> Vec<Option<Span>> {
        let Some(finished) = self.functions.pop() else {
            return Vec::new();
        };
        let slot_names = std::mem::replace(&mut self.slot_names, finished.outer_slot_names);
        self.fn_resolutions[finished.fn_id.index()] = FnResolution {
            frame_size: self.frame_size,
            param_count,
            captures: finished
                .captures
                .iter()
                .map(|capture| (capture.source, capture.ty))
                .collect(),
            callee,
            slot_names,
            param_demands: finished.demanded.clone(),
            // Recorded by `check_fn`, for a function that has any.
            unnarrowed: Vec::new(),
            defaults: Vec::new(),
        };
        // A top-level function's copies decide where its closure is made.
        if self.functions.is_empty()
            && let Some(copies) = self
                .checking_top
                .and_then(|top| self.top_copies.get_mut(top))
        {
            *copies = finished
                .captures
                .iter()
                .map(|capture| capture.binding)
                .collect();
        }
        (self.next_slot, self.frame_size) = (finished.outer_next_slot, finished.outer_frame_size);
        finished.demanded
    }

    /// Types the function expression `id`, whose details are `function`,
    /// and checks its body in a frame of its own (see the module
    /// documentation). Its type is known, and recorded, before the body is
    /// checked: the function type of its declared parameter and return
    /// types, or `Error` if any of them is. A bare type in them is present
    /// for a pure function, the either shape for an impure one, and for a
    /// memoised one present in the parameters and the either shape in the
    /// result (see `bareness_of`), except that a pure function's parameter
    /// whose type is written as a name or a record type has a provisional
    /// type while the body is checked, and is present if the body measures
    /// it and the either shape otherwise: once the body is checked, the type
    /// of every expression in the function is settled accordingly, and so
    /// is the function's type. An impure or memoised function whose body
    /// was checked without an error then has its result inferred from what
    /// the body returns (see [`Checker::inferred`]) and its callback
    /// parameters narrowed by what the body gives them (see
    /// [`Checker::narrow_callbacks`]), and its type, recorded again, is the
    /// function type of those parameters and that result. Its parameters'
    /// defaults are checked before its body, in its frame (see
    /// [`Checker::check_defaults`]).
    pub(super) fn check_fn(&mut self, id: ExprId, function: FnId) -> TyId {
        if let Some(&def) = self.definitions.get(&function) {
            return self.check_definition(id, function, def);
        }
        if let Some(&def) = self.defmetas.get(&function) {
            return self.check_defmeta(id, function, def);
        }
        let callee = self.callee_name.take();
        let decl = *self.ast.fn_decl(function);
        let pure = decl.effect == Effect::Pure;
        let (params_mode, ret_mode) = bareness_of(decl.effect);
        let params = self.ast.params(decl.params);
        let base = self.fn_params.len();
        let mut provisional = false;
        // Each default and the type it is checked against: its parameter's,
        // before any inference.
        let mut defaults = Vec::new();
        // Each parameter's type before any inference, which its default
        // functions' copies of it are given.
        let mut resolved = Vec::with_capacity(params.len());
        self.check_rests(params);
        for (index, param) in params.iter().enumerate() {
            let mut ty = self.resolve_type(param.ty, params_mode);
            resolved.push(ty);
            if let Some(default) = param.default {
                defaults.push((index, default, (ty, param.ty)));
            }
            if pure && ty != TyId::ERROR && self.is_inferred(param.ty) {
                ty = self.types.provisional(function, position(index), ty);
                provisional = true;
            }
            self.fn_params.push((Some(param.name), ty));
        }
        let ret = if let Some(ret) = decl.ret {
            self.resolve_type(ret, ret_mode)
        } else {
            // The parser gives every function expression a return type.
            self.errors.push(
                InternalError {
                    message: "a function expression has no return type",
                    span: self.ast.get(id).span,
                }
                .into(),
            );
            TyId::ERROR
        };
        let mut ty = self.function_type(function, base, (ret, decl.effect));
        self.expr_types[id.index()] = ty;
        self.check_def_type_signature(function, ty);
        let ret_span = decl.ret.map_or(self.ast.get(decl.body).span, |ret| {
            self.ast.type_expr(ret).span
        });
        let (default_fns, measured) = self.check_default_fns((params, &resolved), &mut defaults);
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: decl.effect,
            marker: decl.marker,
            keyword: symbol_span("fn", self.ast.get(id).span.start),
            // What its defaults measure, which come before its body.
            demanded: measured,
            result: Some((ret, ret_span)),
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let defaults = self.check_defaults(&defaults, params.len());
        let span = self.ast.get(id).span;
        let errors = self.errors.len();
        let callbacks = (self.callbacks.len(), self.callback_positions.len());
        self.check_fn_body(&decl, base, (callee, ty, span), ret);
        let returns = self
            .functions
            .last_mut()
            .map(|function| std::mem::take(&mut function.returns))
            .unwrap_or_default();
        // An impure or memoised function's bare result takes its laziness
        // from the body and its `return`s, and its callback parameters are
        // narrowed, unless something is wrong with the function.
        let mut unnarrowed = Vec::new();
        if !pure && ty != TyId::ERROR && self.errors.len() == errors {
            let mut result = ret;
            if let Some(ret_expr) = decl.ret {
                result = self.inferred_result((ret_expr, ret), decl.body, &returns);
            }
            let narrowed = self.narrow_callbacks(base, callbacks.0, &mut unnarrowed);
            if result != ret || narrowed {
                ty = self.function_type(function, base, (result, decl.effect));
                self.expr_types[id.index()] = ty;
            }
        }
        self.callbacks.truncate(callbacks.0);
        self.callback_positions.truncate(callbacks.1);
        self.fn_params.truncate(base);
        let demanded = self.finish_function(decl.params.len(), callee);
        self.add_default_copies();
        if let Some(resolution) = self.fn_resolutions.get_mut(function.index()) {
            resolution.unnarrowed = unnarrowed;
            resolution.defaults =
                merged_defaults(defaults, &default_fns, resolution.captures.len());
        }
        if !provisional {
            return ty;
        }
        let decisions: Vec<bool> = demanded.iter().map(Option::is_some).collect();
        self.settle_function(id, function, &decisions);
        self.expr_types[id.index()]
    }

    /// Checks each default of `defaults`, the defaults of a function whose
    /// parameters are `params`, of the types `resolved` before any inference,
    /// each with its parameter's number, that is no constant, as its default
    /// function (`Checker::check_default_fn`), in the scope the function is
    /// written in, before the function's frame is entered (see "Calls" under
    /// [Functions](crate::check::functions#functions)), and takes it out of
    /// `defaults`, which keeps the constants. Returns each parameter with a
    /// default function, in order, with whether it checked without an error,
    /// and, for each parameter, where a default first measures it, if one does.
    pub(super) fn check_default_fns(
        &mut self,
        (params, resolved): (&[Param], &[TyId]),
        defaults: &mut Vec<(usize, ExprId, (TyId, TypeExprId))>,
    ) -> (Vec<(usize, bool)>, Vec<Option<Span>>) {
        let mut default_fns = Vec::new();
        let mut measured = vec![None; params.len()];
        for &(index, _, expected) in defaults.iter() {
            let Some(default_fn) = params.get(index).and_then(|param| param.default_fn) else {
                continue;
            };
            // A default in a default sees neither's.
            let outer = self.defaults_checked.unseen.len();
            let unseen = params.get(index..).unwrap_or_default();
            let unseen = unseen.iter().map(|param| (param.name, param.span));
            self.defaults_checked.unseen.extend(unseen);
            let earlier = resolved.get(..index).unwrap_or_default();
            let (checked, demanded) = self.check_default_fn(default_fn, (earlier, expected));
            self.defaults_checked.unseen.truncate(outer);
            default_fns.push((index, checked));
            for (first, found) in measured.iter_mut().zip(demanded) {
                *first = first.or(found);
            }
        }
        defaults.retain(|&(index, _, _)| {
            params
                .get(index)
                .is_some_and(|param| param.default_fn.is_none())
        });
        (default_fns, measured)
    }

    /// Once a function's body is checked, if it is the top-level function
    /// whose body is being checked, adds the copies of its defaults to its
    /// own (see "Calls" under [Functions](crate::check::functions#functions)).
    pub(super) fn add_default_copies(&mut self) {
        let copies = &mut self.defaults_checked.copies;
        if copies.is_empty() || !self.functions.is_empty() {
            return;
        }
        if let Some(own) = self
            .checking_top
            .and_then(|top| self.top_copies.get_mut(top))
        {
            own.append(copies);
            own.sort_unstable();
            own.dedup();
        }
        copies.clear();
    }

    /// Checks the default function `function` of a parameter whose type, as
    /// resolved for its function before any inference and as written, is
    /// `expected` (see "Calls" under
    /// [Functions](crate::check::functions#functions)): in a frame of its own,
    /// as a function written where its parameter's function is, whose
    /// parameters are the ones before the parameter, declared with the types
    /// `earlier` their function resolved them to before any inference, not
    /// resolved again, so that their errors are reported once and a memoised
    /// function's default sees them as its body does, and whose body, the
    /// default, is checked against `expected` as an annotated initialiser is
    /// against its annotation. Its effect is its function's, so a pure
    /// function's default is checked as pure code, its bare parameters inferred
    /// as any pure function's are (see [Inferred
    /// parameters](crate::check::inference#inferred-parameters)) and settled
    /// when it ends. Returns whether it checked without an error, and, for each
    /// of its parameters, where it first measures it: for a pure function, what
    /// its default measures it measures, since the default runs on its
    /// arguments before its body does.
    pub(super) fn check_default_fn(
        &mut self,
        function: FnId,
        (earlier, (expected, written)): (&[TyId], (TyId, TypeExprId)),
    ) -> (bool, Vec<Option<Span>>) {
        // A function in the body must not see a name meant for another.
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        let pure = decl.effect == Effect::Pure;
        let params = self.ast.params(decl.params);
        let base = self.fn_params.len();
        let mut provisional = false;
        for (index, param) in params.iter().enumerate() {
            let mut ty = earlier.get(index).copied().unwrap_or(TyId::ERROR);
            if pure && ty != TyId::ERROR && self.is_inferred(param.ty) {
                ty = self.types.provisional(function, position(index), ty);
                provisional = true;
            }
            self.fn_params.push((Some(param.name), ty));
        }
        let context = self.ast.type_expr(written).span;
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: decl.effect,
            marker: decl.marker,
            keyword: decl.marker,
            demanded: vec![None; params.len()],
            result: Some((expected, context)),
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let outer = (self.block_start, self.fields_start);
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        for (index, param) in params.iter().enumerate() {
            let ty = self.param_type_in_body(param, base + index);
            self.declare_binding(param.name, ty, Some(param.span), BindingKind::Variable);
        }
        let errors = self.errors.len();
        let found = if expected == TyId::ERROR {
            self.check_expr(decl.body);
            TyId::ERROR
        } else {
            self.check_expr_expecting(
                decl.body,
                expected,
                context,
                "expected because of this annotation",
            )
        };
        self.bindings.truncate(self.block_start);
        (self.block_start, self.fields_start) = outer;
        self.fn_params.truncate(base);
        let top = self.functions.len() == 1;
        let demanded = self.finish_function(decl.params.len(), None);
        if provisional {
            let decisions: Vec<bool> = demanded.iter().map(Option::is_some).collect();
            self.settle_function(decl.body, function, &decisions);
        }
        // A top-level function's default function's copies are the
        // function's own, which `finish_function` took for its alone.
        if top
            && let Some(copies) = self
                .checking_top
                .and_then(|top| self.top_copies.get_mut(top))
        {
            self.defaults_checked.copies.append(copies);
        }
        (
            found != TyId::ERROR && self.errors.len() == errors,
            demanded,
        )
    }

    /// Checks the defaults of a function of `count` parameters, `defaults`,
    /// each with its parameter's number and the type it is checked against, its
    /// parameter's as resolved before any inference, and as written (see
    /// "Calls" under [Functions](crate::check::functions#functions)), each a
    /// constant, in the function's own frame before its parameters are
    /// declared, so that the slots a record default's check takes are the
    /// function's and are free again after it: each is checked against the type
    /// as an annotated initialiser is against its annotation. Returns the
    /// constant each parameter's default is, for [`FnResolution::defaults`]:
    /// empty if none has one, and `None` for a parameter without one or whose
    /// default is in error.
    pub(super) fn check_defaults(
        &mut self,
        defaults: &[(usize, ExprId, (TyId, TypeExprId))],
        count: usize,
    ) -> Vec<Option<Constant>> {
        if defaults.is_empty() {
            return Vec::new();
        }
        let mut constants = vec![None; count];
        for &(param, default, (ty, written)) in defaults {
            if ty == TyId::ERROR {
                continue;
            }
            let errors = self.errors.len();
            let context = self.ast.type_expr(written).span;
            let found = self.check_expr_expecting(
                default,
                ty,
                context,
                "expected because of this annotation",
            );
            if found == TyId::ERROR || self.errors.len() != errors {
                continue;
            }
            if let Some(entry) = constants.get_mut(param) {
                *entry = self.constant(default);
            }
        }
        constants
    }

    /// Settles every provisional type of function `function`, the function
    /// expression `id` (for a default function, which has none, its body: see
    /// "Calls" under [Functions](crate::check::functions#functions)), whose
    /// body has been checked, by `decisions`, which says for each of its
    /// parameters whether the body measures it: in the type of every expression
    /// of the function, its own included, which are the ids of its subtree, and
    /// in what every function and `lazy` nested in it captures. A provisional
    /// type reaches nothing outside the function but through the function's
    /// type, which is settled here with the rest.
    pub(super) fn settle_function(&mut self, id: ExprId, function: FnId, decisions: &[bool]) {
        let ast = self.ast;
        for node in ast.subtree(id) {
            if let Some(ty) = self.expr_types.get(node.index()).copied() {
                let settled = self.types.settle(ty, function, decisions);
                self.expr_types[node.index()] = settled;
            }
            let (ExprKind::Fn(nested) | ExprKind::Lazy(nested)) = ast.get(node).kind else {
                continue;
            };
            // A function's default functions are nested in it with no node
            // of their own (see "Calls" under [Functions](crate::check::functions#functions)).
            let defaults = ast.params(ast.fn_decl(nested).params);
            let defaults = defaults.iter().filter_map(|param| param.default_fn);
            for nested in std::iter::once(nested).chain(defaults) {
                let Some(resolution) = self.fn_resolutions.get_mut(nested.index()) else {
                    continue;
                };
                for (_, ty) in &mut resolution.captures {
                    *ty = self.types.settle(*ty, function, decisions);
                }
            }
        }
    }

    /// Types the call `id` of `callee` with the arguments `args` (see the
    /// module documentation): the callee's return type, or `Error` if the
    /// callee is not a function or the arguments do not fit its parameters.
    /// Every argument is checked whatever goes wrong. A callee that may be
    /// a future is awaited. A call of a callback parameter through its name
    /// records what it gives each position (see
    /// [`Checker::record_argument`]), and is not a use of it as a value.
    pub(super) fn check_call(&mut self, id: ExprId, callee: ExprId, args: ArgRange) -> TyId {
        if let ExprKind::Field {
            expr: receiver,
            name,
        } = self.ast.get(callee).kind
            && self.sees_method(name)
        {
            return self.check_field_call(id, (callee, receiver, name), args);
        }
        // TEMPORARY (alias rule): the callee of a call may be a built-in whose typing is special,
        // or an alias of one (see the [inference
        // limits](https://learn.peios.org/gcsl/language/inference-limits/)); the call is then typed
        // by its
        // rule.
        let outer = self.alias_site.replace(callee);
        let outer_call = self.call_site.replace(callee);
        let mut unparened = callee;
        while let ExprKind::Paren(inner) = self.ast.get(unparened).kind {
            unparened = inner;
        }
        let outer_generic = self.calls.callee.replace(unparened);
        let (callee_ty, callback) = self.check_callee(callee);
        self.calls.callee = outer_generic;
        (self.alias_site, self.call_site) = (outer, outer_call);
        self.call_of(id, (callee, callee_ty, callback), args)
    }

    /// Types the call `id` of `callee`, of `callee_ty`, which is no
    /// function type, with the arguments `args`, each checked alone: a
    /// callee of type `Never` is never there to call, and gives `Never`;
    /// any other is "cannot call a value of type T", unless it is
    /// erroneous, its error reported where it is. Apart from
    /// [`Checker::call_of`], which every nested call passes through, so
    /// that its frame stays small.
    fn call_of_no_function(
        &mut self,
        id: ExprId,
        (callee, callee_ty): (ExprId, TyId),
        args: ArgRange,
    ) -> TyId {
        if callee_ty == self.never {
            for arg in self.ast.args(args) {
                self.check_alone(arg.value);
            }
            return self.never;
        }
        if callee_ty != TyId::ERROR {
            let callee_ty = self.settle(callee, callee_ty);
            let message = format!("cannot call a value of type {}", self.in_message(callee_ty));
            let label = format!("this is {}", self.in_label(callee_ty));
            let mut error = Diagnostic::error(message, self.ast.get(id).span)
                .with_context(self.ast.get(callee).span, label)
                .with_note("only functions can be called");
            if let Some(note) = self.union_note(callee, callee_ty) {
                error = error.with_note(note);
            }
            self.errors.push(error);
        }
        // An erroneous callee, its error reported where it is, would have
        // typed its arguments: what it would have typed is no error of
        // theirs (see [`Checker::check_list`]).
        let unknown = u32::from(callee_ty == TyId::ERROR);
        self.quiet.expecting_error = self.quiet.expecting_error.saturating_add(unknown);
        for arg in self.ast.args(args) {
            self.check_alone(arg.value);
        }
        self.quiet.expecting_error = self.quiet.expecting_error.saturating_sub(unknown);
        TyId::ERROR
    }

    /// Types the call `id` of `callee`, already checked, of type
    /// `callee_ty`, with the arguments `args`, as [`Checker::check_call`]
    /// does: a call of a built-in by its own rule, a call of anything else
    /// by its function type.
    pub(super) fn call_of(
        &mut self,
        id: ExprId,
        (callee, callee_ty, callback): (ExprId, TyId, Option<usize>),
        args: ArgRange,
    ) -> TyId {
        if let TyKind::Builtin(builtin) = *self.types.kind(callee_ty) {
            return self.check_builtin_call(id, (callee, builtin), args);
        }
        let callee_ty = self.measure(callee, callee_ty);
        let Some(sig) = self.types.fn_sig(callee_ty).cloned() else {
            return self.call_of_no_function(id, (callee, callee_ty), args);
        };
        let sig = match sig.generic {
            Some(generic) => match self.instantiate((id, callee), (args, None), &sig, generic) {
                Some(sig) => sig,
                None => return TyId::ERROR,
            },
            None => sig,
        };
        let erroneous = self.bind_arguments(id, (callee, callee_ty, callback), args, &sig);
        self.call_effect(id, (callee, callee_ty), &sig);
        if erroneous { TyId::ERROR } else { sig.ret }
    }

    /// Records that the call `id` of `callee`, of type `callee_ty` and
    /// signature `sig`, is impure if its callee is, and reports it in a pure
    /// function: "cannot call an impure function from a pure function".
    pub(super) fn call_effect(
        &mut self,
        id: ExprId,
        (callee, callee_ty): (ExprId, TyId),
        sig: &FnTy,
    ) {
        if sig.effect != Effect::Pure {
            self.impure_calls.insert(id);
        }
        if let Some(function) = self.functions.last()
            && function.effect == Effect::Pure
            && sig.effect != Effect::Pure
        {
            let (marker, function) = (function.marker, function.fn_id);
            let label = format!("this is {}", self.in_label(callee_ty));
            // A context's default, which has no marker of its own to change.
            if self.is_context_default(function) {
                self.errors.push(
                    Diagnostic::error(
                        "the default of a context must be pure",
                        self.ast.get(id).span,
                    )
                    .with_context(self.ast.get(callee).span, label)
                    .with_note(
                        "a read of a context is pure, so its default is too; bind an impure \
                         value with `with` instead",
                    ),
                );
                return;
            }
            self.errors.push(
                Diagnostic::error(
                    "cannot call an impure function from a pure function",
                    self.ast.get(id).span,
                )
                .with_context(self.ast.get(callee).span, label)
                .with_context(marker, PURE_CONTEXT)
                .with_note(
                    "a pure function, declared with `:`, may call only pure functions; declare \
                     it with `->` to make it impure",
                ),
            );
        }
    }

    /// Checks what the input declares, as it starts, and declares its
    /// top-level functions, its methods among them, and checks its methods'
    /// declarations.
    pub(super) fn declare_top_level(&mut self) {
        self.check_declarations();
        self.declare_top_fns();
        self.calls.earlier_methods = self.find_earlier_methods();
        self.check_methods();
    }

    /// Checks the body of the function expression `decl`, whose parameters'
    /// types are `fn_params[base..]`, against its return type `ret`, in a
    /// scope of its own. The scope starts with the callee binding, if the
    /// function has a `callee` name (with the function's type and span),
    /// then the parameters, each in the next slot, a callback parameter of
    /// an impure or memoised function recording its calls and uses (see
    /// [`Checker::callback_parameter`]); then the items are
    /// checked, and the body's value against `ret` as an annotated
    /// initialiser's is against its annotation, a record block against a
    /// record return type field by field, and a record literal the body
    /// results in against a record return type with defaults, for the
    /// defaults alone ([`Expecting::literal_only`]).
    pub(super) fn check_fn_body(
        &mut self,
        decl: &FnDecl,
        base: usize,
        (callee, fn_ty, fn_span): (Option<Name>, TyId, Span),
        ret: TyId,
    ) {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let outer_spreads = std::mem::take(&mut self.spreads);
        let nodes_start = self.nodes.len();
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        if let Some(name) = callee {
            let binding = Binding {
                name,
                slot: Slot::NONE,
                ty: fn_ty,
                declared: Some(fn_span),
                kind: BindingKind::Callee,
                function: None,
                callback: None,
            };
            self.record_decl(binding);
            self.bindings.push(binding);
        }
        let params = self.ast.params(decl.params);
        for (index, param) in params.iter().enumerate() {
            if let Some(first) = params[..index]
                .iter()
                .find(|first| first.name == param.name)
            {
                let text = self.ast.names().get(param.name);
                self.errors.push(
                    Diagnostic::error(format!("parameter `{text}` is declared twice"), param.span)
                        .with_context(first.span, "first declared here"),
                );
            }
            let ty = self.param_type_in_body(param, base + index);
            self.declare_binding(param.name, ty, Some(param.span), BindingKind::Variable);
            if decl.effect != Effect::Pure && ty != TyId::ERROR {
                let callback = self.callback_parameter(index, param.ty);
                if let Some(binding) = self.bindings.last_mut() {
                    binding.callback = callback;
                }
            }
        }
        let body = decl.body;
        let block = *self.ast.get(body);
        let context = decl
            .ret
            .map_or(block.span, |ret| self.ast.type_expr(ret).span);
        let why = RESULT_WHY;
        let ExprKind::Block { items, label } = block.kind else {
            // The parser only makes blocks bodies.
            self.errors.push(
                InternalError {
                    message: "a function body is not a block",
                    span: block.span,
                }
                .into(),
            );
            self.bindings.truncate(self.block_start);
            (self.block_start, self.fields_start, self.next_slot) = outer;
            self.spreads = outer_spreads;
            return;
        };
        // As for any record block, against the present member of the
        // either shape of a record type too.
        let record = self.types.strip(ret);
        if self.types.laziness(ret) != Laziness::Future
            && self.types.is_record(record)
            && self.is_record_body(items)
        {
            let expecting = Expecting {
                ty: record,
                context,
                why,
                close: Span::new(block.span.end.saturating_sub(1), block.span.end),
                fill: Some(body),
                reach: Reach::Whole,
            };
            let found = self.check_body(items, Some(expecting));
            self.expr_types[body.index()] = found;
        } else if let Some(found) = self.empty_body(body, ret, (context, why)) {
            self.expr_types[body.index()] = found;
        } else {
            let expecting = self.result_expecting(ret, items, (context, why), block.span);
            // A labelled body is a labelled block like any other, whose
            // `return`s give it its value, checked as ever where only a
            // record literal its result is is reached.
            let returns = expecting.filter(|expecting| expecting.reach == Reach::Whole);
            let label = label.map(|label| self.ast.label(label));
            if let Some(label) = label {
                self.enter_block(body, label, returns);
            }
            let mut found = self.check_body(items, expecting);
            if label.is_some() {
                found = self.leave_block(body, found, returns);
            }
            self.expr_types[body.index()] = found;
            self.expect(body, found, ret, context, why);
        }
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        self.nodes.truncate(nodes_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        self.spreads = outer_spreads;
    }

    /// What is expected of the result of a function's body whose items are
    /// `items`, spanning `span`, which is not a record block, against the
    /// return type `ret`, stated at `context`, labelled `why`: if it has a
    /// result, a list literal it results in is checked against a list
    /// return type or a union, as a record block is against a record one
    /// (see [`Checker::check_list_expected`]); and a record literal it
    /// results in against a record return type with defaults, for the
    /// defaults alone ([`Expecting::literal_only`]). `None` otherwise.
    pub(super) fn result_expecting<'w>(
        &self,
        ret: TyId,
        items: ItemRange,
        (context, why): (Span, &'w str),
        span: Span,
    ) -> Option<Expecting<'w>> {
        if !self.has_result(items) {
            None
        } else if self.expects_list(ret) {
            Some(Expecting::new(ret, (context, why), span))
        } else {
            self.defaulted_record(ret)
                .map(|record| Expecting::literal_only(record, (context, why), span))
        }
    }

    /// The type the body sees `param` as, its type `fn_params[at]`'s: a
    /// rest parameter is the list of what fills it.
    pub(super) fn param_type_in_body(&mut self, param: &Param, at: usize) -> TyId {
        let ty = self.fn_params.get(at).map_or(TyId::ERROR, |&(_, ty)| ty);
        if param.rest && ty != TyId::ERROR {
            self.types.list(ty)
        } else {
            ty
        }
    }

    /// Reports the second rest parameter among `params`, if there is one:
    /// "a function has one rest parameter at most".
    pub(super) fn check_rests(&mut self, params: &[Param]) {
        if let Some(second) = params.iter().filter(|param| param.rest).nth(1) {
            self.errors.push(Diagnostic::error(
                "a function has one rest parameter at most",
                second.span,
            ));
        }
    }
}
