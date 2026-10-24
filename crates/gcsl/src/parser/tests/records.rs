//! Tests of the parser: fields and their names, paths, field access and record
//! types.

use super::*;

#[test]
fn field_items_with_plain_and_quoted_names() {
    assert_eq!(shape("x = 1"), "x = 1");
    assert_eq!(shape("{ x = 1; y = x + 1 }"), "{x = 1; y = (x + 1)}");
    assert_eq!(
        shape("\"a b\" = 1\n\"\"\"c\"\"\" = 2"),
        "\"a b\" = 1; c = 2"
    );
    assert_eq!(shape("x =\n  1"), "x = 1");
    assert_eq!(shape("hello-world = { n = 1 }"), "hello-world = {n = 1}");
    // A quoted name is interned like an identifier: `"x"` is `x`.
    let ast = parse_text("\"x\" = 1; x = 2").unwrap();
    let names: Vec<Name> = ast
        .items(ast.root())
        .iter()
        .map(|&id| match ast.item(id).kind {
            ItemKind::Field { path, .. } => ast.path(path)[0].name().unwrap(),
            other => panic!("expected a field, got {other:?}"),
        })
        .collect();
    assert_eq!(names[0], names[1]);
    assert_eq!(ast.names().get(names[0]), "x");
    // Spans: the item from the name, which includes its quotes.
    let ast = parse_text("\"a b\" = 1 + 2").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(0, 13));
    let ItemKind::Field { path, .. } = item.kind else {
        panic!("expected a field");
    };
    assert_eq!(ast.path(path)[0].span, Span::new(0, 5));
    // `==` is a comparison, not a field.
    assert_eq!(shape("x == 1"), "(x == 1)");
    assert_eq!(shape("\"a\" == \"b\""), "(\"a\" == \"b\")");
}

#[test]
fn field_names_must_be_plain() {
    assert_eq!(
        error("r\"a\" = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 4))
    );
    assert_eq!(
        error("\"a{x}\" = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 6))
    );
    assert_eq!(
        error("a.r\"b\""),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(2, 6))
    );
    assert_eq!(
        error("a.\"{x}\""),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(2, 7))
    );
    assert_eq!(
        error("1 : {r\"a\": Int}"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(5, 9))
    );
    assert_eq!(
        error("1 : {\"{x}\": Int}"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(5, 10))
    );
    // An interpolated literal is fine where an expression is.
    assert_eq!(shape("\"a{x}\" == 1"), "(\"a<x>\" == 1)");
}

#[test]
fn variables_need_plain_names() {
    let message = "variables need plain names".to_owned();
    assert_eq!(error("let \"x\" = 1"), (message.clone(), Span::new(4, 7)));
    assert_eq!(error("let r\"x\" = 1"), (message.clone(), Span::new(4, 8)));
    assert_eq!(error("let \"{y}\" = 1"), (message.clone(), Span::new(4, 9)));
    // A quoted name is never a variable, so the base of a `:=` path
    // cannot be one either, whatever follows it.
    assert_eq!(error("\"x\" := 1"), (message.clone(), Span::new(0, 3)));
    assert_eq!(error("r\"x\" := 1"), (message.clone(), Span::new(0, 4)));
    assert_eq!(error("\"{y}\" := 1"), (message.clone(), Span::new(0, 5)));
    assert_eq!(error("\"x\".y := 1"), (message.clone(), Span::new(0, 3)));
    assert_eq!(error("let \"x\".y = 1"), (message, Span::new(4, 7)));
}

#[test]
fn paths_in_fields_lets_and_rebinds() {
    assert_eq!(shape("a.b = 1"), "a.b = 1");
    assert_eq!(shape("a.b.c = 1"), "a.b.c = 1");
    assert_eq!(shape("a.b := 1"), "a.b := 1");
    assert_eq!(shape("let a.b.c = 1"), "let a.b.c = 1");
    assert_eq!(shape("let a.b: Int = 1"), "let a.b: Int = 1");
    assert_eq!(shape("let a . b = 1"), "let a.b = 1");
    // Quoted segments, anywhere but the start of a variable's path.
    assert_eq!(
        shape("\"log level\".format = \"json\""),
        "\"log level\".format = \"json\""
    );
    assert_eq!(shape("a.\"b c\".d = 1"), "a.\"b c\".d = 1");
    assert_eq!(shape("let a.\"b c\" = 1"), "let a.\"b c\" = 1");
    assert_eq!(shape("a.\"b c\" := 1"), "a.\"b c\" := 1");
    // A quoted segment is the same name as a plain one.
    assert_eq!(shape("a.\"b\" = 1"), "a.b = 1");
    // In blocks, and as values.
    assert_eq!(
        shape("{ server.host = \"h\"; server.port = 80 }"),
        "{server.host = \"h\"; server.port = 80}"
    );
    assert_eq!(shape("a.b =\n  1"), "a.b = 1");
    // `==` after a path is a comparison, and a path alone an access.
    assert_eq!(shape("a.b == 1"), "((a . b) == 1)");
    assert_eq!(shape("a.b"), "(a . b)");
    // Spans: each segment, quotes included, and the item from its first.
    let ast = parse_text("  a . \"b c\".d = 1").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 17));
    let ItemKind::Field { path, .. } = item.kind else {
        panic!("expected a field");
    };
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(
        spans,
        vec![Span::new(2, 3), Span::new(6, 11), Span::new(12, 13)]
    );
    let ast = parse_text("\"x\".y := 1").unwrap_err();
    assert_eq!(ast.span, Span::new(0, 3));
    let ast = parse_text("let a.\"b\".c: Int = 1").unwrap();
    let ItemKind::Let { path, ty, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    assert!(ty.is_some());
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(
        spans,
        vec![Span::new(4, 5), Span::new(6, 9), Span::new(10, 11)]
    );
    let ast = parse_text("\"q\".r = 1").unwrap();
    let ItemKind::Field { path, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a field");
    };
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(spans, vec![Span::new(0, 3), Span::new(4, 5)]);
    // The expression a path was read from leaves no nodes behind: only
    // the values remain.
    assert_eq!(parse_text("a.b.c = 1; d := 2; \"e\" = 3").unwrap().len(), 3);
    assert_eq!(parse_text("{ a.b = { c.d = 1 } }").unwrap().len(), 3);
}

#[test]
fn anything_else_before_eq_is_not_a_path() {
    let found_eq = |at: u32| {
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(at, at + 1),
        )
    };
    assert_eq!(error("a.b + 1 = 2"), found_eq(8));
    assert_eq!(error("(a).b = 1"), found_eq(6));
    assert_eq!(error("(a.b) = 1"), found_eq(6));
    assert_eq!(error("-a.b = 1"), found_eq(5));
    assert_eq!(error("(a).b : Int = 1"), found_eq(12));
    assert_eq!(error("{ x = 1 }.x = 1"), found_eq(12));
    assert_eq!(error("1.x = 1"), found_eq(4));
    assert_eq!(
        error("a.b + 1 := 2"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(8, 10)
        )
    );
    assert_eq!(
        error("(a).b := 2"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(6, 8)
        )
    );
    // A raw or interpolated literal is not a field name, at the start of
    // a path as alone.
    assert_eq!(
        error("r\"a\".b = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 4))
    );
    assert_eq!(
        error("\"a{x}\".b = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 6))
    );
    // After `let`, a segment must be a name.
    assert_eq!(
        error("let a.1 = 1"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("let a. = 1"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(7, 8)
        )
    );
    assert_eq!(
        error("let a.r\"b\" = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(6, 10))
    );
    assert_eq!(
        error("let a.b 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(8, 9)
        )
    );
}

#[test]
fn pretty_output_for_paths() {
    let ast = parse_text("let a.b.c = 1\nx.\"y z\" = 2\na.b := 3").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let a.b.c @0..13\n\
             \x20   (int 1 @12..13))\n\
             \x20 (field x.\"y z\" @14..25\n\
             \x20   (int 2 @24..25))\n\
             \x20 (rebind a.b @26..34\n\
             \x20   (int 3 @33..34)))"
    );
}

#[test]
fn paths_count_towards_height() {
    // A path of N segments is 2(N - 1) levels above its value: with a
    // literal, one short of the limit when 2(N - 1) + 1 is.
    let segments = (MAX_DEPTH - 1) / 2 + 1;
    let path = |segments: usize| format!("a{}", ".b".repeat(segments - 1));
    let deepest = format!("let {} = 1", path(segments));
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(format!("let {} = 1", path(segments + 1)));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(diagnostic.span.start, 0);
    // A field's path too, and in blocks, whose height it adds to: the
    // deepest item is at the limit in one block and over it in two.
    let field = format!("{} = 1", path(segments));
    on_big_stack(move || parse_text(&field).unwrap());
    too_deep(format!("{} = 1", path(segments + 1)));
    let one = format!("{{ {} = 1 }}", path(segments));
    on_big_stack(move || parse_text(&one).unwrap());
    too_deep(format!("{{ {{ {} = 1 }} }}", path(segments)));
    // Far beyond the limit, a `let` path is stopped at its item.
    too_deep(format!("let {} = 1", path(200_000)));
}

#[test]
fn field_access_binds_tighter_than_everything() {
    assert_eq!(shape("a.b"), "(a . b)");
    assert_eq!(shape("-a.b"), "(-(a . b))");
    assert_eq!(shape("a.b.c"), "((a . b) . c)");
    assert_eq!(shape("a.b : Int"), "((a . b) : Int)");
    assert_eq!(shape("a.b + c.d"), "((a . b) + (c . d))");
    assert_eq!(shape("a.b == c.d"), "((a . b) == (c . d))");
    assert_eq!(shape("not a.b"), "(not (a . b))");
    assert_eq!(shape("a.\"b c\".d"), "((a . \"b c\") . d)");
    assert_eq!(shape("a . b"), "(a . b)");
    assert_eq!(shape("(a).b"), "([a] . b)");
    assert_eq!(shape("{ x = 1 }.x"), "({x = 1} . x)");
    assert_eq!(shape("1.x"), "(1 . x)");
    assert_eq!(shape("\"s\".x"), "(\"s\" . x)");
    assert_eq!(shape("a.\"x\""), "(a . x)");
    assert_eq!(shape("a.b * 2 - c.d.e"), "(((a . b) * 2) - ((c . d) . e))");
    // Spans: the node covers its record through its name.
    let ast = parse_text("-ab.\"c d\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Unary { operand, .. } = ast.get(root).kind else {
        panic!("expected a negation");
    };
    let access = ast.get(operand);
    assert_eq!(access.span, Span::new(1, 9));
    assert_eq!(access.member_span(&ast), Some(Span::new(3, 9)));
    assert_eq!(ast.get(root).member_span(&ast), None);
}

#[test]
fn field_access_errors() {
    let message = "expected a field name after `.`".to_owned();
    assert_eq!(error("x.1"), (message.clone(), Span::new(2, 3)));
    assert_eq!(error("x.1.5"), (message.clone(), Span::new(2, 5)));
    assert_eq!(error("x."), (message.clone(), Span::new(2, 2)));
    assert_eq!(error("x.let"), (message.clone(), Span::new(2, 5)));
    assert_eq!(error("x.(y)"), (message.clone(), Span::new(2, 3)));
    // Newlines are not skipped around `.`.
    assert_eq!(error("x.\ny"), (message, Span::new(2, 3)));
    assert_eq!(
        error("x\n.y"),
        ("expected expression, found `.`".to_owned(), Span::new(2, 3))
    );
}

#[test]
fn record_types() {
    assert_eq!(
        shape("let p: {x: Int, y: Int} = 1"),
        "let p: {x: Int, y: Int} = 1"
    );
    assert_eq!(shape("1 : {}"), "(1 : {})");
    assert_eq!(
        shape("1 : { \"a b\": String, nested: {n: Int}, }"),
        "(1 : {\"a b\": String, nested: {n: Int}})"
    );
    // Newlines anywhere inside the braces.
    assert_eq!(
        shape("let p: {\n  x:\n    Int\n  ,\n  y: {\n}\n\n} = 1"),
        "let p: {x: Int, y: {}} = 1"
    );
    // Repeated names are the checker's business.
    assert_eq!(shape("1 : {x: Int, x: Int}"), "(1 : {x: Int, x: Int})");
    // A record type ends an ascription as a name does.
    assert_eq!(shape("x : {a: Int} + 1"), "((x : {a: Int}) + 1)");
    let ast = parse_text("1 : {a: Int, \"b\": {}}").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Ascribe { ty, .. } = ast.get(root).kind else {
        panic!("expected an ascription");
    };
    let ty = ast.type_expr(ty);
    assert_eq!(ty.span, Span::new(4, 21));
    let TypeExprKind::Record(fields) = ty.kind else {
        panic!("expected a record type");
    };
    let fields = ast.field_tys(fields);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].span, Span::new(5, 6));
    assert_eq!(fields[1].span, Span::new(13, 16));
    assert_eq!(
        fields[1].name().map(|name| ast.names().get(name)),
        Some("b")
    );
    assert_eq!(ast.type_expr(fields[1].ty).span, Span::new(18, 20));
}

#[test]
fn record_type_fields_may_have_defaults() {
    // Any expression, after any field, wherever a record type is
    // written: an annotation, a parameter's type, a return type, a
    // function type, nested, in a list or a union. What a default may
    // be is the checker's business.
    assert_eq!(
        shape("let p: {a: Int = 1, b: String, c: List<String> = []} = q"),
        "let p: {a: Int = 1, b: String, c: List<String> = list()} = q"
    );
    assert_eq!(
        shape("fn(e: {a: Int = -1}): {r: Bool = true} { e }"),
        "fn(e: {a: Int = (-1)}): {r: Bool = true} {e}"
    );
    assert_eq!(
        shape("let f: fn({a: Int = 1}): Int = g"),
        "let f: fn({a: Int = 1}): Int = g"
    );
    assert_eq!(
        shape("1 : {a: {b: Int = 1} = { b = 2 }, c: List<{d: String = \"x\"}>} | Int"),
        "(1 : ({a: {b: Int = 1} = {b = 2}, c: List<{d: String = \"x\"}>} | Int))"
    );
    assert_eq!(shape("x is {a: Int = k + 1}"), "(x is {a: Int = (k + 1)})");
    // Newlines around the `=`, and a trailing block in a default, the
    // braces being the type's.
    assert_eq!(
        shape("let p: {\n  a: Int\n  =\n  1,\n  b: Int = g { x = 1 }\n} = q"),
        "let p: {a: Int = 1, b: Int = g(){x = 1}} = q"
    );
    // A spread has none.
    assert_eq!(
        error("1 : {...T = 1}"),
        ("expected `,` or `}`".to_owned(), Span::new(10, 11))
    );
    assert_eq!(
        error("1 : {a: Int = }"),
        (
            "expected expression, found `}`".to_owned(),
            Span::new(14, 15)
        )
    );
    // A default is parsed after its field's type, before what follows
    // the type: the first node of a `let`'s block and of a function, as
    // a parameter's default is.
    let text = "fn(x: {a: Int = 1}, y: Int = 2): {r: Int = 3} { x }";
    let ast = parse_text(text).unwrap();
    let value = only_value(&ast);
    let first = ast.subtree(value).next().unwrap();
    assert_eq!(ast.get(first).span, Span::new(16, 17));
    assert_eq!(ast.subtree(value).count(), ast.len());
    let text = "{\n  let p: List<{a: Int = 7}> = q\n  = p\n}";
    let ast = parse_text(text).unwrap();
    let value = only_value(&ast);
    let first = ast.subtree(value).next().unwrap();
    assert_eq!(
        &text[ast.get(first).span.start as usize..],
        "7}> = q\n  = p\n}"
    );
    assert_eq!(ast.subtree(value).count(), ast.len());
    // The tree dump shows a default after its field's type.
    assert_eq!(
        parse_text("1 : {a: Int = -1, b: List<{c: String}> = [{ c = \"x\" }], d: Int = k}")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (expr @0..67\n    (: {a: Int = -1, b: List<{c: String}> = \
             [{c = \"x\"}], d: Int = ...} @0..67\n      (int 1 @0..1))))"
    );
}

#[test]
fn record_type_errors() {
    assert_eq!(
        error("1 : {1: Int}"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("1 : {x: Int,,}"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("1 : {,}"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("1 : {x Int}"),
        ("expected `:` after field name".to_owned(), Span::new(7, 10))
    );
    assert_eq!(
        error("1 : {x := Int}"),
        ("expected `:` after field name".to_owned(), Span::new(7, 9))
    );
    assert_eq!(
        error("1 : {x: Int y: Int}"),
        ("expected `,` or `}`".to_owned(), Span::new(12, 13))
    );
    assert_eq!(
        error("1 : {x: Int"),
        ("expected `,` or `}`".to_owned(), Span::new(11, 11))
    );
    assert_eq!(
        error("1 : {x: }"),
        ("expected a type".to_owned(), Span::new(8, 9))
    );
    assert_eq!(
        error("1 : {"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(5, 5)
        )
    );
}

#[test]
fn a_record_type_may_have_a_rest_field() {
    assert_eq!(shape("1 : {*: String}"), "(1 : {*: String})");
    // Anywhere among the fields and spreads, and with newlines around its
    // parts, as a field's; how many it may have is the checker's
    // business.
    assert_eq!(
        shape("let p: {\n  to: String,\n  *\n  :\n  Int,\n  ...T\n} = q"),
        "let p: {to: String, *: Int, ...T} = q"
    );
    assert_eq!(
        shape("1 : {*: Int, *: String}"),
        "(1 : {*: Int, *: String})"
    );
    let ast = parse_text("1 : {a: Int, *: {}}").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Ascribe { ty, .. } = ast.get(root).kind else {
        panic!("expected an ascription");
    };
    let TypeExprKind::Record(fields) = ast.type_expr(ty).kind else {
        panic!("expected a record type");
    };
    let fields = ast.field_tys(fields);
    assert_eq!(fields[1].kind, FieldTyKind::Rest);
    assert_eq!(fields[1].name(), None);
    assert_eq!(fields[1].span, Span::new(13, 14));
    assert_eq!(ast.type_expr(fields[1].ty).span, Span::new(16, 18));
    // It has no default, and needs its `:`.
    assert_eq!(
        error("1 : {*: Int = 1}"),
        (
            "a rest-field cannot have a default".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("1 : {* Int}"),
        ("expected `:` after `*`".to_owned(), Span::new(7, 10))
    );
}

#[test]
fn pretty_output_for_records() {
    let text = "let p: {x: Int, \"a b\": {}} = { x = 1; \"a b\" = {} }\n-p.x";
    let ast = parse_text(text).unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let p : {x: Int, \"a b\": {}} @0..50\n\
             \x20   (block @29..50\n\
             \x20     (field x @31..36\n\
             \x20       (int 1 @35..36))\n\
             \x20     (field \"a b\" @38..48\n\
             \x20       (block @46..48))))\n\
             \x20 (expr @51..55\n\
             \x20   (neg @51..55\n\
             \x20     (. x @52..55\n\
             \x20       (name p @52..53)))))"
    );
}

#[test]
fn field_chains_and_record_types_count_towards_depth() {
    // A chain of N accesses is N + 1 levels: each is one above its
    // record. This nests without recursion in the parser.
    let chain = |accesses: usize| format!("a{}", ".b".repeat(accesses));
    let deepest = chain(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(chain(MAX_DEPTH));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // An ascription is one above the taller of its expression and its
    // type, and a record type one above its tallest field type: N
    // nested record types are N + 1 levels.
    let nested = |levels: usize| format!("1 : {}Int{}", "{a: ".repeat(levels), "}".repeat(levels));
    let deepest = nested(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(nested(MAX_DEPTH - 1));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Far beyond the limit, record types are stopped by the recursion
    // count before they could overflow.
    let diagnostic = too_deep(nested(200_000));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // In an annotation, the type counts towards its block's height.
    let annotated = |levels: usize| {
        format!(
            "{{ let x: {}Int{} = 1 }}",
            "{a: ".repeat(levels),
            "}".repeat(levels)
        )
    };
    let deepest = annotated(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(annotated(MAX_DEPTH - 1));
    assert_eq!(diagnostic.message, "expression nested too deeply");
}

#[test]
fn tokens_without_eof_are_treated_as_ending() {
    let source = SourceFile::new("test.gcsl", "1 +").unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    let without_eof = &tokens[..tokens.len() - 1];
    let diagnostic = parse(without_eof, names, strings, &source).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected expression, found end of input"
    );
    assert_eq!(diagnostic.span, Span::new(3, 3));
}
