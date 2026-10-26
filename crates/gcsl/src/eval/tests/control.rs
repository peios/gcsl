//! Tests of the evaluator: unions and narrowing, loops, stops and returns.

use super::*;

#[test]
fn values_of_unions_go_through_the_general_walker() {
    assert_eq!(
        run("let v = if 1 > 2 { 1 } else { \"one\" }\nv")
            .unwrap()
            .to_string(),
        "one"
    );
    assert_eq!(
        string("let v: Int | String = 5\n\"{v} {[v, \"x\"]}\""),
        "5 [5, \"x\"]"
    );
    booleans(&[
        ("let v: Int | String = 1\nv === v", true),
        (
            "let v: Int | String = 1\nlet w: Int | String = \"1\"\nv === w",
            false,
        ),
        (
            "let v: Int | String = 1\nlet w: Int | String = \"1\"\nv !== w",
            true,
        ),
    ]);
    // A union of records, or of lists, is read as its members agree.
    assert_eq!(
        run("let r: {a: Int, b: Int} | {a: String} = { a = \"s\" }\nr.a")
            .unwrap()
            .to_string(),
        "s"
    );
    assert_eq!(
        int("let xs: List<Int> | List<String> = [\"a\", \"b\"]\nlen(xs)"),
        2
    );
}

#[test]
fn is_tests_by_structure() {
    booleans(&[
        // Scalars, by kind.
        ("let v: Int | String = 1\nv is Int", true),
        ("let v: Int | String = \"1\"\nv is Int", false),
        ("let v: Int | Float = 1.5\nv is Float", true),
        // Records, by the fields asked for, whatever else they have.
        (
            "let r: {a: Int} | {b: Int} = { a = 1; c = 2 }\nr is {a: Int}",
            true,
        ),
        (
            "let r: {a: Int} | {b: Int} = { b = 1 }\nr is {a: Int}",
            false,
        ),
        (
            "let r: {a: Int | String} | Int = { a = \"s\" }\nr is {a: Int}",
            false,
        ),
        (
            "let r: {a: {b: Int | String}} | Int = { a = { b = 1 } }\nr is {a: {b: Int}}",
            true,
        ),
        // Lists, element by element; the empty list is every list.
        ("let xs: List<Int | String> = [1, 2]\nxs is List<Int>", true),
        (
            "let xs: List<Int | String> = [1, \"a\"]\nxs is List<Int>",
            false,
        ),
        ("let xs: List<Int | String> = []\nxs is List<String>", true),
        (
            "let xs: List<List<Int | String>> = [[1], [\"a\"]]\nxs is List<List<Int>>",
            false,
        ),
        // A union is any of its members.
        ("let v: Int | String | Bool = true\nv is Int | Bool", true),
        ("let v: Int | String | Bool = \"s\"\nv is Int | Bool", false),
    ]);
}

#[test]
fn narrowed_branches_hold_the_tested_value() {
    let text = "let v: Int | String = 41\nif v is Int { v + 1 } else { len([v]) }";
    assert_eq!(int(text), 42);
    let text = "let v: Int | String = \"s\"\nif v is Int { v + 1 } else { len([v]) }";
    assert_eq!(int(text), 1);
    // Awaited once, by the test: both branches read what it awaited.
    let frame = frame_after(
        "let f = lazy { a = 1 }\n\
             let v: {a: Int} | Int | Future<{a: Int} | Int> = f\n\
             let n = if v is Int { v } else { v.a }",
    );
    // The narrowing slot, free again after the `if`, is `n`'s.
    let [Value::Future(future), Value::Future(v), Value::Int(1)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(Rc::ptr_eq(future, v));
    assert!(future.is_done());
    // A mutable variable rebound after the test leaves the copy alone,
    // and tested under another name it is rebound from the branch.
    let text = "let mut m: Int | String = 1\n\
                    let kept = if m is Int { let f = fn(): Int { m }\nf } else { fn(): Int { 0 } }\n\
                    m := \"s\"\n\
                    kept()";
    assert_eq!(int(text), 1);
    let text = "let mut m: Int | String = 1\n\
                    let v = m\n\
                    let inner = if v is Int { m := v + 4\nv } else { 0 }\n\
                    [inner, m as Int]";
    assert_eq!(run(text).unwrap().to_string(), "[1, 5]");
    // The hidden field: a record of one member that has the field of
    // another passes the test, and is narrowed to what it passed.
    let text = "let hidden: {c: Int} = { c = 1; a = 2 }\n\
                    let x: {a: Int, b: Int} | {c: Int} = hidden\n\
                    if x is {a: Int} { x.a } else { 0 }";
    assert_eq!(int(text), 2);
}

#[test]
fn as_narrows_or_fails() {
    assert_eq!(int("let v: Int | String = 3\n(v as Int) * 2"), 6);
    assert_eq!(string("let v: Int | String = \"s\"\nv as String"), "s");
    let text = "let v: Int | String = \"s\"\nv as Int";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "`v` is not an Int");
    assert_eq!(diagnostic.span, Span::new(26, 34));
    assert_eq!(diagnostic.label.as_deref(), Some("it is a String"));
    for (text, message, label) in [
        (
            "let xs: List<Int | String> = [1, 2, 3, \"a\"]\nxs as List<Int>",
            "`xs` is not a `List<Int>`",
            "it is a list, and its element 3 is a String",
        ),
        (
            "let r: {a: Int | String} | Int = { a = \"s\" }\nr as {a: Int}",
            "`r` is not a `{a: Int}`",
            "it is a record, and its field `a` is a String",
        ),
        (
            "let r: {a: Int} | {b: Int} = { b = 1 }\nr as {a: Int}",
            "`r` is not a `{a: Int}`",
            "it is a record without a field `a`",
        ),
        (
            "fn f(): Int | Bool { true }\nf() as Int",
            "this value is not an Int",
            "it is a Bool",
        ),
        (
            "let v: Int | List<Int | String> = [\"a\"]\nv as Int | List<Int>",
            "`v` is not an `Int | List<Int>`",
            "it is a list, and its element 0 is a String",
        ),
    ] {
        let diagnostic = run(text).unwrap_err();
        assert_eq!(diagnostic.message, message, "{text}");
        assert_eq!(diagnostic.label.as_deref(), Some(label), "{text}");
    }
}

#[test]
fn a_memoised_function_takes_and_returns_a_union() {
    let text = "fn m(v: Int | String) ->> Int | String { if v is Int { v + 1 } else { v } }\n\
                    let a = m(lazy 1)\n\
                    let b = m(\"s\")\n\
                    \"{a} {b} {m(1)}\"";
    assert_eq!(string(text), "2 s 2");
}

#[test]
fn a_typed_walker_rejects_a_node_of_a_union_of_kinds() {
    let mut builder = crate::ir::IrBuilder::with_capacity(4);
    let span = Span::new(0, 1);
    let load = builder.alloc(IrKind::Load(Slot::new(0), ValueKind::Mixed), span);
    let one = builder.alloc(IrKind::IntConst(1), span);
    let add = builder.alloc(
        IrKind::IntAdd(Operands {
            lhs: load,
            rhs: one,
            op_start: 0,
        }),
        span,
    );
    let ir = Rc::new(builder.finish(add, 1, Vec::new()));
    let mut frame = vec![Value::Int(1)];
    let diagnostic = eval_in(&ir, &mut frame).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "internal error: a node of a union of kinds where one kind was expected"
    );
}

#[test]
fn the_values_of_the_loop_forms() {
    assert_eq!(int("let mut n = 0\nfor n < 3 { n := n + 1 }\nn"), 3);
    assert_eq!(
        shown("for let i = 0; i < 4; i := i + 1 { i * i }"),
        "[0, 1, 4, 9]"
    );
    assert_eq!(
        shown("let mut k = 0\nfor { k := k + 1; if k > 3 { break }; k * 10 }"),
        "[10, 20, 30]"
    );
    assert_eq!(
        shown("let mut t = 0\ndo { t := t + 1; t } for t < 3"),
        "[1, 2, 3]"
    );
    // A `do` runs once with a false condition; a condition loop not at
    // all.
    assert_eq!(shown("do { 99 } for false"), "[99]");
    assert_eq!(shown("for false { 99 }"), "[]");
    // Find-first, both ways.
    let xs = "let xs = [1, 7, 3, 9]\n";
    assert_eq!(
        shown(&format!(
            "{xs}for x in xs {{ if x > 5 {{ break x }}; continue }}"
        )),
        "[7]"
    );
    assert_eq!(
        shown(&format!("{xs}for x in xs {{ if x > 5 {{ break x }} }}")),
        "[{}, 7]"
    );
    // Values: `continue E` contributes `E` in the body's place, and
    // `break E` contributes `E` and ends the loop.
    assert_eq!(
        shown(&format!(
            "{xs}for x in xs {{ if x == 7 {{ continue 70 }}; if x == 3 {{ break 30 }}; x }}"
        )),
        "[1, 70, 30]"
    );
    assert_eq!(
        shown(&format!("{xs}for x in xs {{ if x == 3 {{ break }}; x }}")),
        "[1, 7]"
    );
}

#[test]
fn a_loop_runs_its_parts_in_order() {
    // Before-items once, then the condition, the body and the
    // after-items, again and again.
    let text = "let mut s = \"\"\n\
                    let v = for let k = ({ s := s + \"b\"; 0 }); ({ s := s + \"c\"; k < 2 }); \
                    k := ({ s := s + \"a\"; k + 1 }) { s := s + \"B\"; k }\n\
                    \"{s} {v}\"";
    assert_eq!(string(text), "bcBacBac [0, 1]");
    // `continue` skips the rest of the body but not the after-items.
    let text = "let mut s = \"\"\n\
                    let v = for let k = 0; k < 2; k := ({ s := s + \"a\"; k + 1 }) \
                    { s := s + \"B\"; continue; s := s + \"X\" }\n\
                    \"{s} {v}\"";
    assert_eq!(string(text), "BaBa []");
    // `break` skips them, and its value is collected.
    let text = "let mut s = \"\"\n\
                    let v = for let k = 0; true; k := ({ s := s + \"a\"; k + 1 }) \
                    { if k == 1 { break k * 10 }; s := s + \"B\"; k }\n\
                    \"{s} {v}\"";
    assert_eq!(string(text), "Ba [0, 10]");
    // A `do` tests its condition after each body, and `continue` goes
    // on with that test.
    let text = "let mut s = \"\"\nlet mut n = 0\n\
                    let v = do { n := n + 1; s := s + \"B\"; if n == 1 { continue }; n } \
                    for ({ s := s + \"c\"; n < 3 })\n\
                    \"{s} {v}\"";
    assert_eq!(string(text), "BcBcBc [2, 3]");
    // An iteration's guard reads what the body wrote, as before.
    assert_eq!(
        shown("let mut seen = 0\nfor x in [1, 2, 3] if seen < 2 { seen := seen + 1; x }"),
        "[1, 2]"
    );
}

#[test]
fn a_labelled_stop_drops_the_lists_of_the_loops_it_leaves() {
    let rows = "let rows = [[1, 2], [3, -1, 4], [5]]\n";
    assert_eq!(
        shown(&format!(
            "{rows}for'rows r in rows {{ for c in r {{ if c < 0 {{ continue'rows 0 }}; c }} }}"
        )),
        "[[1, 2], 0, [5]]"
    );
    assert_eq!(
        shown(&format!(
            "{rows}for'rows r in rows {{ for c in r {{ if c < 0 {{ break'rows }}; c }} }}"
        )),
        "[[1, 2]]"
    );
    // Through a condition loop and a `do`, whose items after the
    // condition do not run once the stop has left them.
    let text = "let mut s = \"\"\n\
                    let v = for'o x in [1, 2] { for let k = 0; k < 2; k := ({ s := s + \"a\"; k + 1 }) \
                    { do { if x == 2 { break'o 9 } } for false; k } }\n\
                    \"{s} {v}\"";
    assert_eq!(string(text), "aa [[0, 1], 9]");
}

#[test]
fn a_stop_passes_through_every_kind_of_node() {
    // Each evaluates what it holds up to the stop, and nothing after
    // it: `ran` counts what ran after the stopping operand, which only
    // the first iteration reaches.
    for (expr, value) in [
        (
            "{ if x == 2 { break }; x } + ({ ran := ran + 1; 0 })",
            "[1]",
        ),
        (
            "f({ if x == 2 { break }; x }, { ran := ran + 1; 0 })",
            "[1]",
        ),
        (
            "[{ if x == 2 { break }; x }, { ran := ran + 1; 0 }]",
            "[[1, 0]]",
        ),
        (
            "{ a = { if x == 2 { break }; x }; b = { ran := ran + 1; 0 } }",
            "[{a = 1; b = 0}]",
        ),
        (
            "{ let mut r = { a = 0 }; r.a := { if x == 2 { break }; x }; ran := ran + 1; r.a }",
            "[1]",
        ),
        // An update's value runs first, and its position after.
        (
            "{ let mut l = [0]; l[{ if x == 2 { break }; 0 }] := x; ran := ran + 1; l[0] }",
            "[1]",
        ),
        (
            "if { if x == 2 { break }; true } { ran := ran + 1; x } else { 0 }",
            "[1]",
        ),
        (
            "\"{ { if x == 2 { break }; x } }{ { ran := ran + 1; 0 } }\"",
            "[\"10\"]",
        ),
        (
            "if u is Int { if x == 2 { break }; ran := ran + 1; u } else { 0 }",
            "[5]",
        ),
        // A stop out of an `is`'s operand leaves before the test writes
        // its slot.
        (
            "if { if x == 2 { break }; u } is Int { ran := ran + 1; 1 } else { 0 }",
            "[1]",
        ),
        (
            "-{ if x == 2 { break }; x } * { ran := ran + 1; 1 }",
            "[-1]",
        ),
    ] {
        let text = format!(
            "fn f(a: Int, b: Int): Int {{ a + b }}\nlet u: Int | String = 5\nlet mut ran = 0\n\
                 let v = for x in [1, 2, 3] {{ {expr} }}\n\"{{v}} {{ran}}\""
        );
        assert_eq!(string(&text), format!("{value} 1"), "{expr}");
    }
}

#[test]
fn never_typed_code_runs_to_its_stop() {
    for body in [
        "{ break } + 1",
        "f({ continue })",
        "[{ break }]",
        "{ break }.a",
        "{ break }[0]",
        "{ break }(1)",
        "not { break }",
        "if { break } { 1 } else { 2 }",
        "for y in { break } { y }",
        "let r = { break }; r.a",
        "\"{ { break } }\"",
        "{ a = { break } }",
        "{ break } is Int",
        "await { break }",
        "len({ break })",
        "[...{ break }]",
        "{ ...{ break } }",
        "for ({ continue }) { 1 }",
        "for let i = ({ break }); true; i := 1 { i }",
        "let mut r = { break }; r := 1; r.a := 2; let r.b = 3; r",
        "{ break } and true",
        "x > 0 and { break }",
    ] {
        let text =
            format!("fn f(n: Int): Int {{ n }}\nlet v = for x in [1, 2] {{ {body} }}\nlen(v)");
        assert_eq!(run(&text), Ok(Value::Int(0)), "{body}");
    }
}

#[test]
fn loops_in_functions_lazies_and_memoised_functions() {
    // A loop inside a function inside a loop.
    assert_eq!(
        shown(
            "fn g(n: Int): List<Int> { for y in [n, n + 1] { if y > n { break y * 10 }; y } }\n\
                 for x in [1, 2] { g(x) }"
        ),
        "[[1, 20], [2, 30]]"
    );
    // A memoised function's and a `lazy`'s bodies loop and break.
    assert_eq!(
        shown(
            "fn m(n: Int) ->> List<Int> { for let i = 0; true; i := i + 1 { if i == n { break }; i } }\n\
                 [m(3), m(3), m(1)]"
        ),
        "[[0, 1, 2], [0, 1, 2], [0]]"
    );
    assert_eq!(
        shown("let l = lazy { for x in [1, 2, 3] { if x == 2 { continue }; x } }\nawait l"),
        "[1, 3]"
    );
    // A closure made in a loop's body captures that iteration's
    // variable, a header's included.
    assert_eq!(
        shown(
            "let fs = for let i = 0; i < 3; i := i + 1 { fn(): Int { i } }\n\
                 for f in fs { f() }"
        ),
        "[0, 1, 2]"
    );
}

#[test]
fn a_statement_loop_builds_no_list() {
    // It is `{}`, and runs for its effects.
    assert_eq!(int("let mut n = 0\nfor n < 5 { n := n + 1 }\nn"), 5);
    assert_eq!(
        int(
            "let mut n = 0\ndo { n := n + 1 } for n < 5\nfor { if n > 7 { break }; n := n + 1 }\nn"
        ),
        8
    );
    // Each statement loop lowers to a loop that discards.
    let ir = compile("let mut n = 0\nfor n < 2 { n := n + 1 }\ndo { n := n + 1 } for n < 4\nn");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    let kinds: Vec<IrKind> = ir
        .items(items)
        .iter()
        .map(|&item| ir.get(item).kind)
        .collect();
    assert!(matches!(
        kinds[..],
        [
            IrKind::Store(..),
            IrKind::Loop {
                collect: false,
                test_first: true,
                ..
            },
            IrKind::Loop {
                collect: false,
                test_first: false,
                ..
            },
        ]
    ));
}

#[test]
fn errors_inside_loops_report_as_before() {
    assert_eq!(
        error("for let i = 3; true; i := i - 1 { 10 // i }"),
        ("division by zero".to_owned(), Span::new(37, 39))
    );
    assert_eq!(error("do { [1][5] } for true").0, "index out of range");
    assert_eq!(
        error("for (1 // 0) == 0 { 1 }"),
        ("division by zero".to_owned(), Span::new(7, 9))
    );
}

#[test]
fn a_stop_leaves_the_stack_as_it_found_it() {
    // A `continue` in an argument, after the callee's frame has grown,
    // and a `break` out of a deep nest of blocks: the stack is the top
    // level's frame again.
    let ir = compile_prelude(
        "fn f(a: Int, b: Int): Int { a + b }\n\
             let v = for x in [1, 2, 3] { if x == 2 { 0 } else { f(x, { { { if x == 3 { break }; continue } } }) } }\n\
             len(v)",
    );
    let frame = ir.frame_size() as usize;
    let mut stack = vec![Value::Empty; frame];
    let (result, len) = run_on(&ir, &mut stack);
    assert_eq!(result.unwrap(), Value::Int(1));
    assert_eq!(len, frame);
}

#[test]
fn a_stop_at_a_boundary_is_an_internal_error() {
    // No checked program lets a stop out of a function, so one built by
    // hand reaching the top is an internal error, not a diagnostic.
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let span = Span::new(0, 5);
    let stop = builder.alloc(
        IrKind::Break {
            loops: 0,
            value: None,
        },
        span,
    );
    let ir = Rc::new(builder.finish(stop, 0, Vec::new()));
    let diagnostic = eval(&ir).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "internal error: a `break` or `continue` left its function"
    );
}

#[test]
fn a_return_at_the_top_is_an_internal_error() {
    // No checked program lets a `return` out of the top level either.
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let span = Span::new(0, 6);
    let stop = builder.alloc(
        IrKind::Return {
            to: CatchId::FUNCTION,
            value: None,
        },
        span,
    );
    let ir = Rc::new(builder.finish(stop, 0, Vec::new()));
    let diagnostic = eval(&ir).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "internal error: a `return` left its function"
    );
}

#[test]
fn the_values_of_every_target() {
    assert_eq!(
        shown(
            "fn sign(n: Int): Int { if n < 0 { return -1 }; if n > 0 { return 1 }; 0 }\n\
                 [sign(-5), sign(0), sign(8)]"
        ),
        "[-1, 0, 1]"
    );
    assert_eq!(
        int(
            "let cached = true\nlet old = 7\nfn compute() -> Int { 9 }\n\
                 let job = lazy { if cached { return old }; compute() }\nawait job"
        ),
        7
    );
    assert_eq!(
        int("let a = false\nlet b = true\n\
                 let y = {'pick if a { return'pick 1 }; if b { return'pick 2 }; 3 }\ny"),
        2
    );
    assert_eq!(
        shown("let xs = [1, 7, 3]\nfor'scan x in xs { if x > 5 { return'scan x } }"),
        "7"
    );
    assert_eq!(
        shown("let xs = [1, 2]\nfor'scan x in xs { if x > 5 { return'scan x } }"),
        "[{}, {}]"
    );
    assert_eq!(
        int("let mut n = 0\nfn poll(k: Int) -> Int { k - 3 }\n\
                 let v: Int = for'spin { n := n + 1; let r = poll(n); if r > 0 { return'spin r } }\nv"),
        1
    );
    // A labelled loop body: what it returns, the iteration contributes.
    assert_eq!(
        shown("for x in [1, 6, 3] {'it if x < 5 { return'it 0 }; x }"),
        "[0, 6, 0]"
    );
    // A bare `return` returns `{}`.
    assert_eq!(shown("{'b if true { return'b }; {} }"), "{}");
    assert_eq!(shown("fn f() -> {} { return }\nf()"), "{}");
    // An untyped literal adapts to the block's type.
    assert_eq!(
        float("let f: Float = {'a if true { return'a 1 }; 2 }\nf"),
        1.0
    );
}

#[test]
fn a_function_return_leaves_from_any_depth() {
    for (body, value) in [
        // Nested loops, a labelled block and a `do`.
        (
            "for x in [1, 2] { for y in [3, 4] { if y == 4 { return x * 10 + y } } }; 0",
            "14",
        ),
        ("{'b for x in [5] { {'c return x } } }; 0", "5"),
        ("do { return 6 } for true; 0", "6"),
        // An argument list, a list literal, an `if` condition.
        ("g({ return 7 }) + 1", "7"),
        ("[1, { return 8 }][0]", "8"),
        ("if { return 9 } { 1 } else { 2 }", "9"),
        // An update's value and a position of its path.
        ("let mut r = { a = 0 }; r.a := { return 10 }; r.a", "10"),
        ("let mut l = [0]; l[{ return 11 }] := 1; l[0]", "11"),
        // A narrowed branch, an interpolation.
        (
            "let u: Int | String = 12; if u is Int { return u }; 0",
            "12",
        ),
        ("len(\"{ { return 13 } }\")", "13"),
    ] {
        let text = format!("fn g(n: Int): Int {{ n }}\nfn f(): Int {{ {body} }}\nf()");
        assert_eq!(shown(&text), value, "{body}");
    }
}

#[test]
fn nothing_after_a_return_runs() {
    // `ran` counts what runs after the `return`: nothing does.
    for body in [
        "{ return 1 } + { ran := ran + 1; 0 }",
        "g({ return 1 }, { ran := ran + 1; 0 })",
        "[{ return 1 }, { ran := ran + 1; 0 }][0]",
        "for x in [1, 2] { if x == 1 { return 1 }; ran := ran + 1 }; 0",
        "let k = {'b return 1 }; ran := ran + 1; k",
    ] {
        let text = format!(
            "let mut ran = 0\nfn g(a: Int, b: Int): Int {{ a + b }}\n\
                 fn f() -> Int {{ let mut ran = 0\nlet v = {{ {body} }}\nv + ran }}\nf()"
        );
        assert_eq!(int(&text), 1, "{body}");
    }
    // A loop return drops the list and skips the after-items.
    assert_eq!(
        string(
            "let mut trace = 0\n\
                 let e = for'l let i = 0; i < 5; i := ({ trace := trace + 1; i + 1 }) { if i == 2 { return'l 99 }; i }\n\
                 \"{e} {trace}\""
        ),
        "99 2"
    );
    // A labelled block's return inside a loop body leaves the loop
    // running.
    assert_eq!(
        shown("for x in [1, 2, 3] { let k = {'b if x == 2 { return'b 20 }; x }; k + 1 }"),
        "[2, 21, 4]"
    );
}

#[test]
fn a_memoised_function_that_returns_early_is_memoised() {
    // Its body runs once: both calls give the one record it built.
    let text = "fn f(x: Int) ->> {v: Int} { if x > 0 { return { v = x } }; { v = 0 } }\n\
                    a = f(1)\nb = f(1)";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert_eq!(values[0].to_string(), "{v = 1}");
    // A `lazy` that returns early, awaited twice, runs once.
    let text = "let l = lazy { if true { return { a = 1 } }; { a = 2 } }\n\
                    p = await l\nq = await l";
    let values = fields(text);
    assert!(same(&values[0], &values[1]), "{values:?}");
    assert_eq!(values[0].to_string(), "{a = 1}");
    // In a function, the `lazy`'s return resolves the future, and
    // the function goes on.
    assert_eq!(
        int("fn f() -> Int { let j = lazy { return 1 }; await j + 1 }\nf()"),
        2
    );
}

#[test]
fn recursion_with_early_returns() {
    assert_eq!(
        int("fn fact(n: Int): Int { if n <= 1 { return 1 }; n * fact(n - 1) }\nfact(10)"),
        3_628_800
    );
    // To depth, as the pipeline is run; past the budget, its error.
    let depth = on_big_stack(|| {
        int("fn deep(n: Int): Int { if n == 0 { return 0 }; deep(n - 1) + 1 }\ndeep(1500)")
    });
    assert_eq!(depth, 1500);
    let message = on_big_stack(|| {
        error("fn deep(n: Int): Int { if n == 0 { return 0 }; deep(n - 1) + 1 }\ndeep(100000)").0
    });
    assert!(message.starts_with("call nested too deeply"), "{message}");
    // A memoised recursion through early returns.
    assert_eq!(
        int("fn fib(n: Int) ->> Int { if n < 2 { return n }; fib(n - 1) + fib(n - 2) }\nfib(60)"),
        1_548_008_755_920
    );
}

#[test]
fn a_return_leaves_the_stack_as_it_found_it() {
    for text in [
        "fn f(): Int { for x in [1] { { {'b return 3 } } } ; 0 }\nf()",
        "fn g(n: Int): Int { n }\nfn f(): Int { g(g({ return 3 })) }\nf()",
        "fn f(n: Int) ->> Int { if n > 0 { return 3 }; 0 }\nf(1) + f(1) - 3",
        "let l = lazy { for { return 3 } }\nawait l",
        "for'l x in [1, 2] {'b let k = { if x == 1 { return'b 0 }; return'l 3 }; k }",
    ] {
        let ir = compile(text);
        let frame = ir.frame_size() as usize;
        let mut stack = vec![Value::Empty; frame];
        let (result, len) = run_on(&ir, &mut stack);
        assert_eq!(result.unwrap(), Value::Int(3), "{text}");
        assert_eq!(len, frame, "{text}");
    }
}

#[test]
fn a_return_of_a_stop_is_the_stop() {
    // A `break` in the value of a `return` leaves for its loop before
    // anything is returned.
    assert_eq!(
        shown(
            "fn f() -> List<Int> { for x in [1, 2] { if x > 1 { return { break x } }; x } }\nf()"
        ),
        "[1, 2]"
    );
    // A loop that cannot end, where an `Int` is wanted, runs until a
    // stop leaves it.
    assert_eq!(
        shown("for'o x in [1, 2] { let y: Int = 1 + for { break'o x * 10 }; y }"),
        "[10]"
    );
}
