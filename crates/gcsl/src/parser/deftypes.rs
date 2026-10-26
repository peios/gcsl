//! Parsing def-types and definitions: `def` items, their paths, variants,
//! parameters and metas.
//!
//! # Def-types
//!
//! A def-type, a `def` marked `#[def_type]`, makes definitions, and a
//! reference calls one (see the checker's "Def-types"):
//!
//! ```text
//! DEFINITION = "def" DT VARIANT? PATH "(" (NAME ("," NAME)* ","?)? ")" BLOCK
//! REFERENCE  = DT VARIANT? PATH
//! META       = "defmeta" DT VARIANT? PATH (BLOCK | "=" EXPR)
//! DT         = IDENT ("." IDENT)*
//! VARIANT    = "::" IDENT
//! PATH       = SEGMENT ("." (SEGMENT | NUMBER))*
//! SEGMENT    = IDENT | IDENT "+" ...      (the run glued to it, below)
//! NUMBER     = INT ...  | FLOAT ...       (from a digit, below)
//! ```
//!
//! VARIANT names the variant a definition is made with, for a def-type
//! whose definitions are made with one (see the checker's "Variant
//! def-types"): `def Toolchain::C gcc()`, `Toolchain::C gcc`
//! (`Parser::parse_variant_def`, `Parser::parse_variant_ref`). In an
//! expression, a variant as a value with a name on its line after it is a
//! reference, as two names in a row are. A definition's variant with no
//! path on its line after it is "expected the definition's path after
//! `Toolchain::C`".
//!
//! A segment of PATH, and only of PATH, may have `+` in its name after its
//! first character, as real packages' names do, `org.gnu.gmp-c++`, whose
//! `+` the lexer, unchanged, makes tokens of their own
//! (`Parser::path_segment`): when a `+` directly follows a segment's
//! identifier, every token directly following the one before it, nothing
//! between them, whose text is made only of ASCII letters, digits, `_`, `+`
//! and `-` is glued to the identifier, a lone `-` only with a token glued
//! after it, and the segment is named by the source text of the run,
//! interned as one name. So a segment is a maximal run of ASCII letters,
//! digits, `_`, `-` and `+`, starting with a letter or `_` (or, after the
//! first segment, a digit, below), in which every `-` is directly followed
//! by a letter, digit, `_` or `+`, and which may end with `+` but not with
//! `-`, whatever its tokens are: `gmp-c++`, `a+b`,
//! `c++11`, and `gcc-libstdc++-devel`, lexed as `gcc-libstdc`, `+`, `+`, a
//! negation's `-`, and `devel`. What the lexer rejects stays rejected
//! (`c++11-x` is "`-` here is ambiguous", as `1-x` is, and `c++1_` a number
//! literal's error), and a token only part of whose text is of those
//! characters, a float with a fraction (`c++1.5`) or a raw string
//! (`a+r"x"`), ends the segment before it, where both are an error. `.`,
//! `(`, `[`, a space or any other token ends a segment. Anywhere else,
//! `a+b` is the addition it always was: in a field's path, a variable, a
//! `def`'s name (`def a+b()` is "expected `(` after `fn`, found `+`") and
//! DT. One program that parsed before means something else: a reference
//! directly followed by `+`, `package x.a+b` or `package x.a+1`, was the
//! addition of the reference and what followed, and is now one reference
//! (references are new); with a space before the `+`, it is the addition
//! still. A `+` directly after a `.` is the error it was, since a segment
//! starts with an identifier or a digit. The names are definitions' like
//! any other from here on, shown as written wherever a definition's path is
//! ([`Ast::show_definition_path`](super::Ast::show_definition_path)).
//!
//! A segment after the first may start with a digit, as versioned names
//! do, `org.golang.go-stage0-1.24` and `org.rust-lang.rust-1.83`
//! (`Parser::number_segments`): after a `.`, an integer literal starts a
//! segment, and so does a float, split at its `.` as if it were an integer,
//! `.` and an integer, each part with its own span (the lexer, unchanged,
//! makes `x.1.83.0` `x`, `.`, the float `1.83`, `.` and `0`: the segments
//! `x`, `1`, `83` and `0`). The segment the integer starts, or the float's
//! part after its `.`, is the run of it and every token glued to it as
//! above, `+` after it or not, named by the run's source text: `24rc1`
//! (`24` and `rc1`) is one segment, `1.83rc1` the two `1` and `83rc1`, `1_0`
//! is named `1_0`, a float with an exponent and no `.`, `1e5`, is one
//! segment, and `1.5e3` is `1` and `5e3`. A segment of digits alone is a
//! name like any other, `24`, which is not `024`. The first segment still
//! starts with an identifier, which is what makes a definition, a meta or
//! a reference (`def package 1.x(v)` is "expected `(` after `fn`, found
//! `1`", as it was), and nowhere else does a digit start a name: `x.0` in a
//! field's path, an expression, a `def`'s name or DT is the error it was.
//! What the lexer rejects stays rejected: an integer out of the range of
//! `Int`, `x.99999999999999999999` (a float's part after its `.` is no
//! integer, so `x.1.99999999999999999999` is `1` and
//! `99999999999999999999`), and a `-` directly after a number, `x.1-rc`,
//! "`-` here is ambiguous". A float in a run, not directly after a `.`, is
//! not split: `x.c++1.5` and `x.24+1.5` end the segment before it, where it
//! is an error, as `c++1.5` was. No program that parsed means anything
//! else: a number after a `.` that ends a reference was "expected a field
//! name after `.`", and after a definition's or a meta's path "expected `(`
//! after the definition's path" or "expected `{` or `=` after the meta's
//! path".
//!
//! A definition, [`ItemKind::DefTypeDef`], is recognised after `def` and a
//! name or a path (`Parser::parse_def`): an identifier that follows them on
//! their line starts its path. Its parameters are written by name alone,
//! each given a type expression nothing reads ([`TypeExprKind::Invalid`]),
//! its def-type's to give; then any of its own, each `NAME: TYPE = DEFAULT`,
//! since its def-type calls it with none: one with no default is "`x` is a
//! parameter of the definition's own, which `package` calls with none: give
//! it a default, or leave its type to `package`", and one by name alone
//! after one of its own "the parameters `package` gives a definition come
//! before its own, which have a type and a default". A definition with any
//! of its own has as its body a block whose result calls a function of
//! them, without their defaults, whose body is the body written, with their
//! defaults ([`DefTypeUse::body`]), so that the definition evaluates them
//! where its def-type calls it and the function, which the checker types
//! from the def-type, runs the body. A return marker after them "the result
//! type of a `package` definition comes from `package`", at the marker,
//! `package` being its DT as written. Its path is a `def`'s, identifiers
//! joined by `.`, but that a segment may have `+` in it, and one after the
//! first start with a digit, as above ("a `def`'s path is made of
//! identifiers" otherwise, at the segment), then `(`, the parameters and the
//! body, each "expected ..." otherwise. It is an item of a file's outermost body only: anywhere
//! else,
//! "a def-type definition is declared only at the top level of a file", at
//! its `def`. Its function expression spans its path through its body, the
//! expression that reads its DT coming before its body's in the arena.
//!
//! A reference, [`ExprKind::DefTypeRef`], is recognised where a postfix
//! operator may follow an expression (`Parser::parse_expr_inner`): after a
//! name, or a chain of field accesses directly on one, an identifier that
//! starts on the same line starts its path, so `package dev.peios.testpkg()`
//! is the call of the reference, and `-p x` the negation of one; it binds as
//! a field access does. "The same line" is the source's: the lexer emits no
//! newline inside parentheses, so `f(a\nb)` is the error it was, and two
//! names on two lines of a body are two items. Two names in a row never
//! parsed before, so every program that parsed parses as it did, but for
//! a reference directly followed by `+` (above); one that
//! did not may now fail later, at the checker, which notes that two names
//! in a row are read as a reference. Its path is a definition's, read
//! whole, however long, since what part of it names a definition the
//! checker finds (`package dev.peios.testpkg.versions` reads the meta of
//! `package dev.peios.testpkg`, see the checker's "Metas"); but that an
//! index ends it, as in `package x.versions[0]`, where a definition's path
//! is the error below. A quoted
//! field name of the form of a namespace's key (see [`crate::names`],
//! "Namespace keys") is "a field cannot be named `\"<p#1.2>\"`: names of
//! that form are def-types' namespaces", at the name, so no source can
//! write a key, nor a meta key, `\"<defmeta p#1.2@3>\"`. A definition also
//! holds the read of its meta, a name and a field access both standing for
//! nothing until the workspace names them, allocated with its path, before
//! its function in the arena and starting at its `DT`
//! (`Parser::unresolved_meta_read`).
//!
//! A definition's meta, [`ItemKind::DefMeta`] (see the checker's "Metas"),
//! is recognised where an item starts (`Parser::starts_defmeta`): the name
//! `defmeta`, then on its line a name or a dotted path, its DT, then on the
//! line of that an identifier, which starts its PATH, a definition's path.
//! `defmeta` is contextual, as `type` is: anywhere else it is a name, a
//! variable, a field or a def-type (`defmeta = 1`, `x = defmeta p`, `defmeta
//! p { ... }` the call of a reference with a trailing block), and two paths in
//! a row after it never parsed before, so every program that parsed parses
//! as it did. After PATH comes its body, a block, or `=` and any
//! expression, which is the result of a block of that one item, written
//! nowhere: the body of a function expression of no parameters, no return
//! type, and a marker that means nothing, spanning its path through its
//! body, as a definition's does (`Parser::parse_defmeta`). Anything else
//! after PATH is "expected `{` or `=` after the meta's path", and `META` and
//! a marker there "a meta is written only in a function type". It is an
//! item of a file's outermost body only: anywhere else, "a `defmeta` is
//! declared only at the top level of a file", at its `defmeta`.
//! `#[shadowable]` before one is "`#[shadowable]` does not apply to a
//! `defmeta`", at the attribute, since a definition that shadows another
//! brings its own meta; `#[def_type]` is the error it is before anything but
//! a `def`.

use crate::ast::{
    Arg, ArgKind, DefTypeRole, DefTypeUse, Effect, ExprId, ExprKind, FnDecl, FnId, Item, ItemId,
    ItemKind, Param, PathSegment, SegmentKind, TypeExpr, TypeExprKind, path_span,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{
    DEFMETA, DEF_PATH, DEF_STRING_NAME, LEAF_HEIGHT, MISPLACED_META, MUT_PARAM_NOTE,
    PLAIN_VARIABLE_NAME, ParseResult, Parser, UNRESOLVED_NAMESPACE, error_span,
};

impl Parser<'_> {
    /// Parses a public function item, `def PATH(...) ...`, starting at its
    /// `def` keyword, `keyword`. The path is an identifier, then `.` and an
    /// identifier as many times as they follow, directly followed by the
    /// parameters: a string where it starts is "a `def` is named by an
    /// identifier, not a string", a quoted or an index segment after it "a
    /// `def`'s path is made of identifiers", and anything else where a name
    /// is expected an error at it. A path of one name is the named function
    /// item's ([`Parser::parse_named_fn`]); a longer one is the dotted field
    /// of that path, marked as written as a `def` in the same way.
    pub(super) fn parse_def(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        let name_token = self.peek_at(1);
        let name = match name_token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) | TokenKind::StrStart(_) => {
                return Err(self.error(DEF_STRING_NAME, name_token.span));
            }
            _ => {
                let found = self.describe(name_token);
                return Err(self.report(Diagnostic::error(
                    format!("expected the function's name after `def`, found {found}"),
                    name_token.span,
                )));
            }
        };
        let first = PathSegment {
            kind: SegmentKind::Name(name),
            span: name_token.span,
        };
        // `def Toolchain::C gcc`: a definition made with a variant.
        if self.peek_at(2).kind == TokenKind::ColonColon {
            self.advance();
            self.advance();
            return self.parse_variant_def(keyword, &[first]);
        }
        // A name on its line after a name, `def package x`: the definition
        // of `x` made with the def-type `package`.
        if self.is_name_after(name_token.span, self.peek_at(2)) {
            self.advance();
            self.advance();
            return self.parse_def_type_def(keyword, (&[first], None));
        }
        if !matches!(self.peek_at(2).kind, TokenKind::Dot | TokenKind::LBracket) {
            return self.parse_named_fn(keyword, (name, name_token.span));
        }
        self.advance();
        self.advance();
        // Kept apart until the function is parsed: the items of its body
        // read paths of their own.
        let mut segments = vec![first];
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.advance();
                    let token = self.peek();
                    match token.kind {
                        TokenKind::Ident(name) => {
                            self.advance();
                            segments.push(PathSegment {
                                kind: SegmentKind::Name(name),
                                span: token.span,
                            });
                        }
                        TokenKind::Str(_) => return Err(self.error(DEF_PATH, token.span)),
                        TokenKind::StrStart(first) => {
                            let literal = self.parse_interpolation(first)?;
                            let span = self.span_of(literal);
                            return Err(self.error(DEF_PATH, span));
                        }
                        _ => {
                            let found = self.describe(token);
                            return Err(self.error(
                                format!("expected the function's name after `.`, found {found}"),
                                error_span(token),
                            ));
                        }
                    }
                }
                TokenKind::LBracket => {
                    let (_, span) = self.parse_position()?;
                    return Err(self.error(DEF_PATH, span));
                }
                TokenKind::ColonColon => return self.parse_variant_def(keyword, &segments),
                _ => break,
            }
        }
        if let Some(last) = segments.last()
            && self.is_name_after(last.span, self.peek())
        {
            return self.parse_def_type_def(keyword, (&segments, None));
        }
        let value = self.parse_fn(keyword)?;
        let base = self.path.len();
        self.path.extend(segments);
        let (path, shape) = self.finish_path(base);
        let kind = ItemKind::Field {
            path,
            ty: None,
            value,
            function: true,
        };
        self.finish_item(keyword, (kind, value), 0, shape)
    }

    /// Whether `token` is an identifier on the line of `before`, the span of
    /// the name or path before it: what makes the two a def-type and a path
    /// (see [Def-types](#def-types)). The lexer emits no newline inside
    /// parentheses, so the source between them says.
    pub(super) fn is_name_after(&self, before: Span, token: Token) -> bool {
        matches!(token.kind, TokenKind::Ident(_))
            && before.end <= token.span.start
            && !self
                .source
                .slice(Span::new(before.end, token.span.start))
                .contains('\n')
    }

    /// Allocates the expression that reads the path `segments`, a name and
    /// a field access for each segment after it, each spanning the path up
    /// to its segment, and returns the last: a def-type, `DT`, as it is
    /// written, or the read of a definition (see [Def-types](#def-types)).
    pub(super) fn path_expr(&mut self, segments: &[PathSegment]) -> ParseResult<ExprId> {
        let mut read: Option<ExprId> = None;
        for (index, segment) in segments.iter().enumerate() {
            let Some(name) = segment.name() else {
                continue;
            };
            let span = path_span(&segments[..=index]);
            read = Some(match read {
                None => self.alloc(ExprKind::Name(name), span, LEAF_HEIGHT)?,
                Some(expr) => {
                    let height = self.height_of(expr) + 1;
                    self.alloc(ExprKind::Field { expr, name }, span, height)?
                }
            });
        }
        read.ok_or_else(|| self.error("expected a name", Span::new(0, 0)))
    }

    /// Reads the path of a definition or a reference made with a def-type,
    /// `PATH`, starting at its first identifier, the current token:
    /// segments joined by `.`, each an identifier, or an identifier with the
    /// tokens glued to it that make a name with `+` in it
    /// ([`Parser::path_segment`]), or, after the first, one that starts with
    /// a digit, from an integer or a float, split at its `.`
    /// ([`Parser::number_segments`]). A quoted segment, or an index, is "a
    /// `def`'s path is made of identifiers", but that an index ends the path
    /// of a `reference`, to index what it reads. Returns its segments.
    pub(super) fn parse_def_type_path(&mut self, reference: bool) -> ParseResult<Vec<PathSegment>> {
        let mut segments = Vec::new();
        loop {
            let token = self.peek();
            match token.kind {
                TokenKind::Ident(name) => {
                    self.advance();
                    let segment = self.path_segment(token, name);
                    segments.push(segment);
                }
                // A segment after the first may start with a digit: an
                // integer, or a float split at its `.`.
                TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::Duration(_)
                | TokenKind::Size(_)
                    if !segments.is_empty() =>
                {
                    self.advance();
                    self.number_segments(token, &mut segments);
                }
                _ => {
                    let found = self.describe(token);
                    return Err(self.error(
                        format!("expected the definition's name after `.`, found {found}"),
                        error_span(token),
                    ));
                }
            }
            match (self.peek().kind, self.peek_at(1).kind) {
                (
                    TokenKind::Dot,
                    TokenKind::Ident(_)
                    | TokenKind::Int(_)
                    | TokenKind::Float(_)
                    | TokenKind::Duration(_)
                    | TokenKind::Size(_),
                ) => {
                    self.advance();
                }
                (TokenKind::Dot, TokenKind::Str(_) | TokenKind::StrStart(_)) => {
                    self.advance();
                    let token = self.peek();
                    let span = match token.kind {
                        TokenKind::StrStart(first) => {
                            let literal = self.parse_interpolation(first)?;
                            self.span_of(literal)
                        }
                        _ => token.span,
                    };
                    return Err(self.error(DEF_PATH, span));
                }
                // A reference's path ends before an index, which indexes
                // what the reference reads: a list of its definition's
                // meta, `package x.versions[0]` (see the checker's "Metas").
                (TokenKind::LBracket, _) if !reference => {
                    let (_, span) = self.parse_position()?;
                    return Err(self.error(DEF_PATH, span));
                }
                _ => return Ok(segments),
            }
        }
    }

    /// The segment of a definition's path that starts at the identifier
    /// `first`, of name `name`, just consumed (see [Def-types](#def-types)):
    /// the identifier alone, unless a `+` directly follows it, when the
    /// segment is the run of the tokens glued to it ([`Parser::glued_run`]),
    /// consumed, and its name the run's source text, interned, so that
    /// `gmp-c++` is one name. `gmp-c++`, `a+b`, `c++11` and
    /// `gcc-libstdc++-devel` (lexed as `gcc-libstdc`, `+`, `+`, `-`,
    /// `devel`) are each one segment; `a + 1`, with spaces, is not.
    pub(super) fn path_segment(&mut self, first: Token, name: Name) -> PathSegment {
        let (glued, end) = if self.peek().kind == TokenKind::Plus {
            self.glued_run(first.span.end)
        } else {
            (0, first.span.end)
        };
        if glued == 0 {
            return PathSegment {
                kind: SegmentKind::Name(name),
                span: first.span,
            };
        }
        self.pos += glued;
        self.written_segment(Span::new(first.span.start, end))
    }

    /// The segments of a definition's path that start at the number literal
    /// `first`, an integer or a float, just consumed, after the path's first
    /// segment (see [Def-types](#def-types)), pushed onto `segments`: a
    /// float with a fraction is split at its `.`, as if it were an integer,
    /// `.` and an integer, into the digits before it, one segment, and those
    /// after it, which start the next; anything else starts one whole
    /// (`24`, `1_0`, `1e5`). The segment it starts is the run of it and the
    /// tokens glued to it, consumed ([`Parser::glued_run`]), whatever
    /// follows, named by the run's source text, interned: `1.83` is `1` and
    /// `83`, `24rc1` (`24` and `rc1`) one segment, and `1.83rc1` `1` and
    /// `83rc1`.
    pub(super) fn number_segments(&mut self, first: Token, segments: &mut Vec<PathSegment>) {
        let mut start = first.span.start;
        let dot = self
            .source
            .slice(first.span)
            .find('.')
            .and_then(|at| u32::try_from(at).ok());
        if let Some(dot) = dot {
            segments.push(self.written_segment(Span::new(start, start + dot)));
            start += dot + 1;
        }
        let (glued, end) = self.glued_run(first.span.end);
        self.pos += glued;
        segments.push(self.written_segment(Span::new(start, end)));
    }

    /// How many tokens, from the current one on, are glued to a segment of a
    /// definition's path whose run so far ends at `end`, and where the run
    /// then ends (see [Def-types](#def-types)). A token is glued when it
    /// directly follows the one before, nothing between them, and its text
    /// is made only of ASCII letters, digits, `_`, `+` and `-`, a `-` that
    /// ends one being glued only with a token glued after it: so the run is
    /// a maximal one of those characters in which every `-` is directly
    /// followed by a letter, a digit, `_` or `+` (the lexer allowing no
    /// `--`), and that does not end with `-`.
    pub(super) fn glued_run(&self, end: u32) -> (usize, u32) {
        let (mut glued, mut glued_end) = (0, end);
        let (mut ahead, mut end) = (0, end);
        loop {
            let token = self.peek_at(ahead);
            let text = self.source.slice(token.span);
            if token.span.start != end
                || text.is_empty()
                || !text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
            {
                break;
            }
            ahead += 1;
            end = token.span.end;
            if !text.ends_with('-') {
                (glued, glued_end) = (ahead, end);
            }
        }
        (glued, glued_end)
    }

    /// The segment of a definition's path of span `span`, named by its
    /// source text, interned.
    pub(super) fn written_segment(&mut self, span: Span) -> PathSegment {
        PathSegment {
            kind: SegmentKind::Name(self.names.intern(self.source.slice(span))),
            span,
        }
    }

    /// Parses a definition made with a variant of a def-type, `def DT::V
    /// PATH(PARAMS) BODY`, at the `::` after its def-type, `dt`, read, its
    /// `def` `keyword` (see [Def-types](#def-types)): the variant, then the
    /// definition as [`Parser::parse_def_type_def`] parses it. Anything but
    /// a path on the variant's line after it is "expected the definition's
    /// path after `Toolchain::C`".
    pub(super) fn parse_variant_def(
        &mut self,
        keyword: Token,
        dt: &[PathSegment],
    ) -> ParseResult<(ItemId, u32)> {
        let variant = self.parse_def_type_variant()?;
        if let Some((name, span)) = variant
            && !self.is_name_after(span, self.peek())
        {
            let shown = format!("{}::{}", self.show_segments(dt), self.names.get(name));
            let token = self.peek();
            let found = self.describe(token);
            return Err(self.error(
                format!("expected the definition's path after `{shown}`, found {found}"),
                error_span(token),
            ));
        }
        self.parse_def_type_def(keyword, (dt, variant))
    }

    /// The variant written after a def-type, `::C` in `def Toolchain::C
    /// gcc()`, by its name and the name's span, if the current token is
    /// `::` (see [Def-types](#def-types)). Anything but a name after `::` is
    /// "expected the variant's name after `::`".
    pub(super) fn parse_def_type_variant(&mut self) -> ParseResult<Option<(Name, Span)>> {
        if self.peek().kind != TokenKind::ColonColon {
            return Ok(None);
        }
        self.advance();
        let token = self.peek();
        let TokenKind::Ident(name) = token.kind else {
            let found = self.describe(token);
            return Err(self.error(
                format!("expected the variant's name after `::`, found {found}"),
                error_span(token),
            ));
        };
        self.advance();
        Ok(Some((name, token.span)))
    }

    /// Parses a definition made with a def-type, `def DT PATH(PARAMS)
    /// BODY`, starting at its path, its `def`, `keyword`, and its def-type,
    /// `dt`, read (see [Def-types](#def-types)): the parameters by name
    /// alone, each a type the def-type gives it, no return type, and the
    /// body. A parameter's type written is "the parameter types of a
    /// `package` definition come from `package`", at its `:`, and a return
    /// marker "the result type of a `package` definition comes from
    /// `package`", at the marker.
    pub(super) fn parse_def_type_def(
        &mut self,
        keyword: Token,
        (dt, variant): (&[PathSegment], Option<(Name, Span)>),
    ) -> ParseResult<(ItemId, u32)> {
        let mut shown = self.show_segments(dt);
        if let Some((name, _)) = variant {
            shown = format!("{shown}::{}", self.names.get(name));
        }
        let dt_expr = self.path_expr(dt)?;
        let segments = self.parse_def_type_path(false)?;
        // The read of its meta, outside its function's subtree as its `DT`'s
        // is, which the workspace names once it knows its def-type.
        let meta_read = self.unresolved_meta_read(self.span_of(dt_expr), &segments)?;
        let open = self.peek();
        if open.kind != TokenKind::LParen {
            let found = self.describe(open);
            return Err(self.error(
                format!("expected `(` after the definition's path, found {found}"),
                error_span(open),
            ));
        }
        self.advance();
        let (params, own, close) = self.parse_definition_params(open, &shown)?;
        if self.meta_marker().is_some() {
            let span = Span::join(self.peek().span, self.peek_at(1).span);
            return Err(self.error(MISPLACED_META, span));
        }
        let marker = self.peek();
        if matches!(
            marker.kind,
            TokenKind::Colon | TokenKind::Arrow | TokenKind::MemoArrow
        ) {
            return Err(self.error(
                format!("the result type of a `{shown}` definition comes from `{shown}`"),
                marker.span,
            ));
        }
        let brace = self.peek();
        if brace.kind != TokenKind::LBrace {
            let found = self.describe(brace);
            return Err(self.error(
                format!("expected `{{` to start the definition's body, found {found}"),
                error_span(brace),
            ));
        }
        let body = self.parse_block()?;
        let (body, inner) = if own.is_empty() {
            (body, None)
        } else {
            let marker = self.span_of(dt_expr);
            let (block, inner) = self.definition_body(body, &own, marker)?;
            (block, Some(inner))
        };
        let height = self.height_of(body).max(LEAF_HEIGHT) + 1;
        // From its path, after its `DT`, whose expression comes before its
        // body's in the arena ([`Ast::subtree`]).
        let span = Span::join(path_span(&segments), self.span_of(body));
        let function = self.builder.push_fn(FnDecl {
            params,
            ret: None,
            // The def-type's to say: what is written here means nothing.
            effect: Effect::Pure,
            marker: self.span_of(dt_expr),
            body,
        });
        let value = self.alloc(ExprKind::Fn(function), span, height)?;
        let base = self.path.len();
        self.path.extend(segments);
        let (path, mut shape) = self.finish_path(base);
        // Its leaf is under its namespace's root, one segment more.
        shape.segments = shape.segments.saturating_add(1);
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Definition,
            dt: dt_expr,
            variant,
            path,
            read: None,
            key: None,
            meta: None,
            meta_read: Some(meta_read),
            body: inner,
            span: Span::join(open.span, close.span),
        });
        // The key of the context its body binds to itself, which lowering
        // finds by its text.
        self.names.intern(crate::lower::DEFINITION_CONTEXT);
        self.finish_item(
            keyword,
            (ItemKind::DefTypeDef { def, value }, value),
            0,
            shape,
        )
    }

    /// Parses the parameters of a definition made with the def-type
    /// `shown`, as it is written, after their `(`, `open`, through their
    /// `)`: names alone, separated by `,`, each with a type expression of
    /// [`TypeExprKind::Invalid`] that nothing reads, the def-type's to give,
    /// then the definition's own, each with its type and its default (see
    /// [Def-types](#def-types)). Returns the def-type's, the definition's
    /// own, and the `)`. A
    /// parameter that fails is skipped to its `,`, and the list goes on, to
    /// fail at its end (see [Errors and recovery](crate::parser::recovery#errors-and-recovery)).
    pub(super) fn parse_definition_params(
        &mut self,
        open: Token,
        shown: &str,
    ) -> ParseResult<(crate::ast::ParamRange, Vec<Param>, Token)> {
        let base = self.param_stack.len();
        let mut failed = None;
        let close = loop {
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RParen {
                self.advance();
                break token;
            }
            match self.parse_definition_param(open, shown) {
                Ok(Some(close)) => break close,
                Ok(None) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RParen) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        };
        if let Some(reported) = failed {
            return Err(reported);
        }
        // The def-type's, written by name alone, then the definition's own,
        // each with its type and its default.
        let written: Vec<Param> = self.param_stack.drain(base..).collect();
        let given_count = written
            .iter()
            .take_while(|param| param.default.is_none())
            .count();
        if let Some(late) = written
            .iter()
            .skip(given_count)
            .find(|param| param.default.is_none())
        {
            return Err(self.error(
                format!(
                    "the parameters `{shown}` gives a definition come before its own, which have a \
                     type and a default"
                ),
                late.span,
            ));
        }
        let params = self.builder.push_params(&written[..given_count]);
        let own = written.get(given_count..).unwrap_or_default().to_vec();
        Ok((params, own, close))
    }

    /// The body of a definition with the parameters `own` of its own, its
    /// body as written `body` (see [Def-types](#def-types)): a block whose
    /// result calls a function of `own`, without their defaults, whose body
    /// is `body`, with their defaults, in order, and that function, its type
    /// the checker's to give from the def-type's. So the defaults are
    /// evaluated by the definition, where its def-type calls it, and the
    /// body by the function, which a deferring attribute of the definition
    /// makes the step, given what they came to.
    fn definition_body(
        &mut self,
        body: ExprId,
        own: &[Param],
        marker: Span,
    ) -> ParseResult<(ExprId, FnId)> {
        let defaults: Vec<ExprId> = own.iter().filter_map(|param| param.default).collect();
        let params: Vec<Param> = own
            .iter()
            .map(|param| Param {
                default: None,
                ..*param
            })
            .collect();
        let params = self.builder.push_params(&params);
        let inner = self.builder.push_fn(FnDecl {
            params,
            // The def-type's to say, as the definition's own is.
            ret: None,
            effect: Effect::Impure,
            marker,
            body,
        });
        let span = self.span_of(body);
        let height = self.height_of(body).max(LEAF_HEIGHT) + 1;
        let function = self.alloc(ExprKind::Fn(inner), span, height)?;
        let tallest = defaults
            .iter()
            .map(|&default| self.height_of(default))
            .fold(height, u32::max);
        let args: Vec<Arg> = defaults
            .iter()
            .map(|&value| Arg {
                kind: ArgKind::Positional,
                value,
            })
            .collect();
        let args = self.builder.push_args(&args);
        let call = self.alloc(
            ExprKind::Call {
                callee: function,
                args,
            },
            span,
            tallest + 1,
        )?;
        let result = self.builder.alloc_item(Item {
            kind: ItemKind::Result(call),
            span,
        });
        let items = self.builder.push_item_list(&[result]);
        let block = self.alloc(ExprKind::Block { items, label: None }, span, tallest + 2)?;
        Ok((block, inner))
    }

    /// Parses a parameter of a definition made with the def-type `shown`
    /// whose parameter list `open` opens, a name alone, or one of the
    /// definition's own, `NAME: TYPE = DEFAULT`, and the `,` or the `)` after
    /// it. Returns the `)` if it ends the list.
    pub(super) fn parse_definition_param(
        &mut self,
        open: Token,
        shown: &str,
    ) -> ParseResult<Option<Token>> {
        let token = self.peek();
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => return Err(self.error(PLAIN_VARIABLE_NAME, token.span)),
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => {
                return Err(self.misplaced_mut(token, Some(MUT_PARAM_NOTE)));
            }
            _ => {
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected a parameter name, found {found}"),
                    error_span(token),
                ));
            }
        };
        self.advance();
        let colon = self.peek();
        if colon.kind == TokenKind::Colon {
            // One of the definition's own, with its type and its default.
            self.advance();
            let (ty, _) = self.parse_type()?;
            let Some(default) = self.parse_default()? else {
                return Err(self.error(
                    format!(
                        "`{}` is a parameter of the definition's own, which `{shown}` calls with \
                         none: give it a default, or leave its type to `{shown}`",
                        self.names.get(name)
                    ),
                    Span::join(token.span, self.type_span(ty)),
                ));
            };
            self.param_stack.push(Param {
                name,
                span: Span::join(token.span, self.type_span(ty)),
                ty,
                default: Some(default),
                default_fn: None,
                rest: false,
            });
            let separator = self.peek();
            return Ok(self.list_separator(open)?.then_some(separator));
        }
        let ty = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Invalid,
            span: token.span,
        });
        self.param_stack.push(Param {
            name,
            span: token.span,
            ty,
            default: None,
            default_fn: None,
            rest: false,
        });
        let separator = self.peek();
        Ok(self.list_separator(open)?.then_some(separator))
    }

    /// Parses a def-type reference, `DT PATH`, whose def-type, `dt`, has
    /// been parsed and whose path starts at the current token, an identifier
    /// on its line (see [Def-types](#def-types)): the path, and the read of
    /// the definition, a name standing for the def-type's namespace until
    /// the workspace names it, with a field access for each segment.
    pub(super) fn parse_def_type_ref(
        &mut self,
        dt: ExprId,
        variant: Option<(Name, Span)>,
    ) -> ParseResult<ExprId> {
        let segments = self.parse_def_type_path(true)?;
        let base = self.path.len();
        self.path.extend(segments.iter().copied());
        let (path, _) = self.finish_path(base);
        // The namespace's name, which no program can write; the workspace
        // replaces it with the namespace's key.
        let namespace = self.names.intern(UNRESOLVED_NAMESPACE);
        let mut read_path = Vec::with_capacity(segments.len() + 1);
        if let Some(first) = segments.first() {
            read_path.push(PathSegment {
                kind: SegmentKind::Name(namespace),
                span: Span::new(first.span.start, first.span.start),
            });
        }
        read_path.extend(segments.iter().copied());
        let read = self.path_expr(&read_path)?;
        let span = Span::join(self.span_of(dt), path_span(&segments));
        let height = self.height_of(dt).max(self.height_of(read)) + 1;
        // The key of the context its call binds to the definition, which
        // lowering finds by its text.
        self.names.intern(crate::lower::DEFINITION_CONTEXT);
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Reference,
            dt,
            variant,
            path,
            read: Some(read),
            key: None,
            meta: None,
            meta_read: None,
            body: None,
            span,
        });
        self.alloc(ExprKind::DefTypeRef(def), span, height)
    }

    /// Allocates the read of the meta of the definition whose def-type is
    /// written at `dt` and whose path is `segments`, before the workspace names
    /// what it reads (see [Def-types](#def-types)): a name standing for the
    /// namespace of the metas, written nowhere, at the start of `DT`, with one
    /// field access, spanning `DT` through the path, for the meta's leaf, which
    /// is the path as one name; both stand for nothing until the workspace
    /// names them ([`Ast::set_def_type_key`](super::Ast::set_def_type_key)).
    /// Before the definition's function in the arena, and starting before it,
    /// as its `DT` does ([`Ast::subtree`](super::Ast::subtree)).
    pub(super) fn unresolved_meta_read(
        &mut self,
        dt: Span,
        segments: &[PathSegment],
    ) -> ParseResult<ExprId> {
        let unresolved = self.names.intern(UNRESOLVED_NAMESPACE);
        self.path_expr(&[
            PathSegment {
                kind: SegmentKind::Name(unresolved),
                span: Span::new(dt.start, dt.start),
            },
            PathSegment {
                kind: SegmentKind::Name(unresolved),
                span: path_span(segments),
            },
        ])
    }

    /// Whether the current token begins a definition's meta, `defmeta DT
    /// PATH { ... }` or `defmeta DT PATH = EXPR` (see
    /// [Def-types](#def-types)): the name `defmeta`, then on its line a name
    /// or a dotted path, then on its line an identifier. `defmeta` is a name
    /// everywhere else: two names in a row after it never parsed before.
    pub(super) fn starts_defmeta(&self) -> bool {
        let keyword = self.peek();
        if !matches!(keyword.kind, TokenKind::Ident(name) if self.names.get(name) == DEFMETA) {
            return false;
        }
        if !self.is_name_after(keyword.span, self.peek_at(1)) {
            return false;
        }
        // The def-type's path: identifiers joined by `.`, then perhaps `::`
        // and a variant.
        let mut last = 1;
        while self.peek_at(last + 1).kind == TokenKind::Dot
            && matches!(self.peek_at(last + 2).kind, TokenKind::Ident(_))
        {
            last += 2;
        }
        if self.peek_at(last + 1).kind == TokenKind::ColonColon
            && matches!(self.peek_at(last + 2).kind, TokenKind::Ident(_))
        {
            last += 2;
        }
        self.is_name_after(self.peek_at(last).span, self.peek_at(last + 1))
    }

    /// Parses a definition's meta, `defmeta DT PATH { ... }` or `defmeta DT
    /// PATH = EXPR`, starting at its `defmeta`, `keyword`, which
    /// [`Parser::starts_defmeta`] has found (see [Def-types](#def-types)):
    /// its def-type, its path, as a definition's is, and its body, the block
    /// or the expression, which makes a function of no parameters whose
    /// result type and marker are the meta's its def-type declares. Anything
    /// but `{` or `=` after the path is "expected `{` or `=` after the meta's
    /// path".
    pub(super) fn parse_defmeta(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let mut dt = Vec::new();
        while let token = self.peek()
            && let TokenKind::Ident(name) = token.kind
        {
            dt.push(PathSegment {
                kind: SegmentKind::Name(name),
                span: token.span,
            });
            self.advance();
            if self.peek().kind != TokenKind::Dot
                || !matches!(self.peek_at(1).kind, TokenKind::Ident(_))
            {
                break;
            }
            self.advance();
        }
        let dt_expr = self.path_expr(&dt)?;
        let variant = self.parse_def_type_variant()?;
        let segments = self.parse_def_type_path(false)?;
        let token = self.peek();
        let body = match token.kind {
            TokenKind::LBrace => self.parse_block()?,
            // The expression, as the result of a block written nowhere.
            TokenKind::Eq => {
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                let span = self.span_of(value);
                let height = self.height_of(value);
                let result = self.builder.alloc_item(Item {
                    kind: ItemKind::Result(value),
                    span: Span::join(token.span, span),
                });
                let items = self.builder.push_item_list(&[result]);
                self.alloc(
                    ExprKind::Block { items, label: None },
                    span,
                    height.saturating_add(1),
                )?
            }
            _ => {
                if self.meta_marker().is_some() {
                    let span = Span::join(token.span, self.peek_at(1).span);
                    return Err(self.error(MISPLACED_META, span));
                }
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected `{{` or `=` after the meta's path, found {found}"),
                    error_span(token),
                ));
            }
        };
        let height = self.height_of(body).max(LEAF_HEIGHT) + 1;
        let span = Span::join(path_span(&segments), self.span_of(body));
        let params = self.builder.push_params(&[]);
        let function = self.builder.push_fn(FnDecl {
            params,
            ret: None,
            // The meta's to say, as its def-type declares it.
            effect: Effect::Pure,
            marker: self.span_of(dt_expr),
            body,
        });
        let value = self.alloc(ExprKind::Fn(function), span, height)?;
        let path_at = Span::new(keyword.span.start, path_span(&segments).end);
        let base = self.path.len();
        self.path.extend(segments);
        let (path, mut shape) = self.finish_path(base);
        // Its leaf is the path, as one name, under its namespace's root.
        shape.segments = 2;
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Meta,
            dt: dt_expr,
            variant,
            path,
            read: None,
            key: None,
            meta: None,
            meta_read: None,
            body: None,
            span: path_at,
        });
        self.finish_item(keyword, (ItemKind::DefMeta { def, value }, value), 0, shape)
    }

    /// Whether `expr` is a def-type as a reference writes it, a name or a
    /// chain of field accesses directly on one (see [Def-types](#def-types)).
    pub(super) fn is_path_expr(&self, expr: ExprId) -> bool {
        let mut at = expr;
        loop {
            match self.builder.get(at).kind {
                ExprKind::Name(_) => return true,
                ExprKind::Field { expr, .. } => at = expr,
                _ => return false,
            }
        }
    }

    /// The path `segments`, of names, as it is written.
    pub(super) fn show_segments(&self, segments: &[PathSegment]) -> String {
        segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| self.names.get(name))
            .collect::<Vec<_>>()
            .join(".")
    }
}
