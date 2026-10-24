//! Tests of the checker: function expressions, captures, calls and their
//! arguments, and purity.

use super::*;

/// The span of the `n`th (from 0) occurrence of `needle` in `text`.
pub(super) fn nth(text: &str, needle: &str, n: usize) -> Span {
    let start = text
        .match_indices(needle)
        .nth(n)
        .unwrap_or_else(|| panic!("{needle:?} does not occur {} times in {text:?}", n + 1))
        .0;
    let start = u32::try_from(start).unwrap();
    Span::new(start, start + u32::try_from(needle.len()).unwrap())
}

/// The span of the first occurrence of `needle` in `text`.
pub(super) fn at(text: &str, needle: &str) -> Span {
    nth(text, needle, 0)
}

/// The span of the last occurrence of `needle` in `text`.
pub(super) fn last(text: &str, needle: &str) -> Span {
    let start = u32::try_from(text.rfind(needle).unwrap()).unwrap();
    Span::new(start, start + u32::try_from(needle.len()).unwrap())
}

/// Checks `text`, which must succeed, returning its tree and resolution.
pub(super) fn resolved(text: &str) -> (Ast, Resolution) {
    let (ast, _, output) = check_text(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    (ast, output.checked.resolution)
}

/// Where each use of the variable `name` reads from, in allocation
/// order.
pub(super) fn uses(ast: &Ast, resolution: &Resolution, name: &str) -> Vec<VarRef> {
    let name = ast.names().lookup(name).unwrap();
    ast.expr_ids()
        .filter(|&id| ast.get(id).kind == ExprKind::Name(name))
        .map(|id| resolution.expr_var(id))
        .collect()
}

/// Function number `n`'s captures, as `(VarRef, shown type)` pairs.
pub(super) fn captures(text: &str, n: u32) -> Vec<(VarRef, String)> {
    let (ast, types, output) = check_text(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    output
        .checked
        .resolution
        .function(FnId::new(n))
        .captures
        .iter()
        .map(|&(var, ty)| (var, types.display(ty, ast.names()).to_string()))
        .collect()
}

pub(super) const ADD: &str = "fn add(a: Int, b: Int): Int { a + b }\n";

#[test]
fn a_function_expression_has_its_declared_type() {
    // A pure function's bare parameters that its body never measures
    // are the either shape.
    assert_eq!(
        shown_result("fn(x: Int, y: String): Bool { true }"),
        "fn(x: Int | Future<Int>, y: String | Future<String>): Bool"
    );
    assert_eq!(
        shown_result("fn(x: Present<Int>, y: Present<String>): Bool { true }"),
        "fn(x: Int, y: String): Bool"
    );
    assert_eq!(shown_result("fn() -> {} {}"), "fn() -> {}");
    assert_eq!(
        shown_result("fn(f: fn(Int): Int): {r: fn() -> Int} { r = fn() -> Int { f(1) } }"),
        "fn(f: fn(Int): Int): {r: fn() -> Int | Future<Int>}"
    );
    assert_eq!(
        shown_result("let f = fn(x: Int): Int { x }\nf"),
        "fn(x: Int): Int"
    );
    assert_eq!(shown_result("fn f(x: Int): Float { 1 }\nf(1)"), "Float");
    assert_eq!(file_type("fn f(): Int { 1 }"), "{}");
    // An unknown parameter or return type is reported once; the function
    // is erroneous, and so is every call of it, without cascading.
    assert_eq!(
        messages("fn f(x: Nope): Int { 1 }\nf(1) + 1.5"),
        vec!["unknown type `Nope`"]
    );
    assert_eq!(
        messages("fn f(x: Int): Nope { x }\nlet y: Int = f(1)"),
        vec!["unknown type `Nope`"]
    );
    // The parameters are still declared, with the type they have.
    assert_eq!(
        messages("fn f(x: Nope, y: Int): Int { y + 1.5 }"),
        vec!["unknown type `Nope`", "cannot add Int and Float"]
    );
}

#[test]
fn a_body_is_checked_against_the_return_type() {
    // A literal adapts to the return type.
    assert_eq!(shown_result("let f = fn(): Float { 1 }\nf()"), "Float");
    let ast = parse_text("fn(): Float { 1 + 2 }");
    let checked = check(&ast, &mut TypeTable::new()).unwrap().0;
    let ExprKind::Fn(function) = ast.get(result_expr(&ast)).kind else {
        panic!("expected a function");
    };
    let body = ast.fn_decl(function).body;
    assert_eq!(checked.types.get(body), TyId::FLOAT);
    // A mismatch, labelled with the return type.
    let text = "fn(): Int { \"a\" }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, at(text, "{ \"a\" }"));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(at(text, "Int"), "the return type is declared here")]
    );
    // A subtype is accepted.
    assert_eq!(
        shown_result("fn(r: {x: Int, y: Int}): {x: Int} { r }"),
        "fn(r: {x: Int, y: Int}): {x: Int}"
    );
    // A record block is checked against a record return type, field by
    // field.
    let text = "fn(): {x: Int, y: Int} { x = 1 }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "missing field `y` in record");
    assert_eq!(diagnostic.span, Span::new(31, 32));
    assert_eq!(
        labels(diagnostic),
        vec![(
            at(text, "{x: Int, y: Int}"),
            "the return type is declared here"
        )]
    );
    let text = "fn(): {x: Float} { x = 1; z = 2 }";
    let warning = &warnings(text)[0];
    assert_eq!(warning.message, "field `z` is not in `{x: Float}`");
    assert_eq!(warning.span, at(text, "z"));
    assert_eq!(
        labels(warning),
        vec![(at(text, "{x: Float}"), "the return type is declared here")]
    );
    // The literal field adapted to the expected field's type.
    let ast = parse_text(text);
    let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
    let int = ast
        .expr_ids()
        .find(|&id| ast.get(id).kind == ExprKind::Int(1))
        .unwrap();
    assert_eq!(checked.types.get(int), TyId::FLOAT);
    // Through a result, and without one.
    assert_eq!(
        messages("fn(): Int { let x = 1 }"),
        vec!["mismatched types"]
    );
    assert_eq!(shown_result("fn() -> {} { let x = 1 }"), "fn() -> {}");
}

#[test]
fn parameters_are_bindings_of_the_body() {
    // Shadowable, in the body and in blocks within it; a shadowed
    // parameter is never measured, so it is the either shape.
    assert_eq!(
        shown_result("fn(x: Int): String { let x = \"s\"\nx }"),
        "fn(x: Int | Future<Int>): String"
    );
    assert_eq!(
        shown_result("fn(x: Int): Float { { let x = 1.5; x } }"),
        "fn(x: Int | Future<Int>): Float"
    );
    // Rebindable with `:=`, keeping their type.
    assert_eq!(
        shown_result("fn(x: Int): Int { x := x + 1\nx }"),
        "fn(x: Int): Int"
    );
    let text = "fn(x: Int): Int { x := 1.5\nx }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, at(text, "1.5"));
    assert_eq!(
        labels(diagnostic),
        vec![(at(text, "x: Int"), "the variable was declared here")]
    );
    // But not from a block inside the body.
    assert_eq!(
        messages("fn(x: Int): Int { { x := 2 }\nx }"),
        vec!["cannot rebind `x` here: it is declared in an enclosing block"]
    );
    // Declared twice.
    let text = "fn(x: Int, x: Float): Float { x }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "parameter `x` is declared twice");
    assert_eq!(found[0].span, at(text, "x: Float"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "x: Int"), "first declared here")]
    );
    // The parameters take the frame's first slots.
    let (ast, resolution) = resolved("fn(x: Int, y: Int): Int { let z = x\nz + y }");
    let function = resolution.function(FnId::new(0));
    assert_eq!(function.param_count, 2);
    assert_eq!(function.frame_size, 3);
    let slot_names: Vec<&str> = function
        .slot_names
        .iter()
        .map(|&name| ast.names().get(name))
        .collect();
    assert_eq!(slot_names, vec!["x", "y", "z"]);
    assert_eq!(uses(&ast, &resolution, "y"), vec![VarRef::Local(Slot(1))]);
    assert_eq!(uses(&ast, &resolution, "z"), vec![VarRef::Local(Slot(2))]);
}

#[test]
fn a_function_has_a_frame_of_its_own() {
    let text = "let a = 1\nlet f = fn(x: Int): Int { let y = x\ny }\nlet b = 2\nb";
    let (ast, resolution) = resolved(text);
    // The top level's frame holds only its own variables.
    assert_eq!(resolution.frame_size(), 3);
    let slot_names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| ast.names().get(name))
        .collect();
    assert_eq!(slot_names, vec!["a", "f", "b"]);
    assert_eq!(resolution.function(FnId::new(0)).frame_size, 2);
    assert_eq!(uses(&ast, &resolution, "b"), vec![VarRef::Local(Slot(2))]);
    // A function in a block of a function has its own frame too, and
    // its enclosing frame's allocation continues after it.
    let text = "fn(p: Int): Int {\n  let g = fn(q: Int): Int { let r = q\nr }\n  let s = p\n  s\n}";
    let (ast, resolution) = resolved(text);
    assert_eq!(resolution.function(FnId::new(0)).frame_size, 2);
    assert_eq!(resolution.function(FnId::new(1)).frame_size, 3);
    assert_eq!(uses(&ast, &resolution, "s"), vec![VarRef::Local(Slot(2))]);
    assert_eq!(resolution.function(FnId::new(1)).callee, None);
}

#[test]
fn a_function_sees_itself_by_the_name_it_initialises() {
    // A `let`, a field and a named function item.
    for text in [
        "let fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }",
        "{ fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } } }",
        "fn fib(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }",
        "let fib: fn(Int): Int = fn(n: Int): Int { fib(n - 1) + fib(n - 2) }",
    ] {
        let (ast, resolution) = resolved(text);
        assert_eq!(
            uses(&ast, &resolution, "fib"),
            vec![VarRef::Callee, VarRef::Callee],
            "{text}"
        );
        let fib = ast.names().lookup("fib");
        assert_eq!(resolution.function(FnId::new(0)).callee, fib, "{text}");
        assert!(resolution.function(FnId::new(0)).captures.is_empty());
    }
    // Not through parentheses, nor for a dotted path.
    assert_eq!(
        messages("let f = (fn(): Int { f() })"),
        vec!["undefined variable `f`"]
    );
    assert_eq!(
        messages("let a.f = fn(): Int { f() }"),
        vec!["undefined variable `f`"]
    );
    // A parameter of the same name shadows it.
    let text = "fn f(f: Int): Int { f }";
    let (ast, resolution) = resolved(text);
    assert_eq!(uses(&ast, &resolution, "f"), vec![VarRef::Local(Slot(0))]);
    assert_eq!(
        shown_result("fn f(f: Int): Int { f }\nf"),
        "fn(f: Int): Int"
    );
    // A `let` in the body shadows it too.
    assert_eq!(shown_result("fn f(): Int { let f = 2\nf }\nf()"), "Int");
    // `:=` cannot rebind it.
    let text = "fn f(): Int { f := f\n1 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "cannot rebind `f`: it is the function being defined"
    );
    assert_eq!(found[0].span, nth(text, "f", 2));
    assert_eq!(found[0].notes, vec!["shadow it with `let` instead"]);
    assert!(found[0].labels.is_empty());
    // A top-level function is visible above its declaration.
    assert_eq!(
        messages("f()\nfn f(): Int { 1 }"),
        vec!["unused value of type Int"]
    );
    // And top-level functions may be mutually recursive, each reading
    // the other through the top level's frame.
    assert_eq!(
        shown_result("fn a(): Int { b() }\nfn b(): Int { a() }\na"),
        "fn(): Int"
    );
    let (ast, resolution) = resolved("fn a(): Int { b() }\nfn b(): Int { a() }");
    assert_eq!(uses(&ast, &resolution, "b"), vec![VarRef::Fn(Slot(1))]);
}

#[test]
fn a_callee_is_captured_like_a_variable() {
    // A function nested in the body does not see itself by the outer
    // name, but captures the outer function.
    let text = "let f = fn(): fn(): Int { fn(): Int { f()() } }";
    let (ast, resolution) = resolved(text);
    assert_eq!(uses(&ast, &resolution, "f"), vec![VarRef::Capture(0)]);
    assert_eq!(resolution.function(FnId::new(0)).callee, None);
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Callee, "fn(): fn(): Int".to_owned())]
    );
    assert!(captures(text, 1).is_empty());
}

#[test]
fn captures_are_chained_through_enclosing_functions() {
    // A top-level variable, captured once however often it is used.
    let text = "let x = 1\nlet f = fn(): Int { x + x }";
    let (ast, resolution) = resolved(text);
    assert_eq!(
        uses(&ast, &resolution, "x"),
        vec![VarRef::Capture(0), VarRef::Capture(0)]
    );
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Local(Slot(0)), "Int".to_owned())]
    );
    // A field of a record block.
    let text = "{ x = 1.5; f = fn(): Float { x } }";
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Local(Slot(0)), "Float".to_owned())]
    );
    // A parameter, through one level and through two: each function
    // captures from the one enclosing it. The innermost function is
    // number 0, since it is finished first. The innermost, pure,
    // function measures the parameters, which makes them present.
    let text = "fn outer(a: Int, b: String): fn(): fn(): String {\n\
                    \x20 fn(): fn(): String { fn(): String { \"{b}{a}\" } }\n}";
    let (ast, resolution) = resolved(text);
    assert_eq!(uses(&ast, &resolution, "a"), vec![VarRef::Capture(1)]);
    assert_eq!(uses(&ast, &resolution, "b"), vec![VarRef::Capture(0)]);
    assert_eq!(
        captures(text, 0),
        vec![
            (VarRef::Capture(0), "String".to_owned()),
            (VarRef::Capture(1), "Int".to_owned())
        ]
    );
    assert_eq!(
        captures(text, 1),
        vec![
            (VarRef::Local(Slot(1)), "String".to_owned()),
            (VarRef::Local(Slot(0)), "Int".to_owned())
        ]
    );
    assert!(captures(text, 2).is_empty());
    // The callee, through one level and through two. `n` is only passed
    // back to `f`, unmeasured, so it is the either shape, and so are
    // the captures' types, settled with it.
    let text = "fn f(n: Int): fn(): Int { fn(): Int { f(n)() } }";
    assert_eq!(
        captures(text, 0),
        vec![
            (
                VarRef::Callee,
                "fn(n: Int | Future<Int>): fn(): Int".to_owned()
            ),
            (VarRef::Local(Slot(0)), "Int | Future<Int>".to_owned())
        ]
    );
    let text = "fn f(): Int {\n\
                    \x20 let g = fn(): Int {\n\
                    \x20   let h = fn(): Int { f() }\n\
                    \x20   h()\n\
                    \x20 }\n\
                    \x20 g()\n}";
    let (ast, resolution) = resolved(text);
    assert_eq!(uses(&ast, &resolution, "f"), vec![VarRef::Capture(0)]);
    assert_eq!(
        captures(text, 0),
        vec![(VarRef::Capture(0), "fn(): Int".to_owned())]
    );
    assert_eq!(
        captures(text, 1),
        vec![(VarRef::Callee, "fn(): Int".to_owned())]
    );
    // A local of the function itself is not a capture, and neither is
    // a variable shadowing a captured one.
    let text = "let x = 1\nlet f = fn(): Int { let y = x\nlet x = y\nx }";
    let (ast, resolution) = resolved(text);
    assert_eq!(
        uses(&ast, &resolution, "x"),
        vec![VarRef::Capture(0), VarRef::Local(Slot(1))]
    );
    assert_eq!(uses(&ast, &resolution, "y"), vec![VarRef::Local(Slot(0))]);
    // A dotted `let` updating a captured record reads it from the
    // captures.
    let text = "let r = { a = 1 }\nlet f = fn(): {a: Int, b: Int} { let r.b = 2\nr }";
    let (ast, resolution) = resolved(text);
    let update = ast
        .expr_ids()
        .filter_map(|id| match ast.get(id).kind {
            ExprKind::Block { items, .. } => Some(items),
            _ => None,
        })
        .flat_map(|body| ast.items(body).iter().copied())
        .find(|&item| matches!(ast.item(item).kind, ItemKind::Let { .. }))
        .unwrap();
    assert_eq!(
        resolution.item_path(update),
        PathResolution {
            source: VarRef::Capture(0),
            existing: 1,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    // `:=` on a captured variable is rebinding one of an enclosing
    // block.
    let text = "let x = 1\nlet f = fn(): Int { x := 2\nx }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `x` here: it is declared in an enclosing block"
    );
    assert_eq!(
        labels(diagnostic),
        vec![(at(text, "let x = 1"), "declared here")]
    );
}

#[test]
fn calling_what_is_not_a_function() {
    let text = "let x = 1\nx(2)";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "cannot call a value of type Int");
    assert_eq!(diagnostic.span, at(text, "x(2)"));
    assert_eq!(labels(diagnostic), vec![(Span::new(10, 11), "this is Int")]);
    assert_eq!(diagnostic.notes, vec!["only functions can be called"]);
    assert_eq!(
        messages("\"s\"(1)"),
        vec!["cannot call a value of type String"]
    );
    assert_eq!(
        messages("{ x = 1 }()"),
        vec!["cannot call a value of type `{x: Int}`"]
    );
    // An untyped literal callee settles to Int, and the arguments are
    // checked on their own.
    assert_eq!(
        messages("1(2 + 1.5, 3 // 0.5)"),
        vec![
            "cannot call a value of type Int",
            "cannot floor-divide Float"
        ]
    );
    // An erroneous callee is not reported again, but its arguments are
    // checked.
    assert_eq!(
        messages("nope(1.5 // 2) + 2.5"),
        vec!["undefined variable `nope`", "cannot floor-divide Float"]
    );
}

#[test]
fn too_many_arguments() {
    let text = format!("{ADD}add(1, 2, 3)");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "expected 2 arguments, found 3");
    assert_eq!(found[0].span, at(&text, "add(1, 2, 3)"));
    assert_eq!(
        labels(&found[0]),
        vec![
            (
                nth(&text, "add", 1),
                "the function is fn(a: Int, b: Int): Int"
            ),
            (at(&text, "3"), "unexpected argument")
        ]
    );
    // The trailing block counts, as the last.
    let text = "fn one(r: {x: Int}): Int { r.x }\none({ x = 1 }) { x = 2 }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "expected 1 argument, found 2");
    assert_eq!(
        labels(diagnostic),
        vec![
            (nth(text, "one", 1), "the function is fn(r: {x: Int}): Int"),
            (at(text, "{ x = 2 }"), "unexpected argument")
        ]
    );
    // Every argument is still checked, and the call is erroneous without
    // cascading.
    assert_eq!(
        messages(&format!("{ADD}add(1, 2, 1.5 // 2) + 2.5")),
        vec!["expected 2 arguments, found 3", "cannot floor-divide Float"]
    );
    assert_eq!(
        messages("fn z(): Int { 1 }\nz(1, 2)"),
        vec!["expected 0 arguments, found 2"]
    );
}

#[test]
fn named_arguments_that_match_no_parameter() {
    let text = format!("{ADD}add(1, z = 2)");
    let found = diagnostics(&text);
    assert_eq!(found[0].message, "unknown argument `z`");
    assert_eq!(found[0].span, at(&text, "z"));
    assert_eq!(
        labels(&found[0]),
        vec![(
            nth(&text, "add", 1),
            "the function is fn(a: Int, b: Int): Int"
        )]
    );
    // `b` is then missing.
    assert_eq!(found[1].message, "missing argument `b`");
    assert_eq!(found.len(), 2);
    // A function whose parameters have no names.
    let text = format!("{ADD}let f: fn(Int, Int): Int = add\nf(1, x = 2)");
    let found = diagnostics(&text);
    assert_eq!(
        found[0].message,
        "the parameters of this function have no names"
    );
    assert_eq!(found[0].span, last(&text, "x"));
    let callee = last(&text, "f(");
    assert_eq!(
        labels(&found[0]),
        vec![(
            Span::new(callee.start, callee.start + 1),
            "this is fn(Int, Int): Int"
        )]
    );
    assert_eq!(found[0].notes, vec!["call it with positional arguments"]);
    assert_eq!(found[1].message, "missing argument 2 of type Int");
    // With some names, a named argument matches a named parameter only.
    let with_b = format!("{ADD}let f: fn(Int, b: Int): Int = add\n");
    assert_eq!(result_type(&format!("{with_b}f(b = 2, 1)")), TyId::INT);
    assert_eq!(
        messages(&format!("{with_b}f(1, c = 2)")),
        vec!["unknown argument `c`", "missing argument `b`"]
    );
    // With no parameters at all, a name is unknown.
    assert_eq!(
        messages("fn z(): Int { 1 }\nz(x = 1)"),
        vec!["unknown argument `x`"]
    );
}

#[test]
fn a_parameter_given_twice() {
    // Named after positional.
    let text = format!("{ADD}add(1, a = 2, b = 3)");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "argument `a` is given twice");
    assert_eq!(found[0].span, at(&text, "a = 2"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(&text, "1, a"), "first given here")]
            .into_iter()
            .map(|(span, label)| (Span::new(span.start, span.start + 1), label))
            .collect::<Vec<_>>()
    );
    // Positional after named.
    let text = format!("{ADD}add(a = 1, 2, b = 3)");
    let found = diagnostics(&text);
    assert_eq!(found[0].message, "argument `a` is given twice");
    assert_eq!(found[0].span, at(&text, "2"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(&text, "a = 1"), "first given here")]
    );
    // Named twice.
    assert_eq!(
        messages(&format!("{ADD}add(b = 1, b = 2, a = 3)")),
        vec!["argument `b` is given twice"]
    );
    // The trailing block and a named argument for the last parameter.
    let text = "fn g(a: Int, r: {x: Int}): Int { a }\ng(1, r = { x = 1 }) { x = 2 }";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "argument `r` is given twice");
    assert_eq!(found[0].span, at(text, "{ x = 2 }"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "r = { x = 1 }"), "first given here")]
    );
    // An unnamed parameter given twice is numbered.
    let text = format!("{ADD}let f: fn(Int, a: Int): Int = add\nf(a = 1, 2, 3)");
    assert_eq!(messages(&text), vec!["argument `a` is given twice"]);
}

#[test]
fn parameters_left_unfilled() {
    let three = "fn three(a: Int, b: Int, c: Int): Int { a + b + c }\n";
    // With positional arguments only, the call is short as a whole.
    let text = format!("{three}three(1)");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "expected 3 arguments, found 1");
    assert_eq!(found[0].span, at(&text, "three(1)"));
    assert_eq!(
        labels(&found[0]),
        vec![(
            nth(&text, "three", 1),
            "the function is fn(a: Int, b: Int, c: Int): Int"
        )]
    );
    assert_eq!(
        messages(&format!("{three}three()")),
        vec!["expected 3 arguments, found 0"]
    );
    // With a named argument, each missing parameter, at the `)`.
    let text = format!("{three}three(c = 1)");
    let found = diagnostics(&text);
    let shown: Vec<(&str, Span)> = found.iter().map(|d| (d.message.as_str(), d.span)).collect();
    let close = last(&text, ")");
    assert_eq!(
        shown,
        vec![
            ("missing argument `a`", close),
            ("missing argument `b`", close)
        ]
    );
    assert_eq!(
        labels(&found[1]),
        vec![(
            nth(&text, "three", 1),
            "the function is fn(a: Int, b: Int, c: Int): Int"
        )]
    );
    // With a trailing block, at its `}`; an unnamed parameter by its
    // number and type.
    let text = "fn g(a: Int, b: {}, c: Int): Int { a }\n\
                    let f: fn(Int, {}, Int): Int = g\nf(1) { 3 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "missing argument 2 of type `{}`");
    assert_eq!(found[0].span, last(text, "}"));
    let callee = last(text, "f(");
    assert_eq!(
        labels(&found[0]),
        vec![(
            Span::new(callee.start, callee.start + 1),
            "the function is fn(Int, {}, Int): Int"
        )]
    );
}

#[test]
fn arguments_are_checked_against_their_parameters() {
    let text = format!("{ADD}add(1, \"s\")");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(&text, "\"s\""));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(
            nth(&text, "add", 1),
            "the function is fn(a: Int, b: Int): Int"
        )]
    );
    // Literals adapt, and subtypes are accepted.
    let ast = parse_text("fn half(x: Float): Float { x / 2 }\nhalf(3)");
    let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
    let three = ast
        .expr_ids()
        .find(|&id| ast.get(id).kind == ExprKind::Int(3))
        .unwrap();
    assert_eq!(checked.types.get(three), TyId::FLOAT);
    assert_eq!(
        result_type("fn x(r: {x: Int}): Int { r.x }\nlet p = { x = 1; y = 2 }\nx(p)"),
        TyId::INT
    );
    // A function argument, by the function subtyping rules. A pure
    // function whose parameter is present fits an impure callback type
    // that promises to send it present values.
    assert_eq!(
        shown_result(
            "fn apply(f: fn(Present<Int>) -> Int, v: Int) -> Int { f(v) }\n\
                 apply(fn(n: Int): Int { n + 1 }, 1)"
        ),
        "Int | Future<Int>"
    );
    assert_eq!(
        messages(
            "fn apply(f: fn(Int): Int): Int { f(1) }\n\
                 apply(fn(n: Int) -> Int { n })"
        ),
        vec!["mismatched types"]
    );
}

#[test]
fn binding_arguments_to_parameters() {
    let service = "fn service(name: String, config: {port: Int, image: String}): String \
             { \"{name}:{config.port}\" }\n";
    let bound = |text: &str| {
        let (ast, resolution) = resolved(text);
        let call = ast
            .expr_ids()
            .filter(|&id| matches!(ast.get(id).kind, ExprKind::Call { .. }))
            .last()
            .unwrap();
        let ExprKind::Call { args, .. } = ast.get(call).kind else {
            unreachable!();
        };
        resolution.arg_params(args).to_vec()
    };
    // The trailing block fills the last parameter, after positional or
    // named arguments.
    assert_eq!(
        bound(&format!(
            "{service}service(\"web\") {{ port = 80; image = \"nginx\" }}"
        )),
        vec![0, 1]
    );
    assert_eq!(
        bound(&format!(
            "{service}service(name = \"web\") {{ port = 80; image = \"nginx\" }}"
        )),
        vec![0, 1]
    );
    // Named arguments in any order, and mixed with positional ones.
    let f = "fn f(a: Int, b: String, c: Bool): Int { a }\n";
    assert_eq!(
        bound(&format!("{f}f(c = true, a = 1, b = \"s\")")),
        vec![2, 0, 1]
    );
    assert_eq!(
        bound(&format!("{f}f(1, c = true, b = \"s\")")),
        vec![0, 2, 1]
    );
    assert_eq!(bound(&format!("{f}f(c = true, 1, \"s\")")), vec![2, 0, 1]);
    // A name alone after a named argument fills the parameter of its name.
    assert_eq!(
        bound(&format!("{f}let b = \"s\"\nf(1, c = true, b)")),
        vec![0, 2, 1]
    );
    // A shorthand of no parameter's name is the unknown name it is.
    let text = "fn h(alpha: Int, beta: Int): Int { alpha }\nlet betta = 1\nh(alpha = 1, betta)";
    let error = &diagnostics(text)[0];
    assert_eq!(error.message, "unknown argument `betta`");
    assert_eq!(
        error.notes,
        ["`betta` alone after a named argument is shorthand for `betta = betta`"]
    );
    // The suggestion names the parameter and keeps the variable.
    let suggestion = error.suggestion.as_deref().unwrap();
    assert_eq!(suggestion.replacement, "beta = betta");
    // A call with only a trailing block.
    assert_eq!(bound("fn run(body: {}): Int { 1 }\nrun { }"), vec![0]);
    // A trailing block is checked against a record parameter field by
    // field.
    let text = format!("{service}service(\"web\") {{ port = 80 }}");
    let diagnostic = &diagnostics(&text)[0];
    assert_eq!(diagnostic.message, "missing field `image` in record");
    assert_eq!(
        labels(diagnostic),
        vec![(
            nth(&text, "service", 1),
            "the function is fn(name: String, config: {image: String, port: Int}): String"
        )]
    );
    let text = format!("{service}service(\"web\") {{ port = 80; image = \"x\"; extra = 1 }}");
    let warning = &warnings(&text)[0];
    assert_eq!(
        warning.message,
        "field `extra` is not in `{image: String, port: Int}`"
    );
    // An erroneous call binds nothing.
    let ast = parse_text(&format!("{ADD}add(1, 2, 3)"));
    let mut types = TypeTable::new();
    let mut checker = Checker::new(&ast, &mut types);
    let ExprKind::Call { args, .. } = ast.get(result_expr(&ast)).kind else {
        panic!("expected a call");
    };
    checker.check_expr(result_expr(&ast));
    assert!(
        checker.arg_params[args.start()..args.start() + args.count()]
            .iter()
            .all(|&param| param == u32::MAX)
    );
}

#[test]
fn calls_have_their_return_type() {
    assert_eq!(shown_result(&format!("{ADD}add(1, 2)")), "Int");
    assert_eq!(shown_result("fn r(): {x: Int} { x = 1 }\nr()"), "{x: Int}");
    assert_eq!(shown_result("fn r(): {x: Int} { x = 1 }\nr().x"), "Int");
    assert_eq!(
        shown_result("fn make(): fn(Int): Int { fn(n: Int): Int { n } }\nmake()(1)"),
        "Int"
    );
    assert_eq!(
        shown_result("let m = { square = fn(v: Int): Int { v * v } }\nm.square(4)"),
        "Int"
    );
    // A call's value is used like any other.
    assert_eq!(
        messages(&format!("{ADD}add(1, 2) + 1.5")),
        vec!["cannot add Int and Float"]
    );
    assert_eq!(
        messages(&format!("{ADD}add(1, 2)\n1")),
        vec!["unused value of type Int"]
    );
    assert_eq!(
        result_type("fn tick() -> Present<{}> {}\ntick()\n1"),
        TyId::INT
    );
    // An impure function's `{}` result that may be a future: a statement
    // expects `{}`, so it is awaited there, and its effects are not lost.
    let tick = "fn tick() -> {} | Future<{}> {}\n";
    assert_eq!(
        marks(&format!("{tick}tick()\n1")),
        vec![("tick()", AwaitMark::Top)]
    );
    assert_eq!(result_type(&format!("{tick}tick()\n1")), TyId::INT);
    assert_eq!(result_type(&format!("{tick}await tick()\n1")), TyId::INT);
    // One inferred to be present is not awaited.
    assert!(marks("fn tick() -> {} {}\ntick()\n1").is_empty());
    assert_eq!(result_type("fn tick() -> {} {}\ntick()\n1"), TyId::INT);
    // Which a pure function cannot do (the call is reported first: it
    // is checked before the statement's value is used).
    assert_eq!(
        messages(&format!("{tick}fn p(): Int {{ tick()\n1 }}")),
        vec![
            "cannot call an impure function from a pure function",
            "cannot use a value that may be a future in a pure function"
        ]
    );
}

#[test]
fn a_pure_function_cannot_call_an_impure_one() {
    // `tick` returns a present `Int`, so that a pure caller does no
    // more than call it: an impure function's bare `Int` result may be
    // a future, which a pure caller could not use either (see
    // `a_pure_function_cannot_await`).
    let text = "fn tick() -> Present<Int> { 1 }\nfn pure(): Int { tick() }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, nth(text, "tick()", 1));
    assert_eq!(
        labels(&found[0]),
        vec![
            (last(text, "tick"), "this is fn() -> Int"),
            (last(text, ":"), "this function is pure, declared with `:`")
        ]
    );
    assert_eq!(
        found[0].notes,
        vec![
            "a pure function, declared with `:`, may call only pure functions; declare it \
                 with `->` to make it impure"
        ]
    );
    let tick = "fn tick() -> Present<Int> { 1 }\n";
    // At any depth of blocks in the pure function.
    assert_eq!(
        messages(&format!(
            "{tick}fn pure(): Int {{ if true {{ {{ let x = tick()\nx }} }} else {{ 0 }} }}"
        )),
        vec!["cannot call an impure function from a pure function"]
    );
    // A pure function in an impure one is pure.
    assert_eq!(
        messages(&format!(
            "{tick}fn outer() -> Int {{ let g = fn(): Int {{ tick() }}\ng() }}"
        )),
        vec!["cannot call an impure function from a pure function"]
    );
    // Receiving an impure function and calling it.
    assert_eq!(
        messages("fn call(f: fn() -> Present<Int>): Int { f() }"),
        vec!["cannot call an impure function from a pure function"]
    );
    // One whose result may be a future is also used as its bits, as the
    // body's value, which is reported at the body.
    assert_eq!(
        messages("fn call(f: fn() -> Int): Int { f() }"),
        vec![
            "cannot use a value that may be a future in a pure function",
            "cannot call an impure function from a pure function"
        ]
    );
    // Allowed: pure in impure, pure in pure, impure in impure, impure
    // at the top level, and a pure function creating, storing,
    // receiving and returning impure ones.
    for text in [
        format!("{tick}fn pure(): Int {{ 1 }}\nfn impure() -> Int {{ pure() + tick() }}"),
        format!("{tick}fn a(): Int {{ 1 }}\nfn b(): Int {{ a() }}"),
        format!("{tick}tick()"),
        format!("{tick}fn make(): fn() -> Int {{ fn() -> Int {{ tick() }} }}\nmake()()"),
        format!("{tick}fn keep(f: fn() -> Int): {{f: fn() -> Int}} {{ f = f }}\nkeep(tick).f()"),
    ] {
        assert_eq!(
            check(&parse_text(&text), &mut TypeTable::new()).err(),
            None,
            "{text}"
        );
    }
    // The purity of a call, not of its arguments: a pure function may
    // pass an impure one along.
    assert_eq!(
        messages(&format!(
            "{tick}fn pass(f: fn() -> Present<Int>): fn() -> Present<Int> {{ f }}\n\
                 fn p(): Int {{ pass(tick)() }}"
        )),
        vec!["cannot call an impure function from a pure function"]
    );
}

#[test]
fn functions_have_no_equality() {
    for op in ["==", "!=", "===", "!=="] {
        let text = format!("let f = fn(x: Int): Int {{ x }}\nf {op} f");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(found[0].message, "cannot compare functions");
        let operator = last(&text, op);
        assert_eq!(found[0].span, operator);
        let lhs = Span::new(operator.start - 2, operator.start - 1);
        let rhs = Span::new(operator.end + 1, operator.end + 2);
        assert_eq!(
            labels(&found[0]),
            vec![
                (lhs, "this is fn(x: Int): Int"),
                (rhs, "this is fn(x: Int): Int")
            ]
        );
        // Nor do records containing them, at any depth.
        let text = format!("let r = {{ f = fn(): Int {{ 1 }} }}\nr {op} r");
        let found = diagnostics(&text);
        assert_eq!(
            found[0].message,
            "cannot compare `{f: fn(): Int}`: it contains a function"
        );
        assert_eq!(labels(&found[0])[0].1, "this is {f: fn(): Int}");
        let text = format!("let r = {{ a = {{ b = {{ f = fn() -> {{}} {{}} }} }} }}\nr {op} r");
        assert_eq!(
            messages(&text),
            vec!["cannot compare `{a: {b: {f: fn() -> {}}}}`: it contains a function"]
        );
    }
    // The operands must first have one identical type.
    assert_eq!(
        messages("let f = fn(): Int { 1 }\nlet g = fn(): Float { 1.0 }\nf == g"),
        vec!["cannot compare `fn(): Int` and `fn(): Float`"]
    );
    assert_eq!(
        messages("let f = fn(): Int { 1 }\nf == 1"),
        vec!["cannot compare `fn(): Int` and Int"]
    );
    // Nor an order.
    assert_eq!(
        messages("let f = fn(): Int { 1 }\nf < f"),
        vec!["cannot order `fn(): Int`"]
    );
}

#[test]
fn functions_with_other_operators() {
    assert_eq!(
        messages("fn(): Int { 1 } + 1"),
        vec!["cannot add `fn(): Int` and Int"]
    );
    assert_eq!(
        messages("let f = fn(): Int { 1 }\nf.x"),
        vec!["cannot access field `x` of `fn(): Int`"]
    );
    assert_eq!(
        messages("if fn(): Bool { true } { 1 } else { 2 }"),
        vec!["mismatched types"]
    );
    // A function can be shown as text.
    assert_eq!(result_type("\"{fn(): Int { 1 }}\""), TyId::STRING);
    // `if` branches of function types join.
    assert_eq!(
        shown_result(
            "let f = if true { fn(a: Int): Int { a } } \
                 else { fn(b: Present<Int>) -> Int { b } }\nf"
        ),
        "fn(Int) -> Int"
    );
    // A parameter is the narrower of the two: an impure function's bare
    // one may be a future, which a pure one that measures its own
    // cannot take.
    assert_eq!(
        shown_result(
            "let f = if true { fn(a: Int): Int { a } } else { fn(b: Int) -> Int { b } }\nf"
        ),
        "fn(Int) -> Int | Future<Int>"
    );
    // Two functions written alike and inferred differently.
    assert_eq!(
        shown_result(
            "let f = if true { fn(x: Int): Int { x * 2 } } else { fn(x: Int): Int { 5 } }\nf"
        ),
        "fn(x: Int): Int"
    );
    assert_eq!(
        shown_result(
            "let h = if true { fn(f: fn(Int) -> Int) -> Int { f(1) } } \
                 else { fn(f: fn(Int) -> Int) -> Int { let g = f\ng(1) } }\nh"
        ),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    // Unrelated parameter types join to the union of the two.
    assert_eq!(
        warnings("if true { fn(a: Int): Int { a } } else { fn(a: String): Int { 1 } }")[0].message,
        "the branches of this `if` have different types"
    );
    assert_eq!(
        shown_result(
            "let f = if true { fn(a: Int): {x: Int, y: Int} { x = a; y = a } } \
                 else { fn(a: Int): {x: Int} { x = a } }\nf"
        ),
        "fn(a: Int): {x: Int}"
    );
    assert_eq!(
        warnings("if true { fn(a: Int): Int { a } } else { fn(): Int { 1 } }")[0].message,
        "the branches of this `if` have different types"
    );
    // Annotations and ascriptions with function types.
    assert_eq!(
        shown_result("let f: fn(Present<Int>) -> Int = fn(x: Int): Int { x }\nf"),
        "fn(Int) -> Int | Future<Int>"
    );
    assert_eq!(
        messages("let f: fn(Int): Int = fn(x: Int) -> Int { x }"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("let f: fn(Nope): Int = g"),
        vec!["unknown type `Nope`", "undefined variable `g`"]
    );
}
