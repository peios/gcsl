//! Tests of the checker: record blocks and types, field access, dotted fields
//! and the dotted `let`, and equality.

use super::*;

#[test]
fn record_blocks_have_record_types() {
    assert_eq!(shown_result("{ x = 1; y = \"a\" }"), "{x: Int, y: String}");
    assert_eq!(file_type("x = 1\ny = \"a\""), "{x: Int, y: String}");
    assert_eq!(shown_result("{ b = 1; a = 2.5 }"), "{a: Float, b: Int}");
    assert_eq!(
        shown_result("{ a = { n = 1.5 }; b = {} }"),
        "{a: {n: Float}, b: {}}"
    );
    assert_eq!(shown_result("{ \"a b\" = 1 }"), "{\"a b\": Int}");
    // The same fields in another order are the same type.
    assert_eq!(
        result_type("{ x = 1; y = 2 }"),
        result_type("{ y = 2; x = 1 }")
    );
    // A field's untyped initialiser settles to Int, as a `let`'s does.
    assert_eq!(all_types("{ x = 1 }")[0], TyId::INT);
}

#[test]
fn fields_are_in_scope_like_variables() {
    // Visible to later items of the block.
    assert_eq!(shown_result("{ x = 1; y = x + 1 }"), "{x: Int, y: Int}");
    // `let`s are private to the block, not fields.
    assert_eq!(
        shown_result("{ x = 1.5; let t = x * 2.0; y = t }"),
        "{x: Float, y: Float}"
    );
    // A field shadows an earlier binding, and a later `let` shadows a
    // field without changing it.
    assert_eq!(
        shown_result("let x = \"s\"\n{ x = 1; y = x + 1 }"),
        "{x: Int, y: Int}"
    );
    assert_eq!(
        shown_result("{ x = 1; let x = \"s\"; y = x }"),
        "{x: Int, y: String}"
    );
    // Not visible before its item, nor after its block.
    assert_eq!(messages("{ y = x; x = 1 }"), vec!["undefined variable `x`"]);
    assert_eq!(
        messages("let r = { x = 1 }\nx"),
        vec!["undefined variable `x`"]
    );
    // Statements of type `{}` are allowed, the last one included.
    assert_eq!(shown_result("{ x = 1; {}; if true {} }"), "{x: Int}");
}

#[test]
fn a_record_block_has_no_result() {
    let diagnostic = &diagnostics("{ x = 1; = 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "a block with fields cannot have a result"
    );
    assert_eq!(diagnostic.span, Span::new(9, 10));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 7), "first field declared here")]
    );
    assert_eq!(
        diagnostic.notes,
        vec![
            "a block is either a record of its fields or a single value; bind the value \
                 with `let` or make it a field"
        ]
    );
    // Wherever the result is, it is this error, not the ordering one.
    assert_eq!(
        labels(&diagnostics("{ = 2; x = 1 }")[0]),
        vec![(Span::new(7, 12), "first field declared here")]
    );
    assert_eq!(
        messages("{ = 2; x = 1 }"),
        vec!["a block with fields cannot have a result"]
    );
    // A final expression is a statement, so a value there is unused.
    let diagnostic = &diagnostics("{ x = 1; x + 1 }")[0];
    assert_eq!(diagnostic.message, "unused value of type Int");
    assert_eq!(diagnostic.span, Span::new(9, 14));
    assert_eq!(messages("x = 1\n2.5"), vec!["unused value of type Float"]);
}

#[test]
fn rebinding_a_field_is_an_error() {
    let diagnostic = &diagnostics("{ x = 1; x := 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `x`: it is a field, not a variable"
    );
    assert_eq!(diagnostic.span, Span::new(9, 10));
    assert_eq!(labels(diagnostic), vec![(Span::new(2, 7), "declared here")]);
    assert_eq!(
        diagnostic.notes,
        vec!["rebind with `x = ...` is not allowed either: a field is defined once"]
    );
    // A field of an enclosing block, too.
    assert_eq!(
        messages("{ x = 1; y = { x := 2 } }"),
        vec!["cannot rebind `x`: it is a field, not a variable"]
    );
    // A field declared with a quoted name is the same name. At the top
    // level, the field is a field of the file, which `:=` cannot name.
    assert_eq!(
        messages("\"x\" = 1\nx := 2"),
        vec!["`x` is a field of this file"]
    );
    // The rejected rebind still declares a variable, so uses do not
    // cascade.
    assert_eq!(
        messages("{ x = 1; x := 2.5; y = x * 1.5 }"),
        vec!["cannot rebind `x`: it is a field, not a variable"]
    );
}

#[test]
fn a_field_defined_twice_is_an_error() {
    let diagnostic = &diagnostics("{ x = 1; x = 2 }")[0];
    assert_eq!(diagnostic.message, "field `x` is defined twice");
    assert_eq!(diagnostic.span, Span::new(9, 10));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 3), "first defined here")]
    );
    // Each repeat is reported against the first.
    let repeats = diagnostics("{ x = 1; y = 2; x = 3; x = 4 }");
    assert_eq!(repeats.len(), 2);
    assert_eq!(repeats[0].span, Span::new(16, 17));
    assert_eq!(repeats[1].span, Span::new(23, 24));
    for repeat in &repeats {
        assert_eq!(
            labels(repeat),
            vec![(Span::new(2, 3), "first defined here")]
        );
    }
    // A quoted name that is the same name.
    assert_eq!(
        errors("{ x = 1; \"x\" = 2 }"),
        vec![("field `x` is defined twice".to_owned(), Span::new(9, 12))]
    );
    assert_eq!(
        messages("{ \"a b\" = 1; \"a b\" = 2 }"),
        vec!["field `\"a b\"` is defined twice"]
    );
    // The record is an error, so it does not cascade.
    assert_eq!(
        messages("let r = { x = 1; x = 2 }\nr.x + 1.5\nr.y"),
        vec!["field `x` is defined twice"]
    );
}

#[test]
fn field_access() {
    assert_eq!(
        result_type("let p = { x = 1; y = \"a\" }\np.y"),
        TyId::STRING
    );
    assert_eq!(
        result_type("let p = { x = 1; y = \"a\" }\np.x + 1"),
        TyId::INT
    );
    assert_eq!(result_type("{ a = { n = 1.5 } }.a.n"), TyId::FLOAT);
    assert_eq!(result_type("{ \"a b\" = true }.\"a b\""), TyId::BOOL);
    assert_eq!(result_type("-{ x = 2 }.x"), TyId::INT);
    assert_eq!(shown_result("{ a = { n = 1 } }.a"), "{n: Int}");
}

#[test]
fn field_access_errors() {
    let diagnostic = &diagnostics("let p = { x = 1; y = 2 }\np.z")[0];
    assert_eq!(diagnostic.message, "no field `z` in `{x: Int, y: Int}`");
    assert_eq!(diagnostic.span, Span::new(26, 28));
    assert!(diagnostic.labels.is_empty());
    assert_eq!(diagnostic.notes, vec!["the fields are `x` and `y`"]);
    let diagnostic = &diagnostics("{}.x")[0];
    assert_eq!(diagnostic.message, "no field `x` in `{}`");
    assert_eq!(diagnostic.span, Span::new(2, 4));
    assert_eq!(diagnostic.notes, vec!["the record has no fields"]);
    assert_eq!(
        diagnostics("{ x = 1 }.y")[0].notes,
        vec!["the only field is `x`"]
    );
    assert_eq!(
        diagnostics("{ c = 1; a = 2; \"b b\" = 3 }.z")[0].notes,
        vec!["the fields are `a`, `\"b b\"` and `c`"]
    );
    assert_eq!(
        messages("{ x = 1 }.\"a b\""),
        vec!["no field `\"a b\"` in `{x: Int}`"]
    );
    // A value that is not a record.
    let diagnostic = &diagnostics("1.x")[0];
    assert_eq!(diagnostic.message, "cannot access field `x` of Int");
    assert_eq!(diagnostic.span, Span::new(1, 3));
    assert_eq!(labels(diagnostic), vec![(Span::new(0, 1), "this is Int")]);
    let diagnostic = &diagnostics("let s = \"s\"\ns . x")[0];
    assert_eq!(diagnostic.message, "cannot access field `x` of String");
    assert_eq!(diagnostic.span, Span::new(13, 17));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(12, 13), "this is String")]
    );
    // An erroneous record is not reported again.
    assert_eq!(messages("q.x"), vec!["undefined variable `q`"]);
    assert_eq!(messages("q.x.y + 1"), vec!["undefined variable `q`"]);
    assert_eq!(
        messages("{ x = 1 }.z.w"),
        vec!["no field `z` in `{x: Int}`"]
    );
}

#[test]
fn a_record_block_is_checked_against_its_annotation() {
    // Literals adapt to the expected field types, nested blocks too,
    // through parentheses, and in ascriptions.
    assert_eq!(all_types("let p: {x: Float} = { x = 1 }")[0], TyId::FLOAT);
    assert_eq!(
        all_types("let p: {a: {n: Float}} = { a = { n = 1 } }")[0],
        TyId::FLOAT
    );
    assert_eq!(
        all_types("let p: {x: Float} = (({ x = 1 }))")[0],
        TyId::FLOAT
    );
    assert_eq!(all_types("{ x = 1 } : {x: Float}")[0], TyId::FLOAT);
    assert_eq!(
        shown_result("let p: {x: Float} = { x = -(1 + 2) }\np"),
        "{x: Float}"
    );
    assert_eq!(shown_result("{ x = 1 } : {x: Float}"), "{x: Float}");
    // Each parenthesis has the block's type.
    let (ast, types, output) = check_text("let p: {x: Float} = ({ x = 1 })");
    let checked = output.unwrap().checked;
    let ItemKind::Let { value, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(
        types
            .display(checked.types.get(value), ast.names())
            .to_string(),
        "{x: Float}"
    );
    // A field can use earlier fields at their expected types.
    assert_eq!(
        shown_result("let p: {x: Float, y: Float} = { x = 1; y = x + 1 }\np"),
        "{x: Float, y: Float}"
    );
    // A mismatch inside is reported where it is.
    let diagnostic = &diagnostics("let p: {x: Int} = { x = \"s\" }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(24, 27));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 15), "expected because of this annotation")]
    );
    // A block without fields, or a record block against a type that is
    // not a record, is checked as a whole.
    assert_eq!(
        diagnostics("let p: {x: Int} = { 1 }")[0].label.as_deref(),
        Some("expected {x: Int}, found Int")
    );
    assert_eq!(
        diagnostics("let p: Int = { x = 1 }")[0].label.as_deref(),
        Some("expected Int, found {x: Int}")
    );
}

#[test]
fn a_missing_field_is_an_error() {
    let diagnostic = &diagnostics("let p: {x: Int, y: Int} = { x = 1 }")[0];
    assert_eq!(diagnostic.message, "missing field `y` in record");
    assert_eq!(diagnostic.span, Span::new(34, 35));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 23), "expected because of this annotation")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["the type `{x: Int, y: Int}` needs it"]
    );
    // Each missing field, in alphabetical order.
    assert_eq!(
        messages("{ c = 1 } : {b: Int, c: Int, a: Int}"),
        vec!["missing field `a` in record", "missing field `b` in record"]
    );
    assert_eq!(
        labels(&diagnostics("{ c = 1 } : {b: Int, c: Int}")[0]),
        vec![(Span::new(12, 28), "ascribed here")]
    );
    // In a nested block, at its own closing brace; the warning about
    // its extra field comes back with it, in source order.
    assert_eq!(
        errors("let p: {a: {n: Int}} = { a = { m = 1 } }"),
        vec![
            (
                "field `m` is not in `{n: Int}`".to_owned(),
                Span::new(31, 32)
            ),
            ("missing field `n` in record".to_owned(), Span::new(37, 38))
        ]
    );
    // `{}` has no fields, so it is not a record block: it is checked as
    // a whole.
    assert_eq!(
        diagnostics("let p: {a: {n: Int}} = { a = {} }")[0]
            .label
            .as_deref(),
        Some("expected {n: Int}, found {}")
    );
    // It does not cascade.
    assert_eq!(
        messages("let p: {x: Int, y: Int} = { x = 1 }\np.y + 1.5"),
        vec!["missing field `y` in record"]
    );
}

#[test]
fn an_extra_field_is_a_warning() {
    let text = "let p: {x: Int} = { x = 1; z = 2 }\np";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    let warning = &found[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.message, "field `z` is not in `{x: Int}`");
    assert_eq!(warning.span, Span::new(27, 28));
    assert_eq!(
        labels(warning),
        vec![(Span::new(7, 15), "expected because of this annotation")]
    );
    assert_eq!(
        warning.notes,
        vec!["it is allowed, but nothing with this type can see it"]
    );
    // The check succeeds, and the variable has the annotated type.
    assert_eq!(shown_result(text), "{x: Int}");
    assert_eq!(
        messages("let p: {x: Int} = { x = 1; z = 2 }\np.z"),
        vec![
            "field `z` is not in `{x: Int}`",
            "no field `z` in `{x: Int}`"
        ]
    );
    // In an ascription, against `{}`, and in a nested block.
    assert_eq!(
        labels(&warnings("{ x = 1; z = 2 } : {x: Int}")[0]),
        vec![(Span::new(19, 27), "ascribed here")]
    );
    assert_eq!(
        warnings("let e: {} = { x = 1 }")[0].message,
        "field `x` is not in `{}`"
    );
    assert_eq!(
        warnings("let p: {a: {n: Int}} = { a = { n = 1; m = 2 } }")[0].message,
        "field `m` is not in `{n: Int}`"
    );
    // With errors, the warnings come back with them in source order.
    let diagnostics = diagnostics("let p: {x: Int} = { x = 1; z = 2 }\nq");
    let shown: Vec<(Severity, &str)> = diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.severity, diagnostic.message.as_str()))
        .collect();
    assert_eq!(
        shown,
        vec![
            (Severity::Warning, "field `z` is not in `{x: Int}`"),
            (Severity::Error, "undefined variable `q`")
        ]
    );
}

#[test]
fn subtyping_through_a_variable_warns_about_nothing() {
    let text = "let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\np";
    assert!(warnings(text).is_empty());
    assert_eq!(shown_result(text), "{x: Int}");
    let text = "let q = { a = { n = 1; m = 2 }; b = 1 }\nlet p: {a: {n: Int}} = q\np";
    assert!(warnings(text).is_empty());
    assert_eq!(shown_result(text), "{a: {n: Int}}");
    assert!(warnings("let q = { x = 1 }\nq : {}").is_empty());
    // A block in parentheses inside another expression is not checked
    // field by field.
    assert!(
        warnings("let p: {x: Int} = if true { { x = 1; y = 2 } } else { { x = 2 } }").is_empty()
    );
    // A record that lacks a field is not a subtype.
    let diagnostic = &diagnostics("let q = { y = 1 }\nlet p: {x: Int} = q")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected {x: Int}, found {y: Int}")
    );
    assert_eq!(
        diagnostics("let q = { x = 1 }\nlet p: {x: Float} = q")[0]
            .label
            .as_deref(),
        Some("expected {x: Float}, found {x: Int}")
    );
}

#[test]
fn if_joins_record_branches() {
    assert_eq!(
        shown_result("if true { { x = 1; y = 2 } } else { { x = 3 } }"),
        "{x: Int}"
    );
    // No shared fields: the empty record, which is still a record.
    assert_eq!(
        shown_result("if true { { x = 1 } } else { { y = 1 } }"),
        "{}"
    );
    assert_eq!(shown_result("if true { {} } else { { x = 1 } }"), "{}");
    // A shared field whose types have no join is kept, with their
    // union.
    assert_eq!(
        shown_result("if true { { x = 1; y = 2 } } else { { x = 1.5; y = 3 } }"),
        "{x: Int | Float, y: Int}"
    );
    // Nested.
    assert_eq!(
        shown_result("if true { { a = { n = 1; m = 2 }; b = 1 } } else { { a = { n = 3 } } }"),
        "{a: {n: Int}}"
    );
    // A record branch and a non-record branch make a union, and a
    // warning.
    let diagnostic = &warnings("if true { { x = 1 } } else { 1 }")[0];
    assert_eq!(
        diagnostic.message,
        "the branches of this `if` have different types"
    );
    assert_eq!(diagnostic.span, Span::new(29, 30));
    assert_eq!(diagnostic.label.as_deref(), Some("this is Int"));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(10, 19), "this is {x: Int}")]
    );
}

#[test]
fn record_equality_needs_identical_types() {
    assert_eq!(result_type("{ x = 1 } == { x = 2 }"), TyId::BOOL);
    assert_eq!(
        result_type("{ x = 1; y = 2 } != { y = 1; x = 2 }"),
        TyId::BOOL
    );
    assert_eq!(result_type("{} == {}"), TyId::BOOL);
    let diagnostic = &diagnostics("{ x = 1 } == { x = 1; y = 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot compare `{x: Int}` and `{x: Int, y: Int}`"
    );
    assert_eq!(diagnostic.span, Span::new(10, 12));
    assert_eq!(
        labels(diagnostic),
        vec![
            (Span::new(0, 9), "{x: Int}"),
            (Span::new(13, 29), "{x: Int, y: Int}")
        ]
    );
    // Untyped literals in record blocks settle to Int.
    assert_eq!(
        messages("{ x = 1 } == { x = 1.5 }"),
        vec!["cannot compare `{x: Int}` and `{x: Float}`"]
    );
    assert_eq!(
        messages("{} != { x = 1 }"),
        vec!["cannot compare `{}` and `{x: Int}`"]
    );
    assert_eq!(
        messages("{ x = 1 } == 1"),
        vec!["cannot compare `{x: Int}` and Int"]
    );
}

#[test]
fn records_cannot_be_ordered() {
    let diagnostic = &diagnostics("{ x = 1 } < { x = 2 }")[0];
    assert_eq!(diagnostic.message, "cannot order `{x: Int}`");
    assert_eq!(
        labels(diagnostic),
        vec![
            (Span::new(0, 9), "{x: Int}"),
            (Span::new(12, 21), "{x: Int}")
        ]
    );
    assert_eq!(messages("{} >= {}"), vec!["cannot order `{}`"]);
    assert_eq!(messages("1 < { x = 1 }"), vec!["cannot order `{x: Int}`"]);
}

#[test]
fn record_type_annotations() {
    assert_eq!(
        shown_result(
            "let p: { \"a b\": String, nested: {n: Int} } = { \"a b\" = \"s\"; nested = { n = 1 } }\np"
        ),
        "{\"a b\": String, nested: {n: Int}}"
    );
    // A name that appears twice in a type.
    assert_eq!(
        errors("1 : {x: Int, x: Int}"),
        vec![(
            "field `x` appears twice in this type".to_owned(),
            Span::new(13, 14)
        )]
    );
    assert_eq!(
        messages("let p: {a: Int, \"a\": Int, a: Int} = { a = 1 }\np.a + 1.5"),
        vec![
            "field `a` appears twice in this type",
            "field `a` appears twice in this type"
        ]
    );
}

#[test]
fn dotted_fields_merge_into_one_record() {
    assert_eq!(
        file_type("server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true"),
        "{server: {host: String, port: Int, tls: {enabled: Bool}}}"
    );
    assert_eq!(
        shown_result("{ a.b.c = 1; a.b.d = 2.5; a.e = \"s\"; f = 1 }"),
        "{a: {b: {c: Int, d: Float}, e: String}, f: Int}"
    );
    // Quoted segments are names like any other.
    assert_eq!(
        shown_result("{ \"log level\".format = \"json\"; \"log level\".\"to file\" = true }"),
        "{\"log level\": {format: String, \"to file\": Bool}}"
    );
    // Interleaved with other fields and with `let`s.
    assert_eq!(
        shown_result("{ a.x = 1; b = 2; let t = 3; a.y = t }"),
        "{a: {x: Int, y: Int}, b: Int}"
    );
    // The field is one binding, in one slot, whatever extends it.
    let ast = parse_text("a.x = 1\nb = 2\na.y = 3\na.z.w = 4");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    let slots: Vec<Slot> = items.iter().map(|&id| resolution.item_slot(id)).collect();
    assert_eq!(slots, vec![Slot(0), Slot(1), Slot(0), Slot(0)]);
    // And each node below the roots has a cell of its own: `a.x`,
    // `a.y`, `a.z` and `a.z.w`, after the roots.
    assert_eq!(resolution.frame_size(), 6);
    // The first definition builds from nothing; each later one updates
    // the field's slot, through as many segments as exist.
    let paths: Vec<PathResolution> = items.iter().map(|&id| resolution.item_path(id)).collect();
    assert_eq!(
        paths,
        vec![
            PathResolution::NONE,
            PathResolution::NONE,
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 1,
                awaits: PathResolution::NOT_AWAITED
            },
            PathResolution {
                source: VarRef::Local(Slot(0)),
                existing: 1,
                awaits: PathResolution::NOT_AWAITED
            },
        ]
    );
    let ast = parse_text("a.b.c = 1\na.b.d = 2");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    assert_eq!(
        resolution.item_path(ast.items(ast.root())[1]),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 2,
            awaits: PathResolution::NOT_AWAITED
        }
    );
}

#[test]
fn the_type_of_an_open_field_grows_item_by_item() {
    assert_eq!(
        shown_result("{ a.x = 1; let p = a; a.y = \"s\"; let q = a; r = p; s = q }"),
        "{a: {x: Int, y: String}, r: {x: Int}, s: {x: Int, y: String}}"
    );
    // A later definition sees the field as built so far.
    assert_eq!(
        shown_result("{ a.x = 1; a.y = a.x + 1 }"),
        "{a: {x: Int, y: Int}}"
    );
    assert_eq!(
        messages("{ a.x = 1; a.y = a.z }"),
        vec!["no field `z` in `{x: Int}`"]
    );
    // The first definition does not see its own field.
    assert_eq!(messages("{ a.x = a }"), vec!["undefined variable `a`"]);
    // A later `let` shadows the field without changing it.
    assert_eq!(
        shown_result("{ a.x = 1; let a = \"s\"; a.y = a }"),
        "{a: {x: Int, y: String}}"
    );
}

#[test]
fn a_dotted_path_under_a_whole_field_is_an_error() {
    let diagnostic = &diagnostics("{ a = 1; a.b = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a` is already defined");
    assert_eq!(diagnostic.span, Span::new(9, 12));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 3), "defined whole here")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["a field defined as a whole cannot be extended with dotted paths"]
    );
    // Whatever the whole field's type, record or not.
    assert_eq!(
        messages("{ a = { b = 1 }; a.c = 2 }"),
        vec!["field `a` is already defined"]
    );
    // A segment defined whole as a record, by a path's last segment.
    let diagnostic = &diagnostics("{ a.b = { x = 1 }; a.b.y = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a.b` is already defined");
    assert_eq!(diagnostic.span, Span::new(19, 24));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "defined whole here")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["a field defined as a whole cannot be extended with dotted paths"]
    );
    assert_eq!(
        messages("let r = { x = 1 }\n{ a.b = r; a.b.c.d = 1 }"),
        vec!["field `a.b` is already defined"]
    );
    // The rejected path declares nothing, so nothing cascades, but its
    // value is still checked.
    assert_eq!(
        messages("{ a = 1; a.b = 2; c = a + 1 }"),
        vec!["field `a` is already defined"]
    );
    assert_eq!(
        messages("{ a = 1; a.b = 1.0 // 2 }"),
        vec!["field `a` is already defined", "cannot floor-divide Float"]
    );
    // The block's record is erroneous, so nothing outside cascades.
    assert_eq!(
        messages("let r = { a = 1; a.b = 2 }\nr.a + 1.5\nr.q"),
        vec!["field `a` is already defined"]
    );
    assert_eq!(
        messages("let r = { a.b = 1; a.b.c = 2 }\nr.a.b + 1.5"),
        vec!["cannot define `a.b.c`: `a.b` is Int, not a record"]
    );
}

#[test]
fn a_whole_field_after_a_dotted_one_is_defined_twice() {
    let diagnostic = &diagnostics("{ a.b = 1; a = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a` is defined twice");
    assert_eq!(diagnostic.span, Span::new(11, 12));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 3), "first defined here")]
    );
    // Later dotted paths still extend the first, dotted, field.
    assert_eq!(
        messages("{ a.b = 1; a = 2; a.c = 3 }"),
        vec!["field `a` is defined twice"]
    );
}

#[test]
fn a_segment_that_is_not_a_record_cannot_hold_fields() {
    let diagnostic = &diagnostics("{ a.b = 1; a.b.c = 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot define `a.b.c`: `a.b` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(11, 16));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "`a.b` defined here")]
    );
    assert!(diagnostic.notes.is_empty());
    // Deeper, with another type, and quoted.
    assert_eq!(
        messages("{ a.b.c = \"s\"; a.b.c.d.e = 1 }"),
        vec!["cannot define `a.b.c.d.e`: `a.b.c` is String, not a record"]
    );
    assert_eq!(
        messages("{ a.\"b c\" = true; a.\"b c\".d = 1 }"),
        vec!["cannot define `a.\"b c\".d`: `a.\"b c\"` is Bool, not a record"]
    );
    // An erroneous segment has been reported already.
    assert_eq!(
        messages("{ a.b = 1.0 // 2; a.b.c = 1 }"),
        vec!["cannot floor-divide Float"]
    );
}

#[test]
fn a_dotted_path_defined_twice_is_an_error() {
    let diagnostic = &diagnostics("{ a.b = 1; a.b = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a.b` is defined twice");
    assert_eq!(diagnostic.span, Span::new(11, 14));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "first defined here")]
    );
    // A record a longer path created is defined too, where that path
    // wrote it.
    let diagnostic = &diagnostics("{ a.b.c = 1; a.b = 2 }")[0];
    assert_eq!(diagnostic.message, "field `a.b` is defined twice");
    assert_eq!(diagnostic.span, Span::new(13, 16));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 5), "first defined here")]
    );
    // A quoted name is the same name.
    assert_eq!(
        messages("{ \"a b\".c = 1; \"a b\".\"c\" = 2 }"),
        vec!["field `\"a b\".c` is defined twice"]
    );
    // The record type does not cascade.
    assert_eq!(
        messages("let r = { a.b = 1; a.b = 2 }\nr.a.b + 1.5"),
        vec!["field `a.b` is defined twice"]
    );
}

#[test]
fn dotted_fields_are_checked_against_the_expected_type() {
    // The leaf is checked against the type at its full path.
    assert_eq!(
        all_types("let p: {a: {b: Float}} = { a.b = 1 }")[0],
        TyId::FLOAT
    );
    assert_eq!(
        shown_result("let p: {a: {b: Float, c: {d: Float}}} = { a.b = 1; a.c.d = 2 }\np"),
        "{a: {b: Float, c: {d: Float}}}"
    );
    assert!(warnings("let p: {a: {b: Int}} = { a.b = 1 }\np").is_empty());
    // Record blocks at the leaf are checked field by field.
    assert_eq!(
        all_types("let p: {a: {b: {n: Float}}} = { a.b = { n = 1 } }")[0],
        TyId::FLOAT
    );
    let diagnostic = &diagnostics("let p: {a: {b: Int}} = { a.b = \"s\" }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(31, 34));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found String")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 20), "expected because of this annotation")]
    );
    // In an ascription too.
    assert_eq!(all_types("{ a.b = 1 } : {a: {b: Float}}")[0], TyId::FLOAT);
}

#[test]
fn a_missing_nested_field_of_a_dotted_field_is_an_error() {
    let text = "let p: {a: {b: Int, c: Int}} = { a.b = 1 }";
    let diagnostic = &diagnostics(text)[0];
    assert_eq!(diagnostic.message, "missing field `a.c` in record");
    assert_eq!(diagnostic.span, Span::new(41, 42));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 28), "expected because of this annotation")]
    );
    assert_eq!(
        diagnostic.notes,
        vec!["the type `{b: Int, c: Int}` needs it"]
    );
    // At every level, each in alphabetical order, and alongside the
    // block's own missing fields.
    assert_eq!(
        messages("let p: {a: {b: {c: Int, d: Int, e: Int}}, z: Int} = { a.b.d = 1 }"),
        vec![
            "missing field `z` in record",
            "missing field `a.b.c` in record",
            "missing field `a.b.e` in record"
        ]
    );
    // It does not cascade.
    assert_eq!(
        messages("let p: {a: {b: Int, c: Int}} = { a.b = 1 }\np.a.c + 1.5"),
        vec!["missing field `a.c` in record"]
    );
}

#[test]
fn a_dotted_field_the_expected_type_lacks_is_a_warning() {
    let text = "let p: {a: {b: Int}} = { a.b = 1; a.c = 2 }\np";
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    let warning = &found[0];
    assert_eq!(warning.message, "field `c` is not in `{b: Int}`");
    assert_eq!(warning.span, Span::new(36, 37));
    assert_eq!(
        labels(warning),
        vec![(Span::new(7, 20), "expected because of this annotation")]
    );
    assert_eq!(
        warning.notes,
        vec!["it is allowed, but nothing with this type can see it"]
    );
    assert_eq!(shown_result(text), "{a: {b: Int}}");
    // For the first missing segment only, once per such field.
    let found = warnings("let p: {a: Int} = { a = 1; x.p = 1; x.q.r = 2; y.s.t = 3 }");
    let shown: Vec<(&str, Span)> = found
        .iter()
        .map(|warning| (warning.message.as_str(), warning.span))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("field `x` is not in `{a: Int}`", Span::new(27, 28)),
            ("field `y` is not in `{a: Int}`", Span::new(47, 48))
        ]
    );
    let found = warnings("let p: {a: {b: Int}} = { a.b = 1; a.c.d = 2; a.c.e = 3 }");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `c` is not in `{b: Int}`");
}

#[test]
fn a_dotted_field_where_the_expected_type_is_not_a_record_mismatches() {
    let diagnostic = &diagnostics("let p: {a: Int} = { a.b = 1 }")[0];
    assert_eq!(diagnostic.message, "mismatched types");
    assert_eq!(diagnostic.span, Span::new(20, 21));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("expected Int, found {b: Int}")
    );
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(7, 15), "expected because of this annotation")]
    );
    // Nested, at the segment's first definition, and without warnings
    // about the segments below it.
    let text = "let p: {a: {b: String}} = { a.b.c = 1; a.b.d = 2 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].span, Span::new(28, 31));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected String, found {c: Int, d: Int}")
    );
}

#[test]
fn a_dotted_let_without_a_variable_builds_nested_records() {
    assert_eq!(shown_result("let a.b.c = 1\na"), "{b: {c: Int}}");
    assert_eq!(shown_result("let a.b = \"s\"\na.b"), "String");
    // An annotation applies to the leaf.
    assert_eq!(shown_result("let a.b: Float = 5\na"), "{b: Float}");
    assert_eq!(all_types("let a.b: Float = 5")[0], TyId::FLOAT);
    assert_eq!(
        diagnostics("let a.b: Int = 1.5")[0].label.as_deref(),
        Some("expected Int, found Float")
    );
    let ast = parse_text("let a.b.c = 1");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    assert_eq!(
        resolution.item_path(ast.items(ast.root())[0]),
        PathResolution::NONE
    );
}

#[test]
fn a_dotted_let_updates_a_record_variable() {
    // A field added.
    assert_eq!(
        shown_result("let a = { x = 1 }\nlet a.y = \"s\"\na"),
        "{x: Int, y: String}"
    );
    // Intermediate records created, and updated inside.
    assert_eq!(
        shown_result("let a = { x = 1 }\nlet a.b.c = true\na"),
        "{b: {c: Bool}, x: Int}"
    );
    assert_eq!(
        shown_result("let a = { b = { c = 1; d = 2 } }\nlet a.b.e = 3\na"),
        "{b: {c: Int, d: Int, e: Int}}"
    );
    // A leaf replaced, its type changing.
    assert_eq!(
        shown_result("let a = { b = { c = 1; d = 2 } }\nlet a.b.c = \"s\"\na"),
        "{b: {c: String, d: Int}}"
    );
    assert_eq!(
        shown_result("let a = { b = { c = 1 } }\nlet a.b = 2.5\na"),
        "{b: Float}"
    );
    // Through the static type: a field hidden by subtyping is created
    // anew.
    assert_eq!(
        shown_result("let p: {x: Int} = { x = 1; y = 2 }\nlet p.y.z = 3\np"),
        "{x: Int, y: {z: Int}}"
    );
    // How far into the path the old record reaches, for lowering.
    let ast = parse_text("let a = { b = { c = 1 } }\nlet a.b.c = 2\nlet a.x.y.z = 3");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    assert_eq!(
        resolution.item_path(items[1]),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 2,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    assert_eq!(
        resolution.item_path(items[2]),
        PathResolution {
            source: VarRef::Local(Slot(1)),
            existing: 1,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    // The update is a new binding in a new slot.
    assert_eq!(resolution.item_slot(items[1]), Slot(1));
    assert_eq!(resolution.item_slot(items[2]), Slot(2));
}

#[test]
fn a_dotted_let_leaves_the_old_binding_unchanged() {
    let text = "let a = { x = 1 }\nlet old = a\nlet a.x = \"s\"\nold";
    assert_eq!(shown_result(text), "{x: Int}");
    let text = "let a = { x = 1 }\nlet old = a\nlet a.x = \"s\"\na";
    assert_eq!(shown_result(text), "{x: String}");
    // In an inner block, the outer variable is shadowed only there.
    assert_eq!(
        shown_result("let a = { x = 1 }\n{ let a.y = 2; a }"),
        "{x: Int, y: Int}"
    );
    assert_eq!(
        shown_result("let a = { x = 1 }\n{ let a.y = 2 }\na"),
        "{x: Int}"
    );
}

#[test]
fn a_dotted_let_needs_records_along_its_path() {
    let diagnostic = &diagnostics("let a = 1\nlet a.b = 2")[0];
    assert_eq!(
        diagnostic.message,
        "cannot update `a.b`: `a` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(14, 17));
    assert_eq!(labels(diagnostic), vec![(Span::new(14, 15), "this is Int")]);
    let diagnostic = &diagnostics("let a = { b = 1 }\nlet a.b.c = 2")[0];
    assert_eq!(
        diagnostic.message,
        "cannot update `a.b.c`: `a.b` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(22, 27));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(22, 23), "this is {b: Int}")]
    );
    assert_eq!(
        messages("let a = { b = { c = \"s\" } }\nlet a.b.c.d.e = 2"),
        vec!["cannot update `a.b.c.d.e`: `a.b.c` is String, not a record"]
    );
    // The variable is declared as an error, so nothing cascades; the
    // value is still checked.
    assert_eq!(
        messages("let a = 1\nlet a.b = 2\na.b + 1.5"),
        vec!["cannot update `a.b`: `a` is Int, not a record"]
    );
    assert_eq!(
        messages("let a = true\nlet a.b = 1.0 // 2"),
        vec![
            "cannot update `a.b`: `a` is Bool, not a record",
            "cannot floor-divide Float"
        ]
    );
    // An erroneous variable has been reported already.
    assert_eq!(
        messages("let a = 1.0 // 2\nlet a.b = 1"),
        vec!["cannot floor-divide Float"]
    );
}

#[test]
fn a_dotted_rebind_updates_a_variable_of_its_block() {
    assert_eq!(shown_result("let a = { x = 1 }\na.x := 2\na"), "{x: Int}");
    assert_eq!(
        shown_result("let a = { x = 1; y = { z = \"s\" } }\na.y.z := \"t\"\na"),
        "{x: Int, y: {z: String}}"
    );
    // A new binding, like any `:=`.
    let ast = parse_text("let a = { x = 1 }\na.x := 2");
    let resolution = check(&ast, &mut TypeTable::new()).unwrap().0.resolution;
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[1]), Slot(1));
    assert_eq!(
        resolution.item_path(items[1]),
        PathResolution {
            source: VarRef::Local(Slot(0)),
            existing: 1,
            awaits: PathResolution::NOT_AWAITED
        }
    );
    let text = "let a = { x = 1 }\nlet old = a\na.x := 2\nold";
    assert_eq!(shown_result(text), "{x: Int}");
}

#[test]
fn a_dotted_rebind_follows_the_rebind_rules() {
    let diagnostic = &diagnostics("a.x := 1")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a`: no variable named `a` is declared"
    );
    assert_eq!(diagnostic.span, Span::new(0, 1));
    assert_eq!(diagnostic.notes, vec!["declare it with `let a = ...`"]);
    // Declared anyway, so uses do not cascade.
    assert_eq!(
        messages("a.x := 1\na.x + 1.5"),
        vec![
            "cannot rebind `a`: no variable named `a` is declared",
            "cannot add Int and Float"
        ]
    );
    let diagnostic = &diagnostics("{ a = { x = 1 }; a.x := 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a`: it is a field, not a variable"
    );
    assert_eq!(diagnostic.span, Span::new(17, 18));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(2, 15), "declared here")]
    );
    let diagnostic = &diagnostics("let a = { x = 1 }\n{ a.x := 2 }")[0];
    assert_eq!(
        diagnostic.message,
        "cannot rebind `a` here: it is declared in an enclosing block"
    );
    assert_eq!(diagnostic.span, Span::new(20, 21));
    assert_eq!(
        labels(diagnostic),
        vec![(Span::new(0, 17), "declared here")]
    );
    // And the update rules.
    let diagnostic = &diagnostics("let a = 1\na.b := 2")[0];
    assert_eq!(
        diagnostic.message,
        "cannot update `a.b`: `a` is Int, not a record"
    );
    assert_eq!(diagnostic.span, Span::new(10, 13));
    assert_eq!(labels(diagnostic), vec![(Span::new(10, 11), "this is Int")]);
    assert_eq!(
        messages("let a = { b = 1 }\na.b.c := 2"),
        vec!["cannot update `a.b.c`: `a.b` is Int, not a record"]
    );
}

#[test]
fn total_equality_needs_identical_types() {
    for op in ["===", "!=="] {
        assert_eq!(
            result_type(&format!("{{ x = 1 }} {op} {{ x = 2 }}")),
            TyId::BOOL
        );
        assert_eq!(result_type(&format!("1 {op} 2")), TyId::BOOL);
        assert_eq!(result_type(&format!("\"a\" {op} \"b\"")), TyId::BOOL);
        assert_eq!(result_type(&format!("true {op} false")), TyId::BOOL);
        assert_eq!(result_type(&format!("{{}} {op} {{}}")), TyId::BOOL);
        // Untyped literals adapt as for `==`.
        assert_eq!(
            all_types(&format!("1 {op} 2.5")),
            vec![TyId::FLOAT, TyId::FLOAT, TyId::BOOL]
        );
        let diagnostic = &diagnostics(&format!("{{ x = 1 }} {op} {{ x = 1; y = 2 }}"))[0];
        assert_eq!(
            diagnostic.message,
            "cannot compare `{x: Int}` and `{x: Int, y: Int}`"
        );
        assert_eq!(diagnostic.span, Span::new(10, 13));
        assert_eq!(
            labels(diagnostic),
            vec![
                (Span::new(0, 9), "{x: Int}"),
                (Span::new(14, 30), "{x: Int, y: Int}")
            ]
        );
        assert_eq!(
            messages(&format!("true {op} 1")),
            vec!["cannot compare Bool and Int"]
        );
    }
    // Every pair of types, as for `==`.
    for (lhs, lhs_ty, lhs_shown) in VARIABLES {
        for (rhs, rhs_ty, rhs_shown) in VARIABLES {
            let text = format!("{PRELUDE}{lhs} === {rhs}");
            if lhs_ty == rhs_ty {
                assert_eq!(result_type(&text), TyId::BOOL, "{text}");
            } else {
                assert_eq!(
                    messages(&text),
                    vec![format!("cannot compare {lhs_shown} and {rhs_shown}")],
                    "{text}"
                );
            }
        }
    }
    // A narrowed variable compares with values of its static type.
    assert_eq!(
        result_type("let q: {x: Int} = { x = 1; y = 2 }\nq === { x = 1 }"),
        TyId::BOOL
    );
}
