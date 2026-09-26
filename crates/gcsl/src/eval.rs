//! The evaluator: a recursive walk over [`Ir`].
//!
//! There is one walker per numeric result type, [`Evaluator::int`] and
//! [`Evaluator::float`], each returning an unboxed `i64` or `f64`. Every IR
//! node states its type, so the walker for a node is always known from its
//! parent and no runtime value is ever inspected to choose an operation.
//! A general walker, `Evaluator::value`, handles the nodes that are not
//! arithmetic (units, loads, stores and blocks, including the root) and
//! hands arithmetic to the typed walkers, building a [`Value`] only where a
//! value of statically unknown type is needed: at the root, in a slot, or
//! as a block's result evaluated through `value`.
//!
//! Variables live in a frame: one [`Value`] per slot, allocated once before
//! evaluation starts, sized by the checker's slot assignment. A `Load` in
//! an `Int` or `Float` context reads its slot and extracts the number; a
//! slot holding a value of another type there is an internal error, since
//! lowering typed the load. Storing values in separate typed slots
//! (`Vec<i64>`, `Vec<f64>`) would avoid the tag check; that is a later
//! optimisation for a bytecode VM, not something this tree walker needs.
//!
//! Operands are evaluated left to right, so when both would fail, the left
//! one's error is reported. Items are evaluated in order. Evaluation holds
//! no per-node state and allocates nothing beyond the frame, except the
//! diagnostic when it fails. A runtime error points at the operator and
//! labels each operand with the value it had, so the user sees what
//! actually overflowed or divided by zero.

use std::num::NonZeroI64;

use crate::check::Slot;
use crate::diagnostic::Diagnostic;
use crate::ir::{Ir, IrId, IrKind, IrNode, IrRange, Operands};
use crate::source::Span;
use crate::value::{Value, int_to_float};

/// Evaluates `ir` to a value, in a fresh frame.
///
/// # Errors
///
/// Returns a runtime error at the failing operator's span: "integer
/// overflow" for `Int` arithmetic that leaves the range of `i64`, and
/// "division by zero" for `//` or `%` by zero. `Float` arithmetic never
/// fails.
pub fn eval(ir: &Ir) -> Result<Value, Diagnostic> {
    // The one allocation of an evaluation.
    let mut frame = vec![Value::Unit; ir.frame_size() as usize];
    eval_in(ir, &mut frame)
}

/// Evaluates `ir` to a value in an existing `frame`, which must have at
/// least [`Ir::frame_size`] slots. Slots the program does not write keep
/// their values, which is how a [`crate::session::Session`] keeps its
/// variables from one input to the next.
///
/// # Errors
///
/// As [`eval`]; and an internal error if `frame` is too small.
pub fn eval_in(ir: &Ir, frame: &mut [Value]) -> Result<Value, Diagnostic> {
    let root = ir.root();
    if frame.len() < ir.frame_size() as usize {
        return Err(internal(
            "frame is smaller than the IR needs",
            ir.get(root).span,
        ));
    }
    Evaluator { ir, frame }.value(root)
}

struct Evaluator<'a> {
    ir: &'a Ir,
    frame: &'a mut [Value],
}

impl Evaluator<'_> {
    /// Evaluates a node of any type.
    fn value(&mut self, id: IrId) -> Result<Value, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Unit => Ok(Value::Unit),
            IrKind::Load(slot, _) => self.load(slot, node),
            IrKind::Store(slot, value) => {
                let value = self.value(value)?;
                let target = self
                    .frame
                    .get_mut(slot.index())
                    .ok_or_else(|| internal("store to a slot outside the frame", node.span))?;
                *target = value;
                Ok(Value::Unit)
            }
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.value(result)
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => self.int(id).map(Value::Int),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => self.float(id).map(Value::Float),
        }
    }

    /// Evaluates a block's items, in order, for their effects.
    fn items(&mut self, items: IrRange) -> Result<(), Diagnostic> {
        for &item in self.ir.items(items) {
            self.value(item)?;
        }
        Ok(())
    }

    /// Reads the value in `slot`.
    fn load(&self, slot: Slot, node: &IrNode) -> Result<Value, Diagnostic> {
        self.frame
            .get(slot.index())
            .copied()
            .ok_or_else(|| internal("load from a slot outside the frame", node.span))
    }

    /// Evaluates an `Int`-typed node.
    fn int(&mut self, id: IrId) -> Result<i64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::IntConst(value) => Ok(value),
            IrKind::Load(slot, _) => match self.load(slot, node)? {
                Value::Int(value) => Ok(value),
                Value::Float(_) | Value::Unit => Err(internal(
                    "Int load from a slot holding another type",
                    node.span,
                )),
            },
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.int(result)
            }
            IrKind::IntNeg(operand) => {
                let value = self.int(operand)?;
                value
                    .checked_neg()
                    .ok_or_else(|| self.unary_error(node, operand, OVERFLOW, value))
            }
            IrKind::IntAdd(operands) => {
                let (lhs, rhs) = self.ints(operands)?;
                lhs.checked_add(rhs)
                    .ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
            }
            IrKind::IntSub(operands) => {
                let (lhs, rhs) = self.ints(operands)?;
                lhs.checked_sub(rhs)
                    .ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
            }
            IrKind::IntMul(operands) => {
                let (lhs, rhs) = self.ints(operands)?;
                lhs.checked_mul(rhs)
                    .ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
            }
            IrKind::IntFloorDiv(operands) => {
                let (lhs, rhs) = self.ints(operands)?;
                let divisor = NonZeroI64::new(rhs)
                    .ok_or_else(|| self.binary_error(node, operands, DIVISION_BY_ZERO, lhs, rhs))?;
                floor_div(lhs, divisor)
                    .ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
            }
            IrKind::IntRem(operands) => {
                let (lhs, rhs) = self.ints(operands)?;
                let divisor = NonZeroI64::new(rhs)
                    .ok_or_else(|| self.binary_error(node, operands, DIVISION_BY_ZERO, lhs, rhs))?;
                Ok(floor_mod(lhs, divisor))
            }
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => {
                Err(internal("Float node where an Int was expected", node.span))
            }
            IrKind::Unit | IrKind::Store(..) => {
                Err(internal("Unit node where an Int was expected", node.span))
            }
        }
    }

    /// Evaluates both `Int` operands, left to right.
    fn ints(&mut self, operands: Operands) -> Result<(i64, i64), Diagnostic> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        Ok((lhs, rhs))
    }

    /// Evaluates a `Float`-typed node. IEEE 754 semantics; never fails except
    /// through an `Int` operand of `IntToFloat` or an item of a block.
    fn float(&mut self, id: IrId) -> Result<f64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::FloatConst(value) => Ok(value),
            IrKind::Load(slot, _) => match self.load(slot, node)? {
                Value::Float(value) => Ok(value),
                Value::Int(_) | Value::Unit => Err(internal(
                    "Float load from a slot holding another type",
                    node.span,
                )),
            },
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.float(result)
            }
            IrKind::FloatNeg(operand) => Ok(-self.float(operand)?),
            IrKind::FloatAdd(o) => Ok(self.float(o.lhs)? + self.float(o.rhs)?),
            IrKind::FloatSub(o) => Ok(self.float(o.lhs)? - self.float(o.rhs)?),
            IrKind::FloatMul(o) => Ok(self.float(o.lhs)? * self.float(o.rhs)?),
            IrKind::FloatDiv(o) => Ok(self.float(o.lhs)? / self.float(o.rhs)?),
            IrKind::IntToFloat(operand) => Ok(int_to_float(self.int(operand)?)),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => Err(internal("Int node where a Float was expected", node.span)),
            IrKind::Unit | IrKind::Store(..) => {
                Err(internal("Unit node where a Float was expected", node.span))
            }
        }
    }

    /// A runtime error in a binary operator: the message points at the
    /// operator, and each operand is labelled with its value.
    fn binary_error(
        &self,
        node: &IrNode,
        operands: Operands,
        message: &'static str,
        lhs: i64,
        rhs: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operands.lhs).span, lhs.to_string())
            .with_context(self.ir.get(operands.rhs).span, rhs.to_string())
    }

    /// A runtime error in a unary operator, labelling the operand with its
    /// value.
    fn unary_error(
        &self,
        node: &IrNode,
        operand: IrId,
        message: &'static str,
        value: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operand).span, value.to_string())
    }
}

const OVERFLOW: &str = "integer overflow";
const DIVISION_BY_ZERO: &str = "division by zero";

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
    use crate::types::Ty;

    fn compile(text: &str) -> Ir {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, &source).unwrap();
        let checked = check(&ast).unwrap();
        lower(&ast, &checked).unwrap()
    }

    fn run(text: &str) -> Result<Value, Diagnostic> {
        eval(&compile(text))
    }

    fn int(text: &str) -> i64 {
        match run(text).unwrap() {
            Value::Int(value) => value,
            other => panic!("{text}: expected an Int, got {other:?}"),
        }
    }

    fn float(text: &str) -> f64 {
        match run(text).unwrap() {
            Value::Float(value) => value,
            other => panic!("{text}: expected a Float, got {other:?}"),
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
        assert_eq!(int("-(-2)"), 2);
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
        // The error points at the operator.
        assert_eq!(
            error(&format!("{MAX} + 1")),
            (overflow.clone(), Span::new(20, 21))
        );
        assert_eq!(error(&format!("{MIN} - 1")).0, overflow);
        assert_eq!(error(&format!("{MAX} * 2")).0, overflow);
        assert_eq!(
            error(&format!("-{MIN}")),
            (overflow.clone(), Span::new(0, 1))
        );
        assert_eq!(error(&format!("{MIN} // -1")).0, overflow);
        assert_eq!(error(&format!("{MIN} * -1")).0, overflow);
    }

    #[test]
    fn runtime_errors_label_operands_with_their_values() {
        let diagnostic = run("2 * (9223372036854775807 - -1)").unwrap_err();
        assert_eq!(diagnostic.message, "integer overflow");
        assert_eq!(diagnostic.span, Span::new(25, 26));
        let labels: Vec<(Span, &str)> = diagnostic
            .labels
            .iter()
            .map(|label| (label.span, label.text.as_str()))
            .collect();
        assert_eq!(
            labels,
            vec![
                (Span::new(5, 24), "9223372036854775807"),
                (Span::new(27, 29), "-1")
            ]
        );
        let diagnostic = run("-(-9223372036854775807 - 1)").unwrap_err();
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].text, "-9223372036854775808");
        // The parenthesised operand is labelled without its parentheses.
        assert_eq!(diagnostic.labels[0].span, Span::new(2, 26));
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
        assert_eq!(error("7 // 0"), (message.clone(), Span::new(2, 4)));
        assert_eq!(error("7 % 0"), (message.clone(), Span::new(2, 3)));
        assert_eq!(error("1 + 7 % (2 - 2)"), (message, Span::new(6, 7)));
        let diagnostic = run("1 + 7 % (2 - 2)").unwrap_err();
        assert_eq!(diagnostic.labels[1].text, "0");
        assert_eq!(diagnostic.labels[1].span, Span::new(9, 14));
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
            ("division by zero".to_owned(), Span::new(3, 5))
        );
        assert_eq!(
            error("(9223372036854775807 + 1) + (1 // 0)"),
            ("integer overflow".to_owned(), Span::new(21, 22))
        );
    }

    #[test]
    fn root_value_type_follows_the_ir() {
        assert_eq!(run("1").unwrap().ty(), Ty::Int);
        assert_eq!(run("1 / 1").unwrap().ty(), Ty::Float);
        assert_eq!(run("").unwrap().ty(), Ty::Unit);
    }

    #[test]
    fn blocks_yield_their_result() {
        assert_eq!(int("let x = { let y = 1; y + 1 }\nx"), 2);
        assert_eq!(int("{ { { 3 } } }"), 3);
        assert_eq!(int("{ 1 } + { 2 } * 3"), 7);
        assert_eq!(int("{ = 4 }"), 4);
        assert_eq!(int("let x = 1\n= { let x = 5\n= x } - x"), 4);
        assert_eq!(float("{ 1.5 } * 2"), 3.0);
        assert_eq!(float("{ let h = 0.5\nh } / { 2 }"), 0.25);
    }

    #[test]
    fn shadowing() {
        assert_eq!(
            int("let x = 1\nlet x = x + 1\n{ let x = x * 10; x } + x"),
            22
        );
        // A shadowing `let` may change the type.
        assert_eq!(float("let x = 1\nlet x = 2.5\nx"), 2.5);
        // The outer variable is intact after an inner block shadows it.
        assert_eq!(int("let x = 1\n{ let x = 2 }\nx"), 1);
    }

    #[test]
    fn rebinding() {
        assert_eq!(int("let x = 1\nx := x + 1\nx := x * 3\nx"), 6);
        assert_eq!(float("let x = 1\nx := 2.5\nx"), 2.5);
        assert_eq!(int("{ let n = 1; n := n + 1; n }"), 2);
    }

    #[test]
    fn ascription() {
        assert_eq!(float("1 + 2 : Float"), 3.0);
        assert_eq!(float("-5 : Float"), -5.0);
        assert_eq!(float("let x: Float = 5\nx / 2"), 2.5);
        assert_eq!(float("let x = 5 : Float\nx * 2"), 10.0);
        assert_eq!(float("{ 5 } : Float"), 5.0);
        assert_eq!(int("let x = 3\nx : Int"), 3);
    }

    #[test]
    fn nested_blocks_reuse_slots() {
        let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
        assert_eq!(int(text), 114);
        // Each temporary shares its slot with the variable declared after
        // it; only the innermost `w` needs a slot of its own.
        assert_eq!(compile(text).frame_size(), 4);
    }

    #[test]
    fn unit_results() {
        for text in [
            "",
            "{}",
            "let x = 1",
            "{ let x = 1 }",
            "let u = {}\nu",
            "{ {}; {} }",
            "let x = 1\nx := 2",
            "= {}",
        ] {
            assert_eq!(run(text).unwrap(), Value::Unit, "{text}");
        }
    }

    #[test]
    fn items_run_in_order_and_their_errors_are_reported() {
        assert_eq!(
            error("let x = 1 // 0\n5"),
            ("division by zero".to_owned(), Span::new(10, 12))
        );
        assert_eq!(
            error("{ let y = 9223372036854775807 + 1 }\n1"),
            ("integer overflow".to_owned(), Span::new(30, 31))
        );
        // The first failing item wins.
        assert_eq!(
            error("let a = 1 // 0\nlet b = 9223372036854775807 + 1\nb").0,
            "division by zero"
        );
    }

    #[test]
    fn eval_in_uses_the_given_frame() {
        let ir = compile("let x = 7\nlet y = x + 1");
        let mut frame = vec![Value::Unit; 3];
        assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Unit);
        assert_eq!(frame, vec![Value::Int(7), Value::Int(8), Value::Unit]);
        let error = eval_in(&ir, &mut frame[..1]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: frame is smaller than the IR needs"
        );
    }

    #[test]
    fn a_load_of_the_wrong_type_is_an_internal_error() {
        // `x + 1` with `x` loaded as an Int, from a frame holding a Float.
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let load = builder.alloc(IrKind::Load(Slot::new(0), Ty::Int), Span::new(0, 1));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
        let add = builder.alloc(
            IrKind::IntAdd(Operands {
                lhs: load,
                rhs: one,
                op_start: 2,
            }),
            Span::new(0, 5),
        );
        let ir = builder.finish(add, 1, Vec::new());
        assert_eq!(eval_in(&ir, &mut [Value::Int(2)]).unwrap(), Value::Int(3));
        let error = eval_in(&ir, &mut [Value::Float(1.5)]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: Int load from a slot holding another type"
        );
        assert_eq!(error.span, Span::new(0, 1));
    }
}
