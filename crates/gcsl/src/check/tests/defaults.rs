//! Tests of the checker: parameter defaults, default functions and field
//! defaults.

use super::*;

/// The constant of `default`, which must be one.
pub(super) fn constant_of(default: &ParamDefault) -> Constant {
    match default {
        ParamDefault::Constant(constant) => constant.clone(),
        ParamDefault::Function { .. } => panic!("a default function: {default:?}"),
    }
}

/// The defaults of the function expression numbered `n` of `text`, as
/// the check found them, each a constant.
pub(super) fn defaults_of(text: &str, n: u32) -> Vec<Option<Constant>> {
    let (_, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    output
        .checked
        .resolution
        .function(FnId::new(n))
        .defaults
        .iter()
        .map(|default| default.as_ref().map(constant_of))
        .collect()
}

/// Each call of `text` that gives defaults, as its text, with the
/// parameters it gives them and their constants, in source order: each
/// default a constant.
pub(super) fn given_defaults(text: &str) -> Vec<(&str, Vec<(u32, Constant)>)> {
    let (ast, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let resolution = &output.checked.resolution;
    let mut calls: Vec<(Span, Vec<(u32, Constant)>)> = ast
        .expr_ids()
        .filter(|&id| !resolution.call_defaults(id).is_empty())
        .map(|id| {
            let defaults = resolution.call_defaults(id).iter();
            let constants = defaults.map(|(param, default)| (*param, constant_of(default)));
            (ast.get(id).span, constants.collect())
        })
        .collect();
    calls.sort_by_key(|(span, _)| span.start);
    calls
        .into_iter()
        .map(|(span, defaults)| (&text[span.start as usize..span.end as usize], defaults))
        .collect()
}

/// The string constant `text`.
pub(super) fn constant_str(text: &str) -> Constant {
    Constant::Str(text.into())
}

#[test]
fn a_default_is_a_constant() {
    // Each form, nested freely, as checked against its parameter's
    // type: an integer where a `Float` is expected is a `Float`.
    let text = "fn f(\n\
                    s: String = \"text\",\n\
                    r: String = r\"raw\\n\",\n\
                    i: Int = 42,\n\
                    x: Float = 1.5,\n\
                    y: Float = 2,\n\
                    n: Int = -3,\n\
                    m: Float = -2,\n\
                    k: Float = -0.5,\n\
                    b: Bool = true,\n\
                    l: List<Int> = [1, 2],\n\
                    e: List<Int> = [],\n\
                    rec: {a: Int, s: String} = { a = 1; s = \"s\" },\n\
                    deep: List<{a: List<Float>}> = [{ a = [1, 2.5] }],\n\
                    unit: {} = {},\n\
                    none: Int,\n\
                    ): Int { i }";
    let found = defaults_of(text, 0);
    assert_eq!(
        found,
        vec![
            Some(constant_str("text")),
            Some(constant_str("raw\\n")),
            Some(Constant::Int(42)),
            Some(Constant::Float(1.5f64.to_bits())),
            Some(Constant::Float(2.0f64.to_bits())),
            Some(Constant::Int(-3)),
            Some(Constant::Float((-2.0f64).to_bits())),
            Some(Constant::Float((-0.5f64).to_bits())),
            Some(Constant::Bool(true)),
            Some(Constant::List(
                vec![Constant::Int(1), Constant::Int(2)].into()
            )),
            Some(Constant::List(Box::new([]))),
            found[11].clone(),
            found[12].clone(),
            Some(Constant::Empty),
            None,
        ]
    );
    // A record's fields in the order they are written.
    let (ast, _, _) = check_prelude(text);
    let Some(Constant::Record(fields)) = &found[11] else {
        panic!("expected a record: {:?}", found[11]);
    };
    let names: Vec<&str> = fields
        .iter()
        .map(|&(name, _)| ast.names().get(name))
        .collect();
    assert_eq!(names, ["a", "s"]);
    assert_eq!(fields[0].1, Constant::Int(1));
    assert_eq!(fields[1].1, constant_str("s"));
    let Some(Constant::List(deep)) = &found[12] else {
        panic!("expected a list: {:?}", found[12]);
    };
    let [Constant::Record(fields)] = &deep[..] else {
        panic!("expected a record: {deep:?}");
    };
    assert_eq!(
        fields[0].1,
        Constant::List(
            vec![
                Constant::Float(1.0f64.to_bits()),
                Constant::Float(2.5f64.to_bits())
            ]
            .into()
        )
    );
    // A function without defaults has none.
    assert!(defaults_of("fn f(a: Int): Int { a }", 0).is_empty());
}

#[test]
fn a_default_that_is_no_constant_is_a_default_function() {
    // A name, a call, an interpolated string, a spread, and what is
    // not quite a literal, a list or a record of them: each is checked as
    // its parameter's default function, with no error, and the function's
    // closure keeps it after its own captures.
    let cases = [
        ("let x = 1\nfn f(p: Int = x): Int { p }", "x"),
        ("fn f(p: Int = len([1])): Int { p }", "len([1])"),
        (
            "let x = 1\nfn f(p: String = \"{x}\"): String { p }",
            "\"{x}\"",
        ),
        ("fn f(p: List<Int> = [...[1]]): Int { 0 }", "[...[1]]"),
        (
            "let x = 1\nfn f(p: List<Int> = [1, x]): Int { 0 }",
            "[1, x]",
        ),
        ("fn f(p: Int = (1)): Int { p }", "(1)"),
        ("fn f(p: Int = 1 + 1): Int { p }", "1 + 1"),
        ("fn f(p: Bool = not true): Bool { p }", "not true"),
        ("fn f(p: Int = -(1)): Int { p }", "-(1)"),
        (
            "fn f(p: {a: {b: Int}} = { a.b = 1 }): Int { 0 }",
            "{ a.b = 1 }",
        ),
        (
            "fn f(p: {a: Int} = { a: Int = 1 }): Int { 0 }",
            "{ a: Int = 1 }",
        ),
        (
            "fn f(p: {a: Int} = { let b = 1; a = b }): Int { 0 }",
            "{ let b = 1; a = b }",
        ),
        (
            "fn f(p: {a: Int} = { a = { 1 } }): Int { 0 }",
            "{ a = { 1 } }",
        ),
        ("fn f(p: Int = { 1 }): Int { 0 }", "{ 1 }"),
        ("fn f(p: Future<Int> = lazy 1) -> Int { 0 }", "lazy 1"),
        (
            "fn f(p: fn(): Int = fn(): Int { 1 }): Int { 0 }",
            "fn(): Int { 1 }",
        ),
        ("fn f(p: Int = 1 : Int): Int { p }", "1 : Int"),
    ];
    for (text, _) in cases {
        // `f` is the last function the parser makes: its default function,
        // made as its parameters are parsed, comes before it.
        let (ast, _, output) = check_prelude(text);
        let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        let f = FnId::new(u32::try_from(ast.fn_count() - 1).unwrap());
        let resolution = output.checked.resolution.function(f).clone();
        let capture = u32::try_from(resolution.captures.len()).unwrap();
        assert_eq!(
            resolution.defaults,
            [Some(ParamDefault::Function { capture })],
            "{text}"
        );
    }
}

#[test]
fn a_parameter_a_default_measures_is_present() {
    // The default runs on the arguments before the body: what it measures,
    // the function measures, so a pure function's bare parameter is present
    // by it, and a future for it is the call-site error, labelled there.
    // The default function is function 0, `f` function 1.
    let measured = "fn f(a: Int, b: Int = a * 2): {v: Int} { v = b }\n";
    assert_eq!(fn_type(measured, 1), "fn(a: Int, b: Int): {v: Int}");
    let text = format!("{measured}fn p(): {{v: Int}} {{ f(lazy 4) }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "this argument would be awaited inside `f`, which is pure"
    );
    let measuring = Span::new(at(&text, "a * 2").start, at(&text, "a * 2").start + 1);
    assert!(
        labels(&found[0]).contains(&(measuring, "`a` is measured here")),
        "{:?}",
        labels(&found[0])
    );
    // One it passes on as it is stays the either shape.
    let passed =
        "fn f(a: Int, b: {k: Int, w: Int | Future<Int>} = { k = 1 + 0; w = a }): {k: Int} { b }\n";
    assert_eq!(
        fn_type(passed, 1),
        "fn(a: Int | Future<Int>, b: {k: Int, w: Int | Future<Int>}): {k: Int}"
    );
    // A provisional parameter of an enclosing function a default measures
    // is that function's, as in any pure function nested in it.
    let nested = "fn make(n: Int): Int {\n    let g = fn(m: Int = n * 10): Int { m }\n    g()\n}";
    assert_eq!(fn_type(nested, 2), "fn(n: Int): Int");
}

#[test]
fn a_default_function_shares_its_function_s_parameter_types() {
    // Its copies of the earlier parameters are not resolved again, so an
    // error in one is reported once.
    let found = diagnostics("fn f(a: Nope, b: Int = 1 + 1): Int { b }");
    let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(messages, ["unknown type `Nope`"]);
    // A memoised function's default sees a present parameter, as its body
    // does: adding to it needs no await.
    let text = "let k = 3\nfn m(a: Int, b: Int = a + k) ->> Int { a * b }\nr = m(2)";
    let (ast, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    let start = at(text, "a + k").start;
    let read = ast
        .expr_ids()
        .find(|&id| ast.get(id).span == Span::new(start, start + 1))
        .unwrap();
    assert_eq!(output.checked.resolution.await_of(read), AwaitMark::None);
}

#[test]
fn a_default_is_checked_against_its_parameters_type() {
    let text = "fn f(p: Int = \"no\"): Int { p }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "\"no\""));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "Int"), "expected because of this annotation")]
    );
    assert_eq!(
        messages("fn f(p: List<Int> = [\"a\"]): Int { 0 }"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("fn f(p: {a: Int} = { b = 1 }): Int { 0 }"),
        vec![
            "field `b` is not in `{a: Int}`",
            "missing field `a` in record"
        ]
    );
    // A future is no constant's type, a present value being none.
    assert_eq!(
        messages("fn f(p: Future<Int> = 1) -> Int { await p }"),
        vec!["mismatched types"]
    );
    // The either shape of an impure function's bare parameter takes a
    // constant, as its present member, and so does a union.
    assert!(diagnostics_or_none("fn f(p: {a: Int} = { a = 1 }) -> Int { p.a }").is_empty());
    assert!(diagnostics_or_none("fn f(p: Int | String = 1): String { \"{p}\" }").is_empty());
    assert_eq!(
        defaults_of("fn f(p: Int | String = 1): String { \"{p}\" }", 0),
        vec![Some(Constant::Int(1))]
    );
}

#[test]
fn a_functions_type_does_not_mention_its_defaults() {
    assert_eq!(
        shown_result("fn greet(name: String, greeting: String = \"hi\"): String { name }\ngreet"),
        "fn(name: String, greeting: String | Future<String>): String"
    );
    assert_eq!(
        file_type("def greet(name: String, greeting: String = \"hi\"): String { greeting }"),
        "{greet: fn(name: String | Future<String>, greeting: String): String}"
    );
    assert_eq!(
        shown_result("let f = fn(a: Int = 1) ->> Int { a }\nf"),
        "fn(a: Int) ->> Int"
    );
}

#[test]
fn a_call_that_knows_the_function_gives_the_defaults() {
    let hello = || constant_str("hello");
    // Through a `def`, a named `fn`, a `let` and a field, and with
    // positional, named and trailing arguments around defaults in
    // front of, between and after the parameters without one.
    let text = "def greet(name: String, greeting: String = \"hello\"): String { greeting }\n\
                    fn pad(n: Int = 0, s: String): String { s }\n\
                    let mid = fn(a: Int, b: Int = 2, c: Int): Int { a }\n\
                    run = fn(n: Int = 1, body: {z: Int}, m: Int = 3): Int { n }\n\
                    a = greet(\"jack\")\n\
                    b = greet(\"jack\", \"hi\")\n\
                    c = greet(\"jack\", greeting = \"yo\")\n\
                    d = pad(s = \"x\")\n\
                    e = mid(1, c = 3)\n\
                    g = mid(c = 3, a = 1)\n\
                    h = run(body = { z = 1 })\n\
                    i = run(2, { z = 1 })\n\
                    j = run(m = 4, body = { z = 1 }, n = 5)";
    assert_eq!(
        given_defaults(text),
        vec![
            ("greet(\"jack\")", vec![(1, hello())]),
            ("pad(s = \"x\")", vec![(0, Constant::Int(0))]),
            ("mid(1, c = 3)", vec![(1, Constant::Int(2))]),
            ("mid(c = 3, a = 1)", vec![(1, Constant::Int(2))]),
            (
                "run(body = { z = 1 })",
                vec![(0, Constant::Int(1)), (2, Constant::Int(3))]
            ),
            ("run(2, { z = 1 })", vec![(2, Constant::Int(3))]),
        ]
    );
    // The trailing block fills the last parameter, which here has a
    // default: what comes before it is filled as ever.
    let text = "def run(n: Int = 1, body: {z: Int} = { z = 0 }): Int { n }\n\
                    a = run { z = 1 }\n\
                    b = run()\n\
                    c = run(2) { z = 1 }";
    let given = given_defaults(text);
    assert_eq!(given.len(), 2, "{given:?}");
    assert_eq!(given[0], ("run { z = 1 }", vec![(0, Constant::Int(1))]));
    let ("run()", [first, (1, Constant::Record(fields))]) = (given[1].0, &given[1].1[..]) else {
        panic!("{given:?}");
    };
    assert_eq!(first, &(0, Constant::Int(1)));
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].1, Constant::Int(0));
    // A field of a record read through its record is no binding
    // initialised with the function: its calls fill every parameter.
    assert_eq!(
        messages("r = { f = fn(x: Int = 1): Int { x } }\na = r.f()"),
        vec!["expected 1 argument, found 0"]
    );
}

#[test]
fn a_parameter_left_unfilled_without_a_default_is_reported() {
    // Defaults in front of, between and after it.
    let text = "fn pad(n: Int = 0, s: String): String { s }\npad(1)";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "expected 2 arguments, found 1");
    assert_eq!(found[0].span, at(text, "pad(1)"));
    assert_eq!(
        messages("fn f(a: Int = 1, b: Int, c: Int = 3): Int { b }\nf(c = 1)"),
        vec!["missing argument `b`"]
    );
    assert_eq!(
        messages("fn f(a: Int = 1, b: Int, c: Int = 3): Int { b }\nf(1, 2, 3, 4)"),
        vec!["expected 3 arguments, found 4"]
    );
    assert_eq!(
        messages("fn f(a: Int, b: Int = 2): Int { a }\nf(b = 1)"),
        vec!["missing argument `a`"]
    );
    // A default and an argument for one parameter are no clash.
    assert!(diagnostics_or_none("fn f(a: Int = 1, b: Int = 2): Int { a }\nf(b = 1)").is_empty());
}

#[test]
fn a_call_through_another_function_value_gives_no_defaults() {
    let greet = "def greet(name: String, greeting: String = \"hello\"): String { greeting }\n";
    let cases = [
        // A copy, a parameter, a list's element, a mutable variable.
        "let g = greet\ng(\"jack\")",
        "fn call(f: fn(name: String, greeting: String): String): String { f(\"jack\") }",
        "let fs = [greet]\nfs[0](\"jack\")",
        "let mut g = greet\ng(\"jack\")",
        "(greet)(\"jack\")",
    ];
    for case in cases {
        let text = format!("{greet}{case}");
        assert_eq!(
            messages(&text),
            vec!["expected 2 arguments, found 1"],
            "{case}"
        );
    }
    // Nor a recursive call, through the function's own name or a
    // member of its group.
    assert_eq!(
        messages("fn count(n: Int, acc: Int = 0): Int { if n == 0 { acc } else { count(n - 1) } }"),
        vec!["expected 2 arguments, found 1"]
    );
    assert_eq!(
        messages(
            "def ping(n: Int, d: Int = 0): Int { if n == 0 { d } else { pong(n - 1) } }\n\
                 def pong(n: Int, d: Int = 0): Int { ping(n) }"
        ),
        vec![
            "expected 2 arguments, found 1",
            "expected 2 arguments, found 1"
        ]
    );
    // Nor a def-type reference, whose function is not the def-type.
    let text = "#[def_type]\n\
                    def lock(name: String, func: fn(): String, version: String = \"\") -> String {\n\
                    \x20   version\n\
                    }\n\
                    def lock x() { \"x\" }\n\
                    a = lock x()";
    assert_eq!(messages(text), vec!["expected 1 argument, found 0"]);
    // Called by its name, the def-type gives its default.
    let text = text.replace("a = lock x()", "a = lock(\"x\", fn(): String { \"x\" })");
    assert_eq!(
        given_defaults(&text),
        vec![(
            "lock(\"x\", fn(): String { \"x\" })",
            vec![(2, constant_str(""))]
        )]
    );
}

/// Each record block of `text` that the check gave fields their
/// defaults, as its text, with each such field's name and default, in
/// source order.
pub(super) fn filled(text: &str) -> Vec<(&str, Vec<(String, Constant)>)> {
    let (ast, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let resolution = &output.checked.resolution;
    let mut blocks: Vec<(Span, Vec<(String, Constant)>)> = ast
        .expr_ids()
        .filter_map(|id| {
            let fills = resolution.record_fills(id);
            (!fills.is_empty()).then(|| {
                let fills = fills
                    .iter()
                    .map(|(name, default)| (ast.names().get(*name).to_owned(), default.clone()))
                    .collect();
                (ast.get(id).span, fills)
            })
        })
        .collect();
    blocks.sort_by_key(|(span, _)| span.start);
    blocks
        .into_iter()
        .map(|(span, fills)| (&text[span.start as usize..span.end as usize], fills))
        .collect()
}

/// The name `name` as the check of `text` interns it, for constants
/// [`filled`] finds there.
pub(super) fn name_of(text: &str, name: &str) -> Name {
    let (ast, _, _) = check_prelude(text);
    ast.names()
        .lookup(name)
        .unwrap_or_else(|| panic!("{text}: no name `{name}`"))
}

#[test]
fn a_record_literal_is_given_the_defaults_it_lacks() {
    // Wherever a literal is checked against an expected record type:
    // an annotation, an argument, a list's element, an enclosing
    // literal's field, `:=`, a body's result and a `return`, a
    // parameter's default, a spread that lacks the field, an
    // ascription. A field given, by an item or a spread, is not.
    let text = "def f(e: {a: Int, b: Int = 2}): Int { e.b }\n\
                    let ann: {x: Int = 1, y: Int} = { y = 2 }\n\
                    arg = f({ a = 1 })\n\
                    list: List<{n: String = \"n\", m: Int}> = [{ m = 1 }, { m = 2; n = \"o\" }]\n\
                    outer: {inner: {p: Bool = true, q: Int}} = { inner = { q = 1 } }\n\
                    let mut v: {s: Float = 1, t: Int} = { t = 1; s = 2 }\n\
                    v := { t = 3 }\n\
                    def r(): {u: List<Int> = [1], w: Int} { w = 1 }\n\
                    def r2(k: Bool): {u: Int = 5, w: Int} {\n\
                    \x20   if k { return { w = 1 } }\n\
                    \x20   w = 2\n\
                    }\n\
                    def d(o: {z: Int = 9, y: Int} = { y = 1 }): Int { o.z }\n\
                    spread: {a: Int, b: Int = 2} = { ...{ a = 1 } }\n\
                    given: {a: Int, b: Int = 2} = { ...{ a = 1; b = 3 } }\n\
                    asc = ({ c = 1 } : {c: Int, d: {} = {}})";
    let found = filled(text);
    let one = |name: &str, default: Constant| vec![(name.to_owned(), default)];
    assert_eq!(
        found,
        vec![
            ("{ y = 2 }", one("x", Constant::Int(1))),
            ("{ a = 1 }", one("b", Constant::Int(2))),
            ("{ m = 1 }", one("n", constant_str("n"))),
            ("{ q = 1 }", one("p", Constant::Bool(true))),
            ("{ t = 3 }", one("s", Constant::Float(1.0f64.to_bits()))),
            (
                "{ w = 1 }",
                one("u", Constant::List(Box::new([Constant::Int(1)])))
            ),
            (
                "{\n    if k { return { w = 1 } }\n    w = 2\n}",
                one("u", Constant::Int(5))
            ),
            ("{ w = 1 }", one("u", Constant::Int(5))),
            ("{ y = 1 }", one("z", Constant::Int(9))),
            ("{ ...{ a = 1 } }", one("b", Constant::Int(2))),
            ("{ c = 1 }", one("d", Constant::Empty)),
        ]
    );
    // Several, in the order of their names' text; and the literal's
    // type has each, as the expected type has it, so it reads.
    let text = "let e: {z: Int = 1, a: String = \"a\", m: Int} = { m = 0 }\n\
                    = \"{e.a}{e.z}\"";
    assert_eq!(
        filled(text),
        vec![(
            "{ m = 0 }",
            vec![
                ("a".to_owned(), constant_str("a")),
                ("z".to_owned(), Constant::Int(1))
            ]
        )]
    );
    assert_eq!(
        shown_result("= ({ m = 0 } : {z: Int = 1, a: String = \"a\", m: Int}).z"),
        "Int"
    );
    // A parameter's default given a field's is the constant with it.
    assert_eq!(
        defaults_of("fn d(o: {z: Int = 9, y: Int} = { y = 1 }): Int { o.z }", 0)[0]
            .as_ref()
            .map(|constant| match constant {
                Constant::Record(fields) => fields.len(),
                _ => 0,
            }),
        Some(2)
    );
    // An inline record type with defaults in a parameter's type, an
    // impure function's bare parameter being its either shape.
    let text = "def g(e: {a: Int = 1, b: Int}) -> Int { e.a }\nx = g({ b = 2 })";
    assert_eq!(
        filled(text),
        vec![("{ b = 2 }", one("a", Constant::Int(1)))]
    );
}

#[test]
fn a_field_default_must_be_a_constant() {
    // As a parameter's: a name, a call, an interpolation, a spread, an
    // expression of literals, wherever the type is written.
    let cases = [
        ("let x = 1\nlet p: {a: Int = x} = { a = 1 }", "x"),
        ("let p: {a: Int = len([1])} = { a = 1 }", "len([1])"),
        (
            "let x = 1\nlet p: {a: String = \"{x}\"} = { a = \"\" }",
            "\"{x}\"",
        ),
        ("let p: {a: List<Int> = [...[1]]} = { a = [1] }", "[...[1]]"),
        ("let p: {a: Int = 1 + 1} = { a = 1 }", "1 + 1"),
        ("let p: {a: Int = (1)} = { a = 1 }", "(1)"),
        (
            "let p: {a: {b: Int} = { b: Int = 1 }} = { a = { b = 1 } }",
            "{ b: Int = 1 }",
        ),
        ("def f(e: {a: Int = -(1)}): Int { e.a }", "-(1)"),
        (
            "let f: fn({a: Int = not true}): Int = fn(e: {a: Int}): Int { 1 }",
            "not true",
        ),
    ];
    for (text, default) in cases {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        let error = &found[0];
        assert_eq!(error.message, FIELD_NOT_CONSTANT, "{text}");
        assert_eq!(error.notes, vec![CONSTANT_NOTE.to_owned()], "{text}");
        assert_eq!(
            &text[error.span.start as usize..error.span.end as usize],
            default,
            "{text}"
        );
    }
}

#[test]
fn a_field_default_is_checked_against_its_fields_type() {
    let text = "let p: {a: Int = \"no\"} = { a = 1 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "\"no\""));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "Int"), "expected because of this annotation")]
    );
    assert_eq!(
        messages("let p: {a: List<Int> = [\"a\"]} = { a = [1] }"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("let p: {a: {b: Int} = { c = 1 }} = { a = { b = 1 } }"),
        vec![
            "field `c` is not in `{b: Int}`",
            "missing field `b` in record"
        ]
    );
    // An integer where a `Float` is expected is a `Float`, nested too; a
    // record default lacking a defaulted field of its own type is given
    // it, as any record literal is; and `{}`, the empty record's
    // literal, is given every default.
    let text = "let p: {x: Float = 1, l: List<Float> = [2, 2.5], r: {b: Int, c: Int = 3} = { b = 2 }} = {}\n\
                    = p";
    assert_eq!(
        shown_result(text),
        "{l: List<Float> = [2.0, 2.5], r: {b: Int, c: Int = 3} = {b = 2; c = 3}, \
             x: Float = 1.0}"
    );
    let r = Constant::Record(Box::new([
        (name_of(text, "b"), Constant::Int(2)),
        (name_of(text, "c"), Constant::Int(3)),
    ]));
    let floats = [2.0f64, 2.5].map(|value| Constant::Float(value.to_bits()));
    assert_eq!(
        filled(text),
        vec![
            ("{ b = 2 }", vec![("c".to_owned(), Constant::Int(3))]),
            (
                "{}",
                vec![
                    ("l".to_owned(), Constant::List(Box::new(floats))),
                    ("r".to_owned(), r),
                    ("x".to_owned(), Constant::Float(1.0f64.to_bits())),
                ]
            ),
        ]
    );
    // So is `{}` as the default of a field of a type with defaults.
    let text = "let p: {a: {b: Int = 1} = {}} = {}";
    let b = Constant::Record(Box::new([(name_of(text, "b"), Constant::Int(1))]));
    assert_eq!(
        filled(text),
        vec![
            ("{}", vec![("b".to_owned(), Constant::Int(1))]),
            ("{}", vec![("a".to_owned(), b)]),
        ]
    );
    // Checked once, however often its type is resolved.
    assert_eq!(
        messages("def f(e: {a: Int = \"x\"}): {a: Int = true} { a = 1 }\nb = f({ a = 2 })"),
        vec!["mismatched types", "mismatched types"]
    );
}

#[test]
fn the_empty_record_literal_is_given_defaults() {
    // `{}` where a record type with defaults is expected itself: an
    // annotation, an argument, a list's element, through parentheses,
    // a function's body and a parameter's default.
    let text = "def f(e: {a: Int = 2}): Int { e.a }\n\
                    let v: {a: Int = 1, s: String = \"s\"} = {}\n\
                    x = f(({}))\n\
                    l: List<{k: Int = 4}> = [{}, { k = 5 }]\n\
                    def r(): {q: Bool = true} {}\n\
                    def d(o: {z: Int = 9} = {}): Int { o.z }";
    let one = |name: &str, default: Constant| vec![(name.to_owned(), default)];
    assert_eq!(
        filled(text),
        vec![
            (
                "{}",
                vec![
                    ("a".to_owned(), Constant::Int(1)),
                    ("s".to_owned(), constant_str("s"))
                ]
            ),
            ("{}", one("a", Constant::Int(2))),
            ("{}", one("k", Constant::Int(4))),
            ("{}", one("q", Constant::Bool(true))),
            ("{}", one("z", Constant::Int(9))),
        ]
    );
    assert_eq!(
        shown_result("let v: {a: Int = 1, s: String = \"s\"} = {}\n= v.s"),
        "String"
    );
    // A parameter's default that is `{}` is the record of what it is
    // given.
    assert_eq!(
        defaults_of("fn d(o: {z: Int = 9} = {}): Int { o.z }", 0),
        vec![Some(Constant::Record(Box::new([(
            name_of("fn d(o: {z: Int = 9} = {}): Int { o.z }", "z"),
            Constant::Int(9)
        )])))]
    );
    // A field without a default is missing, as for any record literal.
    let text = "let w: {a: Int, b: Int = 1} = {}";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "missing field `a` in record");
    assert_eq!(found[0].span, last(text, "}"));
    // A union expected, or a record type without defaults, or none
    // expected: `{}` is checked as ever.
    assert_eq!(
        messages("let u: {a: Int = 1} | String = {}"),
        vec!["mismatched types"]
    );
    assert_eq!(messages("let p: {a: Int} = {}"), vec!["mismatched types"]);
    assert!(filled("let e: {} = {}\nlet f = {}").is_empty());
    assert_eq!(shown_result("let e = {}\n= e"), "{}");
}

#[test]
fn only_a_record_literal_is_given_defaults() {
    let note = "a default fills only a record literal written where \
                    `{b: Int = 2, c: Int}` is expected";
    let def = "def f(e: {b: Int = 2, c: Int}): Int { e.b }\n";
    // A variable, a call's result, a field read, a literal checked with
    // no expected type and used later: the usual mismatch, noted.
    let cases = [
        ("let plain = { c = 1 }\nm = f(plain)", "plain"),
        ("fn mk(): {c: Int} { c = 1 }\nm = f(mk())", "mk()"),
        ("let o = { i = { c = 1 } }\nm = f(o.i)", "o.i"),
    ];
    for (case, at_text) in cases {
        let text = format!("{def}{case}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{case}: {found:?}");
        assert_eq!(found[0].message, "mismatched types", "{case}");
        assert_eq!(found[0].span, last(&text, at_text), "{case}");
        assert_eq!(
            found[0].label.as_deref(),
            Some("expected {b: Int = 2, c: Int}, found {c: Int}"),
            "{case}"
        );
        assert_eq!(found[0].notes, vec![note.to_owned()], "{case}");
    }
    // Where the record meets the type inside a list or a field too.
    let found = diagnostics(
        "let xs = [{ c = 1 }]\nlet ys: List<{b: Int = 2, c: Int}> = xs\n\
             let o = { r = { c = 1 } }\nlet p: {r: {b: Int = 2, c: Int}} = o",
    );
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].notes, vec![note.to_owned()]);
    assert_eq!(found[1].notes, vec![note.to_owned()]);
    // A spread's field, met where its record's field is expected.
    let found =
        diagnostics("let i = { c = 1 }\nlet p: {r: {b: Int = 2, c: Int}} = { ...{ r = i } }");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].notes, vec![note.to_owned()]);
    // No note where nothing defaulted is lacking.
    let found = diagnostics(&format!("{def}m = f(1)"));
    assert!(found[0].notes.is_empty(), "{found:?}");
    // A union of record types expected fills nothing.
    let text = "let p: {a: Int = 1, b: Int} | String = { b = 2 }";
    assert_eq!(messages(text), vec!["missing field `a` in record"]);
    // A field given is checked as ever.
    assert_eq!(
        messages(&format!("{def}m = f({{ c = 1; b = \"x\" }})")),
        vec!["mismatched types"]
    );
}

#[test]
fn defaults_change_no_types_identity() {
    // Assignable and equal both ways; joined and listed without a
    // union.
    let text = "let p: {a: Int = 1} = { a = 2 }\n\
                    let q: {a: Int} = p\n\
                    let r: {a: Int = 5} = q\n\
                    e = p == q\n\
                    t = q === r\n\
                    u = r != p\n\
                    j = if e { p } else { q }\n\
                    l = [p, q, r]";
    assert!(
        diagnostics_or_none(text).is_empty(),
        "{:?}",
        diagnostics_or_none(text)
    );
    assert!(warnings(text).is_empty(), "{:?}", warnings(text));
    // Shown with their defaults in an error.
    let found = diagnostics("let s: {a: Int = 1, b: List<String> = []} = 5");
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected {a: Int = 1, b: List<String> = []}, found Int")
    );
    // A function's type shows the defaults of its parameters' record
    // types, which are part of them.
    assert_eq!(
        file_type("def f(e: {a: Int = 1}): Int { e.a }"),
        "{f: fn(e: {a: Int = 1}): Int}"
    );
}

/// The type `T` of the tests of where a record type with defaults
/// reaches a literal through a block, an `if` or a `for`.
pub(super) const REACH_T: &str = "type T = { a: Int, b: Int = 2 }\n";

#[test]
fn the_workspace_forms_are_given_defaults() {
    // A spread `for` under an expected list, a `for` that is a
    // function's result, and a body block whose result is the literal.
    // Each is read with no test.
    let text = "type Install = { from: String, to: String, exclude: List<String> = [] }\n\
                    content: List<Install> = [ ...for name in [\"zstd\", \"unzstd\"] { { from = \"dest/usr/bin/{name}\"; to = \"usr/bin/{name}\" } } ]\n\
                    fn licenses(name: String): List<Install> { for file in [\"COPYING\", \"LGPL-2.1\"] { { from = \"src/{file}\"; to = \"x\" } } }\n\
                    fn license(name: String): Install { { from = \"src/LICENSE\"; to = \"y\" } }\n\
                    read: List<List<String>> = [license(\"x\").exclude, licenses(\"x\")[0].exclude, content[0].exclude]";
    let exclude = || vec![("exclude".to_owned(), Constant::List(Box::new([])))];
    assert_eq!(
        filled(text),
        vec![
            (
                "{ from = \"dest/usr/bin/{name}\"; to = \"usr/bin/{name}\" }",
                exclude()
            ),
            ("{ from = \"src/{file}\"; to = \"x\" }", exclude()),
            ("{ from = \"src/LICENSE\"; to = \"y\" }", exclude()),
        ]
    );
    assert!(warnings(text).is_empty(), "{:?}", warnings(text));
}

#[test]
fn a_block_result_is_given_defaults_at_any_depth() {
    // Nested blocks, a `= EXPR` result through parentheses, a labelled
    // block, a function's body, `{}`, and a `break`'s value.
    let text = format!(
        "{REACH_T}type D = {{ d: Int = 7 }}\n\
             x1: T = {{ {{ a = 1 }} }}\n\
             x2: T = {{ let y = 1; {{ let z = 2; {{ a = y + z }} }} }}\n\
             x3: T = ({{ = ({{ a = 3 }}) }})\n\
             x4: T = {{'l {{ a = 4 }} }}\n\
             fn f(): T {{ {{ {{ a = 5 }} }} }}\n\
             fn e(): D {{ {{ {{}} }} }}\n\
             bs: List<T> = for i in [1, 2] {{ if i == 2 {{ break {{ {{ a = i }} }} }}; {{ a = i }} }}\n\
             read: List<Int> = [f().b, x2.b, e().d]"
    );
    let b = || vec![("b".to_owned(), Constant::Int(2))];
    assert_eq!(
        filled(&text),
        vec![
            ("{ a = 1 }", b()),
            ("{ a = y + z }", b()),
            ("{ a = 3 }", b()),
            ("{ a = 4 }", b()),
            ("{ a = 5 }", b()),
            ("{}", vec![("d".to_owned(), Constant::Int(7))]),
            ("{ a = i }", b()),
            ("{ a = i }", b()),
        ]
    );
}

#[test]
fn if_branches_are_given_defaults() {
    // Every branch of an `if` with `else`, `else if` included, a branch
    // that is a record block itself, nested in a block; a branch that
    // gives the field is not given it.
    let text = format!(
        "{REACH_T}fn g(n: Int): T {{ if n == 0 {{ {{ a = 0 }} }} else if n == 1 {{ a = 1 }} else {{ {{ a = 2; b = 3 }} }} }}\n\
             let w: T = {{ if true {{ {{ a = 4 }} }} else {{ {{ a = 5 }} }} }}"
    );
    let b = || vec![("b".to_owned(), Constant::Int(2))];
    assert_eq!(
        filled(&text),
        vec![
            ("{ a = 0 }", b()),
            ("{ a = 1 }", b()),
            ("{ a = 4 }", b()),
            ("{ a = 5 }", b()),
        ]
    );
    assert!(warnings(&text).is_empty(), "{:?}", warnings(&text));
    // An `if` without `else` keeps its error.
    assert_eq!(
        messages(&format!(
            "{REACH_T}fn h(c: Bool): T {{ if c {{ {{ a = 1 }} }} }}"
        )),
        vec!["`if` without `else` must be `{}`"]
    );
}

#[test]
fn for_bodies_are_given_defaults() {
    // Under `List<T>` as an annotation, an argument, a return, a spread
    // beside plain elements, and nested.
    let text = format!(
        "{REACH_T}xs: List<T> = for i in [1, 2] {{ {{ a = i }} }}\n\
             def count(ts: List<T>): Int {{ len(ts) }}\n\
             n = count(for i in [1] {{ {{ a = i + 1 }} }})\n\
             fn r(): List<T> {{ for i in [1] {{ {{ a = i + 2 }} }} }}\n\
             ys: List<T> = [{{ a = 0 }}, ...for i in [1, 2] {{ {{ a = i + 3 }} }}, {{ a = 9; b = 9 }}]\n\
             zs: List<List<T>> = for i in [1] {{ for j in [2] {{ {{ a = i + j }} }} }}\n\
             direct: List<T> = [...for i in [1] {{ a = i }}]"
    );
    let b = || vec![("b".to_owned(), Constant::Int(2))];
    assert_eq!(
        filled(&text),
        vec![
            ("{ a = i }", b()),
            ("{ a = i + 1 }", b()),
            ("{ a = i + 2 }", b()),
            ("{ a = 0 }", b()),
            ("{ a = i + 3 }", b()),
            ("{ a = i + j }", b()),
            ("{ a = i }", b()),
        ]
    );
    assert!(warnings(&text).is_empty(), "{:?}", warnings(&text));
    // Under an expected union element, nothing is given.
    let text = format!("{REACH_T}us: List<T | Int> = for i in [1] {{ {{ a = i }} }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "missing field `b` in record");
    let brace = last(&text, "i }");
    assert_eq!(found[0].span, Span::new(brace.end - 1, brace.end));
}

#[test]
fn only_a_literal_result_is_given_defaults() {
    let note = "a default fills only a record literal written where \
                    `{a: Int, b: Int = 2}` is expected";
    // A result, or a branch, that is no literal: the usual mismatch,
    // noted, at the block or the `if` as a whole.
    let cases = [
        (
            "fn plain(): T { let q = { a = 1 }; q }",
            "{ let q = { a = 1 }; q }",
        ),
        (
            "let q = { a = 1 }\nfn branch(c: Bool): T { if c { q } else { { a = 2 } } }",
            "{ if c { q } else { { a = 2 } } }",
        ),
        (
            "let q = { a = 1 }\nlet v: T = if true { { a = 2 } } else { q }",
            "if true { { a = 2 } } else { q }",
        ),
    ];
    for (case, at_text) in cases {
        let text = format!("{REACH_T}{case}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{case}: {found:?}");
        assert_eq!(found[0].message, "mismatched types", "{case}");
        assert_eq!(found[0].span, last(&text, at_text), "{case}");
        assert_eq!(
            found[0].label.as_deref(),
            Some("expected {a: Int, b: Int = 2}, found {a: Int}"),
            "{case}"
        );
        assert_eq!(found[0].notes, vec![note.to_owned()], "{case}");
    }
    // A literal that gives every defaulted field, or has a spread, is
    // checked as ever: no warning of a field the type lacks, a `[]` not
    // inferred, a mismatch at the block.
    let text = format!("{REACH_T}fn e(): T {{ {{ a = 1; b = 2; c = 3 }} }}");
    assert!(warnings(&text).is_empty(), "{:?}", warnings(&text));
    assert!(filled(&text).is_empty());
    assert_eq!(
        messages("type L = { a: Int, l: List<Int> = [] }\nfn e(): L { { a = 1; l = [] } }"),
        vec!["cannot infer the element type of `[]`"]
    );
    let text = format!("{REACH_T}fn q(): T {{ {{ b = 1 }} }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].span, last(&text, "{ { b = 1 } }"));
    let text = format!("{REACH_T}fn s(): T {{ {{ ...{{ a = 1 }} }} }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].span, last(&text, "{ { ...{ a = 1 } } }"));
    // A union expected gives nothing, as before.
    assert_eq!(
        messages(&format!("{REACH_T}let u: T | String = {{ {{ a = 1 }} }}")),
        vec!["missing field `b` in record"]
    );
    // Nor does a labelled block's `return`, whose value is checked as
    // ever, joined with the block's result.
    let text =
        format!("{REACH_T}fn lb(c: Bool): T {{'l if c {{ return'l {{ a = 1 }} }}; {{ a = 2 }} }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].notes, vec![note.to_owned()]);
}

#[test]
fn a_mismatch_in_a_given_literal() {
    // Inside a literal given the defaults, a field is checked against
    // the type's, as any literal's is where it is expected: reported at
    // the field.
    let text = format!(
        "{REACH_T}fn m(c: Bool): T {{ if c {{ {{ a = \"x\" }} }} else {{ {{ a = 2 }} }} }}"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(&text, "\"x\""));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found String")
    );
    // One reported inside the branch before is reported there still,
    // and alone.
    let text = format!(
        "{REACH_T}fn k(c: Bool): T {{ if c {{ {{ a = 1 + \"x\" }} }} else {{ {{ a = 2 }} }} }}"
    );
    assert_eq!(messages(&text), vec!["cannot add Int and String"]);
    assert_eq!(diagnostics(&text)[0].span, at(&text, "+"));
    // A literal missing another field is missing it, at its brace.
    let text = "type U = { a: Int, c: Int, b: Int = 2 }\nfn u(): U { { { a = 1 } } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "missing field `c` in record");
    let brace = last(text, "1 }");
    assert_eq!(found[0].span, Span::new(brace.end - 1, brace.end));
}

/// What the tests of spreads of an `if` or a block bind beside
/// [`REACH_T`]: an empty list of `T`, a list of records lacking `b`
/// that is no literal, and a list of numbers.
pub(super) const SPREAD_NAMES: &str = "let none: List<T> = []\n\
                                let plain = [{ a = 1 }]\n\
                                let ns = [1, 2]\n";

#[test]
fn the_workspace_spread_if_is_given_defaults() {
    // The form verbatim, with a stub of `kernel_lane`.
    let text = "type Install = { from: String, to: String, exclude: List<String> = [] }\n\
                    fn kernel_lane(v: String): Bool { v == \"kernel\" }\n\
                    let v = \"6.1\"\n\
                    none_installed: List<Install> = []\n\
                    content: List<Install> = [\n\
                    \x20   { from = \"a\"; to = \"b\" },\n\
                    \x20   ...if kernel_lane(v) { none_installed } else {\n\
                    \x20       [ { from = \"src/src/tools/cargo/LICENSE-MIT\"; to = \"c\" } ]\n\
                    \x20   },\n\
                    ]";
    let exclude = || vec![("exclude".to_owned(), Constant::List(Box::new([])))];
    assert_eq!(
        filled(text),
        vec![
            ("{ from = \"a\"; to = \"b\" }", exclude()),
            (
                "{ from = \"src/src/tools/cargo/LICENSE-MIT\"; to = \"c\" }",
                exclude()
            ),
        ]
    );
    assert!(warnings(text).is_empty(), "{:?}", warnings(text));
}

#[test]
fn spreads_of_ifs_and_blocks_are_given_defaults() {
    // Every branch of an `if` with `else`, `else if` included, beside a
    // literal that gives the field; a block's result, after a `let`,
    // labelled and nested, and a `= EXPR` result through parentheses.
    let text = format!(
        "{REACH_T}{SPREAD_NAMES}\
             fn pick(n: Int): List<T> {{ [ ...if n == 0 {{ [ {{ a = 1 }} ] }} else if n == 1 {{ [ {{ a = 2 }}, {{ a = 3; b = 4 }} ] }} else {{ [ {{ a = 5 }} ] }} ] }}\n\
             blocks: List<T> = [ ...{{ let y = 6; [ {{ a = y }} ] }}, ...{{'l {{ [ {{ a = 7 }} ] }} }}, ...({{ = [ {{ a = 8 }} ] }}) ]\n\
             read: List<Int> = [pick(0)[0].b, blocks[2].b]"
    );
    let b = || vec![("b".to_owned(), Constant::Int(2))];
    assert_eq!(
        filled(&text),
        vec![
            ("{ a = 1 }", b()),
            ("{ a = 2 }", b()),
            ("{ a = 5 }", b()),
            ("{ a = y }", b()),
            ("{ a = 7 }", b()),
            ("{ a = 8 }", b()),
        ]
    );
    assert!(warnings(&text).is_empty(), "{:?}", warnings(&text));
}

#[test]
fn spreads_of_ifs_and_fors_nest() {
    // A `...for` in a spread `if`'s list, a `for` that is a branch, an
    // `if` in a spread `for`'s body, a `...if` in a spread `if`'s list,
    // and a `...{ }` in a spread block's list.
    let text = format!(
        "{REACH_T}{SPREAD_NAMES}\
             for_in_if: List<T> = [ ...if true {{ [ ...for n in ns {{ {{ a = n }} }} ] }} else {{ none }} ]\n\
             for_result: List<T> = [ ...if false {{ none }} else {{ for n in ns {{ {{ a = n * 10 }} }} }} ]\n\
             if_in_for: List<T> = [ ...for n in ns {{ if n == 1 {{ {{ a = n }} }} else {{ {{ a = n + 1 }} }} }} ]\n\
             if_in_if: List<T> = [ ...if true {{ [ ...if false {{ none }} else {{ [ {{ a = 11 }} ] }}, {{ a = 12 }} ] }} else {{ none }} ]\n\
             block_in_block: List<T> = [ ...{{ [ ...{{ [ {{ a = 13 }} ] }} ] }} ]"
    );
    let b = || vec![("b".to_owned(), Constant::Int(2))];
    assert_eq!(
        filled(&text),
        vec![
            ("{ a = n }", b()),
            ("{ a = n * 10 }", b()),
            ("{ a = n }", b()),
            ("{ a = n + 1 }", b()),
            ("{ a = 11 }", b()),
            ("{ a = 12 }", b()),
            ("{ a = 13 }", b()),
        ]
    );
    assert!(warnings(&text).is_empty(), "{:?}", warnings(&text));
}

#[test]
fn other_spreads_are_checked_as_before() {
    let found_t = "found List<{a: Int}>";
    // A branch that is a name or a call, a list literal spread itself,
    // and an `if` without `else`: the usual mismatch at the operand.
    let cases = [
        (
            "[...if true { plain } else { none }]",
            "if true { plain } else { none }",
        ),
        (
            "[...if true { id(plain) } else { none }]",
            "if true { id(plain) } else { none }",
        ),
        ("[...[ { a = 1 } ]]", "[ { a = 1 } ]"),
    ];
    for (list, operand) in cases {
        let text = format!(
            "{REACH_T}{SPREAD_NAMES}fn id(x: List<{{a: Int}}>): List<{{a: Int}}> {{ x }}\n\
                 xs: List<T> = {list}"
        );
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{list}: {found:?}");
        assert_eq!(found[0].message, "mismatched types", "{list}");
        assert_eq!(found[0].span, last(&text, operand), "{list}");
        assert_eq!(
            found[0].label.as_deref(),
            Some(format!("expected List<{{a: Int, b: Int = 2}}>, {found_t}").as_str()),
            "{list}"
        );
    }
    let text = format!("{REACH_T}xs: List<T> = [...if true {{ [ {{ a = 1 }} ] }}]");
    assert_eq!(messages(&text), vec!["`if` without `else` must be `{}`"]);
    // A union expected of the elements gives nothing, the branches
    // joined as ever.
    let text =
        format!("{REACH_T}us: List<T | Int> = [...if true {{ [ {{ a = 1 }} ] }} else {{ [2] }}]");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        found[1].message,
        "the branches of this `if` have different types"
    );
    let operand = at(&text, "if true");
    assert_eq!(
        found[0].span,
        Span::new(operand.start, at(&text, "] }]").end - 1)
    );
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected List<Int | {a: Int, b: Int = 2}>, found List<Int | {a: Int}>")
    );
    // `[]` is a branch's list, not inferred.
    let text = format!("{REACH_T}xs: List<T> = [...if true {{ [] }} else {{ [ {{ a = 1 }} ] }}]");
    assert_eq!(
        messages(&text),
        vec!["cannot infer the element type of `[]`"]
    );
    // A record type without defaults reaches nothing.
    let text = "type P = { a: Int }\n\
                    xs: List<P> = [...if true { [ { a = \"x\" } ] } else { [ { a = 1 } ] }]";
    let found = diagnostics(text);
    assert_eq!(found.len(), 2, "{found:?}");
    let operand = at(text, "if true");
    assert_eq!(
        found[0].span,
        Span::new(operand.start, at(text, "] }]").end - 1)
    );
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected List<{a: Int}>, found List<{a: Int | String}>")
    );
    assert_eq!(
        found[1].message,
        "the branches of this `if` have different types"
    );
}

#[test]
fn a_mismatch_in_a_spread_literal() {
    // A literal given the defaults has its fields checked against the
    // type's: a mismatch at the field, and an extra field a warning.
    let text = format!(
        "{REACH_T}{SPREAD_NAMES}xs: List<T> = [...if true {{ [ {{ a = \"x\" }} ] }} else {{ none }}]"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(&text, "\"x\""));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found String")
    );
    let text = format!("{REACH_T}{SPREAD_NAMES}xs: List<T> = [...{{ [ {{ a = 1; c = 3 }} ] }}]");
    let found = warnings(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "field `c` is not in `{a: Int, b: Int = 2}`"
    );
    // One reported inside the literal before is reported there still,
    // and alone.
    let text = format!(
        "{REACH_T}{SPREAD_NAMES}xs: List<T> = [...if true {{ [ {{ a = 1 + \"x\" }} ] }} else {{ none }}]"
    );
    assert_eq!(messages(&text), vec!["cannot add Int and String"]);
}
