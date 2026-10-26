//! Tests of the checker: mutable variables and index paths.

use super::*;

/// The item of `ast`, parsed from `text`, written first at `needle`.
pub(super) fn item_at(ast: &Ast, text: &str, needle: &str) -> ItemId {
    let span = at(text, needle);
    (0..u32::try_from(ast.item_count()).unwrap())
        .map(ItemId::new)
        .find(|&id| ast.item(id).span == span)
        .unwrap_or_else(|| panic!("no item at {needle:?} in {text:?}"))
}

/// The first byte of `span`.
pub(super) fn head(span: Span) -> Span {
    Span::new(span.start, span.start + 1)
}

/// The expression of `ast` that spans `span`.
pub(super) fn expr_at(ast: &Ast, span: Span) -> ExprId {
    ast.expr_ids()
        .find(|&id| ast.get(id).span == span)
        .unwrap_or_else(|| panic!("no expression at {span:?}"))
}

/// The note on rebinding a variable that is not mutable from an inner
/// block, for a variable named `name`.
pub(super) fn enclosing_note(name: &str) -> String {
    format!(
        "shadow it with `let {name} = ...` instead; only a variable declared with `let mut` \
             can be rebound from an inner block"
    )
}

#[test]
fn a_mutable_variable_is_rebound_from_any_block() {
    for text in [
        // The declaring block, and an `if`'s branches.
        "let mut x = 1\nx := 2\nx",
        "let mut x = 1\nif true { x := 2 }\nx",
        "let mut x = 1\nif false { } else { x := 2 }\nx",
        "let mut x = 1\nif false { } else if true { x := 2 }\nx",
        // A `for`'s body, and a block in its guard.
        "let mut x = 0\nfor i in [1, 2] { x := x + i }\nx",
        "let mut x = 0\nlet ys = for i in [1, 2] if { x := x + i; x > 1 } { i }\nx",
        // A nested block, several levels deep, and a record block.
        "let mut x = 0\n{ { x := 1 } }\nx",
        "let mut x = 0\nfor i in [1] { if true { { x := i } } }\nx",
        "let mut x = 0\nlet r = { a = 1; x := 2; b = x }\nx",
        // A block that is a value: an initialiser, an argument, an
        // operand.
        "let mut x = 0\nlet y = { x := 1; 2 }\nx + y",
        "fn id(n: Int): Int { n }\nlet mut x = 0\nid({ x := 1; x }) + x",
        // A function's own variable, from its blocks.
        "fn f(): Int { let mut x = 0; if true { x := 1 }; x }\nf()",
        "let f = fn(n: Int): Int { let mut x = n; for i in 0..n { x := x + i }; x }\nf(2)",
        // A record block declaring and rebinding one of its own.
        "let r = { let mut t = 1; { t := 2 }; a = t }\nr.a",
    ] {
        assert_eq!(shown_result(text), "Int", "{text}");
    }
}

#[test]
fn a_mutable_variable_keeps_its_type() {
    // Its type is fixed where it is declared, with or without an
    // annotation, and literals adapt to it.
    assert_eq!(shown_result("let mut x: Float = 0\n{ x := 1 }\nx"), "Float");
    assert_eq!(shown_result("let mut x = 0.5\n{ x := 1 }\nx"), "Float");
    assert_eq!(
        shown_result("let mut r: {a: Int} = { a = 1; b = 2 }\n{ r := { a = 3; c = 4 } }\nr"),
        "{a: Int}"
    );
    // A mismatch is reported against the declaration, however deep the
    // rebind, and even after earlier rebinds.
    let text = "let mut x = 0\nx := 1\nfor i in [1] { { x := 1.5 } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "1.5"));
    assert_eq!(found[0].label.as_deref(), Some("expected Int, found Float"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "let mut x = 0"), "the variable was declared here")]
    );
    // So a variable is not retyped by a rebind, as it is by a `let`.
    assert_eq!(
        messages("let mut x = 1\n{ x := \"s\" }\nx + 1"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_mutable_variable_has_one_slot() {
    let text = "let mut x = 0\nx := 1\n{ x := 2 }\nfor i in [1] { x := i }\nx";
    let (ast, resolution) = resolved(text);
    let declared = item_at(&ast, text, "let mut x = 0");
    assert_eq!(resolution.item_slot(declared), Slot(0));
    for rebind in ["x := 1", "x := 2", "x := i"] {
        let item = item_at(&ast, text, rebind);
        assert_eq!(resolution.item_slot(item), Slot(0), "{rebind}");
    }
    // The `for`'s variable is the one other slot; the rebinds take none.
    assert_eq!(resolution.frame_size(), 2);
    assert_eq!(
        resolution.expr_var(result_expr(&ast)),
        VarRef::Local(Slot(0))
    );
    let names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| ast.names().get(name))
        .collect();
    assert_eq!(names, vec!["x", "i"]);
    // However many rebinds there are, where a variable that is not
    // mutable takes a slot for each.
    let rebinds = "\nx := x + 1".repeat(10);
    let (_, resolution) = resolved(&format!("let mut x = 0{rebinds}\nx"));
    assert_eq!(resolution.frame_size(), 1);
    let (_, resolution) = resolved(&format!("let x = 0{rebinds}\nx"));
    assert_eq!(resolution.frame_size(), 11);
    // In a function's frame too.
    let text = "fn f(n: Int): Int { let mut x = n; { x := 1 }; x := 2; x }\nf(1)";
    let (ast, resolution) = resolved(text);
    assert_eq!(resolution.function(FnId::new(0)).frame_size, 2);
    for rebind in ["x := 1", "x := 2"] {
        let item = item_at(&ast, text, rebind);
        assert_eq!(resolution.item_slot(item), Slot(1), "{rebind}");
    }
}

#[test]
fn a_variable_that_is_not_mutable_is_rebound_as_before() {
    // In its declaring block, a rebind is a new binding in a new slot.
    let text = "let x = 0\nx := 1\nx";
    let (ast, resolution) = resolved(text);
    assert_eq!(resolution.item_slot(item_at(&ast, text, "x := 1")), Slot(1));
    assert_eq!(resolution.frame_size(), 2);
    // From an inner block it is an error, with the reworded note, for a
    // `let`, a parameter and a `for`'s variable alike.
    for (text, name, declaration) in [
        ("let x = 1\n{ x := 2 }", "x", "let x = 1"),
        ("fn f(n: Int): Int { if true { n := 1 }; n }", "n", "n: Int"),
        ("let ys = for v in [1] { v := 2; v }", "v", "v"),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message,
            format!("cannot rebind `{name}` here: it is declared in an enclosing block"),
            "{text}"
        );
        assert_eq!(found[0].span, head(last(text, &format!("{name} :="))));
        assert_eq!(
            labels(&found[0]),
            vec![(at(text, declaration), "declared here")],
            "{text}"
        );
        assert_eq!(found[0].notes, vec![enclosing_note(name)], "{text}");
    }
}

#[test]
fn mutable_and_immutable_variables_shadow_each_other() {
    // A `let` over a `let mut` is a new variable, which is not mutable.
    let text = "let mut x = 1\nlet x = 2\n{ x := 3 }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot rebind `x` here: it is declared in an enclosing block"
    );
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "let x = 2"), "declared here")]
    );
    // So is a dotted `let` of a mutable variable.
    assert_eq!(
        messages("let mut r = { a = 1 }\nlet r.a = 2\n{ r.a := 3 }"),
        vec!["cannot rebind `r` here: it is declared in an enclosing block"]
    );
    // A `let mut` over a `let` is a new variable, which is mutable, in a
    // slot of its own.
    let text = "let x = 1\nlet mut x = 2.5\n{ x := 3.5 }\nx";
    let (ast, resolution) = resolved(text);
    assert_eq!(
        resolution.item_slot(item_at(&ast, text, "let mut x = 2.5")),
        Slot(1)
    );
    assert_eq!(
        resolution.item_slot(item_at(&ast, text, "x := 3.5")),
        Slot(1)
    );
    assert_eq!(shown_result(text), "Float");
    // An inner variable that is not mutable, over an outer mutable one,
    // is rebound in its own block as any such variable is, with a new
    // binding, leaving the outer one alone; from a block inside it, it
    // is the error.
    let text = "let mut x = 1\n{ let x = 2.5\nx := 3.5 }\nx := 4\nx";
    let (ast, resolution) = resolved(text);
    assert_eq!(
        resolution.item_slot(item_at(&ast, text, "x := 3.5")),
        Slot(2)
    );
    assert_eq!(resolution.item_slot(item_at(&ast, text, "x := 4")), Slot(0));
    assert_eq!(shown_result(text), "Int");
    assert_eq!(
        messages("let mut x = 1\n{ let x = 2.5\n{ x := 3.5 } }"),
        vec!["cannot rebind `x` here: it is declared in an enclosing block"]
    );
    // An inner mutable variable over an outer one is the one rebound.
    let text = "let mut x = 1\n{ let mut x = 2.5\n{ x := 3.5 } }\nx";
    assert_eq!(shown_result(text), "Int");
    let (ast, resolution) = resolved(text);
    assert_eq!(
        resolution.item_slot(item_at(&ast, text, "x := 3.5")),
        Slot(1)
    );
}

#[test]
fn a_mutable_variable_cannot_be_rebound_from_a_function_or_a_lazy() {
    let function_note = "a function holds its own copy of each variable it uses, made when \
                             the function is created; rebinding the copy would not change `x`";
    let lazy_note = "a `lazy` holds its own copy of each variable it uses, made when the \
                         `lazy` is created; rebinding the copy would not change `x`";
    for (text, what, note) in [
        // A top-level variable, one and two levels in.
        (
            "let mut x = 1\nlet f = fn(): Int { x := 2; x }",
            "function",
            function_note,
        ),
        (
            "let mut x = 1\nlet l = lazy { x := 2; x }",
            "`lazy`",
            lazy_note,
        ),
        (
            "let mut x = 1\nlet f = fn(): Int { let g = fn(): Int { x := 2; x }; 1 }",
            "function",
            function_note,
        ),
        (
            "let mut x = 1\nlet f = fn(): Int { let l = lazy { x := 2; x }; 1 }",
            "`lazy`",
            lazy_note,
        ),
        (
            "let mut x = 1\nlet l = lazy { let f = fn(): Int { x := 2; x }; 1 }",
            "function",
            function_note,
        ),
        // A function's variable, one and two levels in.
        (
            "fn f(): Int { let mut x = 1; let g = fn(): Int { x := 2; x }; x }",
            "function",
            function_note,
        ),
        (
            "fn f() -> Int { let mut x = 1; let l = lazy { x := 2; x }; x }",
            "`lazy`",
            lazy_note,
        ),
        (
            "fn f(): Int { let mut x = 1; let g = fn(): Int { let h = fn(): Int { x := 2; x \
                 }; 1 }; x }",
            "function",
            function_note,
        ),
        // From a block of the function, and dotted.
        (
            "let mut x = 1\nlet f = fn(): Int { if true { x := 2 }; x }",
            "function",
            function_note,
        ),
        (
            "let mut x = { a = 1 }\nlet f = fn(): Int { x.a := 2; 1 }",
            "function",
            function_note,
        ),
    ] {
        let found = diagnostics(text);
        // The value is checked on its own and nothing is declared, so
        // nothing cascades.
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message,
            format!("cannot rebind `x` here: it is declared outside this {what}"),
            "{text}"
        );
        let rebind = text.rfind("x :=").or_else(|| text.rfind("x.a :=")).unwrap();
        let rebind = u32::try_from(rebind).unwrap();
        assert_eq!(found[0].span, Span::new(rebind, rebind + 1), "{text}");
        let declaration = at(text, "let mut x = ");
        let declaration = ast_item_span(text, declaration.start);
        assert_eq!(
            labels(&found[0]),
            vec![(declaration, "declared here")],
            "{text}"
        );
        assert_eq!(found[0].notes, vec![note], "{text}");
    }
    // The value is still checked.
    assert_eq!(
        messages("let mut x = 1\nlet f = fn(): Int { x := 1 + \"s\"; 1 }"),
        vec![
            "cannot rebind `x` here: it is declared outside this function",
            "cannot add Int and String"
        ]
    );
    // Reading the variable there captures it, by value, as any.
    let text = "let mut x = 1\nlet f = fn(): Int { x }\nx := 2\nlet l = lazy x\nf()";
    let (ast, resolution) = resolved(text);
    for function in [0, 1] {
        assert_eq!(
            resolution.function(FnId::new(function)).captures,
            vec![(VarRef::Local(Slot(0)), TyId::INT)]
        );
    }
    let read = expr_at(&ast, head(last(text, "x }")));
    assert_eq!(resolution.expr_var(read), VarRef::Capture(0));
}

/// The span of the item of `text` that starts at `start`.
pub(super) fn ast_item_span(text: &str, start: u32) -> Span {
    let ast = parse_text(text);
    (0..u32::try_from(ast.item_count()).unwrap())
        .map(|index| ast.item(ItemId::new(index)).span)
        .find(|span| span.start == start)
        .unwrap_or_else(|| panic!("no item at {start} in {text:?}"))
}

#[test]
fn rebinding_a_mutable_variable_is_allowed_in_any_function() {
    // Nothing outside the function can see it, so a pure function may.
    assert_eq!(
        type_of_f("fn f(n: Int): Int { let mut x = 0; for i in 0..n { x := x + i }; x }"),
        "fn(n: Int): Int"
    );
    assert_eq!(
        type_of_f("fn f(n: Int) ->> Int { let mut x = 0; if n > 1 { x := n }; x }"),
        "fn(n: Int) ->> Int"
    );
    assert_eq!(
        type_of_f("fn f(n: Int) -> Int { let mut x = 0; { x := 1 }; x }"),
        "fn(n: Int | Future<Int>) -> Int"
    );
    // A value awaited to fit is the usual purity error.
    let text = "fn f(l: Future<Int>): Int { let mut x = 0; if true { x := l }; x }";
    assert_implicit_await_error(text, last(text, "l"), "Future<Int>");
}

#[test]
fn a_mutable_variable_that_may_be_a_future() {
    // It has its declared type at every use, so it is measured at each.
    let text = "let mut r: Int | Future<Int> = lazy 1\nlet a = r + 1\nr := 2\n\
                    { r := lazy 3 }\nlet b = r + 1\nb";
    assert_eq!(
        marks(text),
        vec![("r", AwaitMark::Top), ("r", AwaitMark::Top)]
    );
    assert_eq!(
        shown_result("let mut r: Int | Future<Int> = 1\n{ r := lazy 2 }\nr"),
        "Int | Future<Int>"
    );
    // A future where the type does not allow one is awaited to fit.
    let text = "let mut x = 1\nlet l = lazy 2\n{ x := l }\nx";
    assert_eq!(marks(text), vec![("l", AwaitMark::Top)]);
    assert_eq!(shown_result(text), "Int");
    let text = "let mut x = { a = 1 }\nlet l = lazy { a = 2 }\n{ x := l }\nx";
    assert_eq!(marks(text), vec![("l", AwaitMark::Top)]);
}

#[test]
fn dotted_rebinds_of_a_mutable_variable() {
    // From an inner block, writing the variable's own slot.
    let text = "let mut c = { port = 80; host = \"a\" }\nif true { c.port := 443 }\nc";
    assert_eq!(shown_result(text), "{host: String, port: Int}");
    let (ast, resolution) = resolved(text);
    let item = item_at(&ast, text, "c.port := 443");
    assert_eq!(resolution.item_slot(item), Slot(0));
    assert_eq!(
        resolution.item_path(item),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 1,
            awaits: PathResolution::NOT_AWAITED,
        }
    );
    // Deeper, and in a loop.
    let text = "let mut c = { tls = { on = false; port = 1 } }\n\
                    for p in [2, 3] { c.tls.port := c.tls.port + p }\nc.tls.port";
    assert_eq!(shown_result(text), "Int");
    // The variable keeps its declared type: one that may be a future is
    // updated once awaited, and is still awaited where it is used
    // after, as it is not once a variable that is not mutable is.
    let text = "let mut r: {a: Int} | Future<{a: Int}> = lazy { a = 1 }\n\
                    { r.a := 2 }\nr.a";
    assert_eq!(marks(text), vec![("r", AwaitMark::Top)]);
    assert_eq!(shown_result(text), "Int");
    let (ast, resolution) = resolved(text);
    let path = resolution.item_path(item_at(&ast, text, "r.a := 2"));
    assert_eq!(resolution.path_awaits(path), Some(&[true][..]));
    let text = "let r: {a: Int} | Future<{a: Int}> = lazy { a = 1 }\nr.a := 2\nr.a";
    assert_eq!(marks(text), Vec::new());
    // A record along the path that the type gives as a field of the
    // either shape is updated as present.
    let text = "let mut r: {b: {c: Int} | Future<{c: Int}>} = { b = { c = 1 } }\n\
                    { r.b.c := 2 }\nr.b.c";
    assert_eq!(shown_result(text), "Int");
    assert_eq!(marks(text), vec![("r.b", AwaitMark::Top)]);
    // A segment that does not exist, or a variable that is not a
    // record, is an error as for any `:=`.
    assert_eq!(
        messages("let mut r = { a = 1 }\n{ r.b := 2 }"),
        vec!["cannot rebind `r.b`: `r` has no field `b`"]
    );
    assert_eq!(
        messages("let mut n = 1\n{ n.a := 2 }"),
        vec!["cannot update `n.a`: `n` is Int, not a record"]
    );
    assert_eq!(
        messages("let mut r = { a = 1 }\n{ r.a := 2.5 }"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_dotted_rebind_must_keep_the_variables_type() {
    let note = "a mutable variable keeps its type; declare it with a type that allows both, \
                    or await it where it is declared";
    let text = "let mut r: Future<{a: Int}> = lazy { a = 1 }\nr.a := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot rebind `r.a`: `r` is declared `Future<{a: Int}>`, and the update makes it \
             `{a: Int}`"
    );
    assert_eq!(found[0].span, at(text, "r.a"));
    assert_eq!(
        labels(&found[0]),
        vec![(
            at(text, "let mut r: Future<{a: Int}> = lazy { a = 1 }"),
            "declared here"
        )]
    );
    assert_eq!(found[0].notes, vec![note]);
    // A record along the path declared a future, likewise.
    assert_eq!(
        messages("let mut r: {b: Future<{c: Int}>} = { b = lazy { c = 1 } }\n{ r.b.c := 2 }"),
        vec![
            "cannot rebind `r.b.c`: `r` is declared `{b: Future<{c: Int}>}`, and the update \
                 makes it `{b: {c: Int}}`"
        ]
    );
    // The value is still checked.
    let mut found = messages("let mut r: Future<{a: Int}> = lazy { a = 1 }\nr.a := 2.5");
    found.sort();
    assert_eq!(
        found,
        vec![
            "cannot rebind `r.a`: `r` is declared `Future<{a: Int}>`, and the update makes \
                 it `{a: Int}`",
            "mismatched types"
        ]
    );
}

#[test]
fn a_mutable_variable_holding_a_function() {
    // Recursion inside the first function is through its own name, the
    // function being called, whatever the variable holds later.
    let text = "let mut f = fn(n: Int): Int { if n == 0 { 0 } else { f(n - 1) } }\n\
                    let g = fn(n: Int): Int { n * 2 }\nf := g\nf(1)";
    assert_eq!(shown_result(text), "Int");
    let (ast, resolution) = resolved(text);
    let inner = expr_at(&ast, head(at(text, "f(n - 1)")));
    assert_eq!(resolution.expr_var(inner), VarRef::Callee);
    assert_eq!(
        resolution.function(FnId::new(0)).callee,
        ast.names().lookup("f")
    );
    // A call through the variable is typed by its type alone: the
    // variable remembers no function expression, so the call-site form
    // of the implicit-await error does not name the function's
    // parameter, as it does for a variable that is not mutable.
    let text = "let mut f = fn(n: Int): Int { n * 2 }\n\
                    fn p(x: Int | Future<Int>): Int { f(x) }";
    assert_implicit_await_error(text, last(text, "x"), "Int | Future<Int>");
    let text = "let f = fn(n: Int): Int { n * 2 }\n\
                    fn p(x: Int | Future<Int>): Int { f(x) }";
    assert_eq!(
        messages(text),
        vec!["argument `x` would be awaited inside `f`, which is pure"]
    );
    // Its type is the type its initialiser has, an inferred result
    // included; an annotation widens it.
    assert_eq!(
        messages("let mut f = fn() -> Int { 5 }\nf := fn() -> Int { lazy 5 }"),
        vec!["mismatched types"]
    );
    assert_eq!(
        shown_result("let mut f: fn() -> Int = fn() -> Int { 5 }\nf := fn() -> Int { lazy 5 }\nf"),
        "fn() -> Int | Future<Int>"
    );
}

#[test]
fn a_pure_functions_parameter_copied_into_a_mutable_variable() {
    // Measured where the variable is, through any rebind.
    assert_eq!(
        type_of_f("fn f(n: Int): Int { let mut x = n; if true { x := x + 1 }; x }"),
        "fn(n: Int): Int"
    );
    assert_eq!(
        type_of_f("fn f(n: Int): Int { let mut x = n; { x := 5 }; x }"),
        "fn(n: Int): Int"
    );
    // Passed through unmeasured, rebound with the parameter or a
    // present value, it is the either shape.
    assert_eq!(
        type_of_f("fn f(n: Int): {v: Int | Future<Int>} { let mut x = n; { x := n }; v = x }"),
        "fn(n: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    assert_eq!(
        type_of_f("fn f(n: Int): {v: Int | Future<Int>} { let mut x = n; { x := 5 }; v = x }"),
        "fn(n: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    // Updated along a path, it is measured, as a record that is updated
    // is.
    assert_eq!(
        type_of_f("fn f(p: {a: Int}): {a: Int} { let mut r = p; { r.a := 2 }; r }"),
        "fn(p: {a: Int}): {a: Int}"
    );
}

#[test]
fn a_function_whose_result_is_a_mutable_variable() {
    // An impure function's result is what the variable's type says,
    // whatever it was last given.
    assert_eq!(
        type_of_f("fn f() -> Int { let mut r = 1; if true { r := 2 }; r }"),
        "fn() -> Int"
    );
    assert_eq!(
        type_of_f("fn f() -> Int { let mut r = lazy 1; { r := lazy 2 }; r }"),
        "fn() -> Future<Int>"
    );
    assert_eq!(
        type_of_f("fn f(e: Int) -> Int { let mut r = e; { r := 3 }; r }"),
        "fn(e: Int | Future<Int>) -> Int | Future<Int>"
    );
    assert_eq!(
        type_of_f("fn f() -> {a: Int} { let mut r = { a = 1 }; { r.a := 2 }; r }"),
        "fn() -> {a: Int}"
    );
}

#[test]
fn mutable_variables_continue_from_an_existing_scope() {
    let mut names = Names::new();
    let mut types = TypeTable::new();
    let parse_with = |text: &str, names: &mut Names| {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, strings) = tokenize_with(&source, names).unwrap();
        parse(&tokens, names.clone(), strings, &source).unwrap()
    };
    let first = parse_with(
        "let a = 1\nlet mut b = 2\nlet mut c = 3\nlet c = 4",
        &mut names,
    );
    let scope = Checker::new(&first, &mut types).check().unwrap().scope;
    let slots: Vec<Slot> = scope.mutable_slots().collect();
    assert_eq!(slots, vec![Slot(1), Slot(2)]);
    // A later input rebinds an earlier mutable variable from an inner
    // block, in its slot, taking none.
    let text = "{ b := b + 1 }\nb";
    let second = parse_with(text, &mut names);
    let output = Checker::with_scope(&second, scope.clone(), &mut types)
        .check()
        .unwrap();
    let resolution = &output.checked.resolution;
    let block = second.items(second.root())[0];
    let ItemKind::Expr(block) = second.item(block).kind else {
        panic!("expected a block");
    };
    let ExprKind::Block { items, .. } = second.get(block).kind else {
        panic!("expected a block");
    };
    assert_eq!(resolution.item_slot(second.items(items)[0]), Slot(1));
    assert_eq!(resolution.frame_size(), 4);
    assert_eq!(output.scope.len(), scope.len());
    // But not from a function, with no declaration to point at.
    let third = parse_with("let f = fn(): Int { b := 1; b }", &mut names);
    let found = Checker::with_scope(&third, scope.clone(), &mut types)
        .check()
        .unwrap_err();
    assert_eq!(
        found[0].message,
        "cannot rebind `b` here: it is declared outside this function"
    );
    assert!(found[0].labels.is_empty());
    // A mutable variable shadowed by one that is not is not rebound.
    let fourth = parse_with("{ c := 5 }", &mut names);
    let found = Checker::with_scope(&fourth, scope, &mut types)
        .check()
        .unwrap_err();
    assert_eq!(
        found[0].message,
        "cannot rebind `c` here: it is declared in an enclosing block"
    );
    assert_eq!(found[0].notes, vec![enclosing_note("c")]);
}

#[test]
fn an_index_path_updates_an_element() {
    // Every form of path, of a mutable variable and of one that is not:
    // a list's type never changes, and a record's along the path does
    // only as `:=` keeps it.
    for (text, shown) in [
        ("let mut xs = [1, 2, 3]\nxs[0] := 10\nxs", "List<Int>"),
        ("let xs = [1, 2, 3]\nxs[0] := 10\nxs", "List<Int>"),
        (
            "let mut xs = [1, 2, 3]\nfor i in 0..len(xs) { xs[i] := xs[i] * 2 }\nxs",
            "List<Int>",
        ),
        (
            "let mut conf = { hosts = [{ name = \"a\"; up = false }] }\n\
                 conf.hosts[0].up := true\nconf",
            "{hosts: List<{name: String, up: Bool}>}",
        ),
        (
            "let mut grid = [[0, 0], [0, 0]]\nlet y = 1\nlet x = 0\ngrid[y][x] := 1\ngrid",
            "List<List<Int>>",
        ),
        (
            "let xs = [{ a = [1, 2] }]\nxs[0].a[1] := 5\nxs",
            "List<{a: List<Int>}>",
        ),
        // A literal adapts to the element type.
        ("let mut xs = [1.5]\nxs[0] := 2\nxs", "List<Float>"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // Every record and list along the path exists, and is read from
    // the variable's slot.
    let text = "let mut conf = { hosts = [{ up = false }] }\nconf.hosts[0].up := true";
    let (ast, resolution) = resolved(text);
    let item = item_at(&ast, text, "conf.hosts[0].up := true");
    assert_eq!(resolution.item_slot(item), Slot(0));
    assert_eq!(
        resolution.item_path(item),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 3,
            awaits: PathResolution::NOT_AWAITED,
        }
    );
    // A variable that is not mutable takes the updated type: the
    // records and lists along the path present, the list's element type
    // as it was.
    assert_eq!(
        shown_result("let r = lazy { xs = [lazy 1, 2] }\nr.xs[0] := 5\nr"),
        "{xs: List<Int | Future<Int>>}"
    );
    // A position is an `Int` once coerced, and so is the element it
    // reads in the value.
    let text = "let mut xs = [1, 2]\nxs[1] := xs[0]";
    assert_eq!(type_at(text, last(text, "1")), "Int");
}

#[test]
fn an_index_path_needs_lists_and_int_positions() {
    // A value that is not a list where an index selects in it, labelled
    // at the variable with its type.
    let text = "let mut x = 1\nx[0] := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot update `x[0]`: `x` is Int, not a list"
    );
    assert_eq!(found[0].span, at(text, "x[0]"));
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "x[0]"), 1), "this is Int")]
    );
    assert!(found[0].notes.is_empty());
    // An index on a record, a name on a list, and either further along.
    for (text, path, message) in [
        (
            "let mut r = { a = 1 }\nr[0] := 2",
            "r[0]",
            "cannot update `r[0]`: `r` is `{a: Int}`, not a list",
        ),
        (
            "let mut xs = [1]\nxs.a := 2",
            "xs.a",
            "cannot update `xs.a`: `xs` is `List<Int>`, not a record",
        ),
        (
            "let mut xs = [1]\nxs[0].a := 2",
            "xs[0].a",
            "cannot update `xs[0].a`: `xs[0]` is Int, not a record",
        ),
        (
            "let mut r = { a = [{ b = 1 }] }\nr.a[0].b[1] := 2",
            "r.a[0].b[1]",
            "cannot update `r.a[0].b[1]`: `r.a[0].b` is Int, not a list",
        ),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, message, "{text}");
        assert_eq!(found[0].span, last(text, path), "{text}");
    }
    // A position that is not an `Int`, as for an index.
    let text = "let mut xs = [1]\nxs[\"a\"] := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "a list index must be an Int, found String"
    );
    assert_eq!(found[0].span, at(text, "\"a\""));
    assert_eq!(found[0].label.as_deref(), Some("this is String"));
    assert!(found[0].labels.is_empty());
    assert_eq!(
        messages("let mut xs = [1]\nxs[1.5] := 2"),
        vec!["a list index must be an Int, found Float"]
    );
    // A value that does not fit the element type.
    let text = "let mut xs = [1]\nxs[0] := \"a\"";
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
        vec![(at(text, "xs[0]"), "`:=` keeps the element's type")]
    );
    // A field missing after an index: no `let` can add it.
    let text = "let mut xs = [{ a = 1 }]\nxs[0].b := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot rebind `xs[0].b`: `xs[0]` has no field `b`"
    );
    assert_eq!(found[0].span, at(text, "xs[0].b"));
    assert_eq!(
        labels(&found[0]),
        vec![(
            first_bytes(at(text, "xs[0].b"), 2),
            "this is List<{a: Int}>"
        )]
    );
    assert!(found[0].notes.is_empty());
    // A position is shown as it is written.
    assert_eq!(
        messages("let mut xs = [1]\nlet i = 0\nxs[i  +  1].a := 2"),
        vec!["cannot update `xs[i  +  1].a`: `xs[i  +  1]` is Int, not a record"]
    );
}

#[test]
fn every_position_is_checked_once() {
    // Where the walk reaches it, or on its own where the walk stops
    // before it, so that its mistakes are reported too.
    for (text, expected) in [
        (
            "let mut x = 1\nx[y] := 2",
            vec![
                "cannot update `x[y]`: `x` is Int, not a list",
                "undefined variable `y`",
            ],
        ),
        (
            "let mut xs = [1]\nxs[0].a[z] := 2",
            vec![
                "cannot update `xs[0].a[z]`: `xs[0]` is Int, not a record",
                "undefined variable `z`",
            ],
        ),
        (
            "let mut r = { a = [1] }\nr.b[z] := 2",
            vec![
                "cannot rebind `r.b[z]`: `r` has no field `b`",
                "undefined variable `z`",
            ],
        ),
        (
            "let mut xs = [[1]]\nxs[\"a\"][z] := 2",
            vec![
                "a list index must be an Int, found String",
                "undefined variable `z`",
            ],
        ),
        (
            "let xs = [1]\n{ xs[y] := 2 }",
            vec![
                "cannot rebind `xs` here: it is declared in an enclosing block",
                "undefined variable `y`",
            ],
        ),
        (
            "ys[y] := 2",
            vec![
                "cannot rebind `ys`: no variable named `ys` is declared",
                "undefined variable `y`",
            ],
        ),
    ] {
        let mut found = messages(text);
        found.sort();
        let mut expected = expected;
        expected.sort_unstable();
        assert_eq!(found, expected, "{text}");
    }
    // Each position gets a type once, and the program lowers.
    let text = "let mut xs = [[1, 2], [3]]\nlet i = 0\nxs[i][i + 1] := xs[1][0]\nxs";
    assert_eq!(shown_result(text), "List<List<Int>>");
}

#[test]
fn an_updated_element_must_fit_its_list() {
    let note = "a list keeps its element type; declare the elements with a type that allows \
                    both";
    // Elements declared exactly a future, which a path through one makes
    // present.
    let text = "let mut xs: Future<{a: Int}>[] = [lazy { a = 1 }]\nxs[0].a := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot rebind `xs[0].a`: the elements of `xs` are `Future<{a: Int}>`, and the update \
             makes one `{a: Int}`"
    );
    assert_eq!(found[0].span, at(text, "xs[0].a"));
    assert!(found[0].labels.is_empty());
    assert_eq!(found[0].notes, vec![note]);
    // For a variable that is not mutable too, and deeper.
    assert_eq!(
        messages("let r = { xs = [lazy { a = 1 }] }\nr.xs[0].a := 2"),
        vec![
            "cannot rebind `r.xs[0].a`: the elements of `r.xs` are `Future<{a: Int}>`, and \
                 the update makes one `{a: Int}`"
        ]
    );
    // Elements of the either shape take the present element.
    let text = "let xs: List<{a: Int} | Future<{a: Int}>> = [lazy { a = 1 }]\n\
                    xs[0].a := 2\nxs";
    assert_eq!(shown_result(text), "List<{a: Int} | Future<{a: Int}>>");
    assert_eq!(awaited_paths(text, None), vec![(2, vec![false, true])]);
    // An element replaced at the leaf is not read, and keeps its type.
    let text = "let xs = [lazy 1]\nxs[0] := lazy 2\nxs";
    assert_eq!(shown_result(text), "List<Future<Int>>");
    assert_eq!(awaited_paths(text, None), vec![(1, Vec::new())]);
    assert_eq!(
        messages("let xs = [lazy 1]\nxs[0] := 2"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_list_that_may_be_a_future_is_awaited_where_it_is_updated() {
    // A variable that is not mutable becomes present.
    let text = "let xs = lazy [1, 2]\nxs[0] := 5\nxs";
    assert_eq!(shown_result(text), "List<Int>");
    assert_eq!(awaited_paths(text, None), vec![(1, vec![true])]);
    // A mutable one keeps its declared type, which must allow the update.
    let text = "let mut xs: Int[] | Future<Int[]> = lazy [1]\n{ xs[0] := 2 }\nxs";
    assert_eq!(shown_result(text), "List<Int> | Future<List<Int>>");
    let text = "let mut xs: Future<Int[]> = lazy [1]\nxs[0] := 2";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot rebind `xs[0]`: `xs` is declared `Future<List<Int>>`, and the update makes \
             it `List<Int>`"
    );
    assert_eq!(found[0].span, at(text, "xs[0]"));
    // In an impure function, a list and its elements of the either
    // shape are awaited, and so is a position.
    let text = "fn f(xs: List<{a: Int}>, i: Int) -> Int { xs[i].a := 1\n0 }";
    assert_eq!(awaited_paths(text, Some(0)), vec![(2, vec![true, true])]);
    assert_eq!(marks(text), vec![("i", AwaitMark::Top)]);
}

#[test]
fn a_pure_function_and_an_index_path() {
    // A bare parameter is measured by the update, and so present.
    let text = "fn f(xs: Int[]): Int[] { xs[0] := 1\nxs }";
    assert_eq!(demands(text, 0), vec![Some("xs")]);
    assert_eq!(type_of_f(text), "fn(xs: List<Int>): List<Int>");
    assert!(marks(text).is_empty());
    // A parameter held in a record, and a position, measured where the
    // update reads them.
    let text = "fn f(xs: Int[], i: Int): Int { let r = { a = xs }\nr.a[i] := 1\n0 }";
    assert_eq!(demands(text, 0), vec![Some("r.a"), Some("i")]);
    assert_eq!(type_of_f(text), "fn(xs: List<Int>, i: Int): Int");
    // A list that may be a future cannot be awaited.
    let text = "fn f(xs: Future<Int[]>): Int { xs[0] := 1\n0 }";
    assert_implicit_await_error(text, first_bytes(at(text, "xs[0]"), 2), "Future<List<Int>>");
    // Nor can an element along the path.
    let text = "fn f(xs: List<{a: Int} | Future<{a: Int}>>): Int { xs[0].a := 1\n0 }";
    assert_implicit_await_error(text, at(text, "xs[0]"), "{a: Int} | Future<{a: Int}>");
    // Nor a position, which is an expression of the function.
    let text = "fn f(xs: Int[], i: Future<Int>): Int[] { xs[i] := 1\nxs }";
    assert_implicit_await_error(text, first_bytes(at(text, "i]"), 1), "Future<Int>");
    // Nor can a position call an impure function.
    let text = "fn g() -> Int { 0 }\nfn f(xs: Int[]): Int[] { xs[g()] := 1\nxs }";
    assert_eq!(
        messages(text),
        vec!["cannot call an impure function from a pure function"]
    );
    // A value that is not a list once awaited is reported alone.
    assert_eq!(
        messages("fn f(x: Future<Int>): Int { x[0] := 1\n0 }"),
        vec!["cannot update `x[0]`: `x` is `Future<Int>`, not a list"]
    );
}

#[test]
fn an_index_path_follows_the_rebind_rules() {
    // A mutable variable, from an inner block, writing its slot.
    let text = "let mut xs = [1]\nif true { xs[0] := 2 }\nxs";
    assert_eq!(shown_result(text), "List<Int>");
    let (ast, resolution) = resolved(text);
    assert_eq!(
        resolution.item_slot(item_at(&ast, text, "xs[0] := 2")),
        Slot(0)
    );
    // One that is not, from an inner block, from a function or a `lazy`:
    // rejected, with the positions and the value still checked.
    let text = "let xs = [1]\n{ xs[0] := 2 }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot rebind `xs` here: it is declared in an enclosing block"
    );
    assert_eq!(found[0].notes, vec![enclosing_note("xs")]);
    for (text, kind) in [
        (
            "let mut xs = [1]\nlet f = fn(): Int { xs[q] := 2.5\n0 }",
            "function",
        ),
        (
            "let mut xs = [1]\nlet f = lazy { xs[q] := 2.5\n0 }",
            "`lazy`",
        ),
    ] {
        let mut found = messages(text);
        found.sort();
        assert_eq!(
            found,
            vec![
                format!("cannot rebind `xs` here: it is declared outside this {kind}"),
                "undefined variable `q`".to_owned()
            ],
            "{text}"
        );
    }
    // A rejected rebind with an index segment declares its name as
    // erroneous, so that later uses do not cascade.
    assert_eq!(
        messages("let xs = [1]\n{ xs[0] := 2\nxs[0] + \"s\" }"),
        vec!["cannot rebind `xs` here: it is declared in an enclosing block"]
    );
    // A position reads and captures variables as any expression does.
    let text = "let mut xs = [1, 2]\nlet i = 1\n\
                    let f = fn(): List<Int> { let mut ys = xs\nys[i] := 5\nys }";
    let (ast, resolution) = resolved(text);
    assert_eq!(uses(&ast, &resolution, "i"), vec![VarRef::Capture(1)]);
    assert_eq!(
        captures(text, 0),
        vec![
            (VarRef::Local(Slot(0)), "List<Int>".to_owned()),
            (VarRef::Local(Slot(1)), "Int".to_owned())
        ]
    );
}

#[test]
fn a_function_returning_an_updated_list() {
    // An impure function's result is inferred from the updated
    // variable: the list present, its elements of the either shape.
    let text = "fn f(xs: Int[]) -> Int[] { xs[0] := 1\nxs }";
    assert_eq!(
        type_of_f(text),
        "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> List<Int | \
             Future<Int>>"
    );
    // A mutable copy keeps the parameter's type, which is the result.
    let text = "fn f(xs: Int[]) -> Int[] { let mut ys = xs\nys[0] := 1\nys }";
    assert_eq!(
        type_of_f(text),
        "fn(xs: List<Int | Future<Int>> | Future<List<Int | Future<Int>>>) -> List<Int | \
             Future<Int>> | Future<List<Int | Future<Int>>>"
    );
    // A pure function's, present throughout.
    let text = "fn f(xs: Int[], x: Int): Int[] { let mut ys = xs\nys[0] := x\nys }";
    assert_eq!(type_of_f(text), "fn(xs: List<Int>, x: Int): List<Int>");
}
