//! The abstract syntax tree, stored in an arena.
//!
//! Nodes live contiguously in one `Vec<Expr>` and refer to each other by
//! [`ExprId`], a `u32` index. Compared with boxed recursive enums this keeps
//! the tree in one allocation, makes nodes small and `Copy`-friendly, and lets
//! later passes keep per-node data in side tables indexed by `ExprId` (see
//! [`crate::check::Types`]).
//!
//! An [`Ast`] is built through an [`AstBuilder`]: nodes are allocated
//! children-first, and the builder is finished once the root is known, so an
//! `Ast` always has a valid root.

use std::fmt;

use crate::source::Span;

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

/// A prefix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `-x`
    Neg,
}

impl UnaryOp {
    /// The verb used in diagnostics, e.g. "cannot negate ...".
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            UnaryOp::Neg => "negate",
        }
    }

    /// The operator as written in source.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
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
}

impl BinaryOp {
    /// The verb used in diagnostics, e.g. "cannot add Int and Float" or
    /// "cannot take modulo of Float".
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            BinaryOp::Add => "add",
            BinaryOp::Sub => "subtract",
            BinaryOp::Mul => "multiply",
            BinaryOp::Div => "divide",
            BinaryOp::FloorDiv => "floor-divide",
            BinaryOp::Rem => "take modulo of",
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
        }
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
}

/// One node of the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expr {
    /// What the node is.
    pub kind: ExprKind,
    /// The source it covers. A `Paren` covers its parentheses, a `Binary` its
    /// left operand through its right operand, a `Unary` its operator through
    /// its operand.
    pub span: Span,
}

impl Expr {
    /// The span of the operator's symbol, for operator nodes.
    #[must_use]
    pub const fn operator_span(&self) -> Option<Span> {
        match self.kind {
            ExprKind::Unary { op, .. } => Some(op.span_at(self.span.start)),
            ExprKind::Binary { op, op_start, .. } => Some(op.span_at(op_start)),
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Paren(_) => None,
        }
    }
}

/// An arena under construction. Allocate children before their parents, then
/// call [`AstBuilder::finish`] with the root.
#[derive(Clone, Debug, Default)]
pub struct AstBuilder {
    exprs: Vec<Expr>,
}

impl AstBuilder {
    /// Creates an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty builder with room for `capacity` nodes.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            exprs: Vec::with_capacity(capacity),
        }
    }

    /// Adds `expr` to the arena and returns its id.
    ///
    /// An arena holds at most `u32::MAX` nodes. Every node consumes at least
    /// one byte of source and sources are limited to `u32::MAX` bytes, so a
    /// parser can never reach the limit; past it, ids would saturate.
    pub fn alloc(&mut self, expr: Expr) -> ExprId {
        let id = ExprId(u32::try_from(self.exprs.len()).unwrap_or(u32::MAX));
        self.exprs.push(expr);
        id
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

    /// Finishes the arena with `root` as the root expression.
    #[must_use]
    pub fn finish(self, root: ExprId) -> Ast {
        Ast {
            exprs: self.exprs,
            root,
        }
    }
}

/// A complete expression tree: an arena of [`Expr`] nodes and its root.
#[derive(Clone, Debug)]
pub struct Ast {
    exprs: Vec<Expr>,
    root: ExprId,
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

    /// The root expression.
    #[must_use]
    pub fn root(&self) -> ExprId {
        self.root
    }

    /// Number of nodes in the arena, which bounds every [`ExprId`] in it.
    pub(crate) fn len(&self) -> usize {
        self.exprs.len()
    }

    /// Returns a `Display` rendering of the tree as indented S-expressions,
    /// for `--dump-ast`.
    #[must_use]
    pub fn pretty(&self) -> Pretty<'_, NoAnnotation> {
        self.pretty_with(&NoAnnotation)
    }

    /// Like [`Ast::pretty`], with `annotation` written after each node's
    /// head, e.g. its type for `--dump-types`.
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
/// (+ @0..9
///   (int 1 @0..1)
///   (* @4..9
///     (int 2 @4..5)
///     (int 3 @8..9)))
/// ```
///
/// There is no trailing newline.
pub struct Pretty<'a, A> {
    ast: &'a Ast,
    annotation: &'a A,
}

impl<A: Annotate> Pretty<'_, A> {
    fn node(&self, f: &mut fmt::Formatter<'_>, id: ExprId, depth: usize) -> fmt::Result {
        let expr = self.ast.get(id);
        write!(f, "{:indent$}(", "", indent = depth * 2)?;
        match expr.kind {
            ExprKind::Int(value) => write!(f, "int {value}")?,
            ExprKind::Float(value) => write!(f, "float {value:?}")?,
            ExprKind::Unary { op, .. } => match op {
                UnaryOp::Neg => f.write_str("neg")?,
            },
            ExprKind::Binary { op, .. } => f.write_str(op.symbol())?,
            ExprKind::Paren(_) => f.write_str("paren")?,
        }
        self.annotation.annotate(id, f)?;
        write!(f, " @{}", expr.span)?;
        match expr.kind {
            ExprKind::Int(_) | ExprKind::Float(_) => {}
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                f.write_str("\n")?;
                self.node(f, lhs, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, rhs, depth + 1)?;
            }
        }
        f.write_str(")")
    }
}

impl<A: Annotate> fmt::Display for Pretty<'_, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node(f, self.ast.root, 0)
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
        let ast = builder.finish(b);
        assert_eq!(ast.root(), b);
        assert_eq!(ast.len(), 2);
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
        let ast = builder.finish(neg);
        assert_eq!(
            ast.pretty().to_string(),
            "(neg @0..9\n  (paren @0..9\n    (+ @1..8\n      (int 1 @1..2)\n      (float 2.0 @5..8))))"
        );
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
    }
}
