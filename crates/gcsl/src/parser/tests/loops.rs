//! Tests of the parser: loop forms and their headers, `do`, jumps, labels,
//! `return` and labelled blocks.

use super::*;

#[test]
fn every_loop_form_parses_and_prints_back_to_itself() {
    for text in [
        "for x in xs {x}",
        "for x in xs if g {x}",
        "for c {x}",
        "for {break}",
        "for let i = 0; c; i := n {i}",
        "for let i = 0; let j: Int = 1; c {i}",
        "for c; i := n; xs[i] := 1 {i}",
        "for x := 1; c {x}",
        "for let r.a = 1; c {r}",
        "do {x} for c",
        "do'again {break'again 1} for c",
        "for'rows r in rows {for c in r {continue'rows 0}}",
        "for'spin {break'spin}",
        "for x in xs {break x}",
        "for x in xs {continue}",
        "for x in xs {continue x; break}",
    ] {
        assert_eq!(shape(text), text);
    }
    assert_eq!(
        shape("let first = for x in xs { if x > 5 { break x }; continue }"),
        "let first = for x in xs {(if (x > 5) {break x}); continue}"
    );
    // The summary's examples.
    assert_eq!(
        shape("for n < 3 { n := n + 1 }"),
        "for (n < 3) {n := (n + 1)}"
    );
    assert_eq!(
        shape("let sq = for let i = 0; i < 4; i := i + 1 { i * i }"),
        "let sq = for let i = 0; (i < 4); i := (i + 1) {(i * i)}"
    );
    assert_eq!(
        shape("let tries = do { attempt() } for not ok()"),
        "let tries = do {attempt()} for (not ok())"
    );
}

#[test]
fn a_name_before_in_begins_an_iteration_and_anything_else_a_condition() {
    let head = |text: &str| {
        let ast = parse_text(text).unwrap();
        let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
            panic!("{text}: expected a loop");
        };
        ast.for_decl(id).head
    };
    assert!(matches!(head("for x in xs {}"), ForHead::Each { .. }));
    assert!(matches!(head("for'a x in xs {}"), ForHead::Each { .. }));
    // A condition that starts with a name.
    for text in ["for x {}", "for x < 3 {}", "for x.done {}", "for x(1) {}"] {
        assert!(
            matches!(head(text), ForHead::While { cond: Some(_), .. }),
            "{text}"
        );
    }
    assert!(matches!(head("for {}"), ForHead::While { cond: None, .. }));
    assert!(matches!(head("do {} for x"), ForHead::Do { .. }));
    // A `let mut` in a header.
    let ast = parse_text("for let mut i = 0; c {}").unwrap();
    assert!(matches!(
        ast.items(ast.root()).first().map(|&id| ast.item(id).kind),
        Some(ItemKind::Expr(_))
    ));
    assert!(ast.pretty().to_string().contains("(let mut i @4..17"));
    // `in` is a keyword, so `for x in` is always an iteration's start.
    assert_eq!(
        error("for x in {}"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 11)
        )
    );
}

#[test]
fn a_header_splits_into_items_and_one_condition() {
    let parts = |text: &str| {
        let ast = parse_text(text).unwrap();
        let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
            panic!("{text}: expected a loop");
        };
        let ForHead::While {
            before,
            cond,
            after,
        } = ast.for_decl(id).head
        else {
            panic!("{text}: expected a condition loop");
        };
        (
            ast.items(before).len(),
            cond.map(|cond| ast.get(cond).span),
            ast.items(after).len(),
        )
    };
    assert_eq!(parts("for c {}"), (0, Some(Span::new(4, 5)), 0));
    assert_eq!(
        parts("for let i = 0; c {}"),
        (1, Some(Span::new(15, 16)), 0)
    );
    assert_eq!(parts("for c; i := 1 {}"), (0, Some(Span::new(4, 5)), 1));
    assert_eq!(
        parts("for let i = 0; let j = 0; c; i := 1; j := 2 {}"),
        (2, Some(Span::new(26, 27)), 2)
    );
    // A `:=` may come before the condition too.
    assert_eq!(parts("for i := 0; c {}"), (1, Some(Span::new(12, 13)), 0));
    // A newline may follow a `;`.
    assert_eq!(
        parts("for let i = 0;\n  c;\n  i := 1 {}"),
        (1, Some(Span::new(17, 18)), 1)
    );
    // The loop spans its keyword through its body, its header's items
    // being items of the tree.
    let ast = parse_text("for let i = 0; c; i := 1 {}").unwrap();
    let expr = ast.get(only_value(&ast));
    assert_eq!(expr.span, Span::new(0, 27));
    assert_eq!(ast.loop_keyword(only_value(&ast)), Span::new(0, 3));
}

#[test]
fn header_errors() {
    // Two conditions, at the second, labelled at the first.
    let diagnostic = parse_text("for a; b {}").unwrap_err();
    assert_eq!(diagnostic.message, "a `for` header has one condition");
    assert_eq!(diagnostic.span, Span::new(7, 8));
    assert_eq!(diagnostic.labels[0].span, Span::new(4, 5));
    assert_eq!(diagnostic.labels[0].text, "the condition is here");
    // Items alone.
    let diagnostic = parse_text("for let i = 0; i := 1 {}").unwrap_err();
    assert_eq!(diagnostic.message, "a `for` header needs a condition");
    assert_eq!(diagnostic.span, Span::new(4, 21));
    assert_eq!(diagnostic.notes, ["write `true` to loop until a `break`"]);
    // A `let` after the condition.
    let diagnostic = parse_text("for c; let i = 0 {}").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "a `let` after the condition would be new on every iteration and seen by nothing"
    );
    assert_eq!(diagnostic.span, Span::new(7, 16));
    assert_eq!(
        diagnostic.notes,
        ["declare it before the condition and rebind it here with `:=`"]
    );
    // No field, spread or named function.
    assert_eq!(
        error("for x = 1; c {}"),
        (
            "a `for` header cannot define a field".to_owned(),
            Span::new(4, 9)
        )
    );
    assert_eq!(error("for ...x; c {}").0, MISPLACED_SPREAD);
    assert_eq!(
        error("for c\n{}"),
        (
            "expected `;` or `{` in a `for` header, found newline".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("for a < b 1 {}"),
        (
            "expected `;` or `{` in a `for` header, found `1`".to_owned(),
            Span::new(10, 11)
        )
    );
    // A block as a header's value or condition is the body, so the
    // header is in error; in parentheses it is a value.
    let diagnostic = parse_text("for let r = { a = 1 }; r.a < 3; r.a := r.a + 1 { }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "a `{` in a `for` header begins its body"
    );
    assert_eq!(diagnostic.span, Span::new(12, 13));
    assert_eq!(
        shape("for let r = ({ a = 1 }); r.a < 3; r.a := r.a + 1 { }"),
        "for let r = [{a = 1}]; ((r . a) < 3); r.a := ((r . a) + 1) {}"
    );
    // `for` directly followed by `{` is `for BLOCK`, an operand like any
    // other.
    assert_eq!(shape("for {x} < 3; c {}"), "(for {x} < 3); c(){}");
    // No `{` is looked for ahead: a block after a `;` is a part.
    assert_eq!(
        error("for c; { }").0,
        "a `{` in a `for` header begins its body"
    );
    // Inside brackets and calls a block is a block again.
    assert_eq!(
        shape("for let x = f({ 1 }); [{ 2 }] != x {}"),
        "for let x = f({1}); (list({2}) != x) {}"
    );
}

#[test]
fn do_and_its_errors() {
    let ast = parse_text("do { x } for c").unwrap();
    let root = only_value(&ast);
    assert_eq!(ast.get(root).span, Span::new(0, 14));
    assert_eq!(ast.loop_keyword(root), Span::new(0, 2));
    // No `for` after the body.
    assert_eq!(
        error("do { x }"),
        (MISSING_DO_FOR.to_owned(), Span::new(8, 8))
    );
    assert_eq!(
        error("do { x } c"),
        (MISSING_DO_FOR.to_owned(), Span::new(9, 10))
    );
    // `for` on the next line.
    assert_eq!(
        error("do { x }\nfor c"),
        (MISSING_DO_FOR.to_owned(), Span::new(8, 9))
    );
    // Only a condition after the `for`.
    for (text, span) in [
        ("do {} for let i = 0", Span::new(10, 13)),
        ("do {} for x in xs", Span::new(10, 14)),
        ("do {} for'a c", Span::new(9, 11)),
    ] {
        assert_eq!(error(text), (DO_CONDITION_ONLY.to_owned(), span), "{text}");
    }
    // A `{` after the condition is not a body: the condition ends as an
    // expression statement does, where a `{` on its line starts a
    // trailing block.
    assert_eq!(shape("do {} for c {}"), "do {} for c(){}");
    // On the next line, it is the next item.
    assert_eq!(shape("do {} for c\n{}"), "do {} for c; {}");
    assert_eq!(
        error("do x for c"),
        (
            "expected `{` after `do`, found `x`".to_owned(),
            Span::new(3, 4)
        )
    );
}

/// The `break` or `continue` that `text` consists of: its kind, as
/// written, its label's name, its value's span and its span.
pub(super) fn jump(text: &str) -> (&'static str, Option<String>, Option<Span>, Span) {
    let ast = parse_text(text).unwrap();
    let [id] = ast.items(ast.root()) else {
        panic!("{text}: expected one item");
    };
    let item = ast.item(*id);
    let (keyword, jump) = match item.kind {
        ItemKind::Break(jump) => ("break", jump),
        ItemKind::Continue(jump) => ("continue", jump),
        ItemKind::Return(jump) => ("return", jump),
        other => panic!("{text}: expected a jump, got {other:?}"),
    };
    (
        keyword,
        jump.label
            .map(|label| ast.names().get(label.name).to_owned()),
        jump.value.map(|value| ast.get(value).span),
        item.span,
    )
}

#[test]
fn break_and_continue_with_and_without_a_value_and_a_label() {
    assert_eq!(jump("break"), ("break", None, None, Span::new(0, 5)));
    assert_eq!(jump("continue"), ("continue", None, None, Span::new(0, 8)));
    assert_eq!(
        jump("break x + 1"),
        ("break", None, Some(Span::new(6, 11)), Span::new(0, 11))
    );
    assert_eq!(
        jump("break'rows"),
        ("break", Some("rows".to_owned()), None, Span::new(0, 10))
    );
    assert_eq!(
        jump("continue'rows 0"),
        (
            "continue",
            Some("rows".to_owned()),
            Some(Span::new(14, 15)),
            Span::new(0, 15)
        )
    );
    // A newline, a `;` or a `}` after the keyword means no value.
    assert_eq!(shape("break\n1"), "break; 1");
    assert_eq!(shape("continue'a\nx"), "continue'a; x");
    assert_eq!(shape("break; 1"), "break; 1");
    assert_eq!(shape("{ break }"), "{break}");
    // They are items of any plain block, an operand or not.
    assert_eq!(
        shape("let x = if c { break } else { 1 }"),
        "let x = (if c {break} else {1})"
    );
    assert_eq!(shape("f({ continue })"), "f({continue})");
}

#[test]
fn break_and_continue_are_never_expressions() {
    for (text, found, span) in [
        ("x or break", "break", Span::new(5, 10)),
        ("1 + continue", "continue", Span::new(4, 12)),
        ("f(break)", "break", Span::new(2, 7)),
        ("[continue]", "continue", Span::new(1, 9)),
        ("let x = break", "break", Span::new(8, 13)),
        ("= break", "break", Span::new(2, 7)),
    ] {
        assert_eq!(
            error(text),
            (format!("expected expression, found `{found}`"), span),
            "{text}"
        );
    }
}

#[test]
fn a_label_is_written_directly_after_its_keyword() {
    for (text, keyword, span) in [
        ("for 'a x in xs {}", "for", Span::new(4, 6)),
        ("do 'a {} for c", "do", Span::new(3, 5)),
        ("break 'a", "break", Span::new(6, 8)),
        ("continue 'a", "continue", Span::new(9, 11)),
    ] {
        assert_eq!(
            error(text),
            (
                format!("a label is written directly after `{keyword}`, with no space"),
                span
            ),
            "{text}"
        );
    }
    // Anywhere else, a label is misplaced.
    for (text, span) in [
        ("'a", Span::new(0, 2)),
        ("x 'a", Span::new(2, 4)),
        ("1 + 'a", Span::new(4, 6)),
        ("f('a)", Span::new(2, 4)),
        ("let 'a = 1", Span::new(4, 6)),
        ("for'a'b x in xs {}", Span::new(5, 7)),
    ] {
        assert_eq!(error(text), (MISPLACED_LABEL.to_owned(), span), "{text}");
    }
    // Where a particular token is expected, the usual error stands.
    assert_eq!(
        error("if c 'a {}"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(5, 7)
        )
    );
}

#[test]
fn do_break_and_continue_are_not_names() {
    assert_eq!(
        error("let do = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 6))
    );
    assert_eq!(
        error("let break = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 9))
    );
    assert_eq!(
        error("fn(continue: Int): Int { 1 }").0,
        "expected a parameter name, found `continue`"
    );
    assert_eq!(error("x.do").0, "expected a field name after `.`");
    // A quoted field name may still be any text.
    assert_eq!(
        shape("\"do\" = 1; \"break\".x = 2"),
        "\"do\" = 1; \"break\".x = 2"
    );
}

#[test]
fn pretty_output_for_loops_and_jumps() {
    let ast = parse_text(
        "for'w let i = 0; i < 2; i := i + 1 { continue'w i }\n\
             do { break } for c\nfor {}",
    )
    .unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..51\n\
             \x20   (for'w @0..51\n\
             \x20     (let i @6..15\n\
             \x20       (int 0 @14..15))\n\
             \x20     (cond\n\
             \x20       (< @17..22\n\
             \x20         (name i @17..18)\n\
             \x20         (int 2 @21..22)))\n\
             \x20     (rebind i @24..34\n\
             \x20       (+ @29..34\n\
             \x20         (name i @29..30)\n\
             \x20         (int 1 @33..34)))\n\
             \x20     (block @35..51\n\
             \x20       (continue'w @37..49\n\
             \x20         (name i @48..49)))))\n\
             \x20 (expr @52..70\n\
             \x20   (do @52..70\n\
             \x20     (block @55..64\n\
             \x20       (break @57..62))\n\
             \x20     (cond\n\
             \x20       (name c @69..70))))\n\
             \x20 (expr @71..77\n\
             \x20   (for @71..77\n\
             \x20     (block @75..77))))"
    );
}

#[test]
fn return_with_and_without_a_value_and_a_label() {
    assert_eq!(jump("return"), ("return", None, None, Span::new(0, 6)));
    assert_eq!(
        jump("return x + 1"),
        ("return", None, Some(Span::new(7, 12)), Span::new(0, 12))
    );
    assert_eq!(
        jump("return'pick"),
        ("return", Some("pick".to_owned()), None, Span::new(0, 11))
    );
    assert_eq!(
        jump("return'pick 2"),
        (
            "return",
            Some("pick".to_owned()),
            Some(Span::new(12, 13)),
            Span::new(0, 13)
        )
    );
    // A newline, a `;` or a `}` after the keyword means no value.
    assert_eq!(shape("return\n1"), "return; 1");
    assert_eq!(shape("return'a\nx"), "return'a; x");
    assert_eq!(shape("return; 1"), "return; 1");
    assert_eq!(shape("{ return }"), "{return}");
    // An item of any plain block, an operand or not.
    assert_eq!(
        shape("let x = if c { return 1 } else { 2 }"),
        "let x = (if c {return 1} else {2})"
    );
    assert_eq!(shape("f(g({ return 1 }))"), "f(g({return 1}))");
}

#[test]
fn every_return_and_labelled_block_form_prints_back_to_itself() {
    for text in [
        "return",
        "return 1",
        "return'a",
        "return'a x",
        "{'pick return'pick 1; 3}",
        "{'a {'b return'a 1}}",
        "for x in xs {'it return'it 0; x}",
        "for'scan x in xs {return'scan x}",
        "for'spin {return'spin r}",
        "do'd {return'd 1} for c",
        "f(){'t 1}",
    ] {
        assert_eq!(shape(text), text);
    }
    assert_eq!(
        shape("if c {'t 1} else {'e return'e 2}"),
        "(if c {'t 1} else {'e return'e 2})"
    );
    assert_eq!(shape("lazy {'l return 1}"), "(lazy {'l return 1})");
    // A function's body may be labelled, as any block.
    assert_eq!(
        shape("fn f(): Int {'body return'body 1}"),
        "fn f(): Int {'body return'body 1}"
    );
    // The summary's examples.
    assert_eq!(
        shape("let y = {'pick if a { return'pick 1 }; if b { return'pick 2 }; 3 }"),
        "let y = {'pick (if a {return'pick 1}); (if b {return'pick 2}); 3}"
    );
    assert_eq!(
        shape("fn sign(n: Int): Int { if n < 0 { return -1 }; 0 }"),
        "fn sign(n: Int): Int {(if (n < 0) {return (-1)}); 0}"
    );
    assert_eq!(
        shape("let v: Int = for'spin { let r = poll(); if r > 0 { return'spin r } }"),
        "let v: Int = for'spin {let r = poll(); (if (r > 0) {return'spin r})}"
    );
}

#[test]
fn return_is_never_an_expression() {
    for (text, span) in [
        ("x or return 1", Span::new(5, 11)),
        ("1 + return", Span::new(4, 10)),
        ("f(return)", Span::new(2, 8)),
        ("[return 1]", Span::new(1, 7)),
        ("let x = return", Span::new(8, 14)),
        ("= return", Span::new(2, 8)),
    ] {
        assert_eq!(
            error(text),
            ("expected expression, found `return`".to_owned(), span),
            "{text}"
        );
    }
}

#[test]
fn a_block_label_is_written_directly_after_its_brace() {
    for (text, keyword, span) in [
        ("{ 'a 1 }", "{", Span::new(2, 4)),
        ("return 'a", "return", Span::new(7, 9)),
        ("fn f(): Int { 'b 1 }", "{", Span::new(14, 16)),
    ] {
        assert_eq!(
            error(text),
            (
                format!("a label is written directly after `{keyword}`, with no space"),
                span
            ),
            "{text}"
        );
    }
    // A label on the next line, or in an interpolation's braces, is
    // misplaced.
    for (text, span) in [
        ("{\n'a 1 }", Span::new(2, 4)),
        ("\"x{'a 1}\"", Span::new(3, 5)),
        ("{'a'b 1}", Span::new(3, 5)),
    ] {
        assert_eq!(error(text), (MISPLACED_LABEL.to_owned(), span), "{text}");
    }
    // A block in an interpolation may be labelled.
    assert_eq!(shape("\"x{ {'a return'a 1} }\""), "\"x<{'a return'a 1}>\"");
    assert_eq!(
        MISPLACED_LABEL,
        "a label names a loop or a block: it follows `for`, `do`, `break`, `continue`, \
             `return` or `{`"
    );
}

#[test]
fn a_record_block_cannot_be_labelled() {
    for (text, span) in [
        ("{'a x = 1 }", Span::new(1, 3)),
        ("{'a ...r }", Span::new(1, 3)),
        ("let r = {'rec a.b = 1; c = 2 }", Span::new(9, 13)),
    ] {
        assert_eq!(
            error(text),
            ("a record block cannot be labelled".to_owned(), span),
            "{text}"
        );
    }
    // Without its label, or with only a `let`, it is fine.
    assert_eq!(shape("{x = 1}"), "{x = 1}");
    assert_eq!(shape("{'a let x = 1; x}"), "{'a let x = 1; x}");
}

#[test]
fn return_is_not_a_name() {
    assert_eq!(
        error("let return = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 10))
    );
    assert_eq!(
        error("fn(return: Int): Int { 1 }").0,
        "expected a parameter name, found `return`"
    );
    assert_eq!(error("x.return").0, "expected a field name after `.`");
    assert_eq!(shape("\"return\" = 1"), "\"return\" = 1");
}

#[test]
fn pretty_output_for_returns_and_labelled_blocks() {
    let ast = parse_text("{'pick return'pick 1 }\nreturn").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..22\n\
             \x20   (block'pick @0..22\n\
             \x20     (return'pick @7..20\n\
             \x20       (int 1 @19..20))))\n\
             \x20 (return @23..29))"
    );
    // A label's height counts nothing; a value's counts as an item's.
    let ast = parse_text("{'a return'a 1 + 2 }").unwrap();
    let ExprKind::Block { items, label } = ast.get(only_value(&ast)).kind else {
        panic!("expected a block");
    };
    assert_eq!(
        label.map(|label| ast.label(label).span),
        Some(Span::new(1, 3))
    );
    assert_eq!(ast.items(items).len(), 1);
}
