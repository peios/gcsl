//! The lexer: source text to a vector of tokens.
//!
//! A hand-written pass over the source bytes. Spaces, tabs, `\r` and
//! comments (`#` to end of line, but an attribute, below) are skipped. Each
//! `\n` becomes a
//! [`TokenKind::Newline`] token, because newlines separate items, except
//! where the innermost open bracket is a `(` or a `[`, or an
//! interpolation's `{`: the lexer keeps a stack of the open brackets, and
//! emits no `Newline` while a `(` or a `[` is on top, so an expression, a
//! parameter list, an argument list or a list literal in brackets may span
//! lines freely, nor directly inside an interpolation (see [String
//! literals](#string-literals)). A `{` on top makes newlines significant
//! again, since a block body is newline-sensitive wherever it is written: a
//! block or a function body passed as an argument inside a call's
//! parentheses, `f({ ... })` or `f(fn(): Int { ... })`, or written as an
//! element of a list, may span lines, and so may one inside an
//! interpolation. The stack is one byte per open bracket, and a program
//! with no bracket allocates nothing for it. An unmatched closing bracket,
//! which the parser reports, closes nothing: a `)` closes only a `(` on top
//! of the stack, a `]` only a `[` on top, and a `}` closes the nearest `{`,
//! together with any `(` or `[` left open inside it.
//!
//! Identifiers are interned into a [`Names`] table as they are lexed, so a
//! [`TokenKind::Ident`] carries a [`Name`], not text. An identifier starts
//! with an ASCII letter or `_` and continues with ASCII letters, digits and
//! `_`, and with `-` when the `-` is directly followed by one of those:
//! `hello-world` and `x-1` are single names. The keywords are `let`, `mut`,
//! `if`, `else`, `true`, `false`, `and`, `or`, `not`, `fn`, `def`, `lazy`,
//! `await`, `for`, `do`, `break`, `continue`, `return`, `in`, `is` and
//! `as`; the whole identifier is scanned before it is compared with them,
//! so `fn-x`, `def-x`, `lazy-x`, `await-x`, `for-x`, `do-x`, `break-x`,
//! `return-x`, `mut-x` and `is-x` are identifiers. So no variable or
//! unquoted field can be named `def`, `do`, `break`, `continue` or `return`.
//! `def` is only ever written at the start of an item, a public function
//! (see the parser). A *label*,
//! [`TokenKind::Label`], is `'` directly
//! followed by a name, scanned by the rule for identifiers, as in `'outer`,
//! `'_` and `'a-b`, one token holding the name; its text is interned
//! whatever it spells, so `'for` is a label named `for`. A `'` not directly
//! followed by a letter or `_` is an unexpected character. `mut` is only ever written
//! after `let`, which it makes a mutable variable's declaration, but it is
//! a keyword wherever it appears, so no variable or unquoted field can be
//! named `mut`; nor, likewise, `is` or `as`, which are only ever written
//! between an expression and a type.
//! An `r` directly followed by `"` begins a raw string literal; anywhere
//! else it is an ordinary identifier. `Future` and `Present` are not
//! keywords: they are ordinary
//! identifiers, which only the checker gives a meaning, in types. Nor is
//! `type`: it is an identifier here, which the parser takes as the start of
//! a `type` item only where an item starts and an identifier follows it on
//! its line (see the parser's [Types](crate::parser#types)), so a variable
//! or field may still be named `type`.
//!
//! Operators are lexed greedily: `===` is one token, `==` another and `=`
//! followed by anything else a third, and likewise `!==` and `!=`, `<=` and
//! `<`, `>=` and `>`, `:=` and `:`, `//` and `/`. There is no `>>` token, so
//! the two `>` closing `Future<Future<Int>>` are two [`TokenKind::Gt`]s. A
//! `|` is a [`TokenKind::Pipe`], used only in union types. `!` is only
//! accepted as part of `!=` or `!==`; negation is `not`. A `.` is part of a
//! number literal only between two digits, as in
//! `1.5`. Dots are lexed greedily too: three in a row are one
//! [`TokenKind::DotDotDot`], the spread of a list element; otherwise two
//! followed by `=` are one [`TokenKind::DotDotEq`], the inclusive range,
//! and two alone one [`TokenKind::DotDot`], the range; anywhere else a `.`
//! is a [`TokenKind::Dot`]. So `1.x` is `1`, `.`, `x`, `0..10` is `0`,
//! `..`, `10`, `1.5..2` is `1.5`, `..`, `2`, `0...5` is `0`, `...`, `5`,
//! and `....` is `...`, `.`. A `,` is a
//! [`TokenKind::Comma`], which separates the fields of a record type, the
//! parameters of a function, the arguments of a call and the elements of a
//! list. `[` and `]` are [`TokenKind::LBracket`] and
//! [`TokenKind::RBracket`], which delimit list literals and indexes and end
//! the list type sugar `T[]`.
//!
//! A `-` that is not inside an identifier is classified from the bytes
//! either side of it (see `Lexer::minus`). Directly followed by `>>` it is
//! always the memo arrow `->>` ([`TokenKind::MemoArrow`]), and otherwise
//! directly followed by `>` always the arrow `->` ([`TokenKind::Arrow`]),
//! whatever is around it: these are the markers of a memoised impure and of
//! an impure function, the third, `:`, marking a pure one. So `->>>` is the
//! memo arrow and a `>`, and `- >>` a `-` and two `>`s. Otherwise it is
//! subtraction ([`TokenKind::Minus`]) when followed by
//! whitespace, negation ([`TokenKind::Neg`]) when directly followed by an
//! operand, and an error when it directly follows an operand, where `a-b`
//! and `a - b` would be too easily confused. A `]` ends an operand, as a `)`
//! does, and a `[` starts one, as a `(` does.
//!
//! # Attributes
//!
//! A `#` directly followed by `[`, or by `![`, then optional spaces or
//! tabs, an identifier (scanned by the rule for identifiers), optional
//! spaces or tabs and `]`, is an *attribute*, one token holding its name,
//! interned as an identifier is: `#[NAME]` an outer one,
//! [`TokenKind::Attribute`], which applies to the item after it, and
//! `#![NAME]` an inner one, [`TokenKind::InnerAttribute`], which applies to
//! the whole file (see the parser's [Attributes](crate::parser#attributes)).
//! Every other `#` starts a comment to the end of its line, as it always
//! has: so a shebang, `#!/usr/bin/env gcsl`, is a comment, and so is `#
//! [x]`, with a space, and `#[x y]`, `#[` alone or `#[1]`, which are not of
//! the shape. The scan of a string literal's interpolations (see [String
//! literals](#string-literals)) tells the two apart by the same rule
//! (`attribute_at`), so the lexer and the scan always agree on where a
//! comment ends.
//!
//! # String literals
//!
//! A literal is delimited by `n` quotes on each side, where `n` is 1 or at
//! least 3, and `""` is the empty string. The opening run of quotes is
//! counted whole; inside, a run of fewer than `n` quotes is content, and the
//! first `n` quotes of a longer run close the literal. With the `r` prefix
//! the literal is raw: every byte is content. Otherwise `\` begins an escape
//! and a single `{` an interpolation, whose expression is lexed as ordinary
//! tokens between the literal's pieces. A literal without interpolations is
//! one [`TokenKind::Str`]; one with interpolations is a
//! [`TokenKind::StrStart`], the tokens of each expression, a
//! [`TokenKind::StrMid`] between each two, and a [`TokenKind::StrEnd`]. Each
//! of these carries a [`StrId`] into the [`Strings`] table, which holds the
//! piece's final text: indentation stripped and escapes replaced.
//!
//! ## Contexts
//!
//! Lexing an interpolation's expression is lexing in a different context:
//! newlines are not emitted directly inside it (only inside a block within
//! it), and the `}` that matches the interpolation's `{` resumes the literal
//! instead of being a token. The lexer keeps a stack of the interpolations
//! it is inside, innermost last, each with its own brace count, the layout
//! of its literal and where its brackets start on the bracket stack, so that
//! a `(` left open inside cannot affect the enclosing context. A literal
//! nested in an interpolation is lexed by the same routine and pushes its
//! own interpolations. The stack is empty outside interpolations, so a
//! program without any allocates nothing for it.
//!
//! ## Two passes per literal
//!
//! Whether a line's indentation is stripped depends on every other line of
//! the literal, so each literal is lexed in two passes.
//!
//! **The scan** (pass 1, `Lexer::scan`) walks from the opening delimiter to
//! the closing one to find the literal's extent. In a literal's text it
//! skips escapes whole, so that neither `\"` nor the braces of `\u{...}` are
//! taken for delimiters, and skips `{{`. At a single `{` it walks the
//! interpolation as the lexer would tokenise it: it counts braces to find
//! the matching `}`, skips comments to the end of their line, skips
//! identifiers and numbers whole (so an `r` ending one is not taken for a
//! raw prefix), and walks each nested literal the same way, recursively,
//! since a `"` inside a nested literal is not the closer. At every line
//! break in the extent, including those inside interpolations and nested
//! literals, it measures the line that follows: the length of its leading
//! spaces and tabs, and whether it is blank. The common prefix of the
//! non-blank lines is the indentation to strip. Finally it looks back from
//! the closing delimiter to decide whether the final line break is removed.
//! The scan allocates nothing. It reports only the errors that leave the
//! extent unknown: an unterminated literal or interpolation (the innermost,
//! when several are), and nesting deeper than [`MAX_DEPTH`], which bounds its
//! recursion. Everything else is left to pass 2, so errors are reported in
//! source order.
//!
//! **Pass 2** (`Lexer::piece`) produces the text of one piece at a time,
//! starting at the opening delimiter or at the `}` that ends an
//! interpolation: it strips the indentation at the start of each line of the
//! literal's own text, replaces escapes, and stops at the next interpolation
//! or at the end of the text. Each piece is built in a reused buffer and
//! copied into the table at its exact length.
//!
//! A nested literal is scanned again when the lexer reaches it, so lexing
//! takes time proportional to the source's length times the depth to which
//! literals are nested in each other's interpolations.
//!
//! A line break inside a literal is a `\n` byte, and only spaces and tabs are
//! indentation: a `\r` before a line break is ordinary content.
//!
//! # Errors
//!
//! The lexer goes on past every error ([`lex`]), reporting each once, in
//! source order, and puts a [`TokenKind::Error`] where it is, which the
//! parser reports nothing at (see the parser's [Errors and
//! recovery](crate::parser#errors-and-recovery)), so that a mistake is no
//! other token and nothing is reported of it twice:
//!
//! - An unexpected character, a `'` with no name after it, and an
//!   ambiguous `-` are an error token of their own, one character.
//! - A malformed number literal (a misplaced `_`, an exponent with no
//!   digits, a literal too large for its type) is one error token with
//!   every ASCII letter, digit and `_` glued to it, a `.` followed by a
//!   digit or `_`, and a sign after an `e` or `E`, so that no part of it is
//!   lexed again as a number or a name.
//! - An unknown or invalid escape and an unmatched `}` in a string literal
//!   are reported and the literal goes on, the escape standing for nothing
//!   and the `}` for itself: the literal is a string as any is.
//! - A literal whose end cannot be found, unterminated or with an
//!   unterminated interpolation, or nested too deep, is reported by the
//!   scan, and the rest of the source, from it, is one error token: nothing
//!   after it is lexed, no context left open, and the parser reports nothing
//!   after it.
//!
//! The lexer records at most [`MAX_ERRORS`] errors and one more, which says
//! there are too many. [`tokenize`] returns the first error, as it always
//! did, or the tokens if there is none.
//!
//! # Comments
//!
//! A comment is no token, but a tool that rewrites source, the formatter
//! ([`crate::fmt`]), or shows what is written above a declaration needs to
//! know where each is. [`lex_with_comments`] lexes as [`lex`] does and also
//! records every comment's span, `#` through the end of its line, the `\n`
//! left out (trailing spaces and tabs are in it), in
//! [`Comments`], in source order: a comment inside an interpolation among
//! them, where it is. Only the lexing proper records them, where it skips
//! one (`Lexer::skip_trivia`); the scan of a string literal skips the same
//! comments without recording them, and the lexer then reaches each once.
//! [`lex`] and [`tokenize`] record none and pay nothing for them but one
//! test per comment.
//!
//! # Allocation
//!
//! Besides the output vector and the pieces, the lexer allocates only one
//! scratch `String`, reused for every numeric literal that contains `_`
//! separators and for building every piece, the bracket stack, the
//! interpolation stack, and the interning of each distinct name.

use crate::MAX_DEPTH;
use crate::diagnostic::{Diagnostic, InternalError};
use crate::names::{Name, Names};
use crate::source::{SourceFile, Span, offset_to_u32};
use crate::strings::{StrId, Strings};

/// One token: its kind (with the value, for literals) and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// The bytes it was lexed from. For [`TokenKind::Eof`], an empty span
    /// just after the last token other than a [`TokenKind::Newline`] (or at
    /// offset 0 if there is none), so that "found end of input" errors point
    /// at the end of the last line with code on it.
    ///
    /// A [`TokenKind::Str`] covers its whole literal, prefix and delimiters
    /// included. A [`TokenKind::StrStart`] covers the opening delimiter
    /// through the first interpolation's `{`, a [`TokenKind::StrMid`] one
    /// interpolation's `}` through the next one's `{`, and a
    /// [`TokenKind::StrEnd`] the last interpolation's `}` through the
    /// closing delimiter.
    pub span: Span,
}

/// The kind of a [`Token`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    /// An integer literal, already converted.
    Int(i64),
    /// A float literal, already converted.
    Float(f64),
    /// A string literal without interpolations: its whole text.
    Str(StrId),
    /// The text of an interpolated string literal before its first
    /// interpolation.
    StrStart(StrId),
    /// The text of an interpolated string literal between two
    /// interpolations.
    StrMid(StrId),
    /// The text of an interpolated string literal after its last
    /// interpolation.
    StrEnd(StrId),
    /// An identifier other than a keyword.
    Ident(Name),
    /// The keyword `let`.
    Let,
    /// The keyword `mut`, which after `let` declares a mutable variable.
    Mut,
    /// The keyword `if`.
    If,
    /// The keyword `else`.
    Else,
    /// The keyword `true`.
    True,
    /// The keyword `false`.
    False,
    /// The keyword `and`.
    And,
    /// The keyword `or`.
    Or,
    /// The keyword `not`.
    Not,
    /// The keyword `fn`.
    Fn,
    /// The keyword `def`, which begins a public function item.
    Def,
    /// The keyword `import`, which begins an import item of a file's
    /// outermost block.
    Import,
    /// The keyword `lazy`, which defers the expression after it.
    Lazy,
    /// The keyword `await`, which awaits the value after it.
    Await,
    /// The keyword `for`, which starts a `for` expression, and follows the
    /// body of a `do`.
    For,
    /// The keyword `do`, which starts a `do` loop: its body, then `for`
    /// and its condition.
    Do,
    /// The keyword `break`, an item that ends a loop.
    Break,
    /// The keyword `continue`, an item that ends an iteration of a loop.
    Continue,
    /// The keyword `return`, an item that leaves a function, a `lazy`, or
    /// the block or loop its label names, with a value.
    Return,
    /// A loop's or a block's label: `'` directly followed by a name, as in
    /// `'outer`, holding the name, interned as an identifier is (a
    /// keyword's spelling included, since the `'` makes it no keyword).
    Label(Name),
    /// An outer attribute, `#[NAME]`, holding its name (see
    /// [Attributes](#attributes)): it applies to the item after it.
    Attribute(Name),
    /// An inner attribute, `#![NAME]`, holding its name (see
    /// [Attributes](#attributes)): it applies to the whole file.
    InnerAttribute(Name),
    /// The keyword `in`, between a `for`'s variable and what it iterates
    /// over.
    In,
    /// The keyword `is`, which tests whether the value before it is of the
    /// type after it.
    Is,
    /// The keyword `as`, which narrows the value before it to the type
    /// after it, checked at runtime.
    As,
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
    /// `==`
    EqEq,
    /// `!=`
    BangEq,
    /// `===`
    EqEqEq,
    /// `!==`
    BangEqEq,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `:`
    Colon,
    /// `:=`
    ColonEq,
    /// `->`, which marks an impure function's return type.
    Arrow,
    /// `->>`, which marks a memoised impure function's return type.
    MemoArrow,
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
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `;`
    Semi,
    /// `.` not inside a number literal nor among two or three: field access.
    Dot,
    /// `...`, which spreads a list into the list literal it is an element
    /// of.
    DotDotDot,
    /// `..`, the range of the integers from its left operand up to but not
    /// including its right one.
    DotDot,
    /// `..=`, the range of the integers from its left operand up to and
    /// including its right one.
    DotDotEq,
    /// `,`, which separates the fields of a record type, the parameters of
    /// a function, the arguments of a call and the elements of a list.
    Comma,
    /// `|`, which separates the members of a union type.
    Pipe,
    /// A `\n` outside parentheses, brackets and interpolations. Each newline
    /// is its own token.
    Newline,
    /// The source of a lexical error, reported already (see
    /// [Errors](#errors)): a character no token starts with, a malformed
    /// number literal, or, from a string literal whose end cannot be found,
    /// the rest of the source.
    Error,
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
            TokenKind::Mut => Some("mut"),
            TokenKind::If => Some("if"),
            TokenKind::Else => Some("else"),
            TokenKind::True => Some("true"),
            TokenKind::False => Some("false"),
            TokenKind::And => Some("and"),
            TokenKind::Or => Some("or"),
            TokenKind::Not => Some("not"),
            TokenKind::Fn => Some("fn"),
            TokenKind::Def => Some("def"),
            TokenKind::Import => Some("import"),
            TokenKind::Lazy => Some("lazy"),
            TokenKind::Await => Some("await"),
            TokenKind::For => Some("for"),
            TokenKind::Do => Some("do"),
            TokenKind::Break => Some("break"),
            TokenKind::Continue => Some("continue"),
            TokenKind::Return => Some("return"),
            TokenKind::In => Some("in"),
            TokenKind::Is => Some("is"),
            TokenKind::As => Some("as"),
            TokenKind::Plus => Some("+"),
            TokenKind::Minus | TokenKind::Neg => Some("-"),
            TokenKind::Star => Some("*"),
            TokenKind::Slash => Some("/"),
            TokenKind::SlashSlash => Some("//"),
            TokenKind::Percent => Some("%"),
            TokenKind::EqEq => Some("=="),
            TokenKind::BangEq => Some("!="),
            TokenKind::EqEqEq => Some("==="),
            TokenKind::BangEqEq => Some("!=="),
            TokenKind::Lt => Some("<"),
            TokenKind::Le => Some("<="),
            TokenKind::Gt => Some(">"),
            TokenKind::Ge => Some(">="),
            TokenKind::Colon => Some(":"),
            TokenKind::ColonEq => Some(":="),
            TokenKind::Arrow => Some("->"),
            TokenKind::MemoArrow => Some("->>"),
            TokenKind::Eq => Some("="),
            TokenKind::LParen => Some("("),
            TokenKind::RParen => Some(")"),
            TokenKind::LBrace => Some("{"),
            TokenKind::RBrace => Some("}"),
            TokenKind::LBracket => Some("["),
            TokenKind::RBracket => Some("]"),
            TokenKind::Semi => Some(";"),
            TokenKind::Dot => Some("."),
            TokenKind::DotDotDot => Some("..."),
            TokenKind::DotDot => Some(".."),
            TokenKind::DotDotEq => Some("..="),
            TokenKind::Comma => Some(","),
            TokenKind::Pipe => Some("|"),
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Str(_)
            | TokenKind::StrStart(_)
            | TokenKind::StrMid(_)
            | TokenKind::StrEnd(_)
            | TokenKind::Ident(_)
            | TokenKind::Label(_)
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_)
            | TokenKind::Newline
            | TokenKind::Error
            | TokenKind::Eof => None,
        }
    }
}

/// The keyword `text` spells, if it is one.
fn keyword(text: &str) -> Option<TokenKind> {
    Some(match text {
        "let" => TokenKind::Let,
        "mut" => TokenKind::Mut,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        "fn" => TokenKind::Fn,
        "def" => TokenKind::Def,
        "import" => TokenKind::Import,
        "lazy" => TokenKind::Lazy,
        "await" => TokenKind::Await,
        "for" => TokenKind::For,
        "do" => TokenKind::Do,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "return" => TokenKind::Return,
        "in" => TokenKind::In,
        "is" => TokenKind::Is,
        "as" => TokenKind::As,
        _ => return None,
    })
}

/// Whether `text` is a plain identifier: exactly one [`TokenKind::Ident`]
/// token, so a name that can be written in source without quotes. Keywords
/// are not identifiers.
#[must_use]
pub fn is_identifier(text: &str) -> bool {
    let bytes = text.as_bytes();
    matches!(bytes.first(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && identifier_end(bytes, 0) == bytes.len()
        && keyword(text).is_none()
}

/// Lexes the whole of `source`, interning its names into a new table and
/// storing the text of its string literals in another.
///
/// The returned vector always ends with exactly one [`TokenKind::Eof`].
///
/// # Errors
///
/// Returns the first lexical error (see [Errors](#errors)): an unexpected
/// character, an ambiguous `-`, a malformed number (misplaced `_`, missing
/// exponent digits), a literal too large for its type, or a malformed
/// string literal (unterminated, an unknown or invalid escape, a stray `}`,
/// an unterminated interpolation, or literals nested more than
/// [`MAX_DEPTH`] deep in each other's interpolations). [`lex`] gives every
/// error, with the tokens.
pub fn tokenize(source: &SourceFile) -> Result<(Vec<Token>, Names, Strings), Diagnostic> {
    let mut names = Names::new();
    let (tokens, strings) = tokenize_with(source, &mut names)?;
    Ok((tokens, names, strings))
}

/// Like [`tokenize`], interning names into an existing table, so that they
/// compare equal to names lexed earlier into it. The string table is always
/// new, since it belongs to this input alone.
///
/// On error, names met before the error remain interned.
///
/// # Errors
///
/// As [`tokenize`].
pub fn tokenize_with(
    source: &SourceFile,
    names: &mut Names,
) -> Result<(Vec<Token>, Strings), Diagnostic> {
    let lexed = lex(source, names);
    match lexed.diagnostics.into_iter().next() {
        Some(first) => Err(first),
        None => Ok((lexed.tokens, lexed.strings)),
    }
}

/// What [`lex`] makes of a source: every token, the text of its string
/// literals, and every lexical error.
#[derive(Debug)]
pub struct Lexed {
    /// The tokens, ending with exactly one [`TokenKind::Eof`]; the source
    /// of each lexical error a [`TokenKind::Error`].
    pub tokens: Vec<Token>,
    /// The text of the string literals.
    pub strings: Strings,
    /// The lexical errors, in source order, at most [`MAX_ERRORS`] and one
    /// more: the lexer stops recording them there.
    pub diagnostics: Vec<Diagnostic>,
}

/// The most errors reported of one source, its lexer's and its parser's
/// together (see [`crate::parser::parse_all`]): past them, one more says
/// that there are too many.
pub const MAX_ERRORS: usize = 100;

/// Lexes the whole of `source`, as [`tokenize_with`] does, interning names
/// into `names`, but going on past every lexical error (see
/// [Errors](#errors)): the tokens, with a [`TokenKind::Error`] where each
/// error is, and every error.
pub fn lex(source: &SourceFile, names: &mut Names) -> Lexed {
    lex_into(Lexer::new(source, names, None)).0
}

/// Lexes `source` as [`lex`] does, and also records where every comment is
/// (see [Comments](#comments)).
pub fn lex_with_comments(source: &SourceFile, names: &mut Names) -> (Lexed, Comments) {
    let (lexed, spans) = lex_into(Lexer::new(source, names, Some(Vec::new())));
    (
        lexed,
        Comments {
            spans: spans.unwrap_or_default(),
        },
    )
}

/// Runs `lexer` to the end of its source: what it made of it, and the
/// comments it recorded, if it was asked to.
fn lex_into(mut lexer: Lexer<'_>) -> (Lexed, Option<Vec<Span>>) {
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token();
        tokens.push(token);
        if token.kind == TokenKind::Eof {
            let made = Lexed {
                tokens,
                strings: lexer.strings,
                diagnostics: lexer.diagnostics,
            };
            return (made, lexer.comments);
        }
    }
}

/// Where the comments of a source are, as [`lex_with_comments`] finds them:
/// each one's span, `#` through the end of its line, the `\n` left out, in
/// source order, so that the comments around a place are found by binary
/// search, as the run of `#` lines directly above a declaration is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comments {
    /// The spans, in source order.
    spans: Vec<Span>,
}

impl Comments {
    /// Every comment's span, in source order.
    #[must_use]
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The comments that end at or before `offset`, in source order: the
    /// last of them are the nearest above it.
    #[must_use]
    pub fn before(&self, offset: u32) -> &[Span] {
        let end = self.spans.partition_point(|span| span.end <= offset);
        self.spans.get(..end).unwrap_or_default()
    }

    /// The comments that start at or after `offset`, in source order.
    #[must_use]
    pub fn after(&self, offset: u32) -> &[Span] {
        let start = self.spans.partition_point(|span| span.start < offset);
        self.spans.get(start..).unwrap_or_default()
    }

    /// The comment that `offset` is in, if any: from its `#` up to the end
    /// of its line.
    #[must_use]
    pub fn at(&self, offset: u32) -> Option<Span> {
        let after = self.spans.partition_point(|span| span.start <= offset);
        let span = *self.spans.get(after.checked_sub(1)?)?;
        (offset < span.end).then_some(span)
    }
}

/// The opening delimiter of a string literal, as [`Lexer::delimiter`]
/// finds it.
#[derive(Clone, Copy, Debug)]
struct Delimiter {
    /// Whether the literal has the `r` prefix.
    raw: bool,
    /// Offset of the first quote.
    quotes_start: usize,
    /// The number of quotes in each of the literal's delimiters.
    quotes: usize,
    /// Offset of the first byte of content, just after the opening quotes.
    content_start: usize,
}

/// How the lines of a literal's content lose their indentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Indent {
    /// Not at all: the content does not start with a line break.
    None,
    /// Every line loses the common indentation, the `len` bytes of source
    /// at offset `start`. A non-blank line always starts with them; a blank
    /// line loses them if it starts with them and becomes empty otherwise.
    Common {
        /// Offset of a line that starts with the common indentation.
        start: usize,
        /// Length of the common indentation in bytes.
        len: usize,
    },
    /// Every line is blank, so every line becomes empty.
    AllBlank,
}

/// Where the parts of a string literal are, as the scan finds them.
#[derive(Clone, Copy, Debug)]
struct Layout {
    /// Whether the literal is raw: no escapes and no interpolations.
    raw: bool,
    /// Offset where the text to produce starts: just after the opening
    /// delimiter, or just after the first line break when the literal is
    /// stripped.
    text_start: usize,
    /// Offset where the text to produce ends: the closing delimiter, or the
    /// final line break when stripping removes it.
    text_end: usize,
    /// Offset of the closing delimiter.
    content_end: usize,
    /// Offset just after the closing delimiter.
    end: usize,
    /// How the content's lines are stripped.
    indent: Indent,
}

/// An interpolation being lexed: a context entered at the `{` of a non-raw
/// string literal and left at the matching `}`.
#[derive(Clone, Copy, Debug)]
struct Interpolation {
    /// The literal it belongs to, whose text resumes after the `}`.
    layout: Layout,
    /// Offset of the `{`.
    open: usize,
    /// Number of `{` inside the interpolation not yet closed. A `}` at
    /// depth zero ends the interpolation.
    brace_depth: u32,
    /// The length of the bracket stack when the interpolation began: the
    /// brackets above it are the ones open inside the interpolation, and
    /// the stack is cut back to it when the interpolation ends, so that a
    /// `(` left open inside cannot affect the enclosing context.
    brackets_start: usize,
}

/// An open bracket on the lexer's stack, which decides whether newlines
/// are significant: they are trivia while a `(` or a `[` is on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bracket {
    /// `(`
    Paren,
    /// `[` of a list literal or an index.
    Square,
    /// `{` of a block or a record type; not an interpolation's `{`, which
    /// is a context of its own.
    Brace,
}

/// Why pass 2 stopped building a piece.
#[derive(Clone, Copy, Debug)]
enum Stop {
    /// At the end of the literal's text.
    End,
    /// At the `{` of an interpolation, at this offset.
    Interpolation(usize),
}

/// The scan's running measurement of a literal's indentation.
#[derive(Clone, Copy, Debug)]
struct Measure {
    /// Whether the literal is stripped at all. If not, nothing is measured.
    active: bool,
    /// The common indentation of the non-blank lines measured so far, as
    /// the offset of a line starting with it and its length. `None` until a
    /// non-blank line is measured.
    common: Option<(usize, usize)>,
}

impl Measure {
    /// Measures the line after the line break at `newline`.
    ///
    /// A line is blank if only spaces and tabs precede its end. When
    /// `own_quotes` is `Some(n)`, the line break is in the text of the
    /// literal being measured, whose delimiters have `n` quotes, and the
    /// line may also end at that literal's closing delimiter. Elsewhere,
    /// the closing delimiter of a nested literal is part of the measured
    /// literal's line, so such a line is not blank.
    fn line(&mut self, bytes: &[u8], newline: usize, own_quotes: Option<usize>) {
        if !self.active {
            return;
        }
        let start = newline + 1;
        let indent = whitespace_len(bytes, start);
        let after = start + indent;
        let blank = match bytes.get(after) {
            None | Some(b'\n') => true,
            Some(b'"') => own_quotes.is_some_and(|quotes| quote_run(bytes, after) >= quotes),
            Some(_) => false,
        };
        if blank {
            return;
        }
        self.common = Some(match self.common {
            None => (start, indent),
            Some((common, len)) => {
                let shared = common_prefix_len(
                    bytes.get(common..common + len).unwrap_or_default(),
                    bytes.get(start..after).unwrap_or_default(),
                );
                (common, shared)
            }
        });
    }
}

/// Lexer state. Produces one token per [`Lexer::next_token`] call.
struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    /// Byte offset of the next unread byte. Always on a character boundary:
    /// outside comments and string literals the lexer only consumes ASCII
    /// bytes, but a character it does not recognise, whole, a comment is
    /// skipped up to a `\n` or the end of the text, and every piece of a
    /// string literal ends at an ASCII `{` or quote. Slicing `text` at `pos`
    /// is therefore safe.
    pos: usize,
    /// End of the most recent token other than a newline, used to place the
    /// `Eof` token.
    last_end: usize,
    /// The open brackets, outermost first, across every context: the
    /// innermost open one, within the current context, decides whether
    /// newlines are significant (see `Lexer::skip_trivia`).
    brackets: Vec<Bracket>,
    /// The interpolations being lexed, innermost last.
    interpolations: Vec<Interpolation>,
    /// Reused buffer for literals whose `_` separators must be removed and
    /// for building each piece of a string literal.
    scratch: String,
    /// Where identifiers are interned.
    names: &'a mut Names,
    /// Where the pieces of string literals are stored.
    strings: Strings,
    /// The errors found so far, in source order, at most [`MAX_ERRORS`] and
    /// one more.
    diagnostics: Vec<Diagnostic>,
    /// The comments skipped so far, in source order, if they are recorded
    /// (see [Comments](#comments)).
    comments: Option<Vec<Span>>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a SourceFile, names: &'a mut Names, comments: Option<Vec<Span>>) -> Self {
        Self {
            text: source.text(),
            bytes: source.text().as_bytes(),
            pos: 0,
            last_end: 0,
            brackets: Vec::new(),
            interpolations: Vec::new(),
            scratch: String::new(),
            names,
            strings: Strings::new(),
            diagnostics: Vec::new(),
            comments,
        }
    }

    /// Records `error`, unless [`MAX_ERRORS`] and one more are recorded
    /// already, which says there are too many.
    fn report(&mut self, error: Diagnostic) {
        if self.diagnostics.len() <= MAX_ERRORS {
            self.diagnostics.push(error);
        }
    }

    /// The next token: [`Lexer::lex_token`]'s, or where it fails, the
    /// error recorded and a [`TokenKind::Error`] in place of the source it
    /// covers (see [Errors](#errors)).
    fn next_token(&mut self) -> Token {
        self.skip_trivia();
        let start = self.pos;
        match self.lex_token(start) {
            Ok(token) => token,
            Err(error) => self.recover(start, error),
        }
    }

    /// Records `error`, found lexing the token starting at `start`, and
    /// returns the [`TokenKind::Error`] that stands for its source, the
    /// lexer moved past it: a malformed number literal whole, with every
    /// letter, digit, `_` and `.` glued to it; the rest of the source from
    /// a string literal, or the `}` that resumes one, since its end cannot
    /// be found; one character otherwise.
    fn recover(&mut self, start: usize, error: Diagnostic) -> Token {
        self.report(error);
        let len = self.bytes.len();
        let end = match self.bytes.get(start) {
            Some(b'0'..=b'9') => malformed_number_end(self.bytes, start),
            Some(b'"' | b'}') | None => len,
            Some(b'r') if self.bytes.get(start + 1) == Some(&b'"') => len,
            Some(_) => {
                let ch = self.text.get(start..).and_then(|rest| rest.chars().next());
                start + ch.map_or(1, char::len_utf8)
            }
        };
        // Always past `start`, and on a character boundary: the end of the
        // text, or past ASCII bytes or a whole character.
        let end = end.max(start + 1).min(len);
        if end >= len {
            // Nothing after it is lexed: no context is left open.
            self.interpolations.clear();
            self.brackets.clear();
        }
        self.pos = end;
        self.last_end = end;
        Token {
            kind: TokenKind::Error,
            span: span(start, end),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    /// Lexes the token starting at `start`, the next unread byte after the
    /// trivia, which [`Lexer::next_token`] has skipped.
    fn lex_token(&mut self, start: usize) -> Result<Token, Diagnostic> {
        let Some(byte) = self.peek() else {
            // The scan checks that every interpolation of a literal closes
            // before the literal is lexed, so this only guards against the
            // two passes disagreeing.
            if let Some(interpolation) = self.interpolations.last() {
                return Err(unterminated_interpolation(interpolation.open));
            }
            return Ok(Token {
                kind: TokenKind::Eof,
                span: span(self.last_end, self.last_end),
            });
        };
        let kind = match byte {
            // `skip_trivia` only stops at a newline where it is significant.
            b'\n' => {
                self.pos += 1;
                return Ok(Token {
                    kind: TokenKind::Newline,
                    span: span(start, self.pos),
                });
            }
            b'"' => self.string(start)?,
            b'r' if self.peek_at(1) == Some(b'"') => self.string(start)?,
            b'{' => {
                if let Some(interpolation) = self.interpolations.last_mut() {
                    interpolation.brace_depth = interpolation.brace_depth.saturating_add(1);
                }
                self.brackets.push(Bracket::Brace);
                self.single(TokenKind::LBrace)
            }
            b'}' => self.close_brace(start)?,
            b'(' => {
                self.brackets.push(Bracket::Paren);
                self.single(TokenKind::LParen)
            }
            b')' => {
                // An unmatched `)` closes nothing, in this context or an
                // enclosing one; the parser reports it.
                if self.context_brackets().last() == Some(&Bracket::Paren) {
                    self.brackets.pop();
                }
                self.single(TokenKind::RParen)
            }
            b'[' => {
                self.brackets.push(Bracket::Square);
                self.single(TokenKind::LBracket)
            }
            b']' => {
                // As for `)`.
                if self.context_brackets().last() == Some(&Bracket::Square) {
                    self.brackets.pop();
                }
                self.single(TokenKind::RBracket)
            }
            b'0'..=b'9' => self.number(start)?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.identifier(start),
            b'\'' => self.label(start)?,
            // `skip_trivia` only stops at a `#` that starts an attribute.
            b'#' => self.attribute(start)?,
            _ => self.operator(start)?,
        };
        self.last_end = self.pos;
        Ok(Token {
            kind,
            span: span(start, self.pos),
        })
    }

    /// Lexes an operator or punctuation token starting at `start`.
    fn operator(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        let Some(byte) = self.peek() else {
            return Err(self.unexpected_character());
        };
        // For operators that are one byte alone or two with a following `=`.
        let with_eq = |alone: TokenKind, with: TokenKind| {
            if self.peek_at(1) == Some(b'=') {
                (with, 2)
            } else {
                (alone, 1)
            }
        };
        let (kind, len) = match byte {
            b'+' => (TokenKind::Plus, 1),
            b'-' => return self.minus(start),
            b'*' => (TokenKind::Star, 1),
            b'%' => (TokenKind::Percent, 1),
            b';' => (TokenKind::Semi, 1),
            // A `.` inside a number literal has been consumed by `number`.
            b'.' if self.peek_at(1) == Some(b'.') => match self.peek_at(2) {
                Some(b'.') => (TokenKind::DotDotDot, 3),
                Some(b'=') => (TokenKind::DotDotEq, 3),
                _ => (TokenKind::DotDot, 2),
            },
            b'.' => (TokenKind::Dot, 1),
            b',' => (TokenKind::Comma, 1),
            b'|' => (TokenKind::Pipe, 1),
            b'=' if self.peek_at(1) == Some(b'=') && self.peek_at(2) == Some(b'=') => {
                (TokenKind::EqEqEq, 3)
            }
            b'=' => with_eq(TokenKind::Eq, TokenKind::EqEq),
            b':' => with_eq(TokenKind::Colon, TokenKind::ColonEq),
            b'<' => with_eq(TokenKind::Lt, TokenKind::Le),
            b'>' => with_eq(TokenKind::Gt, TokenKind::Ge),
            b'!' if self.peek_at(1) == Some(b'=') && self.peek_at(2) == Some(b'=') => {
                (TokenKind::BangEqEq, 3)
            }
            b'!' if self.peek_at(1) == Some(b'=') => (TokenKind::BangEq, 2),
            b'/' if self.peek_at(1) == Some(b'/') => (TokenKind::SlashSlash, 2),
            b'/' => (TokenKind::Slash, 1),
            _ => return Err(self.unexpected_character()),
        };
        self.pos += len;
        Ok(kind)
    }

    /// Consumes a one-byte token.
    fn single(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    /// Lexes a `}` at `start`: the end of the innermost interpolation if it
    /// matches that interpolation's `{`, and a [`TokenKind::RBrace`]
    /// otherwise, closing the nearest open `{` on the bracket stack with
    /// any `(` or `[` left open inside it. An unmatched `}` closes nothing;
    /// the parser reports it.
    fn close_brace(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        if let Some(interpolation) = self.interpolations.last_mut() {
            if interpolation.brace_depth > 0 {
                interpolation.brace_depth -= 1;
            } else {
                let (layout, brackets_start) = (interpolation.layout, interpolation.brackets_start);
                self.interpolations.pop();
                self.brackets.truncate(brackets_start);
                return self.piece(layout, start + 1, false);
            }
        }
        let context_start = self.context_start();
        if let Some(brace) = self
            .context_brackets()
            .iter()
            .rposition(|&bracket| bracket == Bracket::Brace)
        {
            self.brackets.truncate(context_start + brace);
        }
        Ok(self.single(TokenKind::RBrace))
    }

    /// Where the current context's brackets start on the bracket stack: the
    /// innermost interpolation's start, or the bottom outside interpolations.
    fn context_start(&self) -> usize {
        self.interpolations
            .last()
            .map_or(0, |interpolation| interpolation.brackets_start)
    }

    /// The brackets open in the current context, outermost first.
    fn context_brackets(&self) -> &[Bracket] {
        self.brackets
            .get(self.context_start()..)
            .unwrap_or_default()
    }

    /// Skips whitespace and comments, stopping at a `\n` that must become a
    /// [`TokenKind::Newline`] token: one not directly inside a `(`, a `[`
    /// or an interpolation, which is decided by the innermost open bracket
    /// of the current context, or by the context itself if it has none open.
    fn skip_trivia(&mut self) {
        let newlines_are_trivia = match self.context_brackets().last() {
            Some(Bracket::Paren | Bracket::Square) => true,
            Some(Bracket::Brace) => false,
            None => !self.interpolations.is_empty(),
        };
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' if newlines_are_trivia => self.pos += 1,
                // An attribute is a token (see [Attributes](#attributes)).
                b'#' if attribute_at(self.bytes, self.pos).is_some() => break,
                // Skip to the newline, which is then either skipped or
                // emitted like any other.
                b'#' => {
                    let end = end_of_line(self.bytes, self.pos);
                    if let Some(comments) = &mut self.comments {
                        comments.push(span(self.pos, end));
                    }
                    self.pos = end;
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
        self.pos = identifier_end(self.bytes, start);
        let text = &self.text[start..self.pos];
        keyword(text).unwrap_or_else(|| TokenKind::Ident(self.names.intern(text)))
    }

    /// Lexes a label starting at `start`, which holds a `'`: the `'` and
    /// the name directly after it, scanned as an identifier is, whose text
    /// is interned whatever it spells. A `'` not directly followed by the
    /// start of a name is an unexpected character.
    fn label(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        if !matches!(self.peek_at(1), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')) {
            return Err(self.unexpected_character());
        }
        self.pos = identifier_end(self.bytes, start + 1);
        let text = &self.text[start + 1..self.pos];
        Ok(TokenKind::Label(self.names.intern(text)))
    }

    /// Lexes the attribute starting at `start`, which holds a `#` that
    /// `skip_trivia` found starts one (see [Attributes](#attributes)): an
    /// [`TokenKind::Attribute`] or an [`TokenKind::InnerAttribute`], its name
    /// interned as an identifier's is.
    fn attribute(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        let Some((inner, (from, to), end)) = attribute_at(self.bytes, start) else {
            return Err(self.unexpected_character());
        };
        let name = self
            .names
            .intern(self.text.get(from..to).unwrap_or_default());
        self.pos = end;
        Ok(if inner {
            TokenKind::InnerAttribute(name)
        } else {
            TokenKind::Attribute(name)
        })
    }

    /// Classifies a `-` at `start` that is not part of an identifier or a
    /// number literal, from the bytes on either side of it:
    ///
    /// 1. Directly followed by `>>`: the memo arrow,
    ///    [`TokenKind::MemoArrow`]; otherwise directly followed by `>`: the
    ///    arrow, [`TokenKind::Arrow`]; whatever precedes it, so `a->b`,
    ///    `x -> y` and `)->{` all have an arrow, and `)->>{` a memo arrow.
    /// 2. Directly after an operand end (an identifier character, `)` or
    ///    `]`): an error, since `a-b` would be a single name and `a -b` a
    ///    negation.
    /// 3. Otherwise, followed by whitespace or the end of input:
    ///    subtraction, [`TokenKind::Minus`].
    /// 4. Otherwise, directly followed by an operand start (a letter, digit,
    ///    `_`, `(`, `[`, `{` or `"`): negation, [`TokenKind::Neg`].
    /// 5. Otherwise, as in `--x`: an unexpected character.
    fn minus(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        if self.peek_at(1) == Some(b'>') {
            if self.peek_at(2) == Some(b'>') {
                self.pos += 3;
                return Ok(TokenKind::MemoArrow);
            }
            self.pos += 2;
            return Ok(TokenKind::Arrow);
        }
        let previous = start.checked_sub(1).and_then(|at| self.bytes.get(at));
        if previous.is_some_and(|&byte| is_identifier_byte(byte) || matches!(byte, b')' | b']')) {
            return Err(
                Diagnostic::error("`-` here is ambiguous", span(start, start + 1)).with_note(
                    "write `a - b` with spaces for subtraction, or `a-b` for a single name",
                ),
            );
        }
        match self.peek_at(1) {
            None | Some(b' ' | b'\t' | b'\r' | b'\n') => Ok(self.single(TokenKind::Minus)),
            Some(byte) if is_identifier_byte(byte) || matches!(byte, b'(' | b'[' | b'{' | b'"') => {
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

    /// Lexes the string literal starting at `start`, at its `r` prefix or
    /// first quote: scans it, then produces its first piece.
    fn string(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        let layout = self.scan(start, self.interpolations.len() + 1)?;
        self.piece(layout, layout.text_start, true)
    }

    /// The opening delimiter of the literal starting at `start`, which holds
    /// its `r` prefix or its first quote.
    fn delimiter(&self, start: usize) -> Delimiter {
        let raw = self.bytes.get(start) == Some(&b'r');
        let quotes_start = start + usize::from(raw);
        let run = quote_run(self.bytes, quotes_start);
        // `""` is the empty string, delimited by one quote on each side.
        // Every other run opens the literal whole.
        let quotes = if run == 2 { 1 } else { run };
        Delimiter {
            raw,
            quotes_start,
            quotes,
            content_start: quotes_start + quotes,
        }
    }

    // ----- Pass 1: the scan. -----

    /// Scans the literal starting at `start`, whose nesting `level` is 1
    /// outside any interpolation and one more for each interpolation it is
    /// in, and returns its layout. Allocates nothing.
    fn scan(&self, start: usize, level: usize) -> Result<Layout, Diagnostic> {
        let delimiter = self.delimiter(start);
        if level > MAX_DEPTH {
            return Err(too_deep(start, &delimiter));
        }
        // Stripping applies when the content starts with a line break,
        // optionally after spaces and tabs.
        let first_break =
            delimiter.content_start + whitespace_len(self.bytes, delimiter.content_start);
        let stripped = self.bytes.get(first_break) == Some(&b'\n');
        let mut measure = Measure {
            active: stripped,
            common: None,
        };
        let content_end = self.skip_content(&delimiter, level, &mut measure, true)?;
        let end = content_end + delimiter.quotes;
        if !stripped {
            return Ok(Layout {
                raw: delimiter.raw,
                text_start: delimiter.content_start,
                text_end: content_end,
                content_end,
                end,
                indent: Indent::None,
            });
        }
        let text_start = first_break + 1;
        let indent = match measure.common {
            Some((start, len)) => Indent::Common { start, len },
            None => Indent::AllBlank,
        };
        Ok(Layout {
            raw: delimiter.raw,
            text_start,
            text_end: self.stripped_end(&delimiter, text_start, content_end),
            content_end,
            end,
            indent,
        })
    }

    /// Where the text of a stripped literal ends: at its final line break
    /// if only spaces and tabs follow it before the closing delimiter and it
    /// is not the first line break, which stripping removes anyway;
    /// otherwise at the closing delimiter.
    ///
    /// A line break escaped by a `\` is not removed here: the escape is a
    /// line continuation, which removes it itself, and removing it here
    /// would leave the `\` escaping nothing.
    fn stripped_end(&self, delimiter: &Delimiter, text_start: usize, content_end: usize) -> usize {
        let mut last_line = content_end;
        while last_line > text_start && matches!(self.bytes.get(last_line - 1), Some(b' ' | b'\t'))
        {
            last_line -= 1;
        }
        if last_line <= text_start || self.bytes.get(last_line - 1) != Some(&b'\n') {
            return content_end;
        }
        let line_break = last_line - 1;
        // In a run of backslashes, each pair is one escaped backslash, so an
        // odd run ends with a backslash escaping the line break.
        let backslashes = self
            .bytes
            .get(text_start..line_break)
            .unwrap_or_default()
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'\\')
            .count();
        if !delimiter.raw && !backslashes.is_multiple_of(2) {
            content_end
        } else {
            line_break
        }
    }

    /// Walks a literal's content to its closing delimiter and returns the
    /// delimiter's offset. `own` says whether this is the literal being
    /// measured rather than one nested in it.
    fn skip_content(
        &self,
        delimiter: &Delimiter,
        level: usize,
        measure: &mut Measure,
        own: bool,
    ) -> Result<usize, Diagnostic> {
        let own_quotes = own.then_some(delimiter.quotes);
        let mut pos = delimiter.content_start;
        loop {
            let Some(&byte) = self.bytes.get(pos) else {
                return Err(unterminated_string(delimiter));
            };
            pos = match byte {
                b'"' => {
                    let run = quote_run(self.bytes, pos);
                    if run >= delimiter.quotes {
                        return Ok(pos);
                    }
                    pos + run
                }
                b'\n' => {
                    measure.line(self.bytes, pos, own_quotes);
                    pos + 1
                }
                b'\\' if !delimiter.raw => match self.bytes.get(pos + 1) {
                    Some(b'u') => unicode_escape(self.bytes, pos).0,
                    Some(b'\n') => {
                        measure.line(self.bytes, pos + 1, own_quotes);
                        pos + 2
                    }
                    // Any other escape is two bytes. If the second is part
                    // of a multi-byte character, the rest of it is skipped
                    // byte by byte: continuation bytes are never special.
                    Some(_) => pos + 2,
                    None => pos + 1,
                },
                b'{' if !delimiter.raw => {
                    if self.bytes.get(pos + 1) == Some(&b'{') {
                        pos + 2
                    } else {
                        self.skip_interpolation(pos, level, measure)?
                    }
                }
                _ => pos + 1,
            };
        }
    }

    /// Walks the interpolation whose `{` is at `open`, in a literal at
    /// nesting `level`, and returns the offset just after its matching `}`.
    fn skip_interpolation(
        &self,
        open: usize,
        level: usize,
        measure: &mut Measure,
    ) -> Result<usize, Diagnostic> {
        let mut depth = 0_usize;
        let mut pos = open + 1;
        loop {
            let Some(&byte) = self.bytes.get(pos) else {
                return Err(unterminated_interpolation(open));
            };
            pos = match byte {
                b'\n' => {
                    measure.line(self.bytes, pos, None);
                    pos + 1
                }
                b'#' => attribute_at(self.bytes, pos)
                    .map_or_else(|| end_of_line(self.bytes, pos), |(_, _, end)| end),
                b'{' => {
                    depth += 1;
                    pos + 1
                }
                b'}' if depth == 0 => return Ok(pos + 1),
                b'}' => {
                    depth -= 1;
                    pos + 1
                }
                b'"' => self.skip_nested(pos, level + 1, measure)?,
                b'r' if self.bytes.get(pos + 1) == Some(&b'"') => {
                    self.skip_nested(pos, level + 1, measure)?
                }
                b'0'..=b'9' => number_end(self.bytes, pos),
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => identifier_end(self.bytes, pos),
                _ => pos + 1,
            };
        }
    }

    /// Walks a literal nested at `level` inside the one being scanned,
    /// starting at `start`, and returns the offset just after it.
    fn skip_nested(
        &self,
        start: usize,
        level: usize,
        measure: &mut Measure,
    ) -> Result<usize, Diagnostic> {
        let delimiter = self.delimiter(start);
        if level > MAX_DEPTH {
            return Err(too_deep(start, &delimiter));
        }
        let content_end = self.skip_content(&delimiter, level, measure, false)?;
        Ok(content_end + delimiter.quotes)
    }

    // ----- Pass 2: the pieces. -----

    /// Produces the piece of the literal laid out by `layout` that starts at
    /// `from`: the literal's first piece if `first`, or the piece after an
    /// interpolation's `}`. Stores its text and returns its token kind,
    /// leaving the lexer after the piece: inside the next interpolation,
    /// with its context pushed, or after the literal.
    fn piece(&mut self, layout: Layout, from: usize, first: bool) -> Result<TokenKind, Diagnostic> {
        // The first piece starts the literal's first content line; a piece
        // after an interpolation starts mid-line.
        let stop = self.build_piece(&layout, from, first)?;
        let id = self.strings.push(&self.scratch);
        Ok(match stop {
            Stop::End => {
                self.pos = layout.end;
                if first {
                    TokenKind::Str(id)
                } else {
                    TokenKind::StrEnd(id)
                }
            }
            Stop::Interpolation(open) => {
                self.pos = open + 1;
                self.interpolations.push(Interpolation {
                    layout,
                    open,
                    brace_depth: 0,
                    brackets_start: self.brackets.len(),
                });
                if first {
                    TokenKind::StrStart(id)
                } else {
                    TokenKind::StrMid(id)
                }
            }
        })
    }

    /// Builds the text of the piece starting at `from` in `self.scratch`,
    /// stripping indentation at line starts and replacing escapes, and
    /// returns where it stopped. `line_start` says whether `from` is the
    /// start of a line.
    fn build_piece(
        &mut self,
        layout: &Layout,
        from: usize,
        line_start: bool,
    ) -> Result<Stop, Diagnostic> {
        self.scratch.clear();
        let mut pos = from;
        let mut at_line_start = line_start;
        loop {
            if at_line_start {
                pos = self.strip_indentation(layout, pos);
                at_line_start = false;
            }
            // Copy the run of ordinary bytes up to the next one that needs
            // attention in one go.
            let rest = self.bytes.get(pos..layout.text_end).unwrap_or_default();
            let run = rest
                .iter()
                .position(|&byte| {
                    byte == b'\n' || (!layout.raw && matches!(byte, b'\\' | b'{' | b'}'))
                })
                .unwrap_or(rest.len());
            let Some(text) = self.text.get(pos..pos + run) else {
                return Err(internal(
                    "a string piece is not on character boundaries",
                    pos,
                ));
            };
            self.scratch.push_str(text);
            pos += run;
            if pos >= layout.text_end {
                return Ok(Stop::End);
            }
            let next_is =
                |byte: u8| pos + 1 < layout.text_end && self.bytes.get(pos + 1) == Some(&byte);
            match self.bytes.get(pos) {
                Some(b'\n') => {
                    self.scratch.push('\n');
                    pos += 1;
                    at_line_start = true;
                }
                Some(b'\\') => pos = self.escape(layout, pos)?,
                Some(b'{') if next_is(b'{') => {
                    self.scratch.push('{');
                    pos += 2;
                }
                Some(b'{') => return Ok(Stop::Interpolation(pos)),
                Some(b'}') if next_is(b'}') => {
                    self.scratch.push('}');
                    pos += 2;
                }
                // Reported, and kept as the `}` it most likely means.
                Some(b'}') => {
                    self.report(
                        Diagnostic::error("unmatched `}` in string", span(pos, pos + 1))
                            .with_note("write `}}` for a literal `}`"),
                    );
                    self.scratch.push('}');
                    pos += 1;
                }
                _ => return Err(internal("a string piece stopped at an ordinary byte", pos)),
            }
        }
    }

    /// Strips the indentation of the line of the literal's text starting at
    /// `pos`, returning the offset where its text starts.
    fn strip_indentation(&self, layout: &Layout, pos: usize) -> usize {
        let indent = whitespace_len(self.bytes, pos);
        let (common, len) = match layout.indent {
            Indent::None => return pos,
            Indent::AllBlank => return pos + indent,
            Indent::Common { start, len } => (start, len),
        };
        let after = pos + indent;
        let blank =
            after >= layout.content_end || matches!(self.bytes.get(after), None | Some(b'\n'));
        if !blank {
            // A non-blank line has at least the common indentation.
            return pos + len.min(indent);
        }
        let has_common =
            indent >= len && self.bytes.get(pos..pos + len) == self.bytes.get(common..common + len);
        if has_common { pos + len } else { after }
    }

    /// Replaces the escape whose `\` is at `pos`, returning the offset after
    /// it. An unknown or invalid escape is reported and replaced by nothing,
    /// the literal going on after it (see [Errors](#errors)).
    fn escape(&mut self, layout: &Layout, pos: usize) -> Result<usize, Diagnostic> {
        let next = self
            .bytes
            .get(pos + 1)
            .copied()
            .filter(|_| pos + 1 < layout.text_end);
        let ch = match next {
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'\\') => '\\',
            Some(b'"') => '"',
            Some(b'u') => {
                let (end, value) = unicode_escape(self.bytes, pos);
                match value {
                    Some(ch) => self.scratch.push(ch),
                    None => {
                        self.report(Diagnostic::error("invalid unicode escape", span(pos, end)));
                    }
                }
                return Ok(end);
            }
            // A line continuation: the line break and the next line's
            // leading spaces and tabs are removed. Stripping has happened
            // first, so what it removes is whatever indentation the line has
            // beyond the common indentation.
            Some(b'\n') => return Ok(pos + 2 + whitespace_len(self.bytes, pos + 2)),
            Some(_) => {
                let (error, end) = self.unknown_escape(pos);
                self.report(error);
                return Ok(end);
            }
            // The scan never ends a piece's text just after an unescaped
            // `\`: before a closing quote it is an escaped quote, and before
            // a final line break it stops that line break being removed.
            None => return Err(internal("a string piece ends with a lone `\\`", pos)),
        };
        self.scratch.push(ch);
        Ok(pos + 2)
    }

    /// The error for an unknown escape whose `\` is at `pos`, and the offset
    /// just after the escape: after the character the `\` escapes.
    fn unknown_escape(&self, pos: usize) -> (Diagnostic, usize) {
        let ch = self
            .text
            .get(pos + 1..)
            .and_then(|rest| rest.chars().next())
            .unwrap_or('\u{FFFD}');
        // Show a control character by its escape, since it is invisible;
        // show anything else as written, even `'`, which `escape_debug`
        // would escape.
        let shown = if ch.is_control() {
            ch.escape_debug().to_string()
        } else {
            ch.to_string()
        };
        let end = pos + 1 + ch.len_utf8();
        (
            Diagnostic::error(format!("unknown escape `\\{shown}`"), span(pos, end)),
            end,
        )
    }
}

/// Whether `byte` is an identifier character: an ASCII letter, digit or `_`.
const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The offset just after the identifier starting at `start`: identifier
/// characters, and `-` where directly followed by one. Shared by the lexer
/// and the scan, so they always agree.
fn identifier_end(bytes: &[u8], start: usize) -> usize {
    let mut pos = start;
    while let Some(&byte) = bytes.get(pos) {
        if is_identifier_byte(byte) {
            pos += 1;
        } else if byte == b'-'
            && bytes
                .get(pos + 1)
                .is_some_and(|&next| is_identifier_byte(next))
        {
            // Consume the `-` and the character after it together.
            pos += 2;
        } else {
            break;
        }
    }
    pos
}

/// The offset just after the number literal starting at `start`, as far as
/// the scan needs to know: digits and `_`, a `.` followed by a digit and
/// more of them, and an exponent. For every literal the lexer accepts, this
/// is where `Lexer::number` ends it; for the others the lexer reports an
/// error when it reaches the literal.
fn number_end(bytes: &[u8], start: usize) -> usize {
    let digits = |from: usize| {
        from + bytes
            .get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&byte| byte.is_ascii_digit() || byte == b'_')
            .count()
    };
    let mut pos = digits(start);
    if bytes.get(pos) == Some(&b'.') && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit) {
        pos = digits(pos + 1);
    }
    if matches!(bytes.get(pos), Some(b'e' | b'E')) {
        pos += 1;
        if matches!(bytes.get(pos), Some(b'+' | b'-')) {
            pos += 1;
        }
        pos = digits(pos);
    }
    pos
}

/// The offset just after the malformed number literal starting at `start`,
/// as the lexer skips it once it has reported it (see [Errors](#errors)):
/// every ASCII letter, digit and `_` glued to it, a `.` followed by a digit
/// or `_`, and a sign after an `e` or `E`, so that no part of it is lexed
/// again as a token of its own.
fn malformed_number_end(bytes: &[u8], start: usize) -> usize {
    let mut pos = start;
    while let Some(&byte) = bytes.get(pos) {
        let glued = match byte {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_' => true,
            b'.' => matches!(bytes.get(pos + 1), Some(b'0'..=b'9' | b'_')),
            b'+' | b'-' => pos > start && matches!(bytes.get(pos - 1), Some(b'e' | b'E')),
            _ => false,
        };
        if !glued {
            break;
        }
        pos += 1;
    }
    pos
}

/// The offset of the `\n` ending the line that `from` is on, or the end of
/// `bytes`.
fn end_of_line(bytes: &[u8], from: usize) -> usize {
    bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(bytes.len(), |offset| from + offset)
}

/// The attribute written at `from` (see [Attributes](#attributes)), if the
/// `#` there starts one rather than a comment: whether it is an inner one,
/// `#![`, where its name starts and ends, and the offset just after its
/// `]`. Shared by the lexer and the scan, so they always agree.
fn attribute_at(bytes: &[u8], from: usize) -> Option<(bool, (usize, usize), usize)> {
    if bytes.get(from) != Some(&b'#') {
        return None;
    }
    let inner = bytes.get(from + 1) == Some(&b'!');
    let open = from + 1 + usize::from(inner);
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let start = open + 1 + whitespace_len(bytes, open + 1);
    if !matches!(bytes.get(start), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')) {
        return None;
    }
    let end = identifier_end(bytes, start);
    let close = end + whitespace_len(bytes, end);
    (bytes.get(close) == Some(&b']')).then_some((inner, (start, end), close + 1))
}

/// The number of spaces and tabs starting at `from`.
fn whitespace_len(bytes: &[u8], from: usize) -> usize {
    bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|&&byte| matches!(byte, b' ' | b'\t'))
        .count()
}

/// The number of `"` starting at `from`.
fn quote_run(bytes: &[u8], from: usize) -> usize {
    bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|&&byte| byte == b'"')
        .count()
}

/// The length of the longest common prefix of `a` and `b`.
fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// The `\u` escape whose `\` is at `start`: the offset just after it, and
/// the character it denotes if it is valid. Shared by the scan and pass 2,
/// so they agree on its extent.
///
/// The escape is `\u`, then, if present, a `{`, a run of ASCII letters and
/// digits, and a `}`. It is valid if it has both braces and between them 1
/// to 6 hex digits denoting a Unicode scalar value.
fn unicode_escape(bytes: &[u8], start: usize) -> (usize, Option<char>) {
    let mut pos = start + 2;
    if bytes.get(pos) != Some(&b'{') {
        return (pos, None);
    }
    pos += 1;
    let digits_start = pos;
    while bytes.get(pos).is_some_and(u8::is_ascii_alphanumeric) {
        pos += 1;
    }
    let digits = bytes.get(digits_start..pos).unwrap_or_default();
    if bytes.get(pos) != Some(&b'}') {
        return (pos, None);
    }
    pos += 1;
    if digits.is_empty() || digits.len() > 6 {
        return (pos, None);
    }
    let value = digits.iter().try_fold(0_u32, |value, &digit| {
        char::from(digit)
            .to_digit(16)
            .map(|digit| value * 16 + digit)
    });
    (pos, value.and_then(char::from_u32))
}

fn leading_underscore(at: usize) -> Diagnostic {
    Diagnostic::error("leading `_` in number literal", span(at, at + 1))
}

/// The error for a literal with no closing delimiter, at its opening one.
fn unterminated_string(delimiter: &Delimiter) -> Diagnostic {
    let opening = span(
        delimiter.quotes_start,
        delimiter.quotes_start + delimiter.quotes,
    );
    let error = Diagnostic::error("unterminated string literal", opening);
    if delimiter.quotes >= 3 {
        error.with_note(format!(
            "this string closes with {} quotes",
            delimiter.quotes
        ))
    } else {
        error
    }
}

/// The error for an interpolation with no matching `}`, at its `{`.
fn unterminated_interpolation(open: usize) -> Diagnostic {
    Diagnostic::error("unterminated interpolation", span(open, open + 1))
        .with_note("write `{{` for a literal `{`")
}

/// The error for a literal nested more than [`MAX_DEPTH`] deep, at its
/// prefix and opening delimiter. No tree the parser accepts is that deep,
/// since each level of nesting is a level of the tree.
fn too_deep(start: usize, delimiter: &Delimiter) -> Diagnostic {
    Diagnostic::error(
        "expression nested too deeply",
        span(start, delimiter.content_start),
    )
    .with_note(format!("the limit is {MAX_DEPTH} levels"))
}

/// A violated invariant of the lexer, at `at`.
fn internal(message: &'static str, at: usize) -> Diagnostic {
    InternalError {
        message,
        span: span(at, at),
    }
    .into()
}

/// Builds a span from byte offsets within a [`SourceFile`], which always fit
/// in `u32`.
fn span(start: usize, end: usize) -> Span {
    Span::new(offset_to_u32(start), offset_to_u32(end))
}

#[cfg(test)]
mod tests {
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
        let (tokens, names, strings) =
            tokenize(&SourceFile::new("test.gcsl", text).unwrap()).unwrap();
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
}
