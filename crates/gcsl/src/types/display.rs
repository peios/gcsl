//! Showing a type as it is written in source ([`TypeTable::display`]).

use std::fmt;

use crate::check::Constant;
use crate::names::{Name, Names, key_context};
use crate::strings::{FieldName, Quoted};
use crate::value::write_quantity;

use super::{RecordTy, TyId, TyKind, TypeTable};

/// A type shown as it is written in source. Returned by
/// [`TypeTable::display`].
pub(super) struct Display<'a> {
    pub(super) table: &'a TypeTable,
    pub(super) names: &'a Names,
    pub(super) id: TyId,
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

    /// Writes the record type `record` (see [`TypeTable::display`]): its
    /// fields by name, each with its default if it has one, and its
    /// rest-field after them.
    fn record(&self, record: &RecordTy, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        // The rest-field after the named ones.
        if let Some(rest) = record.rest() {
            if !fields.is_empty() {
                f.write_str(", ")?;
            }
            write!(f, "*: {}", self.nested(rest))?;
        }
        f.write_str("}")
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

impl Display<'_> {
    /// Writes the type, an enum, by its name, a variant's, `Variant<E, T>`,
    /// or a partial record's, `Partial<T>`.
    fn enum_like(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.table.kind(self.id) {
            TyKind::Enum { name, .. } => f.write_str(self.names.get(*name)),
            TyKind::Variant { of, payload, .. } => {
                write!(
                    f,
                    "Variant<{}, {}>",
                    self.nested(*of),
                    self.nested(*payload)
                )
            }
            TyKind::Partial(record) => write!(f, "Partial<{}>", self.nested(*record)),
            TyKind::Secret(value, false) => write!(f, "Secret<{}>", self.nested(*value)),
            TyKind::Secret(value, true) => write!(f, "Revealed<{}>", self.nested(*value)),
            _ => Ok(()),
        }
    }
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.table.kind(self.id) {
            TyKind::Int | TyKind::UntypedInt => f.write_str("Int"),
            TyKind::Float => f.write_str("Float"),
            TyKind::Quantity(quantity) => f.write_str(quantity.type_name()),
            TyKind::Bool => f.write_str("Bool"),
            TyKind::String => f.write_str("String"),
            TyKind::File => f.write_str("File"),
            TyKind::Dir => f.write_str("Dir"),
            TyKind::Sandbox => f.write_str("Sandbox"),
            TyKind::Error => f.write_str("<error>"),
            TyKind::Never => f.write_str("Never"),
            &TyKind::Param { generic, index } => {
                let name = self.table.generic_names(generic).get(usize::from(index));
                f.write_str(name.map_or("?", |&name| self.names.get(name)))
            }
            // TEMPORARY (alias rule): the opaque type of a special built-in (see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
            TyKind::Builtin(builtin) => write!(f, "builtin {}", builtin.name()),
            TyKind::Future(inner) => write!(f, "Future<{}>", self.nested(*inner)),
            TyKind::List(element) => write!(f, "List<{}>", self.nested(*element)),
            TyKind::Enum { .. }
            | TyKind::Variant { .. }
            | TyKind::Partial(_)
            | TyKind::Secret(..) => self.enum_like(f),
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
            TyKind::Record(record) => self.record(record, f),
            TyKind::Fn(sig) => {
                f.write_str("fn")?;
                if let Some(generic) = sig.generic {
                    let names: Vec<&str> = self
                        .table
                        .generic_names(generic)
                        .iter()
                        .map(|&name| self.names.get(name))
                        .collect();
                    write!(f, "<{}>", names.join(", "))?;
                }
                f.write_str("(")?;
                for (index, &(name, ty)) in sig.params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    if sig.rest.map(usize::from) == Some(index) {
                        f.write_str("...")?;
                    }
                    if let Some(name) = name {
                        write!(f, "{}: ", FieldName(self.names.get(name)))?;
                    }
                    self.nested(ty).fmt(f)?;
                }
                // What it needs, after its parameters: `^arch`, or `^arch |
                // ^target` for a need read through a default.
                let mut needs: Vec<String> = self
                    .table
                    .need_set(sig.needs)
                    .iter()
                    .map(|need| {
                        let shown: Vec<String> = need
                            .iter()
                            .map(|&key| self.names.get(key))
                            .map(|key| format!("^{}", key_context(key).unwrap_or(key)))
                            .collect();
                        shown.join(" | ")
                    })
                    .collect();
                needs.sort_unstable();
                for (index, need) in needs.into_iter().enumerate() {
                    if index > 0 || !sig.params.is_empty() {
                        f.write_str(", ")?;
                    }
                    f.write_str(&need)?;
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
        &Constant::Quantity(quantity, amount) => write_quantity(f, quantity, amount),
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
