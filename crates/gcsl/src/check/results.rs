//! What checking a file returns: the types of its expressions and of the names
//! it declares, its slots and places, and the constants and attributes it
//! reads.

use std::fmt;
use std::rc::Rc;

use crate::ast::{Annotate, AttributeId, ExprId, ItemId};
use crate::diagnostic::Diagnostic;
use crate::mentions::{FieldTree, Schedule};
use crate::names::{Name, Names};
use crate::types::{TyId, TypeTable};
use crate::value::Quantity;

use super::{Builtin, Resolution, Scope};

/// The type of every expression in an [`Ast`](super::Ast), indexed by [`ExprId`].
///
/// A `Types` returned by [`check`](super::check) contains neither [`TyId::UNTYPED_INT`]
/// nor [`TyId::ERROR`], nor any provisional type. An expression that is awaited where
/// it is used keeps the type it has before the await; see
/// [`Resolution::await_of`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    pub(super) types: Vec<TyId>,
    /// The types of what is not an expression's, usually empty.
    pub(super) others: OtherTypes,
}

/// The types a [`Types`] holds that are not an expression's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct OtherTypes {
    /// The type each `type` item of the tree stands for, by item, in
    /// source order (see [Type declarations](crate::check::resolve#type-declarations)).
    pub(super) declared: Vec<(ItemId, TyId)>,
    /// Each def-type reference whose path goes on past the definition it
    /// names, by its id, with the read of the definition and the type of
    /// the reference to it (see [Metas](crate::check::attributes#metas)); usually empty.
    pub(super) references_past: Vec<(ExprId, ExprId, TyId)>,
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
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
    /// for.
    #[must_use]
    pub fn get(&self, id: ExprId) -> TyId {
        self.types[id.index()]
    }

    /// Number of entries, equal to the number of nodes in the [`Ast`](super::Ast).
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
    pub(super) types: &'a Types,
    pub(super) table: &'a TypeTable,
    pub(super) names: &'a Names,
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
pub struct Slot(pub(super) u32);

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

/// A parameter's constant default, as its `= DEFAULT` is once checked against
/// the parameter's type (see "Calls" under
/// [Functions](crate::check::functions#functions)), which a call that leaves
/// the parameter unfilled is given, as [`ParamDefault::Constant`] in
/// [`FnResolution::defaults`](super::FnResolution::defaults) and
/// [`Resolution::call_defaults`]. It holds no id of a tree, since a call of a
/// `def` of another file, or of a function of an earlier session input, is
/// given it where that tree is not. A field's default of a record type is one
/// too, held in the type ([`crate::types::RecordTy::defaults`]), which a record
/// literal that lacks the field is given, in [`Resolution::record_fills`] (see
/// "Field defaults" under [Functions](crate::check::functions#functions)).
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
/// default once checked (see "Calls" under
/// [Functions](crate::check::functions#functions)), in
/// [`FnResolution::defaults`](super::FnResolution::defaults) and
/// [`Resolution::call_defaults`]: a constant, copied in where the call is, or
/// the default function of a default that is no constant, which the call calls
/// on its side, with the parameters before it. Neither holds an id of a tree,
/// since a call of a `def` of another file, or of a function of an earlier
/// session input, is given it where that tree is not: the default function is
/// found in the callee's closure, by the index among its captures that the
/// closure keeps it at.
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

/// One attribute of a function item, resolved (see
/// [Attributes](crate::check::attributes#attributes)): what a host knows it by,
/// and its arguments, every field of its record type by its name, in the type's
/// order, each the constant given or its default. A constant record among them
/// names its fields by the program's names, as every [`Constant`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// What a host knows it by.
    pub id: AttributeId,
    /// Its arguments, by field.
    pub args: Box<[(Box<str>, Constant)]>,
}

/// The result of a successful check: the types and the resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The type of every expression.
    pub types: Types,
    /// Where every variable is stored.
    pub resolution: Resolution,
}

/// Everything a successful [`Checker::check`](super::Checker::check) produces.
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

/// The tree of an input's top-level fields and when each of its nodes is
/// computed: what the checker found from [`Mentions`](super::Mentions) and lowering stores
/// the cells by (see [Top-level fields](crate::check::toplevel#top-level-fields)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldSchedule {
    /// The tree of the input's top-level fields.
    pub tree: FieldTree,
    /// The point of each node, and the order the nodes are computed in.
    pub schedule: Schedule,
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
