//! The intermediate representation: a lowered, fully typed tree.
//!
//! Like the AST, the IR is an arena of nodes linked by `u32` ids. Unlike the
//! AST, every node is specialised by type: there is an `IntAdd` and a
//! `FloatAdd` rather than one `Add`, and implicit conversions are explicit
//! `IntToFloat` nodes. The evaluator therefore never inspects a runtime value
//! to decide what to do; the node it is evaluating already says.
//!
//! An [`Ir`] can only be built by [`crate::lower::lower`], which is what
//! guarantees it is well typed.

use std::fmt;

use crate::source::Span;
use crate::types::Ty;

/// Index of an [`IrNode`] in its [`Ir`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IrId(u32);

impl IrId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What an [`IrNode`] computes. Operand ids of `Int*` nodes always refer to
/// Int-typed nodes, and of `Float*` nodes (except [`IrKind::IntToFloat`]) to
/// Float-typed nodes; see [`IrKind::ty`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IrKind {
    /// An `Int` constant.
    IntConst(i64),
    /// A `Float` constant.
    FloatConst(f64),
    /// Checked `Int` negation.
    IntNeg(IrId),
    /// `Float` negation.
    FloatNeg(IrId),
    /// Checked `Int` addition.
    IntAdd(IrId, IrId),
    /// Checked `Int` subtraction.
    IntSub(IrId, IrId),
    /// Checked `Int` multiplication.
    IntMul(IrId, IrId),
    /// Checked `Int` division rounding toward negative infinity.
    IntFloorDiv(IrId, IrId),
    /// `Int` remainder with the sign of the divisor, matching `IntFloorDiv`.
    IntRem(IrId, IrId),
    /// `Float` addition.
    FloatAdd(IrId, IrId),
    /// `Float` subtraction.
    FloatSub(IrId, IrId),
    /// `Float` multiplication.
    FloatMul(IrId, IrId),
    /// `Float` division.
    FloatDiv(IrId, IrId),
    /// Converts an `Int` operand to the nearest `Float`. Inserted by lowering
    /// for concrete `Int` operands of `/`.
    IntToFloat(IrId),
}

impl IrKind {
    /// The type of the value this node produces. Never
    /// [`Ty::UntypedInt`].
    #[must_use]
    pub const fn ty(self) -> Ty {
        match self {
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(..)
            | IrKind::IntSub(..)
            | IrKind::IntMul(..)
            | IrKind::IntFloorDiv(..)
            | IrKind::IntRem(..) => Ty::Int,
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(..)
            | IrKind::FloatSub(..)
            | IrKind::FloatMul(..)
            | IrKind::FloatDiv(..)
            | IrKind::IntToFloat(_) => Ty::Float,
        }
    }

    /// The node's name in [`Ir::pretty`] output.
    const fn name(self) -> &'static str {
        match self {
            IrKind::IntConst(_) => "int",
            IrKind::FloatConst(_) => "float",
            IrKind::IntNeg(_) => "int-neg",
            IrKind::FloatNeg(_) => "float-neg",
            IrKind::IntAdd(..) => "int-add",
            IrKind::IntSub(..) => "int-sub",
            IrKind::IntMul(..) => "int-mul",
            IrKind::IntFloorDiv(..) => "int-floor-div",
            IrKind::IntRem(..) => "int-rem",
            IrKind::FloatAdd(..) => "float-add",
            IrKind::FloatSub(..) => "float-sub",
            IrKind::FloatMul(..) => "float-mul",
            IrKind::FloatDiv(..) => "float-div",
            IrKind::IntToFloat(_) => "int-to-float",
        }
    }
}

/// One IR node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrNode {
    /// What the node computes.
    pub kind: IrKind,
    /// The source it was lowered from. Runtime errors are reported here.
    pub span: Span,
}

/// A lowered program: an arena of [`IrNode`]s and its root.
#[derive(Clone, Debug)]
pub struct Ir {
    nodes: Vec<IrNode>,
    root: IrId,
}

impl Ir {
    /// Returns the node `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn get(&self, id: IrId) -> &IrNode {
        &self.nodes[id.index()]
    }

    /// The root node, whose value is the program's result.
    #[must_use]
    pub fn root(&self) -> IrId {
        self.root
    }

    /// Returns a `Display` rendering of the IR as indented S-expressions, for
    /// `--dump-ir`, in the same style as [`crate::ast::Ast::pretty`]. There
    /// is no trailing newline.
    #[must_use]
    pub fn pretty(&self) -> impl fmt::Display + '_ {
        Pretty(self)
    }
}

/// Builds an [`Ir`] children-first. Crate-private so that only lowering can
/// produce IR.
pub(crate) struct IrBuilder {
    nodes: Vec<IrNode>,
}

impl IrBuilder {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
        }
    }

    /// Adds a node and returns its id. The builder holds at most one node
    /// per AST node plus one conversion per `/` operand, far below
    /// `u32::MAX` for any source a `SourceFile` accepts; past that, ids
    /// would saturate.
    pub(crate) fn alloc(&mut self, kind: IrKind, span: Span) -> IrId {
        let id = IrId(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        self.nodes.push(IrNode { kind, span });
        id
    }

    pub(crate) fn finish(self, root: IrId) -> Ir {
        Ir {
            nodes: self.nodes,
            root,
        }
    }
}

struct Pretty<'a>(&'a Ir);

impl Pretty<'_> {
    fn node(&self, f: &mut fmt::Formatter<'_>, id: IrId, depth: usize) -> fmt::Result {
        let node = self.0.get(id);
        write!(f, "{:indent$}({}", "", node.kind.name(), indent = depth * 2)?;
        match node.kind {
            IrKind::IntConst(value) => write!(f, " {value}")?,
            IrKind::FloatConst(value) => write!(f, " {value:?}")?,
            _ => {}
        }
        write!(f, " @{}", node.span)?;
        match node.kind {
            IrKind::IntConst(_) | IrKind::FloatConst(_) => {}
            IrKind::IntNeg(operand) | IrKind::FloatNeg(operand) | IrKind::IntToFloat(operand) => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            IrKind::IntAdd(lhs, rhs)
            | IrKind::IntSub(lhs, rhs)
            | IrKind::IntMul(lhs, rhs)
            | IrKind::IntFloorDiv(lhs, rhs)
            | IrKind::IntRem(lhs, rhs)
            | IrKind::FloatAdd(lhs, rhs)
            | IrKind::FloatSub(lhs, rhs)
            | IrKind::FloatMul(lhs, rhs)
            | IrKind::FloatDiv(lhs, rhs) => {
                f.write_str("\n")?;
                self.node(f, lhs, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, rhs, depth + 1)?;
            }
        }
        f.write_str(")")
    }
}

impl fmt::Display for Pretty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node(f, self.0.root, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_small() {
        // Guards the arena's density; grow this deliberately, not by accident.
        assert!(size_of::<IrNode>() <= 24);
    }

    #[test]
    fn pretty_prints_nested_s_expressions() {
        let mut builder = IrBuilder::with_capacity(4);
        let seven = builder.alloc(IrKind::IntConst(7), Span::new(0, 1));
        let widened = builder.alloc(IrKind::IntToFloat(seven), Span::new(0, 1));
        let two = builder.alloc(IrKind::FloatConst(2.0), Span::new(4, 5));
        let div = builder.alloc(IrKind::FloatDiv(widened, two), Span::new(0, 5));
        let ir = builder.finish(div);
        assert_eq!(ir.root(), div);
        assert_eq!(ir.get(two).kind, IrKind::FloatConst(2.0));
        assert_eq!(
            ir.pretty().to_string(),
            "(float-div @0..5\n  (int-to-float @0..1\n    (int 7 @0..1))\n  (float 2.0 @4..5))"
        );
    }

    #[test]
    fn node_types() {
        let id = IrId(0);
        assert_eq!(IrKind::IntConst(1).ty(), Ty::Int);
        assert_eq!(IrKind::IntRem(id, id).ty(), Ty::Int);
        assert_eq!(IrKind::FloatDiv(id, id).ty(), Ty::Float);
        assert_eq!(IrKind::IntToFloat(id).ty(), Ty::Float);
    }
}
