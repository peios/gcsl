use super::*;
use crate::check::check;
use crate::lexer::tokenize;
use crate::lower::lower;
use crate::parser::parse;
use crate::source::SourceFile;
use crate::types::TypeTable;
use crate::value::ValueKind;

fn compile(text: &str) -> Rc<Ir> {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    let ast = parse(&tokens, names, strings, &source).unwrap();
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let ir = Rc::new(lower(&ast, &checked, &types, 0).unwrap());
    assert_cells_scheduled(&ir, checked.resolution.field_cells(), text);
    ir
}

/// As [`compile`], but checking `text` as a lone file is, beside the
/// standard library, so that the prelude's names, `len` among them, are
/// its: what the tests of `len` compile with. Its frame's first slots
/// are the standard library's, which a run of it alone leaves `{}`: a
/// call of `len` never reads its cell.
fn compile_prelude(text: &str) -> Rc<Ir> {
    let (ast, types, output) = crate::workspace::check_beside_prelude(text);
    let checked = output
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
        .checked;
    let ir = Rc::new(lower(&ast, &checked, &types, 0).unwrap());
    assert_cells_scheduled(&ir, checked.resolution.field_cells(), text);
    ir
}

/// Asserts the static schedule of the cells of `ir`'s top-level fields
/// (`cells`): walking the top-level block's items in order, every cell
/// is stored before any item that `Load`s it, and before the first item
/// that loads or makes a function whose body, or the body of a function
/// it reads in turn, reads the cell through the frame (`Top`). A cell
/// read before its point holds `Value::Empty`, a valid `{}`, so a
/// scheduling bug would otherwise go unseen. Iterative throughout.
fn assert_cells_scheduled(ir: &Ir, cells: &[Slot], text: &str) {
    let cell = |slot: Slot| cells.contains(&slot) && slot != Slot::NONE;
    // What each node reads: the cells it loads or reads through the
    // frame, the functions it makes, and the top-level slots it reads
    // through the frame as functions.
    let reads = |root: IrId, top_level: bool| {
        let (mut loads, mut fns, mut tops) = (Vec::new(), Vec::new(), Vec::new());
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            match ir.get(id).kind {
                IrKind::Load(slot, _) if top_level => loads.push(slot),
                IrKind::Top(slot, _) => tops.push(slot),
                IrKind::MakeClosure { function, .. } | IrKind::MakeFuture { function, .. } => {
                    fns.push(function);
                }
                _ => {}
            }
            stack.extend(ir.children(id));
        }
        (loads, fns, tops)
    };
    // The slots a function's closure lives in, by the top-level `Store`
    // of its `MakeClosure`, so that a `Top` of a function is followed.
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        return;
    };
    let items = ir.items(items);
    let mut closure_at: Vec<(Slot, FnId)> = Vec::new();
    for &item in items {
        if let IrKind::Store(slot, value) = ir.get(item).kind
            && let IrKind::MakeClosure { function, .. } = ir.get(value).kind
        {
            closure_at.push((slot, function));
        }
    }
    let mut stored: Vec<Slot> = Vec::new();
    for &item in items {
        let (loads, fns, tops) = reads(item, true);
        let mut needed: Vec<Slot> = loads;
        // Functions this item reaches, followed through frame reads.
        let mut work: Vec<FnId> = fns;
        work.extend(
            closure_at
                .iter()
                .filter(|(s, _)| needed.contains(s) || tops.contains(s))
                .map(|&(_, f)| f),
        );
        // Making a top-level function's closure mentions nothing: its
        // body reads cells when it runs, not when it is made.
        if closure_at.iter().any(
            |&(slot, _)| matches!(ir.get(item).kind, IrKind::Store(stored, _) if stored == slot),
        ) {
            work.clear();
        }
        let mut seen: Vec<FnId> = Vec::new();
        while let Some(function) = work.pop() {
            if seen.contains(&function) {
                continue;
            }
            seen.push(function);
            let (_, inner, frame) = reads(ir.function(function).body, false);
            work.extend(inner);
            work.extend(
                closure_at
                    .iter()
                    .filter(|(s, _)| frame.contains(s))
                    .map(|&(_, f)| f),
            );
            needed.extend(frame);
        }
        for slot in needed.into_iter().filter(|&slot| cell(slot)) {
            assert!(
                stored.contains(&slot),
                "{text}: cell {slot} read before its store"
            );
        }
        if let IrKind::Store(slot, _) = ir.get(item).kind
            && cell(slot)
        {
            stored.push(slot);
        }
    }
}

#[test]
fn top_level_fields_are_computed_at_their_points() {
    let last = |text: &str| format!("{:?}", fields(text).last().cloned());
    let int = |value: i64| format!("{:?}", Some(Value::Int(value)));
    // A dotted root read between its items is the whole root.
    assert_eq!(last("a.b = 1\nc = a\na.d = 2\nr = c.d"), int(2));
    assert_eq!(last("a.b = 1\nc = a.b + a.d\na.d = 2"), int(3));
    // A field used above its line, by a `let` and through a function.
    assert_eq!(
        last("let x = y + 1\ny = g()\nfn g(): Int { 41 }\nr = x"),
        int(42)
    );
    assert_eq!(
        last("let x = f()\nfn f(): Int { k.v }\nk.v = 5\nr = x"),
        int(5)
    );
    // A function reads a field through the frame before and after it.
    assert_eq!(
        last("fn f(): Int { n }\nlet x = f()\nn = 3\nlet y = f()\nr = x + y"),
        int(6)
    );
    // A `lazy` reads a field declared below it.
    assert_eq!(last("let l = lazy { m * 2 }\nm = 4\nr = await l"), int(8));
}

#[test]
fn the_fields_of_a_package_may_be_written_in_any_order() {
    let text = "let major = 2\ncom.x.app.deps = [com.x.lib]\n\
                    com.x.app.name = \"app-{version}\"\ncom.x.lib.name = \"lib\"\n\
                    version = describe(major)\nfn describe(n: Int): String { \"v{n}\" }";
    assert_eq!(
        run(text).unwrap().to_string(),
        "{com = {x = {app = {deps = [{name = \"lib\"}]; name = \"app-v2\"}; \
             lib = {name = \"lib\"}}}; version = \"v2\"}"
    );
}

#[test]
fn long_chains_of_fields_need_no_deep_recursion() {
    // The first item needs the whole chain, written below it, and each
    // dotted field reads the one before it: neither recurses per field.
    let count = 3000;
    let plain = (1..count).map(|index| format!("x{index} = x{} + 1\n", index + 1));
    let dotted = (2..=count).map(|index| format!("a.f{index} = a.f{} + 1\n", index - 1));
    let text: String = std::iter::once("total = x1\n".to_string())
        .chain(plain)
        .chain([format!("x{count} = 0\na.f1 = 1\n")])
        .chain(dotted)
        .chain(["last = a.f3000".to_string()])
        .collect();
    let value = run(&text).unwrap().to_string();
    assert!(value.starts_with("{total = 2999; "), "{}", &value[..40]);
    assert!(value.ends_with("last = 3000}"), "{value}");
}

#[test]
fn a_top_level_field_is_computed_once_with_its_effects_at_its_point() {
    // Mentioned by several items above it, a field is stored once.
    let text = "let a = x + 1\nlet b = x * 2\nlet c = { x }\nx = 5\nr = a + b + c";
    assert_eq!(fields(text), vec![Value::Int(5), Value::Int(21)]);
    let ir = compile(text);
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("{text}: the top level is not a block");
    };
    let stores = ir
        .items(items)
        .iter()
        .filter(|&&item| {
            matches!(ir.get(item).kind, IrKind::Store(_, value)
                    if matches!(ir.get(value).kind, IrKind::IntConst(5)))
        })
        .count();
    assert_eq!(stores, 1, "{text}");
    // A field that reads a mutable variable is computed at its own line,
    // with the value the variable has there.
    assert_eq!(
        fields("let mut t = 1\nf = t\nt := t + 1\ng = t\nr = f * 10 + g"),
        vec![Value::Int(1), Value::Int(2), Value::Int(12)]
    );
    // A field's effects happen where it is computed: before the first
    // item that needs it, here above an item that would fail first if
    // the field were computed at its own line; through an impure
    // function too.
    let index = run("[1][5]").unwrap_err().message;
    for text in [
        "let a = b\nlet c = 1 // 0\nb = [1][5]",
        "let a = b\nlet c = 1 // 0\nb = get()\nfn get() -> Int { [1][5] }",
    ] {
        assert_eq!(run(text).unwrap_err().message, index, "{text}");
    }
}

#[test]
fn a_discarded_call_runs_with_what_it_defers() {
    // The call runs where it stands, failing here before the result.
    let index = run("[1][5]").unwrap_err().message;
    for text in [
        "fn boom() -> Int { [1][5] }\nboom()\nx = 1",
        "fn boom() -> Int { [1][5] }\nfn f() -> Int {\n    boom()\n    = 1\n}\nx = f()",
        // What it defers runs too, rather than being lost with its value.
        "fn later() -> Int { = lazy [1][5] }\nlater()\nx = 1",
    ] {
        assert_eq!(run(text).unwrap_err().message, index, "{text}");
    }
    assert_eq!(
        int("fn one() -> Int { 1 }\nfn f() -> Int {\n    one()\n    = 2\n}\nf()"),
        2
    );
}

fn boolean(text: &str) -> bool {
    match run(text).unwrap() {
        Value::Bool(value) => value,
        other => panic!("{text}: expected a Bool, got {other:?}"),
    }
}

fn string(text: &str) -> String {
    match run(text) {
        Ok(Value::Str(value)) => value.to_string(),
        other => panic!("{text}: expected a String, got {other:?}"),
    }
}

/// Checks that each program evaluates to the Bool given.
fn booleans(cases: &[(&str, bool)]) {
    for &(text, expected) in cases {
        assert_eq!(boolean(text), expected, "{text}");
    }
}

fn run(text: &str) -> Result<Value, Diagnostic> {
    eval(&compile_prelude(text))
}

fn int(text: &str) -> i64 {
    match run(text).unwrap() {
        Value::Int(value) => value,
        other => panic!("{text}: expected an Int, got {other:?}"),
    }
}

fn float(text: &str) -> f64 {
    match run(text).unwrap() {
        Value::Float(value) => value,
        other => panic!("{text}: expected a Float, got {other:?}"),
    }
}

fn error(text: &str) -> (String, Span) {
    let diagnostic = run(text).unwrap_err();
    (diagnostic.message, diagnostic.span)
}

/// `i64::MIN`, which cannot be written as a literal.
const MIN: &str = "(-9223372036854775807 - 1)";
const MAX: &str = "9223372036854775807";

fn nz(value: i64) -> NonZeroI64 {
    NonZeroI64::new(value).unwrap()
}

#[test]
fn arithmetic() {
    assert_eq!(int("1 + 2 * 3"), 7);
    assert_eq!(int("8 - 3 - 2"), 3);
    assert_eq!(int("2 * 3 // 4"), 1);
    assert_eq!(int("-2 * 3"), -6);
    assert_eq!(int("-(-2)"), 2);
    assert_eq!(int("2 * -3"), -6);
    assert_eq!(float("7 / 2"), 3.5);
    assert_eq!(float("7 / 2.0"), 3.5);
    assert_eq!(float("(1 + 2) * 3.0"), 9.0);
    assert_eq!(float("1.5 - 2"), -0.5);
    assert_eq!(float("-1.5"), -1.5);
    assert_eq!(float("(7 // 2) / 2"), 1.5);
}

#[test]
fn floor_division_and_modulo_in_all_sign_combinations() {
    assert_eq!((int("7 // 2"), int("7 % 2")), (3, 1));
    assert_eq!((int("-7 // 2"), int("-7 % 2")), (-4, 1));
    assert_eq!((int("7 // -2"), int("7 % -2")), (-4, -1));
    assert_eq!((int("-7 // -2"), int("-7 % -2")), (3, -1));
}

#[test]
fn exact_division_needs_no_adjustment() {
    assert_eq!((int("6 // 2"), int("6 % 2")), (3, 0));
    assert_eq!((int("-6 // 2"), int("-6 % 2")), (-3, 0));
    assert_eq!((int("6 // -2"), int("6 % -2")), (-3, 0));
    assert_eq!((int("-6 // -2"), int("-6 % -2")), (3, 0));
    assert_eq!((int("0 // -5"), int("0 % -5")), (0, 0));
}

#[test]
fn floor_division_identity_holds() {
    for a in [-9, -7, -1, 0, 1, 7, 9, i64::MIN, i64::MAX] {
        for b in [-7, -2, -1, 1, 2, 7, i64::MIN, i64::MAX] {
            let Some(q) = floor_div(a, nz(b)) else {
                assert_eq!((a, b), (i64::MIN, -1));
                continue;
            };
            let r = floor_mod(a, nz(b));
            assert_eq!(i128::from(b) * i128::from(q) + i128::from(r), i128::from(a));
            assert!(r == 0 || (r < 0) == (b < 0), "{a} % {b} = {r}");
            assert!(r.unsigned_abs() < b.unsigned_abs());
        }
    }
}

#[test]
fn differs_from_euclidean_division() {
    assert_eq!(7_i64.div_euclid(-2), -3);
    assert_eq!(floor_div(7, nz(-2)), Some(-4));
    assert_eq!(7_i64.rem_euclid(-2), 1);
    assert_eq!(floor_mod(7, nz(-2)), -1);
}

#[test]
fn overflow_on_each_operator() {
    let overflow = "integer overflow".to_owned();
    // The error points at the operator.
    assert_eq!(
        error(&format!("{MAX} + 1")),
        (overflow.clone(), Span::new(20, 21))
    );
    assert_eq!(error(&format!("{MIN} - 1")).0, overflow);
    assert_eq!(error(&format!("{MAX} * 2")).0, overflow);
    assert_eq!(
        error(&format!("-{MIN}")),
        (overflow.clone(), Span::new(0, 1))
    );
    assert_eq!(error(&format!("{MIN} // -1")).0, overflow);
    assert_eq!(error(&format!("{MIN} * -1")).0, overflow);
}

#[test]
fn runtime_errors_label_operands_with_their_values() {
    let diagnostic = run("2 * (9223372036854775807 - -1)").unwrap_err();
    assert_eq!(diagnostic.message, "integer overflow");
    assert_eq!(diagnostic.span, Span::new(25, 26));
    let labels: Vec<(Span, &str)> = diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect();
    assert_eq!(
        labels,
        vec![
            (Span::new(5, 24), "9223372036854775807"),
            (Span::new(27, 29), "-1")
        ]
    );
    let diagnostic = run("-(-9223372036854775807 - 1)").unwrap_err();
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].text, "-9223372036854775808");
    // The parenthesised operand is labelled without its parentheses.
    assert_eq!(diagnostic.labels[0].span, Span::new(2, 26));
}

#[test]
fn i64_min_cases() {
    assert_eq!(int(MIN), i64::MIN);
    assert_eq!(int(&format!("{MIN} % -1")), 0);
    assert_eq!(int(&format!("{MIN} // 1")), i64::MIN);
    assert_eq!(int(&format!("{MIN} // 2")), i64::MIN / 2);
    assert_eq!(int(&format!("{MIN} // {MAX}")), -2);
    assert_eq!(int(&format!("{MIN} % {MAX}")), i64::MAX - 1);
    assert_eq!(int(&format!("{MIN} % 2")), 0);
    assert_eq!(int(&format!("-({MIN} + 1)")), i64::MAX);
    assert_eq!(float(&format!("{MIN} / -1")), 9_223_372_036_854_775_808.0);
}

#[test]
fn division_by_zero() {
    let message = "division by zero".to_owned();
    assert_eq!(error("7 // 0"), (message.clone(), Span::new(2, 4)));
    assert_eq!(error("7 % 0"), (message.clone(), Span::new(2, 3)));
    assert_eq!(error("1 + 7 % (2 - 2)"), (message, Span::new(6, 7)));
    let diagnostic = run("1 + 7 % (2 - 2)").unwrap_err();
    assert_eq!(diagnostic.labels[1].text, "0");
    assert_eq!(diagnostic.labels[1].span, Span::new(9, 14));
}

#[test]
fn float_division_by_zero_is_ieee() {
    assert_eq!(float("1.0 / 0.0"), f64::INFINITY);
    assert_eq!(float("-1.0 / 0.0"), f64::NEG_INFINITY);
    assert!(float("0.0 / 0.0").is_nan());
    assert_eq!(float("1 / 0"), f64::INFINITY);
    assert!(float("1e308 * 10.0").is_infinite());
    assert!(float("(1.0 / 0.0) - (1.0 / 0.0)").is_nan());
}

#[test]
fn errors_are_reported_left_to_right() {
    assert_eq!(
        error("(1 // 0) + (9223372036854775807 + 1)"),
        ("division by zero".to_owned(), Span::new(3, 5))
    );
    assert_eq!(
        error("(9223372036854775807 + 1) + (1 // 0)"),
        ("integer overflow".to_owned(), Span::new(21, 22))
    );
}

#[test]
fn root_value_type_follows_the_ir() {
    assert_eq!(run("1").unwrap().kind(), ValueKind::Int);
    assert_eq!(run("1 / 1").unwrap().kind(), ValueKind::Float);
    assert_eq!(run("").unwrap().kind(), ValueKind::Record);
    assert_eq!(run("x = 1").unwrap().kind(), ValueKind::Record);
}

#[test]
fn blocks_yield_their_result() {
    assert_eq!(int("let x = { let y = 1; y + 1 }\nx"), 2);
    assert_eq!(int("{ { { 3 } } }"), 3);
    assert_eq!(int("{ 1 } + { 2 } * 3"), 7);
    assert_eq!(int("{ = 4 }"), 4);
    assert_eq!(int("let x = 1\n= { let x = 5\n= x } - x"), 4);
    assert_eq!(float("{ 1.5 } * 2"), 3.0);
    assert_eq!(float("{ let h = 0.5\nh } / { 2 }"), 0.25);
}

#[test]
fn shadowing() {
    assert_eq!(
        int("let x = 1\nlet x = x + 1\n{ let x = x * 10; x } + x"),
        22
    );
    // A shadowing `let` may change the type.
    assert_eq!(float("let x = 1\nlet x = 2.5\nx"), 2.5);
    // The outer variable is intact after an inner block shadows it.
    assert_eq!(int("let x = 1\n{ let x = 2 }\nx"), 1);
}

#[test]
fn rebinding() {
    assert_eq!(int("let x = 1\nx := x + 1\nx := x * 3\nx"), 6);
    // The literal adapts to the variable's type.
    assert_eq!(float("let x: Float = 1\nx := 2\nx"), 2.0);
    assert_eq!(int("{ let n = 1; n := n + 1; n }"), 2);
}

#[test]
fn ascription() {
    assert_eq!(float("1 + 2 : Float"), 3.0);
    assert_eq!(float("-5 : Float"), -5.0);
    assert_eq!(float("let x: Float = 5\nx / 2"), 2.5);
    assert_eq!(float("let x = 5 : Float\nx * 2"), 10.0);
    assert_eq!(float("{ 5 } : Float"), 5.0);
    assert_eq!(int("let x = 3\nx : Int"), 3);
}

#[test]
fn nested_blocks_reuse_slots() {
    let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
    assert_eq!(int(text), 114);
    // Each temporary shares its slot with the variable declared after
    // it; only the innermost `w` needs a slot of its own.
    assert_eq!(compile(text).frame_size(), 4);
}

#[test]
fn empty_record_results() {
    for text in [
        "",
        "{}",
        "let x = 1",
        "{ let x = 1 }",
        "let u = {}\nu",
        "{ {}; {} }",
        "let x = 1\nx := 2",
        "= {}",
        "if true {}",
        "{ e = {} }.e",
    ] {
        assert_eq!(run(text).unwrap(), Value::Empty, "{text}");
    }
}

#[test]
fn items_run_in_order_and_their_errors_are_reported() {
    assert_eq!(
        error("let x = 1 // 0\n5"),
        ("division by zero".to_owned(), Span::new(10, 12))
    );
    assert_eq!(
        error("{ let y = 9223372036854775807 + 1 }\n1"),
        ("integer overflow".to_owned(), Span::new(30, 31))
    );
    // The first failing item wins.
    assert_eq!(
        error("let a = 1 // 0\nlet b = 9223372036854775807 + 1\nb").0,
        "division by zero"
    );
}

#[test]
fn eval_in_uses_the_given_frame() {
    let ir = compile("let x = 7\nlet y = x + 1");
    let mut frame = vec![Value::Empty; 3];
    assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
    // The stack is truncated to the frame on return.
    assert_eq!(frame, vec![Value::Int(7), Value::Int(8)]);
    frame.truncate(1);
    let error = eval_in(&ir, &mut frame).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: frame is smaller than the IR needs"
    );
}

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

/// The record in `value`, which must be one.
fn record_of(value: &Value) -> &Rc<Record> {
    match value {
        Value::Record(record) => record,
        other => panic!("expected a record, got {other:?}"),
    }
}

#[test]
fn dotted_fields_merge_into_records() {
    assert_eq!(
        run("server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true")
            .unwrap()
            .to_string(),
        "{server = {host = \"localhost\"; port = 8080; tls = {enabled = true}}}"
    );
    // Fields in the order of first mention, at every level.
    assert_eq!(
        run("{ a.b.c = 1; d = 2; a.e = 3; a.b.f = 4 }")
            .unwrap()
            .to_string(),
        "{a = {b = {c = 1; f = 4}; e = 3}; d = 2}"
    );
    // Later items see the record built so far.
    assert_eq!(
        run("{ a.x = 1; let p = a; a.y = a.x + 1; q = p }")
            .unwrap()
            .to_string(),
        "{a = {x = 1; y = 2}; q = {x = 1}}"
    );
    assert_eq!(int("{ a.x = 1; a.y = 2 }.a.y"), 2);
}

#[test]
fn an_update_that_replaces_a_field_shares_the_layout() {
    let ir = compile(
        "let a = { x = 1; s = \"text\" }\n\
             let b = a\n\
             let b.x = 5\n\
             let c = a\n\
             let c.z = true",
    );
    let mut frame = vec![Value::Empty; 5];
    eval_in(&ir, &mut frame).unwrap();
    let [a, _, replaced, _, added] = &frame[..] else {
        panic!("expected five slots, got {frame:?}");
    };
    // The old record is intact.
    assert_eq!(a.to_string(), "{x = 1; s = \"text\"}");
    assert_eq!(replaced.to_string(), "{x = 5; s = \"text\"}");
    let (a, replaced) = (record_of(a), record_of(replaced));
    assert!(Rc::ptr_eq(&a.layout, &replaced.layout));
    assert!(!Rc::ptr_eq(a, replaced));
    // An addition makes a new layout, with the name at the end.
    assert_eq!(added.to_string(), "{x = 1; s = \"text\"; z = true}");
    let added = record_of(added);
    assert!(!Rc::ptr_eq(&a.layout, &added.layout));
    assert_eq!(added.layout.names.len(), 3);
    assert_eq!(added.layout.names[..2], a.layout.names[..]);
    assert!(!a.layout.names.contains(&added.layout.names[2]));
    assert_eq!(
        &*added.layout.texts,
        &[Box::from("x"), Box::from("s"), Box::from("z")]
    );
    // The unchanged fields are shared, not copied.
    let shared = |record: &Record| match &record.values[1] {
        Value::Str(text) => Rc::clone(text),
        other => panic!("expected a string, got {other:?}"),
    };
    assert!(Rc::ptr_eq(&shared(a), &shared(replaced)));
    assert!(Rc::ptr_eq(&shared(a), &shared(added)));
}

#[test]
fn updates_nest_and_create_records() {
    assert_eq!(
        run("let a = { b = { c = 1; d = 2 } }\nlet a.b.c = 10\na")
            .unwrap()
            .to_string(),
        "{b = {c = 10; d = 2}}"
    );
    assert_eq!(
        run("let a = { x = 1 }\nlet a.b.c = 2\na")
            .unwrap()
            .to_string(),
        "{x = 1; b = {c = 2}}"
    );
    assert_eq!(
        run("let a.b.c = \"s\"\na").unwrap().to_string(),
        "{b = {c = \"s\"}}"
    );
    assert_eq!(
        run("let a = { x = 1 }\na.x := a.x + 1\nlet a.y.z = false\na.y.z := true\na")
            .unwrap()
            .to_string(),
        "{x = 2; y = {z = true}}"
    );
    // A leaf replaced by a value of another type.
    assert_eq!(
        run("let a = { b = { c = 1 } }\nlet a.b = 2.5\na")
            .unwrap()
            .to_string(),
        "{b = 2.5}"
    );
    // A field the static type has as `{}` may hold the empty record.
    assert_eq!(
        run("let a = { e = {} }\nlet a.e.x = 1\na")
            .unwrap()
            .to_string(),
        "{e = {x = 1}}"
    );
    // A field hidden by subtyping is replaced where the path creates a
    // record, since the static type is the truth.
    assert_eq!(
        run("let p: {x: Int} = { x = 1; y = 2 }\nlet p.y.z = 3\np")
            .unwrap()
            .to_string(),
        "{x = 1; y = {z = 3}}"
    );
    // The value is evaluated after the record, and its errors reported.
    assert_eq!(
        error("let a = { x = 1 }\nlet a.y = 1 // 0").0,
        "division by zero"
    );
}

#[test]
fn an_update_leaves_the_old_value_intact() {
    assert_eq!(
        string("let a = { x = 1; n = { m = 2 } }\nlet old = a\nlet a.n.m = 3\n\"{old} {a}\""),
        "{x = 1; n = {m = 2}} {x = 1; n = {m = 3}}"
    );
    assert_eq!(
        string("let a = { x = 1 }\nlet old = a\na.x := 2\n\"{old} {a}\""),
        "{x = 1} {x = 2}"
    );
    // Through a field: the field's record is extended in its slot, and
    // what was read from it before is unchanged.
    assert_eq!(
        run("{ a.x = 1; let p = a; a.y = 2; q = p }")
            .unwrap()
            .to_string(),
        "{a = {x = 1; y = 2}; q = {x = 1}}"
    );
}

#[test]
fn typed_equality_compares_the_static_fields() {
    let narrowed = "let q: {x: Int} = { x = 1; y = 2 }\n";
    booleans(&[
        (&format!("{narrowed}q == {{ x = 1 }}"), true),
        (&format!("{narrowed}q === {{ x = 1 }}"), false),
        (&format!("{narrowed}q != {{ x = 1 }}"), false),
        (&format!("{narrowed}q !== {{ x = 1 }}"), true),
        (&format!("{narrowed}q == {{ x = 2 }}"), false),
        (&format!("{narrowed}q === q"), true),
        (&format!("{narrowed}q == q"), true),
    ]);
    // Nested records compare over their static fields too.
    let nested = "let q: {a: {n: Int}} = { a = { n = 1; m = 2 }; b = 3 }\n";
    booleans(&[
        (&format!("{nested}q == {{ a = {{ n = 1 }} }}"), true),
        (&format!("{nested}q === {{ a = {{ n = 1 }} }}"), false),
        (&format!("{nested}q == {{ a = {{ n = 2 }} }}"), false),
        (&format!("{nested}q != {{ a = {{ n = 2 }} }}"), true),
        (&format!("{nested}q.a == {{ n = 1 }}"), true),
    ]);
    // A record typed `{}` equals every other.
    booleans(&[
        ("let u: {} = { x = 1 }\nu == {}", true),
        ("let u: {} = { x = 1 }\nu === {}", false),
        ("let u: {} = { x = 1 }\nu != {}", false),
        ("let e: {e: {}} = { e = { x = 1 } }\ne == { e = {} }", true),
    ]);
    // Floats compare as IEEE 754 does either way.
    booleans(&[
        ("let r = { f = 0.0 / 0.0 }\nr == r", false),
        ("let r = { f = 0.0 / 0.0 }\nr === r", false),
        ("let r = { f = 0.0 / 0.0 }\nr != r", true),
        ("{ f = -0.0 } == { f = 0.0 }", true),
        ("{ f = -0.0 } === { f = 0.0 }", true),
    ]);
    // Scalars are the same under both.
    booleans(&[
        ("1 === 1", true),
        ("1 !== 1", false),
        ("\"a\" === \"b\"", false),
        ("true !== false", true),
        ("1.5 === 1.5", true),
    ]);
    // Operands are evaluated left to right.
    assert_eq!(
        error("{ x = 1 // 0 } == { x = 9223372036854775807 + 1 }"),
        ("division by zero".to_owned(), Span::new(8, 10))
    );
    assert_eq!(
        error("{ x = 1 // 0 } === { x = 9223372036854775807 + 1 }"),
        ("division by zero".to_owned(), Span::new(8, 10))
    );
}

/// A record nested `depth` deep through the field named `inner`,
/// ending in `leaf`.
fn nested_record(inner: Name, depth: usize, leaf: Value) -> Value {
    let layout = Rc::new(Layout {
        names: Box::new([inner]),
        texts: Box::new([Box::from("inner")]),
    });
    let mut value = leaf;
    for _ in 0..depth {
        value = Value::Record(Rc::new(Record {
            layout: Rc::clone(&layout),
            values: Box::new([value]),
        }));
    }
    value
}

#[test]
fn typed_equality_of_deep_records_does_not_recurse() {
    // Far deeper than a test thread's stack could take recursively: a
    // shape for each level, the last comparing the leaf as a value.
    const DEPTH: usize = 1_000_000;
    let mut names = crate::names::Names::new();
    let inner = names.intern("inner");
    let mut builder = crate::ir::IrBuilder::with_capacity(1);
    let first = builder.push_shape(crate::ir::Shape {
        fields: Box::default(),
        rest: None,
    });
    let mut shape = first;
    for _ in 1..DEPTH {
        let next = builder.push_shape(crate::ir::Shape {
            fields: Box::default(),
            rest: None,
        });
        builder.set_shape(
            shape,
            crate::ir::Shape {
                fields: Box::new([(inner, Some(next))]),
                rest: None,
            },
        );
        shape = next;
    }
    builder.set_shape(
        shape,
        crate::ir::Shape {
            fields: Box::new([(inner, None)]),
            rest: None,
        },
    );
    let root = builder.alloc(IrKind::Empty, Span::new(0, 0));
    let ir = builder.finish(root, 0, Vec::new());
    let a = nested_record(inner, DEPTH, Value::Int(0));
    let b = nested_record(inner, DEPTH, Value::Int(0));
    let c = nested_record(inner, DEPTH, Value::Int(1));
    assert_eq!(record_eq_shaped(&ir, first, &a, &b), Some(true));
    assert_eq!(record_eq_shaped(&ir, first, &a, &c), Some(false));
    // Shallower records lack a field the shape has at the bottom.
    let shallow = nested_record(inner, DEPTH - 1, Value::Int(0));
    assert_eq!(record_eq_shaped(&ir, first, &a, &shallow), None);
}

#[test]
fn a_record_lacking_a_field_of_its_shape_is_an_internal_error() {
    // `r == r` built by hand, with `r` holding a record without the
    // shape's `y`.
    let mut names = crate::names::Names::new();
    let (x, y) = (names.intern("x"), names.intern("y"));
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let shape = builder.push_shape(crate::ir::Shape {
        fields: Box::new([(y, None)]),
        rest: None,
    });
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Record),
        Span::new(0, 1),
    );
    let eq = builder.alloc(
        IrKind::RecordEq(ShapedOperands {
            shape,
            lhs: load,
            rhs: load,
        }),
        Span::new(0, 6),
    );
    let ir = Rc::new(builder.finish(eq, 1, Vec::new()));
    let record = Value::Record(Rc::new(Record {
        layout: Rc::new(Layout {
            names: Box::new([x]),
            texts: Box::new([Box::from("x")]),
        }),
        values: Box::new([Value::Int(1)]),
    }));
    let error = eval_in(&ir, &mut vec![record]).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: a record lacks a field of its shape"
    );
    assert_eq!(error.span, Span::new(0, 6));
    // So is the empty record, where the shape has fields.
    let error = eval_in(&ir, &mut vec![Value::Empty]).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: a record lacks a field of its shape"
    );
}

#[test]
fn an_update_without_its_name_text_is_an_internal_error() {
    let mut names = crate::names::Names::new();
    let x = names.intern("x");
    let mut builder = crate::ir::IrBuilder::with_capacity(3);
    let nothing = builder.alloc(IrKind::Empty, Span::new(0, 1));
    let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
    let update = builder.alloc(
        IrKind::UpdateField {
            record: nothing,
            name: x,
            value: one,
        },
        Span::new(0, 5),
    );
    let ir = Rc::new(builder.finish(update, 0, Vec::new()));
    let error = eval(&ir).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: an update's name has no text"
    );
    assert_eq!(error.span, Span::new(0, 5));
}

#[test]
fn a_missing_field_is_an_internal_error() {
    // `r.y` built by hand, with `r` holding a record without `y`.
    let mut names = crate::names::Names::new();
    let (x, y) = (names.intern("x"), names.intern("y"));
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Record),
        Span::new(0, 1),
    );
    let field = builder.alloc(
        IrKind::Field {
            record: load,
            name: y,
            kind: ValueKind::Int,
        },
        Span::new(0, 3),
    );
    let ir = Rc::new(builder.finish(field, 1, Vec::new()));
    let record = |value: Value| {
        Value::Record(Rc::new(Record {
            layout: Rc::new(crate::value::Layout {
                names: Box::new([x]),
                texts: Box::new([Box::from("x")]),
            }),
            values: Box::new([value]),
        }))
    };
    let error = eval_in(&ir, &mut vec![record(Value::Int(1))]).unwrap_err();
    assert_eq!(
        error.message,
        "internal error: a record lacks a field the checker found"
    );
    assert_eq!(error.span, Span::new(0, 3));
    let error = eval_in(&ir, &mut vec![Value::Empty]).unwrap_err();
    assert_eq!(error.message, "internal error: a field of the empty record");
}

// ----- Functions -----

/// Runs `f` on a thread with [`crate::STACK_SIZE`] of stack, as the
/// pipeline is meant to be run. Test threads are far smaller by default.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
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
fn run_on(ir: &Rc<Ir>, stack: &mut Vec<Value>) -> (Result<Value, Diagnostic>, usize) {
    let result = Evaluator {
        ir,
        stack,
        base: 0,
        captures: &[],
        callee: None,
        memo: &mut MemoTables::new(),
        shared: &Shared {
            exec_result: None,
            dirs: &[],
            temp_dirs: None,
        },
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

const BUDGET_ERROR: &str =
    "call nested too deeply: the limit is 10000 levels of nesting, calls included";
const BUDGET_NOTE: &str = "every level of expression nesting around a call counts, and so \
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

// ----- Futures -----

/// The labels of a diagnostic as `(span, text)` pairs.
fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
    diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect()
}

/// Runs `text` on a frame of its own and returns the frame afterwards,
/// holding the value of each top-level variable.
fn frame_after(text: &str) -> Vec<Value> {
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
type AwaitNesting = (
    &'static str,
    &'static str,
    fn(&str) -> String,
    fn(i64) -> Value,
);

/// The levels of nesting each kind of `AwaitNesting` wraps a call in.
const AWAIT_WRAP: usize = 60;

/// The costly kinds of nesting, as the end-to-end tests list them for
/// calls.
fn await_nesting_kinds() -> [AwaitNesting; 6] {
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

// ----- Memoised functions -----

/// Runs `text` on a fresh stack with `memo` as its memo tables.
fn run_with(text: &str, memo: &mut MemoTables) -> Result<Value, Diagnostic> {
    let ir = compile(text);
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    eval_with(&ir, &mut stack, memo, (None, &[], None))
}

/// The values of the fields of the record that `text` evaluates to, in
/// definition order.
fn fields(text: &str) -> Vec<Value> {
    match run(text) {
        Ok(Value::Record(record)) => record.values.to_vec(),
        other => panic!("{text}: expected a record, got {other:?}"),
    }
}

/// Whether `a` and `b` are one record, closure or future.
fn same(a: &Value, b: &Value) -> bool {
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
fn hits(setup: &str, ty: &str, first: &str, second: &str) -> bool {
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
fn memoised_closure() -> Rc<Closure> {
    match run("fn h() ->> Int { 1 }\nh") {
        Ok(Value::Fn(closure)) => closure,
        other => panic!("expected a closure, got {other:?}"),
    }
}

/// Starts a call of `closure` with no arguments in `memo`, as
/// [`Evaluator::memo_begin`] does, for a caller of origin `origin`,
/// which then reads the directory `dir` as its `fileroot.caller`.
fn start_reading(memo: &mut MemoTables, closure: &Rc<Closure>, origin: u64, dir: u32) -> Mark {
    let hash = key_hash(&[]);
    let entry = memo.start(closure, hash, &[]);
    memo.running.push(Running {
        closure: Rc::as_ptr(closure),
        hash,
        entry,
        origin,
    });
    memo.depend(origin, Ambient::Caller(dir));
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
fn found(
    memo: &mut MemoTables,
    closure: &Rc<Closure>,
    dir: u32,
) -> Result<Option<Value>, &'static str> {
    let caller = Caller {
        dir,
        helper: false,
        origin: NO_ORIGIN,
    };
    match memo.lookup(closure, &[], (caller, false)) {
        Lookup::Done(value) => Ok(Some(value)),
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
const SPIN: &str = "fn once(n: Int) -> Int { let g = fn(x: Int) ->> Int { x + n }\ng(1) }\n\
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
    eval_with(&ir, &mut frame, &mut memo, (None, &[], None)).unwrap();
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
    let Value::Record(record) = eval_with(&ir, &mut stack, &mut memo, (None, &[], None)).unwrap()
    else {
        panic!("expected a record");
    };
    // The running call's table survived the sweeps its body caused, and
    // the second call found its result.
    assert!(same(&record.values[0], &record.values[1]));
    assert_eq!(record.values[0].to_string(), "{v = 5150}");
    assert_eq!(memo.running_count(), 0);
    assert!(memo.table_count() < MIN_SWEEP_THRESHOLD);
}

// ----- Lists -----

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

// ----- Spreads -----

/// The value of `text`, as it would be written.
fn shown(text: &str) -> String {
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

// ----- Ranges and `for` -----

/// The span of the first occurrence of `needle` in `text`.
fn at(text: &str, needle: &str) -> Span {
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
const RANGE_NOTE: &str = "a range used as a list holds every element, and a list cannot hold \
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

// ----- Updating an element -----

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

// ----- Unions -----

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

// ----- `return`, labelled blocks and loops that return -----

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

/// dpkg's `verrevcmp`, transcribed from its C as it is, a byte past the
/// end read as C's terminating zero, then the byte-wise tie-break of
/// `sort -V`: what [`compare_versions`] must agree with.
fn dpkg_verrevcmp(a: &[u8], b: &[u8]) -> Ordering {
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    let order = |c: u8| -> i32 {
        if c.is_ascii_digit() {
            0
        } else if c.is_ascii_alphabetic() {
            i32::from(c)
        } else if c == b'~' {
            -1
        } else if c != 0 {
            i32::from(c) + 256
        } else {
            0
        }
    };
    let (mut i, mut j) = (0, 0);
    while at(a, i) != 0 || at(b, j) != 0 {
        let mut first_diff = 0;
        while (at(a, i) != 0 && !at(a, i).is_ascii_digit())
            || (at(b, j) != 0 && !at(b, j).is_ascii_digit())
        {
            let (ac, bc) = (order(at(a, i)), order(at(b, j)));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        while at(a, i) == b'0' {
            i += 1;
        }
        while at(b, j) == b'0' {
            j += 1;
        }
        while at(a, i).is_ascii_digit() && at(b, j).is_ascii_digit() {
            if first_diff == 0 {
                first_diff = i32::from(at(a, i)) - i32::from(at(b, j));
            }
            i += 1;
            j += 1;
        }
        if at(a, i).is_ascii_digit() {
            return Ordering::Greater;
        }
        if at(b, j).is_ascii_digit() {
            return Ordering::Less;
        }
        if first_diff != 0 {
            return first_diff.cmp(&0);
        }
    }
    a.cmp(b)
}

#[test]
fn typed_equality_compares_the_fields_a_rest_field_covers() {
    let prefix = "let a: {*: Int} = {x = 1; y = 2}\n\
                  let b: {*: Int} = {y = 2; x = 1}\n\
                  let c: {*: Int} = {x = 1; y = 3}\n\
                  let d: {*: Int} = {x = 1}\n\
                  let e: {*: Int} = {}\n\
                  let f: {n: String, *: Int} = {n = \"a\"; x = 1}\n\
                  let g: {n: String, *: Int} = {n = \"a\"; x = 2}\n\
                  let h: {n: String, *: Int} = {n = \"a\"}\n\
                  let i: {*: {*: Int}} = {p = {q = 1}}\n\
                  let j: {*: {*: Int}} = {p = {q = 1; r = 2}}\n";
    let cases = [
        // The same fields in another order, a value differing, a field
        // missing, and the empty record on either side.
        ("a == b", true),
        ("a == c", false),
        ("a == d", false),
        ("d == a", false),
        ("e == e", true),
        ("d == e", false),
        ("e == d", false),
        // Named fields and the others together.
        ("f == g", false),
        ("f != g", true),
        ("h == h", true),
        ("f == h", false),
        // Through a nested rest-field's shape.
        ("i == j", false),
        ("i == i", true),
    ];
    for (comparison, expected) in cases {
        assert_eq!(
            boolean(&format!("{prefix}{comparison}")),
            expected,
            "{comparison}"
        );
    }
}

#[test]
fn a_deep_await_awaits_the_fields_a_rest_field_covers() {
    let text = "let l: {n: Future<Int>, *: Future<Int>} = {n = lazy 1; p = lazy 1 + 1; q = lazy 3}\n\
                let m: {n: Int, *: Int} = l\n\
                let expected: {n: Int, *: Int} = {n = 1; p = 2; q = 3}\n\
                m == expected";
    assert!(boolean(text));
    let text = "let l: {*: {*: Future<String>}} = {a = {b = lazy \"x\"}}\n\
                let m: {*: {*: String}} = l\n\
                show(m)";
    assert_eq!(string(text), "{a = {b = \"x\"}}");
}

#[test]
fn as_tests_each_field_a_rest_field_covers() {
    let text = "let x = {a = 1; b = 2}\nlet y: {} = x\ny as {*: Int}";
    assert_eq!(run(text).unwrap().to_string(), "{a = 1; b = 2}");
    booleans(&[
        (
            "let x = {a = 1; b = \"s\"}\nlet y: {} = x\ny is {*: Int}",
            false,
        ),
        (
            "let x = {a = 1; b = \"s\"}\nlet y: {} = x\ny is {b: String, *: Int}",
            true,
        ),
        ("let y: {} = {}\ny is {*: Int}", true),
    ]);
    let text = "let x = {a = 1; \"b c\" = \"s\"}\nlet y: {a: Int} = x\ny as {*: Int}";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "`y` is not a `{*: Int}`");
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("it is a record, and its field `\"b c\"` is a String")
    );
}

#[test]
fn compare_versions_is_dpkg_s_order_and_total() {
    // Every string of up to three pieces of a small alphabet that has
    // each kind of byte: digits, a zero, letters, `~`, others, and a
    // character of two bytes.
    let alphabet = ["0", "1", "9", "a", "Z", "~", ".", "+", "\u{e9}"];
    let mut all = vec![String::new()];
    let mut last = vec![String::new()];
    for _ in 0..3 {
        last = last
            .iter()
            .flat_map(|s| alphabet.iter().map(move |piece| format!("{s}{piece}")))
            .collect();
        all.extend(last.iter().cloned());
    }
    for a in &all {
        for b in &all {
            let order = compare_versions(a, b);
            assert_eq!(
                order,
                dpkg_verrevcmp(a.as_bytes(), b.as_bytes()),
                "{a:?} {b:?}"
            );
            assert_eq!(order, compare_versions(b, a).reverse(), "{a:?} {b:?}");
            assert_eq!(order.is_eq(), a == b, "{a:?} {b:?}");
        }
    }
    // A total order: sorted by it, no string is after a later one.
    all.sort_by(|a, b| compare_versions(a, b));
    for (at, a) in all.iter().enumerate() {
        for b in &all[at..] {
            assert!(compare_versions(a, b).is_le(), "{a:?} {b:?}");
        }
    }
    // Digit runs far longer than any integer.
    let nines = "9".repeat(100);
    assert_eq!(
        compare_versions(&format!("1.{nines}"), &format!("1.1{nines}")),
        Ordering::Less
    );
    assert_eq!(
        compare_versions(&format!("000{nines}"), &nines),
        Ordering::Less
    );
}

#[test]
fn temporary_directories_are_made_when_asked_and_removed_when_dropped() {
    // Nothing is made until the first is asked for.
    let temp_dirs = TempDirs::new();
    assert!(temp_dirs.parent.get().is_none());
    drop(temp_dirs);
    let temp_dirs = TempDirs::new();
    let first = temp_dirs.make().unwrap();
    let parent = temp_dirs.parent.get().unwrap().clone();
    assert_eq!(Path::new(&first), parent.join("1"));
    let name = parent.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with(&format!("gcsl-{}-", std::process::id())),
        "{name}"
    );
    // A number something else took is passed over.
    fs::create_dir(parent.join("2")).unwrap();
    assert_eq!(Path::new(&temp_dirs.make().unwrap()), parent.join("3"));
    // Two owners have two parents.
    let other = TempDirs::new();
    let elsewhere = other.make().unwrap();
    assert_ne!(Path::new(&elsewhere).parent(), Some(parent.as_path()));
    fs::write(parent.join("1/file"), "x").unwrap();
    drop(temp_dirs);
    assert!(!parent.exists());
    assert!(Path::new(&elsewhere).is_dir());
    drop(other);
    assert!(!Path::new(&elsewhere).exists());
}

#[test]
fn temporary_directories_are_made_in_the_temp_root() {
    let scratch = std::env::temp_dir().join(format!("gcsl-slice-45-{}-unit", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    // A missing root is made, with its parents, as the first is asked
    // for, never before; the parent is in it, and goes, the root stays.
    let root = scratch.join("a/b/root");
    let mut temp_dirs = TempDirs::new();
    temp_dirs.set_root(root.clone());
    assert!(!scratch.exists());
    let first = temp_dirs.make().unwrap();
    let parent = temp_dirs.parent.get().unwrap().clone();
    assert_eq!(parent.parent(), Some(root.as_path()));
    assert_eq!(Path::new(&first), parent.join("1"));
    let name = parent.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with(&format!("gcsl-{}-", std::process::id())),
        "{name}"
    );
    drop(temp_dirs);
    assert!(!parent.exists());
    assert!(root.is_dir());
    // Set after one was made: the parent made stays in use.
    let mut temp_dirs = TempDirs::new();
    let first = temp_dirs.make().unwrap();
    let parent = temp_dirs.parent.get().unwrap().clone();
    temp_dirs.set_root(root.clone());
    let second = temp_dirs.make().unwrap();
    assert_eq!(Path::new(&first).parent(), Some(parent.as_path()));
    assert_eq!(Path::new(&second), parent.join("2"));
    assert_eq!(parent.parent(), Some(std::env::temp_dir().as_path()));
    drop(temp_dirs);
    assert!(!parent.exists());
    // A root that is not absolute is every call's error.
    let mut temp_dirs = TempDirs::new();
    temp_dirs.set_root(PathBuf::from("rel/tmp"));
    for _ in 0..2 {
        assert_eq!(
            temp_dirs.make(),
            Err("the temporary directory root `rel/tmp` is not an absolute path".to_owned())
        );
    }
    assert!(temp_dirs.parent.get().is_none());
    assert!(!Path::new("rel").exists());
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn a_shorthand_argument_passes_its_variable_to_its_parameter() {
    // `1` fills `a`, `c = 3` fills `c`, and `b` alone, after a named
    // argument, is `b = b`.
    let value = run(
        "fn f(a: Int, b: Int, c: Int): Int { a * 100 + b * 10 + c }\n\
         let b = 2\n= f(1, c = 3, b)",
    )
    .unwrap();
    assert_eq!(value.to_string(), "123");
}

#[test]
fn quantities_compute_exactly() {
    let cases = [
        ("30s + 1m", "1m30s"),
        ("1h - 1ns", "59m59s999ms999us999ns"),
        ("3 * 20s", "1m"),
        ("1m * 3", "3m"),
        ("1h // 7", "8m34s285ms714us285ns"),
        ("-1h // 7", "-8m34s285ms714us286ns"),
        ("1h // 7m", "8"),
        ("1h % 7m", "4m"),
        ("-(1h30m)", "-1h30m"),
        ("2GiB + 512MiB", "2560MiB"),
        ("1GiB // 3", "357913941B"),
        ("30s < 1m", "true"),
        ("1000ms == 1s", "true"),
        ("2KiB > 2KB", "true"),
        ("\"in {1m30s}, {2GiB}\"", "in 1m30s, 2GiB"),
        ("[1s, 1KB]", "[1s, 1KB]"),
        ("{ t = 1h }", "{t = 1h}"),
    ];
    for (text, shown) in cases {
        assert_eq!(run(text).unwrap().to_string(), shown, "{text}");
    }
}

#[test]
fn a_quantity_that_overflows_says_so_with_its_operands_as_literals() {
    let labels = |text: &str| -> (String, Vec<String>) {
        let error = run(text).unwrap_err();
        let labels = error
            .labels
            .iter()
            .map(|label| label.text.clone())
            .collect();
        (error.message, labels)
    };
    assert_eq!(
        labels("106751d * 2"),
        (
            "duration overflow".to_owned(),
            vec!["106751d".to_owned(), "2".to_owned()]
        )
    );
    assert_eq!(
        labels("8191PiB + 8191PiB"),
        (
            "size overflow".to_owned(),
            vec!["8191PiB".to_owned(), "8191PiB".to_owned()]
        )
    );
    assert_eq!(
        labels("let least = -106751d23h47m16s854ms775us807ns - 1ns\n-least"),
        (
            "duration overflow".to_owned(),
            vec!["-106751d23h47m16s854ms775us808ns".to_owned()]
        )
    );
    assert_eq!(
        labels("30s // 0"),
        (
            "division by zero".to_owned(),
            vec!["30s".to_owned(), "0".to_owned()]
        )
    );
    // An `Int`'s is as ever.
    assert_eq!(labels(&format!("{MAX} * 2")).0, "integer overflow");
}

#[test]
fn quantities_are_memo_keys_and_tell_apart_with_is() {
    let text = "fn f(t: Duration) ->> {v: Duration} { v = t }\n\
                    a = f(1s)\nb = f(1000ms)\nc = f(2s)";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert!(!same(&values[0], &values[2]));
    let text = "fn pick(x: Duration | Size | Int): String {\n\
                    \x20   if x is Duration { \"time {x}\" } else if x is Size { \"data {x}\" } else { \"{x}\" }\n\
                    }\n\
                    r = [pick(1s), pick(1B), pick(1)]";
    assert_eq!(
        fields(text)[0].to_string(),
        r#"["time 1s", "data 1B", "1"]"#
    );
}
