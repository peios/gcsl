//! The types of GCSL expressions, interned.
//!
//! Every type is a [`TyId`], an index into a [`TypeTable`] that holds each
//! distinct type once, as a [`TyKind`]. Interning makes type equality an
//! integer comparison and a type a `u32` that copies freely, however large
//! the structure it stands for: a record type's fields refer to their own
//! types by id, so a nested record type is a tree of ids, each stored once.
//!
//! The table pre-interns the types every program has, so they are constants:
//! [`TyId::INT`], [`TyId::FLOAT`], [`TyId::BOOL`], [`TyId::STRING`],
//! [`TyId::EMPTY`] (the empty record `{}`), and the checker-internal
//! [`TyId::UNTYPED_INT`] and [`TyId::ERROR`].
//!
//! # Records
//!
//! Record types are structural: `{x: Int, y: String}` is the type of every
//! record with exactly those fields, however it was built. A [`RecordTy`]
//! keeps its fields sorted by [`Name`], so two records with the same fields
//! written in different orders are the same kind and intern to the same id,
//! and a field is found by binary search. The order of `Name`s is the order
//! the names were first interned, which means nothing to a reader, so
//! [`TypeTable::display`] shows fields in alphabetical order of their text
//! instead.
//!
//! A record type `S` is a subtype of `T` when `S` has every field of `T`,
//! each with a type that is a subtype of the field's type in `T`: width and
//! depth subtyping, covariant. Every record type is therefore a subtype of
//! `{}`. `Int`, `Float`, `Bool` and `String` are subtypes only of
//! themselves (and of the unions they are members of: see
//! [Unions](#unions)). The *join* of two record types is the record of the
//! fields they share, each with the join of its two types; see
//! [`TypeTable::join`].
//!
//! # Defaults
//!
//! A field of a record type as written may have a default, `{exclude:
//! List<String> = [], from: String}`: a [`Constant`], which the checker
//! gives a record literal written where the type is expected and that
//! lacks the field (see [`crate::check`], "Field defaults"). A
//! [`RecordTy`] keeps its defaults beside its fields, sorted by name too,
//! so a record type with defaults is interned as a type of its own, and
//! [`TypeTable::display`] shows them where they are written. A record
//! literal's own type has none, though a field of it may have the type the
//! expected type gave it, defaults and all.
//!
//! Defaults do not change which values a type admits, so nothing that
//! compares types looks at them: subtyping, the test of `is` and `as`,
//! [`TypeTable::overlaps`] and the join compare fields alone, so `{a: Int =
//! 1}` and `{a: Int}` are each a subtype of the other, and where the
//! checker needs two types to be one it asks [`TypeTable::same`], which
//! forgets them. Where one of two such types must be kept, the type made
//! is as follows: the join of two record types has no default (it is built
//! from their shared fields, as a value's type is); a union keeps, of two
//! members that are each a subtype of the other, the one interned first,
//! with its defaults, as it does any two such members; and a record type
//! rebuilt from another's fields ([`TypeTable::settle`],
//! [`TypeTable::present`], [`TypeTable::with_field`]) keeps the defaults of
//! the fields it keeps, a field whose type a functional update sets losing
//! its own.
//!
//! # Lists
//!
//! `List<T>`, [`TyKind::List`], is the type of every list whose elements
//! are all `T`s; the source may write it `T[]`. A list type is a container,
//! as a record type is: its values are never futures themselves, but its
//! element type may be `Future<T>` or the either shape, and everything that
//! looks through a record's fields for futures and provisional types looks
//! through a list's element type the same way ([`TypeTable::present`],
//! [`TypeTable::contains_future`], [`TypeTable::settle`] and the others).
//! Subtyping is covariant, `List<S> <: List<T>` iff `S <: T`, and the join
//! of two list types is the list of the join of their element types.
//!
//! # Functions
//!
//! A function type, [`FnTy`], lists its parameters' types, each with the
//! parameter's name if the type gives one, its return type, and its
//! [`Effect`]: pure (`:`), memoised (`->>`) or impure (`->`). Names are
//! part of the type as interned and displayed, so `fn(x: Int): Int` and
//! `fn(Int): Int` are different ids, and they are what named arguments
//! match, but subtyping ignores them, so each of those is a subtype of the
//! other.
//!
//! The effects form a lattice, a chain: `Pure < Memo < Impure`. A pure
//! function can stand in for a memoised or an impure one, and a memoised
//! one for an impure one, but not the reverse: the caller of a pure
//! function may itself be pure, and call nothing that is not. `fn(P1, ...,
//! Pn) m R` is a subtype of `fn(Q1, ..., Qn) m' R'` when the arities are
//! equal, each `Qi` is a subtype of `Pi` (parameters are contravariant: a
//! function that accepts more can stand in for one that accepts less), `R`
//! is a subtype of `R'` (the return type is covariant), and `m <= m'`. Two
//! function types whose arities are equal, and of each pair of whose
//! parameter types one is a subtype of the other, join to a function type:
//! each parameter is the narrower of its pair, which both functions accept
//! (their types need not be identical, since two functions written with one
//! signature may be inferred different ones), and keeps its name if both
//! sides agree on it; its return type is the join of the two, and its
//! effect is the greater of the two. Any other two function types join to
//! their union.
//!
//! What a memoised function's bare parameter types mean is the checker's
//! business (see [`crate::check`]): they are present, since its arguments
//! are the key its results are memoised by, so the checker interns `fn(x:
//! Int) ->> Int` with a present `Int` parameter, which is not a subtype of
//! `fn(Int) -> Int`, whose bare parameter is the either shape, but is one of
//! `fn(Present<Int>) -> Int`. That is the contravariance above, nothing new.
//!
//! # Metas
//!
//! A function type may have a *meta*, [`Meta`]: a record attached to its
//! values, whose fields are read as the function's own, written after the
//! return type with a marker of its own, `fn(v: Version): PackageDef META->
//! PackageMeta`. The marker is an [`Effect`], as a function's is: `META:`
//! (pure), `META->>` (memoised) or `META->` (impure); the meta's type is a
//! record type, or a future or the either shape of one (see the checker's
//! "Metas"). A meta is part of the type as interned and displayed: `fn():
//! Int META: {a: Int}` and `fn(): Int` are different ids.
//!
//! A meta is purely additive. A function type with a meta is a subtype of
//! the same type without one, the meta forgotten, never the other way round;
//! with a meta on both sides, the meta types are compared as return types
//! are, covariantly, and the markers as effects are, `Pure < Memo <
//! Impure`. Two function types that join to a function type (see
//! [Functions](#functions)) join their metas as their results: a meta on
//! both sides joins to the meta of the join of the two types and the greater
//! of the two markers, widened if that join is; a meta on one side only is
//! forgotten, so the join has none. A function type's meta is looked into
//! wherever its return type is, by [`TypeTable::settle`], the helpers for
//! provisional types and [`TypeTable::special_builtin`].
//!
//! # Futures
//!
//! A future is a deferred computation: `lazy e` has type `Future<T>`,
//! [`TyKind::Future`], where `e: T`, and awaiting it yields a `T`. Three
//! shapes of type are involved for any `T`, and [`TypeTable::laziness`]
//! tells them apart at the top of a type:
//!
//! - a *present* `T`, whose values are never futures (every type that is
//!   neither of the others, records whose fields are futures included);
//! - `Future<T>`, a future whose await yields a `T`;
//! - `T | Future<T>`, the *either* shape, a [`TyKind::Either`] of `T`: a
//!   present `T` or a `Future<T>`. [`TypeTable::deferrable`] builds it, and
//!   its present member is never itself a future or an either, but may be
//!   a union (see [Unions](#unions)): `Int | String | Future<Int | String>`
//!   is the either of `Int | String`.
//!
//! Awaiting resolves a future fully, so no program can tell a future of a
//! future from a future of what it yields in the end, and
//! [`TypeTable::future`] interns `Future<Future<Int>>` and `Future<Int |
//! Future<Int>>` as `Future<Int>`: what a future yields is never itself a
//! future or an either (the fields of a record it yields are untouched),
//! but may be a union.
//!
//! [`TypeTable::strip`] removes every `Future` and either layer at the top of
//! a type, which is what awaiting a value fully does to its type, and
//! [`TypeTable::present`] also strips the fields of a record type and the
//! element type of a list type, recursively (but not the parameters or
//! results of function types), which is what awaiting it deeply does.
//!
//! Subtyping extends to them: `Future<S> <: Future<T>` iff `S <: T`; `S <: T
//! | Future<T>` iff `strip(S) <: T`, whatever shape `S` has (a present value
//! satisfies the either shape, and so do a future and an either whose
//! awaited value would); and nothing lazy is a subtype of a present type,
//! nor is a present type a subtype of `Future<T>`, so a written `Future<T>`
//! demands a deferred value. The rules apply through record fields, list
//! element types and function types as the others do. Joins: two futures
//! join to the future
//! of the join of what they yield, and a future or an either with anything
//! else to the either of the join of both sides stripped.
//!
//! # Unions
//!
//! A *union*, [`TyKind::Union`], is the type of every value of any of its
//! members: `Int | String` holds an `Int` or a `String`. Its members are
//! two or more *present* types, none of them a union, a future, an either
//! or a provisional type, and none a subtype of another;
//! [`TypeTable::union`] builds one from any present types, flattening the
//! unions among them, removing duplicates and every member that is a
//! subtype of another (so `{a: Int, b: Int} | {a: Int}` is `{a: Int}`, and
//! of two members that are each a subtype of the other, as `fn(x: Int):
//! Int` and `fn(Int): Int` are, keeping the one interned first), and making
//! a union of one member that member. The members are kept sorted by
//! [`TyId`], so that a union is interned once whatever order its members are
//! given in. A union is present: its values are never futures, though the
//! either shape and a future may hold one.
//!
//! Subtyping: a type that is not a union is a subtype of a union if it is a
//! subtype of one of its members, and a union is a subtype of a type if
//! each of its members is. With the rules above, that reaches through
//! record fields, list elements, function types and futures.
//! [`TypeTable::strip`] leaves a union as it is, since it is present;
//! [`TypeTable::present`] presents each member; [`TypeTable::contains_fn`],
//! [`TypeTable::contains_future`], [`TypeTable::settle`] and the helpers for
//! provisional types look through its members as they look through a
//! record's fields.
//!
//! The join of two types always exists ([`TypeTable::join`]): where two
//! types have no common supertype of the shapes above, at the top or inside
//! a record field, a list's element type or a function's return type, their
//! join is their union, and the join says it has *widened*, which the
//! checker warns about (see [`crate::check`]). A union joins with a type it
//! is a supertype of to itself, and with any other to the union of both
//! sides' members.
//!
//! A union of members of one kind of value (lists, say) has that kind (see
//! [`TypeTable::value_kind`]); any other has [`ValueKind::Mixed`], whose
//! values only the evaluator's general walker handles. A union all of
//! whose members are unit-like, as `List<{}> | {}` is, is unit-like (see
//! [`TypeTable::is_unit_like`]).
//!
//! A *test* of a type (see [`ValueTest`]) is how the checker's `is` and
//! `as` ask at runtime whether a value is of a type: by the structure of
//! the value, looking no further into it than what the static type of the
//! value leaves in doubt.
//!
//! # Never
//!
//! `Never`, [`TyKind::Never`], is the type of an expression that never
//! completes, a block that ends with a `break` or a `continue`, say: no
//! value has it. It cannot be written in source, and is not one of the
//! pre-interned types: [`TypeTable::never`] interns it on first use. It is
//! a subtype of every type, and no other type is a subtype of it, and it is
//! the identity of the join and of the union: `join(Never, T)` is `T`, not
//! widened, and a union drops it. It is present, holds no future and no
//! function, and [`TypeTable::strip`], [`TypeTable::present`] and
//! [`TypeTable::settle`] leave it as it is. It is unit-like
//! ([`TypeTable::is_unit_like`]), so a statement of it is no unused value,
//! and its kind is [`ValueKind::Never`], the kind of no value.
//!
//! # Built-in functions
//!
//! TEMPORARY (alias rule): a built-in function whose typing is special
//! (see [`crate::check::Builtin`]), `len` or `show`, has no type a function
//! type can hold until built-in functions get real polymorphic types (R11
//! of `docs/INFERENCE_REQS.md`). Its value has an opaque type of its own,
//! [`TyKind::Builtin`], shown `builtin len`, which cannot be written in
//! source: a subtype of itself only, holding no future, a function as far
//! as equality goes ([`TypeTable::contains_fn`]), and of kind
//! [`ValueKind::Fn`]. The checker lets it stand only where the alias rule
//! does, so no other type ever holds it ([`TypeTable::special_builtin`]).
//!
//! # Provisional types
//!
//! A pure function's bare parameter is present if the function's body
//! measures it and the either shape if it never does, which the checker
//! knows only once it has checked the body. While it checks the body, the
//! parameter has a checker-internal *provisional* type,
//! [`TyKind::Provisional`], shown as its type followed by `?` (`Int?`):
//! the parameter's type, `inner`, standing for either `inner` or
//! `deferrable(inner)`, tagged with the function and the parameter's
//! position. Like [`TyId::UNTYPED_INT`], it never survives a successful
//! check: when the body's check ends, [`TypeTable::settle`] replaces each of
//! the function's provisionals, wherever it sits inside a type, with what
//! was decided for its parameter.
//!
//! Until then a provisional is treated as whichever of its two outcomes is
//! the safer where it is: [`TypeTable::strip`] and [`TypeTable::present`]
//! remove it like a future layer; [`TypeTable::laziness`] has a variant of
//! its own for it; as a subtype and in a join it is its either shape
//! (whatever is decided, a value of the parameter satisfies what the either
//! shape does); and as a supertype, which it is only as the parameter of the
//! function's own type, in a recursive call or where the function is used
//! as a value in its own body, it is its present type `inner` (whatever is
//! decided, a present value satisfies it). Two provisionals of the same
//! parameter are one type, so a value passed through unmeasured keeps the
//! parameter's decision.
//!
//! # Lifetime
//!
//! A table only grows. It lives outside the checker so that a
//! [`crate::session::Session`] can keep one across inputs, like its
//! [`crate::names::Names`]. A type interned by an input that later fails is
//! kept: a type is a fact about structure, true whatever program mentioned
//! it, so a stale entry is harmless. (If the failed input's names are
//! forgotten and their `Name`s reused for other text, the entry simply
//! describes a record over the new names, which is still a valid type; no
//! live id can refer to it unless an equal type is interned again.) The
//! same holds for a provisional type: it names a function by its
//! [`FnId`] in the input that interned it, and a later input's function of
//! the same number, parameter and type interns to the same id, which is
//! harmless, since no settled type of the earlier input refers to it.

use std::collections::HashMap;
use std::fmt;

use crate::ast::{Effect, FnId};
use crate::check::{Builtin, Constant};
use crate::names::{Name, Names};
use crate::strings::{FieldName, Quoted};
use crate::value::ValueKind;

/// An interned type: an index into the [`TypeTable`] that produced it.
///
/// Ids are only meaningful for the table that produced them; using one with
/// a different table is a logic error. The constants are pre-interned by
/// every table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TyId(u32);

impl TyId {
    /// Signed 64-bit integer.
    pub const INT: TyId = TyId(0);
    /// IEEE 754 binary64 floating point.
    pub const FLOAT: TyId = TyId(1);
    /// `true` or `false`.
    pub const BOOL: TyId = TyId(2);
    /// An immutable UTF-8 string.
    pub const STRING: TyId = TyId(3);
    /// The empty record `{}`, the type with one value: the value of a block
    /// with no result and no fields, and of every statement.
    pub const EMPTY: TyId = TyId(4);
    /// The type of an integer literal whose type has not been settled by
    /// its context yet. Checker-internal: it never survives checking, and it
    /// displays as `Int` so users never see it.
    pub const UNTYPED_INT: TyId = TyId(5);
    /// The type of an expression that already has a reported type error.
    /// Checker-internal: every operator accepts it and produces it without
    /// reporting anything, so one mistake is reported once rather than at
    /// every enclosing operator. It never survives a successful check.
    pub const ERROR: TyId = TyId(6);

    /// The id as an index into its table.
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What a type is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TyKind {
    /// Signed 64-bit integer.
    Int,
    /// IEEE 754 binary64 floating point.
    Float,
    /// `true` or `false`.
    Bool,
    /// An immutable UTF-8 string.
    String,
    /// A record type: its fields, sorted by [`Name`], with no duplicates.
    Record(RecordTy),
    /// A list type, `List<T>`: the type of its elements. See the module
    /// documentation.
    List(TyId),
    /// A function type. See the module documentation.
    Fn(FnTy),
    /// A future whose await yields the type given, which is neither a
    /// future nor an either shape; [`TypeTable::future`] builds one. See
    /// the module documentation.
    Future(TyId),
    /// The either shape, `T | Future<T>`, of the type given, `T`, which is
    /// neither a future, an either shape nor a provisional type;
    /// [`TypeTable::deferrable`] builds one. See the module documentation.
    Either(TyId),
    /// A union of at least two present types, sorted by [`TyId`], with no
    /// duplicates, none of them a union, a future, an either shape, a
    /// provisional type or a checker-internal type, and none a subtype of
    /// another; [`TypeTable::union`] builds one. See the module
    /// documentation.
    Union(Box<[TyId]>),
    /// A pure function's bare parameter whose laziness is not decided yet:
    /// `inner` if the function's body measures it, and `inner`'s either
    /// shape if it never does. Checker-internal, like
    /// [`TyKind::UntypedInt`]: it never survives a successful check, and
    /// [`TypeTable::provisional`] builds one. See the module documentation.
    Provisional {
        /// The function whose parameter it is.
        function: FnId,
        /// The parameter's position among the function's parameters.
        param: u32,
        /// The parameter's type as written, resolved: never a future, an
        /// either shape or a provisional, but possibly a union.
        inner: TyId,
    },
    /// See [`TyId::UNTYPED_INT`].
    UntypedInt,
    /// See [`TyId::ERROR`].
    Error,
    /// `Never`, the type of an expression that never completes, such as a
    /// block that ends with a `break`: no value has it. See
    /// [Never](#never).
    Never,
    /// TEMPORARY (alias rule): the opaque type of a built-in function whose
    /// typing is special, `builtin len` or `builtin show`, which no type
    /// expression can write, until built-in functions get real polymorphic
    /// types (R11 of `docs/INFERENCE_REQS.md`). See [Built-in
    /// functions](#built-in-functions).
    Builtin(Builtin),
}

/// The fields of a record type, and the defaults of those that have one
/// (see [Defaults](#defaults)).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordTy {
    /// Each field's name and type, sorted by name, with no name twice.
    pub fields: Box<[(Name, TyId)]>,
    /// Each defaulted field's name and default, sorted by name, each a name
    /// of `fields`; `None` for a record type without defaults, never an
    /// empty list. Boxed once more, behind a thin pointer, so that a record
    /// type, nearly every one of which has none, makes a [`TyKind`] no
    /// larger than a function type's signature does.
    pub defaults: Option<Box<Defaults>>,
}

/// The defaults of a record type's fields: see [`RecordTy::defaults`].
pub type Defaults = Box<[(Name, Constant)]>;

/// The signature of a function type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnTy {
    /// Each parameter's name, if the type gives one, and type, in order.
    pub params: Box<[(Option<Name>, TyId)]>,
    /// The return type.
    pub ret: TyId,
    /// What the function may do: pure, declared with `:`, memoised,
    /// declared with `->>`, or impure, declared with `->`.
    pub effect: Effect,
    /// The function's meta, if the type has one (see [Metas](#metas)).
    pub meta: Option<Meta>,
}

/// The meta of a function type, `META-> PackageMeta` in `fn(v: Version):
/// PackageDef META-> PackageMeta`: a record attached to the function value,
/// whose fields are read as the function's (see [Metas](#metas)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Meta {
    /// What reading it does, as its marker says: pure (`META:`), memoised
    /// (`META->>`) or impure (`META->`).
    pub effect: Effect,
    /// Its type: a record type, or a future or the either shape of one.
    pub ty: TyId,
}

/// What the top of a type says about whether its values are futures. See
/// [`TypeTable::laziness`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Laziness {
    /// Never a future.
    Present,
    /// Always a future: `Future<T>`.
    Future,
    /// A future or not: `T | Future<T>`.
    Either,
    /// A pure function's parameter whose laziness is not decided yet: see
    /// [`TyKind::Provisional`].
    Provisional,
}

/// How a value is awaited where it is used as its bits: see
/// [`TypeTable::needs_await`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AwaitKind {
    /// Awaited fully at the top, through every level of future.
    Top,
    /// Awaited fully at the top and then in every field of a record, at any
    /// depth, whose type is a future or an either.
    Deep,
}

/// A test of whether a value is of a type, by the value's structure, as
/// `is` and `as` run it: the *plan* the checker decides for a test of a
/// type against what the static type of the value already says, looking
/// into the value only where that leaves the answer in doubt. The value
/// tested is present wherever the test looks (the checker awaits it as
/// deeply as its plan reaches).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueTest {
    /// Passes for every value: what the static type already guarantees.
    Pass,
    /// Passes for a value of this kind: `Int`, `Float`, `Bool` or `String`,
    /// any record or any list; never [`ValueKind::Fn`],
    /// [`ValueKind::Deferred`] or [`ValueKind::Mixed`].
    Kind(ValueKind),
    /// Passes for a record that has each of these fields, whatever else it
    /// has, the value of each passing the field's test. Never empty: a test
    /// of no field is `Kind(ValueKind::Record)`.
    Record(Box<[(Name, ValueTest)]>),
    /// Passes for a list each of whose elements passes the test, the empty
    /// list included. Never of [`ValueTest::Pass`]: that is
    /// `Kind(ValueKind::List)`.
    List(Box<ValueTest>),
    /// Passes for a value that passes any of the tests, of which there are
    /// at least two.
    Any(Box<[ValueTest]>),
}

impl ValueTest {
    /// Whether the test looks inside the value, at a record's fields or a
    /// list's elements, and so needs them present.
    #[must_use]
    pub fn walks(&self) -> bool {
        match self {
            ValueTest::Pass | ValueTest::Kind(_) => false,
            ValueTest::Record(_) | ValueTest::List(_) => true,
            ValueTest::Any(tests) => tests.iter().any(ValueTest::walks),
        }
    }
}

/// The interning table: each distinct type once, with its [`TyId`]. See the
/// module documentation.
#[derive(Clone, Debug)]
pub struct TypeTable {
    /// The kind of each type, indexed by [`TyId`].
    kinds: Vec<TyKind>,
    /// The id of each kind, the inverse of `kinds`.
    lookup: HashMap<TyKind, TyId>,
    /// Whether each type is or contains a [`TyKind::Provisional`] at any
    /// depth, function types included, indexed by [`TyId`]: computed when
    /// the type is interned, from its parts, which are interned first, so
    /// that the types without one, nearly all of them, are passed over in
    /// constant time.
    provisional: Vec<bool>,
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTable {
    /// A table holding the pre-interned types, whose ids are the constants
    /// on [`TyId`].
    #[must_use]
    pub fn new() -> Self {
        let mut table = Self {
            kinds: Vec::new(),
            lookup: HashMap::new(),
            provisional: Vec::new(),
        };
        // In the order of the constants' numbers.
        for kind in [
            TyKind::Int,
            TyKind::Float,
            TyKind::Bool,
            TyKind::String,
            TyKind::Record(RecordTy {
                fields: Box::default(),
                defaults: None,
            }),
            TyKind::UntypedInt,
            TyKind::Error,
        ] {
            table.intern(kind);
        }
        table
    }

    /// Returns the id of `kind`, adding it to the table if it is new.
    ///
    /// A record's fields, and its defaults, are sorted by name here, so they
    /// may be given in any order; they must not repeat a name (the checker
    /// reports repeated names before it builds a type), and each default
    /// must be of one of the fields. A future, an either shape, a union
    /// and a provisional type must already satisfy the invariants
    /// documented on [`TyKind::Future`], [`TyKind::Either`],
    /// [`TyKind::Union`] and [`TyKind::Provisional`].
    ///
    /// A table holds at most `u32::MAX` types. Every type but the
    /// pre-interned ones is built from source (a record block, a list
    /// literal, a record, list or function type expression, a function
    /// expression or a parameter, each at least two bytes of it, the join of
    /// two types already in the table, which is no larger than the two
    /// together, or a type already in the table with its provisionals
    /// settled, which is no larger than it), so the limit is far beyond any
    /// table; past it, ids would saturate.
    pub fn intern(&mut self, kind: TyKind) -> TyId {
        let kind = match kind {
            TyKind::Record(RecordTy {
                mut fields,
                mut defaults,
            }) => {
                if !fields.is_sorted_by_key(|&(name, _)| name) {
                    fields.sort_unstable_by_key(|&(name, _)| name);
                }
                if let Some(defaults) = &mut defaults
                    && !defaults.is_sorted_by_key(|&(name, _)| name)
                {
                    defaults.sort_unstable_by_key(|&(name, _)| name);
                }
                // No default is none.
                let defaults = defaults.filter(|defaults| !defaults.is_empty());
                TyKind::Record(RecordTy { fields, defaults })
            }
            other => other,
        };
        if let Some(&id) = self.lookup.get(&kind) {
            return id;
        }
        let id = TyId(u32::try_from(self.kinds.len()).unwrap_or(u32::MAX));
        let provisional = match &kind {
            TyKind::Provisional { .. } => true,
            TyKind::Record(record) => record
                .fields
                .iter()
                .any(|&(_, field)| self.has_provisional(field)),
            TyKind::Fn(sig) => {
                self.has_provisional(sig.ret)
                    || sig.meta.is_some_and(|meta| self.has_provisional(meta.ty))
                    || sig.params.iter().any(|&(_, ty)| self.has_provisional(ty))
            }
            TyKind::Future(inner) | TyKind::Either(inner) | TyKind::List(inner) => {
                self.has_provisional(*inner)
            }
            TyKind::Union(members) => members.iter().any(|&member| self.has_provisional(member)),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Builtin(_) => false,
        };
        self.provisional.push(provisional);
        self.kinds.push(kind.clone());
        self.lookup.insert(kind, id);
        id
    }

    /// Whether `id` is or contains a provisional type at any depth; `false`
    /// for an id this table did not produce.
    fn has_provisional(&self, id: TyId) -> bool {
        self.provisional.get(id.index()).copied().unwrap_or(false)
    }

    /// The id of `Never` (see [Never](#never)), adding it to the table if
    /// it is new: it is not one of the pre-interned types, but interned on
    /// first use, as any type is.
    pub fn never(&mut self) -> TyId {
        self.intern(TyKind::Never)
    }

    /// Whether `id` is `Never`.
    #[must_use]
    pub fn is_never(&self, id: TyId) -> bool {
        matches!(self.kinds.get(id.index()), Some(TyKind::Never))
    }

    /// TEMPORARY (alias rule): the id of the opaque type of `builtin`, a
    /// built-in function whose typing is special, adding it to the table if
    /// it is new (see [Built-in functions](#built-in-functions)); replaced
    /// by its polymorphic type (R11 of `docs/INFERENCE_REQS.md`).
    pub fn builtin(&mut self, builtin: Builtin) -> TyId {
        self.intern(TyKind::Builtin(builtin))
    }

    /// TEMPORARY (alias rule): whether `id` is the opaque type of a
    /// built-in function whose typing is special (R11 of
    /// `docs/INFERENCE_REQS.md`).
    #[must_use]
    pub fn is_builtin(&self, id: TyId) -> bool {
        matches!(self.kinds.get(id.index()), Some(TyKind::Builtin(_)))
    }

    /// TEMPORARY (alias rule): the built-in function whose opaque type `id`
    /// is, or that a record, list, future, union or function type `id`
    /// holds at any depth: a type no value of which may stand where the
    /// alias rule does not let one (see [Built-in
    /// functions](#built-in-functions); R11 of `docs/INFERENCE_REQS.md`).
    /// `None` for every other type.
    ///
    /// Recurses once per level of nesting.
    #[must_use]
    pub fn special_builtin(&self, id: TyId) -> Option<Builtin> {
        match self.kinds.get(id.index())? {
            TyKind::Builtin(builtin) => Some(*builtin),
            TyKind::Record(record) => record
                .fields
                .iter()
                .find_map(|&(_, field)| self.special_builtin(field)),
            TyKind::Fn(sig) => sig
                .params
                .iter()
                .find_map(|&(_, param)| self.special_builtin(param))
                .or_else(|| self.special_builtin(sig.ret))
                .or_else(|| sig.meta.and_then(|meta| self.special_builtin(meta.ty))),
            TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Provisional { inner, .. } => self.special_builtin(*inner),
            TyKind::Union(members) => members
                .iter()
                .find_map(|&member| self.special_builtin(member)),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never => None,
        }
    }

    /// Returns the id of the record type with `fields`, in any order, adding
    /// it to the table if it is new. The fields must not repeat a name.
    pub fn record(&mut self, fields: impl IntoIterator<Item = (Name, TyId)>) -> TyId {
        self.record_with_defaults(fields, Vec::new())
    }

    /// Returns the id of the record type with `fields`, in any order, the
    /// fields named in `defaults` having those defaults (see
    /// [Defaults](#defaults)), adding it to the table if it is new. The
    /// fields must not repeat a name, and each default must be of one of
    /// them.
    pub fn record_with_defaults(
        &mut self,
        fields: impl IntoIterator<Item = (Name, TyId)>,
        defaults: Vec<(Name, Constant)>,
    ) -> TyId {
        self.intern(TyKind::Record(RecordTy {
            fields: fields.into_iter().collect(),
            defaults: (!defaults.is_empty()).then(|| Box::new(defaults.into_boxed_slice())),
        }))
    }

    /// The record type with `fields`, sorted by name, keeping the default
    /// that the record type `template` has of each of them that it has one
    /// for, adding it to the table if it is new: a record type rebuilt from
    /// another's fields, settled or made present, which is still what was
    /// written (see [Defaults](#defaults)).
    pub fn record_like(&mut self, template: TyId, fields: Vec<(Name, TyId)>) -> TyId {
        let defaults: Vec<(Name, Constant)> = self
            .defaults(template)
            .iter()
            .filter(|(name, _)| {
                fields
                    .binary_search_by_key(name, |&(field, _)| field)
                    .is_ok()
            })
            .cloned()
            .collect();
        self.record_with_defaults(fields, defaults)
    }

    /// The defaults of the fields of record type `id` that have one, sorted
    /// by name; empty for a record type without defaults and for any other
    /// type.
    #[must_use]
    pub fn defaults(&self, id: TyId) -> &[(Name, Constant)] {
        match self.kinds.get(id.index()) {
            Some(TyKind::Record(RecordTy {
                defaults: Some(defaults),
                ..
            })) => defaults,
            _ => &[],
        }
    }

    /// The default of field `name` of record type `record`, if it has one.
    #[must_use]
    pub fn field_default(&self, record: TyId, name: Name) -> Option<&Constant> {
        let defaults = self.defaults(record);
        let index = defaults
            .binary_search_by_key(&name, |&(field, _)| field)
            .ok()?;
        defaults.get(index).map(|(_, constant)| constant)
    }

    /// Returns the id of the function type with `params`, each a parameter's
    /// name, if it has one, and type, returning `ret`, of effect `effect`,
    /// adding it to the table if it is new.
    pub fn fn_type(
        &mut self,
        params: impl IntoIterator<Item = (Option<Name>, TyId)>,
        ret: TyId,
        effect: Effect,
    ) -> TyId {
        self.fn_type_with_meta(params, ret, effect, None)
    }

    /// Returns the id of the function type [`TypeTable::fn_type`] makes of
    /// `params`, `ret` and `effect`, with the meta `meta`, if it has one (see
    /// [Metas](#metas)), adding it to the table if it is new.
    pub fn fn_type_with_meta(
        &mut self,
        params: impl IntoIterator<Item = (Option<Name>, TyId)>,
        ret: TyId,
        effect: Effect,
        meta: Option<Meta>,
    ) -> TyId {
        self.intern(TyKind::Fn(FnTy {
            params: params.into_iter().collect(),
            ret,
            effect,
            meta,
        }))
    }

    /// Returns the id of the list type whose elements are of type `element`,
    /// `List<T>`, adding it to the table if it is new.
    pub fn list(&mut self, element: TyId) -> TyId {
        self.intern(TyKind::List(element))
    }

    /// The element type of list type `id`, or `None` if it is not a list
    /// type.
    #[must_use]
    pub fn list_element(&self, id: TyId) -> Option<TyId> {
        match self.kinds.get(id.index())? {
            TyKind::List(element) => Some(*element),
            _ => None,
        }
    }

    /// Whether `id` is *unit-like*: `{}`, `Never`, a list type whose element
    /// type is unit-like, as `List<{}>` and `List<List<{}>>` are, or a
    /// union all of whose members are, as `List<{}> | {}` is. A value of
    /// such a type holds nothing but empty records and lists of them, and
    /// `Never` has no value at all, so a statement of one is not an unused
    /// value (see [`crate::check`]). A type that is unit-like only once
    /// awaited, as `List<Future<{}>>` is, is not.
    ///
    /// Recurses once per union met inside a list's element type.
    #[must_use]
    pub fn is_unit_like(&self, id: TyId) -> bool {
        let mut id = id;
        loop {
            if id == TyId::EMPTY || self.is_never(id) {
                return true;
            }
            if let Some(members) = self.union_members(id) {
                return members.iter().all(|&member| self.is_unit_like(member));
            }
            match self.list_element(id) {
                Some(element) => id = element,
                None => return false,
            }
        }
    }

    /// The signature of function type `id`, or `None` if it is not a
    /// function type.
    #[must_use]
    pub fn fn_sig(&self, id: TyId) -> Option<&FnTy> {
        match self.kinds.get(id.index())? {
            TyKind::Fn(sig) => Some(sig),
            _ => None,
        }
    }

    /// Whether `id` is a function type (or the opaque type of a special
    /// built-in function, which is one too), or a record type with a field of a
    /// type that is or contains one, or a list type whose element type is or
    /// contains one, at any depth, looking through futures, unions and
    /// provisional types: a type whose values cannot be compared, since
    /// functions have no equality.
    ///
    /// Recurses once per level of record, list, future, union or
    /// provisional nesting.
    #[must_use]
    pub fn contains_fn(&self, id: TyId) -> bool {
        match self.kinds.get(id.index()) {
            // TEMPORARY (alias rule): a special built-in's opaque type is a
            // function's (R11 of `docs/INFERENCE_REQS.md`).
            Some(TyKind::Fn(_) | TyKind::Builtin(_)) => true,
            Some(TyKind::Record(record)) => record
                .fields
                .iter()
                .any(|&(_, field)| self.contains_fn(field)),
            Some(
                TyKind::Future(inner)
                | TyKind::Either(inner)
                | TyKind::List(inner)
                | TyKind::Provisional { inner, .. },
            ) => self.contains_fn(*inner),
            Some(TyKind::Union(members)) => members.iter().any(|&member| self.contains_fn(member)),
            _ => false,
        }
    }

    /// The type of a future whose await yields `inner`, `Future<T>` with `T`
    /// being `inner` stripped (see [`TypeTable::strip`]), adding it to the
    /// table if it is new. Awaiting resolves fully, so a future of a future,
    /// or of an either, is a future of what it yields in the end:
    /// `Future<Future<Int>>` and `Future<Int | Future<Int>>` are both
    /// `Future<Int>`.
    pub fn future(&mut self, inner: TyId) -> TyId {
        let inner = self.strip(inner);
        self.intern(TyKind::Future(inner))
    }

    /// The either shape of `inner`, `T | Future<T>` with `T` being `inner`
    /// stripped (see [`TypeTable::strip`]), adding it to the table if it is
    /// new. `Error` for `Error`, which has no either shape.
    pub fn deferrable(&mut self, inner: TyId) -> TyId {
        let present = self.strip(inner);
        if present == TyId::ERROR {
            return TyId::ERROR;
        }
        self.intern(TyKind::Either(present))
    }

    /// The present member of `id` if it is an either shape, `T | Future<T>`:
    /// `T`. `None` for every other type, unions included.
    fn either_member(&self, id: TyId) -> Option<TyId> {
        match self.kinds.get(id.index()) {
            Some(&TyKind::Either(present)) => Some(present),
            _ => None,
        }
    }

    /// Whether `id` is an either shape, `T | Future<T>`.
    #[must_use]
    pub fn is_deferrable(&self, id: TyId) -> bool {
        self.either_member(id).is_some()
    }

    /// The union of `members`, present types, adding it to the table if it
    /// is new (see the module documentation): the unions among them
    /// flattened into their members, duplicates removed, and every member
    /// that is a subtype of another dropped, of two members that are each a
    /// subtype of the other the one with the greater id; the one member
    /// left if only one is, and a [`TyKind::Union`] of those left, sorted by
    /// id, otherwise. `Never` is dropped, since it adds no value, and is the
    /// union only if it is every member. `Error` if any member is `Error`, or
    /// if there are none.
    /// No member may be a future, an either shape, a provisional type or an
    /// untyped integer: the checker joins those through
    /// [`TypeTable::join`], which never makes a union of them.
    ///
    /// Takes time quadratic in the number of members, for the subtype tests
    /// between them; unions are small.
    pub fn union(&mut self, members: impl IntoIterator<Item = TyId>) -> TyId {
        let mut flat = Vec::new();
        let mut never = None;
        for member in members {
            match self.kinds.get(member.index()) {
                Some(TyKind::Union(inner)) => flat.extend_from_slice(inner),
                Some(TyKind::Error) | None => return TyId::ERROR,
                // `Never` adds no value to a union.
                Some(TyKind::Never) => never = Some(member),
                Some(_) => flat.push(member),
            }
        }
        if let (Some(never), true) = (never, flat.is_empty()) {
            return never;
        }
        flat.sort_unstable();
        flat.dedup();
        let kept: Vec<TyId> = flat
            .iter()
            .enumerate()
            .filter(|&(at, &member)| {
                !flat.iter().enumerate().any(|(other_at, &other)| {
                    other_at != at
                        && self.is_subtype(member, other)
                        && (other_at < at || !self.is_subtype(other, member))
                })
            })
            .map(|(_, &member)| member)
            .collect();
        match *kept.as_slice() {
            [] => TyId::ERROR,
            [only] => only,
            _ => self.intern(TyKind::Union(kept.into_boxed_slice())),
        }
    }

    /// The members of `id` if it is a union, `None` for every other type.
    #[must_use]
    pub fn union_members(&self, id: TyId) -> Option<&[TyId]> {
        match self.kinds.get(id.index()) {
            Some(TyKind::Union(members)) => Some(members),
            _ => None,
        }
    }

    /// The provisional type of parameter number `param` of function
    /// `function`, whose type as written is `inner`, adding it to the table
    /// if it is new (see [`TyKind::Provisional`]). `inner` must be neither a
    /// future, an either shape, a provisional type nor `Error`: the checker
    /// only makes one for a parameter whose type is written as a name or a
    /// record type and names a type.
    pub fn provisional(&mut self, function: FnId, param: u32, inner: TyId) -> TyId {
        self.intern(TyKind::Provisional {
            function,
            param,
            inner,
        })
    }

    /// Type `id` with every provisional type of function `function` in it,
    /// at any depth, through records, lists, futures, either shapes, unions
    /// and function types,
    /// replaced by what was decided for its parameter: its present type if
    /// `decisions[param]` is `true`, the parameter being measured, and the
    /// either shape of it otherwise. Provisional types of other functions
    /// are left as they are, and so is one whose parameter has no decision.
    /// Interns the types it makes; returns `id` itself, in constant time, if
    /// it has no provisional type in it.
    ///
    /// Recurses once per level of nesting in the type.
    pub fn settle(&mut self, id: TyId, function: FnId, decisions: &[bool]) -> TyId {
        if !self.has_provisional(id) {
            return id;
        }
        // A copy of the kind (ids only, apart from the boxed lists), since
        // settling a part may intern new types.
        let Some(kind) = self.kinds.get(id.index()).cloned() else {
            return id;
        };
        match kind {
            TyKind::Provisional {
                function: of,
                param,
                inner,
            } if of == function => match decisions.get(param as usize) {
                Some(true) => inner,
                Some(false) => self.deferrable(inner),
                None => id,
            },
            TyKind::Record(record) => {
                let fields: Vec<(Name, TyId)> = record
                    .fields
                    .iter()
                    .map(|&(name, ty)| (name, self.settle(ty, function, decisions)))
                    .collect();
                // Still sorted, so interning does not sort it again.
                self.record_like(id, fields)
            }
            TyKind::Fn(sig) => {
                let params: Vec<(Option<Name>, TyId)> = sig
                    .params
                    .iter()
                    .map(|&(name, ty)| (name, self.settle(ty, function, decisions)))
                    .collect();
                let ret = self.settle(sig.ret, function, decisions);
                let meta = sig.meta.map(|meta| Meta {
                    ty: self.settle(meta.ty, function, decisions),
                    ..meta
                });
                self.fn_type_with_meta(params, ret, sig.effect, meta)
            }
            TyKind::Future(inner) => {
                let inner = self.settle(inner, function, decisions);
                self.future(inner)
            }
            TyKind::List(element) => {
                let element = self.settle(element, function, decisions);
                self.list(element)
            }
            TyKind::Either(present) => {
                let present = self.settle(present, function, decisions);
                self.deferrable(present)
            }
            TyKind::Union(members) => {
                // Settling may make one member a subtype of another, so
                // the union is built again.
                let members: Vec<TyId> = members
                    .iter()
                    .map(|&member| self.settle(member, function, decisions))
                    .collect();
                self.union(members)
            }
            TyKind::Provisional { .. }
            | TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Builtin(_) => id,
        }
    }

    /// Whether type `id` is or contains, at any depth, through records,
    /// lists, futures, either shapes, unions and function types, a
    /// provisional type: one of
    /// function `function`'s parameters, or of any function's for `None`.
    /// Constant time for `None`, and for a type with no provisional type at
    /// all.
    ///
    /// Recurses once per level of nesting in the type.
    #[must_use]
    pub fn contains_provisional(&self, id: TyId, function: Option<FnId>) -> bool {
        if !self.has_provisional(id) {
            return false;
        }
        let Some(function) = function else {
            return true;
        };
        match self.kinds.get(id.index()) {
            Some(TyKind::Provisional { function: of, .. }) => *of == function,
            Some(TyKind::Record(record)) => record
                .fields
                .iter()
                .any(|&(_, field)| self.contains_provisional(field, Some(function))),
            Some(TyKind::Fn(sig)) => {
                self.contains_provisional(sig.ret, Some(function))
                    || sig
                        .meta
                        .is_some_and(|meta| self.contains_provisional(meta.ty, Some(function)))
                    || sig
                        .params
                        .iter()
                        .any(|&(_, ty)| self.contains_provisional(ty, Some(function)))
            }
            Some(TyKind::Future(inner) | TyKind::Either(inner) | TyKind::List(inner)) => {
                self.contains_provisional(*inner, Some(function))
            }
            Some(TyKind::Union(members)) => members
                .iter()
                .any(|&member| self.contains_provisional(member, Some(function))),
            _ => false,
        }
    }

    /// Adds to `out` the parameter of each provisional type of function
    /// `function` in type `id`, at any depth, through records, lists,
    /// futures, either shapes, unions and function types, in no particular
    /// order and possibly more than once.
    ///
    /// Recurses once per level of nesting in the type.
    pub fn provisional_params(&self, id: TyId, function: FnId, out: &mut Vec<u32>) {
        if !self.has_provisional(id) {
            return;
        }
        match self.kinds.get(id.index()) {
            Some(TyKind::Provisional {
                function: of,
                param,
                ..
            }) if *of == function => out.push(*param),
            Some(TyKind::Record(record)) => {
                for &(_, field) in &record.fields {
                    self.provisional_params(field, function, out);
                }
            }
            Some(TyKind::Fn(sig)) => {
                for &(_, ty) in &sig.params {
                    self.provisional_params(ty, function, out);
                }
                self.provisional_params(sig.ret, function, out);
                if let Some(meta) = sig.meta {
                    self.provisional_params(meta.ty, function, out);
                }
            }
            Some(TyKind::Future(inner) | TyKind::Either(inner) | TyKind::List(inner)) => {
                self.provisional_params(*inner, function, out);
            }
            Some(TyKind::Union(members)) => {
                for &member in members {
                    self.provisional_params(member, function, out);
                }
            }
            _ => {}
        }
    }

    /// Adds to `out` the parameter of each provisional type of function
    /// `function` in type `found` that sits where `expected` has a present
    /// type (or a provisional one, which stands for its present type as a
    /// supertype): the parameters that a value of type `found` would need
    /// to be present to fit `expected`, in no particular order and possibly
    /// more than once. The two types are walked together, through record
    /// fields that both have, through the element types of list types,
    /// through future and either layers (with
    /// `expected` stripped to match), through the members of a union on
    /// either side (each member of `found` against `expected`, and `found`
    /// against each member of `expected` where `expected` is a union once
    /// stripped and `found` is not a provisional type, conservatively: a
    /// parameter that would need to be present to fit any member is
    /// added), and through the return types of function types, but not
    /// their parameters: a parameter is contravariant, so a provisional one
    /// fits no worse for being left undecided. Nothing is added where
    /// `expected` is a `Future`, which no present value fits.
    ///
    /// Recurses once per level of nesting shared by the two types.
    pub fn provisionals_at_present(
        &self,
        found: TyId,
        expected: TyId,
        function: FnId,
        out: &mut Vec<u32>,
    ) {
        if !self.has_provisional(found) || expected == TyId::ERROR {
            return;
        }
        let provisional = matches!(
            self.kinds.get(found.index()),
            Some(TyKind::Provisional { .. })
        );
        if !provisional
            && self.laziness(expected) != Laziness::Future
            && let Some(members) = self.union_members(self.strip(expected))
        {
            for &member in members {
                self.provisionals_at_present(found, member, function, out);
            }
            return;
        }
        match self.kinds.get(found.index()) {
            Some(TyKind::Provisional {
                function: of,
                param,
                ..
            }) => {
                if *of == function
                    && matches!(
                        self.laziness(expected),
                        Laziness::Present | Laziness::Provisional
                    )
                {
                    out.push(*param);
                }
            }
            Some(TyKind::Record(record)) => {
                if self.laziness(expected) == Laziness::Future {
                    return;
                }
                let expected = self.strip(expected);
                for &(name, field) in &record.fields {
                    if let Some(expected_field) = self.field(expected, name) {
                        self.provisionals_at_present(field, expected_field, function, out);
                    }
                }
            }
            Some(TyKind::List(element)) => {
                if self.laziness(expected) == Laziness::Future {
                    return;
                }
                if let Some(expected) = self.list_element(self.strip(expected)) {
                    self.provisionals_at_present(*element, expected, function, out);
                }
            }
            Some(TyKind::Future(inner) | TyKind::Either(inner)) => {
                self.provisionals_at_present(*inner, self.strip(expected), function, out);
            }
            Some(TyKind::Union(members)) => {
                for &member in members {
                    self.provisionals_at_present(member, expected, function, out);
                }
            }
            Some(TyKind::Fn(sig)) => {
                if let Some(required) = self.fn_sig(self.strip(expected)) {
                    self.provisionals_at_present(sig.ret, required.ret, function, out);
                    // A meta is compared as a return type is.
                    if let (Some(meta), Some(required)) = (sig.meta, required.meta) {
                        self.provisionals_at_present(meta.ty, required.ty, function, out);
                    }
                }
            }
            _ => {}
        }
    }

    /// What the top of type `id` says about whether its values are futures:
    /// [`Laziness::Future`] for a `Future<T>`, [`Laziness::Either`] for a
    /// `T | Future<T>`, [`Laziness::Provisional`] for a provisional type,
    /// and [`Laziness::Present`] for everything else, a record with futures
    /// in its fields, a list of futures and a union included.
    #[must_use]
    pub fn laziness(&self, id: TyId) -> Laziness {
        match self.kinds.get(id.index()) {
            Some(TyKind::Future(_)) => Laziness::Future,
            Some(TyKind::Either(_)) => Laziness::Either,
            Some(TyKind::Provisional { .. }) => Laziness::Provisional,
            _ => Laziness::Present,
        }
    }

    /// Type `id` with every `Future`, either and provisional layer at its
    /// top removed: the type of its value once awaited fully. `Future<Int>`,
    /// `Int | Future<Int>`, `Int?` and `Int` all strip to `Int`; a record's
    /// fields, a list's element type and a union's members are left as they
    /// are.
    #[must_use]
    pub fn strip(&self, id: TyId) -> TyId {
        let mut id = id;
        loop {
            match self.kinds.get(id.index()) {
                Some(
                    TyKind::Future(inner)
                    | TyKind::Either(inner)
                    | TyKind::Provisional { inner, .. },
                ) => id = *inner,
                _ => return id,
            }
        }
    }

    /// Type `id` stripped (see [`TypeTable::strip`]) and then, if it is a
    /// record type, with each field's type made present in the same way,
    /// if it is a list type, with its element type made present, and if it
    /// is a union, with each member made present (the union built again,
    /// since that may make one member a subtype of another), recursively:
    /// the type of its value once awaited deeply. Provisional types are
    /// stripped like the either shapes they may become. Function types are
    /// left as they are, parameters and results alike. Interns the record,
    /// list and union types it makes.
    ///
    /// Recurses once per level of record, list or union nesting.
    pub fn present(&mut self, id: TyId) -> TyId {
        let top = self.strip(id);
        if let Some(members) = self.union_members(top) {
            // A copy (ids only), since presenting a member may intern types.
            let members = members.to_vec();
            let presented: Vec<TyId> = members.iter().map(|&member| self.present(member)).collect();
            return if presented == members {
                top
            } else {
                self.union(presented)
            };
        }
        if let Some(element) = self.list_element(top) {
            let presented = self.present(element);
            return if presented == element {
                top
            } else {
                self.list(presented)
            };
        }
        let Some(fields) = self.record_fields(top) else {
            return top;
        };
        // A copy (ids only), since presenting a field may intern types.
        let fields = fields.to_vec();
        let mut changed = false;
        let mut presented = Vec::with_capacity(fields.len());
        for (name, ty) in fields {
            let field = self.present(ty);
            changed |= field != ty;
            presented.push((name, field));
        }
        if changed {
            // Still sorted, so interning does not sort it again.
            self.record_like(top, presented)
        } else {
            top
        }
    }

    /// Whether a value of type `found`, where `expected` is expected, must
    /// be awaited to fit, and how: `None` if it fits as it is (`found <:
    /// expected`), [`AwaitKind::Top`] if it fits once awaited fully at the
    /// top (`strip(found) <: expected`), [`AwaitKind::Deep`] if it fits
    /// once awaited deeply (`present(found) <: expected`), and `None` again
    /// if it does not fit even then, which the caller tells from the first
    /// case by checking [`TypeTable::is_subtype`] itself first.
    pub fn needs_await(&mut self, found: TyId, expected: TyId) -> Option<AwaitKind> {
        if self.is_subtype(found, expected) {
            return None;
        }
        let stripped = self.strip(found);
        if stripped != found && self.is_subtype(stripped, expected) {
            return Some(AwaitKind::Top);
        }
        let present = self.present(found);
        if present != stripped && self.is_subtype(present, expected) {
            return Some(AwaitKind::Deep);
        }
        None
    }

    /// Whether type `id` is a `Future`, or a record type with a field of a
    /// type that is or contains one, or a list type whose element type is or
    /// contains one, or a union with a member that does, at any depth of
    /// record, list and union nesting. An either shape counts only through
    /// its present member, so `Int | Future<Int>` contains none, a
    /// provisional type likewise through its present type, and function
    /// types are not looked into.
    ///
    /// Recurses once per level of record, list or union nesting.
    #[must_use]
    pub fn contains_future(&self, id: TyId) -> bool {
        match self.kinds.get(id.index()) {
            Some(TyKind::Future(_)) => true,
            Some(
                TyKind::Provisional { inner, .. } | TyKind::List(inner) | TyKind::Either(inner),
            ) => self.contains_future(*inner),
            Some(TyKind::Union(members)) => {
                members.iter().any(|&member| self.contains_future(member))
            }
            Some(TyKind::Record(record)) => record
                .fields
                .iter()
                .any(|&(_, field)| self.contains_future(field)),
            _ => false,
        }
    }

    /// The record type `record` with field `name` set to type `ty`: the
    /// field's type replaced if `record` has it, and the field added
    /// otherwise. `None` if `record` is not a record type. The type of a
    /// functional update that sets one field. The other fields keep their
    /// defaults, if they have any; the field set has none, its type being
    /// another.
    pub fn with_field(&mut self, record: TyId, name: Name, ty: TyId) -> Option<TyId> {
        let fields = self.record_fields(record)?;
        let at = fields.partition_point(|&(field, _)| field < name);
        let replaces = fields.get(at).is_some_and(|&(field, _)| field == name);
        let mut updated = Vec::with_capacity(fields.len() + usize::from(!replaces));
        updated.extend_from_slice(fields.get(..at).unwrap_or_default());
        updated.push((name, ty));
        let rest = at + usize::from(replaces);
        updated.extend_from_slice(fields.get(rest..).unwrap_or_default());
        let defaults: Vec<(Name, Constant)> = self
            .defaults(record)
            .iter()
            .filter(|&&(field, _)| field != name)
            .cloned()
            .collect();
        // Still sorted, so interning does not sort it again.
        Some(self.record_with_defaults(updated, defaults))
    }

    /// Whether `a` and `b` are one type once their defaults are forgotten
    /// (see [Defaults](#defaults)): the same type, or two record types of
    /// the same fields whose types are so in turn, two list types whose
    /// element types are, two function types of the same parameter names,
    /// marker and meta marker whose parameter, return and meta types are,
    /// two futures, either shapes or provisional types of the same
    /// parameter whose inner types are, or two unions each of whose members
    /// is so with one of the other's. What the checker asks where it needs
    /// two types to be one, which defaults do not change.
    ///
    /// Recurses once per level of nesting shared by the two types.
    #[must_use]
    pub fn same(&self, a: TyId, b: TyId) -> bool {
        if a == b {
            return true;
        }
        let (Some(a_kind), Some(b_kind)) = (self.kinds.get(a.index()), self.kinds.get(b.index()))
        else {
            return false;
        };
        match (a_kind, b_kind) {
            (TyKind::Record(a_record), TyKind::Record(b_record)) => {
                a_record.fields.len() == b_record.fields.len()
                    && a_record.fields.iter().zip(&b_record.fields).all(
                        |(&(a_name, a_ty), &(b_name, b_ty))| {
                            a_name == b_name && self.same(a_ty, b_ty)
                        },
                    )
            }
            (TyKind::List(a_inner), TyKind::List(b_inner))
            | (TyKind::Future(a_inner), TyKind::Future(b_inner))
            | (TyKind::Either(a_inner), TyKind::Either(b_inner)) => self.same(*a_inner, *b_inner),
            (
                TyKind::Provisional {
                    function: a_function,
                    param: a_param,
                    inner: a_inner,
                },
                TyKind::Provisional {
                    function: b_function,
                    param: b_param,
                    inner: b_inner,
                },
            ) => a_function == b_function && a_param == b_param && self.same(*a_inner, *b_inner),
            (TyKind::Fn(a_sig), TyKind::Fn(b_sig)) => {
                let metas = match (a_sig.meta, b_sig.meta) {
                    (None, None) => true,
                    (Some(a_meta), Some(b_meta)) => {
                        a_meta.effect == b_meta.effect && self.same(a_meta.ty, b_meta.ty)
                    }
                    _ => false,
                };
                a_sig.effect == b_sig.effect
                    && metas
                    && self.same(a_sig.ret, b_sig.ret)
                    && a_sig.params.len() == b_sig.params.len()
                    && a_sig.params.iter().zip(&b_sig.params).all(
                        |(&(a_name, a_ty), &(b_name, b_ty))| {
                            a_name == b_name && self.same(a_ty, b_ty)
                        },
                    )
            }
            (TyKind::Union(a_members), TyKind::Union(b_members)) => {
                a_members.len() == b_members.len()
                    && a_members.iter().all(|&a_member| {
                        b_members
                            .iter()
                            .any(|&b_member| self.same(a_member, b_member))
                    })
            }
            _ => false,
        }
    }

    /// The kind of type `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this table.
    #[must_use]
    pub fn kind(&self, id: TyId) -> &TyKind {
        &self.kinds[id.index()]
    }

    /// The number of distinct types interned, the pre-interned ones
    /// included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// Always `false`: a table always holds the pre-interned types.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Whether `id` is [`TyId::ERROR`].
    #[must_use]
    pub fn is_error(&self, id: TyId) -> bool {
        id == TyId::ERROR
    }

    /// Whether `id` is a record type, the empty record included.
    #[must_use]
    pub fn is_record(&self, id: TyId) -> bool {
        matches!(self.kind(id), TyKind::Record(_))
    }

    /// The fields of record type `id`, sorted by name, or `None` if it is
    /// not a record type.
    #[must_use]
    pub fn record_fields(&self, id: TyId) -> Option<&[(Name, TyId)]> {
        match self.kind(id) {
            TyKind::Record(record) => Some(&record.fields),
            _ => None,
        }
    }

    /// The type of field `name` of record type `record`, or `None` if it has
    /// no such field or is not a record type.
    #[must_use]
    pub fn field(&self, record: TyId, name: Name) -> Option<TyId> {
        let fields = self.record_fields(record)?;
        let index = fields
            .binary_search_by_key(&name, |&(field, _)| field)
            .ok()?;
        fields.get(index).map(|&(_, ty)| ty)
    }

    /// Whether a value of type `sub` may be used where `sup` is expected:
    /// `sub` is `sup`; or both are record types and `sub` has every field
    /// of `sup`, each of a subtype of that field's type in `sup`; or both
    /// are list types and `sub`'s element type is a subtype of `sup`'s; or
    /// both are function types and `sub`'s signature fits `sup`'s, a meta
    /// `sup` has included (see [Metas](#metas)); or `sup` is
    /// `Future<T>` and `sub` is `Future<S>` with `S <: T`; or `sup` is `T |
    /// Future<T>` and `sub` stripped is a subtype of `T` (see the module
    /// documentation); or `sub` is a union each of whose members is a
    /// subtype of `sup`; or `sub` is not a union and `sup` is one with a
    /// member `sub` is a subtype of. Nothing lazy is a subtype of a present
    /// type. A provisional type, `T?`, is its either shape as `sub` (so it
    /// is a subtype of exactly what `T | Future<T>` is) and its present type
    /// `T` as `sup`, unless the two are the same provisional type.
    ///
    /// `Never`, which no value has, is a subtype of every type, and no
    /// other type is a subtype of it.
    ///
    /// Recurses once per level of record, list, function type, future or
    /// union nesting in the two types.
    #[must_use]
    pub fn is_subtype(&self, sub: TyId, sup: TyId) -> bool {
        if sub == sup || self.is_never(sub) {
            return true;
        }
        match self.kinds.get(sup.index()) {
            Some(TyKind::Provisional { inner, .. }) => return self.is_subtype(sub, *inner),
            Some(TyKind::Future(required)) => {
                return match self.kinds.get(sub.index()) {
                    Some(TyKind::Future(offered)) => self.is_subtype(*offered, *required),
                    _ => false,
                };
            }
            Some(&TyKind::Either(required)) => {
                return self.is_subtype(self.strip(sub), required);
            }
            _ => {}
        }
        if self.laziness(sub) != Laziness::Present {
            return false;
        }
        if let Some(members) = self.union_members(sub) {
            return members.iter().all(|&member| self.is_subtype(member, sup));
        }
        if let Some(members) = self.union_members(sup) {
            return members.iter().any(|&member| self.is_subtype(sub, member));
        }
        if let (Some(offered), Some(required)) = (self.list_element(sub), self.list_element(sup)) {
            return self.is_subtype(offered, required);
        }
        if let (Some(offered), Some(required)) = (self.fn_sig(sub), self.fn_sig(sup)) {
            // A meta is purely additive: one required must be offered, as a
            // result is, and one offered may be forgotten.
            let meta = match (offered.meta, required.meta) {
                (_, None) => true,
                (None, Some(_)) => false,
                (Some(offered), Some(required)) => {
                    offered.effect <= required.effect && self.is_subtype(offered.ty, required.ty)
                }
            };
            return offered.params.len() == required.params.len()
                && offered.effect <= required.effect
                && meta
                && self.is_subtype(offered.ret, required.ret)
                && offered
                    .params
                    .iter()
                    .zip(&required.params)
                    .all(|(&(_, accepted), &(_, passed))| self.is_subtype(passed, accepted));
        }
        let (Some(offered), Some(required)) = (self.record_fields(sub), self.record_fields(sup))
        else {
            return false;
        };
        // Both lists are sorted by name, so one merge-like walk finds every
        // required field among the offered ones.
        let mut offered = offered.iter();
        required.iter().all(|&(name, required_ty)| {
            offered
                .by_ref()
                .find(|&&(offered_name, _)| offered_name >= name)
                .is_some_and(|&(offered_name, offered_ty)| {
                    offered_name == name && self.is_subtype(offered_ty, required_ty)
                })
        })
    }

    /// The least common supertype of `a` and `b`, for the branches of an
    /// `if` and the elements of a list, and whether making it *widened*:
    /// introduced a union anywhere in it, which the checker warns about.
    /// It always exists (see the module documentation):
    ///
    /// - `a` itself if the two are the same type, not widened;
    /// - the other if either is `Never`, which adds no value, not widened;
    /// - `Error` if either is `Error`, not widened, the error having been
    ///   reported; an untyped integer joins as the `Int` it settles to
    ///   (numeric adaptation is the checker's business, done before it
    ///   joins);
    /// - for two futures, the future of the join of what they yield; for a
    ///   future, an either or a provisional type (which joins as its either
    ///   shape) with anything else, the either of the join of both sides
    ///   stripped; widened if that join is;
    /// - for a union and a type, the one of the two that is a supertype of
    ///   the other, not widened, or else the union of both sides' members,
    ///   widened;
    /// - for two list types, the list of the join of their element types;
    /// - for two function types, the join described in the module
    ///   documentation, widened if the join of their return types is, or
    ///   their union, widened, if they have different arities or a pair of
    ///   parameter types neither of which is a subtype of the other;
    /// - for two record types, the record of the fields they share, each
    ///   with the join of its two types (the empty record if there are
    ///   none), widened if any of those is;
    /// - for any other two types, their union, widened.
    ///
    /// Recurses once per level of record, list, return type or future
    /// nesting shared by `a` and `b`.
    pub fn join(&mut self, a: TyId, b: TyId) -> (TyId, bool) {
        if a == b || self.is_never(b) {
            return (a, false);
        }
        if self.is_never(a) {
            return (b, false);
        }
        if a == TyId::ERROR || b == TyId::ERROR {
            return (TyId::ERROR, false);
        }
        let settled = |ty: TyId| {
            if ty == TyId::UNTYPED_INT {
                TyId::INT
            } else {
                ty
            }
        };
        let (a, b) = (settled(a), settled(b));
        if a == b {
            return (a, false);
        }
        if self.laziness(a) != Laziness::Present || self.laziness(b) != Laziness::Present {
            if let (TyKind::Future(a_inner), TyKind::Future(b_inner)) = (self.kind(a), self.kind(b))
            {
                let (a_inner, b_inner) = (*a_inner, *b_inner);
                let (joined, widened) = self.join(a_inner, b_inner);
                return (self.future(joined), widened);
            }
            let (a_present, b_present) = (self.strip(a), self.strip(b));
            let (joined, widened) = self.join(a_present, b_present);
            return (self.deferrable(joined), widened);
        }
        if self.union_members(a).is_some() || self.union_members(b).is_some() {
            return if self.is_subtype(a, b) {
                (b, false)
            } else if self.is_subtype(b, a) {
                (a, false)
            } else {
                (self.union([a, b]), true)
            };
        }
        if let (Some(a_element), Some(b_element)) = (self.list_element(a), self.list_element(b)) {
            let (joined, widened) = self.join(a_element, b_element);
            return (self.list(joined), widened);
        }
        if self.fn_sig(a).is_some() && self.fn_sig(b).is_some() {
            return self.join_fns(a, b);
        }
        if self.is_record(a) && self.is_record(b) {
            return self.join_records(a, b);
        }
        (self.union([a, b]), true)
    }

    /// The join of the function types `a` and `b`, and whether it is
    /// widened, as [`TypeTable::join`] says.
    fn join_fns(&mut self, a: TyId, b: TyId) -> (TyId, bool) {
        let (Some(a_sig), Some(b_sig)) = (self.fn_sig(a), self.fn_sig(b)) else {
            return (self.union([a, b]), true);
        };
        // Parameters are contravariant, so each is the narrower of the two,
        // which both functions accept: one of them, if it is a subtype of
        // the other. A copy, since joining the return types may intern new
        // types and so needs the table mutably.
        let params: Option<Vec<(Option<Name>, TyId)>> = if a_sig.params.len() == b_sig.params.len()
        {
            a_sig
                .params
                .iter()
                .zip(&b_sig.params)
                .map(|(&(a_name, a_param), &(b_name, b_param))| {
                    let param = if self.is_subtype(a_param, b_param) {
                        a_param
                    } else if self.is_subtype(b_param, a_param) {
                        b_param
                    } else {
                        return None;
                    };
                    Some((a_name.filter(|_| a_name == b_name), param))
                })
                .collect()
        } else {
            None
        };
        let Some(params) = params else {
            return (self.union([a, b]), true);
        };
        let (a_ret, b_ret, effect) = (a_sig.ret, b_sig.ret, a_sig.effect.max(b_sig.effect));
        let metas = (a_sig.meta, b_sig.meta);
        let (ret, mut widened) = self.join(a_ret, b_ret);
        // Metas join as results do; one side's alone is forgotten (see
        // [Metas](#metas)).
        let meta = match metas {
            (Some(a_meta), Some(b_meta)) => {
                let (ty, meta_widened) = self.join(a_meta.ty, b_meta.ty);
                widened |= meta_widened;
                Some(Meta {
                    effect: a_meta.effect.max(b_meta.effect),
                    ty,
                })
            }
            _ => None,
        };
        (self.fn_type_with_meta(params, ret, effect, meta), widened)
    }

    /// The join of the record types `a` and `b`, and whether it is widened,
    /// as [`TypeTable::join`] says.
    fn join_records(&mut self, a: TyId, b: TyId) -> (TyId, bool) {
        let (Some(a_fields), Some(b_fields)) = (self.record_fields(a), self.record_fields(b))
        else {
            return (self.union([a, b]), true);
        };
        // Copies of the two field lists (ids only), since joining a field
        // may intern new types and so needs the table mutably.
        let (a_fields, b_fields) = (a_fields.to_vec(), b_fields.to_vec());
        let mut shared = Vec::with_capacity(a_fields.len().min(b_fields.len()));
        let mut widened = false;
        for &(name, a_ty) in &a_fields {
            let Ok(index) = b_fields.binary_search_by_key(&name, |&(field, _)| field) else {
                continue;
            };
            let Some(&(_, b_ty)) = b_fields.get(index) else {
                continue;
            };
            let (joined, field_widened) = self.join(a_ty, b_ty);
            widened |= field_widened;
            shared.push((name, joined));
        }
        // `shared` is in `a`'s order, which is sorted, so interning does
        // not sort it again.
        (self.record(shared), widened)
    }

    /// The kind of value an expression of type `id` produces at runtime, or
    /// `None` for the checker-internal types, which no value has: an untyped
    /// integer, a provisional type or an error never survives a successful
    /// check. A `Future` and an either shape are [`ValueKind::Deferred`]: a
    /// value that may be a future. A union whose members are all of one
    /// kind, as `List<Int> | List<String>` and `{a: Int} | {b: Int}` are,
    /// is of that kind, and any other union is [`ValueKind::Mixed`]: a
    /// present value of a kind only the value itself says. `Never` is
    /// [`ValueKind::Never`], the kind of no value. Also `None`,
    /// rather than a panic, for an id this table did not produce, so that
    /// lowering can report a mismatched table as an internal error.
    #[must_use]
    pub fn value_kind(&self, id: TyId) -> Option<ValueKind> {
        match self.kinds.get(id.index())? {
            TyKind::Never => Some(ValueKind::Never),
            TyKind::Int => Some(ValueKind::Int),
            TyKind::Float => Some(ValueKind::Float),
            TyKind::Bool => Some(ValueKind::Bool),
            TyKind::String => Some(ValueKind::String),
            TyKind::Record(_) => Some(ValueKind::Record),
            TyKind::List(_) => Some(ValueKind::List),
            // TEMPORARY (alias rule): a special built-in's opaque type is a
            // function's kind (R11 of `docs/INFERENCE_REQS.md`).
            TyKind::Fn(_) | TyKind::Builtin(_) => Some(ValueKind::Fn),
            TyKind::Future(_) | TyKind::Either(_) => Some(ValueKind::Deferred),
            TyKind::Union(members) => {
                let mut kinds = members.iter().map(|&member| self.value_kind(member));
                let first = kinds.next()??;
                Some(if kinds.all(|kind| kind == Some(first)) {
                    first
                } else {
                    ValueKind::Mixed
                })
            }
            TyKind::Provisional { .. } | TyKind::UntypedInt | TyKind::Error => None,
        }
    }

    /// Whether a value could be both of type `a` and of type `b`, present
    /// types, as far as their structure says: whether one of them is a
    /// union with a member that overlaps the other; or both are the same
    /// scalar type; or both are record types and each field they both have
    /// has types that overlap (a field only one of them has may be a field
    /// the other's value has without its type saying so); or both are list
    /// types whose element types overlap; or both are function types,
    /// whose signatures no value can be asked for. `true` if either is
    /// `Error`, which has been reported.
    ///
    /// Recurses once per level of record, list or union nesting shared by
    /// the two types.
    #[must_use]
    pub fn overlaps(&self, a: TyId, b: TyId) -> bool {
        if a == b || a == TyId::ERROR || b == TyId::ERROR {
            return true;
        }
        if let Some(members) = self.union_members(a) {
            return members.iter().any(|&member| self.overlaps(member, b));
        }
        if let Some(members) = self.union_members(b) {
            return members.iter().any(|&member| self.overlaps(a, member));
        }
        match (self.kind(a), self.kind(b)) {
            (TyKind::Record(a_record), TyKind::Record(_)) => {
                a_record.fields.iter().all(|&(name, a_field)| {
                    self.field(b, name)
                        .is_none_or(|b_field| self.overlaps(a_field, b_field))
                })
            }
            (TyKind::List(a_element), TyKind::List(b_element)) => {
                self.overlaps(*a_element, *b_element)
            }
            (TyKind::Fn(_), TyKind::Fn(_)) => true,
            _ => false,
        }
    }

    /// The plan for testing at runtime whether a value is of type `ty`, a
    /// present type with no future and no function type in it, given that
    /// it is of one of the present types `value`, a union's members
    /// flattened, or of any type at all for `None` (see [`ValueTest`]):
    /// [`ValueTest::Pass`] if every type of `value` is a subtype of `ty`;
    /// for a union, a test of whether any member passes; for a scalar type,
    /// a test of the value's kind; and for a record or list type, a test of
    /// the value's kind alone if every type of `value` of that kind is a
    /// subtype of `ty`, and otherwise a test of the record's fields of
    /// `ty`, each against what the record types of `value` say of the field
    /// (nothing, if one lacks it), or of the list's elements, against the
    /// element types of its list types, leaving out a field whose test is
    /// `Pass`.
    ///
    /// Recurses once per level of record, list or union nesting in `ty`.
    #[must_use]
    pub fn test_plan(&self, value: Option<&[TyId]>, ty: TyId) -> ValueTest {
        if let Some(value) = value
            && value.iter().all(|&member| self.is_subtype(member, ty))
        {
            return ValueTest::Pass;
        }
        if let Some(alternatives) = self.union_members(ty) {
            let tests: Vec<ValueTest> = alternatives
                .iter()
                .map(|&alternative| self.test_plan(value, alternative))
                .collect();
            return if tests.contains(&ValueTest::Pass) {
                ValueTest::Pass
            } else {
                ValueTest::Any(tests.into_boxed_slice())
            };
        }
        let Some(kind) = self.value_kind(ty) else {
            return ValueTest::Pass;
        };
        // The types the value may have that are of the kind tested, which
        // the test of the kind leaves in doubt only if one does not pass.
        let of_kind: Option<Vec<TyId>> = value.map(|value| {
            value
                .iter()
                .copied()
                .filter(|&member| self.value_kind(member) == Some(kind))
                .collect()
        });
        if of_kind
            .as_ref()
            .is_some_and(|of_kind| of_kind.iter().all(|&member| self.is_subtype(member, ty)))
        {
            return ValueTest::Kind(kind);
        }
        if let Some(element) = self.list_element(ty) {
            let elements = of_kind.map(|lists| {
                self.flattened(lists.iter().filter_map(|&list| self.list_element(list)))
            });
            return match self.test_plan(elements.as_deref(), element) {
                ValueTest::Pass => ValueTest::Kind(ValueKind::List),
                test => ValueTest::List(Box::new(test)),
            };
        }
        if let Some(fields) = self.record_fields(ty) {
            let mut tests = Vec::with_capacity(fields.len());
            for &(name, field_ty) in fields {
                let values = of_kind.as_ref().and_then(|records| {
                    let found: Option<Vec<TyId>> = records
                        .iter()
                        .map(|&record| self.field(record, name))
                        .collect();
                    found.map(|found| self.flattened(found))
                });
                match self.test_plan(values.as_deref(), field_ty) {
                    ValueTest::Pass => {}
                    test => tests.push((name, test)),
                }
            }
            return if tests.is_empty() {
                ValueTest::Kind(ValueKind::Record)
            } else {
                ValueTest::Record(tests.into_boxed_slice())
            };
        }
        ValueTest::Kind(kind)
    }

    /// `types`, each union among them replaced by its members, without
    /// duplicates: the types a value of any of them may have.
    fn flattened(&self, types: impl IntoIterator<Item = TyId>) -> Vec<TyId> {
        let mut flat = Vec::new();
        for ty in types {
            match self.union_members(ty) {
                Some(members) => flat.extend_from_slice(members),
                None => flat.push(ty),
            }
        }
        flat.sort_unstable();
        flat.dedup();
        flat
    }

    /// Shows type `id` as it is written in source, using `names` for the
    /// names of record fields and parameters: `Int`, `{x: Int, y: String}`,
    /// `fn(Int, name: String): Bool`, `fn() -> Int`. A record shows its
    /// fields in alphabetical order of their text, a field whose name is not
    /// a plain identifier quoted as a string literal (`{"a b": Int}`), a
    /// field with a default followed by ` = ` and the default as a value's
    /// `repr` shows it (`{exclude: List<String> = [], from: String}`), and a
    /// nested record nested. The empty record is `{}`. A list type shows as
    /// `List<Int>`, however the source wrote it. A function shows its
    /// parameters in order, each with its name if it has one, then `: ` and
    /// its return type if it is pure, ` ->> ` and its return type if it is
    /// memoised, or ` -> ` and its return type if it is impure, and then its
    /// meta, if it has one, as ` META-> ` and its type (or `META:`,
    /// `META->>`), the return type in parentheses if it is a function type,
    /// which would take the meta, `fn(): (fn(): Int) META: {a: Int}`.
    /// A future shows as `Future<Int>`, and the either shape as `Int |
    /// Future<Int>`, present member first, in parentheses if it is a
    /// function type, `(fn(): Int) | Future<fn(): Int>`, since a function
    /// type's return type takes a union that follows it, as in `fn() -> Int
    /// | Future<Int>`, which returns the either. A provisional type, which
    /// only the types of a failed check hold, shows as its present type
    /// followed by `?`, `Int?`, the present type in parentheses if it is a
    /// function type or a union, `(fn(): Int)?` and `(Int | String)?`, as
    /// for the either shape. An untyped integer shows as `Int`, and an error
    /// as `<error>`. A union shows as its members separated by ` | `, in the
    /// order of their kinds in [`TyKind`] (`Int`, `Float`, `Bool`, `String`,
    /// records, lists, functions) and, among members of one kind, of their
    /// text, each function type in parentheses, as in `Int | {a: Int} |
    /// (fn(): Int)`; the either of a union shows the union as its present
    /// member, as in `Int | String | Future<Int | String>`.
    ///
    /// `names` must be the table the names were interned into. Recurses
    /// once per level of record, function type or union nesting.
    #[must_use]
    pub fn display<'a>(&'a self, id: TyId, names: &'a Names) -> impl fmt::Display + 'a {
        Display {
            table: self,
            names,
            id,
        }
    }

    /// The `members` of a union, each with its text as
    /// [`TypeTable::display`] shows it (a function type without the
    /// parentheses a union puts around it), in the order a union shows its
    /// members: by kind, in the order of [`TyKind`] (`Int`, `Float`,
    /// `Bool`, `String`, records, lists, functions), and among members of
    /// one kind by text.
    #[must_use]
    pub fn shown_members(&self, members: &[TyId], names: &Names) -> Vec<(TyId, String)> {
        let rank = |member: TyId| match self.kinds.get(member.index()) {
            Some(TyKind::Int | TyKind::UntypedInt) => 0,
            Some(TyKind::Float) => 1,
            Some(TyKind::Bool) => 2,
            Some(TyKind::String) => 3,
            Some(TyKind::Record(_)) => 4,
            Some(TyKind::List(_)) => 5,
            Some(TyKind::Fn(_)) => 6,
            _ => 7,
        };
        let mut shown: Vec<(u8, String, TyId)> = members
            .iter()
            .map(|&member| {
                (
                    rank(member),
                    self.display(member, names).to_string(),
                    member,
                )
            })
            .collect();
        shown.sort_unstable();
        shown
            .into_iter()
            .map(|(_, text, member)| (member, text))
            .collect()
    }
}

/// A type shown as it is written in source. Returned by
/// [`TypeTable::display`].
struct Display<'a> {
    table: &'a TypeTable,
    names: &'a Names,
    id: TyId,
}

impl Display<'_> {
    /// Type `id` shown with the same table and names.
    fn nested(&self, id: TyId) -> Self {
        Self {
            table: self.table,
            names: self.names,
            id,
        }
    }

    /// Writes the union of `members` (see [`TypeTable::display`]): in the
    /// order [`TypeTable::shown_members`] gives, separated by ` | `,
    /// function types in parentheses, since a function type's return type
    /// would take the members after it.
    fn union(&self, members: &[TyId], f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (member, text)) in self
            .table
            .shown_members(members, self.names)
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                f.write_str(" | ")?;
            }
            if self.table.fn_sig(member).is_some() {
                write!(f, "({text})")?;
            } else {
                f.write_str(&text)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.table.kind(self.id) {
            TyKind::Int | TyKind::UntypedInt => f.write_str("Int"),
            TyKind::Float => f.write_str("Float"),
            TyKind::Bool => f.write_str("Bool"),
            TyKind::String => f.write_str("String"),
            TyKind::Error => f.write_str("<error>"),
            TyKind::Never => f.write_str("Never"),
            // TEMPORARY (alias rule): the opaque type of a special
            // built-in (R11 of `docs/INFERENCE_REQS.md`).
            TyKind::Builtin(builtin) => write!(f, "builtin {}", builtin.name()),
            TyKind::Future(inner) => write!(f, "Future<{}>", self.nested(*inner)),
            TyKind::List(element) => write!(f, "List<{}>", self.nested(*element)),
            TyKind::Provisional { inner, .. } => {
                let shown = self.nested(*inner);
                // A function type's return type, or a union's last member,
                // would take the `?`.
                if self.table.fn_sig(*inner).is_some() || self.table.union_members(*inner).is_some()
                {
                    write!(f, "({shown})?")
                } else {
                    write!(f, "{shown}?")
                }
            }
            TyKind::Either(present) => {
                let shown = self.nested(*present);
                // A function type's return type would take the union.
                if self.table.fn_sig(*present).is_some() {
                    write!(f, "({shown}) | Future<{shown}>")
                } else {
                    write!(f, "{shown} | Future<{shown}>")
                }
            }
            TyKind::Union(members) => self.union(members, f),
            TyKind::Record(record) => {
                // Stored by `Name`, shown by text.
                let mut fields: Vec<(&str, Name, TyId)> = record
                    .fields
                    .iter()
                    .map(|&(name, ty)| (self.names.get(name), name, ty))
                    .collect();
                fields.sort_unstable_by_key(|&(text, _, _)| text);
                f.write_str("{")?;
                for (index, &(text, name, ty)) in fields.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: {}", FieldName(text), self.nested(ty))?;
                    // A default after its field's type, as it is written.
                    if let Some(constant) = self.table.field_default(self.id, name) {
                        f.write_str(" = ")?;
                        write_constant(f, constant, self.names)?;
                    }
                }
                f.write_str("}")
            }
            TyKind::Fn(sig) => {
                f.write_str("fn(")?;
                for (index, &(name, ty)) in sig.params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    if let Some(name) = name {
                        write!(f, "{}: ", FieldName(self.names.get(name)))?;
                    }
                    self.nested(ty).fmt(f)?;
                }
                f.write_str(sig.effect.params_end())?;
                let Some(meta) = sig.meta else {
                    return self.nested(sig.ret).fmt(f);
                };
                // A return type that is a function type would take the
                // meta.
                if self.table.fn_sig(sig.ret).is_some() {
                    write!(f, "({})", self.nested(sig.ret))?;
                } else {
                    self.nested(sig.ret).fmt(f)?;
                }
                write!(f, " {} {}", meta.effect.meta_marker(), self.nested(meta.ty))
            }
        }
    }
}

/// Writes `constant`, a field's default, as it would be written in source,
/// as a value's `repr` shows it (see [`crate::Value`]), its record fields'
/// names shown with `names`: an `Int` in decimal, a `Float` with a decimal
/// point or an exponent, by the shortest text that reads back as it, `nan`
/// and `inf` as a value shows them, a `Bool` as `true` or `false`, a
/// `String` quoted ([`Quoted`]), a list as `[1, 2]`, a record as `{a = 1;
/// b = "x"}`, its fields in the order they are written, and the empty
/// record as `{}`.
///
/// Recurses once per level of list and record nesting.
pub(crate) fn write_constant(
    f: &mut fmt::Formatter<'_>,
    constant: &Constant,
    names: &Names,
) -> fmt::Result {
    match constant {
        Constant::Int(value) => write!(f, "{value}"),
        Constant::Float(bits) => {
            let value = f64::from_bits(*bits);
            // `{:?}` would show NaN as `NaN`.
            if value.is_nan() {
                f.write_str("nan")
            } else {
                write!(f, "{value:?}")
            }
        }
        Constant::Bool(value) => write!(f, "{value}"),
        Constant::Str(text) => write!(f, "{}", Quoted(text)),
        Constant::Empty => f.write_str("{}"),
        Constant::List(elements) => {
            f.write_str("[")?;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write_constant(f, element, names)?;
            }
            f.write_str("]")
        }
        Constant::Record(fields) => {
            f.write_str("{")?;
            for (index, (name, value)) in fields.iter().enumerate() {
                if index > 0 {
                    f.write_str("; ")?;
                }
                write!(f, "{} = ", FieldName(names.get(*name)))?;
                write_constant(f, value, names)?;
            }
            f.write_str("}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table and names with `a`, `b`, `c` interned in that order.
    fn setup() -> (TypeTable, Names, [Name; 3]) {
        let mut names = Names::new();
        let abc = [names.intern("a"), names.intern("b"), names.intern("c")];
        (TypeTable::new(), names, abc)
    }

    #[test]
    fn the_constants_are_pre_interned_in_order() {
        let mut table = TypeTable::new();
        assert_eq!(table.len(), 7);
        assert!(!table.is_empty());
        assert_eq!(table.intern(TyKind::Int), TyId::INT);
        assert_eq!(table.intern(TyKind::Float), TyId::FLOAT);
        assert_eq!(table.intern(TyKind::Bool), TyId::BOOL);
        assert_eq!(table.intern(TyKind::String), TyId::STRING);
        assert_eq!(table.record([]), TyId::EMPTY);
        assert_eq!(table.intern(TyKind::UntypedInt), TyId::UNTYPED_INT);
        assert_eq!(table.intern(TyKind::Error), TyId::ERROR);
        assert_eq!(table.len(), 7);
        assert_eq!(TypeTable::default().len(), 7);
    }

    #[test]
    fn a_kind_is_small() {
        // A record type's defaults are behind a thin pointer, which a
        // record type without any, nearly every one, leaves null: so a
        // record type is 24 bytes, and a kind no larger than before, a
        // function type's signature being the largest.
        assert_eq!(size_of::<RecordTy>(), 24);
        assert_eq!(size_of::<TyKind>(), 32);
    }

    #[test]
    fn a_record_types_defaults_change_nothing_it_admits() {
        let (mut table, _, [a, b, c]) = setup();
        let one = || Constant::Int(1);
        let plain = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        let defaulted =
            table.record_with_defaults([(a, TyId::INT), (b, TyId::STRING)], vec![(a, one())]);
        // A type of its own, interned by its defaults too, in any order.
        assert_ne!(defaulted, plain);
        assert_eq!(
            table.record_with_defaults([(b, TyId::STRING), (a, TyId::INT)], vec![(a, one())]),
            defaulted
        );
        let other = table.record_with_defaults(
            [(a, TyId::INT), (b, TyId::STRING)],
            vec![(a, Constant::Int(2))],
        );
        assert_ne!(other, defaulted);
        // No default is none.
        assert_eq!(
            table.record_with_defaults([(a, TyId::INT), (b, TyId::STRING)], Vec::new()),
            plain
        );
        assert_eq!(table.defaults(defaulted), &[(a, one())][..]);
        assert_eq!(table.field_default(defaulted, a), Some(&one()));
        assert_eq!(table.field_default(defaulted, b), None);
        assert!(table.defaults(plain).is_empty());
        assert!(table.defaults(TyId::INT).is_empty());
        // Each a subtype of the other, and one type once they are
        // forgotten, at any depth.
        assert!(table.is_subtype(plain, defaulted));
        assert!(table.is_subtype(defaulted, plain));
        assert!(table.same(plain, defaulted));
        assert!(table.same(defaulted, other));
        let (plain_list, defaulted_list) = (table.list(plain), table.list(defaulted));
        assert!(table.same(plain_list, defaulted_list));
        let plain_fn = table.fn_type([(Some(c), plain)], plain_list, Effect::Pure);
        let defaulted_fn = table.fn_type([(Some(c), defaulted)], defaulted_list, Effect::Pure);
        assert!(table.same(plain_fn, defaulted_fn));
        let (plain_either, defaulted_either) =
            (table.deferrable(plain), table.deferrable(defaulted));
        assert!(table.same(plain_either, defaulted_either));
        let plain_union = table.union([plain_list, TyId::INT]);
        let defaulted_union = table.union([defaulted_list, TyId::INT]);
        assert!(table.same(plain_union, defaulted_union));
        // But not where anything else differs.
        let wider = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
        assert!(!table.same(plain, wider));
        let unnamed = table.fn_type([(None, plain)], plain_list, Effect::Pure);
        assert!(!table.same(plain_fn, unnamed));
        let impure = table.fn_type([(Some(c), defaulted)], defaulted_list, Effect::Impure);
        assert!(!table.same(plain_fn, impure));
        assert!(!table.same(plain_list, plain));
        // The join has none, and is not widened; a union keeps, of the two,
        // the one interned first.
        assert_eq!(table.join(defaulted, plain), (plain, false));
        assert_eq!(table.join(defaulted, defaulted), (defaulted, false));
        assert_eq!(table.union([defaulted, plain]), plain);
        let union = table.union([defaulted, TyId::BOOL]);
        assert_eq!(
            table.union_members(union),
            Some(&[TyId::BOOL, defaulted][..])
        );
        let kept = table.union([TyId::BOOL, plain]);
        assert_eq!(table.union([union, plain]), kept);
    }

    #[test]
    fn a_record_types_defaults_are_kept_and_shown() {
        let (mut table, names, [a, b, c]) = setup();
        let one = || Constant::Int(1);
        let defaulted =
            table.record_with_defaults([(a, TyId::INT), (b, TyId::STRING)], vec![(a, one())]);
        let plain = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        let plain_list = table.list(plain);
        // A record type rebuilt from its fields keeps theirs; a field a
        // functional update sets loses its own.
        let lazy_int = table.deferrable(TyId::INT);
        let lazy = table.record_with_defaults([(a, lazy_int), (b, TyId::STRING)], vec![(a, one())]);
        assert_eq!(table.present(lazy), defaulted);
        assert_eq!(
            table
                .with_field(defaulted, c, TyId::BOOL)
                .map(|set| table.defaults(set).len()),
            Some(1)
        );
        let set = table.with_field(defaulted, a, TyId::FLOAT);
        assert_eq!(set.map(|set| table.defaults(set).is_empty()), Some(true));
        // Shown after their fields' types, each as a value's `repr` shows
        // it, a record's fields in the order they are written.
        let every = table.record_with_defaults(
            [(a, defaulted), (b, TyId::FLOAT), (c, TyId::STRING)],
            vec![
                (
                    a,
                    Constant::Record(Box::new([(b, Constant::Str("x\"y".into())), (a, one())])),
                ),
                (b, Constant::Float(2.0f64.to_bits())),
                (c, Constant::Str("{z}".into())),
            ],
        );
        assert_eq!(
            table.display(every, &names).to_string(),
            "{a: {a: Int = 1, b: String} = {b = \"x\\\"y\"; a = 1}, b: Float = 2.0, \
             c: String = \"{{z}}\"}"
        );
        let lists = table.list(TyId::FLOAT);
        let more = table.record_with_defaults(
            [(a, lists), (b, TyId::FLOAT), (c, TyId::EMPTY)],
            vec![
                (
                    a,
                    Constant::List(Box::new([
                        Constant::Float(f64::NAN.to_bits()),
                        Constant::Float(f64::NEG_INFINITY.to_bits()),
                    ])),
                ),
                (b, Constant::Float((-0.5f64).to_bits())),
                (c, Constant::Empty),
            ],
        );
        assert_eq!(
            table.display(more, &names).to_string(),
            "{a: List<Float> = [nan, -inf], b: Float = -0.5, c: {} = {}}"
        );
        let bools = table.record_with_defaults(
            [(a, TyId::BOOL), (b, plain_list)],
            vec![
                (a, Constant::Bool(false)),
                (b, Constant::List(Box::new([]))),
            ],
        );
        assert_eq!(
            table.display(bools, &names).to_string(),
            "{a: Bool = false, b: List<{a: Int, b: String}> = []}"
        );
    }

    #[test]
    fn interning_is_by_structure() {
        let (mut table, _, [a, b, _]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        // The same fields in another order are the same type.
        assert_eq!(table.record([(b, TyId::STRING), (a, TyId::INT)]), ab);
        assert_eq!(
            table.intern(TyKind::Record(RecordTy {
                fields: Box::new([(b, TyId::STRING), (a, TyId::INT)]),
                defaults: None,
            })),
            ab
        );
        // A different field type, a different field name, or a missing
        // field is a different type.
        assert_ne!(table.record([(a, TyId::INT), (b, TyId::INT)]), ab);
        assert_ne!(table.record([(a, TyId::INT)]), ab);
        assert_ne!(table.record([(b, TyId::INT), (a, TyId::STRING)]), ab);
        let before = table.len();
        let nested = table.record([(a, ab)]);
        assert_eq!(table.record([(a, ab)]), nested);
        assert_eq!(table.len(), before + 1);
    }

    #[test]
    fn record_fields_are_sorted_by_name() {
        let (mut table, _, [a, b, c]) = setup();
        let id = table.record([(c, TyId::BOOL), (a, TyId::INT), (b, TyId::FLOAT)]);
        assert_eq!(
            table.record_fields(id),
            Some(&[(a, TyId::INT), (b, TyId::FLOAT), (c, TyId::BOOL)][..])
        );
        assert_eq!(table.field(id, b), Some(TyId::FLOAT));
        assert_eq!(table.field(TyId::EMPTY, b), None);
        assert_eq!(table.field(TyId::INT, b), None);
        assert_eq!(table.record_fields(TyId::STRING), None);
        assert_eq!(table.record_fields(TyId::EMPTY), Some(&[][..]));
        assert!(table.is_record(id));
        assert!(table.is_record(TyId::EMPTY));
        assert!(!table.is_record(TyId::INT));
        assert!(table.is_error(TyId::ERROR));
        assert!(!table.is_error(TyId::EMPTY));
    }

    #[test]
    fn setting_a_field_replaces_or_adds_it() {
        let (mut table, _, [a, b, c]) = setup();
        let ac = table.record([(a, TyId::INT), (c, TyId::BOOL)]);
        // Added in its place among the sorted fields, at the start, middle
        // or end.
        let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
        assert_eq!(table.with_field(ac, b, TyId::STRING), Some(abc));
        let bc = table.record([(b, TyId::FLOAT), (c, TyId::BOOL)]);
        let c_only = table.record([(c, TyId::BOOL)]);
        assert_eq!(table.with_field(c_only, b, TyId::FLOAT), Some(bc));
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        assert_eq!(table.with_field(a_only, b, TyId::INT), Some(ab));
        assert_eq!(table.with_field(TyId::EMPTY, a, TyId::INT), Some(a_only));
        // Replaced, the type changing.
        let ac_float = table.record([(a, TyId::INT), (c, TyId::FLOAT)]);
        assert_eq!(table.with_field(ac, c, TyId::FLOAT), Some(ac_float));
        assert_eq!(table.with_field(ac, a, TyId::INT), Some(ac));
        // Nested records are types like any other.
        let nested = table.record([(a, ac)]);
        assert_eq!(table.with_field(TyId::EMPTY, a, ac), Some(nested));
        // Only records have fields to set.
        assert_eq!(table.with_field(TyId::INT, a, TyId::INT), None);
    }

    #[test]
    fn display_every_kind() {
        let mut names = Names::new();
        let (y, x, spaced, keyword, dashed) = (
            names.intern("y"),
            names.intern("x"),
            names.intern("a b"),
            names.intern("let"),
            names.intern("hello-world"),
        );
        let mut table = TypeTable::new();
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        assert_eq!(show(&table, TyId::INT), "Int");
        assert_eq!(show(&table, TyId::FLOAT), "Float");
        assert_eq!(show(&table, TyId::BOOL), "Bool");
        assert_eq!(show(&table, TyId::STRING), "String");
        assert_eq!(show(&table, TyId::EMPTY), "{}");
        assert_eq!(show(&table, TyId::UNTYPED_INT), "Int");
        assert_eq!(show(&table, TyId::ERROR), "<error>");
        // Alphabetical by text, although `y` was interned first.
        let xy = table.record([(y, TyId::STRING), (x, TyId::INT)]);
        assert_eq!(show(&table, xy), "{x: Int, y: String}");
        // Names that are not plain identifiers are quoted; keywords are not
        // identifiers.
        let quoted = table.record([(spaced, TyId::INT), (keyword, TyId::BOOL), (dashed, xy)]);
        assert_eq!(
            show(&table, quoted),
            "{\"a b\": Int, hello-world: {x: Int, y: String}, \"let\": Bool}"
        );
        let nested = table.record([(x, TyId::EMPTY)]);
        assert_eq!(show(&table, nested), "{x: {}}");
        let union = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::STRING])));
        assert_eq!(show(&table, union), "Int | String");
    }

    #[test]
    fn subtyping_is_width_and_depth() {
        let (mut table, _, [a, b, c]) = setup();
        let a_int = table.record([(a, TyId::INT)]);
        let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
        // Width: more fields is a subtype of fewer.
        assert!(table.is_subtype(ab, a_int));
        assert!(table.is_subtype(abc, a_int));
        assert!(table.is_subtype(abc, ab));
        assert!(!table.is_subtype(a_int, ab));
        // Reflexive.
        assert!(table.is_subtype(ab, ab));
        // A field of the wrong type, or a missing field, is not.
        let a_float = table.record([(a, TyId::FLOAT)]);
        assert!(!table.is_subtype(a_int, a_float));
        assert!(!table.is_subtype(ab, a_float));
        let c_bool = table.record([(c, TyId::BOOL)]);
        assert!(!table.is_subtype(ab, c_bool));
        assert!(table.is_subtype(abc, c_bool));
        let b_only = table.record([(b, TyId::STRING)]);
        assert!(table.is_subtype(abc, b_only));
        assert!(!table.is_subtype(a_int, b_only));
        // Depth, nested.
        let outer_ab = table.record([(c, ab)]);
        let outer_a = table.record([(c, a_int)]);
        assert!(table.is_subtype(outer_ab, outer_a));
        assert!(!table.is_subtype(outer_a, outer_ab));
        let deeper = table.record([(b, outer_ab), (a, TyId::INT)]);
        let shallower = table.record([(b, outer_a)]);
        assert!(table.is_subtype(deeper, shallower));
        assert!(!table.is_subtype(shallower, deeper));
        // Everything record is a subtype of `{}`; nothing else is.
        for id in [a_int, ab, abc, outer_ab, deeper, TyId::EMPTY] {
            assert!(table.is_subtype(id, TyId::EMPTY));
        }
        assert!(!table.is_subtype(TyId::EMPTY, a_int));
        for id in [TyId::INT, TyId::FLOAT, TyId::BOOL, TyId::STRING] {
            assert!(!table.is_subtype(id, TyId::EMPTY));
            assert!(!table.is_subtype(TyId::EMPTY, id));
            assert!(table.is_subtype(id, id));
        }
        // The scalar types are subtypes only of themselves.
        assert!(!table.is_subtype(TyId::INT, TyId::FLOAT));
        assert!(!table.is_subtype(TyId::FLOAT, TyId::INT));
        assert!(!table.is_subtype(TyId::UNTYPED_INT, TyId::INT));
    }

    #[test]
    fn joins() {
        let (mut table, _, [a, b, c]) = setup();
        // Identical types join to themselves, records or not.
        assert_eq!(table.join(TyId::INT, TyId::INT), (TyId::INT, false));
        assert_eq!(table.join(TyId::EMPTY, TyId::EMPTY), (TyId::EMPTY, false));
        // Different non-record types, or a record and a non-record, join
        // to their union, widened.
        let int_float = table.union([TyId::INT, TyId::FLOAT]);
        assert_eq!(table.join(TyId::INT, TyId::FLOAT), (int_float, true));
        assert_eq!(table.join(TyId::INT, TyId::UNTYPED_INT), (TyId::INT, false));
        let empty_string = table.union([TyId::EMPTY, TyId::STRING]);
        assert_eq!(table.join(TyId::EMPTY, TyId::STRING), (empty_string, true));
        // Shared fields.
        let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        let a_int = table.record([(a, TyId::INT)]);
        assert_eq!(table.join(ab, a_int), (a_int, false));
        assert_eq!(table.join(a_int, ab), (a_int, false));
        let bc = table.record([(b, TyId::STRING), (c, TyId::BOOL)]);
        let b_string = table.record([(b, TyId::STRING)]);
        assert_eq!(table.join(ab, bc), (b_string, false));
        assert_eq!(table.join(bc, ab), (b_string, false));
        // No shared fields: the empty record.
        let c_bool = table.record([(c, TyId::BOOL)]);
        assert_eq!(table.join(a_int, c_bool), (TyId::EMPTY, false));
        assert_eq!(table.join(a_int, TyId::EMPTY), (TyId::EMPTY, false));
        // A shared field whose types do not agree is kept, with their
        // union, widened.
        let a_float = table.record([(a, TyId::FLOAT)]);
        let a_either_number = table.record([(a, int_float)]);
        assert_eq!(table.join(ab, a_float), (a_either_number, true));
        // Recursive.
        let outer_ab = table.record([(c, ab), (a, TyId::BOOL)]);
        let outer_bc = table.record([(c, bc)]);
        let joined = table.record([(c, b_string)]);
        assert_eq!(table.join(outer_ab, outer_bc), (joined, false));
        let outer_int = table.record([(c, TyId::INT)]);
        let ab_or_int = table.union([ab, TyId::INT]);
        let outer_union = table.record([(c, ab_or_int)]);
        assert_eq!(table.join(outer_ab, outer_int), (outer_union, true));
        // A field matched out of step: `b` of the second record lies
        // between fields of the first.
        let ac = table.record([(a, TyId::INT), (c, TyId::BOOL)]);
        let bc2 = table.record([(b, TyId::FLOAT), (c, TyId::BOOL)]);
        assert_eq!(table.join(ac, bc2), (c_bool, false));
        let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
        assert_eq!(table.join(abc, bc), (bc, false));
        assert_eq!(table.join(bc, abc), (bc, false));
        assert_eq!(table.join(ac, c_bool), (c_bool, false));
    }

    #[test]
    fn function_types_intern_with_their_names() {
        let (mut table, _, [a, b, _]) = setup();
        let named = table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure);
        let unnamed = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert_ne!(named, unnamed);
        assert_eq!(
            table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure),
            named
        );
        // Names, types, arity, return type and purity all distinguish.
        assert_ne!(
            table.fn_type([(Some(b), TyId::INT)], TyId::INT, Effect::Pure),
            named
        );
        assert_ne!(
            table.fn_type([(Some(a), TyId::FLOAT)], TyId::INT, Effect::Pure),
            named
        );
        assert_ne!(table.fn_type([], TyId::INT, Effect::Pure), named);
        assert_ne!(
            table.fn_type([(Some(a), TyId::INT)], TyId::BOOL, Effect::Pure),
            named
        );
        assert_ne!(
            table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Impure),
            named
        );
        let sig = table.fn_sig(named).unwrap();
        assert_eq!(&*sig.params, &[(Some(a), TyId::INT)]);
        assert_eq!(sig.ret, TyId::INT);
        assert_eq!(sig.effect, Effect::Pure);
        assert!(table.fn_sig(TyId::INT).is_none());
        assert!(table.fn_sig(TyId::EMPTY).is_none());
        assert!(!table.is_record(named));
    }

    #[test]
    fn display_function_types() {
        let mut names = Names::new();
        let (name, times, x) = (
            names.intern("name"),
            names.intern("times"),
            names.intern("x"),
        );
        let mut table = TypeTable::new();
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        let plain = table.fn_type(
            [(None, TyId::INT), (None, TyId::STRING)],
            TyId::BOOL,
            Effect::Pure,
        );
        assert_eq!(show(&table, plain), "fn(Int, String): Bool");
        let with_names = table.fn_type(
            [(Some(name), TyId::STRING), (Some(times), TyId::INT)],
            TyId::STRING,
            Effect::Impure,
        );
        assert_eq!(
            show(&table, with_names),
            "fn(name: String, times: Int) -> String"
        );
        let memo = table.fn_type([(Some(x), TyId::STRING)], TyId::STRING, Effect::Memo);
        assert_eq!(show(&table, memo), "fn(x: String) ->> String");
        let returns_memo = table.fn_type([(None, memo)], memo, Effect::Memo);
        assert_eq!(
            show(&table, returns_memo),
            "fn(fn(x: String) ->> String) ->> fn(x: String) ->> String"
        );
        let record = table.record([(x, TyId::INT)]);
        let returns_record = table.fn_type([], record, Effect::Pure);
        assert_eq!(show(&table, returns_record), "fn(): {x: Int}");
        let int_to_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        let higher = table.fn_type([(None, int_to_int)], TyId::INT, Effect::Pure);
        assert_eq!(show(&table, higher), "fn(fn(Int): Int): Int");
        let returns_fn = table.fn_type([], int_to_int, Effect::Impure);
        assert_eq!(show(&table, returns_fn), "fn() -> fn(Int): Int");
        let in_record = table.record([(x, int_to_int)]);
        assert_eq!(show(&table, in_record), "{x: fn(Int): Int}");
        let mixed = table.fn_type(
            [(Some(x), TyId::INT), (None, TyId::EMPTY)],
            TyId::EMPTY,
            Effect::Pure,
        );
        assert_eq!(show(&table, mixed), "fn(x: Int, {}): {}");
    }

    #[test]
    fn function_subtyping() {
        let (mut table, _, [a, b, c]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let f = |table: &mut TypeTable, params: &[TyId], ret, effect| {
            table.fn_type(params.iter().map(|&ty| (None, ty)), ret, effect)
        };
        // Arity must match.
        let unary = f(&mut table, &[TyId::INT], TyId::INT, Effect::Pure);
        let binary = f(&mut table, &[TyId::INT, TyId::INT], TyId::INT, Effect::Pure);
        assert!(!table.is_subtype(unary, binary));
        assert!(!table.is_subtype(binary, unary));
        // Parameters are contravariant: accepting `{a}` is more than
        // accepting `{a, b}`.
        let takes_a = f(&mut table, &[a_only], TyId::INT, Effect::Pure);
        let takes_ab = f(&mut table, &[ab], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(takes_a, takes_ab));
        assert!(!table.is_subtype(takes_ab, takes_a));
        // The return type is covariant.
        let gives_ab = f(&mut table, &[], ab, Effect::Pure);
        let gives_a = f(&mut table, &[], a_only, Effect::Pure);
        assert!(table.is_subtype(gives_ab, gives_a));
        assert!(!table.is_subtype(gives_a, gives_ab));
        // Pure may stand in for impure, not the reverse.
        let impure = f(&mut table, &[TyId::INT], TyId::INT, Effect::Impure);
        assert!(table.is_subtype(unary, impure));
        assert!(!table.is_subtype(impure, unary));
        // Names are ignored.
        let named = table.fn_type([(Some(c), TyId::INT)], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(named, unary));
        assert!(table.is_subtype(unary, named));
        // Different parameter types, or a function and anything else.
        let takes_float = f(&mut table, &[TyId::FLOAT], TyId::INT, Effect::Pure);
        assert!(!table.is_subtype(unary, takes_float));
        assert!(!table.is_subtype(unary, TyId::INT));
        assert!(!table.is_subtype(unary, TyId::EMPTY));
        assert!(!table.is_subtype(TyId::EMPTY, unary));
        // Nested: a function of functions, and a record of functions.
        let takes_takes_a = f(&mut table, &[takes_a], TyId::INT, Effect::Pure);
        let takes_takes_ab = f(&mut table, &[takes_ab], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(takes_takes_ab, takes_takes_a));
        assert!(!table.is_subtype(takes_takes_a, takes_takes_ab));
        let record_pure = table.record([(a, unary), (b, TyId::INT)]);
        let record_impure = table.record([(a, impure)]);
        assert!(table.is_subtype(record_pure, record_impure));
        assert!(!table.is_subtype(record_impure, record_pure));
    }

    #[test]
    fn the_effect_lattice() {
        let (mut table, _, _) = setup();
        let effects = [Effect::Pure, Effect::Memo, Effect::Impure];
        let of =
            |table: &mut TypeTable, effect| table.fn_type([(None, TyId::INT)], TyId::INT, effect);
        // All nine pairs: the offered effect must be at most the required.
        for offered in effects {
            for required in effects {
                let (sub, sup) = (of(&mut table, offered), of(&mut table, required));
                assert_eq!(
                    table.is_subtype(sub, sup),
                    offered <= required,
                    "{offered:?} against {required:?}"
                );
            }
        }
        assert!(Effect::Pure < Effect::Memo && Effect::Memo < Effect::Impure);
        // Joins take the greater effect, each way round.
        for a in effects {
            for b in effects {
                let (left, right) = (of(&mut table, a), of(&mut table, b));
                let joined = table.join(left, right);
                assert_eq!(joined, (of(&mut table, a.max(b)), false), "{a:?} and {b:?}");
            }
        }
        // A memoised function's bare parameter is present, which the
        // checker decides; with one, it is not a subtype of an impure
        // function type whose caller may pass a future, but is one of a
        // function type that promises a present argument.
        let either = table.deferrable(TyId::INT);
        let memo = table.fn_type([(None, TyId::INT)], either, Effect::Memo);
        let impure_either = table.fn_type([(None, either)], either, Effect::Impure);
        let impure_present = table.fn_type([(None, TyId::INT)], either, Effect::Impure);
        assert!(!table.is_subtype(memo, impure_either));
        assert!(table.is_subtype(memo, impure_present));
        // And a memoised function type never stands in for a pure one.
        let pure = table.fn_type([(None, TyId::INT)], either, Effect::Pure);
        assert!(!table.is_subtype(memo, pure));
        assert!(table.is_subtype(pure, memo));
    }

    #[test]
    fn function_joins() {
        let (mut table, _, [a, b, _]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let b_only = table.record([(b, TyId::INT)]);
        // Names kept where both sides agree, dropped otherwise; the return
        // types' join; the greater effect, so pure only if both are.
        let left = table.fn_type(
            [(Some(a), TyId::INT), (Some(a), TyId::BOOL)],
            ab,
            Effect::Pure,
        );
        let right = table.fn_type(
            [(Some(a), TyId::INT), (Some(b), TyId::BOOL)],
            a_only,
            Effect::Impure,
        );
        let joined = table.fn_type(
            [(Some(a), TyId::INT), (None, TyId::BOOL)],
            a_only,
            Effect::Impure,
        );
        assert_eq!(table.join(left, right), (joined, false));
        assert_eq!(table.join(right, left), (joined, false));
        let pure_right = table.fn_type(
            [(Some(a), TyId::INT), (None, TyId::BOOL)],
            b_only,
            Effect::Pure,
        );
        let pure_join = table.fn_type(
            [(Some(a), TyId::INT), (None, TyId::BOOL)],
            b_only,
            Effect::Pure,
        );
        assert_eq!(table.join(left, pure_right), (pure_join, false));
        let gives_a = table.fn_type(
            [(Some(a), TyId::INT), (None, TyId::BOOL)],
            a_only,
            Effect::Pure,
        );
        let gives_empty = table.fn_type(
            [(Some(a), TyId::INT), (None, TyId::BOOL)],
            TyId::EMPTY,
            Effect::Pure,
        );
        assert_eq!(table.join(pure_right, gives_a), (gives_empty, false));
        // A parameter is the narrower of the two types, which both
        // functions accept, if one is a subtype of the other.
        let takes_a = table.fn_type([(None, a_only)], TyId::INT, Effect::Pure);
        let takes_ab = table.fn_type([(None, ab)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(takes_a, takes_ab), (takes_ab, false));
        assert_eq!(table.join(takes_ab, takes_a), (takes_ab, false));
        let either = table.deferrable(TyId::INT);
        let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Pure);
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(takes_either, takes_int), (takes_int, false));
        assert_eq!(table.join(takes_int, takes_either), (takes_int, false));
        // Each parameter on its own.
        let mixed = table.fn_type([(None, either), (None, ab)], TyId::INT, Effect::Pure);
        let other = table.fn_type([(None, TyId::INT), (None, a_only)], TyId::INT, Effect::Pure);
        let narrow = table.fn_type([(None, TyId::INT), (None, ab)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(mixed, other), (narrow, false));
        // Unrelated parameter types join to the union of the two, widened.
        let unrelated = table.fn_type([(None, b_only)], TyId::INT, Effect::Pure);
        let either_taker = table.union([takes_a, unrelated]);
        assert_eq!(table.join(takes_a, unrelated), (either_taker, true));
        let takes_bool = table.fn_type([(None, TyId::BOOL)], TyId::INT, Effect::Pure);
        let int_or_bool_taker = table.union([takes_int, takes_bool]);
        assert_eq!(table.join(takes_int, takes_bool), (int_or_bool_taker, true));
        // So do different arities; return types that do not agree join to
        // their union, widened.
        let nullary = table.fn_type([], TyId::INT, Effect::Pure);
        let unary = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        let arities = table.union([nullary, unary]);
        assert_eq!(table.join(nullary, unary), (arities, true));
        let gives_float = table.fn_type([], TyId::FLOAT, Effect::Pure);
        let int_float = table.union([TyId::INT, TyId::FLOAT]);
        let gives_number = table.fn_type([], int_float, Effect::Pure);
        assert_eq!(table.join(nullary, gives_float), (gives_number, true));
        // A function and anything else join to their union; a function and
        // itself is itself.
        let with_int = table.union([nullary, TyId::INT]);
        assert_eq!(table.join(nullary, TyId::INT), (with_int, true));
        let with_empty = table.union([nullary, TyId::EMPTY]);
        assert_eq!(table.join(nullary, TyId::EMPTY), (with_empty, true));
        assert_eq!(table.join(nullary, nullary), (nullary, false));
        // Through records: a field whose function types do not join is
        // kept, with their union.
        let with_nullary = table.record([(a, nullary), (b, TyId::INT)]);
        let with_unary = table.record([(a, unary), (b, TyId::INT)]);
        let with_either = table.record([(a, arities), (b, TyId::INT)]);
        assert_eq!(table.join(with_nullary, with_unary), (with_either, true));
    }

    #[test]
    fn a_special_built_in_has_an_opaque_type() {
        let mut table = TypeTable::new();
        let mut names = Names::new();
        let field = names.intern("f");
        let len = table.builtin(Builtin::Len);
        assert_eq!(table.builtin(Builtin::Len), len);
        assert_eq!(table.display(len, &names).to_string(), "builtin len");
        assert_eq!(table.value_kind(len), Some(ValueKind::Fn));
        assert!(table.is_builtin(len));
        assert!(table.contains_fn(len));
        assert!(table.is_subtype(len, len));
        assert!(!table.is_subtype(len, TyId::INT));
        // Found wherever a type holds it, and nowhere else.
        let list = table.list(len);
        let record = table.record([(field, list)]);
        assert_eq!(table.special_builtin(record), Some(Builtin::Len));
        assert!(!table.is_builtin(record));
        assert_eq!(table.special_builtin(TyId::INT), None);
        let function = table.fn_type([(None, TyId::INT)], TyId::FLOAT, Effect::Pure);
        assert_eq!(table.special_builtin(function), None);
    }

    #[test]
    fn containing_a_function() {
        let (mut table, _, [a, b, c]) = setup();
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        assert!(table.contains_fn(function));
        for id in [
            TyId::INT,
            TyId::FLOAT,
            TyId::BOOL,
            TyId::STRING,
            TyId::EMPTY,
        ] {
            assert!(!table.contains_fn(id));
        }
        let flat = table.record([(a, TyId::INT)]);
        assert!(!table.contains_fn(flat));
        let direct = table.record([(a, TyId::INT), (b, function)]);
        assert!(table.contains_fn(direct));
        let deep = table.record([(c, direct)]);
        assert!(table.contains_fn(deep));
        let deeper = table.record([(a, deep), (b, flat)]);
        assert!(table.contains_fn(deeper));
        // A function taking or returning records is a function.
        let takes_flat = table.fn_type([(None, flat)], flat, Effect::Pure);
        assert!(table.contains_fn(takes_flat));
        // An id from a bigger table contains nothing.
        assert!(!TypeTable::new().contains_fn(deep));
    }

    #[test]
    fn value_kinds() {
        let (mut table, _, [a, _, _]) = setup();
        assert_eq!(table.value_kind(TyId::INT), Some(ValueKind::Int));
        assert_eq!(table.value_kind(TyId::FLOAT), Some(ValueKind::Float));
        assert_eq!(table.value_kind(TyId::BOOL), Some(ValueKind::Bool));
        assert_eq!(table.value_kind(TyId::STRING), Some(ValueKind::String));
        assert_eq!(table.value_kind(TyId::EMPTY), Some(ValueKind::Record));
        let record = table.record([(a, TyId::INT)]);
        assert_eq!(table.value_kind(record), Some(ValueKind::Record));
        assert_eq!(table.value_kind(TyId::UNTYPED_INT), None);
        assert_eq!(table.value_kind(TyId::ERROR), None);
        let union = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::BOOL])));
        assert_eq!(table.value_kind(union), Some(ValueKind::Mixed));
        let function = table.fn_type([(None, record)], TyId::INT, Effect::Impure);
        assert_eq!(table.value_kind(function), Some(ValueKind::Fn));
        // An id from a bigger table.
        assert_eq!(TypeTable::new().value_kind(record), None);
        // A future, and the either shape, may be futures.
        let future = table.future(TyId::INT);
        assert_eq!(table.value_kind(future), Some(ValueKind::Deferred));
        let either = table.deferrable(record);
        assert_eq!(table.value_kind(either), Some(ValueKind::Deferred));
    }

    // ----- Futures -----

    #[test]
    fn futures_and_the_either_shape_intern_normalised() {
        let (mut table, _, [a, _, _]) = setup();
        let future = table.future(TyId::INT);
        assert_eq!(table.future(TyId::INT), future);
        assert_eq!(table.kind(future), &TyKind::Future(TyId::INT));
        let either = table.deferrable(TyId::INT);
        assert_eq!(table.kind(either), &TyKind::Either(TyId::INT));
        // A future of a future, or of an either, is a future of what it
        // yields in the end.
        assert_eq!(table.future(future), future);
        assert_eq!(table.future(either), future);
        // But the fields of a record it yields are untouched.
        let lazy_fields = table.record([(a, future)]);
        let future_record = table.future(lazy_fields);
        assert_eq!(table.kind(future_record), &TyKind::Future(lazy_fields));
        assert_eq!(table.future(future_record), future_record);
        // Whatever it is given, stripped: a future or the either itself.
        assert_eq!(table.deferrable(future), either);
        assert_eq!(table.deferrable(either), either);
        // Interning is by structure.
        assert_eq!(table.intern(TyKind::Either(TyId::INT)), either);
        let record = table.record([(a, TyId::INT)]);
        let record_either = table.deferrable(record);
        assert_ne!(record_either, either);
        assert!(table.is_deferrable(either));
        assert!(table.is_deferrable(record_either));
        assert!(!table.is_deferrable(future));
        assert!(!table.is_deferrable(TyId::INT));
        let foreign = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::BOOL])));
        assert!(!table.is_deferrable(foreign));
        // An error has no either shape.
        assert_eq!(table.deferrable(TyId::ERROR), TyId::ERROR);
    }

    #[test]
    fn strip_present_and_laziness() {
        let (mut table, _, [a, b, c]) = setup();
        let future = table.future(TyId::INT);
        let nested = table.future(future);
        let either = table.deferrable(TyId::INT);
        for id in [TyId::INT, future, nested, either] {
            assert_eq!(table.strip(id), TyId::INT);
        }
        assert_eq!(table.laziness(TyId::INT), Laziness::Present);
        assert_eq!(table.laziness(future), Laziness::Future);
        assert_eq!(table.laziness(nested), Laziness::Future);
        assert_eq!(table.laziness(either), Laziness::Either);
        let foreign = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::BOOL])));
        assert_eq!(table.laziness(foreign), Laziness::Present);
        assert_eq!(table.strip(foreign), foreign);
        // Flat: a record's fields are left alone by `strip`, and presented
        // by `present`.
        let flat = table.record([(a, future), (b, either), (c, TyId::BOOL)]);
        let flat_present = table.record([(a, TyId::INT), (b, TyId::INT), (c, TyId::BOOL)]);
        assert_eq!(table.laziness(flat), Laziness::Present);
        assert_eq!(table.strip(flat), flat);
        assert_eq!(table.present(flat), flat_present);
        // Nested, and through the top.
        let inner = table.record([(a, nested)]);
        let inner_present = table.record([(a, TyId::INT)]);
        let outer = table.record([(b, inner), (c, either)]);
        let outer_present = table.record([(b, inner_present), (c, TyId::INT)]);
        assert_eq!(table.present(outer), outer_present);
        let outer_future = table.future(outer);
        assert_eq!(table.strip(outer_future), outer);
        assert_eq!(table.present(outer_future), outer_present);
        // Through an either of a record.
        let inner_either = table.deferrable(inner);
        let holder = table.record([(a, inner_either)]);
        let holder_present = table.record([(a, inner_present)]);
        assert_eq!(table.present(holder), holder_present);
        // Not into functions.
        let function = table.fn_type([(None, either)], future, Effect::Impure);
        let with_fn = table.record([(a, function), (b, future)]);
        let with_fn_present = table.record([(a, function), (b, TyId::INT)]);
        assert_eq!(table.present(with_fn), with_fn_present);
        assert_eq!(table.present(function), function);
        // A present type presents as itself, interning nothing.
        let before = table.len();
        assert_eq!(table.present(outer_present), outer_present);
        assert_eq!(table.present(TyId::STRING), TyId::STRING);
        assert_eq!(table.len(), before);
    }

    #[test]
    fn what_needs_an_await() {
        let (mut table, _, [a, b, _]) = setup();
        let future = table.future(TyId::INT);
        let nested = table.future(future);
        let either = table.deferrable(TyId::INT);
        // Fits as it is.
        assert_eq!(table.needs_await(TyId::INT, TyId::INT), None);
        assert_eq!(table.needs_await(future, either), None);
        assert_eq!(table.needs_await(nested, either), None);
        assert_eq!(table.needs_await(TyId::INT, either), None);
        // At the top.
        assert_eq!(table.needs_await(future, TyId::INT), Some(AwaitKind::Top));
        assert_eq!(table.needs_await(either, TyId::INT), Some(AwaitKind::Top));
        assert_eq!(table.needs_await(nested, TyId::INT), Some(AwaitKind::Top));
        // Deeply: a record with a future field against a present record.
        let lazy_record = table.record([(a, future), (b, TyId::STRING)]);
        let present_record = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        assert_eq!(
            table.needs_await(lazy_record, present_record),
            Some(AwaitKind::Deep)
        );
        assert_eq!(table.needs_await(lazy_record, lazy_record), None);
        let future_record = table.future(lazy_record);
        assert_eq!(
            table.needs_await(future_record, lazy_record),
            Some(AwaitKind::Top)
        );
        assert_eq!(
            table.needs_await(future_record, present_record),
            Some(AwaitKind::Deep)
        );
        // Against a wider present record, deeply too.
        let a_only = table.record([(a, TyId::INT)]);
        assert_eq!(
            table.needs_await(lazy_record, a_only),
            Some(AwaitKind::Deep)
        );
        // Never: a present value against a future, or a mismatch whatever
        // is awaited.
        assert_eq!(table.needs_await(TyId::INT, future), None);
        assert!(!table.is_subtype(TyId::INT, future));
        assert_eq!(table.needs_await(either, future), None);
        assert_eq!(table.needs_await(future, TyId::FLOAT), None);
        assert_eq!(table.needs_await(lazy_record, TyId::INT), None);
    }

    #[test]
    fn subtyping_with_futures() {
        let (mut table, _, [a, b, _]) = setup();
        let future = table.future(TyId::INT);
        let nested = table.future(future);
        let either = table.deferrable(TyId::INT);
        let float_future = table.future(TyId::FLOAT);
        // `Future<S> <: Future<T>` iff `S <: T`.
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let future_ab = table.future(ab);
        let future_a = table.future(a_only);
        assert!(table.is_subtype(future_ab, future_a));
        assert!(!table.is_subtype(future_a, future_ab));
        assert!(!table.is_subtype(future, float_future));
        // A future of a future is interned as the future itself.
        assert_eq!(nested, future);
        // A present value satisfies the either shape, and not a future.
        let either_a = table.deferrable(a_only);
        assert!(table.is_subtype(TyId::INT, either));
        assert!(!table.is_subtype(TyId::INT, future));
        assert!(table.is_subtype(ab, either_a));
        // A future of any depth satisfies the either shape of what it
        // yields once awaited fully.
        assert!(table.is_subtype(future, either));
        assert!(table.is_subtype(nested, either));
        assert!(!table.is_subtype(float_future, either));
        assert!(table.is_subtype(future_ab, either_a));
        // One either shape of another.
        let either_ab = table.deferrable(ab);
        assert!(table.is_subtype(either_ab, either_a));
        assert!(!table.is_subtype(either_a, either_ab));
        // Nothing lazy is a subtype of a present type, and the either
        // shape is not a future.
        assert!(!table.is_subtype(future, TyId::INT));
        assert!(!table.is_subtype(either, TyId::INT));
        assert!(!table.is_subtype(either, future));
        assert!(!table.is_subtype(future_ab, ab));
        // Through record fields.
        let with_future = table.record([(a, future)]);
        let with_either = table.record([(a, either)]);
        let with_int = table.record([(a, TyId::INT)]);
        assert!(table.is_subtype(with_future, with_either));
        assert!(table.is_subtype(with_int, with_either));
        assert!(!table.is_subtype(with_either, with_int));
        assert!(!table.is_subtype(with_future, with_int));
        assert!(!table.is_subtype(with_int, with_future));
        // Through function parameters, contravariant, and results,
        // covariant.
        let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Impure);
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Impure);
        assert!(table.is_subtype(takes_either, takes_int));
        assert!(!table.is_subtype(takes_int, takes_either));
        let gives_future = table.fn_type([], future, Effect::Impure);
        let gives_either = table.fn_type([], either, Effect::Impure);
        assert!(table.is_subtype(gives_future, gives_either));
        assert!(!table.is_subtype(gives_either, gives_future));
        // The two callback cases: a pure function whose parameter is
        // present fits `fn(Present<Int>) -> Int`, whose parameter is
        // present and result either, and not `fn(Int) -> Int`, whose
        // parameter is the either too; one whose parameter is the either
        // fits both.
        let double = table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure);
        let present_callback = table.fn_type([(None, TyId::INT)], either, Effect::Impure);
        let either_callback = table.fn_type([(None, either)], either, Effect::Impure);
        assert!(table.is_subtype(double, present_callback));
        assert!(!table.is_subtype(double, either_callback));
        let passes = table.fn_type([(Some(a), either)], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(passes, present_callback));
        assert!(table.is_subtype(passes, either_callback));
    }

    #[test]
    fn joins_with_futures() {
        let (mut table, _, [a, b, _]) = setup();
        let future = table.future(TyId::INT);
        let either = table.deferrable(TyId::INT);
        // Two futures: the future of the join.
        assert_eq!(table.join(future, future), (future, false));
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let future_ab = table.future(ab);
        let future_a = table.future(a_only);
        assert_eq!(table.join(future_ab, future_a), (future_a, false));
        // A present value and a future, or an either and anything: the
        // either of the join, stripped.
        assert_eq!(table.join(TyId::INT, future), (either, false));
        assert_eq!(table.join(future, TyId::INT), (either, false));
        assert_eq!(table.join(either, future), (either, false));
        assert_eq!(table.join(either, TyId::INT), (either, false));
        let nested = table.future(future);
        assert_eq!(table.join(nested, TyId::INT), (either, false));
        let either_a = table.deferrable(a_only);
        assert_eq!(table.join(ab, future_a), (either_a, false));
        // Each widens iff the inner join does: the future, or the either, of
        // the union.
        let float_future = table.future(TyId::FLOAT);
        let int_float = table.union([TyId::INT, TyId::FLOAT]);
        let future_number = table.future(int_float);
        assert_eq!(table.join(future, float_future), (future_number, true));
        let either_number = table.deferrable(int_float);
        assert_eq!(table.join(TyId::FLOAT, future), (either_number, true));
        // Records join field by field, with these rules per field.
        let with_future = table.record([(a, future), (b, TyId::INT)]);
        let with_int = table.record([(a, TyId::INT)]);
        let with_either = table.record([(a, either)]);
        assert_eq!(table.join(with_future, with_int), (with_either, false));
    }

    #[test]
    fn containing_a_future() {
        let (mut table, _, [a, b, _]) = setup();
        let future = table.future(TyId::INT);
        let either = table.deferrable(TyId::INT);
        assert!(table.contains_future(future));
        assert!(!table.contains_future(either));
        assert!(!table.contains_future(TyId::INT));
        let with_future = table.record([(a, future)]);
        let with_either = table.record([(a, either)]);
        assert!(table.contains_future(with_future));
        assert!(!table.contains_future(with_either));
        let deep = table.record([(b, with_future)]);
        assert!(table.contains_future(deep));
        // An either counts through its present member.
        let either_deep = table.deferrable(deep);
        let holder = table.record([(a, either_deep)]);
        assert!(table.contains_future(holder));
        // Functions are not looked into.
        let function = table.fn_type([(None, future)], future, Effect::Pure);
        assert!(!table.contains_future(function));
        // But `contains_fn` looks through futures and unions.
        let future_fn = table.future(function);
        assert!(table.contains_fn(future_fn));
        let fn_record = table.record([(a, future_fn)]);
        assert!(table.contains_fn(fn_record));
        let either_fn = table.deferrable(function);
        assert!(table.contains_fn(either_fn));
    }

    #[test]
    fn display_futures() {
        let mut names = Names::new();
        let (build, step, x) = (
            names.intern("build"),
            names.intern("step"),
            names.intern("x"),
        );
        let mut table = TypeTable::new();
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        let future = table.future(TyId::INT);
        assert_eq!(show(&table, future), "Future<Int>");
        let either = table.deferrable(TyId::INT);
        assert_eq!(show(&table, either), "Int | Future<Int>");
        let future_empty = table.future(TyId::EMPTY);
        let record = table.record([(build, future_empty)]);
        assert_eq!(show(&table, record), "{build: Future<{}>}");
        let takes_step = table.fn_type([(Some(step), future_empty)], TyId::EMPTY, Effect::Pure);
        assert_eq!(show(&table, takes_step), "fn(step: Future<{}>): {}");
        // A future of a future is interned, and so shown, as a future.
        let nested = table.future(future);
        assert_eq!(show(&table, nested), "Future<Int>");
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        let future_fn = table.future(function);
        assert_eq!(show(&table, future_fn), "Future<fn(): Int>");
        // The either of a function type parenthesises the function type,
        // whose return type would otherwise take the union; not inside
        // `Future<...>`.
        let either_fn = table.deferrable(function);
        assert_eq!(show(&table, either_fn), "(fn(): Int) | Future<fn(): Int>");
        let impure = table.fn_type([], either, Effect::Impure);
        let either_impure = table.deferrable(impure);
        assert_eq!(
            show(&table, either_impure),
            "(fn() -> Int | Future<Int>) | Future<fn() -> Int | Future<Int>>"
        );
        // A function returning an either shows it unparenthesised.
        let callback = table.fn_type([(None, either)], either, Effect::Impure);
        assert_eq!(
            show(&table, callback),
            "fn(Int | Future<Int>) -> Int | Future<Int>"
        );
        let gives_either_fn = table.fn_type([], either_fn, Effect::Impure);
        assert_eq!(
            show(&table, gives_either_fn),
            "fn() -> (fn(): Int) | Future<fn(): Int>"
        );
        let with_either = table.record([(x, either)]);
        let record_either = table.deferrable(with_either);
        assert_eq!(
            show(&table, record_either),
            "{x: Int | Future<Int>} | Future<{x: Int | Future<Int>}>"
        );
    }

    // ----- Provisional types -----

    const F: FnId = FnId::new(0);
    const G: FnId = FnId::new(1);

    #[test]
    fn provisional_types_intern_by_function_parameter_and_type() {
        let (mut table, _, [a, _, _]) = setup();
        let x = table.provisional(F, 0, TyId::INT);
        assert_eq!(table.provisional(F, 0, TyId::INT), x);
        assert_eq!(
            table.kind(x),
            &TyKind::Provisional {
                function: F,
                param: 0,
                inner: TyId::INT
            }
        );
        // Another parameter, function or type is another type.
        assert_ne!(table.provisional(F, 1, TyId::INT), x);
        assert_ne!(table.provisional(G, 0, TyId::INT), x);
        assert_ne!(table.provisional(F, 0, TyId::FLOAT), x);
        // No values have one: it never survives a successful check.
        assert_eq!(table.value_kind(x), None);
        let record = table.record([(a, TyId::INT)]);
        let r = table.provisional(F, 1, record);
        assert_eq!(table.value_kind(r), None);
    }

    #[test]
    fn strip_present_and_laziness_of_provisional_types() {
        let (mut table, _, [a, b, _]) = setup();
        let int = table.provisional(F, 0, TyId::INT);
        assert_eq!(table.laziness(int), Laziness::Provisional);
        assert_eq!(table.strip(int), TyId::INT);
        assert_eq!(table.present(int), TyId::INT);
        // In a record's fields, stripped by `present` only.
        let holder = table.record([(a, int), (b, TyId::STRING)]);
        assert_eq!(table.laziness(holder), Laziness::Present);
        assert_eq!(table.strip(holder), holder);
        let holder_present = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        assert_eq!(table.present(holder), holder_present);
        // Of a record type, stripped to it.
        let record = table.record([(a, TyId::INT)]);
        let of_record = table.provisional(F, 1, record);
        assert_eq!(table.strip(of_record), record);
        // A future or an either of one is of its present type.
        assert_eq!(table.future(int), table.future(TyId::INT));
        assert_eq!(table.deferrable(int), table.deferrable(TyId::INT));
        // Looked through as a union is by `contains_fn`, `contains_future`
        // and `needs_await`.
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        let with_fn = table.record([(a, function)]);
        let of_fn_record = table.provisional(F, 2, with_fn);
        assert!(table.contains_fn(of_fn_record));
        assert!(!table.contains_fn(int));
        let future = table.future(TyId::INT);
        let with_future = table.record([(a, future)]);
        let of_future_record = table.provisional(F, 3, with_future);
        assert!(table.contains_future(of_future_record));
        assert!(!table.contains_future(int));
        assert_eq!(table.needs_await(int, TyId::INT), Some(AwaitKind::Top));
        let either = table.deferrable(TyId::INT);
        assert_eq!(table.needs_await(int, either), None);
        assert!(table.is_subtype(int, either));
        let present_record = table.record([(a, TyId::INT)]);
        assert_eq!(
            table.needs_await(of_future_record, present_record),
            Some(AwaitKind::Deep)
        );
    }

    #[test]
    fn provisional_types_are_their_either_shape_as_subtypes_and_present_as_supertypes() {
        let (mut table, _, [a, _, _]) = setup();
        let x = table.provisional(F, 0, TyId::INT);
        let y = table.provisional(F, 1, TyId::INT);
        let either = table.deferrable(TyId::INT);
        let future = table.future(TyId::INT);
        // As a subtype, what the either shape is a subtype of.
        assert!(table.is_subtype(x, x));
        assert!(table.is_subtype(x, either));
        assert!(!table.is_subtype(x, TyId::INT));
        assert!(!table.is_subtype(x, future));
        assert!(!table.is_subtype(x, TyId::FLOAT));
        // As a supertype, its present type, unless it is the same one.
        assert!(table.is_subtype(TyId::INT, x));
        assert!(!table.is_subtype(either, x));
        assert!(!table.is_subtype(future, x));
        assert!(!table.is_subtype(y, x));
        // Through records and function types: a parameter is contravariant.
        let holder = table.record([(a, x)]);
        let holds_either = table.record([(a, either)]);
        let holds_int = table.record([(a, TyId::INT)]);
        assert!(table.is_subtype(holder, holds_either));
        assert!(!table.is_subtype(holder, holds_int));
        let takes_x = table.fn_type([(None, x)], TyId::INT, Effect::Pure);
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(takes_x, takes_int));
        assert!(!table.is_subtype(takes_x, takes_either));
        let gives_x = table.fn_type([], x, Effect::Pure);
        let gives_either = table.fn_type([], either, Effect::Pure);
        let gives_int = table.fn_type([], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(gives_x, gives_either));
        assert!(!table.is_subtype(gives_x, gives_int));
    }

    #[test]
    fn provisional_types_join_as_their_either_shape() {
        let (mut table, _, [a, _, _]) = setup();
        let x = table.provisional(F, 0, TyId::INT);
        let y = table.provisional(F, 1, TyId::INT);
        let other = table.provisional(G, 0, TyId::INT);
        let either = table.deferrable(TyId::INT);
        let future = table.future(TyId::INT);
        // With itself, itself; with anything else, the either shape of the
        // join.
        assert_eq!(table.join(x, x), (x, false));
        assert_eq!(table.join(x, y), (either, false));
        assert_eq!(table.join(x, other), (either, false));
        assert_eq!(table.join(x, TyId::INT), (either, false));
        assert_eq!(table.join(TyId::INT, x), (either, false));
        assert_eq!(table.join(x, future), (either, false));
        assert_eq!(table.join(x, either), (either, false));
        let int_float = table.union([TyId::INT, TyId::FLOAT]);
        let either_number = table.deferrable(int_float);
        assert_eq!(table.join(x, TyId::FLOAT), (either_number, true));
        // Through records, field by field.
        let holder = table.record([(a, x)]);
        let holds_int = table.record([(a, TyId::INT)]);
        let holds_either = table.record([(a, either)]);
        assert_eq!(table.join(holder, holder), (holder, false));
        assert_eq!(table.join(holder, holds_int), (holds_either, false));
    }

    #[test]
    fn settling_replaces_a_functions_provisional_types_everywhere() {
        let (mut table, _, [a, b, _]) = setup();
        let x = table.provisional(F, 0, TyId::INT);
        let y = table.provisional(F, 1, TyId::STRING);
        let other = table.provisional(G, 0, TyId::INT);
        let int_either = table.deferrable(TyId::INT);
        let string_either = table.deferrable(TyId::STRING);
        // At the top, by the decision for its parameter.
        assert_eq!(table.settle(x, F, &[true, false]), TyId::INT);
        assert_eq!(table.settle(x, F, &[false, false]), int_either);
        assert_eq!(table.settle(y, F, &[true, false]), string_either);
        // Another function's are left as they are, and so is one with no
        // decision.
        assert_eq!(table.settle(other, F, &[true, true]), other);
        assert_eq!(table.settle(y, F, &[true]), y);
        // A type with none is itself, interning nothing.
        let plain = table.record([(a, TyId::INT)]);
        let before = table.len();
        assert_eq!(table.settle(plain, F, &[true]), plain);
        assert_eq!(table.len(), before);
        // Through records, futures, the either shape and function types,
        // parameters and results.
        let holder = table.record([(a, x), (b, other)]);
        let settled = table.record([(a, TyId::INT), (b, other)]);
        assert_eq!(table.settle(holder, F, &[true, true]), settled);
        let future = table.future(holder);
        let settled_future = table.future(settled);
        assert_eq!(table.settle(future, F, &[true, true]), settled_future);
        let either = table.deferrable(holder);
        let settled_either = table.deferrable(settled);
        assert_eq!(table.settle(either, F, &[true, true]), settled_either);
        let function = table.fn_type([(Some(a), x), (Some(b), y)], holder, Effect::Pure);
        let settled_function = {
            let lazy_holder = table.record([(a, int_either), (b, other)]);
            table.fn_type(
                [(Some(a), int_either), (Some(b), TyId::STRING)],
                lazy_holder,
                Effect::Pure,
            )
        };
        assert_eq!(table.settle(function, F, &[false, true]), settled_function);
        // Only `other` is left, until its own function settles it.
        assert!(table.contains_provisional(settled_function, None));
        assert!(!table.contains_provisional(settled_function, Some(F)));
        let fully = table.settle(settled_function, G, &[true]);
        assert!(!table.contains_provisional(fully, None));
    }

    #[test]
    fn finding_provisional_types() {
        let (mut table, _, [a, b, c]) = setup();
        let first = table.provisional(F, 0, TyId::INT);
        let second = table.provisional(F, 1, TyId::INT);
        let other = table.provisional(G, 0, TyId::INT);
        let either = table.deferrable(TyId::INT);
        let future = table.future(TyId::INT);
        // Anywhere, through every kind of type.
        for id in [first, other] {
            assert!(table.contains_provisional(id, None));
        }
        assert!(table.contains_provisional(first, Some(F)));
        assert!(!table.contains_provisional(other, Some(F)));
        assert!(!table.contains_provisional(TyId::INT, None));
        let holder = table.record([(a, first)]);
        let in_future = table.future(holder);
        let in_either = table.deferrable(holder);
        let in_param = table.fn_type([(None, first)], TyId::INT, Effect::Pure);
        let in_result = table.fn_type([], holder, Effect::Pure);
        for id in [holder, in_future, in_either, in_param, in_result] {
            assert!(table.contains_provisional(id, Some(F)), "{id:?}");
            assert!(!table.contains_provisional(id, Some(G)), "{id:?}");
        }
        // Their parameters, anywhere.
        let both = table.record([(a, first), (b, in_param), (c, other)]);
        let mut found = Vec::new();
        table.provisional_params(both, F, &mut found);
        found.sort_unstable();
        assert_eq!(found, vec![0, 0]);
        // Those at positions where a type expected is present: not where it
        // is the either shape or a future, nor in a function's parameters,
        // nor in a field it lacks.
        let at_present = |table: &TypeTable, found: TyId, expected: TyId| {
            let mut out = Vec::new();
            table.provisionals_at_present(found, expected, F, &mut out);
            out.sort_unstable();
            out
        };
        assert_eq!(at_present(&table, first, TyId::INT), vec![0]);
        assert_eq!(at_present(&table, first, second), vec![0]);
        assert_eq!(at_present(&table, first, either), Vec::<u32>::new());
        assert_eq!(at_present(&table, first, future), Vec::<u32>::new());
        assert_eq!(at_present(&table, other, TyId::INT), Vec::<u32>::new());
        let pair = table.record([(a, first), (b, second)]);
        let wants_a = table.record([(a, TyId::INT), (b, either)]);
        assert_eq!(at_present(&table, pair, wants_a), vec![0]);
        let wants_both = table.record([(a, TyId::INT), (b, TyId::INT)]);
        assert_eq!(at_present(&table, pair, wants_both), vec![0, 1]);
        let wants_b_only = table.record([(b, TyId::INT)]);
        assert_eq!(at_present(&table, pair, wants_b_only), vec![1]);
        // Through the either shape and futures of records.
        let either_wants_both = table.deferrable(wants_both);
        assert_eq!(at_present(&table, pair, either_wants_both), vec![0, 1]);
        let future_pair = table.future(pair);
        assert_eq!(at_present(&table, future_pair, wants_both), vec![0, 1]);
        let future_wants_both = table.future(wants_both);
        assert_eq!(
            at_present(&table, pair, future_wants_both),
            Vec::<u32>::new()
        );
        // A function's result, not its parameters.
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert_eq!(at_present(&table, in_param, takes_int), Vec::<u32>::new());
        let gives_first = table.fn_type([], first, Effect::Pure);
        let gives_int = table.fn_type([], TyId::INT, Effect::Pure);
        assert_eq!(at_present(&table, gives_first, gives_int), vec![0]);
    }

    #[test]
    fn display_provisional_types() {
        let mut names = Names::new();
        let x = names.intern("x");
        let mut table = TypeTable::new();
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        let int = table.provisional(F, 0, TyId::INT);
        assert_eq!(show(&table, int), "Int?");
        let record = table.record([(x, TyId::INT)]);
        let r = table.provisional(F, 1, record);
        assert_eq!(show(&table, r), "{x: Int}?");
        let holder = table.record([(x, int)]);
        assert_eq!(show(&table, holder), "{x: Int?}");
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        let f = table.provisional(F, 2, function);
        assert_eq!(show(&table, f), "(fn(): Int)?");
        let takes = table.fn_type([(Some(x), int)], int, Effect::Pure);
        assert_eq!(show(&table, takes), "fn(x: Int?): Int?");
    }

    // ----- Lists -----

    #[test]
    fn list_types_intern_by_element_type() {
        let (mut table, _, [a, ..]) = setup();
        let ints = table.list(TyId::INT);
        assert_eq!(table.list(TyId::INT), ints);
        assert_ne!(table.list(TyId::FLOAT), ints);
        assert_eq!(table.list_element(ints), Some(TyId::INT));
        assert_eq!(table.list_element(TyId::INT), None);
        let nested = table.list(ints);
        assert_eq!(table.list_element(nested), Some(ints));
        let record = table.record([(a, TyId::INT)]);
        let records = table.list(record);
        assert_eq!(table.record([(a, TyId::INT)]), record);
        assert_eq!(table.list(record), records);
        assert_eq!(table.value_kind(ints), Some(ValueKind::List));
        let either = table.deferrable(ints);
        assert_eq!(table.value_kind(either), Some(ValueKind::Deferred));
        assert_eq!(table.laziness(ints), Laziness::Present);
        assert!(!table.contains_fn(ints));
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        let functions = table.list(function);
        assert!(table.contains_fn(functions));
        let in_record = table.record([(a, functions)]);
        assert!(table.contains_fn(in_record));
    }

    #[test]
    fn display_list_types() {
        let (mut table, names, [a, ..]) = setup();
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        let ints = table.list(TyId::INT);
        assert_eq!(show(&table, ints), "List<Int>");
        let nested = table.list(ints);
        assert_eq!(show(&table, nested), "List<List<Int>>");
        let either = table.deferrable(TyId::INT);
        let eithers = table.list(either);
        assert_eq!(show(&table, eithers), "List<Int | Future<Int>>");
        let outer = table.deferrable(eithers);
        assert_eq!(
            show(&table, outer),
            "List<Int | Future<Int>> | Future<List<Int | Future<Int>>>"
        );
        let record = table.record([(a, ints)]);
        let records = table.list(record);
        assert_eq!(show(&table, records), "List<{a: List<Int>}>");
        let function = table.fn_type([], TyId::INT, Effect::Impure);
        let functions = table.list(function);
        assert_eq!(show(&table, functions), "List<fn() -> Int>");
        let provisional = table.provisional(F, 0, ints);
        assert_eq!(show(&table, provisional), "List<Int>?");
    }

    #[test]
    fn list_subtyping_is_covariant() {
        let (mut table, _, [a, b, _]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let just_a = table.record([(a, TyId::INT)]);
        let abs = table.list(ab);
        let just_as = table.list(just_a);
        assert!(table.is_subtype(abs, just_as));
        assert!(!table.is_subtype(just_as, abs));
        let ints = table.list(TyId::INT);
        let floats = table.list(TyId::FLOAT);
        assert!(!table.is_subtype(ints, floats));
        assert!(!table.is_subtype(ints, TyId::EMPTY));
        assert!(!table.is_subtype(TyId::EMPTY, ints));
        // With futures inside: a present element satisfies the either
        // shape, and nothing but a future a future.
        let either = table.deferrable(TyId::INT);
        let future = table.future(TyId::INT);
        let eithers = table.list(either);
        let futures = table.list(future);
        assert!(table.is_subtype(ints, eithers));
        assert!(table.is_subtype(futures, eithers));
        assert!(!table.is_subtype(eithers, ints));
        assert!(!table.is_subtype(ints, futures));
        // A list, or its future, satisfies the either shape of a list type.
        let outer = table.deferrable(eithers);
        let future_ints = table.future(ints);
        assert!(table.is_subtype(ints, outer));
        assert!(table.is_subtype(future_ints, outer));
        assert!(!table.is_subtype(outer, eithers));
        let nested = table.list(ints);
        let nested_eithers = table.list(eithers);
        assert!(table.is_subtype(nested, nested_eithers));
    }

    #[test]
    fn list_joins() {
        let (mut table, _, [a, b, _]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_string = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        let just_a = table.record([(a, TyId::INT)]);
        let abs = table.list(ab);
        let a_strings = table.list(a_string);
        let just_as = table.list(just_a);
        assert_eq!(table.join(abs, just_as), (just_as, false));
        let int_string = table.union([TyId::INT, TyId::STRING]);
        let a_either = table.record([(a, TyId::INT), (b, int_string)]);
        let a_eithers = table.list(a_either);
        assert_eq!(table.join(abs, a_strings), (a_eithers, true));
        let ints = table.list(TyId::INT);
        let floats = table.list(TyId::FLOAT);
        let int_float = table.union([TyId::INT, TyId::FLOAT]);
        let numbers = table.list(int_float);
        assert_eq!(table.join(ints, floats), (numbers, true));
        let ints_or_int = table.union([ints, TyId::INT]);
        assert_eq!(table.join(ints, TyId::INT), (ints_or_int, true));
        let ints_or_empty = table.union([ints, TyId::EMPTY]);
        assert_eq!(table.join(ints, TyId::EMPTY), (ints_or_empty, true));
        let future = table.future(TyId::INT);
        let futures = table.list(future);
        let either = table.deferrable(TyId::INT);
        let eithers = table.list(either);
        assert_eq!(table.join(ints, futures), (eithers, false));
        // A list and a future of one join to the either shape.
        let future_ints = table.future(ints);
        let outer = table.deferrable(ints);
        assert_eq!(table.join(ints, future_ints), (outer, false));
    }

    #[test]
    fn strip_present_and_futures_through_lists() {
        let (mut table, _, [a, ..]) = setup();
        let either = table.deferrable(TyId::INT);
        let future = table.future(TyId::INT);
        let eithers = table.list(either);
        let futures = table.list(future);
        let ints = table.list(TyId::INT);
        // A list is present: stripping leaves its element type, and
        // presenting makes it present too.
        let outer = table.deferrable(eithers);
        assert_eq!(table.strip(outer), eithers);
        assert_eq!(table.present(outer), ints);
        assert_eq!(table.present(futures), ints);
        assert_eq!(table.present(ints), ints);
        let nested = table.list(eithers);
        let nested_ints = table.list(ints);
        assert_eq!(table.present(nested), nested_ints);
        let record = table.record([(a, eithers)]);
        let present_record = table.record([(a, ints)]);
        let records = table.list(record);
        let present_records = table.list(present_record);
        assert_eq!(table.present(records), present_records);
        // Awaits: at the top for a future of a list with present elements,
        // deeply for one with lazy elements.
        let future_ints = table.future(ints);
        assert_eq!(table.needs_await(future_ints, ints), Some(AwaitKind::Top));
        assert_eq!(table.needs_await(eithers, ints), Some(AwaitKind::Deep));
        assert_eq!(table.needs_await(outer, ints), Some(AwaitKind::Deep));
        assert_eq!(table.needs_await(ints, eithers), None);
        assert_eq!(table.needs_await(futures, TyId::INT), None);
        // A `Future` element is a future inside; the either shape is not.
        assert!(table.contains_future(futures));
        assert!(!table.contains_future(eithers));
        let nested_futures = table.list(futures);
        assert!(table.contains_future(nested_futures));
        let holder = table.record([(a, futures)]);
        assert!(table.contains_future(holder));
        assert!(!table.contains_future(ints));
    }

    #[test]
    fn provisional_types_through_lists() {
        let (mut table, _, _) = setup();
        let first = table.provisional(F, 0, TyId::INT);
        let firsts = table.list(first);
        assert!(table.contains_provisional(firsts, Some(F)));
        assert!(!table.contains_provisional(firsts, Some(G)));
        let mut found = Vec::new();
        table.provisional_params(firsts, F, &mut found);
        assert_eq!(found, vec![0]);
        // Settled, element and all.
        let ints = table.list(TyId::INT);
        let either = table.deferrable(TyId::INT);
        let eithers = table.list(either);
        assert_eq!(table.settle(firsts, F, &[true]), ints);
        assert_eq!(table.settle(firsts, F, &[false]), eithers);
        // Stripped and presented as the either shape it may become.
        assert_eq!(table.present(firsts), ints);
        // Those where a present element is expected.
        let at_present = |table: &TypeTable, found: TyId, expected: TyId| {
            let mut out = Vec::new();
            table.provisionals_at_present(found, expected, F, &mut out);
            out
        };
        assert_eq!(at_present(&table, firsts, ints), vec![0]);
        assert_eq!(at_present(&table, firsts, eithers), Vec::<u32>::new());
        let outer = table.deferrable(ints);
        assert_eq!(at_present(&table, firsts, outer), vec![0]);
        let future_ints = table.future(ints);
        assert_eq!(at_present(&table, firsts, future_ints), Vec::<u32>::new());
        // A provisional list is present as a supertype, its either shape as
        // a subtype.
        let list_param = table.provisional(F, 1, ints);
        assert!(table.is_subtype(ints, list_param));
        assert!(!table.is_subtype(list_param, ints));
        let outer_eithers = table.deferrable(ints);
        assert!(table.is_subtype(list_param, outer_eithers));
    }

    // ----- Unions -----

    #[test]
    fn unions_intern_normalised() {
        let (mut table, _, [a, b, _]) = setup();
        let int_string = table.union([TyId::INT, TyId::STRING]);
        assert_eq!(
            table.kind(int_string),
            &TyKind::Union(Box::new([TyId::INT, TyId::STRING]))
        );
        // Any order, duplicates and nested unions intern to the same union.
        assert_eq!(table.union([TyId::STRING, TyId::INT]), int_string);
        assert_eq!(
            table.union([TyId::STRING, TyId::INT, TyId::STRING]),
            int_string
        );
        let three = table.union([int_string, TyId::BOOL]);
        assert_eq!(table.union([TyId::BOOL, TyId::STRING, TyId::INT]), three);
        assert_eq!(
            table.union_members(three),
            Some(&[TyId::INT, TyId::BOOL, TyId::STRING][..])
        );
        // One member is that member; a subtype of another member is dropped.
        assert_eq!(table.union([TyId::INT, TyId::INT]), TyId::INT);
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        assert_eq!(table.union([ab, a_only]), a_only);
        let with_int = table.union([ab, TyId::INT, a_only]);
        assert_eq!(
            table.union_members(with_int),
            Some(&[TyId::INT, a_only][..])
        );
        // Of two function types each a subtype of the other, the one
        // interned first stays.
        let named = table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure);
        let unnamed = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert_eq!(table.union([unnamed, named]), named);
        // An error, or nothing, is an error.
        assert_eq!(table.union([TyId::INT, TyId::ERROR]), TyId::ERROR);
        assert_eq!(table.union([]), TyId::ERROR);
        assert_eq!(table.union_members(TyId::INT), None);
    }

    #[test]
    fn the_either_and_the_future_of_a_union() {
        let mut table = TypeTable::new();
        let int_string = table.union([TyId::INT, TyId::STRING]);
        let either = table.deferrable(int_string);
        assert_eq!(table.kind(either), &TyKind::Either(int_string));
        assert_eq!(table.laziness(either), Laziness::Either);
        assert_eq!(table.strip(either), int_string);
        let future = table.future(int_string);
        assert_eq!(table.kind(future), &TyKind::Future(int_string));
        assert_eq!(table.future(either), future);
        assert_eq!(table.deferrable(future), either);
        // A union is present.
        assert_eq!(table.laziness(int_string), Laziness::Present);
        assert_eq!(table.strip(int_string), int_string);
        assert_eq!(table.value_kind(either), Some(ValueKind::Deferred));
    }

    #[test]
    fn subtyping_with_unions() {
        let (mut table, _, [a, b, _]) = setup();
        let int_string = table.union([TyId::INT, TyId::STRING]);
        let three = table.union([TyId::INT, TyId::STRING, TyId::BOOL]);
        // A member, or a subtype of one, is a subtype of the union.
        assert!(table.is_subtype(TyId::INT, int_string));
        assert!(!table.is_subtype(TyId::FLOAT, int_string));
        // A union is a subtype of a type if each of its members is.
        assert!(table.is_subtype(int_string, three));
        assert!(!table.is_subtype(three, int_string));
        assert!(!table.is_subtype(int_string, TyId::INT));
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let b_only = table.record([(b, TyId::INT)]);
        let records = table.union([a_only, b_only]);
        assert!(table.is_subtype(ab, records));
        assert!(table.is_subtype(records, TyId::EMPTY));
        // Through records, lists, functions and futures.
        let holds = table.record([(a, int_string)]);
        let holds_int = table.record([(a, TyId::INT)]);
        assert!(table.is_subtype(holds_int, holds));
        assert!(!table.is_subtype(holds, holds_int));
        let ints = table.list(TyId::INT);
        let either_ints = table.list(int_string);
        assert!(table.is_subtype(ints, either_ints));
        let strings = table.list(TyId::STRING);
        let lists = table.union([ints, strings]);
        assert!(!table.is_subtype(either_ints, lists));
        assert!(table.is_subtype(lists, either_ints));
        let takes_union = table.fn_type([(None, int_string)], TyId::INT, Effect::Pure);
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert!(table.is_subtype(takes_union, takes_int));
        assert!(!table.is_subtype(takes_int, takes_union));
        let future = table.future(TyId::INT);
        let future_union = table.future(int_string);
        assert!(table.is_subtype(future, future_union));
        // The either of a union takes a member, a future of one, and the
        // either of a narrower union; nothing lazy is a subtype of a union.
        let either = table.deferrable(int_string);
        assert!(table.is_subtype(TyId::STRING, either));
        assert!(table.is_subtype(future, either));
        assert!(table.is_subtype(int_string, either));
        assert!(!table.is_subtype(future, int_string));
        assert!(!table.is_subtype(either, int_string));
    }

    #[test]
    fn total_joins_report_widening() {
        let (mut table, _, [a, _, _]) = setup();
        let int_string = table.union([TyId::INT, TyId::STRING]);
        // At the top.
        assert_eq!(table.join(TyId::INT, TyId::STRING), (int_string, true));
        let a_int = table.record([(a, TyId::INT)]);
        let ints = table.list(TyId::INT);
        let record_or_list = table.union([a_int, ints]);
        assert_eq!(table.join(a_int, ints), (record_or_list, true));
        // A union with a type it is a supertype of is itself, not widened;
        // with any other, the union of both sides' members, widened.
        assert_eq!(table.join(int_string, TyId::INT), (int_string, false));
        assert_eq!(table.join(TyId::STRING, int_string), (int_string, false));
        let three = table.union([TyId::INT, TyId::STRING, TyId::BOOL]);
        assert_eq!(table.join(int_string, TyId::BOOL), (three, true));
        assert_eq!(table.join(three, int_string), (three, false));
        // Inside: list elements and record fields.
        let strings = table.list(TyId::STRING);
        let either_list = table.list(int_string);
        assert_eq!(table.join(ints, strings), (either_list, true));
        let a_string = table.record([(a, TyId::STRING)]);
        let a_union = table.record([(a, int_string)]);
        assert_eq!(table.join(a_int, a_string), (a_union, true));
        // Futures and either shapes, on what they yield.
        let future_int = table.future(TyId::INT);
        let future_string = table.future(TyId::STRING);
        let future_union = table.future(int_string);
        assert_eq!(table.join(future_int, future_string), (future_union, true));
        let either_union = table.deferrable(int_string);
        assert_eq!(table.join(future_int, TyId::STRING), (either_union, true));
        assert_eq!(table.join(either_union, TyId::INT), (either_union, false));
        // An error joins to an error, unwidened.
        assert_eq!(table.join(TyId::ERROR, TyId::INT), (TyId::ERROR, false));
    }

    #[test]
    fn strip_present_laziness_and_futures_through_unions() {
        let (mut table, _, [a, _, _]) = setup();
        let future = table.future(TyId::INT);
        let either = table.deferrable(TyId::INT);
        let lazy_record = table.record([(a, either)]);
        let futures = table.list(future);
        let union = table.union([lazy_record, futures]);
        assert_eq!(table.laziness(union), Laziness::Present);
        assert_eq!(table.strip(union), union);
        let present_record = table.record([(a, TyId::INT)]);
        let ints = table.list(TyId::INT);
        let presented = table.union([present_record, ints]);
        assert_eq!(table.present(union), presented);
        assert!(table.contains_future(union));
        let quiet = table.union([lazy_record, TyId::INT]);
        assert!(!table.contains_future(quiet));
        assert_eq!(table.needs_await(union, presented), Some(AwaitKind::Deep));
        // Presenting may make one member a subtype of another.
        let both = table.union([lazy_record, present_record]);
        assert_eq!(table.present(both), present_record);
        // Functions inside.
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        let with_fn = table.union([function, TyId::INT]);
        assert!(table.contains_fn(with_fn));
        assert!(!table.contains_fn(quiet));
    }

    #[test]
    fn provisional_types_through_unions() {
        let (mut table, _, [a, _, _]) = setup();
        let int_string = table.union([TyId::INT, TyId::STRING]);
        let x = table.provisional(F, 0, int_string);
        assert_eq!(table.strip(x), int_string);
        let holder = table.record([(a, x)]);
        let with_holder = table.union([holder, TyId::BOOL]);
        assert!(table.contains_provisional(with_holder, Some(F)));
        let mut found = Vec::new();
        table.provisional_params(with_holder, F, &mut found);
        assert_eq!(found, vec![0]);
        let present_holder = table.record([(a, int_string)]);
        let settled = table.union([present_holder, TyId::BOOL]);
        assert_eq!(table.settle(with_holder, F, &[true]), settled);
        // A provisional type where a member of an expected union is present
        // is one to measure, and not where it is the either shape.
        let expected = table.union([present_holder, TyId::STRING]);
        let mut out = Vec::new();
        table.provisionals_at_present(holder, expected, F, &mut out);
        assert_eq!(out, vec![0]);
        let either = table.deferrable(int_string);
        let lazy_holder = table.record([(a, either)]);
        let lazy_expected = table.union([lazy_holder, TyId::STRING]);
        let mut out = Vec::new();
        table.provisionals_at_present(holder, lazy_expected, F, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn display_unions() {
        let mut names = Names::new();
        let (b, a) = (names.intern("b"), names.intern("a"));
        let mut table = TypeTable::new();
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        // By kind, in the order of `TyKind`, then by text.
        let a_int = table.record([(a, TyId::INT)]);
        let b_int = table.record([(b, TyId::INT)]);
        let ints = table.list(TyId::INT);
        let function = table.fn_type([], TyId::INT, Effect::Pure);
        let union = table.union([function, ints, b_int, a_int, TyId::STRING, TyId::FLOAT]);
        assert_eq!(
            show(&table, union),
            "Float | String | {a: Int} | {b: Int} | List<Int> | (fn(): Int)"
        );
        // Inside a list, a parameter and a field it needs no parentheses;
        // a function type's return type takes it.
        let int_string = table.union([TyId::STRING, TyId::INT]);
        assert_eq!(show(&table, int_string), "Int | String");
        let list = table.list(int_string);
        assert_eq!(show(&table, list), "List<Int | String>");
        let takes = table.fn_type([(Some(a), int_string)], int_string, Effect::Impure);
        assert_eq!(show(&table, takes), "fn(a: Int | String) -> Int | String");
        let record = table.record([(b, int_string)]);
        assert_eq!(show(&table, record), "{b: Int | String}");
        // The either of a union, and a provisional one.
        let either = table.deferrable(int_string);
        assert_eq!(show(&table, either), "Int | String | Future<Int | String>");
        let provisional = table.provisional(F, 0, int_string);
        assert_eq!(show(&table, provisional), "(Int | String)?");
        let with_fn = table.union([function, TyId::INT]);
        let either_fn = table.deferrable(with_fn);
        assert_eq!(
            show(&table, either_fn),
            "Int | (fn(): Int) | Future<Int | (fn(): Int)>"
        );
    }

    #[test]
    fn value_kinds_of_unions() {
        let (mut table, _, [a, b, _]) = setup();
        let int_string = table.union([TyId::INT, TyId::STRING]);
        assert_eq!(table.value_kind(int_string), Some(ValueKind::Mixed));
        let ints = table.list(TyId::INT);
        let strings = table.list(TyId::STRING);
        let lists = table.union([ints, strings]);
        assert_eq!(table.value_kind(lists), Some(ValueKind::List));
        let a_int = table.record([(a, TyId::INT)]);
        let b_int = table.record([(b, TyId::INT)]);
        let records = table.union([a_int, b_int]);
        assert_eq!(table.value_kind(records), Some(ValueKind::Record));
    }

    #[test]
    fn overlapping_types() {
        let (mut table, _, [a, b, _]) = setup();
        assert!(table.overlaps(TyId::INT, TyId::INT));
        assert!(!table.overlaps(TyId::INT, TyId::STRING));
        // Records overlap unless a field both have cannot.
        let a_int = table.record([(a, TyId::INT)]);
        let a_string = table.record([(a, TyId::STRING)]);
        let b_int = table.record([(b, TyId::INT)]);
        assert!(!table.overlaps(a_int, a_string));
        assert!(table.overlaps(a_int, b_int));
        let int_string = table.union([TyId::INT, TyId::STRING]);
        let a_union = table.record([(a, int_string), (b, TyId::INT)]);
        assert!(table.overlaps(a_union, a_int));
        // Lists by their elements; unions by any member.
        let ints = table.list(TyId::INT);
        let strings = table.list(TyId::STRING);
        assert!(!table.overlaps(ints, strings));
        assert!(table.overlaps(int_string, TyId::STRING));
        assert!(!table.overlaps(int_string, TyId::BOOL));
        assert!(!table.overlaps(a_int, ints));
    }

    #[test]
    fn test_plans_look_only_where_in_doubt() {
        let (mut table, _, [a, b, _]) = setup();
        let ints = table.list(TyId::INT);
        let int_string = table.union([TyId::INT, TyId::STRING]);
        // The kind alone, where every value of that kind passes.
        assert_eq!(
            table.test_plan(Some(&[TyId::INT, ints]), ints),
            ValueTest::Kind(ValueKind::List)
        );
        assert_eq!(
            table.test_plan(Some(&[ints]), TyId::INT),
            ValueTest::Kind(ValueKind::Int)
        );
        assert_eq!(
            table.test_plan(Some(&[TyId::INT]), TyId::INT),
            ValueTest::Pass
        );
        // Into the elements, where a list's elements are in doubt.
        let mixed = table.list(int_string);
        assert_eq!(
            table.test_plan(Some(&[mixed]), ints),
            ValueTest::List(Box::new(ValueTest::Kind(ValueKind::Int)))
        );
        // Into the fields in doubt only.
        let ab = table.record([(a, int_string), (b, TyId::INT)]);
        let wants = table.record([(a, TyId::INT), (b, TyId::INT)]);
        assert_eq!(
            table.test_plan(Some(&[ab]), wants),
            ValueTest::Record(Box::new([(a, ValueTest::Kind(ValueKind::Int))]))
        );
        // A field some record type lacks is tested whole.
        let b_only = table.record([(b, TyId::INT)]);
        let a_list = table.record([(a, ints)]);
        assert_eq!(
            table.test_plan(Some(&[b_only]), a_list),
            ValueTest::Record(Box::new([(
                a,
                ValueTest::List(Box::new(ValueTest::Kind(ValueKind::Int)))
            )]))
        );
        // A union is any of its members' tests.
        let strings = table.list(TyId::STRING);
        let value = [TyId::INT, TyId::BOOL, strings];
        let target = table.union([TyId::INT, strings]);
        assert_eq!(
            table.test_plan(Some(&value), target),
            ValueTest::Any(Box::new([
                ValueTest::Kind(ValueKind::Int),
                ValueTest::Kind(ValueKind::List)
            ]))
        );
        assert!(table.test_plan(Some(&[mixed]), ints).walks());
        assert!(!table.test_plan(Some(&value), target).walks());
        // `{}` is every record's kind.
        assert_eq!(
            table.test_plan(None, TyId::EMPTY),
            ValueTest::Kind(ValueKind::Record)
        );
    }

    /// The function type of no parameters returning `Int`, of effect
    /// `effect`, with the meta `meta`, if any.
    fn with_meta(table: &mut TypeTable, effect: Effect, meta: Option<(Effect, TyId)>) -> TyId {
        let meta = meta.map(|(effect, ty)| Meta { effect, ty });
        let none: [(Option<Name>, TyId); 0] = [];
        table.fn_type_with_meta(none, TyId::INT, effect, meta)
    }

    #[test]
    fn a_meta_is_part_of_its_function_type() {
        let (mut table, _, [a, _, _]) = setup();
        let rec = table.record([(a, TyId::INT)]);
        let plain = with_meta(&mut table, Effect::Pure, None);
        let pure = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, rec)));
        assert_ne!(plain, pure);
        assert_eq!(
            with_meta(&mut table, Effect::Pure, Some((Effect::Pure, rec))),
            pure
        );
        // The marker and the type distinguish.
        assert_ne!(
            with_meta(&mut table, Effect::Pure, Some((Effect::Impure, rec))),
            pure
        );
        assert_ne!(
            with_meta(&mut table, Effect::Pure, Some((Effect::Pure, TyId::EMPTY))),
            pure
        );
        assert_eq!(
            table.fn_sig(pure).and_then(|sig| sig.meta),
            Some(Meta {
                effect: Effect::Pure,
                ty: rec
            })
        );
        assert_eq!(table.fn_sig(plain).and_then(|sig| sig.meta), None);
    }

    #[test]
    fn a_meta_is_purely_additive() {
        let (mut table, _, [a, b, _]) = setup();
        let a_only = table.record([(a, TyId::INT)]);
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let plain = with_meta(&mut table, Effect::Pure, None);
        let with_a = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_only)));
        let with_ab = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, ab)));
        // A function with a meta stands in for one without, never the reverse.
        assert!(table.is_subtype(with_a, plain));
        assert!(!table.is_subtype(plain, with_a));
        // The meta's type is covariant, as a result is.
        assert!(table.is_subtype(with_ab, with_a));
        assert!(!table.is_subtype(with_a, with_ab));
        // The markers are ordered as effects are: all nine pairs.
        let effects = [Effect::Pure, Effect::Memo, Effect::Impure];
        for offered in effects {
            for required in effects {
                let sub = with_meta(&mut table, Effect::Pure, Some((offered, a_only)));
                let sup = with_meta(&mut table, Effect::Pure, Some((required, a_only)));
                assert_eq!(
                    table.is_subtype(sub, sup),
                    offered <= required,
                    "{offered:?} against {required:?}"
                );
            }
        }
        // The function's own effect is still compared on its own.
        let impure = with_meta(&mut table, Effect::Impure, Some((Effect::Pure, a_only)));
        assert!(table.is_subtype(with_a, impure));
        assert!(!table.is_subtype(impure, with_a));
    }

    #[test]
    fn metas_join_as_results_do() {
        let (mut table, _, [a, b, _]) = setup();
        let a_only = table.record([(a, TyId::INT)]);
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_float = table.record([(a, TyId::FLOAT)]);
        let plain = with_meta(&mut table, Effect::Pure, None);
        let pure_ab = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, ab)));
        let impure_a = with_meta(&mut table, Effect::Pure, Some((Effect::Impure, a_only)));
        // Both sides: the join of the types and the greater marker.
        assert_eq!(table.join(pure_ab, impure_a), (impure_a, false));
        assert_eq!(table.join(impure_a, pure_ab), (impure_a, false));
        // One side only: forgotten.
        assert_eq!(table.join(pure_ab, plain), (plain, false));
        assert_eq!(table.join(plain, impure_a), (plain, false));
        // A widened meta widens the join.
        let float_a = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_float)));
        let pure_a = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_only)));
        let int_float = table.union([TyId::INT, TyId::FLOAT]);
        let a_number = table.record([(a, int_float)]);
        let joined = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_number)));
        assert_eq!(table.join(pure_a, float_a), (joined, true));
    }

    #[test]
    fn display_metas_as_written() {
        let mut names = Names::new();
        let tag = names.intern("tag");
        let mut table = TypeTable::new();
        let rec = table.record([(tag, TyId::STRING)]);
        let show = |table: &TypeTable, id| table.display(id, &names).to_string();
        for (effect, shown) in [
            (Effect::Pure, "fn(): Int META: {tag: String}"),
            (Effect::Memo, "fn(): Int META->> {tag: String}"),
            (Effect::Impure, "fn(): Int META-> {tag: String}"),
        ] {
            let ty = with_meta(&mut table, Effect::Pure, Some((effect, rec)));
            assert_eq!(show(&table, ty), shown);
        }
        // A function type returned by one with a meta is in parentheses,
        // which would take the meta otherwise; one with a meta returned by
        // one without is not.
        let inner = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, rec)));
        let meta = Some(Meta {
            effect: Effect::Impure,
            ty: rec,
        });
        let none: [(Option<Name>, TyId); 0] = [];
        let plain_inner = table.fn_type(none, TyId::INT, Effect::Pure);
        let outer = table.fn_type_with_meta(none, plain_inner, Effect::Pure, meta);
        assert_eq!(
            show(&table, outer),
            "fn(): (fn(): Int) META-> {tag: String}"
        );
        let outer = table.fn_type(none, inner, Effect::Impure);
        assert_eq!(show(&table, outer), "fn() -> fn(): Int META: {tag: String}");
        // In a union, in parentheses as any function type is.
        let union = table.union([inner, TyId::INT]);
        assert_eq!(show(&table, union), "Int | (fn(): Int META: {tag: String})");
    }

    #[test]
    fn a_meta_s_provisional_types_are_settled() {
        let (mut table, _, [a, _, _]) = setup();
        let function = FnId::new(3);
        let provisional = table.provisional(function, 0, TyId::INT);
        let held = table.record([(a, provisional)]);
        let ty = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, held)));
        assert!(table.contains_provisional(ty, Some(function)));
        let mut params = Vec::new();
        table.provisional_params(ty, function, &mut params);
        assert_eq!(params, [0]);
        let settled = table.settle(ty, function, &[true]);
        let present = table.record([(a, TyId::INT)]);
        assert_eq!(
            settled,
            with_meta(&mut table, Effect::Pure, Some((Effect::Pure, present)))
        );
    }
}
