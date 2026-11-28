//! Tests of the checker: block parameters, the trailing blocks that fill
//! them and the calls of them.

use super::*;

/// Functions that take a block: `step`, an impure block of `{}`;
/// `pure_of`, a pure block; `measure`, a generic one; and `op`, an impure
/// function, and `pure_op`, a pure one, of an `Int`.
const STEPS: &str = "fn step(name: String, body: block -> {}) -> {} { body(); = {} }\n\
                     fn pure_of(n: Int, body: block: Int): Int { body() + n }\n\
                     fn measure<T>(body: block -> T) -> T { body() }\n\
                     fn op() -> Int { 1 }\n\
                     fn pure_op(): Int { 1 }\n";

/// `text` after [`STEPS`].
fn steps(text: &str) -> String {
    format!("{STEPS}{text}")
}

/// The one diagnostic of `text`.
fn only(text: &str) -> Diagnostic {
    let mut found = diagnostics(text);
    assert_eq!(found.len(), 1, "{text}: {found:?}");
    found.remove(0)
}

/// The span of the `len` bytes from byte `from` of the first occurrence of
/// `needle` in `text`.
fn part(text: &str, needle: &str, from: u32, len: u32) -> Span {
    let start = at(text, needle).start + from;
    Span::new(start, start + len)
}

/// What `step` is shown as in a label.
const STEP_LABEL: &str =
    "the function is fn(name: String | Future<String>, body: block -> {} | Future<{}>) -> {}";

#[test]
fn a_block_parameter_has_a_block_type_and_its_call_the_block_s_result() {
    // The types of the functions, as shown: a block's result is written as
    // a function's is, so bare in an impure block it is the either shape.
    assert_eq!(
        shown_result("fn f(n: Int, body: block -> Int) -> Int { body() + n }\n= f"),
        "fn(n: Int | Future<Int>, body: block -> Int | Future<Int>) -> Int"
    );
    assert_eq!(
        shown_result("fn f(n: Int, body: block: Int): Int { body() + n }\n= f"),
        "fn(n: Int, body: block: Int): Int"
    );
    assert_eq!(
        shown_result("fn f(body: block -> Future<Int>) -> Future<Int> { body() }\n= f"),
        "fn(body: block -> Future<Int>) -> Future<Int>"
    );
    assert_eq!(
        shown_result("fn f(body: block: {a: Int}): Int { body().a }\n= f"),
        "fn(body: block: {a: Int}): Int"
    );
    // A call of a function with a block is of the function's result.
    assert_eq!(shown_result(&steps("= pure_of(1) { 2 }")), "Int");
    assert_eq!(shown_result(&steps("= step(\"a\") { {} }")), "{}");
    // A call of the block is of the block's result, in its function's body.
    assert_eq!(
        shown_result(
            "fn f(body: block: Int): String { let got = body(); = \"{got + 1}\" }\n= f { 2 }"
        ),
        "String"
    );
    assert_eq!(
        messages("fn f(body: block: Int): String { body() }"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_trailing_block_is_checked_against_the_block_s_result() {
    let text = steps("let v = pure_of(1) { \"s\" }");
    let error = only(&text);
    assert_eq!(error.message, "mismatched types");
    assert_eq!(error.span, at(&text, "{ \"s\" }"));
    assert_eq!(error.label.as_deref(), Some("expected Int, found String"));
    assert_eq!(
        labels(&error),
        vec![(
            last(&text, "pure_of"),
            "the function is fn(n: Int, body: block: Int): Int"
        )]
    );
    // A block of `{}` ends as a statement list does, so a value there is
    // unused; one written as its result is the block's, of the wrong type.
    let text = steps("step(\"a\") { 1 }");
    let error = only(&text);
    assert_eq!(error.message, "unused value of type Int");
    assert_eq!(error.span, last(&text, "1"));
    let text = steps("step(\"a\") { = 1 }");
    let error = only(&text);
    assert_eq!(error.message, "mismatched types");
    assert_eq!(error.span, last(&text, "{ = 1 }"));
    assert_eq!(
        error.label.as_deref(),
        Some("expected {} | Future<{}>, found Int")
    );
    assert_eq!(labels(&error), vec![(last(&text, "step"), STEP_LABEL)]);
    // The body's type is expected: a literal takes the block's result, and
    // a record block is checked field by field.
    assert_eq!(
        shown_result("fn f(body: block: Float): Float { body() }\n= f { 1 }"),
        "Float"
    );
    assert_eq!(
        shown_result(
            "fn f(body: block: {a: Int, b: String}): Int { body().a }\n= f { a = 1; b = \"s\" }"
        ),
        "Int"
    );
    assert_eq!(
        messages("fn f(body: block: {a: Int}): Int { body().a }\nlet v = f { a = \"s\" }"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_trailing_block_is_in_its_caller_s_scope() {
    // It reads the caller's variables and rebinds its mutable ones, which
    // a function expression there could not.
    assert!(
        diagnostics_or_none(&steps(
            "let mut seen = 0\nlet word = \"w\"\nstep(word) { seen := seen + len([word]) }\n= seen"
        ))
        .is_empty()
    );
    let text = steps("let mut seen = 0\nlet f = fn() -> {} { seen := 1 }");
    assert_eq!(
        messages(&text),
        vec!["cannot rebind `seen` here: it is declared outside this function"]
    );
    // A variable that is not mutable is rebound from no inner block, a
    // trailing block included.
    let text = steps("let seen = 0\nstep(\"a\") { seen := seen + 1 }");
    assert_eq!(
        messages(&text),
        vec!["cannot rebind `seen` here: it is declared in an enclosing block"]
    );
    // What it declares is its own.
    let text = steps("step(\"a\") { let inner = 1; {} }\nlet z = inner");
    assert_eq!(messages(&text), vec!["undefined variable `inner`"]);
    // In a function, a `lazy` and a function expression, the block is in
    // the scope of each.
    for text in [
        "fn f() -> Int { let mut m = 0; step(\"a\") { m := m + 1 }; = m }",
        "let l = lazy { let mut m = 0; step(\"a\") { m := m + 1 }; m }",
        "let g = fn() -> Int { let mut m = 0; step(\"a\") { m := m + 1 }; = m }",
        "let mut m = 0\nfor i in [1, 2] { step(\"a\") { m := m + i } }",
        "let mut m = 0\nif m == 0 { step(\"a\") { m := 1 } }",
    ] {
        let found = diagnostics_or_none(&steps(text));
        assert!(found.is_empty(), "{text}: {found:?}");
    }
    // A function written in a function does not rebind the outer one's
    // variable through a block either.
    let text = steps("let mut top = 0\nlet g = fn() -> {} { step(\"a\") { top := top + 1 } }");
    assert_eq!(
        messages(&text),
        vec!["cannot rebind `top` here: it is declared outside this function"]
    );
}

#[test]
fn a_block_can_only_be_called() {
    let note = "a block is no value: call it, `body()`, or pass it on by calling it in another \
                call's block, `f(x) { body() }`";
    for (body, needle) in [
        ("let b = body; = 1", "= body"),
        ("body", "{ body"),
        ("body.x", "{ body"),
        ("\"{body}\"", "{body"),
        ("if body is Int { 1 } else { 2 }", "if body"),
        ("[body]; 1", "[body"),
        ("let r = { held = body }; = 1", "= body"),
        ("pure_op(body)", "(body"),
    ] {
        let text = steps(&format!("fn f(body: block -> Int) -> Int {{ {body} }}"));
        let found = diagnostics(&text);
        let error = found
            .iter()
            .find(|error| error.message == "a block can only be called")
            .unwrap_or_else(|| panic!("{body}: {found:?}"));
        let read = last(&text, needle);
        assert_eq!(error.span, Span::new(read.end - 4, read.end), "{body}");
        assert_eq!(
            labels(error),
            vec![(at(&text, "body: block -> Int"), "`body` is a block")],
            "{body}"
        );
        assert_eq!(error.notes, [note], "{body}");
    }
    // Passed as an argument, to a block parameter too.
    let text = steps("fn f(body: block -> {}) -> {} { step(\"a\", body) }");
    let found = diagnostics(&text);
    let shown: Vec<&str> = found.iter().map(|error| error.message.as_str()).collect();
    assert_eq!(
        shown,
        [
            "a block is given after the call's arguments",
            "a block can only be called"
        ]
    );
    // The call of a call of a block is of what the block gives.
    assert_eq!(
        messages("fn f(body: block -> Int) -> Int { body()() }"),
        vec!["cannot call a value of type Int"]
    );
    assert!(
        diagnostics_or_none(
            "fn f(body: block: fn(Int): Int): Int { body()(1) }\n= f { fn(n: Int): Int { n } }"
        )
        .is_empty()
    );
}

#[test]
fn a_block_is_not_used_inside_a_function_or_a_lazy() {
    let text = "fn f(body: block -> Int) -> Int { let g = fn() -> Int { body() }; = g() }";
    let error = only(text);
    assert_eq!(error.message, "a block cannot be used inside a function");
    assert_eq!(error.span, last(text, "body"));
    assert_eq!(
        labels(&error),
        vec![
            (part(text, "fn()", 0, 2), "inside a function here"),
            (at(text, "body: block -> Int"), "`body` is a block"),
        ]
    );
    assert_eq!(
        error.notes,
        [
            "`body` runs in the scope of the call that gave it, which may have ended when the \
             body of a function is called"
        ]
    );
    let text = "fn f(body: block -> Int) -> Int { let g = lazy body(); = await g }";
    let error = only(text);
    assert_eq!(error.message, "a block cannot be used inside a `lazy`");
    assert_eq!(error.span, last(text, "body"));
    assert_eq!(
        labels(&error),
        vec![
            (at(text, "lazy"), "inside a `lazy` here"),
            (at(text, "body: block -> Int"), "`body` is a block"),
        ]
    );
    assert_eq!(
        error.notes,
        [
            "`body` runs in the scope of the call that gave it, which may have ended when the \
             body of a `lazy` is awaited"
        ]
    );
    // A named function in the body, and a block given inside a `lazy`.
    assert_eq!(
        messages("fn outer(body: block -> {}) -> {} { fn inner() -> {} { body() }; inner() }"),
        vec!["a block cannot be used inside a function"]
    );
    assert_eq!(
        messages(&steps(
            "fn outer(body: block -> {}) -> {} { let g = lazy { step(\"x\") { body() } }; await g }"
        )),
        vec!["a block cannot be used inside a `lazy`"]
    );
    // Any read from one, called or not.
    assert_eq!(
        messages(
            "fn f(body: block -> Int) -> Int { let g = fn() -> Int { let b = body; = 1 }; = g() }"
        ),
        vec!["a block cannot be used inside a function"]
    );
}

#[test]
fn a_block_takes_no_arguments() {
    let text = "fn f(body: block -> Int) -> Int { body(1, 2) }";
    let error = only(text);
    assert_eq!(error.message, "a block takes no arguments");
    assert_eq!(error.span, at(text, "1, 2"));
    assert_eq!(
        labels(&error),
        vec![(last(text, "body"), "this is a block")]
    );
    assert_eq!(error.notes, ["call it with empty parentheses"]);
    // A named argument and a trailing block are arguments.
    let text = "fn f(body: block -> Int) -> Int { body(x = 1) }";
    let error = only(text);
    assert_eq!(error.message, "a block takes no arguments");
    assert_eq!(error.span, at(text, "x = 1"));
    let text = "fn f(body: block -> Int) -> Int { body { 1 } }";
    let error = only(text);
    assert_eq!(error.message, "a block takes no arguments");
    assert_eq!(error.span, at(text, "{ 1 }"));
    // Its arguments are checked all the same.
    assert_eq!(
        messages("fn f(body: block -> Int) -> Int { body(missing) }"),
        vec!["a block takes no arguments", "undefined variable `missing`"]
    );
}

#[test]
fn a_block_is_given_as_the_trailing_block_and_only_so() {
    let note = "write it as the call's trailing block, as in `f(x) { ... }`";
    let text = steps("step(\"a\", { {} })");
    let error = only(&text);
    assert_eq!(error.message, "a block is given after the call's arguments");
    assert_eq!(error.span, at(&text, "{ {} }"));
    assert_eq!(labels(&error), vec![(last(&text, "step"), STEP_LABEL)]);
    assert_eq!(error.notes, [note]);
    // By name.
    let text = steps("step(\"a\", body = { {} })");
    let error = only(&text);
    assert_eq!(error.message, "a block is given after the call's arguments");
    assert_eq!(error.span, at(&text, "body = { {} }"));
    assert_eq!(error.notes, [note]);
    // Left out.
    let text = steps("step(\"a\")");
    let error = only(&text);
    assert_eq!(error.message, "missing the block `body`");
    assert_eq!(error.span, last(&text, ")"));
    assert_eq!(labels(&error), vec![(last(&text, "step"), STEP_LABEL)]);
    assert_eq!(
        error.notes,
        ["give it after the call's arguments, as in `f(x) { ... }`"]
    );
    // Of a function type, whose parameters have no names.
    let text = "fn run(g: fn(String, block -> {}) -> {}) -> {} { g(\"x\") }";
    assert_eq!(only(text).message, "missing the block");
    // The block given and another argument left out is the argument's
    // error.
    assert_eq!(
        messages(&steps("step { {} }")),
        vec!["missing argument `name`"]
    );
    // One argument too many, the block given.
    let text = steps("step(\"a\", 2) { {} }");
    assert_eq!(
        messages(&text),
        vec![
            "expected 2 arguments, found 3",
            "a block is given after the call's arguments"
        ]
    );
    // A function that takes no block takes a trailing block as a value.
    assert_eq!(
        shown_result("fn f(r: {a: Int}): Int { r.a }\n= f { a = 1 }"),
        "Int"
    );
}

#[test]
fn a_block_after_a_rest_parameter_is_given_by_the_trailing_block() {
    // The rest parameter takes every positional argument, however many,
    // and the trailing block is still the block.
    let rest = "fn rest(first: Int, ...more: Int, body: block: Int): Int { body() + len(more) }\n";
    for call in [
        "rest(1) { 2 }",
        "rest(1, 2) { 2 }",
        "rest(1, 2, 3, 4) { 2 }",
        "rest(1, ...[2, 3]) { 2 }",
        "rest(first = 1) { 2 }",
    ] {
        assert_eq!(shown_result(&format!("{rest}= {call}")), "Int", "{call}");
    }
    assert_eq!(
        messages(&format!("{rest}let v = rest(1, 2, 3)")),
        vec!["missing the block `body`"]
    );
    assert_eq!(
        messages(&format!("{rest}let v = rest(1, 2, 3) {{ \"s\" }}")),
        vec!["mismatched types"]
    );
    // A generic function's type parameter binds from the block there too,
    // and a method's.
    assert_eq!(
        shown_result(
            "fn rest<T>(...more: Int, body: block: T): T { body() }\n= rest(1, 2) { \"s\" }"
        ),
        "String"
    );
    assert_eq!(
        shown_result(
            "fn (n: Int) rest<T>(...more: Int, body: block: T): T { body() }\n= 3.rest(1, 2) { 2.5 }"
        ),
        "Float"
    );
}

#[test]
fn a_function_with_a_block_is_pure_only_if_its_block_is() {
    let text = "fn f(body: block -> Int): Int { 1 }";
    let error = only(text);
    assert_eq!(error.message, "a pure function cannot take an impure block");
    assert_eq!(error.span, at(text, "block -> Int"));
    assert!(error.labels.is_empty());
    assert_eq!(
        error.notes,
        [
            "a pure function, declared with `:`, may call only a pure block: declare the block \
             `block: T`, or the function with `->`"
        ]
    );
    // Its call is an error of its own.
    let text = "fn f(body: block -> Int): Int { body() }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 2, "{found:?}");
    let error = &found[1];
    assert_eq!(
        error.message,
        "cannot call an impure block from a pure function"
    );
    assert_eq!(error.span, last(text, "body()"));
    assert_eq!(
        labels(error),
        vec![
            (
                last(text, "body"),
                "this is an impure block, declared `block -> T`"
            ),
            (
                part(text, "): Int", 1, 1),
                "this function is pure, declared with `:`"
            ),
        ]
    );
    assert_eq!(
        error.notes,
        ["a pure function, declared with `:`, may call only a pure block, declared `block: T`"]
    );
    // An impure function takes either, and calls both.
    assert!(diagnostics_or_none("fn f(body: block: Int) -> Int { body() }").is_empty());
    assert!(
        diagnostics_or_none(&steps("fn f(body: block: Int) -> Int { op() + body() }")).is_empty()
    );
    // A pure function calls a function with a pure block, and no function
    // with an impure one.
    assert!(diagnostics_or_none(&steps("fn f(): Int { pure_of(1) { 2 } }")).is_empty());
    assert_eq!(
        messages(&steps("fn f(): Int { measure { 2 } }")),
        vec!["cannot call an impure function from a pure function"]
    );
}

#[test]
fn the_body_of_a_pure_block_is_pure_code_whatever_its_caller_is() {
    let text = steps("let a = pure_of(1) { op() }");
    let error = only(&text);
    assert_eq!(
        error.message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(error.span, last(&text, "op()"));
    assert_eq!(
        labels(&error),
        vec![
            (last(&text, "op"), "this is fn() -> Int"),
            (
                last(&text, "pure_of"),
                "this call's block is pure, declared `block: T`"
            ),
        ]
    );
    // An await in it.
    let text = steps("let a = pure_of(1) { await lazy 1 }");
    let error = only(&text);
    assert_eq!(error.message, "cannot await in a pure function");
    assert_eq!(error.span, at(&text, "await"));
    assert_eq!(
        labels(&error),
        vec![(
            last(&text, "pure_of"),
            "this call's block is pure, declared `block: T`"
        )]
    );
    // In an impure function too, and deep in the block.
    assert_eq!(
        messages(&steps(
            "fn f() -> Int { pure_of(1) { if true { op() } else { 2 } } }"
        )),
        vec!["cannot call an impure function from a pure function"]
    );
    // A call of an impure block from it.
    assert_eq!(
        messages(&steps(
            "fn f(body: block -> Int) -> Int { pure_of(1) { body() } }"
        )),
        vec!["cannot call an impure block from a pure function"]
    );
    // Pure code in it is fine: a pure function, a pure block passed on, and
    // a function written in it, which is checked as its own marker says.
    for text in [
        "let a = pure_of(1) { pure_op() + pure_of(2) { 3 } }",
        "fn f(body: block: Int) -> Int { pure_of(1) { body() } }",
        "let a = pure_of(1) { let g = fn() -> Int { op() }; = 2 }",
    ] {
        let found = diagnostics_or_none(&steps(text));
        assert!(found.is_empty(), "{text}: {found:?}");
    }
    // An impure block's body is impure code where its caller is impure,
    // and the call's error where its caller is pure.
    assert!(diagnostics_or_none(&steps("let v = measure { op() }")).is_empty());
    let text = steps("fn f(): Int { measure { op() } }");
    assert_eq!(
        messages(&text),
        vec![
            "cannot call an impure function from a pure function",
            "cannot call an impure function from a pure function"
        ]
    );
}

#[test]
fn no_jump_leaves_a_block() {
    let note = "the block runs when the function it is given to calls it, which the function \
                goes on from; end the block with a value the function can act on instead";
    for (text, keyword, jump) in [
        (
            "fn g() -> Int { step(\"a\") { return 1 }; = 2 }",
            "return",
            "return",
        ),
        (
            "let xs = for i in [1] { step(\"a\") { break } }",
            "break",
            "break",
        ),
        (
            "let xs = for i in [1] { step(\"a\") { continue } }",
            "continue",
            "continue",
        ),
        (
            "let xs = for'outer i in [1] { step(\"a\") { for j in [1] { break'outer } } }",
            "break",
            "break'outer",
        ),
        (
            "let v = {'out step(\"a\") { return'out 1 }; 2 }",
            "return",
            "return'out",
        ),
    ] {
        let text = steps(text);
        let error = only(&text);
        assert_eq!(
            error.message,
            format!("`{keyword}` cannot leave a block"),
            "{text}"
        );
        assert_eq!(error.span, last(&text, jump), "{text}");
        assert_eq!(
            labels(&error),
            vec![(last(&text, "step"), "inside the block given to this call")],
            "{text}"
        );
        assert_eq!(error.notes, [note], "{text}");
    }
    // The innermost block's call is the one named.
    let text = steps("fn g() -> Int { step(\"a\") { let v = pure_of(1) { return 1 }; {} }; = 2 }");
    let error = only(&text);
    assert_eq!(error.message, "`return` cannot leave a block");
    assert_eq!(
        labels(&error),
        vec![(
            last(&text, "pure_of"),
            "inside the block given to this call"
        )]
    );
    // At the top level there is no function to return from either.
    assert_eq!(
        messages(&steps("step(\"a\") { return }")),
        vec!["`return` outside a function"]
    );
    // A block's own loops and labelled blocks are left as any are, and so
    // is a function written in it.
    for text in [
        "step(\"a\") { for i in [1] { break }; {} }",
        "step(\"a\") { for i in [1] { if i > 0 { continue } }; {} }",
        "step(\"a\") { let v = {'in return'in 1 }; {} }",
        "step(\"a\") { for'own i in [1] { for j in [2] { break'own } }; {} }",
        "step(\"a\") { let f = fn() -> Int { return 1 }; {} }",
        "let v = measure { for i in [1, 2] { if i > 1 { break i } } }",
    ] {
        let found = diagnostics_or_none(&steps(text));
        assert!(found.is_empty(), "{text}: {found:?}");
    }
}

#[test]
fn a_block_of_the_empty_record_may_end_with_an_impure_call() {
    // The call's value is discarded, as a call statement's is.
    for text in [
        "step(\"a\") { op() }",
        "step(\"a\") { let x = 1; op() }",
        "step(\"a\") { measure { op() } }",
        "fn (n: Int) times(body: block -> {}) -> {} { body() }\n3.times { op() }",
    ] {
        let (_, _, output) = check_prelude(&steps(text));
        let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        assert!(output.warnings.is_empty(), "{text}: {:?}", output.warnings);
    }
    // The call of the function is still `{}`.
    assert_eq!(shown_result(&steps("= step(\"a\") { op() }")), "{}");
    // Its last expression is a statement like the others: a pure call's
    // value and any other expression's are unused values, and a bare
    // `await` is run and discarded.
    for text in [
        "step(\"a\") { pure_op() }",
        "step(\"a\") { 1 }",
        "step(\"a\") { op() + 1 }",
    ] {
        assert_eq!(
            messages(&steps(text)),
            vec!["unused value of type Int"],
            "{text}"
        );
    }
    assert!(diagnostics_or_none(&steps("step(\"a\") { await lazy op() }")).is_empty());
    assert_eq!(
        messages(&steps("step(\"a\") { if true { op() } }")),
        vec!["`if` without `else` must be `{}`"]
    );
    // A value that holds nothing but `{}`, as a loop's list does, is a
    // statement too, whatever its loop.
    for text in [
        "let mut n = 0\nstep(\"a\") { for i in [1, 2] { n := n + i } }",
        "let mut n = 0\nstep(\"a\") { for n < 3 { n := n + 1 } }",
        "step(\"a\") { for i in [1, 2] { op(); {} } }",
    ] {
        let found = diagnostics_or_none(&steps(text));
        assert!(found.is_empty(), "{text}: {found:?}");
    }
    // A loop that collects values is an unused value there, as it is in
    // any statement.
    for text in [
        "step(\"a\") { for i in [1, 2] { i } }",
        "step(\"a\") { for i in [1, 2] { op() } }",
    ] {
        assert_eq!(
            messages(&steps(text)),
            vec!["unused value of type `List<Int>`"],
            "{text}"
        );
    }
    // Only where the block's result is `{}`: elsewhere the call is the
    // block's value.
    assert_eq!(shown_result(&steps("= measure { op() }")), "Int");
    assert_eq!(
        messages(&steps(
            "fn f(body: block -> String) -> String { body() }\nlet v = f { op() }"
        )),
        vec!["mismatched types"]
    );
    // A call that gives `{}` is the block's value as it is.
    assert!(
        diagnostics_or_none(&steps(
            "fn unit() -> {} { = {} }\nstep(\"a\") { unit() }\nstep(\"b\") { step(\"c\") { {} } }"
        ))
        .is_empty()
    );
    // In a pure block nothing is discarded: the impure call is an error,
    // and its value unused.
    assert_eq!(
        messages("fn f(body: block: {}): {} { body() }\nfn op() -> Int { 1 }\nf { op() }"),
        vec![
            "cannot call an impure function from a pure function",
            "unused value of type Int"
        ]
    );
}

#[test]
fn what_lowering_is_told_of_blocks() {
    let text = steps(
        "fn pass(body: block -> {}) -> {} { step(\"x\") { body() } }\n\
         step(\"a\") { op() }\n\
         step(\"b\") { {} }\n\
         let v = pure_of(1) { 2 }\n\
         let r = pure_op()",
    );
    let (ast, _, output) = check_prelude(&text);
    let resolution = output.unwrap().checked.resolution;
    let mut block_args = Vec::new();
    let mut block_calls = Vec::new();
    for id in ast.expr_ids() {
        let span = ast.get(id).span;
        let written = &text[span.start as usize..span.end as usize];
        if let Some(discards) = resolution.is_block_arg(id) {
            block_args.push((written, discards));
        }
        if resolution.is_block_call(id) {
            block_calls.push(written);
        }
    }
    block_args.sort_unstable();
    // Each trailing block that fills a block parameter, with whether its
    // last expression is a statement.
    assert_eq!(
        block_args,
        [
            ("{ 2 }", false),
            ("{ body() }", true),
            ("{ op() }", true),
            ("{ {} }", true),
        ]
    );
    // Each call of a block parameter, and no other call.
    assert_eq!(block_calls, ["body()", "body()", "body()", "body()"]);
}

#[test]
fn a_generic_function_binds_a_type_parameter_from_its_block() {
    assert_eq!(shown_result(&steps("= measure { 2 }")), "Int");
    assert_eq!(shown_result(&steps("= measure { \"s\" }")), "String");
    assert_eq!(
        shown_result(&steps("= measure { { a = 1; b = [2.5] } }")),
        "{a: Int, b: List<Float>}"
    );
    // A pure block binds as an impure one does.
    assert_eq!(
        shown_result("fn id<T>(body: block: T): T { body() }\n= id { [1] }"),
        "List<Int>"
    );
    // With the other arguments.
    let two = "fn two<A, B>(a: A, body: block -> B) -> {a: A, b: B} { { a = a; b = body() } }\n";
    assert_eq!(
        shown_result(&format!("{two}= two(1) {{ \"s\" }}")),
        "{a: Int, b: String}"
    );
    // A type parameter an argument and the block share is one type.
    let same = "fn same<T>(a: T, body: block: T): List<T> { [a, body()] }\n";
    assert_eq!(
        shown_result(&format!("{same}= same(\"a\") {{ \"b\" }}")),
        "List<String>"
    );
    assert_eq!(
        shown_result(&format!("{same}= same(1.5) {{ 2 }}")),
        "List<Float>"
    );
    assert_eq!(
        shown_result("fn id<T>(body: block: T): T { body() }\n= id { 2 }"),
        "Int"
    );
    // Two types for it are its union, with the warning any call gets.
    let text = format!("{same}let v = same(\"a\") {{ 2 }}");
    let found = warnings(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "the arguments for `T` have different types"
    );
    assert_eq!(found[0].span, at(&text, "{ 2 }"));
    // An expected type does not change what the block binds.
    assert_eq!(
        messages(&steps("let v: Int = measure { 2.5 }")),
        vec!["mismatched types"]
    );
    // The body is checked once, as the block it is: its jumps and its
    // purity are the block's.
    assert_eq!(
        messages(&steps(
            "fn g() -> Int { measure { if true { return 1 }; 2 } }"
        )),
        vec!["`return` cannot leave a block"]
    );
    assert_eq!(
        messages(
            "fn id<T>(body: block: T): T { body() }\nfn op() -> Int { 1 }\nlet v = id { op() }"
        ),
        vec!["cannot call an impure function from a pure function"]
    );
    // A block left out binds nothing.
    assert_eq!(
        messages(&steps("let v = measure()")),
        vec!["cannot infer `T` for this call of `measure`"]
    );
}

#[test]
fn a_block_goes_through_function_values_and_methods() {
    // A function expression's block, called through its variable.
    assert_eq!(
        shown_result("let f = fn(body: block: Int): Int { body() }\n= f { 2 }"),
        "Int"
    );
    // A function type with a block: a function of that type fills it, and
    // a call through it gives the block.
    assert!(
        diagnostics_or_none(&steps(
            "fn each(run: fn(String, block -> {}) -> {}) -> {} { run(\"x\") { op() } }\n\
             each(step)"
        ))
        .is_empty()
    );
    // A block type is a subtype only of itself: the effect and the result
    // must be the same.
    let text = "fn pure_f(body: block: Int): Int { body() }\n\
                let k: fn(block -> Int) -> Int = pure_f";
    let error = only(text);
    assert_eq!(error.message, "mismatched types");
    assert_eq!(error.span, last(text, "pure_f"));
    assert_eq!(
        error.label.as_deref(),
        Some(
            "expected fn(block -> Int | Future<Int>) -> Int | Future<Int>, found fn(body: \
             block: Int): Int"
        )
    );
    assert_eq!(
        messages("fn f(body: block: Int): Int { body() }\nlet k: fn(body: block: String): Int = f"),
        vec!["mismatched types"]
    );
    assert_eq!(
        shown_result(
            "fn f(body: block: Int): Int { body() }\nlet k: fn(body: block: Int): Int = f\n= k { 1 }"
        ),
        "Int"
    );
    assert_eq!(
        shown_result(
            "fn f(body: block -> Int) -> Int { body() }\nlet k: fn(block -> Int) -> Int = f\n= k { 1 }"
        ),
        "Int | Future<Int>"
    );
    // In a function type `block: Int` alone is a parameter named `block`,
    // as `name: Type` is any parameter's: a pure block there has a name.
    assert_eq!(
        messages("fn f(body: block: Int): Int { body() }\nlet k: fn(block: Int): Int = f"),
        vec!["mismatched types"]
    );
    assert_eq!(
        shown_result("fn f(block: Int): Int { block }\nlet k: fn(block: Int): Int = f\n= k(1)"),
        "Int"
    );
    // A method's block, with and without parentheses.
    let times = "fn (n: Int) times(body: block: Int): Int { body() * n }\n";
    assert_eq!(shown_result(&format!("{times}= 3.times {{ 2 }}")), "Int");
    assert_eq!(shown_result(&format!("{times}= 3.times() {{ 2 }}")), "Int");
    assert_eq!(
        messages(&format!("{times}let v = 3.times()")),
        vec!["missing the block `body`"]
    );
    assert_eq!(
        messages(&format!("{times}let v = 3.times {{ \"s\" }}")),
        vec!["mismatched types"]
    );
    // A function with a default before its block.
    let dflt = "fn dflt(n: Int = 2, body: block: Int): Int { body() + n }\n";
    assert_eq!(shown_result(&format!("{dflt}= dflt {{ 1 }}")), "Int");
    assert_eq!(shown_result(&format!("{dflt}= dflt(n = 3) {{ 1 }}")), "Int");
}

#[test]
fn blocks_nest_and_are_passed_on_by_calling_them() {
    for text in [
        // A block in a block in a block.
        "step(\"a\") { step(\"b\") { step(\"c\") { op() } } }",
        // Passed on, in another call's block.
        "fn pass(body: block -> {}) -> {} { step(\"x\") { body() } }\npass { op() }",
        // Called twice, and in a loop and a branch.
        "fn many(n: Int, body: block -> {}) -> {} { for i in 0..n { body() }; if n > 0 { body(); {} } }",
        // Through recursion.
        "fn count(k: Int, body: block -> {}) -> {} { if k > 0 { body(); count(k - 1) { body() }; {} } }",
        // An inner block rebinds what the outer block declares.
        "step(\"a\") { let mut n = 0; step(\"b\") { n := n + 1 }; {} }",
        // A call of a block is an operand like any other.
        "fn sum(body: block: Int): Int { body() + body() * 2 }",
    ] {
        let found = diagnostics_or_none(&steps(text));
        assert!(found.is_empty(), "{text}: {found:?}");
    }
    // A block parameter's name is a binding like any other: a `let` of
    // that name shadows it.
    assert!(diagnostics_or_none("fn f(body: block: Int): Int { let body = 1; = body }").is_empty());
    // And a rebind of it gives it no value, since none is a block.
    assert_eq!(
        messages("fn f(body: block: Int): Int { body := 1; 2 }"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_block_s_result_is_written_as_a_function_s_is() {
    // Bare in an impure block it is the either shape, which a future
    // fills; in a pure block it is present.
    assert!(
        diagnostics_or_none("fn f(body: block -> Int) -> Int { await body() }\n= f { lazy 1 }")
            .is_empty()
    );
    assert_eq!(
        messages("fn f(body: block: Int): Int { body() }\nlet v = f { lazy 1 }"),
        vec!["cannot use a value that may be a future in a pure function"]
    );
    // An unknown type in it is reported once, and no call of the block
    // after it.
    assert_eq!(
        messages("fn f(body: block -> Missing) -> Int { body(); = 1 }"),
        vec!["unknown type `Missing`"]
    );
    // `block` with no marker is a type's name.
    assert_eq!(
        messages("fn f(body: block): Int { 1 }"),
        vec!["unknown type `block`"]
    );
}

#[test]
fn a_deferring_function_and_a_def_type_take_no_block() {
    let text = "#[attribute] #[defers] type hermetic = {on: Bool = true}\n\
                #[hermetic]\n\
                fn f(body: block -> Int) -> Int { body() }";
    let error = only(text);
    assert_eq!(error.message, "a deferring function cannot take a block");
    assert_eq!(error.span, at(text, "block -> Int"));
    assert!(error.labels.is_empty());
    assert_eq!(
        error.notes,
        [
            "a host may run its calls later, as steps, when the scope its block runs in has \
             ended"
        ]
    );
    // An attribute that does not defer is on a function with a block as
    // on any.
    assert!(
        diagnostics_or_none(
            "#[attribute] type hermetic = {on: Bool = true}\n\
             #[hermetic]\n\
             fn f(body: block -> Int) -> Int { body() }"
        )
        .is_empty()
    );
    let text = "#[def_type]\n\
                def t(name: String, func: fn() -> Int, body: block -> Int) -> Int { body() }";
    let error = only(text);
    assert_eq!(error.message, "a def-type cannot take a block");
    assert_eq!(error.span, at(text, "block -> Int"));
    assert!(error.labels.is_empty());
    assert_eq!(
        error.notes,
        [
            "a def-type is invoked by a host, which gives its arguments by name; take the block \
             in a function its definitions call"
        ]
    );
    // A def-type's definition gives a block to what it calls.
    assert!(
        diagnostics_or_none(&steps(
            "#[def_type]\n\
             def test(name: String, func: fn() -> Int) -> Int { func() }\n\
             def test one() { let mut n = 0; step(\"a\") { n := n + 1 }; = n }"
        ))
        .is_empty()
    );
}
