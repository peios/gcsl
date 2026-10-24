//! Checking enums and partial records: variants, their payloads, and
//! `Partial<T>`.
//!
//! # Enums
//!
//! An extensible enum, `enum Tool extensible<B>` (see the parser's
//! [Enums](crate::parser::enums#enums)), is a nominal type, [`TyKind::Enum`], by its
//! key, a name no source can write made of its name and of what declares it
//! ([`crate::names`], "Enum keys"), so that every scan of one file, and every
//! input of a session, has one enum. Its variants are declared anywhere that
//! sees it, `enum Tool::C<{cc: File}>`, each with its payload's type, `{}`
//! without one. Before any file is checked, once the `type` items are, the
//! enums' bounds are resolved in their files (`check_enum_bounds`), and then
//! the variants' payloads in theirs (`check_variant_payloads`), each recorded
//! in the type table by its key: a payload must fit its enum's bound, as the
//! payload is written, so that a record type's fields each fit the bound's
//! field of their name or its rest-field
//! ([`TypeTable::fits_bound`](crate::types::TypeTable::fits_bound)), "the
//! payload of `Tool::C` must fit `{*: File}`"; two variants of one enum and
//! name in one file are "variant `Tool::C` is declared twice", and a variant of
//! what is no enum "`Int` is not an enum". A variant in error is reported
//! there, once, and a use of it fails the check quietly.
//!
//! A variant written as a value, `Tool::C` (`Checker::check_variant`), is of
//! type `Variant<E, T>` ([`TyKind::Variant`]), its enum and its payload, of the
//! one variant of the name the file sees (see [`crate::workspace`], "Enums"):
//! none is "no variant `C` of `Tool`", noting those it sees, and several
//! "variant `Tool::C` is declared in two files". A variant is a subtype of its
//! function type, `fn(T): E`, so a call of it makes a value of the enum, and of
//! no other variant type but one of its enum and payload. It is recorded for
//! lowering, which makes it a constant tag
//! ([`Resolution::variant`](super::Resolution::variant)). A value of an enum is
//! of kind `Mixed`: present, of the kind only it says. `is E` tests a value's
//! variant's enum ([`ValueTest::Enum`](crate::types::ValueTest::Enum)). `==`
//! compares two values of an enum, whole, as `===` does, and two variants, but
//! for an enum with no bound, whose payloads may hold functions: "cannot
//! compare `Tool`: its variants' payloads are not bounded, so they may hold
//! functions".
//!
//! `Variant<E, T>` is written as a type of two arguments, `E` an enum or a
//! type parameter. A variant's payload is taken out by the prelude's
//! methods, `kind.matches(value)` and `kind.payload(value)`, which call the
//! generic built-ins `variant_matches` and `variant_payload`; so
//! `get<T>(kind: Variant<Tool, T>)` finds a value of a variant passed to it
//! and unwraps its `T`.
//!
//! # Partial records
//!
//! `Partial<T>`, of a record type `T` with no rest-field (or of a type
//! parameter a call binds to one), is [`TyKind::Partial`]: the records
//! holding any of `T`'s fields, each of its type. A record type, or a
//! partial record of another, fits it if each field of it `T` names fits
//! `T`'s; a partial record fits only a record type that requires no field.
//! A field of a partial record is not read by name: "field `arch` of
//! `Partial<Platform>` may be absent". The prelude's
//! `partial.matches(full)`, of the generic built-in `partial_matches`, is
//! whether every field the partial record holds equals the full record's of
//! its name.

use crate::ast::{ExprId, TypeArgRange, TypeExprId};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::types::{TyId, TyKind};

use super::{Bareness, Checker, VariantTag};

impl Checker<'_> {
    /// `Variant<E, T>`, written at `span` with the type arguments `args`
    /// (see [Enums](#enums)): the type of a variant of the enum `E` whose
    /// payload is a `T`, both resolved with every bare type present; `Error`
    /// (reported) for other than two arguments, "type `Variant` takes 2 type
    /// arguments, given 1", and for an `E` that is no enum nor a type
    /// parameter.
    pub(super) fn resolve_variant_type(&mut self, span: Span, args: TypeArgRange) -> TyId {
        let resolved: Vec<TyId> = self
            .ast
            .type_args(args)
            .iter()
            .map(|&arg| self.resolve_type(arg, Bareness::Present))
            .collect();
        let &[of, payload] = &resolved[..] else {
            let message = format!(
                "type `Variant` takes 2 type arguments, given {}",
                args.len()
            );
            self.errors.push(Diagnostic::error(message, span));
            return TyId::ERROR;
        };
        if of == TyId::ERROR || payload == TyId::ERROR {
            return TyId::ERROR;
        }
        // An enum, or a type parameter a call binds to one.
        if !matches!(
            self.types.kind(of),
            TyKind::Enum { .. } | TyKind::Param { .. }
        ) {
            let shown = self.in_message(of);
            let at = self
                .ast
                .type_args(args)
                .first()
                .map_or(span, |&first| self.ast.type_expr(first).span);
            self.errors.push(Diagnostic::error(
                format!("`Variant`'s first type argument must be an enum, not {shown}"),
                at,
            ));
            return TyId::ERROR;
        }
        self.types.variant(of, payload)
    }

    /// `Partial<T>` of the type written as `arg`, resolved with every bare
    /// type present (see [Partial records](#partial-records)): `Error`
    /// (reported) for a type that is no record type, or one with a
    /// rest-field.
    pub(super) fn resolve_partial(&mut self, arg: TypeExprId) -> TyId {
        let record = self.resolve_type(arg, Bareness::Present);
        if record == TyId::ERROR {
            return TyId::ERROR;
        }
        // A type parameter a call binds to a record type is one too.
        let param = matches!(self.types.kind(record), TyKind::Param { .. });
        if !param && (!self.types.is_record(record) || self.types.record_rest(record).is_some()) {
            let shown = self.in_message(record);
            self.errors.push(Diagnostic::error(
                format!("`Partial` takes a record type with no rest-field, not {shown}"),
                self.ast.type_expr(arg).span,
            ));
            return TyId::ERROR;
        }
        self.types.partial(record)
    }

    /// Types the variant as a value `id`, `of::name`, the name written at
    /// the end of it (see [Enums](#enums)): `Variant<E, T>` of the enum `of`
    /// resolves to and the payload of the one variant of the name the input
    /// sees, recorded for lowering; `Error` (reported) for an `of` that is no
    /// enum, "`Int` is not an enum", a name no variant the input sees has,
    /// "no variant `C` of `Toolchain`", noting those it sees, and one
    /// several files declare, "variant `Toolchain::C` is declared in two
    /// files", labelled at each.
    #[inline(never)]
    pub(super) fn check_variant(&mut self, id: ExprId, (of, name): (TypeExprId, Name)) -> TyId {
        let text = self.ast.names().get(name);
        let name_span = self.ast.written_name(self.ast.get(id).span.end, text);
        let enum_ty = self.resolve_type(of, Bareness::Present);
        if enum_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        let TyKind::Enum {
            key,
            name: enum_name,
        } = *self.types.kind(enum_ty)
        else {
            let shown = self.in_message(enum_ty);
            self.errors.push(Diagnostic::error(
                format!("{shown} is not an enum"),
                self.ast.type_expr(of).span,
            ));
            return TyId::ERROR;
        };
        let Some((payload, tag)) = self.find_variant((key, enum_name), name, name_span) else {
            return TyId::ERROR;
        };
        self.lowering.variants.insert(id, tag);
        self.types.variant(enum_ty, payload)
    }

    /// The payload and the tag of the variant `name` of the enum of key
    /// `key` and name `enum_name` that the input sees, the one there is (see
    /// [Enums](#enums)); `None` (reported at `name_span`) if it sees none,
    /// "no variant `C` of `Toolchain`", noting those it sees, or several,
    /// "variant `Toolchain::C` is declared in two files", labelled at each,
    /// and (quietly) if the variant is in error, which is reported where it
    /// is declared.
    pub(super) fn find_variant(
        &mut self,
        (key, enum_name): (Name, Name),
        name: Name,
        name_span: Span,
    ) -> Option<(TyId, VariantTag)> {
        let names = self.ast.names();
        let shown = format!("{}::{}", names.get(enum_name), names.get(name));
        let found = self
            .workspace
            .map_or(&[][..], |workspace| workspace.variants(key, name));
        match found {
            [] => {
                let seen = self
                    .workspace
                    .map(|workspace| workspace.variant_names(key))
                    .unwrap_or_default();
                let listed: Vec<String> = seen
                    .iter()
                    .map(|&variant| format!("`{}`", names.get(variant)))
                    .collect();
                let note = if listed.is_empty() {
                    format!("`{}` has no variant visible here", names.get(enum_name))
                } else {
                    format!(
                        "`{}` has the variants {} here",
                        names.get(enum_name),
                        listed.join(", ")
                    )
                };
                self.errors.push(
                    Diagnostic::error(
                        format!(
                            "no variant `{}` of `{}`",
                            names.get(name),
                            names.get(enum_name)
                        ),
                        name_span,
                    )
                    .with_note(note),
                );
                None
            }
            [one] => {
                let payload = self.types.variant_payload(one.key).unwrap_or(TyId::ERROR);
                // A variant in error is reported where it is declared: the
                // check fails quietly.
                if payload == TyId::ERROR {
                    self.quiet.failed = true;
                    return None;
                }
                let tag = VariantTag {
                    key: one.key,
                    of: key,
                    shown: shown.into(),
                };
                Some((payload, tag))
            }
            several => {
                let mut error = Diagnostic::error(
                    format!("variant `{shown}` is declared in two files"),
                    name_span,
                );
                for seen in several {
                    let site = seen
                        .declared
                        .and_then(|declared| self.workspace?.variant_site(declared));
                    if let Some((file, span)) = site {
                        error = error.with_context_in(file, span, "declared here");
                    }
                }
                self.errors.push(error);
                None
            }
        }
    }
}
