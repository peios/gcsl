use super::*;

#[test]
fn nodes_are_small() {
    // Guards the arena's density; grow this deliberately, not by accident.
    assert!(size_of::<IrNode>() <= 24);
}

#[test]
fn pretty_prints_nested_s_expressions() {
    let mut builder = IrBuilder::with_capacity(4);
    let seven = builder.alloc(IrKind::IntConst(7), Span::new(0, 1));
    let widened = builder.alloc(IrKind::IntToFloat(seven), Span::new(0, 1));
    let two = builder.alloc(IrKind::FloatConst(2.0), Span::new(4, 5));
    let div = builder.alloc(
        IrKind::FloatDiv(Operands {
            lhs: widened,
            rhs: two,
            op_start: 2,
        }),
        Span::new(0, 5),
    );
    let ir = builder.finish(div, 0, Vec::new());
    assert_eq!(ir.root(), div);
    assert_eq!(ir.get(two).kind, IrKind::FloatConst(2.0));
    assert_eq!(
        ir.pretty(&Names::new()).to_string(),
        "(float-div @0..5\n  (int-to-float @0..1\n    (int 7 @0..1))\n  (float 2.0 @4..5))"
    );
}

#[test]
fn pretty_prints_blocks_and_slots() {
    let mut names = Names::new();
    let x = names.intern("x");
    let mut builder = IrBuilder::with_capacity(6);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(8, 9));
    let store = builder.alloc(IrKind::Store(Slot::new(0), one), Span::new(0, 9));
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Int),
        Span::new(10, 11),
    );
    let items = builder.push_item_list(&[store]);
    let block = builder.alloc(
        IrKind::Block {
            items,
            result: load,
        },
        Span::new(0, 11),
    );
    let nothing = builder.alloc(IrKind::Empty, Span::new(12, 14));
    let empty = builder.push_item_list(&[]);
    let empty_block = builder.alloc(
        IrKind::Block {
            items: empty,
            result: nothing,
        },
        Span::new(12, 14),
    );
    let outer_items = builder.push_item_list(&[empty_block]);
    let root = builder.alloc(
        IrKind::Block {
            items: outer_items,
            result: block,
        },
        Span::new(0, 14),
    );
    let ir = builder.finish(root, 2, vec![x]);
    assert_eq!(ir.frame_size(), 2);
    assert_eq!(ir.items(outer_items), &[empty_block]);
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(block @0..14\n\
             \x20 (block @12..14\n\
             \x20   (empty @12..14))\n\
             \x20 (block @0..11\n\
             \x20   (store x#0 @0..9\n\
             \x20     (int 1 @8..9))\n\
             \x20   (load x#0 @10..11)))"
    );
    // A slot the IR has no name for.
    let mut builder = IrBuilder::with_capacity(1);
    let load = builder.alloc(IrKind::Load(Slot::new(3), ValueKind::Int), Span::new(0, 1));
    let ir = builder.finish(load, 4, Vec::new());
    assert_eq!(ir.pretty(&names).to_string(), "(load ?#3 @0..1)");
}

#[test]
fn operator_spans() {
    let operands = Operands {
        lhs: IrId(0),
        rhs: IrId(1),
        op_start: 4,
    };
    let node = |kind| IrNode {
        kind,
        span: Span::new(2, 9),
    };
    assert_eq!(
        node(IrKind::IntFloorDiv(operands)).operator_span(),
        Some(Span::new(4, 6))
    );
    assert_eq!(
        node(IrKind::FloatAdd(operands)).operator_span(),
        Some(Span::new(4, 5))
    );
    assert_eq!(
        node(IrKind::IntNeg(IrId(0))).operator_span(),
        Some(Span::new(2, 3))
    );
    assert_eq!(node(IrKind::IntConst(1)).operator_span(), None);
    assert_eq!(node(IrKind::IntToFloat(IrId(0))).operator_span(), None);
    assert_eq!(
        node(IrKind::Load(Slot::new(0), ValueKind::Int)).operator_span(),
        None
    );
    assert_eq!(
        node(IrKind::Store(Slot::new(0), IrId(0))).operator_span(),
        None
    );
}

#[test]
fn node_kinds() {
    let mut names = Names::new();
    let x = names.intern("x");
    let mut builder = IrBuilder::with_capacity(8);
    let int = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
    let rem = builder.alloc(
        IrKind::IntRem(Operands {
            lhs: int,
            rhs: int,
            op_start: 0,
        }),
        Span::new(0, 1),
    );
    let float = builder.alloc(IrKind::IntToFloat(int), Span::new(0, 1));
    let nothing = builder.alloc(IrKind::Empty, Span::new(0, 1));
    let store = builder.alloc(IrKind::Store(Slot::new(0), int), Span::new(0, 1));
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Float),
        Span::new(0, 1),
    );
    let layout = builder.push_layout(Layout {
        names: Box::new([x]),
        texts: Box::new([Box::from("x")]),
    });
    let values = builder.push_item_list(&[load]);
    let record = builder.alloc(IrKind::MakeRecord { layout, values }, Span::new(0, 1));
    let field = builder.alloc(
        IrKind::Field {
            record,
            name: x,
            kind: ValueKind::Float,
        },
        Span::new(0, 1),
    );
    let operands = Operands {
        lhs: record,
        rhs: record,
        op_start: 0,
    };
    let shape = builder.push_shape(Shape {
        fields: Box::new([(x, None)]),
        rest: None,
    });
    let shaped = ShapedOperands {
        shape,
        lhs: record,
        rhs: record,
    };
    let record_eq = builder.alloc(IrKind::RecordEq(shaped), Span::new(0, 1));
    let record_ne = builder.alloc(IrKind::RecordNe(shaped), Span::new(0, 1));
    let record_same = builder.alloc(IrKind::RecordSame(operands), Span::new(0, 1));
    let record_not_same = builder.alloc(IrKind::RecordNotSame(operands), Span::new(0, 1));
    let update = builder.alloc(
        IrKind::UpdateField {
            record,
            name: x,
            value: float,
        },
        Span::new(0, 1),
    );
    let items = builder.push_item_list(&[store]);
    let inner = builder.alloc(
        IrKind::Block {
            items,
            result: float,
        },
        Span::new(0, 1),
    );
    let outer = builder.alloc(
        IrKind::Block {
            items,
            result: inner,
        },
        Span::new(0, 1),
    );
    let ir = builder.finish(outer, 1, Vec::new());
    assert_eq!(ir.kind_of(int), ValueKind::Int);
    assert_eq!(ir.kind_of(rem), ValueKind::Int);
    assert_eq!(ir.kind_of(float), ValueKind::Float);
    assert_eq!(ir.kind_of(nothing), ValueKind::Record);
    assert_eq!(ir.kind_of(store), ValueKind::Record);
    assert_eq!(ir.kind_of(load), ValueKind::Float);
    assert_eq!(ir.kind_of(record), ValueKind::Record);
    assert_eq!(ir.kind_of(field), ValueKind::Float);
    for comparison in [record_eq, record_ne, record_same, record_not_same] {
        assert_eq!(ir.kind_of(comparison), ValueKind::Bool);
    }
    assert_eq!(ir.kind_of(update), ValueKind::Record);
    assert_eq!(ir.kind_of(inner), ValueKind::Float);
    assert_eq!(ir.kind_of(outer), ValueKind::Float);
    assert_eq!(ir.layout_count(), 1);
    assert_eq!(&*ir.layout(layout).names, &[x]);
    assert_eq!(ir.shape_count(), 1);
    assert_eq!(&*ir.shape(shape).fields, &[(x, None)]);
}

#[test]
fn records_show_their_layout_and_fields_their_name() {
    let mut names = Names::new();
    let (x, spaced) = (names.intern("x"), names.intern("a b"));
    let mut builder = IrBuilder::with_capacity(5);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(6, 7));
    let two = builder.alloc(IrKind::IntConst(2), Span::new(17, 18));
    let layout = builder.push_layout(Layout {
        names: Box::new([x, spaced]),
        texts: Box::new([Box::from("x"), Box::from("a b")]),
    });
    let values = builder.push_item_list(&[one, two]);
    let record = builder.alloc(IrKind::MakeRecord { layout, values }, Span::new(0, 20));
    let field = builder.alloc(
        IrKind::Field {
            record,
            name: spaced,
            kind: ValueKind::Int,
        },
        Span::new(0, 26),
    );
    let ir = builder.finish(field, 0, Vec::new());
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(field \"a b\" @0..26\n\
             \x20 (make-record [x \"a b\"] @0..20\n\
             \x20   (int 1 @6..7)\n\
             \x20   (int 2 @17..18)))"
    );
    assert_eq!(ir.get(field).operator_span(), None);
    assert_eq!(ir.get(record).kind.symbol(), None);
}

#[test]
fn cmp_follows_partial_ord() {
    let all = [Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge];
    let results = |lhs: f64, rhs: f64| all.map(|cmp| cmp.test(&lhs, &rhs));
    assert_eq!(results(1.0, 2.0), [false, true, true, true, false, false]);
    assert_eq!(results(2.0, 2.0), [true, false, false, true, false, true]);
    assert_eq!(results(-0.0, 0.0), [true, false, false, true, false, true]);
    // NaN is unequal and unordered to everything, itself included.
    assert_eq!(
        results(f64::NAN, f64::NAN),
        [false, true, false, false, false, false]
    );
    assert_eq!(
        results(f64::NAN, 1.0),
        [false, true, false, false, false, false]
    );
    // Strings compare bytewise.
    assert!(Cmp::Lt.test("Z", "a"));
    assert!(Cmp::Lt.test("", "a"));
    assert!(Cmp::Lt.test("ab", "b"));
    assert!(Cmp::Ge.test("é", "z"));
    assert_eq!(all.map(Cmp::symbol), ["==", "!=", "<", "<=", ">", ">="]);
}

#[test]
fn strings_bools_and_ifs() {
    let mut names = Names::new();
    let s = names.intern("s");
    let mut builder = IrBuilder::with_capacity(12);
    let greeting = builder.push_string("hi \"you\"\n");
    let empty = builder.push_string("");
    let piece = builder.alloc(IrKind::StrConst(greeting), Span::new(0, 14));
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::String),
        Span::new(4, 5),
    );
    let tail = builder.alloc(IrKind::StrConst(empty), Span::new(0, 14));
    let pieces = builder.push_item_list(&[piece, load, tail]);
    let interp = builder.alloc(IrKind::Interp(pieces), Span::new(0, 14));
    let truth = builder.alloc(IrKind::BoolConst(true), Span::new(18, 22));
    let not = builder.alloc(IrKind::Not(truth), Span::new(14, 22));
    let one = builder.alloc(IrKind::IntConst(1), Span::new(23, 24));
    let two = builder.alloc(IrKind::IntConst(2), Span::new(27, 28));
    let cmp = builder.alloc(
        IrKind::IntCmp(
            Cmp::Le,
            Operands {
                lhs: one,
                rhs: two,
                op_start: 25,
            },
        ),
        Span::new(23, 28),
    );
    let and = builder.alloc(
        IrKind::And(Operands {
            lhs: not,
            rhs: cmp,
            op_start: 30,
        }),
        Span::new(14, 28),
    );
    let concat = builder.alloc(
        IrKind::StrConcat(Operands {
            lhs: interp,
            rhs: interp,
            op_start: 40,
        }),
        Span::new(35, 45),
    );
    let if_ = builder.alloc(
        IrKind::If {
            cond: and,
            then: interp,
            else_: concat,
        },
        Span::new(0, 45),
    );
    let ir = builder.finish(if_, 1, vec![s]);
    assert_eq!(&**ir.string(greeting), "hi \"you\"\n");
    assert_eq!(&**ir.string(empty), "");
    assert_eq!(ir.kind_of(if_), ValueKind::String);
    assert_eq!(ir.kind_of(and), ValueKind::Bool);
    assert_eq!(ir.kind_of(cmp), ValueKind::Bool);
    assert_eq!(ir.kind_of(not), ValueKind::Bool);
    assert_eq!(ir.kind_of(truth), ValueKind::Bool);
    assert_eq!(ir.kind_of(concat), ValueKind::String);
    assert_eq!(ir.kind_of(interp), ValueKind::String);
    assert_eq!(ir.get(cmp).operator_span(), Some(Span::new(25, 27)));
    assert_eq!(ir.get(and).operator_span(), Some(Span::new(30, 33)));
    assert_eq!(ir.get(not).operator_span(), Some(Span::new(14, 17)));
    assert_eq!(ir.get(concat).operator_span(), Some(Span::new(40, 41)));
    assert_eq!(ir.get(if_).operator_span(), None);
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(if @0..45\n\
             \x20 (and @14..28\n\
             \x20   (not @14..22\n\
             \x20     (bool true @18..22))\n\
             \x20   (int-cmp <= @23..28\n\
             \x20     (int 1 @23..24)\n\
             \x20     (int 2 @27..28)))\n\
             \x20 (interp @0..14\n\
             \x20   (str \"hi \\\"you\\\"\\n\" @0..14)\n\
             \x20   (load s#0 @4..5)\n\
             \x20   (str \"\" @0..14))\n\
             \x20 (str-concat @35..45\n\
             \x20   (interp @0..14\n\
             \x20     (str \"hi \\\"you\\\"\\n\" @0..14)\n\
             \x20     (load s#0 @4..5)\n\
             \x20     (str \"\" @0..14))\n\
             \x20   (interp @0..14\n\
             \x20     (str \"hi \\\"you\\\"\\n\" @0..14)\n\
             \x20     (load s#0 @4..5)\n\
             \x20     (str \"\" @0..14))))"
    );
}

#[test]
fn equality_of_bools_and_records_shows_its_operator() {
    let mut names = Names::new();
    let (x, inner, n, spaced) = (
        names.intern("x"),
        names.intern("inner"),
        names.intern("n"),
        names.intern("a b"),
    );
    let mut builder = IrBuilder::with_capacity(8);
    let nothing = builder.alloc(IrKind::Empty, Span::new(0, 2));
    let operands = Operands {
        lhs: nothing,
        rhs: nothing,
        op_start: 3,
    };
    // A shape given its id before its fields, as lowering does.
    let outer = builder.push_shape(Shape {
        fields: Box::default(),
        rest: None,
    });
    let nested = builder.push_shape(Shape {
        fields: Box::new([(n, None), (spaced, None)]),
        rest: None,
    });
    builder.set_shape(
        outer,
        Shape {
            fields: Box::new([(x, None), (inner, Some(nested))]),
            rest: None,
        },
    );
    let empty = builder.push_shape(Shape {
        fields: Box::default(),
        rest: None,
    });
    let shaped = |shape| ShapedOperands {
        shape,
        lhs: nothing,
        rhs: nothing,
    };
    let eq = builder.alloc(IrKind::RecordEq(shaped(outer)), Span::new(0, 8));
    let ne = builder.alloc(IrKind::RecordNe(shaped(empty)), Span::new(0, 8));
    let same = builder.alloc(IrKind::RecordSame(operands), Span::new(0, 9));
    let not_same = builder.alloc(IrKind::RecordNotSame(operands), Span::new(0, 9));
    let bool_ne = builder.alloc(IrKind::BoolEq(false, operands), Span::new(0, 8));
    let items = builder.push_item_list(&[eq, ne, same, not_same]);
    let root = builder.alloc(
        IrKind::Block {
            items,
            result: bool_ne,
        },
        Span::new(0, 9),
    );
    let ir = builder.finish(root, 0, Vec::new());
    let symbols = [eq, ne, same, not_same, bool_ne].map(|id| ir.get(id).kind.symbol());
    assert_eq!(
        symbols,
        [Some("=="), Some("!="), Some("==="), Some("!=="), Some("!=")]
    );
    // A typed record comparison cannot fail, and keeps no position.
    assert_eq!(ir.get(eq).operator_span(), None);
    assert_eq!(ir.get(ne).operator_span(), None);
    assert_eq!(ir.get(same).operator_span(), Some(Span::new(3, 6)));
    assert_eq!(ir.get(not_same).operator_span(), Some(Span::new(3, 6)));
    assert_eq!(ir.get(bool_ne).operator_span(), Some(Span::new(3, 5)));
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(block @0..9\n\
             \x20 (record-eq == [x inner[n \"a b\"]] @0..8\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (record-eq != [] @0..8\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (record-same === @0..9\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (record-same !== @0..9\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2))\n\
             \x20 (bool-eq != @0..8\n\
             \x20   (empty @0..2)\n\
             \x20   (empty @0..2)))"
    );
}

/// A program of every list node, all of one list, `[1, ...[1]]`, with
/// `x` interned into `names`: the IR, and the list, the spread, an
/// index, a length, the four comparisons (over a list of records
/// compared over `[x]`, a list of values, and whole) and the root, a
/// block whose result is a deep await of a list of lists.
fn list_program(names: &mut Names) -> (Ir, [IrId; 9]) {
    let x = names.intern("x");
    let mut builder = IrBuilder::with_capacity(12);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(1, 2));
    let inner_items = builder.push_item_list(&[one]);
    let inner = builder.alloc(
        IrKind::MakeList {
            items: inner_items,
            spreads: false,
        },
        Span::new(7, 10),
    );
    let spread = builder.alloc(IrKind::Spread(inner), Span::new(4, 10));
    let items = builder.push_item_list(&[one, spread]);
    let list = builder.alloc(
        IrKind::MakeList {
            items,
            spreads: true,
        },
        Span::new(0, 11),
    );
    let index = builder.alloc(
        IrKind::Index {
            list,
            index: one,
            kind: ValueKind::Int,
        },
        Span::new(0, 14),
    );
    let len = builder.alloc(IrKind::ListLen(list), Span::new(0, 16));
    // A list of records compared over `[x]`, and one of values.
    let record = builder.push_shape(Shape {
        fields: Box::new([(x, None)]),
        rest: None,
    });
    let records = builder.push_shape(Shape {
        fields: Box::default(),
        rest: None,
    });
    builder.set_list_shape(
        records,
        ListShape {
            element: Some(record),
        },
    );
    let values = builder.push_shape(Shape {
        fields: Box::default(),
        rest: None,
    });
    builder.set_list_shape(values, ListShape { element: None });
    let shaped = |shape| ShapedOperands {
        shape,
        lhs: list,
        rhs: list,
    };
    let eq = builder.alloc(IrKind::ListEq(shaped(records)), Span::new(0, 8));
    let ne = builder.alloc(IrKind::ListNe(shaped(values)), Span::new(0, 8));
    let operands = Operands {
        lhs: list,
        rhs: list,
        op_start: 3,
    };
    let same = builder.alloc(IrKind::ListSame(operands), Span::new(0, 9));
    let not_same = builder.alloc(IrKind::ListNotSame(operands), Span::new(0, 9));
    // A deep await of a list of lists whose elements are awaited.
    let each = builder.push_lazy_list(LazyField::Await(LazyShapeId::NONE));
    let lists = builder.push_lazy_list(LazyField::Within(each));
    let await_ = builder.alloc(
        IrKind::Await {
            value: list,
            kind: ValueKind::List,
            shape: lists,
            depth: 0,
        },
        Span::new(0, 11),
    );
    let block_items = builder.push_item_list(&[index, len, eq, ne, same, not_same]);
    let root = builder.alloc(
        IrKind::Block {
            items: block_items,
            result: await_,
        },
        Span::new(0, 16),
    );
    let ir = builder.finish(root, 0, Vec::new());
    (ir, [list, spread, index, len, eq, ne, same, not_same, root])
}

#[test]
fn list_nodes_have_kinds_symbols_and_shapes() {
    let (ir, [list, spread, index, len, eq, ne, same, not_same, root]) =
        list_program(&mut Names::new());
    for (id, kind) in [
        (list, ValueKind::List),
        (spread, ValueKind::List),
        (index, ValueKind::Int),
        (len, ValueKind::Int),
        (eq, ValueKind::Bool),
        (ne, ValueKind::Bool),
        (same, ValueKind::Bool),
        (not_same, ValueKind::Bool),
        (root, ValueKind::List),
    ] {
        assert_eq!(ir.kind_of(id), kind, "{:?}", ir.get(id));
    }
    let symbols = [eq, ne, same, not_same, index, len].map(|id| ir.get(id).kind.symbol());
    assert_eq!(
        symbols,
        [Some("=="), Some("!="), Some("==="), Some("!=="), None, None]
    );
    assert_eq!(ir.get(eq).operator_span(), None);
    assert_eq!(ir.get(same).operator_span(), Some(Span::new(3, 6)));
    let IrKind::ListEq(ShapedOperands { shape: records, .. }) = ir.get(eq).kind else {
        panic!("expected a list comparison");
    };
    let Some(ListShape {
        element: Some(record),
    }) = ir.list_shape(records)
    else {
        panic!("expected a list's shape");
    };
    assert_eq!(ir.list_shape(record), None);
    assert_eq!(ir.shape(record).fields.len(), 1);
    assert_eq!(ir.shape_count(), 3);
    let IrKind::Await { shape: lists, .. } = ir.get(root_result(&ir, root)).kind else {
        panic!("expected an await");
    };
    let Some(LazyField::Within(each)) = ir.lazy_list(lists) else {
        panic!("expected a list's lazy shape");
    };
    assert_eq!(
        ir.lazy_list(each),
        Some(LazyField::Await(LazyShapeId::NONE))
    );
    assert_eq!(ir.lazy_shape_count(), 2);
}

/// The result of the block `root` of `ir`.
fn root_result(ir: &Ir, root: IrId) -> IrId {
    match ir.get(root).kind {
        IrKind::Block { result, .. } => result,
        other => panic!("expected a block, got {other:?}"),
    }
}

#[test]
fn list_nodes_print() {
    let mut names = Names::new();
    let (ir, _) = list_program(&mut names);
    // Each node shows its details and its children; the list, shared by
    // every node here, the same each time.
    let shown = ir.pretty(&names).to_string();
    let list = "(make-list @0..11\n\
                    \x20     (int 1 @1..2)\n\
                    \x20     (spread @4..10\n\
                    \x20       (make-list @7..10\n\
                    \x20         (int 1 @1..2))))";
    for node in [
        format!("\n  (index @0..14\n    {list}\n    (int 1 @1..2))\n"),
        format!("\n  (list-len @0..16\n    {list})\n"),
        format!("\n  (list-eq == *[x] @0..8\n    {list}\n    {list})\n"),
        format!("\n  (list-eq != * @0..8\n    {list}\n"),
        format!("\n  (list-same === @0..9\n    {list}\n"),
        format!("\n  (list-same !== @0..9\n    {list}\n"),
        format!("\n  (await **! @0..11\n    {list}))"),
    ] {
        assert!(shown.contains(&node), "{node}: {shown}");
    }
}

#[test]
fn ranges_and_fors_have_kinds_children_and_print() {
    // `for i in 0..3 if b { i }` and `for-each` of it over `0..=3`, by
    // hand, with `i` in slot 0 and `b` in slot 1.
    let mut names = Names::new();
    let slot_names = vec![names.intern("i"), names.intern("b")];
    let mut builder = IrBuilder::with_capacity(12);
    let zero = builder.alloc(IrKind::IntConst(0), Span::new(9, 10));
    let three = builder.alloc(IrKind::IntConst(3), Span::new(12, 13));
    let operands = |op_start| Operands {
        lhs: zero,
        rhs: three,
        op_start,
    };
    let range = builder.alloc(IrKind::Range(false, operands(10)), Span::new(9, 13));
    let inclusive = builder.alloc(IrKind::Range(true, operands(10)), Span::new(9, 14));
    let guard = builder.alloc(
        IrKind::Load(Slot::new(1), ValueKind::Bool),
        Span::new(17, 18),
    );
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Int),
        Span::new(21, 22),
    );
    let no_items = builder.push_item_list(&[]);
    let body = builder.alloc(
        IrKind::Block {
            items: no_items,
            result: load,
        },
        Span::new(19, 24),
    );
    let parts = builder.push_item_list(&[range, guard, body]);
    let collecting = builder.alloc(
        IrKind::For {
            slot: Slot::new(0),
            parts,
            collect: true,
        },
        Span::new(0, 24),
    );
    let parts = builder.push_item_list(&[inclusive, body]);
    let discarding = builder.alloc(
        IrKind::For {
            slot: Slot::new(0),
            parts,
            collect: false,
        },
        Span::new(0, 25),
    );
    let block_items = builder.push_item_list(&[discarding]);
    let root = builder.alloc(
        IrKind::Block {
            items: block_items,
            result: collecting,
        },
        Span::new(0, 25),
    );
    let ir = builder.finish(root, 2, slot_names);
    for (id, kind) in [
        (range, ValueKind::List),
        (inclusive, ValueKind::List),
        (collecting, ValueKind::List),
        (discarding, ValueKind::Record),
        (root, ValueKind::List),
    ] {
        assert_eq!(ir.kind_of(id), kind, "{:?}", ir.get(id));
    }
    let symbols = [range, inclusive, collecting].map(|id| ir.get(id).kind.symbol());
    assert_eq!(symbols, [Some(".."), Some("..="), None]);
    assert_eq!(ir.get(range).operator_span(), Some(Span::new(10, 12)));
    assert_eq!(ir.get(inclusive).operator_span(), Some(Span::new(10, 13)));
    assert_eq!(ir.get(collecting).operator_span(), None);
    // A `for`'s children are its parts, and a range's its bounds.
    assert_eq!(
        ir.children(collecting).collect::<Vec<_>>(),
        vec![range, guard, body]
    );
    assert_eq!(ir.children(range).collect::<Vec<_>>(), vec![zero, three]);
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(block @0..25\n\
             \x20 (for-each i#0 @0..25\n\
             \x20   (range-inclusive @9..14\n\
             \x20     (int 0 @9..10)\n\
             \x20     (int 3 @12..13))\n\
             \x20   (block @19..24\n\
             \x20     (load i#0 @21..22)))\n\
             \x20 (for i#0 if @0..24\n\
             \x20   (range @9..13\n\
             \x20     (int 0 @9..10)\n\
             \x20     (int 3 @12..13))\n\
             \x20   (load b#1 @17..18)\n\
             \x20   (block @19..24\n\
             \x20     (load i#0 @21..22))))"
    );
}

#[test]
fn updates_show_their_name_record_and_value() {
    let mut names = Names::new();
    let (b, spaced) = (names.intern("b"), names.intern("c d"));
    let mut builder = IrBuilder::with_capacity(5);
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Record),
        Span::new(4, 5),
    );
    let nothing = builder.alloc(IrKind::Empty, Span::new(4, 7));
    let one = builder.alloc(IrKind::IntConst(1), Span::new(16, 17));
    let inner = builder.alloc(
        IrKind::UpdateField {
            record: nothing,
            name: spaced,
            value: one,
        },
        Span::new(4, 13),
    );
    let outer = builder.alloc(
        IrKind::UpdateField {
            record: load,
            name: b,
            value: inner,
        },
        Span::new(4, 7),
    );
    builder.add_name_text(b, "b");
    builder.add_name_text(spaced, "c d");
    // Recording a name twice keeps the first text.
    builder.add_name_text(b, "other");
    let ir = builder.finish(outer, 1, vec![b]);
    assert_eq!(ir.name_text(b), Some("b"));
    assert_eq!(ir.name_text(spaced), Some("c d"));
    assert_eq!(ir.name_text(names.intern("unused")), None);
    assert_eq!(ir.get(outer).kind.symbol(), None);
    assert_eq!(ir.get(outer).operator_span(), None);
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(update-field b @4..7\n\
             \x20 (load b#0 @4..5)\n\
             \x20 (update-field \"c d\" @4..13\n\
             \x20   (empty @4..7)\n\
             \x20   (int 1 @16..17)))"
    );
}

#[test]
fn a_dotted_update_shows_its_path_and_its_parts() {
    // `r.a[i]."c d".e := 1` by hand, its record and the element awaited
    // and `"c d"` created, which no checked program does together.
    let mut names = Names::new();
    let (a, spaced, e) = (names.intern("a"), names.intern("c d"), names.intern("e"));
    let mut builder = IrBuilder::with_capacity(6);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(23, 24));
    let load = builder.alloc(
        IrKind::Load(Slot::new(0), ValueKind::Deferred),
        Span::new(0, 1),
    );
    let root = builder.alloc(
        IrKind::Await {
            value: load,
            kind: ValueKind::Record,
            shape: LazyShapeId::NONE,
            depth: 0,
        },
        Span::new(0, 1),
    );
    let i = builder.alloc(IrKind::Load(Slot::new(1), ValueKind::Int), Span::new(4, 5));
    let parts = builder.push_item_list(&[one, root, i]);
    let step = |at, mode, end| UpdateStep {
        at,
        mode,
        span: Span::new(0, end),
    };
    let path = builder.push_update_path(&[
        step(StepAt::Field(a), StepMode::Read, 3),
        step(StepAt::Element, StepMode::Await, 6),
        step(StepAt::Field(spaced), StepMode::Create, 12),
        step(StepAt::Field(e), StepMode::Set, 14),
    ]);
    let update = builder.alloc(
        IrKind::Update {
            parts,
            path,
            kind: ValueKind::Record,
            depth: 0,
        },
        Span::new(0, 14),
    );
    let store = builder.alloc(IrKind::Store(Slot::new(0), update), Span::new(0, 24));
    let items = builder.push_item_list(&[store]);
    let empty = builder.alloc(IrKind::Empty, Span::new(0, 24));
    let block = builder.alloc(
        IrKind::Block {
            items,
            result: empty,
        },
        Span::new(0, 24),
    );
    let ir = builder.finish(block, 2, vec![names.intern("r"), names.intern("i")]);
    assert_eq!(ir.kind_of(update), ValueKind::Record);
    assert_eq!(ir.get(update).kind.symbol(), None);
    assert_eq!(ir.get(update).operator_span(), None);
    assert_eq!(ir.children(update).collect::<Vec<_>>(), vec![one, root, i]);
    let steps = ir.update_path(path).unwrap();
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[1].mode, StepMode::Await);
    assert_eq!(ir.update_path(UpdatePathId(1)), None);
    // Its depth is set as an await's: the block (0), the store (1), the
    // update (2); and the await of its record is below it (3).
    let IrKind::Update { depth, .. } = ir.get(update).kind else {
        panic!("expected an update");
    };
    assert_eq!(depth, 2);
    assert_eq!(ir.awaits_in(block), vec![(3, Span::new(0, 1))]);
    assert_eq!(
        ir.pretty(&names).to_string(),
        "(block @0..24\n\
             \x20 (store r#0 @0..24\n\
             \x20   (update .a[]!.\"c d\"+.e @0..14\n\
             \x20     (int 1 @23..24)\n\
             \x20     (await @0..1\n\
             \x20       (load r#0 @0..1))\n\
             \x20     (load i#1 @4..5)))\n\
             \x20 (empty @0..24))"
    );
}

#[test]
fn a_dotted_update_is_two_levels_taller_for_each_step() {
    // A function whose body is the update: its height is one above its
    // tallest part, and two for each step.
    let mut builder = IrBuilder::with_capacity(2);
    let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
    let parts = builder.push_item_list(&[one, one]);
    let path = builder.push_update_path(&[UpdateStep {
        at: StepAt::Element,
        mode: StepMode::Set,
        span: Span::new(0, 3),
    }]);
    let update = builder.alloc(
        IrKind::Update {
            parts,
            path,
            kind: ValueKind::List,
            depth: 0,
        },
        Span::new(0, 3),
    );
    let code = FnCode {
        body: update,
        kind: FnKind::Function,
        frame_size: 0,
        param_count: 0,
        capture_count: 0,
        height: 0,
        name: None,
        display: Box::from("fn(): {}"),
        slot_names: Box::default(),
        attributes: Box::default(),
        defers: false,
        retry: 0,
        timeout: None,
        name_text: None,
    };
    let mut functions = vec![code];
    builder.measure(update, &mut functions);
    assert_eq!(functions[0].height, 4);
}
