//! Tests of the evaluator: lists, spreads, ranges and `for`.

use super::*;

#[test]
fn lists_are_built_indexed_and_measured() {
    let text = "let xs: List<Int> = [1, 2, 3]\n\
                    let ys: Int[] = [...xs, 4]\n\
                    a = xs[0]\n\
                    b = len(ys)\n\
                    c = xs == [1, 2, 3]\n\
                    d = \"{ys}\"";
    assert_eq!(
        run(text).unwrap().to_string(),
        "{a = 1; b = 4; c = true; d = \"[1, 2, 3, 4]\"}"
    );
    assert_eq!(run("[]: Int[]").unwrap(), Value::List(List::EMPTY));
    assert_eq!(int("len([]: Int[])"), 0);
    assert_eq!(int("[[1, 2], [3]][1][0]"), 3);
    assert_eq!(float("[1, 2.5][0]"), 1.0);
    assert_eq!(string("[\"a\", \"b\"][1]"), "b");
    assert!(boolean("[true][0]"));
    assert_eq!(int("[{ a = 7 }][0].a"), 7);
    assert_eq!(int("[fn(x: Int): Int { x * 2 }][0](21)"), 42);
    assert_eq!(int("[lazy 5][0] + 1"), 6);
    assert_eq!(run("[{}][0]").unwrap(), Value::Empty);
    assert_eq!(run("[[1]][0]").unwrap().to_string(), "[1]");
    // Spreads, nested and empty.
    assert_eq!(
        run("let e: Int[] = []\n[...e, 1, ...[2, 3], ...e, ...[...[4]]]")
            .unwrap()
            .to_string(),
        "[1, 2, 3, 4]"
    );
    assert_eq!(
        run("let e: Int[] = []\n[...e]").unwrap(),
        Value::List(List::EMPTY)
    );
}

#[test]
fn a_list_evaluates_its_elements_in_order() {
    // The first element that fails is reported, and nothing after it
    // is evaluated.
    assert_eq!(
        error("[1, 1 // 0, 2 // 0]"),
        ("division by zero".to_owned(), Span::new(6, 8))
    );
    assert_eq!(
        error("[1, ...[1 // 0], 2 // 0]"),
        ("division by zero".to_owned(), Span::new(10, 12))
    );
    assert_eq!(
        error("[...[1 // 0], ...[2 // 0]]"),
        ("division by zero".to_owned(), Span::new(7, 9))
    );
    // The list, then the position.
    assert_eq!(
        error("[1 // 0][2 // 0]"),
        ("division by zero".to_owned(), Span::new(3, 5))
    );
    assert_eq!(
        error("[1][2 // 0]"),
        ("division by zero".to_owned(), Span::new(6, 8))
    );
}

#[test]
fn an_index_out_of_range_is_a_runtime_error() {
    for (text, label) in [
        (
            "[1, 2, 3][5]",
            "the index is 5, and the list has 3 elements",
        ),
        (
            "[1, 2, 3][3]",
            "the index is 3, and the list has 3 elements",
        ),
        ("[1][1]", "the index is 1, and the list has 1 element"),
        ("[1, 2, 3][-1]", "the index is -1"),
        (
            "([]: Int[])[0]",
            "the index is 0, and the list has 0 elements",
        ),
    ] {
        let diagnostic = run(text).unwrap_err();
        assert_eq!(diagnostic.message, "index out of range", "{text}");
        // At the whole index.
        let whole = Span::new(0, u32::try_from(text.len()).unwrap());
        assert_eq!(diagnostic.span, whole, "{text}");
        assert_eq!(diagnostic.label.as_deref(), Some(label), "{text}");
        assert!(diagnostic.labels.is_empty(), "{text}");
    }
    // Inside a function, and inside a future, where it happens.
    let text = "fn f(xs: Int[]): Int { xs[3] }\nf([1])";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.span, Span::new(23, 28));
    let text = "let x = lazy [1][9]\nawait x";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.span, Span::new(13, 19));
    assert_eq!(
        labels(&diagnostic),
        vec![(Span::new(20, 27), "awaited here")]
    );
}

#[test]
fn a_spread_shares_the_elements_of_its_list() {
    let frame = frame_after("let xs = [{ a = 1 }, { a = 2 }]\nlet ys = [...xs, ...xs]");
    let [Value::List(xs), Value::List(ys)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert_eq!(ys.len(), 4);
    assert!(same(&xs.values()[0], &ys.values()[0]));
    assert!(same(&xs.values()[1], &ys.values()[1]));
    assert!(same(&xs.values()[0], &ys.values()[2]));
    // An element read by an index is the element, not a copy.
    let frame = frame_after("let xs = [{ a = 1 }]\nlet r = xs[0]");
    let [Value::List(xs), r] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(same(&xs.values()[0], r));
}

#[test]
fn a_list_grown_by_spreads_shares_and_keeps_its_order() {
    // A chain of spreads, as transitive file sets are built: each step joins
    // the list so far, sharing it, rather than copying it.
    let frame = frame_after(
        "let mut xs: List<Int> = []\n\
         for i in 0..3000 { xs := [...xs, i] }\n\
         let ys = [...xs, ...xs]\n\
         let n = xs[2999] + ys[5999]",
    );
    let [Value::List(xs), .., Value::List(ys), Value::Int(n)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert_eq!(xs.len(), 3000);
    assert_eq!(*n, 2999 + 2999);
    let expected: Vec<Value> = (0..3000).chain(0..3000).map(Value::Int).collect();
    assert_eq!(ys.values(), &expected[..]);
    // An update replaces one element of the rope and leaves the old list.
    let frame = frame_after(
        "let mut xs: List<Int> = []\n\
         for i in 0..100 { xs := [...xs, i] }\n\
         let old = xs\n\
         xs[50] := -1\n\
         let r = old[50] + xs[50]",
    );
    let [.., Value::Int(r)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert_eq!(*r, 49);
}

#[test]
fn a_deep_await_of_a_list_runs_each_future_once() {
    let frame = frame_after(
        "let a = lazy { n = 1 }\n\
             let xs = [a, a, lazy { n = 2 }, { n = 3 }]\n\
             let s = \"{xs}\"\n\
             let t = \"{xs}\"",
    );
    let [Value::Future(a), Value::List(xs), s, t] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert_eq!(s.to_string(), "[{n = 1}, {n = 1}, {n = 2}, {n = 3}]");
    assert_eq!(s, t);
    // The list holds the futures, each run by the first await and found
    // done by every later one; the one future `a` is both elements.
    assert!(a.is_done());
    let [
        Value::Future(first),
        Value::Future(second),
        Value::Future(third),
        Value::Record(_),
    ] = xs.values()
    else {
        panic!("unexpected list {xs:?}");
    };
    assert!(Rc::ptr_eq(a, first) && Rc::ptr_eq(a, second));
    assert!(third.is_done());
    // A deep await of a record's list and a list's records.
    assert_eq!(
        string("let r = { xs = [lazy 1, 2] }\n\"{r}\""),
        "{xs = [1, 2]}"
    );
    assert_eq!(
        string("let xs = [{ a = lazy [lazy 1] }]\n\"{xs}\""),
        "[{a = [1]}]"
    );
    // An awaited list with nothing lazy is the list as it was.
    let frame = frame_after("let xs = [1, lazy 2]\nlet ys = [...xs]: List<Int | Future<Int>>");
    assert_eq!(frame[1].to_string(), "[1, Future<Int>]");
}

#[test]
fn list_equality_of_both_kinds() {
    booleans(&[
        ("[1, 2] == [1, 2]", true),
        ("[1, 2] == [1, 3]", false),
        ("[1, 2] == [1, 2, 3]", false),
        ("[1, 2] != [2, 1]", true),
        ("[1, 2] === [1, 2]", true),
        ("[1, 2] !== [1, 2, 3]", true),
        ("[[1], [2, 3]] == [[1], [2, 3]]", true),
        ("[[1], [2, 3]] == [[1], [2]]", false),
        ("[[1], [2, 3]] === [[1], [3, 2]]", false),
        ("[0.0] == [-0.0]", true),
        ("[0.0 / 0.0] == [0.0 / 0.0]", false),
        ("let e: Int[] = []\ne == e", true),
        (
            "let xs = [1, lazy 2]\nxs == [1, 2]: List<Int | Future<Int>>",
            true,
        ),
        ("[{ a = [1] }] == [{ a = [1] }]", true),
        ("[{ a = [1] }] == [{ a = [2] }]", false),
    ]);
    // Typed equality compares records inside by their static type;
    // total equality compares whole values.
    let hidden = "let p: {x: Int} = { x = 1; y = 2 }\n\
                      let q: {x: Int} = { x = 1; y = 3 }\n";
    booleans(&[
        (&format!("{hidden}[p] == [q]"), true),
        (&format!("{hidden}[p] === [q]"), false),
        (&format!("{hidden}[[p]] == [[q]]"), true),
        (&format!("{hidden}{{ xs = [p] }} == {{ xs = [q] }}"), true),
        (&format!("{hidden}{{ xs = [p] }} === {{ xs = [q] }}"), false),
    ]);
}

#[test]
fn lists_display_their_elements() {
    assert_eq!(
        string("\"{[1, 2]} {[\"a\"]} {[[1.5], []: Float[]]} {[{ n = 1 }]}\""),
        "[1, 2] [\"a\"] [[1.5], []] [{n = 1}]"
    );
    assert_eq!(
        string("\"{[fn(): Int { 1 }]} {[lazy 1]}\""),
        "[fn(): Int] [1]"
    );
}

#[test]
fn lists_are_memo_keys() {
    assert!(hits("", "Int[]", "[1, 2]", "[1, 2]"));
    assert!(hits("", "Int[]", "[1, 2]", "[...[1], 2]"));
    assert!(hits("", "Int[]", "[]", "[]"));
    assert!(!hits("", "Int[]", "[1, 2]", "[1, 2, 3]"));
    assert!(!hits("", "Int[]", "[1, 2]", "[2, 1]"));
    assert!(!hits("", "Int[]", "[]", "[1]"));
    assert!(hits("", "Float[]", "[0.0]", "[-0.0]"));
    assert!(!hits("", "Float[]", "[0.0 / 0.0]", "[0.0 / 0.0]"));
    // Deeply, deeper than the hash looks, and by total equality.
    let deep = |leaf: i64| format!("{}{leaf}{}", "[".repeat(6), "]".repeat(6));
    let deep_type = format!("Int{}", "[]".repeat(6));
    assert!(hits("", &deep_type, &deep(1), &deep(1)));
    assert!(!hits("", &deep_type, &deep(1), &deep(2)));
    assert!(hits("", "{a: Int}[]", "[{ a = 1 }]", "[{ a = 1 }]"));
    assert!(!hits("", "{a: Int}[]", "[{ a = 1 }]", "[{ a = 1; b = 2 }]"));
    // Elements that may be futures are awaited at the call first.
    assert!(hits("let x = lazy 1", "Int[]", "[x]", "[1]"));
}

#[test]
fn a_list_is_captured_by_a_closure_and_a_lazy() {
    assert_eq!(
        int("let xs = [1, 2]\nlet f = fn(): Int { xs[1] }\nlet xs = [5]\nf()"),
        2
    );
    assert_eq!(
        int("let xs = [1, 2]\nlet f = lazy len(xs)\nxs := [5]\nawait f"),
        2
    );
    assert_eq!(
        int("let n = 3\nlet f = fn(): Int[] { [n, n] }\nlen(f()) + f()[0]"),
        5
    );
}

/// The value of `text`, as it would be written.
pub(super) fn shown(text: &str) -> String {
    run(text)
        .unwrap_or_else(|error| panic!("{text}: {error:?}"))
        .to_string()
}

#[test]
fn a_spread_copies_the_fields_of_its_static_type() {
    assert_eq!(
        shown(
            "let defaults = { host = \"localhost\"; port = 80 }\n\
                 let conf = {\n  ...defaults\n  port = 8080\n  url = \"{host}:{port}\"\n}\n\
                 conf"
        ),
        "{host = \"localhost\"; port = 8080; url = \"localhost:8080\"}"
    );
    // The spread's fields take their places in the order of their
    // names' text, whatever order the names were first met in, and an
    // overridden field keeps its place.
    assert_eq!(
        shown("{ b = 0; ...{ a = 1; c = 2 }; a = \"s\" }"),
        "{b = 0; a = \"s\"; c = 2}"
    );
    assert_eq!(
        shown("let zed = 1\nlet beta = 2\nlet alpha = 3\n{ ...{ zed = 1; beta = 2; alpha = 3 } }"),
        "{alpha = 3; beta = 2; zed = 1}"
    );
    assert_eq!(
        shown("{ ...{ a = 1; b = 2 }; ...{ b = \"s\"; c = 3 } }"),
        "{a = 1; b = \"s\"; c = 3}"
    );
    // A field the static type hides is not copied, and an ascription
    // selects.
    assert_eq!(
        shown("let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\n{ ...p }"),
        "{x = 1}"
    );
    assert_eq!(
        shown("let r = { a = 2; b = 1 }\n{ ...r : {b: Int} }"),
        "{b = 1}"
    );
    assert_eq!(
        shown("let r = { a = 1; b = lazy 2 }\n{ ...r : {b: Int} }"),
        "{b = 2}"
    );
    // Between a spread and an override the name is the spread's.
    assert_eq!(
        shown("{ ...{ a = 1 }; b = a; a = 2; c = a }"),
        "{a = 2; b = 1; c = 2}"
    );
    // `{}` spreads nothing.
    assert_eq!(shown("{ ...{}; a = 1 }"), "{a = 1}");
    assert_eq!(shown("{ ...{} }"), "{}");
}

#[test]
fn a_spread_evaluates_its_operand_once() {
    // Its fields are read from one value: a future the operand makes is
    // one future in both fields.
    let frame = frame_after("let s = { ...{ let p = lazy 1\na = p; b = p } }");
    let [Value::Record(s), ..] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(same(&s.values[0], &s.values[1]));
    // A field is shared with the operand's record, not copied.
    let frame = frame_after("let r = { a = { x = 1 }; b = 2 }\nlet s = { ...r }");
    let [Value::Record(r), Value::Record(s), ..] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(same(&r.values[0], &s.values[0]));
    // A future operand runs once: a later await of it finds the record
    // the spread read its field from.
    let frame = frame_after("let f = lazy { a = { n = 1 } }\nlet s = { ...f }\nlet t = await f");
    let [Value::Future(f), Value::Record(s), Value::Record(t)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(f.is_done());
    assert!(same(&s.values[0], &t.values[0]));
    // One that fails fails where the spread awaits it.
    let text = "let f = lazy { a = 1 // 0 }\n{ ...f }";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "division by zero");
    assert_eq!(diagnostic.span, Span::new(21, 23));
    assert_eq!(
        labels(&diagnostic).last(),
        Some(&(Span::new(33, 34), "awaited here"))
    );
}

/// The span of the first occurrence of `needle` in `text`.
pub(super) fn at(text: &str, needle: &str) -> Span {
    let start = u32::try_from(text.find(needle).unwrap()).unwrap();
    Span::new(start, start + u32::try_from(needle.len()).unwrap())
}

#[test]
fn ranges_are_lists_of_their_integers() {
    assert_eq!(shown("0..5"), "[0, 1, 2, 3, 4]");
    assert_eq!(shown("0..=5"), "[0, 1, 2, 3, 4, 5]");
    assert_eq!(shown("-3..=-1"), "[-3, -2, -1]");
    assert_eq!(shown("-2..2"), "[-2, -1, 0, 1]");
    assert_eq!(shown("4..=4"), "[4]");
    assert_eq!(shown("0..=0"), "[0]");
    // Empty, and reversed, which is empty too.
    for text in ["3..3", "5..2", "5..=4", "0..-1"] {
        assert_eq!(run(text), Ok(Value::List(List::EMPTY)), "{text}");
    }
    // At the ends of `Int`, without overflowing.
    assert_eq!(
        shown(&format!("{MAX} - 1..={MAX}")),
        "[9223372036854775806, 9223372036854775807]"
    );
    assert_eq!(shown(&format!("{MAX} - 1..{MAX}")), "[9223372036854775806]");
    assert_eq!(
        shown(&format!("{MIN}..{MIN} + 2")),
        "[-9223372036854775808, -9223372036854775807]"
    );
    assert_eq!(shown(&format!("{MIN}..={MIN}")), "[-9223372036854775808]");
    assert_eq!(run(&format!("{MAX}..{MIN}")), Ok(Value::List(List::EMPTY)));
    assert_eq!(run(&format!("{MIN}..{MIN}")), Ok(Value::List(List::EMPTY)));
    // Read as any list is.
    assert_eq!(int("len(0..100)"), 100);
    assert_eq!(int("(10..20)[3]"), 13);
    assert!(boolean("0..3 == [0, 1, 2]"));
    assert_eq!(shown("[...0..2, ...5..=6]"), "[0, 1, 5, 6]");
    // Its bounds are evaluated once, left to right.
    let text = "(1 // 0)..(2 // 0)";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    let text = "let n = lazy 3\n0..n";
    assert_eq!(shown(text), "[0, 1, 2]");
}

/// The note of the error for a range used as a list that is too long.
pub(super) const RANGE_NOTE: &str = "a range used as a list holds every element, and a list cannot hold \
                              more than 384307168202282325. A `for` over a range directly builds \
                              no list";

#[test]
fn a_range_used_as_a_list_is_limited_only_by_what_a_list_can_hold() {
    // The most elements an allocation of `isize::MAX` bytes holds.
    assert_eq!(MAX_LIST_LEN, 384_307_168_202_282_325);
    for (text, len) in [
        (format!("0..{MAX}"), "9223372036854775807"),
        ("0..=384307168202282325".to_owned(), "384307168202282326"),
        (format!("{MIN}..{MAX}"), "18446744073709551615"),
        (format!("{MIN}..={MAX}"), "18446744073709551616"),
    ] {
        let diagnostic = run(&text).unwrap_err();
        assert_eq!(diagnostic.message, "range too large", "{text}");
        // At the whole range.
        let whole = Span::new(0, u32::try_from(text.len()).unwrap());
        assert_eq!(diagnostic.span, whole, "{text}");
        assert_eq!(
            diagnostic.label.as_deref(),
            Some(format!("this range has {len} elements").as_str()),
            "{text}"
        );
        assert!(diagnostic.labels.is_empty(), "{text}");
        assert_eq!(diagnostic.notes, vec![RANGE_NOTE], "{text}");
    }
    // Wherever it is used as a value, in a function too.
    let text = format!("fn f(n: Int): Int {{ len(0..n) }}\nf({MAX})");
    let diagnostic = run(&text).unwrap_err();
    assert_eq!(diagnostic.message, "range too large");
    assert_eq!(diagnostic.span, at(&text, "0..n"));
    // A range in the iterable that is not directly it is built.
    let text = format!("for i in [0..{MAX}][0] {{ i }}");
    assert_eq!(
        error(&text),
        (
            "range too large".to_owned(),
            at(&text, &format!("0..{MAX}"))
        )
    );
    // Below that there is no limit: a list of any size a program asks
    // for is built.
    assert_eq!(shown("len(0..1000000)"), "1000000");
}

#[test]
fn a_for_collects_its_bodys_values() {
    for (text, expected) in [
        ("for x in [1, 2, 3] { x * 2 }", "[2, 4, 6]"),
        ("for x in [1, 2, 3, 4] if x % 2 == 0 { x }", "[2, 4]"),
        ("for i in 0..5 { i * i }", "[0, 1, 4, 9, 16]"),
        ("for i in 0..=4 if i != 2 { i }", "[0, 1, 3, 4]"),
        ("for x in [\"a\", \"b\"] { \"{x}!\" }", "[\"a!\", \"b!\"]"),
        ("for x in [1.5] { x * 2.0 }", "[3.0]"),
        ("for x in [true, false] { not x }", "[false, true]"),
        ("for x in [1] { a = x; b = x + 1 }", "[{a = 1; b = 2}]"),
        ("for x in [1, 2] { }", "[{}, {}]"),
        (
            "for x in [[1, 2], [3]] { for y in x { y * 10 } }",
            "[[10, 20], [30]]",
        ),
        ("for x in for y in [1, 2] { y + 1 } { x * 2 }", "[4, 6]"),
        ("for f in [fn(x: Int): Int { x + 1 }] { f(1) }", "[2]"),
        ("for x in [1, 2] { let y = x * 3\n= y + 1 }", "[4, 7]"),
        // With lazy elements, the variable is the element as it is.
        ("for x in [lazy 1, 2] { x + 1 }", "[2, 3]"),
        // Checked against an expected element type, and awaited each
        // iteration to fit it.
        ("let ys: Float[] = for x in [1, 2] { 1 }\nys", "[1.0, 1.0]"),
        (
            "let xs = [lazy 1, lazy 2]\nlet ys: List<Present<Int>> = for x in xs { x }\nys",
            "[1, 2]",
        ),
    ] {
        assert_eq!(shown(text), expected, "{text}");
    }
    // Empty: an empty list, a guard that keeps nothing, an empty range.
    for text in [
        "for x in []: Int[] { x }",
        "for x in [1, 2] if false { x }",
        "for i in 3..0 { i }",
    ] {
        assert_eq!(run(text), Ok(Value::List(List::EMPTY)), "{text}");
    }
    // Read by every kind of use.
    assert_eq!(
        int("len(for x in [1, 2] { x }) + (for x in [5] { x })[0]"),
        7
    );
    assert!(boolean("for x in [1, 2] { x } == [1, 2]"));
    assert_eq!(string("let ys = for x in [lazy 1] { x }\n\"{ys}\""), "[1]");
    assert_eq!(shown("[...for x in [1] { x }, 2]"), "[1, 2]");
}

#[test]
fn a_for_runs_in_order_and_stops_at_the_first_error() {
    // The body, element by element: the first that fails is reported.
    let text = "for f in [fn(): Int { 1 // 0 }, fn(): Int { 2 // 0 }] { f() }";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    let text = "for x in [3, 2, 1, 0] { 6 // (x - 1) }";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    // The iterable first, then each element's guard, then its body.
    let text = "for x in [1 // 0] { 2 // 0 }";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    let text = "for x in [1, 0] if 1 // x > 0 { 2 // 0 }";
    let body = at(text, "2 //").start + 2;
    assert_eq!(
        error(text),
        ("division by zero".to_owned(), Span::new(body, body + 2))
    );
    // The body only where the guard holds.
    assert_eq!(shown("for x in [0, 1] if x > 0 { 1 // x }"), "[1]");
    // The iterable is evaluated once: both elements are the one future
    // that its single evaluation made.
    let frame = frame_after("let ys = for x in { let a = lazy 5\n= [a, a] } { x }");
    let [Value::List(ys)] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    let [Value::Future(first), Value::Future(second)] = ys.values() else {
        panic!("unexpected list {ys:?}");
    };
    assert!(Rc::ptr_eq(first, second));
}

#[test]
fn a_closure_made_in_the_body_captures_that_iterations_element() {
    assert_eq!(
        int("let fs = for x in [1, 2, 3] { fn(): Int { x } }\n\
                 fs[0]() + 10 * fs[1]() + 100 * fs[2]()"),
        321
    );
    assert_eq!(
        int("let ls = for i in 1..=3 { lazy i * i }\n\
                 await ls[0] + 10 * await ls[1] + 100 * await ls[2]"),
        941
    );
    assert_eq!(
        shown(
            "let fs = for x in [1, 2, 3, 4] if x % 2 == 1 { fn(y: Int): Int { x + y } }\n\
                 for f in fs { f(10) }"
        ),
        "[11, 13]"
    );
}

#[test]
fn a_for_over_a_range_does_not_build_it() {
    // Past the limit of a range used as a list, directly and in
    // parentheses.
    assert_eq!(
        shown("for i in 0..16777220 if i > 16777217 { i }"),
        "[16777218, 16777219]"
    );
    assert_eq!(shown("for i in (0..=16777217) if i == 3 { i }"), "[3]");
    // Up to the largest Int, included, without overflowing its count,
    // and from the smallest.
    assert_eq!(
        shown(&format!("for i in {MAX} - 2..={MAX} {{ i - {MAX} }}")),
        "[-2, -1, 0]"
    );
    assert_eq!(
        shown(&format!("for i in {MAX} - 2..{MAX} {{ i - {MAX} }}")),
        "[-2, -1]"
    );
    assert_eq!(
        shown(&format!("for i in {MIN}..={MIN} + 1 {{ i - {MIN} }}")),
        "[0, 1]"
    );
    assert_eq!(
        run(&format!("for i in {MAX}..={MIN} {{ i }}")),
        Ok(Value::List(List::EMPTY))
    );
    // Its bounds are evaluated once, before any element.
    let text = "for i in 0..(1 // 0) { 2 // 0 }";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    // As a statement, it collects nothing.
    assert_eq!(int("for i in 0..16777220 { }\n1"), 1);
}

#[test]
fn a_unit_like_statement_runs_for_its_effects() {
    // A `for` whose values are `{}` runs its body, and drops them.
    assert_eq!(int("for x in [1, 2] { }\n7"), 7);
    let text = "for x in [1, 0] { let y = 1 // x }\n1";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    // A statement of futures awaits each deeply, once, in order.
    let frame =
        frame_after("let a = lazy {}\nlet steps = [a, a, lazy {}]\nfor s in steps { s }\n1");
    let [Value::Future(a), Value::List(steps), ..] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(a.is_done());
    assert!(
        steps
            .values()
            .iter()
            .all(|step| matches!(step, Value::Future(future) if future.is_done()))
    );
    let frame = frame_after("let steps = [lazy {}, lazy {}]\nfor s in steps { lazy await s }\n1");
    let [Value::List(steps), ..] = &frame[..] else {
        panic!("unexpected frame {frame:?}");
    };
    assert!(
        steps
            .values()
            .iter()
            .all(|step| matches!(step, Value::Future(future) if future.is_done()))
    );
    let text = "let steps = [lazy { let a = 1 }, lazy { let b = 1 // 0 }]\nfor s in steps { s }\n1";
    assert_eq!(error(text), ("division by zero".to_owned(), at(text, "//")));
    // Other unit-like lists.
    assert_eq!(int("[]\n[{}]\n[[{}, {}]]\n3"), 3);
}

#[test]
fn iterations_do_not_consume_the_nesting_budget() {
    on_big_stack(|| {
        // Far more calls than the budget has levels, one an iteration,
        // each at the same depth.
        assert_eq!(
            int("fn id(x: Int): Int { x }\nlen(for i in 0..20000 { id(i) })"),
            20000
        );
        // A recursion through a `for` in each frame is charged for the
        // `for`'s body once a call.
        assert_eq!(
            int("fn f(n: Int): Int {\n\
                     \x20 if n == 0 { 0 } else {\n\
                     \x20   len(for i in 0..3 if i == 0 { f(n - 1) }) + f(n - 1)\n\
                     \x20 }\n\
                     }\n\
                     f(12)"),
            12
        );
    });
}
