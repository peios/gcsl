//! Parsing paths and names: dotted paths of `let` and fields, their segments,
//! and quoted names.
//!
//! # Paths
//!
//! A path is a name followed by any number of segments, each `.` and a
//! name or, in the path of a `:=`, an index, `[EXPR]`:
//!
//! ```text
//! PATH    = NAME SEGMENT*
//! SEGMENT = "." NAME | "[" EXPR "]"
//! ```
//!
//! as in `x`, `server.port`, `"log level".format`, `xs[i]`,
//! `conf.hosts[0].up` and `grid[y][x]`. An index segment's `[` follows the
//! rule it follows in an expression (see [Lists](crate::parser::lists#lists)): it must start on
//! the line the path is on, so a `[` after a newline begins the next item.
//! Its position is any expression, parsed as an index's is and kept in the
//! tree, and the source text it is written as is added to the string table,
//! for showing the path as it is written (see [`crate::ast::PathSegment`]);
//! a position written over several lines is shown on one, each line break,
//! with the spaces around it, one space.
//!
//! After `let` the parser reads the path directly: an identifier, then its
//! segments as they follow. A field item or a rebind has no keyword, so the
//! parser looks ahead first (`Parser::starts_path_item`): if the item's
//! tokens are a name or a plain string literal, then `.` and a name or a
//! `[`, anything and its matching `]` as many times as they follow, and
//! then `=` or `:=`, it reads the path directly in the same way, and its
//! first name is then checked as it was when the path was read back from an
//! expression: a variable's name must be an identifier, so the first
//! segment of the path of a `let` or a `:=` cannot be a literal, and a
//! field's may be a plain string literal; later segments may be quoted.
//! Otherwise the item is parsed as an expression, and an `=` or `:=` after
//! it means that it is not a path: "expected end of item", or, before a
//! chain of field accesses on a literal that cannot be its name, the error
//! for that literal. The lookahead only counts brackets, and the tokens
//! between them are then parsed as the position, so an item starting with
//! an index is looked through twice.
//!
//! A field item may be annotated, `PATH: TYPE = EXPR`, as a `let` may
//! ([`ItemKind::Field`]'s `ty`): the lookahead then finds `:` after the
//! path, and then, before the item can end (`Parser::annotated`), an `=`
//! outside any brackets, after the type. Without that `=` the item is the
//! expression it always was, the path's ascription: `x : Int` alone, a
//! block's `{ x : Int }`, or `x : Int` followed on the next line by `y =
//! 2`. The item ends, for the lookahead, at a `;`, a `:=`, a closing
//! bracket it is inside, the end of input, or a newline outside brackets
//! but after `:`, `|`, `<`, `->` or `->>`, after which the type goes on, as
//! a type itself does (newlines inside a record type's braces and a
//! function type's parentheses are skipped as the type is). An annotation
//! must be followed by
//! `=`, "expected `=` after the field's type" otherwise, and a `:=` is never
//! annotated. A quoted name may be annotated as a plain one, `"a b": Int =
//! 1`, and on a dotted path the annotation applies to the last segment, as
//! for a `let`. A `def` is not annotated: it has its signature.
//!
//! Only a `:=` path may have index segments: a `let` path with one, `let
//! xs[0] = 1`, is "a `let` path cannot index a list", and a field's, `xs[0]
//! = 1`, "a field's path cannot index a list", each at the first index's
//! brackets, with a note writing the path as a `:=`. (`let mut xs[0] = 1`
//! is the first, not "`let mut` needs a plain name".)
//!
//! A dotted path nests what its item declares: later stages build a record,
//! or a list, for each segment but the last, updating one inside another,
//! so a path of `N` segments makes its item `2(N - 1)` levels taller than
//! the taller of its value and its index segments' positions (see
//! [`MAX_DEPTH`](crate::MAX_DEPTH)).
//!
//! A newline ends an item except where the item cannot be complete: after
//! a binary operator, after `:` (in an annotation or an ascription, or as a
//! function's return marker), `->>`, `->`, `is` and `as`, and after `=`,
//! `:=`, `let`,
//! the `mut` of a `let mut` and a spread item's `...`. There
//! the parser skips newlines before going on. Directly inside parentheses,
//! brackets and interpolations the lexer emits no newlines at all, so an
//! expression, a parameter list, an argument list, a list literal or an
//! index there may span lines freely; a
//! block inside them is newline-sensitive again, so a multi-line block or
//! function body may be passed as an argument.
//!
//! # Names
//!
//! A field's name, where it is declared, accessed or written in a record
//! type, may be an identifier or a plain string literal: one without the
//! `r` prefix or interpolations, whose text the parser interns into the
//! [`Names`](super::Names) like an identifier, so `"x"` and `x` are the same name. A raw
//! or interpolated literal there is an error. A variable's name, and so a
//! parameter's, must be an identifier.

use crate::ast::{ExprId, ExprKind, ItemKind, PathRange, PathSegment, SegmentKind, path_span};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::strings::StrId;

use super::{
    DOTTED_MUT, DOTTED_MUT_NOTE, FIELD_INDEX, LET_INDEX, PLAIN_FIELD_NAME, PLAIN_VARIABLE_NAME,
    ParseResult, Parser, PathShape, Reported, error_span, one_line,
};

impl Parser<'_> {
    /// Parses the path after `let`, or after the `mut` of a `let mut`,
    /// `keyword`, as `mutable` says, starting at its first segment: an
    /// identifier, then `.` and a name, plain or quoted, as many times as
    /// they follow. Returns it and its shape.
    ///
    /// An index segment is read as in any path, and then reported at its
    /// brackets: "a `let` path cannot index a list", with a note writing the
    /// path as a `:=`. A `let mut`'s path must be a single name: a dotted one
    /// is "`let mut` needs a plain name", at the path.
    pub(super) fn parse_let_path(
        &mut self,
        keyword: Token,
        mutable: bool,
    ) -> ParseResult<(PathRange, PathShape)> {
        let name_token = self.peek();
        let name = match name_token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => {
                return Err(self.error(PLAIN_VARIABLE_NAME, name_token.span));
            }
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(name_token, None)),
            TokenKind::Label(_) => return Err(self.misplaced_label(name_token)),
            _ => {
                let after = self.describe(keyword);
                return Err(self.error(format!("expected a name after {after}"), name_token.span));
            }
        };
        self.advance();
        let base = self.path.len();
        self.path.push(PathSegment {
            kind: SegmentKind::Name(name),
            span: name_token.span,
        });
        self.parse_segments()?;
        if let Some(error) = self.indexed_path(base, LET_INDEX) {
            return Err(error);
        }
        if mutable && self.path.len() - base > 1 {
            return Err(self.report(
                Diagnostic::error(DOTTED_MUT, path_span(&self.path[base..]))
                    .with_note(DOTTED_MUT_NOTE),
            ));
        }
        Ok(self.finish_path(base))
    }

    /// Parses a field item, `PATH = EXPR` or `PATH: TYPE = EXPR`, or a
    /// rebind, `PATH := EXPR`, whose tokens [`Parser::starts_path_item`] has
    /// found to begin with a path, starting at the path's first segment.
    /// Returns its kind, its value, its path's shape and its annotation's
    /// height (0 if it has none).
    ///
    /// The path's first segment is checked after the rest of the path is
    /// read, as when the path was read back from an expression: a field's
    /// name may be a plain string literal but not a raw one, and a
    /// variable's must be an identifier. Only a rebind's path may have index
    /// segments: a field's is reported at the first, "a field's path cannot
    /// index a list", with a note writing the path as a `:=`. An annotation
    /// is followed by `=`: "expected `=` after the field's type" otherwise.
    pub(super) fn parse_path_item(&mut self) -> ParseResult<(ItemKind, ExprId, PathShape, u32)> {
        let first = self.peek();
        self.advance();
        let base = self.path.len();
        let positions = self.parse_segments()?;
        let (ty, ty_height) = if self.peek().kind == TokenKind::Colon {
            self.advance();
            self.skip_newlines();
            let (ty, height) = self.parse_type()?;
            let eq = self.peek();
            if eq.kind != TokenKind::Eq {
                let found = self.describe(eq);
                return Err(self.error(
                    format!("expected `=` after the field's type, found {found}"),
                    error_span(eq),
                ));
            }
            (Some(ty), height)
        } else {
            (None, 0)
        };
        let operator = self.peek();
        let rebind = operator.kind == TokenKind::ColonEq;
        let name = match first.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) if rebind => return Err(self.error(PLAIN_VARIABLE_NAME, first.span)),
            TokenKind::Str(text) => self.quoted_name(first, text)?,
            _ => {
                let found = self.describe(operator);
                return Err(self.error(
                    format!("expected end of item, found {found}"),
                    operator.span,
                ));
            }
        };
        self.path.insert(
            base,
            PathSegment {
                kind: SegmentKind::Name(name),
                span: first.span,
            },
        );
        if !rebind && let Some(error) = self.indexed_path(base, FIELD_INDEX) {
            return Err(error);
        }
        let (path, mut shape) = self.finish_path(base);
        shape.positions = positions;
        self.advance();
        let value = self.parse_value()?;
        let kind = if rebind {
            ItemKind::Rebind { path, value }
        } else {
            ItemKind::Field {
                path,
                ty,
                value,
                function: false,
            }
        };
        Ok((kind, value, shape, ty_height))
    }

    /// Parses the segments of a path after its first, onto `path`, while
    /// they follow: `.` and a name, plain or quoted, or an index, `[`, a
    /// position and `]`, which, as in an expression, must start on the line
    /// the path is on, a newline ending the path. Returns the height of the
    /// tallest position, 0 if there is none.
    pub(super) fn parse_segments(&mut self) -> ParseResult<u32> {
        let mut tallest = 0;
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.advance();
                    let segment = self.peek();
                    let Some(name) = self.field_name()? else {
                        return Err(
                            self.error("expected a field name after `.`", error_span(segment))
                        );
                    };
                    self.path.push(PathSegment {
                        kind: SegmentKind::Name(name),
                        span: segment.span,
                    });
                }
                TokenKind::LBracket => {
                    let (position, span) = self.parse_position()?;
                    tallest = tallest.max(self.height_of(position));
                    let written = self.source.slice(self.span_of(position));
                    let text = if written.contains('\n') {
                        self.strings.push(&one_line(written))
                    } else {
                        self.strings.push(written)
                    };
                    self.path.push(PathSegment {
                        kind: SegmentKind::Index { position, text },
                        span,
                    });
                }
                _ => return Ok(tallest),
            }
        }
    }

    /// The error `message` at the first index segment of the path being
    /// read from `base` on, if it has one, which may not: "a `let` path
    /// cannot index a list" or "a field's path cannot index a list", with a
    /// note writing the path as it is written, as a `:=`.
    pub(super) fn indexed_path(&mut self, base: usize, message: &str) -> Option<Reported> {
        let path = self.path.get(base..)?;
        let index = path.iter().find(|segment| segment.position().is_some())?;
        let error = Diagnostic::error(message, index.span).with_note(format!(
            "update the element with `{} := ...`",
            one_line(self.source.slice(path_span(path)))
        ));
        Some(self.report(error))
    }

    /// The error for `expr`, parsed at the start of an item and followed by
    /// `operator`, the `=` of a field item or the `:=` of a rebind as
    /// `rebind` says, which is not a path, since [`Parser::starts_path_item`]
    /// found none: "expected end of item" at the operator, unless it is a
    /// chain of field accesses on a literal that is not a name, a raw or
    /// interpolated literal for a field and any literal for a variable.
    pub(super) fn not_a_path(&mut self, expr: ExprId, operator: Token, rebind: bool) -> Reported {
        let mut node = expr;
        loop {
            let inner = *self.builder.get(node);
            match inner.kind {
                ExprKind::Field { expr, .. } => node = expr,
                ExprKind::Str(_) | ExprKind::Interp(_) if rebind => {
                    return self.error(PLAIN_VARIABLE_NAME, inner.span);
                }
                ExprKind::Interp(_) => return self.error(PLAIN_FIELD_NAME, inner.span),
                _ => {
                    let found = self.describe(operator);
                    return self.error(
                        format!("expected end of item, found {found}"),
                        operator.span,
                    );
                }
            }
        }
    }

    /// Moves the path being read, the segments of `path` from `base` on, to
    /// the builder's path pool, returning its range and its shape, with no
    /// positions: the caller knows how tall they are.
    pub(super) fn finish_path(&mut self, base: usize) -> (PathRange, PathShape) {
        let segments = self.path.get(base..).unwrap_or_default();
        let shape = PathShape {
            segments: u32::try_from(segments.len()).unwrap_or(u32::MAX),
            positions: 0,
        };
        let range = self.builder.push_path(segments);
        self.path.truncate(base);
        (range, shape)
    }

    /// The name a plain string literal `token`, of text `text`, gives a
    /// field: its text, interned. A raw literal is an error.
    pub(super) fn quoted_name(&mut self, token: Token, text: StrId) -> ParseResult<Name> {
        if self.source.slice(token.span).starts_with('r') {
            return Err(self.error(PLAIN_FIELD_NAME, token.span));
        }
        // A name of the form of a def-type's namespace key, or of a meta key,
        // could reach its definitions or their metas: no source may write
        // one (see [Def-types](crate::parser::deftypes#def-types)).
        let text = self.strings.get(text);
        if crate::names::is_namespace_key(text) {
            let message = format!(
                "a field cannot be named `{}`: names of that form are def-types' namespaces",
                crate::strings::FieldName(text)
            );
            return Err(self.error(message, token.span));
        }
        self.builder.push_quoted(token.span);
        Ok(self.names.intern(text))
    }

    /// Parses the name of a field after `.` or in a record type, the
    /// current token: an identifier or a plain string literal. Returns
    /// `None`, having consumed nothing, if the token is neither; a raw or
    /// interpolated literal is an error, and so is a `mut`.
    pub(super) fn field_name(&mut self) -> ParseResult<Option<Name>> {
        let token = self.peek();
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(text) => self.quoted_name(token, text)?,
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_FIELD_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(token, None)),
            _ => return Ok(None),
        };
        self.advance();
        Ok(Some(name))
    }
}
