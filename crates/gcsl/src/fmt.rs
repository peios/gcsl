//! The formatter: a file's source laid out in GCSL's one style
//! ([`format()`]), its whitespace and indentation normalised and its line
//! breaks kept.
//!
//! It works on the file's tokens, its comments ([`lexer::lex_with_comments`])
//! and its text, with the syntax tree for what the tokens cannot tell. It
//! never joins or splits lines: every line break of the source is kept, but
//! blank lines, below, and a line's code is the same code. Between two
//! tokens on one line, what is written is decided by the rules below and
//! nothing else, so a well-kept file changes little.
//!
//! # Validity
//!
//! Only a file that lexes and parses with no error is formatted: one with a
//! lexical or a syntax error is left alone, and its errors returned, since
//! the parser's recovery leaves a tree that does not say what the source
//! means. And every result is checked before it is returned, an internal
//! error if any check fails, the file left alone: lexed again, its tokens
//! are the input's, kind and text, `Newline` tokens included, but for the
//! `,` and `;` the rules below add and remove, and for runs of `Newline`
//! tokens, which are compared as one, and those at the start and the end of
//! the file, which are not compared at all, since blank lines are removed
//! (the parser takes a run of separators as one); its comments are the
//! input's, text for text, trailing whitespace apart; and parsed again, its
//! tree, printed ([`Ast::pretty`]) with its spans left out, is the input's.
//!
//! # Lines
//!
//! - Each line is indented by 4 spaces per *level*. A line that starts
//!   with a closing bracket, `)`, `]` or `}`, has the level of the line its
//!   opening bracket is on. Any other line has one level more than the line
//!   the innermost bracket open at its start is on (none open: level 0),
//!   and one more again when it continues the line before: when the last
//!   token before it is one after which the parser skips newlines, an
//!   *infix* operator (`+`, `-`, `*`, `/`, `//`, `%`, a comparison, `and`,
//!   `or`, `..`, `..=`, `|`, `is`, `as`), `=`, `:=`, `:`, `->`, `->>`,
//!   `let`, `mut`, `...` or a `for` header's `;`, but the `>` that closes a
//!   type argument; or when it starts with an infix operator or a `.`, as a
//!   line inside brackets may. So brackets opened on one line indent the
//!   lines after it by one level however many they are, as in `f({` ...
//!   `})`; and a line in brackets that continues an element is a level more
//!   than the element's first line.
//! - Tabs, spaces and `\r`s that indent a line or end one are removed (the
//!   line then indented as above). No line ends in whitespace. A file's
//!   `\r\n` line breaks are `\n` ([`SourceFile::new`]).
//! - Line breaks are kept, but blank lines: a run of them is one, and there
//!   is none at the start of the file, after an opening bracket that ends
//!   its line (comments on its line included), nor before a closing bracket
//!   that starts one, nor at the end of the file. A file ends with exactly
//!   one `\n`, but an empty one, or one of whitespace alone, which is empty.
//! - A comment on a line of its own is indented as the line after it, but
//!   that one before a closing bracket is indented one level more, as what
//!   the bracket closes is, and one at the end of the file at level 0. A
//!   comment after code on its line is one space after it, but in a run of
//!   two or more such comments on consecutive lines that started at the same
//!   column: those stay at one column, the one they had, moved as the first
//!   line's indentation moved, or further right if the longest line of code
//!   in the run now reaches it, one space after that. A comment's text is
//!   kept, its trailing whitespace apart.
//!
//! # Inside a line
//!
//! Between two tokens on one line there is one space or none, as the first
//! of these rules that applies says:
//!
//! - None before a label (`for'outer`, `{'pick`), which the parser needs
//!   directly after its keyword or `{`.
//! - None after a negation's `-`, which the lexer needs glued to its
//!   operand, and after `...`.
//! - None either side of `.`, but between a number and `.`, where a space
//!   is kept as written, so `1 .5` does not become `1.5`.
//! - As written (none or one) after the last token of a def-type's path
//!   before `(`.
//! - None before `,` and `;`.
//! - None after `(` and `[`, and before `)` and `]`.
//! - `{}` for an empty pair of braces, and one space inside a pair with
//!   something between, on one line (`{ a = 1; b = 2 }`), but none inside
//!   a pair the source has nothing inside on either side, as `show` writes
//!   a record (`{a = 1; b = 2}`, `{x: Int}`).
//! - One space after `,` and `;`.
//! - As written either side of `..` and `..=` (`0..n`, `0 .. n`), and
//!   between `META` and a meta's marker, which the parser needs glued
//!   (`META->`, `META:`).
//! - None either side of the `<` of a type argument and before its `>`
//!   (`List<Int>`, `Future<Future<Int>>`), which the parser needs glued.
//! - Before `(`: none after a name, `)`, `]` or `fn`, a call's or a
//!   function's parameters; as written after `}`, a literal or the `>` of a
//!   type argument; one after anything else (`if (a)`, `= (b)`).
//! - Before `[`: none after a name, `)`, `]` or a string, an index or a
//!   list type's `[]`; as written after `}`, a literal other than a string,
//!   or a type argument's `>`; one after anything else, a list literal's.
//! - Before `:`: none (`name: Type`, `fn(x: Int): Int`), but as written
//!   before an ascription's (`x : Int`). One after it.
//! - One space anywhere else: either side of `=`, `:=`, `->`, `->>`, `|`,
//!   every other binary operator, `and`, `or`, `in`, `is` and `as`; before
//!   a negation's `-`; between two names or keywords (`not x`, `await x`);
//!   before a block's `{`; after `}`; around a string literal.
//!
//! What the syntax tree says is kept as written, untouched: inside a
//! def-type's path (`org.gnu.gmp-c++`, `go-stage0-1.24`), whose glued
//! tokens make one name; inside an import's module path
//! (`pkgs/net.busybox.busybox`), whose glued tokens make its segments; and
//! inside the position of an index segment of a
//! `:=` path (`xs[i+1] := v`), whose text the tree keeps, and its printer
//! shows. So is every string literal's text, its lines and
//! indentation, a multi-line or raw one's included; and in an
//! interpolation, the spacing just inside its `{` and `}`, and every gap
//! inside it that holds a line break or a comment, since a multi-line
//! literal's indentation is measured over the lines of its interpolations
//! too. Between other tokens of an interpolation the rules above apply.
//!
//! # Commas and semicolons
//!
//! - A list that ends on a line of its own gets a `,` after its last
//!   element if it lacks one: the arguments of a call, the parameters of a
//!   function, of a function type and of a def-type's definition, a list
//!   literal and a record type, whose closing bracket starts its line. Not
//!   an empty one, nor a parenthesised expression or type, an index, or a
//!   block.
//! - A `;` that ends a line of a block's body, or of the file's, after code
//!   on that line, is removed, since a line break separates items there: a
//!   run of them, `a;;`, wholly. Not one in a `for` header, which separates
//!   its parts, nor one in an interpolation.

use std::collections::HashSet;
use std::mem::{Discriminant, discriminant};

use crate::ast::{Ast, DefTypeRole, ExprKind, FieldTyKind, ItemKind, SegmentKind, Syntax, TypeExprKind};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::lexer::{self, Comments, Lexed, Token, TokenKind};
use crate::names::Names;
use crate::parser;
use crate::source::{SourceFile, Span};

/// The spaces one level of indentation is.
const INDENT: usize = 4;

/// Formats `source` (see the [module docs](self)): its text laid out in the
/// one style, which may be its text as it is.
///
/// # Errors
///
/// Every lexical and syntax error of `source`, if it has any, since only a
/// file that parses is formatted; or an internal error if the result fails
/// a check it should never fail, a bug, the file then to be left alone.
pub fn format(source: &SourceFile) -> Result<String, Vec<Diagnostic>> {
    let input = Input::read(source)?;
    let facts = Facts::of(&input);
    let shape = Shape::of(&input, &facts);
    let text = Printer::new(&input, &facts, &shape).print();
    verify(&input, &shape, &text)?;
    Ok(text)
}

/// A file that lexes and parses: its tokens, comments and tree.
struct Input<'a> {
    /// The file.
    source: &'a SourceFile,
    /// Its text.
    text: &'a str,
    /// Every token, `Newline` and `Eof` included.
    all: Vec<Token>,
    /// The tokens but `Newline` and `Eof`, which the formatter lays out:
    /// a newline is where the text has one.
    tokens: Vec<Token>,
    /// Where its comments are.
    comments: Comments,
    /// Its syntax tree.
    ast: Ast,
    /// The offset of every `\n` of the text, in order, for line numbers.
    breaks: Vec<usize>,
}

impl<'a> Input<'a> {
    /// Lexes and parses `source`, which must have no error.
    fn read(source: &'a SourceFile) -> Result<Self, Vec<Diagnostic>> {
        let mut names = Names::new();
        let (lexed, comments) = lexer::lex_with_comments(source, &mut names);
        let Lexed {
            tokens: all,
            strings,
            diagnostics: lexical,
        } = lexed;
        let (ast, diagnostics) = parser::parse_all(&all, names, strings, source, lexical);
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        if ast.syntax() != Syntax::Clean {
            return Err(vec![internal("a tree with no error is not clean")]);
        }
        let text = source.text();
        let tokens = all
            .iter()
            .filter(|token| !matches!(token.kind, TokenKind::Newline | TokenKind::Eof))
            .copied()
            .collect();
        let breaks = text
            .bytes()
            .enumerate()
            .filter_map(|(at, byte)| (byte == b'\n').then_some(at))
            .collect();
        Ok(Self {
            source,
            text,
            all,
            tokens,
            comments,
            ast,
            breaks,
        })
    }

    /// The kind of the token `at`, among [`Input::tokens`].
    fn kind(&self, at: usize) -> TokenKind {
        self.tokens
            .get(at)
            .map_or(TokenKind::Eof, |token| token.kind)
    }

    /// The source of the token `at`.
    fn slice(&self, at: usize) -> &'a str {
        self.tokens
            .get(at)
            .and_then(|token| self.text.get(range(token.span)))
            .unwrap_or_default()
    }

    /// Where the token `at` starts.
    fn start(&self, at: usize) -> usize {
        self.tokens
            .get(at)
            .map_or(self.text.len(), |token| token.span.start as usize)
    }

    /// Where the token `at` ends.
    fn end(&self, at: usize) -> usize {
        self.tokens
            .get(at)
            .map_or(0, |token| token.span.end as usize)
    }

    /// The index of the first token that starts at or after `offset`.
    fn first_from(&self, offset: u32) -> usize {
        self.tokens
            .partition_point(|token| token.span.start < offset)
    }

    /// The index of the token that starts at `offset`, if one does and is of
    /// the kind `kind` says.
    fn token_at(&self, offset: u32, kind: TokenKind) -> Option<usize> {
        let at = self.first_from(offset);
        let token = self.tokens.get(at)?;
        (token.span.start == offset && discriminant(&token.kind) == discriminant(&kind))
            .then_some(at)
    }

    /// The source line, counted from 0, that `offset` is on.
    fn line_of(&self, offset: usize) -> usize {
        self.breaks.partition_point(|&at| at < offset)
    }

    /// Where the source line that `offset` is on starts.
    fn line_start(&self, offset: usize) -> usize {
        self.text
            .get(..offset)
            .and_then(|before| before.rfind('\n'))
            .map_or(0, |at| at + 1)
    }
}

/// What the syntax tree says of the tokens that they cannot say themselves,
/// each by the index of the token it is about, among [`Input::tokens`].
#[derive(Default)]
struct Facts {
    /// The brackets, opening or closing, of the lists that take a trailing
    /// `,`: a call's arguments, a function's, a function type's and a
    /// def-type definition's parameters, a list literal, a record type.
    lists: HashSet<usize>,
    /// The `<` and `>` of every type argument.
    angles: HashSet<usize>,
    /// The `:` of every ascription.
    ascriptions: HashSet<usize>,
    /// Each `for` header, by offset: from the `for` to its body's `{`.
    headers: Vec<(usize, usize)>,
    /// What is kept as written, by offset: def-types' paths, module paths and the
    /// positions of the index segments of `:=` paths, in no order.
    verbatim: Vec<(usize, usize)>,
    /// The last token of each def-type's path.
    path_ends: HashSet<usize>,
    /// The `*` of every rest-field, which is no operator.
    rests: HashSet<usize>,
    /// The keyword of every method, `fn` or `def`, spaced from its
    /// receiver's `(` as no function expression's `fn` is.
    methods: HashSet<usize>,
}

impl Facts {
    /// What the tree of `input` says.
    fn of(input: &Input<'_>) -> Self {
        let mut facts = Self::default();
        let ast = &input.ast;
        for expr in ast.expr_arena() {
            match expr.kind {
                ExprKind::Call { callee, .. } => {
                    let at = input.first_from(ast.get(callee).span.end);
                    if input.kind(at) == TokenKind::LParen
                        && input.end(at) <= expr.span.end as usize
                    {
                        facts.lists.insert(at);
                    }
                }
                ExprKind::List(_) => {
                    facts
                        .lists
                        .extend(input.token_at(expr.span.start, TokenKind::LBracket));
                }
                ExprKind::Fn(id) => {
                    // The `)` of its parameters is the token before its
                    // marker, if it has one written: one with a return type
                    // (a meta's function and a def-type definition's have
                    // neither).
                    let decl = ast.fn_decl(id);
                    let marker = input.first_from(decl.marker.start);
                    if let Some(close) = marker.checked_sub(1)
                        && decl.ret.is_some()
                        && input.kind(close) == TokenKind::RParen
                    {
                        facts.lists.insert(close);
                    }
                }
                ExprKind::Ascribe { expr: inner, .. } => {
                    let at = input.first_from(ast.get(inner).span.end);
                    if input.kind(at) == TokenKind::Colon {
                        facts.ascriptions.insert(at);
                    }
                }
                ExprKind::For(id) => {
                    let body = ast.get(ast.for_decl(id).body).span.start;
                    facts
                        .headers
                        .push((expr.span.start as usize, body as usize));
                }
                _ => {}
            }
        }
        for id in ast.type_ids() {
            let ty = ast.type_expr(id);
            match ty.kind {
                TypeExprKind::Record(range) => {
                    facts
                        .lists
                        .extend(input.token_at(ty.span.start, TokenKind::LBrace));
                    for field in ast.field_tys(range) {
                        if field.kind == FieldTyKind::Rest {
                            facts
                                .rests
                                .extend(input.token_at(field.span.start, TokenKind::Star));
                        }
                    }
                }
                TypeExprKind::Fn { .. } => {
                    let at = input
                        .token_at(ty.span.start, TokenKind::Fn)
                        .map(|at| at + 1)
                        .filter(|&at| input.kind(at) == TokenKind::LParen);
                    facts.lists.extend(at);
                }
                // The `<` is the first token after the name, whatever space
                // is between them.
                TypeExprKind::Applied { span, .. } => {
                    let open = input.first_from(span.end);
                    if input.kind(open) == TokenKind::Lt {
                        facts.angles.insert(open);
                    }
                    let close = ty.span.end.saturating_sub(1);
                    facts.angles.extend(input.token_at(close, TokenKind::Gt));
                }
                TypeExprKind::PathApplied { path, .. } => {
                    let open = input.first_from(crate::ast::path_span(ast.path(path)).end);
                    if input.kind(open) == TokenKind::Lt {
                        facts.angles.insert(open);
                    }
                    let close = ty.span.end.saturating_sub(1);
                    facts.angles.extend(input.token_at(close, TokenKind::Gt));
                }
                _ => {}
            }
        }
        declaration_facts(input, &mut facts);
        for (_, used) in ast.def_types() {
            let path = crate::ast::path_span(ast.path(used.path));
            facts
                .verbatim
                .push((path.start as usize, path.end as usize));
            facts
                .path_ends
                .extend(input.first_from(path.end).checked_sub(1));
            if used.role == DefTypeRole::Definition {
                facts
                    .lists
                    .extend(input.token_at(used.span.start, TokenKind::LParen));
            }
        }
        facts.items_kept(ast);
        facts
    }

    /// Records what items of `ast` keep as written: an import's module path,
    /// whose glued tokens make its segments, and the position of each index
    /// segment of a `:=` path.
    fn items_kept(&mut self, ast: &Ast) {
        for item in ast.item_arena() {
            match item.kind {
                ItemKind::Import(id) => {
                    let path = crate::ast::path_span(ast.path(ast.import_decl(id).path));
                    self.verbatim.push((path.start as usize, path.end as usize));
                }
                ItemKind::Rebind { path, .. } => {
                    for segment in ast.path(path) {
                        if matches!(segment.kind, SegmentKind::Index { .. }) {
                            let span = segment.span;
                            self.verbatim.push((span.start as usize, span.end as usize));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Whether the gap from `start` to `end` is inside what is kept as
    /// written, not at its edge.
    fn is_verbatim(&self, start: usize, end: usize) -> bool {
        self.verbatim
            .iter()
            .any(|&(from, to)| from < start && end < to)
    }

    /// Whether `offset` is inside a `for` header.
    fn in_header(&self, offset: usize) -> bool {
        self.headers
            .iter()
            .any(|&(from, to)| from <= offset && offset < to)
    }
}

/// The shape of the token stream: brackets, lines, interpolations, and
/// the `,` and `;` the rules add and remove.
struct Shape {
    /// For each token, its matching bracket, if it is one.
    partner: Vec<Option<usize>>,
    /// For each token, whether a line break comes between it and the token
    /// before it.
    broken: Vec<bool>,
    /// For each token, how many interpolated literals it is inside.
    interpolated: Vec<usize>,
    /// The tokens after which a `,` is added.
    commas: HashSet<usize>,
    /// The `;` tokens removed.
    dropped: HashSet<usize>,
}

impl Shape {
    /// The shape of `input`'s tokens, with what `facts` say.
    fn of(input: &Input<'_>, facts: &Facts) -> Self {
        let count = input.tokens.len();
        let mut partner = vec![None; count];
        let mut enclosing = vec![None; count];
        let mut interpolated = vec![0; count];
        let mut broken = vec![false; count];
        let mut open: Vec<usize> = Vec::new();
        let mut depth = 0_usize;
        for at in 0..count {
            let kind = input.kind(at);
            if matches!(kind, TokenKind::StrEnd(_)) {
                depth = depth.saturating_sub(1);
            }
            if is_closer(kind)
                && let Some(opener) = open.pop()
            {
                partner[opener] = Some(at);
                partner[at] = Some(opener);
            }
            enclosing[at] = open.last().copied();
            interpolated[at] = depth;
            let gap = input
                .text
                .get(input.end(at.wrapping_sub(1))..input.start(at));
            broken[at] = at > 0 && gap.is_some_and(|gap| gap.contains('\n'));
            if is_opener(kind) {
                open.push(at);
            }
            if matches!(kind, TokenKind::StrStart(_)) {
                depth += 1;
            }
        }
        let mut shape = Self {
            partner,
            broken,
            interpolated,
            commas: HashSet::new(),
            dropped: HashSet::new(),
        };
        shape.edit(input, facts, &enclosing);
        shape
    }

    /// Decides the `,` added and the `;` removed.
    fn edit(&mut self, input: &Input<'_>, facts: &Facts, enclosing: &[Option<usize>]) {
        let count = input.tokens.len();
        for at in 0..count {
            let Some(close) = self.partner.get(at).copied().flatten() else {
                continue;
            };
            let listed = facts.lists.contains(&at) || facts.lists.contains(&close);
            let own_line = self.broken.get(close).copied().unwrap_or(false);
            if !is_opener(input.kind(at))
                || !listed
                || close <= at + 1
                || !own_line
                || self.interpolated[close] > 0
            {
                continue;
            }
            let last = close - 1;
            if input.kind(last) != TokenKind::Comma {
                self.commas.insert(last);
            }
        }
        for at in (0..count).rev() {
            if input.kind(at) != TokenKind::Semi || self.broken[at] || self.interpolated[at] > 0 {
                continue;
            }
            let in_body = enclosing[at].is_none_or(|opener| {
                input.kind(opener) == TokenKind::LBrace && !facts.lists.contains(&opener)
            });
            if !in_body || facts.in_header(input.start(at)) {
                continue;
            }
            let ends_line = at + 1 >= count
                || self.broken[at + 1]
                || (input.kind(at + 1) == TokenKind::Semi && self.dropped.contains(&(at + 1)));
            if ends_line {
                self.dropped.insert(at);
            }
        }
    }
}

/// A line of the output.
#[derive(Debug, Default)]
struct Line {
    /// Its text, the comment after its code apart. Several lines of the
    /// output when a token or a gap kept as written holds line breaks.
    text: String,
    /// The comment after its code, if it has one.
    comment: Option<Trailing>,
    /// How many columns its indentation moved, right positive.
    shift: isize,
}

/// A comment after code on its line.
#[derive(Debug)]
struct Trailing {
    /// Its text.
    text: String,
    /// The column its `#` was at, in characters.
    column: usize,
    /// The source line it was on.
    line: usize,
}

/// What a line of a gap is, between the line its first token ends and the
/// line its last starts.
enum Entry<'a> {
    /// A blank line.
    Blank,
    /// A comment on a line of its own: its text.
    Comment(&'a str),
}

/// Lays the tokens out, line by line.
struct Printer<'a> {
    /// The file.
    input: &'a Input<'a>,
    /// What its tree says.
    facts: &'a Facts,
    /// Its shape.
    shape: &'a Shape,
    /// The lines so far.
    lines: Vec<Line>,
    /// The brackets open, each with the level of the line it is on.
    open: Vec<(usize, usize)>,
    /// The level of the line being written.
    level: usize,
    /// The last token written.
    last: Option<usize>,
}

impl<'a> Printer<'a> {
    /// A printer of `input`.
    const fn new(input: &'a Input<'a>, facts: &'a Facts, shape: &'a Shape) -> Self {
        Self {
            input,
            facts,
            shape,
            lines: Vec::new(),
            open: Vec::new(),
            level: 0,
            last: None,
        }
    }

    /// The text, laid out.
    fn print(mut self) -> String {
        let count = self.input.tokens.len();
        for at in 0..count {
            if self.shape.dropped.contains(&at) {
                continue;
            }
            let from = if at == 0 { 0 } else { self.input.end(at - 1) };
            self.gap(Some(at), from, self.input.start(at));
            self.token(at);
        }
        let from = if count == 0 {
            0
        } else {
            self.input.end(count - 1)
        };
        self.gap(None, from, self.input.text.len());
        self.finish()
    }

    /// Writes the token `at`, and the `,` after it if one is added.
    fn token(&mut self, at: usize) {
        let kind = self.input.kind(at);
        if is_closer(kind) {
            self.open.pop();
        }
        let text = self.input.slice(at);
        if let Some(line) = self.lines.last_mut() {
            line.text.push_str(text);
            if self.shape.commas.contains(&at) {
                line.text.push(',');
            }
        }
        if is_opener(kind) {
            self.open.push((at, self.level));
        }
        self.last = Some(at);
    }

    /// Writes the gap from `start` to `end`, before the token `next`, or at
    /// the end of the file.
    fn gap(&mut self, next: Option<usize>, start: usize, end: usize) {
        let text = self.input.text.get(start..end).unwrap_or_default();
        let prev = self.last;
        if let (Some(prev), Some(next)) = (prev, next) {
            let interpolated = self.shape.interpolated[next] > 0;
            let touching = matches!(
                self.input.kind(prev),
                TokenKind::StrStart(_) | TokenKind::StrMid(_)
            ) || matches!(
                self.input.kind(next),
                TokenKind::StrMid(_) | TokenKind::StrEnd(_)
            );
            let broken = text.contains('\n') || text.contains('#');
            let kept = self.facts.is_verbatim(start, end) && !broken;
            if touching || kept || (interpolated && broken) {
                self.push(text);
                return;
            }
            if !broken {
                let space = self.space(prev, next);
                self.push(space);
                return;
            }
        }
        self.lines_of(prev, next, start, text);
    }

    /// Appends `text` to the line being written.
    fn push(&mut self, text: &str) {
        if let Some(line) = self.lines.last_mut() {
            line.text.push_str(text);
        }
    }

    /// Writes a gap `text`, at `start`, that ends a line or starts one: the
    /// comment ending the line of `prev`, the lines between, and the start
    /// of the line of `next`.
    fn lines_of(&mut self, prev: Option<usize>, next: Option<usize>, start: usize, text: &str) {
        let mut entries = Vec::new();
        let mut at = start;
        let parts: Vec<&str> = text.split('\n').collect();
        let last = parts.len() - 1;
        for (index, part) in parts.iter().enumerate() {
            let trimmed = part.trim();
            if index == 0 && prev.is_some() {
                if !trimmed.is_empty() {
                    let hash = at + (part.len() - part.trim_start().len());
                    self.trail(hash, trimmed);
                }
            } else if !(index == last && next.is_some()) {
                entries.push(if trimmed.is_empty() {
                    Entry::Blank
                } else {
                    Entry::Comment(trimmed)
                });
            }
            at += part.len() + 1;
        }
        // At most one blank line in a row; none after an opening bracket or
        // at the start of the file, nor before a closing one or at its end.
        let after_opener = prev.is_none_or(|prev| is_opener(self.input.kind(prev)));
        let before_closer = next.is_none_or(|next| is_closer(self.input.kind(next)));
        let mut kept: Vec<Entry<'_>> = Vec::new();
        for entry in entries {
            let skip = matches!(entry, Entry::Blank)
                && match kept.last() {
                    None => after_opener,
                    Some(last) => matches!(last, Entry::Blank),
                };
            if !skip {
                kept.push(entry);
            }
        }
        if before_closer && matches!(kept.last(), Some(Entry::Blank)) {
            kept.pop();
        }
        let level = self.comment_level(next);
        for entry in kept {
            self.lines.push(match entry {
                Entry::Blank => Line::default(),
                Entry::Comment(comment) => Line {
                    text: format!("{}{comment}", " ".repeat(level * INDENT)),
                    ..Line::default()
                },
            });
        }
        if let Some(next) = next {
            self.start_line(next);
        }
    }

    /// Records the comment `text` at `hash`, after the code of the line
    /// being written.
    fn trail(&mut self, hash: usize, text: &str) {
        let line_start = self.input.line_start(hash);
        let column = self
            .input
            .text
            .get(line_start..hash)
            .map_or(0, |before| before.chars().count());
        let line = self.input.line_of(hash);
        if let Some(current) = self.lines.last_mut() {
            current.comment = Some(Trailing {
                text: text.to_owned(),
                column,
                line,
            });
        }
    }

    /// Starts the line of the token `next`, indented.
    fn start_line(&mut self, next: usize) {
        let level = self.line_level(next);
        self.level = level;
        let start = self.input.start(next);
        let line_start = self.input.line_start(start);
        let was = self
            .input
            .text
            .get(line_start..start)
            .map_or(0, |before| before.chars().count());
        let width = level * INDENT;
        self.lines.push(Line {
            text: " ".repeat(width),
            comment: None,
            shift: signed(width) - signed(was),
        });
    }

    /// The level of the line starting with the token `next`.
    fn line_level(&self, next: usize) -> usize {
        let kind = self.input.kind(next);
        if is_closer(kind) {
            return self.open.last().map_or(0, |&(_, level)| level);
        }
        let leads = (is_infix(kind) || kind == TokenKind::Dot)
            && !self.facts.angles.contains(&next)
            && !self.facts.rests.contains(&next);
        self.content_level() + usize::from(leads || self.continues())
    }

    /// The level of a comment on a line of its own before the token `next`,
    /// or at the end of the file.
    fn comment_level(&self, next: Option<usize>) -> usize {
        match next {
            Some(next) if !is_closer(self.input.kind(next)) => self.line_level(next),
            _ => self.content_level(),
        }
    }

    /// The level of what the innermost open bracket holds.
    fn content_level(&self) -> usize {
        self.open.last().map_or(0, |&(_, level)| level + 1)
    }

    /// Whether the last token written is one after which the parser skips
    /// a newline, so that the next line continues its line.
    fn continues(&self) -> bool {
        let Some(last) = self.last else {
            return false;
        };
        let kind = self.input.kind(last);
        if kind == TokenKind::Gt && self.facts.angles.contains(&last) {
            return false;
        }
        if kind == TokenKind::Semi {
            return self.facts.in_header(self.input.start(last));
        }
        is_infix(kind)
            || matches!(
                kind,
                TokenKind::Eq
                    | TokenKind::ColonEq
                    | TokenKind::Colon
                    | TokenKind::Arrow
                    | TokenKind::MemoArrow
                    | TokenKind::Let
                    | TokenKind::Mut
                    | TokenKind::DotDotDot
            )
    }

    /// What goes between the tokens `prev` and `next` on one line: a space
    /// or nothing (see [Inside a line](self#inside-a-line)).
    fn space(&self, prev: usize, next: usize) -> &'static str {
        use TokenKind as K;
        let input = self.input;
        let (p, n) = (input.kind(prev), input.kind(next));
        let written = if input.end(prev) == input.start(next) {
            ""
        } else {
            " "
        };
        let angle = |at: usize| self.facts.angles.contains(&at);
        // Labels, negation and spreads, which the language needs glued.
        if matches!(n, K::Label(_)) {
            let owner = matches!(
                p,
                K::For | K::Do | K::Break | K::Continue | K::Return | K::LBrace
            );
            return if owner { "" } else { written };
        }
        if matches!(p, K::Neg | K::DotDotDot) {
            return "";
        }
        // `.`, `,`, `;` and the brackets.
        let number_dot = matches!(
            (p, n),
            (K::Int(_) | K::Float(_), K::Dot) | (K::Dot, K::Int(_) | K::Float(_))
        );
        if number_dot || (self.facts.path_ends.contains(&prev) && n == K::LParen) {
            return written;
        }
        if matches!(p, K::Dot | K::LParen | K::LBracket | K::ColonColon)
            || matches!(
                n,
                K::Dot | K::Comma | K::Semi | K::RParen | K::RBracket | K::ColonColon
            )
        {
            return "";
        }
        match (p, n) {
            (K::LBrace, K::RBrace) => return "",
            (K::LBrace, _) => return self.brace_inside(Some(prev), self.shape.partner[prev]),
            (_, K::RBrace) => return self.brace_inside(self.shape.partner[next], Some(next)),
            _ => {}
        }
        if matches!(p, K::Comma | K::Semi) {
            return " ";
        }
        // As written: around a range's operator, and a meta's marker.
        let range = matches!(p, K::DotDot | K::DotDotEq) || matches!(n, K::DotDot | K::DotDotEq);
        let meta = matches!(n, K::Colon | K::Arrow | K::MemoArrow)
            && matches!(p, K::Ident(_))
            && input.slice(prev) == "META";
        if range || meta {
            return written;
        }
        // Type arguments.
        if (p == K::Lt && angle(prev)) || (matches!(n, K::Lt | K::Gt) && angle(next)) {
            return "";
        }
        // A call's, a function's, an index's `(` or `[`, or another.
        if matches!(n, K::LParen | K::LBracket) {
            let glued = if n == K::LParen {
                matches!(
                    p,
                    K::Ident(_)
                        | K::RParen
                        | K::RBracket
                        | K::AttributeArgs(_)
                        | K::InnerAttributeArgs(_)
                ) || (p == K::Fn && !self.facts.methods.contains(&prev))
            } else {
                matches!(
                    p,
                    K::Ident(_) | K::RParen | K::RBracket | K::Str(_) | K::StrEnd(_)
                )
            };
            let unsure = matches!(
                p,
                K::Int(_) | K::Float(_) | K::True | K::False | K::Str(_) | K::StrEnd(_) | K::RBrace
            ) || (p == K::Gt && angle(prev));
            return if glued {
                ""
            } else if unsure {
                written
            } else {
                " "
            };
        }
        if n == K::Colon {
            return if self.facts.ascriptions.contains(&next) {
                written
            } else {
                ""
            };
        }
        " "
    }

    /// What goes just inside the braces `open` and `close`, which are on one
    /// line with something between them: nothing if the source has nothing
    /// there on either side, as `show` writes a record (`{a = 1; b = 2}`),
    /// else one space.
    fn brace_inside(&self, open: Option<usize>, close: Option<usize>) -> &'static str {
        let input = self.input;
        let glued = |before: usize| input.end(before) == input.start(before + 1);
        match (open, close) {
            (Some(open), Some(close)) if close > open + 1 && glued(open) && glued(close - 1) => "",
            _ => " ",
        }
    }

    /// The lines, joined, each comment after code placed.
    fn finish(mut self) -> String {
        self.align();
        let mut text = String::new();
        for line in &self.lines {
            text.push_str(&line.text);
            if let Some(comment) = &line.comment {
                text.push_str(&comment.text);
            }
            text.push('\n');
        }
        text
    }

    /// Pads each line's code before its trailing comment: one space, or, for
    /// a run aligned in the source, to the run's column.
    fn align(&mut self) {
        let mut at = 0;
        while at < self.lines.len() {
            let Some(first) = self.lines[at].comment.as_ref() else {
                at += 1;
                continue;
            };
            let (column, mut line) = (first.column, first.line);
            let mut end = at + 1;
            while let Some(Some(comment)) = self.lines.get(end).map(|l| l.comment.as_ref()) {
                if comment.line != line + 1 || comment.column != column {
                    break;
                }
                line += 1;
                end += 1;
            }
            let widest = self.lines[at..end]
                .iter()
                .map(|line| width(&line.text))
                .max()
                .unwrap_or(0);
            let target = if end - at >= 2 {
                let shift = if self.lines[at].text.contains('\n') {
                    0
                } else {
                    self.lines[at].shift
                };
                usize::try_from(signed(column) + shift)
                    .unwrap_or(0)
                    .max(widest + 1)
            } else {
                widest + 1
            };
            for line in &mut self.lines[at..end] {
                let pad = target.saturating_sub(width(&line.text));
                line.text.push_str(&" ".repeat(pad.max(1)));
            }
            at = end;
        }
    }
}

/// Checks the formatted `text` of `input` (see [Validity](self#validity)).
fn verify(input: &Input<'_>, shape: &Shape, text: &str) -> Result<(), Vec<Diagnostic>> {
    let output = SourceFile::new(input.source.name(), text)
        .map_err(|_| vec![internal("the formatted text is too long")])?;
    let mut names = Names::new();
    let (lexed, comments) = lexer::lex_with_comments(&output, &mut names);
    if !lexed.diagnostics.is_empty() {
        return Err(vec![internal("the formatted text does not lex")]);
    }
    let mut expected = Vec::new();
    let mut solid = 0;
    for token in &input.all {
        if matches!(token.kind, TokenKind::Newline | TokenKind::Eof) {
            expected.push((discriminant(&token.kind), "\n"));
            continue;
        }
        if !shape.dropped.contains(&solid) {
            expected.push((discriminant(&token.kind), input.slice(solid)));
        }
        if shape.commas.contains(&solid) {
            expected.push((discriminant(&TokenKind::Comma), ","));
        }
        solid += 1;
    }
    let found = lexed
        .tokens
        .iter()
        .map(|token| (discriminant(&token.kind), output.slice(token.span)));
    if collapsed(expected) != collapsed(found) {
        return Err(vec![internal("the formatted text has other tokens")]);
    }
    let comment_texts = |comments: &Comments, source: &SourceFile| -> Vec<String> {
        comments
            .spans()
            .iter()
            .map(|&span| source.slice(span).trim_end().to_owned())
            .collect()
    };
    if comment_texts(&comments, &output) != comment_texts(&input.comments, input.source) {
        return Err(vec![internal("the formatted text has other comments")]);
    }
    let (ast, diagnostics) =
        parser::parse_all(&lexed.tokens, names, lexed.strings, &output, Vec::new());
    if !diagnostics.is_empty() || ast.syntax() != Syntax::Clean {
        return Err(vec![internal("the formatted text does not parse")]);
    }
    let tree = |ast: &Ast| without_spans(&ast.pretty().to_string());
    if tree(&ast) != tree(&input.ast) {
        return Err(vec![internal("the formatted text has another syntax tree")]);
    }
    Ok(())
}

/// A token as compared: its kind, without its value, and its text.
type Compared<'a> = (Discriminant<TokenKind>, &'a str);

/// `tokens` with each run of `Newline`s one and those at the start and the
/// end left out, and `Eof` left out.
fn collapsed<'a>(tokens: impl IntoIterator<Item = Compared<'a>>) -> Vec<Compared<'a>> {
    let newline = discriminant(&TokenKind::Newline);
    let eof = discriminant(&TokenKind::Eof);
    let mut out: Vec<Compared<'a>> = Vec::new();
    for (kind, text) in tokens {
        if kind == eof || (kind == newline && out.last().is_none_or(|&(last, _)| last == newline)) {
            continue;
        }
        out.push((kind, if kind == newline { "\n" } else { text }));
    }
    if out.last().is_some_and(|&(last, _)| last == newline) {
        out.pop();
    }
    out
}

/// A printed tree with every span, `@START..END`, left out.
fn without_spans(printed: &str) -> String {
    let mut out = String::with_capacity(printed.len());
    let mut rest = printed;
    while let Some(at) = rest.find('@') {
        out.push_str(rest.get(..at).unwrap_or_default());
        let after = rest.get(at + 1..).unwrap_or_default();
        let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
        let first = digits(after);
        let span = first > 0
            && after.get(first..first + 2) == Some("..")
            && digits(after.get(first + 2..).unwrap_or_default()) > 0;
        if span {
            let tail = after.get(first + 2..).unwrap_or_default();
            rest = tail.get(digits(tail)..).unwrap_or_default();
        } else {
            out.push('@');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Whether `kind` is an operator between two operands: an arithmetic one,
/// a comparison, `and`, `or`, a range's, `|`, `is` or `as`. A line that
/// starts with one, inside brackets, continues the line before.
const fn is_infix(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::SlashSlash
            | TokenKind::Percent
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::EqEqEq
            | TokenKind::BangEqEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::DotDot
            | TokenKind::DotDotEq
            | TokenKind::Pipe
            | TokenKind::Is
            | TokenKind::As
    )
}

/// Whether `kind` opens a bracket: `(`, `[` or `{`.
const fn is_opener(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace
    )
}

/// Whether `kind` closes a bracket: `)`, `]` or `}`.
const fn is_closer(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace
    )
}

/// The width of the last line of `text`, in characters.
fn width(text: &str) -> usize {
    text.rsplit('\n').next().unwrap_or_default().chars().count()
}

/// `value` as a signed number of columns.
fn signed(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// The byte range of `span`.
const fn range(span: Span) -> std::ops::Range<usize> {
    span.start as usize..span.end as usize
}

/// An internal error of the formatter, `message`: a bug.
fn internal(message: &'static str) -> Diagnostic {
    InternalError {
        message,
        span: Span::new(0, 0),
    }
    .into()
}

/// Adds to `facts` what the declarations of `input`'s tree say: the
/// brackets of each generic function's or type's parameters, `<T, U>`,
/// which are bracketed as type arguments are, the token before its first
/// parameter and the first `>` after its last; and the keyword of each
/// method.
fn declaration_facts(input: &Input<'_>, facts: &mut Facts) {
    let ast = &input.ast;
    for method in ast.methods() {
        let start = ast.item(method.item).span.start;
        facts.methods.extend(input.token_at(start, TokenKind::Fn));
    }
    // An enum's bound and a variant's payload are bracketed as a type's
    // argument is: the `<` after the name, the `>` that ends the item.
    for &item in ast.items(ast.root()) {
        let (open, ty) = match ast.item(item).kind {
            // After `enum`, the name and `extensible`.
            ItemKind::Enum {
                bound: Some(ty), ..
            } => (input.first_from(ast.item(item).span.start) + 3, ty),
            ItemKind::Variant {
                name_span,
                payload: Some(ty),
                ..
            } => (input.first_from(name_span.end), ty),
            _ => continue,
        };
        let close = input.first_from(ast.type_expr(ty).span.end);
        if input.kind(open) == TokenKind::Lt && input.kind(close) == TokenKind::Gt {
            facts.angles.insert(open);
            facts.angles.insert(close);
        }
    }
    let angles = &mut facts.angles;
    for generics in ast.all_generics() {
        let (Some(first), Some(last)) = (generics.params.first(), generics.params.last()) else {
            continue;
        };
        if let Some(open) = input.first_from(first.1.start).checked_sub(1)
            && input.kind(open) == TokenKind::Lt
        {
            angles.insert(open);
        }
        let close = input.first_from(last.1.end);
        if input.kind(close) == TokenKind::Gt {
            angles.insert(close);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` formatted, which must succeed, and format to itself again.
    fn fmt(text: &str) -> String {
        let source = SourceFile::new("t.gcsl", text).unwrap();
        let formatted = format(&source).unwrap_or_else(|errors| panic!("{text:?}: {errors:?}"));
        let again = SourceFile::new("t.gcsl", formatted.as_str()).unwrap();
        assert_eq!(
            format(&again).unwrap(),
            formatted,
            "not idempotent: {text:?}"
        );
        formatted
    }

    /// Asserts that each of `cases`, an input and its formatted text, is
    /// formatted so.
    fn check(cases: &[(&str, &str)]) {
        for &(input, expected) in cases {
            assert_eq!(fmt(input), expected, "formatting {input:?}");
        }
    }

    #[test]
    fn a_method_s_keyword_is_spaced_from_its_receiver() {
        check(&[
            (
                "fn (x:Int)twice():Int { x*2 }\n",
                "fn (x: Int) twice(): Int { x * 2 }\n",
            ),
            (
                "def ( xs: List<T> ) first<T>( fallback: T ): T { fallback }\n",
                "def (xs: List<T>) first<T>(fallback: T): T { fallback }\n",
            ),
            (
                "f = fn (x: Int): Int { x }\n",
                "f = fn(x: Int): Int { x }\n",
            ),
        ]);
    }

    #[test]
    fn enums_glue_their_colons_and_bracket_their_types() {
        check(&[
            (
                "enum  Shape   extensible< {*: Int} >\n",
                "enum Shape extensible<{*: Int}>\n",
            ),
            (
                "enum Shape :: Sq< {side: Int} >\n",
                "enum Shape::Sq<{side: Int}>\n",
            ),
            (
                "x = Shape :: Sq({ side = 1 })\n",
                "x = Shape::Sq({ side = 1 })\n",
            ),
            (
                "let k: Variant< Shape,Int > = y\n",
                "let k: Variant<Shape, Int> = y\n",
            ),
            (
                "def  Tool :: C   gcc() {\n    1\n}\n",
                "def Tool::C gcc() {\n    1\n}\n",
            ),
            ("r = t.Tool :: C  gcc()\n", "r = t.Tool::C gcc()\n"),
        ]);
    }

    #[test]
    fn type_parameters_are_bracketed_as_type_arguments() {
        check(&[
            ("fn id< T >(x: T): T { x }\n", "fn id<T>(x: T): T { x }\n"),
            (
                "def pair<A,B>(a: A, b: B): A { a }\n",
                "def pair<A, B>(a: A, b: B): A { a }\n",
            ),
            ("x = 1<2\n", "x = 1 < 2\n"),
            (
                "type Pair< A,B > = {first: A, second: B}\n",
                "type Pair<A, B> = {first: A, second: B}\n",
            ),
            (
                "x: t.Pair< Int,String > = y\n",
                "x: t.Pair<Int, String> = y\n",
            ),
            (
                "x: t.Pair <Int, String> = y\n",
                "x: t.Pair<Int, String> = y\n",
            ),
            ("x: List <Int> = y\n", "x: List<Int> = y\n"),
        ]);
    }

    #[test]
    fn an_attribute_s_arguments_are_formatted_as_a_call_s() {
        check(&[
            (
                "#[retry( 3 )]\nfn f(): Int { 1 }\n",
                "#[retry(3)]\nfn f(): Int { 1 }\n",
            ),
            (
                "#[device(name =\"kvm\" , count=2)]   #[retry (1)]\nfn f(): Int { 1 }\n",
                "#[device(name = \"kvm\", count = 2)] #[retry(1)]\nfn f(): Int { 1 }\n",
            ),
        ]);
    }

    #[test]
    fn a_duration_or_a_size_is_kept_as_it_is_written() {
        check(&[
            ("a  =  1h30m\n", "a = 1h30m\n"),
            ("b = -1_000ms\n", "b = -1_000ms\n"),
            ("c = 2GiB+1.5KB\n", "c = 2GiB + 1.5KB\n"),
            (
                "fn f(t: Duration = 30s, s: Size = 0B): Duration { t }\n",
                "fn f(t: Duration = 30s, s: Size = 0B): Duration { t }\n",
            ),
        ]);
    }

    #[test]
    fn a_rest_field_s_star_is_no_operator() {
        check(&[
            // A line it starts is no continuation of the line before.
            (
                "type File = {\nto: String,\n  *: Int,\n}\n",
                "type File = {\n    to: String,\n    *: Int,\n}\n",
            ),
            (
                "type M = {\n*: String\n}\n",
                "type M = {\n    *: String,\n}\n",
            ),
            // On one line, glued to its `:` and spaced as a field is.
            (
                "type T = {a:Int,*:  String}\n",
                "type T = {a: Int, *: String}\n",
            ),
            // An operator still continues its line.
            ("x = (2\n* 3)\n", "x = (2\n        * 3)\n"),
        ]);
    }

    #[test]
    fn lines_are_indented_four_spaces_a_level() {
        check(&[
            ("x = {\na = 1\n}\n", "x = {\n    a = 1\n}\n"),
            (
                "x = {\n\ta = {\n\t\tb = 1\n\t}\n}\n",
                "x = {\n    a = {\n        b = 1\n    }\n}\n",
            ),
            // Brackets opened on one line indent the next by one level.
            ("f({\n      a = 1\n  })\n", "f({\n    a = 1\n})\n"),
            (
                "xs = [\n1,\n  [\n2,\n],\n]\n",
                "xs = [\n    1,\n    [\n        2,\n    ],\n]\n",
            ),
        ]);
    }

    #[test]
    fn a_line_that_continues_the_one_before_is_indented_one_more() {
        check(&[
            ("x =\n1\n", "x =\n    1\n"),
            ("x = 1 +\n2 +\n3\n", "x = 1 +\n    2 +\n    3\n"),
            // A line starting with an operator, inside brackets: one more
            // than what the bracket holds.
            ("y = f(1\n+ 2)\n", "y = f(1\n        + 2)\n"),
            (
                "z = f(\na,\nb\n+ c,\n)\n",
                "z = f(\n    a,\n    b\n        + c,\n)\n",
            ),
            // Not after the `>` of a type argument, nor an opening bracket.
            ("x: List<Int>\n= []\n", "x: List<Int>\n= []\n"),
            // A `for` header's parts after a `;`; its body is opened on the
            // last, so is a level more.
            (
                "for let mut i = 0;\ni < 3;\ni := i + 1 {\ni\n}\n",
                "for let mut i = 0;\n    i < 3;\n    i := i + 1 {\n        i\n    }\n",
            ),
        ]);
    }

    #[test]
    fn whitespace_ending_lines_and_the_file_is_removed() {
        check(&[
            ("x = 1   \ny = 2\t\n", "x = 1\ny = 2\n"),
            ("x = 1", "x = 1\n"),
            ("x = 1\n\n\n", "x = 1\n"),
            ("x = 1\r\ny = 2\r\n", "x = 1\ny = 2\n"),
            ("", ""),
            ("  \n\n\t\n", ""),
        ]);
    }

    #[test]
    fn blank_lines_are_one_at_most_and_none_inside_brackets_edges() {
        check(&[
            ("x = 1\n\n\n\ny = 2\n", "x = 1\n\ny = 2\n"),
            ("\n\nx = 1\n", "x = 1\n"),
            (
                "x = {\n\n  a = 1\n\n  b = 2\n\n}\n",
                "x = {\n    a = 1\n\n    b = 2\n}\n",
            ),
            ("f(\n\n1,\n\n)\n", "f(\n    1,\n)\n"),
            // A comment between: the blank line on its far side stays.
            (
                "x = {\n\n# a\n\na = 1\n# b\n\n}\n",
                "x = {\n    # a\n\n    a = 1\n    # b\n}\n",
            ),
        ]);
    }

    #[test]
    fn comments_are_kept_and_placed() {
        check(&[
            // On a line of its own: as the line after it.
            (
                "x = {\n# a\n        a = 1\n}\n",
                "x = {\n    # a\n    a = 1\n}\n",
            ),
            // Before a closing bracket: as what it closes.
            (
                "x = {\na = 1\n# end\n}\n",
                "x = {\n    a = 1\n    # end\n}\n",
            ),
            // At the end of the file, without a newline; its trailing
            // whitespace removed, its text kept.
            ("x = 1\n  # end   ", "x = 1\n# end\n"),
            (
                "#!/usr/bin/env gcsl\n#   spaced  text\nx = 1\n",
                "#!/usr/bin/env gcsl\n#   spaced  text\nx = 1\n",
            ),
            // After code: one space.
            ("x = 1      # one\n", "x = 1 # one\n"),
            ("f(1,    # one\n2)\n", "f(1, # one\n    2)\n"),
            // A run aligned in the source stays aligned, moved with its
            // indentation, or past its longest line.
            (
                "x = {\n  a = 1    # one\n  bb = 2   # two\n}\n",
                "x = {\n    a = 1    # one\n    bb = 2   # two\n}\n",
            ),
            (
                "a  =  1   # one\nbb = 2    # two\n",
                "a = 1     # one\nbb = 2    # two\n",
            ),
            (
                "a = 100000 # one\nb = 2      # two\n",
                "a = 100000 # one\nb = 2      # two\n",
            ),
            // Inside an interpolation: kept as written.
            ("x = \"{1 # one\n}\"\n", "x = \"{1 # one\n}\"\n"),
        ]);
    }

    #[test]
    fn spaces_inside_a_line() {
        check(&[
            // Binary operators, `=`, `:=`, arrows, `|`, `and`, `or`.
            ("x=1+2*3", "x = 1 + 2 * 3\n"),
            (
                "x = 1<2 and 2>=1 or not(true)",
                "x = 1 < 2 and 2 >= 1 or not (true)\n",
            ),
            ("let mut a = 1\na:=a  //  2", "let mut a = 1\na := a // 2\n"),
            (
                "type F = fn(a:Int)->Int|String",
                "type F = fn(a: Int) -> Int | String\n",
            ),
            ("type F = fn() ->>Int", "type F = fn() ->> Int\n"),
            // `,` and `;`.
            ("x = f(1 ,2,3)", "x = f(1, 2, 3)\n"),
            ("x = { a = 1 ;b = 2 }", "x = { a = 1; b = 2 }\n"),
            // Brackets.
            ("x = f( [ 1 ] )[ 0 ]", "x = f([1])[0]\n"),
            ("x = f ( 1 )", "x = f(1)\n"),
            ("x = { }", "x = {}\n"),
            ("x = {a = 1 }", "x = { a = 1 }\n"),
            // Braces written with nothing inside either side, as `show`
            // writes a record, stay so.
            ("x = {a = 1; b = 2}", "x = {a = 1; b = 2}\n"),
            ("t: {a: Int} = {a = 1}", "t: {a: Int} = {a = 1}\n"),
            ("x = if true{1}else{2}", "x = if true {1} else {2}\n"),
            // `:`: none before, but an ascription's as written.
            ("fn f(a :Int) :Int { a }", "fn f(a: Int): Int { a }\n"),
            ("x = (1 :Int)", "x = (1 : Int)\n"),
            ("x = (1: Int)", "x = (1: Int)\n"),
            // Type arguments, spreads, negation, labels.
            (
                "x: Future<Future<Int>> = lazy 1",
                "x: Future<Future<Int>> = lazy 1\n",
            ),
            ("xs = [ ...ys,-1 ]", "xs = [...ys, -1]\n"),
            ("x = 1 - -2", "x = 1 - -2\n"),
            (
                "x = for'a y in [1] { break'a }",
                "x = for'a y in [1] { break'a }\n",
            ),
            ("x = {'b\nreturn'b 1\n}", "x = {'b\n    return'b 1\n}\n"),
            // Ranges as written.
            (
                "x = for i in 0..3 { i }\ny = 0 ..= 3",
                "x = for i in 0..3 { i }\ny = 0 ..= 3\n",
            ),
            // A meta's marker, glued.
            (
                "type F = fn(): Int META->  {a: Int}",
                "type F = fn(): Int META-> {a: Int}\n",
            ),
            (
                "type F = fn(): Int META:{a: Int}",
                "type F = fn(): Int META: {a: Int}\n",
            ),
            // Field access, attributes.
            ("x = a . b", "x = a.b\n"),
            ("#[shadowable]   x = 1", "#[shadowable] x = 1\n"),
        ]);
    }

    #[test]
    fn what_the_tree_keeps_as_written_is_kept() {
        check(&[
            // A def-type's path, glued tokens and all.
            ("def build gmp-c++.6(v) {}", "def build gmp-c++.6(v) {}\n"),
            (
                "x = build  org.gnu.gcc-libstdc++-devel",
                "x = build org.gnu.gcc-libstdc++-devel\n",
            ),
            // An import's module path: its `/`s are no divisions.
            (
                "import pkgs/net.busybox.busybox  as  b\nimport x,y from lib/a-b/c",
                "import pkgs/net.busybox.busybox as b\nimport x, y from lib/a-b/c\n",
            ),
            // The position of an index segment of a `:=` path.
            (
                "let mut xs = [1]\nxs[0+0]:=2",
                "let mut xs = [1]\nxs[0+0] := 2\n",
            ),
            // String literals, multi-line and raw ones' lines included.
            (
                "y = {\n  s = \"\"\"\n  a\n    b\n  \"\"\"\n  r = r\"  x  \"\n}",
                "y = {\n    s = \"\"\"\n  a\n    b\n  \"\"\"\n    r = r\"  x  \"\n}\n",
            ),
            // An interpolation's surroundings, but not what is inside.
            ("x = \"a{ 1+2 }b{3}c\"", "x = \"a{ 1 + 2 }b{3}c\"\n"),
            // A line break inside an interpolation of a multi-line literal.
            (
                "y = {\n  s = \"\"\"\n  a{\n      1}\n  \"\"\"\n}",
                "y = {\n    s = \"\"\"\n  a{\n      1}\n  \"\"\"\n}\n",
            ),
        ]);
    }

    #[test]
    fn a_list_ending_on_a_line_of_its_own_gets_a_trailing_comma() {
        check(&[
            ("x = f(\n1,\n2\n)", "x = f(\n    1,\n    2,\n)\n"),
            ("xs = [\n1\n]", "xs = [\n    1,\n]\n"),
            (
                "fn f(\na: Int\n): Int { a }",
                "fn f(\n    a: Int,\n): Int { a }\n",
            ),
            ("type T = {\na: Int\n}", "type T = {\n    a: Int,\n}\n"),
            (
                "type F = fn(\na: Int\n): Int",
                "type F = fn(\n    a: Int,\n): Int\n",
            ),
            (
                "def package x.y(\nv\n) {}",
                "def package x.y(\n    v,\n) {}\n",
            ),
            // Before a comment after the last element.
            ("xs = [\n1 # one\n]", "xs = [\n    1, # one\n]\n"),
            // Already there, or the closing bracket after the last element,
            // or empty: nothing.
            ("xs = [\n1,\n]", "xs = [\n    1,\n]\n"),
            ("xs = [\n1]", "xs = [\n    1]\n"),
            ("xs = [\n]", "xs = [\n]\n"),
            // Not a parenthesised expression, an index or a block.
            ("x = (\n1\n)", "x = (\n    1\n)\n"),
            ("x = xs[\n0\n]", "x = xs[\n    0\n]\n"),
            ("x = {\na = 1\n}", "x = {\n    a = 1\n}\n"),
        ]);
    }

    #[test]
    fn what_looks_like_a_list_but_is_not_gets_no_comma() {
        // A meta's function has a marker nothing writes: the `)` before it
        // is no parameter list's.
        for text in [
            "x = (\n    1\n)\ndefmeta package y {}\n",
            "x = (\n    1\n)\ndefmeta package y = { a = 1 }\n",
            "x = (\n    1\n)\ny = lazy 1\n",
        ] {
            assert_eq!(fmt(text), text);
        }
    }

    #[test]
    fn types_in_parentheses_keep_their_type_arguments_glued() {
        for text in [
            "x: (List<Int>) = []\n",
            "type F = fn(): (fn(): Int) META: {a: Int}\n",
            "type G = (fn(): Int)[] | Future<fn(): Int>\n",
            "type H = Int | Future<Int>[]\n",
            "type I = List<(Future<Int>)>\n",
            "x = (y : (List<Int>))\n",
        ] {
            assert_eq!(fmt(text), text);
        }
    }

    #[test]
    fn a_semicolon_ending_a_line_of_a_body_is_removed() {
        check(&[
            ("x = 1;\ny = 2;;", "x = 1\ny = 2\n"),
            (
                "x = {\na = 1; # one\nb = 2;\n}",
                "x = {\n    a = 1 # one\n    b = 2\n}\n",
            ),
            // Not one between items on a line, nor one in a `for` header.
            ("x = { a = 1; b = 2 }", "x = { a = 1; b = 2 }\n"),
            (
                "for let mut i = 0;\ni < 3;\ni := i + 1 { i }",
                "for let mut i = 0;\n    i < 3;\n    i := i + 1 { i }\n",
            ),
        ]);
    }

    #[test]
    fn a_file_with_errors_is_left_alone_with_them() {
        for text in ["x = (", "x = 1 @ 2", "x = \"open", "x = {"] {
            let source = SourceFile::new("t.gcsl", text).unwrap();
            let errors = format(&source).unwrap_err();
            assert!(!errors.is_empty());
            assert!(
                errors
                    .iter()
                    .all(|error| !error.message.starts_with("internal error")),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn a_formatted_file_is_left_as_it_is() {
        let text = "# A file.\nimport _lib/files\n\ntype T = {\n    a: Int,\n    b: List<String> = [],\n}\n\ndef f(t: T) -> {} {\n    let xs = for i in 0..t.a if i > 1 { i * 2 }\n    g(\n        a = xs, # the list\n        b = \"{t.b}\",\n    )\n}\n";
        assert_eq!(fmt(text), text);
    }

    #[test]
    fn spans_are_left_out_of_a_printed_tree() {
        assert_eq!(
            without_spans("(let x @0..16\n  (int 1 @10..11) a@b @5.. @..6)"),
            "(let x \n  (int 1 ) a@b @5.. @..6)"
        );
    }
}
