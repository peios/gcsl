//! Tests of the evaluator: comparisons, records, top-level functions and defs,
//! logic, and strings.

use super::*;

#[test]
fn int_comparisons() {
    booleans(&[
        ("1 == 1", true),
        ("1 == 2", false),
        ("1 != 2", true),
        ("1 != 1", false),
        ("1 < 2", true),
        ("2 < 1", false),
        ("1 < 1", false),
        ("1 <= 1", true),
        ("2 <= 1", false),
        ("2 > 1", true),
        ("1 > 1", false),
        ("1 >= 1", true),
        ("1 >= 2", false),
        ("-5 < 3", true),
        ("(-9223372036854775807 - 1) < 9223372036854775807", true),
        ("let x = 7\nx // 2 == 3", true),
    ]);
}

#[test]
fn float_comparisons_follow_ieee_754() {
    booleans(&[
        ("1.5 == 1.5", true),
        ("1.5 != 1.5", false),
        ("1.5 < 2.5", true),
        ("2.5 <= 2.5", true),
        ("2.5 > 1", true),
        ("1 >= 1.5", false),
        ("0.1 + 0.2 == 0.3", false),
        ("-0.0 == 0.0", true),
        ("-0.0 != 0.0", false),
        ("-0.0 < 0.0", false),
        ("-0.0 >= 0.0", true),
        ("1.0 / 0.0 > 1e308", true),
        ("-1.0 / 0.0 < -1e308", true),
    ]);
    // NaN is unequal and unordered to everything, itself included.
    let nan = "let nan = 0.0 / 0.0\n";
    for (comparison, expected) in [
        ("nan == nan", false),
        ("nan != nan", true),
        ("nan < nan", false),
        ("nan <= nan", false),
        ("nan > nan", false),
        ("nan >= nan", false),
        ("nan == 1.0", false),
        ("nan != 1.0", true),
        ("nan < 1.0", false),
        ("1.0 < nan", false),
        ("nan >= 1.0", false),
        ("1.0 >= nan", false),
    ] {
        assert_eq!(
            boolean(&format!("{nan}{comparison}")),
            expected,
            "{comparison}"
        );
    }
}

#[test]
fn string_comparisons_are_bytewise() {
    booleans(&[
        ("\"a\" < \"b\"", true),
        ("\"Z\" < \"a\"", true),
        ("\"\" < \"a\"", true),
        ("\"ab\" < \"b\"", true),
        ("\"b\" < \"ab\"", false),
        ("\"a\" <= \"a\"", true),
        ("\"a\" > \"A\"", true),
        ("\"abc\" >= \"abd\"", false),
        ("\"é\" > \"z\"", true),
        ("\"a\" == \"a\"", true),
        ("\"a\" == \"b\"", false),
        ("\"a\" != \"b\"", true),
        ("\"\" == \"\"", true),
        ("\"a\" == \"a \"", false),
        // Equality compares content, however the strings were built.
        ("\"{1}{2}\" == \"12\"", true),
        ("\"a\" + \"b\" == r\"ab\"", true),
        ("\"\"\"a\"\"\" == \"a\"", true),
    ]);
}

#[test]
fn bool_and_empty_record_equality() {
    booleans(&[
        ("true == true", true),
        ("false == false", true),
        ("true == false", false),
        ("true != false", true),
        ("false != false", false),
        ("(1 < 2) == (2 > 1)", true),
        ("{} == {}", true),
        ("{} != {}", false),
        ("let u = {}\nu == { let x = 1 }", true),
    ]);
    // Empty record operands are still evaluated.
    assert_eq!(
        error("{ let y = 1 // 0 } == {}"),
        ("division by zero".to_owned(), Span::new(12, 14))
    );
}

#[test]
fn records_are_built_and_read() {
    assert_eq!(int("{ x = 1; y = 2 }.y"), 2);
    assert_eq!(float("{ a = { n = 1.5 } }.a.n"), 1.5);
    assert!(boolean("{ \"a b\" = true }.\"a b\""));
    assert_eq!(string("{ s = \"hi\" }.s"), "hi");
    // Fields see earlier fields and private `let`s; later `let`s do not
    // change a field.
    assert_eq!(int("{ x = 1; let t = x * 10; y = t + x }.y"), 11);
    assert_eq!(
        int("let r = { x = 1; let x = 5; y = x }\nr.x * 10 + r.y"),
        15
    );
    // Stored in a variable and read back, whole and field by field.
    assert_eq!(
        run("let p = { x = 1; y = \"a\" }\nlet q = p\nq")
            .unwrap()
            .to_string(),
        "{x = 1; y = \"a\"}"
    );
    assert_eq!(int("let p = { x = 3; y = 4 }\np.x * p.x + p.y * p.y"), 25);
    // A record's field can be a record, read at any depth, and a record
    // can be rebuilt from another's fields.
    assert_eq!(
        run("let p = { a = { b = { c = 7 } } }\n{ c = p.a.b.c; d = p.a }")
            .unwrap()
            .to_string(),
        "{c = 7; d = {b = {c = 7}}}"
    );
    // An empty record as a field.
    assert_eq!(run("{ e = {} }").unwrap().to_string(), "{e = {}}");
    // Checked against an annotation, a literal is built at the field's
    // type.
    assert_eq!(float("let p: {x: Float} = { x = 1 }\np.x / 2"), 0.5);
    // A file of fields is a record.
    assert_eq!(
        run("x = 1\ny = \"a\"").unwrap().to_string(),
        "{x = 1; y = \"a\"}"
    );
}

#[test]
fn top_level_functions_call_each_other_in_any_order() {
    // Mutual recursion of two, each written before the other.
    let even = "fn even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }";
    let odd = "fn odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
    for text in [
        format!("{even}\n{odd}\neven(10)"),
        format!("{odd}\n{even}\neven(10)"),
    ] {
        assert_eq!(run(&text).unwrap(), Value::Bool(true), "{text}");
    }
    // Of three.
    assert_eq!(
        int("fn a(n: Int): Int { if n == 0 { 0 } else { b(n - 1) } }\n\
                 fn b(n: Int): Int { c(n) }\nfn c(n: Int): Int { a(n) + 1 }\na(9)"),
        9
    );
    // Used, from a function declared above it, inside a loop body, a
    // labelled block, an inner function and a `lazy`.
    let f = "fn f(n: Int): Int { n * 2 }";
    assert_eq!(
        run(&format!(
            "fn g(): List<Int> {{ for i in [1, 2] {{ f(i) }} }}\n{f}\ng()"
        ))
        .unwrap()
        .to_string(),
        "[2, 4]"
    );
    assert_eq!(
        int(&format!(
            "fn g(): Int {{ {{'l return'l f(3) }} }}\n{f}\ng()"
        )),
        6
    );
    assert_eq!(
        int(&format!(
            "fn g(): Int {{ let h = fn(): Int {{ f(1) }}\nh() }}\n{f}\ng()"
        )),
        2
    );
    assert_eq!(
        int(&format!(
            "fn g() -> Int {{ lazy f(4) }}\n{f}\nlet x = g()\nx + 0"
        )),
        8
    );
}

#[test]
fn a_top_level_function_and_a_soft_field_of_its_name() {
    // `fn f` is private: the spread's field stays in the record, and
    // reads of `f` call the function.
    assert_eq!(
        run("let r = {f = 2; g = 3}\n...r\nx = f()\ny = g\nfn f(): String { \"fn\" }")
            .unwrap()
            .to_string(),
        "{f = 2; g = 3; x = \"fn\"; y = 3}"
    );
    // A `def` overrides the soft field with the function.
    assert_eq!(
        run("let r = {f = 2; g = 3}\nx = f()\n...r\ndef f(): String { \"fn\" }")
            .unwrap()
            .to_string(),
        "{x = \"fn\"; f = fn(): String; g = 3}"
    );
}

#[test]
fn top_level_code_uses_a_function_once_it_is_made() {
    // The summary of the slice: used above their declarations, and
    // calling each other.
    let summary = "limit = 10\ntotal = sum-to(limit)\n\
             def describe(n: Int): String { if even(n) { \"even\" } else { \"odd\" } }\n\
             fn even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }\n\
             fn odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }\n\
             fn sum-to(n: Int): Int { if n == 0 { 0 } else { n + sum-to(n - 1) } }";
    assert_eq!(
        run(summary).unwrap().to_string(),
        "{limit = 10; total = 55; describe = fn(n: Int): String}"
    );
    // One that copies nothing, made as the input starts.
    assert_eq!(int("let x = f(2)\nfn f(n: Int): Int { n * 3 }\nx"), 6);
    // One that copies a `let` above the use, made just after it.
    assert_eq!(
        int("let k = 5\nlet x = f(2)\nfn f(n: Int): Int { n + k }\nx"),
        7
    );
    // Through another function, which copies what it reads.
    assert_eq!(
        int("let k = 5\nlet x = f(2)\nfn f(n: Int): Int { g(n) }\nfn g(n: Int): Int { n * k }\nx"),
        10
    );
    // A `def`'s value, made before its field, is the field's.
    assert_eq!(
        run("a = f(1)\ndef f(n: Int): Int { n + 1 }\nb = f(a)")
            .unwrap()
            .to_string(),
        "{a = 2; f = fn(n: Int): Int; b = 3}"
    );
    // A `lazy` that calls a function declared below it.
    assert_eq!(
        int("let x = lazy f(4)\nfn f(n: Int): Int { n * 2 }\nx + 0"),
        8
    );
    // A function that copies a mutable variable copies the value it has
    // at the function's own line, as before.
    assert_eq!(
        int("let mut n = 1\nn := 2\nfn f(): Int { n }\nn := 3\nf() * 10 + n"),
        23
    );
}

#[test]
fn mutually_recursive_top_level_functions_run_to_depth() {
    // Too deep, the nesting budget ends the recursion with its error.
    let (message, _) = on_big_stack(|| {
        error(
            "fn a(n: Int): Int { if n == 0 { 0 } else { b(n - 1) + 1 } }\n\
                 fn b(n: Int): Int { a(n) }\na(100000)",
        )
    });
    assert!(message.starts_with("call nested too deeply"), "{message}");
    // Within it, to depth.
    let depth = on_big_stack(|| {
        int(
            "fn a(n: Int): Int { if n == 0 { 0 } else { b(n - 1) + 1 } }\n\
                 fn b(n: Int): Int { a(n) }\na(700)",
        )
    });
    assert_eq!(depth, 700);
    // Memoised, each body runs once per argument: `a(80)` would not
    // end otherwise.
    assert_eq!(
        int(
            "fn a(n: Int) ->> Int { if n < 2 { n } else { b(n - 1) + a(n - 2) } }\n\
                 fn b(n: Int) ->> Int { if n < 2 { n } else { a(n - 1) + b(n - 2) } }\n\
                 a(80)"
        ),
        23_416_728_348_467_685
    );
}

#[test]
fn a_def_is_a_field_of_its_record() {
    // Where it is written, among the other fields.
    assert_eq!(
        run("a = 1\ndef f(n: Int): Int { n + a }\nb = f(2)")
            .unwrap()
            .to_string(),
        "{a = 1; f = fn(n: Int): Int; b = 3}"
    );
    // Recursion through its own name, and in an inner record block.
    assert_eq!(
        run("r = { def sum(n: Int): Int { if n == 0 { 0 } else { n + sum(n - 1) } }; s = sum(4) }")
            .unwrap()
            .to_string(),
        "{r = {sum = fn(n: Int): Int; s = 10}}"
    );
    // Overriding a soft field, it keeps the soft field's place, as a
    // field does (a spread defines its fields in its type's order).
    assert_eq!(
        run("...{f = 1; a = 2}\ndef f(): Int { 3 }\nb = f()")
            .unwrap()
            .to_string(),
        run("...{f = 1; a = 2}\nf = fn(): Int { 3 }\nb = f()")
            .unwrap()
            .to_string()
    );
    assert_eq!(
        run("...{f = 1; a = 2}\ndef f(): Int { 3 }\nb = f()")
            .unwrap()
            .to_string(),
        "{a = 2; f = fn(): Int; b = 3}"
    );
}

#[test]
fn a_def_named_by_a_dotted_path_runs_as_a_def() {
    // The summary in one file: recursion by path, a use above the
    // declaration, a sibling field, the prefix read whole.
    let summary = "total = dev.peios.count(4)\n\
             def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.count(n: Int): Int {\n\
             \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
             }\n\
             def dev.peios.name(): String { \"world\" }\n\
             dev.peios.version = \"1\"\n\
             greeting = dev.peios.hello()\n\
             all = dev.peios";
    assert_eq!(
        run(summary).unwrap().to_string(),
        "{total = 4; dev = {peios = {hello = fn(): String; count = fn(n: Int): Int; \
             name = fn(): String; version = \"1\"}}; greeting = \"hello world\"; \
             all = {hello = fn(): String; count = fn(n: Int): Int; name = fn(): String; \
             version = \"1\"}}"
    );
    // Mutual recursion with a plain `def`, and between two dotted ones,
    // each written before the other.
    let even = "def even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }";
    let odd = "def dev.odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
    for text in [
        format!("{even}\n{odd}\nr = [even(10), dev.odd(7)]"),
        format!("r = [even(10), dev.odd(7)]\n{odd}\n{even}"),
    ] {
        let shown = run(&text).unwrap().to_string();
        assert!(shown.contains("r = [true, true]"), "{text}: {shown}");
    }
    let f = "def a.x.f(n: Int): Int { if n == 0 { 0 } else { a.y.g(n - 1) + 1 } }";
    let g = "def a.y.g(n: Int): Int { if n == 0 { 0 } else { a.x.f(n - 1) + 1 } }";
    let last = |text: &str| format!("{:?}", fields(text).last().cloned());
    let int = |value: i64| format!("{:?}", Some(Value::Int(value)));
    assert_eq!(last(&format!("{f}\n{g}\nr = a.x.f(9)")), int(9));
    assert_eq!(last(&format!("{g}\n{f}\nr = a.x.f(9)")), int(9));
    // Through the record of a prefix, and from a function reading its
    // own prefix whole, which is no cycle: the record holds closures.
    assert_eq!(
        last("r = dev\ndef dev.peios.twice(n: Int): Int { n * 2 }\nx = r.peios.twice(4)"),
        int(8)
    );
    // (Run without `assert_cells_scheduled`, which takes the `Load` of
    // `a.h`'s closure into the record `a` for a call of it, which reads
    // `a`: the record only holds the closure.)
    let text = "def a.f(): Int { a.g + 1 }\na.g = 1\n\
                    def a.h(): Int { let r = a; r.g + r.f() }\nx = a.h()";
    let outcome = crate::run(&SourceFile::new("t.gcsl", text).unwrap()).unwrap();
    assert_eq!(
        outcome.value.to_string(),
        "{a = {f = fn(): Int; g = 1; h = fn(): Int}; x = 3}"
    );
    // A closure that copies a variable is made after it is bound, and
    // the prefix read below is assembled from it.
    assert_eq!(
        run("let k = 5\ndef dev.get(): Int { k }\nall = dev\nx = all.get()")
            .unwrap()
            .to_string(),
        "{dev = {get = fn(): Int}; all = {get = fn(): Int}; x = 5}"
    );
    // Memoised mutual recursion, each body run once per argument.
    assert_eq!(
        last(
            "def m.a(n: Int) ->> Int { if n < 2 { n } else { m.b(n - 1) + m.a(n - 2) } }\n\
                 def m.b(n: Int) ->> Int { if n < 2 { n } else { m.a(n - 1) + m.b(n - 2) } }\n\
                 x = m.a(80)"
        ),
        int(23_416_728_348_467_685)
    );
    // In an inner block, the block's dotted field, from its item on.
    assert_eq!(
        run("r = { def b.c(n: Int): Int { n + 1 }; d = b.c(1) }")
            .unwrap()
            .to_string(),
        "{r = {b = {c = fn(n: Int): Int}; d = 2}}"
    );
}

#[test]
fn a_dotted_def_recurses_by_path_to_depth() {
    let (message, _) = on_big_stack(|| {
        error(
            "def a.b(n: Int): Int { if n == 0 { 0 } else { a.c(n - 1) + 1 } }\n\
                 def a.c(n: Int): Int { a.b(n) }\nx = a.b(100000)",
        )
    });
    assert!(message.starts_with("call nested too deeply"), "{message}");
    let depth = on_big_stack(|| {
        let found = fields(
            "def a.b(n: Int): Int { if n == 0 { 0 } else { a.c(n - 1) + 1 } }\n\
                 def a.c(n: Int): Int { a.b(n) }\nx = a.b(700)",
        );
        format!("{:?}", found.last())
    });
    assert_eq!(depth, format!("{:?}", Some(Value::Int(700))));
}

#[test]
fn records_display_as_gcsl_syntax() {
    let value = run("{ x = 1; y = \"a\\n\"; inner = { n = 2.0 }; \"a b\" = true }").unwrap();
    let shown = "{x = 1; y = \"a\\n\"; inner = {n = 2.0}; \"a b\" = true}";
    assert_eq!(value.to_string(), shown);
    assert_eq!(value.repr().to_string(), shown);
}

#[test]
fn record_equality_is_deep_and_by_name() {
    booleans(&[
        ("{ x = 1; y = \"a\" } == { x = 1; y = \"a\" }", true),
        ("{ x = 1; y = \"a\" } == { x = 1; y = \"b\" }", false),
        ("{ x = 1; y = \"a\" } != { x = 2; y = \"a\" }", true),
        // Fields in a different order, of the same type.
        ("{ x = 1; y = 2 } == { y = 2; x = 1 }", true),
        ("{ x = 1; y = 2 } == { y = 1; x = 2 }", false),
        // Nested.
        ("{ a = { n = 1 } } == { a = { n = 1 } }", true),
        ("{ a = { n = 1 } } == { a = { n = 2 } }", false),
        ("{ e = {} } == { e = {} }", true),
        // Floats compare as IEEE 754 does, so NaN is unequal to itself.
        ("let r = { f = 0.0 / 0.0 }\nr == r", false),
        ("let r = { f = 0.0 / 0.0 }\nr != r", true),
        ("{ f = -0.0 } == { f = 0.0 }", true),
        // Through variables.
        ("let p = { x = 1 }\nlet q = { x = 1 }\np == q", true),
    ]);
    // Operands are evaluated left to right.
    assert_eq!(
        error("{ x = 1 // 0 } == { x = 9223372036854775807 + 1 }"),
        ("division by zero".to_owned(), Span::new(8, 10))
    );
}

#[test]
fn if_selects_records() {
    assert_eq!(
        run("if true { { x = 1; y = 2 } } else { { x = 3 } }")
            .unwrap()
            .to_string(),
        "{x = 1; y = 2}"
    );
    assert_eq!(
        int("let r = if 1 > 2 { { x = 1; y = 2 } } else { { x = 3 } }\nr.x"),
        3
    );
    assert_eq!(
        int("(if false { { x = 1 } } else { { x = 2; z = 0 } }).x"),
        2
    );
}

#[test]
fn records_in_interpolations_show_their_repr() {
    assert_eq!(
        string("let p = { x = 1; s = \"a\" }\n\"p is {p}, {{}} is { {} }\""),
        "p is {x = 1; s = \"a\"}, {} is {}"
    );
}

#[test]
fn logic() {
    booleans(&[
        ("true and true", true),
        ("true and false", false),
        ("false and true", false),
        ("true or false", true),
        ("false or false", false),
        ("not true", false),
        ("not false", true),
        ("not not true", true),
        ("not 1 == 2 and 3 > 2 or false", true),
    ]);
}

#[test]
fn and_and_or_short_circuit() {
    // The right operand would fail, so it is never evaluated.
    booleans(&[
        ("false and 1 // 0 == 0", false),
        ("true or 1 // 0 == 0", true),
        ("not (false and 1 // 0 == 0)", true),
        ("false and (true or 1 // 0 == 0)", false),
    ]);
    // When the left operand does not decide, it is.
    assert_eq!(
        error("true and 1 // 0 == 0"),
        ("division by zero".to_owned(), Span::new(11, 13))
    );
    assert_eq!(
        error("false or 1 // 0 == 0"),
        ("division by zero".to_owned(), Span::new(11, 13))
    );
}

#[test]
fn if_evaluates_exactly_one_branch() {
    assert_eq!(int("if true { 1 } else { 1 // 0 }"), 1);
    assert_eq!(int("if false { 1 // 0 } else { 2 }"), 2);
    assert_eq!(
        int("if 1 > 2 { 1 // 0 } else if 2 > 1 { 3 } else { 1 // 0 }"),
        3
    );
    assert_eq!(float("if true { 1.5 } else { 1.0 / 0.0 }"), 1.5);
    assert!(!boolean("if false { true } else { false }"));
    assert_eq!(string("if 1 < 2 { \"yes\" } else { \"no\" }"), "yes");
    assert_eq!(
        run("if true {} else { let x = 1 // 0 }").unwrap(),
        Value::Empty
    );
    assert_eq!(int("if false { let x = 1 // 0 }\n5"), 5);
    // The branch taken, and the condition, are evaluated.
    assert_eq!(
        error("if true { 1 // 0 } else { 2 }"),
        ("division by zero".to_owned(), Span::new(12, 14))
    );
    assert_eq!(
        error("if 1 // 0 == 0 { 1 } else { 2 }"),
        ("division by zero".to_owned(), Span::new(5, 7))
    );
    assert_eq!(
        error("if true { let y = 1 // 0 }"),
        ("division by zero".to_owned(), Span::new(20, 22))
    );
    // Adapted and defaulted branches.
    assert_eq!(float("if true { 1 } else { 2.5 }"), 1.0);
    assert_eq!(float("(if false { 1 } else { 2 }) * 0.5"), 1.0);
    assert_eq!(int("let x = if true { 7 } else { 8 }\nx + 1"), 8);
}

#[test]
fn interpolating_every_type() {
    assert_eq!(
        string("\"{1} {2.5} {3.0} {true} {\"s\"} { {} } {1 == 2} {1.0 / 0.0} {0.0 / 0.0} {-0.0}\""),
        "1 2.5 3.0 true s {} false inf nan -0.0"
    );
    assert_eq!(
        string("let n = 3\nlet s = \"x\"\nlet b = n > 2\n\"{s}{n}{s} {b}\""),
        "x3x true"
    );
    assert_eq!(string("\"{ { let a = 2; a * 3 } }\""), "6");
    assert_eq!(string("\"{\"\"}\""), "");
    assert_eq!(string("\"say \\\"{\"hi\"}\\\" {{ok}}\""), "say \"hi\" {ok}");
    assert_eq!(
        string("\"status: {if 1 > 2 { \"ok\" } else { \"failed\" }}\""),
        "status: failed"
    );
}

#[test]
fn nested_interpolation() {
    assert_eq!(string("\"a{ \"b{ \"c{1}\" }\" }d\""), "abc1d");
    assert_eq!(string("\"{ \"{ \"{ 1 + 1 }\" }\" }\""), "2");
    assert_eq!(
        string("let who = \"world\"\n\"<{ \"{who} and {\"{who}!\"}\" }>\""),
        "<world and world!>"
    );
}

#[test]
fn interpolation_errors_come_from_its_values() {
    assert_eq!(
        error("\"a{1 // 0}\""),
        ("division by zero".to_owned(), Span::new(5, 7))
    );
}

#[test]
fn concatenation() {
    assert_eq!(string("\"a\" + \"b\""), "ab");
    assert_eq!(string("\"\" + \"\""), "");
    assert_eq!(string("\"x\" + \"{1 + 1}\" + \"y\""), "x2y");
    assert_eq!(string("let s = \"ab\"\ns + s + s"), "ababab");
    assert_eq!(string("\"é\" + \"😀\""), "é😀");
}

#[test]
fn strings_round_trip_through_variables_and_blocks() {
    assert_eq!(
        string("let s = \"hello\"\nlet t = { let u = s; u }\nt"),
        "hello"
    );
    assert_eq!(string("let s = \"a\"\ns := s + \"b\"\n{ s }"), "ab");
    assert!(boolean("let s = \"x\"\n{ s } == s"));
    assert_eq!(string("let s = \"q\"\n= { = { s } }"), "q");
    assert_eq!(run("\"a\"").unwrap().kind(), ValueKind::String);
    assert_eq!(run("true").unwrap().kind(), ValueKind::Bool);
}

#[test]
fn loading_a_string_shares_it() {
    let ir = compile("let s = \"abc\"\nlet t = s\nlet u = s + \"\"");
    let mut frame = vec![Value::Empty; 3];
    assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
    let [Value::Str(s), Value::Str(t), Value::Str(u)] = &frame[..] else {
        panic!("expected strings, got {frame:?}");
    };
    // A load copies the reference, and concatenation makes a new string.
    assert!(Rc::ptr_eq(s, t));
    assert!(!Rc::ptr_eq(s, u));
    assert_eq!(u.as_ref(), "abc");
}

#[test]
fn a_load_of_the_wrong_type_is_an_internal_error() {
    // `x + 1` with `x` loaded as an Int, from a frame holding a Float.
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let load = builder.alloc(IrKind::Load(Slot::new(0), ValueKind::Int), Span::new(0, 1));
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
    assert_eq!(
        eval_in(&ir, &mut vec![Value::Int(2)]).unwrap(),
        Value::Int(3)
    );
    let error = eval_in(&ir, &mut vec![Value::Float(1.5)]).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: Int load from a slot holding another type"
    );
    assert_eq!(error.span, Span::new(0, 1));
}
