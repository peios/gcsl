//! Tests of the checker: comparisons, logic, `if`, strings and bools.

use super::*;

/// Declares a variable of each type: `i`, `f`, `b`, `s` and `u`.
pub(super) const PRELUDE: &str =
    "let i = 1\nlet f = 1.5\nlet b = true\nlet s = \"s\"\nlet u = {}\n";

/// Each variable of [`PRELUDE`], its type, and the type as messages
/// show it.
pub(super) const VARIABLES: [(&str, TyId, &str); 5] = [
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
