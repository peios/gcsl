//! Resolving type expressions: names, records, unions, lists, applied types and
//! function types, as written, into the type table.
//!
//! # Type declarations
//!
//! `type NAME = TYPE`, an item of a file's top level, declares a type in the
//! file's namespace of types, which is not its variables': a type and a field,
//! `def`, `let` or `fn` of one name coexist, a type's name being read only in a
//! type position. Types are order-free, public as `def`s are, and importable;
//! which file's declaration a name or a qualified name names is the workspace's
//! to say (see [`crate::workspace`], "Types"), and it is said before any file
//! is checked: the workspace *expands* every file's types first
//! ([`Ast::expand_types`](crate::ast::Ast::expand_types)), so that each name of
//! a declared type, wherever it is written (an annotation, an ascription, a
//! parameter, a function type, another `type`), is what the type's definition
//! is, as if the definition were written there, and each record type with
//! spreads is the record of its merged fields. The checker resolves the
//! expanded types with the rules above, unchanged, and knows no type's name: so
//! bareness applies where an alias is used, as if its definition were written
//! there (`type N = Int` used as an impure function's bare parameter is `Int |
//! Future<Int>` there, and used as a pure one's is inferred), a union with an
//! alias of `Future<Int>` among its members is a union with a `Future` member,
//! an alias of a function type is a callback parameter, and an impure
//! function's result written as an alias of a record type is inferred field by
//! field.
//!
//! A spread in a record type, `{...PackageDef, name: String}`, merges as
//! value spread does (see [Spreads and soft
//! assignment](crate::check::records#spreads-and-soft-assignment)): the spread's fields are soft,
//! a later field of one's name overrides it (its type may differ), a later
//! spread overrides a soft one, a field written twice is "field `x` appears
//! twice in this type", and a spread's field of a field written before it
//! is "field `x` is defined twice", at the type spread, labelled "`x` is a
//! field of this, of type T" and "first defined here", with the note "a
//! spread cannot replace a field defined before it; put the spread first"
//! (`Checker::spread_problems`). Only a record type may be spread, written
//! so or named: anything else is "cannot spread type T: it is not a record
//! type", at it, `T` shown as what it stands for. A rest-field, `*: T`,
//! merges as a field would: a spread's is soft, a second written one is
//! "the rest-field appears twice in this type", and a spread's after a
//! written one is "the rest-field is defined twice", labelled "this has a
//! rest-field, of type T".
//!
//! What is wrong in a declaration's definition is reported once, where it is
//! written: the workspace checks every file's `type` items before any file is
//! checked ([`check_types`](super::check_types), `Checker::type_items`), since
//! a file may use a type of a file checked after it; there a name of a built-in
//! type is "`Int` is a built-in type", and a second `type` of one name in one
//! file "type `Foo` is declared twice", labelled "first declared here". A copy
//! of a definition in another file, or in its own (a use of the type), is
//! resolved quietly (`Checker::resolve_quietly`), once per mode however many
//! uses share it: if anything in it is wrong it is `Error`, reported nowhere
//! here, and the check fails *quietly*, with no error of its own (a file's
//! `type` items are resolved quietly too, in its own check). So a type whose
//! definition has an error is erroneous wherever it is used, and reports once.
//! A name that names nothing it can (an unknown qualified name, a module's
//! name, a declared type applied to a type argument, a type nested too deeply
//! once expanded) is reported where it is written, from the problems the
//! expansion kept ([`Ast::type_problems`](crate::ast::Ast::type_problems)).
//! Checked alone, as [`check`](super::check) checks a file, with no workspace
//! to expand its types, a file knows no declared type: each of its names is
//! "unknown type", and a spread an internal error. A `type` item has no
//! existence at runtime: it is never run, nor warned about as never run, and
//! lowers to nothing; `--dump-types` shows each with its expansion
//! ([`Types::declared`](super::Types::declared)).

use crate::ast::{
    Effect, ExprId, FieldDefault, FieldTyKind, FieldTyRange, FnId, GenericOwner, MetaTy,
    NeedTyRange, ParamTyRange, PathSegment, Syntax, TypeArgRange, TypeExprId, TypeExprKind,
    TypeProblem,
};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::suggest;
use crate::types::{GenericId, Laziness, Meta, Need, TyId, TyKind};
use crate::workspace::ContextDefault;

use super::{
    Bareness, Binding, CONSTANT_NOTE, Checker, Constant, FIELD_NOT_CONSTANT, appears_twice,
    bareness_of, position, spread_first_note,
};

/// A need written in a `type` item's type.
const NEED_IN_TYPE: &str = "a declared type cannot need a context yet";

/// Why a need cannot be written in a `type` item's type.
const NEED_IN_TYPE_NOTE: &str = "a declared type is copied where it is used, and resolved there, \
                                 so its contexts would be those of the file using it: write \
                                 the function type where it is used";

impl Checker<'_> {
    /// The type a type expression denotes, with a bare type meaning what
    /// `mode` says (see the module documentation): in `Either` mode, a type
    /// written as a name, a record type or a list type (`List<T>` or `T[]`)
    /// is the either shape, a record type's fields and a list type's
    /// element type being resolved in `Either` mode too. A function type
    /// is never made the either shape at its own position, whatever the
    /// mode: its interior follows its own purity marker, and a function
    /// value that may be a future is written `Future<fn...>` or `fn... |
    /// Future<fn...>`. Nor are an applied type or a union, which say
    /// themselves what they are. `Error` (reported) if the type names no
    /// type, or is a record type with a field that names none or a name
    /// that appears twice, or a function type with a parameter or return
    /// type that is erroneous, or an applied type or a union that is not
    /// one of those accepted, or contains one that is erroneous.
    ///
    /// A type the expansion of the file's types made (see [Type
    /// declarations](#type-declarations)) is resolved as it was made: one
    /// that names nothing it can is `Error`, its problem reported; a copy of
    /// a declared type's definition is resolved with nothing reported, its
    /// own file's check having reported what is wrong in it, and is `Error`
    /// if anything is; a record type with spreads is the record type of its
    /// merged fields, or `Error`, its spreads' problems reported.
    pub(super) fn resolve_type(&mut self, id: TypeExprId, mode: Bareness) -> TyId {
        if self.ast.type_expr(id).kind == TypeExprKind::Invalid {
            return self.invalid_type(id);
        }
        // A copy of a definition is resolved once in each mode, however
        // many uses share it, so that a type built from others costs what
        // its definition does, not what its expansion would, written out.
        if self.ast.is_copied(id) {
            if let Some(&resolved) = self.copies.get(&(id, mode)) {
                self.quiet.failed |= resolved == TyId::ERROR;
                return resolved;
            }
            let resolved = self.resolve_quietly(id, mode);
            self.copies.insert((id, mode), resolved);
            return resolved;
        }
        if !self.quiet.types && self.ast.is_definition_copy(id) {
            return self.resolve_quietly(id, mode);
        }
        self.resolve_written(id, mode)
    }

    /// The type the type expression `id`, a copy of a declared type's
    /// definition or a name made one, denotes in `mode`, resolved with
    /// nothing reported (see [`Checker::resolve_type`]): `Error` if
    /// anything would be, the check then failing quietly.
    pub(super) fn resolve_quietly(&mut self, id: TypeExprId, mode: Bareness) -> TyId {
        let outer = std::mem::replace(&mut self.quiet.types, true);
        let errors = self.errors.len();
        let resolved = self.resolve_written(id, mode);
        self.quiet.types = outer;
        if self.errors.len() > errors {
            self.errors.truncate(errors);
            self.quiet.failed = true;
            return TyId::ERROR;
        }
        resolved
    }

    /// The type `Error` of the type expression `id`, which names nothing it
    /// can ([`TypeExprKind::Invalid`]): its problem, if it is this file's to
    /// report, reported; a check with nothing to report for it fails
    /// quietly (see [Type declarations](#type-declarations)).
    pub(super) fn invalid_type(&mut self, id: TypeExprId) -> TyId {
        let ast = self.ast;
        let mut reported = false;
        if !self.quiet.types && !self.sees_broken() {
            for problem in ast.type_problems(id) {
                if let TypeProblem::Unresolved(error) = problem {
                    self.errors.push(error.clone());
                    reported = true;
                }
            }
        }
        self.quiet.failed |= !reported;
        TyId::ERROR
    }

    /// Reports the problems the expansion of the file's types found with
    /// the spreads of the record type `id` (see [Type
    /// declarations](#type-declarations)), unless resolving quietly, and
    /// returns whether it has any, which make it erroneous: a spread of a
    /// type that is not a record type, "cannot spread type T: it is not a
    /// record type", at the type spread, which is resolved here, being in
    /// none of the record's fields, so that its own errors are reported
    /// instead if it has any; a spread of a field of a field written before
    /// it, "field `x` is defined twice", as for a value's spread; and a
    /// spread of a record type written in place with a name twice, "field
    /// `x` appears twice in this type", at the later, as anywhere else.
    pub(super) fn spread_problems(&mut self, id: TypeExprId) -> bool {
        let ast = self.ast;
        let mut any = false;
        for problem in ast.type_problems(id) {
            any = true;
            let reported = self.errors.len();
            match *problem {
                TypeProblem::NotRecord(operand) => {
                    let ty = self.resolve_type(operand, Bareness::Present);
                    if ty != TyId::ERROR && !self.quiet.types {
                        let shown = self.in_message(ty);
                        let mut error = Diagnostic::error(
                            format!("cannot spread type {shown}: it is not a record type"),
                            ast.type_expr(operand).span,
                        );
                        // A type parameter may be bound to a record type,
                        // but its definition is merged before any is.
                        if self.types.has_param(ty) {
                            error = error.with_note(
                                "a type parameter is not known to be a record type where the \
                                 definition is merged",
                            );
                        }
                        self.errors.push(error);
                    }
                }
                TypeProblem::Replaces {
                    operand,
                    name,
                    ty,
                    first,
                    others,
                } => {
                    let ty = self.resolve_type(ty, Bareness::Present);
                    if !self.quiet.types {
                        let note = spread_first_note(others as usize);
                        let (message, label) = match name {
                            Some(name) => {
                                let text = FieldName(ast.names().get(name)).to_string();
                                (
                                    format!("field `{text}` is defined twice"),
                                    format!(
                                        "`{text}` is a field of this, of type {}",
                                        self.in_label(ty)
                                    ),
                                )
                            }
                            None => (
                                "the rest-field is defined twice".to_owned(),
                                format!("this has a rest-field, of type {}", self.in_label(ty)),
                            ),
                        };
                        self.errors.push(
                            Diagnostic::error(message, ast.type_expr(operand).span)
                                .with_label(label)
                                .with_context(first, "first defined here")
                                .with_note(note),
                        );
                    }
                }
                TypeProblem::Repeated { name, at } => {
                    if !self.quiet.types {
                        self.errors
                            .push(Diagnostic::error(appears_twice(ast.names(), name), at));
                    }
                }
                TypeProblem::Unresolved(_) | TypeProblem::Erroneous => {}
            }
            self.quiet.failed |= self.errors.len() == reported;
        }
        any
    }

    /// The type a type expression denotes, as [`Checker::resolve_type`]
    /// gives it, once it is known to be neither one that names nothing nor
    /// a copy of a declared type's definition to resolve quietly.
    pub(super) fn resolve_written(&mut self, id: TypeExprId, mode: Bareness) -> TyId {
        let ty = *self.ast.type_expr(id);
        match ty.kind {
            // An enum, nominal, bare as a name is.
            TypeExprKind::Enum { key, name } => {
                let ty = self.types.enum_type(key, name);
                self.bare(ty, mode)
            }
            // A `^NAME` parameter's, its context's declared type exactly.
            TypeExprKind::Context(name) => match self.context_named(name, ty.span) {
                Some(context) => self.types.context_type(context.key).unwrap_or(TyId::ERROR),
                None => TyId::ERROR,
            },
            // A type parameter in scope, which no bareness wraps.
            TypeExprKind::Named(name)
                if let Some((owner, index)) = self.ast.type_param(id, name) =>
            {
                let generic = self.generic_of(owner);
                self.types.param(generic, index)
            }
            TypeExprKind::Named(name) => {
                if let Some(&(_, builtin)) = self
                    .builtin_types
                    .iter()
                    .find(|(builtin, _)| *builtin == Some(name))
                {
                    return self.bare(builtin, mode);
                }
                let text = self.ast.names().get(name);
                if self.is_type_constructor(name) {
                    let message =
                        format!("type `{text}` needs a type argument, as in `{text}<Int>`");
                    self.errors.push(Diagnostic::error(message, ty.span));
                } else {
                    let message = format!("unknown type `{text}`");
                    let suggestion = self.type_suggestion(name, ty.span, false);
                    self.unresolved_name(
                        name,
                        Diagnostic::error(message, ty.span).with_suggestion(suggestion),
                    );
                }
                TyId::ERROR
            }
            TypeExprKind::List(element) => self.resolve_list(element, mode),
            TypeExprKind::Applied { name, span, args } => {
                self.resolve_applied((name, span), args, mode)
            }
            TypeExprKind::Union { bar, .. } => self.resolve_union(id, bar, mode),
            TypeExprKind::Record(range) => self.resolve_record(id, range, mode),
            TypeExprKind::Fn {
                params,
                needs,
                ret,
                effect,
                meta,
            } => self.resolve_fn_type(id, (params, needs, ret), effect, meta),
            // A qualified name is made what it names by the expansion of
            // the file's types; checked alone, a file names no module.
            TypeExprKind::Path(path) | TypeExprKind::PathApplied { path, .. } => {
                let segments = self.ast.path(path);
                let shown = self.ast.show_path(segments).to_string();
                let error = Diagnostic::error(format!("unknown type `{shown}`"), ty.span);
                match segments.first().and_then(PathSegment::name) {
                    Some(first) => self.unresolved_name(first, error),
                    None => self.unresolved(error),
                }
                TyId::ERROR
            }
            TypeExprKind::Ref(_) | TypeExprKind::Instance { .. } => {
                self.errors.push(
                    InternalError {
                        message: "a reference to a declared type is in a tree",
                        span: ty.span,
                    }
                    .into(),
                );
                TyId::ERROR
            }
            // Each use of a generic type substitutes its arguments for its
            // parameters; the copy of its definition left is no type of any.
            TypeExprKind::Param(_) => {
                self.errors.push(
                    InternalError {
                        message: "a generic type's parameter is reachable in a tree",
                        span: ty.span,
                    }
                    .into(),
                );
                TyId::ERROR
            }
            TypeExprKind::Invalid => self.invalid_type(id),
        }
    }

    /// The type the record type `id`, whose fields are `range`, denotes in
    /// `mode` (see [`Checker::resolve_type`]): the record of its fields'
    /// types and its rest-field's, if it has one, each resolved in `mode`
    /// too, written bare in `mode`; or `Error` if a field's type is
    /// erroneous, a name appears twice, "field `x` appears twice in this
    /// type", reported at each appearance after its first, the rest-field
    /// appears twice, "the rest-field appears twice in this type", at the
    /// later, or a spread of it had a problem ([`Checker::spread_problems`]).
    pub(super) fn resolve_record(
        &mut self,
        id: TypeExprId,
        range: FieldTyRange,
        mode: Bareness,
    ) -> TyId {
        let fields = self.ast.field_tys(range);
        let base = self.sorted_fields.len();
        let mut erroneous = false;
        let mut defaults: Vec<(Name, Constant)> = Vec::new();
        let mut rest: Option<TyId> = None;
        for (index, field) in fields.iter().enumerate() {
            let name = match field.kind {
                FieldTyKind::Field(name) => name,
                FieldTyKind::Rest => {
                    let rest_ty = self.resolve_type(field.ty, mode);
                    erroneous |= rest_ty == TyId::ERROR;
                    if rest.replace(rest_ty).is_some() {
                        self.errors.push(Diagnostic::error(
                            appears_twice(self.ast.names(), None),
                            field.span,
                        ));
                        erroneous = true;
                    }
                    continue;
                }
                // A spread is merged into the fields by the expansion of the
                // file's types, which a workspace does before checking a file.
                FieldTyKind::Spread => {
                    self.errors.push(
                        InternalError {
                            message: "a spread in a record type was not expanded",
                            span: field.span,
                        }
                        .into(),
                    );
                    erroneous = true;
                    continue;
                }
            };
            // Resolving a nested record type uses `sorted_fields` above this
            // one's entries and truncates back.
            let field_ty = self.resolve_type(field.ty, mode);
            erroneous |= field_ty == TyId::ERROR;
            if let Some(default) = field.default {
                match self.field_default(default, (field_ty, field.ty)) {
                    Some(constant) => defaults.push((name, constant)),
                    None => erroneous = true,
                }
            }
            self.sorted_fields.push((name, position(index), field_ty));
        }
        erroneous |= self.spread_problems(id);
        let sorted = self.sorted_fields.get_mut(base..).unwrap_or_default();
        sorted.sort_unstable_by_key(|&(name, index, _)| (name, index));
        // Every appearance of a name after its first is reported.
        let mut previous = None;
        for &(name, index, _) in &*sorted {
            if previous.replace(name) != Some(name) {
                continue;
            }
            let Some(repeat) = fields.get(index as usize) else {
                continue;
            };
            self.errors.push(Diagnostic::error(
                appears_twice(self.ast.names(), Some(name)),
                repeat.span,
            ));
            erroneous = true;
        }
        let resolved = if erroneous {
            TyId::ERROR
        } else {
            let sorted = self.sorted_fields.get(base..).unwrap_or_default();
            let record = self.types.record_with_rest(
                sorted.iter().map(|&(name, _, ty)| (name, ty)),
                rest,
                defaults,
            );
            self.bare(record, mode)
        };
        self.sorted_fields.truncate(base);
        resolved
    }

    /// The constant that `default`, the default of a field of a record type
    /// whose type is `ty`, written as `written`, is (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)), or `None` if it is in error, which makes
    /// the record type erroneous. One the parser made is checked where its
    /// type is resolved, as a parameter's default is (see
    /// [`Checker::check_defaults`]), and as the rest of the type is, so
    /// that a type resolved in silence first, as a top-level function's
    /// signature is ([`Checker::declared_type`]), reports it once, where it
    /// is resolved again: it must be a constant, "a field's
    /// default must be a constant" at it, noted "a literal, or a list or
    /// record of literals", and is otherwise checked against `ty` as an
    /// annotated initialiser is against its annotation, the usual mismatch
    /// at it if it does not fit. One a copy of a declared type's definition
    /// holds, its own file's check having reported what is wrong in it, is
    /// fitted to `ty` ([`Checker::fit_constant`]), with nothing reported:
    /// one that is no constant, or does not fit, fails the check quietly.
    pub(super) fn field_default(
        &mut self,
        default: FieldDefault,
        (ty, written): (TyId, TypeExprId),
    ) -> Option<Constant> {
        match default {
            FieldDefault::Copied(at) => {
                let fitted = self
                    .ast
                    .copied_default(at)
                    .and_then(|constant| self.fit_constant(constant, ty));
                self.quiet.failed |= fitted.is_none();
                fitted
            }
            FieldDefault::Expr(expr) => self.check_field_default(expr, (ty, written)),
        }
    }

    /// Checks the expression `default`, a field's default that the parser
    /// made, against `ty`, the field's type, written as `written`, as
    /// [`Checker::field_default`] says, and returns the constant it is, or
    /// `None` if it is in error.
    pub(super) fn check_field_default(
        &mut self,
        default: ExprId,
        (ty, written): (TyId, TypeExprId),
    ) -> Option<Constant> {
        if !self.is_constant(default) {
            self.errors.push(
                Diagnostic::error(FIELD_NOT_CONSTANT, self.ast.get(default).span)
                    .with_note(CONSTANT_NOTE),
            );
            return None;
        }
        if ty == TyId::ERROR {
            return None;
        }
        let errors = self.errors.len();
        let context = self.ast.type_expr(written).span;
        let found =
            self.check_expr_expecting(default, ty, context, "expected because of this annotation");
        if found == TyId::ERROR || self.errors.len() != errors {
            return None;
        }
        self.constant(default)
    }

    /// The type the union written as `id`, whose last `|` is at `bar`,
    /// denotes in `mode` (see the module documentation): its members are
    /// the types the `|`s separate, at any depth of the written tree. If one
    /// of them is written `Future<...>`, each is resolved with every bare
    /// type present, and the union must be the either shape: exactly one
    /// member a future, of the union of the others, which it is the either
    /// shape of; anything else is "unions other than `T | Future<T>` are not
    /// supported yet", at `bar`. Otherwise the union is bare: each member is
    /// resolved in `mode`, the either shape the bare type rule gave a bare
    /// member at its own position taken off it, and the union of the
    /// members is what [`Checker::bare`] makes of it in `mode`. `Error` if a
    /// member is erroneous.
    pub(super) fn resolve_union(&mut self, id: TypeExprId, bar: Span, mode: Bareness) -> TyId {
        let mut written = Vec::new();
        let mut pending = vec![id];
        while let Some(member) = pending.pop() {
            match self.ast.type_expr(member).kind {
                TypeExprKind::Union { lhs, rhs, .. } => pending.extend([rhs, lhs]),
                _ => written.push(member),
            }
        }
        let explicit = written.iter().any(|&member| self.is_future(member));
        let member_mode = if explicit { Bareness::Present } else { mode };
        let mut members = Vec::with_capacity(written.len());
        let mut erroneous = false;
        for &member in &written {
            let mut ty = self.resolve_type(member, member_mode);
            if member_mode == Bareness::Either && self.is_bare(member) {
                ty = self.types.strip(ty);
            }
            erroneous |= ty == TyId::ERROR;
            members.push(ty);
        }
        if erroneous {
            return TyId::ERROR;
        }
        let (lazy, present): (Vec<TyId>, Vec<TyId>) = members
            .into_iter()
            .partition(|&member| self.types.laziness(member) != Laziness::Present);
        match *lazy.as_slice() {
            [] => {
                let union = self.types.union(present);
                self.bare(union, mode)
            }
            [future] if !present.is_empty() => {
                let union = self.types.union(present);
                if self.types.kind(future) == &TyKind::Future(union) {
                    self.types.deferrable(union)
                } else {
                    self.union_future_error(bar)
                }
            }
            _ => self.union_future_error(bar),
        }
    }

    /// Reports a union with a `Future` member that is not the either shape,
    /// at its last `|`, `bar`, and returns `Error`.
    pub(super) fn union_future_error(&mut self, bar: Span) -> TyId {
        self.errors.push(Diagnostic::error(
            "a union with a `Future` member must be `T | Future<T>`",
            bar,
        ));
        TyId::ERROR
    }

    /// Whether `ty` is written `Future<...>`.
    pub(super) fn is_future(&self, ty: TypeExprId) -> bool {
        matches!(
            self.ast.type_expr(ty).kind,
            TypeExprKind::Applied { name, .. } if Some(name) == self.constructors.future
        )
    }

    /// Whether the union written as `id` has a member written
    /// `Future<...>`, at any depth of the written tree, which makes it say
    /// what it is.
    pub(super) fn has_future_member(&self, id: TypeExprId) -> bool {
        let mut pending = vec![id];
        while let Some(member) = pending.pop() {
            match self.ast.type_expr(member).kind {
                TypeExprKind::Union { lhs, rhs, .. } => pending.extend([rhs, lhs]),
                _ if self.is_future(member) => return true,
                _ => {}
            }
        }
        false
    }

    /// What the type `ty`, written bare, means in `mode`: `ty` itself in
    /// `Present` mode, and its either shape in `Either` mode.
    pub(super) fn bare(&mut self, ty: TyId, mode: Bareness) -> TyId {
        match mode {
            Bareness::Present => ty,
            Bareness::Either => self.types.deferrable(ty),
        }
    }

    /// The list type whose element type is written as `element`, written
    /// bare in `mode`, as a record type is: the element type is a position
    /// inside it, resolved in `mode` too. `Error` if the element type is
    /// erroneous.
    pub(super) fn resolve_list(&mut self, element: TypeExprId, mode: Bareness) -> TyId {
        let element = self.resolve_type(element, mode);
        if element == TyId::ERROR {
            return TyId::ERROR;
        }
        let list = self.types.list(element);
        self.bare(list, mode)
    }

    /// Whether `name` is `Future`, `Present` or `List`, which take a type
    /// argument.
    pub(super) fn is_type_constructor(&self, name: Name) -> bool {
        Some(name) == self.constructors.future
            || Some(name) == self.constructors.present
            || Some(name) == self.constructors.list
            || Some(name) == self.constructors.variant
            || Some(name) == self.constructors.partial
            || Some(name) == self.constructors.secret
            || Some(name) == self.constructors.revealed
    }

    /// Whether a type written as `ty` is *bare*, so that the bare type rule
    /// applies to it: a name, a record type, a list type, written `List<T>`
    /// or `T[]`, or a union with no member written `Future<...>`. Not
    /// `Present<...>`, `Future<...>`, a union with a `Future` member or a
    /// function type, which say what they are.
    pub(super) fn is_bare(&self, ty: TypeExprId) -> bool {
        match self.ast.type_expr(ty).kind {
            // A type parameter is what it is bound to, lazy or not.
            TypeExprKind::Named(name) if self.ast.type_param(ty, name).is_some() => false,
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::PathApplied { .. }
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Record(_)
            | TypeExprKind::List(_) => true,
            TypeExprKind::Applied { name, .. } => [
                self.constructors.list,
                self.constructors.partial,
                self.constructors.secret,
                self.constructors.revealed,
            ]
            .contains(&Some(name)),
            TypeExprKind::Union { .. } => !self.has_future_member(ty),
            // A context's type is what it is declared to be.
            TypeExprKind::Fn { .. }
            | TypeExprKind::Ref(_)
            | TypeExprKind::Instance { .. }
            | TypeExprKind::Param(_)
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => false,
        }
    }

    /// The type the applied type `NAME<ARG>` denotes, the name `name`
    /// written at `span`, its arguments `args`, in `mode`: `Future<T>` for
    /// `Future` (`T` stripped, as [`TypeTable::future`](crate::types::TypeTable::future) does, so
    /// `Future<Future<Int>>` is `Future<Int>`), and `T` for `Present`, where
    /// `T` is the argument resolved with every bare type present; the list
    /// type of the argument for `List`, written bare in `mode` (see
    /// [`Checker::resolve_list`]); or `Error` (reported) for any other name,
    /// an erroneous argument, or a built-in given other than one argument,
    /// "type `List` takes 1 type argument, given 2". Every argument is
    /// resolved, and its errors reported, whatever the name; a declared
    /// type's application never reaches the checker, the expansion of the
    /// tree's types having made it its definition.
    pub(super) fn resolve_applied(
        &mut self,
        (name, span): (Name, Span),
        args: TypeArgRange,
        mode: Bareness,
    ) -> TyId {
        if Some(name) == self.constructors.variant {
            return self.resolve_variant_type(span, args);
        }
        let builtin = [
            self.constructors.list,
            self.constructors.future,
            self.constructors.present,
            self.constructors.partial,
            self.constructors.secret,
            self.constructors.revealed,
        ]
        .contains(&Some(name));
        let &[arg] = self.ast.type_args(args) else {
            for &arg in self.ast.type_args(args) {
                self.resolve_type(arg, Bareness::Present);
            }
            let text = self.ast.names().get(name);
            if builtin {
                let message = format!("type `{text}` takes 1 type argument, given {}", args.len());
                self.errors.push(Diagnostic::error(message, span));
            } else {
                self.unknown_applied(name, span);
            }
            return TyId::ERROR;
        };
        if Some(name) == self.constructors.list {
            return self.resolve_list(arg, mode);
        }
        if Some(name) == self.constructors.partial {
            return self.resolve_partial(arg);
        }
        let arg = self.resolve_type(arg, Bareness::Present);
        if Some(name) == self.constructors.future {
            return if arg == TyId::ERROR {
                TyId::ERROR
            } else {
                self.types.future(arg)
            };
        }
        if Some(name) == self.constructors.present {
            return arg;
        }
        if Some(name) == self.constructors.secret {
            return self.secret_type(arg, span, false);
        }
        if Some(name) == self.constructors.revealed {
            return self.secret_type(arg, span, true);
        }
        self.unknown_applied(name, span);
        TyId::ERROR
    }

    /// Reports the applied type of the name `name`, written at `span`, that
    /// is not built in taking a type argument: "type `Int` takes no type
    /// argument" for another built-in type, "unknown type `Foo`" for any
    /// other name.
    pub(super) fn unknown_applied(&mut self, name: Name, span: Span) {
        let text = self.ast.names().get(name);
        let builtin = self
            .builtin_types
            .iter()
            .any(|(builtin, _)| *builtin == Some(name));
        if builtin {
            let message = format!("type `{text}` takes no type argument");
            self.errors.push(Diagnostic::error(message, span));
        } else {
            let suggestion = self.type_suggestion(name, span, true);
            self.unresolved_name(
                name,
                Diagnostic::error(format!("unknown type `{text}`"), span)
                    .with_suggestion(suggestion),
            );
        }
    }

    /// The suggestion for the unknown type `name`, written at `span`, with a
    /// type argument if `applied` (see [Suggestions](crate::check::names#suggestions)): the
    /// nearest of the types of the file's view and the built-in types that
    /// take no argument, or, applied, of `List`, `Future` and `Present`.
    /// None where the error is not reported: resolving quietly, or in a
    /// file whose view sees one with syntax errors.
    pub(super) fn type_suggestion(
        &self,
        name: Name,
        span: Span,
        applied: bool,
    ) -> Option<(Span, String)> {
        if self.quiet.types || self.sees_broken() {
            return None;
        }
        let names = self.ast.names();
        let mut candidates: Vec<(&str, u8)> = Vec::new();
        if applied {
            candidates.extend(["List", "Future", "Present"].map(|text| (text, 1)));
        } else {
            candidates.extend(
                ["Int", "Float", "Bool", "String", "Duration", "Size"].map(|text| (text, 1)),
            );
            if let Some(workspace) = self.workspace {
                candidates.extend(
                    workspace
                        .type_names()
                        .map(|declared| (names.get(declared), 0))
                        .filter(|&(text, _)| crate::lexer::is_identifier(text)),
                );
            }
        }
        let best = suggest::best(names.get(name), candidates)?;
        Some((span, best.to_owned()))
    }

    /// Reports `error`, of a name that names nothing, a variable's or a
    /// type's, unless the input's view sees a file with syntax errors, in
    /// whose source that could not be parsed the name may be declared: the
    /// check then fails quietly, its error reported where the source is
    /// (see [Syntax errors](crate::workspace::parse#syntax-errors)).
    pub(super) fn unresolved(&mut self, error: Diagnostic) {
        if self.sees_broken() {
            self.quiet.failed = true;
        } else {
            self.errors.push(error);
        }
    }

    /// [`Checker::unresolved`] of the error `error` of the name `name`,
    /// which is no error, the check failing quietly, if the file reads the
    /// name as `Error` (see [`crate::workspace`], "Scan errors").
    pub(super) fn unresolved_name(&mut self, name: Name, error: Diagnostic) {
        if self
            .workspace
            .is_some_and(|workspace| workspace.poisoned(name))
        {
            self.quiet.failed = true;
        } else {
            self.unresolved(error);
        }
    }

    /// Whether a read of the name `name`, whose binding where it is read is
    /// `found`, is a read of what the file reads as `Error`, silently, the
    /// scan having recovered from an error about it (see
    /// [`crate::workspace`], "Scan errors"): a name of the file's view, or
    /// one that names nothing, not a binding of a function or an inner block
    /// of it. The check then fails quietly.
    pub(super) fn reads_poisoned(&mut self, name: Name, found: Option<(usize, Binding)>) -> bool {
        let poisoned = self
            .workspace
            .is_some_and(|workspace| workspace.has_poison() && workspace.poisoned(name))
            && found.is_none_or(|(index, _)| index < self.top_end);
        self.quiet.failed |= poisoned;
        poisoned
    }

    /// Whether the input's view sees a file with syntax errors, its own
    /// tree among them: a name that names nothing may be declared in source
    /// its parser could not parse, so it is no error, and the check fails
    /// quietly (see [`Checker::unresolved`]).
    pub(super) fn sees_broken(&self) -> bool {
        self.quiet.sees_broken || self.ast.syntax() != Syntax::Clean
    }

    /// The type of `function`, whose parameters' types are `fn_params[base..]`
    /// and whose result is `ret`, of `effect`, its rest parameter its own
    /// ([`Checker::intern_fn_type_with_meta`]), made generic if the function
    /// has type parameters (see [Generics](crate::check::methods#generics)).
    pub(super) fn function_type(
        &mut self,
        function: FnId,
        base: usize,
        (ret, effect): (TyId, Effect),
    ) -> TyId {
        let params = self.ast.params(self.ast.fn_decl(function).params);
        let rest_param = params
            .iter()
            .position(|param| param.rest)
            .and_then(|at| u8::try_from(at).ok());
        let ty = self.intern_fn_type_with_meta(base, (ret, effect), None, rest_param);
        if ty == TyId::ERROR || self.ast.generics(function).is_none() {
            return ty;
        }
        let generic = self.generic_of(GenericOwner::Fn(function));
        self.types.generic_fn(ty, generic)
    }

    /// What the type parameters of the generic function or `type` item
    /// `owner` belong to, made the first time it is asked for.
    pub(super) fn generic_of(&mut self, owner: GenericOwner) -> GenericId {
        if let Some(&generic) = self.calls.ids.get(&owner) {
            return generic;
        }
        let names: Box<[Name]> = self
            .ast
            .generics_of(owner)
            .map(|generics| generics.params.iter().map(|&(name, _)| name).collect())
            .unwrap_or_default();
        let generic = self.types.new_generic(names);
        self.calls.ids.insert(owner, generic);
        generic
    }

    /// Interns the function type whose parameters are `fn_params[base..]`
    /// and whose return type is `ret`, of effect `effect`, with the meta
    /// `meta`, if it has one (see [Metas](crate::check::attributes#metas)),
    /// and its parameter at position `rest`, if any, its rest parameter; or
    /// returns `Error` if any of them is erroneous, having been reported.
    pub(super) fn intern_fn_type_with_meta(
        &mut self,
        base: usize,
        (ret, effect): (TyId, Effect),
        meta: Option<Meta>,
        rest_param: Option<u8>,
    ) -> TyId {
        let params = self.fn_params.get(base..).unwrap_or_default();
        if ret == TyId::ERROR || params.iter().any(|&(_, ty)| ty == TyId::ERROR) {
            return TyId::ERROR;
        }
        self.types
            .fn_type_with_rest(params.iter().copied(), (ret, effect), meta, rest_param)
    }

    /// The function type written as `id`, of the parameters `params` and
    /// the result `ret`, of `effect`, with the meta `meta`, if it has one:
    /// its parameters resolved by its own rule, whatever surrounds it, a
    /// second rest parameter reported ([`Checker::rest_position`]), and an
    /// erroneous meta making it erroneous.
    pub(super) fn resolve_fn_type(
        &mut self,
        id: TypeExprId,
        (params, needs, ret): (ParamTyRange, NeedTyRange, TypeExprId),
        effect: Effect,
        meta: Option<MetaTy>,
    ) -> TyId {
        let needs = self.written_needs(id, needs);
        let (params_mode, ret_mode) = bareness_of(effect);
        let base = self.fn_params.len();
        let rest_param = self.rest_position(
            self.ast.param_tys(params).iter().map(|param| param.rest),
            self.ast.type_expr(id).span,
        );
        for param in self.ast.param_tys(params) {
            // Resolving a nested function type uses `fn_params` above this
            // one's entries and truncates back.
            let param_ty = self.resolve_type(param.ty, params_mode);
            self.fn_params.push((param.name, param_ty));
        }
        let ret = self.resolve_type(ret, ret_mode);
        let meta = meta.map(|meta| self.resolve_meta(meta));
        let ret = if matches!(meta, Some(None)) {
            TyId::ERROR
        } else {
            ret
        };
        let resolved =
            self.intern_fn_type_with_meta(base, (ret, effect), meta.flatten(), rest_param);
        self.fn_params.truncate(base);
        match needs {
            None => TyId::ERROR,
            Some(needs) if needs.is_empty() || resolved == TyId::ERROR => resolved,
            Some(needs) => self.types.with_needs(resolved, &needs),
        }
    }

    /// The needs of the function type `id`, written as `written` (see
    /// [Needs](crate::check::needs#needs)): each a context with no default,
    /// by its declaration's key. `None` if one is erroneous, reported: a
    /// context with a default, "context `level` has a default, so nothing
    /// needs it"; or a need in a `type` item's type, "a declared type cannot
    /// need a context yet", since a copy of it is resolved where it is
    /// used, whose contexts are not its declaration's. A copy of such a type
    /// is erroneous where it is used too, its declaration's error reported.
    pub(super) fn written_needs(
        &mut self,
        id: TypeExprId,
        written: NeedTyRange,
    ) -> Option<Vec<Need>> {
        let written = self.ast.need_tys(written);
        let Some(first) = written.first() else {
            return Some(Vec::new());
        };
        if self.ast.is_definition_copy(id) {
            self.quiet.failed = true;
            return None;
        }
        if self.in_type_item {
            self.errors
                .push(Diagnostic::error(NEED_IN_TYPE, first.span).with_note(NEED_IN_TYPE_NOTE));
            return None;
        }
        let mut needs = Vec::with_capacity(written.len());
        let mut erroneous = false;
        for need in written {
            let Some(context) = self.context_named(need.name, need.span) else {
                erroneous = true;
                continue;
            };
            match context.default {
                ContextDefault::None => needs.push(Need::from([context.key])),
                ContextDefault::Foreign(None) => {
                    self.quiet.failed = true;
                    erroneous = true;
                }
                ContextDefault::Binding(_) | ContextDefault::Foreign(Some(_)) => {
                    let text = self.ast.names().get(need.name);
                    self.errors.push(
                        Diagnostic::error(
                            format!("context `{text}` has a default, so nothing needs it"),
                            need.span,
                        )
                        .with_note(
                            "a function may read a context with a default wherever it is \
                             called: name only contexts with none",
                        ),
                    );
                    erroneous = true;
                }
            }
        }
        (!erroneous).then_some(needs)
    }

    /// The position of the rest parameter among parameters that are rest
    /// parameters as `rests` says, written at `span`, if one is: "a
    /// function has one rest parameter at most" for a second.
    pub(super) fn rest_position(
        &mut self,
        rests: impl Iterator<Item = bool>,
        span: Span,
    ) -> Option<u8> {
        let positions: Vec<usize> = rests
            .enumerate()
            .filter_map(|(index, rest)| rest.then_some(index))
            .collect();
        if positions.len() > 1 {
            self.errors.push(Diagnostic::error(
                "a function has one rest parameter at most",
                span,
            ));
        }
        positions.first().and_then(|&rest| u8::try_from(rest).ok())
    }

    /// The meta the function type's meta written as `meta` denotes (see
    /// [Metas](crate::check::attributes#metas)): its type resolved as a return type of its marker's
    /// effect is, either-shaped under `META->` and `META->>` and present under
    /// `META:` where it is bare, which must be a record type once every
    /// `Future` is taken off it, "a meta must be a record type: its fields
    /// are read from the function" otherwise, at the type. `None` if it is
    /// erroneous, reported.
    pub(super) fn resolve_meta(&mut self, meta: MetaTy) -> Option<Meta> {
        let (_, mode) = bareness_of(meta.effect);
        let ty = self.resolve_type(meta.ty, mode);
        if ty == TyId::ERROR {
            return None;
        }
        if !self.types.is_record(self.types.strip(ty)) {
            self.errors.push(Diagnostic::error(
                "a meta must be a record type: its fields are read from the function",
                self.ast.type_expr(meta.ty).span,
            ));
            return None;
        }
        Some(Meta {
            effect: meta.effect,
            ty,
        })
    }
}
