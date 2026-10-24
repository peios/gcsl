//! Tests of the parser: function expressions, defs, named functions, calls,
//! function types, `lazy`, `await` and applied types.

use super::*;

/// The expression of the file's only item, which must be an expression
/// or a `let`.
pub(super) fn only_value(ast: &Ast) -> ExprId {
    match ast.item(ast.items(ast.root())[0]).kind {
        ItemKind::Expr(value) | ItemKind::Let { value, .. } => value,
        other => panic!("expected an expression or a let, got {other:?}"),
    }
}

#[test]
fn function_expressions() {
    assert_eq!(shape("fn(): Int { 1 }"), "fn(): Int {1}");
    assert_eq!(shape("fn(x: Int): Int { x }"), "fn(x: Int): Int {x}");
    assert_eq!(
        shape("fn(name: String, times: Int) -> String { name }"),
        "fn(name: String, times: Int) -> String {name}"
    );
    // A trailing comma, and a parameter list over several lines.
    assert_eq!(shape("fn(x: Int,): Int { x }"), "fn(x: Int): Int {x}");
    assert_eq!(
        shape("let f = fn(\n  x: Int,\n  y: Int,\n): Int { x + y }"),
        "let f = fn(x: Int, y: Int): Int {(x + y)}"
    );
    // Newlines after the marker, and a body over several lines.
    assert_eq!(
        shape("fn(n: Int) ->\n  {} {\n  let m = n\n  m\n}"),
        "fn(n: Int) -> {} {let m = n; m}"
    );
    assert_eq!(shape("fn():\nInt { 1 }"), "fn(): Int {1}");
    // A record return type followed by the body, and function types as
    // parameter and return types.
    assert_eq!(shape("fn(): {x: Int} { x = 1 }"), "fn(): {x: Int} {x = 1}");
    assert_eq!(
        shape("fn(f: fn(Int): Int): fn() -> Int { f }"),
        "fn(f: fn(Int): Int): fn() -> Int {f}"
    );
    // An operand like any other.
    assert_eq!(shape("fn(): Int { 1 } == g"), "(fn(): Int {1} == g)");
    // The details: parameters with their spans, the marker and purity.
    let ast = parse_text("fn(x: Int, yy: {a: Int}) -> Int { x }").unwrap();
    let expr = ast.get(only_value(&ast));
    assert_eq!(expr.span, Span::new(0, 37));
    let ExprKind::Fn(function) = expr.kind else {
        panic!("expected a function, got {expr:?}");
    };
    assert_eq!(ast.fn_count(), 1);
    let decl = ast.fn_decl(function);
    assert_eq!(decl.effect, Effect::Impure);
    assert_eq!(decl.marker, Span::new(25, 27));
    assert_eq!(ast.type_expr(decl.ret.unwrap()).span, Span::new(28, 31));
    assert_eq!(ast.get(decl.body).span, Span::new(32, 37));
    let spans: Vec<Span> = ast.params(decl.params).iter().map(|p| p.span).collect();
    assert_eq!(spans, vec![Span::new(3, 9), Span::new(11, 23)]);
    assert_eq!(decl.params.len(), 2);
    assert!(!decl.params.is_empty());
    let ast = parse_text("fn(): Int { 1 }").unwrap();
    let ExprKind::Fn(function) = ast.get(only_value(&ast)).kind else {
        panic!("expected a function");
    };
    assert_eq!(ast.fn_decl(function).effect, Effect::Pure);
    assert!(ast.fn_decl(function).params.is_empty());
}

#[test]
fn parameters_may_have_defaults() {
    // In any position, any expression, in a `fn` expression, a named
    // `fn`, a `def` and a def-type's `def`; what a default may be is the
    // checker's business.
    assert_eq!(
        shape("fn(a: Int = 1, b: String, c: List<Int> = []): Int { a }"),
        "fn(a: Int = 1, b: String, c: List<Int> = list()): Int {a}"
    );
    assert_eq!(
        shape("fn f(x: Int = -1) -> Int { x }"),
        "fn f(x: Int = (-1)) -> Int {x}"
    );
    assert_eq!(
        shape("def greet(name: String, greeting: String = \"hello\"): String { greeting }"),
        "def greet(name: String, greeting: String = \"hello\"): String {greeting}"
    );
    assert_eq!(
        shape("#[def_type]\ndef p(n: String, f: fn(): Int, v: String = r\"x\") -> Int { 1 }"),
        "def p(n: String, f: fn(): Int, v: String = \"x\") -> Int {1}"
    );
    assert_eq!(
        shape("fn(r: {a: Int} = { a = 1 }, s: Int = g(x) + 1): Int { 1 }"),
        "fn(r: {a: Int} = {a = 1}, s: Int = (g(x) + 1)): Int {1}"
    );
    // Newlines inside the parentheses, around the `=` too.
    assert_eq!(
        shape("fn(\n  a: Int\n  = 1,\n  b: Int =\n  2,\n): Int { a }"),
        "fn(a: Int = 1, b: Int = 2): Int {a}"
    );
    // The parameter's span is its name through its type; the default is
    // parsed before the body, and so is the first node of the
    // function's subtree.
    let text = "fn(x: Int, y: {a: Int} = { a = 1 }, z: Bool = true): Int { x }";
    let ast = parse_text(text).unwrap();
    let value = only_value(&ast);
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let params = ast.params(ast.fn_decl(function).params);
    let spans: Vec<Span> = params.iter().map(|p| p.span).collect();
    assert_eq!(
        spans,
        vec![Span::new(3, 9), Span::new(11, 22), Span::new(36, 43)]
    );
    let defaults: Vec<Option<Span>> = params
        .iter()
        .map(|p| p.default.map(|default| ast.get(default).span))
        .collect();
    assert_eq!(
        defaults,
        vec![None, Some(Span::new(25, 34)), Some(Span::new(46, 50))]
    );
    let first = ast.subtree(value).next().unwrap();
    assert_eq!(ast.get(first).span, Span::new(31, 32));
    assert_eq!(ast.subtree(value).count(), ast.len());
    // The tree dump shows a default after its parameter's type.
    assert_eq!(
        parse_text("fn(a: Int, b: Int = 2): Int { a }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (expr @0..33\n    (fn @0..33\n      (params (a Int) (b Int\n        \
             (int 2 @20..21)))\n      (ret pure Int)\n      (block @28..33\n        \
             (expr @30..31\n          (name a @30..31))))))"
    );
}

#[test]
fn a_default_is_written_only_in_a_function_expression() {
    // Not in a function type, named or not, nested or not, at its `=`.
    assert_eq!(
        error("let f: fn(a: Int = 1): Int = g"),
        (MISPLACED_DEFAULT.to_owned(), Span::new(17, 18))
    );
    assert_eq!(
        error("let f: fn(Int = 1): Int = g"),
        (MISPLACED_DEFAULT.to_owned(), Span::new(14, 15))
    );
    assert_eq!(
        error("fn(f: fn(x: Int = 1): Int): Int { 1 }"),
        (MISPLACED_DEFAULT.to_owned(), Span::new(16, 17))
    );
    // A function type as a parameter's type, followed by that
    // parameter's default.
    assert_eq!(
        shape("fn(f: fn(Int): Int = g): Int { 1 }"),
        "fn(f: fn(Int): Int = g): Int {1}"
    );
    // A definition's parameters are names alone, as before.
    assert_eq!(
        error("def p x(v = \"x\") { 1 }"),
        (
            "expected `,` or `)`, found `=`".to_owned(),
            Span::new(10, 11)
        )
    );
}

#[test]
fn memoised_functions() {
    // A function expression, a named item, function types and nested
    // function types, each with `->>`, which newlines may follow.
    assert_eq!(
        shape("fn(url: String) ->> String { url }"),
        "fn(url: String) ->> String {url}"
    );
    assert_eq!(
        shape("fn(n: Int) ->>\n  Int {\n  n\n}"),
        "fn(n: Int) ->> Int {n}"
    );
    assert_eq!(
        shape("fn(f: fn(Int) ->> Int): fn() ->> fn(x: Int) ->> Int { f }"),
        "fn(f: fn(Int) ->> Int): fn() ->> fn(x: Int) ->> Int {f}"
    );
    assert_eq!(
        shape("fn(f: fn(fn(Int) ->> Int) -> Int) ->> Int { 1 }"),
        "fn(f: fn(fn(Int) ->> Int) -> Int) ->> Int {1}"
    );
    let text = "fn fetch(url: String) ->> String { url }";
    let ast = parse_text(text).unwrap();
    let ItemKind::Let { value, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let decl = ast.fn_decl(function);
    assert_eq!(decl.effect, Effect::Memo);
    // The marker spans the whole `->>`.
    assert_eq!(decl.marker, Span::new(22, 25));
    assert_eq!(&text[22..25], "->>");
    // A type annotation with a memoised function type.
    let ast = parse_text("let f: fn(Int) ->> Int = g").unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated let");
    };
    assert!(matches!(
        ast.type_expr(ty).kind,
        TypeExprKind::Fn {
            effect: Effect::Memo,
            ..
        }
    ));
    // Printed, it reads back as itself.
    let shown = shape("fn(f: fn(Int) ->> Int) ->> fn() ->> Int { f }");
    assert_eq!(shown, "fn(f: fn(Int) ->> Int) ->> fn() ->> Int {f}");
    assert_eq!(shape(&shown), shown);
    // The tree's debug form shows the effect, and the types as written.
    assert_eq!(
        parse_text("fn f(g: fn(Int) ->> Int) ->> Int { 1 }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n\
             \x20 (fn f @0..38\n\
             \x20   (fn @0..38\n\
             \x20     (params (g fn(Int) ->> Int))\n\
             \x20     (ret memo Int)\n\
             \x20     (block @33..38\n\
             \x20       (expr @35..36\n\
             \x20         (int 1 @35..36))))))"
    );
    // `->>>` is the memo arrow and a `>`, which cannot start a type.
    assert_eq!(
        error("fn(x: Int) ->>> Int { x }"),
        ("expected a type".to_owned(), Span::new(14, 15))
    );
}

#[test]
fn function_expression_errors() {
    let diagnostic = parse_text("fn(x): Int { x }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected `:` and a type after parameter `x`"
    );
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(diagnostic.notes, vec!["parameters must be annotated"]);
    assert_eq!(
        error("fn(x, y: Int): Int { x }"),
        (
            "expected `:` and a type after parameter `x`".to_owned(),
            Span::new(4, 5)
        )
    );
    let diagnostic = parse_text("fn(x: Int) { x }").unwrap_err();
    assert_eq!(diagnostic.message, MISSING_RETURN_TYPE);
    assert_eq!(diagnostic.span, Span::new(11, 12));
    assert_eq!(diagnostic.notes, vec![RETURN_TYPE_NOTE]);
    assert_eq!(
        MISSING_RETURN_TYPE,
        "expected `:`, `->` or `->>` and a return type after the parameters"
    );
    assert_eq!(
        RETURN_TYPE_NOTE,
        "a function declares its return type: `: T` for a pure function, `-> T` for an \
             impure one, `->> T` for a memoised one"
    );
    assert_eq!(
        error("fn() = 1"),
        (MISSING_RETURN_TYPE.to_owned(), Span::new(5, 6))
    );
    // The body's `{` must follow the return type directly.
    assert_eq!(
        error("fn(): Int\n{ 1 }"),
        (
            "expected `{` to start the function body".to_owned(),
            Span::new(9, 10)
        )
    );
    assert_eq!(
        error("fn(): Int 1"),
        (
            "expected `{` to start the function body".to_owned(),
            Span::new(10, 11)
        )
    );
    assert_eq!(
        error("fn(): { 1 }").0,
        "expected a field name in record type"
    );
    // Parameters need plain names.
    assert_eq!(
        error("fn(\"x\": Int): Int { 1 }"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(3, 6))
    );
    assert_eq!(
        error("fn(\"{y}\": Int): Int { 1 }"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(3, 8))
    );
    assert_eq!(
        error("fn(1: Int): Int { 1 }"),
        (
            "expected a parameter name, found `1`".to_owned(),
            Span::new(3, 4)
        )
    );
    let diagnostic = parse_text("fn(x: Int y: Int): Int { x }").unwrap_err();
    assert_eq!(diagnostic.message, "expected `,` or `)`, found `y`");
    assert_eq!(diagnostic.span, Span::new(10, 11));
    assert_eq!(labels(&diagnostic), vec![(Span::new(2, 3), "unclosed `(`")]);
    assert_eq!(
        error("fn(x: Int"),
        (
            "expected `,` or `)`, found end of input".to_owned(),
            Span::new(9, 9)
        )
    );
    assert_eq!(
        error("fn { 1 }"),
        (
            "expected `(` after `fn`, found `{`".to_owned(),
            Span::new(3, 4)
        )
    );
    assert_eq!(error("fn(x: 1): Int { 1 }").0, "expected a type");
}

/// The labels of a diagnostic as `(span, text)` pairs.
pub(super) fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
    diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect()
}

#[test]
fn a_def_is_a_field_of_a_function_expression() {
    let ast = parse_text("def add(a: Int, b: Int): Int { a + b }").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    let ItemKind::Field {
        path,
        ty: None,
        value,
        function: true,
    } = item.kind
    else {
        panic!("expected a def, got {item:?}");
    };
    assert_eq!(item.span, Span::new(0, 38));
    assert_eq!(ast.path(path)[0].span, Span::new(4, 7));
    assert_eq!(ast.get(value).span, Span::new(0, 38));
    // Each marker, as `fn` has them.
    for (marker, printed) in [(":", "():"), ("->>", "() ->>"), ("->", "() ->")] {
        let text = format!("def f(){marker} Int {{ 1 }}");
        assert_eq!(shape(&text), format!("def f{printed} Int {{1}}"));
    }
    // In a record block, among fields, in order.
    assert_eq!(
        shape("{ def f(): Int { 1 }; a = 2 }"),
        "{def f(): Int {1}; a = 2}"
    );
    assert!(
        ast.pretty()
            .to_string()
            .starts_with("(file\n  (def add @0..38\n    (fn @0..38\n")
    );
}

#[test]
fn the_four_forms_of_a_function_binding_print_as_written() {
    for (text, printed) in [
        ("fn f(): Int { 1 }", "fn f(): Int {1}"),
        ("def f(): Int { 1 }", "def f(): Int {1}"),
        ("let f = fn(): Int { 1 }", "let f = fn(): Int {1}"),
        ("f = fn(): Int { 1 }", "f = fn(): Int {1}"),
    ] {
        assert_eq!(shape(text), printed);
        assert_eq!(shape(printed), printed);
    }
    let heads = ["(fn f @", "(def f @", "(let f @", "(field f @"];
    for (text, head) in [
        "fn f(): Int { 1 }",
        "def f(): Int { 1 }",
        "let f = fn(): Int { 1 }",
        "f = fn(): Int { 1 }",
    ]
    .into_iter()
    .zip(heads)
    {
        let pretty = parse_text(text).unwrap().pretty().to_string();
        assert!(pretty.starts_with(&format!("(file\n  {head}")), "{pretty}");
    }
}

#[test]
fn a_def_needs_one_plain_name_and_the_grammar_of_fn() {
    assert_eq!(
        error("def a.\"b\"(): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(6, 9)
        )
    );
    assert_eq!(
        error("def \"a b\"(): Int { 1 }"),
        (
            "a `def` is named by an identifier, not a string".to_owned(),
            Span::new(4, 9)
        )
    );
    assert_eq!(
        error("def (): Int { 1 }"),
        (
            "expected the function's name after `def`, found `(`".to_owned(),
            Span::new(4, 5)
        )
    );
    // Without a return type or a body, what `fn` reports, one byte on.
    for rest in ["f() { 1 }", "f(): Int", "f()"] {
        let (message, span) = error(&format!("fn {rest}"));
        let shifted = Span::new(span.start + 1, span.end + 1);
        assert_eq!(error(&format!("def {rest}")), (message, shifted), "{rest}");
    }
    // At the start of an item it always begins a `def`, so a field
    // named `def` is one without a name.
    assert_eq!(
        error("def = 1"),
        (
            "expected the function's name after `def`, found `=`".to_owned(),
            Span::new(4, 5)
        )
    );
    // As a name elsewhere, it is reported as any keyword is.
    for (with_def, with_let) in [("let def = 1", "let let = 1"), ("x = def", "x = let")] {
        assert_eq!(
            error(with_def).0.replace("`def`", "`let`"),
            error(with_let).0,
            "{with_def}"
        );
    }
}

#[test]
fn a_def_may_be_named_by_a_path_of_identifiers() {
    let text = "def dev.peios.hello(n: Int): Int { n }";
    let ast = parse_text(text).unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    let ItemKind::Field {
        path,
        ty: None,
        value,
        function: true,
    } = item.kind
    else {
        panic!("expected a def, got {item:?}");
    };
    // The dotted field of the path, its value the function from its
    // `(` on, spanning the item.
    let segments = ast.path(path);
    assert_eq!(ast.show_path(segments).to_string(), "dev.peios.hello");
    let spans: Vec<Span> = segments.iter().map(|segment| segment.span).collect();
    assert_eq!(
        spans,
        [Span::new(4, 7), Span::new(8, 13), Span::new(14, 19)]
    );
    assert_eq!(item.span, Span::new(0, 38));
    assert_eq!(ast.get(value).span, Span::new(0, 38));
    // Printed as written, with each marker, and in a record block.
    assert_eq!(shape(text), "def dev.peios.hello(n: Int): Int {n}");
    assert_eq!(shape("def a.b() -> Int { 1 }"), "def a.b() -> Int {1}");
    assert_eq!(
        shape("{ def a.b(): Int { 1 }; c = 2 }"),
        "{def a.b(): Int {1}; c = 2}"
    );
    assert!(
        ast.pretty()
            .to_string()
            .starts_with("(file\n  (def dev.peios.hello @0..38\n    (fn @0..38\n"),
        "{}",
        ast.pretty()
    );
    // A quoted or index segment after the first, and anything else
    // where a name is expected after a `.`, is an error at it.
    assert_eq!(
        error("def a.b.\"c d\"(): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(8, 13)
        )
    );
    assert_eq!(
        error("def a[0](): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(5, 8)
        )
    );
    assert_eq!(
        error("def a.b[i + 1](): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(7, 14)
        )
    );
    assert_eq!(
        error("def a.(): Int { 1 }"),
        (
            "expected the function's name after `.`, found `(`".to_owned(),
            Span::new(6, 7)
        )
    );
    // Without a return type or a body, what `fn` reports.
    for rest in ["() { 1 }", "(): Int", "()"] {
        let (message, span) = error(&format!("fn f{rest}"));
        let shifted = Span::new(span.start + 5, span.end + 5);
        assert_eq!(
            error(&format!("def a.b.f{rest}")),
            (message, shifted),
            "{rest}"
        );
    }
    // Two levels taller for each segment after the first, as a dotted
    // field is.
    let deep = format!("def a{}(): Int {{ 1 }}", ".b".repeat(crate::MAX_DEPTH));
    assert!(
        error(&deep).0.contains("nested too deeply"),
        "{:?}",
        error(&deep)
    );
}

#[test]
fn named_functions_are_lets_of_function_expressions() {
    assert_eq!(
        shape("fn greet(name: String): String { \"hello {name}\" }"),
        "fn greet(name: String): String {\"hello <name>\"}"
    );
    assert_eq!(shape("fn f() -> {} {}\nf()"), "fn f() -> {} {}; f()");
    // The item and its value both span `fn` through the body; the path
    // is the name.
    let ast = parse_text("  fn add(a: Int, b: Int): Int { a + b }").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 39));
    let ItemKind::Let {
        path, ty, value, ..
    } = item.kind
    else {
        panic!("expected a let, got {item:?}");
    };
    assert!(ty.is_none());
    assert_eq!(ast.get(value).span, Span::new(2, 39));
    let segments = ast.path(path);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].span, Span::new(5, 8));
    assert_eq!(ast.names().get(segments[0].name().unwrap()), "add");
    assert!(matches!(ast.get(value).kind, ExprKind::Fn(_)));
    // In a block, and as the pretty printer shows it.
    assert_eq!(
        shape("{ fn f(): Int { 1 }; = f }"),
        "{fn f(): Int {1}; = f}"
    );
    assert_eq!(
        parse_text("fn f(): Int { 1 }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n\
             \x20 (fn f @0..17\n\
             \x20   (fn @0..17\n\
             \x20     (params)\n\
             \x20     (ret pure Int)\n\
             \x20     (block @12..17\n\
             \x20       (expr @14..15\n\
             \x20         (int 1 @14..15))))))"
    );
    // Without a name, `fn` starts an expression item, and an item is
    // never a field or a rebind of a function expression.
    assert_eq!(shape("fn(): Int { 1 }"), "fn(): Int {1}");
    assert_eq!(
        error("fn(): Int { 1 } = 2"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(16, 17)
        )
    );
}

#[test]
fn a_named_function_is_an_item_not_an_expression() {
    let diagnostic = parse_text("let f = fn greet(): Int { 1 }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected `(` after `fn`, found identifier `greet`"
    );
    assert_eq!(diagnostic.span, Span::new(11, 16));
    assert_eq!(
        diagnostic.notes,
        vec![
            "a named function `fn NAME(...)` is an item; write it on a line of its own, or \
                 drop the name"
        ]
    );
    assert_eq!(
        error("1 + fn f(): Int { 1 }").0,
        "expected `(` after `fn`, found identifier `f`"
    );
    assert_eq!(
        error("fn 1"),
        (
            "expected `(` after `fn`, found `1`".to_owned(),
            Span::new(3, 4)
        )
    );
}

#[test]
fn calls() {
    assert_eq!(shape("f()"), "f()");
    assert_eq!(shape("f(1)"), "f(1)");
    assert_eq!(shape("f(1, 2,)"), "f(1, 2)");
    assert_eq!(shape("f(y = 2)"), "f(y = 2)");
    assert_eq!(shape("f(1, y = 2)"), "f(1, y = 2)");
    assert_eq!(shape("f(y = 2, 1)"), "f(y = 2, 1)");
    assert_eq!(shape("f(x == 1, y = x + 1)"), "f((x == 1), y = (x + 1))");
    // Arguments may span lines inside the parentheses, and a block or a
    // function body among them keeps its newlines.
    assert_eq!(shape("f(\n  1,\n  y = 2,\n)"), "f(1, y = 2)");
    assert_eq!(
        shape("f(fn(n: Int): Int {\n  let m = n\n  m\n}, {\n  x = 1\n  y = 2\n})"),
        "f(fn(n: Int): Int {let m = n; m}, {x = 1; y = 2})"
    );
    // Trailing blocks, with and without an argument list.
    assert_eq!(shape("f {}"), "f(){}");
    assert_eq!(shape("f() {}"), "f(){}");
    assert_eq!(shape("f(1) {}"), "f(1){}");
    assert_eq!(
        shape("service(\"web\") { port = 80; image = \"nginx\" }"),
        "service(\"web\"){port = 80; image = \"nginx\"}"
    );
    assert_eq!(shape("run {\n  1\n}"), "run(){1}");
    // A call binds as tightly as field access.
    assert_eq!(shape("math.square(4)"), "(math . square)(4)");
    assert_eq!(shape("a.b(1).c"), "((a . b)(1) . c)");
    assert_eq!(shape("-f(1)"), "(-f(1))");
    assert_eq!(shape("not f(1)"), "(not f(1))");
    assert_eq!(shape("f(1)(2)"), "f(1)(2)");
    assert_eq!(shape("f(1) + g(2) * 3"), "(f(1) + (g(2) * 3))");
    assert_eq!(shape("f(1) : Int"), "(f(1) : Int)");
    assert_eq!(shape("(f)(1)"), "[f](1)");
    assert_eq!(shape("{ x }(1)"), "{x}(1)");
    // Any operand followed by `(` is a call; the checker rejects calling
    // what is not a function.
    assert_eq!(shape("2 (3)"), "2(3)");
    // A newline ends the item before a `{` or `(`.
    assert_eq!(shape("f\n{ 1 }"), "f; {1}");
    assert_eq!(shape("f(1)\n{ 1 }"), "f(1); {1}");
    assert_eq!(shape("f\n(1)"), "f; [1]");
    // The trailing block is the last argument.
    let ast = parse_text("f(1, y = 2) { 3 }").unwrap();
    let call = ast.get(only_value(&ast));
    assert_eq!(call.span, Span::new(0, 17));
    assert_eq!(call.close_span(), Some(Span::new(16, 17)));
    let ExprKind::Call { callee, args } = call.kind else {
        panic!("expected a call, got {call:?}");
    };
    assert_eq!(ast.get(callee).span, Span::new(0, 1));
    let args = ast.args(args);
    let y = ast.names().lookup("y").unwrap();
    assert_eq!(args[0].kind, ArgKind::Positional);
    assert_eq!(
        args[1].kind,
        ArgKind::Named {
            name: y,
            span: Span::new(5, 6),
            shorthand: false,
        }
    );
    assert_eq!(args[2].kind, ArgKind::Trailing);
    assert_eq!(args[1].span(&ast), Span::new(5, 10));
    assert_eq!(args[2].span(&ast), Span::new(12, 17));
    let ast = parse_text("f(1)").unwrap();
    let call = ast.get(only_value(&ast));
    assert_eq!(call.span, Span::new(0, 4));
    assert_eq!(call.close_span(), Some(Span::new(3, 4)));
    assert_eq!(ast.get(callee).close_span(), None);
}

#[test]
fn call_errors() {
    let diagnostic = parse_text("f(1 2)").unwrap_err();
    assert_eq!(diagnostic.message, "expected `,` or `)`, found `2`");
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(labels(&diagnostic), vec![(Span::new(1, 2), "unclosed `(`")]);
    assert_eq!(
        error("f(1"),
        (
            "expected `,` or `)`, found end of input".to_owned(),
            Span::new(3, 3)
        )
    );
    assert_eq!(
        error("f(,)"),
        ("expected expression, found `,`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(
        error("f(y = )"),
        ("expected expression, found `)`".to_owned(), Span::new(6, 7))
    );
    // Only an expression may be an argument.
    assert_eq!(error("f(let x = 1)").0, "expected expression, found `let`");
    // A call has one trailing block at most; a second is a call of the
    // first call's result, as `f(1)(2)` is.
    assert_eq!(shape("f {} {}"), "f(){}(){}");
}

#[test]
fn an_if_condition_takes_no_trailing_block() {
    assert_eq!(shape("if x { 1 }"), "(if x {1})");
    assert_eq!(shape("if a and b { 1 }"), "(if (a and b) {1})");
    assert_eq!(shape("if not a.b { 1 }"), "(if (not (a . b)) {1})");
    assert_eq!(shape("if f(1) { 2 }"), "(if f(1) {2})");
    assert_eq!(
        shape("if a { 1 } else if x { 2 }"),
        "(if a {1} else (if x {2}))"
    );
    assert_eq!(
        shape("if a { 1 } else if b and c { 2 } else { 3 }"),
        "(if a {1} else (if (b and c) {2} else {3}))"
    );
    // Lifted inside parentheses, argument lists, blocks and
    // interpolations, and restored after them.
    assert_eq!(shape("if (f { }) { 1 }"), "(if [f(){}] {1})");
    assert_eq!(shape("if g(f { }) { 1 }"), "(if g(f(){}) {1})");
    assert_eq!(shape("if { f { } } { 1 }"), "(if {f(){}} {1})");
    assert_eq!(shape("if \"{f { }}\" { 1 }"), "(if \"<f(){}>\" {1})");
    assert_eq!(shape("if (a) and b { 1 }"), "(if ([a] and b) {1})");
    // After the condition, blocks are trailing blocks again.
    assert_eq!(shape("if a { f { } }"), "(if a {f(){}})");
    assert_eq!(
        shape("if a { 1 } else { 2 }\nf { }"),
        "(if a {1} else {2}); f(){}"
    );
    // A function expression in a condition has its body, since the
    // body is not a trailing block.
    assert_eq!(
        shape("if fn(): Bool { true }() { 1 }"),
        "(if fn(): Bool {true}() {1})"
    );
}

#[test]
fn function_types() {
    assert_eq!(
        shape("let f: fn(Int, String): Bool = g"),
        "let f: fn(Int, String): Bool = g"
    );
    assert_eq!(
        shape("let f: fn(name: String, times: Int) -> String = g"),
        "let f: fn(name: String, times: Int) -> String = g"
    );
    assert_eq!(
        shape("let f: fn(): {x: Int} = g"),
        "let f: fn(): {x: Int} = g"
    );
    assert_eq!(
        shape("let f: fn(fn(Int): Int): Int = g"),
        "let f: fn(fn(Int): Int): Int = g"
    );
    assert_eq!(
        shape("let r: {f: fn(Int): Int, g: fn() -> {}} = h"),
        "let r: {f: fn(Int): Int, g: fn() -> {}} = h"
    );
    assert_eq!(shape("g : fn(Int): Int"), "(g : fn(Int): Int)");
    assert_eq!(
        shape("let f: fn(x: Int, Int,): fn() -> Int = g"),
        "let f: fn(x: Int, Int): fn() -> Int = g"
    );
    // Newlines inside the parentheses and after the marker.
    assert_eq!(
        shape("let f: fn(\n  Int,\n  x: Int\n) ->\n  Int = g"),
        "let f: fn(Int, x: Int) -> Int = g"
    );
    // A name followed by `:` names the parameter; a name alone is a type.
    let ast = parse_text("g : fn(x: Int, Int): Int").unwrap();
    let ExprKind::Ascribe { ty, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected an ascription");
    };
    let ty = ast.type_expr(ty);
    assert_eq!(ty.span, Span::new(4, 24));
    let TypeExprKind::Fn {
        params,
        ret,
        effect,
        ..
    } = ty.kind
    else {
        panic!("expected a function type, got {ty:?}");
    };
    assert_eq!(effect, Effect::Pure);
    assert_eq!(ast.type_expr(ret).span, Span::new(21, 24));
    let params = ast.param_tys(params);
    assert_eq!(params[0].name, ast.names().lookup("x"));
    assert_eq!(params[1].name, None);
    assert_eq!(ast.type_expr(params[1].ty).span, Span::new(15, 18));
    // Errors.
    let diagnostic = parse_text("let f: fn(Int) Int = g").unwrap_err();
    assert_eq!(diagnostic.message, MISSING_RETURN_TYPE);
    assert_eq!(diagnostic.span, Span::new(15, 18));
    assert_eq!(diagnostic.notes, vec![RETURN_TYPE_NOTE]);
    assert_eq!(
        error("let f: fn Int = g"),
        (
            "expected `(` after `fn`, found `Int`".to_owned(),
            Span::new(10, 13)
        )
    );
    assert_eq!(
        error("let f: fn(Int Int): Int = g").0,
        "expected `,` or `)`, found `Int`"
    );
    assert_eq!(error("let f: fn(x:): Int = g").0, "expected a type");
}

#[test]
fn pretty_output_for_functions_and_calls() {
    let text = "fn f(x: Int, y: fn(Int) -> String) -> {} {}\nf(1, y = g) { 2 }";
    let ast = parse_text(text).unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (fn f @0..43\n\
             \x20   (fn @0..43\n\
             \x20     (params (x Int) (y fn(Int) -> String))\n\
             \x20     (ret impure {})\n\
             \x20     (block @41..43)))\n\
             \x20 (expr @44..61\n\
             \x20   (call @44..61\n\
             \x20     (name f @44..45)\n\
             \x20     (int 1 @46..47)\n\
             \x20     (named y\n\
             \x20       (name g @53..54))\n\
             \x20     (trailing\n\
             \x20       (block @56..61\n\
             \x20         (expr @58..59\n\
             \x20           (int 2 @58..59)))))))"
    );
}

#[test]
fn nested_calls_and_functions_count_towards_depth() {
    // N nested calls around a literal are N + 1 levels: each call is one
    // above its argument.
    let calls = |depth: usize| format!("{}1{}", "f(".repeat(depth), ")".repeat(depth));
    let deepest = calls(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(calls(MAX_DEPTH));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(
        too_deep(calls(10_000)).message,
        "expression nested too deeply"
    );
    // Far beyond the limit, stopped by the recursion count.
    assert_eq!(
        too_deep(calls(200_000)).message,
        "expression nested too deeply"
    );
    // Each nested function expression is two levels, itself and its
    // body; with the innermost body's literal, N of them are 2N + 1.
    let functions =
        |depth: usize| format!("{}1{}", "fn(): Int { ".repeat(depth), " }".repeat(depth));
    let deepest = functions((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(functions((MAX_DEPTH - 1) / 2 + 1)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(functions(10_000)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(functions(200_000)).message,
        "expression nested too deeply"
    );
    // Function types nest by recursion too.
    let types = |depth: usize| format!("let f: {}Int = g", "fn(): ".repeat(depth));
    let deepest = types(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(types(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(types(200_000)).message,
        "expression nested too deeply"
    );
    // Trailing blocks nest as blocks do.
    let trailing = |depth: usize| format!("{}1{}", "f { ".repeat(depth), " }".repeat(depth));
    let deepest = trailing((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(trailing(200_000)).message,
        "expression nested too deeply"
    );
}

#[test]
fn lazy_takes_the_longest_expression() {
    assert_eq!(shape("lazy a + b"), "(lazy (a + b))");
    assert_eq!(shape("lazy a and b"), "(lazy (a and b))");
    assert_eq!(
        shape("lazy a + b * c : Int"),
        "(lazy (a + (b * (c : Int))))"
    );
    assert_eq!(shape("lazy { x = 1 }"), "(lazy {x = 1})");
    assert_eq!(shape("lazy { run(compiler) }"), "(lazy {run(compiler)})");
    assert_eq!(shape("lazy f(x) { y = 1 }"), "(lazy f(x){y = 1})");
    assert_eq!(
        shape("lazy if a { 1 } else { 2 }"),
        "(lazy (if a {1} else {2}))"
    );
    assert_eq!(shape("lazy lazy 1"), "(lazy (lazy 1))");
    // As an operand, it still takes everything after it.
    assert_eq!(shape("1 + lazy 2 * 3"), "(1 + (lazy (2 * 3)))");
    assert_eq!(shape("let x = lazy 5 + 5"), "let x = (lazy (5 + 5))");
    assert_eq!(
        shape("build = lazy { run(compiler) }"),
        "build = (lazy {run(compiler)})"
    );
    // A block after a condition is still the `if`'s.
    assert_eq!(shape("if lazy a { 1 }"), "(if (lazy a) {1})");
    // The body is a parameterless, impure function without a return
    // type, whose marker is the keyword; the expression spans the
    // keyword through the operand.
    let ast = parse_text("  lazy a + b").unwrap();
    let expr = ast.get(only_value(&ast));
    assert_eq!(expr.span, Span::new(2, 12));
    let ExprKind::Lazy(function) = expr.kind else {
        panic!("expected a lazy, got {expr:?}");
    };
    let decl = ast.fn_decl(function);
    assert!(decl.params.is_empty());
    assert_eq!(decl.ret, None);
    assert_eq!(decl.effect, Effect::Impure);
    assert_eq!(decl.marker, Span::new(2, 6));
    assert_eq!(ast.get(decl.body).span, Span::new(7, 12));
    assert!(matches!(ast.get(decl.body).kind, ExprKind::Binary { .. }));
    assert_eq!(expr.operator_span(), None);
    // Lazies and functions share one table.
    let ast = parse_text("let f = fn(): Future<Int> { lazy 1 }\nlazy f").unwrap();
    assert_eq!(ast.fn_count(), 3);
    assert_eq!(
        error("lazy"),
        (
            "expected expression, found end of input".to_owned(),
            Span::new(4, 4)
        )
    );
}

#[test]
fn await_binds_as_tightly_as_negation() {
    assert_eq!(shape("await x + 1"), "((await x) + 1)");
    assert_eq!(shape("await f()"), "(await f())");
    assert_eq!(shape("await x.y"), "(await (x . y))");
    assert_eq!(shape("-await x"), "(-(await x))");
    assert_eq!(shape("not await x"), "(not (await x))");
    assert_eq!(shape("await -x"), "(await (-x))");
    assert_eq!(shape("await x : Int"), "((await x) : Int)");
    assert_eq!(shape("await await x"), "(await (await x))");
    assert_eq!(shape("await x * await y"), "((await x) * (await y))");
    // A bare `await` item.
    assert_eq!(shape("await conf.build"), "(await (conf . build))");
    let ast = parse_text("await conf.build").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let expr = ast.get(root);
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnaryOp::Await,
            ..
        }
    ));
    assert_eq!(expr.span, Span::new(0, 16));
    assert_eq!(expr.operator_span(), Some(Span::new(0, 5)));
    assert_eq!(
        error("await"),
        (
            "expected expression, found end of input".to_owned(),
            Span::new(5, 5)
        )
    );
}

#[test]
fn applied_and_union_types() {
    assert_eq!(shape("let x: Future<Int> = y"), "let x: Future<Int> = y");
    assert_eq!(
        shape("let x: Future<Future<Int>> = y"),
        "let x: Future<Future<Int>> = y"
    );
    assert_eq!(
        shape("let x: Future<{a: Int}> = y"),
        "let x: Future<{a: Int}> = y"
    );
    assert_eq!(shape("let x: Present<Int> = y"), "let x: Present<Int> = y");
    assert_eq!(
        shape("let x: Int | Future<Int> = y"),
        "let x: (Int | Future<Int>) = y"
    );
    // In parameters, return types, record fields and function types.
    assert_eq!(
        shape("fn(x: Future<Int>, y: Int | Future<Int>): Int | Future<Int> { x }"),
        "fn(x: Future<Int>, y: (Int | Future<Int>)): (Int | Future<Int>) {x}"
    );
    assert_eq!(
        shape("fn plan(): {build: Future<{}>} { build = lazy { run(compiler) } }"),
        "fn plan(): {build: Future<{}>} {build = (lazy {run(compiler)})}"
    );
    assert_eq!(
        shape("1 : {a: Int | Future<Int>, b: Present<{c: Int}>}"),
        "(1 : {a: (Int | Future<Int>), b: Present<{c: Int}>})"
    );
    assert_eq!(
        shape("let f: fn(Int | Future<Int>) -> Future<fn(): Int> = g"),
        "let f: fn((Int | Future<Int>)) -> Future<fn(): Int> = g"
    );
    // A function type's return type takes the union.
    assert_eq!(
        shape("let f: fn() -> Int | Future<Int> = g"),
        "let f: fn() -> (Int | Future<Int>) = g"
    );
    // Newlines after `<` and `|`, but not before `>`.
    assert_eq!(
        shape("let x: Future<\n  Int> = y"),
        "let x: Future<Int> = y"
    );
    assert_eq!(
        shape("let x: Int |\n  Future<Int> = y"),
        "let x: (Int | Future<Int>) = y"
    );
    // Unions of three are left-associative; the checker rejects them.
    assert_eq!(
        shape("let x: Int | Int | Future<Int> = y"),
        "let x: ((Int | Int) | Future<Int>) = y"
    );
    // Any name may be applied; `Future` alone is a name.
    assert_eq!(shape("let x: Foo<Int> = y"), "let x: Foo<Int> = y");
    assert_eq!(shape("let x: Future = y"), "let x: Future = y");
    // `Future` and `Present` are ordinary names in expressions.
    assert_eq!(shape("Future < Present"), "(Future < Present)");
    // Spans: the name, the whole, and the bar.
    let ast = parse_text("1 : Int | Future<Int>").unwrap();
    let ExprKind::Ascribe { ty, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected an ascription");
    };
    let union = ast.type_expr(ty);
    assert_eq!(union.span, Span::new(4, 21));
    let TypeExprKind::Union { lhs, rhs, bar } = union.kind else {
        panic!("expected a union, got {union:?}");
    };
    assert_eq!(bar, Span::new(8, 9));
    assert_eq!(ast.type_expr(lhs).span, Span::new(4, 7));
    let applied = ast.type_expr(rhs);
    assert_eq!(applied.span, Span::new(10, 21));
    let TypeExprKind::Applied { name, span, args } = applied.kind else {
        panic!("expected an applied type, got {applied:?}");
    };
    assert_eq!(ast.names().get(name), "Future");
    assert_eq!(span, Span::new(10, 16));
    let &[arg] = ast.type_args(args) else {
        panic!("expected one type argument");
    };
    assert_eq!(ast.type_expr(arg).span, Span::new(17, 20));
}

#[test]
fn parenthesised_types() {
    // A function type in parentheses is a member of the union, not a
    // function returning it.
    let text = "let f: (fn(): Int) | Future<fn(): Int> = g";
    assert_eq!(shape(text), "let f: (fn(): Int | Future<fn(): Int>) = g");
    let ast = parse_text(text).unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated let");
    };
    let TypeExprKind::Union { lhs, rhs, .. } = ast.type_expr(ty).kind else {
        panic!("expected a union");
    };
    assert!(matches!(
        ast.type_expr(lhs).kind,
        TypeExprKind::Fn {
            effect: Effect::Pure,
            ..
        }
    ));
    assert!(matches!(
        ast.type_expr(rhs).kind,
        TypeExprKind::Applied { .. }
    ));
    // The parentheses leave no node: the type inside stands for itself,
    // with its own span.
    assert_eq!(ast.type_expr(lhs).span, Span::new(8, 17));
    // The pretty printer shows the parentheses a function type in a
    // union needs.
    assert!(
        ast.pretty()
            .to_string()
            .contains("(let f : (fn(): Int) | Future<fn(): Int> @")
    );
    // A union in parentheses as a function type's return type is what
    // the return type takes without them.
    assert_eq!(
        shape("let f: fn() -> (Int | Future<Int>) = g"),
        "let f: fn() -> (Int | Future<Int>) = g"
    );
    assert_eq!(
        shape("let f: fn() -> (Int | Future<Int>) = g"),
        shape("let f: fn() -> Int | Future<Int> = g")
    );
    // Anywhere a type is, nested, and with newlines after `(`.
    assert_eq!(shape("let x: ((Int)) = y"), "let x: Int = y");
    assert_eq!(
        shape("1 : {a: (fn() -> Int) | Future<fn() -> Int>}"),
        "(1 : {a: (fn() -> Int | Future<fn() -> Int>)})"
    );
    assert_eq!(shape("let x: Future<(Int)> = y"), "let x: Future<Int> = y");
    assert_eq!(
        shape("let x: (\n  Int |\n  Future<Int>\n) = y"),
        "let x: (Int | Future<Int>) = y"
    );
}

#[test]
fn parenthesised_type_errors() {
    let diagnostic = parse_text("let x: (Int = 1").unwrap_err();
    assert_eq!(diagnostic.message, "expected `)` to close the type");
    assert_eq!(diagnostic.span, Span::new(12, 13));
    assert_eq!(labels(&diagnostic), vec![(Span::new(7, 8), "unclosed `(`")]);
    assert_eq!(
        error("let x: () = 1"),
        ("expected a type".to_owned(), Span::new(8, 9))
    );
    assert_eq!(
        error("let x: (Int | ) = 1"),
        ("expected a type".to_owned(), Span::new(14, 15))
    );
}

#[test]
fn applied_type_errors() {
    let diagnostic = parse_text("let x: Future<Int = 1").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected `>` to close the type arguments"
    );
    assert_eq!(diagnostic.span, Span::new(18, 19));
    assert_eq!(
        labels(&diagnostic),
        vec![(Span::new(13, 14), "unclosed `<`")]
    );
    // `>` must follow the argument directly.
    assert_eq!(
        error("let x: Future<Int\n> = 1"),
        (
            "expected `>` to close the type arguments".to_owned(),
            Span::new(17, 18)
        )
    );
    assert_eq!(
        error("let x: Future<> = 1"),
        ("expected a type".to_owned(), Span::new(14, 15))
    );
    assert_eq!(
        error("let x: Int | = 1"),
        ("expected a type".to_owned(), Span::new(13, 14))
    );
    // `|` is not an expression operator.
    assert_eq!(
        error("1 + | 2"),
        ("expected expression, found `|`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("| 2"),
        ("expected expression, found `|`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("a | b"),
        (
            "expected end of item, found `|`".to_owned(),
            Span::new(2, 3)
        )
    );
}

#[test]
fn pretty_output_for_futures() {
    let ast = parse_text(
        "let x: Int | Future<Int> = lazy 1 + 2\nawait x\nlet p: Present<Future<Int>> = x",
    )
    .unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Int | Future<Int> @0..37\n\
             \x20   (lazy @27..37\n\
             \x20     (+ @32..37\n\
             \x20       (int 1 @32..33)\n\
             \x20       (int 2 @36..37))))\n\
             \x20 (expr @38..45\n\
             \x20   (await @38..45\n\
             \x20     (name x @44..45)))\n\
             \x20 (let p : Present<Future<Int>> @46..77\n\
             \x20   (name x @76..77)))"
    );
}

#[test]
fn nested_futures_and_future_types_count_towards_depth() {
    // N nested lazies around a literal are N + 1 levels, and so are N
    // nested awaits.
    for keyword in ["lazy ", "await "] {
        let nested = |depth: usize| format!("{}1", keyword.repeat(depth));
        let deepest = nested(MAX_DEPTH - 1);
        on_big_stack(move || parse_text(&deepest).unwrap());
        assert_eq!(
            too_deep(nested(MAX_DEPTH)).message,
            "expression nested too deeply",
            "{keyword}"
        );
        assert_eq!(
            too_deep(nested(10_000)).message,
            "expression nested too deeply"
        );
        // Far beyond the limit, stopped by the recursion count.
        assert_eq!(
            too_deep(nested(200_000)).message,
            "expression nested too deeply"
        );
    }
    // N nested applied types around a name are N + 1 levels, in an
    // annotation, whose height is its item's.
    let applied = |depth: usize| {
        format!(
            "let x: {}Int{} = y",
            "Future<".repeat(depth),
            ">".repeat(depth)
        )
    };
    let deepest = applied(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(applied(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(applied(200_000)).message,
        "expression nested too deeply"
    );
    // So are N nested parentheses around a type, though they leave no
    // node of their own.
    let parenthesised =
        |depth: usize| format!("let x: {}Int{} = y", "(".repeat(depth), ")".repeat(depth));
    let deepest = parenthesised(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(parenthesised(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(parenthesised(200_000)).message,
        "expression nested too deeply"
    );
    // A union of N types is N levels, built without recursion.
    let union = |members: usize| format!("let x: Int{} = y", " | Int".repeat(members - 1));
    let deepest = union(MAX_DEPTH);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(union(MAX_DEPTH + 1)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(union(200_000)).message,
        "expression nested too deeply"
    );
}
