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
//! node: lowering gives a block without one an explicit [`IrKind::Empty`],
//! and a record block a [`IrKind::MakeRecord`], so a block's value is always
//! its result's. Likewise an [`IrKind::If`] always has both branches, of
//! the same kind.
//!
//! The IR knows only the *kind* of value each node produces, a
//! [`ValueKind`], not its full type: that is all evaluation needs to pick
//! the walker for a node, and two record types differ only in what the
//! checker already verified. See [`Ir::kind_of`].
//!
//! String constants live in [`Ir::string`]'s table of reference-counted
//! strings, one per literal piece of the program, so evaluating a
//! [`IrKind::StrConst`] is a reference-count increment, never a copy. An
//! [`IrKind::Interp`] lists its literal pieces and expressions in the item
//! pool. Record layouts, the field names of each record block in definition
//! order, live in [`Ir::layout`]'s table, shared by reference count with
//! every record built from them.
//!
//! A functional update, `let a.b = e`, a dotted `:=` or a dotted field,
//! lowers to a chain of [`IrKind::UpdateField`] nodes, one for each segment
//! of its path after the first, from the leaf outwards: each builds a new
//! record from an old one with one field set. When an update adds a field,
//! the evaluator makes a layout for the new record at runtime, and takes the
//! added name's text from [`Ir::name_text`], so that it never needs the
//! names table.
//!
//! Typed equality, `==` and `!=` on records, compares only the fields of the
//! operands' static type, so its node carries that type's [`Shape`]: the
//! names of its fields, with a shape of its own for each field that is a
//! record, in [`Ir::shape`]'s table. Total equality, `===` and `!==`,
//! compares whole values and needs none.
//!
//! # Functions
//!
//! Every function expression of the program has an entry in the function
//! table, [`Ir::function`], indexed by the [`FnId`] the parser gave it: its
//! body, a [`IrKind::Block`] in the same node arena as the rest of the
//! program, and the sizes of its frame, parameters and captures. Each
//! function runs in a frame of its own, so a [`IrKind::Load`] or
//! [`IrKind::Store`] in a body is of a slot of that function's frame, whose
//! first slots are its parameters. A [`IrKind::LoadCapture`] reads one of
//! the values the closure being run captured, and a [`IrKind::LoadCallee`]
//! yields that closure itself, which is how a function refers to itself
//! without capturing itself.
//!
//! A function expression lowers to a [`IrKind::MakeClosure`], whose items
//! are loads, in the frame creating the closure, of the values it captures,
//! in capture order. A call is a [`IrKind::Call`] whose items are
//! [`IrKind::Arg`] nodes in source order, each naming the parameter its
//! value is passed as.
//!
//! Calls nest on the Rust stack, so the evaluator keeps them within a
//! budget of nesting levels, [`crate::MAX_DEPTH`], which they share with
//! expressions (see [`crate::CALL_LEVELS`]). For it, every
//! [`IrKind::Call`] records its depth in the body of the frame it is
//! evaluated in, the body node being at depth 0, and every function the
//! height of its body, [`FnCode::height`]: both counted in nodes, through
//! the children that `IrKind::children` lists. Lowering computes them once,
//! after building the nodes; the evaluator only adds and compares.
//!
//! An `Ir` is shared, by reference count, between the evaluator and every
//! closure made from its code, and records which session input it was
//! lowered from, [`Ir::input`], so that a runtime error in a function of an
//! earlier input can be shown against that input's source.
//!
//! An [`Ir`] can only be built by [`crate::lower::lower`], which is what
//! guarantees it is well typed.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::FnId;
use crate::check::Slot;
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::{FieldName, Quoted};
use crate::value::{Layout, ValueKind};

/// Index of an [`IrNode`] in its [`Ir`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IrId(u32);

impl IrId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A list of nodes (a block's items, or an interpolation's pieces and
/// expressions): a contiguous range of the item pool of the [`Ir`] that
/// holds it. Resolve it with [`Ir::items`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrRange {
    start: u32,
    len: u32,
}

impl IrRange {
    /// The nodes of this list in `pool`, the item pool holding it.
    ///
    /// # Panics
    ///
    /// Panics if the list does not belong to `pool`.
    fn resolve(self, pool: &[IrId]) -> &[IrId] {
        let start = self.start as usize;
        &pool[start..start + self.len as usize]
    }
}

/// Index of a string constant in its [`Ir`]'s table. Resolve it with
/// [`Ir::string`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstId(u32);

impl ConstId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a record layout in its [`Ir`]'s table. Resolve it with
/// [`Ir::layout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutId(u32);

impl LayoutId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a record shape in its [`Ir`]'s table. Resolve it with
/// [`Ir::shape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeId(u32);

impl ShapeId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The static shape of a record type, which typed equality compares over:
/// the name of each of its fields, sorted by [`Name`], with `Some` shape for
/// each field that is itself a record, which is compared through that
/// shape, and `None` for each field compared as a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Each field's name and, for a record field, its own shape.
    pub fields: Box<[(Name, Option<ShapeId>)]>,
}

/// The relation a comparison node tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cmp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl Cmp {
    /// The operator as written in source.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }

    /// Whether `lhs` and `rhs` are in this relation, by `PartialOrd`: for
    /// floats that is IEEE 754 (every relation but `!=` is false when either
    /// is NaN, and `-0.0 == 0.0`), and for strings it is bytewise.
    #[must_use]
    pub fn test<T: PartialOrd + ?Sized>(self, lhs: &T, rhs: &T) -> bool {
        match self {
            Cmp::Eq => lhs == rhs,
            Cmp::Ne => lhs != rhs,
            Cmp::Lt => lhs < rhs,
            Cmp::Le => lhs <= rhs,
            Cmp::Gt => lhs > rhs,
            Cmp::Ge => lhs >= rhs,
        }
    }
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

/// The operands of a typed record comparison, with the shape it compares
/// over. Unlike [`Operands`], it has no operator position: a shape and a
/// position would not both fit in a node (see [`IrNode`]), and a record
/// comparison never fails, so it never needs to point at its operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapedOperands {
    /// The shape of the operands' static type.
    pub shape: ShapeId,
    /// The left operand.
    pub lhs: IrId,
    /// The right operand.
    pub rhs: IrId,
}

/// What an [`IrNode`] computes. Operand ids of `Int*` nodes always refer to
/// Int-typed nodes, of `Float*` nodes (except [`IrKind::IntToFloat`]) to
/// Float-typed nodes, of `Str*` nodes to String-typed nodes, of `Bool*`,
/// `And`, `Or` and `Not` nodes and an `If`'s condition to Bool-typed nodes,
/// and of `Record*`, `Field` and `UpdateField` nodes to record nodes (an
/// `UpdateField`'s value excepted, which may be of any kind), and a `Call`'s
/// callee is a function node; see [`Ir::kind_of`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IrKind {
    /// An `Int` constant.
    IntConst(i64),
    /// A `Float` constant.
    FloatConst(f64),
    /// A `Bool` constant.
    BoolConst(bool),
    /// A `String` constant: an index into the IR's string table.
    StrConst(ConstId),
    /// `String` concatenation.
    StrConcat(Operands),
    /// Compares two `Int`s.
    IntCmp(Cmp, Operands),
    /// Compares two `Float`s, as IEEE 754 does.
    FloatCmp(Cmp, Operands),
    /// Compares two `String`s, bytewise.
    StrCmp(Cmp, Operands),
    /// Compares two `Bool`s: `==` if the flag is `true`, `!=` if `false`.
    BoolEq(bool, Operands),
    /// `==` on two records of the same type: typed equality, comparing only
    /// the fields of their shape, recursively through the shapes of record
    /// fields.
    RecordEq(ShapedOperands),
    /// `!=` on two records of the same type: the negation of
    /// [`IrKind::RecordEq`].
    RecordNe(ShapedOperands),
    /// `===` on two records of the same type: total equality, comparing
    /// whole values deeply and by field name (see
    /// [`crate::value::Record`]'s equality).
    RecordSame(Operands),
    /// `!==` on two records of the same type: the negation of
    /// [`IrKind::RecordSame`].
    RecordNotSame(Operands),
    /// `Bool` conjunction, evaluating the right operand only if the left is
    /// `true`.
    And(Operands),
    /// `Bool` disjunction, evaluating the right operand only if the left is
    /// `false`.
    Or(Operands),
    /// `Bool` negation.
    Not(IrId),
    /// Evaluates the `Bool` condition, then exactly one branch. Both
    /// branches have the `If`'s type.
    If {
        /// The condition.
        cond: IrId,
        /// Evaluated when the condition is true.
        then: IrId,
        /// Evaluated when it is false.
        else_: IrId,
    },
    /// Builds a `String` from the items of the range, in order: a
    /// [`IrKind::StrConst`] item is literal text, and any other item is a
    /// value shown as text. Lowering alternates literal pieces and
    /// expressions, starting and ending with a piece.
    Interp(IrRange),
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
    /// The empty record `{}`.
    Empty,
    /// Reads a variable's slot. Carries the kind of the variable's value so
    /// that the node's kind is known without looking anything up.
    Load(Slot, ValueKind),
    /// Evaluates the node and writes its value to the slot: a `let`, a
    /// field or a `:=`. The store itself is `{}`.
    Store(Slot, IrId),
    /// Evaluates each of `items` for its effect, then yields `result`.
    Block {
        /// The items evaluated before the result.
        items: IrRange,
        /// The block's value: its result expression, an [`IrKind::Empty`]
        /// node if it has none, or an [`IrKind::MakeRecord`] if it is a
        /// record block.
        result: IrId,
    },
    /// Builds a record with at least one field from `values`, which are in
    /// the order of `layout`'s names: the value of a record block.
    MakeRecord {
        /// The record's field names.
        layout: LayoutId,
        /// The value of each field, in the layout's order.
        values: IrRange,
    },
    /// Reads field `name` of the record `record` evaluates to, which the
    /// checker guarantees it has.
    Field {
        /// The record.
        record: IrId,
        /// The field's name.
        name: Name,
        /// The kind of the field's value, which is the node's kind.
        kind: ValueKind,
    },
    /// Evaluates `record` and then `value`, and yields a new record: the
    /// record with field `name` set to the value. If the record has the
    /// field, the new one shares its layout with the value replaced;
    /// otherwise it has a new layout with `name` added at the end. The empty
    /// record becomes a record of that one field. The record evaluated is
    /// never changed.
    UpdateField {
        /// The record to update.
        record: IrId,
        /// The field to set.
        name: Name,
        /// Its new value, of any kind.
        value: IrId,
    },
    /// Builds a closure of function `function` of the IR, capturing the
    /// values of `captures`: loads in the frame creating it, one for each
    /// value the function captures, in capture order.
    MakeClosure {
        /// The function.
        function: FnId,
        /// What it captures.
        captures: IrRange,
    },
    /// Calls the function `callee` evaluates to with the arguments `args`,
    /// which are [`IrKind::Arg`] nodes in source order, and yields its
    /// result, of kind `kind`.
    Call {
        /// The function called.
        callee: IrId,
        /// The arguments, in source order.
        args: IrRange,
        /// The kind of the call's value: its function's return type's.
        kind: ValueKind,
        /// The call's depth in the body of the frame it is evaluated in,
        /// whose body node is at depth 0, for the nesting budget (see
        /// [`crate::MAX_DEPTH`]). Lowering sets it once every node is built;
        /// a depth past `u16::MAX` is stored as `u16::MAX`, which changes
        /// nothing, since any depth past `MAX_DEPTH` exceeds the budget.
        depth: u16,
    },
    /// One argument of a [`IrKind::Call`], and only ever among its
    /// arguments: `value`, evaluated in the caller's frame, passed as
    /// parameter number `param`.
    Arg {
        /// The index of the parameter it fills.
        param: u32,
        /// Its value.
        value: IrId,
    },
    /// Reads one of the values the closure being run captured, by its index
    /// in capture order. Carries the value's kind, as a `Load` does.
    LoadCapture(u32, ValueKind),
    /// Yields the closure being run: a function's reference to itself.
    LoadCallee,
}

// A call's depth is a `u16` (see `IrKind::Call`), which keeps `IrNode` at
// 24 bytes. Storing depths past `u16::MAX` as `u16::MAX` is only harmless
// while the budget is within a `u16`.
const _: () = assert!(
    crate::MAX_DEPTH <= u16::MAX as usize,
    "a call's depth is a u16, so MAX_DEPTH must fit in one"
);

impl IrKind {
    /// The children of a node of this kind, in order, which are the nodes
    /// in `pool`, the item pool, of any list it has: an operator's operands,
    /// an `if`'s condition and branches, a block's items and then its
    /// result, a call's callee and then its arguments, an argument's value,
    /// a store's value, a field access's record, an update's record and
    /// value, an interpolation's pieces, a record's values, and a closure's
    /// captures. A closure's function is not a child: its body is evaluated
    /// in a frame of its own. This is the one list of every node's children,
    /// and its match is exhaustive, so a new kind of node cannot be missed:
    /// [`Ir::pretty`] prints the children it gives, and lowering's walk that
    /// computes call depths and body heights follows them, and the two must
    /// agree.
    ///
    /// # Panics
    ///
    /// Panics if a list of this node does not belong to `pool`.
    pub(crate) fn children(self, pool: &[IrId]) -> impl DoubleEndedIterator<Item = IrId> + '_ {
        const NONE: [Option<IrId>; 3] = [None; 3];
        // The child before the list, the list, and the children after it.
        let (head, list, tail): (Option<IrId>, &[IrId], [Option<IrId>; 3]) = match self {
            IrKind::IntConst(_)
            | IrKind::FloatConst(_)
            | IrKind::BoolConst(_)
            | IrKind::StrConst(_)
            | IrKind::Empty
            | IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::LoadCallee => (None, &[], NONE),
            IrKind::IntNeg(operand)
            | IrKind::FloatNeg(operand)
            | IrKind::IntToFloat(operand)
            | IrKind::Not(operand)
            | IrKind::Field {
                record: operand, ..
            }
            | IrKind::Store(_, operand)
            | IrKind::Arg { value: operand, .. } => (None, &[], [Some(operand), None, None]),
            IrKind::IntAdd(Operands { lhs, rhs, .. })
            | IrKind::IntSub(Operands { lhs, rhs, .. })
            | IrKind::IntMul(Operands { lhs, rhs, .. })
            | IrKind::IntFloorDiv(Operands { lhs, rhs, .. })
            | IrKind::IntRem(Operands { lhs, rhs, .. })
            | IrKind::FloatAdd(Operands { lhs, rhs, .. })
            | IrKind::FloatSub(Operands { lhs, rhs, .. })
            | IrKind::FloatMul(Operands { lhs, rhs, .. })
            | IrKind::FloatDiv(Operands { lhs, rhs, .. })
            | IrKind::StrConcat(Operands { lhs, rhs, .. })
            | IrKind::IntCmp(_, Operands { lhs, rhs, .. })
            | IrKind::FloatCmp(_, Operands { lhs, rhs, .. })
            | IrKind::StrCmp(_, Operands { lhs, rhs, .. })
            | IrKind::BoolEq(_, Operands { lhs, rhs, .. })
            | IrKind::RecordSame(Operands { lhs, rhs, .. })
            | IrKind::RecordNotSame(Operands { lhs, rhs, .. })
            | IrKind::And(Operands { lhs, rhs, .. })
            | IrKind::Or(Operands { lhs, rhs, .. })
            | IrKind::RecordEq(ShapedOperands { lhs, rhs, .. })
            | IrKind::RecordNe(ShapedOperands { lhs, rhs, .. })
            | IrKind::UpdateField {
                record: lhs,
                value: rhs,
                ..
            } => (None, &[], [Some(lhs), Some(rhs), None]),
            IrKind::If { cond, then, else_ } => (None, &[], [Some(cond), Some(then), Some(else_)]),
            IrKind::Interp(items)
            | IrKind::MakeRecord { values: items, .. }
            | IrKind::MakeClosure {
                captures: items, ..
            } => (None, items.resolve(pool), NONE),
            IrKind::Block { items, result } => {
                (None, items.resolve(pool), [Some(result), None, None])
            }
            IrKind::Call { callee, args, .. } => (Some(callee), args.resolve(pool), NONE),
        };
        head.into_iter()
            .chain(list.iter().copied())
            .chain(tail.into_iter().flatten())
    }

    /// The source symbol of an operator node, or `None` for every other
    /// node, including the implicit [`IrKind::IntToFloat`].
    #[must_use]
    pub const fn symbol(self) -> Option<&'static str> {
        match self {
            IrKind::IntNeg(_) | IrKind::FloatNeg(_) | IrKind::IntSub(_) | IrKind::FloatSub(_) => {
                Some("-")
            }
            IrKind::IntAdd(_) | IrKind::FloatAdd(_) | IrKind::StrConcat(_) => Some("+"),
            IrKind::IntMul(_) | IrKind::FloatMul(_) => Some("*"),
            IrKind::FloatDiv(_) => Some("/"),
            IrKind::IntFloorDiv(_) => Some("//"),
            IrKind::IntRem(_) => Some("%"),
            IrKind::IntCmp(cmp, _) | IrKind::FloatCmp(cmp, _) | IrKind::StrCmp(cmp, _) => {
                Some(cmp.symbol())
            }
            IrKind::BoolEq(true, _) | IrKind::RecordEq(_) => Some("=="),
            IrKind::BoolEq(false, _) | IrKind::RecordNe(_) => Some("!="),
            IrKind::RecordSame(_) => Some("==="),
            IrKind::RecordNotSame(_) => Some("!=="),
            IrKind::And(_) => Some("and"),
            IrKind::Or(_) => Some("or"),
            IrKind::Not(_) => Some("not"),
            IrKind::IntConst(_)
            | IrKind::FloatConst(_)
            | IrKind::BoolConst(_)
            | IrKind::StrConst(_)
            | IrKind::If { .. }
            | IrKind::Interp(_)
            | IrKind::IntToFloat(_)
            | IrKind::Empty
            | IrKind::Load(..)
            | IrKind::Store(..)
            | IrKind::Block { .. }
            | IrKind::MakeRecord { .. }
            | IrKind::Field { .. }
            | IrKind::UpdateField { .. }
            | IrKind::MakeClosure { .. }
            | IrKind::Call { .. }
            | IrKind::Arg { .. }
            | IrKind::LoadCapture(..)
            | IrKind::LoadCallee => None,
        }
    }

    /// The node's name in [`Ir::pretty`] output.
    const fn name(self) -> &'static str {
        match self {
            IrKind::IntConst(_) => "int",
            IrKind::FloatConst(_) => "float",
            IrKind::BoolConst(_) => "bool",
            IrKind::StrConst(_) => "str",
            IrKind::StrConcat(_) => "str-concat",
            IrKind::IntCmp(..) => "int-cmp",
            IrKind::FloatCmp(..) => "float-cmp",
            IrKind::StrCmp(..) => "str-cmp",
            IrKind::BoolEq(..) => "bool-eq",
            IrKind::RecordEq(_) | IrKind::RecordNe(_) => "record-eq",
            IrKind::RecordSame(_) | IrKind::RecordNotSame(_) => "record-same",
            IrKind::And(_) => "and",
            IrKind::Or(_) => "or",
            IrKind::Not(_) => "not",
            IrKind::If { .. } => "if",
            IrKind::Interp(_) => "interp",
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
            IrKind::Empty => "empty",
            IrKind::Load(..) => "load",
            IrKind::Store(..) => "store",
            IrKind::Block { .. } => "block",
            IrKind::MakeRecord { .. } => "make-record",
            IrKind::Field { .. } => "field",
            IrKind::UpdateField { .. } => "update-field",
            IrKind::MakeClosure { .. } => "make-closure",
            IrKind::Call { .. } => "call",
            IrKind::Arg { .. } => "arg",
            IrKind::LoadCapture(..) => "load-capture",
            IrKind::LoadCallee => "load-callee",
        }
    }
}

/// One IR node: 24 bytes, a 16-byte kind and an 8-byte span. Every kind's
/// payload therefore fits in 15 bytes beside its tag, which is why a typed
/// record comparison has its shape but not its operator's position (see
/// [`ShapedOperands`]).
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
    /// runtime error in this node is reported. `None` for typed record
    /// comparisons, which cannot fail and do not keep it (see
    /// [`ShapedOperands`]).
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
            | IrKind::FloatDiv(operands)
            | IrKind::StrConcat(operands)
            | IrKind::IntCmp(_, operands)
            | IrKind::FloatCmp(_, operands)
            | IrKind::StrCmp(_, operands)
            | IrKind::BoolEq(_, operands)
            | IrKind::RecordSame(operands)
            | IrKind::RecordNotSame(operands)
            | IrKind::And(operands)
            | IrKind::Or(operands) => operands.op_start,
            // A prefix operator starts its expression.
            IrKind::IntNeg(_) | IrKind::FloatNeg(_) | IrKind::Not(_) => self.span.start,
            IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::IntConst(_)
            | IrKind::FloatConst(_)
            | IrKind::BoolConst(_)
            | IrKind::StrConst(_)
            | IrKind::If { .. }
            | IrKind::Interp(_)
            | IrKind::IntToFloat(_)
            | IrKind::Empty
            | IrKind::Load(..)
            | IrKind::Store(..)
            | IrKind::Block { .. }
            | IrKind::MakeRecord { .. }
            | IrKind::Field { .. }
            | IrKind::UpdateField { .. }
            | IrKind::MakeClosure { .. }
            | IrKind::Call { .. }
            | IrKind::Arg { .. }
            | IrKind::LoadCapture(..)
            | IrKind::LoadCallee => return None,
        };
        Some(crate::ast::symbol_span(symbol, start))
    }
}

/// One function of a program, in [`Ir::function`]'s table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnCode {
    /// The body: a [`IrKind::Block`] in the same node arena as the rest of
    /// the program.
    pub body: IrId,
    /// The number of slots the body's frame needs, its parameters included.
    pub frame_size: u32,
    /// The number of parameters, which occupy slots `0..param_count`.
    pub param_count: u32,
    /// The number of values a closure of the function captures.
    pub capture_count: u32,
    /// The height of the body: the number of nodes on the longest path
    /// from the body node to a leaf, a leaf being 1. A call charges it to
    /// the nesting budget up front (see [`crate::MAX_DEPTH`]). Lowering
    /// sets it once every node is built.
    pub height: u32,
    /// The name the function refers to itself by, if it has one: the
    /// `let`, field or named function item it initialises. For
    /// [`Ir::pretty`].
    pub name: Option<Name>,
    /// The function's type as text, `fn(x: Int): Int`: what its closures
    /// display as.
    pub display: Box<str>,
    /// For each slot of the body's frame, the name of the last variable
    /// assigned to it. For [`Ir::pretty`] only.
    pub slot_names: Box<[Name]>,
}

/// A lowered program: an arena of [`IrNode`]s, the item pool, the string
/// constants, the record layouts and shapes, the text of every name an
/// update may add to a record, the functions, its root, the frame it needs,
/// and the session input it was lowered from.
#[derive(Clone, Debug)]
pub struct Ir {
    nodes: Vec<IrNode>,
    item_lists: Vec<IrId>,
    strings: Vec<Rc<str>>,
    layouts: Vec<Rc<Layout>>,
    shapes: Vec<Shape>,
    /// The text of the name of every [`IrKind::UpdateField`], which may
    /// add it to a record's layout at runtime.
    name_texts: HashMap<Name, Box<str>>,
    /// Every function of the program, indexed by [`FnId`].
    functions: Vec<FnCode>,
    root: IrId,
    frame_size: u32,
    /// For each slot of the root's frame, the name of the last variable
    /// assigned to it. For [`Ir::pretty`] only.
    slot_names: Vec<Name>,
    /// The session input the program was lowered from; 0 for a file.
    input: u32,
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
        range.resolve(&self.item_lists)
    }

    /// The children of node `id`, in order: see [`IrKind::children`].
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    pub(crate) fn children(&self, id: IrId) -> impl DoubleEndedIterator<Item = IrId> + '_ {
        self.get(id).kind.children(&self.item_lists)
    }

    /// Every call in the frame whose body is `body`, with its depth and
    /// span, in the order a depth-first walk meets them, for tests. The
    /// calls in the bodies of the functions the frame makes closures of are
    /// not in it.
    #[cfg(test)]
    pub(crate) fn calls_in(&self, body: IrId) -> Vec<(u16, Span)> {
        let mut calls = Vec::new();
        let mut pending = vec![body];
        while let Some(id) = pending.pop() {
            let node = self.get(id);
            if let IrKind::Call { depth, .. } = node.kind {
                calls.push((depth, node.span));
            }
            pending.extend(self.children(id).rev());
        }
        calls
    }

    /// Returns the string constant `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn string(&self, id: ConstId) -> &Rc<str> {
        &self.strings[id.index()]
    }

    /// Returns the record layout `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn layout(&self, id: LayoutId) -> &Rc<Layout> {
        &self.layouts[id.index()]
    }

    /// The number of distinct record layouts.
    #[must_use]
    pub fn layout_count(&self) -> usize {
        self.layouts.len()
    }

    /// Returns the record shape `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn shape(&self, id: ShapeId) -> &Shape {
        &self.shapes[id.index()]
    }

    /// The number of distinct record shapes.
    #[must_use]
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    /// The text of `name`, if it is the name of an [`IrKind::UpdateField`]:
    /// what the evaluator shows a field it adds to a record as.
    #[must_use]
    pub fn name_text(&self, name: Name) -> Option<&str> {
        self.name_texts.get(&name).map(AsRef::as_ref)
    }

    /// The root node, the file's body, whose value is the program's result.
    #[must_use]
    pub fn root(&self) -> IrId {
        self.root
    }

    /// The number of variable slots evaluation of the root needs.
    #[must_use]
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// Returns function `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn function(&self, id: FnId) -> &FnCode {
        &self.functions[id.index()]
    }

    /// Returns function `id`, or `None` if it does not belong to this IR.
    #[must_use]
    pub fn find_function(&self, id: FnId) -> Option<&FnCode> {
        self.functions.get(id.index())
    }

    /// The number of functions in the program, which bounds every [`FnId`]
    /// in it.
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// The session input the program was lowered from, which its spans
    /// refer to: 0 for a file, and the input's number in a
    /// [`crate::session::Session`].
    #[must_use]
    pub fn input(&self) -> u32 {
        self.input
    }

    /// The kind of value node `id` produces.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn kind_of(&self, id: IrId) -> ValueKind {
        let mut id = id;
        // A block's kind is its result's, and an `if`'s its branches'; follow
        // nested ones by looping, not recursing.
        loop {
            return match self.get(id).kind {
                IrKind::Block { result, .. } => {
                    id = result;
                    continue;
                }
                IrKind::If { then, .. } => {
                    id = then;
                    continue;
                }
                IrKind::Arg { value, .. } => {
                    id = value;
                    continue;
                }
                IrKind::BoolConst(_)
                | IrKind::IntCmp(..)
                | IrKind::FloatCmp(..)
                | IrKind::StrCmp(..)
                | IrKind::BoolEq(..)
                | IrKind::RecordEq(_)
                | IrKind::RecordNe(_)
                | IrKind::RecordSame(_)
                | IrKind::RecordNotSame(_)
                | IrKind::And(_)
                | IrKind::Or(_)
                | IrKind::Not(_) => ValueKind::Bool,
                IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => ValueKind::String,
                IrKind::IntConst(_)
                | IrKind::IntNeg(_)
                | IrKind::IntAdd(_)
                | IrKind::IntSub(_)
                | IrKind::IntMul(_)
                | IrKind::IntFloorDiv(_)
                | IrKind::IntRem(_) => ValueKind::Int,
                IrKind::FloatConst(_)
                | IrKind::FloatNeg(_)
                | IrKind::FloatAdd(_)
                | IrKind::FloatSub(_)
                | IrKind::FloatMul(_)
                | IrKind::FloatDiv(_)
                | IrKind::IntToFloat(_) => ValueKind::Float,
                IrKind::Empty
                | IrKind::Store(..)
                | IrKind::MakeRecord { .. }
                | IrKind::UpdateField { .. } => ValueKind::Record,
                IrKind::Load(_, kind)
                | IrKind::Field { kind, .. }
                | IrKind::Call { kind, .. }
                | IrKind::LoadCapture(_, kind) => kind,
                IrKind::MakeClosure { .. } | IrKind::LoadCallee => ValueKind::Fn,
            };
        }
    }

    /// Returns a `Display` rendering of the IR as indented S-expressions, for
    /// `--dump-ir`, in the same style as [`crate::ast::Ast::pretty`].
    /// Variables are shown by name and slot, as in `(load x#3 @4..5)`, using
    /// `names`, which must be the table the program was interned into. A
    /// block shows its items and then its result, which is always present,
    /// and an `if` its condition and both branches. String constants show
    /// their text quoted and escaped, as in `(str "a\n" @0..5)`, and
    /// comparisons their operator, as in `(int-cmp < @0..5 ...)`. A record
    /// shows its layout's names in brackets and then its values, as in
    /// `(make-record [x y] @0..9 ...)`, and a field access or an update its
    /// name, as in `(field x @0..3 ...)` and `(update-field x @0..3 ...)`,
    /// then its record and, for an update, the value. A name that is not a
    /// plain identifier is quoted. A typed record comparison shows its
    /// operator and its shape's field names in the shape's order, each
    /// record field's own shape after its name, as in
    /// `(record-eq == [x inner[n]] @0..9 ...)`; a total one its operator,
    /// as in `(record-same === @0..9 ...)`. A closure shows its function's
    /// number and then what it captures, as in `(make-closure #0 @0..9
    /// ...)`, a call its callee and then its arguments, each as
    /// `(arg 0 @4..5 ...)` with the parameter it fills, and the reads of a
    /// capture and of the callee as `(load-capture 0 @4..5)` and
    /// `(load-callee @4..5)`. A call's depth is not shown. After the root,
    /// each function is shown on a line of its own as
    /// `fn #0 name (frame 3, params 2, captures 1, height 5)`, or
    /// `fn #1 (frame ...)` if it has no name, followed by its body, whose
    /// slots are named by its own frame's variables. There is no trailing
    /// newline.
    #[must_use]
    pub fn pretty<'a>(&'a self, names: &'a Names) -> impl fmt::Display + 'a {
        Pretty {
            ir: self,
            names,
            slot_names: &self.slot_names,
        }
    }
}

/// Builds an [`Ir`] children-first. Crate-private so that only lowering can
/// produce IR.
pub(crate) struct IrBuilder {
    nodes: Vec<IrNode>,
    item_lists: Vec<IrId>,
    strings: Vec<Rc<str>>,
    layouts: Vec<Rc<Layout>>,
    shapes: Vec<Shape>,
    name_texts: HashMap<Name, Box<str>>,
}

impl IrBuilder {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            item_lists: Vec::new(),
            strings: Vec::new(),
            layouts: Vec::new(),
            shapes: Vec::new(),
            name_texts: HashMap::new(),
        }
    }

    /// Adds a node and returns its id. The builder holds at most one node
    /// per AST node or item, plus one conversion per `/` operand, one
    /// `Empty` or `MakeRecord` per block, one `Load` per field, one
    /// constant per literal piece of an interpolation, for each segment of
    /// a dotted path after the first an update and a record to update, one
    /// `Arg` per argument of a call, and one load per value a closure
    /// captures (a variable used inside functions nested `n` deep in the
    /// one declaring it is captured by each of the `n`), far below
    /// `u32::MAX` for any realistic source; past that, ids would saturate.
    pub(crate) fn alloc(&mut self, kind: IrKind, span: Span) -> IrId {
        let id = IrId(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        self.nodes.push(IrNode { kind, span });
        id
    }

    /// Adds a string constant holding a copy of `text` and returns its id.
    /// There is at most one per literal piece, so ids never saturate either.
    pub(crate) fn push_string(&mut self, text: &str) -> ConstId {
        let id = ConstId(u32::try_from(self.strings.len()).unwrap_or(u32::MAX));
        self.strings.push(Rc::from(text));
        id
    }

    /// Adds a record layout and returns its id. There is at most one per
    /// record block, so ids never saturate either. Adding the same names
    /// twice adds two layouts; deduplicating is the caller's business.
    pub(crate) fn push_layout(&mut self, layout: Layout) -> LayoutId {
        let id = LayoutId(u32::try_from(self.layouts.len()).unwrap_or(u32::MAX));
        self.layouts.push(Rc::new(layout));
        id
    }

    /// Adds a record shape and returns its id. There is at most one per
    /// record type, and fewer record types than bytes of source, so ids
    /// never saturate either. Deduplicating is the caller's business.
    pub(crate) fn push_shape(&mut self, shape: Shape) -> ShapeId {
        let id = ShapeId(u32::try_from(self.shapes.len()).unwrap_or(u32::MAX));
        self.shapes.push(shape);
        id
    }

    /// Replaces the fields of shape `id`, which [`IrBuilder::push_shape`]
    /// returned, so that a shape can be given its id before its fields are
    /// known. Does nothing for an id it did not return.
    pub(crate) fn set_shape(&mut self, id: ShapeId, shape: Shape) {
        if let Some(entry) = self.shapes.get_mut(id.index()) {
            *entry = shape;
        }
    }

    /// Records `text` as the text of `name`, the name of an
    /// [`IrKind::UpdateField`], unless it is recorded already.
    pub(crate) fn add_name_text(&mut self, name: Name, text: &str) {
        self.name_texts
            .entry(name)
            .or_insert_with(|| Box::from(text));
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

    /// Finishes a program without functions, lowered from input 0, for
    /// building IR by hand in tests: its calls are given their depths as
    /// lowering gives them.
    #[cfg(test)]
    pub(crate) fn finish(mut self, root: IrId, frame_size: u32, slot_names: Vec<Name>) -> Ir {
        self.measure(root, &mut []);
        self.finish_program(root, (frame_size, slot_names), Vec::new(), 0)
    }

    /// Gives every [`IrKind::Call`] of the program whose root is `root` and
    /// whose functions are `functions` its depth in its frame, and every
    /// function its body's height, [`FnCode::height`]: one walk of the root
    /// and one of each function's body, through [`IrKind::children`]. The
    /// heights of all the nodes are kept in one vector, and the walks share
    /// one stack.
    ///
    /// The nodes of each frame form a tree, apart from the record nodes an
    /// update chain shares between an update and the field access into the
    /// record it updates (see [`crate::lower`]). The walk visits such a node
    /// once, from the first parent it reaches it by, and every other parent
    /// takes its height as computed then.
    pub(crate) fn measure(&mut self, root: IrId, functions: &mut [FnCode]) {
        // A node's height, or 0 while it is unvisited: a height is at
        // least 1.
        let mut heights = vec![0_u32; self.nodes.len()];
        let mut stack = Vec::new();
        self.walk(root, &mut heights, &mut stack);
        for code in functions {
            code.height = self.walk(code.body, &mut heights, &mut stack);
        }
    }

    /// Walks the frame whose body is `body` depth first, giving each call
    /// its depth on the way down and each node its height in `heights` on
    /// the way up, and returns the body's height. Iterative, with `stack`
    /// as the work list, since a body can be [`crate::MAX_DEPTH`] deep: each
    /// entry is a node, its depth, and whether its children have been
    /// pushed, in which case they have all been walked when it is popped
    /// again.
    fn walk(&mut self, body: IrId, heights: &mut [u32], stack: &mut Vec<(IrId, u32, bool)>) -> u32 {
        let Self {
            nodes, item_lists, ..
        } = self;
        stack.push((body, 0, false));
        while let Some((id, depth, entered)) = stack.pop() {
            let (Some(node), Some(&height)) = (nodes.get_mut(id.index()), heights.get(id.index()))
            else {
                continue;
            };
            if entered {
                let tallest = node
                    .kind
                    .children(item_lists)
                    .filter_map(|child| heights.get(child.index()).copied())
                    .max()
                    .unwrap_or(0);
                if let Some(entry) = heights.get_mut(id.index()) {
                    *entry = tallest.saturating_add(1);
                }
            } else if height == 0 {
                if let IrKind::Call {
                    depth: call_depth, ..
                } = &mut node.kind
                {
                    *call_depth = u16::try_from(depth).unwrap_or(u16::MAX);
                }
                stack.push((id, depth, true));
                // Reversed, so that the children are walked in order.
                for child in node.kind.children(item_lists).rev() {
                    stack.push((child, depth.saturating_add(1), false));
                }
            }
        }
        heights.get(body.index()).copied().unwrap_or(0)
    }

    /// Finishes the program whose root is `root`, whose root frame has
    /// `frame_size` slots named by `slot_names`, whose functions are
    /// `functions`, indexed by [`FnId`], and which was lowered from session
    /// input `input`.
    pub(crate) fn finish_program(
        self,
        root: IrId,
        (frame_size, slot_names): (u32, Vec<Name>),
        functions: Vec<FnCode>,
        input: u32,
    ) -> Ir {
        Ir {
            nodes: self.nodes,
            item_lists: self.item_lists,
            strings: self.strings,
            layouts: self.layouts,
            shapes: self.shapes,
            name_texts: self.name_texts,
            functions,
            root,
            frame_size,
            slot_names,
            input,
        }
    }
}

struct Pretty<'a> {
    ir: &'a Ir,
    names: &'a Names,
    /// The names of the slots of the frame being shown: the root's or a
    /// function's.
    slot_names: &'a [Name],
}

impl Pretty<'_> {
    /// Writes shape `id` as its field names in brackets, each record
    /// field's shape after its name: `[x inner[n]]`. Nested shapes are
    /// written through a work list, not recursion, since a record type can
    /// be nested as deeply as a program is long.
    fn shape(&self, f: &mut fmt::Formatter<'_>, id: ShapeId) -> fmt::Result {
        // Each shape being written, with the index of its next field.
        let mut pending = vec![(id, 0_usize)];
        f.write_str("[")?;
        while let Some((shape, index)) = pending.pop() {
            let fields = self.ir.shapes.get(shape.index()).map(|s| &*s.fields);
            let Some(&(name, nested)) = fields.unwrap_or_default().get(index) else {
                f.write_str("]")?;
                continue;
            };
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{}", FieldName(self.names.get(name)))?;
            pending.push((shape, index + 1));
            if let Some(nested) = nested {
                f.write_str("[")?;
                pending.push((nested, 0));
            }
        }
        Ok(())
    }

    /// Writes a slot as `name#slot`, the name quoted if it is a field's
    /// that is not a plain identifier, or `?#slot` if the IR has no name
    /// for it.
    fn slot(&self, f: &mut fmt::Formatter<'_>, slot: Slot) -> fmt::Result {
        match self.slot_names.get(slot.index()) {
            Some(&name) => write!(f, " {}{slot}", FieldName(self.names.get(name))),
            None => write!(f, " ?{slot}"),
        }
    }

    /// Writes what a node shows between its name and its span: a
    /// constant's value, a slot, an operator, a layout, a shape or a name.
    fn details(&self, f: &mut fmt::Formatter<'_>, node: &IrNode) -> fmt::Result {
        match node.kind {
            IrKind::IntConst(value) => write!(f, " {value}")?,
            IrKind::FloatConst(value) => write!(f, " {value:?}")?,
            IrKind::BoolConst(value) => write!(f, " {value}")?,
            IrKind::StrConst(id) => write!(f, " {}", Quoted(self.ir.string(id)))?,
            IrKind::Load(slot, _) | IrKind::Store(slot, _) => self.slot(f, slot)?,
            IrKind::IntCmp(cmp, _) | IrKind::FloatCmp(cmp, _) | IrKind::StrCmp(cmp, _) => {
                write!(f, " {}", cmp.symbol())?;
            }
            IrKind::BoolEq(eq, _) => f.write_str(if eq { " ==" } else { " !=" })?,
            IrKind::RecordEq(operands) | IrKind::RecordNe(operands) => {
                let eq = matches!(node.kind, IrKind::RecordEq(_));
                f.write_str(if eq { " == " } else { " != " })?;
                self.shape(f, operands.shape)?;
            }
            IrKind::RecordSame(_) => f.write_str(" ===")?,
            IrKind::RecordNotSame(_) => f.write_str(" !==")?,
            IrKind::MakeRecord { layout, .. } => {
                f.write_str(" [")?;
                for (index, text) in self.ir.layout(layout).texts.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{}", FieldName(text))?;
                }
                f.write_str("]")?;
            }
            IrKind::Field { name, .. } | IrKind::UpdateField { name, .. } => {
                write!(f, " {}", FieldName(self.names.get(name)))?;
            }
            IrKind::MakeClosure { function, .. } => write!(f, " {function}")?,
            IrKind::Arg { param, .. } | IrKind::LoadCapture(param, _) => write!(f, " {param}")?,
            _ => {}
        }
        Ok(())
    }

    fn node(&self, f: &mut fmt::Formatter<'_>, id: IrId, depth: usize) -> fmt::Result {
        let node = self.ir.get(id);
        write!(f, "{:indent$}({}", "", node.kind.name(), indent = depth * 2)?;
        self.details(f, node)?;
        write!(f, " @{}", node.span)?;
        // The children, each on a line of its own: inline rather than in a
        // helper, so that printing recurses one frame per level.
        for child in self.ir.children(id) {
            f.write_str("\n")?;
            self.node(f, child, depth + 1)?;
        }
        f.write_str(")")
    }
}

impl fmt::Display for Pretty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node(f, self.ir.root, 0)?;
        for (index, code) in self.ir.functions.iter().enumerate() {
            write!(f, "\nfn #{index}")?;
            if let Some(name) = code.name {
                write!(f, " {}", FieldName(self.names.get(name)))?;
            }
            writeln!(
                f,
                " (frame {}, params {}, captures {}, height {})",
                code.frame_size, code.param_count, code.capture_count, code.height
            )?;
            let body = Pretty {
                ir: self.ir,
                names: self.names,
                slot_names: &code.slot_names,
            };
            body.node(f, code.body, 1)?;
        }
        Ok(())
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
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Int),
            Span::new(10, 11),
        );
        let items = builder.push_item_list(&[store]);
        let block = builder.alloc(
            IrKind::Block {
                items,
                result: load,
            },
            Span::new(0, 11),
        );
        let nothing = builder.alloc(IrKind::Empty, Span::new(12, 14));
        let empty = builder.push_item_list(&[]);
        let empty_block = builder.alloc(
            IrKind::Block {
                items: empty,
                result: nothing,
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
             \x20   (empty @12..14))\n\
             \x20 (block @0..11\n\
             \x20   (store x#0 @0..9\n\
             \x20     (int 1 @8..9))\n\
             \x20   (load x#0 @10..11)))"
        );
        // A slot the IR has no name for.
        let mut builder = IrBuilder::with_capacity(1);
        let load = builder.alloc(IrKind::Load(Slot::new(3), ValueKind::Int), Span::new(0, 1));
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
            node(IrKind::Load(Slot::new(0), ValueKind::Int)).operator_span(),
            None
        );
        assert_eq!(
            node(IrKind::Store(Slot::new(0), IrId(0))).operator_span(),
            None
        );
    }

    #[test]
    fn node_kinds() {
        let mut names = Names::new();
        let x = names.intern("x");
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
        let nothing = builder.alloc(IrKind::Empty, Span::new(0, 1));
        let store = builder.alloc(IrKind::Store(Slot::new(0), int), Span::new(0, 1));
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Float),
            Span::new(0, 1),
        );
        let layout = builder.push_layout(Layout {
            names: Box::new([x]),
            texts: Box::new([Box::from("x")]),
        });
        let values = builder.push_item_list(&[load]);
        let record = builder.alloc(IrKind::MakeRecord { layout, values }, Span::new(0, 1));
        let field = builder.alloc(
            IrKind::Field {
                record,
                name: x,
                kind: ValueKind::Float,
            },
            Span::new(0, 1),
        );
        let operands = Operands {
            lhs: record,
            rhs: record,
            op_start: 0,
        };
        let shape = builder.push_shape(Shape {
            fields: Box::new([(x, None)]),
        });
        let shaped = ShapedOperands {
            shape,
            lhs: record,
            rhs: record,
        };
        let record_eq = builder.alloc(IrKind::RecordEq(shaped), Span::new(0, 1));
        let record_ne = builder.alloc(IrKind::RecordNe(shaped), Span::new(0, 1));
        let record_same = builder.alloc(IrKind::RecordSame(operands), Span::new(0, 1));
        let record_not_same = builder.alloc(IrKind::RecordNotSame(operands), Span::new(0, 1));
        let update = builder.alloc(
            IrKind::UpdateField {
                record,
                name: x,
                value: float,
            },
            Span::new(0, 1),
        );
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
        assert_eq!(ir.kind_of(int), ValueKind::Int);
        assert_eq!(ir.kind_of(rem), ValueKind::Int);
        assert_eq!(ir.kind_of(float), ValueKind::Float);
        assert_eq!(ir.kind_of(nothing), ValueKind::Record);
        assert_eq!(ir.kind_of(store), ValueKind::Record);
        assert_eq!(ir.kind_of(load), ValueKind::Float);
        assert_eq!(ir.kind_of(record), ValueKind::Record);
        assert_eq!(ir.kind_of(field), ValueKind::Float);
        for comparison in [record_eq, record_ne, record_same, record_not_same] {
            assert_eq!(ir.kind_of(comparison), ValueKind::Bool);
        }
        assert_eq!(ir.kind_of(update), ValueKind::Record);
        assert_eq!(ir.kind_of(inner), ValueKind::Float);
        assert_eq!(ir.kind_of(outer), ValueKind::Float);
        assert_eq!(ir.layout_count(), 1);
        assert_eq!(&*ir.layout(layout).names, &[x]);
        assert_eq!(ir.shape_count(), 1);
        assert_eq!(&*ir.shape(shape).fields, &[(x, None)]);
    }

    #[test]
    fn records_show_their_layout_and_fields_their_name() {
        let mut names = Names::new();
        let (x, spaced) = (names.intern("x"), names.intern("a b"));
        let mut builder = IrBuilder::with_capacity(5);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(6, 7));
        let two = builder.alloc(IrKind::IntConst(2), Span::new(17, 18));
        let layout = builder.push_layout(Layout {
            names: Box::new([x, spaced]),
            texts: Box::new([Box::from("x"), Box::from("a b")]),
        });
        let values = builder.push_item_list(&[one, two]);
        let record = builder.alloc(IrKind::MakeRecord { layout, values }, Span::new(0, 20));
        let field = builder.alloc(
            IrKind::Field {
                record,
                name: spaced,
                kind: ValueKind::Int,
            },
            Span::new(0, 26),
        );
        let ir = builder.finish(field, 0, Vec::new());
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(field \"a b\" @0..26\n\
             \x20 (make-record [x \"a b\"] @0..20\n\
             \x20   (int 1 @6..7)\n\
             \x20   (int 2 @17..18)))"
        );
        assert_eq!(ir.get(field).operator_span(), None);
        assert_eq!(ir.get(record).kind.symbol(), None);
    }

    #[test]
    fn cmp_follows_partial_ord() {
        let all = [Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge];
        let results = |lhs: f64, rhs: f64| all.map(|cmp| cmp.test(&lhs, &rhs));
        assert_eq!(results(1.0, 2.0), [false, true, true, true, false, false]);
        assert_eq!(results(2.0, 2.0), [true, false, false, true, false, true]);
        assert_eq!(results(-0.0, 0.0), [true, false, false, true, false, true]);
        // NaN is unequal and unordered to everything, itself included.
        assert_eq!(
            results(f64::NAN, f64::NAN),
            [false, true, false, false, false, false]
        );
        assert_eq!(
            results(f64::NAN, 1.0),
            [false, true, false, false, false, false]
        );
        // Strings compare bytewise.
        assert!(Cmp::Lt.test("Z", "a"));
        assert!(Cmp::Lt.test("", "a"));
        assert!(Cmp::Lt.test("ab", "b"));
        assert!(Cmp::Ge.test("é", "z"));
        assert_eq!(all.map(Cmp::symbol), ["==", "!=", "<", "<=", ">", ">="]);
    }

    #[test]
    fn strings_bools_and_ifs() {
        let mut names = Names::new();
        let s = names.intern("s");
        let mut builder = IrBuilder::with_capacity(12);
        let greeting = builder.push_string("hi \"you\"\n");
        let empty = builder.push_string("");
        let piece = builder.alloc(IrKind::StrConst(greeting), Span::new(0, 14));
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::String),
            Span::new(4, 5),
        );
        let tail = builder.alloc(IrKind::StrConst(empty), Span::new(0, 14));
        let pieces = builder.push_item_list(&[piece, load, tail]);
        let interp = builder.alloc(IrKind::Interp(pieces), Span::new(0, 14));
        let truth = builder.alloc(IrKind::BoolConst(true), Span::new(18, 22));
        let not = builder.alloc(IrKind::Not(truth), Span::new(14, 22));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(23, 24));
        let two = builder.alloc(IrKind::IntConst(2), Span::new(27, 28));
        let cmp = builder.alloc(
            IrKind::IntCmp(
                Cmp::Le,
                Operands {
                    lhs: one,
                    rhs: two,
                    op_start: 25,
                },
            ),
            Span::new(23, 28),
        );
        let and = builder.alloc(
            IrKind::And(Operands {
                lhs: not,
                rhs: cmp,
                op_start: 30,
            }),
            Span::new(14, 28),
        );
        let concat = builder.alloc(
            IrKind::StrConcat(Operands {
                lhs: interp,
                rhs: interp,
                op_start: 40,
            }),
            Span::new(35, 45),
        );
        let if_ = builder.alloc(
            IrKind::If {
                cond: and,
                then: interp,
                else_: concat,
            },
            Span::new(0, 45),
        );
        let ir = builder.finish(if_, 1, vec![s]);
        assert_eq!(&**ir.string(greeting), "hi \"you\"\n");
        assert_eq!(&**ir.string(empty), "");
        assert_eq!(ir.kind_of(if_), ValueKind::String);
        assert_eq!(ir.kind_of(and), ValueKind::Bool);
        assert_eq!(ir.kind_of(cmp), ValueKind::Bool);
        assert_eq!(ir.kind_of(not), ValueKind::Bool);
        assert_eq!(ir.kind_of(truth), ValueKind::Bool);
        assert_eq!(ir.kind_of(concat), ValueKind::String);
        assert_eq!(ir.kind_of(interp), ValueKind::String);
        assert_eq!(ir.get(cmp).operator_span(), Some(Span::new(25, 27)));
        assert_eq!(ir.get(and).operator_span(), Some(Span::new(30, 33)));
        assert_eq!(ir.get(not).operator_span(), Some(Span::new(14, 17)));
        assert_eq!(ir.get(concat).operator_span(), Some(Span::new(40, 41)));
        assert_eq!(ir.get(if_).operator_span(), None);
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(if @0..45\n\
             \x20 (and @14..28\n\
             \x20   (not @14..22\n\
             \x20     (bool true @18..22))\n\
             \x20   (int-cmp <= @23..28\n\
             \x20     (int 1 @23..24)\n\
             \x20     (int 2 @27..28)))\n\
             \x20 (interp @0..14\n\
             \x20   (str \"hi \\\"you\\\"\\n\" @0..14)\n\
             \x20   (load s#0 @4..5)\n\
             \x20   (str \"\" @0..14))\n\
             \x20 (str-concat @35..45\n\
             \x20   (interp @0..14\n\
             \x20     (str \"hi \\\"you\\\"\\n\" @0..14)\n\
             \x20     (load s#0 @4..5)\n\
             \x20     (str \"\" @0..14))\n\
             \x20   (interp @0..14\n\
             \x20     (str \"hi \\\"you\\\"\\n\" @0..14)\n\
             \x20     (load s#0 @4..5)\n\
             \x20     (str \"\" @0..14))))"
        );
    }

    #[test]
    fn equality_of_bools_and_records_shows_its_operator() {
        let mut names = Names::new();
        let (x, inner, n, spaced) = (
            names.intern("x"),
            names.intern("inner"),
            names.intern("n"),
            names.intern("a b"),
        );
        let mut builder = IrBuilder::with_capacity(8);
        let nothing = builder.alloc(IrKind::Empty, Span::new(0, 2));
        let operands = Operands {
            lhs: nothing,
            rhs: nothing,
            op_start: 3,
        };
        // A shape given its id before its fields, as lowering does.
        let outer = builder.push_shape(Shape {
            fields: Box::default(),
        });
        let nested = builder.push_shape(Shape {
            fields: Box::new([(n, None), (spaced, None)]),
        });
        builder.set_shape(
            outer,
            Shape {
                fields: Box::new([(x, None), (inner, Some(nested))]),
            },
        );
        let empty = builder.push_shape(Shape {
            fields: Box::default(),
        });
        let shaped = |shape| ShapedOperands {
            shape,
            lhs: nothing,
            rhs: nothing,
        };
        let eq = builder.alloc(IrKind::RecordEq(shaped(outer)), Span::new(0, 8));
        let ne = builder.alloc(IrKind::RecordNe(shaped(empty)), Span::new(0, 8));
        let same = builder.alloc(IrKind::RecordSame(operands), Span::new(0, 9));
        let not_same = builder.alloc(IrKind::RecordNotSame(operands), Span::new(0, 9));
        let bool_ne = builder.alloc(IrKind::BoolEq(false, operands), Span::new(0, 8));
        let items = builder.push_item_list(&[eq, ne, same, not_same]);
        let root = builder.alloc(
            IrKind::Block {
                items,
                result: bool_ne,
            },
            Span::new(0, 9),
        );
        let ir = builder.finish(root, 0, Vec::new());
        let symbols = [eq, ne, same, not_same, bool_ne].map(|id| ir.get(id).kind.symbol());
        assert_eq!(
            symbols,
            [Some("=="), Some("!="), Some("==="), Some("!=="), Some("!=")]
        );
        // A typed record comparison cannot fail, and keeps no position.
        assert_eq!(ir.get(eq).operator_span(), None);
        assert_eq!(ir.get(ne).operator_span(), None);
        assert_eq!(ir.get(same).operator_span(), Some(Span::new(3, 6)));
        assert_eq!(ir.get(not_same).operator_span(), Some(Span::new(3, 6)));
        assert_eq!(ir.get(bool_ne).operator_span(), Some(Span::new(3, 5)));
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(block @0..9\n\
             \x20 (record-eq == [x inner[n \"a b\"]] @0..8\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (record-eq != [] @0..8\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (record-same === @0..9\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (record-same !== @0..9\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (bool-eq != @0..8\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2)))"
        );
    }

    #[test]
    fn updates_show_their_name_record_and_value() {
        let mut names = Names::new();
        let (b, spaced) = (names.intern("b"), names.intern("c d"));
        let mut builder = IrBuilder::with_capacity(5);
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Record),
            Span::new(4, 5),
        );
        let nothing = builder.alloc(IrKind::Empty, Span::new(4, 7));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(16, 17));
        let inner = builder.alloc(
            IrKind::UpdateField {
                record: nothing,
                name: spaced,
                value: one,
            },
            Span::new(4, 13),
        );
        let outer = builder.alloc(
            IrKind::UpdateField {
                record: load,
                name: b,
                value: inner,
            },
            Span::new(4, 7),
        );
        builder.add_name_text(b, "b");
        builder.add_name_text(spaced, "c d");
        // Recording a name twice keeps the first text.
        builder.add_name_text(b, "other");
        let ir = builder.finish(outer, 1, vec![b]);
        assert_eq!(ir.name_text(b), Some("b"));
        assert_eq!(ir.name_text(spaced), Some("c d"));
        assert_eq!(ir.name_text(names.intern("unused")), None);
        assert_eq!(ir.get(outer).kind.symbol(), None);
        assert_eq!(ir.get(outer).operator_span(), None);
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(update-field b @4..7\n\
             \x20 (load b#0 @4..5)\n\
             \x20 (update-field \"c d\" @4..13\n\
             \x20   (empty @4..7)\n\
             \x20   (int 1 @16..17)))"
        );
    }
}
