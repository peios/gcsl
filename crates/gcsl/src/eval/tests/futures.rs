//! Tests of the evaluator: `lazy`, awaits and their nesting.

use super::*;

/// The labels of a diagnostic as `(span, text)` pairs.
pub(super) fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
    diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect()
}

/// Runs `text` on a frame of its own and returns the frame afterwards,
/// holding the value of each top-level variable.
pub(super) fn frame_after(text: &str) -> Vec<Value> {
    let ir = compile(text);
    let mut frame = vec![Value::Empty; ir.frame_size() as usize];
    eval_in(&ir, &mut frame).unwrap();
    frame
}

#[test]
fn a_lazy_runs_nothing_until_awaited() {
    assert_eq!(int("let x = lazy 1 // 0\n5"), 5);
    assert_eq!(run("lazy 1 // 0").unwrap().to_string(), "Future<Int>");
    let text = "let x = lazy 1 // 0\nawait x";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "division by zero");
    // At the failing operator in the body, with "awaited here" on the
    // await that ran it.
    assert_eq!(diagnostic.span, Span::new(15, 17));
    assert_eq!(
        labels(&diagnostic),
        vec![
            (Span::new(13, 14), "1"),
            (Span::new(18, 19), "0"),
            (Span::new(20, 27), "awaited here")
        ]
    );
    assert_eq!(diagnostic.input, None);
    // Through an implicit await, on the expression awaited.
    let diagnostic = run("let x = lazy 1 // 0\nx + 1").unwrap_err();
    assert_eq!(diagnostic.span, Span::new(15, 17));
    assert_eq!(labels(&diagnostic)[2], (Span::new(20, 21), "awaited here"));
    // An await that runs another's body labels its own await too.
    let diagnostic = run("let x = lazy 1 // 0\nlet y = lazy x + 1\nawait y").unwrap_err();
    assert_eq!(
        labels(&diagnostic)[2..],
        [
            (Span::new(33, 34), "awaited here"),
            (Span::new(39, 46), "awaited here")
        ]
    );
}

#[test]
fn a_future_captures_by_value_and_runs_once() {
    assert_eq!(int("let x = 1\nlet f = lazy x\nx := 2\nawait f"), 1);
    assert_eq!(int("let x = 1\nlet f = lazy x\nlet x = 5\nf + x"), 6);
    // A future shared by a variable and a closure's capture is one
    // future: its value is the same record, built once.
    let frame = frame_after(
        "let f = lazy { a = 1 }\n\
             let g = fn() -> {a: Int} { await f }\n\
             let p = await f\n\
             let q = g()",
    );
    let [Value::Future(future), _, Value::Record(p), Value::Record(q)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(Rc::ptr_eq(p, q));
    assert!(future.is_done());
    // Awaited twice, directly and through a record field.
    assert_eq!(
        string("let f = lazy 20 + 1\nlet r = { a = f }\n\"{f} {r.a} {await f}\""),
        "21 21 21"
    );
}

#[test]
fn an_update_of_a_future_record_runs_it_once() {
    // The update awaits the future, and a later await of the same future
    // finds the record that one run built: the update shares its
    // untouched field with it.
    let frame = frame_after(
        "let r = lazy { a = 1; b = { c = 2 } }\n\
             let old = r\n\
             let r.a = 5\n\
             let s = await old",
    );
    let [
        Value::Future(future),
        Value::Future(old),
        Value::Record(updated),
        Value::Record(read),
    ] = &frame[..]
    else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(Rc::ptr_eq(future, old));
    assert!(future.is_done());
    assert_eq!(frame[2].to_string(), "{a = 5; b = {c = 2}}");
    assert_eq!(frame[3].to_string(), "{a = 1; b = {c = 2}}");
    let (Value::Record(updated), Value::Record(read)) = (&updated.values[1], &read.values[1])
    else {
        panic!("expected records");
    };
    assert!(Rc::ptr_eq(updated, read));
    // Every record along the path, by `let` and `:=`.
    assert_eq!(
        string("let r = lazy { a = lazy { b = 1; c = 2 } }\nr.a.b := 5\n\"{r}\""),
        "{a = {b = 5; c = 2}}"
    );
    assert_eq!(
        string(
            "fn f(conf: {a: {b: Int}}) -> {a: {b: Int}} { let conf.a.b = conf.a.b + 1\nconf }\n\
                 let x = f(lazy { a = lazy { b = 1 } })\n\
                 let y = f({ a = { b = 10 } })\n\
                 \"{await x} {await y}\""
        ),
        "{a = {b = 2}} {a = {b = 11}}"
    );
    // A failing future fails where the update awaits it.
    let diagnostic = run("let r = lazy { a = 1 // 0 }\nlet r.a = 2").unwrap_err();
    assert_eq!(diagnostic.message, "division by zero");
    assert_eq!(
        labels(&diagnostic).last(),
        Some(&(Span::new(32, 33), "awaited here"))
    );
}

#[test]
fn awaits_through_every_walker() {
    let text = "let i = lazy 20\n\
                    let x = lazy 1.5\n\
                    let b = lazy true\n\
                    let s = lazy \"s\"\n\
                    let r = lazy { v = 2 }\n\
                    let g = lazy fn(): Int { 3 }\n\
                    \"{i + 1} {x * 2.0} {not b} {s + \"!\"} {r.v} {g()} {await i} {i} {r}\"";
    assert_eq!(string(text), "21 3.0 false s! 2 3 20 20 {v = 2}");
    // Each typed walker, at the root.
    assert_eq!(int("let i = lazy 20\nawait i"), 20);
    assert_eq!(float("let x = lazy 1.5\nx * 2"), 3.0);
    assert!(boolean("let b = lazy false\nnot b"));
    assert_eq!(string("let s = lazy \"a\"\ns + s"), "aa");
    assert_eq!(int("let r = lazy { v = 2 }\nr.v"), 2);
    assert_eq!(int("let g = lazy fn(): Int { 3 }\ng()"), 3);
    assert_eq!(
        run("let r = lazy { v = 2 }\nawait r").unwrap().to_string(),
        "{v = 2}"
    );
    // A future of a future, awaited fully.
    assert_eq!(int("let x = lazy lazy 7\nawait x + 1"), 8);
    assert_eq!(int("let x = lazy lazy 7\nx + 1"), 8);
    // An impure function taking and returning values that may be
    // futures, and a pure one passing one through.
    assert_eq!(int("fn f(x: Int) -> Int { x * 2 }\nf(lazy 20) + f(1)"), 42);
    assert_eq!(
        int(
            "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }\n\
                 let k = keep(lazy 40)\n\
                 k.v + 2"
        ),
        42
    );
    // An `if` of futures and present values.
    assert_eq!(
        int("let x = lazy 1\nlet y = if true { x } else { 2 }\ny + 1"),
        2
    );
    assert_eq!(
        int("let x = lazy 1\nlet y = if false { x } else { 2 }\ny + 1"),
        3
    );
}

#[test]
fn a_deep_await_shares_what_it_does_not_change() {
    let frame = frame_after(
        "fn f(r: {a: Int, b: String}): {a: Int, b: String} { r }\n\
             let r = { a = lazy 1; b = \"text\" }\n\
             let s = f(r)",
    );
    let [_, Value::Record(r), Value::Record(s)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    // A new record, sharing the layout and the untouched field.
    assert!(!Rc::ptr_eq(r, s));
    assert!(Rc::ptr_eq(&r.layout, &s.layout));
    let (Value::Str(old), Value::Str(new)) = (&r.values[1], &s.values[1]) else {
        panic!("expected strings");
    };
    assert!(Rc::ptr_eq(old, new));
    assert_eq!(s.values[0], Value::Int(1));
    // The original still holds the future, now run.
    let Value::Future(future) = &r.values[0] else {
        panic!("expected a future");
    };
    assert!(future.is_done());
    // A record with nothing lazy in it is passed through as it is.
    let frame = frame_after(
        "let r: {a: Int | Future<Int>, b: {c: Int | Future<Int>}} = { a = 1; b = { c = 2 } }\n\
             let s: {a: Int, b: {c: Int}} = r",
    );
    let [Value::Record(r), Value::Record(s)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(Rc::ptr_eq(r, s));
    // Nested records are rebuilt only where they change.
    let frame = frame_after(
        "let r = { a = { b = lazy 1 }; c = { d = 2 }; e = lazy { f = lazy 3 } }\n\
             let s: {a: {b: Int}, c: {d: Int}, e: {f: Int}} = r",
    );
    assert_eq!(
        frame[1].to_string(),
        "{a = {b = 1}; c = {d = 2}; e = {f = 3}}"
    );
    // (The record block's temporaries leave values in later slots.)
    let [Value::Record(r), Value::Record(s), ..] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    let (Value::Record(old), Value::Record(new)) = (&r.values[1], &s.values[1]) else {
        panic!("expected records");
    };
    assert!(Rc::ptr_eq(old, new));
    let (Value::Record(old), Value::Record(new)) = (&r.values[0], &s.values[0]) else {
        panic!("expected records");
    };
    assert!(!Rc::ptr_eq(old, new));
    // Equality and interpolation await deeply too.
    assert!(boolean(
        "fn f(r: {a: Int}) -> Bool { r == { a = 1 } }\nlet r = { a = lazy 1 }\nawait f(r)"
    ));
    assert_eq!(
        string("let r = { a = lazy 1; b = { c = lazy \"x\" } }\n\"{r}\""),
        "{a = 1; b = {c = \"x\"}}"
    );
}

#[test]
fn a_future_displays_as_its_type() {
    let frame = frame_after("let f = lazy 1\nlet r = { a = f; b = lazy \"s\" }\nlet v = await f");
    let [Value::Future(future), record, Value::Int(1)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    // Whether it has run or not.
    assert!(future.is_done());
    assert_eq!(frame[0].to_string(), "Future<Int>");
    assert_eq!(frame[0].repr().to_string(), "Future<Int>");
    assert_eq!(frame[0].kind(), ValueKind::Deferred);
    assert_eq!(record.to_string(), "{a = Future<Int>; b = Future<String>}");
}

#[test]
fn a_failed_await_truncates_the_stack() {
    let ir = compile("let x = lazy { let a = 1\nlet b = a // 0\nb }\nlet y = await x");
    let frame = ir.frame_size() as usize;
    let mut stack = vec![Value::Empty; frame];
    let (result, len) = run_on(&ir, &mut stack);
    assert_eq!(result.unwrap_err().message, "division by zero");
    assert_eq!(len, frame);
}

#[test]
fn a_future_that_depends_on_itself_is_an_error() {
    // A future whose body stores the future, which it captured, in a
    // slot of its frame and awaits it: the checker never lets this
    // through, since no name is in scope in its own `lazy`.
    let mut builder = crate::ir::IrBuilder::with_capacity(8);
    let capture = builder.alloc(
        IrKind::LoadCapture(0, ValueKind::Deferred),
        Span::new(10, 11),
    );
    let store = builder.alloc(IrKind::Store(Slot::new(0), capture), Span::new(10, 11));
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Deferred),
        Span::new(20, 21),
    );
    let inner = builder.alloc(
        IrKind::Await {
            value: load,
            kind: ValueKind::Int,
            shape: LazyShapeId::NONE,
            depth: 0,
        },
        Span::new(14, 21),
    );
    let items = builder.push_item_list(&[store]);
    let body = builder.alloc(
        IrKind::Block {
            items,
            result: inner,
        },
        Span::new(8, 23),
    );
    let root_load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Deferred),
        Span::new(30, 31),
    );
    let root = builder.alloc(
        IrKind::Await {
            value: root_load,
            kind: ValueKind::Int,
            shape: LazyShapeId::NONE,
            depth: 0,
        },
        Span::new(24, 31),
    );
    let mut functions = vec![crate::ir::FnCode {
        body,
        kind: crate::ir::FnKind::Future,
        frame_size: 1,
        param_count: 0,
        capture_count: 1,
        height: 0,
        name: None,
        display: Box::from("Future<Int>"),
        slot_names: Box::default(),
        attributes: Box::default(),
        defers: false,
        retry: 0,
        timeout: None,
        name_text: None,
        needs: Box::default(),
    }];
    builder.measure(root, &mut functions);
    let ir = Rc::new(builder.finish_program(root, (1, Vec::new()), functions, (0, 0)));
    let future = Rc::new(Future::new(Rc::clone(&ir), FnId::new(0), Box::default()));
    // The future captures itself.
    assert!(
        future.finish(FutureState::Pending(Box::new([Value::Future(Rc::clone(
            &future
        ))])))
    );
    let mut stack = vec![Value::Future(Rc::clone(&future))];
    let diagnostic = eval_in(&ir, &mut stack).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "this deferred expression depends on its own value"
    );
    assert_eq!(diagnostic.span, Span::new(14, 21));
    assert_eq!(
        labels(&diagnostic),
        vec![(Span::new(24, 31), "awaited here")]
    );
    assert_eq!(stack.len(), 1);
    // Left unrun; the cycle is broken so that nothing leaks.
    assert!(!future.is_done());
    assert!(future.finish(FutureState::Pending(Box::default())));
}

#[test]
fn the_nesting_budget_covers_awaits() {
    on_big_stack(|| {
        // Each call awaits a future whose body makes the next call.
        let recurse = "fn f(n: Int) -> Int {\n\
                           \x20 let x = lazy { if n == 0 { 0 } else { f(n - 1) + 1 } }\n\
                           \x20 await x\n\
                           }\n";
        let ir = compile(&format!("{recurse}f(0)"));
        let [(outer, _)] = ir.calls_in(ir.root())[..] else {
            panic!("expected one call at the top level");
        };
        // The lazy is function 0, finished before `f`, function 1.
        let (future, function) = (ir.function(FnId::new(0)), ir.function(FnId::new(1)));
        let [(inner, call_span)] = ir.calls_in(future.body)[..] else {
            panic!("expected one call in the future's body");
        };
        let [(awaited, await_span)] = ir.awaits_in(function.body)[..] else {
            panic!("expected one await in the function's body");
        };
        let (future_height, function_height) = (future.height as usize, function.height as usize);
        // Walk the chain as the evaluator charges it: each call enters
        // its frame at its depth plus `CALL_LEVELS` further in, and the
        // body's await enters the future's frame the same way. `f(n)`
        // makes n + 1 calls and runs as many futures; count how many
        // pairs fit.
        let mut pairs = 0_usize;
        let mut levels = 0;
        let mut depth = usize::from(outer);
        loop {
            let entering = levels + depth + CALL_LEVELS;
            if entering + function_height > MAX_DEPTH {
                break;
            }
            let running = entering + usize::from(awaited) + CALL_LEVELS;
            if running + future_height > MAX_DEPTH {
                break;
            }
            pairs += 1;
            levels = running;
            depth = usize::from(inner);
        }
        assert!(pairs > 100, "{pairs}");
        let last = i64::try_from(pairs - 1).unwrap();
        assert_eq!(
            run(&format!("{recurse}f({last})")).unwrap(),
            Value::Int(last)
        );
        let diagnostic = run(&format!("{recurse}f({})", last + 1)).unwrap_err();
        assert_eq!(diagnostic.message, BUDGET_ERROR);
        assert_eq!(diagnostic.notes, vec![BUDGET_NOTE]);
        assert!(
            diagnostic.span == call_span || diagnostic.span == await_span,
            "{diagnostic:?}"
        );
    });
}

/// A kind of nesting to wrap a recursive call in, for
/// `the_nesting_budget_fits_the_stack_for_awaits`: its name, the return
/// type of the function whose call it wraps, how it wraps an
/// expression in 60 levels, and the value of a recursion making the
/// number of calls given.
pub(super) type AwaitNesting = (
    &'static str,
    &'static str,
    fn(&str) -> String,
    fn(i64) -> Value,
);

/// The levels of nesting each kind of `AwaitNesting` wraps a call in.
pub(super) const AWAIT_WRAP: usize = 60;

/// The costly kinds of nesting, as the end-to-end tests list them for
/// calls.
pub(super) fn await_nesting_kinds() -> [AwaitNesting; 6] {
    [
        (
            "additions",
            "Int",
            |inner| format!("{inner}{}", " + 1".repeat(AWAIT_WRAP)),
            |calls| Value::Int(calls * i64::try_from(AWAIT_WRAP).unwrap() - 1),
        ),
        (
            "negations",
            "Int",
            |inner| {
                format!(
                    "{}{inner}{}",
                    "-(".repeat(AWAIT_WRAP),
                    ")".repeat(AWAIT_WRAP)
                )
            },
            |_| Value::Int(-1),
        ),
        (
            "interpolated strings",
            "String",
            |inner| {
                format!(
                    "{}{inner}{}",
                    "\"{".repeat(AWAIT_WRAP),
                    "}\"".repeat(AWAIT_WRAP)
                )
            },
            |_| Value::Str("-1".into()),
        ),
        (
            "nested blocks",
            "Int",
            |inner| {
                format!(
                    "{}{inner}{}",
                    "{ ".repeat(AWAIT_WRAP),
                    " }".repeat(AWAIT_WRAP)
                )
            },
            |_| Value::Int(-1),
        ),
        (
            "blocks with a declaration each",
            "Int",
            |inner| {
                format!(
                    "{}{inner}{}{}",
                    "{ let v = 1\n".repeat(AWAIT_WRAP / 2),
                    " + v".repeat(AWAIT_WRAP / 2),
                    "\n}".repeat(AWAIT_WRAP / 2)
                )
            },
            |calls| Value::Int(calls * i64::try_from(AWAIT_WRAP / 2).unwrap() - 1),
        ),
        (
            "fors",
            "Int",
            // Each `for`, its body and the index reading its one value
            // back are three levels.
            |inner| {
                format!(
                    "{}{inner}{}",
                    "for v in [1] { ".repeat(AWAIT_WRAP / 3),
                    " }[0]".repeat(AWAIT_WRAP / 3)
                )
            },
            |_| Value::Int(-1),
        ),
    ]
}

/// `STACK_SIZE` covers the nesting budget when it is spent on awaits as
/// well as calls, unoptimised, whatever kind of nesting they are made
/// from: for each costly kind, a recursion through futures, each call
/// awaiting a future whose body makes the next call wrapped in that
/// nesting, runs as many calls as the budget allows, which takes the
/// deepest future's body to exactly `MAX_DEPTH` levels, and one more
/// call fails.
#[test]
fn the_nesting_budget_fits_the_stack_for_awaits() {
    on_big_stack(|| {
        let wrap = AWAIT_WRAP;
        for (kind, ret, wrap_in, value) in await_nesting_kinds() {
            // `f(n)` makes n + 1 calls, each running a future.
            let program = |calls: usize, padding: usize| {
                format!(
                    "fn f(n: Int) -> {ret} {{\n\
                         \x20 await lazy {{ if n == 0 {{ {} }} else {{ {} }} }}\n\
                         }}\n{}f({}){}",
                    wrap_in("-(-(n - 1))"),
                    wrap_in("f(n - 1)"),
                    "{ ".repeat(padding),
                    calls - 1,
                    " }".repeat(padding)
                )
            };
            // The shape of the IR does not depend on the count, and the
            // padding only moves the first call deeper.
            let ir = compile(&program(1, 0));
            let [(outer, _)] = ir.calls_in(ir.root())[..] else {
                panic!("{kind}: expected one call at the top level");
            };
            let (future, function) = (ir.function(FnId::new(0)), ir.function(FnId::new(1)));
            let [(inner, _)] = ir.calls_in(future.body)[..] else {
                panic!("{kind}: expected one call in the future's body");
            };
            let [(awaited, _)] = ir.awaits_in(function.body)[..] else {
                panic!("{kind}: expected one await in the function's body");
            };
            let (outer, inner, awaited) =
                (usize::from(outer), usize::from(inner), usize::from(awaited));
            assert!(inner > wrap, "{kind}: {inner}");
            let (future_height, function_height) =
                (future.height as usize, function.height as usize);
            // Each call enters its frame at its depth plus `CALL_LEVELS`
            // further in, and its body's await enters the future's frame
            // the same way; the deepest future's body is the tallest
            // part. With the padding making up the remainder, it
            // reaches exactly `MAX_DEPTH`.
            let first = outer + CALL_LEVELS + awaited + CALL_LEVELS + future_height;
            let step = inner + CALL_LEVELS + awaited + CALL_LEVELS;
            let room = MAX_DEPTH - first;
            let (calls, padding) = (1 + room / step, room % step);
            assert!(calls > 50, "{kind}: {calls}");
            // Every call's frame fits too.
            let mut entering = padding + outer + CALL_LEVELS;
            for _ in 0..calls {
                assert!(entering + function_height <= MAX_DEPTH, "{kind}");
                let running = entering + awaited + CALL_LEVELS;
                assert!(running + future_height <= MAX_DEPTH, "{kind}");
                entering = running + inner + CALL_LEVELS;
            }
            let expected = value(i64::try_from(calls).unwrap());
            assert_eq!(run(&program(calls, padding)), Ok(expected), "{kind}");
            // One more call, or one more level of padding, is past the
            // budget: it is used exactly.
            for past in [program(calls + 1, padding), program(calls, padding + 1)] {
                let diagnostic = run(&past).unwrap_err();
                assert_eq!(diagnostic.message, BUDGET_ERROR, "{kind}");
            }
        }
    });
}

#[test]
fn a_deep_await_of_a_deep_record() {
    on_big_stack(|| {
        // Record nesting is limited by the parser's, two levels a
        // record; the deep await descends it all, one frame a level,
        // and runs the future at the bottom.
        let depth = 4_000;
        let text = format!(
            "let r = {}lazy 1{}\n\"{{r}}\"",
            "{ a = ".repeat(depth),
            " }".repeat(depth)
        );
        let shown = string(&text);
        assert!(shown.starts_with("{a = {a = "));
        assert!(shown.ends_with(&format!("{{a = 1{}", "}".repeat(depth))));
        assert_eq!(shown.len(), depth * "{a = }".len() + 1);
    });
}

#[test]
fn mistyped_awaits_are_internal_errors() {
    // An await whose value is of another kind than it states, through
    // the general walker and a typed one.
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
    let awaited = builder.alloc(
        IrKind::Await {
            value: one,
            kind: ValueKind::Bool,
            shape: LazyShapeId::NONE,
            depth: 0,
        },
        Span::new(0, 7),
    );
    let ir = Rc::new(builder.finish(awaited, 0, Vec::new()));
    assert_eq!(
        eval(&ir).unwrap_err().message,
        "internal error: an await produced a value of another kind than its node"
    );
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
    let awaited = builder.alloc(
        IrKind::Await {
            value: one,
            kind: ValueKind::Bool,
            shape: LazyShapeId::NONE,
            depth: 0,
        },
        Span::new(0, 7),
    );
    let not = builder.alloc(IrKind::Not(awaited), Span::new(0, 11));
    let ir = Rc::new(builder.finish(not, 0, Vec::new()));
    assert_eq!(
        eval(&ir).unwrap_err().message,
        "internal error: an await produced a value of another kind than its node"
    );
    // A typed walker meeting a node that may be a future, whatever the
    // slot holds.
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Deferred),
        Span::new(0, 1),
    );
    let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
    let add = builder.alloc(
        IrKind::IntAdd(Operands {
            lhs: load,
            rhs: one,
            op_start: 2,
        }),
        Span::new(0, 5),
    );
    let ir = Rc::new(builder.finish(add, 1, Vec::new()));
    let error = eval_in(&ir, &mut vec![Value::Int(2)]).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: a node that may be a future where an Int was expected"
    );
    assert_eq!(error.span, Span::new(0, 1));
    // A deep await of a record lacking a field of its shape.
    let mut names = crate::names::Names::new();
    let (x, y) = (names.intern("x"), names.intern("y"));
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let shape = builder.push_lazy_shape(crate::ir::LazyShape {
        fields: Box::new([(y, crate::ir::LazyField::Await(LazyShapeId::NONE))]),
        rest: None,
    });
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Record),
        Span::new(0, 1),
    );
    let awaited = builder.alloc(
        IrKind::Await {
            value: load,
            kind: ValueKind::Record,
            shape,
            depth: 0,
        },
        Span::new(0, 1),
    );
    let ir = Rc::new(builder.finish(awaited, 1, Vec::new()));
    let record = Value::Record(Rc::new(Record {
        layout: Rc::new(Layout {
            names: Box::new([x]),
            texts: Box::new([Box::from("x")]),
        }),
        values: Box::new([Value::Int(1)]),
    }));
    assert_eq!(
        eval_in(&ir, &mut vec![record]).unwrap_err().message,
        "internal error: a record lacks a field of its lazy shape"
    );
    // The empty record has no fields to await.
    assert_eq!(eval_in(&ir, &mut vec![Value::Empty]).unwrap(), Value::Empty);
}
