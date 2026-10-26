//! Tests of the evaluator: mutable variables and index updates.

use super::*;

#[test]
fn a_mutable_variable_is_rebound_from_inner_blocks() {
    // A sum, a count and a maximum over a list.
    let text = "let xs = [3, 1, 4, 1, 5]\n\
                    let mut total = 0\n\
                    let mut count = 0\n\
                    let mut max = 0\n\
                    for x in xs {\n\
                    \x20 total := total + x\n\
                    \x20 count := count + 1\n\
                    \x20 if x > max { max := x }\n\
                    }\n\
                    \"{total} {count} {max}\"";
    assert_eq!(string(text), "14 5 5");
    // The examples of the summary.
    let text = "let mut total = 0\n\
                    for x in [1, 2, 3] { total := total + x }\n\
                    if total > 5 { total := total * 2 }\n\
                    let mut conf = { port = 80; host = \"a\" }\n\
                    if true { conf.port := 443 }\n\
                    \"{total} {conf}\"";
    assert_eq!(string(text), "12 {port = 443; host = \"a\"}");
    // In the declaring block, in every branch, and deep.
    assert_eq!(int("let mut x = 1\nx := x + 1\nx := x * 10\nx"), 20);
    assert_eq!(
        int("let mut x = 0\nif false { x := 1 } else if true { x := 2 } else { x := 3 }\nx"),
        2
    );
    assert_eq!(
        int("let mut x = 0\nfor i in [1] { if true { { { x := 7 } } } }\nx"),
        7
    );
    // A record block's own mutable variable.
    assert_eq!(
        string("let r = { let mut t = 1; { t := t + 1 }; a = t; t := 10; b = t }\n\"{r}\""),
        "{a = 2; b = 10}"
    );
}

#[test]
fn a_for_sees_its_rebinds_in_order() {
    // A guard that reads what the body rebinds: an element stops
    // contributing once it fails.
    let text = "let mut acc = 0\n\
                    let taken = for v in [3, 1, 4, 1, 5] if acc < 5 { acc := acc + v; v }\n\
                    \"{taken} {acc}\"";
    assert_eq!(string(text), "[3, 1, 4] 8");
    // A guard that rebinds, in a block of its own.
    let text = "let mut seen = 0\n\
                    let odd = for v in 1..=5 if { seen := seen + 1; v % 2 == 1 } { v }\n\
                    \"{odd} {seen}\"";
    assert_eq!(string(text), "[1, 3, 5] 5");
    // Nested loops.
    assert_eq!(
        int("let mut n = 0\nfor i in 0..4 { for j in 0..i { n := n + 1 } }\nn"),
        6
    );
    let text = "let mut p = 0\n\
                    for i in 1..=3 {\n\
                    \x20 let mut row = 0\n\
                    \x20 for j in 1..=3 { row := row + i * j }\n\
                    \x20 p := p * 100 + row\n\
                    }\n\
                    p";
    assert_eq!(int(text), 61218);
    // The iterable is evaluated once, before the body rebinds it.
    let text = "let mut xs = [1, 2, 3]\n\
                    let ys = for x in xs { xs := [x]; x }\n\
                    len(ys) * 10 + len(xs)";
    assert_eq!(int(text), 31);
}

#[test]
fn closures_and_lazies_copy_a_mutable_variable_when_made() {
    // Made before and after a rebind, each sees the value it had then.
    let text = "let mut x = 1\n\
                    let f = fn(): Int { x }\n\
                    let a = lazy x\n\
                    x := 2\n\
                    let g = fn(): Int { x }\n\
                    let b = lazy x\n\
                    x := 3\n\
                    f() * 10000 + g() * 1000 + (await a) * 100 + (await b) * 10 + x";
    assert_eq!(int(text), 12_123);
    // One closure for each iteration, each with that iteration's value.
    let text = "let mut x = 0\n\
                    let fs = for i in 0..3 { x := x + 10; fn(): Int { x } }\n\
                    x := 1000\n\
                    fs[0]() + fs[1]() * 100 + fs[2]() * 10000";
    assert_eq!(int(text), 302_010);
    // A `lazy` awaited after the variable changes still has its copy.
    let text = "let mut x = 5\nlet l = lazy x * 2\nfor i in [1] { x := 0 }\nawait l";
    assert_eq!(int(text), 10);
}

#[test]
fn a_read_of_a_mutable_variable_gives_its_value_when_evaluated() {
    // Evaluation is in source order: the left operand is read before
    // the right one rebinds it.
    assert_eq!(int("let mut x = 1\nlet y = x + { x := 5; x }\ny"), 6);
    assert_eq!(int("let mut x = 1\nlet y = { x := 5; x } + x\ny"), 10);
    // An argument rebinding it, before the next is evaluated.
    assert_eq!(
        int("fn sub(a: Int, b: Int): Int { a - b }\nlet mut x = 1\nsub(x, { x := 10; x })"),
        -9
    );
    // A dotted rebind evaluates its value first, which may rebind the
    // variable too, and then reads the record it updates: the update of
    // the variable as it then is is what is stored.
    assert_eq!(
        string("let mut r = { a = 1; b = 2 }\nr.a := { r := { a = 7; b = 8 }; r.b }\n\"{r}\""),
        "{a = 8; b = 8}"
    );
}

#[test]
fn a_dotted_rebind_of_a_mutable_variable() {
    let text = "let mut c = { hits = 0; tags = { last = \"\" } }\n\
                    for s in [\"a\", \"b\"] { c.hits := c.hits + 1; c.tags.last := s }\n\
                    \"{c.hits} {c.tags.last}\"";
    assert_eq!(string(text), "2 b");
    // The old record is not changed.
    assert_eq!(
        int("let mut c = { a = 1 }\nlet old = c\n{ c.a := 2 }\nold.a * 10 + c.a"),
        12
    );
    // A record that may be a future is awaited where it is updated, and
    // where it is read after.
    assert_eq!(
        int("let mut r: {a: Int} | Future<{a: Int}> = lazy { a = 1 }\n{ r.a := r.a + 1 }\nr.a"),
        2
    );
}

#[test]
fn each_call_has_its_own_mutable_variables() {
    // A function called many times starts from a fresh variable each
    // time.
    let text = "fn count(xs: Int[]): Int { let mut n = 0; for x in xs { n := n + 1 }; n }\n\
                    count([1, 2]) * 100 + count([1, 2, 3]) * 10 + count([])";
    assert_eq!(int(text), 230);
    assert_eq!(
        int("let f = fn(): Int { let mut n = 0; n := n + 1; n }\nf() + f() + f()"),
        3
    );
    // Recursion with a mutable local, rebound before and after the
    // recursive call: each frame has its own.
    let text = "fn sum(n: Int): Int { let mut s = n; if n > 0 { s := s + sum(n - 1) }; s }\n\
                    sum(4)";
    assert_eq!(int(text), 10);
    let text = "fn f(n: Int): Int {\n\
                    \x20 let mut a = n\n\
                    \x20 if n > 0 { let b = f(n - 1); a := a * 10 + b }\n\
                    \x20 a\n\
                    }\n\
                    f(2)";
    assert_eq!(int(text), 30);
    // A memoised one too, which runs its body once per argument.
    let text = "fn fact(n: Int) ->> Int { let mut p = 1; for i in 1..=n { p := p * i }; p }\n\
                    fact(5) + fact(5) + fact(3)";
    assert_eq!(int(text), 246);
    // And one that rebinds a variable holding a function.
    let text = "let mut f = fn(n: Int): Int { if n == 0 { 0 } else { f(n - 1) + 1 } }\n\
                    let a = f(3)\n\
                    f := fn(n: Int): Int { n * 2 }\n\
                    a * 10 + f(3)";
    assert_eq!(int(text), 36);
}

#[test]
fn an_index_path_replaces_one_element() {
    for (text, expected) in [
        ("let mut xs = [1, 2, 3]\nxs[0] := 10\nxs", "[10, 2, 3]"),
        (
            "let mut xs = [1, 2, 3]\nfor i in 0..len(xs) { xs[i] := xs[i] * 2 }\nxs",
            "[2, 4, 6]",
        ),
        (
            "let mut conf = { hosts = [{ name = \"a\"; up = false }] }\n\
                 conf.hosts[0].up := true\nconf",
            "{hosts = [{name = \"a\"; up = true}]}",
        ),
        (
            "let mut grid = [[0, 0], [0, 0]]\nlet y = 1\nlet x = 0\ngrid[y][x] := 1\ngrid",
            "[[0, 0], [1, 0]]",
        ),
        // First, middle and last; of a variable that is not mutable too.
        ("let xs = [1, 2, 3]\nxs[0] := 9\nxs", "[9, 2, 3]"),
        ("let xs = [1, 2, 3]\nxs[1] := 9\nxs", "[1, 9, 3]"),
        ("let xs = [1, 2, 3]\nxs[2] := 9\nxs", "[1, 2, 9]"),
        // Nested lists, lists of records and records of lists.
        (
            "let mut m = [[1, 2], [3, 4]]\nm[1][0] := 30\nm[0] := [5]\nm",
            "[[5], [30, 4]]",
        ),
        (
            "let mut rs = [{ a = 1; b = [1] }, { a = 2; b = [2] }]\n\
                 rs[1].a := 20\nrs[0].b[0] := 10\nrs",
            "[{a = 1; b = [10]}, {a = 20; b = [2]}]",
        ),
        (
            "let mut r = { xs = [1, 2]; n = { ys = [[0]] } }\nr.xs[1] := 5\n\
                 r.n.ys[0][0] := 7\nr",
            "{xs = [1, 5]; n = {ys = [[7]]}}",
        ),
        // An element of another type, as the element type allows.
        ("let mut xs = [1.5, 2.5]\nxs[1] := 3\nxs", "[1.5, 3.0]"),
        (
            "let mut xs = [\"a\"]\nxs[0] := xs[0] + \"b\"\nxs",
            "[\"ab\"]",
        ),
    ] {
        assert_eq!(shown(text), expected, "{text}");
    }
}

#[test]
fn an_index_update_leaves_the_old_list_to_its_other_holders() {
    // Another variable, a closure's capture, a record's field and a
    // future's capture all still see the old elements.
    let text = "let mut xs = [1, 2]\nlet old = xs\nlet f = fn(): List<Int> { xs }\n\
                    let r = { held = xs }\nlet later = lazy xs\nxs[0] := 10\n\
                    \"{xs} {old} {f()} {r.held} {await later}\"";
    assert_eq!(string(text), "[10, 2] [1, 2] [1, 2] [1, 2] [1, 2]");
    // The new list shares every element it did not replace, and the old
    // list is the one the other variable holds.
    let frame =
        frame_after("let mut xs = [{ a = 1 }, { a = 2 }]\nlet old = xs\nxs[1] := { a = 3 }");
    let [Value::List(xs), Value::List(old), ..] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(same(&xs.values()[0], &old.values()[0]));
    assert!(!same(&xs.values()[1], &old.values()[1]));
    assert_eq!(old.values()[1].to_string(), "{a = 2}");
    // A record along the path shares its other fields.
    let frame =
        frame_after("let mut r = { xs = [1]; other = { n = 1 } }\nlet old = r\nr.xs[0] := 2");
    let [Value::Record(r), Value::Record(old)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(same(&r.values[1], &old.values[1]));
    assert_eq!(old.values[0].to_string(), "[1]");
}

#[test]
fn a_position_outside_the_list_fails_and_changes_nothing() {
    for (text, label, path, before) in [
        (
            "let mut xs = [1, 2]\nxs[2] := 5",
            "the index is 2, and the list has 2 elements",
            "xs[2]",
            "[1, 2]",
        ),
        (
            "let mut xs = [1, 2]\nxs[-1] := 5",
            "the index is -1",
            "xs[-1]",
            "[1, 2]",
        ),
        (
            "let mut xs: Int[] = []\nxs[0] := 5",
            "the index is 0, and the list has 0 elements",
            "xs[0]",
            "[]",
        ),
        (
            "let mut xs = [{ a = [1] }]\nxs[1].a[0] := 5",
            "the index is 1, and the list has 1 element",
            "xs[1]",
            "[{a = [1]}]",
        ),
        (
            "let mut xs = [{ a = [1] }]\nxs[0].a[3] := 5",
            "the index is 3, and the list has 1 element",
            "xs[0].a[3]",
            "[{a = [1]}]",
        ),
    ] {
        let ir = compile(text);
        let mut frame = vec![Value::Empty; ir.frame_size() as usize];
        let diagnostic = eval_in(&ir, &mut frame).unwrap_err();
        assert_eq!(diagnostic.message, "index out of range", "{text}");
        assert_eq!(diagnostic.label.as_deref(), Some(label), "{text}");
        assert_eq!(diagnostic.span, at(text, path), "{text}");
        assert!(diagnostic.labels.is_empty(), "{text}");
        // The variable is left as it was.
        assert_eq!(frame[0].to_string(), before, "{text}");
    }
    // Appending is by spread, not by index.
    assert_eq!(shown("let mut xs = [1]\nxs := [...xs, 2]\nxs"), "[1, 2]");
}

#[test]
fn an_update_evaluates_its_value_first_and_each_position_once_in_order() {
    // Blocks that rebind a mutable variable of the top level as they
    // run show the order: the value, once, and then each position, once,
    // left to right.
    let text = "let mut log = \"\"\nlet mut xs = [[0, 0], [0, 0]]\n\
                    xs[{ log := log + \"i\"; 1 }][{ log := log + \"j\"; 0 }] := \
                    { log := log + \"v\"; 5 }\n\"{log} {xs}\"";
    assert_eq!(string(text), "vij [[0, 0], [5, 0]]");
    // The variable is read after the value, so a value that rebinds it
    // is updated as it then is, for `:=` and a dotted `let` alike.
    assert_eq!(
        shown("let mut r = { a = 1; b = 2 }\nr.a := { r := { a = 100; b = 7 }; r.b + 1 }\nr"),
        "{a = 8; b = 7}"
    );
    assert_eq!(
        shown("let mut r = { a = 1; b = 2 }\nlet r.a = { r := { a = 100; b = 7 }; r.b + 1 }\nr"),
        "{a = 8; b = 7}"
    );
    // A position that rebinds it runs after it is read, and the store
    // overwrites what it stored.
    assert_eq!(
        shown("let mut xs = [1, 2]\nxs[{ xs := [7, 8, 9]; 1 }] := 5\nxs"),
        "[1, 5]"
    );
    // A value that fails leaves the variable as it was, before anything
    // of the path is read.
    for text in [
        "let mut xs = [1]\nxs[0] := 1 // 0",
        "let mut xs = [1]\nxs[1] := 1 // 0",
    ] {
        let ir = compile(text);
        let mut frame = vec![Value::Empty; ir.frame_size() as usize];
        let diagnostic = eval_in(&ir, &mut frame).unwrap_err();
        assert_eq!(diagnostic.message, "division by zero", "{text}");
        assert_eq!(frame[0].to_string(), "[1]", "{text}");
    }
}

#[test]
fn a_future_along_the_path_is_awaited_after_the_value() {
    // The value's effects come first: here it replaces the future that
    // would fail with one that does not, which is the one awaited.
    let text = "let mut r: {a: Int, b: Int} | Future<{a: Int, b: Int}> = \
                    lazy { a = 1 // 0; b = 2 }\n\
                    r.a := { r := lazy { a = 7; b = 8 }; 1 }\nr";
    assert_eq!(shown(text), "{a = 1; b = 8}");
    // A value that fails fails before the future is awaited, which stays
    // unrun.
    let text = "let mut r: {a: Int} | Future<{a: Int}> = lazy { a = 1 }\nr.a := 2 // 0";
    let ir = compile(text);
    let mut frame = vec![Value::Empty; ir.frame_size() as usize];
    let diagnostic = eval_in(&ir, &mut frame).unwrap_err();
    assert_eq!(diagnostic.span, at(text, "//"));
    let Value::Future(future) = &frame[0] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(!future.is_done());
    // An element along the path likewise, awaited by its step, at the
    // path up to it.
    let text = "let mut xs: List<{a: Int} | Future<{a: Int}>> = [lazy { a = 1 // 0 }]\n\
                    xs[0].a := { xs := [{ a = 5 }]; 6 }\nxs";
    assert_eq!(shown(text), "[{a = 6}]");
    let text = "let xs: List<{a: Int} | Future<{a: Int}>> = [lazy { a = 1 // 0 }]\n\
                    xs[0].a := 2";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "division by zero");
    assert_eq!(
        labels(&diagnostic).last(),
        Some(&(at(text, "xs[0]"), "awaited here"))
    );
}

#[test]
fn an_element_that_is_a_future_is_awaited_only_on_the_way() {
    // Replaced at the leaf, it is not read, and never runs.
    let frame = frame_after("let xs = [lazy 1 // 0]\nlet old = xs\nxs[0] := lazy 2");
    let [Value::List(xs), Value::List(old), Value::List(new)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(xs.shares(old));
    let (Value::Future(replaced), Value::Future(_)) = (&old.values()[0], &new.values()[0]) else {
        panic!("expected futures");
    };
    assert!(!replaced.is_done());
    assert_eq!(
        int("let xs = [lazy 1 // 0]\nxs[0] := lazy 2\nawait xs[0]"),
        2
    );
    // Gone through, it is awaited once: a later await of the same future
    // finds the record that one run built, which the update shares its
    // untouched field with.
    let frame = frame_after(
        "let xs: List<{a: Int, b: {c: Int}} | Future<{a: Int, b: {c: Int}}>> = \
             [lazy { a = 1; b = { c = 2 } }]\nlet old = xs\nxs[0].a := 5\n\
             let s = await old[0]",
    );
    let [
        _,
        Value::List(old),
        Value::List(updated),
        Value::Record(read),
    ] = &frame[..]
    else {
        panic!("unexpected frame {frame:?}");
    };
    let Value::Future(future) = &old.values()[0] else {
        panic!("expected a future");
    };
    assert!(future.is_done());
    let Value::Record(element) = &updated.values()[0] else {
        panic!("expected a record");
    };
    assert_eq!(element.values[0], Value::Int(5));
    assert!(same(&element.values[1], &read.values[1]));
}

#[test]
fn a_long_path_and_a_deep_position_use_the_nesting_budget() {
    on_big_stack(|| {
        // A function whose body is as tall as `levels`, from a dotted
        // `let` of a long path, a deep position, or a chain of the same
        // height, called from the top level: each fails exactly where
        // the others do.
        let path =
            |steps: usize| format!("fn f(): Int {{ let a{} = 1\n0 }}\nf()", ".b".repeat(steps));
        let position = |terms: usize| {
            format!(
                "fn f(): Int {{ let mut xs = [0]\nxs[0{}] := 1\n0 }}\nf()",
                " + 0".repeat(terms - 1)
            )
        };
        let chain = |terms: usize| {
            format!(
                "fn f(): Int {{ let mut xs = [0]\nlet x = 0{}\n0 }}\nf()",
                " + 0".repeat(terms - 1)
            )
        };
        let height = |text: &str| compile(text).function(FnId::new(0)).height as usize;
        // The update is two levels taller than its tallest part for each
        // step, and its store and the body's block two more.
        assert_eq!(height(&path(10)), 2 * 10 + 4);
        assert_eq!(height(&position(10)), 10 + 5);
        assert_eq!(height(&chain(13)), 13 + 2);
        // The call enters `f`'s frame at 1 + CALL_LEVELS levels.
        let room = MAX_DEPTH - 1 - CALL_LEVELS;
        let steps = (room - 4) / 2;
        let terms = room - 5;
        for (within, past) in [
            (path(steps), path(steps + 1)),
            (position(terms), position(terms + 1)),
            (chain(terms + 3), chain(terms + 4)),
        ] {
            assert_eq!(run(&within).unwrap(), Value::Int(0));
            let diagnostic = run(&past).unwrap_err();
            assert_eq!(diagnostic.message, BUDGET_ERROR);
            let call = u32::try_from(past.len()).unwrap();
            assert_eq!(diagnostic.span, Span::new(call - 3, call));
        }
    });
}

#[test]
fn a_loop_of_updates_uses_no_budget_per_iteration() {
    // Many more iterations than the budget has levels, each updating an
    // element and a record's field through a path.
    let text = "fn f(n: Int): Int {\n\
                    \x20 let mut xs = [0, 0]\n\
                    \x20 let mut r = { counts = [0] }\n\
                    \x20 for i in 0..n { xs[i % 2] := i; r.counts[0] := r.counts[0] + 1 }\n\
                    \x20 xs[0] + xs[1] + r.counts[0]\n\
                    }\n";
    let n = 3 * MAX_DEPTH;
    let expected = i64::try_from(n - 2 + n - 1 + n).unwrap();
    assert_eq!(int(&format!("{text}f({n})")), expected);
    // The function's height does not depend on the count.
    let height = |count: usize| {
        compile(&format!("{text}f({count})"))
            .function(FnId::new(0))
            .height
    };
    assert_eq!(height(1), height(n));
}
