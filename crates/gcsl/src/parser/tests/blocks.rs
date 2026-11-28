//! Tests of the parser: block parameters, their types and the errors of
//! where one is written.

use super::mutation::error_with_notes;
use super::*;

/// The type of the last parameter of the function expression that is the
/// value of the file's only item, in `ast`.
fn last_param_type(ast: &Ast) -> TypeExprId {
    let value = match ast.item(ast.items(ast.root())[0]).kind {
        ItemKind::Expr(value) | ItemKind::Let { value, .. } | ItemKind::Field { value, .. } => {
            value
        }
        other => panic!("expected a function's item, got {other:?}"),
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let params = ast.params(ast.fn_decl(function).params);
    params.last().unwrap().ty
}

#[test]
fn a_block_parameter_is_written_with_either_marker() {
    assert_eq!(
        shape("fn step(name: String, body: block -> {}) -> {} { body() }"),
        "fn step(name: String, body: block -> {}) -> {} {body()}"
    );
    assert_eq!(
        shape("fn twice(body: block: Int): Int { body() + body() }"),
        "fn twice(body: block: Int): Int {(body() + body())}"
    );
    // The block alone, and its marker glued or spaced.
    assert_eq!(
        shape("fn run(body:block->Int) -> Int { body() }"),
        "fn run(body: block -> Int) -> Int {body()}"
    );
    assert_eq!(
        shape("fn run(body:block:Int): Int { body() }"),
        "fn run(body: block: Int): Int {body()}"
    );
    // The result is any type: a union takes every member, and a function
    // type its own marker.
    assert_eq!(
        shape("fn run(body: block -> Int | String) -> {} { {} }"),
        "fn run(body: block -> (Int | String)) -> {} {{}}"
    );
    assert_eq!(
        shape("fn run(body: block -> List<{a: Int}>) -> {} { {} }"),
        "fn run(body: block -> List<{a: Int}>) -> {} {{}}"
    );
    assert_eq!(
        shape("fn run(body: block: fn(Int): Int): Int { body()(1) }"),
        "fn run(body: block: fn(Int): Int): Int {body()(1)}"
    );
    assert_eq!(
        shape("fn run(body: block -> Future<Int>) -> Int { await body() }"),
        "fn run(body: block -> Future<Int>) -> Int {(await body())}"
    );
    // Newlines after the marker, as after a function's.
    assert_eq!(
        shape("fn run(body: block ->\n  Int) -> Int { body() }"),
        "fn run(body: block -> Int) -> Int {body()}"
    );
    assert_eq!(
        shape("fn run(\n  name: String,\n  body: block: Int,\n): Int { body() }"),
        "fn run(name: String, body: block: Int): Int {body()}"
    );
}

#[test]
fn every_form_of_function_takes_a_block() {
    for (text, printed) in [
        (
            "fn f(body: block -> Int) -> Int { body() }",
            "fn f(body: block -> Int) -> Int {body()}",
        ),
        (
            "def f(body: block -> Int) -> Int { body() }",
            "def f(body: block -> Int) -> Int {body()}",
        ),
        (
            "let f = fn(body: block: Int): Int { body() }",
            "let f = fn(body: block: Int): Int {body()}",
        ),
        (
            "f = fn(n: Int, body: block -> Int) -> Int { body() }",
            "f = fn(n: Int, body: block -> Int) -> Int {body()}",
        ),
        // After a parameter with a default.
        (
            "fn f(n: Int = 1, m: Int, body: block -> Int) -> Int { body() }",
            "fn f(n: Int = 1, m: Int, body: block -> Int) -> Int {body()}",
        ),
    ] {
        assert_eq!(shape(text), printed, "{text}");
        assert_eq!(shape(printed), printed, "{printed}");
    }
    // A generic function's block may give a type parameter.
    assert_eq!(
        shape("fn measure<T>(name: String, body: block -> T) -> T { body() }"),
        "fn measure(name: String, body: block -> T) -> T {body()}"
    );
    // After a rest parameter, in a function and in a function type.
    assert!(parse_text("fn f(...more: Int, body: block -> Int) -> Int { body() }").is_ok());
    assert!(parse_text("let f: fn(...Int, block -> Int) -> Int = g").is_ok());
    // A method takes one after its receiver.
    let ast = parse_text("fn (n: Int) times(body: block -> {}) -> {} { body() }").unwrap();
    assert_eq!(ast.methods().len(), 1);
    let method = ast.methods()[0];
    let ItemKind::Let { value, .. } = ast.item(method.item).kind else {
        panic!("expected a method's `let`");
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let params = ast.params(ast.fn_decl(function).params);
    assert_eq!(params.len(), 2);
    assert!(matches!(
        ast.type_expr(params[1].ty).kind,
        TypeExprKind::Block {
            effect: Effect::Impure,
            ..
        }
    ));
}

#[test]
fn a_block_type_holds_its_result_its_effect_and_its_span() {
    let text = "fn(name: String, body: block -> {a: Int}) -> {} { body() }";
    let ast = parse_text(text).unwrap();
    let ty = ast.type_expr(last_param_type(&ast));
    // From `block` to the end of its result.
    assert_eq!(ty.span, Span::new(23, 40));
    assert_eq!(&text[23..40], "block -> {a: Int}");
    let TypeExprKind::Block { ret, effect } = ty.kind else {
        panic!("expected a block type, got {ty:?}");
    };
    assert_eq!(effect, Effect::Impure);
    assert_eq!(ast.type_expr(ret).span, Span::new(32, 40));
    // The pure marker.
    let text = "fn(body: block: Int): Int { body() }";
    let ast = parse_text(text).unwrap();
    let ty = ast.type_expr(last_param_type(&ast));
    assert_eq!(ty.span, Span::new(9, 19));
    let TypeExprKind::Block { ret, effect } = ty.kind else {
        panic!("expected a block type, got {ty:?}");
    };
    assert_eq!(effect, Effect::Pure);
    assert_eq!(ast.type_expr(ret).span, Span::new(16, 19));
    // The parameter spans its name through its block type.
    let ExprKind::Fn(function) = ast.get(only_value(&ast)).kind else {
        panic!("expected a function");
    };
    let spans: Vec<Span> = ast
        .params(ast.fn_decl(function).params)
        .iter()
        .map(|param| param.span)
        .collect();
    assert_eq!(spans, vec![Span::new(3, 19)]);
}

#[test]
fn the_tree_dump_shows_a_block_type_as_it_is_written() {
    let pretty = parse_text("fn(n: Int, body: block -> Int) -> Int { body() }")
        .unwrap()
        .pretty()
        .to_string();
    assert!(
        pretty.contains("(params (n Int) (body block -> Int))"),
        "{pretty}"
    );
    let pretty = parse_text("fn(body: block: {a: Int}): Int { body().a }")
        .unwrap()
        .pretty()
        .to_string();
    assert!(
        pretty.contains("(params (body block: {a: Int}))"),
        "{pretty}"
    );
    let pretty = parse_text("type Runner = fn(String, block -> {}) -> {}")
        .unwrap()
        .pretty()
        .to_string();
    assert!(pretty.contains("fn(String, block -> {}) -> {}"), "{pretty}");
}

#[test]
fn a_function_type_takes_a_block_as_its_last_parameter() {
    for text in [
        "let f: fn(block -> {}) -> {} = g",
        "let f: fn(String, block -> Int) -> Int = g",
        "let f: fn(block: Int): Int = g",
        "let f: fn(name: String, body: block: Int): Int = g",
        "let f: fn(Int, block -> Int, ^arch) -> Int = g",
    ] {
        assert_eq!(shape(text), text, "{text}");
    }
    // As a parameter's type, a field's and a type item's.
    assert_eq!(
        shape("fn each(run: fn(String, block -> {}) -> {}) -> {} { run(\"a\") { {} } }"),
        "fn each(run: fn(String, block -> {}) -> {}) -> {} {run(\"a\"){{}}}"
    );
    assert!(parse_text("type Runner = fn(String, block: Int): Int").is_ok());
    assert!(parse_text("x: fn(block -> {}) -> {} = g").is_ok());
    let ast = parse_text("let f: fn(Int, body: block -> {a: Int}) -> {} = g").unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated `let`");
    };
    let TypeExprKind::Fn { params, .. } = ast.type_expr(ty).kind else {
        panic!("expected a function type");
    };
    let params = ast.param_tys(params);
    assert_eq!(params.len(), 2);
    let block = ast.type_expr(params[1].ty);
    assert_eq!(block.span, Span::new(21, 38));
    assert!(matches!(
        block.kind,
        TypeExprKind::Block {
            effect: Effect::Impure,
            ..
        }
    ));
}

#[test]
fn block_is_a_name_wherever_no_marker_follows_it() {
    // A variable, a field, a parameter and a function of that name.
    assert_eq!(shape("let block = 1"), "let block = 1");
    assert_eq!(shape("block = block + 1"), "block = (block + 1)");
    assert_eq!(shape("block(1) { 2 }"), "block(1){2}");
    assert_eq!(
        shape("fn f(block: Int): Int { block }"),
        "fn f(block: Int): Int {block}"
    );
    // A parameter named `block` whose type is a block's.
    assert_eq!(
        shape("fn f(block: block: Int): Int { block() }"),
        "fn f(block: block: Int): Int {block()}"
    );
    // With no marker it is a type's name, which the checker looks up.
    assert_eq!(
        shape("fn f(body: block): Int { 1 }"),
        "fn f(body: block): Int {1}"
    );
    assert_eq!(shape("let x: block = 1"), "let x: block = 1");
    assert_eq!(
        shape("let x: List<block> = []"),
        "let x: List<block> = list()"
    );
}

#[test]
fn a_block_is_a_function_s_last_parameter() {
    let note = "a call gives the block after its other arguments, as `f(x) { ... }`";
    let message = "a block is a function's last parameter";
    assert_eq!(
        error_with_notes("fn f(body: block -> {}, x: Int) -> {} { body() }"),
        (message.to_owned(), Span::new(11, 22), vec![note.to_owned()])
    );
    // Two blocks: the first is not the last.
    assert_eq!(
        error_with_notes("fn f(a: block: Int, b: block: Int): Int { a() }"),
        (message.to_owned(), Span::new(8, 18), vec![note.to_owned()])
    );
    // In a function expression, a `def`, a method and a function type.
    assert_eq!(
        error_with_notes("let f = fn(body: block -> Int, x: Int) -> Int { body() }"),
        (message.to_owned(), Span::new(17, 29), vec![note.to_owned()])
    );
    assert_eq!(
        error("def f(body: block -> Int, x: Int) -> Int { body() }"),
        (message.to_owned(), Span::new(12, 24))
    );
    assert_eq!(
        error("fn (n: Int) f(body: block -> Int, x: Int) -> Int { body() }"),
        (message.to_owned(), Span::new(20, 32))
    );
    assert_eq!(
        error_with_notes("type F = fn(block -> {}, Int) -> {}"),
        (message.to_owned(), Span::new(12, 23), vec![note.to_owned()])
    );
    // A need, which is no parameter, comes after it.
    assert_eq!(
        shape("let f: fn(block -> Int, ^arch) -> Int = g"),
        "let f: fn(block -> Int, ^arch) -> Int = g"
    );
    assert_eq!(
        error("let f: fn(^arch, block -> Int) -> Int = g").0,
        "a function type's needs come after its parameters"
    );
}

#[test]
fn a_block_is_no_receiver_rest_parameter_or_parameter_with_a_default() {
    assert_eq!(
        error_with_notes("fn (body: block -> {}) run() -> {} { body() }"),
        (
            "a method's receiver is not a block".to_owned(),
            Span::new(10, 21),
            vec![]
        )
    );
    assert_eq!(
        error_with_notes("fn f(...body: block -> Int) -> Int { 1 }"),
        (
            "a rest parameter is not a block".to_owned(),
            Span::new(14, 26),
            vec![]
        )
    );
    assert_eq!(
        error("let f = fn(...body: block: Int): Int { 1 }"),
        (
            "a rest parameter is not a block".to_owned(),
            Span::new(20, 30)
        )
    );
    assert_eq!(
        error("type F = fn(...block -> Int) -> Int"),
        (
            "a rest parameter is not a block".to_owned(),
            Span::new(15, 27)
        )
    );
    assert_eq!(
        error_with_notes("fn f(body: block -> Int = 1) -> Int { body() }"),
        (
            "a block parameter has no default".to_owned(),
            Span::new(24, 25),
            vec!["every call gives the block, as `f(x) { ... }`".to_owned()]
        )
    );
    // A function type's parameter has no default at all.
    assert_eq!(
        error("type F = fn(body: block: Int = 1): Int"),
        (
            "a default is written only in a function expression's parameters".to_owned(),
            Span::new(29, 30)
        )
    );
}

#[test]
fn neither_a_block_nor_a_function_that_takes_one_is_memoised() {
    let note = "a block runs each time its function calls it; write `block -> T`, or `block: T` \
                for one that is pure";
    assert_eq!(
        error_with_notes("fn f(body: block ->> Int) -> Int { body() }"),
        (
            "a block is not memoised".to_owned(),
            Span::new(17, 20),
            vec![note.to_owned()]
        )
    );
    assert_eq!(
        error_with_notes("type F = fn(block ->> Int) -> Int"),
        (
            "a block is not memoised".to_owned(),
            Span::new(18, 21),
            vec![note.to_owned()]
        )
    );
    let note = "a memoised call is found by its arguments, and a block is no value to find it \
                by; declare the function with `->`";
    let message = "a memoised function cannot take a block";
    assert_eq!(
        error_with_notes("fn f(body: block -> Int) ->> Int { body() }"),
        (message.to_owned(), Span::new(25, 28), vec![note.to_owned()])
    );
    assert_eq!(
        error_with_notes("let f = fn(n: Int, body: block: Int) ->> Int { body() }"),
        (message.to_owned(), Span::new(37, 40), vec![note.to_owned()])
    );
    assert_eq!(
        error_with_notes("type F = fn(block -> Int) ->> Int"),
        (message.to_owned(), Span::new(26, 29), vec![note.to_owned()])
    );
    // A memoised function that takes none, and gives one, is as it was.
    assert_eq!(
        shape("fn f(n: Int) ->> Int { run { n } }"),
        "fn f(n: Int) ->> Int {run(){n}}"
    );
}

#[test]
fn a_block_type_is_written_only_as_a_parameter_s_type() {
    // Anywhere else `block` is a type's name, and its marker what follows
    // a type there.
    assert_eq!(
        error("let x: block -> Int = 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(13, 15)
        )
    );
    assert_eq!(
        error("type B = block -> Int"),
        (
            "expected end of item, found `->`".to_owned(),
            Span::new(15, 17)
        )
    );
    assert_eq!(
        error("fn f(xs: List<block -> Int>) -> Int { 1 }"),
        (
            "expected `>` to close the type arguments".to_owned(),
            Span::new(20, 22)
        )
    );
    assert_eq!(
        error("fn f(body: (block -> Int)) -> Int { 1 }"),
        (
            "expected `)` to close the type".to_owned(),
            Span::new(18, 20)
        )
    );
    // No function returns one.
    assert_eq!(
        error("fn f(body: block -> Int) -> block -> Int { body }"),
        (
            "expected `{` to start the function body".to_owned(),
            Span::new(34, 36)
        )
    );
    assert_eq!(
        error("fn f(g: fn() -> block -> Int) -> Int { 1 }"),
        (
            "expected `,` or `)`, found `->`".to_owned(),
            Span::new(22, 24)
        )
    );
    // A context parameter's type is its context's, never a block's.
    assert_eq!(
        message("fn f(^body: block -> Int) -> Int { 1 }"),
        "expected `,` or `)`, found `->`"
    );
    // A block has no meta.
    assert_eq!(
        message("fn f(body: block -> Int META -> Int) -> Int { 1 }"),
        "expected `,` or `)`, found `META`"
    );
}

#[test]
fn a_block_type_needs_its_result() {
    assert_eq!(
        error("fn f(body: block ->) -> Int { 1 }"),
        ("expected a type".to_owned(), Span::new(19, 20))
    );
    assert_eq!(
        error("fn f(body: block:) -> Int { 1 }"),
        ("expected a type".to_owned(), Span::new(17, 18))
    );
    // In the parameters' parentheses a newline separates nothing, so the
    // marker may start a line.
    assert_eq!(
        shape("fn f(body: block\n-> Int) -> Int { body() }"),
        "fn f(body: block -> Int) -> Int {body()}"
    );
}

#[test]
fn a_block_s_result_counts_towards_depth() {
    // A block type is one level around its result, as a list type is.
    let nested = |levels: usize| {
        format!(
            "fn f(body: block -> {}Int{}) -> {{}} {{ {{}} }}",
            "List<".repeat(levels),
            ">".repeat(levels)
        )
    };
    assert!(parse_text(&nested(8)).is_ok());
    assert_eq!(
        too_deep(nested(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
}
