//! The evaluator: a recursive walk over [`Ir`].
//!
//! There is one walker per result type, [`Evaluator::int`] and
//! [`Evaluator::float`], each returning an unboxed `i64` or `f64`. Every IR
//! node states its type, so the walker for a node is always known from its
//! parent and no runtime value is ever inspected to choose an operation. A
//! [`Value`] is only built once, at the root.
//!
//! Operands are evaluated left to right, so when both would fail, the left
//! one's error is reported. Evaluation holds no per-node state and allocates
//! nothing, except the message of the diagnostic when it fails.

use std::num::NonZeroI64;

use crate::diagnostic::Diagnostic;
use crate::ir::{Ir, IrId, IrKind};
use crate::source::Span;
use crate::types::Ty;
use crate::value::{Value, int_to_float};

/// Evaluates `ir` to a value.
///
/// # Errors
///
/// Returns a runtime error at the failing operator's span: "integer
/// overflow" for `Int` arithmetic that leaves the range of `i64`, and
/// "division by zero" for `//` or `%` by zero. `Float` arithmetic never
/// fails.
pub fn eval(ir: &Ir) -> Result<Value, Diagnostic> {
    let evaluator = Evaluator { ir };
    let root = ir.get(ir.root());
    match root.kind.ty() {
        Ty::Int => evaluator.int(ir.root()).map(Value::Int),
        Ty::Float => evaluator.float(ir.root()).map(Value::Float),
        Ty::UntypedInt | Ty::Error => Err(internal("IR node has no concrete type", root.span)),
    }
}

struct Evaluator<'a> {
    ir: &'a Ir,
}

impl Evaluator<'_> {
    /// Evaluates an `Int`-typed node.
    fn int(&self, id: IrId) -> Result<i64, Diagnostic> {
        let node = self.ir.get(id);
        let span = node.span;
        match node.kind {
            IrKind::IntConst(value) => Ok(value),
            IrKind::IntNeg(operand) => self
                .int(operand)?
                .checked_neg()
                .ok_or_else(|| overflow(span)),
            IrKind::IntAdd(lhs, rhs) => {
                let (lhs, rhs) = (self.int(lhs)?, self.int(rhs)?);
                lhs.checked_add(rhs).ok_or_else(|| overflow(span))
            }
            IrKind::IntSub(lhs, rhs) => {
                let (lhs, rhs) = (self.int(lhs)?, self.int(rhs)?);
                lhs.checked_sub(rhs).ok_or_else(|| overflow(span))
            }
            IrKind::IntMul(lhs, rhs) => {
                let (lhs, rhs) = (self.int(lhs)?, self.int(rhs)?);
                lhs.checked_mul(rhs).ok_or_else(|| overflow(span))
            }
            IrKind::IntFloorDiv(lhs, rhs) => {
                let (lhs, rhs) = (self.int(lhs)?, self.int(rhs)?);
                let rhs = NonZeroI64::new(rhs).ok_or_else(|| division_by_zero(span))?;
                floor_div(lhs, rhs).ok_or_else(|| overflow(span))
            }
            IrKind::IntRem(lhs, rhs) => {
                let (lhs, rhs) = (self.int(lhs)?, self.int(rhs)?);
                let rhs = NonZeroI64::new(rhs).ok_or_else(|| division_by_zero(span))?;
                Ok(floor_mod(lhs, rhs))
            }
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(..)
            | IrKind::FloatSub(..)
            | IrKind::FloatMul(..)
            | IrKind::FloatDiv(..)
            | IrKind::IntToFloat(_) => Err(internal("Float node where an Int was expected", span)),
        }
    }

    /// Evaluates a `Float`-typed node. IEEE 754 semantics; never fails except
    /// through an `Int` operand of `IntToFloat`.
    fn float(&self, id: IrId) -> Result<f64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::FloatConst(value) => Ok(value),
            IrKind::FloatNeg(operand) => Ok(-self.float(operand)?),
            IrKind::FloatAdd(lhs, rhs) => Ok(self.float(lhs)? + self.float(rhs)?),
            IrKind::FloatSub(lhs, rhs) => Ok(self.float(lhs)? - self.float(rhs)?),
            IrKind::FloatMul(lhs, rhs) => Ok(self.float(lhs)? * self.float(rhs)?),
            IrKind::FloatDiv(lhs, rhs) => Ok(self.float(lhs)? / self.float(rhs)?),
            IrKind::IntToFloat(operand) => Ok(int_to_float(self.int(operand)?)),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(..)
            | IrKind::IntSub(..)
            | IrKind::IntMul(..)
            | IrKind::IntFloorDiv(..)
            | IrKind::IntRem(..) => Err(internal("Int node where a Float was expected", node.span)),
        }
    }
}

/// Floor division: the quotient rounded toward negative infinity. Returns
/// `None` on overflow, which only `i64::MIN // -1` causes.
///
/// Derivation. Rust's `/` truncates toward zero and `%` takes the sign of
/// the dividend, so `a == b * q + r` with `q = a / b`, `r = a % b` and
/// `|r| < |b|`. If `r == 0` the division is exact and `q` is already the
/// floor. Otherwise the exact quotient `a / b` lies strictly between `q` and
/// the next integer away from zero, and it is negative exactly when `r` and
/// `b` have opposite signs. In that case truncation rounded *up*, so the
/// floor is `q - 1`. When the signs agree the exact quotient is positive,
/// truncation rounded down, and `q` is the floor.
///
/// `q - 1` cannot overflow: it only happens when the exact quotient is
/// negative and not whole, so `q <= 0` and `q > i64::MIN`.
///
/// This is not `i64::div_euclid`, which rounds so that the remainder is
/// never negative, and so differs from floor division when `b < 0`
/// (`7.div_euclid(-2) == -3`, but `7 // -2 == -4`).
fn floor_div(a: i64, b: NonZeroI64) -> Option<i64> {
    let b = b.get();
    let q = a.checked_div(b)?;
    // Cannot wrap: `checked_div` has already rejected `i64::MIN / -1`.
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        Some(q - 1)
    } else {
        Some(q)
    }
}

/// The remainder matching [`floor_div`]: `a - b * floor_div(a, b)`, which
/// has the sign of the divisor `b` (or is zero).
///
/// Derivation. With `q`, `r` the truncating quotient and remainder as in
/// [`floor_div`], the floored quotient is `q - 1` exactly when `r != 0` and
/// `r`, `b` have opposite signs. Then the floored remainder is
/// `a - b * (q - 1) = (a - b * q) + b = r + b`, which has the sign of `b`
/// because `|r| < |b|`. Otherwise it is `r`, which is zero or already has
/// the sign of `b`. `r + b` cannot overflow, since `r` and `b` have opposite
/// signs.
///
/// Never overflows: `i64::MIN % -1` is mathematically `0`, which is what
/// `wrapping_rem` returns for it.
fn floor_mod(a: i64, b: NonZeroI64) -> i64 {
    let b = b.get();
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    }
}

fn overflow(span: Span) -> Diagnostic {
    Diagnostic::error("integer overflow", span)
}

fn division_by_zero(span: Span) -> Diagnostic {
    Diagnostic::error("division by zero", span)
}

fn internal(message: &'static str, span: Span) -> Diagnostic {
    crate::diagnostic::InternalError { message, span }.into()
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::lower::lower;
    use crate::parser::parse;
    use crate::source::SourceFile;

    fn run(text: &str) -> Result<Value, Diagnostic> {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let ast = parse(&tokenize(&source).unwrap(), &source).unwrap();
        let types = check(&ast).unwrap();
        eval(&lower(&ast, &types).unwrap())
    }

    fn int(text: &str) -> i64 {
        match run(text).unwrap() {
            Value::Int(value) => value,
            other @ Value::Float(_) => panic!("{text}: expected an Int, got {other:?}"),
        }
    }

    fn float(text: &str) -> f64 {
        match run(text).unwrap() {
            Value::Float(value) => value,
            other @ Value::Int(_) => panic!("{text}: expected a Float, got {other:?}"),
        }
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = run(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
    }

    /// `i64::MIN`, which cannot be written as a literal.
    const MIN: &str = "(-9223372036854775807 - 1)";
    const MAX: &str = "9223372036854775807";

    fn nz(value: i64) -> NonZeroI64 {
        NonZeroI64::new(value).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(int("1 + 2 * 3"), 7);
        assert_eq!(int("8 - 3 - 2"), 3);
        assert_eq!(int("2 * 3 // 4"), 1);
        assert_eq!(int("-2 * 3"), -6);
        assert_eq!(int("--2"), 2);
        assert_eq!(int("2 * -3"), -6);
        assert_eq!(float("7 / 2"), 3.5);
        assert_eq!(float("7 / 2.0"), 3.5);
        assert_eq!(float("(1 + 2) * 3.0"), 9.0);
        assert_eq!(float("1.5 - 2"), -0.5);
        assert_eq!(float("-1.5"), -1.5);
        assert_eq!(float("(7 // 2) / 2"), 1.5);
    }

    #[test]
    fn floor_division_and_modulo_in_all_sign_combinations() {
        assert_eq!((int("7 // 2"), int("7 % 2")), (3, 1));
        assert_eq!((int("-7 // 2"), int("-7 % 2")), (-4, 1));
        assert_eq!((int("7 // -2"), int("7 % -2")), (-4, -1));
        assert_eq!((int("-7 // -2"), int("-7 % -2")), (3, -1));
    }

    #[test]
    fn exact_division_needs_no_adjustment() {
        assert_eq!((int("6 // 2"), int("6 % 2")), (3, 0));
        assert_eq!((int("-6 // 2"), int("-6 % 2")), (-3, 0));
        assert_eq!((int("6 // -2"), int("6 % -2")), (-3, 0));
        assert_eq!((int("-6 // -2"), int("-6 % -2")), (3, 0));
        assert_eq!((int("0 // -5"), int("0 % -5")), (0, 0));
    }

    #[test]
    fn floor_division_identity_holds() {
        for a in [-9, -7, -1, 0, 1, 7, 9, i64::MIN, i64::MAX] {
            for b in [-7, -2, -1, 1, 2, 7, i64::MIN, i64::MAX] {
                let Some(q) = floor_div(a, nz(b)) else {
                    assert_eq!((a, b), (i64::MIN, -1));
                    continue;
                };
                let r = floor_mod(a, nz(b));
                assert_eq!(i128::from(b) * i128::from(q) + i128::from(r), i128::from(a));
                assert!(r == 0 || (r < 0) == (b < 0), "{a} % {b} = {r}");
                assert!(r.unsigned_abs() < b.unsigned_abs());
            }
        }
    }

    #[test]
    fn differs_from_euclidean_division() {
        assert_eq!(7_i64.div_euclid(-2), -3);
        assert_eq!(floor_div(7, nz(-2)), Some(-4));
        assert_eq!(7_i64.rem_euclid(-2), 1);
        assert_eq!(floor_mod(7, nz(-2)), -1);
    }

    #[test]
    fn overflow_on_each_operator() {
        let overflow = "integer overflow".to_owned();
        assert_eq!(
            error(&format!("{MAX} + 1")),
            (overflow.clone(), Span::new(0, 23))
        );
        assert_eq!(error(&format!("{MIN} - 1")).0, overflow);
        assert_eq!(error(&format!("{MAX} * 2")).0, overflow);
        assert_eq!(
            error(&format!("-{MIN}")),
            (overflow.clone(), Span::new(0, 27))
        );
        assert_eq!(error(&format!("{MIN} // -1")).0, overflow);
        assert_eq!(error(&format!("{MIN} * -1")).0, overflow);
    }

    #[test]
    fn i64_min_cases() {
        assert_eq!(int(MIN), i64::MIN);
        assert_eq!(int(&format!("{MIN} % -1")), 0);
        assert_eq!(int(&format!("{MIN} // 1")), i64::MIN);
        assert_eq!(int(&format!("{MIN} // 2")), i64::MIN / 2);
        assert_eq!(int(&format!("{MIN} // {MAX}")), -2);
        assert_eq!(int(&format!("{MIN} % {MAX}")), i64::MAX - 1);
        assert_eq!(int(&format!("{MIN} % 2")), 0);
        assert_eq!(int(&format!("-({MIN} + 1)")), i64::MAX);
        assert_eq!(float(&format!("{MIN} / -1")), 9_223_372_036_854_775_808.0);
    }

    #[test]
    fn division_by_zero() {
        let message = "division by zero".to_owned();
        assert_eq!(error("7 // 0"), (message.clone(), Span::new(0, 6)));
        assert_eq!(error("7 % 0"), (message.clone(), Span::new(0, 5)));
        assert_eq!(error("1 + 7 % (2 - 2)"), (message, Span::new(4, 15)));
    }

    #[test]
    fn float_division_by_zero_is_ieee() {
        assert_eq!(float("1.0 / 0.0"), f64::INFINITY);
        assert_eq!(float("-1.0 / 0.0"), f64::NEG_INFINITY);
        assert!(float("0.0 / 0.0").is_nan());
        assert_eq!(float("1 / 0"), f64::INFINITY);
        assert!(float("1e308 * 10.0").is_infinite());
        assert!(float("(1.0 / 0.0) - (1.0 / 0.0)").is_nan());
    }

    #[test]
    fn errors_are_reported_left_to_right() {
        assert_eq!(
            error("(1 // 0) + (9223372036854775807 + 1)"),
            ("division by zero".to_owned(), Span::new(1, 7))
        );
        assert_eq!(
            error("(9223372036854775807 + 1) + (1 // 0)"),
            ("integer overflow".to_owned(), Span::new(1, 24))
        );
    }

    #[test]
    fn root_value_type_follows_the_ir() {
        assert_eq!(run("1").unwrap().ty(), Ty::Int);
        assert_eq!(run("1 / 1").unwrap().ty(), Ty::Float);
    }
}
