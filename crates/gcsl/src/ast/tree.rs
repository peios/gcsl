//! The tree, `Ast`: its nodes, items and type expressions, and the queries of
//! them the later passes make.

use std::fmt;
use std::collections::HashMap;

use crate::MAX_DEPTH;
use crate::check::Constant;
use crate::diagnostic::Diagnostic;
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::Strings;

use super::{
    Annotate, Arg, ArgRange, AttributeUse, ContextDecl, DefTypeId, DefTypeRole, DefTypeUse,
    Element, ElementRange, Expr, ExprId, ExprKind, FieldDefault, FieldTy, FieldTyKind,
    FieldTyRange, FnDecl, FnId, ForDecl, ForHead, ForId, GenericOwner, Generics, ImportDecl,
    ImportForm, ImportGlob, ImportId, ImportMember, ImportMemberRange, Item, ItemId, ItemKind,
    ItemRange, Label, LabelId, MethodDecl, NeedTy, NeedTyRange, NoAnnotation, Param, ParamRange,
    ParamTy, ParamTyRange, PartRange, PathDisplay, PathRange, PathSegment, Pretty, RecoverDecl,
    RecoverId, SegmentKind, StrPart, Syntax, TypeArena, TypeArgRange, TypeExpr, TypeExprId,
    TypeExprKind, TypeLink, TypeProblem, TypeStore, WithBinding, WithRange, WrittenType, next_id,
    symbol_span,
};

/// A complete syntax tree: the arenas, the file's body, the names and the
/// string literals' text.
#[derive(Clone, Debug)]
pub struct Ast {
    pub(super) exprs: Vec<Expr>,
    pub(super) items: Vec<Item>,
    pub(super) item_lists: Vec<ItemId>,
    pub(super) parts: Vec<StrPart>,
    pub(super) types: TypeArena,
    pub(super) paths: Vec<PathSegment>,
    pub(super) fns: Vec<FnDecl>,
    pub(super) params: Vec<Param>,
    pub(super) args: Vec<Arg>,
    pub(super) with_bindings: Vec<WithBinding>,
    pub(super) elements: Vec<Element>,
    pub(super) fors: Vec<ForDecl>,
    pub(super) recovers: Vec<RecoverDecl>,
    pub(super) labels: Vec<Label>,
    pub(super) imports: Vec<ImportDecl>,
    pub(super) import_members: Vec<ImportMember>,
    /// The items marked `#[shadowable]`, in id order.
    pub(super) shadowable: Vec<ItemId>,
    /// The items marked `#[def_type]`, in id order.
    pub(super) def_type_marks: Vec<ItemId>,
    /// The `type` items marked `#[attribute]`, in id order.
    pub(super) attribute_marks: Vec<ItemId>,
    /// The `type` items marked `#[defers]`, sorted.
    pub(super) defers_marks: Vec<ItemId>,
    /// The uses of attributes that are none of the parser's, by the items
    /// they apply to, in id order, and on each item in source order.
    pub(super) attribute_uses: Vec<AttributeUse>,
    /// The type parameters of the generic function items, in the order
    /// their functions were made.
    pub(super) generics: Vec<Generics>,
    /// The methods of the outermost block, in source order.
    pub(super) methods: Vec<MethodDecl>,
    /// The contexts the outermost block declares, in source order.
    pub(super) contexts: Vec<ContextDecl>,
    /// The def-type definitions and references, in the order they were
    /// parsed (see [Def-types](crate::ast#def-types)).
    pub(super) def_types: Vec<DefTypeUse>,
    /// Where the file's `#![no_prelude]` is written, if it has one.
    pub(super) no_prelude: Option<Span>,
    /// Whether it was parsed from source with syntax errors.
    pub(super) syntax: Syntax,
    /// Where each field name written quoted, as a string literal, is, by
    /// where it ends (see [`Ast::written_name`]).
    pub(super) quoted: Vec<Span>,
    /// The implicit `#[shadowable] import * from prelude`, if a workspace
    /// gave the file one.
    pub(super) implicit_import: Option<ItemId>,
    /// What the expansion of its types did to the tree.
    pub(super) expansion: Expansion,
    pub(super) root: ItemRange,
    pub(super) names: Names,
    pub(super) strings: Strings,
}

/// What the expansion of a tree's types did to it ([`Ast::expand_types`]).
#[derive(Clone, Debug, Default)]
pub(super) struct Expansion {
    /// The number of type expressions the parser made, once the tree is
    /// expanded: every one after them is a copy of a declared type's
    /// definition. `None` until then.
    pub(super) parsed: Option<u32>,
    /// Each type expression the expansion made into another, with its kind
    /// as written, by id.
    pub(super) written: Vec<(TypeExprId, TypeExprKind)>,
    /// The problems it found, by id.
    pub(super) problems: Vec<(TypeExprId, TypeProblem)>,
}

impl Ast {
    /// Returns the node `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn get(&self, id: ExprId) -> &Expr {
        &self.exprs[id.index()]
    }

    /// Returns the item `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id.index()]
    }

    /// Returns the items of the list `range`, in source order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn items(&self, range: ItemRange) -> &[ItemId] {
        let start = range.start as usize;
        &self.item_lists[start..start + range.len as usize]
    }

    /// Returns the parts of the interpolated string `range`, in source
    /// order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn parts(&self, range: PartRange) -> &[StrPart] {
        let start = range.start as usize;
        &self.parts[start..start + range.len as usize]
    }

    /// Returns the type expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn type_expr(&self, id: TypeExprId) -> &TypeExpr {
        self.types.get(id)
    }

    /// The kind the type expression `id` is written as: its kind, unless
    /// the expansion of the tree's types made it into another ([`Ast::
    /// expand_types`]), as an alias's name into its definition, which the
    /// printer does not show.
    #[must_use]
    pub fn written_type(&self, id: TypeExprId) -> TypeExprKind {
        self.rewritten(id).unwrap_or_else(|| self.types.kind(id))
    }

    /// The kind the type expression `id` is written as, if the expansion of
    /// the tree's types made it into another.
    pub(super) fn rewritten(&self, id: TypeExprId) -> Option<TypeExprKind> {
        let written = &self.expansion.written;
        let at = written.binary_search_by_key(&id.0, |&(at, _)| at.0).ok()?;
        written.get(at).map(|&(_, kind)| kind)
    }

    /// Whether the type expression `id` is a node of a copy of a declared
    /// type's definition that the expansion of the tree's types added
    /// ([`Ast::expand_types`]), one that no parse made: never changed once
    /// made, and shared by every use of the definition in the tree.
    #[must_use]
    pub fn is_copied(&self, id: TypeExprId) -> bool {
        self.expansion.parsed.is_some_and(|parsed| id.0 >= parsed)
    }

    /// Whether the type expression `id` stands for a declared type's
    /// definition, which the type's own file checks: a copy of one that the
    /// expansion of the tree's types added ([`Ast::is_copied`]), or a name
    /// of a declared type, written bare or qualified, that it made into one
    /// ([`Ast::expand_types`]).
    #[must_use]
    pub fn is_definition_copy(&self, id: TypeExprId) -> bool {
        let Some(parsed) = self.expansion.parsed else {
            return false;
        };
        id.0 >= parsed
            || matches!(
                self.rewritten(id),
                Some(
                    TypeExprKind::Named(_)
                        | TypeExprKind::Path(_)
                        | TypeExprKind::Applied { .. }
                        | TypeExprKind::PathApplied { .. }
                )
            )
    }

    /// The problems the expansion of the tree's types found with the type
    /// expression `id`, in source order, for the checker to report where it
    /// resolves it.
    pub fn type_problems(&self, id: TypeExprId) -> impl Iterator<Item = &TypeProblem> {
        let problems = &self.expansion.problems;
        let start = problems.partition_point(|&(at, _)| at.0 < id.0);
        problems
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(move |&&(at, _)| at == id)
            .map(|(_, problem)| problem)
    }

    /// The ids of the type expressions the parser made, in order, children
    /// first: not the copies the expansion of the tree's types adds.
    pub fn type_ids(&self) -> impl Iterator<Item = TypeExprId> + use<> {
        let parsed = self
            .expansion
            .parsed
            .unwrap_or_else(|| next_id(self.types.len()));
        (0..parsed).map(TypeExprId)
    }

    /// Every expression node of the tree, in the order allocated, for a walk
    /// that needs each node once and not the tree's shape: the formatter's
    /// ([`crate::fmt`]).
    pub(crate) fn expr_arena(&self) -> &[Expr] {
        &self.exprs
    }

    /// Every item of the tree, in the order allocated, as
    /// [`Ast::expr_arena`] gives the expressions.
    pub(crate) fn item_arena(&self) -> &[Item] {
        &self.items
    }

    /// The type expression `id` as it is written in source, as the printer
    /// shows it ([`Ast::pretty`]), its names shown with `names`: the tree's
    /// own table, or, when the tree's is lent out, the table it is lent to.
    #[must_use]
    pub fn show_type_with(&self, id: TypeExprId, names: &Names) -> String {
        /// The type, displayed.
        struct Shown<'a>(WrittenType<'a>, TypeExprId);
        impl fmt::Display for Shown<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.type_expr(f, self.1)
            }
        }
        Shown(WrittenType { ast: self, names }, id).to_string()
    }

    /// The item numbered `number` of the tree, if it has one.
    #[must_use]
    pub fn item_at(&self, number: u32) -> Option<ItemId> {
        (number < next_id(self.item_count())).then_some(ItemId(number))
    }

    /// Returns the fields of the record type expression `range`, in source
    /// order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn field_tys(&self, range: FieldTyRange) -> &[FieldTy] {
        self.types.fields(range)
    }

    /// The constant that the default [`FieldDefault::Copied`] `at` of a
    /// copy of a declaration's definition is (see [Types](crate::ast::typeexprs#types)): `None`
    /// if it is written as something that is not a constant, which its own
    /// file's check reports.
    #[must_use]
    pub fn copied_default(&self, at: u32) -> Option<&Constant> {
        self.types.constants.get(at as usize)?.as_ref()
    }

    /// The first of the defaults written in the type expression `id` that
    /// the parser made, at any depth, in the order it parsed them: the one
    /// of the least id, since each is a subtree of its own, allocated where
    /// it is written. `None` if it has none. Recurses once per level of
    /// nesting in the type.
    pub(super) fn first_default(&self, id: TypeExprId) -> Option<ExprId> {
        let mut first: Option<ExprId> = None;
        let mut take = |found: Option<ExprId>| {
            if let Some(found) = found
                && first.is_none_or(|first| found.0 < first.0)
            {
                first = Some(found);
            }
        };
        match self.types.kind(id) {
            TypeExprKind::Record(range) => {
                for field in self.types.fields(range) {
                    take(self.first_default(field.ty));
                    if let Some(FieldDefault::Expr(default)) = field.default {
                        take(Some(default));
                    }
                }
            }
            TypeExprKind::Fn {
                params, ret, meta, ..
            } => {
                for param in self.types.params(params) {
                    take(self.first_default(param.ty));
                }
                take(self.first_default(ret));
                if let Some(meta) = meta {
                    take(self.first_default(meta.ty));
                }
            }
            TypeExprKind::Applied { args, .. }
            | TypeExprKind::PathApplied { args, .. }
            | TypeExprKind::Instance { args, .. } => {
                for &arg in self.types.args(args) {
                    take(self.first_default(arg));
                }
            }
            TypeExprKind::List(element) => {
                take(self.first_default(element));
            }
            TypeExprKind::Union { lhs, rhs, .. } => {
                take(self.first_default(lhs));
                take(self.first_default(rhs));
            }
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::Ref(_)
            | TypeExprKind::Param(_)
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => {}
        }
        first
    }

    /// Returns the segments of the path `range`, in source order. A path
    /// always has at least one.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn path(&self, range: PathRange) -> &[PathSegment] {
        let start = range.start as usize;
        &self.paths[start..start + range.len as usize]
    }

    /// Returns the function expression or `lazy` `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn fn_decl(&self, id: FnId) -> &FnDecl {
        &self.fns[id.index()]
    }

    /// The number of function expressions and `lazy`s in the tree, which
    /// bounds every [`FnId`] in it: they are `0..fn_count()`.
    #[must_use]
    pub fn fn_count(&self) -> usize {
        self.fns.len()
    }

    /// Returns the `for` expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn for_decl(&self, id: ForId) -> &ForDecl {
        &self.fors[id.index()]
    }

    /// The number of `for` expressions in the tree, which bounds every
    /// [`ForId`] in it: they are `0..for_count()`.
    #[must_use]
    pub fn for_count(&self) -> usize {
        self.fors.len()
    }

    /// Returns the `recover` expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn recover_decl(&self, id: RecoverId) -> &RecoverDecl {
        &self.recovers[id.index()]
    }

    /// The number of `recover` expressions in the tree, which bounds every
    /// [`RecoverId`] in it.
    #[must_use]
    pub fn recover_count(&self) -> usize {
        self.recovers.len()
    }

    /// Returns the import item `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn import_decl(&self, id: ImportId) -> &ImportDecl {
        &self.imports[id.index()]
    }

    /// Returns the members of a `from` import.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn import_members(&self, range: ImportMemberRange) -> &[ImportMember] {
        let start = range.start as usize;
        &self.import_members[start..start + range.len as usize]
    }

    /// Returns the label of a labelled block, `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn label(&self, id: LabelId) -> Label {
        self.labels[id.index()]
    }

    /// The label of the expression `id`, if it is a labelled block, and
    /// `None` for every other expression.
    #[must_use]
    pub fn block_label(&self, id: ExprId) -> Option<Label> {
        match self.get(id).kind {
            ExprKind::Block {
                label: Some(label), ..
            } => Some(self.label(label)),
            _ => None,
        }
    }

    /// Returns the parameters of the function expression whose parameters
    /// are `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn params(&self, range: ParamRange) -> &[Param] {
        let start = range.start as usize;
        &self.params[start..start + range.len as usize]
    }

    /// Returns the arguments of the call whose arguments are `range`, in
    /// source order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn args(&self, range: ArgRange) -> &[Arg] {
        let start = range.start as usize;
        &self.args[start..start + range.len as usize]
    }

    /// The bindings of a `with` (see the parser's
    /// [Context](crate::parser::context#context)).
    #[must_use]
    pub fn with_bindings(&self, range: WithRange) -> &[WithBinding] {
        let start = range.start as usize;
        &self.with_bindings[start..start + range.len as usize]
    }

    /// The number of arguments of every call in the tree: the length of
    /// the argument pool, which side tables indexed like it share.
    pub(crate) fn arg_count(&self) -> usize {
        self.args.len()
    }

    /// Returns the parameters of the function type expression whose
    /// parameters are `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn param_tys(&self, range: ParamTyRange) -> &[ParamTy] {
        self.types.params(range)
    }

    /// Returns the needs of the function type expression whose needs are
    /// `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn need_tys(&self, range: NeedTyRange) -> &[NeedTy] {
        self.types.needs(range)
    }

    /// Returns the type arguments of the applied type whose arguments are
    /// `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn type_args(&self, range: TypeArgRange) -> &[TypeExprId] {
        self.types.args(range)
    }

    /// Returns the elements of the list literal whose elements are `range`,
    /// in source order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn elements(&self, range: ElementRange) -> &[Element] {
        let start = range.start as usize;
        &self.elements[start..start + range.len as usize]
    }

    /// Shows `segments`, a path of this tree or a prefix of one, as it is
    /// written (see [`PathDisplay`]).
    #[must_use]
    pub fn show_path<'a>(&'a self, segments: &'a [PathSegment]) -> PathDisplay<'a> {
        PathDisplay {
            names: &self.names,
            strings: &self.strings,
            segments,
            definition: false,
        }
    }

    /// Shows `segments`, a module path of an import, as it is written: its
    /// segments joined by `/`, each as its source spells it, dots and all,
    /// as in `pkgs/net.busybox.busybox` (see the parser's
    /// [Imports](crate::parser::imports#imports)).
    #[must_use]
    pub fn show_module_path(&self, segments: &[PathSegment]) -> String {
        segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| self.names.get(name))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Shows the path of the import `decl` as it is written, its glob
    /// included: `pkgs/net.busybox.busybox`, `pkgs/*`, or `*` alone (see
    /// the parser's [Imports](crate::parser::imports#imports)).
    #[must_use]
    pub fn show_import_path(&self, decl: &ImportDecl) -> String {
        let path = self.show_module_path(self.path(decl.path));
        let glob = match decl.glob {
            ImportGlob::Exact => return path,
            ImportGlob::Children => "*",
            ImportGlob::Descendants => "**",
        };
        if path.is_empty() {
            glob.to_owned()
        } else {
            format!("{path}/{glob}")
        }
    }

    /// Shows `segments`, the path of a definition or a reference made with a
    /// def-type, `PATH`, or a prefix of one, as it is written: its names
    /// joined by `.`, each as its source spells it, `+` and all, as in
    /// `org.gnu.gmp-c++`, and digits and all, as in
    /// `org.golang.go-stage0-1.24` (see [`PathDisplay`]).
    #[must_use]
    pub fn show_definition_path<'a>(&'a self, segments: &'a [PathSegment]) -> PathDisplay<'a> {
        PathDisplay {
            definition: true,
            ..self.show_path(segments)
        }
    }

    /// The file's body: the outermost block's items.
    #[must_use]
    pub fn root(&self) -> ItemRange {
        self.root
    }

    /// Whether the item `id` is marked `#[shadowable]`, the implicit import
    /// included (see [Attributes](crate::ast#attributes)).
    #[must_use]
    pub fn is_shadowable(&self, id: ItemId) -> bool {
        self.shadowable
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
    }

    /// Whether the `type` item `id` is marked `#[attribute]`: a declaration
    /// of an attribute (see the parser's
    /// [Attributes](crate::parser::attributes#attributes)).
    #[must_use]
    pub fn is_attribute(&self, id: ItemId) -> bool {
        self.attribute_marks
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
    }

    /// Whether the `type` item `id` is marked `#[defers]`: its attribute, if
    /// it declares one, defers the calls of the functions it is on (see the
    /// parser's [Attributes](crate::parser::attributes#attributes)).
    #[must_use]
    pub fn is_defers(&self, id: ItemId) -> bool {
        self.defers_marks
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
    }

    /// The uses of attributes that are none of the parser's on the item
    /// `id`, in source order (see the parser's
    /// [Attributes](crate::parser::attributes#attributes)).
    #[must_use]
    pub fn attribute_uses(&self, id: ItemId) -> &[AttributeUse] {
        let start = self
            .attribute_uses
            .partition_point(|used| used.item.0 < id.0);
        let end = self
            .attribute_uses
            .partition_point(|used| used.item.0 <= id.0);
        self.attribute_uses.get(start..end).unwrap_or_default()
    }

    /// The methods of the file's outermost block, in source order.
    #[must_use]
    pub fn methods(&self) -> &[MethodDecl] {
        &self.methods
    }

    /// Every call of a field, `x.NAME(...)`, of the tree, by the name and
    /// where the callee is written, in id order: the calls that may be of a
    /// method (see the checker's "Methods").
    pub fn field_calls(&self) -> impl Iterator<Item = (Name, Span)> + '_ {
        self.exprs.iter().filter_map(|expr| match expr.kind {
            ExprKind::Call { callee, .. } | ExprKind::Unclosed { callee, .. } => {
                match self.get(callee).kind {
                    ExprKind::Field { name, .. } => Some((name, self.get(callee).span)),
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// The method whose item is `item`, if it is one.
    #[must_use]
    pub fn method(&self, item: ItemId) -> Option<&MethodDecl> {
        self.methods.iter().find(|method| method.item == item)
    }

    /// The contexts the file declares, in source order.
    #[must_use]
    pub fn contexts(&self) -> &[ContextDecl] {
        &self.contexts
    }

    /// Every read of a context in the tree, `^NAME`, a dotted key's of a
    /// `with` and a `^NAME` parameter's default among them, by the name and
    /// the read's span, in the order the parser made them.
    #[must_use]
    pub fn context_reads(&self) -> Vec<(Name, Span)> {
        self.exprs
            .iter()
            .filter_map(|expr| match expr.kind {
                ExprKind::ContextRead(name) => Some((name, expr.span)),
                _ => None,
            })
            .collect()
    }

    /// The type parameters of every generic function and `type` item, in the
    /// order the parser finished them.
    #[must_use]
    pub fn all_generics(&self) -> &[Generics] {
        &self.generics
    }

    /// The type parameters of `owner`, if it is generic.
    #[must_use]
    pub fn generics_of(&self, owner: GenericOwner) -> Option<&Generics> {
        self.generics
            .iter()
            .find(|generics| generics.owner == owner)
    }

    /// The type parameters of `function`, if it is a generic function item.
    #[must_use]
    pub fn generics(&self, function: FnId) -> Option<&Generics> {
        self.generics_of(GenericOwner::Fn(function))
    }

    /// How many type parameters the `type` item `item` has: none unless it
    /// is generic.
    #[must_use]
    pub fn type_arity(&self, item: ItemId) -> u32 {
        self.generics_of(GenericOwner::Type(item))
            .map_or(0, |generics| next_id(generics.params.len()))
    }

    /// What has the type parameter that the name `name`, written as the type
    /// expression `id`, is, and its number, if one is in scope there: that
    /// of the innermost generic function whose signature or body `id` is
    /// part of, or the generic `type` item whose definition it is part of,
    /// that has a type parameter of the name.
    #[must_use]
    pub fn type_param(&self, id: TypeExprId, name: Name) -> Option<(GenericOwner, u8)> {
        self.generics
            .iter()
            .filter(|generics| generics.types.0 <= id.0 && id.0 < generics.types.1)
            .filter_map(|generics| {
                let index = generics
                    .params
                    .iter()
                    .position(|&(param, _)| param == name)?;
                Some((generics, u8::try_from(index).ok()?))
            })
            .min_by_key(|(generics, _)| generics.types.1 - generics.types.0)
            .map(|(generics, index)| (generics.owner, index))
    }

    /// Every use of an attribute that is none of the parser's, by the items
    /// they apply to, in id order.
    #[must_use]
    pub fn all_attribute_uses(&self) -> &[AttributeUse] {
        &self.attribute_uses
    }

    /// Whether the item `id` is marked `#[def_type]` (see
    /// [Attributes](crate::ast#attributes)).
    #[must_use]
    pub fn is_def_type(&self, id: ItemId) -> bool {
        self.def_type_marks
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
    }

    /// The details of the def-type definition or reference `id` (see
    /// [Def-types](crate::ast#def-types)).
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn def_type(&self, id: DefTypeId) -> &DefTypeUse {
        &self.def_types[id.index()]
    }

    /// Every def-type definition and reference of the tree, with its id, in
    /// the order they were parsed.
    pub fn def_types(&self) -> impl Iterator<Item = (DefTypeId, &DefTypeUse)> + '_ {
        (0..next_id(self.def_types.len()))
            .map(DefTypeId)
            .zip(&self.def_types)
    }

    /// The names of the def-type, `DT`, of the definition or reference
    /// `id`, as it is written: a name, then the name of each field access
    /// on it.
    #[must_use]
    pub fn def_type_names(&self, id: DefTypeId) -> Vec<Name> {
        let mut names = Vec::new();
        let mut at = self.def_type(id).dt;
        loop {
            match self.get(at).kind {
                ExprKind::Field { expr, name } => {
                    names.push(name);
                    at = expr;
                }
                ExprKind::Name(name) => {
                    names.push(name);
                    break;
                }
                _ => break,
            }
        }
        names.reverse();
        names
    }

    /// The def-type, `DT`, of the definition or reference `id`, with its
    /// variant if one is written, and its path, `PATH`, each as it is
    /// written, as in `t.package`, `Toolchain::C` and `dev.peios.testpkg`
    /// ([`Ast::show_definition_path`]).
    #[must_use]
    pub fn show_def_type(&self, id: DefTypeId) -> (String, String) {
        let names: Vec<&str> = self
            .def_type_names(id)
            .into_iter()
            .map(|name| self.names.get(name))
            .collect();
        let mut shown = names.join(".");
        if let Some((variant, _)) = self.def_type(id).variant {
            shown.push_str("::");
            shown.push_str(self.names.get(variant));
        }
        let path = self
            .show_definition_path(self.path(self.def_type(id).path))
            .to_string();
        (shown, path)
    }

    /// The def-type member `id` of an import as it is written, of every
    /// variant if `every` (see the parser's
    /// [Imports](crate::parser::imports#imports)): `DT *`, `DT::V *`,
    /// `DT::*`, or the one definition `DT NAME` or `DT::V NAME`.
    #[must_use]
    pub fn show_def_type_member(&self, id: DefTypeId, every: bool) -> String {
        let (dt, path) = self.show_def_type(id);
        if every {
            format!("{dt}::*")
        } else if path.is_empty() {
            format!("{dt} *")
        } else {
            format!("{dt} {path}")
        }
    }

    /// Gives the def-type definition, reference or meta `id` `key`, the key
    /// of its def-type's namespace, which the workspace found its `DT` names
    /// (see [Def-types](crate::ast#def-types)), and, for a definition or a meta,
    /// `meta`, the key of the namespace of the metas of the def-type's
    /// definitions in the directory of the tree's file and PATH as one name:
    /// a reference's read of the definition is then of the namespace, its
    /// name made the key, and a definition's read of its meta of the
    /// namespace of the metas, its name made that key and its field access
    /// PATH's one name.
    pub fn set_def_type_key(&mut self, id: DefTypeId, key: Name, meta: Option<(Name, Name)>) {
        let Some(entry) = self.def_types.get_mut(id.index()) else {
            return;
        };
        entry.key = Some(key);
        if entry.role != DefTypeRole::Reference {
            entry.meta = meta;
        }
        let (read, meta_read) = (entry.read, entry.meta_read);
        let mut at = read;
        while let Some(node) = at.and_then(|node| self.exprs.get_mut(node.index())) {
            match node.kind {
                ExprKind::Field { expr, .. } => at = Some(expr),
                ExprKind::Name(_) => {
                    node.kind = ExprKind::Name(key);
                    break;
                }
                _ => break,
            }
        }
        // A definition's read of its meta: the leaf's name, then the key's.
        if let (Some(read), Some((meta_key, leaf))) = (meta_read, meta)
            && let Some(node) = self.exprs.get_mut(read.index())
            && let ExprKind::Field { expr, .. } = node.kind
        {
            node.kind = ExprKind::Field { expr, name: leaf };
            if let Some(base) = self.exprs.get_mut(expr.index()) {
                base.kind = ExprKind::Name(meta_key);
            }
        }
    }

    /// Whether an item of the file's own is marked `#[shadowable]`, the
    /// implicit import aside: without one, nothing of the file is shadowed.
    #[must_use]
    pub fn marks_any(&self) -> bool {
        self.shadowable
            .iter()
            .any(|&marked| Some(marked) != self.implicit_import)
    }

    /// Where the file's `#![no_prelude]` is written, if it is marked so.
    #[must_use]
    pub fn no_prelude(&self) -> Option<Span> {
        self.no_prelude
    }

    /// Whether the tree was parsed from source with syntax errors, and so
    /// how far its shape can be trusted: [`Syntax::Clean`] for every tree
    /// that is run.
    #[must_use]
    pub fn syntax(&self) -> Syntax {
        self.syntax
    }

    /// Where the name whose text is `text`, written last in a span that
    /// ends at `end`, is written: the string literal of a quoted field name
    /// that ends there, quotes and all, if there is one, else the `text`'s
    /// length before `end`, an identifier written as its text. A field
    /// access spans its record through its name, so this is the name of
    /// `r.x` and of `r."x"`.
    #[must_use]
    pub fn written_name(&self, end: u32, text: &str) -> Span {
        let at = self.quoted.partition_point(|span| span.end < end);
        match self.quoted.get(at) {
            Some(&quoted) if quoted.end == end => quoted,
            _ => {
                let length = u32::try_from(text.len()).unwrap_or(u32::MAX);
                Span::new(end.saturating_sub(length), end)
            }
        }
    }

    /// The implicit `#[shadowable] import * from prelude` a workspace gave
    /// the file, if it has one (see [Attributes](crate::ast#attributes)).
    #[must_use]
    pub fn implicit_import(&self) -> Option<ItemId> {
        self.implicit_import
    }

    /// Gives the file its implicit `#[shadowable] import * from prelude`,
    /// `prelude` being the name of the prelude's stem: an import item of the
    /// tree written nowhere, kept apart from the file's body (see
    /// [Attributes](crate::ast#attributes)). Does nothing to a file marked
    /// `#![no_prelude]` or given one already.
    pub fn add_implicit_import(&mut self, prelude: Name) {
        if self.no_prelude.is_some() || self.implicit_import.is_some() {
            return;
        }
        let nowhere = Span::new(0, 0);
        let path_start = next_id(self.paths.len());
        self.paths.push(PathSegment {
            kind: SegmentKind::Name(prelude),
            span: nowhere,
        });
        let members_start = next_id(self.import_members.len());
        self.import_members.push(ImportMember::All(nowhere));
        let import = ImportId(next_id(self.imports.len()));
        self.imports.push(ImportDecl {
            path: PathRange {
                start: path_start,
                len: 1,
            },
            form: ImportForm::Members(ImportMemberRange {
                start: members_start,
                len: 1,
            }),
            glob: ImportGlob::Exact,
            path_at: nowhere,
        });
        let item = ItemId(next_id(self.items.len()));
        self.items.push(Item {
            kind: ItemKind::Import(import),
            span: nowhere,
        });
        self.implicit_import = Some(item);
        let at = self.shadowable.partition_point(|&marked| marked.0 < item.0);
        self.shadowable.insert(at, item);
    }

    /// Expands the tree's types, once (see [Types](crate::ast::typeexprs#types)). Every type
    /// expression the parser made is taken in order, children first, as it
    /// made them:
    ///
    /// - one that `links` (by id, in order) has a declaration for, a name
    ///   of a declared type, written bare or qualified, becomes what that
    ///   declaration's settled definition in `store` is, its nodes copied
    ///   into the tree's arena after the parsed ones, each once, shared by
    ///   every use; a generic one's, applied to its arguments, with them in
    ///   place of its parameters, the nodes above a parameter made for the
    ///   use (`TypeArena::substitute`); or [`TypeExprKind::Invalid`] if
    ///   the declaration is erroneous, which its own file reports;
    /// - one that `links` has an error for becomes `Invalid`, with the
    ///   problem [`TypeProblem::Unresolved`];
    /// - a record type with spreads becomes the record type of its merged
    ///   fields (see [`TypeArena::merge_record`]), with its problems;
    /// - one that would then be nested more than [`MAX_DEPTH`] levels deep
    ///   becomes `Invalid`, with the problem "expression nested too
    ///   deeply" at it.
    ///
    /// The kind each was written as is kept for the printer
    /// ([`Ast::written_type`]). A tree with no link and no spread is only
    /// noted as expanded.
    pub fn expand_types(&mut self, store: &TypeStore, links: &[(TypeExprId, TypeLink)]) {
        if self.expansion.parsed.is_some() {
            return;
        }
        let parsed = self.types.len();
        self.expansion.parsed = Some(next_id(parsed));
        if links.is_empty()
            && self
                .types
                .fields
                .iter()
                .all(|field| field.kind != FieldTyKind::Spread)
        {
            return;
        }
        // The height of each node, by id: the parsed ones' as they are
        // taken, the copies' as they are made.
        let mut heights: Vec<u32> = vec![1; parsed];
        let mut memo = HashMap::new();
        let mut written = Vec::new();
        let mut problems = Vec::new();
        for id in self.types.ids_from(0).take(parsed) {
            let kind = self.types.kind(id);
            let link = links
                .binary_search_by_key(&id.0, |&(at, _)| at.0)
                .ok()
                .and_then(|at| links.get(at))
                .map(|(_, link)| link);
            match link {
                Some(TypeLink::Decl(decl)) => {
                    let definition = store
                        .root(*decl)
                        .filter(|&root| store.arena.kind(root) != TypeExprKind::Invalid);
                    let expanded = match definition {
                        Some(root) => {
                            let start = self.types.len();
                            // A store holds no expression of a tree.
                            let mut copy = self.types.copy(
                                &store.arena,
                                root,
                                &mut memo,
                                (&|_| None, &|_| None),
                            );
                            // A generic type's use, its arity checked, has
                            // its arguments in place of its parameters.
                            if let TypeExprKind::Applied { args, .. }
                            | TypeExprKind::PathApplied { args, .. } = kind
                            {
                                let args = self.types.args(args).to_vec();
                                copy = self.types.substitute(copy, &args, &mut HashMap::new());
                            }
                            // The copies are children first, so each one's
                            // height follows from its children's.
                            for new in self.types.ids_from(start) {
                                let height = self.types.height(new, &heights);
                                heights.push(height);
                            }
                            self.types.kind(copy)
                        }
                        None => TypeExprKind::Invalid,
                    };
                    self.types.set_kind(id, expanded);
                    written.push((id, kind));
                }
                Some(TypeLink::Error(error)) => {
                    self.types.set_kind(id, TypeExprKind::Invalid);
                    written.push((id, kind));
                    problems.push((id, TypeProblem::Unresolved(error.clone())));
                }
                None => {
                    let found = self.types.merge_record(id);
                    if self.types.kind(id) != kind {
                        written.push((id, kind));
                    }
                    problems.extend(found.into_iter().map(|problem| (id, problem)));
                }
            }
            let mut height = self.types.height(id, &heights);
            if height as usize > MAX_DEPTH {
                let span = self.types.get(id).span;
                if written.last().map(|&(at, _)| at) != Some(id) {
                    written.push((id, kind));
                }
                self.types.set_kind(id, TypeExprKind::Invalid);
                problems.push((
                    id,
                    TypeProblem::Unresolved(Diagnostic::error(
                        "expression nested too deeply",
                        span,
                    )),
                ));
                height = 1;
            }
            if let Some(entry) = heights.get_mut(id.index()) {
                *entry = height;
            }
        }
        self.expansion.written = written;
        self.expansion.problems = problems;
    }

    /// The span of the file's body: from its first item to its last, or an
    /// empty span at offset 0 if it has none.
    #[must_use]
    pub fn root_span(&self) -> Span {
        let items = self.items(self.root);
        match (items.first(), items.last()) {
            (Some(&first), Some(&last)) => Span::join(self.item(first).span, self.item(last).span),
            _ => Span::new(0, 0),
        }
    }

    /// The names the tree's identifiers were interned into.
    #[must_use]
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// Consumes the tree, returning its names.
    #[must_use]
    pub fn into_names(self) -> Names {
        self.names
    }

    /// Lends the tree `names` in place of its own, returning those. A
    /// workspace (see [`crate::workspace`]) keeps one table for all its
    /// files and lends it to each file's tree while that file is checked.
    pub fn replace_names(&mut self, names: Names) -> Names {
        std::mem::replace(&mut self.names, names)
    }

    /// The text of the tree's string literals.
    #[must_use]
    pub fn strings(&self) -> &Strings {
        &self.strings
    }

    /// Number of expression nodes in the arena, which bounds every
    /// [`ExprId`] in it.
    pub(crate) fn len(&self) -> usize {
        self.exprs.len()
    }

    /// Number of items in the arena, which bounds every [`ItemId`] in it.
    pub(crate) fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Every expression's id, in allocation order: for tests that look for
    /// nodes of a kind, and for an index of the tree's names
    /// ([`crate::index`]).
    pub(crate) fn expr_ids(&self) -> impl Iterator<Item = ExprId> + use<> {
        (0..next_id(self.exprs.len())).map(ExprId)
    }

    /// Every item's id, in allocation order, for an index of the tree's
    /// names ([`crate::index`]).
    pub(crate) fn item_ids(&self) -> impl Iterator<Item = ItemId> + use<> {
        (0..next_id(self.items.len())).map(ItemId)
    }

    /// The ids of the nodes of expression `id`'s subtree, its descendants
    /// and itself, in allocation order. They are contiguous, ending at `id`:
    /// nodes are allocated children-first, and while the parser parses an
    /// expression it allocates only that expression's nodes (a path is read
    /// from its tokens, allocating only the positions of its index
    /// segments, which belong to its item).
    /// The first is found by following, from `id`, the child the parser
    /// parses first down to a leaf: a block's first item's value, or the
    /// position of the first index segment of its path if it has one, or
    /// the first field default in its annotation if it has one, an
    /// `if`'s condition, a call's callee, a binary operator's left operand,
    /// a function's first parameter default, or field default in a
    /// parameter's or its return type, if it has one (a default is
    /// parsed before the body) and its body otherwise, a `lazy`'s body, an
    /// interpolated string's first expression, a list literal's first
    /// element's, an index's list, an
    /// iteration's iterable, a condition loop's first header item's, its
    /// condition or its body, whichever comes first, a `do` loop's body,
    /// and a def-type reference's `DT`, its read of the definition coming
    /// between that and itself. (A block's first item may hold no
    /// expression, a `break` without a value: the first item that holds one
    /// is followed.) Debug builds check that every node of the range lies
    /// within `id`'s span and the node before it starts before `id`.
    pub(crate) fn subtree(&self, id: ExprId) -> impl Iterator<Item = ExprId> + use<> {
        let first = self.first_node(id);
        debug_assert!(
            (first.0..=id.0).all(|node| {
                let span = self.get(ExprId(node)).span;
                span.start >= self.get(id).span.start && span.end <= self.get(id).span.end
            }) && (first.0 == 0
                || self.get(ExprId(first.0 - 1)).span.start < self.get(id).span.start),
            "the subtree of an expression is not contiguous"
        );
        (first.0..=id.0).map(ExprId)
    }

    /// The first node the parser allocates for expression `id`: see
    /// [`Ast::subtree`].
    pub(super) fn first_node(&self, id: ExprId) -> ExprId {
        let mut node = id;
        loop {
            node = match self.get(node).kind {
                ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Duration(_)
                | ExprKind::Size(_)
                | ExprKind::Bool(_)
                | ExprKind::Str(_)
                | ExprKind::Name(_)
                | ExprKind::Variant { .. }
                | ExprKind::ContextRead(_)
                | ExprKind::Error => return node,
                // The body before the bindings after it, `f() with {...}`,
                // and the bindings before it, `with {...} {...}`: the
                // earliest of its parts.
                ExprKind::With { bindings, body } => self
                    .with_bindings(bindings)
                    .iter()
                    .flat_map(|binding| [binding.current, Some(binding.value)])
                    .flatten()
                    .chain([body])
                    .min_by_key(|part| part.0)
                    .unwrap_or(body),
                ExprKind::Interp(parts) => {
                    let first = self.parts(parts).iter().find_map(|&part| match part {
                        StrPart::Expr(expr) => Some(expr),
                        StrPart::Lit(_) => None,
                    });
                    match first {
                        Some(expr) => expr,
                        None => return node,
                    }
                }
                ExprKind::Block { items, .. } => match self.first_of_items(items) {
                    Some(first) => first,
                    None => return node,
                },
                ExprKind::If { cond, .. } | ExprKind::Assert { cond, .. } => cond,
                ExprKind::Unary { operand, .. } => operand,
                ExprKind::Binary { lhs, .. } => lhs,
                ExprKind::List(elements) => match self.elements(elements).first() {
                    Some(element) => element.value,
                    None => return node,
                },
                ExprKind::Paren(inner)
                | ExprKind::Field { expr: inner, .. }
                | ExprKind::Index { expr: inner, .. }
                | ExprKind::Ascribe { expr: inner, .. }
                | ExprKind::Is { expr: inner, .. }
                | ExprKind::As { expr: inner, .. }
                | ExprKind::NoField(inner) => inner,
                // A parameter's default, and a field's default in a
                // parameter's or the return type, is parsed before the body.
                ExprKind::Fn(function) => {
                    let decl = self.fn_decl(function);
                    self.params(decl.params)
                        .iter()
                        .flat_map(|param| [self.first_default(param.ty), param.default])
                        .chain([decl.ret.and_then(|ret| self.first_default(ret))])
                        .flatten()
                        .min_by_key(|default| default.0)
                        .unwrap_or(decl.body)
                }
                ExprKind::Lazy(function) => self.fn_decl(function).body,
                ExprKind::DefTypeRef(def) => self.def_type(def).dt,
                ExprKind::Recover(id) => self.recover_decl(id).body,
                ExprKind::Call { callee, .. } | ExprKind::Unclosed { callee, .. } => callee,
                ExprKind::For(id) => {
                    let decl = self.for_decl(id);
                    match decl.head {
                        ForHead::Each { iterable, .. } => iterable,
                        ForHead::While {
                            before,
                            cond,
                            after,
                        } => self
                            .first_of_items(before)
                            .or(cond)
                            .or_else(|| self.first_of_items(after))
                            .unwrap_or(decl.body),
                        ForHead::Do { .. } => decl.body,
                    }
                }
            };
        }
    }

    /// The expression whose subtree the parser allocates first among the
    /// items `items`, in order: the first item's that has one, the
    /// position of its path's first index segment if it has one, since a
    /// path is parsed before its value, and its value otherwise; `None` if
    /// no item holds an expression, as a `break` without a value holds
    /// none.
    pub(super) fn first_of_items(&self, items: ItemRange) -> Option<ExprId> {
        self.items(items)
            .iter()
            .find_map(|&item| match self.item(item).kind {
                ItemKind::Rebind { path, value } => Some(
                    self.path(path)
                        .iter()
                        .find_map(PathSegment::position)
                        .unwrap_or(value),
                ),
                // A field's default in the annotation is parsed before the
                // value.
                ItemKind::Let {
                    ty: Some(ty),
                    value,
                    ..
                }
                | ItemKind::Field {
                    ty: Some(ty),
                    value,
                    ..
                } => Some(self.first_default(ty).unwrap_or(value)),
                kind => kind.value(),
            })
    }

    /// The span of the keyword that starts the loop `id`, an
    /// [`ExprKind::For`]: its `do` for a `do` loop, and its `for` otherwise.
    /// The span of the whole expression for any other node.
    #[must_use]
    pub fn loop_keyword(&self, id: ExprId) -> Span {
        let expr = self.get(id);
        match expr.kind {
            ExprKind::For(for_id) => match self.for_decl(for_id).head {
                ForHead::Do { .. } => symbol_span("do", expr.span.start),
                ForHead::Each { .. } | ForHead::While { .. } => symbol_span("for", expr.span.start),
            },
            _ => expr.span,
        }
    }

    /// Returns a `Display` rendering of the tree as indented S-expressions,
    /// for `--dump-ast`.
    #[must_use]
    pub fn pretty(&self) -> Pretty<'_, NoAnnotation> {
        self.pretty_with(&NoAnnotation)
    }

    /// Like [`Ast::pretty`], with `annotation` written after each expression
    /// node's head, e.g. its type for `--dump-types`.
    #[must_use]
    pub fn pretty_with<'a, A: Annotate>(&'a self, annotation: &'a A) -> Pretty<'a, A> {
        Pretty {
            ast: self,
            annotation,
        }
    }
}
