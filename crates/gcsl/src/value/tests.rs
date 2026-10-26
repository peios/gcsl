use super::*;
use crate::names::Names;

fn show(value: f64) -> String {
    Value::Float(value).to_string()
}

fn string(text: &str) -> Value {
    Value::Str(Rc::from(text))
}

/// A layout for `names`, interned into `table`.
fn layout(table: &mut Names, names: &[&str]) -> Rc<Layout> {
    Rc::new(Layout {
        names: names.iter().map(|text| table.intern(text)).collect(),
        texts: names.iter().map(|&text| Box::from(text)).collect(),
    })
}

fn record(layout: &Rc<Layout>, values: Vec<Value>) -> Value {
    Value::Record(Rc::new(Record {
        layout: Rc::clone(layout),
        values: values.into_boxed_slice(),
    }))
}

/// A record nested `depth` deep through the field `inner`, ending in
/// `leaf`.
fn nested(names: &mut Names, depth: usize, leaf: Value) -> Value {
    let layout = layout(names, &["inner"]);
    let mut value = leaf;
    for _ in 0..depth {
        value = record(&layout, vec![value]);
    }
    value
}

#[test]
fn comparing_deeply_nested_records_does_not_recurse() {
    // Far deeper than a test thread's stack could take recursively.
    let mut names = Names::new();
    let a = nested(&mut names, 1_000_000, Value::Int(0));
    let b = nested(&mut names, 1_000_000, Value::Int(0));
    let c = nested(&mut names, 1_000_000, Value::Int(1));
    assert_eq!(a, b);
    assert_ne!(a, c);
    // Different depths differ where one bottoms out.
    assert_ne!(a, nested(&mut names, 999_999, Value::Int(0)));
}

#[test]
fn dropping_a_deeply_nested_record_does_not_recurse() {
    // Far deeper than a test thread's stack could take recursively.
    let mut names = Names::new();
    let value = nested(&mut names, 1_000_000, Value::Int(0));
    // A shared nested record is released, not freed, and stays intact.
    let Value::Record(outer) = &value else {
        panic!("expected a record");
    };
    let shared = outer.values[0].clone();
    drop(value);
    let Value::Record(shared) = shared else {
        panic!("expected a record");
    };
    assert!(matches!(shared.values[0], Value::Record(_)));
    drop(shared);
}

#[test]
fn values_are_small() {
    // A string is a fat `Rc<str>` pointer, 16 bytes, plus the tag; a
    // record and a closure are thin pointers. A thin string pointer
    // would bring this back to 16 bytes; that is a later optimisation
    // for the VM's value representation, not this tree walker's.
    assert_eq!(size_of::<Value>(), 24);
    assert_eq!(size_of::<Rc<Record>>(), size_of::<usize>());
    assert_eq!(size_of::<Rc<Closure>>(), size_of::<usize>());
}

#[test]
fn ints_display_as_decimal() {
    assert_eq!(Value::Int(0).to_string(), "0");
    assert_eq!(Value::Int(-42).to_string(), "-42");
    assert_eq!(Value::Int(i64::MIN).to_string(), "-9223372036854775808");
}

#[test]
fn floats_always_show_their_type() {
    assert_eq!(show(3.0), "3.0");
    assert_eq!(show(1.5), "1.5");
    assert_eq!(show(-0.0), "-0.0");
    assert_eq!(show(0.1 + 0.2), "0.30000000000000004");
}

#[test]
fn non_finite_floats() {
    assert_eq!(show(f64::INFINITY), "inf");
    assert_eq!(show(f64::NEG_INFINITY), "-inf");
    assert_eq!(show(f64::NAN), "nan");
    assert_eq!(show(-f64::NAN), "nan");
}

#[test]
fn large_and_small_floats() {
    assert_eq!(show(1e300), "1e300");
    assert_eq!(show(1e16), "1e16");
    assert_eq!(show(1e15), "1000000000000000.0");
    assert_eq!(show(1.5e-7), "1.5e-7");
    assert_eq!(show(0.0001), "0.0001");
    assert_eq!(show(f64::MAX), "1.7976931348623157e308");
    assert_eq!(show(f64::MIN_POSITIVE), "2.2250738585072014e-308");
    assert_eq!(show(5e-324), "5e-324");
}

#[test]
fn kinds() {
    let mut names = Names::new();
    let xs = layout(&mut names, &["x"]);
    assert_eq!(Value::Int(1).kind(), ValueKind::Int);
    assert_eq!(Value::Float(1.0).kind(), ValueKind::Float);
    assert_eq!(Value::Bool(true).kind(), ValueKind::Bool);
    assert_eq!(string("a").kind(), ValueKind::String);
    assert_eq!(Value::Empty.kind(), ValueKind::Record);
    assert_eq!(record(&xs, vec![Value::Int(1)]).kind(), ValueKind::Record);
}

#[test]
fn the_empty_record_displays_as_empty_braces() {
    assert_eq!(Value::Empty.to_string(), "{}");
    assert_eq!(Value::Empty.repr().to_string(), "{}");
}

#[test]
fn bools_and_strings_display_plainly() {
    assert_eq!(Value::Bool(true).to_string(), "true");
    assert_eq!(Value::Bool(false).to_string(), "false");
    // A string's text, unquoted and unescaped.
    assert_eq!(string("say \"hi\"\n\t{x}").to_string(), "say \"hi\"\n\t{x}");
    assert_eq!(string("").to_string(), "");
}

#[test]
fn repr_quotes_strings_and_shows_everything_else_as_display() {
    assert_eq!(string("hello").repr().to_string(), "\"hello\"");
    assert_eq!(string("").repr().to_string(), "\"\"");
    assert_eq!(
        string("a\nb\tc\rd\\e\"f").repr().to_string(),
        "\"a\\nb\\tc\\rd\\\\e\\\"f\""
    );
    // Braces are doubled so the literal reads back; other characters
    // are verbatim.
    assert_eq!(string("{é}").repr().to_string(), "\"{{é}}\"");
    assert_eq!(Value::Int(-3).repr().to_string(), "-3");
    assert_eq!(Value::Float(2.0).repr().to_string(), "2.0");
    assert_eq!(Value::Float(f64::NAN).repr().to_string(), "nan");
    assert_eq!(Value::Bool(false).repr().to_string(), "false");
}

#[test]
fn records_display_as_gcsl_syntax() {
    let mut names = Names::new();
    let inner = record(&layout(&mut names, &["n"]), vec![Value::Int(2)]);
    let outer = record(
        &layout(&mut names, &["x", "y", "inner", "a b", "e"]),
        vec![
            Value::Int(1),
            string("a\n"),
            inner,
            Value::Float(1.0),
            Value::Empty,
        ],
    );
    // Definition order, separated by `;`, values in `repr` form, quoted
    // names.
    let shown = "{x = 1; y = \"a\\n\"; inner = {n = 2}; \"a b\" = 1.0; e = {}}";
    assert_eq!(outer.to_string(), shown);
    assert_eq!(outer.repr().to_string(), shown);
}

#[test]
fn records_compare_deeply_by_field_name() {
    let mut names = Names::new();
    let xy = layout(&mut names, &["x", "y"]);
    let yx = layout(&mut names, &["y", "x"]);
    let xz = layout(&mut names, &["x", "z"]);
    let x = layout(&mut names, &["x"]);
    let a = record(&xy, vec![Value::Int(1), string("s")]);
    // The same layout, or another with the same names in another order.
    assert_eq!(a, record(&xy, vec![Value::Int(1), string("s")]));
    assert_eq!(a, record(&yx, vec![string("s"), Value::Int(1)]));
    assert_ne!(a, record(&yx, vec![string("t"), Value::Int(1)]));
    assert_ne!(a, record(&xy, vec![Value::Int(2), string("s")]));
    // Different names, or more or fewer of them.
    assert_ne!(a, record(&xz, vec![Value::Int(1), string("s")]));
    assert_ne!(a, record(&x, vec![Value::Int(1)]));
    assert_ne!(record(&x, vec![Value::Int(1)]), a);
    assert_ne!(record(&x, vec![Value::Int(1)]), Value::Empty);
    assert_eq!(Value::Empty, Value::Empty);
    // Nested, and IEEE for floats.
    let nested = |value: f64| record(&x, vec![record(&x, vec![Value::Float(value)])]);
    assert_eq!(nested(1.0), nested(1.0));
    assert_eq!(nested(0.0), nested(-0.0));
    assert_ne!(nested(1.0), nested(2.0));
    assert_ne!(nested(f64::NAN), nested(f64::NAN));
    // Reading a field.
    let Value::Record(fields) = &a else {
        panic!("expected a record");
    };
    assert_eq!(fields.get(names.intern("y")), Some(&string("s")));
    assert_eq!(fields.get(names.intern("z")), None);
}

#[test]
fn cloning_a_string_shares_its_text() {
    let original = string("shared");
    let copy = original.clone();
    let (Value::Str(a), Value::Str(b)) = (&original, &copy) else {
        panic!("expected strings");
    };
    assert!(Rc::ptr_eq(a, b));
    assert_eq!(original, copy);
    // Equality compares content, not identity.
    assert_eq!(string("x"), string("x"));
    assert_ne!(string("x"), string("y"));
}

/// An IR with one function, displayed as `display`, whose body is `{}`.
fn one_function(display: &str) -> Rc<Ir> {
    let mut builder = crate::ir::IrBuilder::with_capacity(2);
    let body = builder.alloc(crate::ir::IrKind::Empty, crate::source::Span::new(0, 0));
    let root = builder.alloc(crate::ir::IrKind::Empty, crate::source::Span::new(0, 0));
    let code = crate::ir::FnCode {
        body,
        kind: crate::ir::FnKind::Function,
        frame_size: 0,
        param_count: 0,
        capture_count: 1,
        // The body is a leaf.
        height: 1,
        name: None,
        display: Box::from(display),
        slot_names: Box::default(),
        attributes: Box::default(),
        defers: false,
        retry: 0,
        timeout: None,
        name_text: None,
        needs: Box::default(),
    };
    Rc::new(builder.finish_program(root, (0, Vec::new()), vec![code], (0, 0)))
}

fn closure(code: &Rc<Ir>, captures: Vec<Value>) -> Value {
    Value::Fn(Rc::new(Closure {
        code: Rc::clone(code),
        function: FnId::new(0),
        captures: captures.into_boxed_slice(),
    }))
}

#[test]
fn a_closure_displays_as_its_type() {
    let code = one_function("fn(x: Int): Int");
    let value = closure(&code, vec![Value::Int(1)]);
    assert_eq!(value.kind(), ValueKind::Fn);
    assert_eq!(value.to_string(), "fn(x: Int): Int");
    assert_eq!(value.repr().to_string(), "fn(x: Int): Int");
    // In a record, as a field's value.
    let mut names = Names::new();
    let record = record(&layout(&mut names, &["f"]), vec![value.clone()]);
    assert_eq!(record.to_string(), "{f = fn(x: Int): Int}");
    // Its debug form shows the function and captures, not the IR.
    let shown = format!("{value:?}");
    assert!(shown.contains("function: FnId(0)"), "{shown}");
    assert!(shown.contains("type: \"fn(x: Int): Int\""), "{shown}");
    assert!(shown.contains("captures: [Int(1)]"), "{shown}");
    // A closure of a function its IR lacks shows as nothing.
    let stray = Value::Fn(Rc::new(Closure {
        code: Rc::clone(&code),
        function: FnId::new(5),
        captures: Box::default(),
    }));
    assert_eq!(stray.to_string(), "");
}

#[test]
fn closures_compare_by_identity() {
    let code = one_function("fn(): Int");
    let a = closure(&code, Vec::new());
    let b = closure(&code, Vec::new());
    assert_eq!(a, a.clone());
    assert_ne!(a, b);
    let mut names = Names::new();
    let fields = layout(&mut names, &["f"]);
    assert_eq!(
        record(&fields, vec![a.clone()]),
        record(&fields, vec![a.clone()])
    );
    assert_ne!(record(&fields, vec![a]), record(&fields, vec![b]));
}

#[test]
fn dropping_a_long_chain_of_closures_does_not_recurse() {
    // A million closures, each capturing the one before, far more than a
    // recursive drop could take on a test thread's stack.
    let code = one_function("fn(): Int");
    let mut value = Value::Int(0);
    for _ in 0..1_000_000 {
        value = closure(&code, vec![value]);
    }
    // A capture still held elsewhere is released, not freed.
    let Value::Fn(outer) = &value else {
        panic!("expected a closure");
    };
    let shared = outer.captures[0].clone();
    drop(value);
    let Value::Fn(shared) = shared else {
        panic!("expected a closure");
    };
    assert!(matches!(shared.captures[0], Value::Fn(_)));
    drop(shared);
    // Closures and records nested in each other, alternately.
    let mut names = Names::new();
    let fields = layout(&mut names, &["f"]);
    let mut value = Value::Empty;
    for index in 0..1_000_000 {
        value = if index % 2 == 0 {
            closure(&code, vec![value])
        } else {
            record(&fields, vec![value])
        };
    }
    drop(value);
    // The IR outlives every closure made from it only as long as they
    // hold it.
    assert_eq!(Rc::strong_count(&code), 1);
}

#[test]
fn a_built_in_function_is_a_function_value() {
    let int = Value::Builtin(Builtin::Int);
    assert_eq!(size_of::<Value>(), 24);
    assert_eq!(int.kind(), ValueKind::Fn);
    // Shown as its type, or for one whose typing is special, which has
    // none, as what it is.
    assert_eq!(int.to_string(), "fn(x: Float): Int");
    assert_eq!(int.repr().to_string(), "fn(x: Float): Int");
    assert_eq!(
        Value::Builtin(Builtin::Float).to_string(),
        "fn(x: Int): Float"
    );
    assert_eq!(Value::Builtin(Builtin::Len).to_string(), "builtin len");
    // Equal when the same built-in.
    assert_eq!(int, Value::Builtin(Builtin::Int));
    assert_ne!(int, Value::Builtin(Builtin::Float));
    let mut names = Names::new();
    let record = record(&layout(&mut names, &["f"]), vec![int]);
    assert_eq!(record.to_string(), "{f = fn(x: Float): Int}");
}

#[test]
fn exec_and_panic_show_as_their_types() {
    let exec = Value::Builtin(Builtin::Exec);
    assert_eq!(exec.kind(), ValueKind::Fn);
    assert_eq!(
        exec.to_string(),
        "fn(argv: List<String>, options: {cwd: String, env: {*: String}, interactive: Bool, \
         panic: Bool, until: String}) -> {status: Int, stderr: String, stdout: String}"
    );
    assert_eq!(exec.repr().to_string(), exec.to_string());
    assert_eq!(
        Value::Builtin(Builtin::Panic).to_string(),
        "fn(message: String): Never"
    );
    assert_ne!(exec, Value::Builtin(Builtin::Panic));
}

#[test]
fn int_to_float_rounds_to_nearest() {
    assert_eq!(int_to_float(3), 3.0);
    assert_eq!(int_to_float(9_007_199_254_740_993), 9_007_199_254_740_992.0);
    assert_eq!(int_to_float(i64::MIN), -9_223_372_036_854_775_808.0);
}

// ----- Lists -----

fn list(values: Vec<Value>) -> Value {
    Value::List(List::from_vec(values))
}

#[test]
fn lists_are_small_and_the_empty_list_allocates_nothing() {
    assert_eq!(size_of::<List>(), size_of::<usize>());
    assert_eq!(size_of::<Value>(), 24);
    let empty = List::from_vec(Vec::new());
    assert!(empty.is_empty());
    assert!(empty.shares(&List::EMPTY));
    assert_eq!(empty, List::EMPTY);
    assert!(List::from_exact(std::iter::empty()).is_empty());
    let one = List::from_exact([Value::Int(1)].into_iter());
    assert_eq!(one.len(), 1);
    assert_eq!(one.values(), &[Value::Int(1)]);
    assert!(one.shares(&one.clone()));
    assert!(!one.shares(&List::from_vec(vec![Value::Int(1)])));
    assert_eq!(list(Vec::new()).kind(), ValueKind::List);
}

#[test]
fn lists_display_their_elements_in_repr_form() {
    let mut names = Names::new();
    let record = record(&layout(&mut names, &["n"]), vec![Value::Int(2)]);
    let value = list(vec![
        Value::Int(1),
        string("a\n"),
        record,
        list(vec![Value::Float(1.0), list(Vec::new())]),
        Value::Empty,
    ]);
    let shown = "[1, \"a\\n\", {n = 2}, [1.0, []], {}]";
    assert_eq!(value.to_string(), shown);
    assert_eq!(value.repr().to_string(), shown);
    assert_eq!(list(Vec::new()).to_string(), "[]");
    let inside = record_of_list(&mut names);
    assert_eq!(inside.to_string(), "{xs = [\"b\"]}");
}

/// A record whose field `xs` is a list of one string.
fn record_of_list(names: &mut Names) -> Value {
    record(&layout(names, &["xs"]), vec![list(vec![string("b")])])
}

#[test]
fn lists_compare_element_by_element_in_order() {
    let mut names = Names::new();
    let xs = |values: &[i64]| list(values.iter().map(|&value| Value::Int(value)).collect());
    assert_eq!(xs(&[1, 2]), xs(&[1, 2]));
    assert_ne!(xs(&[1, 2]), xs(&[2, 1]));
    assert_ne!(xs(&[1, 2]), xs(&[1, 2, 3]));
    assert_ne!(xs(&[1, 2, 3]), xs(&[1, 2]));
    assert_eq!(xs(&[]), xs(&[]));
    assert_ne!(xs(&[]), Value::Empty);
    // Nested, and in records, deeply.
    assert_eq!(list(vec![xs(&[1])]), list(vec![xs(&[1])]));
    assert_ne!(list(vec![xs(&[1])]), list(vec![xs(&[2])]));
    assert_eq!(record_of_list(&mut names), record_of_list(&mut names));
    let floats = |value: f64| list(vec![Value::Float(value)]);
    assert_eq!(floats(0.0), floats(-0.0));
    assert_ne!(floats(f64::NAN), floats(f64::NAN));
}

/// A list nested `depth` deep, alternately in records through the
/// field `inner` if `records` says so, ending in `leaf`.
fn nested_list(names: &mut Names, depth: usize, records: bool, leaf: Value) -> Value {
    let layout = layout(names, &["inner"]);
    let mut value = leaf;
    for index in 0..depth {
        value = if records && index % 2 == 1 {
            record(&layout, vec![value])
        } else {
            list(vec![Value::Int(0), value])
        };
    }
    value
}

#[test]
fn comparing_and_dropping_deeply_nested_lists_does_not_recurse() {
    // Far deeper than a test thread's stack could take recursively,
    // lists in lists and lists and records alternately.
    let mut names = Names::new();
    for records in [false, true] {
        let a = nested_list(&mut names, 1_000_000, records, Value::Int(0));
        let b = nested_list(&mut names, 1_000_000, records, Value::Int(0));
        let c = nested_list(&mut names, 1_000_000, records, Value::Int(1));
        assert_eq!(a, b);
        assert_ne!(a, c);
        drop(a);
        drop(b);
        drop(c);
    }
    // A list still held elsewhere is released, not freed.
    let inner = List::from_vec(vec![Value::Int(1)]);
    let outer = list(vec![Value::List(inner.clone())]);
    drop(outer);
    assert_eq!(inner.values(), &[Value::Int(1)]);
}

// ----- Futures -----

/// A pending future of `code`'s one function, holding `captures`.
fn future(code: &Rc<Ir>, captures: Vec<Value>) -> Rc<Future> {
    Rc::new(Future::new(
        Rc::clone(code),
        FnId::new(0),
        captures.into_boxed_slice(),
    ))
}

#[test]
fn a_future_displays_as_its_type_and_compares_by_identity() {
    let code = one_function("Future<Int>");
    let pending = future(&code, vec![Value::Int(1)]);
    let value = Value::Future(Rc::clone(&pending));
    assert_eq!(value.kind(), ValueKind::Deferred);
    assert_eq!(value.to_string(), "Future<Int>");
    assert_eq!(value.repr().to_string(), "Future<Int>");
    let mut names = Names::new();
    let fields = layout(&mut names, &["f"]);
    assert_eq!(
        record(&fields, vec![value.clone()]).to_string(),
        "{f = Future<Int>}"
    );
    // Its debug form shows the function, the type and the state, and
    // neither the IR nor the values it holds.
    let shown = format!("{pending:?}");
    assert_eq!(
        shown,
        "Future { function: FnId(0), type: \"Future<Int>\", state: \"pending\", .. }"
    );
    assert!(!pending.is_done());
    assert!(pending.finish(FutureState::Done(Value::Int(2))));
    assert!(pending.is_done());
    assert_eq!(
        format!("{pending:?}"),
        "Future { function: FnId(0), type: \"Future<Int>\", state: \"done\", .. }"
    );
    // Done or not, it shows as its type.
    assert_eq!(value.to_string(), "Future<Int>");
    assert!(pending.finish(FutureState::Running));
    assert!(format!("{pending:?}").contains("state: \"running\""));
    // By identity: equal only to itself.
    let other = Value::Future(future(&code, vec![Value::Int(1)]));
    assert_eq!(value, value.clone());
    assert_ne!(value, other);
    // A future of a function its IR lacks shows as nothing.
    let stray = Future::new(Rc::clone(&code), FnId::new(5), Box::default());
    assert_eq!(stray.display(), "");
}

#[test]
fn beginning_an_await_takes_the_captures_and_marks_it_running() {
    let code = one_function("Future<Int>");
    let pending = future(&code, vec![Value::Int(7)]);
    let Some(Begin::Pending(captures)) = pending.begin() else {
        panic!("expected the captures");
    };
    assert_eq!(&*captures, &[Value::Int(7)]);
    assert!(matches!(pending.begin(), Some(Begin::Running)));
    assert!(pending.finish(FutureState::Done(Value::Int(8))));
    assert!(matches!(pending.begin(), Some(Begin::Done(Value::Int(8)))));
    // Still done.
    assert!(matches!(pending.begin(), Some(Begin::Done(Value::Int(8)))));
}

#[test]
fn dropping_a_long_chain_of_futures_does_not_recurse() {
    // A million pending futures, each capturing the one before.
    let code = one_function("Future<Int>");
    let mut value = Value::Int(0);
    for _ in 0..1_000_000 {
        value = Value::Future(future(&code, vec![value]));
    }
    drop(value);
    // A million done futures, each holding the one before as its value.
    let mut value = Value::Int(0);
    for _ in 0..1_000_000 {
        let done = future(&code, Vec::new());
        assert!(done.finish(FutureState::Done(value)));
        value = Value::Future(done);
    }
    drop(value);
    // Futures, closures and records nested in each other, alternately.
    let mut names = Names::new();
    let fields = layout(&mut names, &["f"]);
    let mut value = Value::Empty;
    for index in 0..1_000_000 {
        value = match index % 3 {
            0 => Value::Future(future(&code, vec![value])),
            1 => closure(&code, vec![value]),
            _ => record(&fields, vec![value]),
        };
    }
    drop(value);
    // A future still held elsewhere is released, not freed.
    let inner = future(&code, vec![Value::Int(1)]);
    let outer = Value::Future(future(&code, vec![Value::Future(Rc::clone(&inner))]));
    drop(outer);
    assert_eq!(Rc::strong_count(&inner), 1);
    drop(inner);
    assert_eq!(Rc::strong_count(&code), 1);
}

#[test]
fn a_quantity_shows_as_its_canonical_literal() {
    const S: i64 = 1_000_000_000;
    let cases = [
        (Quantity::Duration, 0, "0s"),
        (Quantity::Duration, 90 * S, "1m30s"),
        (Quantity::Duration, S + S / 2, "1s500ms"),
        (Quantity::Duration, -30 * S, "-30s"),
        (Quantity::Duration, 2 * 86_400 * S + 1, "2d1ns"),
        (
            Quantity::Duration,
            i64::MAX,
            "106751d23h47m16s854ms775us807ns",
        ),
        (
            Quantity::Duration,
            i64::MIN,
            "-106751d23h47m16s854ms775us808ns",
        ),
        (Quantity::Size, 0, "0B"),
        (Quantity::Size, 2 << 30, "2GiB"),
        (Quantity::Size, 1_500, "1500B"),
        (Quantity::Size, 1_000_000, "1MB"),
        // 1000KiB rather than 1024KB: the larger unit.
        (Quantity::Size, 1_024_000, "1000KiB"),
        (Quantity::Size, -512, "-512B"),
        (Quantity::Size, 1 << 50, "1PiB"),
    ];
    for (quantity, amount, shown) in cases {
        let value = Value::Quantity(quantity, amount);
        assert_eq!(value.to_string(), shown, "{amount}");
        assert_eq!(value.repr().to_string(), shown, "{amount}");
        assert_eq!(value.kind(), ValueKind::Quantity(quantity));
    }
}

#[test]
fn every_shown_quantity_is_a_literal_of_its_value() {
    // But the most negative duration, whose magnitude no literal reaches,
    // as no `Int` literal reaches `i64::MIN`'s.
    let amounts = [
        0,
        1,
        7,
        999,
        1_000,
        59_999_999_999,
        86_400_000_000_001,
        i64::MAX,
        -42,
    ];
    for quantity in [Quantity::Duration, Quantity::Size] {
        for amount in amounts {
            let shown = Value::Quantity(quantity, amount).to_string();
            let source =
                crate::source::SourceFile::new("t", shown.trim_start_matches('-')).unwrap();
            let (tokens, _, _) = crate::lexer::tokenize(&source).unwrap();
            let read = match tokens[0].kind {
                crate::lexer::TokenKind::Duration(read) | crate::lexer::TokenKind::Size(read) => {
                    read
                }
                other => panic!("{shown}: {other:?}"),
            };
            assert_eq!(if amount < 0 { -read } else { read }, amount, "{shown}");
        }
    }
}
