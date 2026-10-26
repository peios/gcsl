//! Tests of the evaluator: arithmetic, division and overflow, blocks, and items
//! run in order.

use super::*;

pub(super) fn boolean(text: &str) -> bool {
    match run(text).unwrap() {
        Value::Bool(value) => value,
        other => panic!("{text}: expected a Bool, got {other:?}"),
    }
}

pub(super) fn string(text: &str) -> String {
    match run(text) {
        Ok(Value::Str(value)) => value.to_string(),
        other => panic!("{text}: expected a String, got {other:?}"),
    }
}

/// Checks that each program evaluates to the Bool given.
pub(super) fn booleans(cases: &[(&str, bool)]) {
    for &(text, expected) in cases {
        assert_eq!(boolean(text), expected, "{text}");
    }
}

pub(super) fn run(text: &str) -> Result<Value, Diagnostic> {
    eval(&compile_prelude(text))
}

pub(super) fn int(text: &str) -> i64 {
    match run(text).unwrap() {
        Value::Int(value) => value,
        other => panic!("{text}: expected an Int, got {other:?}"),
    }
}

pub(super) fn float(text: &str) -> f64 {
    match run(text).unwrap() {
        Value::Float(value) => value,
        other => panic!("{text}: expected a Float, got {other:?}"),
    }
}

pub(super) fn error(text: &str) -> (String, Span) {
    let diagnostic = run(text).unwrap_err();
    (diagnostic.message, diagnostic.span)
}

/// `i64::MIN`, which cannot be written as a literal.
pub(super) const MIN: &str = "(-9223372036854775807 - 1)";

pub(super) const MAX: &str = "9223372036854775807";

pub(super) fn nz(value: i64) -> NonZeroI64 {
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
    assert_eq!(run("1").unwrap().kind(), ValueKind::Int);
    assert_eq!(run("1 / 1").unwrap().kind(), ValueKind::Float);
    assert_eq!(run("").unwrap().kind(), ValueKind::Record);
    assert_eq!(run("x = 1").unwrap().kind(), ValueKind::Record);
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
    // The literal adapts to the variable's type.
    assert_eq!(float("let x: Float = 1\nx := 2\nx"), 2.0);
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
fn empty_record_results() {
    for text in [
        "",
        "{}",
        "let x = 1",
        "{ let x = 1 }",
        "let u = {}\nu",
        "{ {}; {} }",
        "let x = 1\nx := 2",
        "= {}",
        "if true {}",
        "{ e = {} }.e",
    ] {
        assert_eq!(run(text).unwrap(), Value::Empty, "{text}");
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
    let mut frame = vec![Value::Empty; 3];
    assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
    // The stack is truncated to the frame on return.
    assert_eq!(frame, vec![Value::Int(7), Value::Int(8)]);
    frame.truncate(1);
    let error = eval_in(&ir, &mut frame).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: frame is smaller than the IR needs"
    );
}
