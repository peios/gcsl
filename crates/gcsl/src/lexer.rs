//! The lexer: source text to a vector of tokens.
//!
//! A single hand-written pass over the source bytes. Whitespace (space, tab,
//! `\r`, `\n`) and comments (`#` to end of line) are skipped. The only
//! allocation besides the output vector is one scratch `String`, reused for
//! every numeric literal that contains `_` separators.

use crate::diagnostic::Diagnostic;
use crate::source::{SourceFile, Span, offset_to_u32};

/// One token: its kind (with the value, for literals) and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// The bytes it was lexed from. For [`TokenKind::Eof`], an empty span
    /// just after the last real token (or at offset 0 if there is none).
    pub span: Span,
}

/// The kind of a [`Token`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    /// An integer literal, already converted.
    Int(i64),
    /// A float literal, already converted.
    Float(f64),
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `//`
    SlashSlash,
    /// `%`
    Percent,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// End of input. Always the last token.
    Eof,
}

impl TokenKind {
    /// The fixed spelling of an operator or delimiter token, or `None` for
    /// literals and [`TokenKind::Eof`], whose text is not fixed.
    #[must_use]
    pub const fn symbol(self) -> Option<&'static str> {
        match self {
            TokenKind::Plus => Some("+"),
            TokenKind::Minus => Some("-"),
            TokenKind::Star => Some("*"),
            TokenKind::Slash => Some("/"),
            TokenKind::SlashSlash => Some("//"),
            TokenKind::Percent => Some("%"),
            TokenKind::LParen => Some("("),
            TokenKind::RParen => Some(")"),
            TokenKind::Int(_) | TokenKind::Float(_) | TokenKind::Eof => None,
        }
    }
}

/// Lexes the whole of `source`.
///
/// The returned vector always ends with exactly one [`TokenKind::Eof`].
///
/// # Errors
///
/// Returns the first lexical error: an unexpected character, a malformed
/// number (misplaced `_`, missing exponent digits) or an integer literal too
/// large for `Int`.
pub fn tokenize(source: &SourceFile) -> Result<Vec<Token>, Diagnostic> {
    let mut lexer = Lexer::new(source);
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
    /// End of the most recent token, used to place the `Eof` token.
    last_end: usize,
    /// Reused buffer for literals whose `_` separators must be removed.
    scratch: String,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a SourceFile) -> Self {
        Self {
            text: source.text(),
            bytes: source.text().as_bytes(),
            pos: 0,
            last_end: 0,
            scratch: String::new(),
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
            b'+' => self.single(TokenKind::Plus),
            b'-' => self.single(TokenKind::Minus),
            b'*' => self.single(TokenKind::Star),
            b'%' => self.single(TokenKind::Percent),
            b'(' => self.single(TokenKind::LParen),
            b')' => self.single(TokenKind::RParen),
            b'/' => {
                if self.peek_at(1) == Some(b'/') {
                    self.pos += 2;
                    TokenKind::SlashSlash
                } else {
                    self.single(TokenKind::Slash)
                }
            }
            b'0'..=b'9' => self.number(start)?,
            b'_' if self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) => {
                return Err(leading_underscore(start));
            }
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

    fn skip_trivia(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b'#' => {
                    // Skip to the newline; the loop then consumes it.
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

    /// Lexes an integer or float literal starting at `start`, which holds a
    /// digit.
    ///
    /// Grammar, where each `digits` group is ASCII digits with single `_`
    /// separators between them:
    ///
    /// ```text
    /// number   = digits ( "." digits )? exponent?
    /// exponent = "e" ( "+" | "-" )? digits
    /// ```
    ///
    /// A literal with a fraction or an exponent is a float. A `.` not followed
    /// by a digit is not part of the literal.
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

        if self.peek() == Some(b'e') {
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
            // the control flow. Literals beyond `f64::MAX` round to infinity.
            digits
                .parse()
                .map(TokenKind::Float)
                .map_err(|_| Diagnostic::error("malformed float literal", literal_span))
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
        tokenize(&SourceFile::new("test.gcsl", text).unwrap())
    }

    fn kinds(text: &str) -> Vec<TokenKind> {
        lex(text).unwrap().into_iter().map(|t| t.kind).collect()
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = lex(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
    }

    #[test]
    fn every_token_kind() {
        use TokenKind::*;
        assert_eq!(
            kinds("1 2.5 + - * / // % ( )"),
            vec![
                Int(1),
                Float(2.5),
                Plus,
                Minus,
                Star,
                Slash,
                SlashSlash,
                Percent,
                LParen,
                RParen,
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
    fn eof_is_placed_after_the_last_token() {
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
    fn comments_and_newlines_are_skipped() {
        assert_eq!(
            kinds("# leading\n1 # one\r\n+ # plus\n2 #"),
            vec![
                TokenKind::Int(1),
                TokenKind::Plus,
                TokenKind::Int(2),
                TokenKind::Eof
            ]
        );
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
        let expected = ("leading `_` in number literal".to_owned(), Span::new(0, 1));
        assert_eq!(error("_1"), expected);
        assert_eq!(error("1._5").1, Span::new(2, 3));
        assert_eq!(error("1e_5").1, Span::new(2, 3));
        assert_eq!(error("1e-_5").1, Span::new(3, 4));
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
    fn uppercase_exponent_is_not_part_of_the_literal() {
        assert_eq!(
            error("1E5"),
            ("unexpected character `E`".to_owned(), Span::new(1, 2))
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
    fn huge_float_literal_rounds_to_infinity() {
        assert_eq!(kinds("1e400")[0], TokenKind::Float(f64::INFINITY));
    }

    #[test]
    fn unexpected_characters_are_reported_whole() {
        assert_eq!(
            error("1 + x"),
            ("unexpected character `x`".to_owned(), Span::new(4, 5))
        );
        assert_eq!(
            error("1 € 2"),
            ("unexpected character `€`".to_owned(), Span::new(2, 5))
        );
        assert_eq!(
            error("_"),
            ("unexpected character `_`".to_owned(), Span::new(0, 1))
        );
        assert_eq!(
            error("\u{0}"),
            ("unexpected character `\\0`".to_owned(), Span::new(0, 1))
        );
    }

    #[test]
    fn symbols() {
        assert_eq!(TokenKind::SlashSlash.symbol(), Some("//"));
        assert_eq!(TokenKind::Int(1).symbol(), None);
        assert_eq!(TokenKind::Eof.symbol(), None);
    }
}
