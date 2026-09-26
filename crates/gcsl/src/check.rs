//! The type checker and name resolver.
//!
//! One pass over the tree both types every expression and resolves every
//! variable. Its results are side tables indexed by the AST's ids:
//! [`Types`], the type of every expression, and [`Resolution`], the storage
//! slot of every variable use and declaration. Both are returned together as
//! [`Checked`]. Types are [`TyId`]s interned into a [`TypeTable`] the caller
//! owns, so that a [`crate::session::Session`] can keep one table across
//! inputs.
//!
//! # Types
//!
//! `Int` and `Float` never mix implicitly. The one exception is that integer
//! literals adapt to their context: a literal starts as
//! [`TyId::UNTYPED_INT`], and an operator that needs a concrete type for it
//! decides one. The checker works in two directions:
//!
//! - **Up**: [`Checker::check_expr`] computes each node's type from its
//!   children's. Operators whose operands are all untyped produce
//!   `UntypedInt` themselves where the rules allow, so `1 + 2` stays untyped,
//!   and so does a block whose result is untyped.
//! - **Down**: when an untyped subtree meets a concrete type, as in
//!   `(1 + 2) * 3.0` or `let x: Float = 5`, [`Checker::coerce`] walks the
//!   untyped subtree and rewrites every entry in it to the decided type. It
//!   stops at nodes that are already concrete, so each node is rewritten at
//!   most once and the whole check stays linear.
//!
//! A variable must have a concrete type, so an untyped initialiser defaults
//! to `Int` where it is bound: after `let x = 5`, `x` is an `Int`, and
//! `x + 2.5` is an error. Finalisation defaults an untyped file result to
//! `Int` in the same way, and so do the other places that need a concrete
//! type but give no context: two untyped operands of a comparison, and an
//! interpolated expression. Every untyped node is either coerced by the
//! nearest concrete context or defaulted where it is bound or used, so
//! afterwards the table contains no `UntypedInt`.
//!
//! An `if` with `else` has the type its branches agree on, with untyped
//! branches adapting as operands of `+` do; if both are untyped, the `if`
//! stays untyped, like a block, and `coerce` pushes a type into both
//! branches. Two record branches agree on their join (see
//! [`TypeTable::join`]), so they never mismatch. An `if` without `else` is
//! `{}`, and so must its block be. There is no truthiness: conditions and
//! the operands of `and`, `or` and `not` must be `Bool`.
//!
//! A node whose operator rejects its operands is reported once and given
//! [`TyId::ERROR`]. Every operator accepts `Error` operands silently and
//! produces `Error` itself, so the mistake does not cascade into a spurious
//! error at each enclosing operator. A variable whose initialiser has an
//! error is still declared, with type `Error`, and a use of an undefined
//! variable has type `Error`, so neither cascades either. Independent
//! mistakes elsewhere in the tree are still reported, so one run shows every
//! real error.
//!
//! In a diagnostic's message and notes, a type written as a name, such as
//! `Int`, is shown plainly, and a record type in backticks, since it
//! contains spaces and punctuation: "cannot add Int and `{}`". Labels show
//! every type plainly: "expected {x: Int}, found Int".
//!
//! # Records
//!
//! A block with at least one field item is a *record block*. Its value is
//! the record of its fields, so it has no result: a `= EXPR` in it is an
//! error, and a final expression statement is a statement like any other.
//! A field is declared like a variable: it takes a slot, is visible from the
//! next item to the end of its block, shadows and is shadowed like one, and
//! is a binding marked as a field so that `:=` on it can be rejected. Each
//! block also records its fields in order in one flat vector of field
//! declarations, as it does its bindings, and at the end of a record block
//! the block's type is the record of those fields.
//!
//! Where a type is *expected* (an annotation or an ascription), a subtype is
//! accepted (see [`TypeTable::is_subtype`]). A record block written where a
//! record type is expected, through any parentheses, is checked against it
//! directly by `Checker::check_expr_expecting`: each field's initialiser
//! is checked against the expected field's type, so literals adapt and
//! nested record blocks are checked the same way; a missing field is an
//! error, and a field the type lacks is a *warning*, since nothing with the
//! expected type can see it. Warnings are collected apart from errors and
//! returned with a successful check.
//!
//! # Scopes
//!
//! Every block, including the file's own body, is a scope. A `let` declares
//! a variable visible from the next item to the end of its block; a later
//! `let` of the same name shadows it, in the same block or an inner one. A
//! `:=` is a new declaration too, allowed only for a name already declared
//! in the same block, and not for a field.
//!
//! The scopes in effect form a stack, kept as one flat vector of bindings
//! with the innermost block's bindings last: entering a block records where
//! its bindings start, and leaving it truncates the vector back. Lookup
//! searches the vector from the end, which visits the innermost block first
//! and, within a block, the latest declaration first, so shadowing needs no
//! extra work. Blocks are short, so a linear search beats a hash map, and
//! the one vector means entering a block allocates nothing.
//!
//! # Slots
//!
//! Every variable and field is stored in a [`Slot`] of one flat frame,
//! assigned stack-fashion, as in Lua and clox: entering a block records the
//! next free slot; each declaration (`let`, a field, and `:=`, which is a
//! new binding) takes the next free slot; leaving the block restores the
//! next free slot to what it was on entry, so the slots of a finished block
//! are reused by whatever comes after it. The highest slot count reached is
//! the frame size. A variable's slot is therefore known statically and never
//! changes, and evaluation needs no environment lookups and no allocation
//! per block.
//!
//! # Sessions
//!
//! A [`Checker`] can start from an existing outermost [`Scope`], which it
//! returns extended by the new input's top-level declarations. This is what
//! lets a [`crate::session::Session`] check each input in the context of the
//! variables its earlier inputs declared, with the same code that checks a
//! file.

use std::fmt;

use crate::ast::{
    Annotate, Ast, BinaryOp, ExprId, ExprKind, ItemId, ItemKind, ItemRange, PartRange, StrPart,
    TypeExprId, TypeExprKind, UnaryOp,
};
use crate::diagnostic::{self, Diagnostic, InternalError};
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::{TyId, TyKind, TypeTable};

/// The type of every expression in an [`Ast`], indexed by [`ExprId`].
///
/// A `Types` returned by [`check`] contains neither [`TyId::UNTYPED_INT`]
/// nor [`TyId::ERROR`], nor any union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    types: Vec<TyId>,
}

impl Types {
    /// Wraps a raw table. Only the checker (and tests) build tables.
    pub(crate) fn new(types: Vec<TyId>) -> Self {
        Self { types }
    }

    /// The type of expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn get(&self, id: ExprId) -> TyId {
        self.types[id.index()]
    }

    /// Number of entries, equal to the number of nodes in the [`Ast`].
    pub(crate) fn len(&self) -> usize {
        self.types.len()
    }

    /// An [`Annotate`] that writes ` : Type` after each node, for
    /// `--dump-types`, showing the types with `table`, the table they were
    /// interned into, and `names`, the tree's names.
    #[must_use]
    pub fn annotation<'a>(&'a self, table: &'a TypeTable, names: &'a Names) -> TypeAnnotation<'a> {
        TypeAnnotation {
            types: self,
            table,
            names,
        }
    }
}

/// Writes ` : Type` after each node. Returned by [`Types::annotation`].
#[derive(Clone, Copy, Debug)]
pub struct TypeAnnotation<'a> {
    types: &'a Types,
    table: &'a TypeTable,
    names: &'a Names,
}

impl Annotate for TypeAnnotation<'_> {
    fn annotate(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            " : {}",
            self.table.display(self.types.get(id), self.names)
        )
    }
}

/// The index of a variable's storage in the evaluator's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot(u32);

impl Slot {
    /// The absence of a slot, in [`Resolution`] entries for nodes that are
    /// not variables.
    pub const NONE: Slot = Slot(u32::MAX);

    /// The slot numbered `number`. Slots are normally assigned by the
    /// checker; this is for building IR by hand in tests.
    #[cfg(test)]
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The slot's number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The slot as an index into a frame.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Where every variable of an [`Ast`] is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The slot each `Name` expression reads, indexed by [`ExprId`];
    /// [`Slot::NONE`] for every other expression.
    expr_slots: Vec<Slot>,
    /// The slot each `Let`, `Field` and `Rebind` item writes, indexed by
    /// [`ItemId`]; [`Slot::NONE`] for every other item.
    item_slots: Vec<Slot>,
    /// The number of slots the frame needs.
    frame_size: u32,
    /// For each slot, the name of the last variable assigned to it.
    slot_names: Vec<Name>,
}

impl Resolution {
    /// The slot read by expression `id`, or [`Slot::NONE`] if it is not a
    /// variable use.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn expr_slot(&self, id: ExprId) -> Slot {
        self.expr_slots[id.index()]
    }

    /// The slot written by item `id`, or [`Slot::NONE`] if it is not a
    /// declaration.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn item_slot(&self, id: ItemId) -> Slot {
        self.item_slots[id.index()]
    }

    /// The number of slots the frame needs: one more than the highest slot
    /// used.
    #[must_use]
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// For each slot, the name of the last variable assigned to it. Only
    /// for showing slots to people, as `--dump-ir` does.
    #[must_use]
    pub fn slot_names(&self) -> &[Name] {
        &self.slot_names
    }

    /// A resolution for a tree with `exprs` expressions and `items` items and
    /// no variables, for building checked trees by hand in tests.
    #[cfg(test)]
    pub(crate) fn without_variables(exprs: usize, items: usize) -> Self {
        Self {
            expr_slots: vec![Slot::NONE; exprs],
            item_slots: vec![Slot::NONE; items],
            frame_size: 0,
            slot_names: Vec::new(),
        }
    }

    /// Number of expression entries, equal to the number of expressions in
    /// the [`Ast`].
    pub(crate) fn expr_len(&self) -> usize {
        self.expr_slots.len()
    }

    /// Number of item entries, equal to the number of items in the [`Ast`].
    pub(crate) fn item_len(&self) -> usize {
        self.item_slots.len()
    }
}

/// The result of a successful check: the types and the resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The type of every expression.
    pub types: Types,
    /// Where every variable is stored.
    pub resolution: Resolution,
}

/// Everything a successful [`Checker::check`] produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOutput {
    /// The types and the resolution.
    pub checked: Checked,
    /// The outermost scope, extended with the body's top-level
    /// declarations.
    pub scope: Scope,
    /// The type of the body's value: its result's, the record of its fields,
    /// or `{}`.
    pub ty: TyId,
    /// The warnings found, in source order.
    pub warnings: Vec<Diagnostic>,
}

/// A variable or field in scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    name: Name,
    slot: Slot,
    ty: TyId,
    /// The `let`, field or `:=` item that declared it, or `None` if it was
    /// declared by an earlier input of a session, whose tree is gone.
    declared: Option<ItemId>,
    /// Whether it is a field, which `:=` may not rebind.
    is_field: bool,
}

/// A field declared in the record block being checked.
#[derive(Clone, Copy, Debug)]
struct FieldDecl {
    name: Name,
    name_span: Span,
    ty: TyId,
}

/// The record type a record block is checked against, and where the
/// expectation comes from, for [`Checker::check_expr_expecting`].
#[derive(Clone, Copy, Debug)]
struct Expecting {
    /// The expected record type.
    ty: TyId,
    /// The annotation or ascription stating it.
    context: Span,
    /// The label on `context`.
    why: &'static str,
    /// The block's closing brace, where a missing field is reported.
    close: Span,
}

/// The variables of an outermost scope, with their slots and types: what a
/// [`crate::session::Session`] carries from one input to the next.
///
/// Only a [`Checker`] produces a non-empty scope. Its bindings occupy slots
/// `0..n` in declaration order, since the outermost block's declarations are
/// the first to take slots and no block is left open around them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    bindings: Vec<Binding>,
}

impl Scope {
    /// An empty scope, as at the start of a file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of variables declared, counting shadowed ones, which is
    /// also the number of slots they occupy.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether no variable has been declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Type-checks and resolves `ast` as a file, starting from an empty scope,
/// interning its types into `types`. On success, returns the tables and the
/// warnings found, in source order.
///
/// # Errors
///
/// Returns every type and scope error found, together with the warnings, in
/// source order. Checking continues after an error by giving the offending
/// node [`TyId::ERROR`], which every enclosing operator accepts silently, so
/// one run reports each independent error exactly once.
pub fn check(
    ast: &Ast,
    types: &mut TypeTable,
) -> Result<(Checked, Vec<Diagnostic>), Vec<Diagnostic>> {
    Checker::new(ast, types)
        .check()
        .map(|output| (output.checked, output.warnings))
}

/// The type names every program can use, as `(name, type)` pairs. A name
/// the program never mentions has no [`Name`].
type BuiltinTypes = [(Option<Name>, TyId); 4];

/// A check in progress. See the module documentation.
pub struct Checker<'a> {
    ast: &'a Ast,
    /// Where types are interned.
    types: &'a mut TypeTable,
    /// The type of each expression, indexed by [`ExprId`].
    expr_types: Vec<TyId>,
    expr_slots: Vec<Slot>,
    item_slots: Vec<Slot>,
    /// Every variable and field in scope, outermost block first.
    bindings: Vec<Binding>,
    /// Index in `bindings` of the innermost block's first binding.
    block_start: usize,
    /// The fields of every block being checked, outermost block first.
    fields: Vec<FieldDecl>,
    /// Index in `fields` of the innermost block's first field.
    fields_start: usize,
    /// The next free slot.
    next_slot: u32,
    /// The highest `next_slot` reached.
    frame_size: u32,
    slot_names: Vec<Name>,
    builtin_types: BuiltinTypes,
    /// Reused for sorting the fields of a record block or a record type by
    /// name: each field's name, its position in source order and its type.
    /// Nested uses push above the entries of the use enclosing them and
    /// truncate back, so it is a stack and one allocation serves them all.
    sorted_fields: Vec<(Name, u32, TyId)>,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    /// A checker for `ast` as a file, starting from an empty scope and
    /// interning types into `types`.
    #[must_use]
    pub fn new(ast: &'a Ast, types: &'a mut TypeTable) -> Self {
        Self::with_scope(ast, Scope::new(), types)
    }

    /// A checker for `ast` whose outermost block starts with the variables
    /// of `scope` already declared. `scope` must come from checking earlier
    /// input interned into the same [`crate::names::Names`] as `ast` and the
    /// same [`TypeTable`] as `types`.
    #[must_use]
    pub fn with_scope(ast: &'a Ast, scope: Scope, types: &'a mut TypeTable) -> Self {
        let names = ast.names();
        let slot_names: Vec<Name> = scope.bindings.iter().map(|binding| binding.name).collect();
        // The scope's bindings occupy exactly slots `0..len`.
        let next_slot = u32::try_from(slot_names.len()).unwrap_or(u32::MAX);
        Self {
            ast,
            types,
            // Placeholder only: every node is written by `check_expr` before
            // it is read, and the final invariant check catches any that
            // are not.
            expr_types: vec![TyId::ERROR; ast.len()],
            expr_slots: vec![Slot::NONE; ast.len()],
            item_slots: vec![Slot::NONE; ast.item_count()],
            bindings: scope.bindings,
            block_start: 0,
            fields: Vec::new(),
            fields_start: 0,
            next_slot,
            frame_size: next_slot,
            slot_names,
            builtin_types: [
                (names.lookup("Int"), TyId::INT),
                (names.lookup("Float"), TyId::FLOAT),
                (names.lookup("Bool"), TyId::BOOL),
                (names.lookup("String"), TyId::STRING),
            ],
            sorted_fields: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Checks the file's body. On success, returns the tables, the
    /// outermost scope extended with the body's top-level declarations, the
    /// body's type and the warnings.
    ///
    /// # Errors
    ///
    /// As [`check`].
    pub fn check(mut self) -> Result<CheckOutput, Vec<Diagnostic>> {
        let root = self.ast.root();
        let ty = self.check_body(root, None);
        // Finalisation: an integer result with no context defaults to Int.
        self.coerce_result(root, TyId::INT);
        let ty = if ty == TyId::UNTYPED_INT {
            TyId::INT
        } else {
            ty
        };
        if !self.errors.is_empty() {
            return Err(diagnostic::merge(self.errors, self.warnings));
        }
        let types = &*self.types;
        if self.expr_types.iter().any(|&ty| {
            matches!(
                types.kind(ty),
                TyKind::Error | TyKind::UntypedInt | TyKind::Union(_)
            )
        }) {
            let error = InternalError {
                message: "a type table entry is untyped, erroneous or a union after a successful \
                          check",
                span: self.ast.root_span(),
            };
            return Err(vec![error.into()]);
        }
        // Item ids are meaningless once this tree is gone.
        let bindings = self
            .bindings
            .into_iter()
            .map(|binding| Binding {
                declared: None,
                ..binding
            })
            .collect();
        let checked = Checked {
            types: Types::new(self.expr_types),
            resolution: Resolution {
                expr_slots: self.expr_slots,
                item_slots: self.item_slots,
                frame_size: self.frame_size,
                slot_names: self.slot_names,
            },
        };
        let mut warnings = self.warnings;
        warnings.sort_by_key(|warning| warning.span.start);
        Ok(CheckOutput {
            checked,
            scope: Scope { bindings },
            ty,
            warnings,
        })
    }

    /// Checks a nested block's body in a scope of its own, returning its
    /// type. With `expecting`, the block is a record block checked against
    /// an expected record type.
    fn check_block(&mut self, body: ItemRange, expecting: Option<Expecting>) -> TyId {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        let ty = self.check_body(body, expecting);
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        ty
    }

    /// Checks the items of a body in the current scope, returning its type:
    /// for a record block, the record of its fields; otherwise the type of
    /// its result, the `= EXPR` or final expression statement, or `{}` if
    /// it has none.
    fn check_body(&mut self, body: ItemRange, expecting: Option<Expecting>) -> TyId {
        let items = self.ast.items(body);
        let first_field = items
            .iter()
            .copied()
            .find(|&id| matches!(self.ast.item(id).kind, ItemKind::Field { .. }));
        let mut result = TyId::EMPTY;
        for (index, &id) in items.iter().enumerate() {
            let next = items.get(index + 1).copied();
            let item = *self.ast.item(id);
            match item.kind {
                ItemKind::Let {
                    name, ty, value, ..
                } => self.check_let(id, name, ty, value),
                ItemKind::Rebind {
                    name,
                    name_span,
                    value,
                } => self.check_rebind(id, name, name_span, value),
                ItemKind::Field {
                    name,
                    name_span,
                    value,
                } => self.check_field(id, (name, name_span), value, expecting),
                ItemKind::Result(value) => {
                    let ty = self.check_expr(value);
                    let eq = Span::new(item.span.start, item.span.start + 1);
                    if let Some(first_field) = first_field {
                        self.settle(value, ty);
                        self.errors.push(
                            Diagnostic::error("a block with fields cannot have a result", eq)
                                .with_context(
                                    self.ast.item(first_field).span,
                                    "first field declared here",
                                )
                                .with_note(
                                    "a block is either a record of its fields or a single \
                                     value; bind the value with `let` or make it a field",
                                ),
                        );
                        continue;
                    }
                    match next {
                        None => result = ty,
                        Some(next) => {
                            self.settle(value, ty);
                            self.errors.push(
                                Diagnostic::error(
                                    "the result must be the last item in its block",
                                    eq,
                                )
                                .with_context(self.ast.item(next).span, "this item comes after it"),
                            );
                        }
                    }
                }
                ItemKind::Expr(value) => {
                    let ty = self.check_expr(value);
                    // In a record block, even the final expression is a
                    // statement.
                    if next.is_none() && first_field.is_none() {
                        result = ty;
                    } else if ty != TyId::EMPTY && ty != TyId::ERROR {
                        let ty = self.settle(value, ty);
                        let shown = self.in_message(ty);
                        self.errors.push(
                            Diagnostic::error(
                                format!("unused value of type {shown}"),
                                self.ast.get(value).span,
                            )
                            .with_note(
                                "make it the block's result with `= ...`, bind it with `let`, \
                                 or remove it",
                            ),
                        );
                    }
                }
            }
        }
        match first_field {
            Some(_) => self.record_type(expecting),
            None => result,
        }
    }

    /// Checks `let NAME (: TYPE)? = VALUE` and declares the variable.
    fn check_let(&mut self, id: ItemId, name: Name, annotation: Option<TypeExprId>, value: ExprId) {
        let ty = match annotation {
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
            Some(annotation) => {
                let expected = self.resolve_type(annotation);
                let context = self.ast.type_expr(annotation).span;
                let found = self.check_expr_expecting(
                    value,
                    expected,
                    context,
                    "expected because of this annotation",
                );
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
        };
        self.declare(name, ty, id, false);
    }

    /// Checks the field item `id`, `NAME = VALUE`, whose name is written at
    /// `name_span`, and declares the field. With `expecting`, its block is
    /// checked against an expected record type: a field the type has is
    /// checked against that field's type and then has it, and one the type
    /// lacks is a warning.
    fn check_field(
        &mut self,
        id: ItemId,
        (name, name_span): (Name, Span),
        value: ExprId,
        expecting: Option<Expecting>,
    ) {
        let expected = expecting.map(|expecting| (expecting, self.types.field(expecting.ty, name)));
        let ty = match expected {
            Some((expecting, Some(expected))) => {
                let found =
                    self.check_expr_expecting(value, expected, expecting.context, expecting.why);
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
            Some((expecting, None)) => {
                let found = self.check_expr(value);
                let ty = self.settle(value, found);
                let text = self.ast.names().get(name);
                let record = self.in_message(expecting.ty);
                self.warnings.push(
                    Diagnostic::warning(
                        format!("field `{}` is not in {record}", FieldName(text)),
                        name_span,
                    )
                    .with_context(expecting.context, expecting.why)
                    .with_note("it is allowed, but nothing with this type can see it"),
                );
                ty
            }
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
        };
        self.declare(name, ty, id, true);
        self.fields.push(FieldDecl {
            name,
            name_span,
            ty,
        });
    }

    /// The type of the record block whose fields are the innermost block's,
    /// once they are all declared: the record of the fields, or `Error` if
    /// a field is defined twice or has an erroneous type. With `expecting`,
    /// also reports each field of the expected type that the block lacks,
    /// which makes the type `Error` too.
    fn record_type(&mut self, expecting: Option<Expecting>) -> TyId {
        let fields = self.fields.get(self.fields_start..).unwrap_or_default();
        let base = self.sorted_fields.len();
        self.sorted_fields.extend(
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| (field.name, position(index), field.ty)),
        );
        let mut erroneous = fields.iter().any(|field| field.ty == TyId::ERROR);
        let sorted = self.sorted_fields.get_mut(base..).unwrap_or_default();
        sorted.sort_unstable_by_key(|&(name, index, _)| (name, index));
        // A name's first definition sorts first among its definitions, and
        // each later one is reported against it.
        let mut first: Option<(Name, &FieldDecl)> = None;
        for &(name, index, _) in &*sorted {
            let Some(field) = fields.get(index as usize) else {
                continue;
            };
            match first {
                Some((first_name, first)) if first_name == name => {
                    let text = self.ast.names().get(name);
                    self.errors.push(
                        Diagnostic::error(
                            format!("field `{}` is defined twice", FieldName(text)),
                            field.name_span,
                        )
                        .with_context(first.name_span, "first defined here"),
                    );
                    erroneous = true;
                }
                _ => first = Some((name, field)),
            }
        }
        if let Some(expecting) = expecting {
            erroneous |= self.report_missing_fields(base, expecting);
        }
        let ty = if erroneous {
            TyId::ERROR
        } else {
            let sorted = self.sorted_fields.get(base..).unwrap_or_default();
            self.types
                .record(sorted.iter().map(|&(name, _, ty)| (name, ty)))
        };
        self.sorted_fields.truncate(base);
        ty
    }

    /// Reports each field of `expecting`'s record type that is not among the
    /// block's fields, `sorted_fields[base..]`, in alphabetical order.
    /// Returns whether any was missing.
    fn report_missing_fields(&mut self, base: usize, expecting: Expecting) -> bool {
        let names = self.ast.names();
        let present = self.sorted_fields.get(base..).unwrap_or_default();
        let mut missing: Vec<&str> = self
            .types
            .record_fields(expecting.ty)
            .unwrap_or_default()
            .iter()
            .filter(|&&(name, _)| {
                present
                    .binary_search_by_key(&name, |&(present, _, _)| present)
                    .is_err()
            })
            .map(|&(name, _)| names.get(name))
            .collect();
        if missing.is_empty() {
            return false;
        }
        missing.sort_unstable();
        let record = self.in_message(expecting.ty);
        for text in missing {
            self.errors.push(
                Diagnostic::error(
                    format!("missing field `{}` in record", FieldName(text)),
                    expecting.close,
                )
                .with_context(expecting.context, expecting.why)
                .with_note(format!("the type {record} needs it")),
            );
        }
        true
    }

    /// Checks `NAME := VALUE` and declares the new binding. The binding is
    /// declared even when the rebind is rejected, so that later uses of the
    /// name do not cascade into "undefined variable" errors.
    fn check_rebind(&mut self, id: ItemId, name: Name, name_span: Span, value: ExprId) {
        let text = self.ast.names().get(name);
        match self.lookup(name) {
            Some((_, binding)) if binding.is_field => {
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}`: it is a field, not a variable"),
                    name_span,
                );
                // A field from an earlier session input has no item here to
                // point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(self.ast.item(declared).span, "declared here");
                }
                self.errors.push(error.with_note(format!(
                    "rebind with `{text} = ...` is not allowed either: a field is defined once"
                )));
            }
            Some((index, _)) if index >= self.block_start => {}
            Some((_, binding)) => {
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}` here: it is declared in an enclosing block"),
                    name_span,
                );
                // A binding from an earlier session input has no item here
                // to point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(self.ast.item(declared).span, "declared here");
                }
                let error = error.with_note(format!("shadow it with `let {text} = ...` instead"));
                self.errors.push(error);
            }
            None => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: no variable named `{text}` is declared"),
                        name_span,
                    )
                    .with_note(format!("declare it with `let {text} = ...`")),
                );
            }
        }
        let found = self.check_expr(value);
        let ty = self.settle(value, found);
        self.declare(name, ty, id, false);
    }

    /// Declares a variable, or a field if `is_field`, in the innermost
    /// block, in the next free slot.
    fn declare(&mut self, name: Name, ty: TyId, id: ItemId, is_field: bool) {
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        match self.slot_names.get_mut(slot.index()) {
            Some(entry) => *entry = name,
            // Slots are taken in order, so a slot beyond the table is the
            // next one.
            None => self.slot_names.push(name),
        }
        self.item_slots[id.index()] = slot;
        self.bindings.push(Binding {
            name,
            slot,
            ty,
            declared: Some(id),
            is_field,
        });
    }

    /// The innermost, latest binding of `name`, with its index in
    /// `bindings`.
    fn lookup(&self, name: Name) -> Option<(usize, Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .rev()
            .find(|(_, binding)| binding.name == name)
            .map(|(index, &binding)| (index, binding))
    }

    /// The type a type expression denotes, or `Error` (reported) if it names
    /// no type, or is a record type with a field that names none or a name
    /// that appears twice.
    fn resolve_type(&mut self, id: TypeExprId) -> TyId {
        let ty = *self.ast.type_expr(id);
        match ty.kind {
            TypeExprKind::Named(name) => {
                if let Some(&(_, builtin)) = self
                    .builtin_types
                    .iter()
                    .find(|(builtin, _)| *builtin == Some(name))
                {
                    return builtin;
                }
                let text = self.ast.names().get(name);
                self.errors
                    .push(Diagnostic::error(format!("unknown type `{text}`"), ty.span));
                TyId::ERROR
            }
            TypeExprKind::Record(range) => {
                let fields = self.ast.field_tys(range);
                let base = self.sorted_fields.len();
                let mut erroneous = false;
                for (index, field) in fields.iter().enumerate() {
                    // Resolving a nested record type uses `sorted_fields`
                    // above this one's entries and truncates back.
                    let field_ty = self.resolve_type(field.ty);
                    erroneous |= field_ty == TyId::ERROR;
                    self.sorted_fields
                        .push((field.name, position(index), field_ty));
                }
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
                    let text = self.ast.names().get(name);
                    self.errors.push(Diagnostic::error(
                        format!("field `{}` appears twice in this type", FieldName(text)),
                        repeat.span,
                    ));
                    erroneous = true;
                }
                let resolved = if erroneous {
                    TyId::ERROR
                } else {
                    let sorted = self.sorted_fields.get(base..).unwrap_or_default();
                    self.types
                        .record(sorted.iter().map(|&(name, _, ty)| (name, ty)))
                };
                self.sorted_fields.truncate(base);
                resolved
            }
        }
    }

    /// Checks that `expr`, of type `found`, can have type `expected`: an
    /// untyped integer is coerced to `Int` or `Float`, and any other type
    /// must be a subtype of `expected`. A mismatch is reported at `expr`
    /// with a context label on `context`, the source of the expectation.
    fn expect(&mut self, expr: ExprId, found: TyId, expected: TyId, context: Span, why: &str) {
        match (found, expected) {
            (TyId::ERROR, _) => {}
            // The expected type is unknown and has been reported.
            (_, TyId::ERROR) => {
                self.settle(expr, found);
            }
            (TyId::UNTYPED_INT, TyId::INT | TyId::FLOAT) => self.coerce(expr, expected),
            _ if self.types.is_subtype(found, expected) => {}
            _ => {
                let found = self.settle(expr, found);
                let label = format!(
                    "expected {}, found {}",
                    self.in_label(expected),
                    self.in_label(found)
                );
                self.errors.push(
                    Diagnostic::error("mismatched types", self.ast.get(expr).span)
                        .with_label(label)
                        .with_context(context, why),
                );
            }
        }
    }

    /// Checks `id` where a value of type `expected` is expected, as stated
    /// at `context`, labelled `why`, and returns the type it has.
    ///
    /// If `expected` is a record type and `id` is, through any parentheses,
    /// a record block, the block is checked against it field by field (see
    /// the module documentation) and its type, and that of each parenthesis
    /// around it, is the record of its fields, a subtype of `expected`, or
    /// `Error` if a field is missing. Otherwise `id` is checked bottom-up
    /// and then must be a subtype of `expected`, as [`Checker::expect`]
    /// says.
    fn check_expr_expecting(
        &mut self,
        id: ExprId,
        expected: TyId,
        context: Span,
        why: &'static str,
    ) -> TyId {
        if self.types.is_record(expected)
            && let Some((block, body)) = self.record_block_within(id)
        {
            let span = self.ast.get(block).span;
            let expecting = Expecting {
                ty: expected,
                context,
                why,
                close: Span::new(span.end.saturating_sub(1), span.end),
            };
            let ty = self.check_block(body, Some(expecting));
            let mut node = id;
            loop {
                self.expr_types[node.index()] = ty;
                match self.ast.get(node).kind {
                    ExprKind::Paren(inner) => node = inner,
                    _ => break,
                }
            }
            return ty;
        }
        let found = self.check_expr(id);
        self.expect(id, found, expected, context, why);
        found
    }

    /// The record block that `id` is, through any parentheses, with its
    /// body, or `None` if it is not one.
    fn record_block_within(&self, id: ExprId) -> Option<(ExprId, ItemRange)> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block(body) if self.is_record_body(body) => return Some((node, body)),
                _ => return None,
            }
        }
    }

    /// Whether `body` has a field item, which makes it a record block.
    fn is_record_body(&self, body: ItemRange) -> bool {
        self.ast
            .items(body)
            .iter()
            .any(|&id| matches!(self.ast.item(id).kind, ItemKind::Field { .. }))
    }

    /// Computes, records and returns the type of `id`, bottom-up.
    fn check_expr(&mut self, id: ExprId) -> TyId {
        let expr = self.ast.get(id);
        let ty = match expr.kind {
            ExprKind::Int(_) => TyId::UNTYPED_INT,
            ExprKind::Float(_) => TyId::FLOAT,
            ExprKind::Bool(_) => TyId::BOOL,
            ExprKind::Str(_) => TyId::STRING,
            ExprKind::Interp(parts) => self.check_interpolation(parts),
            ExprKind::If {
                cond,
                then,
                else_,
                has_else,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                self.check_if(keyword, cond, (then, else_), has_else)
            }
            ExprKind::Paren(inner) => self.check_expr(inner),
            ExprKind::Unary { op, operand } => {
                let operand_ty = self.check_expr(operand);
                let op_span = expr.operator_span().unwrap_or(expr.span);
                self.unary(op, op_span, (operand, operand_ty))
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match op {
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                        self.arithmetic(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
                    }
                    BinaryOp::Div => self.division(id, (lhs, lhs_ty), (rhs, rhs_ty)),
                    BinaryOp::FloorDiv | BinaryOp::Rem => {
                        self.integer_only(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
                    }
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge => self.comparison(id, op, (lhs, lhs_ty), (rhs, rhs_ty)),
                    BinaryOp::And | BinaryOp::Or => {
                        let op_span = expr.operator_span().unwrap_or(expr.span);
                        self.logic_operand(op.verb(), op_span, lhs, lhs_ty);
                        self.logic_operand(op.verb(), op_span, rhs, rhs_ty);
                        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
                            TyId::ERROR
                        } else {
                            TyId::BOOL
                        }
                    }
                }
            }
            ExprKind::Name(name) => self.check_name(id, name, expr.span),
            ExprKind::Block(body) => self.check_block(body, None),
            ExprKind::Field { expr: record, name } => {
                let record_ty = self.check_expr(record);
                self.field_access(id, (record, record_ty), name)
            }
            ExprKind::Ascribe { expr: inner, ty } => {
                let expected = self.resolve_type(ty);
                let context = self.ast.type_expr(ty).span;
                let found = self.check_expr_expecting(inner, expected, context, "ascribed here");
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
        };
        self.expr_types[id.index()] = ty;
        ty
    }

    /// Types the field access `id`, reading field `name` of `record`, of
    /// type `record_ty`: the field's type. Reading a field the record type
    /// lacks, or reading any field of a value that is not a record, is
    /// reported at the `.NAME`.
    fn field_access(
        &mut self,
        id: ExprId,
        (record, record_ty): (ExprId, TyId),
        name: Name,
    ) -> TyId {
        if record_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        let expr = self.ast.get(id);
        let member = expr.member_span(self.ast).unwrap_or(expr.span);
        let text = FieldName(self.ast.names().get(name));
        if let Some(fields) = self.types.record_fields(record_ty) {
            if let Some(ty) = self.types.field(record_ty, name) {
                return ty;
            }
            let note = self.field_list(fields);
            let message = format!("no field `{text}` in {}", self.in_message(record_ty));
            self.errors
                .push(Diagnostic::error(message, member).with_note(note));
            return TyId::ERROR;
        }
        let record_ty = self.settle(record, record_ty);
        let message = format!(
            "cannot access field `{text}` of {}",
            self.in_message(record_ty)
        );
        let label = format!("this is {}", self.in_label(record_ty));
        self.errors.push(
            Diagnostic::error(message, member).with_context(self.ast.get(record).span, label),
        );
        TyId::ERROR
    }

    /// The note listing a record type's `fields`, in alphabetical order:
    /// "the fields are `x` and `y`".
    fn field_list(&self, fields: &[(Name, TyId)]) -> String {
        let names = self.ast.names();
        let mut texts: Vec<&str> = fields.iter().map(|&(name, _)| names.get(name)).collect();
        texts.sort_unstable();
        match texts.as_slice() {
            [] => "the record has no fields".to_owned(),
            [only] => format!("the only field is `{}`", FieldName(only)),
            [init @ .., last] => {
                let init: Vec<String> = init
                    .iter()
                    .map(|text| format!("`{}`", FieldName(text)))
                    .collect();
                format!(
                    "the fields are {} and `{}`",
                    init.join(", "),
                    FieldName(last)
                )
            }
        }
    }

    /// Types an `if` whose keyword is at `keyword`: its condition must be
    /// `Bool`, and its branches agree (see [`Checker::branches`]) or, without
    /// `else`, its block is `{}`.
    fn check_if(
        &mut self,
        keyword: Span,
        cond: ExprId,
        (then, else_): (ExprId, ExprId),
        has_else: bool,
    ) -> TyId {
        let cond_ty = self.check_expr(cond);
        self.expect(
            cond,
            cond_ty,
            TyId::BOOL,
            keyword,
            "`if` conditions are Bool",
        );
        let then_ty = self.check_expr(then);
        let else_ty = self.check_expr(else_);
        if has_else {
            self.branches(keyword, (then, then_ty), (else_, else_ty))
        } else {
            self.empty_branch(keyword, then, then_ty)
        }
    }

    /// Types the prefix operator `op`, at `op_span`, applied to `operand`
    /// of type `operand_ty`. Negation accepts every numeric type and
    /// preserves it; `not` needs a `Bool`.
    fn unary(&mut self, op: UnaryOp, op_span: Span, (operand, operand_ty): (ExprId, TyId)) -> TyId {
        if operand_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        match op {
            UnaryOp::Neg if is_numeric(operand_ty) => operand_ty,
            UnaryOp::Neg => {
                let message = format!("cannot {} {}", op.verb(), self.in_message(operand_ty));
                let label = self.in_label(operand_ty);
                self.errors.push(
                    Diagnostic::error(message, op_span)
                        .with_context(self.ast.get(operand).span, label),
                );
                TyId::ERROR
            }
            UnaryOp::Not => {
                self.logic_operand(op.verb(), op_span, operand, operand_ty);
                TyId::BOOL
            }
        }
    }

    /// Resolves a use of the variable `name` at expression `id`, recording
    /// its slot, and returns its type.
    fn check_name(&mut self, id: ExprId, name: Name, span: Span) -> TyId {
        if let Some((_, binding)) = self.lookup(name) {
            self.expr_slots[id.index()] = binding.slot;
            return binding.ty;
        }
        let text = self.ast.names().get(name);
        let mut error = Diagnostic::error(format!("undefined variable `{text}`"), span);
        if text.contains('-') {
            error = error.with_note(
                "names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction",
            );
        }
        self.errors.push(error);
        TyId::ERROR
    }

    /// Types `+`, `-` and `*`: both operands must end up the same numeric
    /// type, with untyped operands adapting to a concrete one, except that
    /// `+` also concatenates two strings.
    fn arithmetic(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if op == BinaryOp::Add && lhs_ty == TyId::STRING && rhs_ty == TyId::STRING {
            return TyId::STRING;
        }
        if let Some(ty) = self.numeric((lhs, lhs_ty), (rhs, rhs_ty)) {
            return ty;
        }
        let message = format!(
            "cannot {} {} and {}",
            op.verb(),
            self.in_message(lhs_ty),
            self.in_message(rhs_ty)
        );
        self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
        TyId::ERROR
    }

    /// Decides the common numeric type of two expressions that must end up
    /// the same numeric type, as the operands of arithmetic must: untyped
    /// ones adapt to a concrete `Int` or `Float`, and two untyped ones stay
    /// untyped. Returns `Some(TyId::ERROR)` without a report if either is
    /// already an error, and `None` if they cannot agree, having settled any
    /// untyped one to `Int` for the caller's report.
    fn numeric(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        match (lhs_ty, rhs_ty) {
            // An erroneous operand has been reported already. The other
            // operand still needs a concrete type in case it is untyped.
            (TyId::ERROR, _) => {
                self.coerce(rhs, TyId::INT);
                Some(TyId::ERROR)
            }
            (_, TyId::ERROR) => {
                self.coerce(lhs, TyId::INT);
                Some(TyId::ERROR)
            }
            (TyId::UNTYPED_INT, TyId::UNTYPED_INT) => Some(TyId::UNTYPED_INT),
            (TyId::UNTYPED_INT, concrete @ (TyId::INT | TyId::FLOAT)) => {
                self.coerce(lhs, concrete);
                Some(concrete)
            }
            (concrete @ (TyId::INT | TyId::FLOAT), TyId::UNTYPED_INT) => {
                self.coerce(rhs, concrete);
                Some(concrete)
            }
            (TyId::INT, TyId::INT) => Some(TyId::INT),
            (TyId::FLOAT, TyId::FLOAT) => Some(TyId::FLOAT),
            _ => {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                None
            }
        }
    }

    /// Types a comparison. Both operands must be the same type, untyped
    /// operands adapting as for arithmetic and two untyped operands
    /// settling to `Int`: any type for `==` and `!=` (two records must be
    /// exactly the same type), and `Int`, `Float` or `String` for ordering.
    /// The result is `Bool`.
    fn comparison(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        let ordering = !matches!(op, BinaryOp::Eq | BinaryOp::Ne);
        if ordering && lhs_ty != TyId::ERROR && rhs_ty != TyId::ERROR {
            // An operand of a type with no order is reported as such, before
            // any mismatch.
            let unordered = [lhs_ty, rhs_ty]
                .into_iter()
                .find(|&ty| ty == TyId::BOOL || self.types.is_record(ty));
            if let Some(unordered) = unordered {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                let message = format!("cannot order {}", self.in_message(unordered));
                self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
                return TyId::ERROR;
            }
        }
        match self.agree((lhs, lhs_ty), (rhs, rhs_ty)) {
            Some(TyId::ERROR) => TyId::ERROR,
            // Two untyped literals compare as Int.
            Some(TyId::UNTYPED_INT) => {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                TyId::BOOL
            }
            Some(_) => TyId::BOOL,
            None => {
                let message = format!(
                    "cannot {} {} and {}",
                    op.verb(),
                    self.in_message(lhs_ty),
                    self.in_message(rhs_ty)
                );
                self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
                TyId::ERROR
            }
        }
    }

    /// Decides the common type of two expressions that must have the same
    /// type: `Bool`, `String` and record types must match exactly, and
    /// numeric types agree as for [`Checker::numeric`], whose results this
    /// returns.
    fn agree(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        if lhs_ty == rhs_ty
            && (lhs_ty == TyId::BOOL || lhs_ty == TyId::STRING || self.types.is_record(lhs_ty))
        {
            Some(lhs_ty)
        } else {
            self.numeric((lhs, lhs_ty), (rhs, rhs_ty))
        }
    }

    /// Checks that `operand`, of type `ty`, of the logic operator `verb` at
    /// `op_span`, is a `Bool`.
    fn logic_operand(&mut self, verb: &str, op_span: Span, operand: ExprId, ty: TyId) {
        let why = format!("`{verb}` requires Bool operands");
        self.expect(operand, ty, TyId::BOOL, op_span, &why);
    }

    /// Types the branches of an `if` with `else`, whose keyword is at
    /// `keyword`: two record branches have their join, and otherwise the
    /// branches must agree, untyped branches adapting as operands of
    /// arithmetic do, and two untyped branches leaving the `if` untyped.
    fn branches(
        &mut self,
        keyword: Span,
        (then, then_ty): (ExprId, TyId),
        (else_, else_ty): (ExprId, TyId),
    ) -> TyId {
        let common = if self.types.is_record(then_ty) && self.types.is_record(else_ty) {
            self.types.join(then_ty, else_ty)
        } else {
            self.agree((then, then_ty), (else_, else_ty))
        };
        common.unwrap_or_else(|| {
            let then_label = format!("this is {}", self.in_label(then_ty));
            let else_label = format!("this is {}", self.in_label(else_ty));
            self.errors.push(
                Diagnostic::error("`if` branches have different types", keyword)
                    .with_context(self.ast.get(then).span, then_label)
                    .with_context(self.ast.get(else_).span, else_label),
            );
            TyId::ERROR
        })
    }

    /// Types an `if` without `else`, whose keyword is at `keyword`: it is
    /// `{}`, so its block must be too.
    fn empty_branch(&mut self, keyword: Span, then: ExprId, then_ty: TyId) -> TyId {
        match then_ty {
            TyId::EMPTY => TyId::EMPTY,
            TyId::ERROR => TyId::ERROR,
            _ => {
                let then_ty = self.settle(then, then_ty);
                let label = format!("this branch is {}", self.in_label(then_ty));
                self.errors.push(
                    Diagnostic::error("`if` without `else` must be `{}`", keyword)
                        .with_context(self.ast.get(then).span, label)
                        .with_note("add an `else` branch, or make the block's result `{}`"),
                );
                TyId::ERROR
            }
        }
    }

    /// Types an interpolated string: each interpolated expression is
    /// checked, and an untyped one settles to `Int`. The result is a
    /// `String`, or an error, without a further report, if a part is one.
    fn check_interpolation(&mut self, parts: PartRange) -> TyId {
        let mut ty = TyId::STRING;
        for &part in self.ast.parts(parts) {
            let StrPart::Expr(expr) = part else {
                continue;
            };
            let found = self.check_expr(expr);
            // A value of every type can be shown as text today, a record in
            // its `repr` form. Once some type's values cannot be, it is
            // reported here as "cannot display".
            if self.settle(expr, found) == TyId::ERROR {
                ty = TyId::ERROR;
            }
        }
        ty
    }

    /// Types `/`: it accepts any mix of numeric operands and always yields
    /// `Float`. Untyped operands become `Float` literals; concrete `Int`
    /// operands are converted by lowering.
    fn division(
        &mut self,
        id: ExprId,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        self.coerce(lhs, TyId::FLOAT);
        self.coerce(rhs, TyId::FLOAT);
        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        if !is_numeric(lhs_ty) || !is_numeric(rhs_ty) {
            let message = format!(
                "cannot {} {} and {}",
                BinaryOp::Div.verb(),
                self.in_message(lhs_ty),
                self.in_message(rhs_ty)
            );
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            return TyId::ERROR;
        }
        TyId::FLOAT
    }

    /// Reports an operator rejecting its operands: the message points at the
    /// operator, and each operand is labelled with its type.
    fn operand_error(
        &mut self,
        id: ExprId,
        message: String,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) {
        let expr = self.ast.get(id);
        let span = expr.operator_span().unwrap_or(expr.span);
        let (lhs_label, rhs_label) = (self.in_label(lhs_ty), self.in_label(rhs_ty));
        self.errors.push(
            Diagnostic::error(message, span)
                .with_context(self.ast.get(lhs).span, lhs_label)
                .with_context(self.ast.get(rhs).span, rhs_label),
        );
    }

    /// Types `//` and `%`: both operands must be integers, and the result is
    /// a concrete `Int`. Untyped operands are coerced to `Int`; they never
    /// adapt to `Float`, since these operators do not accept it.
    fn integer_only(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        // Whatever the outcome, untyped operands settle to Int: these
        // operators never accept Float, so nothing else could decide them.
        self.coerce(lhs, TyId::INT);
        self.coerce(rhs, TyId::INT);
        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        let rejected = [lhs_ty, rhs_ty]
            .into_iter()
            .find(|&ty| !matches!(ty, TyId::INT | TyId::UNTYPED_INT));
        if let Some(rejected) = rejected {
            let message = format!("cannot {} {}", op.verb(), self.in_message(rejected));
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            return TyId::ERROR;
        }
        TyId::INT
    }

    /// Gives an untyped expression its default type, `Int`, where it is
    /// bound or discarded rather than used in a typed context. Returns the
    /// resulting type.
    fn settle(&mut self, id: ExprId, ty: TyId) -> TyId {
        if ty == TyId::UNTYPED_INT {
            self.coerce(id, TyId::INT);
            TyId::INT
        } else {
            ty
        }
    }

    /// Pushes the concrete type `ty` down into the untyped subtree rooted at
    /// `id`. Does nothing if `id` already has a concrete type (or is an
    /// error).
    ///
    /// Only integer literals, parentheses, negation, `+ - *`, blocks and
    /// `if`s with `else` can be untyped, and an untyped node's children
    /// (an `if`'s branches, not its condition) are untyped or concrete of
    /// the same kind, so the walk only ever meets those nodes. Every other
    /// node always has a concrete type; a record block, in particular, is
    /// always a record.
    fn coerce(&mut self, id: ExprId, ty: TyId) {
        if self.expr_types[id.index()] != TyId::UNTYPED_INT {
            return;
        }
        self.expr_types[id.index()] = ty;
        match self.ast.get(id).kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Interp(_)
            | ExprKind::Name(_)
            | ExprKind::Field { .. }
            | ExprKind::Ascribe { .. } => {}
            ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
                self.coerce(inner, ty);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.coerce(lhs, ty);
                self.coerce(rhs, ty);
            }
            ExprKind::If { then, else_, .. } => {
                self.coerce(then, ty);
                self.coerce(else_, ty);
            }
            ExprKind::Block(body) => self.coerce_result(body, ty),
        }
    }

    /// Coerces the result expression of `body`, if it has one, to `ty`. (In
    /// a record block the final expression statement is not a result, but
    /// it has been settled already, so coercing it does nothing.)
    fn coerce_result(&mut self, body: ItemRange, ty: TyId) {
        let Some(&last) = self.ast.items(body).last() else {
            return;
        };
        match self.ast.item(last).kind {
            ItemKind::Result(value) | ItemKind::Expr(value) => self.coerce(value, ty),
            ItemKind::Let { .. } | ItemKind::Rebind { .. } | ItemKind::Field { .. } => {}
        }
    }

    /// Type `ty` as a diagnostic's message or notes show it: plainly if it
    /// is written as a name, and in backticks if it is structural, as a
    /// record type is.
    fn in_message(&self, ty: TyId) -> String {
        let shown = self.types.display(ty, self.ast.names());
        match self.types.kind(ty) {
            TyKind::Record(_) | TyKind::Union(_) => format!("`{shown}`"),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::UntypedInt
            | TyKind::Error => shown.to_string(),
        }
    }

    /// Type `ty` as a diagnostic's labels show it: always plainly.
    fn in_label(&self, ty: TyId) -> String {
        self.types.display(ty, self.ast.names()).to_string()
    }
}

/// Whether `ty` is a numeric type, untyped integers included.
fn is_numeric(ty: TyId) -> bool {
    matches!(ty, TyId::INT | TyId::FLOAT | TyId::UNTYPED_INT)
}

/// A field's position in its block or type, as stored in
/// `Checker::sorted_fields`. A block or type has fewer fields than its
/// source has bytes, so the conversion never saturates.
fn position(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;
    use crate::lexer::{tokenize, tokenize_with};
    use crate::parser::parse;
    use crate::source::SourceFile;

    fn parse_text(text: &str) -> Ast {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        parse(&tokens, names, strings, &source).unwrap()
    }

    /// The expression of the file's final result or expression item.
    fn result_expr(ast: &Ast) -> ExprId {
        let last = *ast.items(ast.root()).last().unwrap();
        match ast.item(last).kind {
            ItemKind::Result(value) | ItemKind::Expr(value) => value,
            other => panic!("the last item has no result: {other:?}"),
        }
    }

    /// Checks `text` as a file with a fresh type table.
    fn check_text(text: &str) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let output = Checker::new(&ast, &mut types).check();
        (ast, types, output)
    }

    /// The type of the file's result.
    fn result_type(text: &str) -> TyId {
        let ast = parse_text(text);
        check(&ast, &mut TypeTable::new())
            .unwrap()
            .0
            .types
            .get(result_expr(&ast))
    }

    /// The type of the file's result, as shown to users.
    fn shown_result(text: &str) -> String {
        let (ast, types, output) = check_text(text);
        let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        let ty = output.checked.types.get(result_expr(&ast));
        types.display(ty, ast.names()).to_string()
    }

    /// The type of the file's value, as shown to users.
    fn file_type(text: &str) -> String {
        let (ast, types, output) = check_text(text);
        let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        types.display(output.ty, ast.names()).to_string()
    }

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        check(&parse_text(text), &mut TypeTable::new()).unwrap_err()
    }

    /// The warnings of a check that must succeed.
    fn warnings(text: &str) -> Vec<Diagnostic> {
        check(&parse_text(text), &mut TypeTable::new())
            .unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
            .1
    }

    fn errors(text: &str) -> Vec<(String, Span)> {
        diagnostics(text)
            .into_iter()
            .map(|d| (d.message, d.span))
            .collect()
    }

    fn messages(text: &str) -> Vec<String> {
        errors(text).into_iter().map(|(m, _)| m).collect()
    }

    /// Every node's type in allocation order (children before parents).
    fn all_types(text: &str) -> Vec<TyId> {
        check(&parse_text(text), &mut TypeTable::new())
            .unwrap()
            .0
            .types
            .types
    }

    /// The labels of a diagnostic as `(span, text)` pairs.
    fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
        diagnostic
            .labels
            .iter()
            .map(|label| (label.span, label.text.as_str()))
            .collect()
    }

    // A concrete Int operand is written `(7 // 2)`: `//` always produces a
    // concrete Int, so the typing rows with a concrete Int are reachable
    // without variables.

    #[test]
    fn literals() {
        assert_eq!(result_type("1"), TyId::INT);
        assert_eq!(result_type("1.5"), TyId::FLOAT);
    }

    #[test]
    fn untyped_op_untyped_stays_untyped_until_the_root() {
        for op in ["+", "-", "*"] {
            let text = format!("1 {op} 2");
            assert_eq!(all_types(&text), vec![TyId::INT; 3], "{text}");
        }
    }

    #[test]
    fn untyped_op_int_is_int() {
        for op in ["+", "-", "*"] {
            assert_eq!(result_type(&format!("1 {op} (7 // 2)")), TyId::INT);
            assert_eq!(result_type(&format!("(7 // 2) {op} 1")), TyId::INT);
        }
    }

    #[test]
    fn untyped_op_float_is_float() {
        for op in ["+", "-", "*"] {
            let text = format!("1 {op} 2.0");
            assert_eq!(all_types(&text), vec![TyId::FLOAT; 3], "{text}");
            let text = format!("2.0 {op} 1");
            assert_eq!(all_types(&text), vec![TyId::FLOAT; 3], "{text}");
        }
    }

    #[test]
    fn same_concrete_types() {
        assert_eq!(result_type("(7 // 2) + (7 // 2)"), TyId::INT);
        assert_eq!(result_type("1.0 * 2.0"), TyId::FLOAT);
    }

    #[test]
    fn int_and_float_do_not_mix() {
        assert_eq!(
            errors("(7 // 2) + 1.5"),
            vec![("cannot add Int and Float".to_owned(), Span::new(9, 10))]
        );
        assert_eq!(
            messages("1.5 - (7 // 2)"),
            vec!["cannot subtract Float and Int"]
        );
        assert_eq!(
            messages("(7 % 2) * 1.5"),
            vec!["cannot multiply Int and Float"]
        );
    }

    #[test]
    fn negation_preserves_type() {
        assert_eq!(all_types("-1"), vec![TyId::INT; 2]);
        assert_eq!(all_types("-1.5"), vec![TyId::FLOAT; 2]);
        assert_eq!(result_type("-(7 // 2)"), TyId::INT);
        // An untyped negation adapts like a literal.
        assert_eq!(all_types("-1 + 2.0"), vec![TyId::FLOAT; 4]);
    }

    #[test]
    fn division_accepts_any_mix_and_yields_float() {
        assert_eq!(all_types("7 / 2"), vec![TyId::FLOAT; 3]);
        assert_eq!(all_types("7 / 2.0"), vec![TyId::FLOAT; 3]);
        assert_eq!(all_types("7.0 / 2"), vec![TyId::FLOAT; 3]);
        assert_eq!(result_type("(7 // 2) / 2.0"), TyId::FLOAT);
        assert_eq!(result_type("2.0 / (7 // 2)"), TyId::FLOAT);
        assert_eq!(result_type("(7 // 2) / (7 // 2)"), TyId::FLOAT);
    }

    #[test]
    fn division_leaves_concrete_int_operands_int() {
        let ast = parse_text("(7 // 2) / 2");
        let types = check(&ast, &mut TypeTable::new()).unwrap().0.types;
        let ExprKind::Binary { lhs, rhs, .. } = ast.get(result_expr(&ast)).kind else {
            panic!("expected binary");
        };
        assert_eq!(types.get(lhs), TyId::INT);
        assert_eq!(types.get(rhs), TyId::FLOAT);
    }

    #[test]
    fn floor_division_and_modulo_need_integers() {
        assert_eq!(all_types("7 // 2"), vec![TyId::INT; 3]);
        assert_eq!(all_types("7 % 2"), vec![TyId::INT; 3]);
        assert_eq!(result_type("(7 // 2) // 2"), TyId::INT);
        assert_eq!(
            errors("7.0 // 2"),
            vec![("cannot floor-divide Float".to_owned(), Span::new(4, 6))]
        );
        assert_eq!(messages("7 // 2.0"), vec!["cannot floor-divide Float"]);
        assert_eq!(messages("7.0 % 2"), vec!["cannot take modulo of Float"]);
        assert_eq!(messages("7 % 2.0"), vec!["cannot take modulo of Float"]);
        assert_eq!(messages("7.0 % 2.0"), vec!["cannot take modulo of Float"]);
    }

    #[test]
    fn floor_division_result_does_not_adapt_to_float() {
        assert_eq!(messages("(7 // 2) + 1.0"), vec!["cannot add Int and Float"]);
    }

    #[test]
    fn untyped_subtree_is_coerced_all_the_way_down() {
        // Allocation order: 1, 2, +, paren, 3.0, *.
        assert_eq!(all_types("(1 + 2) * 3.0"), vec![TyId::FLOAT; 6]);
        assert_eq!(all_types("3.0 * -(1 - -2)"), vec![TyId::FLOAT; 8]);
    }

    #[test]
    fn division_makes_untyped_siblings_float() {
        // `2 / 3` is Float, so the untyped `1` beside it adapts to Float.
        assert_eq!(all_types("1 + 2 / 3"), vec![TyId::FLOAT; 5]);
    }

    #[test]
    fn untyped_root_defaults_to_int_throughout() {
        assert_eq!(all_types("-(1 + 2) * 3"), vec![TyId::INT; 7]);
        assert_eq!(all_types("= 1 + 2"), vec![TyId::INT; 3]);
        assert_eq!(file_type("1 + 2"), "Int");
    }

    #[test]
    fn multiple_errors_in_one_run() {
        assert_eq!(
            errors("(7.0 // 2) + (1 % 2.5)"),
            vec![
                ("cannot floor-divide Float".to_owned(), Span::new(5, 7)),
                ("cannot take modulo of Float".to_owned(), Span::new(16, 17)),
            ]
        );
    }

    #[test]
    fn operands_are_labelled_with_their_types() {
        let concrete = diagnostics("(7 // 2) + 1.5");
        assert_eq!(
            labels(&concrete[0]),
            vec![(Span::new(0, 8), "Int"), (Span::new(11, 14), "Float")]
        );
        // An untyped literal is labelled as the Int it would default to.
        let untyped = diagnostics("1 % 2.5");
        let texts: Vec<&str> = labels(&untyped[0]).into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, vec!["Int", "Float"]);
    }

    #[test]
    fn errors_do_not_cascade() {
        // The failed `//` becomes an error type that every enclosing operator
        // accepts, whatever the other operand is.
        for text in [
            "(1.0 // 2) + (7 // 2)",
            "(1.0 // 2) + 1.5",
            "(1.0 // 2) + 1",
            "1.5 * (1.0 // 2)",
            "-(1.0 // 2) // 2",
            "(1.0 // 2) / 2.5",
            "2 % (1.0 // 2)",
            "(1.0 // 2) : Float",
            "{ 1.0 // 2 } + 1.5",
            "{ x = 1.0 // 2 }.x + 1",
            "(1.0 // 2).x",
        ] {
            assert_eq!(messages(text), vec!["cannot floor-divide Float"], "{text}");
        }
    }

    #[test]
    fn the_empty_record_is_rejected_by_every_operator() {
        assert_eq!(
            errors("{} + 1"),
            vec![("cannot add `{}` and Int".to_owned(), Span::new(3, 4))]
        );
        assert_eq!(messages("1.5 * {}"), vec!["cannot multiply Float and `{}`"]);
        assert_eq!(messages("{} / 2"), vec!["cannot divide `{}` and Int"]);
        assert_eq!(messages("{} // 2"), vec!["cannot floor-divide `{}`"]);
        assert_eq!(messages("7 % {}"), vec!["cannot take modulo of `{}`"]);
        assert_eq!(messages("1.5 // {}"), vec!["cannot floor-divide Float"]);
        let negation = &diagnostics("-{}")[0];
        assert_eq!(negation.message, "cannot negate `{}`");
        assert_eq!(negation.span, Span::new(0, 1));
        assert_eq!(labels(negation), vec![(Span::new(1, 3), "{}")]);
        // The same goes for records with fields.
        assert_eq!(messages("-{ x = 1 }"), vec!["cannot negate `{x: Int}`"]);
        assert_eq!(
            labels(&diagnostics("{ x = 1 } + 1")[0]),
            vec![(Span::new(0, 9), "{x: Int}"), (Span::new(12, 13), "Int")]
        );
    }

    #[test]
    fn error_type_never_escapes_a_successful_check() {
        // Every node of a successful check has a concrete type.
        for text in [
            "1",
            "1.5",
            "(1 + 2) * 3.0",
            "7 / 2",
            "(7 // 2) / 2",
            "let x = 5\nx + 1",
            "let y = { 1 + 2 }\n{ 3 } : Float",
            "= { let a = 1\n= a } * 2",
            "{}",
            "x = 1\ny = { z = 2 }",
            "let p: {x: Float} = { x = 1 }\np.x",
            "if true { { a = 1 } } else { { a = 2 } }",
        ] {
            for ty in all_types(text) {
                assert!(
                    ty != TyId::ERROR && ty != TyId::UNTYPED_INT,
                    "{text}: {ty:?}"
                );
            }
        }
    }

    #[test]
    fn annotation_renders_types() {
        let ast = parse_text("1 + 2.0");
        let table = &mut TypeTable::new();
        let types = check(&ast, table).unwrap().0.types;
        assert_eq!(
            ast.pretty_with(&types.annotation(table, ast.names()))
                .to_string(),
            "(file\n  (expr @0..7\n    (+ : Float @0..7\n      (int 1 : Float @0..1)\n      (float 2.0 : Float @4..7))))"
        );
        let ast = parse_text("let x = { 1 }\nx : Float");
        assert!(check(&ast, table).is_err());
        let ast = parse_text("let x = { 1 }\n{}");
        let types = check(&ast, table).unwrap().0.types;
        assert_eq!(
            ast.pretty_with(&types.annotation(table, ast.names()))
                .to_string(),
            "(file\n\
             \x20 (let x @0..13\n\
             \x20   (block : Int @8..13\n\
             \x20     (expr @10..11\n\
             \x20       (int 1 : Int @10..11))))\n\
             \x20 (expr @14..16\n\
             \x20   (block : {} @14..16)))"
        );
        let ast = parse_text("{ y = \"s\"; x = 1 }.x");
        let types = check(&ast, table).unwrap().0.types;
        assert_eq!(
            ast.pretty_with(&types.annotation(table, ast.names()))
                .to_string(),
            "(file\n\
             \x20 (expr @0..20\n\
             \x20   (. x : Int @0..20\n\
             \x20     (block : {x: Int, y: String} @0..18\n\
             \x20       (field y @2..9\n\
             \x20         (str \"s\" : String @6..9))\n\
             \x20       (field x @11..16\n\
             \x20         (int 1 : Int @15..16))))))"
        );
    }

    #[test]
    fn variables_have_the_type_they_were_bound_with() {
        assert_eq!(result_type("let x = 5\nx"), TyId::INT);
        assert_eq!(result_type("let x = 5.5\nx"), TyId::FLOAT);
        assert_eq!(result_type("let x = {}\nx"), TyId::EMPTY);
        // The untyped literal defaults where it is bound.
        assert_eq!(
            errors("let x = 5\nx + 2.5"),
            vec![("cannot add Int and Float".to_owned(), Span::new(12, 13))]
        );
        assert_eq!(result_type("let x = 5 : Float\nx + 2.5"), TyId::FLOAT);
        assert_eq!(result_type("let x: Float = 5\nx + 2.5"), TyId::FLOAT);
    }

    #[test]
    fn a_let_is_visible_from_the_next_item() {
        assert_eq!(
            errors("let x = x"),
            vec![("undefined variable `x`".to_owned(), Span::new(8, 9))]
        );
        // The initialiser sees the binding being shadowed.
        assert_eq!(result_type("let x = 1.5\nlet x = x + 1\nx"), TyId::FLOAT);
    }

    #[test]
    fn shadowing_in_the_same_block() {
        assert_eq!(result_type("let x = 1\nlet x = 2.5\nx"), TyId::FLOAT);
        let ast = parse_text("let x = 1\nlet x = 2.5\nx");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        assert_eq!(resolution.expr_slot(result_expr(&ast)), Slot(1));
    }

    #[test]
    fn shadowing_across_blocks() {
        assert_eq!(result_type("let x = 1\n{ let x = 2.5; x }"), TyId::FLOAT);
        // The outer variable is visible in an inner block until shadowed.
        assert_eq!(result_type("let x = 1\n{ let y = x; y }"), TyId::INT);
        // And is back in scope after the block.
        assert_eq!(result_type("let x = 1\n{ let x = 2.5 }\nx"), TyId::INT);
    }

    #[test]
    fn variables_are_not_visible_after_their_block() {
        assert_eq!(
            errors("{ let y = 1 }\ny"),
            vec![("undefined variable `y`".to_owned(), Span::new(14, 15))]
        );
        assert_eq!(
            messages("let z = { let inner = 1; inner }\ninner"),
            vec!["undefined variable `inner`"]
        );
    }

    #[test]
    fn undefined_variables() {
        let diagnostic = &diagnostics("x + 1")[0];
        assert_eq!(diagnostic.message, "undefined variable `x`");
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert!(diagnostic.notes.is_empty());
        let diagnostic = &diagnostics("let x = 2\nx-1")[0];
        assert_eq!(diagnostic.message, "undefined variable `x-1`");
        assert_eq!(diagnostic.span, Span::new(10, 13));
        assert_eq!(
            diagnostic.notes,
            vec!["names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction"]
        );
    }

    #[test]
    fn rebinding_in_the_same_block() {
        assert_eq!(result_type("let x = 1\nx := 2.5\nx"), TyId::FLOAT);
        assert_eq!(
            result_type("let x = 1\nx := x + 1\nx := x * 2\nx"),
            TyId::INT
        );
        // In an inner block, of a variable declared in that block, even one
        // shadowing an outer variable.
        assert_eq!(
            result_type("let x = 1\n{ let x = 2; x := 3.5; x }"),
            TyId::FLOAT
        );
        // The new value sees the old binding.
        let ast = parse_text("let x = 1\nx := x + 1\nx");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(0));
        // `:=` takes a fresh slot.
        assert_eq!(resolution.item_slot(items[1]), Slot(1));
        assert_eq!(resolution.expr_slot(result_expr(&ast)), Slot(1));
    }

    #[test]
    fn rebinding_an_undeclared_name() {
        let diagnostic = &diagnostics("x := 1")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `x`: no variable named `x` is declared"
        );
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert!(diagnostic.labels.is_empty());
        assert_eq!(diagnostic.notes, vec!["declare it with `let x = ...`"]);
        // A later declaration does not count.
        assert_eq!(
            messages("x := 1\nlet x = 2"),
            vec!["cannot rebind `x`: no variable named `x` is declared"]
        );
    }

    #[test]
    fn rebinding_a_name_from_an_enclosing_block() {
        let diagnostic = &diagnostics("let x = 1\n{\n  x := 2\n}")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `x` here: it is declared in an enclosing block"
        );
        assert_eq!(diagnostic.span, Span::new(14, 15));
        assert_eq!(labels(diagnostic), vec![(Span::new(0, 9), "declared here")]);
        assert_eq!(
            diagnostic.notes,
            vec!["shadow it with `let x = ...` instead"]
        );
        // The label points at the latest declaration.
        let diagnostic = &diagnostics("let x = 1\nx := 2\n{ x := 3 }")[0];
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(10, 16), "declared here")]
        );
    }

    #[test]
    fn a_rejected_rebind_still_declares_the_variable() {
        assert_eq!(
            messages("y := 1\ny + 1"),
            vec!["cannot rebind `y`: no variable named `y` is declared"]
        );
        assert_eq!(
            messages("let x = 1\n{ x := 2.5; x * 1.5 }"),
            vec!["cannot rebind `x` here: it is declared in an enclosing block"]
        );
    }

    #[test]
    fn annotations() {
        let ast = parse_text("let x: Float = 5");
        assert_eq!(
            check(&ast, &mut TypeTable::new()).unwrap().0.types.types,
            vec![TyId::FLOAT]
        );
        assert_eq!(result_type("let x: Int = 5\nx"), TyId::INT);
        assert_eq!(result_type("let x: Float = 5\nx"), TyId::FLOAT);
        assert_eq!(result_type("let x: Float = 2.5\nx"), TyId::FLOAT);
        assert_eq!(result_type("let x: {} = {}\nx"), TyId::EMPTY);
        assert_eq!(result_type("let x: Float = -(1 + 2)\nx"), TyId::FLOAT);
        assert_eq!(result_type("let x: Int = 7 // 2\nx"), TyId::INT);
    }

    #[test]
    fn annotation_mismatches() {
        let diagnostic = &diagnostics("let x: Int = 5.0")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(13, 16));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found Float")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 10), "expected because of this annotation")]
        );
        assert_eq!(
            diagnostics("let x: Float = 7 // 2")[0].label.as_deref(),
            Some("expected Float, found Int")
        );
        assert_eq!(
            diagnostics("let x: {} = 1")[0].label.as_deref(),
            Some("expected {}, found Int")
        );
        assert_eq!(
            diagnostics("let x: Int = {}")[0].label.as_deref(),
            Some("expected Int, found {}")
        );
        // The variable has the annotated type, so uses do not cascade.
        assert_eq!(
            messages("let x: Int = 5.0\nlet y = x + 1\n= x // y"),
            vec!["mismatched types"]
        );
    }

    #[test]
    fn unknown_types() {
        assert_eq!(
            errors("let x: Foo = 1"),
            vec![("unknown type `Foo`".to_owned(), Span::new(7, 10))]
        );
        assert_eq!(
            errors("1 : int"),
            vec![("unknown type `int`".to_owned(), Span::new(4, 7))]
        );
        // `Unit` is gone: the empty record is written `{}`.
        assert_eq!(
            errors("let x: Unit = {}"),
            vec![("unknown type `Unit`".to_owned(), Span::new(7, 11))]
        );
        // A variable is not a type.
        assert_eq!(
            messages("let x = 1\nlet y: x = 2"),
            vec!["unknown type `x`"]
        );
        // Neither the variable nor the ascription cascades.
        assert_eq!(
            messages("let x: Foo = 1.0 // 2\nx + 1.5\n(1 : Bar) + 2.5"),
            vec![
                "unknown type `Foo`",
                "cannot floor-divide Float",
                "unknown type `Bar`"
            ]
        );
        // Nor does an unknown type inside a record type.
        assert_eq!(
            messages("let p: {a: Foo} = { a = 1 }\np.a + 1.5"),
            vec!["unknown type `Foo`"]
        );
    }

    #[test]
    fn ascriptions() {
        assert_eq!(all_types("5 : Float"), vec![TyId::FLOAT; 2]);
        assert_eq!(result_type("-5 : Float"), TyId::FLOAT);
        assert_eq!(result_type("1 + 2 : Float"), TyId::FLOAT);
        assert_eq!(result_type("let x = 1\nx : Int"), TyId::INT);
        assert_eq!(result_type("{} : {}"), TyId::EMPTY);
        assert_eq!(result_type("1 : Int : Int"), TyId::INT);
        // An untyped block adapts like a literal.
        assert_eq!(all_types("{ 5 } : Float"), vec![TyId::FLOAT; 3]);
        let diagnostic = &diagnostics("5.0 : Int")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(0, 3));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found Float")
        );
        assert_eq!(labels(diagnostic), vec![(Span::new(6, 9), "ascribed here")]);
        assert_eq!(
            diagnostics("let x = 1\nx : Float")[0].label.as_deref(),
            Some("expected Float, found Int")
        );
        // Chained ascriptions are each checked in turn.
        assert_eq!(
            errors("1 : Int : Float"),
            vec![("mismatched types".to_owned(), Span::new(0, 7))]
        );
    }

    #[test]
    fn block_types() {
        assert_eq!(result_type("{}"), TyId::EMPTY);
        assert_eq!(result_type("{ 1.5 }"), TyId::FLOAT);
        assert_eq!(result_type("{ = 1.5 }"), TyId::FLOAT);
        assert_eq!(result_type("{ let a = 1 }"), TyId::EMPTY);
        assert_eq!(result_type("{ let a = 1; a }"), TyId::INT);
        assert_eq!(result_type("{ {} }"), TyId::EMPTY);
        assert_eq!(result_type("{ 1 } + 2.5"), TyId::FLOAT);
        assert_eq!(result_type("let x = { 1 }\nx"), TyId::INT);
        assert_eq!(file_type(""), "{}");
        assert_eq!(file_type("let x = 1"), "{}");
    }

    #[test]
    fn unused_values() {
        let diagnostic = &diagnostics("1 + 2\n3")[0];
        assert_eq!(diagnostic.message, "unused value of type Int");
        assert_eq!(diagnostic.span, Span::new(0, 5));
        assert_eq!(
            diagnostic.notes,
            vec!["make it the block's result with `= ...`, bind it with `let`, or remove it"]
        );
        assert_eq!(messages("1.5; 2"), vec!["unused value of type Float"]);
        assert_eq!(
            messages("let x = 1\nx\nx"),
            vec!["unused value of type Int"]
        );
        assert_eq!(messages("{ 1; 2 }"), vec!["unused value of type Int"]);
        // A record with fields is a value like any other.
        assert_eq!(
            messages("{ x = 1 }\n2"),
            vec!["unused value of type `{x: Int}`"]
        );
        // `{}` statements are fine anywhere.
        assert_eq!(result_type("{}\n{ let a = 1 }\n{ {} }\n2"), TyId::INT);
        // An erroneous statement is reported once, not also as unused.
        assert_eq!(messages("1.0 // 2\n3"), vec!["cannot floor-divide Float"]);
    }

    #[test]
    fn the_result_must_be_the_last_item() {
        let diagnostic = &diagnostics("= 1\nlet y = 2")[0];
        assert_eq!(
            diagnostic.message,
            "the result must be the last item in its block"
        );
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(4, 13), "this item comes after it")]
        );
        // A second result is caught by the same rule, at the first.
        assert_eq!(
            errors("{ = 1; = 2 }"),
            vec![(
                "the result must be the last item in its block".to_owned(),
                Span::new(2, 3)
            )]
        );
        // A final expression after a misplaced result is not also unused.
        assert_eq!(
            messages("= 1\n2"),
            vec!["the result must be the last item in its block"]
        );
    }

    /// Every `let` in `body` and the blocks nested in it, in the order their
    /// declarations complete.
    fn lets(ast: &Ast, body: ItemRange, out: &mut Vec<ItemId>) {
        fn visit(ast: &Ast, id: ExprId, out: &mut Vec<ItemId>) {
            match ast.get(id).kind {
                ExprKind::Block(body) => lets(ast, body, out),
                ExprKind::Binary { lhs, rhs, .. } => {
                    visit(ast, lhs, out);
                    visit(ast, rhs, out);
                }
                ExprKind::Paren(inner)
                | ExprKind::Unary { operand: inner, .. }
                | ExprKind::Field { expr: inner, .. }
                | ExprKind::Ascribe { expr: inner, .. } => visit(ast, inner, out),
                ExprKind::If {
                    cond, then, else_, ..
                } => {
                    visit(ast, cond, out);
                    visit(ast, then, out);
                    visit(ast, else_, out);
                }
                ExprKind::Interp(parts) => {
                    for &part in ast.parts(parts) {
                        if let StrPart::Expr(inner) = part {
                            visit(ast, inner, out);
                        }
                    }
                }
                ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Bool(_)
                | ExprKind::Str(_)
                | ExprKind::Name(_) => {}
            }
        }
        for &id in ast.items(body) {
            match ast.item(id).kind {
                ItemKind::Let { value, .. } => {
                    visit(ast, value, out);
                    out.push(id);
                }
                ItemKind::Rebind { value, .. }
                | ItemKind::Field { value, .. }
                | ItemKind::Result(value)
                | ItemKind::Expr(value) => {
                    visit(ast, value, out);
                }
            }
        }
    }

    #[test]
    fn slots_are_assigned_stack_fashion() {
        let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
        let ast = parse_text(text);
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let names = ast.names();
        let mut all_lets = Vec::new();
        lets(&ast, ast.root(), &mut all_lets);
        let slot_of = |text: &str| -> Vec<u32> {
            let wanted = names.lookup(text).unwrap();
            all_lets
                .iter()
                .filter(|&&id| matches!(ast.item(id).kind, ItemKind::Let { name, .. } if name == wanted))
                .map(|&id| resolution.item_slot(id).get())
                .collect()
        };
        // Each initialiser's temporary takes the slot its variable then
        // reuses, since the variable is declared after its initialiser's
        // block has ended.
        assert_eq!(slot_of("t"), vec![0]);
        assert_eq!(slot_of("a"), vec![0]);
        assert_eq!(slot_of("u"), vec![1]);
        assert_eq!(slot_of("b"), vec![1]);
        assert_eq!(slot_of("v"), vec![2]);
        assert_eq!(slot_of("w"), vec![3]);
        assert_eq!(slot_of("c"), vec![2]);
        assert_eq!(resolution.frame_size(), 4);
        let slot_names: Vec<&str> = resolution
            .slot_names()
            .iter()
            .map(|&name| names.get(name))
            .collect();
        assert_eq!(slot_names, vec!["a", "b", "c", "w"]);
        // Uses read the slot of the binding in scope.
        let ExprKind::Binary { lhs, rhs, .. } = ast.get(result_expr(&ast)).kind else {
            panic!("expected a sum");
        };
        assert_eq!(resolution.expr_slot(rhs), Slot(2));
        let ExprKind::Binary { lhs: a, rhs: b, .. } = ast.get(lhs).kind else {
            panic!("expected a sum");
        };
        assert_eq!(resolution.expr_slot(a), Slot(0));
        assert_eq!(resolution.expr_slot(b), Slot(1));
        // Nodes that are not variables have no slot.
        assert_eq!(resolution.expr_slot(lhs), Slot::NONE);
    }

    #[test]
    fn slots_after_a_block_are_reused() {
        let ast = parse_text("let a = 1\n{ let b = 2; let c = 3 }\nlet d = 4\nd");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(0));
        assert_eq!(resolution.item_slot(items[1]), Slot::NONE);
        // `b` was in slot 1; `d` reuses it.
        assert_eq!(resolution.item_slot(items[2]), Slot(1));
        assert_eq!(resolution.frame_size(), 3);
        assert_eq!(resolution.expr_slot(result_expr(&ast)), Slot(1));
        assert_eq!(Slot(3).to_string(), "#3");
    }

    #[test]
    fn fields_take_slots_like_variables() {
        let ast = parse_text("let a = 1\nlet r = { x = 2; let t = x; y = t }\nb = a");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(0));
        // The block's field, variable and field take slots 1 to 3; `r`,
        // declared after its initialiser, reuses slot 1, and the file's
        // field `b` takes slot 2.
        assert_eq!(resolution.item_slot(items[1]), Slot(1));
        assert_eq!(resolution.item_slot(items[2]), Slot(2));
        assert_eq!(resolution.frame_size(), 4);
        let names: Vec<&str> = resolution
            .slot_names()
            .iter()
            .map(|&name| ast.names().get(name))
            .collect();
        assert_eq!(names, vec!["a", "r", "b", "y"]);
    }

    #[test]
    fn a_program_without_variables_needs_no_frame() {
        let ast = parse_text("1 + { 2 }");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        assert_eq!(resolution.frame_size(), 0);
        assert!(resolution.slot_names().is_empty());
    }

    #[test]
    fn recovery_does_not_cascade() {
        // A `let` whose initialiser fails is still declared, as Error.
        assert_eq!(
            messages("let x = 1.0 // 2\nlet y = x + 1\n= x * 2.5 + y"),
            vec!["cannot floor-divide Float"]
        );
        // An undefined variable is reported at each use, and nothing more.
        assert_eq!(
            messages("let y = z + 1.5\nz * 2\ny // 2"),
            vec!["undefined variable `z`", "undefined variable `z`"]
        );
        // Independent errors are all reported.
        assert_eq!(
            messages("let x = 1.0 // 2\nlet y: Int = 2.5\nq"),
            vec![
                "cannot floor-divide Float",
                "mismatched types",
                "undefined variable `q`"
            ]
        );
    }

    /// Declares a variable of each type: `i`, `f`, `b`, `s` and `u`.
    const PRELUDE: &str = "let i = 1\nlet f = 1.5\nlet b = true\nlet s = \"s\"\nlet u = {}\n";

    /// Each variable of [`PRELUDE`], its type, and the type as messages
    /// show it.
    const VARIABLES: [(&str, TyId, &str); 5] = [
        ("i", TyId::INT, "Int"),
        ("f", TyId::FLOAT, "Float"),
        ("b", TyId::BOOL, "Bool"),
        ("s", TyId::STRING, "String"),
        ("u", TyId::EMPTY, "`{}`"),
    ];

    #[test]
    fn literals_of_the_new_types() {
        assert_eq!(result_type("true"), TyId::BOOL);
        assert_eq!(result_type("false"), TyId::BOOL);
        assert_eq!(result_type("\"a\""), TyId::STRING);
        assert_eq!(result_type("r\"a\""), TyId::STRING);
        assert_eq!(result_type("\"{1}\""), TyId::STRING);
    }

    #[test]
    fn equality_accepts_two_operands_of_any_one_type() {
        for (lhs, lhs_ty, lhs_shown) in VARIABLES {
            for (rhs, rhs_ty, rhs_shown) in VARIABLES {
                for op in ["==", "!="] {
                    let text = format!("{PRELUDE}{lhs} {op} {rhs}");
                    if lhs_ty == rhs_ty {
                        assert_eq!(result_type(&text), TyId::BOOL, "{text}");
                    } else {
                        assert_eq!(
                            messages(&text),
                            vec![format!("cannot compare {lhs_shown} and {rhs_shown}")],
                            "{text}"
                        );
                    }
                }
            }
        }
        assert_eq!(result_type("{} == {}"), TyId::BOOL);
    }

    #[test]
    fn ordering_accepts_two_ints_floats_or_strings() {
        for (lhs, lhs_ty, lhs_shown) in VARIABLES {
            for (rhs, rhs_ty, rhs_shown) in VARIABLES {
                for op in ["<", "<=", ">", ">="] {
                    let text = format!("{PRELUDE}{lhs} {op} {rhs}");
                    let unordered = [(lhs_ty, lhs_shown), (rhs_ty, rhs_shown)]
                        .into_iter()
                        .find(|&(ty, _)| ty == TyId::BOOL || ty == TyId::EMPTY);
                    if let Some((_, unordered)) = unordered {
                        assert_eq!(
                            messages(&text),
                            vec![format!("cannot order {unordered}")],
                            "{text}"
                        );
                    } else if lhs_ty == rhs_ty {
                        assert_eq!(result_type(&text), TyId::BOOL, "{text}");
                    } else {
                        assert_eq!(
                            messages(&text),
                            vec![format!("cannot compare {lhs_shown} and {rhs_shown}")],
                            "{text}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn comparison_errors_point_at_the_operator_and_label_the_operands() {
        let diagnostic = &diagnostics("(7 // 2) == 1.5")[0];
        assert_eq!(diagnostic.message, "cannot compare Int and Float");
        assert_eq!(diagnostic.span, Span::new(9, 11));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 8), "Int"), (Span::new(12, 15), "Float")]
        );
        let diagnostic = &diagnostics("true < false")[0];
        assert_eq!(diagnostic.message, "cannot order Bool");
        assert_eq!(diagnostic.span, Span::new(5, 6));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 4), "Bool"), (Span::new(7, 12), "Bool")]
        );
        // An unordered type is reported before a mismatch.
        assert_eq!(messages("1 >= {}"), vec!["cannot order `{}`"]);
        assert_eq!(messages("true <= 1"), vec!["cannot order Bool"]);
        // An untyped operand is labelled as the Int it defaults to.
        let diagnostic = &diagnostics("\"a\" != 1")[0];
        assert_eq!(diagnostic.message, "cannot compare String and Int");
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 3), "String"), (Span::new(7, 8), "Int")]
        );
    }

    #[test]
    fn untyped_literals_adapt_in_comparisons() {
        // Allocation order: operands, then the comparison.
        assert_eq!(all_types("1 == 2"), vec![TyId::INT, TyId::INT, TyId::BOOL]);
        assert_eq!(
            all_types("1 < 2.5"),
            vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
        );
        assert_eq!(
            all_types("2.5 >= 1"),
            vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
        );
        assert_eq!(
            all_types("1 + 2 != 3"),
            vec![TyId::INT, TyId::INT, TyId::INT, TyId::INT, TyId::BOOL]
        );
        assert_eq!(
            all_types("{ 1 } == 1.5"),
            vec![TyId::FLOAT, TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
        );
        assert_eq!(result_type("1 <= (7 // 2)"), TyId::BOOL);
        assert_eq!(
            all_types("let x = 1 == 1\nx"),
            vec![TyId::INT, TyId::INT, TyId::BOOL, TyId::BOOL]
        );
    }

    #[test]
    fn comparison_errors_do_not_cascade() {
        assert_eq!(
            messages("(1.0 // 2) == 1"),
            vec!["cannot floor-divide Float"]
        );
        assert_eq!(
            messages("true < (1.0 // 2)"),
            vec!["cannot floor-divide Float"]
        );
        assert_eq!(
            messages("(1 == 1.5 // 2) and true"),
            vec!["cannot floor-divide Float"]
        );
    }

    #[test]
    fn logic_operators_need_bools() {
        assert_eq!(result_type("true and false"), TyId::BOOL);
        assert_eq!(result_type("true or false"), TyId::BOOL);
        assert_eq!(result_type("not true"), TyId::BOOL);
        assert_eq!(result_type("not 1 == 2 or 1 < 2 and true"), TyId::BOOL);
        let diagnostic = &diagnostics("1 and true")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Bool, found Int")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 5), "`and` requires Bool operands")]
        );
        let diagnostic = &diagnostics("true or 2.5")[0];
        assert_eq!(diagnostic.span, Span::new(8, 11));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Bool, found Float")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(5, 7), "`or` requires Bool operands")]
        );
        let diagnostic = &diagnostics("not \"a\"")[0];
        assert_eq!(diagnostic.span, Span::new(4, 7));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Bool, found String")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 3), "`not` requires Bool operands")]
        );
        // Each wrong operand is reported.
        let both = diagnostics("1 or {}");
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].label.as_deref(), Some("expected Bool, found Int"));
        assert_eq!(both[1].label.as_deref(), Some("expected Bool, found {}"));
        // The operator still has type Bool, so nothing cascades.
        assert_eq!(messages("(1 and true) == false"), vec!["mismatched types"]);
        assert_eq!(messages("not not 1"), vec!["mismatched types"]);
        assert_eq!(
            messages("(1.0 // 2) or not (1.0 // 2)"),
            vec!["cannot floor-divide Float", "cannot floor-divide Float"]
        );
    }

    #[test]
    fn if_conditions_are_bool() {
        let diagnostic = &diagnostics("if 1 { 2 } else { 3 }")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(3, 4));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Bool, found Int")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 2), "`if` conditions are Bool")]
        );
        // The `if` still has its branches' type, so nothing cascades.
        assert_eq!(
            messages("let x = if \"a\" { 2 } else { 3 }\nx + 1"),
            vec!["mismatched types"]
        );
        assert_eq!(result_type("if 1 < 2 { 1.5 } else { 2.5 }"), TyId::FLOAT);
    }

    #[test]
    fn if_branches_must_agree() {
        assert_eq!(
            result_type("if true { \"a\" } else { \"b\" }"),
            TyId::STRING
        );
        assert_eq!(result_type("if true { true } else { false }"), TyId::BOOL);
        assert_eq!(result_type("if true {} else {}"), TyId::EMPTY);
        assert_eq!(result_type("if true { 1.5 } else { 2.5 }"), TyId::FLOAT);
        let diagnostic = &diagnostics("if true { 1 } else { \"a\" }")[0];
        assert_eq!(diagnostic.message, "`if` branches have different types");
        assert_eq!(diagnostic.span, Span::new(0, 2));
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(8, 13), "this is Int"),
                (Span::new(19, 26), "this is String")
            ]
        );
        assert_eq!(
            messages("if true { 1.5 } else { 7 // 2 }"),
            vec!["`if` branches have different types"]
        );
        assert_eq!(
            messages("if true {} else { 1 }"),
            vec!["`if` branches have different types"]
        );
        // In an `else if`, the label is on the nested `if`.
        let diagnostic = &diagnostics("if true { 1.5 } else if false { true } else { false }")[0];
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(8, 15), "this is Float"),
                (Span::new(21, 53), "this is Bool")
            ]
        );
        // A mismatch does not cascade.
        assert_eq!(
            messages("let x = if true { 1 } else { \"a\" }\nx + 1"),
            vec!["`if` branches have different types"]
        );
        assert_eq!(
            messages("if true { 1.0 // 2 } else { \"a\" }"),
            vec!["cannot floor-divide Float"]
        );
    }

    #[test]
    fn untyped_if_branches_adapt_and_default() {
        use TyId as T;
        // Allocation order: condition, then's literal and block, else's
        // literal and block, the `if`.
        assert_eq!(
            all_types("if true { 1 } else { 2.0 }"),
            vec![T::BOOL, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT]
        );
        assert_eq!(
            all_types("if true { 1.0 } else { 2 }"),
            vec![T::BOOL, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT]
        );
        // Two untyped branches stay untyped until the context decides...
        assert_eq!(
            all_types("if true { 1 } else { 2 } : Float"),
            vec![
                T::BOOL,
                T::FLOAT,
                T::FLOAT,
                T::FLOAT,
                T::FLOAT,
                T::FLOAT,
                T::FLOAT
            ]
        );
        assert_eq!(result_type("(if true { 1 } else { 2 }) * 0.5"), TyId::FLOAT);
        assert_eq!(
            result_type("let x: Float = if true { 1 } else if false { 2 } else { 3 }\nx"),
            TyId::FLOAT
        );
        // ...or default to Int.
        assert_eq!(
            all_types("if true { 1 } else { 2 }"),
            vec![T::BOOL, T::INT, T::INT, T::INT, T::INT, T::INT]
        );
        assert_eq!(
            result_type("let x = if true { 1 } else { 2 }\nx"),
            TyId::INT
        );
        assert_eq!(
            messages("let x = if true { 1 } else { 2 }\nx + 0.5"),
            vec!["cannot add Int and Float"]
        );
        // A concrete Int branch decides for an untyped one.
        assert_eq!(
            messages("(if true { 1 } else { 7 // 2 }) + 0.5"),
            vec!["cannot add Int and Float"]
        );
    }

    #[test]
    fn an_if_without_else_is_the_empty_record() {
        assert_eq!(result_type("if true {}"), TyId::EMPTY);
        assert_eq!(result_type("if true { let x = 1 }"), TyId::EMPTY);
        assert_eq!(
            result_type("let x = 1\nif x > 0 { let y = x }"),
            TyId::EMPTY
        );
        let diagnostic = &diagnostics("if true { 1 }")[0];
        assert_eq!(diagnostic.message, "`if` without `else` must be `{}`");
        assert_eq!(diagnostic.span, Span::new(0, 2));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(8, 13), "this branch is Int")]
        );
        assert_eq!(
            diagnostic.notes,
            vec!["add an `else` branch, or make the block's result `{}`"]
        );
        assert_eq!(
            labels(&diagnostics("if true { \"a\" }")[0]),
            vec![(Span::new(8, 15), "this branch is String")]
        );
        // A record with fields is not `{}`.
        assert_eq!(
            labels(&diagnostics("if true { { x = 1 } }")[0]),
            vec![(Span::new(8, 21), "this branch is {x: Int}")]
        );
        // Not reported again as unused or by an enclosing operator.
        assert_eq!(
            messages("if true { 1 }\n2"),
            vec!["`if` without `else` must be `{}`"]
        );
        assert_eq!(
            messages("(if true { 1.5 }) + 1"),
            vec!["`if` without `else` must be `{}`"]
        );
    }

    #[test]
    fn interpolating_every_type() {
        assert_eq!(
            result_type("\"{1} {1.5} {true} {\"s\"} { {} } {1 == 1} { { x = 1 } }\""),
            TyId::STRING
        );
        assert_eq!(
            result_type(&format!("{PRELUDE}\"{{i}}{{f}}{{b}}{{s}}{{u}}\"")),
            TyId::STRING
        );
        // An untyped expression settles to Int.
        assert_eq!(all_types("\"{1}\""), vec![TyId::INT, TyId::STRING]);
        assert_eq!(
            all_types("\"{1 + 2}\""),
            vec![TyId::INT, TyId::INT, TyId::INT, TyId::STRING]
        );
        assert_eq!(result_type("\"{ \"{1}\" }\""), TyId::STRING);
        // An erroneous part is reported once.
        assert_eq!(
            messages("\"{1.0 // 2}\" + \"a\""),
            vec!["cannot floor-divide Float"]
        );
        assert_eq!(messages("\"{x}\""), vec!["undefined variable `x`"]);
    }

    #[test]
    fn strings_concatenate_with_strings_only() {
        assert_eq!(result_type("\"a\" + \"b\""), TyId::STRING);
        assert_eq!(result_type("\"a\" + \"{1}\" + r\"c\""), TyId::STRING);
        let diagnostic = &diagnostics("\"a\" + 1")[0];
        assert_eq!(diagnostic.message, "cannot add String and Int");
        assert_eq!(diagnostic.span, Span::new(4, 5));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 3), "String"), (Span::new(6, 7), "Int")]
        );
        assert_eq!(messages("1.5 + \"a\""), vec!["cannot add Float and String"]);
        assert_eq!(
            messages("\"a\" - \"b\""),
            vec!["cannot subtract String and String"]
        );
        assert_eq!(
            messages("\"a\" * 2"),
            vec!["cannot multiply String and Int"]
        );
        assert_eq!(messages("\"a\" / 2"), vec!["cannot divide String and Int"]);
        assert_eq!(messages("\"a\" // 2"), vec!["cannot floor-divide String"]);
        assert_eq!(messages("7 % \"a\""), vec!["cannot take modulo of String"]);
        assert_eq!(messages("-\"a\""), vec!["cannot negate String"]);
    }

    #[test]
    fn bools_are_not_numbers() {
        assert_eq!(messages("true + 1"), vec!["cannot add Bool and Int"]);
        assert_eq!(
            messages("2.5 / false"),
            vec!["cannot divide Float and Bool"]
        );
        assert_eq!(messages("true // 2"), vec!["cannot floor-divide Bool"]);
        assert_eq!(messages("-true"), vec!["cannot negate Bool"]);
    }

    #[test]
    fn bool_and_string_annotations_and_ascriptions() {
        assert_eq!(result_type("let x: Bool = true\nx"), TyId::BOOL);
        assert_eq!(result_type("let x: String = \"a\"\nx"), TyId::STRING);
        assert_eq!(result_type("true : Bool"), TyId::BOOL);
        assert_eq!(result_type("\"a\" : String"), TyId::STRING);
        assert_eq!(result_type("(1 < 2) : Bool"), TyId::BOOL);
        assert_eq!(
            diagnostics("let x: Bool = 1")[0].label.as_deref(),
            Some("expected Bool, found Int")
        );
        assert_eq!(
            diagnostics("let x: String = true")[0].label.as_deref(),
            Some("expected String, found Bool")
        );
        assert_eq!(
            diagnostics("\"a\" : Int")[0].label.as_deref(),
            Some("expected Int, found String")
        );
        assert_eq!(
            diagnostics("1 : String")[0].label.as_deref(),
            Some("expected String, found Int")
        );
        // The names are types, not keywords: they can still name variables.
        assert_eq!(result_type("let String = 1\nString"), TyId::INT);
    }

    #[test]
    fn new_types_never_leave_untyped_or_error_entries() {
        for text in [
            "\"a{1}b{2.5}\"",
            "if 1 < 2 { 1 } else { 2 }",
            "if true { 1 } else if false { 2 } else { 3 } + 0.5",
            "let x = 1 == 1 and not false\nx",
            "if true {}",
        ] {
            for ty in all_types(text) {
                assert!(
                    ty != TyId::ERROR && ty != TyId::UNTYPED_INT,
                    "{text}: {ty:?}"
                );
            }
        }
    }

    #[test]
    fn checking_continues_from_an_existing_scope() {
        let mut names = Names::new();
        let mut types = TypeTable::new();
        let parse_with = |text: &str, names: &mut Names| {
            let source = SourceFile::new("test.gcsl", text).unwrap();
            let (tokens, strings) = tokenize_with(&source, names).unwrap();
            parse(&tokens, names.clone(), strings, &source).unwrap()
        };
        let first = parse_with("let a = 1\nlet b = 2.5", &mut names);
        let scope = Checker::new(&first, &mut types).check().unwrap().scope;
        assert_eq!(scope.len(), 2);

        let second = parse_with("a := a + 1\nlet c = b * 2.0\n= a", &mut names);
        let output = Checker::with_scope(&second, scope.clone(), &mut types)
            .check()
            .unwrap();
        assert_eq!(output.scope.len(), 4);
        assert_eq!(output.ty, TyId::INT);
        let checked = output.checked;
        assert_eq!(checked.types.get(result_expr(&second)), TyId::INT);
        let resolution = &checked.resolution;
        let items = second.items(second.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(2));
        assert_eq!(resolution.item_slot(items[1]), Slot(3));
        assert_eq!(resolution.frame_size(), 4);
        let slot_names: Vec<&str> = resolution
            .slot_names()
            .iter()
            .map(|&name| second.names().get(name))
            .collect();
        assert_eq!(slot_names, vec!["a", "b", "a", "c"]);

        // A variable from an earlier input is in the enclosing block of an
        // inner one, with no item to point at.
        let third = parse_with("{ b := 1.5 }", &mut names);
        let diagnostics = Checker::with_scope(&third, scope, &mut types)
            .check()
            .unwrap_err();
        assert_eq!(
            diagnostics[0].message,
            "cannot rebind `b` here: it is declared in an enclosing block"
        );
        assert!(diagnostics[0].labels.is_empty());

        // A field declared at the top level of an input is a field in later
        // ones too, whose type is in the shared table.
        let fourth = parse_with("x = { n = 1 }", &mut names);
        let output = Checker::new(&fourth, &mut types).check().unwrap();
        assert_eq!(
            types.display(output.ty, &names).to_string(),
            "{x: {n: Int}}"
        );
        let fifth = parse_with("x.n + 1", &mut names);
        let sixth = parse_with("x := 2", &mut names);
        assert_eq!(
            Checker::with_scope(&fifth, output.scope.clone(), &mut types)
                .check()
                .unwrap()
                .ty,
            TyId::INT
        );
        let diagnostics = Checker::with_scope(&sixth, output.scope, &mut types)
            .check()
            .unwrap_err();
        assert_eq!(
            diagnostics[0].message,
            "cannot rebind `x`: it is a field, not a variable"
        );
        assert!(diagnostics[0].labels.is_empty());
    }

    // ----- Records -----

    #[test]
    fn record_blocks_have_record_types() {
        assert_eq!(shown_result("{ x = 1; y = \"a\" }"), "{x: Int, y: String}");
        assert_eq!(file_type("x = 1\ny = \"a\""), "{x: Int, y: String}");
        assert_eq!(shown_result("{ b = 1; a = 2.5 }"), "{a: Float, b: Int}");
        assert_eq!(
            shown_result("{ a = { n = 1.5 }; b = {} }"),
            "{a: {n: Float}, b: {}}"
        );
        assert_eq!(shown_result("{ \"a b\" = 1 }"), "{\"a b\": Int}");
        // The same fields in another order are the same type.
        assert_eq!(
            result_type("{ x = 1; y = 2 }"),
            result_type("{ y = 2; x = 1 }")
        );
        // A field's untyped initialiser settles to Int, as a `let`'s does.
        assert_eq!(all_types("{ x = 1 }")[0], TyId::INT);
    }

    #[test]
    fn fields_are_in_scope_like_variables() {
        // Visible to later items of the block.
        assert_eq!(shown_result("{ x = 1; y = x + 1 }"), "{x: Int, y: Int}");
        // `let`s are private to the block, not fields.
        assert_eq!(
            shown_result("{ x = 1.5; let t = x * 2.0; y = t }"),
            "{x: Float, y: Float}"
        );
        // A field shadows an earlier binding, and a later `let` shadows a
        // field without changing it.
        assert_eq!(
            shown_result("let x = \"s\"\n{ x = 1; y = x + 1 }"),
            "{x: Int, y: Int}"
        );
        assert_eq!(
            shown_result("{ x = 1; let x = \"s\"; y = x }"),
            "{x: Int, y: String}"
        );
        // Not visible before its item, nor after its block.
        assert_eq!(messages("{ y = x; x = 1 }"), vec!["undefined variable `x`"]);
        assert_eq!(
            messages("let r = { x = 1 }\nx"),
            vec!["undefined variable `x`"]
        );
        // Statements of type `{}` are allowed, the last one included.
        assert_eq!(shown_result("{ x = 1; {}; if true {} }"), "{x: Int}");
    }

    #[test]
    fn a_record_block_has_no_result() {
        let diagnostic = &diagnostics("{ x = 1; = 2 }")[0];
        assert_eq!(
            diagnostic.message,
            "a block with fields cannot have a result"
        );
        assert_eq!(diagnostic.span, Span::new(9, 10));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 7), "first field declared here")]
        );
        assert_eq!(
            diagnostic.notes,
            vec![
                "a block is either a record of its fields or a single value; bind the value \
                 with `let` or make it a field"
            ]
        );
        // Wherever the result is, it is this error, not the ordering one.
        assert_eq!(
            labels(&diagnostics("{ = 2; x = 1 }")[0]),
            vec![(Span::new(7, 12), "first field declared here")]
        );
        assert_eq!(
            messages("{ = 2; x = 1 }"),
            vec!["a block with fields cannot have a result"]
        );
        // A final expression is a statement, so a value there is unused.
        let diagnostic = &diagnostics("{ x = 1; x + 1 }")[0];
        assert_eq!(diagnostic.message, "unused value of type Int");
        assert_eq!(diagnostic.span, Span::new(9, 14));
        assert_eq!(messages("x = 1\n2.5"), vec!["unused value of type Float"]);
    }

    #[test]
    fn rebinding_a_field_is_an_error() {
        let diagnostic = &diagnostics("{ x = 1; x := 2 }")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `x`: it is a field, not a variable"
        );
        assert_eq!(diagnostic.span, Span::new(9, 10));
        assert_eq!(labels(diagnostic), vec![(Span::new(2, 7), "declared here")]);
        assert_eq!(
            diagnostic.notes,
            vec!["rebind with `x = ...` is not allowed either: a field is defined once"]
        );
        // A field of an enclosing block, too.
        assert_eq!(
            messages("{ x = 1; y = { x := 2 } }"),
            vec!["cannot rebind `x`: it is a field, not a variable"]
        );
        // A field declared with a quoted name is the same name.
        assert_eq!(
            messages("\"x\" = 1\nx := 2"),
            vec!["cannot rebind `x`: it is a field, not a variable"]
        );
        // The rejected rebind still declares a variable, so uses do not
        // cascade.
        assert_eq!(
            messages("{ x = 1; x := 2.5; y = x * 1.5 }"),
            vec!["cannot rebind `x`: it is a field, not a variable"]
        );
    }

    #[test]
    fn a_field_defined_twice_is_an_error() {
        let diagnostic = &diagnostics("{ x = 1; x = 2 }")[0];
        assert_eq!(diagnostic.message, "field `x` is defined twice");
        assert_eq!(diagnostic.span, Span::new(9, 10));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 3), "first defined here")]
        );
        // Each repeat is reported against the first.
        let repeats = diagnostics("{ x = 1; y = 2; x = 3; x = 4 }");
        assert_eq!(repeats.len(), 2);
        assert_eq!(repeats[0].span, Span::new(16, 17));
        assert_eq!(repeats[1].span, Span::new(23, 24));
        for repeat in &repeats {
            assert_eq!(
                labels(repeat),
                vec![(Span::new(2, 3), "first defined here")]
            );
        }
        // A quoted name that is the same name.
        assert_eq!(
            errors("{ x = 1; \"x\" = 2 }"),
            vec![("field `x` is defined twice".to_owned(), Span::new(9, 12))]
        );
        assert_eq!(
            messages("{ \"a b\" = 1; \"a b\" = 2 }"),
            vec!["field `\"a b\"` is defined twice"]
        );
        // The record is an error, so it does not cascade.
        assert_eq!(
            messages("let r = { x = 1; x = 2 }\nr.x + 1.5\nr.y"),
            vec!["field `x` is defined twice"]
        );
    }

    #[test]
    fn field_access() {
        assert_eq!(
            result_type("let p = { x = 1; y = \"a\" }\np.y"),
            TyId::STRING
        );
        assert_eq!(
            result_type("let p = { x = 1; y = \"a\" }\np.x + 1"),
            TyId::INT
        );
        assert_eq!(result_type("{ a = { n = 1.5 } }.a.n"), TyId::FLOAT);
        assert_eq!(result_type("{ \"a b\" = true }.\"a b\""), TyId::BOOL);
        assert_eq!(result_type("-{ x = 2 }.x"), TyId::INT);
        assert_eq!(shown_result("{ a = { n = 1 } }.a"), "{n: Int}");
    }

    #[test]
    fn field_access_errors() {
        let diagnostic = &diagnostics("let p = { x = 1; y = 2 }\np.z")[0];
        assert_eq!(diagnostic.message, "no field `z` in `{x: Int, y: Int}`");
        assert_eq!(diagnostic.span, Span::new(26, 28));
        assert!(diagnostic.labels.is_empty());
        assert_eq!(diagnostic.notes, vec!["the fields are `x` and `y`"]);
        let diagnostic = &diagnostics("{}.x")[0];
        assert_eq!(diagnostic.message, "no field `x` in `{}`");
        assert_eq!(diagnostic.span, Span::new(2, 4));
        assert_eq!(diagnostic.notes, vec!["the record has no fields"]);
        assert_eq!(
            diagnostics("{ x = 1 }.y")[0].notes,
            vec!["the only field is `x`"]
        );
        assert_eq!(
            diagnostics("{ c = 1; a = 2; \"b b\" = 3 }.z")[0].notes,
            vec!["the fields are `a`, `\"b b\"` and `c`"]
        );
        assert_eq!(
            messages("{ x = 1 }.\"a b\""),
            vec!["no field `\"a b\"` in `{x: Int}`"]
        );
        // A value that is not a record.
        let diagnostic = &diagnostics("1.x")[0];
        assert_eq!(diagnostic.message, "cannot access field `x` of Int");
        assert_eq!(diagnostic.span, Span::new(1, 3));
        assert_eq!(labels(diagnostic), vec![(Span::new(0, 1), "this is Int")]);
        let diagnostic = &diagnostics("let s = \"s\"\ns . x")[0];
        assert_eq!(diagnostic.message, "cannot access field `x` of String");
        assert_eq!(diagnostic.span, Span::new(13, 17));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(12, 13), "this is String")]
        );
        // An erroneous record is not reported again.
        assert_eq!(messages("q.x"), vec!["undefined variable `q`"]);
        assert_eq!(messages("q.x.y + 1"), vec!["undefined variable `q`"]);
        assert_eq!(
            messages("{ x = 1 }.z.w"),
            vec!["no field `z` in `{x: Int}`"]
        );
    }

    #[test]
    fn a_record_block_is_checked_against_its_annotation() {
        // Literals adapt to the expected field types, nested blocks too,
        // through parentheses, and in ascriptions.
        assert_eq!(all_types("let p: {x: Float} = { x = 1 }")[0], TyId::FLOAT);
        assert_eq!(
            all_types("let p: {a: {n: Float}} = { a = { n = 1 } }")[0],
            TyId::FLOAT
        );
        assert_eq!(
            all_types("let p: {x: Float} = (({ x = 1 }))")[0],
            TyId::FLOAT
        );
        assert_eq!(all_types("{ x = 1 } : {x: Float}")[0], TyId::FLOAT);
        assert_eq!(
            shown_result("let p: {x: Float} = { x = -(1 + 2) }\np"),
            "{x: Float}"
        );
        assert_eq!(shown_result("{ x = 1 } : {x: Float}"), "{x: Float}");
        // Each parenthesis has the block's type.
        let (ast, types, output) = check_text("let p: {x: Float} = ({ x = 1 })");
        let checked = output.unwrap().checked;
        let ItemKind::Let { value, .. } = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected a let");
        };
        assert_eq!(
            types
                .display(checked.types.get(value), ast.names())
                .to_string(),
            "{x: Float}"
        );
        // A field can use earlier fields at their expected types.
        assert_eq!(
            shown_result("let p: {x: Float, y: Float} = { x = 1; y = x + 1 }\np"),
            "{x: Float, y: Float}"
        );
        // A mismatch inside is reported where it is.
        let diagnostic = &diagnostics("let p: {x: Int} = { x = \"s\" }")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(24, 27));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found String")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 15), "expected because of this annotation")]
        );
        // A block without fields, or a record block against a type that is
        // not a record, is checked as a whole.
        assert_eq!(
            diagnostics("let p: {x: Int} = { 1 }")[0].label.as_deref(),
            Some("expected {x: Int}, found Int")
        );
        assert_eq!(
            diagnostics("let p: Int = { x = 1 }")[0].label.as_deref(),
            Some("expected Int, found {x: Int}")
        );
    }

    #[test]
    fn a_missing_field_is_an_error() {
        let diagnostic = &diagnostics("let p: {x: Int, y: Int} = { x = 1 }")[0];
        assert_eq!(diagnostic.message, "missing field `y` in record");
        assert_eq!(diagnostic.span, Span::new(34, 35));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 23), "expected because of this annotation")]
        );
        assert_eq!(
            diagnostic.notes,
            vec!["the type `{x: Int, y: Int}` needs it"]
        );
        // Each missing field, in alphabetical order.
        assert_eq!(
            messages("{ c = 1 } : {b: Int, c: Int, a: Int}"),
            vec!["missing field `a` in record", "missing field `b` in record"]
        );
        assert_eq!(
            labels(&diagnostics("{ c = 1 } : {b: Int, c: Int}")[0]),
            vec![(Span::new(12, 28), "ascribed here")]
        );
        // In a nested block, at its own closing brace; the warning about
        // its extra field comes back with it, in source order.
        assert_eq!(
            errors("let p: {a: {n: Int}} = { a = { m = 1 } }"),
            vec![
                (
                    "field `m` is not in `{n: Int}`".to_owned(),
                    Span::new(31, 32)
                ),
                ("missing field `n` in record".to_owned(), Span::new(37, 38))
            ]
        );
        // `{}` has no fields, so it is not a record block: it is checked as
        // a whole.
        assert_eq!(
            diagnostics("let p: {a: {n: Int}} = { a = {} }")[0]
                .label
                .as_deref(),
            Some("expected {n: Int}, found {}")
        );
        // It does not cascade.
        assert_eq!(
            messages("let p: {x: Int, y: Int} = { x = 1 }\np.y + 1.5"),
            vec!["missing field `y` in record"]
        );
    }

    #[test]
    fn an_extra_field_is_a_warning() {
        let text = "let p: {x: Int} = { x = 1; z = 2 }\np";
        let found = warnings(text);
        assert_eq!(found.len(), 1);
        let warning = &found[0];
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.message, "field `z` is not in `{x: Int}`");
        assert_eq!(warning.span, Span::new(27, 28));
        assert_eq!(
            labels(warning),
            vec![(Span::new(7, 15), "expected because of this annotation")]
        );
        assert_eq!(
            warning.notes,
            vec!["it is allowed, but nothing with this type can see it"]
        );
        // The check succeeds, and the variable has the annotated type.
        assert_eq!(shown_result(text), "{x: Int}");
        assert_eq!(
            messages("let p: {x: Int} = { x = 1; z = 2 }\np.z"),
            vec![
                "field `z` is not in `{x: Int}`",
                "no field `z` in `{x: Int}`"
            ]
        );
        // In an ascription, against `{}`, and in a nested block.
        assert_eq!(
            labels(&warnings("{ x = 1; z = 2 } : {x: Int}")[0]),
            vec![(Span::new(19, 27), "ascribed here")]
        );
        assert_eq!(
            warnings("let e: {} = { x = 1 }")[0].message,
            "field `x` is not in `{}`"
        );
        assert_eq!(
            warnings("let p: {a: {n: Int}} = { a = { n = 1; m = 2 } }")[0].message,
            "field `m` is not in `{n: Int}`"
        );
        // With errors, the warnings come back with them in source order.
        let diagnostics = diagnostics("let p: {x: Int} = { x = 1; z = 2 }\nq");
        let shown: Vec<(Severity, &str)> = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.severity, diagnostic.message.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (Severity::Warning, "field `z` is not in `{x: Int}`"),
                (Severity::Error, "undefined variable `q`")
            ]
        );
    }

    #[test]
    fn subtyping_through_a_variable_warns_about_nothing() {
        let text = "let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\np";
        assert!(warnings(text).is_empty());
        assert_eq!(shown_result(text), "{x: Int}");
        let text = "let q = { a = { n = 1; m = 2 }; b = 1 }\nlet p: {a: {n: Int}} = q\np";
        assert!(warnings(text).is_empty());
        assert_eq!(shown_result(text), "{a: {n: Int}}");
        assert!(warnings("let q = { x = 1 }\nq : {}").is_empty());
        // A block in parentheses inside another expression is not checked
        // field by field.
        assert!(
            warnings("let p: {x: Int} = if true { { x = 1; y = 2 } } else { { x = 2 } }")
                .is_empty()
        );
        // A record that lacks a field is not a subtype.
        let diagnostic = &diagnostics("let q = { y = 1 }\nlet p: {x: Int} = q")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected {x: Int}, found {y: Int}")
        );
        assert_eq!(
            diagnostics("let q = { x = 1 }\nlet p: {x: Float} = q")[0]
                .label
                .as_deref(),
            Some("expected {x: Float}, found {x: Int}")
        );
    }

    #[test]
    fn if_joins_record_branches() {
        assert_eq!(
            shown_result("if true { { x = 1; y = 2 } } else { { x = 3 } }"),
            "{x: Int}"
        );
        // No shared fields: the empty record, which is still a record.
        assert_eq!(
            shown_result("if true { { x = 1 } } else { { y = 1 } }"),
            "{}"
        );
        assert_eq!(shown_result("if true { {} } else { { x = 1 } }"), "{}");
        // A shared field whose types have no join is not shared.
        assert_eq!(
            shown_result("if true { { x = 1; y = 2 } } else { { x = 1.5; y = 3 } }"),
            "{y: Int}"
        );
        // Nested.
        assert_eq!(
            shown_result("if true { { a = { n = 1; m = 2 }; b = 1 } } else { { a = { n = 3 } } }"),
            "{a: {n: Int}}"
        );
        // A record branch and a non-record branch mismatch.
        let diagnostic = &diagnostics("if true { { x = 1 } } else { 1 }")[0];
        assert_eq!(diagnostic.message, "`if` branches have different types");
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(8, 21), "this is {x: Int}"),
                (Span::new(27, 32), "this is Int")
            ]
        );
    }

    #[test]
    fn record_equality_needs_identical_types() {
        assert_eq!(result_type("{ x = 1 } == { x = 2 }"), TyId::BOOL);
        assert_eq!(
            result_type("{ x = 1; y = 2 } != { y = 1; x = 2 }"),
            TyId::BOOL
        );
        assert_eq!(result_type("{} == {}"), TyId::BOOL);
        let diagnostic = &diagnostics("{ x = 1 } == { x = 1; y = 2 }")[0];
        assert_eq!(
            diagnostic.message,
            "cannot compare `{x: Int}` and `{x: Int, y: Int}`"
        );
        assert_eq!(diagnostic.span, Span::new(10, 12));
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(0, 9), "{x: Int}"),
                (Span::new(13, 29), "{x: Int, y: Int}")
            ]
        );
        // Untyped literals in record blocks settle to Int.
        assert_eq!(
            messages("{ x = 1 } == { x = 1.5 }"),
            vec!["cannot compare `{x: Int}` and `{x: Float}`"]
        );
        assert_eq!(
            messages("{} != { x = 1 }"),
            vec!["cannot compare `{}` and `{x: Int}`"]
        );
        assert_eq!(
            messages("{ x = 1 } == 1"),
            vec!["cannot compare `{x: Int}` and Int"]
        );
    }

    #[test]
    fn records_cannot_be_ordered() {
        let diagnostic = &diagnostics("{ x = 1 } < { x = 2 }")[0];
        assert_eq!(diagnostic.message, "cannot order `{x: Int}`");
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(0, 9), "{x: Int}"),
                (Span::new(12, 21), "{x: Int}")
            ]
        );
        assert_eq!(messages("{} >= {}"), vec!["cannot order `{}`"]);
        assert_eq!(messages("1 < { x = 1 }"), vec!["cannot order `{x: Int}`"]);
    }

    #[test]
    fn record_type_annotations() {
        assert_eq!(
            shown_result(
                "let p: { \"a b\": String, nested: {n: Int} } = { \"a b\" = \"s\"; nested = { n = 1 } }\np"
            ),
            "{\"a b\": String, nested: {n: Int}}"
        );
        // A name that appears twice in a type.
        assert_eq!(
            errors("1 : {x: Int, x: Int}"),
            vec![(
                "field `x` appears twice in this type".to_owned(),
                Span::new(13, 14)
            )]
        );
        assert_eq!(
            messages("let p: {a: Int, \"a\": Int, a: Int} = { a = 1 }\np.a + 1.5"),
            vec![
                "field `a` appears twice in this type",
                "field `a` appears twice in this type"
            ]
        );
    }
}
