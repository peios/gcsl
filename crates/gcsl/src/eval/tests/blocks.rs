//! Tests of the evaluator: blocks, which run in the frame of the call that
//! gave them each time their function calls them.

use super::*;

/// Functions that take a block: `run`, which calls it once and gives its
/// value; `twice`, which calls it twice; `never`, which does not call it;
/// and `pure_of`, whose block is pure.
const RUNNERS: &str = "fn run(body: block -> Int) -> Int { body() }\n\
                       fn twice(body: block -> {}) -> {} { body(); body(); = {} }\n\
                       fn never(body: block -> {}) -> {} { = {} }\n\
                       fn pure_of(n: Int, body: block: Int): Int { body() + n }\n";

/// `text` after [`RUNNERS`].
fn with_runners(text: &str) -> String {
    format!("{RUNNERS}{text}")
}

#[test]
fn a_block_rebinds_its_caller_s_variables() {
    // The block's `:=` is of the caller's variable, as the caller reads it
    // after the call.
    let text = with_runners("let mut n = 1\nrun { n := n + 10; n }\nn");
    assert_eq!(int(&text), 11);
    // Several variables, a record's field and a list's element among them.
    let text = with_runners(
        "let mut total = 0\n\
         let mut conf = { port = 80; host = \"a\" }\n\
         let mut xs = [1, 2, 3]\n\
         twice { total := total + 5; conf.port := conf.port + 1; xs[0] := xs[0] * 10 }\n\
         \"{total} {conf} {xs}\"",
    );
    assert_eq!(string(&text), "10 {port = 82; host = \"a\"} [100, 2, 3]");
    // In a function's frame, each call of the function with its own.
    let text = with_runners(
        "fn count(start: Int) -> Int { let mut n = start; twice { n := n + 1 }; = n }\n\
         count(0) * 100 + count(40)",
    );
    assert_eq!(int(&text), 242);
    // The caller reads a variable between two calls of its block's
    // function, and the block reads what the caller wrote.
    let text = with_runners(
        "let mut n = 0\ntwice { n := n + 1 }\nlet seen = n\nn := 100\ntwice { n := n + 1 }\n\
         seen * 1000 + n",
    );
    assert_eq!(int(&text), 2102);
    // A closure made after the block ran copies what the block wrote.
    let text =
        with_runners("let mut n = 1\ntwice { n := n * 3 }\nlet f = fn(): Int { n }\nn := 0\nf()");
    assert_eq!(int(&text), 9);
}

#[test]
fn a_block_runs_each_time_it_is_called_and_not_otherwise() {
    assert_eq!(
        int(&with_runners("let mut n = 0\ntwice { n := n + 1 }\nn")),
        2
    );
    assert_eq!(
        int(&with_runners("let mut n = 0\nnever { n := n + 1 }\nn")),
        0
    );
    // A block that would fail is not run by a function that does not call
    // it.
    assert_eq!(
        int(&with_runners(
            "let zero = 0\nnever { let x = 1 // zero }\n7"
        )),
        7
    );
    // Many times, in a loop, and as often as a condition says.
    let text = "fn times(n: Int, body: block -> {}) -> {} { for i in 0..n { body() }; = {} }\n\
                fn until(limit: Int, body: block -> Int) -> Int {\n\
                \x20   let mut last = 0\n\
                \x20   for last < limit { last := await body() }\n\
                \x20   = last\n\
                }\n\
                let mut n = 0\n\
                times(1000) { n := n + 1 }\n\
                let mut k = 1\n\
                let reached = until(500) { k := k * 2; k }\n\
                \"{n} {reached} {k}\"";
    assert_eq!(string(text), "1000 512 512");
    // Each call gives the block's value as it then is.
    let text = "fn pair(body: block: Int): List<Int> { [body(), body() * 2] }\n\
                let mut n = 0\n\
                = pair { 3 }";
    assert_eq!(
        string(&format!("let r = {{ {text} }}\n\"{{r}}\"")),
        "[3, 6]"
    );
    let text = "fn both(body: block -> Int) -> List<Int> { let a = await body(); let b = await body(); = [a, b] }\n\
                let mut n = 0\n\
                let got = both { n := n + 1; n * n }\n\
                \"{got}\"";
    assert_eq!(string(text), "[1, 4]");
}

#[test]
fn a_block_s_own_variables_are_new_at_each_call() {
    // Declared in the block, in its caller's frame: each call starts them
    // again.
    let text = with_runners(
        "let mut log = \"\"\n\
         twice { let mut seen = 0; seen := seen + 1; log := log + \"{seen}\" }\n\
         log",
    );
    assert_eq!(string(&text), "11");
    // They do not disturb the caller's variables declared after the call.
    let text = with_runners(
        "let a = 1\nlet got = run { let inner = 20; let more = 300; inner + more }\nlet b = 4000\n\
         a + got + b",
    );
    assert_eq!(int(&text), 4321);
    // A loop and a function of its own in a block.
    let text = with_runners(
        "let mut sum = 0\n\
         let got = run { for i in 1..=4 { sum := sum + i }; let f = fn(k: Int): Int { k * 2 }; f(sum) }\n\
         sum * 100 + got",
    );
    assert_eq!(int(&text), 1020);
}

#[test]
fn a_call_s_arguments_are_evaluated_before_its_block_runs() {
    let text = "fn step(a: String, b: String, body: block -> {}) -> String {\n\
                \x20   body()\n\
                \x20   = \"{a}{b}\"\n\
                }\n\
                let mut log = \"\"\n\
                let got = step({ log := log + \"a\"; \"x\" }, { log := log + \"b\"; \"y\" }) {\n\
                \x20   log := log + \"c\"\n\
                }\n\
                \"{log} {got}\"";
    assert_eq!(string(text), "abc xy");
    // The block runs where the function calls it, among what the function
    // does.
    let text = "fn around(body: block -> {}) -> String {\n\
                \x20   let before = \"<\"\n\
                \x20   body()\n\
                \x20   let after = \">\"\n\
                \x20   = before + after\n\
                }\n\
                let mut log = \"\"\n\
                let got = around { log := log + \"in\" }\n\
                log + got";
    assert_eq!(string(text), "in<>");
}

#[test]
fn a_block_gives_a_value_of_every_kind() {
    // Through each typed walker: the call of the block is an operand of an
    // operator of its kind.
    assert_eq!(
        int("fn f(body: block: Int): Int { body() * 2 + body() }\nf { 7 }"),
        21
    );
    assert_eq!(
        float("fn f(body: block: Float): Float { body() / 2.0 }\nf { 5.0 }"),
        2.5
    );
    assert!(boolean(
        "fn f(body: block: Bool): Bool { not body() or body() }\nf { false }"
    ));
    assert_eq!(
        string("fn f(body: block: String): String { body() + \"!\" }\nf { \"hey\" }"),
        "hey!"
    );
    assert_eq!(
        int("fn f(body: block: {a: Int, b: Int}): Int { body().a + body().b }\nf { a = 1; b = 2 }"),
        3
    );
    assert_eq!(
        int("fn f(body: block: List<Int>): Int { body()[1] + len(body()) }\nf { [5, 6, 7] }"),
        9
    );
    assert_eq!(
        int("fn f(body: block: fn(Int): Int): Int { body()(4) }\nf { fn(n: Int): Int { n * n } }"),
        16
    );
    assert_eq!(
        string(
            "fn f(body: block: Int | String): String { let v = body(); = if v is Int { \"int\" } else { \"string\" } }\nf { \"s\" }"
        ),
        "string"
    );
    // The condition of an `if` and of a loop, and an interpolation.
    assert_eq!(
        string(
            "fn f(body: block: Bool): String { if body() { \"yes {body()}\" } else { \"no\" } }\n\
             f { 1 < 2 }"
        ),
        "yes true"
    );
    // An impure block's value may be a future, which its function awaits.
    assert_eq!(
        int("fn f(body: block -> Int) -> Int { await body() }\nf { lazy 5 }"),
        5
    );
    assert_eq!(
        int(&with_runners("let l = lazy 6\nrun { await l } + run { 1 }")),
        7
    );
    // A block of `{}` that ends with an impure call discards its value.
    let text =
        with_runners("fn bump() -> Int { 5 }\nlet mut n = 0\ntwice { n := n + 1; bump() }\nn");
    assert_eq!(int(&text), 2);
}

#[test]
fn a_pure_block_runs_in_its_caller_s_frame_too() {
    // It reads its caller's variables as they are when it is called.
    assert_eq!(
        int(&with_runners(
            "let mut n = 4\nn := n + 1\npure_of(100) { n * 10 }"
        )),
        150
    );
    // In a pure function.
    assert_eq!(
        int(&with_runners(
            "fn f(k: Int): Int { let base = k * 2; = pure_of(1) { base + k } }\nf(5)"
        )),
        16
    );
}

#[test]
fn blocks_nest_and_are_passed_on() {
    // A block in a block, each rebinding the same variable.
    let text = with_runners("let mut n = 0\ntwice { n := n + 1; twice { n := n + 10 } }\nn");
    assert_eq!(int(&text), 42);
    // Passed on by calling it in another call's block: it still runs in
    // the frame of the call that gave it, two frames down.
    let text = with_runners(
        "fn outer(body: block -> {}) -> Int {\n\
         \x20   let mut mine = 0\n\
         \x20   twice { mine := mine + 1; body() }\n\
         \x20   = mine\n\
         }\n\
         let mut n = 0\n\
         let got = outer { n := n + 100 }\n\
         n + got",
    );
    assert_eq!(int(&text), 202);
    // A block's function may call itself with a block that calls the
    // first.
    let text = "fn count(k: Int, body: block -> {}) -> {} {\n\
                \x20   if k > 0 { body(); count(k - 1) { body() }; {} }\n\
                }\n\
                let mut n = 0\n\
                count(5) { n := n + 1 }\n\
                n";
    assert_eq!(int(text), 5);
    // Each level adding its own work to the block it passes on.
    let text = "fn nest(k: Int, body: block: String): String {\n\
                \x20   if k == 0 { body() } else { nest(k - 1) { \"({k}{body()})\" } }\n\
                }\n\
                nest(3) { \"x\" }";
    assert_eq!(string(text), "(1(2(3x)))");
    // A block given inside a block's function to a function that takes
    // one, with variables of three frames.
    let text = with_runners(
        "fn middle(k: Int, body: block -> Int) -> Int {\n\
         \x20   let mut here = k\n\
         \x20   = run { here := here + await body(); here * 2 }\n\
         }\n\
         let mut top = 3\n\
         middle(10) { top := top + 1; top }",
    );
    assert_eq!(int(&text), 28);
}

#[test]
fn recursion_through_a_function_that_takes_a_block() {
    // A function that gives a block and is called from that block.
    let text = with_runners(
        "fn fact(n: Int) -> Int { if n <= 1 { 1 } else { run { n * fact(n - 1) } } }\nfact(10)",
    );
    assert_eq!(int(&text), 3_628_800);
    // Each recursive call has its own variable for its block to rebind.
    let text = with_runners(
        "fn sum(xs: List<Int>, at: Int) -> Int {\n\
         \x20   let mut acc = 0\n\
         \x20   if at < len(xs) { twice { acc := acc + xs[at] }; acc := acc + sum(xs, at + 1) }\n\
         \x20   = acc\n\
         }\n\
         sum([1, 2, 3, 4], 0)",
    );
    assert_eq!(int(&text), 20);
}

#[test]
fn a_function_recovers_a_failure_of_its_block() {
    let guarded = "let zero = 0\n\
                   fn guarded(body: block -> {}) -> String {\n\
                   \x20   = recover { body(); = \"passed\" } else (failure) { \"caught {failure.message}\" }\n\
                   }\n";
    assert_eq!(
        string(&format!("{guarded}guarded {{ let x = 1 // zero }}")),
        "caught division by zero"
    );
    assert_eq!(string(&format!("{guarded}guarded {{ {{}} }}")), "passed");
    // What the block wrote before it failed is kept, and the caller goes
    // on.
    let text = format!(
        "{guarded}let mut n = 0\n\
         let a = guarded {{ n := n + 1; let x = 1 % zero }}\n\
         let b = guarded {{ n := n + 10; let xs = [1]; let x = xs[5] }}\n\
         let c = guarded {{ n := n + 100 }}\n\
         \"{{a}} / {{b}} / {{c}} / {{n}}\""
    );
    assert_eq!(
        string(&text),
        "caught division by zero / caught index out of range / passed / 111"
    );
    // The block recovers its own failure, and its function sees none.
    let text =
        format!("{guarded}guarded {{ let got = recover {{ 1 // zero }} else (failure) {{ 0 }} }}");
    assert_eq!(string(&text), "passed");
    // A failure of the function after the block ran is not the block's.
    let text = "fn after(body: block -> {}) -> Int { body(); = 1 // 0 }\n\
                let mut n = 0\n\
                let got = recover { after { n := 5 } } else (failure) { -1 }\n\
                got * 10 + n";
    assert_eq!(int(text), -5);
}

#[test]
fn a_failure_in_a_block_is_reported_where_it_was_raised() {
    let text = with_runners("let zero = 0\nrun { 10 // zero }");
    let (message, span) = error(&text);
    assert_eq!(message, "division by zero");
    let at = u32::try_from(text.find("//").unwrap()).unwrap();
    assert_eq!(span, Span::new(at, at + 2));
    // Through two functions, the inner block's failure.
    let text = with_runners("let xs = [1]\ntwice { let v = run { xs[3] } }");
    assert_eq!(error(&text).0, "index out of range");
    // The stack is as it was found.
    let ir = compile_prelude(&with_runners(
        "let mut n = 0\ntwice { n := n + 1; let x = 1 // (n - 1) }",
    ));
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    let (result, len) = run_on(&ir, &mut stack);
    assert_eq!(result.unwrap_err().message, "division by zero");
    assert_eq!(len, ir.frame_size() as usize);
}

#[test]
fn a_block_in_a_lazy_and_in_a_closure_runs_with_their_captures() {
    // A `lazy`'s body gives a block that reads what the `lazy` captured
    // and rebinds the body's own variable.
    let text = with_runners(
        "let x = 5\n\
         let later = lazy { let mut m = x; twice { m := m + x }; m }\n\
         await later",
    );
    assert_eq!(int(&text), 15);
    // Awaited after the captured variable changed: the copy is read.
    let text = with_runners(
        "let mut x = 5\n\
         let later = lazy { let mut m = 0; twice { m := m + x }; m }\n\
         x := 1000\n\
         await later",
    );
    assert_eq!(int(&text), 10);
    // A closure's, called later and twice.
    let text = with_runners(
        "let k = 7\n\
         let f = fn(start: Int) -> Int { let mut m = start; twice { m := m + k }; = m }\n\
         f(0) * 100 + f(1)",
    );
    assert_eq!(int(&text), 1415);
    // A `lazy` awaited inside a function called from a block.
    let text = with_runners(
        "let base = 3\n\
         let later = lazy { run { base * 2 } }\n\
         let mut n = 0\n\
         twice { n := n + await later }\n\
         n",
    );
    assert_eq!(int(&text), 12);
    // A memoised function's body gives a block, and is run once.
    let text = with_runners(
        "fn made(k: Int) ->> Int { let mut m = k; twice { m := m * 2 }; = m }\n\
         made(3) + made(3) + made(1)",
    );
    assert_eq!(int(&text), 28);
}

#[test]
fn a_block_sees_the_context_set_around_its_call() {
    // The `with` in the block's function is around the call of the block,
    // so the block reads what it sets.
    let depth = "context depth: Int = 0\n\
                 fn deeper(body: block -> Int) -> Int { with { depth = ^depth + 1 } { body() } }\n";
    assert_eq!(int(&format!("{depth}deeper {{ ^depth }}")), 1);
    assert_eq!(
        int(&format!(
            "{depth}deeper {{ deeper {{ deeper {{ ^depth }} }} }}"
        )),
        3
    );
    // Outside the `with`, the block reads the context of its function's
    // call.
    let text = format!(
        "{depth}fn both(body: block -> Int) -> List<Int> {{\n\
         \x20   let inside = with {{ depth = 10 }} {{ await body() }}\n\
         \x20   let outside = await body()\n\
         \x20   = [inside, outside]\n\
         }}\n\
         let got = with {{ depth = 5 }} {{ both {{ ^depth }} }}\n\
         \"{{got}}\""
    );
    assert_eq!(string(&text), "[10, 5]");
    // A function the block calls reads it too.
    let text = format!(
        "{depth}fn read() -> Int {{ ^depth }}\nlet mut seen = 0\n\
         let got = deeper {{ seen := read(); seen + 100 }}\n\
         got * 10 + seen"
    );
    assert_eq!(int(&text), 1011);
}

#[test]
fn a_block_goes_through_methods_generics_defaults_and_rest_parameters() {
    let text = "fn (n: Int) times(body: block -> {}) -> {} { for i in 0..n { body() }; = {} }\n\
                let mut count = 0\n\
                3.times { count := count + 1 }\n\
                4.times() { count := count + 10 }\n\
                count";
    assert_eq!(int(text), 43);
    let text = "fn measure<T>(name: String, body: block -> T) -> {name: String, value: T} {\n\
                \x20   = { name = name; value = await body() }\n\
                }\n\
                let a = measure(\"n\") { 1 + 1 }\n\
                let b = measure(\"s\") { \"text\" }\n\
                \"{a} {b}\"";
    assert_eq!(
        string(text),
        "{name = \"n\"; value = 2} {name = \"s\"; value = \"text\"}"
    );
    let text = "fn dflt(n: Int = 2, body: block: Int): Int { body() + n }\n\
                dflt { 1 } * 100 + dflt(n = 30) { 1 }";
    assert_eq!(int(text), 331);
    // After a rest parameter, which takes every positional argument.
    let text = "fn rest(first: Int, ...more: Int, body: block: Int): Int {\n\
                \x20   first + len(more) * 10 + body() * 100\n\
                }\n\
                rest(1) { 2 } + rest(1, 9, 9, 9) { 3 } * 1000 + rest(4, ...[7, 7]) { 5 } * 1000000";
    assert_eq!(int(text), 524_331_201);
    // Through a function value and a function type.
    let text = with_runners(
        "fn each(f: fn(block -> Int) -> Int) -> Int { let mut n = 1; = f { n := n + 1; n * 5 } }\n\
         let g = run\n\
         each(g) + each(fn(body: block -> Int) -> Int { await body() + await body() })",
    );
    assert_eq!(int(&text), 35);
}

#[test]
fn a_block_in_a_record_block_a_branch_a_loop_and_a_list() {
    // In a record block, whose variable it rebinds.
    let text = with_runners(
        "let r = { let mut t = 1; twice { t := t * 4 }; a = t; b = run { t + 1 } }\n\"{r}\"",
    );
    assert_eq!(string(&text), "{a = 16; b = 17}");
    // In an `if`'s branch, a loop's body and a list's element.
    let text = with_runners(
        "let mut n = 0\n\
         if n == 0 { twice { n := n + 1 } }\n\
         let xs = for i in 1..=3 { run { n := n + i; n } }\n\
         let ys = [run { 1 }, run { n }]\n\
         \"{xs} {ys} {n}\"",
    );
    assert_eq!(string(&text), "[3, 5, 8] [1, 8] 8");
}

#[test]
fn the_nesting_budget_covers_a_block_s_body() {
    on_big_stack(|| {
        // `run`'s block is the sum of `terms + 1` terms, so `terms` levels
        // taller than that of one.
        let program = |terms: usize| {
            format!(
                "fn run(body: block: Int): Int {{ body() }}\nrun {{ 1{} }}",
                " + 1".repeat(terms)
            )
        };
        let ir = compile(&program(0));
        let [(outer, _)] = ir.calls_in(ir.root())[..] else {
            panic!("expected one call at the top level");
        };
        let [(kind, inner)] = ir.block_calls()[..] else {
            panic!("expected one call of a block");
        };
        assert_eq!(kind, ValueKind::Int);
        let [height] = ir.block_heights()[..] else {
            panic!("expected one block");
        };
        // The call of `run` enters its frame at `outer + CALL_LEVELS`
        // levels and the call of the block `inner + CALL_LEVELS` further
        // in, where the block's body must fit.
        let entering = usize::from(outer) + CALL_LEVELS + usize::from(inner) + CALL_LEVELS;
        let terms = MAX_DEPTH - entering - usize::from(height);
        let ir = compile(&program(terms));
        assert_eq!(
            ir.block_heights(),
            [u16::try_from(usize::from(height) + terms).unwrap()]
        );
        let sum = i64::try_from(terms + 1).unwrap();
        assert_eq!(eval(&ir).unwrap(), Value::Int(sum));
        // One level more fails, at the call of the block.
        let text = program(terms + 1);
        let diagnostic = run(&text).unwrap_err();
        assert_eq!(diagnostic.message, BUDGET_ERROR);
        assert_eq!(diagnostic.notes, vec![BUDGET_NOTE]);
        let at = u32::try_from(text.find("body()").unwrap()).unwrap();
        assert_eq!(diagnostic.span, Span::new(at, at + 6));
    });
}

#[test]
fn blocks_that_nest_without_end_run_out_of_the_nesting_budget() {
    on_big_stack(|| {
        // Each call gives a block that calls the one it was given: the
        // calls nest until one would pass the budget.
        let text = "fn forever(body: block: Int): Int { forever { body() + 1 } }\nforever { 0 }";
        assert_eq!(error(text).0, BUDGET_ERROR);
        // The calls fit, and the chain of blocks the innermost starts does
        // not: each block calls the one before it, a level further in.
        let deep = "fn deep(n: Int, body: block: Int): Int {\n\
                    \x20   if n == 0 { body() } else { deep(n - 1) { body() + 1 } }\n\
                    }\n";
        assert_eq!(int(&format!("{deep}deep(100) {{ 0 }}")), 100);
        let text = format!("{deep}deep(700) {{ 0 }}");
        let (message, span) = error(&text);
        assert_eq!(message, BUDGET_ERROR);
        // At a call of a block, not of `deep`.
        let written = &text[span.start as usize..span.end as usize];
        assert_eq!(written, "body()");
        // Nothing is left on the stack by the failure.
        let ir = compile_prelude(&text);
        let mut stack = vec![Value::Empty; ir.frame_size() as usize];
        let (result, len) = run_on(&ir, &mut stack);
        assert_eq!(result.unwrap_err().message, BUDGET_ERROR);
        assert_eq!(len, ir.frame_size() as usize);
    });
}
