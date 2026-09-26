//! The types of GCSL expressions.

use std::fmt;

/// The type of an expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Signed 64-bit integer.
    Int,
    /// IEEE 754 binary64 floating point.
    Float,
    /// The type with one value, `()`: the value of a block with no result.
    Unit,
    /// The type of an integer literal whose type has not been settled by its
    /// context yet. Checker-internal: it never survives checking, and it
    /// displays as `Int` so users never see it.
    UntypedInt,
    /// The type of an expression that already has a reported type error.
    /// Checker-internal: every operator accepts it and produces it without
    /// reporting anything, so one mistake is reported once rather than at
    /// every enclosing operator. It never survives a successful check.
    Error,
}

impl Ty {
    /// Whether this is [`Ty::Error`].
    #[must_use]
    pub const fn is_error(self) -> bool {
        matches!(self, Ty::Error)
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Ty::Int | Ty::UntypedInt => "Int",
            Ty::Float => "Float",
            Ty::Unit => "Unit",
            Ty::Error => "<error>",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untyped_int_displays_as_int() {
        assert_eq!(Ty::Int.to_string(), "Int");
        assert_eq!(Ty::Float.to_string(), "Float");
        assert_eq!(Ty::Unit.to_string(), "Unit");
        assert_eq!(Ty::UntypedInt.to_string(), "Int");
        assert_eq!(Ty::Error.to_string(), "<error>");
        assert!(Ty::Error.is_error());
        assert!(!Ty::Int.is_error());
    }
}
