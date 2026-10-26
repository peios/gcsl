//! Checking field access: reading a field of a record, and the suggestions for
//! one it lacks.

use crate::ast::{ExprId, ExprKind};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::suggest;
use crate::types::{TyId, TyKind};

use super::{Checker, REST_READ_NOTE};

impl Checker<'_> {
    /// Types the field access `id`, reading field `name` of `record`, of
    /// type `record_ty`: the field's type, as the record type declares it,
    /// or for a union of record types each with the field, the join of
    /// theirs (see [`Checker::field_of`]). A record that may be a future is
    /// awaited at the top first. Reading a field the record type lacks, or
    /// reading any field of a value that is not a record, or of a union not
    /// every member of which has the field, is reported at the `.NAME`, a
    /// union's with the note saying to tell its members apart first.
    pub(super) fn field_access(
        &mut self,
        id: ExprId,
        (record, record_ty): (ExprId, TyId),
        name: Name,
    ) -> TyId {
        let record_ty = self.measure(record, record_ty);
        // A record of type `Never` is never there to read.
        if record_ty == TyId::ERROR || record_ty == self.never {
            return record_ty;
        }
        // A secret record's field is a secret (see [Secrets](crate::check::secrets#secrets)).
        if let Some(value) = self.types.secret_value(record_ty) {
            let found = self.field_access(id, (record, value), name);
            return self.secret_result(id, found, [(record, Some(value))]);
        }
        // A field of a function with a meta is the meta's.
        if let Some(meta) = self.types.fn_sig(record_ty).and_then(|sig| sig.meta) {
            return self.meta_read(id, record, meta, name);
        }
        if let ty @ (TyId::FILE | TyId::DIR) = self.types.strip(record_ty) {
            return self.file_field(id, name, ty);
        }
        if let Some(ty) = self.field_of(record_ty, name) {
            return ty;
        }
        let expr = self.ast.get(id);
        let member = expr.member_span(self.ast).unwrap_or(expr.span);
        let text = FieldName(self.ast.names().get(name));
        if let Some(fields) = self.types.record_fields(record_ty) {
            let rest = self.types.record_rest(record_ty).is_some();
            // A record type with a rest-field and no named field has fields;
            // it names none.
            let note = (!(rest && fields.is_empty())).then(|| self.field_list(fields, rest));
            let suggestion = self.field_suggestion(id, name, fields);
            let message = format!("no field `{text}` in {}", self.in_message(record_ty));
            let mut error = Diagnostic::error(message, member).with_suggestion(suggestion);
            if let Some(note) = note {
                error = error.with_note(note);
            }
            if rest {
                error = error.with_note(REST_READ_NOTE);
            }
            // A path of the view's names nothing then: its field may be
            // declared in source that could not be parsed.
            self.unresolved(error);
            return TyId::ERROR;
        }
        let record_ty = self.settle(record, record_ty);
        // A partial record's field may be absent (see [Partial
        // records](crate::check::enums#partial-records)).
        if let &TyKind::Partial(full) = self.types.kind(self.types.strip(record_ty))
            && self.types.field(full, name).is_some()
        {
            let shown = self.in_message(record_ty);
            self.errors.push(
                Diagnostic::error(format!("field `{text}` of {shown} may be absent"), member)
                    .with_note("match a whole record against it with `.matches(...)`"),
            );
            return TyId::ERROR;
        }
        let message = format!(
            "cannot access field `{text}` of {}",
            self.in_message(record_ty)
        );
        let label = format!("this is {}", self.in_label(record_ty));
        let mut error =
            Diagnostic::error(message, member).with_context(self.ast.get(record).span, label);
        if let Some(note) = self.union_note(record, record_ty) {
            error = error.with_note(note);
        }
        self.errors.push(error);
        TyId::ERROR
    }

    /// The suggestion for the field `name` that the field access `id` reads
    /// and that its record, of the record type with `fields`, lacks (see
    /// [Suggestions](crate::check::names#suggestions)): the field nearest it, written as a
    /// field's name is, in place of the name as it is written, plain or
    /// quoted.
    pub(super) fn field_suggestion(
        &self,
        id: ExprId,
        name: Name,
        fields: &[(Name, TyId)],
    ) -> Option<(Span, String)> {
        let expr = self.ast.get(id);
        if !matches!(expr.kind, ExprKind::Field { .. }) {
            return None;
        }
        let names = self.ast.names();
        let text = names.get(name);
        let candidates = fields
            .iter()
            .map(|&(field, _)| names.get(field))
            .filter(|field| !crate::names::is_namespace_key(field))
            .map(|field| (field, 0));
        let best = suggest::best(text, candidates)?;
        let span = self.ast.written_name(expr.span.end, text);
        Some((span, FieldName(best).to_string()))
    }

    /// The note listing a record type's `fields`, in alphabetical order:
    /// "the fields are `x` and `y`", or for a record type with a
    /// rest-field, whose other fields it does not name, "the named fields
    /// are `x` and `y`".
    pub(super) fn field_list(&self, fields: &[(Name, TyId)], rest: bool) -> String {
        let names = self.ast.names();
        let mut texts: Vec<&str> = fields.iter().map(|&(name, _)| names.get(name)).collect();
        texts.sort_unstable();
        let which = if rest { "named " } else { "" };
        match texts.as_slice() {
            [] => format!("the record has no {which}fields"),
            [only] => format!("the only {which}field is `{}`", FieldName(only)),
            [init @ .., last] => {
                let init: Vec<String> = init
                    .iter()
                    .map(|text| format!("`{}`", FieldName(text)))
                    .collect();
                format!(
                    "the {which}fields are {} and `{}`",
                    init.join(", "),
                    FieldName(last)
                )
            }
        }
    }

    /// The type of field `name` of `ty`, a type once measured, if it is a
    /// record type with that field, or a union whose members are all record
    /// types with it: the join of their fields' types, which is no warning,
    /// since the union says the value is any of them. `None` for any other
    /// type.
    pub(super) fn field_of(&mut self, ty: TyId, name: Name) -> Option<TyId> {
        if self.types.is_record(ty) {
            return self.types.field(ty, name);
        }
        let members = self.types.union_members(ty)?.to_vec();
        let mut joined: Option<TyId> = None;
        for member in members {
            let field = self.types.field(member, name)?;
            joined = Some(match joined {
                Some(so_far) => self.types.join(so_far, field).0,
                None => field,
            });
        }
        joined
    }
}
