//! Tests of the checker: inferred parameters and results, and awaits in updates
//! and captures.

use super::*;

/// The type of the function `f` that `text` declares, as shown.
pub(super) fn type_of_f(text: &str) -> String {
    shown_result(&format!("{text}\nf"))
}

/// Where the body of function number `n` of `text` first measures each
/// of its parameters, as source text, whether the check succeeds or
/// not.
pub(super) fn demands(text: &str, n: u32) -> Vec<Option<&str>> {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let mut checker = Checker::new(&ast, &mut types);
    checker.check_body(ast.root(), None);
    checker.fn_resolutions[FnId::new(n).index()]
        .param_demands
        .iter()
        .map(|demand| demand.map(|span| &text[span.start as usize..span.end as usize]))
        .collect()
}

#[test]
fn pure_parameters_are_inferred() {
    // The functions of the specification.
    assert_eq!(type_of_f("fn f(x: Int): Int { x * 2 }"), "fn(x: Int): Int");
    assert_eq!(
        type_of_f("fn f(x: Int): {v: Int | Future<Int>} { v = x }"),
        "fn(x: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    // A pure function's bare result is still present, so returning a
    // parameter measures it.
    assert_eq!(type_of_f("fn f(x: Int): Int { x }"), "fn(x: Int): Int");
    // Each parameter on its own; a record type too.
    assert_eq!(
        type_of_f("fn f(a: Int, r: {n: Int}, s: String): Int { a + r.n }"),
        "fn(a: Int, r: {n: Int}, s: String | Future<String>): Int"
    );
    assert_eq!(
        type_of_f("fn f(r: {n: Int}): {r: {n: Int} | Future<{n: Int}>} { r = r }"),
        "fn(r: {n: Int} | Future<{n: Int}>): {r: {n: Int} | Future<{n: Int}>}"
    );
    // Not a parameter written `Present<...>`, `Future<...>`, a union or
    // a function type, which says what it is.
    assert_eq!(
        type_of_f(
            "fn f(a: Present<Int>, b: Future<Int>, c: Int | Future<Int>, g: fn(): Int): Int { 1 }"
        ),
        "fn(a: Int, b: Future<Int>, c: Int | Future<Int>, g: fn(): Int): Int"
    );
    // Nor an impure function's, which is always the either shape.
    assert_eq!(
        type_of_f("fn f(x: Int) -> Int { 1 }"),
        "fn(x: Int | Future<Int>) -> Int"
    );
}

#[test]
fn each_measuring_position_measures_a_parameter() {
    // Measuring operators, whatever else they find.
    for (body, ret) in [
        ("x + 1", "Int"),
        ("1 - x", "Int"),
        ("x * 2", "Int"),
        ("x / 2", "Float"),
        ("x // 2", "Int"),
        ("x % 2", "Int"),
        ("x < 1", "Bool"),
        ("x <= 1", "Bool"),
        ("x > 1", "Bool"),
        ("x >= 1", "Bool"),
        ("x == 1", "Bool"),
        ("x != 1", "Bool"),
        ("x === 1", "Bool"),
        ("x !== 1", "Bool"),
        ("-x", "Int"),
        ("\"{x}\"", "String"),
        // Present positions: an annotation, an ascription, a `:=`
        // target, a field, the result.
        ("let y: Int = x\ny", "Int"),
        ("let y = (x : Int)\ny", "Int"),
        ("let y = 0\ny := x\ny", "Int"),
    ] {
        let text = format!("fn f(x: Int): {ret} {{ {body} }}");
        assert_eq!(demands(&text, 0), vec![Some("x")], "{text}");
        assert_eq!(type_of_f(&text), format!("fn(x: Int): {ret}"), "{text}");
    }
    // The result, measured where the body is checked against the
    // return type.
    let text = "fn f(x: Int): Int { x }";
    assert_eq!(demands(text, 0), vec![Some("{ x }")]);
    for body in [
        "b and true",
        "false or b",
        "not b",
        "if b { true } else { false }",
    ] {
        let text = format!("fn f(b: Bool): Bool {{ {body} }}");
        assert_eq!(demands(&text, 0), vec![Some("b")], "{text}");
        assert_eq!(type_of_f(&text), "fn(b: Bool): Bool", "{text}");
    }
    // A field access; a field of a present record type.
    let text = "fn f(r: {a: Int}): Int { r.a }";
    assert_eq!(demands(text, 0), vec![Some("r")]);
    let text = "fn f(x: Int): {v: Int} { v = x }";
    assert_eq!(demands(text, 0), vec![Some("x")]);
    assert_eq!(type_of_f(text), "fn(x: Int): {v: Int}");
    // A call of it, which is an error, since no parameter written as a
    // name or a record type is a function.
    let text = "fn f(x: {}): Int { x() }";
    assert_eq!(demands(text, 0), vec![Some("x")]);
    assert_eq!(messages(text), vec!["cannot call a value of type `{}`"]);
    // An argument against a present parameter, written so or inferred.
    let text = "fn h(n: Present<Int>): Int { n }\nfn f(x: Int): Int { h(x) }";
    assert_eq!(demands(text, 1), vec![Some("x")]);
    let text = "fn h(n: Int): Int { n + 1 }\nfn f(x: Int): Int { h(x) }";
    assert_eq!(demands(text, 1), vec![Some("x")]);
    assert_eq!(type_of_f(text), "fn(x: Int): Int");
    // A recursive call, every argument that is a parameter, even
    // against a parameter that is passed through.
    let text = "fn f(x: Int, n: Int): Int | Future<Int> {\n\
                    \x20 if n == 0 { 0 } else { f(x, n - 1) }\n}";
    assert_eq!(demands(text, 0), vec![Some("x"), Some("n")]);
    assert_eq!(type_of_f(text), "fn(x: Int, n: Int): Int | Future<Int>");
    let text = "fn f(x: Int, y: Int): Int { if true { 0 } else { f(y, x) } }";
    assert_eq!(demands(text, 0), vec![Some("x"), Some("y")]);
    // A statement whose value is `{}` once awaited.
    let text = "fn f(x: {}): Int { x\n1 }";
    assert_eq!(demands(text, 0), vec![Some("x")]);
    assert_eq!(type_of_f(text), "fn(x: {}): Int");
    // An update of a record reads it.
    let text = "fn f(r: {a: Int}): {a: Int, b: Int} { let r.b = 2\nr }";
    assert_eq!(demands(text, 0), vec![Some("r")]);
    assert_eq!(type_of_f(text), "fn(r: {a: Int}): {a: Int, b: Int}");
    // The first measuring use in source order is the one recorded.
    let text = "fn f(x: Int): Int { let y = x + 1\nlet z: Int = x\ny + z }";
    let first = at(text, "x + 1");
    assert_eq!(
        resolved(text).1.function(FnId::new(0)).param_demands,
        vec![Some(Span::new(first.start, first.start + 1))]
    );
    // Also when a later use in source order is checked first: the body
    // is checked against the return type after its items.
    let text = "fn f(x: Int): {a: Int, b: Int} { a = x; b = x + 1 }";
    assert_eq!(
        resolved(text).1.function(FnId::new(0)).param_demands,
        vec![Some(nth(text, "x", 1))]
    );
}

#[test]
fn a_parameter_passed_on_is_not_measured() {
    for (text, shown) in [
        // Passed to an either parameter.
        (
            "fn h(n: Int | Future<Int>): Int { 1 }\nfn f(x: Int): Int { h(x) }",
            "fn(x: Int | Future<Int>): Int",
        ),
        // Stored in a record whose field is the either shape, and
        // returned against an either return type.
        (
            "fn f(x: Int): {v: Int | Future<Int>} { v = x }",
            "fn(x: Int | Future<Int>): {v: Int | Future<Int>}",
        ),
        (
            "fn f(x: Int): Int | Future<Int> { x }",
            "fn(x: Int | Future<Int>): Int | Future<Int>",
        ),
        // Captured by a closure or a `lazy`, which measure it on their
        // own account.
        (
            "fn f(x: Int): fn() -> Int { fn() -> Int { x + 1 } }",
            "fn(x: Int | Future<Int>): fn() -> Int | Future<Int>",
        ),
        (
            "fn f(x: Int): Future<Int> { lazy x + 1 }",
            "fn(x: Int | Future<Int>): Future<Int>",
        ),
        // Bound by a `let` without an annotation.
        (
            "fn f(x: Int): Int | Future<Int> { let y = x\ny }",
            "fn(x: Int | Future<Int>): Int | Future<Int>",
        ),
        // Unused.
        ("fn f(x: Int): Int { 1 }", "fn(x: Int | Future<Int>): Int"),
    ] {
        assert_eq!(type_of_f(text), shown, "{text}");
        // `f` is the last function finished, so the last numbered.
        let (ast, resolution) = resolved(text);
        let f = FnId::new(u32::try_from(ast.fn_count() - 1).unwrap());
        assert_eq!(resolution.function(f).param_demands, vec![None], "{text}");
    }
}

#[test]
fn blind_assignment_keeps_the_parameter() {
    // Through a `let`, a record's field read back, and an `if` whose
    // branches are both the parameter: measured where it is used.
    let text = "fn f(x: Int): Int { let y = x\ny + 1 }";
    assert_eq!(demands(text, 0), vec![Some("y")]);
    let text = "fn f(x: Int): Int { let r = { v = x }\nr.v + 1 }";
    assert_eq!(demands(text, 0), vec![Some("r.v")]);
    let text = "fn f(c: Bool, x: Int): Int { (if c { x } else { x }) + 1 }";
    assert_eq!(
        demands(text, 0),
        vec![Some("c"), Some("(if c { x } else { x })")]
    );
    // And passed on unmeasured through each.
    for text in [
        "fn f(x: Int): {v: Int | Future<Int>} { let y = x\nv = y }",
        "fn f(x: Int): {v: Int | Future<Int>} { let r = { w = x }\nv = r.w }",
        "fn f(c: Bool, x: Int): Int | Future<Int> { if c { x } else { x } }",
    ] {
        assert_eq!(demands(text, 0).last(), Some(&None), "{text}");
    }
    // Two parameters joined are both measured, conservatively, and so
    // is a parameter joined with anything else.
    let text = "fn f(c: Bool, x: Int, y: Int): Int | Future<Int> { if c { x } else { y } }";
    assert_eq!(
        demands(text, 0),
        vec![Some("c"), Some("{ x }"), Some("{ y }")]
    );
    assert_eq!(
        type_of_f(text),
        "fn(c: Bool, x: Int, y: Int): Int | Future<Int>"
    );
    let text = "fn f(c: Bool, x: Int): Int { if c { x } else { 1 } }";
    assert_eq!(demands(text, 0), vec![Some("c"), Some("{ x }")]);
    assert_eq!(type_of_f(text), "fn(c: Bool, x: Int): Int");
    // (The `lazy` is function number 0, finished first.)
    let text = "fn f(c: Bool, x: Int): Int | Future<Int> { if c { x } else { lazy 1 } }";
    assert_eq!(demands(text, 1), vec![Some("c"), Some("{ x }")]);
    assert_eq!(
        shown_result(&format!("{text}\nf(true, 1)")),
        "Int | Future<Int>"
    );
}

#[test]
fn a_nested_function_measures_an_enclosing_parameter_through_pure_functions() {
    // A nested pure function measuring it makes it present for the
    // function it belongs to, which is where its caller awaits it.
    let text = "fn f(x: Int): fn(): Int { fn(): Int { x + 1 } }";
    assert_eq!(type_of_f(text), "fn(x: Int): fn(): Int");
    assert_eq!(demands(text, 1), vec![Some("x")]);
    assert!(marks(text).is_empty());
    // Through two pure functions, and where the nested function uses
    // it as its result.
    let text = "fn f(x: Int): fn(): fn(): Int { fn(): fn(): Int { fn(): Int { x } } }";
    assert_eq!(type_of_f(text), "fn(x: Int): fn(): fn(): Int");
    assert_eq!(demands(text, 2), vec![Some("{ x }")]);
    // Passed on unmeasured by the nested function, it stays the either
    // shape, and the nested function's own type says so.
    let text = "fn f(x: Int): fn(): Int | Future<Int> { fn(): Int | Future<Int> { x } }";
    assert_eq!(
        type_of_f(text),
        "fn(x: Int | Future<Int>): fn(): Int | Future<Int>"
    );
    assert_eq!(demands(text, 1), vec![None]);
    // Through a `lazy` or an impure function, a pure function cannot
    // measure it: the `lazy` may await it, so it may be a future there.
    let text = "fn f(x: Int): Future<Int> { lazy { let g = fn(): Int { x + 1 }\ng() } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "cannot use a value that may be a future in a pure function"
    );
    assert_eq!(found[0].span, nth(text, "x", 1));
    assert_eq!(found[0].label.as_deref(), Some("this is Int | Future<Int>"));
    // The nested function's marker, and nothing measured for `f`.
    assert_eq!(labels(&found[0]), vec![(nth(text, ":", 2), PURE_CONTEXT)]);
    assert_eq!(demands(text, 2), vec![None]);
    // A nested `lazy` or impure function awaits it where it measures it.
    assert_eq!(
        marks("fn f(x: Int): Future<Int> { lazy x + 1 }"),
        vec![("x", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(x: Int): fn() -> Int { fn() -> Int { x + 1 } }"),
        vec![("x", AwaitMark::Top)]
    );
    // Even where the enclosing function measures it too, so it is in
    // fact present: the await then finds a present value.
    let text = "fn f(x: Int): Future<Int> { let y = x + 1\nlazy x + y }";
    assert_eq!(type_of_f(text), "fn(x: Int): Future<Int>");
    assert_eq!(marks(text), vec![("x", AwaitMark::Top)]);
}

#[test]
fn every_type_in_a_body_is_settled() {
    let text = "fn f(x: Int, r: {a: Int}, s: String): {v: Int | Future<Int>, w: Int} {\n\
                    \x20 let y = x\n\
                    \x20 let q = { k = y; t = s }\n\
                    \x20 let g = fn() -> Int { r.a }\n\
                    \x20 v = if true { q.k } else { y }\n\
                    \x20 w = r.a\n\
                    }\nf";
    let (ast, types, output) = check_text(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    let dump = ast
        .pretty_with(&output.checked.types.annotation(&types, ast.names()))
        .to_string();
    assert!(!dump.contains('?'), "{dump}");
    assert!(
        dump.contains(
            "fn(x: Int | Future<Int>, r: {a: Int}, s: String | Future<String>): \
                 {v: Int | Future<Int>, w: Int}"
        ),
        "{dump}"
    );
    assert!(
        dump.contains("(name s : String | Future<String> @"),
        "{dump}"
    );
    // The captures of a nested function are settled too.
    let (ast, types, output) = check_text(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    let captured: Vec<String> = output
        .checked
        .resolution
        .function(FnId::new(0))
        .captures
        .iter()
        .map(|&(_, ty)| types.display(ty, ast.names()).to_string())
        .collect();
    assert_eq!(captured, vec!["{a: Int}"]);
    // A function is settled even when its check fails, so no `?` is
    // left in a failed check's types either.
    let text = "fn f(x: Int): Int { x + \"a\" }";
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let table = {
        let mut checker = Checker::new(&ast, &mut types);
        checker.check_body(ast.root(), None);
        assert_eq!(checker.errors.len(), 1);
        Types::new(checker.expr_types)
    };
    let dump = ast
        .pretty_with(&table.annotation(&types, ast.names()))
        .to_string();
    assert!(dump.contains("(name x : Int @20..21)"), "{dump}");
    assert!(!dump.contains('?'), "{dump}");
}

/// The note of the call-site form of the implicit-await error, with the
/// call written with its await, if any.
pub(super) fn call_site_note(call: Option<&str>) -> String {
    let note = "a pure function cannot await; await the argument before the call, in an \
                    impure function";
    match call {
        Some(call) => format!("{note}: `{call}`"),
        None => note.to_owned(),
    }
}

#[test]
fn the_call_site_form_of_the_implicit_await_error() {
    let text = "fn double(x: Int): Int {\n  let y = x + 5\n  y\n}\n\
                    fn caller(my_future: Future<Int>): Int {\n  double(my_future)\n}";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    let error = &found[0];
    assert_eq!(
        error.message,
        "argument `my_future` would be awaited inside `double`, which is pure"
    );
    assert_eq!(error.span, last(text, "my_future"));
    assert_eq!(error.label.as_deref(), Some("this is Future<Int>"));
    let marker = nth(text, "):", 1);
    assert_eq!(
        labels(error),
        vec![
            (nth(text, "x", 1), "`x` is measured here"),
            (Span::new(marker.start + 1, marker.end), PURE_CONTEXT)
        ]
    );
    assert_eq!(
        error.notes,
        vec![call_site_note(Some("double(await my_future)"))]
    );
    // An either argument; a dotted path, among other arguments, one
    // named; an argument that is neither a name nor a dotted path.
    let double = "fn double(x: Int, n: Int): Int { x * n }\n";
    let text = format!("{double}fn caller(v: Int | Future<Int>): Int {{ double(v, 2) }}");
    let error = &diagnostics(&text)[0];
    assert_eq!(
        error.message,
        "argument `v` would be awaited inside `double`, which is pure"
    );
    assert_eq!(error.label.as_deref(), Some("this is Int | Future<Int>"));
    // `2` is not a name, so the call is not written out.
    assert_eq!(error.notes, vec![call_site_note(None)]);
    let text = format!(
        "{double}fn caller(r: {{f: Future<Int>}}, m: Int): Int {{ double(n = m, x = r.f) }}"
    );
    let error = &diagnostics(&text)[0];
    assert_eq!(
        error.message,
        "argument `r.f` would be awaited inside `double`, which is pure"
    );
    assert_eq!(
        error.notes,
        vec![call_site_note(Some("double(n = m, x = await r.f)"))]
    );
    let text = format!("{double}fn caller(): Int {{ double(lazy 1, 2) }}");
    let error = &diagnostics(&text)[0];
    assert_eq!(
        error.message,
        "this argument would be awaited inside `double`, which is pure"
    );
    assert_eq!(error.span, at(&text, "lazy 1"));
    // An impure caller awaits the argument at the call.
    let text = format!("{double}fn caller(v: Future<Int>) -> Int {{ double(v, 2) }}");
    assert_eq!(marks(&text), vec![("v", AwaitMark::Top)]);
    // A parameter written present, or of a function that is not known,
    // gets the generic form.
    for text in [
        "fn double(x: Present<Int>): Int { x * 2 }\nfn caller(v: Future<Int>): Int { double(v) }",
        "fn double(x: Int): Int { x * 2 }\nlet d = double\nfn caller(v: Future<Int>): Int { d(v) }",
        "fn caller(d: fn(Int): Int, v: Future<Int>): Int { d(v) }",
    ] {
        assert_eq!(
            messages(text),
            vec!["cannot use a value that may be a future in a pure function"],
            "{text}"
        );
    }
}

#[test]
fn the_call_site_form_for_a_function_of_an_earlier_input() {
    let mut names = Names::new();
    let mut types = TypeTable::new();
    let parse_with = |text: &str, names: &mut Names| {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, strings) = tokenize_with(&source, names).unwrap();
        parse(&tokens, names.clone(), strings, &source).unwrap()
    };
    let first = parse_with(
        "fn double(x: Int): Int { x * 2 }\nfn keep(x: Int): {v: Int | Future<Int>} { v = x }\n\
             fn written(x: Present<Int>): Int { x }",
        &mut names,
    );
    let scope = Checker::new(&first, &mut types).check().unwrap().scope;
    // The call-site form, without the label on the measuring use, which
    // is in the earlier input's source.
    let text = "fn caller(v: Future<Int>): Int { double(v) }";
    let second = parse_with(text, &mut names);
    let found = Checker::with_scope(&second, scope.clone(), &mut types)
        .check()
        .unwrap_err();
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "argument `v` would be awaited inside `double`, which is pure"
    );
    assert_eq!(found[0].span, last(text, "v"));
    assert_eq!(labels(&found[0]), vec![(nth(text, ":", 1), PURE_CONTEXT)]);
    assert_eq!(
        found[0].notes,
        vec![call_site_note(Some("double(await v)"))]
    );
    // Passed through, nothing is awaited; written present, the generic
    // form.
    let third = parse_with(
        "fn caller(v: Future<Int>): {v: Int | Future<Int>} { keep(v) }",
        &mut names,
    );
    assert!(
        Checker::with_scope(&third, scope.clone(), &mut types)
            .check()
            .is_ok()
    );
    let fourth = parse_with("fn caller(v: Future<Int>): Int { written(v) }", &mut names);
    let found = Checker::with_scope(&fourth, scope, &mut types)
        .check()
        .unwrap_err();
    assert_eq!(
        found[0].message,
        "cannot use a value that may be a future in a pure function"
    );
}

/// The type of function expression number `n` of `text`, as shown,
/// whether the check succeeds or not.
pub(super) fn fn_type(text: &str, n: u32) -> String {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let mut checker = Checker::new(&ast, &mut types);
    checker.check_body(ast.root(), None);
    let id = ast
        .expr_ids()
        .find(|&id| ast.get(id).kind == ExprKind::Fn(FnId::new(n)))
        .unwrap_or_else(|| panic!("no function {n} in {text:?}"));
    let ty = checker.expr_types[id.index()];
    types.display(ty, ast.names()).to_string()
}

#[test]
fn an_impure_functions_bare_result_is_inferred() {
    // Present where the body's value is, and so is a call's.
    let get = "fn get() -> {a: Int, b: String} { a = 1; b = \"x\" }\n";
    assert_eq!(
        shown_result(&format!("{get}get")),
        "fn() -> {a: Int, b: String}"
    );
    assert_eq!(shown_result(&format!("{get}get()")), "{a: Int, b: String}");
    assert!(marks(&format!("{get}get().a + 1")).is_empty());
    // A future where it is one.
    assert_eq!(
        shown_result("fn plan() -> {build: Int} { build = lazy 5 }\nplan"),
        "fn() -> {build: Future<Int>}"
    );
    assert_eq!(
        shown_result("fn g() -> {b: Int, a: String} { a = \"x\"; b = lazy 1 }\ng"),
        "fn() -> {a: String, b: Future<Int>}"
    );
    assert_eq!(
        shown_result("fn later() -> Int { lazy 5 }\nlater"),
        "fn() -> Future<Int>"
    );
    // The either shape where it may be either: a parameter, which may
    // be a future, but not once measured.
    assert_eq!(
        shown_result("fn pass(x: Int) -> Int { x }\npass"),
        "fn(x: Int | Future<Int>) -> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn add(x: Int) -> Int { x + 1 }\nadd"),
        "fn(x: Int | Future<Int>) -> Int"
    );
    // An `if` joining a present value and a future.
    assert_eq!(
        shown_result("fn j(c: Bool) -> Int { if c { 1 } else { lazy 2 } }\nj"),
        "fn(c: Bool | Future<Bool>) -> Int | Future<Int>"
    );
    // A field the declared type lacks plays no part, and is warned about
    // as before.
    let text = "fn wide() -> {a: Int} { a = 1; b = 2 }\nwide";
    assert_eq!(shown_result(text), "fn() -> {a: Int}");
    assert_eq!(
        warnings(text)[0].message,
        "field `b` is not in `{a: Int | Future<Int>}`"
    );
    // Nested records, inferred at every bare position.
    assert_eq!(
        shown_result("fn n() -> {a: {b: Int}} { a = { b = 1 } }\nn"),
        "fn() -> {a: {b: Int}}"
    );
    assert_eq!(
        shown_result("fn n() -> {a: {b: Int}} { a = lazy { b = 1 } }\nn"),
        "fn() -> {a: Future<{b: Int}>}"
    );
    assert_eq!(
        shown_result("fn n(x: Int) -> {a: {b: Int, c: Int}} { a = { b = x; c = 1 } }\nn"),
        "fn(x: Int | Future<Int>) -> {a: {b: Int | Future<Int>, c: Int}}"
    );
    // A record that is not a record block, whose fields are its type's.
    assert_eq!(
        shown_result("fn f() -> {a: Int} { = { a = lazy 1 } }\nf"),
        "fn() -> {a: Future<Int>}"
    );
    // The empty record.
    assert_eq!(shown_result("fn tick() -> {} { }\ntick"), "fn() -> {}");
    assert_eq!(
        shown_result("fn tock() -> {} { lazy {} }\ntock"),
        "fn() -> Future<{}>"
    );
}

#[test]
fn an_inferred_result_is_what_the_body_returns_once_awaited() {
    // A body awaited deeply to fit a field written present is awaited only
    // toward the type it fits: a bare position, which a future fits, keeps
    // its laziness.
    let text = "fn f() -> {a: Present<Int>, b: Int} { = { a = lazy 1; b = lazy 2 } }\nf";
    assert_eq!(shown_result(text), "fn() -> {a: Int, b: Future<Int>}");
    assert!(matches!(
        marks(text)[..],
        [("{ = { a = lazy 1; b = lazy 2 } }", AwaitMark::Toward(_))]
    ));
    // A field's value awaited to fit its field is present; one that is
    // not keeps its laziness.
    let text = "fn m(x: Future<Int>) -> {a: Present<Int>, b: Int} { a = x; b = x }\nm";
    assert_eq!(
        shown_result(text),
        "fn(x: Future<Int>) -> {a: Int, b: Future<Int>}"
    );
    assert_eq!(marks(text), vec![("x", AwaitMark::Top)]);
    // An untyped literal has been coerced.
    assert_eq!(
        shown_result("fn f() -> Float { 1 + 2 }\nf"),
        "fn() -> Float"
    );
}

#[test]
fn an_inferred_result_keeps_what_is_written_explicitly() {
    assert_eq!(
        shown_result("fn e() -> Int | Future<Int> { 5 }\ne"),
        "fn() -> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn p() -> Present<Int> { 5 }\np"),
        "fn() -> Int"
    );
    assert_eq!(
        shown_result("fn f() -> Future<Int> { lazy 5 }\nf"),
        "fn() -> Future<Int>"
    );
    assert_eq!(
        shown_result("fn u() -> {a: Int} | Future<{a: Int}> { a = 1 }\nu"),
        "fn() -> {a: Int} | Future<{a: Int}>"
    );
    // A function type at its own position, whatever the function the
    // body returns is inferred to be.
    assert_eq!(
        shown_result("fn mk() -> fn() -> Int { fn() -> Int { 5 } }\nmk"),
        "fn() -> fn() -> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn mk() -> {g: fn() -> Int} { g = fn() -> Int { 5 } }\nmk"),
        "fn() -> {g: fn() -> Int | Future<Int>}"
    );
    // A pure function's result is present, as written.
    assert_eq!(shown_result("fn p(): Int { 5 }\np"), "fn(): Int");
}

#[test]
fn a_recursive_call_has_the_declared_result() {
    // Inferred once, when the body ends, from the declared type of the
    // recursive call's result.
    assert_eq!(
        shown_result("fn count(n: Int) -> Int { if n == 0 { 0 } else { count(n - 1) } }\ncount"),
        "fn(n: Int | Future<Int>) -> Int | Future<Int>"
    );
    let text = "fn sum(n: Int) -> Int { if n == 0 { 0 } else { sum(n - 1) + n } }";
    assert_eq!(
        shown_result(&format!("{text}\nsum")),
        "fn(n: Int | Future<Int>) -> Int"
    );
    // Inside the body, the name has the declared type.
    let (ast, types, output) = check_text(text);
    let output = output.unwrap();
    let recursive = ast
        .expr_ids()
        .find(|&id| ast.get(id).span == last(text, "sum"))
        .unwrap();
    assert_eq!(
        types
            .display(output.checked.types.get(recursive), ast.names())
            .to_string(),
        "fn(n: Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn a_statement_awaits_an_inferred_result_only_if_it_may_be_a_future() {
    assert!(marks("fn tick() -> {} { }\ntick()\n1").is_empty());
    assert_eq!(
        marks("fn tock() -> {} { lazy {} }\ntock()\n1"),
        vec![("tock()", AwaitMark::Top)]
    );
}

#[test]
fn dotted_fields_build_a_record_that_may_be_a_future() {
    // The record a dotted field builds is present, which the either
    // shape of a record type, as an impure function's bare result is at
    // every level, accepts. An open field's type is inferred (see
    // `dotted_fields_in_an_inferred_result`).
    let text = "fn n(c: Int) -> {a: {b: Int, d: Int}} { a.b = c; a.d = 2 }\nn(lazy 4).a.d";
    assert!(warnings(text).is_empty());
    assert_eq!(shown_result(text), "Int");
    assert_eq!(
        shown_result("fn n() -> {a: {b: {c: Int}}} { a.b.c = 1 }\nn().a.b.c"),
        "Int"
    );
    // A segment defined whole there cannot be extended, as anywhere.
    for whole in ["{ c = 1 }", "lazy { c = 1 }"] {
        let text = format!("fn n() -> {{a: {{b: {{c: Int}}}}}} {{ a.b = {whole}; a.b.d = 2 }}");
        assert_eq!(
            messages(&text),
            vec!["field `a.b` is already defined"],
            "{text}"
        );
    }
    // A rebind that is rejected does not go on to report its path.
    assert_eq!(
        messages("let r = lazy { a = 1 }\nlet g = fn(): Int { r.a := 2\n1 }"),
        vec!["cannot rebind `r` here: it is declared in an enclosing block"]
    );
    // Written as the either shape, in an annotation too.
    assert_eq!(
        shown_result("let r: {a: {b: Int} | Future<{b: Int}>} = { a.b = 1 }\nr.a.b"),
        "Int"
    );
    // A missing field is reported against the record type.
    let found = diagnostics("fn n() -> {a: {b: Int, c: Int}} { a.b = 1 }");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "missing field `a.c` in record");
    assert_eq!(
        found[0].notes,
        vec!["the type `{b: Int | Future<Int>, c: Int | Future<Int>}` needs it"]
    );
    // A future is not what a dotted field builds.
    assert_eq!(
        messages("fn n() -> {a: Future<{b: Int}>} { a.b = 1 }"),
        vec!["mismatched types"]
    );
    // A leaf written present is awaited.
    let text = "fn n() -> {a: {b: Present<Int>}} { let x = lazy 1\na.b = x }";
    assert_eq!(marks(text), vec![("x", AwaitMark::Top)]);
}

#[test]
fn dotted_fields_in_an_inferred_result() {
    // A field built by dotted paths is a present record, inferred as a
    // field defined whole is.
    assert_eq!(
        shown_result("fn n() -> {a: {b: Int}} { a.b = 1 }\nn()"),
        "{a: {b: Int}}"
    );
    assert_eq!(
        shown_result("fn w() -> {a: {b: Int}} { a = { b = 1 } }\nw()"),
        "{a: {b: Int}}"
    );
    assert_eq!(
        shown_result("fn m(c: Int) -> {a: {b: Int, d: Int}} { a.b = c; a.d = 2 }\nm"),
        "fn(c: Int | Future<Int>) -> {a: {b: Int | Future<Int>, d: Int}}"
    );
    // Three levels, and a segment extended by several paths.
    assert_eq!(
        shown_result("fn n() -> {a: {b: {c: Int}}} { a.b.c = 1 }\nn"),
        "fn() -> {a: {b: {c: Int}}}"
    );
    assert_eq!(
        shown_result(
            "fn n() -> {a: {b: {c: Int, d: Int}, e: Int}} { a.b.c = lazy 1; a.e = 2; a.b.d = 3 }\nn"
        ),
        "fn() -> {a: {b: {c: Future<Int>, d: Int}, e: Int}}"
    );
    // Whole and dotted fields in one block.
    assert_eq!(
        shown_result(
            "fn n() -> {x: Int, a: {b: Int, c: {d: Int}}, y: {z: Int}} {\n\
                 \x20 x = 1; a.b = 2; y = { z = lazy 3 }; a.c.d = lazy 4\n\
                 }\nn"
        ),
        "fn() -> {a: {b: Int, c: {d: Future<Int>}}, x: Int, y: {z: Future<Int>}}"
    );
    // A leaf that is a future.
    assert_eq!(
        shown_result("fn n() -> {a: {b: Int}} { a.b = lazy 1 }\nn"),
        "fn() -> {a: {b: Future<Int>}}"
    );
    // A leaf that is a record block, checked field by field.
    assert_eq!(
        shown_result("fn n() -> {a: {b: {c: Int}}} { a.b = { c = lazy 1 } }\nn"),
        "fn() -> {a: {b: {c: Future<Int>}}}"
    );
    // A leaf written present, given a future, is awaited, and stays as
    // written.
    let text = "fn n() -> {a: {b: Present<Int>}} { let x = lazy 1\na.b = x }\nn";
    assert_eq!(shown_result(text), "fn() -> {a: {b: Int}}");
    assert_eq!(marks(text), vec![("x", AwaitMark::Top)]);
    // A dotted field under an explicit either record type is kept as
    // written.
    assert_eq!(
        shown_result("fn n() -> {a: {b: Int} | Future<{b: Int}>} { a.b = 1 }\nn"),
        "fn() -> {a: {b: Int} | Future<{b: Int}>}"
    );
    // A dotted field, or a segment of one, the declared type lacks plays
    // no part, and is warned about as before.
    let text = "fn n() -> {a: {b: Int}} { a.b = 1; a.c = lazy 2; q.r = lazy 3 }\nn";
    assert_eq!(shown_result(text), "fn() -> {a: {b: Int}}");
    let found: Vec<String> = warnings(text).into_iter().map(|w| w.message).collect();
    assert_eq!(
        found,
        vec![
            "field `c` is not in `{b: Int | Future<Int>}`",
            "field `q` is not in `{a: {b: Int | Future<Int>} | Future<{b: Int | Future<Int>}>}`"
        ]
    );
    // A memoised function's result, in the same way.
    assert_eq!(
        shown_result("fn n(x: Int) ->> {a: {b: Int, c: Int}} { a.b = x; a.c = lazy x }\nn"),
        "fn(x: Int) ->> {a: {b: Int, c: Future<Int>}}"
    );
}

#[test]
fn a_function_with_an_error_keeps_its_declared_result() {
    // In the body, or where the body does not fit: one error, and the
    // declared type, so that its calls do not cascade.
    for (text, message, shown) in [
        (
            "fn f() -> Int { 1 + \"a\" }\nf() + 1",
            "cannot add Int and String",
            "fn() -> Int | Future<Int>",
        ),
        (
            "fn f() -> Int { \"a\" }\nf() + 1",
            "mismatched types",
            "fn() -> Int | Future<Int>",
        ),
        (
            "fn f() -> {a: Int, b: Int} { b = 1 }\nf().a + 1",
            "missing field `a` in record",
            "fn() -> {a: Int | Future<Int>, b: Int | Future<Int>} | Future<{a: Int | \
                 Future<Int>, b: Int | Future<Int>}>",
        ),
        // An error elsewhere in the body.
        (
            "fn f() -> Int { let x: Int = \"a\"\n1 }\nf() + 1",
            "mismatched types",
            "fn() -> Int | Future<Int>",
        ),
    ] {
        assert_eq!(messages(text), vec![message], "{text}");
        assert_eq!(fn_type(text, 0), shown, "{text}");
    }
    // And with an erroneous parameter or result type, erroneous itself.
    assert_eq!(fn_type("fn f(x: Nope) -> Int { 1 }", 0), "<error>");
}

#[test]
fn an_impure_function_nested_in_a_pure_one_is_settled() {
    // A parameter of the pure function, returned unmeasured, may be a
    // future for all the nested body knows, whether or not the pure
    // function measures it elsewhere.
    for text in [
        "fn outer(n: Int): fn() -> Int { fn() -> Int { n } }",
        "fn outer(n: Int): fn() -> Int { let m = n + 1\nfn() -> Int { n } }",
    ] {
        assert_eq!(fn_type(text, 0), "fn() -> Int | Future<Int>", "{text}");
        assert!(check(&parse_text(text), &mut TypeTable::new()).is_ok());
    }
    assert_eq!(
        type_of_f("fn f(n: Int): fn() -> Int { fn() -> Int { n } }"),
        "fn(n: Int | Future<Int>): fn() -> Int | Future<Int>"
    );
    // Measured in the nested body, it is awaited there, and present.
    let text = "fn outer(n: Int): fn() -> Int { fn() -> Int { n + 1 } }";
    assert_eq!(fn_type(text, 0), "fn() -> Int");
    assert_eq!(marks(text), vec![("n", AwaitMark::Top)]);
}

#[test]
fn an_inferred_result_is_a_subtype_of_the_declared_one() {
    // A looser written type is satisfied as before, and a tighter one
    // now too.
    assert!(
        check(
            &parse_text(
                "let f: fn() -> Int = fn(): Int { 5 }\nlet g: fn() -> Int = fn() -> Int { 5 }"
            ),
            &mut TypeTable::new()
        )
        .is_ok()
    );
    assert_eq!(
        shown_result("let g: fn() -> Present<Int> = fn() -> Int { 5 }\ng()"),
        "Int"
    );
    assert_eq!(
        messages("let g: fn() -> Present<Int> = fn() -> Int { lazy 5 }"),
        vec!["mismatched types"]
    );
    // A future of the call's present record satisfies a written future.
    let get = "fn get() -> {a: Int, b: String} { a = 1; b = \"x\" }\n";
    assert_eq!(
        shown_result(&format!(
            "{get}let f: Future<{{a: Int}}> = lazy get()\n(await f).a"
        )),
        "Int"
    );
}

/// The type of the expression of `text`, which must check, that spans
/// `span`, as shown.
pub(super) fn type_at(text: &str, span: Span) -> String {
    let (ast, types, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let id = ast
        .expr_ids()
        .find(|&id| ast.get(id).span == span)
        .unwrap_or_else(|| panic!("no expression at {span:?} in {text:?}"));
    types
        .display(output.checked.types.get(id), ast.names())
        .to_string()
}

/// For each dotted `let` and `:=` in the body of function number `n` of
/// `text`, which must check, or in the file's body if `n` is `None`, in
/// order: how many records along its path exist, and whether each of
/// them is awaited.
pub(super) fn awaited_paths(text: &str, n: Option<u32>) -> Vec<(u32, Vec<bool>)> {
    let (ast, resolution) = resolved(text);
    let body = match n {
        None => ast.root(),
        Some(n) => match ast.get(ast.fn_decl(FnId::new(n)).body).kind {
            ExprKind::Block { items, .. } => items,
            other => panic!("the body is not a block: {other:?}"),
        },
    };
    ast.items(body)
        .iter()
        .filter(|&&id| {
            matches!(
                ast.item(id).kind,
                ItemKind::Let { path, .. } | ItemKind::Rebind { path, .. }
                    if ast.path(path).len() > 1
            )
        })
        .map(|&id| {
            let path = resolution.item_path(id);
            (
                path.existing,
                resolution.path_awaits(path).unwrap().to_vec(),
            )
        })
        .collect()
}

/// The first `n` bytes of `span`.
pub(super) fn first_bytes(span: Span, n: u32) -> Span {
    Span::new(span.start, span.start + n)
}

#[test]
fn a_dotted_update_awaits_a_record_that_may_be_a_future() {
    // A result written as the either shape: its record is awaited at
    // the top, and the updated variable is present. `:=` keeps the
    // field's type, and `let` replaces it.
    let get = "fn get() -> {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}> { a = 1 }\n\
                   let r = get()\n";
    let text = format!("{get}r.a := 2\nr");
    assert_eq!(shown_result(&text), "{a: Int | Future<Int>}");
    assert_eq!(awaited_paths(&text, None), vec![(1, vec![true])]);
    // The await is recorded with the path, not on any expression.
    assert!(marks(&text).is_empty());
    let text = format!("{get}let r.a = 2\nr");
    assert_eq!(shown_result(&text), "{a: Int}");
    assert_eq!(awaited_paths(&text, None), vec![(1, vec![true])]);
    // An either-typed leaf accepts a future as well as a present value.
    let text = format!("{get}r.a := lazy 2\nr");
    assert_eq!(shown_result(&text), "{a: Int | Future<Int>}");
    // A future of a record, whose fields are present.
    let lazy = "let r = lazy { a = 1 }\n";
    for update in ["r.a := 2", "let r.a = 2"] {
        let text = format!("{lazy}{update}\nr");
        assert_eq!(shown_result(&text), "{a: Int}", "{text}");
        assert_eq!(awaited_paths(&text, None), vec![(1, vec![true])], "{text}");
    }
    // A field added to it, and one added under a field it lacks.
    assert_eq!(
        shown_result(&format!("{lazy}let r.b = \"s\"\nr")),
        "{a: Int, b: String}"
    );
    let text = format!("{lazy}let r.b.c = 1\nr");
    assert_eq!(shown_result(&text), "{a: Int, b: {c: Int}}");
    assert_eq!(awaited_paths(&text, None), vec![(1, vec![true])]);
    // A present record awaits nothing, and has no entries.
    let text = "let r = { a = { b = 1 } }\nlet r.a.b = 2\nr.a.b := 3";
    assert_eq!(
        awaited_paths(text, None),
        vec![(2, Vec::new()), (2, Vec::new())]
    );
}

#[test]
fn a_dotted_update_awaits_every_record_along_its_path() {
    // An impure function's parameter, the either shape at every level:
    // both `conf` and `conf.a` are awaited, and every record along the
    // path is present afterwards.
    for (update, shown) in [
        ("let conf.a.b = 2", "{a: {b: Int}}"),
        ("conf.a.b := 2", "{a: {b: Int | Future<Int>}}"),
    ] {
        let text = format!("fn f(conf: {{a: {{b: Int}}}}) -> Int {{\n  {update}\n  conf.a.b\n}}");
        assert_eq!(
            awaited_paths(&text, Some(0)),
            vec![(2, vec![true, true])],
            "{text}"
        );
        let conf = last(&text, "conf");
        assert_eq!(type_at(&text, conf), shown, "{text}");
        // The access after the update awaits nothing but, for `:=`, the
        // field it returns, which is passed on as the result.
        assert!(marks(&text).is_empty(), "{text}");
    }
    // Only the records that may be futures are awaited.
    let text = "let r = { a = lazy { b = 1 }; c = 2 }\nlet r.a.b = 3\nr";
    assert_eq!(shown_result(text), "{a: {b: Int}, c: Int}");
    assert_eq!(awaited_paths(text, None), vec![(2, vec![false, true])]);
    // Through a future of a future.
    let text = "let r = lazy { a = lazy { b = 1; c = 2 } }\nr.a.b := 5\nr";
    assert_eq!(shown_result(text), "{a: {b: Int, c: Int}}");
    assert_eq!(awaited_paths(text, None), vec![(2, vec![true, true])]);
}

#[test]
fn a_pure_function_cannot_update_a_record_that_may_be_a_future() {
    // Reported once, at the path up to the record, for `let` and `:=`.
    for update in ["let r.a = 2", "r.a := 2"] {
        let text = format!("fn f(r: Future<{{a: Int}}>): {{a: Int}} {{ {update}\nr }}");
        assert_implicit_await_error(&text, first_bytes(at(&text, "r.a"), 1), "Future<{a: Int}>");
    }
    // A record further along the path: the parameter itself is measured,
    // which makes it present, and its field may be a future.
    let text = "fn f(r: {a: Future<{b: Int}>}): Int { let r.a.b = 2\nr.a.b }";
    assert_implicit_await_error(text, at(text, "r.a"), "Future<{b: Int}>");
    assert_eq!(demands(text, 0), vec![Some("r")]);
    // A record a pure closure captures, that can be awaited before the
    // closure is created: see
    // `the_variable_a_dotted_update_reads_gets_the_capture_hint`.
    let text = "let r = lazy { a = 1 }\nlet g = fn(): {a: Int} { let r.a = 2\nr }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].span, first_bytes(last(text, "r.a"), 1));
    assert_eq!(found[0].notes, vec![capture_note("r")]);
}

#[test]
fn a_dotted_update_measures_every_parameter_along_its_path() {
    // The updated variable, by `:=` as by `let`.
    let text = "fn f(r: {a: Int}): {a: Int} { r.a := 2\nr }";
    assert_eq!(demands(text, 0), vec![Some("r")]);
    assert_eq!(type_of_f(text), "fn(r: {a: Int}): {a: Int}");
    assert!(marks(text).is_empty());
    // A parameter held in a field of another variable, measured where
    // the update reads it, and nowhere else.
    for update in ["let q.inner.x = 2", "q.inner.x := 2"] {
        let text = format!("fn f(p: {{x: Int}}): Int {{ let q = {{ inner = p }}\n{update}\n1 }}");
        assert_eq!(demands(&text, 0), vec![Some("q.inner")], "{text}");
        assert_eq!(type_of_f(&text), "fn(p: {x: Int}): Int", "{text}");
        assert!(marks(&text).is_empty(), "{text}");
        assert_eq!(
            awaited_paths(&text, Some(0)),
            vec![(2, Vec::new())],
            "{text}"
        );
    }
    // Not the leaf, which is replaced, not read.
    let text = "fn f(p: {x: Int}): Int { let q = { inner = p }\nlet q.inner = 2\n1 }";
    assert_eq!(demands(text, 0), vec![None]);
    assert_eq!(type_of_f(text), "fn(p: {x: Int} | Future<{x: Int}>): Int");
}

#[test]
fn a_record_that_is_not_one_once_awaited_cannot_be_updated() {
    // Shown as it was before the await.
    for update in ["let r.a.b = 1", "r.a.b := 1"] {
        let text = format!("let r = {{ a = lazy 1 }}\n{update}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(
            found[0].message,
            "cannot update `r.a.b`: `r.a` is `Future<Int>`, not a record"
        );
        assert_eq!(found[0].span, at(&text, "r.a.b"));
        assert_eq!(
            labels(&found[0]),
            vec![(
                first_bytes(at(&text, "r.a.b"), 1),
                "this is {a: Future<Int>}"
            )]
        );
    }
    assert_eq!(
        messages("let r = lazy 1\nlet r.a = 2"),
        vec!["cannot update `r.a`: `r` is `Future<Int>`, not a record"]
    );
    // In a pure function too, alone: the update cannot work whether or
    // not the value is awaited, so the await is not reported.
    for update in ["let r.a.b = 1", "r.a.b := 1"] {
        let text = format!("fn f(r: {{a: Future<Int>}}): Int {{ {update}\n1 }}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message,
            "cannot update `r.a.b`: `r.a` is `Future<Int>`, not a record"
        );
        assert_eq!(found[0].span, at(&text, "r.a.b"));
        assert_eq!(
            labels(&found[0]),
            vec![(
                first_bytes(at(&text, "r.a.b"), 1),
                "this is {a: Future<Int>}"
            )]
        );
    }
    // The variable itself, too.
    assert_eq!(
        messages("fn f(r: Future<Int>): Int { let r.a = 1\n1 }"),
        vec!["cannot update `r.a`: `r` is `Future<Int>`, not a record"]
    );
    // A record that may be a future is still reported, once.
    for update in ["let r.a.b = 1", "r.a.b := 1"] {
        let text = format!("fn f(r: {{a: Future<{{b: Int}}>}}): Int {{ {update}\n1 }}");
        assert_implicit_await_error(&text, at(&text, "r.a"), "Future<{b: Int}>");
    }
    // `:=` still adds no field to a record that may be a future.
    let text = "let r = lazy { a = 1 }\nr.b := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "cannot rebind `r.b`: `r` has no field `b`"
    );
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "r.b"), 1), "this is Future<{a: Int}>")]
    );
    assert_eq!(
        messages("let r = lazy { a = 1 }\nr.b.c := 2"),
        vec!["cannot rebind `r.b.c`: `r` has no field `b`"]
    );
}

/// The note of the implicit-await error for a variable `name` that a
/// pure function captures and that can be awaited before it is created.
pub(super) fn capture_note(name: &str) -> String {
    format!(
        "a pure function cannot await, and `{name}` may be a future; await it before the \
             function is created, in the enclosing impure code: `let {name} = await {name}`"
    )
}

/// The notes of the one error of `text`, which must be the implicit-await
/// error.
pub(super) fn implicit_await_notes(text: &str) -> Vec<String> {
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{text}: {found:?}");
    assert_eq!(
        found[0].message, "cannot use a value that may be a future in a pure function",
        "{text}"
    );
    found[0].notes.clone()
}

#[test]
fn a_captured_value_that_can_be_awaited_outside_gets_a_hint() {
    // Through a `lazy`, which is impure, from the pure function whose
    // parameter it is.
    let text = "fn outer(n: Int): Future<Int> {\n\
                    \x20 lazy {\n\
                    \x20   let g = fn(): Int { n + 1 }\n\
                    \x20   = g()\n\
                    \x20 }\n\
                    }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("n")]);
    let found = diagnostics(text);
    assert_eq!(found[0].span, first_bytes(last(text, "n + 1"), 1));
    // From the top level, which is impure; through parentheses; as the
    // record of a field access.
    for text in [
        "let n = lazy 1\nlet g = fn(): Int { n + 1 }",
        "let n = lazy 1\nlet g = fn(): Int { (n) + 1 }",
        "let n = lazy { a = 1 }\nlet g = fn(): Int { n.a + 1 }",
    ] {
        assert_eq!(
            implicit_await_notes(text),
            vec![capture_note("n")],
            "{text}"
        );
    }
    // From an impure function, with nothing between.
    let text = "fn f(n: Int) -> fn(): Int { fn(): Int { n + 1 } }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("n")]);
    // As the result of the pure function's body: the body, a block, is
    // what is awaited against the result type.
    for text in [
        "let n = lazy 1\nlet g = fn(): Int { n }",
        "let n = lazy 1\nlet g = fn(): Int { = (n) }",
    ] {
        assert_eq!(
            implicit_await_notes(text),
            vec![capture_note("n")],
            "{text}"
        );
    }
    // As an argument against a parameter written present.
    let text = "fn h(x: Present<Int>): Int { x }\nlet n = lazy 1\nlet g = fn(): Int { h(n) }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("n")]);
    // The suggested fix checks.
    let text = "fn outer(n: Int): Future<Int> {\n\
                    \x20 lazy {\n\
                    \x20   let n = await n\n\
                    \x20   let g = fn(): Int { n + 1 }\n\
                    \x20   = g()\n\
                    \x20 }\n\
                    }\nouter";
    assert_eq!(shown_result(text), "fn(n: Int | Future<Int>): Future<Int>");
}

#[test]
fn a_value_that_cannot_be_awaited_outside_gets_the_usual_note() {
    for text in [
        // A pure function's own parameter.
        "fn f(n: Future<Int>): Int { n + 1 }",
        // A capture through pure functions only.
        "fn f(n: Future<Int>): fn(): Int { fn(): Int { n + 1 } }",
        // A capture whose home is pure, though an impure function
        // encloses that: the home cannot await it either.
        "fn g() -> Int {\n  let h = fn(n: Future<Int>): fn(): Int { fn(): Int { n + 1 } }\n  1\n}",
        // A field of a captured record, which is not a plain name.
        "let r = { a = lazy 1 }\nlet g = fn(): Int { r.a + 1 }",
        // A variable of the pure function's own body.
        "fn f(): Int { let n = lazy 1\nn + 1 }",
        // A block whose result is a variable of its own.
        "fn f(): Int { let n = lazy 1\n= n }",
    ] {
        assert_eq!(
            implicit_await_notes(text),
            vec![IMPLICIT_AWAIT_NOTE],
            "{text}"
        );
    }
}

#[test]
fn the_variable_a_dotted_update_reads_gets_the_capture_hint() {
    // The variable itself, a capture that can be awaited outside, by the
    // same conditions as for an expression.
    let text = "let r = lazy { a = 1 }\nlet g = fn(): {a: Int} { let r.a = 2\nr }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("r")]);
    let found = diagnostics(text);
    assert_eq!(found[0].span, first_bytes(last(text, "r.a"), 1));
    assert_eq!(found[0].label.as_deref(), Some("this is Future<{a: Int}>"));
    // From an impure function, with nothing between.
    let text = "fn f(r: Future<{a: Int}>) -> fn(): Int { fn(): Int { let r.a = 2\n1 } }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("r")]);
    for text in [
        // A record further along the path.
        "let r = { a = lazy { b = 1 } }\nlet g = fn(): Int { let r.a.b = 2\n1 }",
        // A capture whose home is pure, reached through pure functions
        // only.
        "fn f(r: Future<{a: Int}>): fn(): Int { fn(): Int { let r.a = 2\n1 } }",
        // The function's own parameter.
        "fn f(r: Future<{a: Int}>): Int { let r.a = 2\n1 }",
        "fn f(r: Future<{a: Int}>): Int { r.a := 2\n1 }",
    ] {
        assert_eq!(
            implicit_await_notes(text),
            vec![IMPLICIT_AWAIT_NOTE],
            "{text}"
        );
    }
}
