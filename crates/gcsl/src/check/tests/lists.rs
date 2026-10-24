//! Tests of the checker: list types, literals, spreads and indexes.

use super::*;

#[test]
fn list_types_resolve_either_way() {
    for (ty, shown) in [
        ("List<Int>", "List<Int>"),
        ("Int[]", "List<Int>"),
        ("Int[][]", "List<List<Int>>"),
        ("List<Int>[]", "List<List<Int>>"),
        ("{a: Int}[]", "List<{a: Int}>"),
        ("Future<Int>[]", "List<Future<Int>>"),
        ("(fn(): Int)[]", "List<fn(): Int>"),
        ("(Int | Future<Int>)[]", "List<Int | Future<Int>>"),
        ("fn(): Int[]", "fn(): List<Int>"),
        (
            "List<Int> | Future<List<Int>>",
            "List<Int> | Future<List<Int>>",
        ),
    ] {
        let text = format!("let f = fn(x: Present<{ty}>): {{}} {{ }}\nf");
        assert_eq!(shown_result(&text), format!("fn(x: {shown}): {{}}"), "{ty}");
    }
    // `List` takes an argument, as `Future` does.
    let text = "let x: List = y";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "type `List` needs a type argument, as in `List<Int>`"
    );
    assert_eq!(found[0].span, at(text, "List"));
    assert_eq!(
        messages("let x: Lust<Int> = 1"),
        vec!["unknown type `Lust`"]
    );
    // `[]` binds tighter than `|`: a list of futures, not an either.
    assert_eq!(
        shown_result("let x: Int | Future<Int>[] = 1\nx"),
        "Int | List<Future<Int>>"
    );
}

#[test]
fn the_bare_type_rule_passes_through_list_types() {
    // Impure: the either shape at the list and at its element.
    for (ty, shown) in [
        (
            "List<Int>",
            "List<Int | Future<Int>> | Future<List<Int | Future<Int>>>",
        ),
        (
            "Int[]",
            "List<Int | Future<Int>> | Future<List<Int | Future<Int>>>",
        ),
        (
            "{a: Int}[]",
            "List<{a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>> | \
                 Future<List<{a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>>>",
        ),
        // Explicit parts are kept.
        ("List<Present<Int>>", "List<Int> | Future<List<Int>>"),
        (
            "List<Future<Int>>",
            "List<Future<Int>> | Future<List<Future<Int>>>",
        ),
        ("Present<List<Int>>", "List<Int>"),
        ("Future<List<Int>>", "Future<List<Int>>"),
    ] {
        let text = format!("let f = fn(x: {ty}) -> {{}} {{ }}\nf");
        assert_eq!(
            shown_result(&text),
            format!("fn(x: {shown}) -> {{}}"),
            "{ty}"
        );
    }
    // A written impure function type's positions too, and a memoised
    // function's result; not a pure or memoised one's parameters.
    assert_eq!(
        shown_result("let f = fn(g: fn(Int[]) ->> Int[]) -> {} { }\nf"),
        "fn(g: fn(List<Int>) ->> List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) \
             -> {}"
    );
    assert_eq!(
        shown_result("let f = fn(g: Future<fn(Int[]) -> Int>) -> {} { }\nf"),
        "fn(g: Future<fn(List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> Int \
             | Future<Int>>) -> {}"
    );
}

#[test]
fn list_parameters_and_results_are_inferred() {
    // Slice 9: a pure function's list parameter, measured or not.
    assert_eq!(
        type_of_f("fn f(xs: List<Int>): Int { len(xs) }"),
        "fn(xs: List<Int>): Int"
    );
    assert_eq!(
        type_of_f("fn f(xs: Int[]): Int { xs[0] }"),
        "fn(xs: List<Int>): Int"
    );
    assert_eq!(
        type_of_f("fn f(xs: Int[]): List<Int> { [...xs] }"),
        "fn(xs: List<Int>): List<Int>"
    );
    assert_eq!(
        type_of_f("fn f(xs: Int[]): {v: Int[] | Future<Int[]>} { v = xs }"),
        "fn(xs: List<Int> | Future<List<Int>>): {v: List<Int> | Future<List<Int>>}"
    );
    // Slices 11 and 13: a list result position, and its element
    // position, from what the body returns.
    for (text, shown) in [
        ("fn f() -> List<Int> { [1, 2] }", "fn() -> List<Int>"),
        (
            "fn f() -> List<Int> { [1, lazy 2] }",
            "fn() -> List<Int | Future<Int>>",
        ),
        (
            "fn f() -> List<Int> { [lazy 1] }",
            "fn() -> List<Future<Int>>",
        ),
        ("fn f() -> List<Int> { [] }", "fn() -> List<Int>"),
        ("fn f() -> Int[] { lazy [1] }", "fn() -> Future<List<Int>>"),
        (
            "fn f(xs: Int[]) -> Int[] { xs }",
            "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> \
                 List<Int | Future<Int>> | Future<List<Int | Future<Int>>>",
        ),
        (
            "fn f() -> List<{a: Int, b: Int}> { [{ a = 1; b = lazy 2 }] }",
            "fn() -> List<{a: Int, b: Future<Int>}>",
        ),
        (
            "fn f() -> {xs: Int[]} { xs = [1] }",
            "fn() -> {xs: List<Int>}",
        ),
        (
            "fn f() -> List<Int[]> { [[1], [lazy 2]] }",
            "fn() -> List<List<Int | Future<Int>>>",
        ),
        // Explicit parts keep what they say.
        ("fn f() -> List<Present<Int>> { [1] }", "fn() -> List<Int>"),
        ("fn f() -> Present<List<Int>> { [1] }", "fn() -> List<Int>"),
        ("fn f() ->> Int[] { [1] }", "fn() ->> List<Int>"),
    ] {
        assert_eq!(type_of_f(text), shown, "{text}");
    }
    // A `->>` function's list parameter is present throughout, so an
    // argument whose elements may be futures is awaited deeply.
    let text = "fn m(xs: Int[]) ->> Int { len(xs) }\nlet ys = [1, lazy 2]\nlet n = m(ys)";
    assert_eq!(
        shown_result(&format!("{text}\nm")),
        "fn(xs: List<Int>) ->> Int"
    );
    assert_eq!(marks(text), vec![("ys", AwaitMark::Deep)]);
}

#[test]
fn list_callback_positions_are_narrowable() {
    assert_eq!(
        type_of_run("fn run(f: fn(List<Int>) -> Int) -> Int { f([1]) }"),
        "fn(f: fn(List<Int>) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    assert_eq!(
        type_of_run("fn run(f: fn(Int[]) -> Int, xs: Present<Int[]>) -> Int { f(xs) }"),
        "fn(f: fn(List<Int>) -> Int | Future<Int>, xs: List<Int>) -> Int | Future<Int>"
    );
    // An element that may be a future keeps the position as written.
    assert_eq!(
        type_of_run("fn run(f: fn(Int[]) -> Int, j: Int) -> Int { f([j]) }"),
        "fn(f: fn(List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> Int | \
             Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>"
    );
    // So a pure function measuring its list is accepted once narrowed.
    let text = "fn sum(xs: List<Int>): Int { xs[0] + xs[1] }\n\
                    fn run(f: fn(List<Int>) -> Int) -> Int { f([1, 2]) }\n\
                    run(sum)";
    assert!(check(&parse_text(text), &mut TypeTable::new()).is_ok());
}

#[test]
fn list_literals_join_their_elements() {
    for (text, shown) in [
        ("[1, 2]", "List<Int>"),
        ("[1, 2.5]", "List<Float>"),
        ("[2.5, 1]", "List<Float>"),
        ("[1, 2, 2.5]", "List<Float>"),
        ("[\"a\"]", "List<String>"),
        ("[1, lazy 2]", "List<Int | Future<Int>>"),
        ("[lazy 1, lazy 2]", "List<Future<Int>>"),
        ("[1, lazy 2.5]", "List<Float | Future<Float>>"),
        ("[{ a = 1; b = 2 }, { a = 3 }]", "List<{a: Int}>"),
        ("[{ a = 1 }, { b = 3 }]", "List<{}>"),
        ("[[1], [2, 3]]", "List<List<Int>>"),
        ("[...[1], 2]", "List<Int>"),
        ("[1, ...[2.5]]", "List<Float>"),
        ("[...[1], ...[lazy 2]]", "List<Int | Future<Int>>"),
        ("[fn(x: Int): Int { x }]", "List<fn(x: Int): Int>"),
        ("[{}]", "List<{}>"),
    ] {
        assert_eq!(shown_result(text), text_type(shown), "{text}");
    }
    // Elements are checked, and nothing is awaited.
    assert_eq!(marks("[1, lazy 2]"), vec![]);
}

/// `shown` as it is: for the table of `list_literals_join_their_elements`.
pub(super) fn text_type(shown: &str) -> String {
    shown.to_owned()
}

#[test]
fn list_elements_without_a_join_are_reported() {
    let text = "[1, 2, \"a\", 3]";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    let error = &found[0];
    assert_eq!(
        error.message,
        "the elements of this list have different types"
    );
    assert_eq!(error.span, at(text, "\"a\""));
    assert_eq!(error.label.as_deref(), Some("this is String"));
    assert_eq!(
        labels(error),
        vec![(at(text, "1"), "the elements so far are Int")]
    );
    // A spread is labelled with its elements' type.
    let text = "[1.5, ...[\"a\"]]";
    let error = &warnings(text)[0];
    assert_eq!(
        error.message,
        "the elements of this list have different types"
    );
    assert_eq!(error.span, at(text, "...[\"a\"]"));
    assert_eq!(error.label.as_deref(), Some("its elements are String"));
    assert_eq!(
        labels(error),
        vec![(at(text, "1.5"), "the elements so far are Float")]
    );
    // Records of any width join; functions whose parameters do not
    // compare join to their union.
    let text = "[fn(x: {a: Int}): Int { 1 }, fn(x: {b: Int}): Int { 1 }]";
    assert_eq!(
        warnings(text)[0].message,
        "the elements of this list have different types"
    );
    // An erroneous element is not reported again.
    assert_eq!(messages("[1, y, \"a\"]"), vec!["undefined variable `y`"]);
}

#[test]
fn an_empty_literal_needs_an_expected_type() {
    for text in [
        "[]",
        "let xs = []",
        "[...[]]",
        "if true { [] } else { [1] }",
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, "cannot infer the element type of `[]`");
        assert_eq!(found[0].span, at(text, "[]"), "{text}");
        assert_eq!(
            found[0].notes,
            vec!["annotate it, as in `let xs: List<Int> = []`"]
        );
    }
    // With one, anywhere a record block gets one, and through the
    // results of blocks and the branches of an `if`.
    for text in [
        "let xs: List<Int> = []\nxs",
        "[] : Int[]",
        "let xs: Int[][] = [[1], []]\nxs[1]",
        "fn f(xs: Int[]): Int { len(xs) }\nf([])",
        "fn f(): Int[] { [] }\nf()",
        "fn f(c: Bool): Int[] { if c { [] } else { [1] } }\nf(true)",
        "let r: {xs: Int[]} = { xs = [] }\nr.xs",
        "let xs: Int[] = [1]\nxs := []\nxs",
        "let xs: Int[] = { let n = 1\n [] }\nxs",
        "let xs: Int[] = ([])\nxs",
    ] {
        assert!(
            check_prelude(text).2.is_ok(),
            "{text}: {:?}",
            diagnostics(text)
        );
    }
    // Its elements count as present, where that fits.
    let text = "[] : List<Int | Future<Int>>";
    assert_eq!(type_at(text, at(text, "[]")), "List<Int>");
    let text = "[] : List<{a: Int | Future<Int>}>";
    assert_eq!(type_at(text, at(text, "[]")), "List<{a: Int}>");
    let text = "[] : List<Future<Int>>";
    assert_eq!(type_at(text, at(text, "[]")), "List<Future<Int>>");
}

#[test]
fn a_literal_against_an_expected_type_fits_each_element() {
    // An element that fits once awaited is awaited, at the top level.
    let text = "let xs: List<Present<Int>> = [1, lazy 2]\nxs";
    assert_eq!(marks(text), vec![("lazy 2", AwaitMark::Top)]);
    assert_eq!(shown_result(text), "List<Int>");
    // A literal of records and of lists, deeply.
    let text = "let xs: List<Present<{a: Int}>> = [{ a = 1 }, lazy { a = 2 }]\nxs";
    assert_eq!(marks(text), vec![("lazy { a = 2 }", AwaitMark::Top)]);
    let text = "let ys = [lazy 1]\nlet xs: List<Present<Int[]>> = [ys]\nxs";
    assert_eq!(marks(text), vec![("ys", AwaitMark::Deep)]);
    // Untyped integers adapt to the element type.
    assert_eq!(shown_result("[1, 2] : Float[]"), "List<Float>");
    // Anything else is a mismatch at the element.
    let text = "let xs: List<Int> = [1, \"a\"]";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "\"a\""));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "List<Int>"), "expected because of this annotation")]
    );
    // In a pure function, an await of an element is the purity error.
    let text = "fn f(): List<Present<Int>> { [1, lazy 2] }";
    assert_implicit_await_error(text, at(text, "lazy 2"), "Future<Int>");
    // The literal's type is what its elements are once awaited.
    let text = "fn f() -> List<Int> { [1, lazy 2, await lazy 3] }\nf";
    assert_eq!(shown_result(text), "fn() -> List<Int | Future<Int>>");
}

#[test]
fn spreads_are_measured_and_must_be_lists() {
    // Present, future and provisional lists.
    assert_eq!(shown_result("let xs = [1]\n[...xs, 2]"), "List<Int>");
    let text = "let xs = lazy [1]\n[...xs, 2]";
    assert_eq!(shown_result(text), "List<Int>");
    assert_eq!(marks(text), vec![("xs", AwaitMark::Top)]);
    let text = "fn f(xs: Future<Int[]>): Int[] { [...xs] }";
    assert_implicit_await_error(text, last(text, "xs"), "Future<List<Int>>");
    assert_eq!(
        type_of_f("fn f(xs: Int[]): Int { len([...xs]) }"),
        "fn(xs: List<Int>): Int"
    );
    // Anything else is not a list.
    let text = "[...1]";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot spread a value of type Int: it is not a list"
    );
    assert_eq!(found[0].span, at(text, "...1"));
    assert_eq!(labels(&found[0]), vec![(at(text, "1"), "this is Int")]);
    assert_eq!(
        messages("[...{ a = 1 }]"),
        vec!["cannot spread a value of type `{a: Int}`: it is not a list"]
    );
    // Against an expected element type, its elements must fit as they
    // are: there is nothing to await for each.
    let text = "let xs: List<Present<Int>> = [1, ...[lazy 2]]";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "[lazy 2]"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected List<Int>, found List<Future<Int>>")
    );
    assert_eq!(found[0].notes.len(), 1);
    assert!(found[0].notes[0].contains("no expression to await"));
    // A mismatch for another reason has no note.
    let found = diagnostics("let xs: List<Int> = [...[\"a\"]]");
    assert_eq!(found[0].message, "mismatched types");
    assert!(found[0].notes.is_empty());
    // Elements that fit, and a provisional list's elements where a
    // present element is expected, which are measured.
    assert_eq!(
        shown_result("let xs: List<Int | Future<Int>> = [...[lazy 1], 2]\nxs"),
        "List<Int | Future<Int>>"
    );
    assert_eq!(
        type_of_f("fn f(x: Int): Int[] {\n let ys = [x]\n [...ys] : Int[] }"),
        "fn(x: Int): List<Int>"
    );
}

#[test]
fn indexes_read_an_element_as_it_is() {
    assert_eq!(shown_result("[1, 2][0]"), "Int");
    assert_eq!(shown_result("let xs = [lazy 1]\nxs[0]"), "Future<Int>");
    // Using the element awaits it, not the list or the index.
    let text = "let xs = [1, lazy 2]\nxs[1] + 1";
    assert_eq!(marks(text), vec![("xs[1]", AwaitMark::Top)]);
    // A list or an index that may be a future is awaited.
    let text = "let xs = lazy [1]\nlet i = lazy 0\nxs[i]";
    assert_eq!(
        marks(text),
        vec![("xs", AwaitMark::Top), ("i", AwaitMark::Top)]
    );
    // In a pure function that is the purity error.
    let text = "fn f(xs: Future<Int[]>): Int { xs[0] }";
    assert_implicit_await_error(text, last(text, "xs"), "Future<List<Int>>");
    // A pure function's parameter is measured.
    assert_eq!(
        type_of_f("fn f(xs: Int[], i: Int): Int { xs[i] }"),
        "fn(xs: List<Int>, i: Int): Int"
    );
}

#[test]
fn index_errors() {
    let text = "let x = 1\nx[0]";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot index a value of type Int: it is not a list"
    );
    assert_eq!(found[0].span, at(text, "[0]"));
    assert_eq!(labels(&found[0]), vec![(last(text, "x"), "this is Int")]);
    assert_eq!(
        messages("{ a = 1 }[0]"),
        vec!["cannot index a value of type `{a: Int}`: it is not a list"]
    );
    let text = "[1][\"a\"]";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "a list index must be an Int, found String"
    );
    assert_eq!(found[0].span, at(text, "\"a\""));
    assert_eq!(found[0].label.as_deref(), Some("this is String"));
    assert_eq!(
        messages("[1][1.5]"),
        vec!["a list index must be an Int, found Float"]
    );
    // Both are reported.
    assert_eq!(
        messages("1[2.5]"),
        vec![
            "cannot index a value of type Int: it is not a list",
            "a list index must be an Int, found Float"
        ]
    );
}
