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
//! themselves. The *join* of two record types is the record of the fields
//! they share whose types have a join, so it always exists; see
//! [`TypeTable::join`].
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
//! of two list types is the list of the join of their element types, if
//! that exists.
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
//! function types have a join when their arities are equal, of each pair
//! of parameter types one is a subtype of the other, and their return types
//! have a join: each parameter is the narrower of its pair, which both
//! functions accept (their types need not be identical, since two functions
//! written with one signature may be inferred different ones), and keeps
//! its name if both sides agree on it; its return type is that join, and
//! its effect is the greater of the two.
//!
//! What a memoised function's bare parameter types mean is the checker's
//! business (see [`crate::check`]): they are present, since its arguments
//! are the key its results are memoised by, so the checker interns `fn(x:
//! Int) ->> Int` with a present `Int` parameter, which is not a subtype of
//! `fn(Int) -> Int`, whose bare parameter is the either shape, but is one of
//! `fn(Present<Int>) -> Int`. That is the contravariance above, nothing new.
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
//! - `T | Future<T>`, the *either* shape, a [`TyKind::Union`] of exactly
//!   those two members: a present `T` or a `Future<T>`. It is the only union
//!   the table builds, through [`TypeTable::deferrable`], and its present
//!   member is never itself a future or an either.
//!
//! Awaiting resolves a future fully, so no program can tell a future of a
//! future from a future of what it yields in the end, and
//! [`TypeTable::future`] interns `Future<Future<Int>>` and `Future<Int |
//! Future<Int>>` as `Future<Int>`: what a future yields is never itself a
//! future or an either (the fields of a record it yields are untouched).
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
use crate::names::{Name, Names};
use crate::strings::FieldName;
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
    /// A union of at least two types, sorted, deduplicated and flattened
    /// (no member is itself a union). The only one built is the either
    /// shape, `T | Future<T>`, through [`TypeTable::deferrable`]; see the
    /// module documentation.
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
        /// either shape or a provisional.
        inner: TyId,
    },
    /// See [`TyId::UNTYPED_INT`].
    UntypedInt,
    /// See [`TyId::ERROR`].
    Error,
}

/// The fields of a record type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordTy {
    /// Each field's name and type, sorted by name, with no name twice.
    pub fields: Box<[(Name, TyId)]>,
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
    /// A record's fields are sorted by name here, so they may be given in
    /// any order; they must not repeat a name (the checker reports repeated
    /// names before it builds a type). A future, a union and a provisional
    /// type must already satisfy the invariants documented on
    /// [`TyKind::Future`], [`TyKind::Union`] and [`TyKind::Provisional`].
    ///
    /// A table holds at most `u32::MAX` types. Every type but the
    /// pre-interned ones is built from source (a record block, a list
    /// literal, a record, list or function type expression, a function
    /// expression or a parameter, each at least two bytes of it, the join of
    /// two types already in the table,
    /// which is no larger than either, or a type already in the table with
    /// its provisionals settled, which is no larger than it), so the limit is
    /// far beyond any table; past it, ids would saturate.
    pub fn intern(&mut self, kind: TyKind) -> TyId {
        let kind = match kind {
            TyKind::Record(RecordTy { mut fields }) => {
                if !fields.is_sorted_by_key(|&(name, _)| name) {
                    fields.sort_unstable_by_key(|&(name, _)| name);
                }
                TyKind::Record(RecordTy { fields })
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
                    || sig.params.iter().any(|&(_, ty)| self.has_provisional(ty))
            }
            TyKind::Future(inner) | TyKind::List(inner) => self.has_provisional(*inner),
            TyKind::Union(members) => members.iter().any(|&member| self.has_provisional(member)),
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::UntypedInt
            | TyKind::Error => false,
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

    /// Returns the id of the record type with `fields`, in any order, adding
    /// it to the table if it is new. The fields must not repeat a name.
    pub fn record(&mut self, fields: impl IntoIterator<Item = (Name, TyId)>) -> TyId {
        self.intern(TyKind::Record(RecordTy {
            fields: fields.into_iter().collect(),
        }))
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
        self.intern(TyKind::Fn(FnTy {
            params: params.into_iter().collect(),
            ret,
            effect,
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

    /// Whether `id` is *unit-like*: `{}`, or a list type whose element type
    /// is unit-like, as `List<{}>` and `List<List<{}>>` are. A value of such
    /// a type holds nothing but empty records and lists of them, so a
    /// statement of one is not an unused value (see [`crate::check`]). A
    /// type that is unit-like only once awaited, as `List<Future<{}>>` is,
    /// is not.
    #[must_use]
    pub fn is_unit_like(&self, id: TyId) -> bool {
        let mut id = id;
        loop {
            if id == TyId::EMPTY {
                return true;
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

    /// Whether `id` is a function type, or a record type with a field of a
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
            Some(TyKind::Fn(_)) => true,
            Some(TyKind::Record(record)) => record
                .fields
                .iter()
                .any(|&(_, field)| self.contains_fn(field)),
            Some(
                TyKind::Future(inner) | TyKind::List(inner) | TyKind::Provisional { inner, .. },
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
    /// new: the one kind of union there is. `Error` for `Error`, which has
    /// no either shape.
    pub fn deferrable(&mut self, inner: TyId) -> TyId {
        let present = self.strip(inner);
        if present == TyId::ERROR {
            return TyId::ERROR;
        }
        let future = self.future(present);
        // A future is interned after what it yields, but the order is the
        // union's invariant, so it is not left to that.
        let mut members = [present, future];
        members.sort_unstable();
        self.intern(TyKind::Union(Box::new(members)))
    }

    /// The present member of `id` if it is an either shape, `T | Future<T>`:
    /// `T`. `None` for every other type, other unions included.
    fn either_member(&self, id: TyId) -> Option<TyId> {
        let Some(TyKind::Union(members)) = self.kinds.get(id.index()) else {
            return None;
        };
        let [first, second] = **members else {
            return None;
        };
        let future_of =
            |future: TyId, of: TyId| self.kinds.get(future.index()) == Some(&TyKind::Future(of));
        if future_of(second, first) {
            Some(first)
        } else if future_of(first, second) {
            Some(second)
        } else {
            None
        }
    }

    /// Whether `id` is an either shape, `T | Future<T>`, the one union a
    /// successful check may leave in its types.
    #[must_use]
    pub fn is_deferrable(&self, id: TyId) -> bool {
        self.either_member(id).is_some()
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
    /// at any depth, through records, lists, futures, unions and function
    /// types,
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
                self.record(fields)
            }
            TyKind::Fn(sig) => {
                let params: Vec<(Option<Name>, TyId)> = sig
                    .params
                    .iter()
                    .map(|&(name, ty)| (name, self.settle(ty, function, decisions)))
                    .collect();
                let ret = self.settle(sig.ret, function, decisions);
                self.fn_type(params, ret, sig.effect)
            }
            TyKind::Future(inner) => {
                let inner = self.settle(inner, function, decisions);
                self.future(inner)
            }
            TyKind::List(element) => {
                let element = self.settle(element, function, decisions);
                self.list(element)
            }
            TyKind::Union(members) => {
                if let Some(present) = self.either_member(id) {
                    let present = self.settle(present, function, decisions);
                    return self.deferrable(present);
                }
                let mut members: Vec<TyId> = members
                    .iter()
                    .map(|&member| self.settle(member, function, decisions))
                    .collect();
                members.sort_unstable();
                members.dedup();
                self.intern(TyKind::Union(members.into_boxed_slice()))
            }
            TyKind::Provisional { .. }
            | TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::UntypedInt
            | TyKind::Error => id,
        }
    }

    /// Whether type `id` is or contains, at any depth, through records,
    /// lists, futures, unions and function types, a provisional type: one of
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
                        .params
                        .iter()
                        .any(|&(_, ty)| self.contains_provisional(ty, Some(function)))
            }
            Some(TyKind::Future(inner) | TyKind::List(inner)) => {
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
    /// futures, unions and function types, in no particular order and
    /// possibly more than once.
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
            }
            Some(TyKind::Future(inner) | TyKind::List(inner)) => {
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
    /// `expected` stripped to match), and through the return types of
    /// function types, but not their parameters: a parameter is
    /// contravariant, so a provisional one fits no worse for being left
    /// undecided. Nothing is added where `expected` is a `Future`, which no
    /// present value fits.
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
            Some(TyKind::Future(inner)) => {
                self.provisionals_at_present(*inner, self.strip(expected), function, out);
            }
            Some(TyKind::Union(members)) => {
                let expected = self.strip(expected);
                for &member in members {
                    self.provisionals_at_present(member, expected, function, out);
                }
            }
            Some(TyKind::Fn(sig)) => {
                if let Some(required) = self.fn_sig(self.strip(expected)) {
                    self.provisionals_at_present(sig.ret, required.ret, function, out);
                }
            }
            _ => {}
        }
    }

    /// What the top of type `id` says about whether its values are futures:
    /// [`Laziness::Future`] for a `Future<T>`, [`Laziness::Either`] for a
    /// `T | Future<T>`, [`Laziness::Provisional`] for a provisional type,
    /// and [`Laziness::Present`] for everything else, a record with futures
    /// in its fields and a list of futures included.
    #[must_use]
    pub fn laziness(&self, id: TyId) -> Laziness {
        match self.kinds.get(id.index()) {
            Some(TyKind::Future(_)) => Laziness::Future,
            Some(TyKind::Union(_)) if self.is_deferrable(id) => Laziness::Either,
            Some(TyKind::Provisional { .. }) => Laziness::Provisional,
            _ => Laziness::Present,
        }
    }

    /// Type `id` with every `Future`, either and provisional layer at its
    /// top removed: the type of its value once awaited fully. `Future<Int>`,
    /// `Int | Future<Int>`, `Int?` and `Int` all strip to `Int`; a record's
    /// fields and a list's element type are left as they are.
    #[must_use]
    pub fn strip(&self, id: TyId) -> TyId {
        let mut id = id;
        loop {
            match self.kinds.get(id.index()) {
                Some(TyKind::Future(inner) | TyKind::Provisional { inner, .. }) => id = *inner,
                Some(TyKind::Union(_)) => match self.either_member(id) {
                    Some(present) => id = present,
                    None => return id,
                },
                _ => return id,
            }
        }
    }

    /// Type `id` stripped (see [`TypeTable::strip`]) and then, if it is a
    /// record type, with each field's type made present in the same way,
    /// and if it is a list type, with its element type made present,
    /// recursively: the type of its value once awaited deeply. Provisional
    /// types are stripped like the either shapes they may become. Function
    /// types are left as they are, parameters and results alike. Interns the
    /// record and list types it makes.
    ///
    /// Recurses once per level of record or list nesting.
    pub fn present(&mut self, id: TyId) -> TyId {
        let top = self.strip(id);
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
            self.record(presented)
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
    /// contains one, at any depth of record and list nesting. An either
    /// shape counts only through its present member, so `Int | Future<Int>`
    /// contains none, a provisional type likewise through its present type,
    /// and function types are not looked into.
    ///
    /// Recurses once per level of record or list nesting.
    #[must_use]
    pub fn contains_future(&self, id: TyId) -> bool {
        match self.kinds.get(id.index()) {
            Some(TyKind::Future(_)) => true,
            Some(TyKind::Provisional { inner, .. } | TyKind::List(inner)) => {
                self.contains_future(*inner)
            }
            Some(TyKind::Union(_)) => self
                .either_member(id)
                .is_some_and(|present| self.contains_future(present)),
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
    /// functional update that sets one field.
    pub fn with_field(&mut self, record: TyId, name: Name, ty: TyId) -> Option<TyId> {
        let fields = self.record_fields(record)?;
        let at = fields.partition_point(|&(field, _)| field < name);
        let replaces = fields.get(at).is_some_and(|&(field, _)| field == name);
        let mut updated = Vec::with_capacity(fields.len() + usize::from(!replaces));
        updated.extend_from_slice(fields.get(..at).unwrap_or_default());
        updated.push((name, ty));
        let rest = at + usize::from(replaces);
        updated.extend_from_slice(fields.get(rest..).unwrap_or_default());
        // Still sorted, so interning does not sort it again.
        Some(self.intern(TyKind::Record(RecordTy {
            fields: updated.into_boxed_slice(),
        })))
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
    /// both are function types and `sub`'s signature fits `sup`'s; or `sup` is
    /// `Future<T>` and `sub` is `Future<S>` with `S <: T`; or `sup` is `T |
    /// Future<T>` and `sub` stripped is a subtype of `T` (see the module
    /// documentation). Nothing lazy is a subtype of a present type. A
    /// provisional type, `T?`, is its either shape as `sub` (so it is a
    /// subtype of exactly what `T | Future<T>` is) and its present type `T`
    /// as `sup`, unless the two are the same provisional type.
    ///
    /// Recurses once per level of record, list, function type or future
    /// nesting in the two types.
    #[must_use]
    pub fn is_subtype(&self, sub: TyId, sup: TyId) -> bool {
        if sub == sup {
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
            Some(TyKind::Union(_)) => {
                return self
                    .either_member(sup)
                    .is_some_and(|required| self.is_subtype(self.strip(sub), required));
            }
            _ => {}
        }
        if self.laziness(sub) != Laziness::Present {
            return false;
        }
        if let (Some(offered), Some(required)) = (self.list_element(sub), self.list_element(sup)) {
            return self.is_subtype(offered, required);
        }
        if let (Some(offered), Some(required)) = (self.fn_sig(sub), self.fn_sig(sup)) {
            return offered.params.len() == required.params.len()
                && offered.effect <= required.effect
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
    /// `if`: `a` itself if the two are the same type; for two record types,
    /// the record of the fields they share whose types have a join, each
    /// with that join (the empty record if there are none); for two list
    /// types, the list of the join of their element types, if they have
    /// one; for two
    /// function types, the join described in the module documentation, if
    /// they have one; for two futures, the future of the join of what they
    /// yield; for a future, an either or a provisional type (which joins as
    /// its either shape) with anything else, the either of the join of both
    /// stripped; each of the last two existing if the inner join does; and
    /// `None` otherwise. Numeric adaptation of untyped
    /// integers is the checker's business, not the table's, so `Int` and an
    /// untyped integer have no join here.
    ///
    /// Recurses once per level of record, list, return type or future
    /// nesting shared by `a` and `b`.
    pub fn join(&mut self, a: TyId, b: TyId) -> Option<TyId> {
        if a == b {
            return Some(a);
        }
        if self.laziness(a) != Laziness::Present || self.laziness(b) != Laziness::Present {
            if let (TyKind::Future(a_inner), TyKind::Future(b_inner)) = (self.kind(a), self.kind(b))
            {
                let (a_inner, b_inner) = (*a_inner, *b_inner);
                let joined = self.join(a_inner, b_inner)?;
                return Some(self.future(joined));
            }
            let (a_present, b_present) = (self.strip(a), self.strip(b));
            let joined = self.join(a_present, b_present)?;
            return Some(self.deferrable(joined));
        }
        if let (Some(a_element), Some(b_element)) = (self.list_element(a), self.list_element(b)) {
            let joined = self.join(a_element, b_element)?;
            return Some(self.list(joined));
        }
        if let (Some(a_sig), Some(b_sig)) = (self.fn_sig(a), self.fn_sig(b)) {
            if a_sig.params.len() != b_sig.params.len() {
                return None;
            }
            // Parameters are contravariant, so each is the narrower of the
            // two, which both functions accept: one of them, if it is a
            // subtype of the other. A copy, since joining the return types
            // may intern new types and so needs the table mutably.
            let params: Vec<(Option<Name>, TyId)> = a_sig
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
                .collect::<Option<_>>()?;
            let (a_ret, b_ret, effect) = (a_sig.ret, b_sig.ret, a_sig.effect.max(b_sig.effect));
            let ret = self.join(a_ret, b_ret)?;
            return Some(self.fn_type(params, ret, effect));
        }
        let (TyKind::Record(a_record), TyKind::Record(b_record)) = (self.kind(a), self.kind(b))
        else {
            return None;
        };
        // Copies of the two field lists (ids only), since joining a field
        // may intern new types and so needs the table mutably.
        let (a_fields, b_fields) = (a_record.fields.clone(), b_record.fields.clone());
        let mut shared = Vec::with_capacity(a_fields.len().min(b_fields.len()));
        for &(name, a_ty) in &a_fields {
            let Ok(index) = b_fields.binary_search_by_key(&name, |&(field, _)| field) else {
                continue;
            };
            let Some(&(_, b_ty)) = b_fields.get(index) else {
                continue;
            };
            if let Some(joined) = self.join(a_ty, b_ty) {
                shared.push((name, joined));
            }
        }
        // `shared` is in `a`'s order, which is sorted, so interning does
        // not sort it again.
        Some(self.record(shared))
    }

    /// The kind of value an expression of type `id` produces at runtime, or
    /// `None` for the checker-internal types and the unions other than the
    /// either shape, which no value has: an untyped integer, a provisional
    /// type or an error never survives a successful check, and no other
    /// union is built. A
    /// `Future` and an either shape are [`ValueKind::Deferred`]: a value
    /// that may be a future. Also `None`, rather than a panic, for an id
    /// this table did not produce, so that lowering can report a mismatched
    /// table as an internal error.
    #[must_use]
    pub fn value_kind(&self, id: TyId) -> Option<ValueKind> {
        match self.kinds.get(id.index())? {
            TyKind::Int => Some(ValueKind::Int),
            TyKind::Float => Some(ValueKind::Float),
            TyKind::Bool => Some(ValueKind::Bool),
            TyKind::String => Some(ValueKind::String),
            TyKind::Record(_) => Some(ValueKind::Record),
            TyKind::List(_) => Some(ValueKind::List),
            TyKind::Fn(_) => Some(ValueKind::Fn),
            TyKind::Future(_) => Some(ValueKind::Deferred),
            TyKind::Union(_) if self.is_deferrable(id) => Some(ValueKind::Deferred),
            TyKind::Union(_) | TyKind::Provisional { .. } | TyKind::UntypedInt | TyKind::Error => {
                None
            }
        }
    }

    /// Shows type `id` as it is written in source, using `names` for the
    /// names of record fields and parameters: `Int`, `{x: Int, y: String}`,
    /// `fn(Int, name: String): Bool`, `fn() -> Int`. A record shows its
    /// fields in alphabetical order of their text, a field whose name is not
    /// a plain identifier quoted as a string literal (`{"a b": Int}`), and a
    /// nested record nested. The empty record is `{}`. A list type shows as
    /// `List<Int>`, however the source wrote it. A function shows its
    /// parameters in order, each with its name if it has one, then `: ` and
    /// its return type if it is pure, ` ->> ` and its return type if it is
    /// memoised, or ` -> ` and its return type if it is impure.
    /// A future shows as `Future<Int>`, and the either shape as `Int |
    /// Future<Int>`, present member first, in parentheses if it is a
    /// function type, `(fn(): Int) | Future<fn(): Int>`, since a function
    /// type's return type takes a union that follows it, as in `fn() -> Int
    /// | Future<Int>`, which returns the either. A provisional type, which
    /// only the types of a failed check hold, shows as its present type
    /// followed by `?`, `Int?`, the present type in parentheses if it is a
    /// function type, `(fn(): Int)?`, as for the either shape. An untyped
    /// integer shows as `Int`, an error as `<error>`, and any other union as
    /// its members separated by ` | `.
    ///
    /// `names` must be the table the names were interned into. Recurses
    /// once per level of record or function type nesting.
    #[must_use]
    pub fn display<'a>(&'a self, id: TyId, names: &'a Names) -> impl fmt::Display + 'a {
        Display {
            table: self,
            names,
            id,
        }
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
    fn nested(&self, id: TyId) -> Self {
        Self {
            table: self.table,
            names: self.names,
            id,
        }
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
            TyKind::Future(inner) => write!(f, "Future<{}>", self.nested(*inner)),
            TyKind::List(element) => write!(f, "List<{}>", self.nested(*element)),
            TyKind::Provisional { inner, .. } => {
                let shown = self.nested(*inner);
                // A function type's return type would take the `?`.
                if self.table.fn_sig(*inner).is_some() {
                    write!(f, "({shown})?")
                } else {
                    write!(f, "{shown}?")
                }
            }
            TyKind::Union(members) => {
                if let Some(present) = self.table.either_member(self.id) {
                    let shown = self.nested(present);
                    // A function type's return type would take the union.
                    return if self.table.fn_sig(present).is_some() {
                        write!(f, "({shown}) | Future<{shown}>")
                    } else {
                        write!(f, "{shown} | Future<{shown}>")
                    };
                }
                for (index, &member) in members.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" | ")?;
                    }
                    self.nested(member).fmt(f)?;
                }
                Ok(())
            }
            TyKind::Record(record) => {
                // Stored by `Name`, shown by text.
                let mut fields: Vec<(&str, TyId)> = record
                    .fields
                    .iter()
                    .map(|&(name, ty)| (self.names.get(name), ty))
                    .collect();
                fields.sort_unstable_by_key(|&(text, _)| text);
                f.write_str("{")?;
                for (index, &(text, ty)) in fields.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: {}", FieldName(text), self.nested(ty))?;
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
                self.nested(sig.ret).fmt(f)
            }
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
    fn interning_is_by_structure() {
        let (mut table, _, [a, b, _]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        // The same fields in another order are the same type.
        assert_eq!(table.record([(b, TyId::STRING), (a, TyId::INT)]), ab);
        assert_eq!(
            table.intern(TyKind::Record(RecordTy {
                fields: Box::new([(b, TyId::STRING), (a, TyId::INT)]),
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
        assert_eq!(table.join(TyId::INT, TyId::INT), Some(TyId::INT));
        assert_eq!(table.join(TyId::EMPTY, TyId::EMPTY), Some(TyId::EMPTY));
        // Different non-record types, or a record and a non-record, have
        // no join.
        assert_eq!(table.join(TyId::INT, TyId::FLOAT), None);
        assert_eq!(table.join(TyId::INT, TyId::UNTYPED_INT), None);
        assert_eq!(table.join(TyId::EMPTY, TyId::STRING), None);
        // Shared fields.
        let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
        let a_int = table.record([(a, TyId::INT)]);
        assert_eq!(table.join(ab, a_int), Some(a_int));
        assert_eq!(table.join(a_int, ab), Some(a_int));
        let bc = table.record([(b, TyId::STRING), (c, TyId::BOOL)]);
        let b_string = table.record([(b, TyId::STRING)]);
        assert_eq!(table.join(ab, bc), Some(b_string));
        assert_eq!(table.join(bc, ab), Some(b_string));
        // No shared fields: the empty record.
        let c_bool = table.record([(c, TyId::BOOL)]);
        assert_eq!(table.join(a_int, c_bool), Some(TyId::EMPTY));
        assert_eq!(table.join(a_int, TyId::EMPTY), Some(TyId::EMPTY));
        // A shared field whose types have no join is not shared.
        let a_float = table.record([(a, TyId::FLOAT)]);
        assert_eq!(table.join(ab, a_float), Some(TyId::EMPTY));
        // Recursive.
        let outer_ab = table.record([(c, ab), (a, TyId::BOOL)]);
        let outer_bc = table.record([(c, bc)]);
        let joined = table.record([(c, b_string)]);
        assert_eq!(table.join(outer_ab, outer_bc), Some(joined));
        let outer_int = table.record([(c, TyId::INT)]);
        assert_eq!(table.join(outer_ab, outer_int), Some(TyId::EMPTY));
        // A field matched out of step: `b` of the second record lies
        // between fields of the first.
        let ac = table.record([(a, TyId::INT), (c, TyId::BOOL)]);
        let bc2 = table.record([(b, TyId::FLOAT), (c, TyId::BOOL)]);
        assert_eq!(table.join(ac, bc2), Some(c_bool));
        let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
        assert_eq!(table.join(abc, bc), Some(bc));
        assert_eq!(table.join(bc, abc), Some(bc));
        assert_eq!(table.join(ac, c_bool), Some(c_bool));
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
                assert_eq!(joined, Some(of(&mut table, a.max(b))), "{a:?} and {b:?}");
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
        assert_eq!(table.join(left, right), Some(joined));
        assert_eq!(table.join(right, left), Some(joined));
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
        assert_eq!(table.join(left, pure_right), Some(pure_join));
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
        assert_eq!(table.join(pure_right, gives_a), Some(gives_empty));
        // A parameter is the narrower of the two types, which both
        // functions accept, if one is a subtype of the other.
        let takes_a = table.fn_type([(None, a_only)], TyId::INT, Effect::Pure);
        let takes_ab = table.fn_type([(None, ab)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(takes_a, takes_ab), Some(takes_ab));
        assert_eq!(table.join(takes_ab, takes_a), Some(takes_ab));
        let either = table.deferrable(TyId::INT);
        let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Pure);
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(takes_either, takes_int), Some(takes_int));
        assert_eq!(table.join(takes_int, takes_either), Some(takes_int));
        // Each parameter on its own.
        let mixed = table.fn_type([(None, either), (None, ab)], TyId::INT, Effect::Pure);
        let other = table.fn_type([(None, TyId::INT), (None, a_only)], TyId::INT, Effect::Pure);
        let narrow = table.fn_type([(None, TyId::INT), (None, ab)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(mixed, other), Some(narrow));
        // Unrelated parameter types have none.
        let unrelated = table.fn_type([(None, b_only)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(takes_a, unrelated), None);
        let takes_bool = table.fn_type([(None, TyId::BOOL)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(takes_int, takes_bool), None);
        // Arities must match, and the return types must have a join.
        let nullary = table.fn_type([], TyId::INT, Effect::Pure);
        let unary = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
        assert_eq!(table.join(nullary, unary), None);
        let gives_float = table.fn_type([], TyId::FLOAT, Effect::Pure);
        assert_eq!(table.join(nullary, gives_float), None);
        // A function and anything else have none; a function and itself is
        // itself.
        assert_eq!(table.join(nullary, TyId::INT), None);
        assert_eq!(table.join(nullary, TyId::EMPTY), None);
        assert_eq!(table.join(nullary, nullary), Some(nullary));
        // Through records: a field whose function types do not join is not
        // shared.
        let with_nullary = table.record([(a, nullary), (b, TyId::INT)]);
        let with_unary = table.record([(a, unary), (b, TyId::INT)]);
        assert_eq!(table.join(with_nullary, with_unary), Some(b_only));
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
        assert_eq!(table.value_kind(union), None);
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
        assert_eq!(
            table.kind(either),
            &TyKind::Union(Box::new([TyId::INT, future]))
        );
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
        // Interning is order-insensitive: the members are sorted.
        assert_eq!(
            table.intern(TyKind::Union(Box::new([TyId::INT, future]))),
            either
        );
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
        assert_eq!(table.join(future, future), Some(future));
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let future_ab = table.future(ab);
        let future_a = table.future(a_only);
        assert_eq!(table.join(future_ab, future_a), Some(future_a));
        // A present value and a future, or an either and anything: the
        // either of the join, stripped.
        assert_eq!(table.join(TyId::INT, future), Some(either));
        assert_eq!(table.join(future, TyId::INT), Some(either));
        assert_eq!(table.join(either, future), Some(either));
        assert_eq!(table.join(either, TyId::INT), Some(either));
        let nested = table.future(future);
        assert_eq!(table.join(nested, TyId::INT), Some(either));
        let either_a = table.deferrable(a_only);
        assert_eq!(table.join(ab, future_a), Some(either_a));
        // Each exists iff the inner join does.
        let float_future = table.future(TyId::FLOAT);
        assert_eq!(table.join(future, float_future), None);
        assert_eq!(table.join(TyId::FLOAT, future), None);
        // Records join field by field, with these rules per field.
        let with_future = table.record([(a, future), (b, TyId::INT)]);
        let with_int = table.record([(a, TyId::INT)]);
        let with_either = table.record([(a, either)]);
        assert_eq!(table.join(with_future, with_int), Some(with_either));
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
        assert_eq!(table.join(x, x), Some(x));
        assert_eq!(table.join(x, y), Some(either));
        assert_eq!(table.join(x, other), Some(either));
        assert_eq!(table.join(x, TyId::INT), Some(either));
        assert_eq!(table.join(TyId::INT, x), Some(either));
        assert_eq!(table.join(x, future), Some(either));
        assert_eq!(table.join(x, either), Some(either));
        assert_eq!(table.join(x, TyId::FLOAT), None);
        // Through records, field by field.
        let holder = table.record([(a, x)]);
        let holds_int = table.record([(a, TyId::INT)]);
        let holds_either = table.record([(a, either)]);
        assert_eq!(table.join(holder, holder), Some(holder));
        assert_eq!(table.join(holder, holds_int), Some(holds_either));
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
        assert_eq!(table.join(abs, a_strings), Some(just_as));
        let ints = table.list(TyId::INT);
        let floats = table.list(TyId::FLOAT);
        assert_eq!(table.join(ints, floats), None);
        assert_eq!(table.join(ints, TyId::INT), None);
        assert_eq!(table.join(ints, TyId::EMPTY), None);
        let future = table.future(TyId::INT);
        let futures = table.list(future);
        let either = table.deferrable(TyId::INT);
        let eithers = table.list(either);
        assert_eq!(table.join(ints, futures), Some(eithers));
        // A list and a future of one join to the either shape.
        let future_ints = table.future(ints);
        let outer = table.deferrable(ints);
        assert_eq!(table.join(ints, future_ints), Some(outer));
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
}
