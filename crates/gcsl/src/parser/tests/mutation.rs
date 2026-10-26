//! Tests of the parser: mutable lets and index paths.

use super::*;

/// Whether each item of the file's body is a `let mut`.
pub(super) fn mutability(text: &str) -> Vec<bool> {
    let ast = parse_text(text).unwrap();
    ast.items(ast.root())
        .iter()
        .map(|&id| matches!(ast.item(id).kind, ItemKind::Let { mutable: true, .. }))
        .collect()
}

/// The message, span and notes of the syntax error in `text`.
pub(super) fn error_with_notes(text: &str) -> (String, Span, Vec<String>) {
    let diagnostic = parse_text(text).unwrap_err();
    (diagnostic.message, diagnostic.span, diagnostic.notes)
}

#[test]
fn mutable_lets() {
    // Both forms, which are `let`s, marked as mutable.
    assert_eq!(
        mutability("let mut x = 1\nlet y = 2\nlet mut z: Float = 3\nz := 4\nfn f(): Int { 1 }"),
        vec![true, false, true, false, false]
    );
    assert_eq!(shape("let mut x = 1"), "let x = 1");
    assert_eq!(shape("let mut x: Int = 1"), "let x: Int = 1");
    // A newline may follow `mut`, as it may follow `let`.
    assert_eq!(mutability("let mut\n  x = 1"), vec![true]);
    assert_eq!(mutability("let\nmut\nx =\n1"), vec![true]);
    assert_eq!(mutability("let mut x:\n  Int = 1"), vec![true]);
    // The item spans `let` through its value, and its name is its path.
    let ast = parse_text("  let mut total = 0").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 19));
    let ItemKind::Let { path, ty, .. } = item.kind else {
        panic!("expected a let");
    };
    assert!(ty.is_none());
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(spans, vec![Span::new(10, 15)]);
    // In blocks and function bodies too, each marked on its own.
    let ast = parse_text("{ let a = 1; let mut b = 2; fn(): Int { let mut c = 3; c } }").unwrap();
    let printed = ast.pretty().to_string();
    for head in ["(let a @", "(let mut b @", "(let mut c @"] {
        assert!(printed.contains(head), "{head}: {printed}");
    }
}

#[test]
fn pretty_output_for_mutable_lets() {
    let text = "let mut x = 1\nlet y = 2\nlet mut z: Float = 3";
    let expected = "(file\n\
                        \x20 (let mut x @0..13\n\
                        \x20   (int 1 @12..13))\n\
                        \x20 (let y @14..23\n\
                        \x20   (int 2 @22..23))\n\
                        \x20 (let mut z : Float @24..44\n\
                        \x20   (int 3 @43..44)))";
    assert_eq!(parse_text(text).unwrap().pretty().to_string(), expected);
    // The printed tree says all the source does: laid out otherwise, the
    // same items print the same but for their spans, and a `let` that is
    // not mutable prints without `mut`.
    let spread = "let\nmut\nx = 1\nlet y = 2\nlet mut\nz: Float = 3";
    let printed = parse_text(spread).unwrap().pretty().to_string();
    let heads = |printed: &str| -> Vec<String> {
        printed
            .lines()
            .map(|line| line.split(" @").next().unwrap_or(line).to_owned())
            .collect()
    };
    assert_eq!(heads(&printed), heads(expected));
    let plain = parse_text("let x = 1").unwrap().pretty().to_string();
    assert_eq!(plain, "(file\n  (let x @0..9\n    (int 1 @8..9)))");
}

#[test]
fn mut_is_only_allowed_after_let() {
    let misplaced = |at: u32, note: Option<&str>| {
        (
            MISPLACED_MUT.to_owned(),
            Span::new(at, at + 3),
            note.map(str::to_owned).into_iter().collect::<Vec<_>>(),
        )
    };
    // A field, with a note: dotted, quoted or plain.
    assert_eq!(
        error_with_notes("mut x = 1"),
        misplaced(0, Some(MUT_FIELD_NOTE))
    );
    assert_eq!(
        error_with_notes("{ mut a.\"b c\".d = 1 }"),
        misplaced(2, Some(MUT_FIELD_NOTE))
    );
    assert_eq!(
        error_with_notes("mut \"q\" = 1"),
        misplaced(0, Some(MUT_FIELD_NOTE))
    );
    assert_eq!(
        MUT_FIELD_NOTE,
        "a field is defined once; declare a variable with `let mut x = ...` and define the \
             field from it"
    );
    // A parameter, with a note.
    assert_eq!(
        error_with_notes("fn(mut n: Int): Int { n }"),
        misplaced(3, Some(MUT_PARAM_NOTE))
    );
    assert_eq!(
        error_with_notes("fn f(a: Int, mut b: Int): Int { a }"),
        misplaced(13, Some(MUT_PARAM_NOTE))
    );
    assert_eq!(
        MUT_PARAM_NOTE,
        "copy the parameter into a mutable variable, as in `let mut n = n`"
    );
    // A `for`'s variable, a named function item and a function
    // expression, without one.
    for (text, at) in [
        ("for mut x in xs { x }", 4),
        ("fn mut f(): Int { 1 }", 3),
        ("let g = fn mut(): Int { 1 }", 11),
    ] {
        assert_eq!(error_with_notes(text), misplaced(at, None), "{text}");
    }
    // Anywhere else an item, an expression, a name or a type starts, or
    // an item ends.
    for (text, at) in [
        ("mut x := 1", 0),
        ("mut x", 0),
        ("mut", 0),
        ("1 + mut", 4),
        ("f(mut x)", 2),
        ("f(mut = 1)", 2),
        ("[mut]", 1),
        ("x := mut", 5),
        ("let x: mut = 1", 7),
        ("let x: List<mut> = []", 12),
        ("let f: fn(mut: Int): Int = g", 10),
        ("a.mut", 2),
        ("a.mut = 1", 2),
        ("let r: {mut: Int} = r", 8),
        ("let mut mut x = 1", 8),
        ("let x = 1 mut", 10),
        ("x mut", 2),
        ("if mut { 1 }", 3),
        ("\"{mut}\"", 2),
    ] {
        assert_eq!(error_with_notes(text), misplaced(at, None), "{text}");
    }
    // Where a separator or a closing bracket is expected inside an
    // expression, the usual error stands.
    assert_eq!(
        error("f(x mut)"),
        (
            "expected `,` or `)`, found `mut`".to_owned(),
            Span::new(4, 7)
        )
    );
}

#[test]
fn a_mutable_let_needs_a_plain_name() {
    let note = vec![DOTTED_MUT_NOTE.to_owned()];
    assert_eq!(
        error_with_notes("let mut a.b = 1"),
        (DOTTED_MUT.to_owned(), Span::new(8, 11), note.clone())
    );
    assert_eq!(
        error_with_notes("let mut a.\"b c\".d: Int = 1"),
        (DOTTED_MUT.to_owned(), Span::new(8, 17), note)
    );
    assert_eq!(
        DOTTED_MUT_NOTE,
        "a dotted `let` updates a variable; declare the variable with `let mut a = ...` and \
             update it with `a.b := ...`"
    );
    // A plain name, as after `let`.
    assert_eq!(
        error("let mut \"x\" = 1"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(8, 11))
    );
    assert_eq!(
        error("let mut = 1"),
        ("expected a name after `mut`".to_owned(), Span::new(8, 9))
    );
    assert_eq!(
        error("let = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn index_segments_in_rebind_paths() {
    for (text, expected) in [
        // First, last, mixed with names, and two in a row.
        ("xs[0] := 1", "xs[0] := 1"),
        ("xs[0].a := 1", "xs[0].a := 1"),
        ("a.b[0] := 1", "a.b[0] := 1"),
        ("conf.hosts[0].up := true", "conf.hosts[0].up := true"),
        ("grid[y][x] := 1", "grid[y][x] := 1"),
        ("a.\"b c\"[0].d := 1", "a.\"b c\"[0].d := 1"),
        // Any expression as the position: a call, arithmetic, another
        // index, a block, even one holding a path of its own.
        ("xs[f(1, y = 2)] := 1", "xs[f(1, y = 2)] := 1"),
        ("xs[i + 1 * 2] := 1", "xs[(i + (1 * 2))] := 1"),
        ("xs[ys[0]] := 1", "xs[(ys at 0)] := 1"),
        ("xs[{ let i = 1; i }] := 2", "xs[{let i = 1; i}] := 2"),
        (
            "xs[{ ys[0] := 1; 0 }][a.b] := 2",
            "xs[{ys[0] := 1; 0}][(a . b)] := 2",
        ),
        // Inside brackets, newlines do not end the item.
        ("xs[\n  i\n] := 1", "xs[i] := 1"),
        ("xs[0] :=\n  1", "xs[0] := 1"),
        // In a block.
        ("{ xs[0] := 1 }", "{xs[0] := 1}"),
    ] {
        assert_eq!(shape(text), expected, "{text}");
    }
    // An index read, compared or accessed is an expression, as ever.
    assert_eq!(shape("xs[0]"), "(xs at 0)");
    assert_eq!(shape("xs[0] == 1"), "((xs at 0) == 1)");
    assert_eq!(shape("xs[0].a"), "((xs at 0) . a)");
    // A path is read from its tokens: its positions are its only nodes.
    assert_eq!(parse_text("xs[i].a[0] := 1").unwrap().len(), 3);
}

#[test]
fn index_segments_span_their_brackets() {
    let text = "  xs[i + 1].name[ 0 ] := v";
    let ast = parse_text(text).unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 26));
    let ItemKind::Rebind { path, .. } = item.kind else {
        panic!("expected a rebind");
    };
    let segments = ast.path(path);
    let spans: Vec<Span> = segments.iter().map(|segment| segment.span).collect();
    assert_eq!(
        spans,
        vec![
            Span::new(2, 4),
            Span::new(4, 11),
            Span::new(12, 16),
            Span::new(16, 21)
        ]
    );
    assert_eq!(path_span(segments), Span::new(2, 21));
    // Each position is an expression of the tree, and the path shows it
    // as it is written between the brackets.
    let positions: Vec<Span> = segments
        .iter()
        .filter_map(PathSegment::position)
        .map(|position| ast.get(position).span)
        .collect();
    assert_eq!(positions, vec![Span::new(5, 10), Span::new(18, 19)]);
    assert_eq!(ast.show_path(segments).to_string(), "xs[i + 1].name[0]");
    assert_eq!(ast.show_path(&segments[..2]).to_string(), "xs[i + 1]");
    assert_eq!(segments[0].name(), ast.names().lookup("xs"));
    assert_eq!(segments[1].name(), None);
    // A block whose first item has a position starts with it.
    let ast = parse_text("{ xs[i] := v }").unwrap();
    let ItemKind::Expr(block) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a block");
    };
    let nodes: Vec<Span> = ast.subtree(block).map(|id| ast.get(id).span).collect();
    assert_eq!(
        nodes,
        vec![Span::new(5, 6), Span::new(11, 12), Span::new(0, 14)]
    );
}

#[test]
fn an_index_on_the_next_line_is_not_a_segment() {
    // As in an expression, the `[` starts the next item, a list literal,
    // which is not a path.
    assert_eq!(shape("xs\n[0]"), "xs; list(0)");
    assert_eq!(
        error("xs\n[0] := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(7, 9)
        )
    );
    assert_eq!(
        error("{ a.b\n[0] = 1 }"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(10, 11)
        )
    );
    assert_eq!(
        error("let xs\n[0] = 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(6, 7)
        )
    );
}

#[test]
fn only_a_rebind_path_may_index_a_list() {
    for (text, message, (start, end), path) in [
        ("let xs[0] = 1", LET_INDEX, (6, 9), "xs[0]"),
        (
            "let a.b[i + 1].c: Int = 1",
            LET_INDEX,
            (7, 14),
            "a.b[i + 1].c",
        ),
        ("let mut xs[0] = 1", LET_INDEX, (10, 13), "xs[0]"),
        ("xs[0] = 1", FIELD_INDEX, (2, 5), "xs[0]"),
        ("{ a.xs[0].b = 1 }", FIELD_INDEX, (6, 9), "a.xs[0].b"),
        ("\"q\"[0] = 1", FIELD_INDEX, (3, 6), "\"q\"[0]"),
        // A path written over several lines is shown on one.
        (
            "let xs[\n  i +\n  1\n] = 1",
            LET_INDEX,
            (6, 19),
            "xs[ i + 1 ]",
        ),
    ] {
        assert_eq!(
            error_with_notes(text),
            (
                message.to_owned(),
                Span::new(start, end),
                vec![format!("update the element with `{path} := ...`")]
            ),
            "{text}"
        );
    }
    assert_eq!(LET_INDEX, "a `let` path cannot index a list");
    assert_eq!(FIELD_INDEX, "a field's path cannot index a list");
    // A rebind's first name is a variable's, as ever.
    assert_eq!(
        error("\"x\"[0] := 1"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(0, 3))
    );
    // A mistake in a position is reported first.
    assert_eq!(
        error("let xs[) = 1"),
        ("expected expression, found `)`".to_owned(), Span::new(7, 8))
    );
    // Anything else before `:=` is still not a path.
    let found_colon_eq = |at: u32| {
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(at, at + 2),
        )
    };
    assert_eq!(error("xs[0] + 1 := 2"), found_colon_eq(10));
    assert_eq!(error("(xs)[0] := 2"), found_colon_eq(8));
    assert_eq!(error("f()[0] := 2"), found_colon_eq(7));
}

#[test]
fn pretty_output_for_index_paths() {
    let text = "xs[i + 1].up := true\ngrid[y][0] := 1";
    let expected = "(file\n\
                        \x20 (rebind xs[i + 1].up @0..20\n\
                        \x20   (at\n\
                        \x20     (+ @3..8\n\
                        \x20       (name i @3..4)\n\
                        \x20       (int 1 @7..8)))\n\
                        \x20   (bool true @16..20))\n\
                        \x20 (rebind grid[y][0] @21..36\n\
                        \x20   (at\n\
                        \x20     (name y @26..27))\n\
                        \x20   (at\n\
                        \x20     (int 0 @29..30))\n\
                        \x20   (int 1 @35..36)))";
    assert_eq!(parse_text(text).unwrap().pretty().to_string(), expected);
    // Laid out otherwise, the same items print the same but for their
    // spans.
    let spread = "xs[\n  i +\n  1\n].up :=\n  true\ngrid[ y ][0] := 1";
    let heads = |printed: &str| -> Vec<String> {
        printed
            .lines()
            .map(|line| line.split(" @").next().unwrap_or(line).to_owned())
            .collect()
    };
    let printed = parse_text(spread).unwrap().pretty().to_string();
    assert_eq!(heads(&printed), heads(expected));
    // A path shown as it is written reads back as the same path.
    for path in [
        "conf.hosts[i + 1].up",
        "grid[y][f(x, 2)]",
        "a.\"b c\"[ys[0]].d",
    ] {
        let text = format!("{path} := 1");
        let ast = parse_text(&text).unwrap();
        let ItemKind::Rebind { path: range, .. } = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected a rebind");
        };
        let shown = ast.show_path(ast.path(range)).to_string();
        assert_eq!(shown, path);
        assert_eq!(shape(&format!("{shown} := 1")), shape(&text));
    }
}

#[test]
fn index_positions_count_towards_height() {
    // A chain of N terms is N levels: as a position of a path of two
    // segments, the item is two levels more, at the limit with one fewer
    // than the limit allows a chain alone.
    let chain = |terms: usize| format!("0{}", " + 0".repeat(terms - 1));
    let deepest = format!("xs[{}] := 1", chain(MAX_DEPTH - 2));
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(format!("xs[{}] := 1", chain(MAX_DEPTH - 1)));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(diagnostic.span.start, 0);
    // Taller than the value, the position decides the height.
    let deep = format!("{{ xs[{}] := 1 }}", chain(MAX_DEPTH - 3));
    on_big_stack(move || parse_text(&deep).unwrap());
    too_deep(format!("{{ xs[{}] := 1 }}", chain(MAX_DEPTH - 2)));
    // Positions nested in each other recurse, and are stopped as any
    // expression nested too deeply is.
    let nested = format!(
        "xs[{}0{}] := 1",
        "ys[".repeat(MAX_DEPTH),
        "]".repeat(MAX_DEPTH)
    );
    assert_eq!(too_deep(nested).message, "expression nested too deeply");
}
