//! Checking `record`, and `entries`, its inverse.
//!
//! # Records of any names
//!
//! `record(entries)` is a record whose fields' names are known only as the
//! program runs: one field for each entry of the list `entries`, each a
//! `{key: String, value: T}`, named by its key, whose value is the entry's.
//! Its type is `{*: T}`, a rest-field alone, so that no field of it is read
//! by name, which nothing could know is there; it is given to what takes a
//! record of any names, a program's environment say. A key twice is a
//! runtime error at the call.
//!
//! The type expected of the call reaches it as it reaches `json.decode`'s
//! (see [JSON decoding](crate::check::json#json-decoding)): a `{*: T}`
//! expected makes the entries `List<{key: String, value: T}>` expected, so
//! that `let env: {*: String} = record([])` needs nothing more. Otherwise
//! the entries are checked on their own, and must be a list of records
//! with a `key` that is a `String` and a `value`.
//!
//! The builtin is pure. It names each field as the program's own names
//! name it, so that a record it makes equals one written with the same
//! fields.
//!
//! # Entries of a record
//!
//! `entries(r)` is the list of an entry, `{key: String, value: T}`, for each
//! field of the record `r`, sorted by the field's name: what `record` takes
//! to make `r` again. The record's type must have a rest-field, since a
//! record keeps fields its type hides, of any type (see
//! [`crate::types`]): a type without one says nothing of them, and a field
//! it names is read by name instead. `T` covers every field the type says
//! is there: it is the join of the rest-field's type and of each named
//! field's ([`TypeTable::join`](crate::types::TypeTable::join)), so a
//! `{*: String}` gives `List<{key: String, value: String}>`, and a `{to:
//! String, *: Int}` gives entries whose value is an `Int | String`, which
//! is no warning, since the type says the fields may be either.
//!
//! A rest-field of `*`, any data, gives entries of the type `{key: String,
//! *: *}`: no expression has the type `*`, so each entry's value is a field
//! the entry's own rest-field covers, which a test reads, `e as {value:
//! T}`, as it reads any field `*` covers. Every field the type names must
//! then be data too.
//!
//! The record is awaited deeply, as `record`'s entries are, so that each
//! value is present. The builtin is pure.

use crate::ast::ExprId;
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::types::TyId;

use super::{Builtin, Checker};

impl Checker<'_> {
    /// Types the call `id` of `record`, the callee `callee`, whose argument
    /// is `entries` (see [Records of any names](#records-of-any-names)):
    /// `{*: T}` for entries of `{key: String, value: T}`; `Error` if they are
    /// erroneous or not entries.
    pub(super) fn check_record(&mut self, id: ExprId, callee: ExprId, entries: ExprId) -> TyId {
        let callee_span = self.ast.get(callee).span;
        let names = self.ast.names();
        let (key, value) = (names.lookup("key"), names.lookup("value"));
        // The value type a `{*: T}` expected of the call asks for.
        let expected = self
            .decoding
            .targets
            .remove(&id)
            .map(|ty| self.types.present(ty));
        let wanted = expected.and_then(|ty| {
            let rest = self.types.record_rest(ty)?;
            self.types.record_fields(ty)?.is_empty().then_some(rest)
        });
        let found = if let (Some(rest), Some(key), Some(value)) = (wanted, key, value) {
            let entry = self.types.record([(key, TyId::STRING), (value, rest)]);
            let list = self.types.list(entry);
            self.check_expr_expecting(
                entries,
                list,
                callee_span,
                "`record` makes a record of these entries",
            )
        } else {
            let found = self.check_expr(entries);
            self.settle(entries, found)
        };
        if found == TyId::ERROR {
            return TyId::ERROR;
        }
        let measured = self.measure_deep(entries, found);
        if measured == TyId::ERROR {
            return TyId::ERROR;
        }
        let element = self.element_of(measured);
        let fields = element
            .zip(key)
            .zip(value)
            .and_then(|((element, key), value)| {
                let key = self.types.field(element, key)?;
                let value = self.types.field(element, value)?;
                Some((key, value))
            });
        let entry = fields.filter(|&(key_ty, _)| self.types.is_subtype(key_ty, TyId::STRING));
        let Some((_, value_ty)) = entry else {
            let shown = self.in_message(measured);
            self.errors.push(
                Diagnostic::error(
                    format!("`record` takes a list of `{{key: String, value: T}}` entries, found {shown}"),
                    self.ast.get(entries).span,
                )
                .with_note("each entry gives one field: its key the field's name, its value the field's"),
            );
            return TyId::ERROR;
        };
        let result = self.types.record_with_rest([], Some(value_ty), Vec::new());
        self.expr_types[callee.index()] =
            self.types
                .fn_type([(None, measured)], result, Builtin::Record.effect());
        result
    }

    /// Types the call of `entries`, the callee `callee`, whose argument is
    /// `record` (see [Entries of a record](#entries-of-a-record)): a list of
    /// `{key: String, value: T}`, or of `{key: String, *: *}`; `Error` if the
    /// record is erroneous or its type has no rest-field.
    pub(super) fn check_entries(&mut self, callee: ExprId, record: ExprId) -> TyId {
        let names = self.ast.names();
        let (key, value) = (names.lookup("key"), names.lookup("value"));
        let found = self.check_expr(record);
        let found = self.settle(record, found);
        let measured = self.measure_deep(record, found);
        if measured == TyId::ERROR {
            return TyId::ERROR;
        }
        // A record of type `Never` is never there to list.
        if measured == self.never {
            self.expr_types[callee.index()] =
                self.types
                    .fn_type([(None, measured)], self.never, Builtin::Entries.effect());
            return self.never;
        }
        let (Some(key), Some(value)) = (key, value) else {
            return TyId::ERROR;
        };
        let Some(value_ty) = self.entry_value(record, measured) else {
            return TyId::ERROR;
        };
        let entry = if value_ty == TyId::DATA {
            self.types
                .record_with_rest([(key, TyId::STRING)], Some(TyId::DATA), Vec::new())
        } else {
            self.types.record([(key, TyId::STRING), (value, value_ty)])
        };
        let result = self.types.list(entry);
        self.expr_types[callee.index()] =
            self.types
                .fn_type([(None, measured)], result, Builtin::Entries.effect());
        result
    }

    /// The type of the value of each entry of `record`, the argument of
    /// `entries`, of type `ty`: the join of its type's rest-field and named
    /// fields, `*` if its rest-field is (see [Entries of a
    /// record](#entries-of-a-record)); `None`, the error reported, if `ty`
    /// is no record type with a rest-field, or one of `*` names a field
    /// that is not data.
    fn entry_value(&mut self, record: ExprId, ty: TyId) -> Option<TyId> {
        let span = self.ast.get(record).span;
        let rest = self.types.record_rest(ty);
        let fields: Option<Vec<(Name, TyId)>> = self.types.record_fields(ty).map(<[_]>::to_vec);
        let (Some(rest), Some(fields)) = (rest, fields) else {
            let shown = self.in_message(ty);
            let note = if self.types.is_record(ty) {
                "a record may hold fields its type does not name, so only a type with a \
                 rest-field, `*: T`, says what each field is; read a named field by its name"
            } else {
                "each entry is a field's name and its value"
            };
            self.errors.push(
                Diagnostic::error(
                    format!("`entries` takes a record whose type has a rest-field, found {shown}"),
                    span,
                )
                .with_note(note),
            );
            return None;
        };
        if rest == TyId::DATA {
            let not_data = fields
                .iter()
                .find(|&&(_, field)| !self.types.is_data(field));
            if let Some(&(name, field)) = not_data {
                let shown = self.in_message(field);
                let name = self.ast.names().get(name).to_owned();
                self.errors.push(
                    Diagnostic::error(
                        format!("`entries` lists data, but the field `{name}` is {shown}"),
                        span,
                    )
                    .with_note("the entries of a record whose rest-field is `*` are data"),
                );
                return None;
            }
            return Some(TyId::DATA);
        }
        Some(
            fields
                .into_iter()
                .fold(rest, |joined, (_, field)| self.types.join(joined, field).0),
        )
    }
}
