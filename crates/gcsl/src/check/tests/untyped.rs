//! Tests of the checker: parameters written with no type, typed by their
//! defaults.

use super::*;

use defaults::{constant_str, given_defaults};

/// The one error of `text`.
fn only_error(text: &str) -> Diagnostic {
    let mut found = diagnostics(text);
    assert_eq!(found.len(), 1, "{text}: {found:?}");
    found.remove(0)
}

#[test]
fn a_parameter_has_its_default_s_type() {
    // As `let` would give it: a literal's, a list's, a record's, a call's,
    // an operator's, an untyped integer settled to `Int`.
    for (text, shown) in [
        ("fn f(n = 3): Int { n }", "fn(n: Int): Int"),
        ("fn f(x = 1.5): Float { x }", "fn(x: Float): Float"),
        ("fn f(s = \"x\"): String { s }", "fn(s: String): String"),
        ("fn f(b = true): Bool { b }", "fn(b: Bool): Bool"),
        ("fn f(n = -3): Int { n }", "fn(n: Int): Int"),
        ("fn f(xs = [1, 2]): Int { xs[0] }", "fn(xs: List<Int>): Int"),
        (
            "fn f(o = { a = 1; b = \"x\" }): Int { o.a }",
            "fn(o: {a: Int, b: String}): Int",
        ),
        ("fn f(o = {}): Int { 1 }", "fn(o: {} | Future<{}>): Int"),
        (
            "fn g(): String { \"x\" }\nfn f(s = g()): String { s }",
            "fn(s: String): String",
        ),
        ("fn f(n = 1 + 2): Int { n }", "fn(n: Int): Int"),
        ("fn f(n = (3)): Int { n }", "fn(n: Int): Int"),
        ("fn f(n = len([1])): Int { n }", "fn(n: Int): Int"),
        (
            "fn f(g = fn(n: Int): Int { n }): Int { g(1) }",
            "fn(g: fn(n: Int): Int): Int",
        ),
    ] {
        assert_eq!(type_of_f(text), shown, "{text}");
    }
    // In any position, beside parameters written with their types, and a
    // function expression's as a named function's.
    assert_eq!(
        type_of_f(
            "fn f(a = 1, b: String, c = [true]): Int { if c[0] and b == \"x\" { a } else { 0 } }"
        ),
        "fn(a: Int, b: String, c: List<Bool>): Int"
    );
    assert_eq!(type_of_f("let f = fn(n = 3): Int { n }"), "fn(n: Int): Int");
    assert_eq!(file_type("def f(n = 3): Int { n }"), "{f: fn(n: Int): Int}");
    // The body uses it as its type.
    assert_eq!(
        messages("fn f(n = 3): String { n }"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_default_reads_the_parameters_before_it() {
    assert_eq!(
        type_of_f("fn f(a: Int, b = a * 2): Int { a + b }"),
        "fn(a: Int, b: Int): Int"
    );
    // One written with no type too, which the next reads as its type; and
    // as with the types written, a default that needs an earlier parameter
    // present measures it.
    let untyped = type_of_f("fn f(a = \"x\", b = [a], c = b): Int { len(c) }");
    assert_eq!(
        untyped,
        "fn(a: String, b: List<String>, c: List<String>): Int"
    );
    assert_eq!(
        untyped,
        type_of_f(
            "fn f(a: String = \"x\", b: List<String> = [a], c: List<String> = b): Int { len(c) }"
        )
    );
    // What a default measures its function measures.
    assert_eq!(
        type_of_f("fn f(a: Int, b = a + 1): {v: Int | Future<Int>} { v = b }"),
        "fn(a: Int, b: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    // Not its own parameter, nor a later one.
    let text = "fn f(a = b, b = 1): Int { 1 }";
    let error = only_error(text);
    assert_eq!(error.message, "undefined variable `b`");
    assert_eq!(error.span, at(text, "b"));
    assert_eq!(
        labels(&error),
        vec![(nth(text, "b", 1), "`b` is a parameter from here on")]
    );
    assert_eq!(
        error.notes,
        vec!["a parameter's default sees only the parameters before it"]
    );
    assert_eq!(
        messages("fn f(a = a): Int { 1 }"),
        vec!["undefined variable `a`"]
    );
}

#[test]
fn a_default_with_no_type_of_its_own_is_an_error() {
    // As `let xs = []` is.
    let text = "fn f(xs = []): Int { 1 }";
    let error = only_error(text);
    let of_let = only_error("let xs = []");
    assert_eq!(error.message, "cannot infer the element type of `[]`");
    assert_eq!(error.message, of_let.message);
    assert_eq!(error.span, at(text, "[]"));
    assert_eq!(error.notes, of_let.notes);
    // An erroneous default is reported once, and its function is erroneous
    // for its users, quietly.
    assert_eq!(
        messages("fn f(n = nope): Int { 1 }\nlet a = f()\nlet b = f(1)"),
        vec!["undefined variable `nope`"]
    );
}

#[test]
fn a_call_is_checked_against_the_inferred_type() {
    let f = "fn f(n = 3, s = \"x\"): Int { if s == \"x\" { n } else { 0 } }\n";
    // Given by position and by name, and left out.
    assert_eq!(
        file_type(&format!(
            "{f}a = f()\nb = f(1)\nc = f(1, \"y\")\nd = f(s = \"y\")"
        )),
        "{a: Int, b: Int, c: Int, d: Int}"
    );
    // A call that leaves it out is given the default.
    let text = format!("{f}a = f()\nb = f(s = \"y\")");
    assert_eq!(
        given_defaults(&text),
        vec![
            ("f()", vec![(0, Constant::Int(3)), (1, constant_str("x"))]),
            ("f(s = \"y\")", vec![(0, Constant::Int(3))]),
        ]
    );
    // A mismatch is the usual error, the function shown with the type.
    for call in ["f(\"s\")", "f(n = \"s\")"] {
        let text = format!("{f}a = {call}");
        let error = only_error(&text);
        assert_eq!(error.message, "mismatched types", "{call}");
        assert_eq!(error.span, last(&text, "\"s\""), "{call}");
        assert_eq!(
            error.label.as_deref(),
            Some("expected Int, found String"),
            "{call}"
        );
        assert_eq!(
            labels(&error),
            vec![(
                last(&text, "f"),
                "the function is fn(n: Int, s: String): Int"
            )],
            "{call}"
        );
    }
    // Its defaults are kept as an annotated parameter's are: a constant,
    // or a default function after the function's captures.
    assert_eq!(
        defaults_of("fn f(n = 3, s = \"x\", o = { a = 1 }): Int { n }", 0)[..2],
        [Some(Constant::Int(3)), Some(constant_str("x"))]
    );
    assert!(matches!(
        defaults_of("fn f(o = { a = [1] }): Int { 1 }", 0)[..],
        [Some(Constant::Record(_))]
    ));
    let text = "let x = 1\nfn f(a = x, b: Int = x, c = a + b): Int { a + b + c }\nf()";
    let (ast, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    let f = FnId::new(u32::try_from(ast.fn_count() - 1).unwrap());
    let resolution = output.checked.resolution.function(f);
    let captures = u32::try_from(resolution.captures.len()).unwrap();
    let kept: Vec<u32> = resolution
        .defaults
        .iter()
        .map(|default| match default {
            Some(ParamDefault::Function { capture, .. }) => *capture,
            other => panic!("no default function: {other:?}"),
        })
        .collect();
    assert_eq!(kept, vec![captures, captures + 1, captures + 2]);
}

#[test]
fn a_parameter_is_as_if_its_type_were_written() {
    // In a pure function, present if the body measures it and the either
    // shape if it never does, as a bare parameter is.
    assert_eq!(type_of_f("fn f(n = 3): Int { n + 1 }"), "fn(n: Int): Int");
    assert_eq!(
        type_of_f("fn f(n = 3): {v: Int | Future<Int>} { v = n }"),
        type_of_f("fn f(n: Int = 3): {v: Int | Future<Int>} { v = n }")
    );
    assert_eq!(
        type_of_f("fn f(n = 3): {v: Int | Future<Int>} { v = n }"),
        "fn(n: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    // In an impure function, the either shape, at every bare position
    // inside it too.
    for (untyped, written) in [
        ("n = 3", "n: Int = 3"),
        ("xs = [1, 2]", "xs: List<Int> = [1, 2]"),
        (
            "o = { a = 1; b = [\"x\"] }",
            "o: {a: Int, b: List<String>} = { a = 1; b = [\"x\"] }",
        ),
        (
            "g = fn(n: Int): Int { n }",
            "g: fn(n: Int): Int = fn(n: Int): Int { n }",
        ),
    ] {
        assert_eq!(
            type_of_f(&format!("fn f({untyped}) -> Int {{ 1 }}")),
            type_of_f(&format!("fn f({written}) -> Int {{ 1 }}")),
            "{untyped}"
        );
    }
    assert_eq!(
        type_of_f("fn f(xs = [1, 2]) -> Int { 1 }"),
        "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> Int"
    );
    // A position that says what it is stays what it is.
    assert_eq!(
        type_of_f("fn f(o = { a = lazy 1 }) -> Int { 1 }"),
        "fn(o: {a: Future<Int>} | Future<{a: Future<Int>}>) -> Int"
    );
    // The type is the default's once awaited: a future default makes no
    // future parameter.
    assert_eq!(
        type_of_f("fn f(n = lazy 3) -> Int { n }"),
        type_of_f("fn f(n: Int = 3) -> Int { n }")
    );
    // In a memoised function, present, deeply: the default is awaited.
    assert_eq!(
        type_of_f("fn f(n = lazy 3) ->> Int { n }"),
        "fn(n: Int) ->> Int"
    );
    assert_eq!(
        type_of_f("fn g() -> List<Int> { [1] }\nfn f(xs = [g()]) ->> Int { 1 }"),
        "fn(xs: List<List<Int>>) ->> Int"
    );
    // In a pure function a future default would be awaited: the usual
    // purity error, as for the type written.
    let text = "fn f(n = lazy 3): Int { n }";
    assert_implicit_await_error(text, at(text, "lazy 3"), "Future<Int>");
    assert_eq!(messages("fn f(n: Int = lazy 3): Int { n }"), messages(text));
    // A type parameter is what it is bound to.
    assert_eq!(
        file_type("fn f<T>(x: T, y = x): T { y }\na = f(1)\nb = f(\"s\", \"t\")"),
        "{a: Int, b: String}"
    );
    // An enclosing function's parameter the default reads is measured, as
    // an annotation would have it.
    assert_eq!(
        type_of_f("fn f(a: Int): fn(x: Int): Int { fn(x = a): Int { x } }"),
        type_of_f("fn f(a: Int): fn(x: Int): Int { fn(x: Int = a): Int { x } }")
    );
}

#[test]
fn top_level_functions_are_used_in_any_order() {
    // Below its use, by a field and by a function of another group.
    assert_eq!(
        file_type("a = f()\nb = g()\nfn g(): Int { f(1) + f() }\nfn f(n = 3): Int { n }"),
        "{a: Int, b: Int}"
    );
    // A default that calls a function declared below it.
    assert_eq!(
        type_of_f("fn f(n = g()): String { n }\nfn g(): String { \"x\" }"),
        "fn(n: String): String"
    );
    // One whose default calls another's, which is typed by its default.
    assert_eq!(
        type_of_f("fn f(n = g()): String { n }\nfn g(s = \"x\"): String { s }"),
        "fn(n: String): String"
    );
    // Mutual recursion: each sees the other's signature, in either order.
    for text in [
        "fn f(x = g(1)): Int { x }\nfn g(y = 1): Int { if y > 5 { f(y) } else { y } }",
        "fn g(y = 1): Int { if y > 5 { f(y) } else { y } }\nfn f(x = g(1)): Int { x }",
    ] {
        assert_eq!(type_of_f(text), "fn(x: Int): Int", "{text}");
        assert_eq!(
            shown_result(&format!("{text}\ng")),
            "fn(y: Int): Int",
            "{text}"
        );
    }
    // Recursion through its own name.
    assert_eq!(
        type_of_f("fn f(n = 5): Int { if n < 2 { 1 } else { n * f(n - 1) } }"),
        "fn(n: Int): Int"
    );
    // A `def` named by a dotted path, and a method.
    assert_eq!(
        file_type("def a.b.f(n = 2): Int { n }\nx = a.b.f()\ny = a.b.f(3)"),
        "{a: {b: {f: fn(n: Int): Int}}, x: Int, y: Int}"
    );
    assert_eq!(
        file_type(
            "fn (x: Int) add(n = 1): Int { x + n }\n\
             fn (x: String) add(n = \"!\"): String { \"{x}{n}\" }\n\
             a = 1.add()\nb = \"s\".add()\nc = 2.add(n = 5)"
        ),
        "{a: Int, b: String, c: Int}"
    );
    // A method's declaration is checked once its signature is found.
    let text = "fn (x: Int) add(n = 1): Int { x + n }\nfn (x: Int) add(n = 2): Int { x + n }";
    let error = only_error(text);
    assert_eq!(error.message, "method `add` is declared twice for type Int");
    assert_eq!(error.span, nth(text, "add", 1));
    assert_eq!(
        labels(&error),
        vec![(at(text, "add"), "first declared here")]
    );
}

#[test]
fn a_default_cannot_need_its_own_signature() {
    // Its own function.
    let text = "fn f(x = f()): Int { 1 }";
    let error = only_error(text);
    assert_eq!(
        error.message,
        "cannot infer the type of `x`: its default needs the type of `f`, which needs it"
    );
    assert_eq!(error.span, nth(text, "f", 2));
    assert_eq!(
        labels(&error),
        vec![(at(text, "x"), "`x` is written with no type")]
    );
    assert_eq!(
        error.notes,
        vec!["write the parameter's type, as in `x: T = ...`"]
    );
    // Two whose untyped parameters need each other: one error, whichever
    // is written first.
    for text in [
        "fn f(x = g()): Int { 1 }\nfn g(y = f()): Int { 2 }",
        "fn g(y = f()): Int { 2 }\nfn f(x = g()): Int { 1 }",
    ] {
        let found = messages(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert!(
            found[0].starts_with("cannot infer the type of `"),
            "{text}: {found:?}"
        );
        assert!(found[0].ends_with(", which needs it"), "{text}: {found:?}");
    }
    assert_eq!(
        messages("fn f(x = g()): Int { 1 }\nfn g(y = f()): Int { 2 }"),
        vec!["cannot infer the type of `y`: its default needs the type of `f`, which needs it"]
    );
    // Through a function the default only builds, and through a third.
    assert_eq!(
        messages("fn f(x = fn(): Int { f() }): Int { 1 }"),
        vec!["cannot infer the type of `x`: its default needs the type of `f`, which needs it"]
    );
    let found = messages(
        "fn f(x = g()): Int { 1 }\nfn g(y = h()): Int { 2 }\nfn h(z = f()): Int { 3 }\n\
         let a = f()",
    );
    assert_eq!(found.len(), 1, "{found:?}");
    // A method, and a `def` named by a dotted path, by its path.
    assert_eq!(
        messages("fn (x: Int) m(n = 1.m()): Int { n }"),
        vec!["cannot infer the type of `n`: its default needs the type of `m`, which needs it"]
    );
    assert_eq!(
        messages("def a.b.f(n = a.b.f()): Int { n }"),
        vec!["cannot infer the type of `n`: its default needs the type of `a.b.f`, which needs it"]
    );
    // A prefix of its path, the record that holds it.
    let text = "def a.b.f(n = a.b): Int { 1 }\nx = a.b.f";
    let error = only_error(text);
    assert_eq!(
        error.message,
        "cannot infer the type of `n`: its default needs the type of `a.b.f`, which needs it"
    );
    assert_eq!(error.span, nth(text, "a.b", 1));
    assert_eq!(
        messages("def a.b.f(n = a): Int { 1 }\nx = a.b.f"),
        vec!["cannot infer the type of `n`: its default needs the type of `a.b.f`, which needs it"]
    );
    // Another `def` under the prefix is read by its own path.
    assert_eq!(
        file_type("def a.b.h(): Int { 1 }\ndef a.b.g(m = a.b.h()): Int { m }\nx = a.b.g()"),
        "{a: {b: {g: fn(m: Int): Int, h: fn(): Int}}, x: Int}"
    );
    // With the type written, the default may call its function.
    assert_eq!(
        type_of_f("fn f(x: Int = f(1)): Int { x }"),
        "fn(x: Int): Int"
    );
    // A function expression that is no top-level function does not see its
    // own name in a default, as ever.
    assert_eq!(
        messages("fn outer(): Int { let g = fn(x = g()): Int { x }; 1 }"),
        vec!["undefined variable `g`"]
    );
    // A `return` would return from the default.
    let text = "fn f(a = { return 1 }): Int { a }";
    let error = only_error(text);
    assert_eq!(
        error.message,
        "a default its parameter's type is inferred from cannot `return`"
    );
    assert_eq!(
        error.notes,
        vec!["write the parameter's type, or give the default's block a label"]
    );
}

#[test]
fn a_definition_s_own_parameter_has_its_default_s_type() {
    let def_type = "#[def_type]\ndef test(name: String, f: fn() -> String) -> String { f() }\n\
                fn machine(kind: String = \"vm\") -> {name: String} { name = kind }\n";
    assert_eq!(
        file_type(&format!(
            "{def_type}def test t(m = machine(), n = 2, s: String = \"x\") {{ \"{{m.name}} {{n}} \
             {{s}}\" }}\nout = test t()"
        )),
        file_type(&format!(
            "{def_type}def test t(m: {{name: String}} = machine(), n: Int = 2, s: String = \"x\") \
             {{ \"{{m.name}} {{n}} {{s}}\" }}\nout = test t()"
        ))
    );
    // The body uses it as its type.
    let text = format!("{def_type}def test t(n = 2) {{ n }}");
    let error = only_error(&text);
    assert_eq!(error.message, "mismatched types");
    assert_eq!(
        error.label.as_deref(),
        Some("expected String | Future<String>, found Int | Future<Int>")
    );
    // A default with no type of its own, and one that reads the definition.
    assert_eq!(
        messages(&format!("{def_type}def test t(xs = []) {{ \"x\" }}")),
        vec!["cannot infer the element type of `[]`"]
    );
    // An erroneous default is reported once, beside one that is not.
    assert_eq!(
        messages(&format!(
            "{def_type}def test t(a = 1, m = nope, s: String = \"x\") {{ s }}"
        )),
        vec!["undefined variable `nope`"]
    );
    let text = format!("{def_type}def test t(again = test t) {{ \"x\" }}\nout = test t()");
    assert!(check_prelude(&text).2.is_ok());
    // A pure def-type's definition is pure: a future default is the purity
    // error, as with the type written.
    let pure = "#[def_type]\ndef pkg(name: String, f: fn(): Int) -> Int { f() }\n";
    assert_eq!(
        messages(&format!("{pure}def pkg p(n = lazy 1) {{ n }}")),
        messages(&format!("{pure}def pkg p(n: Int = lazy 1) {{ n }}"))
    );
}
