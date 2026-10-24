//! Tests of the evaluator: calls, closures and the nesting budget.

use super::*;

/// Runs `f` on a thread with [`crate::STACK_SIZE`] of stack, as the
/// pipeline is meant to be run. Test threads are far smaller by default.
pub(super) fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn calls_of_every_kind_of_result() {
    // Through each typed walker.
    assert_eq!(int("fn f(): Int { 41 }\nf() + 1"), 42);
    assert_eq!(float("fn f(x: Float): Float { x / 2 }\nf(3) * 2"), 3.0);
    assert!(boolean("fn f(x: Int): Bool { x > 1 }\nnot f(1)"));
    assert_eq!(
        string("fn f(s: String): String { s + s }\nf(\"ab\") + \"!\""),
        "abab!"
    );
    assert_eq!(int("fn f(): {x: Int} { x = 1 }\nf().x"), 1);
    assert!(boolean("fn f(): {x: Int} { x = 1 }\nf() == { x = 1 }"));
    assert_eq!(run("fn f() -> {} {}\nf()").unwrap(), Value::Empty);
    assert_eq!(int("fn f(): fn(): Int { fn(): Int { 7 } }\nf()()"), 7);
    // And through the general walker.
    assert_eq!(
        string("fn f(n: Int): Int { n * 2 }\n\"{f(2)} {f(3)}\""),
        "4 6"
    );
    assert_eq!(
        run("fn f(n: Int): {n: Int} { n = n }\n{ a = f(1); b = f(2) }")
            .unwrap()
            .to_string(),
        "{a = {n = 1}; b = {n = 2}}"
    );
    // A call in a condition, an operand, and an argument.
    assert_eq!(
        int(
            "fn even(n: Int): Bool { n % 2 == 0 }\nfn half(n: Int): Int { n // 2 }\n\
                 if even(4) { half(half(8)) } else { 0 }"
        ),
        2
    );
    // Captured values of each kind, read by each walker.
    let text = "let i = 1\nlet x = 1.5\nlet b = true\nlet s = \"s\"\nlet r = { v = 2 }\n\
                    let g = fn(): Int { 3 }\n\
                    let f = fn(): String { \"{i + 1} {x * 2} {not b} {s + s} {r.v} {g()} {r}\" }\n\
                    f()";
    assert_eq!(string(text), "2 3.0 false ss 2 3 {v = 2}");
}

#[test]
fn recursion() {
    assert_eq!(
        int("fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\nfact(20)"),
        2_432_902_008_176_640_000
    );
    assert_eq!(
        int(
            "let fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
                 fib(20)"
        ),
        6765
    );
    // Through a field, and through a callee captured by a closure.
    assert_eq!(
        int(
            "let m = { sum = fn(n: Int): Int { if n == 0 { 0 } else { n + sum(n - 1) } } }\n\
                 m.sum(10)"
        ),
        55
    );
    assert_eq!(
        int("fn down(n: Int): fn(): Int {\n\
                 \x20 fn(): Int { if n == 0 { 0 } else { down(n - 1)() + 1 } }\n\
                 }\n\
                 down(5)()"),
        5
    );
    // An overflow deep in a recursion is reported where it happens.
    let diagnostic = run(
        "fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\n\
                              fact(21)",
    )
    .unwrap_err();
    assert_eq!(diagnostic.message, "integer overflow");
    // The `*` of the outermost multiplication, `21 * fact(20)`.
    assert_eq!(diagnostic.span, Span::new(48, 49));
    assert_eq!(diagnostic.labels[0].text, "21");
    assert_eq!(diagnostic.input, None);
}

#[test]
fn closures_capture_by_value() {
    assert_eq!(int("let x = 1\nlet f = fn(): Int { x }\nx := 2\nf()"), 1);
    assert_eq!(
        int("let x = 1\nlet f = fn(): Int { x }\nlet x = 5\nf() + x"),
        6
    );
    // Each closure holds what was in scope when it was made.
    assert_eq!(
        string(
            "fn make(n: Int): fn(): Int { fn(): Int { n } }\n\
                    let a = make(1)\nlet b = make(2)\n\"{a()} {b()} {a()}\""
        ),
        "1 2 1"
    );
    // A record captured is the record as it was.
    assert_eq!(
        string("let r = { x = 1 }\nlet f = fn(): {x: Int} { r }\nlet r.x = 2\n\"{f()} {r}\""),
        "{x = 1} {x = 2}"
    );
    // A dotted `let` of a captured record inside the function updates
    // the captured value into a new local, leaving the capture and the
    // outer variable as they were.
    assert_eq!(
        string(
            "let r = { a = 1 }\n\
                 let f = fn(): {a: Int, b: Int} { let r.b = 2\nr }\n\
                 \"{f()} {f()} {r}\""
        ),
        "{a = 1; b = 2} {a = 1; b = 2} {a = 1}"
    );
}

#[test]
fn nested_closures_and_closures_called_later() {
    assert_eq!(
        // The innermost function measures every parameter, through the
        // pure functions around it, so each is present.
        int("fn adder(a: Int): fn(Int): fn(Int): Int {\n\
                   fn(b: Int): fn(Int): Int { fn(c: Int): Int { a * 100 + b * 10 + c } }\n\
                 }\n\
                 adder(1)(2)(3)"),
        123
    );
    assert_eq!(
        int(
            "fn counter(start: Int): fn(): Int { fn(): Int { start + 1 } }\n\
                 let c = counter(41)\nlet unrelated = counter(0)()\nc()"
        ),
        42
    );
    // A function-valued field, called through `.`.
    assert_eq!(
        int("let math = { square = fn(v: Int): Int { v * v }; k = 3 }\nmath.square(math.k)"),
        9
    );
    assert_eq!(
        int(
            "fn tools(): {twice: fn(Int): Int} { twice = fn(v: Int): Int { v * 2 } }\n\
                 tools().twice(21)"
        ),
        42
    );
    // Passed as an argument, and chosen by an `if`.
    assert_eq!(
        int("fn apply(f: fn(Int): Int, v: Int): Int { f(v) }\n\
                 apply(if true { fn(n: Int): Int { n + 1 } } else { fn(n: Int): Int { n } }, 1)"),
        2
    );
}

#[test]
fn arguments_are_evaluated_in_source_order() {
    let f = "fn f(a: Int, b: Int): Int { a + b }\n";
    assert_eq!(int(&format!("{f}f(b = 1, a = 2)")), 3);
    // The first-written argument fails first, whatever it fills.
    assert_eq!(
        error(&format!("{f}f(b = 1 // 0, a = 9223372036854775807 + 1)")).0,
        "division by zero"
    );
    assert_eq!(
        error(&format!("{f}f(b = 9223372036854775807 + 1, a = 1 // 0)")).0,
        "integer overflow"
    );
    // The trailing block comes last.
    let g = "fn g(a: Int, r: {x: Int}): Int { a + r.x }\n";
    assert_eq!(int(&format!("{g}g(1) {{ x = 2 }}")), 3);
    assert_eq!(int(&format!("{g}g(a = 1) {{ x = 2 }}")), 3);
    assert_eq!(
        error(&format!("{g}g(1 // 0) {{ x = 9223372036854775807 + 1 }}")).0,
        "division by zero"
    );
    assert_eq!(
        error(&format!("{g}g(9223372036854775807 + 1) {{ x = 1 // 0 }}")).0,
        "integer overflow"
    );
    // The callee is evaluated before the arguments.
    assert_eq!(
            error("fn f(x: Int): fn(Int): Int { fn(y: Int): Int { y } }\nf(1 // 0)(9223372036854775807 + 1)").0,
            "division by zero"
        );
}

/// Evaluates `ir` without `eval_in`'s truncation, returning the result
/// and the stack's length afterwards.
pub(super) fn run_on(ir: &Rc<Ir>, stack: &mut Vec<Value>) -> (Result<Value, Diagnostic>, usize) {
    let result = Evaluator {
        ir,
        stack,
        base: 0,
        captures: &[],
        callee: None,
        memo: &mut MemoTables::new(),
        shared: &Shared::new(None, &[], None),
        caller: Caller::default(),
        levels: 0,
    }
    .value(ir.root())
    .map_err(|unwind| unwind.into_error(ir.get(ir.root()).span));
    (result, stack.len())
}

#[test]
fn a_stop_makes_a_walkers_result_no_larger() {
    // Every walker returns one of these, so its size is what each level
    // of nesting copies; a stop rides in the error's room.
    assert_eq!(
        size_of::<Flow<Value>>(),
        size_of::<Result<Value, Diagnostic>>()
    );
}

#[test]
fn a_call_truncates_the_stack_whether_it_succeeds_or_fails() {
    let ir = compile(
        "fn f(n: Int): Int { let m = n + 1\nlet k = m * 2\nk }\n\
             fn g(n: Int): Int { let p = f(n)\nf(p) }\nlet a = g(1)\n= g(a)",
    );
    let frame = ir.frame_size() as usize;
    let mut stack = vec![Value::Empty; frame];
    let (result, len) = run_on(&ir, &mut stack);
    // g(1) = f(f(1)) = 10, and g(10) = f(f(10)) = 46.
    assert_eq!(result.unwrap(), Value::Int(46));
    assert_eq!(len, frame);
    // A failure inside a nested call leaves the stack at the frame too.
    let ir = compile(
        "fn f(n: Int): Int { let m = n // 0\nm }\n\
             fn g(n: Int): Int { let p = n\nf(p) }\nlet a = 1\n= g(a)",
    );
    let frame = ir.frame_size() as usize;
    let mut stack = vec![Value::Empty; frame];
    let (result, len) = run_on(&ir, &mut stack);
    assert_eq!(result.unwrap_err().message, "division by zero");
    assert_eq!(len, frame);
    // So does a failure in an argument, after the frame has grown.
    let ir = compile("fn f(a: Int, b: Int): Int { a }\nf(1, 1 // 0)");
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    let (result, len) = run_on(&ir, &mut stack);
    assert_eq!(result.unwrap_err().message, "division by zero");
    assert_eq!(len, ir.frame_size() as usize);
}

#[test]
fn a_warm_stack_does_not_grow_again() {
    let ir = compile(
        "let fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
             fib(15)",
    );
    let frame = ir.frame_size() as usize;
    let mut stack = vec![Value::Empty; frame];
    assert_eq!(eval_in(&ir, &mut stack).unwrap(), Value::Int(610));
    let capacity = stack.capacity();
    // Calls grew it: one slot per call in progress at the deepest.
    assert!(capacity >= frame + 15, "{capacity}");
    assert_eq!(stack.len(), frame);
    assert_eq!(eval_in(&ir, &mut stack).unwrap(), Value::Int(610));
    assert_eq!(stack.capacity(), capacity);
    assert_eq!(stack.len(), frame);
}

pub(super) const BUDGET_ERROR: &str =
    "call nested too deeply: the limit is 10000 levels of nesting, calls included";

pub(super) const BUDGET_NOTE: &str = "every level of expression nesting around a call counts, and so \
                               does the body of the function called; GCSL does not eliminate \
                               tail calls, so recursion this deep is not supported";

#[test]
fn the_first_call_past_the_nesting_budget_fails() {
    on_big_stack(|| {
        let recurse = "fn f(n: Int): Int { if n == 0 { 0 } else { f(n - 1) + 1 } }\n";
        // The shape of the IR does not depend on the argument.
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
        // The first call enters its frame at `outer + CALL_LEVELS`
        // levels, and each nested call `inner + CALL_LEVELS` further in;
        // each is made while the body it enters fits within the budget.
        let allowed = 1 + (MAX_DEPTH - height - outer - CALL_LEVELS) / (inner + CALL_LEVELS);
        // `f(n)` makes n + 1 calls: exactly the allowed number succeeds.
        let last = i64::try_from(allowed - 1).unwrap();
        assert_eq!(
            run(&format!("{recurse}f({last})")).unwrap(),
            Value::Int(last)
        );
        // One more fails, at the innermost call, which would exceed it.
        let text = format!("{recurse}f({})", last + 1);
        let diagnostic = run(&text).unwrap_err();
        assert_eq!(diagnostic.message, BUDGET_ERROR);
        assert_eq!(
            diagnostic.message,
            format!(
                "call nested too deeply: the limit is {MAX_DEPTH} levels of nesting, calls \
                     included"
            )
        );
        assert_eq!(diagnostic.notes, vec![BUDGET_NOTE]);
        assert_eq!(diagnostic.span, inner_span);
        assert_eq!(diagnostic.span, Span::new(43, 51));
        assert!(diagnostic.labels.is_empty());
        // Nothing is left on the stack by the failure.
        let ir = compile(&text);
        let mut stack = vec![Value::Empty; ir.frame_size() as usize];
        let (result, len) = run_on(&ir, &mut stack);
        assert_eq!(result.unwrap_err().message, BUDGET_ERROR);
        assert_eq!(len, ir.frame_size() as usize);
    });
}

#[test]
fn the_nesting_around_a_top_level_call_counts() {
    on_big_stack(|| {
        // `f()` as the left operand of `terms` additions, which put it
        // `terms + 1` levels deep in the file's body.
        let program = |terms: usize| format!("fn f(): Int {{ 1 }}\nf(){}", " + 1".repeat(terms));
        let ir = compile(&program(1));
        let [(depth, _)] = ir.calls_in(ir.root())[..] else {
            panic!("expected one call at the top level");
        };
        assert_eq!(depth, 2);
        let height = ir.function(FnId::new(0)).height as usize;
        // The deepest the call may be, and so the most additions.
        let terms = MAX_DEPTH - CALL_LEVELS - height - 1;
        let ir = compile(&program(terms));
        let [(depth, _)] = ir.calls_in(ir.root())[..] else {
            panic!("expected one call at the top level");
        };
        assert_eq!(usize::from(depth), terms + 1);
        let sum = i64::try_from(terms + 1).unwrap();
        assert_eq!(eval(&ir).unwrap(), Value::Int(sum));
        let diagnostic = run(&program(terms + 1)).unwrap_err();
        assert_eq!(diagnostic.message, BUDGET_ERROR);
        assert_eq!(diagnostic.span, Span::new(18, 21));
    });
}

#[test]
fn function_values_and_their_internal_errors() {
    // A closure shows as its type, its parameters' laziness inferred.
    let value = run("fn(x: Int, s: String): {x: Int} { x = x }").unwrap();
    assert_eq!(value.kind(), ValueKind::Fn);
    assert_eq!(
        value.to_string(),
        "fn(x: Int, s: String | Future<String>): {x: Int}"
    );
    assert_eq!(
        value.repr().to_string(),
        "fn(x: Int, s: String | Future<String>): {x: Int}"
    );
    // An impure function's bare types are the either shape, but for
    // what its result's take from its body.
    let value = run("fn(x: Int, s: String) -> {x: Int} { x = x }").unwrap();
    assert_eq!(
        value.to_string(),
        "fn(x: Int | Future<Int>, s: String | Future<String>) -> {x: Int | Future<Int>}"
    );
    assert_eq!(
        string("let f = fn(): Int { 1 }\n\"f is {f}\""),
        "f is fn(): Int"
    );
    // The callee outside a function, and an argument outside a call.
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let callee = builder.alloc(IrKind::LoadCallee, Span::new(0, 1));
    let ir = Rc::new(builder.finish(callee, 0, Vec::new()));
    assert_eq!(
        eval(&ir).unwrap_err().message,
        "internal error: load of the callee outside a function"
    );
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
    let arg = builder.alloc(
        IrKind::Arg {
            param: 0,
            value: crate::ir::ArgValue::Node(one),
        },
        Span::new(0, 1),
    );
    let ir = Rc::new(builder.finish(arg, 0, Vec::new()));
    assert_eq!(
        eval(&ir).unwrap_err().message,
        "internal error: an argument outside a call"
    );
    // A closure of a function its IR does not have.
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let empty = builder.push_item_list(&[]);
    let closure = builder.alloc(
        IrKind::MakeClosure {
            function: FnId::new(3),
            captures: empty,
        },
        Span::new(0, 1),
    );
    let call = builder.alloc(
        IrKind::Call {
            callee: closure,
            args: empty,
            kind: ValueKind::Int,
            depth: 0,
        },
        Span::new(0, 3),
    );
    let ir = Rc::new(builder.finish(call, 0, Vec::new()));
    assert_eq!(
        eval(&ir).unwrap_err().message,
        "internal error: a closure of a function its IR lacks"
    );
}

#[test]
fn a_rest_parameter_is_the_list_of_its_arguments() {
    assert_eq!(
        run("fn sum(...xs: Int, scale: Int = 1): Int { let mut t = 0\nfor x in xs { t := t + x }\nt * scale }\n\
             let some = [4, 5]\n\
             {a = sum(); b = sum(1, 2, 3); c = sum(...some); d = sum(1, ...some, 6, scale = 10)}")
        .unwrap()
        .to_string(),
        "{a = 0; b = 6; c = 9; d = 160}"
    );
    // Through a value of a function type, and a generic one.
    assert_eq!(
        run(
            "let f: fn(...xs: Int): Int = fn(...xs: Int): Int { xs[0] }\n\
             fn first<T>(...xs: T): T { xs[0] }\n\
             {a = f(7, 8); b = first(\"x\", \"y\"); c = first(...[1.5])}"
        )
        .unwrap()
        .to_string(),
        "{a = 7; b = \"x\"; c = 1.5}"
    );
}
