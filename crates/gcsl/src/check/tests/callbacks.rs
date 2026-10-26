//! Tests of the checker: narrowed callbacks.

use super::*;

/// The type of the function named `run` in `text`, which must check, as
/// shown.
pub(super) fn type_of_run(text: &str) -> String {
    shown_result(&format!("{text}\nrun"))
}

/// The type of the function expression written first in `text` at the
/// first occurrence of `needle`, which starts with its `fn`, as shown,
/// whether the check succeeds or not.
pub(super) fn type_of_fn_at(text: &str, needle: &str) -> String {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let mut checker = Checker::new(&ast, &mut types);
    checker.check_body(ast.root(), None);
    let start = at(text, needle).start;
    let id = ast
        .expr_ids()
        .find(|&id| matches!(ast.get(id).kind, ExprKind::Fn(_)) && ast.get(id).span.start == start)
        .unwrap_or_else(|| panic!("no function at {needle:?} in {text:?}"));
    let ty = checker.expr_types[id.index()];
    types.display(ty, ast.names()).to_string()
}

/// The span of the `n`th (from 0) occurrence of `needle` in `text`,
/// from `offset` bytes into it, `len` bytes long.
pub(super) fn within(text: &str, needle: &str, n: usize, (offset, len): (u32, u32)) -> Span {
    let found = nth(text, needle, n);
    Span::new(found.start + offset, found.start + offset + len)
}

#[test]
fn callback_parameters_are_narrowed() {
    // The rows of the specification.
    for (text, shown) in [
        (
            "fn run(f: fn(Int) -> Int) -> Int { f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // `j` may be a future.
        (
            "fn run(f: fn(Int) -> Int, j: Int) -> Int { f(j) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> Int \
                 | Future<Int>",
        ),
        (
            "fn run(f: fn(Int) -> Int, j: Int) -> Int { f(await j) }",
            "fn(f: fn(Int) -> Int | Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Every call must fit.
        (
            "fn run(f: fn(Int) -> Int, j: Int) -> Int { f(1) + f(j) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> Int",
        ),
        // Each position on its own.
        (
            "fn run(f: fn(Int, Int) -> Int, j: Int) -> Int { f(1, j) }",
            "fn(f: fn(Int, Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> \
                 Int | Future<Int>",
        ),
        (
            "fn run(f: fn(Int) -> Int) -> Future<Int> { lazy f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Future<Int>",
        ),
        // Used as a value.
        (
            "fn run(f: fn(Int) -> Int) -> Int { let g = f\n g(1) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        (
            "fn other(g: fn(Int) -> Int) -> Int { g(1) }\n\
                 fn run(f: fn(Int) -> Int) -> Int { other(f) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Explicit positions.
        (
            "fn run(f: fn(Present<Int>) -> Int) -> Int { f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        (
            "fn run(f: fn(Int | Future<Int>) -> Int) -> Int { f(1) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // A record type, narrowed to its present type.
        (
            "fn run(f: fn({a: Int}) -> Int) -> Int { f({ a = 1 }) }",
            "fn(f: fn({a: Int}) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        (
            "fn run(f: fn({a: Int}) -> Int, j: Int) -> Int { f({ a = j }) }",
            "fn(f: fn({a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> Int | \
                 Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Vacuously: the call is of the shadowing `f`.
        (
            "fn run(f: fn(Int) -> Int) -> Int { let f = fn(x: Int) -> Int { x }\n f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Not a callback parameter.
        (
            "fn run(f: fn(Int) ->> Int) -> Int { f(1) }",
            "fn(f: fn(Int) ->> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // A memoised function's.
        (
            "fn run(f: fn(Int) -> Int) ->> Int { f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) ->> Int | Future<Int>",
        ),
    ] {
        assert_eq!(type_of_run(text), shown, "{text}");
    }
    // Explicit parts of a record type are kept, and its bare fields
    // are present.
    assert_eq!(
        type_of_run("fn run(f: fn({a: Future<Int>, b: Int}) -> Int) -> Int { 1 }"),
        "fn(f: fn({a: Future<Int>, b: Int}) -> Int | Future<Int>) -> Int"
    );
    // Neither is a function type's position, nor the callback's
    // result.
    assert_eq!(
        type_of_run("fn run(f: fn(fn(Int) -> Int) -> {a: Int}) -> Int { 1 }"),
        "fn(f: fn(fn(Int | Future<Int>) -> Int | Future<Int>) -> {a: Int | Future<Int>} | \
             Future<{a: Int | Future<Int>}>) -> Int"
    );
    // Nor anything of a pure function's, nor a parameter whose function
    // type is not written directly.
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int): Int { 1 }"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>): Int"
    );
    assert_eq!(
        type_of_run("fn run(r: {f: fn(Int) -> Int}) -> Int { r.f(1) }"),
        "fn(r: {f: fn(Int | Future<Int>) -> Int | Future<Int>} | Future<{f: fn(Int | \
             Future<Int>) -> Int | Future<Int>}>) -> Int | Future<Int>"
    );
    // Other uses as a value.
    for body in ["(f)(1)", "\"{f}\"", "f"] {
        let text = format!("fn run(f: fn(Int) -> Int) -> {{}} {{ let x = {body}\n{{}} }}");
        assert_eq!(
            type_of_run(&text),
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> {}",
            "{text}"
        );
    }
}

#[test]
fn a_narrowed_callback_takes_a_pure_function_that_measures() {
    let double = "fn double(x: Int): Int { x * 2 }\n";
    let run = "fn run(f: fn(Int) -> Int) -> Int { f(1) }\n";
    assert_eq!(
        shown_result(&format!("{double}{run}run(double)")),
        "Int | Future<Int>"
    );
    assert_eq!(
        shown_result(&format!("{run}run(fn(x: Int): Int {{ x * 2 }})")),
        "Int | Future<Int>"
    );
    // A written function type with the narrowed type's shape takes the
    // host.
    assert!(
        check(
            &parse_text(&format!(
                "{run}let r: fn(fn(Present<Int>) -> Int) -> Int = run"
            )),
            &mut TypeTable::new()
        )
        .is_ok()
    );
}

#[test]
fn a_callback_given_a_future_labels_the_mismatch() {
    let double = "fn double(x: Int): Int { x * 2 }\n";
    let function = "the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int \
                        | Future<Int>) -> Int";
    let given = "`f` is given a value that may be a future here";
    let used = "`f` is used as a value here, so it may be given a future";
    // A call that gives the position a value that may be a future.
    let text = format!(
        "{double}fn run(f: fn(Int) -> Int, j: Int) -> Int {{ f(1) + f(j) }}\nrun(double, 1)"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, last(&text, "double"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected fn(Int | Future<Int>) -> Int | Future<Int>, found fn(x: Int): Int")
    );
    assert_eq!(
        labels(&found[0]),
        vec![
            (last(&text, "run"), function),
            (within(&text, "f(j)", 0, (2, 1)), given)
        ]
    );
    // A use as a value.
    let text = format!(
        "{double}fn run(f: fn(Int) -> Int) -> Int {{\n  let g = f\n  g(1)\n}}\nrun(double)"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(
        labels(&found[0]),
        vec![
            (
                last(&text, "run"),
                "the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | \
                     Future<Int>"
            ),
            (within(&text, "= f", 0, (2, 1)), used)
        ]
    );
    // The first in source order.
    for (body, needle, label) in [
        ("let n = f(j)\n  let g = f\n  n", "f(j)", given),
        ("let g = f\n  f(j)", "= f", used),
    ] {
        let text = format!(
            "{double}fn run(f: fn(Int) -> Int, j: Int) -> Int {{\n  {body}\n}}\nrun(double, 1)"
        );
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        let span = within(&text, needle, 0, (2, 1));
        assert_eq!(labels(&found[0]).get(1), Some(&(span, label)), "{text}");
    }
    // At a position the argument needs: the second's, where only it
    // measures its parameter.
    let text = "fn run(f: fn(Int, Int) -> Int, j: Int) -> Int { f(j, 1) + f(1, j) }\n\
                    run(fn(a: Int | Future<Int>, b: Int): Int { b }, 1)";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        labels(&found[0]).get(1),
        Some(&(within(text, "1, j)", 0, (3, 1)), given))
    );
    // No label where the mismatch has another cause: the argument would
    // not fit had the position been narrowed.
    for arg in [
        "fn(x: String): Int { 1 }",
        "fn(x: Int): String { \"a\" }",
        "1",
    ] {
        let text = format!("fn run(f: fn(Int) -> Int, j: Int) -> Int {{ f(j) }}\nrun({arg}, 1)");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, "mismatched types", "{text}");
        assert_eq!(labels(&found[0]).len(), 1, "{text}: {found:?}");
    }
    // Nor for a callee that is not known.
    let text = format!(
        "{double}let r = {{ run = fn(f: fn(Int) -> Int, j: Int) -> Int {{ f(j) }} }}\n\
             r.run(double, 1)"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(labels(&found[0]).len(), 1, "{found:?}");
}

#[test]
fn a_callbacks_position_is_the_parameter_its_argument_fills() {
    // Named arguments, in any order.
    assert_eq!(
        type_of_run("fn run(f: fn(x: Int, y: Int) -> Int, j: Int) -> Int { f(y = 1, x = j) }"),
        "fn(f: fn(x: Int | Future<Int>, y: Int) -> Int | Future<Int>, j: Int | Future<Int>) \
             -> Int | Future<Int>"
    );
    // A trailing block fills the last.
    assert_eq!(
        type_of_run(
            "fn run(f: fn(n: Int, body: {a: Int}) -> Int, j: Int) -> Int { f(j) { a = 1 } }"
        ),
        "fn(f: fn(n: Int | Future<Int>, body: {a: Int}) -> Int | Future<Int>, j: Int | \
             Future<Int>) -> Int | Future<Int>"
    );
    assert_eq!(
        type_of_run(
            "fn run(f: fn(n: Int, body: {a: Int}) -> Int, j: Int) -> Int { f(1) { a = j } }"
        ),
        "fn(f: fn(n: Int, body: {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> \
             Int | Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn calls_of_a_callback_count_in_nested_functions_and_lazies() {
    let narrowed = "fn(f: fn(Int) -> Int | Future<Int>)";
    let written = "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>)";
    for (body, shown) in [
        ("let g = fn(x: Int) -> Int { f(1) }\n lazy g(2)", narrowed),
        ("let g = fn(x: Int) -> Int { f(x) }\n lazy g(2)", written),
        (
            "let g = fn(x: Int) -> Future<Int> { lazy f(1) }\n g(2)",
            narrowed,
        ),
        (
            "let g = fn(x: Int) -> Future<Int> { lazy f(x) }\n g(2)",
            written,
        ),
        // Awaited in the nested function first.
        (
            "let g = fn(x: Int) -> Future<Int> { let y = await x\n lazy f(y) }\n g(2)",
            narrowed,
        ),
    ] {
        let text = format!("fn run(f: fn(Int) -> Int) -> Future<Int> {{ {body} }}");
        assert_eq!(
            type_of_run(&text),
            format!("{shown} -> Future<Int>"),
            "{text}"
        );
    }
}

#[test]
fn callback_parameters_are_narrowed_independently() {
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int, g: fn(Int) -> Int, j: Int) -> Int { f(1) + g(j) }"),
        "fn(f: fn(Int) -> Int | Future<Int>, g: fn(Int | Future<Int>) -> Int | Future<Int>, \
             j: Int | Future<Int>) -> Int"
    );
    // A host nested in another, each with a callback, the inner calling
    // the outer's.
    let text = "fn run(f: fn(Int) -> Int, j: Int) -> Int {\n\
                    \x20 let inner = fn(g: fn(Int) -> Int) -> Int { f(j) + g(1) }\n\
                    \x20 inner(fn(x: Int) -> Int { x })\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> Int"
    );
    assert_eq!(
        type_of_fn_at(text, "fn(g"),
        "fn(g: fn(Int) -> Int | Future<Int>) -> Int"
    );
    let text = "fn run(f: fn(Int) -> Int, j: Int) -> Int {\n\
                    \x20 let inner = fn(g: fn(Int) -> Int) -> Int { f(1) + g(j) }\n\
                    \x20 inner(fn(x: Int) -> Int { x })\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int) -> Int | Future<Int>, j: Int | Future<Int>) -> Int"
    );
    assert_eq!(
        type_of_fn_at(text, "fn(g"),
        "fn(g: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int"
    );
    // An inner parameter of the same name shadows the outer one.
    let text = "fn run(f: fn(Int) -> Int, j: Int) -> Int {\n\
                    \x20 let inner = fn(f: fn(Int) -> Int) -> Int { f(j) }\n\
                    \x20 inner(fn(x: Int) -> Int { x })\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int) -> Int | Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn a_host_with_an_error_keeps_its_declared_type() {
    let declared = "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>";
    for text in [
        "fn run(f: fn(Int) -> Int) -> Int { f(1) + \"a\" }",
        "fn run(f: fn(Int) -> Int) -> Int { let x: String = f(1)\n1 }",
        "fn run(f: fn(Int) -> Int) -> Int { f(\"a\") }",
    ] {
        assert_eq!(messages(text).len(), 1, "{text}");
        assert_eq!(type_of_fn_at(text, "fn run"), declared, "{text}");
    }
    // And with an erroneous parameter type, erroneous itself.
    assert_eq!(
        type_of_fn_at("fn run(f: fn(Nope) -> Int) -> Int { f(1) }", "fn run"),
        "<error>"
    );
    // Its own name in its body has the declared type.
    let text = "fn run(f: fn(Int) -> Int, n: Int) -> Int {\n\
                    \x20 if n == 0 { f(1) } else { run(f, n - 1) }\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, n: Int | Future<Int>) -> Int | \
             Future<Int>"
    );
}

#[test]
fn a_host_is_narrowed_and_has_its_result_inferred() {
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int) -> Int { f(1) + 1 }"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> Int"
    );
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int) -> {v: Int, w: Int} { v = f(2); w = 3 }"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> {v: Int | Future<Int>, w: Int}"
    );
}

#[test]
fn an_untyped_literal_fits_a_callbacks_position_once_coerced() {
    for arg in ["1", "1 + 2", "(3)"] {
        let text = format!("fn run(f: fn(Float) -> Int) -> Int {{ f({arg}) }}");
        assert_eq!(
            type_of_run(&text),
            "fn(f: fn(Float) -> Int | Future<Int>) -> Int | Future<Int>",
            "{text}"
        );
    }
    assert_eq!(
        type_of_run("fn run(f: fn({a: Float}) -> Int) -> Int { f({ a = 1 }) }"),
        "fn(f: fn({a: Float}) -> Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn a_provisional_parameter_given_to_a_callback_may_be_a_future() {
    // The enclosing pure function's parameter may be a future for all
    // the nested impure host knows.
    let text = "fn outer(n: Int): fn(fn(Int) -> Int) -> Int {\n\
                    \x20 fn(f: fn(Int) -> Int) -> Int { f(n) }\n\
                    }";
    assert!(check(&parse_text(text), &mut TypeTable::new()).is_ok());
    assert_eq!(
        type_of_fn_at(text, "fn(f:"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    // Even where the pure function measures it elsewhere.
    let text = "fn outer(n: Int): fn(fn(Int) -> Int) -> Int {\n\
                    \x20 let m = n + 1\n\
                    \x20 fn(f: fn(Int) -> Int) -> Int { f(n) }\n\
                    }";
    assert_eq!(
        type_of_fn_at(text, "fn(f:"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    // Awaited in the host first, it is present.
    let text = "fn outer(n: Int): fn(fn(Int) -> Int) -> Int {\n\
                    \x20 fn(f: fn(Int) -> Int) -> Int { f(await n) }\n\
                    }";
    assert_eq!(
        type_of_fn_at(text, "fn(f:"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>"
    );
}
