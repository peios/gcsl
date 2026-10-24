use super::*;

fn leaf(builder: &mut AstBuilder, value: i64, at: u32) -> ExprId {
    builder.alloc(Expr {
        kind: ExprKind::Int(value),
        span: Span::new(at, at + 1),
    })
}

/// An unlabelled block of `items`.
const fn unlabelled(items: ItemRange) -> ExprKind {
    ExprKind::Block { items, label: None }
}

/// Finishes `builder` with a body of one expression item, `root`.
fn finish_expr(mut builder: AstBuilder, root: ExprId) -> Ast {
    let span = builder.get(root).span;
    let item = builder.alloc_item(Item {
        kind: ItemKind::Expr(root),
        span,
    });
    let body = builder.push_item_list(&[item]);
    builder.finish(body, Names::new(), Strings::new())
}

#[test]
fn nodes_are_small() {
    // Guards the arena's density; grow this deliberately, not by accident.
    assert!(size_of::<Expr>() <= 24);
}

#[test]
fn an_item_is_small() {
    // What tells a named function item and a `def` from a `let` or a
    // field whose value is a function expression fits in the padding.
    assert_eq!(size_of::<Item>(), 36);
}

#[test]
fn a_parameter_is_small() {
    // Its name, span and type, 16 bytes, its optional default, 8, and its
    // optional default function, 8.
    assert_eq!(size_of::<Param>(), 32);
}

#[test]
fn a_field_type_is_small() {
    // Its optional name, span and type, 20 bytes, and its optional
    // default, 8: a tag and an expression or a constant's index.
    assert_eq!(size_of::<FieldTy>(), 28);
}

#[test]
fn a_blocks_label_fits_beside_its_items() {
    // The label's id has a niche, so that a block's optional label
    // costs four bytes, and an unlabelled block nothing beyond them.
    assert_eq!(size_of::<Option<LabelId>>(), 4);
    assert_eq!(size_of::<ExprKind>(), 16);
    let mut builder = AstBuilder::new();
    let a = Label {
        name: Names::new().intern("a"),
        span: Span::new(1, 3),
    };
    let first = builder.push_label(a);
    let second = builder.push_label(Label {
        span: Span::new(5, 7),
        ..a
    });
    assert_ne!(first, second);
    let empty = builder.push_item_list(&[]);
    let block = builder.alloc(Expr {
        kind: ExprKind::Block {
            items: empty,
            label: Some(second),
        },
        span: Span::new(4, 9),
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Expr(block),
        span: Span::new(4, 9),
    });
    let body = builder.push_item_list(&[item]);
    let ast = builder.finish(body, Names::new(), Strings::new());
    assert_eq!(ast.label(first).span, Span::new(1, 3));
    assert_eq!(
        ast.block_label(block).map(|label| label.span),
        Some(Span::new(5, 7))
    );
}

#[test]
fn alloc_returns_sequential_ids() {
    let mut builder = AstBuilder::new();
    let a = leaf(&mut builder, 1, 0);
    let b = leaf(&mut builder, 2, 4);
    assert_eq!((a.index(), b.index()), (0, 1));
    assert_eq!(builder.get(b).kind, ExprKind::Int(2));
    let ast = finish_expr(builder, b);
    assert_eq!(ast.items(ast.root()).len(), 1);
    assert_eq!(ast.len(), 2);
    assert_eq!(ast.item_count(), 1);
}

#[test]
fn item_lists_are_ranges_of_the_pool() {
    let mut builder = AstBuilder::new();
    let one = leaf(&mut builder, 1, 0);
    let two = leaf(&mut builder, 2, 2);
    let items: Vec<ItemId> = [one, two]
        .into_iter()
        .map(|value| {
            builder.alloc_item(Item {
                kind: ItemKind::Expr(value),
                span: builder.get(value).span,
            })
        })
        .collect();
    let inner = builder.push_item_list(&items[1..]);
    let outer = builder.push_item_list(&items);
    let empty = builder.push_item_list(&[]);
    let ast = builder.finish(outer, Names::new(), Strings::new());
    assert_eq!(ast.items(inner), &items[1..]);
    assert_eq!(ast.items(outer), &items[..]);
    assert!(ast.items(empty).is_empty());
    assert_eq!(ast.root_span(), Span::new(0, 3));
    let mut builder = AstBuilder::new();
    let empty = builder.push_item_list(&[]);
    let ast = builder.finish(empty, Names::new(), Strings::new());
    assert_eq!(ast.root_span(), Span::new(0, 0));
}

#[test]
fn pretty_prints_nested_s_expressions() {
    let mut builder = AstBuilder::new();
    let one = leaf(&mut builder, 1, 1);
    let two = builder.alloc(Expr {
        kind: ExprKind::Float(2.0),
        span: Span::new(5, 8),
    });
    let sum = builder.alloc(Expr {
        kind: ExprKind::Binary {
            op: BinaryOp::Add,
            op_start: 3,
            lhs: one,
            rhs: two,
        },
        span: Span::new(1, 8),
    });
    assert_eq!(builder.get(sum).operator_span(), Some(Span::new(3, 4)));
    let paren = builder.alloc(Expr {
        kind: ExprKind::Paren(sum),
        span: Span::new(0, 9),
    });
    let neg = builder.alloc(Expr {
        kind: ExprKind::Unary {
            op: UnaryOp::Neg,
            operand: paren,
        },
        span: Span::new(0, 9),
    });
    assert_eq!(builder.get(neg).operator_span(), Some(Span::new(0, 1)));
    assert_eq!(builder.get(paren).operator_span(), None);
    let ast = finish_expr(builder, neg);
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n  (expr @0..9\n    (neg @0..9\n      (paren @0..9\n        (+ @1..8\n          (int 1 @1..2)\n          (float 2.0 @5..8))))))"
    );
}

#[test]
fn pretty_prints_items_blocks_names_and_types() {
    // let x: Float = { y := z : Int; = 1 }
    // (The text is only for orientation; spans are made up.)
    let mut names = Names::new();
    let (x, y, z, int, float) = (
        names.intern("x"),
        names.intern("y"),
        names.intern("z"),
        names.intern("Int"),
        names.intern("Float"),
    );
    let mut builder = AstBuilder::new();
    let z_use = builder.alloc(Expr {
        kind: ExprKind::Name(z),
        span: Span::new(23, 24),
    });
    let int_ty = builder.alloc_type(TypeExpr {
        kind: TypeExprKind::Named(int),
        span: Span::new(27, 30),
    });
    let ascribed = builder.alloc(Expr {
        kind: ExprKind::Ascribe {
            expr: z_use,
            ty: int_ty,
        },
        span: Span::new(23, 30),
    });
    let y_path = builder.push_path(&[PathSegment {
        kind: SegmentKind::Name(y),
        span: Span::new(18, 19),
    }]);
    let rebind = builder.alloc_item(Item {
        kind: ItemKind::Rebind {
            path: y_path,
            value: ascribed,
        },
        span: Span::new(18, 30),
    });
    let one = leaf(&mut builder, 1, 34);
    let result = builder.alloc_item(Item {
        kind: ItemKind::Result(one),
        span: Span::new(32, 35),
    });
    let body = builder.push_item_list(&[rebind, result]);
    let block = builder.alloc(Expr {
        kind: unlabelled(body),
        span: Span::new(16, 37),
    });
    let float_ty = builder.alloc_type(TypeExpr {
        kind: TypeExprKind::Named(float),
        span: Span::new(7, 12),
    });
    let x_path = builder.push_path(&[PathSegment {
        kind: SegmentKind::Name(x),
        span: Span::new(4, 5),
    }]);
    let with_type = builder.alloc_item(Item {
        kind: ItemKind::Let {
            path: x_path,
            ty: Some(float_ty),
            value: block,
            mutable: false,
            function: false,
        },
        span: Span::new(0, 37),
    });
    let empty_body = builder.push_item_list(&[]);
    let empty = builder.alloc(Expr {
        kind: unlabelled(empty_body),
        span: Span::new(46, 48),
    });
    let y_path = builder.push_path(&[PathSegment {
        kind: SegmentKind::Name(y),
        span: Span::new(42, 43),
    }]);
    let without_type = builder.alloc_item(Item {
        kind: ItemKind::Let {
            path: y_path,
            ty: None,
            value: empty,
            mutable: false,
            function: false,
        },
        span: Span::new(38, 48),
    });
    let root = builder.push_item_list(&[with_type, without_type]);
    let ast = builder.finish(root, names, Strings::new());
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Float @0..37\n\
             \x20   (block @16..37\n\
             \x20     (rebind y @18..30\n\
             \x20       (: Int @23..30\n\
             \x20         (name z @23..24)))\n\
             \x20     (result @32..35\n\
             \x20       (int 1 @34..35))))\n\
             \x20 (let y @38..48\n\
             \x20   (block @46..48)))"
    );
}

#[test]
fn empty_file_prints_without_items() {
    let mut builder = AstBuilder::new();
    let root = builder.push_item_list(&[]);
    let ast = builder.finish(root, Names::new(), Strings::new());
    assert_eq!(ast.pretty().to_string(), "(file)");
}

#[test]
fn verbs_and_symbols() {
    assert_eq!(BinaryOp::Add.verb(), "add");
    assert_eq!(BinaryOp::Sub.verb(), "subtract");
    assert_eq!(BinaryOp::Mul.verb(), "multiply");
    assert_eq!(BinaryOp::Div.verb(), "divide");
    assert_eq!(BinaryOp::FloorDiv.verb(), "floor-divide");
    assert_eq!(BinaryOp::Rem.verb(), "take modulo of");
    assert_eq!(UnaryOp::Neg.verb(), "negate");
    assert_eq!(BinaryOp::FloorDiv.symbol(), "//");
    assert_eq!(BinaryOp::Rem.symbol(), "%");
    assert_eq!(UnaryOp::Neg.symbol(), "-");
    assert_eq!(BinaryOp::FloorDiv.span_at(4), Span::new(4, 6));
    assert_eq!(UnaryOp::Neg.span_at(4), Span::new(4, 5));
    for (op, symbol, equality) in [
        (BinaryOp::Eq, "==", true),
        (BinaryOp::Ne, "!=", true),
        (BinaryOp::Same, "===", true),
        (BinaryOp::NotSame, "!==", true),
        (BinaryOp::Lt, "<", false),
        (BinaryOp::Le, "<=", false),
        (BinaryOp::Gt, ">", false),
        (BinaryOp::Ge, ">=", false),
    ] {
        assert_eq!(op.symbol(), symbol);
        assert_eq!(op.verb(), "compare");
        assert!(op.is_comparison());
        assert_eq!(op.is_equality(), equality, "{op:?}");
    }
    assert_eq!(BinaryOp::Same.span_at(4), Span::new(4, 7));
    assert_eq!(BinaryOp::And.verb(), "and");
    assert_eq!(BinaryOp::Or.verb(), "or");
    assert_eq!(UnaryOp::Not.verb(), "not");
    assert_eq!(BinaryOp::And.symbol(), "and");
    assert_eq!(BinaryOp::Or.symbol(), "or");
    assert_eq!(UnaryOp::Not.symbol(), "not");
    assert_eq!(BinaryOp::Or.span_at(4), Span::new(4, 6));
    assert_eq!(UnaryOp::Not.span_at(4), Span::new(4, 7));
    assert_eq!(UnaryOp::Await.verb(), "await");
    assert_eq!(UnaryOp::Await.symbol(), "await");
    assert_eq!(UnaryOp::Await.span_at(4), Span::new(4, 9));
    for op in [
        BinaryOp::Add,
        BinaryOp::Sub,
        BinaryOp::Mul,
        BinaryOp::Div,
        BinaryOp::FloorDiv,
        BinaryOp::Rem,
        BinaryOp::And,
        BinaryOp::Or,
    ] {
        assert!(!op.is_comparison(), "{op:?}");
        assert!(!op.is_equality(), "{op:?}");
    }
}

#[test]
fn paths_are_ranges_of_the_pool_and_show_as_written() {
    // let a.b."c d".let = 1, spans made up.
    let mut names = Names::new();
    let (a, b, spaced, keyword) = (
        names.intern("a"),
        names.intern("b"),
        names.intern("c d"),
        names.intern("let"),
    );
    let mut builder = AstBuilder::new();
    let segment = |name, start, end| PathSegment {
        kind: SegmentKind::Name(name),
        span: Span::new(start, end),
    };
    let segments = [
        segment(a, 4, 5),
        segment(b, 6, 7),
        segment(spaced, 8, 13),
        segment(keyword, 14, 19),
    ];
    let single = builder.push_path(&segments[..1]);
    let dotted = builder.push_path(&segments);
    let one = leaf(&mut builder, 1, 22);
    let items = [
        ItemKind::Let {
            path: dotted,
            ty: None,
            value: one,
            mutable: false,
            function: false,
        },
        ItemKind::Field {
            path: dotted,
            ty: None,
            value: one,
            function: false,
        },
        ItemKind::Rebind {
            path: single,
            value: one,
        },
    ]
    .map(|kind| {
        builder.alloc_item(Item {
            kind,
            span: Span::new(0, 23),
        })
    });
    let root = builder.push_item_list(&items);
    let ast = builder.finish(root, names, Strings::new());
    assert_eq!(ast.path(single), &segments[..1]);
    assert_eq!(ast.path(dotted), &segments);
    assert_eq!(
        ast.show_path(ast.path(dotted)).to_string(),
        "a.b.\"c d\".\"let\""
    );
    assert_eq!(ast.show_path(&segments[..2]).to_string(), "a.b");
    assert_eq!(path_span(ast.path(dotted)), Span::new(4, 19));
    assert_eq!(path_span(&segments[1..3]), Span::new(6, 13));
    assert_eq!(path_span(&[]), Span::new(0, 0));
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let a.b.\"c d\".\"let\" @0..23\n\
             \x20   (int 1 @22..23))\n\
             \x20 (field a.b.\"c d\".\"let\" @0..23\n\
             \x20   (int 1 @22..23))\n\
             \x20 (rebind a @0..23\n\
             \x20   (int 1 @22..23)))"
    );
}

#[test]
fn nodes_with_strings_and_ifs_print_and_resolve() {
    // if c { "a{x}" } else { "b" }, spans made up.
    let mut names = Names::new();
    let (c, x) = (names.intern("c"), names.intern("x"));
    let mut strings = Strings::new();
    let (a, empty, b) = (strings.push("a\n"), strings.push(""), strings.push("\"b\""));
    let mut builder = AstBuilder::new();
    let cond = builder.alloc(Expr {
        kind: ExprKind::Name(c),
        span: Span::new(3, 4),
    });
    let x_use = builder.alloc(Expr {
        kind: ExprKind::Name(x),
        span: Span::new(10, 11),
    });
    let parts = builder.push_parts(&[StrPart::Lit(a), StrPart::Expr(x_use), StrPart::Lit(empty)]);
    let interp = builder.alloc(Expr {
        kind: ExprKind::Interp(parts),
        span: Span::new(7, 13),
    });
    let then_item = builder.alloc_item(Item {
        kind: ItemKind::Expr(interp),
        span: Span::new(7, 13),
    });
    let then_body = builder.push_item_list(&[then_item]);
    let then = builder.alloc(Expr {
        kind: unlabelled(then_body),
        span: Span::new(5, 15),
    });
    let lit = builder.alloc(Expr {
        kind: ExprKind::Str(b),
        span: Span::new(23, 26),
    });
    let else_item = builder.alloc_item(Item {
        kind: ItemKind::Expr(lit),
        span: Span::new(23, 26),
    });
    let else_body = builder.push_item_list(&[else_item]);
    let else_ = builder.alloc(Expr {
        kind: unlabelled(else_body),
        span: Span::new(21, 28),
    });
    let if_ = builder.alloc(Expr {
        kind: ExprKind::If {
            cond,
            then,
            else_,
            has_else: true,
        },
        span: Span::new(0, 28),
    });
    assert_eq!(builder.get(if_).keyword_span(), Some(Span::new(0, 2)));
    assert_eq!(builder.get(if_).operator_span(), None);
    assert_eq!(builder.get(cond).keyword_span(), None);
    let truth = builder.alloc(Expr {
        kind: ExprKind::Bool(true),
        span: Span::new(33, 37),
    });
    let not = builder.alloc(Expr {
        kind: ExprKind::Unary {
            op: UnaryOp::Not,
            operand: truth,
        },
        span: Span::new(29, 37),
    });
    let items = [if_, not].map(|value| {
        builder.alloc_item(Item {
            kind: ItemKind::Expr(value),
            span: builder.get(value).span,
        })
    });
    let root = builder.push_item_list(&items);
    let ast = builder.finish(root, names, strings);
    assert_eq!(
        ast.parts(parts),
        &[StrPart::Lit(a), StrPart::Expr(x_use), StrPart::Lit(empty)]
    );
    assert_eq!(ast.strings().get(b), "\"b\"");
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..28\n\
             \x20   (if @0..28\n\
             \x20     (name c @3..4)\n\
             \x20     (block @5..15\n\
             \x20       (expr @7..13\n\
             \x20         (interp @7..13\n\
             \x20           (lit \"a\\n\")\n\
             \x20           (name x @10..11)\n\
             \x20           (lit \"\"))))\n\
             \x20     (block @21..28\n\
             \x20       (expr @23..26\n\
             \x20         (str \"\\\"b\\\"\" @23..26)))))\n\
             \x20 (expr @29..37\n\
             \x20   (not @29..37\n\
             \x20     (bool true @33..37))))"
    );
}

#[test]
fn lists_and_indexes_are_subtrees_of_the_pool() {
    // `[1, ...xs][0]`, built children first as the parser does.
    let mut names = Names::new();
    let xs = names.intern("xs");
    let mut builder = AstBuilder::new();
    let one = leaf(&mut builder, 1, 1);
    let xs_use = builder.alloc(Expr {
        kind: ExprKind::Name(xs),
        span: Span::new(7, 9),
    });
    let elements = builder.push_elements(&[
        Element {
            value: one,
            spread: None,
        },
        Element {
            value: xs_use,
            spread: Some(Span::new(4, 7)),
        },
    ]);
    let list = builder.alloc(Expr {
        kind: ExprKind::List(elements),
        span: Span::new(0, 10),
    });
    let zero = leaf(&mut builder, 0, 11);
    let index = builder.alloc(Expr {
        kind: ExprKind::Index {
            expr: list,
            index: zero,
        },
        span: Span::new(0, 13),
    });
    let empty_elements = builder.push_elements(&[]);
    let empty = builder.alloc(Expr {
        kind: ExprKind::List(empty_elements),
        span: Span::new(14, 16),
    });
    let items: Vec<ItemId> = [index, empty]
        .into_iter()
        .map(|value| {
            builder.alloc_item(Item {
                kind: ItemKind::Expr(value),
                span: builder.get(value).span,
            })
        })
        .collect();
    let root = builder.push_item_list(&items);
    let ast = builder.finish(root, names, Strings::new());
    assert_eq!(ast.elements(elements).len(), 2);
    assert!(ast.elements(empty_elements).is_empty());
    assert_eq!(ast.elements(elements)[1].span(&ast), Span::new(4, 9));
    // A literal's subtree starts at its first element's, an index's at
    // its list's; an empty literal's is itself.
    assert_eq!(
        ast.subtree(index).collect::<Vec<_>>(),
        vec![one, xs_use, list, zero, index]
    );
    assert_eq!(
        ast.subtree(list).collect::<Vec<_>>(),
        vec![one, xs_use, list]
    );
    assert_eq!(ast.subtree(empty).collect::<Vec<_>>(), vec![empty]);
    assert_eq!(ast.get(index).operator_span(), None);
    assert_eq!(ast.get(index).bracket_span(&ast), Some(Span::new(10, 13)));
    assert_eq!(ast.get(list).bracket_span(&ast), None);
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..13\n\
             \x20   (index @0..13\n\
             \x20     (list @0..10\n\
             \x20       (int 1 @1..2)\n\
             \x20       (spread\n\
             \x20         (name xs @7..9)))\n\
             \x20     (int 0 @11..12)))\n\
             \x20 (expr @14..16\n\
             \x20   (list @14..16)))"
    );
}

#[test]
fn ranges_are_binary_operators() {
    for (op, symbol) in [(BinaryOp::Range, ".."), (BinaryOp::RangeInclusive, "..=")] {
        assert_eq!(op.symbol(), symbol);
        assert!(op.is_range());
        assert!(!op.is_comparison());
        assert!(!op.is_equality());
    }
    assert_eq!(BinaryOp::Range.span_at(4), Span::new(4, 6));
    assert_eq!(BinaryOp::RangeInclusive.span_at(4), Span::new(4, 7));
    assert!(!BinaryOp::Add.is_range());
    assert!(!BinaryOp::Lt.is_range());
}

#[test]
fn a_for_is_a_subtree_of_the_pool() {
    // `for x in 0..3 if g { x }`, built children first as the parser
    // does.
    let mut names = Names::new();
    let (x, g) = (names.intern("x"), names.intern("g"));
    let mut builder = AstBuilder::new();
    let zero = leaf(&mut builder, 0, 9);
    let three = leaf(&mut builder, 3, 12);
    let range = builder.alloc(Expr {
        kind: ExprKind::Binary {
            op: BinaryOp::Range,
            op_start: 10,
            lhs: zero,
            rhs: three,
        },
        span: Span::new(9, 13),
    });
    let guard = builder.alloc(Expr {
        kind: ExprKind::Name(g),
        span: Span::new(17, 18),
    });
    let x_use = builder.alloc(Expr {
        kind: ExprKind::Name(x),
        span: Span::new(21, 22),
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Expr(x_use),
        span: Span::new(21, 22),
    });
    let items = builder.push_item_list(&[item]);
    let body = builder.alloc(Expr {
        kind: unlabelled(items),
        span: Span::new(19, 24),
    });
    let id = builder.push_for(ForDecl {
        label: None,
        head: ForHead::Each {
            name: x,
            name_span: Span::new(4, 5),
            iterable: range,
            guard: Some(guard),
        },
        body,
    });
    let for_expr = builder.alloc(Expr {
        kind: ExprKind::For(id),
        span: Span::new(0, 24),
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Expr(for_expr),
        span: Span::new(0, 24),
    });
    let root = builder.push_item_list(&[item]);
    let ast = builder.finish(root, names, Strings::new());
    assert_eq!(ast.for_count(), 1);
    assert_eq!(id.index(), 0);
    assert_eq!(ast.for_decl(id).body, body);
    // A `for`'s subtree starts at its iterable's.
    assert_eq!(
        ast.subtree(for_expr).collect::<Vec<_>>(),
        vec![zero, three, range, guard, x_use, body, for_expr]
    );
    assert_eq!(ast.get(for_expr).operator_span(), None);
    assert_eq!(ast.get(for_expr).keyword_span(), Some(Span::new(0, 3)));
    assert_eq!(ast.get(range).operator_span(), Some(Span::new(10, 12)));
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..24\n\
             \x20   (for x @0..24\n\
             \x20     (.. @9..13\n\
             \x20       (int 0 @9..10)\n\
             \x20       (int 3 @12..13))\n\
             \x20     (guard\n\
             \x20       (name g @17..18))\n\
             \x20     (block @19..24\n\
             \x20       (expr @21..22\n\
             \x20         (name x @21..22))))))"
    );
}
