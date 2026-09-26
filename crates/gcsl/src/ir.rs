//! The intermediate representation: a lowered, fully typed tree.
//!
//! Like the AST, the IR is an arena of nodes linked by `u32` ids. Unlike the
//! AST, every node is specialised by type: there is an `IntAdd` and a
//! `FloatAdd` rather than one `Add`, and implicit conversions are explicit
//! `IntToFloat` nodes. The evaluator therefore never inspects a runtime value
//! to decide what to do; the node it is evaluating already says.
//!
//! Variables are resolved to frame [`Slot`]s: a [`IrKind::Load`] reads one
//! and a [`IrKind::Store`] writes one. A [`IrKind::Block`] runs its items for
//! their effect and then yields its result; the items of every block live
//! in one pool, [`Ir::items`], as the AST's do. A block always has a result
//! node: lowering gives a block without one an explicit [`IrKind::Unit`], so
//! a block's type is always its result's.
//!
//! An [`Ir`] can only be built by [`crate::lower::lower`], which is what
//! guarantees it is well typed.

use std::fmt;

use crate::check::Slot;
use crate::names::{Name, Names};
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

/// A list of nodes (a block's items): a contiguous range of the item pool of
/// the [`Ir`] that holds it. Resolve it with [`Ir::items`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrRange {
    start: u32,
    len: u32,
}

/// The operands of a binary node, with the position of its operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operands {
    /// The left operand.
    pub lhs: IrId,
    /// The right operand.
    pub rhs: IrId,
    /// Byte offset of the operator's symbol, for runtime diagnostics. The
    /// symbol's length is known from the node's kind (see
    /// [`IrKind::symbol`]), which keeps [`IrNode`] small.
    pub op_start: u32,
}

/// What an [`IrNode`] computes. Operand ids of `Int*` nodes always refer to
/// Int-typed nodes, and of `Float*` nodes (except [`IrKind::IntToFloat`]) to
/// Float-typed nodes; see [`Ir::ty_of`].
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
    IntAdd(Operands),
    /// Checked `Int` subtraction.
    IntSub(Operands),
    /// Checked `Int` multiplication.
    IntMul(Operands),
    /// Checked `Int` division rounding toward negative infinity.
    IntFloorDiv(Operands),
    /// `Int` remainder with the sign of the divisor, matching `IntFloorDiv`.
    IntRem(Operands),
    /// `Float` addition.
    FloatAdd(Operands),
    /// `Float` subtraction.
    FloatSub(Operands),
    /// `Float` multiplication.
    FloatMul(Operands),
    /// `Float` division.
    FloatDiv(Operands),
    /// Converts an `Int` operand to the nearest `Float`. Inserted by lowering
    /// for concrete `Int` operands of `/`.
    IntToFloat(IrId),
    /// The `Unit` value `()`.
    Unit,
    /// Reads a variable's slot. Carries the variable's type so that the
    /// node's type is known without looking anything up.
    Load(Slot, Ty),
    /// Evaluates the node and writes its value to the slot: a `let` or a
    /// `:=`. The store itself has type `Unit`.
    Store(Slot, IrId),
    /// Evaluates each of `items` for its effect, then yields `result`.
    Block {
        /// The items evaluated before the result.
        items: IrRange,
        /// The block's value: its result expression, or a [`IrKind::Unit`]
        /// node if it has none.
        result: IrId,
    },
}

impl IrKind {
    /// The source symbol of an operator node, or `None` for every other
    /// node, including the implicit [`IrKind::IntToFloat`].
    #[must_use]
    pub const fn symbol(self) -> Option<&'static str> {
        match self {
            IrKind::IntNeg(_) | IrKind::FloatNeg(_) | IrKind::IntSub(_) | IrKind::FloatSub(_) => {
                Some("-")
            }
            IrKind::IntAdd(_) | IrKind::FloatAdd(_) => Some("+"),
            IrKind::IntMul(_) | IrKind::FloatMul(_) => Some("*"),
            IrKind::FloatDiv(_) => Some("/"),
            IrKind::IntFloorDiv(_) => Some("//"),
            IrKind::IntRem(_) => Some("%"),
            IrKind::IntConst(_)
            | IrKind::FloatConst(_)
            | IrKind::IntToFloat(_)
            | IrKind::Unit
            | IrKind::Load(..)
            | IrKind::Store(..)
            | IrKind::Block { .. } => None,
        }
    }

    /// The node's name in [`Ir::pretty`] output.
    const fn name(self) -> &'static str {
        match self {
            IrKind::IntConst(_) => "int",
            IrKind::FloatConst(_) => "float",
            IrKind::IntNeg(_) => "int-neg",
            IrKind::FloatNeg(_) => "float-neg",
            IrKind::IntAdd(_) => "int-add",
            IrKind::IntSub(_) => "int-sub",
            IrKind::IntMul(_) => "int-mul",
            IrKind::IntFloorDiv(_) => "int-floor-div",
            IrKind::IntRem(_) => "int-rem",
            IrKind::FloatAdd(_) => "float-add",
            IrKind::FloatSub(_) => "float-sub",
            IrKind::FloatMul(_) => "float-mul",
            IrKind::FloatDiv(_) => "float-div",
            IrKind::IntToFloat(_) => "int-to-float",
            IrKind::Unit => "unit",
            IrKind::Load(..) => "load",
            IrKind::Store(..) => "store",
            IrKind::Block { .. } => "block",
        }
    }
}

/// One IR node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrNode {
    /// What the node computes.
    pub kind: IrKind,
    /// The source it was lowered from, parentheses excluded: an expression,
    /// or for a [`IrKind::Store`], its whole `let` or `:=` item.
    pub span: Span,
}

impl IrNode {
    /// The span of the operator's symbol, for operator nodes: where a
    /// runtime error in this node is reported.
    #[must_use]
    pub fn operator_span(&self) -> Option<Span> {
        let symbol = self.kind.symbol()?;
        let start = match self.kind {
            IrKind::IntAdd(operands)
            | IrKind::IntSub(operands)
            | IrKind::IntMul(operands)
            | IrKind::IntFloorDiv(operands)
            | IrKind::IntRem(operands)
            | IrKind::FloatAdd(operands)
            | IrKind::FloatSub(operands)
            | IrKind::FloatMul(operands)
            | IrKind::FloatDiv(operands) => operands.op_start,
            // A prefix operator starts its expression.
            IrKind::IntNeg(_) | IrKind::FloatNeg(_) => self.span.start,
            IrKind::IntConst(_)
            | IrKind::FloatConst(_)
            | IrKind::IntToFloat(_)
            | IrKind::Unit
            | IrKind::Load(..)
            | IrKind::Store(..)
            | IrKind::Block { .. } => return None,
        };
        Some(crate::ast::symbol_span(symbol, start))
    }
}

/// A lowered program: an arena of [`IrNode`]s, the item pool, its root, and
/// the frame it needs.
#[derive(Clone, Debug)]
pub struct Ir {
    nodes: Vec<IrNode>,
    item_lists: Vec<IrId>,
    root: IrId,
    frame_size: u32,
    /// For each slot, the name of the last variable assigned to it. For
    /// [`Ir::pretty`] only.
    slot_names: Vec<Name>,
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

    /// Returns the nodes of the list `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this IR.
    #[must_use]
    pub fn items(&self, range: IrRange) -> &[IrId] {
        let start = range.start as usize;
        &self.item_lists[start..start + range.len as usize]
    }

    /// The root node, the file's body, whose value is the program's result.
    #[must_use]
    pub fn root(&self) -> IrId {
        self.root
    }

    /// The number of variable slots evaluation needs.
    #[must_use]
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// The type of the value node `id` produces. Never [`Ty::UntypedInt`] or
    /// [`Ty::Error`].
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn ty_of(&self, id: IrId) -> Ty {
        let mut id = id;
        // A block's type is its result's; follow nested blocks by looping,
        // not recursing.
        loop {
            return match self.get(id).kind {
                IrKind::Block { result, .. } => {
                    id = result;
                    continue;
                }
                IrKind::IntConst(_)
                | IrKind::IntNeg(_)
                | IrKind::IntAdd(_)
                | IrKind::IntSub(_)
                | IrKind::IntMul(_)
                | IrKind::IntFloorDiv(_)
                | IrKind::IntRem(_) => Ty::Int,
                IrKind::FloatConst(_)
                | IrKind::FloatNeg(_)
                | IrKind::FloatAdd(_)
                | IrKind::FloatSub(_)
                | IrKind::FloatMul(_)
                | IrKind::FloatDiv(_)
                | IrKind::IntToFloat(_) => Ty::Float,
                IrKind::Unit | IrKind::Store(..) => Ty::Unit,
                IrKind::Load(_, ty) => ty,
            };
        }
    }

    /// Returns a `Display` rendering of the IR as indented S-expressions, for
    /// `--dump-ir`, in the same style as [`crate::ast::Ast::pretty`].
    /// Variables are shown by name and slot, as in `(load x#3 @4..5)`, using
    /// `names`, which must be the table the program was interned into. A
    /// block shows its items and then its result, which is always present.
    /// There is no trailing newline.
    #[must_use]
    pub fn pretty<'a>(&'a self, names: &'a Names) -> impl fmt::Display + 'a {
        Pretty { ir: self, names }
    }
}

/// Builds an [`Ir`] children-first. Crate-private so that only lowering can
/// produce IR.
pub(crate) struct IrBuilder {
    nodes: Vec<IrNode>,
    item_lists: Vec<IrId>,
}

impl IrBuilder {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            item_lists: Vec::new(),
        }
    }

    /// Adds a node and returns its id. The builder holds at most one node
    /// per AST node or item, plus one conversion per `/` operand and one
    /// `Unit` per block, far below `u32::MAX` for any source a `SourceFile`
    /// accepts; past that, ids would saturate.
    pub(crate) fn alloc(&mut self, kind: IrKind, span: Span) -> IrId {
        let id = IrId(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        self.nodes.push(IrNode { kind, span });
        id
    }

    /// Appends `items` to the item pool as one list and returns its range.
    pub(crate) fn push_item_list(&mut self, items: &[IrId]) -> IrRange {
        let start = u32::try_from(self.item_lists.len()).unwrap_or(u32::MAX);
        self.item_lists.extend_from_slice(items);
        IrRange {
            start,
            len: u32::try_from(items.len()).unwrap_or(u32::MAX),
        }
    }

    pub(crate) fn finish(self, root: IrId, frame_size: u32, slot_names: Vec<Name>) -> Ir {
        Ir {
            nodes: self.nodes,
            item_lists: self.item_lists,
            root,
            frame_size,
            slot_names,
        }
    }
}

struct Pretty<'a> {
    ir: &'a Ir,
    names: &'a Names,
}

impl Pretty<'_> {
    /// Writes a slot as `name#slot`, or `?#slot` if the IR has no name for
    /// it.
    fn slot(&self, f: &mut fmt::Formatter<'_>, slot: Slot) -> fmt::Result {
        let name = self
            .ir
            .slot_names
            .get(slot.index())
            .map_or("?", |&name| self.names.get(name));
        write!(f, " {name}{slot}")
    }

    fn node(&self, f: &mut fmt::Formatter<'_>, id: IrId, depth: usize) -> fmt::Result {
        let node = self.ir.get(id);
        write!(f, "{:indent$}({}", "", node.kind.name(), indent = depth * 2)?;
        match node.kind {
            IrKind::IntConst(value) => write!(f, " {value}")?,
            IrKind::FloatConst(value) => write!(f, " {value:?}")?,
            IrKind::Load(slot, _) | IrKind::Store(slot, _) => self.slot(f, slot)?,
            _ => {}
        }
        write!(f, " @{}", node.span)?;
        match node.kind {
            IrKind::IntConst(_) | IrKind::FloatConst(_) | IrKind::Unit | IrKind::Load(..) => {}
            IrKind::IntNeg(operand)
            | IrKind::FloatNeg(operand)
            | IrKind::IntToFloat(operand)
            | IrKind::Store(_, operand) => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            IrKind::IntAdd(operands)
            | IrKind::IntSub(operands)
            | IrKind::IntMul(operands)
            | IrKind::IntFloorDiv(operands)
            | IrKind::IntRem(operands)
            | IrKind::FloatAdd(operands)
            | IrKind::FloatSub(operands)
            | IrKind::FloatMul(operands)
            | IrKind::FloatDiv(operands) => {
                f.write_str("\n")?;
                self.node(f, operands.lhs, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, operands.rhs, depth + 1)?;
            }
            IrKind::Block { items, result } => {
                for &item in self.ir.items(items) {
                    f.write_str("\n")?;
                    self.node(f, item, depth + 1)?;
                }
                f.write_str("\n")?;
                self.node(f, result, depth + 1)?;
            }
        }
        f.write_str(")")
    }
}

impl fmt::Display for Pretty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node(f, self.ir.root, 0)
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
        let div = builder.alloc(
            IrKind::FloatDiv(Operands {
                lhs: widened,
                rhs: two,
                op_start: 2,
            }),
            Span::new(0, 5),
        );
        let ir = builder.finish(div, 0, Vec::new());
        assert_eq!(ir.root(), div);
        assert_eq!(ir.get(two).kind, IrKind::FloatConst(2.0));
        assert_eq!(
            ir.pretty(&Names::new()).to_string(),
            "(float-div @0..5\n  (int-to-float @0..1\n    (int 7 @0..1))\n  (float 2.0 @4..5))"
        );
    }

    #[test]
    fn pretty_prints_blocks_and_slots() {
        let mut names = Names::new();
        let x = names.intern("x");
        let mut builder = IrBuilder::with_capacity(6);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(8, 9));
        let store = builder.alloc(IrKind::Store(Slot::new(0), one), Span::new(0, 9));
        let load = builder.alloc(IrKind::Load(Slot::new(0), Ty::Int), Span::new(10, 11));
        let items = builder.push_item_list(&[store]);
        let block = builder.alloc(
            IrKind::Block {
                items,
                result: load,
            },
            Span::new(0, 11),
        );
        let unit = builder.alloc(IrKind::Unit, Span::new(12, 14));
        let empty = builder.push_item_list(&[]);
        let empty_block = builder.alloc(
            IrKind::Block {
                items: empty,
                result: unit,
            },
            Span::new(12, 14),
        );
        let outer_items = builder.push_item_list(&[empty_block]);
        let root = builder.alloc(
            IrKind::Block {
                items: outer_items,
                result: block,
            },
            Span::new(0, 14),
        );
        let ir = builder.finish(root, 2, vec![x]);
        assert_eq!(ir.frame_size(), 2);
        assert_eq!(ir.items(outer_items), &[empty_block]);
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(block @0..14\n\
             \x20 (block @12..14\n\
             \x20   (unit @12..14))\n\
             \x20 (block @0..11\n\
             \x20   (store x#0 @0..9\n\
             \x20     (int 1 @8..9))\n\
             \x20   (load x#0 @10..11)))"
        );
        // A slot the IR has no name for.
        let mut builder = IrBuilder::with_capacity(1);
        let load = builder.alloc(IrKind::Load(Slot::new(3), Ty::Int), Span::new(0, 1));
        let ir = builder.finish(load, 4, Vec::new());
        assert_eq!(ir.pretty(&names).to_string(), "(load ?#3 @0..1)");
    }

    #[test]
    fn operator_spans() {
        let operands = Operands {
            lhs: IrId(0),
            rhs: IrId(1),
            op_start: 4,
        };
        let node = |kind| IrNode {
            kind,
            span: Span::new(2, 9),
        };
        assert_eq!(
            node(IrKind::IntFloorDiv(operands)).operator_span(),
            Some(Span::new(4, 6))
        );
        assert_eq!(
            node(IrKind::FloatAdd(operands)).operator_span(),
            Some(Span::new(4, 5))
        );
        assert_eq!(
            node(IrKind::IntNeg(IrId(0))).operator_span(),
            Some(Span::new(2, 3))
        );
        assert_eq!(node(IrKind::IntConst(1)).operator_span(), None);
        assert_eq!(node(IrKind::IntToFloat(IrId(0))).operator_span(), None);
        assert_eq!(
            node(IrKind::Load(Slot::new(0), Ty::Int)).operator_span(),
            None
        );
        assert_eq!(
            node(IrKind::Store(Slot::new(0), IrId(0))).operator_span(),
            None
        );
    }

    #[test]
    fn node_types() {
        let mut builder = IrBuilder::with_capacity(8);
        let int = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
        let rem = builder.alloc(
            IrKind::IntRem(Operands {
                lhs: int,
                rhs: int,
                op_start: 0,
            }),
            Span::new(0, 1),
        );
        let float = builder.alloc(IrKind::IntToFloat(int), Span::new(0, 1));
        let unit = builder.alloc(IrKind::Unit, Span::new(0, 1));
        let store = builder.alloc(IrKind::Store(Slot::new(0), int), Span::new(0, 1));
        let load = builder.alloc(IrKind::Load(Slot::new(0), Ty::Float), Span::new(0, 1));
        let items = builder.push_item_list(&[store]);
        let inner = builder.alloc(
            IrKind::Block {
                items,
                result: float,
            },
            Span::new(0, 1),
        );
        let outer = builder.alloc(
            IrKind::Block {
                items,
                result: inner,
            },
            Span::new(0, 1),
        );
        let ir = builder.finish(outer, 1, Vec::new());
        assert_eq!(ir.ty_of(int), Ty::Int);
        assert_eq!(ir.ty_of(rem), Ty::Int);
        assert_eq!(ir.ty_of(float), Ty::Float);
        assert_eq!(ir.ty_of(unit), Ty::Unit);
        assert_eq!(ir.ty_of(store), Ty::Unit);
        assert_eq!(ir.ty_of(load), Ty::Float);
        assert_eq!(ir.ty_of(inner), Ty::Float);
        assert_eq!(ir.ty_of(outer), Ty::Float);
    }
}
