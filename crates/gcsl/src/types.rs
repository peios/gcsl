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
//! `{}`. `Int`, `Float`, `Bool`, `String`, `Duration` and `Size` are
//! subtypes only of themselves (and of the unions they are members of: see
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
//! # Rest-fields
//!
//! A record type may have a *rest-field*, `*: T`, [`RecordTy::rest`]: the
//! type of every field it does not name. `{*: String}` is the type of a
//! record of any fields, each a `String`, and `{from: String, to: String,
//! *: Int}` of a record with a `String` `from` and `to` and any other fields
//! `Int`s: a named field is exempt from the rest-field, whatever its type.
//! A rest-field is part of the type as interned and displayed, shown after
//! the named fields; it has no default.
//!
//! A record value keeps the fields its static type hides (see
//! [Records](#records)), so a type without a rest-field says nothing of
//! the fields it does not name: a value of type `{a: Int}` may hold `b =
//! "x"`. A record type `S` is therefore a subtype of `T` with a rest-field
//! `R` only when `S` has a rest-field that is a subtype of `R` and each
//! named field of `S` that `T` does not name is of a subtype of `R`, as
//! well as having every named field of `T` as ever. A type with a
//! rest-field is a subtype of one without, which forgets it, as width
//! subtyping forgets a field. The checker gives a record block written
//! where a type with a rest-field is expected that rest-field, once each
//! field it does not name is checked against it (see [`crate::check`],
//! "Records"), so a literal can be one.
//!
//! The join of two record types with rest-fields has the rest-field of the
//! join of the two, and of every named field only one side has, which the
//! join no longer names; the join of two record types of which one has no
//! rest-field has none. Two record types overlap when each field one names
//! overlaps the field of its name on the other side, or else the other's
//! rest-field. A test of a type with a rest-field, by `is` or `as`, tests
//! each field of the value that the type does not name against it
//! ([`ValueTest::RecordRest`]), unless the value's own type already
//! guarantees it.
//!
//! A field the type does not name is not read by name: `r.x` of a type with
//! a rest-field but no field `x` is an error, as for any type without the
//! field.
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
//! A function type also carries its *needs* ([`FnTy::needs`]): the contexts
//! with no default that a call of it may read, which the caller must bind
//! (see [Needs](crate::types::needs#needs) and the checker's
//! [Needs](crate::check::needs#needs)). They are shown after its
//! parameters, `fn(x: Int, ^arch): String`. One that needs less stands in
//! for one that needs more, as a pure function for an impure one, never the
//! reverse; two function types join to one needing what either needs.
//! Instantiating or settling a function type keeps its needs.
//!
//! What a memoised function's bare parameter types mean is the checker's
//! business (see [`crate::check`]): they are present, since its arguments
//! are the key its results are memoised by, so the checker interns `fn(x:
//! Int) ->> Int` with a present `Int` parameter, which is not a subtype of
//! `fn(Int) -> Int`, whose bare parameter is the either shape, but is one of
//! `fn(Present<Int>) -> Int`. That is the contravariance above, nothing new.
//!
//! # Generics
//!
//! A generic function's type, `fn<T>(x: T): T`, is a function type whose
//! [`FnTy::generic`] names what its type parameters belong to, a
//! [`GenericId`] made for the function once ([`TypeTable::new_generic`]),
//! and in whose parameters' and result's types each parameter is a
//! [`TyKind::Param`] of it, shown by its name. The parameters of two generic
//! functions are never one type, whatever their names and files. A
//! parameter is opaque: a subtype only of itself, laziness-present to every
//! check that inserts an await (it is never measured), and of the value kind
//! that may hold any value ([`ValueKind::Deferred`]). A call binds each
//! parameter from its arguments' types ([`TypeTable::bind`]) and then uses
//! the signature with each replaced by its binding
//! ([`TypeTable::substitute`]); see the checker's "Generics". Two function
//! types join only if they are of one generic, or neither is generic.
//!
//! # Enums
//!
//! An enum, [`TyKind::Enum`], is nominal, by its key, a name no source can
//! write (see [`crate::names`], "Enum keys"), and shown by its name; it is
//! a subtype only of itself, and contains a function as far as equality
//! knows unless its bound holds none ([`TypeTable::set_enum_bound`]). A
//! variant as a value, `Variant<E, T>` ([`TyKind::Variant`]), is called as
//! its function type, `fn(T): E` ([`TypeTable::fn_sig`]), which it is a
//! subtype of; of other variant types, only of one of its enum and an
//! equivalent payload. Each variant's payload is recorded by its key
//! ([`TypeTable::set_variant_payload`]). See the checker's "Enums".
//!
//! # Secrets
//!
//! `Secret<T>` ([`TyKind::Secret`]) is a value of the present type `T` that
//! never leaks (see the checker's "Secrets"). A plain `T` fits it, and a
//! secret fits another if its value's type fits the other's; a secret fits
//! no plain type. A join of a secret and a value of its type is the
//! secret ([`TypeTable::join`]). Whether a type holds one anywhere, a
//! field's, an element's, a member's or a payload's at any depth, is what
//! makes a comparison of it secret ([`TypeTable::holds_secret`]).
//!
//! A secret may be revealed, `Revealed<T>` (the flag of [`TyKind::Secret`]),
//! which a program may be given: it fits `Secret<T>` of its value, as a
//! plain `T` fits it, and a secret that is not revealed fits no revealed
//! one. A secret of a secret, and a join, is revealed only if both are
//! ([`TypeTable::secret_of`]).
//!
//! # Partial records
//!
//! `Partial<T>` ([`TyKind::Partial`]) holds any of `T`'s fields: a record or
//! another partial record fits it if each of its fields `T` names fits
//! `T`'s; it fits a record type only if that requires no field. See the
//! checker's "Partial records".
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
//! TEMPORARY (alias rule): a built-in function whose typing is special (see
//! [`crate::check::Builtin`]), `len` or `show`, has no type a function type can hold until built-in
//! functions get real polymorphic types (see the [inference
//! limits](https://learn.peios.org/gcsl/language/inference-limits/)). Its value has an opaque type
//! of its
//! own, [`TyKind::Builtin`], shown `builtin len`, which cannot be written in source: a subtype of
//! itself only, holding no future, a function as far as equality goes ([`TypeTable::contains_fn`]),
//! and of kind [`ValueKind::Fn`]. The checker lets it stand only where the alias rule does, so no
//! other type ever holds it ([`TypeTable::special_builtin`]).
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
use crate::value::{Quantity, Value, ValueKind};
use std::num::NonZeroU32;

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
    /// A duration, of whole nanoseconds.
    pub const DURATION: TyId = TyId(7);
    /// A size, of whole bytes.
    pub const SIZE: TyId = TyId(8);
    /// A file, `File`: a path and the content identity a host gives it
    /// (see [Files](crate::check::files#files)).
    pub const FILE: TyId = TyId(9);
    /// A directory, `Dir`: a tree of files, its path and the content
    /// identity a host gives it (see [Files](crate::check::files#files)).
    pub const DIR: TyId = TyId(10);
    /// A sandbox, `Sandbox`: a live copy of a directory that commands run
    /// in, its `/` (see [Files](crate::check::files#files)).
    pub const SANDBOX: TyId = TyId(11);

    /// The type of `quantity`: [`TyId::DURATION`] or [`TyId::SIZE`].
    #[must_use]
    pub const fn of_quantity(quantity: Quantity) -> TyId {
        match quantity {
            Quantity::Duration => TyId::DURATION,
            Quantity::Size => TyId::SIZE,
        }
    }

    /// The quantity this type is, if it is `Duration` or `Size`.
    #[must_use]
    pub const fn quantity(self) -> Option<Quantity> {
        match self {
            TyId::DURATION => Some(Quantity::Duration),
            TyId::SIZE => Some(Quantity::Size),
            _ => None,
        }
    }

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
    /// A duration or a size: an `i64` of nanoseconds or of bytes.
    Quantity(Quantity),
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
    /// TEMPORARY (alias rule): the opaque type of a built-in function whose typing is special,
    /// `builtin len` or `builtin show`, which no type expression can write, until built-in
    /// functions get real polymorphic types (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)). See [Built-in
    /// functions](#built-in-functions).
    Builtin(Builtin),
    /// A type parameter of a generic function, `T` in `fn id<T>(x: T): T`:
    /// the `index`-th of the function type of `generic` (see
    /// [Generics](#generics)). Opaque: a subtype only of itself.
    Param {
        /// The generic function type it belongs to.
        generic: GenericId,
        /// Its position among that type's parameters.
        index: u8,
    },
    /// An extensible enum, `enum Toolchain extensible<B>`: nominal, by its
    /// key, a subtype only of itself, and shown by its name (see
    /// [Enums](#enums)). Its values are its variants' values.
    Enum {
        /// Its key, a name no source can write that is its identity
        /// ([`crate::names::enum_key`]).
        key: Name,
        /// Its name, as it is declared.
        name: Name,
    },
    /// `Variant<E, T>`, a variant of the enum `E` whose payload is of type
    /// `T`, as a value (see [Enums](#enums)): called, `fn(T): E`, its
    /// `call` type, which it is a subtype of, it makes a value of `E`.
    Variant {
        /// The enum, an [`TyKind::Enum`].
        of: TyId,
        /// The payload's type.
        payload: TyId,
        /// Its function type, `fn(T): E`.
        call: TyId,
    },
    /// `Partial<T>` of a record type `T` with no rest-field: the records
    /// holding any of `T`'s fields, each of its type (see [Partial
    /// records](#partial-records)).
    Partial(TyId),
    /// `Secret<T>` of a present type `T`: a value of `T` that never leaks
    /// (see [Secrets](#secrets)), or, if revealed, `Revealed<T>`, one that a
    /// program may be given. Never of another secret.
    Secret(TyId, bool),
    /// `File`, [`TyId::FILE`]: a file, a type of its own that no record fits
    /// (see [Files](crate::check::files#files)).
    File,
    /// `Dir`, [`TyId::DIR`]: a directory, a tree of files, a type of its
    /// own as `File` is (see [Files](crate::check::files#files)).
    Dir,
    /// `Sandbox`, [`TyId::SANDBOX`]: a live copy of a directory that
    /// commands run in, its `/`, a type of its own (see
    /// [Files](crate::check::files#files)).
    Sandbox,
}

/// What a generic function's type parameters belong to: one of its own,
/// made for it once ([`TypeTable::new_generic`]), so that the parameters of
/// two generic functions, of any files, are never confused (see
/// [Generics](#generics)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenericId(NonZeroU32);

/// The fields of a record type, the defaults of those that have one (see
/// [Defaults](#defaults)), and its rest-field, if it has one (see
/// [Rest-fields](#rest-fields)).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordTy {
    /// Each field's name and type, sorted by name, with no name twice: its
    /// *named* fields.
    pub fields: Box<[(Name, TyId)]>,
    /// Its defaults and its rest-field, if it has either; `None` for a
    /// record type with neither, never an empty [`RecordExtras`]. Boxed,
    /// behind a thin pointer, so that a record type, nearly every one of
    /// which has neither, makes a [`TyKind`] no larger than a function
    /// type's signature does.
    pub extras: Option<Box<RecordExtras>>,
}

/// What few record types have beyond their fields: see
/// [`RecordTy::extras`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordExtras {
    /// Each defaulted field's name and default, sorted by name, each a name
    /// of the record type's fields; empty for a record type without
    /// defaults.
    pub defaults: Defaults,
    /// The type of every field that is not a named one, `T` of `*: T`;
    /// `None` for a record type without a rest-field.
    pub rest: Option<TyId>,
}

impl RecordTy {
    /// The record type of `fields`, the rest-field `rest`, if it is one,
    /// and `defaults`, as [`TypeTable::intern`] takes it.
    #[must_use]
    pub fn new(
        fields: Box<[(Name, TyId)]>,
        rest: Option<TyId>,
        defaults: Vec<(Name, Constant)>,
    ) -> Self {
        let extras = (rest.is_some() || !defaults.is_empty()).then(|| {
            Box::new(RecordExtras {
                defaults: defaults.into_boxed_slice(),
                rest,
            })
        });
        Self { fields, extras }
    }

    /// The type of its rest-field, `T` of `*: T`, if it has one.
    #[must_use]
    pub fn rest(&self) -> Option<TyId> {
        self.extras.as_ref().and_then(|extras| extras.rest)
    }

    /// Its defaults, sorted by name; empty if it has none.
    #[must_use]
    pub fn defaults(&self) -> &[(Name, Constant)] {
        self.extras.as_ref().map_or(&[], |extras| &extras.defaults)
    }

    /// The types it holds: each named field's, in order, then its
    /// rest-field's, if it has one. What a walk that looks into every part
    /// of a type alike looks into.
    pub fn types(&self) -> impl Iterator<Item = TyId> + '_ {
        self.fields.iter().map(|&(_, ty)| ty).chain(self.rest())
    }
}

/// The defaults of a record type's fields: see [`RecordExtras::defaults`].
pub type Defaults = Box<[(Name, Constant)]>;

/// Where a def-type's parameters are, by their positions (see the
/// checker's "Def-types"): after the definition's path, the variant its
/// definitions are made with, if they are, and then the definition; its
/// invocation parameters after that ([`TypeTable::def_type_shape`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefTypeShape {
    /// The position of the parameter of the variant, `Variant<E, T>` of an
    /// enum, that a definition's header names, `def Toolchain::C gcc()`:
    /// 1, if the def-type's definitions are made with a variant.
    pub variant: Option<usize>,
    /// The position of the parameter of the definition, a function.
    pub function: usize,
}

impl DefTypeShape {
    /// The position of the def-type's first invocation parameter, after
    /// its definition's.
    #[must_use]
    pub const fn invocation(self) -> usize {
        self.function + 1
    }
}

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
    /// What its type parameters belong to, if it is generic: the
    /// [`TyKind::Param`]s of it in its parameters' and result's types stand
    /// for whatever each call binds them to (see [Generics](#generics)).
    pub generic: Option<GenericId>,
    /// The position of its rest parameter, if it has one, `...words: T`,
    /// whose type in `params` is its elements', `T`: the positional
    /// arguments from that position on fill it, a `List<T>` in the body,
    /// and the parameters after it are given by name only.
    pub rest: Option<u8>,
    /// What a call of it needs: the contexts with no default it may read with
    /// no `with` of its own around the read (see
    /// [Needs](crate::types::needs#needs)).
    pub needs: NeedsId,
}

impl FnTy {
    /// The parameter the positional argument number `index` fills: its
    /// own, or the rest parameter for one at or past its position; `None`
    /// for one past every parameter.
    #[must_use]
    pub fn positional(&self, index: usize) -> Option<usize> {
        match self.rest.map(usize::from) {
            Some(rest) if index >= rest => Some(rest),
            _ => (index < self.params.len()).then_some(index),
        }
    }

    /// This signature without its first `offset` parameters, as a method's
    /// is after its receiver: its rest parameter's position moved down.
    #[must_use]
    pub fn after(&self, offset: usize) -> FnTy {
        let offset_u8 = u8::try_from(offset).unwrap_or(u8::MAX);
        FnTy {
            params: self.params.get(offset..).unwrap_or_default().into(),
            rest: self.rest.and_then(|rest| rest.checked_sub(offset_u8)),
            ..self.clone()
        }
    }
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
    /// Passes for a value of this kind: `Int`, `Float`, `Bool`, `String`,
    /// `Duration` or `Size`, any record or any list; never [`ValueKind::Fn`],
    /// [`ValueKind::Deferred`] or [`ValueKind::Mixed`].
    Kind(ValueKind),
    /// Passes for a record that has each of these fields, whatever else it
    /// has, the value of each passing the field's test. Never empty: a test
    /// of no field is `Kind(ValueKind::Record)`.
    Record(Box<[(Name, ValueTest)]>),
    /// Passes for a record that passes the test of its named fields, and
    /// each of whose other fields passes the test of the rest-field: the
    /// test of a record type with a rest-field whose values may hold fields
    /// that do not fit it (see [Rest-fields](#rest-fields)).
    RecordRest {
        /// The fields to test by name, as [`ValueTest::Record`] has them;
        /// possibly none.
        fields: Box<[(Name, ValueTest)]>,
        /// Every field the type names, sorted, which the test of the
        /// rest-field passes over.
        named: Box<[Name]>,
        /// The test of every other field. Never [`ValueTest::Pass`].
        rest: Box<ValueTest>,
    },
    /// Passes for a list each of whose elements passes the test, the empty
    /// list included. Never of [`ValueTest::Pass`]: that is
    /// `Kind(ValueKind::List)`.
    List(Box<ValueTest>),
    /// Passes for a value that passes any of the tests, of which there are
    /// at least two.
    Any(Box<[ValueTest]>),
    /// Passes for a value of the enum of this key, of any of its variants
    /// (see [Enums](#enums)).
    Enum(Name),
}

impl ValueTest {
    /// Whether the test looks inside the value, at a record's fields or a
    /// list's elements, and so needs them present.
    #[must_use]
    pub fn walks(&self) -> bool {
        match self {
            ValueTest::Pass | ValueTest::Kind(_) | ValueTest::Enum(_) => false,
            ValueTest::Record(_) | ValueTest::RecordRest { .. } | ValueTest::List(_) => true,
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
    /// The names of the type parameters of each generic function type, by
    /// its [`GenericId`]: what a [`TyKind::Param`] is shown as.
    generics: Vec<Box<[Name]>>,
    /// The bound of each enum, by its key, if it has one (see
    /// [Enums](#enums)).
    enum_bounds: HashMap<Name, Option<TyId>>,
    /// The payload's type of each variant, by its key (see [Enums](#enums)).
    variant_payloads: HashMap<Name, TyId>,
    /// The declared type of each context, by its declaration's key (see
    /// the checker's "Context").
    context_types: HashMap<Name, TyId>,
    /// The sets of needs of function types, and what contexts' defaults
    /// need (see [Needs](crate::types::needs#needs)).
    need_table: needs::NeedTable,
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
            generics: Vec::new(),
            enum_bounds: HashMap::new(),
            variant_payloads: HashMap::new(),
            context_types: HashMap::new(),
            need_table: needs::NeedTable::default(),
        };
        // In the order of the constants' numbers.
        for kind in [
            TyKind::Int,
            TyKind::Float,
            TyKind::Bool,
            TyKind::String,
            TyKind::Record(RecordTy {
                fields: Box::default(),
                extras: None,
            }),
            TyKind::UntypedInt,
            TyKind::Error,
            TyKind::Quantity(Quantity::Duration),
            TyKind::Quantity(Quantity::Size),
            TyKind::File,
            TyKind::Dir,
            TyKind::Sandbox,
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
            TyKind::Record(RecordTy { mut fields, extras }) => {
                if !fields.is_sorted_by_key(|&(name, _)| name) {
                    fields.sort_unstable_by_key(|&(name, _)| name);
                }
                let extras = extras.and_then(|mut extras| {
                    if !extras.defaults.is_sorted_by_key(|&(name, _)| name) {
                        extras.defaults.sort_unstable_by_key(|&(name, _)| name);
                    }
                    // No default and no rest-field is no extras.
                    (extras.rest.is_some() || !extras.defaults.is_empty()).then_some(extras)
                });
                TyKind::Record(RecordTy { fields, extras })
            }
            other => other,
        };
        if let Some(&id) = self.lookup.get(&kind) {
            return id;
        }
        let id = TyId(u32::try_from(self.kinds.len()).unwrap_or(u32::MAX));
        let provisional = match &kind {
            TyKind::Provisional { .. } => true,
            TyKind::Record(record) => record.types().any(|field| self.has_provisional(field)),
            TyKind::Fn(sig) => {
                self.has_provisional(sig.ret)
                    || sig.meta.is_some_and(|meta| self.has_provisional(meta.ty))
                    || sig.params.iter().any(|&(_, ty)| self.has_provisional(ty))
            }
            TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Variant { payload: inner, .. }
            | TyKind::Partial(inner)
            | TyKind::Secret(inner, _) => self.has_provisional(*inner),
            TyKind::Union(members) => members.iter().any(|&member| self.has_provisional(member)),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::File
            | TyKind::Dir
            | TyKind::Sandbox
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Builtin(_)
            | TyKind::Param { .. }
            | TyKind::Enum { .. } => false,
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

    /// TEMPORARY (alias rule): the id of the opaque type of `builtin`, a built-in function whose
    /// typing is special, adding it to the table if it is new (see [Built-in
    /// functions](#built-in-functions)); replaced by its polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    pub fn builtin(&mut self, builtin: Builtin) -> TyId {
        self.intern(TyKind::Builtin(builtin))
    }

    /// TEMPORARY (alias rule): whether `id` is the opaque type of a built-in function whose typing
    /// is special (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    #[must_use]
    pub fn is_builtin(&self, id: TyId) -> bool {
        matches!(self.kinds.get(id.index()), Some(TyKind::Builtin(_)))
    }

    /// TEMPORARY (alias rule): the built-in function whose opaque type `id` is, or that a record,
    /// list, future, union or function type `id` holds at any depth: a type no value of which may
    /// stand where the alias rule does not let one (see [Built-in functions](#built-in-functions);
    /// see the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)). `None`
    /// for every
    /// other type.
    ///
    /// Recurses once per level of nesting.
    #[must_use]
    pub fn special_builtin(&self, id: TyId) -> Option<Builtin> {
        match self.kinds.get(id.index())? {
            TyKind::Builtin(builtin) => Some(*builtin),
            TyKind::Record(record) => record.types().find_map(|field| self.special_builtin(field)),
            TyKind::Fn(sig) => sig
                .params
                .iter()
                .find_map(|&(_, param)| self.special_builtin(param))
                .or_else(|| self.special_builtin(sig.ret))
                .or_else(|| sig.meta.and_then(|meta| self.special_builtin(meta.ty))),
            TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Variant { payload: inner, .. }
            | TyKind::Partial(inner)
            | TyKind::Secret(inner, _)
            | TyKind::Provisional { inner, .. } => self.special_builtin(*inner),
            TyKind::Union(members) => members
                .iter()
                .find_map(|&member| self.special_builtin(member)),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::File
            | TyKind::Dir
            | TyKind::Sandbox
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Param { .. }
            | TyKind::Enum { .. } => None,
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
        self.record_with_rest(fields, None, defaults)
    }

    /// Returns the id of the record type with `fields`, in any order, the
    /// rest-field `rest`, if it is one (see [Rest-fields](#rest-fields)),
    /// and the fields named in `defaults` having those defaults, adding it
    /// to the table if it is new. The fields must not repeat a name, and
    /// each default must be of one of them.
    pub fn record_with_rest(
        &mut self,
        fields: impl IntoIterator<Item = (Name, TyId)>,
        rest: Option<TyId>,
        defaults: Vec<(Name, Constant)>,
    ) -> TyId {
        self.intern(TyKind::Record(RecordTy::new(
            fields.into_iter().collect(),
            rest,
            defaults,
        )))
    }

    /// The record type with `fields`, sorted by name, and the rest-field
    /// `rest`, keeping the default that the record type `template` has of
    /// each of the fields that it has one for, adding it to the table if it
    /// is new: a record type rebuilt from another's fields, settled or made
    /// present, which is still what was written (see [Defaults](#defaults)).
    pub fn record_like(
        &mut self,
        template: TyId,
        fields: Vec<(Name, TyId)>,
        rest: Option<TyId>,
    ) -> TyId {
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
        self.record_with_rest(fields, rest, defaults)
    }

    /// The type of the rest-field of record type `id`, `T` of `*: T`, or
    /// `None` if it has none or is not a record type (see
    /// [Rest-fields](#rest-fields)).
    #[must_use]
    pub fn record_rest(&self, id: TyId) -> Option<TyId> {
        match self.kinds.get(id.index()) {
            Some(TyKind::Record(record)) => record.rest(),
            _ => None,
        }
    }

    /// The defaults of the fields of record type `id` that have one, sorted
    /// by name; empty for a record type without defaults and for any other
    /// type.
    #[must_use]
    pub fn defaults(&self, id: TyId) -> &[(Name, Constant)] {
        match self.kinds.get(id.index()) {
            Some(TyKind::Record(record)) => record.defaults(),
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
        self.fn_type_with_rest(params, (ret, effect), meta, None)
    }

    /// Returns the id of the function type [`TypeTable::fn_type_with_meta`]
    /// makes, its parameter at position `rest`, if any, its rest parameter,
    /// of elements of that parameter's type.
    pub fn fn_type_with_rest(
        &mut self,
        params: impl IntoIterator<Item = (Option<Name>, TyId)>,
        (ret, effect): (TyId, Effect),
        meta: Option<Meta>,
        rest_param: Option<u8>,
    ) -> TyId {
        self.intern(TyKind::Fn(FnTy {
            params: params.into_iter().collect(),
            ret,
            effect,
            meta,
            generic: None,
            rest: rest_param,
            needs: NeedsId::NONE,
        }))
    }

    /// A new generic function type's own [`GenericId`], whose type
    /// parameters are named `names`, in order (see [Generics](#generics)).
    pub fn new_generic(&mut self, names: Box<[Name]>) -> GenericId {
        let number = u32::try_from(self.generics.len()).unwrap_or(u32::MAX - 1);
        self.generics.push(names);
        GenericId(NonZeroU32::MIN.saturating_add(number))
    }

    /// The names of the type parameters of `generic`, in order.
    #[must_use]
    pub fn generic_names(&self, generic: GenericId) -> &[Name] {
        self.generics
            .get(generic.0.get() as usize - 1)
            .map_or(&[], |names| names)
    }

    /// Whether a value of type `found` may be the receiver of a method whose
    /// receiver type is `receiver`, a type parameter of `generic`, if the
    /// method is generic, matching any type, each time it occurs: what a
    /// language server offers after a `.`, without binding anything. The
    /// checker's choice of a method binds the parameters
    /// ([`TypeTable::bind`]); this answers for any `found` the checker could
    /// choose the method for, and for a few more, a parameter's occurrences
    /// not being made to agree.
    ///
    /// Recurses once per level of nesting in `receiver`.
    #[must_use]
    pub fn may_receive(&self, found: TyId, receiver: TyId, generic: Option<GenericId>) -> bool {
        let (found, receiver) = (self.strip(found), self.strip(receiver));
        let Some(generic) = generic.filter(|&generic| self.mentions(receiver, generic)) else {
            return self.is_subtype(found, receiver);
        };
        match (self.kind(found), self.kind(receiver)) {
            (_, &TyKind::Param { generic: of, .. }) if of == generic => true,
            (&TyKind::List(element), &TyKind::List(pattern)) => {
                self.may_receive(element, pattern, Some(generic))
            }
            (TyKind::Record(record), TyKind::Record(pattern)) => {
                pattern.fields.iter().all(|&(name, field)| {
                    record
                        .fields
                        .iter()
                        .find(|&&(own, _)| own == name)
                        .is_some_and(|&(_, own)| self.may_receive(own, field, Some(generic)))
                })
            }
            (TyKind::Fn(sig), TyKind::Fn(pattern)) => {
                sig.params.len() == pattern.params.len()
                    && sig.rest == pattern.rest
                    && self.may_receive(sig.ret, pattern.ret, Some(generic))
            }
            (_, TyKind::Union(members)) => members
                .iter()
                .any(|&member| self.may_receive(found, member, Some(generic))),
            (
                &TyKind::Variant { of, payload, .. },
                &TyKind::Variant {
                    of: of_pattern,
                    payload: pattern,
                    ..
                },
            ) => {
                self.may_receive(of, of_pattern, Some(generic))
                    && self.may_receive(payload, pattern, Some(generic))
            }
            (&TyKind::Partial(record), &TyKind::Partial(pattern))
            | (&TyKind::Secret(record, _), &TyKind::Secret(pattern, _)) => {
                self.may_receive(record, pattern, Some(generic))
            }
            _ => false,
        }
    }

    /// Whether `ty` mentions a type parameter of `generic`, at any depth.
    ///
    /// Recurses once per level of nesting in the type.
    #[must_use]
    pub fn mentions(&self, ty: TyId, generic: GenericId) -> bool {
        self.any_param(ty, &|of| of == generic)
    }

    /// Whether `ty` mentions any type parameter, at any depth.
    #[must_use]
    pub fn has_param(&self, ty: TyId) -> bool {
        self.any_param(ty, &|_| true)
    }

    /// Whether `ty` mentions a type parameter of a generic that `of` accepts.
    ///
    /// Recurses once per level of nesting in the type.
    fn any_param(&self, ty: TyId, of: &dyn Fn(GenericId) -> bool) -> bool {
        match self.kind(ty) {
            &TyKind::Param { generic, .. } => of(generic),
            TyKind::Record(record) => record.types().any(|field| self.any_param(field, of)),
            TyKind::Fn(sig) => {
                sig.params
                    .iter()
                    .any(|&(_, param)| self.any_param(param, of))
                    || self.any_param(sig.ret, of)
                    || sig.meta.is_some_and(|meta| self.any_param(meta.ty, of))
            }
            &(TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Partial(inner)
            | TyKind::Secret(inner, _)
            | TyKind::Provisional { inner, .. }) => self.any_param(inner, of),
            TyKind::Union(members) => members.iter().any(|&member| self.any_param(member, of)),
            &TyKind::Variant {
                of: enumeration,
                payload,
                ..
            } => self.any_param(enumeration, of) || self.any_param(payload, of),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::File
            | TyKind::Dir
            | TyKind::Sandbox
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Builtin(_)
            | TyKind::Enum { .. } => false,
        }
    }

    /// `ty` with each type parameter of `generic` replaced by its binding,
    /// `args[index]` (see [Generics](#generics)): every other type rebuilt
    /// as it is, a future's or an either shape's inner type stripped as their
    /// constructors strip it, so that a parameter bound to a lazy type makes
    /// a lazy type of the usual shape, and a union made again.
    ///
    /// Recurses once per level of nesting in the type.
    pub fn substitute(&mut self, ty: TyId, generic: GenericId, args: &[TyId]) -> TyId {
        if !self.mentions(ty, generic) {
            return ty;
        }
        match self.kind(ty).clone() {
            TyKind::Param { generic: of, index } if of == generic => {
                args.get(usize::from(index)).copied().unwrap_or(TyId::ERROR)
            }
            TyKind::Record(record) => {
                let fields: Vec<(Name, TyId)> = record
                    .fields
                    .iter()
                    .map(|&(name, field)| (name, self.substitute(field, generic, args)))
                    .collect();
                let rest = record
                    .rest()
                    .map(|rest| self.substitute(rest, generic, args));
                let defaults = record.defaults().to_vec();
                self.intern(TyKind::Record(RecordTy::new(fields.into(), rest, defaults)))
            }
            TyKind::List(element) => {
                let element = self.substitute(element, generic, args);
                self.list(element)
            }
            TyKind::Future(inner) => {
                let inner = self.substitute(inner, generic, args);
                self.future(inner)
            }
            TyKind::Either(inner) => {
                let inner = self.substitute(inner, generic, args);
                self.deferrable(inner)
            }
            TyKind::Union(members) => {
                let members: Vec<TyId> = members
                    .iter()
                    .map(|&member| self.substitute(member, generic, args))
                    .collect();
                self.union(members)
            }
            TyKind::Fn(sig) => {
                let params: Vec<(Option<Name>, TyId)> = sig
                    .params
                    .iter()
                    .map(|&(name, param)| (name, self.substitute(param, generic, args)))
                    .collect();
                let ret = self.substitute(sig.ret, generic, args);
                let meta = sig.meta.map(|meta| Meta {
                    ty: self.substitute(meta.ty, generic, args),
                    ..meta
                });
                let made = self.fn_type_with_rest(params, (ret, sig.effect), meta, sig.rest);
                let made = self.with_need_set(made, sig.needs);
                match sig.generic {
                    Some(own) => self.generic_fn(made, own),
                    None => made,
                }
            }
            TyKind::Provisional {
                function,
                param,
                inner,
            } => {
                let inner = self.substitute(inner, generic, args);
                self.provisional(function, param, inner)
            }
            TyKind::Variant { of, payload, .. } => {
                let of = self.substitute(of, generic, args);
                let payload = self.substitute(payload, generic, args);
                self.variant(of, payload)
            }
            TyKind::Partial(record) => {
                let record = self.substitute(record, generic, args);
                self.partial(record)
            }
            TyKind::Secret(inner, revealed) => {
                let inner = self.substitute(inner, generic, args);
                self.secret_of(inner, revealed)
            }
            _ => ty,
        }
    }

    /// Binds the type parameters of `generic` that `param`, a parameter's
    /// type, mentions from `arg`, the type of the argument given it, into
    /// `bound`, by their positions (see [Generics](#generics)): a parameter
    /// where `param` is it binds it to `arg`, the join with what it was
    /// bound to already if it was; and inside, a list's element, a record's
    /// field of each name both have, a function's parameters and result, and
    /// what a future or an either shape holds, each against the same of
    /// `arg`'s, its lazy layers stripped, bind what they mention. Nothing
    /// else binds: what does not match is the argument's mismatch, which its
    /// check against the substituted type reports. Returns the position of
    /// the first parameter whose binding this widened to a union, if any.
    ///
    /// Recurses once per level of nesting in `param`.
    pub fn bind(
        &mut self,
        param: TyId,
        arg: TyId,
        generic: GenericId,
        bound: &mut [Option<TyId>],
    ) -> Option<u8> {
        if !self.mentions(param, generic) || arg == TyId::ERROR {
            return None;
        }
        let present = self.strip(arg);
        match self.kind(param).clone() {
            TyKind::Param { generic: of, index } if of == generic => {
                let slot = bound.get_mut(usize::from(index))?;
                let (joined, widened) = match *slot {
                    Some(earlier) => self.join(earlier, arg),
                    None => (arg, false),
                };
                *slot = Some(joined);
                widened.then_some(index)
            }
            TyKind::List(element) => match *self.kind(present) {
                TyKind::List(given) => self.bind(element, given, generic, bound),
                _ => None,
            },
            TyKind::Record(record) => {
                let TyKind::Record(given) = self.kind(present).clone() else {
                    return None;
                };
                let mut widened = None;
                for &(name, field) in &record.fields {
                    if let Some(&(_, of)) = given.fields.iter().find(|&&(given, _)| given == name) {
                        widened = widened.or(self.bind(field, of, generic, bound));
                    }
                }
                widened
            }
            TyKind::Fn(sig) => {
                let given = self.fn_sig(present).cloned()?;
                let mut widened = None;
                for (&(_, param), &(_, of)) in sig.params.iter().zip(given.params.iter()) {
                    widened = widened.or(self.bind(param, of, generic, bound));
                }
                widened.or(self.bind(sig.ret, given.ret, generic, bound))
            }
            TyKind::Future(inner) | TyKind::Either(inner) | TyKind::Provisional { inner, .. } => {
                self.bind(inner, present, generic, bound)
            }
            TyKind::Variant { of, payload, .. } => match *self.kind(present) {
                TyKind::Variant {
                    of: given_of,
                    payload: given,
                    ..
                } => {
                    let widened = self.bind(of, given_of, generic, bound);
                    widened.or(self.bind(payload, given, generic, bound))
                }
                _ => None,
            },
            // A partial record binds from a record's fields or a partial
            // one's, of each name both have.
            TyKind::Partial(record) => {
                let given = match *self.kind(present) {
                    TyKind::Partial(given) => given,
                    _ => present,
                };
                self.bind(record, given, generic, bound)
            }
            // A secret binds from a secret's value or a plain value.
            TyKind::Secret(inner, _) => {
                let given = self.secret_value(present).unwrap_or(present);
                self.bind(inner, given, generic, bound)
            }
            _ => None,
        }
    }

    /// The type parameter number `index` of `generic`.
    pub fn param(&mut self, generic: GenericId, index: u8) -> TyId {
        self.intern(TyKind::Param { generic, index })
    }

    /// The function type `ty`, made generic, its type parameters
    /// belonging to `generic`; `ty` itself if it is no function type.
    pub fn generic_fn(&mut self, ty: TyId, generic: GenericId) -> TyId {
        let Some(TyKind::Fn(sig)) = self.kinds.get(ty.index()).cloned() else {
            return ty;
        };
        self.intern(TyKind::Fn(FnTy {
            generic: Some(generic),
            ..sig
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

    /// Where the parameters of a def-type of type `ty` are, if it is the
    /// type of one (see the checker's "Def-types"): a function whose first
    /// parameter is a `String` and whose second is the definition, a
    /// function, present or not; or whose second is a variant of an enum,
    /// `Variant<E, T>`, and third the definition. A variant is callable, so
    /// it is taken for the variant first.
    #[must_use]
    pub fn def_type_shape(&self, ty: TyId) -> Option<DefTypeShape> {
        let sig = self.fn_sig(self.strip(ty))?;
        let &(_, name) = sig.params.first()?;
        if self.strip(name) != TyId::STRING {
            return None;
        }
        let function = |at: usize| {
            sig.params.get(at).is_some_and(|&(_, param)| {
                let param = self.strip(param);
                !matches!(self.kind(param), TyKind::Variant { .. }) && self.fn_sig(param).is_some()
            })
        };
        let &(_, second) = sig.params.get(1)?;
        match *self.kind(self.strip(second)) {
            TyKind::Variant { of, .. } => (matches!(self.kind(of), TyKind::Enum { .. })
                && function(2))
            .then_some(DefTypeShape {
                variant: Some(1),
                function: 2,
            }),
            _ => function(1).then_some(DefTypeShape {
                variant: None,
                function: 1,
            }),
        }
    }

    /// Whether `value`, data a host gives, is a value of type `ty` (see the
    /// checker's "Context"): of its kind, a record with each named field
    /// fitting, and each other fitting its rest-field if it has one, a
    /// list whose every element fits, a member of a union, a value of an
    /// enum's variant whose payload fits the variant's, a variant of it
    /// whose payload's type is the one written, a partial record whose
    /// fields fit. A function, a future, or anything nested deeper than
    /// [`crate::MAX_DEPTH`] levels is none: the evaluator trusts every value
    /// to be of its node's type, which no host's function or future can be
    /// shown to be.
    ///
    /// Recurses once per level of record, list or payload nesting in
    /// `value`.
    #[must_use]
    pub fn holds(&self, value: &Value, ty: TyId) -> bool {
        self.holds_within(value, ty, crate::MAX_DEPTH)
    }

    /// [`TypeTable::holds`], `depth` more levels allowed.
    fn holds_within(&self, value: &Value, ty: TyId, depth: usize) -> bool {
        let Some(depth) = depth.checked_sub(1) else {
            return false;
        };
        match (self.kind(self.strip(ty)), value) {
            (TyKind::Int, Value::Int(_))
            | (TyKind::Float, Value::Float(_))
            | (TyKind::Bool, Value::Bool(_))
            | (TyKind::String, Value::Str(_))
            | (TyKind::File, Value::File(_))
            | (TyKind::Dir, Value::Dir(_))
            | (TyKind::Sandbox, Value::Sandbox(_)) => true,
            (TyKind::Quantity(wanted), Value::Quantity(quantity, _)) => wanted == quantity,
            (TyKind::Record(record), Value::Empty) => record.fields.is_empty(),
            (TyKind::Record(record), Value::Record(fields)) => {
                let named = record.fields.iter().all(|&(name, field)| {
                    fields
                        .get(name)
                        .is_some_and(|found| self.holds_within(found, field, depth))
                });
                let rest = record.rest().is_none_or(|rest| {
                    fields
                        .layout
                        .names
                        .iter()
                        .zip(&fields.values)
                        .all(|(name, found)| {
                            record.fields.iter().any(|&(field, _)| field == *name)
                                || self.holds_within(found, rest, depth)
                        })
                });
                named && rest
            }
            (TyKind::List(element), Value::List(list)) => list
                .values()
                .iter()
                .all(|found| self.holds_within(found, *element, depth)),
            (TyKind::Union(members), _) => members
                .iter()
                .any(|&member| self.holds_within(value, member, depth)),
            (TyKind::Enum { key, .. }, Value::Variant(variant)) => {
                variant.tag.of == *key
                    && self
                        .variant_payload(variant.tag.key)
                        .is_some_and(|payload| self.holds_within(&variant.payload, payload, depth))
            }
            (TyKind::Variant { of, payload, .. }, Value::Tag(tag)) => {
                matches!(self.kind(*of), TyKind::Enum { key, .. } if *key == tag.of)
                    && self.variant_payload(tag.key) == Some(*payload)
            }
            (TyKind::Partial(full), Value::Empty) => self.is_record(*full),
            (TyKind::Partial(full), Value::Record(fields)) => fields
                .layout
                .names
                .iter()
                .zip(&fields.values)
                .all(|(name, found)| {
                    self.field(*full, *name)
                        .is_some_and(|field| self.holds_within(found, field, depth))
                }),
            _ => false,
        }
    }

    /// The signature of function type `id`, or `None` if it is not a
    /// function type.
    #[must_use]
    pub fn fn_sig(&self, id: TyId) -> Option<&FnTy> {
        match self.kinds.get(id.index())? {
            TyKind::Fn(sig) => Some(sig),
            // A variant is called as its function type.
            &TyKind::Variant { call, .. } => self.fn_sig(call),
            _ => None,
        }
    }

    /// The type of the enum of the key `key`, named `name` (see
    /// [Enums](#enums)), adding it to the table if it is new.
    pub fn enum_type(&mut self, key: Name, name: Name) -> TyId {
        self.intern(TyKind::Enum { key, name })
    }

    /// Records the bound every variant's payload of the enum of the key
    /// `key` fits, if it has one (see [Enums](#enums)).
    pub fn set_enum_bound(&mut self, key: Name, bound: Option<TyId>) {
        self.enum_bounds.insert(key, bound);
    }

    /// The bound of the enum of the key `key`, if it has one; `None` too
    /// for one whose bound is not known.
    #[must_use]
    pub fn enum_bound(&self, key: Name) -> Option<TyId> {
        self.enum_bounds.get(&key).copied().flatten()
    }

    /// Records the declared type of the context whose declaration's key is
    /// `key`, `Error` for one in error (see the checker's "Context").
    pub fn set_context_type(&mut self, key: Name, ty: TyId) {
        self.context_types.insert(key, ty);
    }

    /// The declared type of the context whose declaration's key is `key`, if
    /// it is known.
    #[must_use]
    pub fn context_type(&self, key: Name) -> Option<TyId> {
        self.context_types.get(&key).copied()
    }

    /// Records the payload's type of the variant of the key `key` (see
    /// [Enums](#enums)).
    pub fn set_variant_payload(&mut self, key: Name, payload: TyId) {
        self.variant_payloads.insert(key, payload);
    }

    /// The payload's type of the variant of the key `key`, if it is known.
    #[must_use]
    pub fn variant_payload(&self, key: Name) -> Option<TyId> {
        self.variant_payloads.get(&key).copied()
    }

    /// `Variant<E, T>`, the type of a variant of the enum `of` whose
    /// payload is of type `payload`, as a value, callable as `fn(T): E`
    /// (see [Enums](#enums)), adding it to the table if it is new.
    pub fn variant(&mut self, of: TyId, payload: TyId) -> TyId {
        let call = self.fn_type([(None, payload)], of, Effect::Pure);
        self.intern(TyKind::Variant { of, payload, call })
    }

    /// `Partial<T>` of the record type `record` (see [Partial
    /// records](#partial-records)), adding it to the table if it is new.
    pub fn partial(&mut self, record: TyId) -> TyId {
        self.intern(TyKind::Partial(record))
    }

    /// `Secret<T>` of `value`, `T` (see [Secrets](#secrets)), adding it to
    /// the table if it is new: a secret's value is never a secret, so of a
    /// secret, `Secret<T>` of its value; `Error` for an erroneous one.
    pub fn secret(&mut self, value: TyId) -> TyId {
        self.secret_of(value, false)
    }

    /// `Revealed<T>` of `value`, `T`, a secret a program may be given (see
    /// [Secrets](#secrets)), as [`TypeTable::secret`] makes `Secret<T>`; of
    /// a secret not revealed, that secret, since revealing is `reveal()`'s.
    pub fn revealed(&mut self, value: TyId) -> TyId {
        self.secret_of(value, true)
    }

    /// `Revealed<T>` of `value` if `revealed`, else `Secret<T>`: of a
    /// secret, the secret of its value, revealed only if both are.
    pub fn secret_of(&mut self, value: TyId, revealed: bool) -> TyId {
        if value == TyId::ERROR {
            return value;
        }
        match *self.kind(value) {
            TyKind::Secret(inner, inner_revealed) => {
                self.intern(TyKind::Secret(inner, revealed && inner_revealed))
            }
            _ => self.intern(TyKind::Secret(value, revealed)),
        }
    }

    /// The type of the value of the secret type `id`, if it is one, revealed
    /// or not.
    #[must_use]
    pub fn secret_value(&self, id: TyId) -> Option<TyId> {
        match self.kind(id) {
            &TyKind::Secret(value, _) => Some(value),
            _ => None,
        }
    }

    /// Whether `id` is `Revealed<T>`, a secret a program may be given.
    #[must_use]
    pub fn is_revealed(&self, id: TyId) -> bool {
        matches!(self.kind(id), TyKind::Secret(_, true))
    }

    /// Whether `id` holds a secret that is not revealed anywhere: is one,
    /// or holds one where [`TypeTable::holds_secret`] looks.
    #[must_use]
    pub fn holds_kept_secret(&self, id: TyId) -> bool {
        match self.kind(id) {
            &TyKind::Secret(_, revealed) => !revealed,
            TyKind::Record(record) => {
                record
                    .fields
                    .iter()
                    .any(|&(_, field)| self.holds_kept_secret(field))
                    || record
                        .rest()
                        .is_some_and(|rest| self.holds_kept_secret(rest))
            }
            &(TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Partial(inner)
            | TyKind::Provisional { inner, .. }
            | TyKind::Variant { payload: inner, .. }) => self.holds_kept_secret(inner),
            TyKind::Union(members) => members.iter().any(|&member| self.holds_kept_secret(member)),
            &TyKind::Enum { key, .. } => self
                .enum_bound(key)
                .is_none_or(|bound| self.holds_kept_secret(bound)),
            _ => false,
        }
    }

    /// Whether `id` holds `Revealed<T>` anywhere [`TypeTable::holds_secret`]
    /// looks: whether a secret given where `id` is expected would be given
    /// to a program, revealed.
    #[must_use]
    pub fn holds_revealed(&self, id: TyId) -> bool {
        match self.kind(id) {
            &TyKind::Secret(_, revealed) => revealed,
            TyKind::Record(record) => {
                record
                    .fields
                    .iter()
                    .any(|&(_, field)| self.holds_revealed(field))
                    || record.rest().is_some_and(|rest| self.holds_revealed(rest))
            }
            &(TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Partial(inner)
            | TyKind::Provisional { inner, .. }
            | TyKind::Variant { payload: inner, .. }) => self.holds_revealed(inner),
            TyKind::Union(members) => members.iter().any(|&member| self.holds_revealed(member)),
            _ => false,
        }
    }

    /// `id` with every secret in it its value's type, at any depth: what
    /// a comparison of values holding secrets compares them as (see
    /// [Secrets](#secrets)). A record's defaults are kept.
    ///
    /// Recurses once per level of nesting in `id`.
    pub fn without_secrets(&mut self, id: TyId) -> TyId {
        if !self.holds_secret(id) {
            return id;
        }
        match self.kind(id).clone() {
            TyKind::Secret(value, _) => self.without_secrets(value),
            TyKind::List(element) => {
                let element = self.without_secrets(element);
                self.list(element)
            }
            TyKind::Union(members) => {
                let members: Vec<TyId> = members.iter().map(|&m| self.without_secrets(m)).collect();
                self.union(members)
            }
            TyKind::Record(record) => {
                let fields: Vec<(Name, TyId)> = record
                    .fields
                    .iter()
                    .map(|&(name, field)| (name, self.without_secrets(field)))
                    .collect();
                let rest = record.rest().map(|rest| self.without_secrets(rest));
                self.record_with_rest(fields, rest, record.defaults().to_vec())
            }
            _ => id,
        }
    }

    /// Whether `id` holds a secret anywhere: is one, or is a record type,
    /// a partial one, a list type, a union, a future or an enum's variant
    /// whose field, element, member, value or payload does, at any depth
    /// (see [Secrets](#secrets)). An enum, whose variants' payloads are of
    /// any type its bound allows, holds one if its bound may.
    ///
    /// Recurses once per level of nesting in `id`.
    #[must_use]
    pub fn holds_secret(&self, id: TyId) -> bool {
        match self.kind(id) {
            TyKind::Secret(..) => true,
            TyKind::Record(record) => {
                record
                    .fields
                    .iter()
                    .any(|&(_, field)| self.holds_secret(field))
                    || record.rest().is_some_and(|rest| self.holds_secret(rest))
            }
            &(TyKind::Future(inner)
            | TyKind::Either(inner)
            | TyKind::List(inner)
            | TyKind::Partial(inner)
            | TyKind::Provisional { inner, .. }
            | TyKind::Variant { payload: inner, .. }) => self.holds_secret(inner),
            TyKind::Union(members) => members.iter().any(|&member| self.holds_secret(member)),
            &TyKind::Enum { key, .. } => self
                .enum_bound(key)
                .is_none_or(|bound| self.holds_secret(bound)),
            _ => false,
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
            // TEMPORARY (alias rule): a special built-in's opaque type is a function's (see the
            // [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
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
            // A variant's payload may hold a function, unless its bound
            // says it cannot; a variant itself compares as data.
            Some(&TyKind::Enum { key, .. }) => self
                .enum_bound(key)
                .is_none_or(|bound| self.contains_fn(bound)),
            Some(&TyKind::Partial(record) | &TyKind::Secret(record, _)) => self.contains_fn(record),
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
                let rest = record
                    .rest()
                    .map(|rest| self.settle(rest, function, decisions));
                // Still sorted, so interning does not sort it again.
                self.record_like(id, fields, rest)
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
                let settled = self.fn_type_with_rest(params, (ret, sig.effect), meta, sig.rest);
                let settled = self.with_need_set(settled, sig.needs);
                match sig.generic {
                    Some(generic) => self.generic_fn(settled, generic),
                    None => settled,
                }
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
            TyKind::Variant { of, payload, .. } => {
                let payload = self.settle(payload, function, decisions);
                self.variant(of, payload)
            }
            TyKind::Partial(record) => {
                let record = self.settle(record, function, decisions);
                self.partial(record)
            }
            TyKind::Secret(value, revealed) => {
                let value = self.settle(value, function, decisions);
                self.secret_of(value, revealed)
            }
            TyKind::Provisional { .. }
            | TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::File
            | TyKind::Dir
            | TyKind::Sandbox
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Builtin(_)
            | TyKind::Param { .. }
            | TyKind::Enum { .. } => id,
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
                .types()
                .any(|field| self.contains_provisional(field, Some(function))),
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
                for field in record.types() {
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
                let expected_rest = self.record_rest(expected);
                // A named field that `expected` does not name is held to
                // its rest-field, if it has one.
                for &(name, field) in &record.fields {
                    if let Some(expected_field) = self.field(expected, name).or(expected_rest) {
                        self.provisionals_at_present(field, expected_field, function, out);
                    }
                }
                if let (Some(rest), Some(expected_rest)) = (record.rest(), expected_rest) {
                    self.provisionals_at_present(rest, expected_rest, function, out);
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
        let rest = self.record_rest(top).map(|rest| {
            let field = self.present(rest);
            changed |= field != rest;
            field
        });
        if changed {
            // Still sorted, so interning does not sort it again.
            self.record_like(top, presented, rest)
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
            Some(TyKind::Record(record)) => record.types().any(|field| self.contains_future(field)),
            _ => false,
        }
    }

    /// The record type `record` with field `name` set to type `ty`: the
    /// field's type replaced if `record` has it, and the field added
    /// otherwise. `None` if `record` is not a record type. The type of a
    /// functional update that sets one field. The other fields keep their
    /// defaults, if they have any; the field set has none, its type being
    /// another. A rest-field is kept: a field added is a named one, which
    /// the rest-field does not cover.
    pub fn with_field(&mut self, record: TyId, name: Name, ty: TyId) -> Option<TyId> {
        let rest_field = self.record_rest(record);
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
        Some(self.record_with_rest(updated, rest_field, defaults))
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
                let rests = match (a_record.rest(), b_record.rest()) {
                    (None, None) => true,
                    (Some(a_rest), Some(b_rest)) => self.same(a_rest, b_rest),
                    _ => false,
                };
                rests
                    && a_record.fields.len() == b_record.fields.len()
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
                    && a_sig.needs == b_sig.needs
                    && metas
                    && self.same(a_sig.ret, b_sig.ret)
                    && a_sig.params.len() == b_sig.params.len()
                    && a_sig.rest == b_sig.rest
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

    /// The type record type `record` holds field `name` to: the type of its
    /// field of the name, or else of its rest-field (see
    /// [Rest-fields](#rest-fields)); `None` if it has neither or is not a
    /// record type. What a field written where `record` is expected is
    /// checked against.
    #[must_use]
    pub fn field_or_rest(&self, record: TyId, name: Name) -> Option<TyId> {
        self.field(record, name)
            .or_else(|| self.record_rest(record))
    }

    /// Whether `sub` is a subtype of `sup` where either is a secret (see
    /// [Secrets](#secrets)): a plain value fits a secret of its type, and
    /// a secret only a secret whose value's type its value's fits. `None`
    /// if neither is a secret.
    fn secret_subtype(&self, sub: TyId, sup: TyId) -> Option<bool> {
        match (self.kind(sub), self.kind(sup)) {
            // A secret not revealed is no revealed one.
            (&TyKind::Secret(offered, offered_revealed), &TyKind::Secret(required, revealed)) => {
                Some((offered_revealed || !revealed) && self.is_subtype(offered, required))
            }
            (TyKind::Secret(..), _) => Some(false),
            (_, &TyKind::Secret(required, _)) => Some(self.is_subtype(sub, required)),
            _ => None,
        }
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
        if let Some(fits) = self.secret_subtype(sub, sup) {
            return fits;
        }
        match (self.kind(sub), self.kind(sup)) {
            // A variant fits only a variant of its enum whose payload is
            // its payload's type, since it both makes and unwraps one; a
            // function, however typed, is no variant (see [Enums](#enums)).
            (
                &TyKind::Variant { of, payload, .. },
                &TyKind::Variant {
                    of: of_required,
                    payload: required,
                    ..
                },
            ) => {
                return of == of_required
                    && self.is_subtype(payload, required)
                    && self.is_subtype(required, payload);
            }
            (_, TyKind::Variant { .. } | TyKind::Enum { .. }) => return false,
            (_, &TyKind::Partial(required)) => return self.fits_partial(sub, required),
            // Every field a partial record holds may be absent: it fits a
            // record type that requires none.
            (TyKind::Partial(_), TyKind::Record(record)) => {
                return record.fields.is_empty() && record.rest().is_none();
            }
            _ => {}
        }
        if let (Some(offered), Some(required)) = (self.fn_sig(sub), self.fn_sig(sup)) {
            return self.fn_fits(offered, required);
        }
        let (Some(offered), Some(required)) = (self.record_fields(sub), self.record_fields(sup))
        else {
            return false;
        };
        // A rest-field required covers every field the required type does
        // not name, so the offered type must say what all of those are: by
        // a rest-field of its own, and by each named field it has beyond
        // the required ones (see [Rest-fields](#rest-fields)).
        if let Some(required_rest) = self.record_rest(sup) {
            let Some(offered_rest) = self.record_rest(sub) else {
                return false;
            };
            if !self.is_subtype(offered_rest, required_rest) {
                return false;
            }
            let beyond_fit = offered.iter().all(|&(name, offered_ty)| {
                required
                    .binary_search_by_key(&name, |&(field, _)| field)
                    .is_ok()
                    || self.is_subtype(offered_ty, required_rest)
            });
            if !beyond_fit {
                return false;
            }
        }
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

    /// Whether a variant's payload, of the type `payload` as its declaration
    /// writes it, fits its enum's bound, `bound` (see [Enums](#enums)): as a
    /// record type that has exactly the fields it names, so that a record
    /// type's fields each fit the bound's field of their name or its
    /// rest-field, and its own rest-field fits the bound's, `{}` fitting any
    /// record bound; any other type as a subtype does. A value of the
    /// payload's type may still hold fields its type hides, as any record
    /// may.
    #[must_use]
    pub fn fits_bound(&self, payload: TyId, bound: TyId) -> bool {
        let (TyKind::Record(offered), TyKind::Record(required)) =
            (self.kind(payload), self.kind(bound))
        else {
            return self.is_subtype(payload, bound);
        };
        let rest = required.rest();
        let field_fits =
            |name: Name, ty: TyId| match required.fields.iter().find(|&&(own, _)| own == name) {
                Some(&(_, required_ty)) => self.is_subtype(ty, required_ty),
                None => rest.is_some_and(|rest| self.is_subtype(ty, rest)),
            };
        let required_present = required
            .fields
            .iter()
            .all(|&(name, _)| offered.fields.iter().any(|&(own, _)| own == name));
        required_present
            && offered
                .fields
                .iter()
                .all(|&(name, ty)| field_fits(name, ty))
            && offered
                .rest()
                .is_none_or(|own| rest.is_some_and(|rest| self.is_subtype(own, rest)))
    }

    /// Whether a function of the signature `offered` fits where one of
    /// `required` is expected, as [`TypeTable::is_subtype`] says of function
    /// types (see [Functions](#functions)).
    fn fn_fits(&self, offered: &FnTy, required: &FnTy) -> bool {
        // A meta is purely additive: one required must be offered, as a
        // result is, and one offered may be forgotten.
        let meta = match (offered.meta, required.meta) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(offered), Some(required)) => {
                offered.effect <= required.effect && self.is_subtype(offered.ty, required.ty)
            }
        };
        // A rest parameter, in the same place, of elements the offered
        // accepts, as any parameter is.
        offered.params.len() == required.params.len()
            && offered.rest == required.rest
            && offered.effect <= required.effect
            && self.needs_fit(offered.needs, required.needs)
            && meta
            && self.is_subtype(offered.ret, required.ret)
            && offered
                .params
                .iter()
                .zip(&required.params)
                .all(|(&(_, accepted), &(_, passed))| self.is_subtype(passed, accepted))
    }

    /// Whether a value of type `sub`, present, fits `Partial<R>` of the
    /// record type `required` (see [Partial records](#partial-records)): it
    /// is a record, or a partial record, each field of which `required`
    /// names is of a type that fits `required`'s.
    fn fits_partial(&self, sub: TyId, required: TyId) -> bool {
        let offered = match *self.kind(sub) {
            TyKind::Partial(record) => record,
            _ => sub,
        };
        let (Some(offered), Some(required)) =
            (self.record_fields(offered), self.record_fields(required))
        else {
            return false;
        };
        offered.iter().all(|&(name, offered_ty)| {
            required
                .binary_search_by_key(&name, |&(field, _)| field)
                .ok()
                .and_then(|at| required.get(at))
                .is_none_or(|&(_, required_ty)| self.is_subtype(offered_ty, required_ty))
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
        // A secret's join with a value of its type, or with another secret,
        // is the secret of the join of their values, revealed only if
        // every secret joined is.
        let values = (self.secret_value(a), self.secret_value(b));
        if values.0.is_some() || values.1.is_some() {
            let revealed = [(a, values.0), (b, values.1)]
                .iter()
                .all(|&(ty, value)| value.is_none() || self.is_revealed(ty));
            let (joined, widened) = self.join(values.0.unwrap_or(a), values.1.unwrap_or(b));
            return (self.secret_of(joined, revealed), widened);
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
        let generics = (a_sig.generic, b_sig.generic);
        // Parameters are contravariant, so each is the narrower of the two,
        // which both functions accept: one of them, if it is a subtype of
        // the other. A copy, since joining the return types may intern new
        // types and so needs the table mutably.
        let params: Option<Vec<(Option<Name>, TyId)>> =
            if a_sig.params.len() == b_sig.params.len() && a_sig.rest == b_sig.rest {
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
        // Function types of two generics share no parameters to join.
        let Some(params) = params.filter(|_| generics.0 == generics.1) else {
            return (self.union([a, b]), true);
        };
        let (a_ret, b_ret, effect) = (a_sig.ret, b_sig.ret, a_sig.effect.max(b_sig.effect));
        let (a_needs, b_needs) = (a_sig.needs, b_sig.needs);
        let rest_param = a_sig.rest;
        let metas = (a_sig.meta, b_sig.meta);
        let needs = self.joined_needs(a_needs, b_needs);
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
        let joined = self.fn_type_with_rest(params, (ret, effect), meta, rest_param);
        let joined = self.with_need_set(joined, needs);
        let joined = match generics.0 {
            Some(generic) => self.generic_fn(joined, generic),
            None => joined,
        };
        (joined, widened)
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
        let rests = (self.record_rest(a), self.record_rest(b));
        let mut shared = Vec::with_capacity(a_fields.len().min(b_fields.len()));
        // The types of the named fields only one side has, which a joined
        // rest-field must cover.
        let mut unshared = Vec::new();
        let mut widened = false;
        for &(name, a_ty) in &a_fields {
            let Ok(index) = b_fields.binary_search_by_key(&name, |&(field, _)| field) else {
                unshared.push(a_ty);
                continue;
            };
            let Some(&(_, b_ty)) = b_fields.get(index) else {
                continue;
            };
            let (joined, field_widened) = self.join(a_ty, b_ty);
            widened |= field_widened;
            shared.push((name, joined));
        }
        // The rest-field of the join, if both sides have one: the join of
        // both, and of every named field only one side has, which the join
        // no longer names (see [Rest-fields](#rest-fields)).
        let rest = if let (Some(a_rest), Some(b_rest)) = rests {
            unshared.extend(
                b_fields
                    .iter()
                    .filter(|&&(name, _)| {
                        a_fields
                            .binary_search_by_key(&name, |&(field, _)| field)
                            .is_err()
                    })
                    .map(|&(_, ty)| ty),
            );
            let (mut rest, mut rest_widened) = self.join(a_rest, b_rest);
            for ty in unshared {
                let (joined, field_widened) = self.join(rest, ty);
                rest = joined;
                rest_widened |= field_widened;
            }
            widened |= rest_widened;
            Some(rest)
        } else {
            None
        };
        // `shared` is in `a`'s order, which is sorted, so interning does
        // not sort it again.
        (self.record_with_rest(shared, rest, Vec::new()), widened)
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
            &TyKind::Quantity(quantity) => Some(ValueKind::Quantity(quantity)),
            TyKind::Bool => Some(ValueKind::Bool),
            TyKind::String => Some(ValueKind::String),
            TyKind::Record(_) | TyKind::Partial(_) => Some(ValueKind::Record),
            TyKind::List(_) => Some(ValueKind::List),
            // A secret, a file, and a value of an enum, which is present, are
            // of the kind only they say.
            TyKind::Secret(..)
            | TyKind::File
            | TyKind::Dir
            | TyKind::Sandbox
            | TyKind::Enum { .. } => Some(ValueKind::Mixed),
            // TEMPORARY (alias rule): a special built-in's opaque type is a function's kind (see
            // the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
            // A variant is called as a function is.
            TyKind::Fn(_) | TyKind::Builtin(_) | TyKind::Variant { .. } => Some(ValueKind::Fn),
            // A type parameter's: any value at all, what a call binds it to.
            TyKind::Future(_) | TyKind::Either(_) | TyKind::Param { .. } => {
                Some(ValueKind::Deferred)
            }
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
            (TyKind::Record(a_record), TyKind::Record(b_record)) => {
                // A field one side names is held, on the other, to the field
                // of its name, or else to its rest-field, if it has one.
                let fits = |field: Name, ty: TyId, other: TyId, rest: Option<TyId>| {
                    self.field(other, field)
                        .or(rest)
                        .is_none_or(|other_ty| self.overlaps(ty, other_ty))
                };
                a_record
                    .fields
                    .iter()
                    .all(|&(name, a_field)| fits(name, a_field, b, b_record.rest()))
                    && b_record
                        .fields
                        .iter()
                        .all(|&(name, b_field)| fits(name, b_field, a, a_record.rest()))
            }
            (TyKind::List(a_element), TyKind::List(b_element)) => {
                self.overlaps(*a_element, *b_element)
            }
            (TyKind::Fn(_) | TyKind::Variant { .. }, TyKind::Fn(_) | TyKind::Variant { .. }) => {
                true
            }
            // A partial record is a record whose fields may be absent.
            (&TyKind::Partial(record) | &TyKind::Secret(record, _), _) => self.overlaps(record, b),
            (_, &TyKind::Partial(record) | &TyKind::Secret(record, _)) => self.overlaps(a, record),
            _ => false,
        }
    }

    /// Whether `id` is or holds a type of values `is` cannot tell apart, a
    /// function type or a variant's, as [`TypeTable::contains_fn`] says but
    /// for an enum, whose value is told apart by its variant alone, whatever
    /// its payload holds (see [Enums](#enums)).
    ///
    /// Recurses once per level of record, list, future, union or
    /// provisional nesting.
    #[must_use]
    pub fn holds_untestable(&self, id: TyId) -> bool {
        match self.kinds.get(id.index()) {
            Some(TyKind::Enum { .. }) => false,
            Some(TyKind::Variant { .. }) => true,
            Some(TyKind::Record(record)) => record
                .fields
                .iter()
                .any(|&(_, field)| self.holds_untestable(field)),
            Some(
                TyKind::Future(inner)
                | TyKind::Either(inner)
                | TyKind::List(inner)
                | TyKind::Partial(inner)
                | TyKind::Secret(inner, _)
                | TyKind::Provisional { inner, .. },
            ) => self.holds_untestable(*inner),
            Some(TyKind::Union(members)) => {
                members.iter().any(|&member| self.holds_untestable(member))
            }
            _ => self.contains_fn(id),
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
        // An enum's value is told apart by its variant's enum alone.
        if let &TyKind::Enum { key, .. } = self.kind(ty) {
            return ValueTest::Enum(key);
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
            if let Some(rest) = self.record_rest(ty) {
                // What the fields `ty` does not name may be, by the record
                // types of `value`: each one's rest-field, and its named
                // fields that `ty` does not name; anything at all if one
                // has no rest-field, since a field its type hides may be
                // of any type.
                let others = of_kind.as_ref().and_then(|records| {
                    let mut found = Vec::new();
                    for &record in records {
                        found.push(self.record_rest(record)?);
                        found.extend(
                            self.record_fields(record)
                                .unwrap_or_default()
                                .iter()
                                .filter(|&&(name, _)| self.field(ty, name).is_none())
                                .map(|&(_, field)| field),
                        );
                    }
                    Some(self.flattened(found))
                });
                let rest = self.test_plan(others.as_deref(), rest);
                if rest != ValueTest::Pass {
                    return ValueTest::RecordRest {
                        fields: tests.into_boxed_slice(),
                        named: fields.iter().map(|&(name, _)| name).collect(),
                        rest: Box::new(rest),
                    };
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
    /// `Bool`, `String`, `Duration`, `Size`, records, lists, functions), and among members of
    /// one kind by text.
    #[must_use]
    pub fn shown_members(&self, members: &[TyId], names: &Names) -> Vec<(TyId, String)> {
        let rank = |member: TyId| match self.kinds.get(member.index()) {
            Some(TyKind::Int | TyKind::UntypedInt) => 0,
            Some(TyKind::Float) => 1,
            Some(TyKind::Bool) => 2,
            Some(TyKind::String) => 3,
            Some(TyKind::Quantity(Quantity::Duration)) => 4,
            Some(TyKind::Quantity(Quantity::Size)) => 5,
            Some(TyKind::Record(_)) => 6,
            Some(TyKind::List(_)) => 7,
            Some(TyKind::Fn(_)) => 8,
            _ => 9,
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

mod decode;
mod display;
mod needs;

pub(crate) use display::write_constant;
pub use decode::{Decode, DecodeField, DecodeRecord, Undecodable};
use display::Display;
pub use needs::{Need, NeedsId, implies, met, through};

#[cfg(test)]
mod tests;
