//! Tests of the checker: memoised functions.

use super::*;

#[test]
fn a_memoised_functions_bare_parameters_are_present() {
    assert_eq!(
        shown_result("fn f(x: Int) ->> Int { x }\nf"),
        "fn(x: Int) ->> Int"
    );
    // At every position inside, and whether the body measures them or
    // not: they are not inferred.
    assert_eq!(
        shown_result("fn g(p: {a: Int, b: {c: String}}) ->> Int { 1 }\ng"),
        "fn(p: {a: Int, b: {c: String}}) ->> Int"
    );
    // What is written explicitly means what it says.
    assert_eq!(
        shown_result(
            "fn h(x: Future<Int>, y: Int | Future<Int>, z: Present<Int>) ->> Int { 1 }\nh"
        ),
        "fn(x: Future<Int>, y: Int | Future<Int>, z: Int) ->> Int"
    );
    // A function type parameter follows its own marker, and a callback
    // parameter the body never calls is narrowed (see
    // `callback_parameters_are_narrowed`).
    assert_eq!(
        shown_result("fn k(f: fn(Int) -> Int) ->> Int { 1 }\nk"),
        "fn(f: fn(Int) -> Int | Future<Int>) ->> Int"
    );
    // Its own name in its body has the declared type.
    let text = "fn f(n: Int) ->> Int { if n < 2 { n } else { f(n - 1) + 1 } }\nf";
    assert_eq!(shown_result(text), "fn(n: Int) ->> Int");
}

#[test]
fn a_memoised_functions_bare_result_is_inferred() {
    assert_eq!(
        shown_result("fn l(x: Int) ->> Int { lazy x }\nl"),
        "fn(x: Int) ->> Future<Int>"
    );
    assert_eq!(
        shown_result("fn j(c: Bool) ->> Int { if c { 1 } else { lazy 2 } }\nj"),
        "fn(c: Bool) ->> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn r() ->> {a: Int, b: Int} { a = 1; b = lazy 2 }\nr"),
        "fn() ->> {a: Int, b: Future<Int>}"
    );
    // A call's result is the inferred one.
    assert_eq!(
        shown_result("fn l(x: Int) ->> Int { lazy x }\nl(1)"),
        "Future<Int>"
    );
    // A future passed through, as written.
    assert_eq!(
        shown_result("fn p(x: Future<Int>) ->> Int { x }\np"),
        "fn(x: Future<Int>) ->> Future<Int>"
    );
}

#[test]
fn a_written_memoised_function_type() {
    // Parameters present, result the either shape.
    assert_eq!(
        shown_result("let f: fn(Int, {a: Int}) ->> Int = fn(x: Int, r: {a: Int}) ->> Int { x }\nf"),
        "fn(Int, {a: Int}) ->> Int | Future<Int>"
    );
    // Nested in another function type, with its own rule.
    assert_eq!(
        shown_result(
            "let f: fn(fn(Int) ->> Int) -> Int = fn(g: fn(Int) ->> Int) -> Int { g(1) }\nf"
        ),
        "fn(fn(Int) ->> Int | Future<Int>) -> Int | Future<Int>"
    );
    // A memoised function stands in for an impure one whose caller
    // passes present arguments, and a pure one for a memoised one.
    let memo = "let m = fn(x: Int) ->> Int { x }\n";
    assert!(diagnostics_of(&format!("{memo}let g: fn(Present<Int>) -> Int = m")).is_empty());
    assert!(diagnostics_of("let g: fn(Int) ->> Int = fn(x: Int): Int { x }").is_empty());
    // Not for one whose caller may pass a future, nor for a pure one.
    for target in ["fn(Int) -> Int", "fn(Present<Int>): Int"] {
        assert_eq!(
            messages(&format!("{memo}let g: {target} = m")),
            vec!["mismatched types"],
            "{target}"
        );
    }
}

/// The diagnostics of `text`, none if it checks.
pub(super) fn diagnostics_of(text: &str) -> Vec<Diagnostic> {
    check(&parse_text(text), &mut TypeTable::new())
        .err()
        .unwrap_or_default()
}

#[test]
fn an_argument_that_may_be_a_future_is_awaited_at_a_memoised_call() {
    let f = "fn f(x: Int) ->> Int { x }\n";
    assert_eq!(
        marks(&format!("{f}let v = lazy 1\nf(v)")),
        vec![("v", AwaitMark::Top)]
    );
    // Deeply, where the parameter's type needs it.
    let g = "fn g(p: {a: Int}) ->> Int { p.a }\n";
    assert_eq!(
        marks(&format!("{g}let r = {{ a = lazy 1 }}\ng(r)")),
        vec![("r", AwaitMark::Deep)]
    );
    // Not where the parameter is written lazy.
    assert!(marks("fn h(x: Future<Int>) ->> Int { 1 }\nh(lazy 1)").is_empty());
}

#[test]
fn a_pure_function_cannot_call_a_memoised_one() {
    let text = "fn m(x: Int) ->> Int { x }\nfn p(): Int { m(1) }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(
        labels(&found[0]),
        vec![
            (last(text, "m"), "this is fn(x: Int) ->> Int"),
            (last(text, ":"), "this function is pure, declared with `:`")
        ]
    );
    // Nor can it await, whatever it calls.
    assert_eq!(
        messages("fn p(v: Future<Int>): Int { await v }"),
        vec!["cannot await in a pure function"]
    );
    // But it may make, hold and return one.
    assert_eq!(
        shown_result("fn make(): fn(Int) ->> Int { fn(x: Int) ->> Int { x } }\nmake"),
        "fn(): fn(Int) ->> Int | Future<Int>"
    );
}

#[test]
fn a_memoised_functions_body_is_impure() {
    // It may call impure functions and await, explicitly or not.
    let text = "fn tick() -> Int { 1 }\n\
                    fn m(x: Int) ->> Int {\n\
                    \x20 let f = lazy x + 1\n\
                    \x20 tick() + await f + f\n\
                    }\nm";
    assert_eq!(shown_result(text), "fn(x: Int) ->> Int");
    // And call memoised ones, itself included.
    let text = "fn m(x: Int) ->> Int { if x == 0 { 0 } else { m(x - 1) } }\n\
                    fn n(x: Int) ->> Int { m(x) + 1 }\nn";
    assert_eq!(shown_result(text), "fn(x: Int) ->> Int");
}

#[test]
fn a_pure_closure_in_a_memoised_function() {
    // A memoised function's parameters are present, so a pure closure
    // may use them.
    assert_eq!(
        shown_result("fn m(n: Int) ->> fn(): Int { fn(): Int { n + 1 } }\nm"),
        "fn(n: Int) ->> fn(): Int"
    );
    // A variable of its body that may be a future cannot be awaited by
    // the pure closure, and the memoised function, being impure, could
    // await it first: the hint says so.
    let text = "fn m(n: Int) ->> Int {\n\
                    \x20 let v = lazy n\n\
                    \x20 let g = fn(): Int { v + 1 }\n\
                    \x20 g()\n\
                    }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("v")]);
    // A pure enclosing function's bare parameter, seen through the
    // memoised function, is not claimable: measuring it there awaits it
    // in the memoised function, and it stays the either shape.
    assert_eq!(
        shown_result("fn outer(n: Int): fn(Int) ->> Int { fn(x: Int) ->> Int { n + x } }\nouter"),
        "fn(n: Int | Future<Int>): fn(Int) ->> Int | Future<Int>"
    );
    // Through the memoised function into a pure closure it is the
    // purity error, with the hint, the memoised function being able to
    // await it.
    let text = "fn outer(n: Int): fn() ->> fn(): Int {\n\
                    \x20 fn() ->> fn(): Int { fn(): Int { n + 1 } }\n\
                    }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("n")]);
    // With a pure function in its place, the parameter is claimed.
    assert_eq!(
        shown_result(
            "fn outer(n: Int): fn(): fn(): Int { fn(): fn(): Int { fn(): Int { n + 1 } } }\nouter"
        ),
        "fn(n: Int): fn(): fn(): Int"
    );
}
