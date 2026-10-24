//! Tests of the parser: type parameters, methods, enums, contexts, `assert` and
//! `recover`.

use super::*;

#[test]
fn a_named_function_takes_type_parameters() {
    let ast = parse_text("fn id<T>(x: T): T { x }\ndef pair<A, B>(a: A, b: B): A { a }").unwrap();
    let all: Vec<Vec<&str>> = ast
        .all_generics()
        .iter()
        .map(|generics| {
            generics
                .params
                .iter()
                .map(|&(name, _)| ast.names().get(name))
                .collect()
        })
        .collect();
    assert_eq!(all, [vec!["T"], vec!["A", "B"]]);
    // In scope in the signature and the body only.
    let text = "fn id<T>(x: T): T { let y: T = x\n y }\nz: T = 1";
    let ast = parse_text(text).unwrap();
    let scoped: Vec<bool> = ast
        .type_ids()
        .map(|id| match ast.type_expr(id).kind {
            TypeExprKind::Named(name) => ast.type_param(id, name).is_some(),
            _ => false,
        })
        .collect();
    assert_eq!(scoped, [true, true, true, false]);
    for (text, message) in [
        (
            "fn f<T, T>(x: T): T { x }",
            "type parameter `T` is declared twice",
        ),
        (
            "fn f<>(x: Int): Int { x }",
            "expected a type parameter's name, found `>`",
        ),
        (
            "fn f<T,>(x: T): T { x }",
            "expected a type parameter's name, found `>`",
        ),
        (
            "fn f<T U>(x: T): T { x }",
            "expected `,` or `>` after a type parameter, found `U`",
        ),
    ] {
        assert_eq!(message_of(text), message, "{text}");
    }
}

pub(super) fn message_of(text: &str) -> String {
    parse_text(text).unwrap_err().message
}

#[test]
fn applied_types_take_argument_lists_and_qualified_names() {
    assert_eq!(
        shape("let x: Pair<Int, String> = y"),
        "let x: Pair<Int, String> = y"
    );
    assert_eq!(
        shape("let x: t.Pair<Int,\n String> = y"),
        "let x: t.Pair<Int, String> = y"
    );
    assert_eq!(
        shape("let x: Box<Box<Int>[]> = y"),
        "let x: Box<(Box<Int>)[]> = y"
    );
    let ast = parse_text("type Pair<A, B> = {first: A, second: B}").unwrap();
    let item = ast.items(ast.root())[0];
    assert_eq!(ast.type_arity(item), 2);
    let scoped: Vec<bool> = ast
        .type_ids()
        .filter_map(|id| match ast.type_expr(id).kind {
            TypeExprKind::Named(name) => Some(ast.type_param(id, name).is_some()),
            _ => None,
        })
        .collect();
    assert_eq!(scoped, [true, true]);
    assert_eq!(
        error("let x: Pair<Int String> = y").0,
        "expected `>` to close the type arguments"
    );
}

#[test]
fn a_method_is_a_function_binding_with_its_receiver_first() {
    let ast =
        parse_text("fn (x: Int) twice(): Int { x * 2 }\ndef (xs: List<T>) first<T>(f: T): T { f }")
            .unwrap();
    let methods: Vec<(&str, bool)> = ast
        .methods()
        .iter()
        .map(|method| (ast.names().get(method.name), method.public))
        .collect();
    assert_eq!(methods, [("twice", false), ("first", true)]);
    let method = ast.methods()[1];
    assert_eq!(ast.names().get(method.binding), "<method first@0>");
    let ItemKind::Let {
        value,
        function: true,
        ..
    } = ast.item(method.item).kind
    else {
        panic!("expected a function binding");
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let params: Vec<&str> = ast
        .params(ast.fn_decl(function).params)
        .iter()
        .map(|param| ast.names().get(param.name))
        .collect();
    assert_eq!(params, ["xs", "f"]);
    // The type parameter is in scope in the receiver.
    let scoped = ast
        .type_ids()
        .filter(|&id| matches!(ast.type_expr(id).kind, TypeExprKind::Named(name) if ast.type_param(id, name).is_some()))
        .count();
    assert_eq!(scoped, 3);
    // A function expression is still one.
    assert!(
        parse_text("f = fn (x: Int): Int { x }")
            .unwrap()
            .methods()
            .is_empty()
    );
}

#[test]
fn method_errors() {
    for (text, message) in [
        (
            "x = { fn (y: Int) m(): Int { y } }",
            "a method is declared only at the top level of a file",
        ),
        ("fn () m(): Int { 1 }", "expected the method's receiver"),
        (
            "fn (x: Int, y: Int) m(): Int { y }",
            "a method has one receiver",
        ),
        (
            "fn (x: Int = 1) m(): Int { x }",
            "a receiver takes no default",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn enum_items_and_variants_parse() {
    assert_eq!(
        shape(
            "enum Shape extensible<{*: Int}>\nenum Shape::Square<{side: Int}>\nenum t.Shape::Dot"
        ),
        "enum Shape extensible<{*: Int}>; enum Shape::Square<{side: Int}>; enum t.Shape::Dot"
    );
    assert_eq!(shape("x = Shape::Square"), "x = Shape::Square");
    assert_eq!(
        shape("x = t.Shape::Square({ side = 1 })"),
        "x = t.Shape::Square({side = 1})"
    );
    // `enum` is a field's name elsewhere.
    assert_eq!(shape("enum = 1"), "enum = 1");
    for (text, message) in [
        (
            "x = { enum Shape extensible }",
            "`enum` is declared only at the top level of a file",
        ),
        (
            "enum Shape",
            "expected `extensible` after the enum's name, found end of input",
        ),
        (
            "enum Shape::",
            "expected the variant's name after `::`, found end of input",
        ),
        (
            "enum Shape extensible<Int",
            "expected `>` to close the type",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn definitions_and_references_made_with_a_variant_parse() {
    assert_eq!(
        shape("def Toolchain::C gcc() { 1 }"),
        "def Toolchain::C gcc() {1}"
    );
    assert_eq!(
        shape("def t.Toolchain::C dev.gcc(x) { x }"),
        "def t.Toolchain::C dev.gcc(x) {x}"
    );
    assert_eq!(shape("r = Toolchain::C gcc()"), "r = (Toolchain::C gcc)()");
    assert_eq!(
        shape("r = t.Toolchain::C gcc.meta"),
        "r = (t.Toolchain::C gcc.meta)"
    );
    assert_eq!(
        shape("defmeta Toolchain::C gcc = 1"),
        "defmeta Toolchain::C gcc {= 1}"
    );
    // A variant followed by no name on its line is a variant as a value.
    assert_eq!(
        shape("r = Toolchain::C\ngcc = 1"),
        "r = Toolchain::C; gcc = 1"
    );
    for (text, message) in [
        (
            "def Toolchain::C() { 1 }",
            "expected the definition's path after `Toolchain::C`, found `(`",
        ),
        (
            "def Toolchain::(x) { 1 }",
            "expected the variant's name after `::`, found `(`",
        ),
        (
            "def Toolchain::C gcc(x: Int) { 1 }",
            "`x` is a parameter of the definition's own, which `Toolchain::C` calls with none: \
             give it a default, or leave its type to `Toolchain::C`",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn contexts_reads_withs_and_context_parameters_parse() {
    assert_eq!(
        shape("context arch: String = \"x86_64\"\ncontext toolchains: List<Int>"),
        "context arch: String = {= \"x86_64\"}; context toolchains: List<Int>"
    );
    assert_eq!(shape("r = ^arch"), "r = ^arch");
    assert_eq!(shape("r = ^platform.arch"), "r = (^platform . arch)");
    assert_eq!(
        shape("r = f() with { arch = \"x\"; platform.os = \"y\" }"),
        "r = (f() with {arch = \"x\"; platform.os = \"y\"})"
    );
    assert_eq!(
        shape("r = a + f() with { arch }"),
        "r = (a + (f() with {arch = arch}))"
    );
    assert_eq!(
        shape("r = with { arch = \"x\" } { f() }"),
        "r = ({f()} with {arch = \"x\"})"
    );
    assert_eq!(
        shape("fn f(^arch, ^os: String): String { arch }"),
        "fn f(arch: ^arch = ^arch, os: String = ^os): String {arch}"
    );
    // `context` and `with` are names everywhere else.
    assert_eq!(
        shape("context = 1\nwith = 2\nr = with"),
        "context = 1; with = 2; r = with"
    );
    // A `with` on the next line is a new item.
    assert_eq!(
        error("r = f()\nwith { arch = \"x\" }").0,
        "expected `{` to start the body of `with`, found end of input"
    );
    for (text, message) in [
        (
            "x = { context y: Int = 1 }",
            "`context` is declared only at the top level of a file",
        ),
        ("r = ^1", "expected a context's name after `^`, found `1`"),
        (
            "r = f() with { 1 }",
            "expected a context's name in `with`, found `1`",
        ),
        (
            "r = f() with { platform.arch }",
            "expected `=` after `platform.arch`, found `}`",
        ),
        (
            "r = f() with { a = 1 b = 2 }",
            "expected `;` or a line break after a binding, found `b`",
        ),
        (
            "fn f(^arch = \"x\"): String { arch }",
            "a context parameter's default is its context",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn assertions_and_recovers_parse() {
    assert_eq!(shape("assert x == 1"), "assert (x == 1), \"`x == 1`\"");
    assert_eq!(
        shape("assert ok, \"it failed\""),
        "assert ok, \"it failed\""
    );
    assert_eq!(
        shape("r = recover { f() } else { 0 }"),
        "r = recover {f()} else {0}"
    );
    assert_eq!(
        shape("r = recover { f() }\nelse (e) { e.message }"),
        "r = recover {f()} else (e) {(e . message)}"
    );
    // `assert` and `recover` are names everywhere else.
    assert_eq!(
        shape("assert = 1\nrecover = 2\nr = recover"),
        "assert = 1; recover = 2; r = recover"
    );
    for (text, message) in [
        (
            "r = recover { 1 } 2",
            "expected `else` after the body of `recover`, found `2`",
        ),
        (
            "r = recover { 1 } else (1) { 2 }",
            "expected the failure's name after `(`, found `1`",
        ),
        (
            "r = recover { 1 } else (e { 2 }",
            "expected `)` after the failure's name, found `{`",
        ),
        (
            "r = recover { 1 } else 2",
            "expected `{` to start the fallback of `recover`, found `2`",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}
