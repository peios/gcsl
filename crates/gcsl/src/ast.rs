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
//! `#[def_type]` (see the parser's [Attributes](crate::parser::attributes#attributes)),
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

use std::fmt;
use std::num::NonZeroU32;

use crate::check::Constant;
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::{FieldName, StrId, Strings};

pub use typeexprs::{AttributeId, TypeArena, TypeLink, TypeProblem, TypeStore, TypeStoreMark};
use typeexprs::next_id;
pub use builder::{AstBuilder, BuilderMark, ConstantTree, Syntax, is_constant};
pub use tree::Ast;
use tree::Expansion;
pub use pretty::{Annotate, NoAnnotation, Pretty};
use pretty::WrittenType;

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

/// Index of a `recover` expression's details in the recover arena of the
/// [`Ast`] that allocated it (see [`Ast::recover_decl`]); later stages
/// index their per-`recover` tables by it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecoverId(u32);

impl RecoverId {
    /// The id as an index into the arena and into side tables.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A `recover` expression, `recover { BODY } else (NAME) { FALLBACK }` (see
/// the parser's [Failure](crate::parser::failure#failure)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoverDecl {
    /// The body, a block.
    pub body: ExprId,
    /// The name the failure is bound to in the fallback, and where it is
    /// written, if one is.
    pub error: Option<(Name, Span)>,
    /// The fallback, a block.
    pub fallback: ExprId,
}

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
    /// An import of every definition of the def-type, `import DT * from
    /// PATH` (see the parser's [Imports](crate::parser::imports#imports)):
    /// its PATH is empty.
    Import,
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
    /// For a definition with parameters of its own, written with a type and
    /// a default after its def-type's (see the parser's "Def-types"), the
    /// function of its body, which those parameters are of: the
    /// definition's own function calls it with their defaults. `None` for
    /// any other use.
    pub body: Option<FnId>,
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
    /// `DT *`, the import's one member: every definition of the def-type
    /// `DT`, the use `def`, in what the path names, written at the span,
    /// `DT` through `*`.
    DefType {
        /// The def-type's use, of [`DefTypeRole::Import`].
        def: DefTypeId,
        /// Where it is written.
        span: Span,
    },
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
            ImportMember::All(_) | ImportMember::DefType { .. } => None,
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
            ImportMember::All(span) | ImportMember::DefType { span, .. } => span,
        }
    }
}

/// Which directories an import's path names (see the parser's
/// [Imports](crate::parser::imports#imports)): the one it names, every
/// directory directly in it, `pkgs/*`, or every directory at any depth
/// below it, `pkgs/**`. Only an import of a def-type's definitions has a
/// glob.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportGlob {
    /// No glob: what the path names.
    Exact,
    /// `PATH/*`: every directory directly in it.
    Children,
    /// `PATH/**`: every directory at any depth below it.
    Descendants,
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

/// An import item (see the parser's [Imports](crate::parser::imports#imports)):
/// held in the [`Ast`]'s import arena; the item is an [`ItemKind::Import`]
/// with its [`ImportId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImportDecl {
    /// The module path: one or more name segments, absolute from the
    /// workspace root, never an index segment.
    pub path: PathRange,
    /// What the import binds.
    pub form: ImportForm,
    /// Which directories the path names, by its glob, if it has one.
    pub glob: ImportGlob,
    /// Where the path is written, its glob included: `pkgs/*`, or `*`
    /// alone.
    pub path_at: Span,
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
/// shorthand `NAME` (see the parser's [Context](crate::parser::context#context)).
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

/// The needs written in a function type expression, `^arch`: a contiguous
/// range of the need pool of the arena that allocated it. Resolve it with
/// [`Ast::need_tys`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct NeedTyRange {
    start: u32,
    len: u32,
}

/// A context a function type is written to need, `^arch` in `fn(^arch) ->
/// {}`: its name, as the file where it is written reads it, and where it is
/// written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NeedTy {
    /// The context's name.
    pub name: Name,
    /// Where `^NAME` is written.
    pub span: Span,
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
/// (see the parser's [Attributes](crate::parser::attributes#attributes)).
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
    /// any type's name (see [Types](crate::ast::typeexprs#types)).
    pub ty: TypeExprId,
}

/// A method, `fn (RECV: TYPE) NAME(...) ...` or `def (RECV: TYPE) NAME(...)
/// ...` (see the parser's [Methods](crate::parser::functions#methods)): its item is a
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
/// NAME: TYPE` (see the parser's [Context](crate::parser::context#context)): with a
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
/// calls](crate::parser::functions#functions-and-calls)), or a generic `type` item,
/// `type NAME<T> = TYPE` (see [Types](crate::ast::typeexprs#types)).
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
    /// calls](crate::parser::functions#functions-and-calls)).
    pub default_fn: Option<FnId>,
    /// Whether it is the rest parameter, `...name: T`, which the positional
    /// arguments from its position on fill, a `List<T>` in the body (see
    /// the parser's [Functions and calls](crate::parser::functions#functions-and-calls)).
    pub rest: bool,
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
    /// [Types](crate::ast::typeexprs#types)).
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
    /// parser's [Functions and calls](crate::parser::functions#functions-and-calls)).
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
    /// `...EXPR`, a list whose elements fill the callee's rest parameter,
    /// among the positional arguments that fill it one each.
    Spread {
        /// Where its `...` is written.
        span: Span,
    },
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
    /// for a named argument, its `...` through its value for a spread one,
    /// and its value otherwise.
    #[must_use]
    pub fn span(&self, ast: &Ast) -> Span {
        let value = ast.get(self.value).span;
        match self.kind {
            ArgKind::Named { span, .. } | ArgKind::Spread { span } => Span::join(span, value),
            ArgKind::Positional | ArgKind::Trailing => value,
        }
    }
}

/// One parameter of a function type expression: `NAME: TYPE` or `TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamTy {
    /// The parameter's name, if the type gives one.
    pub name: Option<Name>,
    /// Its type: for the rest parameter, its elements'.
    pub ty: TypeExprId,
    /// Whether it is the rest parameter, `...words: String`.
    pub rest: bool,
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
/// not name (see [Types](crate::ast::typeexprs#types)).
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
/// [Types](crate::ast::typeexprs#types)).
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
    /// `assert COND` or `assert COND, MESSAGE`, a statement of type `{}`
    /// (see the parser's [Failure](crate::parser::failure#failure)).
    Assert {
        /// The condition.
        cond: ExprId,
        /// The message: the one written, or else a string literal written
        /// nowhere of the condition as it is written, in backticks.
        message: ExprId,
    },
    /// `recover { BODY } else { FALLBACK }`, its details in the recover
    /// arena (see [`Ast::recover_decl`]).
    Recover(RecoverId),
    /// A read of a context, `^NAME`, as in `^arch` (see the parser's
    /// [Context](crate::parser::context#context)). The name ends the expression's
    /// span.
    ContextRead(Name),
    /// `BODY with { NAME = VALUE; ... }`, or `with { ... } { BODY }`: the
    /// body evaluated with the contexts the bindings name bound (see the
    /// parser's [Context](crate::parser::context#context)).
    With {
        /// The bindings, in source order (see [`Ast::with_bindings`]).
        bindings: WithRange,
        /// What is evaluated with them.
        body: ExprId,
    },
    /// What the parser put in place of source it could not parse, or of a
    /// lexical error's token (see the parser's [Errors and
    /// recovery](crate::parser::recovery#errors-and-recovery)): its error is
    /// reported already, so it is of type `Error`, which every use accepts
    /// silently. Only ever in a tree with syntax errors ([`Ast::syntax`]),
    /// which is never lowered.
    Error,
    /// A field access with no name after its `.`, `EXPR.`, as an editor
    /// leaves one half-typed: what the parser kept of it, its record,
    /// which is checked as any expression is, the access itself of type
    /// `Error` (see the parser's [Errors and
    /// recovery](crate::parser::recovery#errors-and-recovery)). Only ever in a tree
    /// with syntax errors, which is never lowered.
    NoField(ExprId),
    /// A call whose `(` is never closed, `callee(ARGS` at the end of the
    /// input or of the block around it, as an editor leaves one
    /// half-typed: what the parser kept of it, its callee and the
    /// arguments it parsed, each checked as any expression is, the call
    /// itself of type `Error` (see the parser's [Errors and
    /// recovery](crate::parser::recovery#errors-and-recovery)). Only ever in a tree
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
            | ExprKind::Assert { .. }
            | ExprKind::Recover(_)
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
    /// an alias of `TYPE` (see [Types](crate::ast::typeexprs#types)). Only ever an item of a
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
    /// [Context](crate::parser::context#context)); one with a default is the `let`
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
    /// recovery](crate::parser::recovery#errors-and-recovery)): its error is
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
    /// `.`, in the path pool (see [Types](crate::ast::typeexprs#types)).
    Path(PathRange),
    /// A record type, `{x: Int, "a b": String}` or `{...T, x: Int}`: its
    /// fields and spreads in source order. Repeated names are allowed here
    /// and reported by the checker.
    Record(FieldTyRange),
    /// A function type, `fn(Int, name: String): Bool`, `fn(Int) ->> Int` or
    /// `fn() -> Int`, with a meta or not, as in `fn(v: Version): PackageDef
    /// META-> PackageMeta`, and needing contexts or not, `fn(^arch) -> {}`
    /// (see [Types](crate::ast::typeexprs#types)).
    Fn {
        /// The parameters, each with its name if it has one.
        params: ParamTyRange,
        /// The contexts it is written to need, after its parameters.
        needs: NeedTyRange,
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
    /// arguments in (see [Types](crate::ast::typeexprs#types)). Never written.
    Param(u8),
    /// The enum whose key is `key`, named `name`: the definition of an
    /// enum's declaration in a [`TypeStore`], a type of its own, and of each
    /// use of it (see [Types](crate::ast::typeexprs#types)). Never written.
    Enum {
        /// Its key, a name no source can write that is its identity
        /// ([`crate::names::enum_key`]).
        key: Name,
        /// Its name, as it is declared.
        name: Name,
    },
    /// The declared type of the context `NAME`, of a parameter written
    /// `(^NAME)` with no type (see the parser's
    /// [Context](crate::parser::context#context)), which the checker resolves.
    Context(Name),
    /// A type that names nothing it can, put in place of a written one by
    /// the expansion of a tree's types ([`Ast::expand_types`]): its error,
    /// if it is this one's to report, is among the tree's problems
    /// ([`Ast::type_problems`]). Never written.
    Invalid,
}

/// The meta of a function type expression, `META-> PackageMeta`: what its
/// marker says reading it does, and its type (see [Types](crate::ast::typeexprs#types)).
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

pub(crate) mod builder;
pub(crate) mod pretty;
pub(crate) mod tree;
pub(crate) mod typeexprs;

#[cfg(test)]
mod tests;
