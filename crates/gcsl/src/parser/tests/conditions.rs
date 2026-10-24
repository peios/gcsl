//! Tests of the parser: logic and comparisons, `is`, unions, `if`, and
//! interpolation.

use super::*;

#[test]
fn precedence_of_logic_and_comparison() {
    assert_eq!(shape("not a == b"), "(not (a == b))");
    assert_eq!(shape("a + 1 < b"), "((a + 1) < b)");
    assert_eq!(shape("a or b and c"), "(a or (b and c))");
    assert_eq!(shape("not not a"), "(not (not a))");
    assert_eq!(shape("-x < y : Int"), "((-x) < (y : Int))");
    assert_eq!(shape("not a and b"), "((not a) and b)");
    assert_eq!(shape("a and b or c and d"), "((a and b) or (c and d))");
    assert_eq!(shape("a or b or c"), "((a or b) or c)");
    assert_eq!(shape("a and b and c"), "((a and b) and c)");
    assert_eq!(shape("a == b and c != d"), "((a == b) and (c != d))");
    assert_eq!(shape("1 + 2 * 3 >= 7 // 2"), "((1 + (2 * 3)) >= (7 // 2))");
    assert_eq!(shape("not -x <= y"), "(not ((-x) <= y))");
    assert_eq!(shape("true or false"), "(true or false)");
    assert_eq!(
        shape("\"a\" + \"b\" == \"ab\""),
        "((\"a\" + \"b\") == \"ab\")"
    );
    // Every comparison operator, total equality included, at the same
    // level.
    for op in ["==", "!=", "===", "!==", "<", "<=", ">", ">="] {
        assert_eq!(shape(&format!("a {op} b")), format!("(a {op} b)"));
        assert_eq!(
            shape(&format!("a + 1 {op} b and c")),
            format!("(((a + 1) {op} b) and c)")
        );
    }
    assert_eq!(shape("not a === b"), "(not (a === b))");
}

#[test]
fn is_binds_as_comparisons_and_as_as_ascription() {
    // `is` takes in arithmetic, and is taken in by `not` and `and`.
    assert_eq!(shape("x + 1 is Int"), "((x + 1) is Int)");
    assert_eq!(shape("not x is Int"), "(not (x is Int))");
    assert_eq!(
        shape("x is Int and y is String"),
        "((x is Int) and (y is String))"
    );
    assert_eq!(shape("x is Int | String"), "(x is (Int | String))");
    assert_eq!(shape("x.a is {a: Int}"), "((x . a) is {a: Int})");
    // `as` binds as `:` does, and chains with it.
    assert_eq!(shape("a + b as Int"), "(a + (b as Int))");
    assert_eq!(shape("-x as Int"), "((-x) as Int)");
    assert_eq!(shape("x as Int : Int"), "((x as Int) : Int)");
    assert_eq!(shape("x : Int as Int"), "((x : Int) as Int)");
    assert_eq!(shape("x as Int is Int"), "((x as Int) is Int)");
    assert_eq!(shape("x as Int == 1"), "((x as Int) == 1)");
    // In a condition, the block after the type is the `if`'s.
    assert_eq!(shape("if x is Int { x }"), "(if (x is Int) {x})");
    assert_eq!(
        shape("if x is {a: Int} { x } else { 0 }"),
        "(if (x is {a: Int}) {x} else {0})"
    );
    // A newline after the keyword continues the item.
    assert_eq!(shape("x is\n  Int"), "(x is Int)");
    assert_eq!(shape("x as\n  Int"), "(x as Int)");
    let ast = parse_text("x is Int").unwrap();
    let printed = ast.pretty().to_string();
    assert!(printed.contains("(is Int @0..8"), "{printed}");
    let ast = parse_text("x as Int").unwrap();
    assert!(ast.pretty().to_string().contains("(as Int @0..8"));
}

#[test]
fn is_does_not_chain_with_comparisons() {
    for (text, at) in [
        ("a == b is Int", Span::new(7, 9)),
        ("x is Int == y", Span::new(9, 11)),
        ("x is Int is Bool", Span::new(9, 11)),
        ("a < b is Bool", Span::new(6, 8)),
    ] {
        assert_eq!(
            error(text),
            ("comparison operators cannot be chained".to_owned(), at),
            "{text}"
        );
    }
    assert_eq!(shape("(a == b) is Bool"), "([(a == b)] is Bool)");
    // Neither keyword can be a name.
    assert_eq!(error("let is = 1").0, "expected a name after `let`");
    assert_eq!(error("as").0, "expected expression, found `as`");
}

#[test]
fn written_unions_of_any_members() {
    for (text, shown) in [
        ("let x: Int | String = y", "let x: (Int | String) = y"),
        (
            "let x: {a: Int} | List<Int> | Bool = y",
            "let x: (({a: Int} | List<Int>) | Bool) = y",
        ),
        (
            "let x: (fn(): Int) | Int = y",
            "let x: (fn(): Int | Int) = y",
        ),
        (
            "let x: Int | String | Future<Int | String> = y",
            "let x: ((Int | String) | Future<(Int | String)>) = y",
        ),
    ] {
        assert_eq!(shape(text), shown, "{text}");
    }
}

#[test]
fn comparisons_cannot_be_chained() {
    let diagnostic = parse_text("a < b < c").unwrap_err();
    assert_eq!(diagnostic.message, "comparison operators cannot be chained");
    assert_eq!(diagnostic.span, Span::new(6, 7));
    assert_eq!(
        diagnostic.notes,
        vec!["write `a < b and b < c`, or add parentheses"]
    );
    assert_eq!(error("a == b != c").1, Span::new(7, 9));
    assert_eq!(error("1 + 2 < 3 == true").1, Span::new(10, 12));
    assert_eq!(error("x and a >= b > c").1, Span::new(13, 14));
    assert_eq!(error("not a < b <= c").1, Span::new(10, 12));
    // Total equality is non-associative too, with every comparison.
    assert_eq!(error("a === b === c").1, Span::new(8, 11));
    assert_eq!(error("a !== b == c").1, Span::new(8, 10));
    assert_eq!(error("a == b !== c").1, Span::new(7, 10));
    assert_eq!(shape("(a === b) !== c"), "([(a === b)] !== c)");
    // Parentheses and logic operators separate comparisons.
    assert_eq!(shape("(a < b) == c"), "([(a < b)] == c)");
    assert_eq!(shape("a == (b < c)"), "(a == [(b < c)])");
    assert_eq!(shape("a < b and b < c"), "((a < b) and (b < c))");
}

#[test]
fn items_continue_after_logic_and_comparison_operators() {
    for op in ["==", "!=", "===", "!==", "<", "<=", ">", ">=", "and", "or"] {
        let text = format!("a {op}\n\n  b");
        assert_eq!(shape(&text), format!("(a {op} b)"), "{text}");
    }
}

#[test]
fn if_shapes() {
    assert_eq!(shape("if a { 1 } else { 2 }"), "(if a {1} else {2})");
    assert_eq!(
        shape("if a { 1 } else if b { 2 } else { 3 }"),
        "(if a {1} else (if b {2} else {3}))"
    );
    assert_eq!(
        shape("if a { 1 } else if b { 2 }"),
        "(if a {1} else (if b {2}))"
    );
    assert_eq!(shape("if a { 1 }"), "(if a {1})");
    assert_eq!(shape("if a {}"), "(if a {})");
    assert_eq!(shape("if a == b and c { x }"), "(if ((a == b) and c) {x})");
    assert_eq!(shape("if { a } { 1 }"), "(if {a} {1})");
    assert_eq!(
        shape("let x = if a { 1 } else { 2 }"),
        "let x = (if a {1} else {2})"
    );
    // An `if` is an operand like any other prefix expression.
    assert_eq!(
        shape("1 + if a { 2 } else { 3 }"),
        "(1 + (if a {2} else {3}))"
    );
    assert_eq!(
        shape("if a { 1 } else { 2 } * 3"),
        "((if a {1} else {2}) * 3)"
    );
    assert_eq!(
        shape("if a { 1 } else { 2 } : Float"),
        "((if a {1} else {2}) : Float)"
    );
    assert_eq!(
        shape("if if a { b } else { c } { 1 }"),
        "(if (if a {b} else {c}) {1})"
    );
    // Blocks keep their items.
    assert_eq!(
        shape("if a {\n  let y = 1\n  y\n} else {\n  2\n}"),
        "(if a {let y = 1; y} else {2})"
    );
}

#[test]
fn else_may_start_a_later_line() {
    assert_eq!(shape("if a { 1 }\nelse { 2 }"), "(if a {1} else {2})");
    assert_eq!(
        shape("if a { 1 }\n\n# comment\nelse if b { 2 }\nelse { 3 }"),
        "(if a {1} else (if b {2} else {3}))"
    );
}

#[test]
fn without_else_the_newlines_still_separate_items() {
    assert_eq!(shape("if a { 1 }\n2"), "(if a {1}); 2");
    assert_eq!(shape("if a { 1 }\n\n\nx := 2"), "(if a {1}); x := 2");
    assert_eq!(shape("{ if a { 1 }\n}"), "{(if a {1})}");
    assert_eq!(shape("if a { 1 }\n"), "(if a {1})");
    // The empty else block sits at the end of the `if`'s block, and the
    // `if` spans only its keyword through that block.
    let ast = parse_text("if a { 1 }\n").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::If {
        else_, has_else, ..
    } = ast.get(root).kind
    else {
        panic!("expected an if");
    };
    assert!(!has_else);
    assert_eq!(ast.get(root).span, Span::new(0, 10));
    assert_eq!(ast.get(else_).span, Span::new(10, 10));
}

#[test]
fn if_errors() {
    assert_eq!(
        error("if a 1"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("if a\n{ 1 }"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(4, 5)
        )
    );
    assert_eq!(
        error("if (a) + 1"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(10, 10)
        )
    );
    assert_eq!(
        error("if a { 1 } else 2"),
        (
            "expected `{` or `if` after `else`".to_owned(),
            Span::new(16, 17)
        )
    );
    assert_eq!(
        error("if a { 1 } else\n{ 2 }"),
        (
            "expected `{` or `if` after `else`".to_owned(),
            Span::new(15, 16)
        )
    );
    assert_eq!(
        error("if { 1 }"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(8, 8)
        )
    );
    assert_eq!(
        error("else { 1 }"),
        (
            "expected expression, found `else`".to_owned(),
            Span::new(0, 4)
        )
    );
    assert_eq!(
        error("if a { 1 } 2"),
        (
            "expected end of item, found `2`".to_owned(),
            Span::new(11, 12)
        )
    );
}

#[test]
fn literals_of_the_new_types() {
    assert_eq!(shape("true; false"), "true; false");
    assert_eq!(shape("\"hi\""), "\"hi\"");
    assert_eq!(shape("r\"\\n\""), "\"\\n\"");
    let ast = parse_text("\"a\" + \"\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Binary { lhs, rhs, .. } = ast.get(root).kind else {
        panic!("expected a sum");
    };
    assert_eq!(ast.get(lhs).span, Span::new(0, 3));
    assert_eq!(ast.get(rhs).span, Span::new(6, 8));
}

#[test]
fn interpolation_parts() {
    assert_eq!(shape("\"a{x}b{y + 1}\""), "\"a<x>b<(y + 1)>\"");
    assert_eq!(shape("\"{ \"{x}\" }\""), "\"<\"<x>\">\"");
    assert_eq!(shape("\"{ {} }\""), "\"<{}>\"");
    assert_eq!(
        shape("\"status: {if ok { \"ok\" } else { \"failed\" }}\""),
        "\"status: <(if ok {\"ok\"} else {\"failed\"})>\""
    );
    let ast = parse_text("\"a{x}b{y}\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let interp = ast.get(root);
    assert_eq!(interp.span, Span::new(0, 10));
    let ExprKind::Interp(parts) = interp.kind else {
        panic!("expected an interpolation, got {interp:?}");
    };
    let parts = ast.parts(parts);
    let text = |part: &StrPart| match *part {
        StrPart::Lit(id) => ast.strings().get(id).to_owned(),
        StrPart::Expr(id) => format!("<{:?}>", ast.get(id).kind),
    };
    let shown: Vec<String> = parts.iter().map(text).collect();
    assert_eq!(shown.len(), 5);
    assert_eq!(shown[0], "a");
    assert!(shown[1].starts_with("<Name"));
    assert_eq!(shown[2], "b");
    assert!(shown[3].starts_with("<Name"));
    assert_eq!(shown[4], "");
}

#[test]
fn interpolation_errors() {
    let diagnostic = parse_text("\"{1 2}\"").unwrap_err();
    assert_eq!(diagnostic.message, "expected `}`, found `2`");
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Span::new(1, 2));
    assert_eq!(diagnostic.labels[0].text, "unclosed `{`");
    // In a later interpolation, the label is on that one's `{`.
    let diagnostic = parse_text("\"{a}{b 1}\"").unwrap_err();
    assert_eq!(diagnostic.labels[0].span, Span::new(4, 5));
    // An empty interpolation, and one missing its closing parenthesis,
    // are reported at the `}`.
    assert_eq!(
        error("\"{}\""),
        ("expected expression, found `}`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(
        error("\"{a}{}b\""),
        ("expected expression, found `}`".to_owned(), Span::new(5, 6))
    );
    assert_eq!(
        error("\"{(1}\""),
        ("expected `)`, found `}`".to_owned(), Span::new(4, 5))
    );
    // Only an expression may be interpolated.
    assert_eq!(
        message("\"{let x = 1}\""),
        "expected expression, found `let`"
    );
}

#[test]
fn pretty_output_for_a_program_with_strings_and_ifs() {
    let text = "let s = \"n: {n}\"\n\
                    if not a == b and c { s } else if d { \"x\" }";
    let ast = parse_text(text).unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let s @0..16\n\
             \x20   (interp @8..16\n\
             \x20     (lit \"n: \")\n\
             \x20     (name n @13..14)\n\
             \x20     (lit \"\")))\n\
             \x20 (expr @17..60\n\
             \x20   (if @17..60\n\
             \x20     (and @20..36\n\
             \x20       (not @20..30\n\
             \x20         (== @24..30\n\
             \x20           (name a @24..25)\n\
             \x20           (name b @29..30)))\n\
             \x20       (name c @35..36))\n\
             \x20     (block @37..42\n\
             \x20       (expr @39..40\n\
             \x20         (name s @39..40)))\n\
             \x20     (if @48..60\n\
             \x20       (name d @51..52)\n\
             \x20       (block @53..60\n\
             \x20         (expr @55..58\n\
             \x20           (str \"x\" @55..58)))\n\
             \x20       (block @60..60)))))"
    );
}

#[test]
fn else_if_chains_are_limited() {
    // A chain of N `if`s is N + 1 levels deep: the innermost `if` is one
    // above its blocks.
    let chain = |ifs: usize| format!("{}{{}}", "if a {} else ".repeat(ifs));
    let deepest = chain(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(chain(MAX_DEPTH));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Far beyond the limit, where uncounted recursion would overflow.
    let diagnostic = too_deep(chain(200_000));
    assert_eq!(diagnostic.message, "expression nested too deeply");
}

#[test]
fn interpolations_count_towards_depth() {
    // An interpolation is one level above its expression: around N
    // parentheses and a name, N + 2 levels. (Literals nested in each
    // other's interpolations are limited the same way; the lexer test
    // `nesting_is_limited` and the end-to-end depth test cover them.)
    let wrapped = |parens: usize| format!("\"{{{}x{}}}\"", "(".repeat(parens), ")".repeat(parens));
    let deepest = wrapped(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(wrapped(MAX_DEPTH - 1));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Two levels of literal around one name.
    let ast = parse_text("\"{\"{x}\"}\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Interp(parts) = ast.get(root).kind else {
        panic!("expected an interpolation");
    };
    let StrPart::Expr(inner) = ast.parts(parts)[1] else {
        panic!("expected an expression part");
    };
    assert!(matches!(ast.get(inner).kind, ExprKind::Interp(_)));
}
