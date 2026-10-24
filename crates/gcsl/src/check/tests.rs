use super::*;
use crate::diagnostic::Severity;
use crate::lexer::{tokenize, tokenize_with};
use crate::parser::parse;
use crate::source::SourceFile;

fn parse_text(text: &str) -> Ast {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    parse(&tokens, names, strings, &source).unwrap()
}

/// The expression of the file's final result or expression item.
fn result_expr(ast: &Ast) -> ExprId {
    let last = *ast.items(ast.root()).last().unwrap();
    match ast.item(last).kind {
        ItemKind::Result(value) | ItemKind::Expr(value) => value,
        other => panic!("the last item has no result: {other:?}"),
    }
}

/// Checks `text` as a file with a fresh type table.
fn check_text(text: &str) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let output = Checker::new(&ast, &mut types).check();
    (ast, types, output)
}

/// Checks `text` as a lone file is, beside the standard library, so
/// that the prelude's names, `len` among them, are its: what the
/// helpers that look only at types and diagnostics check with.
fn check_prelude(text: &str) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    crate::workspace::check_beside_prelude(text)
}

/// The number of slots the standard library takes, the first: where a
/// text checked by [`check_prelude`] starts its own.
fn standard_slots() -> u32 {
    let (_, _, output) = check_prelude("");
    u32::try_from(output.unwrap().scope.into_slot_names().len()).unwrap()
}

/// The type of the file's result.
fn result_type(text: &str) -> TyId {
    let (ast, _, output) = check_prelude(text);
    output.unwrap().checked.types.get(result_expr(&ast))
}

/// The type of the file's result, as shown to users.
fn shown_result(text: &str) -> String {
    let (ast, types, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let ty = output.checked.types.get(result_expr(&ast));
    types.display(ty, ast.names()).to_string()
}

/// The type of the file's value, as shown to users.
fn file_type(text: &str) -> String {
    let (ast, types, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    types.display(output.ty, ast.names()).to_string()
}

fn diagnostics(text: &str) -> Vec<Diagnostic> {
    check_prelude(text).2.unwrap_err()
}

/// The warnings of a check that must succeed.
fn warnings(text: &str) -> Vec<Diagnostic> {
    check_prelude(text)
        .2
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
        .warnings
}

fn errors(text: &str) -> Vec<(String, Span)> {
    diagnostics(text)
        .into_iter()
        .map(|d| (d.message, d.span))
        .collect()
}

fn messages(text: &str) -> Vec<String> {
    errors(text).into_iter().map(|(m, _)| m).collect()
}

/// Every node's type in allocation order (children before parents).
fn all_types(text: &str) -> Vec<TyId> {
    check_prelude(text).2.unwrap().checked.types.types
}

/// The labels of a diagnostic as `(span, text)` pairs.
fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
    diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect()
}

// A concrete Int operand is written `(7 // 2)`: `//` always produces a
// concrete Int, so the typing rows with a concrete Int are reachable
// without variables.

#[test]
fn literals() {
    assert_eq!(result_type("1"), TyId::INT);
    assert_eq!(result_type("1.5"), TyId::FLOAT);
}

#[test]
fn untyped_op_untyped_stays_untyped_until_the_root() {
    for op in ["+", "-", "*"] {
        let text = format!("1 {op} 2");
        assert_eq!(all_types(&text), vec![TyId::INT; 3], "{text}");
    }
}

#[test]
fn untyped_op_int_is_int() {
    for op in ["+", "-", "*"] {
        assert_eq!(result_type(&format!("1 {op} (7 // 2)")), TyId::INT);
        assert_eq!(result_type(&format!("(7 // 2) {op} 1")), TyId::INT);
    }
}

#[test]
fn untyped_op_float_is_float() {
    for op in ["+", "-", "*"] {
        let text = format!("1 {op} 2.0");
        assert_eq!(all_types(&text), vec![TyId::FLOAT; 3], "{text}");
        let text = format!("2.0 {op} 1");
        assert_eq!(all_types(&text), vec![TyId::FLOAT; 3], "{text}");
    }
}

#[test]
fn same_concrete_types() {
    assert_eq!(result_type("(7 // 2) + (7 // 2)"), TyId::INT);
    assert_eq!(result_type("1.0 * 2.0"), TyId::FLOAT);
}

#[test]
fn int_and_float_do_not_mix() {
    assert_eq!(
        errors("(7 // 2) + 1.5"),
        vec![("cannot add Int and Float".to_owned(), Span::new(9, 10))]
    );
    assert_eq!(
        messages("1.5 - (7 // 2)"),
        vec!["cannot subtract Float and Int"]
    );
    assert_eq!(
        messages("(7 % 2) * 1.5"),
        vec!["cannot multiply Int and Float"]
    );
}

#[test]
fn negation_preserves_type() {
    assert_eq!(all_types("-1"), vec![TyId::INT; 2]);
    assert_eq!(all_types("-1.5"), vec![TyId::FLOAT; 2]);
    assert_eq!(result_type("-(7 // 2)"), TyId::INT);
    // An untyped negation adapts like a literal.
    assert_eq!(all_types("-1 + 2.0"), vec![TyId::FLOAT; 4]);
}

#[test]
fn division_accepts_any_mix_and_yields_float() {
    assert_eq!(all_types("7 / 2"), vec![TyId::FLOAT; 3]);
    assert_eq!(all_types("7 / 2.0"), vec![TyId::FLOAT; 3]);
    assert_eq!(all_types("7.0 / 2"), vec![TyId::FLOAT; 3]);
    assert_eq!(result_type("(7 // 2) / 2.0"), TyId::FLOAT);
    assert_eq!(result_type("2.0 / (7 // 2)"), TyId::FLOAT);
    assert_eq!(result_type("(7 // 2) / (7 // 2)"), TyId::FLOAT);
}

#[test]
fn division_leaves_concrete_int_operands_int() {
    let ast = parse_text("(7 // 2) / 2");
    let types = check(&ast, &mut TypeTable::new()).unwrap().0.types;
    let ExprKind::Binary { lhs, rhs, .. } = ast.get(result_expr(&ast)).kind else {
        panic!("expected binary");
    };
    assert_eq!(types.get(lhs), TyId::INT);
    assert_eq!(types.get(rhs), TyId::FLOAT);
}

#[test]
fn floor_division_and_modulo_need_integers() {
    assert_eq!(all_types("7 // 2"), vec![TyId::INT; 3]);
    assert_eq!(all_types("7 % 2"), vec![TyId::INT; 3]);
    assert_eq!(result_type("(7 // 2) // 2"), TyId::INT);
    assert_eq!(
        errors("7.0 // 2"),
        vec![("cannot floor-divide Float".to_owned(), Span::new(4, 6))]
    );
    assert_eq!(messages("7 // 2.0"), vec!["cannot floor-divide Float"]);
    assert_eq!(messages("7.0 % 2"), vec!["cannot take modulo of Float"]);
    assert_eq!(messages("7 % 2.0"), vec!["cannot take modulo of Float"]);
    assert_eq!(messages("7.0 % 2.0"), vec!["cannot take modulo of Float"]);
}

#[test]
fn floor_division_result_does_not_adapt_to_float() {
    assert_eq!(messages("(7 // 2) + 1.0"), vec!["cannot add Int and Float"]);
}

#[test]
fn untyped_subtree_is_coerced_all_the_way_down() {
    // Allocation order: 1, 2, +, paren, 3.0, *.
    assert_eq!(all_types("(1 + 2) * 3.0"), vec![TyId::FLOAT; 6]);
    assert_eq!(all_types("3.0 * -(1 - -2)"), vec![TyId::FLOAT; 8]);
}

#[test]
fn division_makes_untyped_siblings_float() {
    // `2 / 3` is Float, so the untyped `1` beside it adapts to Float.
    assert_eq!(all_types("1 + 2 / 3"), vec![TyId::FLOAT; 5]);
}

#[test]
fn untyped_root_defaults_to_int_throughout() {
    assert_eq!(all_types("-(1 + 2) * 3"), vec![TyId::INT; 7]);
    assert_eq!(all_types("= 1 + 2"), vec![TyId::INT; 3]);
    assert_eq!(file_type("1 + 2"), "Int");
}

#[test]
fn multiple_errors_in_one_run() {
    assert_eq!(
        errors("(7.0 // 2) + (1 % 2.5)"),
        vec![
            ("cannot floor-divide Float".to_owned(), Span::new(5, 7)),
            ("cannot take modulo of Float".to_owned(), Span::new(16, 17)),
        ]
    );
}

#[test]
fn operands_are_labelled_with_their_types() {
    let concrete = diagnostics("(7 // 2) + 1.5");
    assert_eq!(
        labels(&concrete[0]),
        vec![(Span::new(0, 8), "Int"), (Span::new(11, 14), "Float")]
    );
    // An untyped literal is labelled as the Int it would default to.
    let untyped = diagnostics("1 % 2.5");
    let texts: Vec<&str> = labels(&untyped[0]).into_iter().map(|(_, t)| t).collect();
    assert_eq!(texts, vec!["Int", "Float"]);
}

#[test]
fn errors_do_not_cascade() {
    // The failed `//` becomes an error type that every enclosing operator
    // accepts, whatever the other operand is.
    for text in [
        "(1.0 // 2) + (7 // 2)",
        "(1.0 // 2) + 1.5",
        "(1.0 // 2) + 1",
        "1.5 * (1.0 // 2)",
        "-(1.0 // 2) // 2",
        "(1.0 // 2) / 2.5",
        "2 % (1.0 // 2)",
        "(1.0 // 2) : Float",
        "{ 1.0 // 2 } + 1.5",
        "{ x = 1.0 // 2 }.x + 1",
        "(1.0 // 2).x",
    ] {
        assert_eq!(messages(text), vec!["cannot floor-divide Float"], "{text}");
    }
}

#[test]
fn an_import_outside_a_workspace_is_an_error_with_the_files_others() {
    // At the import, whatever its form, alongside the file's other
    // errors.
    let message = |path: &str| {
        format!(
            "`{path}` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
        )
    };
    assert_eq!(
        errors("x = 1 + true\nimport a/b as c"),
        vec![
            ("cannot add Int and Bool".to_owned(), Span::new(6, 7)),
            (message("a/b"), Span::new(13, 28)),
        ]
    );
    assert_eq!(messages("import x, * from lib"), vec![message("lib")]);
    // Checked with no workspace at all, there is nothing to import.
    let (_, _, output) = check_text("import lib");
    assert_eq!(
        output.unwrap_err()[0].message,
        "`import` needs a workspace: no `workspace.gcsl` above this file"
    );
}

#[test]
fn the_empty_record_is_rejected_by_every_operator() {
    assert_eq!(
        errors("{} + 1"),
        vec![("cannot add `{}` and Int".to_owned(), Span::new(3, 4))]
    );
    assert_eq!(messages("1.5 * {}"), vec!["cannot multiply Float and `{}`"]);
    assert_eq!(messages("{} / 2"), vec!["cannot divide `{}` and Int"]);
    assert_eq!(messages("{} // 2"), vec!["cannot floor-divide `{}`"]);
    assert_eq!(messages("7 % {}"), vec!["cannot take modulo of `{}`"]);
    assert_eq!(messages("1.5 // {}"), vec!["cannot floor-divide Float"]);
    let negation = &diagnostics("-{}")[0];
    assert_eq!(negation.message, "cannot negate `{}`");
    assert_eq!(negation.span, Span::new(0, 1));
    assert_eq!(labels(negation), vec![(Span::new(1, 3), "{}")]);
    // The same goes for records with fields.
    assert_eq!(messages("-{ x = 1 }"), vec!["cannot negate `{x: Int}`"]);
    assert_eq!(
        labels(&diagnostics("{ x = 1 } + 1")[0]),
        vec![(Span::new(0, 9), "{x: Int}"), (Span::new(12, 13), "Int")]
    );
}

#[test]
fn error_type_never_escapes_a_successful_check() {
    // Every node of a successful check has a concrete type.
    for text in [
        "1",
        "1.5",
        "(1 + 2) * 3.0",
        "7 / 2",
        "(7 // 2) / 2",
        "let x = 5\nx + 1",
        "let y = { 1 + 2 }\n{ 3 } : Float",
        "= { let a = 1\n= a } * 2",
        "{}",
        "x = 1\ny = { z = 2 }",
        "let p: {x: Float} = { x = 1 }\np.x",
        "if true { { a = 1 } } else { { a = 2 } }",
    ] {
        for ty in all_types(text) {
            assert!(
                ty != TyId::ERROR && ty != TyId::UNTYPED_INT,
                "{text}: {ty:?}"
            );
        }
    }
}

#[test]
fn annotation_renders_types() {
    let ast = parse_text("1 + 2.0");
    let table = &mut TypeTable::new();
    let types = check(&ast, table).unwrap().0.types;
    assert_eq!(
        ast.pretty_with(&types.annotation(table, ast.names()))
            .to_string(),
        "(file\n  (expr @0..7\n    (+ : Float @0..7\n      (int 1 : Float @0..1)\n      (float 2.0 : Float @4..7))))"
    );
    let ast = parse_text("let x = { 1 }\nx : Float");
    assert!(check(&ast, table).is_err());
    let ast = parse_text("let x = { 1 }\n{}");
    let types = check(&ast, table).unwrap().0.types;
    assert_eq!(
        ast.pretty_with(&types.annotation(table, ast.names()))
            .to_string(),
        "(file\n\
             \x20 (let x @0..13\n\
             \x20   (block : Int @8..13\n\
             \x20     (expr @10..11\n\
             \x20       (int 1 : Int @10..11))))\n\
             \x20 (expr @14..16\n\
             \x20   (block : {} @14..16)))"
    );
    let ast = parse_text("{ y = \"s\"; x = 1 }.x");
    let types = check(&ast, table).unwrap().0.types;
    assert_eq!(
        ast.pretty_with(&types.annotation(table, ast.names()))
            .to_string(),
        "(file\n\
             \x20 (expr @0..20\n\
             \x20   (. x : Int @0..20\n\
             \x20     (block : {x: Int, y: String} @0..18\n\
             \x20       (field y @2..9\n\
             \x20         (str \"s\" : String @6..9))\n\
             \x20       (field x @11..16\n\
             \x20         (int 1 : Int @15..16))))))"
    );
}

#[test]
fn variables_have_the_type_they_were_bound_with() {
    assert_eq!(result_type("let x = 5\nx"), TyId::INT);
    assert_eq!(result_type("let x = 5.5\nx"), TyId::FLOAT);
    assert_eq!(result_type("let x = {}\nx"), TyId::EMPTY);
    // The untyped literal defaults where it is bound.
    assert_eq!(
        errors("let x = 5\nx + 2.5"),
        vec![("cannot add Int and Float".to_owned(), Span::new(12, 13))]
    );
    assert_eq!(result_type("let x = 5 : Float\nx + 2.5"), TyId::FLOAT);
    assert_eq!(result_type("let x: Float = 5\nx + 2.5"), TyId::FLOAT);
}

#[test]
fn a_let_is_visible_from_the_next_item() {
    assert_eq!(
        errors("let x = x"),
        vec![("undefined variable `x`".to_owned(), Span::new(8, 9))]
    );
    // The initialiser sees the binding being shadowed.
    assert_eq!(result_type("let x = 1.5\nlet x = x + 1\nx"), TyId::FLOAT);
}

#[test]
fn shadowing_in_the_same_block() {
    assert_eq!(result_type("let x = 1\nlet x = 2.5\nx"), TyId::FLOAT);
    let ast = parse_text("let x = 1\nlet x = 2.5\nx");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    assert_eq!(
        resolution.expr_var(result_expr(&ast)),
        VarRef::Local(Slot(1))
    );
}

#[test]
fn shadowing_across_blocks() {
    assert_eq!(result_type("let x = 1\n{ let x = 2.5; x }"), TyId::FLOAT);
    // The outer variable is visible in an inner block until shadowed.
    assert_eq!(result_type("let x = 1\n{ let y = x; y }"), TyId::INT);
    // And is back in scope after the block.
    assert_eq!(result_type("let x = 1\n{ let x = 2.5 }\nx"), TyId::INT);
}

#[test]
fn variables_are_not_visible_after_their_block() {
    assert_eq!(
        errors("{ let y = 1 }\ny"),
        vec![("undefined variable `y`".to_owned(), Span::new(14, 15))]
    );
    assert_eq!(
        messages("let z = { let inner = 1; inner }\ninner"),
        vec!["undefined variable `inner`"]
    );
}

#[test]
fn undefined_variables() {
    let diagnostic = &diagnostics("x + 1")[0];
    assert_eq!(diagnostic.message, "undefined variable `x`");
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert!(diagnostic.notes.is_empty());
    let diagnostic = &diagnostics("let x = 2\nx-1")[0];
    assert_eq!(diagnostic.message, "undefined variable `x-1`");
    assert_eq!(diagnostic.span, Span::new(10, 13));
    assert_eq!(
        diagnostic.notes,
        vec!["names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction"]
    );
}

#[test]
fn rebinding_in_the_same_block() {
    assert_eq!(result_type("let x = 1\nx := 2\nx"), TyId::INT);
    assert_eq!(
        result_type("let x = 1\nx := x + 1\nx := x * 2\nx"),
        TyId::INT
    );
    // In an inner block, of a variable declared in that block, even one
    // shadowing an outer variable.
    assert_eq!(
        result_type("let x = 1\n{ let x = 2.5; x := 3.5; x }"),
        TyId::FLOAT
    );
    // The new value sees the old binding.
    let ast = parse_text("let x = 1\nx := x + 1\nx");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[0]), Slot(0));
    // `:=` takes a fresh slot.
    assert_eq!(resolution.item_slot(items[1]), Slot(1));
    assert_eq!(
        resolution.expr_var(result_expr(&ast)),
        VarRef::Local(Slot(1))
    );
}

#[test]
fn rebinding_is_assignment_compatible() {
    // The literal adapts to the variable's type, which the rebind keeps.
    assert_eq!(result_type("let x: Float = 1\nx := 2\nx"), TyId::FLOAT);
    // A subtype fits, and the variable keeps its wider type.
    assert_eq!(
        shown_result("let p: {x: Int} = { x = 1 }\np := { x = 2; y = 3 }\np"),
        "{x: Int}"
    );
    // A different type does not: changing a type takes a `let`.
    let diagnostic = &diagnostics("let x = 1\nx := 2.5\nx")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(15, 18));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found Float")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 9), "the variable was declared here")]
    );
    // The variable keeps its type afterwards, so nothing cascades.
    assert_eq!(
        messages("let x = 1\nx := 2.5\nx + 1"),
        vec!["mismatched types"]
    );
    assert_eq!(
        result_type("let x = 1\nlet x = 2.5\nx := 3.5\nx"),
        TyId::FLOAT
    );
}

#[test]
fn a_dotted_rebind_keeps_the_leaf_type_and_adds_nothing() {
    assert_eq!(
        shown_result("let a = { x = 1; n = { m = 1 } }\na.n.m := 5\na"),
        "{n: {m: Int}, x: Int}"
    );
    // A subtype fits at the leaf.
    assert_eq!(
        shown_result("let a: {r: {x: Int}} = { r = { x = 1 } }\na.r := { x = 2; y = 3 }\na"),
        "{r: {x: Int}}"
    );
    let diagnostic = &diagnostics("let a = { x = 1 }\na.x := \"s\"\na")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(18, 21), "`:=` keeps the field's type")]
    );
    let diagnostic = &diagnostics("let a = { x = 1 }\na.y := 2\na")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a.y`: `a` has no field `y`"
    );
    assert_eq!(diagnostic.span, Span::new(18, 21));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(18, 19), "this is {x: Int}")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["`:=` cannot add a field; add it with `let a.y = ...`"]
    );
    assert_eq!(
        messages("let a = { x = 1 }\na.x.z := 2"),
        vec!["cannot update `a.x.z`: `a.x` is Int, not a record"]
    );
    // The rejected rebind declares `a` as an error, so nothing cascades.
    assert_eq!(
        messages("let a = { x = 1 }\na.y := 2\na.x + 1"),
        vec!["cannot rebind `a.y`: `a` has no field `y`"]
    );
}

#[test]
fn rebinding_an_undeclared_name() {
    let diagnostic = &diagnostics("x := 1")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `x`: no variable named `x` is declared"
    );
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert!(diagnostic.labels.is_empty());
    assert_eq!(diagnostic.notes, vec!["declare it with `let x = ...`"]);
    // A later declaration does not count.
    assert_eq!(
        messages("x := 1\nlet x = 2"),
        vec!["cannot rebind `x`: no variable named `x` is declared"]
    );
}

#[test]
fn rebinding_a_name_from_an_enclosing_block() {
    let diagnostic = &diagnostics("let x = 1\n{\n  x := 2\n}")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `x` here: it is declared in an enclosing block"
    );
    assert_eq!(diagnostic.span, Span::new(14, 15));
    assert_eq!(labels(diagnostic), vec![(Span::new(0, 9), "declared here")]);
    assert_eq!(
        diagnostic.notes,
        vec![
            "shadow it with `let x = ...` instead; only a variable declared with `let mut` \
                 can be rebound from an inner block"
        ]
    );
    // The label points at the latest declaration.
    let diagnostic = &diagnostics("let x = 1\nx := 2\n{ x := 3 }")[0];
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(10, 16), "declared here")]
    );
    // From a function or a `lazy`, `let mut` would not help, so the
    // note does not suggest it.
    for text in [
        "let x = 1\nlet f = fn(): Int { x := 2; x }",
        "let x = 1\nlet l = lazy { x := 2; x }",
        "let x = 1\nlet f = fn(): Int { if true { x := 2 }; x }",
    ] {
        let diagnostic = &diagnostics(text)[0];
        assert_eq!(
            diagnostic.message, "cannot rebind `x` here: it is declared in an enclosing block",
            "{text}"
        );
        assert_eq!(
            diagnostic.notes,
            vec!["shadow it with `let x = ...` instead"],
            "{text}"
        );
    }
}

#[test]
fn a_rejected_rebind_still_declares_the_variable() {
    assert_eq!(
        messages("y := 1\ny + 1"),
        vec!["cannot rebind `y`: no variable named `y` is declared"]
    );
    assert_eq!(
        messages("let x = 1\n{ x := 2.5; x * 1.5 }"),
        vec!["cannot rebind `x` here: it is declared in an enclosing block"]
    );
}

#[test]
fn annotations() {
    let ast = parse_text("let x: Float = 5");
    assert_eq!(
        check(&ast, &mut TypeTable::new()).unwrap().0.types.types,
        vec![TyId::FLOAT]
    );
    assert_eq!(result_type("let x: Int = 5\nx"), TyId::INT);
    assert_eq!(result_type("let x: Float = 5\nx"), TyId::FLOAT);
    assert_eq!(result_type("let x: Float = 2.5\nx"), TyId::FLOAT);
    assert_eq!(result_type("let x: {} = {}\nx"), TyId::EMPTY);
    assert_eq!(result_type("let x: Float = -(1 + 2)\nx"), TyId::FLOAT);
    assert_eq!(result_type("let x: Int = 7 // 2\nx"), TyId::INT);
}

#[test]
fn annotation_mismatches() {
    let diagnostic = &diagnostics("let x: Int = 5.0")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(13, 16));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found Float")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 10), "expected because of this annotation")]
    );
    assert_eq!(
        diagnostics("let x: Float = 7 // 2")[0].label.as_deref(),
        Some("expected Float, found Int")
    );
    assert_eq!(
        diagnostics("let x: {} = 1")[0].label.as_deref(),
        Some("expected {}, found Int")
    );
    assert_eq!(
        diagnostics("let x: Int = {}")[0].label.as_deref(),
        Some("expected Int, found {}")
    );
    // The variable has the annotated type, so uses do not cascade.
    assert_eq!(
        messages("let x: Int = 5.0\nlet y = x + 1\n= x // y"),
        vec!["mismatched types"]
    );
}

#[test]
fn unknown_types() {
    assert_eq!(
        errors("let x: Foo = 1"),
        vec![("unknown type `Foo`".to_owned(), Span::new(7, 10))]
    );
    assert_eq!(
        errors("1 : int"),
        vec![("unknown type `int`".to_owned(), Span::new(4, 7))]
    );
    // `Unit` is gone: the empty record is written `{}`.
    assert_eq!(
        errors("let x: Unit = {}"),
        vec![("unknown type `Unit`".to_owned(), Span::new(7, 11))]
    );
    // A variable is not a type.
    assert_eq!(
        messages("let x = 1\nlet y: x = 2"),
        vec!["unknown type `x`"]
    );
    // Neither the variable nor the ascription cascades.
    assert_eq!(
        messages("let x: Foo = 1.0 // 2\nx + 1.5\n(1 : Bar) + 2.5"),
        vec![
            "unknown type `Foo`",
            "cannot floor-divide Float",
            "unknown type `Bar`"
        ]
    );
    // Nor does an unknown type inside a record type.
    assert_eq!(
        messages("let p: {a: Foo} = { a = 1 }\np.a + 1.5"),
        vec!["unknown type `Foo`"]
    );
}

#[test]
fn ascriptions() {
    assert_eq!(all_types("5 : Float"), vec![TyId::FLOAT; 2]);
    assert_eq!(result_type("-5 : Float"), TyId::FLOAT);
    assert_eq!(result_type("1 + 2 : Float"), TyId::FLOAT);
    assert_eq!(result_type("let x = 1\nx : Int"), TyId::INT);
    assert_eq!(result_type("{} : {}"), TyId::EMPTY);
    assert_eq!(result_type("1 : Int : Int"), TyId::INT);
    // An untyped block adapts like a literal.
    assert_eq!(all_types("{ 5 } : Float"), vec![TyId::FLOAT; 3]);
    let diagnostic = &diagnostics("5.0 : Int")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(0, 3));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found Float")
    );
    assert_eq!(labels(diagnostic), vec![(Span::new(6, 9), "ascribed here")]);
    assert_eq!(
        diagnostics("let x = 1\nx : Float")[0].label.as_deref(),
        Some("expected Float, found Int")
    );
    // Chained ascriptions are each checked in turn.
    assert_eq!(
        errors("1 : Int : Float"),
        vec![("mismatched types".to_owned(), Span::new(0, 7))]
    );
}

#[test]
fn block_types() {
    assert_eq!(result_type("{}"), TyId::EMPTY);
    assert_eq!(result_type("{ 1.5 }"), TyId::FLOAT);
    assert_eq!(result_type("{ = 1.5 }"), TyId::FLOAT);
    assert_eq!(result_type("{ let a = 1 }"), TyId::EMPTY);
    assert_eq!(result_type("{ let a = 1; a }"), TyId::INT);
    assert_eq!(result_type("{ {} }"), TyId::EMPTY);
    assert_eq!(result_type("{ 1 } + 2.5"), TyId::FLOAT);
    assert_eq!(result_type("let x = { 1 }\nx"), TyId::INT);
    assert_eq!(file_type(""), "{}");
    assert_eq!(file_type("let x = 1"), "{}");
}

#[test]
fn unused_values() {
    let diagnostic = &diagnostics("1 + 2\n3")[0];
    assert_eq!(diagnostic.message, "unused value of type Int");
    assert_eq!(diagnostic.span, Span::new(0, 5));
    assert_eq!(
        diagnostic.notes,
        vec!["make it the block's result with `= ...`, bind it with `let`, or remove it"]
    );
    assert_eq!(messages("1.5; 2"), vec!["unused value of type Float"]);
    assert_eq!(
        messages("let x = 1\nx\nx"),
        vec!["unused value of type Int"]
    );
    assert_eq!(messages("{ 1; 2 }"), vec!["unused value of type Int"]);
    // A record with fields is a value like any other.
    assert_eq!(
        messages("{ x = 1 }\n2"),
        vec!["unused value of type `{x: Int}`"]
    );
    // `{}` statements are fine anywhere.
    assert_eq!(result_type("{}\n{ let a = 1 }\n{ {} }\n2"), TyId::INT);
    // An erroneous statement is reported once, not also as unused.
    assert_eq!(messages("1.0 // 2\n3"), vec!["cannot floor-divide Float"]);
}

#[test]
fn a_call_that_is_not_pure_may_discard_its_value() {
    for text in [
        "fn one() -> Int { 1 }\none()\n2",
        "fn one() -> Int { 1 }\nfn f() -> Int {\n    one()\n    = 2\n}\nf()",
        "fn one() ->> Int { 1 }\none()\n2",
        "let one = fn() -> Int { 1 }\none()\n2",
        "fn later() -> Int { = lazy 1 }\nlater()\n2",
        "sh(\"true\")\n2",
    ] {
        assert_eq!(result_type(text), TyId::INT, "{text}");
        assert_eq!(warnings(text).len(), 0, "{text}");
    }
    // A pure call's value is unused, as any pure value is.
    assert_eq!(
        messages("fn one(): Int { 1 }\none()\n2"),
        vec!["unused value of type Int"]
    );
    // In a pure function the call is an error, and its value unused.
    assert_eq!(
        messages("fn one() -> Int { 1 }\nfn f(): Int {\n    one()\n    = 2\n}\nf()"),
        vec![
            "cannot call an impure function from a pure function",
            "unused value of type Int"
        ]
    );
}

#[test]
fn the_result_must_be_the_last_item() {
    let diagnostic = &diagnostics("= 1\nlet y = 2")[0];
    assert_eq!(
        diagnostic.message,
        "the result must be the last item in its block"
    );
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(4, 13), "this item comes after it")]
    );
    // A second result is caught by the same rule, at the first.
    assert_eq!(
        errors("{ = 1; = 2 }"),
        vec![(
            "the result must be the last item in its block".to_owned(),
            Span::new(2, 3)
        )]
    );
    // A final expression after a misplaced result is not also unused.
    assert_eq!(
        messages("= 1\n2"),
        vec!["the result must be the last item in its block"]
    );
}

/// Every `let` in `body` and the blocks nested in it, in the order their
/// declarations complete.
fn lets(ast: &Ast, body: ItemRange, out: &mut Vec<ItemId>) {
    fn visit(ast: &Ast, id: ExprId, out: &mut Vec<ItemId>) {
        match ast.get(id).kind {
            ExprKind::Block { items: body, .. } => lets(ast, body, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                visit(ast, lhs, out);
                visit(ast, rhs, out);
            }
            ExprKind::Paren(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::Field { expr: inner, .. }
            | ExprKind::Ascribe { expr: inner, .. }
            | ExprKind::Is { expr: inner, .. }
            | ExprKind::As { expr: inner, .. }
            | ExprKind::NoField(inner) => visit(ast, inner, out),
            ExprKind::If {
                cond, then, else_, ..
            } => {
                visit(ast, cond, out);
                visit(ast, then, out);
                visit(ast, else_, out);
            }
            ExprKind::Interp(parts) => {
                for &part in ast.parts(parts) {
                    if let StrPart::Expr(inner) = part {
                        visit(ast, inner, out);
                    }
                }
            }
            ExprKind::Fn(function) | ExprKind::Lazy(function) => {
                visit(ast, ast.fn_decl(function).body, out);
            }
            ExprKind::Call { callee, args } | ExprKind::Unclosed { callee, args } => {
                visit(ast, callee, out);
                for arg in ast.args(args) {
                    visit(ast, arg.value, out);
                }
            }
            ExprKind::List(elements) => {
                for element in ast.elements(elements) {
                    visit(ast, element.value, out);
                }
            }
            ExprKind::Index { expr, index } => {
                visit(ast, expr, out);
                visit(ast, index, out);
            }
            ExprKind::For(id) => {
                let decl = ast.for_decl(id);
                if let ForHead::Each {
                    iterable, guard, ..
                } = decl.head
                {
                    visit(ast, iterable, out);
                    if let Some(guard) = guard {
                        visit(ast, guard, out);
                    }
                }
                visit(ast, decl.body, out);
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Name(_)
            | ExprKind::DefTypeRef(_)
            | ExprKind::Error => {}
        }
    }
    for &id in ast.items(body) {
        match ast.item(id).kind {
            ItemKind::Let { value, .. } => {
                visit(ast, value, out);
                out.push(id);
            }
            ItemKind::Rebind { value, .. }
            | ItemKind::Field { value, .. }
            | ItemKind::Spread(value)
            | ItemKind::Result(value)
            | ItemKind::Expr(value)
            | ItemKind::DefTypeDef { value, .. }
            | ItemKind::DefMeta { value, .. } => {
                visit(ast, value, out);
            }
            ItemKind::Break(_)
            | ItemKind::Continue(_)
            | ItemKind::Return(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::Error => {}
        }
    }
}

#[test]
fn slots_are_assigned_stack_fashion() {
    let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
    let ast = parse_text(text);
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let names = ast.names();
    let mut all_lets = Vec::new();
    lets(&ast, ast.root(), &mut all_lets);
    let slot_of = |text: &str| -> Vec<u32> {
        let wanted = names.lookup(text).unwrap();
        all_lets
                .iter()
                .filter(|&&id| {
                    matches!(ast.item(id).kind, ItemKind::Let { path, .. } if ast.path(path)[0].name() == Some(wanted))
                })
                .map(|&id| resolution.item_slot(id).get())
                .collect()
    };
    // Each initialiser's temporary takes the slot its variable then
    // reuses, since the variable is declared after its initialiser's
    // block has ended.
    assert_eq!(slot_of("t"), vec![0]);
    assert_eq!(slot_of("a"), vec![0]);
    assert_eq!(slot_of("u"), vec![1]);
    assert_eq!(slot_of("b"), vec![1]);
    assert_eq!(slot_of("v"), vec![2]);
    assert_eq!(slot_of("w"), vec![3]);
    assert_eq!(slot_of("c"), vec![2]);
    assert_eq!(resolution.frame_size(), 4);
    let slot_names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| names.get(name))
        .collect();
    assert_eq!(slot_names, vec!["a", "b", "c", "w"]);
    // Uses read the slot of the binding in scope.
    let ExprKind::Binary { lhs, rhs, .. } = ast.get(result_expr(&ast)).kind else {
        panic!("expected a sum");
    };
    assert_eq!(resolution.expr_var(rhs), VarRef::Local(Slot(2)));
    let ExprKind::Binary { lhs: a, rhs: b, .. } = ast.get(lhs).kind else {
        panic!("expected a sum");
    };
    assert_eq!(resolution.expr_var(a), VarRef::Local(Slot(0)));
    assert_eq!(resolution.expr_var(b), VarRef::Local(Slot(1)));
    // Nodes that are not variables have no slot.
    assert_eq!(resolution.expr_var(lhs), VarRef::None);
}

#[test]
fn slots_after_a_block_are_reused() {
    let ast = parse_text("let a = 1\n{ let b = 2; let c = 3 }\nlet d = 4\nd");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[0]), Slot(0));
    assert_eq!(resolution.item_slot(items[1]), Slot::NONE);
    // `b` was in slot 1; `d` reuses it.
    assert_eq!(resolution.item_slot(items[2]), Slot(1));
    assert_eq!(resolution.frame_size(), 3);
    assert_eq!(
        resolution.expr_var(result_expr(&ast)),
        VarRef::Local(Slot(1))
    );
    assert_eq!(Slot(3).to_string(), "#3");
}

#[test]
fn fields_take_slots_like_variables() {
    let ast = parse_text("let a = 1\nlet r = { x = 2; let t = x; y = t }\nb = a");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    // The file's field `b`, a top-level field, is declared as the input
    // starts, in slot 0.
    assert_eq!(resolution.item_slot(items[2]), Slot(0));
    assert_eq!(resolution.item_slot(items[0]), Slot(1));
    // The block's field, variable and field take slots 2 to 4; `r`,
    // declared after its initialiser, reuses slot 2.
    assert_eq!(resolution.item_slot(items[1]), Slot(2));
    assert_eq!(resolution.frame_size(), 5);
    let names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| ast.names().get(name))
        .collect();
    assert_eq!(names, vec!["b", "a", "r", "t", "y"]);
}

#[test]
fn a_program_without_variables_needs_no_frame() {
    let ast = parse_text("1 + { 2 }");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    assert_eq!(resolution.frame_size(), 0);
    assert!(resolution.slot_names().is_empty());
}

#[test]
fn recovery_does_not_cascade() {
    // A `let` whose initialiser fails is still declared, as Error.
    assert_eq!(
        messages("let x = 1.0 // 2\nlet y = x + 1\n= x * 2.5 + y"),
        vec!["cannot floor-divide Float"]
    );
    // An undefined variable is reported at each use, and nothing more.
    assert_eq!(
        messages("let y = z + 1.5\nz * 2\ny // 2"),
        vec!["undefined variable `z`", "undefined variable `z`"]
    );
    // Independent errors are all reported.
    assert_eq!(
        messages("let x = 1.0 // 2\nlet y: Int = 2.5\nq"),
        vec![
            "cannot floor-divide Float",
            "mismatched types",
            "undefined variable `q`"
        ]
    );
}

/// Declares a variable of each type: `i`, `f`, `b`, `s` and `u`.
const PRELUDE: &str = "let i = 1\nlet f = 1.5\nlet b = true\nlet s = \"s\"\nlet u = {}\n";

/// Each variable of [`PRELUDE`], its type, and the type as messages
/// show it.
const VARIABLES: [(&str, TyId, &str); 5] = [
    ("i", TyId::INT, "Int"),
    ("f", TyId::FLOAT, "Float"),
    ("b", TyId::BOOL, "Bool"),
    ("s", TyId::STRING, "String"),
    ("u", TyId::EMPTY, "`{}`"),
];

#[test]
fn literals_of_the_new_types() {
    assert_eq!(result_type("true"), TyId::BOOL);
    assert_eq!(result_type("false"), TyId::BOOL);
    assert_eq!(result_type("\"a\""), TyId::STRING);
    assert_eq!(result_type("r\"a\""), TyId::STRING);
    assert_eq!(result_type("\"{1}\""), TyId::STRING);
}

#[test]
fn equality_accepts_two_operands_of_any_one_type() {
    for (lhs, lhs_ty, lhs_shown) in VARIABLES {
        for (rhs, rhs_ty, rhs_shown) in VARIABLES {
            for op in ["==", "!="] {
                let text = format!("{PRELUDE}{lhs} {op} {rhs}");
                if lhs_ty == rhs_ty {
                    assert_eq!(result_type(&text), TyId::BOOL, "{text}");
                } else {
                    assert_eq!(
                        messages(&text),
                        vec![format!("cannot compare {lhs_shown} and {rhs_shown}")],
                        "{text}"
                    );
                }
            }
        }
    }
    assert_eq!(result_type("{} == {}"), TyId::BOOL);
}

#[test]
fn ordering_accepts_two_ints_floats_or_strings() {
    for (lhs, lhs_ty, lhs_shown) in VARIABLES {
        for (rhs, rhs_ty, rhs_shown) in VARIABLES {
            for op in ["<", "<=", ">", ">="] {
                let text = format!("{PRELUDE}{lhs} {op} {rhs}");
                let unordered = [(lhs_ty, lhs_shown), (rhs_ty, rhs_shown)]
                    .into_iter()
                    .find(|&(ty, _)| ty == TyId::BOOL || ty == TyId::EMPTY);
                if let Some((_, unordered)) = unordered {
                    assert_eq!(
                        messages(&text),
                        vec![format!("cannot order {unordered}")],
                        "{text}"
                    );
                } else if lhs_ty == rhs_ty {
                    assert_eq!(result_type(&text), TyId::BOOL, "{text}");
                } else {
                    assert_eq!(
                        messages(&text),
                        vec![format!("cannot compare {lhs_shown} and {rhs_shown}")],
                        "{text}"
                    );
                }
            }
        }
    }
}

#[test]
fn comparison_errors_point_at_the_operator_and_label_the_operands() {
    let diagnostic = &diagnostics("(7 // 2) == 1.5")[0];
    assert_eq!(diagnostic.message, "cannot compare Int and Float");
    assert_eq!(diagnostic.span, Span::new(9, 11));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 8), "Int"), (Span::new(12, 15), "Float")]
    );
    let diagnostic = &diagnostics("true < false")[0];
    assert_eq!(diagnostic.message, "cannot order Bool");
    assert_eq!(diagnostic.span, Span::new(5, 6));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 4), "Bool"), (Span::new(7, 12), "Bool")]
    );
    // An unordered type is reported before a mismatch.
    assert_eq!(messages("1 >= {}"), vec!["cannot order `{}`"]);
    assert_eq!(messages("true <= 1"), vec!["cannot order Bool"]);
    // An untyped operand is labelled as the Int it defaults to.
    let diagnostic = &diagnostics("\"a\" != 1")[0];
    assert_eq!(diagnostic.message, "cannot compare String and Int");
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 3), "String"), (Span::new(7, 8), "Int")]
    );
}

#[test]
fn untyped_literals_adapt_in_comparisons() {
    // Allocation order: operands, then the comparison.
    assert_eq!(all_types("1 == 2"), vec![TyId::INT, TyId::INT, TyId::BOOL]);
    assert_eq!(
        all_types("1 < 2.5"),
        vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
    );
    assert_eq!(
        all_types("2.5 >= 1"),
        vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
    );
    assert_eq!(
        all_types("1 + 2 != 3"),
        vec![TyId::INT, TyId::INT, TyId::INT, TyId::INT, TyId::BOOL]
    );
    assert_eq!(
        all_types("{ 1 } == 1.5"),
        vec![TyId::FLOAT, TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
    );
    assert_eq!(result_type("1 <= (7 // 2)"), TyId::BOOL);
    assert_eq!(
        all_types("let x = 1 == 1\nx"),
        vec![TyId::INT, TyId::INT, TyId::BOOL, TyId::BOOL]
    );
}

#[test]
fn comparison_errors_do_not_cascade() {
    assert_eq!(
        messages("(1.0 // 2) == 1"),
        vec!["cannot floor-divide Float"]
    );
    assert_eq!(
        messages("true < (1.0 // 2)"),
        vec!["cannot floor-divide Float"]
    );
    assert_eq!(
        messages("(1 == 1.5 // 2) and true"),
        vec!["cannot floor-divide Float"]
    );
}

#[test]
fn logic_operators_need_bools() {
    assert_eq!(result_type("true and false"), TyId::BOOL);
    assert_eq!(result_type("true or false"), TyId::BOOL);
    assert_eq!(result_type("not true"), TyId::BOOL);
    assert_eq!(result_type("not 1 == 2 or 1 < 2 and true"), TyId::BOOL);
    let diagnostic = &diagnostics("1 and true")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Bool, found Int")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "`and` requires Bool operands")]
    );
    let diagnostic = &diagnostics("true or 2.5")[0];
    assert_eq!(diagnostic.span, Span::new(8, 11));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Bool, found Float")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(5, 7), "`or` requires Bool operands")]
    );
    let diagnostic = &diagnostics("not \"a\"")[0];
    assert_eq!(diagnostic.span, Span::new(4, 7));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Bool, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 3), "`not` requires Bool operands")]
    );
    // Each wrong operand is reported.
    let both = diagnostics("1 or {}");
    assert_eq!(both.len(), 2);
    assert_eq!(both[0].label.as_deref(), Some("expected Bool, found Int"));
    assert_eq!(both[1].label.as_deref(), Some("expected Bool, found {}"));
    // The operator still has type Bool, so nothing cascades.
    assert_eq!(messages("(1 and true) == false"), vec!["mismatched types"]);
    assert_eq!(messages("not not 1"), vec!["mismatched types"]);
    assert_eq!(
        messages("(1.0 // 2) or not (1.0 // 2)"),
        vec!["cannot floor-divide Float", "cannot floor-divide Float"]
    );
}

#[test]
fn if_conditions_are_bool() {
    let diagnostic = &diagnostics("if 1 { 2 } else { 3 }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Bool, found Int")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 2), "`if` conditions are Bool")]
    );
    // The `if` still has its branches' type, so nothing cascades.
    assert_eq!(
        messages("let x = if \"a\" { 2 } else { 3 }\nx + 1"),
        vec!["mismatched types"]
    );
    assert_eq!(result_type("if 1 < 2 { 1.5 } else { 2.5 }"), TyId::FLOAT);
}

#[test]
fn if_branches_must_agree() {
    assert_eq!(
        result_type("if true { \"a\" } else { \"b\" }"),
        TyId::STRING
    );
    assert_eq!(result_type("if true { true } else { false }"), TyId::BOOL);
    assert_eq!(result_type("if true {} else {}"), TyId::EMPTY);
    assert_eq!(result_type("if true { 1.5 } else { 2.5 }"), TyId::FLOAT);
    let diagnostic = &warnings("if true { 1 } else { \"a\" }")[0];
    assert_eq!(
        diagnostic.message,
        "the branches of this `if` have different types"
    );
    assert_eq!(diagnostic.span, Span::new(21, 24));
    assert_eq!(diagnostic.label.as_deref(), Some("this is String"));
    assert_eq!(labels(diagnostic), vec![(Span::new(10, 11), "this is Int")]);
    assert_eq!(
        warnings("if true { 1.5 } else { 7 // 2 }")[0].message,
        "the branches of this `if` have different types"
    );
    assert_eq!(
        warnings("if true {} else { 1 }")[0].message,
        "the branches of this `if` have different types"
    );
    // In an `else if`, the warning is on the nested `if`.
    let diagnostic = &warnings("if true { 1.5 } else if false { true } else { false }")[0];
    assert_eq!(diagnostic.span, Span::new(21, 53));
    assert_eq!(diagnostic.label.as_deref(), Some("this is Bool"));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(10, 13), "this is Float")]
    );
    // The union is misused, not the `if`.
    assert_eq!(
        messages("let x = if true { 1 } else { \"a\" }\nx + 1"),
        vec![
            "the branches of this `if` have different types",
            "cannot add `Int | String` and Int"
        ]
    );
    assert_eq!(
        messages("if true { 1.0 // 2 } else { \"a\" }"),
        vec!["cannot floor-divide Float"]
    );
}

#[test]
fn untyped_if_branches_adapt_and_default() {
    use TyId as T;
    // Allocation order: condition, then's literal and block, else's
    // literal and block, the `if`.
    assert_eq!(
        all_types("if true { 1 } else { 2.0 }"),
        vec![T::BOOL, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT]
    );
    assert_eq!(
        all_types("if true { 1.0 } else { 2 }"),
        vec![T::BOOL, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT, T::FLOAT]
    );
    // Two untyped branches stay untyped until the context decides...
    assert_eq!(
        all_types("if true { 1 } else { 2 } : Float"),
        vec![
            T::BOOL,
            T::FLOAT,
            T::FLOAT,
            T::FLOAT,
            T::FLOAT,
            T::FLOAT,
            T::FLOAT
        ]
    );
    assert_eq!(result_type("(if true { 1 } else { 2 }) * 0.5"), TyId::FLOAT);
    assert_eq!(
        result_type("let x: Float = if true { 1 } else if false { 2 } else { 3 }\nx"),
        TyId::FLOAT
    );
    // ...or default to Int.
    assert_eq!(
        all_types("if true { 1 } else { 2 }"),
        vec![T::BOOL, T::INT, T::INT, T::INT, T::INT, T::INT]
    );
    assert_eq!(
        result_type("let x = if true { 1 } else { 2 }\nx"),
        TyId::INT
    );
    assert_eq!(
        messages("let x = if true { 1 } else { 2 }\nx + 0.5"),
        vec!["cannot add Int and Float"]
    );
    // A concrete Int branch decides for an untyped one.
    assert_eq!(
        messages("(if true { 1 } else { 7 // 2 }) + 0.5"),
        vec!["cannot add Int and Float"]
    );
}

#[test]
fn an_if_without_else_is_the_empty_record() {
    assert_eq!(result_type("if true {}"), TyId::EMPTY);
    assert_eq!(result_type("if true { let x = 1 }"), TyId::EMPTY);
    assert_eq!(
        result_type("let x = 1\nif x > 0 { let y = x }"),
        TyId::EMPTY
    );
    let diagnostic = &diagnostics("if true { 1 }")[0];
    assert_eq!(diagnostic.message, "`if` without `else` must be `{}`");
    assert_eq!(diagnostic.span, Span::new(0, 2));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(8, 13), "this branch is Int")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["add an `else` branch, or make the block's result `{}`"]
    );
    assert_eq!(
        labels(&diagnostics("if true { \"a\" }")[0]),
        vec![(Span::new(8, 15), "this branch is String")]
    );
    // A record with fields is not `{}`.
    assert_eq!(
        labels(&diagnostics("if true { { x = 1 } }")[0]),
        vec![(Span::new(8, 21), "this branch is {x: Int}")]
    );
    // Not reported again as unused or by an enclosing operator.
    assert_eq!(
        messages("if true { 1 }\n2"),
        vec!["`if` without `else` must be `{}`"]
    );
    assert_eq!(
        messages("(if true { 1.5 }) + 1"),
        vec!["`if` without `else` must be `{}`"]
    );
}

#[test]
fn interpolating_every_type() {
    assert_eq!(
        result_type("\"{1} {1.5} {true} {\"s\"} { {} } {1 == 1} { { x = 1 } }\""),
        TyId::STRING
    );
    assert_eq!(
        result_type(&format!("{PRELUDE}\"{{i}}{{f}}{{b}}{{s}}{{u}}\"")),
        TyId::STRING
    );
    // An untyped expression settles to Int.
    assert_eq!(all_types("\"{1}\""), vec![TyId::INT, TyId::STRING]);
    assert_eq!(
        all_types("\"{1 + 2}\""),
        vec![TyId::INT, TyId::INT, TyId::INT, TyId::STRING]
    );
    assert_eq!(result_type("\"{ \"{1}\" }\""), TyId::STRING);
    // An erroneous part is reported once.
    assert_eq!(
        messages("\"{1.0 // 2}\" + \"a\""),
        vec!["cannot floor-divide Float"]
    );
    assert_eq!(messages("\"{x}\""), vec!["undefined variable `x`"]);
}

#[test]
fn strings_concatenate_with_strings_only() {
    assert_eq!(result_type("\"a\" + \"b\""), TyId::STRING);
    assert_eq!(result_type("\"a\" + \"{1}\" + r\"c\""), TyId::STRING);
    let diagnostic = &diagnostics("\"a\" + 1")[0];
    assert_eq!(diagnostic.message, "cannot add String and Int");
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 3), "String"), (Span::new(6, 7), "Int")]
    );
    assert_eq!(messages("1.5 + \"a\""), vec!["cannot add Float and String"]);
    assert_eq!(
        messages("\"a\" - \"b\""),
        vec!["cannot subtract String and String"]
    );
    assert_eq!(
        messages("\"a\" * 2"),
        vec!["cannot multiply String and Int"]
    );
    assert_eq!(messages("\"a\" / 2"), vec!["cannot divide String and Int"]);
    assert_eq!(messages("\"a\" // 2"), vec!["cannot floor-divide String"]);
    assert_eq!(messages("7 % \"a\""), vec!["cannot take modulo of String"]);
    assert_eq!(messages("-\"a\""), vec!["cannot negate String"]);
}

#[test]
fn bools_are_not_numbers() {
    assert_eq!(messages("true + 1"), vec!["cannot add Bool and Int"]);
    assert_eq!(
        messages("2.5 / false"),
        vec!["cannot divide Float and Bool"]
    );
    assert_eq!(messages("true // 2"), vec!["cannot floor-divide Bool"]);
    assert_eq!(messages("-true"), vec!["cannot negate Bool"]);
}

#[test]
fn bool_and_string_annotations_and_ascriptions() {
    assert_eq!(result_type("let x: Bool = true\nx"), TyId::BOOL);
    assert_eq!(result_type("let x: String = \"a\"\nx"), TyId::STRING);
    assert_eq!(result_type("true : Bool"), TyId::BOOL);
    assert_eq!(result_type("\"a\" : String"), TyId::STRING);
    assert_eq!(result_type("(1 < 2) : Bool"), TyId::BOOL);
    assert_eq!(
        diagnostics("let x: Bool = 1")[0].label.as_deref(),
        Some("expected Bool, found Int")
    );
    assert_eq!(
        diagnostics("let x: String = true")[0].label.as_deref(),
        Some("expected String, found Bool")
    );
    assert_eq!(
        diagnostics("\"a\" : Int")[0].label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        diagnostics("1 : String")[0].label.as_deref(),
        Some("expected String, found Int")
    );
    // The names are types, not keywords: they can still name variables.
    assert_eq!(result_type("let String = 1\nString"), TyId::INT);
}

#[test]
fn new_types_never_leave_untyped_or_error_entries() {
    for text in [
        "\"a{1}b{2.5}\"",
        "if 1 < 2 { 1 } else { 2 }",
        "if true { 1 } else if false { 2 } else { 3 } + 0.5",
        "let x = 1 == 1 and not false\nx",
        "if true {}",
    ] {
        for ty in all_types(text) {
            assert!(
                ty != TyId::ERROR && ty != TyId::UNTYPED_INT,
                "{text}: {ty:?}"
            );
        }
    }
}

#[test]
fn checking_continues_from_an_existing_scope() {
    let mut names = Names::new();
    let mut types = TypeTable::new();
    let parse_with = |text: &str, names: &mut Names| {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, strings) = tokenize_with(&source, names).unwrap();
        parse(&tokens, names.clone(), strings, &source).unwrap()
    };
    let first = parse_with("let a = 1\nlet b = 2.5", &mut names);
    let scope = Checker::new(&first, &mut types).check().unwrap().scope;
    assert_eq!(scope.len(), 2);

    let second = parse_with("a := a + 1\nlet c = b * 2.0\n= a", &mut names);
    let output = Checker::with_scope(&second, scope.clone(), &mut types)
        .check()
        .unwrap();
    assert_eq!(output.scope.len(), 4);
    assert_eq!(output.ty, TyId::INT);
    let checked = output.checked;
    assert_eq!(checked.types.get(result_expr(&second)), TyId::INT);
    let resolution = &checked.resolution;
    let items = second.items(second.root());
    assert_eq!(resolution.item_slot(items[0]), Slot(2));
    assert_eq!(resolution.item_slot(items[1]), Slot(3));
    assert_eq!(resolution.frame_size(), 4);
    let slot_names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| second.names().get(name))
        .collect();
    assert_eq!(slot_names, vec!["a", "b", "a", "c"]);

    // A variable from an earlier input is in the enclosing block of an
    // inner one, with no item to point at.
    let third = parse_with("{ b := 1.5 }", &mut names);
    let diagnostics = Checker::with_scope(&third, scope, &mut types)
        .check()
        .unwrap_err();
    assert_eq!(
        diagnostics[0].message,
        "cannot rebind `b` here: it is declared in an enclosing block"
    );
    assert!(diagnostics[0].labels.is_empty());

    // A field declared at the top level of an input is a field in later
    // ones too, whose type is in the shared table.
    let fourth = parse_with("x = { n = 1 }", &mut names);
    let output = Checker::new(&fourth, &mut types).check().unwrap();
    assert_eq!(
        types.display(output.ty, &names).to_string(),
        "{x: {n: Int}}"
    );
    let fifth = parse_with("x.n + 1", &mut names);
    let sixth = parse_with("x := 2", &mut names);
    assert_eq!(
        Checker::with_scope(&fifth, output.scope.clone(), &mut types)
            .check()
            .unwrap()
            .ty,
        TyId::INT
    );
    let diagnostics = Checker::with_scope(&sixth, output.scope, &mut types)
        .check()
        .unwrap_err();
    assert_eq!(
        diagnostics[0].message,
        "cannot rebind `x`: it is a field, not a variable"
    );
    assert!(diagnostics[0].labels.is_empty());
}

// ----- Records -----

#[test]
fn record_blocks_have_record_types() {
    assert_eq!(shown_result("{ x = 1; y = \"a\" }"), "{x: Int, y: String}");
    assert_eq!(file_type("x = 1\ny = \"a\""), "{x: Int, y: String}");
    assert_eq!(shown_result("{ b = 1; a = 2.5 }"), "{a: Float, b: Int}");
    assert_eq!(
        shown_result("{ a = { n = 1.5 }; b = {} }"),
        "{a: {n: Float}, b: {}}"
    );
    assert_eq!(shown_result("{ \"a b\" = 1 }"), "{\"a b\": Int}");
    // The same fields in another order are the same type.
    assert_eq!(
        result_type("{ x = 1; y = 2 }"),
        result_type("{ y = 2; x = 1 }")
    );
    // A field's untyped initialiser settles to Int, as a `let`'s does.
    assert_eq!(all_types("{ x = 1 }")[0], TyId::INT);
}

#[test]
fn fields_are_in_scope_like_variables() {
    // Visible to later items of the block.
    assert_eq!(shown_result("{ x = 1; y = x + 1 }"), "{x: Int, y: Int}");
    // `let`s are private to the block, not fields.
    assert_eq!(
        shown_result("{ x = 1.5; let t = x * 2.0; y = t }"),
        "{x: Float, y: Float}"
    );
    // A field shadows an earlier binding, and a later `let` shadows a
    // field without changing it.
    assert_eq!(
        shown_result("let x = \"s\"\n{ x = 1; y = x + 1 }"),
        "{x: Int, y: Int}"
    );
    assert_eq!(
        shown_result("{ x = 1; let x = \"s\"; y = x }"),
        "{x: Int, y: String}"
    );
    // Not visible before its item, nor after its block.
    assert_eq!(messages("{ y = x; x = 1 }"), vec!["undefined variable `x`"]);
    assert_eq!(
        messages("let r = { x = 1 }\nx"),
        vec!["undefined variable `x`"]
    );
    // Statements of type `{}` are allowed, the last one included.
    assert_eq!(shown_result("{ x = 1; {}; if true {} }"), "{x: Int}");
}

#[test]
fn a_record_block_has_no_result() {
    let diagnostic = &diagnostics("{ x = 1; = 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "a block with fields cannot have a result"
    );
    assert_eq!(diagnostic.span, Span::new(9, 10));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 7), "first field declared here")]
    );
    assert_eq!(
        diagnostic.notes,
        vec![
            "a block is either a record of its fields or a single value; bind the value \
                 with `let` or make it a field"
        ]
    );
    // Wherever the result is, it is this error, not the ordering one.
    assert_eq!(
        labels(&diagnostics("{ = 2; x = 1 }")[0]),
        vec![(Span::new(7, 12), "first field declared here")]
    );
    assert_eq!(
        messages("{ = 2; x = 1 }"),
        vec!["a block with fields cannot have a result"]
    );
    // A final expression is a statement, so a value there is unused.
    let diagnostic = &diagnostics("{ x = 1; x + 1 }")[0];
    assert_eq!(diagnostic.message, "unused value of type Int");
    assert_eq!(diagnostic.span, Span::new(9, 14));
    assert_eq!(messages("x = 1\n2.5"), vec!["unused value of type Float"]);
}

#[test]
fn rebinding_a_field_is_an_error() {
    let diagnostic = &diagnostics("{ x = 1; x := 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `x`: it is a field, not a variable"
    );
    assert_eq!(diagnostic.span, Span::new(9, 10));
    assert_eq!(labels(diagnostic), vec![(Span::new(2, 7), "declared here")]);
    assert_eq!(
        diagnostic.notes,
        vec!["rebind with `x = ...` is not allowed either: a field is defined once"]
    );
    // A field of an enclosing block, too.
    assert_eq!(
        messages("{ x = 1; y = { x := 2 } }"),
        vec!["cannot rebind `x`: it is a field, not a variable"]
    );
    // A field declared with a quoted name is the same name. At the top
    // level, the field is a field of the file, which `:=` cannot name.
    assert_eq!(
        messages("\"x\" = 1\nx := 2"),
        vec!["`x` is a field of this file"]
    );
    // The rejected rebind still declares a variable, so uses do not
    // cascade.
    assert_eq!(
        messages("{ x = 1; x := 2.5; y = x * 1.5 }"),
        vec!["cannot rebind `x`: it is a field, not a variable"]
    );
}

#[test]
fn a_field_defined_twice_is_an_error() {
    let diagnostic = &diagnostics("{ x = 1; x = 2 }")[0];
    assert_eq!(diagnostic.message, "field `x` is defined twice");
    assert_eq!(diagnostic.span, Span::new(9, 10));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 3), "first defined here")]
    );
    // Each repeat is reported against the first.
    let repeats = diagnostics("{ x = 1; y = 2; x = 3; x = 4 }");
    assert_eq!(repeats.len(), 2);
    assert_eq!(repeats[0].span, Span::new(16, 17));
    assert_eq!(repeats[1].span, Span::new(23, 24));
    for repeat in &repeats {
        assert_eq!(
            labels(repeat),
            vec![(Span::new(2, 3), "first defined here")]
        );
    }
    // A quoted name that is the same name.
    assert_eq!(
        errors("{ x = 1; \"x\" = 2 }"),
        vec![("field `x` is defined twice".to_owned(), Span::new(9, 12))]
    );
    assert_eq!(
        messages("{ \"a b\" = 1; \"a b\" = 2 }"),
        vec!["field `\"a b\"` is defined twice"]
    );
    // The record is an error, so it does not cascade.
    assert_eq!(
        messages("let r = { x = 1; x = 2 }\nr.x + 1.5\nr.y"),
        vec!["field `x` is defined twice"]
    );
}

#[test]
fn field_access() {
    assert_eq!(
        result_type("let p = { x = 1; y = \"a\" }\np.y"),
        TyId::STRING
    );
    assert_eq!(
        result_type("let p = { x = 1; y = \"a\" }\np.x + 1"),
        TyId::INT
    );
    assert_eq!(result_type("{ a = { n = 1.5 } }.a.n"), TyId::FLOAT);
    assert_eq!(result_type("{ \"a b\" = true }.\"a b\""), TyId::BOOL);
    assert_eq!(result_type("-{ x = 2 }.x"), TyId::INT);
    assert_eq!(shown_result("{ a = { n = 1 } }.a"), "{n: Int}");
}

#[test]
fn field_access_errors() {
    let diagnostic = &diagnostics("let p = { x = 1; y = 2 }\np.z")[0];
    assert_eq!(diagnostic.message, "no field `z` in `{x: Int, y: Int}`");
    assert_eq!(diagnostic.span, Span::new(26, 28));
    assert!(diagnostic.labels.is_empty());
    assert_eq!(diagnostic.notes, vec!["the fields are `x` and `y`"]);
    let diagnostic = &diagnostics("{}.x")[0];
    assert_eq!(diagnostic.message, "no field `x` in `{}`");
    assert_eq!(diagnostic.span, Span::new(2, 4));
    assert_eq!(diagnostic.notes, vec!["the record has no fields"]);
    assert_eq!(
        diagnostics("{ x = 1 }.y")[0].notes,
        vec!["the only field is `x`"]
    );
    assert_eq!(
        diagnostics("{ c = 1; a = 2; \"b b\" = 3 }.z")[0].notes,
        vec!["the fields are `a`, `\"b b\"` and `c`"]
    );
    assert_eq!(
        messages("{ x = 1 }.\"a b\""),
        vec!["no field `\"a b\"` in `{x: Int}`"]
    );
    // A value that is not a record.
    let diagnostic = &diagnostics("1.x")[0];
    assert_eq!(diagnostic.message, "cannot access field `x` of Int");
    assert_eq!(diagnostic.span, Span::new(1, 3));
    assert_eq!(labels(diagnostic), vec![(Span::new(0, 1), "this is Int")]);
    let diagnostic = &diagnostics("let s = \"s\"\ns . x")[0];
    assert_eq!(diagnostic.message, "cannot access field `x` of String");
    assert_eq!(diagnostic.span, Span::new(13, 17));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(12, 13), "this is String")]
    );
    // An erroneous record is not reported again.
    assert_eq!(messages("q.x"), vec!["undefined variable `q`"]);
    assert_eq!(messages("q.x.y + 1"), vec!["undefined variable `q`"]);
    assert_eq!(
        messages("{ x = 1 }.z.w"),
        vec!["no field `z` in `{x: Int}`"]
    );
}

#[test]
fn a_record_block_is_checked_against_its_annotation() {
    // Literals adapt to the expected field types, nested blocks too,
    // through parentheses, and in ascriptions.
    assert_eq!(all_types("let p: {x: Float} = { x = 1 }")[0], TyId::FLOAT);
    assert_eq!(
        all_types("let p: {a: {n: Float}} = { a = { n = 1 } }")[0],
        TyId::FLOAT
    );
    assert_eq!(
        all_types("let p: {x: Float} = (({ x = 1 }))")[0],
        TyId::FLOAT
    );
    assert_eq!(all_types("{ x = 1 } : {x: Float}")[0], TyId::FLOAT);
    assert_eq!(
        shown_result("let p: {x: Float} = { x = -(1 + 2) }\np"),
        "{x: Float}"
    );
    assert_eq!(shown_result("{ x = 1 } : {x: Float}"), "{x: Float}");
    // Each parenthesis has the block's type.
    let (ast, types, output) = check_text("let p: {x: Float} = ({ x = 1 })");
    let checked = output.unwrap().checked;
    let ItemKind::Let { value, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(
        types
            .display(checked.types.get(value), ast.names())
            .to_string(),
        "{x: Float}"
    );
    // A field can use earlier fields at their expected types.
    assert_eq!(
        shown_result("let p: {x: Float, y: Float} = { x = 1; y = x + 1 }\np"),
        "{x: Float, y: Float}"
    );
    // A mismatch inside is reported where it is.
    let diagnostic = &diagnostics("let p: {x: Int} = { x = \"s\" }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(24, 27));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 15), "expected because of this annotation")]
    );
    // A block without fields, or a record block against a type that is
    // not a record, is checked as a whole.
    assert_eq!(
        diagnostics("let p: {x: Int} = { 1 }")[0].label.as_deref(),
        Some("expected {x: Int}, found Int")
    );
    assert_eq!(
        diagnostics("let p: Int = { x = 1 }")[0].label.as_deref(),
        Some("expected Int, found {x: Int}")
    );
}

#[test]
fn a_missing_field_is_an_error() {
    let diagnostic = &diagnostics("let p: {x: Int, y: Int} = { x = 1 }")[0];
    assert_eq!(diagnostic.message, "missing field `y` in record");
    assert_eq!(diagnostic.span, Span::new(34, 35));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 23), "expected because of this annotation")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["the type `{x: Int, y: Int}` needs it"]
    );
    // Each missing field, in alphabetical order.
    assert_eq!(
        messages("{ c = 1 } : {b: Int, c: Int, a: Int}"),
        vec!["missing field `a` in record", "missing field `b` in record"]
    );
    assert_eq!(
        labels(&diagnostics("{ c = 1 } : {b: Int, c: Int}")[0]),
        vec![(Span::new(12, 28), "ascribed here")]
    );
    // In a nested block, at its own closing brace; the warning about
    // its extra field comes back with it, in source order.
    assert_eq!(
        errors("let p: {a: {n: Int}} = { a = { m = 1 } }"),
        vec![
            (
                "field `m` is not in `{n: Int}`".to_owned(),
                Span::new(31, 32)
            ),
            ("missing field `n` in record".to_owned(), Span::new(37, 38))
        ]
    );
    // `{}` has no fields, so it is not a record block: it is checked as
    // a whole.
    assert_eq!(
        diagnostics("let p: {a: {n: Int}} = { a = {} }")[0]
            .label
            .as_deref(),
        Some("expected {n: Int}, found {}")
    );
    // It does not cascade.
    assert_eq!(
        messages("let p: {x: Int, y: Int} = { x = 1 }\np.y + 1.5"),
        vec!["missing field `y` in record"]
    );
}

#[test]
fn an_extra_field_is_a_warning() {
    let text = "let p: {x: Int} = { x = 1; z = 2 }\np";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    let warning = &found[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.message, "field `z` is not in `{x: Int}`");
    assert_eq!(warning.span, Span::new(27, 28));
    assert_eq!(
        labels(warning),
        vec![(Span::new(7, 15), "expected because of this annotation")]
    );
    assert_eq!(
        warning.notes,
        vec!["it is allowed, but nothing with this type can see it"]
    );
    // The check succeeds, and the variable has the annotated type.
    assert_eq!(shown_result(text), "{x: Int}");
    assert_eq!(
        messages("let p: {x: Int} = { x = 1; z = 2 }\np.z"),
        vec![
            "field `z` is not in `{x: Int}`",
            "no field `z` in `{x: Int}`"
        ]
    );
    // In an ascription, against `{}`, and in a nested block.
    assert_eq!(
        labels(&warnings("{ x = 1; z = 2 } : {x: Int}")[0]),
        vec![(Span::new(19, 27), "ascribed here")]
    );
    assert_eq!(
        warnings("let e: {} = { x = 1 }")[0].message,
        "field `x` is not in `{}`"
    );
    assert_eq!(
        warnings("let p: {a: {n: Int}} = { a = { n = 1; m = 2 } }")[0].message,
        "field `m` is not in `{n: Int}`"
    );
    // With errors, the warnings come back with them in source order.
    let diagnostics = diagnostics("let p: {x: Int} = { x = 1; z = 2 }\nq");
    let shown: Vec<(Severity, &str)> = diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.severity, diagnostic.message.as_str()))
        .collect();
    assert_eq!(
        shown,
        vec![
            (Severity::Warning, "field `z` is not in `{x: Int}`"),
            (Severity::Error, "undefined variable `q`")
        ]
    );
}

#[test]
fn subtyping_through_a_variable_warns_about_nothing() {
    let text = "let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\np";
    assert!(warnings(text).is_empty());
    assert_eq!(shown_result(text), "{x: Int}");
    let text = "let q = { a = { n = 1; m = 2 }; b = 1 }\nlet p: {a: {n: Int}} = q\np";
    assert!(warnings(text).is_empty());
    assert_eq!(shown_result(text), "{a: {n: Int}}");
    assert!(warnings("let q = { x = 1 }\nq : {}").is_empty());
    // A block in parentheses inside another expression is not checked
    // field by field.
    assert!(
        warnings("let p: {x: Int} = if true { { x = 1; y = 2 } } else { { x = 2 } }").is_empty()
    );
    // A record that lacks a field is not a subtype.
    let diagnostic = &diagnostics("let q = { y = 1 }\nlet p: {x: Int} = q")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected {x: Int}, found {y: Int}")
    );
    assert_eq!(
        diagnostics("let q = { x = 1 }\nlet p: {x: Float} = q")[0]
            .label
            .as_deref(),
        Some("expected {x: Float}, found {x: Int}")
    );
}

#[test]
fn if_joins_record_branches() {
    assert_eq!(
        shown_result("if true { { x = 1; y = 2 } } else { { x = 3 } }"),
        "{x: Int}"
    );
    // No shared fields: the empty record, which is still a record.
    assert_eq!(
        shown_result("if true { { x = 1 } } else { { y = 1 } }"),
        "{}"
    );
    assert_eq!(shown_result("if true { {} } else { { x = 1 } }"), "{}");
    // A shared field whose types have no join is kept, with their
    // union.
    assert_eq!(
        shown_result("if true { { x = 1; y = 2 } } else { { x = 1.5; y = 3 } }"),
        "{x: Int | Float, y: Int}"
    );
    // Nested.
    assert_eq!(
        shown_result("if true { { a = { n = 1; m = 2 }; b = 1 } } else { { a = { n = 3 } } }"),
        "{a: {n: Int}}"
    );
    // A record branch and a non-record branch make a union, and a
    // warning.
    let diagnostic = &warnings("if true { { x = 1 } } else { 1 }")[0];
    assert_eq!(
        diagnostic.message,
        "the branches of this `if` have different types"
    );
    assert_eq!(diagnostic.span, Span::new(29, 30));
    assert_eq!(diagnostic.label.as_deref(), Some("this is Int"));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(10, 19), "this is {x: Int}")]
    );
}

#[test]
fn record_equality_needs_identical_types() {
    assert_eq!(result_type("{ x = 1 } == { x = 2 }"), TyId::BOOL);
    assert_eq!(
        result_type("{ x = 1; y = 2 } != { y = 1; x = 2 }"),
        TyId::BOOL
    );
    assert_eq!(result_type("{} == {}"), TyId::BOOL);
    let diagnostic = &diagnostics("{ x = 1 } == { x = 1; y = 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot compare `{x: Int}` and `{x: Int, y: Int}`"
    );
    assert_eq!(diagnostic.span, Span::new(10, 12));
    assert_eq!(
        labels(diagnostic),
        vec![
            (Span::new(0, 9), "{x: Int}"),
            (Span::new(13, 29), "{x: Int, y: Int}")
        ]
    );
    // Untyped literals in record blocks settle to Int.
    assert_eq!(
        messages("{ x = 1 } == { x = 1.5 }"),
        vec!["cannot compare `{x: Int}` and `{x: Float}`"]
    );
    assert_eq!(
        messages("{} != { x = 1 }"),
        vec!["cannot compare `{}` and `{x: Int}`"]
    );
    assert_eq!(
        messages("{ x = 1 } == 1"),
        vec!["cannot compare `{x: Int}` and Int"]
    );
}

#[test]
fn records_cannot_be_ordered() {
    let diagnostic = &diagnostics("{ x = 1 } < { x = 2 }")[0];
    assert_eq!(diagnostic.message, "cannot order `{x: Int}`");
    assert_eq!(
        labels(diagnostic),
        vec![
            (Span::new(0, 9), "{x: Int}"),
            (Span::new(12, 21), "{x: Int}")
        ]
    );
    assert_eq!(messages("{} >= {}"), vec!["cannot order `{}`"]);
    assert_eq!(messages("1 < { x = 1 }"), vec!["cannot order `{x: Int}`"]);
}

#[test]
fn record_type_annotations() {
    assert_eq!(
        shown_result(
            "let p: { \"a b\": String, nested: {n: Int} } = { \"a b\" = \"s\"; nested = { n = 1 } }\np"
        ),
        "{\"a b\": String, nested: {n: Int}}"
    );
    // A name that appears twice in a type.
    assert_eq!(
        errors("1 : {x: Int, x: Int}"),
        vec![(
            "field `x` appears twice in this type".to_owned(),
            Span::new(13, 14)
        )]
    );
    assert_eq!(
        messages("let p: {a: Int, \"a\": Int, a: Int} = { a = 1 }\np.a + 1.5"),
        vec![
            "field `a` appears twice in this type",
            "field `a` appears twice in this type"
        ]
    );
}

// ----- Dotted fields -----

#[test]
fn dotted_fields_merge_into_one_record() {
    assert_eq!(
        file_type("server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true"),
        "{server: {host: String, port: Int, tls: {enabled: Bool}}}"
    );
    assert_eq!(
        shown_result("{ a.b.c = 1; a.b.d = 2.5; a.e = \"s\"; f = 1 }"),
        "{a: {b: {c: Int, d: Float}, e: String}, f: Int}"
    );
    // Quoted segments are names like any other.
    assert_eq!(
        shown_result("{ \"log level\".format = \"json\"; \"log level\".\"to file\" = true }"),
        "{\"log level\": {format: String, \"to file\": Bool}}"
    );
    // Interleaved with other fields and with `let`s.
    assert_eq!(
        shown_result("{ a.x = 1; b = 2; let t = 3; a.y = t }"),
        "{a: {x: Int, y: Int}, b: Int}"
    );
    // The field is one binding, in one slot, whatever extends it.
    let ast = parse_text("a.x = 1\nb = 2\na.y = 3\na.z.w = 4");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    let slots: Vec<Slot> = items.iter().map(|&id| resolution.item_slot(id)).collect();
    assert_eq!(slots, vec![Slot(0), Slot(1), Slot(0), Slot(0)]);
    // And each node below the roots has a cell of its own: `a.x`,
    // `a.y`, `a.z` and `a.z.w`, after the roots.
    assert_eq!(resolution.frame_size(), 6);
    // The first definition builds from nothing; each later one updates
    // the field's slot, through as many segments as exist.
    let paths: Vec<PathResolution> = items.iter().map(|&id| resolution.item_path(id)).collect();
    assert_eq!(
        paths,
        vec![
            PathResolution::NONE,
            PathResolution::NONE,
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 1,
                awaits: PathResolution::NOT_AWAITED
            },
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 1,
                awaits: PathResolution::NOT_AWAITED
            },
        ]
    );
    let ast = parse_text("a.b.c = 1\na.b.d = 2");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    assert_eq!(
        resolution.item_path(ast.items(ast.root())[1]),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 2,
            awaits: PathResolution::NOT_AWAITED
        }
    );
}

#[test]
fn the_type_of_an_open_field_grows_item_by_item() {
    assert_eq!(
        shown_result("{ a.x = 1; let p = a; a.y = \"s\"; let q = a; r = p; s = q }"),
        "{a: {x: Int, y: String}, r: {x: Int}, s: {x: Int, y: String}}"
    );
    // A later definition sees the field as built so far.
    assert_eq!(
        shown_result("{ a.x = 1; a.y = a.x + 1 }"),
        "{a: {x: Int, y: Int}}"
    );
    assert_eq!(
        messages("{ a.x = 1; a.y = a.z }"),
        vec!["no field `z` in `{x: Int}`"]
    );
    // The first definition does not see its own field.
    assert_eq!(messages("{ a.x = a }"), vec!["undefined variable `a`"]);
    // A later `let` shadows the field without changing it.
    assert_eq!(
        shown_result("{ a.x = 1; let a = \"s\"; a.y = a }"),
        "{a: {x: Int, y: String}}"
    );
}

#[test]
fn a_dotted_path_under_a_whole_field_is_an_error() {
    let diagnostic = &diagnostics("{ a = 1; a.b = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a` is already defined");
    assert_eq!(diagnostic.span, Span::new(9, 12));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 3), "defined whole here")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["a field defined as a whole cannot be extended with dotted paths"]
    );
    // Whatever the whole field's type, record or not.
    assert_eq!(
        messages("{ a = { b = 1 }; a.c = 2 }"),
        vec!["field `a` is already defined"]
    );
    // A segment defined whole as a record, by a path's last segment.
    let diagnostic = &diagnostics("{ a.b = { x = 1 }; a.b.y = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a.b` is already defined");
    assert_eq!(diagnostic.span, Span::new(19, 24));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "defined whole here")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["a field defined as a whole cannot be extended with dotted paths"]
    );
    assert_eq!(
        messages("let r = { x = 1 }\n{ a.b = r; a.b.c.d = 1 }"),
        vec!["field `a.b` is already defined"]
    );
    // The rejected path declares nothing, so nothing cascades, but its
    // value is still checked.
    assert_eq!(
        messages("{ a = 1; a.b = 2; c = a + 1 }"),
        vec!["field `a` is already defined"]
    );
    assert_eq!(
        messages("{ a = 1; a.b = 1.0 // 2 }"),
        vec!["field `a` is already defined", "cannot floor-divide Float"]
    );
    // The block's record is erroneous, so nothing outside cascades.
    assert_eq!(
        messages("let r = { a = 1; a.b = 2 }\nr.a + 1.5\nr.q"),
        vec!["field `a` is already defined"]
    );
    assert_eq!(
        messages("let r = { a.b = 1; a.b.c = 2 }\nr.a.b + 1.5"),
        vec!["cannot define `a.b.c`: `a.b` is Int, not a record"]
    );
}

#[test]
fn a_whole_field_after_a_dotted_one_is_defined_twice() {
    let diagnostic = &diagnostics("{ a.b = 1; a = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a` is defined twice");
    assert_eq!(diagnostic.span, Span::new(11, 12));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 3), "first defined here")]
    );
    // Later dotted paths still extend the first, dotted, field.
    assert_eq!(
        messages("{ a.b = 1; a = 2; a.c = 3 }"),
        vec!["field `a` is defined twice"]
    );
}

#[test]
fn a_segment_that_is_not_a_record_cannot_hold_fields() {
    let diagnostic = &diagnostics("{ a.b = 1; a.b.c = 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot define `a.b.c`: `a.b` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(11, 16));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "`a.b` defined here")]
    );
    assert!(diagnostic.notes.is_empty());
    // Deeper, with another type, and quoted.
    assert_eq!(
        messages("{ a.b.c = \"s\"; a.b.c.d.e = 1 }"),
        vec!["cannot define `a.b.c.d.e`: `a.b.c` is String, not a record"]
    );
    assert_eq!(
        messages("{ a.\"b c\" = true; a.\"b c\".d = 1 }"),
        vec!["cannot define `a.\"b c\".d`: `a.\"b c\"` is Bool, not a record"]
    );
    // An erroneous segment has been reported already.
    assert_eq!(
        messages("{ a.b = 1.0 // 2; a.b.c = 1 }"),
        vec!["cannot floor-divide Float"]
    );
}

#[test]
fn a_dotted_path_defined_twice_is_an_error() {
    let diagnostic = &diagnostics("{ a.b = 1; a.b = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a.b` is defined twice");
    assert_eq!(diagnostic.span, Span::new(11, 14));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "first defined here")]
    );
    // A record a longer path created is defined too, where that path
    // wrote it.
    let diagnostic = &diagnostics("{ a.b.c = 1; a.b = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a.b` is defined twice");
    assert_eq!(diagnostic.span, Span::new(13, 16));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "first defined here")]
    );
    // A quoted name is the same name.
    assert_eq!(
        messages("{ \"a b\".c = 1; \"a b\".\"c\" = 2 }"),
        vec!["field `\"a b\".c` is defined twice"]
    );
    // The record type does not cascade.
    assert_eq!(
        messages("let r = { a.b = 1; a.b = 2 }\nr.a.b + 1.5"),
        vec!["field `a.b` is defined twice"]
    );
}

#[test]
fn dotted_fields_are_checked_against_the_expected_type() {
    // The leaf is checked against the type at its full path.
    assert_eq!(
        all_types("let p: {a: {b: Float}} = { a.b = 1 }")[0],
        TyId::FLOAT
    );
    assert_eq!(
        shown_result("let p: {a: {b: Float, c: {d: Float}}} = { a.b = 1; a.c.d = 2 }\np"),
        "{a: {b: Float, c: {d: Float}}}"
    );
    assert!(warnings("let p: {a: {b: Int}} = { a.b = 1 }\np").is_empty());
    // Record blocks at the leaf are checked field by field.
    assert_eq!(
        all_types("let p: {a: {b: {n: Float}}} = { a.b = { n = 1 } }")[0],
        TyId::FLOAT
    );
    let diagnostic = &diagnostics("let p: {a: {b: Int}} = { a.b = \"s\" }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(31, 34));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 20), "expected because of this annotation")]
    );
    // In an ascription too.
    assert_eq!(all_types("{ a.b = 1 } : {a: {b: Float}}")[0], TyId::FLOAT);
}

#[test]
fn a_missing_nested_field_of_a_dotted_field_is_an_error() {
    let text = "let p: {a: {b: Int, c: Int}} = { a.b = 1 }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "missing field `a.c` in record");
    assert_eq!(diagnostic.span, Span::new(41, 42));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 28), "expected because of this annotation")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["the type `{b: Int, c: Int}` needs it"]
    );
    // At every level, each in alphabetical order, and alongside the
    // block's own missing fields.
    assert_eq!(
        messages("let p: {a: {b: {c: Int, d: Int, e: Int}}, z: Int} = { a.b.d = 1 }"),
        vec![
            "missing field `z` in record",
            "missing field `a.b.c` in record",
            "missing field `a.b.e` in record"
        ]
    );
    // It does not cascade.
    assert_eq!(
        messages("let p: {a: {b: Int, c: Int}} = { a.b = 1 }\np.a.c + 1.5"),
        vec!["missing field `a.c` in record"]
    );
}

#[test]
fn a_dotted_field_the_expected_type_lacks_is_a_warning() {
    let text = "let p: {a: {b: Int}} = { a.b = 1; a.c = 2 }\np";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    let warning = &found[0];
    assert_eq!(warning.message, "field `c` is not in `{b: Int}`");
    assert_eq!(warning.span, Span::new(36, 37));
    assert_eq!(
        labels(warning),
        vec![(Span::new(7, 20), "expected because of this annotation")]
    );
    assert_eq!(
        warning.notes,
        vec!["it is allowed, but nothing with this type can see it"]
    );
    assert_eq!(shown_result(text), "{a: {b: Int}}");
    // For the first missing segment only, once per such field.
    let found = warnings("let p: {a: Int} = { a = 1; x.p = 1; x.q.r = 2; y.s.t = 3 }");
    let shown: Vec<(&str, Span)> = found
        .iter()
        .map(|warning| (warning.message.as_str(), warning.span))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("field `x` is not in `{a: Int}`", Span::new(27, 28)),
            ("field `y` is not in `{a: Int}`", Span::new(47, 48))
        ]
    );
    let found = warnings("let p: {a: {b: Int}} = { a.b = 1; a.c.d = 2; a.c.e = 3 }");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `c` is not in `{b: Int}`");
}

#[test]
fn a_dotted_field_where_the_expected_type_is_not_a_record_mismatches() {
    let diagnostic = &diagnostics("let p: {a: Int} = { a.b = 1 }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(20, 21));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found {b: Int}")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 15), "expected because of this annotation")]
    );
    // Nested, at the segment's first definition, and without warnings
    // about the segments below it.
    let text = "let p: {a: {b: String}} = { a.b.c = 1; a.b.d = 2 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].span, Span::new(28, 31));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected String, found {c: Int, d: Int}")
    );
}

// ----- Dotted `let` and `:=` -----

#[test]
fn a_dotted_let_without_a_variable_builds_nested_records() {
    assert_eq!(shown_result("let a.b.c = 1\na"), "{b: {c: Int}}");
    assert_eq!(shown_result("let a.b = \"s\"\na.b"), "String");
    // An annotation applies to the leaf.
    assert_eq!(shown_result("let a.b: Float = 5\na"), "{b: Float}");
    assert_eq!(all_types("let a.b: Float = 5")[0], TyId::FLOAT);
    assert_eq!(
        diagnostics("let a.b: Int = 1.5")[0].label.as_deref(),
        Some("expected Int, found Float")
    );
    let ast = parse_text("let a.b.c = 1");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    assert_eq!(
        resolution.item_path(ast.items(ast.root())[0]),
        PathResolution::NONE
    );
}

#[test]
fn a_dotted_let_updates_a_record_variable() {
    // A field added.
    assert_eq!(
        shown_result("let a = { x = 1 }\nlet a.y = \"s\"\na"),
        "{x: Int, y: String}"
    );
    // Intermediate records created, and updated inside.
    assert_eq!(
        shown_result("let a = { x = 1 }\nlet a.b.c = true\na"),
        "{b: {c: Bool}, x: Int}"
    );
    assert_eq!(
        shown_result("let a = { b = { c = 1; d = 2 } }\nlet a.b.e = 3\na"),
        "{b: {c: Int, d: Int, e: Int}}"
    );
    // A leaf replaced, its type changing.
    assert_eq!(
        shown_result("let a = { b = { c = 1; d = 2 } }\nlet a.b.c = \"s\"\na"),
        "{b: {c: String, d: Int}}"
    );
    assert_eq!(
        shown_result("let a = { b = { c = 1 } }\nlet a.b = 2.5\na"),
        "{b: Float}"
    );
    // Through the static type: a field hidden by subtyping is created
    // anew.
    assert_eq!(
        shown_result("let p: {x: Int} = { x = 1; y = 2 }\nlet p.y.z = 3\np"),
        "{x: Int, y: {z: Int}}"
    );
    // How far into the path the old record reaches, for lowering.
    let ast = parse_text("let a = { b = { c = 1 } }\nlet a.b.c = 2\nlet a.x.y.z = 3");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    assert_eq!(
        resolution.item_path(items[1]),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 2,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    assert_eq!(
        resolution.item_path(items[2]),
        PathResolution {
            source: VarRef::Local(Slot(1)),
            existing: 1,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    // The update is a new binding in a new slot.
    assert_eq!(resolution.item_slot(items[1]), Slot(1));
    assert_eq!(resolution.item_slot(items[2]), Slot(2));
}

#[test]
fn a_dotted_let_leaves_the_old_binding_unchanged() {
    let text = "let a = { x = 1 }\nlet old = a\nlet a.x = \"s\"\nold";
    assert_eq!(shown_result(text), "{x: Int}");
    let text = "let a = { x = 1 }\nlet old = a\nlet a.x = \"s\"\na";
    assert_eq!(shown_result(text), "{x: String}");
    // In an inner block, the outer variable is shadowed only there.
    assert_eq!(
        shown_result("let a = { x = 1 }\n{ let a.y = 2; a }"),
        "{x: Int, y: Int}"
    );
    assert_eq!(
        shown_result("let a = { x = 1 }\n{ let a.y = 2 }\na"),
        "{x: Int}"
    );
}

#[test]
fn a_dotted_let_needs_records_along_its_path() {
    let diagnostic = &diagnostics("let a = 1\nlet a.b = 2")[0];
    assert_eq!(
        diagnostic.message,
        "cannot update `a.b`: `a` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(14, 17));
    assert_eq!(labels(diagnostic), vec![(Span::new(14, 15), "this is Int")]);
    let diagnostic = &diagnostics("let a = { b = 1 }\nlet a.b.c = 2")[0];
    assert_eq!(
        diagnostic.message,
        "cannot update `a.b.c`: `a.b` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(22, 27));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(22, 23), "this is {b: Int}")]
    );
    assert_eq!(
        messages("let a = { b = { c = \"s\" } }\nlet a.b.c.d.e = 2"),
        vec!["cannot update `a.b.c.d.e`: `a.b.c` is String, not a record"]
    );
    // The variable is declared as an error, so nothing cascades; the
    // value is still checked.
    assert_eq!(
        messages("let a = 1\nlet a.b = 2\na.b + 1.5"),
        vec!["cannot update `a.b`: `a` is Int, not a record"]
    );
    assert_eq!(
        messages("let a = true\nlet a.b = 1.0 // 2"),
        vec![
            "cannot update `a.b`: `a` is Bool, not a record",
            "cannot floor-divide Float"
        ]
    );
    // An erroneous variable has been reported already.
    assert_eq!(
        messages("let a = 1.0 // 2\nlet a.b = 1"),
        vec!["cannot floor-divide Float"]
    );
}

#[test]
fn a_dotted_rebind_updates_a_variable_of_its_block() {
    assert_eq!(shown_result("let a = { x = 1 }\na.x := 2\na"), "{x: Int}");
    assert_eq!(
        shown_result("let a = { x = 1; y = { z = \"s\" } }\na.y.z := \"t\"\na"),
        "{x: Int, y: {z: String}}"
    );
    // A new binding, like any `:=`.
    let ast = parse_text("let a = { x = 1 }\na.x := 2");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[1]), Slot(1));
    assert_eq!(
        resolution.item_path(items[1]),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 1,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    let text = "let a = { x = 1 }\nlet old = a\na.x := 2\nold";
    assert_eq!(shown_result(text), "{x: Int}");
}

#[test]
fn a_dotted_rebind_follows_the_rebind_rules() {
    let diagnostic = &diagnostics("a.x := 1")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a`: no variable named `a` is declared"
    );
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert_eq!(diagnostic.notes, vec!["declare it with `let a = ...`"]);
    // Declared anyway, so uses do not cascade.
    assert_eq!(
        messages("a.x := 1\na.x + 1.5"),
        vec![
            "cannot rebind `a`: no variable named `a` is declared",
            "cannot add Int and Float"
        ]
    );
    let diagnostic = &diagnostics("{ a = { x = 1 }; a.x := 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a`: it is a field, not a variable"
    );
    assert_eq!(diagnostic.span, Span::new(17, 18));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 15), "declared here")]
    );
    let diagnostic = &diagnostics("let a = { x = 1 }\n{ a.x := 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a` here: it is declared in an enclosing block"
    );
    assert_eq!(diagnostic.span, Span::new(20, 21));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 17), "declared here")]
    );
    // And the update rules.
    let diagnostic = &diagnostics("let a = 1\na.b := 2")[0];
    assert_eq!(
        diagnostic.message,
        "cannot update `a.b`: `a` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(10, 13));
    assert_eq!(labels(diagnostic), vec![(Span::new(10, 11), "this is Int")]);
    assert_eq!(
        messages("let a = { b = 1 }\na.b.c := 2"),
        vec!["cannot update `a.b.c`: `a.b` is Int, not a record"]
    );
}

// ----- Total equality -----

#[test]
fn total_equality_needs_identical_types() {
    for op in ["===", "!=="] {
        assert_eq!(
            result_type(&format!("{{ x = 1 }} {op} {{ x = 2 }}")),
            TyId::BOOL
        );
        assert_eq!(result_type(&format!("1 {op} 2")), TyId::BOOL);
        assert_eq!(result_type(&format!("\"a\" {op} \"b\"")), TyId::BOOL);
        assert_eq!(result_type(&format!("true {op} false")), TyId::BOOL);
        assert_eq!(result_type(&format!("{{}} {op} {{}}")), TyId::BOOL);
        // Untyped literals adapt as for `==`.
        assert_eq!(
            all_types(&format!("1 {op} 2.5")),
            vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
        );
        let diagnostic = &diagnostics(&format!("{{ x = 1 }} {op} {{ x = 1; y = 2 }}"))[0];
        assert_eq!(
            diagnostic.message,
            "cannot compare `{x: Int}` and `{x: Int, y: Int}`"
        );
        assert_eq!(diagnostic.span, Span::new(10, 13));
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(0, 9), "{x: Int}"),
                (Span::new(14, 30), "{x: Int, y: Int}")
            ]
        );
        assert_eq!(
            messages(&format!("true {op} 1")),
            vec!["cannot compare Bool and Int"]
        );
    }
    // Every pair of types, as for `==`.
    for (lhs, lhs_ty, lhs_shown) in VARIABLES {
        for (rhs, rhs_ty, rhs_shown) in VARIABLES {
            let text = format!("{PRELUDE}{lhs} === {rhs}");
            if lhs_ty == rhs_ty {
                assert_eq!(result_type(&text), TyId::BOOL, "{text}");
            } else {
                assert_eq!(
                    messages(&text),
                    vec![format!("cannot compare {lhs_shown} and {rhs_shown}")],
                    "{text}"
                );
            }
        }
    }
    // A narrowed variable compares with values of its static type.
    assert_eq!(
        result_type("let q: {x: Int} = { x = 1; y = 2 }\nq === { x = 1 }"),
        TyId::BOOL
    );
}

// ----- Functions -----

/// The span of the `n`th (from 0) occurrence of `needle` in `text`.
fn nth(text: &str, needle: &str, n: usize) -> Span {
    let start = text
        .match_indices(needle)
        .nth(n)
        .unwrap_or_else(|| panic!("{needle:?} does not occur {} times in {text:?}", n + 1))
        .0;
    let start = u32::try_from(start).unwrap();
    Span::new(start, start + u32::try_from(needle.len()).unwrap())
}

/// The span of the first occurrence of `needle` in `text`.
fn at(text: &str, needle: &str) -> Span {
    nth(text, needle, 0)
}

/// The span of the last occurrence of `needle` in `text`.
fn last(text: &str, needle: &str) -> Span {
    let start = u32::try_from(text.rfind(needle).unwrap()).unwrap();
    Span::new(start, start + u32::try_from(needle.len()).unwrap())
}

/// Checks `text`, which must succeed, returning its tree and resolution.
fn resolved(text: &str) -> (Ast, Resolution) {
    let (ast, _, output) = check_text(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    (ast, output.checked.resolution)
}

/// Where each use of the variable `name` reads from, in allocation
/// order.
fn uses(ast: &Ast, resolution: &Resolution, name: &str) -> Vec<VarRef> {
    let name = ast.names().lookup(name).unwrap();
    ast.expr_ids()
        .filter(|&id| ast.get(id).kind == ExprKind::Name(name))
        .map(|id| resolution.expr_var(id))
        .collect()
}

/// Function number `n`'s captures, as `(VarRef, shown type)` pairs.
fn captures(text: &str, n: u32) -> Vec<(VarRef, String)> {
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

const ADD: &str = "fn add(a: Int, b: Int): Int { a + b }\n";

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

// ----- Futures -----

/// Every expression of `text`, which must check, that is awaited where
/// it is used, as its source text and its mark, in allocation order.
fn marks(text: &str) -> Vec<(&str, AwaitMark)> {
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
const IMPLICIT_AWAIT_NOTE: &str = "using the value here would await it, which may run \
                                       effects; a pure function may hold and pass on a future \
                                       but not await it. Declare the function with `->` to \
                                       make it impure";

/// Checks that `text` has exactly one error, the error for an implicit
/// await in a pure function of `awaited`, whose type is shown as
/// `shown`, the function being the last declared with `:`.
fn assert_implicit_await_error(text: &str, awaited: Span, shown: &str) {
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

// ----- Inferred parameters -----

/// The type of the function `f` that `text` declares, as shown.
fn type_of_f(text: &str) -> String {
    shown_result(&format!("{text}\nf"))
}

/// Where the body of function number `n` of `text` first measures each
/// of its parameters, as source text, whether the check succeeds or
/// not.
fn demands(text: &str, n: u32) -> Vec<Option<&str>> {
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
fn call_site_note(call: Option<&str>) -> String {
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

// ----- Inferred results -----

/// The type of function expression number `n` of `text`, as shown,
/// whether the check succeeds or not.
fn fn_type(text: &str, n: u32) -> String {
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
    // A body awaited deeply to fit a field written present is present at
    // every bare position too, since the deep await resolves them all.
    let text = "fn f() -> {a: Present<Int>, b: Int} { = { a = lazy 1; b = lazy 2 } }\nf";
    assert_eq!(shown_result(text), "fn() -> {a: Int, b: Int}");
    assert_eq!(
        marks(text),
        vec![("{ = { a = lazy 1; b = lazy 2 } }", AwaitMark::Deep)]
    );
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

// ----- Dotted updates of records that may be futures -----

/// The type of the expression of `text`, which must check, that spans
/// `span`, as shown.
fn type_at(text: &str, span: Span) -> String {
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
fn awaited_paths(text: &str, n: Option<u32>) -> Vec<(u32, Vec<bool>)> {
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
fn first_bytes(span: Span, n: u32) -> Span {
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

// ----- Awaiting a captured value before a pure function is created -----

/// The note of the implicit-await error for a variable `name` that a
/// pure function captures and that can be awaited before it is created.
fn capture_note(name: &str) -> String {
    format!(
        "a pure function cannot await, and `{name}` may be a future; await it before the \
             function is created, in the enclosing impure code: `let {name} = await {name}`"
    )
}

/// The notes of the one error of `text`, which must be the implicit-await
/// error.
fn implicit_await_notes(text: &str) -> Vec<String> {
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

// ----- Memoised functions -----

#[test]
fn a_memoised_functions_bare_parameters_are_present() {
    assert_eq!(
        shown_result("fn f(x: Int) ->> Int { x }\nf"),
        "fn(x: Int) ->> Int"
    );
    // At every position inside, and whether the body measures them or
    // not: they are not inferred.
    assert_eq!(
        shown_result("fn g(p: {a: Int, b: {c: String}}) ->> Int { 1 }\ng"),
        "fn(p: {a: Int, b: {c: String}}) ->> Int"
    );
    // What is written explicitly means what it says.
    assert_eq!(
        shown_result(
            "fn h(x: Future<Int>, y: Int | Future<Int>, z: Present<Int>) ->> Int { 1 }\nh"
        ),
        "fn(x: Future<Int>, y: Int | Future<Int>, z: Int) ->> Int"
    );
    // A function type parameter follows its own marker, and a callback
    // parameter the body never calls is narrowed (see
    // `callback_parameters_are_narrowed`).
    assert_eq!(
        shown_result("fn k(f: fn(Int) -> Int) ->> Int { 1 }\nk"),
        "fn(f: fn(Int) -> Int | Future<Int>) ->> Int"
    );
    // Its own name in its body has the declared type.
    let text = "fn f(n: Int) ->> Int { if n < 2 { n } else { f(n - 1) + 1 } }\nf";
    assert_eq!(shown_result(text), "fn(n: Int) ->> Int");
}

#[test]
fn a_memoised_functions_bare_result_is_inferred() {
    assert_eq!(
        shown_result("fn l(x: Int) ->> Int { lazy x }\nl"),
        "fn(x: Int) ->> Future<Int>"
    );
    assert_eq!(
        shown_result("fn j(c: Bool) ->> Int { if c { 1 } else { lazy 2 } }\nj"),
        "fn(c: Bool) ->> Int | Future<Int>"
    );
    assert_eq!(
        shown_result("fn r() ->> {a: Int, b: Int} { a = 1; b = lazy 2 }\nr"),
        "fn() ->> {a: Int, b: Future<Int>}"
    );
    // A call's result is the inferred one.
    assert_eq!(
        shown_result("fn l(x: Int) ->> Int { lazy x }\nl(1)"),
        "Future<Int>"
    );
    // A future passed through, as written.
    assert_eq!(
        shown_result("fn p(x: Future<Int>) ->> Int { x }\np"),
        "fn(x: Future<Int>) ->> Future<Int>"
    );
}

#[test]
fn a_written_memoised_function_type() {
    // Parameters present, result the either shape.
    assert_eq!(
        shown_result("let f: fn(Int, {a: Int}) ->> Int = fn(x: Int, r: {a: Int}) ->> Int { x }\nf"),
        "fn(Int, {a: Int}) ->> Int | Future<Int>"
    );
    // Nested in another function type, with its own rule.
    assert_eq!(
        shown_result(
            "let f: fn(fn(Int) ->> Int) -> Int = fn(g: fn(Int) ->> Int) -> Int { g(1) }\nf"
        ),
        "fn(fn(Int) ->> Int | Future<Int>) -> Int | Future<Int>"
    );
    // A memoised function stands in for an impure one whose caller
    // passes present arguments, and a pure one for a memoised one.
    let memo = "let m = fn(x: Int) ->> Int { x }\n";
    assert!(diagnostics_of(&format!("{memo}let g: fn(Present<Int>) -> Int = m")).is_empty());
    assert!(diagnostics_of("let g: fn(Int) ->> Int = fn(x: Int): Int { x }").is_empty());
    // Not for one whose caller may pass a future, nor for a pure one.
    for target in ["fn(Int) -> Int", "fn(Present<Int>): Int"] {
        assert_eq!(
            messages(&format!("{memo}let g: {target} = m")),
            vec!["mismatched types"],
            "{target}"
        );
    }
}

/// The diagnostics of `text`, none if it checks.
fn diagnostics_of(text: &str) -> Vec<Diagnostic> {
    check(&parse_text(text), &mut TypeTable::new())
        .err()
        .unwrap_or_default()
}

#[test]
fn an_argument_that_may_be_a_future_is_awaited_at_a_memoised_call() {
    let f = "fn f(x: Int) ->> Int { x }\n";
    assert_eq!(
        marks(&format!("{f}let v = lazy 1\nf(v)")),
        vec![("v", AwaitMark::Top)]
    );
    // Deeply, where the parameter's type needs it.
    let g = "fn g(p: {a: Int}) ->> Int { p.a }\n";
    assert_eq!(
        marks(&format!("{g}let r = {{ a = lazy 1 }}\ng(r)")),
        vec![("r", AwaitMark::Deep)]
    );
    // Not where the parameter is written lazy.
    assert!(marks("fn h(x: Future<Int>) ->> Int { 1 }\nh(lazy 1)").is_empty());
}

#[test]
fn a_pure_function_cannot_call_a_memoised_one() {
    let text = "fn m(x: Int) ->> Int { x }\nfn p(): Int { m(1) }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(
        labels(&found[0]),
        vec![
            (last(text, "m"), "this is fn(x: Int) ->> Int"),
            (last(text, ":"), "this function is pure, declared with `:`")
        ]
    );
    // Nor can it await, whatever it calls.
    assert_eq!(
        messages("fn p(v: Future<Int>): Int { await v }"),
        vec!["cannot await in a pure function"]
    );
    // But it may make, hold and return one.
    assert_eq!(
        shown_result("fn make(): fn(Int) ->> Int { fn(x: Int) ->> Int { x } }\nmake"),
        "fn(): fn(Int) ->> Int | Future<Int>"
    );
}

#[test]
fn a_memoised_functions_body_is_impure() {
    // It may call impure functions and await, explicitly or not.
    let text = "fn tick() -> Int { 1 }\n\
                    fn m(x: Int) ->> Int {\n\
                    \x20 let f = lazy x + 1\n\
                    \x20 tick() + await f + f\n\
                    }\nm";
    assert_eq!(shown_result(text), "fn(x: Int) ->> Int");
    // And call memoised ones, itself included.
    let text = "fn m(x: Int) ->> Int { if x == 0 { 0 } else { m(x - 1) } }\n\
                    fn n(x: Int) ->> Int { m(x) + 1 }\nn";
    assert_eq!(shown_result(text), "fn(x: Int) ->> Int");
}

#[test]
fn a_pure_closure_in_a_memoised_function() {
    // A memoised function's parameters are present, so a pure closure
    // may use them.
    assert_eq!(
        shown_result("fn m(n: Int) ->> fn(): Int { fn(): Int { n + 1 } }\nm"),
        "fn(n: Int) ->> fn(): Int"
    );
    // A variable of its body that may be a future cannot be awaited by
    // the pure closure, and the memoised function, being impure, could
    // await it first: the hint says so.
    let text = "fn m(n: Int) ->> Int {\n\
                    \x20 let v = lazy n\n\
                    \x20 let g = fn(): Int { v + 1 }\n\
                    \x20 g()\n\
                    }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("v")]);
    // A pure enclosing function's bare parameter, seen through the
    // memoised function, is not claimable: measuring it there awaits it
    // in the memoised function, and it stays the either shape.
    assert_eq!(
        shown_result("fn outer(n: Int): fn(Int) ->> Int { fn(x: Int) ->> Int { n + x } }\nouter"),
        "fn(n: Int | Future<Int>): fn(Int) ->> Int | Future<Int>"
    );
    // Through the memoised function into a pure closure it is the
    // purity error, with the hint, the memoised function being able to
    // await it.
    let text = "fn outer(n: Int): fn() ->> fn(): Int {\n\
                    \x20 fn() ->> fn(): Int { fn(): Int { n + 1 } }\n\
                    }";
    assert_eq!(implicit_await_notes(text), vec![capture_note("n")]);
    // With a pure function in its place, the parameter is claimed.
    assert_eq!(
        shown_result(
            "fn outer(n: Int): fn(): fn(): Int { fn(): fn(): Int { fn(): Int { n + 1 } } }\nouter"
        ),
        "fn(n: Int): fn(): fn(): Int"
    );
}

// ----- Narrowed callbacks -----

/// The type of the function named `run` in `text`, which must check, as
/// shown.
fn type_of_run(text: &str) -> String {
    shown_result(&format!("{text}\nrun"))
}

/// The type of the function expression written first in `text` at the
/// first occurrence of `needle`, which starts with its `fn`, as shown,
/// whether the check succeeds or not.
fn type_of_fn_at(text: &str, needle: &str) -> String {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let mut checker = Checker::new(&ast, &mut types);
    checker.check_body(ast.root(), None);
    let start = at(text, needle).start;
    let id = ast
        .expr_ids()
        .find(|&id| matches!(ast.get(id).kind, ExprKind::Fn(_)) && ast.get(id).span.start == start)
        .unwrap_or_else(|| panic!("no function at {needle:?} in {text:?}"));
    let ty = checker.expr_types[id.index()];
    types.display(ty, ast.names()).to_string()
}

/// The span of the `n`th (from 0) occurrence of `needle` in `text`,
/// from `offset` bytes into it, `len` bytes long.
fn within(text: &str, needle: &str, n: usize, (offset, len): (u32, u32)) -> Span {
    let found = nth(text, needle, n);
    Span::new(found.start + offset, found.start + offset + len)
}

#[test]
fn callback_parameters_are_narrowed() {
    // The rows of the specification.
    for (text, shown) in [
        (
            "fn run(f: fn(Int) -> Int) -> Int { f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // `j` may be a future.
        (
            "fn run(f: fn(Int) -> Int, j: Int) -> Int { f(j) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> Int \
                 | Future<Int>",
        ),
        (
            "fn run(f: fn(Int) -> Int, j: Int) -> Int { f(await j) }",
            "fn(f: fn(Int) -> Int | Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Every call must fit.
        (
            "fn run(f: fn(Int) -> Int, j: Int) -> Int { f(1) + f(j) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> Int",
        ),
        // Each position on its own.
        (
            "fn run(f: fn(Int, Int) -> Int, j: Int) -> Int { f(1, j) }",
            "fn(f: fn(Int, Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> \
                 Int | Future<Int>",
        ),
        (
            "fn run(f: fn(Int) -> Int) -> Future<Int> { lazy f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Future<Int>",
        ),
        // Used as a value.
        (
            "fn run(f: fn(Int) -> Int) -> Int { let g = f\n g(1) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        (
            "fn other(g: fn(Int) -> Int) -> Int { g(1) }\n\
                 fn run(f: fn(Int) -> Int) -> Int { other(f) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Explicit positions.
        (
            "fn run(f: fn(Present<Int>) -> Int) -> Int { f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        (
            "fn run(f: fn(Int | Future<Int>) -> Int) -> Int { f(1) }",
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // A record type, narrowed to its present type.
        (
            "fn run(f: fn({a: Int}) -> Int) -> Int { f({ a = 1 }) }",
            "fn(f: fn({a: Int}) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        (
            "fn run(f: fn({a: Int}) -> Int, j: Int) -> Int { f({ a = j }) }",
            "fn(f: fn({a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> Int | \
                 Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Vacuously: the call is of the shadowing `f`.
        (
            "fn run(f: fn(Int) -> Int) -> Int { let f = fn(x: Int) -> Int { x }\n f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // Not a callback parameter.
        (
            "fn run(f: fn(Int) ->> Int) -> Int { f(1) }",
            "fn(f: fn(Int) ->> Int | Future<Int>) -> Int | Future<Int>",
        ),
        // A memoised function's.
        (
            "fn run(f: fn(Int) -> Int) ->> Int { f(1) }",
            "fn(f: fn(Int) -> Int | Future<Int>) ->> Int | Future<Int>",
        ),
    ] {
        assert_eq!(type_of_run(text), shown, "{text}");
    }
    // Explicit parts of a record type are kept, and its bare fields
    // are present.
    assert_eq!(
        type_of_run("fn run(f: fn({a: Future<Int>, b: Int}) -> Int) -> Int { 1 }"),
        "fn(f: fn({a: Future<Int>, b: Int}) -> Int | Future<Int>) -> Int"
    );
    // Neither is a function type's position, nor the callback's
    // result.
    assert_eq!(
        type_of_run("fn run(f: fn(fn(Int) -> Int) -> {a: Int}) -> Int { 1 }"),
        "fn(f: fn(fn(Int | Future<Int>) -> Int | Future<Int>) -> {a: Int | Future<Int>} | \
             Future<{a: Int | Future<Int>}>) -> Int"
    );
    // Nor anything of a pure function's, nor a parameter whose function
    // type is not written directly.
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int): Int { 1 }"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>): Int"
    );
    assert_eq!(
        type_of_run("fn run(r: {f: fn(Int) -> Int}) -> Int { r.f(1) }"),
        "fn(r: {f: fn(Int | Future<Int>) -> Int | Future<Int>} | Future<{f: fn(Int | \
             Future<Int>) -> Int | Future<Int>}>) -> Int | Future<Int>"
    );
    // Other uses as a value.
    for body in ["(f)(1)", "\"{f}\"", "f"] {
        let text = format!("fn run(f: fn(Int) -> Int) -> {{}} {{ let x = {body}\n{{}} }}");
        assert_eq!(
            type_of_run(&text),
            "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> {}",
            "{text}"
        );
    }
}

#[test]
fn a_narrowed_callback_takes_a_pure_function_that_measures() {
    let double = "fn double(x: Int): Int { x * 2 }\n";
    let run = "fn run(f: fn(Int) -> Int) -> Int { f(1) }\n";
    assert_eq!(
        shown_result(&format!("{double}{run}run(double)")),
        "Int | Future<Int>"
    );
    assert_eq!(
        shown_result(&format!("{run}run(fn(x: Int): Int {{ x * 2 }})")),
        "Int | Future<Int>"
    );
    // A written function type with the narrowed type's shape takes the
    // host.
    assert!(
        check(
            &parse_text(&format!(
                "{run}let r: fn(fn(Present<Int>) -> Int) -> Int = run"
            )),
            &mut TypeTable::new()
        )
        .is_ok()
    );
}

#[test]
fn a_callback_given_a_future_labels_the_mismatch() {
    let double = "fn double(x: Int): Int { x * 2 }\n";
    let function = "the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int \
                        | Future<Int>) -> Int";
    let given = "`f` is given a value that may be a future here";
    let used = "`f` is used as a value here, so it may be given a future";
    // A call that gives the position a value that may be a future.
    let text = format!(
        "{double}fn run(f: fn(Int) -> Int, j: Int) -> Int {{ f(1) + f(j) }}\nrun(double, 1)"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, last(&text, "double"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected fn(Int | Future<Int>) -> Int | Future<Int>, found fn(x: Int): Int")
    );
    assert_eq!(
        labels(&found[0]),
        vec![
            (last(&text, "run"), function),
            (within(&text, "f(j)", 0, (2, 1)), given)
        ]
    );
    // A use as a value.
    let text = format!(
        "{double}fn run(f: fn(Int) -> Int) -> Int {{\n  let g = f\n  g(1)\n}}\nrun(double)"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(
        labels(&found[0]),
        vec![
            (
                last(&text, "run"),
                "the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | \
                     Future<Int>"
            ),
            (within(&text, "= f", 0, (2, 1)), used)
        ]
    );
    // The first in source order.
    for (body, needle, label) in [
        ("let n = f(j)\n  let g = f\n  n", "f(j)", given),
        ("let g = f\n  f(j)", "= f", used),
    ] {
        let text = format!(
            "{double}fn run(f: fn(Int) -> Int, j: Int) -> Int {{\n  {body}\n}}\nrun(double, 1)"
        );
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        let span = within(&text, needle, 0, (2, 1));
        assert_eq!(labels(&found[0]).get(1), Some(&(span, label)), "{text}");
    }
    // At a position the argument needs: the second's, where only it
    // measures its parameter.
    let text = "fn run(f: fn(Int, Int) -> Int, j: Int) -> Int { f(j, 1) + f(1, j) }\n\
                    run(fn(a: Int | Future<Int>, b: Int): Int { b }, 1)";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        labels(&found[0]).get(1),
        Some(&(within(text, "1, j)", 0, (3, 1)), given))
    );
    // No label where the mismatch has another cause: the argument would
    // not fit had the position been narrowed.
    for arg in [
        "fn(x: String): Int { 1 }",
        "fn(x: Int): String { \"a\" }",
        "1",
    ] {
        let text = format!("fn run(f: fn(Int) -> Int, j: Int) -> Int {{ f(j) }}\nrun({arg}, 1)");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, "mismatched types", "{text}");
        assert_eq!(labels(&found[0]).len(), 1, "{text}: {found:?}");
    }
    // Nor for a callee that is not known.
    let text = format!(
        "{double}let r = {{ run = fn(f: fn(Int) -> Int, j: Int) -> Int {{ f(j) }} }}\n\
             r.run(double, 1)"
    );
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(labels(&found[0]).len(), 1, "{found:?}");
}

#[test]
fn a_callbacks_position_is_the_parameter_its_argument_fills() {
    // Named arguments, in any order.
    assert_eq!(
        type_of_run("fn run(f: fn(x: Int, y: Int) -> Int, j: Int) -> Int { f(y = 1, x = j) }"),
        "fn(f: fn(x: Int | Future<Int>, y: Int) -> Int | Future<Int>, j: Int | Future<Int>) \
             -> Int | Future<Int>"
    );
    // A trailing block fills the last.
    assert_eq!(
        type_of_run(
            "fn run(f: fn(n: Int, body: {a: Int}) -> Int, j: Int) -> Int { f(j) { a = 1 } }"
        ),
        "fn(f: fn(n: Int | Future<Int>, body: {a: Int}) -> Int | Future<Int>, j: Int | \
             Future<Int>) -> Int | Future<Int>"
    );
    assert_eq!(
        type_of_run(
            "fn run(f: fn(n: Int, body: {a: Int}) -> Int, j: Int) -> Int { f(1) { a = j } }"
        ),
        "fn(f: fn(n: Int, body: {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> \
             Int | Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn calls_of_a_callback_count_in_nested_functions_and_lazies() {
    let narrowed = "fn(f: fn(Int) -> Int | Future<Int>)";
    let written = "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>)";
    for (body, shown) in [
        ("let g = fn(x: Int) -> Int { f(1) }\n lazy g(2)", narrowed),
        ("let g = fn(x: Int) -> Int { f(x) }\n lazy g(2)", written),
        (
            "let g = fn(x: Int) -> Future<Int> { lazy f(1) }\n g(2)",
            narrowed,
        ),
        (
            "let g = fn(x: Int) -> Future<Int> { lazy f(x) }\n g(2)",
            written,
        ),
        // Awaited in the nested function first.
        (
            "let g = fn(x: Int) -> Future<Int> { let y = await x\n lazy f(y) }\n g(2)",
            narrowed,
        ),
    ] {
        let text = format!("fn run(f: fn(Int) -> Int) -> Future<Int> {{ {body} }}");
        assert_eq!(
            type_of_run(&text),
            format!("{shown} -> Future<Int>"),
            "{text}"
        );
    }
}

#[test]
fn callback_parameters_are_narrowed_independently() {
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int, g: fn(Int) -> Int, j: Int) -> Int { f(1) + g(j) }"),
        "fn(f: fn(Int) -> Int | Future<Int>, g: fn(Int | Future<Int>) -> Int | Future<Int>, \
             j: Int | Future<Int>) -> Int"
    );
    // A host nested in another, each with a callback, the inner calling
    // the outer's.
    let text = "fn run(f: fn(Int) -> Int, j: Int) -> Int {\n\
                    \x20 let inner = fn(g: fn(Int) -> Int) -> Int { f(j) + g(1) }\n\
                    \x20 inner(fn(x: Int) -> Int { x })\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | Future<Int>) -> Int"
    );
    assert_eq!(
        type_of_fn_at(text, "fn(g"),
        "fn(g: fn(Int) -> Int | Future<Int>) -> Int"
    );
    let text = "fn run(f: fn(Int) -> Int, j: Int) -> Int {\n\
                    \x20 let inner = fn(g: fn(Int) -> Int) -> Int { f(1) + g(j) }\n\
                    \x20 inner(fn(x: Int) -> Int { x })\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int) -> Int | Future<Int>, j: Int | Future<Int>) -> Int"
    );
    assert_eq!(
        type_of_fn_at(text, "fn(g"),
        "fn(g: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int"
    );
    // An inner parameter of the same name shadows the outer one.
    let text = "fn run(f: fn(Int) -> Int, j: Int) -> Int {\n\
                    \x20 let inner = fn(f: fn(Int) -> Int) -> Int { f(j) }\n\
                    \x20 inner(fn(x: Int) -> Int { x })\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int) -> Int | Future<Int>, j: Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn a_host_with_an_error_keeps_its_declared_type() {
    let declared = "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>";
    for text in [
        "fn run(f: fn(Int) -> Int) -> Int { f(1) + \"a\" }",
        "fn run(f: fn(Int) -> Int) -> Int { let x: String = f(1)\n1 }",
        "fn run(f: fn(Int) -> Int) -> Int { f(\"a\") }",
    ] {
        assert_eq!(messages(text).len(), 1, "{text}");
        assert_eq!(type_of_fn_at(text, "fn run"), declared, "{text}");
    }
    // And with an erroneous parameter type, erroneous itself.
    assert_eq!(
        type_of_fn_at("fn run(f: fn(Nope) -> Int) -> Int { f(1) }", "fn run"),
        "<error>"
    );
    // Its own name in its body has the declared type.
    let text = "fn run(f: fn(Int) -> Int, n: Int) -> Int {\n\
                    \x20 if n == 0 { f(1) } else { run(f, n - 1) }\n\
                    }";
    assert_eq!(
        type_of_run(text),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, n: Int | Future<Int>) -> Int | \
             Future<Int>"
    );
}

#[test]
fn a_host_is_narrowed_and_has_its_result_inferred() {
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int) -> Int { f(1) + 1 }"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> Int"
    );
    assert_eq!(
        type_of_run("fn run(f: fn(Int) -> Int) -> {v: Int, w: Int} { v = f(2); w = 3 }"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> {v: Int | Future<Int>, w: Int}"
    );
}

#[test]
fn an_untyped_literal_fits_a_callbacks_position_once_coerced() {
    for arg in ["1", "1 + 2", "(3)"] {
        let text = format!("fn run(f: fn(Float) -> Int) -> Int {{ f({arg}) }}");
        assert_eq!(
            type_of_run(&text),
            "fn(f: fn(Float) -> Int | Future<Int>) -> Int | Future<Int>",
            "{text}"
        );
    }
    assert_eq!(
        type_of_run("fn run(f: fn({a: Float}) -> Int) -> Int { f({ a = 1 }) }"),
        "fn(f: fn({a: Float}) -> Int | Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn a_provisional_parameter_given_to_a_callback_may_be_a_future() {
    // The enclosing pure function's parameter may be a future for all
    // the nested impure host knows.
    let text = "fn outer(n: Int): fn(fn(Int) -> Int) -> Int {\n\
                    \x20 fn(f: fn(Int) -> Int) -> Int { f(n) }\n\
                    }";
    assert!(check(&parse_text(text), &mut TypeTable::new()).is_ok());
    assert_eq!(
        type_of_fn_at(text, "fn(f:"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    // Even where the pure function measures it elsewhere.
    let text = "fn outer(n: Int): fn(fn(Int) -> Int) -> Int {\n\
                    \x20 let m = n + 1\n\
                    \x20 fn(f: fn(Int) -> Int) -> Int { f(n) }\n\
                    }";
    assert_eq!(
        type_of_fn_at(text, "fn(f:"),
        "fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | Future<Int>"
    );
    // Awaited in the host first, it is present.
    let text = "fn outer(n: Int): fn(fn(Int) -> Int) -> Int {\n\
                    \x20 fn(f: fn(Int) -> Int) -> Int { f(await n) }\n\
                    }";
    assert_eq!(
        type_of_fn_at(text, "fn(f:"),
        "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>"
    );
}

// ----- Lists -----

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
fn text_type(shown: &str) -> String {
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

#[test]
fn len_is_a_built_in_function() {
    assert_eq!(shown_result("len([1, 2])"), "Int");
    assert_eq!(shown_result("len(list = [1, 2])"), "Int");
    assert_eq!(shown_result("len([lazy 1])"), "Int");
    // Its argument is measured at the top, its elements not at all.
    let text = "let xs = lazy [lazy 1]\nlen(xs)";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Top)]);
    // It is pure, so a pure function may call it.
    assert_eq!(
        type_of_f("fn f(xs: Int[]): Int { len(xs) + 1 }"),
        "fn(xs: List<Int>): Int"
    );
    // Its callee has the function type of the call.
    let text = "len([1])";
    assert_eq!(type_at(text, at(text, "len")), "fn(List<Int>): Int");
    let (ast, _, output) = check_prelude(text);
    let resolution = output.unwrap().checked.resolution;
    let callee = ast
        .expr_ids()
        .find(|&id| ast.get(id).span == at(text, "len"))
        .unwrap();
    assert_eq!(resolution.expr_var(callee), VarRef::Builtin(Builtin::Len));
    // A future list in a pure function is the purity error.
    let text = "fn f(xs: Future<Int[]>): Int { len(xs) }";
    assert_implicit_await_error(text, last(text, "xs"), "Future<List<Int>>");
}

#[test]
fn a_built_in_is_shadowed_as_any_outer_name() {
    assert_eq!(shown_result("let len = 5\nlen"), "Int");
    assert_eq!(
        shown_result("let len = fn(x: String): String { x }\nlen(\"a\")"),
        "String"
    );
    assert_eq!(
        type_of_f("fn f(len: Int): Int { len + 1 }"),
        "fn(len: Int): Int"
    );
    assert_eq!(file_type("len = 1\nn = len"), "{len: Int, n: Int}");
    // `:=` never finds it.
    assert_eq!(
        messages("len := 1"),
        vec!["cannot rebind `len`: no variable named `len` is declared"]
    );
}

#[test]
fn a_built_in_is_not_a_value_yet() {
    for (text, span) in [
        ("len", Span::new(0, 3)),
        ("let mut f = len", Span::new(12, 15)),
        ("(len)([1])", Span::new(1, 4)),
        ("\"{len}\"", Span::new(2, 5)),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message,
            "`len` is a built-in function and cannot be used as a value yet"
        );
        assert_eq!(found[0].span, span, "{text}");
        assert_eq!(
            found[0].notes,
            vec![
                "a built-in function with a type for any list cannot be written as a type \
                     yet; call it, or bind it to a name"
            ]
        );
    }
}

#[test]
fn the_alias_rule_lets_a_special_built_in_stand_only_where_it_says() {
    const NOT_A_VALUE: &str = "`len` is a built-in function and cannot be used as a value yet";
    // Every other use is the error, at the reference: an argument, a
    // list element, a field of an inner record, a function's result,
    // an operand of `==` and `===`, an `is`, an `as`, an ascription, a
    // `let` that is not a top-level one of one name, a branch, a
    // `lazy`, a rebind, a dotted field and a record read whole.
    for (text, needle) in [
        ("fn f(g: Int): Int { g }\nx = f(len)", "len)"),
        ("x = [len]", "len]"),
        ("x = { inner = len }", "len }"),
        ("fn h(): Int { len }", "len }"),
        ("x = len == 1", "len =="),
        ("x = 1 === len", "len"),
        ("x = len is Int", "len is"),
        ("x = len as Int", "len as"),
        ("x = len : Int", "len :"),
        ("x = { let l = len; 1 }", "len;"),
        ("let mut l = len", "len"),
        ("let l: fn(xs: Int[]): Int = len", "len"),
        ("x = if true { len } else { len }", "len }"),
        ("x = lazy len", "len"),
        ("let l = len\nl := len", "len"),
        ("com.l = len", "len"),
        ("let l = len\nx = [l]", "l]"),
        ("l = len\nx = { l }", "l }"),
    ] {
        let found = diagnostics(text);
        assert!(!found.is_empty(), "{text}");
        assert!(
            found.iter().all(|error| error.message == NOT_A_VALUE),
            "{text}: {found:?}"
        );
        let start = u32::try_from(text.rfind(needle).unwrap()).unwrap();
        assert!(
            found.iter().any(|error| error.span.start == start),
            "{text}: {found:?}"
        );
    }
    // Where it may: the callee of a call, and the whole initialiser of
    // a top-level field or `let` of one name, each then an alias, of
    // the opaque type, called by its rule.
    assert_eq!(file_type("l = len\nn = l([1])"), "{l: builtin len, n: Int}");
    assert_eq!(
        file_type("let l = len\nm = l\nn = m([1])"),
        "{m: builtin len, n: Int}"
    );
    assert_eq!(shown_result("let l = len\nlet m = l\nm([[1]])"), "Int");
    assert_eq!(
        messages("let l = len\nn = l(1)"),
        ["`len` takes a list, found Int"]
    );
    // Through `builtins`, a path to its cell, never the record whole.
    assert_eq!(shown_result("import builtins\nbuiltins.len([1])"), "Int");
    assert_eq!(
        shown_result("import builtins\nlet l = builtins.len\nl([1])"),
        "Int"
    );
    assert_eq!(messages("import builtins\nlet b = builtins"), [NOT_A_VALUE]);
}

#[test]
fn int_and_float_have_function_types() {
    assert_eq!(shown_result("int"), "fn(x: Float): Int");
    assert_eq!(shown_result("float"), "fn(x: Int): Float");
    assert_eq!(shown_result("int(2.5)"), "Int");
    assert_eq!(shown_result("float(x = 2)"), "Float");
    assert_eq!(shown_result("half(1)"), "Float");
    assert_eq!(
        shown_result("import builtins\nbuiltins.int"),
        "fn(x: Float): Int"
    );
    assert_eq!(
        file_type("i = int\nf = [float]"),
        "{f: List<fn(x: Int): Float>, i: fn(x: Float): Int}"
    );
    // Called as any function is: an integer literal adapts to `Float`.
    assert_eq!(shown_result("int(1)"), "Int");
    assert_eq!(messages("int(1 // 1)"), ["mismatched types"]);
    assert_eq!(messages("float(1.5)"), ["mismatched types"]);
    // No equality, as for any function.
    assert_eq!(messages("int === int"), ["cannot compare functions"]);
    assert_eq!(messages("int == int"), ["cannot compare functions"]);
    // Pure: a pure function may call them.
    assert_eq!(
        type_of_f("fn f(x: Int): Int { int(float(x) * 1.5) }"),
        "fn(x: Int): Int"
    );
}

#[test]
fn the_string_builtins_have_function_types() {
    for (text, shown) in [
        ("split", "fn(s: String, sep: String): List<String>"),
        ("join", "fn(parts: List<String>, sep: String): String"),
        ("starts_with", "fn(s: String, prefix: String): Bool"),
        ("ends_with", "fn(s: String, suffix: String): Bool"),
        ("trim", "fn(s: String): String"),
        (
            "import builtins\nbuiltins.split",
            "fn(s: String, sep: String): List<String>",
        ),
        ("split(\"a/b\", \"/\")", "List<String>"),
        ("join([\"a\"], sep = \",\")", "String"),
        ("starts_with(prefix = \"/\", s = \"/usr\")", "Bool"),
        ("ends_with(\"x.gz\", \".gz\")", "Bool"),
        ("trim(\" x \")", "String"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // The summary's string lines.
    assert_eq!(
        file_type(
            "parts = split(\"a/b/c\", \"/\")\n\
                 s = join(parts, \"-\")\n\
                 yes = starts_with(\"/usr\", \"/\")\n\
                 gz = ends_with(\"x.tar.gz\", \".gz\")\n\
                 hash = trim(sh(\"echo h\"))"
        ),
        "{gz: Bool, hash: String, parts: List<String>, s: String, yes: Bool}"
    );
    // Their arguments are checked as any call's.
    assert_eq!(messages("split(1, \"/\")"), ["mismatched types"]);
    assert_eq!(messages("join([1], \",\")"), ["mismatched types"]);
    assert_eq!(messages("trim([\"x\"])"), ["mismatched types"]);
    assert_eq!(messages("split(\"a\")"), ["expected 2 arguments, found 1"]);
    assert_eq!(
        messages("ends_with(s = \"a\", prefix = \"b\")"),
        ["unknown argument `prefix`", "missing argument `suffix`"]
    );
    // Pure: a pure function may call them.
    assert_eq!(
        type_of_f(
            "fn f(p: String): Bool { starts_with(trim(join(split(p, \"/\"), \"-\")), \"-\") }"
        ),
        "fn(p: String): Bool"
    );
    // A file's own `join` shadows the prelude's silently.
    assert_eq!(
        file_type("join = \"mine\"\nv = join"),
        "{join: String, v: String}"
    );
}

#[test]
fn the_more_string_builtins_and_compare_versions_have_function_types() {
    for (text, shown) in [
        ("strip_prefix", "fn(s: String, prefix: String): String"),
        ("strip_suffix", "fn(s: String, suffix: String): String"),
        ("lines", "fn(s: String): List<String>"),
        ("fields", "fn(s: String): List<String>"),
        ("compare_versions", "fn(a: String, b: String): Int"),
        (
            "import builtins\nbuiltins.fields",
            "fn(s: String): List<String>",
        ),
        ("strip_prefix(\"ab\", \"a\")", "String"),
        ("strip_suffix(suffix = \".gz\", s = \"a.gz\")", "String"),
        ("lines(\"a\\nb\")", "List<String>"),
        ("fields(\" a b \")", "List<String>"),
        ("compare_versions(b = \"6.6\", a = \"6.10\")", "Int"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // The summary of slice 41.
    assert_eq!(
        file_type(
            "a = strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")\n\
                 b = strip_prefix(\"x\", \"y\")\n\
                 c = strip_suffix(\"a.tar.gz\", \".gz\")\n\
                 d = lines(\"a\\nb\\n\")\n\
                 e = fields(\"  tcp\\t6  TCP \")\n\
                 f = compare_versions(\"6.10\", \"6.6\")"
        ),
        "{a: String, b: String, c: String, d: List<String>, e: List<String>, f: Int}"
    );
    // Their arguments are checked as any call's.
    assert_eq!(messages("lines([\"x\"])"), ["mismatched types"]);
    assert_eq!(messages("compare_versions(1, \"2\")"), ["mismatched types"]);
    assert_eq!(
        messages("fields(\"a\", \"b\")"),
        ["expected 1 argument, found 2"]
    );
    assert_eq!(
        messages("strip_suffix(s = \"a\", prefix = \"b\")"),
        ["unknown argument `prefix`", "missing argument `suffix`"]
    );
    // Pure: a pure function may call them.
    assert_eq!(
        type_of_f(
            "fn f(v: String, w: String): Bool { \
                 compare_versions(strip_prefix(v, \"v\"), strip_suffix(w, \"\\n\")) > 0 }"
        ),
        "fn(v: String, w: String): Bool"
    );
    assert_eq!(
        type_of_f("fn f(s: String): Int { len(fields(lines(s)[0])) }"),
        "fn(s: String): Int"
    );
    // A file's own `fields` shadows the prelude's silently.
    assert_eq!(file_type("fields = 1\nv = fields"), "{fields: Int, v: Int}");
}

#[test]
fn the_file_system_builtins_have_impure_function_types() {
    for (text, shown) in [
        ("import fs\nfs.exists", "fn(p: String) -> Bool"),
        ("import fs\nfs.read", "fn(p: String) -> String"),
        ("import fs\nfs.mkdir", "fn(p: String) -> {}"),
        ("import fs\nfs.copy", "fn(from: String, to: String) -> {}"),
        ("import fs\nfs.temp_dir", "fn() -> String"),
        (
            "import builtins\nbuiltins.fs_read",
            "fn(p: String) -> String",
        ),
        ("import read from fs\nread", "fn(p: String) -> String"),
        ("import fs\nfs.exists(\"/\")", "Bool"),
        ("import fs\nfs.read(p = \"/x\")", "String"),
        ("import fs\nfs.mkdir(\"/x\")", "{}"),
        ("import fs\nfs.copy(to = \"/b\", from = \"/a\")", "{}"),
        ("import fs\nfs.temp_dir()", "String"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // Not the prelude's: without `import fs`, no such names.
    assert_eq!(
        messages("fs_read(\"/x\")"),
        ["undefined variable `fs_read`"]
    );
    assert_eq!(messages("read(\"/x\")"), ["undefined variable `read`"]);
    // Their arguments are checked as any call's.
    assert_eq!(messages("import fs\nfs.read(1)"), ["mismatched types"]);
    assert_eq!(
        messages("import fs\nfs.temp_dir(\"/x\")"),
        ["expected 0 arguments, found 1"]
    );
    assert_eq!(
        messages("import fs\nfs.copy(\"/a\")"),
        ["expected 2 arguments, found 1"]
    );
    // Impure: from a pure function, the existing error, through the
    // module, a member import, an alias and `builtins` alike.
    for text in [
        "import fs\nfn f(p: String): String { fs.read(p) }",
        "import read from fs\nfn f(p: String): String { read(p) }",
        "import fs as files\nfn f(): String { files.temp_dir() }",
        "import builtins\nfn f(p: String): Bool { builtins.fs_exists(p) }",
        "import fs\nfn f(p: String): {} { fs.mkdir(p) }",
    ] {
        assert_eq!(
            messages(text),
            ["cannot call an impure function from a pure function"],
            "{text}"
        );
    }
    // From a `lazy` in a pure function, an impure or memoised function,
    // and the top level, it is fine.
    assert_eq!(
        type_of_f("import fs\nfn f(p: String): Future<String> { lazy { fs.read(p) } }"),
        "fn(p: String | Future<String>): Future<String>"
    );
    assert_eq!(
        type_of_f("import fs\nfn f() -> String { fs.read(\"/x\") }"),
        "fn() -> String"
    );
    assert_eq!(
        type_of_f("import fs\nfn f() ->> String { fs.temp_dir() }"),
        "fn() ->> String"
    );
}

#[test]
fn fs_write_and_sort_versions_have_function_types() {
    for (text, shown) in [
        (
            "import fs\nfs.write",
            "fn(p: String, contents: String) -> {}",
        ),
        (
            "import write from fs\nwrite",
            "fn(p: String, contents: String) -> {}",
        ),
        (
            "import builtins\nbuiltins.fs_write",
            "fn(p: String, contents: String) -> {}",
        ),
        ("import fs\nfs.write(\"/a\", \"x\")", "{}"),
        ("import fs\nfs.write(contents = \"x\", p = \"/a\")", "{}"),
        ("sort_versions", "fn(vs: List<String>): List<String>"),
        ("sort_versions([\"1\", \"0\"])", "List<String>"),
        ("sort_versions([])", "List<String>"),
        ("sort_versions(vs = [\"1\"])", "List<String>"),
        (
            "import builtins\nbuiltins.sort_versions",
            "fn(vs: List<String>): List<String>",
        ),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // Not the prelude's: without `import fs`, no `write`.
    assert_eq!(
        messages("write(\"/a\", \"x\")"),
        ["undefined variable `write`"]
    );
    // Their arguments are checked as any call's.
    assert_eq!(
        messages("import fs\nfs.write(\"/a\", 1)"),
        ["mismatched types"]
    );
    assert_eq!(
        messages("import fs\nfs.write(\"/a\")"),
        ["expected 2 arguments, found 1"]
    );
    assert_eq!(messages("sort_versions([1])"), ["mismatched types"]);
    assert_eq!(messages("sort_versions(\"1\")"), ["mismatched types"]);
    // `fs.write` is impure: from a pure function, the existing error, by
    // any road; `sort_versions` is pure.
    for text in [
        "import fs\nfn f(p: String): {} { fs.write(p, \"x\") }",
        "import write from fs\nfn f(p: String): {} { write(p, \"x\") }",
        "import builtins\nfn f(p: String): {} { builtins.fs_write(p, \"x\") }",
    ] {
        assert_eq!(
            messages(text),
            ["cannot call an impure function from a pure function"],
            "{text}"
        );
    }
    assert_eq!(
        type_of_f("import fs\nfn f(p: String) -> {} { fs.write(p, \"x\") }"),
        "fn(p: String | Future<String>) -> {}"
    );
    assert_eq!(
        type_of_f("fn f(vs: List<String>): List<String> { sort_versions(vs) }"),
        "fn(vs: List<String>): List<String>"
    );
}

#[test]
fn show_is_typed_by_its_rule() {
    // Any type without a function in it, to `String`.
    for text in [
        "show(1)",
        "show(1.5)",
        "show(true)",
        "show(\"a\")",
        "show({})",
        "show({a = 1; b = [\"x\"]})",
        "show([[1], [2]])",
        "let u: Int | String = 1\nshow(u)",
        "type Pin = {version: String, commit: String}\nlet p: Pin = {version = \"1\"; commit = \"c\"}\nshow(p)",
        "show(value = 1)",
        "import builtins\nbuiltins.show(1)",
        "let s = show\ns(1)",
        "import show as render from prelude\nrender(1)",
        "show(lazy 1)",
    ] {
        assert_eq!(shown_result(text), "String", "{text}");
    }
    // A special built-in: shown as itself, and no value.
    assert_eq!(file_type("s = show"), "{s: builtin show}");
    let found = diagnostics("v = [show]");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "`show` is a built-in function and cannot be used as a value yet"
    );
    assert_eq!(
        found[0].notes,
        [
            "a built-in function with a type for any value cannot be written as a type yet; \
              call it, or bind it to a name"
        ]
    );
    // `len`'s note is as it was.
    assert_eq!(
        diagnostics("v = [len]")[0].notes,
        [
            "a built-in function with a type for any list cannot be written as a type yet; \
              call it, or bind it to a name"
        ]
    );
    // Its arguments bind as any call's.
    assert_eq!(messages("show()"), ["expected 1 argument, found 0"]);
    assert_eq!(messages("show(1, 2)"), ["expected 1 argument, found 2"]);
    assert_eq!(
        messages("show(v = 1)"),
        ["unknown argument `v`", "missing argument `value`"]
    );
    // `[]` alone has no element type to infer, as anywhere.
    assert_eq!(
        messages("show([])"),
        ["cannot infer the element type of `[]`"]
    );
    // Pure, and awaiting deeply as an interpolation does: in a pure
    // function a present argument is fine, its bare parameter made
    // present, and one that may be a future is the implicit-await error.
    assert_eq!(
        type_of_f("fn f(r: {a: Int}): String { show(r) }"),
        "fn(r: {a: Int}): String"
    );
    assert_eq!(
        type_of_f("fn f(xs: List<Int>): String { show(xs) }"),
        "fn(xs: List<Int>): String"
    );
    assert_eq!(
        messages("fn f(r: {a: Future<Int>}): String { show(r) }"),
        ["cannot use a value that may be a future in a pure function"]
    );
    assert_eq!(
        type_of_f("fn f(r: {a: Future<Int>}) -> String { show(r) }"),
        "fn(r: {a: Future<Int>} | Future<{a: Future<Int>}>) -> String"
    );
    // A value of type `Never` is never there to show.
    assert_eq!(shown_result("show(panic(\"p\"))"), "Never");
}

#[test]
fn show_rejects_a_function_anywhere_in_its_argument() {
    // A function anywhere in the argument's type is an error at the
    // argument, its type in a note: the function itself, a field, an
    // element, a member of a union, an alias's expansion, a built-in.
    for (text, argument, shown) in [
        (
            "show(fn(x: Int): Int { x })",
            "fn(x: Int): Int { x }",
            "`fn(x: Int): Int`",
        ),
        (
            "show({f = fn(): Int { 1 }})",
            "{f = fn(): Int { 1 }}",
            "`{f: fn(): Int}`",
        ),
        (
            "show([fn(): Int { 1 }])",
            "[fn(): Int { 1 }]",
            "`List<fn(): Int>`",
        ),
        (
            "let u: Int | fn(): Int = 1\nshow(u)",
            "u)",
            "`Int | (fn(): Int)`",
        ),
        (
            "type Handler = {run: fn(): Int}\nlet h: Handler = {run = fn(): Int { 1 }}\nshow(h)",
            "h)",
            "`{run: fn(): Int}`",
        ),
        ("show(int)", "int)", "`fn(x: Float): Int`"),
        (
            "show({e = exec})",
            "{e = exec}",
            "`{e: fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}}`",
        ),
        (
            "show(lazy fn(): Int { 1 })",
            "lazy fn(): Int { 1 }",
            "`Future<fn(): Int>`",
        ),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message, "cannot show a function: `show` renders data as source",
            "{text}"
        );
        // The argument of the last `show(` of the text.
        let argument = argument.trim_end_matches(')');
        let start = text.rfind("show(").unwrap() + "show(".len();
        let span = Span::new(
            u32::try_from(start).unwrap(),
            u32::try_from(start + argument.len()).unwrap(),
        );
        assert_eq!(&text[start..start + argument.len()], argument, "{text}");
        assert_eq!(found[0].span, span, "{text}");
        assert_eq!(
            found[0].notes,
            [format!("the argument is of type {shown}")],
            "{text}"
        );
        assert!(found[0].labels.is_empty(), "{text}");
    }
    // `len` as the argument is the alias rule's error, not this one.
    assert_eq!(
        messages("show(len)"),
        ["`len` is a built-in function and cannot be used as a value yet"]
    );
    // One error, not two, for a function that may be a future in a pure
    // function.
    assert_eq!(
        messages("fn f(g: Future<fn(): Int>): String { show(g) }"),
        ["cannot show a function: `show` renders data as source"]
    );
}

/// The type `exec` returns, as the type table shows it.
const EXEC_RESULT: &str = "{status: Int, stderr: String, stdout: String}";

#[test]
fn exec_and_panic_have_function_types() {
    assert_eq!(
        shown_result("exec"),
        "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}"
    );
    assert_eq!(shown_result("panic"), "fn(message: String): Never");
    assert_eq!(
        shown_result("import builtins\nbuiltins.exec"),
        shown_result("exec")
    );
    // The summary of slice 29.
    assert_eq!(
        file_type(
            "r = exec([\"ls\", \"-1\", \"src\"])\n\
                 lines = r.stdout\n\
                 ok = r.status == 0\n\
                 out = sh(\"git rev-parse HEAD\")"
        ),
        format!("{{lines: String, ok: Bool, out: String, r: {EXEC_RESULT}}}")
    );
    assert_eq!(
        type_of_f(
            "fn f(name: String): { build: Future<String> } { { build = lazy { sh(\"make {name}\") } } }"
        ),
        "fn(name: String | Future<String>): {build: Future<String>}"
    );
    assert_eq!(
        type_of_f("fn f(n: Int): Int {\nif n < 0 { panic(\"negative size {n}\") }\nn\n}"),
        "fn(n: Int): Int"
    );
    // Arguments bind by position or by name, as any call's.
    assert_eq!(shown_result("exec(argv = [\"true\"])"), EXEC_RESULT);
    assert_eq!(shown_result("panic(message = \"no\")"), "Never");
    assert_eq!(messages("exec([1])"), ["mismatched types"]);
    assert_eq!(messages("exec(\"true\")"), ["mismatched types"]);
    assert_eq!(messages("panic(1)"), ["mismatched types"]);
    assert_eq!(
        messages("exec(args = [\"true\"])"),
        ["unknown argument `args`", "missing argument `argv`"]
    );
    assert_eq!(messages("panic()"), ["expected 1 argument, found 0"]);
    // A value of the built-in's type: the display of each ordinary
    // built-in is the type table's, so that the two never drift.
    let mut types = TypeTable::new();
    let mut names = Names::new();
    for builtin in Builtin::ALL {
        let ty = builtin.ty(&mut types, &mut names);
        let shown = types.display(ty, &names).to_string();
        match builtin.typing() {
            Typing::Ordinary => assert_eq!(builtin.display(), shown),
            Typing::Special => {
                assert_eq!(builtin.display(), format!("builtin {}", builtin.name()));
            }
        }
        assert!(builtin.params().len() <= Builtin::MAX_PARAMS);
    }
}

#[test]
fn exec_is_impure_and_called_as_any_impure_function() {
    // From a pure function, the existing error.
    let text = "fn f(): Int { exec([\"true\"]).status }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, at(text, "exec([\"true\"])"));
    assert_eq!(
        labels(&found[0])[0],
        (
            at(text, "exec"),
            "this is fn(argv: List<String>) -> {status: Int, stderr: String, stdout: \
                 String}"
        )
    );
    // Through a variable or a parameter, the same.
    assert_eq!(
        messages("let e = exec\nfn f(): Int { e([\"true\"]).status }"),
        ["cannot call an impure function from a pure function"]
    );
    // From a `lazy` in a pure function, an impure or memoised function,
    // and the top level, it is fine.
    assert_eq!(
        type_of_f("fn f(): Future<Int> { lazy { exec([\"true\"]).status } }"),
        "fn(): Future<Int>"
    );
    assert_eq!(
        type_of_f("fn f() -> Int { exec([\"true\"]).status }"),
        "fn() -> Int"
    );
    assert_eq!(
        type_of_f("fn f(a: String) ->> String { exec([a]).stdout }"),
        "fn(a: String) ->> String"
    );
    assert_eq!(shown_result("exec([\"true\"]).status"), "Int");
    // As a value: passed, stored, called through a variable; it has no
    // equality, as no function has. Its parameter is a present list, so
    // it fits a written impure function type whose parameter says
    // `Present`, as an impure function declared so does, and not one
    // whose bare parameter may be a future.
    let pick = "fn pick(f: fn(argv: Present<List<String>>) -> {status: Int}): \
                    fn(Present<List<String>>) -> {status: Int} { f }\n";
    assert_eq!(
        shown_result(&format!("{pick}let e = pick(exec)\ne([\"true\"]).status")),
        "Int | Future<Int>"
    );
    assert_eq!(
        messages("fn pick(f: fn(argv: List<String>) -> {status: Int}): Int { 1 }\npick(exec)"),
        ["mismatched types"]
    );
    assert_eq!(
        shown_result("let r = { e = exec }\nlet e = r.e\ne([\"true\"]).stdout"),
        "String"
    );
    assert_eq!(messages("exec === exec"), ["cannot compare functions"]);
    assert_eq!(messages("panic === panic"), ["cannot compare functions"]);
}

#[test]
fn exec_live_is_typed_as_exec_and_impure() {
    assert_eq!(shown_result("exec_live"), shown_result("exec"));
    assert_eq!(
        shown_result("import builtins\nbuiltins.exec_live"),
        shown_result("exec")
    );
    assert_eq!(shown_result("exec_live([\"true\"])"), EXEC_RESULT);
    assert_eq!(shown_result("exec_live(argv = [\"true\"])"), EXEC_RESULT);
    assert_eq!(messages("exec_live([1])"), ["mismatched types"]);
    // From a pure function, the existing error, through its name or a
    // variable; from a `lazy`, an impure function or the top level, fine.
    let text = "fn f(): Int { exec_live([\"true\"]).status }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, at(text, "exec_live([\"true\"])"));
    assert_eq!(
        messages("let e = exec_live\nfn f(): Int { e([\"true\"]).status }"),
        ["cannot call an impure function from a pure function"]
    );
    assert_eq!(
        messages("fn f(): String { sh_live(\"true\") }"),
        ["cannot call an impure function from a pure function"]
    );
    assert_eq!(
        type_of_f("fn f(): Future<Int> { lazy { exec_live([\"true\"]).status } }"),
        "fn(): Future<Int>"
    );
    assert_eq!(
        type_of_f("fn f() -> String { sh_live(\"true\") }"),
        "fn() -> String"
    );
    assert_eq!(messages("exec_live === exec"), ["cannot compare functions"]);
}

#[test]
fn exec_awaits_a_future_argument_as_any_call_does() {
    // At the top level and in an impure function, a future list is
    // awaited at the top, as an argument of any function whose
    // parameter is a present `List<String>`.
    assert_eq!(
        marks("let xs = lazy [\"true\"]\nexec(xs)"),
        vec![("xs", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(xs: Future<List<String>>) -> Int { exec(xs).status }"),
        vec![("xs", AwaitMark::Top)]
    );
    // A list of futures is awaited deeply, as for any parameter of a
    // present `List<String>`.
    assert_eq!(
        marks("let xs = [lazy \"true\"]\nexec(xs)"),
        vec![("xs", AwaitMark::Deep)]
    );
    // In a pure function, the call is the purity error, and so is the
    // await, as for any call.
    assert_eq!(
        messages("fn f(xs: Future<List<String>>): Int { exec(xs).status }"),
        [
            "cannot call an impure function from a pure function",
            "cannot use a value that may be a future in a pure function"
        ]
    );
}

#[test]
fn a_call_of_panic_is_of_type_never() {
    assert_eq!(shown_result("panic(\"x\")"), "Never");
    assert_eq!(
        shown_result("let c = true\nif c { panic(\"x\") } else { 1 }"),
        "Int"
    );
    assert_eq!(
        shown_result("let c = true\nif c { 1 } else { panic(\"x\") }"),
        "Int"
    );
    // An `if` whose branch panics needs no `else`.
    assert_eq!(
        type_of_f("fn f(c: Bool): Int {\nif c { panic(\"x\") }\n1\n}"),
        "fn(c: Bool): Int"
    );
    // A `let` of it and a statement of it are fine; what follows is
    // never run, which is a warning, not an error.
    for text in ["let x = panic(\"no\")\n1", "panic(\"no\")\n1"] {
        let found = warnings(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, "this is never run", "{text}");
    }
    assert!(warnings("let x = panic(\"no\")").is_empty());
    assert!(warnings("panic(\"no\")").is_empty());
    // `panic` fits a parameter of type `fn(String): Int`, since `Never`
    // fits every result.
    assert_eq!(
        shown_result("fn apply(f: fn(String): Int): Int { f(\"a\") }\napply(panic)"),
        "Int"
    );
    assert_eq!(
        shown_result("let p = panic\nlet fs: List<fn(m: String): Int> = [p]\nfs"),
        "List<fn(m: String): Int>"
    );
    // Pure: a pure function may call it.
    assert_eq!(
        type_of_f("fn f(s: String): Int { panic(s) }"),
        "fn(s: String): Int"
    );
}

#[test]
fn exec_panic_and_sh_are_shadowed_as_any_prelude_name() {
    assert_eq!(file_type("exec = 1\nn = exec + 1"), "{exec: Int, n: Int}");
    assert_eq!(
        file_type("def sh(n: Int): Int { n * 2 }\nx = sh(2)"),
        "{sh: fn(n: Int): Int, x: Int}"
    );
    assert_eq!(shown_result("let panic = \"no\"\npanic"), "String");
    assert_eq!(
        shown_result("fn panic(m: String): Int { 0 }\npanic(\"x\")"),
        "Int"
    );
}

#[test]
fn a_built_in_s_arguments_are_bound_as_a_function_s() {
    let text = "len(1)";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "`len` takes a list, found Int");
    assert_eq!(found[0].span, at(text, "1"));
    assert_eq!(found[0].label.as_deref(), Some("this is Int"));
    assert_eq!(
        messages("len({ a = 1 })"),
        vec!["`len` takes a list, found `{a: Int}`"]
    );
    let text = "len([1], [2])";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "expected 1 argument, found 2");
    assert_eq!(found[0].span, Span::new(0, 13));
    assert_eq!(
        labels(&found[0]),
        vec![
            (at(text, "len"), "`len` is a built-in function"),
            (at(text, "[2]"), "unexpected argument")
        ]
    );
    let text = "len()";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "expected 1 argument, found 0");
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "len"), "`len` is a built-in function")]
    );
    let text = "len(lst = [1])";
    assert_eq!(
        errors(text),
        vec![
            ("unknown argument `lst`".to_owned(), at(text, "lst")),
            ("missing argument `list`".to_owned(), at(text, ")"))
        ]
    );
    let text = "len([1], list = [2])";
    assert_eq!(
        errors(text),
        vec![(
            "argument `list` is given twice".to_owned(),
            at(text, "list = [2]")
        )]
    );
    // Every argument is checked whatever goes wrong.
    assert_eq!(
        messages("len(y, z)"),
        vec![
            "expected 1 argument, found 2",
            "undefined variable `y`",
            "undefined variable `z`"
        ]
    );
}

#[test]
fn lists_compare_as_records_do() {
    assert_eq!(shown_result("[1] == [1]"), "Bool");
    assert_eq!(shown_result("[[1]] !== [[2]]"), "Bool");
    assert_eq!(shown_result("[{ a = 1 }] == [{ a = 2 }]"), "Bool");
    // Either elements are awaited deeply first.
    let text = "let xs = [1, lazy 2]\nxs == xs";
    assert_eq!(
        marks(text),
        vec![("xs", AwaitMark::Deep), ("xs", AwaitMark::Deep)]
    );
    // Their types must be identical, without a future or a function.
    assert_eq!(
        messages("[1] == [1.5]"),
        vec!["cannot compare `List<Int>` and `List<Float>`"]
    );
    let text = "let xs: List<Future<Int>> = [lazy 1]\nxs == xs";
    assert_eq!(
        messages(text),
        vec!["cannot compare `List<Future<Int>>`: it contains a future"]
    );
    let text = "let fs = [fn(): Int { 1 }]\nfs == fs";
    assert_eq!(
        messages(text),
        vec!["cannot compare `List<fn(): Int>`: it contains a function"]
    );
    // They have no order, and no arithmetic.
    let text = "[1] < [2]";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "cannot order `List<Int>`");
    assert_eq!(
        labels(&found[0]),
        vec![
            (at(text, "[1]"), "List<Int>"),
            (at(text, "[2]"), "List<Int>")
        ]
    );
    assert_eq!(
        messages("[1] + [2]"),
        vec!["cannot add `List<Int>` and `List<Int>`"]
    );
    assert_eq!(messages("-[1]"), vec!["cannot negate `List<Int>`"]);
    assert_eq!(messages("[true] and true"), vec!["mismatched types"]);
}

#[test]
fn interpolating_a_list_awaits_it_deeply() {
    let text = "let xs = [lazy 1]\n\"{xs}\"";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Deep)]);
    assert_eq!(shown_result(text), "String");
    let text = "let xs = [1]\n\"{xs}\"";
    assert_eq!(marks(text), vec![]);
}

// ----- Spreads -----

/// The span of the operand of the first spread `spread`, written with
/// its `...`, in `text`.
fn operand(text: &str, spread: &str) -> Span {
    let spread = at(text, spread);
    Span::new(spread.start + 3, spread.end)
}

/// The note on a spread that defines a field defined before it.
const SPREAD_TWICE_NOTE: &str =
    "a spread cannot replace a field defined before it; put the spread first";

#[test]
fn a_def_is_a_field_whose_value_is_the_function() {
    // A field of the record, in order among the other fields.
    assert_eq!(
        file_type("def f(): Int { 1 }\na = 2"),
        file_type("f = fn(): Int { 1 }\na = 2")
    );
    assert_eq!(
        file_type("a = 2\ndef f(n: Int) ->> Int { n }"),
        "{a: Int, f: fn(n: Int) ->> Int}"
    );
    // In an inner record block, from the next item on.
    assert_eq!(
        file_type("r = { def f(): Int { 1 }; g = f() }"),
        "{r: {f: fn(): Int, g: Int}}"
    );
    // Above a field its body reads: the body is checked when something
    // mentions the function, or after the last item, not at the
    // field's point, since a closure reads the field when it runs.
    assert_eq!(
        file_type("def f(): Int { v }\nv = 3"),
        "{f: fn(): Int, v: Int}"
    );
    assert_eq!(
        file_type("def f(): Int { v.a }\nlet k = 1\nv.a = k\nx = f()"),
        "{f: fn(): Int, v: {a: Int}, x: Int}"
    );
    assert_eq!(
        messages("x = f()\ndef f(): Int { v }\nlet k = 1\nv = k"),
        ["`f` cannot be used here yet"]
    );
    // A dotted `let` of a field's name is reported once.
    assert_eq!(
        messages("let z.w = 1\nz = 3"),
        ["`z` is a field of this file"]
    );
    // Its body sees its own name as the function being called.
    assert_eq!(
        file_type("def down(n: Int): Int { if n == 0 { 0 } else { down(n - 1) } }"),
        "{down: fn(n: Int): Int}"
    );
    // Defined twice, with a field and with another `def`.
    for text in [
        "def f(): Int { 1 }\nf = 2",
        "f = 2\ndef f(): Int { 1 }",
        "def f(): Int { 1 }\ndef f(): Int { 2 }",
    ] {
        assert_eq!(messages(text), ["field `f` is defined twice"], "{text}");
    }
    // It overrides a soft field from a spread (which keeps its place:
    // see the evaluator's tests).
    assert_eq!(
        file_type("...{f = 1; a = 2}\ndef f(): Int { 3 }"),
        "{a: Int, f: fn(): Int}"
    );
    // A block with a `def` is a record block.
    assert_eq!(
        messages("let x = { def f(): Int { 1 }; = 2 }"),
        ["a block with fields cannot have a result"]
    );
}

#[test]
fn top_level_functions_are_visible_in_the_whole_input() {
    // Used above its declaration.
    assert_eq!(file_type("x = f()\nfn f(): Int { 1 }"), "{x: Int}");
    assert_eq!(
        file_type("x = f()\ndef f(): Int { 1 }"),
        "{f: fn(): Int, x: Int}"
    );
    // Mutual recursion of two and of three, in any order.
    let even = "fn even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }";
    let odd = "fn odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
    for text in [
        format!("x = even(4)\n{even}\n{odd}"),
        format!("{odd}\nx = even(4)\n{even}"),
        format!("{even}\n{odd}\nx = even(4)"),
    ] {
        assert_eq!(file_type(&text), "{x: Bool}", "{text}");
    }
    assert_eq!(
        file_type(
            "x = a(9)\nfn a(n: Int): Int { if n == 0 { 0 } else { b(n - 1) } }\n\
                 fn b(n: Int): Int { c(n) }\nfn c(n: Int): Int { a(n) + 1 }"
        ),
        "{x: Int}"
    );
}

#[test]
fn a_top_level_function_outside_a_group_is_seen_resolved() {
    // `a`'s parameter `b` shadows the function `b`, so `a` does not
    // mention `b`: they are not a group, and `b` sees `a` resolved
    // (its result a future) in either order.
    let a = "fn a(b: Int) -> Int { lazy b }";
    let b = "fn b() -> Int { a(1) }";
    for text in [format!("{a}\n{b}\nx = b"), format!("{b}\n{a}\nx = b")] {
        assert_eq!(file_type(&text), "{x: fn() -> Future<Int>}", "{text}");
    }
    // Inside a group, a use of another member sees its declared type,
    // as a recursive call through a function's own name does, from
    // either member and in either order.
    let a = "fn a(n: Int) -> Int { if n == 0 { lazy 5 } else { b(n - 1) } }";
    let b = "fn b(n: Int) -> Int { a(n) }";
    let first = file_type(&format!("{a}\n{b}\nx = a\ny = b"));
    let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
    assert_eq!(first, format!("{{x: {either}, y: {either}}}"));
    // As a function calling itself is.
    assert_eq!(
        file_type("fn a(n: Int) -> Int { if n == 0 { lazy 5 } else { a(n - 1) } }\nx = a"),
        format!("{{x: {either}}}")
    );
    assert_eq!(first, file_type(&format!("{b}\n{a}\nx = a\ny = b")));
    assert_eq!(first, file_type(&format!("x = a\ny = b\n{a}\n{b}")));
}

#[test]
fn a_def_named_by_a_dotted_path_is_a_top_level_function() {
    // The leaf of its path, used above its declaration, recursive by
    // path, and in its prefix's record with the fields beside it.
    let summary = "total = dev.peios.count(4)\n\
             def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.count(n: Int): Int {\n\
             \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
             }\n\
             def dev.peios.name(): String { \"world\" }\n\
             dev.peios.version = \"1\"\n\
             greeting = dev.peios.hello()\n\
             all = dev.peios";
    let record = "{count: fn(n: Int): Int, hello: fn(): String, name: fn(): String, \
                      version: String}";
    assert_eq!(
        file_type(summary),
        format!("{{all: {record}, dev: {{peios: {record}}}, greeting: String, total: Int}}")
    );
    // With no lone file's prelude too.
    let (ast, types, output) = check_text(summary);
    let output = output.unwrap();
    assert_eq!(
        types.display(output.ty, ast.names()).to_string(),
        file_type(summary)
    );
    // Mutual recursion with a plain `def`, and between two dotted ones,
    // in any order.
    let even = "def even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }";
    let odd = "def dev.odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
    for text in [
        format!("x = dev.odd(4)\n{even}\n{odd}"),
        format!("{odd}\nx = even(4)\n{even}"),
    ] {
        assert_eq!(
            file_type(&text),
            "{dev: {odd: fn(n: Int): Bool}, even: fn(n: Int): Bool, x: Bool}",
            "{text}"
        );
    }
    let f = "def a.x.f(n: Int): Int { if n == 0 { 0 } else { a.y.g(n - 1) } }";
    let g = "def a.y.g(n: Int): Int { a.x.f(n) }";
    for text in [
        format!("{f}\n{g}\nr = a.x.f(2)"),
        format!("r = a.x.f(2)\n{g}\n{f}"),
    ] {
        assert!(file_type(&text).ends_with("r: Int}"), "{text}");
    }
    // There is no short name: inside `def dev.count`, `count` is not
    // in scope unless something else binds it.
    assert_eq!(
        errors("def dev.count(n: Int): Int { if n == 0 { 0 } else { count(n - 1) } }"),
        [("undefined variable `count`".to_owned(), Span::new(52, 57))]
    );
    assert_eq!(
        file_type(
            "fn count(n: Int): Int { n * 10 }\n\
                 def dev.count(n: Int): Int { count(n) }\nx = dev.count(2)"
        ),
        "{dev: {count: fn(n: Int): Int}, x: Int}"
    );
}

#[test]
fn a_dotted_def_is_typed_as_a_plain_def_is_by_its_group() {
    // Outside its group, seen resolved; inside, its declared type, from
    // either member and in either order, as a plain name is.
    let a = "def p.a(b: Int) -> Int { lazy b }";
    let b = "def p.b() -> Int { p.a(1) }";
    for text in [format!("{a}\n{b}\nx = p.b"), format!("x = p.b\n{b}\n{a}")] {
        assert!(
            file_type(&text).contains("x: fn() -> Future<Int>"),
            "{text}: {}",
            file_type(&text)
        );
    }
    let a = "def p.a(n: Int) -> Int { if n == 0 { lazy 5 } else { p.b(n - 1) } }";
    let b = "def p.b(n: Int) -> Int { p.a(n) }";
    let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
    for text in [
        format!("{a}\n{b}\nx = p.a\ny = p.b"),
        format!("x = p.a\ny = p.b\n{b}\n{a}"),
    ] {
        assert!(
            file_type(&text).contains(&format!("x: {either}, y: {either}")),
            "{text}: {}",
            file_type(&text)
        );
    }
    // A prefix read whole above the declaration has the type the body
    // gives the function, not the one its signature declares.
    let resolved = "def dev.get(n: Int) -> Int { n + 1 }";
    for text in [
        format!("r = dev\n{resolved}"),
        format!("{resolved}\nr = dev"),
    ] {
        assert_eq!(
            file_type(&text),
            "{dev: {get: fn(n: Int | Future<Int>) -> Int}, \
                 r: {get: fn(n: Int | Future<Int>) -> Int}}",
            "{text}"
        );
    }
    // A call by path is of a known callee once its group is checked: the
    // call-site form of the implicit-await error names the path.
    assert_eq!(
        messages("def dev.inc(n: Int): Int { n + 1 }\nfn p(f: Future<Int>): Int { dev.inc(f) }"),
        ["argument `f` would be awaited inside `dev.inc`, which is pure"]
    );
    // A pure dotted `def` calling an impure one is the existing error.
    let text = "def dev.p(): Int { dev.q() }\ndef dev.q() -> Int { 1 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, Span::new(19, 26));
    assert_eq!(
        labels(&found[0]),
        [
            (Span::new(19, 24), "this is fn() -> Int"),
            (
                Span::new(11, 12),
                "this function is pure, declared with `:`"
            ),
        ]
    );
}

#[test]
fn a_dotted_def_cannot_be_used_before_what_it_reads_is_bound() {
    // By its path, and by a prefix read whole, which holds its closure.
    let text = "x = dev.f()\nall = dev\nlet k = 1\ndef dev.f(): Int { k }";
    let found = diagnostics(text);
    let label = [(Span::new(22, 31), "`dev.f` reads `k`, which is bound here")];
    assert_eq!(found.len(), 2, "{found:?}");
    for (diagnostic, span) in found.iter().zip([Span::new(4, 9), Span::new(18, 21)]) {
        assert_eq!(diagnostic.message, "`dev.f` cannot be used here yet");
        assert_eq!(diagnostic.span, span);
        assert_eq!(labels(diagnostic), label);
    }
    // Through another function, the chain names each by its path.
    assert_eq!(
        messages("x = a.b()\nlet k = 1\ndef a.b(): Int { c.d() }\ndef c.d(): Int { k }"),
        ["`a.b` cannot be used here yet"]
    );
    let found = diagnostics("x = a.b()\nlet k = 1\ndef a.b(): Int { c.d() }\ndef c.d(): Int { k }");
    assert_eq!(
        labels(&found[0]),
        [(
            Span::new(10, 19),
            "`a.b` uses `c.d`, and `c.d` reads `k`, which is bound here"
        )]
    );
    // Below what it reads, it is fine.
    assert_eq!(
        file_type("let k = 1\ndef dev.f(): Int { k }\nx = dev.f()\nall = dev"),
        "{all: {f: fn(): Int}, dev: {f: fn(): Int}, x: Int}"
    );
}

#[test]
fn a_dotted_def_conflicts_as_a_leaf_does() {
    // With a field leaf, and with a field or a `def` at a prefix, in
    // either order: the existing texts at the later item.
    for (text, expected) in [
        (
            "def a.b(): Int { 1 }\na.b = 2",
            "field `a.b` is defined twice",
        ),
        (
            "a.b = 2\ndef a.b(): Int { 1 }",
            "field `a.b` is defined twice",
        ),
        (
            "def a.b(): Int { 1 }\ndef a.b(): Int { 2 }",
            "field `a.b` is defined twice",
        ),
        (
            "a = 1\ndef a.b(): Int { 1 }",
            "field `a` is already defined",
        ),
        ("def a.b(): Int { 1 }\na = 1", "field `a` is defined twice"),
        (
            "def a(): Int { 1 }\ndef a.b(): Int { 1 }",
            "field `a` is already defined",
        ),
        (
            "def a.b(): Int { 1 }\ndef a(): Int { 1 }",
            "field `a` is defined twice",
        ),
    ] {
        assert_eq!(messages(text), [expected], "{text}");
    }
    // As a dotted field does, with the same labels and notes.
    let shown = |text: &str| -> Vec<(String, Vec<String>, Vec<String>)> {
        diagnostics(text)
            .into_iter()
            .map(|d| {
                let labels = d.labels.iter().map(|l| l.text.clone()).collect();
                (d.message, labels, d.notes)
            })
            .collect()
    };
    for (dotted, field) in [
        (
            "def a.b(): Int { 1 }\na = 1",
            "a.b = fn(): Int { 1 }\na = 1",
        ),
        (
            "a = 1\ndef a.b(): Int { 1 }",
            "a = 1\na.b = fn(): Int { 1 }",
        ),
        (
            "def a.b(): Int { 1 }\na.b = 1",
            "a.b = fn(): Int { 1 }\na.b = 1",
        ),
    ] {
        assert_eq!(shown(dotted), shown(field), "{dotted}");
    }
    // A rejected one is no top-level function: the groups of the others
    // are found as ever, and nothing else is reported.
    assert_eq!(
        messages(
            "a.b = 1\ndef a.b(): Int { 1 }\nfn p(): Int { q() }\nfn q(): Int { p() }\n\
                 x = p"
        ),
        ["field `a.b` is defined twice"]
    );
    // Its root clashes as a dotted field's does.
    assert_eq!(
        messages("fn e(): Int { 1 }\ndef e.f(): Int { 1 }"),
        ["`e` is a function of this file"]
    );
    assert_eq!(
        messages("let g = 1\ndef g.h(): Int { 1 }"),
        ["`g` is a field of this file"]
    );
}

#[test]
fn a_dotted_def_is_shadowable_as_any_def_is() {
    // Shadowed by a later plain one and by a field, for the whole input.
    assert_eq!(
        file_type(
            "before = dev.x()\n#[shadowable] def dev.x(): String { \"a\" }\n\
                 def dev.x(): Int { 2 }"
        ),
        "{before: Int, dev: {x: fn(): Int}}"
    );
    assert_eq!(
        file_type("#[shadowable] def dev.y(): Int { 3 }\ndev.y = \"s\"\nz = dev.y"),
        "{dev: {y: String}, z: String}"
    );
    // A shadowed one is still checked as it is written, by itself.
    assert_eq!(
        messages("#[shadowable] def dev.x(): Int { \"no\" }\ndef dev.x(): Int { 2 }"),
        ["mismatched types"]
    );
    // A later declaration overlapping a plain one conflicts.
    assert_eq!(
        messages("def dev.x(): Int { 1 }\n#[shadowable] dev.x = 2"),
        ["field `dev.x` is defined twice"]
    );
}

#[test]
fn a_dotted_def_in_an_inner_block_is_its_dotted_field() {
    assert_eq!(
        file_type("r = { def b.c(n: Int): Int { n + 1 }\nd = b.c(1) }"),
        "{r: {b: {c: fn(n: Int): Int}, d: Int}}"
    );
    // Ordered: not visible above its item, and it sees no short name.
    assert_eq!(
        messages("r = { d = b.c(1)\ndef b.c(n: Int): Int { n + 1 } }"),
        ["undefined variable `b`"]
    );
    assert_eq!(
        messages("r = { def b.c(n: Int): Int { c(n) } }"),
        ["undefined variable `c`"]
    );
}

#[test]
fn a_top_level_functions_body_sees_the_bindings_above_it() {
    // Checked early, at the item above its declaration that uses it.
    assert_eq!(
        file_type("let k = 1\nx = g()\nfn g(): Int { k }"),
        "{x: Int}"
    );
    // Checked after the last item: the `k` above it, not the one below.
    let text = "let k = 1\nfn g(): Int { k }\nlet k = \"s\"\nx = k";
    assert_eq!(file_type(text), "{x: String}");
    // First used by an item below a `let` that is below the function:
    // it does not see that `let`.
    let text = "let k = 1\nfn g(): Int { k }\nlet k = \"s\"\nx = g()";
    assert_eq!(file_type(text), "{x: Int}");
}

#[test]
fn a_top_level_function_shadows_a_soft_field_of_its_name() {
    // The spread still defines the record's field `f`, but every read of
    // `f` in the input is the function, above the spread and below it.
    let spread = "let r = {f = 2; g = 3}";
    assert_eq!(
        file_type(&format!(
            "{spread}\n...r\nx = f()\ny = g\nfn f(): String {{ \"fn\" }}"
        )),
        "{f: Int, g: Int, x: String, y: Int}"
    );
    assert_eq!(
        file_type(&format!(
            "{spread}\nx = f()\n...r\nfn f(): String {{ \"fn\" }}"
        )),
        "{f: Int, g: Int, x: String}"
    );
    // A `def` of that name overrides the soft field, as a field does.
    assert_eq!(
        file_type(&format!(
            "{spread}\nx = f()\n...r\ndef f(): String {{ \"fn\" }}"
        )),
        "{f: fn(): String, g: Int, x: String}"
    );
}

/// Each error of `text`, as its message and the text it is at, then
/// each label's text and the text it is at, then each note.
fn explained(text: &str) -> Vec<String> {
    let at = |span: Span| {
        text.get(span.start as usize..span.end as usize)
            .unwrap_or("")
    };
    diagnostics(text)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| {
            let mut parts = vec![format!(
                "{} at `{}`",
                diagnostic.message,
                at(diagnostic.span)
            )];
            for label in &diagnostic.labels {
                parts.push(format!("{} at `{}`", label.text, at(label.span)));
            }
            parts.extend(diagnostic.notes.iter().cloned());
            parts.join(" | ")
        })
        .collect()
}

#[test]
fn a_top_level_function_is_used_once_what_it_reads_is_bound() {
    const BOUND: &str = "a function can be used once every variable it reads is bound";
    const MUTABLE: &str = "a function copies a mutable variable where the function is written";
    // Allowed: it copies nothing, or a `let` above the use.
    assert_eq!(file_type("x = f()\nfn f(): Int { 1 }"), "{x: Int}");
    assert_eq!(
        file_type("let k = 1\nx = f()\nfn f(): Int { k }"),
        "{x: Int}"
    );
    // Not allowed: a `let` below the use.
    assert_eq!(
        explained("let x = total()\nlet k = 10\nfn total(): Int { k }"),
        vec![format!(
            "`total` cannot be used here yet at `total` | `total` reads `k`, which is bound \
                 here at `let k = 10` | {BOUND}"
        )]
    );
    // A mutable, even above the use: it exists from its function's line.
    assert_eq!(
        explained("let mut n = 1\nx = f()\nfn f(): Int { n }"),
        vec![format!(
            "`f` cannot be used here yet at `f` | `f` reads the mutable variable `n`, so it \
                 exists from here at `fn f(): Int {{ n }}` | {MUTABLE}"
        )]
    );
    // Through one, two and four other functions.
    let chain = "let k = 1\nfn b(): Int { c() }\nfn c(): Int { d() }\nfn d(): Int { e() }\n\
                     fn e(): Int { k }";
    let reads = "reads `k`, which is bound here at `let k = 1`";
    assert_eq!(
        explained(&format!("x = d()\n{chain}")),
        vec![format!(
            "`d` cannot be used here yet at `d` | `d` uses `e`, and `e` {reads} | {BOUND}"
        )]
    );
    assert_eq!(
        explained(&format!("x = c()\n{chain}")),
        vec![format!(
            "`c` cannot be used here yet at `c` | `c` uses `d`, `d` uses `e`, and `e` \
                 {reads} | {BOUND}"
        )]
    );
    assert_eq!(
        explained(&format!("x = a()\nfn a(): Int {{ b() }}\n{chain}")),
        vec![format!(
            "`a` cannot be used here yet at `a` | `a` uses `b`, ..., and `e` {reads} | \
                 {BOUND}"
        )]
    );
    // Mentioned only inside a `lazy` or a function expression the item
    // builds.
    for item in ["x = lazy f()", "x = fn(): Int { f() }"] {
        let found = explained(&format!("{item}\nlet k = 1\nfn f(): Int {{ k }}"));
        assert_eq!(found.len(), 1, "{item}");
        assert!(
            found[0].starts_with("`f` cannot be used here yet"),
            "{item}"
        );
    }
    // No error where a parameter or a local shadows the name.
    for text in [
        "x = fn(f: Int): Int { f }\nlet k = 1\nfn f(): Int { k }",
        "x = { let f = 2; = f }\nlet k = 1\nfn f(): Int { k }",
        // Inside function bodies there is no check.
        "fn g(): Int { f() }\nlet k = 1\nfn f(): Int { k }",
    ] {
        assert!(
            check(&parse_text(text), &mut TypeTable::new()).is_ok(),
            "{text}"
        );
    }
}

#[test]
fn a_top_level_functions_closure_is_made_once_what_it_copies_is_bound() {
    let text = "let k = 1\nlet mut n = 2\nfn a(): Int { 1 }\nfn b(): Int { k }\n\
                    fn c(): Int { n }\nfn d(): Int { e() }\nfn e(): Int { 1 }";
    let ast = parse_text(text);
    let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
    let root = ast.items(ast.root());
    assert_eq!(
        checked.resolution.top_functions(),
        [
            // Copies nothing: made as the input starts.
            (root[2], MadeAt::Start),
            // Copies `k`: made just after its `let`.
            (root[3], MadeAt::After(root[0])),
            // Copies a mutable: made at its own declaration.
            (root[4], MadeAt::Own),
            // Reads another top-level function, which it does not copy.
            (root[5], MadeAt::Start),
            (root[6], MadeAt::Start),
        ]
    );
}

#[test]
fn a_body_checked_early_does_not_see_what_is_declared_below_the_use() {
    const BOUND: &str = "a function can be used once every variable it reads is bound";
    // The function reads the second `k`, the latest above itself.
    assert_eq!(
        explained("let k = 1\nx = g()\nlet k = 2\nfn g(): Int { k }"),
        vec![format!(
            "`g` cannot be used here yet at `g` | `g` reads `k`, which is bound here at \
                 `let k = 2` | {BOUND}"
        )]
    );
    // Declared again between the use and the function by `:=` and by a
    // dotted field.
    assert_eq!(
        explained("let r = {a = 1}\nx = g()\nr.a := 2\nfn g(): Int { r.a }"),
        vec![format!(
            "`g` cannot be used here yet at `g` | `g` reads `r`, which is bound here at \
                 `r.a := 2` | {BOUND}"
        )]
    );
    // A top-level field is order-free: `g` reads `r.a` through the top
    // level's frame, and `r.a` is computed before `x`.
    assert_eq!(
        file_type("x = g()\nr.a = 2\nfn g(): Int { r.a }"),
        "{r: {a: Int}, x: Int}"
    );
    // A spread between may define any name.
    assert_eq!(
        explained("let s = {port = 1}\nx = g()\n...s\nfn g(): Int { port }"),
        vec![format!(
            "`g` cannot be used here yet at `g` | `g` reads `port`, which this spread may \
                 define at `...s` | {BOUND}"
        )]
    );
    // The function is then erroneous for its users, and its body's
    // errors are reported once.
    let text = "x = g()\nlet k = 1\nfn g(): Int { k + true }\ny = g()";
    let found = messages(text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0], "`g` cannot be used here yet");
}

#[test]
fn a_top_level_functions_name_is_not_also_a_variable() {
    const NOTE: &str = "a function declared at the top level is visible in the whole \
                            file, so its name cannot also be a variable or a field";
    assert_eq!(
        explained("fn f(): Int { 1 }\nfn f(): Int { 2 }"),
        vec!["function `f` is declared twice at `f` | first declared here at `f`"]
    );
    for (text, at) in [
        ("fn f(): Int { 1 }\nlet f = 2", "f"),
        ("let f = 2\nfn f(): Int { 1 }", "f"),
        ("fn f(): Int { 1 }\nlet mut f = 2", "f"),
        ("fn f(): Int { 1 }\nf = 2", "f"),
        ("let mut x = 1\nfn f(): Int { 1 }\nf := 2", "f"),
        ("def f(): Int { 1 }\nlet f = 2", "f"),
    ] {
        let found = explained(text);
        assert_eq!(
            found.first(),
            Some(&format!(
                "`f` is a function of this file at `{at}` | declared here at `f` | {NOTE}"
            )),
            "{text}"
        );
    }
    // A `def` and a field, or two `def`s, are a field defined twice.
    for text in [
        "def f(): Int { 1 }\nf = 2",
        "def f(): Int { 1 }\ndef f(): Int { 2 }",
    ] {
        assert_eq!(messages(text), vec!["field `f` is defined twice"], "{text}");
    }
    // Inner blocks still shadow.
    assert_eq!(
        file_type("fn f(): Int { 1 }\nx = { let f = \"s\"; = f }"),
        "{x: String}"
    );
}

#[test]
fn a_top_level_field_cannot_also_be_a_variable() {
    const NOTE: &str = "a field declared at the top level is visible in the whole file, \
                            so its name cannot also be a variable";
    for text in [
        "a = 1\nlet a = 2",
        "let a = 2\na = 1",
        "a.b = 1\nlet mut a = 2",
        "let mut a = 2\na.b = 1",
        "let mut x = 1\na = 1\na := 2",
        "a.b = 1\nlet a.c = 2",
    ] {
        let found = explained(text);
        assert_eq!(
            found.first(),
            Some(&format!(
                "`a` is a field of this file at `a` | declared here at `a` | {NOTE}"
            )),
            "{text}"
        );
    }
    // A function and a field of one name: the function's error, at the
    // field, in either order.
    for text in ["a = 1\nfn a(): Int { 1 }", "fn a(): Int { 1 }\na.b = 1"] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some("`a` is a function of this file"),
            "{text}"
        );
    }
    // Inner blocks still shadow.
    assert_eq!(
        file_type("a = 1\nx = { let a = \"s\"; = a }"),
        "{a: Int, x: String}"
    );
}

#[test]
fn top_level_fields_are_visible_throughout_the_input() {
    let summary = "let major = 2\ncom.x.app.deps = [com.x.lib]\ncom.x.app.name = \
                       \"app-{version}\"\ncom.x.lib.name = \"lib\"\nversion = describe(major)\n\
                       fn describe(n: Int): String { \"v{n}\" }";
    assert_eq!(
        file_type(summary),
        "{com: {x: {app: {deps: List<{name: String}>, name: String}, lib: {name: String}}}, \
             version: String}"
    );
    // A field used above its declaration, from every kind of item and
    // position: allowed, since it reads nothing that stops it.
    for text in [
        "let x = y\ny = 1",
        "x = y\ny = 1",
        "{ let z = y }\ny = 1",
        "...r\nr = {a = 1}",
        "let x = lazy { y }\ny = 1",
        "let f = fn(): Int { y }\ny = 1",
        "let s = \"{y}\"\ny = 1",
        "let l = [1, 2]\nlet e = l[y]\ny = 0",
        "let mut m = 0\nm := y\ny = 1",
        "let l = for i in [y] { i }\ny = 1",
        "let l = for let i = y; i < y; i := i + y { i }\ny = 1",
        "let x = g(v = y)\nfn g(v: Int): Int { v }\ny = 1",
        "let x = g() { y }\nfn g(v: Int): Int { v }\ny = 1",
        // A leaf by its path while a sibling is not ready yet.
        "x = a.b\na.b = 1\nlet k = 2\na.c = k",
    ] {
        // `file_type` fails on an error.
        assert!(file_type(text).starts_with('{'), "{text}");
    }
    // Types of a root, a prefix, a leaf and an access beyond a leaf.
    assert_eq!(
        file_type("w = r\nx = r.q\ny = r.q.k\nz = r.q.k.m\nr.q.k = {m = 1}"),
        "{r: {q: {k: {m: Int}}}, w: {q: {k: {m: Int}}}, x: {k: {m: Int}}, y: {m: Int}, z: \
             Int}"
    );
    // A `def` as a leaf.
    assert_eq!(
        file_type("x = f()\ndef f(): Int { 1 }"),
        "{f: fn(): Int, x: Int}"
    );
}

#[test]
fn a_field_used_before_it_can_be_computed_is_an_error() {
    const FIELD: &str = "a field can be computed once every variable it reads is bound";
    const FUNCTION: &str = "a function can be used once every variable it reads is bound";
    let cases = [
        (
            "x = a\na.b = 1\nlet k = 2\na.c = k",
            format!(
                "`a` cannot be used here yet at `a` | `a` has `a.c`, and `a.c` reads `k`, \
                     which is bound here at `let k = 2` | {FIELD}"
            ),
        ),
        (
            "x = v\nlet k = 1\nv = k",
            format!(
                "`v` cannot be used here yet at `v` | `v` reads `k`, which is bound here \
                     at `let k = 1` | {FIELD}"
            ),
        ),
        (
            "let mut n = 1\nx = v\nv = n",
            "`v` cannot be used here yet at `v` | `v` reads the mutable variable `n`, so it \
                 is computed here at `v = n` | a field that reads a mutable variable is \
                 computed where it is written"
                .to_string(),
        ),
        (
            "x = v\nlet k = 1\nv = f()\nfn f(): Int { k }",
            format!(
                "`v` cannot be used here yet at `v` | `v` uses `f`, and `f` reads `k`, \
                     which is bound here at `let k = 1` | {FUNCTION}"
            ),
        ),
        (
            "x = g()\nlet k = 1\nv = k\nfn g(): Int { v }",
            format!(
                "`g` cannot be used here yet at `g` | `g` uses `v`, and `v` reads `k`, \
                     which is bound here at `let k = 1` | {FIELD}"
            ),
        ),
        (
            "x = v\n...{q = 1}\nv = q",
            format!(
                "`v` cannot be used here yet at `v` | `v` reads `q`, which this spread may \
                     define at `...{{q = 1}}` | {FIELD}"
            ),
        ),
        (
            "let k = 1\nx = v\nlet k = 2\nv = k",
            format!(
                "`v` cannot be used here yet at `v` | `v` reads `k`, which is bound here \
                     at `let k = 2` | {FIELD}"
            ),
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(explained(text), vec![expected], "{text}");
    }
    // Below the field: undefined, with the new label.
    assert_eq!(
        explained("v = k\nlet k = 1"),
        vec![
            "undefined variable `k` at `k` | `k` is declared here, below the field at `let \
                 k = 1` | a field's initialiser sees the variables declared above it"
                .to_string()
        ]
    );
}

#[test]
fn a_cycle_through_a_field_is_an_error() {
    const NOTE: &str = "a field cannot be computed from itself, even through a function";
    let cases = [
        (
            "a = b\nb = a",
            vec!["`a` uses `b` here at `b`", "`b` uses `a` here at `a`"],
        ),
        (
            "a = f()\nfn f(): Int { a }",
            vec!["`a` uses `f` here at `f`", "`f` uses `a` here at `a`"],
        ),
        (
            "a = f()\ndef f(): Int { a }",
            vec!["`a` uses `f` here at `f`", "`f` uses `a` here at `a`"],
        ),
        ("s = lazy { s }", vec!["`s` uses `s` here at `s`"]),
        (
            "a = b\nb = c\nc = a",
            vec![
                "`a` uses `b` here at `b`",
                "`b` uses `c` here at `c`",
                "`c` uses `a` here at `a`",
            ],
        ),
    ];
    for (text, labels) in cases {
        let found = explained(text);
        let first = text.split(' ').next().unwrap_or("");
        let expected = format!(
            "`{first}` depends on itself at `{first}` | {} | {NOTE}",
            labels.join(" | ")
        );
        assert_eq!(found, vec![expected], "{text}");
    }
    // Through an interior node: one error, at the leaf.
    let found = messages("a.b = a\nx = 1");
    assert_eq!(found, vec!["`a.b` depends on itself".to_string()]);
    // A cycle of functions alone is fine.
    assert_eq!(
        file_type("fn f(): Int { g() }\nfn g(): Int { f() }\nx = f()"),
        "{x: Int}"
    );
}

#[test]
fn a_spread_and_a_top_level_field_agree_in_either_order() {
    for text in ["...{a = 1}\na = \"s\"", "a = \"s\"\n...{a = 1}"] {
        assert_eq!(file_type(text), "{a: String}", "{text}");
    }
    for text in [
        "...{a = 1}\ndef a(): Int { 2 }",
        "def a(): Int { 2 }\n...{a = 1}",
    ] {
        assert_eq!(file_type(text), "{a: fn(): Int}", "{text}");
    }
    // A soft field as a dotted root, a leaf under a leaf, a leaf twice:
    // errors in either order.
    for (first, second) in [
        ("...{a = {b = 1}}", "a.c = 2"),
        ("a.b = 1", "a.b.c = 2"),
        ("a.b = 1", "a.b = 2"),
        ("a = 1", "a.b = 2"),
    ] {
        for text in [format!("{first}\n{second}"), format!("{second}\n{first}")] {
            assert_eq!(messages(&text).len(), 1, "{text}: {:?}", messages(&text));
        }
    }
    // Quoted names.
    for text in ["\"a\" = 1\nb = a", "b = a\n\"a\" = 1"] {
        assert_eq!(file_type(text).len(), "{a: Int, b: Int}".len(), "{text}");
    }
}

#[test]
fn an_undefined_variable_declared_below_the_function_says_so() {
    assert_eq!(
        explained("fn d(): Int { q }\nlet q = 2"),
        vec![
            "undefined variable `q` at `q` | `q` is declared here, below the function at \
                 `let q = 2` | a function sees the variables declared above it"
        ]
    );
    // Not for a name declared nowhere.
    assert_eq!(
        explained("fn d(): Int { q }"),
        vec!["undefined variable `q` at `q`"]
    );
}

#[test]
fn a_spread_defines_the_fields_of_its_record() {
    let text = "let defaults = { host = \"localhost\"; port = 80 }\n\
                    let conf = {\n  ...defaults\n  port = 8080\n  url = \"{host}:{port}\"\n}\n\
                    conf";
    assert_eq!(shown_result(text), "{host: String, port: Int, url: String}");
    // A block whose only item is a spread is a record block, and so is a
    // file's body.
    assert_eq!(shown_result("let r = { a = 1 }\n{ ...r }"), "{a: Int}");
    assert_eq!(file_type("...{ a = 1; b = \"s\" }"), "{a: Int, b: String}");
    // So it cannot have a result, with the existing error.
    let text = "let r = { a = 1 }\n{ ...r; = 2 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "a block with fields cannot have a result");
    assert_eq!(found[0].span, first_bytes(at(text, "= 2"), 1));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "...r"), "first field declared here")]
    );
    // Its final expression is a statement.
    assert_eq!(
        messages("...{ a = 1 }\na + 1"),
        vec!["unused value of type Int"]
    );
}

#[test]
fn a_spread_defines_exactly_the_fields_of_the_static_type() {
    // `y` is hidden by width subtyping, so it is neither a field nor a
    // name, and a field item may define it with another type.
    let hidden = "let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\n";
    assert_eq!(shown_result(&format!("{hidden}{{ ...p }}")), "{x: Int}");
    assert_eq!(
        messages(&format!("{hidden}{{ ...p; z = y }}")),
        vec!["undefined variable `y`"]
    );
    assert_eq!(
        shown_result(&format!("{hidden}{{ ...p; y = \"s\"; x = true }}")),
        "{x: Bool, y: String}"
    );
}

#[test]
fn an_ascribed_spread_selects_fields() {
    let r = "let r = { a = 2; b = 1 }\n";
    assert_eq!(
        shown_result(&format!("{r}{{ ...r : {{b: Int}} }}")),
        "{b: Int}"
    );
    assert_eq!(
        messages(&format!("{r}{{ ...r : {{b: Int}}; c = a }}")),
        vec!["undefined variable `a`"]
    );
    // A bare type in an ascription is present, so a field that may be a
    // future is awaited by it; the either shape keeps it lazy.
    let r = "let r: {a: Int, b: Int | Future<Int>} = { a = 1; b = lazy 2 }\n";
    let text = format!("{r}{{ ...r : {{b: Int}} }}");
    assert_eq!(shown_result(&text), "{b: Int}");
    assert_eq!(marks(&text), vec![("r", AwaitMark::Deep)]);
    let text = format!("{r}{{ ...r : {{b: Int | Future<Int>}} }}");
    assert_eq!(shown_result(&text), "{b: Int | Future<Int>}");
    assert_eq!(marks(&text), vec![]);
    // An ascribed literal warns about the field its type lacks, as any
    // ascribed literal does.
    let text = "{ ...{ a = 1; b = 2 } : {b: Int} }";
    assert_eq!(shown_result(text), "{b: Int}");
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is not in `{b: Int}`");
    assert_eq!(found[0].span, at(text, "a"));
}

#[test]
fn a_soft_field_may_be_overridden() {
    // By a field item, with another type: the field is then hard.
    assert_eq!(
        shown_result("let r = { a = 1; b = 2 }\n{ ...r; a = \"s\" }"),
        "{a: String, b: Int}"
    );
    // By a later spread, and then still soft.
    assert_eq!(
        shown_result("{ ...{ a = 1 }; ...{ a = \"s\"; b = 2 } }"),
        "{a: String, b: Int}"
    );
    assert_eq!(
        shown_result("{ ...{ a = 1 }; ...{ a = \"s\" }; a = true }"),
        "{a: Bool}"
    );
    // Every definition of the field writes its one slot; the spread's
    // operand is held in the next free slot, which it leaves free.
    let text = "let r = { a = 1; b = 2 }\n...r\na = \"s\"\nlet c = 3";
    let (ast, resolution) = resolved(text);
    // `a`, a top-level field, has slot 0 from the start of the input,
    // and the spread writes nothing for it.
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[1]), Slot(3));
    assert_eq!(resolution.spread_slots(items[1]), &[Slot::NONE, Slot(2)]);
    assert_eq!(resolution.item_slot(items[2]), Slot(0));
    assert!(resolution.spread_slots(items[2]).is_empty());
    assert_eq!(resolution.item_slot(items[3]), Slot(3));
    assert_eq!(resolution.frame_size(), 4);
    let (ast, resolution) = resolved("...{ a = 1 }\n...{ a = 2; b = 3 }");
    let items = ast.items(ast.root());
    assert_eq!(resolution.spread_slots(items[0]), &[Slot(0)]);
    assert_eq!(resolution.item_slot(items[0]), Slot(1));
    assert_eq!(resolution.spread_slots(items[1]), &[Slot(0), Slot(1)]);
    assert_eq!(resolution.item_slot(items[1]), Slot(2));
}

#[test]
fn a_spread_cannot_replace_a_hard_field() {
    let text = "let r = { a = 1 }\n{ a = 2; ...r }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(found[0].span, operand(text, "...r"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("`a` is a field of this, of type Int")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a = 2"), 1), "first defined here")]
    );
    assert_eq!(found[0].notes, vec![SPREAD_TWICE_NOTE]);
    // One error per spread, for the first such field in the type's
    // order, counting the others.
    let r = "let r = { a = 1; b = 2; c = 3 }\n";
    let text = format!("{r}{{ c = 0; a = 0; ...r }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(&text, "a = 0"), 1), "first defined here")]
    );
    assert_eq!(
        found[0].notes,
        vec![format!(
            "{SPREAD_TWICE_NOTE} (it also has 1 other field defined before it)"
        )]
    );
    let found = diagnostics(&format!("{r}{{ a = 0; b = 0; c = 0; ...r }}"));
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].notes,
        vec![format!(
            "{SPREAD_TWICE_NOTE} (it also has 2 other fields defined before it)"
        )]
    );
    // A dotted field is hard, and so is one a field item overrode.
    let text = "{ a.x = 1; ...{ a = 2 } }";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a.x"), 1), "first defined here")]
    );
    let text = "{ ...{ a = 1 }; a = 2; ...{ a = 3 } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].span, at(text, "{ a = 3 }"));
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a = 2"), 1), "first defined here")]
    );
    // The spread's other fields are still defined, and the block's
    // record is erroneous, so neither cascades.
    assert_eq!(
        messages("{ a = 1; ...{ a = 2; b = 3 }; c = b + 1 }"),
        vec!["field `a` is defined twice"]
    );
    assert_eq!(
        messages("let q = { a = 1; ...{ a = 2 } }\nq.z"),
        vec!["field `a` is defined twice"]
    );
    // A field item after a field item is defined twice as before; after
    // one that overrode a soft field, against that one.
    let text = "{ ...{ a = 1 }; a = 2; a = 3 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(found[0].span, first_bytes(at(text, "a = 3"), 1));
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a = 2"), 1), "first defined here")]
    );
}

#[test]
fn a_spreads_fields_are_names_in_scope() {
    // From the next item on, shadowing an outer variable of the name.
    assert_eq!(
        shown_result("let x = \"s\"\n{ ...{ x = 1 }; y = x + 1 }"),
        "{x: Int, y: Int}"
    );
    // Between a spread and an override the name is the spread's, and
    // after it the override's.
    assert_eq!(
        shown_result("{ ...{ a = 1 }; b = a; a = \"s\"; c = a }"),
        "{a: String, b: Int, c: String}"
    );
    assert_eq!(
        shown_result("{ ...{ a = 1 }; b = a; ...{ a = \"s\" }; c = a }"),
        "{a: String, b: Int, c: String}"
    );
    // A later `let` shadows the field without changing it.
    assert_eq!(
        shown_result("{ ...{ a = 1 }; let a = \"s\"; b = a }"),
        "{a: Int, b: String}"
    );
    // Not visible before its spread, nor after its block.
    assert_eq!(
        messages("{ b = a; ...{ a = 1 } }"),
        vec!["undefined variable `a`"]
    );
    assert_eq!(
        messages("let r = { ...{ a = 1 } }\na"),
        vec!["undefined variable `a`"]
    );
    // `:=` on one is the error for a field.
    let text = "let r = { a = 1 }\n{ ...r; a := 2 }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot rebind `a`: it is a field, not a variable"
    );
    assert_eq!(found[0].span, last(text, "a"));
    assert_eq!(labels(&found[0]), vec![(at(text, "...r"), "declared here")]);
}

#[test]
fn a_dotted_path_under_a_soft_field_is_an_error() {
    let text = "let r = { server = { port = 1 } }\n{ ...r; server.port = 2 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `server` is already defined");
    assert_eq!(found[0].span, at(text, "server.port"));
    assert_eq!(
        labels(&found[0]),
        vec![(operand(text, "...r"), "defined whole here")]
    );
    assert_eq!(found[0].notes, vec![WHOLE_FIELD_NOTE]);
    // A dotted `let` of the name updates the variable as ever.
    assert_eq!(
        shown_result(
            "let r = { server = { port = 1 } }\n\
                 { ...r; let server.port = \"s\"; p = server.port }"
        ),
        "{p: String, server: {port: Int}}"
    );
}

#[test]
fn a_soft_field_is_checked_against_the_expected_type() {
    let text = "let r = { a = 1 }\nlet x: {a: String} = { ...r }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, operand(text, "...r"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected String, found Int for field `a`")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(
            at(text, "{a: String}"),
            "expected because of this annotation"
        )]
    );
    assert!(found[0].notes.is_empty());
    // It cannot be awaited to fit.
    let text = "let r = { a = lazy 1 }\nlet x: {a: Int} = { ...r }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found Future<Int> for field `a`")
    );
    assert_eq!(
        found[0].notes,
        vec![
            "a spread's field cannot be awaited; define the field with its value, as in \
                 `a = r.a`"
        ]
    );
    assert_eq!(
        diagnostics("let x: {a: Int} = { ...{ a = lazy 1 } }")[0].notes,
        vec!["a spread's field cannot be awaited; define the field with its value"]
    );
    // Only the definition that stands is checked.
    assert_eq!(
        shown_result("let r = { a = 1 }\nlet x: {a: String} = { ...r; a = \"s\" }\nx"),
        "{a: String}"
    );
    assert_eq!(
        messages("let r = { a = \"s\" }\nlet x: {a: String} = { ...r; a = 1 }"),
        vec!["mismatched types"]
    );
    // A subtype fits.
    assert_eq!(
        shown_result("let r = { a = { b = 1; c = 2 } }\nlet x: {a: {b: Int}} = { ...r }\nx"),
        "{a: {b: Int}}"
    );
    // A soft field the type lacks is not warned about; a field item is.
    assert!(warnings("let r = { a = 1; z = 2 }\nlet x: {a: Int} = { ...r }").is_empty());
    let found = warnings("let x: {a: Int} = { ...{ a = 1 }; z = 2 }");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `z` is not in `{a: Int}`");
    // A missing field is reported as ever.
    let text = "let r = { a = 1 }\nlet x: {a: Int, b: Int} = { ...r }";
    assert_eq!(
        errors(text),
        vec![("missing field `b` in record".to_owned(), last(text, "}"))]
    );
}

#[test]
fn a_spreads_operand_is_measured_and_must_be_a_record() {
    // A future is awaited at the top, once.
    let text = "let f = lazy { a = 1; b = 2 }\n{ ...f; c = a + b }";
    assert_eq!(shown_result(text), "{a: Int, b: Int, c: Int}");
    assert_eq!(marks(text), vec![("f", AwaitMark::Top)]);
    // So is the either shape; nothing inside it is awaited.
    let text = "fn f(r: {a: Int}) -> {a: Int} { ...r }\nf";
    assert_eq!(marks(text), vec![("r", AwaitMark::Top)]);
    // A pure function's bare parameter is measured, which makes it
    // present.
    let text = "fn f(r: {a: Int}): {a: Int} { ...r }";
    assert_eq!(type_of_f(text), "fn(r: {a: Int}): {a: Int}");
    assert_eq!(demands(text, 0), vec![Some("r")]);
    // One in a field of the operand is measured where the expected type
    // is present, at the operand.
    let text = "fn f(x: Int): {a: Int} { ...{ a = x } }";
    assert_eq!(type_of_f(text), "fn(x: Int): {a: Int}");
    assert_eq!(demands(text, 0), vec![Some("{ a = x }")]);
    let text = "fn f(x: Int): {a: Int | Future<Int>} { ...{ a = x } }";
    assert_eq!(
        type_of_f(text),
        "fn(x: Int | Future<Int>): {a: Int | Future<Int>}"
    );
    assert_eq!(demands(text, 0), vec![None]);
    // In a pure function, one that may be a future is the purity error.
    let text = "fn f(r: Future<{a: Int}>): {a: Int} { ...r }";
    assert_implicit_await_error(text, last(text, "r"), "Future<{a: Int}>");
    // Anything else is not a record.
    let text = "{ ...5 }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot spread a value of type Int: it is not a record"
    );
    assert_eq!(found[0].span, at(text, "...5"));
    assert_eq!(labels(&found[0]), vec![(at(text, "5"), "this is Int")]);
    assert_eq!(
        messages("{ ...[1] }"),
        vec!["cannot spread a value of type `List<Int>`: it is not a record"]
    );
    assert_eq!(
        messages("let f = lazy 1\n{ ...f }"),
        vec!["cannot spread a value of type Int: it is not a record"]
    );
    // Neither that nor an erroneous operand cascades.
    assert_eq!(
        messages("let q = { ...5 }\nq.a + 1"),
        vec!["cannot spread a value of type Int: it is not a record"]
    );
    assert_eq!(
        messages("let q = { ...z; b = 1 }\nq.a"),
        vec!["undefined variable `z`"]
    );
    // `{}` spreads nothing.
    assert_eq!(shown_result("{ ...{} }"), "{}");
    assert_eq!(shown_result("let e = {}\n{ ...e; a = 1 }"), "{a: Int}");
}

#[test]
fn a_soft_field_in_an_inferred_result() {
    for (text, expected) in [
        // From the type the operand gives it, as it is.
        (
            "fn f() -> {a: Int, b: Int} { ...{ a = 1; b = lazy 2 } }",
            "fn() -> {a: Int, b: Future<Int>}",
        ),
        (
            "fn f(r: Present<{a: Int, b: Int}>) -> {a: Int, b: Int} { ...r }",
            "fn(r: {a: Int, b: Int}) -> {a: Int, b: Int}",
        ),
        (
            "fn f(r: {a: Int}) -> {a: Int} { ...r }",
            "fn(r: {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> {a: Int | \
                 Future<Int>}",
        ),
        (
            "fn f(r: {a: Int}) ->> {a: Int} { ...r }",
            "fn(r: {a: Int}) ->> {a: Int}",
        ),
        // An overridden one from the definition that stands.
        (
            "fn f() -> {a: Int, b: Int} { ...{ a = 1; b = lazy 2 }; b = 3 }",
            "fn() -> {a: Int, b: Int}",
        ),
        (
            "fn f() -> {a: Int, b: Int} { ...{ a = 1; b = 3 }; ...{ b = lazy 2 } }",
            "fn() -> {a: Int, b: Future<Int>}",
        ),
        // A field item before the spread, and a record block nested in
        // the result.
        (
            "fn f() -> {a: Int, b: Int} { a = lazy 1; ...{ b = 2 } }",
            "fn() -> {a: Future<Int>, b: Int}",
        ),
        (
            "fn f() -> {x: {a: Int}} { x = { ...{ a = lazy 1 } } }",
            "fn() -> {x: {a: Future<Int>}}",
        ),
        // The operand's other fields play no part.
        (
            "fn f() -> {a: Int} { ...{ a = 1; z = lazy 2 } }",
            "fn() -> {a: Int}",
        ),
    ] {
        assert_eq!(type_of_f(text), expected, "{text}");
    }
}

#[test]
fn a_soft_field_of_an_earlier_input_shares_its_slot() {
    let mut names = Names::new();
    let mut types = TypeTable::new();
    let parse_with = |text: &str, names: &mut Names| {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, strings) = tokenize_with(&source, names).unwrap();
        parse(&tokens, names.clone(), strings, &source).unwrap()
    };
    let first = parse_with("...{ a = 1; b = 2 }\na = \"s\"", &mut names);
    let scope = Checker::new(&first, &mut types).check().unwrap().scope;
    // Two bindings, in two slots: the top-level field `a`, declared as
    // the input starts, and `b`; the spread defines nothing for `a`.
    assert_eq!(scope.len(), 2);
    let second = parse_with("let c = a", &mut names);
    let output = Checker::with_scope(&second, scope, &mut types)
        .check()
        .unwrap();
    let resolution = &output.checked.resolution;
    let items = second.items(second.root());
    assert_eq!(resolution.item_slot(items[0]), Slot(2));
    let slot_names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| second.names().get(name))
        .collect();
    assert_eq!(slot_names, vec!["a", "b", "c"]);
    let ItemKind::Let { value, .. } = second.item(items[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(output.checked.types.get(value), TyId::STRING);
}

// ----- Ranges and `for` -----

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

// ----- Mutable variables -----

/// The item of `ast`, parsed from `text`, written first at `needle`.
fn item_at(ast: &Ast, text: &str, needle: &str) -> ItemId {
    let span = at(text, needle);
    (0..u32::try_from(ast.item_count()).unwrap())
        .map(ItemId::new)
        .find(|&id| ast.item(id).span == span)
        .unwrap_or_else(|| panic!("no item at {needle:?} in {text:?}"))
}

/// The first byte of `span`.
fn head(span: Span) -> Span {
    Span::new(span.start, span.start + 1)
}

/// The expression of `ast` that spans `span`.
fn expr_at(ast: &Ast, span: Span) -> ExprId {
    ast.expr_ids()
        .find(|&id| ast.get(id).span == span)
        .unwrap_or_else(|| panic!("no expression at {span:?}"))
}

/// The note on rebinding a variable that is not mutable from an inner
/// block, for a variable named `name`.
fn enclosing_note(name: &str) -> String {
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
fn ast_item_span(text: &str, start: u32) -> Span {
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

// ----- Updating an element -----

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

// ----- Unions -----

/// The message of each warning of `text`, which must check.
fn warning_messages(text: &str) -> Vec<String> {
    warnings(text).into_iter().map(|w| w.message).collect()
}

#[test]
fn written_unions_are_accepted() {
    assert_eq!(shown_result("let w: Int | String = 1\nw"), "Int | String");
    assert_eq!(
        shown_result("let w: {a: Int} | List<Int> | Bool = true\nw"),
        "Bool | {a: Int} | List<Int>"
    );
    // A `Future` member makes it the either shape of the others.
    assert_eq!(
        shown_result("let w: Int | String | Future<String | Int> = 1\nw"),
        "Int | String | Future<Int | String>"
    );
    assert_eq!(
        shown_result("let w: Future<Int | String> | String | Int = 1\nw"),
        "Int | String | Future<Int | String>"
    );
    // Anything else with a `Future` member is rejected, at the last `|`.
    let text = "let w: Int | String | Future<Int> = 1";
    assert_eq!(
        errors(text),
        vec![(
            "a union with a `Future` member must be `T | Future<T>`".to_owned(),
            last(text, "|")
        )]
    );
}

#[test]
fn unions_display_as_they_are_written_and_round_trip() {
    for ty in [
        "Int | String",
        "Bool | {a: Int} | List<Int>",
        "Int | (fn(): Int)",
        "(fn(): Int) | (fn(): String)",
        "Int | String | Future<Int | String>",
        "List<Int | String>",
        "{a: Int | String}",
        "fn(Int | String): Int | String",
        "Bool | (fn(): Int | String)",
    ] {
        let text = format!("let f = fn(x: Present<{ty}>): {{}} {{ }}\nf");
        assert_eq!(shown_result(&text), format!("fn(x: {ty}): {{}}"), "{ty}");
    }
    // Written in any order, they show in one.
    assert_eq!(
        shown_result("let f = fn(x: Present<List<Int> | String | {a: Int}>): {} { }\nf"),
        "fn(x: String | {a: Int} | List<Int>): {}"
    );
}

#[test]
fn bare_unions_follow_the_bare_type_rule() {
    // An impure function's parameter and result are the either shape of
    // the union, each member's inside following the rule too.
    assert_eq!(
        type_of_f("fn f(v: Int | {a: Int}) -> Int { 1 }"),
        "fn(v: Int | {a: Int | Future<Int>} | Future<Int | {a: Int | Future<Int>}>) -> Int"
    );
    // A memoised function's parameter is present; its result inferred.
    assert_eq!(
        type_of_f("fn f(v: Int | String) ->> Int | String { v }"),
        "fn(v: Int | String) ->> Int | String"
    );
    // A pure function's parameter is inferred: present if measured, the
    // either shape if passed through.
    assert_eq!(
        type_of_f("fn f(v: Int | String): Int { if v is Int { v } else { 0 } }"),
        "fn(v: Int | String): Int"
    );
    assert_eq!(
        type_of_f("fn f(v: Int | String): {k: Int | String | Future<Int | String>} { k = v }"),
        "fn(v: Int | String | Future<Int | String>): {k: Int | String | Future<Int | String>}"
    );
    // A function type member follows its own marker.
    assert_eq!(
        type_of_f("fn f(v: Int | fn(Int) -> Int) -> Int { 1 }"),
        "fn(v: Int | (fn(Int | Future<Int>) -> Int | Future<Int>) | Future<Int | (fn(Int | \
             Future<Int>) -> Int | Future<Int>)>) -> Int"
    );
    // The result of an impure function is inferred at its top.
    assert_eq!(
        type_of_f("fn f(c: Bool) -> Int | String { if c { 1 } else { \"a\" } }"),
        "fn(c: Bool | Future<Bool>) -> Int | String"
    );
    assert_eq!(
        type_of_f("fn f() -> Int | String { lazy 1 }"),
        "fn() -> Future<Int | String>"
    );
}

#[test]
fn a_widened_join_is_a_warning() {
    let text = "let v = if true { 1 } else { \"one\" }\nv";
    assert_eq!(shown_result(text), "Int | String");
    let warning = &warnings(text)[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(
        warning.notes,
        vec![
            "the `if` is `Int | String`; annotate it with that type if both are meant, or \
                 tell them apart with `is`"
        ]
    );
    // A join that finds a supertype is not widened.
    assert!(warnings("if true { { a = 1; b = 2 } } else { { a = 3 } }").is_empty());
    assert!(warnings("let u: Int | String = 1\nif true { u } else { 2 }").is_empty());
    // Literals adapt before the join.
    assert!(warnings("if true { 1 } else { 2.5 }").is_empty());
    assert_eq!(result_type("if true { 1 } else { 2.5 }"), TyId::FLOAT);
    // Lists: the elements, with the type the list has.
    let text = "let xs = [1, \"a\", true]\nxs";
    assert_eq!(shown_result(text), "List<Int | Bool | String>");
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].span, at(text, "\"a\""));
    assert_eq!(
        found[0].notes,
        vec![
            "the list is `List<Int | Bool | String>`; annotate it with that type if both \
                 are meant, or tell them apart with `is`"
        ]
    );
    // Inside: a record field the branches disagree on.
    let text = "if true { { a = 1; b = 2 } } else { { a = \"x\" } }";
    assert_eq!(shown_result(text), "{a: Int | String}");
    assert_eq!(warning_messages(text).len(), 1);
}

#[test]
fn an_expected_union_makes_no_join() {
    for text in [
        "let w: Int | String = if true { 1 } else { \"one\" }",
        "let w = (if true { 1 } else { \"one\" }) : Int | String",
        "fn f(v: Int | String): Int { 1 }\nf(if true { 1 } else { \"one\" })",
        "fn f(c: Bool): Int | String { if c { 1 } else { \"one\" } }",
        "let r: {a: Int | String} = { a = if true { 1 } else { \"one\" } }",
        "let w: Int | String = { let c = true\nif c { 1 } else { \"one\" } }",
        "let xs: List<Int | String> = [1, \"one\", if true { 2 } else { \"two\" }]",
        "let w: List<Int | String> | Bool = [1, \"one\"]",
    ] {
        assert!(warnings(text).is_empty(), "{text}: {:?}", warnings(text));
    }
    // Literals adapt to a union's numeric member.
    let text = "let w: Float | String = 1\nw";
    assert_eq!(shown_result(text), "Float | String");
    assert_eq!(all_types(text)[0], TyId::FLOAT);
    // A record block is checked against a union's one record member.
    assert_eq!(
        messages("let w: {a: Int} | String = { b = 1 }"),
        vec![
            "field `b` is not in `{a: Int}`",
            "missing field `a` in record"
        ]
    );
    // An annotation the value does not fit is still an error.
    assert_eq!(
        messages("let w: Int | String = true"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_misused_union_is_an_error_with_a_note() {
    let note = "tell them apart first, as in `if v is Int { ... }`";
    let union = "let v: Int | String = 1\n";
    for (rest, message) in [
        ("v + 1", "cannot add `Int | String` and Int"),
        ("v < 1", "cannot order `Int | String`"),
        ("-v", "cannot negate `Int | String`"),
        ("if v { 1 } else { 2 }", "mismatched types"),
        (
            "[1, 2][v]",
            "a list index must be an Int, found `Int | String`",
        ),
        ("v..3", "a range's bounds must be Int, found `Int | String`"),
        ("v.a", "cannot access field `a` of `Int | String`"),
        (
            "v[0]",
            "cannot index a value of type `Int | String`: it is not a list",
        ),
        (
            "for x in v { x }",
            "cannot iterate over a value of type `Int | String`: it is not a list",
        ),
        ("len(v)", "`len` takes a list, found `Int | String`"),
        ("v(1)", "cannot call a value of type `Int | String`"),
        ("v and true", "mismatched types"),
    ] {
        let text = format!("{union}{rest}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, message, "{text}");
        assert!(
            found[0].notes.iter().any(|n| n == note),
            "{text}: {:?}",
            found[0].notes
        );
    }
    // Not a variable: a placeholder name.
    let found = diagnostics("fn f(): Int | String { 1 }\nf() + 1");
    assert_eq!(
        found[0].notes,
        vec!["tell them apart first, as in `if x is Int { ... }`"]
    );
    // Functions alone cannot be told apart.
    let found =
        diagnostics("let g = if true { fn(a: Int): Int { a } } else { fn(): Int { 1 } }\ng(1)");
    assert_eq!(
        found
            .iter()
            .find(|d| d.severity == Severity::Error)
            .map(|d| d.notes.clone()),
        Some(vec![
            "only functions can be called".to_owned(),
            "`is` cannot tell function types apart; annotate the value with one function \
                 type that each of them fits"
                .to_owned()
        ])
    );
    // `==` needs one type that is not a union; `===` compares values.
    let found = diagnostics(&format!("{union}v == v"));
    assert_eq!(found[0].message, "cannot compare `Int | String` with `==`");
    assert_eq!(found[0].notes, vec!["narrow it, or compare with `===`"]);
    assert_eq!(result_type(&format!("{union}v === v")), TyId::BOOL);
    assert_eq!(result_type(&format!("{union}v !== v")), TyId::BOOL);
    // Interpolation shows any value.
    assert_eq!(result_type(&format!("{union}\"{{v}}\"")), TyId::STRING);
}

#[test]
fn records_and_lists_of_unions_are_used_as_their_members_agree() {
    // A field every member has: the join of its types, no warning.
    let text = "let r: {a: Int, b: Int} | {a: String} = { a = 1; b = 2 }\nr.a";
    assert_eq!(shown_result(text), "Int | String");
    assert!(warnings(text).is_empty());
    let found = diagnostics("let r: {a: Int} | {b: Int} = { a = 1 }\nr.b");
    assert_eq!(
        found[0].message,
        "cannot access field `b` of `{a: Int} | {b: Int}`"
    );
    assert_eq!(
        found[0].notes,
        vec!["tell them apart first, as in `if r is {a: Int} { ... }`"]
    );
    // Indexing, iterating and `len` of a union of lists.
    let lists = "let xs: List<Int> | List<String> = [1]\n";
    assert_eq!(shown_result(&format!("{lists}xs[0]")), "Int | String");
    assert_eq!(
        shown_result(&format!("{lists}for x in xs {{ x }}")),
        "List<Int | String>"
    );
    assert_eq!(result_type(&format!("{lists}len(xs)")), TyId::INT);
}

#[test]
fn is_tests_a_type() {
    let union = "let v: Int | List<Int> = 1\n";
    assert_eq!(result_type(&format!("{union}v is Int")), TyId::BOOL);
    assert!(warnings(&format!("{union}v is List<Int>")).is_empty());
    // A test that always passes is a warning.
    let text = "let n = 1\nn is Int";
    let found = warnings(text);
    assert_eq!(found[0].message, "`n` is always an Int");
    assert_eq!(found[0].span, at(text, "n is Int"));
    let found = warnings("let r = { a = 1 }\n(r) is {}");
    assert_eq!(found[0].message, "`r` is always a `{}`");
    // One that cannot is an error, at the type.
    let text = format!("{union}v is String");
    let found = diagnostics(&text);
    assert_eq!(found[0].message, "`v` is never a String");
    assert_eq!(found[0].span, last(&text, "String"));
    assert_eq!(
        labels(&found[0]),
        vec![(last(&text, "v"), "this is Int | List<Int>")]
    );
    assert_eq!(messages("[1, 2] is {}"), vec!["this value is never a `{}`"]);
    // Width: `{}` may be any member record.
    assert!(warnings("let x: Int | {a: Int} = 1\nx is {}").is_empty());
    // A member that overlaps the type may pass, though neither is a
    // subtype of the other.
    let text = "let x: {kind: String, value: Int | String} = { kind = \"n\"; value = 1 }\n\
                    x is {value: Int}";
    assert_eq!(result_type(text), TyId::BOOL);
    // Futures and function types cannot be tested.
    let found = diagnostics(&format!("{union}v is Future<Int>"));
    assert_eq!(found[0].message, "`is` cannot test a future");
    let found = diagnostics(&format!("{union}v is {{a: Int | Future<Int>}}"));
    assert_eq!(found[0].message, "`is` cannot test a future");
    let found = diagnostics(&format!("{union}v is fn(): Int"));
    assert_eq!(found[0].message, "`is` cannot test a function type");
    assert_eq!(
        messages(&format!("{union}v is Nope")),
        vec!["unknown type `Nope`"]
    );
}

#[test]
fn is_awaits_as_deep_as_its_plan_looks() {
    // At the top, for a test of the kind alone.
    assert_eq!(
        marks(
            "let x: List<Int | Future<Int>> | Int | Future<List<Int | Future<Int>> | Int> = 1\n\
                 x is Int"
        ),
        vec![("x", AwaitMark::Top)]
    );
    // Deeply, for one that looks at the elements.
    assert_eq!(
        marks("let x: List<Int | Future<Int>> | List<String> = []\nx is List<Int>"),
        vec![("x", AwaitMark::Deep)]
    );
    // In a pure function, an await is the purity error; a parameter is
    // measured instead.
    let text = "fn f(): Bool { let l: Int | String | Future<Int | String> = 1\nl is Int }";
    assert_eq!(
        messages(text),
        vec!["cannot use a value that may be a future in a pure function"]
    );
    assert_eq!(
        type_of_f("fn f(v: Int | String): Bool { v is Int }"),
        "fn(v: Int | String): Bool"
    );
    assert_eq!(
        demands("fn f(v: Int | String): Bool { v is Int }", 0),
        vec![Some("v")]
    );
}

#[test]
fn as_narrows_with_a_check() {
    let union = "let v: Int | String = 1\n";
    assert_eq!(result_type(&format!("{union}v as Int")), TyId::INT);
    // As precise as the value's type allows.
    assert_eq!(
        shown_result("let r: {a: Int, b: Int} | String = \"s\"\nr as {a: Int}"),
        "{a: Int, b: Int}"
    );
    let found = warnings("let n = 1\nn as Int");
    assert_eq!(
        found[0].message,
        "`as` changes nothing here: `n` is already an Int"
    );
    assert_eq!(found[0].notes, vec!["`: Int` says the same without a test"]);
    assert_eq!(
        messages(&format!("{union}v as Bool")),
        vec!["`v` is never a Bool"]
    );
    let found = diagnostics(&format!("{union}v as Future<Int>"));
    assert_eq!(found[0].message, "`as` cannot test a future");
    // It is pure, where its operand needs no await.
    assert_eq!(
        type_of_f("fn f(v: Int | String): Int { v as Int }"),
        "fn(v: Int | String): Int"
    );
    // Its resolution names what it tests.
    let (ast, resolution) = resolved(&format!("{union}v as Int"));
    let id = result_expr(&ast);
    let test = resolution.test_of(id).unwrap();
    assert_eq!(&*test.subject, "`v`");
    assert_eq!(&*test.target, "an Int");
    assert_eq!(test.test, ValueTest::Kind(ValueKind::Int));
    assert_eq!(test.slot, Slot::NONE);
}

#[test]
fn an_if_narrows_its_branches() {
    let text = "let v: Int | String = 1\nif v is Int { v + 1 } else { len([v]) }";
    assert_eq!(type_at(text, nth(text, "v", 2)), "Int");
    assert_eq!(type_at(text, nth(text, "v", 3)), "String");
    // The branches read a slot of their own, which the test writes,
    // after the standard library's.
    let (ast, _, output) = check_prelude(text);
    let resolution = output.unwrap().checked.resolution;
    let uses = uses(&ast, &resolution, "v");
    let base = standard_slots();
    assert_eq!(
        uses,
        vec![
            VarRef::Local(Slot(base)),
            VarRef::Local(Slot(base + 1)),
            VarRef::Local(Slot(base + 1))
        ]
    );
    // `else if` narrows the remainder in turn.
    let text = "let v: Int | String | Bool = 1\n\
                    if v is Int { 0 } else if v is String { len([v]) } else { if v { 1 } else { 2 } }";
    assert_eq!(type_at(text, nth(text, "v", 3)), "String");
    assert_eq!(type_at(text, nth(text, "v", 4)), "Bool");
    // A mutable variable, a parameter, a field and a `for`'s variable:
    // the use in the `then` branch, `{ n }`.
    let inside = |text: &str| {
        let block = at(text, "{ n }");
        Span::new(block.start + 2, block.start + 3)
    };
    for text in [
        "let mut n: Int | String = 1\nif n is Int { n } else { 0 }",
        "fn f(n: Int | String): Int { if n is Int { n } else { 0 } }",
        "{ n = if true { 1 } else { \"s\" }; b = if n is Int { n } else { 0 } }",
        "let ns: List<Int | String> = []\nfor n in ns { if n is Int { n } else { 0 } }",
    ] {
        assert_eq!(type_at(text, inside(text)), "Int", "{text}");
    }
    // The `else` branch keeps the variable when nothing always passes
    // and nothing was awaited, so a mutable variable is still one.
    let text = "let mut w: {a: Int} | String = \"s\"\n\
                    if w is {a: Int, b: Int} { 0 } else { w := \"t\"\n0 }";
    assert!(diagnostics_or_none(text).is_empty());
    // Where a member always passes, it is a copy of the remainder
    // there, which `:=` may not rebind, in either branch, at any depth,
    // by a plain or a dotted path: it would not change the variable.
    for text in [
        "let mut w: Int | String = 1\nif w is Int { 0 } else { w := \"t\"\n0 }",
        "let mut w: Int | String = 1\nif w is Int { w := 2\n0 } else { 0 }",
        "let mut w: Int | String = 1\nif w is Int { for i in 0..2 { w := i }\n0 } else { 0 }",
        "let w: Int | String = 1\nif w is Int { w := 2\n0 } else { 0 }",
        "let mut w: {a: Int} | String = \"s\"\nif w is String { 0 } else { w.a := 2\n0 }",
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(
            found[0].message, "cannot rebind `w` here: it is narrowed by `is`",
            "{text}"
        );
        let narrowing = if text.contains("w is Int") {
            at(text, "w is Int")
        } else {
            at(text, "w is String")
        };
        assert_eq!(
            labels(&found[0]),
            vec![(narrowing, "narrowed here")],
            "{text}"
        );
        assert_eq!(
            found[0].notes,
            vec![
                "in this branch `w` is a copy of the value that was tested, so rebinding it \
                     would not change the variable; test a copy under another name instead, as \
                     in `let v = w` and `if v is ...`, which leaves `w` free to rebind"
            ],
            "{text}"
        );
    }
    // Tested under another name, the variable is free to rebind, and a
    // `let` in the branch shadows the copy as any `let` does.
    for text in [
        "let mut w: Int | String = 1\nlet v = w\nif v is Int { w := v + 1 }",
        "let w: Int | String = 1\nif w is Int { let w = w + 1\nw } else { 0 }",
    ] {
        assert!(diagnostics_or_none(text).is_empty(), "{text}");
    }
    // ...and has the remainder when the test awaited the value.
    let text = "let q: Int | String | Future<Int | String> = 1\nif q is Int { q } else { q }";
    assert_eq!(type_at(text, nth(text, "q", 2)), "Int");
    assert_eq!(type_at(text, nth(text, "q", 3)), "String");
    // Through parentheses.
    let text = "let v: Int | String = 1\nif ((v) is Int) { v } else { 0 }";
    assert_eq!(type_at(text, nth(text, "v", 2)), "Int");
}

#[test]
fn other_conditions_narrow_nothing() {
    let union = "let v: Int | String = 1\nlet r = { a = v }\n";
    for rest in [
        "if v is Int and true { v + 1 } else { 0 }",
        "if not (v is Int) { 0 } else { v + 1 }",
        "if r.a is Int { r.a + 1 } else { 0 }",
    ] {
        let found = diagnostics(&format!("{union}{rest}"));
        assert!(
            found.iter().any(|d| d.severity == Severity::Error),
            "{rest}: {found:?}"
        );
    }
    // Nothing narrows after the `if`.
    let text = format!("{union}let n = if v is Int {{ v }} else {{ 0 }}\nv + 1");
    assert_eq!(messages(&text), vec!["cannot add `Int | String` and Int"]);
}

#[test]
fn a_union_of_functions_narrowed_in_else_is_called() {
    let text = "let g: Int | fn(): Int = fn(): Int { 7 }\nif g is Int { g } else { g() }";
    assert_eq!(result_type(text), TyId::INT);
}

/// The diagnostics of `text`, or none if it checks.
fn diagnostics_or_none(text: &str) -> Vec<Diagnostic> {
    check_prelude(text).2.err().unwrap_or_default()
}

// ----- Loops, `break`, `continue`, labels and Never -----

/// The type of the file's value as shown, after `xs`, a list of three
/// `Int`s, and `flag`, a `Bool`, are declared.
fn loop_type(text: &str) -> String {
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
fn warnings_of(text: &str) -> Vec<Diagnostic> {
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
fn result_expr_of_let(ast: &Ast) -> ExprId {
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

// ----- `return`, labelled blocks and loops that return -----

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

// ----- Parameter defaults -----

/// The constant of `default`, which must be one.
fn constant_of(default: &ParamDefault) -> Constant {
    match default {
        ParamDefault::Constant(constant) => constant.clone(),
        ParamDefault::Function { .. } => panic!("a default function: {default:?}"),
    }
}

/// The defaults of the function expression numbered `n` of `text`, as
/// the check found them, each a constant.
fn defaults_of(text: &str, n: u32) -> Vec<Option<Constant>> {
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
fn given_defaults(text: &str) -> Vec<(&str, Vec<(u32, Constant)>)> {
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
fn constant_str(text: &str) -> Constant {
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

// ----- Field defaults -----

/// Each record block of `text` that the check gave fields their
/// defaults, as its text, with each such field's name and default, in
/// source order.
fn filled(text: &str) -> Vec<(&str, Vec<(String, Constant)>)> {
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
fn name_of(text: &str, name: &str) -> Name {
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
const REACH_T: &str = "type T = { a: Int, b: Int = 2 }\n";

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
const SPREAD_NAMES: &str = "let none: List<T> = []\n\
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

/// Each error of `text` with its suggestion, as the text it replaces,
/// read from `text` at its span, and its replacement.
fn suggested(text: &str) -> Vec<(String, Option<(String, String)>)> {
    diagnostics(text)
        .into_iter()
        .map(|diagnostic| {
            let suggestion = diagnostic.suggestion.map(|found| {
                let span = found.span.start as usize..found.span.end as usize;
                (text[span].to_owned(), found.replacement)
            });
            (diagnostic.message, suggestion)
        })
        .collect()
}

/// The suggestion of the one error of `text`, as [`suggested`] shows it.
fn suggestion(text: &str) -> Option<(String, String)> {
    let found = suggested(text);
    assert_eq!(found.len(), 1, "{text}: {found:?}");
    found
        .into_iter()
        .next()
        .and_then(|(_, suggestion)| suggestion)
}

/// The suggestion to replace `written` with `replacement`, as
/// [`suggestion`] gives it: `written -> replacement`.
fn fix(fix: &str) -> Option<(String, String)> {
    let (written, replacement) = fix.split_once(" -> ")?;
    Some((written.to_owned(), replacement.to_owned()))
}

#[test]
fn an_undefined_variable_suggests_the_nearest_name_in_scope() {
    let found = suggested("let version = 1\nx = versoin + 1");
    assert_eq!(
        found,
        [(
            "undefined variable `versoin`".to_owned(),
            fix("versoin -> version")
        )]
    );
    // A parameter, a field of the block being built, a name of the
    // prelude.
    assert_eq!(
        suggestion("fn f(count: Int): Int { cuont }\nr = f(1)"),
        fix("cuont -> count")
    );
    assert_eq!(
        suggestion("r = { port = 80; next = prot + 1 }"),
        fix("prot -> port")
    );
    assert_eq!(suggestion("n = lne(\"abc\")"), fix("lne -> len"));
    // Differing in case only, which always qualifies.
    assert_eq!(
        suggestion("let Total = 1\nx = total"),
        fix("total -> Total")
    );
    // Too far from every name: none.
    assert_eq!(suggestion("let version = 1\nx = vrsn"), None);
    // A short name: never another short one, only a longer one an edit
    // away.
    assert_eq!(suggestion("let y = 1\nx = z"), None);
    assert_eq!(
        suggestion("let ab = 1\nlet idx = 2\nx = id"),
        fix("id -> idx")
    );
    // `x-1` is one name, as its note says, and not a misspelling of `x`.
    assert_eq!(suggestion("let count = 1\nx = count-1"), None);
}

#[test]
fn a_name_not_usable_yet_is_never_suggested() {
    // A `let` declared later in the block.
    assert_eq!(
        suggestion("fn f(): Int {\n    let a = valeu\n    let value = 1\n    a\n}\nr = f()"),
        None
    );
    // A top-level `let` below the function, which the error labels.
    let found = suggested("fn f(): Int { totl }\nlet total = 1\nr = f()");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].1, None);
    // The field being declared, which would read itself.
    assert_eq!(suggestion("count = coutn + 1"), None);
    // But another field, wherever it is declared.
    assert_eq!(
        suggestion("count = countr + 1\ncounter = 1"),
        fix("countr -> counter")
    );
    // The `fileroot` module, which is no value.
    assert_eq!(suggestion("import fileroot\nx = filerootthis"), None);
    // But a module misspelled where it is imported.
    assert_eq!(
        suggestion("import fileroo\nx = 1"),
        fix("fileroo -> fileroot")
    );
}

#[test]
fn a_tie_goes_to_the_nearer_scope_then_the_first_in_alphabetical_order() {
    assert_eq!(
        suggestion("let cart = 1\nlet card = 2\nx = car"),
        fix("car -> card")
    );
    assert_eq!(
        suggestion("let card = 1\nfn f(): Int {\n    let cart = 2\n    car\n}\nr = f()"),
        fix("car -> cart")
    );
}

#[test]
fn a_missing_field_suggests_the_nearest_field_written_as_it_must_be() {
    let found = suggested("let r = {port = 80; host = \"h\"}\nx = r.prot");
    assert_eq!(
        found,
        [(
            "no field `prot` in `{host: String, port: Int}`".to_owned(),
            fix("prot -> port")
        )]
    );
    // Quoted, the quotes replaced too; after a call, the name alone.
    assert_eq!(
        suggestion("let r = {port = 80}\nx = r.\"prot\""),
        fix("\"prot\" -> port")
    );
    assert_eq!(
        suggestion("fn g(): {port: Int} { {port = 80} }\nx = g().prot"),
        fix("prot -> port")
    );
    // A field whose name must be quoted is suggested quoted.
    assert_eq!(
        suggestion("let r = {\"a b\" = 1}\nx = r.\"a c\""),
        fix("\"a c\" -> \"a b\"")
    );
    // Differing in case only; too far.
    assert_eq!(
        suggestion("let r = {Port = 80}\nx = r.port"),
        fix("port -> Port")
    );
    assert_eq!(suggestion("let r = {port = 80}\nx = r.host"), None);
}

#[test]
fn an_unknown_type_suggests_a_type_that_can_be_written_there() {
    assert_eq!(suggestion("let x: Strng = \"a\""), fix("Strng -> String"));
    assert_eq!(suggestion("let x: int = 1"), fix("int -> Int"));
    // `List` takes a type argument: suggested only where one is given.
    assert_eq!(suggestion("let x: Lst<Int> = []"), fix("Lst -> List"));
    assert_eq!(suggestion("let x: Lst = 1"), None);
    assert_eq!(suggestion("let x: Strig<Int> = 1"), None);
}

#[test]
fn an_unknown_argument_suggests_a_parameter_no_other_argument_fills() {
    let text = "fn f(name: String, count: Int): Int { count }\nr = f(name = \"a\", cuont = 1)";
    let found = suggested(text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        found[0],
        ("unknown argument `cuont`".to_owned(), fix("cuont -> count"))
    );
    // `name` is filled by position already: suggesting it would give it
    // twice.
    let found = suggested("fn f(name: String, count: Int): Int { count }\nr = f(\"a\", nmae = 1)");
    assert_eq!(found[0].0, "unknown argument `nmae`");
    assert_eq!(found[0].1, None);
    // A built-in function's parameters.
    let found = suggested("r = exec(args = [\"true\"])");
    assert_eq!(
        found[0],
        ("unknown argument `args`".to_owned(), fix("args -> argv"))
    );
}

#[test]
fn a_record_literal_where_a_rest_field_is_expected_has_it() {
    // Each field the type does not name is checked against the
    // rest-field, and the record has it: no field is extra.
    let text = "let m: {*: String} = {\"bin/*\" = \"bin/*\"; b = \"x\"}\nm";
    assert_eq!(warnings(text), []);
    assert_eq!(shown_result(text), "{*: String}");
    let text = "let f: {to: String, *: Int} = {to = \"a\"; x = 1}\nf";
    assert_eq!(shown_result(text), "{to: String, *: Int}");
    // `{}`, and a literal that a block results in, an `if`'s branches, a
    // list's elements, a loop's results and an argument alike.
    for text in [
        "let e: {*: Int} = {}\ne",
        "let e: {*: Int} = { let x = 1; {a = x} }\ne",
        "let e: {*: Int} = if true { {a = 1} } else { {} }\ne",
        "let e: List<{*: Int}> = [{a = 1}, {}]\ne[0]",
        "let e: List<{*: Int}> = for n in [1, 2] { {a = n} }\ne[0]",
        "fn f(r: {*: Int}): {*: Int} { r }\nf({a = 1})",
    ] {
        assert_eq!(warnings(text), [], "{text}");
        assert_eq!(shown_result(text), "{*: Int}", "{text}");
    }
    // A nested literal is checked against the rest-field as any field's
    // value is.
    let text = "let e: {*: {*: Int}} = {a = {b = 1}}\ne";
    assert_eq!(warnings(text), []);
    assert_eq!(shown_result(text), "{*: {*: Int}}");
}

#[test]
fn a_field_a_rest_field_covers_must_fit_it() {
    let found = diagnostics("let m: {*: String} = {a = \"x\"; b = 1}");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, Span::new(35, 36));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected String, found Int")
    );
    // An annotated field, and a spread's.
    assert_eq!(
        messages("let m: {*: String} = {a: Int = 1}"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("let s = {a = 1}\nlet m: {*: String} = {...s}"),
        vec!["mismatched types"]
    );
    // A named field is exempt.
    assert_eq!(
        warnings("let f: {to: String, *: Int} = {to = \"a\"}\nf"),
        []
    );
    // A dotted path builds a record without one, which cannot have it.
    let found = diagnostics("let m: {*: {*: Int}} = {a.b = 1}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected {*: Int}, found {b: Int}")
    );
    assert_eq!(
        found[0].notes,
        vec![
            "dotted paths build a record without a rest-field; define the field whole, as \
             in `a = {...}`"
        ]
    );
}

#[test]
fn a_value_whose_type_hides_fields_does_not_fit_a_rest_field() {
    // `x` may hold fields its type does not show, of any type.
    assert_eq!(
        messages("let x = {a = 1}\nlet m: {*: Int} = x"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("fn f(r: {*: Int}): Int { 1 }\nlet x = {a = 1}\nf(x)"),
        vec!["mismatched types"]
    );
    // One whose type has a rest-field that fits does.
    assert_eq!(
        shown_result("let x: {a: Int, *: Int} = {a = 1}\nlet m: {*: Int} = x\nm"),
        "{*: Int}"
    );
}

#[test]
fn rest_field_errors() {
    assert_eq!(
        errors("1 : {*: Int, a: Int, *: String}"),
        vec![(
            "the rest-field appears twice in this type".to_owned(),
            Span::new(21, 22)
        )]
    );
    // A spread defines the fields its operand's type names.
    let found = diagnostics("let m: {*: Int} = {a = 1}\nlet c = {...m}");
    assert_eq!(
        found[0].message,
        "cannot spread a value of type `{*: Int}`: it has a rest-field"
    );
    // A field the rest-field covers is not read by name.
    let found = diagnostics("let m: {*: Int} = {a = 1}\nm.a");
    assert_eq!(found[0].message, "no field `a` in `{*: Int}`");
    assert_eq!(
        found[0].notes,
        vec!["a field that only the rest-field `*: ...` covers cannot be read by name"]
    );
    let found = diagnostics("let f: {to: String, *: Int} = {to = \"a\"}\nf.x");
    assert_eq!(
        found[0].notes,
        vec![
            "the only named field is `to`",
            "a field that only the rest-field `*: ...` covers cannot be read by name"
        ]
    );
    // A named field is read as ever.
    assert_eq!(
        shown_result("let f: {to: String, *: Int} = {to = \"a\"; x = 1}\nf.to"),
        "String"
    );
}

#[test]
fn a_rest_field_s_fields_are_tested_by_is_and_as() {
    // A record whose type hides its fields is tested at runtime; one that
    // can never be one is an error, as for any type.
    assert_eq!(
        shown_result("let x = {a = 1}\nlet y: {} = x\ny as {*: Int}"),
        "{*: Int}"
    );
    assert_eq!(
        messages("let f: {to: String, *: Int} = {to = \"a\"}\nf is {*: Int}"),
        vec!["`f` is never a `{*: Int}`"]
    );
}

// ----- Quantities -----

#[test]
fn quantities_have_their_own_arithmetic() {
    let cases = [
        ("30s", "Duration"),
        ("2GiB", "Size"),
        ("1s + 1m", "Duration"),
        ("1B - 1KiB", "Size"),
        ("2 * 1s", "Duration"),
        ("1s * 2", "Duration"),
        ("1s // 2", "Duration"),
        ("1h // 1m", "Int"),
        ("1h % 7m", "Duration"),
        ("-1s", "Duration"),
        ("1s < 2s", "Bool"),
        ("1s == 1000ms", "Bool"),
        ("\"{1s}\"", "String"),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
    // An untyped integer beside one is an `Int`.
    let types = all_types("1s * 2");
    assert!(types.contains(&TyId::INT), "{types:?}");
}

#[test]
fn quantities_mix_with_nothing_else() {
    let cases = [
        ("1s + 1", "cannot add Duration and Int"),
        ("1s + 1B", "cannot add Duration and Size"),
        ("1s * 1.5", "cannot multiply Duration and Float"),
        ("1s * 1s", "cannot multiply Duration and Duration"),
        ("1s / 2", "cannot divide Duration and Int"),
        ("1s % 2", "cannot take modulo of Duration and Int"),
        ("2 // 1s", "cannot floor-divide Int and Duration"),
        ("1s // 1B", "cannot floor-divide Duration and Size"),
        ("0 == 0s", "cannot compare Int and Duration"),
        ("1s < 1B", "cannot compare Duration and Size"),
        ("let x: Duration = 5\nx", "mismatched types"),
    ];
    for (text, message) in cases {
        assert_eq!(messages(text), [message], "{text}");
    }
    // `/` says to use `//`, and `//` and `%` what they take.
    let found = diagnostics("1B / 2");
    assert_eq!(
        found[0].notes,
        ["use `//`; a Size is a whole number of bytes"]
    );
    let found = diagnostics("1s % 2");
    assert_eq!(
        found[0].notes,
        [
            "a Duration divides with `//` by an Int, giving a Duration, or by a Duration, giving \
          an Int, and `%` gives the remainder of one Duration by another"
        ]
    );
    // `Duration` and `Size` are built-in type names.
    assert_eq!(messages("type Size = Int"), ["`Size` is a built-in type"]);
}

#[test]
fn a_quantity_literal_is_a_constant_default() {
    let text = "fn f(t: Duration = 30s, s: Size = -1KiB): Int { 0 }";
    assert_eq!(
        defaults_of(text, 0),
        [
            Some(Constant::Quantity(Quantity::Duration, 30_000_000_000)),
            Some(Constant::Quantity(Quantity::Size, -1024)),
        ]
    );
    // A field's default too, shown in its type.
    let text = "type T = {t: Duration = 1m30s}\nfn f(r: T): T { r }\nf";
    assert_eq!(
        file_type(text),
        "fn(r: {t: Duration = 1m30s}): {t: Duration = 1m30s}"
    );
}

// ----- Attributes -----

#[test]
fn the_checker_s_own_attributes_are_checked_without_a_workspace() {
    let text = "#[retry(3)] #[timeout(1m)]\nfn f() -> Int { 1 }\nf()";
    let (_, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(
        output.checked.resolution.attributes(FnId::new(0)),
        [
            Attribute {
                id: AttributeId::Builtin("retry"),
                args: Box::new([(Box::from("times"), Constant::Int(3))]),
            },
            Attribute {
                id: AttributeId::Builtin("timeout"),
                args: Box::new([(
                    Box::from("limit"),
                    Constant::Quantity(Quantity::Duration, 60_000_000_000)
                )]),
            },
        ]
    );
    let cases = [
        ("#[retry]", "`#[retry]` needs `times`"),
        ("#[retry(\"3\")]", "mismatched types"),
        ("#[retry(3, 4)]", "`#[retry]` takes one argument"),
        ("#[retry(tims = 3)]", "`#[retry]` has no argument `tims`"),
        ("#[retry(times = 1, times = 2)]", "`times` is given twice"),
        ("#[timeout(30)]", "mismatched types"),
        ("#[rety(3)]", "unknown attribute `rety`"),
        (
            "#[retry(1 + 2)]",
            "an attribute's argument must be a constant",
        ),
    ];
    for (attribute, message) in cases {
        let text = format!("{attribute}\nfn f() -> Int {{ 1 }}\nf()");
        assert_eq!(messages(&text), [message], "{attribute}");
    }
    // A misspelt one is suggested, and a declared one is unknown without a
    // workspace to declare it.
    let found = diagnostics("#[rety(3)]\nfn f() -> Int { 1 }\nf()");
    assert_eq!(
        found[0].suggestion.as_ref().map(|s| s.replacement.as_str()),
        Some("retry")
    );
}

#[test]
fn retry_and_timeout_need_arguments_they_can_mean() {
    let cases = [
        ("#[retry(-1)]", "`#[retry]` needs a `times` of at least 0"),
        ("#[timeout(0s)]", "`#[timeout]` needs a positive `limit`"),
        ("#[timeout(-1s)]", "`#[timeout]` needs a positive `limit`"),
    ];
    for (attribute, message) in cases {
        let text = format!("{attribute}\nfn f() -> Int {{ 1 }}\nf()");
        assert_eq!(messages(&text), [message], "{attribute}");
    }
    // Retrying a pure function is a warning.
    assert_eq!(
        warnings("#[retry(2)]\nfn f(): Int { 1 }\nf()")
            .iter()
            .map(|warning| warning.message.as_str())
            .collect::<Vec<_>>(),
        ["retrying a pure function cannot change its result"]
    );
    assert!(warnings("#[retry(2)]\nfn f() -> Int { 1 }\nf()").is_empty());
}

// ----- Generics -----

#[test]
fn a_call_of_a_generic_function_binds_its_type_parameters() {
    let cases = [
        ("fn id<T>(x: T): T { x }\nid(1)", "Int"),
        ("fn id<T>(x: T): T { x }\nid(\"a\")", "String"),
        ("fn id<T>(x: T): T { x }\nid([1.5])", "List<Float>"),
        (
            "fn pair<A, B>(a: A, b: B): {first: A, second: B} { { first = a; second = b } }\n\
             pair(1, \"x\")",
            "{first: Int, second: String}",
        ),
        (
            "fn first<T>(xs: List<T>, fallback: T): T { fallback }\nfirst([], \"none\")",
            "String",
        ),
        // An integer literal adapts to what another argument binds.
        (
            "fn first<T>(xs: List<T>, fallback: T): T { fallback }\nfirst([1.5], 0)",
            "Float",
        ),
        (
            "fn apply<T, U>(f: fn(x: T): U, x: T): U { f(x) }\n\
             fn double(n: Int): Int { n * 2 }\napply(double, 21)",
            "Int",
        ),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
    // A generic function's type shows its type parameters.
    let (ast, types, output) = check_prelude("def keep<T>(x: T): {v: T} { v = x }");
    let output = output.unwrap();
    assert_eq!(
        types.display(output.ty, ast.names()).to_string(),
        "{keep: fn<T>(x: T): {v: T}}"
    );
}

#[test]
fn a_generic_body_holds_its_type_parameters_opaque() {
    let cases = [
        ("fn f<T>(x: T): T { x + 1 }", "cannot add T and Int"),
        (
            "fn f<T>(x: T): String { \"{x}\" }",
            "cannot interpolate a value of type T, of a type parameter",
        ),
        ("fn f<T>(x: T): Int { x }", "mismatched types"),
        ("fn f<T>(x: T): Bool { x == x }", "cannot compare T and T"),
        (
            "fn id<T>(x: T): T { x }\nlet g = id",
            "a generic function must be called",
        ),
        (
            "fn none<T>(): List<T> { [] }\nnone()",
            "cannot infer `T` for this call of `none`",
        ),
        (
            "fn id<T>(x: T): T { x }\nlet x: Int = id(\"s\")\nx",
            "mismatched types",
        ),
    ];
    for (text, message) in cases {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
}

#[test]
fn arguments_binding_a_type_parameter_differently_widen_it_with_a_warning() {
    let pick = "fn pick<T>(a: T, b: T): T { a }\n";
    for (call, ty, warned) in [
        ("let a = 1\npick(a, \"s\")", "Int | String", true),
        ("pick(1, \"s\")", "Int | String", true),
        // A literal adapts to what the other argument binds.
        ("pick(2.5, 1)", "Float", false),
        ("pick(1, 2)", "Int", false),
        ("pick((1 : Int | String), \"s\")", "Int | String", false),
    ] {
        let text = format!("{pick}{call}");
        assert_eq!(file_type(&text), ty, "{text}");
        let messages = warning_messages(&text);
        assert_eq!(
            messages
                .iter()
                .any(|m| m == "the arguments for `T` have different types"),
            warned,
            "{text}: {messages:?}"
        );
    }
}

#[test]
fn a_built_in_type_takes_one_type_argument() {
    for (text, message) in [
        (
            "let x: List<Int, Int> = []\nx",
            "type `List` takes 1 type argument, given 2",
        ),
        (
            "let x: Future<Int, Int> = 1\nx",
            "type `Future` takes 1 type argument, given 2",
        ),
        (
            "let x: Int<String> = 1\nx",
            "type `Int` takes no type argument",
        ),
        ("let x: Nope<Int, Int> = 1\nx", "unknown type `Nope`"),
    ] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
}

// ----- Methods -----

#[test]
fn a_method_is_chosen_by_its_receiver_type() {
    let cases = [
        ("fn (x: Int) twice(): Int { x * 2 }\n3.twice()", "Int"),
        (
            "fn (x: Int) twice(): Int { x * 2 }\nlet a = 3\na.twice()",
            "Int",
        ),
        (
            "fn (xs: List<T>) first<T>(f: T): T { f }\n[\"a\"].first(\"b\")",
            "String",
        ),
        // The most specific receiver wins.
        (
            "fn (xs: List<T>) size<T>(): String { \"any\" }\nfn (xs: List<Int>) size(): Int { 1 }\n[1].size()",
            "Int",
        ),
        (
            "fn (xs: List<T>) size<T>(): String { \"any\" }\nfn (xs: List<Int>) size(): Int { 1 }\n[true].size()",
            "String",
        ),
        // A record with more fields than the receiver's has.
        (
            "fn (r: {a: Int}) get(): Int { r.a }\n{ a = 1; b = 2 }.get()",
            "Int",
        ),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
}

#[test]
fn method_call_errors() {
    for (text, message) in [
        (
            "fn (x: String) shout(): String { x }\n3.shout()",
            "no method `shout` for type Int",
        ),
        (
            "fn (r: {a: Int}) b(): Int { r.a }\nlet v = { a = 1; b = 2 }\nv.b()",
            "`b` is both a field of `{a: Int, b: Int}` and a method",
        ),
        (
            "fn (r: {a: Int}) m(): Int { 1 }\nfn (r: {b: Int}) m(): Int { 2 }\n{ a = 1; b = 2 }.m()",
            "method `m` is ambiguous for type `{a: Int, b: Int}`",
        ),
        (
            "fn (x: Int) m(): Int { 1 }\nfn (y: Int) m(): Int { 2 }\n1",
            "method `m` is declared twice for type Int",
        ),
        (
            "fn (r: {a: Int}) a(): Int { 1 }\n1",
            "method `a` has the name of a field of its receiver type `{a: Int}`",
        ),
        (
            "fn (x: Int) m(y: Int): Int { y }\n3.m(\"s\")",
            "mismatched types",
        ),
        ("fn (x: Int) m(): Int { x }\nm", "undefined variable `m`"),
    ] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
    let text = "def (x: Int) loud() -> Int { x }\nfn quiet(): Int { 3.loud() }\n1";
    let found = messages(text);
    assert!(
        found
            .iter()
            .any(|m| m == "cannot call an impure function from a pure function"),
        "{found:?}"
    );
}

#[test]
fn a_method_s_name_alone_notes_how_to_call_it() {
    let (_, _, output) = check_text("fn (x: Int) twice(): Int { x }\ntwice");
    let found = output.unwrap_err();
    let error = found
        .iter()
        .find(|d| d.message == "undefined variable `twice`")
        .unwrap();
    assert_eq!(
        error.notes,
        ["`twice` is a method: call it on a value, as in `x.twice()`"]
    );
}
