//! Runtime values.

use std::fmt;
use std::rc::Rc;

use crate::strings::Quoted;
use crate::types::Ty;

/// The result of evaluating an expression.
///
/// Strings have value semantics but are shared: cloning a `Value::Str`
/// increments a reference count and never copies the text.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An `Int`.
    Int(i64),
    /// A `Float`.
    Float(f64),
    /// A `Bool`.
    Bool(bool),
    /// A `String`.
    Str(Rc<str>),
    /// `()`, the only `Unit` value.
    Unit,
}

impl Value {
    /// The type of the value.
    #[must_use]
    pub const fn ty(&self) -> Ty {
        match self {
            Value::Int(_) => Ty::Int,
            Value::Float(_) => Ty::Float,
            Value::Bool(_) => Ty::Bool,
            Value::Str(_) => Ty::String,
            Value::Unit => Ty::Unit,
        }
    }

    /// The value as it would be written in a program, for the REPL: a
    /// string double-quoted with its special characters escaped (see
    /// [`Quoted`]), and every other value as its `Display` form.
    #[must_use]
    pub const fn repr(&self) -> Repr<'_> {
        Repr(self)
    }
}

/// Shows the value only, never its type.
///
/// A `Float` always shows a decimal point or an exponent, so it cannot be
/// mistaken for an `Int`: `3.0`, `1.5`, `1e300`, `1e-7`. It uses the
/// shortest text that reads back as the same value. The non-finite values
/// show as `inf`, `-inf` and `nan`. A `Bool` shows as `true` or `false`, a
/// `String` as its text with no quotes or escaping, and `Unit` as `()`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(value) => write!(f, "{value}"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Str(text) => f.write_str(text),
            Value::Unit => f.write_str("()"),
            // `{:?}` would show NaN as `NaN`.
            Value::Float(value) if value.is_nan() => f.write_str("nan"),
            // `{:?}` on f64 is the shortest round-trip representation and
            // always includes `.` or `e`; it shows infinities as `inf`/`-inf`.
            Value::Float(value) => write!(f, "{value:?}"),
        }
    }
}

/// A [`Value`] shown as it would be written in a program. Returned by
/// [`Value::repr`].
#[derive(Clone, Copy, Debug)]
pub struct Repr<'a>(&'a Value);

impl fmt::Display for Repr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Str(text) => Quoted(text).fmt(f),
            value => value.fmt(f),
        }
    }
}

/// Converts an `Int` to the nearest `Float` (ties to even), the meaning of
/// both an integer literal in `Float` context and an `IntToFloat` node.
#[expect(
    clippy::cast_precision_loss,
    reason = "rounding to the nearest f64 is the defined meaning of Int-to-Float conversion"
)]
pub(crate) const fn int_to_float(value: i64) -> f64 {
    value as f64
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests {
    use super::*;

    fn show(value: f64) -> String {
        Value::Float(value).to_string()
    }

    fn string(text: &str) -> Value {
        Value::Str(Rc::from(text))
    }

    #[test]
    fn values_are_small() {
        // A string is a fat `Rc<str>` pointer, 16 bytes, plus the tag. A
        // thin string pointer would bring this back to 16 bytes; that is a
        // later optimisation for the VM's value representation, not this
        // tree walker's.
        assert!(size_of::<Value>() <= 24);
    }

    #[test]
    fn ints_display_as_decimal() {
        assert_eq!(Value::Int(0).to_string(), "0");
        assert_eq!(Value::Int(-42).to_string(), "-42");
        assert_eq!(Value::Int(i64::MIN).to_string(), "-9223372036854775808");
    }

    #[test]
    fn floats_always_show_their_type() {
        assert_eq!(show(3.0), "3.0");
        assert_eq!(show(1.5), "1.5");
        assert_eq!(show(-0.0), "-0.0");
        assert_eq!(show(0.1 + 0.2), "0.30000000000000004");
    }

    #[test]
    fn non_finite_floats() {
        assert_eq!(show(f64::INFINITY), "inf");
        assert_eq!(show(f64::NEG_INFINITY), "-inf");
        assert_eq!(show(f64::NAN), "nan");
        assert_eq!(show(-f64::NAN), "nan");
    }

    #[test]
    fn large_and_small_floats() {
        assert_eq!(show(1e300), "1e300");
        assert_eq!(show(1e16), "1e16");
        assert_eq!(show(1e15), "1000000000000000.0");
        assert_eq!(show(1.5e-7), "1.5e-7");
        assert_eq!(show(0.0001), "0.0001");
        assert_eq!(show(f64::MAX), "1.7976931348623157e308");
        assert_eq!(show(f64::MIN_POSITIVE), "2.2250738585072014e-308");
        assert_eq!(show(5e-324), "5e-324");
    }

    #[test]
    fn types() {
        assert_eq!(Value::Int(1).ty(), Ty::Int);
        assert_eq!(Value::Float(1.0).ty(), Ty::Float);
        assert_eq!(Value::Bool(true).ty(), Ty::Bool);
        assert_eq!(string("a").ty(), Ty::String);
        assert_eq!(Value::Unit.ty(), Ty::Unit);
    }

    #[test]
    fn unit_displays_as_empty_parentheses() {
        assert_eq!(Value::Unit.to_string(), "()");
    }

    #[test]
    fn bools_and_strings_display_plainly() {
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Bool(false).to_string(), "false");
        // A string's text, unquoted and unescaped.
        assert_eq!(string("say \"hi\"\n\t{x}").to_string(), "say \"hi\"\n\t{x}");
        assert_eq!(string("").to_string(), "");
    }

    #[test]
    fn repr_quotes_strings_and_shows_everything_else_as_display() {
        assert_eq!(string("hello").repr().to_string(), "\"hello\"");
        assert_eq!(string("").repr().to_string(), "\"\"");
        assert_eq!(
            string("a\nb\tc\rd\\e\"f").repr().to_string(),
            "\"a\\nb\\tc\\rd\\\\e\\\"f\""
        );
        // Braces are doubled so the literal reads back; other characters
        // are verbatim.
        assert_eq!(string("{é}").repr().to_string(), "\"{{é}}\"");
        assert_eq!(Value::Int(-3).repr().to_string(), "-3");
        assert_eq!(Value::Float(2.0).repr().to_string(), "2.0");
        assert_eq!(Value::Float(f64::NAN).repr().to_string(), "nan");
        assert_eq!(Value::Bool(false).repr().to_string(), "false");
        assert_eq!(Value::Unit.repr().to_string(), "()");
    }

    #[test]
    fn cloning_a_string_shares_its_text() {
        let original = string("shared");
        let copy = original.clone();
        let (Value::Str(a), Value::Str(b)) = (&original, &copy) else {
            panic!("expected strings");
        };
        assert!(Rc::ptr_eq(a, b));
        assert_eq!(original, copy);
        // Equality compares content, not identity.
        assert_eq!(string("x"), string("x"));
        assert_ne!(string("x"), string("y"));
    }

    #[test]
    fn int_to_float_rounds_to_nearest() {
        assert_eq!(int_to_float(3), 3.0);
        assert_eq!(int_to_float(9_007_199_254_740_993), 9_007_199_254_740_992.0);
        assert_eq!(int_to_float(i64::MIN), -9_223_372_036_854_775_808.0);
    }
}
