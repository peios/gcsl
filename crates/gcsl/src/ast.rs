//! The abstract syntax tree, stored in arenas.
//!
//! Expression nodes live contiguously in one `Vec<Expr>` and refer to each
//! other by [`ExprId`], a `u32` index. Compared with boxed recursive enums
//! this keeps the tree in one allocation, makes nodes small and
//! `Copy`-friendly, and lets later passes keep per-node data in side tables
//! indexed by `ExprId` (see [`crate::check::Types`]).
//!
//! Items (the statements of a block body) and type expressions get arenas
//! of their own, indexed by [`ItemId`] and [`TypeExprId`]. A block body is a
//! list of items. Nodes are allocated children-first, so a block's items are
//! not contiguous in the item arena: a nested block's items are allocated
//! while the outer block's are being parsed. Lists of items are therefore
//! kept in a separate pool, [`Ast::items`], each list a contiguous
//! [`ItemRange`] within it. The parts of interpolated strings live in a pool
//! of their own in the same way, [`Ast::parts`], as do the fields of record
//! type expressions, [`Ast::field_tys`], and the same pattern will serve
//! argument lists.
//!
//! An [`Ast`] is built through an [`AstBuilder`]: nodes are allocated
//! children-first, and the builder is finished once the root is known, so an
//! `Ast` always has a valid root. The `Ast` also owns the [`Names`] table its
//! identifiers were interned into and the [`Strings`] table holding the text
//! of its string literals.

use std::fmt;

use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::{FieldName, Quoted, StrId, Strings};

/// Index of an [`Expr`] in the arena of the [`Ast`] (or [`AstBuilder`]) that
/// allocated it.
///
/// Ids are only meaningful for the arena that produced them; using one with a
/// different arena is a logic error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(u32);

impl ExprId {
    /// The id as an index into the arena and into side tables.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of an [`Item`] in the item arena of the [`Ast`] (or [`AstBuilder`])
/// that allocated it. Like [`ExprId`], only meaningful for that arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(u32);

impl ItemId {
    /// The id as an index into the arena and into side tables.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a [`TypeExpr`] in the type arena of the [`Ast`] (or
/// [`AstBuilder`]) that allocated it. Like [`ExprId`], only meaningful for
/// that arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeExprId(u32);

impl TypeExprId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A list of items (a block body): a contiguous range of the item-list pool
/// of the [`Ast`] that allocated it. Resolve it with [`Ast::items`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemRange {
    start: u32,
    len: u32,
}

/// The parts of an interpolated string: a contiguous range of the part pool
/// of the [`Ast`] that allocated it. Resolve it with [`Ast::parts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PartRange {
    start: u32,
    len: u32,
}

/// The fields of a record type expression: a contiguous range of the
/// field-type pool of the [`Ast`] that allocated it. Resolve it with
/// [`Ast::field_tys`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldTyRange {
    start: u32,
    len: u32,
}

/// One field of a record type expression, `NAME: TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldTy {
    /// The field's name, plain or quoted in source.
    pub name: Name,
    /// Where the name is written.
    pub span: Span,
    /// The field's type.
    pub ty: TypeExprId,
}

/// One part of an interpolated string.
///
/// The parts of an [`ExprKind::Interp`] alternate, starting and ending with
/// a [`StrPart::Lit`], which may be empty: `"a{x}b{y}"` is `a`, `x`, `b`,
/// `y` and the empty text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrPart {
    /// Literal text.
    Lit(StrId),
    /// An interpolated expression, whose value is shown as text.
    Expr(ExprId),
}

/// A prefix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `-x`
    Neg,
    /// `not x`
    Not,
}

impl UnaryOp {
    /// The verb used in diagnostics, e.g. "cannot negate ..." or "`not`
    /// requires Bool operands".
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            UnaryOp::Neg => "negate",
            UnaryOp::Not => "not",
        }
    }

    /// The operator as written in source.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "not",
        }
    }

    /// The span of this operator's symbol when it starts at byte `start`.
    #[must_use]
    pub const fn span_at(self, start: u32) -> Span {
        symbol_span(self.symbol(), start)
    }
}

/// The span of an operator `symbol` starting at byte `start`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "operator symbols are a few bytes long"
)]
pub(crate) const fn symbol_span(symbol: &str, start: u32) -> Span {
    Span::new(start, start + symbol.len() as u32)
}

/// An infix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// `a + b`
    Add,
    /// `a - b`
    Sub,
    /// `a * b`
    Mul,
    /// `a / b`, always float division.
    Div,
    /// `a // b`, integer division rounding toward negative infinity.
    FloorDiv,
    /// `a % b`, the remainder matching [`BinaryOp::FloorDiv`].
    Rem,
    /// `a == b`
    Eq,
    /// `a != b`
    Ne,
    /// `a < b`
    Lt,
    /// `a <= b`
    Le,
    /// `a > b`
    Gt,
    /// `a >= b`
    Ge,
    /// `a and b`, short-circuiting.
    And,
    /// `a or b`, short-circuiting.
    Or,
}

impl BinaryOp {
    /// The verb used in diagnostics, e.g. "cannot add Int and Float",
    /// "cannot take modulo of Float", "cannot compare Int and String" or
    /// "`and` requires Bool operands".
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            BinaryOp::Add => "add",
            BinaryOp::Sub => "subtract",
            BinaryOp::Mul => "multiply",
            BinaryOp::Div => "divide",
            BinaryOp::FloorDiv => "floor-divide",
            BinaryOp::Rem => "take modulo of",
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => "compare",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }

    /// The operator as written in source.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::FloorDiv => "//",
            BinaryOp::Rem => "%",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }

    /// Whether this is one of the comparison operators, `==`, `!=`, `<`,
    /// `<=`, `>` and `>=`.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        )
    }

    /// The span of this operator's symbol when it starts at byte `start`.
    #[must_use]
    pub const fn span_at(self, start: u32) -> Span {
        symbol_span(self.symbol(), start)
    }
}

/// What an [`Expr`] is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExprKind {
    /// An integer literal.
    Int(i64),
    /// A float literal.
    Float(f64),
    /// `true` or `false`.
    Bool(bool),
    /// A string literal without interpolations.
    Str(StrId),
    /// A string literal with interpolations: its parts, alternating literal
    /// text and expressions (see [`StrPart`]).
    Interp(PartRange),
    /// `if COND { ... } else ...`, including `else if` chains and an `if`
    /// without `else`.
    If {
        /// The condition.
        cond: ExprId,
        /// The block run when the condition is true.
        then: ExprId,
        /// What runs otherwise: a block, another `if` for `else if`, or,
        /// when the source has no `else`, an empty block the parser adds
        /// with an empty span at the end of `then`, so that no later stage
        /// has to handle a missing branch.
        else_: ExprId,
        /// Whether the source has an `else`.
        has_else: bool,
    },
    /// A prefix operator applied to an operand.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: ExprId,
    },
    /// An infix operator applied to two operands.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Byte offset of the operator's symbol, so diagnostics can point at
        /// it: its span is `op.span_at(op_start)`. Only the start is stored
        /// because the symbol's length is known from `op`, which keeps
        /// [`Expr`] small.
        op_start: u32,
        /// The left operand.
        lhs: ExprId,
        /// The right operand.
        rhs: ExprId,
    },
    /// A parenthesised expression. Kept so `--dump-ast` shows the source
    /// shape; it carries no semantics and is dropped by lowering.
    Paren(ExprId),
    /// A use of a variable (or of a field, by its name, later in the block
    /// that declares it).
    Name(Name),
    /// A block `{ ... }`: a body of items in a scope of its own. With at
    /// least one [`ItemKind::Field`] it is a record block, whose value is
    /// the record of its fields.
    Block(ItemRange),
    /// Field access, `EXPR.NAME` or `EXPR."quoted name"`. The span of the
    /// `.NAME` part is not stored; see [`Expr::member_span`].
    Field {
        /// The record whose field is read.
        expr: ExprId,
        /// The field's name.
        name: Name,
    },
    /// A type ascription `expr : Type`.
    Ascribe {
        /// The expression whose type is stated.
        expr: ExprId,
        /// The stated type.
        ty: TypeExprId,
    },
}

/// One node of the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expr {
    /// What the node is.
    pub kind: ExprKind,
    /// The source it covers. A `Paren` covers its parentheses and a `Block`
    /// its braces, a `Binary` its left operand through its right operand, a
    /// `Unary` its operator through its operand, and an `Ascribe` its
    /// expression through its type. A string literal covers its prefix and
    /// delimiters, an `If` its `if` keyword through its last branch, and a
    /// `Field` its record expression through its name.
    pub span: Span,
}

impl Expr {
    /// The span of the operator's symbol, for operator nodes.
    #[must_use]
    pub const fn operator_span(&self) -> Option<Span> {
        match self.kind {
            ExprKind::Unary { op, .. } => Some(op.span_at(self.span.start)),
            ExprKind::Binary { op, op_start, .. } => Some(op.span_at(op_start)),
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Interp(_)
            | ExprKind::If { .. }
            | ExprKind::Paren(_)
            | ExprKind::Name(_)
            | ExprKind::Block(_)
            | ExprKind::Field { .. }
            | ExprKind::Ascribe { .. } => None,
        }
    }

    /// The span of the `.NAME` part of a `Field` node, for diagnostics about
    /// the access: from the end of its record expression, found in `ast`,
    /// to the end of the name. `None` for every other node.
    #[must_use]
    pub fn member_span(&self, ast: &Ast) -> Option<Span> {
        match self.kind {
            ExprKind::Field { expr, .. } => Some(Span::new(ast.get(expr).span.end, self.span.end)),
            _ => None,
        }
    }

    /// The span of the `if` keyword, for `If` nodes.
    #[must_use]
    pub const fn keyword_span(&self) -> Option<Span> {
        match self.kind {
            ExprKind::If { .. } => Some(symbol_span("if", self.span.start)),
            _ => None,
        }
    }
}

/// What an [`Item`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// `let NAME = EXPR` or `let NAME: TYPE = EXPR`: declares a variable.
    Let {
        /// The variable's name.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// The annotated type, if any.
        ty: Option<TypeExprId>,
        /// The initialiser.
        value: ExprId,
    },
    /// `NAME = EXPR` or `"quoted name" = EXPR`: declares a field of the
    /// record its block evaluates to, which is also a name in scope for the
    /// block's later items.
    Field {
        /// The field's name.
        name: Name,
        /// Where the name is written, quotes included for a quoted name.
        name_span: Span,
        /// The field's value.
        value: ExprId,
    },
    /// `NAME := EXPR`: rebinds a variable declared in the same block.
    Rebind {
        /// The variable's name.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// The new value.
        value: ExprId,
    },
    /// `= EXPR`: the block's result.
    Result(ExprId),
    /// An expression evaluated on its own.
    Expr(ExprId),
}

/// One item of a block body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    /// What the item is.
    pub kind: ItemKind,
    /// The source it covers: from `let`, the name, `=` or the start of the
    /// expression, through the end of the expression.
    pub span: Span,
}

/// What a [`TypeExpr`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeExprKind {
    /// A type written as a name, such as `Int`.
    Named(Name),
    /// A record type, `{x: Int, "a b": String}`: its fields in source
    /// order. Repeated names are allowed here and reported by the checker.
    Record(FieldTyRange),
}

/// A type as written in source, in an annotation or an ascription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeExpr {
    /// What the type expression is.
    pub kind: TypeExprKind,
    /// The source it covers.
    pub span: Span,
}

/// Converts an arena length to the next id, saturating past `u32::MAX`.
///
/// An arena holds at most `u32::MAX` entries. Every entry consumes at least
/// one byte of source and sources are limited to `u32::MAX` bytes, so a
/// parser can never reach the limit; past it, ids would saturate.
fn next_id(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// Arenas under construction. Allocate children before their parents, then
/// call [`AstBuilder::finish`] with the root.
#[derive(Clone, Debug, Default)]
pub struct AstBuilder {
    exprs: Vec<Expr>,
    items: Vec<Item>,
    item_lists: Vec<ItemId>,
    parts: Vec<StrPart>,
    type_exprs: Vec<TypeExpr>,
    field_tys: Vec<FieldTy>,
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
        let id = TypeExprId(next_id(self.type_exprs.len()));
        self.type_exprs.push(ty);
        id
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
        let start = next_id(self.field_tys.len());
        self.field_tys.extend_from_slice(fields);
        FieldTyRange {
            start,
            len: next_id(fields.len()),
        }
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
        &self.type_exprs[id.index()]
    }

    /// Finishes the arenas with `root` as the file's body, taking ownership
    /// of the `names` its identifiers were interned into and the `strings`
    /// holding the text of its string literals.
    #[must_use]
    pub fn finish(self, root: ItemRange, names: Names, strings: Strings) -> Ast {
        Ast {
            exprs: self.exprs,
            items: self.items,
            item_lists: self.item_lists,
            parts: self.parts,
            type_exprs: self.type_exprs,
            field_tys: self.field_tys,
            root,
            names,
            strings,
        }
    }
}

/// A complete syntax tree: the arenas, the file's body, the names and the
/// string literals' text.
#[derive(Clone, Debug)]
pub struct Ast {
    exprs: Vec<Expr>,
    items: Vec<Item>,
    item_lists: Vec<ItemId>,
    parts: Vec<StrPart>,
    type_exprs: Vec<TypeExpr>,
    field_tys: Vec<FieldTy>,
    root: ItemRange,
    names: Names,
    strings: Strings,
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
        &self.type_exprs[id.index()]
    }

    /// Returns the fields of the record type expression `range`, in source
    /// order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn field_tys(&self, range: FieldTyRange) -> &[FieldTy] {
        let start = range.start as usize;
        &self.field_tys[start..start + range.len as usize]
    }

    /// The file's body: the outermost block's items.
    #[must_use]
    pub fn root(&self) -> ItemRange {
        self.root
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

/// Per-node information written by [`Ast::pretty_with`].
pub trait Annotate {
    /// Writes the annotation for node `id`, including any leading space.
    ///
    /// # Errors
    ///
    /// Propagates errors from the formatter.
    fn annotate(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

/// The empty annotation used by [`Ast::pretty`].
#[derive(Clone, Copy, Debug)]
pub struct NoAnnotation;

impl Annotate for NoAnnotation {
    fn annotate(&self, _: ExprId, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// An [`Ast`] rendered as indented S-expressions, one node per line:
///
/// ```text
/// (file
///   (let x : Float @0..16
///     (+ @10..16
///       (int 1 @10..11)
///       (int 2 @15..16)))
///   (expr @17..18
///     (name x @17..18)))
/// ```
///
/// Items print as `(let x ...)`, `(let x : Type ...)`, `(field x ...)`,
/// `(rebind x ...)`, `(result ...)` and `(expr ...)`, followed by their
/// expression; blocks as `(block ...)` followed by their items; ascriptions
/// as `(: Type ...)`; field accesses as `(. x ...)` followed by their record
/// expression. A field name that is not a plain identifier is quoted, as in
/// `(field "a b" ...)`. Record types print as written, fields in source
/// order: `{x: Int, "a b": {}}`.
/// Booleans print as `(bool true)`, string literals as `(str "text")` with
/// the text quoted and escaped, and interpolated strings as `(interp ...)`
/// followed by their parts, literal text as `(lit "text")` (which has no
/// span) and expressions as their nodes. `(if ...)` is followed by its
/// condition, its block and its else branch, which for an `if` without
/// `else` is the parser's empty block. Operators print as their symbol,
/// except negation, `(neg ...)`. The file has no span. There is no trailing
/// newline.
pub struct Pretty<'a, A> {
    ast: &'a Ast,
    annotation: &'a A,
}

impl<A: Annotate> Pretty<'_, A> {
    fn items(&self, f: &mut fmt::Formatter<'_>, range: ItemRange, depth: usize) -> fmt::Result {
        for &item in self.ast.items(range) {
            f.write_str("\n")?;
            self.item(f, item, depth)?;
        }
        Ok(())
    }

    fn item(&self, f: &mut fmt::Formatter<'_>, id: ItemId, depth: usize) -> fmt::Result {
        let item = self.ast.item(id);
        let names = self.ast.names();
        write!(f, "{:indent$}(", "", indent = depth * 2)?;
        let value = match item.kind {
            ItemKind::Let {
                name, ty, value, ..
            } => {
                write!(f, "let {}", names.get(name))?;
                if let Some(ty) = ty {
                    f.write_str(" : ")?;
                    self.type_expr(f, ty)?;
                }
                value
            }
            ItemKind::Field { name, value, .. } => {
                write!(f, "field {}", FieldName(names.get(name)))?;
                value
            }
            ItemKind::Rebind { name, value, .. } => {
                write!(f, "rebind {}", names.get(name))?;
                value
            }
            ItemKind::Result(value) => {
                f.write_str("result")?;
                value
            }
            ItemKind::Expr(value) => {
                f.write_str("expr")?;
                value
            }
        };
        writeln!(f, " @{}", item.span)?;
        self.node(f, value, depth + 1)?;
        f.write_str(")")
    }

    /// Writes a type expression as it is written in source.
    fn type_expr(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        let names = self.ast.names();
        match self.ast.type_expr(id).kind {
            TypeExprKind::Named(name) => f.write_str(names.get(name)),
            TypeExprKind::Record(fields) => {
                f.write_str("{")?;
                for (index, field) in self.ast.field_tys(fields).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: ", FieldName(names.get(field.name)))?;
                    self.type_expr(f, field.ty)?;
                }
                f.write_str("}")
            }
        }
    }

    fn node(&self, f: &mut fmt::Formatter<'_>, id: ExprId, depth: usize) -> fmt::Result {
        let expr = self.ast.get(id);
        write!(f, "{:indent$}(", "", indent = depth * 2)?;
        match expr.kind {
            ExprKind::Int(value) => write!(f, "int {value}")?,
            ExprKind::Float(value) => write!(f, "float {value:?}")?,
            ExprKind::Bool(value) => write!(f, "bool {value}")?,
            ExprKind::Str(id) => write!(f, "str {}", Quoted(self.ast.strings().get(id)))?,
            ExprKind::Interp(_) => f.write_str("interp")?,
            ExprKind::If { .. } => f.write_str("if")?,
            ExprKind::Unary { op, .. } => match op {
                UnaryOp::Neg => f.write_str("neg")?,
                UnaryOp::Not => f.write_str("not")?,
            },
            ExprKind::Binary { op, .. } => f.write_str(op.symbol())?,
            ExprKind::Paren(_) => f.write_str("paren")?,
            ExprKind::Name(name) => write!(f, "name {}", self.ast.names().get(name))?,
            ExprKind::Block(_) => f.write_str("block")?,
            ExprKind::Field { name, .. } => {
                write!(f, ". {}", FieldName(self.ast.names().get(name)))?;
            }
            ExprKind::Ascribe { ty, .. } => {
                f.write_str(": ")?;
                self.type_expr(f, ty)?;
            }
        }
        self.annotation.annotate(id, f)?;
        write!(f, " @{}", expr.span)?;
        match expr.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Name(_) => {}
            ExprKind::Unary { operand, .. }
            | ExprKind::Paren(operand)
            | ExprKind::Field { expr: operand, .. }
            | ExprKind::Ascribe { expr: operand, .. } => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                f.write_str("\n")?;
                self.node(f, lhs, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, rhs, depth + 1)?;
            }
            ExprKind::If {
                cond, then, else_, ..
            } => {
                for child in [cond, then, else_] {
                    f.write_str("\n")?;
                    self.node(f, child, depth + 1)?;
                }
            }
            ExprKind::Interp(parts) => {
                for &part in self.ast.parts(parts) {
                    f.write_str("\n")?;
                    match part {
                        StrPart::Lit(text) => write!(
                            f,
                            "{:indent$}(lit {})",
                            "",
                            Quoted(self.ast.strings().get(text)),
                            indent = (depth + 1) * 2
                        )?,
                        StrPart::Expr(expr) => self.node(f, expr, depth + 1)?,
                    }
                }
            }
            ExprKind::Block(items) => self.items(f, items, depth + 1)?,
        }
        f.write_str(")")
    }
}

impl<A: Annotate> fmt::Display for Pretty<'_, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(file")?;
        self.items(f, self.ast.root, 1)?;
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(builder: &mut AstBuilder, value: i64, at: u32) -> ExprId {
        builder.alloc(Expr {
            kind: ExprKind::Int(value),
            span: Span::new(at, at + 1),
        })
    }

    /// Finishes `builder` with a body of one expression item, `root`.
    fn finish_expr(mut builder: AstBuilder, root: ExprId) -> Ast {
        let span = builder.get(root).span;
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(root),
            span,
        });
        let body = builder.push_item_list(&[item]);
        builder.finish(body, Names::new(), Strings::new())
    }

    #[test]
    fn nodes_are_small() {
        // Guards the arena's density; grow this deliberately, not by accident.
        assert!(size_of::<Expr>() <= 24);
    }

    #[test]
    fn alloc_returns_sequential_ids() {
        let mut builder = AstBuilder::new();
        let a = leaf(&mut builder, 1, 0);
        let b = leaf(&mut builder, 2, 4);
        assert_eq!((a.index(), b.index()), (0, 1));
        assert_eq!(builder.get(b).kind, ExprKind::Int(2));
        let ast = finish_expr(builder, b);
        assert_eq!(ast.items(ast.root()).len(), 1);
        assert_eq!(ast.len(), 2);
        assert_eq!(ast.item_count(), 1);
    }

    #[test]
    fn item_lists_are_ranges_of_the_pool() {
        let mut builder = AstBuilder::new();
        let one = leaf(&mut builder, 1, 0);
        let two = leaf(&mut builder, 2, 2);
        let items: Vec<ItemId> = [one, two]
            .into_iter()
            .map(|value| {
                builder.alloc_item(Item {
                    kind: ItemKind::Expr(value),
                    span: builder.get(value).span,
                })
            })
            .collect();
        let inner = builder.push_item_list(&items[1..]);
        let outer = builder.push_item_list(&items);
        let empty = builder.push_item_list(&[]);
        let ast = builder.finish(outer, Names::new(), Strings::new());
        assert_eq!(ast.items(inner), &items[1..]);
        assert_eq!(ast.items(outer), &items[..]);
        assert!(ast.items(empty).is_empty());
        assert_eq!(ast.root_span(), Span::new(0, 3));
        let mut builder = AstBuilder::new();
        let empty = builder.push_item_list(&[]);
        let ast = builder.finish(empty, Names::new(), Strings::new());
        assert_eq!(ast.root_span(), Span::new(0, 0));
    }

    #[test]
    fn pretty_prints_nested_s_expressions() {
        let mut builder = AstBuilder::new();
        let one = leaf(&mut builder, 1, 1);
        let two = builder.alloc(Expr {
            kind: ExprKind::Float(2.0),
            span: Span::new(5, 8),
        });
        let sum = builder.alloc(Expr {
            kind: ExprKind::Binary {
                op: BinaryOp::Add,
                op_start: 3,
                lhs: one,
                rhs: two,
            },
            span: Span::new(1, 8),
        });
        assert_eq!(builder.get(sum).operator_span(), Some(Span::new(3, 4)));
        let paren = builder.alloc(Expr {
            kind: ExprKind::Paren(sum),
            span: Span::new(0, 9),
        });
        let neg = builder.alloc(Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Neg,
                operand: paren,
            },
            span: Span::new(0, 9),
        });
        assert_eq!(builder.get(neg).operator_span(), Some(Span::new(0, 1)));
        assert_eq!(builder.get(paren).operator_span(), None);
        let ast = finish_expr(builder, neg);
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n  (expr @0..9\n    (neg @0..9\n      (paren @0..9\n        (+ @1..8\n          (int 1 @1..2)\n          (float 2.0 @5..8))))))"
        );
    }

    #[test]
    fn pretty_prints_items_blocks_names_and_types() {
        // let x: Float = { y := z : Int; = 1 }
        // (The text is only for orientation; spans are made up.)
        let mut names = Names::new();
        let (x, y, z, int, float) = (
            names.intern("x"),
            names.intern("y"),
            names.intern("z"),
            names.intern("Int"),
            names.intern("Float"),
        );
        let mut builder = AstBuilder::new();
        let z_use = builder.alloc(Expr {
            kind: ExprKind::Name(z),
            span: Span::new(23, 24),
        });
        let int_ty = builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Named(int),
            span: Span::new(27, 30),
        });
        let ascribed = builder.alloc(Expr {
            kind: ExprKind::Ascribe {
                expr: z_use,
                ty: int_ty,
            },
            span: Span::new(23, 30),
        });
        let rebind = builder.alloc_item(Item {
            kind: ItemKind::Rebind {
                name: y,
                name_span: Span::new(18, 19),
                value: ascribed,
            },
            span: Span::new(18, 30),
        });
        let one = leaf(&mut builder, 1, 34);
        let result = builder.alloc_item(Item {
            kind: ItemKind::Result(one),
            span: Span::new(32, 35),
        });
        let body = builder.push_item_list(&[rebind, result]);
        let block = builder.alloc(Expr {
            kind: ExprKind::Block(body),
            span: Span::new(16, 37),
        });
        let float_ty = builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Named(float),
            span: Span::new(7, 12),
        });
        let with_type = builder.alloc_item(Item {
            kind: ItemKind::Let {
                name: x,
                name_span: Span::new(4, 5),
                ty: Some(float_ty),
                value: block,
            },
            span: Span::new(0, 37),
        });
        let empty_body = builder.push_item_list(&[]);
        let empty = builder.alloc(Expr {
            kind: ExprKind::Block(empty_body),
            span: Span::new(46, 48),
        });
        let without_type = builder.alloc_item(Item {
            kind: ItemKind::Let {
                name: y,
                name_span: Span::new(42, 43),
                ty: None,
                value: empty,
            },
            span: Span::new(38, 48),
        });
        let root = builder.push_item_list(&[with_type, without_type]);
        let ast = builder.finish(root, names, Strings::new());
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n\
             \x20 (let x : Float @0..37\n\
             \x20   (block @16..37\n\
             \x20     (rebind y @18..30\n\
             \x20       (: Int @23..30\n\
             \x20         (name z @23..24)))\n\
             \x20     (result @32..35\n\
             \x20       (int 1 @34..35))))\n\
             \x20 (let y @38..48\n\
             \x20   (block @46..48)))"
        );
    }

    #[test]
    fn empty_file_prints_without_items() {
        let mut builder = AstBuilder::new();
        let root = builder.push_item_list(&[]);
        let ast = builder.finish(root, Names::new(), Strings::new());
        assert_eq!(ast.pretty().to_string(), "(file)");
    }

    #[test]
    fn verbs_and_symbols() {
        assert_eq!(BinaryOp::Add.verb(), "add");
        assert_eq!(BinaryOp::Sub.verb(), "subtract");
        assert_eq!(BinaryOp::Mul.verb(), "multiply");
        assert_eq!(BinaryOp::Div.verb(), "divide");
        assert_eq!(BinaryOp::FloorDiv.verb(), "floor-divide");
        assert_eq!(BinaryOp::Rem.verb(), "take modulo of");
        assert_eq!(UnaryOp::Neg.verb(), "negate");
        assert_eq!(BinaryOp::FloorDiv.symbol(), "//");
        assert_eq!(BinaryOp::Rem.symbol(), "%");
        assert_eq!(UnaryOp::Neg.symbol(), "-");
        assert_eq!(BinaryOp::FloorDiv.span_at(4), Span::new(4, 6));
        assert_eq!(UnaryOp::Neg.span_at(4), Span::new(4, 5));
        for (op, symbol) in [
            (BinaryOp::Eq, "=="),
            (BinaryOp::Ne, "!="),
            (BinaryOp::Lt, "<"),
            (BinaryOp::Le, "<="),
            (BinaryOp::Gt, ">"),
            (BinaryOp::Ge, ">="),
        ] {
            assert_eq!(op.symbol(), symbol);
            assert_eq!(op.verb(), "compare");
            assert!(op.is_comparison());
        }
        assert_eq!(BinaryOp::And.verb(), "and");
        assert_eq!(BinaryOp::Or.verb(), "or");
        assert_eq!(UnaryOp::Not.verb(), "not");
        assert_eq!(BinaryOp::And.symbol(), "and");
        assert_eq!(BinaryOp::Or.symbol(), "or");
        assert_eq!(UnaryOp::Not.symbol(), "not");
        assert_eq!(BinaryOp::Or.span_at(4), Span::new(4, 6));
        assert_eq!(UnaryOp::Not.span_at(4), Span::new(4, 7));
        for op in [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::FloorDiv,
            BinaryOp::Rem,
            BinaryOp::And,
            BinaryOp::Or,
        ] {
            assert!(!op.is_comparison(), "{op:?}");
        }
    }

    #[test]
    fn nodes_with_strings_and_ifs_print_and_resolve() {
        // if c { "a{x}" } else { "b" }, spans made up.
        let mut names = Names::new();
        let (c, x) = (names.intern("c"), names.intern("x"));
        let mut strings = Strings::new();
        let (a, empty, b) = (strings.push("a\n"), strings.push(""), strings.push("\"b\""));
        let mut builder = AstBuilder::new();
        let cond = builder.alloc(Expr {
            kind: ExprKind::Name(c),
            span: Span::new(3, 4),
        });
        let x_use = builder.alloc(Expr {
            kind: ExprKind::Name(x),
            span: Span::new(10, 11),
        });
        let parts =
            builder.push_parts(&[StrPart::Lit(a), StrPart::Expr(x_use), StrPart::Lit(empty)]);
        let interp = builder.alloc(Expr {
            kind: ExprKind::Interp(parts),
            span: Span::new(7, 13),
        });
        let then_item = builder.alloc_item(Item {
            kind: ItemKind::Expr(interp),
            span: Span::new(7, 13),
        });
        let then_body = builder.push_item_list(&[then_item]);
        let then = builder.alloc(Expr {
            kind: ExprKind::Block(then_body),
            span: Span::new(5, 15),
        });
        let lit = builder.alloc(Expr {
            kind: ExprKind::Str(b),
            span: Span::new(23, 26),
        });
        let else_item = builder.alloc_item(Item {
            kind: ItemKind::Expr(lit),
            span: Span::new(23, 26),
        });
        let else_body = builder.push_item_list(&[else_item]);
        let else_ = builder.alloc(Expr {
            kind: ExprKind::Block(else_body),
            span: Span::new(21, 28),
        });
        let if_ = builder.alloc(Expr {
            kind: ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            },
            span: Span::new(0, 28),
        });
        assert_eq!(builder.get(if_).keyword_span(), Some(Span::new(0, 2)));
        assert_eq!(builder.get(if_).operator_span(), None);
        assert_eq!(builder.get(cond).keyword_span(), None);
        let truth = builder.alloc(Expr {
            kind: ExprKind::Bool(true),
            span: Span::new(33, 37),
        });
        let not = builder.alloc(Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Not,
                operand: truth,
            },
            span: Span::new(29, 37),
        });
        let items = [if_, not].map(|value| {
            builder.alloc_item(Item {
                kind: ItemKind::Expr(value),
                span: builder.get(value).span,
            })
        });
        let root = builder.push_item_list(&items);
        let ast = builder.finish(root, names, strings);
        assert_eq!(
            ast.parts(parts),
            &[StrPart::Lit(a), StrPart::Expr(x_use), StrPart::Lit(empty)]
        );
        assert_eq!(ast.strings().get(b), "\"b\"");
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n\
             \x20 (expr @0..28\n\
             \x20   (if @0..28\n\
             \x20     (name c @3..4)\n\
             \x20     (block @5..15\n\
             \x20       (expr @7..13\n\
             \x20         (interp @7..13\n\
             \x20           (lit \"a\\n\")\n\
             \x20           (name x @10..11)\n\
             \x20           (lit \"\"))))\n\
             \x20     (block @21..28\n\
             \x20       (expr @23..26\n\
             \x20         (str \"\\\"b\\\"\" @23..26)))))\n\
             \x20 (expr @29..37\n\
             \x20   (not @29..37\n\
             \x20     (bool true @33..37))))"
        );
    }
}
