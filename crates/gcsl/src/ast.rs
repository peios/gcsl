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
//! type expressions, [`Ast::field_tys`], the segments of the paths that
//! name what a `let`, a field or a `:=` declares, [`Ast::path`], the
//! parameters of function expressions, [`Ast::params`], the arguments of
//! calls, [`Ast::args`], the parameters of function types,
//! [`Ast::param_tys`], and the elements of list literals, [`Ast::elements`];
//! and the details of function expressions and `for` expressions have
//! arenas of their own (see [Functions](#functions) and [Lists](#lists)).
//! So do import items, [`Ast::import_decl`]: an [`ItemKind::Import`] holds
//! only its [`ImportId`], so that an [`Item`] stays 36 bytes; the path is
//! in the path pool and the members of a `from` import in a pool of their
//! own, [`Ast::import_members`].
//!
//! # Lists
//!
//! A list literal, `[1, ...xs, 3]`, [`ExprKind::List`], lists its elements
//! in source order, each an expression or a spread of one ([`Element`]). An
//! index, `xs[0]`, is [`ExprKind::Index`]. A list type is written
//! `List<Int>`, an applied type like `Future<Int>`, or `Int[]`, which the
//! tree keeps as written, [`TypeExprKind::List`], for printing; the checker
//! gives both the same meaning.
//!
//! A spread is also an item of a block, `...EXPR`, [`ItemKind::Spread`],
//! which defines the fields of the record `EXPR` is in the block's record
//! (see [`crate::check`]).
//!
//! A range, `a..b` or `a..=b`, is a binary operator, [`BinaryOp::Range`]
//! or [`BinaryOp::RangeInclusive`]. A `for` expression, `for x in xs if
//! GUARD { ... }`, is one of the loops (see [Loops](#loops)).
//!
//! # Loops
//!
//! Every loop is an expression, [`ExprKind::For`], whose details do not
//! fit in an [`Expr`], so its node holds only a [`ForId`] into an arena of
//! [`ForDecl`]s, [`Ast::for_decl`]: its label, if any ([`Label`]), its
//! body, a block, and its form, a [`ForHead`]: an iteration, `for x in xs
//! if GUARD { ... }`, with its variable's name and where it is written, its
//! iterable and its guard, if any; a condition loop, `for BEFORE; COND;
//! AFTER { ... }`, with the items before its condition and after it, each
//! a list of items in the item pool, and its condition, `for { ... }`
//! having none; or a `do` loop, `do { ... } for COND`, with its condition.
//!
//! `break` and `continue` are items, [`ItemKind::Break`] and
//! [`ItemKind::Continue`], each with a [`Jump`]: the label of the loop it
//! is for, if it names one, and its value, if it has one. So is `return`,
//! [`ItemKind::Return`], whose [`Jump`]'s label names the block or loop it
//! leaves, and which without one leaves the function or `lazy` around it.
//!
//! # Labelled blocks
//!
//! A block may be labelled, `{'name ... }`, the label written directly
//! after its `{`: [`ExprKind::Block`] holds its items and, if it has one,
//! a [`LabelId`] into an arena of [`Label`]s, [`Ast::label`]. The label is
//! kept apart because a [`Label`] would not fit in an [`Expr`] beside the
//! items, and only labelled blocks pay for it: an id with a niche, so that
//! a block without a label costs nothing and an [`Expr`] stays 24 bytes.
//! Loop labels and block labels are one namespace (see [`crate::check`]).
//!
//! # Functions
//!
//! A function expression, `fn(x: Int): Int { ... }`, has more to it than
//! fits in an [`Expr`], so its node, [`ExprKind::Fn`], holds only a
//! [`FnId`] into an arena of [`FnDecl`]s, [`Ast::fn_decl`]: its parameters,
//! return type, [`Effect`] and body. Every function expression of a tree has
//! an id, from 0 to [`Ast::fn_count`], which later stages use to index
//! their own per-function tables. A named function item, `fn f(...) ...`,
//! is a `let` whose value is a function expression, and a public function
//! item, `def f(...) ...`, is a field whose value is one; each is marked as
//! written (`function` on [`ItemKind::Let`] and [`ItemKind::Field`]), so the
//! tree tells `fn f`, `def f`, `let f = fn` and `f = fn` apart, and the
//! printer prints each as it was written. A call, [`ExprKind::Call`], lists its arguments in
//! source order, each positional, named or the trailing block, which is
//! always last.
//!
//! A `lazy EXPR`, [`ExprKind::Lazy`], is a deferred expression: its operand
//! is the body of an anonymous, parameterless function, run when the
//! future it builds is first awaited. It is a function of the same arena,
//! with its own [`FnId`] among the functions' (one table for both): an
//! [`FnDecl`] with no parameters, no return type, impure, whose marker is
//! the `lazy` keyword and whose body is the operand, any expression. `await
//! EXPR` is a prefix operator, [`UnaryOp::Await`].
//!
//! # Paths
//!
//! A `let`, a field item and a `:=` each declare a [path](PathRange): a
//! name, an identifier or a plain string literal, followed by any number of
//! segments, each `.` and a name, as in `x`, `server.port` or `"log
//! level".format`, or, in the path of a `:=` only, `[EXPR]`, the position of
//! an element of a list, as in `xs[i]`, `conf.hosts[0].up` or
//! `grid[y][x]`:
//!
//! ```text
//! PATH    = NAME SEGMENT*
//! SEGMENT = "." NAME | "[" EXPR "]"
//! ```
//!
//! Each segment is a [`PathSegment`], of a [`SegmentKind`]. An index
//! segment's position is an expression of the tree like any other, and the
//! source text it is written as is kept in the [`Strings`] table, so that a
//! path can be shown as it is written ([`Ast::show_path`]) without the
//! source. A path of one segment is the plain case, declaring that name; a
//! longer one is a dotted definition or update, whose meaning the checker
//! gives (see [`crate::check`]).
//!
//! # Mutable variables
//!
//! `let mut NAME = EXPR` declares a *mutable* variable, which `:=` may
//! rebind from any block of the function declaring it (see
//! [`crate::check`]). It is an [`ItemKind::Let`] like any other, marked as
//! mutable, whose path is always a single name.
//!
//! # Type tests
//!
//! `EXPR is TYPE`, [`ExprKind::Is`], tests whether a value is of a type, and
//! `EXPR as TYPE`, [`ExprKind::As`], narrows it to the type, failing at
//! runtime if it is not one; each holds its expression and its type, as an
//! ascription does (see [`crate::check`]).
//!
//! # Attributes
//!
//! An item of a file's outermost block may be marked `#[shadowable]` (see
//! the parser's [Attributes](crate::parser#attributes)), and the file
//! itself `#![no_prelude]`. Neither fits in an [`Item`], which stays 36
//! bytes: the marked items are kept in a side table of the tree
//! ([`Ast::is_shadowable`]), and the file's attribute as the span it is
//! written at ([`Ast::no_prelude`]). A workspace gives every file whose
//! tree has no `#![no_prelude]` an *implicit* `#[shadowable] import * from
//! prelude` ([`Ast::add_implicit_import`]): an import item of the tree's
//! arenas, written nowhere (its spans are `0..0`), which is not among the
//! items of the file's body, so that no item's position, span or line moves,
//! and which the workspace takes as the first of the file's imports
//! ([`Ast::implicit_import`]; see [`crate::workspace`]). The printer shows
//! the attributes as written and nothing of the implicit import.
//!
//! An [`Ast`] is built through an [`AstBuilder`]: nodes are allocated
//! children-first, and the builder is finished once the root is known, so an
//! `Ast` always has a valid root. The `Ast` also owns the [`Names`] table its
//! identifiers were interned into and the [`Strings`] table holding the text
//! of its string literals.

use std::fmt;
use std::num::NonZeroU32;

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
    /// The item numbered `number`. Ids are normally assigned by the parser;
    /// this is for finding items by number in tests.
    #[cfg(test)]
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

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

/// The segments of a path: a contiguous range of the path pool of the
/// [`Ast`] that allocated it, never empty. Resolve it with [`Ast::path`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathRange {
    start: u32,
    len: u32,
}

/// Index of a function expression's [`FnDecl`] in the function arena of the
/// [`Ast`] (or [`AstBuilder`]) that allocated it. Like [`ExprId`], only
/// meaningful for that arena; later stages index their per-function tables
/// by it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FnId(u32);

impl FnId {
    /// The function numbered `number`. Ids are normally assigned by the
    /// parser; this is for building trees and IR by hand in tests.
    #[cfg(test)]
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The id as an index into the arena and into side tables.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for FnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Index of a `for` expression's [`ForDecl`] in the `for` arena of the
/// [`Ast`] (or [`AstBuilder`]) that allocated it. Like [`ExprId`], only
/// meaningful for that arena; later stages index their per-`for` tables by
/// it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ForId(u32);

impl ForId {
    /// The `for` numbered `number`. Ids are normally assigned by the
    /// parser; this is for tests.
    #[cfg(test)]
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The id as an index into the arena and into side tables.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of an import item's [`ImportDecl`] in the import arena of the
/// [`Ast`] (or [`AstBuilder`]) that allocated it. Like [`ExprId`], only
/// meaningful for that arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImportId(u32);

impl ImportId {
    /// The id as an index into the arena and into side tables.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A name written in an import item, with where it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImportName {
    /// The name.
    pub name: Name,
    /// Where the name is written.
    pub span: Span,
}

/// One member of `import MEMBER, ... from PATH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportMember {
    /// `NAME` or `NAME as ALIAS`: the root `NAME` of what the path names,
    /// bound as `ALIAS` if one is given, else as `NAME`.
    Named {
        /// The root imported.
        name: ImportName,
        /// The name it is bound as, if written with `as`.
        alias: Option<ImportName>,
    },
    /// `*`, written at the span: every root of what the path names, each
    /// bound as itself.
    All(Span),
}

impl ImportMember {
    /// The name the member binds, or `None` for `*`.
    #[must_use]
    pub const fn bound(&self) -> Option<ImportName> {
        match *self {
            ImportMember::Named { name, alias } => match alias {
                Some(alias) => Some(alias),
                None => Some(name),
            },
            ImportMember::All(_) => None,
        }
    }

    /// Where the member is written: its name through its alias, or the
    /// `*`.
    #[must_use]
    pub const fn span(&self) -> Span {
        match *self {
            ImportMember::Named { name, alias } => match alias {
                Some(alias) => name.span.join(alias.span),
                None => name.span,
            },
            ImportMember::All(span) => span,
        }
    }
}

/// The members of one `from` import: a contiguous range of the member pool
/// of the [`Ast`] that allocated it, never empty. Resolve it with
/// [`Ast::import_members`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImportMemberRange {
    start: u32,
    len: u32,
}

/// What an import item binds, [`ImportDecl::form`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportForm {
    /// `import PATH` or `import PATH as NAME`: what the path names, bound
    /// as `NAME` if one is given, else as the path's last segment.
    Whole(Option<ImportName>),
    /// `import MEMBER, ... from PATH`: members of what the path names.
    Members(ImportMemberRange),
}

/// An import item (see the parser's [Imports](crate::parser#imports)):
/// held in the [`Ast`]'s import arena; the item is an [`ItemKind::Import`]
/// with its [`ImportId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImportDecl {
    /// The module path: one or more name segments, absolute from the
    /// workspace root, never an index segment.
    pub path: PathRange,
    /// What the import binds.
    pub form: ImportForm,
}

/// A loop: a `for` expression of any form, or a `do` loop (see
/// [Loops](#loops)). Held in the [`Ast`]'s `for` arena; the expression node
/// is an [`ExprKind::For`] with its [`ForId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ForDecl {
    /// The loop's label, written directly after its `for` or `do`, if it
    /// has one.
    pub label: Option<Label>,
    /// What runs around the body: the form of the loop.
    pub head: ForHead,
    /// The body, an [`ExprKind::Block`], whose value each iteration
    /// contributes, unless a `break` or `continue` says otherwise.
    pub body: ExprId,
}

/// The form of a loop, [`ForDecl::head`]: what it runs around its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForHead {
    /// `for NAME in ITERABLE BLOCK` or `for NAME in ITERABLE if GUARD
    /// BLOCK`: an iteration over a list.
    Each {
        /// The element variable's name, always an identifier.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// What is iterated over, a list.
        iterable: ExprId,
        /// The guard after `if`, if any, which decides whether an element
        /// contributes.
        guard: Option<ExprId>,
    },
    /// `for BEFORE; COND; AFTER BLOCK`, each of `BEFORE` and `AFTER` items
    /// separated by `;` and either series possibly empty, so `for COND
    /// BLOCK` too; and `for BLOCK`, with no condition, which loops until
    /// something leaves it.
    While {
        /// The items before the condition, run once: `let`s and `:=`s.
        before: ItemRange,
        /// The condition, tested before each iteration; `None` for `for
        /// BLOCK`, which is the loop whose condition is `true`.
        cond: Option<ExprId>,
        /// The items after the condition, run after each iteration's body:
        /// `:=`s only.
        after: ItemRange,
    },
    /// `do BLOCK for COND`: the body, then the condition, and the body
    /// again while it is true.
    Do {
        /// The condition, tested after each iteration.
        cond: ExprId,
    },
}

/// A label, `'NAME`, written directly after the `for`, `do`, `break`,
/// `continue` or `return` it belongs to, or after the `{` of the block it
/// names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label {
    /// The name, without its `'`.
    pub name: Name,
    /// Where the label is written, its `'` included.
    pub span: Span,
}

/// Index of a labelled block's [`Label`] in the label arena of the [`Ast`]
/// (or [`AstBuilder`]) that allocated it. Never zero, so that an
/// `Option<LabelId>` is as small as the id, which keeps an unlabelled
/// [`ExprKind::Block`] free of the label's cost. Like [`ExprId`], only
/// meaningful for that arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LabelId(NonZeroU32);

impl LabelId {
    /// The id as an index into the arena.
    const fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

/// What a `break`, `continue` or `return` item says: the loop, or for a
/// `return` the block or loop, it is for, by label, and its value: what it
/// contributes to that loop's list, or what it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Jump {
    /// The label of the loop or block it is for; `None` for the innermost
    /// loop whose body holds it, or for a `return`, the innermost function
    /// or `lazy`.
    pub label: Option<Label>,
    /// Its value, if it has one.
    pub value: Option<ExprId>,
}

/// The parameters of a function expression: a contiguous range of the
/// parameter pool of the [`Ast`] that allocated it. Resolve it with
/// [`Ast::params`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamRange {
    start: u32,
    len: u32,
}

impl ParamRange {
    /// The number of parameters.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    /// Whether there are no parameters.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// The arguments of a call: a contiguous range of the argument pool of the
/// [`Ast`] that allocated it. Resolve it with [`Ast::args`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArgRange {
    start: u32,
    len: u32,
}

impl ArgRange {
    /// The index of the range's first argument in the argument pool, which
    /// side tables indexed like the pool (see
    /// [`crate::check::Resolution::arg_params`]) share.
    pub(crate) const fn start(self) -> usize {
        self.start as usize
    }

    /// The number of arguments.
    pub(crate) const fn count(self) -> usize {
        self.len as usize
    }
}

/// The parameters of a function type expression: a contiguous range of the
/// parameter-type pool of the [`Ast`] that allocated it. Resolve it with
/// [`Ast::param_tys`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamTyRange {
    start: u32,
    len: u32,
}

/// The elements of a list literal: a contiguous range of the element pool
/// of the [`Ast`] that allocated it. Resolve it with [`Ast::elements`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElementRange {
    start: u32,
    len: u32,
}

/// One element of a list literal: an expression, whose value is the
/// element, or `...` and an expression, whose value is a list whose
/// elements are spread into the literal in its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Element {
    /// The expression.
    pub value: ExprId,
    /// For a spread, where its `...` is written; `None` for an element
    /// that is not one.
    pub spread: Option<Span>,
}

impl Element {
    /// The span of the whole element in `ast`: its `...` through its
    /// expression for a spread, and its expression otherwise.
    #[must_use]
    pub fn span(&self, ast: &Ast) -> Span {
        let value = ast.get(self.value).span;
        match self.spread {
            Some(spread) => Span::join(spread, value),
            None => value,
        }
    }
}

/// One parameter of a function expression, `NAME: TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Param {
    /// The parameter's name, always an identifier.
    pub name: Name,
    /// Where it is written: its name through its type.
    pub span: Span,
    /// Its type.
    pub ty: TypeExprId,
}

/// What a function may do, as the marker before its return type declares
/// it, in a function expression and a function type alike: `:` for pure,
/// `->>` for impure and memoised, `->` for impure.
///
/// Ordered `Pure < Memo < Impure`, from the function that promises most to
/// the one that promises least, which is the order in which one can stand
/// in for another (see [`crate::types`]). Only `Pure` is pure: a memoised
/// function may do everything an impure one may, and runs its body at most
/// once per distinct arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    /// Pure, declared with `:`: it may neither call an impure function nor
    /// await.
    Pure,
    /// Impure and memoised, declared with `->>`: each closure runs its body
    /// at most once per distinct list of arguments.
    Memo,
    /// Impure, declared with `->`, run on every call.
    Impure,
}

impl Effect {
    /// The text between a function's parameters and its return type, as a
    /// type is shown: `"): "`, `") ->> "` or `") -> "`, the `)` closing the
    /// parameters included.
    #[must_use]
    pub const fn params_end(self) -> &'static str {
        match self {
            Effect::Pure => "): ",
            Effect::Memo => ") ->> ",
            Effect::Impure => ") -> ",
        }
    }

    /// The effect's name, as the tree's and the IR's debug forms show it:
    /// `pure`, `memo` or `impure`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Effect::Pure => "pure",
            Effect::Memo => "memo",
            Effect::Impure => "impure",
        }
    }
}

/// A function expression, `fn (PARAMS) : RET BODY`, `fn (PARAMS) ->> RET
/// BODY` or `fn (PARAMS) -> RET BODY`, or the body of a `lazy`. Held in the
/// [`Ast`]'s function arena; the expression node is an [`ExprKind::Fn`] or
/// an [`ExprKind::Lazy`] with its [`FnId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FnDecl {
    /// The parameters, in order; none for a `lazy`.
    pub params: ParamRange,
    /// The declared return type; `None` for a `lazy`, which declares none.
    pub ret: Option<TypeExprId>,
    /// What the function may do, as its marker declares: pure (`:`),
    /// memoised (`->>`) or impure (`->`). A `lazy` is always
    /// [`Effect::Impure`].
    pub effect: Effect,
    /// The `:`, `->>` or `->` before the return type, or a `lazy`'s keyword.
    pub marker: Span,
    /// The body: an [`ExprKind::Block`] for a function expression, and any
    /// expression, the operand, for a `lazy`.
    pub body: ExprId,
}

/// How an [`Arg`] is passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArgKind {
    /// `EXPR`, filling the parameters left to right.
    Positional,
    /// `NAME = EXPR`, filling the parameter named `NAME`.
    Named {
        /// The parameter's name.
        name: Name,
        /// Where the name is written.
        span: Span,
    },
    /// The block after the argument list, or in place of one, filling the
    /// last parameter. Always the call's last argument.
    Trailing,
}

/// One argument of a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Arg {
    /// How it is passed.
    pub kind: ArgKind,
    /// Its value; for a trailing argument, an [`ExprKind::Block`].
    pub value: ExprId,
}

impl Arg {
    /// The span of the whole argument in `ast`: its name through its value
    /// for a named argument, and its value otherwise.
    #[must_use]
    pub fn span(&self, ast: &Ast) -> Span {
        let value = ast.get(self.value).span;
        match self.kind {
            ArgKind::Named { span, .. } => Span::join(span, value),
            ArgKind::Positional | ArgKind::Trailing => value,
        }
    }
}

/// One parameter of a function type expression: `NAME: TYPE` or `TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamTy {
    /// The parameter's name, if the type gives one.
    pub name: Option<Name>,
    /// Its type.
    pub ty: TypeExprId,
}

/// One segment of a path (see [Paths](crate::ast#paths)): a name, or, in
/// the path of a `:=` and never as its first segment, the position of an
/// element of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathSegment {
    /// What the segment selects.
    pub kind: SegmentKind,
    /// Where the segment is written: a name, quotes included for a quoted
    /// one, or an index's brackets and what is between them.
    pub span: Span,
}

/// What a [`PathSegment`] selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SegmentKind {
    /// A name, written as an identifier or a plain string literal: the
    /// variable or field the path starts with, or a field of the record
    /// before it.
    Name(Name),
    /// `[EXPR]`: the element of the list before it at a position.
    Index {
        /// The position, an expression of the tree.
        position: ExprId,
        /// The source text of the position, as it is written between the
        /// brackets, for showing the path: from its first token to its
        /// last, on one line (see [`crate::parser`]).
        text: StrId,
    },
}

impl PathSegment {
    /// The segment's name, or `None` for an index segment.
    #[must_use]
    pub const fn name(&self) -> Option<Name> {
        match self.kind {
            SegmentKind::Name(name) => Some(name),
            SegmentKind::Index { .. } => None,
        }
    }

    /// The position of an index segment, or `None` for a name.
    #[must_use]
    pub const fn position(&self) -> Option<ExprId> {
        match self.kind {
            SegmentKind::Index { position, .. } => Some(position),
            SegmentKind::Name(_) => None,
        }
    }
}

/// The span of the path, or the prefix of one, made of `segments`: from its
/// first segment through its last. An empty span at offset 0 for no
/// segments, which no path has.
#[must_use]
pub fn path_span(segments: &[PathSegment]) -> Span {
    match (segments.first(), segments.last()) {
        (Some(first), Some(last)) => Span::join(first.span, last.span),
        _ => Span::new(0, 0),
    }
}

/// A path, or the prefix of one, shown as it is written: its first name,
/// then `.` and a name for each name segment, each a plain identifier or a
/// quoted string literal as [`FieldName`] shows it, and each index segment
/// in brackets, with the source text of its position, as in `server.port`,
/// `"log level".format` or `conf.hosts[i + 1].up`. Returned by
/// [`Ast::show_path`].
#[derive(Clone, Copy, Debug)]
pub struct PathDisplay<'a> {
    names: &'a Names,
    strings: &'a Strings,
    segments: &'a [PathSegment],
}

impl fmt::Display for PathDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, segment) in self.segments.iter().enumerate() {
            match segment.kind {
                SegmentKind::Name(name) => {
                    if index > 0 {
                        f.write_str(".")?;
                    }
                    FieldName(self.names.get(name)).fmt(f)?;
                }
                SegmentKind::Index { text, .. } => {
                    write!(f, "[{}]", self.strings.get(text))?;
                }
            }
        }
        Ok(())
    }
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
    /// `await x`: the value of `x`, awaited fully if it is a future.
    Await,
}

impl UnaryOp {
    /// The verb used in diagnostics, e.g. "cannot negate ..." or "`not`
    /// requires Bool operands".
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            UnaryOp::Neg => "negate",
            UnaryOp::Not => "not",
            UnaryOp::Await => "await",
        }
    }

    /// The operator as written in source.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "not",
            UnaryOp::Await => "await",
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
    /// `a == b`, typed equality: records compare only the fields of their
    /// static type.
    Eq,
    /// `a != b`, the negation of [`BinaryOp::Eq`].
    Ne,
    /// `a === b`, total equality: records compare as whole runtime values.
    Same,
    /// `a !== b`, the negation of [`BinaryOp::Same`].
    NotSame,
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
    /// `a..b`, the list of the integers from `a` up to but not including
    /// `b`.
    Range,
    /// `a..=b`, the list of the integers from `a` up to and including `b`.
    RangeInclusive,
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
            | BinaryOp::Same
            | BinaryOp::NotSame
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => "compare",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::Range | BinaryOp::RangeInclusive => "make a range of",
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
            BinaryOp::Same => "===",
            BinaryOp::NotSame => "!==",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::Range => "..",
            BinaryOp::RangeInclusive => "..=",
        }
    }

    /// Whether this is one of the range operators, `..` and `..=`.
    #[must_use]
    pub const fn is_range(self) -> bool {
        matches!(self, BinaryOp::Range | BinaryOp::RangeInclusive)
    }

    /// Whether this is one of the comparison operators, `==`, `!=`, `===`,
    /// `!==`, `<`, `<=`, `>` and `>=`.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Same
                | BinaryOp::NotSame
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
        )
    }

    /// Whether this is one of the equality operators, `==`, `!=`, `===` and
    /// `!==`, which accept operands of any one type, rather than an
    /// ordering.
    #[must_use]
    pub const fn is_equality(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Same | BinaryOp::NotSame
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
    /// least one [`ItemKind::Field`] or [`ItemKind::Spread`] it is a record
    /// block, whose value is the record of its fields. A block that is not
    /// one may be labelled, `{'name ... }`, for the `return`s that leave it
    /// (see [Labelled blocks](#labelled-blocks)).
    Block {
        /// Its items.
        items: ItemRange,
        /// Its label, if it has one: see [`Ast::label`].
        label: Option<LabelId>,
    },
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
    /// A type test `expr is Type`: whether the value is of the type.
    Is {
        /// The expression whose value is tested.
        expr: ExprId,
        /// The type tested for.
        ty: TypeExprId,
    },
    /// A checked narrowing `expr as Type`: the value, as of the type, or a
    /// runtime error if it is not.
    As {
        /// The expression whose value is narrowed.
        expr: ExprId,
        /// The type it is narrowed to.
        ty: TypeExprId,
    },
    /// A function expression, whose details are in the function arena (see
    /// [`Ast::fn_decl`]).
    Fn(FnId),
    /// A call, `callee(ARGS)`, `callee(ARGS) { ... }` or `callee { ... }`.
    Call {
        /// The function called.
        callee: ExprId,
        /// The arguments, in source order, the trailing block last.
        args: ArgRange,
    },
    /// `lazy EXPR`: a future of `EXPR`, which is not evaluated where it is
    /// written. Its details are in the function arena (see
    /// [`Ast::fn_decl`]), the operand being the body.
    Lazy(FnId),
    /// A list literal, `[a, ...b, c]`: its elements, in source order (see
    /// [`Element`]).
    List(ElementRange),
    /// An index, `EXPR[INDEX]`: the element of a list at a position. The
    /// span of the `[INDEX]` part is not stored; see [`Expr::bracket_span`].
    Index {
        /// The list.
        expr: ExprId,
        /// The position.
        index: ExprId,
    },
    /// A loop, of any form: `for NAME in ITERABLE (if GUARD)? { ... }`, the
    /// list of the body's values for the elements of the iterable that
    /// pass the guard, or a condition loop, `for { ... }` or a `do` loop,
    /// the list of what their iterations contribute. Its details are in
    /// the `for` arena (see [`Ast::for_decl`]).
    For(ForId),
}

/// One node of the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expr {
    /// What the node is.
    pub kind: ExprKind,
    /// The source it covers. A `Paren` covers its parentheses and a `Block`
    /// its braces, a `Binary` its left operand through its right operand, a
    /// `Unary` its operator through its operand, and an `Ascribe`, an `Is`
    /// and an `As` its expression through its type. A string literal covers
    /// its prefix and
    /// delimiters, an `If` its `if` keyword through its last branch, and a
    /// `Field` its record expression through its name. A `Fn` covers its
    /// `fn` keyword through its body's `}`, a `Call` its callee through
    /// its `)` or its trailing block's `}`, and a `Lazy` its `lazy` keyword
    /// through its operand. A `List` covers its brackets, an `Index` its
    /// list expression through its `]`, and a `For` its `for` keyword
    /// through its body's `}`, or, for a `do` loop, its `do` keyword
    /// through its condition.
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
            | ExprKind::Block { .. }
            | ExprKind::Field { .. }
            | ExprKind::Ascribe { .. }
            | ExprKind::Is { .. }
            | ExprKind::As { .. }
            | ExprKind::Fn(_)
            | ExprKind::Call { .. }
            | ExprKind::Lazy(_)
            | ExprKind::List(_)
            | ExprKind::Index { .. }
            | ExprKind::For(_) => None,
        }
    }

    /// The span of the `[INDEX]` part of an `Index` node, for diagnostics
    /// about the indexing: from the end of its list expression, found in
    /// `ast`, to its `]`. `None` for every other node.
    #[must_use]
    pub fn bracket_span(&self, ast: &Ast) -> Option<Span> {
        match self.kind {
            ExprKind::Index { expr, .. } => Some(Span::new(ast.get(expr).span.end, self.span.end)),
            _ => None,
        }
    }

    /// The span of a `Call` node's closing `)` or `}`: its last byte.
    /// `None` for every other node.
    #[must_use]
    pub const fn close_span(&self) -> Option<Span> {
        match self.kind {
            ExprKind::Call { .. } => {
                Some(Span::new(self.span.end.saturating_sub(1), self.span.end))
            }
            _ => None,
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

    /// The span of the `if` keyword, for `If` nodes, and of the `for`
    /// keyword, for `For` nodes, which start with one, but a `do` loop's,
    /// which starts with `do`: [`Ast::loop_keyword`] tells them apart.
    #[must_use]
    pub const fn keyword_span(&self) -> Option<Span> {
        match self.kind {
            ExprKind::If { .. } => Some(symbol_span("if", self.span.start)),
            ExprKind::For(_) => Some(symbol_span("for", self.span.start)),
            _ => None,
        }
    }
}

/// What an [`Item`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// `let PATH = EXPR` or `let PATH: TYPE = EXPR`: declares a variable,
    /// named by the path's first segment, which is always an identifier. A
    /// dotted path declares it as an update of the variable in scope, if
    /// any, with the rest of the path set, and an annotation then applies to
    /// the path's last segment. A named function item, `fn NAME(...) ...`,
    /// is one of these too: a `let` of `NAME`, without an annotation, whose
    /// value is the function expression, with `function` set. So is `let
    /// mut NAME = EXPR`, with or without an annotation, whose path is a
    /// single name.
    Let {
        /// The variable's name, then any further segments.
        path: PathRange,
        /// The annotated type, if any.
        ty: Option<TypeExprId>,
        /// The initialiser.
        value: ExprId,
        /// Whether it is a `let mut`, which declares a mutable variable.
        mutable: bool,
        /// Whether it was written as a named function item, `fn NAME(...)
        /// ...`, rather than as `let NAME = fn(...) ...`.
        function: bool,
    },
    /// `PATH = EXPR`, as in `x = 1`, `"quoted name" = 1` or `a.b = 1`:
    /// declares a field of the record its block evaluates to, named by the
    /// path's first segment, which is also a name in scope for the block's
    /// later items. A dotted path declares or extends a field that is a
    /// record built by such paths. A public function item, `def NAME(...)
    /// ...`, is one of these too: the field `NAME`, whose value is the
    /// function expression, with `function` set.
    Field {
        /// The field's name, then any further segments.
        path: PathRange,
        /// The field's value, or the value at the end of the path.
        value: ExprId,
        /// Whether it was written as a public function item, `def NAME(...)
        /// ...`, rather than as `NAME = fn(...) ...`.
        function: bool,
    },
    /// `...EXPR`, a spread: defines each field of the record `EXPR` is, by
    /// its static type, as a *soft* field of the record its block evaluates
    /// to, and as a name in scope for the block's later items, which a later
    /// item may define again.
    Spread(ExprId),
    /// `PATH := EXPR`: rebinds a variable declared in the same block, or a
    /// mutable variable of the same function, named by the path's first
    /// segment, which is always an identifier; a dotted path rebinds it to
    /// an update of its value. Its path is the only one that may have index
    /// segments, `xs[i] := v`, each of which replaces an element of a list.
    Rebind {
        /// The variable's name, then any further segments.
        path: PathRange,
        /// The new value, or the value at the end of the path.
        value: ExprId,
    },
    /// `= EXPR`: the block's result.
    Result(ExprId),
    /// An expression evaluated on its own.
    Expr(ExprId),
    /// `break`, `break VALUE`, `break'LABEL` or `break'LABEL VALUE`: ends a
    /// loop.
    Break(Jump),
    /// `continue`, with a value and a label as for `break`: ends an
    /// iteration of a loop.
    Continue(Jump),
    /// `return`, `return VALUE`, `return'LABEL` or `return'LABEL VALUE`:
    /// leaves the innermost function or `lazy`, or with a label, the block
    /// or loop it names, which evaluates to the value, `{}` without one.
    Return(Jump),
    /// `import PATH`, `import PATH as NAME` or `import MEMBER, ... from
    /// PATH`: binds private names of the file to what the path names, in
    /// the import arena (see [`Ast::import_decl`]). Only ever an item of a
    /// file's outermost block. It has no expression.
    Import(ImportId),
}

impl ItemKind {
    /// The expression the item holds, if it has one: every item's but a
    /// `break`'s, `continue`'s or `return`'s without a value, and an
    /// import's.
    #[must_use]
    pub const fn value(self) -> Option<ExprId> {
        match self {
            ItemKind::Let { value, .. }
            | ItemKind::Field { value, .. }
            | ItemKind::Rebind { value, .. }
            | ItemKind::Spread(value)
            | ItemKind::Result(value)
            | ItemKind::Expr(value) => Some(value),
            ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => jump.value,
            ItemKind::Import(_) => None,
        }
    }
}

/// One item of a block body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    /// What the item is.
    pub kind: ItemKind,
    /// The source it covers: from `let`, the path, `=`, `...` or the start
    /// of the expression, through the end of the expression; for a
    /// `break`, `continue` or `return`, from its keyword through its value,
    /// its label or itself, whichever is last.
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
    /// A function type, `fn(Int, name: String): Bool`, `fn(Int) ->> Int` or
    /// `fn() -> Int`.
    Fn {
        /// The parameters, each with its name if it has one.
        params: ParamTyRange,
        /// The return type.
        ret: TypeExprId,
        /// What it may do, as its marker says: `:`, `->>` or `->`.
        effect: Effect,
    },
    /// A name applied to a type argument, `NAME<TYPE>`, as in
    /// `Future<Int>`. The parser accepts any name; the checker knows which
    /// take an argument.
    Applied {
        /// The name.
        name: Name,
        /// Where the name is written.
        span: Span,
        /// The type argument.
        arg: TypeExprId,
    },
    /// `TYPE[]`, the sugar for the list type `List<TYPE>`: its element
    /// type. `List<TYPE>` written out is an [`TypeExprKind::Applied`].
    List(TypeExprId),
    /// A union, `TYPE | TYPE`. `|` binds looser than everything else in a
    /// type, and a union of three is the union of a union and the third.
    Union {
        /// The left member.
        lhs: TypeExprId,
        /// The right member.
        rhs: TypeExprId,
        /// Where the `|` is written.
        bar: Span,
    },
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
    paths: Vec<PathSegment>,
    fns: Vec<FnDecl>,
    params: Vec<Param>,
    args: Vec<Arg>,
    param_tys: Vec<ParamTy>,
    elements: Vec<Element>,
    fors: Vec<ForDecl>,
    labels: Vec<Label>,
    imports: Vec<ImportDecl>,
    import_members: Vec<ImportMember>,
    shadowable: Vec<ItemId>,
    no_prelude: Option<Span>,
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
        let start = next_id(self.param_tys.len());
        self.param_tys.extend_from_slice(params);
        ParamTyRange {
            start,
            len: next_id(params.len()),
        }
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

    /// Marks the item `id` `#[shadowable]` (see [Attributes](#attributes)).
    pub fn mark_shadowable(&mut self, id: ItemId) {
        let at = self.shadowable.partition_point(|&marked| marked.0 < id.0);
        if self.shadowable.get(at) != Some(&id) {
            self.shadowable.insert(at, id);
        }
    }

    /// Records that the file is marked `#![no_prelude]`, written at `span`
    /// (see [Attributes](#attributes)).
    pub fn set_no_prelude(&mut self, span: Span) {
        self.no_prelude = Some(span);
    }

    /// Where the file's `#![no_prelude]` is written, if it is marked so
    /// already.
    #[must_use]
    pub fn no_prelude(&self) -> Option<Span> {
        self.no_prelude
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
        Ast {
            exprs: self.exprs,
            items: self.items,
            item_lists: self.item_lists,
            parts: self.parts,
            type_exprs: self.type_exprs,
            field_tys: self.field_tys,
            paths: self.paths,
            fns: self.fns,
            params: self.params,
            args: self.args,
            param_tys: self.param_tys,
            elements: self.elements,
            fors: self.fors,
            labels: self.labels,
            imports: self.imports,
            import_members: self.import_members,
            shadowable: self.shadowable,
            no_prelude: self.no_prelude,
            implicit_import: None,
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
    paths: Vec<PathSegment>,
    fns: Vec<FnDecl>,
    params: Vec<Param>,
    args: Vec<Arg>,
    param_tys: Vec<ParamTy>,
    elements: Vec<Element>,
    fors: Vec<ForDecl>,
    labels: Vec<Label>,
    imports: Vec<ImportDecl>,
    import_members: Vec<ImportMember>,
    /// The items marked `#[shadowable]`, in id order.
    shadowable: Vec<ItemId>,
    /// Where the file's `#![no_prelude]` is written, if it has one.
    no_prelude: Option<Span>,
    /// The implicit `#[shadowable] import * from prelude`, if a workspace
    /// gave the file one.
    implicit_import: Option<ItemId>,
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
        let start = range.start as usize;
        &self.param_tys[start..start + range.len as usize]
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
        }
    }

    /// The file's body: the outermost block's items.
    #[must_use]
    pub fn root(&self) -> ItemRange {
        self.root
    }

    /// Whether the item `id` is marked `#[shadowable]`, the implicit import
    /// included (see [Attributes](#attributes)).
    #[must_use]
    pub fn is_shadowable(&self, id: ItemId) -> bool {
        self.shadowable
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
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

    /// The implicit `#[shadowable] import * from prelude` a workspace gave
    /// the file, if it has one (see [Attributes](#attributes)).
    #[must_use]
    pub fn implicit_import(&self) -> Option<ItemId> {
        self.implicit_import
    }

    /// Gives the file its implicit `#[shadowable] import * from prelude`,
    /// `prelude` being the name of the prelude's stem: an import item of the
    /// tree written nowhere, kept apart from the file's body (see
    /// [Attributes](#attributes)). Does nothing to a file marked
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

    /// Every expression's id, in allocation order, for tests that look for
    /// nodes of a kind.
    #[cfg(test)]
    pub(crate) fn expr_ids(&self) -> impl Iterator<Item = ExprId> {
        (0..next_id(self.exprs.len())).map(ExprId)
    }

    /// The ids of the nodes of expression `id`'s subtree, its descendants
    /// and itself, in allocation order. They are contiguous, ending at `id`:
    /// nodes are allocated children-first, and while the parser parses an
    /// expression it allocates only that expression's nodes (a path is read
    /// from its tokens, allocating only the positions of its index
    /// segments, which belong to its item).
    /// The first is found by following, from `id`, the child the parser
    /// parses first down to a leaf: a block's first item's value, or the
    /// position of the first index segment of its path if it has one, an
    /// `if`'s condition, a call's callee, a binary operator's left operand,
    /// a function's or `lazy`'s body, an interpolated string's first
    /// expression, a list literal's first element's, an index's list, an
    /// iteration's iterable, a condition loop's first header item's, its
    /// condition or its body, whichever comes first, and a `do` loop's
    /// body. (A block's first item may hold no expression, a `break`
    /// without a value: the first item that holds one is followed.)
    /// Debug builds check that every node of the range lies
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
    fn first_node(&self, id: ExprId) -> ExprId {
        let mut node = id;
        loop {
            node = match self.get(node).kind {
                ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Bool(_)
                | ExprKind::Str(_)
                | ExprKind::Name(_) => return node,
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
                ExprKind::If { cond, .. } => cond,
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
                | ExprKind::As { expr: inner, .. } => inner,
                ExprKind::Fn(function) | ExprKind::Lazy(function) => self.fn_decl(function).body,
                ExprKind::Call { callee, .. } => callee,
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
    fn first_of_items(&self, items: ItemRange) -> Option<ExprId> {
        self.items(items)
            .iter()
            .find_map(|&item| match self.item(item).kind {
                ItemKind::Rebind { path, value } => Some(
                    self.path(path)
                        .iter()
                        .find_map(PathSegment::position)
                        .unwrap_or(value),
                ),
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
/// Items print as `(let x ...)`, `(let x : Type ...)`, `(let mut x ...)`,
/// `(let mut x : Type ...)`, `(field x ...)`,
/// `(rebind x ...)`, `(spread ...)`, `(result ...)` and `(expr ...)`,
/// followed by their span and their expression, a dotted path in full, as
/// in `(let a.b.c ...)`, and an index segment as it is written, as in
/// `(rebind xs[i + 1].up ...)`, its position then shown before the value as
/// `(at EXPR)`, one for each index segment, in order; blocks as
/// `(block ...)` followed by their items; ascriptions as `(: Type ...)`;
/// field accesses as `(. x ...)` followed by their record expression. A
/// field name, or a path segment, that is not a plain identifier is quoted,
/// as in `(field "a b" ...)`. Types print as written, a record type's fields
/// in source order: `{x: Int, "a b": {}}`, `fn(x: Int) -> String`. A function
/// expression prints as `(fn ...)` followed by `(params (x Int) (y String))`,
/// `(ret pure Int)`, `(ret memo Int)` or `(ret impure Int)`, and its body; a
/// call as
/// `(call ...)` followed by its callee and its arguments in source order, a
/// named one as `(named x EXPR)` and the trailing block as
/// `(trailing BLOCK)`. A named function item is the `let` it is parsed as,
/// printed as written, `(fn f (fn ...))`, and a `def` the field,
/// `(def f (fn ...))`; `let f = fn ...` and `f = fn ...` print as `(let f
/// (fn ...))` and `(field f (fn ...))`. A `lazy` prints as `(lazy ...)` followed by its body,
/// and an `await` as `(await ...)` followed by its operand. A list literal
/// prints as `(list ...)` followed by its elements in source order, a
/// spread one as `(spread EXPR)`, and an index as `(index ...)` followed by
/// its list and its position. A `for` prints as `(for x ...)`, `x` being
/// its variable, followed by its iterable, its guard as `(guard EXPR)` if
/// it has one, and its body; a condition loop as `(for ...)` followed by
/// the items before its condition, its condition as `(cond EXPR)` if it
/// has one, the items after it and its body; a `do` loop as `(do ...)`
/// followed by its body and its condition as `(cond EXPR)`. A loop's label
/// follows its keyword as it is written, as in `(for'rows r ...)`, and a
/// block's follows `block`, as in `(block'pick @0..30 ...)`. A `break`,
/// `continue` or `return` item prints as `(break @0..5)`, its label after
/// its keyword, as in `(continue'rows @0..14)` or `(return'pick @2..16
/// ...)`, and its value, if it has one, after its span. A range prints as
/// its operator, `..` or `..=`,
/// like any binary operator. Applied and
/// union types print as written, `Future<Int>`, `Present<Int>` and
/// `Int | Future<Int>`, a function type that is a member of a union in
/// parentheses, `(fn(): Int) | Future<fn(): Int>`, since its return type
/// would otherwise take the rest of the union; a list type written `T[]`
/// prints so, its element type in parentheses if it is a function type or
/// a union, `(fn(): Int)[]`; other parentheses in a type
/// leave no trace in the tree and are not printed.
/// Booleans print as `(bool true)`, string literals as `(str "text")` with
/// the text quoted and escaped, and interpolated strings as `(interp ...)`
/// followed by their parts, literal text as `(lit "text")` (which has no
/// span) and expressions as their nodes. `(if ...)` is followed by its
/// condition, its block and its else branch, which for an `if` without
/// `else` is the parser's empty block. Operators print as their symbol,
/// except negation, `(neg ...)`. The file has no span. An item marked
/// `#[shadowable]` is preceded by the attribute, as in `#[shadowable] (field
/// arch ...)`, and a file marked `#![no_prelude]` has it after `file`, as in
/// `(file #![no_prelude] ...)`; the implicit import a workspace gives a
/// file is not printed (see [Attributes](crate::ast#attributes)). There is
/// no trailing newline.
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

    /// Writes an import item as it is written, without its span.
    fn import(&self, f: &mut fmt::Formatter<'_>, decl: &ImportDecl) -> fmt::Result {
        let ast = self.ast;
        let name = |name: ImportName| ast.names().get(name.name);
        f.write_str("import ")?;
        let path = ast.show_path(ast.path(decl.path));
        match decl.form {
            ImportForm::Whole(alias) => {
                write!(f, "{path}")?;
                if let Some(alias) = alias {
                    write!(f, " as {}", name(alias))?;
                }
            }
            ImportForm::Members(range) => {
                for (index, member) in ast.import_members(range).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    match *member {
                        ImportMember::Named {
                            name: imported,
                            alias,
                        } => {
                            f.write_str(name(imported))?;
                            if let Some(alias) = alias {
                                write!(f, " as {}", name(alias))?;
                            }
                        }
                        ImportMember::All(_) => f.write_str("*")?,
                    }
                }
                write!(f, " from {path}")?;
            }
        }
        Ok(())
    }

    fn item(&self, f: &mut fmt::Formatter<'_>, id: ItemId, depth: usize) -> fmt::Result {
        let item = self.ast.item(id);
        let ast = self.ast;
        let marked = if ast.is_shadowable(id) {
            "#[shadowable] "
        } else {
            ""
        };
        write!(f, "{:indent$}{marked}(", "", indent = depth * 2)?;
        let value = match item.kind {
            ItemKind::Let {
                path,
                ty,
                value,
                mutable,
                function,
            } => {
                let keyword = match (mutable, function) {
                    (_, true) => "fn",
                    (true, false) => "let mut",
                    (false, false) => "let",
                };
                write!(f, "{keyword} {}", ast.show_path(ast.path(path)))?;
                if let Some(ty) = ty {
                    f.write_str(" : ")?;
                    self.type_expr(f, ty)?;
                }
                value
            }
            ItemKind::Field {
                path,
                value,
                function,
            } => {
                let keyword = if function { "def" } else { "field" };
                write!(f, "{keyword} {}", ast.show_path(ast.path(path)))?;
                value
            }
            ItemKind::Rebind { path, value } => {
                write!(f, "rebind {}", ast.show_path(ast.path(path)))?;
                value
            }
            ItemKind::Spread(value) => {
                f.write_str("spread")?;
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
            ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
                let keyword = match item.kind {
                    ItemKind::Break(_) => "break",
                    ItemKind::Return(_) => "return",
                    _ => "continue",
                };
                return self.jump(f, (keyword, jump), item.span, depth);
            }
            ItemKind::Import(id) => {
                self.import(f, ast.import_decl(id))?;
                return write!(f, " @{})", item.span);
            }
        };
        writeln!(f, " @{}", item.span)?;
        // The positions of a `:=` path's index segments, in order, before
        // its value, as they are parsed.
        if let ItemKind::Rebind { path, .. } = item.kind {
            for position in ast.path(path).iter().filter_map(PathSegment::position) {
                writeln!(f, "{:indent$}(at", "", indent = (depth + 1) * 2)?;
                self.node(f, position, depth + 2)?;
                f.write_str(")\n")?;
            }
        }
        self.node(f, value, depth + 1)?;
        f.write_str(")")
    }

    /// Writes the rest of a `break` or `continue` item, `keyword`, whose
    /// label and value are `jump`, spanning `span`, at `depth`, after its
    /// `(`: the keyword and the label, as in `break'outer`, its span, and
    /// its value, if it has one, on a line of its own.
    fn jump(
        &self,
        f: &mut fmt::Formatter<'_>,
        (keyword, jump): (&str, Jump),
        span: Span,
        depth: usize,
    ) -> fmt::Result {
        f.write_str(keyword)?;
        self.label(f, jump.label)?;
        write!(f, " @{span}")?;
        if let Some(value) = jump.value {
            f.write_str("\n")?;
            self.node(f, value, depth + 1)?;
        }
        f.write_str(")")
    }

    /// Writes `label`, if there is one, as it is written: `'` and its name.
    fn label(&self, f: &mut fmt::Formatter<'_>, label: Option<Label>) -> fmt::Result {
        match label {
            Some(label) => write!(f, "'{}", self.ast.names().get(label.name)),
            None => Ok(()),
        }
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
            TypeExprKind::Fn {
                params,
                ret,
                effect,
            } => {
                f.write_str("fn(")?;
                for (index, param) in self.ast.param_tys(params).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    if let Some(name) = param.name {
                        write!(f, "{}: ", FieldName(names.get(name)))?;
                    }
                    self.type_expr(f, param.ty)?;
                }
                f.write_str(effect.params_end())?;
                self.type_expr(f, ret)
            }
            TypeExprKind::Applied { name, arg, .. } => {
                write!(f, "{}<", names.get(name))?;
                self.type_expr(f, arg)?;
                f.write_str(">")
            }
            TypeExprKind::Union { lhs, rhs, .. } => {
                self.union_member(f, lhs)?;
                f.write_str(" | ")?;
                self.union_member(f, rhs)
            }
            TypeExprKind::List(element) => {
                // The `[]` would belong to a function type's return type,
                // and bind tighter than a union's `|`.
                if matches!(
                    self.ast.type_expr(element).kind,
                    TypeExprKind::Fn { .. } | TypeExprKind::Union { .. }
                ) {
                    f.write_str("(")?;
                    self.type_expr(f, element)?;
                    f.write_str(")[]")
                } else {
                    self.type_expr(f, element)?;
                    f.write_str("[]")
                }
            }
        }
    }

    /// Writes the type expression `id`, a member of a union, in parentheses
    /// if it is a function type, whose return type would otherwise take
    /// what follows.
    fn union_member(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        if matches!(self.ast.type_expr(id).kind, TypeExprKind::Fn { .. }) {
            f.write_str("(")?;
            self.type_expr(f, id)?;
            f.write_str(")")
        } else {
            self.type_expr(f, id)
        }
    }

    /// Writes the parameters and return type of the function expression
    /// `id`, each on a line of its own at `depth`, with a leading newline.
    fn fn_head(&self, f: &mut fmt::Formatter<'_>, id: FnId, depth: usize) -> fmt::Result {
        let decl = self.ast.fn_decl(id);
        write!(f, "\n{:indent$}(params", "", indent = depth * 2)?;
        for param in self.ast.params(decl.params) {
            write!(f, " ({} ", FieldName(self.ast.names().get(param.name)))?;
            self.type_expr(f, param.ty)?;
            f.write_str(")")?;
        }
        f.write_str(")")?;
        let effect = decl.effect.name();
        write!(f, "\n{:indent$}(ret {effect}", "", indent = depth * 2)?;
        // A function expression always has a return type; only a `lazy`,
        // which is not printed through here, has none.
        if let Some(ret) = decl.ret {
            f.write_str(" ")?;
            self.type_expr(f, ret)?;
        }
        f.write_str(")")
    }

    /// Writes the arguments `range` of a call, each on a line of its own at
    /// `depth`, with a leading newline.
    fn args(&self, f: &mut fmt::Formatter<'_>, range: ArgRange, depth: usize) -> fmt::Result {
        for arg in self.ast.args(range) {
            f.write_str("\n")?;
            match arg.kind {
                ArgKind::Positional => self.node(f, arg.value, depth)?,
                ArgKind::Named { name, .. } => {
                    let name = FieldName(self.ast.names().get(name));
                    writeln!(f, "{:indent$}(named {name}", "", indent = depth * 2)?;
                    self.node(f, arg.value, depth + 1)?;
                    f.write_str(")")?;
                }
                ArgKind::Trailing => {
                    writeln!(f, "{:indent$}(trailing", "", indent = depth * 2)?;
                    self.node(f, arg.value, depth + 1)?;
                    f.write_str(")")?;
                }
            }
        }
        Ok(())
    }

    /// Writes the elements `range` of a list literal, each on a line of its
    /// own at `depth`, with a leading newline.
    fn elements(
        &self,
        f: &mut fmt::Formatter<'_>,
        range: ElementRange,
        depth: usize,
    ) -> fmt::Result {
        for element in self.ast.elements(range) {
            f.write_str("\n")?;
            if element.spread.is_some() {
                writeln!(f, "{:indent$}(spread", "", indent = depth * 2)?;
                self.node(f, element.value, depth + 1)?;
                f.write_str(")")?;
            } else {
                self.node(f, element.value, depth)?;
            }
        }
        Ok(())
    }

    /// Writes the head of the loop `id`: its keyword, `for` or `do`, its
    /// label, if it has one, and, for an iteration, its variable's name, as
    /// in `for x`, `for'rows r`, `for` and `do'again`.
    fn for_head(&self, f: &mut fmt::Formatter<'_>, id: ForId) -> fmt::Result {
        let decl = self.ast.for_decl(id);
        f.write_str(match decl.head {
            ForHead::Do { .. } => "do",
            ForHead::Each { .. } | ForHead::While { .. } => "for",
        })?;
        self.label(f, decl.label)?;
        match decl.head {
            ForHead::Each { name, .. } => write!(f, " {}", self.ast.names().get(name)),
            ForHead::While { .. } | ForHead::Do { .. } => Ok(()),
        }
    }

    /// Writes the parts of the loop `id`, each on a line of its own at
    /// `depth`, with a leading newline: for an iteration, its iterable, its
    /// guard, as `(guard EXPR)`, if there is one, and its body; for a
    /// condition loop, the items before its condition, its condition, as
    /// `(cond EXPR)`, if it has one, the items after it and its body; and
    /// for a `do` loop, its body and its condition.
    fn for_parts(&self, f: &mut fmt::Formatter<'_>, id: ForId, depth: usize) -> fmt::Result {
        let decl = self.ast.for_decl(id);
        let (first, guard, cond, last) = match decl.head {
            ForHead::Each {
                iterable, guard, ..
            } => (Some(iterable), guard, None, Some(decl.body)),
            ForHead::While {
                before,
                cond,
                after,
            } => {
                for &item in self.ast.items(before) {
                    f.write_str("\n")?;
                    self.item(f, item, depth)?;
                }
                if let Some(cond) = cond {
                    self.wrapped(f, ("cond", cond), depth)?;
                }
                for &item in self.ast.items(after) {
                    f.write_str("\n")?;
                    self.item(f, item, depth)?;
                }
                (Some(decl.body), None, None, None)
            }
            ForHead::Do { cond } => (Some(decl.body), None, Some(cond), None),
        };
        if let Some(first) = first {
            f.write_str("\n")?;
            self.node(f, first, depth)?;
        }
        if let Some(guard) = guard {
            self.wrapped(f, ("guard", guard), depth)?;
        }
        if let Some(cond) = cond {
            self.wrapped(f, ("cond", cond), depth)?;
        }
        if let Some(last) = last {
            f.write_str("\n")?;
            self.node(f, last, depth)?;
        }
        Ok(())
    }

    /// Writes `expr` wrapped as `(WORD EXPR)`, at `depth`, with a leading
    /// newline: a loop's guard or condition.
    fn wrapped(
        &self,
        f: &mut fmt::Formatter<'_>,
        (word, expr): (&str, ExprId),
        depth: usize,
    ) -> fmt::Result {
        writeln!(f, "\n{:indent$}({word}", "", indent = depth * 2)?;
        self.node(f, expr, depth + 1)?;
        f.write_str(")")
    }

    /// Writes the head of the node `expr`: what it is, and for a literal,
    /// a name, a field access, an operator, an ascription, an `is` and an
    /// `as`, what it holds.
    fn head(&self, f: &mut fmt::Formatter<'_>, expr: &Expr) -> fmt::Result {
        match expr.kind {
            ExprKind::Int(value) => write!(f, "int {value}"),
            ExprKind::Float(value) => write!(f, "float {value:?}"),
            ExprKind::Bool(value) => write!(f, "bool {value}"),
            ExprKind::Str(id) => write!(f, "str {}", Quoted(self.ast.strings().get(id))),
            ExprKind::Interp(_) => f.write_str("interp"),
            ExprKind::If { .. } => f.write_str("if"),
            ExprKind::Unary { op, .. } => match op {
                UnaryOp::Neg => f.write_str("neg"),
                UnaryOp::Not => f.write_str("not"),
                UnaryOp::Await => f.write_str("await"),
            },
            ExprKind::Binary { op, .. } => f.write_str(op.symbol()),
            ExprKind::Paren(_) => f.write_str("paren"),
            ExprKind::Name(name) => write!(f, "name {}", self.ast.names().get(name)),
            ExprKind::Block { label, .. } => {
                f.write_str("block")?;
                self.label(f, label.map(|label| self.ast.label(label)))
            }
            ExprKind::Field { name, .. } => {
                write!(f, ". {}", FieldName(self.ast.names().get(name)))
            }
            ExprKind::Ascribe { ty, .. } => {
                f.write_str(": ")?;
                self.type_expr(f, ty)
            }
            ExprKind::Is { ty, .. } => {
                f.write_str("is ")?;
                self.type_expr(f, ty)
            }
            ExprKind::As { ty, .. } => {
                f.write_str("as ")?;
                self.type_expr(f, ty)
            }
            ExprKind::Fn(_) => f.write_str("fn"),
            ExprKind::Call { .. } => f.write_str("call"),
            ExprKind::Lazy(_) => f.write_str("lazy"),
            ExprKind::List(_) => f.write_str("list"),
            ExprKind::Index { .. } => f.write_str("index"),
            ExprKind::For(id) => self.for_head(f, id),
        }
    }

    fn node(&self, f: &mut fmt::Formatter<'_>, id: ExprId, depth: usize) -> fmt::Result {
        let expr = self.ast.get(id);
        write!(f, "{:indent$}(", "", indent = depth * 2)?;
        self.head(f, expr)?;
        self.annotation.annotate(id, f)?;
        write!(f, " @{}", expr.span)?;
        match expr.kind {
            ExprKind::Fn(function) => {
                self.fn_head(f, function, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, self.ast.fn_decl(function).body, depth + 1)?;
            }
            ExprKind::Lazy(function) => {
                f.write_str("\n")?;
                self.node(f, self.ast.fn_decl(function).body, depth + 1)?;
            }
            ExprKind::Call { callee, args } => {
                f.write_str("\n")?;
                self.node(f, callee, depth + 1)?;
                self.args(f, args, depth + 1)?;
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Name(_) => {}
            ExprKind::Unary { operand, .. }
            | ExprKind::Paren(operand)
            | ExprKind::Field { expr: operand, .. }
            | ExprKind::Ascribe { expr: operand, .. }
            | ExprKind::Is { expr: operand, .. }
            | ExprKind::As { expr: operand, .. } => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Index {
                expr: lhs,
                index: rhs,
            } => {
                f.write_str("\n")?;
                self.node(f, lhs, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, rhs, depth + 1)?;
            }
            ExprKind::List(elements) => self.elements(f, elements, depth + 1)?,
            ExprKind::For(id) => self.for_parts(f, id, depth + 1)?,
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
            ExprKind::Block { items, .. } => self.items(f, items, depth + 1)?,
        }
        f.write_str(")")
    }
}

impl<A: Annotate> fmt::Display for Pretty<'_, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(file")?;
        if self.ast.no_prelude.is_some() {
            f.write_str(" #![no_prelude]")?;
        }
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

    /// An unlabelled block of `items`.
    const fn unlabelled(items: ItemRange) -> ExprKind {
        ExprKind::Block { items, label: None }
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
    fn an_item_is_small() {
        // What tells a named function item and a `def` from a `let` or a
        // field whose value is a function expression fits in the padding.
        assert_eq!(size_of::<Item>(), 36);
    }

    #[test]
    fn a_blocks_label_fits_beside_its_items() {
        // The label's id has a niche, so that a block's optional label
        // costs four bytes, and an unlabelled block nothing beyond them.
        assert_eq!(size_of::<Option<LabelId>>(), 4);
        assert_eq!(size_of::<ExprKind>(), 16);
        let mut builder = AstBuilder::new();
        let a = Label {
            name: Names::new().intern("a"),
            span: Span::new(1, 3),
        };
        let first = builder.push_label(a);
        let second = builder.push_label(Label {
            span: Span::new(5, 7),
            ..a
        });
        assert_ne!(first, second);
        let empty = builder.push_item_list(&[]);
        let block = builder.alloc(Expr {
            kind: ExprKind::Block {
                items: empty,
                label: Some(second),
            },
            span: Span::new(4, 9),
        });
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(block),
            span: Span::new(4, 9),
        });
        let body = builder.push_item_list(&[item]);
        let ast = builder.finish(body, Names::new(), Strings::new());
        assert_eq!(ast.label(first).span, Span::new(1, 3));
        assert_eq!(
            ast.block_label(block).map(|label| label.span),
            Some(Span::new(5, 7))
        );
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
        let y_path = builder.push_path(&[PathSegment {
            kind: SegmentKind::Name(y),
            span: Span::new(18, 19),
        }]);
        let rebind = builder.alloc_item(Item {
            kind: ItemKind::Rebind {
                path: y_path,
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
            kind: unlabelled(body),
            span: Span::new(16, 37),
        });
        let float_ty = builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Named(float),
            span: Span::new(7, 12),
        });
        let x_path = builder.push_path(&[PathSegment {
            kind: SegmentKind::Name(x),
            span: Span::new(4, 5),
        }]);
        let with_type = builder.alloc_item(Item {
            kind: ItemKind::Let {
                path: x_path,
                ty: Some(float_ty),
                value: block,
                mutable: false,
                function: false,
            },
            span: Span::new(0, 37),
        });
        let empty_body = builder.push_item_list(&[]);
        let empty = builder.alloc(Expr {
            kind: unlabelled(empty_body),
            span: Span::new(46, 48),
        });
        let y_path = builder.push_path(&[PathSegment {
            kind: SegmentKind::Name(y),
            span: Span::new(42, 43),
        }]);
        let without_type = builder.alloc_item(Item {
            kind: ItemKind::Let {
                path: y_path,
                ty: None,
                value: empty,
                mutable: false,
                function: false,
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
        for (op, symbol, equality) in [
            (BinaryOp::Eq, "==", true),
            (BinaryOp::Ne, "!=", true),
            (BinaryOp::Same, "===", true),
            (BinaryOp::NotSame, "!==", true),
            (BinaryOp::Lt, "<", false),
            (BinaryOp::Le, "<=", false),
            (BinaryOp::Gt, ">", false),
            (BinaryOp::Ge, ">=", false),
        ] {
            assert_eq!(op.symbol(), symbol);
            assert_eq!(op.verb(), "compare");
            assert!(op.is_comparison());
            assert_eq!(op.is_equality(), equality, "{op:?}");
        }
        assert_eq!(BinaryOp::Same.span_at(4), Span::new(4, 7));
        assert_eq!(BinaryOp::And.verb(), "and");
        assert_eq!(BinaryOp::Or.verb(), "or");
        assert_eq!(UnaryOp::Not.verb(), "not");
        assert_eq!(BinaryOp::And.symbol(), "and");
        assert_eq!(BinaryOp::Or.symbol(), "or");
        assert_eq!(UnaryOp::Not.symbol(), "not");
        assert_eq!(BinaryOp::Or.span_at(4), Span::new(4, 6));
        assert_eq!(UnaryOp::Not.span_at(4), Span::new(4, 7));
        assert_eq!(UnaryOp::Await.verb(), "await");
        assert_eq!(UnaryOp::Await.symbol(), "await");
        assert_eq!(UnaryOp::Await.span_at(4), Span::new(4, 9));
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
            assert!(!op.is_equality(), "{op:?}");
        }
    }

    #[test]
    fn paths_are_ranges_of_the_pool_and_show_as_written() {
        // let a.b."c d".let = 1, spans made up.
        let mut names = Names::new();
        let (a, b, spaced, keyword) = (
            names.intern("a"),
            names.intern("b"),
            names.intern("c d"),
            names.intern("let"),
        );
        let mut builder = AstBuilder::new();
        let segment = |name, start, end| PathSegment {
            kind: SegmentKind::Name(name),
            span: Span::new(start, end),
        };
        let segments = [
            segment(a, 4, 5),
            segment(b, 6, 7),
            segment(spaced, 8, 13),
            segment(keyword, 14, 19),
        ];
        let single = builder.push_path(&segments[..1]);
        let dotted = builder.push_path(&segments);
        let one = leaf(&mut builder, 1, 22);
        let items = [
            ItemKind::Let {
                path: dotted,
                ty: None,
                value: one,
                mutable: false,
                function: false,
            },
            ItemKind::Field {
                path: dotted,
                value: one,
                function: false,
            },
            ItemKind::Rebind {
                path: single,
                value: one,
            },
        ]
        .map(|kind| {
            builder.alloc_item(Item {
                kind,
                span: Span::new(0, 23),
            })
        });
        let root = builder.push_item_list(&items);
        let ast = builder.finish(root, names, Strings::new());
        assert_eq!(ast.path(single), &segments[..1]);
        assert_eq!(ast.path(dotted), &segments);
        assert_eq!(
            ast.show_path(ast.path(dotted)).to_string(),
            "a.b.\"c d\".\"let\""
        );
        assert_eq!(ast.show_path(&segments[..2]).to_string(), "a.b");
        assert_eq!(path_span(ast.path(dotted)), Span::new(4, 19));
        assert_eq!(path_span(&segments[1..3]), Span::new(6, 13));
        assert_eq!(path_span(&[]), Span::new(0, 0));
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n\
             \x20 (let a.b.\"c d\".\"let\" @0..23\n\
             \x20   (int 1 @22..23))\n\
             \x20 (field a.b.\"c d\".\"let\" @0..23\n\
             \x20   (int 1 @22..23))\n\
             \x20 (rebind a @0..23\n\
             \x20   (int 1 @22..23)))"
        );
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
            kind: unlabelled(then_body),
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
            kind: unlabelled(else_body),
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

    #[test]
    fn lists_and_indexes_are_subtrees_of_the_pool() {
        // `[1, ...xs][0]`, built children first as the parser does.
        let mut names = Names::new();
        let xs = names.intern("xs");
        let mut builder = AstBuilder::new();
        let one = leaf(&mut builder, 1, 1);
        let xs_use = builder.alloc(Expr {
            kind: ExprKind::Name(xs),
            span: Span::new(7, 9),
        });
        let elements = builder.push_elements(&[
            Element {
                value: one,
                spread: None,
            },
            Element {
                value: xs_use,
                spread: Some(Span::new(4, 7)),
            },
        ]);
        let list = builder.alloc(Expr {
            kind: ExprKind::List(elements),
            span: Span::new(0, 10),
        });
        let zero = leaf(&mut builder, 0, 11);
        let index = builder.alloc(Expr {
            kind: ExprKind::Index {
                expr: list,
                index: zero,
            },
            span: Span::new(0, 13),
        });
        let empty_elements = builder.push_elements(&[]);
        let empty = builder.alloc(Expr {
            kind: ExprKind::List(empty_elements),
            span: Span::new(14, 16),
        });
        let items: Vec<ItemId> = [index, empty]
            .into_iter()
            .map(|value| {
                builder.alloc_item(Item {
                    kind: ItemKind::Expr(value),
                    span: builder.get(value).span,
                })
            })
            .collect();
        let root = builder.push_item_list(&items);
        let ast = builder.finish(root, names, Strings::new());
        assert_eq!(ast.elements(elements).len(), 2);
        assert!(ast.elements(empty_elements).is_empty());
        assert_eq!(ast.elements(elements)[1].span(&ast), Span::new(4, 9));
        // A literal's subtree starts at its first element's, an index's at
        // its list's; an empty literal's is itself.
        assert_eq!(
            ast.subtree(index).collect::<Vec<_>>(),
            vec![one, xs_use, list, zero, index]
        );
        assert_eq!(
            ast.subtree(list).collect::<Vec<_>>(),
            vec![one, xs_use, list]
        );
        assert_eq!(ast.subtree(empty).collect::<Vec<_>>(), vec![empty]);
        assert_eq!(ast.get(index).operator_span(), None);
        assert_eq!(ast.get(index).bracket_span(&ast), Some(Span::new(10, 13)));
        assert_eq!(ast.get(list).bracket_span(&ast), None);
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n\
             \x20 (expr @0..13\n\
             \x20   (index @0..13\n\
             \x20     (list @0..10\n\
             \x20       (int 1 @1..2)\n\
             \x20       (spread\n\
             \x20         (name xs @7..9)))\n\
             \x20     (int 0 @11..12)))\n\
             \x20 (expr @14..16\n\
             \x20   (list @14..16)))"
        );
    }

    #[test]
    fn ranges_are_binary_operators() {
        for (op, symbol) in [(BinaryOp::Range, ".."), (BinaryOp::RangeInclusive, "..=")] {
            assert_eq!(op.symbol(), symbol);
            assert!(op.is_range());
            assert!(!op.is_comparison());
            assert!(!op.is_equality());
        }
        assert_eq!(BinaryOp::Range.span_at(4), Span::new(4, 6));
        assert_eq!(BinaryOp::RangeInclusive.span_at(4), Span::new(4, 7));
        assert!(!BinaryOp::Add.is_range());
        assert!(!BinaryOp::Lt.is_range());
    }

    #[test]
    fn a_for_is_a_subtree_of_the_pool() {
        // `for x in 0..3 if g { x }`, built children first as the parser
        // does.
        let mut names = Names::new();
        let (x, g) = (names.intern("x"), names.intern("g"));
        let mut builder = AstBuilder::new();
        let zero = leaf(&mut builder, 0, 9);
        let three = leaf(&mut builder, 3, 12);
        let range = builder.alloc(Expr {
            kind: ExprKind::Binary {
                op: BinaryOp::Range,
                op_start: 10,
                lhs: zero,
                rhs: three,
            },
            span: Span::new(9, 13),
        });
        let guard = builder.alloc(Expr {
            kind: ExprKind::Name(g),
            span: Span::new(17, 18),
        });
        let x_use = builder.alloc(Expr {
            kind: ExprKind::Name(x),
            span: Span::new(21, 22),
        });
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(x_use),
            span: Span::new(21, 22),
        });
        let items = builder.push_item_list(&[item]);
        let body = builder.alloc(Expr {
            kind: unlabelled(items),
            span: Span::new(19, 24),
        });
        let id = builder.push_for(ForDecl {
            label: None,
            head: ForHead::Each {
                name: x,
                name_span: Span::new(4, 5),
                iterable: range,
                guard: Some(guard),
            },
            body,
        });
        let for_expr = builder.alloc(Expr {
            kind: ExprKind::For(id),
            span: Span::new(0, 24),
        });
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(for_expr),
            span: Span::new(0, 24),
        });
        let root = builder.push_item_list(&[item]);
        let ast = builder.finish(root, names, Strings::new());
        assert_eq!(ast.for_count(), 1);
        assert_eq!(id.index(), 0);
        assert_eq!(ast.for_decl(id).body, body);
        // A `for`'s subtree starts at its iterable's.
        assert_eq!(
            ast.subtree(for_expr).collect::<Vec<_>>(),
            vec![zero, three, range, guard, x_use, body, for_expr]
        );
        assert_eq!(ast.get(for_expr).operator_span(), None);
        assert_eq!(ast.get(for_expr).keyword_span(), Some(Span::new(0, 3)));
        assert_eq!(ast.get(range).operator_span(), Some(Span::new(10, 12)));
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n\
             \x20 (expr @0..24\n\
             \x20   (for x @0..24\n\
             \x20     (.. @9..13\n\
             \x20       (int 0 @9..10)\n\
             \x20       (int 3 @12..13))\n\
             \x20     (guard\n\
             \x20       (name g @17..18))\n\
             \x20     (block @19..24\n\
             \x20       (expr @21..22\n\
             \x20         (name x @21..22))))))"
        );
    }
}
