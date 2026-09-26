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
//! # Functions
//!
//! A function type, [`FnTy`], lists its parameters' types, each with the
//! parameter's name if the type gives one, its return type, and whether it
//! is pure. Names are part of the type as interned and displayed, so
//! `fn(x: Int): Int` and `fn(Int): Int` are different ids, and they are
//! what named arguments match, but subtyping ignores them, so each of those
//! is a subtype of the other.
//!
//! `fn(P1, ..., Pn) m R` is a subtype of `fn(Q1, ..., Qn) m' R'` when the
//! arities are equal, each `Qi` is a subtype of `Pi` (parameters are
//! contravariant: a function that accepts more can stand in for one that
//! accepts less), `R` is a subtype of `R'` (the return type is covariant),
//! and `m` is pure or `m'` impure (a pure function can stand in for an
//! impure one, not the reverse). Two function types have a join when their
//! arities are equal, their parameter types are identical pairwise and
//! their return types have a join: its parameters keep the names both sides
//! agree on, its return type is that join, and it is pure only if both are.
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
//! [`TypeTable::present`] also strips the fields of a record type,
//! recursively (but not the parameters or results of function types), which
//! is what awaiting it deeply does.
//!
//! Subtyping extends to them: `Future<S> <: Future<T>` iff `S <: T`; `S <: T
//! | Future<T>` iff `strip(S) <: T`, whatever shape `S` has (a present value
//! satisfies the either shape, and so do a future and an either whose
//! awaited value would); and nothing lazy is a subtype of a present type,
//! nor is a present type a subtype of `Future<T>`, so a written `Future<T>`
//! demands a deferred value. The rules apply through record fields and
//! function types as the others do. Joins: two futures join to the future
//! of the join of what they yield, and a future or an either with anything
//! else to the either of the join of both sides stripped.
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
//! live id can refer to it unless an equal type is interned again.)

use std::collections::HashMap;
use std::fmt;

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
    /// Whether the function is pure, declared with `:`, rather than impure,
    /// declared with `->`.
    pub pure: bool,
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
    /// names before it builds a type). A future and a union must already
    /// satisfy the invariants documented on [`TyKind::Future`] and
    /// [`TyKind::Union`].
    ///
    /// A table holds at most `u32::MAX` types. Every type but the
    /// pre-interned ones is built from source (a record block, a record or
    /// function type expression or a function expression, each at least two
    /// bytes of it, or the join of two types already in the table, which is
    /// no larger than either), so the limit is far beyond any table; past
    /// it, ids would saturate.
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
        self.kinds.push(kind.clone());
        self.lookup.insert(kind, id);
        id
    }

    /// Returns the id of the record type with `fields`, in any order, adding
    /// it to the table if it is new. The fields must not repeat a name.
    pub fn record(&mut self, fields: impl IntoIterator<Item = (Name, TyId)>) -> TyId {
        self.intern(TyKind::Record(RecordTy {
            fields: fields.into_iter().collect(),
        }))
    }

    /// Returns the id of the function type with `params`, each a parameter's
    /// name, if it has one, and type, returning `ret`, pure if `pure`,
    /// adding it to the table if it is new.
    pub fn fn_type(
        &mut self,
        params: impl IntoIterator<Item = (Option<Name>, TyId)>,
        ret: TyId,
        pure: bool,
    ) -> TyId {
        self.intern(TyKind::Fn(FnTy {
            params: params.into_iter().collect(),
            ret,
            pure,
        }))
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
    /// type that is or contains one, at any depth, looking through futures
    /// and unions: a type whose values cannot be compared, since functions
    /// have no equality.
    ///
    /// Recurses once per level of record, future or union nesting.
    #[must_use]
    pub fn contains_fn(&self, id: TyId) -> bool {
        match self.kinds.get(id.index()) {
            Some(TyKind::Fn(_)) => true,
            Some(TyKind::Record(record)) => record
                .fields
                .iter()
                .any(|&(_, field)| self.contains_fn(field)),
            Some(TyKind::Future(inner)) => self.contains_fn(*inner),
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

    /// What the top of type `id` says about whether its values are futures:
    /// [`Laziness::Future`] for a `Future<T>`, [`Laziness::Either`] for a
    /// `T | Future<T>`, and [`Laziness::Present`] for everything else, a
    /// record with futures in its fields included.
    #[must_use]
    pub fn laziness(&self, id: TyId) -> Laziness {
        match self.kinds.get(id.index()) {
            Some(TyKind::Future(_)) => Laziness::Future,
            Some(TyKind::Union(_)) if self.is_deferrable(id) => Laziness::Either,
            _ => Laziness::Present,
        }
    }

    /// Type `id` with every `Future` and either layer at its top removed:
    /// the type of its value once awaited fully. `Future<Int>`, `Int |
    /// Future<Int>` and `Int` all strip to `Int`; a record's fields are left
    /// as they are.
    #[must_use]
    pub fn strip(&self, id: TyId) -> TyId {
        let mut id = id;
        loop {
            match self.kinds.get(id.index()) {
                Some(TyKind::Future(inner)) => id = *inner,
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
    /// recursively: the type of its value once awaited deeply. Function
    /// types are left as they are, parameters and results alike. Interns the
    /// record types it makes.
    ///
    /// Recurses once per level of record nesting.
    pub fn present(&mut self, id: TyId) -> TyId {
        let top = self.strip(id);
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
    /// type that is or contains one, at any depth of record nesting. An
    /// either shape counts only through its present member, so `Int |
    /// Future<Int>` contains none, and function types are not looked into.
    ///
    /// Recurses once per level of record nesting.
    #[must_use]
    pub fn contains_future(&self, id: TyId) -> bool {
        match self.kinds.get(id.index()) {
            Some(TyKind::Future(_)) => true,
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
    /// are function types and `sub`'s signature fits `sup`'s; or `sup` is
    /// `Future<T>` and `sub` is `Future<S>` with `S <: T`; or `sup` is `T |
    /// Future<T>` and `sub` stripped is a subtype of `T` (see the module
    /// documentation). Nothing lazy is a subtype of a present type.
    ///
    /// Recurses once per level of record, function type or future nesting
    /// in the two types.
    #[must_use]
    pub fn is_subtype(&self, sub: TyId, sup: TyId) -> bool {
        if sub == sup {
            return true;
        }
        match self.kinds.get(sup.index()) {
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
        if let (Some(offered), Some(required)) = (self.fn_sig(sub), self.fn_sig(sup)) {
            return offered.params.len() == required.params.len()
                && (offered.pure || !required.pure)
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
    /// with that join (the empty record if there are none); for two
    /// function types, the join described in the module documentation, if
    /// they have one; for two futures, the future of the join of what they
    /// yield; for a future or an either with anything else, the either of
    /// the join of both stripped; each of the last two existing if the inner
    /// join does; and `None` otherwise. Numeric adaptation of untyped
    /// integers is the checker's business, not the table's, so `Int` and an
    /// untyped integer have no join here.
    ///
    /// Recurses once per level of record, return type or future nesting
    /// shared by `a` and `b`.
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
        if let (Some(a_sig), Some(b_sig)) = (self.fn_sig(a), self.fn_sig(b)) {
            let same_params = a_sig.params.len() == b_sig.params.len()
                && a_sig
                    .params
                    .iter()
                    .zip(&b_sig.params)
                    .all(|(&(_, a_param), &(_, b_param))| a_param == b_param);
            if !same_params {
                return None;
            }
            // A copy of the parameters, since joining the return types may
            // intern new types and so needs the table mutably.
            let params: Vec<(Option<Name>, TyId)> = a_sig
                .params
                .iter()
                .zip(&b_sig.params)
                .map(|(&(a_name, ty), &(b_name, _))| (a_name.filter(|_| a_name == b_name), ty))
                .collect();
            let (a_ret, b_ret, pure) = (a_sig.ret, b_sig.ret, a_sig.pure && b_sig.pure);
            let ret = self.join(a_ret, b_ret)?;
            return Some(self.fn_type(params, ret, pure));
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
    /// either shape, which no value has: an untyped integer or an error
    /// never survives a successful check, and no other union is built. A
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
            TyKind::Fn(_) => Some(ValueKind::Fn),
            TyKind::Future(_) => Some(ValueKind::Deferred),
            TyKind::Union(_) if self.is_deferrable(id) => Some(ValueKind::Deferred),
            TyKind::Union(_) | TyKind::UntypedInt | TyKind::Error => None,
        }
    }

    /// Shows type `id` as it is written in source, using `names` for the
    /// names of record fields and parameters: `Int`, `{x: Int, y: String}`,
    /// `fn(Int, name: String): Bool`, `fn() -> Int`. A record shows its
    /// fields in alphabetical order of their text, a field whose name is not
    /// a plain identifier quoted as a string literal (`{"a b": Int}`), and a
    /// nested record nested. The empty record is `{}`. A function shows its
    /// parameters in order, each with its name if it has one, then `: ` and
    /// its return type if it is pure, or ` -> ` and its return type if not.
    /// A future shows as `Future<Int>`, and the either shape as `Int |
    /// Future<Int>`, present member first, in parentheses if it is a
    /// function type, `(fn(): Int) | Future<fn(): Int>`, since a function
    /// type's return type takes a union that follows it, as in `fn() -> Int
    /// | Future<Int>`, which returns the either. An untyped integer shows as
    /// `Int`, an error as `<error>`, and any other union as its members
    /// separated by ` | `.
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
                f.write_str(if sig.pure { "): " } else { ") -> " })?;
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
        let named = table.fn_type([(Some(a), TyId::INT)], TyId::INT, true);
        let unnamed = table.fn_type([(None, TyId::INT)], TyId::INT, true);
        assert_ne!(named, unnamed);
        assert_eq!(
            table.fn_type([(Some(a), TyId::INT)], TyId::INT, true),
            named
        );
        // Names, types, arity, return type and purity all distinguish.
        assert_ne!(
            table.fn_type([(Some(b), TyId::INT)], TyId::INT, true),
            named
        );
        assert_ne!(
            table.fn_type([(Some(a), TyId::FLOAT)], TyId::INT, true),
            named
        );
        assert_ne!(table.fn_type([], TyId::INT, true), named);
        assert_ne!(
            table.fn_type([(Some(a), TyId::INT)], TyId::BOOL, true),
            named
        );
        assert_ne!(
            table.fn_type([(Some(a), TyId::INT)], TyId::INT, false),
            named
        );
        let sig = table.fn_sig(named).unwrap();
        assert_eq!(&*sig.params, &[(Some(a), TyId::INT)]);
        assert_eq!(sig.ret, TyId::INT);
        assert!(sig.pure);
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
        let plain = table.fn_type([(None, TyId::INT), (None, TyId::STRING)], TyId::BOOL, true);
        assert_eq!(show(&table, plain), "fn(Int, String): Bool");
        let with_names = table.fn_type(
            [(Some(name), TyId::STRING), (Some(times), TyId::INT)],
            TyId::STRING,
            false,
        );
        assert_eq!(
            show(&table, with_names),
            "fn(name: String, times: Int) -> String"
        );
        let record = table.record([(x, TyId::INT)]);
        let returns_record = table.fn_type([], record, true);
        assert_eq!(show(&table, returns_record), "fn(): {x: Int}");
        let int_to_int = table.fn_type([(None, TyId::INT)], TyId::INT, true);
        let higher = table.fn_type([(None, int_to_int)], TyId::INT, true);
        assert_eq!(show(&table, higher), "fn(fn(Int): Int): Int");
        let returns_fn = table.fn_type([], int_to_int, false);
        assert_eq!(show(&table, returns_fn), "fn() -> fn(Int): Int");
        let in_record = table.record([(x, int_to_int)]);
        assert_eq!(show(&table, in_record), "{x: fn(Int): Int}");
        let mixed = table.fn_type(
            [(Some(x), TyId::INT), (None, TyId::EMPTY)],
            TyId::EMPTY,
            true,
        );
        assert_eq!(show(&table, mixed), "fn(x: Int, {}): {}");
    }

    #[test]
    fn function_subtyping() {
        let (mut table, _, [a, b, c]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let f = |table: &mut TypeTable, params: &[TyId], ret, pure| {
            table.fn_type(params.iter().map(|&ty| (None, ty)), ret, pure)
        };
        // Arity must match.
        let unary = f(&mut table, &[TyId::INT], TyId::INT, true);
        let binary = f(&mut table, &[TyId::INT, TyId::INT], TyId::INT, true);
        assert!(!table.is_subtype(unary, binary));
        assert!(!table.is_subtype(binary, unary));
        // Parameters are contravariant: accepting `{a}` is more than
        // accepting `{a, b}`.
        let takes_a = f(&mut table, &[a_only], TyId::INT, true);
        let takes_ab = f(&mut table, &[ab], TyId::INT, true);
        assert!(table.is_subtype(takes_a, takes_ab));
        assert!(!table.is_subtype(takes_ab, takes_a));
        // The return type is covariant.
        let gives_ab = f(&mut table, &[], ab, true);
        let gives_a = f(&mut table, &[], a_only, true);
        assert!(table.is_subtype(gives_ab, gives_a));
        assert!(!table.is_subtype(gives_a, gives_ab));
        // Pure may stand in for impure, not the reverse.
        let impure = f(&mut table, &[TyId::INT], TyId::INT, false);
        assert!(table.is_subtype(unary, impure));
        assert!(!table.is_subtype(impure, unary));
        // Names are ignored.
        let named = table.fn_type([(Some(c), TyId::INT)], TyId::INT, true);
        assert!(table.is_subtype(named, unary));
        assert!(table.is_subtype(unary, named));
        // Different parameter types, or a function and anything else.
        let takes_float = f(&mut table, &[TyId::FLOAT], TyId::INT, true);
        assert!(!table.is_subtype(unary, takes_float));
        assert!(!table.is_subtype(unary, TyId::INT));
        assert!(!table.is_subtype(unary, TyId::EMPTY));
        assert!(!table.is_subtype(TyId::EMPTY, unary));
        // Nested: a function of functions, and a record of functions.
        let takes_takes_a = f(&mut table, &[takes_a], TyId::INT, true);
        let takes_takes_ab = f(&mut table, &[takes_ab], TyId::INT, true);
        assert!(table.is_subtype(takes_takes_ab, takes_takes_a));
        assert!(!table.is_subtype(takes_takes_a, takes_takes_ab));
        let record_pure = table.record([(a, unary), (b, TyId::INT)]);
        let record_impure = table.record([(a, impure)]);
        assert!(table.is_subtype(record_pure, record_impure));
        assert!(!table.is_subtype(record_impure, record_pure));
    }

    #[test]
    fn function_joins() {
        let (mut table, _, [a, b, _]) = setup();
        let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
        let a_only = table.record([(a, TyId::INT)]);
        let b_only = table.record([(b, TyId::INT)]);
        // Names kept where both sides agree, dropped otherwise; the return
        // types' join; pure only if both are.
        let left = table.fn_type([(Some(a), TyId::INT), (Some(a), TyId::BOOL)], ab, true);
        let right = table.fn_type([(Some(a), TyId::INT), (Some(b), TyId::BOOL)], a_only, false);
        let joined = table.fn_type([(Some(a), TyId::INT), (None, TyId::BOOL)], a_only, false);
        assert_eq!(table.join(left, right), Some(joined));
        assert_eq!(table.join(right, left), Some(joined));
        let pure_right = table.fn_type([(Some(a), TyId::INT), (None, TyId::BOOL)], b_only, true);
        let pure_join = table.fn_type([(Some(a), TyId::INT), (None, TyId::BOOL)], b_only, true);
        assert_eq!(table.join(left, pure_right), Some(pure_join));
        let gives_a = table.fn_type([(Some(a), TyId::INT), (None, TyId::BOOL)], a_only, true);
        let gives_empty = table.fn_type(
            [(Some(a), TyId::INT), (None, TyId::BOOL)],
            TyId::EMPTY,
            true,
        );
        assert_eq!(table.join(pure_right, gives_a), Some(gives_empty));
        // Parameter types must be identical, not merely related.
        let takes_a = table.fn_type([(None, a_only)], TyId::INT, true);
        let takes_ab = table.fn_type([(None, ab)], TyId::INT, true);
        assert_eq!(table.join(takes_a, takes_ab), None);
        // Arities must match, and the return types must have a join.
        let nullary = table.fn_type([], TyId::INT, true);
        let unary = table.fn_type([(None, TyId::INT)], TyId::INT, true);
        assert_eq!(table.join(nullary, unary), None);
        let gives_float = table.fn_type([], TyId::FLOAT, true);
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
        let function = table.fn_type([], TyId::INT, true);
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
        let takes_flat = table.fn_type([(None, flat)], flat, true);
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
        let function = table.fn_type([(None, record)], TyId::INT, false);
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
        let function = table.fn_type([(None, either)], future, false);
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
        let takes_either = table.fn_type([(None, either)], TyId::INT, false);
        let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, false);
        assert!(table.is_subtype(takes_either, takes_int));
        assert!(!table.is_subtype(takes_int, takes_either));
        let gives_future = table.fn_type([], future, false);
        let gives_either = table.fn_type([], either, false);
        assert!(table.is_subtype(gives_future, gives_either));
        assert!(!table.is_subtype(gives_either, gives_future));
        // The two callback cases: a pure function whose parameter is
        // present fits `fn(Present<Int>) -> Int`, whose parameter is
        // present and result either, and not `fn(Int) -> Int`, whose
        // parameter is the either too; one whose parameter is the either
        // fits both.
        let double = table.fn_type([(Some(a), TyId::INT)], TyId::INT, true);
        let present_callback = table.fn_type([(None, TyId::INT)], either, false);
        let either_callback = table.fn_type([(None, either)], either, false);
        assert!(table.is_subtype(double, present_callback));
        assert!(!table.is_subtype(double, either_callback));
        let passes = table.fn_type([(Some(a), either)], TyId::INT, true);
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
        let function = table.fn_type([(None, future)], future, true);
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
        let takes_step = table.fn_type([(Some(step), future_empty)], TyId::EMPTY, true);
        assert_eq!(show(&table, takes_step), "fn(step: Future<{}>): {}");
        // A future of a future is interned, and so shown, as a future.
        let nested = table.future(future);
        assert_eq!(show(&table, nested), "Future<Int>");
        let function = table.fn_type([], TyId::INT, true);
        let future_fn = table.future(function);
        assert_eq!(show(&table, future_fn), "Future<fn(): Int>");
        // The either of a function type parenthesises the function type,
        // whose return type would otherwise take the union; not inside
        // `Future<...>`.
        let either_fn = table.deferrable(function);
        assert_eq!(show(&table, either_fn), "(fn(): Int) | Future<fn(): Int>");
        let impure = table.fn_type([], either, false);
        let either_impure = table.deferrable(impure);
        assert_eq!(
            show(&table, either_impure),
            "(fn() -> Int | Future<Int>) | Future<fn() -> Int | Future<Int>>"
        );
        // A function returning an either shows it unparenthesised.
        let callback = table.fn_type([(None, either)], either, false);
        assert_eq!(
            show(&table, callback),
            "fn(Int | Future<Int>) -> Int | Future<Int>"
        );
        let gives_either_fn = table.fn_type([], either_fn, false);
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
}
