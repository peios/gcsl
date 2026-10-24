//! Tests of the parser: literals and operators, items and their separators,
//! blocks, spans, errors and the limits of nesting.

use super::*;

#[test]
fn literals() {
    assert_eq!(shape("42"), "42");
    assert_eq!(shape("2.5"), "2.5");
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    assert_eq!(shape("1 + 2 * 3"), "(1 + (2 * 3))");
    assert_eq!(shape("1 * 2 + 3"), "((1 * 2) + 3)");
    assert_eq!(shape("1 - 6 / 3 // 2 % 5"), "(1 - (((6 / 3) // 2) % 5))");
}

#[test]
fn binary_operators_are_left_associative() {
    assert_eq!(shape("8 - 3 - 2"), "((8 - 3) - 2)");
    assert_eq!(shape("1 + 2 - 3 + 4"), "(((1 + 2) - 3) + 4)");
    assert_eq!(shape("2 * 3 // 4"), "((2 * 3) // 4)");
    assert_eq!(shape("8 / 4 / 2"), "((8 / 4) / 2)");
    assert_eq!(shape("9 % 5 * 2"), "((9 % 5) * 2)");
}

#[test]
fn unary_minus_binds_tighter_than_multiplication() {
    assert_eq!(shape("-2 * 3"), "((-2) * 3)");
    assert_eq!(shape("-2 + 3"), "((-2) + 3)");
    assert_eq!(shape("-x * y"), "((-x) * y)");
}

#[test]
fn nested_unary_minus() {
    // Negating a negation needs parentheses: `--2` and `--x` are lexical
    // errors (reported through `parse_text`, which lexes first), and
    // `- - -2` starts with a subtraction `-`.
    assert_eq!(shape("-(-2)"), "(-[(-2)])");
    assert_eq!(shape("-(-x)"), "(-[(-x)])");
    assert_eq!(
        error("- - -2"),
        ("expected expression, found `-`".to_owned(), Span::new(0, 1))
    );
    for text in ["--2", "--x", "1 + --x"] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(diagnostic.message, "unexpected character `-`", "{text}");
    }
    assert_eq!(error("--x").1, Span::new(0, 1));
}

#[test]
fn spaced_minus_cannot_start_an_expression() {
    assert_eq!(
        error("- 2 * 3"),
        ("expected expression, found `-`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("1 * - 2"),
        ("expected expression, found `-`".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn unary_minus_inside_binary() {
    assert_eq!(shape("2 * -3"), "(2 * (-3))");
    assert_eq!(shape("2 - -3"), "(2 - (-3))");
}

#[test]
fn negation_directly_after_an_item_is_reported_as_a_likely_subtraction() {
    let diagnostic = parse_text("a -b").unwrap_err();
    assert_eq!(diagnostic.message, "expected end of item, found `-b`");
    assert_eq!(diagnostic.span, Span::new(2, 4));
    assert_eq!(
        diagnostic.notes,
        vec!["write `a - b` with spaces for subtraction"]
    );
    let diagnostic = parse_text("let x = 1 -2.5").unwrap_err();
    assert_eq!(diagnostic.message, "expected end of item, found `-2.5`");
    assert_eq!(diagnostic.span, Span::new(10, 14));
    let diagnostic = parse_text("{ x -hello-world }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected end of item, found `-hello-world`"
    );
}

#[test]
fn parentheses_group() {
    assert_eq!(shape("(1 + 2) * 3"), "([(1 + 2)] * 3)");
    assert_eq!(shape("((1))"), "[[1]]");
    assert_eq!(shape("2 * (3 - (4 + 5))"), "(2 * [(3 - [(4 + 5)])])");
    assert_eq!(shape("-(1 + 2)"), "(-[(1 + 2)])");
}

#[test]
fn names() {
    assert_eq!(shape("x"), "x");
    assert_eq!(shape("hello-world + x-1"), "(hello-world + x-1)");
    assert_eq!(shape("x - 1"), "(x - 1)");
}

#[test]
fn items_are_separated_by_newlines_and_semicolons() {
    assert_eq!(shape("let x = 1\nx"), "let x = 1; x");
    assert_eq!(shape("let x = 1; x"), "let x = 1; x");
    assert_eq!(shape("let x = 1\nx := 2; = x"), "let x = 1; x := 2; = x");
    // Blank lines, comment-only lines and repeated separators.
    assert_eq!(
        shape("\n\n# comment\nlet x = 1;;\n\n  ;\n# another\n x ;\n"),
        "let x = 1; x"
    );
    assert_eq!(shape(";"), "");
    assert_eq!(shape(""), "");
    assert_eq!(shape("# only a comment"), "");
}

#[test]
fn items_continue_after_a_trailing_operator() {
    for op in ["+", "-", "*", "/", "//", "%"] {
        let text = format!("1 {op}\n\n  2");
        assert_eq!(shape(&text), format!("(1 {op} 2)"), "{text}");
    }
    assert_eq!(shape("1 :\nFloat"), "(1 : Float)");
    // A comment after the operator does not end the item either.
    assert_eq!(shape("1 + # more\n2"), "(1 + 2)");
}

#[test]
fn items_continue_after_eq_colon_eq_and_let() {
    assert_eq!(shape("let x =\n1"), "let x = 1");
    assert_eq!(shape("let\nx = 1"), "let x = 1");
    assert_eq!(shape("let x:\nInt = 1"), "let x: Int = 1");
    assert_eq!(shape("x :=\n\n1"), "x := 1");
    assert_eq!(shape("=\n1"), "= 1");
}

#[test]
fn newlines_elsewhere_end_the_item() {
    // A leading operator on the next line does not continue the item.
    assert_eq!(
        error("1\n+ 2"),
        ("expected expression, found `+`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(
        error("let x\n= 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(5, 6)
        )
    );
}

#[test]
fn items_continue_inside_parentheses() {
    assert_eq!(shape("(1\n+\n2)\n3"), "[(1 + 2)]; 3");
    assert_eq!(shape("let x = (\n  1\n)"), "let x = [1]");
}

#[test]
fn blocks() {
    assert_eq!(shape("{}"), "{}");
    assert_eq!(shape("{ 1 }"), "{1}");
    assert_eq!(
        shape("let x = {\n  let y = 1\n  y + 1\n}"),
        "let x = {let y = 1; (y + 1)}"
    );
    assert_eq!(shape("{ {} ; { = 1 } }"), "{{}; {= 1}}");
    assert_eq!(shape("{{{ 1 }}}"), "{{{1}}}");
    assert_eq!(shape("1 + { 2 } * 3"), "(1 + ({2} * 3))");
    let ast = parse_text("x + {\n1 }").unwrap();
    let root = ast.items(ast.root())[0];
    let ItemKind::Expr(sum) = ast.item(root).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Binary { rhs, .. } = ast.get(sum).kind else {
        panic!("expected a binary node");
    };
    // A block's span covers its braces.
    assert_eq!(ast.get(rhs).span, Span::new(4, 9));
}

#[test]
fn result_items_parse_anywhere_in_a_body() {
    // Whether `= e` is the last item is checked by the checker, which
    // can point at the item after it.
    assert_eq!(shape("= 1; 2"), "= 1; 2");
    assert_eq!(shape("= 1; = 2"), "= 1; = 2");
}

#[test]
fn eq_is_only_accepted_at_the_start_of_an_item() {
    // Or after a field's name.
    assert_eq!(
        error("(x) = 1"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(4, 5)
        )
    );
    assert_eq!(
        error("x + 1 = 1"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("1 + = 2"),
        ("expected expression, found `=`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("(= 1)"),
        ("expected expression, found `=`".to_owned(), Span::new(1, 2))
    );
    assert_eq!(
        error("let x = = 1"),
        ("expected expression, found `=`".to_owned(), Span::new(8, 9))
    );
}

#[test]
fn rebind_needs_a_bare_name() {
    assert_eq!(
        error("(x) := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(4, 6)
        )
    );
    assert_eq!(
        error("x + y := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(6, 8)
        )
    );
    // `: =` is an ascription missing its type, not a rebind.
    assert_eq!(
        error("x : = 1"),
        ("expected a type".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn item_spans() {
    let ast = parse_text("let x: Int = 1 + 2\nx := 3\n= x\nx").unwrap();
    let spans: Vec<Span> = ast
        .items(ast.root())
        .iter()
        .map(|&id| ast.item(id).span)
        .collect();
    assert_eq!(
        spans,
        vec![
            Span::new(0, 18),
            Span::new(19, 25),
            Span::new(26, 29),
            Span::new(30, 31)
        ]
    );
    let ItemKind::Let { path, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(ast.path(path)[0].span, Span::new(4, 5));
}

#[test]
fn ascription_binds_between_multiplication_and_negation() {
    assert_eq!(shape("1 + 2 : Float"), "(1 + (2 : Float))");
    assert_eq!(shape("-5 : Float"), "((-5) : Float)");
    assert_eq!(shape("2 * 3 : Float"), "(2 * (3 : Float))");
    assert_eq!(shape("(1 + 2) : Float"), "([(1 + 2)] : Float)");
    assert_eq!(shape("x : Int : Int"), "((x : Int) : Int)");
    assert_eq!(shape("x : Int + 1"), "((x : Int) + 1)");
}

#[test]
fn ascription_shapes_are_exact() {
    // `1 + 2 : Float` is Add(1, Ascribe(2)).
    let ast = parse_text("1 + 2 : Float").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Binary {
        op: BinaryOp::Add,
        lhs,
        rhs,
        ..
    } = ast.get(root).kind
    else {
        panic!("expected an addition, got {:?}", ast.get(root));
    };
    assert_eq!(ast.get(lhs).kind, ExprKind::Int(1));
    let ExprKind::Ascribe { expr, ty } = ast.get(rhs).kind else {
        panic!("expected an ascription, got {:?}", ast.get(rhs));
    };
    assert_eq!(ast.get(expr).kind, ExprKind::Int(2));
    assert_eq!(ast.get(rhs).span, Span::new(4, 13));
    assert_eq!(ast.type_expr(ty).span, Span::new(8, 13));

    // `-5 : Float` is Ascribe(Neg(5)).
    let ast = parse_text("-5 : Float").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Ascribe { expr, .. } = ast.get(root).kind else {
        panic!("expected an ascription, got {:?}", ast.get(root));
    };
    let ExprKind::Unary {
        op: UnaryOp::Neg,
        operand,
    } = ast.get(expr).kind
    else {
        panic!("expected a negation, got {:?}", ast.get(expr));
    };
    assert_eq!(ast.get(operand).kind, ExprKind::Int(5));
}

#[test]
fn spans() {
    let ast = parse_text("-(1 + 2) * 30").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let root = ast.get(root);
    assert_eq!(root.span, Span::new(0, 13));
    let ExprKind::Binary { lhs, rhs, .. } = root.kind else {
        panic!("expected binary, got {root:?}");
    };
    let neg = ast.get(lhs);
    assert_eq!(neg.span, Span::new(0, 8));
    let ExprKind::Unary { operand, .. } = neg.kind else {
        panic!("expected unary, got {neg:?}");
    };
    let paren = ast.get(operand);
    assert_eq!(paren.span, Span::new(1, 8));
    let ExprKind::Paren(sum) = paren.kind else {
        panic!("expected paren, got {paren:?}");
    };
    assert_eq!(ast.get(sum).span, Span::new(2, 7));
    assert_eq!(ast.get(rhs).span, Span::new(11, 13));
}

#[test]
fn expected_expression() {
    assert_eq!(
        error("1 +"),
        (
            "expected expression, found end of input".to_owned(),
            Span::new(3, 3)
        )
    );
    assert_eq!(
        error("* 2"),
        ("expected expression, found `*`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("()"),
        ("expected expression, found `)`".to_owned(), Span::new(1, 2))
    );
    assert_eq!(
        error("1 + // 2"),
        (
            "expected expression, found `//`".to_owned(),
            Span::new(4, 6)
        )
    );
    assert_eq!(
        error("1 + let"),
        (
            "expected expression, found `let`".to_owned(),
            Span::new(4, 7)
        )
    );
    assert_eq!(
        error("{ 1 + }"),
        ("expected expression, found `}`".to_owned(), Span::new(6, 7))
    );
    assert_eq!(
        error("x := ;"),
        ("expected expression, found `;`".to_owned(), Span::new(5, 6))
    );
}

#[test]
fn expected_close_paren() {
    assert_eq!(
        error("(1 + 2"),
        (
            "expected `)`, found end of input".to_owned(),
            Span::new(6, 6)
        )
    );
    let diagnostic = parse_text("(1 2)").unwrap_err();
    assert_eq!(diagnostic.message, "expected `)`, found `2`");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Span::new(0, 1));
    assert_eq!(diagnostic.labels[0].text, "unclosed `(`");
    assert_eq!(message("(1; 2)"), "expected `)`, found `;`");
}

#[test]
fn unterminated_block() {
    let diagnostic = parse_text("let x = {\n  1\n").unwrap_err();
    assert_eq!(diagnostic.message, "expected `}`, found end of input");
    assert_eq!(diagnostic.span, Span::new(13, 13));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Span::new(8, 9));
    assert_eq!(diagnostic.labels[0].text, "unclosed `{`");
    // The innermost unclosed brace is the one reported.
    let diagnostic = parse_text("{ { }").unwrap_err();
    assert_eq!(diagnostic.labels[0].span, Span::new(0, 1));
    let diagnostic = parse_text("{ {").unwrap_err();
    assert_eq!(diagnostic.labels[0].span, Span::new(2, 3));
}

#[test]
fn stray_close_brace_at_top_level() {
    assert_eq!(error("}"), ("unexpected `}`".to_owned(), Span::new(0, 1)));
    assert_eq!(error("1 }"), ("unexpected `}`".to_owned(), Span::new(2, 3)));
    assert_eq!(
        error("{ 1 }\n}"),
        ("unexpected `}`".to_owned(), Span::new(6, 7))
    );
}

#[test]
fn let_errors() {
    assert_eq!(
        error("let x 5"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("let x: Int 5"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(11, 12)
        )
    );
    assert_eq!(
        error("let x"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(5, 5)
        )
    );
    assert_eq!(
        error("let 5 = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("let let = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 7))
    );
    assert_eq!(
        error("let = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("let"),
        ("expected a name after `let`".to_owned(), Span::new(3, 3))
    );
}

#[test]
fn expected_a_type() {
    assert_eq!(
        error("let x: = 1"),
        ("expected a type".to_owned(), Span::new(7, 8))
    );
    assert_eq!(
        error("1 : 2"),
        ("expected a type".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("1 :"),
        ("expected a type".to_owned(), Span::new(3, 3))
    );
    // A type may be parenthesised, and what is inside must be a type.
    assert_eq!(shape("1 : (Int)"), "(1 : Int)");
    assert_eq!(
        error("1 : (2)"),
        ("expected a type".to_owned(), Span::new(5, 6))
    );
}

#[test]
fn binary_nodes_record_their_operator() {
    let ast = parse_text("1 + 2 // 3").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let root = ast.get(root);
    assert_eq!(root.operator_span(), Some(Span::new(2, 3)));
    let ExprKind::Binary { rhs, .. } = root.kind else {
        panic!("expected binary, got {root:?}");
    };
    assert_eq!(ast.get(rhs).operator_span(), Some(Span::new(6, 8)));
}

#[test]
fn expected_end_of_item() {
    assert_eq!(
        error("1 2"),
        (
            "expected end of item, found `2`".to_owned(),
            Span::new(2, 3)
        )
    );
    assert_eq!(
        error("1 1_000.5e3"),
        (
            "expected end of item, found `1_000.5e3`".to_owned(),
            Span::new(2, 11)
        )
    );
    assert_eq!(
        error("(1))"),
        (
            "expected end of item, found `)`".to_owned(),
            Span::new(3, 4)
        )
    );
    // (`2 (3)` is a call of `2`, which the checker rejects.)
    assert_eq!(
        error("f(3) x"),
        (
            "expected end of item, found `x`".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("{ x 1 }"),
        (
            "expected end of item, found `1`".to_owned(),
            Span::new(4, 5)
        )
    );
    assert_eq!(
        error("let x = 1 let y = 2"),
        (
            "expected end of item, found `let`".to_owned(),
            Span::new(10, 13)
        )
    );
}

#[test]
fn pretty_output_for_a_small_program() {
    let ast = parse_text("let x: Float = 1\ny := { x : Float }\n= -x").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Float @0..16\n\
             \x20   (int 1 @15..16))\n\
             \x20 (rebind y @17..35\n\
             \x20   (block @22..35\n\
             \x20     (expr @24..33\n\
             \x20       (: Float @24..33\n\
             \x20         (name x @24..25)))))\n\
             \x20 (result @36..40\n\
             \x20   (neg @38..40\n\
             \x20     (name x @39..40))))"
    );
}

/// Runs `f` on a thread with [`crate::STACK_SIZE`] of stack, as the
/// pipeline is meant to be run. Test threads are far smaller by default.
pub(super) fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

pub(super) fn too_deep(text: String) -> Diagnostic {
    on_big_stack(move || parse_text(&text).unwrap_err())
}

#[test]
fn nesting_by_recursion_is_limited() {
    // MAX_DEPTH parentheses around a literal is MAX_DEPTH + 1 levels.
    let just_over = format!("{}1{}", "(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
    let diagnostic = too_deep(just_over);
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(
        diagnostic.notes,
        vec![format!("the limit is {MAX_DEPTH} levels")]
    );
    let deepest = format!(
        "{}1{}",
        "(".repeat(MAX_DEPTH - 1),
        ")".repeat(MAX_DEPTH - 1)
    );
    on_big_stack(move || parse_text(&deepest).unwrap());
    // Negations as `-(-(...))`: each pair is two levels.
    let pairs = MAX_DEPTH / 2;
    let negations = format!("{}1{}", "-(".repeat(pairs), ")".repeat(pairs));
    too_deep(negations);
}

#[test]
fn nesting_blocks_is_limited() {
    // Each block is two levels of recursion: the block's body and the
    // expression inside it. With the outermost expression, N blocks are
    // 2N + 1 levels.
    let deepest_blocks = (MAX_DEPTH - 1) / 2;
    let deepest = format!(
        "{}1{}",
        "{".repeat(deepest_blocks),
        "}".repeat(deepest_blocks)
    );
    on_big_stack(move || parse_text(&deepest).unwrap());
    let just_over = format!(
        "{}1{}",
        "{".repeat(deepest_blocks + 1),
        "}".repeat(deepest_blocks + 1)
    );
    let diagnostic = too_deep(just_over);
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Items inside the blocks do not change the count.
    let with_items = format!(
        "{}1{}",
        "{ let x = 1\n".repeat(deepest_blocks + 1),
        "}".repeat(deepest_blocks + 1)
    );
    too_deep(with_items);
}

#[test]
fn blocks_count_towards_height() {
    // A chain one short of the limit, inside one block, is at the limit;
    // inside two, it is over.
    let chain = format!("1{}", " + 1".repeat(MAX_DEPTH - 2));
    let one = format!("{{ {chain} }}");
    on_big_stack(move || parse_text(&one).unwrap());
    let two = format!("{{ {{ {chain} }} }}");
    let diagnostic = too_deep(two);
    assert_eq!(diagnostic.message, "expression nested too deeply");
}

#[test]
fn nesting_by_left_chaining_is_limited() {
    // Each `+` is one level above its left operand, so N terms are N
    // levels deep. This nests without recursion in the parser.
    let just_over = format!("1{}", " + 1".repeat(MAX_DEPTH));
    let diagnostic = too_deep(just_over);
    assert_eq!(diagnostic.message, "expression nested too deeply");
    let deepest = format!("1{}", " + 1".repeat(MAX_DEPTH - 1));
    on_big_stack(move || parse_text(&deepest).unwrap());
    // Ascriptions chain the same way.
    let ascriptions = format!("1{}", " : Int".repeat(MAX_DEPTH));
    too_deep(ascriptions);
}
