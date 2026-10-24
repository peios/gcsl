//! Checking def-types and definitions: a def-type's signature, generic and
//! variant def-types, and the definitions a file makes and reads.
//!
//! # Def-types
//!
//! A `def` marked `#[def_type]` (see the parser's
//! [Attributes](crate::parser::attributes#attributes)) is a *def-type*, as in
//! `#[def_type] def package(name: String, func: fn(): PackageDef) -> R`. Its
//! first parameter must be a `String` and its second a function, its type
//! F, present or the either shape of one: the function type of the
//! *definitions* made with it; its further parameters, if any, are its
//! *invocation parameters*, and its effect and result are free. Otherwise it
//! is "a def-type takes the name as a `String` and then the definition as a
//! function, as in `def package(name: String, func: fn(): T)`", the
//! def-type's own name in place of `package`, at its parameters
//! (`Checker::check_def_type_signature`), noting the shape of a def-type
//! whose definitions are made with a variant (see [Variant
//! def-types](#variant-def-types)). It is still an ordinary `def`,
//! called as any is.
//!
//! **Definitions.** `def DT PATH(PARAMS) BODY` ([`ItemKind::DefTypeDef`])
//! binds nothing a variable or a field names. Its function is a leaf of
//! DT's *namespace*, a root of the tree of top-level fields that no source
//! can write, the namespace's key (see [`crate::names`], "Namespace keys"),
//! which the workspace finds once it knows what DT names (see
//! [`crate::workspace`], "Def-types"). So it is a top-level function in
//! every sense a `def` named by a dotted path is (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)): usable above it and from every file
//! that sees it, in its group, its closure made where the rules say, its
//! leaf's cell holding it. Its type is F: as the input starts, its `DT` is
//! read quietly and its leaf's cell given F (`Checker::definition_type`),
//! after the other dotted `def`s', so that a def-type may be one. Its body
//! is checked (`Checker::check_definition`), its `DT` read again and what
//! is wrong with it reported, as a function of F's parameters, which
//! PARAMS name in order, against F's result, with F's effect: so with
//! `fn(): PackageDef` it is pure and makes a `PackageDef`. Nothing is
//! inferred of it, no parameter, no result, no callback narrowed: its type
//! is F exactly. Parameters not as many as F's are "`package` defines
//! functions of no parameters" ("of 1 parameter, `x`", "of 2 parameters,
//! `a` and `b`", naming them if F names them all), at its parameter list.
//! A definition's parameters of its own, written with a type and a default
//! after F's (see the parser's "Def-types"), are those of its body's
//! function, which its own function calls with their defaults, so that its
//! type is still F's: the body's function is checked with F's result and
//! F's effect, its parameters typed as written, and known by the
//! definition's last segment (`Checker::note_definition_body`); and it
//! takes the definition's attributes, so that a deferring one makes it the
//! step, given what its defaults came to where the def-type called the
//! definition.
//!
//! When the walk reaches a definition (`Checker::place_definition`), it is
//! placed in its namespace as the dotted field of its path under the key is
//! placed in a record, but among the namespaces' own fields, apart from the
//! input's record: so a file of definitions alone has no fields, and two of
//! one path, one under another and the like are what the rules of dotted
//! fields say they are, the path shown as the definition writes it ("field
//! `package x` is defined twice"). One whose DT is no def-type is "`DT` is
//! not a def-type", noted "a def-type is a `def` marked `#[def_type]`", and
//! its body is not checked, having no type to be checked as. A shadowed
//! definition is checked as written (see [Shadowed
//! declarations](crate::check::items#shadowed-declarations)), but one whose def-type it cannot
//! read, its file not otherwise needing the def-type's, is not: its types
//! are the def-type's to give. Where an error names a definition as what
//! cannot be used yet, or along the chain of uses that says why, it is shown
//! by its def-type and its path, as written, "`package dev.peios.x` cannot be
//! used here yet" (`Checker::need_text`).
//!
//! **References.** `DT PATH` ([`ExprKind::DefTypeRef`]) is a function: of
//! the def-type's invocation parameters, with its effect and its result,
//! and the meta of F, if F has one, so that it carries its definition's
//! (see [Metas](crate::check::attributes#metas), "Through a reference")
//! (`Checker::check_def_type_ref`). Its DT is read as any value is, and must
//! name a def-type: "`DT` is not a def-type" otherwise, noted as above and
//! "two names in a row are read as a def-type reference, as in `package
//! dev.x()`; is an operator or a line break missing?", with which the
//! "undefined variable" error of an undefined DT is noted too. A DT that
//! names a parameter or a local nearer than the input's top-level
//! declarations is no def-type, whatever the workspace found its name to
//! name at the top
//! level (`Checker::shadows_def_type`). Its path must name a definition of
//! the namespace that the file sees (`Checker::definition_at`): a leaf of
//! the view under the key, or, in a session, of an earlier input's
//! namespace, a binding of the key, by its record type. A path that names
//! nothing is "`package` has no definition `dev.peios.nope`", with a note
//! listing those the file sees ("the only definition of `package` here is
//! `x`", "the definitions of `package` here are `a` and `b.c`", "`package`
//! has no definitions here"); one that names a node above definitions,
//! "`dev.peios` is not a `package` definition", at the path. For a def-type
//! whose F has a meta, a path that goes on past a definition names it, the
//! rest of the path read from its meta (see [Metas](crate::check::attributes#metas)); for any other
//! the whole path must name one, as here. The
//! definition is then read as its leaf's cell is, by its path
//! (`Checker::field_reference`). A call of a reference is a call of a
//! function value, of no known callee: calling one of an impure def-type
//! from a pure function is the purity error. A record read whole through
//! an import holds no namespace (`Checker::imported_record`).
//!
//! # Variant def-types
//!
//! A def-type whose second parameter is a variant of an enum, of type
//! `Variant<E, T>`, and whose third is the definition, makes its definitions
//! with a variant
//! ([`TypeTable::def_type_shape`](crate::types::TypeTable::def_type_shape)):
//! `#[def_type] def Toolchain<T>(name: String, kind: Variant<Toolchain, T>,
//! func: fn(): ToolchainDef<T>): Registered`. Its further parameters are its
//! invocation parameters, as a def-type's after its second are. Such a def-type
//! may be generic: a definition's variant binds its type parameters, so each
//! must appear in the variant parameter's type, "type parameter `U` of a
//! def-type must appear in its variant parameter's type", at its parameters
//! (`Checker::check_generic_def_type`); a generic def-type of the other shape
//! is one of [Generic results](#generic-results). Its
//! `E` must be an enum, not a type parameter: the variant's name is looked up
//! in it.
//!
//! A definition names its variant after its def-type, `def Toolchain::C gcc()`,
//! and so does a reference, `Toolchain::C gcc`, and a meta, `defmeta
//! Toolchain::C gcc`. Each variant has a namespace of its own (see
//! [`crate::names`], "Namespace keys"), so `Toolchain::C gcc` and
//! `Toolchain::Rust gcc` are two definitions, each shown with its variant. For
//! every use (`Checker::use_signature`), the variant is the one of its name, of
//! the enum `E`, that the input sees, found as a variant as a value is (see
//! [Enums](crate::check::enums#enums)), "no variant `Nope` of `Toolchain`"
//! otherwise; its type, `Variant<E, P>` of its payload `P`, binds the
//! def-type's type parameters
//! ([`TypeTable::bind`](crate::types::TypeTable::bind)), and the def-type's
//! signature is substituted with them: so the definition's function type is
//! `fn(): ToolchainDef<P>`, its body checked against it, and a reference's type
//! is the substituted signature's after its definition's. The variant's type
//! must fit the variant parameter's, substituted: "the variant of a `Shape`
//! definition is a `Variant<Tool, {cc: String}>`, not `Variant<Tool, {rustc:
//! String}>`", at the variant's name. A variant written for a def-type of the
//! other shape is "`package` takes no variant", at it; none written for one of
//! this shape is "def-type `Toolchain` needs a variant: `def Toolchain::V
//! name()`", at the def-type. Since a reference calls the def-type, its `DT` is
//! read as a call's callee (`Checker::check_def_type_expr`): a generic def-type
//! is no generic function read as a value. A reference's variant is recorded
//! for lowering, which calls the def-type with the definition's path, the
//! variant's tag, a constant, the definition and the invocation arguments. A
//! def-type of this shape is not listed to be invoked by name (see
//! [`crate::workspace`], "Def-types by name"): its definitions are reached by
//! references alone.
//!
//! # Generic results
//!
//! A generic def-type whose definitions are made with no variant is generic
//! in its definitions' results: `#[def_type] def build<T>(name: String, f:
//! fn() -> T) -> T`, the prelude's. Each type parameter must appear in the
//! result of its definition parameter's function type, "type parameter `U` of
//! a def-type must appear in its definitions' result type" otherwise, at its
//! parameters (`Checker::check_result_generic`), since a definition's body
//! binds them and nothing else. A definition's body is checked against no
//! result (`Checker::check_definition`), and the definition is a function of
//! its parameters' types, the def-type's for it, and of the type its body
//! gives: `def build hello() { out = f }` is a `fn() -> {out: File}`. A
//! reference binds the def-type's type parameters by its definition's type
//! against the definition parameter's ([`TypeTable::bind`](crate::types::TypeTable::bind),
//! `Checker::bound_signature`), so `build hello` is called with the
//! def-type's signature so bound, and gives what its definition does.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    DefTypeId, DefTypeRole, Effect, ExprId, ExprKind, FnId, ItemId, ItemKind, PathSegment,
    SegmentKind, path_span,
};
use crate::diagnostic::Diagnostic;
use crate::mentions::Resolve;
use crate::names::Name;
use crate::source::Span;
use crate::types::{DefTypeShape, FnTy, GenericId, Meta, TyId, TyKind};

use super::{
    Attribute, Checker, DefinitionAt, Function, JUXTAPOSED_NOTE, KnownCallee, NO_NODE, Slot,
    UseSignature, VarRef, VariantTag,
};

/// What the checker keeps of the definitions made with def-types, and of
/// the def-types' own functions, of the input's outermost block (see
/// [Def-types](#def-types)).
#[derive(Debug, Default)]
pub(super) struct DefTypesChecked {
    /// The definition each function expression is, if it is one.
    pub(super) definitions: HashMap<FnId, DefTypeId>,
    /// The function expressions of the `def`s marked `#[def_type]`.
    pub(super) fns: HashSet<FnId>,
    /// The function of the body of each definition with parameters of its
    /// own, as its definition's check finds it, with the result type and the
    /// effect its def-type gives it and the name it is known by, its
    /// definition's.
    pub(super) bodies: HashMap<FnId, (TyId, Effect, Option<Name>)>,
    /// The definition's meta each function expression is, if it is one (see
    /// [Metas](crate::check::attributes#metas)).
    pub(super) metas: HashMap<FnId, DefTypeId>,
}

impl Checker<'_> {
    /// As the input starts, once the cells are declared: gives the cell of
    /// each `def` named by a dotted path the type its signature declares,
    /// which a use of it sees until its group is checked, as a plain
    /// function's binding has (see [Top-level
    /// functions](crate::check::toplevel#top-level-functions)).
    pub(super) fn type_dotted_defs(&mut self) {
        // A definition made with a def-type has the type the def-type gives
        // it, which may be a dotted `def` of the input: those after the
        // others (see [Def-types](#def-types)).
        for definitions in [false, true] {
            for index in 0..self.top_fns.len() {
                let Some(top) = self.top_fns.get(index).copied() else {
                    continue;
                };
                let Some(leaf) = top.leaf else {
                    continue;
                };
                let defmeta = self.def_types.metas.get(&top.function).copied();
                let ty = match (
                    self.def_types.definitions.get(&top.function),
                    defmeta,
                    definitions,
                ) {
                    (Some(&def), _, true) => self.definition_type(def),
                    // A meta is the function its def-type's meta makes.
                    (None, Some(def), true) => self.defmeta_type(def),
                    (None, None, false) => self.declared_type(top.function),
                    _ => continue,
                };
                if let Some(cell) = self.field_cells.get_mut(leaf as usize) {
                    cell.ty = Some(ty);
                }
            }
        }
    }

    /// As the input starts: finds which function expressions of its
    /// outermost block are definitions made with a def-type, which are
    /// definitions' metas, and which are `def`s marked `#[def_type]` (see
    /// [Def-types](#def-types)).
    pub(super) fn find_def_types(&mut self) {
        let ast = self.ast;
        for &id in ast.items(ast.root()) {
            match ast.item(id).kind {
                ItemKind::DefTypeDef { def, value } => {
                    if let ExprKind::Fn(function) = ast.get(value).kind {
                        self.def_types.definitions.insert(function, def);
                    }
                }
                ItemKind::DefMeta { def, value } => {
                    if let ExprKind::Fn(function) = ast.get(value).kind {
                        self.def_types.metas.insert(function, def);
                    }
                }
                ItemKind::Field {
                    value,
                    function: true,
                    ..
                } if ast.is_def_type(id) => {
                    if let ExprKind::Fn(function) = ast.get(value).kind {
                        self.def_types.fns.insert(function);
                    }
                }
                _ => {}
            }
        }
    }

    /// The type of the definition `def` made with a def-type, before its
    /// body is checked: the function type the def-type's second parameter
    /// has, read quietly, or `Error` if its def-type is not one (which its
    /// own check reports).
    pub(super) fn definition_type(&mut self, def: DefTypeId) -> TyId {
        let (errors, warnings) = (self.errors.len(), self.warnings.len());
        let dt = self.ast.def_type(def).dt;
        let ty = self.check_def_type_expr(dt);
        let definition = self
            .use_signature(def, ty)
            .map_or(TyId::ERROR, |found| found.definition);
        self.errors.truncate(errors);
        self.warnings.truncate(warnings);
        definition
    }

    /// The type of the definition's meta `def`, before its body is checked:
    /// the function of no parameters, of the effect of its marker, returning
    /// its type, that the meta of the function type its def-type gives its
    /// definitions makes, read quietly; `Error` if its def-type is not one or
    /// declares no meta, which its own check reports (see [Metas](crate::check::attributes#metas)).
    pub(super) fn defmeta_type(&mut self, def: DefTypeId) -> TyId {
        let function = self.definition_type(def);
        match self.types.fn_sig(function).and_then(|sig| sig.meta) {
            Some(meta) => {
                let none: [(Option<Name>, TyId); 0] = [];
                self.types.fn_type(none, meta.ty, meta.effect)
            }
            None => TyId::ERROR,
        }
    }

    /// What the def-type of type `dt_ty` gives its use `def`, if it is the type
    /// of a def-type
    /// ([`TypeTable::def_type_shape`](crate::types::TypeTable::def_type_shape)):
    /// the function type of its definitions, present, and its signature, with
    /// the variant the use names, if its definitions are made with one, and
    /// that variant's payload binding its type parameters (see [Variant
    /// def-types](#variant-def-types)). Reports, at the use, a variant written
    /// for a def-type that takes none, "`package` takes no variant", none
    /// written for one that does, "`Toolchain` needs a variant", what
    /// [`Checker::find_variant`] does not find, and a variant whose type is not
    /// its parameter's. `None` for what is reported, and for a `dt_ty` that is
    /// no def-type's, which its declaration reports.
    pub(super) fn use_signature(&mut self, def: DefTypeId, dt_ty: TyId) -> Option<UseSignature> {
        let shape = self.types.def_type_shape(dt_ty)?;
        let sig = self.types.fn_sig(self.types.strip(dt_ty))?.clone();
        let use_ = *self.ast.def_type(def);
        let (shown, _) = self.ast.show_def_type(def);
        let (sig, tag) = match (shape.variant, use_.variant) {
            // A generic one's definitions bind its type parameters by their
            // bodies (see [Generic results](#generic-results)).
            (None, None) => (sig, None),
            (None, Some((_, at))) => {
                let dt: Vec<&str> = self
                    .ast
                    .def_type_names(def)
                    .into_iter()
                    .map(|name| self.ast.names().get(name))
                    .collect();
                self.errors.push(
                    Diagnostic::error(format!("`{}` takes no variant", dt.join(".")), at)
                        .with_note(format!(
                            "its definitions are written `def {} NAME()`",
                            dt.join(".")
                        )),
                );
                return None;
            }
            (Some(_), None) => {
                self.errors.push(Diagnostic::error(
                    format!("def-type `{shown}` needs a variant: `def {shown}::V name()`"),
                    self.ast.get(use_.dt).span,
                ));
                return None;
            }
            (Some(at), Some((name, name_span))) => {
                let (sig, tag) = self.variant_signature(&sig, at, (name, name_span), &shown)?;
                (sig, Some(tag))
            }
        };
        let &(_, definition) = sig.params.get(shape.function)?;
        Some(UseSignature {
            definition: self.types.strip(definition),
            sig,
            shape,
            tag,
        })
    }

    /// The signature `sig` of a def-type whose definitions are made with a
    /// variant, shown `shown`, its parameter at `at` the variant's, for a
    /// use that names the variant `name`, at `name_span`: the variant of the
    /// enum of that parameter's type the input sees, and `sig` with the type
    /// parameters it mentions bound by the variant's type, if the def-type
    /// is generic. Reports, at the name, the variant not found and a variant
    /// whose type is not the parameter's, "the variant of a `Shape`
    /// definition is a `Variant<Tool, {cc: String}>`, not
    /// `Variant<Tool, {rustc: String}>`".
    pub(super) fn variant_signature(
        &mut self,
        sig: &FnTy,
        at: usize,
        (name, name_span): (Name, Span),
        shown: &str,
    ) -> Option<(FnTy, VariantTag)> {
        let &(_, param) = sig.params.get(at)?;
        let TyKind::Variant { of, .. } = *self.types.kind(self.types.strip(param)) else {
            return None;
        };
        let TyKind::Enum {
            key,
            name: enum_name,
        } = *self.types.kind(of)
        else {
            return None;
        };
        let (payload, tag) = self.find_variant((key, enum_name), name, name_span)?;
        let given = self.types.variant(of, payload);
        let sig = match sig.generic {
            None => sig.clone(),
            // Each is bound: the def-type's declaration says so.
            Some(_) => self.bound_signature(sig, param, given)?,
        };
        let &(_, wanted) = sig.params.get(at)?;
        if !self.types.is_subtype(given, wanted) {
            let shown_variant = self.in_message(given);
            let wanted = self.in_message(wanted);
            let dt = shown.rsplit_once("::").map_or(shown, |(dt, _)| dt);
            self.errors.push(Diagnostic::error(
                format!("the variant of a `{dt}` definition is a {wanted}, not {shown_variant}"),
                name_span,
            ));
            return None;
        }
        Some((sig, tag))
    }

    /// The signature `sig` of a generic def-type with its type parameters
    /// bound by `given`, the type of what is given for its parameter of type
    /// `param`: `None` if `sig` is not generic or one is left unbound.
    pub(super) fn bound_signature(&mut self, sig: &FnTy, param: TyId, given: TyId) -> Option<FnTy> {
        let generic = sig.generic?;
        let mut bound = vec![None; self.types.generic_names(generic).len()];
        self.types.bind(param, given, generic, &mut bound);
        let bindings: Option<Vec<TyId>> = bound.into_iter().collect();
        let bindings = bindings?;
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

    /// The signature `sig` of a generic def-type whose parameters are where
    /// `shape` says, bound by `definition`, the type of one of its
    /// definitions (see [Generic results](#generic-results)).
    pub(super) fn bound_by_definition(
        &mut self,
        sig: &FnTy,
        shape: DefTypeShape,
        definition: TyId,
    ) -> Option<FnTy> {
        let &(_, param) = sig.params.get(shape.function)?;
        self.bound_signature(sig, param, definition)
    }

    /// When the walk of the outermost block reaches the item `id`, the
    /// top-level function `index`, a `def` named by a dotted path: adds its
    /// path to the record of the input as the dotted field it is, its leaf
    /// typed as its function is by now, which its group's check settles if
    /// it has not yet (`Checker::finish_group`), and gives the item its
    /// leaf's cell, which its closure is stored in.
    pub(super) fn reach_dotted_def(&mut self, index: usize, id: ItemId) {
        let before = self.nodes.len();
        match self.ast.item(id).kind {
            ItemKind::Field { path, value, .. } => {
                let segments = self.ast.path(path);
                if let Some(first) = segments.first().and_then(PathSegment::name) {
                    self.check_name_clash(id, first);
                }
                self.check_dotted_field(id, segments, (None, value), None);
            }
            // A definition made with a def-type: the dotted field of its
            // path under its namespace's key, apart from the input's record;
            // a definition's meta likewise, under its meta key (see
            // [Def-types](#def-types)).
            ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } => {
                self.place_definition(id, def, value);
            }
            _ => return,
        }
        let Some(top) = self.top_fns.get_mut(index) else {
            return;
        };
        // The leaf is the last node the path added.
        if self.nodes.len() > before {
            top.node = u32::try_from(self.nodes.len() - 1).unwrap_or(NO_NODE);
        }
        let slot = top
            .leaf
            .and_then(|leaf| self.field_cells.get(leaf as usize))
            .map_or(Slot::NONE, |cell| cell.slot);
        self.item_slots[id.index()] = slot;
    }

    /// When the walk of the outermost block reaches the definition `id`,
    /// made with a def-type, `def`, of function `value`: places it in its
    /// def-type's namespace as the dotted field of its path under the
    /// namespace's key is placed in a record, among the namespaces' own
    /// fields ([`Checker::def_fields`]), never the input's record, so that
    /// what the rules of dotted fields reject is reported as they report it
    /// ("field `package x` is defined twice"), its value then checked on its
    /// own; or, if its `DT` is no def-type, says so (see
    /// [Def-types](#def-types)).
    pub(super) fn place_definition(&mut self, id: ItemId, def: DefTypeId, value: ExprId) {
        let Some(segments) = self.definition_path(def) else {
            let dt = self.ast.def_type(def).dt;
            if self.check_def_type_expr(dt) != TyId::ERROR {
                self.not_a_def_type(def);
            }
            return;
        };
        let fields = std::mem::replace(&mut self.fields, std::mem::take(&mut self.def_fields));
        let start = std::mem::replace(&mut self.fields_start, 0);
        let spreads = std::mem::take(&mut self.spreads);
        self.check_dotted_field(id, &segments, (None, value), None);
        self.def_fields = std::mem::replace(&mut self.fields, fields);
        self.fields_start = start;
        self.spreads = spreads;
    }

    /// The path of the definition `def` made with a def-type under its
    /// def-type's namespace: the namespace's key, written where its `DT` is,
    /// then its path; for a definition's meta, its leaf, the meta key,
    /// written where its `DT` is, then its path as one name, written where
    /// its path is; `None` if its `DT` is no def-type.
    pub(super) fn definition_path(&self, def: DefTypeId) -> Option<Vec<PathSegment>> {
        let use_ = self.ast.def_type(def);
        if use_.role == DefTypeRole::Meta {
            let (key, leaf) = use_.meta?;
            let path = path_span(self.ast.path(use_.path));
            return Some(vec![
                PathSegment {
                    kind: SegmentKind::Name(key),
                    span: self.ast.get(use_.dt).span,
                },
                PathSegment {
                    kind: SegmentKind::Name(leaf),
                    span: path,
                },
            ]);
        }
        let key = PathSegment {
            kind: SegmentKind::Name(use_.key?),
            span: self.ast.get(use_.dt).span,
        };
        Some(
            std::iter::once(key)
                .chain(self.ast.path(use_.path).iter().copied())
                .collect(),
        )
    }

    /// Reports the def-type definition or reference `def`, whose `DT` names
    /// something that is not a def-type: "`package` is not a def-type", at
    /// its `DT`.
    pub(super) fn not_a_def_type(&mut self, def: DefTypeId) {
        let (shown, _) = self.ast.show_def_type(def);
        let use_ = self.ast.def_type(def);
        let at = self.ast.get(use_.dt).span;
        let mut error = Diagnostic::error(format!("`{shown}` is not a def-type"), at)
            .with_note("a def-type is a `def` marked `#[def_type]`");
        // A reference may be two names that meant something else.
        if use_.read.is_some() {
            error = error.with_note(JUXTAPOSED_NOTE);
        }
        self.errors.push(error);
    }

    /// Types the function expression `id`, of function `function`, the
    /// definition `def` made with a def-type, and checks its body (see
    /// [Def-types](#def-types)): its type is the function type of its
    /// def-type's second parameter, present, whose parameters' types its
    /// parameters, written by name alone, take in order, whose result its
    /// body is checked against, and whose effect it has; a count of
    /// parameters other than that type's is "`package` defines functions of
    /// no parameters" (or "of 2 parameters, `a` and `b`"), at its parameter
    /// list. `Error` if its def-type is not one, which its own check
    /// reports, or the count is wrong.
    pub(super) fn check_definition(&mut self, id: ExprId, function: FnId, def: DefTypeId) -> TyId {
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        let use_ = *self.ast.def_type(def);
        let errors = self.errors.len();
        let dt_ty = self.check_def_type_expr(use_.dt);
        // A shadowed definition needs nothing, so its def-type, read where
        // its file does not otherwise need the def-type's, may not be had:
        // then it is not checked, its types being the def-type's to give
        // (see [Shadowed declarations](crate::check::items#shadowed-declarations)).
        if self.checking_shadowed && dt_ty == TyId::ERROR {
            self.errors.truncate(errors);
            return TyId::ERROR;
        }
        let signature = self.use_signature(def, dt_ty).and_then(|found| {
            let sig = self.types.fn_sig(found.definition)?.clone();
            let generic = found.sig.generic.is_some();
            Some((found.definition, sig, generic))
        });
        let Some((ty, sig, generic)) = signature else {
            return TyId::ERROR;
        };
        // A generic def-type's definition gives its own result, whatever its
        // body is (see [Generic results](#generic-results)).
        let ret = if generic { TyId::ERROR } else { sig.ret };
        let params = self.ast.params(decl.params);
        if params.len() != sig.params.len() {
            self.definition_arity(def, &sig);
            return TyId::ERROR;
        }
        let base = self.fn_params.len();
        for (param, &(_, param_ty)) in params.iter().zip(sig.params.iter()) {
            self.fn_params.push((Some(param.name), param_ty));
        }
        self.expr_types[id.index()] = ty;
        let span = self.ast.get(id).span;
        let dt_span = self.ast.get(use_.dt).span;
        if let Some(body) = use_.body {
            self.note_definition_body(body, (ret, sig.effect), def);
        }
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: sig.effect,
            marker: dt_span,
            keyword: dt_span,
            demanded: vec![None; params.len()],
            result: Some((ret, dt_span)),
            returns: Vec::new(),
            with_depth: self.contexts.bound.len(),
            needs: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        // Known by its path's last segment, as a host names its steps.
        let named = self.ast.path(use_.path).last().and_then(PathSegment::name);
        self.check_fn_body(&decl, base, (named, ty, span), ret);
        let ty = if generic {
            let found = self.expr_types[decl.body.index()];
            let params = sig.params.iter().copied();
            let made = self
                .types
                .fn_type_with_meta(params, found, sig.effect, sig.meta);
            self.expr_types[id.index()] = made;
            made
        } else {
            ty
        };
        self.fn_params.truncate(base);
        self.inherit_attributes(function, use_.dt);
        // A definition with parameters of its own gives its attributes to
        // its body's function, which is then the step.
        let deferred = match use_.body {
            Some(body) => {
                if let Some(attributes) = self.lowering.fn_attributes.remove(&function) {
                    self.lowering.fn_attributes.insert(body, attributes);
                }
                body
            }
            None => function,
        };
        self.definition_defers(deferred, sig.effect, def);
        // Its meta's cell, read as its body reads a top-level function.
        self.definition_meta(function, def, sig.meta);
        let (_, needs) = self.finish_function(decl.params.len(), named);
        let allowed = self.types.need_set(sig.needs).to_vec();
        self.definition_needs(&needs, &allowed);
        ty
    }

    /// Records `body`, the function of the body of the definition `def`,
    /// which has parameters of its own, as one whose result type is `ret`
    /// and whose effect is `effect`, its def-type's, and whose name is the
    /// definition's last segment, for its check (see [Def-types](#def-types)).
    fn note_definition_body(&mut self, body: FnId, (ret, effect): (TyId, Effect), def: DefTypeId) {
        let use_ = *self.ast.def_type(def);
        let name = self.ast.path(use_.path).last().and_then(PathSegment::name);
        self.def_types.bodies.insert(body, (ret, effect, name));
    }

    /// Records the definition `def`, of function `function`, whose function
    /// type's effect is `effect`, as a function whose calls a host may run as
    /// steps, if an attribute it has, its own or its def-type's, defers (see
    /// [Deferring attributes](crate::check::attributes#deferring-attributes)):
    /// the definition, which its def-type's body calls, is the step, and
    /// that body is not, so a def-type such as `test` awaits its definition's
    /// result and recovers its failure outside it. A definition of a pure
    /// function type cannot defer: "a `check` definition cannot defer: its
    /// def-type gives it a pure function type", at its def-type.
    fn definition_defers(&mut self, function: FnId, effect: Effect, def: DefTypeId) {
        let defers = self
            .lowering
            .fn_attributes
            .get(&function)
            .is_some_and(|attributes| attributes.iter().any(|attribute| attribute.defers));
        if !defers {
            return;
        }
        if effect == Effect::Pure {
            let use_ = *self.ast.def_type(def);
            let span = self.ast.get(use_.dt).span;
            let (shown, _) = self.ast.show_def_type(def);
            self.errors.push(
                Diagnostic::error(
                    format!(
                        "a `{shown}` definition cannot defer: its def-type gives it a pure function \
                         type"
                    ),
                    span,
                )
                .with_note(
                    "a deferring attribute lets a host run the definition's calls as steps, \
                     which only an impure function's can be: declare the def-type's function \
                     parameter with `->`",
                ),
            );
            return;
        }
        self.lowering.deferred.insert(function);
    }

    /// Gives the definition of function `function` the attributes of its
    /// def-type, the function `dt` names, as defaults (see
    /// [Attributes](crate::check::attributes#attributes)): the def-type's, in its order, each
    /// replaced whole by the definition's own of the same attribute, then
    /// the definition's others, in theirs. A def-type known by no tree of
    /// this input is known by what its cell carries ([`KnownParams`](super::KnownParams)).
    pub(super) fn inherit_attributes(&mut self, function: FnId, dt: ExprId) {
        let inherited: Vec<Attribute> = match self.known_callee(dt) {
            Some(KnownCallee::Current(def_type)) => self
                .lowering
                .fn_attributes
                .get(&def_type)
                .map(|a| a.to_vec())
                .unwrap_or_default(),
            Some(KnownCallee::Earlier(index)) => self
                .scope_known
                .get(index)
                .and_then(Option::as_ref)
                .map(|known| known.attributes.to_vec())
                .unwrap_or_default(),
            Some(KnownCallee::Imported(node)) => self
                .workspace
                .and_then(|workspace| workspace.known(node))
                .map(|known| known.attributes.to_vec())
                .unwrap_or_default(),
            // A method is never a def-type.
            Some(KnownCallee::Method(..)) | None => Vec::new(),
        };
        if inherited.is_empty() {
            return;
        }
        let own = self
            .lowering
            .fn_attributes
            .remove(&function)
            .unwrap_or_default();
        let mut merged: Vec<Attribute> = inherited
            .into_iter()
            .map(|default| {
                own.iter()
                    .find(|attribute| attribute.id == default.id)
                    .cloned()
                    .unwrap_or(default)
            })
            .collect();
        for attribute in &own {
            if !merged.iter().any(|kept| kept.id == attribute.id) {
                merged.push(attribute.clone());
            }
        }
        self.lowering
            .fn_attributes
            .insert(function, merged.into_boxed_slice());
    }

    /// Resolves the read of its meta of the definition `def` made with a
    /// def-type, of function `function`, whose def-type declares `meta` for
    /// its definitions, if it does (see [Metas](crate::check::attributes#metas)): the cell of the
    /// meta of its path that its directory declares, read as a reference to
    /// it is, whose slot the definition's code names, so that reading the
    /// meta of a closure of it calls the closure in it. A definition whose
    /// directory declares none is "`package dev.peios.x` has no `defmeta` in
    /// its directory", at its def-type and path, noted "`package` declares a
    /// meta, `META-> PackageMeta`, for each of its definitions". The read is
    /// typed `{}`, and nothing else is done, when the def-type declares no
    /// meta, when the directory declares none, and for a shadowed
    /// definition, which is never made.
    pub(super) fn definition_meta(&mut self, function: FnId, def: DefTypeId, meta: Option<Meta>) {
        let use_ = *self.ast.def_type(def);
        let Some(read) = use_.meta_read else {
            return;
        };
        let unread = |checker: &mut Self| {
            let mut at = read;
            loop {
                checker.expr_types[at.index()] = TyId::EMPTY;
                match checker.ast.get(at).kind {
                    ExprKind::Field { expr, .. } => at = expr,
                    _ => break,
                }
            }
        };
        let Some(meta) = meta else {
            unread(self);
            return;
        };
        let unpaired = use_.meta.is_none()
            || self
                .workspace
                .is_some_and(|workspace| workspace.unpaired(def));
        if !unpaired && self.checking_shadowed {
            unread(self);
            return;
        }
        let read_ty = if unpaired {
            None
        } else {
            self.field_reference(read)
        };
        let slot = match self.expr_vars[read.index()] {
            VarRef::Local(slot) | VarRef::Fn(slot) => Some(slot),
            _ => None,
        };
        if let (Some(ty), Some(slot)) = (read_ty, slot) {
            self.expr_types[read.index()] = ty;
            self.metas.push((function, slot));
        } else {
            unread(self);
            self.no_defmeta(def, meta);
        }
    }

    /// Reports the definition `def` made with a def-type whose def-type
    /// declares `meta` for its definitions and whose directory declares none
    /// for it: "`package dev.peios.x` has no `defmeta` in its directory", at
    /// its def-type and path, noted "`package` declares a meta, `META->
    /// PackageMeta`, for each of its definitions", the meta as the def-type's
    /// declaration writes it if the workspace has it, else as its type shows
    /// (see [Metas](crate::check::attributes#metas)).
    pub(super) fn no_defmeta(&mut self, def: DefTypeId, meta: Meta) {
        let use_ = *self.ast.def_type(def);
        let (shown, path) = self.ast.show_def_type(def);
        let at = Span::join(
            self.ast.get(use_.dt).span,
            path_span(self.ast.path(use_.path)),
        );
        let written = use_.key.and_then(|key| {
            self.workspace
                .and_then(|workspace| workspace.meta_written(key, self.ast.names()))
        });
        let written = written.unwrap_or_else(|| {
            // Bare as written, its either shapes those the marker makes.
            let ty = if meta.effect == Effect::Pure {
                meta.ty
            } else {
                self.types.present(meta.ty)
            };
            let ty = self.types.display(ty, self.ast.names()).to_string();
            format!("{} {ty}", meta.effect.meta_marker())
        });
        self.errors.push(
            Diagnostic::error(
                format!("`{shown} {path}` has no `defmeta` in its directory"),
                at,
            )
            .with_note(format!(
                "`{shown}` declares a meta, `{written}`, for each of its definitions"
            )),
        );
    }

    /// Types the function expression `id`, of function `function`, the
    /// definition's meta `def`, and checks its body (see [Metas](crate::check::attributes#metas)):
    /// its type is the function of no parameters, of the effect of its
    /// marker, returning its type, that the meta its def-type declares for
    /// its definitions makes, which its body is checked against, as the
    /// body of such a function. A def-type that declares none is
    /// "`package` declares no meta for its definitions", at the meta's
    /// `defmeta` through its path. `Error` if its def-type is not one, which
    /// its own check reports, or declares no meta.
    pub(super) fn check_defmeta(&mut self, id: ExprId, function: FnId, def: DefTypeId) -> TyId {
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        let use_ = *self.ast.def_type(def);
        let dt_ty = self.check_def_type_expr(use_.dt);
        let Some(UseSignature { definition, .. }) = self.use_signature(def, dt_ty) else {
            return TyId::ERROR;
        };
        let Some(meta) = self.types.fn_sig(definition).and_then(|sig| sig.meta) else {
            let (shown, _) = self.ast.show_def_type(def);
            self.errors.push(
                Diagnostic::error(
                    format!("`{shown}` declares no meta for its definitions"),
                    use_.span,
                )
                .with_note(
                    "a def-type declares one by its definition parameter's type, as in `func: \
                     fn(): PackageDef META: PackageMeta`",
                ),
            );
            return TyId::ERROR;
        };
        let none: [(Option<Name>, TyId); 0] = [];
        let ty = self.types.fn_type(none, meta.ty, meta.effect);
        self.expr_types[id.index()] = ty;
        let span = self.ast.get(id).span;
        let dt_span = self.ast.get(use_.dt).span;
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: meta.effect,
            marker: dt_span,
            keyword: dt_span,
            demanded: Vec::new(),
            result: Some((meta.ty, dt_span)),
            returns: Vec::new(),
            with_depth: self.contexts.bound.len(),
            needs: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let base = self.fn_params.len();
        self.check_fn_body(&decl, base, (None, ty, span), meta.ty);
        let (_, needs) = self.finish_function(0, None);
        self.definition_needs(&needs, &[]);
        ty
    }

    /// Reports the definition `def` made with a def-type, whose parameters
    /// are not as many as those of `sig`, the function type its def-type
    /// gives it: "`package` defines functions of no parameters", "of 1
    /// parameter, `x`" or "of 2 parameters, `a` and `b`", naming them if the
    /// type names them all, at its parameter list.
    pub(super) fn definition_arity(&mut self, def: DefTypeId, sig: &FnTy) {
        let (shown, _) = self.ast.show_def_type(def);
        let count = sig.params.len();
        let param_names: Option<Vec<String>> = sig
            .params
            .iter()
            .map(|&(name, _)| Some(format!("`{}`", self.ast.names().get(name?))))
            .collect();
        let counted = match count {
            0 => "no parameters".to_owned(),
            1 => "1 parameter".to_owned(),
            _ => format!("{count} parameters"),
        };
        let named = match param_names.as_deref() {
            Some([only]) => format!(", {only}"),
            Some([init @ .., last]) => format!(", {} and {last}", init.join(", ")),
            _ => String::new(),
        };
        let at = self.ast.def_type(def).span;
        self.errors.push(Diagnostic::error(
            format!("`{shown}` defines functions of {counted}{named}"),
            at,
        ));
    }

    /// Reports the `#[def_type]` `def` of function `function`, of type
    /// `ty`, if its parameters are not those of a def-type: "a def-type takes
    /// the name as a `String` and then the definition as a function, as in
    /// `def package(name: String, func: fn(): T)`", at its parameters (see
    /// [Def-types](#def-types)).
    pub(super) fn check_def_type_signature(&mut self, function: FnId, ty: TyId) {
        if !self.def_types.fns.contains(&function) || ty == TyId::ERROR {
            return;
        }
        if let Some(shape) = self.types.def_type_shape(ty) {
            self.check_generic_def_type(function, ty, shape);
            return;
        }
        let decl = *self.ast.fn_decl(function);
        let params = self.ast.params(decl.params);
        let at = match (params.first(), params.last()) {
            (Some(first), Some(last)) => Span::join(first.span, last.span),
            _ => self.ast.get(decl.body).span,
        };
        let name = self
            .ast
            .items(self.ast.root())
            .iter()
            .find_map(|&item| match self.ast.item(item).kind {
                ItemKind::Field { path, value, .. }
                    if self.ast.get(value).kind == ExprKind::Fn(function) =>
                {
                    Some(self.ast.show_path(self.ast.path(path)).to_string())
                }
                _ => None,
            })
            .unwrap_or_default();
        self.errors.push(
            Diagnostic::error(
                format!(
                    "a def-type takes the name as a `String` and then the definition as a \
                     function, as in `def {name}(name: String, func: fn(): T)`"
                ),
                at,
            )
            .with_note(
                "or, for definitions made with a variant, the name, the variant and the \
                 definition, as in `def Toolchain<T>(name: String, kind: Variant<Toolchain, T>, \
                 func: fn(): T)`",
            ),
        );
    }

    /// Reports the def-type of function `function`, of type `ty`, whose
    /// parameters are where `shape` says, if it is generic and its
    /// definitions are made with no variant, "a generic def-type takes its
    /// definitions' variant after the name", or a type parameter of it is
    /// one its variant parameter's type does not mention, "type parameter
    /// `U` of a def-type must appear in its variant parameter's type", at
    /// its type parameters: a definition's variant binds them, and nothing
    /// else (see [Variant def-types](#variant-def-types)).
    pub(super) fn check_generic_def_type(&mut self, function: FnId, ty: TyId, shape: DefTypeShape) {
        let Some(sig) = self.types.fn_sig(self.types.strip(ty)).cloned() else {
            return;
        };
        let Some(generic) = sig.generic else {
            return;
        };
        let decl = *self.ast.fn_decl(function);
        let params = self.ast.params(decl.params);
        let at = match (params.first(), params.last()) {
            (Some(first), Some(last)) => Span::join(first.span, last.span),
            _ => self.ast.get(decl.body).span,
        };
        let Some(variant) = shape.variant.and_then(|at| sig.params.get(at)) else {
            self.check_result_generic(&sig, generic, shape, at);
            return;
        };
        let (variant_name, variant_ty) = *variant;
        let names = self.types.generic_names(generic).to_vec();
        let mut bound = vec![None; names.len()];
        self.types.bind(variant_ty, variant_ty, generic, &mut bound);
        let kind = variant_name.map_or("kind", |name| self.ast.names().get(name));
        for (&name, binding) in names.iter().zip(&bound) {
            if binding.is_none() {
                self.errors.push(
                    Diagnostic::error(
                        format!(
                            "type parameter `{}` of a def-type must appear in its variant \
                             parameter's type",
                            self.ast.names().get(name)
                        ),
                        at,
                    )
                    .with_note(format!(
                        "a definition's variant, given as `{kind}`, binds the def-type's type \
                         parameters"
                    )),
                );
            }
        }
    }

    /// Reports each type parameter of the generic def-type of signature
    /// `sig` whose definitions are made with no variant that its definition
    /// parameter's result does not mention, "type parameter `U` of a def-type
    /// must appear in its definitions' result type", at `at`, its
    /// parameters: a definition's body binds them, and nothing else (see
    /// [Generic results](#generic-results)).
    pub(super) fn check_result_generic(
        &mut self,
        sig: &FnTy,
        generic: GenericId,
        shape: DefTypeShape,
        at: Span,
    ) {
        let Some(&(_, definition)) = sig.params.get(shape.function) else {
            return;
        };
        let result = self
            .types
            .fn_sig(self.types.strip(definition))
            .map_or(TyId::ERROR, |f| f.ret);
        let names = self.types.generic_names(generic).to_vec();
        let mut bound = vec![None; names.len()];
        self.types.bind(result, result, generic, &mut bound);
        for (&name, binding) in names.iter().zip(&bound) {
            if binding.is_none() {
                self.errors.push(
                    Diagnostic::error(
                        format!(
                            "type parameter `{}` of a def-type must appear in its definitions' \
                             result type",
                            self.ast.names().get(name)
                        ),
                        at,
                    )
                    .with_note(
                        "a definition's body binds the def-type's type parameters; one written \
                         with a variant, `def Toolchain::C gcc()`, binds them by the variant",
                    ),
                );
            }
        }
    }

    /// Types the def-type reference `id`, `DT PATH`, of `def` (see
    /// [Def-types](#def-types)): the function that calls the def-type with
    /// the definition's path, the definition and its own arguments, of the
    /// type of the def-type's parameters after its second, its result and
    /// its effect. Its `DT` must name a def-type, "`package` is not a
    /// def-type" otherwise, and its path a definition of the def-type's
    /// namespace that the file sees, "`package` has no definition
    /// `dev.peios.nope`", with a note listing those it sees, or, for a path
    /// above some, "`dev.peios` is not a `package` definition", at the path.
    pub(super) fn check_def_type_ref(&mut self, id: ExprId, def: DefTypeId) -> TyId {
        let use_ = *self.ast.def_type(def);
        let errors = self.errors.len();
        let dt_ty = self.check_def_type_expr(use_.dt);
        if dt_ty == TyId::ERROR {
            // Two names in a row, the first undefined, may have meant
            // something else.
            let at = self.ast.get(use_.dt).span;
            for error in self.errors.get_mut(errors..).unwrap_or_default() {
                if error.message.starts_with("undefined variable")
                    && error.span.start >= at.start
                    && error.span.end <= at.end
                {
                    error.notes.push(JUXTAPOSED_NOTE.to_owned());
                }
            }
            return TyId::ERROR;
        }
        let (Some(key), Some(read)) = (use_.key, use_.read) else {
            self.not_a_def_type(def);
            return TyId::ERROR;
        };
        if self.shadows_def_type(use_.dt) {
            self.not_a_def_type(def);
            return TyId::ERROR;
        }
        // What is wrong with a def-type's own parameters is reported where
        // it is declared.
        let Some(UseSignature {
            definition,
            mut sig,
            shape,
            tag,
        }) = self.use_signature(def, dt_ty)
        else {
            return TyId::ERROR;
        };
        let generic = sig.generic.is_some();
        // The variant the def-type is called with, a constant.
        if let Some(tag) = tag {
            self.lowering.variants.insert(id, tag);
        }
        // The meta the def-type declares for its definitions, which a
        // reference carries, and which a path past its definition reads.
        let meta = self.types.fn_sig(definition).and_then(|f| f.meta);
        let (shown, path) = self.ast.show_def_type(def);
        let at = self.ast.get(read).span;
        let mut defined = read;
        match self.definition_at(key, read, meta.is_some()) {
            DefinitionAt::Leaf => {}
            DefinitionAt::Past(prefix) => defined = prefix,
            DefinitionAt::Interior => {
                self.errors.push(Diagnostic::error(
                    format!("`{path}` is not a `{shown}` definition"),
                    at,
                ));
                return TyId::ERROR;
            }
            DefinitionAt::Missing(seen) => {
                let note = seen_note(&shown, &seen);
                self.errors.push(
                    Diagnostic::error(format!("`{shown}` has no definition `{path}`"), at)
                        .with_note(note),
                );
                return TyId::ERROR;
            }
        }
        // The definition's cell, read as any dotted field's is.
        let read_ty = match self.field_reference(defined) {
            Some(ty) => ty,
            None => self.check_expr(defined),
        };
        self.expr_types[defined.index()] = read_ty;
        if read_ty == TyId::ERROR {
            self.unread_past(read, defined);
            return TyId::ERROR;
        }
        // A generic def-type's type parameters are bound by the definition's
        // own type (see [Generic results](#generic-results)).
        if generic {
            let Some(bound) = self.bound_by_definition(&sig, shape, read_ty) else {
                return TyId::ERROR;
            };
            sig = bound;
        }
        let ty = self.types.fn_type_with_meta(
            sig.params
                .get(shape.invocation()..)
                .unwrap_or_default()
                .iter()
                .copied(),
            sig.ret,
            sig.effect,
            meta,
        );
        // A reference calls the def-type, so it needs what the def-type does.
        let ty = self.types.with_need_set(ty, sig.needs);
        if defined == read {
            self.expr_types[id.index()] = ty;
            return ty;
        }
        self.references_past.push((id, defined, ty));
        let found = self.read_past(def, read, (defined, read_ty));
        self.expr_types[id.index()] = found;
        found
    }

    /// Types the field accesses of the read `read` of the def-type reference
    /// `def` past `defined`, its prefix that reads the definition, of type
    /// `defined_ty`, as the field accesses they are: the first reads the
    /// definition's meta, `package dev.peios.testpkg.versions` the meta's
    /// field `versions`, each further one a field of the one before (see
    /// [Metas](crate::check::attributes#metas)). The meta's errors name the reference, `package
    /// dev.peios.testpkg`, as reading its meta would (`Checker::meta_read`).
    /// Returns the type of the last.
    pub(super) fn read_past(
        &mut self,
        def: DefTypeId,
        read: ExprId,
        defined: (ExprId, TyId),
    ) -> TyId {
        // The accesses past the definition, innermost first.
        let mut accesses = Vec::new();
        let mut at = read;
        while at != defined.0 {
            let ExprKind::Field { expr, name } = self.ast.get(at).kind else {
                break;
            };
            accesses.push((at, name));
            at = expr;
        }
        accesses.reverse();
        let outer = self.reading_past.replace((defined.0, def));
        let (mut record, mut ty) = defined;
        for &(access, name) in &accesses {
            ty = if ty == TyId::ERROR {
                TyId::ERROR
            } else {
                self.field_access(access, (record, ty), name)
            };
            self.expr_types[access.index()] = ty;
            record = access;
        }
        self.reading_past = outer;
        ty
    }

    /// Types the field accesses of the read `read` of a def-type reference
    /// past `defined`, its prefix that reads the definition, as erroneous,
    /// when the definition cannot be read, so that none is left untyped.
    pub(super) fn unread_past(&mut self, read: ExprId, defined: ExprId) {
        let mut at = read;
        while at != defined {
            self.expr_types[at.index()] = TyId::ERROR;
            let ExprKind::Field { expr, .. } = self.ast.get(at).kind else {
                break;
            };
            at = expr;
        }
    }

    /// Whether the def-type `dt` of a reference names a binding nearer than
    /// the input's top-level declarations, a parameter or a local, which is
    /// then what it names, whatever the workspace found its name names at
    /// the top level (see [Def-types](#def-types)).
    pub(super) fn shadows_def_type(&self, dt: ExprId) -> bool {
        let mut at = dt;
        while let ExprKind::Field { expr, .. } = self.ast.get(at).kind {
            at = expr;
        }
        let ExprKind::Name(name) = self.ast.get(at).kind else {
            return false;
        };
        self.lookup(name)
            .is_some_and(|(index, _)| index >= self.top_end)
    }

    /// What the path `read` of a def-type reference, a namespace's key,
    /// `key`, and the reference's path, names in the file's view: a
    /// definition, a node above some, or nothing, with the paths of the
    /// definitions under the key it sees (see [Def-types](#def-types)). An
    /// earlier session input's namespace is a binding of the key's, whose
    /// record type says. With `past`, for a def-type that declares a meta,
    /// a definition at a proper prefix of the path is what it names, the
    /// rest of the path read from its meta, since no definition lies under
    /// another (see [Metas](crate::check::attributes#metas)).
    pub(super) fn definition_at(&mut self, key: Name, read: ExprId, past: bool) -> DefinitionAt {
        // The path's names, each with the prefix of the read through it.
        let mut path = Vec::new();
        let mut at = read;
        while let ExprKind::Field { expr, name } = self.ast.get(at).kind {
            path.push((name, at));
            at = expr;
        }
        path.reverse();
        let last = path.len().saturating_sub(1);
        if let Some(workspace) = self.workspace
            && let Some(root) = workspace.scope.root(key)
        {
            let scope = &workspace.scope;
            let mut node = root;
            for (index, &(name, prefix)) in path.iter().enumerate() {
                match scope.child(node, name) {
                    Some(child) => node = child,
                    None => return DefinitionAt::Missing(self.definitions_under(root)),
                }
                if past && index < last && scope.is_leaf(node) {
                    return DefinitionAt::Past(prefix);
                }
            }
            return if scope.is_leaf(node) {
                DefinitionAt::Leaf
            } else {
                DefinitionAt::Interior
            };
        }
        let Some((_, binding)) = self.lookup(key) else {
            return DefinitionAt::Missing(Vec::new());
        };
        let mut ty = binding.ty;
        for (index, &(name, prefix)) in path.iter().enumerate() {
            match self.types.field(self.types.strip(ty), name) {
                Some(field) => ty = field,
                None => return DefinitionAt::Missing(self.definitions_of(binding.ty)),
            }
            if past && index < last && self.types.fn_sig(self.types.strip(ty)).is_some() {
                return DefinitionAt::Past(prefix);
            }
        }
        if self.types.fn_sig(self.types.strip(ty)).is_some() {
            DefinitionAt::Leaf
        } else {
            DefinitionAt::Interior
        }
    }

    /// The paths of the definitions at or under the scope's number `root`
    /// of the file's view, a namespace's root, in the view's order, as
    /// written after the def-type.
    pub(super) fn definitions_under(&self, root: u32) -> Vec<String> {
        let Some(workspace) = self.workspace else {
            return Vec::new();
        };
        let names = self.ast.names();
        let mut found = Vec::new();
        let mut stack: Vec<(u32, String)> = vec![(root, String::new())];
        while let Some((node, path)) = stack.pop() {
            if workspace.scope.is_leaf(node) {
                found.push(path);
                continue;
            }
            for (name, child) in workspace.scope.children(node).into_iter().rev() {
                let text = names.get(name);
                let child_path = if path.is_empty() {
                    text.to_owned()
                } else {
                    format!("{path}.{text}")
                };
                stack.push((child, child_path));
            }
        }
        found
    }

    /// The paths of the definitions of a namespace of type `ty`, an earlier
    /// session input's: the paths of its function-typed leaves, in the
    /// order of its fields' names.
    pub(super) fn definitions_of(&self, ty: TyId) -> Vec<String> {
        let names = self.ast.names();
        let mut found = Vec::new();
        let mut stack: Vec<(TyId, String)> = vec![(ty, String::new())];
        while let Some((ty, path)) = stack.pop() {
            let ty = self.types.strip(ty);
            let Some(fields) = self.types.record_fields(ty) else {
                found.push(path);
                continue;
            };
            let mut fields: Vec<(&str, TyId)> = fields
                .iter()
                .map(|&(name, field)| (names.get(name), field))
                .collect();
            fields.sort_unstable_by(|a, b| b.0.cmp(a.0));
            for (text, field) in fields {
                let child = if path.is_empty() {
                    text.to_owned()
                } else {
                    format!("{path}.{text}")
                };
                stack.push((field, child));
            }
        }
        found
    }
}

/// The note of a def-type reference whose path names nothing, the def-type
/// shown `shown`, of the definitions the file sees, `seen`: "`package` has
/// no definitions here", "the only definition of `package` here is `x`",
/// "the definitions of `package` here are `a` and `b.c`".
fn seen_note(shown: &str, seen: &[String]) -> String {
    match seen {
        [] => format!("`{shown}` has no definitions here"),
        [only] => format!("the only definition of `{shown}` here is `{only}`"),
        [init @ .., last] => {
            let init: Vec<String> = init.iter().map(|d| format!("`{d}`")).collect();
            format!(
                "the definitions of `{shown}` here are {} and `{last}`",
                init.join(", ")
            )
        }
    }
}
