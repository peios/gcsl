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
//! item, `def f(...) ...`, is a field whose value is one, its path that of
//! the field, of one name or of several (`def dev.peios.hello(...) ...`, a
//! dotted field, every segment an identifier); each is marked as
//! written (`function` on [`ItemKind::Let`] and [`ItemKind::Field`]), so the
//! tree tells `fn f`, `def f`, `let f = fn` and `f = fn` apart, and the
//! printer prints each as it was written, a `def` by its path. A call, [`ExprKind::Call`], lists
//! its arguments in
//! source order, each positional, named or the trailing block, which is
//! always last.
//!
//! A parameter of a function expression may have a default, `greeting:
//! String = "hello"`: an expression of the function's tree
//! ([`Param::default`]), parsed after the parameter's type and so before
//! the function's body, which a call that knows the function gives the
//! parameter when it leaves it unfilled (see [`crate::check`], "Calls").
//! One that is no constant ([`is_constant`]) has a default function too
//! ([`Param::default_fn`]), a function of the parameters before it whose
//! body is the default, which the parser makes, and which a call calls on
//! its side for the parameter.
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
//! # Types
//!
//! A `type` item, `type NAME = TYPE` ([`ItemKind::Type`]), declares a type,
//! an alias of `TYPE`, in the file's namespace of types, apart from its
//! variables and fields. Its name is read only in a type position, written
//! bare, `PackageDef`, or qualified through an import, `t.PackageDef`, a
//! [`TypeExprKind::Path`] in the path pool. A record type may spread
//! another, `{...PackageDef, name: String}`, and may have a rest-field, the
//! type of every field it does not name, `{to: String, *: Int}`: each an
//! entry of its fields with no name ([`FieldTy`], [`FieldTyKind`]). A
//! spread's rest-field merges as its fields do. A field item may be
//! annotated, `x: Int = 1`, as a
//! `let` may: the annotation is in its [`ItemKind::Field`], and an [`Item`]
//! is still 36 bytes.
//!
//! The type expressions of a tree are a [`TypeArena`]. Before a file is
//! checked, a workspace *expands* its types ([`Ast::expand_types`]): each
//! name of a declared type becomes what the type's definition is, as if the
//! definition were written there, its nodes copied into the tree's arena,
//! after the parsed ones, out of a [`TypeStore`], which holds every
//! declaration's definition apart from its file's tree, each *settled*: its
//! names of other declarations made what they define, its spreads merged.
//! And each record type with spreads becomes the record type of its merged
//! fields. A spread's fields are SOFT, as value spread's are: a later field
//! of one's name replaces it, and so does a later spread's; a spread's field
//! of the name of a field written before the spread is a problem, and so is
//! a spread of a type that is not a record type. What the expansion finds
//! wrong it keeps as the tree's problems ([`TypeProblem`],
//! [`Ast::type_problems`]), by node, for the checker to report where it
//! resolves the node; and what each node it changed was written as, for the
//! printer ([`Ast::written_type`]). So the checker sees the expanded types,
//! and the printer the written ones. The checker knows the copies apart
//! ([`Ast::is_definition_copy`]), since what is wrong in one is its own
//! file's to report. A copy is made once per tree and declaration, and a
//! definition used twice shares its nodes, so a tree's type expressions may
//! share children; the parser's are still allocated children first.
//!
//! A `type` item may be generic, `type Pair<A, B> = {first: A, second: B}`,
//! its type parameters recorded as a function item's are ([`Generics`],
//! [`GenericOwner::Type`]), and is then used applied to its type arguments,
//! `Pair<Int, String>` ([`TypeExprKind::Applied`], or
//! [`TypeExprKind::PathApplied`] qualified). In the store, its definition
//! holds each of its type parameters as a [`TypeExprKind::Param`], and a
//! use of it in another definition is an [`TypeExprKind::Instance`] of it,
//! its arguments; settling makes an instance the definition with each
//! parameter the argument of its number (`TypeArena::substitute`), and
//! expanding a tree does the same at each use. Substituting makes new nodes
//! only above a parameter, sharing the rest and each argument, so an
//! instance costs no more nodes than its definition has, however deeply
//! its arguments nest. A generic type is its definition, as any declared
//! type is: types are structural, and `Pair<Int, String>` is
//! `{first: Int, second: String}`. A spread of a type parameter is the
//! problem a spread of anything but a record type is, the definition being
//! merged before any argument is known.
//!
//! A field of a record type may have a default, `{exclude: List<String> =
//! [], from: String}` (see the checker's "Field defaults"): its
//! [`FieldTy::default`], a [`FieldDefault`]. As the parser writes it, it is
//! an expression of the tree, any expression, parsed after the field's type
//! and so before whatever follows the type (a `let`'s value, a function's
//! body), and its nodes are part of the subtree of the expression or item
//! the type is written in (`Ast::subtree`). A copy of a definition cannot
//! hold an expression of another tree, so a default copied out of a tree
//! into a [`TypeStore`] is made the [`Constant`] it is written as, if it is
//! one, its integers left as integers and nothing filled
//! (`check::written_constant`), held in the arena's pool of constants,
//! and copied on, pool and all, into each tree using its declaration; the
//! checker fits it to its field's type there.
//!
//! # Def-types
//!
//! A `def` marked `#[def_type]` is a *def-type*, and other `def`s are made
//! with it: `def package dev.peios.testpkg() { ... }`, a definition,
//! [`ItemKind::DefTypeDef`], and `package dev.peios.testpkg`, a reference
//! to one, [`ExprKind::DefTypeRef`] (see the checker's "Def-types"). What
//! does not fit in their nodes is in an arena of its own, [`Ast::def_type`],
//! a [`DefTypeUse`] for each: the def-type, `DT`, written as a name or a
//! dotted path and kept as the expression that reads it, a name with a
//! field access for each further segment, outside the item's or the
//! reference's own subtree but before it in the arena; the definition's
//! path, `PATH`, in the path pool; and, once the workspace has found what
//! `DT` names, the key of its namespace (see [`crate::names`], "Namespace
//! keys"), under which the definitions made with it are leaves of the tree
//! of top-level fields. A definition's function expression is a
//! [`FnDecl`] like any other, but that its parameters are written by name
//! alone, each with a type expression of [`TypeExprKind::Invalid`] that
//! nothing reads, and it has no return type and a marker that means
//! nothing: all three are the def-type's. A reference also holds the read
//! of the definition, a name with a field access for each segment of
//! `PATH`, whose name stands for nothing until the workspace makes it the
//! namespace's key ([`Ast::set_def_type_key`]): it is then an ordinary
//! reference to the definition's cell.
//!
//! A function type may have a *meta*, written after its return type with
//! a marker of its own, `fn(v: Version): PackageDef META-> PackageMeta`: a
//! [`MetaTy`] in its [`TypeExprKind::Fn`]. A definition's meta is declared
//! by `defmeta DT PATH { ... }` or `defmeta DT PATH = EXPR`,
//! [`ItemKind::DefMeta`], whose [`DefTypeUse`] has its def-type and path as
//! a definition's does, and whose value is a function expression of no
//! parameters, written nowhere, whose body is the block, or a block whose
//! result is the expression; like a definition's, it has no return type
//! and a marker that means nothing, both being the meta's its def-type
//! declares. A definition and a meta are each, once the workspace has found
//! their def-type, also given the leaf of the meta in the namespace of the
//! metas of the def-type's definitions in their file's directory (see
//! [`crate::names`], "Namespace keys"), and a definition holds the read of
//! that leaf, a name with a field access, which its workspace makes a
//! reference to the meta's cell as it makes a reference's read one to the
//! definition's.
//!
//! A reference's `PATH` is read whole, however long: what part of it names
//! the definition is known only once the file's view is, so `package
//! dev.peios.testpkg.versions` is one reference, its read one field access
//! longer than the definition's path, whose last access the checker finds
//! reads the definition's meta (see the checker's "Metas"). The tree keeps
//! it as parsed; only a printer told so ([`Annotate::reference_past`]), as
//! `--dump-types` is, shows it as the reference to the definition with a
//! field access around it for each segment past it.
//!
//! # Attributes
//!
//! An item of a file's outermost block may be marked `#[shadowable]` or
//! `#[def_type]` (see the parser's [Attributes](crate::parser#attributes)),
//! and the file itself `#![no_prelude]`. None fits in an [`Item`], which
//! stays 36 bytes: the marked items are kept in side tables of the tree
//! ([`Ast::is_shadowable`], [`Ast::is_def_type`]), and the file's attribute
//! as the span it is written at ([`Ast::no_prelude`]). A workspace gives every file whose
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

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::num::NonZeroU32;
use std::rc::Rc;

use crate::MAX_DEPTH;
use crate::check::{Constant, written_constant};
use crate::diagnostic::Diagnostic;
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::{FieldName, Quoted, StrId, Strings};
use crate::types::write_constant;

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
    /// No item yet: what the parser's use of an attribute holds until the
    /// item after it is parsed, which then gives it its own.
    pub(crate) const PENDING: ItemId = ItemId(u32::MAX);

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

    /// The function numbered `number`, at or past [`Ast::fn_count`], that
    /// lowering makes for code no function expression of the tree is: the
    /// function a def-type reference evaluates to (see
    /// [Def-types](#def-types)).
    pub(crate) const fn lowered(number: u32) -> Self {
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

/// Index of a def-type definition's or reference's [`DefTypeUse`] in the
/// def-type arena of the [`Ast`] (or [`AstBuilder`]) that allocated it (see
/// [Def-types](#def-types)). Like [`ExprId`], only meaningful for that arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DefTypeId(u32);

impl DefTypeId {
    /// The id as an index into the arena and into side tables.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Which use of a def-type a [`DefTypeUse`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefTypeRole {
    /// A definition, `def DT PATH(PARAMS) BODY`.
    Definition,
    /// A reference, `DT PATH`.
    Reference,
    /// The meta of a definition, `defmeta DT PATH { ... }`.
    Meta,
}

/// A use of a def-type, a definition made with it, `def DT PATH(PARAMS)
/// BODY` ([`ItemKind::DefTypeDef`]), a reference to one, `DT PATH`
/// ([`ExprKind::DefTypeRef`]), or a definition's meta, `defmeta DT PATH {
/// ... }` ([`ItemKind::DefMeta`]): what does not fit in their nodes, in the
/// tree's def-type arena (see [Def-types](#def-types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DefTypeUse {
    /// Which use it is.
    pub role: DefTypeRole,
    /// The def-type, `DT`, as it is written: a name, or a chain of field
    /// accesses directly on one, an expression of the tree read as any value
    /// is, which is not among the expressions of the item or the reference
    /// it belongs to but before them in the arena.
    pub dt: ExprId,
    /// The variant written after DT, `::C` in `def Toolchain::C gcc()`, by
    /// its name and the name's span, for a def-type whose definitions are
    /// made with a variant (see the checker's "Def-types"); `None` if none
    /// is written.
    pub variant: Option<(Name, Span)>,
    /// The definition's path, `PATH`, as it is written: identifiers, or
    /// names with `+` in them, `gmp-c++`, or, after the first, starting with
    /// a digit, `24` or `24rc1`, each one name (see the parser's
    /// "Def-types"), joined by `.`.
    pub path: PathRange,
    /// For a reference, the read of the definition: a name standing for
    /// DT's namespace with a field access for each segment of PATH, which
    /// the workspace, once it finds DT to be a def-type, makes a reference
    /// to the definition's cell by naming the namespace's key
    /// ([`Ast::set_def_type_key`]); `None` for a definition.
    pub read: Option<ExprId>,
    /// The key of DT's namespace (see [`crate::names`], "Namespace keys"),
    /// once the workspace has found DT to be a def-type; `None` until then,
    /// and for good when it is not one.
    pub key: Option<Name>,
    /// For a definition and a meta, the leaf of PATH in the namespace of
    /// the metas of DT's definitions in the directory of the tree's file,
    /// once the workspace has found DT to be a def-type: that namespace's
    /// key and PATH as one name (see [`crate::names`], "Namespace keys");
    /// `None` until then, and for a reference.
    pub meta: Option<(Name, Name)>,
    /// For a definition, the read of its meta: a name standing for the
    /// namespace of the metas with a field access for PATH, as a reference's
    /// read of its definition, which the workspace makes the read of the
    /// meta's cell as it gives the definition its key
    /// ([`Ast::set_def_type_key`]), outside the item's subtree but before it
    /// in the arena; `None` for a reference and a meta.
    pub meta_read: Option<ExprId>,
    /// For a definition, its parameter list, `(` through `)`; for a
    /// reference, the whole of it, DT through PATH; for a meta, its
    /// `defmeta` through its PATH.
    pub span: Span,
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

/// The bindings of a `with`: a contiguous range of the pool of `with`
/// bindings of the [`Ast`] that allocated it. Resolve it with
/// [`Ast::with_bindings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WithRange {
    start: u32,
    len: u32,
}

/// One binding of a `with`, `NAME = VALUE`, `NAME.PATH = VALUE` or the
/// shorthand `NAME` (see the parser's [Context](crate::parser#context)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WithBinding {
    /// The context's name.
    pub name: Name,
    /// Where it is written.
    pub name_span: Span,
    /// The rest of a dotted key's path, the fields after the name, set in
    /// the context's value; empty for a plain one.
    pub path: PathRange,
    /// The value, the variable of the name for the shorthand.
    pub value: ExprId,
    /// For a dotted key, the read of the context's value the key updates,
    /// `^NAME`, written nowhere; `None` for a plain one.
    pub current: Option<ExprId>,
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

/// The type arguments of an applied type, `<Int, String>`: a contiguous
/// range of the type-argument pool of the arena that allocated it. Resolve
/// it with [`Ast::type_args`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeArgRange {
    start: u32,
    len: u32,
}

impl TypeArgRange {
    /// The number of type arguments.
    #[must_use]
    pub fn len(self) -> u32 {
        self.len
    }

    /// Whether there are none, which no applied type the parser makes has.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
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

/// A use of an attribute that is none of the parser's built-in ones, `#[NAME]`
/// or `#[NAME(ARGS)]`, on the item it applies to: a declared attribute,
/// whose name the workspace resolves as a type's, or one of the checker's
/// (see the parser's [Attributes](crate::parser#attributes)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributeUse {
    /// The item it applies to.
    pub item: ItemId,
    /// Its name.
    pub name: Name,
    /// Where it is written, from its `#` through its `]`.
    pub span: Span,
    /// Where its name is written.
    pub name_span: Span,
    /// Its arguments, if it is written with parentheses: as a call's are,
    /// positional or named.
    pub args: Option<ArgRange>,
    /// Its name as a type, at its name: an attribute is a type marked
    /// `#[attribute]`, which the workspace resolves and expands as it does
    /// any type's name (see [Types](#types)).
    pub ty: TypeExprId,
}

/// A method, `fn (RECV: TYPE) NAME(...) ...` or `def (RECV: TYPE) NAME(...)
/// ...` (see the parser's [Methods](crate::parser#methods)): its item is a
/// `let` of its binding, a name no source can write
/// ([`crate::names::method_binding`]), whose value is its function, the
/// receiver its first parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodDecl {
    /// Its item.
    pub item: ItemId,
    /// The name it is called by.
    pub name: Name,
    /// Where the name is written.
    pub name_span: Span,
    /// Its binding's name.
    pub binding: Name,
    /// Whether it is declared with `def`, visible beyond its file, rather
    /// than with `fn`.
    pub public: bool,
}

/// A context a file declares, `context NAME: TYPE = DEFAULT` or `context
/// NAME: TYPE` (see the parser's [Context](crate::parser#context)): with a
/// default, its item is the `let` of its binding, a name no source can
/// write ([`crate::names::context_binding`]), whose value is its default's
/// function, pure and of no parameters, returning its type; without one,
/// an [`ItemKind::Context`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextDecl {
    /// Its item.
    pub item: ItemId,
    /// Its name.
    pub name: Name,
    /// Where the name is written.
    pub name_span: Span,
    /// Its type.
    pub ty: TypeExprId,
    /// Its default's binding, if it has a default.
    pub binding: Option<Name>,
}

/// What has type parameters: a generic function item, `fn NAME<T, U>(...)`
/// or `def NAME<T>(...)` (see the parser's [Functions and
/// calls](crate::parser#functions-and-calls)), or a generic `type` item,
/// `type NAME<T> = TYPE` (see [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericOwner {
    /// A generic function item's function.
    Fn(FnId),
    /// A generic `type` item.
    Type(ItemId),
}

/// The type parameters of a generic function or `type` item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generics {
    /// What has them.
    pub owner: GenericOwner,
    /// Each type parameter's name and where it is written, in order.
    pub params: Box<[(Name, Span)]>,
    /// The type expressions the parser made of the function's signature and
    /// body, or of the type's definition, by their numbers, from the first
    /// to the one after the last: where its type parameters are in scope.
    pub types: (u32, u32),
}

/// One parameter of a function expression, `NAME: TYPE`, or `NAME: TYPE =
/// DEFAULT` (see [Functions](#functions)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Param {
    /// The parameter's name, always an identifier.
    pub name: Name,
    /// Where it is written: its name through its type.
    pub span: Span,
    /// Its type.
    pub ty: TypeExprId,
    /// Its default, the expression after its `=`, if it has one.
    pub default: Option<ExprId>,
    /// The default function of its default, if it has one that is no
    /// constant ([`is_constant`]): a function of the parameters before it,
    /// whose body is the default, which a call that leaves the parameter out
    /// calls on its side of the call (see the parser's [Functions and
    /// calls](crate::parser#functions-and-calls)).
    pub default_fn: Option<FnId>,
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

    /// The marker of a function type's meta of this effect, as it is
    /// written and shown: `META:`, `META->>` or `META->` (see
    /// [Types](#types)).
    #[must_use]
    pub const fn meta_marker(self) -> &'static str {
        match self {
            Effect::Pure => "META:",
            Effect::Memo => "META->>",
            Effect::Impure => "META->",
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
    /// `NAME = EXPR`, filling the parameter named `NAME`; or, after another
    /// named argument, `NAME` alone, shorthand for `NAME = NAME` (see the
    /// parser's [Functions and calls](crate::parser#functions-and-calls)).
    Named {
        /// The parameter's name.
        name: Name,
        /// Where the name is written.
        span: Span,
        /// Whether it is the shorthand, its name alone: its value is then a
        /// read of the variable of its name, written at the same span.
        shorthand: bool,
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
/// `"log level".format` or `conf.hosts[i + 1].up`; a first name that is a
/// namespace key as its def-type and a space, as a definition is written,
/// `package dev.peios.testpkg`, and one that is a meta key as `defmeta`, its
/// def-type and a space, the meta's path after it as written, `defmeta
/// package dev.peios.testpkg` (see [`crate::names`], "Namespace keys"). The
/// names of a definition's path, after a namespace key or in a path shown by
/// [`Ast::show_definition_path`], are shown as their source spells them,
/// never quoted as a field's name that is no identifier is: each is an
/// identifier, or a name with `+` in it, `org.gnu.gmp-c++`, or, after the
/// first, one starting with a digit, `org.golang.go-stage0-1.24` (see the
/// parser's "Def-types"). Returned by [`Ast::show_path`] and
/// [`Ast::show_definition_path`].
#[derive(Clone, Copy, Debug)]
pub struct PathDisplay<'a> {
    names: &'a Names,
    strings: &'a Strings,
    segments: &'a [PathSegment],
    /// Whether the path is a definition's, made with a def-type, whose
    /// names are shown as written.
    definition: bool,
}

impl fmt::Display for PathDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // What goes before the next name: nothing before the first, a space
        // after a namespace key, which is shown as its def-type so that the
        // path reads as a definition is written (see [`crate::names`],
        // "Namespace keys"), and `.` otherwise.
        let mut separator = "";
        // Whether the name is a meta's path as one name, shown as written.
        let mut meta = false;
        // Whether the names are a definition's path, each shown as written.
        let mut definition = self.definition;
        for (index, segment) in self.segments.iter().enumerate() {
            match segment.kind {
                SegmentKind::Name(name) => {
                    let text = self.names.get(name);
                    if index == 0
                        && let Some(def_type) = crate::names::key_def_type(text)
                    {
                        f.write_str(def_type)?;
                        (separator, definition) = (" ", true);
                        continue;
                    }
                    if index == 0
                        && let Some(def_type) = crate::names::key_meta(text)
                    {
                        write!(f, "defmeta {def_type}")?;
                        (separator, meta) = (" ", true);
                        continue;
                    }
                    f.write_str(separator)?;
                    if std::mem::take(&mut meta) || definition {
                        f.write_str(text)?;
                    } else {
                        FieldName(text).fmt(f)?;
                    }
                    separator = ".";
                }
                SegmentKind::Index { text, .. } => {
                    write!(f, "[{}]", self.strings.get(text))?;
                }
            }
        }
        Ok(())
    }
}

/// One entry of a record type expression: a field, `NAME: TYPE` or `NAME:
/// TYPE = DEFAULT`, a spread, `...TYPE`, whose fields join the record type,
/// or a rest-field, `*: TYPE`, the type of every field the record type does
/// not name (see [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldTy {
    /// What the entry is: a field and its name, a spread or a rest-field.
    pub kind: FieldTyKind,
    /// Where the name is written, or a spread's `...`, or a rest-field's
    /// `*`.
    pub span: Span,
    /// The field's type, the type a spread spreads, or the rest-field's.
    pub ty: TypeExprId,
    /// The field's default, if it has one; a spread never has one, and a
    /// rest-field's is reported by the checker.
    pub default: Option<FieldDefault>,
}

/// What a [`FieldTy`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldTyKind {
    /// A field, its name plain or quoted in source.
    Field(Name),
    /// A spread, `...TYPE`.
    Spread,
    /// A rest-field, `*: TYPE`.
    Rest,
}

impl FieldTy {
    /// The field's name, or `None` for a spread or a rest-field.
    #[must_use]
    pub const fn name(&self) -> Option<Name> {
        match self.kind {
            FieldTyKind::Field(name) => Some(name),
            FieldTyKind::Spread | FieldTyKind::Rest => None,
        }
    }
}

/// What [`TypeArena::copy`] makes a node of a kind of, if anything: the
/// kind the node is given in place of its own, with nothing under it.
type Replace<'r> = &'r dyn Fn(TypeExprId) -> Option<TypeExprKind>;

/// What [`TypeArena::copy`] makes the constant of a field's default that is
/// an expression of the tree copied from: the constant it is written as,
/// if it is one.
type ToConstant<'c> = &'c dyn Fn(ExprId) -> Option<Constant>;

/// The default of a field of a record type expression (see
/// [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldDefault {
    /// As the parser made it: the expression after the field's `=`, of the
    /// tree whose arena holds the field.
    Expr(ExprId),
    /// As a copy of a declaration's definition holds it: an index into the
    /// pool of constants of the arena that holds the field
    /// ([`Ast::copied_default`]).
    Copied(u32),
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
    /// A duration literal, `30s` or `1h30m`: a number of nanoseconds.
    Duration(i64),
    /// A size literal, `2GiB`: a number of bytes.
    Size(i64),
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
    /// A variant as a value, `ENUM::NAME`, as in `Toolchain::C`: the enum
    /// a type written by its name or qualified, which the expansion of the
    /// tree's types resolves as any is (see [Enums](#enums)). The name
    /// ends the expression's span.
    Variant {
        /// The enum, as a type expression.
        of: TypeExprId,
        /// The variant's name.
        name: Name,
    },
    /// A def-type reference, `DT PATH`, as in `package dev.peios.testpkg`:
    /// the function that calls the def-type with the definition's path, the
    /// definition and its own arguments (see [Def-types](#def-types)). Its
    /// details are in the def-type arena (see [`Ast::def_type`]).
    DefTypeRef(DefTypeId),
    /// A read of a context, `^NAME`, as in `^arch` (see the parser's
    /// [Context](crate::parser#context)). The name ends the expression's
    /// span.
    ContextRead(Name),
    /// `BODY with { NAME = VALUE; ... }`, or `with { ... } { BODY }`: the
    /// body evaluated with the contexts the bindings name bound (see the
    /// parser's [Context](crate::parser#context)).
    With {
        /// The bindings, in source order (see [`Ast::with_bindings`]).
        bindings: WithRange,
        /// What is evaluated with them.
        body: ExprId,
    },
    /// What the parser put in place of source it could not parse, or of a
    /// lexical error's token (see the parser's [Errors and
    /// recovery](crate::parser#errors-and-recovery)): its error is
    /// reported already, so it is of type `Error`, which every use accepts
    /// silently. Only ever in a tree with syntax errors ([`Ast::syntax`]),
    /// which is never lowered.
    Error,
    /// A field access with no name after its `.`, `EXPR.`, as an editor
    /// leaves one half-typed: what the parser kept of it, its record,
    /// which is checked as any expression is, the access itself of type
    /// `Error` (see the parser's [Errors and
    /// recovery](crate::parser#errors-and-recovery)). Only ever in a tree
    /// with syntax errors, which is never lowered.
    NoField(ExprId),
    /// A call whose `(` is never closed, `callee(ARGS` at the end of the
    /// input or of the block around it, as an editor leaves one
    /// half-typed: what the parser kept of it, its callee and the
    /// arguments it parsed, each checked as any expression is, the call
    /// itself of type `Error` (see the parser's [Errors and
    /// recovery](crate::parser#errors-and-recovery)). Only ever in a tree
    /// with syntax errors, which is never lowered.
    Unclosed {
        /// The function called.
        callee: ExprId,
        /// The arguments parsed, in source order.
        args: ArgRange,
    },
}

impl ExprKind {
    /// Whether it stands for source the parser could not parse, or could
    /// not finish: an error node, a field access with no name, an unclosed
    /// call. Only a tree with syntax errors has one, which is never
    /// lowered.
    #[must_use]
    pub const fn is_unparsed(&self) -> bool {
        matches!(self, Self::Error | Self::NoField(_) | Self::Unclosed { .. })
    }
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
    /// through its condition. A def-type definition's `Fn` covers its path
    /// through its body's `}`, and a `DefTypeRef` its def-type through its
    /// path.
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
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Interp(_)
            | ExprKind::If { .. }
            | ExprKind::Paren(_)
            | ExprKind::Name(_)
            | ExprKind::Variant { .. }
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
            | ExprKind::For(_)
            | ExprKind::DefTypeRef(_)
            | ExprKind::ContextRead(_)
            | ExprKind::With { .. }
            | ExprKind::Error
            | ExprKind::NoField(_)
            | ExprKind::Unclosed { .. } => None,
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
    /// `PATH = EXPR` or `PATH: TYPE = EXPR`, as in `x = 1`, `"quoted name"
    /// = 1`, `a.b = 1` or `x: Int = 1`: declares a field of the record its
    /// block evaluates to, named by the path's first segment, which is also
    /// a name in scope for the block's later items. A dotted path declares
    /// or extends a field that is a record built by such paths, and an
    /// annotation then applies to the path's last segment, as for a `let`.
    /// A public function item, `def NAME(...) ...`, is one of these too: the
    /// field `NAME`, without an annotation, whose value is the function
    /// expression, with `function` set.
    Field {
        /// The field's name, then any further segments.
        path: PathRange,
        /// The annotated type, if any.
        ty: Option<TypeExprId>,
        /// The field's value, or the value at the end of the path.
        value: ExprId,
        /// Whether it was written as a public function item, `def PATH(...)
        /// ...`, rather than as `PATH = fn(...) ...`.
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
    /// `type NAME = TYPE`: declares `NAME` in the file's namespace of types,
    /// an alias of `TYPE` (see [Types](#types)). Only ever an item of a
    /// file's outermost block. It has no expression.
    Type {
        /// The name declared, always an identifier.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// The type it stands for.
        ty: TypeExprId,
    },
    /// `enum NAME extensible<BOUND>` or `enum NAME extensible`: declares
    /// `NAME` in the file's namespace of types, a nominal type whose values
    /// are the variants any module adds to it, each with a payload that fits
    /// `BOUND`, if it has one (see [Enums](#enums)). Only ever an item of a
    /// file's outermost block. It has no expression.
    Enum {
        /// The name declared, always an identifier.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// The bound every variant's payload fits, if it has one.
        bound: Option<TypeExprId>,
    },
    /// `context NAME: TYPE`, a context with no default (see the parser's
    /// [Context](crate::parser#context)); one with a default is the `let`
    /// of its default's function under a binding no source can write
    /// ([`ContextDecl`]). Only ever an item of a file's outermost block. It
    /// has no expression.
    Context {
        /// The context's name, always an identifier.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// Its type.
        ty: TypeExprId,
    },
    /// `enum ENUM::NAME<PAYLOAD>` or `enum ENUM::NAME`: adds the variant
    /// `NAME` to the enum `ENUM`, a type written by its name or qualified,
    /// whose payload is of type `PAYLOAD`, `{}` without one (see
    /// [Enums](#enums)). Only ever an item of a file's outermost block. It
    /// has no expression.
    Variant {
        /// The enum, as a type expression.
        of: TypeExprId,
        /// The variant's name, always an identifier.
        name: Name,
        /// Where the name is written.
        name_span: Span,
        /// Its payload's type, if it is written.
        payload: Option<TypeExprId>,
    },
    /// `def DT PATH(PARAMS) BODY`: a definition made with the def-type `DT`,
    /// as in `def package dev.peios.testpkg() { ... }` (see
    /// [Def-types](#def-types)). It binds nothing a variable or a field
    /// could name: its function is a leaf of `DT`'s namespace. Only ever an
    /// item of a file's outermost block.
    DefTypeDef {
        /// Its def-type and path, in the def-type arena.
        def: DefTypeId,
        /// The function expression, whose parameters are written by name
        /// alone, their types, its return type and its marker being the
        /// def-type's to say.
        value: ExprId,
    },
    /// `defmeta DT PATH { ... }` or `defmeta DT PATH = EXPR`: the meta of
    /// the definition `def DT PATH(...)` of the same directory (see
    /// [Def-types](#def-types)). It binds nothing a source can read: it is
    /// reached only through its definition. Only ever an item of a file's
    /// outermost block.
    DefMeta {
        /// Its def-type and path, in the def-type arena.
        def: DefTypeId,
        /// A function expression of no parameters whose body is the block,
        /// or a block whose result is the expression; its return type and
        /// its marker, which it does not write, are the meta's its def-type
        /// declares.
        value: ExprId,
    },
    /// What the parser put in place of an item it could not parse, its
    /// span the source it skipped (see the parser's [Errors and
    /// recovery](crate::parser#errors-and-recovery)): its error is
    /// reported already. It declares nothing, has no value, and is no
    /// field, result or statement of its block, whose type is `Error`.
    /// Only ever in a tree with syntax errors ([`Ast::syntax`]).
    Error,
}

impl ItemKind {
    /// The expression the item holds, if it has one: every item's but a
    /// `break`'s, `continue`'s or `return`'s without a value, an import's,
    /// a `type`'s, an `enum`'s, a context's without a default and an error
    /// item's.
    #[must_use]
    pub const fn value(self) -> Option<ExprId> {
        match self {
            ItemKind::Let { value, .. }
            | ItemKind::Field { value, .. }
            | ItemKind::Rebind { value, .. }
            | ItemKind::Spread(value)
            | ItemKind::Result(value)
            | ItemKind::Expr(value)
            | ItemKind::DefTypeDef { value, .. }
            | ItemKind::DefMeta { value, .. } => Some(value),
            ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => jump.value,
            ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::Enum { .. }
            | ItemKind::Variant { .. }
            | ItemKind::Context { .. }
            | ItemKind::Error => None,
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
    /// A type written as a name, such as `Int`, or the name of a declared
    /// type, such as `PackageDef`.
    Named(Name),
    /// A qualified type name, `t.PackageDef`: two or more names separated by
    /// `.`, in the path pool (see [Types](#types)).
    Path(PathRange),
    /// A record type, `{x: Int, "a b": String}` or `{...T, x: Int}`: its
    /// fields and spreads in source order. Repeated names are allowed here
    /// and reported by the checker.
    Record(FieldTyRange),
    /// A function type, `fn(Int, name: String): Bool`, `fn(Int) ->> Int` or
    /// `fn() -> Int`, with a meta or not, as in `fn(v: Version): PackageDef
    /// META-> PackageMeta` (see [Types](#types)).
    Fn {
        /// The parameters, each with its name if it has one.
        params: ParamTyRange,
        /// The return type.
        ret: TypeExprId,
        /// What it may do, as its marker says: `:`, `->>` or `->`.
        effect: Effect,
        /// Its meta, if it has one.
        meta: Option<MetaTy>,
    },
    /// A name applied to type arguments, `NAME<TYPE, ...>`, as in
    /// `Future<Int>` or `Pair<Int, String>`. The parser accepts any name and
    /// one argument or more; the checker knows how many each built-in type
    /// takes, and the workspace each declared type.
    Applied {
        /// The name.
        name: Name,
        /// Where the name is written.
        span: Span,
        /// The type arguments, in order.
        args: TypeArgRange,
    },
    /// A qualified name applied to type arguments, `t.Pair<Int, String>`:
    /// its path in the path pool, which the expansion of a tree's types
    /// resolves as it does a [`TypeExprKind::Path`], and its arguments.
    PathApplied {
        /// The qualified name.
        path: PathRange,
        /// The type arguments, in order.
        args: TypeArgRange,
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
    /// The definition of the declared type numbered so in a [`TypeStore`],
    /// which a type expression there refers to by name. Never in a tree.
    Ref(u32),
    /// The definition of the generic declared type numbered `decl` in a
    /// [`TypeStore`] applied to the type arguments `args`, which a type
    /// expression there applies it to by name. Never in a tree.
    Instance {
        /// The declaration.
        decl: u32,
        /// The type arguments, in order.
        args: TypeArgRange,
    },
    /// The type parameter numbered so of the generic declared type whose
    /// definition it is part of: in a [`TypeStore`], and in the copies of
    /// definitions in a tree, which each use of the type substitutes its
    /// arguments in (see [Types](#types)). Never written.
    Param(u8),
    /// The enum whose key is `key`, named `name`: the definition of an
    /// enum's declaration in a [`TypeStore`], a type of its own, and of each
    /// use of it (see [Types](#types)). Never written.
    Enum {
        /// Its key, a name no source can write that is its identity
        /// ([`crate::names::enum_key`]).
        key: Name,
        /// Its name, as it is declared.
        name: Name,
    },
    /// The declared type of the context `NAME`, of a parameter written
    /// `(^NAME)` with no type (see the parser's
    /// [Context](crate::parser#context)), which the checker resolves.
    Context(Name),
    /// A type that names nothing it can, put in place of a written one by
    /// the expansion of a tree's types ([`Ast::expand_types`]): its error,
    /// if it is this one's to report, is among the tree's problems
    /// ([`Ast::type_problems`]). Never written.
    Invalid,
}

/// The meta of a function type expression, `META-> PackageMeta`: what its
/// marker says reading it does, and its type (see [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MetaTy {
    /// What reading it does, as its marker says: `META:` pure, `META->>`
    /// memoised, `META->` impure.
    pub effect: Effect,
    /// Its type.
    pub ty: TypeExprId,
}

/// A type as written in source, in an annotation or an ascription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeExpr {
    /// What the type expression is.
    pub kind: TypeExprKind,
    /// The source it covers.
    pub span: Span,
}

/// The type expressions of a tree, or of a [`TypeStore`]: the nodes, in an
/// arena indexed by [`TypeExprId`], with the fields of record types and the
/// parameters of function types each in a pool of their own, and the
/// defaults of fields that copies hold ([`FieldDefault::Copied`]) in a
/// third, each the constant it is, or `None` for one that is not a
/// constant.
#[derive(Clone, Debug, Default)]
pub struct TypeArena {
    exprs: Vec<TypeExpr>,
    fields: Vec<FieldTy>,
    params: Vec<ParamTy>,
    args: Vec<TypeExprId>,
    constants: Vec<Option<Constant>>,
}

impl TypeArena {
    /// Adds `ty` to the arena and returns its id.
    fn alloc(&mut self, ty: TypeExpr) -> TypeExprId {
        let id = TypeExprId(next_id(self.exprs.len()));
        self.exprs.push(ty);
        id
    }

    /// Appends `fields` to the field pool as one record type's and returns
    /// their range.
    fn push_fields(&mut self, fields: &[FieldTy]) -> FieldTyRange {
        let start = next_id(self.fields.len());
        self.fields.extend_from_slice(fields);
        FieldTyRange {
            start,
            len: next_id(fields.len()),
        }
    }

    /// Appends `params` to the parameter pool as one function type's and
    /// returns their range.
    fn push_params(&mut self, params: &[ParamTy]) -> ParamTyRange {
        let start = next_id(self.params.len());
        self.params.extend_from_slice(params);
        ParamTyRange {
            start,
            len: next_id(params.len()),
        }
    }

    /// Appends `args` to the type-argument pool as one applied type's and
    /// returns their range.
    fn push_args(&mut self, args: &[TypeExprId]) -> TypeArgRange {
        let start = next_id(self.args.len());
        self.args.extend_from_slice(args);
        TypeArgRange {
            start,
            len: next_id(args.len()),
        }
    }

    /// The type arguments of the applied type whose arguments are `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not of the arena.
    fn args(&self, range: TypeArgRange) -> &[TypeExprId] {
        let start = range.start as usize;
        &self.args[start..start + range.len as usize]
    }

    /// The node `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not a node of the arena.
    fn get(&self, id: TypeExprId) -> &TypeExpr {
        &self.exprs[id.index()]
    }

    /// The kind of the node `id`, [`TypeExprKind::Invalid`] if the arena has
    /// no such node.
    fn kind(&self, id: TypeExprId) -> TypeExprKind {
        self.exprs
            .get(id.index())
            .map_or(TypeExprKind::Invalid, |ty| ty.kind)
    }

    /// Makes the node `id` one of kind `kind`, keeping its span.
    fn set_kind(&mut self, id: TypeExprId, kind: TypeExprKind) {
        if let Some(entry) = self.exprs.get_mut(id.index()) {
            entry.kind = kind;
        }
    }

    /// The fields of the record type whose fields are `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not of the arena.
    fn fields(&self, range: FieldTyRange) -> &[FieldTy] {
        let start = range.start as usize;
        &self.fields[start..start + range.len as usize]
    }

    /// The parameters of the function type whose parameters are `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not of the arena.
    fn params(&self, range: ParamTyRange) -> &[ParamTy] {
        let start = range.start as usize;
        &self.params[start..start + range.len as usize]
    }

    /// The number of nodes.
    fn len(&self) -> usize {
        self.exprs.len()
    }

    /// The ids of the nodes from number `start` on, in order.
    fn ids_from(&self, start: usize) -> impl Iterator<Item = TypeExprId> + use<> {
        (next_id(start)..next_id(self.exprs.len())).map(TypeExprId)
    }

    /// Copies the node `root` of `from`, and every node under it, into this
    /// arena, children first, and returns the copy of `root`: a node `memo`
    /// has a copy of already is that copy, shared, and every node copied is
    /// added to it; a node `replace` gives a kind is a node of that kind, of
    /// its span, with nothing under it, but an applied type it makes a
    /// [`TypeExprKind::Ref`], which is an [`TypeExprKind::Instance`] of its
    /// arguments, copied; every other node is a node of its
    /// kind and span, its children copied. A qualified name `replace` gives
    /// no kind is [`TypeExprKind::Invalid`], its path being of `from`'s tree.
    /// A field's default is copied into this arena's pool of constants: one
    /// `from` holds as an expression, of `from`'s tree, as what `constant`
    /// makes of it, and one `from` holds in its pool as it is there.
    /// Recurses once per level of nesting under `root`.
    fn copy(
        &mut self,
        from: &TypeArena,
        root: TypeExprId,
        memo: &mut HashMap<TypeExprId, TypeExprId>,
        (replace, constant): (Replace<'_>, ToConstant<'_>),
    ) -> TypeExprId {
        if let Some(&copied) = memo.get(&root) {
            return copied;
        }
        let TypeExpr { kind, span } = from.exprs.get(root.index()).copied().unwrap_or(TypeExpr {
            kind: TypeExprKind::Invalid,
            span: Span::new(0, 0),
        });
        let kind = match (replace(root), kind) {
            // A declared type applied to arguments, whose arguments are
            // copied as its instance's.
            (
                Some(TypeExprKind::Ref(decl)),
                TypeExprKind::Applied { args, .. } | TypeExprKind::PathApplied { args, .. },
            ) => TypeExprKind::Instance {
                decl,
                args: self.copy_args(from, args, memo, (replace, constant)),
            },
            (Some(kind), _) => kind,
            (None, kind) => match kind {
                TypeExprKind::Record(range) => {
                    let fields: Vec<FieldTy> = from
                        .fields(range)
                        .iter()
                        .map(|field| {
                            let default = field.default.map(|default| {
                                let copied = match default {
                                    FieldDefault::Expr(expr) => constant(expr),
                                    FieldDefault::Copied(at) => {
                                        from.constants.get(at as usize).cloned().flatten()
                                    }
                                };
                                let at = next_id(self.constants.len());
                                self.constants.push(copied);
                                FieldDefault::Copied(at)
                            });
                            FieldTy {
                                ty: self.copy(from, field.ty, memo, (replace, constant)),
                                default,
                                ..*field
                            }
                        })
                        .collect();
                    TypeExprKind::Record(self.push_fields(&fields))
                }
                TypeExprKind::Fn {
                    params,
                    ret,
                    effect,
                    meta,
                } => {
                    let params: Vec<ParamTy> = from
                        .params(params)
                        .iter()
                        .map(|param| ParamTy {
                            ty: self.copy(from, param.ty, memo, (replace, constant)),
                            ..*param
                        })
                        .collect();
                    let ret = self.copy(from, ret, memo, (replace, constant));
                    let meta = meta.map(|meta| MetaTy {
                        ty: self.copy(from, meta.ty, memo, (replace, constant)),
                        ..meta
                    });
                    TypeExprKind::Fn {
                        params: self.push_params(&params),
                        ret,
                        effect,
                        meta,
                    }
                }
                TypeExprKind::Applied { name, span, args } => TypeExprKind::Applied {
                    name,
                    span,
                    args: self.copy_args(from, args, memo, (replace, constant)),
                },
                TypeExprKind::List(element) => {
                    TypeExprKind::List(self.copy(from, element, memo, (replace, constant)))
                }
                TypeExprKind::Union { lhs, rhs, bar } => {
                    let lhs = self.copy(from, lhs, memo, (replace, constant));
                    let rhs = self.copy(from, rhs, memo, (replace, constant));
                    TypeExprKind::Union { lhs, rhs, bar }
                }
                TypeExprKind::Instance { decl, args } => TypeExprKind::Instance {
                    decl,
                    args: self.copy_args(from, args, memo, (replace, constant)),
                },
                TypeExprKind::Path(_) | TypeExprKind::PathApplied { .. } => TypeExprKind::Invalid,
                kind @ (TypeExprKind::Named(_)
                | TypeExprKind::Ref(_)
                | TypeExprKind::Param(_)
                | TypeExprKind::Enum { .. }
                | TypeExprKind::Context(_)
                | TypeExprKind::Invalid) => kind,
            },
        };
        let id = self.alloc(TypeExpr { kind, span });
        memo.insert(root, id);
        id
    }

    /// The node `root` with the type argument `args[i]` in place of each
    /// [`TypeExprKind::Param`] `i` under it: an argument the arena has no
    /// node for, `args` lacking it, a node `Invalid`. A node with no
    /// parameter under it is itself, shared, and so is a parameter's
    /// argument; every other is a new node of its kind and span, its
    /// children substituted, made children first, with the defaults of a
    /// record type's fields as they are. `memo` holds what each node
    /// substituted is, so that a node shared under `root` is substituted
    /// once.
    ///
    /// Recurses once per level of nesting under `root`.
    fn substitute(
        &mut self,
        root: TypeExprId,
        args: &[TypeExprId],
        memo: &mut HashMap<TypeExprId, TypeExprId>,
    ) -> TypeExprId {
        if let Some(&done) = memo.get(&root) {
            return done;
        }
        let Some(&TypeExpr { kind, span }) = self.exprs.get(root.index()) else {
            return root;
        };
        let kind = match kind {
            TypeExprKind::Param(index) => {
                let invalid = TypeExpr {
                    kind: TypeExprKind::Invalid,
                    span,
                };
                let done =
                    (args.get(usize::from(index)).copied()).unwrap_or_else(|| self.alloc(invalid));
                memo.insert(root, done);
                return done;
            }
            TypeExprKind::Record(range) => {
                let fields = self.fields(range).to_vec();
                let substituted: Vec<FieldTy> = fields
                    .iter()
                    .map(|field| FieldTy {
                        ty: self.substitute(field.ty, args, memo),
                        ..*field
                    })
                    .collect();
                (substituted != fields)
                    .then(|| TypeExprKind::Record(self.push_fields(&substituted)))
            }
            TypeExprKind::Fn {
                params,
                ret,
                effect,
                meta,
            } => {
                let written = self.params(params).to_vec();
                let substituted: Vec<ParamTy> = written
                    .iter()
                    .map(|param| ParamTy {
                        ty: self.substitute(param.ty, args, memo),
                        ..*param
                    })
                    .collect();
                let new_ret = self.substitute(ret, args, memo);
                let new_meta = meta.map(|meta| MetaTy {
                    ty: self.substitute(meta.ty, args, memo),
                    ..meta
                });
                (substituted != written || new_ret != ret || new_meta != meta).then(|| {
                    TypeExprKind::Fn {
                        params: self.push_params(&substituted),
                        ret: new_ret,
                        effect,
                        meta: new_meta,
                    }
                })
            }
            TypeExprKind::Applied {
                args: given,
                name,
                span,
            } => self
                .substitute_args(given, args, memo)
                .map(|given| TypeExprKind::Applied {
                    name,
                    span,
                    args: given,
                }),
            TypeExprKind::Instance { decl, args: given } => self
                .substitute_args(given, args, memo)
                .map(|given| TypeExprKind::Instance { decl, args: given }),
            TypeExprKind::List(element) => {
                let new = self.substitute(element, args, memo);
                (new != element).then_some(TypeExprKind::List(new))
            }
            TypeExprKind::Union { lhs, rhs, bar } => {
                let new = (
                    self.substitute(lhs, args, memo),
                    self.substitute(rhs, args, memo),
                );
                (new != (lhs, rhs)).then_some(TypeExprKind::Union {
                    lhs: new.0,
                    rhs: new.1,
                    bar,
                })
            }
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::PathApplied { .. }
            | TypeExprKind::Ref(_)
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => None,
        };
        let done = match kind {
            Some(kind) => self.alloc(TypeExpr { kind, span }),
            None => root,
        };
        memo.insert(root, done);
        done
    }

    /// The type arguments `given` with [`TypeArena::substitute`]'s
    /// substitution of `args` made in each, if that changes any.
    fn substitute_args(
        &mut self,
        given: TypeArgRange,
        args: &[TypeExprId],
        memo: &mut HashMap<TypeExprId, TypeExprId>,
    ) -> Option<TypeArgRange> {
        let written = self.args(given).to_vec();
        let substituted: Vec<TypeExprId> = written
            .iter()
            .map(|&arg| self.substitute(arg, args, memo))
            .collect();
        (substituted != written).then(|| self.push_args(&substituted))
    }

    /// Copies the type arguments `args` of `from` into this arena, as
    /// [`TypeArena::copy`] copies each, and returns their range here.
    fn copy_args(
        &mut self,
        from: &TypeArena,
        args: TypeArgRange,
        memo: &mut HashMap<TypeExprId, TypeExprId>,
        (replace, constant): (Replace<'_>, ToConstant<'_>),
    ) -> TypeArgRange {
        let copied: Vec<TypeExprId> = from
            .args(args)
            .iter()
            .map(|&arg| self.copy(from, arg, memo, (replace, constant)))
            .collect();
        self.push_args(&copied)
    }

    /// The height of the node `id` as it is now, its children's being in
    /// `heights` by id: 1 for one with no children (1 too for a child
    /// `heights` lacks), one more than its tallest child otherwise.
    fn height(&self, id: TypeExprId, heights: &[u32]) -> u32 {
        self.children(id)
            .iter()
            .map(|child| heights.get(child.index()).copied().unwrap_or(1))
            .max()
            .map_or(1, |tallest| tallest.saturating_add(1))
    }

    /// The children of the node `id`, as they are now: a record type's
    /// fields' types, a function type's parameters', return type and meta's,
    /// an applied type's arguments, a list type's element, a union's sides.
    fn children(&self, id: TypeExprId) -> Vec<TypeExprId> {
        match self.kind(id) {
            TypeExprKind::Record(range) => self.fields(range).iter().map(|f| f.ty).collect(),
            TypeExprKind::Fn {
                params, ret, meta, ..
            } => self
                .params(params)
                .iter()
                .map(|p| p.ty)
                .chain([ret])
                .chain(meta.map(|meta| meta.ty))
                .collect(),
            TypeExprKind::Applied { args, .. }
            | TypeExprKind::PathApplied { args, .. }
            | TypeExprKind::Instance { args, .. } => self.args(args).to_vec(),
            TypeExprKind::List(element) => vec![element],
            TypeExprKind::Union { lhs, rhs, .. } => vec![lhs, rhs],
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::Ref(_)
            | TypeExprKind::Param(_)
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => Vec::new(),
        }
    }

    /// Merges the spreads of the record type `id` into its fields, as value
    /// spread merges a record block's (see [Types](#types)): a spread's
    /// fields join the record SOFT; a later field of a soft field's name
    /// replaces it, and so does a later spread's; a spread's field of the
    /// name of a field written before it is the problem
    /// [`TypeProblem::Replaces`], once for the spread, the field written
    /// before standing. A spread of anything but a record type is
    /// [`TypeProblem::NotRecord`], or, of an erroneous type,
    /// [`TypeProblem::Erroneous`], and spreads nothing. Two fields written of
    /// one name are both kept, for the checker to report; a spread of a
    /// record type with two of one name spreads the first, the later being
    /// [`TypeProblem::Repeated`]. A rest-field merges as a field of its own
    /// name would: a spread's is soft, and replaces or is replaced as a
    /// field is. The node becomes
    /// the record type of the merged fields; one without a spread is left
    /// as it is. Returns the problems, in source order. A spread's operand
    /// must have its own spreads merged already.
    fn merge_record(&mut self, id: TypeExprId) -> Vec<TypeProblem> {
        let TypeExprKind::Record(range) = self.kind(id) else {
            return Vec::new();
        };
        let written = self.fields(range);
        if written
            .iter()
            .all(|field| field.kind != FieldTyKind::Spread)
        {
            return Vec::new();
        }
        let written = written.to_vec();
        let mut merged: Vec<(FieldTy, bool)> = Vec::with_capacity(written.len());
        // Where each field is merged, by its name, `None` being the
        // rest-field's.
        let mut at: HashMap<Option<Name>, usize> = HashMap::new();
        let mut problems = Vec::new();
        // Whether the field merged at an index is soft.
        let soft = |merged: &[(FieldTy, bool)], index: usize| {
            merged.get(index).is_some_and(|&(_, soft)| soft)
        };
        for entry in written {
            if entry.kind != FieldTyKind::Spread {
                let name = entry.name();
                match at.get(&name).copied() {
                    Some(index) if soft(&merged, index) => {
                        if let Some(slot) = merged.get_mut(index) {
                            *slot = (entry, false);
                        }
                    }
                    _ => {
                        at.entry(name).or_insert(merged.len());
                        merged.push((entry, false));
                    }
                }
                continue;
            }
            let operand = entry.ty;
            let spread = match self.kind(operand) {
                TypeExprKind::Record(spread) => spread,
                TypeExprKind::Invalid => {
                    problems.push(TypeProblem::Erroneous);
                    continue;
                }
                _ => {
                    problems.push(TypeProblem::NotRecord(operand));
                    continue;
                }
            };
            // The first field written before the spread, and the number of
            // others.
            let mut replaced: Option<(Option<Name>, TypeExprId, Span)> = None;
            let mut others = 0_u32;
            // The names the spread has given so far: one it gives twice, a
            // record type written in place with a name twice, is the
            // repeated-name error it is anywhere else, the later dropped.
            let mut given: HashSet<Option<Name>> = HashSet::new();
            for field in self.fields(spread) {
                if field.kind == FieldTyKind::Spread {
                    continue;
                }
                let name = field.name();
                if !given.insert(name) {
                    problems.push(TypeProblem::Repeated {
                        name,
                        at: field.span,
                    });
                    continue;
                }
                match at.get(&name).copied() {
                    Some(index) if soft(&merged, index) => {
                        if let Some(slot) = merged.get_mut(index) {
                            *slot = (*field, true);
                        }
                    }
                    Some(index) => match replaced {
                        None => {
                            let first = merged.get(index).map_or(entry.span, |(hard, _)| hard.span);
                            replaced = Some((name, field.ty, first));
                        }
                        Some(_) => others = others.saturating_add(1),
                    },
                    None => {
                        at.insert(name, merged.len());
                        merged.push((*field, true));
                    }
                }
            }
            if let Some((name, ty, first)) = replaced {
                problems.push(TypeProblem::Replaces {
                    operand,
                    name,
                    ty,
                    first,
                    others,
                });
            }
        }
        let fields: Vec<FieldTy> = merged.into_iter().map(|(field, _)| field).collect();
        let range = self.push_fields(&fields);
        self.set_kind(id, TypeExprKind::Record(range));
        problems
    }
}

/// What the expansion of a tree's types ([`Ast::expand_types`]) found
/// wrong with one of its type expressions, for the checker to report where
/// it resolves it (see [Types](#types)).
#[derive(Clone, Debug, PartialEq)]
pub enum TypeProblem {
    /// The type names nothing it can: the error, as it is reported. The
    /// node is [`TypeExprKind::Invalid`].
    Unresolved(Diagnostic),
    /// The record type spreads the type `operand`, which is not a record
    /// type: "cannot spread type T: it is not a record type", at the
    /// operand.
    NotRecord(TypeExprId),
    /// The record type spreads the type `operand`, which has a field
    /// `name`, of type `ty`, that the record type writes before the spread,
    /// its name at `first`, and `others` more such fields: "field `x` is
    /// defined twice", at the operand; for the rest-field, "the rest-field
    /// is defined twice".
    Replaces {
        /// The type spread.
        operand: TypeExprId,
        /// The first field it has that is written before it: its name, or
        /// `None` for the rest-field.
        name: Option<Name>,
        /// The type it has that field of.
        ty: TypeExprId,
        /// Where the field written before it is.
        first: Span,
        /// How many more such fields it has.
        others: u32,
    },
    /// The record type spreads a type that is erroneous, whose error is
    /// reported where it is written: the record type is erroneous too.
    Erroneous,
    /// The record type spreads a record type written in place that has the
    /// field `name` twice, the later at `at`: "field `x` appears twice in
    /// this type", at the later, as anywhere else; for the rest-field, "the
    /// rest-field appears twice in this type".
    Repeated {
        /// The name written twice, or `None` for the rest-field.
        name: Option<Name>,
        /// Where it is written the second time.
        at: Span,
    },
}

/// What a type expression of a tree that reads a declared type by its name
/// names, as the workspace resolves it in the tree's file (see
/// [`crate::workspace`], "Types"), for [`Ast::expand_types`] and
/// [`TypeStore::declare`]. A name of no declared type has none: a built-in
/// type, or an unknown one, which the checker reports.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeLink {
    /// The declaration numbered so in the [`TypeStore`].
    Decl(u32),
    /// Nothing it can: the error.
    Error(Diagnostic),
}

/// Where a [`TypeStore`] was, to put it back there ([`TypeStore::mark`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeStoreMark {
    exprs: usize,
    fields: usize,
    params: usize,
    args: usize,
    constants: usize,
    decls: usize,
}

/// What a host knows an attribute by (see the parser's
/// [Attributes](crate::parser#attributes)): its declaration, by the module
/// that declares it, the path of its file without `.gcsl`, and its name;
/// or one of GCSL's own, `retry` and `timeout`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttributeId {
    /// A `type` marked `#[attribute]`.
    Declared {
        /// The module that declares it.
        module: Rc<str>,
        /// The name it declares.
        name: Rc<str>,
    },
    /// One of GCSL's own, by its name.
    Builtin(&'static str),
}

impl AttributeId {
    /// Its name, as an attribute is written.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            AttributeId::Declared { name, .. } => name,
            AttributeId::Builtin(name) => name,
        }
    }
}

/// The definitions of declared types, apart from any tree, so that a tree
/// can be given the definition of a type another file declares, as if it
/// were written there (see [Types](#types)): each declaration's definition
/// copied out of its file's tree, each name of another declared type in it
/// a [`TypeExprKind::Ref`], until it is settled ([`TypeStore::settle`]),
/// after which it refers to nothing and holds no spread. A workspace's scan
/// has one; a session keeps one, its inputs' declarations after each
/// other's.
#[derive(Clone, Debug, Default)]
pub struct TypeStore {
    arena: TypeArena,
    /// The height of each node, once its declaration is settled: 1 for a
    /// name, one more than its tallest child otherwise.
    heights: Vec<u32>,
    /// Each declaration's definition: the number of its first node, and its
    /// root, every node from the first to the root being its own.
    decls: Vec<(u32, TypeExprId)>,
    /// The declarations marked `#[attribute]`, by their numbers, each with
    /// what a host knows it by.
    attributes: HashMap<u32, AttributeId>,
    /// How many type parameters each declaration has, by number: none but
    /// for a generic one.
    arities: Vec<u32>,
}

impl TypeStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of declarations.
    #[must_use]
    pub fn len(&self) -> u32 {
        next_id(self.decls.len())
    }

    /// Whether the store has no declaration.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }

    /// Where the store is now.
    #[must_use]
    pub fn mark(&self) -> TypeStoreMark {
        TypeStoreMark {
            exprs: self.arena.exprs.len(),
            fields: self.arena.fields.len(),
            params: self.arena.params.len(),
            args: self.arena.args.len(),
            constants: self.arena.constants.len(),
            decls: self.decls.len(),
        }
    }

    /// Forgets everything added since `mark`.
    pub fn truncate(&mut self, mark: TypeStoreMark) {
        self.arena.exprs.truncate(mark.exprs);
        self.arena.fields.truncate(mark.fields);
        self.arena.params.truncate(mark.params);
        self.arena.args.truncate(mark.args);
        self.arena.constants.truncate(mark.constants);
        self.heights.truncate(mark.exprs);
        self.decls.truncate(mark.decls);
        self.arities.truncate(mark.decls);
        let decls = u32::try_from(mark.decls).unwrap_or(u32::MAX);
        self.attributes.retain(|&decl, _| decl < decls);
    }

    /// Records that the declaration numbered `decl` is marked
    /// `#[attribute]`, known to a host as `id`.
    pub fn mark_attribute(&mut self, decl: u32, id: AttributeId) {
        self.attributes.insert(decl, id);
    }

    /// What a host knows the declaration numbered `decl` by, if it is
    /// marked `#[attribute]`.
    #[must_use]
    pub fn attribute(&self, decl: u32) -> Option<&AttributeId> {
        self.attributes.get(&decl)
    }

    /// Adds the declarations numbered from the store's length on, `count`
    /// of them, each defined as nothing yet, to be defined by
    /// [`TypeStore::declare`] in any order. Returns the first's number.
    pub fn reserve(&mut self, count: usize) -> u32 {
        let first = self.len();
        let nothing = TypeExprId(u32::MAX);
        self.decls
            .extend(std::iter::repeat_n((u32::MAX, nothing), count));
        self.arities.resize(self.decls.len(), 0);
        first
    }

    /// The enum the declaration numbered `decl` is, by its key and its
    /// name, if it is an enum's (see [Types](#types)).
    #[must_use]
    pub fn enum_of(&self, decl: u32) -> Option<(Name, Name)> {
        match self.arena.kind(self.root(decl)?) {
            TypeExprKind::Enum { key, name } => Some((key, name)),
            _ => None,
        }
    }

    /// Defines the declaration `decl` as the enum of the key `key`, named
    /// `name`, declared at `span`, an [`TypeExprKind::Enum`]: a type of its
    /// own, which nothing it refers to defines (see [Types](#types)).
    pub fn declare_enum(&mut self, decl: u32, (key, name): (Name, Name), span: Span) {
        let first = next_id(self.arena.len());
        let root = self.arena.alloc(TypeExpr {
            kind: TypeExprKind::Enum { key, name },
            span,
        });
        self.heights.resize(self.arena.len(), 1);
        if let Some(entry) = self.decls.get_mut(decl as usize) {
            *entry = (first, root);
        }
    }

    /// Records that the declaration numbered `decl` has `arity` type
    /// parameters, as [`Ast::type_arity`] says of its item.
    pub fn set_arity(&mut self, decl: u32, arity: u32) {
        if let Some(entry) = self.arities.get_mut(decl as usize) {
            *entry = arity;
        }
    }

    /// How many type parameters the declaration numbered `decl` has: none
    /// unless it is generic.
    #[must_use]
    pub fn arity(&self, decl: u32) -> u32 {
        self.arities.get(decl as usize).copied().unwrap_or(0)
    }

    /// Defines the declaration `decl` as the type expression `root` of
    /// `ast`, copied out of it, with each node `links` has an entry for (by
    /// id, in order) a [`TypeExprKind::Ref`] to the declaration it names, an
    /// [`TypeExprKind::Instance`] of it if it applies it to arguments, or
    /// [`TypeExprKind::Invalid`] if it names none it can; and each name that
    /// is a type parameter of the declaration's own, a generic one's, the
    /// [`TypeExprKind::Param`] of its number. Spreads are
    /// copied as they are written, until it is settled. A field's default
    /// is copied as the constant it is written as, if it is one
    /// (`check::written_constant`), which the checker of each tree using
    /// the declaration fits to the field's type; what is wrong in it, its
    /// own file's check reports.
    pub fn declare(
        &mut self,
        decl: u32,
        (ast, root): (&Ast, TypeExprId),
        links: &[(TypeExprId, TypeLink)],
    ) {
        let first = next_id(self.arena.len());
        let replace = |id: TypeExprId| {
            if let TypeExprKind::Named(name) = ast.types.kind(id)
                && let Some((GenericOwner::Type(_), index)) = ast.type_param(id, name)
            {
                return Some(TypeExprKind::Param(index));
            }
            let at = links.binary_search_by_key(&id.0, |&(at, _)| at.0).ok()?;
            Some(match links.get(at).map(|(_, link)| link) {
                Some(TypeLink::Decl(decl)) => TypeExprKind::Ref(*decl),
                _ => TypeExprKind::Invalid,
            })
        };
        let constant = |expr: ExprId| written_constant(ast, expr);
        let copied = self
            .arena
            .copy(&ast.types, root, &mut HashMap::new(), (&replace, &constant));
        self.heights.resize(self.arena.len(), 1);
        if let Some(entry) = self.decls.get_mut(decl as usize) {
            *entry = (first, copied);
        }
    }

    /// The declarations the definition of `decl` refers to by name, each
    /// with where, in its own nodes' order: the edges of a cycle of types.
    #[must_use]
    pub fn refs(&self, decl: u32) -> Vec<(u32, Span)> {
        let Some(&(first, root)) = self.decls.get(decl as usize) else {
            return Vec::new();
        };
        (first..=root.0)
            .filter_map(|at| {
                let ty = self.arena.exprs.get(at as usize)?;
                match ty.kind {
                    TypeExprKind::Ref(to) | TypeExprKind::Instance { decl: to, .. } => {
                        Some((to, ty.span))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// Settles the declaration `decl`, every declaration it refers to being
    /// settled already, or it being on a cycle of types (`cyclic`): each
    /// reference to another becomes what that one's definition is, shared,
    /// and each record type its spreads merged, its own nodes taken in
    /// order, children first (see [Types](#types)). It is erroneous, its
    /// definition then [`TypeExprKind::Invalid`] alone, if it is on a cycle,
    /// if a spread of it has a problem, or if it would be nested more than
    /// [`MAX_DEPTH`] levels deep; whatever is wrong is reported where it is
    /// written, in its file. A reference to an erroneous one is erroneous.
    pub fn settle(&mut self, decl: u32, cyclic: bool) {
        let Some(&(first, root)) = self.decls.get(decl as usize) else {
            return;
        };
        let mut erroneous = cyclic;
        for id in self
            .arena
            .ids_from(first as usize)
            .take_while(|&id| id.0 <= root.0)
        {
            if erroneous {
                break;
            }
            match self.arena.kind(id) {
                TypeExprKind::Ref(to) => {
                    let target = self.root(to);
                    let kind = target.map_or(TypeExprKind::Invalid, |at| self.arena.kind(at));
                    self.arena.set_kind(id, kind);
                    let height = target
                        .and_then(|at| self.heights.get(at.index()))
                        .copied()
                        .unwrap_or(1);
                    if let Some(entry) = self.heights.get_mut(id.index()) {
                        *entry = height;
                    }
                    continue;
                }
                TypeExprKind::Instance { decl: to, args } => {
                    let height = self.instantiate(id, to, args);
                    erroneous |= height as usize > MAX_DEPTH;
                    continue;
                }
                TypeExprKind::Record(_) => {
                    erroneous |= !self.arena.merge_record(id).is_empty();
                }
                _ => {}
            }
            let height = self.arena.height(id, &self.heights);
            erroneous |= height as usize > MAX_DEPTH;
            if let Some(entry) = self.heights.get_mut(id.index()) {
                *entry = height;
            }
        }
        if erroneous {
            self.arena.set_kind(root, TypeExprKind::Invalid);
            if let Some(entry) = self.heights.get_mut(root.index()) {
                *entry = 1;
            }
        }
    }

    /// Makes the node `id`, an instance of the declaration `decl` applied to
    /// `args`, what `decl`'s settled definition is with `args` substituted
    /// for its type parameters (`TypeArena::substitute`), the nodes made
    /// shared as a reference's are, or `Invalid` if `decl` is erroneous.
    /// Returns its height.
    fn instantiate(&mut self, id: TypeExprId, decl: u32, args: TypeArgRange) -> u32 {
        let args = self.arena.args(args).to_vec();
        let target = self
            .root(decl)
            .filter(|&root| self.arena.kind(root) != TypeExprKind::Invalid);
        let (kind, height) = match target {
            Some(root) => {
                let start = self.arena.len();
                let made = self.arena.substitute(root, &args, &mut HashMap::new());
                self.heights.resize(self.arena.len(), 1);
                for new in self.arena.ids_from(start) {
                    let height = self.arena.height(new, &self.heights);
                    if let Some(entry) = self.heights.get_mut(new.index()) {
                        *entry = height;
                    }
                }
                let height = self.heights.get(made.index()).copied().unwrap_or(1);
                (self.arena.kind(made), height)
            }
            None => (TypeExprKind::Invalid, 1),
        };
        self.arena.set_kind(id, kind);
        if let Some(entry) = self.heights.get_mut(id.index()) {
            *entry = height;
        }
        height
    }

    /// The root of the definition of `decl`, if it is defined.
    fn root(&self, decl: u32) -> Option<TypeExprId> {
        self.decls
            .get(decl as usize)
            .map(|&(_, root)| root)
            .filter(|root| root.index() < self.arena.len())
    }
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
    types: TypeArena,
    paths: Vec<PathSegment>,
    fns: Vec<FnDecl>,
    params: Vec<Param>,
    args: Vec<Arg>,
    with_bindings: Vec<WithBinding>,
    elements: Vec<Element>,
    fors: Vec<ForDecl>,
    labels: Vec<Label>,
    imports: Vec<ImportDecl>,
    import_members: Vec<ImportMember>,
    shadowable: Vec<ItemId>,
    def_type_marks: Vec<ItemId>,
    attribute_marks: Vec<ItemId>,
    attribute_uses: Vec<AttributeUse>,
    generics: Vec<Generics>,
    methods: Vec<MethodDecl>,
    contexts: Vec<ContextDecl>,
    def_types: Vec<DefTypeUse>,
    no_prelude: Option<Span>,
    syntax: Syntax,
    quoted: Vec<Span>,
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
/// [Calls](crate::check#functions)): a literal, a string without
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
    exprs: usize,
    items: usize,
    item_lists: usize,
    parts: usize,
    types: usize,
    fields: usize,
    param_tys: usize,
    type_args: usize,
    paths: usize,
    fns: usize,
    params: usize,
    args: usize,
    with_bindings: usize,
    elements: usize,
    fors: usize,
    labels: usize,
    imports: usize,
    import_members: usize,
    def_types: usize,
}

/// Whether a tree was parsed from source with syntax errors, and so how far
/// its shape can be trusted (see the parser's [Errors and
/// recovery](crate::parser#errors-and-recovery)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Syntax {
    /// No error: the tree is the source as written.
    #[default]
    Clean,
    /// Errors, each in an error node ([`ExprKind::Error`],
    /// [`ItemKind::Error`], [`TypeExprKind::Invalid`]) or left out of the
    /// tree; everything else as written.
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

    /// Marks the item `id` `#[shadowable]` (see [Attributes](#attributes)).
    pub fn mark_shadowable(&mut self, id: ItemId) {
        let at = self.shadowable.partition_point(|&marked| marked.0 < id.0);
        if self.shadowable.get(at) != Some(&id) {
            self.shadowable.insert(at, id);
        }
    }

    /// Marks the `type` item `id` `#[attribute]` (see the parser's
    /// [Attributes](crate::parser#attributes)).
    pub fn mark_attribute(&mut self, id: ItemId) {
        let at = self
            .attribute_marks
            .partition_point(|&marked| marked.0 < id.0);
        if self.attribute_marks.get(at) != Some(&id) {
            self.attribute_marks.insert(at, id);
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

    /// Marks the item `id` `#[def_type]` (see [Attributes](#attributes)).
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
    /// [Def-types](#def-types)).
    pub fn push_def_type(&mut self, def_type: DefTypeUse) -> DefTypeId {
        let id = DefTypeId(next_id(self.def_types.len()));
        self.def_types.push(def_type);
        id
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
        self.labels.truncate(mark.labels);
        self.imports.truncate(mark.imports);
        self.import_members.truncate(mark.import_members);
        self.def_types.truncate(mark.def_types);
        let items = next_id(mark.items);
        self.shadowable.retain(|item| item.0 < items);
        self.def_type_marks.retain(|item| item.0 < items);
        self.attribute_marks.retain(|item| item.0 < items);
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
            labels: self.labels,
            imports: self.imports,
            import_members: self.import_members,
            shadowable: self.shadowable,
            def_type_marks: self.def_type_marks,
            attribute_marks: self.attribute_marks,
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

/// A complete syntax tree: the arenas, the file's body, the names and the
/// string literals' text.
#[derive(Clone, Debug)]
pub struct Ast {
    exprs: Vec<Expr>,
    items: Vec<Item>,
    item_lists: Vec<ItemId>,
    parts: Vec<StrPart>,
    types: TypeArena,
    paths: Vec<PathSegment>,
    fns: Vec<FnDecl>,
    params: Vec<Param>,
    args: Vec<Arg>,
    with_bindings: Vec<WithBinding>,
    elements: Vec<Element>,
    fors: Vec<ForDecl>,
    labels: Vec<Label>,
    imports: Vec<ImportDecl>,
    import_members: Vec<ImportMember>,
    /// The items marked `#[shadowable]`, in id order.
    shadowable: Vec<ItemId>,
    /// The items marked `#[def_type]`, in id order.
    def_type_marks: Vec<ItemId>,
    /// The `type` items marked `#[attribute]`, in id order.
    attribute_marks: Vec<ItemId>,
    /// The uses of attributes that are none of the parser's, by the items
    /// they apply to, in id order, and on each item in source order.
    attribute_uses: Vec<AttributeUse>,
    /// The type parameters of the generic function items, in the order
    /// their functions were made.
    generics: Vec<Generics>,
    /// The methods of the outermost block, in source order.
    methods: Vec<MethodDecl>,
    /// The contexts the outermost block declares, in source order.
    contexts: Vec<ContextDecl>,
    /// The def-type definitions and references, in the order they were
    /// parsed (see [Def-types](#def-types)).
    def_types: Vec<DefTypeUse>,
    /// Where the file's `#![no_prelude]` is written, if it has one.
    no_prelude: Option<Span>,
    /// Whether it was parsed from source with syntax errors.
    syntax: Syntax,
    /// Where each field name written quoted, as a string literal, is, by
    /// where it ends (see [`Ast::written_name`]).
    quoted: Vec<Span>,
    /// The implicit `#[shadowable] import * from prelude`, if a workspace
    /// gave the file one.
    implicit_import: Option<ItemId>,
    /// What the expansion of its types did to the tree.
    expansion: Expansion,
    root: ItemRange,
    names: Names,
    strings: Strings,
}

/// What the expansion of a tree's types did to it ([`Ast::expand_types`]).
#[derive(Clone, Debug, Default)]
struct Expansion {
    /// The number of type expressions the parser made, once the tree is
    /// expanded: every one after them is a copy of a declared type's
    /// definition. `None` until then.
    parsed: Option<u32>,
    /// Each type expression the expansion made into another, with its kind
    /// as written, by id.
    written: Vec<(TypeExprId, TypeExprKind)>,
    /// The problems it found, by id.
    problems: Vec<(TypeExprId, TypeProblem)>,
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
        self.types.get(id)
    }

    /// The kind the type expression `id` is written as: its kind, unless
    /// the expansion of the tree's types made it into another ([`Ast::
    /// expand_types`]), as an alias's name into its definition, which the
    /// printer does not show.
    #[must_use]
    pub fn written_type(&self, id: TypeExprId) -> TypeExprKind {
        self.rewritten(id).unwrap_or_else(|| self.types.kind(id))
    }

    /// The kind the type expression `id` is written as, if the expansion of
    /// the tree's types made it into another.
    fn rewritten(&self, id: TypeExprId) -> Option<TypeExprKind> {
        let written = &self.expansion.written;
        let at = written.binary_search_by_key(&id.0, |&(at, _)| at.0).ok()?;
        written.get(at).map(|&(_, kind)| kind)
    }

    /// Whether the type expression `id` is a node of a copy of a declared
    /// type's definition that the expansion of the tree's types added
    /// ([`Ast::expand_types`]), one that no parse made: never changed once
    /// made, and shared by every use of the definition in the tree.
    #[must_use]
    pub fn is_copied(&self, id: TypeExprId) -> bool {
        self.expansion.parsed.is_some_and(|parsed| id.0 >= parsed)
    }

    /// Whether the type expression `id` stands for a declared type's
    /// definition, which the type's own file checks: a copy of one that the
    /// expansion of the tree's types added ([`Ast::is_copied`]), or a name
    /// of a declared type, written bare or qualified, that it made into one
    /// ([`Ast::expand_types`]).
    #[must_use]
    pub fn is_definition_copy(&self, id: TypeExprId) -> bool {
        let Some(parsed) = self.expansion.parsed else {
            return false;
        };
        id.0 >= parsed
            || matches!(
                self.rewritten(id),
                Some(
                    TypeExprKind::Named(_)
                        | TypeExprKind::Path(_)
                        | TypeExprKind::Applied { .. }
                        | TypeExprKind::PathApplied { .. }
                )
            )
    }

    /// The problems the expansion of the tree's types found with the type
    /// expression `id`, in source order, for the checker to report where it
    /// resolves it.
    pub fn type_problems(&self, id: TypeExprId) -> impl Iterator<Item = &TypeProblem> {
        let problems = &self.expansion.problems;
        let start = problems.partition_point(|&(at, _)| at.0 < id.0);
        problems
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(move |&&(at, _)| at == id)
            .map(|(_, problem)| problem)
    }

    /// The ids of the type expressions the parser made, in order, children
    /// first: not the copies the expansion of the tree's types adds.
    pub fn type_ids(&self) -> impl Iterator<Item = TypeExprId> + use<> {
        let parsed = self
            .expansion
            .parsed
            .unwrap_or_else(|| next_id(self.types.len()));
        (0..parsed).map(TypeExprId)
    }

    /// Every expression node of the tree, in the order allocated, for a walk
    /// that needs each node once and not the tree's shape: the formatter's
    /// ([`crate::fmt`]).
    pub(crate) fn expr_arena(&self) -> &[Expr] {
        &self.exprs
    }

    /// Every item of the tree, in the order allocated, as
    /// [`Ast::expr_arena`] gives the expressions.
    pub(crate) fn item_arena(&self) -> &[Item] {
        &self.items
    }

    /// The type expression `id` as it is written in source, as the printer
    /// shows it ([`Ast::pretty`]), its names shown with `names`: the tree's
    /// own table, or, when the tree's is lent out, the table it is lent to.
    #[must_use]
    pub fn show_type_with(&self, id: TypeExprId, names: &Names) -> String {
        /// The type, displayed.
        struct Shown<'a>(WrittenType<'a>, TypeExprId);
        impl fmt::Display for Shown<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.type_expr(f, self.1)
            }
        }
        Shown(WrittenType { ast: self, names }, id).to_string()
    }

    /// The item numbered `number` of the tree, if it has one.
    #[must_use]
    pub fn item_at(&self, number: u32) -> Option<ItemId> {
        (number < next_id(self.item_count())).then_some(ItemId(number))
    }

    /// Returns the fields of the record type expression `range`, in source
    /// order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn field_tys(&self, range: FieldTyRange) -> &[FieldTy] {
        self.types.fields(range)
    }

    /// The constant that the default [`FieldDefault::Copied`] `at` of a
    /// copy of a declaration's definition is (see [Types](#types)): `None`
    /// if it is written as something that is not a constant, which its own
    /// file's check reports.
    #[must_use]
    pub fn copied_default(&self, at: u32) -> Option<&Constant> {
        self.types.constants.get(at as usize)?.as_ref()
    }

    /// The first of the defaults written in the type expression `id` that
    /// the parser made, at any depth, in the order it parsed them: the one
    /// of the least id, since each is a subtree of its own, allocated where
    /// it is written. `None` if it has none. Recurses once per level of
    /// nesting in the type.
    fn first_default(&self, id: TypeExprId) -> Option<ExprId> {
        let mut first: Option<ExprId> = None;
        let mut take = |found: Option<ExprId>| {
            if let Some(found) = found
                && first.is_none_or(|first| found.0 < first.0)
            {
                first = Some(found);
            }
        };
        match self.types.kind(id) {
            TypeExprKind::Record(range) => {
                for field in self.types.fields(range) {
                    take(self.first_default(field.ty));
                    if let Some(FieldDefault::Expr(default)) = field.default {
                        take(Some(default));
                    }
                }
            }
            TypeExprKind::Fn {
                params, ret, meta, ..
            } => {
                for param in self.types.params(params) {
                    take(self.first_default(param.ty));
                }
                take(self.first_default(ret));
                if let Some(meta) = meta {
                    take(self.first_default(meta.ty));
                }
            }
            TypeExprKind::Applied { args, .. }
            | TypeExprKind::PathApplied { args, .. }
            | TypeExprKind::Instance { args, .. } => {
                for &arg in self.types.args(args) {
                    take(self.first_default(arg));
                }
            }
            TypeExprKind::List(element) => {
                take(self.first_default(element));
            }
            TypeExprKind::Union { lhs, rhs, .. } => {
                take(self.first_default(lhs));
                take(self.first_default(rhs));
            }
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::Ref(_)
            | TypeExprKind::Param(_)
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => {}
        }
        first
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

    /// The bindings of a `with` (see the parser's
    /// [Context](crate::parser#context)).
    #[must_use]
    pub fn with_bindings(&self, range: WithRange) -> &[WithBinding] {
        let start = range.start as usize;
        &self.with_bindings[start..start + range.len as usize]
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
        self.types.params(range)
    }

    /// Returns the type arguments of the applied type whose arguments are
    /// `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics if `range` does not belong to this tree.
    #[must_use]
    pub fn type_args(&self, range: TypeArgRange) -> &[TypeExprId] {
        self.types.args(range)
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
            definition: false,
        }
    }

    /// Shows `segments`, a module path of an import, as it is written: its
    /// segments joined by `/`, each as its source spells it, dots and all,
    /// as in `pkgs/net.busybox.busybox` (see the parser's
    /// [Imports](crate::parser#imports)).
    #[must_use]
    pub fn show_module_path(&self, segments: &[PathSegment]) -> String {
        segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| self.names.get(name))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Shows `segments`, the path of a definition or a reference made with a
    /// def-type, `PATH`, or a prefix of one, as it is written: its names
    /// joined by `.`, each as its source spells it, `+` and all, as in
    /// `org.gnu.gmp-c++`, and digits and all, as in
    /// `org.golang.go-stage0-1.24` (see [`PathDisplay`]).
    #[must_use]
    pub fn show_definition_path<'a>(&'a self, segments: &'a [PathSegment]) -> PathDisplay<'a> {
        PathDisplay {
            definition: true,
            ..self.show_path(segments)
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

    /// Whether the `type` item `id` is marked `#[attribute]`: a declaration
    /// of an attribute (see the parser's
    /// [Attributes](crate::parser#attributes)).
    #[must_use]
    pub fn is_attribute(&self, id: ItemId) -> bool {
        self.attribute_marks
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
    }

    /// The uses of attributes that are none of the parser's on the item
    /// `id`, in source order (see the parser's
    /// [Attributes](crate::parser#attributes)).
    #[must_use]
    pub fn attribute_uses(&self, id: ItemId) -> &[AttributeUse] {
        let start = self
            .attribute_uses
            .partition_point(|used| used.item.0 < id.0);
        let end = self
            .attribute_uses
            .partition_point(|used| used.item.0 <= id.0);
        self.attribute_uses.get(start..end).unwrap_or_default()
    }

    /// The methods of the file's outermost block, in source order.
    #[must_use]
    pub fn methods(&self) -> &[MethodDecl] {
        &self.methods
    }

    /// Every call of a field, `x.NAME(...)`, of the tree, by the name and
    /// where the callee is written, in id order: the calls that may be of a
    /// method (see the checker's "Methods").
    pub fn field_calls(&self) -> impl Iterator<Item = (Name, Span)> + '_ {
        self.exprs.iter().filter_map(|expr| match expr.kind {
            ExprKind::Call { callee, .. } | ExprKind::Unclosed { callee, .. } => {
                match self.get(callee).kind {
                    ExprKind::Field { name, .. } => Some((name, self.get(callee).span)),
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// The method whose item is `item`, if it is one.
    #[must_use]
    pub fn method(&self, item: ItemId) -> Option<&MethodDecl> {
        self.methods.iter().find(|method| method.item == item)
    }

    /// The contexts the file declares, in source order.
    #[must_use]
    pub fn contexts(&self) -> &[ContextDecl] {
        &self.contexts
    }

    /// Every read of a context in the tree, `^NAME`, a dotted key's of a
    /// `with` and a `^NAME` parameter's default among them, by the name and
    /// the read's span, in the order the parser made them.
    #[must_use]
    pub fn context_reads(&self) -> Vec<(Name, Span)> {
        self.exprs
            .iter()
            .filter_map(|expr| match expr.kind {
                ExprKind::ContextRead(name) => Some((name, expr.span)),
                _ => None,
            })
            .collect()
    }

    /// The type parameters of every generic function and `type` item, in the
    /// order the parser finished them.
    #[must_use]
    pub fn all_generics(&self) -> &[Generics] {
        &self.generics
    }

    /// The type parameters of `owner`, if it is generic.
    #[must_use]
    pub fn generics_of(&self, owner: GenericOwner) -> Option<&Generics> {
        self.generics
            .iter()
            .find(|generics| generics.owner == owner)
    }

    /// The type parameters of `function`, if it is a generic function item.
    #[must_use]
    pub fn generics(&self, function: FnId) -> Option<&Generics> {
        self.generics_of(GenericOwner::Fn(function))
    }

    /// How many type parameters the `type` item `item` has: none unless it
    /// is generic.
    #[must_use]
    pub fn type_arity(&self, item: ItemId) -> u32 {
        self.generics_of(GenericOwner::Type(item))
            .map_or(0, |generics| next_id(generics.params.len()))
    }

    /// What has the type parameter that the name `name`, written as the type
    /// expression `id`, is, and its number, if one is in scope there: that
    /// of the innermost generic function whose signature or body `id` is
    /// part of, or the generic `type` item whose definition it is part of,
    /// that has a type parameter of the name.
    #[must_use]
    pub fn type_param(&self, id: TypeExprId, name: Name) -> Option<(GenericOwner, u8)> {
        self.generics
            .iter()
            .filter(|generics| generics.types.0 <= id.0 && id.0 < generics.types.1)
            .filter_map(|generics| {
                let index = generics
                    .params
                    .iter()
                    .position(|&(param, _)| param == name)?;
                Some((generics, u8::try_from(index).ok()?))
            })
            .min_by_key(|(generics, _)| generics.types.1 - generics.types.0)
            .map(|(generics, index)| (generics.owner, index))
    }

    /// Every use of an attribute that is none of the parser's, by the items
    /// they apply to, in id order.
    #[must_use]
    pub fn all_attribute_uses(&self) -> &[AttributeUse] {
        &self.attribute_uses
    }

    /// Whether the item `id` is marked `#[def_type]` (see
    /// [Attributes](#attributes)).
    #[must_use]
    pub fn is_def_type(&self, id: ItemId) -> bool {
        self.def_type_marks
            .binary_search_by_key(&id.0, |marked| marked.0)
            .is_ok()
    }

    /// The details of the def-type definition or reference `id` (see
    /// [Def-types](#def-types)).
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this tree.
    #[must_use]
    pub fn def_type(&self, id: DefTypeId) -> &DefTypeUse {
        &self.def_types[id.index()]
    }

    /// Every def-type definition and reference of the tree, with its id, in
    /// the order they were parsed.
    pub fn def_types(&self) -> impl Iterator<Item = (DefTypeId, &DefTypeUse)> + '_ {
        (0..next_id(self.def_types.len()))
            .map(DefTypeId)
            .zip(&self.def_types)
    }

    /// The names of the def-type, `DT`, of the definition or reference
    /// `id`, as it is written: a name, then the name of each field access
    /// on it.
    #[must_use]
    pub fn def_type_names(&self, id: DefTypeId) -> Vec<Name> {
        let mut names = Vec::new();
        let mut at = self.def_type(id).dt;
        loop {
            match self.get(at).kind {
                ExprKind::Field { expr, name } => {
                    names.push(name);
                    at = expr;
                }
                ExprKind::Name(name) => {
                    names.push(name);
                    break;
                }
                _ => break,
            }
        }
        names.reverse();
        names
    }

    /// The def-type, `DT`, of the definition or reference `id`, with its
    /// variant if one is written, and its path, `PATH`, each as it is
    /// written, as in `t.package`, `Toolchain::C` and `dev.peios.testpkg`
    /// ([`Ast::show_definition_path`]).
    #[must_use]
    pub fn show_def_type(&self, id: DefTypeId) -> (String, String) {
        let names: Vec<&str> = self
            .def_type_names(id)
            .into_iter()
            .map(|name| self.names.get(name))
            .collect();
        let mut shown = names.join(".");
        if let Some((variant, _)) = self.def_type(id).variant {
            shown.push_str("::");
            shown.push_str(self.names.get(variant));
        }
        let path = self
            .show_definition_path(self.path(self.def_type(id).path))
            .to_string();
        (shown, path)
    }

    /// Gives the def-type definition, reference or meta `id` `key`, the key
    /// of its def-type's namespace, which the workspace found its `DT` names
    /// (see [Def-types](#def-types)), and, for a definition or a meta,
    /// `meta`, the key of the namespace of the metas of the def-type's
    /// definitions in the directory of the tree's file and PATH as one name:
    /// a reference's read of the definition is then of the namespace, its
    /// name made the key, and a definition's read of its meta of the
    /// namespace of the metas, its name made that key and its field access
    /// PATH's one name.
    pub fn set_def_type_key(&mut self, id: DefTypeId, key: Name, meta: Option<(Name, Name)>) {
        let Some(entry) = self.def_types.get_mut(id.index()) else {
            return;
        };
        entry.key = Some(key);
        if entry.role != DefTypeRole::Reference {
            entry.meta = meta;
        }
        let (read, meta_read) = (entry.read, entry.meta_read);
        let mut at = read;
        while let Some(node) = at.and_then(|node| self.exprs.get_mut(node.index())) {
            match node.kind {
                ExprKind::Field { expr, .. } => at = Some(expr),
                ExprKind::Name(_) => {
                    node.kind = ExprKind::Name(key);
                    break;
                }
                _ => break,
            }
        }
        // A definition's read of its meta: the leaf's name, then the key's.
        if let (Some(read), Some((meta_key, leaf))) = (meta_read, meta)
            && let Some(node) = self.exprs.get_mut(read.index())
            && let ExprKind::Field { expr, .. } = node.kind
        {
            node.kind = ExprKind::Field { expr, name: leaf };
            if let Some(base) = self.exprs.get_mut(expr.index()) {
                base.kind = ExprKind::Name(meta_key);
            }
        }
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

    /// Whether the tree was parsed from source with syntax errors, and so
    /// how far its shape can be trusted: [`Syntax::Clean`] for every tree
    /// that is run.
    #[must_use]
    pub fn syntax(&self) -> Syntax {
        self.syntax
    }

    /// Where the name whose text is `text`, written last in a span that
    /// ends at `end`, is written: the string literal of a quoted field name
    /// that ends there, quotes and all, if there is one, else the `text`'s
    /// length before `end`, an identifier written as its text. A field
    /// access spans its record through its name, so this is the name of
    /// `r.x` and of `r."x"`.
    #[must_use]
    pub fn written_name(&self, end: u32, text: &str) -> Span {
        let at = self.quoted.partition_point(|span| span.end < end);
        match self.quoted.get(at) {
            Some(&quoted) if quoted.end == end => quoted,
            _ => {
                let length = u32::try_from(text.len()).unwrap_or(u32::MAX);
                Span::new(end.saturating_sub(length), end)
            }
        }
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

    /// Expands the tree's types, once (see [Types](#types)). Every type
    /// expression the parser made is taken in order, children first, as it
    /// made them:
    ///
    /// - one that `links` (by id, in order) has a declaration for, a name
    ///   of a declared type, written bare or qualified, becomes what that
    ///   declaration's settled definition in `store` is, its nodes copied
    ///   into the tree's arena after the parsed ones, each once, shared by
    ///   every use; a generic one's, applied to its arguments, with them in
    ///   place of its parameters, the nodes above a parameter made for the
    ///   use (`TypeArena::substitute`); or [`TypeExprKind::Invalid`] if
    ///   the declaration is erroneous, which its own file reports;
    /// - one that `links` has an error for becomes `Invalid`, with the
    ///   problem [`TypeProblem::Unresolved`];
    /// - a record type with spreads becomes the record type of its merged
    ///   fields (see [`TypeArena::merge_record`]), with its problems;
    /// - one that would then be nested more than [`MAX_DEPTH`] levels deep
    ///   becomes `Invalid`, with the problem "expression nested too
    ///   deeply" at it.
    ///
    /// The kind each was written as is kept for the printer
    /// ([`Ast::written_type`]). A tree with no link and no spread is only
    /// noted as expanded.
    pub fn expand_types(&mut self, store: &TypeStore, links: &[(TypeExprId, TypeLink)]) {
        if self.expansion.parsed.is_some() {
            return;
        }
        let parsed = self.types.len();
        self.expansion.parsed = Some(next_id(parsed));
        if links.is_empty()
            && self
                .types
                .fields
                .iter()
                .all(|field| field.kind != FieldTyKind::Spread)
        {
            return;
        }
        // The height of each node, by id: the parsed ones' as they are
        // taken, the copies' as they are made.
        let mut heights: Vec<u32> = vec![1; parsed];
        let mut memo = HashMap::new();
        let mut written = Vec::new();
        let mut problems = Vec::new();
        for id in self.types.ids_from(0).take(parsed) {
            let kind = self.types.kind(id);
            let link = links
                .binary_search_by_key(&id.0, |&(at, _)| at.0)
                .ok()
                .and_then(|at| links.get(at))
                .map(|(_, link)| link);
            match link {
                Some(TypeLink::Decl(decl)) => {
                    let definition = store
                        .root(*decl)
                        .filter(|&root| store.arena.kind(root) != TypeExprKind::Invalid);
                    let expanded = match definition {
                        Some(root) => {
                            let start = self.types.len();
                            // A store holds no expression of a tree.
                            let mut copy = self.types.copy(
                                &store.arena,
                                root,
                                &mut memo,
                                (&|_| None, &|_| None),
                            );
                            // A generic type's use, its arity checked, has
                            // its arguments in place of its parameters.
                            if let TypeExprKind::Applied { args, .. }
                            | TypeExprKind::PathApplied { args, .. } = kind
                            {
                                let args = self.types.args(args).to_vec();
                                copy = self.types.substitute(copy, &args, &mut HashMap::new());
                            }
                            // The copies are children first, so each one's
                            // height follows from its children's.
                            for new in self.types.ids_from(start) {
                                let height = self.types.height(new, &heights);
                                heights.push(height);
                            }
                            self.types.kind(copy)
                        }
                        None => TypeExprKind::Invalid,
                    };
                    self.types.set_kind(id, expanded);
                    written.push((id, kind));
                }
                Some(TypeLink::Error(error)) => {
                    self.types.set_kind(id, TypeExprKind::Invalid);
                    written.push((id, kind));
                    problems.push((id, TypeProblem::Unresolved(error.clone())));
                }
                None => {
                    let found = self.types.merge_record(id);
                    if self.types.kind(id) != kind {
                        written.push((id, kind));
                    }
                    problems.extend(found.into_iter().map(|problem| (id, problem)));
                }
            }
            let mut height = self.types.height(id, &heights);
            if height as usize > MAX_DEPTH {
                let span = self.types.get(id).span;
                if written.last().map(|&(at, _)| at) != Some(id) {
                    written.push((id, kind));
                }
                self.types.set_kind(id, TypeExprKind::Invalid);
                problems.push((
                    id,
                    TypeProblem::Unresolved(Diagnostic::error(
                        "expression nested too deeply",
                        span,
                    )),
                ));
                height = 1;
            }
            if let Some(entry) = heights.get_mut(id.index()) {
                *entry = height;
            }
        }
        self.expansion.written = written;
        self.expansion.problems = problems;
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

    /// Every expression's id, in allocation order: for tests that look for
    /// nodes of a kind, and for an index of the tree's names
    /// ([`crate::index`]).
    pub(crate) fn expr_ids(&self) -> impl Iterator<Item = ExprId> + use<> {
        (0..next_id(self.exprs.len())).map(ExprId)
    }

    /// Every item's id, in allocation order, for an index of the tree's
    /// names ([`crate::index`]).
    pub(crate) fn item_ids(&self) -> impl Iterator<Item = ItemId> + use<> {
        (0..next_id(self.items.len())).map(ItemId)
    }

    /// The ids of the nodes of expression `id`'s subtree, its descendants
    /// and itself, in allocation order. They are contiguous, ending at `id`:
    /// nodes are allocated children-first, and while the parser parses an
    /// expression it allocates only that expression's nodes (a path is read
    /// from its tokens, allocating only the positions of its index
    /// segments, which belong to its item).
    /// The first is found by following, from `id`, the child the parser
    /// parses first down to a leaf: a block's first item's value, or the
    /// position of the first index segment of its path if it has one, or
    /// the first field default in its annotation if it has one, an
    /// `if`'s condition, a call's callee, a binary operator's left operand,
    /// a function's first parameter default, or field default in a
    /// parameter's or its return type, if it has one (a default is
    /// parsed before the body) and its body otherwise, a `lazy`'s body, an
    /// interpolated string's first expression, a list literal's first
    /// element's, an index's list, an
    /// iteration's iterable, a condition loop's first header item's, its
    /// condition or its body, whichever comes first, a `do` loop's body,
    /// and a def-type reference's `DT`, its read of the definition coming
    /// between that and itself. (A block's first item may hold no
    /// expression, a `break` without a value: the first item that holds one
    /// is followed.) Debug builds check that every node of the range lies
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
                | ExprKind::Duration(_)
                | ExprKind::Size(_)
                | ExprKind::Bool(_)
                | ExprKind::Str(_)
                | ExprKind::Name(_)
                | ExprKind::Variant { .. }
                | ExprKind::ContextRead(_)
                | ExprKind::Error => return node,
                // The body before the bindings after it, `f() with {...}`,
                // and the bindings before it, `with {...} {...}`: the
                // earliest of its parts.
                ExprKind::With { bindings, body } => self
                    .with_bindings(bindings)
                    .iter()
                    .flat_map(|binding| [binding.current, Some(binding.value)])
                    .flatten()
                    .chain([body])
                    .min_by_key(|part| part.0)
                    .unwrap_or(body),
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
                | ExprKind::As { expr: inner, .. }
                | ExprKind::NoField(inner) => inner,
                // A parameter's default, and a field's default in a
                // parameter's or the return type, is parsed before the body.
                ExprKind::Fn(function) => {
                    let decl = self.fn_decl(function);
                    self.params(decl.params)
                        .iter()
                        .flat_map(|param| [self.first_default(param.ty), param.default])
                        .chain([decl.ret.and_then(|ret| self.first_default(ret))])
                        .flatten()
                        .min_by_key(|default| default.0)
                        .unwrap_or(decl.body)
                }
                ExprKind::Lazy(function) => self.fn_decl(function).body,
                ExprKind::DefTypeRef(def) => self.def_type(def).dt,
                ExprKind::Call { callee, .. } | ExprKind::Unclosed { callee, .. } => callee,
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
                // A field's default in the annotation is parsed before the
                // value.
                ItemKind::Let {
                    ty: Some(ty),
                    value,
                    ..
                }
                | ItemKind::Field {
                    ty: Some(ty),
                    value,
                    ..
                } => Some(self.first_default(ty).unwrap_or(value)),
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

    /// Writes the annotation for the `type` item `item`, after its type as
    /// written, including any leading space: by default, nothing.
    ///
    /// # Errors
    ///
    /// Propagates errors from the formatter.
    fn annotate_type(&self, item: ItemId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (item, f);
        Ok(())
    }

    /// If the def-type reference `id`, `DT PATH`, was found to have a path
    /// that goes on past the definition it names, as `package
    /// dev.peios.testpkg.versions` reads the field `versions` of the meta of
    /// `package dev.peios.testpkg` (see the checker's "Metas"): the prefix
    /// of its read ([`DefTypeUse::read`]) that reads the definition. The
    /// reference is then printed as the reference to the definition with a
    /// field access around it for each segment past it. By default, `None`:
    /// every reference as it is parsed.
    fn reference_past(&self, id: ExprId) -> Option<ExprId> {
        let _ = id;
        None
    }

    /// Writes the annotation, including any leading space, of the reference
    /// to the definition that the path of the def-type reference `id` goes
    /// on past ([`Annotate::reference_past`]): by default, nothing.
    ///
    /// # Errors
    ///
    /// Propagates errors from the formatter.
    fn annotate_reference(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (id, f);
        Ok(())
    }
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
/// `(let mut x : Type ...)`, `(field x ...)`, `(field x : Type ...)`,
/// `(rebind x ...)`, `(spread ...)`, `(result ...)` and `(expr ...)`,
/// followed by their span and their expression, a dotted path in full, as
/// in `(let a.b.c ...)`, and an index segment as it is written, as in
/// `(rebind xs[i + 1].up ...)`, its position then shown before the value as
/// `(at EXPR)`, one for each index segment, in order; blocks as
/// `(block ...)` followed by their items; ascriptions as `(: Type ...)`;
/// field accesses as `(. x ...)` followed by their record expression. A
/// field name, or a path segment, that is not a plain identifier is quoted,
/// as in `(field "a b" ...)`. Types print as written, whatever the
/// expansion of the tree's types made of them ([`Ast::written_type`]), a
/// record type's fields and spreads in source order: `{x: Int, "a b": {}}`,
/// `{...PackageDef, name: String}`, `fn(x: Int) -> String`, and a qualified
/// name as its path, `t.PackageDef`. A `type` item prints as `(type NAME
/// TYPE @0..20)`, with the annotation's [`Annotate::annotate_type`] after
/// its type, which `--dump-types` makes its expansion. A function
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
/// a union, `(fn(): Int)[]`; a function type's meta prints after its return
/// type as written, `fn(): Int META-> {a: Int}`, the return type in
/// parentheses if it is a function type; other parentheses in a type
/// leave no trace in the tree and are not printed.
/// Booleans print as `(bool true)`, string literals as `(str "text")` with
/// the text quoted and escaped, and interpolated strings as `(interp ...)`
/// followed by their parts, literal text as `(lit "text")` (which has no
/// span) and expressions as their nodes. `(if ...)` is followed by its
/// condition, its block and its else branch, which for an `if` without
/// `else` is the parser's empty block. Operators print as their symbol,
/// except negation, `(neg ...)`. A def-type definition prints as
/// `(def-type-def package dev.peios.testpkg ...)`, its def-type and its path
/// as written, followed by its function, whose parameters are their names
/// alone, as in `(params a b)`, with no `(ret ...)`; a reference to one as
/// `(def-type-ref package dev.peios.testpkg @0..30)`, with nothing under it,
/// its whole path as written, unless the annotation says the path goes on
/// past the definition ([`Annotate::reference_past`]): then as a field access
/// for each segment past it, `(. versions ...)`, around the reference to the
/// definition, `(def-type-ref package dev.peios.testpkg ...)`;
/// a definition's meta as `(defmeta package dev.peios.testpkg ...)`,
/// followed by its function, of no parameters, with no `(ret ...)`, whose
/// body is the block written, or for `defmeta DT PATH = EXPR` a block whose
/// result is the expression.
/// The file has no span. An item marked `#[shadowable]`, `#[def_type]` or
/// `#[attribute]` is preceded by the attribute, as in `#[shadowable] (field
/// arch ...)` or `#[def_type] (def package ...)`, several in that order,
/// then each use of an attribute on it, in source order, its arguments, if
/// it has any, shown as `(…)`, as in `#[retry(…)] (fn f ...)`; and a file marked `#![no_prelude]` has it after `file`, as in
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
        let path = ast.show_module_path(ast.path(decl.path));
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

    /// Writes the attributes of the item `id`, each followed by a space: the
    /// parser's, then the uses of others, in source order.
    fn attributes(&self, f: &mut fmt::Formatter<'_>, id: ItemId) -> fmt::Result {
        let ast = self.ast;
        let marks = [
            (ast.is_shadowable(id), "#[shadowable] "),
            (ast.is_def_type(id), "#[def_type] "),
            (ast.is_attribute(id), "#[attribute] "),
        ];
        for (marked, text) in marks {
            if marked {
                f.write_str(text)?;
            }
        }
        for used in ast.attribute_uses(id) {
            let name = ast.names().get(used.name);
            let args = if used.args.is_some() { "(…)" } else { "" };
            write!(f, "#[{name}{args}] ")?;
        }
        Ok(())
    }

    /// Writes the head of the item `id`, a `type`, an `enum` or a variant.
    fn type_item(&self, f: &mut fmt::Formatter<'_>, id: ItemId) -> fmt::Result {
        let ast = self.ast;
        match ast.item(id).kind {
            ItemKind::Type { name, ty, .. } => {
                write!(f, "type {} ", ast.names().get(name))?;
                self.type_expr(f, ty)?;
                self.annotation.annotate_type(id, f)
            }
            ItemKind::Enum { name, bound, .. } => {
                write!(f, "enum {} extensible", ast.names().get(name))?;
                match bound {
                    Some(bound) => {
                        f.write_str(" ")?;
                        self.type_expr(f, bound)
                    }
                    None => Ok(()),
                }
            }
            ItemKind::Variant {
                of, name, payload, ..
            } => {
                f.write_str("enum variant ")?;
                self.type_expr(f, of)?;
                write!(f, "::{}", ast.names().get(name))?;
                match payload {
                    Some(payload) => {
                        f.write_str(" ")?;
                        self.type_expr(f, payload)
                    }
                    None => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    /// Writes the head of the `let` item `id` of `path`, a `let mut` if
    /// `mutable`, a `fn` if `function`: a method and a context's default
    /// shown by their names, not their bindings'.
    fn let_head(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: ItemId,
        (path, mutable, function): (PathRange, bool, bool),
    ) -> fmt::Result {
        let ast = self.ast;
        let keyword = match (mutable, function) {
            (_, true) => "fn",
            (true, false) => "let mut",
            (false, false) => "let",
        };
        let context = ast.contexts().iter().find(|context| context.item == id);
        match (ast.method(id), context) {
            (Some(method), _) => {
                let keyword = if method.public { "def" } else { "fn" };
                write!(f, "{keyword} method {}", ast.names().get(method.name))
            }
            (None, Some(context)) => {
                self.context_head(f, context.name, context.ty)?;
                f.write_str(" =")
            }
            (None, None) => write!(f, "{keyword} {}", ast.show_path(ast.path(path))),
        }
    }

    /// Writes a context's declaration, of `name` and the type `ty`, but its
    /// default: `context arch : String`.
    fn context_head(&self, f: &mut fmt::Formatter<'_>, name: Name, ty: TypeExprId) -> fmt::Result {
        write!(f, "context {} : ", self.ast.names().get(name))?;
        self.type_expr(f, ty)
    }

    fn item(&self, f: &mut fmt::Formatter<'_>, id: ItemId, depth: usize) -> fmt::Result {
        let item = self.ast.item(id);
        let ast = self.ast;
        write!(f, "{:indent$}", "", indent = depth * 2)?;
        self.attributes(f, id)?;
        f.write_str("(")?;
        let value = match item.kind {
            ItemKind::Let {
                path,
                ty,
                value,
                mutable,
                function,
            } => {
                self.let_head(f, id, (path, mutable, function))?;
                if let Some(ty) = ty {
                    f.write_str(" : ")?;
                    self.type_expr(f, ty)?;
                }
                value
            }
            ItemKind::Field {
                path,
                ty,
                value,
                function,
            } => {
                let keyword = if function { "def" } else { "field" };
                write!(f, "{keyword} {}", ast.show_path(ast.path(path)))?;
                if let Some(ty) = ty {
                    f.write_str(" : ")?;
                    self.type_expr(f, ty)?;
                }
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
            ItemKind::Type { .. } | ItemKind::Enum { .. } | ItemKind::Variant { .. } => {
                self.type_item(f, id)?;
                return write!(f, " @{})", item.span);
            }
            ItemKind::Context { name, ty, .. } => {
                self.context_head(f, name, ty)?;
                return write!(f, " @{})", item.span);
            }
            ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } => {
                self.def_type_head(f, item.kind, def)?;
                value
            }
            ItemKind::Error => return write!(f, "error @{})", item.span),
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

    /// Writes what an item of `kind`, a definition made with a def-type or a
    /// definition's meta, whose use of its def-type is `def`, shows before
    /// its span: `def-type-def` or `defmeta`, then its def-type and its path,
    /// each as written.
    fn def_type_head(
        &self,
        f: &mut fmt::Formatter<'_>,
        kind: ItemKind,
        def: DefTypeId,
    ) -> fmt::Result {
        let (def_type, path) = self.ast.show_def_type(def);
        let keyword = match kind {
            ItemKind::DefMeta { .. } => "defmeta",
            _ => "def-type-def",
        };
        write!(f, "{keyword} {def_type} {path}")
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

    /// Writes a type expression as it is written in source: as the parser
    /// made it, whatever the expansion of the tree's types made of it
    /// ([`Ast::written_type`]).
    fn type_expr(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        WrittenType {
            ast: self.ast,
            names: self.ast.names(),
        }
        .type_expr(f, id)
    }
}

/// The type expressions of a tree as they are written in source, their
/// names shown with a table that need not be the tree's own: the printer's
/// ([`Pretty`]), and [`Ast::show_type_with`]'s.
struct WrittenType<'a> {
    ast: &'a Ast,
    names: &'a Names,
}

impl WrittenType<'_> {
    /// Writes the qualified type name `path` as it is written.
    fn path(&self, f: &mut fmt::Formatter<'_>, path: PathRange) -> fmt::Result {
        let shown = PathDisplay {
            names: self.names,
            strings: &self.ast.strings,
            segments: self.ast.path(path),
            definition: false,
        };
        write!(f, "{shown}")
    }

    /// Writes the type arguments `args` as they are written, `<A, B>`.
    fn type_args(&self, f: &mut fmt::Formatter<'_>, args: TypeArgRange) -> fmt::Result {
        f.write_str("<")?;
        for (index, &arg) in self.ast.type_args(args).iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            self.type_expr(f, arg)?;
        }
        f.write_str(">")
    }

    /// Writes a type expression as it is written in source: as the parser
    /// made it, whatever the expansion of the tree's types made of it
    /// ([`Ast::written_type`]).
    fn type_expr(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        let names = self.names;
        match self.ast.written_type(id) {
            // An enum, a declaration's definition, is written by its name.
            TypeExprKind::Named(name) | TypeExprKind::Enum { name, .. } => {
                f.write_str(names.get(name))
            }
            TypeExprKind::Context(name) => write!(f, "^{}", self.names.get(name)),
            TypeExprKind::Path(path) => self.path(f, path),
            TypeExprKind::Record(fields) => {
                f.write_str("{")?;
                for (index, field) in self.ast.field_tys(fields).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    match field.kind {
                        FieldTyKind::Field(name) => write!(f, "{}: ", FieldName(names.get(name)))?,
                        FieldTyKind::Spread => f.write_str("...")?,
                        FieldTyKind::Rest => f.write_str("*: ")?,
                    }
                    self.type_expr(f, field.ty)?;
                    match field.default {
                        Some(FieldDefault::Expr(default)) => {
                            f.write_str(" = ")?;
                            self.default_expr(f, default)?;
                        }
                        Some(FieldDefault::Copied(at)) => {
                            f.write_str(" = ")?;
                            match self.ast.copied_default(at) {
                                Some(constant) => write_constant(f, constant, names)?,
                                None => f.write_str("...")?,
                            }
                        }
                        None => {}
                    }
                }
                f.write_str("}")
            }
            TypeExprKind::Fn {
                params,
                ret,
                effect,
                meta,
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
                let Some(meta) = meta else {
                    return self.type_expr(f, ret);
                };
                // A return type that is a function type would take the meta.
                self.union_member(f, ret)?;
                write!(f, " {} ", meta.effect.meta_marker())?;
                self.type_expr(f, meta.ty)
            }
            TypeExprKind::Applied { name, args, .. } => {
                f.write_str(names.get(name))?;
                self.type_args(f, args)
            }
            TypeExprKind::PathApplied { path, args } => {
                self.path(f, path)?;
                self.type_args(f, args)
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
                    self.ast.written_type(element),
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
            // Never written: the store's references and the expansion's
            // errors are kept apart from what a tree prints.
            TypeExprKind::Ref(_)
            | TypeExprKind::Instance { .. }
            | TypeExprKind::Param(_)
            | TypeExprKind::Invalid => f.write_str("<invalid>"),
        }
    }

    /// Writes the type expression `id`, a member of a union, in parentheses
    /// if it is a function type, whose return type would otherwise take
    /// what follows.
    fn union_member(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        if matches!(self.ast.written_type(id), TypeExprKind::Fn { .. }) {
            f.write_str("(")?;
            self.type_expr(f, id)?;
            f.write_str(")")
        } else {
            self.type_expr(f, id)
        }
    }

    /// Writes the expression `id`, a field's default as the parser made it,
    /// on one line, as a type is written: a literal as a value's `repr`
    /// shows it (an integer or a float as written, `true`, `false`, a
    /// string quoted), `-` before a number, a list literal of such in
    /// brackets, `[1, 2]`, and a record block of fields of one name each,
    /// `{a = 1; b = 2}`, `{}` included; and anything else, which is no
    /// constant (see the checker's "Field defaults"), as `...`.
    ///
    /// Recurses once per level of list and record nesting.
    fn default_expr(&self, f: &mut fmt::Formatter<'_>, id: ExprId) -> fmt::Result {
        let ast = self.ast;
        match ast.get(id).kind {
            ExprKind::Int(value) => write!(f, "{value}"),
            ExprKind::Float(value) => write!(f, "{value:?}"),
            ExprKind::Bool(value) => write!(f, "{value}"),
            ExprKind::Str(text) => write!(f, "{}", Quoted(ast.strings.get(text))),
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => {
                f.write_str("-")?;
                self.default_expr(f, operand)
            }
            ExprKind::List(elements) => {
                f.write_str("[")?;
                for (index, element) in ast.elements(elements).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    if element.spread.is_some() {
                        f.write_str("...")?;
                    }
                    self.default_expr(f, element.value)?;
                }
                f.write_str("]")
            }
            ExprKind::Block { items, label: None } => {
                f.write_str("{")?;
                for (index, &item) in ast.items(items).iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    let ItemKind::Field { path, value, .. } = ast.item(item).kind else {
                        f.write_str("...")?;
                        continue;
                    };
                    let name = match ast.path(path) {
                        [field] => field.name(),
                        _ => None,
                    };
                    match name {
                        Some(name) => {
                            write!(f, "{} = ", FieldName(self.names.get(name)))?;
                            self.default_expr(f, value)?;
                        }
                        None => f.write_str("...")?,
                    }
                }
                f.write_str("}")
            }
            _ => f.write_str("..."),
        }
    }
}

impl<A: Annotate> Pretty<'_, A> {
    /// Writes the parameters and return type of the function expression
    /// `id`, each on a line of its own at `depth`, with a leading newline: a
    /// parameter's default, if it has one, is its node, one level deeper,
    /// after its type, `(greeting String` then the node and `)`.
    fn fn_head(&self, f: &mut fmt::Formatter<'_>, id: FnId, depth: usize) -> fmt::Result {
        let decl = self.ast.fn_decl(id);
        write!(f, "\n{:indent$}(params", "", indent = depth * 2)?;
        for param in self.ast.params(decl.params) {
            let name = FieldName(self.ast.names().get(param.name));
            // A def-type definition's parameter is written by name alone,
            // its type the def-type's, and so is its result.
            if self.ast.written_type(param.ty) == TypeExprKind::Invalid {
                write!(f, " {name}")?;
                continue;
            }
            write!(f, " ({name} ")?;
            self.type_expr(f, param.ty)?;
            // A default, on lines of its own after its type.
            if let Some(default) = param.default {
                f.write_str("\n")?;
                self.node(f, default, depth + 1)?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")?;
        if decl.ret.is_none() {
            return Ok(());
        }
        let effect = decl.effect.name();
        write!(f, "\n{:indent$}(ret {effect}", "", indent = depth * 2)?;
        // Written: a definition, which writes none, has returned above.
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
                ArgKind::Named {
                    shorthand: true, ..
                } => {
                    // Its value is the read of its name, which says it all.
                    write!(f, "{:indent$}(shorthand ", "", indent = depth * 2)?;
                    self.node(f, arg.value, 0)?;
                    f.write_str(")")?;
                }
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
            ExprKind::Duration(value) => write!(f, "duration {value}"),
            ExprKind::Size(value) => write!(f, "size {value}"),
            ExprKind::Variant { of, name, .. } => {
                f.write_str("variant ")?;
                WrittenType {
                    ast: self.ast,
                    names: self.ast.names(),
                }
                .type_expr(f, of)?;
                write!(f, "::{}", self.ast.names().get(name))
            }
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
            ExprKind::DefTypeRef(def) => {
                let (def_type, path) = self.ast.show_def_type(def);
                write!(f, "def-type-ref {def_type} {path}")
            }
            ExprKind::ContextRead(name) => write!(f, "context ^{}", self.ast.names().get(name)),
            ExprKind::With { .. } => f.write_str("with"),
            ExprKind::Error => f.write_str("error"),
            ExprKind::NoField(_) => f.write_str(". <missing>"),
            ExprKind::Unclosed { .. } => f.write_str("unclosed call"),
        }
    }

    /// Writes the def-type reference `id`, of `def`, whose path goes on past
    /// the definition its read's prefix `read` names, at `depth`: as a field
    /// access for each segment past the definition, the outermost spanning
    /// the reference and annotated as it is, around the reference to the
    /// definition, its def-type and the path's prefix, annotated with
    /// [`Annotate::annotate_reference`]; each spanning from the reference's
    /// def-type through its last segment.
    fn reference_past(
        &self,
        f: &mut fmt::Formatter<'_>,
        (id, def, read): (ExprId, DefTypeId, ExprId),
        depth: usize,
    ) -> fmt::Result {
        let ast = self.ast;
        let use_ = ast.def_type(def);
        let start = ast.get(use_.dt).span;
        // The accesses past the definition, outermost first.
        let mut accesses = Vec::new();
        let mut at = use_.read.unwrap_or(read);
        while at != read {
            let ExprKind::Field { expr, name } = ast.get(at).kind else {
                break;
            };
            accesses.push((at, name));
            at = expr;
        }
        for (index, &(access, name)) in accesses.iter().enumerate() {
            if index > 0 {
                f.write_str("\n")?;
            }
            let indent = (depth + index) * 2;
            write!(f, "{:indent$}(. {}", "", FieldName(ast.names().get(name)))?;
            if index == 0 {
                self.annotation.annotate(id, f)?;
                write!(f, " @{}", ast.get(id).span)?;
            } else {
                self.annotation.annotate(access, f)?;
                write!(f, " @{}", Span::join(start, ast.get(access).span))?;
            }
        }
        let (def_type, _) = ast.show_def_type(def);
        let segments = ast.path(use_.path);
        let defined = segments.len().saturating_sub(accesses.len());
        let path = ast.show_definition_path(segments.get(..defined).unwrap_or_default());
        if !accesses.is_empty() {
            f.write_str("\n")?;
        }
        write!(
            f,
            "{:indent$}(def-type-ref {def_type} {path}",
            "",
            indent = (depth + accesses.len()) * 2
        )?;
        self.annotation.annotate_reference(id, f)?;
        write!(f, " @{})", Span::join(start, ast.get(read).span))?;
        for _ in &accesses {
            f.write_str(")")?;
        }
        Ok(())
    }

    /// Writes the bindings, `bindings`, and the body, `body`, of a `with`,
    /// each on a line of its own at `depth`: a binding as `(bind NAME`, its
    /// dotted path, then its read of what it updates, if it is dotted, and
    /// its value.
    fn with_parts(
        &self,
        f: &mut fmt::Formatter<'_>,
        (bindings, body): (WithRange, ExprId),
        depth: usize,
    ) -> fmt::Result {
        for binding in self.ast.with_bindings(bindings) {
            let name = self.ast.names().get(binding.name);
            write!(f, "\n{:indent$}(bind {name}", "", indent = depth * 2)?;
            for segment in self.ast.path(binding.path) {
                if let Some(field) = segment.name() {
                    write!(f, ".{}", self.ast.names().get(field))?;
                }
            }
            for part in binding.current.into_iter().chain([binding.value]) {
                f.write_str("\n")?;
                self.node(f, part, depth + 1)?;
            }
            f.write_str(")")?;
        }
        f.write_str("\n")?;
        self.node(f, body, depth)
    }

    fn node(&self, f: &mut fmt::Formatter<'_>, id: ExprId, depth: usize) -> fmt::Result {
        let expr = self.ast.get(id);
        if let ExprKind::DefTypeRef(def) = expr.kind
            && let Some(read) = self.annotation.reference_past(id)
        {
            return self.reference_past(f, (id, def, read), depth);
        }
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
            ExprKind::Call { callee, args } | ExprKind::Unclosed { callee, args } => {
                f.write_str("\n")?;
                self.node(f, callee, depth + 1)?;
                self.args(f, args, depth + 1)?;
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Name(_)
            | ExprKind::Variant { .. }
            | ExprKind::DefTypeRef(_)
            | ExprKind::ContextRead(_)
            | ExprKind::Error => {}
            ExprKind::Unary { operand, .. }
            | ExprKind::Paren(operand)
            | ExprKind::Field { expr: operand, .. }
            | ExprKind::NoField(operand)
            | ExprKind::Ascribe { expr: operand, .. }
            | ExprKind::Is { expr: operand, .. }
            | ExprKind::As { expr: operand, .. } => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            ExprKind::With { bindings, body } => self.with_parts(f, (bindings, body), depth + 1)?,
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
mod tests;
