//! Tests of the evaluator: dotted fields, updates and typed equality of
//! records.

use super::*;

/// The record in `value`, which must be one.
pub(super) fn record_of(value: &Value) -> &Rc<Record> {
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
pub(super) fn nested_record(inner: Name, depth: usize, leaf: Value) -> Value {
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
