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
//! A functional update, `let a.b = e` or a dotted `:=`, lowers to one
//! [`IrKind::Update`] node for the whole update: its children are its
//! value, the record or list it updates (the variable's, read and awaited
//! where the checker marked it, or the empty record a `let` builds from) and
//! the positions of its path's index segments, in that order, which is the
//! order they are evaluated in; the steps of its path, one for each segment
//! after the first, are in a side table, [`Ir::update_path`], each an
//! [`UpdateStep`]: a field by name or an element at the next position, and
//! whether what it selects is read, read and awaited, created as the empty
//! record, or, for the last, set. Evaluating it walks the path down once and
//! builds the new value back up (see [`crate::eval`]), so every node of it
//! is evaluated exactly once. A dotted field, which only extends a record
//! its block is building and never awaits or indexes, lowers instead to a
//! chain of [`IrKind::UpdateField`] nodes, one for each segment of its path
//! after the first, from the leaf outwards: each builds a new record from
//! an old one with one field set. When an update of either kind adds a
//! field, the evaluator makes a layout for the new record at runtime, and
//! takes the added name's text from [`Ir::name_text`], so that it never
//! needs the names table.
//!
//! Typed equality, `==` and `!=` on records, compares only the fields of the
//! operands' static type, so its node carries that type's [`Shape`]: the
//! names of its fields, with a shape of its own for each field that is a
//! record or a list, in [`Ir::shape`]'s table. On lists, it compares the
//! elements through the shape of their static type, which a list's shape,
//! a [`ListShape`] ([`Ir::list_shape`]) in the same table, gives. Total
//! equality, `===` and `!==`, compares whole values and needs none.
//!
//! # Lists
//!
//! A list literal lowers to an [`IrKind::MakeList`] of its elements in
//! source order, each a node of any kind, or an [`IrKind::Spread`] of a
//! list node for a spread element; the node says whether it has any spread,
//! so that one without is built at its known length. An index is an
//! [`IrKind::Index`], of the kind of the element read, like a field access.
//! A call of a built-in function whose typing is special lowers to a node
//! of its own, specialised as any operator is: `len`'s to
//! [`IrKind::ListLen`]. An ordinary built-in function (`int`, `float`) is a
//! value of a function type, which a name or a parameter may hold as well
//! as any closure, so a call of it is an [`IrKind::Call`] like any other:
//! no node of its own could be chosen without knowing which function a
//! value is, and the evaluator applies it when its callee turns out to be
//! one (see [`crate::eval`], "Calls"). No node is added for them.
//!
//! A range, `..` or `..=`, is an [`IrKind::Range`] of its two `Int`
//! bounds, a list node. A `for` is an [`IrKind::For`]: the slot
//! of its variable, in the current frame, and its parts, the iterable, the
//! guard if it has one and the body, in the item pool, which are its
//! children, so that its height is one above the tallest of them, as an
//! `if`'s is, and the nesting budget is charged for its body once, however
//! many times it runs. It either collects the body's values into a list or,
//! as a statement whose value nothing would hold, discards them and is
//! `{}`.
//!
//! # Loops and stops
//!
//! A condition loop or a `do` loop is an [`IrKind::Loop`], whose parts are
//! its condition, a `Bool` node (`true` for `for { ... }`), its body and the
//! `Store`s of its header's items after the condition, and which says
//! whether it tests its condition before its first iteration, as all but a
//! `do` loop do; the `Store`s of the items before the condition are the
//! items of an [`IrKind::Block`] around it. It collects or discards as a
//! `For` does. A `break` and a `continue` are an [`IrKind::Break`] and an
//! [`IrKind::Continue`], each with its value, if it has one, a node of any
//! kind, which is its child, and the number of loops it leaves before the
//! one it is for, counted from the innermost loop whose body holds it.
//!
//! A `return` is an [`IrKind::Return`], with its value, if it has one, its
//! child, and the [`CatchId`] it leaves for: [`CatchId::FUNCTION`] for the
//! function or `lazy` around it, and otherwise the number of an
//! [`IrKind::Catch`] around it. A catch wraps a labelled block, or a loop,
//! that some `return` leaves, and only those, so that a block or loop no
//! `return` leaves is the node it always was. Its node is the block or the
//! loop, and its kind that of the whole, which is the join of the block's
//! or loop's own value and what is returned (a loop's list or a value
//! returned to it may differ in kind, so the loop node keeps its own). A
//! `return` cannot name its catch by node, since lowering builds children
//! before their parents; it names it by a number lowering gives each catch
//! as it starts it, once per program.
//!
//! Such a node is of kind [`ValueKind::Never`], and so is a block whose
//! result is one, an `if` both of whose branches are, and a node lowered
//! from an expression of type `Never`; a node of that kind may stand where
//! a node of any kind is expected, and every walker of the evaluator hands
//! a `Break`, `Continue` or `Return` it meets to the one place that unwinds
//! (see [`crate::eval`]). The other nodes of kind `Never`, a load of a
//! variable or a field access of type `Never`, say, are only ever in code
//! that does not run, but for a loop that cannot end, a `for { ... }` no
//! `break` is for, which runs and never yields: its node is a `Loop` that
//! collects nothing, of kind [`ValueKind::Record`] as a node, though what
//! stands where it is may be of any kind, and every walker runs it; and
//! for a call of a function whose result is `Never`, `panic`'s, which runs
//! and always unwinds with an error: a `Call` of kind `Never`, which every
//! walker evaluates as it does any call.
//!
//! # Functions
//!
//! Every function expression of the program has an entry in the function
//! table, [`Ir::function`], indexed by the [`FnId`] the parser gave it: its
//! body, a [`IrKind::Block`] in the same node arena as the rest of the
//! program, the sizes of its frame, parameters and captures, and its
//! [`FnKind`], which says whether calls of its closures are memoised
//! (declared with `->>`) or not (declared with `:` or `->`). Each
//! function runs in a frame of its own, so a [`IrKind::Load`] or
//! [`IrKind::Store`] in a body is of a slot of that function's frame, whose
//! first slots are its parameters. A [`IrKind::LoadCapture`] reads one of
//! the values the closure being run captured, and a [`IrKind::LoadCallee`]
//! yields that closure itself, which is how a function refers to itself
//! without capturing itself. A [`IrKind::Top`] reads a slot of the top
//! level's frame, which is at the bottom of the value stack however deep
//! the call it runs in: it is how a function or `lazy` refers to a
//! top-level function of its own input (see the checker's "Top-level
//! functions"), so that two of them can refer to each other, which copies
//! could not do, and to the cell of a top-level field of its input (see
//! the checker's "Top-level fields"), so that a closure never waits for a
//! field to be computed before it is made. From top-level code the same
//! function or cell is a plain [`IrKind::Load`].
//!
//! The closure of a top-level function is stored into its slot where the
//! checker says it is made ([`crate::check::MadeAt`]): before the input's
//! first item if it copies nothing from the input, just after the item
//! that binds the last of what it copies, or at its own item if it copies a
//! mutable variable. Every other item stays where it is written; a `def`
//! whose closure is stored earlier still has its record field there.
//!
//! A function expression lowers to a [`IrKind::MakeClosure`], whose items
//! are loads, in the frame creating the closure, of the values it captures,
//! in capture order. A call is a [`IrKind::Call`] whose items are
//! [`IrKind::Arg`] nodes in source order, each naming the parameter its
//! value is passed as.
//!
//! A program carries the directory of the file it was lowered from, by its
//! index in its run's table of directories ([`Ir::dir`]), which every call
//! its code makes, a closure's or a future's of it included, passes on as
//! the callee's caller, but after `fileroot.helper()`. The `fileroot`
//! module's members that are not constants are nodes of their own:
//! [`IrKind::Caller`], `fileroot.caller`, a `String` read from the running
//! frame, and [`IrKind::Helper`], `fileroot.helper()`, `{}`, which sets the
//! running frame's helper flag (see [`crate::eval`], "Callers and
//! helpers"); `fileroot.this` and `fileroot.workspace` are
//! [`IrKind::StrConst`]s.
//!
//! Calls nest on the Rust stack, so the evaluator keeps them within a
//! budget of nesting levels, [`crate::MAX_DEPTH`], which they share with
//! expressions (see [`crate::CALL_LEVELS`]). For it, every
//! [`IrKind::Call`], [`IrKind::Await`] and [`IrKind::Update`], whose steps
//! may await, records its depth in the body of the frame it is evaluated
//! in, the body node being at depth 0, and every function the height of
//! its body, [`FnCode::height`]: both counted in nodes, through the
//! children that `IrKind::children` lists, an update counting two levels
//! more for each step of its path. Lowering computes them once, after
//! building the nodes; the evaluator only adds and compares.
//!
//! # Cells
//!
//! Every node of the tree of an input's top-level fields has a *cell*, a
//! slot of the top level's frame written once, at the node's *point*:
//! before the item of the outermost block the checker's schedule names,
//! dependencies first, or after the last item. A leaf's cell holds its
//! initialiser's value; an interior node's, the [`IrKind::MakeRecord`] of
//! its children's cells, in the order of their first items. The record of
//! the input is made from the cells of the roots. [`IrKind::UpdateField`]
//! is used only for the dotted fields of inner blocks: a top-level dotted
//! field never rebuilds its root item by item. A read of a cell is a
//! [`IrKind::Load`] from top-level code and an [`IrKind::Top`] of the
//! cell's kind from a function or `lazy`; the schedule is static, so the
//! store always comes first, and a [`IrKind::Top`] that finds its slot not
//! yet written as a kind other than a record's (whose empty slot is a valid
//! `{}`) is an internal error, not a panic. The dumps show each cell's store
//! where its point is, and a `top` read of it by the cell's name.
//!
//! # Futures
//!
//! A `lazy` is a function of the table too, of [`FnKind::Future`], with no
//! parameters, whose body is its operand, any node. It lowers to a
//! [`IrKind::MakeFuture`], which captures as a `MakeClosure` does and builds
//! a future instead of a closure.
//!
//! A node whose static type is a `Future` or an either, `T | Future<T>`, is
//! of kind [`ValueKind::Deferred`]: its value may be a future or not. Only
//! the evaluator's general walker evaluates such nodes; every typed walker
//! treats one as mistyped. They are loads, captures, field accesses, calls,
//! blocks and `if`s of such types, and `MakeFuture`s. Where the checker
//! found that a value that may be a future is used as its bits, lowering
//! wraps its node in an [`IrKind::Await`], which evaluates it through the
//! general walker, awaits it fully, and yields a value of its own, present
//! kind; so nothing else ever inspects a runtime value to see whether it is
//! a future. An await is *deep* when it also awaits the lazy fields of a
//! record, as the [`LazyShape`] it carries lists them, from
//! [`Ir::lazy_shape`]'s table, or the elements of a list, as a list's lazy
//! shape says to await each ([`Ir::lazy_list`]), in the same table. An
//! await runs a future's body in a frame of
//! its own, so, like a call, it records its depth in its frame's body for
//! the nesting budget.
//!
//! # Unions
//!
//! A node whose static type is a union of members of different kinds,
//! `Int | String`, is of kind [`ValueKind::Mixed`]: its value is present,
//! but of a kind only the value says. As for [`ValueKind::Deferred`], only
//! the evaluator's general walker evaluates such nodes; every typed walker
//! treats one as mistyped. They are loads, captures, field accesses,
//! indexes, calls, awaits, `as`es, blocks and `if`s of such types; where one
//! is expected, as a branch of an `if` of a union type, a node of any kind
//! but `Deferred` is accepted, since a value of any member's kind may be
//! where the union may. A union whose members are all of one kind, lists
//! or records, say, is of that kind, and needs nothing new. Total equality
//! of two `Mixed` values is an [`IrKind::ValueSame`] (or
//! [`IrKind::ValueNotSame`]), which compares them whole. A deep await of a
//! union carries the lazy shape of each member that has one
//! ([`Ir::lazy_union`]).
//!
//! `is` lowers to an [`IrKind::Is`] and `as` to an [`IrKind::As`], each of
//! its operand, awaited where the checker marked it, and of its test, in
//! [`Ir::test`]'s table: the *plan* the checker made of the test (a
//! [`ValueTest`]), which says exactly how far into the value the test
//! looks, and what an `as`'s failure says. The evaluator walks no types.
//! An `is` that narrows an `if`'s branches also writes the value it tests
//! to the slot the branches' narrowed bindings read.
//!
//! An `Ir` is shared, by reference count, between the evaluator and every
//! closure and future made from its code, and records which session input it was
//! lowered from, [`Ir::input`], so that a runtime error in a function of an
//! earlier input can be shown against that input's source.
//!
//! An [`Ir`] can only be built by [`crate::lower::lower`], which is what
//! guarantees it is well typed.

use std::collections::HashMap;
use std::fmt;
use std::mem;
use std::rc::Rc;

use crate::ast::FnId;
use crate::check::Slot;
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::{FieldName, Quoted};
use crate::types::ValueTest;
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
/// each field that is itself a record or a list, which is compared through
/// that shape, and `None` for each field compared as a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Each field's name and, for a record or list field, its own shape.
    pub fields: Box<[(Name, Option<ShapeId>)]>,
}

/// The static shape of a list type, which typed equality compares over:
/// how its elements compare. A list's shape is in the same table as the
/// records', so that a record's field can name it; see [`Ir::list_shape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListShape {
    /// The shape of the element type, if it is a record or a list type,
    /// through which each element is compared; `None` if each is compared
    /// as a value.
    pub element: Option<ShapeId>,
}

/// Index of the path of an [`IrKind::Update`] in its [`Ir`]'s table.
/// Resolve it with [`Ir::update_path`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UpdatePathId(u32);

impl UpdatePathId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The steps of one update's path: a contiguous range of the step pool of
/// the [`Ir`] that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StepRange {
    start: u32,
    len: u32,
}

/// One step of the path of an [`IrKind::Update`], one for each segment of
/// the source path after the first: what it selects in the record or list
/// the step before it reached, the variable's value for the first, and
/// what it does with it. Every step but the last goes down a level; the
/// last sets the leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UpdateStep {
    /// What the step selects.
    pub at: StepAt,
    /// What it does with it.
    pub mode: StepMode,
    /// The path up to and including the step's segment, as written: where
    /// an error at the step is reported.
    pub span: Span,
}

/// What an [`UpdateStep`] selects in the record or list before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepAt {
    /// The field of this name, of a record.
    Field(Name),
    /// An element of a list, at the position that the next of the update's
    /// positions evaluates to (see [`IrKind::Update`]).
    Element,
}

/// What an [`UpdateStep`] does with what it selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepMode {
    /// Reads it, a present record or list, for the next step to update.
    Read,
    /// Reads it, a record or list that may be a future, and awaits it fully,
    /// as an [`IrKind::Await`] does, for the next step to update.
    Await,
    /// Ignores it, being a field the static type of the record does not
    /// have, and starts the next step from the empty record, as a dotted
    /// `let` creates a record where its path goes beyond the static type.
    Create,
    /// Replaces it, or adds the field, with the update's value: the last
    /// step, and only the last.
    Set,
}

/// Index of a lazy shape in its [`Ir`]'s table, or [`LazyShapeId::NONE`].
/// Resolve it with [`Ir::lazy_shape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LazyShapeId(u32);

impl LazyShapeId {
    /// No shape: nothing below the top of the value to await.
    pub const NONE: LazyShapeId = LazyShapeId(u32::MAX);

    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What a deep [`IrKind::Await`] awaits in a record: each field, by name,
/// that is lazy itself or holds a record or list with lazy values somewhere
/// inside. The fields are sorted by [`Name`], as a record type's are. A
/// list's lazy shape is in the same table, with no fields, and says what is
/// done with each element instead: see [`Ir::lazy_list`]. So is a union's,
/// which lists the lazy shapes of its members that have one instead: see
/// [`Ir::lazy_union`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LazyShape {
    /// Each such field's name and what is done with its value.
    pub fields: Box<[(Name, LazyField)]>,
}

/// What a deep await does with one field of a record, or with each element
/// of a list (see [`LazyShape`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LazyField {
    /// The field or element is a future or an either: its value is awaited
    /// fully and then, unless the shape is [`LazyShapeId::NONE`], what is
    /// lazy inside it is awaited as the shape says.
    Await(LazyShapeId),
    /// The field or element is a present record or list with lazy values
    /// somewhere inside: they are awaited as the shape says.
    Within(LazyShapeId),
}

/// What a `return` leaves, named by number: a [`IrKind::Catch`] of the
/// same number, around a labelled block or a loop, or for
/// [`CatchId::FUNCTION`], the function or `lazy` being run. Lowering
/// numbers the catches of a program from 0, each once, so no two catches
/// of one IR share a number (see [`crate::lower`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CatchId(u32);

impl CatchId {
    /// The function or `lazy` whose body is being run: what an unlabelled
    /// `return` leaves, caught by the call, the memoised call or the await
    /// running it.
    pub const FUNCTION: CatchId = CatchId(u32::MAX);

    /// The catch numbered `number`.
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The catch's number, as a stop carries it (see [`crate::eval`]).
    pub(crate) const fn number(self) -> u32 {
        self.0
    }
}

impl fmt::Display for CatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == CatchId::FUNCTION {
            f.write_str("fn")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// Index of a test of an `is` or an `as` in its [`Ir`]'s table. Resolve it
/// with [`Ir::test`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TestId(u32);

impl TestId {
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The test an [`IrKind::Is`] or [`IrKind::As`] runs, as the checker
/// planned it, with what an `as` says when its test fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCode {
    /// The plan of the test.
    pub test: ValueTest,
    /// How a failure names the value tested: "`x`" or "this value".
    pub subject: Box<str>,
    /// How a failure names the type tested for, with its article: "an
    /// Int".
    pub target: Box<str>,
}

/// Whether a function of the table is a function expression, memoised or
/// not, or a `lazy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FnKind {
    /// A function expression declared with `:` or `->`, made into closures
    /// by `MakeClosure`, whose body runs on every call.
    Function,
    /// A function expression declared with `->>`, made into closures by
    /// `MakeClosure`, each of whose closures runs its body at most once per
    /// distinct list of arguments: see [`crate::eval`].
    Memo,
    /// The body of a `lazy`, made into futures by `MakeFuture`.
    Future,
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
/// `And`, `Or` and `Not` nodes, an `If`'s condition, a `For`'s guard and a
/// `Loop`'s condition to Bool-typed nodes, and of `Record*`, `Field` and
/// `UpdateField` nodes
/// to record nodes (an `UpdateField`'s value excepted, which may be of any
/// kind), of `List*` and `Spread` nodes, an `Index`'s list and a `For`'s
/// iterable to list nodes, an `Index`'s position, an `Update`'s positions
/// and a `Range`'s bounds are Int nodes, an `Update`'s record or list is
/// of the update's kind, and a `Call`'s
/// callee is a function node; see [`Ir::kind_of`]. No typed operand is ever
/// of kind [`ValueKind::Deferred`]: an operand that may be a future is an
/// [`IrKind::Await`] of it; nor of kind [`ValueKind::Mixed`]: an operand of
/// a union is narrowed first, by an `as`, or in the branch of an `if` whose
/// `is` narrows it. Any operand may instead be of kind [`ValueKind::Never`],
/// which never yields a value (see [Loops and
/// stops](crate::ir#loops-and-stops)).
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
    /// `==` on two lists of the same type: typed equality, comparing their
    /// lengths and their elements in order, each through the shape of the
    /// element type, as the list's shape (see [`ListShape`]) says.
    ListEq(ShapedOperands),
    /// `!=` on two lists of the same type: the negation of
    /// [`IrKind::ListEq`].
    ListNe(ShapedOperands),
    /// `===` on two lists of the same type: total equality, comparing whole
    /// values deeply (see [`crate::value::List`]'s equality).
    ListSame(Operands),
    /// `!==` on two lists of the same type: the negation of
    /// [`IrKind::ListSame`].
    ListNotSame(Operands),
    /// `===` on two values of the same union type of members of different
    /// kinds, nodes of kind [`ValueKind::Mixed`]: total equality, comparing
    /// whole values deeply, as [`Value`](crate::value::Value)'s equality
    /// does.
    ValueSame(Operands),
    /// `!==` on two such values: the negation of [`IrKind::ValueSame`].
    ValueNotSame(Operands),
    /// `is`: evaluates `value`, a node of any kind but
    /// [`ValueKind::Deferred`], and yields whether it passes the test
    /// `test`, a `Bool`; first, unless `slot` is [`Slot::NONE`], writes the
    /// value to `slot` of the current frame, where the narrowed bindings of
    /// the branches of the `if` it is the condition of read it.
    Is {
        /// The value tested.
        value: IrId,
        /// The test.
        test: TestId,
        /// The slot the value is written to, or [`Slot::NONE`].
        slot: Slot,
    },
    /// `as`: evaluates `value`, a node of any kind but
    /// [`ValueKind::Deferred`], and yields it, of kind `kind`, if it passes
    /// the test `test`; otherwise fails with "`x` is not an Int".
    As {
        /// The value narrowed.
        value: IrId,
        /// The test.
        test: TestId,
        /// The kind of the narrowed type, and so of the node.
        kind: ValueKind,
    },
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
    /// A dotted `let` or `:=`: evaluates its value, the first of `parts`, a
    /// node of any kind; then the record or list it updates, the second, a
    /// node of kind `kind`; then walks `path` down from it, one step at a
    /// time, evaluating the position of each step that selects an element,
    /// the rest of `parts`, in order, `Int` nodes, where it reaches it; and
    /// finally builds the new record or list, of kind `kind`, from the leaf
    /// outwards: each record or list along the path with what the step
    /// after it selects replaced or, for a field, added. Nothing evaluated
    /// is ever changed. A position outside its list is a runtime error.
    Update {
        /// The value, the record or list updated, and each position.
        parts: IrRange,
        /// The steps of the path after its first segment.
        path: UpdatePathId,
        /// The kind of the record or list updated, and so of the node:
        /// [`ValueKind::Record`] or [`ValueKind::List`].
        kind: ValueKind,
        /// The update's depth in the body of the frame it is evaluated in,
        /// for the nesting budget of the awaits of its steps, as an
        /// [`IrKind::Await`]'s. Lowering sets it once every node is built.
        depth: u16,
    },
    /// Builds a list of the values of `items`, in order: each item is a
    /// node of any kind, whose value is the next element, or an
    /// [`IrKind::Spread`], whose list's elements are the next ones.
    MakeList {
        /// The elements and spreads, in source order.
        items: IrRange,
        /// Whether any item is a spread; if none is, the list's length is
        /// the number of items.
        spreads: bool,
    },
    /// One spread element of an [`IrKind::MakeList`], and only ever among
    /// its items: the list node whose elements it puts in its place.
    Spread(IrId),
    /// Reads the element of the list `list` evaluates to at the position
    /// `index` evaluates to, counting from 0; a position outside the list
    /// is a runtime error.
    Index {
        /// The list.
        list: IrId,
        /// The position, an `Int`.
        index: IrId,
        /// The kind of the element, which is the node's kind.
        kind: ValueKind,
    },
    /// The number of elements of the list its operand evaluates to, an
    /// `Int`: a call of the built-in function `len`.
    ListLen(IrId),
    /// A range: the list of the `Int`s from the left operand, an `Int`, up
    /// to the right one, an `Int`, in order, the right one included if the
    /// flag is `true` (`a..=b`) and not if it is `false` (`a..b`); empty if
    /// there are none. A runtime error if it would have more elements than
    /// a list can hold ([`crate::eval::MAX_LIST_LEN`]), unless it is
    /// directly the iterable of a [`IrKind::For`], which does not build it.
    Range(bool, Operands),
    /// A `for`: evaluates the iterable, the first of `parts`, a list node,
    /// once, and then for each of its elements in order writes the element
    /// to `slot`, evaluates the guard, the second of three parts, a `Bool`
    /// node, if there are three, and if it is true or there is none, the
    /// body, the last part, a node of any kind. An iterable that is
    /// directly a [`IrKind::Range`] is not built: its bounds are evaluated
    /// and its integers counted.
    For {
        /// The slot of the `for`'s variable, in the current frame.
        slot: Slot,
        /// The iterable, the guard if there is one, and the body.
        parts: IrRange,
        /// Whether the `for` yields the list of the body's values, in
        /// order, and is of kind [`ValueKind::List`]; otherwise it discards
        /// them and yields `{}`, a statement whose value nothing would
        /// hold, and is of kind [`ValueKind::Record`].
        collect: bool,
    },
    /// A condition loop or a `do` loop: of `parts`, the condition, a `Bool`
    /// node, the body, a node of any kind, and the items after the
    /// condition, each a node of any kind, run for their effect. While the
    /// condition is true, evaluates the body and then the items; if not
    /// `test_first`, a `do` loop's, the first iteration runs without the
    /// test. Collects what each iteration contributes as a `For` does.
    Loop {
        /// The condition, the body, and the items after the condition.
        parts: IrRange,
        /// Whether the loop yields the list of what its iterations
        /// contribute, of kind [`ValueKind::List`], or discards it and is
        /// `{}`, as a `For`'s flag says.
        collect: bool,
        /// Whether the condition is tested before the first iteration:
        /// `false` for a `do` loop.
        test_first: bool,
    },
    /// `break`: evaluates `value`, if there is one, a node of any kind, and
    /// unwinds to the loop `loops` loops out from the innermost one whose
    /// body it is in, which collects the value and ends (see
    /// [`crate::eval`]). Of kind [`ValueKind::Never`].
    Break {
        /// How many loops it leaves before its own: 0 for the innermost.
        loops: u32,
        /// What it contributes to its loop's list, if anything.
        value: Option<IrId>,
    },
    /// `continue`: as [`IrKind::Break`], but its loop collects the value
    /// and goes on with its next iteration.
    Continue {
        /// How many loops it leaves before its own: 0 for the innermost.
        loops: u32,
        /// What it contributes to its loop's list, if anything.
        value: Option<IrId>,
    },
    /// `return`: evaluates `value`, if there is one, a node of any kind,
    /// and unwinds to the [`IrKind::Catch`] numbered `to`, or for
    /// [`CatchId::FUNCTION`] out of the body of the function or `lazy`
    /// being run, which evaluates to the value, `{}` without one (see
    /// [`crate::eval`]). Of kind [`ValueKind::Never`].
    Return {
        /// What it leaves.
        to: CatchId,
        /// The value it returns, if it has one.
        value: Option<IrId>,
    },
    /// A labelled block, or a loop, that a `return` leaves: evaluates
    /// `body`, the block or the loop, a node of any kind, and yields its
    /// value, or the value of a `return` to `id` that comes out of it, of
    /// kind `kind`. Only a labelled block or a loop that some `return`
    /// leaves has one.
    Catch {
        /// The block or loop.
        body: IrId,
        /// The number the `return`s that leave it name.
        id: CatchId,
        /// The kind of the value: the block's or loop's type's, which is
        /// the join of its own and what is returned.
        kind: ValueKind,
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
    /// `fileroot.caller`: yields the text of the running frame's caller, the
    /// directory of the file whose code made the call the frame runs for,
    /// and records it as a dependency of the memoised calls it can affect
    /// (see [`crate::eval`], "Callers and helpers"). Of kind
    /// [`ValueKind::String`].
    Caller,
    /// `fileroot.helper()`: sets the running frame's helper flag, so that
    /// every call it makes after this one passes its own caller on; `{}`,
    /// of kind [`ValueKind::Record`] (see [`crate::eval`], "Callers and
    /// helpers").
    Helper,
    /// Builds a future of function `function` of the IR, a `lazy`'s body,
    /// capturing the values of `captures` as [`IrKind::MakeClosure`] does.
    /// Of kind [`ValueKind::Deferred`].
    MakeFuture {
        /// The function.
        function: FnId,
        /// What it captures.
        captures: IrRange,
        /// Whether the body can read the caller the future captures: it
        /// reads `fileroot.caller`, calls `fileroot.helper()`, or makes a
        /// `lazy` so marked, a nested function's body not counting. Making
        /// a marked future is a read of the making frame's caller (see
        /// [`crate::eval`], "Memoised functions").
        reads_caller: bool,
    },
    /// Evaluates `value`, a node of any kind, awaits it fully if it is a
    /// future, running its body if it has not run, and then, unless
    /// `shape` is [`LazyShapeId::NONE`], awaits the lazy fields of the
    /// record it is as the shape says; and yields the result, of kind
    /// `kind`, never [`ValueKind::Deferred`].
    Await {
        /// What is awaited.
        value: IrId,
        /// The kind of the awaited value: its type's, once present.
        kind: ValueKind,
        /// The record fields to await deeply, or [`LazyShapeId::NONE`].
        shape: LazyShapeId,
        /// The await's depth in the body of the frame it is evaluated in,
        /// for the nesting budget, as a call's (see [`IrKind::Call`]).
        /// Lowering sets it once every node is built.
        depth: u16,
    },
    /// Reads a top-level declaration of the input, a function or the cell
    /// of a top-level field, as a value of the given kind, by its slot in
    /// the top level's frame, which is at the bottom of the value stack
    /// (frame base 0) however deep the call it runs in: how a function or
    /// `lazy` refers to a top-level declaration of its own input, never by
    /// a capture. Of kind `Fn` it is slice 23's read of a top-level
    /// function, shown as `fn` in a dump; of any other kind, a field's
    /// cell, shown as `top`. Last, so that adding it numbered no other kind
    /// anew.
    Top(Slot, ValueKind),
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
    /// a store's value, a field access's record, a field update's record
    /// and value, a dotted update's value, record or list and positions, an
    /// interpolation's pieces, a record's values, a list's items,
    /// a spread's list, an index's list and position, a length's list, a
    /// range's bounds, a `for`'s iterable, guard and body, a condition
    /// loop's condition, body and items after it, a `break`'s, a
    /// `continue`'s and a `return`'s value, a catch's block or loop, a
    /// closure's and
    /// a future's captures, and an await's, an `is`'s and an `as`'s value.
    /// A closure's
    /// or a future's
    /// function is not a child: its body is evaluated in a frame of its own.
    /// This is the one list of every node's children, and its match is
    /// exhaustive, so a new kind of node cannot be missed: [`Ir::pretty`]
    /// prints the children it gives, and lowering's walk that computes the
    /// depths of calls and awaits and the heights of bodies follows them,
    /// and the two must agree.
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
            | IrKind::Top(..)
            | IrKind::LoadCallee
            | IrKind::Caller
            | IrKind::Helper => (None, &[], NONE),
            IrKind::IntNeg(operand)
            | IrKind::FloatNeg(operand)
            | IrKind::IntToFloat(operand)
            | IrKind::Not(operand)
            | IrKind::Field {
                record: operand, ..
            }
            | IrKind::Store(_, operand)
            | IrKind::Arg { value: operand, .. }
            | IrKind::Spread(operand)
            | IrKind::ListLen(operand)
            | IrKind::Is { value: operand, .. }
            | IrKind::As { value: operand, .. }
            | IrKind::Await { value: operand, .. }
            | IrKind::Catch { body: operand, .. } => (None, &[], [Some(operand), None, None]),
            IrKind::Break { value, .. }
            | IrKind::Continue { value, .. }
            | IrKind::Return { value, .. } => (None, &[], [value, None, None]),
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
            | IrKind::ListSame(Operands { lhs, rhs, .. })
            | IrKind::ListNotSame(Operands { lhs, rhs, .. })
            | IrKind::ValueSame(Operands { lhs, rhs, .. })
            | IrKind::ValueNotSame(Operands { lhs, rhs, .. })
            | IrKind::And(Operands { lhs, rhs, .. })
            | IrKind::Or(Operands { lhs, rhs, .. })
            | IrKind::Range(_, Operands { lhs, rhs, .. })
            | IrKind::RecordEq(ShapedOperands { lhs, rhs, .. })
            | IrKind::RecordNe(ShapedOperands { lhs, rhs, .. })
            | IrKind::ListEq(ShapedOperands { lhs, rhs, .. })
            | IrKind::ListNe(ShapedOperands { lhs, rhs, .. })
            | IrKind::UpdateField {
                record: lhs,
                value: rhs,
                ..
            }
            | IrKind::Index {
                list: lhs,
                index: rhs,
                ..
            } => (None, &[], [Some(lhs), Some(rhs), None]),
            IrKind::If { cond, then, else_ } => (None, &[], [Some(cond), Some(then), Some(else_)]),
            IrKind::Interp(items)
            | IrKind::MakeRecord { values: items, .. }
            | IrKind::MakeList { items, .. }
            | IrKind::For { parts: items, .. }
            | IrKind::Loop { parts: items, .. }
            | IrKind::Update { parts: items, .. }
            | IrKind::MakeClosure {
                captures: items, ..
            }
            | IrKind::MakeFuture {
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
            IrKind::BoolEq(true, _) | IrKind::RecordEq(_) | IrKind::ListEq(_) => Some("=="),
            IrKind::BoolEq(false, _) | IrKind::RecordNe(_) | IrKind::ListNe(_) => Some("!="),
            IrKind::RecordSame(_) | IrKind::ListSame(_) | IrKind::ValueSame(_) => Some("==="),
            IrKind::RecordNotSame(_) | IrKind::ListNotSame(_) | IrKind::ValueNotSame(_) => {
                Some("!==")
            }
            IrKind::And(_) => Some("and"),
            IrKind::Or(_) => Some("or"),
            IrKind::Not(_) => Some("not"),
            IrKind::Range(false, _) => Some(".."),
            IrKind::Range(true, _) => Some("..="),
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
            | IrKind::Update { .. }
            | IrKind::MakeClosure { .. }
            | IrKind::Call { .. }
            | IrKind::Arg { .. }
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::LoadCallee
            | IrKind::Caller
            | IrKind::Helper
            | IrKind::MakeFuture { .. }
            | IrKind::Await { .. }
            | IrKind::MakeList { .. }
            | IrKind::Spread(_)
            | IrKind::Index { .. }
            | IrKind::ListLen(_)
            | IrKind::For { .. }
            | IrKind::Loop { .. }
            | IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::Is { .. }
            | IrKind::As { .. } => None,
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
            IrKind::ListEq(_) | IrKind::ListNe(_) => "list-eq",
            IrKind::ListSame(_) | IrKind::ListNotSame(_) => "list-same",
            IrKind::ValueSame(_) | IrKind::ValueNotSame(_) => "value-same",
            IrKind::Is { .. } => "is",
            IrKind::As { .. } => "as",
            IrKind::MakeList { .. } => "make-list",
            IrKind::Spread(_) => "spread",
            IrKind::Index { .. } => "index",
            IrKind::ListLen(_) => "list-len",
            IrKind::Range(false, _) => "range",
            IrKind::Range(true, _) => "range-inclusive",
            IrKind::For { collect: true, .. } => "for",
            IrKind::For { collect: false, .. } => "for-each",
            IrKind::Loop {
                collect: true,
                test_first: true,
                ..
            } => "loop",
            IrKind::Loop {
                collect: false,
                test_first: true,
                ..
            } => "loop-each",
            IrKind::Loop {
                collect: true,
                test_first: false,
                ..
            } => "do",
            IrKind::Loop {
                collect: false,
                test_first: false,
                ..
            } => "do-each",
            IrKind::Break { .. } => "break",
            IrKind::Continue { .. } => "continue",
            IrKind::Return { .. } => "return",
            IrKind::Catch { .. } => "catch",
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
            IrKind::Update { .. } => "update",
            IrKind::MakeClosure { .. } => "make-closure",
            IrKind::Call { .. } => "call",
            IrKind::Arg { .. } => "arg",
            IrKind::LoadCapture(..) => "load-capture",
            IrKind::Top(_, ValueKind::Fn) => "fn",
            IrKind::Top(..) => "top",
            IrKind::LoadCallee => "load-callee",
            IrKind::Caller => "caller",
            IrKind::Helper => "helper",
            IrKind::MakeFuture { .. } => "make-future",
            IrKind::Await { .. } => "await",
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
    /// runtime error in this node is reported. `None` for typed record and
    /// list comparisons, which cannot fail and do not keep it (see
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
            | IrKind::ListSame(operands)
            | IrKind::ListNotSame(operands)
            | IrKind::ValueSame(operands)
            | IrKind::ValueNotSame(operands)
            | IrKind::And(operands)
            | IrKind::Or(operands)
            | IrKind::Range(_, operands) => operands.op_start,
            // A prefix operator starts its expression.
            IrKind::IntNeg(_) | IrKind::FloatNeg(_) | IrKind::Not(_) => self.span.start,
            IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::ListEq(_)
            | IrKind::ListNe(_)
            | IrKind::MakeList { .. }
            | IrKind::Spread(_)
            | IrKind::Index { .. }
            | IrKind::ListLen(_)
            | IrKind::For { .. }
            | IrKind::Loop { .. }
            | IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
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
            | IrKind::Update { .. }
            | IrKind::MakeClosure { .. }
            | IrKind::Call { .. }
            | IrKind::Arg { .. }
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::LoadCallee
            | IrKind::Caller
            | IrKind::Helper
            | IrKind::MakeFuture { .. }
            | IrKind::Await { .. }
            | IrKind::Is { .. }
            | IrKind::As { .. } => return None,
        };
        Some(crate::ast::symbol_span(symbol, start))
    }
}

/// One function of a program, or one `lazy`'s body, in [`Ir::function`]'s
/// table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnCode {
    /// The body, in the same node arena as the rest of the program: a
    /// [`IrKind::Block`] for a function, or for a labelled body that a
    /// `return` leaves an [`IrKind::Catch`] of one (or an
    /// [`IrKind::Await`] of either, where its value is awaited to fit its
    /// return type), and any node for a `lazy`. A `return` for the
    /// function or `lazy` comes out of it as a stop, which the call, the
    /// memoised call or the await running it catches.
    pub body: IrId,
    /// Whether this is a function expression's code, memoised or not, or a
    /// `lazy`'s.
    pub kind: FnKind,
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
    /// The function's type as text, `fn(x: Int): Int`, or a `lazy`'s,
    /// `Future<Int>`: what its closures or futures display as.
    pub display: Box<str>,
    /// For each slot of the body's frame, the name of the last variable
    /// assigned to it. For [`Ir::pretty`] only.
    pub slot_names: Box<[Name]>,
}

/// A lowered program: an arena of [`IrNode`]s, the item pool, the string
/// constants, the record layouts, shapes and lazy shapes, the text of every
/// name an update may add to a record, the functions, its root, the frame
/// it needs, and the session input it was lowered from.
#[derive(Clone, Debug)]
pub struct Ir {
    nodes: Vec<IrNode>,
    item_lists: Vec<IrId>,
    strings: Vec<Rc<str>>,
    layouts: Vec<Rc<Layout>>,
    shapes: Vec<Shape>,
    /// For each shape of `shapes`, a list's shape if it is one, whose entry
    /// in `shapes` then has no fields.
    list_shapes: Vec<Option<ListShape>>,
    lazy_shapes: Vec<LazyShape>,
    /// For each lazy shape of `lazy_shapes`, if it is a list's, what is done
    /// with each element; its entry in `lazy_shapes` then has no fields.
    lazy_lists: Vec<Option<LazyField>>,
    /// For each lazy shape of `lazy_shapes`, if it is a union's, the lazy
    /// shapes of its members; its entry in `lazy_shapes` then has no
    /// fields, and it has none in `lazy_lists`.
    lazy_unions: Vec<Option<Box<[LazyShapeId]>>>,
    /// The test of every `is` and `as`, indexed by [`TestId`].
    tests: Vec<TestCode>,
    /// The text of the name of every [`IrKind::UpdateField`], and of every
    /// field an [`IrKind::Update`]'s path steps to, which may add it to a
    /// record's layout at runtime.
    name_texts: HashMap<Name, Box<str>>,
    /// The steps of every update's path, each path's together.
    update_steps: Vec<UpdateStep>,
    /// Where each update's path is in `update_steps`, indexed by
    /// [`UpdatePathId`].
    update_paths: Vec<StepRange>,
    /// Every function of the program, indexed by [`FnId`].
    functions: Vec<FnCode>,
    root: IrId,
    frame_size: u32,
    /// For each slot of the root's frame, the name of the last variable
    /// assigned to it. For [`Ir::pretty`] only.
    slot_names: Vec<Name>,
    /// The session input the program was lowered from; 0 for a file.
    input: u32,
    /// The directory of the file the program was lowered from, by its index
    /// in its run's table of directories (see [`Ir::dir`]).
    dir: u32,
    /// Each top-level function named by a dotted path, `def
    /// dev.peios.hello`, which refers to itself by no name, with its path,
    /// which [`Ir::pretty`] shows where it shows another's name. For
    /// [`Ir::pretty`] only; usually empty.
    paths: Vec<(FnId, Box<[Name]>)>,
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

    /// Every await in the frame whose body is `body`, with its depth and
    /// span, in the order a depth-first walk meets them, for tests, as
    /// [`Ir::calls_in`] finds calls.
    #[cfg(test)]
    pub(crate) fn awaits_in(&self, body: IrId) -> Vec<(u16, Span)> {
        let mut awaits = Vec::new();
        let mut pending = vec![body];
        while let Some(id) = pending.pop() {
            let node = self.get(id);
            if let IrKind::Await { depth, .. } = node.kind {
                awaits.push((depth, node.span));
            }
            pending.extend(self.children(id).rev());
        }
        awaits
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

    /// Returns the record shape `id`: one with no fields if it is a list's
    /// shape (see [`Ir::list_shape`]).
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR.
    #[must_use]
    pub fn shape(&self, id: ShapeId) -> &Shape {
        &self.shapes[id.index()]
    }

    /// The list shape `id`, or `None` if it is a record's shape, or an id
    /// this IR does not have.
    #[must_use]
    pub fn list_shape(&self, id: ShapeId) -> Option<ListShape> {
        self.list_shapes.get(id.index()).copied().flatten()
    }

    /// The number of distinct record and list shapes.
    #[must_use]
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    /// Returns the lazy shape `id`: one with no fields if it is a list's
    /// lazy shape (see [`Ir::lazy_list`]).
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this IR, as
    /// [`LazyShapeId::NONE`] never does.
    #[must_use]
    pub fn lazy_shape(&self, id: LazyShapeId) -> &LazyShape {
        &self.lazy_shapes[id.index()]
    }

    /// Returns the lazy shape `id`, or `None` for [`LazyShapeId::NONE`] and
    /// for an id this IR does not have.
    #[must_use]
    pub fn find_lazy_shape(&self, id: LazyShapeId) -> Option<&LazyShape> {
        self.lazy_shapes.get(id.index())
    }

    /// What a deep await does with each element of a list whose lazy shape
    /// is `id`, or `None` if `id` is a record's lazy shape,
    /// [`LazyShapeId::NONE`], or an id this IR does not have.
    #[must_use]
    pub fn lazy_list(&self, id: LazyShapeId) -> Option<LazyField> {
        self.lazy_lists.get(id.index()).copied().flatten()
    }

    /// The lazy shapes of the members of the union whose lazy shape is
    /// `id`, those that have one, or `None` if `id` is not a union's lazy
    /// shape, is [`LazyShapeId::NONE`], or is an id this IR does not have.
    /// A deep await applies each to a value of its member's kind, a
    /// record's to a record and a list's to a list, *tolerantly*: since
    /// which member the value is of is not known, a field the shape lists
    /// that the record lacks, and a value inside that is not of the kind its
    /// shape is for, are passed over, at every level below.
    #[must_use]
    pub fn lazy_union(&self, id: LazyShapeId) -> Option<&[LazyShapeId]> {
        self.lazy_unions.get(id.index())?.as_deref()
    }

    /// The number of distinct lazy shapes, of records, lists and unions.
    #[must_use]
    pub fn lazy_shape_count(&self) -> usize {
        self.lazy_shapes.len()
    }

    /// The test of an `is` or an `as`, `id`, or `None` for an id this IR
    /// does not have.
    #[must_use]
    pub fn test(&self, id: TestId) -> Option<&TestCode> {
        self.tests.get(id.index())
    }

    /// The text of `name`, if it is the name of an [`IrKind::UpdateField`],
    /// of a field an [`IrKind::Update`]'s path steps to, or of a field a
    /// test looks at: what the evaluator shows a field it adds to a record,
    /// or that fails a test, as.
    #[must_use]
    pub fn name_text(&self, name: Name) -> Option<&str> {
        self.name_texts.get(&name).map(AsRef::as_ref)
    }

    /// The steps of the update path `id`, in order, the last setting the
    /// leaf; `None` for an id this IR does not have.
    #[must_use]
    pub fn update_path(&self, id: UpdatePathId) -> Option<&[UpdateStep]> {
        let range = self.update_paths.get(id.index())?;
        let start = range.start as usize;
        self.update_steps.get(start..start + range.len as usize)
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

    /// Replaces the names of the root frame's slots, which only
    /// [`Ir::pretty`] reads, with `slot_names`, returning the old ones: a
    /// workspace keeps one table for every file's program rather than a
    /// copy of the whole frame's in each (see [`crate::workspace`]).
    pub(crate) fn replace_slot_names(&mut self, slot_names: Vec<Name>) -> Vec<Name> {
        mem::replace(&mut self.slot_names, slot_names)
    }

    /// Sets the paths of the program's top-level functions named by dotted
    /// paths, `paths`, each with its function, which only [`Ir::pretty`]
    /// reads.
    pub(crate) fn set_paths(&mut self, paths: Vec<(FnId, Box<[Name]>)>) {
        self.paths = paths;
    }

    /// The path of the function whose index is `index`, if it is a
    /// top-level function named by a dotted path (see [`Ir::set_paths`]);
    /// empty otherwise.
    fn path_of(&self, index: usize) -> &[Name] {
        self.paths
            .iter()
            .find(|(function, _)| function.index() == index)
            .map_or(&[][..], |(_, path)| path)
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

    /// The directory of the file the program was lowered from, by its index
    /// in its run's table of directories, which the driver built
    /// ([`crate::check::Place::dir`]): what a call made by its code, or by a
    /// closure or a future of it, passes on as its callee's caller (see
    /// [`crate::eval`], "Callers and helpers"). 0, the root's, for a
    /// program lowered with no workspace's context.
    #[must_use]
    pub fn dir(&self) -> u32 {
        self.dir
    }

    /// The kind of value node `id` produces: [`ValueKind::Deferred`] for a
    /// `MakeFuture` and for a load, capture, field access, index or call of
    /// a value that may be a future, and never for an `Await`;
    /// [`ValueKind::Mixed`] for a load, capture, field access, index, call,
    /// await or `as` of a union of kinds. A block's kind is its result's,
    /// and an `if`'s its `then` branch's, which for an `if` of a union of
    /// kinds, whose branches may be of different kinds, is one of them, or
    /// its `else` branch's if the `then` branch is of kind
    /// [`ValueKind::Never`], as a `break` and a `continue` are.
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
                // A branch that never completes says nothing of the `if`'s
                // kind, unless both are such.
                IrKind::If { then, else_, .. } => {
                    id = if self.kind_of(then) == ValueKind::Never {
                        else_
                    } else {
                        then
                    };
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
                | IrKind::ListEq(_)
                | IrKind::ListNe(_)
                | IrKind::ListSame(_)
                | IrKind::ListNotSame(_)
                | IrKind::ValueSame(_)
                | IrKind::ValueNotSame(_)
                | IrKind::Is { .. }
                | IrKind::And(_)
                | IrKind::Or(_)
                | IrKind::Not(_) => ValueKind::Bool,
                IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) | IrKind::Caller => {
                    ValueKind::String
                }
                IrKind::IntConst(_)
                | IrKind::IntNeg(_)
                | IrKind::IntAdd(_)
                | IrKind::IntSub(_)
                | IrKind::IntMul(_)
                | IrKind::IntFloorDiv(_)
                | IrKind::IntRem(_)
                | IrKind::ListLen(_) => ValueKind::Int,
                IrKind::MakeList { .. }
                | IrKind::Spread(_)
                | IrKind::Range(..)
                | IrKind::For { collect: true, .. }
                | IrKind::Loop { collect: true, .. } => ValueKind::List,
                IrKind::Break { .. } | IrKind::Continue { .. } | IrKind::Return { .. } => {
                    ValueKind::Never
                }
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
                | IrKind::UpdateField { .. }
                | IrKind::Helper
                | IrKind::For { collect: false, .. }
                | IrKind::Loop { collect: false, .. } => ValueKind::Record,
                IrKind::Load(_, kind)
                | IrKind::Field { kind, .. }
                | IrKind::Update { kind, .. }
                | IrKind::Index { kind, .. }
                | IrKind::Call { kind, .. }
                | IrKind::LoadCapture(_, kind)
                | IrKind::Await { kind, .. }
                | IrKind::As { kind, .. }
                | IrKind::Catch { kind, .. }
                | IrKind::Top(_, kind) => kind,
                IrKind::MakeClosure { .. } | IrKind::LoadCallee => ValueKind::Fn,
                IrKind::MakeFuture { .. } => ValueKind::Deferred,
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
    /// `(make-record [x y] @0..9 ...)`, and a field access or a field update
    /// its name, as in `(field x @0..3 ...)` and `(update-field x @0..3
    /// ...)`, then its record and, for an update, the value. A dotted `let`
    /// or `:=` shows the steps of its path, as in `(update .hosts[].up
    /// @0..16 ...)`, a step whose record or list is awaited followed by `!`
    /// and one whose record is created by `+`, and then its value, the
    /// record or list it updates and its positions. A name that is not a
    /// plain identifier is quoted. A typed record comparison shows its
    /// operator and its shape's field names in the shape's order, each
    /// record field's own shape after its name, as in
    /// `(record-eq == [x inner[n]] @0..9 ...)`; a total one its operator,
    /// as in `(record-same === @0..9 ...)`. A list comparison shows the
    /// same, a list's shape being `*` and its element type's shape, as in
    /// `(list-eq == *[x] @0..9 ...)`, `(list-eq == * @0..9 ...)` and
    /// `(list-same === @0..9 ...)`. A list shows as `(make-list @0..9 ...)`
    /// and then its items, a spread as `(spread @0..5 ...)` and its list, an
    /// index as `(index @0..5 ...)`, its list and its position, and a
    /// length as `(list-len @0..7 ...)` and its list. A range shows as
    /// `(range @0..4 ...)` or `(range-inclusive @0..5 ...)` and its bounds,
    /// and a `for` as `(for x#2 @0..20 ...)`, its variable's slot followed
    /// by ` if` if it has a guard, and then its iterable, its guard and its
    /// body; one that discards the body's values as `(for-each x#2 @0..20
    /// ...)`. A condition loop shows as `(loop @0..20 ...)`, or
    /// `(loop-each @0..20 ...)` if it discards, and a `do` loop as `(do
    /// @0..20 ...)` or `(do-each @0..20 ...)`, each followed by its
    /// condition, its body and the items after its condition; a `break` as
    /// `(break 1 @0..7 ...)` and a `continue` as `(continue 0 @0..8)`, with
    /// the number of loops it leaves before its own, and its value, if it
    /// has one; a `return` as `(return 0 @0..9 ...)`, with the number of
    /// the catch it leaves for, or as `(return fn @0..8 ...)` for one that
    /// leaves its function or `lazy`, and its value, if it has one; and a
    /// catch as `(catch 0 @0..30 ...)`, with its number, and its block or
    /// loop. A closure shows its
    /// function's
    /// number and then what it captures, as in `(make-closure #0 @0..9
    /// ...)`, a call its callee and then its arguments, each as
    /// `(arg 0 @4..5 ...)` with the parameter it fills, and the reads of a
    /// capture and of the callee as `(load-capture 0 @4..5)` and
    /// `(load-callee @4..5)`. A future shows its function's number and then
    /// what it captures, as in `(make-future #1 @0..9 ...)`, and an await
    /// its value, as in `(await @0..9 ...)`, or, for a deep await, its
    /// shape first: the names of the fields it awaits, each followed by `!`
    /// if its value is awaited and by its own shape in brackets if the
    /// await descends into it, as in `(await [a! b[c!]] @0..9 ...)`, or for
    /// a list, `*`, `!` if each element is awaited, and the element's shape
    /// if the await descends into it, as in `(await *! @0..9 ...)`, or for
    /// a union, its members' shapes separated by ` | ` in parentheses, as in
    /// `(await ([a!] | *!) @0..9 ...)`. The depths of calls and awaits are
    /// not shown. An `is` shows its test's plan and, if it narrows, its
    /// slot, as in `(is [int] x#2 @0..9 ...)`, and an `as` its plan, as in
    /// `(as {a: int} @0..9 ...)` (see `Pretty::test` for how a plan is
    /// written); a total comparison of two values of a union of kinds shows
    /// as `(value-same === @0..9 ...)`. After the root, each
    /// function is shown on a line of its own as
    /// `fn #0 name (frame 3, params 2, captures 1, height 5)`, a top-level
    /// function named by a dotted path by its path, `fn #0 dev.peios.count
    /// (frame ...)`, or `fn #1 (frame ...)` if it has no name, a memoised
    /// function as
    /// `memo fn #2 name (frame ...)`, and each `lazy`'s body as
    /// `future #3 (frame ...)`, followed by its body, whose slots are named
    /// by its own frame's variables. There is no trailing newline.
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
    list_shapes: Vec<Option<ListShape>>,
    lazy_shapes: Vec<LazyShape>,
    lazy_lists: Vec<Option<LazyField>>,
    lazy_unions: Vec<Option<Box<[LazyShapeId]>>>,
    tests: Vec<TestCode>,
    name_texts: HashMap<Name, Box<str>>,
    update_steps: Vec<UpdateStep>,
    update_paths: Vec<StepRange>,
}

impl IrBuilder {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            item_lists: Vec::new(),
            strings: Vec::new(),
            layouts: Vec::new(),
            shapes: Vec::new(),
            list_shapes: Vec::new(),
            lazy_shapes: Vec::new(),
            lazy_lists: Vec::new(),
            lazy_unions: Vec::new(),
            tests: Vec::new(),
            name_texts: HashMap::new(),
            update_steps: Vec::new(),
            update_paths: Vec::new(),
        }
    }

    /// Adds a node and returns its id. The builder holds at most one node
    /// per AST node or item, plus one `Spread` per spread element of a list,
    /// one conversion per `/` operand, one
    /// `Empty` or `MakeRecord` per block, one `Load` per field, one
    /// constant per literal piece of an interpolation, for each segment of
    /// a dotted field's path after the first an update and a record to
    /// update, for each dotted `let` or `:=` an update and the record or
    /// list it reads and awaits, one
    /// `Arg` per argument of a call, one `Await` per expression awaited
    /// where it is used, and one load per value a closure or a future
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
    /// record or list type, and fewer such types than bytes of source, so
    /// ids never saturate either. Deduplicating is the caller's business.
    pub(crate) fn push_shape(&mut self, shape: Shape) -> ShapeId {
        let id = ShapeId(u32::try_from(self.shapes.len()).unwrap_or(u32::MAX));
        self.shapes.push(shape);
        self.list_shapes.push(None);
        id
    }

    /// Replaces the fields of shape `id`, which [`IrBuilder::push_shape`]
    /// returned, so that a shape can be given its id before its fields are
    /// known, and makes it a record's shape. Does nothing for an id it did
    /// not return.
    pub(crate) fn set_shape(&mut self, id: ShapeId, shape: Shape) {
        if let Some(entry) = self.shapes.get_mut(id.index()) {
            *entry = shape;
        }
        if let Some(entry) = self.list_shapes.get_mut(id.index()) {
            *entry = None;
        }
    }

    /// Makes shape `id`, which [`IrBuilder::push_shape`] returned, the list
    /// shape `shape`, with no fields, so that a list's shape too can be
    /// given its id before what it holds is known. Does nothing for an id
    /// it did not return.
    pub(crate) fn set_list_shape(&mut self, id: ShapeId, shape: ListShape) {
        if let Some(entry) = self.shapes.get_mut(id.index()) {
            entry.fields = Box::default();
        }
        if let Some(entry) = self.list_shapes.get_mut(id.index()) {
            *entry = Some(shape);
        }
    }

    /// Adds a record's lazy shape and returns its id. There is at most one
    /// per record or list type, so ids never saturate, nor reach
    /// [`LazyShapeId::NONE`]. Deduplicating is the caller's business.
    pub(crate) fn push_lazy_shape(&mut self, shape: LazyShape) -> LazyShapeId {
        let id = LazyShapeId(u32::try_from(self.lazy_shapes.len()).unwrap_or(u32::MAX));
        self.lazy_shapes.push(shape);
        self.lazy_lists.push(None);
        self.lazy_unions.push(None);
        id
    }

    /// Adds a list's lazy shape, which awaits each element as `element`
    /// says, and returns its id, as [`IrBuilder::push_lazy_shape`] does.
    pub(crate) fn push_lazy_list(&mut self, element: LazyField) -> LazyShapeId {
        let id = self.push_lazy_shape(LazyShape {
            fields: Box::default(),
        });
        if let Some(entry) = self.lazy_lists.get_mut(id.index()) {
            *entry = Some(element);
        }
        id
    }

    /// Adds a union's lazy shape, which awaits what each of `members`, the
    /// lazy shapes of the union's members that have one, says, of a value
    /// of that member's kind and as far as the value has what it lists (see
    /// [`Ir::lazy_union`]), and returns its id, as
    /// [`IrBuilder::push_lazy_shape`] does.
    pub(crate) fn push_lazy_union(&mut self, members: Box<[LazyShapeId]>) -> LazyShapeId {
        let id = self.push_lazy_shape(LazyShape {
            fields: Box::default(),
        });
        if let Some(entry) = self.lazy_unions.get_mut(id.index()) {
            *entry = Some(members);
        }
        id
    }

    /// Adds the test of an `is` or an `as` and returns its id. There is at
    /// most one per such expression, so ids never saturate either.
    pub(crate) fn push_test(&mut self, test: TestCode) -> TestId {
        let id = TestId(u32::try_from(self.tests.len()).unwrap_or(u32::MAX));
        self.tests.push(test);
        id
    }

    /// Records `text` as the text of `name`, the name of an
    /// [`IrKind::UpdateField`], of a field an [`IrKind::Update`]'s path
    /// steps to or of a field a test looks at, unless it is recorded
    /// already.
    pub(crate) fn add_name_text(&mut self, name: Name, text: &str) {
        self.name_texts
            .entry(name)
            .or_insert_with(|| Box::from(text));
    }

    /// Adds the steps of an update's path, `steps`, and returns its id.
    /// There is at most one per dotted `let` or `:=`, and one step per
    /// segment of its path, so ids never saturate either.
    pub(crate) fn push_update_path(&mut self, steps: &[UpdateStep]) -> UpdatePathId {
        let id = UpdatePathId(u32::try_from(self.update_paths.len()).unwrap_or(u32::MAX));
        self.update_paths.push(StepRange {
            start: u32::try_from(self.update_steps.len()).unwrap_or(u32::MAX),
            len: u32::try_from(steps.len()).unwrap_or(u32::MAX),
        });
        self.update_steps.extend_from_slice(steps);
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

    /// Finishes a program without functions, lowered from input 0, for
    /// building IR by hand in tests: its calls are given their depths as
    /// lowering gives them.
    #[cfg(test)]
    pub(crate) fn finish(mut self, root: IrId, frame_size: u32, slot_names: Vec<Name>) -> Ir {
        self.measure(root, &mut []);
        self.finish_program(root, (frame_size, slot_names), Vec::new(), (0, 0))
    }

    /// Gives every [`IrKind::Call`], [`IrKind::Await`] and
    /// [`IrKind::Update`] of the program whose root is `root` and whose
    /// functions are `functions` its depth in its frame, and every function
    /// its body's height, [`FnCode::height`]: one walk of the root and one of
    /// each function's body, through [`IrKind::children`]. The heights of all
    /// the nodes are kept in one vector, and the walks share one stack.
    ///
    /// An update is two levels taller than its tallest child for each step
    /// of its path, as the parser counts each segment of a path after the
    /// first (see [`crate::MAX_DEPTH`]), although the evaluator walks the
    /// path without recursing: so the nesting budget is charged for a long
    /// path as it is for deep nesting.
    ///
    /// The nodes of each frame form a tree, apart from the record nodes a
    /// dotted field's update chain shares between an update and the field
    /// access into the record it updates (see [`crate::lower`]). The walk
    /// visits such a node once, from the first parent it reaches it by, and
    /// every other parent takes its height as computed then.
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
    /// and await its depth on the way down and each node its height in
    /// `heights` on the way up, and returns the body's height. Iterative,
    /// with `stack` as the work list, since a body can be
    /// [`crate::MAX_DEPTH`] deep: each entry is a node, its depth, and
    /// whether its children have been pushed, in which case they have all
    /// been walked when it is popped again.
    fn walk(&mut self, body: IrId, heights: &mut [u32], stack: &mut Vec<(IrId, u32, bool)>) -> u32 {
        let Self {
            nodes,
            item_lists,
            update_paths,
            ..
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
                // Two levels for each step of an update's path.
                let path = match node.kind {
                    IrKind::Update { path, .. } => update_paths
                        .get(path.index())
                        .map_or(0, |steps| steps.len.saturating_mul(2)),
                    _ => 0,
                };
                if let Some(entry) = heights.get_mut(id.index()) {
                    *entry = tallest.saturating_add(1).saturating_add(path);
                }
            } else if height == 0 {
                if let IrKind::Call {
                    depth: node_depth, ..
                }
                | IrKind::Await {
                    depth: node_depth, ..
                }
                | IrKind::Update {
                    depth: node_depth, ..
                } = &mut node.kind
                {
                    *node_depth = u16::try_from(depth).unwrap_or(u16::MAX);
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
    /// input `input`, a file in the directory of index `dir` in its run's
    /// table of directories.
    pub(crate) fn finish_program(
        self,
        root: IrId,
        (frame_size, slot_names): (u32, Vec<Name>),
        functions: Vec<FnCode>,
        (input, dir): (u32, u32),
    ) -> Ir {
        Ir {
            nodes: self.nodes,
            item_lists: self.item_lists,
            strings: self.strings,
            layouts: self.layouts,
            shapes: self.shapes,
            list_shapes: self.list_shapes,
            lazy_shapes: self.lazy_shapes,
            lazy_lists: self.lazy_lists,
            lazy_unions: self.lazy_unions,
            tests: self.tests,
            name_texts: self.name_texts,
            update_steps: self.update_steps,
            update_paths: self.update_paths,
            functions,
            root,
            frame_size,
            slot_names,
            input,
            dir,
            paths: Vec::new(),
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
    /// Writes shape `id` as its field names in brackets, each record or
    /// list field's shape after its name: `[x inner[n]]`; a list's shape is
    /// `*` followed by its element type's shape, if it has one: `*[x]`,
    /// `**[x]` or `*`. Nested shapes are written through a work list, not
    /// recursion, since a record or list type can be nested as deeply as a
    /// program is long.
    fn shape(&self, f: &mut fmt::Formatter<'_>, id: ShapeId) -> fmt::Result {
        // Each record shape being written, with the index of its next field.
        let mut pending = Vec::new();
        self.open_shape(f, id, &mut pending)?;
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
                self.open_shape(f, nested, &mut pending)?;
            }
        }
        Ok(())
    }

    /// Starts writing shape `id` (see `Pretty::shape`): a `*` for each list
    /// shape down to the first shape that is not a list's, and if that is a
    /// record's, its `[`, queueing it on `pending` to have its fields
    /// written.
    fn open_shape(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: ShapeId,
        pending: &mut Vec<(ShapeId, usize)>,
    ) -> fmt::Result {
        let mut current = id;
        while let Some(list) = self.ir.list_shape(current) {
            f.write_str("*")?;
            match list.element {
                Some(element) => current = element,
                None => return Ok(()),
            }
        }
        f.write_str("[")?;
        pending.push((current, 0));
        Ok(())
    }

    /// Writes lazy shape `id` as the names of the fields it awaits in
    /// brackets, each followed by `!` if its value is awaited and by its own
    /// shape if the await descends into it: `[a! b[c!]]`; a list's lazy
    /// shape is `*`, followed by `!` if each element is awaited and by the
    /// element's own shape if the await descends into it: `*!`, `*[a!]`.
    /// Nested shapes are written through a work list, not recursion, as
    /// `Pretty::shape` writes record shapes.
    fn lazy_shape(&self, f: &mut fmt::Formatter<'_>, id: LazyShapeId) -> fmt::Result {
        // Each record's shape being written, with the index of its next
        // field.
        let mut pending = Vec::new();
        self.open_lazy_shape(f, id, &mut pending)?;
        while let Some((shape, index)) = pending.pop() {
            let fields = self.ir.find_lazy_shape(shape).map(|s| &*s.fields);
            let Some(&(name, field)) = fields.unwrap_or_default().get(index) else {
                f.write_str("]")?;
                continue;
            };
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{}", FieldName(self.names.get(name)))?;
            pending.push((shape, index + 1));
            let nested = match field {
                LazyField::Await(nested) => {
                    f.write_str("!")?;
                    nested
                }
                LazyField::Within(nested) => nested,
            };
            if nested != LazyShapeId::NONE {
                self.open_lazy_shape(f, nested, &mut pending)?;
            }
        }
        Ok(())
    }

    /// Starts writing lazy shape `id` (see `Pretty::lazy_shape`): a `*`,
    /// and a `!` for an awaited element, for each list's lazy shape down to
    /// the first that is not a list's; then, if that is a union's, its
    /// members' shapes separated by ` | ` in parentheses, as in `(*! |
    /// [a!])`, and if it is a record's, its `[`, queueing it on `pending` to
    /// have its fields written.
    fn open_lazy_shape(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: LazyShapeId,
        pending: &mut Vec<(LazyShapeId, usize)>,
    ) -> fmt::Result {
        let mut current = id;
        while let Some(element) = self.ir.lazy_list(current) {
            f.write_str("*")?;
            current = match element {
                LazyField::Await(nested) => {
                    f.write_str("!")?;
                    nested
                }
                LazyField::Within(nested) => nested,
            };
            if current == LazyShapeId::NONE {
                return Ok(());
            }
        }
        // A union's members' shapes, each whole, recursing once per union
        // nested in one of them.
        if let Some(members) = self.ir.lazy_union(current) {
            f.write_str("(")?;
            for (index, &member) in members.iter().enumerate() {
                if index > 0 {
                    f.write_str(" | ")?;
                }
                self.lazy_shape(f, member)?;
            }
            return f.write_str(")");
        }
        f.write_str("[")?;
        pending.push((current, 0));
        Ok(())
    }

    /// Writes the steps of the update path `id` as the path after its first
    /// segment: `.` and the name for a field and `[]` for an element, each
    /// followed by `!` if what it selects is awaited and by `+` if it is
    /// created, as in `.hosts[].up`, `.a!.b` and `.c+.d`.
    fn update_path(&self, f: &mut fmt::Formatter<'_>, id: UpdatePathId) -> fmt::Result {
        for step in self.ir.update_path(id).unwrap_or_default() {
            match step.at {
                StepAt::Field(name) => write!(f, ".{}", FieldName(self.names.get(name)))?,
                StepAt::Element => f.write_str("[]")?,
            }
            match step.mode {
                StepMode::Await => f.write_str("!")?,
                StepMode::Create => f.write_str("+")?,
                StepMode::Read | StepMode::Set => {}
            }
        }
        Ok(())
    }

    /// Writes the plan of the test `test` (see [`ValueTest`]): `*` for one
    /// that always passes, a kind's name for a test of the kind alone
    /// (`int`, `float`, `bool`, `string`, `record`, `list`), the fields a
    /// record must have, each with its own test, in braces, as in `{a:
    /// int}`, the test of a list's elements in brackets, as in `[int]`, and
    /// the alternatives of a union separated by ` | ` in parentheses, as in
    /// `(int | [string])`. So the dump shows how far into a value a test
    /// walks.
    ///
    /// Recurses once per level of record, list or union nesting in the
    /// plan, which is no deeper than the type it tests for.
    fn test(&self, f: &mut fmt::Formatter<'_>, test: &ValueTest) -> fmt::Result {
        match test {
            ValueTest::Pass => f.write_str("*"),
            ValueTest::Kind(kind) => f.write_str(match kind {
                ValueKind::Int => "int",
                ValueKind::Float => "float",
                ValueKind::Bool => "bool",
                ValueKind::String => "string",
                ValueKind::Record => "record",
                ValueKind::List => "list",
                ValueKind::Fn => "fn",
                ValueKind::Deferred => "deferred",
                ValueKind::Mixed => "mixed",
                ValueKind::Never => "never",
            }),
            ValueTest::Record(fields) => {
                f.write_str("{")?;
                for (index, (name, field)) in fields.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: ", FieldName(self.names.get(*name)))?;
                    self.test(f, field)?;
                }
                f.write_str("}")
            }
            ValueTest::List(element) => {
                f.write_str("[")?;
                self.test(f, element)?;
                f.write_str("]")
            }
            ValueTest::Any(alternatives) => {
                f.write_str("(")?;
                for (index, alternative) in alternatives.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" | ")?;
                    }
                    self.test(f, alternative)?;
                }
                f.write_str(")")
            }
        }
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
            // A slot of the top level's frame, whatever function it is in,
            // so named by the top level's slot names.
            IrKind::Top(slot, _) => match self.ir.slot_names.get(slot.index()) {
                Some(&name) => write!(f, " top {}{slot}", FieldName(self.names.get(name)))?,
                None => write!(f, " top ?{slot}")?,
            },
            IrKind::IntCmp(cmp, _) | IrKind::FloatCmp(cmp, _) | IrKind::StrCmp(cmp, _) => {
                write!(f, " {}", cmp.symbol())?;
            }
            IrKind::BoolEq(eq, _) => f.write_str(if eq { " ==" } else { " !=" })?,
            IrKind::RecordEq(operands)
            | IrKind::RecordNe(operands)
            | IrKind::ListEq(operands)
            | IrKind::ListNe(operands) => {
                let eq = matches!(node.kind, IrKind::RecordEq(_) | IrKind::ListEq(_));
                f.write_str(if eq { " == " } else { " != " })?;
                self.shape(f, operands.shape)?;
            }
            IrKind::RecordSame(_) | IrKind::ListSame(_) | IrKind::ValueSame(_) => {
                f.write_str(" ===")?;
            }
            IrKind::RecordNotSame(_) | IrKind::ListNotSame(_) | IrKind::ValueNotSame(_) => {
                f.write_str(" !==")?;
            }
            IrKind::Is { test, slot, .. } => {
                if let Some(code) = self.ir.test(test) {
                    f.write_str(" ")?;
                    self.test(f, &code.test)?;
                }
                if slot != Slot::NONE {
                    self.slot(f, slot)?;
                }
            }
            IrKind::As { test, .. } => {
                if let Some(code) = self.ir.test(test) {
                    f.write_str(" ")?;
                    self.test(f, &code.test)?;
                }
            }
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
            IrKind::Update { path, .. } => {
                f.write_str(" ")?;
                self.update_path(f, path)?;
            }
            IrKind::MakeClosure { function, .. } | IrKind::MakeFuture { function, .. } => {
                write!(f, " {function}")?;
            }
            IrKind::Await { shape, .. } if shape != LazyShapeId::NONE => {
                f.write_str(" ")?;
                self.lazy_shape(f, shape)?;
            }
            IrKind::Arg { param, .. } | IrKind::LoadCapture(param, _) => write!(f, " {param}")?,
            IrKind::For { slot, parts, .. } => {
                self.slot(f, slot)?;
                // The iterable, the guard and the body.
                if self.ir.items(parts).len() == 3 {
                    f.write_str(" if")?;
                }
            }
            IrKind::Break { loops, .. } | IrKind::Continue { loops, .. } => {
                write!(f, " {loops}")?;
            }
            IrKind::Return { to, .. } => write!(f, " {to}")?,
            IrKind::Catch { id, .. } => write!(f, " {id}")?,
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
            let head = match code.kind {
                FnKind::Function => "fn",
                FnKind::Memo => "memo fn",
                FnKind::Future => "future",
            };
            write!(f, "\n{head} #{index}")?;
            if let Some(name) = code.name {
                write!(f, " {}", FieldName(self.names.get(name)))?;
            }
            for (at, &segment) in self.ir.path_of(index).iter().enumerate() {
                let before = if at == 0 { " " } else { "." };
                write!(f, "{before}{}", FieldName(self.names.get(segment)))?;
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

    /// A program of every list node, all of one list, `[1, ...[1]]`, with
    /// `x` interned into `names`: the IR, and the list, the spread, an
    /// index, a length, the four comparisons (over a list of records
    /// compared over `[x]`, a list of values, and whole) and the root, a
    /// block whose result is a deep await of a list of lists.
    fn list_program(names: &mut Names) -> (Ir, [IrId; 9]) {
        let x = names.intern("x");
        let mut builder = IrBuilder::with_capacity(12);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(1, 2));
        let inner_items = builder.push_item_list(&[one]);
        let inner = builder.alloc(
            IrKind::MakeList {
                items: inner_items,
                spreads: false,
            },
            Span::new(7, 10),
        );
        let spread = builder.alloc(IrKind::Spread(inner), Span::new(4, 10));
        let items = builder.push_item_list(&[one, spread]);
        let list = builder.alloc(
            IrKind::MakeList {
                items,
                spreads: true,
            },
            Span::new(0, 11),
        );
        let index = builder.alloc(
            IrKind::Index {
                list,
                index: one,
                kind: ValueKind::Int,
            },
            Span::new(0, 14),
        );
        let len = builder.alloc(IrKind::ListLen(list), Span::new(0, 16));
        // A list of records compared over `[x]`, and one of values.
        let record = builder.push_shape(Shape {
            fields: Box::new([(x, None)]),
        });
        let records = builder.push_shape(Shape {
            fields: Box::default(),
        });
        builder.set_list_shape(
            records,
            ListShape {
                element: Some(record),
            },
        );
        let values = builder.push_shape(Shape {
            fields: Box::default(),
        });
        builder.set_list_shape(values, ListShape { element: None });
        let shaped = |shape| ShapedOperands {
            shape,
            lhs: list,
            rhs: list,
        };
        let eq = builder.alloc(IrKind::ListEq(shaped(records)), Span::new(0, 8));
        let ne = builder.alloc(IrKind::ListNe(shaped(values)), Span::new(0, 8));
        let operands = Operands {
            lhs: list,
            rhs: list,
            op_start: 3,
        };
        let same = builder.alloc(IrKind::ListSame(operands), Span::new(0, 9));
        let not_same = builder.alloc(IrKind::ListNotSame(operands), Span::new(0, 9));
        // A deep await of a list of lists whose elements are awaited.
        let each = builder.push_lazy_list(LazyField::Await(LazyShapeId::NONE));
        let lists = builder.push_lazy_list(LazyField::Within(each));
        let await_ = builder.alloc(
            IrKind::Await {
                value: list,
                kind: ValueKind::List,
                shape: lists,
                depth: 0,
            },
            Span::new(0, 11),
        );
        let block_items = builder.push_item_list(&[index, len, eq, ne, same, not_same]);
        let root = builder.alloc(
            IrKind::Block {
                items: block_items,
                result: await_,
            },
            Span::new(0, 16),
        );
        let ir = builder.finish(root, 0, Vec::new());
        (ir, [list, spread, index, len, eq, ne, same, not_same, root])
    }

    #[test]
    fn list_nodes_have_kinds_symbols_and_shapes() {
        let (ir, [list, spread, index, len, eq, ne, same, not_same, root]) =
            list_program(&mut Names::new());
        for (id, kind) in [
            (list, ValueKind::List),
            (spread, ValueKind::List),
            (index, ValueKind::Int),
            (len, ValueKind::Int),
            (eq, ValueKind::Bool),
            (ne, ValueKind::Bool),
            (same, ValueKind::Bool),
            (not_same, ValueKind::Bool),
            (root, ValueKind::List),
        ] {
            assert_eq!(ir.kind_of(id), kind, "{:?}", ir.get(id));
        }
        let symbols = [eq, ne, same, not_same, index, len].map(|id| ir.get(id).kind.symbol());
        assert_eq!(
            symbols,
            [Some("=="), Some("!="), Some("==="), Some("!=="), None, None]
        );
        assert_eq!(ir.get(eq).operator_span(), None);
        assert_eq!(ir.get(same).operator_span(), Some(Span::new(3, 6)));
        let IrKind::ListEq(ShapedOperands { shape: records, .. }) = ir.get(eq).kind else {
            panic!("expected a list comparison");
        };
        let Some(ListShape {
            element: Some(record),
        }) = ir.list_shape(records)
        else {
            panic!("expected a list's shape");
        };
        assert_eq!(ir.list_shape(record), None);
        assert_eq!(ir.shape(record).fields.len(), 1);
        assert_eq!(ir.shape_count(), 3);
        let IrKind::Await { shape: lists, .. } = ir.get(root_result(&ir, root)).kind else {
            panic!("expected an await");
        };
        let Some(LazyField::Within(each)) = ir.lazy_list(lists) else {
            panic!("expected a list's lazy shape");
        };
        assert_eq!(
            ir.lazy_list(each),
            Some(LazyField::Await(LazyShapeId::NONE))
        );
        assert_eq!(ir.lazy_shape_count(), 2);
    }

    /// The result of the block `root` of `ir`.
    fn root_result(ir: &Ir, root: IrId) -> IrId {
        match ir.get(root).kind {
            IrKind::Block { result, .. } => result,
            other => panic!("expected a block, got {other:?}"),
        }
    }

    #[test]
    fn list_nodes_print() {
        let mut names = Names::new();
        let (ir, _) = list_program(&mut names);
        // Each node shows its details and its children; the list, shared by
        // every node here, the same each time.
        let shown = ir.pretty(&names).to_string();
        let list = "(make-list @0..11\n\
                    \x20     (int 1 @1..2)\n\
                    \x20     (spread @4..10\n\
                    \x20       (make-list @7..10\n\
                    \x20         (int 1 @1..2))))";
        for node in [
            format!("\n  (index @0..14\n    {list}\n    (int 1 @1..2))\n"),
            format!("\n  (list-len @0..16\n    {list})\n"),
            format!("\n  (list-eq == *[x] @0..8\n    {list}\n    {list})\n"),
            format!("\n  (list-eq != * @0..8\n    {list}\n"),
            format!("\n  (list-same === @0..9\n    {list}\n"),
            format!("\n  (list-same !== @0..9\n    {list}\n"),
            format!("\n  (await **! @0..11\n    {list}))"),
        ] {
            assert!(shown.contains(&node), "{node}: {shown}");
        }
    }

    #[test]
    fn ranges_and_fors_have_kinds_children_and_print() {
        // `for i in 0..3 if b { i }` and `for-each` of it over `0..=3`, by
        // hand, with `i` in slot 0 and `b` in slot 1.
        let mut names = Names::new();
        let slot_names = vec![names.intern("i"), names.intern("b")];
        let mut builder = IrBuilder::with_capacity(12);
        let zero = builder.alloc(IrKind::IntConst(0), Span::new(9, 10));
        let three = builder.alloc(IrKind::IntConst(3), Span::new(12, 13));
        let operands = |op_start| Operands {
            lhs: zero,
            rhs: three,
            op_start,
        };
        let range = builder.alloc(IrKind::Range(false, operands(10)), Span::new(9, 13));
        let inclusive = builder.alloc(IrKind::Range(true, operands(10)), Span::new(9, 14));
        let guard = builder.alloc(
            IrKind::Load(Slot::new(1), ValueKind::Bool),
            Span::new(17, 18),
        );
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Int),
            Span::new(21, 22),
        );
        let no_items = builder.push_item_list(&[]);
        let body = builder.alloc(
            IrKind::Block {
                items: no_items,
                result: load,
            },
            Span::new(19, 24),
        );
        let parts = builder.push_item_list(&[range, guard, body]);
        let collecting = builder.alloc(
            IrKind::For {
                slot: Slot::new(0),
                parts,
                collect: true,
            },
            Span::new(0, 24),
        );
        let parts = builder.push_item_list(&[inclusive, body]);
        let discarding = builder.alloc(
            IrKind::For {
                slot: Slot::new(0),
                parts,
                collect: false,
            },
            Span::new(0, 25),
        );
        let block_items = builder.push_item_list(&[discarding]);
        let root = builder.alloc(
            IrKind::Block {
                items: block_items,
                result: collecting,
            },
            Span::new(0, 25),
        );
        let ir = builder.finish(root, 2, slot_names);
        for (id, kind) in [
            (range, ValueKind::List),
            (inclusive, ValueKind::List),
            (collecting, ValueKind::List),
            (discarding, ValueKind::Record),
            (root, ValueKind::List),
        ] {
            assert_eq!(ir.kind_of(id), kind, "{:?}", ir.get(id));
        }
        let symbols = [range, inclusive, collecting].map(|id| ir.get(id).kind.symbol());
        assert_eq!(symbols, [Some(".."), Some("..="), None]);
        assert_eq!(ir.get(range).operator_span(), Some(Span::new(10, 12)));
        assert_eq!(ir.get(inclusive).operator_span(), Some(Span::new(10, 13)));
        assert_eq!(ir.get(collecting).operator_span(), None);
        // A `for`'s children are its parts, and a range's its bounds.
        assert_eq!(
            ir.children(collecting).collect::<Vec<_>>(),
            vec![range, guard, body]
        );
        assert_eq!(ir.children(range).collect::<Vec<_>>(), vec![zero, three]);
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(block @0..25\n\
             \x20 (for-each i#0 @0..25\n\
             \x20   (range-inclusive @9..14\n\
             \x20     (int 0 @9..10)\n\
             \x20     (int 3 @12..13))\n\
             \x20   (block @19..24\n\
             \x20     (load i#0 @21..22)))\n\
             \x20 (for i#0 if @0..24\n\
             \x20   (range @9..13\n\
             \x20     (int 0 @9..10)\n\
             \x20     (int 3 @12..13))\n\
             \x20   (load b#1 @17..18)\n\
             \x20   (block @19..24\n\
             \x20     (load i#0 @21..22))))"
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

    #[test]
    fn a_dotted_update_shows_its_path_and_its_parts() {
        // `r.a[i]."c d".e := 1` by hand, its record and the element awaited
        // and `"c d"` created, which no checked program does together.
        let mut names = Names::new();
        let (a, spaced, e) = (names.intern("a"), names.intern("c d"), names.intern("e"));
        let mut builder = IrBuilder::with_capacity(6);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(23, 24));
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Deferred),
            Span::new(0, 1),
        );
        let root = builder.alloc(
            IrKind::Await {
                value: load,
                kind: ValueKind::Record,
                shape: LazyShapeId::NONE,
                depth: 0,
            },
            Span::new(0, 1),
        );
        let i = builder.alloc(IrKind::Load(Slot::new(1), ValueKind::Int), Span::new(4, 5));
        let parts = builder.push_item_list(&[one, root, i]);
        let step = |at, mode, end| UpdateStep {
            at,
            mode,
            span: Span::new(0, end),
        };
        let path = builder.push_update_path(&[
            step(StepAt::Field(a), StepMode::Read, 3),
            step(StepAt::Element, StepMode::Await, 6),
            step(StepAt::Field(spaced), StepMode::Create, 12),
            step(StepAt::Field(e), StepMode::Set, 14),
        ]);
        let update = builder.alloc(
            IrKind::Update {
                parts,
                path,
                kind: ValueKind::Record,
                depth: 0,
            },
            Span::new(0, 14),
        );
        let store = builder.alloc(IrKind::Store(Slot::new(0), update), Span::new(0, 24));
        let items = builder.push_item_list(&[store]);
        let empty = builder.alloc(IrKind::Empty, Span::new(0, 24));
        let block = builder.alloc(
            IrKind::Block {
                items,
                result: empty,
            },
            Span::new(0, 24),
        );
        let ir = builder.finish(block, 2, vec![names.intern("r"), names.intern("i")]);
        assert_eq!(ir.kind_of(update), ValueKind::Record);
        assert_eq!(ir.get(update).kind.symbol(), None);
        assert_eq!(ir.get(update).operator_span(), None);
        assert_eq!(ir.children(update).collect::<Vec<_>>(), vec![one, root, i]);
        let steps = ir.update_path(path).unwrap();
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[1].mode, StepMode::Await);
        assert_eq!(ir.update_path(UpdatePathId(1)), None);
        // Its depth is set as an await's: the block (0), the store (1), the
        // update (2); and the await of its record is below it (3).
        let IrKind::Update { depth, .. } = ir.get(update).kind else {
            panic!("expected an update");
        };
        assert_eq!(depth, 2);
        assert_eq!(ir.awaits_in(block), vec![(3, Span::new(0, 1))]);
        assert_eq!(
            ir.pretty(&names).to_string(),
            "(block @0..24\n\
             \x20 (store r#0 @0..24\n\
             \x20   (update .a[]!.\"c d\"+.e @0..14\n\
             \x20     (int 1 @23..24)\n\
             \x20     (await @0..1\n\
             \x20       (load r#0 @0..1))\n\
             \x20     (load i#1 @4..5)))\n\
             \x20 (empty @0..24))"
        );
    }

    #[test]
    fn a_dotted_update_is_two_levels_taller_for_each_step() {
        // A function whose body is the update: its height is one above its
        // tallest part, and two for each step.
        let mut builder = IrBuilder::with_capacity(2);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
        let parts = builder.push_item_list(&[one, one]);
        let path = builder.push_update_path(&[UpdateStep {
            at: StepAt::Element,
            mode: StepMode::Set,
            span: Span::new(0, 3),
        }]);
        let update = builder.alloc(
            IrKind::Update {
                parts,
                path,
                kind: ValueKind::List,
                depth: 0,
            },
            Span::new(0, 3),
        );
        let code = FnCode {
            body: update,
            kind: FnKind::Function,
            frame_size: 0,
            param_count: 0,
            capture_count: 0,
            height: 0,
            name: None,
            display: Box::from("fn(): {}"),
            slot_names: Box::default(),
        };
        let mut functions = vec![code];
        builder.measure(update, &mut functions);
        assert_eq!(functions[0].height, 4);
    }
}
