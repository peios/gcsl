//! Lowering calls: built-in functions, method calls, and the arguments of a
//! call.

use crate::ast::{ArgKind, ArgRange, ExprId, ExprKind};
use crate::check::{Builtin, ParamDefault};
use crate::diagnostic::InternalError;
use crate::ir::{ArgValue, IrId, IrKind, IrRange};
use crate::source::Span;
use crate::value::ValueKind;

use super::{Lowerer, read};

impl Lowerer<'_> {
    /// TEMPORARY (alias rule): lowers the call spanning `span` of the built-in function `builtin`,
    /// whose typing is special, with the arguments `args`, of kind `kind`, to the built-in's own
    /// node: `len` to a `ListLen` of its list, and `show` to a `Show` of its value, of any kind,
    /// awaited as the checker marked it. (With polymorphic types for them, see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/), a call of one is an
    /// ordinary call,
    /// or this stays as a specialisation of a known callee.)
    pub(super) fn lower_builtin(
        &mut self,
        builtin: Builtin,
        args: ArgRange,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        match builtin {
            Builtin::Len if kind == ValueKind::Int => {
                let list = self.builtin_argument(args, 0, span)?;
                Ok(IrKind::ListLen(self.lower_operand(list, ValueKind::List)?))
            }
            Builtin::Len => Err(InternalError {
                message: "a call of `len` typed as something other than an Int",
                span,
            }),
            Builtin::Show if kind == ValueKind::String => {
                let value = self.builtin_argument(args, 0, span)?;
                Ok(IrKind::Show(self.lower_expr(value)?))
            }
            Builtin::Show => Err(InternalError {
                message: "a call of `show` typed as something other than a String",
                span,
            }),
            // An ordinary built-in is called as any function value is.
            Builtin::Int
            | Builtin::Float
            | Builtin::Exec
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsTempDir
            | Builtin::FsWrite
            | Builtin::SortVersions
            | Builtin::HostArch
            | Builtin::HostOs
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue
            | Builtin::SecretReveal
            | Builtin::File
            | Builtin::FileRead
            | Builtin::Fetch
            | Builtin::ArchiveUnpack
            | Builtin::Which
            | Builtin::ExecSecret
            | Builtin::Dir
            | Builtin::DirFiles
            | Builtin::DirFile
            | Builtin::DirDir
            | Builtin::FsComposeDir
            | Builtin::DirSandbox
            | Builtin::SandboxExec
            | Builtin::SandboxFile
            | Builtin::SandboxDir
            | Builtin::Contains
            | Builtin::ListUnique
            | Builtin::ListContains => Err(InternalError {
                message: "a call of a built-in function of a function type resolved as special",
                span,
            }),
        }
    }

    /// The argument, among `args`, of the call spanning `span` of a
    /// built-in function, that the checker bound to its parameter number
    /// `param`.
    pub(super) fn builtin_argument(
        &self,
        args: ArgRange,
        param: u32,
        span: Span,
    ) -> Result<ExprId, InternalError> {
        self.ast
            .args(args)
            .iter()
            .zip(self.resolution.arg_params(args))
            .find(|&(_, &bound)| bound == param)
            .map(|(arg, _)| arg.value)
            .ok_or(InternalError {
                message: "a parameter of a built-in function is bound to no argument",
                span,
            })
    }

    /// Lowers the call `id` of a method (see the checker's "Methods"): a
    /// `Call` of a read of the method's binding, the variable the checker
    /// resolved its callee to, whose first argument is the receiver,
    /// evaluated before the call's own arguments.
    pub(super) fn lower_method_call(&mut self, id: ExprId) -> Result<IrKind, InternalError> {
        let expr = *self.ast.get(id);
        let (ExprKind::Call { callee, args }, Some(receiver)) =
            (expr.kind, self.resolution.method_receiver(id))
        else {
            return Err(InternalError {
                message: "a method call is no call",
                span: expr.span,
            });
        };
        let kind = self.kind(id)?;
        let read_kind =
            read(self.resolution.expr_var(callee), ValueKind::Fn).ok_or(InternalError {
                message: "a method's binding has no slot",
                span: expr.span,
            })?;
        let callee_node = self.builder.alloc(read_kind, self.ast.get(callee).span);
        let receiver_span = self.ast.get(receiver).span;
        let value = ArgValue::Node(self.lower_expr(receiver)?);
        let receiver_arg = self
            .builder
            .alloc(IrKind::Arg { param: 0, value }, receiver_span);
        let args = self.lower_args_after(id, args, (expr.span, Some(receiver_arg)))?;
        Ok(IrKind::Call {
            callee: callee_node,
            args,
            kind,
            // Set by `IrBuilder::measure` once every node is built.
            depth: 0,
        })
    }

    /// Lowers the arguments `args` of the call `id` spanning `span` to `Arg`
    /// nodes in source order, each with the parameter the checker bound it
    /// to, followed by one for each parameter the call gives its default
    /// ([`crate::check::Resolution::call_defaults`]), in parameter order,
    /// each the constant's nodes, at the call's closing parenthesis, and
    /// returns their list.
    pub(super) fn lower_args(
        &mut self,
        id: ExprId,
        args: ArgRange,
        span: Span,
    ) -> Result<IrRange, InternalError> {
        self.lower_args_after(id, args, (span, None))
    }

    /// As [`Lowerer::lower_args`], after `first`, a method's receiver's
    /// `Arg`, if the call is a method's.
    pub(super) fn lower_args_after(
        &mut self,
        id: ExprId,
        args: ArgRange,
        (span, first): (Span, Option<IrId>),
    ) -> Result<IrRange, InternalError> {
        let (ast, resolution) = (self.ast, self.resolution);
        let params = resolution.arg_params(args);
        let base = self.item_stack.len();
        self.item_stack.extend(first);
        let close = ast.get(id).close_span().unwrap_or(span);
        // The rest parameter's arguments make one list, given it where the
        // first of them is written, or after every other if none is.
        let rest = resolution.call_rest(id);
        let mut rest_items: Vec<IrId> = Vec::new();
        let (mut rest_at, mut rest_span, mut spreads) = (None, close, false);
        for (arg, &param) in ast.args(args).iter().zip(params) {
            if param == u32::MAX {
                return Err(InternalError {
                    message: "an argument of a call is bound to no parameter",
                    span,
                });
            }
            if Some(param) == rest {
                if rest_at.is_none() {
                    rest_at = Some(self.item_stack.len());
                    rest_span = arg.span(ast);
                } else {
                    rest_span = Span::join(rest_span, arg.span(ast));
                }
                let item = if let ArgKind::Spread { .. } = arg.kind {
                    spreads = true;
                    let list = self.lower_operand(arg.value, ValueKind::List)?;
                    self.builder.alloc(IrKind::Spread(list), arg.span(ast))
                } else {
                    self.lower_expr(arg.value)?
                };
                rest_items.push(item);
                continue;
            }
            let value = ArgValue::Node(self.lower_expr(arg.value)?);
            let node = self
                .builder
                .alloc(IrKind::Arg { param, value }, arg.span(ast));
            self.item_stack.push(node);
        }
        if let Some(param) = rest {
            let items = self.builder.push_item_list(&rest_items);
            let list = self
                .builder
                .alloc(IrKind::MakeList { items, spreads }, rest_span);
            let value = ArgValue::Node(list);
            let node = self.builder.alloc(IrKind::Arg { param, value }, rest_span);
            let at = rest_at.unwrap_or(self.item_stack.len());
            self.item_stack.insert(at, node);
        }
        for (param, default) in resolution.call_defaults(id) {
            let value = match default {
                ParamDefault::Constant(constant) => {
                    ArgValue::Node(self.lower_constant(constant, close))
                }
                &ParamDefault::Function { capture, .. } => {
                    let kind = self.param_kind(id, *param).ok_or(InternalError {
                        message: "a call's default is for a parameter its callee lacks",
                        span,
                    })?;
                    ArgValue::Default { capture, kind }
                }
            };
            self.push_arg(*param, value, close);
        }
        let list = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(list)
    }

    /// Pushes on the item stack an `Arg` of `value` for parameter number
    /// `param`, spanning `span`.
    pub(super) fn push_arg(&mut self, param: u32, value: ArgValue, span: Span) {
        let arg = self.builder.alloc(IrKind::Arg { param, value }, span);
        self.item_stack.push(arg);
    }

    /// The kind of the value of parameter number `param` of the callee of
    /// the call `call`, by the callee's function type; `None` if it has no
    /// such parameter.
    pub(super) fn param_kind(&self, call: ExprId, param: u32) -> Option<ValueKind> {
        let (ExprKind::Call { callee, .. } | ExprKind::Unclosed { callee, .. }) =
            self.ast.get(call).kind
        else {
            return None;
        };
        let callee_ty = self.table.strip(self.expr_types.get(callee));
        let sig = self.table.fn_sig(callee_ty)?;
        let &(_, ty) = sig.params.get(param as usize)?;
        self.table.value_kind(ty)
    }
}
