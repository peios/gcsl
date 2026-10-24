//! The type checker and name resolver.
//!
//! One pass over the tree both types every expression and resolves every
//! variable. Its results are side tables indexed by the AST's ids:
//! [`Types`], the type of every expression, and [`Resolution`], where every
//! variable use reads from and every declaration writes to, what each
//! function's and `lazy`'s frame and captures are, which parameter each
//! argument fills, and which expressions are awaited where they are used.
//! Both are returned together as [`Checked`]. Types are [`TyId`]s interned
//! into a [`TypeTable`] the caller owns, so that a
//! [`crate::session::Session`] can keep one table across inputs.
//!
//! This module holds the checker's state, its results and its driver, the
//! walk of a file's body and of each expression. The checking of each part
//! of the language is a submodule of its own, documented there:
//!
//! - Items and declarations: [`items`], [`records`], [`mutation`],
//!   [`scopes`] and [`resolve`], the type expressions.
//! - The top level: [`toplevel`], [`imports`], [`schedule`] and
//!   [`deftypes`].
//! - Expressions: [`expect`], [`operators`], [`names`], [`fields`],
//!   [`lists`], [`loops`] and [`narrowing`].
//! - Functions and calls: [`functions`], [`calls`], [`methods`],
//!   [`inference`], [`futures`] and [`attributes`].
//! - The rest of the language: [`enums`], [`context`], [`failure`] and
//!   [`secrets`].
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
//! An `if` with `else` has the join of its branches' types (see
//! [`TypeTable::join`]), with untyped branches adapting as operands of `+`
//! do first; if both are untyped, the `if` stays untyped, like a block, and
//! `coerce` pushes a type into both branches. The join always exists:
//! branches that do not agree make a union, which is a warning, not an
//! error (see [Unions](crate::check::narrowing#unions)). An `if` without `else` is `{}`, and so
//! must its block be, or `Never` (see [Never](crate::check::narrowing#never)). There is no
//! truthiness: conditions and the operands
//! of `and`, `or` and `not` must be `Bool`.
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
//! A tree the parser recovered from syntax errors ([`Ast::syntax`]) is
//! checked as any is, an error node ([`ExprKind::Error`]) being of type
//! `Error` and a block with an error item ([`ItemKind::Error`]) erroneous,
//! its fields' and its result's types unknown; its check always fails. What
//! the parser kept of a half-typed field access or call
//! ([`ExprKind::NoField`], [`ExprKind::Unclosed`]) is of type `Error` too,
//! its parts checked as any expression is, for an index, with no
//! diagnostic: alone, a part may be what it may not be (a module or a
//! built-in read whole, a callback whose parameters only its call types),
//! and the error is the parser's (`Checker::check_unfinished`). In
//! it, and in a workspace's file whose view sees a file with syntax errors,
//! a name that names nothing, a variable, a type or a record's field, is no
//! error, since it may be declared where the source could not be parsed:
//! the check fails quietly instead (`Checker::unresolved`, and see
//! [`crate::workspace`], "Syntax errors"). In a workspace whose scan
//! recovered from its own errors, a name the scan poisoned in the file (what
//! an import that failed binds, a name in conflict) is read as `Error`,
//! silently, as a value or as a type, unless a binding of a function or an
//! inner block shadows it (`Checker::reads_poisoned`), and a check that
//! found an erroneous type with nothing to report, a read of what the scan
//! poisoned, fails quietly, its result never lowered (see
//! [`crate::workspace`], "Scan errors"). Nor is an empty list an error of
//! its own where an erroneous expected type, or an erroneous callee's
//! parameter, would have typed it (`Checker::check_list`).
//!
//! In a diagnostic's message and notes, a type written as a name, such as
//! `Int`, is shown plainly, and a structural type (a record, list,
//! function, future or union type) in backticks, since it contains spaces and
//! punctuation: "cannot add Int and `{}`". Labels show every type plainly:
//! "expected {x: Int}, found Int".
//!
//! A type may also be written as the name of a declared type, `type
//! PackageDef = {...}`, bare or qualified through an import (`t.PackageDef`),
//! and a record type may spread one, `{...PackageDef, name: String}` (see
//! [Type declarations](crate::check::resolve#type-declarations)). A declared type is a
//! STRUCTURAL ALIAS: it is its definition, not a new type, so two aliases of
//! one record type are one type, a record of the right fields is a
//! `PackageDef` with no ceremony, and a type is shown as what it stands for,
//! never by the name of an alias of it: an error about a `PackageDef` shows
//! the record type.
//!
//! # Sessions
//!
//! A [`Checker`] can start from an existing outermost [`Scope`], which it
//! returns extended by the new input's top-level declarations. This is what
//! lets a [`crate::session::Session`] check each input in the context of the
//! variables its earlier inputs declared, with the same code that checks a
//! file. The scope keeps each binding's kind, so a later input may rebind an
//! earlier input's mutable variable from any block of its top level, but
//! not from a function or a `lazy`, as if it continued the same block; and
//! it lists their slots ([`Scope::mutable_slots`]), the only ones of the
//! scope a later input can write.
//!
//! Within an input, top-level functions are in any order (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)); between inputs, order is as it was: a
//! later input sees the earlier ones, and an earlier one is never changed by
//! a later one. A later input's top-level functions take its own first
//! slots, after the scope's, and may hide an earlier input's names, and an
//! earlier input's functions keep reading their own input's slots, which no
//! later input writes. Nothing about top-level functions is kept in the
//! [`Scope`] beyond their bindings.
//!
//! The same holds of top-level fields. Every cell slot of an input, a root's
//! or another node's, is committed with the input: the scope's next free
//! slot is past the last of them, so no later input reuses one, and a
//! closure or a future of the input reads its cells by slot for as long as
//! it exists. A failed input's cells are forgotten with it, as its other
//! slots are. A later input sees an earlier input's field root as a field
//! of its type, whole, as before this order-free model; `a.c = 2` in a later
//! input declares that input's own `a`.
//!
//! # Recording
//!
//! A language server answers what a name refers to and what it is from an
//! index of the workspace ([`crate::index`]), which needs what the check
//! found of the names it resolved by scope, since scopes are gone once it
//! ends. A check asked to (`Checker::recording`, which only the index's
//! entry points do: never `gcsl run`'s, a session's or `pekit`'s) records
//! (`Record`) each binding as it is declared, where, under its name, of its
//! type then and of what kind, and each `Name` it reads from a binding
//! declared in this input, by where that binding was declared. A binding
//! committed by another file, or by an earlier input of a session, was
//! declared nowhere here, and a read of it is recorded as none, as is a
//! name read through the file's view: the index resolves those again in
//! the view, as the check does (see [Imports](crate::check::imports#imports)), and so a read of
//! the input's own top-level field or `def`, whose declarations are nodes
//! of the view. A name read from the narrowed copy of a variable is read
//! from the `is` that narrowed it, and a function's name for itself from
//! the function expression. A `:=` is recorded with the binding it
//! rebinds: a variable's declares one in its place, which an index takes as
//! the same variable. `Checker::check_recorded` returns the record
//! with the type of every expression, of every `type`
//! item and of every cell of the input's tree as the check left them,
//! WHETHER IT SUCCEEDED OR FAILED, so that a file with errors is indexed
//! for everything that resolved: an expression that failed is `Error`, and
//! after a failure some may be untyped or provisional. Not recording costs
//! one test of an `Option` per binding declared and per name read.
//!

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use crate::ast::{
    Annotate, Arg, ArgKind, ArgRange, Ast, AttributeId, DefTypeId, Effect, ExprId, ExprKind, FnId,
    ForId, GenericOwner, ItemId, ItemKind, ItemRange, Label, PathSegment, RecoverId, SegmentKind,
    Syntax, TypeExprId, UnaryOp, symbol_span,
};
use crate::diagnostic::{self, Diagnostic, InternalError};
use crate::mentions::{FieldTree, Mentions, Schedule};
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::FieldName;
use crate::suggest;
use crate::types::{AwaitKind, DefTypeShape, FnTy, GenericId, TyId, TyKind, TypeTable, ValueTest};
use crate::value::{Quantity, int_to_float};
use crate::workspace::{Context, Target};

/// The type of every expression in an [`Ast`], indexed by [`ExprId`].
///
/// A `Types` returned by [`check`] contains neither [`TyId::UNTYPED_INT`]
/// nor [`TyId::ERROR`], nor any provisional type. An expression that is awaited where
/// it is used keeps the type it has before the await; see
/// [`Resolution::await_of`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    types: Vec<TyId>,
    /// The types of what is not an expression's, usually empty.
    others: OtherTypes,
}

/// The types a [`Types`] holds that are not an expression's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OtherTypes {
    /// The type each `type` item of the tree stands for, by item, in
    /// source order (see [Type declarations](crate::check::resolve#type-declarations)).
    declared: Vec<(ItemId, TyId)>,
    /// Each def-type reference whose path goes on past the definition it
    /// names, by its id, with the read of the definition and the type of
    /// the reference to it (see [Metas](crate::check::attributes#metas)); usually empty.
    references_past: Vec<(ExprId, ExprId, TyId)>,
}

impl Types {
    /// Wraps a raw table. Only the checker (and tests) build tables.
    pub(crate) fn new(types: Vec<TyId>) -> Self {
        Self {
            types,
            others: OtherTypes::default(),
        }
    }

    /// If the def-type reference `id` has a path that goes on past the
    /// definition it names, `package dev.peios.testpkg.versions`: the read
    /// of that definition, the prefix of the reference's hidden read that
    /// names it, and the type of the reference to it, the function with the
    /// definition's meta, whose meta the rest of the path reads, the
    /// reference itself having the type of the last field read (see
    /// [Metas](crate::check::attributes#metas)). `None` for any other expression.
    #[must_use]
    pub fn reference_past(&self, id: ExprId) -> Option<(ExprId, TyId)> {
        self.others
            .references_past
            .iter()
            .find(|&&(at, ..)| at == id)
            .map(|&(_, read, ty)| (read, ty))
    }

    /// The type the `type` item `item` stands for, its expansion, if it is
    /// one the check resolved.
    #[must_use]
    pub fn declared(&self, item: ItemId) -> Option<TyId> {
        self.others
            .declared
            .iter()
            .find(|&&(at, _)| at == item)
            .map(|&(_, ty)| ty)
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

    /// Writes ` = Type` after a `type` item's type as written: its
    /// expansion, the type it stands for.
    fn annotate_type(&self, item: ItemId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.types.declared(item) {
            Some(ty) => write!(f, " = {}", self.table.display(ty, self.names)),
            None => Ok(()),
        }
    }

    /// The read of the definition a def-type reference's path goes on past,
    /// as the check found it ([`Types::reference_past`]).
    fn reference_past(&self, id: ExprId) -> Option<ExprId> {
        self.types.reference_past(id).map(|(read, _)| read)
    }

    /// Writes ` : Type` after the reference to the definition a def-type
    /// reference's path goes on past: its type, with the definition's meta.
    fn annotate_reference(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.types.reference_past(id) {
            Some((_, ty)) => write!(f, " : {}", self.table.display(ty, self.names)),
            None => Ok(()),
        }
    }
}

/// The index of a variable's storage in the evaluator's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot(u32);

impl Slot {
    /// The absence of a slot, in [`Resolution`] entries for nodes that are
    /// not variables.
    pub const NONE: Slot = Slot(u32::MAX);

    /// The slot `index`: in a workspace, a merged node's cell is the slot
    /// of its index (see [Directory scope](crate::check::imports#directory-scope)).
    pub(crate) const fn at(index: u32) -> Slot {
        Slot(index)
    }

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
    /// A top-level function of the input, by its slot in the top level's
    /// frame, read from inside a function or `lazy` of the same input:
    /// never captured, read where the top level's frame is when it runs
    /// (see [Top-level functions](crate::check::toplevel#top-level-functions)).
    Fn(Slot),
    /// The closure being called: a function's reference to itself.
    Callee,
    /// TEMPORARY (alias rule): a built-in function whose typing is special, called: only the callee
    /// of a call of it, or of an alias of it, resolves to one, and lowering makes the call the
    /// built-in's own node. Replaced by an ordinary call when it gets a polymorphic type (see the
    /// [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    Builtin(Builtin),
    /// A member of the `fileroot` module, read as `fileroot.NAME` (see
    /// [Imports](crate::check::imports#imports)): the field access that reads `this`,
    /// `workspace` or `caller`, and the callee of a call of `helper`, which
    /// lowering makes the member's own node, never reading the module.
    Fileroot(Fileroot),
    /// Nothing: the expression or item is not a variable use.
    None,
}

/// A member of the `fileroot` module, implemented in Rust: where files are
/// (see [Imports](crate::check::imports#imports) and [`crate::eval`], "Callers and helpers").
/// The module has no value and no cells: each member is read only as
/// `fileroot.NAME`, through a whole import of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fileroot {
    /// `fileroot.this: String`: the directory of the file the expression is
    /// written in, a constant of the file.
    This,
    /// `fileroot.workspace: String`: the root directory of the workspace, a
    /// constant of the run.
    Workspace,
    /// `fileroot.caller: String`: the directory of the file whose code made
    /// the call the running function is running for, read at runtime.
    Caller,
    /// `fileroot.helper(): {}`: from the point it runs until its function
    /// returns, the calls the function makes pass its own caller on.
    Helper,
}

impl Fileroot {
    /// Every member, in the order of their numbers among a file's scope's
    /// targets (see [`crate::workspace::Target::Fileroot`]).
    pub const ALL: [Fileroot; 4] = [
        Fileroot::This,
        Fileroot::Workspace,
        Fileroot::Caller,
        Fileroot::Helper,
    ];

    /// The name of the module.
    pub const MODULE: &'static str = "fileroot";

    /// Its name, as `fileroot.NAME` reads it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Fileroot::This => "this",
            Fileroot::Workspace => "workspace",
            Fileroot::Caller => "caller",
            Fileroot::Helper => "helper",
        }
    }

    /// The member named `text`, if one is.
    #[must_use]
    pub fn named(text: &str) -> Option<Fileroot> {
        Fileroot::ALL
            .into_iter()
            .find(|member| member.name() == text)
    }

    /// Its index in [`Fileroot::ALL`].
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Fileroot::This => 0,
            Fileroot::Workspace => 1,
            Fileroot::Caller => 2,
            Fileroot::Helper => 3,
        }
    }
}

/// Where a file is, as the driver of its run resolved it, for the `fileroot`
/// module (see [`crate::workspace`], "The standard library"): its directory
/// by its index in the run's table of directories, which the code lowered
/// from it carries ([`crate::ir::Ir::dir`]), and the texts of
/// `fileroot.this` and `fileroot.workspace`, which lowering makes constants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// The index of the file's directory in the run's table of directories.
    pub dir: u32,
    /// The absolute path of the file's directory: `fileroot.this`.
    pub this: Rc<str>,
    /// The absolute path of the workspace's root directory:
    /// `fileroot.workspace`.
    pub workspace: Rc<str>,
}

/// A built-in function: a member of the `builtins` module, implemented in
/// Rust, whose value is a [`crate::Value::Builtin`]. See the module
/// documentation, [Built-in functions](crate::check::calls#built-in-functions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// `len(list)`: the number of elements of a list, an `Int`. Its typing
    /// is special.
    Len,
    /// `int(x: Float): Int`: `x` truncated toward zero.
    Int,
    /// `float(x: Int): Float`: the `Float` nearest `x`.
    Float,
    /// `exec(argv: List<String>) -> {status: Int, stderr: String, stdout:
    /// String}`: runs the program `argv[0]` with the arguments `argv[1..]`
    /// and reports what it did (see [`crate::eval`]). Impure.
    Exec,
    /// `panic(message: String): Never`: ends the run with the runtime error
    /// `message`, at the call.
    Panic,
    /// `split(s: String, sep: String): List<String>`: the pieces of `s`
    /// between occurrences of `sep`, in order, as Rust's `str::split` gives
    /// them; an empty `sep` is a runtime error.
    Split,
    /// `join(parts: List<String>, sep: String): String`: the parts with
    /// `sep` between each pair.
    Join,
    /// `starts_with(s: String, prefix: String): Bool`: whether `s` begins
    /// with the bytes of `prefix`.
    StartsWith,
    /// `ends_with(s: String, suffix: String): Bool`: whether `s` ends with
    /// the bytes of `suffix`.
    EndsWith,
    /// `trim(s: String): String`: `s` without its leading and trailing
    /// Unicode whitespace.
    Trim,
    /// `strip_prefix(s: String, prefix: String): String`: `s` without the
    /// bytes of `prefix` if it begins with them, else `s` unchanged.
    StripPrefix,
    /// `strip_suffix(s: String, suffix: String): String`: `s` without the
    /// bytes of `suffix` if it ends with them, else `s` unchanged.
    StripSuffix,
    /// `lines(s: String): List<String>`: the lines of `s`, as Rust's
    /// `str::lines` gives them.
    Lines,
    /// `fields(s: String): List<String>`: the pieces of `s` between runs of
    /// Unicode whitespace, as Rust's `str::split_whitespace` gives them.
    Fields,
    /// `compare_versions(a: String, b: String): Int`: `-1`, `0` or `1` as
    /// `a` sorts before, equal to or after `b` in version order.
    CompareVersions,
    /// `fs_exists(p: String) -> Bool`, `fs.exists`: whether something is at
    /// the absolute path `p`, symbolic links followed. Impure.
    FsExists,
    /// `fs_read(p: String) -> String`, `fs.read`: the contents of the file
    /// at the absolute path `p`, which must be UTF-8 text. Impure.
    FsRead,
    /// `fs_mkdir(p: String) -> {}`, `fs.mkdir`: makes the directory at the
    /// absolute path `p` and any missing parents, as `mkdir -p`. Impure.
    FsMkdir,
    /// `fs_copy(from: String, to: String) -> {}`, `fs.copy`: copies the
    /// file at the absolute path `from` to the absolute path `to`. Impure.
    FsCopy,
    /// `fs_temp_dir() -> String`, `fs.temp_dir`: makes a new empty
    /// directory, removed with everything in it when the run that made it
    /// is dropped, and gives its absolute path. Impure.
    FsTempDir,
    /// `fs_write(p: String, contents: String) -> {}`, `fs.write`: writes
    /// `contents` to the file at the absolute path `p`, made or replaced
    /// whole, atomically, in a directory that must exist. Impure.
    FsWrite,
    /// `sort_versions(vs: List<String>): List<String>`: `vs` sorted in
    /// version order, as `compare_versions` orders two, a stable sort.
    SortVersions,
    /// `show(value)`: the GCSL source of a value, as text, which reads back
    /// as an equal value. Its typing is special.
    Show,
    /// `exec_live(argv: List<String>) -> {status: Int, stderr: String,
    /// stdout: String}`: `exec`, but the program's standard output and
    /// standard error are also copied, line by line as they arrive, to this
    /// process's standard error (see [`crate::eval`]). Impure.
    ExecLive,
    /// `variant_matches<E, T>(kind: Variant<E, T>, value: E): Bool`:
    /// whether `value`, a value of an enum, is of the variant `kind` (see
    /// [Enums](crate::check::enums#enums)), the prelude's `kind.matches(value)`.
    VariantMatches,
    /// `variant_payload<E, T>(kind: Variant<E, T>, value: E): T`: the
    /// payload of `value`, of the variant `kind`, and a runtime error if it
    /// is another's, the prelude's `kind.payload(value)`.
    VariantPayload,
    /// `partial_matches<T>(partial: Partial<T>, full: T): Bool`: whether
    /// every field `partial` holds equals `full`'s of its name (see [Partial
    /// records](crate::check::enums#partial-records)), the prelude's `partial.matches(full)`.
    PartialMatches,
    /// `secret<T>(value: T, identity: String): Secret<T>`: `value` as a
    /// secret, of the public identity `identity` (see [Secrets](crate::check::secrets#secrets)).
    Secret,
    /// `secret_value<T>(s: Secret<T>): T`: the value of the secret `s`, the
    /// prelude's `s.unsafe_value()`, the deliberate escape (see
    /// [Secrets](crate::check::secrets#secrets)).
    SecretValue,
}

/// How a built-in function is typed (see [Built-in
/// functions](crate::check::calls#built-in-functions)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typing {
    /// By a function type the type table can hold, of any parameters, named
    /// as [`Builtin::params`] says, any result and any effect, which
    /// [`Builtin::ty`] interns, the table in hand: its value is a value of
    /// that type, and a call of it is a call like any other.
    Ordinary,
    /// TEMPORARY (alias rule): call by call, by the checker's rule for it, since no function type
    /// the table can hold is its type (`len`: a `List<T>` for any `T` to `Int`; `show`: any `T`
    /// without a function in it to `String`); its value has an opaque type of its own,
    /// [`TyKind::Builtin`]. To be replaced by a real polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    Special,
}

impl Builtin {
    /// Every built-in function, in the order of the `builtins` module, which
    /// is also the order of their cells, the first slots of a workspace's
    /// frame (see [`crate::workspace`]).
    pub const ALL: [Builtin; 29] = [
        Builtin::Len,
        Builtin::Int,
        Builtin::Float,
        Builtin::Exec,
        Builtin::Panic,
        Builtin::Split,
        Builtin::Join,
        Builtin::StartsWith,
        Builtin::EndsWith,
        Builtin::Trim,
        Builtin::StripPrefix,
        Builtin::StripSuffix,
        Builtin::Lines,
        Builtin::Fields,
        Builtin::CompareVersions,
        Builtin::FsExists,
        Builtin::FsRead,
        Builtin::FsMkdir,
        Builtin::FsCopy,
        Builtin::FsTempDir,
        Builtin::FsWrite,
        Builtin::SortVersions,
        Builtin::Show,
        Builtin::ExecLive,
        Builtin::VariantMatches,
        Builtin::VariantPayload,
        Builtin::PartialMatches,
        Builtin::Secret,
        Builtin::SecretValue,
    ];

    /// The most parameters any built-in function has: what a call of one
    /// collects its arguments into (see [`crate::eval`]).
    pub const MAX_PARAMS: usize = 2;

    /// The names of the fields of what `exec` and `exec_live` return, in the
    /// order of the record each builds: its exit status, its standard output
    /// and its standard error.
    pub const EXEC_FIELDS: [&'static str; 3] = ["status", "stdout", "stderr"];

    /// The name of its member of the `builtins` module.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Builtin::Len => "len",
            Builtin::Int => "int",
            Builtin::Float => "float",
            Builtin::Exec => "exec",
            Builtin::Panic => "panic",
            Builtin::Split => "split",
            Builtin::Join => "join",
            Builtin::StartsWith => "starts_with",
            Builtin::EndsWith => "ends_with",
            Builtin::Trim => "trim",
            Builtin::StripPrefix => "strip_prefix",
            Builtin::StripSuffix => "strip_suffix",
            Builtin::Lines => "lines",
            Builtin::Fields => "fields",
            Builtin::CompareVersions => "compare_versions",
            Builtin::FsExists => "fs_exists",
            Builtin::FsRead => "fs_read",
            Builtin::FsMkdir => "fs_mkdir",
            Builtin::FsCopy => "fs_copy",
            Builtin::FsTempDir => "fs_temp_dir",
            Builtin::FsWrite => "fs_write",
            Builtin::SortVersions => "sort_versions",
            Builtin::Show => "show",
            Builtin::ExecLive => "exec_live",
            Builtin::VariantMatches => "variant_matches",
            Builtin::VariantPayload => "variant_payload",
            Builtin::PartialMatches => "partial_matches",
            Builtin::Secret => "secret",
            Builtin::SecretValue => "secret_value",
        }
    }

    /// The name of the member of the standard `fs` module that is it,
    /// `fs.read`, for one of the file system's, by which its runtime errors
    /// name it whatever name it is called by; `None` for every other
    /// built-in function.
    #[must_use]
    pub const fn fs_name(self) -> Option<&'static str> {
        match self {
            Builtin::FsExists => Some("fs.exists"),
            Builtin::FsRead => Some("fs.read"),
            Builtin::FsMkdir => Some("fs.mkdir"),
            Builtin::FsCopy => Some("fs.copy"),
            Builtin::FsTempDir => Some("fs.temp_dir"),
            Builtin::FsWrite => Some("fs.write"),
            Builtin::Len
            | Builtin::Int
            | Builtin::Float
            | Builtin::Exec
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::SortVersions
            | Builtin::Show
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => None,
        }
    }

    /// The names of its parameters, in order, which named arguments match.
    /// None has more than [`Builtin::MAX_PARAMS`].
    #[must_use]
    pub const fn params(self) -> &'static [&'static str] {
        match self {
            Builtin::Len => &["list"],
            Builtin::Int | Builtin::Float => &["x"],
            Builtin::Exec | Builtin::ExecLive => &["argv"],
            Builtin::Panic => &["message"],
            Builtin::Split => &["s", "sep"],
            Builtin::Join => &["parts", "sep"],
            Builtin::StartsWith | Builtin::StripPrefix => &["s", "prefix"],
            Builtin::EndsWith | Builtin::StripSuffix => &["s", "suffix"],
            Builtin::Trim | Builtin::Lines | Builtin::Fields | Builtin::SecretValue => &["s"],
            Builtin::CompareVersions => &["a", "b"],
            Builtin::FsExists | Builtin::FsRead | Builtin::FsMkdir => &["p"],
            Builtin::FsCopy => &["from", "to"],
            Builtin::FsTempDir => &[],
            Builtin::FsWrite => &["p", "contents"],
            Builtin::SortVersions => &["vs"],
            Builtin::Show => &["value"],
            Builtin::VariantMatches | Builtin::VariantPayload => &["kind", "value"],
            Builtin::PartialMatches => &["partial", "full"],
            Builtin::Secret => &["value", "identity"],
        }
    }

    /// What it may do: only a pure one may be called from a pure function.
    /// `exec` and `exec_live`, which run a program, are impure, and so is
    /// each of the file
    /// system's, `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`,
    /// `fs_temp_dir` and `fs_write`, which touch the world; every other
    /// built-in function is pure, `panic` included, which ends the run as a
    /// division by zero does, `split`, whose empty separator does the same,
    /// and `show`, which fails so on a `Float` with no literal.
    #[must_use]
    pub const fn effect(self) -> Effect {
        match self {
            Builtin::Exec
            | Builtin::ExecLive
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsTempDir
            | Builtin::FsWrite => Effect::Impure,
            Builtin::Len
            | Builtin::Int
            | Builtin::Float
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::SortVersions
            | Builtin::Show
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => Effect::Pure,
        }
    }

    /// How it is typed.
    #[must_use]
    pub const fn typing(self) -> Typing {
        match self {
            Builtin::Len | Builtin::Show => Typing::Special,
            Builtin::Int
            | Builtin::Float
            | Builtin::Exec
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsTempDir
            | Builtin::FsWrite
            | Builtin::SortVersions
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => Typing::Ordinary,
        }
    }

    /// How its value is shown: as its type, `fn(x: Float): Int`, for an
    /// ordinary one, as any function is, and as the type table shows that
    /// type (a record's fields by name, `panic`'s result `Never`, which no
    /// annotation can write); as `builtin len`, `builtin show`, for a
    /// special one.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Builtin::Len => "builtin len",
            Builtin::Show => "builtin show",
            Builtin::Int => "fn(x: Float): Int",
            Builtin::Float => "fn(x: Int): Float",
            Builtin::Exec | Builtin::ExecLive => {
                "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}"
            }
            Builtin::Panic => "fn(message: String): Never",
            Builtin::Split => "fn(s: String, sep: String): List<String>",
            Builtin::Join => "fn(parts: List<String>, sep: String): String",
            Builtin::StartsWith => "fn(s: String, prefix: String): Bool",
            Builtin::EndsWith => "fn(s: String, suffix: String): Bool",
            Builtin::Trim => "fn(s: String): String",
            Builtin::StripPrefix => "fn(s: String, prefix: String): String",
            Builtin::StripSuffix => "fn(s: String, suffix: String): String",
            Builtin::Lines | Builtin::Fields => "fn(s: String): List<String>",
            Builtin::CompareVersions => "fn(a: String, b: String): Int",
            Builtin::FsExists => "fn(p: String) -> Bool",
            Builtin::FsRead => "fn(p: String) -> String",
            Builtin::FsMkdir => "fn(p: String) -> {}",
            Builtin::FsCopy => "fn(from: String, to: String) -> {}",
            Builtin::FsTempDir => "fn() -> String",
            Builtin::FsWrite => "fn(p: String, contents: String) -> {}",
            Builtin::SortVersions => "fn(vs: List<String>): List<String>",
            Builtin::VariantMatches => "fn<E, T>(kind: Variant<E, T>, value: E): Bool",
            Builtin::VariantPayload => "fn<E, T>(kind: Variant<E, T>, value: E): T",
            Builtin::PartialMatches => "fn<T>(partial: Partial<T>, full: T): Bool",
            Builtin::Secret => "fn<T>(value: T, identity: String): Secret<T>",
            Builtin::SecretValue => "fn<T>(s: Secret<T>): T",
        }
    }

    /// The type of its value, interned into `types`, the names of its
    /// parameters, and of the fields of its result, into `names`: for an
    /// ordinary one its function type, its parameters' types and its result
    /// built here, with its [`Builtin::effect`]; the opaque type of a
    /// special one.
    pub(crate) fn ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        // What fills the entries of `param_types` past the built-in's
        // parameters, which the `zip` with `params` below never reads.
        const UNUSED: TyId = TyId::ERROR;
        // Each parameter's type, in the order of `params`, and the result's.
        let (param_types, result): ([TyId; Builtin::MAX_PARAMS], TyId) = match self {
            Builtin::Int => ([TyId::FLOAT, UNUSED], TyId::INT),
            Builtin::Float => ([TyId::INT, UNUSED], TyId::FLOAT),
            Builtin::Exec | Builtin::ExecLive => {
                let argv = types.list(TyId::STRING);
                let [status, stdout, stderr] = Builtin::EXEC_FIELDS.map(|text| names.intern(text));
                let result = types.record([
                    (status, TyId::INT),
                    (stdout, TyId::STRING),
                    (stderr, TyId::STRING),
                ]);
                ([argv, UNUSED], result)
            }
            Builtin::Panic => ([TyId::STRING, UNUSED], types.never()),
            Builtin::Split => ([TyId::STRING, TyId::STRING], types.list(TyId::STRING)),
            Builtin::Join => ([types.list(TyId::STRING), TyId::STRING], TyId::STRING),
            Builtin::StartsWith | Builtin::EndsWith => ([TyId::STRING, TyId::STRING], TyId::BOOL),
            Builtin::Trim | Builtin::FsRead => ([TyId::STRING, UNUSED], TyId::STRING),
            Builtin::StripPrefix | Builtin::StripSuffix => {
                ([TyId::STRING, TyId::STRING], TyId::STRING)
            }
            Builtin::Lines | Builtin::Fields => ([TyId::STRING, UNUSED], types.list(TyId::STRING)),
            Builtin::CompareVersions => ([TyId::STRING, TyId::STRING], TyId::INT),
            Builtin::FsExists => ([TyId::STRING, UNUSED], TyId::BOOL),
            Builtin::FsMkdir => ([TyId::STRING, UNUSED], TyId::EMPTY),
            Builtin::FsCopy | Builtin::FsWrite => ([TyId::STRING, TyId::STRING], TyId::EMPTY),
            Builtin::FsTempDir => ([UNUSED, UNUSED], TyId::STRING),
            Builtin::SortVersions => {
                let list = types.list(TyId::STRING);
                ([list, UNUSED], list)
            }
            // TEMPORARY (alias rule): the opaque type, until a polymorphic one (see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
            Builtin::Len | Builtin::Show => return types.builtin(self),
            Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => {
                return self.generic_ty(types, names);
            }
        };
        let params = self
            .params()
            .iter()
            .zip(param_types)
            .map(|(&text, ty)| (Some(names.intern(text)), ty));
        types.fn_type(params, result, self.effect())
    }
}

impl Builtin {
    /// The type of a generic built-in's value, interned into `types`, its
    /// type parameters' and parameters' names into `names` (see
    /// [Generics](crate::check::methods#generics)): `variant_matches` and `variant_payload` of
    /// `E` and `T`, `partial_matches`, `secret` and `secret_value` of `T`;
    /// `Error` for any other.
    fn generic_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        if matches!(self, Builtin::Secret | Builtin::SecretValue) {
            return self.secret_ty(types, names);
        }
        let partial = self == Builtin::PartialMatches;
        let parameters: &[&str] = if partial { &["T"] } else { &["E", "T"] };
        let generic = types.new_generic(parameters.iter().map(|text| names.intern(text)).collect());
        let [first, second] = match self.params() {
            &[first, second] => [first, second].map(|text| names.intern(text)),
            _ => return TyId::ERROR,
        };
        let ty = if partial {
            let full = types.param(generic, 0);
            let record = types.partial(full);
            types.fn_type(
                [(Some(first), record), (Some(second), full)],
                TyId::BOOL,
                Effect::Pure,
            )
        } else {
            let (of, payload) = (types.param(generic, 0), types.param(generic, 1));
            let kind = types.variant(of, payload);
            let result = if self == Builtin::VariantMatches {
                TyId::BOOL
            } else {
                payload
            };
            types.fn_type(
                [(Some(first), kind), (Some(second), of)],
                result,
                Effect::Pure,
            )
        };
        types.generic_fn(ty, generic)
    }
}

impl Builtin {
    /// The type of `secret` or `secret_value`, as [`Builtin::generic_ty`]
    /// makes it (see [Secrets](crate::check::secrets#secrets)).
    fn secret_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        let generic = types.new_generic(Box::new([names.intern("T")]));
        let value = types.param(generic, 0);
        let secret = types.secret(value);
        let params: Vec<Name> = self
            .params()
            .iter()
            .map(|text| names.intern(text))
            .collect();
        let ty = match (self, params.as_slice()) {
            (Builtin::Secret, &[first, second]) => types.fn_type(
                [(Some(first), value), (Some(second), TyId::STRING)],
                secret,
                Effect::Pure,
            ),
            (_, &[only]) => types.fn_type([(Some(only), secret)], value, Effect::Pure),
            _ => return TyId::ERROR,
        };
        types.generic_fn(ty, generic)
    }
}

/// Whether and how an expression's value is awaited where it is used, in
/// [`Resolution::await_of`]. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AwaitMark {
    /// Not awaited: the value is used as it is.
    None,
    /// Awaited fully at the top, through every level of future.
    Top,
    /// Awaited fully at the top and then in every lazy field of the record
    /// it is, at any depth.
    Deep,
}

impl From<AwaitKind> for AwaitMark {
    fn from(kind: AwaitKind) -> Self {
        match kind {
            AwaitKind::Top => AwaitMark::Top,
            AwaitKind::Deep => AwaitMark::Deep,
        }
    }
}

/// What the checker learns about one function expression or `lazy`, in
/// [`Resolution::function`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FnResolution {
    /// The number of slots the body's frame needs, its parameters included.
    pub frame_size: u32,
    /// The number of parameters, which take slots `0..param_count`.
    pub param_count: u32,
    /// What the function captures, in capture order: where each value is
    /// read from in the frame that creates the closure, with its type. Its
    /// closure keeps, after these, the closures of its default functions
    /// (see [`FnResolution::defaults`]).
    pub captures: Vec<(VarRef, TyId)>,
    /// The name the function sees itself by, if it is the direct
    /// initialiser of a single-name `let`, field or named function item.
    pub callee: Option<Name>,
    /// For each slot of the body's frame, the name of the last variable
    /// assigned to it. Only for showing slots to people.
    pub slot_names: Vec<Name>,
    /// For each parameter, in order, where the body, or a default, first
    /// measures it, in source order, if it is a pure function's bare
    /// parameter that one measures, which makes it present; `None` for every other
    /// parameter. Empty for a `lazy`. See the module documentation.
    pub param_demands: Vec<Option<Span>>,
    /// For each parameter, in order, if it is a callback parameter of an
    /// impure or memoised function whose body was checked without an
    /// error, and was not narrowed at some narrowable position, what kept
    /// it from being narrowed; `None` for every other parameter. Empty if
    /// no parameter has an entry, as for a `lazy`. See the module
    /// documentation.
    pub unnarrowed: Vec<Option<Unnarrowed>>,
    /// For each parameter, in order, its default once checked, a constant or
    /// its default function's place among the closure's captures, if it
    /// declares one (see [Calls](#calls)); `None` for every other
    /// parameter, and one whose default is in error. Empty if no parameter
    /// declares one, as for a `lazy`.
    pub defaults: Vec<Option<ParamDefault>>,
}

/// A callback parameter that was not narrowed at some of its narrowable
/// positions, in [`FnResolution::unnarrowed`]: what the call-site label on
/// a mismatched argument needs. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unnarrowed {
    /// The parameter's type had every narrowable position been narrowed.
    pub narrowed: TyId,
    /// For each parameter of the parameter's function type, in order, what
    /// first kept it from being narrowed, in source order; `None` for one
    /// that was narrowed or is not narrowable.
    pub blocks: Box<[Option<NarrowingBlock>]>,
}

/// What kept a narrowable position of a callback parameter from being
/// narrowed. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NarrowingBlock {
    /// A call of the callback gives the position a value that may be a
    /// future: the argument.
    Given(Span),
    /// The callback is used as a value, other than as the callee of a
    /// call: the use of its name.
    Used(Span),
}

impl NarrowingBlock {
    /// Where it is.
    #[must_use]
    pub const fn span(self) -> Span {
        match self {
            NarrowingBlock::Given(span) | NarrowingBlock::Used(span) => span,
        }
    }
}

/// A parameter's constant default, as its `= DEFAULT` is once checked
/// against the parameter's type (see [Calls](#calls)), which a call that
/// leaves the parameter unfilled is given, as [`ParamDefault::Constant`] in
/// [`FnResolution::defaults`] and [`Resolution::call_defaults`]. It holds no id of a tree, since a call of a
/// `def` of another file, or of a function of an earlier session input, is
/// given it where that tree is not. A field's default of a record type is
/// one too, held in the type ([`crate::types::RecordTy::defaults`]), which
/// a record literal that lacks the field is given, in
/// [`Resolution::record_fills`] (see [Field defaults](#field-defaults)).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constant {
    /// An `Int`.
    Int(i64),
    /// A `Duration` or a `Size`, of its smallest unit.
    Quantity(Quantity, i64),
    /// A `Float`, by its bits ([`f64::to_bits`]), so that a constant has
    /// equality: an integer literal typed `Float` included.
    Float(u64),
    /// A `Bool`.
    Bool(bool),
    /// A `String`.
    Str(Box<str>),
    /// A list, of its elements, in order.
    List(Box<[Constant]>),
    /// A record of at least one field, of its fields, in the order they are
    /// written.
    Record(Box<[(Name, Constant)]>),
    /// The empty record, `{}`.
    Empty,
}

/// What a call that leaves a parameter unfilled gives it, its parameter's
/// default once checked (see [Calls](#calls)), in [`FnResolution::defaults`]
/// and [`Resolution::call_defaults`]: a constant, copied in where the call
/// is, or the default function of a default that is no constant, which the
/// call calls on its side, with the parameters before it. Neither holds an
/// id of a tree, since a call of a `def` of another file, or of a function
/// of an earlier session input, is given it where that tree is not: the
/// default function is found in the callee's closure, by the index among its
/// captures that the closure keeps it at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ParamDefault {
    /// A constant default.
    Constant(Constant),
    /// A default that is no constant: its default function's closure is the
    /// callee closure's capture number `capture`.
    Function {
        /// The index of the default function's closure among the callee
        /// closure's captures, after the function's own.
        capture: u32,
    },
}

/// What the checker keeps of generic functions, methods and their calls
/// (see [Generics](crate::check::methods#generics) and [Methods](crate::check::methods#methods)).
#[derive(Debug, Default)]
struct CallsChecked {
    /// What the type parameters of each generic function and `type` item
    /// belong to, made once for it.
    ids: HashMap<GenericOwner, GenericId>,
    /// The arguments of the call being bound that were checked to infer its
    /// type parameters, each with its type: checked once, then only
    /// expected to fit its substituted parameter.
    prechecked: HashMap<ExprId, TyId>,
    /// The callee of the call being checked, through any parentheses: the
    /// one place a generic function's name may be read.
    callee: Option<ExprId>,
    /// The methods of the earlier inputs of a session, by their names, each
    /// with its binding's.
    earlier_methods: HashMap<Name, Vec<Name>>,
}

/// What a call of a generic function has bound of its type parameters so
/// far, as [`Checker::instantiate`] binds them.
struct TypeBinding {
    /// What the type parameters belong to.
    generic: GenericId,
    /// Each type parameter's binding, by its position, once one is made.
    bound: Vec<Option<TyId>>,
    /// The argument that bound each first.
    binders: Vec<Option<ExprId>>,
}

/// What the check gives lowering besides types and slots, as
/// [`Resolution`] carries it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ForLowering {
    /// Each call that gives a parameter its default, by its id, with each
    /// such parameter and its default, in parameter order (see
    /// [Calls](#calls)); only a program with one has entries.
    call_defaults: HashMap<ExprId, Box<[(u32, ParamDefault)]>>,
    /// Each call of a method, by its id, with its receiver, its callee's
    /// parameter 0 (see [Methods](crate::check::methods#methods)).
    method_calls: HashMap<ExprId, ExprId>,
    /// Each variant as a value, by its id (see [Enums](crate::check::enums#enums)).
    variants: HashMap<ExprId, VariantTag>,
    /// Each read of a context, by its id, with its declaration's key (see
    /// [Context](crate::check::context#context)).
    context_reads: HashMap<ExprId, Name>,
    /// Each `with`, by its id, with the key of the declaration of each
    /// binding's context, in order (see [Context](crate::check::context#context)).
    withs: HashMap<ExprId, Box<[Name]>>,
    /// The slot each `recover` whose fallback names its failure stores the
    /// failure in (see [Failure](crate::check::failure#failure)).
    recover_slots: HashMap<RecoverId, Slot>,
    /// Each operation on a secret, by its id, with the type of the value it
    /// computes, which lowering makes a secret (see [Secrets](crate::check::secrets#secrets)).
    secret_ops: HashMap<ExprId, TyId>,
    /// Each operand of an operation on secrets that is or holds a secret,
    /// by its id, with the type of what it is unwrapped to and whether that
    /// is deeply, every secret it holds its value (see [Secrets](crate::check::secrets#secrets)).
    secret_operands: HashMap<ExprId, (TyId, bool)>,
    /// Each operand of an operation on secrets that neither is nor holds
    /// one, which lowering marks as an operand, so that its failure is not
    /// the operation's (see [Secrets](crate::check::secrets#secrets)).
    plain_operands: HashSet<ExprId>,
    /// The attributes of each function item that has any, by its function,
    /// in source order (see [Attributes](crate::check::attributes#attributes)).
    fn_attributes: HashMap<FnId, Box<[Attribute]>>,
    /// Each record literal, a record block or `{}`, that the checker gave
    /// fields their defaults, by its id, with each such field and its
    /// default, in the order of
    /// their names' text (see [Field defaults](#field-defaults)); only a
    /// program with one has entries.
    record_fills: RecordFills,
}

/// A variant as a value, as lowering makes it a constant (see
/// [Enums](crate::check::enums#enums)): its key, its enum's, and how it is shown,
/// `Toolchain::C`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantTag {
    /// Its key, its identity ([`crate::names::variant_key`]).
    pub key: Name,
    /// Its enum's key ([`crate::names::enum_key`]).
    pub of: Name,
    /// How it is shown, its enum's name and its own.
    pub shown: Box<str>,
}

/// What a def-type gives one of its uses (see [Def-types](crate::check::deftypes#def-types),
/// `Checker::use_signature`): the function type of its definitions, its
/// signature, with what the use's variant binds bound, where its parameters
/// are, and the use's variant, if its definitions are made with one.
struct UseSignature {
    definition: TyId,
    sig: FnTy,
    shape: DefTypeShape,
    tag: Option<VariantTag>,
}

/// Where a method's name is declared, in the input checked (`None`) or
/// another, if that is known (see [Methods](crate::check::methods#methods)).
pub(crate) type MethodSite = Option<(Option<u32>, Span)>;

/// The names of the built-in types applied to type arguments, each if the
/// program's table holds it.
#[derive(Clone, Copy, Debug)]
struct Constructors {
    /// `Future`, which applied to a type is a future of it.
    future: Option<Name>,
    /// `Present`, which applied to a type is it with every bare type in it
    /// present.
    present: Option<Name>,
    /// `List`, which applied to a type is a list of it.
    list: Option<Name>,
    /// `Variant`, for `Variant<E, T>` (see [Enums](crate::check::enums#enums)).
    variant: Option<Name>,
    /// `Partial`, for `Partial<T>` (see [Partial records](crate::check::enums#partial-records)).
    partial: Option<Name>,
    /// `Secret`, for `Secret<T>` (see [Secrets](crate::check::secrets#secrets)).
    secret: Option<Name>,
}

impl Constructors {
    /// The names `names` holds.
    fn of(names: &Names) -> Self {
        Self {
            future: names.lookup("Future"),
            present: names.lookup("Present"),
            list: names.lookup("List"),
            variant: names.lookup("Variant"),
            partial: names.lookup("Partial"),
            secret: names.lookup("Secret"),
        }
    }
}

/// A method a call of its name may mean (see [Methods](crate::check::methods#methods)).
#[derive(Clone, Copy, Debug)]
struct MethodCandidate {
    /// How its closure is read.
    source: MethodSource,
    /// Its function's type, the receiver its first parameter.
    ty: TyId,
    /// Where its name is declared, in this input (`None`) or another, if
    /// it is known.
    declared: MethodSite,
}

/// How a method's closure is read (see [Methods](crate::check::methods#methods)).
#[derive(Clone, Copy, Debug)]
enum MethodSource {
    /// From its binding, of this input or an earlier one of a session, as a
    /// top-level function's is.
    Binding(Name),
    /// From its slot, as another file's: by its name and its position among
    /// those of the name the file sees ([`Context::methods`]).
    Foreign(Name, u32),
}

/// How a call's arguments bind to its callee's parameters beyond what the
/// call shows (see [`Checker::bind_call`]): the callee, if it is known, and
/// how many of its first parameters the call fills apart from its argument
/// list, a method's receiver.
#[derive(Clone, Copy, Debug)]
struct Bound {
    /// The callee, if it is known.
    known: Option<KnownCallee>,
    /// The number of the parameter the signature bound starts at.
    offset: usize,
}

/// One attribute of a function item, resolved (see [Attributes](crate::check::attributes#attributes)):
/// what a host knows it by, and its arguments, every field of its record
/// type by its name, in the type's order, each the constant given or its
/// default. A constant record among them names its fields by the program's
/// names, as every [`Constant`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// What a host knows it by.
    pub id: AttributeId,
    /// Its arguments, by field.
    pub args: Box<[(Box<str>, Constant)]>,
}

/// The defaults of a function, for [`FnResolution::defaults`]: `constants`,
/// each parameter's constant default, if it has one that checked, and
/// `functions`, each parameter with a default function, in parameter order,
/// with whether it checked, whose closures follow the function's
/// `captures` own captures in its closure (see [Calls](#calls)). Empty if no
/// parameter has a default.
fn merged_defaults(
    constants: Vec<Option<Constant>>,
    functions: &[(usize, bool)],
    captures: usize,
) -> Vec<Option<ParamDefault>> {
    let count = functions
        .iter()
        .map(|&(param, _)| param + 1)
        .max()
        .unwrap_or(0)
        .max(constants.len());
    if count == 0 {
        return Vec::new();
    }
    let mut merged: Vec<Option<ParamDefault>> = constants
        .into_iter()
        .map(|constant| constant.map(ParamDefault::Constant))
        .collect();
    merged.resize(count, None);
    for (at, &(param, checked)) in functions.iter().enumerate() {
        if let (true, Some(entry)) = (checked, merged.get_mut(param)) {
            let capture = u32::try_from(captures + at).unwrap_or(u32::MAX);
            *entry = Some(ParamDefault::Function { capture });
        }
    }
    merged
}

/// Each record literal given fields' defaults, by its id, with each such
/// field and its default: see [`Resolution::record_fills`].
type RecordFills = HashMap<ExprId, Box<[(Name, Constant)]>>;

/// What a call of a function whose tree is gone needs to know of its
/// parameters: a function declared by an earlier session input, through
/// the binding its scope carries ([`Scope`]), or a `def` of another file of
/// a workspace, through its cell (see [Imports](crate::check::imports#imports)). For each
/// parameter, in order, whether its body measures it, which makes it
/// present, for the call-site form of the implicit-await error, and its
/// default, if it has one, which a call that leaves it unfilled is given;
/// and its attributes, which a def-type gives its definitions (see
/// [Attributes](crate::check::attributes#attributes)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KnownParams {
    /// For each parameter, whether the body measures it.
    measured: Box<[bool]>,
    /// For each parameter, its default, if it has one; empty if none has.
    defaults: Box<[Option<ParamDefault>]>,
    /// The function's attributes, in source order.
    attributes: Box<[Attribute]>,
}

impl KnownParams {
    /// What a call of the function `resolution` describes, whose attributes
    /// are `attributes`, needs to know of its parameters.
    pub(crate) fn of(resolution: &FnResolution, attributes: &[Attribute]) -> Rc<Self> {
        Rc::new(Self {
            measured: resolution
                .param_demands
                .iter()
                .map(Option::is_some)
                .collect(),
            defaults: resolution.defaults.iter().cloned().collect(),
            attributes: attributes.into(),
        })
    }

    /// Whether the body measures parameter number `param`.
    fn measured(&self, param: usize) -> bool {
        self.measured.get(param).copied().unwrap_or(false)
    }

    /// The default of parameter number `param`, if it has one.
    fn default(&self, param: usize) -> Option<&ParamDefault> {
        self.defaults.get(param).and_then(Option::as_ref)
    }
}

/// How the path of a dotted `let`, field or `:=` resolves: the record, or
/// for a `:=` with an index segment the list, it updates, how far into its
/// path that value's static type reaches, and which of the records and
/// lists along the way are awaited. See the module documentation.
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
    /// How many of the path's leading segments name records, or lists, that
    /// exist in the static type of the updated value: 0 without a source,
    /// and otherwise 1 for the source itself and one more for each
    /// following segment, short of the last, that the type has as a record
    /// or a list (every one, for a `:=`). Every segment after them, the last
    /// excepted, is created as a record.
    pub existing: u32,
    /// Where the item's `existing` entries start in the resolution's table
    /// of awaited records (see [`Resolution::path_awaits`]), or
    /// [`PathResolution::NOT_AWAITED`] if none of its records is awaited,
    /// which is the case of every item but a dotted `let` or `:=` of a
    /// record that may be a future.
    pub awaits: u32,
}

impl PathResolution {
    /// The `awaits` of an item none of whose records is awaited.
    pub const NOT_AWAITED: u32 = u32::MAX;

    /// The resolution of an item that is not a dotted declaration, and of
    /// one that builds its record from nothing.
    pub const NONE: PathResolution = PathResolution::unawaited(VarRef::None, 0);

    /// The resolution of an item that updates the record read from
    /// `source`, `existing` of whose path's segments exist, none of them
    /// awaited.
    const fn unawaited(source: VarRef, existing: u32) -> Self {
        Self {
            source,
            existing,
            awaits: PathResolution::NOT_AWAITED,
        }
    }
}

/// What the checker decided for one `is` or `as` expression, in
/// [`Resolution::test_of`]. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResolution {
    /// The plan of the test the expression runs at runtime, against the
    /// value of its operand once awaited as [`Resolution::await_of`] says.
    pub test: ValueTest,
    /// For an `is` that narrows the branches of the `if` it is the
    /// condition of, the slot the value it tests is stored in, which the
    /// branches' narrowed bindings read; [`Slot::NONE`] for every other.
    pub slot: Slot,
    /// For an `as`, its operand as its failure names it: the operand as
    /// written in backticks if it is a name or a path, as in "`x`", and
    /// "this value" otherwise.
    pub subject: Box<str>,
    /// For an `as`, the type it tests for as its failure names it, with its
    /// article, as in "an Int" or "a `List<Int>`".
    pub target: Box<str>,
}

/// For each node of a workspace file's tree, the children that other
/// files declare under it, each with its name, cell and type (see
/// [Directory scope](crate::check::imports#directory-scope)).
type SharedChildren = Vec<Vec<(Name, Slot, TyId)>>;

/// The records a file of a workspace reads whole through its imports: each
/// one's slot and fields (see [`Resolution::imported_records`]).
type ImportedRecords = Vec<ImportedRecord>;

/// A record a file of a workspace reads whole through an import: its
/// private slot, and the name, cell and type of each of its fields.
pub type ImportedRecord = (Slot, Vec<(Name, Slot, TyId)>);

/// A record a file of a workspace assembles of a node of its view with a
/// leaf of the file under it, as a node of its schedule: its private slot
/// ([`Slot::NONE`] if the file never reads it whole), the position of the
/// first item of the file under it, and the name, cell and type of each of
/// its fields, in the view's order.
pub type ViewRecord = (Slot, usize, Vec<(Name, Slot, TyId)>);

/// Where every variable of an [`Ast`] is stored, what every function's
/// frame and captures are, and which parameter every argument fills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// Where each `Name` expression reads, indexed by [`ExprId`];
    /// [`VarRef::None`] for every other expression.
    expr_vars: Vec<VarRef>,
    /// Whether and how each expression is awaited where it is used,
    /// indexed by [`ExprId`].
    awaits: Vec<AwaitMark>,
    /// The slot each `Let`, `Field` and `Rebind` item writes, indexed by
    /// [`ItemId`], and for a `Spread` item the slot its operand's value is
    /// held in while its fields are read; [`Slot::NONE`] for every other
    /// item. Every definition of an open field writes the field's one slot,
    /// and so does every definition of a field that overrides a soft one.
    item_slots: Vec<Slot>,
    /// Where the slots of each `Spread` item's fields are in
    /// `spread_slots`, indexed by [`ItemId`]: their start and their number;
    /// `(0, 0)` for every other item.
    item_spreads: Vec<(u32, u32)>,
    /// The slot each field of each spread's operand is stored in, in the
    /// order of the fields of the operand's static type, each spread's
    /// together.
    spread_slots: Vec<Slot>,
    /// How the path of each dotted `Let`, `Field` and `Rebind` item
    /// resolves, indexed by [`ItemId`]; [`PathResolution::NONE`] for every
    /// other item, and for one that builds its record from nothing.
    item_paths: Vec<PathResolution>,
    /// For each dotted `let` and `:=` item of which some record along its
    /// path is awaited, whether each of its existing records is, from its
    /// source down, where its [`PathResolution::awaits`] says. Items with no
    /// awaited record have no entries.
    path_awaits: Vec<bool>,
    /// Each top-level function of the input, by its item, in source order,
    /// with where its closure is made.
    top_functions: Vec<(ItemId, MadeAt)>,
    /// The cell of each node of the input's tree of top-level fields
    /// ([`Mentions::tree`]), by its index there: the slot of the top level's
    /// frame that holds its value. A root's is its binding's slot, a `def`'s
    /// its function's; the others are slots no name finds.
    field_cells: Vec<Slot>,
    /// For a file of a workspace, the private slot of the file's own record
    /// of each node it shares with another file, by its index in the tree;
    /// [`Slot::NONE`] for any other node, and empty outside a workspace
    /// (see [Directory scope](crate::check::imports#directory-scope)).
    own_records: Vec<Slot>,
    /// For a file of a workspace, the children of each node of its tree
    /// that other files declare, with their cells and types: what the
    /// node's assembly reads besides its own children. Empty outside a
    /// workspace.
    shared_children: SharedChildren,
    /// For a file of a workspace, the records it reads whole through its
    /// imports (a directory's, a file's own, a file's part of a shared
    /// node), each with the private slot it is assembled into as the file
    /// starts and the name, cell and type of each field, nested records
    /// before the records that hold them (see [Imports](crate::check::imports#imports)).
    imported_records: ImportedRecords,
    /// For a file of a workspace, the record of each node of its view with
    /// a leaf of the file under it, by its index among them ([`Mentions::views`]):
    /// stored at its point in the schedule, after its own children, if the
    /// file reads it whole (see [Imports](crate::check::imports#imports)).
    view_records: Vec<ViewRecord>,
    /// For a value file of a workspace that another file imports, the cell
    /// its result is stored in as its block ends; [`Slot::NONE`] otherwise.
    value_cell: Slot,
    /// Where the input is, as its run's driver resolved it: what its code
    /// carries and what `fileroot.this` and `fileroot.workspace` are in it;
    /// the default, directory 0 and empty texts, outside a workspace's
    /// context, where nothing can import `fileroot`.
    place: Place,
    /// The tree of the input's top-level fields and when each node is
    /// computed (see [Top-level fields](crate::check::toplevel#top-level-fields)).
    field_schedule: FieldSchedule,
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
    /// The slot each `for` expression's variable is stored in, indexed by
    /// [`ForId`].
    for_slots: Vec<Slot>,
    /// What each `break`, `continue` and `return` item is for, indexed by
    /// [`ItemId`]; `None` for every other item, and for one that is in
    /// error.
    jumps: Vec<Option<JumpTarget>>,
    /// What was found of each loop, indexed by [`ForId`].
    loops: Vec<LoopResolution>,
    /// The labelled blocks that some `return` leaves, by their ids.
    returned_blocks: HashSet<ExprId>,
    /// The statements whose values are discarded, by their ids: calls of a
    /// function that is not pure, outside a pure function.
    discarded: HashSet<ExprId>,
    /// What was decided for each `is` and `as` expression, by its id; only
    /// a program that has one has entries.
    tests: HashMap<ExprId, TestResolution>,
    /// Each field access that reads a function's meta, `func.versions`, by
    /// its id, with the meta's type and how the meta is awaited before its
    /// field is read (see [Metas](crate::check::attributes#metas)); only a program that reads one
    /// has entries.
    meta_reads: HashMap<ExprId, (TyId, AwaitMark)>,
    /// Each definition made with a def-type that declares a meta, by its
    /// function, with the cell of the top level's frame that holds its
    /// meta's closure (see [Metas](crate::check::attributes#metas)).
    metas: Vec<(FnId, Slot)>,
    /// What the check gives lowering besides types and slots: see the
    /// accessors.
    lowered: ForLowering,
}

/// The field of a `recover`'s failure, `{message: String}` (see
/// [Failure](crate::check::failure#failure)).
pub(crate) const FAILURE_MESSAGE: &str = "message";

/// What a `break`, `continue` or `return` item is for (see
/// [`Resolution::target`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JumpTarget {
    /// A loop: the one a `break` or `continue` is for, or the one a
    /// `return'L` names.
    Loop(ForId),
    /// The labelled block, by its expression, that a `return'L` names.
    Block(ExprId),
    /// The innermost function or `lazy` around an unlabelled `return`.
    Function,
}

/// What the checker found of a loop, for lowering: see
/// [`Resolution::loop_of`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoopResolution {
    /// Whether the loop cannot end: a `for BLOCK` that no `break` is for,
    /// whose ordinary type is `Never`, and which builds no list.
    pub endless: bool,
    /// Whether some `return'L` leaves it, so that it evaluates to the
    /// value returned.
    pub returned: bool,
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

    /// Whether and how expression `id` is awaited where it is used:
    /// [`AwaitMark::None`] if it is used as it is. An awaited expression's
    /// type in [`Types`] is its type before the await.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn await_of(&self, id: ExprId) -> AwaitMark {
        self.awaits[id.index()]
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

    /// The parameters that the call `id` gives their defaults, each with
    /// its default, in parameter order: those it leaves unfilled that have
    /// one, if its callee is known (see [Calls](#calls)); none for any
    /// other call, and for an expression that is no call.
    #[must_use]
    pub fn call_defaults(&self, id: ExprId) -> &[(u32, ParamDefault)] {
        self.lowered
            .call_defaults
            .get(&id)
            .map_or(&[], |defaults| defaults)
    }

    /// The variant the expression `id` is, a variant as a value (see
    /// [Enums](crate::check::enums#enums)), which lowering makes a constant.
    #[must_use]
    pub fn variant(&self, id: ExprId) -> Option<&VariantTag> {
        self.lowered.variants.get(&id)
    }

    /// The key of the declaration of the context the expression `id`, a
    /// read of one, reads (see [Context](crate::check::context#context)); its default's function,
    /// if it has one, is the read's variable ([`Resolution::expr_var`]).
    #[must_use]
    pub fn context_read(&self, id: ExprId) -> Option<Name> {
        self.lowered.context_reads.get(&id).copied()
    }

    /// The type of the value the expression `id` computes, if it is an
    /// operation on a secret, whose result lowering makes a secret (see
    /// [Secrets](crate::check::secrets#secrets)).
    #[must_use]
    pub fn secret_op(&self, id: ExprId) -> Option<TyId> {
        self.lowered.secret_ops.get(&id).copied()
    }

    /// The type of what the expression `id` is unwrapped to, and whether
    /// deeply, every secret it holds its value, if it is an operand of an
    /// operation on secrets that is or holds a secret, which lowering
    /// unwraps (see [Secrets](crate::check::secrets#secrets)).
    #[must_use]
    pub fn secret_operand(&self, id: ExprId) -> Option<(TyId, bool)> {
        self.lowered.secret_operands.get(&id).copied()
    }

    /// Whether the expression `id` is an operand of an operation on secrets
    /// that neither is nor holds one, which lowering marks as an operand,
    /// so that its failure is not the operation's (see [Secrets](crate::check::secrets#secrets)).
    #[must_use]
    pub fn is_plain_operand(&self, id: ExprId) -> bool {
        self.lowered.plain_operands.contains(&id)
    }

    /// The slot the `recover` `id` stores its failure in for its fallback,
    /// if the fallback names it (see [Failure](crate::check::failure#failure)).
    #[must_use]
    pub fn recover_slot(&self, id: RecoverId) -> Option<Slot> {
        self.lowered.recover_slots.get(&id).copied()
    }

    /// The keys of the declarations of the contexts the `with` `id` binds,
    /// one for each of its bindings, in order (see [Context](crate::check::context#context)).
    #[must_use]
    pub fn with_keys(&self, id: ExprId) -> Option<&[Name]> {
        self.lowered.withs.get(&id).map(|keys| &keys[..])
    }

    /// The receiver of the call `id`, if it is a method's (see
    /// [Methods](crate::check::methods#methods)): its callee's parameter 0, the callee itself
    /// read as its method's binding, not as the field access it is written
    /// as.
    #[must_use]
    pub fn method_receiver(&self, id: ExprId) -> Option<ExprId> {
        self.lowered.method_calls.get(&id).copied()
    }

    /// The attributes of function `function`, in source order: none for a
    /// function that is no function item, or has none (see
    /// [Attributes](crate::check::attributes#attributes)).
    #[must_use]
    pub fn attributes(&self, function: FnId) -> &[Attribute] {
        self.lowered
            .fn_attributes
            .get(&function)
            .map_or(&[], |attributes| attributes)
    }

    /// The fields that the record literal `block`, a record block or `{}`,
    /// lacks and is given, each with its default, in the order of their
    /// names' text: the defaulted fields of the record type it is written
    /// where it is expected (see [Field defaults](#field-defaults)); none
    /// for any other block, and for an expression that is no block.
    #[must_use]
    pub fn record_fills(&self, block: ExprId) -> &[(Name, Constant)] {
        self.lowered
            .record_fills
            .get(&block)
            .map_or(&[], |fills| fills)
    }

    /// The slot written by item `id`, or [`Slot::NONE`] if it is not a
    /// declaration. For a spread item, the slot its operand's value is held
    /// in while its fields are read, which no binding owns (see
    /// [`Resolution::spread_slots`] for its fields').
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn item_slot(&self, id: ItemId) -> Slot {
        self.item_slots[id.index()]
    }

    /// Each top-level function of the input, by its item, in source order,
    /// with where its closure is made: what lowering stores in each one's
    /// slot, and when (see [When a function may be
    /// used](crate::check::schedule#when-a-function-may-be-used)).
    #[must_use]
    pub fn top_functions(&self) -> &[(ItemId, MadeAt)] {
        &self.top_functions
    }

    /// For a file of a workspace, the private slot of the file's own record
    /// of each node of its tree it shares with another file, by the node's
    /// index; [`Slot::NONE`] for the others.
    #[must_use]
    pub fn own_records(&self) -> &[Slot] {
        &self.own_records
    }

    /// For a file of a workspace, the children of the node `node` of its
    /// tree that other files declare, with their cells and types.
    #[must_use]
    pub fn shared_children(&self, node: u32) -> &[(Name, Slot, TyId)] {
        self.shared_children
            .get(node as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// The cell of each node of the input's tree of top-level fields, by
    /// its index in [`FieldSchedule::tree`]: the slot of the top level's
    /// frame its value is stored in, once, at its point.
    #[must_use]
    pub fn field_cells(&self) -> &[Slot] {
        &self.field_cells
    }

    /// The records the input, a file of a workspace, reads whole through
    /// its imports, assembled as it starts: each one's private slot and the
    /// name, cell and type of each of its fields, nested records first (see
    /// [Imports](crate::check::imports#imports)).
    #[must_use]
    pub fn imported_records(&self) -> &[ImportedRecord] {
        &self.imported_records
    }

    /// For a file of a workspace, the record of the node of its view with a
    /// leaf of the file under it numbered `node` among the field nodes
    /// (after its tree's), if it is one: stored at the node's point (see
    /// [Imports](crate::check::imports#imports)).
    #[must_use]
    pub fn view_record(&self, node: u32) -> Option<&ViewRecord> {
        let at = (node as usize).checked_sub(self.field_schedule.tree.nodes.len())?;
        self.view_records.get(at)
    }

    /// Every record of [`Resolution::view_record`], by the view node's
    /// index among the file's nodes of its view with a leaf of it under
    /// them.
    #[cfg(test)]
    pub(crate) fn view_records(&self) -> &[ViewRecord] {
        &self.view_records
    }

    /// The cell the input's result is stored in as its block ends, if it is
    /// a value file of a workspace that another file imports; else
    /// [`Slot::NONE`].
    #[must_use]
    pub const fn value_cell(&self) -> Slot {
        self.value_cell
    }

    /// Where the input is: the index of its directory in its run's table,
    /// which its code carries, and the texts `fileroot.this` and
    /// `fileroot.workspace` lower to in it.
    #[must_use]
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// Places the input at `place`: what an input checked with no
    /// workspace, which knows no place of its own, is given by a driver that
    /// runs it in one, as the call a workspace makes to invoke a definition
    /// by name (see [`crate::workspace`], "Def-types by name").
    pub(crate) fn set_place(&mut self, place: Place) {
        self.place = place;
    }

    /// The tree of the input's top-level fields and when each node is
    /// computed: what lowering stores in each cell, and before which item.
    #[must_use]
    pub fn field_schedule(&self) -> &FieldSchedule {
        &self.field_schedule
    }

    /// The slot each field of the spread item `id`'s operand is stored in,
    /// in the order of the fields of the operand's static type once
    /// awaited: a new slot for a name that is not yet a field of its block,
    /// and the field's one slot for a soft field it overrides (see the
    /// module documentation). Empty for every other item.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn spread_slots(&self, id: ItemId) -> &[Slot] {
        let (start, count) = self.item_spreads[id.index()];
        let (start, count) = (start as usize, count as usize);
        self.spread_slots
            .get(start..start.saturating_add(count))
            .unwrap_or_default()
    }

    /// The slot the variable of the `for` expression `id` is stored in,
    /// which each element is written to before the guard and the body run.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn for_slot(&self, id: ForId) -> Slot {
        self.for_slots[id.index()]
    }

    /// The loop that the `break` or `continue` item `id` is for: the
    /// innermost loop of its function whose body holds it, or the one its
    /// label names. `None` for any other item, and for an id this table
    /// does not have.
    #[must_use]
    pub fn jump_target(&self, id: ItemId) -> Option<ForId> {
        match self.target(id)? {
            JumpTarget::Loop(loop_id) => Some(loop_id),
            JumpTarget::Block(_) | JumpTarget::Function => None,
        }
    }

    /// What the `break`, `continue` or `return` item `id` is for: a loop,
    /// a labelled block or, for an unlabelled `return`, the innermost
    /// function or `lazy`. `None` for any other item, for one in error,
    /// and for an id this table does not have.
    #[must_use]
    pub fn target(&self, id: ItemId) -> Option<JumpTarget> {
        self.jumps.get(id.index()).copied().flatten()
    }

    /// What was found of the loop `id`: whether it cannot end, and whether
    /// a `return` leaves it. The default, a loop that can end and that no
    /// `return` leaves, for an id this table does not have.
    #[must_use]
    pub fn loop_of(&self, id: ForId) -> LoopResolution {
        self.loops.get(id.index()).copied().unwrap_or_default()
    }

    /// Whether some `return` leaves the labelled block `id`, so that it
    /// evaluates to the value returned.
    #[must_use]
    pub fn is_returned_block(&self, id: ExprId) -> bool {
        self.returned_blocks.contains(&id)
    }

    /// Whether the statement `id` is a call of a function that is not pure
    /// whose value, whatever it is, is discarded once awaited, as a bare
    /// `await`'s is.
    #[must_use]
    pub fn is_discarded(&self, id: ExprId) -> bool {
        self.discarded.contains(&id)
    }

    /// What was decided for the `is` or `as` expression `id`: its test's
    /// plan, and where an `is` that narrows stores the value it tests, or
    /// how an `as` names what it tests. `None` for any other expression.
    #[must_use]
    pub fn test_of(&self, id: ExprId) -> Option<&TestResolution> {
        self.tests.get(&id)
    }

    /// If the field access `id` reads a function's meta (see
    /// [Metas](crate::check::attributes#metas)): the meta's type, before any await, and how the
    /// meta is awaited before its field is read. `None` for any other
    /// expression.
    #[must_use]
    pub fn meta_read(&self, id: ExprId) -> Option<(TyId, AwaitMark)> {
        self.meta_reads.get(&id).copied()
    }

    /// Each definition made with a def-type that declares a meta, by its
    /// function, with the cell of the top level's frame that holds its
    /// meta's closure, which lowering gives its code (see [Metas](crate::check::attributes#metas)).
    #[must_use]
    pub fn metas(&self) -> &[(FnId, Slot)] {
        &self.metas
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

    /// For each of the `path.existing` records along the path that `path`
    /// resolves, from its source down, whether it is awaited before it is
    /// updated, being a value that may be a future: empty if none is (see
    /// the module documentation). `None` if `path` names entries this
    /// resolution does not have.
    #[must_use]
    pub fn path_awaits(&self, path: PathResolution) -> Option<&[bool]> {
        if path.awaits == PathResolution::NOT_AWAITED {
            return Some(&[]);
        }
        let start = path.awaits as usize;
        self.path_awaits
            .get(start..start.checked_add(path.existing as usize)?)
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
            awaits: vec![AwaitMark::None; exprs],
            item_slots: vec![Slot::NONE; items],
            item_spreads: vec![(0, 0); items],
            spread_slots: Vec::new(),
            item_paths: vec![PathResolution::NONE; items],
            path_awaits: Vec::new(),
            top_functions: Vec::new(),
            field_cells: Vec::new(),
            own_records: Vec::new(),
            shared_children: Vec::new(),
            imported_records: Vec::new(),
            view_records: Vec::new(),
            value_cell: Slot::NONE,
            place: Place::default(),
            field_schedule: FieldSchedule::default(),
            frame_size: 0,
            slot_names: Vec::new(),
            functions: Vec::new(),
            arg_params: Vec::new(),
            for_slots: Vec::new(),
            jumps: vec![None; items],
            loops: Vec::new(),
            returned_blocks: HashSet::new(),
            discarded: HashSet::new(),
            tests: HashMap::new(),
            meta_reads: HashMap::new(),
            metas: Vec::new(),
            lowered: ForLowering::default(),
        }
    }

    /// Forgets the parameter of every argument, as for an erroneous call,
    /// for testing what lowering makes of that.
    #[cfg(test)]
    pub(crate) fn unbind_arguments(&mut self) {
        self.arg_params.fill(u32::MAX);
    }

    /// Forgets the slot of every `for`'s variable, for testing what
    /// lowering makes of that.
    #[cfg(test)]
    pub(crate) fn forget_for_slots(&mut self) {
        self.for_slots.fill(Slot::NONE);
    }

    /// Number of expression entries in both expression tables, equal to
    /// the number of expressions in the [`Ast`].
    pub(crate) fn expr_len(&self) -> usize {
        self.expr_vars.len().min(self.awaits.len())
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

    /// Number of `for` entries, equal to the number of `for` expressions
    /// in the [`Ast`].
    pub(crate) fn for_len(&self) -> usize {
        self.for_slots.len()
    }

    /// Number of item entries in every item table, equal to the number of
    /// items in the [`Ast`].
    pub(crate) fn item_len(&self) -> usize {
        self.item_slots
            .len()
            .min(self.item_paths.len())
            .min(self.item_spreads.len())
            .min(self.jumps.len())
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
    /// The type of the cell of each node of the input's tree of top-level
    /// fields, by its index there; a `def`'s is its function's type.
    pub cell_types: Vec<TyId>,
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
    /// The function expression a single-name `let`, field or named function
    /// item was initialised with directly, for the call-site form of the
    /// implicit-await error (see `Checker::argument_await_error`) and the
    /// defaults a call of it gives (see [Calls](#calls)); `None` for every
    /// other binding, and for one declared by an earlier input of a session,
    /// whose [`FnId`]s belong to another tree.
    function: Option<FnId>,
    /// For a callback parameter of an impure or memoised function being
    /// checked, its index in `Checker::callbacks`, where its calls and uses
    /// are recorded (see `Checker::narrow_callbacks`); `None` for every
    /// other binding.
    callback: Option<usize>,
}

impl Binding {
    /// The binding as a later input's [`Scope`] keeps it: without its span
    /// and function ids, which are meaningless for a later input, with its
    /// own source and tree.
    fn forgotten(self) -> Self {
        Self {
            declared: None,
            function: None,
            callback: None,
            ..self
        }
    }
}

/// What a check records of its input's names for an index of it, when it
/// is asked to ([`Checker::recording`]; see [Recording](#recording)), with
/// what its tables were as it ended, whether it succeeded or failed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Record {
    /// Each binding the check declared, in the order it declared them.
    pub(crate) decls: Vec<RecordedDecl>,
    /// Each name read from a binding declared in this input: the `Name`
    /// expression and where its binding was declared
    /// ([`RecordedDecl::declared`]). A name read through the file's view,
    /// or from a declaration another input committed, has none.
    pub(crate) uses: Vec<(ExprId, Span)>,
    /// Each `:=` that rebinds a binding declared in this input: its item,
    /// and where the binding was declared. A variable's `:=` declares a
    /// binding in its place, at the item, which an index takes as the same
    /// one; a mutable variable's, or a `for` header's, keeps its binding.
    pub(crate) rebinds: Vec<(Span, Span)>,
    /// The type of every expression as the check left it, indexed by
    /// [`ExprId`]: `Error` for one that failed, and, after a failed check,
    /// possibly untyped or provisional.
    pub(crate) types: Vec<TyId>,
    /// The type each `type` item of the input stands for, by item.
    pub(crate) declared_types: Vec<(ItemId, TyId)>,
    /// The type of the cell of each node of the input's tree of top-level
    /// fields, by its index there, as [`CheckOutput::cell_types`] has it.
    pub(crate) cells: Vec<TyId>,
    /// The type of the input's value, as [`CheckOutput::ty`] has it.
    pub(crate) value: Option<TyId>,
    /// Each call of a method (see [Methods](crate::check::methods#methods)), by its callee, with
    /// where the method's name is declared, in this input (`None`) or
    /// another, if that is known.
    pub(crate) methods: Vec<(ExprId, MethodSite)>,
}

/// A binding a check declared, as [`Record`] keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecordedDecl {
    /// Where it was declared, as [`Binding`] says: a `let`'s, field's or
    /// `:=`'s item, a spread's item for its soft fields, a parameter, a
    /// `for`'s name, a function expression for its name for itself, and an
    /// `is` for the narrowed copy of the variable it tests.
    pub(crate) declared: Span,
    /// Its name.
    pub(crate) name: Name,
    /// Its type as it was declared.
    pub(crate) ty: TyId,
    /// What it is.
    pub(crate) kind: BindingKind,
}

/// What a [`Binding`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindingKind {
    /// A variable: declared by a `let`, a `:=` or a parameter.
    Variable,
    /// A mutable variable, declared by a `let mut`, which `:=` rebinds in
    /// its one slot, from any block of its function, keeping its type and
    /// its binding.
    Mutable,
    /// A field, which `:=` may not rebind.
    Field,
    /// A function's name for itself, inside its own body, which `:=` may
    /// not rebind either; it has no slot.
    Callee,
    /// A variable declared by a plain `let` in the header of a `for`: one
    /// slot for its life, as a mutable variable's, which a `:=` in the
    /// header, after the condition, rebinds, keeping its type, but no `:=`
    /// in the body may (see [For](crate::check::loops#for)).
    Header,
    /// The copy of a variable that a branch of `if NAME is TYPE` sees, of
    /// the narrowed type or the remainder, which `:=` may not rebind: the
    /// rebind would change the copy, not the variable it was copied from.
    Narrowed,
}

/// How the checker resolves the copies of declared types' definitions in a
/// file (see [Type declarations](crate::check::resolve#type-declarations)).
#[derive(Clone, Copy, Debug, Default)]
struct Quiet {
    /// Whether a copy of a declared type's definition, or a type of a file
    /// a workspace checked the `type` items of already, is being resolved:
    /// its errors are reported where it is written, not here.
    types: bool,
    /// Whether a type was erroneous with nothing of this file's to report,
    /// its error being reported where it is written: the check fails then,
    /// with no error of its own if it has none.
    failed: bool,
    /// Whether the input's view sees another file with syntax errors (see
    /// [`Checker::sees_broken`]).
    sees_broken: bool,
    /// How many values being checked, one inside another, are checked
    /// against an erroneous expected type, one whose error is reported
    /// elsewhere, or as the arguments of an erroneous callee: in one, an
    /// empty list, which the expected type would have typed, is no error of
    /// its own, the check failing quietly (see [`Checker::check_list`]).
    expecting_error: u32,
}

/// What a bare type written in a type expression means: see the module
/// documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Bareness {
    /// A bare type is present: `Int` is `Int`.
    Present,
    /// A bare type written as a name, a record type, a list type or a union
    /// with no `Future` member is the either shape: `Int` is `Int |
    /// Future<Int>`, `{a: Int}` is `{a: Int | Future<Int>} | Future<{a: Int
    /// | Future<Int>}>`, and `Int | String` is `Int | String | Future<Int |
    /// String>`. A function type is not (see `Checker::resolve_type`).
    Either,
}

/// What bare types mean in the parameters and in the return type of a
/// function of effect `effect`, as written in a function type or a function
/// expression's signature: present in both for a pure function; present in
/// the parameters, which are the key its results are memoised by, and the
/// either shape in the return type for a memoised one; the either shape in
/// both for an impure one. (A function expression then infers what its
/// pure parameters and its impure or memoised results are: see
/// `Checker::check_fn`.)
const fn bareness_of(effect: Effect) -> (Bareness, Bareness) {
    match effect {
        Effect::Pure => (Bareness::Present, Bareness::Present),
        Effect::Memo => (Bareness::Present, Bareness::Either),
        Effect::Impure => (Bareness::Either, Bareness::Either),
    }
}

/// The name the item `id` of the outermost block declares as a variable or
/// field: the first segment of the path of a `let`, `let mut`, field or
/// `:=` that is not a named function item or a `def`.
fn root_declares(ast: &Ast, id: ItemId) -> Option<Name> {
    let (ItemKind::Let {
        path,
        function: false,
        ..
    }
    | ItemKind::Field {
        path,
        function: false,
        ..
    }
    | ItemKind::Rebind { path, .. }) = ast.item(id).kind
    else {
        return None;
    };
    ast.path(path).first().and_then(PathSegment::name)
}

/// The tree of an input's top-level fields and when each of its nodes is
/// computed: what the checker found from [`Mentions`] and lowering stores
/// the cells by (see [Top-level fields](crate::check::toplevel#top-level-fields)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldSchedule {
    /// The tree of the input's top-level fields.
    pub tree: FieldTree,
    /// The point of each node, and the order the nodes are computed in.
    pub schedule: Schedule,
}

/// What the checker knows of one node of the tree of the input's top-level
/// fields (see [Top-level fields](crate::check::toplevel#top-level-fields)), or, in a workspace,
/// of one node of the file's view numbered after them (see
/// [Imports](crate::check::imports#imports)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldCell {
    /// The slot of the top level's frame that holds its value.
    slot: Slot,
    /// For a root, the index in `bindings` of the binding of its name,
    /// declared as the input starts.
    binding: Option<usize>,
    /// Its type, once its initialiser is checked (a leaf) or its children
    /// are (an interior node); a reference to it before that, which only a
    /// cycle makes, is erroneous.
    ty: Option<TyId>,
    /// Once a leaf is checked, the position among the outermost block's
    /// items of the first item before which it can be computed: `0` if
    /// it reads nothing that stops it, its own item's if it reads a mutable
    /// variable, `usize::MAX` if it read a name it cannot see yet or is on a
    /// cycle (see [Top-level fields](crate::check::toplevel#top-level-fields)).
    ready: usize,
    /// Why a leaf cannot be computed before `ready`, if it reads something
    /// that stops it; `None` on a cycle, which is reported once.
    blocker: Option<Blocker>,
    /// Whether it is on a cycle through fields, already reported: it is
    /// erroneous for its users.
    cyclic: bool,
}

impl FieldCell {
    /// A node's cell before the input is walked: no slot, binding or type
    /// yet, and nothing known that stops it.
    const NEW: Self = Self {
        slot: Slot::NONE,
        binding: None,
        ty: None,
        ready: 0,
        blocker: None,
        cyclic: false,
    };
}

/// Where the check of a top-level function's body is (see [Top-level
/// functions](crate::check::toplevel#top-level-functions)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyState {
    /// Not checked yet: a use sees the declared type.
    Unchecked,
    /// Its group's bodies are being checked: a use from inside the group is
    /// treated as a recursive call through a function's own name is.
    Checking,
    /// Checked: a use sees the function fully resolved.
    Done,
}

/// One top-level function of the input being checked.
#[derive(Clone, Copy, Debug)]
struct TopFn {
    /// Its item, `fn NAME ...`, `def NAME ...` or `def PATH ...`.
    item: ItemId,
    /// Its function expression.
    expr: ExprId,
    /// Its function.
    function: FnId,
    /// Its name; `None` for a `def` named by a dotted path, which has no
    /// binding (see [Top-level functions](crate::check::toplevel#top-level-functions)).
    name: Option<Name>,
    /// Its binding's index in `bindings`, declared as the input starts;
    /// `usize::MAX` for a `def` named by a dotted path, and, until
    /// `Checker::declare_shadowing_fns` declares it, for a `fn` that
    /// shadows a field root.
    binding: usize,
    /// For a `def` named by a dotted path, its leaf of the tree of
    /// top-level fields, whose cell holds its closure and its type: the
    /// type its signature declares, then, once its group is checked, the
    /// type its body gives it, as a plain function's binding has.
    leaf: Option<u32>,
    /// For a `def` named by a dotted path, once the walk has reached it,
    /// the walk's node of its leaf in the record of the input, which takes
    /// its type once its group is checked (see `Checker::nodes`); `NO_NODE`
    /// until then.
    node: u32,
    /// The number of bindings declared when the walk reached its
    /// declaration, once it has: the bindings its body sees.
    cut: Option<usize>,
    /// How far its body's check is.
    state: BodyState,
    /// Once its body is checked, the position among the outermost block's
    /// items of the first item before which its closure exists: `0` if it
    /// copies nothing of this input's top level, `usize::MAX` if its body
    /// read a name it cannot see yet (see [When a function may be
    /// used](crate::check::schedule#when-a-function-may-be-used)).
    ready: usize,
    /// Why it does not exist before `ready`, if it does not exist from the
    /// start.
    blocker: Option<Blocker>,
    /// Where its closure is made.
    made: MadeAt,
}

/// Why a top-level function does not exist before some item: what it
/// reads, and where that is bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Blocker {
    /// The name it reads.
    name: Name,
    /// The declaration labelled: the item binding the name, the spread
    /// that may define it, or, for a mutable variable, the function's own.
    span: Span,
    /// Which of those it is.
    kind: BlockerKind,
}

/// What a [`Blocker`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockerKind {
    /// A variable or field that is not mutable, bound by an item.
    Bound,
    /// A mutable variable, which the function copies where it is written.
    Mutable,
    /// A name a top-level spread may define.
    Spread,
}

/// Where the closure of a top-level function is made and stored in its
/// slot (see [When a function may be used](crate::check::schedule#when-a-function-may-be-used)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadeAt {
    /// As the input starts, before its first item: it copies nothing of
    /// the input's top level.
    Start,
    /// Just after the item given, the one that binds the last of the
    /// variables and fields it copies.
    After(ItemId),
    /// At its own declaration: it copies a mutable variable, or it is
    /// erroneous.
    Own,
}

/// A function expression or a `lazy` being checked: one entry of the
/// checker's stack of the functions it is inside, innermost last.
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
    /// What the function may do, as declared: pure (`:`), memoised (`->>`)
    /// or impure (`->`); always impure for a `lazy`. Only
    /// [`Effect::Pure`] is pure.
    effect: Effect,
    /// Its `:`, `->>` or `->`, or a `lazy`'s keyword.
    marker: Span,
    /// Its `fn` or `lazy` keyword, where a `break` or `continue` that would
    /// leave it points.
    keyword: Span,
    /// For each parameter, where the body first measures it, in source
    /// order, so far; only a pure function's bare parameters, whose types
    /// are provisional, are ever measured. Empty for a `lazy`.
    demanded: Vec<Option<Span>>,
    /// The declared result type, as resolved, and where it is written,
    /// against which every `return` value is checked as the body's result
    /// is; `None` for a `lazy`, which declares none.
    result: Option<(TyId, Span)>,
    /// The `return`s for it so far, in the order they are checked: its
    /// result sites besides the body's own result (see
    /// [`return`](crate::check::loops#return)).
    returns: Vec<Site>,
}

/// What the checker keeps while default functions are checked (see
/// [Calls](#calls)).
#[derive(Debug, Default)]
struct DefaultsChecked {
    /// The copies of the default functions of the top-level function being
    /// checked, which are its own too: its closure keeps their closures, so
    /// it is made where they can be.
    copies: Vec<usize>,
    /// While a parameter's default function is checked, that parameter and
    /// the ones after it, by name and where each is written, and those of
    /// each default it is in: what the default cannot see, which an
    /// "undefined variable" error of one of their names points at.
    unseen: Vec<(Name, Span)>,
}

/// One value that a `return` gives its target: a result site of a
/// function, a `lazy`, a labelled block or a loop (see
/// [`return`](crate::check::loops#return)).
#[derive(Clone, Copy, Debug)]
struct Site {
    /// The value, or `None` for a `return` without one, which gives `{}`.
    value: Option<ExprId>,
    /// Where it is shown: the value, or the `return` and its label.
    at: Span,
    /// Its type: with an expected type, what it is once the awaits marked
    /// on it have run (see [`Checker::fit`]), and its own type otherwise.
    ty: TyId,
}

/// Where an await is decided, for the purity rule's error in a pure
/// function: see `Checker::await_here`.
#[derive(Clone, Copy, Debug)]
enum AwaitSite {
    /// Implicitly, where a value is used as its bits.
    Implicit,
    /// Explicitly, by an `await` whose keyword is at the span.
    Explicit(Span),
    /// Implicitly, at the top, as an argument of a call that fills a
    /// parameter the callee's body measures.
    Argument(CallSite),
}

/// An argument awaited because the parameter it fills is present by
/// inference, which the call-site form of the implicit-await error
/// describes.
#[derive(Clone, Copy, Debug)]
struct CallSite {
    /// The call.
    call: ExprId,
    /// Its callee.
    callee: ExprId,
    /// Where the callee's body first measures the parameter, and the
    /// parameter's name, if the callee's function is known: one of this
    /// input's function expressions.
    measured: Option<(Span, Name)>,
}

/// A callee whose function is known well enough to tell which of its
/// parameters are present because its body measures them, and which have
/// defaults (see [Calls](#calls)).
#[derive(Clone, Copy, Debug)]
enum KnownCallee {
    /// A function expression of the input being checked.
    Current(FnId),
    /// A binding of an earlier session input, by its index in
    /// `Checker::bindings`, which is also its index in
    /// `Checker::scope_measured`.
    Earlier(usize),
    /// A `def` of another file reached through an import, by its merged
    /// node in the workspace (see [Imports](crate::check::imports#imports)), whose measured
    /// parameters the workspace context carries.
    Imported(u32),
    /// A method of another file, by its name and its position among those
    /// of the name the file sees ([`Context::methods`]).
    Method(Name, u32),
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

/// A callback parameter of an impure or memoised function being checked,
/// in `Checker::callbacks`: what its body does with it so far, from which
/// its type is narrowed when the body ends. See the module documentation.
#[derive(Clone, Copy, Debug)]
struct Callback {
    /// The index of the parameter among its function's parameters.
    param: usize,
    /// Where the body first uses it as a value, other than as the callee of
    /// a call, in source order, so far.
    used: Option<Span>,
    /// The index in `Checker::callback_positions` of the first of its
    /// positions, one for each parameter of its function type, in order.
    positions: usize,
    /// The number of its positions.
    count: usize,
}

/// One parameter position of a callback parameter's function type, in
/// `Checker::callback_positions`.
#[derive(Clone, Copy, Debug)]
struct CallbackPosition {
    /// For a narrowable position, one written bare, the type expression it
    /// is written as and its present type, what it is narrowed to; `None`
    /// for any other.
    narrowable: Option<(TypeExprId, TyId)>,
    /// Where a call of the callback first gives the position an argument
    /// that does not fit its present type, in source order, so far.
    given: Option<Span>,
}

/// A field declared in the record block being checked. Its place in the
/// block's fields is where it was first defined, which an override keeps.
#[derive(Clone, Copy, Debug)]
struct FieldDecl {
    name: Name,
    /// Where the name is written in the field's first definition, or in
    /// the field item that overrode it, if one did; for a soft field, the
    /// operand of the spread whose definition stands.
    name_span: Span,
    ty: TyId,
    /// Its one slot, which every definition of it writes.
    slot: Slot,
    /// For an open field, one first defined by a dotted path, where its
    /// segments are tracked; `None` for a field defined whole.
    open: Option<OpenField>,
    /// For a soft field, one a spread defines, which a later item may
    /// define again, the operand of the spread whose definition stands so
    /// far; `None` for a hard field, one a field item defines.
    spread: Option<ExprId>,
    /// Whether a dotted path under it was rejected, which makes its block's
    /// record erroneous, so that the rejection does not cascade.
    rejected: bool,
}

/// What the path of a def-type reference names in its def-type's namespace
/// (see [Def-types](crate::check::deftypes#def-types)).
#[derive(Clone, Debug, PartialEq, Eq)]
enum DefinitionAt {
    /// A definition.
    Leaf,
    /// A definition at a prefix of the path, whose read is this prefix of
    /// the reference's read, the rest of the path being read from its meta
    /// (see [Metas](crate::check::attributes#metas)): only for a def-type that declares one.
    Past(ExprId),
    /// A node above definitions, no definition itself.
    Interior,
    /// Nothing: with the paths of the definitions of the namespace the file
    /// sees, as written after the def-type.
    Missing(Vec<String>),
}

/// What a read of a meta reads it of, as its errors show it (see
/// [Metas](crate::check::attributes#metas)); `Checker::meta_of` says what each is.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MetaOf {
    /// "the meta of `func`".
    subject: String,
    /// Where the function is written.
    func: Span,
    /// Where the read is written.
    read: Span,
    /// For a def-type reference's path past its definition, the reference
    /// to the definition, `package dev.peios.testpkg`.
    past: Option<String>,
}

/// What the spread items of the innermost block checked so far have done,
/// in `Checker::spreads`. See the module documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BlockSpreads {
    /// The block has none yet, so every field of it is hard, and a field
    /// item need not look for a soft field of its name.
    #[default]
    None,
    /// It has some, and none was rejected.
    Some,
    /// It has some, and one was rejected: its operand is erroneous or not a
    /// record, or it defines a field defined before it. The block's record
    /// is erroneous, so that the rejection does not cascade.
    Rejected,
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

/// The record type a record block is checked against, or the list type the
/// result of a block that is not one is checked against, or the record type
/// with defaults a record literal it results in is (see
/// [`Expecting::literal_only`]), or that the elements of a list literal it
/// results in are (see [`Expecting::elements_only`]), and where the
/// expectation comes from, for [`Checker::check_expr_expecting`].
#[derive(Clone, Copy, Debug)]
struct Expecting<'w> {
    /// The expected record type, present; or the expected list type or
    /// union, as it is expected, which may be its either shape.
    ty: TyId,
    /// The annotation, ascription, return type or callee stating it.
    context: Span,
    /// The label on `context`.
    why: &'w str,
    /// The block's closing brace, where a missing field is reported.
    close: Span,
    /// For a record literal, a record block or `{}`, whose expected record
    /// type is what is expected itself, once stripped, and not the one
    /// record member of a union: its id, by which the defaulted fields it
    /// lacks are recorded as given their defaults (see [Field
    /// defaults](#field-defaults)).
    fill: Option<ExprId>,
    /// For the result of a block that is not a record block: what of it
    /// `ty` reaches (see [`Reach`]).
    reach: Reach,
}

/// What the type an [`Expecting`] carries reaches of the result of a block
/// that is not a record block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// The whole result, which is checked against the type
    /// ([`Checker::fit`]).
    Whole,
    /// Only a record literal the result is, through blocks and the
    /// branches of an `if` with `else`, the type being a record type with
    /// defaults, to give it the defaults it lacks, the result being checked
    /// otherwise as it is with nothing expected (see
    /// [`Checker::fill_result`]).
    Literal,
    /// Only the elements of a list literal the result is, or the body of a
    /// `for` it is, through blocks and the branches of an `if` with `else`,
    /// the type being a record type with defaults that the elements are
    /// expected to be of, to give a record literal among them the defaults
    /// it lacks, the result being checked otherwise as it is with nothing
    /// expected (see [`Checker::fill_elements`]).
    Elements,
}

impl<'w> Expecting<'w> {
    /// The expectation of a value of type `ty`, stated at `context`,
    /// labelled `why`, of the block spanning `block`, which fills nothing.
    const fn new(ty: TyId, (context, why): (Span, &'w str), block: Span) -> Self {
        Self {
            ty,
            context,
            why,
            close: Span::new(block.end.saturating_sub(1), block.end),
            fill: None,
            reach: Reach::Whole,
        }
    }

    /// The expectation of the record type with defaults `record`, stated at
    /// `context`, labelled `why`, of the result of the block spanning
    /// `block`, which is not a record block, that reaches only a record
    /// literal the result is (see [`Reach::Literal`]).
    const fn literal_only(record: TyId, (context, why): (Span, &'w str), block: Span) -> Self {
        Self {
            reach: Reach::Literal,
            ..Self::new(record, (context, why), block)
        }
    }

    /// The expectation of the record type with defaults `record`, stated at
    /// `context`, labelled `why`, of the elements of the result of the
    /// block spanning `block`, which is not a record block, that reaches
    /// only the elements of a list literal the result is, or the body of a
    /// `for` it is (see [`Reach::Elements`]).
    const fn elements_only(record: TyId, (context, why): (Span, &'w str), block: Span) -> Self {
        Self {
            reach: Reach::Elements,
            ..Self::new(record, (context, why), block)
        }
    }
}

/// A type expected of a value, with where that is stated and its label.
type Stated<'w> = (TyId, (Span, &'w str));

/// What is expected of each branch of an `if`, for [`Checker::check_if`]
/// and [`Checker::check_branch`].
#[derive(Clone, Copy, Debug)]
enum Branch<'w> {
    /// Nothing: the branch is checked bottom-up.
    Free,
    /// A type, with where it is stated and its label, which the branch is
    /// checked against ([`Checker::fit`]).
    Fits(TyId, (Span, &'w str)),
    /// A record type with defaults, with where it is stated and its label,
    /// which reaches only a record literal the branch results in
    /// ([`Checker::fill_result`]).
    Fills(TyId, (Span, &'w str)),
    /// A record type with defaults that the elements of the branch, a list,
    /// are expected to be of, with where that is stated and its label,
    /// which reaches only the elements of a list literal the branch results
    /// in, or the body of a `for` it does ([`Checker::fill_elements`]).
    FillsElements(TyId, (Span, &'w str)),
}

/// What [`Checker::check_test`] found of an `is` or `as` that is not
/// erroneous. See the module documentation.
#[derive(Clone, Copy, Debug)]
struct Tested {
    /// The narrowed type: what the value is when the test passes.
    narrowed: TyId,
    /// The remainder: what the value is when the test fails; `None` if it
    /// always passes.
    remainder: Option<TyId>,
    /// Whether the remainder is the whole of the value's type: whether no
    /// member of it always passes.
    whole: bool,
    /// Whether the operand is marked to be awaited, which makes the value
    /// tested another than the operand's.
    awaited: bool,
    /// Whether the operand is of type `Never`, so the test is never run:
    /// the expression is `Never` too, nothing is planned and nothing
    /// narrows.
    unreached: bool,
}

/// A loop whose body, or a labelled block whose items, are being checked,
/// in `Checker::loops`, innermost last: what a `break`, `continue` or
/// `return` in it needs to find it, and what they give it (see [`break`
/// and `continue`](crate::check::loops#break-and-continue), [Labels](crate::check::loops#labels) and
/// [`return`](crate::check::loops#return)). An unlabelled block has none.
#[derive(Clone, Debug)]
struct LoopFrame {
    /// The loop, or `None` for a labelled block.
    id: Option<ForId>,
    /// The loop or block expression.
    expr: ExprId,
    /// Its label, if it has one; a block's always has one.
    label: Option<Label>,
    /// The number of functions being checked when its body began: a
    /// `break` in a function or `lazy` nested in the body sees more, and
    /// cannot leave it for this loop.
    depth: usize,
    /// The element type expected of the loop, if a list type is, with where
    /// that is stated and its label: the value of each `break` and
    /// `continue` for it is checked against it.
    expected: Option<(TyId, Span, String)>,
    /// The value of each `break` and `continue` for it, in the order they
    /// are checked, with what it is: its type, or with an expected type,
    /// what it is once the awaits marked on it have run.
    values: Vec<(ExprId, TyId)>,
    /// Whether a `break` is for it, which a loop with no condition needs
    /// to end at all (see [Never-ending loops](crate::check::loops#never-ending-loops)).
    broken: bool,
    /// The type expected of the whole loop or block, if one is, with where
    /// that is stated and its label: the value of each `return` for it is
    /// checked against it.
    returns_expected: Option<(TyId, Span, String)>,
    /// The `return`s for it so far, in the order they are checked.
    returns: Vec<Site>,
}

impl LoopFrame {
    /// The frame of the loop `id`, the expression `expr`, labelled `label`
    /// if it has one, in the body of the function at `depth`, of which the
    /// element type `expected` is expected, and the type `returns_expected`
    /// as a whole, if they are.
    fn new(
        (id, expr): (Option<ForId>, ExprId),
        label: Option<Label>,
        depth: usize,
        expected: Option<(TyId, Span, String)>,
        returns_expected: Option<(TyId, Span, String)>,
    ) -> Self {
        Self {
            id,
            expr,
            label,
            depth,
            expected,
            values: Vec::new(),
            broken: false,
            returns_expected,
            returns: Vec::new(),
        }
    }

    /// The target that a `break`, `continue` or `return` for it has.
    const fn target(&self) -> JumpTarget {
        match self.id {
            Some(id) => JumpTarget::Loop(id),
            None => JumpTarget::Block(self.expr),
        }
    }

    /// What it is, as diagnostics name it: "loop" or "block".
    const fn noun(&self) -> &'static str {
        match self.id {
            Some(_) => "loop",
            None => "block",
        }
    }
}

/// The bindings an `if` whose condition narrows a variable gives its
/// branches, and the slot they share. See the module documentation.
#[derive(Clone, Copy, Debug, Default)]
struct Narrowing {
    /// The `then` branch's binding.
    then: Option<Binding>,
    /// The `else` branch's binding, if it has one.
    else_: Option<Binding>,
    /// The next free slot before the slot the bindings share was taken,
    /// to which it goes back once the `if` is checked; `None` if the `if`
    /// does not narrow.
    next_slot: Option<u32>,
}

/// What an impure function's body is known to return at one position of
/// its result type, once every await the checker marked for fitting it to
/// the declared result has run, for [`Checker::inferred`]. See the module
/// documentation.
#[derive(Clone, Copy, Debug)]
enum BodyValue {
    /// A value that certainly has this type.
    Typed(TyId),
    /// The present record that a record block checked field by field
    /// against the declared record type builds: the block's items, whose
    /// fields' values say what the record holds.
    Fields(ItemRange),
    /// The present record that the dotted paths of such a block build at
    /// an open field or a segment of one: the index of its node in
    /// `Checker::built`, whose leaves' values say what the record holds.
    Open(u32),
}

/// One segment of an open field of a record block that an impure
/// function's body returns, rebuilt from the block's dotted paths for
/// [`Checker::inferred`], since the block's own [`Node`]s are gone once it
/// ends. Its children are a linked list through `first_child` and
/// `next_sibling`, most recent first, as a [`Node`]'s are.
#[derive(Clone, Copy, Debug)]
struct Built {
    name: Name,
    /// For the end of a dotted path, the value it is defined whole with;
    /// `None` for a record the paths build.
    leaf: Option<ExprId>,
    first_child: u32,
    next_sibling: u32,
}

/// How a field of a record block that an impure function's body returns
/// is defined, for [`Checker::infer_field`].
#[derive(Clone, Copy, Debug)]
enum Definition {
    /// Whole, with this value: a field, or the end of a dotted path.
    Whole(ExprId),
    /// Softly, by a spread, whose operand gives the field this type: the
    /// type of the value the spread reads from the operand and stores, as
    /// it is.
    Soft(TyId),
    /// By dotted paths: an open field or a segment of one, the index of
    /// its node in `Checker::built`.
    Dotted(u32),
}

/// The variables of an outermost scope, with their slots and types: what a
/// [`crate::session::Session`] carries from one input to the next.
///
/// Only a [`Checker`] produces a non-empty scope. Its slots are `0..n`, `n`
/// being its first free slot, which it keeps explicitly (`slot_names`) since
/// not every slot is a binding's: the outermost block's declarations are the
/// first to take slots and no block is left open around them, and a slot
/// that no name finds, such as a cell of a top-level dotted field, is
/// committed with them. The bindings occupy their slots in declaration
/// order; the definitions of a field that overrides a soft one share the
/// field's slot, so `n` is not the number of bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    bindings: Vec<Binding>,
    /// The name each slot `0..n` shows in a dump, `n` being the first slot
    /// the next input may take: the last binding in it, or for a slot no
    /// binding owns, what the checker named it.
    slot_names: Vec<Name>,
    /// For each binding, if it was initialised directly with a function
    /// expression, what a call of it needs to know of that function's
    /// parameters (see [`KnownParams`]): which are present because its body
    /// measures them, which the call-site form of the implicit-await error
    /// needs, and their defaults, of a function declared by an earlier
    /// input, whose tree and spans are gone.
    known: Vec<Option<Rc<KnownParams>>>,
}

impl Scope {
    /// An empty scope, as at the start of a file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of variables declared, counting shadowed ones and every
    /// definition of a field.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether no variable has been declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The slots of the scope's mutable variables, in declaration order. A
    /// later input checked from the scope can write no other of its slots:
    /// it may rebind a mutable variable from any block of its top level,
    /// and every declaration it makes takes a slot of its own, after the
    /// scope's.
    pub fn mutable_slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.bindings
            .iter()
            .filter(|binding| binding.kind == BindingKind::Mutable)
            .map(|binding| binding.slot)
    }

    /// A scope with the variables of each of `parts` in turn that its
    /// indices pick, in their order, whose first free slot is
    /// `slot_names.len()`, its slots named by `slot_names`: what a file of a
    /// workspace is checked from, the declarations of its directory chain
    /// of the names it spells (see [Directory scope](crate::check::imports#directory-scope)).
    pub(crate) fn joined<'s>(
        parts: impl Iterator<Item = (&'s Scope, Vec<usize>)>,
        slot_names: Vec<Name>,
    ) -> Self {
        let mut scope = Self {
            slot_names,
            ..Self::default()
        };
        for (part, picked) in parts {
            for index in picked {
                if let (Some(&binding), Some(known)) =
                    (part.bindings.get(index), part.known.get(index))
                {
                    scope.bindings.push(binding);
                    scope.known.push(known.clone());
                }
            }
        }
        scope
    }

    /// The name of each of the scope's variables, in declaration order.
    pub(crate) fn names(&self) -> impl Iterator<Item = Name> + '_ {
        self.bindings.iter().map(|binding| binding.name)
    }

    /// Adds the fields of `from` after its first `start` whose names `keep`
    /// accepts, without their slots' names: a file's declarations, committed
    /// to its directory's scope. A variable is never one, even of such a
    /// name: a top-level `let` or `fn` that shadows a shadowable field of its
    /// file is the file's alone (see [Shadowed
    /// declarations](crate::check::items#shadowed-declarations)).
    pub(crate) fn commit(&mut self, from: &Scope, start: usize, keep: impl Fn(Name) -> bool) {
        let bindings = from.bindings.iter().zip(&from.known).skip(start);
        for (binding, known) in bindings {
            if binding.kind == BindingKind::Field && keep(binding.name) {
                self.bindings.push(*binding);
                self.known.push(known.clone());
            }
        }
    }

    /// Makes each variable from the `start`th on whose name `poisoned`
    /// accepts of type `Error`, with nothing known of a function it holds:
    /// a declaration of a workspace that the scan found conflicting, which
    /// the files checked after it read silently as erroneous (see
    /// [`crate::workspace`], "Scan errors").
    pub(crate) fn poison_from(&mut self, start: usize, poisoned: impl Fn(Name) -> bool) {
        let bindings = self.bindings.iter_mut().zip(&mut self.known).skip(start);
        for (binding, known) in bindings {
            if poisoned(binding.name) {
                binding.ty = TyId::ERROR;
                binding.function = None;
                *known = None;
            }
        }
    }

    /// The names of the scope's slots, `0..n`, `n` being its first free
    /// slot.
    pub(crate) fn into_slot_names(self) -> Vec<Name> {
        self.slot_names
    }

    /// The scope's first free slot.
    pub(crate) fn slot_count(&self) -> usize {
        self.slot_names.len()
    }

    /// The scope with no variables whose slots `0..n` are named by `names`:
    /// what a session's first input is checked from, the standard
    /// library's slots taken (see [`crate::session`]).
    pub(crate) fn of_slots(names: Vec<Name>) -> Self {
        Self {
            slot_names: names,
            ..Self::default()
        }
    }

    /// The scope with slots named by `names` added after its own: the cells
    /// of a session's input, which its check finds taken (see
    /// [`crate::workspace::check_input`]).
    pub(crate) fn with_slots(mut self, names: impl IntoIterator<Item = Name>) -> Self {
        self.slot_names.extend(names);
        self
    }

    /// The scope with `fields` added after its own variables, each a
    /// field of a name, read from a slot the scope's slots already include,
    /// with a type, and, for a `def`'s cell, what a call of it needs to know
    /// of its parameters: cells of a workspace, under names no source can
    /// write, which the call a workspace makes to invoke a definition by
    /// name reads, and so calls the def-type as a known callee (see
    /// [`crate::workspace`], "Def-types by name").
    pub(crate) fn with_fields(
        mut self,
        fields: impl IntoIterator<Item = (Name, Slot, TyId, Option<Rc<KnownParams>>)>,
    ) -> Self {
        for (name, slot, ty, known) in fields {
            self.bindings.push(Binding {
                name,
                slot,
                ty,
                declared: None,
                kind: BindingKind::Field,
                function: None,
                callback: None,
            });
            self.known.push(known);
        }
        self
    }

    /// The slot, the type and what a call of its function needs to know of
    /// its parameters of the latest binding of `name`, if there is one: a
    /// method's, which another file calls (see [Methods](crate::check::methods#methods)).
    pub(crate) fn function_binding(
        &self,
        name: Name,
    ) -> Option<(Slot, TyId, Option<Rc<KnownParams>>)> {
        let (binding, known) = self
            .bindings
            .iter()
            .zip(&self.known)
            .rev()
            .find(|(binding, _)| binding.name == name)?;
        Some((binding.slot, binding.ty, known.clone()))
    }

    /// For each slot bound, what a call of the function its latest binding
    /// holds needs to know of its parameters, if that binding was
    /// initialised directly with a function expression: what a call of an
    /// imported `def` needs (see [Imports](crate::check::imports#imports)). One map for every
    /// slot, so that a file's cells are looked up in it in constant time
    /// each.
    pub(crate) fn known_by_slot(&self) -> HashMap<Slot, Option<&Rc<KnownParams>>> {
        self.bindings
            .iter()
            .zip(&self.known)
            .map(|(binding, known)| (binding.slot, known.as_ref()))
            .collect()
    }
}

/// Type-checks and resolves `ast` as a file, starting from an empty scope,
/// interning its types into `types`. On success, returns the tables and the
/// warnings found, in source order. The file is checked with no standard
/// library: it knows no name it does not declare, `len` among them, and
/// imports nothing; [`crate::run`] checks a file beside it.
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

/// Checks the `type` items of `ast`, a file of a workspace whose types are
/// expanded ([`Ast::expand_types`]), interning types into `types`: what is
/// wrong in each declaration, reported once, here, before any file is
/// checked, since a file may use a type of a file checked after it (see
/// [Type declarations](crate::check::resolve#type-declarations)). Returns the errors, none for a
/// file with no `type` item. If the file's view `sees_broken`, a file with
/// syntax errors, an unknown type is none of them (see
/// [`Checker::unresolved`]).
pub(crate) fn check_types(ast: &Ast, types: &mut TypeTable, sees_broken: bool) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    checker.quiet.sees_broken = sees_broken;
    checker.type_items();
    checker.errors
}

/// Resolves the bounds of the enums of `ast`, each by its item and its key
/// (see [Enums](crate::check::enums#enums)), interning into `types`, which records each: one
/// erroneous is no bound, its error reported. Returns the errors found.
pub(crate) fn check_enum_bounds(
    ast: &Ast,
    types: &mut TypeTable,
    enums: &[(ItemId, Name)],
) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    for &(item, key) in enums {
        let ItemKind::Enum { bound, .. } = ast.item(item).kind else {
            continue;
        };
        let bound = bound
            .map(|bound| checker.resolve_type(bound, Bareness::Present))
            .filter(|&bound| bound != TyId::ERROR);
        checker.types.set_enum_bound(key, bound);
    }
    checker.errors
}

/// Resolves the declared types of the contexts of `ast`, each by its
/// position among the file's contexts and its declaration's key (see
/// [Context](crate::check::context#context)), interning into `types`, which records each, `Error`
/// for one in error, its error reported. Returns the errors found.
pub(crate) fn check_context_types(
    ast: &Ast,
    types: &mut TypeTable,
    contexts: &[(u32, Name)],
) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    for &(decl, key) in contexts {
        let Some(context) = ast.contexts().get(decl as usize) else {
            continue;
        };
        let ty = checker.resolve_type(context.ty, Bareness::Present);
        checker.types.set_context_type(key, ty);
    }
    checker.errors
}

/// A variant's item, its enum's key and name, if what it names is an enum,
/// and its own key (see [Enums](crate::check::enums#enums)).
pub(crate) type VariantItem = (ItemId, Option<(Name, Name)>, Name);

/// Resolves the payloads of the variants of `ast`, each by its item, its
/// enum's key and name, if what it names is an enum, and its own key (see
/// [Enums](crate::check::enums#enums)), every enum's bound being known: each payload must fit
/// its enum's bound, "the payload of `Toolchain::C` must fit `{*: File |
/// Dir}`"; a variant of an enum one earlier of the file has the name of is
/// "variant `Toolchain::C` is declared twice", and a variant of what is no
/// enum "`Int` is not an enum". Records each payload, `Error` for one in
/// error, in `types`. Returns the errors found.
pub(crate) fn check_variant_payloads(
    ast: &Ast,
    types: &mut TypeTable,
    variants: &[VariantItem],
) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    let mut declared: Vec<(Name, Name, Span)> = Vec::new();
    for &(item, enum_of, key) in variants {
        let ItemKind::Variant {
            of,
            name,
            name_span,
            payload,
        } = ast.item(item).kind
        else {
            continue;
        };
        let Some((enum_key, enum_name)) = enum_of else {
            let ty = checker.resolve_type(of, Bareness::Present);
            if ty != TyId::ERROR {
                let shown = checker.in_message(ty);
                checker.errors.push(Diagnostic::error(
                    format!("{shown} is not an enum"),
                    ast.type_expr(of).span,
                ));
            }
            continue;
        };
        let names = ast.names();
        let shown = format!("{}::{}", names.get(enum_name), names.get(name));
        if let Some(&(_, _, first)) = declared
            .iter()
            .find(|&&(of, earlier, _)| of == enum_key && earlier == name)
        {
            checker.errors.push(
                Diagnostic::error(format!("variant `{shown}` is declared twice"), name_span)
                    .with_context(first, "first declared here"),
            );
            continue;
        }
        declared.push((enum_key, name, name_span));
        let ty = match payload {
            Some(payload) => checker.resolve_type(payload, Bareness::Present),
            None => TyId::EMPTY,
        };
        let ty = match checker.types.enum_bound(enum_key) {
            Some(bound) if ty != TyId::ERROR && !checker.types.fits_bound(ty, bound) => {
                let (found, bound_shown) = (checker.in_message(ty), checker.in_message(bound));
                let at = payload.map_or(name_span, |payload| ast.type_expr(payload).span);
                checker.errors.push(
                    Diagnostic::error(
                        format!("the payload of `{shown}` must fit {bound_shown}"),
                        at,
                    )
                    .with_label(format!("this is {found}"))
                    .with_note(format!(
                        "`{}` bounds its variants' payloads as it is declared",
                        names.get(enum_name)
                    )),
                );
                TyId::ERROR
            }
            _ => ty,
        };
        checker.types.set_variant_payload(key, ty);
    }
    checker.errors
}

/// The names of the built-in types: those of [`BuiltinTypes`], and the
/// type constructors, `List`, `Future` and `Present`. A `type` may not be
/// named like one: "`Int` is a built-in type".
pub(crate) const BUILTIN_TYPE_NAMES: [&str; 12] = [
    "Int", "Float", "Bool", "String", "Duration", "Size", "List", "Future", "Present", "Variant",
    "Partial", "Secret",
];

/// The name of the checker's attribute that retries a call.
const RETRY: &str = "retry";

/// The name of the checker's attribute that limits a call's time.
const TIMEOUT: &str = "timeout";

/// The error for an `#[attribute]` type that is no record of constants.
const NOT_CONSTANT_RECORD: &str = "an attribute's arguments are a record of constants";

/// The note on [`NOT_CONSTANT_RECORD`].
const CONSTANT_FIELDS_NOTE: &str = "each field must be an `Int`, `Float`, `Bool`, `String`, \
     `Duration` or `Size`, or a list or record of them";

/// The fields of an attribute's record, by name, in its order, each with
/// its type and default, if it has one (see [Attributes](crate::check::attributes#attributes)).
type AttributeFields = Vec<(Box<str>, TyId, Option<Constant>)>;

/// The arguments of `args`, by their values, each with the parameter of
/// `sig` it fills, as [`Checker::bind_arguments`] binds them: positional
/// ones left to right, the trailing block the last, and named ones the
/// parameters of their names; only the first that fills a parameter, and
/// none that fills nothing.
fn argument_targets(args: &[Arg], sig: &FnTy) -> Vec<(ExprId, usize)> {
    let count = sig.params.len();
    let positional = args
        .iter()
        .filter(|arg| arg.kind == ArgKind::Positional)
        .count();
    let mut next = 0;
    let mut taken = vec![false; count];
    let mut targets = Vec::new();
    for arg in args {
        let param = match arg.kind {
            ArgKind::Positional => {
                next += 1;
                Some(next - 1)
            }
            ArgKind::Trailing => (positional < count).then(|| count - 1),
            ArgKind::Named { name, .. } => sig
                .params
                .iter()
                .position(|&(param, _)| param == Some(name)),
        };
        if let Some(param) = param.filter(|&param| param < count)
            && let Some(slot @ false) = taken.get_mut(param)
        {
            *slot = true;
            targets.push((arg.value, param));
        }
    }
    targets
}

/// The function of the item `item`, a function item that attributes apply
/// to: a named `fn`, a `def` or a definition (see [Attributes](crate::check::attributes#attributes)).
fn item_function(ast: &Ast, item: ItemId) -> Option<FnId> {
    let (ItemKind::Let {
        value,
        function: true,
        ..
    }
    | ItemKind::Field {
        value,
        function: true,
        ..
    }
    | ItemKind::DefTypeDef { value, .. }) = ast.item(item).kind
    else {
        return None;
    };
    match ast.get(value).kind {
        ExprKind::Fn(function) => Some(function),
        _ => None,
    }
}

/// The names of the built-in attributes: the parser's, `shadowable`,
/// `def_type`, `attribute` and `no_prelude`, and the checker's, `retry`
/// and `timeout` (see [Attributes](crate::check::attributes#attributes)). No `#[attribute]` type
/// may be named like one: "`retry` is a built-in attribute".
pub(crate) const BUILTIN_ATTRIBUTES: [&str; 6] = [
    "shadowable",
    "def_type",
    "attribute",
    "no_prelude",
    "retry",
    "timeout",
];

/// The type names every program can use, as `(name, type)` pairs. A name
/// the program never mentions has no [`Name`].
type BuiltinTypes = [(Option<Name>, TyId); 6];

/// The [`BuiltinTypes`] of a program whose names are `names`.
fn builtin_types(names: &Names) -> BuiltinTypes {
    [
        (names.lookup("Int"), TyId::INT),
        (names.lookup("Float"), TyId::FLOAT),
        (names.lookup("Bool"), TyId::BOOL),
        (names.lookup("String"), TyId::STRING),
        (names.lookup("Duration"), TyId::DURATION),
        (names.lookup("Size"), TyId::SIZE),
    ]
}

/// A check in progress. See the module documentation.
pub struct Checker<'a> {
    ast: &'a Ast,
    /// Where types are interned.
    types: &'a mut TypeTable,
    /// The type of each expression, indexed by [`ExprId`].
    expr_types: Vec<TyId>,
    expr_vars: Vec<VarRef>,
    /// Whether and how each expression is awaited, indexed by [`ExprId`].
    awaits: Vec<AwaitMark>,
    item_slots: Vec<Slot>,
    /// See [`Resolution::spread_slots`].
    item_spreads: Vec<(u32, u32)>,
    spread_slots: Vec<Slot>,
    item_paths: Vec<PathResolution>,
    /// Whether each existing record along the path of each dotted `let` or
    /// `:=` with an awaited one is awaited: see [`Resolution::path_awaits`].
    path_awaits: Vec<bool>,
    /// What has been found for each function expression, indexed by
    /// [`FnId`].
    fn_resolutions: Vec<FnResolution>,
    /// The parameter each argument fills, indexed like the argument pool.
    arg_params: Vec<u32>,
    /// The slot of each `for` expression's variable, indexed by [`ForId`].
    for_slots: Vec<Slot>,
    /// What each `break`, `continue` and `return` item is for, indexed by
    /// [`ItemId`]: see [`Resolution::target`].
    jumps: Vec<Option<JumpTarget>>,
    /// The loops whose bodies, and the labelled blocks whose items, are
    /// being checked, outermost first.
    loops: Vec<LoopFrame>,
    /// What was found of each loop, indexed by [`ForId`]: see
    /// [`Resolution::loop_of`].
    loop_resolutions: Vec<LoopResolution>,
    /// The labelled blocks some `return` leaves: see
    /// [`Resolution::is_returned_block`].
    returned_blocks: HashSet<ExprId>,
    /// The calls of a function or built-in that is not pure, by their ids,
    /// which a statement may discard the value of.
    impure_calls: HashSet<ExprId>,
    /// The statements whose values are discarded: see
    /// [`Resolution::is_discarded`].
    discarded: HashSet<ExprId>,
    /// The values of the `return`s for each labelled block that has any,
    /// by the block's id, which an untyped integer pushed into the block
    /// reaches as it reaches the block's result (see
    /// [`Checker::coerce`]).
    block_returns: HashMap<ExprId, Vec<ExprId>>,
    /// `Never`, interned into the table once, as the checker starts.
    never: TyId,
    /// Set by [`Checker::walk_path`] when it stops at a record or list of
    /// type `Never` along a path, which no value has, so that the update is
    /// unreachable and its caller declares quietly, with `Never`, rather
    /// than with `Error`.
    unreached_path: bool,
    /// Every variable, field and callee in scope, outermost block first.
    bindings: Vec<Binding>,
    /// For each binding of the scope the check started from, its entry of
    /// [`Scope`]'s `known`.
    scope_known: Vec<Option<Rc<KnownParams>>>,
    /// What the check gives lowering besides types and slots.
    lowering: ForLowering,
    /// What the check keeps of generic functions, methods and their calls.
    calls: CallsChecked,
    /// The contexts the `with`s around the expression being checked bind,
    /// by their declarations' keys, innermost last (see [Context](crate::check::context#context)).
    with_bound: Vec<Name>,
    /// What the input's top-level items mention, and the groups of its
    /// top-level functions (see [`crate::mentions`]).
    mentions: Mentions,
    /// The workspace the input is a file of, if it is one (see [Directory
    /// scope](crate::check::imports#directory-scope)).
    workspace: Option<&'a Context<'a>>,
    /// Each reference through an import to a merged node, with the node:
    /// a call of one is of a known callee if it is a `def`'s (see
    /// [`KnownCallee::Imported`]).
    imported_callees: HashMap<ExprId, u32>,
    /// The records the file reads whole through its imports, each with its
    /// private slot and fields (see [`Resolution::imported_records`]).
    imported_records: ImportedRecords,
    /// The target of each of `imported_records`, by its index there, with
    /// the record's type.
    imported_targets: Vec<(Target, TyId)>,
    /// See [`Resolution::view_record`]: the record of each node of the
    /// file's view with a leaf of the file under it, by its index among
    /// them.
    view_records: Vec<ViewRecord>,
    /// See [`Resolution::value_cell`].
    value_cell: Slot,
    /// For each node of the input's tree that is a shared interior node of
    /// its directory, the private slot its own record is stored in, for the
    /// input's value; [`Slot::NONE`] for any other node.
    own_records: Vec<Slot>,
    /// The cell of each node of the input's tree of top-level fields, by
    /// its index in `mentions.tree` (see [Top-level
    /// fields](crate::check::toplevel#top-level-fields)).
    field_cells: Vec<FieldCell>,
    /// For each node of the graph of what needs what (`Checker::need_index`),
    /// once known: the latest item before which something it needs, itself
    /// included, cannot be had (`Checker::closure_ready`), 0 if nothing it
    /// needs is ever blocked. Lets `Checker::check_uses` skip the search for
    /// a reason in the common case, so a chain of N fields costs N steps.
    closure_ready: Vec<Option<usize>>,
    /// When each node of the tree is computed.
    schedule: Schedule,
    /// How far `schedule.order` has been checked: the nodes before this
    /// index in it are.
    scheduled: usize,
    /// The end of the bindings of the input's top-level declarations in
    /// `bindings`: from `top_start`, those of its top-level functions, then
    /// those of its top-level field roots that are not `def`s.
    top_end: usize,
    /// The input's top-level functions, indexed as in `mentions`.
    top_fns: Vec<TopFn>,
    /// The index in `bindings` of the first top-level function's binding:
    /// those of `top_fns` follow it, in order.
    top_start: usize,
    /// For each top-level function, once its body is checked, the indexes
    /// in `bindings` of what it copies.
    top_copies: Vec<Vec<usize>>,
    /// What the check of the default functions being checked keeps.
    defaults_checked: DefaultsChecked,
    /// The top-level function whose body is being checked, if one is.
    checking_top: Option<usize>,
    /// The leaf of the tree of top-level fields whose initialiser is being
    /// checked, if one is (see [Top-level fields](crate::check::toplevel#top-level-fields)).
    checking_leaf: Option<u32>,
    /// The top-level functions on a cycle through fields, already reported:
    /// erroneous for their users, as an erroneous function is.
    cyclic_fns: Vec<u32>,
    /// While a top-level function's body is checked before the walk has
    /// reached it: the positions among the outermost block's items of the
    /// item being checked and of the function's declaration. A read of a
    /// name an item between them declares is not resolved (see [When each
    /// body is checked](crate::check::toplevel#top-level-functions)).
    early: Option<(usize, usize)>,
    /// The position among the outermost block's items of the item the walk
    /// is at.
    root_position: usize,
    /// While a block nested in the top level's code (not in a function) is
    /// being checked, the number of the top level's own bindings: those
    /// below are the nested blocks'.
    root_end: Option<usize>,
    /// The functions being checked, innermost last; empty at the top
    /// level.
    functions: Vec<Function>,
    /// Reused for collecting the parameters of provisional types in a type.
    claimed: Vec<u32>,
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
    /// The roots of the namespaces of the def-types the input's
    /// definitions are made with, declared as the walk reaches them, as
    /// `fields` holds the outermost block's: apart from them, since a
    /// namespace is no field of the input's record (see
    /// [Def-types](crate::check::deftypes#def-types)).
    def_fields: Vec<FieldDecl>,
    /// The definition made with a def-type each function expression of the
    /// input's outermost block is, if it is one.
    definitions: HashMap<FnId, DefTypeId>,
    /// The function expressions of the input's `def`s marked `#[def_type]`.
    def_type_fns: HashSet<FnId>,
    /// The definition's meta each function expression of the input's
    /// outermost block is, if it is one (see [Metas](crate::check::attributes#metas)).
    defmetas: HashMap<FnId, DefTypeId>,
    /// See [`Resolution::meta_read`].
    meta_reads: HashMap<ExprId, (TyId, AwaitMark)>,
    /// See [`Resolution::metas`].
    metas: Vec<(FnId, Slot)>,
    /// See [`Types::reference_past`].
    references_past: Vec<(ExprId, ExprId, TyId)>,
    /// While the path of a def-type reference past its definition is read
    /// from the definition's meta: the read of the definition, a prefix of
    /// the reference's read, and the reference, so that the meta's errors
    /// name the reference (see [Metas](crate::check::attributes#metas)).
    reading_past: Option<(ExprId, DefTypeId)>,
    /// What the innermost block's spread items have done so far.
    spreads: BlockSpreads,
    /// The next free slot.
    next_slot: u32,
    /// The highest `next_slot` reached.
    frame_size: u32,
    slot_names: Vec<Name>,
    /// The first slot past the outermost block's slots that no binding
    /// owns, such as the cells of the input's top-level fields, which a
    /// session commits with the input: the [`Scope`]'s first free slot is
    /// at least this.
    top_slots_end: u32,
    /// The last top-level item reported as "`a` is a field of this file",
    /// whose `:=` is then not reported again as a rebind of a field.
    field_clash: Option<ItemId>,
    /// The top-level items a later declaration shadows, each with the
    /// number of bindings declared when the walk passed it: checked after
    /// the last item, as written, in the view of the top level above them
    /// (see [Shadowed declarations](crate::check::items#shadowed-declarations)).
    shadowed_items: Vec<(ItemId, usize)>,
    /// Whether a shadowed declaration is being checked, whose reads of
    /// records it alone would need are erroneous.
    checking_shadowed: bool,
    builtin_types: BuiltinTypes,
    /// The names of the built-in types applied to type arguments.
    constructors: Constructors,
    /// TEMPORARY (alias rule): the expression that may be a reference to a built-in function whose
    /// typing is special while it is checked: the callee of the call being checked, or the
    /// initialiser of the top-level field or `let` of one name being checked (see
    /// [`Checker::builtin_use`]). Replaced when built-in functions get real polymorphic types (see
    /// the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    alias_site: Option<ExprId>,
    /// The callee of the call being checked, while it is checked: where
    /// `fileroot.helper` may stand, which is called and never a value (see
    /// [Imports](crate::check::imports#imports)).
    call_site: Option<ExprId>,
    /// Reused for the types of the elements of the list literals being
    /// checked, innermost literal's last: a stack, as `sorted_fields` is.
    element_types: Vec<TyId>,
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
    /// The segments of the open fields of every record block whose value
    /// an impure function's result is being inferred from, rebuilt from
    /// their dotted paths: a stack, as `sorted_fields` is, each block's
    /// segments above those of the block enclosing it.
    built: Vec<Built>,
    /// For each declared field of every record block with a spread whose
    /// value an impure function's result is being inferred from, the
    /// definition of it that stands so far, from the block's first spread
    /// on: a stack, as `built` is (see [`Checker::infer_block_fields`]).
    standing: Vec<Option<Definition>>,
    /// Reused for the records along the path of a dotted `let` or `:=` that
    /// exist: each one's type once measured, and whether it is awaited. A
    /// `:=` walks its path before its value is checked and declares its
    /// variable after, so an update nested in the value pushes above the
    /// entries of the one enclosing it and truncates back, as with
    /// `sorted_fields`.
    path_types: Vec<(TyId, bool)>,
    /// Reused as the work list of open nodes still to check against their
    /// expected types when a record block ends.
    pending: Vec<(u32, TyId)>,
    /// The callback parameters of every impure or memoised function being
    /// checked, outermost function first; each function truncates it back
    /// when it ends.
    callbacks: Vec<Callback>,
    /// The parameter positions of the function types of `callbacks`, each
    /// callback's together, in the same order.
    callback_positions: Vec<CallbackPosition>,
    /// See [`Resolution::test_of`].
    tests: HashMap<ExprId, TestResolution>,
    /// Whether types are being resolved quietly, and whether one was
    /// erroneous with nothing to report here.
    quiet: Quiet,
    /// The type of each `type` item of the file, for `--dump-types`.
    declared_types: Vec<(ItemId, TyId)>,
    /// What each node of a copy of a declared type's definition denotes in
    /// each mode it has been resolved in (see [`Checker::resolve_type`]).
    copies: HashMap<(TypeExprId, Bareness), TyId>,
    /// What is recorded of the input's names for an index of it, if the
    /// check was asked to record ([`Checker::recording`]); `None`, and so
    /// costing one test per binding and per name read, on every other path
    /// (see [Recording](#recording)).
    record: Option<Box<Record>>,
    /// The roots of the file's view with their ranks, for suggestions (see
    /// [Suggestions](crate::check::names#suggestions)): gathered at the first undefined
    /// variable, never on valid input.
    view_roots: Option<Vec<(Name, u8)>>,
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

    /// A checker for `ast`, a file of the workspace `workspace`, checked
    /// from `scope`, the declarations of its directory chain that earlier
    /// files committed (see [Directory scope](crate::check::imports#directory-scope)).
    #[must_use]
    pub(crate) fn in_workspace(
        ast: &'a Ast,
        scope: Scope,
        types: &'a mut TypeTable,
        workspace: &'a Context<'a>,
    ) -> Self {
        let mut checker = Self::with_scope(ast, scope, types);
        checker.quiet.sees_broken = workspace.sees_broken();
        Self {
            workspace: Some(workspace),
            ..checker
        }
    }

    /// A checker for `ast` whose outermost block starts with the variables
    /// of `scope` already declared. `scope` must come from checking earlier
    /// input interned into the same [`crate::names::Names`] as `ast` and the
    /// same [`TypeTable`] as `types`.
    #[must_use]
    pub fn with_scope(ast: &'a Ast, scope: Scope, types: &'a mut TypeTable) -> Self {
        let names = ast.names();
        // The scope's slots are exactly `0..n`, each named as it was when
        // the scope was made; some, such as a dotted field's cells, belong
        // to no binding.
        let next_slot = u32::try_from(scope.slot_names.len()).unwrap_or(u32::MAX);
        Self {
            ast,
            never: types.never(),
            types,
            // Placeholder only: every node is written by `check_expr` before
            // it is read, and the final invariant check catches any that
            // are not.
            expr_types: vec![TyId::ERROR; ast.len()],
            expr_vars: vec![VarRef::None; ast.len()],
            awaits: vec![AwaitMark::None; ast.len()],
            item_slots: vec![Slot::NONE; ast.item_count()],
            item_spreads: vec![(0, 0); ast.item_count()],
            spread_slots: Vec::new(),
            item_paths: vec![PathResolution::NONE; ast.item_count()],
            path_awaits: Vec::new(),
            fn_resolutions: vec![FnResolution::default(); ast.fn_count()],
            arg_params: vec![u32::MAX; ast.arg_count()],
            for_slots: vec![Slot::NONE; ast.for_count()],
            jumps: vec![None; ast.item_count()],
            loops: Vec::new(),
            loop_resolutions: vec![LoopResolution::default(); ast.for_count()],
            returned_blocks: HashSet::new(),
            impure_calls: HashSet::new(),
            discarded: HashSet::new(),
            block_returns: HashMap::new(),
            unreached_path: false,
            bindings: scope.bindings,
            scope_known: scope.known,
            lowering: ForLowering::default(),
            calls: CallsChecked::default(),
            with_bound: Vec::new(),
            mentions: Mentions::default(),
            workspace: None,
            imported_callees: HashMap::new(),
            imported_records: Vec::new(),
            imported_targets: Vec::new(),
            view_records: Vec::new(),
            value_cell: Slot::NONE,
            own_records: Vec::new(),
            field_cells: Vec::new(),
            closure_ready: Vec::new(),
            schedule: Schedule::default(),
            scheduled: 0,
            top_end: 0,
            top_fns: Vec::new(),
            top_copies: Vec::new(),
            defaults_checked: DefaultsChecked::default(),
            checking_top: None,
            checking_leaf: None,
            cyclic_fns: Vec::new(),
            early: None,
            root_position: 0,
            top_start: 0,
            root_end: None,
            functions: Vec::new(),
            claimed: Vec::new(),
            callee_name: None,
            fn_params: Vec::new(),
            filled: Vec::new(),
            block_start: 0,
            fields: Vec::new(),
            fields_start: 0,
            def_fields: Vec::new(),
            definitions: HashMap::new(),
            def_type_fns: HashSet::new(),
            defmetas: HashMap::new(),
            meta_reads: HashMap::new(),
            metas: Vec::new(),
            references_past: Vec::new(),
            reading_past: None,
            spreads: BlockSpreads::None,
            next_slot,
            frame_size: next_slot,
            slot_names: scope.slot_names,
            top_slots_end: next_slot,
            field_clash: None,
            shadowed_items: Vec::new(),
            checking_shadowed: false,
            builtin_types: builtin_types(names),
            constructors: Constructors::of(names),
            alias_site: None,
            call_site: None,
            element_types: Vec::new(),
            sorted_fields: Vec::new(),
            nodes: Vec::new(),
            built: Vec::new(),
            standing: Vec::new(),
            path_types: Vec::new(),
            pending: Vec::new(),
            callbacks: Vec::new(),
            callback_positions: Vec::new(),
            tests: HashMap::new(),
            quiet: Quiet::default(),
            declared_types: Vec::new(),
            copies: HashMap::new(),
            record: None,
            view_roots: None,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// The checker, recording what each name it reads refers to and what
    /// each binding it declares is, for an index of the input (see
    /// [Recording](#recording)): [`Checker::check_recorded`] returns it.
    #[must_use]
    pub(crate) fn recording(mut self) -> Self {
        self.record = Some(Box::default());
        self
    }

    /// The checker, taking a name that names nothing as no error, as in an
    /// input that sees a file with syntax errors (see
    /// [`Checker::unresolved`]): for a file checked for its index alone,
    /// which needs a file whose check failed and whose declarations it may
    /// read.
    #[must_use]
    pub(crate) fn blind(mut self) -> Self {
        self.quiet.sees_broken = true;
        self
    }

    /// Checks the file's body. On success, returns the tables, the
    /// outermost scope extended with the body's top-level declarations, the
    /// body's type and the warnings.
    ///
    /// # Errors
    ///
    /// As [`check`].
    pub fn check(self) -> Result<CheckOutput, Vec<Diagnostic>> {
        self.check_recorded().0
    }

    /// [`Checker::check`], with what it recorded if it was asked to
    /// ([`Checker::recording`]), whether it succeeded or failed (see
    /// [Recording](#recording)).
    pub(crate) fn check_recorded(
        mut self,
    ) -> (Result<CheckOutput, Vec<Diagnostic>>, Option<Box<Record>>) {
        let root = self.ast.root();
        // A tree with syntax errors is checked for the errors of what was
        // parsed, and is never lowered: its check always fails.
        self.quiet.failed |= self.ast.syntax() != Syntax::Clean;
        // A workspace has checked the file's `type` items already, before
        // any file's check ([`check_types`]): they are resolved quietly.
        self.declare_top_level();
        let ty = self.check_body(root, None);
        // Finalisation: an integer result with no context defaults to Int.
        self.coerce_result(root, TyId::INT);
        let ty = if ty == TyId::UNTYPED_INT {
            TyId::INT
        } else {
            ty
        };
        let record = self.finish_record(ty);
        if !self.errors.is_empty() || self.quiet.failed {
            return (Err(diagnostic::merge(self.errors, self.warnings)), record);
        }
        // In a workspace whose scan found errors, which never runs, a read
        // of what the scan poisoned is erroneous, silently: the check fails
        // quietly (see [`crate::workspace`], "Scan errors").
        if self.workspace.is_some_and(Context::scan_failed) && !self.settled() {
            return (Err(diagnostic::merge(self.errors, self.warnings)), record);
        }
        if !self.settled() {
            let error = InternalError {
                message: "a type table entry is untyped, erroneous or provisional after a \
                          successful check",
                span: self.ast.root_span(),
            };
            return (Err(vec![error.into()]), record);
        }
        let scope_slot_names = self.scope_slot_names();
        let (cell_types, shared_children) = self.cell_tables();
        let known = self.known();
        let field_cells = self.tree_cells();
        let bindings = self.bindings.into_iter().map(Binding::forgotten).collect();
        let top_functions = self
            .top_fns
            .iter()
            .map(|top| (top.item, top.made))
            .collect();
        let field_schedule = FieldSchedule {
            tree: std::mem::take(&mut self.mentions.tree),
            schedule: std::mem::take(&mut self.schedule),
        };
        let checked = Checked {
            types: Types {
                others: OtherTypes {
                    declared: self.declared_types,
                    references_past: self.references_past,
                },
                ..Types::new(self.expr_types)
            },
            resolution: Resolution {
                expr_vars: self.expr_vars,
                awaits: self.awaits,
                item_slots: self.item_slots,
                item_spreads: self.item_spreads,
                spread_slots: self.spread_slots,
                item_paths: self.item_paths,
                path_awaits: self.path_awaits,
                top_functions,
                field_cells,
                own_records: self.own_records,
                shared_children,
                imported_records: self.imported_records,
                view_records: self.view_records,
                value_cell: self.value_cell,
                place: self
                    .workspace
                    .map(|workspace| workspace.place().clone())
                    .unwrap_or_default(),
                field_schedule,
                frame_size: self.frame_size,
                slot_names: self.slot_names,
                functions: self.fn_resolutions,
                arg_params: self.arg_params,
                for_slots: self.for_slots,
                jumps: self.jumps,
                loops: self.loop_resolutions,
                returned_blocks: self.returned_blocks,
                discarded: self.discarded,
                tests: self.tests,
                meta_reads: self.meta_reads,
                metas: self.metas,
                lowered: self.lowering,
            },
        };
        let mut warnings = self.warnings;
        warnings.sort_by_key(|warning| warning.span.start);
        let output = CheckOutput {
            checked,
            scope: Scope {
                bindings,
                slot_names: scope_slot_names,
                known,
            },
            ty,
            cell_types,
            warnings,
        };
        (Ok(output), record)
    }

    /// The cells of the nodes of the input's tree of top-level fields, by
    /// their indices there: the view's are in `view_records`.
    fn tree_cells(&self) -> Vec<Slot> {
        self.field_cells
            .iter()
            .take(self.mentions.tree.nodes.len())
            .map(|cell| cell.slot)
            .collect()
    }

    /// Whether, after a check that found no error, every expression has a
    /// type that is neither untyped, erroneous nor provisional, and every
    /// function's captures are settled: the invariant lowering relies on.
    /// A shadowed declaration's expressions are never lowered, and may read
    /// cells no file this one needs computes, whose types are erroneous
    /// (see [Shadowed declarations](crate::check::items#shadowed-declarations)): they are
    /// exempt.
    fn settled(&self) -> bool {
        let shadowed = self.shadowed_exprs();
        let types = &*self.types;
        let unsettled = |ty: TyId| types.contains_provisional(ty, None);
        let lowered = |at: usize| {
            !shadowed
                .iter()
                .any(|&(first, last)| (first..=last).contains(&at))
        };
        !self.expr_types.iter().enumerate().any(|(at, &ty)| {
            lowered(at)
                && (unsettled(ty) || matches!(types.kind(ty), TyKind::Error | TyKind::UntypedInt))
        }) && !self
            .fn_resolutions
            .iter()
            .any(|function| function.captures.iter().any(|&(_, ty)| unsettled(ty)))
    }

    /// Checks a nested block's body in a scope of its own, returning its
    /// type. With `expecting`, the block is a record block checked against
    /// an expected record type, or its result is checked against what is
    /// expected, as [`Checker::check_body`] says.
    fn check_block(&mut self, body: ItemRange, expecting: Option<Expecting<'_>>) -> TyId {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let outer_spreads = std::mem::take(&mut self.spreads);
        let nodes_start = self.nodes.len();
        let leaves_root = self.functions.is_empty() && self.root_end.is_none();
        if leaves_root {
            self.root_end = Some(self.bindings.len());
        }
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        let ty = self.check_body(body, expecting);
        if leaves_root {
            self.root_end = None;
        }
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        self.nodes.truncate(nodes_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        self.spreads = outer_spreads;
        ty
    }

    /// Checks the items of a body in the current scope, returning its type:
    /// for a record block, the record of its fields; otherwise the type of
    /// its result, the `= EXPR` or final expression statement, or `{}` if
    /// it has none. With `expecting`, a record block is checked against the
    /// expected record type, and a block that is not one has its result
    /// checked against the expected type, which it does only for a list
    /// type or a union (see [`Checker::check_list_expected`]), and has the
    /// type its result has once any await marked on it has run (see
    /// [`Checker::fit`]); or, for a record type with defaults that reaches
    /// only a record literal its result is ([`Expecting::literal_only`]),
    /// or only the elements of a list literal it is
    /// ([`Expecting::elements_only`]), has its result checked so
    /// ([`Checker::fill_result`], [`Checker::fill_elements`]), and the type
    /// its result has. A `break` or `continue` that is its last item
    /// makes it `Never`. The first item after one that never completes is
    /// warned about, "this is never run" (see [Never](crate::check::narrowing#never)).
    fn check_body(&mut self, body: ItemRange, expecting: Option<Expecting<'_>>) -> TyId {
        let items = self.ast.items(body);
        let first_field = items.iter().copied().find(|&id| {
            matches!(
                self.ast.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        });
        let mut result = TyId::EMPTY;
        // Whether this is the input's outermost block, whose top-level
        // functions are order-free.
        let root = body == self.ast.root() && self.functions.is_empty();
        // The first item that never completes, and whether the item after
        // it has been warned about.
        let (mut leaves, mut warned) = (None, false);
        // Whether an item is one the parser could not parse.
        let mut broken = false;
        for (index, &id) in items.iter().enumerate() {
            let next = items.get(index + 1).copied();
            let item = *self.ast.item(id);
            if root {
                self.root_position = index;
                self.compute_fields(index);
                if self.set_aside_shadowed(index, id) || self.reach_top_fn(id) {
                    continue;
                }
                self.check_mentioned(index);
                self.check_uses(index);
                self.check_clash(id);
            }
            // A top-level field computed before its own item, because an
            // item above needed it, does not run here, nor does a `type`:
            // neither is warned about nor counted as never completing.
            let early = root && self.runs_elsewhere(index, item.kind);
            if let (Some(leaves), false, false) = (leaves, warned, early) {
                self.warnings.push(
                    Diagnostic::warning("this is never run", item.span)
                        .with_context(leaves, "this always leaves the block"),
                );
                warned = true;
            }
            // The result of a block that is not a record block, checked
            // against what is expected of the block.
            let result_expected = expecting.filter(|_| next.is_none() && first_field.is_none());
            match item.kind {
                ItemKind::Let {
                    path,
                    ty,
                    value,
                    mutable,
                    ..
                } => {
                    // TEMPORARY (alias rule): a top-level `let` of one name, neither annotated nor
                    // mutable, may bind a built-in function whose typing is special (see the
                    // [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
                    let alias = root && !mutable && ty.is_none() && self.ast.path(path).len() == 1;
                    let outer = alias.then(|| self.alias_site.replace(value));
                    self.check_let(id, path, ty, (value, mutable));
                    if let Some(outer) = outer {
                        self.alias_site = outer;
                    }
                }
                ItemKind::Rebind { path, value } => self.check_rebind(id, path, value),
                ItemKind::Field {
                    path, ty, value, ..
                } => {
                    self.check_field(id, path, (ty, value), expecting);
                }
                ItemKind::Spread(value) => self.check_spread_item(id, value, root),
                ItemKind::Import(_) => self.check_import(id, item.span),
                // A definition, and a meta, is placed as the walk reaches it,
                // whether or not it is a top-level function
                // (`Checker::reach_top_fn`).
                ItemKind::Type { .. }
                | ItemKind::Enum { .. }
                | ItemKind::Variant { .. }
                | ItemKind::Context { .. }
                | ItemKind::DefTypeDef { .. }
                | ItemKind::DefMeta { .. } => {}
                ItemKind::Result(value) => {
                    let ty = self.expression_item(value, true, result_expected);
                    if self.result_item(item.span, (value, ty), (first_field, next)) {
                        result = ty;
                    }
                }
                ItemKind::Expr(value) => {
                    // In a record block, even the final expression is a
                    // statement.
                    let is_result = next.is_none() && first_field.is_none();
                    let ty = self.expression_item(value, is_result, result_expected);
                    if is_result {
                        result = ty;
                    } else {
                        self.check_statement(value, ty);
                    }
                }
                ItemKind::Break(jump) | ItemKind::Continue(jump) => {
                    self.check_jump((id, item), jump, first_field);
                    if next.is_none() && first_field.is_none() {
                        result = self.left_type(id);
                    }
                }
                ItemKind::Return(jump) => {
                    self.check_return((id, item), jump, first_field);
                    if next.is_none() && first_field.is_none() {
                        result = self.left_type(id);
                    }
                }
                // What the parser could not parse: the block's result or
                // fields are unknown.
                ItemKind::Error => broken = true,
            }
            if leaves.is_none() && !early && self.never_completes(id, item.kind) {
                leaves = Some(item.span);
            }
        }
        if root {
            self.finish_top_level(items.len());
        }
        self.body_type(first_field.is_some(), (result, broken), expecting)
    }

    /// The type of a body, a record block if `record`, whose result's type
    /// is `result`, as [`Checker::check_body`] gives it: `Error` if it has
    /// an error item (`broken`), whose fields' and result's types are
    /// unknown, nothing then being expected of its record.
    fn body_type(
        &mut self,
        record: bool,
        (result, broken): (TyId, bool),
        expecting: Option<Expecting<'_>>,
    ) -> TyId {
        match (record, broken) {
            (true, true) => {
                self.record_type(None);
                TyId::ERROR
            }
            (true, false) => self.record_type(expecting),
            (false, true) => TyId::ERROR,
            (false, false) => result,
        }
    }

    /// Computes, records and returns the type of `id`, bottom-up.
    fn check_expr(&mut self, id: ExprId) -> TyId {
        let expr = self.ast.get(id);
        let ty = match expr.kind {
            ExprKind::Int(_) => TyId::UNTYPED_INT,
            ExprKind::Float(_) => TyId::FLOAT,
            ExprKind::Duration(_) => TyId::DURATION,
            ExprKind::Variant { of, name } => self.check_variant(id, (of, name)),
            ExprKind::ContextRead(name) => self.check_context_read(id, name),
            ExprKind::With { bindings, body } => self.check_with(id, bindings, body),
            ExprKind::Assert { cond, message } => self.check_assert(id, cond, message),
            ExprKind::Recover(recover) => self.check_recover(recover),
            ExprKind::Size(_) => TyId::SIZE,
            ExprKind::Bool(_) => TyId::BOOL,
            ExprKind::Str(_) => TyId::STRING,
            ExprKind::Interp(parts) => self.check_interpolation(id, parts),
            ExprKind::If {
                cond,
                then,
                else_,
                has_else,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                self.check_if(keyword, cond, (then, else_), (has_else, Branch::Free))
            }
            ExprKind::Paren(inner) => self.check_expr(inner),
            ExprKind::Unary { op, operand } => {
                let operand_ty = self.check_expr(operand);
                let op_span = expr.operator_span().unwrap_or(expr.span);
                self.unary((id, op), op_span, (operand, operand_ty))
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                self.binary(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
            }
            ExprKind::Name(name) => self.check_name(id, name, expr.span),
            ExprKind::Block { items, label } => match label {
                Some(label) => self.labelled_block(id, label, items, None),
                None => self.check_block(items, None),
            },
            ExprKind::Field { expr: record, name } => {
                // A reference to a node of the tree of the input's top-level
                // fields below its root reads that node's cell.
                let imports = self.workspace.is_some_and(Context::has_view);
                let reference = if self.mentions.tree.nodes.is_empty() && !imports {
                    None
                } else {
                    self.field_reference(id)
                };
                if let Some(ty) = reference {
                    self.builtin_use(id, ty, expr.span)
                } else if let Some(ty) = self.fileroot_lacks(id, record, name) {
                    ty
                } else {
                    let record_ty = self.check_expr(record);
                    self.field_access(id, (record, record_ty), name)
                }
            }
            ExprKind::Ascribe { expr: inner, ty } => {
                let expected = self.resolve_type(ty, Bareness::Present);
                let context = self.ast.type_expr(ty).span;
                let found = self.check_expr_expecting(inner, expected, context, "ascribed here");
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
            ExprKind::Is { expr: operand, ty } => match self.check_test(id, operand, ty, "is") {
                Some(tested) if tested.unreached => self.never,
                Some(_) => TyId::BOOL,
                None => TyId::ERROR,
            },
            ExprKind::As { expr: operand, ty } => self
                .check_test(id, operand, ty, "as")
                .map_or(TyId::ERROR, |tested| tested.narrowed),
            ExprKind::Fn(function) => self.check_fn(id, function),
            ExprKind::Call { callee, args } => self.check_call(id, callee, args),
            ExprKind::Lazy(function) => {
                self.check_lazy(function, symbol_span("lazy", expr.span.start))
            }
            ExprKind::List(elements) => self.check_list(id, elements, None),
            ExprKind::Index { expr: list, index } => self.check_index(id, list, index),
            ExprKind::For(for_id) => self.check_for(id, for_id, (None, None)),
            ExprKind::DefTypeRef(def) => self.check_def_type_ref(id, def),
            // What the parser could not parse, reported already.
            ExprKind::Error => TyId::ERROR,
            // What the parser kept of what it could not finish, reported
            // already: checked for what it is, silently.
            ExprKind::NoField(record) => {
                self.check_unfinished(&[record]);
                TyId::ERROR
            }
            ExprKind::Unclosed { callee, args } => {
                let mut parts = vec![callee];
                parts.extend(self.ast.args(args).iter().map(|arg| arg.value));
                self.check_unfinished(&parts);
                TyId::ERROR
            }
        };
        let ty = self.called_if_generic(id, ty);
        self.expr_types[id.index()] = ty;
        ty
    }

    /// Types `dt`, the def-type of a definition, a reference or a meta, as
    /// the callee it is: a reference calls it, so a generic def-type is read
    /// as a generic function called (see [Variant
    /// def-types](crate::check::deftypes#variant-def-types)).
    fn check_def_type_expr(&mut self, dt: ExprId) -> TyId {
        let outer = self.calls.callee.replace(dt);
        let ty = self.check_expr(dt);
        self.calls.callee = outer;
        ty
    }

    /// `ty`, the type of `id`, unless it is a generic function's and `id`, a
    /// name or a field access, is no call's callee: "a generic function must
    /// be called", and `Error` (see [Generics](crate::check::methods#generics)).
    fn called_if_generic(&mut self, id: ExprId, ty: TyId) -> TyId {
        let named = matches!(
            self.ast.get(id).kind,
            ExprKind::Name(_) | ExprKind::Field { .. }
        );
        let generic = self
            .types
            .fn_sig(ty)
            .is_some_and(|sig| sig.generic.is_some());
        if !named || !generic || self.calls.callee == Some(id) {
            return ty;
        }
        self.errors.push(
            Diagnostic::error("a generic function must be called", self.ast.get(id).span)
                .with_note("each call binds its type parameters; a value of it would bind none"),
        );
        TyId::ERROR
    }

    /// Checks `parts`, what the parser kept of an expression it could not
    /// finish (a field access with no name's record, an unclosed call's
    /// callee and arguments), each as any expression is but with no
    /// diagnostic: the expression's error is the parser's, and what each is
    /// alone may be what it may not be alone, a module or a built-in read
    /// whole, a callback whose parameters only the call types (see
    /// [Recording](#recording)).
    fn check_unfinished(&mut self, parts: &[ExprId]) {
        let (errors, warnings) = (self.errors.len(), self.warnings.len());
        for &part in parts {
            self.check_expr(part);
        }
        self.errors.truncate(errors);
        self.warnings.truncate(warnings);
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
    /// Only integer literals, parentheses, negation, `await` of a present
    /// value, `+ - *`, blocks, `with`s, `recover`s and `if`s with `else`
    /// can be untyped, and an
    /// untyped node's children
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
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Interp(_)
            | ExprKind::Name(_)
            | ExprKind::Variant { .. }
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
            | ExprKind::Assert { .. }
            | ExprKind::Error
            | ExprKind::NoField(_)
            | ExprKind::Unclosed { .. } => {}
            // An `await` of an untyped operand is one of a present value,
            // which it passes through, and a `with` its body.
            ExprKind::Paren(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::With { body: inner, .. } => {
                self.coerce(inner, ty);
            }
            ExprKind::Recover(id) => {
                let decl = *self.ast.recover_decl(id);
                self.coerce(decl.body, ty);
                self.coerce(decl.fallback, ty);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.coerce(lhs, ty);
                self.coerce(rhs, ty);
            }
            ExprKind::If { then, else_, .. } => {
                self.coerce(then, ty);
                self.coerce(else_, ty);
            }
            ExprKind::Block { items, label } => {
                self.coerce_result(items, ty);
                // A labelled block's `return`s are its result sites too.
                if label.is_some()
                    && let Some(values) = self.block_returns.get(&id).cloned()
                {
                    for value in values {
                        self.coerce(value, ty);
                    }
                }
            }
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
            ItemKind::Let { .. }
            | ItemKind::Rebind { .. }
            | ItemKind::Field { .. }
            | ItemKind::Spread(_)
            | ItemKind::Break(_)
            | ItemKind::Continue(_)
            | ItemKind::Return(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::Enum { .. }
            | ItemKind::Variant { .. }
            | ItemKind::Context { .. }
            | ItemKind::DefTypeDef { .. }
            | ItemKind::DefMeta { .. }
            | ItemKind::Error => {}
        }
    }

    /// Type `ty` as a diagnostic's message or notes show it (see
    /// [`Checker::shown`]): plainly if it is written as a name, and in
    /// backticks if it is structural, as a record, list, function, future
    /// or union type is.
    fn in_message(&mut self, ty: TyId) -> String {
        let ty = self.shown(ty);
        let shown = self.types.display(ty, self.ast.names());
        match self.types.kind(ty) {
            TyKind::Record(_)
            | TyKind::List(_)
            | TyKind::Fn(_)
            | TyKind::Future(_)
            | TyKind::Either(_)
            | TyKind::Union(_)
            | TyKind::Provisional { .. }
            | TyKind::Variant { .. }
            | TyKind::Partial(_)
            | TyKind::Secret(_)
            // TEMPORARY (alias rule): see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/).
            | TyKind::Builtin(_) => {
                format!("`{shown}`")
            }
            TyKind::Enum { .. }
            | TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Param { .. } => shown.to_string(),
        }
    }

    /// Type `ty` as a diagnostic's labels show it (see
    /// [`Checker::shown`]): always plainly.
    fn in_label(&mut self, ty: TyId) -> String {
        let ty = self.shown(ty);
        self.types.display(ty, self.ast.names()).to_string()
    }

    /// Type `ty` as a diagnostic shows it, with no provisional type in it,
    /// since it is shown before the parameters are decided: each of a
    /// claimable function's (see [`Checker::claimable`]) as what its
    /// parameter is if nothing more measures it, present if the body has
    /// measured it so far and the either shape otherwise, and each of any
    /// other enclosing function's as the either shape, which is what the
    /// body being checked treats it as.
    fn shown(&mut self, ty: TyId) -> TyId {
        if !self.types.contains_provisional(ty, None) {
            return ty;
        }
        let mut ty = ty;
        let claimable = self.claimable();
        for (index, function) in self.functions.iter().enumerate() {
            if function.demanded.is_empty() {
                continue;
            }
            let decisions: Vec<bool> = if claimable.contains(&index) {
                function.demanded.iter().map(Option::is_some).collect()
            } else {
                vec![false; function.demanded.len()]
            };
            ty = self.types.settle(ty, function.fn_id, &decisions);
        }
        ty
    }
}

/// The label on a function's declared result type where a value does not
/// fit it: the body's result, or a `return`'s value.
const RESULT_WHY: &str = "the return type is declared here";

/// What `Checker::filled` holds for a parameter a call gives its default
/// (see [Calls](#calls)): no argument's index, and not `u32::MAX`, which is
/// a parameter left unfilled.
const DEFAULTED: u32 = u32::MAX - 1;

/// The note on [`FIELD_NOT_CONSTANT`]: what a
/// constant is.
const CONSTANT_NOTE: &str = "a literal, or a list or record of literals";

/// The error for a field's default of a record type that is not a constant,
/// at the default (see [Field defaults](#field-defaults)).
const FIELD_NOT_CONSTANT: &str = "a field's default must be a constant";

/// The constant that `id` of `ast`, a parameter's or a field's default that
/// [`is_constant`] accepts, is, `types` being the type of each expression,
/// as the check found it, and `fills` the fields each record literal was
/// given ([`Resolution::record_fills`]): an integer literal that is a
/// `Float` there a `Float`, and an integer otherwise (an untyped one
/// settles to `Int`, and one of no type known is an `Int`); a record block
/// the record of its fields in the order they are written and then of
/// those it was given; and `{}` the record of the fields it was given, if
/// any, and the empty record otherwise. `None` if it is not one after all.
///
/// Recurses once per level of list and record nesting.
fn constant_of(ast: &Ast, id: ExprId, (types, fills): (&[TyId], &RecordFills)) -> Option<Constant> {
    let float = |id: ExprId| types.get(id.index()) == Some(&TyId::FLOAT);
    Some(match ast.get(id).kind {
        ExprKind::Int(value) if float(id) => Constant::Float(int_to_float(value).to_bits()),
        ExprKind::Int(value) => Constant::Int(value),
        ExprKind::Float(value) => Constant::Float(value.to_bits()),
        ExprKind::Duration(amount) => Constant::Quantity(Quantity::Duration, amount),
        ExprKind::Size(amount) => Constant::Quantity(Quantity::Size, amount),
        ExprKind::Bool(value) => Constant::Bool(value),
        ExprKind::Str(text) => Constant::Str(ast.strings().get(text).into()),
        // Negated as lowering negates it: as the whole is typed.
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => match ast.get(operand).kind {
            ExprKind::Int(value) if float(id) => Constant::Float((-int_to_float(value)).to_bits()),
            ExprKind::Int(value) => Constant::Int(value.checked_neg()?),
            ExprKind::Float(value) => Constant::Float((-value).to_bits()),
            ExprKind::Duration(amount) => {
                Constant::Quantity(Quantity::Duration, amount.checked_neg()?)
            }
            ExprKind::Size(amount) => Constant::Quantity(Quantity::Size, amount.checked_neg()?),
            _ => return None,
        },
        ExprKind::List(elements) => Constant::List(
            ast.elements(elements)
                .iter()
                .map(|element| constant_of(ast, element.value, (types, fills)))
                .collect::<Option<_>>()?,
        ),
        // `{}` given fields is the record of them.
        ExprKind::Block { items, label: None } if ast.items(items).is_empty() => {
            match fills.get(&id) {
                Some(filled) => Constant::Record(filled.clone()),
                None => Constant::Empty,
            }
        }
        ExprKind::Block { items, label: None } => {
            let mut fields = ast
                .items(items)
                .iter()
                .map(|&item| match ast.item(item).kind {
                    ItemKind::Field { path, value, .. } => {
                        let name = ast.path(path).first()?.name()?;
                        Some((name, constant_of(ast, value, (types, fills))?))
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            if let Some(filled) = fills.get(&id) {
                fields.extend(filled.iter().cloned());
            }
            Constant::Record(fields.into_boxed_slice())
        }
        _ => return None,
    })
}

/// The constant that `id` of `ast`, a field's default of a record type
/// copied out of its tree into a [`crate::ast::TypeStore`], is written as,
/// with no type known and nothing filled ([`constant_of`]), if
/// [`is_constant`] accepts it; `None` if it is not a constant, which the
/// check of its own tree reports (see [Field defaults](#field-defaults)).
/// The checker of each tree using the type fits it to its field's type
/// (`Checker::fit_constant`).
pub(crate) fn written_constant(ast: &Ast, id: ExprId) -> Option<Constant> {
    if !crate::ast::is_constant(ast, id) {
        return None;
    }
    constant_of(ast, id, (&[], &HashMap::new()))
}

/// The span of the keyword `keyword` of a `break`, `continue` or `return`
/// item spanning `item`, which it starts, through its label, if it has one.
fn jump_span(keyword: &str, item: Span, label: Option<Label>) -> Span {
    let written = symbol_span(keyword, item.start);
    label.map_or(written, |label| Span::join(written, label.span))
}

/// Whether `ty` is a numeric type, untyped integers included.
fn is_numeric(ty: TyId) -> bool {
    matches!(ty, TyId::INT | TyId::FLOAT | TyId::UNTYPED_INT)
}

/// Whether `ty` is a quantity, `Duration` or `Size`.
fn is_quantity(ty: TyId) -> bool {
    matches!(ty, TyId::DURATION | TyId::SIZE)
}

/// Whether `ty` is an `Int`, an untyped integer being one.
fn is_int(ty: TyId) -> bool {
    matches!(ty, TyId::INT | TyId::UNTYPED_INT)
}

/// A field's position in its block or type, as stored in
/// `Checker::sorted_fields`, a node's index in `Checker::nodes`, or a count
/// of path segments. There are fewer of each than the source has bytes, so
/// the conversion never saturates.
fn position(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// The label, at the path of the dotted `:=` whose path is `segments`, on
/// the type its value must fit: the type of its last field, or of the
/// elements of the list its last segment indexes.
fn leaf_label(segments: &[PathSegment]) -> &'static str {
    match segments.last().map(|segment| segment.kind) {
        Some(SegmentKind::Index { .. }) => "`:=` keeps the element's type",
        _ => "`:=` keeps the field's type",
    }
}

/// Which of the `count` parameters of a callee the arguments `args` fill,
/// by position: the positional arguments the first, a trailing block the
/// last, and each named argument the parameter `param_named` finds by its
/// name. What a suggestion for an unknown argument leaves out, a parameter
/// it would give twice.
fn filled_by(
    args: &[crate::ast::Arg],
    count: usize,
    param_named: impl Fn(Name) -> Option<usize>,
) -> Vec<bool> {
    let mut filled = vec![false; count];
    let positional = args
        .iter()
        .filter(|arg| arg.kind == ArgKind::Positional)
        .count();
    for slot in filled.iter_mut().take(positional) {
        *slot = true;
    }
    if positional < count
        && args.iter().any(|arg| arg.kind == ArgKind::Trailing)
        && let Some(last) = filled.last_mut()
    {
        *last = true;
    }
    for arg in args {
        if let ArgKind::Named { name, .. } = arg.kind
            && let Some(slot) = param_named(name).and_then(|at| filled.get_mut(at))
        {
            *slot = true;
        }
    }
    filled
}

/// The error of the named argument `arg`, of a call of `ast` whose
/// arguments are `args`, that no parameter of the built-in function, named
/// `params`, has: "unknown argument `x`", labelled `label` at the callee,
/// at `callee`, suggesting the nearest of the parameters that no other
/// argument fills ([`unknown_argument`]).
fn unknown_builtin_argument(
    ast: &Ast,
    args: &[crate::ast::Arg],
    params: &[&str],
    (arg, (callee, label)): (crate::ast::Arg, (Span, &str)),
) -> Diagnostic {
    let ArgKind::Named {
        name,
        span,
        shorthand,
    } = arg.kind
    else {
        // Only a named argument can name no parameter: kept for safety.
        return Diagnostic::error("unknown argument", arg.span(ast));
    };
    let text = ast.names().get(name);
    let filled = filled_by(args, params.len(), |name| {
        let text = ast.names().get(name);
        params.iter().position(|&param| param == text)
    });
    let free = params
        .iter()
        .zip(&filled)
        .filter(|&(_, &filled)| !filled)
        .map(|(&param, _)| (param, 0));
    unknown_argument((text, span, shorthand), free).with_context(callee, label)
}

/// The error of the named argument `text`, written at `span`, the shorthand
/// `text` alone if `shorthand`, that no parameter has: "unknown argument
/// `x`", suggesting the nearest of `free`, the named parameters no other
/// argument fills (see [Suggestions](crate::check::names#suggestions)). For the shorthand, which
/// reads the variable of its name too, the suggestion names the parameter
/// and keeps the variable, `best = x`, and a note says what the shorthand
/// is.
fn unknown_argument<'a>(
    (text, span, shorthand): (&str, Span, bool),
    free: impl Iterator<Item = (&'a str, u8)>,
) -> Diagnostic {
    let best = suggest::best(text, free);
    let error = Diagnostic::error(format!("unknown argument `{text}`"), span);
    if shorthand {
        let suggestion = best.map(|best| (span, format!("{best} = {text}")));
        error
            .with_note(format!(
                "`{text}` alone after a named argument is shorthand for `{text} = {text}`"
            ))
            .with_suggestion(suggestion)
    } else {
        error.with_suggestion(best.map(|best| (span, best.to_owned())))
    }
}

/// The error for a `:=`, whose name `text` is at `name_span`, of
/// `binding`, the copy of a variable that a branch of an `if NAME is TYPE`
/// sees: "cannot rebind `x` here: it is narrowed by `is`", labelled at the
/// test. Rebinding the copy would not change the variable it was copied
/// from, a mutable one included, so the note says how to keep the variable
/// free to rebind.
fn narrowed_rebind_error(text: &str, name_span: Span, binding: Binding) -> Diagnostic {
    let mut error = Diagnostic::error(
        format!("cannot rebind `{text}` here: it is narrowed by `is`"),
        name_span,
    );
    if let Some(declared) = binding.declared {
        error = error.with_context(declared, "narrowed here");
    }
    error.with_note(format!(
        "in this branch `{text}` is a copy of the value that was tested, so rebinding it would \
         not change the variable; test a copy under another name instead, as in `let v = \
         {text}` and `if v is ...`, which leaves `{text}` free to rebind"
    ))
}

/// The note on a dotted path under a field, or a segment, defined whole.
const WHOLE_FIELD_NOTE: &str = "a field defined as a whole cannot be extended with dotted paths";

/// The note on a def-type reference whose def-type is undefined or no
/// def-type: two names in a row that may have meant something else (see
/// [Def-types](crate::check::deftypes#def-types)).
const JUXTAPOSED_NOTE: &str = "two names in a row are read as a def-type reference, as in \
     `package dev.x()`; is an operator or a line break missing?";

/// The note on a field that the expected record type lacks.
const EXTRA_FIELD_NOTE: &str = "it is allowed, but nothing with this type can see it";

/// The note on a read of a field that a record type's rest-field covers.
const REST_READ_NOTE: &str =
    "a field that only the rest-field `*: ...` covers cannot be read by name";

/// The note on a spread of a value whose type has a rest-field.
const SPREAD_REST_NOTE: &str = "a spread defines the fields its operand's type names, and the fields a rest-field covers are known only when it runs";

/// The note on a field that dotted paths build where a record type with a
/// rest-field is expected of it.
const DOTTED_REST_NOTE: &str =
    "dotted paths build a record without a rest-field; define the field whole, as in `a = {...}`";

/// The label on a pure function's marker, where it does something only an
/// impure function may.
const PURE_CONTEXT: &str = "this function is pure, declared with `:`";

/// The note on a use of the `fileroot` module other than a read of one of
/// its members.
pub(crate) const FILEROOT_MEMBERS: &str =
    "`fileroot` has the members `this`, `workspace` and `caller`, each a String, and `helper()`";

/// The note on "field `x` is defined twice" for a spread, of a value or of
/// a type, with a field of the name of one defined before it and `others`
/// more such fields.
fn spread_first_note(others: usize) -> String {
    let also = match others {
        0 => String::new(),
        1 => " (it also has 1 other field defined before it)".to_owned(),
        others => format!(" (it also has {others} other fields defined before it)"),
    };
    format!("a spread cannot replace a field defined before it; put the spread first{also}")
}

/// The error for a record type that has the field `name`, or the
/// rest-field for `None`, more than once: "field `x` appears twice in this
/// type", "the rest-field appears twice in this type".
fn appears_twice(names: &Names, name: Option<Name>) -> String {
    match name {
        Some(name) => format!(
            "field `{}` appears twice in this type",
            FieldName(names.get(name))
        ),
        None => "the rest-field appears twice in this type".to_owned(),
    }
}

/// The error for an explicit `await`, whose keyword is at `keyword`, in a
/// pure function whose marker is at `marker`.
fn explicit_await_error(keyword: Span, marker: Span) -> Diagnostic {
    Diagnostic::error("cannot await in a pure function", keyword)
        .with_context(marker, PURE_CONTEXT)
        .with_note(
            "awaiting runs a deferred expression, which may have effects; declare the function \
             with `->` to make it impure",
        )
}

pub(crate) mod attributes;
pub(crate) mod calls;
pub(crate) mod context;
pub(crate) mod deftypes;
pub(crate) mod enums;
pub(crate) mod expect;
pub(crate) mod failure;
pub(crate) mod fields;
pub(crate) mod functions;
pub(crate) mod futures;
pub(crate) mod imports;
pub(crate) mod inference;
pub(crate) mod items;
pub(crate) mod lists;
pub(crate) mod loops;
pub(crate) mod methods;
pub(crate) mod mutation;
pub(crate) mod names;
pub(crate) mod narrowing;
pub(crate) mod operators;
pub(crate) mod records;
pub(crate) mod resolve;
pub(crate) mod schedule;
pub(crate) mod scopes;
pub(crate) mod secrets;
pub(crate) mod toplevel;

#[cfg(test)]
mod tests;
