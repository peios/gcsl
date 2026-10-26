//! Tests of the checker: loop forms, `break`, `continue`, labels, `Never` and
//! `return`.

use super::*;

/// The diagnostics of `text`, or none if it checks.
pub(super) fn diagnostics_or_none(text: &str) -> Vec<Diagnostic> {
    check_prelude(text).2.err().unwrap_or_default()
}

/// The type of the file's value as shown, after `xs`, a list of three
/// `Int`s, and `flag`, a `Bool`, are declared.
pub(super) fn loop_type(text: &str) -> String {
    file_type(&format!("let xs = [1, 7, 3]\nlet flag = true\n= {text}"))
}

#[test]
fn the_types_of_the_loop_forms() {
    assert_eq!(loop_type("for flag { 1 }"), "List<Int>");
    assert_eq!(
        loop_type("for let i = 0; i < 4; i := i + 1 { i * i }"),
        "List<Int>"
    );
    assert_eq!(loop_type("for { break }"), "List<{}>");
    assert_eq!(loop_type("do { 2.5 } for flag"), "List<Float>");
    // Find-first, with a final `continue` and without.
    assert_eq!(
        loop_type("for x in xs { if x > 5 { break x }; continue }"),
        "List<Int>"
    );
    assert_eq!(
        loop_type("for x in xs { if x > 5 { break x } }"),
        "List<Int | {}>"
    );
    // A body of type Never contributes nothing.
    assert_eq!(loop_type("for x in xs { break x }"), "List<Int>");
    assert_eq!(loop_type("for x in xs { continue }"), "List<{}>");
    // A labelled value joins its target's element type only.
    assert_eq!(
        loop_type("for'outer x in xs { for y in xs { if y > x { continue'outer 1.5 }; y } }"),
        "List<Float | List<Int>>"
    );
    // A loop whose body is `{}` is a unit-like statement.
    assert_eq!(
        file_type("let mut n = 0\nfor n < 3 { n := n + 1 }"),
        "List<{}>"
    );
    assert!(warnings("let mut n = 0\nfor n < 3 { n := n + 1 }\nn").is_empty());
}

#[test]
fn untyped_values_settle_together_and_expected_types_reach_them() {
    assert_eq!(
        loop_type("for x in xs { if x > 5 { break 1 }; 2.5 }"),
        "List<Float>"
    );
    assert_eq!(
        loop_type("for x in xs { if x > 5 { continue 2.5 }; 1 }"),
        "List<Float>"
    );
    assert_eq!(
        loop_type("for x in xs { if x > 5 { break 1 }; 2 }"),
        "List<Int>"
    );
    // Under an expected type, each value is checked against it.
    assert_eq!(
        file_type("let fs: List<Float> = for x in [1] { if x > 5 { break 1 }; 2 }\n= fs"),
        "List<Float>"
    );
    assert_eq!(
        file_type(
            "let us: List<Int | String> = for x in [1] { if x > 5 { break \"a\" }; x }\n= us"
        ),
        "List<Int | String>"
    );
    let text = "let ns: List<Int> = for x in [1] { if x > 5 { break \"a\" }; x }";
    assert_eq!(messages(text), vec!["mismatched types"]);
    // The Never case takes the expected element type.
    assert_eq!(
        file_type("let ns: List<Int> = for { break }\n= ns"),
        "List<Int>"
    );
}

#[test]
fn a_widened_loop_is_warned_about_at_each_contributing_site() {
    let text = "let xs = [1]\nlet r = for x in xs { if x > 5 { break \"big\" }; x }";
    let warnings = warnings(text);
    assert_eq!(warnings.len(), 1);
    let warning = &warnings[0];
    assert_eq!(
        warning.message,
        "the values this loop collects have different types"
    );
    assert_eq!(warning.span, at(text, "for"));
    assert_eq!(
        labels(warning),
        vec![
            (at(text, "\"big\""), "this is String"),
            (last(text, "x"), "this is Int"),
        ]
    );
    assert_eq!(
        warning.notes,
        [
            "the loop is `List<Int | String>`; annotate it with that type if all are meant, or \
              tell them apart with `is`"
        ]
    );
    // Bare `break` and `continue` contribute nothing, so join nothing.
    assert!(warnings_of("for x in [1] { if x > 5 { break }; x }").is_empty());
}

/// The warnings of a check that must succeed, of `text` as a statement
/// bound to a name.
pub(super) fn warnings_of(text: &str) -> Vec<Diagnostic> {
    warnings(&format!("let v = {text}"))
}

#[test]
fn a_statement_loop_with_a_value_is_an_unused_value() {
    assert_eq!(
        messages("for x in [1] { break 5 }\n1"),
        vec!["unused value of type `List<Int>`"]
    );
    assert!(warnings("for x in [1] { break }\n1").is_empty());
}

#[test]
fn header_variables_keep_their_slot_and_type() {
    // Rebound from the header, keeping its type.
    assert_eq!(
        messages("let v = for let i = 0; i < 3; i := 1.5 { i }"),
        vec!["mismatched types"]
    );
    // From the body, an error whose note says to write `let mut`.
    let text = "let v = for let i = 0; i < 3; i := i + 1 { i := 5; i }";
    let diagnostics = diagnostics(text);
    assert_eq!(diagnostics.len(), 1);
    let error = &diagnostics[0];
    assert_eq!(
        error.message,
        "cannot rebind `i` here: it is declared in the header of a `for`"
    );
    assert_eq!(error.span, nth(text, "i", 4));
    assert_eq!(
        labels(error),
        vec![(at(text, "let i = 0"), "declared here")]
    );
    assert!(error.notes[0].contains("`let mut i = ...` in the header"));
    // A typed `let` in the header adapts its literal as any does.
    assert_eq!(
        file_type("let v = for let i: Float = 0; i < 2.0; i := i + 1.0 { i }\n= v"),
        "List<Float>"
    );
    // A `let mut` in the header may be rebound from the body too.
    assert_eq!(
        file_type("let v = for let mut i = 0; i < 3; i := i + 1 { i := i + 1; i }\n= v"),
        "List<Int>"
    );
    // A `:=` in the header of a variable from outside follows the
    // existing rules.
    assert_eq!(
        file_type("let mut k = 0\nlet v = for k < 3; k := k + 1 { k }\n= v"),
        "List<Int>"
    );
    assert_eq!(
        messages("let k = 0\nlet v = { for k < 3; k := k + 1 { k } }"),
        vec!["cannot rebind `k` here: it is declared in an enclosing block"]
    );
    // Not visible after the loop.
    assert_eq!(
        messages("let v = for let i = 0; i < 3; i := i + 1 { i }\ni"),
        vec!["undefined variable `i`"]
    );
    // From a function in the body, the error for any variable outside
    // a function.
    assert_eq!(
        messages("let v = for let i = 0; i < 3; i := i + 1 { let f = fn(): Int { i := 1; i }; i }"),
        vec!["cannot rebind `i` here: it is declared in an enclosing block"]
    );
}

#[test]
fn a_header_variable_is_one_slot_for_life() {
    let (ast, resolution) = resolved("let v = for let i = 0; i < 3; i := i + 1 { i }");
    // The `let` and the `:=` write the same slot, which `v` takes once
    // the loop has left it.
    let slots: Vec<Slot> = (0..ast.item_count())
        .map(|item| resolution.item_slot(ItemId::new(u32::try_from(item).unwrap())))
        .filter(|&slot| slot != Slot::NONE)
        .collect();
    assert_eq!(slots, vec![Slot(0), Slot(0), Slot(0)]);
}

#[test]
fn conditions_are_measured_bools_and_narrow_nothing() {
    assert_eq!(messages("let v = for 1 { 2 }"), vec!["mismatched types"]);
    assert_eq!(
        messages("let v = do { 2 } for \"no\""),
        vec!["mismatched types"]
    );
    // `for x is Int` leaves `x` as it was.
    assert_eq!(
        messages("let x: Int | String = 1\nlet v = for x is Int { x + 1 }"),
        vec!["cannot add `Int | String` and Int"]
    );
    // A `do`'s condition cannot see the body's variables.
    assert_eq!(
        messages("let v = do { let t = true; 1 } for t"),
        vec!["undefined variable `t`"]
    );
    // In a pure function, a condition that may be a future is awaited,
    // which is the purity error; in an impure one it is fine.
    let pure = "fn f(c: Future<Bool>): List<Int> { for c { 1 } }";
    assert_eq!(
        messages(pure),
        vec!["cannot use a value that may be a future in a pure function"]
    );
    let impure = "fn f(c: Future<Bool>) -> List<Int> { for await c { break 1 } }\n= f(lazy true)";
    assert_eq!(file_type(impure), "List<Int>");
    // Loops are allowed in pure functions.
    assert_eq!(
        shown_result("fn f(n: Int): List<Int> { for let i = 0; i < n; i := i + 1 { i } }\nf"),
        "fn(n: Int): List<Int>"
    );
}

#[test]
fn break_and_continue_errors() {
    // No loop at all.
    let text = "break";
    assert_eq!(
        errors(text),
        vec![("`break` outside a loop".to_owned(), at(text, "break"))]
    );
    assert_eq!(messages("continue 1"), vec!["`continue` outside a loop"]);
    // In a guard, a condition or a header item, a `break` belongs to an
    // outer loop, if there is one.
    assert_eq!(
        messages("let v = for x in [1] if { break } { x }"),
        vec!["`break` outside a loop"]
    );
    assert_eq!(
        messages("let v = for ({ break }) { 1 }"),
        vec!["`break` outside a loop"]
    );
    assert_eq!(
        messages("let v = for let i = ({ break }); true; i := ({ continue }) { 1 }"),
        vec!["`break` outside a loop", "`continue` outside a loop"]
    );
    assert_eq!(
        messages("let v = do { 1 } for { continue }"),
        vec!["`continue` outside a loop"]
    );
    assert_eq!(
        file_type("let v = for x in [1] { for y in [2] if { continue } { y } }\n= v"),
        "List<List<Int>>"
    );
    // Across a function and a `lazy`.
    let text = "let v = for x in [1] { let f = fn(): Int { break; 1 }; x }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "`break` cannot leave a function");
    assert_eq!(diagnostic.span, at(text, "break"));
    assert_eq!(
        labels(diagnostic),
        vec![(at(text, "fn"), "inside a function here")]
    );
    assert_eq!(
        diagnostic.notes,
        [
            "the body of a function runs when it is called, which may be after the loop has \
              ended"
        ]
    );
    let text = "let v = for x in [1] { let l = lazy { continue 2 }; x }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "`continue` cannot leave a `lazy`");
    assert_eq!(diagnostic.span, at(text, "continue"));
    assert_eq!(
        labels(diagnostic),
        vec![(at(text, "lazy"), "inside a `lazy` here")]
    );
    assert_eq!(
        diagnostic.notes,
        [
            "the body of a `lazy` runs when it is awaited, which may be after the loop has \
              ended"
        ]
    );
    // As an item of a record block.
    let text = "let v = for x in [1] { { a = 1; break } }";
    assert_eq!(
        errors(text),
        vec![(
            "`break` cannot be an item of a record block".to_owned(),
            at(text, "break")
        )]
    );
    assert_eq!(
        messages("let v = for x in [1] { { continue'a; a = 1 } }"),
        vec!["`continue` cannot be an item of a record block"]
    );
    // The fields after it are checked, and nothing else is reported.
    assert_eq!(
        messages("let v = for x in [1] { { a = 1; break 2; b = \"x\" + 1 } }"),
        vec![
            "`break` cannot be an item of a record block",
            "cannot add String and Int"
        ]
    );
}

#[test]
fn labels_resolve_to_the_loop_whose_body_holds_them() {
    let text = "let v = for'a x in [1] { for'b y in [2] { break'a } }";
    let (ast, resolution) = resolved(text);
    let jump = (0..ast.item_count())
        .map(|item| ItemId::new(u32::try_from(item).unwrap()))
        .find_map(|item| resolution.jump_target(item))
        .unwrap();
    assert_eq!(jump, ForId::new(1));
    // Unknown.
    let text = "let v = for'a x in [1] { break'b }";
    assert_eq!(
        errors(text),
        vec![(
            "no enclosing loop is labelled `'b`".to_owned(),
            at(text, "'b")
        )]
    );
    // A label in the loop's own header is not in its body.
    assert_eq!(
        messages("let v = for'a x in [1] if { break'a } { x }"),
        vec!["no enclosing loop is labelled `'a`"]
    );
    // Reused inside its own loop's body, labelled at the outer one.
    let text = "let v = for'a x in [1] { for'a y in [2] { y } }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "label `'a` is already in use");
    assert_eq!(diagnostic.span, nth(text, "'a", 1));
    assert_eq!(
        labels(diagnostic),
        vec![(at(text, "'a"), "the loop around this one is labelled so")]
    );
    // Reused after the loop, or in a function nested in it, is fine.
    assert!(warnings("let v = for'a x in [1] { x }\nlet w = for'a y in [2] { y }").is_empty());
    assert!(
        warnings("let v = for'a x in [1] { let f = fn(): List<Int> { for'a y in [2] { y } }; x }")
            .is_empty()
    );
    // A label outside the function is "cannot leave", labelling the loop.
    let text = "let v = for'a x in [1] { let f = fn(): Int { for y in [2] { break'a }; 1 }; x }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "`break` cannot leave a function");
    assert_eq!(diagnostic.span, at(text, "break'a"));
    assert_eq!(
        labels(diagnostic),
        vec![
            (at(text, "fn"), "inside a function here"),
            (at(text, "'a"), "the loop it names is outside it"),
        ]
    );
    // An unused label is fine.
    assert!(warnings("let v = for'unused x in [1] { x }").is_empty());
}

#[test]
fn never_is_the_type_of_what_never_completes() {
    // The type of the last item of the body of a loop, given as `body`.
    let in_loop = |body: &str| {
        let (ast, types, output) = check_prelude(&format!("let v = for x in [1] {{ {body} }}"));
        let output = output.unwrap_or_else(|errors| panic!("{body}: {errors:?}"));
        let ExprKind::For(id) = ast.get(result_expr_of_let(&ast)).kind else {
            panic!("{body}: expected a loop");
        };
        let ExprKind::Block { items, .. } = ast.get(ast.for_decl(id).body).kind else {
            panic!("{body}: expected a block");
        };
        let last = *ast.items(items).last().unwrap();
        let value = ast.item(last).kind.value().unwrap();
        types
            .display(output.checked.types.get(value), ast.names())
            .to_string()
    };
    assert_eq!(in_loop("{ break }"), "Never");
    assert_eq!(in_loop("if x > 1 { break } else { continue }"), "Never");
    assert_eq!(in_loop("if x > 1 { break } else { 5 }"), "Int");
    assert_eq!(in_loop("if x > 1 { break }"), "{}");
    assert_eq!(in_loop("{ { break } }"), "Never");
    assert_eq!(in_loop("{ break } + 1"), "Never");
    assert_eq!(in_loop("1 + { break }"), "Never");
    assert_eq!(in_loop("x > 1 and { break }"), "Bool");
    assert_eq!(in_loop("{ break } and true"), "Never");
    assert_eq!(in_loop("{ break } == 1"), "Never");
    assert_eq!(in_loop("-{ break }"), "Never");
    assert_eq!(in_loop("{ break }.a"), "Never");
    assert_eq!(in_loop("{ break }[0]"), "Never");
    assert_eq!(in_loop("[1][{ break }]"), "Never");
    assert_eq!(in_loop("{ break }(1)"), "Never");
    assert_eq!(in_loop("len({ break })"), "Never");
    assert_eq!(in_loop("\"{ { break } }\""), "Never");
    assert_eq!(in_loop("{ break } is Int"), "Never");
    assert_eq!(in_loop("{ break } as Int"), "Never");
    assert_eq!(in_loop("await { break }"), "Never");
    assert_eq!(in_loop("if { break } { 1 } else { 2 }"), "Never");
    assert_eq!(in_loop("for y in { break } { y }"), "Never");
    // A condition runs before the first iteration; a `do`'s, after.
    assert_eq!(in_loop("for ({ break }) { 1 }"), "Never");
    assert_eq!(in_loop("do { 1 } for ({ break })"), "List<Int>");
    // A guard may never run.
    assert_eq!(in_loop("for y in [1] if { break } { y }"), "List<Int>");
    assert_eq!(in_loop("[{ break }]"), "List<Never>");
    assert_eq!(in_loop("{ a = { break } }"), "{a: Never}");
    assert_eq!(in_loop("({ break } : Int)"), "Int");
    assert_eq!(in_loop("let r = { break }; r"), "Never");
}

/// The value of the file's first `let`.
pub(super) fn result_expr_of_let(ast: &Ast) -> ExprId {
    let first = *ast.items(ast.root()).first().unwrap();
    ast.item(first).kind.value().unwrap()
}

#[test]
fn never_is_a_subtype_of_everything_and_joins_as_nothing() {
    let mut table = TypeTable::new();
    let never = table.never();
    assert_eq!(table.never(), never);
    let list = table.list(TyId::INT);
    for ty in [TyId::INT, TyId::STRING, TyId::EMPTY, list] {
        assert!(table.is_subtype(never, ty));
        assert!(!table.is_subtype(ty, never));
        assert_eq!(table.join(never, ty), (ty, false));
        assert_eq!(table.join(ty, never), (ty, false));
        assert_eq!(table.union([never, ty]), ty);
    }
    assert_eq!(table.union([never]), never);
    assert!(table.is_unit_like(never));
    assert_eq!(table.strip(never), never);
    assert_eq!(table.present(never), never);
    assert_eq!(table.laziness(never), Laziness::Present);
    assert!(!table.contains_future(never));
    assert!(!table.contains_fn(never));
    assert_eq!(table.value_kind(never), Some(ValueKind::Never));
    assert_eq!(table.display(never, &Names::new()).to_string(), "Never");
}

#[test]
fn unreachable_code_is_never_an_error_because_of_never() {
    for body in [
        "{ break } + 1",
        "f({ continue })",
        "[{ break }]",
        "{ break }.a",
        "{ break }[0]",
        "{ break }(1)",
        "not { break }",
        "if { break } { 1 } else { 2 }",
        "for y in { break } { y }",
        "let r = { break }; r.a",
        "\"{ { break } }\"",
        "{ a = { break } }",
        "{ break } is Int",
        "await { break }",
        "len({ break })",
        "[...{ break }]",
        "{ ...{ break } }",
        "let mut r = { break }; r := 1; r.a := 2; let r.b = 3; r",
        "let r = { a = { break } }; r.a.b := 1; let r.a.c = 2; r",
        "if (r is Int) { r } else { 0 }",
    ] {
        let text = format!(
            "fn f(n: Int): Int {{ n }}\nlet r: Int | String = 1\nlet v = for x in [1] {{ {body} }}"
        );
        let found = diagnostics_or_none(&text);
        assert!(
            found.iter().all(|d| d.severity == Severity::Warning),
            "{body}: {found:?}"
        );
    }
}

#[test]
fn a_top_level_field_computed_early_is_not_code_that_is_never_run() {
    // Computed at its own line, a field after an item that never
    // completes is warned about, as any item is.
    let text = "let x = for { }\ny = 1";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].span, at(text, "y = 1"));
    // Computed before an earlier item, it is neither warned about nor
    // counted: the warning starts at the next item that runs there.
    let text = "let a = y\nlet x = for { }\ny = 1\nlet z = 2";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].span, at(text, "let z = 2"));
    assert_eq!(warnings("let a = y\nlet x = for { }\ny = 1"), vec![]);
}

#[test]
fn a_top_level_function_is_not_code_that_is_never_run() {
    // Its closure may have been made already, so the warning neither
    // starts at a top-level function nor counts it: it is at the first
    // other item.
    let text = "let x = for { }\nfn f(): Int { 1 }\ndef g(): Int { 2 }\nlet y = f()";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "this is never run");
    assert_eq!(found[0].span, at(text, "let y = f()"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "let x = for { }"), "this always leaves the block")]
    );
    // Nothing after it but functions: no warning.
    assert_eq!(warnings("let x = for { }\nfn f(): Int { 1 }"), vec![]);
    // In an inner block a named function is an item as any other.
    let text = "x = { let a = for { }; fn f(): Int { 1 }; f() }";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].span, at(text, "fn f(): Int { 1 }"));
}

#[test]
fn the_first_item_after_one_that_never_completes_is_warned_about() {
    let text = "let v = for x in [1] { break; let y = x + 1; y }";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "this is never run");
    assert_eq!(found[0].span, at(text, "let y = x + 1"));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "break"), "this always leaves the block")]
    );
    // After an expression, a `let` or a `:=` of type Never too, once
    // per block, the block's type being its last item's.
    for body in [
        "{ continue }; 1",
        "let y = { break }; y",
        "let mut z = 0; z := { break }; z",
    ] {
        let found = warnings(&format!("let v = for x in [1] {{ {body} }}"));
        assert_eq!(found.len(), 1, "{body}: {found:?}");
        assert_eq!(found[0].message, "this is never run");
    }
    // Nested blocks warn each for itself; nothing follows the last item.
    assert!(warnings("let v = for x in [1] { if x > 1 { break }; x }").is_empty());
    assert_eq!(
        file_type("let v = for x in [1] { break; 1 }\n= v"),
        "List<Int>"
    );
    // A `break` that is an error leaves nothing, so nothing after it is
    // warned about.
    let found = errors("break\nlet y = 1");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "`break` outside a loop");
}

#[test]
fn closures_made_in_a_header_loop_capture_each_iteration() {
    let text = "let fs = for let i = 0; i < 3; i := i + 1 { fn(): Int { i } }\n= fs";
    assert_eq!(file_type(text), "List<fn(): Int>");
}

#[test]
fn result_inference_sees_each_loop_form() {
    // An impure function's bare result takes what the body returns, a
    // list of what its loop collects.
    for (body, ty) in [
        ("for x in [1, 2] { x }", "fn() -> List<Int>"),
        (
            "for let i = 0; i < 2; i := i + 1 { i }",
            "fn() -> List<Int>",
        ),
        ("do { lazy 1 } for false", "fn() -> List<Future<Int>>"),
        (
            "for x in [1] { if x > 0 { break lazy x }; 1 }",
            "fn() -> List<Int | Future<Int>>",
        ),
        // Nothing is collected: the element type is the expected one.
        ("for { break }", "fn() -> List<Int | Future<Int>>"),
    ] {
        let text = format!("fn f() -> List<Int> {{ {body} }}\nf");
        assert_eq!(shown_result(&text), ty, "{body}");
    }
}

#[test]
fn each_target_is_typed_by_its_result_sites() {
    // A function: its declared type.
    assert_eq!(
        shown_result(
            "fn sign(n: Int): Int { if n < 0 { return -1 }; if n > 0 { return 1 }; 0 }\nsign"
        ),
        "fn(n: Int): Int"
    );
    // A `lazy`: the join of its operand and what it returns.
    assert_eq!(
        loop_type("lazy { if flag { return 1.5 }; 2 }"),
        "Future<Float>"
    );
    assert_eq!(loop_type("lazy { return 1 }"), "Future<Int>");
    // A labelled block: the join of its result and what it returns.
    assert_eq!(
        loop_type("{'pick if flag { return'pick 1 }; if xs[0] > 1 { return'pick 2 }; 3 }"),
        "Int"
    );
    assert_eq!(loop_type("{'b return'b 2.5 }"), "Float");
    assert_eq!(loop_type("{'b if flag { return'b 1 }; 2.5 }"), "Float");
    // A loop: the join of its list and what it returns.
    assert_eq!(
        loop_type("for'scan x in xs { if x > 5 { return'scan [x] }; x }"),
        "List<Int>"
    );
    assert_eq!(
        loop_type("for'scan x in xs { if x > 5 { return'scan x }; continue }"),
        "Int | List<{}>"
    );
    // A bare `return` returns `{}`.
    assert_eq!(loop_type("{'b if flag { return'b }; {} }"), "{}");
    // A labelled loop body is a labelled block: what it returns is what
    // the iteration contributes.
    assert_eq!(
        loop_type("for x in xs {'it if x < 5 { return'it 0 }; x }"),
        "List<Int>"
    );
    // The summary's examples, end to end.
    let text = "let a = true\nlet b = false\n\
                    let y = {'pick if a { return'pick 1 }; if b { return'pick 2 }; 3 }\n\
                    let v: Int = for'spin { let r = 4; if r > 0 { return'spin r } }\n= y + v";
    assert_eq!(file_type(text), "Int");
    assert!(warnings(text).is_empty());
}

#[test]
fn a_function_return_is_checked_against_the_result_type() {
    for marker in [":", "->>", "->"] {
        let text = format!("fn g(n: Int){marker} Int {{ if n > 0 {{ return \"a\" }}; 0 }}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{marker}: {found:?}");
        assert_eq!(found[0].message, "mismatched types");
        assert_eq!(found[0].span, at(&text, "\"a\""));
        let expected = if marker == ":" {
            "expected Int, found String"
        } else {
            "expected Int | Future<Int>, found String"
        };
        assert_eq!(found[0].label.as_deref(), Some(expected), "{marker}");
        assert_eq!(
            labels(&found[0]),
            vec![(last(&text, "Int"), "the return type is declared here")]
        );
    }
    // An untyped literal adapts, a record block is checked field by
    // field, and a list literal against a list type.
    assert_eq!(
        shown_result("fn f(c: Bool): Float { if c { return 1 }; 2.5 }\nf"),
        "fn(c: Bool): Float"
    );
    assert!(
        diagnostics_or_none("fn f(c: Bool): {a: Float} { if c { return { a = 1 } }; { a = 2.5 } }")
            .is_empty()
    );
    assert!(
        diagnostics_or_none("fn f(c: Bool): List<Float> { if c { return [1, 2] }; [] }").is_empty()
    );
    assert_eq!(
        messages("fn f(c: Bool): {a: Int} { if c { return { b = 1 } }; { a = 1 } }"),
        vec![
            "field `b` is not in `{a: Int}`",
            "missing field `a` in record"
        ]
    );
    // A bare `return` returns `{}`, which an `Int` result is not.
    let text = "fn f(): Int { return }";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, at(text, "return"));
    assert_eq!(found[0].label.as_deref(), Some("expected Int, found {}"));
    assert_eq!(found[0].notes, ["a `return` without a value returns `{}`"]);
    // It is fine where `{}`, or its either shape, is the result.
    assert_eq!(
        shown_result("fn f(c: Bool) -> {} { if c { return }; {} }\nf"),
        "fn(c: Bool | Future<Bool>) -> {}"
    );
    // A body that ends in `return` is `Never`, and fits any result.
    assert_eq!(
        shown_result("fn f(): String { return \"x\" }\nf"),
        "fn(): String"
    );
    // A value that fits once awaited is awaited, where the function may
    // await.
    let (ast, resolution) = resolved("fn f(x: Future<Int>) -> Present<Int> { return x }");
    let x = ast
        .expr_ids()
        .filter(|&id| matches!(ast.get(id).kind, ExprKind::Name(_)))
        .last()
        .unwrap();
    assert_eq!(resolution.await_of(x), AwaitMark::Top);
}

#[test]
fn result_inference_decides_from_every_site() {
    for (body, ty) in [
        // Present, future and either, with the body's own result.
        ("if c { return 1 }; 2", "-> Int"),
        ("if c { return lazy 1 }; lazy 2", "-> Future<Int>"),
        ("if c { return 1 }; lazy 2", "-> Int | Future<Int>"),
        // With only `return`s.
        ("if c { return 1 }; return 2", "-> Int"),
        ("if c { return lazy 1 }; return lazy 2", "-> Future<Int>"),
        ("if c { return lazy 1 }; return 2", "-> Int | Future<Int>"),
    ] {
        let text = format!("fn f(c: Bool) -> Int {{ {body} }}\nf");
        assert_eq!(
            shown_result(&text),
            format!("fn(c: Bool | Future<Bool>) {ty}"),
            "{body}"
        );
    }
    // Memoised functions alike.
    assert_eq!(
        shown_result("fn f(c: Bool) ->> Int { if c { return lazy 1 }; lazy 2 }\nf"),
        "fn(c: Bool) ->> Future<Int>"
    );
    // A record's fields are inferred site by site, dotted ones too.
    assert_eq!(
        shown_result(
            "fn f(c: Bool) -> {a: {b: Int}} { if c { return { a.b = 1 } }; { a = { b = 2 } } }\nf"
        ),
        "fn(c: Bool | Future<Bool>) -> {a: {b: Int}}"
    );
    assert_eq!(
        shown_result("fn f(c: Bool) -> {a: Int} { if c { return { a = lazy 1 } }; { a = 2 } }\nf"),
        "fn(c: Bool | Future<Bool>) -> {a: Int | Future<Int>}"
    );
    // A `return` in a `lazy` is no site of the function around it.
    assert_eq!(
        shown_result("fn f() -> Int { let j = lazy { return 1 }; 2 }\nf"),
        "fn() -> Int"
    );
}

#[test]
fn a_pure_function_measures_what_it_returns_as_its_result() {
    // Returned where an `Int` is expected, the parameter is measured,
    // as returning it as the body's result does.
    assert_eq!(
        shown_result("fn id(x: Int): Int { return x }\nid"),
        shown_result("fn id(x: Int): Int { x }\nid")
    );
    assert_eq!(
        shown_result("fn id(x: Int): Int { return x }\nid"),
        "fn(x: Int): Int"
    );
    // Returned where the either shape is expected, it is not.
    assert_eq!(
        shown_result("fn keep(x: Int): {v: Int | Future<Int>} { return { v = x } }\nkeep"),
        "fn(x: Int | Future<Int>): {v: Int | Future<Int>}"
    );
    // A callback returned is used as a value, so not narrowed; one only
    // called is.
    assert_eq!(
        shown_result("fn run(f: fn(Int) -> Int) -> Int { return f(1) }\nrun"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn pass(f: fn(Int) -> Int) -> fn(Int) -> Int { return f }\npass"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> fn(Int | Future<Int>) -> Int | \
             Future<Int>"
    );
}

#[test]
fn return_errors() {
    // At the top level.
    let text = "let x = 1\nreturn x";
    assert_eq!(
        errors(text),
        vec![("`return` outside a function".to_owned(), at(text, "return"))]
    );
    assert_eq!(
        messages("let v = for x in [1] { if x > 0 { return 1 }; x }"),
        vec!["`return` outside a function"]
    );
    // As an item of a record block.
    let text = "fn f(): {a: Int} { { a = 1; return } }";
    assert_eq!(
        errors(text),
        vec![(
            "`return` cannot be an item of a record block".to_owned(),
            at(text, "return")
        )]
    );
    // An unknown label.
    let text = "fn f(): Int {'a return'b 1 }";
    assert_eq!(
        errors(text),
        vec![(
            "no enclosing block or loop is labelled `'b`".to_owned(),
            at(text, "'b")
        )]
    );
    // A label only outside the function or `lazy`: block and loop.
    for (text, kind, noun) in [
        (
            "let v = {'o let f = fn(): Int { return'o 1 }; 2 }",
            "a function",
            "block",
        ),
        (
            "let v = for'o x in [1] { let f = fn(): Int { return'o 1 }; x }",
            "a function",
            "loop",
        ),
        (
            "let v = {'o let l = lazy { return'o 1 }; 2 }",
            "a `lazy`",
            "block",
        ),
        (
            "let v = for'o x in [1] { let l = lazy { return'o 1 }; x }",
            "a `lazy`",
            "loop",
        ),
    ] {
        let found = &diagnostics(text)[0];
        assert_eq!(
            found.message,
            format!("`return` cannot leave {kind}"),
            "{text}"
        );
        assert_eq!(found.span, at(text, "return'o"), "{text}");
        assert_eq!(
            labels(found)[1],
            (
                at(text, "'o"),
                format!("the {noun} it names is outside it").as_str()
            ),
            "{text}"
        );
    }
    // A `break` or `continue` that names a block.
    let text = "let v = {'blk for x in [1] { break'blk } }";
    let found = &diagnostics(text)[0];
    assert_eq!(found.message, "`'blk` labels a block, not a loop");
    assert_eq!(found.span, at(text, "break'blk"));
    assert_eq!(
        labels(found),
        vec![(at(text, "'blk"), "the block is labelled here")]
    );
    assert_eq!(found.notes, ["use `return'blk` to leave a block"]);
    // An unlabelled one goes through the block to the loop.
    assert_eq!(
        loop_type("for x in xs {'b if x > 1 { break }; x }"),
        "List<Int>"
    );
    // A `return'L` in a loop's iterable, guard or header is not for it.
    assert_eq!(
        messages("fn f(): Int { let v = for'a x in [1] if { return'a true } { x }; 1 }"),
        vec!["no enclosing block or loop is labelled `'a`"]
    );
    // A `return` in error leaves nothing to warn about after it.
    let found = errors("return\nlet y = 1");
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn labels_are_one_namespace() {
    for (text, outer_noun, inner_noun) in [
        ("let v = {'a for'a x in [1] { x } }", "block", "loop"),
        ("let v = for'a x in [1] {'a x }", "loop", "block"),
        ("let v = {'a {'a 1 } }", "block", "block"),
    ] {
        let found = &diagnostics(text)[0];
        assert_eq!(found.message, "label `'a` is already in use", "{text}");
        assert_eq!(found.span, last(text, "'a"), "{text}");
        assert_eq!(
            labels(found)[0].1,
            format!("the {outer_noun} around this one is labelled so")
        );
        assert_eq!(
            found.notes,
            [format!("give the inner {inner_noun} another label")]
        );
    }
    // A function nested in a labelled block starts with no labels, and
    // its labels end with it.
    assert!(
        diagnostics_or_none("let v = {'a let f = fn(): Int {'a return'a 1 }; 2 }\n= v").is_empty()
    );
    // A return names the innermost of its label.
    let text = "fn f(): Int {'a let v = for x in [1] {'b if x > 0 { return'a 3 }; x }; 1 }";
    let (ast, resolution) = resolved(text);
    let target = (0..ast.item_count())
        .map(|item| ItemId::new(u32::try_from(item).unwrap()))
        .find_map(|item| {
            resolution
                .target(item)
                .filter(|_| matches!(ast.item(item).kind, ItemKind::Return(_)))
        })
        .unwrap();
    let JumpTarget::Block(block) = target else {
        panic!("expected a block, got {target:?}");
    };
    assert_eq!(ast.get(block).span.start, at(text, "{'a").start);
    assert!(resolution.is_returned_block(block));
}

#[test]
fn a_lazy_returns_to_its_future() {
    assert_eq!(
        loop_type("lazy { if flag { return xs[0] }; 2 }"),
        "Future<Int>"
    );
    // A widened join is a warning, labelled at each site.
    let text = "let c = true\nlet l = lazy { if c { return 1.5 }; \"s\" }";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "the values this `lazy` returns have different types"
    );
    assert_eq!(found[0].span, at(text, "lazy"));
    assert_eq!(
        labels(&found[0]),
        vec![
            (at(text, "\"s\""), "this is String"),
            (at(text, "1.5"), "this is Float"),
        ]
    );
    assert_eq!(
        found[0].notes,
        [
            "the `lazy` is `Future<Float | String>`; annotate it with that type if all are \
              meant, or tell them apart with `is`"
        ]
    );
    // Under an expected type, the future as a whole fits it, as before.
    assert_eq!(
        file_type("let l: Future<Int | String> = lazy { if true { return 1 }; \"s\" }\n= 1"),
        "Int"
    );
    assert_eq!(
        messages("let l: Future<String> = lazy { if true { return 1 }; \"s\" }"),
        vec![
            "mismatched types",
            "the values this `lazy` returns have different types"
        ]
    );
    // In a function, the `return` leaves the `lazy`, not the function.
    assert_eq!(
        shown_result("fn f() -> String { let j = lazy { return 1 }; \"x\" }\nf"),
        "fn() -> String"
    );
    // A `lazy` inside a labelled block cannot return to the block.
    assert_eq!(
        messages("let v = {'b let l = lazy { return'b 1 }; 2 }"),
        vec!["`return` cannot leave a `lazy`"]
    );
}

#[test]
fn a_labelled_block_joins_its_result_and_its_returns() {
    // Untyped values stay untyped together, and adapt to a type
    // expected later.
    assert_eq!(
        file_type("let f: Float = {'a if true { return'a 1 }; 2 }\n= f"),
        "Float"
    );
    assert_eq!(loop_type("{'a if flag { return'a 1 }; 2 }"), "Int");
    // A widened join is a warning at the label, labelled at each site.
    let text = "let c = true\nlet y = {'b if c { return'b 1 }; \"two\" }";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "the values this block returns have different types"
    );
    assert_eq!(found[0].span, at(text, "'b"));
    assert_eq!(
        labels(&found[0]),
        vec![
            (at(text, "\"two\""), "this is String"),
            (last(text, "1"), "this is Int"),
        ]
    );
    assert_eq!(
        found[0].notes,
        [
            "the block is `Int | String`; annotate it with that type if all are meant, or tell \
              them apart with `is`"
        ]
    );
    // Under an expected union each fits it, and there is no warning.
    let text = "let y: Int | String = {'b if true { return'b 1 }; \"two\" }\n= y";
    assert_eq!(file_type(text), "Int | String");
    assert!(warnings(text).is_empty());
    assert_eq!(
        messages("let y: Int | String = {'b if true { return'b 1.5 }; \"two\" }"),
        vec!["mismatched types"]
    );
    // Never when every path returns, and its values' type then.
    assert_eq!(loop_type("{'b return'b 1 }"), "Int");
    assert_eq!(
        loop_type("{'b if flag { return'b \"y\" } else { return'b \"n\" } }"),
        "String"
    );
    assert_eq!(
        file_type("let v = for x in [1] { {'b if x > 0 { break } else { continue } } }\n= v"),
        "List<{}>"
    );
    // What follows an `if` every branch of which returns is never run.
    let text = "let v = {'b if true { return'b 1 } else { return'b 2 }; 3 }";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "this is never run");
    assert_eq!(found[0].span, last(text, "3"));
    // Nested labels: each `return` names its own.
    assert_eq!(
        loop_type("{'o let i = {'i if flag { return'o 2.5 }; 1 }; 3.5 }"),
        "Float"
    );
    // An unused label is fine.
    assert_eq!(loop_type("{'unused 1 }"), "Int");
}

#[test]
fn a_loop_that_returns_is_its_list_or_what_it_returns() {
    let text = "let xs = [1, 7]\nlet hit = for'scan x in xs { if x > 5 { return'scan x } }";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "this loop is its list or a value returned to it, of different types"
    );
    assert_eq!(found[0].span, at(text, "for"));
    assert_eq!(found[0].label.as_deref(), Some("its list is List<{}>"));
    assert_eq!(labels(&found[0]), vec![(last(text, "x"), "this is Int")]);
    assert_eq!(
        found[0].notes,
        [
            "the loop is `Int | List<{}>`; annotate it with that type if all are meant, or tell \
              them apart with `is`"
        ]
    );
    // The union is told apart with `is`.
    assert_eq!(
        file_type(&format!("{text}\n= if hit is Int {{ hit }} else {{ 0 }}")),
        "Int"
    );
    // Under an expected type: a union covers it, with no warning.
    let annotated = "let xs = [1, 7]\n\
                         let hit: Int | List<{}> = for'scan x in xs { if x > 5 { return'scan x } }";
    assert!(warnings(annotated).is_empty());
    // A list type: the body against its element type, the values
    // against the whole.
    assert_eq!(
        file_type("let v: List<Float> = for'l x in [1] { if x > 5 { return'l [2] }; 3 }\n= v"),
        "List<Float>"
    );
    // A union with one list member: the body against its element type.
    assert_eq!(
        file_type(
            "let v: Float | List<Float> = for'l x in [1] { if x > 5 { return'l 2 }; 3 }\n= v"
        ),
        "Float | List<Float>"
    );
    // A type the list does not fit: the loop's own error.
    let text = "let v: Int = for'L x in [1, 2] { if x > 1 { return'L x } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "this loop may end as a list, `List<{}>`, which is not Int"
    );
    assert_eq!(found[0].span, at(text, "for"));
    assert_eq!(
        found[0].notes,
        ["annotate it as `Int | List<{}>`, or return from every way the loop can end"]
    );
    // A value returned that does not fit is a mismatch at it.
    assert_eq!(
        messages("let v: List<Int> = for'l x in [1] { if x > 5 { return'l \"a\" }; x }"),
        vec!["mismatched types"]
    );
    // A loop no `return` leaves keeps the generic mismatch.
    assert_eq!(
        messages("let v: Int = for x in [1] { x }"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_loop_with_no_condition_and_no_break_never_ends() {
    assert_eq!(
        file_type("let v: Int = for'spin { let r = 4; if r > 0 { return'spin r } }\n= v"),
        "Int"
    );
    assert!(warnings("let v: Int = for'spin { if true { return'spin 1 } }").is_empty());
    assert_eq!(loop_type("for'spin { return'spin \"s\" }"), "String");
    // With a `break`, its own or a labelled one from a loop inside it,
    // it ends; one aimed further out does not end it.
    assert_eq!(loop_type("for { break }"), "List<{}>");
    assert_eq!(
        loop_type("for'o { for x in xs { break'o } }"),
        "List<List<{}>>"
    );
    // The inner loop is `Never`, a body that contributes nothing.
    assert_eq!(loop_type("for'o x in xs { for { break'o } }"), "List<{}>");
    // Numbered children first: the inner loop is 0.
    let (_, resolution) = resolved("let v = for'o { for { break'o } }");
    assert!(resolution.loop_of(ForId::new(0)).endless);
    assert!(!resolution.loop_of(ForId::new(1)).endless);
    // `for true` and a `do` loop are not the form.
    assert_eq!(loop_type("for'l true { return'l 1 }"), "Int | List<{}>");
    // As a statement, what follows it is never run.
    let text = "fn work() -> {} { {} }\nfn serve() -> {} {\n  for { work() }\n  work()\n}";
    let found = warnings(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "this is never run");
    assert_eq!(found[0].span, last(text, "work()"));
    // A function whose only result site is such a loop: honestly
    // present.
    assert_eq!(
        shown_result("fn work() -> {} { {} }\nfn serve() -> {} { for { work() } }\nserve"),
        "fn() -> {}"
    );
    assert_eq!(
        shown_result("fn spin(c: Bool) -> Int { for'l { if c { return'l 5 } } }\nspin"),
        "fn(c: Bool | Future<Bool>) -> Int"
    );
    let (_, resolution) = resolved("for { }");
    assert!(resolution.loop_of(ForId::new(0)).endless);
    let (_, resolution) = resolved("for { break }");
    assert!(!resolution.loop_of(ForId::new(0)).endless);
}

#[test]
fn a_union_of_unit_like_members_is_unit_like() {
    let mut types = TypeTable::new();
    let empties = types.list(TyId::EMPTY);
    let union = types.union([empties, TyId::EMPTY]);
    assert!(types.is_unit_like(union));
    let union = types.union([empties, TyId::INT]);
    assert!(!types.is_unit_like(union));
    // So a statement loop a bare `return` leaves is no unused value.
    let text = "fn f() -> {} { for'l x in [1] { if x > 0 { return'l } }; {} }";
    assert!(diagnostics_or_none(text).is_empty());
    assert_eq!(
        messages("fn f() -> {} { for'l x in [1] { if x > 0 { return'l 1 } }; {} }"),
        vec![
            "unused value of type `Int | List<{}>`",
            "this loop is its list or a value returned to it, of different types"
        ]
    );
}
