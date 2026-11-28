use super::*;

/// The note of an operation's parameter or result that is no data.
const DATA: &str =
    "the host is given plain data and handles: no function, future, secret or sandbox";

/// The call the parser gave the first operation of `ast` for a body.
fn body_call(ast: &Ast) -> ExprId {
    let decl = ast.fn_decl(ast.extern_fns()[0].function);
    let ExprKind::Block { items, .. } = ast.get(decl.body).kind else {
        panic!("an operation's body is no block");
    };
    let ItemKind::Expr(call) = ast.item(ast.items(items)[0]).kind else {
        panic!("an operation's body holds no call");
    };
    call
}

#[test]
fn an_operation_is_an_impure_function_of_its_declared_type() {
    for (text, ty) in [
        // A bare parameter may be given a future, as any impure
        // function's; the result is present.
        (
            "extern fn f(x: Int, y: String = \"a\") -> Int\nf",
            "fn(x: Int | Future<Int>, y: String | Future<String>) -> Int",
        ),
        ("extern fn f(x: Int, y: String = \"a\") -> Int\nf(1)", "Int"),
        (
            "extern fn f(x: Int, y: String = \"a\") -> Int\nf(y = \"b\", x = 2)",
            "Int",
        ),
        ("extern fn f(x: Int) -> {}\nf(1)", "{}"),
        (
            "extern fn f(x: Future<Int>) -> List<Int>\nf(lazy 1)",
            "List<Int>",
        ),
        (
            "extern fn f(x: Present<Int>) -> Int\nf",
            "fn(x: Int) -> Int",
        ),
        (
            "extern fn f(...words: String) -> Int | String\nf(\"a\", ...[\"b\"])",
            "Int | String",
        ),
        ("extern type Vm\nextern fn boot() -> Vm\nboot()", "Vm"),
        (
            "extern type Vm\nextern fn boot() -> Vm\n\
             extern fn (m: Vm) run(...argv: String) -> {status: Int}\nboot().run(\"a\", \"b\")",
            "{status: Int}",
        ),
        // A handle is data: in records and lists, and under `*`.
        (
            "extern type Vm\nextern fn all(ms: List<{m: Vm}>, any: {*: *}) -> List<Vm>\n\
             extern fn boot() -> Vm\nall([{m = boot()}], {m = boot()})",
            "List<Vm>",
        ),
        // Files and directories, and an enum whose bound is data.
        (
            "enum Tool extensible<{*: String}>\n\
             extern fn f(d: Dir, t: Tool) -> File\nf",
            "fn(d: Dir | Future<Dir>, t: Tool | Future<Tool>) -> File",
        ),
        // It is passed and stored as a value of its type.
        (
            "extern fn f(x: Int) -> Int\n\
             fn twice(g: fn(Int) -> Int) -> Int { g(g(1)) }\ntwice(f)",
            "Int | Future<Int>",
        ),
        (
            "extern fn f(x: Int) -> Int\nlet held = [f]\nheld[0](3)",
            "Int",
        ),
    ] {
        assert_eq!(shown_result(text), ty, "{text}");
    }
}

#[test]
fn a_call_of_an_operation_is_a_call_like_any_other() {
    for (text, message, span) in [
        (
            "extern fn f(x: Int) -> Int\nfn g(): Int { f(1) }\n1",
            "cannot call an impure function from a pure function",
            Span::new(41, 45),
        ),
        (
            "extern fn f(x: Int) -> Int\nf(\"a\")",
            "mismatched types",
            Span::new(29, 32),
        ),
        (
            "extern fn f(x: Int) -> Int\nf()",
            "expected 1 argument, found 0",
            Span::new(27, 30),
        ),
        (
            "extern fn f(x: Int) -> Int\nf(1, y = 2)",
            "unknown argument `y`",
            Span::new(32, 33),
        ),
        (
            "extern fn f(x: Int, x: Int) -> Int\n1",
            "parameter `x` is declared twice",
            Span::new(20, 26),
        ),
        (
            "extern fn f(x: Nope) -> Int\n1",
            "unknown type `Nope`",
            Span::new(15, 19),
        ),
        (
            "extern fn f(x: Int) -> Nope\n1",
            "unknown type `Nope`",
            Span::new(23, 27),
        ),
        (
            "extern fn f(x: Int = \"a\") -> Int\n1",
            "mismatched types",
            Span::new(21, 24),
        ),
    ] {
        assert_eq!(errors(text), [(message.to_owned(), span)], "{text}");
    }
    // A statement that calls one is no unused value.
    assert!(warnings("extern fn f(x: Int) -> {}\nfn g() -> Int { f(1)\n = 2 }\n1").is_empty());
}

#[test]
fn an_operation_s_parameters_and_result_are_data() {
    for (text, message, span, label) in [
        (
            "extern fn f(run: fn(): Int) -> Int\n1",
            "an operation's parameter must be data, found `fn(): Int`",
            Span::new(12, 26),
            "this is fn(): Int",
        ),
        (
            "extern fn f(x: {a: List<fn(): Int>}) -> Int\n1",
            "an operation's parameter must be data, found `{a: List<fn(): Int>}`",
            Span::new(12, 35),
            "this is {a: List<fn(): Int>}",
        ),
        (
            "extern fn f(x: Secret<String>) -> Int\n1",
            "an operation's parameter must be data, found `Secret<String>`",
            Span::new(12, 29),
            "this is Secret<String>",
        ),
        (
            "extern fn f(x: Sandbox) -> Int\n1",
            "an operation's parameter must be data, found Sandbox",
            Span::new(12, 22),
            "this is Sandbox",
        ),
        // An enum with no bound may hold anything.
        (
            "enum S extensible\nextern fn f(x: S) -> Int\n1",
            "an operation's parameter must be data, found S",
            Span::new(30, 34),
            "this is S",
        ),
        (
            "enum S extensible<Int>\nextern fn f(x: Variant<S, Int>) -> Int\n1",
            "an operation's parameter must be data, found `Variant<S, Int>`",
            Span::new(35, 53),
            "this is Variant<S, Int>",
        ),
        // A rest parameter is the list of what fills it.
        (
            "extern fn f(x: Int, ...rest: fn(): Int) -> Int\n1",
            "an operation's parameter must be data, found `List<fn(): Int>`",
            Span::new(20, 38),
            "this is List<fn(): Int>",
        ),
        // A method's receiver is a parameter.
        (
            "extern fn (s: Sandbox) f() -> Int\n1",
            "an operation's parameter must be data, found Sandbox",
            Span::new(11, 21),
            "this is Sandbox",
        ),
        (
            "extern fn f() -> fn(): Int\n1",
            "an operation's result must be data, found `fn(): Int`",
            Span::new(17, 26),
            "this is fn(): Int",
        ),
        (
            "extern fn f() -> {key: Secret<Int>}\n1",
            "an operation's result must be data, found `{key: Secret<Int>}`",
            Span::new(17, 35),
            "this is {key: Secret<Int>}",
        ),
        // The host's answer is a value: a result written a future is none.
        (
            "extern fn f() -> Future<Int>\n1",
            "an operation's result must be data, found `Future<Int>`",
            Span::new(17, 28),
            "this is Future<Int>",
        ),
        (
            "extern fn f() -> List<Future<Int>>\n1",
            "an operation's result must be data, found `List<Future<Int>>`",
            Span::new(17, 34),
            "this is List<Future<Int>>",
        ),
    ] {
        let found = diagnostics(text);
        let [error] = &found[..] else {
            panic!("{text}: {found:?}");
        };
        assert_eq!(
            (error.message.as_str(), error.span),
            (message, span),
            "{text}"
        );
        assert_eq!(error.label.as_deref(), Some(label), "{text}");
        assert_eq!(error.notes, [DATA], "{text}");
        assert!(error.labels.is_empty(), "{text}");
    }
    // Each parameter that is none, and the result, is reported.
    assert_eq!(
        messages("extern fn f(a: Sandbox, b: Int, c: Secret<Int>) -> Sandbox\n1"),
        [
            "an operation's parameter must be data, found Sandbox",
            "an operation's parameter must be data, found `Secret<Int>`",
            "an operation's result must be data, found Sandbox",
        ]
    );
}

#[test]
fn an_operation_s_call_awaits_its_parameters_and_tests_its_result() {
    let text = "extern type Vm\n\
                extern fn f(m: Vm, p: {a: Int}, ...rest: String) -> {status: Int, at: Vm}\n1";
    let (ast, _, output) = check_prelude(text);
    let checked = output.unwrap().checked;
    let call = body_call(&ast);
    // Each read of a parameter is awaited, deeply where it holds more.
    let ExprKind::Call { callee, args } = ast.get(call).kind else {
        panic!("expected a call");
    };
    assert_eq!(ast.get(callee).kind, ExprKind::Operation);
    let ExprKind::List(elements) = ast.get(ast.args(args)[1].value).kind else {
        panic!("expected a list");
    };
    let awaited: Vec<bool> = ast
        .elements(elements)
        .iter()
        .map(|element| checked.resolution.await_of(element.value) != AwaitMark::None)
        .collect();
    assert_eq!(awaited, [true, true, true]);
    // The result's test is of a value known to be of no type: whole.
    let planned = checked.resolution.test_of(call).unwrap();
    assert_eq!(&*planned.subject, "the result of operation `f`");
    assert_eq!(&*planned.target, "a `{at: Vm, status: Int}`");
    let ValueTest::Record(fields) = &planned.test else {
        panic!("expected a record's test, got {:?}", planned.test);
    };
    let tests: Vec<(&str, bool)> = fields
        .iter()
        .map(|(name, field)| (ast.names().get(*name), matches!(field, ValueTest::Enum(_))))
        .collect();
    assert_eq!(tests, [("at", true), ("status", false)]);
    assert!(matches!(fields[1].1, ValueTest::Kind(ValueKind::Int)));
    // A result of `{}` is tested to be a record.
    let (ast, _, output) = check_prelude("extern fn f() -> {}\n1");
    let checked = output.unwrap().checked;
    let planned = checked.resolution.test_of(body_call(&ast)).unwrap();
    assert_eq!(planned.test, ValueTest::Kind(ValueKind::Record));
    assert_eq!(&*planned.target, "a `{}`");
}

#[test]
fn an_operation_reports_its_failures_at_its_call() {
    let tracked = |text: &str| -> Vec<String> {
        let (ast, _, output) = check_prelude(text);
        let checked = output.unwrap().checked;
        checked
            .resolution
            .attributes(ast.extern_fns()[0].function)
            .iter()
            .map(|attribute| match attribute.id {
                AttributeId::Builtin(name) => name.to_owned(),
                AttributeId::Declared { .. } => "declared".to_owned(),
            })
            .collect()
    };
    assert_eq!(tracked("extern fn f() -> Int\n1"), ["track_caller"]);
    // After its own, and once if it is written.
    assert_eq!(
        tracked("#[retry(2)]\nextern fn f() -> Int\n1"),
        ["retry", "track_caller"]
    );
    assert_eq!(
        tracked("#[track_caller]\n#[timeout(1s)]\nextern fn f() -> Int\n1"),
        ["track_caller", "timeout"]
    );
    // No other function is given it.
    let (ast, _, output) = check_prelude("extern fn f() -> Int\nfn g() -> Int { f() }\n1");
    let checked = output.unwrap().checked;
    let functions = u32::try_from(ast.fn_count()).unwrap();
    let with: Vec<usize> = (0..functions)
        .map(|at| {
            checked
                .resolution
                .attributes(crate::ast::FnId::new(at))
                .len()
        })
        .collect();
    assert_eq!(with, [1, 0]);
}

#[test]
fn a_handle_type_is_a_type_of_its_own() {
    for (text, ty) in [
        (
            "extern type Vm\nextern fn boot() -> Vm\nboot() == boot()",
            "Bool",
        ),
        (
            "extern type Vm\nextern fn boot() -> Vm\nboot() === boot()",
            "Bool",
        ),
        (
            "extern type Vm\nextern fn boot() -> Vm\nlet v: Vm | Int = boot()\nv is Vm",
            "Bool",
        ),
        (
            "extern type Vm\nextern fn boot() -> Vm\nlet v: Vm | Int = boot()\nv as Vm",
            "Vm",
        ),
        (
            "extern type Vm\nextern type Disk\nextern fn boot() -> Vm\n\
             let v: Vm | Disk = boot()\nif v is Disk { 1 } else { 2 }",
            "Int",
        ),
        (
            "extern type Vm\nextern fn boot() -> Vm\n\"{boot()}\"",
            "String",
        ),
        (
            "extern type Vm\nextern fn boot() -> Vm\nshow([boot()])",
            "String",
        ),
        (
            "extern type Vm\nextern fn boot() -> Vm\nlet any: {*: *} = {m = boot()}\nany",
            "{*: *}",
        ),
    ] {
        assert_eq!(shown_result(text), ty, "{text}");
    }
}

#[test]
fn handle_type_errors() {
    for (text, message, span) in [
        (
            "extern type Vm\nenum Vm::Fake<Int>\n1",
            "`Vm` is a handle type: only the host makes its values",
            Span::new(24, 28),
        ),
        (
            "extern type Int\n1",
            "`Int` is a built-in type",
            Span::new(12, 15),
        ),
        (
            "extern type Vm\nextern type Vm = \"other\"\n1",
            "type `Vm` is declared twice",
            Span::new(27, 29),
        ),
        (
            "extern type Vm\ntype Vm = Int\n1",
            "type `Vm` is declared twice",
            Span::new(20, 22),
        ),
        (
            "type Vm = Int\nextern type Vm\n1",
            "type `Vm` is declared twice",
            Span::new(26, 28),
        ),
        (
            "enum Vm extensible\nextern type Vm\n1",
            "type `Vm` is declared twice",
            Span::new(31, 33),
        ),
        (
            "extern type Vm\nVm::Made",
            "no variant `Made` of `Vm`",
            Span::new(19, 23),
        ),
        (
            "extern type Vm\nlet m: Vm = 3\n1",
            "mismatched types",
            Span::new(27, 28),
        ),
        (
            "extern type Vm\nextern type Disk\nextern fn boot() -> Vm\nlet d: Disk = boot()\n1",
            "mismatched types",
            Span::new(69, 75),
        ),
    ] {
        assert_eq!(errors(text), [(message.to_owned(), span)], "{text}");
    }
    // A variant declared for one is reported once, a use of it quietly.
    let found = diagnostics("extern type Vm\nenum Vm::Fake<Int>\nVm::Fake(3)");
    let [error] = &found[..] else {
        panic!("{found:?}");
    };
    assert_eq!(
        labels(error),
        [(Span::new(12, 14), "declared `extern` here")]
    );
    assert_eq!(
        error.notes,
        ["an operation of the host's gives a handle; no variant is declared for one"]
    );
}
