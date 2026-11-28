use super::*;

#[test]
fn handle_types_parse() {
    assert_eq!(
        shape("extern type Vm = \"vm.machine\"\nextern type Network"),
        "extern type Vm = \"vm.machine\"; extern type Network = \"Network\""
    );
    let ast = parse_text("extern type Vm = \"vm.machine\"\nextern type Network").unwrap();
    let [vm, network] = ast.extern_types() else {
        panic!("expected two handle types");
    };
    // The item is an enum with no bound written, from `extern` on.
    let item = ast.item(vm.item);
    assert!(matches!(
        item.kind,
        ItemKind::Enum {
            bound: None,
            name_span: Span { start: 12, end: 14 },
            ..
        }
    ));
    assert_eq!(item.span, Span::new(0, 29));
    assert_eq!(vm.keyword, Span::new(0, 6));
    assert_eq!(vm.kind_span, Some(Span::new(17, 29)));
    assert_eq!(ast.strings().get(vm.kind), "vm.machine");
    // Left out, the kind is the type's name.
    assert_eq!(ast.item(network.item).span, Span::new(30, 49));
    assert_eq!(network.kind_span, None);
    assert_eq!(ast.strings().get(network.kind), "Network");
    assert!(ast.extern_type(vm.item).is_some());
    assert!(ast.extern_fns().is_empty());
}

#[test]
fn operations_parse_as_the_function_items_they_are() {
    for (text, printed) in [
        (
            "extern fn boot(profile: Profile, nics: List<Network> = []) -> Vm = \"vm.boot\"",
            "extern fn boot(profile: Profile, nics: List<Network> = list()) -> Vm = \"vm.boot\"",
        ),
        // A method, whose receiver is its first parameter.
        (
            "extern fn (m: Vm) read(path: String, limit: Size = 64MiB) -> String = \"vm.file.read\"",
            "extern fn \"<method read@0>\"(m: Vm, path: String, limit: Size = 64MiB) -> String = \
             \"vm.file.read\"",
        ),
        (
            "extern def (m: Vm) run(...argv: String) -> {status: Int, stdout: String} = \"vm.run\"",
            "extern fn \"<method run@0>\"(m: Vm, argv: String) -> {status: Int, stdout: String} = \
             \"vm.run\"",
        ),
        // Left out, the operation's name is the function's own, the last
        // of a dotted path's.
        (
            "extern fn halt(m: Vm) -> {}",
            "extern fn halt(m: Vm) -> {} = \"halt\"",
        ),
        (
            "extern def a.b.c(^arch) -> Int",
            "extern def a.b.c(arch: ^arch = ^arch) -> Int = \"c\"",
        ),
        // The return type may start the next line, as any function's.
        (
            "extern fn f() ->\n  Int = \"f.g\"",
            "extern fn f() -> Int = \"f.g\"",
        ),
    ] {
        assert_eq!(shape(text), printed, "{text}");
    }
    // A name on the next line is no part of it: a newline ends the item.
    assert_eq!(
        shape("extern fn f() -> Int\ng = \"x\""),
        "extern fn f() -> Int = \"f\"; g = \"x\""
    );
}

#[test]
fn an_operation_is_recorded_with_its_names_and_spans() {
    let text = "extern def (m: Vm) run(...argv: String) -> Int = \"vm.run\"";
    let ast = parse_text(text).unwrap();
    let [operation] = ast.extern_fns() else {
        panic!("expected one operation");
    };
    assert_eq!(operation.keyword, Span::new(0, 6));
    assert_eq!(operation.declared, Span::new(19, 22));
    assert_eq!(ast.strings().get(operation.name), "vm.run");
    assert_eq!(operation.name_span, Some(Span::new(49, 57)));
    // The item and its function span the `extern` through the name.
    let method = ast.methods()[0];
    assert!(method.public);
    assert_eq!(ast.item(method.item).span, Span::new(0, 57));
    assert_eq!(ast.extern_fn_of(method.item), Some(operation));
    let decl = ast.fn_decl(operation.function);
    assert_eq!(decl.effect, Effect::Impure);
    // With no name written, it is the function's, and the item ends at
    // the return type.
    let ast = parse_text("extern fn halt(m: Vm) -> {}").unwrap();
    let [operation] = ast.extern_fns() else {
        panic!("expected one operation");
    };
    assert_eq!(operation.name_span, None);
    assert_eq!(operation.declared, Span::new(10, 14));
    assert_eq!(ast.strings().get(operation.name), "halt");
    assert_eq!(ast.item(ast.items(ast.root())[0]).span, Span::new(0, 27));
}

#[test]
fn an_operation_s_body_calls_the_host_with_its_name_and_parameters() {
    let text = "extern fn (m: Vm) read(path: String, limit: Size = 64MiB) -> String = \"r\"";
    let ast = parse_text(text).unwrap();
    let whole = Span::new(0, 73);
    let decl = ast.fn_decl(ast.extern_fns()[0].function);
    let ExprKind::Block { items, label: None } = ast.get(decl.body).kind else {
        panic!("expected a block");
    };
    let [item] = ast.items(items) else {
        panic!("expected one item");
    };
    let ItemKind::Expr(call) = ast.item(*item).kind else {
        panic!("expected an expression");
    };
    let ExprKind::Call { callee, args } = ast.get(call).kind else {
        panic!("expected a call");
    };
    assert_eq!(ast.get(callee).kind, ExprKind::Operation);
    let [name, list] = ast.args(args) else {
        panic!("expected two arguments");
    };
    assert_eq!(
        (name.kind, list.kind),
        (ArgKind::Positional, ArgKind::Positional)
    );
    let ExprKind::Str(text) = ast.get(name.value).kind else {
        panic!("expected a string");
    };
    assert_eq!(ast.strings().get(text), "r");
    let ExprKind::List(elements) = ast.get(list.value).kind else {
        panic!("expected a list");
    };
    // A read of each parameter, the receiver first, spanning the
    // parameter; every other node spans the declaration.
    let reads: Vec<(&str, Span)> = ast
        .elements(elements)
        .iter()
        .map(|element| {
            let ExprKind::Name(name) = ast.get(element.value).kind else {
                panic!("expected a name");
            };
            assert_eq!(element.spread, None);
            (ast.names().get(name), ast.get(element.value).span)
        })
        .collect();
    assert_eq!(
        reads,
        [
            ("m", Span::new(11, 16)),
            ("path", Span::new(23, 35)),
            ("limit", Span::new(37, 48)),
        ]
    );
    for id in [callee, name.value, list.value, call, decl.body] {
        assert_eq!(ast.get(id).span, whole);
    }
    // Its nodes are its function's subtree, the default first.
    let ExprKind::Fn(_) = ast.get(only_value(&ast)).kind else {
        panic!("expected a function");
    };
    assert_eq!(ast.subtree(only_value(&ast)).count(), 10);
}

#[test]
fn extern_is_an_identifier_everywhere_else() {
    assert_eq!(
        shape("extern = 1\nlet extern = 2\ny = extern.x\nz = f(extern)"),
        "extern = 1; let extern = 2; y = (extern . x); z = f(extern)"
    );
    // A reference made with a def-type named `extern`, as before, with
    // contexts bound around it or not, and its own name on a line.
    assert_eq!(
        shape("extern type with { a = 1 }"),
        "((extern type) with {a = 1})"
    );
    assert_eq!(shape("extern type"), "(extern type)");
    assert_eq!(shape("x = extern type"), "x = (extern type)");
    assert_eq!(shape("def extern type() { 1 }"), "def extern type() {1}");
    assert_eq!(
        shape("extern\nfn f(): Int { 1 }"),
        "extern; fn f(): Int {1}"
    );
    assert!(
        parse_text("extern type with { a = 1 }")
            .unwrap()
            .extern_types()
            .is_empty()
    );
    // In a block it starts no item, and is the error it always was.
    assert_eq!(
        error("x = { extern fn f() -> Int }"),
        (
            "expected end of item, found `fn`".to_owned(),
            Span::new(13, 15)
        )
    );
    assert_eq!(
        error("x = { extern type Vm }"),
        (
            "expected end of item, found `Vm`".to_owned(),
            Span::new(18, 20)
        )
    );
}

#[test]
fn what_an_operation_cannot_be() {
    const MARKER: &str = "an operation is an effect of the host's, run on every call: it is not \
                          pure, and it is never memoised; a memoised function may call one";
    const GENERIC: &str =
        "the host is given plain data and handles, whose types the declaration says";
    const DEFINITION: &str = "its def-type says what a definition is; declare the operation with \
                              `extern def NAME(...)` and call it from the definition";
    for (text, message, span, note) in [
        (
            "extern fn f(): Int",
            "an operation is declared with `->`",
            Span::new(13, 14),
            MARKER,
        ),
        (
            "extern fn f() ->> Int",
            "an operation is declared with `->`",
            Span::new(14, 17),
            MARKER,
        ),
        (
            "extern fn f<T>(x: T) -> T",
            "an operation has no type parameters",
            Span::new(11, 12),
            GENERIC,
        ),
        (
            "extern def (m: Vm) f<T>(x: T) -> T",
            "an operation has no type parameters",
            Span::new(20, 21),
            GENERIC,
        ),
        (
            "extern def a.f<T>(x: T) -> T",
            "an operation has no type parameters",
            Span::new(14, 15),
            GENERIC,
        ),
        (
            "extern fn f(x: Int, body: block -> Int) -> Int",
            "an operation takes no block",
            Span::new(26, 38),
            "the host is given plain data and handles; write a function that takes the block \
             and calls the operation",
        ),
        (
            "extern def package x() -> Int",
            "a definition made with a def-type is not `extern`",
            Span::new(0, 6),
            DEFINITION,
        ),
        (
            "extern def Toolchain::C gcc() -> Int",
            "a definition made with a def-type is not `extern`",
            Span::new(0, 6),
            DEFINITION,
        ),
        (
            "extern def a.b package() -> Int",
            "a definition made with a def-type is not `extern`",
            Span::new(0, 6),
            DEFINITION,
        ),
        (
            "#[def_type]\nextern def f(name: String) -> Int",
            "`#[def_type]` does not apply to an operation",
            Span::new(0, 11),
            "a def-type is called with each definition made with it, which the host knows \
             nothing of",
        ),
    ] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(
            (diagnostic.message.as_str(), diagnostic.span),
            (message, span),
            "{text}"
        );
        assert_eq!(diagnostic.notes, [note], "{text}");
        assert!(labels(&diagnostic).is_empty(), "{text}");
    }
    // A body, labelled at the `extern`.
    let diagnostic = parse_text("extern fn f() -> Int { 1 }").unwrap_err();
    assert_eq!(diagnostic.message, "an operation has no body");
    assert_eq!(diagnostic.span, Span::new(21, 22));
    assert_eq!(
        labels(&diagnostic),
        [(Span::new(0, 6), "declared `extern` here")]
    );
    assert_eq!(
        diagnostic.notes,
        [
            "the host runs an operation: declare it by its signature alone, `extern fn NAME(...) -> \
          T`"
        ]
    );
    assert_eq!(
        error("extern fn () -> Int"),
        (
            "expected the operation's name after `extern fn`, found `(`".to_owned(),
            Span::new(10, 11)
        )
    );
}

#[test]
fn the_name_a_host_knows_a_declaration_by_is_plain_text() {
    const OWN: &str = "leave the `=` and the string out for the declaration's own name";
    const SAME: &str =
        "the host knows it by this text, which is the same wherever the declaration is read";
    for (text, message, span, notes) in [
        (
            "extern fn f() -> Int = \"\"",
            "an operation's name is not empty",
            Span::new(23, 25),
            &[OWN][..],
        ),
        (
            "extern fn f(x: Int) -> Int = \"a{x}\"",
            "an operation's name is plain text",
            Span::new(29, 35),
            &[SAME],
        ),
        (
            "extern fn f() -> Int = 3",
            "expected the operation's name, a string, after `=`, found `3`",
            Span::new(23, 24),
            &[],
        ),
        (
            "extern type Vm = \"\"",
            "a handle type's kind is not empty",
            Span::new(17, 19),
            &[OWN],
        ),
        (
            "extern type Vm = \"a{x}\"",
            "a handle type's kind is plain text",
            Span::new(17, 23),
            &[SAME],
        ),
        (
            "extern type Vm = 3",
            "expected the kind's name, a string, after `=`, found `3`",
            Span::new(17, 18),
            &[],
        ),
        (
            "extern type Vm<T>",
            "a handle type has no type parameters",
            Span::new(14, 15),
            &["a handle stands for one thing of the host's, of one kind"],
        ),
    ] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(
            (diagnostic.message.as_str(), diagnostic.span),
            (message, span),
            "{text}"
        );
        assert_eq!(diagnostic.notes, notes, "{text}");
    }
}

#[test]
fn an_extern_item_that_fails_is_one_error_item() {
    // The item after it is parsed as ever, and nothing of the operation
    // stays in the tree.
    let source = SourceFile::new(
        "test.gcsl",
        "extern fn f(): Int\nx = 1\nextern fn g() -> Int",
    )
    .unwrap();
    let (ast, errors) = parse_source(&source);
    assert_eq!(errors.len(), 1);
    let kinds: Vec<bool> = ast
        .items(ast.root())
        .iter()
        .map(|&item| matches!(ast.item(item).kind, ItemKind::Error))
        .collect();
    assert_eq!(kinds, [true, false, false]);
    let [operation] = ast.extern_fns() else {
        panic!("expected one operation");
    };
    assert_eq!(ast.strings().get(operation.name), "g");
}
