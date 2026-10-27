use super::*;
use crate::ast::{AstBuilder, Expr, Item};
use crate::check::check;
use crate::ir::{FnKind, LazyField, ListShape, ShapedOperands, StepAt, StepMode};
use crate::lexer::tokenize;
use crate::names::Names;
use crate::parser::parse;
use crate::source::SourceFile;
use crate::strings::Strings;

fn parse_text(text: &str) -> Ast {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    parse(&tokens, names, strings, &source).unwrap()
}

fn lower_text(text: &str) -> Ir {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    lower(&ast, &checked, &types, 0).unwrap()
}

fn dump(text: &str) -> String {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let ir = lower(&ast, &checked, &types, 0).unwrap();
    ir.pretty(ast.names()).to_string()
}

/// As [`lower_text`], but checking `text` as a lone file is, beside the
/// standard library, so that the prelude's names, `len` among them, are
/// its: what the tests of `len` lower with. Its slots come after the
/// standard library's.
fn lower_prelude(text: &str) -> (Ast, Ir) {
    let (ast, types, output) = crate::workspace::check_beside_prelude(text);
    let checked = output.unwrap().checked;
    let ir = lower(&ast, &checked, &types, 0).unwrap();
    (ast, ir)
}

/// As [`dump`], with [`lower_prelude`].
fn dump_prelude(text: &str) -> String {
    let (ast, ir) = lower_prelude(text);
    ir.pretty(ast.names()).to_string()
}

/// The node a file of one expression lowers to: its root block's result.
fn result_of(ir: &Ir) -> IrId {
    let IrKind::Block { items, result } = ir.get(ir.root()).kind else {
        panic!("the root is not a block: {:?}", ir.get(ir.root()));
    };
    assert!(ir.items(items).is_empty());
    result
}

/// `7 / 2` built by hand with both literals given the types in `types`,
/// e.g. both concrete `Int`, the shape variables produce.
fn concrete_int_division(types: Vec<TyId>) -> (Ast, Checked) {
    let mut builder = AstBuilder::new();
    let lhs = builder.alloc(Expr {
        kind: ExprKind::Int(7),
        span: Span::new(0, 1),
    });
    let rhs = builder.alloc(Expr {
        kind: ExprKind::Int(2),
        span: Span::new(4, 5),
    });
    let div = builder.alloc(Expr {
        kind: ExprKind::Binary {
            op: BinaryOp::Div,
            op_start: 2,
            lhs,
            rhs,
        },
        span: Span::new(0, 5),
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Expr(div),
        span: Span::new(0, 5),
    });
    let root = builder.push_item_list(&[item]);
    let ast = builder.finish(root, Names::new(), Strings::new());
    let checked = Checked {
        types: Types::new(types),
        resolution: Resolution::without_variables(3, 1),
    };
    (ast, checked)
}

#[test]
fn untyped_division_operands_become_float_constants() {
    let ir = lower_text("7 / 2");
    let root = ir.get(result_of(&ir));
    let IrKind::FloatDiv(operands) = root.kind else {
        panic!("expected FloatDiv, got {root:?}");
    };
    assert_eq!(ir.get(operands.lhs).kind, IrKind::FloatConst(7.0));
    assert_eq!(ir.get(operands.rhs).kind, IrKind::FloatConst(2.0));
    assert_eq!(operands.op_start, 2);
    assert_eq!(
        dump("7 / 2"),
        "(block @0..5\n  (float-div @0..5\n    (float 7.0 @0..1)\n    (float 2.0 @4..5)))"
    );
}

#[test]
fn concrete_int_division_operands_are_converted() {
    let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::FLOAT]);
    let ir = lower(&ast, &checked, &TypeTable::new(), 0).unwrap();
    assert_eq!(
        ir.pretty(ast.names()).to_string(),
        "(block @0..5\n  (float-div @0..5\n    (int-to-float @0..1\n      (int 7 @0..1))\n    (int-to-float @4..5\n      (int 2 @4..5))))"
    );
}

#[test]
fn concrete_int_from_floor_division_is_converted() {
    assert_eq!(
        dump("(7 // 2) / 2"),
        "(block @0..12\n  (float-div @0..12\n    (int-to-float @0..8\n      (int-floor-div @1..7\n        (int 7 @1..2)\n        (int 2 @6..7)))\n    (float 2.0 @11..12)))"
    );
}

#[test]
fn parentheses_and_ascriptions_are_dropped() {
    // The root block covers the file's items, parentheses included.
    assert_eq!(dump("((1))"), "(block @0..5\n  (int 1 @2..3))");
    assert_eq!(dump("(1 : Float)"), "(block @0..11\n  (float 1.0 @1..2))");
}

#[test]
fn operators_are_specialised_by_type() {
    let ir = lower_text("-(1 + 2 - 3 * 4 // 5 % 6)");
    let result = result_of(&ir);
    assert_eq!(ir.kind_of(result), ValueKind::Int);
    assert_eq!(
        dump("-(1 + 2 - 3 * 4 // 5 % 6)"),
        "(block @0..25\n  (int-neg @0..25\n    (int-sub @2..24\n      (int-add @2..7\n        (int 1 @2..3)\n        (int 2 @6..7))\n      (int-rem @10..24\n        (int-floor-div @10..20\n          (int-mul @10..15\n            (int 3 @10..11)\n            (int 4 @14..15))\n          (int 5 @19..20))\n        (int 6 @23..24)))))"
    );
    assert_eq!(
        dump("-(1.0 + 2 - 3 * 4.0)"),
        "(block @0..20\n  (float-neg @0..20\n    (float-sub @2..19\n      (float-add @2..9\n        (float 1.0 @2..5)\n        (float 2.0 @8..9))\n      (float-mul @12..19\n        (float 3.0 @12..13)\n        (float 4.0 @16..19)))))"
    );
}

#[test]
fn blocks_variables_and_statements() {
    let text = "let x = 1\n\
                    {\n  let y: Float = 2\n  y := y * 2.0\n}\n\
                    x := x + 1\n\
                    = { x }";
    // Slots are named by the last variable assigned to them: slot 1
    // holds `y` inside the block and then the rebound `x` after it, so
    // it shows as `x#1` throughout.
    assert_eq!(
        dump(text),
        "(block @0..66\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (block @10..47\n\
             \x20   (store x#1 @14..30\n\
             \x20     (float 2.0 @29..30))\n\
             \x20   (store y#2 @33..45\n\
             \x20     (float-mul @38..45\n\
             \x20       (load x#1 @38..39)\n\
             \x20       (float 2.0 @42..45)))\n\
             \x20   (empty @10..47))\n\
             \x20 (store x#1 @48..58\n\
             \x20   (int-add @53..58\n\
             \x20     (load x#0 @53..54)\n\
             \x20     (int 1 @57..58)))\n\
             \x20 (block @61..66\n\
             \x20   (load x#1 @63..64)))"
    );
}

#[test]
fn empty_file_is_an_empty_block_with_an_empty_result() {
    assert_eq!(dump(""), "(block @0..0\n  (empty @0..0))");
    assert_eq!(dump("# nothing\n"), "(block @0..0\n  (empty @0..0))");
    let ir = lower_text("");
    assert_eq!(ir.kind_of(ir.root()), ValueKind::Record);
    assert_eq!(ir.frame_size(), 0);
    assert_eq!(ir.layout_count(), 0);
}

#[test]
fn frame_size_comes_from_the_resolution() {
    assert_eq!(
        lower_text("let a = 1\n{ let b = 2; let c = 3 }").frame_size(),
        3
    );
}

#[test]
fn large_untyped_literal_rounds_to_nearest_float() {
    let ir = lower_text("9007199254740993 / 1");
    let IrKind::FloatDiv(operands) = ir.get(result_of(&ir)).kind else {
        panic!("expected FloatDiv");
    };
    assert_eq!(
        ir.get(operands.lhs).kind,
        IrKind::FloatConst(9_007_199_254_740_992.0)
    );
}

#[test]
fn untyped_entry_is_an_internal_error() {
    let (ast, checked) = concrete_int_division(vec![TyId::UNTYPED_INT, TyId::INT, TyId::FLOAT]);
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(error.message, "an untyped integer survived type checking");
    assert_eq!(error.span, Span::new(0, 1));
}

#[test]
fn error_entry_is_an_internal_error() {
    let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::ERROR, TyId::FLOAT]);
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(error.message, "a type error survived type checking");
    assert_eq!(error.span, Span::new(4, 5));
}

#[test]
fn a_type_from_another_table_is_an_internal_error() {
    let ast = parse_text("{ x = 1 }");
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(
        error.message,
        "a type with no values survived type checking"
    );
}

#[test]
fn impossible_result_type_is_an_internal_error() {
    let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::INT]);
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(error.message, "operator has a result type it cannot have");
    assert_eq!(error.span, Span::new(0, 5));
}

#[test]
fn mismatched_operand_type_is_an_internal_error() {
    let ast = parse_text("1 + 2");
    let checked = Checked {
        types: Types::new(vec![TyId::INT, TyId::FLOAT, TyId::INT]),
        resolution: Resolution::without_variables(3, 1),
    };
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(error.message, "operand type does not match its operator");
    assert_eq!(error.span, Span::new(4, 5));
}

#[test]
fn unresolved_variable_is_an_internal_error() {
    let ast = parse_text("let x = 1\nx");
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let unresolved = Checked {
        types: checked.types.clone(),
        resolution: Resolution::without_variables(2, 2),
    };
    let error = lower(&ast, &unresolved, &types, 0).unwrap_err();
    assert_eq!(error.message, "a declaration has no slot");
    assert_eq!(error.span, Span::new(0, 9));
}

#[test]
fn interpolations_list_pieces_and_expressions() {
    assert_eq!(
        dump("let n = 1\n\"n = {n}, {n + 1}.\""),
        "(block @0..29\n\
             \x20 (store n#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (interp @10..29\n\
             \x20   (str \"n = \" @10..29)\n\
             \x20   (load n#0 @16..17)\n\
             \x20   (str \", \" @10..29)\n\
             \x20   (int-add @21..26\n\
             \x20     (load n#0 @21..22)\n\
             \x20     (int 1 @25..26))\n\
             \x20   (str \".\" @10..29)))"
    );
}

#[test]
fn comparisons_are_specialised_by_operand_type() {
    assert_eq!(
        dump("1 < 2.5"),
        "(block @0..7\n  (float-cmp < @0..7\n    (float 1.0 @0..1)\n    (float 2.5 @4..7)))"
    );
    assert_eq!(
        dump("1 >= 2"),
        "(block @0..6\n  (int-cmp >= @0..6\n    (int 1 @0..1)\n    (int 2 @5..6)))"
    );
    assert_eq!(
        dump("\"a\" != \"b\""),
        "(block @0..10\n  (str-cmp != @0..10\n    (str \"a\" @0..3)\n    (str \"b\" @7..10)))"
    );
    assert_eq!(
        dump("true != false"),
        "(block @0..13\n  (bool-eq != @0..13\n    (bool true @0..4)\n    (bool false @8..13)))"
    );
    assert_eq!(
        dump("{} == {}"),
        "(block @0..8\n  (record-eq == [] @0..8\n    (block @0..2\n      (empty @0..2))\n    (block @6..8\n      (empty @6..8))))"
    );
}

#[test]
fn if_keeps_both_branches() {
    assert_eq!(
        dump("if true { 1 } else { 2 }"),
        "(block @0..24\n\
             \x20 (if @0..24\n\
             \x20   (bool true @3..7)\n\
             \x20   (block @8..13\n\
             \x20     (int 1 @10..11))\n\
             \x20   (block @19..24\n\
             \x20     (int 2 @21..22))))"
    );
    // The implicit else branch is an empty block yielding `{}`.
    assert_eq!(
        dump("if true {}"),
        "(block @0..10\n\
             \x20 (if @0..10\n\
             \x20   (bool true @3..7)\n\
             \x20   (block @8..10\n\
             \x20     (empty @8..10))\n\
             \x20   (block @10..10\n\
             \x20     (empty @10..10))))"
    );
    let ir = lower_text("if true { 1 } else { 2.5 }");
    assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Float);
}

#[test]
fn logic_and_concatenation() {
    assert_eq!(
        dump("true and false"),
        "(block @0..14\n  (and @0..14\n    (bool true @0..4)\n    (bool false @9..14)))"
    );
    assert_eq!(
        dump("not true or false"),
        "(block @0..17\n  (or @0..17\n    (not @0..8\n      (bool true @4..8))\n    (bool false @12..17)))"
    );
    assert_eq!(
        dump("\"a\" + r\"b\""),
        "(block @0..10\n  (str-concat @0..10\n    (str \"a\" @0..3)\n    (str \"b\" @6..10)))"
    );
}

#[test]
fn string_constants_are_resolved_into_the_irs_table() {
    let ir = lower_text("\"a{1}b\" + \"c\\n\"");
    let IrKind::StrConcat(operands) = ir.get(result_of(&ir)).kind else {
        panic!("expected a concatenation");
    };
    let IrKind::Interp(items) = ir.get(operands.lhs).kind else {
        panic!("expected an interpolation");
    };
    let text = |id: IrId| match ir.get(id).kind {
        IrKind::StrConst(constant) => ir.string(constant).to_string(),
        other => format!("{other:?}"),
    };
    let items: Vec<String> = ir.items(items).iter().map(|&id| text(id)).collect();
    assert_eq!(items, vec!["a", "IntConst(1)", "b"]);
    assert_eq!(text(operands.rhs), "c\n");
}

#[test]
fn tables_of_the_wrong_size_are_an_internal_error() {
    let (ast, _) = concrete_int_division(Vec::new());
    let checked = Checked {
        types: Types::new(vec![TyId::INT]),
        resolution: Resolution::without_variables(3, 1),
    };
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(error.message, "type table does not belong to this AST");
    let checked = Checked {
        types: Types::new(vec![TyId::INT, TyId::INT, TyId::FLOAT]),
        resolution: Resolution::without_variables(3, 2),
    };
    let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
    assert_eq!(error.message, "resolution does not belong to this AST");
}

#[test]
fn a_record_block_stores_its_fields_and_makes_a_record_of_them() {
    // Private `let`s and statements are items too; the record is the
    // fields' slots, loaded in definition order.
    assert_eq!(
        dump("{ x = 1; let t = x; {}; \"a b\" = t }"),
        "(block @0..35\n\
             \x20 (block @0..35\n\
             \x20   (store x#0 @2..7\n\
             \x20     (int 1 @6..7))\n\
             \x20   (store t#1 @9..18\n\
             \x20     (load x#0 @17..18))\n\
             \x20   (block @20..22\n\
             \x20     (empty @20..22))\n\
             \x20   (store \"a b\"#2 @24..33\n\
             \x20     (load t#1 @32..33))\n\
             \x20   (make-record [x \"a b\"] @0..35\n\
             \x20     (load x#0 @2..3)\n\
             \x20     (load \"a b\"#2 @24..29))))"
    );
    // A file can be a record block too.
    assert_eq!(
        dump("y = 2.5\nx = true"),
        "(block @0..16\n\
             \x20 (store y#0 @0..7\n\
             \x20   (float 2.5 @4..7))\n\
             \x20 (store x#1 @8..16\n\
             \x20   (bool true @12..16))\n\
             \x20 (make-record [y x] @0..16\n\
             \x20   (load y#0 @0..1)\n\
             \x20   (load x#1 @8..9)))"
    );
    let ir = lower_text("{ x = 1 }");
    assert_eq!(ir.kind_of(ir.root()), ValueKind::Record);
}

#[test]
fn field_access_and_nested_records() {
    // The inner block's `n` takes the slot that `a`, declared after its
    // initialiser, then reuses, so it shows as `a#0` too.
    assert_eq!(
        dump("{ a = { n = 1.5 } }.a.n"),
        "(block @0..23\n\
             \x20 (field n @0..23\n\
             \x20   (field a @0..21\n\
             \x20     (block @0..19\n\
             \x20       (store a#0 @2..17\n\
             \x20         (block @6..17\n\
             \x20           (store a#0 @8..15\n\
             \x20             (float 1.5 @12..15))\n\
             \x20           (make-record [n] @6..17\n\
             \x20             (load a#0 @8..9))))\n\
             \x20       (make-record [a] @0..19\n\
             \x20         (load a#0 @2..3))))))"
    );
    let ir = lower_text("{ a = { n = 1.5 } }.a.n");
    assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Float);
    // Record equality and a field of every kind.
    assert_eq!(
        dump("{ x = 1 } != { x = 2 }"),
        "(block @0..22\n\
             \x20 (record-eq != [x] @0..22\n\
             \x20   (block @0..9\n\
             \x20     (store x#0 @2..7\n\
             \x20       (int 1 @6..7))\n\
             \x20     (make-record [x] @0..9\n\
             \x20       (load x#0 @2..3)))\n\
             \x20   (block @13..22\n\
             \x20     (store x#0 @15..20\n\
             \x20       (int 2 @19..20))\n\
             \x20     (make-record [x] @13..22\n\
             \x20       (load x#0 @15..16)))))"
    );
}

#[test]
fn a_literal_checked_against_a_record_type_is_lowered_at_that_type() {
    assert_eq!(
        dump("let p: {x: Float} = { x = 1 }"),
        "(block @0..29\n\
             \x20 (store p#0 @0..29\n\
             \x20   (block @20..29\n\
             \x20     (store p#0 @22..27\n\
             \x20       (float 1.0 @26..27))\n\
             \x20     (make-record [x] @20..29\n\
             \x20       (load p#0 @22..23))))\n\
             \x20 (empty @0..29))"
    );
}

#[test]
fn layouts_are_shared_by_blocks_with_the_same_names() {
    let ir =
        lower_text("let a = { x = 1; y = 2 }\nlet b = { x = \"s\"; y = true }\n{ y = 1; x = 2 }");
    // `x y` twice, then `y x`, a different order.
    assert_eq!(ir.layout_count(), 2);
    let ir = lower_text("{ x = 1 } == { x = 2 }");
    assert_eq!(ir.layout_count(), 1);
    let IrKind::RecordEq(operands) = ir.get(result_of(&ir)).kind else {
        panic!("expected a record comparison");
    };
    let layout_of = |block: IrId| {
        let IrKind::Block { result, .. } = ir.get(block).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeRecord { layout, .. } = ir.get(result).kind else {
            panic!("expected a record");
        };
        layout
    };
    let (lhs, rhs) = (layout_of(operands.lhs), layout_of(operands.rhs));
    assert_eq!(lhs, rhs);
    assert_eq!(&*ir.layout(lhs).texts, &[Box::from("x")]);
}

#[test]
fn a_merged_field_updates_its_slot() {
    // Each leaf is stored in its cell at its own line; each interior node
    // is assembled from its children's cells after its last child, the
    // root last. The field is loaded once for the record, from its root's
    // cell.
    assert_eq!(
        dump("a.x = 1\na.y.z = 2\na.y.w = 3"),
        "(block @0..27\n\
             \x20 (store x#1 @0..7\n\
             \x20   (int 1 @6..7))\n\
             \x20 (store z#3 @8..17\n\
             \x20   (int 2 @16..17))\n\
             \x20 (store w#4 @18..27\n\
             \x20   (int 3 @26..27))\n\
             \x20 (store y#2 @8..17\n\
             \x20   (make-record [z w] @8..17\n\
             \x20     (load z#3 @8..17)\n\
             \x20     (load w#4 @18..27)))\n\
             \x20 (store a#0 @0..7\n\
             \x20   (make-record [x y] @0..7\n\
             \x20     (load x#1 @0..7)\n\
             \x20     (load y#2 @8..17)))\n\
             \x20 (make-record [a] @0..27\n\
             \x20   (load a#0 @0..1)))"
    );
    // The name of every update of an inner block's dotted field is
    // recorded for the evaluator.
    let ast = parse_text("r = { a.x = 1\na.\"y z\".w = 2 }");
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let ir = lower(&ast, &checked, &types, 0).unwrap();
    let text = |text: &str| ir.name_text(ast.names().lookup(text).unwrap());
    assert_eq!(text("x"), Some("x"));
    assert_eq!(text("y z"), Some("y z"));
    assert_eq!(text("w"), Some("w"));
    // Not the field's own name, which no update adds.
    assert_eq!(text("a"), None);
}

#[test]
fn a_merged_field_takes_its_place_where_first_mentioned() {
    assert_eq!(
        dump("{ a.x = 1; b = 2; a.y = 3 }"),
        "(block @0..27\n\
             \x20 (block @0..27\n\
             \x20   (store a#0 @2..9\n\
             \x20     (update-field x @2..5\n\
             \x20       (empty @2..3)\n\
             \x20       (int 1 @8..9)))\n\
             \x20   (store b#1 @11..16\n\
             \x20     (int 2 @15..16))\n\
             \x20   (store a#0 @18..25\n\
             \x20     (update-field y @18..21\n\
             \x20       (load a#0 @18..19)\n\
             \x20       (int 3 @24..25)))\n\
             \x20   (make-record [a b] @0..27\n\
             \x20     (load a#0 @2..3)\n\
             \x20     (load b#1 @11..12))))"
    );
}

#[test]
fn a_dotted_let_updates_the_old_record_into_a_new_slot() {
    assert_eq!(
        dump("let a = { b = { c = 1 } }\nlet a.b.c = 2"),
        "(block @0..39\n\
             \x20 (store a#0 @0..25\n\
             \x20   (block @8..25\n\
             \x20     (store a#0 @10..23\n\
             \x20       (block @14..23\n\
             \x20         (store a#0 @16..21\n\
             \x20           (int 1 @20..21))\n\
             \x20         (make-record [c] @14..23\n\
             \x20           (load a#0 @16..17))))\n\
             \x20     (make-record [b] @8..25\n\
             \x20       (load a#0 @10..11))))\n\
             \x20 (store a#1 @26..39\n\
             \x20   (update .b.c @30..35\n\
             \x20     (int 2 @38..39)\n\
             \x20     (load a#0 @30..31)))\n\
             \x20 (empty @0..39))"
    );
    // Where the static type lacks a segment, its record is created.
    assert_eq!(
        dump("let a = 1\nlet b.c.d = a\nb.c := { d = 2 }"),
        "(block @0..40\n\
             \x20 (store a#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store b#1 @10..23\n\
             \x20   (update .c+.d @14..19\n\
             \x20     (load a#0 @22..23)\n\
             \x20     (empty @14..15)))\n\
             \x20 (store b#2 @24..40\n\
             \x20   (update .c @24..27\n\
             \x20     (block @31..40\n\
             \x20       (store b#2 @33..38\n\
             \x20         (int 2 @37..38))\n\
             \x20       (make-record [d] @31..40\n\
             \x20         (load b#2 @33..34)))\n\
             \x20     (load b#1 @24..25)))\n\
             \x20 (empty @0..40))"
    );
}

#[test]
fn a_rebind_of_a_mutable_variable_stores_into_its_slot() {
    // From an inner block, plainly or along a path, the rebind is a
    // store into the variable's one slot, the dotted one of an update
    // of the record loaded from it.
    assert_eq!(
        dump("let mut x = 1\n{ x := 2 }\nlet mut r = { a = x }\nif true { r.a := x + 1 }\nr.a"),
        "(block @0..75\n\
             \x20 (store x#0 @0..13\n\
             \x20   (int 1 @12..13))\n\
             \x20 (block @14..24\n\
             \x20   (store x#0 @16..22\n\
             \x20     (int 2 @21..22))\n\
             \x20   (empty @14..24))\n\
             \x20 (store r#1 @25..46\n\
             \x20   (block @37..46\n\
             \x20     (store r#1 @39..44\n\
             \x20       (load x#0 @43..44))\n\
             \x20     (make-record [a] @37..46\n\
             \x20       (load r#1 @39..40))))\n\
             \x20 (if @47..71\n\
             \x20   (bool true @50..54)\n\
             \x20   (block @55..71\n\
             \x20     (store r#1 @57..69\n\
             \x20       (update .a @57..60\n\
             \x20         (int-add @64..69\n\
             \x20           (load x#0 @64..65)\n\
             \x20           (int 1 @68..69))\n\
             \x20         (load r#1 @57..58)))\n\
             \x20     (empty @55..71))\n\
             \x20   (block @71..71\n\
             \x20     (empty @71..71)))\n\
             \x20 (field a @72..75\n\
             \x20   (load r#1 @72..73)))"
    );
    // A frame does not grow with the rebinds.
    assert_eq!(
        lower_text("let mut x = 1\nx := 2\nx := 3\nx").frame_size(),
        1
    );
}

#[test]
fn typed_record_equality_compares_over_a_shape() {
    assert_eq!(
        dump("let p = { x = 1; inner = { n = 2 } }\np == p"),
        "(block @0..43\n\
             \x20 (store p#0 @0..36\n\
             \x20   (block @8..36\n\
             \x20     (store p#0 @10..15\n\
             \x20       (int 1 @14..15))\n\
             \x20     (store inner#1 @17..34\n\
             \x20       (block @25..34\n\
             \x20         (store inner#1 @27..32\n\
             \x20           (int 2 @31..32))\n\
             \x20         (make-record [n] @25..34\n\
             \x20           (load inner#1 @27..28))))\n\
             \x20     (make-record [x inner] @8..36\n\
             \x20       (load p#0 @10..11)\n\
             \x20       (load inner#1 @17..22))))\n\
             \x20 (record-eq == [x inner[n]] @37..43\n\
             \x20   (load p#0 @37..38)\n\
             \x20   (load p#0 @42..43)))"
    );
    // The shape is the static type's, not the value's.
    let text = "let q: {x: Int} = { x = 1; y = 2 }\nq != { x = 1 }";
    assert!(dump(text).contains("(record-eq != [x] @"));
    // One shape per record type, nested ones included, shared by every
    // comparison of it.
    let ir = lower_text(
        "let p = { x = 1; inner = { n = 2 } }\n\
             let a = p == p\n\
             let b = p.inner != p.inner\n\
             let c = { n = 3 } == p.inner\n\
             = { e = {} } == { e = {} }",
    );
    assert_eq!(ir.shape_count(), 4);
    // An empty record's shape has no fields.
    assert_eq!(
        dump("{ e = {} } == { e = {} }").lines().nth(1),
        Some("  (record-eq == [e[]] @0..24")
    );
}

#[test]
fn total_equality_lowers_like_typed_equality_but_on_records() {
    assert_eq!(
        dump("{ x = 1 } !== { x = 1 }"),
        "(block @0..23\n\
             \x20 (record-same !== @0..23\n\
             \x20   (block @0..9\n\
             \x20     (store x#0 @2..7\n\
             \x20       (int 1 @6..7))\n\
             \x20     (make-record [x] @0..9\n\
             \x20       (load x#0 @2..3)))\n\
             \x20   (block @14..23\n\
             \x20     (store x#0 @16..21\n\
             \x20       (int 1 @20..21))\n\
             \x20     (make-record [x] @14..23\n\
             \x20       (load x#0 @16..17)))))"
    );
    assert_eq!(
        dump("{} === {}"),
        "(block @0..9\n  (record-same === @0..9\n    (block @0..2\n      (empty @0..2))\n    (block @7..9\n      (empty @7..9))))"
    );
    // Scalars lower exactly as `==` and `!=` do.
    assert_eq!(
        dump("1 === 2"),
        "(block @0..7\n  (int-cmp == @0..7\n    (int 1 @0..1)\n    (int 2 @6..7)))"
    );
    assert_eq!(
        dump("1.5 !== 2"),
        "(block @0..9\n  (float-cmp != @0..9\n    (float 1.5 @0..3)\n    (float 2.0 @8..9)))"
    );
    assert_eq!(
        dump("\"a\" === \"b\""),
        "(block @0..11\n  (str-cmp == @0..11\n    (str \"a\" @0..3)\n    (str \"b\" @8..11)))"
    );
    assert_eq!(
        dump("true !== false"),
        "(block @0..14\n  (bool-eq != @0..14\n    (bool true @0..4)\n    (bool false @9..14)))"
    );
    assert_eq!(lower_text("{ x = 1 } === { x = 2 }").shape_count(), 0);
}

#[test]
fn a_closure_captures_in_the_creating_frame() {
    assert_eq!(
        dump("let x = 1\nlet f = fn(y: Int): Int { x + y }"),
        "(block @0..43\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store f#1 @10..43\n\
             \x20   (make-closure #0 @18..43\n\
             \x20     (load x#0 @18..43)))\n\
             \x20 (empty @0..43))\n\
             fn #0 f (frame 1, params 1, captures 1, height 3)\n\
             \x20 (block @34..43\n\
             \x20   (int-add @36..41\n\
             \x20     (load-capture 0 @36..37)\n\
             \x20     (load y#0 @40..41)))"
    );
}

#[test]
fn a_call_passes_its_arguments_in_source_order() {
    let text = "fn f(a: Int, b: String): Int { a }\nf(b = \"s\", a = 1)";
    assert_eq!(
        dump(text),
        "(block @0..52\n\
             \x20 (store f#0 @0..34\n\
             \x20   (make-closure #0 @0..34))\n\
             \x20 (call @35..52\n\
             \x20   (load f#0 @35..36)\n\
             \x20   (arg 1 @37..44\n\
             \x20     (str \"s\" @41..44))\n\
             \x20   (arg 0 @46..51\n\
             \x20     (int 1 @50..51))))\n\
             fn #0 f (frame 2, params 2, captures 0, height 2)\n\
             \x20 (block @29..34\n\
             \x20   (load a#0 @31..32))"
    );
    // The root's result, after the function's declaration.
    let call = |ir: &Ir| match ir.get(ir.root()).kind {
        IrKind::Block { result, .. } => result,
        other => panic!("the root is not a block: {other:?}"),
    };
    let ir = lower_text(text);
    assert_eq!(ir.kind_of(call(&ir)), ValueKind::Int);
    let IrKind::Call { args, kind, .. } = ir.get(call(&ir)).kind else {
        panic!("expected a call");
    };
    assert_eq!(kind, ValueKind::Int);
    let arg = ir.items(args)[0];
    assert_eq!(ir.kind_of(arg), ValueKind::String);
    // A trailing block is the last argument.
    let ir = lower_text("fn f(a: Int, r: {x: Int}): Int { a }\nf(1) { x = 2 }");
    let IrKind::Call { args, .. } = ir.get(call(&ir)).kind else {
        panic!("expected a call");
    };
    let params: Vec<u32> = ir
        .items(args)
        .iter()
        .map(|&arg| match ir.get(arg).kind {
            IrKind::Arg { param, .. } => param,
            other => panic!("expected an argument, got {other:?}"),
        })
        .collect();
    assert_eq!(params, vec![0, 1]);
}

#[test]
fn a_recursive_function_loads_its_callee() {
    assert_eq!(
        dump("fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }"),
        "(block @0..65\n\
             \x20 (store fact#0 @0..65\n\
             \x20   (make-closure #0 @0..65))\n\
             \x20 (empty @0..65))\n\
             fn #0 fact (frame 1, params 1, captures 0, height 8)\n\
             \x20 (block @21..65\n\
             \x20   (if @23..63\n\
             \x20     (int-cmp <= @26..32\n\
             \x20       (load n#0 @26..27)\n\
             \x20       (int 1 @31..32))\n\
             \x20     (block @33..38\n\
             \x20       (int 1 @35..36))\n\
             \x20     (block @44..63\n\
             \x20       (int-mul @46..61\n\
             \x20         (load n#0 @46..47)\n\
             \x20         (call @50..61\n\
             \x20           (load-callee @50..54)\n\
             \x20           (arg 0 @55..60\n\
             \x20             (int-sub @55..60\n\
             \x20               (load n#0 @55..56)\n\
             \x20               (int 1 @59..60))))))))"
    );
}

#[test]
fn nested_functions_chain_their_captures() {
    // The innermost, pure, function measures `a` as its result, which
    // makes `a` present for `outer`.
    let text = "fn outer(a: Int): fn(): fn(): Int { fn(): fn(): Int { fn(): Int { a } } }";
    assert_eq!(
        dump(text),
        "(block @0..73\n\
             \x20 (store outer#0 @0..73\n\
             \x20   (make-closure #2 @0..73))\n\
             \x20 (empty @0..73))\n\
             fn #0 (frame 0, params 0, captures 1, height 2)\n\
             \x20 (block @64..69\n\
             \x20   (load-capture 0 @66..67))\n\
             fn #1 (frame 0, params 0, captures 1, height 3)\n\
             \x20 (block @52..71\n\
             \x20   (make-closure #0 @54..69\n\
             \x20     (load-capture 0 @54..69)))\n\
             fn #2 outer (frame 1, params 1, captures 0, height 3)\n\
             \x20 (block @34..73\n\
             \x20   (make-closure #1 @36..71\n\
             \x20     (load a#0 @36..71)))"
    );
    // A captured callee is loaded as the callee in the creating frame.
    let ir = lower_text("fn f(): fn(): Int { fn(): Int { f()() } }");
    let outer = ir.function(FnId::new(1));
    let IrKind::Block { result, .. } = ir.get(outer.body).kind else {
        panic!("expected a block");
    };
    let IrKind::MakeClosure { function, captures } = ir.get(result).kind else {
        panic!("expected a closure");
    };
    assert_eq!(function, FnId::new(0));
    let captures: Vec<IrKind> = ir
        .items(captures)
        .iter()
        .map(|&id| ir.get(id).kind)
        .collect();
    assert_eq!(captures, vec![IrKind::LoadCallee]);
}

#[test]
fn a_memoised_function_is_code_of_its_own_kind() {
    let ir = lower_text(
        "fn f(x: Int) ->> Int { x }\n\
             fn g(x: Int) -> Int { x }\n\
             fn h(x: Int): Int { x }\n\
             let l = lazy f(1)",
    );
    let kinds: Vec<FnKind> = (0..4).map(|id| ir.function(FnId::new(id)).kind).collect();
    assert_eq!(
        kinds,
        vec![
            FnKind::Memo,
            FnKind::Function,
            FnKind::Function,
            FnKind::Future
        ]
    );
    assert_eq!(&*ir.function(FnId::new(0)).display, "fn(x: Int) ->> Int");
    // The IR's dump shows it.
    assert_eq!(
        dump("fn f(x: Int) ->> Int { x }"),
        "(block @0..26\n\
             \x20 (store f#0 @0..26\n\
             \x20   (make-closure #0 @0..26))\n\
             \x20 (empty @0..26))\n\
             memo fn #0 f (frame 1, params 1, captures 0, height 2)\n\
             \x20 (block @21..26\n\
             \x20   (load x#0 @23..24))"
    );
}

#[test]
fn the_function_table_describes_each_function() {
    let ast = parse_text(
        "let k = 2\n\
             fn scale(x: Int, by: {f: Float}) -> Float { let y = x * k\ny / by.f }\n\
             let anon = fn(): {} {}",
    );
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let ir = lower(&ast, &checked, &types, 7).unwrap();
    assert_eq!(ir.input(), 7);
    assert_eq!(ir.function_count(), 2);
    let scale = ir.function(FnId::new(0));
    assert_eq!(
        &*scale.display,
        "fn(x: Int | Future<Int>, by: {f: Float | Future<Float>} | Future<{f: Float | \
             Future<Float>}>) -> Float"
    );
    assert_eq!(scale.kind, FnKind::Function);
    assert_eq!(scale.frame_size, 3);
    assert_eq!(scale.param_count, 2);
    assert_eq!(scale.capture_count, 1);
    assert_eq!(scale.name, ast.names().lookup("scale"));
    let slot_names: Vec<&str> = scale
        .slot_names
        .iter()
        .map(|&name| ast.names().get(name))
        .collect();
    assert_eq!(slot_names, vec!["x", "by", "y"]);
    assert_eq!(ir.kind_of(scale.body), ValueKind::Float);
    let anon = ir.find_function(FnId::new(1)).unwrap();
    assert_eq!(&*anon.display, "fn(): {}");
    assert_eq!(anon.name, ast.names().lookup("anon"));
    assert_eq!(
        (anon.frame_size, anon.param_count, anon.capture_count),
        (0, 0, 0)
    );
    assert!(ir.find_function(FnId::new(2)).is_none());
    assert_eq!(lower_text("1").input(), 0);
    // A closure is a function.
    let ir = lower_text("fn f(): fn(): Int { fn(): Int { 1 } }");
    assert_eq!(ir.kind_of(ir.function(FnId::new(1)).body), ValueKind::Fn);
}

/// The depth of each call in the frame whose body is `body`, in the
/// order a depth-first walk meets them.
fn depths(ir: &Ir, body: IrId) -> Vec<u16> {
    ir.calls_in(body)
        .into_iter()
        .map(|(depth, _)| depth)
        .collect()
}

#[test]
fn a_call_is_as_deep_as_its_path_from_its_frames_body() {
    let g = "fn g(n: Int): Int { n }\n";
    // In a function's body: the block (0), the `+` (1), the `*` (2),
    // the call (3).
    let ir = lower_text(&format!("{g}fn f(n: Int): Int {{ 1 + g(n) * 2 }}"));
    assert_eq!(depths(&ir, ir.function(FnId::new(1)).body), vec![3]);
    // In the file's body: the block (0), the `let`'s store (1), the
    // outer call (2), its argument (3), the inner call (4).
    let ir = lower_text(&format!("{g}let x = g(g(1))\nx"));
    assert_eq!(depths(&ir, ir.root()), vec![2, 4]);
    // A callee is a child of its call: the block (0), the call of the
    // closure (1), the call that makes it (2).
    let ir = lower_text("fn make(): fn(): Int { fn(): Int { 1 } }\nmake()()");
    assert_eq!(depths(&ir, ir.root()), vec![1, 2]);
}

#[test]
fn the_shared_nodes_of_an_update_chain_do_not_disturb_depths() {
    let g = "fn g(n: Int): Int { n }\n";
    // `let a.b.c = g(a.b.c)`: the block (0), the store (1), the update
    // (2), the call (3), the update's value, its first child, which no
    // node of its path shares; the later call's depth is its own path's
    // (1).
    let ir = lower_text(&format!(
        "{g}let a = {{ b = {{ c = 1 }} }}\nlet a.b.c = g(a.b.c)\n= g(a.b.c)"
    ));
    assert_eq!(depths(&ir, ir.root()), vec![3, 1]);
    // In a function's body, where the update's longest path is through
    // the call: the block (1), the store (2), the update, two levels
    // more for each of its path's two steps (7), the call (8), its
    // argument (9), the literal (10).
    let ir = lower_text(&format!(
        "{g}fn u(r: {{b: {{c: Int}}}}): {{b: {{c: Int}}}} {{ let r.b.c = g(1)\nr }}"
    ));
    let u = ir.function(FnId::new(1));
    assert_eq!(depths(&ir, u.body), vec![3]);
    assert_eq!(u.height, 10);
}

#[test]
fn a_closures_body_is_a_frame_of_its_own() {
    let text = "fn g(n: Int): Int { n }\nlet k = { { fn(): Int { g(1) } } }\ng(k())";
    let ir = lower_text(text);
    // In the frame creating the closure: the block (0), the call of `g`
    // (1), its argument (2), the call of `k` (3). The call in the
    // closure's body is not in this frame.
    assert_eq!(depths(&ir, ir.root()), vec![1, 3]);
    // In the closure's own, however deep it was made: the block (0), the
    // call (1). Its body's height: the block, the call, its argument, the
    // literal.
    let closure = ir.function(FnId::new(1));
    assert_eq!(depths(&ir, closure.body), vec![1]);
    assert_eq!(closure.height, 4);
    // A body that makes a closure is as tall as the loads of what it
    // captures, not the closure's body: the block and the closure, which
    // captures nothing, since `g` is a top-level function, read through
    // the top level's frame.
    let ir =
        lower_text("fn g(n: Int): Int { n }\nfn outer(): fn(): Int { fn(): Int { g(g(g(1))) } }");
    assert_eq!(ir.function(FnId::new(2)).height, 2);
    // The block, three calls and their arguments, and the literal.
    assert_eq!(ir.function(FnId::new(1)).height, 8);
}

#[test]
fn a_function_body_is_as_tall_as_its_longest_path() {
    // The block (1), the `if` (2), the then-block (3), the negations (4
    // and 5), the load (6); the condition and the else branch are
    // shorter.
    let ir = lower_text("fn h(n: Int): Int { if n > 0 { -(-n) } else { 0 } }");
    assert_eq!(ir.function(FnId::new(0)).height, 6);
    // An empty body is a block with an empty result.
    let ir = lower_text("fn e(): {} {}");
    assert_eq!(ir.function(FnId::new(0)).height, 2);
}

#[test]
fn an_argument_bound_to_no_parameter_is_an_internal_error() {
    let ast = parse_text("fn f(a: Int): Int { a }\nf(1)");
    let mut types = TypeTable::new();
    let (mut checked, _) = check(&ast, &mut types).unwrap();
    checked.resolution.unbind_arguments();
    let error = lower(&ast, &checked, &types, 0).unwrap_err();
    assert_eq!(
        error.message,
        "an argument of a call is bound to no parameter"
    );
    // A resolution without the tree's functions does not belong to it.
    let unresolved = Checked {
        types: checked.types.clone(),
        resolution: Resolution::without_variables(ast.len(), ast.item_count()),
    };
    let error = lower(&ast, &unresolved, &types, 0).unwrap_err();
    assert_eq!(error.message, "resolution does not belong to this AST");
}

// ----- Futures -----

#[test]
fn a_lazy_makes_a_future_capturing_in_the_creating_frame() {
    let text = "let x = 1\nlet f = lazy x + 1";
    assert_eq!(
        dump(text),
        "(block @0..28\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store f#1 @10..28\n\
             \x20   (make-future #0 @18..28\n\
             \x20     (load x#0 @18..28)))\n\
             \x20 (empty @0..28))\n\
             future #0 (frame 0, params 0, captures 1, height 2)\n\
             \x20 (int-add @23..28\n\
             \x20   (load-capture 0 @23..24)\n\
             \x20   (int 1 @27..28))"
    );
    let ir = lower_text(text);
    let code = ir.function(FnId::new(0));
    assert_eq!(code.kind, FnKind::Future);
    assert_eq!(&*code.display, "Future<Int>");
    assert_eq!(
        (code.param_count, code.capture_count, code.name),
        (0, 1, None)
    );
    // A future's body is any node, of the kind of what it yields.
    assert_eq!(ir.kind_of(code.body), ValueKind::Int);
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    let IrKind::Store(_, future) = ir.get(ir.items(items)[1]).kind else {
        panic!("expected a store");
    };
    assert_eq!(ir.kind_of(future), ValueKind::Deferred);
    // A future made in a function body is made in that frame. `n` is
    // only captured by the `lazy`, which does not measure it for `f`,
    // so it is the either shape and its load `Deferred`.
    let ir = lower_text("fn f(n: Int): Future<Int> { lazy n * 2 }");
    assert_eq!(ir.function(FnId::new(0)).kind, FnKind::Future);
    assert_eq!(ir.function(FnId::new(1)).kind, FnKind::Function);
    let IrKind::Block { result, .. } = ir.get(ir.function(FnId::new(1)).body).kind else {
        panic!("expected a block");
    };
    let IrKind::MakeFuture {
        function, captures, ..
    } = ir.get(result).kind
    else {
        panic!("expected a future");
    };
    assert_eq!(function, FnId::new(0));
    assert_eq!(
        ir.items(captures)
            .iter()
            .map(|&id| ir.get(id).kind)
            .collect::<Vec<_>>(),
        vec![IrKind::Load(Slot::new(0), ValueKind::Deferred)]
    );
}

#[test]
fn an_await_wraps_what_is_awaited() {
    // At the top, of the kind of what is awaited once present.
    let text = "let x = lazy 1\nx + 1";
    assert_eq!(
        dump(text),
        "(block @0..20\n\
             \x20 (store x#0 @0..14\n\
             \x20   (make-future #0 @8..14))\n\
             \x20 (int-add @15..20\n\
             \x20   (await @15..16\n\
             \x20     (load x#0 @15..16))\n\
             \x20   (int 1 @19..20)))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 1 @13..14)"
    );
    let ir = lower_text(text);
    let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
        panic!("expected an addition");
    };
    let IrKind::Await {
        value, kind, shape, ..
    } = ir.get(operands.lhs).kind
    else {
        panic!("expected an await");
    };
    assert_eq!(kind, ValueKind::Int);
    assert_eq!(shape, LazyShapeId::NONE);
    assert_eq!(ir.kind_of(operands.lhs), ValueKind::Int);
    assert_eq!(ir.kind_of(value), ValueKind::Deferred);
    // An explicit await is one node, spanning the `await`.
    assert_eq!(
        dump("let x = lazy 1\nawait x").lines().nth(3),
        Some("  (await @15..22")
    );
    let ir = lower_text("let x = lazy 1\nawait x");
    let awaited = result_after_items(&ir);
    let IrKind::Await { value, .. } = ir.get(awaited).kind else {
        panic!("expected an await");
    };
    assert!(matches!(ir.get(value).kind, IrKind::Load(..)));
    // Of a present value, it is its operand.
    assert_eq!(
        dump("let x = 1\nawait x"),
        "(block @0..17\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (load x#0 @16..17))"
    );
    // A parenthesis is dropped, and the await spans it.
    assert_eq!(
        dump("let x = lazy 1\n(x) + 1").lines().nth(4),
        Some("    (await @15..18")
    );
}

#[test]
fn a_call_of_an_inferred_present_result_is_not_awaited() {
    // An impure function whose body returns a present record of present
    // fields returns one: its call is of a concrete kind, and neither it
    // nor its field is awaited.
    let body = "{ a = 1; b = \"x\" }\n";
    let text = format!("fn get() -> {{a: Int, b: String}} {body}get().a + 1");
    let ir = lower_text(&text);
    let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
        panic!("expected an addition");
    };
    let IrKind::Field { record, kind, .. } = ir.get(operands.lhs).kind else {
        panic!("expected a field access");
    };
    assert_eq!(kind, ValueKind::Int);
    let IrKind::Call { kind, .. } = ir.get(record).kind else {
        panic!("expected a call");
    };
    assert_eq!(kind, ValueKind::Record);
    assert!(!dump(&text).contains("(await"));
    // Written as the either shape, the call may be a future, and so may
    // its field: both are awaited.
    let either = "{a: Int | Future<Int>, b: String | Future<String>}";
    let text = format!("fn get() -> {either} | Future<{either}> {body}get().a + 1");
    let ir = lower_text(&text);
    let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
        panic!("expected an addition");
    };
    let IrKind::Await { value, .. } = ir.get(operands.lhs).kind else {
        panic!("expected an await of the field");
    };
    let IrKind::Field { record, .. } = ir.get(value).kind else {
        panic!("expected a field access");
    };
    let IrKind::Await { value, .. } = ir.get(record).kind else {
        panic!("expected an await of the call");
    };
    let IrKind::Call { kind, .. } = ir.get(value).kind else {
        panic!("expected a call");
    };
    assert_eq!(kind, ValueKind::Deferred);
    assert_eq!(dump(&text).matches("(await").count(), 2);
}

#[test]
fn a_deep_await_carries_a_lazy_shape() {
    let text = "let r = { a = lazy 1; b = { c = lazy 2; d = 3 }; e = 4 }\n\"{r}\"";
    let shown = dump(text);
    assert!(
        shown.contains(
            "\n  (interp @57..62\n\
                 \x20   (str \"\" @57..62)\n\
                 \x20   (await [a! b[c!]] @59..60\n\
                 \x20     (load r#0 @59..60))\n"
        ),
        "{shown}"
    );
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let ir = lower(&ast, &checked, &types, 0).unwrap();
    assert_eq!(ir.lazy_shape_count(), 2);
    let IrKind::Interp(items) = ir.get(result_after_items(&ir)).kind else {
        panic!("expected an interpolation");
    };
    let IrKind::Await { shape, kind, .. } = ir.get(ir.items(items)[1]).kind else {
        panic!("expected an await");
    };
    assert_eq!(kind, ValueKind::Record);
    let name = |text: &str| ast.names().lookup(text).unwrap();
    let [
        (a, LazyField::Await(LazyShapeId::NONE)),
        (b, LazyField::Within(inner)),
    ] = *ir.lazy_shape(shape).fields
    else {
        panic!("unexpected shape {:?}", ir.lazy_shape(shape));
    };
    assert_eq!((a, b), (name("a"), name("b")));
    assert_eq!(
        &*ir.lazy_shape(inner).fields,
        &[(name("c"), LazyField::Await(LazyShapeId::NONE))]
    );
    assert_eq!(ir.find_lazy_shape(LazyShapeId::NONE), None);
    // A field that is lazy and descended into, and a present record
    // with lazy fields.
    let shown =
        dump("let r = { a = lazy { x = lazy 1; y = 2 }; b = { c = { d = lazy 3 } } }\n= \"{r}\"");
    assert!(shown.contains("(await [a![x!] b[c[d!]]] @"), "{shown}");
    // Only the lazy parts are listed; a record with none needs no
    // shape, and is not awaited.
    let shown = dump("let r = { a = 1; b = { c = 2 } }\n\"{r}\"");
    assert!(!shown.contains("await"), "{shown}");
    // Shapes are shared by the deep awaits of one type.
    let ir = lower_text("let r = { a = lazy 1 }\nlet s = \"{r}\"\nlet t = \"{r} {r}\"");
    assert_eq!(ir.lazy_shape_count(), 1);
    // An equality awaits deeply and compares over the present fields;
    // the nested record type, in an impure signature, may be a future
    // itself.
    let shown = dump("fn f(r: {a: {b: Int}}) -> Bool { r == r }");
    assert!(shown.contains("\n    (record-eq == [a[b]] @"), "{shown}");
    assert!(shown.contains("\n      (await [a![b!]] @"), "{shown}");
}

#[test]
fn values_that_may_be_futures_are_of_kind_deferred() {
    // An `if` of such a type, with a present branch.
    let text = "let x = lazy 1\nlet y = if true { x } else { 2 }";
    assert_eq!(
        dump(text),
        "(block @0..47\n\
             \x20 (store x#0 @0..14\n\
             \x20   (make-future #0 @8..14))\n\
             \x20 (store y#1 @15..47\n\
             \x20   (if @23..47\n\
             \x20     (bool true @26..30)\n\
             \x20     (block @31..36\n\
             \x20       (load x#0 @33..34))\n\
             \x20     (block @42..47\n\
             \x20       (int 2 @44..45))))\n\
             \x20 (empty @0..47))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 1 @13..14)"
    );
    let ir = lower_text(text);
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    let IrKind::Store(_, choice) = ir.get(ir.items(items)[1]).kind else {
        panic!("expected a store");
    };
    assert_eq!(ir.kind_of(choice), ValueKind::Deferred);
    // A present value stored in a slot of such a type.
    assert_eq!(
        dump("let x: Int | Future<Int> = 1"),
        "(block @0..28\n\
             \x20 (store x#0 @0..28\n\
             \x20   (int 1 @27..28))\n\
             \x20 (empty @0..28))"
    );
    // A pure function passing an either parameter along: a load of
    // kind `Deferred`, stored in a record, never awaited.
    let text = "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }";
    assert_eq!(
        dump(text),
        "(block @0..63\n\
             \x20 (store keep#0 @0..63\n\
             \x20   (make-closure #0 @0..63))\n\
             \x20 (empty @0..63))\n\
             fn #0 keep (frame 2, params 1, captures 0, height 3)\n\
             \x20 (block @54..63\n\
             \x20   (store v#1 @56..61\n\
             \x20     (load x#0 @60..61))\n\
             \x20   (make-record [v] @54..63\n\
             \x20     (load v#1 @56..57)))"
    );
    let ir = lower_text(text);
    let IrKind::Block { items, .. } = ir.get(ir.function(FnId::new(0)).body).kind else {
        panic!("expected a block");
    };
    let IrKind::Store(_, load) = ir.get(ir.items(items)[0]).kind else {
        panic!("expected a store");
    };
    assert_eq!(
        ir.get(load).kind,
        IrKind::Load(Slot::new(0), ValueKind::Deferred)
    );
    // A bare `await` item of a value that is not `{}`.
    let ir = lower_text("let x = lazy 1\nawait x\n2");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    assert!(matches!(
        ir.get(ir.items(items)[1]).kind,
        IrKind::Await {
            kind: ValueKind::Int,
            ..
        }
    ));
}

#[test]
fn awaits_are_as_deep_as_their_path_from_their_frames_body() {
    // In the file's body: the block (0), the `let`'s store (1), the `+`
    // (2), the await (3). And the explicit await of the result (1).
    let ir = lower_text("let x = lazy 1\nlet y = x + 1\nawait x");
    assert_eq!(
        ir.awaits_in(ir.root())
            .into_iter()
            .map(|(depth, _)| depth)
            .collect::<Vec<_>>(),
        vec![3, 1]
    );
    // In a function's body, which counts the await in its height: the
    // block (1), the `*` (2), the await (3), the load (4).
    let ir = lower_text("fn f(x: Int) -> Int { x * 2 }");
    let code = ir.function(FnId::new(0));
    assert_eq!(
        ir.awaits_in(code.body)
            .into_iter()
            .map(|(depth, _)| depth)
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(code.height, 4);
    // In a future's body: the await (0), the capture (1).
    let ir = lower_text("let x = lazy 1\nlet y = lazy x + 1");
    let code = ir.function(FnId::new(1));
    assert_eq!(
        ir.awaits_in(code.body)
            .into_iter()
            .map(|(depth, _)| depth)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(code.height, 3);
}

#[test]
fn an_update_awaits_each_record_along_its_path_that_may_be_a_future() {
    // An impure function's parameter, the either shape at every level:
    // each record is awaited, the variable's by an await of its read,
    // spanning its name, and the record at `r.a` by the step that reads
    // it, spanning the path up to it.
    let text = "fn f(r: {a: {b: Int}}) -> {} { let r.a.b = 2 }";
    assert_eq!(
        dump(text),
        "(block @0..46\n\
             \x20 (store f#0 @0..46\n\
             \x20   (make-closure #0 @0..46))\n\
             \x20 (empty @0..46))\n\
             fn #0 f (frame 2, params 1, captures 0, height 9)\n\
             \x20 (block @29..46\n\
             \x20   (store r#1 @31..44\n\
             \x20     (update .a!.b @35..40\n\
             \x20       (int 2 @43..44)\n\
             \x20       (await @35..36\n\
             \x20         (load r#0 @35..36))))\n\
             \x20   (empty @29..46))"
    );
    let ir = lower_text(text);
    let code = ir.function(FnId::new(0));
    let IrKind::Block { items, .. } = ir.get(code.body).kind else {
        panic!("expected a block");
    };
    let IrKind::Store(_, update) = ir.get(ir.items(items)[0]).kind else {
        panic!("expected a store");
    };
    let IrKind::Update {
        parts,
        path,
        kind: ValueKind::Record,
        ..
    } = ir.get(update).kind
    else {
        panic!("expected an update of a record");
    };
    let &[_, first] = ir.items(parts) else {
        panic!("expected a value and a record");
    };
    // The read is of kind `Deferred`, and the await of a record.
    let IrKind::Await {
        value: load,
        kind: ValueKind::Record,
        shape: LazyShapeId::NONE,
        ..
    } = ir.get(first).kind
    else {
        panic!("expected an await of a record");
    };
    assert_eq!(
        ir.get(load).kind,
        IrKind::Load(Slot::new(0), ValueKind::Deferred)
    );
    let steps = ir.update_path(path).unwrap();
    let modes: Vec<_> = steps.iter().map(|step| (step.mode, step.span)).collect();
    assert_eq!(
        modes,
        vec![
            (StepMode::Await, Span::new(35, 38)),
            (StepMode::Set, Span::new(35, 40))
        ]
    );
    // The block (0), the store (1), the update (2), its record's await
    // (3); the step's await is the update's own.
    assert_eq!(
        ir.awaits_in(code.body)
            .into_iter()
            .map(|(depth, _)| depth)
            .collect::<Vec<_>>(),
        vec![3]
    );
    // A record that is present is read as before, beside one that is
    // not.
    let text = "let r = { a = lazy { b = 1 } }\nlet r.a.b = 2";
    let shown = dump(text);
    assert!(
        shown.contains(
            "  (store r#1 @31..44\n\
                 \x20   (update .a!.b @35..40\n\
                 \x20     (int 2 @43..44)\n\
                 \x20     (load r#0 @35..36)))"
        ),
        "{shown}"
    );
}

#[test]
fn the_shared_nodes_of_a_dotted_fields_chain_do_not_disturb_depths() {
    // `a.b.d = g(a.b.c)`, extending `a`: the block (0), its item's block
    // (1), the store (2), the update of `b` (3), the update of `d` (4),
    // the call (5). The update of `b` shares its load of `a` with the
    // access `a.b` that the update of `d` builds on, so the walk
    // reaches that load twice; the call's depth is its own path's.
    let ir = lower_text("fn g(n: Int): Int { n }\n{ a.b.c = 1; a.b.d = g(a.b.c) }");
    assert_eq!(depths(&ir, ir.root()), vec![5]);
}

#[test]
fn an_index_path_lowers_to_one_update_of_its_list() {
    // The value, the list and each position, in the order they are
    // evaluated, and a step for each segment after the first.
    assert_eq!(
        dump("let mut xs = [1, 2]\nlet i = 0\nxs[i + 1] := 5"),
        "(block @0..44\n\
             \x20 (store xs#0 @0..19\n\
             \x20   (make-list @13..19\n\
             \x20     (int 1 @14..15)\n\
             \x20     (int 2 @17..18)))\n\
             \x20 (store i#1 @20..29\n\
             \x20   (int 0 @28..29))\n\
             \x20 (store xs#0 @30..44\n\
             \x20   (update [] @30..39\n\
             \x20     (int 5 @43..44)\n\
             \x20     (load xs#0 @30..32)\n\
             \x20     (int-add @33..38\n\
             \x20       (load i#1 @33..34)\n\
             \x20       (int 1 @37..38))))\n\
             \x20 (empty @0..44))"
    );
    // Names and indexes mixed, two in a row, and a record updated: the
    // update is of the kind of what it updates.
    let ir =
        lower_text("let mut conf = { hosts = [[{ up = false }]] }\nconf.hosts[0][1].up := true");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("expected a block");
    };
    let IrKind::Store(_, update) = ir.get(ir.items(items)[1]).kind else {
        panic!("expected a store");
    };
    let IrKind::Update {
        parts,
        path,
        kind: ValueKind::Record,
        ..
    } = ir.get(update).kind
    else {
        panic!("expected an update of a record");
    };
    let parts: Vec<_> = ir.items(parts).iter().map(|&id| ir.get(id).kind).collect();
    assert_eq!(
        parts,
        vec![
            IrKind::BoolConst(true),
            IrKind::Load(Slot::new(0), ValueKind::Record),
            IrKind::IntConst(0),
            IrKind::IntConst(1)
        ]
    );
    let steps: Vec<_> = ir
        .update_path(path)
        .unwrap()
        .iter()
        .map(|step| (matches!(step.at, StepAt::Element), step.mode, step.span))
        .collect();
    assert_eq!(
        steps,
        vec![
            (false, StepMode::Read, Span::new(46, 56)),
            (true, StepMode::Read, Span::new(46, 59)),
            (true, StepMode::Read, Span::new(46, 62)),
            (false, StepMode::Set, Span::new(46, 65))
        ]
    );
    // A list updated is of kind `List`, and so is its read.
    let shown = dump("let xs = [[1]]\nxs[0][0] := 2");
    assert!(
        shown.contains(
            "  (store xs#1 @15..28\n\
                 \x20   (update [][] @15..23\n\
                 \x20     (int 2 @27..28)\n\
                 \x20     (load xs#0 @15..17)\n\
                 \x20     (int 0 @18..19)\n\
                 \x20     (int 0 @21..22)))"
        ),
        "{shown}"
    );
    let ir = lower_text("let xs = [[1]]\nxs[0][0] := 2");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("expected a block");
    };
    let IrKind::Store(_, update) = ir.get(ir.items(items)[1]).kind else {
        panic!("expected a store");
    };
    assert_eq!(ir.kind_of(update), ValueKind::List);
}

#[test]
fn an_index_path_awaits_the_lists_and_elements_that_may_be_futures() {
    // An impure function's parameter and its elements, the either shape:
    // the list read is awaited, as a list, and so is the element its
    // step reads; the position too, where it is used.
    let text = "fn f(xs: List<{a: Int}>, i: Int) -> {} { xs[i].a := 1 }";
    let shown = dump(text);
    assert!(
        shown.contains(
            "    (store xs#2 @41..53\n\
                 \x20     (update []!.a @41..48\n\
                 \x20       (int 1 @52..53)\n\
                 \x20       (await @41..43\n\
                 \x20         (load xs#0 @41..43))\n\
                 \x20       (await @44..45\n\
                 \x20         (load i#1 @44..45))))"
        ),
        "{shown}"
    );
    let ir = lower_text(text);
    let code = ir.function(FnId::new(0));
    let IrKind::Block { items, .. } = ir.get(code.body).kind else {
        panic!("expected a block");
    };
    let IrKind::Store(_, update) = ir.get(ir.items(items)[0]).kind else {
        panic!("expected a store");
    };
    let IrKind::Update { parts, .. } = ir.get(update).kind else {
        panic!("expected an update");
    };
    let IrKind::Await { kind, value, .. } = ir.get(ir.items(parts)[1]).kind else {
        panic!("expected an await");
    };
    assert_eq!(kind, ValueKind::List);
    assert_eq!(
        ir.get(value).kind,
        IrKind::Load(Slot::new(0), ValueKind::Deferred)
    );
    // The update gets its depth, for its steps' awaits: the block (0),
    // the store (1), the update (2); and the awaits below it (3).
    let IrKind::Update { depth, .. } = ir.get(update).kind else {
        panic!("expected an update");
    };
    assert_eq!(depth, 2);
    assert_eq!(
        ir.awaits_in(code.body)
            .into_iter()
            .map(|(depth, _)| depth)
            .collect::<Vec<_>>(),
        vec![3, 3]
    );
    // Two levels for each of its two steps: the update is 1 + 2 + 4
    // tall, the store 8 and the block 9.
    assert_eq!(code.height, 9);
}

/// The result of the root block of `ir`, whatever items come before it.
fn result_after_items(ir: &Ir) -> IrId {
    match ir.get(ir.root()).kind {
        IrKind::Block { result, .. } => result,
        other => panic!("the root is not a block: {other:?}"),
    }
}

// ----- Lists -----

#[test]
fn a_list_literal_makes_a_list_of_its_elements() {
    assert_eq!(
        dump("[1, lazy 2, ...[3]]"),
        "(block @0..19\n\
             \x20 (make-list @0..19\n\
             \x20   (int 1 @1..2)\n\
             \x20   (make-future #0 @4..10)\n\
             \x20   (spread @12..18\n\
             \x20     (make-list @15..18\n\
             \x20       (int 3 @16..17)))))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 2 @9..10)"
    );
    let ir = lower_text("[1, 2]");
    assert!(matches!(
        ir.get(result_of(&ir)).kind,
        IrKind::MakeList { spreads: false, .. }
    ));
    assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::List);
    let ir = lower_text("[...[1]]");
    assert!(matches!(
        ir.get(result_of(&ir)).kind,
        IrKind::MakeList { spreads: true, .. }
    ));
    // An element awaited to fit its expected type is wrapped in its
    // await, and a spread's list that may be a future too.
    let shown = dump("let xs: List<Present<Int>> = [lazy 1]\nlet ys = lazy [2]\n[...ys]");
    assert!(
        shown.contains(
            "(make-list @29..37\n\
                 \x20     (await @30..36\n\
                 \x20       (make-future #0 @30..36)))"
        ),
        "{shown}"
    );
    assert!(
        shown.contains(
            "(spread @57..62\n\
                 \x20     (await @60..62\n\
                 \x20       (load ys#1 @60..62)))"
        ),
        "{shown}"
    );
}

#[test]
fn an_index_reads_an_element_of_its_kind() {
    assert_eq!(
        dump("let xs = [1.5]\nxs[0] + 1.0"),
        "(block @0..26\n\
             \x20 (store xs#0 @0..14\n\
             \x20   (make-list @9..14\n\
             \x20     (float 1.5 @10..13)))\n\
             \x20 (float-add @15..26\n\
             \x20   (index @15..20\n\
             \x20     (load xs#0 @15..17)\n\
             \x20     (int 0 @18..19))\n\
             \x20   (float 1.0 @23..26)))"
    );
    // An element that may be a future is read as one and awaited where
    // it is used.
    let ir = lower_text("let xs = [lazy 1]\nxs[0]");
    let index = result_after_items(&ir);
    assert!(matches!(
        ir.get(index).kind,
        IrKind::Index {
            kind: ValueKind::Deferred,
            ..
        }
    ));
    let shown = dump("let xs = [1, lazy 2]\nxs[1] * 2");
    assert!(
        shown.contains(
            "(int-mul @21..30\n\
                 \x20   (await @21..26\n\
                 \x20     (index @21..26\n"
        ),
        "{shown}"
    );
}

#[test]
fn len_lowers_to_the_length_of_its_list() {
    assert_eq!(
        dump_prelude("len(list = [1])"),
        "(block @0..15\n\
             \x20 (list-len @0..15\n\
             \x20   (make-list @11..14\n\
             \x20     (int 1 @12..13))))"
    );
    let (_, ir) = lower_prelude("len([1])");
    assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Int);
    // Its list is awaited if it may be a future.
    let shown = dump_prelude("let xs = lazy [1]\nlen(xs)");
    assert!(
        shown.contains("(list-len @18..25\n    (await @22..24\n"),
        "{shown}"
    );
}

#[test]
fn exec_and_panic_lower_to_calls() {
    // Ordinary built-ins: a `Call` of the value of their cell.
    let (_, ir) = lower_prelude("exec([\"true\"])");
    let call = result_of(&ir);
    let IrKind::Call { kind, .. } = ir.get(call).kind else {
        panic!("not a call: {:?}", ir.get(call));
    };
    assert_eq!(kind, ValueKind::Record);
    // A call of `panic` is a `Call` of kind `Never`, lowered as it is:
    // no operand of it is `Never`.
    for text in ["panic(\"x\")", "let s = \"x\"\npanic(s)"] {
        let (_, ir) = lower_prelude(text);
        let call = result_of_last(&ir);
        let IrKind::Call { kind, .. } = ir.get(call).kind else {
            panic!("{text}: not a call: {:?}", ir.get(call));
        };
        assert_eq!(kind, ValueKind::Never, "{text}");
        assert_eq!(ir.kind_of(call), ValueKind::Never, "{text}");
    }
    // As a branch, it leaves the `if` the other branch's kind.
    let (_, ir) = lower_prelude("let c = true\nif c { panic(\"x\") } else { 1 }");
    assert_eq!(ir.kind_of(result_of_last(&ir)), ValueKind::Int);
    let shown = dump_prelude("let c = true\nif c { panic(\"x\") } else { 1 }");
    assert!(
        shown.contains("(block @18..32\n      (call @20..30\n        (load panic#"),
        "{shown}"
    );
}

/// The result of the root block of `ir`, whatever items come before it.
fn result_of_last(ir: &Ir) -> IrId {
    let IrKind::Block { result, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block: {:?}", ir.get(ir.root()));
    };
    result
}

#[test]
fn list_equality_compares_over_a_list_shape() {
    assert_eq!(
        dump("[1] == [2]"),
        "(block @0..10\n\
             \x20 (list-eq == * @0..10\n\
             \x20   (make-list @0..3\n\
             \x20     (int 1 @1..2))\n\
             \x20   (make-list @7..10\n\
             \x20     (int 2 @8..9))))"
    );
    assert!(dump("[1] !== [2]").contains("(list-same !== @0..11\n"));
    assert!(dump("[1] === [2]").contains("(list-same === @0..11\n"));
    assert!(dump("[1] != [2]").contains("(list-eq != * @0..10\n"));
    // Through records' shapes, and records' fields through lists'.
    let text = "let q: {x: Int} = { x = 1; y = 2 }\n[q] == [q]";
    assert!(dump(text).contains("(list-eq == *[x] @"), "{}", dump(text));
    let text = "let q = { xs = [[{ a = 1 }]]; n = 1 }\nq == q";
    assert!(
        dump(text).contains("(record-eq == [xs**[a] n] @"),
        "{}",
        dump(text)
    );
    // One shape per type, nested ones included.
    let ir = lower_text("let q = [[{ a = 1 }]]\nlet a = q == q\n= q[0] == q[0]");
    assert_eq!(ir.shape_count(), 3);
    let ir = lower_text("[{ a = 1 }] == [{ a = 1 }]");
    let IrKind::ListEq(ShapedOperands { shape, .. }) = ir.get(result_of(&ir)).kind else {
        panic!("expected a list comparison");
    };
    let Some(ListShape {
        element: Some(element),
    }) = ir.list_shape(shape)
    else {
        panic!("expected a list's shape");
    };
    assert_eq!(ir.list_shape(element), None);
    assert_eq!(ir.shape(element).fields.len(), 1);
    assert!(ir.shape(shape).fields.is_empty());
}

#[test]
fn a_deep_await_of_a_list_awaits_each_element() {
    let text = "let xs = [1, lazy 2]\n\"{xs}\"";
    let shown = dump(text);
    assert!(
        shown.contains("(await *! @23..25\n      (load xs#0 @23..25))"),
        "{shown}"
    );
    let ir = lower_text(text);
    let IrKind::Interp(items) = ir.get(result_after_items(&ir)).kind else {
        panic!("expected an interpolation");
    };
    let IrKind::Await { shape, kind, .. } = ir.get(ir.items(items)[1]).kind else {
        panic!("expected an await");
    };
    assert_eq!(kind, ValueKind::List);
    assert_eq!(
        ir.lazy_list(shape),
        Some(LazyField::Await(LazyShapeId::NONE))
    );
    assert!(ir.lazy_shape(shape).fields.is_empty());
    assert_eq!(ir.lazy_list(LazyShapeId::NONE), None);
    // Each element's lazy values, and a record's lazy list.
    for (text, shape) in [
        ("let xs = [{ a = lazy 1 }]\n\"{xs}\"", "*[a!]"),
        ("let xs = [[1, lazy 2]]\n\"{xs}\"", "**!"),
        ("let xs = [lazy [1, lazy 2]]\n\"{xs}\"", "*!*!"),
        ("let r = { xs = [lazy 1]; n = 2 }\n\"{r}\"", "[xs*!]"),
        (
            "let xs = [{ a = lazy [{ b = lazy 1 }] }]\n\"{xs}\"",
            "*[a!*[b!]]",
        ),
    ] {
        let shown = dump(text);
        assert!(
            shown.contains(&format!("(await {shape} @")),
            "{text}: {shown}"
        );
    }
    // A list with nothing lazy inside is not awaited.
    assert!(!dump("let xs = [[1]]\n\"{xs}\"").contains("await"));
}

// ----- Spreads -----

#[test]
fn a_spread_stores_its_operand_once_and_each_field_by_name() {
    // The operand is held in the item's own slot, which no variable
    // owns, and each field of its static type, in the type's order, is
    // read from it into the field's slot. A top-level field of the
    // input, `a`, has its own slot from the start, which the spread
    // does not write; the field keeps the place the spread gave it in
    // the record, loaded as the definition that stands.
    let text = "let r = { a = 1; b = 2 }\n...r\na = \"s\"";
    assert_eq!(
        dump(text),
        "(block @0..37\n\
             \x20 (store r#1 @0..24\n\
             \x20   (block @8..24\n\
             \x20     (store r#1 @10..15\n\
             \x20       (int 1 @14..15))\n\
             \x20     (store b#2 @17..22\n\
             \x20       (int 2 @21..22))\n\
             \x20     (make-record [a b] @8..24\n\
             \x20       (load r#1 @10..11)\n\
             \x20       (load b#2 @17..18))))\n\
             \x20 (store ?#3 @25..29\n\
             \x20   (load r#1 @28..29))\n\
             \x20 (store b#2 @25..29\n\
             \x20   (field b @28..29\n\
             \x20     (load ?#3 @28..29)))\n\
             \x20 (store a#0 @30..37\n\
             \x20   (str \"s\" @34..37))\n\
             \x20 (make-record [a b] @0..37\n\
             \x20   (load a#0 @30..31)\n\
             \x20   (load b#2 @28..29)))"
    );
    let ir = lower_text(text);
    let IrKind::MakeRecord { values, .. } = ir.get(result_after_items(&ir)).kind else {
        panic!("expected a record");
    };
    let loads: Vec<IrKind> = ir.items(values).iter().map(|&id| ir.get(id).kind).collect();
    assert_eq!(
        loads,
        vec![
            IrKind::Load(Slot::new(0), ValueKind::String),
            IrKind::Load(Slot::new(2), ValueKind::Int)
        ]
    );
    // A later spread overrides a soft field in its slot too.
    let ir = lower_text("{ ...{ a = 1; b = 2 }; ...{ b = \"s\"; c = 3 } }");
    let IrKind::Block { result, .. } = ir.get(result_after_items(&ir)).kind else {
        panic!("expected a block");
    };
    let IrKind::MakeRecord { layout, values } = ir.get(result).kind else {
        panic!("expected a record");
    };
    let names: Vec<&str> = ir.layout(layout).texts.iter().map(AsRef::as_ref).collect();
    assert_eq!(names, vec!["a", "b", "c"]);
    let loads: Vec<IrKind> = ir.items(values).iter().map(|&id| ir.get(id).kind).collect();
    assert_eq!(
        loads,
        vec![
            IrKind::Load(Slot::new(0), ValueKind::Int),
            IrKind::Load(Slot::new(1), ValueKind::String),
            IrKind::Load(Slot::new(2), ValueKind::Int)
        ]
    );
}

#[test]
fn a_spreads_operand_that_may_be_a_future_is_awaited_once() {
    assert_eq!(
        dump("let f = lazy { a = 1 }\n...f")
            .lines()
            .collect::<Vec<_>>()[..11],
        [
            "(block @0..27",
            "  (store f#0 @0..22",
            "    (make-future #0 @8..22))",
            "  (store ?#2 @23..27",
            "    (await @26..27",
            "      (load f#0 @26..27)))",
            "  (store a#1 @23..27",
            "    (field a @26..27",
            "      (load ?#2 @26..27)))",
            "  (make-record [a] @0..27",
            "    (load a#1 @26..27)))",
        ]
    );
    // A field that may be a future is read as it is, not awaited.
    let ir = lower_text("...{ a = lazy 1 }");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    let IrKind::Store(_, field) = ir.get(ir.items(items)[1]).kind else {
        panic!("expected a store");
    };
    assert!(matches!(
        ir.get(field).kind,
        IrKind::Field {
            kind: ValueKind::Deferred,
            ..
        }
    ));
}

// ----- Ranges and `for` -----

#[test]
fn a_range_lowers_to_a_list_of_its_bounds() {
    assert_eq!(
        dump("-1..=2"),
        "(block @0..6\n\
             \x20 (range-inclusive @0..6\n\
             \x20   (int-neg @0..2\n\
             \x20     (int 1 @1..2))\n\
             \x20   (int 2 @5..6)))"
    );
    let ir = lower_text("0..2");
    let range = result_of(&ir);
    assert_eq!(ir.kind_of(range), ValueKind::List);
    assert_eq!(ir.get(range).operator_span(), Some(Span::new(1, 3)));
    let ir = lower_text("0..=2");
    assert_eq!(
        ir.get(result_of(&ir)).operator_span(),
        Some(Span::new(1, 4))
    );
    // A bound that may be a future is awaited.
    let shown = dump("let n = lazy 2\n0..n");
    assert!(
        shown.contains(
            "(range @15..19\n\
                 \x20   (int 0 @15..16)\n\
                 \x20   (await @18..19\n\
                 \x20     (load n#0 @18..19)))"
        ),
        "{shown}"
    );
}

#[test]
fn a_for_lowers_to_its_slot_and_parts() {
    assert_eq!(
        dump("let xs = [1]\nfor x in xs if x > 0 { x * 2 }"),
        "(block @0..43\n\
             \x20 (store xs#0 @0..12\n\
             \x20   (make-list @9..12\n\
             \x20     (int 1 @10..11)))\n\
             \x20 (for x#1 if @13..43\n\
             \x20   (load xs#0 @22..24)\n\
             \x20   (int-cmp > @28..33\n\
             \x20     (load x#1 @28..29)\n\
             \x20     (int 0 @32..33))\n\
             \x20   (block @34..43\n\
             \x20     (int-mul @36..41\n\
             \x20       (load x#1 @36..37)\n\
             \x20       (int 2 @40..41)))))"
    );
    let ir = lower_text("for x in [1] { x }");
    let for_node = result_of(&ir);
    let IrKind::For {
        slot,
        parts,
        collect,
        ..
    } = ir.get(for_node).kind
    else {
        panic!("expected a for, got {:?}", ir.get(for_node));
    };
    assert_eq!(slot, Slot::new(0));
    assert_eq!(ir.items(parts).len(), 2);
    assert!(collect);
    assert_eq!(ir.kind_of(for_node), ValueKind::List);
    assert_eq!(ir.get(for_node).operator_span(), None);
    // The iterable is a range node as it is, through parentheses.
    let ir = lower_text("for i in (0..3) { i }");
    let IrKind::For { parts, .. } = ir.get(result_of(&ir)).kind else {
        panic!("expected a for");
    };
    assert!(matches!(
        ir.get(ir.items(parts)[0]).kind,
        IrKind::Range(false, _)
    ));
    // An iterable that may be a future is awaited once, and an element
    // is read as it is and awaited where it is used.
    let shown = dump("let xs = lazy [lazy 1]\nfor x in xs { x + 1 }");
    assert!(
        shown.contains(
            "(for x#1 @23..44\n\
                 \x20   (await @32..34\n\
                 \x20     (load xs#0 @32..34))\n\
                 \x20   (block @35..44\n\
                 \x20     (int-add @37..42\n\
                 \x20       (await @37..38\n\
                 \x20         (load x#1 @37..38))"
        ),
        "{shown}"
    );
}

#[test]
fn a_body_awaited_to_fit_is_awaited_in_the_for() {
    // (The variable's slot is named by `ys`, the last variable stored
    // in it, as the slot of any variable of a finished block is.)
    let shown = dump("let xs = [lazy 1]\nlet ys: List<Present<Int>> = for x in xs { x }\nys");
    assert!(
        shown.contains(
            "(for ys#1 @47..64\n\
                 \x20     (load xs#0 @56..58)\n\
                 \x20     (await @59..64\n\
                 \x20       (block @59..64\n\
                 \x20         (load ys#1 @61..62))))"
        ),
        "{shown}"
    );
}

#[test]
fn a_unit_like_for_statement_builds_no_list() {
    // As it is: a `for` that discards its body's values and is `{}`.
    let ir = lower_text("for x in [1] { }\nfor y in [2] { for z in [3] { } }\n1");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    for &item in ir.items(items) {
        assert!(
            matches!(ir.get(item).kind, IrKind::For { collect: false, .. }),
            "{:?}",
            ir.get(item)
        );
        assert_eq!(ir.kind_of(item), ValueKind::Record);
    }
    assert_eq!(
        dump("(for x in [1] { })\n1"),
        "(block @0..20\n\
             \x20 (for-each x#0 @1..17\n\
             \x20   (make-list @10..13\n\
             \x20     (int 1 @11..12))\n\
             \x20   (block @14..17\n\
             \x20     (empty @14..17)))\n\
             \x20 (int 1 @19..20))"
    );
    // Unit-like only once awaited: the list is built and awaited
    // deeply.
    let shown = dump("let steps = [lazy {}]\nfor s in steps { s }\n1");
    assert!(
        shown.contains(
            "(await *! @22..42\n\
                 \x20   (for s#1 @22..42\n"
        ),
        "{shown}"
    );
    // Any other unit-like statement is evaluated and dropped, as it is.
    let ir = lower_text("[]\n[{}]\n1");
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    for &item in ir.items(items) {
        assert!(matches!(ir.get(item).kind, IrKind::MakeList { .. }));
    }
    // A `for` that is its block's result builds its list.
    let ir = lower_text("for x in [1] { }");
    assert!(matches!(
        ir.get(result_of(&ir)).kind,
        IrKind::For { collect: true, .. }
    ));
}

#[test]
fn a_for_is_one_level_above_its_parts() {
    let g = "fn g(n: Int): Int { n }\n";
    // In a function's body: the block (0), the `for` (1), its body (2),
    // the call (3); and the body's height: the block, the `for`, its
    // body, the call, its argument, the load.
    let ir = lower_text(&format!(
        "{g}fn f(xs: Int[]): Int[] {{ for x in xs {{ g(x) }} }}"
    ));
    let f = ir.function(FnId::new(1));
    assert_eq!(depths(&ir, f.body), vec![3]);
    assert_eq!(f.height, 6);
    // In the iterable and the guard: the block (0), the `for` (1), the
    // list (2) and the guard's call (2).
    let ir = lower_text(&format!("{g}for x in [g(1)] if g(x) > 0 {{ }}"));
    assert_eq!(depths(&ir, ir.root()), vec![3, 3]);
    // Nested, each `for` and its body are a level each.
    let ir = lower_text(&format!("{g}for x in [1] {{ for y in [2] {{ g(y) }} }}"));
    assert_eq!(depths(&ir, ir.root()), vec![5]);
}

/// The result of the root block of `ir`, items or no items.
fn root_result(ir: &Ir) -> IrId {
    let IrKind::Block { result, .. } = ir.get(ir.root()).kind else {
        panic!("the root is not a block");
    };
    result
}

#[test]
fn a_union_is_of_mixed_kind_unless_its_members_agree() {
    let ir = lower_text("let v = if true { 1 } else { \"one\" }\nv");
    let result = root_result(&ir);
    assert_eq!(
        ir.get(result).kind,
        IrKind::Load(Slot::new(0), ValueKind::Mixed)
    );
    // The `if` holds branches of each member's kind.
    let dumped = dump("let v = if true { 1 } else { \"one\" }\nv");
    assert!(dumped.contains("(int 1 @"), "{dumped}");
    assert!(dumped.contains("(str \"one\" @"), "{dumped}");
    // A union of lists is a list.
    let ir = lower_text("let xs: List<Int> | List<String> = [1]\nxs");
    let result = root_result(&ir);
    assert_eq!(
        ir.get(result).kind,
        IrKind::Load(Slot::new(0), ValueKind::List)
    );
    // Total equality on values of a union of kinds.
    let dumped = dump("let v: Int | String = 1\nv === v");
    assert!(dumped.contains("(value-same === @24..31"), "{dumped}");
    let dumped = dump("let v: Int | String = 1\nv !== v");
    assert!(dumped.contains("(value-same !== @24..31"), "{dumped}");
}

#[test]
fn a_test_shows_its_plan() {
    // Only what the value's type leaves in doubt is looked at.
    for (text, plan) in [
        ("let v: Int | List<Int> = 1\nv is Int", "(is int @"),
        ("let v: Int | List<Int> = 1\nv is List<Int>", "(is list @"),
        (
            "let v: List<Int | String> = []\nv is List<Int>",
            "(is [int] @",
        ),
        (
            "let v: {a: Int | String, b: Int} | Int = 1\nv is {a: Int, b: Int}",
            "(is {a: int} @",
        ),
        (
            "let v: Int | String | Bool = 1\nv is Int | String",
            "(is (int | string) @",
        ),
        ("let v: Int | String = 1\nv as Int", "(as int @"),
    ] {
        let dumped = dump(text);
        assert!(dumped.contains(plan), "{text}: {dumped}");
    }
    // An `as` is of the kind of what it narrows to.
    let ir = lower_text("let v: Int | String = 1\nv as Int");
    let result = root_result(&ir);
    assert_eq!(ir.kind_of(result), ValueKind::Int);
    // A narrowing `is` writes its slot, which its branches read.
    let dumped = dump("let v: Int | String = 1\nif v is Int { v } else { 0 }");
    assert!(
        dumped.contains(
            "(is int v#1 @27..35\n\
                 \x20     (load v#0 @27..28))\n\
                 \x20   (block @36..41\n\
                 \x20     (load v#1 @38..39))"
        ),
        "{dumped}"
    );
}

#[test]
fn a_unions_deep_await_awaits_each_members_shape() {
    let dumped = dump("let u: Int | {a: Int | Future<Int>} | List<Future<Int>> = 1\n\"{u}\"");
    assert!(dumped.contains("(await ([a!] | *!) @"), "{dumped}");
    // A union with nothing lazy inside is not awaited deeply.
    let dumped = dump("let u: Int | {a: Int} = 1\n\"{u}\"");
    assert!(!dumped.contains("(await"), "{dumped}");
}

#[test]
fn a_test_the_checker_did_not_plan_is_an_internal_error() {
    let ast = parse_text("let v: Int | String = 1\nv is Int");
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let unplanned = Checked {
        types: checked.types.clone(),
        resolution: Resolution::without_variables(ast.len(), ast.item_count()),
    };
    let error = lower(&ast, &unplanned, &types, 0).unwrap_err();
    assert!(
        matches!(
            error.message,
            "an `is` or `as` has no test" | "a declaration has no slot"
        ),
        "{error:?}"
    );
}

#[test]
fn a_for_without_its_slot_is_an_internal_error() {
    let ast = parse_text("for x in [1] { x }");
    let mut types = TypeTable::new();
    let (mut checked, _) = check(&ast, &mut types).unwrap();
    checked.resolution.forget_for_slots();
    let error = lower(&ast, &checked, &types, 0).unwrap_err();
    assert_eq!(error.message, "a `for` has no slot for its variable");
}

#[test]
fn a_condition_loop_lowers_to_a_loop_in_a_block_of_its_header() {
    assert_eq!(
        dump("for let i = 0; i < 2; i := i + 1 { i }"),
        "(block @0..38\n\
             \x20 (block @0..38\n\
             \x20   (store i#0 @4..13\n\
             \x20     (int 0 @12..13))\n\
             \x20   (loop @0..38\n\
             \x20     (int-cmp < @15..20\n\
             \x20       (load i#0 @15..16)\n\
             \x20       (int 2 @19..20))\n\
             \x20     (block @33..38\n\
             \x20       (load i#0 @35..36))\n\
             \x20     (store i#0 @22..32\n\
             \x20       (int-add @27..32\n\
             \x20         (load i#0 @27..28)\n\
             \x20         (int 1 @31..32))))))"
    );
    // `for { }` tests `true`, at its keyword; a `do` tests after.
    assert_eq!(
        dump("for { break 1 }"),
        "(block @0..15\n\
             \x20 (loop @0..15\n\
             \x20   (bool true @0..3)\n\
             \x20   (block @4..15\n\
             \x20     (break 0 @6..13\n\
             \x20       (int 1 @12..13)))))"
    );
    assert_eq!(
        dump("do { continue } for false"),
        "(block @0..25\n\
             \x20 (do @0..25\n\
             \x20   (bool false @20..25)\n\
             \x20   (block @3..15\n\
             \x20     (continue 0 @5..13))))"
    );
    let ir = lower_text("do { 1 } for false");
    let node = result_of(&ir);
    assert_eq!(ir.kind_of(node), ValueKind::List);
    assert_eq!(ir.get(node).operator_span(), None);
}

#[test]
fn a_stop_counts_the_loops_it_leaves() {
    let shown = dump("for'o x in [1] { for y in [2] { if y > x { break'o }; continue } }");
    assert!(shown.contains("(break 1 @"), "{shown}");
    assert!(shown.contains("(continue 0 @"), "{shown}");
    // A function's body starts with no loop around it.
    let shown = dump("for x in [1] { let f = fn(): List<Int> { for y in [2] { break y } }; x }");
    assert!(shown.contains("(break 0 @"), "{shown}");
    // A stop is of kind Never, and so is a block ending in one, and an
    // `if` is of the kind of its branch that is not.
    let ir = lower_text("for x in [1] { if x > 0 { break } else { 5 } }");
    let IrKind::For { parts, .. } = ir.get(result_of(&ir)).kind else {
        panic!("expected a for");
    };
    let body = ir.items(parts)[1];
    assert_eq!(ir.kind_of(body), ValueKind::Int);
    let IrKind::Block { result, .. } = ir.get(body).kind else {
        panic!("expected a block");
    };
    let IrKind::If { then, .. } = ir.get(result).kind else {
        panic!("expected an if");
    };
    assert_eq!(ir.kind_of(then), ValueKind::Never);
}

#[test]
fn a_never_operation_lowers_to_its_operands_up_to_the_one_that_stops() {
    // The left operand runs, as a statement, then the right one, which
    // stops; the addition is never made. (`x`'s slot is named by `v`,
    // the last variable stored in it.)
    let shown = dump("let mut n = 0\nlet v = for x in [1] { ({ n := 1; x }) + { break } }");
    assert!(
        shown.contains(
            "(block @37..64\n\
                 \x20         (block @38..51\n\
                 \x20           (store n#0 @40..46\n\
                 \x20             (int 1 @45..46))\n\
                 \x20           (load v#1 @48..49))\n\
                 \x20         (block @55..64\n\
                 \x20           (break 0 @57..62)))"
        ),
        "{shown}"
    );
    // A function in what never runs is still lowered.
    let ir = lower_text("let v = for x in [1] { { break } + (fn(): Int { 1 })() }\nv");
    assert_eq!(ir.function_count(), 1);
    // A loop over a value of type Never is its iterable.
    let shown = dump("let v = for x in [1] { for y in { break } { y } }\nv");
    assert!(!shown.contains("(for y"), "{shown}");
}

#[test]
fn a_return_names_what_it_leaves() {
    // A function's or a `lazy`'s, by `fn`.
    let shown = dump("fn f(n: Int): Int { if n > 0 { return n }; 0 }\nf(1)");
    assert!(shown.contains("(return fn @31..39\n"), "{shown}");
    let shown = dump("let l = lazy { return 1 }\nawait l");
    assert!(shown.contains("(return fn @15..23\n"), "{shown}");
    // A labelled block's, by the number of its catch, around it; the
    // block keeps its own kind, the catch takes the join's.
    let ir = lower_text("let c = true\n{'b if c { return'b 1 }; \"s\" }");
    let IrKind::Catch { body, id, kind } = ir.get(result_after_items(&ir)).kind else {
        panic!("expected a catch: {:?}", ir.get(result_after_items(&ir)));
    };
    assert_eq!(id, CatchId::new(0));
    assert_eq!(kind, ValueKind::Mixed);
    assert_eq!(ir.kind_of(body), ValueKind::String);
    let shown = dump("let c = true\n{'b if c { return'b 1 }; \"s\" }");
    assert!(
        shown.contains("(catch 0 @13..43\n    (block @13..43\n"),
        "{shown}"
    );
    assert!(shown.contains("(return 0 @24..34\n"), "{shown}");
    // A labelled block no `return` leaves is its block, as any.
    let shown = dump("{'b 1 }");
    assert!(!shown.contains("catch"), "{shown}");
    // Nested catches are numbered as they start, and a `return` finds
    // its own.
    let shown = dump("{'a {'b if true { return'a 1 }; return'b 2 } }");
    assert!(shown.contains("(catch 0 @"), "{shown}");
    assert!(shown.contains("(catch 1 @"), "{shown}");
    assert!(shown.contains("(return 0 @"), "{shown}");
    assert!(shown.contains("(return 1 @"), "{shown}");
    // A loop's, around the loop, which keeps collecting.
    let shown = dump("let xs = [1, 7]\nlet v = for'l x in xs { if x > 5 { return'l x }; x }");
    // (`x`'s slot is named by `v`, the last variable stored in it.)
    assert!(
        shown.contains("(catch 0 @24..68\n      (for v#1 @24..68\n"),
        "{shown}"
    );
    assert!(shown.contains("(return 0 @51..61\n"), "{shown}");
}

#[test]
fn a_loop_that_cannot_end_collects_nothing() {
    // Whatever its body's type.
    let shown = dump("let v: Int = for'spin { if true { return'spin 1 }; 5 }\nv");
    assert!(shown.contains("(loop-each @13..54\n"), "{shown}");
    assert!(shown.contains("(catch 0 @13..54\n"), "{shown}");
    // As a statement, and of type Never where an `Int` is wanted.
    let shown = dump("let v = for'o x in [1] { let y: Int = 1 + for { break'o }; y }\nv");
    assert!(shown.contains("(loop-each @"), "{shown}");
    assert!(!shown.contains("catch"), "{shown}");
    // A loop that can end keeps its list.
    let shown = dump("let v = for { break 1 }\nv");
    assert!(shown.contains("(loop @"), "{shown}");
}

#[test]
fn a_statement_of_a_unit_like_union_is_lowered_as_it_is() {
    let ir = lower_text("fn f() -> {} { for'l x in [1] { if x > 0 { return'l } }; {} }\nf()");
    let Some(code) = ir.find_function(FnId::new(0)) else {
        panic!("expected a function");
    };
    let IrKind::Block { items, .. } = ir.get(code.body).kind else {
        panic!("expected a block");
    };
    let [statement] = ir.items(items) else {
        panic!("expected one statement");
    };
    let IrKind::Catch { body, kind, .. } = ir.get(*statement).kind else {
        panic!("expected a catch: {:?}", ir.get(*statement));
    };
    assert_eq!(kind, ValueKind::Mixed);
    // The statement loop discards its values.
    assert!(matches!(
        ir.get(body).kind,
        IrKind::For { collect: false, .. }
    ));
}

/// Everything the checker and lowering decide about `text` that does
/// not depend on where its items are written: the file's type; each
/// diagnostic's severity and message, sorted; and the lines of the IR
/// dump with spans, slot numbers and function numbers removed, the
/// names of a record's fields sorted, and the lines sorted, so that
/// neither slots nor item order count.
fn decided(text: &str) -> (String, Vec<String>, Vec<String>) {
    decided_by(text, |text| {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let output = crate::check::Checker::new(&ast, &mut types).check();
        (ast, types, output)
    })
}

/// What a check of a text gives: its tree, its types, and the outcome.
type CheckedText = (
    Ast,
    TypeTable,
    Result<crate::check::CheckOutput, Vec<crate::diagnostic::Diagnostic>>,
);

/// As [`decided`], `text` checked by `check`.
fn decided_by(text: &str, check: fn(&str) -> CheckedText) -> (String, Vec<String>, Vec<String>) {
    let (ast, types, output) = check(text);
    let output = match output {
        Ok(output) => output,
        Err(diagnostics) => {
            let mut messages: Vec<String> = diagnostics
                .iter()
                .map(|d| format!("{:?}: {}", d.severity, d.message))
                .collect();
            messages.sort();
            return (String::new(), messages, Vec::new());
        }
    };
    let mut messages: Vec<String> = output
        .warnings
        .iter()
        .map(|d| format!("{:?}: {}", d.severity, d.message))
        .collect();
    messages.sort();
    let shown = types.display(output.ty, ast.names()).to_string();
    let ir = lower(&ast, &output.checked, &types, 0).unwrap();
    let mut lines: Vec<String> = ir
        .pretty(ast.names())
        .to_string()
        .lines()
        .map(normalised)
        .collect();
    lines.sort();
    (shown, messages, lines)
}

/// A line of an IR dump without its span, the numbers after each `#`,
/// and with the names in its `[...]` sorted.
fn normalised(line: &str) -> String {
    let line = line.split(" @").next().unwrap_or(line);
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '#' {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        }
    }
    if let (Some(open), Some(close)) = (out.find('['), out.find(']')) {
        let mut names: Vec<&str> = out[open + 1..close].split(' ').collect();
        names.sort_unstable();
        out = format!("{}[{}]{}", &out[..open], names.join(" "), &out[close + 1..]);
    }
    out
}

/// Asserts that `parts`, joined by newlines in the order given and in
/// the reverse order, decide the same: a program and its reordering
/// checked against each other. Returns what they decide.
fn order_free(parts: &[&str]) -> (String, Vec<String>, Vec<String>) {
    let forward = parts.join("\n");
    let backward: Vec<&str> = parts.iter().rev().copied().collect();
    let backward = backward.join("\n");
    let first = decided(&forward);
    assert_eq!(first, decided(&backward), "{forward}\n----\n{backward}");
    first
}

/// As [`order_free`], each order checked as a lone file is, beside the
/// standard library, so that the prelude's names are its.
fn order_free_beside_prelude(parts: &[&str]) -> (String, Vec<String>, Vec<String>) {
    let forward = parts.join("\n");
    let backward: Vec<&str> = parts.iter().rev().copied().collect();
    let backward = backward.join("\n");
    let check = crate::workspace::check_beside_prelude;
    let first = decided_by(&forward, check);
    assert_eq!(
        first,
        decided_by(&backward, check),
        "{forward}\n----\n{backward}"
    );
    first
}

#[test]
fn the_prelude_s_names_do_not_depend_on_order() {
    // Its names read anywhere, shadowed or not, as the items fall.
    let (shown, messages, lines) = order_free_beside_prelude(&[
        "n = len(xs) + count(xs)",
        "xs = [1, 2, 3]",
        "fn count(v: Int[]): Int { len(v) }",
        "h = half(n)",
        "def float(x: Int): Float { if x > 0 { 0.5 } else { 1.5 } }",
        "f = float(2)",
        "i = int(2.5)",
    ]);
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(
        shown,
        "{f: Float, float: fn(x: Int): Float, h: Float, i: Int, n: Int, xs: List<Int>}"
    );
    assert!(
        lines.iter().any(|line| line.contains("list-len")),
        "{lines:?}"
    );
    // The file's own `float` shadows the prelude's silently: it is 0.5.
    let text = "def float(x: Int): Float { if x > 0 { 0.5 } else { 1.5 } }\nf = float(2)";
    let f = crate::run(&SourceFile::new("t.gcsl", text).unwrap());
    assert_eq!(
        f.unwrap().value.to_string(),
        "{float = fn(x: Int): Float; f = 0.5}"
    );
}

#[test]
fn reads_of_shadowing_declarations_do_not_depend_on_order() {
    // A read above, below or in a function sees the last of a run of
    // shadowable declarations, each run kept in its order as one part.
    let (shown, messages, lines) = order_free(&[
        "x = a + 1",
        "#[shadowable] a = 1\na = 2",
        "fn get(): Int { a }",
        "y = get()",
        "#[shadowable] d = {j = 4}\nd.j = 8",
        "z = d.j",
    ]);
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(shown, "{a: Int, d: {j: Int}, x: Int, y: Int, z: Int}");
    // The shadowed declarations are lowered to nothing: no `4`.
    assert!(
        !lines.iter().any(|line| line.contains("int 4")),
        "{lines:?}"
    );
    // And beside the prelude, whose names a dotted root shadows.
    let (shown, messages, _) =
        order_free_beside_prelude(&["n = len.x", "len.x = 1", "m = trim(\" a \")"]);
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(shown, "{len: {x: Int}, m: String, n: Int}");
}

#[test]
fn the_reordering_helper_sees_a_difference() {
    // A field's position decides nothing it compares, but a `let`'s
    // does: below its user it is undefined.
    assert_eq!(
        normalised("(make-record [x run show] @0..75"),
        "(make-record [run show x]"
    );
    assert_eq!(normalised("  (fn top even#12 @51..55)"), "  (fn top even#");
    assert_ne!(decided("let k = 1\nx = k"), decided("x = k\nlet k = 1"));
}

/// Asserts that `parts`, joined by newlines in the order given and in
/// the reverse order, check to the same type, warnings and errors,
/// whose messages it returns with the type.
fn checks_order_free(parts: &[&str]) -> (String, Vec<String>) {
    let forward = parts.join("\n");
    let backward: Vec<&str> = parts.iter().rev().copied().collect();
    let backward = backward.join("\n");
    let (shown, messages, _) = decided(&forward);
    let (again, also, _) = decided(&backward);
    assert_eq!(
        (&shown, &messages),
        (&again, &also),
        "{forward}\n----\n{backward}"
    );
    (shown, messages)
}

#[test]
fn top_level_fields_do_not_depend_on_order() {
    // Valid programs: the same type and, normalised, the same IR.
    let (shown, messages, _) = order_free(&[
        "com.x.app.deps = [com.x.lib]",
        "com.x.app.name = \"app-{version}\"",
        "com.x.lib.name = \"lib\"",
        "version = describe(2)",
        "fn describe(n: Int): String { \"v{n}\" }",
    ]);
    assert!(messages.is_empty(), "{messages:?}");
    assert!(shown.contains("version: String"), "{shown}");
    for parts in [
        &["a.b = 1", "c = a.b + a.d", "a.d = 2"][..],
        &["x = f()", "def f(): Int { y }", "y = 3"][..],
        &["s = lazy { t }", "t.u = 1", "t.v = [t.u]"][..],
    ] {
        let (_, messages, _) = order_free(parts);
        assert!(messages.is_empty(), "{parts:?}: {messages:?}");
    }
    for (parts, expected) in [
        (
            &["a = f()", "fn f(): Int { a }"][..],
            "Error: `a` depends on itself",
        ),
        (
            &["a.b = 1", "a.b = 2"][..],
            "Error: field `a.b` is defined twice",
        ),
        (
            &["a = 1", "let a = 2"][..],
            "Error: `a` is a field of this file",
        ),
        (&["...{a = {b = 1}}", "a.c = 2"][..], ""),
    ] {
        let (_, messages) = checks_order_free(parts);
        assert_eq!(messages.len(), 1, "{parts:?}: {messages:?}");
        assert!(
            expected.is_empty() || messages.first().map(String::as_str) == Some(expected),
            "{parts:?}: {messages:?}"
        );
    }
}

#[test]
fn inferred_parameters_do_not_depend_on_order() {
    // `show` leaves its parameter provisional; `add` measures it.
    let (shown, messages, _) = order_free(&[
        "def show(n: Int): Int { n }",
        "def add(n: Int): Int { n + 1 }",
        "def run(n: Int): Int { add(show(n)) }",
        "x = run(lazy 1)",
    ]);
    assert!(messages.is_empty(), "{messages:?}");
    assert!(shown.contains("x: Int"), "{shown}");
    // The callee written below its caller, and above it.
    let used = "def pass(n: Int): Int { n }";
    let client = "def use(n: Int): Int { pass(n) }";
    let a = order_free(&[used, client, "x = use(lazy 2)"]);
    let b = order_free(&[client, used, "x = use(lazy 2)"]);
    assert_eq!(a, b);
}

#[test]
fn inferred_results_do_not_depend_on_order() {
    // An impure function's result is inferred from its body.
    let used = "def get(n: Int) -> Int { if n == 0 { lazy 0 } else { n } }";
    let client = "def twice(n: Int) -> Int { get(n) + get(n) }";
    let a = order_free(&[used, client, "x = twice(1)"]);
    let b = order_free(&[client, used, "x = twice(1)"]);
    assert_eq!(a, b);
    assert!(a.1.is_empty(), "{:?}", a.1);
    // A pure caller of a pure callee that leaves its parameter
    // provisional: outside a group it sees the callee resolved, so
    // the call needs no `await` and is no error, in either order.
    let used = "def a(b: Int): Int { b }";
    let pure = "def total(): Int { a(1) + 1 }";
    let a = order_free(&[used, pure, "x = total()"]);
    let b = order_free(&[pure, used, "x = total()"]);
    assert_eq!(a, b);
    assert!(a.1.is_empty(), "{:?}", a.1);
}

#[test]
fn narrowed_callbacks_do_not_depend_on_order() {
    // `apply`'s callback parameter is narrowed by its body, so `run`
    // may pass it the pure `double`, wherever the three are written.
    let host = "def apply(f: fn(Int) -> Int) -> Int { f(1) }";
    let user = "def run() -> Int { apply(double) }";
    let double = "fn double(x: Int): Int { x * 2 }";
    let a = order_free(&[double, host, user, "x = run()"]);
    let b = order_free(&[user, host, double, "x = run()"]);
    assert_eq!(a, b);
    assert!(a.1.is_empty(), "{:?}", a.1);
}

#[test]
fn groups_and_their_neighbours_do_not_depend_on_order() {
    // Not a group: `a`'s parameter shadows `b`.
    order_free(&["def a(b: Int) -> Int { lazy b }", "def b() -> Int { a(1) }"]);
    // A group: the conservative view, the same from either member.
    let (shown, _, _) = order_free(&[
        "def a(n: Int) -> Int { if n == 0 { lazy 5 } else { b(n - 1) } }",
        "def b(n: Int) -> Int { a(n) }",
    ]);
    let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
    assert_eq!(shown, format!("{{a: {either}, b: {either}}}"));
    // A nested spread that defines the function's name: the mention is
    // counted (the documented over-approximation), so `f`'s body is
    // checked before the item, and the item reads the spread's field.
    let (shown, messages, _) =
        order_free(&["let r = {f = 2}\nx = { ...r; y = f }", "fn f(): Int { 1 }"]);
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(shown, "{x: {f: Int, y: Int}}");
}

#[test]
fn dotted_defs_do_not_depend_on_order() {
    // The summary in one file, and mutual recursion by path.
    let (shown, messages, _) = order_free(&[
        "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }",
        "def dev.peios.count(n: Int): Int { if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) } }",
        "def dev.peios.name(): String { \"world\" }",
        "dev.peios.version = \"1\"",
        "greeting = dev.peios.hello()",
        "all = dev.peios",
        "def even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
        "def dev.odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }",
    ]);
    assert!(messages.is_empty(), "{messages:?}");
    assert!(shown.contains("greeting: String"), "{shown}");
    // Inferred parameters, inferred results and narrowed callbacks, a
    // prefix read whole among the readers.
    let (shown, messages, _) = order_free(&[
        "def p.show(n: Int): Int { n }",
        "def p.add(n: Int): Int { n + 1 }",
        "def p.run(n: Int): Int { p.add(p.show(n)) }",
        "x = p.run(lazy 1)",
        "def q.get(n: Int) -> Int { if n == 0 { lazy 0 } else { n } }",
        "def q.twice(n: Int) -> Int { q.get(n) + q.get(n) }",
        "y = q.twice(1)",
        "whole = q",
        "def r.apply(f: fn(Int) -> Int) -> Int { f(1) }",
        "def r.run() -> Int { r.apply(r.double) }",
        "def r.double(x: Int): Int { x * 2 }",
        "z = r.run()",
    ]);
    assert!(messages.is_empty(), "{messages:?}");
    assert!(shown.contains("x: Int"), "{shown}");
    // A group by path: the conservative view, from either member.
    let (shown, _, _) = order_free(&[
        "def g.a(n: Int) -> Int { if n == 0 { lazy 5 } else { g.b(n - 1) } }",
        "def g.b(n: Int) -> Int { g.a(n) }",
    ]);
    let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
    assert_eq!(shown, format!("{{g: {{a: {either}, b: {either}}}}}"));
}

#[test]
fn a_dotted_def_s_closure_is_stored_in_its_leaf_s_cell() {
    // Made at the start, stored in its leaf's cell, which the prefix's
    // record loads; shown by its path in the dump.
    let text = "def dev.peios.count(n: Int): Int {\n\
                    \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
                    }\nall = dev.peios";
    assert_eq!(
        dump(text),
        "(block @0..108\n\
             \x20 (store count#3 @0..92\n\
             \x20   (make-closure #0 @0..92))\n\
             \x20 (store peios#2 @0..92\n\
             \x20   (make-record [count] @0..92\n\
             \x20     (load count#3 @0..92)))\n\
             \x20 (store dev#0 @0..92\n\
             \x20   (make-record [peios] @0..92\n\
             \x20     (load peios#2 @0..92)))\n\
             \x20 (store all#1 @93..108\n\
             \x20   (load peios#2 @99..108))\n\
             \x20 (make-record [dev all] @0..108\n\
             \x20   (load dev#0 @4..7)\n\
             \x20   (load all#1 @93..96)))\n\
             fn #0 dev.peios.count (frame 1, params 1, captures 0, height 8)\n\
             \x20 (block @33..92\n\
             \x20   (if @39..90\n\
             \x20     (int-cmp == @42..48\n\
             \x20       (load n#0 @42..43)\n\
             \x20       (int 0 @47..48))\n\
             \x20     (block @49..54\n\
             \x20       (int 0 @51..52))\n\
             \x20     (block @60..90\n\
             \x20       (int-add @62..88\n\
             \x20         (int 1 @62..63)\n\
             \x20         (call @66..88\n\
             \x20           (fn top count#3 @66..81)\n\
             \x20           (arg 0 @82..87\n\
             \x20             (int-sub @82..87\n\
             \x20               (load n#0 @82..83)\n\
             \x20               (int 1 @86..87))))))))"
    );
    // Copying a variable, it is made after it is bound.
    let made = dump("let k = 1\ndef a.f(): Int { k }\nx = a.f()");
    assert!(
        made.starts_with(
            "(block @0..40\n\
                 \x20 (store k#3 @0..9\n\
                 \x20   (int 1 @8..9))\n\
                 \x20 (store f#2 @10..30\n\
                 \x20   (make-closure #0 @10..30\n\
                 \x20     (load k#3 @10..30)))\n"
        ),
        "{made}"
    );
}

#[test]
fn an_iteration_is_forkable_unless_it_assigns_outside_or_leaves() {
    let forkable = |text: &str| lower_text(text).forkable_fors();
    assert_eq!(forkable("let xs = [1, 2]\n= for x in xs { x + 1 }"), [true]);
    // An accumulator: each iteration reads what the one before wrote.
    assert_eq!(
        forkable("let mut t = 0\nfor x in [1, 2] { t := t + x }\n= t"),
        [false]
    );
    // A variable declared in the body is the iteration's own.
    assert_eq!(
        forkable("= for x in [1, 2] { let mut y = x\n y := y + 1\n = y }"),
        [true]
    );
    // A `break` ends the loop: later iterations may not run.
    assert_eq!(
        forkable("= for x in [1, 2] { if x > 1 { break }\n = x }"),
        [false]
    );
    // A `continue` ends only its own iteration.
    assert_eq!(
        forkable("= for x in [1, 2] { if x > 1 { continue }\n = x }"),
        [true]
    );
    // An inner loop's `break` leaves only the inner loop.
    assert_eq!(
        forkable("= for x in [1, 2] { = for y in [3] { if y > x { break }\n = y } }"),
        [false, true]
    );
    // A `return` leaves every loop it is in.
    assert_eq!(
        forkable(
            "fn first(xs: List<Int>) -> Int { for x in xs { if x > 1 { return x } }\n = 0 }\n= first([1])"
        ),
        [false]
    );
    // A function inside the body assigns only its own variables.
    assert_eq!(
        forkable(
            "= for x in [1, 2] { let f = fn(): Int { let mut z = x\n z := z + 1\n = z }\n = f() }"
        ),
        [true]
    );
}
