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
//! its line (see the parser's [Types](crate::parser::types#types)), so a variable
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
//! and `....` is `...`, `.`. A number glued to the name of a unit, `30s`, `1h30m` or
//! `2GiB`, is a duration or a size literal ([`TokenKind::Duration`],
//! [`TokenKind::Size`]), its value converted exactly to nanoseconds or bytes
//! (see `Lexer::quantity`); glued to any other letters, a number is still a
//! number and the letters a name, so `7zip` is `7`, `zip`. A `,` is a
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
//! the whole file (see the parser's [Attributes](crate::parser::attributes#attributes)).
//! The name followed, after optional spaces or tabs, by `(` instead of `]`
//! is an attribute with arguments: one token through its name,
//! [`TokenKind::AttributeArgs`] or [`TokenKind::InnerAttributeArgs`], whose
//! `[` the lexer leaves open on its bracket stack, as a list's, so that
//! newlines in its arguments are not significant and the `]` after them,
//! an ordinary [`TokenKind::RBracket`], closes it; the arguments are
//! ordinary tokens. A `#[` or `#![` and a name that are not of the shape,
//! `#[x y]`, `#[x` or `#[a.b]`, are an error, "an attribute is named by one
//! name" for a dotted one, "expected `]` or `(` after the attribute's name"
//! otherwise, rather than a comment that would hide the code after them on
//! their line. Every other `#` starts a comment to the end of its line, as
//! it always has: so a shebang, `#!/usr/bin/env gcsl`, is a comment, and so
//! are `# [x]`, with a space, `#[` alone, `#[]` and `#[1]`. The scan of a
//! string literal's interpolations (see [String
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
//! recovery](crate::parser::recovery#errors-and-recovery)), so that a mistake is no
//! other token and nothing is reported of it twice:
//!
//! - An unexpected character, a `'` with no name after it, and an
//!   ambiguous `-` are an error token of their own, one character.
//! - A malformed number literal (a misplaced `_`, an exponent with no
//!   digits, a literal too large for its type, a duration's units out of
//!   order, a fraction that is no whole number of nanoseconds or bytes) is
//!   one error token with
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
use crate::value::{DURATION_UNITS, Quantity, SIZE_UNITS, Unit};

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
    /// A duration literal, already converted: a number of nanoseconds (see
    /// `Lexer::quantity`).
    Duration(i64),
    /// A size literal, already converted: a number of bytes (see
    /// `Lexer::quantity`).
    Size(i64),
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
    /// The start of an outer attribute with arguments, `#[NAME`, holding its
    /// name, before the `(` of its arguments (see [Attributes](#attributes)).
    /// Its `[` is open until the `]` after the arguments, an
    /// [`TokenKind::RBracket`].
    AttributeArgs(Name),
    /// The start of an inner attribute with arguments, `#![NAME`, as
    /// [`TokenKind::AttributeArgs`] is of an outer one.
    InnerAttributeArgs(Name),
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
    /// `::`, between an enum and its variant, `Toolchain::C`.
    ColonColon,
    /// `^`, before a context's name, `^arch`.
    Caret,
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
            TokenKind::ColonColon => Some("::"),
            TokenKind::Caret => Some("^"),
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
            | TokenKind::Duration(_)
            | TokenKind::Size(_)
            | TokenKind::Str(_)
            | TokenKind::StrStart(_)
            | TokenKind::StrMid(_)
            | TokenKind::StrEnd(_)
            | TokenKind::Ident(_)
            | TokenKind::Label(_)
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_)
            | TokenKind::AttributeArgs(_)
            | TokenKind::InnerAttributeArgs(_)
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

/// Whether `text` may be a segment of a module path (see
/// [Imports](crate::parser::imports#imports)): ASCII letters, digits, `_`, `-`, `+` and `.`,
/// starting with a letter, a digit or `_` and ending with one or `+`, as
/// `gmp-c++` does, with no `..`.
#[must_use]
pub fn is_module_segment(text: &str) -> bool {
    let word = |byte: &u8| byte.is_ascii_alphanumeric() || *byte == b'_';
    let bytes = text.as_bytes();
    bytes.first().is_some_and(word)
        && bytes.last().is_some_and(|byte| word(byte) || *byte == b'+')
        && !text.contains("..")
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'.'))
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
            b'^' => (TokenKind::Caret, 1),
            b'=' if self.peek_at(1) == Some(b'=') && self.peek_at(2) == Some(b'=') => {
                (TokenKind::EqEqEq, 3)
            }
            b'=' => with_eq(TokenKind::Eq, TokenKind::EqEq),
            b':' if self.peek_at(1) == Some(b':') => (TokenKind::ColonColon, 2),
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
                b'#' if attribute_at(self.bytes, self.pos).is_some()
                    || attribute_attempt(self.bytes, self.pos).is_some() =>
                {
                    break;
                }
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
    /// interned as an identifier's is; or, for one with arguments, an
    /// [`TokenKind::AttributeArgs`] or [`TokenKind::InnerAttributeArgs`]
    /// through its name, its `[` left open on the bracket stack, as a list's
    /// is, for the `]` after its arguments to close.
    fn attribute(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        let Some((inner, (from, to), args, end)) = attribute_at(self.bytes, start) else {
            return Err(self.malformed_attribute(start));
        };
        let name = self
            .names
            .intern(self.text.get(from..to).unwrap_or_default());
        self.pos = end;
        if args {
            self.brackets.push(Bracket::Square);
        }
        Ok(match (inner, args) {
            (false, false) => TokenKind::Attribute(name),
            (true, false) => TokenKind::InnerAttribute(name),
            (false, true) => TokenKind::AttributeArgs(name),
            (true, true) => TokenKind::InnerAttributeArgs(name),
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
    /// part of it, except as the sign of an exponent. Letters glued to a
    /// literal without an exponent that are a unit's name ([`unit_at`]) make
    /// it a duration or a size (`Lexer::quantity`); after an exponent they
    /// are "a number with an exponent cannot have a unit". Any other letters
    /// end the literal, as they always did, and start a name: so a module
    /// path's `pkgs/7zip` is still the number and the name it was, and the
    /// parser reports `30sec` where an expression is expected (see the
    /// parser's [Expressions](crate::parser::expressions#expressions)).
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

        if unit_at(self.bytes, self.pos) {
            return self.quantity(start);
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
            if unit_at(self.bytes, self.pos) {
                return Err(Diagnostic::error(
                    "a number with an exponent cannot have a unit",
                    span(self.pos, letters_end(self.bytes, self.pos)),
                ));
            }
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

    /// Lexes the duration or size literal starting at `start`, a digit: one
    /// or more *components*, each digits with `_` separators, an optional
    /// fraction, `.` and digits, and a unit glued to them, ASCII letters.
    ///
    /// ```text
    /// duration  = component+     (each with a unit of `DURATION_UNITS`)
    /// size      = component      (with a unit of `SIZE_UNITS`)
    /// component = digits ( "." digits )? unit
    /// ```
    ///
    /// A duration's components go from the largest unit to the smallest,
    /// each unit at most once, and only the last may have a fraction:
    /// `1h30m`, `1m0.5s`. Its value is the sum of its components' in
    /// nanoseconds, computed exactly (`exact_amount`), and must be a whole
    /// number of them; a size's, in bytes, likewise. Each mistake is its own
    /// error, at what is wrong: an unknown unit, "unknown unit `sec`", noted
    /// with the units there are; a unit out of order, "`h` cannot follow
    /// `m` in a duration"; a fraction before the last component; a size
    /// unit in a duration or a second component of a size; a fraction that
    /// is no whole number of the type's smallest unit; one with more digits
    /// than are kept; and a value too large for an `i64`, "duration literal
    /// out of range for Duration" or "size literal out of range for Size".
    fn quantity(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        self.pos = start;
        let mut total: i128 = 0;
        // The previous component's unit, and where it is written.
        let mut previous: Option<(&'static Unit, Span)> = None;
        // The previous component, if it has a fraction.
        let mut fractional: Option<Span> = None;
        loop {
            let component_start = self.pos;
            self.digits()?;
            let whole_end = self.pos;
            let mut fraction = None;
            if self.peek() == Some(b'.') {
                match self.peek_at(1) {
                    Some(b'0'..=b'9') => {
                        self.pos += 1;
                        let fraction_start = self.pos;
                        self.digits()?;
                        fraction = Some((fraction_start, self.pos));
                    }
                    Some(b'_') => return Err(leading_underscore(self.pos + 1)),
                    _ => {}
                }
            }
            let unit_start = self.pos;
            while self.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
                self.pos += 1;
            }
            let unit_span = span(unit_start, self.pos);
            let unit = unit_named(&self.text[unit_start..self.pos], previous, unit_span)?;
            if let Some(at) = fractional {
                return Err(Diagnostic::error(
                    "only the last component of a duration may have a fraction",
                    at,
                ));
            }
            let component = span(component_start, self.pos);
            let whole = &self.text[component_start..whole_end];
            let fraction = fraction.map_or("", |(from, to)| &self.text[from..to]);
            let amount = exact_amount((whole, fraction), unit, component)?;
            total = total
                .checked_add(amount)
                .ok_or_else(|| out_of_range(unit.quantity, span(start, self.pos)))?;
            if !fraction.is_empty() {
                fractional = Some(component);
            }
            previous = Some((unit, unit_span));
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                break;
            }
            if unit.quantity == Quantity::Size {
                let next = self.pos;
                let end = malformed_number_end(self.bytes, next);
                return Err(Diagnostic::error(
                    "a size literal has one unit",
                    span(next, end),
                ));
            }
        }
        let literal = span(start, self.pos);
        let value = i64::try_from(total).map_err(|_| out_of_range(Quantity::Duration, literal));
        match previous.map(|(unit, _)| unit.quantity) {
            Some(Quantity::Size) => Ok(TokenKind::Size(
                i64::try_from(total).map_err(|_| out_of_range(Quantity::Size, literal))?,
            )),
            _ => Ok(TokenKind::Duration(value?)),
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

    /// The error for the attribute begun at `start` but not of the shape
    /// (see [Attributes](#attributes)): "an attribute is named by one name"
    /// for a dotted one, else "expected `]` or `(` after the attribute's
    /// name", from its `#` through its name.
    fn malformed_attribute(&self, start: usize) -> Diagnostic {
        let Some(end) = attribute_attempt(self.bytes, start) else {
            return self.unexpected_character();
        };
        let at = span(start, end);
        if self.bytes.get(end) == Some(&b'.') {
            return Diagnostic::error("an attribute is named by one name", at).with_note(
                "import the attribute by its name, as in `import step from lib/attrs`, and write \
                 `#[step]`",
            );
        }
        Diagnostic::error("expected `]` or `(` after the attribute's name", at)
            .with_note("`#[` before a name starts an attribute; any other `#` starts a comment")
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
                    .map_or_else(|| end_of_line(self.bytes, pos), |(_, _, _, end)| end),
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
    } else if unit_at(bytes, pos) {
        // A unit, and the components after it, as in `1h30m`.
        pos = letters_end(bytes, pos);
        if bytes.get(pos).is_some_and(u8::is_ascii_digit) {
            return number_end(bytes, pos);
        }
    }
    pos
}

/// The offset just after the ASCII letters starting at `from`.
fn letters_end(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count()
}

/// Whether the ASCII letters at `from`, glued to a number, are the name of a
/// duration's or a size's unit, which makes the number a duration or a size
/// (see `Lexer::number`).
fn unit_at(bytes: &[u8], from: usize) -> bool {
    let letters = bytes
        .get(from..letters_end(bytes, from))
        .unwrap_or_default();
    !letters.is_empty()
        && DURATION_UNITS
            .iter()
            .chain(SIZE_UNITS.iter())
            .any(|unit| unit.name.as_bytes() == letters)
}

/// The note of an error of a duration's or a size's unit: the units there
/// are.
const DURATION_UNITS_NOTE: &str = "a duration's units are `d`, `h`, `m`, `s`, `ms`, `us` and `ns`";
pub(crate) const UNITS_NOTE: &str = "a duration's units are `d`, `h`, `m`, `s`, `ms`, `us` and `ns`, \
     and a size's `B`, `KB`, `MB`, `GB`, `TB`, `PB`, `KiB`, `MiB`, `GiB`, `TiB` and `PiB`";

/// The unit named `name`, written at `at`, of a component after one of the
/// unit `previous` names, if any (see `Lexer::quantity`): an error if
/// there is none of that name, if one comes after a duration's component
/// that is no duration's unit, or if it is no smaller than the previous.
fn unit_named(
    name: &str,
    previous: Option<(&'static Unit, Span)>,
    at: Span,
) -> Result<&'static Unit, Diagnostic> {
    let found = DURATION_UNITS
        .iter()
        .chain(SIZE_UNITS.iter())
        .find(|unit| unit.name == name);
    let Some((before, _)) = previous else {
        return found.ok_or_else(|| {
            Diagnostic::error(format!("unknown unit `{name}`"), at).with_note(UNITS_NOTE)
        });
    };
    match found {
        None => {
            Err(Diagnostic::error(format!("unknown unit `{name}`"), at)
                .with_note(DURATION_UNITS_NOTE))
        }
        Some(unit) if unit.quantity != Quantity::Duration => Err(Diagnostic::error(
            format!("`{name}` is not a duration's unit"),
            at,
        )
        .with_note(DURATION_UNITS_NOTE)),
        Some(unit) if unit.factor >= before.factor => Err(Diagnostic::error(
            format!("`{name}` cannot follow `{}` in a duration", before.name),
            at,
        )
        .with_note(
            "a duration's components go from the largest unit to the smallest, each unit \
             once, as in `1h30m`",
        )),
        Some(unit) => Ok(unit),
    }
}

/// The most digits of a fraction, once its trailing zeros are dropped, that
/// a duration or a size literal may have: as many as a `u128` always holds.
const MAX_FRACTION_DIGITS: usize = 38;

/// The amount, in the smallest unit of its measure, of the component
/// written at `at` of digits `whole` and fraction digits `fraction`, each
/// with `_` separators, and of unit `unit`: exact, computed with integer
/// arithmetic, the fraction's power of ten cancelled against the unit's
/// factors of 2 and 5. An error if it is no whole number of the smallest
/// unit, if the fraction has more than [`MAX_FRACTION_DIGITS`] digits, or if
/// it is beyond an `i128`, which the caller's range check then covers.
fn exact_amount(
    (whole, fraction): (&str, &str),
    unit: &Unit,
    at: Span,
) -> Result<i128, Diagnostic> {
    let out_of_range = || out_of_range(unit.quantity, at);
    let digits = |text: &str| -> Result<u128, Diagnostic> {
        text.bytes()
            .filter(|&byte| byte != b'_')
            .try_fold(0u128, |sum, byte| {
                sum.checked_mul(10)?.checked_add(u128::from(byte - b'0'))
            })
            .ok_or_else(out_of_range)
    };
    let whole = digits(whole)?
        .checked_mul(unit.factor)
        .ok_or_else(out_of_range)?;
    let fraction: String = fraction.chars().filter(|&c| c != '_').collect();
    let fraction = fraction.trim_end_matches('0');
    if fraction.len() > MAX_FRACTION_DIGITS {
        return Err(Diagnostic::error(
            format!(
                "the fraction of a {} literal has too many digits",
                unit.quantity.name()
            ),
            at,
        )
        .with_note(format!(
            "write at most {MAX_FRACTION_DIGITS} digits after the `.`"
        )));
    }
    // `remainder / 10^k` of the unit, whole only if 10^k divides
    // `remainder * factor`: its 2s and 5s from the factor first, the rest
    // from the remainder.
    let mut remainder = digits(fraction)?;
    let mut factor = unit.factor;
    let not_whole = || {
        Diagnostic::error(
            format!(
                "a {} literal must be a whole number of {}",
                unit.quantity.name(),
                unit.quantity.smallest()
            ),
            at,
        )
    };
    for prime in [2, 5] {
        let mut needed = fraction.len();
        while needed > 0 && factor.is_multiple_of(prime) {
            factor /= prime;
            needed -= 1;
        }
        while needed > 0 {
            if !remainder.is_multiple_of(prime) {
                return Err(not_whole());
            }
            remainder /= prime;
            needed -= 1;
        }
    }
    let part = remainder.checked_mul(factor).ok_or_else(out_of_range)?;
    let amount = whole.checked_add(part).ok_or_else(out_of_range)?;
    i128::try_from(amount).map_err(|_| out_of_range())
}

/// The error of a `quantity` literal, written at `at`, too large for an
/// `i64` of its smallest unit.
fn out_of_range(quantity: Quantity, at: Span) -> Diagnostic {
    let (message, largest) = match quantity {
        Quantity::Duration => (
            "duration literal out of range for Duration",
            "106751d23h47m16s854ms775us807ns",
        ),
        Quantity::Size => ("size literal out of range for Size", "9223372036854775807B"),
    };
    Diagnostic::error(message, at).with_note(format!(
        "the largest {} literal is {largest}",
        quantity.type_name()
    ))
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
/// `#![`, where its name starts and ends, whether it has arguments, a `(`
/// after its name, and the offset just after its `]`, or, for one with
/// arguments, just after its name, where its arguments' tokens start.
/// Shared by the lexer and the scan, so they always agree.
fn attribute_at(bytes: &[u8], from: usize) -> Option<(bool, (usize, usize), bool, usize)> {
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
    match bytes.get(close) {
        Some(b']') => Some((inner, (start, end), false, close + 1)),
        Some(b'(') => Some((inner, (start, end), true, end)),
        _ => None,
    }
}

/// Where the name of an attribute begun at `from` ends, if the `#` there
/// begins one, `#[` or `#![` and, after optional spaces or tabs, a name, but
/// is not of an attribute's shape, which [`attribute_at`] reads (see
/// [Attributes](#attributes)): `#[x y]`, `#[x` or `#[a.b]`, which the lexer
/// reports rather than skipping as a comment, so that no code after it on
/// its line is lost.
fn attribute_attempt(bytes: &[u8], from: usize) -> Option<usize> {
    if attribute_at(bytes, from).is_some() || bytes.get(from) != Some(&b'#') {
        return None;
    }
    let inner = bytes.get(from + 1) == Some(&b'!');
    let open = from + 1 + usize::from(inner);
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let start = open + 1 + whitespace_len(bytes, open + 1);
    matches!(bytes.get(start), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        .then(|| identifier_end(bytes, start))
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
mod tests;
