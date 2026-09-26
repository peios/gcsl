//! The types of GCSL expressions.

use std::fmt;

/// The type of an expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Signed 64-bit integer.
    Int,
    /// IEEE 754 binary64 floating point.
    Float,
    /// The type of an integer literal whose type has not been settled by its
    /// context yet. Checker-internal: it never survives checking, and it
    /// displays as `Int` so users never see it.
    UntypedInt,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Ty::Int | Ty::UntypedInt => "Int",
            Ty::Float => "Float",
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
        assert_eq!(Ty::UntypedInt.to_string(), "Int");
    }
}
