//! Tests of the evaluator: memoised functions, their keys and tables.

use super::*;

/// Runs `text` on a fresh stack with `memo` as its memo tables.
pub(super) fn run_with(text: &str, memo: &mut MemoTables) -> Result<Value, Diagnostic> {
    let ir = compile(text);
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    eval_with(
        &ir,
        &mut stack,
        memo,
        (None, &[], &Direct { temp_dirs: None }),
    )
}

/// The values of the fields of the record that `text` evaluates to, in
/// definition order.
pub(super) fn fields(text: &str) -> Vec<Value> {
    match run(text) {
        Ok(Value::Record(record)) => record.values.to_vec(),
        other => panic!("{text}: expected a record, got {other:?}"),
    }
}

/// Whether `a` and `b` are one record, closure or future.
pub(super) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Record(a), Value::Record(b)) => Rc::ptr_eq(a, b),
        (Value::Fn(a), Value::Fn(b)) => Rc::ptr_eq(a, b),
        (Value::Future(a), Value::Future(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

/// Whether a memoised function of a parameter of type `ty`, whose body
/// builds a fresh record, runs its body once when called with `first`
/// and then `second`, after `setup`: whether its results are one
/// record.
pub(super) fn hits(setup: &str, ty: &str, first: &str, second: &str) -> bool {
    let text = format!(
        "fn f(x: {ty}) ->> {{v: Int}} {{ v = 1 }}\n{setup}\na = f({first})\nb = f({second})"
    );
    let values = fields(&text);
    same(&values[0], &values[1])
}

#[test]
fn a_memoised_function_runs_once_per_distinct_arguments() {
    let text = "fn f(x: Int) ->> {v: Int} { v = x }\na = f(1)\nb = f(1)\nc = f(2)\nd = f(2)";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert!(same(&values[2], &values[3]));
    assert!(!same(&values[0], &values[2]));
    assert_eq!(values[0].to_string(), "{v = 1}");
    assert_eq!(values[2].to_string(), "{v = 2}");
    // An impure function runs every time.
    let values = fields(&text.replace("->>", "->"));
    assert!(!same(&values[0], &values[1]));
    // The key is the arguments in parameter order, however passed.
    let text = "fn g(x: Int, y: Int) ->> {v: Int} { v = x }\n\
                    a = g(1, 2)\nb = g(y = 2, x = 1)\nc = g(2, 1)";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert!(!same(&values[0], &values[2]));
    // No parameters: one key.
    let values = fields("fn h() ->> {v: Int} { v = 1 }\na = h()\nb = h()");
    assert!(same(&values[0], &values[1]));
}

#[test]
fn a_default_given_is_the_memo_key_an_argument_would_be() {
    // The key is the full list of arguments, a default given at the call
    // as if written: an omitted default and the same value written
    // share an entry, and another value does not.
    let text = "fn f(x: Int, y: String = \"d\", z: List<{a: Int}> = [{ a = 1 }]) ->> {v: Int} {\n\
                    \x20   v = x\n\
                    }\n\
                    a = f(1)\n\
                    b = f(1, \"d\")\n\
                    c = f(1, \"d\", [{ a = 1 }])\n\
                    d = f(1, z = [{ a = 1 }])\n\
                    e = f(1, \"e\")\n\
                    g = f(1, z = [])";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert!(same(&values[0], &values[2]));
    assert!(same(&values[0], &values[3]));
    assert!(!same(&values[0], &values[4]));
    assert!(!same(&values[0], &values[5]));
}

#[test]
fn a_default_that_is_no_constant_is_evaluated_for_each_call() {
    // Each call that leaves the parameter out evaluates it anew, with the
    // parameters before it, written or given their defaults, and what the
    // function's scope holds where the function is written.
    let text = "let base = 100\n\
                    fn f(a: Int, b: Int = a * 2, c: Int = a + b + base): List<Int> { [a, b, c] }\n\
                    x = f(1)\n\
                    y = f(2)\n\
                    z = f(1, 5)\n\
                    w = f(c = 0, a = 3)";
    let shown: Vec<String> = fields(text).iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        ["[1, 2, 103]", "[2, 4, 106]", "[1, 5, 106]", "[3, 6, 0]"]
    );
    // A nested function's default captures from its own frame, each time
    // it is made.
    let text = "fn make(n: Int): Int {\n\
                    \x20   let g = fn(m: Int = n * 10): Int { m }\n\
                    \x20   g()\n\
                    }\n\
                    a = make(1)\n\
                    b = make(2)";
    let shown: Vec<String> = fields(text).iter().map(ToString::to_string).collect();
    assert_eq!(shown, ["10", "20"]);
}

#[test]
fn what_a_default_reads_orders_the_fields_and_shadowing_keeps_it() {
    // A field the default alone reads is computed before the field that
    // calls the function, as one its body read would be.
    let shown: Vec<String> = fields("r = f(1)\nk = 3\nfn f(a: Int, b: Int = a + k): Int { b }")
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(shown, ["4", "3"]);
    // A shadowed function's default is never run, and the one that stands
    // gives its own.
    let text = "let k = 2\n\
                    #[shadowable] def greet(a: Int, b: Int = a * k): Int { b }\n\
                    def greet(a: Int, b: Int = a + k): Int { b }\n\
                    r = greet(5)";
    assert_eq!(fields(text)[1].to_string(), "7");
}

#[test]
fn a_default_that_is_no_constant_is_given_before_the_memo_key_is_made() {
    // A memoised function's default is evaluated at the call, so the key is
    // the arguments with it, as if written.
    let text = "fn f(x: Int, y: Int = x + 1) ->> {v: Int} { v = x * y }\n\
                    a = f(1)\n\
                    b = f(1, 2)\n\
                    c = f(1, 3)";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert!(!same(&values[0], &values[2]));
    assert_eq!(values[0].to_string(), "{v = 2}");
}

#[test]
fn a_failing_default_fails_the_call_at_the_default() {
    let text = "fn f(a: List<Int>, b: Int = a[5]): Int { b }\nx = f([1])";
    let error = run(text).unwrap_err();
    let at = text.find("a[5]").unwrap();
    assert_eq!(error.span.start as usize, at, "{error:?}");
}

#[test]
fn memo_keys_compare_by_key_equality() {
    let deep = |leaf: i64| format!("{}{leaf}{}", "{ a = ".repeat(6), " }".repeat(6));
    let deep_type = format!("{}Int{}", "{a: ".repeat(6), "}".repeat(6));
    let cases = [
        ("", "Int", "1", "1", true),
        ("", "Int", "1", "2", false),
        // IEEE equality: the zeros are equal, and NaN is equal to
        // nothing, itself included.
        ("", "Float", "0.0", "-0.0", true),
        ("", "Float", "1.5", "1.5", true),
        ("", "Float", "1.5", "2.5", false),
        ("", "Float", "0.0 / 0.0", "0.0 / 0.0", false),
        ("let n = 0.0 / 0.0", "Float", "n", "n", false),
        // By content, not by allocation.
        ("", "String", "\"ab\"", "\"a\" + \"b\"", true),
        ("", "String", "\"ab\"", "\"ba\"", false),
        ("", "Bool", "true", "true", true),
        ("", "Bool", "true", "false", false),
        ("", "{}", "{}", "{}", true),
        // Records by total equality: by field name, whatever the
        // order, and every field, those the static type hides included.
        (
            "",
            "{x: Int, y: Int}",
            "{ x = 1; y = 2 }",
            "{ y = 2; x = 1 }",
            true,
        ),
        (
            "",
            "{x: Int, y: Int}",
            "{ x = 1; y = 2 }",
            "{ x = 1; y = 3 }",
            false,
        ),
        ("", "{x: Int}", "{ x = 1; y = 2 }", "{ y = 2; x = 1 }", true),
        (
            "",
            "{x: Int}",
            "{ x = 1; y = 2 }",
            "{ x = 1; y = 3 }",
            false,
        ),
        ("", "{x: Int}", "{ x = 1 }", "{ x = 1; y = 2 }", false),
        (
            "",
            "{a: {b: Int}}",
            "{ a = { b = 1 } }",
            "{ a = { b = 1 } }",
            true,
        ),
        (
            "",
            "{a: {b: Int}}",
            "{ a = { b = 1 } }",
            "{ a = { b = 2 } }",
            false,
        ),
        (
            "",
            "{a: {b: Float}}",
            "{ a = { b = 0.0 } }",
            "{ a = { b = -0.0 } }",
            true,
        ),
        // Nested deeper than the hash looks.
        ("", &deep_type, &deep(1), &deep(1), true),
        ("", &deep_type, &deep(1), &deep(2), false),
        // Functions and futures by identity.
        ("let g = fn(): Int { 1 }", "fn(): Int", "g", "g", true),
        ("", "fn(): Int", "fn(): Int { 1 }", "fn(): Int { 1 }", false),
        ("let v = lazy 1", "Future<Int>", "v", "v", true),
        ("", "Future<Int>", "lazy 1", "lazy 1", false),
        (
            "let g = fn(): Int { 1 }",
            "{f: fn(): Int}",
            "{ f = g }",
            "{ f = g }",
            true,
        ),
        (
            "",
            "{f: fn(): Int}",
            "{ f = fn(): Int { 1 } }",
            "{ f = fn(): Int { 1 } }",
            false,
        ),
    ];
    for (setup, ty, first, second, expected) in cases {
        assert_eq!(
            hits(setup, ty, first, second),
            expected,
            "{ty}: {first} then {second}"
        );
    }
}

#[test]
fn a_memoised_recursion_runs_its_body_once_per_argument() {
    // Unmemoised, this would make about 10^19 calls.
    assert_eq!(
        int(
            "fn fib(n: Int) ->> Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
                 fib(90)"
        ),
        2_880_067_194_370_816_120
    );
}

#[test]
fn a_memoised_recursion_has_the_nesting_budget_of_any_other() {
    on_big_stack(|| {
        let recurse = "fn f(n: Int) ->> Int { if n == 0 { 0 } else { f(n - 1) + 1 } }\n";
        let ir = compile(&format!("{recurse}f(0)"));
        let [(outer, _)] = ir.calls_in(ir.root())[..] else {
            panic!("expected one call at the top level");
        };
        let code = ir.function(FnId::new(0));
        let [(inner, inner_span)] = ir.calls_in(code.body)[..] else {
            panic!("expected one call in the body");
        };
        let (outer, inner) = (usize::from(outer), usize::from(inner));
        let height = code.height as usize;
        let allowed = 1 + (MAX_DEPTH - height - outer - CALL_LEVELS) / (inner + CALL_LEVELS);
        // A first call with the most calls the budget allows succeeds,
        // using all of it, on the stack the pipeline is run with.
        let last = i64::try_from(allowed - 1).unwrap();
        assert_eq!(
            run(&format!("{recurse}f({last})")).unwrap(),
            Value::Int(last)
        );
        // One more fails as any recursion does, and leaves nothing
        // running.
        let text = format!("{recurse}f({})", last + 1);
        let diagnostic = run(&text).unwrap_err();
        assert_eq!(diagnostic.message, BUDGET_ERROR);
        assert_eq!(diagnostic.notes, vec![BUDGET_NOTE]);
        assert_eq!(diagnostic.span, inner_span);
        assert!(diagnostic.labels.is_empty());
        let mut memo = MemoTables::new();
        assert_eq!(
            run_with(&text, &mut memo).unwrap_err().message,
            BUDGET_ERROR
        );
        assert_eq!(memo.running_count(), 0);
        // A call that finds its result charges nothing for the body: in
        // steps, each within the budget, the recursion goes past it.
        let steps = i64::try_from(allowed / 100 + 2).unwrap();
        let warm = format!(
            "{recurse}fn warm(n: Int) -> Int {{ if n <= 0 {{ 0 }} else {{ warm(n - 100) + f(n) }} }}\n\
                 warm({})",
            100 * steps
        );
        assert_eq!(
            run(&warm).unwrap(),
            Value::Int(100 * steps * (steps + 1) / 2)
        );
    });
}

/// `STACK_SIZE` covers the nesting budget when it is spent on memoised
/// calls, whose recursion takes a frame more than any other call's,
/// unoptimised, whatever kind of nesting they are made from: for each
/// costly kind, a memoised recursion whose recursive call is wrapped in
/// that nesting makes as many calls as the budget allows, which takes
/// its deepest frame's body to exactly `MAX_DEPTH` levels, and one more
/// call fails.
#[test]
fn the_nesting_budget_fits_the_stack_for_memoised_calls() {
    on_big_stack(|| {
        for (kind, ret, wrap_in, value) in await_nesting_kinds() {
            // `f(n)` makes n + 1 calls, none finding its result.
            let program = |calls: usize, padding: usize| {
                format!(
                    "fn f(n: Int) ->> {ret} {{ if n == 0 {{ {} }} else {{ {} }} }}\n{}f({}){}",
                    wrap_in("-(-(n - 1))"),
                    wrap_in("f(n - 1)"),
                    "{ ".repeat(padding),
                    calls - 1,
                    " }".repeat(padding)
                )
            };
            let ir = compile(&program(1, 0));
            let [(outer, _)] = ir.calls_in(ir.root())[..] else {
                panic!("{kind}: expected one call at the top level");
            };
            let code = ir.function(FnId::new(0));
            assert_eq!(code.kind, FnKind::Memo, "{kind}");
            let [(inner, _)] = ir.calls_in(code.body)[..] else {
                panic!("{kind}: expected one call in the body");
            };
            let (outer, inner) = (usize::from(outer), usize::from(inner));
            assert!(inner > AWAIT_WRAP, "{kind}: {inner}");
            let height = code.height as usize;
            // The first call enters its frame at `outer + CALL_LEVELS`
            // levels, each nested one `inner + CALL_LEVELS` further in;
            // with the padding making up the remainder, the deepest
            // frame's body reaches exactly `MAX_DEPTH`.
            let step = inner + CALL_LEVELS;
            let room = MAX_DEPTH - height - outer - CALL_LEVELS;
            let (calls, padding) = (1 + room / step, room % step);
            assert!(calls > 100, "{kind}: {calls}");
            let expected = value(i64::try_from(calls).unwrap());
            assert_eq!(run(&program(calls, padding)), Ok(expected), "{kind}");
            for past in [program(calls + 1, padding), program(calls, padding + 1)] {
                let diagnostic = run(&past).unwrap_err();
                assert_eq!(diagnostic.message, BUDGET_ERROR, "{kind}");
            }
        }
    });
}

#[test]
fn a_memoised_call_depending_on_its_own_result_fails() {
    let text = "fn f(n: Int) ->> Int { if n == 0 { f(0) } else { f(n - 1) + 1 } }\nf(3)";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "this call depends on its own result");
    // At the call that found the function running with its arguments,
    // labelled at the call that is running it.
    let inner = text.find("f(0)").unwrap();
    let running = text.find("f(n - 1)").unwrap();
    let span = |start: usize, len: usize| {
        Span::new(
            u32::try_from(start).unwrap(),
            u32::try_from(start + len).unwrap(),
        )
    };
    assert_eq!(diagnostic.span, span(inner, 4));
    assert_eq!(
        labels(&diagnostic),
        vec![(span(running, 8), "called here with the same arguments")]
    );
    assert_eq!(
        diagnostic.notes,
        vec![
            "a `->>` function called again with the same arguments while it is running \
                 would never finish"
        ]
    );
    // Every running call's mark is removed as the error passes.
    let mut memo = MemoTables::new();
    run_with(text, &mut memo).unwrap_err();
    assert_eq!(memo.running_count(), 0);
    assert_eq!(memo.table_count(), 1);
    // With other arguments, the same function runs.
    assert_eq!(
        int("fn f(n: Int) ->> Int { if n == 0 { 0 } else { f(n - 1) + 1 } }\nf(3)"),
        3
    );
}

#[test]
fn a_memoised_call_of_no_arguments_depending_on_its_own_result_fails() {
    let text = "fn f() ->> Int { f() + 1 }\nf()";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "this call depends on its own result");
    let inner = text.find("f() + 1").unwrap();
    let running = text.rfind("f()").unwrap();
    let span = |start: usize| {
        Span::new(
            u32::try_from(start).unwrap(),
            u32::try_from(start + 3).unwrap(),
        )
    };
    assert_eq!(diagnostic.span, span(inner));
    assert_eq!(
        labels(&diagnostic),
        vec![(span(running), "called here with the same arguments")]
    );
    let mut memo = MemoTables::new();
    run_with(text, &mut memo).unwrap_err();
    assert_eq!(memo.running_count(), 0);
}

/// The closure of a memoised function of no parameters, for driving
/// [`MemoTables`] by hand.
pub(super) fn memoised_closure() -> Rc<Closure> {
    match run("fn h() ->> Int { 1 }\nh") {
        Ok(Value::Fn(closure)) => closure,
        other => panic!("expected a closure, got {other:?}"),
    }
}

/// Starts a call of `closure` with no arguments in `memo`, as
/// [`Evaluator::memo_begin`] does, for a caller of origin `origin`,
/// which then reads the directory `dir` as its `fileroot.caller`.
pub(super) fn start_reading(
    memo: &mut MemoTables,
    closure: &Rc<Closure>,
    origin: u64,
    dir: u32,
) -> Mark {
    let hash = key_hash(&[]);
    let entry = memo.start(closure, hash, &[]);
    memo.running.push(Running {
        closure: Rc::as_ptr(closure),
        hash,
        entry,
        origin,
        context: None,
    });
    memo.depend(origin, &Ambient::Caller(dir));
    Mark {
        hash,
        entry,
        entering: 0,
        caller: Caller {
            dir,
            helper: false,
            origin,
        },
    }
}

/// What a call of `closure` with no arguments from the directory `dir`
/// finds in `memo`: its value, `None` if the entry is running, or the
/// text `missing`.
pub(super) fn found(
    memo: &mut MemoTables,
    closure: &Rc<Closure>,
    dir: u32,
) -> Result<Option<Value>, &'static str> {
    let caller = Caller {
        dir,
        helper: false,
        origin: NO_ORIGIN,
    };
    match memo.lookup(closure, &[], (caller, false), None) {
        Lookup::Done(value, _) => Ok(Some(value)),
        Lookup::Running => Ok(None),
        Lookup::Missing(_) => Err("missing"),
    }
}

#[test]
fn entries_with_empty_keys_are_told_apart() {
    // Two calls of no arguments, from two directories, each reading
    // its caller: two entries of one bucket, both with empty keys,
    // whose boxes share an address.
    let closure = memoised_closure();
    let mut memo = MemoTables::new();
    let first = start_reading(&mut memo, &closure, 1, 1);
    memo.running.pop();
    assert_eq!(
        memo.finish(&closure, &first, Some(&Value::Int(10))),
        Some(false)
    );
    let second = start_reading(&mut memo, &closure, 2, 2);
    assert_ne!(first.entry, second.entry);
    // The second is running: found so from its directory, and marked
    // re-entered, the first untouched.
    assert_eq!(found(&mut memo, &closure, 2), Ok(None));
    assert_eq!(found(&mut memo, &closure, 1), Ok(Some(Value::Int(10))));
    memo.running.pop();
    assert_eq!(
        memo.finish(&closure, &second, Some(&Value::Int(20))),
        Some(false)
    );
    // Both completed, each with its own dependency, and both reused.
    assert_eq!(memo.running_count(), 0);
    for _ in 0..2 {
        assert_eq!(found(&mut memo, &closure, 1), Ok(Some(Value::Int(10))));
        assert_eq!(found(&mut memo, &closure, 2), Ok(Some(Value::Int(20))));
        assert_eq!(found(&mut memo, &closure, 3), Err("missing"));
    }
    assert!(memo.completed(&closure, &[], 1));
    assert!(memo.completed(&closure, &[], 2));
    assert!(!memo.completed(&closure, &[], 3));
    // A third that fails, re-entered, removes itself and nothing else.
    let third = start_reading(&mut memo, &closure, 3, 3);
    assert_eq!(found(&mut memo, &closure, 3), Ok(None));
    memo.running.pop();
    assert_eq!(memo.finish(&closure, &third, None), Some(true));
    assert_eq!(found(&mut memo, &closure, 3), Err("missing"));
    assert_eq!(found(&mut memo, &closure, 1), Ok(Some(Value::Int(10))));
    assert_eq!(found(&mut memo, &closure, 2), Ok(Some(Value::Int(20))));
    assert_eq!(memo.running_count(), 0);
}

#[test]
fn a_memoised_closure_is_memoised_however_it_is_called() {
    // Through a variable of a supertype, and passed as a callback to an
    // impure function: memoisation belongs to the closure.
    let text = "fn f(x: Int) ->> {v: Int} { v = x }\n\
                    let g: fn(Present<Int>) -> {v: Int} = f\n\
                    fn apply(h: fn(Present<Int>) -> {v: Int}, x: Int) -> {v: Int} { h(x) }\n\
                    a = f(1)\nb = g(1)\nc = apply(f, 1)\nd = apply(g, 1)";
    let values = fields(text);
    for value in &values[1..] {
        assert!(same(&values[0], value), "{values:?}");
    }
}

#[test]
fn two_closures_of_one_function_expression_have_separate_tables() {
    let text = "fn make() -> fn(Int) ->> {v: Int} { fn(x: Int) ->> {v: Int} { v = x } }\n\
                    let p = make()\nlet q = make()\n\
                    a = p(1)\nb = q(1)\nc = p(1)\nd = q(1)";
    let values = fields(text);
    assert!(same(&values[0], &values[2]));
    assert!(same(&values[1], &values[3]));
    assert!(!same(&values[0], &values[1]));
}

#[test]
fn a_memoised_future_is_one_future_run_once() {
    let text = "fn f(x: Int) ->> Int { lazy x * 2 }\na = f(1)\nb = f(1)\nc = await a";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    // Awaiting one ran the other.
    let Value::Future(future) = &values[1] else {
        panic!("expected a future");
    };
    assert!(future.is_done());
    assert_eq!(values[2], Value::Int(2));
}

/// Makes `n` short-lived memoised closures, one at a time, each called
/// once and dead once `once` returns.
pub(super) const SPIN: &str = "fn once(n: Int) -> Int { let g = fn(x: Int) ->> Int { x + n }\ng(1) }\n\
                        fn spin(n: Int) -> Int { if n == 0 { 0 } else { once(n) + spin(n - 1) } }\n";

#[test]
fn arguments_that_equal_nothing_make_no_entry() {
    // A NaN argument never finds an entry, so none is made for it: the
    // body runs each time, and the table does not grow.
    let text = "let nan = 0.0 / 0.0\n\
                    fn f(x: Float, r: {a: Float}) ->> {v: Float} { v = x }\n";
    for args in ["nan, { a = 1.0 }", "1.0, { a = nan }"] {
        let mut memo = MemoTables::new();
        let both = format!("{text}{{ a = f({args}); b = f({args}) }}");
        let Ok(Value::Record(record)) = run_with(&both, &mut memo) else {
            panic!("{both}: expected a record");
        };
        assert!(!same(&record.values[0], &record.values[1]), "{args}");
        assert_eq!(memo.table_count(), 0, "{args}");
    }
    // An error in such a call is reported as in any other.
    let diagnostic =
        run("let nan = 0.0 / 0.0\nfn d(x: Float) ->> Int { 1 // 0 }\nd(nan)").unwrap_err();
    assert_eq!(diagnostic.message, "division by zero");
}

#[test]
fn a_rollback_forgets_the_entries_made_since_its_checkpoint() {
    let text = "fn f(x: Int) ->> {v: Int} { v = x }\n";
    let mut memo = MemoTables::new();
    run_with(&format!("{text}f(1)"), &mut memo).unwrap();
    assert_eq!(memo.table_count(), 1);
    let checkpoint = memo.checkpoint();
    run_with(&format!("{text}f(1)"), &mut memo).unwrap();
    assert_eq!(memo.table_count(), 2);
    memo.rollback(checkpoint);
    assert_eq!(memo.table_count(), 1);
    memo.rollback(MemoCheckpoint(0));
    assert_eq!(memo.table_count(), 0);
}

#[test]
fn the_tables_of_dead_closures_are_swept() {
    let mut memo = MemoTables::new();
    assert_eq!(
        run_with(&format!("{SPIN}spin(100)"), &mut memo).unwrap(),
        Value::Int(100 + 5050)
    );
    // Swept each time the count reached the threshold, all dead: 100
    // tables made, 32 at a time.
    assert_eq!(memo.table_count(), 100 % MIN_SWEEP_THRESHOLD);
    // Closures alive while more are made keep their tables, and the
    // threshold grows with them.
    let live = "fn live(n: Int) -> Int {\n\
                    \x20 let g = fn(x: Int) ->> Int { x + n }\n\
                    \x20 if n == 0 { g(1) } else { g(1) + live(n - 1) }\n\
                    }\n";
    let mut memo = MemoTables::new();
    assert_eq!(
        run_with(&format!("{live}live(99)"), &mut memo).unwrap(),
        Value::Int(100 + 4950)
    );
    assert_eq!(memo.table_count(), 100);
    // Once they are dead, a later sweep drops their tables.
    run_with(&format!("{SPIN}spin(40)"), &mut memo).unwrap();
    assert!(memo.table_count() < 40, "{}", memo.table_count());
}

#[test]
fn sweeping_frees_what_a_dead_closure_held() {
    let text = "let r = { a = 1 }\n\
                    fn make(n: Int) -> Int { let g = fn(x: Int) ->> {a: Int} { r }\ng(n).a }\n\
                    let s = make(1)";
    let ir = compile(text);
    let mut frame = vec![Value::Empty; ir.frame_size() as usize];
    let mut memo = MemoTables::new();
    eval_with(
        &ir,
        &mut frame,
        &mut memo,
        (None, &[], &Direct { temp_dirs: None }),
    )
    .unwrap();
    // `make`, a top-level function, has slot 0, and `r` slot 1.
    let Value::Record(record) = frame[1].clone() else {
        panic!("expected a record");
    };
    // Held here, by the frame, by `make`'s captures and by `g`'s table,
    // as its result; `g` is dead, and what it captured is freed.
    assert_eq!(Rc::strong_count(&record), 4);
    drop(frame);
    assert_eq!(Rc::strong_count(&record), 2);
    assert_eq!(memo.table_count(), 1);
    // A sweep drops the dead closure's table, and what it held.
    run_with(&format!("{SPIN}spin(40)"), &mut memo).unwrap();
    assert_eq!(Rc::strong_count(&record), 1);
}

#[test]
fn a_sweep_during_a_memoised_call_is_harmless() {
    let mut memo = MemoTables::new();
    let text = format!(
        "{SPIN}fn outer(n: Int) ->> {{v: Int}} {{ v = spin(n) }}\na = outer(100)\nb = outer(100)"
    );
    let ir = compile(&text);
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    let Value::Record(record) = eval_with(
        &ir,
        &mut stack,
        &mut memo,
        (None, &[], &Direct { temp_dirs: None }),
    )
    .unwrap() else {
        panic!("expected a record");
    };
    // The running call's table survived the sweeps its body caused, and
    // the second call found its result.
    assert!(same(&record.values[0], &record.values[1]));
    assert_eq!(record.values[0].to_string(), "{v = 5150}");
    assert_eq!(memo.running_count(), 0);
    assert!(memo.table_count() < MIN_SWEEP_THRESHOLD);
}
