use super::*;
use crate::strings::Quoted;

fn lex(text: &str) -> Result<Vec<Token>, Diagnostic> {
    tokenize(&SourceFile::new("test.gcsl", text).unwrap()).map(|(tokens, _, _)| tokens)
}

fn kinds(text: &str) -> Vec<TokenKind> {
    lex(text).unwrap().into_iter().map(|t| t.kind).collect()
}

/// The tokens with identifiers shown by their text, e.g. `Ident(x)`, and
/// string pieces by their quoted text, e.g. `Str("a")`.
fn show(text: &str) -> Vec<String> {
    let (tokens, names, strings) = tokenize(&SourceFile::new("test.gcsl", text).unwrap()).unwrap();
    tokens
        .iter()
        .map(|token| match token.kind {
            TokenKind::Ident(name) => format!("Ident({})", names.get(name)),
            TokenKind::Attribute(name) => format!("Attribute({})", names.get(name)),
            TokenKind::InnerAttribute(name) => {
                format!("InnerAttribute({})", names.get(name))
            }
            TokenKind::Str(id) => format!("Str({})", Quoted(strings.get(id))),
            TokenKind::StrStart(id) => format!("StrStart({})", Quoted(strings.get(id))),
            TokenKind::StrMid(id) => format!("StrMid({})", Quoted(strings.get(id))),
            TokenKind::StrEnd(id) => format!("StrEnd({})", Quoted(strings.get(id))),
            kind => format!("{kind:?}"),
        })
        .collect()
}

/// The text of the single string literal without interpolations that
/// `text` consists of.
fn string(text: &str) -> String {
    let (tokens, _, strings) = tokenize(&SourceFile::new("test.gcsl", text).unwrap()).unwrap();
    match tokens[..] {
        [
            Token {
                kind: TokenKind::Str(id),
                ..
            },
            Token {
                kind: TokenKind::Eof,
                ..
            },
        ] => strings.get(id).to_owned(),
        _ => panic!("{text:?} is not a single string literal: {tokens:?}"),
    }
}

fn error(text: &str) -> (String, Span) {
    let diagnostic = lex(text).unwrap_err();
    (diagnostic.message, diagnostic.span)
}

fn spans(text: &str) -> Vec<(u32, u32)> {
    lex(text)
        .unwrap()
        .iter()
        .map(|t| (t.span.start, t.span.end))
        .collect()
}

#[test]
fn tokens_are_small() {
    // A 16-byte kind (the `i64`/`f64` payloads) and an 8-byte span.
    assert_eq!(size_of::<Token>(), 24);
}

#[test]
fn every_token_kind() {
    use TokenKind::*;
    let (tokens, names, _) = tokenize(
        &SourceFile::new(
            "test.gcsl",
            "1 2.5 + - -x * / // % ( ) x let : := = { } ; . ,\n\
                 if else true false and or not == != < <= > >= === !== fn -> ->> lazy await |",
        )
        .unwrap(),
    )
    .unwrap();
    let x = names.lookup("x").unwrap();
    let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Int(1),
            Float(2.5),
            Plus,
            Minus,
            Neg,
            Ident(x),
            Star,
            Slash,
            SlashSlash,
            Percent,
            LParen,
            RParen,
            Ident(x),
            Let,
            Colon,
            ColonEq,
            Eq,
            LBrace,
            RBrace,
            Semi,
            Dot,
            Comma,
            Newline,
            If,
            Else,
            True,
            False,
            And,
            Or,
            Not,
            EqEq,
            BangEq,
            Lt,
            Le,
            Gt,
            Ge,
            EqEqEq,
            BangEqEq,
            Fn,
            Arrow,
            MemoArrow,
            Lazy,
            Await,
            Pipe,
            Eof
        ]
    );
    assert_eq!(Fn.symbol(), Some("fn"));
    assert_eq!(Arrow.symbol(), Some("->"));
    assert_eq!(MemoArrow.symbol(), Some("->>"));
    assert_eq!(Lazy.symbol(), Some("lazy"));
    assert_eq!(Await.symbol(), Some("await"));
    assert_eq!(Pipe.symbol(), Some("|"));
}

#[test]
fn def_is_a_keyword_only_as_a_whole_word() {
    assert_eq!(
        show("def def-x defx de f _def def_ Def"),
        [
            "Def",
            "Ident(def-x)",
            "Ident(defx)",
            "Ident(de)",
            "Ident(f)",
            "Ident(_def)",
            "Ident(def_)",
            "Ident(Def)",
            "Eof"
        ]
    );
    assert_eq!(TokenKind::Def.symbol(), Some("def"));
}

#[test]
fn import_is_a_keyword_and_from_is_not() {
    assert_eq!(
        show("import imports from"),
        ["Import", "Ident(imports)", "Ident(from)", "Eof"]
    );
    assert_eq!(TokenKind::Import.symbol(), Some("import"));
}

#[test]
fn is_and_as_are_keywords_only_as_whole_words() {
    assert_eq!(
        show("x is Int as is-x as-x isa asy Is As"),
        vec![
            "Ident(x)",
            "Is",
            "Ident(Int)",
            "As",
            "Ident(is-x)",
            "Ident(as-x)",
            "Ident(isa)",
            "Ident(asy)",
            "Ident(Is)",
            "Ident(As)",
            "Eof"
        ]
    );
    assert_eq!(TokenKind::Is.symbol(), Some("is"));
    assert_eq!(TokenKind::As.symbol(), Some("as"));
    assert!(!is_identifier("is"));
    assert!(!is_identifier("as"));
    assert!(is_identifier("is-x"));
}

#[test]
fn lazy_and_await_are_keywords_only_as_whole_words() {
    assert_eq!(
        show("lazy lazy-x lazyx await await-x awaits Lazy Future Present"),
        vec![
            "Lazy",
            "Ident(lazy-x)",
            "Ident(lazyx)",
            "Await",
            "Ident(await-x)",
            "Ident(awaits)",
            "Ident(Lazy)",
            "Ident(Future)",
            "Ident(Present)",
            "Eof"
        ]
    );
    assert!(!is_identifier("lazy"));
    assert!(!is_identifier("await"));
    assert!(is_identifier("lazy-x"));
    assert!(is_identifier("await-x"));
    // A negation before `await`, and `await` before a negation.
    assert_eq!(show("-await x"), vec!["Neg", "Await", "Ident(x)", "Eof"]);
    assert_eq!(show("await -x"), vec!["Await", "Neg", "Ident(x)", "Eof"]);
}

#[test]
fn a_pipe_is_one_token_and_closing_angles_are_two() {
    assert_eq!(
        show("Int|Future<Int>"),
        vec![
            "Ident(Int)",
            "Pipe",
            "Ident(Future)",
            "Lt",
            "Ident(Int)",
            "Gt",
            "Eof"
        ]
    );
    assert_eq!(
        show("Future<Future<Int>>"),
        vec![
            "Ident(Future)",
            "Lt",
            "Ident(Future)",
            "Lt",
            "Ident(Int)",
            "Gt",
            "Gt",
            "Eof"
        ]
    );
    assert_eq!(show("||"), vec!["Pipe", "Pipe", "Eof"]);
    assert_eq!(spans("a | b"), vec![(0, 1), (2, 3), (4, 5), (5, 5)]);
}

#[test]
fn fn_is_a_keyword_only_as_a_whole_word() {
    assert_eq!(
        show("fn fn-x fnx f n _fn fn_ Fn"),
        vec![
            "Fn",
            "Ident(fn-x)",
            "Ident(fnx)",
            "Ident(f)",
            "Ident(n)",
            "Ident(_fn)",
            "Ident(fn_)",
            "Ident(Fn)",
            "Eof"
        ]
    );
    assert_eq!(show("fn(")[..2], ["Fn", "LParen"]);
    assert!(!is_identifier("fn"));
    assert!(is_identifier("fn-x"));
}

#[test]
fn arrow_in_every_spacing() {
    // Directly after an operand, before one, between parentheses and
    // braces, and spaced: always one token.
    assert_eq!(show("a->b"), vec!["Ident(a)", "Arrow", "Ident(b)", "Eof"]);
    assert_eq!(show("x -> y"), vec!["Ident(x)", "Arrow", "Ident(y)", "Eof"]);
    assert_eq!(show(")->{"), vec!["RParen", "Arrow", "LBrace", "Eof"]);
    assert_eq!(show("->"), vec!["Arrow", "Eof"]);
    assert_eq!(show("1->2"), vec!["Int(1)", "Arrow", "Int(2)", "Eof"]);
    // Only the `-` directly before the `>` joins it.
    assert_eq!(
        error("-->"),
        ("unexpected character `-`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(show("- ->")[..2], ["Minus", "Arrow"]);
    assert_eq!(spans("a -> b"), vec![(0, 1), (2, 4), (5, 6), (6, 6)]);
    assert_eq!(spans("a->b"), vec![(0, 1), (1, 3), (3, 4), (4, 4)]);
    // `->>` is the memo arrow, one token, in every spacing, and a
    // further `>` is a greater-than.
    assert_eq!(show("->>"), vec!["MemoArrow", "Eof"]);
    assert_eq!(show("->>>"), vec!["MemoArrow", "Gt", "Eof"]);
    assert_eq!(
        show("a->>b"),
        vec!["Ident(a)", "MemoArrow", "Ident(b)", "Eof"]
    );
    assert_eq!(show(")->>{"), vec!["RParen", "MemoArrow", "LBrace", "Eof"]);
    assert_eq!(show("-> ->>"), vec!["Arrow", "MemoArrow", "Eof"]);
    assert_eq!(spans("a ->> b"), vec![(0, 1), (2, 5), (6, 7), (7, 7)]);
    // Only a `-` directly before `>>` makes one.
    assert_eq!(show("- >>"), vec!["Minus", "Gt", "Gt", "Eof"]);
    assert_eq!(show("-> >"), vec!["Arrow", "Gt", "Eof"]);
    // A `-` not directly followed by `>` follows the usual rules.
    assert_eq!(show("a - > b")[1..3], ["Minus", "Gt"]);
    assert_eq!(
        error("a- >b"),
        ("`-` here is ambiguous".to_owned(), Span::new(1, 2))
    );
    assert_eq!(show("-x")[0], "Neg");
}

#[test]
fn spans_are_exact() {
    assert_eq!(
        spans("12 +\t3.25//(  7)"),
        vec![
            (0, 2),
            (3, 4),
            (5, 9),
            (9, 11),
            (11, 12),
            (14, 15),
            (15, 16),
            (16, 16)
        ]
    );
}

#[test]
fn eof_is_placed_after_the_last_token_other_than_a_newline() {
    let tokens = lex("1 +  # trailing comment\n\n").unwrap();
    assert_eq!(tokens.last().unwrap().span, Span::new(3, 3));
    let tokens = lex("   ").unwrap();
    assert_eq!(
        tokens,
        vec![Token {
            kind: TokenKind::Eof,
            span: Span::new(0, 0)
        }]
    );
}

#[test]
fn slash_slash_is_one_token_and_triple_slash_is_two() {
    assert_eq!(
        kinds("///"),
        vec![TokenKind::SlashSlash, TokenKind::Slash, TokenKind::Eof]
    );
    assert_eq!(
        kinds("/ /"),
        vec![TokenKind::Slash, TokenKind::Slash, TokenKind::Eof]
    );
}

#[test]
fn comparison_operators_are_greedy() {
    assert_eq!(
        show("a==b a=b a<=b a<b a>=b a>b a!=b"),
        vec![
            "Ident(a)", "EqEq", "Ident(b)", "Ident(a)", "Eq", "Ident(b)", "Ident(a)", "Le",
            "Ident(b)", "Ident(a)", "Lt", "Ident(b)", "Ident(a)", "Ge", "Ident(b)", "Ident(a)",
            "Gt", "Ident(b)", "Ident(a)", "BangEq", "Ident(b)", "Eof"
        ]
    );
    // Three `=` are one token, lexed before `==` and `=`; four are `===`
    // then `=`; spaced, they are separate.
    assert_eq!(show("==="), vec!["EqEqEq", "Eof"]);
    assert_eq!(show("===="), vec!["EqEqEq", "Eq", "Eof"]);
    assert_eq!(show("== ="), vec!["EqEq", "Eq", "Eof"]);
    assert_eq!(show("= =="), vec!["Eq", "EqEq", "Eof"]);
    assert_eq!(show("= ="), vec!["Eq", "Eq", "Eof"]);
    assert_eq!(
        show("a===b a!==b a==b a!=b a=b"),
        vec![
            "Ident(a)", "EqEqEq", "Ident(b)", "Ident(a)", "BangEqEq", "Ident(b)", "Ident(a)",
            "EqEq", "Ident(b)", "Ident(a)", "BangEq", "Ident(b)", "Ident(a)", "Eq", "Ident(b)",
            "Eof"
        ]
    );
    assert_eq!(show("!=="), vec!["BangEqEq", "Eof"]);
    assert_eq!(show("!= ="), vec!["BangEq", "Eq", "Eof"]);
    assert_eq!(show("!==="), vec!["BangEqEq", "Eq", "Eof"]);
    assert_eq!(spans("a === b"), vec![(0, 1), (2, 5), (6, 7), (7, 7)]);
    assert_eq!(spans("a !== b"), vec![(0, 1), (2, 5), (6, 7), (7, 7)]);
    assert_eq!(show("< ="), vec!["Lt", "Eq", "Eof"]);
    assert_eq!(show("<<"), vec!["Lt", "Lt", "Eof"]);
    assert_eq!(spans("a <= b"), vec![(0, 1), (2, 4), (5, 6), (6, 6)]);
}

#[test]
fn a_lone_bang_is_rejected() {
    assert_eq!(
        error("!x"),
        ("unexpected character `!`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("a ! = b"),
        ("unexpected character `!`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(show("a != b")[1], "BangEq");
}

#[test]
fn comments_are_skipped_and_their_newlines_kept() {
    assert_eq!(
        show("# leading\n1 # one\r\n+ # plus\n2 #"),
        vec![
            "Newline", "Int(1)", "Newline", "Plus", "Newline", "Int(2)", "Eof"
        ]
    );
}

#[test]
fn attributes_are_tokens_and_every_other_hash_a_comment() {
    assert_eq!(
        show("#![no_prelude]\n#[shadowable] a = 1\n#[ shadowable\t]"),
        vec![
            "InnerAttribute(no_prelude)",
            "Newline",
            "Attribute(shadowable)",
            "Ident(a)",
            "Eq",
            "Int(1)",
            "Newline",
            "Attribute(shadowable)",
            "Eof"
        ]
    );
    assert_eq!(spans("#[shadowable] a"), vec![(0, 13), (14, 15), (15, 15)]);
    // A shebang, a `#` with a space before its `[`, and anything not of
    // the shape, are comments to the end of the line.
    for comment in [
        "#!/usr/bin/env gcsl",
        "# [x]",
        "#! [x]",
        "#[x y]",
        "#[1]",
        "#[",
        "#[x",
        "#[]",
        "#!x",
    ] {
        let text = format!("{comment} a\n1");
        assert_eq!(show(&text), vec!["Newline", "Int(1)", "Eof"], "{comment}");
    }
    // An attribute inside an interpolation is skipped by the scan as the
    // lexer lexes it, not as a comment to the end of the line.
    assert_eq!(
        show("\"{ {#[a] x = 1} }\""),
        vec![
            "StrStart(\"\")",
            "LBrace",
            "Attribute(a)",
            "Ident(x)",
            "Eq",
            "Int(1)",
            "RBrace",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
}

#[test]
fn each_newline_is_a_token() {
    // A `\r\n` is normalised to `\n` when the source is loaded.
    let tokens = lex("1\n\n2\r\n").unwrap();
    let kinds: Vec<(TokenKind, Span)> = tokens.iter().map(|t| (t.kind, t.span)).collect();
    assert_eq!(
        kinds,
        vec![
            (TokenKind::Int(1), Span::new(0, 1)),
            (TokenKind::Newline, Span::new(1, 2)),
            (TokenKind::Newline, Span::new(2, 3)),
            (TokenKind::Int(2), Span::new(3, 4)),
            (TokenKind::Newline, Span::new(4, 5)),
            (TokenKind::Eof, Span::new(4, 4)),
        ]
    );
}

#[test]
fn newlines_are_suppressed_inside_parentheses() {
    assert_eq!(
        show("(1 +\n(2\n)\n)\n3"),
        vec![
            "LParen", "Int(1)", "Plus", "LParen", "Int(2)", "RParen", "RParen", "Newline",
            "Int(3)", "Eof"
        ]
    );
    // Braces do not cause the suppression, and a block inside the
    // parentheses is newline-sensitive again, as is a block inside a
    // block inside them; parentheses inside that suppress again.
    assert_eq!(
        show("({\n1\n})\n{\n2\n}"),
        vec![
            "LParen", "LBrace", "Newline", "Int(1)", "Newline", "RBrace", "RParen", "Newline",
            "LBrace", "Newline", "Int(2)", "Newline", "RBrace", "Eof"
        ]
    );
    assert_eq!(
        show("f(fn(): Int {\nlet y = (1 +\n2)\ny\n})"),
        vec![
            "Ident(f)",
            "LParen",
            "Fn",
            "LParen",
            "RParen",
            "Colon",
            "Ident(Int)",
            "LBrace",
            "Newline",
            "Let",
            "Ident(y)",
            "Eq",
            "LParen",
            "Int(1)",
            "Plus",
            "Int(2)",
            "RParen",
            "Newline",
            "Ident(y)",
            "Newline",
            "RBrace",
            "RParen",
            "Eof"
        ]
    );
    // A comment inside parentheses ends at a suppressed newline.
    assert_eq!(
        show("(1 # one\n)"),
        vec!["LParen", "Int(1)", "RParen", "Eof"]
    );
    // An unmatched `)` closes nothing: not the `{` it is inside, and
    // nothing at the top level.
    assert_eq!(show(")\n1"), vec!["RParen", "Newline", "Int(1)", "Eof"]);
    assert_eq!(
        show("{ )\n1 }"),
        vec!["LBrace", "RParen", "Newline", "Int(1)", "RBrace", "Eof"]
    );
    // A `}` closes the nearest `{`, with a `(` left open inside it.
    assert_eq!(
        show("{ (\n1 }\n2"),
        vec![
            "LBrace", "LParen", "Int(1)", "RBrace", "Newline", "Int(2)", "Eof"
        ]
    );
    assert_eq!(show("}\n1"), vec!["RBrace", "Newline", "Int(1)", "Eof"]);
}

#[test]
fn newlines_in_interpolations_follow_the_brackets() {
    // Directly inside an interpolation, and inside parentheses there,
    // newlines are trivia; inside a block there they are tokens.
    assert_eq!(
        show("\"{\n1\n}\""),
        vec!["StrStart(\"\")", "Int(1)", "StrEnd(\"\")", "Eof"]
    );
    assert_eq!(
        show("\"{(\n1\n)}\""),
        vec![
            "StrStart(\"\")",
            "LParen",
            "Int(1)",
            "RParen",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
    assert_eq!(
        show("\"{ {\n1\n} }\""),
        vec![
            "StrStart(\"\")",
            "LBrace",
            "Newline",
            "Int(1)",
            "Newline",
            "RBrace",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
    // A `(` left open inside an interpolation does not suppress the
    // newlines after the literal, nor does one inside a block; and a
    // `)` inside an interpolation does not close a `(` around the
    // literal.
    assert_eq!(
        show("(\"{)}\"\n2)\n3"),
        vec![
            "LParen",
            "StrStart(\"\")",
            "RParen",
            "StrEnd(\"\")",
            "Int(2)",
            "RParen",
            "Newline",
            "Int(3)",
            "Eof"
        ]
    );
    assert_eq!(
        show("\"{(1}\"\n2"),
        vec![
            "StrStart(\"\")",
            "LParen",
            "Int(1)",
            "StrEnd(\"\")",
            "Newline",
            "Int(2)",
            "Eof"
        ]
    );
    assert_eq!(
        show("({ (1 }\n2)\n3"),
        vec![
            "LParen", "LBrace", "LParen", "Int(1)", "RBrace", "Int(2)", "RParen", "Newline",
            "Int(3)", "Eof"
        ]
    );
}

#[test]
fn semicolons_and_braces() {
    assert_eq!(
        show("{;;}"),
        vec!["LBrace", "Semi", "Semi", "RBrace", "Eof"]
    );
}

#[test]
fn colon_eq_is_one_token_only_when_adjacent() {
    assert_eq!(show("x := 1")[1], "ColonEq");
    assert_eq!(show("x:=1")[1], "ColonEq");
    assert_eq!(
        show("x : = 1"),
        vec!["Ident(x)", "Colon", "Eq", "Int(1)", "Eof"]
    );
    assert_eq!(show("x: Int")[1], "Colon");
}

#[test]
fn identifiers() {
    assert_eq!(
        show("hello-world x-1 a_b2 _ _1 Int a-b-c"),
        vec![
            "Ident(hello-world)",
            "Ident(x-1)",
            "Ident(a_b2)",
            "Ident(_)",
            "Ident(_1)",
            "Ident(Int)",
            "Ident(a-b-c)",
            "Eof"
        ]
    );
    let tokens = lex("  hello-world ").unwrap();
    assert_eq!(tokens[0].span, Span::new(2, 13));
}

#[test]
fn keywords_are_whole_words() {
    assert_eq!(
        show("let letter le lets let-x Let"),
        vec![
            "Let",
            "Ident(letter)",
            "Ident(le)",
            "Ident(lets)",
            "Ident(let-x)",
            "Ident(Let)",
            "Eof"
        ]
    );
    assert_eq!(
        show("if iffy else-if true_ False nota and-or or not"),
        vec![
            "If",
            "Ident(iffy)",
            "Ident(else-if)",
            "Ident(true_)",
            "Ident(False)",
            "Ident(nota)",
            "Ident(and-or)",
            "Or",
            "Not",
            "Eof"
        ]
    );
}

#[test]
fn r_is_an_identifier_unless_a_quote_follows() {
    assert_eq!(show("r + 1"), vec!["Ident(r)", "Plus", "Int(1)", "Eof"]);
    assert_eq!(show("r"), vec!["Ident(r)", "Eof"]);
    assert_eq!(show("r \"a\""), vec!["Ident(r)", "Str(\"a\")", "Eof"]);
    // Inside a longer identifier, `r` is part of it and the quote starts
    // an ordinary literal.
    assert_eq!(
        show("bar\"\\n\""),
        vec!["Ident(bar)", "Str(\"\\n\")", "Eof"]
    );
    assert_eq!(show("r\"\\n\""), vec!["Str(\"\\\\n\")", "Eof"]);
}

#[test]
fn interning_gives_equal_names_for_equal_text() {
    let (tokens, names, _) =
        tokenize(&SourceFile::new("test.gcsl", "x y x-1 x y").unwrap()).unwrap();
    let name = |i: usize| match tokens[i].kind {
        TokenKind::Ident(name) => name,
        other => panic!("expected an identifier, got {other:?}"),
    };
    assert_eq!(name(0), name(3));
    assert_eq!(name(1), name(4));
    assert_ne!(name(0), name(1));
    assert_ne!(name(0), name(2));
    assert_eq!(names.len(), 3);
}

#[test]
fn tokenize_with_shares_the_name_table_but_not_the_string_table() {
    let mut names = Names::new();
    let (first, first_strings) =
        tokenize_with(&SourceFile::new("a", "x \"s\"").unwrap(), &mut names).unwrap();
    let (second, second_strings) =
        tokenize_with(&SourceFile::new("b", "y x").unwrap(), &mut names).unwrap();
    assert_eq!(first[0].kind, second[1].kind);
    assert_eq!(names.len(), 2);
    assert_eq!(first_strings.len(), 1);
    assert!(second_strings.is_empty());
}

#[test]
fn a_trailing_hyphen_is_not_part_of_an_identifier() {
    // `a-` is `a` followed by a `-` directly after an operand.
    assert_eq!(error("a-").1, Span::new(1, 2));
    assert_eq!(show("a -")[..2], ["Ident(a)", "Minus"]);
}

#[test]
fn minus_after_an_operand_end_is_ambiguous() {
    for (text, at) in [
        ("1-2", 1),
        ("f(x)-1", 4),
        ("(a)-b", 3),
        ("a- b", 1),
        ("2.5-x", 3),
    ] {
        let diagnostic = lex(text).unwrap_err();
        assert_eq!(diagnostic.message, "`-` here is ambiguous", "{text}");
        assert_eq!(diagnostic.span, Span::new(at, at + 1), "{text}");
        assert_eq!(
            diagnostic.notes,
            vec!["write `a - b` with spaces for subtraction, or `a-b` for a single name"],
            "{text}"
        );
    }
}

#[test]
fn minus_followed_by_whitespace_is_subtraction() {
    assert_eq!(show("a - b")[1], "Minus");
    assert_eq!(show("a -\n b")[1..3], ["Minus", "Newline"]);
    assert_eq!(show("a -\tb")[1], "Minus");
    assert_eq!(show("a -\r\nb")[1], "Minus");
    assert_eq!(show("1 -"), vec!["Int(1)", "Minus", "Eof"]);
    assert_eq!(show("-"), vec!["Minus", "Eof"]);
    // After punctuation, a spaced `-` is still subtraction.
    assert_eq!(show("(- 1)")[1], "Minus");
    assert_eq!(show("x =- 1")[2], "Minus");
}

#[test]
fn minus_followed_by_an_operand_is_negation() {
    assert_eq!(show("2 * -3")[2], "Neg");
    assert_eq!(show("x := -1")[2], "Neg");
    assert_eq!(show("= -1")[1], "Neg");
    assert_eq!(show("(-1)")[1], "Neg");
    assert_eq!(show("-1")[0], "Neg");
    assert_eq!(show("1\n-x")[2], "Neg");
    assert_eq!(show("-_")[0], "Neg");
    assert_eq!(show("-(1)")[0], "Neg");
    assert_eq!(show("-{1}")[0], "Neg");
    assert_eq!(show("-\"a\"")[..2], ["Neg", "Str(\"a\")"]);
    assert_eq!(show("a -b"), vec!["Ident(a)", "Neg", "Ident(b)", "Eof"]);
    // Directly after punctuation other than `)`.
    assert_eq!(show("x=-1")[1..3], ["Eq", "Neg"]);
    assert_eq!(show("1+-1")[1..3], ["Plus", "Neg"]);
    assert_eq!(show("x:-1")[1..3], ["Colon", "Neg"]);
    assert_eq!(show(";-1")[1], "Neg");
    assert_eq!(show("a<-1")[1..3], ["Lt", "Neg"]);
}

#[test]
fn minus_followed_by_anything_else_is_unexpected() {
    let expected = |at: u32| ("unexpected character `-`".to_owned(), Span::new(at, at + 1));
    assert_eq!(error("--x"), expected(0));
    assert_eq!(error("x --1"), expected(2));
    assert_eq!(error("-=1"), expected(0));
    assert_eq!(error("x = -=1"), expected(4));
    assert_eq!(error("1 + -.5"), expected(4));
    assert_eq!(error("a -# comment"), expected(2));
}

#[test]
fn number_literals_never_absorb_a_minus() {
    assert_eq!(error("1-2").1, Span::new(1, 2));
    assert_eq!(error("1.5-2").1, Span::new(3, 4));
    // The sign of an exponent is part of the literal.
    assert_eq!(kinds("1e-2"), vec![TokenKind::Float(1e-2), TokenKind::Eof]);
}

#[test]
fn underscore_separators() {
    assert_eq!(kinds("1_000_000")[0], TokenKind::Int(1_000_000));
    assert_eq!(kinds("1_0.2_5")[0], TokenKind::Float(10.25));
    assert_eq!(kinds("1e1_0")[0], TokenKind::Float(1e10));
    assert_eq!(kinds("1_2.3_4e-1_0")[0], TokenKind::Float(12.34e-10));
    assert_eq!(lex("1_000").unwrap()[0].span, Span::new(0, 5));
}

#[test]
fn scratch_buffer_is_reset_between_literals() {
    assert_eq!(
        kinds("1_2 + 3_4"),
        vec![
            TokenKind::Int(12),
            TokenKind::Plus,
            TokenKind::Int(34),
            TokenKind::Eof
        ]
    );
    // And between numbers and string pieces, which share it.
    assert_eq!(
        show("1_2 \"ab\" 3_4 \"c\""),
        vec!["Int(12)", "Str(\"ab\")", "Int(34)", "Str(\"c\")", "Eof"]
    );
}

#[test]
fn leading_underscore_is_an_error() {
    let expected = ("leading `_` in number literal".to_owned(), Span::new(2, 3));
    assert_eq!(error("1._5"), expected);
    assert_eq!(error("1e_5").1, Span::new(2, 3));
    assert_eq!(error("1e-_5").1, Span::new(3, 4));
    // At the start of a token, `_` begins an identifier instead.
    assert_eq!(show("_1"), vec!["Ident(_1)", "Eof"]);
}

#[test]
fn trailing_underscore_is_an_error() {
    let message = "trailing `_` in number literal".to_owned();
    assert_eq!(error("1_"), (message.clone(), Span::new(1, 2)));
    assert_eq!(error("1_ + 2"), (message.clone(), Span::new(1, 2)));
    assert_eq!(error("1_.5"), (message.clone(), Span::new(1, 2)));
    assert_eq!(error("1.5_"), (message.clone(), Span::new(3, 4)));
    assert_eq!(error("1_e5"), (message.clone(), Span::new(1, 2)));
    assert_eq!(error("1e5_"), (message, Span::new(3, 4)));
}

#[test]
fn doubled_underscore_is_an_error() {
    let message = "consecutive `_` in number literal".to_owned();
    assert_eq!(error("1__0"), (message.clone(), Span::new(1, 3)));
    assert_eq!(error("1.0__1"), (message, Span::new(3, 5)));
}

#[test]
fn exponent_forms() {
    assert_eq!(kinds("2.0e10")[0], TokenKind::Float(2.0e10));
    assert_eq!(kinds("1.5e-3")[0], TokenKind::Float(1.5e-3));
    assert_eq!(kinds("1.5e+3")[0], TokenKind::Float(1.5e3));
    assert_eq!(kinds("1e10")[0], TokenKind::Float(1e10));
    assert_eq!(kinds("0e0")[0], TokenKind::Float(0.0));
    assert_eq!(lex("1.5e-3").unwrap()[0].span, Span::new(0, 6));
}

#[test]
fn missing_exponent_digits_is_an_error() {
    let message = "missing digits in exponent".to_owned();
    assert_eq!(error("1e"), (message.clone(), Span::new(1, 2)));
    assert_eq!(error("1.5e+"), (message.clone(), Span::new(3, 5)));
    assert_eq!(error("1e-x"), (message, Span::new(1, 3)));
}

#[test]
fn uppercase_exponent_is_accepted() {
    assert_eq!(kinds("1E5")[0], TokenKind::Float(1e5));
    assert_eq!(kinds("2.5E-1")[0], TokenKind::Float(0.25));
    assert_eq!(
        error("1E"),
        ("missing digits in exponent".to_owned(), Span::new(1, 2))
    );
}

#[test]
fn dot_without_digits_on_both_sides_is_not_a_float() {
    assert_eq!(
        kinds("1."),
        vec![TokenKind::Int(1), TokenKind::Dot, TokenKind::Eof]
    );
    assert_eq!(
        kinds(".5"),
        vec![TokenKind::Dot, TokenKind::Int(5), TokenKind::Eof]
    );
    assert_eq!(spans("1."), vec![(0, 1), (1, 2), (2, 2)]);
}

#[test]
fn dots_are_field_access() {
    assert_eq!(show("a.b"), vec!["Ident(a)", "Dot", "Ident(b)", "Eof"]);
    assert_eq!(spans("a.b"), vec![(0, 1), (1, 2), (2, 3), (3, 3)]);
    assert_eq!(
        show("a.\"b c\""),
        vec!["Ident(a)", "Dot", "Str(\"b c\")", "Eof"]
    );
    assert_eq!(
        show("a . b.c"),
        vec!["Ident(a)", "Dot", "Ident(b)", "Dot", "Ident(c)", "Eof"]
    );
    // A `.` between digits is part of a float; anywhere else it is not.
    assert_eq!(kinds("1.5"), vec![TokenKind::Float(1.5), TokenKind::Eof]);
    assert_eq!(show("1.x"), vec!["Int(1)", "Dot", "Ident(x)", "Eof"]);
    assert_eq!(show("x.1"), vec!["Ident(x)", "Dot", "Int(1)", "Eof"]);
    assert_eq!(show("x.1.5"), vec!["Ident(x)", "Dot", "Float(1.5)", "Eof"]);
    assert_eq!(show("a-b.c"), vec!["Ident(a-b)", "Dot", "Ident(c)", "Eof"]);
    // Inside an interpolation too.
    assert_eq!(
        show("\"{a.b}\""),
        vec![
            "StrStart(\"\")",
            "Ident(a)",
            "Dot",
            "Ident(b)",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
    assert_eq!(TokenKind::Dot.symbol(), Some("."));
    assert_eq!(TokenKind::Comma.symbol(), Some(","));
    assert_eq!(
        show("{a: Int,\nb: {}}"),
        vec![
            "LBrace",
            "Ident(a)",
            "Colon",
            "Ident(Int)",
            "Comma",
            "Newline",
            "Ident(b)",
            "Colon",
            "LBrace",
            "RBrace",
            "RBrace",
            "Eof"
        ]
    );
}

#[test]
fn plain_identifiers() {
    for text in ["x", "_", "_1", "hello-world", "x-1", "Int", "r", "lets"] {
        assert!(is_identifier(text), "{text}");
    }
    for text in [
        "", "1x", "a b", "a-", "-a", "a--b", "é", "let", "if", "else", "true", "false", "and",
        "or", "not", "a.b", "fn", "def", "import", "lazy", "await", "a|b",
    ] {
        assert!(!is_identifier(text), "{text}");
    }
}

#[test]
fn integer_limits() {
    assert_eq!(kinds("9223372036854775807")[0], TokenKind::Int(i64::MAX));
    let diagnostic = lex("1 + 9223372036854775808").unwrap_err();
    assert_eq!(diagnostic.message, "integer literal out of range for Int");
    assert_eq!(diagnostic.span, Span::new(4, 23));
    assert_eq!(
        diagnostic.notes,
        vec!["the largest Int literal is 9223372036854775807"]
    );
    let (message, span) = error("99_999_999_999_999_999_999");
    assert_eq!(message, "integer literal out of range for Int");
    assert_eq!(span, Span::new(0, 26));
}

#[test]
fn float_literal_limits() {
    assert_eq!(
        kinds("1.7976931348623157e308")[0],
        TokenKind::Float(f64::MAX)
    );
    let diagnostic = lex("1 + 1e400").unwrap_err();
    assert_eq!(diagnostic.message, "float literal out of range for Float");
    assert_eq!(diagnostic.span, Span::new(4, 9));
    assert_eq!(
        diagnostic.notes,
        vec!["the largest Float literal is 1.7976931348623157e308"]
    );
    // Underflow is not an error: the literal rounds to zero.
    assert_eq!(kinds("1e-400")[0], TokenKind::Float(0.0));
}

#[test]
fn unexpected_characters_are_reported_whole() {
    assert_eq!(
        error("1 + $"),
        ("unexpected character `$`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("1 € 2"),
        ("unexpected character `€`".to_owned(), Span::new(2, 5))
    );
    // Identifiers are ASCII only.
    assert_eq!(
        error("café"),
        ("unexpected character `é`".to_owned(), Span::new(3, 5))
    );
    assert_eq!(
        error("\u{0}"),
        ("unexpected character `\\0`".to_owned(), Span::new(0, 1))
    );
    // Non-ASCII text in a comment is fine.
    assert_eq!(kinds("# café\n")[0], TokenKind::Newline);
}

#[test]
fn symbols() {
    assert_eq!(TokenKind::SlashSlash.symbol(), Some("//"));
    assert_eq!(TokenKind::ColonEq.symbol(), Some(":="));
    assert_eq!(TokenKind::Neg.symbol(), Some("-"));
    assert_eq!(TokenKind::Let.symbol(), Some("let"));
    assert_eq!(TokenKind::Not.symbol(), Some("not"));
    assert_eq!(TokenKind::BangEq.symbol(), Some("!="));
    assert_eq!(TokenKind::EqEqEq.symbol(), Some("==="));
    assert_eq!(TokenKind::BangEqEq.symbol(), Some("!=="));
    assert_eq!(TokenKind::Ge.symbol(), Some(">="));
    assert_eq!(TokenKind::Int(1).symbol(), None);
    assert_eq!(TokenKind::Newline.symbol(), None);
    assert_eq!(TokenKind::Eof.symbol(), None);
    let (tokens, _, _) = tokenize(&SourceFile::new("t", "\"a\"").unwrap()).unwrap();
    assert_eq!(tokens[0].kind.symbol(), None);
}

// ----- Lists -----

#[test]
fn brackets_and_spreads_are_tokens() {
    assert_eq!(
        show("[1, ...xs][0]"),
        [
            "LBracket",
            "Int(1)",
            "Comma",
            "DotDotDot",
            "Ident(xs)",
            "RBracket",
            "LBracket",
            "Int(0)",
            "RBracket",
            "Eof",
        ]
    );
    assert_eq!(spans("[...a]"), [(0, 1), (1, 4), (4, 5), (5, 6), (6, 6)]);
    assert_eq!(TokenKind::LBracket.symbol(), Some("["));
    assert_eq!(TokenKind::RBracket.symbol(), Some("]"));
    assert_eq!(TokenKind::DotDotDot.symbol(), Some("..."));
}

#[test]
fn three_dots_are_one_token_two_a_range_and_one_field_access() {
    use TokenKind::{Dot, DotDot, DotDotDot, Eof};
    assert_eq!(kinds("..."), [DotDotDot, Eof]);
    assert_eq!(kinds(".."), [DotDot, Eof]);
    assert_eq!(kinds("."), [Dot, Eof]);
    assert_eq!(kinds("...."), [DotDotDot, Dot, Eof]);
    assert_eq!(kinds("......"), [DotDotDot, DotDotDot, Eof]);
    assert_eq!(kinds(". .."), [Dot, DotDot, Eof]);
    // A number literal takes a `.` only between two digits.
    assert_eq!(show("1...x"), ["Int(1)", "DotDotDot", "Ident(x)", "Eof"]);
    assert_eq!(show("1.5..."), ["Float(1.5)", "DotDotDot", "Eof"]);
}

// ----- Ranges and `for` -----

#[test]
fn two_dots_are_a_range_and_two_and_an_equals_sign_an_inclusive_one() {
    use TokenKind::{Dot, DotDot, DotDotDot, DotDotEq, Eof, Eq, EqEq};
    assert_eq!(kinds(".."), [DotDot, Eof]);
    assert_eq!(kinds("..="), [DotDotEq, Eof]);
    assert_eq!(kinds("..=="), [DotDotEq, Eq, Eof]);
    assert_eq!(kinds(".. ="), [DotDot, Eq, Eof]);
    assert_eq!(kinds(". .="), [Dot, Dot, Eq, Eof]);
    assert_eq!(kinds("...="), [DotDotDot, Eq, Eof]);
    assert_eq!(kinds(".. =="), [DotDot, EqEq, Eof]);
    // A number takes neither dot of a range after it: a `.` not followed
    // by a digit is not part of a number.
    assert_eq!(show("0..10"), ["Int(0)", "DotDot", "Int(10)", "Eof"]);
    assert_eq!(show("0..=10"), ["Int(0)", "DotDotEq", "Int(10)", "Eof"]);
    assert_eq!(show("1.5..2"), ["Float(1.5)", "DotDot", "Int(2)", "Eof"]);
    assert_eq!(show("1.5..=2"), ["Float(1.5)", "DotDotEq", "Int(2)", "Eof"]);
    assert_eq!(show("0...5"), ["Int(0)", "DotDotDot", "Int(5)", "Eof"]);
    assert_eq!(show("a..b"), ["Ident(a)", "DotDot", "Ident(b)", "Eof"]);
    assert_eq!(show("a..=b"), ["Ident(a)", "DotDotEq", "Ident(b)", "Eof"]);
    assert_eq!(show("0 .. 1"), ["Int(0)", "DotDot", "Int(1)", "Eof"]);
    assert_eq!(show("0 ..= 1"), ["Int(0)", "DotDotEq", "Int(1)", "Eof"]);
    assert_eq!(
        show("x.a..y.b"),
        [
            "Ident(x)", "Dot", "Ident(a)", "DotDot", "Ident(y)", "Dot", "Ident(b)", "Eof"
        ]
    );
    // A `-` after a range is a negation.
    assert_eq!(show("0..-1"), ["Int(0)", "DotDot", "Neg", "Int(1)", "Eof"]);
    assert_eq!(
        show("-1..=-1"),
        ["Neg", "Int(1)", "DotDotEq", "Neg", "Int(1)", "Eof"]
    );
    assert_eq!(spans("0..10"), [(0, 1), (1, 3), (3, 5), (5, 5)]);
    assert_eq!(spans("0..=10"), [(0, 1), (1, 4), (4, 6), (6, 6)]);
    assert_eq!(TokenKind::DotDot.symbol(), Some(".."));
    assert_eq!(TokenKind::DotDotEq.symbol(), Some("..="));
}

#[test]
fn for_and_in_are_keywords_only_as_whole_words() {
    assert_eq!(
        show("for in for-x in-x fore into For In"),
        vec![
            "For",
            "In",
            "Ident(for-x)",
            "Ident(in-x)",
            "Ident(fore)",
            "Ident(into)",
            "Ident(For)",
            "Ident(In)",
            "Eof"
        ]
    );
    assert_eq!(
        show("for x in xs { x }"),
        [
            "For",
            "Ident(x)",
            "In",
            "Ident(xs)",
            "LBrace",
            "Ident(x)",
            "RBrace",
            "Eof"
        ]
    );
    assert!(!is_identifier("for"));
    assert!(!is_identifier("in"));
    assert!(is_identifier("for-x"));
    assert!(is_identifier("into"));
    assert_eq!(TokenKind::For.symbol(), Some("for"));
    assert_eq!(TokenKind::In.symbol(), Some("in"));
}

#[test]
fn do_break_and_continue_are_keywords_only_as_whole_words() {
    assert_eq!(
        show("do break continue do-x break-x continue-x done breaks Do _do"),
        vec![
            "Do",
            "Break",
            "Continue",
            "Ident(do-x)",
            "Ident(break-x)",
            "Ident(continue-x)",
            "Ident(done)",
            "Ident(breaks)",
            "Ident(Do)",
            "Ident(_do)",
            "Eof"
        ]
    );
    for keyword in ["do", "break", "continue"] {
        assert!(!is_identifier(keyword), "{keyword}");
    }
    assert_eq!(TokenKind::Do.symbol(), Some("do"));
    assert_eq!(TokenKind::Break.symbol(), Some("break"));
    assert_eq!(TokenKind::Continue.symbol(), Some("continue"));
}

#[test]
fn return_is_a_keyword_only_as_a_whole_word() {
    assert_eq!(
        show("return return-x returns Return _return"),
        vec![
            "Return",
            "Ident(return-x)",
            "Ident(returns)",
            "Ident(Return)",
            "Ident(_return)",
            "Eof"
        ]
    );
    assert!(!is_identifier("return"));
    assert!(is_identifier("return-x"));
    assert_eq!(TokenKind::Return.symbol(), Some("return"));
    // A label is written directly after it, and after a `{`.
    assert_eq!(
        with_labels("{'pick return'pick 1 }"),
        [
            "LBrace", "'pick", "Return", "'pick", "Int(1)", "RBrace", "Eof"
        ]
    );
    assert_eq!(spans("{'a"), [(0, 1), (1, 3), (3, 3)]);
}

/// The tokens of `text`, a label shown as `'` and its name, and every
/// other token by its kind.
fn with_labels(text: &str) -> Vec<String> {
    let (tokens, names, _) = tokenize(&SourceFile::new("test.gcsl", text).unwrap()).unwrap();
    tokens
        .iter()
        .map(|token| match token.kind {
            TokenKind::Label(name) => format!("'{}", names.get(name)),
            TokenKind::Ident(name) => names.get(name).to_owned(),
            kind => format!("{kind:?}"),
        })
        .collect()
}

#[test]
fn a_label_is_a_quote_and_a_name() {
    assert_eq!(
        with_labels("'a '_ 'a-b 'x1 'for"),
        ["'a", "'_", "'a-b", "'x1", "'for", "Eof"]
    );
    // Written directly after its keyword, the label is a token of its
    // own, spanning its quote and its name.
    assert_eq!(
        with_labels("for'rows r in rows { break'rows }"),
        [
            "For", "'rows", "r", "In", "rows", "LBrace", "Break", "'rows", "RBrace", "Eof"
        ]
    );
    assert_eq!(spans("for'a"), [(0, 3), (3, 5), (5, 5)]);
    assert_eq!(spans("do'a-b {"), [(0, 2), (2, 6), (7, 8), (8, 8)]);
    // A label has no fixed spelling.
    assert_eq!(kinds("'a")[0].symbol(), None);
    // A label's name is interned as an identifier's is.
    let (tokens, names, _) = tokenize(&SourceFile::new("test.gcsl", "'a a").unwrap()).unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Label(names.lookup("a").unwrap()));
    assert_eq!(tokens[1].kind, TokenKind::Ident(names.lookup("a").unwrap()));
}

#[test]
fn a_stray_quote_is_an_unexpected_character() {
    assert_eq!(
        error("'"),
        ("unexpected character `\\'`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("break ' a"),
        ("unexpected character `\\'`".to_owned(), Span::new(6, 7))
    );
    assert_eq!(
        error("'1"),
        ("unexpected character `\\'`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("'-a"),
        ("unexpected character `\\'`".to_owned(), Span::new(0, 1))
    );
}

#[test]
fn mut_is_a_keyword_only_as_a_whole_word() {
    assert_eq!(
        show("mut mut-x mutable muts Mut _mut"),
        vec![
            "Mut",
            "Ident(mut-x)",
            "Ident(mutable)",
            "Ident(muts)",
            "Ident(Mut)",
            "Ident(_mut)",
            "Eof"
        ]
    );
    assert_eq!(
        show("let mut x = 1"),
        ["Let", "Mut", "Ident(x)", "Eq", "Int(1)", "Eof"]
    );
    assert!(!is_identifier("mut"));
    assert!(is_identifier("mut-x"));
    assert!(is_identifier("mutable"));
    assert_eq!(TokenKind::Mut.symbol(), Some("mut"));
    // A keyword is a token of its own, at its own span.
    assert_eq!(spans("let mut x"), vec![(0, 3), (4, 7), (8, 9), (9, 9)]);
}

#[test]
fn newlines_are_suppressed_inside_brackets() {
    assert_eq!(
        show("[1,\n2\n]\nx"),
        [
            "LBracket", "Int(1)", "Comma", "Int(2)", "RBracket", "Newline", "Ident(x)", "Eof"
        ]
    );
    // Inside an index too.
    assert_eq!(
        show("xs[\n0\n]"),
        ["Ident(xs)", "LBracket", "Int(0)", "RBracket", "Eof"]
    );
    // A `]` closes only a `[` on top: an unmatched one closes nothing.
    assert_eq!(
        show("(]\n)\nx"),
        ["LParen", "RBracket", "RParen", "Newline", "Ident(x)", "Eof"]
    );
}

#[test]
fn a_block_inside_brackets_is_newline_sensitive() {
    assert_eq!(
        show("[{\na = 1\n}\n, 2]\nx"),
        [
            "LBracket", "LBrace", "Newline", "Ident(a)", "Eq", "Int(1)", "Newline", "RBrace",
            "Comma", "Int(2)", "RBracket", "Newline", "Ident(x)", "Eof"
        ]
    );
    // A `}` closes a `[` left open inside its block.
    assert_eq!(
        show("{[\n}\nx"),
        ["LBrace", "LBracket", "RBrace", "Newline", "Ident(x)", "Eof"]
    );
}

#[test]
fn a_minus_next_to_brackets_is_classified_as_next_to_parentheses() {
    assert_eq!(
        error("xs[0]-1"),
        ("`-` here is ambiguous".to_owned(), Span::new(5, 6))
    );
    assert_eq!(
        show("-[1]"),
        ["Neg", "LBracket", "Int(1)", "RBracket", "Eof"]
    );
    assert_eq!(
        show("xs[0] - 1"),
        [
            "Ident(xs)",
            "LBracket",
            "Int(0)",
            "RBracket",
            "Minus",
            "Int(1)",
            "Eof"
        ]
    );
}

// ----- String literals -----

#[test]
fn delimiters() {
    assert_eq!(string("\"\""), "");
    assert_eq!(string("\"a\""), "a");
    assert_eq!(string("\"\"\"a\"b\"\"\""), "a\"b");
    assert_eq!(string("\"\"\"\"a\"\"\"b\"\"\"\""), "a\"\"\"b");
    assert_eq!(string("\"\"\"a\"\"b\"\"\""), "a\"\"b");
    // Non-ASCII content.
    assert_eq!(string("\"héllo 😀\""), "héllo 😀");
    // Strings may span lines.
    assert_eq!(string("\"a\nb\""), "a\nb");
}

#[test]
fn the_first_n_quotes_of_a_longer_run_close_the_literal() {
    // `"""a"""` then `""`, the empty string.
    assert_eq!(
        show("\"\"\"a\"\"\"\"\""),
        vec!["Str(\"a\")", "Str(\"\")", "Eof"]
    );
    // `""` then `"b"`.
    assert_eq!(show("\"\" \"b\""), vec!["Str(\"\")", "Str(\"b\")", "Eof"]);
    // `"a"` then `"`, unterminated.
    assert_eq!(
        error("\"a\"\""),
        ("unterminated string literal".to_owned(), Span::new(3, 4))
    );
}

#[test]
fn raw_strings_keep_every_byte() {
    assert_eq!(string("r\"\\n{x}\""), "\\n{x}");
    assert_eq!(string("r\"}{ \\ \\u{41}\""), "}{ \\ \\u{41}");
    assert_eq!(string("r\"\""), "");
    // A backslash cannot escape the closing quote.
    assert_eq!(string("r\"a\\\""), "a\\");
    assert_eq!(string("r\"\"\"say \"hi\"\\n\"\"\""), "say \"hi\"\\n");
}

#[test]
fn every_escape() {
    assert_eq!(string(r#""\n\r\t\\\"""#), "\n\r\t\\\"");
    assert_eq!(string(r#""a\"b""#), "a\"b");
    assert_eq!(string("\"\"\"\\\"\"\"\""), "\"");
    assert_eq!(string(r#""{{}}""#), "{}");
    assert_eq!(string(r#""{{x}} {{{{""#), "{x} {{");
}

#[test]
fn unknown_escapes() {
    assert_eq!(
        error(r#"x = "a\qb""#),
        ("unknown escape `\\q`".to_owned(), Span::new(6, 8))
    );
    assert_eq!(
        error(r#""\{""#),
        ("unknown escape `\\{`".to_owned(), Span::new(1, 3))
    );
    assert_eq!(
        error(r#""\'""#),
        ("unknown escape `\\'`".to_owned(), Span::new(1, 3))
    );
    // A multi-byte character is shown and spanned whole.
    assert_eq!(
        error("\"\\é\""),
        ("unknown escape `\\é`".to_owned(), Span::new(1, 4))
    );
    // A control character is shown by its escape.
    assert_eq!(
        error("\"\\\t\""),
        ("unknown escape `\\\\t`".to_owned(), Span::new(1, 3))
    );
}

#[test]
fn unicode_escapes() {
    assert_eq!(string(r#""\u{41}\u{e9}\u{1F600}""#), "Aé😀");
    assert_eq!(string(r#""\u{0}\u{000041}\u{10FFFF}""#), "\0A\u{10FFFF}");
    let invalid = |text: &str, start: u32, end: u32| {
        assert_eq!(
            error(text),
            ("invalid unicode escape".to_owned(), Span::new(start, end)),
            "{text}"
        );
    };
    invalid(r#""\u{}""#, 1, 5);
    invalid(r#""\u{1234567}""#, 1, 12);
    invalid(r#""\u{D800}""#, 1, 9);
    invalid(r#""\u{110000}""#, 1, 11);
    invalid(r#""\u{zz}""#, 1, 7);
    invalid(r#""a\u41""#, 2, 4);
    invalid(r#""\u{41""#, 1, 6);
    invalid(r#""\u{4_1}""#, 1, 5);
}

#[test]
fn the_braces_of_a_unicode_escape_are_not_an_interpolation() {
    assert_eq!(string(r#""\u{7B}\u{7D}""#), "{}");
    // An invalid escape's braces are not one either: the error is the
    // escape, not an unterminated interpolation.
    assert_eq!(error(r#""\u{x""#).0, "invalid unicode escape");
}

#[test]
fn line_continuations() {
    assert_eq!(string("\"a \\\n     b\""), "a b");
    assert_eq!(string("\"a\\\n\t \tb\""), "ab");
    // Only the next line's leading whitespace goes, not further lines.
    assert_eq!(string("\"a\\\n\nb\""), "a\nb");
    // An escaped backslash before a line break is not a continuation.
    assert_eq!(string("\"a\\\\\nb\""), "a\\\nb");
    // In a raw string it is content.
    assert_eq!(string("r\"a\\\n b\""), "a\\\n b");
}

#[test]
fn simple_interpolation() {
    assert_eq!(
        show("\"a {x} b\""),
        vec!["StrStart(\"a \")", "Ident(x)", "StrEnd(\" b\")", "Eof"]
    );
    assert_eq!(
        show("\"{x}\""),
        vec!["StrStart(\"\")", "Ident(x)", "StrEnd(\"\")", "Eof"]
    );
    assert_eq!(
        show("\"{1 + 2}\" + 3"),
        vec![
            "StrStart(\"\")",
            "Int(1)",
            "Plus",
            "Int(2)",
            "StrEnd(\"\")",
            "Plus",
            "Int(3)",
            "Eof"
        ]
    );
    // `{{` and `}}` next to an interpolation.
    assert_eq!(
        show("\"{{{x}}}\""),
        vec!["StrStart(\"{{\")", "Ident(x)", "StrEnd(\"}}\")", "Eof"]
    );
}

#[test]
fn multiple_interpolations() {
    assert_eq!(
        show("\"{a}, {b} and {c}.\""),
        vec![
            "StrStart(\"\")",
            "Ident(a)",
            "StrMid(\", \")",
            "Ident(b)",
            "StrMid(\" and \")",
            "Ident(c)",
            "StrEnd(\".\")",
            "Eof"
        ]
    );
}

#[test]
fn nested_interpolations() {
    assert_eq!(
        show("\"{ \"{x}\" }\""),
        vec![
            "StrStart(\"\")",
            "StrStart(\"\")",
            "Ident(x)",
            "StrEnd(\"\")",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
    // A `"` or `}` inside a nested literal does not end the outer one.
    assert_eq!(
        show("\"a{\"}}\"}b\""),
        vec!["StrStart(\"a\")", "Str(\"}}\")", "StrEnd(\"b\")", "Eof"]
    );
    assert_eq!(
        show("\"a{r\"\\\"}b\""),
        vec!["StrStart(\"a\")", "Str(\"\\\\\")", "StrEnd(\"b\")", "Eof"]
    );
    // Blocks and braces inside an interpolation.
    assert_eq!(
        show("\"{ {} }\""),
        vec!["StrStart(\"\")", "LBrace", "RBrace", "StrEnd(\"\")", "Eof"]
    );
    assert_eq!(
        show("\"{ { \"}}\" } }x\""),
        vec![
            "StrStart(\"\")",
            "LBrace",
            "Str(\"}}\")",
            "RBrace",
            "StrEnd(\"x\")",
            "Eof"
        ]
    );
}

#[test]
fn interpolations_are_tokenised_as_the_lexer_reads_them() {
    // An `r` ending an identifier or following a number is not a raw
    // prefix only where the lexer agrees.
    assert_eq!(
        show("\"{bar\"\\\"\"}\""),
        vec![
            "StrStart(\"\")",
            "Ident(bar)",
            "Str(\"\\\"\")",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
    assert_eq!(
        show("\"{2r\"\\\"}\""),
        vec![
            "StrStart(\"\")",
            "Int(2)",
            "Str(\"\\\\\")",
            "StrEnd(\"\")",
            "Eof"
        ]
    );
    // A comment runs to the end of its line, braces and quotes included.
    assert_eq!(
        show("\"{x # }\"\n}\""),
        vec!["StrStart(\"\")", "Ident(x)", "StrEnd(\"\")", "Eof"]
    );
}

#[test]
fn newlines_inside_an_interpolation_are_not_emitted() {
    assert_eq!(
        show("\"{\n  a\n  +\n  b\n}\"\n1"),
        vec![
            "StrStart(\"\")",
            "Ident(a)",
            "Plus",
            "Ident(b)",
            "StrEnd(\"\")",
            "Newline",
            "Int(1)",
            "Eof"
        ]
    );
    // Parentheses inside an interpolation do not leak out of it.
    assert_eq!(
        show("\"{(}\"\n1"),
        vec![
            "StrStart(\"\")",
            "LParen",
            "StrEnd(\"\")",
            "Newline",
            "Int(1)",
            "Eof"
        ]
    );
    assert_eq!(
        show("(\"{)}\"\n1"),
        vec![
            "LParen",
            "StrStart(\"\")",
            "RParen",
            "StrEnd(\"\")",
            "Int(1)",
            "Eof"
        ]
    );
}

#[test]
fn unterminated_strings() {
    let diagnostic = lex("x = \"abc").unwrap_err();
    assert_eq!(diagnostic.message, "unterminated string literal");
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert!(diagnostic.notes.is_empty());
    let diagnostic = lex("\"\"\"abc\"\"").unwrap_err();
    assert_eq!(diagnostic.message, "unterminated string literal");
    assert_eq!(diagnostic.span, Span::new(0, 3));
    assert_eq!(diagnostic.notes, vec!["this string closes with 3 quotes"]);
    let diagnostic = lex("r\"\"\"\"x").unwrap_err();
    assert_eq!(diagnostic.span, Span::new(1, 5));
    assert_eq!(diagnostic.notes, vec!["this string closes with 4 quotes"]);
    // An escaped quote does not close it.
    assert_eq!(error("\"a\\\"").1, Span::new(0, 1));
    // The innermost unterminated literal is reported.
    assert_eq!(
        error("\"{ \"abc"),
        ("unterminated string literal".to_owned(), Span::new(3, 4))
    );
}

#[test]
fn unterminated_interpolations() {
    let diagnostic = lex("\"a {x").unwrap_err();
    assert_eq!(diagnostic.message, "unterminated interpolation");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(diagnostic.notes, vec!["write `{{` for a literal `{`"]);
    // A brace inside the interpolation needs its own match.
    assert_eq!(
        error("\"{ { x }"),
        ("unterminated interpolation".to_owned(), Span::new(1, 2))
    );
    // A nested literal that closes leaves the interpolation open.
    assert_eq!(
        error("\"{ \"}\" "),
        ("unterminated interpolation".to_owned(), Span::new(1, 2))
    );
}

#[test]
fn a_stray_close_brace_in_a_string() {
    let diagnostic = lex("\"a } b\"").unwrap_err();
    assert_eq!(diagnostic.message, "unmatched `}` in string");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(diagnostic.notes, vec!["write `}}` for a literal `}`"]);
    assert_eq!(error("\"{x}}\"").1, Span::new(4, 5));
    // Errors inside a literal are reported in source order.
    assert_eq!(error("\"\\q }\"").0, "unknown escape `\\q`");
    assert_eq!(error("\"} \\q\"").0, "unmatched `}` in string");
}

#[test]
fn string_token_spans() {
    assert_eq!(
        spans("x \"ab\" r\"c\""),
        vec![(0, 1), (2, 6), (7, 11), (11, 11)]
    );
    assert_eq!(spans("\"\"\"a\"\"\""), vec![(0, 7), (7, 7)]);
    // `StrStart` through the `{`, `StrMid` from `}` through `{`, `StrEnd`
    // from `}` through the closing delimiter.
    assert_eq!(
        spans("\"ab{x}cd{y}e\""),
        vec![(0, 4), (4, 5), (5, 9), (9, 10), (10, 13), (13, 13)]
    );
    assert_eq!(
        spans("\"\"\"{x}\"\"\""),
        vec![(0, 4), (4, 5), (5, 9), (9, 9)]
    );
}

#[test]
fn nesting_is_limited() {
    fn nested(levels: usize) -> String {
        format!("{}x{}", "\"{".repeat(levels), "}\"".repeat(levels))
    }
    // The scan recurses per level, so give it the stack the pipeline is
    // meant to run with. Lexing the deepest literals whole takes time
    // quadratic in their depth, as each is scanned again when reached,
    // so only the outermost literal's scan, which walks every level, is
    // run here; the end-to-end depth test lexes such literals whole.
    let (deepest, too_deep) = std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(|| {
            let source = SourceFile::new("test.gcsl", nested(MAX_DEPTH)).unwrap();
            let mut names = Names::new();
            let deepest = Lexer::new(&source, &mut names, None)
                .scan(0, 1)
                .map(|layout| layout.end);
            (deepest, lex(&nested(MAX_DEPTH + 1)))
        })
        .unwrap()
        .join()
        .unwrap();
    // The outermost literal is the whole source.
    assert_eq!(deepest.unwrap(), 4 * MAX_DEPTH + 1);
    let diagnostic = too_deep.unwrap_err();
    assert_eq!(diagnostic.message, "expression nested too deeply");
    let at = u32::try_from(2 * MAX_DEPTH).unwrap();
    assert_eq!(diagnostic.span, Span::new(at, at + 1));
    assert_eq!(
        diagnostic.notes,
        vec![format!("the limit is {MAX_DEPTH} levels")]
    );
}

// ----- Indentation stripping -----

#[test]
fn stripping_the_specification_examples() {
    assert_eq!(
        string("\"\n    echo hello\n    echo world\n\""),
        "echo hello\necho world"
    );
    assert_eq!(
        string("\"\n    echo hello \\\n      echo world\n\""),
        "echo hello echo world"
    );
    assert_eq!(string("\"  hello\n   world\""), "  hello\n   world");
    assert_eq!(
        string("\"\"\"\n  She said \"hi\"\n\"\"\""),
        "She said \"hi\""
    );
}

#[test]
fn stripping_as_code_is_usually_indented() {
    let text = "let script = \"\n    set -e\n      make all\n    make install\n  \"";
    let (tokens, _, strings) = tokenize(&SourceFile::new("t", text).unwrap()).unwrap();
    let TokenKind::Str(id) = tokens[3].kind else {
        panic!("expected a string, got {:?}", tokens[3]);
    };
    assert_eq!(strings.get(id), "set -e\n  make all\nmake install");
}

#[test]
fn stripping_with_tabs_compares_bytes() {
    assert_eq!(string("\"\n\t\ta\n\t\t\tb\n\t\""), "a\n\tb");
    // A tab and spaces share no prefix, so nothing is stripped.
    assert_eq!(string("\"\n\ta\n    b\n\""), "\ta\n    b");
    // Mixed, sharing a leading tab.
    assert_eq!(string("\"\n\t a\n\t\tb\n\""), " a\n\tb");
}

#[test]
fn blank_lines_do_not_count_and_lose_the_indentation() {
    // An empty line, a shorter blank line, and a longer one.
    assert_eq!(string("\"\n    a\n\n  \n      \n    b\n\""), "a\n\n\n  \nb");
    // A blank line without the common indentation becomes empty.
    assert_eq!(string("\"\n    a\n\t\t\t\t\t\n    b\n\""), "a\n\nb");
    // A literal whose lines are all blank.
    assert_eq!(string("\"\n   \n\n  \""), "\n");
    assert_eq!(string("\"\n\""), "");
    assert_eq!(string("\"  \n  \""), "");
}

#[test]
fn the_common_indentation_is_the_minimum() {
    // Whichever line is least indented sets it, first, middle or last.
    assert_eq!(string("\"\n  a\n    b\n      c\n\""), "a\n  b\n    c");
    assert_eq!(string("\"\n    a\n  b\n      c\n\""), "  a\nb\n    c");
    assert_eq!(string("\"\n    a\n      b\n  c\""), "  a\n    b\nc");
    // The closing line counts when it has content.
    assert_eq!(string("\"\n    a\n  b\""), "  a\nb");
}

#[test]
fn content_on_the_quote_line_is_untouched() {
    assert_eq!(string("\"a\n    b\n  \""), "a\n    b\n  ");
    assert_eq!(string("\" \n  x\""), "x");
    // Something other than whitespace before the line break.
    assert_eq!(string("\"\\n\n  x\n\""), "\n\n  x\n");
}

#[test]
fn the_final_line_break_goes_only_when_the_last_line_is_blank() {
    assert_eq!(string("\"\n  a\n  \""), "a");
    assert_eq!(string("\"\n  a\n\n  \""), "a\n");
    assert_eq!(string("\"\n  a\n  b\""), "a\nb");
    // A continuation escaping the final line break removes it itself.
    assert_eq!(string("\"\n  a \\\n  \""), "a ");
    assert_eq!(string("\"\n  a \\\\\n  \""), "a \\");
}

#[test]
fn stripping_applies_to_raw_strings() {
    assert_eq!(
        string("r\"\"\"\n    C:\\path\\{x}\n      \"quoted\"\n    \"\"\""),
        "C:\\path\\{x}\n  \"quoted\""
    );
    assert_eq!(string("r\"\n  a \\\n  \""), "a \\");
}

#[test]
fn stripping_happens_before_escapes() {
    // An escaped line break or tab is not indentation.
    assert_eq!(string("\"\n    \\ta\n    b\n\""), "\ta\nb");
    assert_eq!(string("\"\\\n    a\n    b\""), "a\n    b");
}

#[test]
fn stripping_with_interpolations() {
    // On a stripped line.
    assert_eq!(
        show("\"\n    a {x}\n    b\n\""),
        vec!["StrStart(\"a \")", "Ident(x)", "StrEnd(\"\\nb\")", "Eof"]
    );
    // An interpolation spanning lines is measured too, and its own lines
    // are not stripped of anything, being expression.
    assert_eq!(
        show("\"\n    a {\n  x\n    } b\n    c\n\""),
        vec![
            "StrStart(\"  a \")",
            "Ident(x)",
            "StrEnd(\" b\\n  c\")",
            "Eof"
        ]
    );
    // A nested literal is stripped by its own lines, and its lines count
    // for the outer literal too: the outer literal's common indentation
    // is the nested `  x`'s two spaces.
    assert_eq!(
        show("\"\n    {\"\n  x\n      \"}\n    y\n\""),
        vec![
            "StrStart(\"  \")",
            "Str(\"x\")",
            "StrEnd(\"\\n  y\")",
            "Eof"
        ]
    );
}

/// The tokens of `text` lexed going on past its errors, and the errors'
/// messages, each with the source it is at.
fn lex_all(text: &str) -> (Vec<TokenKind>, Vec<(String, String)>) {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let lexed = super::lex(&source, &mut Names::new());
    let errors = lexed
        .diagnostics
        .iter()
        .map(|d| (d.message.clone(), source.slice(d.span).to_owned()))
        .collect();
    (
        lexed.tokens.iter().map(|token| token.kind).collect(),
        errors,
    )
}

#[test]
fn the_lexer_goes_on_past_each_error() {
    // Two unexpected characters are two errors, each an error token.
    let (tokens, errors) = lex_all("a @ b $ c");
    assert_eq!(
        errors,
        [
            ("unexpected character `@`".to_owned(), "@".to_owned()),
            ("unexpected character `$`".to_owned(), "$".to_owned()),
        ]
    );
    assert_eq!(tokens.len(), 6);
    assert_eq!(tokens[1], TokenKind::Error);
    assert_eq!(tokens[3], TokenKind::Error);
    // A multi-byte character is one error token.
    let (tokens, errors) = lex_all("a é b");
    assert_eq!(errors.len(), 1);
    assert_eq!(tokens.len(), 4);
    // A malformed number is one error token, glued letters and all; an
    // ambiguous `-` one of its own.
    let (tokens, errors) = lex_all("1__0x 2e f(x)-1");
    assert_eq!(
        errors,
        [
            (
                "consecutive `_` in number literal".to_owned(),
                "__".to_owned()
            ),
            ("missing digits in exponent".to_owned(), "e".to_owned()),
            ("`-` here is ambiguous".to_owned(), "-".to_owned()),
        ]
    );
    assert_eq!(tokens[0], TokenKind::Error);
    assert_eq!(tokens[1], TokenKind::Error);
    assert_eq!(tokens[6], TokenKind::Error);
    assert!(matches!(tokens[7], TokenKind::Int(1)));
    // Errors inside a string literal are reported, and it is a string.
    let (tokens, errors) = lex_all(r#""a\qb\u{zz}c}d" x"#);
    assert_eq!(
        errors,
        [
            ("unknown escape `\\q`".to_owned(), "\\q".to_owned()),
            ("invalid unicode escape".to_owned(), "\\u{zz}".to_owned()),
            ("unmatched `}` in string".to_owned(), "}".to_owned()),
        ]
    );
    assert!(matches!(tokens[0], TokenKind::Str(_)));
    assert!(matches!(tokens[1], TokenKind::Ident(_)));
    // A literal whose end cannot be found is an error token to the end
    // of the source.
    let (tokens, errors) = lex_all("x = \"a{ (\n y");
    assert_eq!(
        errors,
        [("unterminated interpolation".to_owned(), "{".to_owned())]
    );
    assert!(matches!(
        tokens.as_slice(),
        [
            TokenKind::Ident(_),
            TokenKind::Eq,
            TokenKind::Error,
            TokenKind::Eof
        ]
    ));
}

#[test]
fn comments_are_recorded_in_source_order_when_asked_for() {
    let text =
        "#!/usr/bin/env gcsl\n# a\n#[shadowable]\nx = (1 # b\n) # c \t\ny = \"{z # d\n}\"\n# e";
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (lexed, comments) = lex_with_comments(&source, &mut Names::new());
    let shown: Vec<&str> = comments
        .spans()
        .iter()
        .map(|&span| source.slice(span))
        .collect();
    // A shebang is a comment, an attribute is not; one inside brackets
    // and one inside an interpolation are, and the spaces and tabs
    // before the line's end are in it.
    assert_eq!(
        shown,
        ["#!/usr/bin/env gcsl", "# a", "# b", "# c \t", "# d", "# e"]
    );
    // The tokens are `lex`'s.
    let plain = super::lex(&source, &mut Names::new());
    assert_eq!(lexed.tokens, plain.tokens);
    // Found by position: the run above `x`, after it, and at an offset.
    let x = u32::try_from(text.find("x =").unwrap()).unwrap();
    let above: Vec<&str> = comments
        .before(x)
        .iter()
        .map(|&span| source.slice(span))
        .collect();
    assert_eq!(above, ["#!/usr/bin/env gcsl", "# a"]);
    assert_eq!(comments.after(x).len(), 4);
    let b = u32::try_from(text.find("# b").unwrap()).unwrap();
    assert_eq!(comments.at(b + 2).map(|span| span.start), Some(b));
    assert_eq!(comments.at(x), None);
    assert_eq!(comments.at(b - 1), None);
}

#[test]
fn duration_and_size_literals_are_exact() {
    const S: i64 = 1_000_000_000;
    let cases = [
        ("30s", TokenKind::Duration(30 * S)),
        ("500ms", TokenKind::Duration(500_000_000)),
        ("1h30m", TokenKind::Duration(90 * 60 * S)),
        ("2d", TokenKind::Duration(2 * 86_400 * S)),
        ("1us", TokenKind::Duration(1_000)),
        ("7ns", TokenKind::Duration(7)),
        ("0s", TokenKind::Duration(0)),
        // A fraction on the last component, exactly.
        ("1m0.5s", TokenKind::Duration(60 * S + S / 2)),
        ("1.5h", TokenKind::Duration(90 * 60 * S)),
        ("0.000000001s", TokenKind::Duration(1)),
        (
            "2.5000000000000000000000000000000000000000000s",
            TokenKind::Duration(2 * S + S / 2),
        ),
        ("1_000ms", TokenKind::Duration(S)),
        (
            "106751d23h47m16s854ms775us807ns",
            TokenKind::Duration(i64::MAX),
        ),
        ("512B", TokenKind::Size(512)),
        ("1KB", TokenKind::Size(1_000)),
        ("1.5MB", TokenKind::Size(1_500_000)),
        ("1KiB", TokenKind::Size(1_024)),
        ("2GiB", TokenKind::Size(2 << 30)),
        ("1PiB", TokenKind::Size(1 << 50)),
        ("0.5KiB", TokenKind::Size(512)),
        ("0.0009765625KiB", TokenKind::Size(1)),
        ("8191PiB", TokenKind::Size(8191 << 50)),
        ("9223372036854775807B", TokenKind::Size(i64::MAX)),
    ];
    for (text, kind) in cases {
        assert_eq!(kinds(text), [kind, TokenKind::Eof], "{text}");
    }
    // A minus before one is negation, as before a number.
    assert_eq!(
        kinds("-30s"),
        [TokenKind::Neg, TokenKind::Duration(30 * S), TokenKind::Eof]
    );
}

#[test]
fn each_mistake_in_a_duration_or_size_literal_is_its_own_error() {
    // Each case, its error and the text it points at: the last such text,
    // where it occurs more than once.
    let cases = [
        ("1m1h", "`h` cannot follow `m` in a duration", "h"),
        ("1s1s", "`s` cannot follow `s` in a duration", "s"),
        (
            "1.5m30s",
            "only the last component of a duration may have a fraction",
            "1.5m",
        ),
        ("1h2GiB", "`GiB` is not a duration's unit", "GiB"),
        ("1h30sec", "unknown unit `sec`", "sec"),
        ("1GiB2MiB", "a size literal has one unit", "2MiB"),
        (
            "1.5ns",
            "a duration literal must be a whole number of nanoseconds",
            "1.5ns",
        ),
        (
            "0.3B",
            "a size literal must be a whole number of bytes",
            "0.3B",
        ),
        (
            "106751d23h47m16s854ms775us808ns",
            "duration literal out of range for Duration",
            "106751d23h47m16s854ms775us808ns",
        ),
        (
            "106752d",
            "duration literal out of range for Duration",
            "106752d",
        ),
        (
            "9223372036854775808B",
            "size literal out of range for Size",
            "9223372036854775808B",
        ),
        ("8192PiB", "size literal out of range for Size", "8192PiB"),
        ("1e3s", "a number with an exponent cannot have a unit", "s"),
        (
            "1.000000000000000000000000000000000000001s",
            "the fraction of a duration literal has too many digits",
            "1.000000000000000000000000000000000000001s",
        ),
    ];
    for (text, message, at) in cases {
        let start = u32::try_from(text.rfind(at).unwrap()).unwrap();
        let span = Span::new(start, start + u32::try_from(at.len()).unwrap());
        assert_eq!(error(text), (message.to_owned(), span), "{text}");
    }
    // An unknown unit is noted with every unit, or, after a duration's
    // component, a duration's.
    let unknown = lex("1h30sec").unwrap_err();
    assert_eq!(unknown.notes, [DURATION_UNITS_NOTE]);
    let largest = lex("8192PiB").unwrap_err();
    assert_eq!(
        largest.notes,
        ["the largest Size literal is 9223372036854775807B"]
    );
    // A malformed one is one error token, all its letters and digits.
    let (tokens, errors) = lex_all("1m1h30s x");
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        tokens[..],
        [TokenKind::Error, TokenKind::Ident(_), TokenKind::Eof]
    ));
}

#[test]
fn a_number_glued_to_letters_that_name_no_unit_is_still_a_number() {
    // So a module path's `pkgs/7zip` still lexes as it did, and the parser
    // reports `30sec` where an expression is expected.
    assert_eq!(show("7zip"), ["Int(7)", "Ident(zip)", "Eof"]);
    assert_eq!(show("30sec"), ["Int(30)", "Ident(sec)", "Eof"]);
    assert_eq!(show("1.5x"), ["Float(1.5)", "Ident(x)", "Eof"]);
    assert_eq!(show("1e5"), ["Float(100000.0)", "Eof"]);
    // Units are case-sensitive.
    assert_eq!(show("5gib"), ["Int(5)", "Ident(gib)", "Eof"]);
    // The scan ends a literal where the lexer does.
    for (text, end) in [("1h30m x", 5), ("7zip", 1), ("2GiB", 4), ("1.5s+", 4)] {
        assert_eq!(number_end(text.as_bytes(), 0), end, "{text}");
    }
}

#[test]
fn an_attribute_with_arguments_opens_a_bracket_its_arguments_close() {
    let (tokens, names, _) =
        tokenize(&SourceFile::new("t", "#[device(\n  name = \"kvm\",\n)]\nfn").unwrap()).unwrap();
    let kinds: Vec<String> = tokens
        .iter()
        .map(|token| match token.kind {
            TokenKind::AttributeArgs(name) => format!("AttributeArgs({})", names.get(name)),
            TokenKind::Ident(name) => format!("Ident({})", names.get(name)),
            kind => format!("{kind:?}"),
        })
        .collect();
    // No newline inside the brackets; the one after its `]` is an item's
    // end as ever.
    assert_eq!(
        kinds[..],
        [
            "AttributeArgs(device)",
            "LParen",
            "Ident(name)",
            "Eq",
            "Str(StrId(0))",
            "Comma",
            "RParen",
            "RBracket",
            "Newline",
            "Fn",
            "Eof"
        ]
    );
    // The token is its `#[` and name; spaces before the `(` are allowed.
    assert_eq!(spans("#[retry (3)]")[0], (0, 7));
    assert!(matches!(
        kinds_of("#![x(1)]")[0],
        TokenKind::InnerAttributeArgs(_)
    ));
    // Without `]` or `(` after its name, it is a comment, as before.
    assert_eq!(kinds_of("#[retry 3]"), [TokenKind::Eof]);
    assert_eq!(kinds_of("#[retry"), [TokenKind::Eof]);
}

fn kinds_of(text: &str) -> Vec<TokenKind> {
    kinds(text)
}
