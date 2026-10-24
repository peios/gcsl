//! Checking attributes and metas: the attributes a function is marked with,
//! their constant arguments, and the metas a function's values carry.
//!
//! # Attributes
//!
//! An attribute says how to run a function, for the host that runs it or
//! for GCSL: its arguments are a record of constants, and so an attribute
//! is declared as the record type of its arguments, a `type` marked
//! `#[attribute]` (see the parser's [Attributes](crate::parser::attributes#attributes)),
//! `#[attribute] type hermetic = {on: Bool = true}`. Being a type, it is
//! named, imported and reported in conflicts as any type is (see
//! [`crate::workspace`], "Types"), and is usable as one. Its type must be a
//! record type, with no rest-field, whose fields hold only constants:
//! `Int`, `Float`, `Bool`, `String`, `Duration`, `Size`, or lists and
//! records of them (`Checker::holds_constants`), "an attribute's arguments
//! are a record of constants" otherwise, at its definition, noted with what
//! a field may be; and it may not be named like a built-in attribute
//! ([`BUILTIN_ATTRIBUTES`](super::BUILTIN_ATTRIBUTES)), "`retry` is a built-in attribute".
//!
//! `retry`, `{times: Int}`, and `timeout`, `{limit: Duration}`, are the
//! checker's own, known in every file, whatever its types.
//!
//! Each use on a function item (`Checker::check_attribute_uses`), checked
//! before the file's body, names one of the checker's or a type of the
//! file's view: "unknown attribute `x`" for none, with a suggestion among
//! the attributes in view and the checker's, and "`PackageDef` is not an
//! attribute", noted "an attribute is a `type` marked `#[attribute]`", for
//! a type that is not marked. Its arguments fill its record's fields: one
//! positional argument for an attribute of one field, and named ones for
//! any, "`#[device]` takes its arguments by name, as in `#[device(name =
//! ...)]`" for a positional one of another; "`#[retry]` takes one
//! argument", "`#[retry]` has no argument `x`", "`times` is given twice"
//! and "`#[retry]` takes no block" otherwise. Each must be a constant, by
//! the rule for constant defaults, "an attribute's argument must be a
//! constant", and is checked against its field's type as a constant
//! default is, "expected by this attribute" labelling the attribute's
//! name. A field given nothing takes its default, and one with none is
//! "`#[device]` needs `name`". `#[hermetic()]` is `#[hermetic]`. What the
//! arguments take of the frame's slots is given back: an attribute is never
//! run.
//!
//! A function's attributes ([`Attribute`]), each what a host knows it by
//! ([`AttributeId`]) and its arguments, every field by name with the
//! constant given or its default, are recorded by its function, in source
//! order ([`Resolution::attributes`](super::Resolution::attributes)), for lowering to give its code
//! ([`crate::ir::FnCode::attributes`]). A definition made with a def-type
//! takes the def-type's attributes as defaults (`Checker::inherit_attributes`):
//! the def-type's, each replaced whole by the definition's own of the same
//! attribute, then the definition's others; a def-type of another file or
//! of an earlier session input is known by what its cell or binding
//! carries ([`KnownParams`](super::KnownParams)). Plain GCSL gives a host's attributes no
//! meaning, and runs `retry` and `timeout` (see [`crate::eval`], "Retry and
//! timeout"). `#[retry]` needs a `times` of at least 0, "`#[retry]` needs a
//! `times` of at least 0" otherwise, and `#[timeout]` a positive `limit`,
//! "`#[timeout]` needs a positive `limit`" (`Checker::fits_builtin`); and
//! `#[retry]` on a pure function, a `fn` or `def` declared with `:`, is the
//! warning "retrying a pure function cannot change its result", noted "a
//! pure function's result depends only on its arguments".
//!
//! # Metas
//!
//! A function type may have a *meta*, written after its return type with a
//! marker of its own (see the parser's "Types"), as in `fn(v: Version):
//! PackageDef META-> PackageMeta`: a record attached to the function value,
//! whose fields are read as the function's. Its type is resolved as a
//! return type of its marker's effect is (`Checker::resolve_meta`): written
//! bare, it is the either shape under `META->` and `META->>`, at its
//! position and every bare position inside it, and present under `META:`,
//! so a `META: Future<X>` meta is a future. It must be a record type once
//! every `Future` is taken off it: "a meta must be a record type: its
//! fields are read from the function" otherwise, at the type. A function
//! type with a meta is a subtype of the same without one, and metas compare
//! as results do (see [`crate::types`], "Metas"); a function expression, a
//! `def` and a built-in function have no meta, so none fits a type with one.
//!
//! **Reading.** A field access on a value of a function type with a meta,
//! once a value that may be a future is awaited as any record a field access
//! reads is, reads the meta's field (`Checker::meta_read`): `func.versions`
//! is the meta's field `versions`, of the field's type, and `func.a.b` goes
//! on from it. A field the meta lacks is "the meta of `func` has no field
//! `x`", at the `.NAME`, noted with the meta's fields ("the only field is
//! `versions`"); a field access on a function with no meta is the error it
//! always was, "cannot access field `x` of `fn(): Int`". A read is a call of
//! the meta's closure, of the meta's effect: from a pure function, reading a
//! `META->` or `META->>` meta is "cannot read an impure meta from a pure
//! function", labelled "reading the meta of `func` is impure (`META->`)" at
//! the function and at the pure function's marker, noted as the call error
//! is; a memoised function may read one wherever it may call an impure
//! function. A meta that may be a future, the either shape of an impure
//! meta's or a `Future`, is then awaited at the top, as a record a field
//! access reads is, subject to the purity rule, unless the read is already
//! that error. What a read reads, and how its meta is awaited, is recorded
//! for lowering ([`Resolution::meta_read`](super::Resolution::meta_read)).
//!
//! **Declaring.** A def-type declares the meta of its definitions by the
//! meta of its definition parameter's function type, F. `defmeta DT PATH {
//! ... }` or `defmeta DT PATH = EXPR` ([`ItemKind::DefMeta`]) declares the
//! meta of the definition `def DT PATH(...)` of its directory: a function of
//! no parameters, its type the function of F's meta's effect returning F's
//! meta's type, its body checked as such a function's
//! (`Checker::check_defmeta`), so under `META->` it may `exec` and under
//! `META:` it is pure. Its `DT` is read as a definition's is ("`DT` is not
//! a def-type" otherwise); a def-type that declares no meta is "`package`
//! declares no meta for its definitions", noted "a def-type declares one by
//! its definition parameter's type, as in `func: fn(): PackageDef META:
//! PackageMeta`", at the meta's `defmeta` through its path. A meta binds
//! nothing a source can read: it is a top-level function, as a definition
//! is, and the leaf of its path, as one name, under the meta key of its
//! def-type and its file's directory (see [`crate::names`], "Namespace
//! keys"), placed among the namespaces' fields as a definition is
//! (`Checker::place_definition`) and typed as the input starts
//! (`Checker::defmeta_type`).
//!
//! **Pairing.** A definition whose def-type declares a meta reads the cell of
//! its meta, the leaf of its path among its directory's metas, as a reference
//! to it would, from inside its own function, as its body reads a top-level
//! function (`Checker::definition_meta`), and the slot that read reads is what
//! its code names ([`Resolution::metas`](super::Resolution::metas)), which a
//! read of its closure's meta calls; so the definition needs its meta, as
//! mentions say ([`crate::mentions`], "Def-types"). One whose directory
//! declares no meta of its path, which the workspace finds
//! ([`crate::workspace`], "Def-types"), is "`package dev.peios.x` has no
//! `defmeta` in its directory", at its def-type and path, noted "`package`
//! declares a meta, `META-> PackageMeta`, for each of its definitions": the
//! meta as the def-type's parameter writes it if the workspace has the
//! def-type's tree, and otherwise as its type shows, present under `META->` and
//! `META->>`, as a bare type would be written there. A meta declared twice in a
//! directory, or with no definition of its path there, is the workspace's
//! error. A definition reads nothing, its read typed `{}`, when its def-type
//! declares no meta, when its directory declares none, and when it is shadowed,
//! since it is never made.
//!
//! **Through a reference.** A reference to a definition, `package
//! dev.peios.testpkg`, carries the definition's meta: its type is the
//! function of the def-type's invocation parameters with F's meta, its marker
//! and type, `fn(version: String) -> R META-> PackageMeta` (see
//! [Def-types](crate::check::deftypes#def-types)), so `(package dev.peios.testpkg).versions`, or a
//! reference stored, `let r = package x` then `r.versions`, reads it as any
//! function's meta is read, the errors naming the reference as written ("the
//! meta of `package dev.peios.testpkg` has no field `x`"). Without the
//! parentheses, a reference means the longest prefix of its path that names
//! a definition, since no definition lies under another
//! (`Checker::definition_at`): the rest of the path is read from that
//! definition's meta, `package dev.peios.testpkg.versions` its field
//! `versions`, each further segment a field of the one before. Only a
//! def-type whose F has a meta reads a path so: for any other, `package x.y`
//! names the definition `x.y`, as it always did, and a path no prefix of
//! which is a definition is the error it was. The parser reads the whole
//! path, and the reference's read is a field access for each of its
//! segments, so the accesses past the definition are typed as the field
//! accesses they are, the first reading the meta of the definition's cell
//! (`Checker::read_past`), with the meta read's errors, effect and awaiting,
//! a field the meta lacks noted "`package dev.peios.testpkg.x` is read as
//! the field `x` of the meta of `package dev.peios.testpkg`, its longest
//! prefix that is a definition". The reference has the type of the last of
//! them, and what it reads past, with the type of the reference to the
//! definition, is recorded for lowering and the dump
//! ([`Types::reference_past`](super::Types::reference_past)). A read through a reference and a read
//! through the definition, as the def-type's `func.versions`, read the one
//! meta, one closure in one cell: so a `META->>` meta's body runs once for
//! both, and a `META->` meta's at each. A read has the meta's effect wherever
//! it is, allowed at the top level of a file, an error in a pure function.
//! It needs what reading the reference needs, the definition, and so, as the
//! definition reads its meta, the meta.

use std::collections::HashMap;

use crate::ast::{ArgKind, AttributeId, AttributeUse, Effect, ExprId, ExprKind, FnId, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::suggest;
use crate::types::{Laziness, Meta, TyId, TyKind};
use crate::value::{Quantity, int_to_float};
use crate::workspace::{AttributeLookup, Context};

use super::{
    Attribute, AttributeFields, AwaitMark, Bareness, CONSTANT_NOTE, Checker, Constant, MetaOf,
    PURE_CONTEXT, RETRY, TIMEOUT, constant_of, item_function,
};

impl Checker<'_> {
    /// Checks what the file declares before its body is checked: its `type`
    /// items, quietly in a workspace, which has checked them first
    /// ([`check_types`](super::check_types)), and the uses of attributes on its functions.
    pub(super) fn check_declarations(&mut self) {
        let quiet = std::mem::replace(&mut self.quiet.types, self.workspace.is_some());
        self.type_items();
        self.quiet.types = quiet;
        self.check_attribute_uses();
    }

    /// Whether `ty` holds only constants, as an attribute's arguments must
    /// (see [Attributes](#attributes)): an `Int`, `Float`, `Bool`, `String`,
    /// `Duration` or `Size`, a list of such, or a record of such fields with
    /// no rest-field; at the `top`, only such a record.
    ///
    /// Recurses once per level of list and record nesting in the type.
    pub(super) fn holds_constants(&self, ty: TyId, top: bool) -> bool {
        match self.types.kind(ty) {
            TyKind::Record(record) => {
                record.rest().is_none()
                    && record
                        .fields
                        .iter()
                        .all(|&(_, field)| self.holds_constants(field, false))
            }
            _ if top => false,
            TyKind::Int | TyKind::Float | TyKind::Bool | TyKind::String | TyKind::Quantity(_) => {
                true
            }
            &TyKind::List(element) => self.holds_constants(element, false),
            _ => false,
        }
    }

    /// Checks every use of an attribute that is none of the parser's, on the
    /// function items they apply to (see [Attributes](#attributes)), and
    /// records each function's, in source order, as
    /// [`Resolution::attributes`](super::Resolution::attributes) gives them.
    /// Its arguments' constants take no slot of the frame: what their checks
    /// allocate is given back.
    pub(super) fn check_attribute_uses(&mut self) {
        let ast = self.ast;
        let uses = ast.all_attribute_uses();
        if uses.is_empty() {
            return;
        }
        let outer = (self.next_slot, self.frame_size, self.slot_names.len());
        let mut found: HashMap<FnId, Vec<Attribute>> = HashMap::new();
        for used in uses {
            let attribute = self.check_attribute_use(used);
            let attribute = attribute.filter(|attribute| self.fits_builtin(attribute, used));
            if let (Some(attribute), Some(function)) = (attribute, item_function(ast, used.item)) {
                if attribute.id == AttributeId::Builtin(RETRY)
                    && !matches!(ast.item(used.item).kind, ItemKind::DefTypeDef { .. })
                    && ast.fn_decl(function).effect == Effect::Pure
                {
                    self.warnings.push(
                        Diagnostic::warning(
                            "retrying a pure function cannot change its result",
                            used.span,
                        )
                        .with_note("a pure function's result depends only on its arguments"),
                    );
                }
                found.entry(function).or_default().push(attribute);
            }
        }
        (self.next_slot, self.frame_size) = (outer.0, outer.1);
        self.slot_names.truncate(outer.2);
        self.lowering.fn_attributes = found
            .into_iter()
            .map(|(function, attributes)| (function, attributes.into_boxed_slice()))
            .collect();
    }

    /// The attribute `used` names, with its arguments, once checked (see
    /// [Attributes](#attributes)): `retry` and `timeout` the checker's own,
    /// any other name a type of the file's view marked `#[attribute]`,
    /// whose record type its arguments are checked against. `None`, the
    /// error reported, if it names none, or its arguments do not fit.
    pub(super) fn check_attribute_use(&mut self, used: &AttributeUse) -> Option<Attribute> {
        let names = self.ast.names();
        let text = names.get(used.name);
        let (id, fields) = match text {
            RETRY => (
                AttributeId::Builtin(RETRY),
                vec![(Box::from("times"), TyId::INT, None)],
            ),
            TIMEOUT => (
                AttributeId::Builtin(TIMEOUT),
                vec![(Box::from("limit"), TyId::DURATION, None)],
            ),
            _ => self.declared_attribute(used)?,
        };
        let given = self.attribute_args(used, &fields)?;
        let mut args = Vec::with_capacity(fields.len());
        let mut failed = false;
        for ((field, ty, default), value) in fields.into_iter().zip(given) {
            let constant = match value {
                Some(value) => self.attribute_arg(value, ty, used.name_span),
                None if default.is_some() => default,
                None => {
                    let message = format!("`#[{text}]` needs `{field}`");
                    self.errors.push(Diagnostic::error(message, used.span));
                    None
                }
            };
            match constant {
                Some(constant) => args.push((field, constant)),
                None => failed = true,
            }
        }
        (!failed).then(|| Attribute {
            id,
            args: args.into_boxed_slice(),
        })
    }

    /// Whether `attribute`, written as `used`, has arguments its meaning
    /// allows, if it is one of the checker's (see [Attributes](#attributes)):
    /// `#[retry]` a `times` of at least 0, "`#[retry]` needs a `times` of
    /// at least 0" otherwise, and `#[timeout]` a positive `limit`,
    /// "`#[timeout]` needs a positive `limit`", each at the attribute.
    pub(super) fn fits_builtin(&mut self, attribute: &Attribute, used: &AttributeUse) -> bool {
        let message = match (&attribute.id, attribute.args.first()) {
            (AttributeId::Builtin(RETRY), Some((_, Constant::Int(times)))) if *times < 0 => {
                "`#[retry]` needs a `times` of at least 0"
            }
            (
                AttributeId::Builtin(TIMEOUT),
                Some((_, Constant::Quantity(Quantity::Duration, limit))),
            ) if *limit <= 0 => "`#[timeout]` needs a positive `limit`",
            _ => return true,
        };
        self.errors.push(Diagnostic::error(message, used.span));
        false
    }

    /// The attribute of the file's view of types that `used` names, and its
    /// record type's fields, by name, in its order, each with its type and
    /// default (see [Attributes](#attributes)). `None` for a name that
    /// names none: "unknown attribute `x`", with a suggestion among the
    /// attributes in view and the checker's, or "`T` is not an attribute"
    /// for a type that is not marked `#[attribute]`; and, quietly, for one
    /// whose type is erroneous, which its declaration reports.
    pub(super) fn declared_attribute(
        &mut self,
        used: &AttributeUse,
    ) -> Option<(AttributeId, AttributeFields)> {
        let names = self.ast.names();
        let text = names.get(used.name);
        let lookup = self
            .workspace
            .map_or(AttributeLookup::Nothing, |workspace| {
                workspace.attribute(used.name)
            });
        let id = match lookup {
            AttributeLookup::Attribute(id) => id.clone(),
            AttributeLookup::NotAttribute => {
                self.errors.push(
                    Diagnostic::error(format!("`{text}` is not an attribute"), used.name_span)
                        .with_note("an attribute is a `type` marked `#[attribute]`"),
                );
                return None;
            }
            AttributeLookup::Nothing => {
                let declared = self
                    .workspace
                    .map(Context::attribute_names)
                    .unwrap_or_default();
                let candidates = declared
                    .iter()
                    .map(|&name| (names.get(name), 0))
                    .chain([(RETRY, 1), (TIMEOUT, 1)]);
                let suggestion =
                    suggest::best(text, candidates).map(|best| (used.name_span, best.to_owned()));
                self.errors.push(
                    Diagnostic::error(format!("unknown attribute `{text}`"), used.name_span)
                        .with_suggestion(suggestion),
                );
                return None;
            }
        };
        let ty = self.resolve_type(used.ty, Bareness::Present);
        let TyKind::Record(record) = self.types.kind(ty) else {
            return None;
        };
        let fields = record
            .fields
            .iter()
            .map(|&(name, field)| {
                let default = record
                    .defaults()
                    .iter()
                    .find(|&&(defaulted, _)| defaulted == name)
                    .map(|(_, constant)| constant.clone());
                (Box::from(names.get(name)), field, default)
            })
            .collect();
        Some((id, fields))
    }

    /// The argument `used` gives each of `fields`, the fields of its
    /// attribute's record, by name, in their order: one positional argument
    /// for an attribute of one field, and named arguments for any. `None`,
    /// the error reported, for a positional argument of an attribute of
    /// another number of fields, a second positional one, a named one that
    /// names no field or one named twice, or a trailing block.
    pub(super) fn attribute_args(
        &mut self,
        used: &AttributeUse,
        fields: &AttributeFields,
    ) -> Option<Vec<Option<ExprId>>> {
        let ast = self.ast;
        let text = ast.names().get(used.name);
        let mut given: Vec<Option<ExprId>> = vec![None; fields.len()];
        let mut failed = false;
        for arg in used.args.map_or(&[][..], |args| ast.args(args)) {
            let error = match arg.kind {
                ArgKind::Positional if fields.len() != 1 => Some(format!(
                    "`#[{text}]` takes its arguments by name, as in `#[{text}({} = ...)]`",
                    fields.first().map_or("x", |(name, _, _)| name)
                )),
                ArgKind::Positional => match given.first_mut() {
                    Some(slot @ None) => {
                        *slot = Some(arg.value);
                        None
                    }
                    _ => Some(format!("`#[{text}]` takes one argument")),
                },
                ArgKind::Named { name, .. } => {
                    let field_name = ast.names().get(name);
                    match fields
                        .iter()
                        .position(|(field, _, _)| &**field == field_name)
                    {
                        None => Some(format!("`#[{text}]` has no argument `{field_name}`")),
                        Some(at) => match given.get_mut(at) {
                            Some(slot @ None) => {
                                *slot = Some(arg.value);
                                None
                            }
                            _ => Some(format!("`{field_name}` is given twice")),
                        },
                    }
                }
                ArgKind::Trailing => Some(format!("`#[{text}]` takes no block")),
            };
            if let Some(message) = error {
                self.errors.push(Diagnostic::error(message, arg.span(ast)));
                failed = true;
            }
        }
        (!failed).then_some(given)
    }

    /// The constant that `value`, an argument of an attribute named at
    /// `name`, is, checked against its field's type `ty` as a parameter's
    /// constant default is (see [Attributes](#attributes)): "an attribute's
    /// argument must be a constant", noted with what a constant is, for one
    /// that is not. `None`, the error reported, if it does not fit.
    pub(super) fn attribute_arg(
        &mut self,
        value: ExprId,
        ty: TyId,
        name: Span,
    ) -> Option<Constant> {
        if !self.is_constant(value) {
            self.errors.push(
                Diagnostic::error(
                    "an attribute's argument must be a constant",
                    self.ast.get(value).span,
                )
                .with_note(CONSTANT_NOTE),
            );
            return None;
        }
        let errors = self.errors.len();
        let found = self.check_expr_expecting(value, ty, name, "expected by this attribute");
        if found == TyId::ERROR || self.errors.len() != errors {
            return None;
        }
        self.constant(value)
    }

    /// Whether the expression `id`, a parameter's or a field's default, is a
    /// constant (see "Calls" under
    /// [Functions](crate::check::functions#functions)):
    /// [`crate::ast::is_constant`].
    pub(super) fn is_constant(&self, id: ExprId) -> bool {
        crate::ast::is_constant(self.ast, id)
    }

    /// The constant that `id`, a parameter's or a field's default that
    /// [`Checker::is_constant`] accepts, is once checked without an error
    /// ([`constant_of`]): an integer literal that is a `Float` there a `Float`,
    /// and a record block given fields' defaults with those fields (see "Field
    /// defaults" under [Functions](crate::check::functions#functions)). `None`
    /// if it is not one after all.
    pub(super) fn constant(&self, id: ExprId) -> Option<Constant> {
        constant_of(
            self.ast,
            id,
            (&self.expr_types, &self.lowering.record_fills),
        )
    }

    /// The constant that `constant`, a field's default of a declared type as a
    /// copy of its definition holds it, written with no type known (see
    /// [`written_constant`](super::written_constant)), is where the field is of
    /// type `expected`, as [`Checker::check_expr_expecting`] would check the
    /// default it is written as (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)): `None` if it does not
    /// fit. An `Int` is the `Int` or `Float` that an integer literal adapts to;
    /// a `Float`, `Bool` or `String` must be a subtype; a list is fitted
    /// against the list type expected, or the one list member of the union
    /// expected, or, for one with several, the first of them it fits, as a list
    /// literal checked bottom up would be; a record likewise against a record
    /// type, a field the type lacks kept as it is, and, where the record type
    /// is expected itself and not a union of it, each defaulted field it lacks
    /// given its default, as a record block is; and `{}` is given every default
    /// of a record type with defaults expected itself, as a record block is,
    /// and must be a subtype otherwise. Nothing fits a `Future`.
    ///
    /// Recurses once per level of list and record nesting.
    pub(super) fn fit_constant(&self, constant: &Constant, expected: TyId) -> Option<Constant> {
        if expected == TyId::ERROR || self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let stripped = self.types.strip(expected);
        // The members a list or record is tried against, in turn.
        let members: &[TyId] = match self.types.union_members(stripped) {
            Some(members) => members,
            None => std::slice::from_ref(&stripped),
        };
        match constant {
            Constant::Int(value) => match self.numeric_member(stripped)? {
                TyId::FLOAT => Some(Constant::Float(int_to_float(*value).to_bits())),
                _ => Some(Constant::Int(*value)),
            },
            Constant::Float(_) => self
                .types
                .is_subtype(TyId::FLOAT, expected)
                .then(|| constant.clone()),
            Constant::Bool(_) => self
                .types
                .is_subtype(TyId::BOOL, expected)
                .then(|| constant.clone()),
            &Constant::Quantity(quantity, _) => self
                .types
                .is_subtype(TyId::of_quantity(quantity), expected)
                .then(|| constant.clone()),
            Constant::Str(_) => self
                .types
                .is_subtype(TyId::STRING, expected)
                .then(|| constant.clone()),
            // `{}` is given the defaults of a record type with defaults
            // expected itself, as a record block is, and is checked as ever
            // otherwise.
            Constant::Empty => match self.defaulted_record(expected) {
                Some(record) => self.fit_record(&[], record, true),
                None => self
                    .types
                    .is_subtype(TyId::EMPTY, expected)
                    .then_some(Constant::Empty),
            },
            Constant::List(elements) => members.iter().find_map(|&member| {
                let element = self.types.list_element(member)?;
                elements
                    .iter()
                    .map(|value| self.fit_constant(value, element))
                    .collect::<Option<_>>()
                    .map(Constant::List)
            }),
            Constant::Record(fields) => {
                // Only a record type expected itself fills.
                let fills = members.len() == 1;
                members
                    .iter()
                    .find_map(|&member| self.fit_record(fields, member, fills))
            }
        }
    }

    /// The record constant of `fields` fitted to the record type `record`,
    /// as [`Checker::fit_constant`] says, a field `record` does not name
    /// fitted to its rest-field if it has one, each defaulted field it lacks
    /// given its default if `fills`, after the fields written, in the order
    /// of their names; `None` if `record` is no record type, a field does
    /// not fit, or one it lacks has no default.
    ///
    /// Recurses once per level of list and record nesting.
    pub(super) fn fit_record(
        &self,
        fields: &[(Name, Constant)],
        record: TyId,
        fills: bool,
    ) -> Option<Constant> {
        let expected = self.types.record_fields(record)?;
        let mut fitted = Vec::with_capacity(expected.len().max(fields.len()));
        for (name, value) in fields {
            let value = match self.types.field_or_rest(record, *name) {
                Some(ty) => self.fit_constant(value, ty)?,
                // Checked on its own, an integer settling to `Int`.
                None => value.clone(),
            };
            fitted.push((*name, value));
        }
        let mut filled: Vec<(Name, Constant)> = Vec::new();
        for &(name, _) in expected {
            if fields.iter().any(|&(written, _)| written == name) {
                continue;
            }
            let default = self.types.field_default(record, name).filter(|_| fills)?;
            filled.push((name, default.clone()));
        }
        let names = self.ast.names();
        filled.sort_unstable_by(|(a, _), (b, _)| names.get(*a).cmp(names.get(*b)));
        fitted.extend(filled);
        // `{}` where a record type with a rest-field but no defaulted field
        // is expected is given nothing.
        if fitted.is_empty() {
            return Some(Constant::Empty);
        }
        Some(Constant::Record(fitted.into_boxed_slice()))
    }

    /// Types the field access `id`, `func.name`, which reads the meta `meta` of
    /// `func`, the function `record` evaluates to (see [Metas](#metas)): the
    /// type of the meta's field `name`. Reading it is calling the meta's
    /// closure, of the meta's effect: from a pure function, a `META->` or
    /// `META->>` meta is "cannot read an impure meta from a pure function",
    /// labelled "reading the meta of `func` is impure (`META->`)" at `func` and
    /// at the function's marker, as calling an impure function is. A meta that
    /// may be a future is awaited at the top, subject to the purity rule, as a
    /// record a field access reads is, unless the read is already that error. A
    /// field the meta lacks is "the meta of `func` has no field `x`", at the
    /// `.NAME`, noted with the meta's fields. Recorded for lowering
    /// ([`Resolution::meta_read`](super::Resolution::meta_read)). A def-type
    /// reference, read in parentheses or by a path past its definition, is
    /// named as it is written, "the meta of `package dev.peios.testpkg`", and a
    /// field missing past a definition is noted as read from its meta
    /// (`Checker::meta_of`).
    pub(super) fn meta_read(&mut self, id: ExprId, record: ExprId, meta: Meta, name: Name) -> TyId {
        let MetaOf {
            subject,
            func: func_span,
            read: read_span,
            past,
        } = self.meta_of(id, record);
        let mut impure = false;
        if meta.effect != Effect::Pure
            && let Some(function) = self.functions.last()
            && function.effect == Effect::Pure
        {
            impure = true;
            let marker = function.marker;
            let label = format!(
                "reading {subject} is impure (`{}`)",
                meta.effect.meta_marker()
            );
            self.errors.push(
                Diagnostic::error("cannot read an impure meta from a pure function", read_span)
                    .with_context(func_span, label)
                    .with_context(marker, PURE_CONTEXT)
                    .with_note(
                        "a pure function, declared with `:`, may read only pure metas, `META:`; \
                         declare it with `->` to make it impure",
                    ),
            );
        }
        let mut mark = AwaitMark::None;
        let mut ty = meta.ty;
        if self.types.laziness(ty) != Laziness::Present {
            mark = AwaitMark::Top;
            if !impure && let Some(marker) = self.pure_marker() {
                let error = self.implicit_await_error(read_span, ty, marker, None);
                self.errors.push(error);
            }
            ty = self.types.strip(ty);
        }
        self.meta_reads.insert(id, (meta.ty, mark));
        if let Some(field) = self.types.field(ty, name) {
            return field;
        }
        let expr = self.ast.get(id);
        let member = expr.member_span(self.ast).unwrap_or(expr.span);
        let text = FieldName(self.ast.names().get(name));
        let fields = self.types.record_fields(ty);
        let rest = self.types.record_rest(ty).is_some();
        let note = fields.map_or_else(String::new, |fields| self.field_list(fields, rest));
        let suggestion = fields.and_then(|fields| self.field_suggestion(id, name, fields));
        let mut error = Diagnostic::error(format!("{subject} has no field `{text}`"), member)
            .with_note(note)
            .with_suggestion(suggestion);
        // A path past a definition that may have been meant as a longer
        // definition's.
        if let Some(reference) = past {
            error = error.with_note(format!(
                "`{reference}.{text}` is read as the field `{text}` of the meta of \
                 `{reference}`, its longest prefix that is a definition"
            ));
        }
        self.errors.push(error);
        TyId::ERROR
    }

    /// What the field access `id` reads the meta of, the function `record`
    /// evaluates to, as a meta read's errors show it (see [Metas](#metas)):
    /// "the meta of `func`" for a variable, "the meta of `package
    /// dev.peios.testpkg`" for a def-type reference, in parentheses or read
    /// by a path past its definition ([`Checker::reading_past`]), and "the
    /// meta of this function" for anything else; the span of the function
    /// and of the read, which for a path past a definition start at the
    /// reference's def-type; and for such a path, the reference to the
    /// definition, as written, for the error of a missing field's note.
    pub(super) fn meta_of(&self, id: ExprId, record: ExprId) -> MetaOf {
        let ast = self.ast;
        if let Some((defined, def)) = self.reading_past
            && defined == record
        {
            let start = ast.get(ast.def_type(def).dt).span;
            let (shown, _) = ast.show_def_type(def);
            let mut path = Vec::new();
            let mut at = defined;
            while let ExprKind::Field { expr, name } = ast.get(at).kind {
                path.push(ast.names().get(name));
                at = expr;
            }
            path.reverse();
            let reference = format!("{shown} {}", path.join("."));
            return MetaOf {
                subject: format!("the meta of `{reference}`"),
                func: Span::join(start, ast.get(defined).span),
                read: Span::join(start, ast.get(id).span),
                past: Some(reference),
            };
        }
        let mut inner = record;
        while let ExprKind::Paren(expr) = ast.get(inner).kind {
            inner = expr;
        }
        let subject = match (ast.get(record).kind, ast.get(inner).kind) {
            (ExprKind::Name(func), _) => format!("the meta of `{}`", ast.names().get(func)),
            (_, ExprKind::DefTypeRef(def)) => {
                let (shown, path) = ast.show_def_type(def);
                format!("the meta of `{shown} {path}`")
            }
            _ => "the meta of this function".to_owned(),
        };
        MetaOf {
            subject,
            func: ast.get(record).span,
            read: ast.get(id).span,
            past: None,
        }
    }
}
