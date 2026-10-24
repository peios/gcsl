//! Tests of the parser: lists, indexes, spreads, ranges and `for`.

use super::*;

#[test]
fn list_literals() {
    assert_eq!(shape("[]"), "list()");
    assert_eq!(shape("[1]"), "list(1)");
    assert_eq!(shape("[1, 2,]"), "list(1, 2)");
    assert_eq!(
        shape("[[1], [], [[2]]]"),
        "list(list(1), list(), list(list(2)))"
    );
    // A spread takes a whole element, parsed as any element is.
    assert_eq!(
        shape("[...xs, 1 + 2, ...a + b]"),
        "list(...xs, (1 + 2), ...(a + b))"
    );
    assert_eq!(shape("[...[...xs]]"), "list(...list(...xs))");
    // Across lines, with blocks and records inside.
    assert_eq!(shape("[\n  1,\n  2\n]"), "list(1, 2)");
    assert_eq!(
        shape("[\n  { a = 1\n    b = 2 },\n  {}\n]"),
        "list({a = 1; b = 2}, {})"
    );
    // An `if` condition may hold a list.
    assert_eq!(shape("if xs == [] { 1 }"), "(if (xs == list()) {1})");
    let ast = parse_text("[1, ...xs]").unwrap();
    let list = ast.get(only_value(&ast));
    assert_eq!(list.span, Span::new(0, 10));
    let ExprKind::List(elements) = list.kind else {
        panic!("expected a list, got {list:?}");
    };
    let [first, second] = ast.elements(elements) else {
        panic!("expected two elements");
    };
    assert_eq!(first.spread, None);
    assert_eq!(first.span(&ast), Span::new(1, 2));
    assert_eq!(second.spread, Some(Span::new(4, 7)));
    assert_eq!(second.span(&ast), Span::new(4, 9));
    assert_eq!(ast.get(second.value).span, Span::new(7, 9));
}

#[test]
fn indexes_chain_with_calls_and_field_access() {
    assert_eq!(shape("xs[0]"), "(xs at 0)");
    assert_eq!(shape("xs[0][1]"), "((xs at 0) at 1)");
    assert_eq!(shape("f(x)[0].name[1]"), "(((f(x) at 0) . name) at 1)");
    assert_eq!(shape("xs[i + 1]"), "(xs at (i + 1))");
    assert_eq!(shape("[1, 2][0]"), "(list(1, 2) at 0)");
    assert_eq!(shape("-xs[0] * 2"), "((-(xs at 0)) * 2)");
    assert_eq!(shape("xs[0](1)"), "(xs at 0)(1)");
    assert_eq!(shape("a : Int[0]"), "((a : Int) at 0)");
    // A position may span lines, as inside any brackets.
    assert_eq!(shape("xs[\n  0\n]"), "(xs at 0)");
    let ast = parse_text("xs [ 1 ]").unwrap();
    let index = ast.get(only_value(&ast));
    assert_eq!(index.span, Span::new(0, 8));
    assert_eq!(index.bracket_span(&ast), Some(Span::new(2, 8)));
    let ExprKind::Index { expr, index } = index.kind else {
        panic!("expected an index, got {index:?}");
    };
    assert_eq!(ast.get(expr).span, Span::new(0, 2));
    assert_eq!(ast.get(index).span, Span::new(5, 6));
}

#[test]
fn a_bracket_after_a_newline_starts_a_list() {
    // The item has ended, so the `[` starts the next one.
    assert_eq!(shape("xs\n[0]"), "xs; list(0)");
    assert_eq!(shape("let x = f(1)\n[1, 2]"), "let x = f(1); list(1, 2)");
    // Inside parentheses and brackets there are no newline tokens, so
    // it is still an index.
    assert_eq!(shape("(xs\n[0])"), "[(xs at 0)]");
    assert_eq!(shape("[xs\n[0]]"), "list((xs at 0))");
}

#[test]
fn list_types() {
    assert_eq!(shape("let x: List<Int> = y"), "let x: List<Int> = y");
    assert_eq!(shape("let x: Int[] = y"), "let x: (Int)[] = y");
    assert_eq!(shape("let x: Int[][] = y"), "let x: ((Int)[])[] = y");
    assert_eq!(shape("let x: {a: Int}[] = y"), "let x: ({a: Int})[] = y");
    assert_eq!(
        shape("let x: Future<Int>[] = y"),
        "let x: (Future<Int>)[] = y"
    );
    assert_eq!(shape("let x: List<Int>[] = y"), "let x: (List<Int>)[] = y");
    assert_eq!(
        shape("let x: (fn(): Int)[] = y"),
        "let x: (fn(): Int)[] = y"
    );
    assert_eq!(
        shape("let x: (Int | Future<Int>)[] = y"),
        "let x: ((Int | Future<Int>))[] = y"
    );
    // The postfix binds tighter than `|`, and belongs to a function
    // type's return type.
    assert_eq!(
        shape("let x: Int | Future<Int>[] = y"),
        "let x: (Int | (Future<Int>)[]) = y"
    );
    assert_eq!(shape("let x: fn(): Int[] = y"), "let x: fn(): (Int)[] = y");
    assert_eq!(
        shape("fn(xs: Int[]): Int[] { xs }"),
        "fn(xs: (Int)[]): (Int)[] {xs}"
    );
    let ast = parse_text("let x: Int[][] = y").unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated let");
    };
    let outer = ast.type_expr(ty);
    assert_eq!(outer.span, Span::new(7, 14));
    let TypeExprKind::List(inner) = outer.kind else {
        panic!("expected a list type, got {outer:?}");
    };
    assert_eq!(ast.type_expr(inner).span, Span::new(7, 12));
    // A `[` not directly followed by `]` is not part of the type.
    assert_eq!(
        error("let x: Int[3] = y"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(10, 11)
        )
    );
}

#[test]
fn pretty_output_for_lists() {
    let ast = parse_text(
        "let x: Int[] = [1, ...ys]\nx[0]\nlet f: (fn(): Int)[] = g\nlet u: (Int | \
             Future<Int>)[][] = v\nlet r: fn(): List<Int>[] = h",
    )
    .unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Int[] @0..25\n\
             \x20   (list @15..25\n\
             \x20     (int 1 @16..17)\n\
             \x20     (spread\n\
             \x20       (name ys @22..24))))\n\
             \x20 (expr @26..30\n\
             \x20   (index @26..30\n\
             \x20     (name x @26..27)\n\
             \x20     (int 0 @28..29)))\n\
             \x20 (let f : (fn(): Int)[] @31..55\n\
             \x20   (name g @54..55))\n\
             \x20 (let u : (Int | Future<Int>)[][] @56..90\n\
             \x20   (name v @89..90))\n\
             \x20 (let r : fn(): List<Int>[] @91..119\n\
             \x20   (name h @118..119)))"
    );
}

#[test]
fn list_errors() {
    // A spread anywhere an expression starts, but in a list, as an
    // argument or at the start of an item.
    for (text, at) in [("let x = ...xs", 8), ("[(...xs)]", 2), ("f((...xs))", 3)] {
        assert_eq!(
            error(text),
            (MISPLACED_SPREAD.to_owned(), Span::new(at, at + 3)),
            "{text}"
        );
    }
    let diagnostic = parse_text("[1 2]").unwrap_err();
    assert_eq!(diagnostic.message, "expected `,` or `]`, found `2`");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(labels(&diagnostic), vec![(Span::new(0, 1), "unclosed `[`")]);
    assert_eq!(
        error("[1, 2"),
        (
            "expected `,` or `]`, found end of input".to_owned(),
            Span::new(5, 5)
        )
    );
    assert_eq!(
        error("[,]"),
        ("expected expression, found `,`".to_owned(), Span::new(1, 2))
    );
    assert_eq!(
        error("]"),
        ("expected expression, found `]`".to_owned(), Span::new(0, 1))
    );
    let diagnostic = parse_text("xs[0").unwrap_err();
    assert_eq!(diagnostic.message, "expected `]`, found end of input");
    assert_eq!(labels(&diagnostic), vec![(Span::new(2, 3), "unclosed `[`")]);
    assert_eq!(
        error("xs[]"),
        ("expected expression, found `]`".to_owned(), Span::new(3, 4))
    );
    assert_eq!(
        error("xs[0, 1]"),
        ("expected `]`, found `,`".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn spread_items() {
    // A spread item in a block, first, last and alone.
    assert_eq!(shape("{ ...a; b = 1 }"), "{...a; b = 1}");
    assert_eq!(shape("{ b = 1; ...a }"), "{b = 1; ...a}");
    assert_eq!(shape("{ ...a }"), "{...a}");
    assert_eq!(shape("{\n  ...a\n  ...{ c = 2 }\n}"), "{...a; ...{c = 2}}");
    // At the top level, and in a function body.
    assert_eq!(shape("...a\nb = 1"), "...a; b = 1");
    assert_eq!(shape("...a"), "...a");
    assert_eq!(shape("fn(): {} { ...a }"), "fn(): {} {...a}");
    // Its expression is parsed as a field's value is, at minimum 0, so
    // an ascription belongs to it.
    assert_eq!(shape("{ ...a : {b: Int} }"), "{...(a : {b: Int})}");
    assert_eq!(shape("{ ...a.b + c }"), "{...((a . b) + c)}");
    assert_eq!(shape("{ ...f(x) }"), "{...f(x)}");
    // A newline after `...` does not end the item.
    assert_eq!(shape("{ ...\n  a }"), "{...a}");
    // A block with a result parses; that is the checker's error.
    assert_eq!(shape("{ ...a; = 1 }"), "{...a; = 1}");
    // What is printed parses back to itself.
    for text in [
        "{...a; b = 1}",
        "...a; ...{c = 2}",
        "fn(): {} {x = 1; ...a}",
    ] {
        assert_eq!(shape(text), text);
    }
    // The item spans `...` through its expression.
    let ast = parse_text("{ ...ab : T }").unwrap();
    let ExprKind::Block { items, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected a block");
    };
    let item = ast.item(ast.items(items)[0]);
    assert_eq!(item.span, Span::new(2, 11));
    let ItemKind::Spread(value) = item.kind else {
        panic!("expected a spread, got {item:?}");
    };
    assert_eq!(ast.get(value).span, Span::new(5, 11));
    // A `...` inside an item's expression is still misplaced.
    for (text, at) in [
        ("x = ...xs", 4),
        ("{ (...xs) }", 3),
        ("{ ...xs + ...ys }", 10),
    ] {
        assert_eq!(
            error(text),
            (MISPLACED_SPREAD.to_owned(), Span::new(at, at + 3)),
            "{text}"
        );
    }
    assert_eq!(
        error("{ ... }"),
        ("expected expression, found `}`".to_owned(), Span::new(6, 7))
    );
    // A spread item nests as a field item does: each block is two
    // levels of recursion, as `nesting_blocks_is_limited` counts them.
    let spreads = |depth: usize| format!("{}1{}", "{...".repeat(depth), "}".repeat(depth));
    let deepest = spreads((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(spreads((MAX_DEPTH - 1) / 2 + 1)).message,
        "expression nested too deeply"
    );
}

#[test]
fn pretty_output_for_spread_items() {
    let ast = parse_text("...a\n{ ...b }").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (spread @0..4\n\
             \x20   (name a @3..4))\n\
             \x20 (expr @5..13\n\
             \x20   (block @5..13\n\
             \x20     (spread @7..11\n\
             \x20       (name b @10..11)))))"
    );
}

#[test]
fn lists_and_indexes_count_towards_depth() {
    // N nested literals around a literal are N + 1 levels.
    let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let deepest = nested(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(nested(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(nested(200_000)).message,
        "expression nested too deeply"
    );
    // A spread is as tall as its expression.
    let spreads = |depth: usize| format!("{}1{}", "[...".repeat(depth), "]".repeat(depth));
    let deepest = spreads(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(spreads(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    // A chain of N indexes on a name is N + 1 levels, built without
    // recursion; so is N nested in each other's positions.
    let chain = |depth: usize| format!("x{}", "[0]".repeat(depth));
    let deepest = chain(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(chain(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(chain(200_000)).message,
        "expression nested too deeply"
    );
    let positions = |depth: usize| format!("{}0{}", "x[".repeat(depth), "]".repeat(depth));
    let deepest = positions(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(positions(200_000)).message,
        "expression nested too deeply"
    );
    // N `[]`s on a name are N + 1 levels of type.
    let ty = |depth: usize| format!("let x: Int{} = y", "[]".repeat(depth));
    let deepest = ty(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(ty(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(ty(200_000)).message,
        "expression nested too deeply"
    );
}

#[test]
fn ranges_bind_between_comparison_and_addition() {
    assert_eq!(shape("0..10"), "(0 .. 10)");
    assert_eq!(shape("0..=10"), "(0 ..= 10)");
    assert_eq!(shape("a..b"), "(a .. b)");
    assert_eq!(shape("0 .. 1"), "(0 .. 1)");
    // Looser than `+` and `-`, on either side.
    assert_eq!(shape("0..len(xs) - 1"), "(0 .. (len(xs) - 1))");
    assert_eq!(shape("a + 1..=b * 2"), "((a + 1) ..= (b * 2))");
    // Tighter than the comparisons, on either side.
    assert_eq!(shape("a..b == c"), "((a .. b) == c)");
    assert_eq!(shape("c == a..=b"), "(c == (a ..= b))");
    assert_eq!(shape("a..b < c..d"), "((a .. b) < (c .. d))");
    assert_eq!(shape("x and a..b != c"), "(x and ((a .. b) != c))");
    assert_eq!(shape("not a..b"), "(not (a .. b))");
    // Negation binds tighter still.
    assert_eq!(shape("-1..1"), "((-1) .. 1)");
    assert_eq!(shape("0..-1"), "(0 .. (-1))");
    assert_eq!(shape("-a..=-b"), "((-a) ..= (-b))");
    // Postfix operators apply to the bound, and an ascription too.
    assert_eq!(shape("a.b..c[0]"), "((a . b) .. (c at 0))");
    assert_eq!(shape("0..n : Int"), "(0 .. (n : Int))");
    assert_eq!(shape("(0..n)[1]"), "([(0 .. n)] at 1)");
    // Newlines after the operator are skipped, as after any binary
    // operator, and not before it.
    assert_eq!(shape("0..\n\n  10"), "(0 .. 10)");
    assert_eq!(shape("0..=\n10"), "(0 ..= 10)");
    assert_eq!(
        error("0\n..10"),
        (
            "expected expression, found `..`".to_owned(),
            Span::new(2, 4)
        )
    );
    // A range is an operand like any other: in a list, as an argument,
    // spread.
    assert_eq!(shape("[0..2, ...1..=3]"), "list((0 .. 2), ...(1 ..= 3))");
    assert_eq!(shape("f(0..n, m = 1..2)"), "f((0 .. n), m = (1 .. 2))");
    // Neither dot of a range is part of a number.
    assert_eq!(shape("1.5..2"), "(1.5 .. 2)");
}

#[test]
fn ranges_cannot_be_chained() {
    let diagnostic = parse_text("a..b..c").unwrap_err();
    assert_eq!(diagnostic.message, "`..` cannot be chained");
    assert_eq!(diagnostic.span, Span::new(4, 6));
    assert!(diagnostic.notes.is_empty());
    assert_eq!(
        error("a..=b..=c"),
        ("`..=` cannot be chained".to_owned(), Span::new(5, 8))
    );
    // With each other, either way round.
    assert_eq!(
        error("a..b..=c"),
        ("`..=` cannot be chained".to_owned(), Span::new(4, 7))
    );
    assert_eq!(
        error("a..=b..c"),
        ("`..` cannot be chained".to_owned(), Span::new(5, 7))
    );
    // After a right operand that is a sum, and on the right of a
    // comparison.
    assert_eq!(
        error("0..n + 1..m"),
        ("`..` cannot be chained".to_owned(), Span::new(8, 10))
    );
    assert_eq!(
        error("x == 0..1..2"),
        ("`..` cannot be chained".to_owned(), Span::new(9, 11))
    );
    // Parentheses separate them.
    assert_eq!(shape("(a..b)..c"), "([(a .. b)] .. c)");
    assert_eq!(shape("a..(b..c)"), "(a .. [(b .. c)])");
}

#[test]
fn for_shapes() {
    assert_eq!(shape("for x in xs { x }"), "for x in xs {x}");
    assert_eq!(shape("for x in xs {}"), "for x in xs {}");
    // The guard.
    assert_eq!(
        shape("for x in xs if x > 1 { x }"),
        "for x in xs if (x > 1) {x}"
    );
    assert_eq!(
        shape("for x in xs if a and not b { x }"),
        "for x in xs if (a and (not b)) {x}"
    );
    // An iterable that is a call, an index, a field, a range, an `if`,
    // a block, a list.
    assert_eq!(shape("for x in f(y) { x }"), "for x in f(y) {x}");
    assert_eq!(shape("for x in xs[0] { x }"), "for x in (xs at 0) {x}");
    assert_eq!(shape("for x in r.xs { x }"), "for x in (r . xs) {x}");
    assert_eq!(shape("for i in 0..n { i }"), "for i in (0 .. n) {i}");
    assert_eq!(
        shape("for i in (0..=n) if i != 2 { i }"),
        "for i in [(0 ..= n)] if (i != 2) {i}"
    );
    assert_eq!(
        shape("for x in if a { xs } else { ys } { x }"),
        "for x in (if a {xs} else {ys}) {x}"
    );
    assert_eq!(
        shape("for x in if a { xs } else { ys } if x { x }"),
        "for x in (if a {xs} else {ys}) if x {x}"
    );
    assert_eq!(shape("for x in { xs } { x }"), "for x in {xs} {x}");
    assert_eq!(shape("for x in [1, 2] { x }"), "for x in list(1, 2) {x}");
    // A `{` after the iterable or the guard is the body, never a
    // trailing block; in parentheses it is one.
    assert_eq!(shape("for x in f { x }"), "for x in f {x}");
    assert_eq!(shape("for x in xs if g { x }"), "for x in xs if g {x}");
    assert_eq!(shape("for x in (f { 1 }) { x }"), "for x in [f(){1}] {x}");
    assert_eq!(shape("for x in f(g { 1 }) { x }"), "for x in f(g(){1}) {x}");
    // Nested, and in the body.
    assert_eq!(
        shape("for x in xs { for y in x { y } }"),
        "for x in xs {for y in x {y}}"
    );
    assert_eq!(
        shape("for x in for y in ys { y } { x }"),
        "for x in for y in ys {y} {x}"
    );
    assert_eq!(
        shape("for x in xs {\n  let y = x * 2\n  y\n}"),
        "for x in xs {let y = (x * 2); y}"
    );
    // A `for` is an operand like any other prefix expression: in a
    // list, as an argument, spread, in a `let`, followed by an index.
    assert_eq!(
        shape("[for x in xs { x }, ...for y in ys { y }]"),
        "list(for x in xs {x}, ...for y in ys {y})"
    );
    assert_eq!(
        shape("f(for x in xs { x }, n = for y in ys { y })"),
        "f(for x in xs {x}, n = for y in ys {y})"
    );
    assert_eq!(
        shape("let ys = for x in xs { x }"),
        "let ys = for x in xs {x}"
    );
    assert_eq!(shape("for x in xs { x }[0]"), "(for x in xs {x} at 0)");
    assert_eq!(shape("1 + for x in xs { x }"), "(1 + for x in xs {x})");
    assert_eq!(shape("{ ...for x in xs { x } }"), "{...for x in xs {x}}");
    // What is printed parses back to itself.
    for text in [
        "for x in xs {x}",
        "for x in xs if g {x}",
        "let ys = for x in xs {for y in x {y}}",
        "f(for x in xs {x}, n = for y in ys {y})",
    ] {
        assert_eq!(shape(text), text);
    }
}

#[test]
fn a_for_spans_its_keyword_through_its_body() {
    let ast = parse_text("for x in xs if g { x }").unwrap();
    let root = only_value(&ast);
    let expr = ast.get(root);
    assert_eq!(expr.span, Span::new(0, 22));
    assert_eq!(expr.keyword_span(), Some(Span::new(0, 3)));
    let ExprKind::For(id) = expr.kind else {
        panic!("expected a for, got {expr:?}");
    };
    let decl = ast.for_decl(id);
    let ForHead::Each {
        name,
        name_span,
        iterable,
        guard,
    } = decl.head
    else {
        panic!("expected an iteration, got {decl:?}");
    };
    assert_eq!(ast.names().get(name), "x");
    assert_eq!(name_span, Span::new(4, 5));
    assert_eq!(ast.get(iterable).span, Span::new(9, 11));
    assert_eq!(
        guard.map(|guard| ast.get(guard).span),
        Some(Span::new(15, 16))
    );
    assert_eq!(ast.get(decl.body).span, Span::new(17, 22));
    assert_eq!(ast.for_count(), 1);
    // Without a guard.
    let ast = parse_text("for x in xs {}").unwrap();
    let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
        panic!("expected a for");
    };
    let ForHead::Each { guard, .. } = ast.for_decl(id).head else {
        panic!("expected an iteration");
    };
    assert_eq!(guard, None);
}

#[test]
fn for_errors() {
    // A body on the next line is the error an `if`'s is.
    assert_eq!(
        error("for x in xs\n{ x }"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 12)
        )
    );
    assert_eq!(
        error("for x in xs if g\n{ x }"),
        (
            "expected `{` after `for` guard".to_owned(),
            Span::new(16, 17)
        )
    );
    assert_eq!(
        error("for x in xs x"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(13, 13)
        )
    );
    assert_eq!(
        error("for x in xs"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 11)
        )
    );
    // The variable is a plain name.
    assert_eq!(
        error("for 1 in xs {}"),
        ("expected a name after `for`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("for \"x\" in xs {}"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(4, 7))
    );
    assert_eq!(
        error("for a.b in xs {}"),
        (
            "expected `in` after the `for` variable, found `.`".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("for x 1 {}"),
        (
            "expected `in` after the `for` variable, found `1`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("for x in {}"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 11)
        )
    );
    assert_eq!(
        error("for x in xs if {}"),
        (
            "expected `{` after `for` guard".to_owned(),
            Span::new(17, 17)
        )
    );
    assert_eq!(
        error("in"),
        (
            "expected expression, found `in`".to_owned(),
            Span::new(0, 2)
        )
    );
    assert_eq!(
        error("for x in xs { x } 2"),
        (
            "expected end of item, found `2`".to_owned(),
            Span::new(18, 19)
        )
    );
    // `for` and `in` are keywords, not names.
    assert_eq!(
        error("let in = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 6))
    );
}

#[test]
fn pretty_output_for_ranges_and_fors() {
    let ast = parse_text("for i in 0..=n if i != 1 { i }\nfor x in xs {}").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..30\n\
             \x20   (for i @0..30\n\
             \x20     (..= @9..14\n\
             \x20       (int 0 @9..10)\n\
             \x20       (name n @13..14))\n\
             \x20     (guard\n\
             \x20       (!= @18..24\n\
             \x20         (name i @18..19)\n\
             \x20         (int 1 @23..24)))\n\
             \x20     (block @25..30\n\
             \x20       (expr @27..28\n\
             \x20         (name i @27..28)))))\n\
             \x20 (expr @31..45\n\
             \x20   (for x @31..45\n\
             \x20     (name xs @40..42)\n\
             \x20     (block @43..45))))"
    );
}

#[test]
fn fors_count_towards_depth() {
    // A `for` is one above the tallest of its parts, as an `if` is:
    // nested in each other's bodies, N `for`s around a literal are 2N +
    // 1 levels, and two levels of parser recursion each.
    let nested =
        |depth: usize| format!("{}1{}", "for x in xs { ".repeat(depth), " }".repeat(depth));
    let deepest = nested((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(nested((MAX_DEPTH - 1) / 2 + 1)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(nested(200_000)).message,
        "expression nested too deeply"
    );
    // Nested in each other's iterables, N `for`s are N + 2 levels, the
    // innermost being one above its body, a block around a name; and in
    // each other's guards.
    let iterables =
        |depth: usize| format!("{}xs{}", "for x in ".repeat(depth), " { x }".repeat(depth));
    let deepest = iterables(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(iterables(MAX_DEPTH - 1)).message,
        "expression nested too deeply"
    );
    let guards = |depth: usize| {
        format!(
            "{}g{}",
            "for x in xs if ".repeat(depth),
            " {}".repeat(depth)
        )
    };
    assert_eq!(
        too_deep(guards(200_000)).message,
        "expression nested too deeply"
    );
    // A chain of ranges is an error, but ranges nest in parentheses.
    let ranges = |depth: usize| format!("{}0{}", "(0..".repeat(depth), ")".repeat(depth));
    assert_eq!(
        too_deep(ranges(200_000)).message,
        "expression nested too deeply"
    );
}
