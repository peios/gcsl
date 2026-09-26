//! The type checker and name resolver.
//!
//! One pass over the tree both types every expression and resolves every
//! variable. Its results are side tables indexed by the AST's ids:
//! [`Types`], the type of every expression, and [`Resolution`], where every
//! variable use reads from and every declaration writes to, what each
//! function's frame and captures are, and which parameter each argument
//! fills. Both are returned together as [`Checked`]. Types are [`TyId`]s interned into a [`TypeTable`] the caller
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
//! # Dotted fields
//!
//! A field item whose path has several segments, `a.b.c = e`, declares the
//! field `a` as a record containing `b`, a record containing `c`. Dotted
//! definitions with a common prefix merge, as in TOML:
//!
//! ```text
//! server.host = "localhost"
//! server.port = 8080
//! server.tls.enabled = true
//! # one field, server = {host = ...; port = ...; tls = {enabled = true}}
//! ```
//!
//! A field whose first definition is dotted is *open*: it is one field
//! declaration and one binding, in one slot, and each further dotted path
//! under it extends it. The binding's type grows with each extension, so
//! after `server.host = "..."` the name `server` has type `{host: String}`
//! from the next item on, and after the next line `{host: String, port:
//! Int}`; its slot holds the record built so far. The record's fields are in
//! the order they were first mentioned, at every level, and the field takes
//! its place in its block where it was first mentioned.
//!
//! The rules, checked in the block declaring the field, each reported at the
//! offending path with a label on the earlier definition it conflicts with:
//!
//! 1. A field defined whole (`a = e`) cannot be extended: a later dotted
//!    path under it is "field `a` is already defined". Nor can a segment
//!    whose value a dotted path defined whole as a record (`a.b = { x = 1 }`,
//!    then `a.b.y = 2`: "field `a.b` is already defined"). A whole
//!    definition after a dotted one is "field `a` is defined twice", as for
//!    any field defined twice.
//! 2. A segment along the path that exists as some other value cannot hold
//!    fields: "cannot define `a.b.c`: `a.b` is Int, not a record".
//! 3. The path's last segment must not exist yet: "field `a.b` is defined
//!    twice".
//!
//! A rejected path declares nothing, but its value is still checked, and
//! its block's record is erroneous, so that the mistake does not cascade.
//!
//! To check these, the checker keeps a small tree of the segments of each
//! open field (`Node`), in one arena for the whole checker that each block
//! truncates back when it ends, as it does its field declarations. After an
//! extension, the types of the segments along its path are interned again,
//! from the leaf outwards.
//!
//! In a record block checked against an expected record type, a dotted
//! field's value is checked against the type the expected type has at the
//! value's full path, as a field's value is against its field's type, if it
//! has one there. If it lacks a segment of the path, the value is checked
//! on its own and the missing segment is warned about as an extra field, at
//! its first definition only. When the block ends, each open field is
//! checked against the expected type at every level of nesting: a missing
//! field is an error, "missing field `a.x` in record", and a segment that
//! the expected type has as something other than a record is a mismatch.
//!
//! # Dotted `let` and `:=`
//!
//! `let a.b.c = e` is a functional update. If no `a` is in scope, it
//! declares `a` as `{b = {c = e}}`. If `a` is in scope with a record type,
//! it declares a new binding `a`, shadowing the old one as any `let` does,
//! whose value is the old value with the path set: each segment between the
//! first and the last that the old value's type has as a record is updated
//! inside, each it lacks is created as a record, and the last is replaced
//! if the type has it (its type may change) or added. An annotation applies
//! to the last segment: `let a.b: Int = 5`. A segment along the way that is
//! not a record, or `a` itself not being one, is an error: "cannot update
//! `a.b.c`: `a.b` is Int, not a record". The new type is computed by walking
//! the old one along the path and interning the updated record types from
//! the leaf outwards.
//!
//! `a.b.c := e` is an update too, but `:=` is assignment-compatible: `a`
//! must be something `:=` may rebind, every segment of the path must
//! already exist (`:=` never adds a field: "cannot rebind `a.b.c`: `a.b`
//! has no field `c`"), and `e` must fit the leaf's type, a subtype of it or
//! an adapting literal, which the new binding keeps. Changing a variable's
//! type, or adding a field, takes a `let`.
//!
//! The old value is never changed. An update builds a new record, sharing
//! the old one's unchanged fields, so another binding holding the old value
//! still sees the old record:
//!
//! ```text
//! let a = { x = 1 }
//! let old = a
//! let a.x = 2      # a is {x = 2}; old is still {x = 1}
//! ```
//!
//! The static type is the truth. An update that creates an intermediate
//! record where the static type has no such field replaces whatever the
//! runtime value holds there: a field hidden by subtyping, as in
//! `let p: {x: Int} = { x = 1; y = 2 }` followed by `let p.y.z = 3`, whose
//! `y` becomes `{z = 3}`.
//!
//! How a dotted item resolves (the slot of the record it updates, and how
//! many of its path's segments name records that already exist) is
//! recorded for lowering as a [`PathResolution`].
//!
//! # Equality
//!
//! `==` and `!=` are *typed* equality, and `===` and `!==` *total*
//! equality. Both kinds need operands of one identical type. On scalars
//! they are the same. On records, typed equality compares only the fields
//! of the operands' static type, and of the static types of their nested
//! records, so after `let q: {x: Int} = { x = 1; y = 2 }`, `q == { x = 1 }`
//! is `true`; total equality compares whole runtime values, deeply and by
//! name, so `q === { x = 1 }` is `false`. The checker records nothing for
//! either: lowering reads the operands' type from the type table.
//!
//! Functions have no equality: once the operands are found to have one
//! identical type, a type that is a function type, or a record type
//! containing one at any depth (see [`TypeTable::contains_fn`]), is an
//! error. Nor do they have an order.
//!
//! # Functions
//!
//! A function expression's type is known before its body is checked, since
//! every parameter and the return type are declared: its parameter and
//! return types are resolved and its function type interned first, and then
//! the body is checked as a block whose scope starts with the parameters,
//! against the return type as an annotated initialiser is checked against
//! its annotation. The parameters are bindings like any other, so a `let`
//! may shadow one and a `:=` rebind one.
//!
//! **Frames.** Each function runs in a frame of its own. The checker keeps a
//! stack of the functions it is inside (`Function`); entering one saves the
//! enclosing frame's slot allocation and starts a fresh one at slot 0, in
//! which the parameters take the first slots, and leaving it records the
//! function's frame size and restores the enclosing allocation. The
//! top-level body is the outermost frame. What the checker learns about each
//! function is recorded in [`Resolution`] as a [`FnResolution`], indexed by
//! its [`FnId`].
//!
//! **Captures.** A use of a name resolves to a [`VarRef`]: a slot of the
//! current frame, a captured value, or the function being called. `lookup`
//! finds the binding as for any variable; if it lies below the innermost
//! function's first binding, it belongs to an enclosing function (or the
//! top level), and the use is a capture. Every function between the
//! binding's home and the use captures it: the outermost of them copies it
//! from its home frame (its slot, or the home function's callee), and each
//! function nested in that one copies it from the captures of the function
//! enclosing it, so the use reads its own function's capture. This is the
//! upvalue chain of Lua and the free-variable cells of Python, except that
//! there are no cells: each closure holds copies, made when its function
//! expression is evaluated. Capturing by value is the only choice consistent
//! with `:=`, which declares a new binding rather than assigning to the old
//! one: after `let x = 1; let f = fn(): Int { x }; x := 2`, the `x` that `f`
//! uses is the first binding, whose value is 1 whenever it is read, so `f()`
//! is 1. A capture list is searched linearly, since it is short.
//!
//! **Self-reference.** A function expression that is the direct initialiser
//! of a single-name `let`, field or named function item (not through
//! parentheses) sees that name throughout its body as the function being
//! called: a binding declared before the parameters, so a parameter of the
//! same name shadows it, of kind `BindingKind::Callee`, with no slot. It
//! resolves to [`VarRef::Callee`], which the evaluator serves from the
//! closure being run, so a recursive closure never holds a reference to
//! itself and reference counts stay free of cycles. It cannot be rebound
//! with `:=`, and never reaches a session's [`Scope`]. Functions are
//! visible only from the item after their own, so there is no mutual
//! recursion or forward reference.
//!
//! **Calls.** The callee must have a function type. Positional arguments
//! fill its parameters left to right, the trailing block fills the last,
//! and named arguments fill the parameters of their names; each argument
//! is then checked against its parameter's type as an annotated
//! initialiser is. The parameter each argument fills is recorded for
//! lowering in [`Resolution::arg_params`]. Too many positional arguments, an
//! unknown or unusable name, a parameter filled twice and parameters left
//! unfilled are each reported, and make the call erroneous.
//!
//! **Purity.** A function declared with `:` is pure, and one declared with
//! `->` impure; the top level is impure, and a block has the purity of the
//! function it is in. Calling an impure function from a pure one is an
//! error. Calling is the only gated operation: a pure function may create,
//! pass, store and return impure functions.
//!
//! # Scopes
//!
//! Every block, including the file's own body, is a scope. A `let` declares
//! a variable visible from the next item to the end of its block; a later
//! `let` of the same name shadows it, in the same block or an inner one. A
//! `:=` is a new declaration too, allowed only for a name already declared
//! in the same block, and not for a field; its value must fit the
//! variable's type (a subtype, or an adapting literal), and the new binding
//! keeps that type, so `:=` reads as assignment: a `let` is needed to
//! change a variable's type.
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
//! Every variable and field is stored in a [`Slot`] of its function's flat
//! frame (the top level's, outside functions), assigned stack-fashion, as
//! in Lua and clox: entering a block records the next free slot; each
//! declaration (a parameter, a `let`, a field, and `:=`, which is a new
//! binding) takes the next free slot; leaving the block restores the next
//! free slot to what it was on entry, so the slots of a finished block are
//! reused by whatever comes after it. The highest slot count reached is the
//! frame size. A variable's slot is therefore known statically and never
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
    Annotate, ArgKind, ArgRange, Ast, BinaryOp, Expr, ExprId, ExprKind, FnDecl, FnId, ItemId,
    ItemKind, ItemRange, PartRange, PathRange, PathSegment, StrPart, TypeExprId, TypeExprKind,
    UnaryOp, path_span,
};
use crate::diagnostic::{self, Diagnostic, InternalError};
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::{FnTy, TyId, TyKind, TypeTable};

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

/// Where a use of a variable reads its value from. See the module
/// documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VarRef {
    /// A slot of the current function's frame, or of the top level's.
    Local(Slot),
    /// One of the values the current closure captured, by its index in the
    /// function's capture order.
    Capture(u32),
    /// The closure being called: a function's reference to itself.
    Callee,
    /// Nothing: the expression or item is not a variable use.
    None,
}

/// What the checker learns about one function expression, in
/// [`Resolution::function`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FnResolution {
    /// The number of slots the body's frame needs, its parameters included.
    pub frame_size: u32,
    /// The number of parameters, which take slots `0..param_count`.
    pub param_count: u32,
    /// What the function captures, in capture order: where each value is
    /// read from in the frame that creates the closure, with its type.
    pub captures: Vec<(VarRef, TyId)>,
    /// The name the function sees itself by, if it is the direct
    /// initialiser of a single-name `let`, field or named function item.
    pub callee: Option<Name>,
    /// For each slot of the body's frame, the name of the last variable
    /// assigned to it. Only for showing slots to people.
    pub slot_names: Vec<Name>,
}

/// How the path of a dotted `let`, field or `:=` resolves: the record it
/// updates, and how far into its path that record's static type reaches.
/// See the module documentation.
///
/// For `let a.b.c = e`, with `a` in scope as `{b: {c: Int}}`, the source is
/// where `a` is read from and all of `a` and `a.b` exist, so `existing` is
/// 2; with `a` as `{x: Int}`, only `a` exists, and `existing` is 1; with no
/// `a`, there is no source and `existing` is 0. A dotted field's first
/// definition has no source; each later one has the field's own slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathResolution {
    /// Where the record the item updates is read from, as a use of its
    /// variable would read it (a slot, or a captured value), or
    /// [`VarRef::None`] if it builds its record from nothing.
    pub source: VarRef,
    /// How many of the path's leading segments name records that exist in
    /// the static type of the updated value: 0 without a source, and
    /// otherwise 1 for the source itself and one more for each following
    /// segment, short of the last, that the type has as a record. Every
    /// segment after them, the last excepted, is created as a record.
    pub existing: u32,
}

impl PathResolution {
    /// The resolution of an item that is not a dotted declaration, and of
    /// one that builds its record from nothing.
    pub const NONE: PathResolution = PathResolution {
        source: VarRef::None,
        existing: 0,
    };
}

/// Where every variable of an [`Ast`] is stored, what every function's
/// frame and captures are, and which parameter every argument fills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// Where each `Name` expression reads, indexed by [`ExprId`];
    /// [`VarRef::None`] for every other expression.
    expr_vars: Vec<VarRef>,
    /// The slot each `Let`, `Field` and `Rebind` item writes, indexed by
    /// [`ItemId`]; [`Slot::NONE`] for every other item. Every definition of
    /// an open field writes the field's one slot.
    item_slots: Vec<Slot>,
    /// How the path of each dotted `Let`, `Field` and `Rebind` item
    /// resolves, indexed by [`ItemId`]; [`PathResolution::NONE`] for every
    /// other item, and for one that builds its record from nothing.
    item_paths: Vec<PathResolution>,
    /// The number of slots the top level's frame needs.
    frame_size: u32,
    /// For each slot of the top level's frame, the name of the last
    /// variable assigned to it.
    slot_names: Vec<Name>,
    /// What the checker found for each function expression, indexed by
    /// [`FnId`].
    functions: Vec<FnResolution>,
    /// The index of the parameter each argument fills, indexed like the
    /// [`Ast`]'s argument pool; `u32::MAX` for an argument of an erroneous
    /// call.
    arg_params: Vec<u32>,
}

impl Resolution {
    /// Where expression `id` reads its value from, or [`VarRef::None`] if it
    /// is not a variable use.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn expr_var(&self, id: ExprId) -> VarRef {
        self.expr_vars[id.index()]
    }

    /// What the checker found for function expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn function(&self, id: FnId) -> &FnResolution {
        &self.functions[id.index()]
    }

    /// The parameter each argument of `args`, a call's arguments, fills,
    /// in the order of [`Ast::args`]; `u32::MAX` for an argument of an
    /// erroneous call.
    ///
    /// # Panics
    ///
    /// Panics if `args` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn arg_params(&self, args: ArgRange) -> &[u32] {
        &self.arg_params[args.start()..args.start() + args.count()]
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

    /// How the path of item `id` resolves: [`PathResolution::NONE`] if it
    /// is not a dotted declaration or builds its record from nothing.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn item_path(&self, id: ItemId) -> PathResolution {
        self.item_paths[id.index()]
    }

    /// The number of slots the top level's frame needs: one more than the
    /// highest slot used.
    #[must_use]
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// For each slot of the top level's frame, the name of the last
    /// variable assigned to it. Only for showing slots to people, as
    /// `--dump-ir` does.
    #[must_use]
    pub fn slot_names(&self) -> &[Name] {
        &self.slot_names
    }

    /// A resolution for a tree with `exprs` expressions and `items` items and
    /// no variables, functions or calls, for building checked trees by hand
    /// in tests.
    #[cfg(test)]
    pub(crate) fn without_variables(exprs: usize, items: usize) -> Self {
        Self {
            expr_vars: vec![VarRef::None; exprs],
            item_slots: vec![Slot::NONE; items],
            item_paths: vec![PathResolution::NONE; items],
            frame_size: 0,
            slot_names: Vec::new(),
            functions: Vec::new(),
            arg_params: Vec::new(),
        }
    }

    /// Forgets the parameter of every argument, as for an erroneous call,
    /// for testing what lowering makes of that.
    #[cfg(test)]
    pub(crate) fn unbind_arguments(&mut self) {
        self.arg_params.fill(u32::MAX);
    }

    /// Number of expression entries, equal to the number of expressions in
    /// the [`Ast`].
    pub(crate) fn expr_len(&self) -> usize {
        self.expr_vars.len()
    }

    /// Number of function entries, equal to the number of function
    /// expressions in the [`Ast`].
    pub(crate) fn function_len(&self) -> usize {
        self.functions.len()
    }

    /// Number of argument entries, equal to the number of arguments in the
    /// [`Ast`].
    pub(crate) fn arg_len(&self) -> usize {
        self.arg_params.len()
    }

    /// Number of item entries in both item tables, equal to the number of
    /// items in the [`Ast`].
    pub(crate) fn item_len(&self) -> usize {
        self.item_slots.len().min(self.item_paths.len())
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

/// A variable, field or callee in scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    name: Name,
    /// Its slot in its function's frame; [`Slot::NONE`] for a callee.
    slot: Slot,
    ty: TyId,
    /// Where it was declared: the span of the `let`, field or `:=` item, or
    /// of the parameter, or of the function expression for a callee; `None`
    /// if it was declared by an earlier input of a session, whose source is
    /// not this one.
    declared: Option<Span>,
    kind: BindingKind,
}

/// What a [`Binding`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    /// A variable: declared by a `let`, a `:=` or a parameter.
    Variable,
    /// A field, which `:=` may not rebind.
    Field,
    /// A function's name for itself, inside its own body, which `:=` may
    /// not rebind either; it has no slot.
    Callee,
}

/// A function expression being checked: one entry of the checker's stack
/// of the functions it is inside, innermost last.
#[derive(Debug)]
struct Function {
    fn_id: FnId,
    /// The index in `Checker::bindings` of the function's first binding:
    /// every binding below it belongs to an enclosing function or the top
    /// level, and must be captured to be used.
    bindings_start: usize,
    /// The enclosing frame's slot allocation, restored on exit.
    outer_next_slot: u32,
    outer_frame_size: u32,
    outer_slot_names: Vec<Name>,
    /// What the function captures, in capture order.
    captures: Vec<Capture>,
    /// Whether the function is pure, declared with `:`.
    pure: bool,
    /// Its `:` or `->`.
    marker: Span,
}

/// One value a function captures.
#[derive(Clone, Copy, Debug)]
struct Capture {
    /// The index in `Checker::bindings` of the binding captured.
    binding: usize,
    /// Where the value is read from in the frame creating the closure.
    source: VarRef,
    /// Its type.
    ty: TyId,
}

/// A field declared in the record block being checked.
#[derive(Clone, Copy, Debug)]
struct FieldDecl {
    name: Name,
    /// Where the name is written in the field's first definition.
    name_span: Span,
    ty: TyId,
    /// For an open field, one first defined by a dotted path, where its
    /// segments are tracked; `None` for a field defined whole.
    open: Option<OpenField>,
    /// Whether a dotted path under it was rejected, which makes its block's
    /// record erroneous, so that the rejection does not cascade.
    rejected: bool,
}

/// An open field: a record built by dotted paths.
#[derive(Clone, Copy, Debug)]
struct OpenField {
    /// Its root node in `Checker::nodes`.
    node: u32,
    /// The index of its binding in `Checker::bindings`, whose type grows
    /// with it.
    binding: usize,
}

/// The index of no node, where a node has no parent, child or sibling.
const NO_NODE: u32 = u32::MAX;

/// What a [`Node`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    /// A record being built by dotted paths, which further paths may
    /// extend: an open field, or a segment before the last of a dotted
    /// path.
    Open,
    /// The value at the end of a dotted path, defined whole.
    Leaf,
}

/// One segment of an open field, in `Checker::nodes`: the field itself, or
/// a segment within it. Its children are a linked list through
/// `first_child` and `next_sibling`, most recent first.
#[derive(Clone, Copy, Debug)]
struct Node {
    name: Name,
    /// The path from the field's name through this segment, where the item
    /// that first defined the segment writes it: where diagnostics about it
    /// point.
    span: Span,
    /// Its type: for an open node the record of its children's types, or
    /// `Error` if any of them is; for a leaf its value's.
    ty: TyId,
    kind: NodeKind,
    parent: u32,
    first_child: u32,
    next_sibling: u32,
}

/// The record type a record block is checked against, and where the
/// expectation comes from, for [`Checker::check_expr_expecting`].
#[derive(Clone, Copy, Debug)]
struct Expecting<'w> {
    /// The expected record type.
    ty: TyId,
    /// The annotation, ascription, return type or callee stating it.
    context: Span,
    /// The label on `context`.
    why: &'w str,
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
    expr_vars: Vec<VarRef>,
    item_slots: Vec<Slot>,
    item_paths: Vec<PathResolution>,
    /// What has been found for each function expression, indexed by
    /// [`FnId`].
    fn_resolutions: Vec<FnResolution>,
    /// The parameter each argument fills, indexed like the argument pool.
    arg_params: Vec<u32>,
    /// Every variable, field and callee in scope, outermost block first.
    bindings: Vec<Binding>,
    /// The functions being checked, innermost last; empty at the top
    /// level.
    functions: Vec<Function>,
    /// The name the next function expression checked sees itself by: set by
    /// a single-name `let` or field whose initialiser is directly a
    /// function expression, and taken by that function when its check
    /// starts, so no function nested in it sees it.
    callee_name: Option<Name>,
    /// Reused for building function types: each parameter's name and type.
    /// Nested uses push above the entries of the use enclosing them and
    /// truncate back, as `sorted_fields` does.
    fn_params: Vec<(Option<Name>, TyId)>,
    /// Reused for binding a call's arguments: for each parameter, the index
    /// in the call's arguments of the argument filling it, or `u32::MAX`.
    filled: Vec<u32>,
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
    /// Also where the fields of an open node are gathered to intern its
    /// type.
    sorted_fields: Vec<(Name, u32, TyId)>,
    /// The segments of the open fields of every block being checked,
    /// outermost block first; each block truncates it back when it ends.
    nodes: Vec<Node>,
    /// Reused for the types of the records along the path of a dotted
    /// `let` or `:=`.
    path_types: Vec<TyId>,
    /// Reused as the work list of open nodes still to check against their
    /// expected types when a record block ends.
    pending: Vec<(u32, TyId)>,
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
            expr_vars: vec![VarRef::None; ast.len()],
            item_slots: vec![Slot::NONE; ast.item_count()],
            item_paths: vec![PathResolution::NONE; ast.item_count()],
            fn_resolutions: vec![FnResolution::default(); ast.fn_count()],
            arg_params: vec![u32::MAX; ast.arg_count()],
            bindings: scope.bindings,
            functions: Vec::new(),
            callee_name: None,
            fn_params: Vec::new(),
            filled: Vec::new(),
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
            nodes: Vec::new(),
            path_types: Vec::new(),
            pending: Vec::new(),
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
        // Spans are meaningless for a later input, which has its own
        // source.
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
                expr_vars: self.expr_vars,
                item_slots: self.item_slots,
                item_paths: self.item_paths,
                frame_size: self.frame_size,
                slot_names: self.slot_names,
                functions: self.fn_resolutions,
                arg_params: self.arg_params,
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
    fn check_block(&mut self, body: ItemRange, expecting: Option<Expecting<'_>>) -> TyId {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let nodes_start = self.nodes.len();
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        let ty = self.check_body(body, expecting);
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        self.nodes.truncate(nodes_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        ty
    }

    /// Checks the items of a body in the current scope, returning its type:
    /// for a record block, the record of its fields; otherwise the type of
    /// its result, the `= EXPR` or final expression statement, or `{}` if
    /// it has none.
    fn check_body(&mut self, body: ItemRange, expecting: Option<Expecting<'_>>) -> TyId {
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
                ItemKind::Let { path, ty, value } => self.check_let(id, path, ty, value),
                ItemKind::Rebind { path, value } => self.check_rebind(id, path, value),
                ItemKind::Field { path, value } => self.check_field(id, path, value, expecting),
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

    /// Checks `let PATH (: TYPE)? = VALUE` and declares the variable: with
    /// the value's type, or for a dotted path the type of the update.
    fn check_let(
        &mut self,
        id: ItemId,
        path: PathRange,
        annotation: Option<TypeExprId>,
        value: ExprId,
    ) {
        let expected = annotation.map(|annotation| {
            (
                self.resolve_type(annotation),
                self.ast.type_expr(annotation).span,
            )
        });
        let segments = self.ast.path(path);
        self.name_callee(segments, value);
        let ty = match expected {
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
            Some((expected, context)) => {
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
        self.callee_name = None;
        match segments {
            [variable] => self.declare(variable.name, ty, id, BindingKind::Variable),
            segments => self.declare_update(id, segments, ty),
        }
    }

    /// Before the initialiser `value` of a `let` or field whose path is
    /// `segments` is checked: if the path is a single name and `value` is
    /// directly a function expression, lets that function see itself by
    /// the name (see the module documentation). The caller clears the name
    /// afterwards, in case the function did not take it.
    fn name_callee(&mut self, segments: &[PathSegment], value: ExprId) {
        self.callee_name = match (segments, self.ast.get(value).kind) {
            ([single], ExprKind::Fn(_)) => Some(single.name),
            _ => None,
        };
    }

    /// Checks the field item `id`, `PATH = VALUE`, and declares or extends
    /// the field. With `expecting`, its block is checked against an
    /// expected record type: a field the type has is checked against that
    /// field's type and then has it, and one the type lacks is a warning.
    fn check_field(
        &mut self,
        id: ItemId,
        path: PathRange,
        value: ExprId,
        expecting: Option<Expecting<'_>>,
    ) {
        let segments = self.ast.path(path);
        let [field] = segments else {
            self.check_dotted_field(id, segments, value, expecting);
            return;
        };
        let (name, name_span) = (field.name, field.span);
        let expected = expecting.map(|expecting| (expecting, self.types.field(expecting.ty, name)));
        self.name_callee(segments, value);
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
                    .with_note(EXTRA_FIELD_NOTE),
                );
                ty
            }
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
        };
        self.callee_name = None;
        self.declare(name, ty, id, BindingKind::Field);
        self.fields.push(FieldDecl {
            name,
            name_span,
            ty,
            open: None,
            rejected: false,
        });
    }

    /// Checks the dotted field item `id`, `a.b.c = VALUE`, whose path is
    /// `segments`, and declares the open field `a` or extends it (see the
    /// module documentation). A path that breaks the rules is reported, and
    /// its value checked on its own, without declaring anything.
    fn check_dotted_field(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
        expecting: Option<Expecting<'_>>,
    ) {
        let Some(first) = segments.first() else {
            return;
        };
        // Where the path attaches to what the block already has: the field's
        // declaration and the deepest existing node along the path, with the
        // number of segments that exist.
        let declared = self.fields[self.fields_start..]
            .iter()
            .position(|field| field.name == first.name)
            .map(|index| index + self.fields_start);
        let attach = match declared {
            None => None,
            Some(index) => {
                let field = self.fields[index];
                let Some(open) = field.open else {
                    let shown = self.ast.show_path(&segments[..1]);
                    self.errors.push(
                        Diagnostic::error(
                            format!("field `{shown}` is already defined"),
                            path_span(segments),
                        )
                        .with_context(field.name_span, "defined whole here")
                        .with_note(WHOLE_FIELD_NOTE),
                    );
                    self.fields[index].rejected = true;
                    self.check_alone(value);
                    return;
                };
                let Some((node, existing)) = self.attach_point(open.node, segments) else {
                    self.fields[index].rejected = true;
                    self.check_alone(value);
                    return;
                };
                Some((index, open, node, existing))
            }
        };
        let existing = attach.map_or(0, |(_, _, _, existing)| existing);
        let expected = expecting.and_then(|expecting| {
            self.expected_at(expecting, segments, existing)
                .map(|ty| (expecting, ty))
        });
        let leaf_ty = match expected {
            Some((expecting, expected)) => {
                let found =
                    self.check_expr_expecting(value, expected, expecting.context, expecting.why);
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
            None => self.check_alone(value),
        };
        // The new nodes, from the first segment that does not exist yet.
        let (mut parent, root) = if let Some((_, open, node, _)) = attach {
            (node, open.node)
        } else {
            let root = self.add_node(NO_NODE, first.name, first.span, NodeKind::Open);
            (root, root)
        };
        let last = segments.len() - 1;
        for (index, segment) in segments.iter().enumerate().skip(existing.max(1)) {
            let span = path_span(&segments[..=index]);
            let kind = if index == last {
                NodeKind::Leaf
            } else {
                NodeKind::Open
            };
            parent = self.add_node(parent, segment.name, span, kind);
        }
        // `parent` is now the leaf. Its type is the value's, and the type of
        // each node above it changes.
        self.nodes[parent as usize].ty = leaf_ty;
        let mut node = self.nodes[parent as usize].parent;
        while node != NO_NODE {
            let ty = self.node_type(node);
            let entry = &mut self.nodes[node as usize];
            entry.ty = ty;
            node = entry.parent;
        }
        let ty = self.nodes[root as usize].ty;
        match attach {
            None => {
                self.declare(first.name, ty, id, BindingKind::Field);
                self.fields.push(FieldDecl {
                    name: first.name,
                    name_span: first.span,
                    ty,
                    open: Some(OpenField {
                        node: root,
                        binding: self.bindings.len() - 1,
                    }),
                    rejected: false,
                });
            }
            Some((index, open, _, existing)) => {
                self.fields[index].ty = ty;
                let binding = &mut self.bindings[open.binding];
                binding.ty = ty;
                self.item_slots[id.index()] = binding.slot;
                // The field is in this block, so in the current frame.
                self.item_paths[id.index()] = PathResolution {
                    source: VarRef::Local(binding.slot),
                    existing: position(existing),
                };
            }
        }
    }

    /// Finds where the path `segments`, whose first segment names the open
    /// field whose root node is `root`, attaches to that field: the deepest
    /// node along the path that exists, and the number of the path's
    /// segments that exist, at least 1 and short of all of them. Reports the
    /// path and returns `None` if it breaks a rule: a segment that exists as
    /// a leaf, or a last segment that exists at all.
    fn attach_point(&mut self, root: u32, segments: &[PathSegment]) -> Option<(u32, usize)> {
        let mut node = root;
        let last = segments.len() - 1;
        for (index, segment) in segments.iter().enumerate().skip(1) {
            let Some(child) = self.child(node, segment.name) else {
                return Some((node, index));
            };
            let found = self.nodes[child as usize];
            let prefix = self.ast.show_path(&segments[..=index]);
            let path = path_span(segments);
            if index == last {
                self.errors.push(
                    Diagnostic::error(format!("field `{prefix}` is defined twice"), path)
                        .with_context(found.span, "first defined here"),
                );
                return None;
            }
            match found.kind {
                NodeKind::Open => node = child,
                // Reported already.
                NodeKind::Leaf if found.ty == TyId::ERROR => return None,
                NodeKind::Leaf if self.types.is_record(found.ty) => {
                    self.errors.push(
                        Diagnostic::error(format!("field `{prefix}` is already defined"), path)
                            .with_context(found.span, "defined whole here")
                            .with_note(WHOLE_FIELD_NOTE),
                    );
                    return None;
                }
                NodeKind::Leaf => {
                    let shown = self.ast.show_path(segments);
                    let ty = self.in_message(found.ty);
                    self.errors.push(
                        Diagnostic::error(
                            format!("cannot define `{shown}`: `{prefix}` is {ty}, not a record"),
                            path,
                        )
                        .with_context(found.span, format!("`{prefix}` defined here")),
                    );
                    return None;
                }
            }
        }
        // The last segment either exists or does not, and both return
        // above.
        None
    }

    /// The type `expecting` expects at the end of the path `segments`, of
    /// which the first `existing` segments are defined already, or `None`
    /// if it has none there. Walking the expected record type along the
    /// path stops at a segment it does not have, which is warned about as
    /// an extra field if this path defines it first, or at a type that is
    /// not a record, which is reported as a mismatch when the block ends.
    fn expected_at(
        &mut self,
        expecting: Expecting<'_>,
        segments: &[PathSegment],
        existing: usize,
    ) -> Option<TyId> {
        let mut expected = expecting.ty;
        for (index, segment) in segments.iter().enumerate() {
            // Not a record: reported when the block ends.
            self.types.record_fields(expected)?;
            let Some(ty) = self.types.field(expected, segment.name) else {
                if index >= existing {
                    let text = self.ast.names().get(segment.name);
                    let record = self.in_message(expected);
                    self.warnings.push(
                        Diagnostic::warning(
                            format!("field `{}` is not in {record}", FieldName(text)),
                            segment.span,
                        )
                        .with_context(expecting.context, expecting.why)
                        .with_note(EXTRA_FIELD_NOTE),
                    );
                }
                return None;
            };
            expected = ty;
        }
        Some(expected)
    }

    /// Adds a node named `name`, at `span`, as the newest child of `parent`
    /// (or as a root, for [`NO_NODE`]), and returns its index. Its type is
    /// set by the caller.
    fn add_node(&mut self, parent: u32, name: Name, span: Span, kind: NodeKind) -> u32 {
        let index = position(self.nodes.len());
        let next_sibling = match self.nodes.get_mut(parent as usize) {
            Some(parent) => std::mem::replace(&mut parent.first_child, index),
            None => NO_NODE,
        };
        self.nodes.push(Node {
            name,
            span,
            ty: TyId::EMPTY,
            kind,
            parent,
            first_child: NO_NODE,
            next_sibling,
        });
        index
    }

    /// The child of `node` named `name`, if it has one.
    fn child(&self, node: u32, name: Name) -> Option<u32> {
        let mut child = self.nodes.get(node as usize)?.first_child;
        while let Some(entry) = self.nodes.get(child as usize) {
            if entry.name == name {
                return Some(child);
            }
            child = entry.next_sibling;
        }
        None
    }

    /// The type of the open node `node`: the record of its children's
    /// types, or `Error` if any of them is.
    fn node_type(&mut self, node: u32) -> TyId {
        let base = self.sorted_fields.len();
        let mut erroneous = false;
        let mut child = self
            .nodes
            .get(node as usize)
            .map_or(NO_NODE, |n| n.first_child);
        while let Some(entry) = self.nodes.get(child as usize) {
            erroneous |= entry.ty == TyId::ERROR;
            self.sorted_fields.push((entry.name, 0, entry.ty));
            child = entry.next_sibling;
        }
        let ty = if erroneous {
            TyId::ERROR
        } else {
            let fields = self.sorted_fields.get(base..).unwrap_or_default();
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        self.sorted_fields.truncate(base);
        ty
    }

    /// Checks `value` with no expected type, settling an untyped integer to
    /// `Int`, and returns its type.
    fn check_alone(&mut self, value: ExprId) -> TyId {
        let found = self.check_expr(value);
        self.settle(value, found)
    }

    /// The type of the record block whose fields are the innermost block's,
    /// once they are all declared: the record of the fields, or `Error` if
    /// a field is defined twice, has an erroneous type or had a dotted path
    /// under it rejected. With `expecting`,
    /// also reports each field of the expected type that the block lacks,
    /// at any level of an open field, and each open field or segment of one
    /// that the expected type has as something other than a record, which
    /// make the type `Error` too.
    fn record_type(&mut self, expecting: Option<Expecting<'_>>) -> TyId {
        let fields = self.fields.get(self.fields_start..).unwrap_or_default();
        let base = self.sorted_fields.len();
        self.sorted_fields.extend(
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| (field.name, position(index), field.ty)),
        );
        let mut erroneous = fields
            .iter()
            .any(|field| field.ty == TyId::ERROR || field.rejected);
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
            erroneous |= self.check_open_fields(expecting);
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
    fn report_missing_fields(&mut self, base: usize, expecting: Expecting<'_>) -> bool {
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

    /// Checks each open field of the innermost block that `expecting`'s
    /// record type has against the type it has there, at every level of
    /// nesting, through a work list rather than recursion. A segment is
    /// checked against a record type by its fields: each one it lacks is
    /// reported at the block's closing brace as "missing field `a.x` in
    /// record", and each open segment is checked in turn (a leaf was checked
    /// against its type when it was defined). A segment expected to be
    /// something other than a record is a mismatch, reported at its first
    /// definition. Returns whether anything was reported.
    fn check_open_fields(&mut self, expecting: Expecting<'_>) -> bool {
        let mut reported = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            let Some(open) = field.open else {
                continue;
            };
            // An erroneous field has been reported already.
            if field.ty == TyId::ERROR {
                continue;
            }
            let Some(expected) = self.types.field(expecting.ty, field.name) else {
                continue;
            };
            self.pending.push((open.node, expected));
            while let Some((node, expected)) = self.pending.pop() {
                reported |= self.check_open_node(node, expected, expecting);
            }
        }
        reported
    }

    /// Checks the open node `node` against the type `expected`, reporting a
    /// mismatch or each missing field, and queues its open children that
    /// the type has on `pending`. Returns whether anything was reported.
    fn check_open_node(&mut self, node: u32, expected: TyId, expecting: Expecting<'_>) -> bool {
        let entry = self.nodes[node as usize];
        // An unknown type has been reported, and a record type never has a
        // field of one, but nothing is to be checked against it either way.
        if expected == TyId::ERROR || entry.ty == TyId::ERROR {
            return false;
        }
        let Some(fields) = self.types.record_fields(expected) else {
            let label = format!(
                "expected {}, found {}",
                self.in_label(expected),
                self.in_label(entry.ty)
            );
            self.errors.push(
                Diagnostic::error("mismatched types", entry.span)
                    .with_label(label)
                    .with_context(expecting.context, expecting.why),
            );
            return true;
        };
        let names = self.ast.names();
        let mut missing: Vec<&str> = Vec::new();
        for &(name, ty) in fields {
            match self.child(node, name) {
                None => missing.push(names.get(name)),
                Some(child) if self.nodes[child as usize].kind == NodeKind::Open => {
                    self.pending.push((child, ty));
                }
                Some(_) => {}
            }
        }
        if missing.is_empty() {
            return false;
        }
        missing.sort_unstable();
        let path = self.node_path(node);
        let record = self.in_message(expected);
        for text in missing {
            self.errors.push(
                Diagnostic::error(
                    format!("missing field `{path}.{}` in record", FieldName(text)),
                    expecting.close,
                )
                .with_context(expecting.context, expecting.why)
                .with_note(format!("the type {record} needs it")),
            );
        }
        true
    }

    /// The path from its field's name to the node `node`, as it is
    /// written, for diagnostics.
    fn node_path(&self, node: u32) -> String {
        let names = self.ast.names();
        let mut texts = Vec::new();
        let mut current = node;
        while let Some(entry) = self.nodes.get(current as usize) {
            texts.push(FieldName(names.get(entry.name)).to_string());
            current = entry.parent;
        }
        texts.reverse();
        texts.join(".")
    }

    /// Checks `PATH := VALUE` and declares the new binding: with the value's
    /// type, or for a dotted path the type of the update. The binding is
    /// declared even when the rebind is rejected, so that later uses of the
    /// name do not cascade into "undefined variable" errors.
    fn check_rebind(&mut self, id: ItemId, path: PathRange, value: ExprId) {
        let segments = self.ast.path(path);
        let Some(&PathSegment {
            name,
            span: name_span,
        }) = segments.first()
        else {
            return;
        };
        let text = self.ast.names().get(name);
        // The binding being rebound, once the `:=` rules pass.
        let target = match self.lookup(name) {
            Some((_, binding)) if binding.kind == BindingKind::Field => {
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}`: it is a field, not a variable"),
                    name_span,
                );
                // A field from an earlier session input has no item here to
                // point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(declared, "declared here");
                }
                self.errors.push(error.with_note(format!(
                    "rebind with `{text} = ...` is not allowed either: a field is defined once"
                )));
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Callee => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: it is the function being defined"),
                        name_span,
                    )
                    .with_note("shadow it with `let` instead"),
                );
                None
            }
            Some((index, binding)) if index >= self.block_start => Some(binding),
            Some((_, binding)) => {
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}` here: it is declared in an enclosing block"),
                    name_span,
                );
                // A binding from an earlier session input has no item here
                // to point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(declared, "declared here");
                }
                let error = error.with_note(format!("shadow it with `let {text} = ...` instead"));
                self.errors.push(error);
                None
            }
            None => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: no variable named `{text}` is declared"),
                        name_span,
                    )
                    .with_note(format!("declare it with `let {text} = ...`")),
                );
                None
            }
        };
        let Some(target) = target.filter(|target| target.ty != TyId::ERROR) else {
            // The rebind was rejected (or the variable is already erroneous):
            // check the value on its own and declare the name with what it
            // has, so later uses do not cascade.
            let ty = self.check_alone(value);
            if segments.len() == 1 {
                self.declare(name, ty, id, BindingKind::Variable);
            } else {
                self.declare_update(id, segments, ty);
            }
            return;
        };
        // `:=` is assignment-compatible: the value must fit the variable's
        // type (or the path's leaf type), and the binding keeps that type.
        let context = target.declared.unwrap_or(name_span);
        if segments.len() == 1 {
            self.check_expr_expecting(value, target.ty, context, "the variable was declared here");
            self.declare(name, target.ty, id, BindingKind::Variable);
            return;
        }
        if let Some(leaf) = self.rebind_leaf(segments, target.ty) {
            self.check_expr_expecting(
                value,
                leaf,
                path_span(segments),
                "`:=` keeps the field's type",
            );
            self.declare_update(id, segments, leaf);
        } else {
            self.check_alone(value);
            self.declare(name, TyId::ERROR, id, BindingKind::Variable);
        }
    }

    /// The type of the leaf of the dotted `:=` path `segments`, whose first
    /// segment has record type `root`. Every segment must already exist,
    /// since `:=` never adds a field: a missing one, or a segment along the
    /// way that is not a record, is reported and `None` returned.
    fn rebind_leaf(&mut self, segments: &[PathSegment], root: TyId) -> Option<TyId> {
        let mut current = root;
        for (index, segment) in segments.iter().enumerate().skip(1) {
            if !self.types.is_record(current) {
                let message = format!(
                    "cannot update `{}`: `{}` is {}, not a record",
                    self.ast.show_path(segments),
                    self.ast.show_path(&segments[..index]),
                    self.in_message(current)
                );
                let label = format!("this is {}", self.in_label(root));
                self.errors.push(
                    Diagnostic::error(message, path_span(segments))
                        .with_context(segments[0].span, label),
                );
                return None;
            }
            let Some(ty) = self.types.field(current, segment.name) else {
                let path = self.ast.show_path(segments);
                let prefix = self.ast.show_path(&segments[..index]);
                let field = FieldName(self.ast.names().get(segment.name));
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{path}`: `{prefix}` has no field `{field}`"),
                        path_span(segments),
                    )
                    .with_context(segments[0].span, format!("this is {}", self.in_label(root)))
                    .with_note(format!(
                        "`:=` cannot add a field; add it with `let {path} = ...`"
                    )),
                );
                return None;
            };
            current = ty;
        }
        Some(current)
    }

    /// Declares the variable of the dotted `let` or `:=` item `id`, whose
    /// path is `segments` and whose value has type `leaf_ty`: a new binding
    /// of the path's first segment, `a`, whose type is that of `a` in scope,
    /// or of `{}` if there is none, with the rest of the path set to
    /// `leaf_ty` (see the module documentation). If `a` or a segment along
    /// the way is not a record, that is reported and the binding is
    /// `Error`.
    fn declare_update(&mut self, id: ItemId, segments: &[PathSegment], leaf_ty: TyId) {
        let Some((first, rest)) = segments.split_first() else {
            return;
        };
        self.path_types.clear();
        let found = self.lookup(first.name);
        let old = found.map(|(_, binding)| binding);
        let mut failed = leaf_ty == TyId::ERROR;
        if let Some(old) = old {
            if old.ty == TyId::ERROR {
                // Reported already.
                failed = true;
            } else if self.types.is_record(old.ty) {
                // The type of each record along the path that exists, from
                // `a` down; the last segment is never one of them.
                let mut current = old.ty;
                self.path_types.push(current);
                for (index, segment) in rest.iter().enumerate() {
                    if index + 1 == rest.len() {
                        break;
                    }
                    match self.types.field(current, segment.name) {
                        None => break,
                        Some(ty) if self.types.is_record(ty) => {
                            current = ty;
                            self.path_types.push(current);
                        }
                        Some(ty) => {
                            let prefix = self.ast.show_path(&segments[..index + 2]);
                            let message = format!(
                                "cannot update `{}`: `{prefix}` is {}, not a record",
                                self.ast.show_path(segments),
                                self.in_message(ty)
                            );
                            let label = format!("this is {}", self.in_label(old.ty));
                            self.errors.push(
                                Diagnostic::error(message, path_span(segments))
                                    .with_context(first.span, label),
                            );
                            failed = true;
                            break;
                        }
                    }
                }
            } else {
                let message = format!(
                    "cannot update `{}`: `{}` is {}, not a record",
                    self.ast.show_path(segments),
                    self.ast.show_path(&segments[..1]),
                    self.in_message(old.ty)
                );
                let label = format!("this is {}", self.in_label(old.ty));
                self.errors.push(
                    Diagnostic::error(message, path_span(segments)).with_context(first.span, label),
                );
                failed = true;
            }
        }
        let existing = self.path_types.len();
        let ty = if failed {
            TyId::ERROR
        } else {
            // From the leaf outwards: each record along the path, as it was
            // or created empty, with its segment set.
            let mut ty = leaf_ty;
            for (level, segment) in rest.iter().enumerate().rev() {
                let base = self.path_types.get(level).copied().unwrap_or(TyId::EMPTY);
                ty = self
                    .types
                    .with_field(base, segment.name, ty)
                    .unwrap_or(TyId::ERROR);
            }
            ty
        };
        if let Some((index, old)) = found
            && existing > 0
        {
            // The old record may belong to an enclosing function, and is
            // then read from the closure's captures.
            let source = self.resolve(index, old);
            self.item_paths[id.index()] = PathResolution {
                source,
                existing: position(existing),
            };
        }
        self.declare(first.name, ty, id, BindingKind::Variable);
    }

    /// Declares a variable or field, as `kind` says, for the item `id`, in
    /// the innermost block, in the next free slot, which the item writes.
    fn declare(&mut self, name: Name, ty: TyId, id: ItemId, kind: BindingKind) {
        let span = self.ast.item(id).span;
        let slot = self.declare_binding(name, ty, Some(span), kind);
        self.item_slots[id.index()] = slot;
    }

    /// Declares a binding of `kind` in the innermost block, in the next free
    /// slot of the current frame, and returns that slot. `declared` is where
    /// its declaration is written.
    fn declare_binding(
        &mut self,
        name: Name,
        ty: TyId,
        declared: Option<Span>,
        kind: BindingKind,
    ) -> Slot {
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        match self.slot_names.get_mut(slot.index()) {
            Some(entry) => *entry = name,
            // Slots are taken in order, so a slot beyond the table is the
            // next one.
            None => self.slot_names.push(name),
        }
        self.bindings.push(Binding {
            name,
            slot,
            ty,
            declared,
            kind,
        });
        slot
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

    /// Where a use of `binding`, at `index` in `bindings`, reads its value
    /// from in the current function: its slot, or the callee, if it is the
    /// current function's or the top level's own; and otherwise a capture,
    /// which this adds to every function between the binding's home and
    /// the current one that lacks it (see the module documentation).
    fn resolve(&mut self, index: usize, binding: Binding) -> VarRef {
        let home = match binding.kind {
            BindingKind::Callee => VarRef::Callee,
            BindingKind::Variable | BindingKind::Field => VarRef::Local(binding.slot),
        };
        // The outermost function the binding is not in: every function from
        // there inwards must capture it.
        let Some(first) = self
            .functions
            .iter()
            .position(|function| function.bindings_start > index)
        else {
            return home;
        };
        let mut source = home;
        for function in &mut self.functions[first..] {
            let existing = function
                .captures
                .iter()
                .position(|capture| capture.binding == index);
            let capture = existing.unwrap_or_else(|| {
                function.captures.push(Capture {
                    binding: index,
                    source,
                    ty: binding.ty,
                });
                function.captures.len() - 1
            });
            source = VarRef::Capture(position(capture));
        }
        source
    }

    /// The type a type expression denotes, or `Error` (reported) if it names
    /// no type, or is a record type with a field that names none or a name
    /// that appears twice, or a function type with a parameter or return
    /// type that is erroneous.
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
            TypeExprKind::Fn { params, ret, pure } => {
                let base = self.fn_params.len();
                for param in self.ast.param_tys(params) {
                    // Resolving a nested function type uses `fn_params`
                    // above this one's entries and truncates back.
                    let param_ty = self.resolve_type(param.ty);
                    self.fn_params.push((param.name, param_ty));
                }
                let ret = self.resolve_type(ret);
                let resolved = self.intern_fn_type(base, ret, pure);
                self.fn_params.truncate(base);
                resolved
            }
        }
    }

    /// Interns the function type whose parameters are `fn_params[base..]`
    /// and whose return type is `ret`, or returns `Error` if any of them is
    /// erroneous, having been reported.
    fn intern_fn_type(&mut self, base: usize, ret: TyId, pure: bool) -> TyId {
        let params = self.fn_params.get(base..).unwrap_or_default();
        if ret == TyId::ERROR || params.iter().any(|&(_, ty)| ty == TyId::ERROR) {
            return TyId::ERROR;
        }
        self.types.fn_type(params.iter().copied(), ret, pure)
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
        why: &str,
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
                    | BinaryOp::Same
                    | BinaryOp::NotSame
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
            ExprKind::Fn(function) => self.check_fn(id, function),
            ExprKind::Call { callee, args } => self.check_call(id, callee, args),
        };
        self.expr_types[id.index()] = ty;
        ty
    }

    /// Types the function expression `id`, whose details are `function`,
    /// and checks its body in a frame of its own (see the module
    /// documentation). Its type is known, and recorded, before the body is
    /// checked: the function type of its declared parameter and return
    /// types, or `Error` if any of them is.
    fn check_fn(&mut self, id: ExprId, function: FnId) -> TyId {
        let callee = self.callee_name.take();
        let decl = *self.ast.fn_decl(function);
        let params = self.ast.params(decl.params);
        let base = self.fn_params.len();
        for param in params {
            let ty = self.resolve_type(param.ty);
            self.fn_params.push((Some(param.name), ty));
        }
        let ret = self.resolve_type(decl.ret);
        let ty = self.intern_fn_type(base, ret, decl.pure);
        self.expr_types[id.index()] = ty;
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            pure: decl.pure,
            marker: decl.marker,
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let span = self.ast.get(id).span;
        self.check_fn_body(&decl, base, (callee, ty, span), ret);
        self.fn_params.truncate(base);
        if let Some(finished) = self.functions.pop() {
            let slot_names = std::mem::replace(&mut self.slot_names, finished.outer_slot_names);
            self.fn_resolutions[finished.fn_id.index()] = FnResolution {
                frame_size: self.frame_size,
                param_count: decl.params.len(),
                captures: finished
                    .captures
                    .iter()
                    .map(|capture| (capture.source, capture.ty))
                    .collect(),
                callee,
                slot_names,
            };
            (self.next_slot, self.frame_size) =
                (finished.outer_next_slot, finished.outer_frame_size);
        }
        ty
    }

    /// Types the call `id` of `callee` with the arguments `args` (see the
    /// module documentation): the callee's return type, or `Error` if the
    /// callee is not a function or the arguments do not fit its parameters.
    /// Every argument is checked whatever goes wrong.
    fn check_call(&mut self, id: ExprId, callee: ExprId, args: ArgRange) -> TyId {
        let callee_ty = self.check_expr(callee);
        let Some(sig) = self.types.fn_sig(callee_ty).cloned() else {
            if callee_ty != TyId::ERROR {
                let callee_ty = self.settle(callee, callee_ty);
                let message = format!("cannot call a value of type {}", self.in_message(callee_ty));
                let label = format!("this is {}", self.in_label(callee_ty));
                self.errors.push(
                    Diagnostic::error(message, self.ast.get(id).span)
                        .with_context(self.ast.get(callee).span, label)
                        .with_note("only functions can be called"),
                );
            }
            for arg in self.ast.args(args) {
                self.check_alone(arg.value);
            }
            return TyId::ERROR;
        };
        let erroneous = self.bind_arguments(id, (callee, callee_ty), args, &sig);
        if let Some(function) = self.functions.last()
            && function.pure
            && !sig.pure
        {
            let marker = function.marker;
            let label = format!("this is {}", self.in_label(callee_ty));
            self.errors.push(
                Diagnostic::error(
                    "cannot call an impure function from a pure function",
                    self.ast.get(id).span,
                )
                .with_context(self.ast.get(callee).span, label)
                .with_context(marker, "this function is pure, declared with `:`")
                .with_note(
                    "a pure function, declared with `:`, may call only pure functions; declare \
                     it with `->` to make it impure",
                ),
            );
        }
        if erroneous { TyId::ERROR } else { sig.ret }
    }

    /// Binds the arguments `args` of the call `id` to the parameters of
    /// its callee, of function type `callee_ty` with signature `sig`:
    /// positional arguments fill parameters left to right, the trailing
    /// block the last, and named arguments the parameters of their names.
    /// Checks each argument against its parameter's type, or alone if it
    /// fills none, and records the parameter each fills. Reports every
    /// argument that cannot be bound and every parameter left unfilled, and
    /// returns whether there was any.
    fn bind_arguments(
        &mut self,
        id: ExprId,
        (callee, callee_ty): (ExprId, TyId),
        args: ArgRange,
        sig: &FnTy,
    ) -> bool {
        let ast = self.ast;
        let call = ast.get(id);
        let callee_span = ast.get(callee).span;
        let function_label = format!("the function is {}", self.in_label(callee_ty));
        let arg_list = ast.args(args);
        let count = sig.params.len();
        // `positional` counts the positional arguments before the trailing
        // block; the trailing block, if any, comes after them all.
        let positional = arg_list
            .iter()
            .filter(|arg| arg.kind == ArgKind::Positional)
            .count();
        let trailing = arg_list.iter().any(|arg| arg.kind == ArgKind::Trailing);
        let named = arg_list
            .iter()
            .any(|arg| matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional + usize::from(trailing);
        let mut erroneous = false;
        if found > count {
            self.report_extra_arguments((call, callee_span, &function_label), args, count);
            erroneous = true;
        }
        let base = self.filled.len();
        self.filled.resize(base + count, u32::MAX);
        let mut next_positional = 0;
        for (index, arg) in arg_list.iter().enumerate() {
            let target = match arg.kind {
                ArgKind::Positional => {
                    next_positional += 1;
                    Some(next_positional - 1).filter(|&param| param < count)
                }
                ArgKind::Trailing => (positional < count).then(|| count - 1),
                ArgKind::Named { name, span } => {
                    let param = sig
                        .params
                        .iter()
                        .position(|&(param, _)| param == Some(name));
                    if param.is_none() {
                        self.report_unknown_argument((name, span), (callee, callee_ty), sig);
                        erroneous = true;
                    }
                    param
                }
            };
            let Some(param) = target else {
                self.check_alone(arg.value);
                continue;
            };
            let first = self.filled.get(base + param).copied().unwrap_or(u32::MAX);
            if let Some(first) = arg_list.get(first as usize) {
                let name = sig.params.get(param).and_then(|&(name, _)| name);
                self.report_given_twice((param, name), first.span(ast), arg.span(ast));
                erroneous = true;
                self.check_alone(arg.value);
                continue;
            }
            if let Some(entry) = self.filled.get_mut(base + param) {
                *entry = position(index);
            }
            let param_ty = sig.params.get(param).map_or(TyId::ERROR, |&(_, ty)| ty);
            self.check_expr_expecting(arg.value, param_ty, callee_span, &function_label);
        }
        erroneous |= self.report_unfilled(
            base,
            (call, callee_span, &function_label),
            sig,
            (found, named, trailing),
        );
        if !erroneous {
            for (param, &arg) in self
                .filled
                .get(base..)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                if let Some(entry) = self.arg_params.get_mut(args.start() + arg as usize) {
                    *entry = position(param);
                }
            }
        }
        self.filled.truncate(base);
        erroneous
    }

    /// Reports that the call `call`, whose callee is at `callee_span` and
    /// labelled `function_label`, has more positional arguments, among
    /// `args`, than its callee's `count` parameters, labelling each one
    /// beyond them (the trailing block counting as the last).
    fn report_extra_arguments(
        &mut self,
        (call, callee_span, function_label): (&Expr, Span, &str),
        args: ArgRange,
        count: usize,
    ) {
        let ast = self.ast;
        let positional = ast
            .args(args)
            .iter()
            .filter(|arg| !matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional.clone().count();
        let noun = if count == 1 { "argument" } else { "arguments" };
        let mut error =
            Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                .with_context(callee_span, function_label);
        for extra in positional.skip(count) {
            error = error.with_context(extra.span(ast), "unexpected argument");
        }
        self.errors.push(error);
    }

    /// Reports the argument written at `second` for parameter number
    /// `param`, named `name` if it has one, which the argument at `first`
    /// already fills.
    fn report_given_twice(
        &mut self,
        (param, name): (usize, Option<Name>),
        first: Span,
        second: Span,
    ) {
        let shown = match name {
            Some(name) => format!("`{}`", self.ast.names().get(name)),
            None => (param + 1).to_string(),
        };
        self.errors.push(
            Diagnostic::error(format!("argument {shown} is given twice"), second)
                .with_context(first, "first given here"),
        );
    }

    /// Reports the named argument `name`, written at `span`, that no
    /// parameter of the callee, of type `callee_ty` with signature `sig`,
    /// is named: an unknown argument, or, if none of its parameters has a
    /// name, a function that takes none.
    fn report_unknown_argument(
        &mut self,
        (name, span): (Name, Span),
        (callee, callee_ty): (ExprId, TyId),
        sig: &FnTy,
    ) {
        let callee_span = self.ast.get(callee).span;
        let shown = self.in_label(callee_ty);
        let error = if !sig.params.is_empty() && sig.params.iter().all(|(name, _)| name.is_none()) {
            Diagnostic::error("the parameters of this function have no names", span)
                .with_context(callee_span, format!("this is {shown}"))
                .with_note("call it with positional arguments")
        } else {
            let text = self.ast.names().get(name);
            Diagnostic::error(format!("unknown argument `{text}`"), span)
                .with_context(callee_span, format!("the function is {shown}"))
        };
        self.errors.push(error);
    }

    /// Reports the parameters of the call `call`'s callee, at
    /// `callee_span` and labelled `function_label`, with signature `sig`,
    /// that `filled[base..]` shows no argument fills. A call with only
    /// positional arguments, of which it has `found`, is short of arguments
    /// as a whole; one with named arguments or a trailing block is missing
    /// each unfilled parameter. Returns whether any was unfilled.
    fn report_unfilled(
        &mut self,
        base: usize,
        (call, callee_span, function_label): (&Expr, Span, &str),
        sig: &FnTy,
        (found, named, trailing): (usize, bool, bool),
    ) -> bool {
        let filled = self.filled.get(base..).unwrap_or_default();
        if filled.iter().all(|&arg| arg != u32::MAX) {
            return false;
        }
        let count = sig.params.len();
        if !named && !trailing {
            let noun = if count == 1 { "argument" } else { "arguments" };
            self.errors.push(
                Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                    .with_context(callee_span, function_label),
            );
            return true;
        }
        let close = call.close_span().unwrap_or(call.span);
        let unfilled: Vec<usize> = filled
            .iter()
            .enumerate()
            .filter(|&(_, &arg)| arg == u32::MAX)
            .map(|(param, _)| param)
            .collect();
        for param in unfilled {
            let message = match sig.params.get(param) {
                Some(&(Some(name), _)) => {
                    format!("missing argument `{}`", self.ast.names().get(name))
                }
                Some(&(None, ty)) => {
                    format!(
                        "missing argument {} of type {}",
                        param + 1,
                        self.in_message(ty)
                    )
                }
                None => continue,
            };
            self.errors
                .push(Diagnostic::error(message, close).with_context(callee_span, function_label));
        }
        true
    }

    /// Checks the body of the function expression `decl`, whose parameters'
    /// types are `fn_params[base..]`, against its return type `ret`, in a
    /// scope of its own. The scope starts with the callee binding, if the
    /// function has a `callee` name (with the function's type and span),
    /// then the parameters, each in the next slot; then the items are
    /// checked, and the body's value against `ret` as an annotated
    /// initialiser's is against its annotation, a record block against a
    /// record return type field by field.
    fn check_fn_body(
        &mut self,
        decl: &FnDecl,
        base: usize,
        (callee, fn_ty, fn_span): (Option<Name>, TyId, Span),
        ret: TyId,
    ) {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let nodes_start = self.nodes.len();
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        if let Some(name) = callee {
            self.bindings.push(Binding {
                name,
                slot: Slot::NONE,
                ty: fn_ty,
                declared: Some(fn_span),
                kind: BindingKind::Callee,
            });
        }
        let params = self.ast.params(decl.params);
        for (index, param) in params.iter().enumerate() {
            if let Some(first) = params[..index]
                .iter()
                .find(|first| first.name == param.name)
            {
                let text = self.ast.names().get(param.name);
                self.errors.push(
                    Diagnostic::error(format!("parameter `{text}` is declared twice"), param.span)
                        .with_context(first.span, "first declared here"),
                );
            }
            let ty = self
                .fn_params
                .get(base + index)
                .map_or(TyId::ERROR, |&(_, ty)| ty);
            self.declare_binding(param.name, ty, Some(param.span), BindingKind::Variable);
        }
        let context = self.ast.type_expr(decl.ret).span;
        let why = "the return type is declared here";
        let body = decl.body;
        let block = *self.ast.get(body);
        let ExprKind::Block(items) = block.kind else {
            // The parser only makes blocks bodies.
            self.errors.push(
                InternalError {
                    message: "a function body is not a block",
                    span: block.span,
                }
                .into(),
            );
            self.bindings.truncate(self.block_start);
            (self.block_start, self.fields_start, self.next_slot) = outer;
            return;
        };
        if self.types.is_record(ret) && self.is_record_body(items) {
            let expecting = Expecting {
                ty: ret,
                context,
                why,
                close: Span::new(block.span.end.saturating_sub(1), block.span.end),
            };
            let found = self.check_body(items, Some(expecting));
            self.expr_types[body.index()] = found;
        } else {
            let found = self.check_body(items, None);
            self.expr_types[body.index()] = found;
            self.expect(body, found, ret, context, why);
        }
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        self.nodes.truncate(nodes_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
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
    /// where it reads from, capturing it if it belongs to an enclosing
    /// function, and returns its type.
    fn check_name(&mut self, id: ExprId, name: Name, span: Span) -> TyId {
        if let Some((index, binding)) = self.lookup(name) {
            self.expr_vars[id.index()] = self.resolve(index, binding);
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
    /// settling to `Int`: any type without functions in it for `==`, `!=`,
    /// `===` and `!==` (two records must be exactly the same type), and
    /// `Int`, `Float` or `String` for ordering. The result is `Bool`.
    fn comparison(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        let ordering = !op.is_equality();
        if ordering && lhs_ty != TyId::ERROR && rhs_ty != TyId::ERROR {
            // An operand of a type with no order is reported as such, before
            // any mismatch.
            let unordered = [lhs_ty, rhs_ty].into_iter().find(|&ty| {
                ty == TyId::BOOL || self.types.is_record(ty) || self.types.fn_sig(ty).is_some()
            });
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
            // Functions have no equality, even inside records.
            Some(ty) if self.types.contains_fn(ty) => {
                let message = if self.types.fn_sig(ty).is_some() {
                    "cannot compare functions".to_owned()
                } else {
                    format!(
                        "cannot compare {}: it contains a function",
                        self.in_message(ty)
                    )
                };
                let expr = self.ast.get(id);
                let span = expr.operator_span().unwrap_or(expr.span);
                let label = format!("this is {}", self.in_label(ty));
                self.errors.push(
                    Diagnostic::error(message, span)
                        .with_context(self.ast.get(lhs).span, label.as_str())
                        .with_context(self.ast.get(rhs).span, label),
                );
                TyId::ERROR
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
    /// type: `Bool`, `String`, record and function types must match
    /// exactly, and numeric types agree as for [`Checker::numeric`], whose
    /// results this returns.
    fn agree(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        if lhs_ty == rhs_ty
            && (lhs_ty == TyId::BOOL
                || lhs_ty == TyId::STRING
                || self.types.is_record(lhs_ty)
                || self.types.fn_sig(lhs_ty).is_some())
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
    /// `keyword`: two record branches, or two function branches, have their
    /// join, and otherwise the branches must agree, untyped branches
    /// adapting as operands of arithmetic do, and two untyped branches
    /// leaving the `if` untyped.
    fn branches(
        &mut self,
        keyword: Span,
        (then, then_ty): (ExprId, TyId),
        (else_, else_ty): (ExprId, TyId),
    ) -> TyId {
        let joinable = (self.types.is_record(then_ty) && self.types.is_record(else_ty))
            || (self.types.fn_sig(then_ty).is_some() && self.types.fn_sig(else_ty).is_some());
        let common = if joinable {
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
            | ExprKind::Ascribe { .. }
            | ExprKind::Fn(_)
            | ExprKind::Call { .. } => {}
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
    /// record or function type is.
    fn in_message(&self, ty: TyId) -> String {
        let shown = self.types.display(ty, self.ast.names());
        match self.types.kind(ty) {
            TyKind::Record(_) | TyKind::Fn(_) | TyKind::Union(_) => format!("`{shown}`"),
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
/// `Checker::sorted_fields`, a node's index in `Checker::nodes`, or a count
/// of path segments. There are fewer of each than the source has bytes, so
/// the conversion never saturates.
fn position(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// The note on a dotted path under a field, or a segment, defined whole.
const WHOLE_FIELD_NOTE: &str = "a field defined as a whole cannot be extended with dotted paths";

/// The note on a field that the expected record type lacks.
const EXTRA_FIELD_NOTE: &str = "it is allowed, but nothing with this type can see it";

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
        assert_eq!(
            resolution.expr_var(result_expr(&ast)),
            VarRef::Local(Slot(1))
        );
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
        assert_eq!(result_type("let x = 1\nx := 2\nx"), TyId::INT);
        assert_eq!(
            result_type("let x = 1\nx := x + 1\nx := x * 2\nx"),
            TyId::INT
        );
        // In an inner block, of a variable declared in that block, even one
        // shadowing an outer variable.
        assert_eq!(
            result_type("let x = 1\n{ let x = 2.5; x := 3.5; x }"),
            TyId::FLOAT
        );
        // The new value sees the old binding.
        let ast = parse_text("let x = 1\nx := x + 1\nx");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(0));
        // `:=` takes a fresh slot.
        assert_eq!(resolution.item_slot(items[1]), Slot(1));
        assert_eq!(
            resolution.expr_var(result_expr(&ast)),
            VarRef::Local(Slot(1))
        );
    }

    #[test]
    fn rebinding_is_assignment_compatible() {
        // The literal adapts to the variable's type, which the rebind keeps.
        assert_eq!(result_type("let x: Float = 1\nx := 2\nx"), TyId::FLOAT);
        // A subtype fits, and the variable keeps its wider type.
        assert_eq!(
            shown_result("let p: {x: Int} = { x = 1 }\np := { x = 2; y = 3 }\np"),
            "{x: Int}"
        );
        // A different type does not: changing a type takes a `let`.
        let diagnostic = &diagnostics("let x = 1\nx := 2.5\nx")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(15, 18));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found Float")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 9), "the variable was declared here")]
        );
        // The variable keeps its type afterwards, so nothing cascades.
        assert_eq!(
            messages("let x = 1\nx := 2.5\nx + 1"),
            vec!["mismatched types"]
        );
        assert_eq!(
            result_type("let x = 1\nlet x = 2.5\nx := 3.5\nx"),
            TyId::FLOAT
        );
    }

    #[test]
    fn a_dotted_rebind_keeps_the_leaf_type_and_adds_nothing() {
        assert_eq!(
            shown_result("let a = { x = 1; n = { m = 1 } }\na.n.m := 5\na"),
            "{n: {m: Int}, x: Int}"
        );
        // A subtype fits at the leaf.
        assert_eq!(
            shown_result("let a: {r: {x: Int}} = { r = { x = 1 } }\na.r := { x = 2; y = 3 }\na"),
            "{r: {x: Int}}"
        );
        let diagnostic = &diagnostics("let a = { x = 1 }\na.x := \"s\"\na")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found String")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(18, 21), "`:=` keeps the field's type")]
        );
        let diagnostic = &diagnostics("let a = { x = 1 }\na.y := 2\na")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `a.y`: `a` has no field `y`"
        );
        assert_eq!(diagnostic.span, Span::new(18, 21));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(18, 19), "this is {x: Int}")]
        );
        assert_eq!(
            diagnostic.notes,
            vec!["`:=` cannot add a field; add it with `let a.y = ...`"]
        );
        assert_eq!(
            messages("let a = { x = 1 }\na.x.z := 2"),
            vec!["cannot update `a.x.z`: `a.x` is Int, not a record"]
        );
        // The rejected rebind declares `a` as an error, so nothing cascades.
        assert_eq!(
            messages("let a = { x = 1 }\na.y := 2\na.x + 1"),
            vec!["cannot rebind `a.y`: `a` has no field `y`"]
        );
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
                ExprKind::Fn(function) => visit(ast, ast.fn_decl(function).body, out),
                ExprKind::Call { callee, args } => {
                    visit(ast, callee, out);
                    for arg in ast.args(args) {
                        visit(ast, arg.value, out);
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
                .filter(|&&id| {
                    matches!(ast.item(id).kind, ItemKind::Let { path, .. } if ast.path(path)[0].name == wanted)
                })
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
        assert_eq!(resolution.expr_var(rhs), VarRef::Local(Slot(2)));
        let ExprKind::Binary { lhs: a, rhs: b, .. } = ast.get(lhs).kind else {
            panic!("expected a sum");
        };
        assert_eq!(resolution.expr_var(a), VarRef::Local(Slot(0)));
        assert_eq!(resolution.expr_var(b), VarRef::Local(Slot(1)));
        // Nodes that are not variables have no slot.
        assert_eq!(resolution.expr_var(lhs), VarRef::None);
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
        assert_eq!(
            resolution.expr_var(result_expr(&ast)),
            VarRef::Local(Slot(1))
        );
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

    // ----- Dotted fields -----

    #[test]
    fn dotted_fields_merge_into_one_record() {
        assert_eq!(
            file_type("server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true"),
            "{server: {host: String, port: Int, tls: {enabled: Bool}}}"
        );
        assert_eq!(
            shown_result("{ a.b.c = 1; a.b.d = 2.5; a.e = \"s\"; f = 1 }"),
            "{a: {b: {c: Int, d: Float}, e: String}, f: Int}"
        );
        // Quoted segments are names like any other.
        assert_eq!(
            shown_result("{ \"log level\".format = \"json\"; \"log level\".\"to file\" = true }"),
            "{\"log level\": {format: String, \"to file\": Bool}}"
        );
        // Interleaved with other fields and with `let`s.
        assert_eq!(
            shown_result("{ a.x = 1; b = 2; let t = 3; a.y = t }"),
            "{a: {x: Int, y: Int}, b: Int}"
        );
        // The field is one binding, in one slot, whatever extends it.
        let ast = parse_text("a.x = 1\nb = 2\na.y = 3\na.z.w = 4");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        let slots: Vec<Slot> = items.iter().map(|&id| resolution.item_slot(id)).collect();
        assert_eq!(slots, vec![Slot(0), Slot(1), Slot(0), Slot(0)]);
        assert_eq!(resolution.frame_size(), 2);
        // The first definition builds from nothing; each later one updates
        // the field's slot, through as many segments as exist.
        let paths: Vec<PathResolution> = items.iter().map(|&id| resolution.item_path(id)).collect();
        assert_eq!(
            paths,
            vec![
                PathResolution::NONE,
                PathResolution::NONE,
                PathResolution {
                    source: VarRef::Local(Slot(0)),
                    existing: 1
                },
                PathResolution {
                    source: VarRef::Local(Slot(0)),
                    existing: 1
                },
            ]
        );
        let ast = parse_text("a.b.c = 1\na.b.d = 2");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        assert_eq!(
            resolution.item_path(ast.items(ast.root())[1]),
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 2
            }
        );
    }

    #[test]
    fn the_type_of_an_open_field_grows_item_by_item() {
        assert_eq!(
            shown_result("{ a.x = 1; let p = a; a.y = \"s\"; let q = a; r = p; s = q }"),
            "{a: {x: Int, y: String}, r: {x: Int}, s: {x: Int, y: String}}"
        );
        // A later definition sees the field as built so far.
        assert_eq!(
            shown_result("{ a.x = 1; a.y = a.x + 1 }"),
            "{a: {x: Int, y: Int}}"
        );
        assert_eq!(
            messages("{ a.x = 1; a.y = a.z }"),
            vec!["no field `z` in `{x: Int}`"]
        );
        // The first definition does not see its own field.
        assert_eq!(messages("{ a.x = a }"), vec!["undefined variable `a`"]);
        // A later `let` shadows the field without changing it.
        assert_eq!(
            shown_result("{ a.x = 1; let a = \"s\"; a.y = a }"),
            "{a: {x: Int, y: String}}"
        );
    }

    #[test]
    fn a_dotted_path_under_a_whole_field_is_an_error() {
        let diagnostic = &diagnostics("{ a = 1; a.b = 2 }")[0];
        assert_eq!(diagnostic.message, "field `a` is already defined");
        assert_eq!(diagnostic.span, Span::new(9, 12));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 3), "defined whole here")]
        );
        assert_eq!(
            diagnostic.notes,
            vec!["a field defined as a whole cannot be extended with dotted paths"]
        );
        // Whatever the whole field's type, record or not.
        assert_eq!(
            messages("{ a = { b = 1 }; a.c = 2 }"),
            vec!["field `a` is already defined"]
        );
        // A segment defined whole as a record, by a path's last segment.
        let diagnostic = &diagnostics("{ a.b = { x = 1 }; a.b.y = 2 }")[0];
        assert_eq!(diagnostic.message, "field `a.b` is already defined");
        assert_eq!(diagnostic.span, Span::new(19, 24));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 5), "defined whole here")]
        );
        assert_eq!(
            diagnostic.notes,
            vec!["a field defined as a whole cannot be extended with dotted paths"]
        );
        assert_eq!(
            messages("let r = { x = 1 }\n{ a.b = r; a.b.c.d = 1 }"),
            vec!["field `a.b` is already defined"]
        );
        // The rejected path declares nothing, so nothing cascades, but its
        // value is still checked.
        assert_eq!(
            messages("{ a = 1; a.b = 2; c = a + 1 }"),
            vec!["field `a` is already defined"]
        );
        assert_eq!(
            messages("{ a = 1; a.b = 1.0 // 2 }"),
            vec!["field `a` is already defined", "cannot floor-divide Float"]
        );
        // The block's record is erroneous, so nothing outside cascades.
        assert_eq!(
            messages("let r = { a = 1; a.b = 2 }\nr.a + 1.5\nr.q"),
            vec!["field `a` is already defined"]
        );
        assert_eq!(
            messages("let r = { a.b = 1; a.b.c = 2 }\nr.a.b + 1.5"),
            vec!["cannot define `a.b.c`: `a.b` is Int, not a record"]
        );
    }

    #[test]
    fn a_whole_field_after_a_dotted_one_is_defined_twice() {
        let diagnostic = &diagnostics("{ a.b = 1; a = 2 }")[0];
        assert_eq!(diagnostic.message, "field `a` is defined twice");
        assert_eq!(diagnostic.span, Span::new(11, 12));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 3), "first defined here")]
        );
        // Later dotted paths still extend the first, dotted, field.
        assert_eq!(
            messages("{ a.b = 1; a = 2; a.c = 3 }"),
            vec!["field `a` is defined twice"]
        );
    }

    #[test]
    fn a_segment_that_is_not_a_record_cannot_hold_fields() {
        let diagnostic = &diagnostics("{ a.b = 1; a.b.c = 2 }")[0];
        assert_eq!(
            diagnostic.message,
            "cannot define `a.b.c`: `a.b` is Int, not a record"
        );
        assert_eq!(diagnostic.span, Span::new(11, 16));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 5), "`a.b` defined here")]
        );
        assert!(diagnostic.notes.is_empty());
        // Deeper, with another type, and quoted.
        assert_eq!(
            messages("{ a.b.c = \"s\"; a.b.c.d.e = 1 }"),
            vec!["cannot define `a.b.c.d.e`: `a.b.c` is String, not a record"]
        );
        assert_eq!(
            messages("{ a.\"b c\" = true; a.\"b c\".d = 1 }"),
            vec!["cannot define `a.\"b c\".d`: `a.\"b c\"` is Bool, not a record"]
        );
        // An erroneous segment has been reported already.
        assert_eq!(
            messages("{ a.b = 1.0 // 2; a.b.c = 1 }"),
            vec!["cannot floor-divide Float"]
        );
    }

    #[test]
    fn a_dotted_path_defined_twice_is_an_error() {
        let diagnostic = &diagnostics("{ a.b = 1; a.b = 2 }")[0];
        assert_eq!(diagnostic.message, "field `a.b` is defined twice");
        assert_eq!(diagnostic.span, Span::new(11, 14));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 5), "first defined here")]
        );
        // A record a longer path created is defined too, where that path
        // wrote it.
        let diagnostic = &diagnostics("{ a.b.c = 1; a.b = 2 }")[0];
        assert_eq!(diagnostic.message, "field `a.b` is defined twice");
        assert_eq!(diagnostic.span, Span::new(13, 16));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 5), "first defined here")]
        );
        // A quoted name is the same name.
        assert_eq!(
            messages("{ \"a b\".c = 1; \"a b\".\"c\" = 2 }"),
            vec!["field `\"a b\".c` is defined twice"]
        );
        // The record type does not cascade.
        assert_eq!(
            messages("let r = { a.b = 1; a.b = 2 }\nr.a.b + 1.5"),
            vec!["field `a.b` is defined twice"]
        );
    }

    #[test]
    fn dotted_fields_are_checked_against_the_expected_type() {
        // The leaf is checked against the type at its full path.
        assert_eq!(
            all_types("let p: {a: {b: Float}} = { a.b = 1 }")[0],
            TyId::FLOAT
        );
        assert_eq!(
            shown_result("let p: {a: {b: Float, c: {d: Float}}} = { a.b = 1; a.c.d = 2 }\np"),
            "{a: {b: Float, c: {d: Float}}}"
        );
        assert!(warnings("let p: {a: {b: Int}} = { a.b = 1 }\np").is_empty());
        // Record blocks at the leaf are checked field by field.
        assert_eq!(
            all_types("let p: {a: {b: {n: Float}}} = { a.b = { n = 1 } }")[0],
            TyId::FLOAT
        );
        let diagnostic = &diagnostics("let p: {a: {b: Int}} = { a.b = \"s\" }")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(31, 34));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found String")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 20), "expected because of this annotation")]
        );
        // In an ascription too.
        assert_eq!(all_types("{ a.b = 1 } : {a: {b: Float}}")[0], TyId::FLOAT);
    }

    #[test]
    fn a_missing_nested_field_of_a_dotted_field_is_an_error() {
        let text = "let p: {a: {b: Int, c: Int}} = { a.b = 1 }";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(diagnostic.message, "missing field `a.c` in record");
        assert_eq!(diagnostic.span, Span::new(41, 42));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 28), "expected because of this annotation")]
        );
        assert_eq!(
            diagnostic.notes,
            vec!["the type `{b: Int, c: Int}` needs it"]
        );
        // At every level, each in alphabetical order, and alongside the
        // block's own missing fields.
        assert_eq!(
            messages("let p: {a: {b: {c: Int, d: Int, e: Int}}, z: Int} = { a.b.d = 1 }"),
            vec![
                "missing field `z` in record",
                "missing field `a.b.c` in record",
                "missing field `a.b.e` in record"
            ]
        );
        // It does not cascade.
        assert_eq!(
            messages("let p: {a: {b: Int, c: Int}} = { a.b = 1 }\np.a.c + 1.5"),
            vec!["missing field `a.c` in record"]
        );
    }

    #[test]
    fn a_dotted_field_the_expected_type_lacks_is_a_warning() {
        let text = "let p: {a: {b: Int}} = { a.b = 1; a.c = 2 }\np";
        let found = warnings(text);
        assert_eq!(found.len(), 1);
        let warning = &found[0];
        assert_eq!(warning.message, "field `c` is not in `{b: Int}`");
        assert_eq!(warning.span, Span::new(36, 37));
        assert_eq!(
            labels(warning),
            vec![(Span::new(7, 20), "expected because of this annotation")]
        );
        assert_eq!(
            warning.notes,
            vec!["it is allowed, but nothing with this type can see it"]
        );
        assert_eq!(shown_result(text), "{a: {b: Int}}");
        // For the first missing segment only, once per such field.
        let found = warnings("let p: {a: Int} = { a = 1; x.p = 1; x.q.r = 2; y.s.t = 3 }");
        let shown: Vec<(&str, Span)> = found
            .iter()
            .map(|warning| (warning.message.as_str(), warning.span))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("field `x` is not in `{a: Int}`", Span::new(27, 28)),
                ("field `y` is not in `{a: Int}`", Span::new(47, 48))
            ]
        );
        let found = warnings("let p: {a: {b: Int}} = { a.b = 1; a.c.d = 2; a.c.e = 3 }");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "field `c` is not in `{b: Int}`");
    }

    #[test]
    fn a_dotted_field_where_the_expected_type_is_not_a_record_mismatches() {
        let diagnostic = &diagnostics("let p: {a: Int} = { a.b = 1 }")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(20, 21));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found {b: Int}")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 15), "expected because of this annotation")]
        );
        // Nested, at the segment's first definition, and without warnings
        // about the segments below it.
        let text = "let p: {a: {b: String}} = { a.b.c = 1; a.b.d = 2 }";
        let found = diagnostics(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].span, Span::new(28, 31));
        assert_eq!(
            found[0].label.as_deref(),
            Some("expected String, found {c: Int, d: Int}")
        );
    }

    // ----- Dotted `let` and `:=` -----

    #[test]
    fn a_dotted_let_without_a_variable_builds_nested_records() {
        assert_eq!(shown_result("let a.b.c = 1\na"), "{b: {c: Int}}");
        assert_eq!(shown_result("let a.b = \"s\"\na.b"), "String");
        // An annotation applies to the leaf.
        assert_eq!(shown_result("let a.b: Float = 5\na"), "{b: Float}");
        assert_eq!(all_types("let a.b: Float = 5")[0], TyId::FLOAT);
        assert_eq!(
            diagnostics("let a.b: Int = 1.5")[0].label.as_deref(),
            Some("expected Int, found Float")
        );
        let ast = parse_text("let a.b.c = 1");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        assert_eq!(
            resolution.item_path(ast.items(ast.root())[0]),
            PathResolution::NONE
        );
    }

    #[test]
    fn a_dotted_let_updates_a_record_variable() {
        // A field added.
        assert_eq!(
            shown_result("let a = { x = 1 }\nlet a.y = \"s\"\na"),
            "{x: Int, y: String}"
        );
        // Intermediate records created, and updated inside.
        assert_eq!(
            shown_result("let a = { x = 1 }\nlet a.b.c = true\na"),
            "{b: {c: Bool}, x: Int}"
        );
        assert_eq!(
            shown_result("let a = { b = { c = 1; d = 2 } }\nlet a.b.e = 3\na"),
            "{b: {c: Int, d: Int, e: Int}}"
        );
        // A leaf replaced, its type changing.
        assert_eq!(
            shown_result("let a = { b = { c = 1; d = 2 } }\nlet a.b.c = \"s\"\na"),
            "{b: {c: String, d: Int}}"
        );
        assert_eq!(
            shown_result("let a = { b = { c = 1 } }\nlet a.b = 2.5\na"),
            "{b: Float}"
        );
        // Through the static type: a field hidden by subtyping is created
        // anew.
        assert_eq!(
            shown_result("let p: {x: Int} = { x = 1; y = 2 }\nlet p.y.z = 3\np"),
            "{x: Int, y: {z: Int}}"
        );
        // How far into the path the old record reaches, for lowering.
        let ast = parse_text("let a = { b = { c = 1 } }\nlet a.b.c = 2\nlet a.x.y.z = 3");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        assert_eq!(
            resolution.item_path(items[1]),
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 2
            }
        );
        assert_eq!(
            resolution.item_path(items[2]),
            PathResolution {
                source: VarRef::Local(Slot(1)),
                existing: 1
            }
        );
        // The update is a new binding in a new slot.
        assert_eq!(resolution.item_slot(items[1]), Slot(1));
        assert_eq!(resolution.item_slot(items[2]), Slot(2));
    }

    #[test]
    fn a_dotted_let_leaves_the_old_binding_unchanged() {
        let text = "let a = { x = 1 }\nlet old = a\nlet a.x = \"s\"\nold";
        assert_eq!(shown_result(text), "{x: Int}");
        let text = "let a = { x = 1 }\nlet old = a\nlet a.x = \"s\"\na";
        assert_eq!(shown_result(text), "{x: String}");
        // In an inner block, the outer variable is shadowed only there.
        assert_eq!(
            shown_result("let a = { x = 1 }\n{ let a.y = 2; a }"),
            "{x: Int, y: Int}"
        );
        assert_eq!(
            shown_result("let a = { x = 1 }\n{ let a.y = 2 }\na"),
            "{x: Int}"
        );
    }

    #[test]
    fn a_dotted_let_needs_records_along_its_path() {
        let diagnostic = &diagnostics("let a = 1\nlet a.b = 2")[0];
        assert_eq!(
            diagnostic.message,
            "cannot update `a.b`: `a` is Int, not a record"
        );
        assert_eq!(diagnostic.span, Span::new(14, 17));
        assert_eq!(labels(diagnostic), vec![(Span::new(14, 15), "this is Int")]);
        let diagnostic = &diagnostics("let a = { b = 1 }\nlet a.b.c = 2")[0];
        assert_eq!(
            diagnostic.message,
            "cannot update `a.b.c`: `a.b` is Int, not a record"
        );
        assert_eq!(diagnostic.span, Span::new(22, 27));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(22, 23), "this is {b: Int}")]
        );
        assert_eq!(
            messages("let a = { b = { c = \"s\" } }\nlet a.b.c.d.e = 2"),
            vec!["cannot update `a.b.c.d.e`: `a.b.c` is String, not a record"]
        );
        // The variable is declared as an error, so nothing cascades; the
        // value is still checked.
        assert_eq!(
            messages("let a = 1\nlet a.b = 2\na.b + 1.5"),
            vec!["cannot update `a.b`: `a` is Int, not a record"]
        );
        assert_eq!(
            messages("let a = true\nlet a.b = 1.0 // 2"),
            vec![
                "cannot update `a.b`: `a` is Bool, not a record",
                "cannot floor-divide Float"
            ]
        );
        // An erroneous variable has been reported already.
        assert_eq!(
            messages("let a = 1.0 // 2\nlet a.b = 1"),
            vec!["cannot floor-divide Float"]
        );
    }

    #[test]
    fn a_dotted_rebind_updates_a_variable_of_its_block() {
        assert_eq!(shown_result("let a = { x = 1 }\na.x := 2\na"), "{x: Int}");
        assert_eq!(
            shown_result("let a = { x = 1; y = { z = \"s\" } }\na.y.z := \"t\"\na"),
            "{x: Int, y: {z: String}}"
        );
        // A new binding, like any `:=`.
        let ast = parse_text("let a = { x = 1 }\na.x := 2");
        let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[1]), Slot(1));
        assert_eq!(
            resolution.item_path(items[1]),
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 1
            }
        );
        let text = "let a = { x = 1 }\nlet old = a\na.x := 2\nold";
        assert_eq!(shown_result(text), "{x: Int}");
    }

    #[test]
    fn a_dotted_rebind_follows_the_rebind_rules() {
        let diagnostic = &diagnostics("a.x := 1")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `a`: no variable named `a` is declared"
        );
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(diagnostic.notes, vec!["declare it with `let a = ...`"]);
        // Declared anyway, so uses do not cascade.
        assert_eq!(
            messages("a.x := 1\na.x + 1.5"),
            vec![
                "cannot rebind `a`: no variable named `a` is declared",
                "cannot add Int and Float"
            ]
        );
        let diagnostic = &diagnostics("{ a = { x = 1 }; a.x := 2 }")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `a`: it is a field, not a variable"
        );
        assert_eq!(diagnostic.span, Span::new(17, 18));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(2, 15), "declared here")]
        );
        let diagnostic = &diagnostics("let a = { x = 1 }\n{ a.x := 2 }")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `a` here: it is declared in an enclosing block"
        );
        assert_eq!(diagnostic.span, Span::new(20, 21));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(0, 17), "declared here")]
        );
        // And the update rules.
        let diagnostic = &diagnostics("let a = 1\na.b := 2")[0];
        assert_eq!(
            diagnostic.message,
            "cannot update `a.b`: `a` is Int, not a record"
        );
        assert_eq!(diagnostic.span, Span::new(10, 13));
        assert_eq!(labels(diagnostic), vec![(Span::new(10, 11), "this is Int")]);
        assert_eq!(
            messages("let a = { b = 1 }\na.b.c := 2"),
            vec!["cannot update `a.b.c`: `a.b` is Int, not a record"]
        );
    }

    // ----- Total equality -----

    #[test]
    fn total_equality_needs_identical_types() {
        for op in ["===", "!=="] {
            assert_eq!(
                result_type(&format!("{{ x = 1 }} {op} {{ x = 2 }}")),
                TyId::BOOL
            );
            assert_eq!(result_type(&format!("1 {op} 2")), TyId::BOOL);
            assert_eq!(result_type(&format!("\"a\" {op} \"b\"")), TyId::BOOL);
            assert_eq!(result_type(&format!("true {op} false")), TyId::BOOL);
            assert_eq!(result_type(&format!("{{}} {op} {{}}")), TyId::BOOL);
            // Untyped literals adapt as for `==`.
            assert_eq!(
                all_types(&format!("1 {op} 2.5")),
                vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
            );
            let diagnostic = &diagnostics(&format!("{{ x = 1 }} {op} {{ x = 1; y = 2 }}"))[0];
            assert_eq!(
                diagnostic.message,
                "cannot compare `{x: Int}` and `{x: Int, y: Int}`"
            );
            assert_eq!(diagnostic.span, Span::new(10, 13));
            assert_eq!(
                labels(diagnostic),
                vec![
                    (Span::new(0, 9), "{x: Int}"),
                    (Span::new(14, 30), "{x: Int, y: Int}")
                ]
            );
            assert_eq!(
                messages(&format!("true {op} 1")),
                vec!["cannot compare Bool and Int"]
            );
        }
        // Every pair of types, as for `==`.
        for (lhs, lhs_ty, lhs_shown) in VARIABLES {
            for (rhs, rhs_ty, rhs_shown) in VARIABLES {
                let text = format!("{PRELUDE}{lhs} === {rhs}");
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
        // A narrowed variable compares with values of its static type.
        assert_eq!(
            result_type("let q: {x: Int} = { x = 1; y = 2 }\nq === { x = 1 }"),
            TyId::BOOL
        );
    }

    // ----- Functions -----

    /// The span of the `n`th (from 0) occurrence of `needle` in `text`.
    fn nth(text: &str, needle: &str, n: usize) -> Span {
        let start = text
            .match_indices(needle)
            .nth(n)
            .unwrap_or_else(|| panic!("{needle:?} does not occur {} times in {text:?}", n + 1))
            .0;
        let start = u32::try_from(start).unwrap();
        Span::new(start, start + u32::try_from(needle.len()).unwrap())
    }

    /// The span of the first occurrence of `needle` in `text`.
    fn at(text: &str, needle: &str) -> Span {
        nth(text, needle, 0)
    }

    /// The span of the last occurrence of `needle` in `text`.
    fn last(text: &str, needle: &str) -> Span {
        let start = u32::try_from(text.rfind(needle).unwrap()).unwrap();
        Span::new(start, start + u32::try_from(needle.len()).unwrap())
    }

    /// Checks `text`, which must succeed, returning its tree and resolution.
    fn resolved(text: &str) -> (Ast, Resolution) {
        let (ast, _, output) = check_text(text);
        let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        (ast, output.checked.resolution)
    }

    /// Where each use of the variable `name` reads from, in allocation
    /// order.
    fn uses(ast: &Ast, resolution: &Resolution, name: &str) -> Vec<VarRef> {
        let name = ast.names().lookup(name).unwrap();
        ast.expr_ids()
            .filter(|&id| ast.get(id).kind == ExprKind::Name(name))
            .map(|id| resolution.expr_var(id))
            .collect()
    }

    /// Function number `n`'s captures, as `(VarRef, shown type)` pairs.
    fn captures(text: &str, n: u32) -> Vec<(VarRef, String)> {
        let (ast, types, output) = check_text(text);
        let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        output
            .checked
            .resolution
            .function(FnId::new(n))
            .captures
            .iter()
            .map(|&(var, ty)| (var, types.display(ty, ast.names()).to_string()))
            .collect()
    }

    const ADD: &str = "fn add(a: Int, b: Int): Int { a + b }\n";

    #[test]
    fn a_function_expression_has_its_declared_type() {
        assert_eq!(
            shown_result("fn(x: Int, y: String): Bool { true }"),
            "fn(x: Int, y: String): Bool"
        );
        assert_eq!(shown_result("fn() -> {} {}"), "fn() -> {}");
        assert_eq!(
            shown_result("fn(f: fn(Int): Int): {r: fn() -> Int} { r = fn() -> Int { f(1) } }"),
            "fn(f: fn(Int): Int): {r: fn() -> Int}"
        );
        assert_eq!(
            shown_result("let f = fn(x: Int): Int { x }\nf"),
            "fn(x: Int): Int"
        );
        assert_eq!(shown_result("fn f(x: Int): Float { 1 }\nf(1)"), "Float");
        assert_eq!(file_type("fn f(): Int { 1 }"), "{}");
        // An unknown parameter or return type is reported once; the function
        // is erroneous, and so is every call of it, without cascading.
        assert_eq!(
            messages("fn f(x: Nope): Int { 1 }\nf(1) + 1.5"),
            vec!["unknown type `Nope`"]
        );
        assert_eq!(
            messages("fn f(x: Int): Nope { x }\nlet y: Int = f(1)"),
            vec!["unknown type `Nope`"]
        );
        // The parameters are still declared, with the type they have.
        assert_eq!(
            messages("fn f(x: Nope, y: Int): Int { y + 1.5 }"),
            vec!["unknown type `Nope`", "cannot add Int and Float"]
        );
    }

    #[test]
    fn a_body_is_checked_against_the_return_type() {
        // A literal adapts to the return type.
        assert_eq!(shown_result("let f = fn(): Float { 1 }\nf()"), "Float");
        let ast = parse_text("fn(): Float { 1 + 2 }");
        let checked = check(&ast, &mut TypeTable::new()).unwrap().0;
        let ExprKind::Fn(function) = ast.get(result_expr(&ast)).kind else {
            panic!("expected a function");
        };
        let body = ast.fn_decl(function).body;
        assert_eq!(checked.types.get(body), TyId::FLOAT);
        // A mismatch, labelled with the return type.
        let text = "fn(): Int { \"a\" }";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, at(text, "{ \"a\" }"));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found String")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(at(text, "Int"), "the return type is declared here")]
        );
        // A subtype is accepted.
        assert_eq!(
            shown_result("fn(r: {x: Int, y: Int}): {x: Int} { r }"),
            "fn(r: {x: Int, y: Int}): {x: Int}"
        );
        // A record block is checked against a record return type, field by
        // field.
        let text = "fn(): {x: Int, y: Int} { x = 1 }";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(diagnostic.message, "missing field `y` in record");
        assert_eq!(diagnostic.span, Span::new(31, 32));
        assert_eq!(
            labels(diagnostic),
            vec![(
                at(text, "{x: Int, y: Int}"),
                "the return type is declared here"
            )]
        );
        let text = "fn(): {x: Float} { x = 1; z = 2 }";
        let warning = &warnings(text)[0];
        assert_eq!(warning.message, "field `z` is not in `{x: Float}`");
        assert_eq!(warning.span, at(text, "z"));
        assert_eq!(
            labels(warning),
            vec![(at(text, "{x: Float}"), "the return type is declared here")]
        );
        // The literal field adapted to the expected field's type.
        let ast = parse_text(text);
        let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
        let int = ast
            .expr_ids()
            .find(|&id| ast.get(id).kind == ExprKind::Int(1))
            .unwrap();
        assert_eq!(checked.types.get(int), TyId::FLOAT);
        // Through a result, and without one.
        assert_eq!(
            messages("fn(): Int { let x = 1 }"),
            vec!["mismatched types"]
        );
        assert_eq!(shown_result("fn() -> {} { let x = 1 }"), "fn() -> {}");
    }

    #[test]
    fn parameters_are_bindings_of_the_body() {
        // Shadowable, in the body and in blocks within it.
        assert_eq!(
            shown_result("fn(x: Int): String { let x = \"s\"\nx }"),
            "fn(x: Int): String"
        );
        assert_eq!(
            shown_result("fn(x: Int): Float { { let x = 1.5; x } }"),
            "fn(x: Int): Float"
        );
        // Rebindable with `:=`, keeping their type.
        assert_eq!(
            shown_result("fn(x: Int): Int { x := x + 1\nx }"),
            "fn(x: Int): Int"
        );
        let text = "fn(x: Int): Int { x := 1.5\nx }";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, at(text, "1.5"));
        assert_eq!(
            labels(diagnostic),
            vec![(at(text, "x: Int"), "the variable was declared here")]
        );
        // But not from a block inside the body.
        assert_eq!(
            messages("fn(x: Int): Int { { x := 2 }\nx }"),
            vec!["cannot rebind `x` here: it is declared in an enclosing block"]
        );
        // Declared twice.
        let text = "fn(x: Int, x: Float): Float { x }";
        let found = diagnostics(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "parameter `x` is declared twice");
        assert_eq!(found[0].span, at(text, "x: Float"));
        assert_eq!(
            labels(&found[0]),
            vec![(at(text, "x: Int"), "first declared here")]
        );
        // The parameters take the frame's first slots.
        let (ast, resolution) = resolved("fn(x: Int, y: Int): Int { let z = x\nz + y }");
        let function = resolution.function(FnId::new(0));
        assert_eq!(function.param_count, 2);
        assert_eq!(function.frame_size, 3);
        let slot_names: Vec<&str> = function
            .slot_names
            .iter()
            .map(|&name| ast.names().get(name))
            .collect();
        assert_eq!(slot_names, vec!["x", "y", "z"]);
        assert_eq!(uses(&ast, &resolution, "y"), vec![VarRef::Local(Slot(1))]);
        assert_eq!(uses(&ast, &resolution, "z"), vec![VarRef::Local(Slot(2))]);
    }

    #[test]
    fn a_function_has_a_frame_of_its_own() {
        let text = "let a = 1\nlet f = fn(x: Int): Int { let y = x\ny }\nlet b = 2\nb";
        let (ast, resolution) = resolved(text);
        // The top level's frame holds only its own variables.
        assert_eq!(resolution.frame_size(), 3);
        let slot_names: Vec<&str> = resolution
            .slot_names()
            .iter()
            .map(|&name| ast.names().get(name))
            .collect();
        assert_eq!(slot_names, vec!["a", "f", "b"]);
        assert_eq!(resolution.function(FnId::new(0)).frame_size, 2);
        assert_eq!(uses(&ast, &resolution, "b"), vec![VarRef::Local(Slot(2))]);
        // A function in a block of a function has its own frame too, and
        // its enclosing frame's allocation continues after it.
        let text =
            "fn(p: Int): Int {\n  let g = fn(q: Int): Int { let r = q\nr }\n  let s = p\n  s\n}";
        let (ast, resolution) = resolved(text);
        assert_eq!(resolution.function(FnId::new(0)).frame_size, 2);
        assert_eq!(resolution.function(FnId::new(1)).frame_size, 3);
        assert_eq!(uses(&ast, &resolution, "s"), vec![VarRef::Local(Slot(2))]);
        assert_eq!(resolution.function(FnId::new(1)).callee, None);
    }

    #[test]
    fn a_function_sees_itself_by_the_name_it_initialises() {
        // A `let`, a field and a named function item.
        for text in [
            "let fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }",
            "{ fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } } }",
            "fn fib(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }",
            "let fib: fn(Int): Int = fn(n: Int): Int { fib(n - 1) + fib(n - 2) }",
        ] {
            let (ast, resolution) = resolved(text);
            assert_eq!(
                uses(&ast, &resolution, "fib"),
                vec![VarRef::Callee, VarRef::Callee],
                "{text}"
            );
            let fib = ast.names().lookup("fib");
            assert_eq!(resolution.function(FnId::new(0)).callee, fib, "{text}");
            assert!(resolution.function(FnId::new(0)).captures.is_empty());
        }
        // Not through parentheses, nor for a dotted path.
        assert_eq!(
            messages("let f = (fn(): Int { f() })"),
            vec!["undefined variable `f`"]
        );
        assert_eq!(
            messages("let a.f = fn(): Int { f() }"),
            vec!["undefined variable `f`"]
        );
        // A parameter of the same name shadows it.
        let text = "fn f(f: Int): Int { f }";
        let (ast, resolution) = resolved(text);
        assert_eq!(uses(&ast, &resolution, "f"), vec![VarRef::Local(Slot(0))]);
        assert_eq!(
            shown_result("fn f(f: Int): Int { f }\nf"),
            "fn(f: Int): Int"
        );
        // A `let` in the body shadows it too.
        assert_eq!(shown_result("fn f(): Int { let f = 2\nf }\nf()"), "Int");
        // `:=` cannot rebind it.
        let text = "fn f(): Int { f := f\n1 }";
        let found = diagnostics(text);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "cannot rebind `f`: it is the function being defined"
        );
        assert_eq!(found[0].span, nth(text, "f", 2));
        assert_eq!(found[0].notes, vec!["shadow it with `let` instead"]);
        assert!(found[0].labels.is_empty());
        // No hoisting, and so no mutual recursion.
        assert_eq!(
            messages("f()\nfn f(): Int { 1 }"),
            vec!["undefined variable `f`"]
        );
        assert_eq!(
            messages("fn a(): Int { b() }\nfn b(): Int { a() }"),
            vec!["undefined variable `b`"]
        );
    }

    #[test]
    fn a_callee_is_captured_like_a_variable() {
        // A function nested in the body does not see itself by the outer
        // name, but captures the outer function.
        let text = "let f = fn(): fn(): Int { fn(): Int { f()() } }";
        let (ast, resolution) = resolved(text);
        assert_eq!(uses(&ast, &resolution, "f"), vec![VarRef::Capture(0)]);
        assert_eq!(resolution.function(FnId::new(0)).callee, None);
        assert_eq!(
            captures(text, 0),
            vec![(VarRef::Callee, "fn(): fn(): Int".to_owned())]
        );
        assert!(captures(text, 1).is_empty());
    }

    #[test]
    fn captures_are_chained_through_enclosing_functions() {
        // A top-level variable, captured once however often it is used.
        let text = "let x = 1\nlet f = fn(): Int { x + x }";
        let (ast, resolution) = resolved(text);
        assert_eq!(
            uses(&ast, &resolution, "x"),
            vec![VarRef::Capture(0), VarRef::Capture(0)]
        );
        assert_eq!(
            captures(text, 0),
            vec![(VarRef::Local(Slot(0)), "Int".to_owned())]
        );
        // A field of a record block.
        let text = "{ x = 1.5; f = fn(): Float { x } }";
        assert_eq!(
            captures(text, 0),
            vec![(VarRef::Local(Slot(0)), "Float".to_owned())]
        );
        // A parameter, through one level and through two: each function
        // captures from the one enclosing it. The innermost function is
        // number 0, since it is finished first.
        let text = "fn outer(a: Int, b: String): fn(): fn(): String {\n\
                    \x20 fn(): fn(): String { fn(): String { \"{b}{a}\" } }\n}";
        let (ast, resolution) = resolved(text);
        assert_eq!(uses(&ast, &resolution, "a"), vec![VarRef::Capture(1)]);
        assert_eq!(uses(&ast, &resolution, "b"), vec![VarRef::Capture(0)]);
        assert_eq!(
            captures(text, 0),
            vec![
                (VarRef::Capture(0), "String".to_owned()),
                (VarRef::Capture(1), "Int".to_owned())
            ]
        );
        assert_eq!(
            captures(text, 1),
            vec![
                (VarRef::Local(Slot(1)), "String".to_owned()),
                (VarRef::Local(Slot(0)), "Int".to_owned())
            ]
        );
        assert!(captures(text, 2).is_empty());
        // The callee, through one level and through two.
        let text = "fn f(n: Int): fn(): Int { fn(): Int { f(n)() } }";
        assert_eq!(
            captures(text, 0),
            vec![
                (VarRef::Callee, "fn(n: Int): fn(): Int".to_owned()),
                (VarRef::Local(Slot(0)), "Int".to_owned())
            ]
        );
        let text = "fn f(): Int {\n\
                    \x20 let g = fn(): Int {\n\
                    \x20   let h = fn(): Int { f() }\n\
                    \x20   h()\n\
                    \x20 }\n\
                    \x20 g()\n}";
        let (ast, resolution) = resolved(text);
        assert_eq!(uses(&ast, &resolution, "f"), vec![VarRef::Capture(0)]);
        assert_eq!(
            captures(text, 0),
            vec![(VarRef::Capture(0), "fn(): Int".to_owned())]
        );
        assert_eq!(
            captures(text, 1),
            vec![(VarRef::Callee, "fn(): Int".to_owned())]
        );
        // A local of the function itself is not a capture, and neither is
        // a variable shadowing a captured one.
        let text = "let x = 1\nlet f = fn(): Int { let y = x\nlet x = y\nx }";
        let (ast, resolution) = resolved(text);
        assert_eq!(
            uses(&ast, &resolution, "x"),
            vec![VarRef::Capture(0), VarRef::Local(Slot(1))]
        );
        assert_eq!(uses(&ast, &resolution, "y"), vec![VarRef::Local(Slot(0))]);
        // A dotted `let` updating a captured record reads it from the
        // captures.
        let text = "let r = { a = 1 }\nlet f = fn(): {a: Int, b: Int} { let r.b = 2\nr }";
        let (ast, resolution) = resolved(text);
        let update = ast
            .expr_ids()
            .filter_map(|id| match ast.get(id).kind {
                ExprKind::Block(body) => Some(body),
                _ => None,
            })
            .flat_map(|body| ast.items(body).iter().copied())
            .find(|&item| matches!(ast.item(item).kind, ItemKind::Let { .. }))
            .unwrap();
        assert_eq!(
            resolution.item_path(update),
            PathResolution {
                source: VarRef::Capture(0),
                existing: 1
            }
        );
        // `:=` on a captured variable is rebinding one of an enclosing
        // block.
        let text = "let x = 1\nlet f = fn(): Int { x := 2\nx }";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `x` here: it is declared in an enclosing block"
        );
        assert_eq!(
            labels(diagnostic),
            vec![(at(text, "let x = 1"), "declared here")]
        );
    }

    #[test]
    fn calling_what_is_not_a_function() {
        let text = "let x = 1\nx(2)";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(diagnostic.message, "cannot call a value of type Int");
        assert_eq!(diagnostic.span, at(text, "x(2)"));
        assert_eq!(labels(diagnostic), vec![(Span::new(10, 11), "this is Int")]);
        assert_eq!(diagnostic.notes, vec!["only functions can be called"]);
        assert_eq!(
            messages("\"s\"(1)"),
            vec!["cannot call a value of type String"]
        );
        assert_eq!(
            messages("{ x = 1 }()"),
            vec!["cannot call a value of type `{x: Int}`"]
        );
        // An untyped literal callee settles to Int, and the arguments are
        // checked on their own.
        assert_eq!(
            messages("1(2 + 1.5, 3 // 0.5)"),
            vec![
                "cannot call a value of type Int",
                "cannot floor-divide Float"
            ]
        );
        // An erroneous callee is not reported again, but its arguments are
        // checked.
        assert_eq!(
            messages("nope(1.5 // 2) + 2.5"),
            vec!["undefined variable `nope`", "cannot floor-divide Float"]
        );
    }

    #[test]
    fn too_many_arguments() {
        let text = format!("{ADD}add(1, 2, 3)");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "expected 2 arguments, found 3");
        assert_eq!(found[0].span, at(&text, "add(1, 2, 3)"));
        assert_eq!(
            labels(&found[0]),
            vec![
                (
                    nth(&text, "add", 1),
                    "the function is fn(a: Int, b: Int): Int"
                ),
                (at(&text, "3"), "unexpected argument")
            ]
        );
        // The trailing block counts, as the last.
        let text = "fn one(r: {x: Int}): Int { 1 }\none({ x = 1 }) { x = 2 }";
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(diagnostic.message, "expected 1 argument, found 2");
        assert_eq!(
            labels(diagnostic),
            vec![
                (nth(text, "one", 1), "the function is fn(r: {x: Int}): Int"),
                (at(text, "{ x = 2 }"), "unexpected argument")
            ]
        );
        // Every argument is still checked, and the call is erroneous without
        // cascading.
        assert_eq!(
            messages(&format!("{ADD}add(1, 2, 1.5 // 2) + 2.5")),
            vec!["expected 2 arguments, found 3", "cannot floor-divide Float"]
        );
        assert_eq!(
            messages("fn z(): Int { 1 }\nz(1, 2)"),
            vec!["expected 0 arguments, found 2"]
        );
    }

    #[test]
    fn named_arguments_that_match_no_parameter() {
        let text = format!("{ADD}add(1, z = 2)");
        let found = diagnostics(&text);
        assert_eq!(found[0].message, "unknown argument `z`");
        assert_eq!(found[0].span, at(&text, "z"));
        assert_eq!(
            labels(&found[0]),
            vec![(
                nth(&text, "add", 1),
                "the function is fn(a: Int, b: Int): Int"
            )]
        );
        // `b` is then missing.
        assert_eq!(found[1].message, "missing argument `b`");
        assert_eq!(found.len(), 2);
        // A function whose parameters have no names.
        let text = format!("{ADD}let f: fn(Int, Int): Int = add\nf(1, x = 2)");
        let found = diagnostics(&text);
        assert_eq!(
            found[0].message,
            "the parameters of this function have no names"
        );
        assert_eq!(found[0].span, last(&text, "x"));
        let callee = last(&text, "f(");
        assert_eq!(
            labels(&found[0]),
            vec![(
                Span::new(callee.start, callee.start + 1),
                "this is fn(Int, Int): Int"
            )]
        );
        assert_eq!(found[0].notes, vec!["call it with positional arguments"]);
        assert_eq!(found[1].message, "missing argument 2 of type Int");
        // With some names, a named argument matches a named parameter only.
        let with_b = format!("{ADD}let f: fn(Int, b: Int): Int = add\n");
        assert_eq!(result_type(&format!("{with_b}f(b = 2, 1)")), TyId::INT);
        assert_eq!(
            messages(&format!("{with_b}f(1, c = 2)")),
            vec!["unknown argument `c`", "missing argument `b`"]
        );
        // With no parameters at all, a name is unknown.
        assert_eq!(
            messages("fn z(): Int { 1 }\nz(x = 1)"),
            vec!["unknown argument `x`"]
        );
    }

    #[test]
    fn a_parameter_given_twice() {
        // Named after positional.
        let text = format!("{ADD}add(1, a = 2, b = 3)");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "argument `a` is given twice");
        assert_eq!(found[0].span, at(&text, "a = 2"));
        assert_eq!(
            labels(&found[0]),
            vec![(at(&text, "1, a"), "first given here")]
                .into_iter()
                .map(|(span, label)| (Span::new(span.start, span.start + 1), label))
                .collect::<Vec<_>>()
        );
        // Positional after named.
        let text = format!("{ADD}add(a = 1, 2, b = 3)");
        let found = diagnostics(&text);
        assert_eq!(found[0].message, "argument `a` is given twice");
        assert_eq!(found[0].span, at(&text, "2"));
        assert_eq!(
            labels(&found[0]),
            vec![(at(&text, "a = 1"), "first given here")]
        );
        // Named twice.
        assert_eq!(
            messages(&format!("{ADD}add(b = 1, b = 2, a = 3)")),
            vec!["argument `b` is given twice"]
        );
        // The trailing block and a named argument for the last parameter.
        let text = "fn g(a: Int, r: {x: Int}): Int { a }\ng(1, r = { x = 1 }) { x = 2 }";
        let found = diagnostics(text);
        assert_eq!(found[0].message, "argument `r` is given twice");
        assert_eq!(found[0].span, at(text, "{ x = 2 }"));
        assert_eq!(
            labels(&found[0]),
            vec![(at(text, "r = { x = 1 }"), "first given here")]
        );
        // An unnamed parameter given twice is numbered.
        let text = format!("{ADD}let f: fn(Int, a: Int): Int = add\nf(a = 1, 2, 3)");
        assert_eq!(messages(&text), vec!["argument `a` is given twice"]);
    }

    #[test]
    fn parameters_left_unfilled() {
        let three = "fn three(a: Int, b: Int, c: Int): Int { a }\n";
        // With positional arguments only, the call is short as a whole.
        let text = format!("{three}three(1)");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "expected 3 arguments, found 1");
        assert_eq!(found[0].span, at(&text, "three(1)"));
        assert_eq!(
            labels(&found[0]),
            vec![(
                nth(&text, "three", 1),
                "the function is fn(a: Int, b: Int, c: Int): Int"
            )]
        );
        assert_eq!(
            messages(&format!("{three}three()")),
            vec!["expected 3 arguments, found 0"]
        );
        // With a named argument, each missing parameter, at the `)`.
        let text = format!("{three}three(c = 1)");
        let found = diagnostics(&text);
        let shown: Vec<(&str, Span)> = found.iter().map(|d| (d.message.as_str(), d.span)).collect();
        let close = last(&text, ")");
        assert_eq!(
            shown,
            vec![
                ("missing argument `a`", close),
                ("missing argument `b`", close)
            ]
        );
        assert_eq!(
            labels(&found[1]),
            vec![(
                nth(&text, "three", 1),
                "the function is fn(a: Int, b: Int, c: Int): Int"
            )]
        );
        // With a trailing block, at its `}`; an unnamed parameter by its
        // number and type.
        let text = "fn g(a: Int, b: {}, c: Int): Int { a }\n\
                    let f: fn(Int, {}, Int): Int = g\nf(1) { 3 }";
        let found = diagnostics(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "missing argument 2 of type `{}`");
        assert_eq!(found[0].span, last(text, "}"));
        let callee = last(text, "f(");
        assert_eq!(
            labels(&found[0]),
            vec![(
                Span::new(callee.start, callee.start + 1),
                "the function is fn(Int, {}, Int): Int"
            )]
        );
    }

    #[test]
    fn arguments_are_checked_against_their_parameters() {
        let text = format!("{ADD}add(1, \"s\")");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "mismatched types");
        assert_eq!(found[0].span, at(&text, "\"s\""));
        assert_eq!(
            found[0].label.as_deref(),
            Some("expected Int, found String")
        );
        assert_eq!(
            labels(&found[0]),
            vec![(
                nth(&text, "add", 1),
                "the function is fn(a: Int, b: Int): Int"
            )]
        );
        // Literals adapt, and subtypes are accepted.
        let ast = parse_text("fn half(x: Float): Float { x / 2 }\nhalf(3)");
        let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
        let three = ast
            .expr_ids()
            .find(|&id| ast.get(id).kind == ExprKind::Int(3))
            .unwrap();
        assert_eq!(checked.types.get(three), TyId::FLOAT);
        assert_eq!(
            result_type("fn x(r: {x: Int}): Int { r.x }\nlet p = { x = 1; y = 2 }\nx(p)"),
            TyId::INT
        );
        // A function argument, by the function subtyping rules.
        assert_eq!(
            result_type(
                "fn apply(f: fn(Int) -> Int, v: Int) -> Int { f(v) }\n\
                 apply(fn(n: Int): Int { n + 1 }, 1)"
            ),
            TyId::INT
        );
        assert_eq!(
            messages(
                "fn apply(f: fn(Int): Int): Int { f(1) }\n\
                 apply(fn(n: Int) -> Int { n })"
            ),
            vec!["mismatched types"]
        );
    }

    #[test]
    fn binding_arguments_to_parameters() {
        let service =
            "fn service(name: String, config: {port: Int, image: String}): String { name }\n";
        let bound = |text: &str| {
            let (ast, resolution) = resolved(text);
            let call = ast
                .expr_ids()
                .filter(|&id| matches!(ast.get(id).kind, ExprKind::Call { .. }))
                .last()
                .unwrap();
            let ExprKind::Call { args, .. } = ast.get(call).kind else {
                unreachable!();
            };
            resolution.arg_params(args).to_vec()
        };
        // The trailing block fills the last parameter, after positional or
        // named arguments.
        assert_eq!(
            bound(&format!(
                "{service}service(\"web\") {{ port = 80; image = \"nginx\" }}"
            )),
            vec![0, 1]
        );
        assert_eq!(
            bound(&format!(
                "{service}service(name = \"web\") {{ port = 80; image = \"nginx\" }}"
            )),
            vec![0, 1]
        );
        // Named arguments in any order, and mixed with positional ones.
        let f = "fn f(a: Int, b: String, c: Bool): Int { a }\n";
        assert_eq!(
            bound(&format!("{f}f(c = true, a = 1, b = \"s\")")),
            vec![2, 0, 1]
        );
        assert_eq!(
            bound(&format!("{f}f(1, c = true, b = \"s\")")),
            vec![0, 2, 1]
        );
        assert_eq!(bound(&format!("{f}f(c = true, 1, \"s\")")), vec![2, 0, 1]);
        // A call with only a trailing block.
        assert_eq!(bound("fn run(body: {}): Int { 1 }\nrun { }"), vec![0]);
        // A trailing block is checked against a record parameter field by
        // field.
        let text = format!("{service}service(\"web\") {{ port = 80 }}");
        let diagnostic = &diagnostics(&text)[0];
        assert_eq!(diagnostic.message, "missing field `image` in record");
        assert_eq!(
            labels(diagnostic),
            vec![(
                nth(&text, "service", 1),
                "the function is fn(name: String, config: {image: String, port: Int}): String"
            )]
        );
        let text = format!("{service}service(\"web\") {{ port = 80; image = \"x\"; extra = 1 }}");
        let warning = &warnings(&text)[0];
        assert_eq!(
            warning.message,
            "field `extra` is not in `{image: String, port: Int}`"
        );
        // An erroneous call binds nothing.
        let ast = parse_text(&format!("{ADD}add(1, 2, 3)"));
        let mut types = TypeTable::new();
        let mut checker = Checker::new(&ast, &mut types);
        let ExprKind::Call { args, .. } = ast.get(result_expr(&ast)).kind else {
            panic!("expected a call");
        };
        checker.check_expr(result_expr(&ast));
        assert!(
            checker.arg_params[args.start()..args.start() + args.count()]
                .iter()
                .all(|&param| param == u32::MAX)
        );
    }

    #[test]
    fn calls_have_their_return_type() {
        assert_eq!(shown_result(&format!("{ADD}add(1, 2)")), "Int");
        assert_eq!(shown_result("fn r(): {x: Int} { x = 1 }\nr()"), "{x: Int}");
        assert_eq!(shown_result("fn r(): {x: Int} { x = 1 }\nr().x"), "Int");
        assert_eq!(
            shown_result("fn make(): fn(Int): Int { fn(n: Int): Int { n } }\nmake()(1)"),
            "Int"
        );
        assert_eq!(
            shown_result("let m = { square = fn(v: Int): Int { v * v } }\nm.square(4)"),
            "Int"
        );
        // A call's value is used like any other.
        assert_eq!(
            messages(&format!("{ADD}add(1, 2) + 1.5")),
            vec!["cannot add Int and Float"]
        );
        assert_eq!(
            messages(&format!("{ADD}add(1, 2)\n1")),
            vec!["unused value of type Int"]
        );
        assert_eq!(result_type("fn tick() -> {} {}\ntick()\n1"), TyId::INT);
    }

    #[test]
    fn a_pure_function_cannot_call_an_impure_one() {
        let text = "fn tick() -> Int { 1 }\nfn pure(): Int { tick() }";
        let found = diagnostics(text);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "cannot call an impure function from a pure function"
        );
        assert_eq!(found[0].span, nth(text, "tick()", 1));
        assert_eq!(
            labels(&found[0]),
            vec![
                (last(text, "tick"), "this is fn() -> Int"),
                (last(text, ":"), "this function is pure, declared with `:`")
            ]
        );
        assert_eq!(
            found[0].notes,
            vec![
                "a pure function, declared with `:`, may call only pure functions; declare it \
                 with `->` to make it impure"
            ]
        );
        let tick = "fn tick() -> Int { 1 }\n";
        // At any depth of blocks in the pure function.
        assert_eq!(
            messages(&format!(
                "{tick}fn pure(): Int {{ if true {{ {{ let x = tick()\nx }} }} else {{ 0 }} }}"
            )),
            vec!["cannot call an impure function from a pure function"]
        );
        // A pure function in an impure one is pure.
        assert_eq!(
            messages(&format!(
                "{tick}fn outer() -> Int {{ let g = fn(): Int {{ tick() }}\ng() }}"
            )),
            vec!["cannot call an impure function from a pure function"]
        );
        // Receiving an impure function and calling it.
        assert_eq!(
            messages("fn call(f: fn() -> Int): Int { f() }"),
            vec!["cannot call an impure function from a pure function"]
        );
        // Allowed: pure in impure, pure in pure, impure in impure, impure
        // at the top level, and a pure function creating, storing,
        // receiving and returning impure ones.
        for text in [
            format!("{tick}fn pure(): Int {{ 1 }}\nfn impure() -> Int {{ pure() + tick() }}"),
            format!("{tick}fn a(): Int {{ 1 }}\nfn b(): Int {{ a() }}"),
            format!("{tick}tick()"),
            format!("{tick}fn make(): fn() -> Int {{ fn() -> Int {{ tick() }} }}\nmake()()"),
            format!(
                "{tick}fn keep(f: fn() -> Int): {{f: fn() -> Int}} {{ f = f }}\nkeep(tick).f()"
            ),
        ] {
            assert_eq!(
                check(&parse_text(&text), &mut TypeTable::new()).err(),
                None,
                "{text}"
            );
        }
        // The purity of a call, not of its arguments: a pure function may
        // pass an impure one along.
        assert_eq!(
            messages(&format!(
                "{tick}fn pass(f: fn() -> Int): fn() -> Int {{ f }}\nfn p(): Int {{ pass(tick)() }}"
            )),
            vec!["cannot call an impure function from a pure function"]
        );
    }

    #[test]
    fn functions_have_no_equality() {
        for op in ["==", "!=", "===", "!=="] {
            let text = format!("let f = fn(x: Int): Int {{ x }}\nf {op} f");
            let found = diagnostics(&text);
            assert_eq!(found.len(), 1, "{text}");
            assert_eq!(found[0].message, "cannot compare functions");
            let operator = last(&text, op);
            assert_eq!(found[0].span, operator);
            let lhs = Span::new(operator.start - 2, operator.start - 1);
            let rhs = Span::new(operator.end + 1, operator.end + 2);
            assert_eq!(
                labels(&found[0]),
                vec![
                    (lhs, "this is fn(x: Int): Int"),
                    (rhs, "this is fn(x: Int): Int")
                ]
            );
            // Nor do records containing them, at any depth.
            let text = format!("let r = {{ f = fn(): Int {{ 1 }} }}\nr {op} r");
            let found = diagnostics(&text);
            assert_eq!(
                found[0].message,
                "cannot compare `{f: fn(): Int}`: it contains a function"
            );
            assert_eq!(labels(&found[0])[0].1, "this is {f: fn(): Int}");
            let text = format!("let r = {{ a = {{ b = {{ f = fn() -> {{}} {{}} }} }} }}\nr {op} r");
            assert_eq!(
                messages(&text),
                vec!["cannot compare `{a: {b: {f: fn() -> {}}}}`: it contains a function"]
            );
        }
        // The operands must first have one identical type.
        assert_eq!(
            messages("let f = fn(): Int { 1 }\nlet g = fn(): Float { 1.0 }\nf == g"),
            vec!["cannot compare `fn(): Int` and `fn(): Float`"]
        );
        assert_eq!(
            messages("let f = fn(): Int { 1 }\nf == 1"),
            vec!["cannot compare `fn(): Int` and Int"]
        );
        // Nor an order.
        assert_eq!(
            messages("let f = fn(): Int { 1 }\nf < f"),
            vec!["cannot order `fn(): Int`"]
        );
    }

    #[test]
    fn functions_with_other_operators() {
        assert_eq!(
            messages("fn(): Int { 1 } + 1"),
            vec!["cannot add `fn(): Int` and Int"]
        );
        assert_eq!(
            messages("let f = fn(): Int { 1 }\nf.x"),
            vec!["cannot access field `x` of `fn(): Int`"]
        );
        assert_eq!(
            messages("if fn(): Bool { true } { 1 } else { 2 }"),
            vec!["mismatched types"]
        );
        // A function can be shown as text.
        assert_eq!(result_type("\"{fn(): Int { 1 }}\""), TyId::STRING);
        // `if` branches of function types join.
        assert_eq!(
            shown_result(
                "let f = if true { fn(a: Int): Int { a } } else { fn(b: Int) -> Int { b } }\nf"
            ),
            "fn(Int) -> Int"
        );
        assert_eq!(
            shown_result(
                "let f = if true { fn(a: Int): {x: Int, y: Int} { x = a; y = a } } \
                 else { fn(a: Int): {x: Int} { x = a } }\nf"
            ),
            "fn(a: Int): {x: Int}"
        );
        assert_eq!(
            messages("if true { fn(a: Int): Int { a } } else { fn(): Int { 1 } }"),
            vec!["`if` branches have different types"]
        );
        // Annotations and ascriptions with function types.
        assert_eq!(
            shown_result("let f: fn(Int) -> Int = fn(x: Int): Int { x }\nf"),
            "fn(Int) -> Int"
        );
        assert_eq!(
            messages("let f: fn(Int): Int = fn(x: Int) -> Int { x }"),
            vec!["mismatched types"]
        );
        assert_eq!(
            messages("let f: fn(Nope): Int = g"),
            vec!["unknown type `Nope`", "undefined variable `g`"]
        );
    }
}
