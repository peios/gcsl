//! Checking `record`.
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

use crate::ast::ExprId;
use crate::diagnostic::Diagnostic;
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
}
