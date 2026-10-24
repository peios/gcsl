//! Checking calls: binding arguments to parameters, filling defaults, and the
//! built-in functions whose typing is special.
//!
//! # Built-in functions
//!
//! The built-in functions are the members of the `builtins` module, a
//! module implemented in Rust (see [`crate::workspace`], "The standard
//! library"): `len`, `int`, `float`, `exec`, `panic`, `split`, `join`,
//! `starts_with`, `ends_with`, `trim`, `strip_prefix`, `strip_suffix`,
//! `lines`, `fields`, `compare_versions`, the file system's `fs_exists`,
//! `fs_read`, `fs_mkdir`, `fs_copy`, `fs_temp_dir` and `fs_write`,
//! `sort_versions`, `show` and `exec_live`, each a [`Builtin`],
//! with its name, its parameters' names (at most [`Builtin::MAX_PARAMS`],
//! two), its [`Effect`] (`exec`, `exec_live` and the file system's are
//! impure, every other one pure) and its [`Typing`]. The checker knows no name of one: a
//! file has them as it has any other module's members, by an import, or by
//! the prelude, which binds each of them but the file system's to its own
//! name, or by the standard `fs` module, whose members `exists`, `read`,
//! `mkdir`, `copy`, `temp_dir` and `write` are the file system's, bound so
//! too; the prelude is what every file and session input imports
//! implicitly, `#[shadowable]` and first of all in its view's order, but a
//! file marked `#![no_prelude]` (see [`crate::workspace`], "Shadowable
//! declarations"). A file that declares its own `len` (a field, a `def`, a
//! `let`, a `fn`, or a path below it, `len.x`)
//! shadows the prelude's, and loses the built-in there. The cell of each is
//! a slot of the frame the driver writes before anything runs, its value a
//! [`crate::Value::Builtin`], its type each one's, as the workspace's
//! context gives every cell's.
//!
//! An ORDINARY built-in ([`Typing::Ordinary`]) has a function type the type
//! table can hold, of any parameters, result and effect, which
//! [`Builtin::ty`] interns with the table in hand (its parameters' and its
//! result's names into the run's names): `int` (`fn(x: Float): Int`,
//! truncation toward zero), `float` (`fn(x: Int): Float`), `exec`
//! (`fn(argv: List<Revealed<String>>, options: {cwd: String, env: {*:
//! Revealed<String>}, interactive: Bool, panic: Bool, until: String}) ->
//! {status: Int, stderr: String, stdout: String}`, which runs a program as
//! its options say, and which the prelude's `exec` calls, taking the
//! options by name), `exec_live` (of `exec`'s type, which
//! runs a program as `exec` does, copying its output to standard error as
//! it runs), `panic` (`fn(message: String):
//! Never`, which ends the run with a runtime error of the program's own
//! wording), the nine on strings: `split` (`fn(s: String, sep:
//! String): List<String>`, the pieces between the separators; an empty
//! separator is a runtime error), `join` (`fn(parts: List<String>, sep:
//! String): String`), `starts_with` (`fn(s: String, prefix: String):
//! Bool`), `ends_with` (`fn(s: String, suffix: String): Bool`), `trim`
//! (`fn(s: String): String`), `strip_prefix` (`fn(s: String, prefix:
//! String): String`), `strip_suffix` (`fn(s: String, suffix: String):
//! String`), `lines` and `fields` (each `fn(s: String): List<String>`),
//! each as Rust's `str` does it, and `compare_versions` (`fn(a: String, b:
//! String): Int`, `-1`, `0` or `1` as `a` sorts before, equal to or after
//! `b` in version order) and `sort_versions` (`fn(vs: List<String>):
//! List<String>`, the list sorted in that order), and the six of the file
//! system, each impure and each taking only absolute paths: `fs_exists`
//! (`fn(p: String) -> Bool`), `fs_read` (`fn(p: String) -> String`),
//! `fs_mkdir` (`fn(p: String) -> {}`), `fs_copy` (`fn(from: String, to:
//! String) -> {}`), `fs_temp_dir` (`fn() -> String`) and `fs_write` (`fn(p:
//! String, contents: String) -> {}`), which a file reaches as `fs.exists`
//! and so on after `import fs` (see [`crate::eval`], "The built-in
//! functions" for all of them); a file's own `join` shadows
//! the prelude's as any declaration does, and so does `path`'s, in
//! `path.gcsl` and in a file importing it by member or `*`. Each is a
//! value of its type like any function: passed, stored, called through a
//! variable or a parameter, and a call of it is a call
//! like any other, typed by its type, binding its arguments by position or
//! by its parameters' names, awaiting a future argument as any call
//! does, lowered to a `Call` (see [`crate::eval`], which applies a built-in
//! it finds called). So a call of `exec`, of `exec_live` or of `fs.read`,
//! from a pure
//! function is "cannot call an impure function from a pure function", as
//! for any impure
//! function, and from a `lazy`, an impure or memoised function or the top
//! level it is fine. Like any function it has no equality: `int === int` is
//! "cannot compare functions".
//!
//! A call of `panic` is of type `Never`, its result type, as a `break` is:
//! `if c { panic("x") } else { 1 }` is an `Int`, an `if` whose branch
//! panics needs no `else`, and what follows a panicking statement is never
//! run (see [Never](crate::check::narrowing#never)). `panic` fits a parameter of type `fn(String):
//! Int`, since `Never` is a subtype of every result. `Never` is shown in its
//! type although no annotation can write it.
//!
//! A SPECIAL built-in ([`Typing::Special`]), `len` or `show`, is typed call
//! by call by the checker, since it is generic. `len(list)`'s argument is
//! measured at the top, its elements not at all, and must be a list ("`len`
//! takes a list, found T"), and the call is an `Int`. `show(value)`'s
//! argument may be of any type with no function in it anywhere, a field,
//! an element, a member of a union, an alias's expansion, at any depth, a
//! built-in function's value being one too: else "cannot show a function:
//! `show` renders data as source", at the argument, noting "the argument is
//! of type `{run: fn(): Int}`" (`Checker::check_show`); it is used as its
//! bits all the way down, as an interpolated expression is, so awaited
//! deeply if it may be a future or hold one, which a pure function cannot
//! do; and the call is a `String`, the value's GCSL source (see
//! [`crate::eval`], "The built-in functions"). A call of one binds its
//! arguments by position or by the table's names, with the errors of any
//! call's arguments (`Checker::bind_builtin_arguments`); its callee then
//! resolves to [`VarRef::Builtin`] and has the function type of this call,
//! and lowering makes the call the built-in's own node, `ListLen` or
//! `Show`, never reading the callee. Calling a built-in that is not pure
//! from a pure function is an error, as for any function.
//!
//! ## The alias rule, temporary
//!
//! TEMPORARY (alias rule): a special built-in has no type the table can hold, so until the
//! inference engine gives built-in functions real polymorphic types (see the [inference
//! limits](https://learn.peios.org/gcsl/language/inference-limits/)), its value has an opaque type
//! of its own, [`TyKind::Builtin`](crate::types::TyKind::Builtin), shown `builtin len` or `builtin
//! show`, which no type expression can write, so that no annotation, parameter type, field type or
//! result type accepts it. A reference whose type is it may stand in two places only
//! (`Checker::alias_site`): the callee of a call, which is then typed by the built-in's rule as
//! above, and the whole initialiser of a top-level field or a top-level `let` of one name, neither
//! annotated nor mutable (`len = builtins.len`, `let l = len`, `m = l`), which makes that name an
//! *alias* of the built-in, of the same opaque type. A member or `*` import of one is an alias of
//! the same kind, the cell it names being of that type. The type is the alias's identity: it is
//! what every boundary already carries, the type of a cell another file reads
//! ([`crate::workspace`]'s cell types), of a root committed to a directory's scope, of a session's
//! [`Scope`](super::Scope) binding, of an import's target, so a call through an alias is typed and
//! lowered as a call of the built-in in any file or session input. Any other reference to one, or
//! to a record holding one (`builtins` or the prelude read whole), is "`len` is a built-in function
//! and cannot be used as a value yet", with the note "a built-in function with a type for any list
//! cannot be written as a type yet; call it, or bind it to a name" (for `show`, "for any value";
//! `Checker::builtin_use`, the one place that says so): an argument, a list element, a field of an
//! inner record, a function's result, an operand of `==` or `===`, an `is`, an `as`, an ascription,
//! a `let` in a block, a `let mut`, a rebind, a branch, a dotted field, a callee in parentheses.
//! What replaces it: a polymorphic type for `len` (`fn<T>(list: List<T>): Int`) and for `show`
//! (`fn<T>(value: T): String`, `T` bounded to types without functions), with which each is a value
//! like `int`, its calls ordinary calls, and the opaque type, `alias_site`, `builtin_use` and the
//! special path of `Checker::check_call` go.

use crate::ast::{
    Arg, ArgKind, ArgRange, Effect, Expr, ExprId, ExprKind, FnId, TypeExprId, TypeExprKind,
};
use crate::diagnostic::Diagnostic;
use crate::mentions::Resolve;
use crate::names::Name;
use crate::source::Span;
use crate::types::{FnTy, Laziness, TyId};

use super::{
    AwaitSite, Bareness, BodyState, Bound, Builtin, CallSite, Callback, CallbackPosition, Checker,
    DEFAULTED, KnownCallee, KnownParams, PURE_CONTEXT, ParamDefault, Typing, VarRef, filled_by,
    position, unknown_argument, unknown_builtin_argument,
};

/// What the binding of a call's arguments knows of the call, once, for
/// each of its arguments (see [`Checker::bind_call`]).
struct Binding<'a> {
    /// The call.
    id: ExprId,
    /// Its callee.
    callee: ExprId,
    /// The callee's type.
    callee_ty: TyId,
    /// The callback the call's arguments are recorded for, if any.
    callback: Option<usize>,
    /// The callee, if it is known.
    known: Option<KnownCallee>,
    /// The number of the callee's first parameter the call binds.
    offset: usize,
    /// Whether the call is of the function it is in, which measures its
    /// arguments.
    recursive: bool,
    /// Where the callee is written.
    callee_span: Span,
    /// "the function is ...", the label of an error of an argument.
    function_label: &'a str,
    /// Where the call's entries of `filled` begin.
    base: usize,
    /// The parameters bound.
    sig: &'a FnTy,
    /// The call's arguments.
    args: ArgRange,
    /// The call's arguments, as the tree holds them.
    arg_list: &'a [Arg],
}

/// How the arguments of a call are written (see [`Checker::bind_call`]).
#[derive(Clone, Copy)]
struct ArgShape {
    /// The positional arguments before the trailing block: the trailing
    /// block, if any, comes after them all.
    positional: usize,
    /// Whether there is a trailing block.
    trailing: bool,
    /// Whether any argument is named.
    named: bool,
}

impl ArgShape {
    /// The shape of the arguments `args`.
    fn of(args: &[Arg]) -> Self {
        ArgShape {
            positional: args
                .iter()
                .filter(|arg| arg.kind == ArgKind::Positional)
                .count(),
            trailing: args.iter().any(|arg| arg.kind == ArgKind::Trailing),
            named: args
                .iter()
                .any(|arg| matches!(arg.kind, ArgKind::Named { .. })),
        }
    }
}

impl Checker<'_> {
    /// TEMPORARY (alias rule): types the call `id` of the built-in function `builtin`, whose typing
    /// is special, the callee `callee` (a reference to it or to an alias of it, already checked),
    /// with the arguments `args` (see the module documentation): the arguments are bound to the
    /// built-in's parameters as a function's are (see [`Checker::bind_builtin_arguments`]), and
    /// then typed as the built-in says, each built-in its own way. The callee's type is the
    /// function type the built-in has at this call, and it resolves to [`VarRef::Builtin`], so that
    /// lowering makes the call the built-in's own node without reading the callee. Calling a
    /// built-in that is not pure from a pure function is an error, as for any function. Replaced by
    /// an ordinary call when built-in functions get real polymorphic types (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    pub(super) fn check_builtin_call(
        &mut self,
        id: ExprId,
        (callee, builtin): (ExprId, Builtin),
        args: ArgRange,
    ) -> TyId {
        self.expr_vars[callee.index()] = VarRef::Builtin(builtin);
        let base = self.filled.len();
        let erroneous = self.bind_builtin_arguments(id, (callee, builtin), args);
        // Each parameter's argument, as bound.
        let bound: Vec<ExprId> = {
            let arg_list = self.ast.args(args);
            self.filled
                .get(base..)
                .unwrap_or_default()
                .iter()
                .filter_map(|&arg| arg_list.get(arg as usize).map(|arg| arg.value))
                .collect()
        };
        self.filled.truncate(base);
        let ty = match (builtin.typing(), bound.as_slice()) {
            (Typing::Special, &[list]) if builtin == Builtin::Len && !erroneous => {
                self.check_len(callee, list)
            }
            (Typing::Special, &[value]) if builtin == Builtin::Show && !erroneous => {
                self.check_show(callee, value)
            }
            _ => {
                // Every argument is checked, whatever goes wrong.
                for &arg in &bound {
                    self.check_alone(arg);
                }
                TyId::ERROR
            }
        };
        if builtin.effect() != Effect::Pure {
            self.impure_calls.insert(id);
        }
        if let Some(function) = self.functions.last()
            && function.effect == Effect::Pure
            && builtin.effect() != Effect::Pure
        {
            let marker = function.marker;
            self.errors.push(
                Diagnostic::error(
                    "cannot call an impure function from a pure function",
                    self.ast.get(id).span,
                )
                .with_context(
                    self.ast.get(callee).span,
                    format!("`{}` is an impure built-in function", builtin.name()),
                )
                .with_context(marker, PURE_CONTEXT),
            );
        }
        ty
    }

    /// TEMPORARY (alias rule): `len`'s rule, until its polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)). Types the call of `len`,
    /// the callee
    /// `callee`, whose argument is `list`: the argument is used as its bits at the top (see
    /// [`Checker::measure`]), its elements not at all, and must be a list, or a union of lists,
    /// "`len` takes a list, found T". The call is an `Int`, and the callee a pure function from
    /// that type to `Int`; `Error` if the argument is not a list or is erroneous.
    pub(super) fn check_len(&mut self, callee: ExprId, list: ExprId) -> TyId {
        let found = self.check_expr(list);
        let found = self.settle(list, found);
        let ty = self.measure(list, found);
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        // A list of type `Never` is never there to count.
        if ty == self.never {
            self.expr_types[callee.index()] =
                self.types
                    .fn_type([(None, ty)], TyId::INT, Builtin::Len.effect());
            return self.never;
        }
        if self.element_of(ty).is_none() {
            let shown = self.in_message(ty);
            let label = format!("this is {}", self.in_label(ty));
            let mut error = Diagnostic::error(
                format!("`{}` takes a list, found {shown}", Builtin::Len.name()),
                self.ast.get(list).span,
            )
            .with_label(label);
            if let Some(note) = self.union_note(list, ty) {
                error = error.with_note(note);
            }
            self.errors.push(error);
            return TyId::ERROR;
        }
        self.expr_types[callee.index()] =
            self.types
                .fn_type([(None, ty)], TyId::INT, Builtin::Len.effect());
        TyId::INT
    }

    /// TEMPORARY (alias rule): `show`'s rule, until its polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)). Types the call of `show`,
    /// the callee `callee`, whose argument is `value`: the argument may be of any type without a
    /// function in it, anywhere, a field, an element, a member of a union, an alias's expansion, at
    /// any depth ([`TypeTable::contains_fn`](crate::types::TypeTable::contains_fn), which a
    /// built-in function's type is too), else "cannot show a function: `show` renders data as
    /// source", at the argument, its type in a note; and it is used as its bits all the way down,
    /// as an interpolated expression is ([`Checker::measure_deep`]), so one that may be a future,
    /// or hold one, is awaited deeply, subject to the purity rule. The call is a `String`, and the
    /// callee a pure function from the argument's type, once awaited, to `String`; `Error` if the
    /// argument is erroneous or holds a function.
    pub(super) fn check_show(&mut self, callee: ExprId, value: ExprId) -> TyId {
        let found = self.check_expr(value);
        let found = self.settle(value, found);
        if found == TyId::ERROR {
            return TyId::ERROR;
        }
        // Before measuring, so that an argument with a function in it that
        // may also be a future in a pure function is one error, not two.
        if self.types.contains_fn(found) {
            let shown = self.in_message(found);
            self.errors.push(
                Diagnostic::error(
                    format!(
                        "cannot show a function: `{}` renders data as source",
                        Builtin::Show.name()
                    ),
                    self.ast.get(value).span,
                )
                .with_note(format!("the argument is of type {shown}")),
            );
            return TyId::ERROR;
        }
        let ty = self.measure_deep(value, found);
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        self.expr_types[callee.index()] =
            self.types
                .fn_type([(None, ty)], TyId::STRING, Builtin::Show.effect());
        // A value of type `Never` is never there to show.
        if ty == self.never {
            return self.never;
        }
        TyId::STRING
    }

    /// Binds the arguments `args` of the call `id` of the built-in function
    /// `builtin`, the callee `callee`, to its parameters, as
    /// [`Checker::bind_arguments`] binds a function's, by position or by
    /// the names its table gives, and records the argument filling each
    /// parameter above `filled`'s length on entry, which the caller
    /// truncates back. An argument that fills no parameter is checked alone
    /// here; the others are left to the caller. Reports every argument that
    /// cannot be bound and every parameter left unfilled with the errors a
    /// function's call reports, and returns whether there was any.
    pub(super) fn bind_builtin_arguments(
        &mut self,
        id: ExprId,
        (callee, builtin): (ExprId, Builtin),
        args: ArgRange,
    ) -> bool {
        let ast = self.ast;
        let call = ast.get(id);
        let callee_span = ast.get(callee).span;
        let label = format!("`{}` is a built-in function", builtin.name());
        let params = builtin.params();
        let arg_list = ast.args(args);
        let count = params.len();
        let positional = arg_list
            .iter()
            .filter(|arg| arg.kind == ArgKind::Positional)
            .count();
        let trailing = arg_list.iter().any(|arg| arg.kind == ArgKind::Trailing);
        let named = arg_list
            .iter()
            .any(|arg| matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional + usize::from(trailing);
        let mut erroneous = false;
        if found > count {
            self.report_extra_arguments((call, callee_span, &label), args, count);
            erroneous = true;
        }
        let base = self.filled.len();
        self.filled.resize(base + count, u32::MAX);
        let mut next_positional = 0;
        for (index, arg) in arg_list.iter().enumerate() {
            let target = match arg.kind {
                ArgKind::Positional => {
                    next_positional += 1;
                    Some(next_positional - 1).filter(|&param| param < count)
                }
                ArgKind::Spread { span } => {
                    self.misplaced_spread(span, (callee_span, &label), &mut erroneous)
                }
                ArgKind::Trailing => (positional < count).then(|| count - 1),
                ArgKind::Named { name, .. } => {
                    let text = ast.names().get(name);
                    let param = params.iter().position(|&param| param == text);
                    if param.is_none() {
                        let at = (*arg, (callee_span, label.as_str()));
                        self.errors
                            .push(unknown_builtin_argument(ast, arg_list, params, at));
                        erroneous = true;
                    }
                    param
                }
            };
            let Some(param) = target else {
                self.check_alone(arg.value);
                continue;
            };
            let first = self.filled.get(base + param).copied().unwrap_or(u32::MAX);
            if let Some(first) = arg_list.get(first as usize) {
                let name = params.get(param).and_then(|&text| ast.names().lookup(text));
                self.report_given_twice((param, name), first.span(ast), arg.span(ast));
                erroneous = true;
                self.check_alone(arg.value);
                continue;
            }
            if let Some(entry) = self.filled.get_mut(base + param) {
                *entry = position(index);
            }
        }
        let unfilled: Vec<&str> = params
            .iter()
            .zip(self.filled.get(base..).unwrap_or_default())
            .filter(|&(_, &arg)| arg == u32::MAX)
            .map(|(&text, _)| text)
            .collect();
        if !unfilled.is_empty() {
            erroneous = true;
            if !named && !trailing {
                let noun = if count == 1 { "argument" } else { "arguments" };
                self.errors.push(
                    Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                        .with_context(callee_span, label.as_str()),
                );
            } else {
                let close = call.close_span().unwrap_or(call.span);
                for text in unfilled {
                    self.errors.push(
                        Diagnostic::error(format!("missing argument `{text}`"), close)
                            .with_context(callee_span, label.as_str()),
                    );
                }
            }
        }
        if !erroneous {
            for (param, &arg) in self
                .filled
                .get(base..)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                if let Some(entry) = self.arg_params.get_mut(args.start() + arg as usize) {
                    *entry = position(param);
                }
            }
        }
        erroneous
    }

    /// TEMPORARY (alias rule): the type of the reference `id`, spanning `span`, of type `ty`: `ty`,
    /// unless it is the opaque type of a built-in function whose typing is special, or holds one (a
    /// record read whole with an alias of one among its fields), and `id` is not where the alias
    /// rule lets one stand ([`Checker::alias_site`]: the callee of a call, or the whole initialiser
    /// of a top-level field or `let` of one name). Then it is "`len` is a built-in function and
    /// cannot be used as a value yet", reported here, the one place the restriction is made, and
    /// `Error`. Replaced, with the opaque type, when built-in functions get real polymorphic types
    /// (see the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    pub(super) fn builtin_use(&mut self, id: ExprId, ty: TyId, span: Span) -> TyId {
        let Some(builtin) = self.types.special_builtin(ty) else {
            return ty;
        };
        let alias = self.alias_site == Some(id) && self.types.is_builtin(ty);
        if alias {
            return ty;
        }
        let name = builtin.name();
        // What the built-in's type is for, which no type can write yet.
        let any = if builtin == Builtin::Show {
            "value"
        } else {
            "list"
        };
        self.errors.push(
            Diagnostic::error(
                format!("`{name}` is a built-in function and cannot be used as a value yet"),
                span,
            )
            .with_note(format!(
                "a built-in function with a type for any {any} cannot be written as a type \
                 yet; call it, or bind it to a name"
            )),
        );
        TyId::ERROR
    }

    /// Binds the arguments `args` of the call `id` to the parameters of its
    /// callee, of function type `callee_ty` with signature `sig`: positional
    /// arguments fill parameters left to right, the trailing block the last,
    /// and named arguments the parameters of their names. Checks each argument
    /// against its parameter's type, or alone if it fills none, and records the
    /// parameter each fills. A known callee gives each parameter left unfilled
    /// that has a default its default, which is recorded for the call (see
    /// "Calls" under [Functions](crate::check::functions#functions)). Reports
    /// every argument that cannot be bound and every parameter left unfilled
    /// after that, and returns whether there was any.
    ///
    /// An argument awaited at the top because the parameter it fills is
    /// present by inference, as far as the callee is known, is reported in
    /// a pure caller in the call-site form (see
    /// [`Checker::argument_await_error`]). In a recursive call through the
    /// function's own name, an argument whose type is a provisional type of
    /// the function is measured. In a call of the callback parameter
    /// `callbacks[callback]`, if `callback` is `Some`, what each argument
    /// gives the position it fills is recorded (see
    /// [`Checker::record_argument`]). A mismatched argument filling a
    /// callback parameter of a known callee of this input that was not
    /// narrowed is labelled with what kept it so (see
    /// [`Checker::narrowing_label`]).
    pub(super) fn bind_arguments(
        &mut self,
        id: ExprId,
        (callee, callee_ty, callback): (ExprId, TyId, Option<usize>),
        args: ArgRange,
        sig: &FnTy,
    ) -> bool {
        let bound = Bound {
            known: self.known_callee(callee),
            offset: 0,
        };
        self.bind_call(id, (callee, callee_ty, callback), args, (sig, bound))
    }

    /// As [`Checker::bind_arguments`], the callee and the parameters `sig`
    /// is of, those from number `bound.offset` on, as `bound` says.
    pub(super) fn bind_call(
        &mut self,
        id: ExprId,
        (callee, callee_ty, callback): (ExprId, TyId, Option<usize>),
        args: ArgRange,
        (sig, bound): (&FnTy, Bound),
    ) -> bool {
        let function_label = format!("the function is {}", self.in_label(callee_ty));
        let site = Binding {
            id,
            callee,
            callee_ty,
            callback,
            known: bound.known,
            offset: bound.offset,
            recursive: self.is_recursive(callee),
            callee_span: self.ast.get(callee).span,
            function_label: &function_label,
            base: self.filled.len(),
            sig,
            args,
            arg_list: self.ast.args(args),
        };
        let shape = ArgShape::of(site.arg_list);
        let count = sig.params.len();
        let found = shape.positional + usize::from(shape.trailing);
        // A rest parameter takes every positional argument from its place.
        let extra = found > count && sig.rest.is_none();
        if extra {
            let call = self.ast.get(id);
            self.report_extra_arguments((call, site.callee_span, &function_label), args, count);
        }
        self.filled.resize(site.base + count, u32::MAX);
        let rest_base = self.rest_args.len();
        let (bound_badly, misplaced_spread) = self.bind_each(&site, shape);
        let erroneous = self.finish_call(
            &site,
            shape,
            rest_base,
            (misplaced_spread, extra | bound_badly),
        );
        self.filled.truncate(site.base);
        erroneous
    }

    /// Binds each of the arguments of the call `site` is to the parameter it
    /// fills, as [`Checker::bind_call`] says, the arguments being of the
    /// shape `shape`: whether any was bound badly, and whether one is a
    /// spread where there is no rest parameter, which is then the error to
    /// see, not the parameters it leaves unfilled. On the path of every
    /// nested call, it keeps few locals of its own.
    fn bind_each(&mut self, site: &Binding<'_>, shape: ArgShape) -> (bool, bool) {
        let sig = site.sig;
        let (count, rest) = (sig.params.len(), sig.rest.map(usize::from));
        let label = (site.callee_span, site.function_label);
        let (mut erroneous, mut misplaced_spread) = (false, false);
        // Where the rest parameter's arguments are written.
        let (mut next_positional, mut rest_run) = (0, None);
        for (index, arg) in site.arg_list.iter().enumerate() {
            let target = match arg.kind {
                ArgKind::Positional => {
                    next_positional += 1;
                    sig.positional(next_positional - 1)
                }
                ArgKind::Spread { span } if rest.is_none() => {
                    misplaced_spread = true;
                    self.misplaced_spread(span, label, &mut erroneous)
                }
                ArgKind::Spread { .. } => rest,
                ArgKind::Trailing => (shape.positional < count).then(|| count - 1),
                ArgKind::Named {
                    name,
                    span,
                    shorthand,
                } => {
                    let param = self.named_target(site, (name, span, shorthand));
                    erroneous |= param.is_none();
                    param
                }
            };
            erroneous |= match target {
                None => {
                    self.check_alone(arg.value);
                    false
                }
                Some(param) if Some(param) == rest => {
                    self.bind_rest_argument(site, (arg, index, param), &mut rest_run)
                }
                Some(param) => self.bind_one_argument(site, (arg, index, param)),
            };
        }
        (erroneous, misplaced_spread)
    }

    /// Ends the binding of the arguments, of the shape `shape`, of the call
    /// `site` is, as [`Checker::bind_call`] says: a rest parameter given
    /// nothing is the empty list, the defaults are given, the parameters
    /// left unfilled reported unless a spread is `misplaced_spread`, and
    /// what lowering needs recorded, the rest parameter's arguments from
    /// `rest_args[rest_base..]`, unless the call is `erroneous` already or
    /// leaves a parameter unfilled. Whether the call is in error.
    fn finish_call(
        &mut self,
        site: &Binding<'_>,
        shape: ArgShape,
        rest_base: usize,
        (misplaced_spread, erroneous): (bool, bool),
    ) -> bool {
        let sig = site.sig;
        let (base, count, rest) = (site.base, sig.params.len(), sig.rest.map(usize::from));
        if let Some(entry) = rest.and_then(|rest| self.filled.get_mut(base + rest))
            && *entry == u32::MAX
        {
            *entry = DEFAULTED;
        }
        let defaults = site
            .known
            .map(|known| self.fill_defaults(known, (base, site.offset), count));
        let found = shape.positional + usize::from(shape.trailing);
        let unfilled = !misplaced_spread
            && self.report_unfilled(
                base,
                (self.ast.get(site.id), site.callee_span, site.function_label),
                sig,
                (found, shape.named || rest.is_some(), shape.trailing),
            );
        let rest_args = self.rest_args.split_off(rest_base);
        let erroneous = erroneous || unfilled;
        if !erroneous {
            self.record_bound((site.id, site.args), (base, site.offset), defaults);
            self.record_rest((site.id, site.args), &rest_args, (rest, site.offset));
        }
        erroneous
    }

    /// Binds `arg`, number `index` of the arguments of the call `site` is,
    /// to its parameter number `param`, no rest one: marked filled in
    /// `filled[base..]` and checked against its type, measured if the call
    /// is recursive, and recorded for the callback, if any, as
    /// [`Checker::bind_call`] says. A parameter filled twice is reported:
    /// true.
    fn bind_one_argument(
        &mut self,
        site: &Binding<'_>,
        (arg, index, param): (&Arg, usize, usize),
    ) -> bool {
        let ast = self.ast;
        let first = self
            .filled
            .get(site.base + param)
            .copied()
            .unwrap_or(u32::MAX);
        if let Some(first) = site.arg_list.get(first as usize) {
            let name = site.sig.params.get(param).and_then(|&(name, _)| name);
            self.report_given_twice((param, name), first.span(ast), arg.span(ast));
            self.check_alone(arg.value);
            return true;
        }
        if let Some(entry) = self.filled.get_mut(site.base + param) {
            *entry = position(index);
        }
        let param_ty = site
            .sig
            .params
            .get(param)
            .map_or(TyId::ERROR, |&(_, ty)| ty);
        let context =
            self.argument_context(site.known, param + site.offset, (site.id, site.callee));
        let found = self.check_expr_expecting_at(
            arg.value,
            param_ty,
            (site.callee_span, site.function_label),
            context,
        );
        // A recursive call measures every argument that is a parameter
        // of the function, conservatively.
        if site.recursive && self.types.laziness(found) == Laziness::Provisional {
            self.claim_top(ast.get(arg.value).span, found);
        }
        if let Some(callback) = site.callback {
            self.record_argument(callback, param, arg.value, param_ty);
        }
        false
    }

    /// The parameter the named argument `name`, written at `span`,
    /// shorthand if `shorthand`, of the call `site` is, fills: the one of
    /// its name, reported unknown if there is none
    /// ([`Checker::report_unknown_argument`]).
    fn named_target(
        &mut self,
        site: &Binding<'_>,
        (name, span, shorthand): (Name, Span, bool),
    ) -> Option<usize> {
        let params = &site.sig.params;
        let param = params.iter().position(|&(param, _)| param == Some(name));
        if param.is_none() {
            let filled = filled_by(site.arg_list, params.len(), |name| {
                params.iter().position(|&(param, _)| param == Some(name))
            });
            self.report_unknown_argument(
                (name, span, shorthand),
                (site.callee, site.callee_ty),
                site.sig,
                &filled,
            );
        }
        param
    }

    /// Reports the spread argument written at `span` of a call of a
    /// function, labelled `function_label` at `callee_span`, that has no
    /// rest parameter for it to fill, making the call `erroneous`; `None`,
    /// the parameter it fills.
    pub(super) fn misplaced_spread(
        &mut self,
        span: Span,
        (callee_span, function_label): (Span, &str),
        erroneous: &mut bool,
    ) -> Option<usize> {
        *erroneous = true;
        self.errors.push(
            Diagnostic::error("a spread argument fills a rest parameter", span)
                .with_context(callee_span, function_label)
                .with_note("this function has no rest parameter, `...name: T`"),
        );
        None
    }

    /// Binds `arg`, number `index` of the arguments of the call `site` is,
    /// to its rest parameter, number `param` (see "Rest parameters" under
    /// [Functions](crate::check::functions#functions)): checked against its
    /// elements' type, or a spread against a list of them, the parameter
    /// marked filled in `filled[base..]` if it was not, and kept to record
    /// for lowering. A named argument is "a rest parameter is given by
    /// position", and one apart from the others given it, as `run` says, is
    /// reported ([`Checker::rest_argument`]): true for either.
    fn bind_rest_argument(
        &mut self,
        site: &Binding<'_>,
        (arg, index, param): (&Arg, usize, usize),
        run: &mut Option<(usize, usize)>,
    ) -> bool {
        let label = (site.callee_span, site.function_label);
        if matches!(arg.kind, ArgKind::Named { .. }) {
            self.errors.push(
                Diagnostic::error("a rest parameter is given by position", arg.span(self.ast))
                    .with_context(label.0, label.1)
                    .with_note("`...list` gives it a list's elements"),
            );
            self.check_alone(arg.value);
            return true;
        }
        let apart = self.rest_argument(arg, (index, run), label);
        if let Some(entry) = self.filled.get_mut(site.base + param)
            && *entry == u32::MAX
        {
            *entry = position(index);
        }
        let element = site
            .sig
            .params
            .get(param)
            .map_or(TyId::ERROR, |&(_, ty)| ty);
        let expected = if matches!(arg.kind, ArgKind::Spread { .. }) {
            self.types.list(element)
        } else {
            element
        };
        let context =
            self.argument_context(site.known, param + site.offset, (site.id, site.callee));
        self.check_expr_expecting_at(arg.value, expected, label, context);
        self.rest_args.push((index, param));
        apart
    }

    /// Records, for lowering, that the arguments `rest_args` of the call
    /// `id`, each by its index in `args` with its parameter, fill its
    /// callee's rest parameter, number `rest`, numbered from `offset`, and
    /// that the callee has one, if it does.
    fn record_rest(
        &mut self,
        (id, args): (ExprId, ArgRange),
        rest_args: &[(usize, usize)],
        (rest, offset): (Option<usize>, usize),
    ) {
        for &(index, param) in rest_args {
            if let Some(entry) = self.arg_params.get_mut(args.start() + index) {
                *entry = position(param + offset);
            }
        }
        if let Some(rest) = rest {
            self.lowering.call_rests.insert(id, position(rest + offset));
        }
    }

    /// Checks that `arg`, number `index` of a call's arguments, given to
    /// the rest parameter, is written beside the others given it, as
    /// `run`, the first and last of them so far, says, and widens `run` to
    /// it: "a rest parameter's arguments are written together" otherwise,
    /// returning true.
    fn rest_argument(
        &mut self,
        arg: &Arg,
        (index, run): (usize, &mut Option<(usize, usize)>),
        (callee_span, function_label): (Span, &str),
    ) -> bool {
        let apart = run.is_some_and(|(_, last)| last + 1 != index);
        *run = Some(run.map_or((index, index), |(first, _)| (first, index)));
        if apart {
            self.errors.push(
                Diagnostic::error(
                    "a rest parameter's arguments are written together",
                    arg.span(self.ast),
                )
                .with_context(callee_span, function_label)
                .with_note("so that they are evaluated in the order they are written"),
            );
        }
        apart
    }

    /// Records, for lowering, the parameter each of the arguments `args` of
    /// the call `id` fills, as `filled[base..]` shows, numbered from
    /// `offset`, and the defaults the call gives, `defaults`, if it gives
    /// any (see "Calls" under [Functions](crate::check::functions#functions)).
    pub(super) fn record_bound(
        &mut self,
        (id, args): (ExprId, ArgRange),
        (base, offset): (usize, usize),
        defaults: Option<Vec<(u32, ParamDefault)>>,
    ) {
        for (param, &arg) in self
            .filled
            .get(base..)
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if arg == DEFAULTED {
                continue;
            }
            if let Some(entry) = self.arg_params.get_mut(args.start() + arg as usize) {
                *entry = position(param + offset);
            }
        }
        if let Some(defaults) = defaults.filter(|defaults| !defaults.is_empty()) {
            self.lowering.call_defaults.insert(id, defaults.into());
        }
    }

    /// Gives each of the `count` parameters of `known`, a known callee, from
    /// number `offset` on, that `filled[base..]` shows no argument fills and
    /// that has a default its default (see "Calls" under
    /// [Functions](crate::check::functions#functions)): marks it [`DEFAULTED`]
    /// there, so that it is not unfilled, and returns each with its default, in
    /// parameter order (a default in error, which fails the check with its own
    /// error, left out).
    pub(super) fn fill_defaults(
        &mut self,
        known: KnownCallee,
        (base, offset): (usize, usize),
        count: usize,
    ) -> Vec<(u32, ParamDefault)> {
        let mut defaults = Vec::new();
        for param in 0..count {
            if self.filled.get(base + param) != Some(&u32::MAX)
                || !self.has_default(known, param + offset)
            {
                continue;
            }
            if let Some(entry) = self.filled.get_mut(base + param) {
                *entry = DEFAULTED;
            }
            if let Some(default) = self.default_of(known, param + offset) {
                defaults.push((position(param + offset), default));
            }
        }
        defaults
    }

    /// The function that `callee` calls, if it is known: `callee` is a
    /// plain name whose binding was initialised directly with a function
    /// expression of this input, or declared by an earlier session input
    /// (whose measured parameters and defaults the scope carries), or the
    /// path of a
    /// top-level function of this input named by a dotted path once its
    /// group is checked, as a plain function's binding is then.
    pub(super) fn known_callee(&self, callee: ExprId) -> Option<KnownCallee> {
        if let Some(&node) = self.imported_callees.get(&callee) {
            let known = self.workspace.and_then(|workspace| workspace.known(node));
            return known.map(|_| KnownCallee::Imported(node));
        }
        if let ExprKind::Field { .. } = self.ast.get(callee).kind {
            if let Some(node) = self.committed_callee(callee) {
                return Some(KnownCallee::Imported(node));
            }
            // A reference reads its node's cell: a slot of the top level's
            // frame, from top-level code or from inside a function.
            let slot = match self.expr_vars.get(callee.index()) {
                Some(&VarRef::Fn(slot)) => slot,
                Some(&VarRef::Local(slot)) if self.functions.is_empty() => slot,
                _ => return None,
            };
            let top = self.top_fn_in(slot)?;
            let known =
                top.leaf.is_some() && top.state == BodyState::Done && top.ready != usize::MAX;
            return known.then_some(KnownCallee::Current(top.function));
        }
        let ExprKind::Name(name) = self.ast.get(callee).kind else {
            return None;
        };
        let (index, binding) = self.lookup(name)?;
        match binding.function {
            Some(function) => Some(KnownCallee::Current(function)),
            None if index < self.scope_known.len() => Some(KnownCallee::Earlier(index)),
            None => None,
        }
    }

    /// The merged node of `callee`, if it is the path of a `def` named by a
    /// dotted path that another file of the directory chain declares, read
    /// through the root the chain committed (an access of the root's
    /// record, which knows no function), whose file is checked: a call of
    /// it is of a known callee, as a call of an imported `def` is, or of a
    /// `def` of one name of the chain, through its committed binding.
    pub(super) fn committed_callee(&self, callee: ExprId) -> Option<u32> {
        let workspace = self.workspace?;
        let (node, at) = workspace.scope.reference(self.ast, callee)?;
        if at != callee {
            return None;
        }
        let mut base = callee;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let (index, _) = self.lookup(name)?;
        if index >= self.top_start {
            return None;
        }
        let global = workspace.scope.merged(node)?;
        workspace.known(global).map(|_| global)
    }

    /// How an argument of the call `call` of `callee`, whose function is
    /// `known` if it is known, that fills its parameter number `param` is
    /// checked: how an await of it is reported in a pure caller (see
    /// [`Checker::argument_site`]), and, for a callee of this input, the
    /// callee's function and the parameter, for the label on a mismatch
    /// (see [`Checker::narrowing_label`]).
    pub(super) fn argument_context(
        &self,
        known: Option<KnownCallee>,
        param: usize,
        (call, callee): (ExprId, ExprId),
    ) -> (AwaitSite, Option<(FnId, usize)>) {
        match known {
            Some(known @ KnownCallee::Current(function)) => (
                self.argument_site(known, param, (call, callee)),
                Some((function, param)),
            ),
            Some(
                known @ (KnownCallee::Earlier(_)
                | KnownCallee::Imported(_)
                | KnownCallee::Method(..)),
            ) => (self.argument_site(known, param, (call, callee)), None),
            None => (AwaitSite::Implicit, None),
        }
    }

    /// How an await of an argument of the call `call` of `callee`, a known
    /// callee `known`, that fills its parameter number `param` is reported
    /// in a pure caller: [`AwaitSite::Argument`] if the parameter is present
    /// because the function's body measures it, with where the body first
    /// does and the parameter's name if the function is one of this
    /// input's; [`AwaitSite::Implicit`] otherwise.
    pub(super) fn argument_site(
        &self,
        known: KnownCallee,
        param: usize,
        (call, callee): (ExprId, ExprId),
    ) -> AwaitSite {
        let argument = |measured| {
            AwaitSite::Argument(CallSite {
                call,
                callee,
                measured,
            })
        };
        match known {
            KnownCallee::Current(function) => {
                let demand = self
                    .fn_resolutions
                    .get(function.index())
                    .and_then(|resolution| resolution.param_demands.get(param).copied().flatten());
                let decl = self.ast.fn_decl(function);
                let name = self.ast.params(decl.params).get(param);
                match (demand, name) {
                    (Some(span), Some(name)) => argument(Some((span, name.name))),
                    _ => AwaitSite::Implicit,
                }
            }
            KnownCallee::Earlier(_) | KnownCallee::Imported(_) | KnownCallee::Method(..) => {
                let measured = self
                    .known_params(known)
                    .is_some_and(|known| known.measured(param));
                if measured {
                    argument(None)
                } else {
                    AwaitSite::Implicit
                }
            }
        }
    }

    /// What a call of `known`, a known callee whose tree is gone, needs to
    /// know of its parameters ([`KnownParams`]): the scope's entry for a
    /// function of an earlier input, the workspace's for a `def` of another
    /// file; `None` for a function of this input, and if it is not known.
    pub(super) fn known_params(&self, known: KnownCallee) -> Option<&KnownParams> {
        match known {
            KnownCallee::Current(_) => None,
            KnownCallee::Earlier(index) => self.scope_known.get(index)?.as_deref(),
            KnownCallee::Imported(node) => self.workspace?.known(node),
            KnownCallee::Method(name, index) => {
                self.workspace?.method(name, index)?.known.as_deref()
            }
        }
    }

    /// Whether parameter number `param` of `known`, a known callee, has a
    /// default (see "Calls" under
    /// [Functions](crate::check::functions#functions)): as its function
    /// expression declares, for a function of this input, and as its
    /// [`KnownParams`] say for one whose tree is gone.
    pub(super) fn has_default(&self, known: KnownCallee, param: usize) -> bool {
        match known {
            KnownCallee::Current(function) => {
                let decl = self.ast.fn_decl(function);
                self.ast
                    .params(decl.params)
                    .get(param)
                    .is_some_and(|param| param.default.is_some())
            }
            KnownCallee::Earlier(_) | KnownCallee::Imported(_) | KnownCallee::Method(..) => {
                self.default_of(known, param).is_some()
            }
        }
    }

    /// The default of parameter number `param` of `known`, a known callee, if
    /// it has one that is known (see "Calls" under
    /// [Functions](crate::check::functions#functions)): a function of this
    /// input whose default is in error has none, and fails the check with that
    /// default's error.
    pub(super) fn default_of(&self, known: KnownCallee, param: usize) -> Option<ParamDefault> {
        match known {
            KnownCallee::Current(function) => self
                .fn_resolutions
                .get(function.index())?
                .defaults
                .get(param)?
                .clone(),
            KnownCallee::Earlier(_) | KnownCallee::Imported(_) | KnownCallee::Method(..) => {
                self.known_params(known)?.default(param).cloned()
            }
        }
    }

    /// Whether `callee`, through any parentheses, is the innermost
    /// function's own name for itself: whether the call is a recursive
    /// call through the callee binding.
    pub(super) fn is_recursive(&self, callee: ExprId) -> bool {
        let mut node = callee;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        match self.expr_vars.get(node.index()) {
            Some(VarRef::Callee) => true,
            // A call of a member of the group being checked is treated as
            // a recursive call, from whichever member.
            Some(&VarRef::Fn(slot)) => self
                .top_fn_in(slot)
                .is_some_and(|top| top.state == BodyState::Checking),
            _ => false,
        }
    }

    /// Reports that the call `call`, whose callee is at `callee_span` and
    /// labelled `function_label`, has more positional arguments, among
    /// `args`, than its callee's `count` parameters, labelling each one
    /// beyond them (the trailing block counting as the last).
    pub(super) fn report_extra_arguments(
        &mut self,
        (call, callee_span, function_label): (&Expr, Span, &str),
        args: ArgRange,
        count: usize,
    ) {
        let ast = self.ast;
        let positional = ast
            .args(args)
            .iter()
            .filter(|arg| !matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional.clone().count();
        let noun = if count == 1 { "argument" } else { "arguments" };
        let mut error =
            Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                .with_context(callee_span, function_label);
        for extra in positional.skip(count) {
            error = error.with_context(extra.span(ast), "unexpected argument");
        }
        self.errors.push(error);
    }

    /// Reports the argument written at `second` for parameter number
    /// `param`, named `name` if it has one, which the argument at `first`
    /// already fills.
    pub(super) fn report_given_twice(
        &mut self,
        (param, name): (usize, Option<Name>),
        first: Span,
        second: Span,
    ) {
        let shown = match name {
            Some(name) => format!("`{}`", self.ast.names().get(name)),
            None => (param + 1).to_string(),
        };
        self.errors.push(
            Diagnostic::error(format!("argument {shown} is given twice"), second)
                .with_context(first, "first given here"),
        );
    }

    /// Reports the named argument `name`, written at `span`, the shorthand
    /// `name` alone if `shorthand`, that no parameter of the callee, of type
    /// `callee_ty` with signature `sig`, is named: an unknown argument,
    /// suggesting the nearest of the named parameters that `filled` (by
    /// position) shows no argument fills ([`unknown_argument`]), or, if none
    /// of its parameters has a name, a function that takes none.
    pub(super) fn report_unknown_argument(
        &mut self,
        (name, span, shorthand): (Name, Span, bool),
        (callee, callee_ty): (ExprId, TyId),
        sig: &FnTy,
        filled: &[bool],
    ) {
        let callee_span = self.ast.get(callee).span;
        let shown = self.in_label(callee_ty);
        let error = if !sig.params.is_empty() && sig.params.iter().all(|(name, _)| name.is_none()) {
            Diagnostic::error("the parameters of this function have no names", span)
                .with_context(callee_span, format!("this is {shown}"))
                .with_note("call it with positional arguments")
        } else {
            let names = self.ast.names();
            let text = names.get(name);
            let free = sig
                .params
                .iter()
                .zip(filled)
                .filter(|&(_, &filled)| !filled)
                .filter_map(|(&(param, _), _)| Some((names.get(param?), 0)));
            unknown_argument((text, span, shorthand), free)
                .with_context(callee_span, format!("the function is {shown}"))
        };
        self.errors.push(error);
    }

    /// Reports the parameters of the call `call`'s callee, at
    /// `callee_span` and labelled `function_label`, with signature `sig`,
    /// that `filled[base..]` shows no argument fills. A call with only
    /// positional arguments, of which it has `found`, is short of arguments
    /// as a whole; one with named arguments or a trailing block is missing
    /// each unfilled parameter. Returns whether any was unfilled.
    pub(super) fn report_unfilled(
        &mut self,
        base: usize,
        (call, callee_span, function_label): (&Expr, Span, &str),
        sig: &FnTy,
        (found, named, trailing): (usize, bool, bool),
    ) -> bool {
        let filled = self.filled.get(base..).unwrap_or_default();
        if filled.iter().all(|&arg| arg != u32::MAX) {
            return false;
        }
        let count = sig.params.len();
        if !named && !trailing {
            let noun = if count == 1 { "argument" } else { "arguments" };
            self.errors.push(
                Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                    .with_context(callee_span, function_label),
            );
            return true;
        }
        let close = call.close_span().unwrap_or(call.span);
        let unfilled: Vec<usize> = filled
            .iter()
            .enumerate()
            .filter(|&(_, &arg)| arg == u32::MAX)
            .map(|(param, _)| param)
            .collect();
        for param in unfilled {
            let message = match sig.params.get(param) {
                Some(&(Some(name), _)) => {
                    format!("missing argument `{}`", self.ast.names().get(name))
                }
                Some(&(None, ty)) => {
                    format!(
                        "missing argument {} of type {}",
                        param + 1,
                        self.in_message(ty)
                    )
                }
                None => continue,
            };
            self.errors
                .push(Diagnostic::error(message, close).with_context(callee_span, function_label));
        }
        true
    }

    /// If the parameter number `param` of an impure or memoised function,
    /// whose type is written as `ty` and is not erroneous, is a callback
    /// parameter, one written directly as an impure function type, adds it
    /// to `callbacks`, with its positions, and returns its index there;
    /// `None` otherwise. A position written bare, as a name, a record type
    /// or a list type (see [`Checker::is_bare`]), is narrowable, to its
    /// present type: the type it is written as with every bare type in it
    /// present. See the module documentation.
    pub(super) fn callback_parameter(&mut self, param: usize, ty: TypeExprId) -> Option<usize> {
        let ast = self.ast;
        let TypeExprKind::Fn {
            params,
            effect: Effect::Impure,
            ..
        } = ast.type_expr(ty).kind
        else {
            return None;
        };
        let positions = self.callback_positions.len();
        for position in ast.param_tys(params) {
            // The type resolved without an error in its own mode, so it
            // resolves without one in this one too.
            let narrowable = self
                .is_bare(position.ty)
                .then(|| {
                    (
                        position.ty,
                        self.resolve_type(position.ty, Bareness::Present),
                    )
                })
                .filter(|&(_, present)| present != TyId::ERROR);
            self.callback_positions.push(CallbackPosition {
                narrowable,
                given: None,
            });
        }
        self.callbacks.push(Callback {
            param,
            used: None,
            positions,
            count: self.callback_positions.len() - positions,
        });
        Some(self.callbacks.len() - 1)
    }
}
