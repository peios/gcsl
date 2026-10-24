//! Tests of the checker: literals and numbers, variables and their scopes,
//! rebinding, annotations, statements and slots.

use super::*;

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

/// The parts of the expression `id`, a `with`, an assertion or a
/// `recover`, as they are evaluated: a `with`'s bindings' values and body,
/// an assertion's condition and message, a `recover`'s body and fallback.
pub(super) fn parts_of(ast: &Ast, id: ExprId) -> Vec<ExprId> {
    match ast.get(id).kind {
        ExprKind::With { bindings, body } => {
            let values = ast
                .with_bindings(bindings)
                .iter()
                .map(|binding| binding.value);
            values.chain([body]).collect()
        }
        ExprKind::Assert { cond, message } => vec![cond, message],
        ExprKind::Recover(recover) => {
            let decl = ast.recover_decl(recover);
            vec![decl.body, decl.fallback]
        }
        _ => Vec::new(),
    }
}

/// Every `let` in `body` and the blocks nested in it, in the order their
/// declarations complete.
pub(super) fn lets(ast: &Ast, body: ItemRange, out: &mut Vec<ItemId>) {
    for &id in ast.items(body) {
        match ast.item(id).kind {
            ItemKind::Let { value, .. } => {
                visit_lets(ast, value, out);
                out.push(id);
            }
            ItemKind::Rebind { value, .. }
            | ItemKind::Field { value, .. }
            | ItemKind::Spread(value)
            | ItemKind::Result(value)
            | ItemKind::Expr(value)
            | ItemKind::DefTypeDef { value, .. }
            | ItemKind::DefMeta { value, .. } => {
                visit_lets(ast, value, out);
            }
            ItemKind::Context { .. }
            | ItemKind::Break(_)
            | ItemKind::Continue(_)
            | ItemKind::Return(_)
            | ItemKind::Import(_)
            | ItemKind::Enum { .. }
            | ItemKind::Variant { .. }
            | ItemKind::Type { .. }
            | ItemKind::Error => {}
        }
    }
}

/// Every `let` in the blocks nested in the expression `id`, as [`lets`]
/// finds them.
pub(super) fn visit_lets(ast: &Ast, id: ExprId, out: &mut Vec<ItemId>) {
    match ast.get(id).kind {
        ExprKind::Block { items: body, .. } => lets(ast, body, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            visit_lets(ast, lhs, out);
            visit_lets(ast, rhs, out);
        }
        ExprKind::With { .. } | ExprKind::Assert { .. } | ExprKind::Recover(_) => {
            for part in parts_of(ast, id) {
                visit_lets(ast, part, out);
            }
        }
        ExprKind::Paren(inner)
        | ExprKind::Unary { operand: inner, .. }
        | ExprKind::Field { expr: inner, .. }
        | ExprKind::Ascribe { expr: inner, .. }
        | ExprKind::Is { expr: inner, .. }
        | ExprKind::As { expr: inner, .. }
        | ExprKind::NoField(inner) => visit_lets(ast, inner, out),
        ExprKind::If {
            cond, then, else_, ..
        } => {
            visit_lets(ast, cond, out);
            visit_lets(ast, then, out);
            visit_lets(ast, else_, out);
        }
        ExprKind::Interp(parts) => {
            for &part in ast.parts(parts) {
                if let StrPart::Expr(inner) = part {
                    visit_lets(ast, inner, out);
                }
            }
        }
        ExprKind::Fn(function) | ExprKind::Lazy(function) => {
            visit_lets(ast, ast.fn_decl(function).body, out);
        }
        ExprKind::Call { callee, args } | ExprKind::Unclosed { callee, args } => {
            visit_lets(ast, callee, out);
            for arg in ast.args(args) {
                visit_lets(ast, arg.value, out);
            }
        }
        ExprKind::List(elements) => {
            for element in ast.elements(elements) {
                visit_lets(ast, element.value, out);
            }
        }
        ExprKind::Index { expr, index } => {
            visit_lets(ast, expr, out);
            visit_lets(ast, index, out);
        }
        ExprKind::For(id) => {
            let decl = ast.for_decl(id);
            if let ForHead::Each {
                iterable, guard, ..
            } = decl.head
            {
                visit_lets(ast, iterable, out);
                if let Some(guard) = guard {
                    visit_lets(ast, guard, out);
                }
            }
            visit_lets(ast, decl.body, out);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Duration(_)
        | ExprKind::Size(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Name(_)
        | ExprKind::Variant { .. }
        | ExprKind::DefTypeRef(_)
        | ExprKind::ContextRead(_)
        | ExprKind::Error => {}
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
