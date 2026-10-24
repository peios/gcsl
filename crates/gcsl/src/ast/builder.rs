//! Building a tree: the builder the parser fills, its marks for backtracking,
//! constant trees, and the syntax a node was written in.

use std::num::NonZeroU32;

use crate::names::Names;
use crate::source::Span;
use crate::strings::Strings;

use super::{
    Arg, ArgRange, Ast, AttributeUse, ContextDecl, DefTypeId, DefTypeUse, Element, ElementRange,
    Expansion, Expr, ExprId, ExprKind, FieldTy, FieldTyRange, FnDecl, FnId, ForDecl, ForId,
    GenericOwner, Generics, ImportDecl, ImportId, ImportMember, ImportMemberRange, Item, ItemId,
    ItemKind, ItemRange, Label, LabelId, MethodDecl, Param, ParamRange, ParamTy, ParamTyRange,
    PartRange, PathRange, PathSegment, RecoverDecl, RecoverId, StrPart, TypeArena, TypeArgRange,
    TypeExpr, TypeExprId, UnaryOp, WithBinding, WithRange, next_id,
};

/// Arenas under construction. Allocate children before their parents, then
/// call [`AstBuilder::finish`] with the root.
#[derive(Clone, Debug, Default)]
pub struct AstBuilder {
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
    pub(super) shadowable: Vec<ItemId>,
    pub(super) def_type_marks: Vec<ItemId>,
    pub(super) attribute_marks: Vec<ItemId>,
    /// The `type` items marked `#[defers]`, sorted.
    pub(super) defers_marks: Vec<ItemId>,
    pub(super) attribute_uses: Vec<AttributeUse>,
    pub(super) generics: Vec<Generics>,
    pub(super) methods: Vec<MethodDecl>,
    pub(super) contexts: Vec<ContextDecl>,
    pub(super) def_types: Vec<DefTypeUse>,
    pub(super) no_prelude: Option<Span>,
    pub(super) syntax: Syntax,
    pub(super) quoted: Vec<Span>,
}

/// What [`is_constant`] reads of a tree: a finished [`Ast`], or the parser's
/// [`AstBuilder`] while it builds one, so that the parser and the checker
/// apply one rule.
pub trait ConstantTree {
    /// The expression `id`.
    fn constant_expr(&self, id: ExprId) -> &Expr;
    /// The elements of the list `range`.
    fn constant_elements(&self, range: ElementRange) -> &[Element];
    /// The items of the list `range`.
    fn constant_items(&self, range: ItemRange) -> &[ItemId];
    /// The item `id`.
    fn constant_item(&self, id: ItemId) -> &Item;
    /// The segments of the path `range`.
    fn constant_path(&self, range: PathRange) -> &[PathSegment];
}

impl ConstantTree for Ast {
    fn constant_expr(&self, id: ExprId) -> &Expr {
        self.get(id)
    }
    fn constant_elements(&self, range: ElementRange) -> &[Element] {
        self.elements(range)
    }
    fn constant_items(&self, range: ItemRange) -> &[ItemId] {
        self.items(range)
    }
    fn constant_item(&self, id: ItemId) -> &Item {
        self.item(id)
    }
    fn constant_path(&self, range: PathRange) -> &[PathSegment] {
        self.path(range)
    }
}

impl ConstantTree for AstBuilder {
    fn constant_expr(&self, id: ExprId) -> &Expr {
        self.get(id)
    }
    fn constant_elements(&self, range: ElementRange) -> &[Element] {
        let start = range.start as usize;
        &self.elements[start..start + range.len as usize]
    }
    fn constant_items(&self, range: ItemRange) -> &[ItemId] {
        self.items(range)
    }
    fn constant_item(&self, id: ItemId) -> &Item {
        self.item(id)
    }
    fn constant_path(&self, range: PathRange) -> &[PathSegment] {
        let start = range.start as usize;
        &self.paths[start..start + range.len as usize]
    }
}

/// Whether the expression `id` of `tree`, a parameter's or a field's
/// default, is a constant (see the checker's
/// [Calls](crate::check::functions#functions)): a literal, a string without
/// interpolation (raw or not), an integer, a float, `true` or `false`; `-`
/// applied directly to an integer, float, duration or size literal; a list literal without
/// a spread whose elements are constants; or a record block whose items are
/// all fields `NAME = CONSTANT`, of one name each and not annotated, `{}`
/// among them. Nothing else, not even in parentheses. A parameter's default
/// that is none is evaluated at each call that leaves it out, through its
/// default function ([`Param::default_fn`]); a field's must be one.
///
/// Recurses once per level of list and record nesting.
#[must_use]
pub fn is_constant(tree: &impl ConstantTree, id: ExprId) -> bool {
    match tree.constant_expr(id).kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Duration(_)
        | ExprKind::Size(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_) => true,
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => matches!(
            tree.constant_expr(operand).kind,
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Duration(_) | ExprKind::Size(_)
        ),
        ExprKind::List(elements) => tree
            .constant_elements(elements)
            .iter()
            .all(|element| element.spread.is_none() && is_constant(tree, element.value)),
        ExprKind::Block { items, label: None } => {
            tree.constant_items(items)
                .iter()
                .all(|&item| match tree.constant_item(item).kind {
                    ItemKind::Field {
                        path,
                        ty: None,
                        value,
                        function: false,
                    } => tree.constant_path(path).len() == 1 && is_constant(tree, value),
                    _ => false,
                })
        }
        _ => false,
    }
}

/// Where every arena and pool of an [`AstBuilder`] was, to put it back
/// there ([`AstBuilder::mark`], [`AstBuilder::truncate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuilderMark {
    pub(super) exprs: usize,
    pub(super) items: usize,
    pub(super) item_lists: usize,
    pub(super) parts: usize,
    pub(super) types: usize,
    pub(super) fields: usize,
    pub(super) param_tys: usize,
    pub(super) type_args: usize,
    pub(super) paths: usize,
    pub(super) fns: usize,
    pub(super) params: usize,
    pub(super) args: usize,
    pub(super) with_bindings: usize,
    pub(super) elements: usize,
    pub(super) fors: usize,
    pub(super) recovers: usize,
    pub(super) labels: usize,
    pub(super) imports: usize,
    pub(super) import_members: usize,
    pub(super) def_types: usize,
}

/// Whether a tree was parsed from source with syntax errors, and so how far
/// its shape can be trusted (see the parser's [Errors and
/// recovery](crate::parser::recovery#errors-and-recovery)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Syntax {
    /// No error: the tree is the source as written.
    #[default]
    Clean,
    /// Errors, each in an error node ([`ExprKind::Error`], [`ItemKind::Error`],
    /// [`TypeExprKind::Invalid`](super::TypeExprKind::Invalid)) or left out of
    /// the tree; everything else as written.
    Recovered,
    /// Errors that leave unknown which block some items belong to: a `}`
    /// that closes no block, or the end of the source inside a block. The
    /// tree is not checked.
    Unbalanced,
}

impl AstBuilder {
    /// Creates an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty builder with room for `capacity` expression nodes.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            exprs: Vec::with_capacity(capacity),
            ..Self::default()
        }
    }

    /// Adds `expr` to the arena and returns its id.
    ///
    /// An arena holds at most `u32::MAX` nodes. Every node consumes at least
    /// one byte of source and sources are limited to `u32::MAX` bytes, so a
    /// parser can never reach the limit; past it, ids would saturate. The
    /// same holds for the item and type arenas.
    pub fn alloc(&mut self, expr: Expr) -> ExprId {
        let id = ExprId(next_id(self.exprs.len()));
        self.exprs.push(expr);
        id
    }

    /// Adds `item` to the item arena and returns its id.
    pub fn alloc_item(&mut self, item: Item) -> ItemId {
        let id = ItemId(next_id(self.items.len()));
        self.items.push(item);
        id
    }

    /// Adds `ty` to the type arena and returns its id.
    pub fn alloc_type(&mut self, ty: TypeExpr) -> TypeExprId {
        self.types.alloc(ty)
    }

    /// Appends `items` to the item-list pool as one list and returns its
    /// range.
    pub fn push_item_list(&mut self, items: &[ItemId]) -> ItemRange {
        let start = next_id(self.item_lists.len());
        self.item_lists.extend_from_slice(items);
        ItemRange {
            start,
            len: next_id(items.len()),
        }
    }

    /// Appends `parts` to the part pool as one interpolated string's parts
    /// and returns their range.
    pub fn push_parts(&mut self, parts: &[StrPart]) -> PartRange {
        let start = next_id(self.parts.len());
        self.parts.extend_from_slice(parts);
        PartRange {
            start,
            len: next_id(parts.len()),
        }
    }

    /// Appends `fields` to the field-type pool as one record type's fields
    /// and returns their range.
    pub fn push_field_tys(&mut self, fields: &[FieldTy]) -> FieldTyRange {
        self.types.push_fields(fields)
    }

    /// Appends `segments` to the path pool as one path and returns its
    /// range. A path is never empty, so `segments` must not be.
    pub fn push_path(&mut self, segments: &[PathSegment]) -> PathRange {
        let start = next_id(self.paths.len());
        self.paths.extend_from_slice(segments);
        PathRange {
            start,
            len: next_id(segments.len()),
        }
    }

    /// Adds the import item `decl` to the import arena and returns its id,
    /// which its [`ItemKind::Import`] then carries.
    pub fn push_import(&mut self, decl: ImportDecl) -> ImportId {
        let id = ImportId(next_id(self.imports.len()));
        self.imports.push(decl);
        id
    }

    /// Appends `members` to the import-member pool as one `from` import's
    /// members and returns their range.
    pub fn push_import_members(&mut self, members: &[ImportMember]) -> ImportMemberRange {
        let start = next_id(self.import_members.len());
        self.import_members.extend_from_slice(members);
        ImportMemberRange {
            start,
            len: next_id(members.len()),
        }
    }

    /// Adds the function expression or `lazy` `decl` to the function arena
    /// and returns its id, which its [`ExprKind::Fn`] or [`ExprKind::Lazy`]
    /// node then carries.
    pub fn push_fn(&mut self, decl: FnDecl) -> FnId {
        let id = FnId(next_id(self.fns.len()));
        self.fns.push(decl);
        id
    }

    /// Adds the `recover` expression `decl` to the recover arena and
    /// returns its id, which its [`ExprKind::Recover`] node then carries.
    pub fn push_recover(&mut self, decl: RecoverDecl) -> RecoverId {
        let id = RecoverId(next_id(self.recovers.len()));
        self.recovers.push(decl);
        id
    }

    /// Adds the `for` expression `decl` to the `for` arena and returns its
    /// id, which its [`ExprKind::For`] node then carries.
    pub fn push_for(&mut self, decl: ForDecl) -> ForId {
        let id = ForId(next_id(self.fors.len()));
        self.fors.push(decl);
        id
    }

    /// Adds the label of a labelled block to the label arena and returns
    /// its id, which the block's [`ExprKind::Block`] then carries.
    pub fn push_label(&mut self, label: Label) -> LabelId {
        self.labels.push(label);
        // The arena's length after the push, so never zero; saturating as
        // every arena does (see [`AstBuilder::alloc`]).
        LabelId(NonZeroU32::MIN.saturating_add(next_id(self.labels.len()) - 1))
    }

    /// Appends `params` to the parameter pool as one function's parameters
    /// and returns their range.
    pub fn push_params(&mut self, params: &[Param]) -> ParamRange {
        let start = next_id(self.params.len());
        self.params.extend_from_slice(params);
        ParamRange {
            start,
            len: next_id(params.len()),
        }
    }

    /// Appends `bindings` to the pool of `with` bindings as one `with`'s
    /// and returns their range.
    pub fn push_with_bindings(&mut self, bindings: &[WithBinding]) -> WithRange {
        let start = next_id(self.with_bindings.len());
        self.with_bindings.extend_from_slice(bindings);
        WithRange {
            start,
            len: next_id(bindings.len()),
        }
    }

    /// Appends `args` to the argument pool as one call's arguments and
    /// returns their range. A trailing argument must be the last.
    pub fn push_args(&mut self, args: &[Arg]) -> ArgRange {
        let start = next_id(self.args.len());
        self.args.extend_from_slice(args);
        ArgRange {
            start,
            len: next_id(args.len()),
        }
    }

    /// Appends `params` to the parameter-type pool as one function type's
    /// parameters and returns their range.
    pub fn push_param_tys(&mut self, params: &[ParamTy]) -> ParamTyRange {
        self.types.push_params(params)
    }

    /// Appends `args` to the type-argument pool as one applied type's
    /// arguments and returns their range.
    pub fn push_type_args(&mut self, args: &[TypeExprId]) -> TypeArgRange {
        self.types.push_args(args)
    }

    /// Appends `elements` to the element pool as one list literal's
    /// elements and returns their range.
    pub fn push_elements(&mut self, elements: &[Element]) -> ElementRange {
        let start = next_id(self.elements.len());
        self.elements.extend_from_slice(elements);
        ElementRange {
            start,
            len: next_id(elements.len()),
        }
    }

    /// Marks the item `id` `#[shadowable]` (see [Attributes](crate::ast#attributes)).
    pub fn mark_shadowable(&mut self, id: ItemId) {
        let at = self.shadowable.partition_point(|&marked| marked.0 < id.0);
        if self.shadowable.get(at) != Some(&id) {
            self.shadowable.insert(at, id);
        }
    }

    /// Marks the `type` item `id` `#[attribute]` (see the parser's
    /// [Attributes](crate::parser::attributes#attributes)).
    pub fn mark_attribute(&mut self, id: ItemId) {
        let at = self
            .attribute_marks
            .partition_point(|&marked| marked.0 < id.0);
        if self.attribute_marks.get(at) != Some(&id) {
            self.attribute_marks.insert(at, id);
        }
    }

    /// Marks the `type` item `id` `#[defers]`: its attribute defers the
    /// calls of the functions it is on (see the parser's
    /// [Attributes](crate::parser::attributes#attributes)).
    pub fn mark_defers(&mut self, id: ItemId) {
        let at = self.defers_marks.partition_point(|&marked| marked.0 < id.0);
        if self.defers_marks.get(at) != Some(&id) {
            self.defers_marks.insert(at, id);
        }
    }

    /// Records the type parameters of a generic function item.
    pub fn push_generics(&mut self, generics: Generics) {
        self.generics.push(generics);
    }

    /// Records a method of the outermost block.
    pub fn push_method(&mut self, method: MethodDecl) {
        self.methods.push(method);
    }

    /// Records `context`, a context the file declares.
    pub fn push_context(&mut self, context: ContextDecl) {
        self.contexts.push(context);
    }

    /// The number of type expressions made so far: the number of the next.
    #[must_use]
    pub fn type_count(&self) -> u32 {
        next_id(self.types.len())
    }

    /// Records `used`, a use of an attribute that is none of the parser's,
    /// on the item it applies to, after the item's earlier ones.
    pub fn push_attribute_use(&mut self, used: AttributeUse) {
        let at = self
            .attribute_uses
            .partition_point(|earlier| earlier.item.0 <= used.item.0);
        self.attribute_uses.insert(at, used);
    }

    /// Marks the item `id` `#[def_type]` (see [Attributes](crate::ast#attributes)).
    pub fn mark_def_type(&mut self, id: ItemId) {
        let at = self
            .def_type_marks
            .partition_point(|&marked| marked.0 < id.0);
        if self.def_type_marks.get(at) != Some(&id) {
            self.def_type_marks.insert(at, id);
        }
    }

    /// Appends `def_type`, a def-type definition's or reference's details,
    /// to the def-type arena and returns its id (see
    /// [Def-types](crate::ast#def-types)).
    pub fn push_def_type(&mut self, def_type: DefTypeUse) -> DefTypeId {
        let id = DefTypeId(next_id(self.def_types.len()));
        self.def_types.push(def_type);
        id
    }

    /// Records that the file is marked `#![no_prelude]`, written at `span`
    /// (see [Attributes](crate::ast#attributes)).
    pub fn set_no_prelude(&mut self, span: Span) {
        self.no_prelude = Some(span);
    }

    /// Where the file's `#![no_prelude]` is written, if it is marked so
    /// already.
    #[must_use]
    pub fn no_prelude(&self) -> Option<Span> {
        self.no_prelude
    }

    /// Records that a field's name is written quoted, as the string literal
    /// at `span` (see [`Ast::written_name`]).
    pub fn push_quoted(&mut self, span: Span) {
        self.quoted.push(span);
    }

    /// Records that the tree has syntax errors of at least the kind
    /// `syntax` (see [`Ast::syntax`]): a tree is as bad as its worst error.
    pub fn set_syntax(&mut self, syntax: Syntax) {
        self.syntax = self.syntax.max(syntax);
    }

    /// The syntax errors recorded so far ([`AstBuilder::set_syntax`]).
    #[must_use]
    pub fn syntax(&self) -> Syntax {
        self.syntax
    }

    /// Puts back what [`AstBuilder::syntax`] was, as a parser does that
    /// parses again what it found errors in, forgetting them.
    pub fn reset_syntax(&mut self, syntax: Syntax) {
        self.syntax = syntax;
    }

    /// Where every arena and pool is now, for [`AstBuilder::truncate`].
    #[must_use]
    pub fn mark(&self) -> BuilderMark {
        BuilderMark {
            exprs: self.exprs.len(),
            items: self.items.len(),
            item_lists: self.item_lists.len(),
            parts: self.parts.len(),
            types: self.types.exprs.len(),
            fields: self.types.fields.len(),
            param_tys: self.types.params.len(),
            type_args: self.types.args.len(),
            paths: self.paths.len(),
            fns: self.fns.len(),
            params: self.params.len(),
            args: self.args.len(),
            with_bindings: self.with_bindings.len(),
            elements: self.elements.len(),
            fors: self.fors.len(),
            recovers: self.recovers.len(),
            labels: self.labels.len(),
            imports: self.imports.len(),
            import_members: self.import_members.len(),
            def_types: self.def_types.len(),
        }
    }

    /// Forgets everything allocated since `mark`, and the marks of the
    /// items forgotten: what a parser does with the nodes of source it
    /// could not parse, so that no node of the tree is unreachable and a
    /// subtree's nodes stay contiguous ([`Ast::subtree`]).
    pub fn truncate(&mut self, mark: BuilderMark) {
        self.exprs.truncate(mark.exprs);
        self.items.truncate(mark.items);
        self.item_lists.truncate(mark.item_lists);
        self.parts.truncate(mark.parts);
        self.types.exprs.truncate(mark.types);
        self.types.fields.truncate(mark.fields);
        self.types.params.truncate(mark.param_tys);
        self.types.args.truncate(mark.type_args);
        self.paths.truncate(mark.paths);
        self.fns.truncate(mark.fns);
        self.params.truncate(mark.params);
        self.args.truncate(mark.args);
        self.with_bindings.truncate(mark.with_bindings);
        self.elements.truncate(mark.elements);
        self.fors.truncate(mark.fors);
        self.recovers.truncate(mark.recovers);
        self.labels.truncate(mark.labels);
        self.imports.truncate(mark.imports);
        self.import_members.truncate(mark.import_members);
        self.def_types.truncate(mark.def_types);
        let items = next_id(mark.items);
        self.shadowable.retain(|item| item.0 < items);
        self.def_type_marks.retain(|item| item.0 < items);
        self.attribute_marks.retain(|item| item.0 < items);
        self.defers_marks.retain(|item| item.0 < items);
        self.attribute_uses.retain(|used| used.item.0 < items);
        let fns = next_id(mark.fns);
        self.generics.retain(|generics| match generics.owner {
            GenericOwner::Fn(function) => function.0 < fns,
            GenericOwner::Type(item) => item.0 < items,
        });
        self.methods.retain(|method| method.item.0 < items);
        self.contexts.retain(|context| context.item.0 < items);
    }

    /// Returns the node `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not allocated by this builder.
    #[must_use]
    pub fn get(&self, id: ExprId) -> &Expr {
        &self.exprs[id.index()]
    }

    /// Returns the item `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not allocated by this builder.
    #[must_use]
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id.index()]
    }

    /// Returns the type expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not allocated by this builder.
    #[must_use]
    pub fn type_expr(&self, id: TypeExprId) -> &TypeExpr {
        self.types.get(id)
    }

    /// Returns the items of the list `range`, in source order.
    ///
    /// # Panics
    ///
    /// Panics if `range` was not allocated by this builder.
    #[must_use]
    pub fn items(&self, range: ItemRange) -> &[ItemId] {
        let start = range.start as usize;
        &self.item_lists[start..start + range.len as usize]
    }

    /// Finishes the arenas with `root` as the file's body, taking ownership
    /// of the `names` its identifiers were interned into and the `strings`
    /// holding the text of its string literals.
    #[must_use]
    pub fn finish(self, root: ItemRange, names: Names, strings: Strings) -> Ast {
        let mut quoted = self.quoted;
        quoted.sort_unstable_by_key(|span| (span.end, span.start));
        quoted.dedup();
        Ast {
            quoted,
            exprs: self.exprs,
            items: self.items,
            item_lists: self.item_lists,
            parts: self.parts,
            types: self.types,
            paths: self.paths,
            fns: self.fns,
            params: self.params,
            args: self.args,
            with_bindings: self.with_bindings,
            elements: self.elements,
            fors: self.fors,
            recovers: self.recovers,
            labels: self.labels,
            imports: self.imports,
            import_members: self.import_members,
            shadowable: self.shadowable,
            def_type_marks: self.def_type_marks,
            attribute_marks: self.attribute_marks,
            defers_marks: self.defers_marks,
            attribute_uses: self.attribute_uses,
            generics: self.generics,
            methods: self.methods,
            contexts: self.contexts,
            def_types: self.def_types,
            no_prelude: self.no_prelude,
            syntax: self.syntax,
            implicit_import: None,
            expansion: Expansion::default(),
            root,
            names,
            strings,
        }
    }
}
