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
    /// A union of at least two types, sorted, deduplicated and flattened
    /// (no member is itself a union). Nothing produces one yet; the kind
    /// exists so that the table's shape does not change when unions arrive.
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
    /// names before it builds a type). A union must already satisfy the
    /// invariants documented on [`TyKind::Union`].
    ///
    /// A table holds at most `u32::MAX` types. Every type but the
    /// pre-interned ones is built from source (a record block or a record
    /// type expression, each at least two bytes of it, or the join of two
    /// types already in the table, which is no larger than either), so the
    /// limit is far beyond any table; past it, ids would saturate.
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
    /// `sub` is `sup`, or both are record types and `sub` has every field
    /// of `sup`, each of a subtype of that field's type in `sup`.
    ///
    /// Recurses once per level of record nesting in `sup`.
    #[must_use]
    pub fn is_subtype(&self, sub: TyId, sup: TyId) -> bool {
        if sub == sup {
            return true;
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
    /// with that join (the empty record if there are none); and `None`
    /// otherwise. Numeric adaptation of untyped integers is the checker's
    /// business, not the table's, so `Int` and an untyped integer have no
    /// join here.
    ///
    /// Recurses once per level of record nesting shared by `a` and `b`.
    pub fn join(&mut self, a: TyId, b: TyId) -> Option<TyId> {
        if a == b {
            return Some(a);
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
    /// `None` for the checker-internal types and unions, which no value
    /// has: an untyped integer or an error never survives a successful
    /// check, and nothing produces a union yet. Also `None`, rather than a
    /// panic, for an id this table did not produce, so that lowering can
    /// report a mismatched table as an internal error.
    #[must_use]
    pub fn value_kind(&self, id: TyId) -> Option<ValueKind> {
        match self.kinds.get(id.index())? {
            TyKind::Int => Some(ValueKind::Int),
            TyKind::Float => Some(ValueKind::Float),
            TyKind::Bool => Some(ValueKind::Bool),
            TyKind::String => Some(ValueKind::String),
            TyKind::Record(_) => Some(ValueKind::Record),
            TyKind::Union(_) | TyKind::UntypedInt | TyKind::Error => None,
        }
    }

    /// Shows type `id` as it is written in source, using `names` for the
    /// names of record fields: `Int`, `{x: Int, y: String}`. A record shows
    /// its fields in alphabetical order of their text, a field whose name is
    /// not a plain identifier quoted as a string literal (`{"a b": Int}`),
    /// and a nested record nested. The empty record is `{}`. An untyped
    /// integer shows as `Int`, an error as `<error>`, and a union as its
    /// members separated by ` | `.
    ///
    /// `names` must be the table the fields' names were interned into.
    /// Recurses once per level of record nesting.
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
            TyKind::Union(members) => {
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
        // An id from a bigger table.
        assert_eq!(TypeTable::new().value_kind(record), None);
    }
}
