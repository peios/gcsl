//! Tests of the checker: ranges, `for` and unit-like statements.

use super::*;

#[test]
fn ranges_are_present_lists_of_ints() {
    for text in [
        "0..10",
        "0..=10",
        "let n = 3\n0..n",
        "let n = 3\n-n..=n + 1",
        "(0..2)[1] .. 5",
    ] {
        assert_eq!(shown_result(text), "List<Int>", "{text}");
    }
    assert_eq!(shown_result("(0..2)[1]"), "Int");
    // Untyped bounds become `Int`s.
    assert!(!all_types("0..=10").contains(&TyId::UNTYPED_INT));
    let text = "0..10";
    assert_eq!(type_at(text, at(text, "0")), "Int");
    // Both bounds are measured: awaited if they may be futures.
    let text = "let a = lazy 1\nlet b = lazy 5\na..=b";
    assert_eq!(
        marks(text),
        vec![("a", AwaitMark::Top), ("b", AwaitMark::Top)]
    );
    assert_eq!(shown_result(text), "List<Int>");
    // A pure function's parameter is measured, and a range is pure.
    assert_eq!(
        type_of_f("fn f(n: Int): Int[] { 0..n }"),
        "fn(n: Int): List<Int>"
    );
    let text = "fn f(n: Future<Int>): Int[] { 0..n }";
    assert_implicit_await_error(text, last(text, "n"), "Future<Int>");
}

#[test]
fn range_errors() {
    let text = "0..1.5";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "a range's bounds must be Int, found Float"
    );
    assert_eq!(found[0].span, at(text, "1.5"));
    assert_eq!(found[0].label.as_deref(), Some("this is Float"));
    assert!(found[0].labels.is_empty());
    let text = "\"a\"..=1";
    assert_eq!(
        errors(text),
        vec![(
            "a range's bounds must be Int, found String".to_owned(),
            at(text, "\"a\"")
        )]
    );
    assert_eq!(
        messages("{ a = 1 }..2"),
        vec!["a range's bounds must be Int, found `{a: Int}`"]
    );
    // Both bounds are reported.
    assert_eq!(
        messages("1.5..true"),
        vec![
            "a range's bounds must be Int, found Float",
            "a range's bounds must be Int, found Bool"
        ]
    );
    // An erroneous bound is not reported again, nor a range of one.
    assert_eq!(messages("0..y"), vec!["undefined variable `y`"]);
    assert_eq!(messages("len(0..y) + 1"), vec!["undefined variable `y`"]);
}

#[test]
fn a_for_is_the_list_of_its_body() {
    for (text, shown) in [
        ("let xs = [1, 2]\nfor x in xs { x * 2 }", "List<Int>"),
        ("for x in [1, 2] if x % 2 == 0 { x }", "List<Int>"),
        ("for i in 0..10 { i * i }", "List<Int>"),
        ("for i in 0..=10 if i > 2 { \"{i}\" }", "List<String>"),
        // An untyped integer settles as a list literal's element does.
        ("for x in [1] { 1 }", "List<Int>"),
        ("for x in [1] { 2.5 }", "List<Float>"),
        ("for x in [1.5] { x / 2 }", "List<Float>"),
        // A record block makes a list of records.
        (
            "for x in [1] { a = x; b = \"b\" }",
            "List<{a: Int, b: String}>",
        ),
        ("for x in [1] { }", "List<{}>"),
        // The element may be a future, and so may the body.
        ("for x in [lazy 1] { x }", "List<Future<Int>>"),
        ("for x in [1, lazy 2] { x }", "List<Int | Future<Int>>"),
        ("for x in [1] { lazy x }", "List<Future<Int>>"),
        // Nested, in the body and in the iterable.
        ("for x in [[1]] { for y in x { y } }", "List<List<Int>>"),
        ("for x in for y in [1] { y } { x }", "List<Int>"),
        ("[...for x in [1] { x }, 2]", "List<Int>"),
        ("len(for x in [1] { x })", "Int"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // Every node settles.
    assert!(!all_types("for x in [1] { 1 }").contains(&TyId::UNTYPED_INT));
}

#[test]
fn the_variable_has_the_element_type_in_a_scope_of_its_own() {
    // As it is, lazy or not.
    let text = "for x in [lazy 1] { x }";
    assert_eq!(type_at(text, last(text, "x")), "Future<Int>");
    assert_eq!(marks(text), vec![]);
    let text = "for x in [1, lazy 2] if true { x }";
    assert_eq!(type_at(text, last(text, "x")), "Int | Future<Int>");
    // Used as its bits, it is awaited there.
    let text = "for x in [lazy 1] { x + 1 }";
    assert_eq!(marks(text), vec![("x", AwaitMark::Top)]);
    assert_eq!(shown_result(text), "List<Int>");
    // It shadows as a `let` does, and is gone after the `for`.
    let text = "let x = \"a\"\nlet ys = for x in [1] { x + 1 }\nx";
    assert_eq!(shown_result(text), "String");
    assert_eq!(
        messages("let ys = for x in [1] { x }\nx"),
        vec!["undefined variable `x`"]
    );
    // The guard sees it.
    assert_eq!(shown_result("for x in [1, 2] if x > 1 { x }"), "List<Int>");
    // It cannot be rebound from the body, whose block is its own.
    let text = "for x in [1] {\n  x := 2\n  x\n}";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot rebind `x` here: it is declared in an enclosing block"
    );
    assert_eq!(labels(&found[0]), vec![(at(text, "x"), "declared here")]);
    // It takes the next free slot, which the `for` leaves when it ends;
    // the body's variables take those after it.
    let text = "let a = 1\nlet ys = for x in [a] {\n  let y = x\n  y\n}\nlet b = 2";
    let (ast, resolution) = resolved(text);
    assert_eq!(resolution.for_slot(ForId::new(0)), Slot(1));
    assert_eq!(uses(&ast, &resolution, "x"), vec![VarRef::Local(Slot(1))]);
    assert_eq!(uses(&ast, &resolution, "y"), vec![VarRef::Local(Slot(2))]);
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[1]), Slot(1));
    assert_eq!(resolution.item_slot(items[2]), Slot(2));
    assert_eq!(resolution.frame_size(), 3);
    // A closure made in the body captures it as any variable of the
    // enclosing function, from its slot.
    let text = "let fs = for x in [1] { fn(): Int { x } }";
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Local(Slot(0)), "Int".to_owned())]
    );
    // In a function, the slot is the function's.
    let text = "fn f(n: Int): Int[] { for x in [n] { x } }";
    let (_, resolution) = resolved(text);
    assert_eq!(resolution.for_slot(ForId::new(0)), Slot(1));
    assert_eq!(resolution.function(FnId::new(0)).frame_size, 2);
}

#[test]
fn for_errors() {
    let text = "for x in 1 { x }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot iterate over a value of type Int: it is not a list"
    );
    assert_eq!(found[0].span, at(text, "1"));
    assert_eq!(found[0].label.as_deref(), Some("this is Int"));
    assert!(found[0].labels.is_empty());
    assert_eq!(
        messages("for x in { a = 1 } { x }"),
        vec!["cannot iterate over a value of type `{a: Int}`: it is not a list"]
    );
    // Once awaited: a future of a list is one.
    assert_eq!(
        messages("let n = lazy 1\nfor x in n { x }"),
        vec!["cannot iterate over a value of type Int: it is not a list"]
    );
    // The guard must be `Bool`.
    let text = "for x in [1] if x { x }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, nth(text, "x", 1));
    assert_eq!(found[0].label.as_deref(), Some("expected Bool, found Int"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "for"), "`for` guards are Bool")]
    );
    // Errors do not cascade: an erroneous iterable, whose variable is
    // then erroneous, and an erroneous body.
    assert_eq!(
        messages("for x in y { x + 1 }"),
        vec!["undefined variable `y`"]
    );
    assert_eq!(
        messages("for x in 1 { x + 1 }\n2"),
        vec!["cannot iterate over a value of type Int: it is not a list"]
    );
    assert_eq!(
        messages("let ys = for x in [1] { z }\nlen(ys)"),
        vec!["undefined variable `z`"]
    );
    // Every part is checked whatever goes wrong.
    assert_eq!(
        messages("for x in y if z { w }"),
        vec![
            "undefined variable `y`",
            "undefined variable `z`",
            "undefined variable `w`"
        ]
    );
}

#[test]
fn the_iterable_is_measured_at_the_top() {
    // A future, or an either, is awaited; its elements are not.
    let text = "let xs = lazy [lazy 1]\nfor x in xs { x }";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Top)]);
    assert_eq!(shown_result(text), "List<Future<Int>>");
    let text = "let xs: Int[] | Future<Int[]> = [1]\nfor x in xs { x }";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Top)]);
    // In an impure function, where a bare list parameter may be a
    // future, and its elements too.
    let text = "fn f(xs: Int[]) -> Int[] { for x in xs { x } }";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Top)]);
    assert_eq!(
        fn_type(text, 0),
        "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> List<Int | \
             Future<Int>>"
    );
    // In a pure function a future is the purity error, and a bare
    // parameter is measured, which makes it present.
    let text = "fn f(xs: Future<Int[]>): Int[] { for x in xs { x } }";
    assert_implicit_await_error(text, last(text, "xs"), "Future<List<Int>>");
    let text = "fn f(xs: Int[] | Future<Int[]>): Int[] { for x in xs { x } }";
    assert_implicit_await_error(text, last(text, "xs"), "List<Int> | Future<List<Int>>");
    assert_eq!(
        type_of_f("fn f(xs: Int[]): Int { len(for x in xs { x }) }"),
        "fn(xs: List<Int>): Int"
    );
    assert_eq!(
        demands("fn f(xs: Int[]): Int { len(for x in xs { x }) }", 0),
        vec![Some("xs")]
    );
    // So is one captured from an enclosing pure function.
    assert_eq!(
        type_of_f("fn f(xs: Int[]): fn(): Int[] { fn(): Int[] { for x in xs { x } } }"),
        "fn(xs: List<Int>): fn(): List<Int>"
    );
}

#[test]
fn the_body_is_checked_in_the_enclosing_function() {
    // Its awaits are the enclosing function's: in a pure one, the purity
    // error.
    let text = "fn f(xs: List<Future<Int>>): Int[] { for x in xs { x + 1 } }";
    assert_implicit_await_error(text, nth(text, "x", 3), "Future<Int>");
    let text = "fn f(xs: List<Future<Int>>): Int[] { for x in xs if x > 0 { 1 } }";
    assert_implicit_await_error(text, nth(text, "x", 3), "Future<Int>");
    let text = "fn f(n: Future<Int>): Int[] { for x in [1] { await n } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "cannot await in a pure function");
    // And its calls: a pure function calls nothing impure there either.
    let text = "fn g() -> Int { 1 }\nfn f(): Int[] { for x in [1] { g() } }";
    assert_eq!(
        messages(text),
        vec!["cannot call an impure function from a pure function"]
    );
    // In an impure function, or at the top level, the same is fine.
    let text = "fn f(xs: List<Future<Int>>) -> Int[] { for x in xs { x + 1 } }";
    assert_eq!(
        marks(text),
        vec![("xs", AwaitMark::Top), ("x", AwaitMark::Top)]
    );
    // A parameter of the enclosing pure function used in the body is
    // measured as anywhere in its body.
    assert_eq!(
        type_of_f("fn f(n: Int): Int[] { for x in [1] { x + n } }"),
        "fn(n: Int): List<Int>"
    );
    assert_eq!(
        type_of_f("fn f(n: Int): List<Int | Future<Int>> { for x in [1] { n } }"),
        "fn(n: Int | Future<Int>): List<Int | Future<Int>>"
    );
}

#[test]
fn an_expected_list_type_reaches_the_body() {
    // An untyped integer adapts to the element type.
    let text = "let ys: List<Float> = for x in [1, 2] { 1 }\nys";
    assert_eq!(shown_result(text), "List<Float>");
    assert_eq!(
        type_at(text, at(text, "for x in [1, 2] { 1 }")),
        "List<Float>"
    );
    assert_eq!(type_at(text, last(text, "1")), "Float");
    // A body that fits once awaited is awaited, each iteration, and the
    // `for` is the list of what it is then.
    let text = "let xs = [lazy 1, lazy 2]\nlet ys: List<Present<Int>> = for x in xs { x }\nys";
    assert_eq!(marks(text), vec![("{ x }", AwaitMark::Top)]);
    assert_eq!(type_at(text, at(text, "for x in xs { x }")), "List<Int>");
    // A list type expected of the body reaches its result, as it reaches
    // a block's.
    let text = "let xs = [[lazy 1]]\nlet ys: List<Present<Int[]>> = for x in xs { x }\nys";
    assert_eq!(marks(text), vec![("x", AwaitMark::Deep)]);
    assert_eq!(
        type_at(text, at(text, "for x in xs { x }")),
        "List<List<Int>>"
    );
    // An either element type takes what the body is.
    let text = "let ys: List<Int | Future<Int>> = for x in [1] { lazy x }\nys";
    assert_eq!(
        type_at(text, at(text, "for x in [1] { lazy x }")),
        "List<Future<Int>>"
    );
    // Through parentheses, the result of a block, an ascription and an
    // argument.
    for text in [
        "let ys: List<Float> = (for x in [1] { 1 })\nys",
        "let ys: List<Float> = { for x in [1] { 1 } }\nys",
        "(for x in [1] { 1 }) : List<Float>",
        "fn f(ys: Float[]): Float[] { ys }\nf(for x in [1] { 1 })",
    ] {
        assert_eq!(shown_result(text), "List<Float>", "{text}");
    }
    // A record block is checked against the expected record type.
    let text = "let ys: List<{a: Float}> = for x in [1] { a = 1 }\nys";
    assert_eq!(shown_result(text), "List<{a: Float}>");
    let text = "let ys: List<{a: Int, b: Int}> = for x in [1] { a = 1 }";
    assert_eq!(messages(text), vec!["missing field `b` in record"]);
    // A body that does not fit is a mismatch at the body.
    let text = "let ys: List<Int> = for x in [1] { \"a\" }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "{ \"a\" }"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "List<Int>"), "expected because of this annotation")]
    );
    // In a pure function, an await of the body is the purity error.
    let text = "fn f(xs: List<Future<Int>>): List<Int> { for x in xs { x } }";
    assert_implicit_await_error(text, at(text, "{ x }"), "Future<Int>");
}

#[test]
fn a_function_returning_a_for_has_its_result_inferred() {
    for (text, shown) in [
        (
            "fn f() -> Int[] { for x in [1, 2] { x } }\nf",
            "fn() -> List<Int>",
        ),
        (
            "fn f() -> Int[] { for x in [1, 2] { lazy x } }\nf",
            "fn() -> List<Future<Int>>",
        ),
        (
            "fn f(xs: Int[]) -> Int[] { for x in xs { x } }\nf",
            "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> List<Int | \
                 Future<Int>>",
        ),
        (
            "fn f(xs: Int[]) -> Int[] { for x in xs { x + 1 } }\nf",
            "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> List<Int>",
        ),
        (
            "fn f() ->> {a: Int}[] { for x in [1] { a = lazy x } }\nf",
            "fn() ->> List<{a: Future<Int>}>",
        ),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
}

#[test]
fn a_unit_like_statement_is_not_an_unused_value() {
    // `{}`, a list of `{}`s, a list of those, and `[]` itself.
    for text in [
        "for x in [1] { }\n1",
        "let steps = [1]\nfor s in steps { let t = s }\n1",
        "for x in [1] { for y in [2] { } }\n1",
        "[{}, {}]\n1",
        "[[{}]]\n1",
        "[]\n1",
        "{ []\n= 1 }",
        "{ a = 1; [] }",
        "fn f(): Int { []\n1 }",
        "fn f(xs: List<{}>): Int { xs\n1 }",
    ] {
        assert_eq!(
            check(&parse_text(text), &mut TypeTable::new()).err(),
            None,
            "{text}"
        );
        assert_eq!(marks(text), vec![], "{text}");
    }
    // A statement `[]` is the unit-like `List<{}>`; one in parentheses,
    // or in a literal, is an ordinary empty literal.
    let text = "[]\n1";
    assert_eq!(type_at(text, at(text, "[]")), "List<{}>");
    for text in ["([])\n1", "[[]]\n1"] {
        assert_eq!(
            messages(text),
            vec!["cannot infer the element type of `[]`"],
            "{text}"
        );
    }
    // A pure function's parameter is measured by a statement, as a
    // `{}` one is.
    assert_eq!(
        type_of_f("fn f(xs: List<{}>): Int { xs\n1 }"),
        "fn(xs: List<{}>): Int"
    );
}

#[test]
fn a_statement_unit_like_once_awaited_is_awaited() {
    // At the top, as before, and deeply.
    let text = "let s = lazy [{}]\ns\n1";
    assert_eq!(marks(text), vec![("s", AwaitMark::Top)]);
    let text = "let s = lazy [lazy {}]\ns\n1";
    assert_eq!(marks(text), vec![("s", AwaitMark::Deep)]);
    let text = "let steps = [lazy {}, lazy {}]\nfor s in steps { s }\n1";
    assert_eq!(marks(text), vec![("for s in steps { s }", AwaitMark::Deep)]);
    let text = "fn run(n: Int) -> {} { }\nfor s in [1, 2] { lazy run(s) }\n1";
    assert_eq!(
        marks(text),
        vec![("for s in [1, 2] { lazy run(s) }", AwaitMark::Deep)]
    );
    let text = "let s = [lazy [lazy {}]]\ns\n1";
    assert_eq!(marks(text), vec![("s", AwaitMark::Deep)]);
    // In a pure function, that is the purity error.
    let text = "fn f(steps: List<Future<{}>>): Int { for s in steps { s }\n1 }";
    assert_implicit_await_error(text, at(text, "for s in steps { s }"), "List<Future<{}>>");
    let text = "fn f(steps: List<Future<{}>>): Int { steps\n1 }";
    assert_implicit_await_error(text, last(text, "steps"), "List<Future<{}>>");
}

#[test]
fn any_other_statement_is_still_an_unused_value() {
    for (text, shown) in [
        ("[1]\n2", "`List<Int>`"),
        ("for x in [1] { x }\n2", "`List<Int>`"),
        ("[lazy 1]\n2", "`List<Future<Int>>`"),
        ("[{ a = 1 }]\n2", "`List<{a: Int}>`"),
        ("for x in [1] { lazy x }\n2", "`List<Future<Int>>`"),
        ("let s = lazy [1]\ns\n2", "`Future<List<Int>>`"),
    ] {
        assert_eq!(
            messages(text),
            vec![format!("unused value of type {shown}")],
            "{text}"
        );
    }
    let text = "for x in [1] { x }\n2";
    assert_eq!(diagnostics(text)[0].span, at(text, "for x in [1] { x }"));
}
