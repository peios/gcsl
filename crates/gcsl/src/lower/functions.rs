//! Lowering functions: references to them, closures and their codes, defaults,
//! `lazy`, def-type references and variants.

use crate::ast::{DefTypeId, Effect, ExprId, ExprKind, FnId};
use crate::check::{Fileroot, Slot, VarRef};
use crate::diagnostic::InternalError;
use crate::ir::{ArgValue, FnCode, FnKind, IrId, IrKind, IrRange};
use crate::names::Name;
use crate::source::Span;
use crate::types::FnTy;
use crate::value::ValueKind;

use super::{DEFINITION_CONTEXT, Lowerer, REFERENCE_DEFINITION, read};
use crate::check::{retry_of, timeout_of};

impl Lowerer<'_> {
    /// Lowers `id`, of kind `kind`, which is a variable use, a function
    /// expression, a call or a `lazy`. Apart from `lower_plain`, which is on
    /// the recursion path of every expression, so that its frame does not
    /// hold the temporaries of these cases.
    pub(super) fn lower_use_or_function(
        &mut self,
        id: ExprId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let expr = *self.ast.get(id);
        match (expr.kind, kind) {
            (ExprKind::Name(_), _) => {
                read(self.resolution.expr_var(id), kind).ok_or(InternalError {
                    message: "a variable use has no slot",
                    span: expr.span,
                })
            }
            (ExprKind::Fn(function), ValueKind::Fn) => self.lower_fn(id, function),
            (ExprKind::Lazy(function), ValueKind::Deferred) => self.lower_lazy(id, function),
            (ExprKind::Lazy(_), _) => Err(InternalError {
                message: "a `lazy` typed as something other than a future",
                span: expr.span,
            }),
            (ExprKind::Call { callee, args }, _) => match self.resolution.expr_var(callee) {
                VarRef::Builtin(builtin) => self.lower_builtin(id, builtin, args, kind),
                // `fileroot.helper()`, which takes no argument, is its own
                // node, never reading its callee; the calls after it pass on
                // the caller the body runs with.
                VarRef::Fileroot(Fileroot::Helper) if kind == ValueKind::Record => {
                    self.read_caller();
                    Ok(IrKind::Helper)
                }
                VarRef::Fileroot(_) => Err(InternalError {
                    message: "a call of a member of `fileroot` other than `helper`, or of a kind \
                              other than a record's",
                    span: expr.span,
                }),
                _ if self.resolution.method_receiver(id).is_some() => self.lower_method_call(id),
                _ => Ok(IrKind::Call {
                    callee: self.lower_operand(callee, ValueKind::Fn)?,
                    args: self.lower_args(id, args, expr.span)?,
                    kind,
                    // Set by `IrBuilder::measure` once every node is built.
                    depth: 0,
                }),
            },
            (ExprKind::Fn(_), _) => Err(InternalError {
                message: "a function expression typed as a non-function type",
                span: expr.span,
            }),
            _ => Err(InternalError {
                message: "an expression lowered as a variable, function or call is none",
                span: expr.span,
            }),
        }
    }

    /// Lowers the function expression `id`, whose details are `function`:
    /// its body into the function table, as code of [`FnKind::Memo`] if it
    /// is declared with `->>` and of [`FnKind::Function`] otherwise, and the
    /// expression to the `MakeClosure` that captures, in the current frame,
    /// what the checker found the function captures.
    pub(super) fn lower_fn(&mut self, id: ExprId, function: FnId) -> Result<IrKind, InternalError> {
        let kind = self.fn_kind(id, function);
        // A function's caller is its callers' file, whatever the frame that
        // makes it: what its body reads of the caller is no `lazy`'s.
        self.reads_caller.push(false);
        let captures = self.lower_function(id, function, kind);
        self.reads_caller.pop();
        let captures = self.with_default_fns(function, captures?)?;
        Ok(IrKind::MakeClosure { function, captures })
    }

    /// The items of the `MakeClosure` of function `function`, whose own
    /// captures are `captures`: those, then a `MakeClosure` of the default
    /// function of each of its parameters' defaults that is no constant, in
    /// parameter order, made here, in the frame making the function's
    /// closure (see the checker's "Calls"); `captures` itself if it has
    /// none.
    pub(super) fn with_default_fns(
        &mut self,
        function: FnId,
        captures: IrRange,
    ) -> Result<IrRange, InternalError> {
        let ast = self.ast;
        let defaults: Vec<FnId> = ast
            .params(ast.fn_decl(function).params)
            .iter()
            .filter_map(|param| param.default_fn)
            .collect();
        if defaults.is_empty() {
            return Ok(captures);
        }
        let mut items: Vec<IrId> = self.builder.items(captures).to_vec();
        for default in defaults {
            items.push(self.lower_default_fn(default)?);
        }
        Ok(self.builder.push_item_list(&items))
    }

    /// Lowers the default function `function` of a parameter (see the
    /// checker's "Calls"): its body, the default, into the function table,
    /// as code of [`FnKind::Function`], and itself to the `MakeClosure` that
    /// captures, in the current frame, what the checker found it captures.
    pub(super) fn lower_default_fn(&mut self, function: FnId) -> Result<IrId, InternalError> {
        let body_expr = self.ast.fn_decl(function).body;
        let span = self.ast.get(body_expr).span;
        self.reads_caller.push(false);
        // A default starts with no loop or labelled block around it.
        let outer = (
            std::mem::take(&mut self.loops),
            std::mem::take(&mut self.catches),
        );
        let body = self.lower_expr(body_expr);
        (self.loops, self.catches) = outer;
        let captures = body.and_then(|body| {
            // A default is called by its caller, whose needs it charges.
            let display = Box::from("default");
            self.enter_code(
                function,
                (FnKind::Function, body),
                (span, display, Box::default()),
            )
        });
        self.reads_caller.pop();
        let captures = captures?;
        Ok(self
            .builder
            .alloc(IrKind::MakeClosure { function, captures }, span))
    }

    /// Lowers a def-type reference of `def`, spanning `span`, whose path goes
    /// on past its definition, `package dev.peios.testpkg.versions` (see the
    /// checker's "Metas"), of kind `kind`: to its read, whose accesses past
    /// the definition the checker typed as reads of the meta of the
    /// definition's cell and fields of them, so that it makes no closure.
    pub(super) fn lower_reference_past(
        &mut self,
        def: DefTypeId,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let read = self.ast.def_type(def).read.ok_or(InternalError {
            message: "a def-type reference without a definition",
            span,
        })?;
        self.lower_operand(read, kind)
    }

    /// Lowers `id`, a variant as a value (see the checker's "Enums"): a
    /// constant tag.
    pub(super) fn lower_variant(&mut self, id: ExprId) -> Result<IrKind, InternalError> {
        let tag = self.resolution.variant(id).ok_or(InternalError {
            message: "a variant as a value the check did not resolve",
            span: self.ast.get(id).span,
        })?;
        Ok(IrKind::Tag {
            key: tag.key,
            of: tag.of,
            shown: self.builder.push_string(&tag.shown),
        })
    }

    /// The arguments with which the closure of the def-type reference `id`,
    /// of def-type `dt`, its own signature `sig`, calls the def-type
    /// ([`Lowerer::lower_def_type_ref`]): the definition's path, `name`, the
    /// variant the reference names, a constant tag, if the def-type's
    /// definitions are made with one, before the definition, `definition`,
    /// as the def-type's parameters are placed, then the closure's own
    /// parameters, the def-type's invocation parameters.
    pub(super) fn reference_args(
        &mut self,
        (id, dt): (ExprId, ExprId),
        sig: &FnTy,
        (name, definition): (IrId, IrId),
    ) -> Result<IrRange, InternalError> {
        let span = self.ast.get(id).span;
        let mistyped = InternalError {
            message: "a def-type reference without a def-type of a def-type's type",
            span,
        };
        let shape = self
            .table
            .def_type_shape(self.expr_types.get(dt))
            .ok_or(mistyped)?;
        let variant = match shape.variant {
            Some(_) => {
                let tag = self.resolution.variant(id).ok_or(InternalError {
                    message: "a def-type reference whose variant the check did not resolve",
                    span,
                })?;
                let kind = IrKind::Tag {
                    key: tag.key,
                    of: tag.of,
                    shown: self.builder.push_string(&tag.shown),
                };
                Some(self.builder.alloc(kind, span))
            }
            None => None,
        };
        let position = |at: usize| u32::try_from(at).unwrap_or(u32::MAX);
        let base = self.item_stack.len();
        self.push_arg(0, ArgValue::Node(name), span);
        if let (Some(at), Some(tag)) = (shape.variant, variant) {
            self.push_arg(position(at), ArgValue::Node(tag), span);
        }
        self.push_arg(position(shape.function), ArgValue::Node(definition), span);
        let invocation = position(shape.invocation());
        for (at, &(_, ty)) in (0u32..).zip(sig.params.iter()) {
            let kind = self.table.value_kind(ty).ok_or(mistyped)?;
            let value = self.builder.alloc(IrKind::Load(Slot::at(at), kind), span);
            self.push_arg(at.saturating_add(invocation), ArgValue::Node(value), span);
        }
        let args = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(args)
    }

    /// `call`, a def-type reference's call of its def-type, of kind `kind`,
    /// at `span`, in a `with` that binds the reference's definition, its
    /// capture, to the context of [`DEFINITION_CONTEXT`], so that every
    /// node the call makes knows the definition it is of.
    fn with_definition(
        &mut self,
        call: IrId,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let key = self
            .ast
            .names()
            .lookup(DEFINITION_CONTEXT)
            .ok_or(InternalError {
                message: "a def-type reference whose context the parser did not name",
                span,
            })?;
        let held = self.builder.alloc(
            IrKind::LoadCapture(REFERENCE_DEFINITION, ValueKind::Fn),
            span,
        );
        let bind = self
            .builder
            .alloc(IrKind::ContextBind { key, value: held }, span);
        let parts = self.builder.push_item_list(&[bind, call]);
        Ok(self.builder.alloc(IrKind::With { parts, kind }, span))
    }

    pub(super) fn lower_def_type_ref(
        &mut self,
        id: ExprId,
        def: DefTypeId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        let def = *self.ast.def_type(def);
        let mistyped = InternalError {
            message: "a def-type reference without a definition or a function type",
            span,
        };
        if kind != ValueKind::Fn {
            return Err(mistyped);
        }
        let read = def.read.ok_or(mistyped)?;
        let sig = self
            .table
            .fn_sig(self.expr_types.get(id))
            .cloned()
            .ok_or(mistyped)?;
        let def_type = self.lower_expr(def.dt)?;
        let definition = self.lower_expr(read)?;
        let base = self.item_stack.len();
        self.item_stack.push(def_type);
        self.item_stack.push(definition);
        let captures = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        // The body: the helper flag set, then the call of the def-type.
        let path = self
            .ast
            .show_definition_path(self.ast.path(def.path))
            .to_string();
        let path = self.builder.push_string(&path);
        let callee = self
            .builder
            .alloc(IrKind::LoadCapture(0, ValueKind::Fn), span);
        let name = self.builder.alloc(IrKind::StrConst(path), span);
        let definition = self.builder.alloc(
            IrKind::LoadCapture(REFERENCE_DEFINITION, ValueKind::Fn),
            span,
        );
        let args = self.reference_args((id, def.dt), &sig, (name, definition))?;
        let kind = self.table.value_kind(sig.ret).ok_or(mistyped)?;
        let call = self.builder.alloc(
            IrKind::Call {
                callee,
                args,
                kind,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        );
        let call = self.with_definition(call, kind, span)?;
        let helper = self.builder.alloc(IrKind::Helper, span);
        let items = self.builder.push_item_list(&[helper]);
        let body = self.builder.alloc(
            IrKind::Block {
                items,
                result: call,
            },
            span,
        );
        let params = u32::try_from(sig.params.len()).unwrap_or(u32::MAX);
        let function = FnId::lowered(u32::try_from(self.functions.len()).unwrap_or(u32::MAX));
        // A reference carries its definition's meta: its closure's is the
        // definition's, its second capture.
        if sig.meta.is_some() {
            self.meta_captures.push(function);
        }
        self.references.push(function);
        self.functions.push(Some(FnCode {
            body,
            kind: FnKind::Function,
            frame_size: params,
            param_count: params,
            capture_count: 2,
            // Set by `IrBuilder::measure` once every node is built.
            height: 0,
            name: None,
            display: self
                .table
                .display(self.expr_types.get(id), self.ast.names())
                .to_string()
                .into_boxed_str(),
            slot_names: sig
                .params
                .iter()
                .map(|&(name, _)| name)
                .collect::<Option<Box<[Name]>>>()
                .unwrap_or_default(),
            // A reference's function is no function item.
            attributes: Box::default(),
            defers: false,
            retry: 0,
            timeout: None,
            name_text: None,
            // It defers nothing, so is never a step.
            needs: Box::default(),
        }));
        Ok(IrKind::MakeClosure { function, captures })
    }

    /// The kind of the code of the function expression `id`, of function
    /// `function`: [`FnKind::Memo`] if it is declared with `->>`, and
    /// [`FnKind::Function`] otherwise. A definition made with a def-type
    /// declares nothing: its function type, its def-type's to give, says
    /// (see the checker's "Def-types").
    pub(super) fn fn_kind(&self, id: ExprId, function: FnId) -> FnKind {
        let effect = match self.ast.get(id).kind {
            ExprKind::Fn(_) if self.ast.fn_decl(function).ret.is_none() => self
                .table
                .fn_sig(self.expr_types.get(id))
                .map_or(Effect::Impure, |sig| sig.effect),
            _ => self.ast.fn_decl(function).effect,
        };
        match effect {
            Effect::Memo => FnKind::Memo,
            Effect::Pure | Effect::Impure => FnKind::Function,
        }
    }

    /// Lowers the `lazy` `id`, whose body is function `function`: the body
    /// into the function table, and the expression to the `MakeFuture` that
    /// captures, in the current frame, what the checker found it captures,
    /// marked if the body can read the caller it captures (see
    /// [`IrKind::MakeFuture`]), which marks the body around it too.
    pub(super) fn lower_lazy(
        &mut self,
        id: ExprId,
        function: FnId,
    ) -> Result<IrKind, InternalError> {
        self.reads_caller.push(false);
        let captures = self.lower_function(id, function, FnKind::Future);
        let reads_caller = self.reads_caller.pop().unwrap_or(false);
        if reads_caller {
            self.read_caller();
        }
        Ok(IrKind::MakeFuture {
            function,
            captures: captures?,
            reads_caller,
        })
    }

    /// Notes that the function or `lazy` body being lowered can read the
    /// caller it runs with (see [`Lowerer::reads_caller`]'s field).
    pub(super) fn read_caller(&mut self) {
        if let Some(reads) = self.reads_caller.last_mut() {
            *reads = true;
        }
    }

    /// Lowers the body of function `function`, of kind `kind`, the function
    /// expression or `lazy` `id`, into the function table, and returns the
    /// list of the loads, in the current frame, of what it captures. On the
    /// recursion path of nested functions and lazies, so the work after the
    /// body is in a helper whose temporaries this frame does not hold.
    pub(super) fn lower_function(
        &mut self,
        id: ExprId,
        function: FnId,
        kind: FnKind,
    ) -> Result<IrRange, InternalError> {
        // A function's body starts with no loop or labelled block around
        // it: no `break` leaves it, and no labelled `return`.
        let outer = (
            std::mem::take(&mut self.loops),
            std::mem::take(&mut self.catches),
        );
        let body = self.lower_expr(self.ast.fn_decl(function).body);
        (self.loops, self.catches) = outer;
        let body = body?;
        let body = if self.definitions.contains(&function) {
            self.as_definition(id, body)?
        } else {
            body
        };
        self.enter_function(id, function, (kind, body))
    }

    /// `body`, the body of the function expression `id`, a definition's
    /// value, in a `with` that binds the function being run, the
    /// definition, to the context of [`DEFINITION_CONTEXT`]: so that every
    /// node its body makes knows the definition it is of, wherever it is
    /// called from, a def-type's closure that runs it later say, as a
    /// reference's call binds it around the def-type's.
    fn as_definition(&mut self, id: ExprId, body: IrId) -> Result<IrId, InternalError> {
        let span = self.ast.get(id).span;
        let mistyped = InternalError {
            message: "a definition whose value has no function type",
            span,
        };
        let key = self
            .ast
            .names()
            .lookup(DEFINITION_CONTEXT)
            .ok_or(InternalError {
                message: "a definition whose context the parser did not name",
                span,
            })?;
        let ret = self
            .table
            .fn_sig(self.expr_types.get(id))
            .map(|sig| sig.ret)
            .ok_or(mistyped)?;
        let kind = self.table.value_kind(ret).ok_or(mistyped)?;
        let callee = self.builder.alloc(IrKind::LoadCallee, span);
        let bind = self
            .builder
            .alloc(IrKind::ContextBind { key, value: callee }, span);
        let parts = self.builder.push_item_list(&[bind, body]);
        Ok(self.builder.alloc(IrKind::With { parts, kind }, span))
    }

    /// Enters function `function`, of kind `kind`, the function expression
    /// or `lazy` `id`, whose body lowered to `body`, into the function
    /// table, and returns the list of the loads, in the current frame, of
    /// what it captures.
    pub(super) fn enter_function(
        &mut self,
        id: ExprId,
        function: FnId,
        (kind, body): (FnKind, IrId),
    ) -> Result<IrRange, InternalError> {
        let span = self.ast.get(id).span;
        let ty = self.expr_types.get(id);
        let display = self
            .table
            .display(ty, self.ast.names())
            .to_string()
            .into_boxed_str();
        let mut needs: Vec<Name> = self
            .table
            .needs(ty)
            .iter()
            .filter_map(|need| need.first().copied())
            .collect();
        needs.sort_unstable();
        needs.dedup();
        self.enter_code(function, (kind, body), (span, display, needs.into()))
    }

    /// Enters function `function`, of kind `kind`, whose body lowered to
    /// `body`, into the function table, shown as `display`, needing the
    /// contexts with no default `needs` ([`FnCode::needs`]), an error
    /// located at `span`, and returns the list of the loads, in the current
    /// frame, of what it captures.
    pub(super) fn enter_code(
        &mut self,
        function: FnId,
        (kind, body): (FnKind, IrId),
        (span, display, needs): (Span, Box<str>, Box<[Name]>),
    ) -> Result<IrRange, InternalError> {
        let resolution = self.resolution.function(function);
        let code = FnCode {
            body,
            kind,
            frame_size: resolution.frame_size,
            param_count: resolution.param_count,
            capture_count: u32::try_from(resolution.captures.len()).unwrap_or(u32::MAX),
            // Set by `IrBuilder::measure` once every node is built.
            height: 0,
            name: resolution.callee,
            display,
            slot_names: resolution.slot_names.clone().into_boxed_slice(),
            attributes: self.resolution.attributes(function).into(),
            defers: self.resolution.defers(function),
            retry: retry_of(self.resolution.attributes(function)),
            timeout: timeout_of(self.resolution.attributes(function)),
            name_text: resolution
                .callee
                .filter(|_| !self.resolution.attributes(function).is_empty())
                .map(|name| Box::from(self.ast.names().get(name))),
            needs,
        };
        let Some(entry) = self.functions.get_mut(function.index()) else {
            return Err(InternalError {
                message: "a function expression has no entry in the function table",
                span,
            });
        };
        *entry = Some(code);
        let base = self.item_stack.len();
        for &(var, ty) in &resolution.captures {
            let kind = self.table.value_kind(ty).ok_or(InternalError {
                message: "a captured value has a type with no values",
                span,
            })?;
            let read = read(var, kind).ok_or(InternalError {
                message: "a captured value is not a variable",
                span,
            })?;
            let load = self.builder.alloc(read, span);
            self.item_stack.push(load);
        }
        let captures = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(captures)
    }
}
