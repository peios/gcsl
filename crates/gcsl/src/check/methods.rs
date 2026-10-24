//! Checking generics and methods: instantiating a generic function, binding its
//! type parameters, and finding the method a call names.
//!
//! # Generics
//!
//! A generic function item (see the parser's [Functions and
//! calls](crate::parser::functions#functions-and-calls)) has a type parameter, as a
//! [`TyKind::Param`](crate::types::TyKind::Param) of its own [`GenericId`]
//! (`Checker::generic_of`), for each name written as a type in its scope that
//! is one of its parameters (`Checker::resolve_written`); no bareness wraps
//! one, and a pure function's parameter of it is not inferred, since what a
//! call binds it to may be lazy or not (`Checker::is_bare`). Its function type
//! is generic (`Checker::function_type`). Its body is checked once, for every
//! binding: a value of a type parameter can be passed, returned and stored in
//! records and lists, and every other use is the usual error of a type that
//! does not fit, as "cannot add T and Int"; interpolating one is "cannot
//! interpolate a value of type T, of a type parameter", since it may be a
//! future that nothing awaits.
//!
//! A call of a generic function infers (`Checker::instantiate`): each argument
//! that fills a parameter whose type mentions a type parameter, as the
//! arguments are bound ([`argument_targets`]), is checked with no type
//! expected, and binds what that type mentions
//! ([`TypeTable::bind`](crate::types::TypeTable::bind)), a parameter bound
//! twice the join of its bindings, which, widened to a union, is the warning
//! "the arguments for `T` have different types", as a list literal's elements
//! are; but `[]` and `{}`, whose types are what they are expected to be, bind
//! nothing, and an integer literal binds, as an `Int`, only what nothing else
//! binds or what it does not fit, so that another argument can make it a
//! `Float`. A type parameter left unbound is "cannot infer `T` for this call of
//! `f`", noted "`T` appears in no parameter's type given an argument here". The
//! call is then bound and checked as any is, with the signature its bindings
//! substitute ([`TypeTable::substitute`](crate::types::TypeTable::substitute)),
//! an argument checked already only expected to fit. A generic function read
//! other than as a call's callee, through any parentheses, is "a generic
//! function must be called" (`Checker::called_if_generic`): a value of it would
//! bind nothing.
//!
//! # Methods
//!
//! A method (see the parser's [Methods](crate::parser::functions#methods)) is a
//! top-level function of its input under a binding no source can write,
//! declared, grouped, checked, captured and lowered as a `fn` is (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)), its receiver its first parameter.
//! As the input starts, once its functions are declared, its methods are
//! checked (`Checker::check_methods`): one whose receiver type is a record
//! type with a field of its name is "method `m` has the name of a field of
//! its receiver type T", and one whose receiver type and an earlier one's
//! of its name each fit the other's, the same type, "method `m` is
//! declared twice for type T".
//!
//! A method is *visible* to an input if the input declares it; or, in a
//! session, an earlier input did: those are found among the bindings the
//! input inherits, by their names (`Checker::find_earlier_methods`); or, in
//! a workspace, it is a `def` method of another file the file sees (see
//! [`crate::workspace`], "Methods"), checked already, whose closure is
//! read from its slot as an imported cell is. A
//! call whose callee is a field access, `x.m(...)`, of a name some method
//! visible has (`Checker::check_field_call`), is a reference to a
//! top-level field or a member of `fileroot` if it is one, called as any
//! callee is. Otherwise its receiver `x` is checked first, measured, an
//! integer literal settled to `Int`; a method *applies* if the receiver's
//! type, stripped of its lazy layers, fits the method's receiver type,
//! stripped too, once each type parameter that type mentions is bound by
//! the receiver's as a call binds it (`Checker::method_applies`). Then:
//!
//! - a receiver type with a field `m` (or a meta, which has fields) and a
//!   method that applies is "`m` is both a field of T and a method";
//! - a receiver type with a field `m` is a call of the field, as without
//!   methods;
//! - a receiver type with neither is "no method `m` for type T", noting
//!   the receiver types of the visible methods of the name;
//! - a method of the standard library is set aside if any other applies,
//!   which shadows it as a declaration shadows a prelude's name, silently;
//! - of the methods that apply, the one whose receiver type fits every
//!   other's is called (`Checker::most_specific`): `List<Int>`'s over
//!   `List<T>`'s for a `List<Int>`; none such is "method `m` is ambiguous
//!   for type T", and several, whose receiver types are one type, "method
//!   `m` is declared twice for type T".
//!
//! The call of the method chosen (`Checker::call_method`) reads its binding as
//! its callee, as a call of a function by its name does, so a call of a method
//! of the input is of a known callee, which gives each parameter left out its
//! default, and of an earlier input's too. A generic method's type parameters
//! are bound by the receiver first and then the arguments
//! (`Checker::instantiate`); the receiver is expected to fit the receiver
//! parameter, its await reported as an argument's is; and the arguments are
//! bound to the parameters after it (`Checker::bind_call`), their positions
//! recorded from 1. The call is recorded for lowering with its receiver
//! ([`Resolution::method_receiver`](super::Resolution::method_receiver)), which
//! lowers it as a call of the binding with the receiver its first argument. A
//! method is never a value: its name alone is no variable, and `x.m` without a
//! call reads a field. An empty list, `[]`, is no receiver: its type is found
//! before any method is chosen, so it is "cannot infer the element type of
//! `[]`".

use std::collections::HashMap;

use crate::ast::{ArgRange, ExprId, ExprKind};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::strings::FieldName;
use crate::types::{FnTy, GenericId, Meta, TyId};
use crate::workspace::Context;

use super::{Bound, Checker, KnownCallee, MethodCandidate, MethodSource, TypeBinding, argument_targets};

impl Checker<'_> {
    /// Checks the input's methods as their bindings are declared (see
    /// [Methods](#methods)): one whose receiver type is a record type with a
    /// field of its name is "method `m` has the name of a field of its
    /// receiver type", and one whose receiver type is the same type as an
    /// earlier one's of its name, each fitting the other's, "method `m` is
    /// declared twice for type T", labelled "first declared here".
    pub(super) fn check_methods(&mut self) {
        let methods = self.ast.methods();
        for (index, method) in methods.iter().enumerate() {
            let Some((_, binding)) = self.lookup(method.binding) else {
                continue;
            };
            let receiver = self.receiver_of(binding.ty);
            if receiver == TyId::ERROR {
                continue;
            }
            if self.types.is_record(receiver) && self.types.field(receiver, method.name).is_some() {
                let text = FieldName(self.ast.names().get(method.name));
                let shown = self.in_message(receiver);
                self.errors.push(Diagnostic::error(
                    format!("method `{text}` has the name of a field of its receiver type {shown}"),
                    method.name_span,
                ));
            }
            for earlier in methods
                .iter()
                .take(index)
                .filter(|earlier| earlier.name == method.name)
            {
                let Some((_, earlier_binding)) = self.lookup(earlier.binding) else {
                    continue;
                };
                let earlier_receiver = self.receiver_of(earlier_binding.ty);
                if self.method_applies(receiver, earlier_binding.ty)
                    && self.method_applies(earlier_receiver, binding.ty)
                {
                    let text = self.ast.names().get(method.name);
                    let shown = self.in_message(receiver);
                    self.errors.push(
                        Diagnostic::error(
                            format!("method `{text}` is declared twice for type {shown}"),
                            method.name_span,
                        )
                        .with_context(earlier.name_span, "first declared here"),
                    );
                    break;
                }
            }
        }
    }

    /// Whether a method named `name` is visible to the input (see
    /// [Methods](#methods)).
    pub(super) fn sees_method(&self, name: Name) -> bool {
        self.ast.methods().iter().any(|method| method.name == name)
            || self.calls.earlier_methods.contains_key(&name)
            || self
                .workspace
                .is_some_and(|workspace| workspace.sees_method(name))
    }

    /// The methods named `name` visible to the input (see
    /// [Methods](#methods)): its own, whose bindings are declared, and an
    /// earlier session input's.
    pub(super) fn method_candidates(&self, name: Name) -> Vec<MethodCandidate> {
        let own = self
            .ast
            .methods()
            .iter()
            .filter(|method| method.name == name)
            .map(|method| (method.binding, Some(method.name_span)));
        let earlier = self
            .calls
            .earlier_methods
            .get(&name)
            .into_iter()
            .flatten()
            .map(|&binding| (binding, None));
        let mut candidates: Vec<MethodCandidate> = own
            .chain(earlier)
            .filter_map(|(binding_name, span)| {
                let (_, binding) = self.lookup(binding_name)?;
                Some(MethodCandidate {
                    source: MethodSource::Binding(binding_name),
                    ty: binding.ty,
                    declared: span.map(|span| (None, span)),
                })
            })
            .collect();
        if let Some(workspace) = self.workspace {
            for (index, method) in workspace.methods(name) {
                candidates.push(MethodCandidate {
                    source: MethodSource::Foreign(name, index),
                    ty: method.checked.ty,
                    declared: Some((Some(method.file), method.span)),
                });
            }
        }
        candidates
    }

    /// `error` with a label of `text` at where `candidate`, a method, is
    /// declared, if that is known, in its own input.
    pub(super) fn label_method(
        error: Diagnostic,
        candidate: &MethodCandidate,
        text: &str,
    ) -> Diagnostic {
        match candidate.declared {
            Some((None, span)) => error.with_context(span, text),
            Some((Some(input), span)) => error.with_context_in(input, span, text),
            None => error,
        }
    }

    /// The methods of the earlier inputs of a session the input inherits,
    /// by their names, each with its binding's (see [Methods](#methods)):
    /// its bindings before its own top-level ones are those it inherits.
    pub(super) fn find_earlier_methods(&self) -> HashMap<Name, Vec<Name>> {
        let names = self.ast.names();
        let mut found: HashMap<Name, Vec<Name>> = HashMap::new();
        for binding in self.bindings.iter().take(self.top_start) {
            let Some(method) = crate::names::binding_method(names.get(binding.name)) else {
                continue;
            };
            if let Some(name) = names.lookup(method) {
                found.entry(name).or_default().push(binding.name);
            }
        }
        found
    }

    /// Types the call `id` of `callee`, `receiver.name`, with the arguments
    /// `args`, a method of the name being visible (see [Methods](#methods)):
    /// a reference to a top-level field, or a member of `fileroot`, is
    /// called as any callee is; otherwise the receiver is checked, and the
    /// call is of the field of its type if it has one, or of the method of
    /// the name that applies to it, the most specific of those that do.
    pub(super) fn check_field_call(
        &mut self,
        id: ExprId,
        (callee, receiver, name): (ExprId, ExprId, Name),
        args: ArgRange,
    ) -> TyId {
        let span = self.ast.get(callee).span;
        let outer = self.alias_site.replace(callee);
        let outer_call = self.call_site.replace(callee);
        let imports = self.workspace.is_some_and(Context::has_view);
        let reference = if self.mentions.tree.nodes.is_empty() && !imports {
            None
        } else {
            self.field_reference(callee)
        };
        let early = match reference {
            Some(ty) => Some(self.builtin_use(callee, ty, span)),
            None => self.fileroot_lacks(callee, receiver, name),
        };
        (self.alias_site, self.call_site) = (outer, outer_call);
        if let Some(ty) = early {
            self.expr_types[callee.index()] = ty;
            return self.call_of(id, (callee, ty, None), args);
        }
        let found = self.check_expr(receiver);
        let found = self.measure(receiver, found);
        let found = if found == TyId::UNTYPED_INT {
            self.settle(receiver, found)
        } else {
            found
        };
        let candidates = if found == TyId::ERROR || found == self.never {
            Vec::new()
        } else {
            self.method_candidates(name)
        };
        let applicable: Vec<MethodCandidate> = candidates
            .iter()
            .copied()
            .filter(|candidate| self.method_applies(found, candidate.ty))
            .collect();
        let present = self.types.strip(found);
        let has_field = self
            .types
            .fn_sig(present)
            .and_then(|sig| sig.meta)
            .is_some()
            || self.field_of(present, name).is_some();
        if has_field && !applicable.is_empty() {
            let shown = self.in_message(found);
            let text = FieldName(self.ast.names().get(name));
            let at = self.ast.get(callee).member_span(self.ast).unwrap_or(span);
            let mut error = Diagnostic::error(
                format!("`{text}` is both a field of {shown} and a method"),
                at,
            );
            for candidate in &applicable {
                error = Self::label_method(error, candidate, "the method is declared here");
            }
            self.errors.push(error);
            return self.args_alone(args);
        }
        if has_field || candidates.is_empty() {
            let ty = self.field_access(callee, (receiver, found), name);
            self.expr_types[callee.index()] = ty;
            return self.call_of(id, (callee, ty, None), args);
        }
        if applicable.is_empty() {
            self.no_method(callee, (found, name), &candidates);
            return self.args_alone(args);
        }
        match self.most_specific(callee, (found, name), &applicable) {
            Some(method) => self.call_method(id, (callee, receiver, found), args, method),
            None => self.args_alone(args),
        }
    }

    /// Checks each of the arguments `args` alone, as a call that fails
    /// before binding them does, and returns `Error`.
    pub(super) fn args_alone(&mut self, args: ArgRange) -> TyId {
        for arg in self.ast.args(args) {
            self.check_alone(arg.value);
        }
        TyId::ERROR
    }

    /// Whether the method of type `method` applies to a receiver of type
    /// `found` (see [Methods](#methods)): `found`, its lazy layers stripped,
    /// fits the method's receiver type, stripped too, once each type
    /// parameter that type mentions is bound by `found`, as a call binds it
    /// ([`TypeTable::bind`](crate::types::TypeTable::bind)); one it leaves unbound, and so no fit.
    pub(super) fn method_applies(&mut self, found: TyId, method: TyId) -> bool {
        let Some(sig) = self.types.fn_sig(method).cloned() else {
            return false;
        };
        let Some(&(_, receiver)) = sig.params.first() else {
            return false;
        };
        let present = self.types.strip(found);
        let expected = match sig.generic {
            Some(generic) => {
                let count = self.types.generic_names(generic).len();
                let mut bound = vec![None; count];
                self.types.bind(receiver, present, generic, &mut bound);
                let mut args = Vec::with_capacity(count);
                for (index, binding) in (0u8..).zip(bound) {
                    args.push(binding.unwrap_or_else(|| self.types.param(generic, index)));
                }
                let substituted = self.types.substitute(receiver, generic, &args);
                if self.types.mentions(substituted, generic) {
                    return false;
                }
                substituted
            }
            None => receiver,
        };
        let expected = self.types.strip(expected);
        expected != TyId::ERROR && self.types.is_subtype(present, expected)
    }

    /// The method of `applicable`, those named `name` that apply to a
    /// receiver of type `found`, that a call through `callee` means (see
    /// [Methods](#methods)): the one whose receiver type fits every other's.
    /// None is "method `m` is ambiguous for type T", several, whose receiver
    /// types fit each other's and so are one type, "method `m` is declared
    /// twice for type T"; both reported, and then `None`.
    pub(super) fn most_specific(
        &mut self,
        callee: ExprId,
        (found, name): (TyId, Name),
        applicable: &[MethodCandidate],
    ) -> Option<MethodCandidate> {
        // A method of the standard library is shadowed by any other that
        // applies, as a prelude's name is by a declaration of its own.
        let standard = |candidate: &MethodCandidate| {
            matches!(candidate.declared, Some((Some(file), _))
                if self.workspace.is_some_and(|workspace| workspace.is_standard(file)))
        };
        let own: Vec<MethodCandidate> = applicable
            .iter()
            .filter(|candidate| !standard(candidate))
            .copied()
            .collect();
        let applicable = if own.is_empty() { applicable } else { &own[..] };
        let mut winners = Vec::new();
        for (index, candidate) in applicable.iter().enumerate() {
            let receiver = self.receiver_of(candidate.ty);
            let mut fits_all = true;
            for (other, method) in applicable.iter().enumerate() {
                if other != index && !self.method_applies(receiver, method.ty) {
                    fits_all = false;
                    break;
                }
            }
            if fits_all {
                winners.push(*candidate);
            }
        }
        if let [only] = winners[..] {
            return Some(only);
        }
        let shown = self.in_message(found);
        let text = self.ast.names().get(name);
        let callee_expr = self.ast.get(callee);
        let at = callee_expr
            .member_span(self.ast)
            .unwrap_or(callee_expr.span);
        let error = if winners.is_empty() {
            let mut error =
                Diagnostic::error(format!("method `{text}` is ambiguous for type {shown}"), at)
                    .with_note("no receiver type of the methods that apply fits all the others");
            for candidate in applicable {
                error = Self::label_method(error, candidate, "this applies");
            }
            error
        } else {
            let mut error = Diagnostic::error(
                format!("method `{text}` is declared twice for type {shown}"),
                at,
            );
            for candidate in &winners {
                error = Self::label_method(error, candidate, "declared here");
            }
            error
        };
        self.errors.push(error);
        None
    }

    /// The receiver type of the method of type `method`, its first
    /// parameter's, stripped; `Error` if it has none.
    pub(super) fn receiver_of(&self, method: TyId) -> TyId {
        self.types
            .fn_sig(method)
            .and_then(|sig| sig.params.first())
            .map_or(TyId::ERROR, |&(_, receiver)| self.types.strip(receiver))
    }

    /// Reports the call through `callee` of the method `name` on a receiver
    /// of type `found`, to which none of `candidates`, the visible methods
    /// of the name, applies: "no method `m` for type T", noting their
    /// receiver types.
    pub(super) fn no_method(
        &mut self,
        callee: ExprId,
        (found, name): (TyId, Name),
        candidates: &[MethodCandidate],
    ) {
        let shown = self.in_message(found);
        let text = self.ast.names().get(name);
        let callee_expr = self.ast.get(callee);
        let at = callee_expr
            .member_span(self.ast)
            .unwrap_or(callee_expr.span);
        let receivers: Vec<String> = candidates
            .iter()
            .map(|candidate| {
                let receiver = self.receiver_of(candidate.ty);
                self.in_message(receiver)
            })
            .collect();
        let note = match &receivers[..] {
            [one] => format!("the visible method `{text}` is for {one}"),
            many => format!("the visible methods `{text}` are for {}", many.join(", ")),
        };
        self.errors.push(
            Diagnostic::error(format!("no method `{text}` for type {shown}"), at).with_note(note),
        );
    }

    /// Types the call `id` of `callee`, `receiver.NAME`, the receiver of
    /// type `found`, as a call of `method` with the receiver its first
    /// argument and `args` the rest (see [Methods](#methods)): the callee
    /// read as the method's binding, a generic method's type parameters
    /// bound by the receiver and then the arguments, the receiver expected
    /// to fit its parameter, and the arguments bound to the parameters
    /// after it, defaults included, as a call by the function's own name.
    pub(super) fn call_method(
        &mut self,
        id: ExprId,
        (callee, receiver, found): (ExprId, ExprId, TyId),
        args: ArgRange,
        method: MethodCandidate,
    ) -> TyId {
        let span = self.ast.get(callee).span;
        let (method_ty, known) = match method.source {
            MethodSource::Binding(binding) => {
                let (method_ty, _) = self.use_name(callee, binding, span);
                let known =
                    self.lookup(binding)
                        .and_then(|(index, binding)| match binding.function {
                            Some(function) => Some(KnownCallee::Current(function)),
                            None => (index < self.scope_known.len())
                                .then_some(KnownCallee::Earlier(index)),
                        });
                (method_ty, known)
            }
            // Another file's, checked and run before this one.
            MethodSource::Foreign(name, index) => {
                let checked = self
                    .workspace
                    .and_then(|workspace| workspace.method(name, index));
                let Some(checked) = checked else {
                    return self.args_alone(args);
                };
                self.read_import(callee, checked.slot);
                let known = checked
                    .known
                    .is_some()
                    .then_some(KnownCallee::Method(name, index));
                (checked.ty, known)
            }
        };
        self.expr_types[callee.index()] = method_ty;
        let Some(sig) = self.types.fn_sig(method_ty).cloned() else {
            return self.args_alone(args);
        };
        let sig = match sig.generic {
            Some(generic) => {
                let receiver = Some((receiver, found));
                match self.instantiate((id, callee), (args, receiver), &sig, generic) {
                    Some(sig) => sig,
                    None => return TyId::ERROR,
                }
            }
            None => sig,
        };
        let label = format!("the method is {}", self.in_label(method_ty));
        let receiver_ty = sig.params.first().map_or(TyId::ERROR, |&(_, ty)| ty);
        let context = self.argument_context(known, 0, (id, callee));
        self.expect_at(receiver, found, receiver_ty, (span, &label), context);
        let rest = sig.after(1);
        let bound = Bound { known, offset: 1 };
        let erroneous = self.bind_call(id, (callee, method_ty, None), args, (&rest, bound));
        self.lowering.method_calls.insert(id, receiver);
        if let Some(record) = &mut self.record {
            record.methods.push((callee, method.declared));
        }
        self.call_effect(id, (callee, method_ty), &sig);
        if erroneous { TyId::ERROR } else { sig.ret }
    }

    /// The signature `sig` of the generic callee `callee` of the call `id`,
    /// whose type parameters belong to `generic`, with each bound as the
    /// arguments `args` bind it, after a method's `receiver`, checked
    /// already, with its type, if the call is a method's (see
    /// [Generics](#generics)): each argument
    /// that fills a parameter whose type mentions one is checked, with no
    /// expected type, and kept for its binding ([`CallsChecked`](super::CallsChecked)), and
    /// binds what that type mentions ([`Checker::bind_argument`]); an
    /// integer literal binds only once the others have, and only what they
    /// leave unbound or what it does not fit. A type parameter left unbound
    /// is "cannot infer `T` for this call of `f`", and then `None`, every
    /// argument checked.
    pub(super) fn instantiate(
        &mut self,
        (id, callee): (ExprId, ExprId),
        (args, receiver): (ArgRange, Option<(ExprId, TyId)>),
        sig: &FnTy,
        generic: GenericId,
    ) -> Option<FnTy> {
        let ast = self.ast;
        let mut binding = TypeBinding {
            generic,
            bound: vec![None; self.types.generic_names(generic).len()],
            binders: vec![None; self.types.generic_names(generic).len()],
        };
        // A method's receiver, checked already, binds first, as its first
        // argument.
        let offset = usize::from(receiver.is_some());
        if let Some((value, found)) = receiver
            && let Some(&(_, param_ty)) = sig.params.first()
        {
            let present = self.types.strip(found);
            self.bind_argument(&mut binding, (value, param_ty), present);
        }
        let after_receiver = sig.after(offset);
        let mut untyped: Vec<(ExprId, TyId)> = Vec::new();
        for (value, param, spread) in argument_targets(ast.args(args), &after_receiver) {
            let Some(&(_, param_ty)) = sig.params.get(param + offset) else {
                continue;
            };
            // A spread gives the rest parameter a list of its elements.
            let param_ty = if spread {
                self.types.list(param_ty)
            } else {
                param_ty
            };
            if !self.types.mentions(param_ty, generic) || self.expects_its_type(value) {
                continue;
            }
            let found = self.check_expr(value);
            self.calls.prechecked.insert(value, found);
            if found == TyId::UNTYPED_INT {
                untyped.push((value, param_ty));
            } else {
                self.bind_argument(&mut binding, (value, param_ty), found);
            }
        }
        // An integer literal binds last, so that another argument can make
        // it a `Float`.
        for (value, param_ty) in untyped {
            let mut trial = binding.bound.clone();
            self.types.bind(param_ty, TyId::INT, generic, &mut trial);
            let binds_new = binding
                .bound
                .iter()
                .zip(&trial)
                .any(|(was, is)| was.is_none() && is.is_some());
            let bindings: Vec<TyId> = binding
                .bound
                .iter()
                .map(|ty| ty.unwrap_or(TyId::ERROR))
                .collect();
            let expected = self.types.substitute(param_ty, generic, &bindings);
            if binds_new || !self.untyped_fits(expected) {
                let found = self.settle(value, TyId::UNTYPED_INT);
                self.calls.prechecked.insert(value, found);
                self.bind_argument(&mut binding, (value, param_ty), found);
            }
        }
        if let Some(unbound) = binding.bound.iter().position(Option::is_none) {
            self.not_inferred((id, callee), args, (generic, unbound));
            return None;
        }
        let bindings: Vec<TyId> = binding.bound.into_iter().flatten().collect();
        let params = sig
            .params
            .iter()
            .map(|&(name, param)| (name, self.types.substitute(param, generic, &bindings)))
            .collect();
        let ret = self.types.substitute(sig.ret, generic, &bindings);
        let meta = sig.meta.map(|meta| Meta {
            ty: self.types.substitute(meta.ty, generic, &bindings),
            ..meta
        });
        Some(FnTy {
            params,
            ret,
            effect: sig.effect,
            meta,
            generic: None,
            rest: sig.rest,
            needs: sig.needs,
        })
    }

    /// Binds what `param_ty`, the type of the parameter that `value` fills,
    /// mentions of `binding`'s type parameters from `found`, `value`'s type
    /// ([`TypeTable::bind`](crate::types::TypeTable::bind)), noting `value` as
    /// the binder of each it binds first. One whose binding this widens to a
    /// union is the warning "the arguments for `T` have different types", as
    /// the elements of a list literal are.
    pub(super) fn bind_argument(
        &mut self,
        binding: &mut TypeBinding,
        (value, param_ty): (ExprId, TyId),
        found: TyId,
    ) {
        let before = binding.bound.clone();
        let widened = self
            .types
            .bind(param_ty, found, binding.generic, &mut binding.bound);
        for ((binder, was), is) in binding.binders.iter_mut().zip(&before).zip(&binding.bound) {
            if was.is_none() && is.is_some() {
                *binder = Some(value);
            }
        }
        let Some(index) = widened else {
            return;
        };
        let at = usize::from(index);
        let name = self
            .types
            .generic_names(binding.generic)
            .get(at)
            .map_or("?", |&name| self.ast.names().get(name))
            .to_owned();
        let (Some(Some(so_far)), Some(Some(joined))) = (before.get(at), binding.bound.get(at))
        else {
            return;
        };
        let (so_far, joined) = (*so_far, *joined);
        let mut warning = Diagnostic::warning(
            format!("the arguments for `{name}` have different types"),
            self.ast.get(value).span,
        )
        .with_label(format!("this is {}", self.in_label(found)))
        .with_note(format!(
            "`{name}` is {} for this call; ascribe an argument that type, as in `(x : {})`, if \
             both are meant",
            self.in_message(joined),
            self.in_label(joined)
        ));
        if let Some(Some(binder)) = binding.binders.get(at) {
            let context = format!("`{name}` is {} by this", self.in_label(so_far));
            warning = warning.with_context(self.ast.get(*binder).span, context);
        }
        self.warnings.push(warning);
    }

    /// Reports that the type parameter number `unbound` of `generic` is
    /// bound by no argument of the call `id` of `callee`, the arguments
    /// `args`: "cannot infer `T` for this call of `f`"; and checks each
    /// argument not checked to bind another.
    pub(super) fn not_inferred(
        &mut self,
        (id, callee): (ExprId, ExprId),
        args: ArgRange,
        (generic, unbound): (GenericId, usize),
    ) {
        let ast = self.ast;
        let name = self
            .types
            .generic_names(generic)
            .get(unbound)
            .map_or("?", |&name| ast.names().get(name));
        let callee_text = match ast.get(callee).kind {
            ExprKind::Name(callee) => format!(" of `{}`", ast.names().get(callee)),
            _ => String::new(),
        };
        self.errors.push(
            Diagnostic::error(
                format!("cannot infer `{name}` for this call{callee_text}"),
                ast.get(id).span,
            )
            .with_note(format!(
                "`{name}` appears in no parameter's type given an argument here"
            )),
        );
        for arg in ast.args(args) {
            if self.calls.prechecked.remove(&arg.value).is_none() {
                self.check_alone(arg.value);
            }
        }
    }

    /// Whether `value`, through any parentheses, is a literal whose type is
    /// what it is expected to be, `[]` or `{}`: an argument that binds no
    /// type parameter, but is checked against what the others bind.
    pub(super) fn expects_its_type(&self, value: ExprId) -> bool {
        let mut node = value;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::List(elements) => return self.ast.elements(elements).is_empty(),
                ExprKind::Block { items, label: None } => return self.ast.items(items).is_empty(),
                _ => return false,
            }
        }
    }
}
