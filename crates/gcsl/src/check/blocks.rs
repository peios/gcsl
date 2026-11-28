//! Checking blocks: a function's block parameter, the trailing block that
//! fills it, and the calls of it.
//!
//! # Blocks
//!
//! A function's last parameter may be a *block*, `body: block -> T` or
//! `body: block: T` (see the parser's
//! [Blocks](crate::parser::functions#blocks)), of the type
//! [`TyKind::Block`](crate::types::TyKind::Block): what a call of it gives,
//! written as a function's result is, so bare in an impure block it is the
//! either shape, and what its body may do. A block is no value. In its
//! function's body the parameter is a binding like any other, but a read
//! of it is only the callee of a call with no arguments, `body()`
//! (`Checker::block_use`): anything else, binding it, passing it, storing
//! it, returning it, is "a block can only be called", and a read from a
//! function or a `lazy` written in the body, which would copy it, is "a
//! block cannot be used inside a function". It is passed on by calling it
//! in another call's block, `inner(x) { body() }`. A call of it
//! (`Checker::check_block_call`) is of its result's type, and is impure if
//! the block is, so that in a pure function it is "cannot call an impure
//! block from a pure function".
//!
//! A call of a function whose last parameter is a block gives it as its
//! trailing block, `step("boot") { ... }`, and only so: a positional or a
//! named argument that would fill it is "a block is given after the call's
//! arguments". After a rest parameter, which takes every positional
//! argument from its place on, the trailing block still fills the block
//! (`Checker::block_after_rest`). The trailing block
//! (`Checker::bind_block_argument`) is checked where it is written, as the
//! block it is, against the block's
//! result as a function's body is against its return type: it is in its
//! caller's scope, so it reads the caller's variables, declares its own in
//! the caller's frame, and rebinds the caller's mutable variables with
//! `:=`, which a function expression there could not. Where the block's
//! result is `{}`, its last expression is a statement, as the others are
//! (`Checker::ends_with_statement`): a call of an impure function, whose
//! value is discarded, or a loop, which collects nothing, so that a block
//! that ends with a call or a loop needs no `{}` after it, and any other
//! value there is an unused value. A result written `= EXPR` is the
//! block's value still. The body of a pure block,
//! `block: T`, is pure code whatever its caller is: a call of an impure
//! function or an await in it is the usual error, labelled at the call
//! the block is given to.
//!
//! A block is a boundary for jumps, as a function's body is, since it runs
//! when its function calls it, any number of times: `return` in it is
//! "`return` cannot leave a block", and a `break`, `continue` or labelled
//! `return` for a loop or block outside it "`break` cannot leave a block".
//! Its own loops and labelled blocks are left as any are.
//!
//! What lowering needs is which trailing blocks fill a block parameter
//! ([`Resolution::is_block_arg`](super::Resolution::is_block_arg)) and
//! which calls are of a block
//! ([`Resolution::is_block_call`](super::Resolution::is_block_call)).

use crate::ast::{Arg, ArgKind, ArgRange, Effect, ExprId, ExprKind, FnId, ItemKind, ItemRange};
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::{FnTy, TyId};

use super::{AwaitSite, Binding, Checker, PURE_CONTEXT};

/// What the purity rule's errors say of a pure block's body, where they
/// say of a pure function "this function is pure".
const PURE_BLOCK_CONTEXT: &str = "this call's block is pure, declared `block: T`";

/// A trailing block that fills a block parameter, whose body is being
/// checked (see [Blocks](self#blocks)).
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockArg {
    /// How many functions the checker was inside where it is written.
    pub(super) depth: usize,
    /// How many loops and labelled blocks were open where it is written:
    /// those from here on are its own.
    pub(super) loops: usize,
    /// The callee of the call it is given to, if the block is pure: where
    /// the purity rule's errors point.
    pub(super) pure: Option<Span>,
    /// The callee of the call it is given to.
    pub(super) callee: Span,
}

impl Checker<'_> {
    /// Reports what the function `function`, whose parameters' types are
    /// the last of `fn_params`, may not be if its last parameter is a
    /// block (see [Blocks](self#blocks)): a pure
    /// function with an impure block, "a pure function cannot take an
    /// impure block"; a function a deferring attribute is on, "a deferring
    /// function cannot take a block"; and a def-type, "a def-type cannot
    /// take a block".
    pub(super) fn check_block_param(&mut self, function: FnId) {
        let decl = *self.ast.fn_decl(function);
        let effect = decl.effect;
        let Some(last) = self.ast.params(decl.params).last().map(|param| param.ty) else {
            return;
        };
        // Its own parameters are the last of those being resolved.
        let ty = self.fn_params.last().map_or(TyId::ERROR, |&(_, ty)| ty);
        let Some((_, block)) = self.types.block_sig(ty) else {
            return;
        };
        let span = self.ast.type_expr(last).span;
        if effect == Effect::Pure && block != Effect::Pure {
            self.errors.push(
                Diagnostic::error("a pure function cannot take an impure block", span).with_note(
                    "a pure function, declared with `:`, may call only a pure block: declare \
                     the block `block: T`, or the function with `->`",
                ),
            );
        }
        if self.lowering.deferred.contains(&function) {
            self.errors.push(
                Diagnostic::error("a deferring function cannot take a block", span).with_note(
                    "a host may run its calls later, as steps, when the scope its block runs \
                     in has ended",
                ),
            );
        }
        if self.def_types.fns.contains(&function) {
            self.errors.push(
                Diagnostic::error("a def-type cannot take a block", span).with_note(
                    "a def-type is invoked by a host, which gives its arguments by name; take \
                     the block in a function its definitions call",
                ),
            );
        }
    }

    /// Whether the last parameter of `sig` is a block and one before it a
    /// rest parameter (see [Blocks](self#blocks)): the trailing block then
    /// fills the block however many positional arguments the call has,
    /// since the rest parameter takes every one from its place on.
    pub(super) fn block_after_rest(&self, sig: &FnTy) -> bool {
        sig.rest.is_some()
            && sig
                .params
                .last()
                .is_some_and(|&(_, ty)| self.types.block_sig(ty).is_some())
    }

    /// The trailing block whose body is being checked, if the innermost
    /// function being checked is the one it is written in.
    pub(super) fn block_arg(&self) -> Option<BlockArg> {
        self.calls
            .block_args
            .last()
            .copied()
            .filter(|block| block.depth == self.functions.len())
    }

    /// Whether the read `id` of `binding`, number `index` among the
    /// bindings, written at `span`, is a misuse of a block parameter (see
    /// [Blocks](self#blocks)), reported here: a read from a function or a
    /// `lazy` inside the parameter's function, or one that is not the
    /// callee of a call.
    pub(super) fn block_misused(
        &mut self,
        id: ExprId,
        (index, binding): (usize, Binding),
        span: Span,
    ) -> bool {
        if self.types.block_sig(binding.ty).is_none() {
            return false;
        }
        let text = self.ast.names().get(binding.name).to_owned();
        let inner = self
            .functions
            .last()
            .filter(|function| index < function.bindings_start);
        if let Some(function) = inner {
            let lazy = self.ast.fn_decl(function.fn_id).ret.is_none();
            let (kind, when) = if lazy {
                ("a `lazy`", "awaited")
            } else {
                ("a function", "called")
            };
            let mut error =
                Diagnostic::error(format!("a block cannot be used inside {kind}"), span)
                    .with_context(function.keyword, format!("inside {kind} here"))
                    .with_note(format!(
                        "`{text}` runs in the scope of the call that gave it, which may have \
                         ended when the body of {kind} is {when}"
                    ));
            if let Some(declared) = binding.declared {
                error = error.with_context(declared, format!("`{text}` is a block"));
            }
            self.errors.push(error);
            return true;
        }
        if self.call_site == Some(id) {
            return false;
        }
        let mut error = Diagnostic::error("a block can only be called", span).with_note(format!(
            "a block is no value: call it, `{text}()`, or pass it on by calling it in another \
             call's block, `f(x) {{ {text}() }}`"
        ));
        if let Some(declared) = binding.declared {
            error = error.with_context(declared, format!("`{text}` is a block"));
        }
        self.errors.push(error);
        true
    }

    /// Types the call `id` of `callee`, a block, of the result `ret` and
    /// the effect `effect`, with the arguments `args` (see
    /// [Blocks](self#blocks)): a block takes none, "a block takes no
    /// arguments"; the call is of the block's result, impure if the block
    /// is, and recorded for lowering.
    pub(super) fn check_block_call(
        &mut self,
        id: ExprId,
        callee: ExprId,
        (ret, effect): (TyId, Effect),
        args: ArgRange,
    ) -> TyId {
        let callee_span = self.ast.get(callee).span;
        let arg_list = self.ast.args(args);
        if let Some(first) = arg_list.first() {
            let last = arg_list.last().unwrap_or(first);
            let span = Span::join(first.span(self.ast), last.span(self.ast));
            self.errors.push(
                Diagnostic::error("a block takes no arguments", span)
                    .with_context(callee_span, "this is a block")
                    .with_note("call it with empty parentheses"),
            );
            for arg in arg_list {
                self.check_alone(arg.value);
            }
            return TyId::ERROR;
        }
        self.lowering.block_calls.insert(id);
        if effect == Effect::Pure {
            return ret;
        }
        self.impure_calls.insert(id);
        if let Some(marker) = self.pure_marker() {
            let context = self.pure_context();
            self.errors.push(
                Diagnostic::error(
                    "cannot call an impure block from a pure function",
                    self.ast.get(id).span,
                )
                .with_context(
                    callee_span,
                    "this is an impure block, declared `block -> T`",
                )
                .with_context(marker, context)
                .with_note(
                    "a pure function, declared with `:`, may call only a pure block, declared \
                     `block: T`",
                ),
            );
            return TyId::ERROR;
        }
        ret
    }

    /// What the purity rule's errors say at [`Checker::pure_marker`]: of a
    /// pure block's body, that its block is pure, and otherwise that the
    /// function is.
    pub(super) fn pure_context(&self) -> &'static str {
        match self.block_arg() {
            Some(BlockArg { pure: Some(_), .. }) => PURE_BLOCK_CONTEXT,
            _ => PURE_CONTEXT,
        }
    }

    /// Binds `arg` of a call to its callee's block parameter, of the result
    /// `ret` and the effect `effect`, the callee written at `callee`,
    /// labelled `label` (see [Blocks](self#blocks)): an argument that is
    /// not the trailing block is reported, true; the trailing block is
    /// checked as the block's body, with `how` saying how an await at its
    /// top is reported.
    pub(super) fn bind_block_argument(
        &mut self,
        arg: &Arg,
        (ret, effect): (TyId, Effect),
        (callee, label): (Span, &str),
        how: (AwaitSite, Option<(FnId, usize)>),
    ) -> bool {
        if arg.kind != ArgKind::Trailing {
            self.errors.push(
                Diagnostic::error(
                    "a block is given after the call's arguments",
                    arg.span(self.ast),
                )
                .with_context(callee, label)
                .with_note("write it as the call's trailing block, as in `f(x) { ... }`"),
            );
            self.check_alone(arg.value);
            return true;
        }
        self.check_block_body(arg.value, (Some(ret), effect), (callee, label), how);
        false
    }

    /// Checks `value`, a trailing block that fills a block parameter of
    /// the effect `effect`, as the block's body (see [Blocks](self#blocks)),
    /// against the block's result `ret` if it is known, the callee written
    /// at `callee`, labelled `label`: jumps do not leave it, and a pure
    /// block's body is pure code. Its type.
    pub(super) fn check_block_body(
        &mut self,
        value: ExprId,
        (ret, effect): (Option<TyId>, Effect),
        (callee, label): (Span, &str),
        how: (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        // Checked already, to infer a call's type parameters.
        if self.calls.prechecked.contains_key(&value) {
            return match ret {
                Some(ret) => self.check_expr_expecting_at(value, ret, (callee, label), how),
                None => TyId::ERROR,
            };
        }
        // An impure block's body in a pure function is the call's error.
        let pure = (effect == Effect::Pure && self.pure_marker().is_none()).then_some(callee);
        self.calls.block_args.push(BlockArg {
            depth: self.functions.len(),
            loops: self.loops.len(),
            pure,
            callee,
        });
        let errors = self.errors.len();
        // A block whose result is `{}` ends as a statement list does.
        let statements = ret.and_then(|ret| self.ends_with_statement(value, ret));
        self.lowering.block_args.insert(value, statements.is_some());
        let found = match (ret, statements) {
            (Some(ret), Some(items)) => {
                self.calls.statements = Some(items);
                let found = self.check_expr(value);
                self.calls.statements = None;
                self.expect_at(value, found, ret, (callee, label), how);
                found
            }
            (Some(ret), None) => self.check_expr_expecting_at(value, ret, (callee, label), how),
            (None, _) => self.check_expr(value),
        };
        self.calls.block_args.pop();
        // The purity rule's errors name the function, written before a
        // block's body could be pure.
        if pure.is_some() {
            for error in self.errors.iter_mut().skip(errors) {
                for label in &mut error.labels {
                    if label.span == callee && label.text == PURE_CONTEXT {
                        PURE_BLOCK_CONTEXT.clone_into(&mut label.text);
                    }
                }
            }
        }
        found
    }

    /// The items of the trailing block `value`, which fills a block
    /// parameter of the result `ret`, if the block ends as a statement
    /// list does (see [Blocks](self#blocks)): if `ret`, once awaited, is
    /// `{}`, the block is no record block, and its last item is an
    /// expression, not a result written `= EXPR`.
    pub(super) fn ends_with_statement(&mut self, value: ExprId, ret: TyId) -> Option<ItemRange> {
        let ExprKind::Block { items, .. } = self.ast.get(value).kind else {
            return None;
        };
        if self.types.present(ret) != TyId::EMPTY || self.is_record_body(items) {
            return None;
        }
        let last = self.ast.items(items).last().copied()?;
        matches!(self.ast.item(last).kind, ItemKind::Expr(_)).then_some(items)
    }

    /// Whether `body` is the items of the trailing block being checked
    /// whose last expression is a statement, as
    /// [`Checker::ends_with_statement`] found: asked once, as its items
    /// are checked.
    pub(super) fn is_statements(&mut self, body: ItemRange) -> bool {
        self.calls
            .statements
            .take_if(|items| *items == body)
            .is_some()
    }
}
