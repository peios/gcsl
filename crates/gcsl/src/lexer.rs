//! The lexer: source text to a vector of tokens.
//!
//! A single hand-written pass over the source bytes. Spaces, tabs, `\r` and
//! comments (`#` to end of line) are skipped. Each `\n` becomes a
//! [`TokenKind::Newline`] token, because newlines separate items, except
//! inside parentheses: the lexer tracks the depth of open `(` and emits no
//! `Newline` while it is above zero, so an expression in parentheses may
//! span lines freely. Braces do not affect the depth, since a block body
//! inside `{ }` is newline-sensitive again.
//!
//! Known limitation: because only parentheses are tracked, a newline inside
//! a `{ }` that is itself inside `( )` is not emitted either, so a
//! multi-line block written in parentheses runs its items together. No
//! construct needs that yet; when function calls arrive (`f({ ... })`), the
//! parser will have to handle it, for instance by having the lexer track a
//! stack of bracket kinds rather than a depth.
//!
//! Identifiers are interned into a [`Names`] table as they are lexed, so a
//! [`TokenKind::Ident`] carries a [`Name`], not text. An identifier starts
//! with an ASCII letter or `_` and continues with ASCII letters, digits and
//! `_`, and with `-` when the `-` is directly followed by one of those:
//! `hello-world` and `x-1` are single names. `let` is the only keyword.
//!
//! A `-` that is not inside an identifier is classified from the bytes
//! either side of it (see `Lexer::minus`): subtraction ([`TokenKind::Minus`])
//! when followed by whitespace, negation ([`TokenKind::Neg`]) when directly
//! followed by an operand, and an error when it directly follows an operand,
//! where `a-b` and `a - b` would be too easily confused.
//!
//! The only allocations besides the output vector are one scratch `String`,
//! reused for every numeric literal that contains `_` separators, and the
//! interning of each distinct name.

use crate::diagnostic::Diagnostic;
use crate::names::{Name, Names};
use crate::source::{SourceFile, Span, offset_to_u32};

/// One token: its kind (with the value, for literals) and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// The bytes it was lexed from. For [`TokenKind::Eof`], an empty span
    /// just after the last token other than a [`TokenKind::Newline`] (or at
    /// offset 0 if there is none), so that "found end of input" errors point
    /// at the end of the last line with code on it.
    pub span: Span,
}

/// The kind of a [`Token`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    /// An integer literal, already converted.
    Int(i64),
    /// A float literal, already converted.
    Float(f64),
    /// An identifier other than a keyword.
    Ident(Name),
    /// The keyword `let`.
    Let,
    /// `+`
    Plus,
    /// `-` as subtraction: not directly after an operand, and followed by
    /// whitespace.
    Minus,
    /// `-` as negation: not directly after an operand, and directly followed
    /// by one.
    Neg,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `//`
    SlashSlash,
    /// `%`
    Percent,
    /// `:`
    Colon,
    /// `:=`
    ColonEq,
    /// `=`
    Eq,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `;`
    Semi,
    /// A `\n` outside parentheses. Each newline is its own token.
    Newline,
    /// End of input. Always the last token.
    Eof,
}

impl TokenKind {
    /// The fixed spelling of an operator, delimiter or keyword token, or
    /// `None` for literals, identifiers, [`TokenKind::Newline`] and
    /// [`TokenKind::Eof`], whose text is not fixed or not visible.
    #[must_use]
    pub const fn symbol(self) -> Option<&'static str> {
        match self {
            TokenKind::Let => Some("let"),
            TokenKind::Plus => Some("+"),
            TokenKind::Minus | TokenKind::Neg => Some("-"),
            TokenKind::Star => Some("*"),
            TokenKind::Slash => Some("/"),
            TokenKind::SlashSlash => Some("//"),
            TokenKind::Percent => Some("%"),
            TokenKind::Colon => Some(":"),
            TokenKind::ColonEq => Some(":="),
            TokenKind::Eq => Some("="),
            TokenKind::LParen => Some("("),
            TokenKind::RParen => Some(")"),
            TokenKind::LBrace => Some("{"),
            TokenKind::RBrace => Some("}"),
            TokenKind::Semi => Some(";"),
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Ident(_)
            | TokenKind::Newline
            | TokenKind::Eof => None,
        }
    }
}

/// Lexes the whole of `source`, interning its names into a new table.
///
/// The returned vector always ends with exactly one [`TokenKind::Eof`].
///
/// # Errors
///
/// Returns the first lexical error: an unexpected character, an ambiguous
/// `-`, a malformed number (misplaced `_`, missing exponent digits) or a
/// literal too large for its type.
pub fn tokenize(source: &SourceFile) -> Result<(Vec<Token>, Names), Diagnostic> {
    let mut names = Names::new();
    let tokens = tokenize_with(source, &mut names)?;
    Ok((tokens, names))
}

/// Like [`tokenize`], interning names into an existing table, so that they
/// compare equal to names lexed earlier into it.
///
/// On error, names met before the error remain interned.
///
/// # Errors
///
/// As [`tokenize`].
pub fn tokenize_with(source: &SourceFile, names: &mut Names) -> Result<Vec<Token>, Diagnostic> {
    let mut lexer = Lexer::new(source, names);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        tokens.push(token);
        if token.kind == TokenKind::Eof {
            return Ok(tokens);
        }
    }
}

/// Lexer state. Produces one token per [`Lexer::next_token`] call.
struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    /// Byte offset of the next unread byte. Always on a character boundary:
    /// outside comments the lexer only consumes ASCII bytes and stops with an
    /// error at the first byte it does not recognise, and a comment is
    /// skipped up to a `\n` or the end of the text. Slicing `text` at `pos`
    /// is therefore safe.
    pos: usize,
    /// End of the most recent token other than a newline, used to place the
    /// `Eof` token.
    last_end: usize,
    /// Number of `(` not yet closed. Newlines are only emitted at zero.
    /// Saturates at zero on an unmatched `)`, which the parser reports.
    paren_depth: u32,
    /// Reused buffer for literals whose `_` separators must be removed.
    scratch: String,
    /// Where identifiers are interned.
    names: &'a mut Names,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a SourceFile, names: &'a mut Names) -> Self {
        Self {
            text: source.text(),
            bytes: source.text().as_bytes(),
            pos: 0,
            last_end: 0,
            paren_depth: 0,
            scratch: String::new(),
            names,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    fn next_token(&mut self) -> Result<Token, Diagnostic> {
        self.skip_trivia();
        let start = self.pos;
        let Some(byte) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: span(self.last_end, self.last_end),
            });
        };
        let kind = match byte {
            // `skip_trivia` only stops at a newline outside parentheses.
            b'\n' => {
                self.pos += 1;
                return Ok(Token {
                    kind: TokenKind::Newline,
                    span: span(start, self.pos),
                });
            }
            b'+' => self.single(TokenKind::Plus),
            b'-' => self.minus(start)?,
            b'*' => self.single(TokenKind::Star),
            b'%' => self.single(TokenKind::Percent),
            b'(' => {
                self.paren_depth = self.paren_depth.saturating_add(1);
                self.single(TokenKind::LParen)
            }
            b')' => {
                self.paren_depth = self.paren_depth.saturating_sub(1);
                self.single(TokenKind::RParen)
            }
            b'{' => self.single(TokenKind::LBrace),
            b'}' => self.single(TokenKind::RBrace),
            b';' => self.single(TokenKind::Semi),
            b'=' => self.single(TokenKind::Eq),
            b':' => {
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    TokenKind::ColonEq
                } else {
                    self.single(TokenKind::Colon)
                }
            }
            b'/' => {
                if self.peek_at(1) == Some(b'/') {
                    self.pos += 2;
                    TokenKind::SlashSlash
                } else {
                    self.single(TokenKind::Slash)
                }
            }
            b'0'..=b'9' => self.number(start)?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.identifier(start),
            _ => return Err(self.unexpected_character()),
        };
        self.last_end = self.pos;
        Ok(Token {
            kind,
            span: span(start, self.pos),
        })
    }

    /// Consumes a one-byte token.
    fn single(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    /// Skips whitespace and comments, stopping at a `\n` that must become a
    /// [`TokenKind::Newline`] token.
    fn skip_trivia(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' if self.paren_depth > 0 => self.pos += 1,
                b'#' => {
                    // Skip to the newline, which is then either skipped or
                    // emitted like any other.
                    self.pos = self
                        .bytes
                        .get(self.pos..)
                        .unwrap_or_default()
                        .iter()
                        .position(|&b| b == b'\n')
                        .map_or(self.bytes.len(), |offset| self.pos + offset);
                }
                _ => break,
            }
        }
    }

    /// Lexes an identifier or keyword starting at `start`, which holds an
    /// ASCII letter or `_`.
    ///
    /// A `-` continues the identifier only when the byte after it is an
    /// identifier character, so `a-b` is one name but `a-` and `a- b` are
    /// not.
    fn identifier(&mut self, start: usize) -> TokenKind {
        while let Some(byte) = self.peek() {
            if is_identifier_byte(byte) {
                self.pos += 1;
            } else if byte == b'-' && self.peek_at(1).is_some_and(is_identifier_byte) {
                // Consume the `-` and the character after it together.
                self.pos += 2;
            } else {
                break;
            }
        }
        let text = &self.text[start..self.pos];
        if text == "let" {
            TokenKind::Let
        } else {
            TokenKind::Ident(self.names.intern(text))
        }
    }

    /// Classifies a `-` at `start` that is not part of an identifier or a
    /// number literal, from the bytes on either side of it:
    ///
    /// 1. Directly after an operand end (an identifier character or `)`):
    ///    an error, since `a-b` would be a single name and `a -b` a
    ///    negation.
    /// 2. Otherwise, followed by whitespace or the end of input:
    ///    subtraction, [`TokenKind::Minus`].
    /// 3. Otherwise, directly followed by an operand start (a letter, digit,
    ///    `_`, `(` or `{`): negation, [`TokenKind::Neg`].
    /// 4. Otherwise, as in `--x`: an unexpected character.
    fn minus(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        let previous = start.checked_sub(1).and_then(|at| self.bytes.get(at));
        if previous.is_some_and(|&byte| is_identifier_byte(byte) || byte == b')') {
            return Err(
                Diagnostic::error("`-` here is ambiguous", span(start, start + 1)).with_note(
                    "write `a - b` with spaces for subtraction, or `a-b` for a single name",
                ),
            );
        }
        match self.peek_at(1) {
            None | Some(b' ' | b'\t' | b'\r' | b'\n') => Ok(self.single(TokenKind::Minus)),
            Some(byte) if is_identifier_byte(byte) || byte == b'(' || byte == b'{' => {
                Ok(self.single(TokenKind::Neg))
            }
            Some(_) => Err(self.unexpected_character()),
        }
    }

    /// Lexes an integer or float literal starting at `start`, which holds a
    /// digit.
    ///
    /// Grammar, where each `digits` group is ASCII digits with single `_`
    /// separators between them:
    ///
    /// ```text
    /// number   = digits ( "." digits )? exponent?
    /// exponent = ( "e" | "E" ) ( "+" | "-" )? digits
    /// ```
    ///
    /// A literal with a fraction or an exponent is a float. A `.` not followed
    /// by a digit is not part of the literal. A `-` after a literal is never
    /// part of it, except as the sign of an exponent.
    fn number(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        let mut has_underscore = self.digits()?;
        let mut is_float = false;

        if self.peek() == Some(b'.') {
            match self.peek_at(1) {
                Some(b'0'..=b'9') => {
                    self.pos += 1;
                    has_underscore |= self.digits()?;
                    is_float = true;
                }
                Some(b'_') => return Err(leading_underscore(self.pos + 1)),
                _ => {}
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            let exponent_start = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            match self.peek() {
                Some(b'0'..=b'9') => has_underscore |= self.digits()?,
                Some(b'_') => return Err(leading_underscore(self.pos)),
                _ => {
                    return Err(Diagnostic::error(
                        "missing digits in exponent",
                        span(exponent_start, self.pos),
                    ));
                }
            }
            is_float = true;
        }

        let literal_span = span(start, self.pos);
        let raw = &self.text[start..self.pos];
        let digits = if has_underscore {
            self.scratch.clear();
            self.scratch.extend(raw.chars().filter(|&c| c != '_'));
            self.scratch.as_str()
        } else {
            raw
        };

        if is_float {
            // The grammar above only admits text that `f64::from_str` accepts,
            // so this cannot fail; the error arm keeps that assumption out of
            // the control flow.
            let value: f64 = digits
                .parse()
                .map_err(|_| Diagnostic::error("malformed float literal", literal_span))?;
            // `f64::from_str` rounds a literal beyond `f64::MAX` to infinity.
            // A literal that does not denote the value written is an error,
            // not a silent infinity.
            if value.is_infinite() {
                return Err(Diagnostic::error(
                    "float literal out of range for Float",
                    literal_span,
                )
                .with_note(format!("the largest Float literal is {:?}", f64::MAX)));
            }
            Ok(TokenKind::Float(value))
        } else {
            // Only digits reach here, so the one possible failure is overflow.
            digits.parse().map(TokenKind::Int).map_err(|_| {
                Diagnostic::error("integer literal out of range for Int", literal_span)
                    .with_note(format!("the largest Int literal is {}", i64::MAX))
            })
        }
    }

    /// Consumes a group of digits with `_` separators, starting at a digit.
    /// Returns whether any `_` was seen.
    fn digits(&mut self) -> Result<bool, Diagnostic> {
        let mut has_underscore = false;
        while let Some(byte) = self.peek() {
            match byte {
                b'0'..=b'9' => self.pos += 1,
                b'_' => match self.peek_at(1) {
                    Some(b'0'..=b'9') => {
                        has_underscore = true;
                        self.pos += 1;
                    }
                    Some(b'_') => {
                        return Err(Diagnostic::error(
                            "consecutive `_` in number literal",
                            span(self.pos, self.pos + 2),
                        ));
                    }
                    _ => {
                        return Err(Diagnostic::error(
                            "trailing `_` in number literal",
                            span(self.pos, self.pos + 1),
                        ));
                    }
                },
                _ => break,
            }
        }
        Ok(has_underscore)
    }

    fn unexpected_character(&self) -> Diagnostic {
        let ch = self.text[self.pos..].chars().next().unwrap_or('\u{FFFD}');
        Diagnostic::error(
            format!("unexpected character `{}`", ch.escape_debug()),
            span(self.pos, self.pos + ch.len_utf8()),
        )
    }
}

/// Whether `byte` is an identifier character: an ASCII letter, digit or `_`.
const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn leading_underscore(at: usize) -> Diagnostic {
    Diagnostic::error("leading `_` in number literal", span(at, at + 1))
}

/// Builds a span from byte offsets within a [`SourceFile`], which always fit
/// in `u32`.
fn span(start: usize, end: usize) -> Span {
    Span::new(offset_to_u32(start), offset_to_u32(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(text: &str) -> Result<Vec<Token>, Diagnostic> {
        tokenize(&SourceFile::new("test.gcsl", text).unwrap()).map(|(tokens, _)| tokens)
    }

    fn kinds(text: &str) -> Vec<TokenKind> {
        lex(text).unwrap().into_iter().map(|t| t.kind).collect()
    }

    /// The tokens with identifiers shown by their text, e.g. `Ident(x)`.
    fn show(text: &str) -> Vec<String> {
        let (tokens, names) = tokenize(&SourceFile::new("test.gcsl", text).unwrap()).unwrap();
        tokens
            .iter()
            .map(|token| match token.kind {
                TokenKind::Ident(name) => format!("Ident({})", names.get(name)),
                kind => format!("{kind:?}"),
            })
            .collect()
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = lex(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
    }

    #[test]
    fn tokens_are_small() {
        // A 16-byte kind (the `i64`/`f64` payloads) and an 8-byte span.
        assert_eq!(size_of::<Token>(), 24);
    }

    #[test]
    fn every_token_kind() {
        use TokenKind::*;
        let (tokens, names) = tokenize(
            &SourceFile::new(
                "test.gcsl",
                "1 2.5 + - -x * / // % ( ) x let : := = { } ;\n",
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
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn spans_are_exact() {
        let tokens = lex("12 +\t3.25//(  7)").unwrap();
        let spans: Vec<(u32, u32)> = tokens.iter().map(|t| (t.span.start, t.span.end)).collect();
        assert_eq!(
            spans,
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
    fn comments_are_skipped_and_their_newlines_kept() {
        assert_eq!(
            show("# leading\n1 # one\r\n+ # plus\n2 #"),
            vec![
                "Newline", "Int(1)", "Newline", "Plus", "Newline", "Int(2)", "Eof"
            ]
        );
    }

    #[test]
    fn each_newline_is_a_token() {
        let tokens = lex("1\n\n2\r\n").unwrap();
        let kinds: Vec<(TokenKind, Span)> = tokens.iter().map(|t| (t.kind, t.span)).collect();
        assert_eq!(
            kinds,
            vec![
                (TokenKind::Int(1), Span::new(0, 1)),
                (TokenKind::Newline, Span::new(1, 2)),
                (TokenKind::Newline, Span::new(2, 3)),
                (TokenKind::Int(2), Span::new(3, 4)),
                (TokenKind::Newline, Span::new(5, 6)),
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
        // Braces do not reset the suppression (a known limitation), and do
        // not cause it either.
        assert_eq!(
            show("({\n1\n})\n{\n2\n}"),
            vec![
                "LParen", "LBrace", "Int(1)", "RBrace", "RParen", "Newline", "LBrace", "Newline",
                "Int(2)", "Newline", "RBrace", "Eof"
            ]
        );
        // A comment inside parentheses ends at a suppressed newline.
        assert_eq!(
            show("(1 # one\n)"),
            vec!["LParen", "Int(1)", "RParen", "Eof"]
        );
        // An unmatched `)` does not make the depth negative.
        assert_eq!(show(")\n1"), vec!["RParen", "Newline", "Int(1)", "Eof"]);
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
    fn let_is_the_only_keyword() {
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
    }

    #[test]
    fn interning_gives_equal_names_for_equal_text() {
        let (tokens, names) =
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
    fn tokenize_with_shares_the_table() {
        let mut names = Names::new();
        let first = tokenize_with(&SourceFile::new("a", "x").unwrap(), &mut names).unwrap();
        let second = tokenize_with(&SourceFile::new("b", "y x").unwrap(), &mut names).unwrap();
        assert_eq!(first[0].kind, second[1].kind);
        assert_eq!(names.len(), 2);
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
        assert_eq!(show("a -b"), vec!["Ident(a)", "Neg", "Ident(b)", "Eof"]);
        // Directly after punctuation other than `)`.
        assert_eq!(show("x=-1")[1..3], ["Eq", "Neg"]);
        assert_eq!(show("1+-1")[1..3], ["Plus", "Neg"]);
        assert_eq!(show("x:-1")[1..3], ["Colon", "Neg"]);
        assert_eq!(show(";-1")[1], "Neg");
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
            error("1."),
            ("unexpected character `.`".to_owned(), Span::new(1, 2))
        );
        assert_eq!(
            error(".5"),
            ("unexpected character `.`".to_owned(), Span::new(0, 1))
        );
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
        assert_eq!(TokenKind::Int(1).symbol(), None);
        assert_eq!(TokenKind::Newline.symbol(), None);
        assert_eq!(TokenKind::Eof.symbol(), None);
    }
}
