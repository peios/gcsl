//! Tests of the parser: recovery from errors, and attribute uses.

use super::*;

/// Every error of `text`, lexical and syntax, as its message and the
/// source it is at.
pub(super) fn all_errors(text: &str) -> Vec<(String, String)> {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (_, diagnostics) = parse_source(&source);
    diagnostics
        .iter()
        .map(|d| (d.message.clone(), source.slice(d.span).to_owned()))
        .collect()
}

/// The tree recovered from `text`, as [`shape`] writes it, an error
/// node as `<error>`, and how far it can be trusted.
pub(super) fn recovered(text: &str) -> (String, Syntax) {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (ast, _) = parse_source(&source);
    let mut out = String::new();
    body(&ast, ast.root(), &mut out);
    (out, ast.syntax())
}

/// `pairs` as [`all_errors`] gives them.
pub(super) fn errors_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|&(message, at)| (message.to_owned(), at.to_owned()))
        .collect()
}

#[test]
fn a_valid_file_parses_as_it_did_with_no_error() {
    let text = "x = f(1, [2, 3])\ny: {a: Int} = {a = 1}\ns = \"a{x}b\"";
    assert!(all_errors(text).is_empty());
    let (shape_of, syntax) = recovered(text);
    assert_eq!(shape_of, shape(text));
    assert_eq!(syntax, Syntax::Clean);
}

#[test]
fn items_recover_at_their_newline_or_semicolon() {
    // An item that fails having gone on past a newline, after an
    // operator or `=`, to a line that is an item of its own, ends at
    // that newline: the line is the next item.
    assert_eq!(
        all_errors("let a = 1 +\nlet b: Int = \"x\"\nlet c = f(1,, 2)\nlet e = 3"),
        errors_of(&[
            ("expected expression, found newline", "\n"),
            ("expected expression, found `,`", ","),
        ])
    );
    assert_eq!(
        recovered("let a = 1 +\nlet b: Int = \"x\"\nlet c = f(1,, 2)\nlet e = 3").0,
        "let a = <error>; let b: Int = \"x\"; let c = <error>; let e = 3"
    );
    assert_eq!(
        all_errors(
            "x = \ny = 2\nz: Int =\n#[shadowable] w = 1\nfn f(): Int { 1 }\nv =\nfn g(): Int { 2 }"
        ),
        errors_of(&[
            ("expected expression, found newline", "\n"),
            ("expected expression, found newline", "\n"),
            ("expected expression, found newline", "\n"),
        ])
    );
    assert_eq!(
        recovered("x = \ny = 2\nz: Int =\n#[shadowable] w = 1\nv =\nfn g(): Int { 2 }").0,
        "x = <error>; y = 2; z: Int = <error>; w = 1; v = <error>; fn g(): Int {2}"
    );
    // In a block, and for an item that fails before its value.
    assert_eq!(
        recovered("f = {\n    a = 1 +\n    b = 2\n}\ntype T =\nc = 3").0,
        "f = {a = <error>; b = 2}; <error>; c = 3"
    );
    assert_eq!(
        all_errors("type T =\nc = 3"),
        errors_of(&[("expected a type", "\n")])
    );
    // A line that may go on the line before is not an item of its own:
    // it is the operand the line before was missing, and the error
    // after it the item's one error.
    assert_eq!(
        all_errors("x = 1 +\n2 3\ny = 4"),
        errors_of(&[("expected end of item, found `3`", "3")])
    );
    assert_eq!(recovered("x = 1 +\n2 3\ny = 4").0, "x = <error>; y = 4");
    // A line inside brackets left open is no item of the body: the
    // lexer emits no newline there, so the rest of the file is the
    // list's.
    assert_eq!(
        all_errors("d = [1, 2\ne = 3\nf = 4"),
        errors_of(&[("expected `,` or `]`, found `e`", "e")])
    );
    assert_eq!(recovered("d = [1, 2\ne = 3").0, "d = <error>");
    // A value may be missing, its separator ending it still.
    assert_eq!(
        all_errors("x = ; y = 1"),
        errors_of(&[("expected expression, found `;`", ";")])
    );
    assert_eq!(recovered("x = ; y = 1").0, "x = <error>; y = 1");
    let expected = errors_of(&[
        ("expected end of item, found `2`", "2"),
        ("expected end of item, found `5`", "5"),
    ]);
    assert_eq!(all_errors("x = 1 2\ny = 3\nz = 4 5"), expected);
    assert_eq!(all_errors("x = 1 2; y = 3; z = 4 5"), expected);
    // The items keep their names, their values are errors.
    let (tree, syntax) = recovered("x = 1 2\ny = 3\nz = 4 5");
    assert_eq!(tree, "x = <error>; y = 3; z = <error>");
    assert_eq!(syntax, Syntax::Recovered);
    // An item that fails before its value is an error item.
    assert_eq!(
        all_errors("x . = 1\ny = 2\n) z"),
        errors_of(&[
            ("expected a field name after `.`", "="),
            ("expected expression, found `)`", ")"),
        ])
    );
    assert_eq!(
        recovered("x . = 1\ny = 2\n) z").0,
        "<error>; y = 2; <error>"
    );
}

#[test]
fn a_block_s_items_recover_at_its_closing_brace() {
    assert_eq!(
        all_errors("f = { a = 1 2 }\ng = { b = 3 4 }"),
        errors_of(&[
            ("expected end of item, found `2`", "2"),
            ("expected end of item, found `4`", "4"),
        ])
    );
    assert_eq!(
        recovered("f = { a = 1 2 }\ng = { b = 3 4 }").0,
        "f = {a = <error>}; g = {b = <error>}"
    );
    // An error inside a block is that block's item's alone.
    assert_eq!(
        all_errors("f = {\n    a = 1\n    b = = 2\n    c = 3\n}\ng = 4 4"),
        errors_of(&[
            ("expected expression, found `=`", "="),
            ("expected end of item, found `4`", "4"),
        ])
    );
    assert_eq!(
        recovered("f = {\n    a = 1\n    b = = 2\n    c = 3\n}").0,
        "f = {a = 1; b = <error>; c = 3}"
    );
}

#[test]
fn lists_of_arguments_parameters_and_elements_recover_at_their_commas() {
    assert_eq!(
        all_errors("x = f(1 2, 3, 4 5)"),
        errors_of(&[
            ("expected `,` or `)`, found `2`", "2"),
            ("expected `,` or `)`, found `5`", "5"),
        ])
    );
    assert_eq!(
        all_errors("x = f(+, 2, *)"),
        errors_of(&[
            ("expected expression, found `+`", "+"),
            ("expected expression, found `*`", "*"),
        ])
    );
    assert_eq!(
        all_errors("fn f(a: Int b: Int, c: Int, d: ): Int { 1 }"),
        errors_of(&[
            ("expected `,` or `)`, found `b`", "b"),
            ("expected a type", ")"),
        ])
    );
    assert_eq!(
        all_errors("x = [1 2, 3, 4 5]"),
        errors_of(&[
            ("expected `,` or `]`, found `2`", "2"),
            ("expected `,` or `]`, found `5`", "5"),
        ])
    );
    assert_eq!(
        all_errors("t: fn(Int Int, String, ): Int = g\nu = fn(+): Int { 1 }"),
        errors_of(&[
            ("expected `,` or `)`, found `Int`", "Int"),
            ("expected a parameter name, found `+`", "+"),
        ])
    );
    // The value of an item whose list has an error is an error.
    assert_eq!(recovered("x = f(1 2, 3)\ny = 2").0, "x = <error>; y = 2");
    // A named function whose parameters fail declares nothing.
    assert_eq!(recovered("fn f(a: ): Int { 1 }\ny = 2").0, "<error>; y = 2");
}

#[test]
fn record_types_recover_at_their_commas() {
    assert_eq!(
        all_errors("x: {a: Int b: Int, c: Int, d: } = {}"),
        errors_of(&[("expected `,` or `}`", "b"), ("expected a type", "}"),])
    );
    assert_eq!(
        all_errors("type T = {\n    a: Int\n    b Int\n    c: String,\n    1: Int\n}"),
        errors_of(&[
            ("expected `,` or `}`", "b"),
            ("expected a field name in record type", "1"),
        ])
    );
}

#[test]
fn an_interpolation_recovers_at_its_closing_brace() {
    assert_eq!(
        all_errors("s = \"a{1 +}b{f(}c{2 3}\""),
        errors_of(&[
            ("expected expression, found `}`", "}"),
            ("expected expression, found `}`", "}"),
            ("expected `}`, found `3`", "3"),
        ])
    );
    assert_eq!(recovered("s = \"a{1 +}b\"\nt = 2").0, "s = <error>; t = 2");
}

#[test]
fn unclosed_brackets_and_strings_at_the_end_of_the_file() {
    // A `{`: the block ends with the file; the tree is unbalanced.
    assert_eq!(
        all_errors("x = 1\nf = {\n    a = 2"),
        errors_of(&[("expected `}`, found end of input", "")])
    );
    assert_eq!(recovered("x = 1\nf = {\n    a = 2").1, Syntax::Unbalanced);
    // Nested ones are reported once, for the innermost.
    assert_eq!(
        all_errors("f = {\n    g = {\n        a = 2"),
        errors_of(&[("expected `}`, found end of input", "")])
    );
    // A `(` and a `[`, inside which there are no newlines: the rest of
    // the file is their item's. An unclosed call keeps what it parsed.
    assert_eq!(
        all_errors("x = 1\ny = f(1, 2\nz = 3"),
        errors_of(&[("expected `,` or `)`, found `z`", "z")])
    );
    assert_eq!(
        recovered("x = 1\ny = f(1, 2\nz = 3").0,
        "x = 1; y = <unclosed f(1, 2)>"
    );
    assert_eq!(
        all_errors("x = [1, 2"),
        errors_of(&[("expected `,` or `]`, found end of input", "")])
    );
    // A string: nothing after its quote was lexed.
    assert_eq!(
        all_errors("x = 1\ny = \"abc\nz = (\n"),
        errors_of(&[("unterminated string literal", "\"")])
    );
    assert_eq!(recovered("x = 1\ny = \"abc").0, "x = 1; y = <error>");
    assert_eq!(
        all_errors("y = \"a{1\nz = 2"),
        errors_of(&[("unterminated interpolation", "{")])
    );
}

#[test]
fn a_field_access_with_no_name_keeps_its_record() {
    // At the end of a line, and of the file: one error, the record kept.
    for (text, tree) in [
        ("x = a.\ny = 2", "x = (a . <missing>); y = 2"),
        ("x = a.b.", "x = ((a . b) . <missing>)"),
        ("x = f(1).\n", "x = (f(1) . <missing>)"),
        ("f = {\n    x = a.\n}", "f = {x = (a . <missing>)}"),
        ("x = g(a.)", "x = g((a . <missing>))"),
        ("x = g(a., 1)", "x = g((a . <missing>), 1)"),
        ("x = [a.]", "x = list((a . <missing>))"),
    ] {
        let errors = all_errors(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
        assert_eq!(errors[0].0, "expected a field name after `.`", "{text:?}");
        assert_eq!(
            recovered(text),
            (tree.to_owned(), Syntax::Recovered),
            "{text:?}"
        );
    }
    // Where something else follows, as before: the value is an error.
    for text in ["x = a.)", "x = a. + 1", "x = a.}"] {
        assert!(recovered(text).0.starts_with("x = <error>"), "{text:?}");
    }
    // At the end of the file in a bracket, or before the `}` that
    // closes the bracket: the bracket's error is not reported again.
    for text in ["x = g(a.", "x = (a.", "x = [a.", "f = {\n    x = g(a.\n}"] {
        let errors = all_errors(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
    }
}

#[test]
fn an_unclosed_call_keeps_its_callee_and_arguments() {
    for (text, tree, error) in [
        (
            "x = f(",
            "x = <unclosed f()>",
            "expected expression, found end of input",
        ),
        (
            "x = f(1, y = 2",
            "x = <unclosed f(1, y = 2)>",
            "expected `,` or `)`, found end of input",
        ),
        (
            "x = f(a.",
            "x = <unclosed f((a . <missing>))>",
            "expected a field name after `.`",
        ),
        (
            "x = g(f(1, ",
            "x = <unclosed g(<unclosed f(1)>)>",
            "expected expression, found end of input",
        ),
        (
            "fn h(): Int {\n    let y = f(1,\n}",
            "fn h(): Int {let y = <unclosed f(1)>}",
            "expected expression, found `}`",
        ),
    ] {
        let errors = all_errors(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
        assert_eq!(errors[0].0, error, "{text:?}");
        assert_eq!(
            recovered(text),
            (tree.to_owned(), Syntax::Recovered),
            "{text:?}"
        );
    }
    // A list that ends at its `)` fails as before.
    assert_eq!(recovered("x = f(1 2)").0, "x = <error>");
}

#[test]
fn a_closing_brace_that_closes_nothing_is_reported_and_unbalances_the_tree() {
    assert_eq!(
        all_errors("x = 1\n}\ny = 2 2"),
        errors_of(&[
            ("unexpected `}`", "}"),
            ("expected end of item, found `2`", "2"),
        ])
    );
    let (tree, syntax) = recovered("x = 1\n}\ny = 2");
    assert_eq!(tree, "x = 1; y = 2");
    assert_eq!(syntax, Syntax::Unbalanced);
}

#[test]
fn lexical_errors_are_reported_once_by_the_lexer() {
    // Each error token is the lexer's, and what it is part of fails
    // silently.
    assert_eq!(
        all_errors("x = 1 @ 2\ny = $\nz = f(1, ?)"),
        errors_of(&[
            ("unexpected character `@`", "@"),
            ("unexpected character `$`", "$"),
            ("unexpected character `?`", "?"),
        ])
    );
    assert_eq!(
        recovered("x = 1 @ 2\ny = $\nz = 3").0,
        "x = <error>; y = <error>; z = 3"
    );
    // A malformed number is one token, however it goes on.
    assert_eq!(
        all_errors("x = 1__0.5e+3x + 1\ny = 2"),
        errors_of(&[("consecutive `_` in number literal", "__")])
    );
}

#[test]
fn errors_are_capped() {
    let text = "x = 1 1\n".repeat(150);
    let errors = all_errors(&text);
    assert_eq!(errors.len(), MAX_ERRORS + 1);
    assert_eq!(errors[MAX_ERRORS - 1].0, "expected end of item, found `1`");
    assert_eq!(errors[MAX_ERRORS].0, "too many errors");
    let text = "@\n".repeat(150);
    assert_eq!(all_errors(&text).len(), MAX_ERRORS + 1);
}

#[test]
fn parse_still_gives_the_first_error() {
    assert_eq!(
        message("x = 1 2\ny = 3 4"),
        "expected end of item, found `2`"
    );
}

#[test]
fn attributes_and_misplaced_items_recover() {
    // A misplaced attribute makes the item after it an error item.
    assert_eq!(
        all_errors("#[bogus]\nx = 1\ny = 2 2"),
        errors_of(&[
            (MISPLACED_USE, "#[bogus]"),
            ("expected end of item, found `2`", "2"),
        ])
    );
    assert_eq!(recovered("#[bogus]\nx = 1\ny = 2").0, "<error>; y = 2");
    // An import in a block, an error item.
    assert_eq!(
        recovered("f = {\n    import x\n    a = 1\n}").0,
        "f = {<error>; a = 1}"
    );
}

#[test]
fn nothing_hangs_on_any_prefix_of_a_program() {
    let text = "#![no_prelude]\nimport a/b as c\ntype T = {a: Int, b: String[] = []}\n\
                    #[shadowable] def f(x: Int, y: T = {a = 1}): Int -> Int {\n\
                    \x20   let mut z = [x, ...[1, 2]]\n    for i in 0..3 if i > 0 { z := z }\n\
                    \x20   do { break } for false\n    \"s{x}t{y.a}\" \n    = x\n}\n\
                    defmeta p q.r = 1\nv = p q.r(1)[0].x : Int as Int is Int";
    for end in (0..=text.len()).filter(|&end| text.is_char_boundary(end)) {
        let source = SourceFile::new("t.gcsl", &text[..end]).unwrap();
        let _ = parse_source(&source);
    }
}

#[test]
fn a_number_glued_to_a_name_that_is_no_unit_is_an_unknown_unit() {
    let error = parse_text("x = 30sec").unwrap_err();
    assert_eq!(error.message, "unknown unit `sec`");
    assert_eq!(error.span, Span::new(6, 9));
    assert_eq!(error.notes, [crate::lexer::UNITS_NOTE]);
    // A module path is no expression: `7zip` is its segment.
    assert!(parse_text("import pkgs/7zip as zip\nx = zip.v").is_ok());
    // A quantity is a literal, negated as a number is.
    assert_eq!(shape("x = -1h30m"), "x = (-1h30m)");
}

#[test]
fn attribute_uses_are_kept_on_the_function_items_they_apply_to() {
    let text = "#[attribute] type hermetic = {on: Bool = true}\n\
                #[hermetic] #[retry(3)]\nfn f(): Int { 1 }\n\
                #[shadowable] #[timeout(30s)] def g(): Int {\n    #[retry(1)] fn h(): Int { 2 }\n    h()\n}\n\
                #[hermetic(false)] def test x() { 1 }";
    let ast = parse_text(text).unwrap();
    let shown = ast.pretty().to_string();
    assert!(shown.contains("#[attribute] (type hermetic"), "{shown}");
    assert!(shown.contains("#[hermetic] #[retry(…)] (fn f"), "{shown}");
    assert!(
        shown.contains("#[shadowable] #[timeout(…)] (def g"),
        "{shown}"
    );
    assert!(shown.contains("#[retry(…)] (fn h"), "{shown}");
    assert!(shown.contains("#[hermetic(…)] (def-type-def"), "{shown}");
    // Each use, by its item, a nested `fn` before the item around it, its
    // name read as a type's.
    let uses = ast.all_attribute_uses();
    let named: Vec<&str> = uses.iter().map(|used| ast.names().get(used.name)).collect();
    assert_eq!(named, ["hermetic", "retry", "retry", "timeout", "hermetic"]);
    for used in uses {
        assert_eq!(ast.type_expr(used.ty).span, used.name_span);
        assert_eq!(
            &text[used.name_span.start as usize..used.name_span.end as usize],
            ast.names().get(used.name)
        );
    }
}

#[test]
fn misplaced_and_malformed_attribute_uses_are_errors() {
    assert_eq!(
        all_errors("#[retry(3)]\ny = 1"),
        errors_of(&[(MISPLACED_USE, "#[retry(3)]")])
    );
    assert_eq!(
        all_errors("#[retry(3)] #[retry(4)]\nfn f(): Int { 1 }"),
        errors_of(&[("`#[retry]` is written twice", "#[retry(4)]")])
    );
    assert_eq!(
        all_errors("#[shadowable(1)]\nx = 1"),
        errors_of(&[("`#[shadowable]` takes no arguments", "#[shadowable")])
    );
    assert_eq!(
        all_errors("#![x(1)]\nx = 1"),
        errors_of(&[("an inner attribute takes no arguments", "#![x")])
    );
    assert_eq!(
        all_errors("#[retry(3) x]\nfn f(): Int { 1 }"),
        errors_of(&[(
            "expected `]` after the attribute's arguments, found `x`",
            "x"
        )])
    );
    assert_eq!(
        all_errors("#[attribute] x = 1"),
        errors_of(&[(MISPLACED_ATTRIBUTE, "#[attribute]")])
    );
    assert_eq!(
        all_errors("#[retry(3)]"),
        errors_of(&[("`#[retry]` is not followed by anything", "#[retry(3)]")])
    );
    // A malformed attribute makes the item after it an error item, and the
    // next item is parsed as ever; one whose brackets do not balance leaves
    // them open, as a list's would, so what follows is part of it.
    assert_eq!(
        all_errors("#[retry(3) x]\nfn f(): Int { 1 }\ny = 2 2"),
        errors_of(&[
            (
                "expected `]` after the attribute's arguments, found `x`",
                "x"
            ),
            ("expected end of item, found `2`", "2"),
        ])
    );
    assert_eq!(
        all_errors("#[retry(3]\nfn f(): Int { 1 }\ny = 2 2"),
        errors_of(&[("expected `,` or `)`, found `]`", "]")])
    );
}
