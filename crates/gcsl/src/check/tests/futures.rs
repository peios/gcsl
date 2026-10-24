//! Tests of the checker: the bare-type rule, `lazy`, awaits and futures where
//! values are expected.

use super::*;

/// Every expression of `text`, which must check, that is awaited where
/// it is used, as its source text and its mark, in allocation order.
pub(super) fn marks(text: &str) -> Vec<(&str, AwaitMark)> {
    let (ast, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let resolution = output.checked.resolution;
    ast.expr_ids()
        .filter_map(|id| match resolution.await_of(id) {
            AwaitMark::None => None,
            mark => {
                let span = ast.get(id).span;
                Some((&text[span.start as usize..span.end as usize], mark))
            }
        })
        .collect()
}

/// The note of the error for an implicit await in a pure function.
pub(super) const IMPLICIT_AWAIT_NOTE: &str = "using the value here would await it, which may run \
                                       effects; a pure function may hold and pass on a future \
                                       but not await it. Declare the function with `->` to \
                                       make it impure";

/// Checks that `text` has exactly one error, the error for an implicit
/// await in a pure function of `awaited`, whose type is shown as
/// `shown`, the function being the last declared with `:`.
pub(super) fn assert_implicit_await_error(text: &str, awaited: Span, shown: &str) {
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{text}: {found:?}");
    let error = &found[0];
    assert_eq!(
        error.message, "cannot use a value that may be a future in a pure function",
        "{text}"
    );
    assert_eq!(error.span, awaited, "{text}");
    assert_eq!(
        error.label.as_deref(),
        Some(format!("this is {shown}").as_str()),
        "{text}"
    );
    let marker = last(text, "):");
    let marker = Span::new(marker.start + 1, marker.end);
    assert_eq!(labels(error), vec![(marker, PURE_CONTEXT)], "{text}");
    assert_eq!(error.notes, vec![IMPLICIT_AWAIT_NOTE], "{text}");
}

#[test]
fn bare_types_are_either_in_impure_signatures_and_present_elsewhere() {
    // An impure function's parameters and result, a record type being
    // the either shape at its own position and through its fields; the
    // result's bare positions then take their laziness from the body
    // (see `an_impure_functions_bare_result_is_inferred`).
    assert_eq!(
        shown_result("fn(x: Int, r: {a: String}) -> {v: Int} { v = x }"),
        "fn(x: Int | Future<Int>, r: {a: String | Future<String>} | Future<{a: String | \
             Future<String>}>) -> {v: Int | Future<Int>}"
    );
    assert_eq!(
        shown_result("fn f(r: {a: String}) -> {v: Int} { v = 1 }\nf"),
        "fn(r: {a: String | Future<String>} | Future<{a: String | Future<String>}>) -> {v: \
             Int}"
    );
    assert_eq!(
        shown_result("fn(r: {a: {b: Int}}) -> Int { 1 }"),
        "fn(r: {a: {b: Int | Future<Int>} | Future<{b: Int | Future<Int>}>} | Future<{a: \
             {b: Int | Future<Int>} | Future<{b: Int | Future<Int>}>}>) -> Int"
    );
    // But not a function type, at its own position: it has its own rule
    // inside, and a function that may be a future is written so.
    assert_eq!(
        shown_result("fn(f: fn(): Int, r: {g: fn(): Int}) -> Int { 1 }"),
        "fn(f: fn(): Int, r: {g: fn(): Int} | Future<{g: fn(): Int}>) -> Int"
    );
    // A pure function's result is present, and so are its parameters
    // that its body measures; those it does not are inferred to be the
    // either shape (see `pure_parameters_are_inferred`).
    assert_eq!(
        shown_result("fn(x: Int, r: {a: String}): {v: Int} { v = x }"),
        "fn(x: Int, r: {a: String} | Future<{a: String}>): {v: Int}"
    );
    assert_eq!(
        shown_result("fn(x: Int, r: {a: String}): {v: Int} { v = x; w = r.a }"),
        "fn(x: Int, r: {a: String}): {v: Int}"
    );
    // A written impure function type's, and a pure one's; and each has
    // its own rule inside the other.
    assert_eq!(
        shown_result("fn(x: Int) -> {v: Int} { v = x } : fn(Int) -> {v: Int}"),
        "fn(Int | Future<Int>) -> {v: Int | Future<Int>} | Future<{v: Int | Future<Int>}>"
    );
    assert_eq!(
        shown_result("fn(x: Int): Int { x } : fn(Int): Int"),
        "fn(Int): Int"
    );
    assert_eq!(
        shown_result("fn(f: fn(Int): Int) -> Int { 1 }"),
        "fn(f: fn(Int): Int) -> Int"
    );
    assert_eq!(
        shown_result("fn(f: fn(Int) -> Int): Int { 1 }"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>): Int"
    );
    // Annotations and ascriptions are present.
    assert_eq!(shown_result("let x: {a: Int} = { a = 1 }\nx"), "{a: Int}");
    assert_eq!(shown_result("1 : Int"), "Int");
    // `Present<T>` and `Future<T>` override, their interiors present.
    assert_eq!(
        shown_result("fn(r: Present<{a: Int}>) -> Present<Int> { 1 }"),
        "fn(r: {a: Int}) -> Int"
    );
    assert_eq!(
        shown_result("fn(r: Future<{a: Int}>) -> Int { 1 }"),
        "fn(r: Future<{a: Int}>) -> Int"
    );
    assert_eq!(
        shown_result("fn(r: Future<{a: Int}>): Future<{a: Int}> { r }"),
        "fn(r: Future<{a: Int}>): Future<{a: Int}>"
    );
    // The either shape may be written anywhere.
    assert_eq!(
        shown_result("let x: Int | Future<Int> = 1\nx"),
        "Int | Future<Int>"
    );
    assert_eq!(
        shown_result("let x: Future<Int> | Int = lazy 1\nx"),
        "Int | Future<Int>"
    );
    // The two functions of the specification; `keep`'s parameter,
    // written as the either shape here, is inferred to be one when it is
    // written bare too.
    assert_eq!(
        shown_result("fn double(x: Int): Int { x * 2 }\ndouble"),
        "fn(x: Int): Int"
    );
    assert_eq!(
        shown_result("fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }\nkeep"),
        "fn(x: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    assert_eq!(
        shown_result("fn keep(x: Int): {v: Int | Future<Int>} { v = x }\nkeep"),
        "fn(x: Int | Future<Int>): {v: Int | Future<Int>}"
    );
}

#[test]
fn an_impure_function_takes_a_future_record() {
    let run = "fn run(step: {}) -> {} { }\n";
    assert_eq!(
        shown_result(&format!("{run}run")),
        "fn(step: {} | Future<{}>) -> {}"
    );
    // A future of a record is passed as it is, unawaited.
    let text = format!("{run}run(lazy {{}})");
    assert_eq!(shown_result(&text), "{}");
    assert!(marks(&text).is_empty());
    let text = format!("{run}run({{}})");
    assert!(marks(&text).is_empty());
}

#[test]
fn an_either_record_parameter_is_awaited_by_a_field_access() {
    let text = "fn f(r: {a: Int}) -> Int { r.a }\nf(lazy { a = 1 })";
    // The record is awaited at the top; its field, of the either shape,
    // is passed on as the result unawaited.
    assert_eq!(marks(text), vec![("r", AwaitMark::Top)]);
    assert_eq!(shown_result(text), "Int | Future<Int>");
    // Measured, the field is awaited in turn.
    assert_eq!(
        marks("fn f(r: {a: Int}) -> Int { r.a + 1 }"),
        vec![("r", AwaitMark::Top), ("r.a", AwaitMark::Top)]
    );
}

#[test]
fn a_pure_function_record_parameter_stays_present() {
    let text = "fn f(r: {a: Int}): Int { r.a + 1 }\nf";
    assert_eq!(shown_result(text), "fn(r: {a: Int}): Int");
    assert!(marks(text).is_empty());
    // So a future of one is awaited at the call, in impure code.
    assert_eq!(
        marks("fn f(r: {a: Int}): Int { r.a }\nf(lazy { a = 1 })"),
        vec![("lazy { a = 1 }", AwaitMark::Top)]
    );
}

#[test]
fn a_written_function_type_record_parameter_follows_its_marker() {
    assert_eq!(
        shown_result("fn(r: {a: Int}) -> Int { r.a } : fn({a: Int}) -> Int"),
        "fn({a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn(r: {a: Int}): Int { r.a } : fn({a: Int}): Int"),
        "fn({a: Int}): Int"
    );
    // In an annotation, whose own bare types are present.
    assert_eq!(
        shown_result("let g: {f: fn({a: Int}) -> {}} = { f = fn(r: {a: Int}) -> {} { } }\ng"),
        "{f: fn({a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> {} | Future<{}>}"
    );
    // A pure function taking a present record does not fit the impure
    // type, which promises it may pass a future.
    assert_eq!(
        messages("let g: fn({a: Int}) -> Int = fn(r: {a: Int}): Int { r.a }"),
        vec!["mismatched types"]
    );
}

#[test]
fn an_either_function_type_displays_in_parentheses_and_round_trips() {
    let branches = "if true { fn(): Int { 1 } } else { lazy fn(): Int { 2 } }";
    assert_eq!(shown_result(branches), "(fn(): Int) | Future<fn(): Int>");
    let annotated = format!("let f: (fn(): Int) | Future<fn(): Int> = {branches}\nf");
    assert_eq!(shown_result(&annotated), "(fn(): Int) | Future<fn(): Int>");
    // Without the parentheses it is a function returning a union.
    assert_eq!(
        messages("let f: fn(): Int | Future<fn(): Int> = g"),
        vec![
            "a union with a `Future` member must be `T | Future<T>`",
            "undefined variable `g`"
        ]
    );
}

#[test]
fn a_lazy_is_a_future_of_its_body() {
    // A literal settles to Int.
    assert_eq!(shown_result("lazy 1"), "Future<Int>");
    assert_eq!(shown_result("lazy 1 + 2.5"), "Future<Float>");
    assert_eq!(shown_result("lazy { let a = 1; a + 1 }"), "Future<Int>");
    assert_eq!(
        shown_result("lazy { a = 1; b = \"s\" }"),
        "Future<{a: Int, b: String}>"
    );
    assert_eq!(shown_result("lazy { }"), "Future<{}>");
    assert_eq!(shown_result("lazy fn(): Int { 1 }"), "Future<fn(): Int>");
    // A future of a future is a future, since awaiting resolves fully;
    // so is a future of an either.
    assert_eq!(shown_result("lazy lazy 1"), "Future<Int>");
    assert_eq!(
        shown_result("let x: Int | Future<Int> = 1\nlazy x"),
        "Future<Int>"
    );
    assert_eq!(
        shown_result("let x: Future<Future<Int>> = lazy 1\nx"),
        "Future<Int>"
    );
    assert_eq!(
        shown_result("let x: Future<Int | Future<Int>> = lazy lazy 1\nx"),
        "Future<Int>"
    );
    // An impure function's bare result is the either shape, and a lazy
    // of its call fits a declared future.
    let compile = "fn compile(target: String) -> String { \"built {target}\" }\n";
    assert_eq!(
        shown_result(&format!("{compile}lazy compile(\"a\")")),
        "Future<String>"
    );
    assert_eq!(
        shown_result(&format!(
            "{compile}let build: Future<String> = lazy compile(\"a\")\nbuild"
        )),
        "Future<String>"
    );
    // The fields of a record it yields are untouched.
    assert_eq!(
        shown_result("lazy { a = lazy 1 }"),
        "Future<{a: Future<Int>}>"
    );
    let ast = parse_text("lazy 1");
    let checked = check(&ast, &mut TypeTable::new()).unwrap().0;
    let ExprKind::Lazy(function) = ast.get(result_expr(&ast)).kind else {
        panic!("expected a lazy");
    };
    assert_eq!(checked.types.get(ast.fn_decl(function).body), TyId::INT);
    // An erroneous body is reported once.
    assert_eq!(
        messages("let x = lazy 1 + \"a\"\nx + 1"),
        vec!["cannot add Int and String"]
    );
    // The name it initialises is not in scope in it.
    assert_eq!(
        errors("let x = lazy x"),
        vec![("undefined variable `x`".to_owned(), Span::new(13, 14))]
    );
}

#[test]
fn a_lazy_captures_as_a_closure_does() {
    // A top-level variable, in a frame of its own.
    let text = "let x = 1\nlet f = lazy { let y = x; y + x }";
    let (ast, resolution) = resolved(text);
    assert_eq!(
        uses(&ast, &resolution, "x"),
        vec![VarRef::Capture(0), VarRef::Capture(0)]
    );
    assert_eq!(uses(&ast, &resolution, "y"), vec![VarRef::Local(Slot(0))]);
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Local(Slot(0)), "Int".to_owned())]
    );
    let function = resolution.function(FnId::new(0));
    assert_eq!(
        (function.frame_size, function.param_count, function.callee),
        (1, 0, None)
    );
    // A parameter, through a function nested in the body: the lazy is
    // number 1, finished after the function nested in it, number 0, and
    // before the function it is in, number 2.
    let text = "fn f(n: Present<Int>) -> Future<fn(): Int> { lazy fn(): Int { n + 1 } }";
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Capture(0), "Int".to_owned())]
    );
    assert_eq!(
        captures(text, 1),
        vec![(VarRef::Local(Slot(0)), "Int".to_owned())]
    );
    // And the callee of the function it is in; `n` is passed back to
    // it unmeasured, so it is the either shape.
    let text = "fn f(n: Int): Future<Int> { lazy await f(n) }";
    assert_eq!(
        captures(text, 0),
        vec![
            (
                VarRef::Callee,
                "fn(n: Int | Future<Int>): Future<Int>".to_owned()
            ),
            (VarRef::Local(Slot(0)), "Int | Future<Int>".to_owned())
        ]
    );
    // A lazy in a function's initialiser does not see the function's
    // name, and a function in the lazy does not either.
    assert_eq!(
        messages("let f = lazy fn(): Int { f() }"),
        vec!["undefined variable `f`"]
    );
    // `:=` on a captured variable is rebinding one of an enclosing
    // block.
    assert_eq!(
        messages("let x = 1\nlet f = lazy { x := 2\nx }"),
        vec!["cannot rebind `x` here: it is declared in an enclosing block"]
    );
}

#[test]
fn a_lazy_body_is_impure_wherever_it_is_written() {
    let tick = "fn tick() -> Present<Int> { 1 }\nfn step() -> {} { }\n";
    for text in [
        format!("{tick}fn plan(): Future<Int> {{ lazy tick() }}"),
        format!("{tick}fn plan(): {{build: Future<{{}}>}} {{ build = lazy {{ step() }} }}"),
        "fn plan(x: Future<Int>): Future<Int> { lazy await x + 1 }".to_owned(),
        "fn plan(x: Future<Int>): Future<Int> { lazy x + 1 }".to_owned(),
        "fn plan(x: Future<Int>): Future<String> { lazy \"{x}\" }".to_owned(),
    ] {
        assert_eq!(
            check(&parse_text(&text), &mut TypeTable::new()).err(),
            None,
            "{text}"
        );
    }
    // A pure function in the lazy is pure.
    assert_eq!(
        messages(&format!("{tick}let p = lazy fn(): Int {{ tick() }}")),
        vec!["cannot call an impure function from a pure function"]
    );
}

#[test]
fn await_resolves_its_operand_fully() {
    assert_eq!(shown_result("let x = lazy 1\nawait x"), "Int");
    assert_eq!(shown_result("let x = lazy lazy 1\nawait x"), "Int");
    assert_eq!(shown_result("let x: Int | Future<Int> = 1\nawait x"), "Int");
    // The fields of a record keep their declared types.
    assert_eq!(
        shown_result("let r = lazy { a = lazy 1 }\nawait r"),
        "{a: Future<Int>}"
    );
    assert_eq!(shown_result("let x = lazy 2\nawait x + 1"), "Int");
    assert_eq!(
        marks("let x = lazy lazy 1\nawait x"),
        vec![("x", AwaitMark::Top)]
    );
    // Accepted at the top level, in an impure function and in a lazy.
    assert_eq!(
        shown_result("fn f(x: Int) -> Present<Int> { await x }\nf"),
        "fn(x: Int | Future<Int>) -> Int"
    );
}

#[test]
fn awaiting_a_present_value_does_nothing() {
    let text = "let x = 1\nawait x";
    assert!(warnings(text).is_empty());
    assert_eq!(shown_result(text), "Int");
    assert!(marks(text).is_empty());
    // A call's result that is inferred to be present.
    let text = "fn tick() -> Int { 1 }\nawait tick()";
    assert!(warnings(text).is_empty());
    assert!(marks(text).is_empty());
    // In a pure function too: nothing is awaited.
    assert!(warnings("fn f(): Int { let x = 1\nawait x }").is_empty());
    // An untyped operand stays untyped, and adapts.
    assert_eq!(shown_result("await 5 + 1.5"), "Float");
    assert!(warnings("await 5").is_empty());
}

#[test]
fn a_pure_function_cannot_await() {
    // Explicitly.
    let text = "fn f(x: Future<Int>): Int { await x }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "cannot await in a pure function");
    assert_eq!(found[0].span, at(text, "await"));
    assert_eq!(labels(&found[0]), vec![(nth(text, ":", 1), PURE_CONTEXT)]);
    assert_eq!(
        found[0].notes,
        vec![
            "awaiting runs a deferred expression, which may have effects; declare the \
                 function with `->` to make it impure"
        ]
    );
    // Implicitly, by each measuring operator.
    let either = "Int | Future<Int>";
    for (body, ret) in [
        ("x + 1", "Int"),
        ("1 - x", "Int"),
        ("x * 2", "Int"),
        ("x / 2", "Float"),
        ("x // 2", "Int"),
        ("x % 2", "Int"),
        ("-x", "Int"),
        ("x < 1", "Bool"),
        ("x >= 1", "Bool"),
    ] {
        let text = format!("fn f(x: Int | Future<Int>): {ret} {{ {body} }}");
        assert_implicit_await_error(&text, last(&text, "x"), either);
    }
    for body in ["b and true", "false or b", "not b"] {
        let text = format!("fn f(b: Bool | Future<Bool>): Bool {{ {body} }}");
        assert_implicit_await_error(&text, last(&text, "b"), "Bool | Future<Bool>");
    }
    // An `if` condition.
    let text = "fn f(b: Future<Bool>): Int { if b { 1 } else { 2 } }";
    assert_implicit_await_error(text, last(text, "b"), "Future<Bool>");
    // A field access, and a call of the value.
    let text = "fn f(r: Future<{a: Int}>): Int { r.a }";
    assert_implicit_await_error(text, last(text, "r"), "Future<{a: Int}>");
    let text = "fn f(g: Future<fn(): Int>): Int { g() }";
    assert_implicit_await_error(text, last(text, "g"), "Future<fn(): Int>");
    // An argument against a parameter written present. (One present by
    // inference has the call-site form: see
    // `the_call_site_form_of_the_implicit_await_error`.)
    let text = "fn h(n: Present<Int>): Int { n }\nfn f(x: Future<Int>): Int { h(x) }";
    assert_implicit_await_error(text, last(text, "x"), "Future<Int>");
    // A body against a present return type.
    let text = "fn f(x: Int | Future<Int>): Int { x }";
    assert_implicit_await_error(text, last(text, "{ x }"), either);
    // A record field against a present field, deeply.
    let text = "fn f(r: {a: Future<Int>}): {a: Int} { r }";
    assert_implicit_await_error(text, last(text, "{ r }"), "{a: Future<Int>}");
    // An interpolation and an equality, at any depth of blocks.
    let text = "fn f(x: Future<Int>): String { { { \"{x}\" } } }";
    assert_implicit_await_error(text, last(text, "x"), "Future<Int>");
    let text = "fn f(x: Future<Int>): Bool { if true { x == 1 } else { false } }";
    assert_implicit_await_error(text, last(text, "x"), "Future<Int>");
    let text = "fn f(r: {a: Int | Future<Int>}): Bool { r == r }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].span, nth(text, "r", 2));
    assert_eq!(found[1].span, nth(text, "r", 3));
    assert_eq!(
        found[0].label.as_deref(),
        Some("this is {a: Int | Future<Int>}")
    );
    // A pure function nested in an impure one is pure; a pure
    // function's own bare parameter is measured, which makes it present,
    // so measuring it is fine.
    assert_eq!(
        messages("fn f(x: Int) -> Int { let g = fn(): Int { x + 1 }\ng() }"),
        vec!["cannot use a value that may be a future in a pure function"]
    );
    assert!(marks("fn f(x: Int): Int { x + 1 }").is_empty());
}

#[test]
fn a_pure_function_may_hold_and_pass_on_futures() {
    for text in [
        "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }",
        "fn pass(x: Future<Int>): Future<Int> { x }",
        "fn pass(x: Future<Int>): Int | Future<Int> { x }",
        "fn bind(x: Future<Int>): Future<Int> { let y = x\ny }",
        "fn store(x: Future<Int>): {f: Future<Int>} { f = x }",
        "fn later(x: Future<Int>): fn() -> Int { fn() -> Int { x } }",
        "fn g(y: Future<Int>): Future<Int> { y }\nfn f(x: Future<Int>): Future<Int> { g(x) }",
        "fn g(y: Int | Future<Int>): Int | Future<Int> { y }\n\
             fn f(x: Future<Int>): Int | Future<Int> { g(x) }",
        "fn pick(b: Bool, x: Future<Int>): Int | Future<Int> { if b { x } else { 1 } }",
    ] {
        let (ast, resolution) = resolved(text);
        assert!(
            ast.expr_ids()
                .all(|id| resolution.await_of(id) == AwaitMark::None),
            "{text}"
        );
    }
}

#[test]
fn the_same_programs_await_when_impure() {
    // The programs `a_pure_function_cannot_await` rejects, impure: each
    // use as bits is marked, and nothing is an error.
    for (body, ret) in [
        ("x + 1", "Int"),
        ("1 - x", "Int"),
        ("x * 2", "Int"),
        ("x / 2", "Float"),
        ("x // 2", "Int"),
        ("x % 2", "Int"),
        ("-x", "Int"),
        ("x < 1", "Bool"),
        ("x >= 1", "Bool"),
        ("\"{x}\"", "String"),
        ("x == 1", "Bool"),
    ] {
        let text = format!("fn f(x: Future<Int>) -> {ret} {{ {body} }}");
        assert_eq!(marks(&text), vec![("x", AwaitMark::Top)], "{text}");
        // And at the top level.
        let text = format!("let x = lazy 1\nlet y: {ret} = {body}");
        assert_eq!(marks(&text), vec![("x", AwaitMark::Top)], "{text}");
    }
    for (params, body, awaited) in [
        ("b: Future<Bool>", "if b { 1 } else { 2 }", "b"),
        ("b: Future<Bool>", "if not b { 1 } else { 2 }", "b"),
        ("r: Future<{a: Int}>", "r.a", "r"),
        ("g: Future<fn(): Int>", "g()", "g"),
        (
            "x: Int | Future<Int>",
            "{ { x } } : Present<Int>",
            "{ { x } }",
        ),
    ] {
        let text = format!("fn f({params}) -> Int {{ {body} }}");
        assert_eq!(marks(&text), vec![(awaited, AwaitMark::Top)], "{text}");
    }
    assert_eq!(
        marks("fn h(n: Int): Int { n }\nfn f(x: Future<Int>) -> Int { h(x) }"),
        vec![("x", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(r: {a: Future<Int>}) -> Present<{a: Int}> { r }"),
        vec![("{ r }", AwaitMark::Deep)]
    );
    // In an impure function, and at the top level, each use as bits is
    // marked.
    assert_eq!(
        marks("fn f(x: Int) -> Int { x + 1 }"),
        vec![("x", AwaitMark::Top)]
    );
    assert_eq!(
        marks("let x = lazy 1\nlet y = -x * 2\ny"),
        vec![("x", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(b: Bool, c: Bool) -> Bool { b and not c }"),
        vec![("b", AwaitMark::Top), ("c", AwaitMark::Top)]
    );
    assert_eq!(
        marks("let b = lazy true\nif b { 1 } else { 2 }"),
        vec![("b", AwaitMark::Top)]
    );
    assert_eq!(
        marks("let r = lazy { a = 1 }\nr.a"),
        vec![("r", AwaitMark::Top)]
    );
    assert_eq!(
        marks("let g = lazy fn(): Int { 1 }\ng()"),
        vec![("g", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn h(n: Int): Int { n }\nlet x = lazy 1\nh(x)"),
        vec![("x", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(x: Int) -> Present<Int> { x }"),
        vec![("{ x }", AwaitMark::Top)]
    );
    // Through parentheses and ascriptions, the outermost is marked.
    assert_eq!(
        marks("let x = lazy 1\n(x) + 1"),
        vec![("(x)", AwaitMark::Top)]
    );
    assert_eq!(
        marks("let x = lazy 1\nlet y: Int = x : Int | Future<Int>\ny"),
        vec![("x : Int | Future<Int>", AwaitMark::Top)]
    );
    // Deeply, where fields must be present, in interpolations and in
    // equalities; at the top where only the top is lazy.
    assert_eq!(
        marks("let r = { a = lazy 1; b = 2 }\nlet s: {a: Int} = r\ns"),
        vec![("r", AwaitMark::Deep)]
    );
    assert_eq!(
        marks("let r = { a = lazy 1 }\n\"{r}\""),
        vec![("r", AwaitMark::Deep)]
    );
    assert_eq!(
        marks("let x = lazy 1\n\"{x}\""),
        vec![("x", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(r: {a: Int}) -> Bool { r == { a = 1 } }"),
        vec![("r", AwaitMark::Deep)]
    );
    assert_eq!(marks("let x = lazy 1\nx == 1"), vec![("x", AwaitMark::Top)]);
    // A record with no lazy field inside is not awaited.
    assert!(marks("let r = { a = 1 }\nlet s = \"{r}\"\nr == r").is_empty());
}

#[test]
fn a_present_value_where_a_future_is_expected() {
    let note = "a present value does not satisfy `Future<Int>`; write `lazy` before the \
                    expression to defer it";
    // An annotation.
    let text = "let x: Future<Int> = 1";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "1"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Future<Int>, found Int")
    );
    assert_eq!(found[0].notes, vec![note]);
    // A parameter, a `:=` and a field.
    for text in [
        "fn f(x: Future<Int>): Future<Int> { x }\nf(1)",
        "let x = lazy 1\nx := 2",
        "let r: {f: Future<Int>} = { f = 1 }",
        "fn plan(): {build: Future<Int>} { build = 1 }",
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(found[0].message, "mismatched types", "{text}");
        assert_eq!(found[0].notes, vec![note], "{text}");
    }
    // An either where a future is expected.
    let text = "fn f(x: Int) -> Future<Int> { x }";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Future<Int>, found Int | Future<Int>")
    );
    assert_eq!(
        found[0].notes,
        vec!["this value may already be present; a `Future<Int>` must be deferred"]
    );
    // A future of another type: no note.
    let text = "let x: Future<Float> = lazy 1";
    let found = diagnostics(text);
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Future<Float>, found Future<Int>")
    );
    assert!(found[0].notes.is_empty());
    // A record block is checked against the present member of the
    // either shape of a record type, as against the record type.
    assert_eq!(
        shown_result("let r: {a: Float} | Future<{a: Float}> = { a = 1 }\nr"),
        "{a: Float} | Future<{a: Float}>"
    );
    assert_eq!(
        shown_result("fn() -> {a: Float} | Future<{a: Float}> { a = 1 }"),
        "fn() -> {a: Float} | Future<{a: Float}>"
    );
    assert_eq!(
        messages("let r: {a: Int} | Future<{a: Int}> = { b = 1 }"),
        vec![
            "field `b` is not in `{a: Int}`",
            "missing field `a` in record"
        ]
    );
    // The either shape accepts both, and adapts a literal.
    assert_eq!(
        check(
            &parse_text("let a: Int | Future<Int> = 1\nlet b: Int | Future<Int> = lazy 1"),
            &mut TypeTable::new()
        )
        .err(),
        None
    );
    assert_eq!(
        shown_result("let a: Float | Future<Float> = 1\na"),
        "Float | Future<Float>"
    );
    let ast = parse_text("let a: Float | Future<Float> = 1");
    let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
    let one = ast
        .expr_ids()
        .find(|&id| ast.get(id).kind == ExprKind::Int(1))
        .unwrap();
    assert_eq!(checked.types.get(one), TyId::FLOAT);
}

#[test]
fn unions_other_than_the_either_shape_are_rejected() {
    for (text, bar) in [
        ("let x: Int | Future<Float> = 1", 11),
        ("let x: Future<Int> | Future<Int> = lazy 1", 19),
        ("let x: Int | Future<Int> | Future<Int> = 1", 25),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(
            found[0].message, "a union with a `Future` member must be `T | Future<T>`",
            "{text}"
        );
        assert_eq!(found[0].span, Span::new(bar, bar + 1), "{text}");
    }
    // An erroneous member is reported once.
    assert_eq!(
        messages("let x: Nope | Future<Int> = 1"),
        vec!["unknown type `Nope`"]
    );
}

#[test]
fn unknown_applied_types() {
    assert_eq!(
        errors("let x: Foo<Int> = 1"),
        vec![("unknown type `Foo`".to_owned(), Span::new(7, 10))]
    );
    assert_eq!(
        errors("let x: Int<Int> = 1"),
        vec![(
            "type `Int` takes no type argument".to_owned(),
            Span::new(7, 10)
        )]
    );
    assert_eq!(
        errors("let x: Future = 1"),
        vec![(
            "type `Future` needs a type argument, as in `Future<Int>`".to_owned(),
            Span::new(7, 13)
        )]
    );
    assert_eq!(
        messages("let x: Present = 1"),
        vec!["type `Present` needs a type argument, as in `Present<Int>`"]
    );
    // The argument is checked too.
    assert_eq!(
        messages("let x: Foo<Nope> = 1"),
        vec!["unknown type `Foo`", "unknown type `Nope`"]
    );
    // `Future` and `Present` are ordinary names outside types.
    assert_eq!(shown_result("let Future = 1\nFuture + 1"), "Int");
}

#[test]
fn callbacks_must_accept_what_they_are_promised() {
    let double = "fn double(x: Int): Int { x * 2 }\n";
    // `run` only ever passes its callback present values, so its
    // callback parameter is narrowed, and takes `double` (see
    // `callback_parameters_are_narrowed`).
    assert_eq!(
        shown_result(&format!(
            "{double}fn run(f: fn(Int) -> Int) -> Int {{ f(1) }}\nrun(double)"
        )),
        "Int | Future<Int>"
    );
    // This `run` passes its callback a value that may be a future.
    let text =
        format!("{double}fn run(f: fn(Int) -> Int, j: Int) -> Int {{ f(j) }}\nrun(double, 1)");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, last(&text, "double"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected fn(Int | Future<Int>) -> Int | Future<Int>, found fn(x: Int): Int")
    );
    // One that promises present values accepts it.
    assert_eq!(
        shown_result(&format!(
            "{double}fn run(f: fn(Present<Int>) -> Int) -> Int {{ f(1) }}\nrun(double)"
        )),
        "Int | Future<Int>"
    );
    // A pure function that passes values through fits both, whether its
    // parameter is declared the either shape or inferred to be one.
    for keep in [
        "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }\n",
        "fn keep(x: Int): {v: Int | Future<Int>} { v = x }\n",
    ] {
        assert_eq!(
            shown_result(&format!(
                "{keep}fn run(f: fn(Int) -> {{v: Int}}) -> {{v: Int}} {{ f(1) }}\nrun(keep)"
            )),
            "{v: Int | Future<Int>} | Future<{v: Int | Future<Int>}>"
        );
        assert_eq!(
            shown_result(&format!(
                "{keep}fn run(f: fn(Present<Int>) -> {{v: Int}}) -> {{v: Int}} {{ f(1) }}\n\
                     run(keep)"
            )),
            "{v: Int | Future<Int>} | Future<{v: Int | Future<Int>}>"
        );
    }
}

#[test]
fn if_branches_that_may_be_futures_join() {
    assert_eq!(
        shown_result("if true { lazy 1 } else { lazy 2 }"),
        "Future<Int>"
    );
    assert_eq!(
        shown_result("if true { 1 } else { lazy 2 }"),
        "Int | Future<Int>"
    );
    assert_eq!(
        shown_result("if true { lazy 1.5 } else { 2 }"),
        "Float | Future<Float>"
    );
    assert_eq!(
        shown_result("let x: Int | Future<Int> = 1\nif true { lazy 1 } else { x }"),
        "Int | Future<Int>"
    );
    assert_eq!(
        shown_result("if true { { a = lazy 1; b = 1 } } else { { a = 2 } }"),
        "{a: Int | Future<Int>}"
    );
    assert_eq!(
        shown_result("if true { lazy { a = 1; b = 1 } } else { lazy { a = 2 } }"),
        "Future<{a: Int}>"
    );
    let text = "if true { lazy 1 } else { lazy \"s\" }";
    let found = warnings(text);
    assert_eq!(
        found[0].message,
        "the branches of this `if` have different types"
    );
    assert_eq!(found[0].span, at(text, "lazy \"s\""));
    assert_eq!(found[0].label.as_deref(), Some("this is Future<String>"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "lazy 1"), "this is Future<Int>")]
    );
    assert_eq!(shown_result(text), "Future<Int | String>");
    // An erroneous branch is reported once.
    assert_eq!(
        messages("if true { lazy 1 } else { nope }"),
        vec!["undefined variable `nope`"]
    );
}

#[test]
fn futures_have_no_equality() {
    let text = "let r = { a = lazy 1 }\nr == r";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "cannot compare `{a: Future<Int>}`: it contains a future"
    );
    assert_eq!(found[0].span, at(text, "=="));
    assert_eq!(
        labels(&found[0]),
        vec![
            (nth(text, "r", 1), "this is {a: Future<Int>}"),
            (nth(text, "r", 2), "this is {a: Future<Int>}")
        ]
    );
    assert_eq!(
        found[0].notes,
        vec!["futures have no equality; await the future and compare its value"]
    );
    // Under a future at the top, for every equality.
    for op in ["==", "!=", "===", "!=="] {
        assert_eq!(
            messages(&format!("let r = lazy {{ a = lazy 1 }}\nr {op} r")),
            vec!["cannot compare `{a: Future<Int>}`: it contains a future"],
            "{op}"
        );
    }
    // A field of the either shape is awaited deeply and compared.
    assert_eq!(
        shown_result("fn f(r: {a: Int}) -> Bool { r == { a = 1 } }\nf({ a = 1 })"),
        "Bool"
    );
    // A future at the top is awaited as for any operator.
    assert_eq!(shown_result("(lazy 1) === 1"), "Bool");
}

#[test]
fn unused_futures() {
    let text = "let x = lazy 1\nx\n1";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "unused value of type `Future<Int>`");
    assert_eq!(
        found[0].notes,
        vec!["await it with `await`, bind it with `let`, or remove it"]
    );
    // A statement expects `{}`: a future of `{}`, or a value that may
    // be one, is awaited there, so its effects are not lost.
    assert_eq!(marks("lazy { }\n1"), vec![("lazy { }", AwaitMark::Top)]);
    assert_eq!(
        marks("let x = lazy { }\nlet y: {} | Future<{}> = {}\nx\ny\n1"),
        vec![("x", AwaitMark::Top), ("y", AwaitMark::Top)]
    );
    assert_eq!(
        messages("fn p(x: Future<{}>): Int { x\n1 }"),
        vec!["cannot use a value that may be a future in a pure function"]
    );
    assert_eq!(
        diagnostics("let x: Int | Future<Int> = 1\nx\n1")[0].notes,
        vec!["await it with `await`, bind it with `let`, or remove it"]
    );
    // A bare `await` is exempt, whatever its value, in any block.
    for text in [
        "let x = lazy 1\nawait x\n2",
        "let x = lazy { }\n{ await x; = 1 }",
        "let x = lazy 1\n{ a = 1; await x }",
        "fn run(step: Future<Int>) -> {} { await step\n= {} }",
    ] {
        assert_eq!(
            check(&parse_text(text), &mut TypeTable::new()).err(),
            None,
            "{text}"
        );
    }
}

#[test]
fn a_spread_whose_elements_may_be_futures_awaits_its_list_deeply() {
    // An impure function's `List<String>` parameter's elements may be
    // futures: spread into a list of present strings, it is awaited.
    let text = "fn run(words: List<String>) -> List<String> { let all: List<String> = [\"a\", ...words]\n= all }";
    assert_eq!(marks(text), [("words", AwaitMark::Deep)]);
    // One whose elements fit as they are is awaited at the top alone, as
    // any spread's operand is measured.
    assert_eq!(
        marks("fn run(words: List<String>) -> List<String | Future<String>> { [\"a\", ...words] }"),
        [("words", AwaitMark::Top)]
    );
    // In a pure function, awaiting it is an error.
    let text = "fn g(xs: List<Int | Future<Int>>): List<Int> { [...xs] }";
    assert_implicit_await_error(text, last(text, "xs"), "List<Int | Future<Int>>");
}
