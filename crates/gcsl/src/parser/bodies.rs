//! Parsing items and bodies: a file's or a block's items, `let`, fields and
//! their annotations, and the end of an item.
//!
//! # Items and bodies
//!
//! A file, like the inside of a block `{ ... }`, is a *body*: a sequence of
//! items separated by newlines or `;` (`Parser::parse_body`). Runs of
//! separators, and so blank and comment-only lines, are skipped. An item is
//! `let PATH (: TYPE)? = EXPR`, `let mut NAME (: TYPE)? = EXPR`, `PATH :=
//! EXPR`, a field `PATH (: TYPE)? = EXPR`, `= EXPR`, a spread `...EXPR`, a
//! named function `fn NAME(...) ...`, a `break`, a `continue` or a `return`
//! (see [Loops](crate::parser::loops#loops)), an expression, or, in a file's
//! body only, an `import` (see [Imports](crate::parser::imports#imports)), a
//! `type` (see [Types](crate::parser::types#types)) or a definition made with a
//! def-type (see [Def-types](crate::parser::deftypes#def-types)). After an item
//! the next token must be a separator or the end of the body; anything else is
//! "expected end of item".
//!
//! `let mut` declares a mutable variable: an ordinary `let` item, marked as
//! mutable (see [`ItemKind::Let`]). Its path must be a single name: `let mut a.b = e` is "`let mut`
//! needs a
//! plain name", at the path, since a dotted `let` updates a variable rather
//! than declaring one. `mut` is allowed nowhere else, and is "`mut` is only
//! allowed after `let`" at the `mut` wherever an item, an expression, a
//! name, a type or a function's parameter list would start, or an item
//! would end: before a field (`mut x = 1`) and a parameter (`fn(mut n:
//! Int)`) with a note saying what to write instead, and before a `for`'s
//! variable, after `fn` and everywhere else of those without one. (Where a
//! particular token is expected after a complete part of an item or an
//! expression, a separator, a closing bracket, `=`, `in` or a block, as in
//! `f(x mut)` or `let x mut = 1`, the usual "expected ..., found `mut`"
//! stands.)
//!
//! A spread item is recognised by `...` at the start of an item, and its
//! expression is parsed as a field's value is, at minimum binding power 0,
//! so `...conf : {b: Int}` spreads `conf : {b: Int}`. Whether the block it
//! is in may have a result is the checker's business, as for a field.
//!
//! A named function is recognised by `fn` followed by an identifier at the
//! start of an item, and is parsed as `let NAME = <the function expression
//! from its '(' on>`, both spanning `fn` through the body's `}`. `fn`
//! followed by anything else starts an ordinary expression item.
//!
//! A public function, `def PATH PARAMS RETURN BODY`, is recognised by `def`
//! at the start of an item, and has exactly a named function's grammar,
//! with the same three markers, but that it is named by a path; it is
//! parsed as the field `PATH = <the function expression from its '(' on>`.
//! Both forms are marked as written (`function` on [`ItemKind::Let`] and
//! [`ItemKind::Field`]), so the tree tells them from `let NAME = fn ...`
//! and `PATH = fn ...`. A `def`'s path is made of identifiers, `def
//! hello`, `def dev.peios.hello`: one that starts with a string is "a `def`
//! is named by an identifier, not a string", a quoted or an index segment
//! after its first, `def a."b"` or `def a[0]`, is "a `def`'s path is made
//! of identifiers", at the segment, and anything else where a name is
//! expected, after `def` or after a `.`, is an error at it. `def` is a
//! keyword, so anywhere else, as a name or an expression, it is reported as
//! any keyword is there.

use super::Sync;
use crate::MAX_DEPTH;
use crate::ast::{ExprId, Item, ItemId, ItemKind, ItemRange, Syntax};
use crate::diagnostic::Diagnostic;
use crate::lexer::{MAX_ERRORS, Token, TokenKind};
use crate::source::Span;

use super::{
    MISPLACED_CONTEXT, MISPLACED_DEFMETA, MISPLACED_DEF_TYPE_DEF, MISPLACED_ENUM, MISPLACED_IMPORT,
    MISPLACED_METHOD, MISPLACED_MUT, MISPLACED_TYPE, MUT_FIELD_NOTE, ParseResult, Parser,
    PathShape, Pending, Recovery, Reported,
};

impl Parser<'_> {
    /// Parses a body up to its end: the `}` matching `open` (not consumed),
    /// or the end of input for the file's body, when `open` is `None`.
    /// Returns the body and the height of its tallest item expression (0 if
    /// it has no items).
    ///
    /// An item that fails is an error item, spanning it and what is skipped
    /// after it to the item's end (see [Errors and
    /// recovery](crate::parser::recovery#errors-and-recovery)), and so is the item after an
    /// attribute that is an error. A block's body ends at the end of the
    /// input too, which is reported once, "expected `}`", for the innermost
    /// block; and a `}` in the file's body is reported, "unexpected `}`",
    /// and skipped. Either makes the tree [`Syntax::Unbalanced`].
    pub(super) fn parse_body(&mut self, open: Option<Span>) -> (ItemRange, u32) {
        let base = self.item_stack.len();
        let mut height = 0;
        let top = open.is_none();
        let recovery = std::mem::replace(&mut self.recovery, Recovery::Body { top });
        // The outer attributes waiting for the item they apply to, and
        // whether one was an error, which makes it an error item.
        let mut pending = Pending::default();
        let mut poisoned = false;
        loop {
            self.skip_separators();
            let token = self.peek();
            if self.diagnostics.len() > MAX_ERRORS {
                // Too many errors: the rest is not parsed.
                self.pos = self.tokens.len().max(self.pos);
                break;
            }
            let first = open.is_none() && self.item_stack.len() == base;
            if let Some(failed) = self.body_attribute(token, &mut pending, first) {
                poisoned |= failed;
                continue;
            }
            match (token.kind, open) {
                (TokenKind::Eof, None) | (TokenKind::RBrace, Some(_)) => {
                    if !poisoned && let Some((written, attribute)) = pending.first(self.names) {
                        self.error(
                            format!("`#[{written}]` is not followed by anything"),
                            attribute,
                        );
                    }
                    break;
                }
                (TokenKind::Eof, Some(open)) => {
                    self.builder.set_syntax(Syntax::Unbalanced);
                    if !self.unclosed {
                        self.unclosed = true;
                        let found = self.describe(token);
                        self.report(
                            Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                                .with_context(open, "unclosed `{`"),
                        );
                    }
                    break;
                }
                (TokenKind::RBrace, None) => {
                    self.builder.set_syntax(Syntax::Unbalanced);
                    self.error("unexpected `}`", token.span);
                    self.advance();
                    continue;
                }
                _ => {}
            }
            let start = self.pos;
            let marks = self.marks();
            let waiting = pending.clone();
            let mut parsed = self.parse_body_item(token, open.is_some(), &mut pending);
            // Gone on to a line that is an item of its own: parsed again to
            // the end of its own line, and the next line left for the next
            // item.
            if parsed.is_err()
                && let Some(line) = self.item_line(start)
            {
                self.rewind(start, marks);
                pending = waiting;
                let outer = self.line_end.replace(line);
                parsed = self.parse_body_item(token, open.is_some(), &mut pending);
                self.line_end = outer;
            }
            let (item, item_height) = match parsed {
                Ok(parsed) if !poisoned => parsed,
                Ok(_) | Err(Reported) => {
                    pending = Pending::default();
                    let item = self.error_item(start, marks, Sync::Item { top });
                    // An item starts at no separator, closing brace or end
                    // of input, where a skip stops, so it consumes a token
                    // and the loop ends; this keeps it so whatever happens.
                    let stops = matches!(
                        self.peek().kind,
                        TokenKind::Eof | TokenKind::RBrace | TokenKind::Newline | TokenKind::Semi
                    );
                    if self.pos == start && !stops {
                        self.advance();
                    }
                    (item, 0)
                }
            };
            poisoned = false;
            self.item_stack.push(item);
            height = height.max(item_height);
        }
        self.recovery = recovery;
        let range = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        (range, height)
    }

    /// Parses the item of a body, a block's if `inner`, starting at
    /// `token`, which is followed by the end of the item, and applies the
    /// outer attributes `pending` to it. Returns it and its height.
    pub(super) fn parse_body_item(
        &mut self,
        token: Token,
        inner: bool,
        pending: &mut Pending,
    ) -> ParseResult<(ItemId, u32)> {
        let (item, item_height) = match token.kind {
            TokenKind::Import if inner => return Err(self.error(MISPLACED_IMPORT, token.span)),
            TokenKind::Import => (self.parse_import(token)?, 0),
            _ if self.starts_type_item() => {
                if inner {
                    return Err(self.error(MISPLACED_TYPE, token.span));
                }
                self.parse_type_item(token)?
            }
            _ if self.starts_enum_item() => {
                if inner {
                    return Err(self.error(MISPLACED_ENUM, token.span));
                }
                self.parse_enum_item(token)?
            }
            _ if self.starts_assert() => (self.parse_assert(token)?, 0),
            _ if self.starts_context_item() => {
                if inner {
                    return Err(self.error(MISPLACED_CONTEXT, token.span));
                }
                self.parse_context_item(token)?
            }
            _ if self.starts_method(self.pos) => {
                if inner {
                    return Err(self.error(MISPLACED_METHOD, token.span));
                }
                self.parse_method(token)?
            }
            _ if self.starts_defmeta() => {
                if inner {
                    return Err(self.error(MISPLACED_DEFMETA, token.span));
                }
                self.parse_defmeta(token)?
            }
            _ => self.parse_item()?,
        };
        if inner
            && let item = *self.builder.item(item)
            && matches!(item.kind, ItemKind::DefTypeDef { .. })
        {
            // At its `def`.
            let start = item.span.start;
            let keyword = Span::new(start, start.saturating_add(3));
            return Err(self.error(MISPLACED_DEF_TYPE_DEF, keyword));
        }
        if let Some(attribute) = pending.shadowable.take() {
            self.attach_shadowable(attribute, item, !inner)?;
        }
        if let Some(attribute) = pending.def_type.take() {
            self.attach_def_type(attribute, item, !inner)?;
        }
        if let Some(attribute) = pending.attribute.take() {
            self.attach_attribute(attribute, item, !inner)?;
        }
        if let Some(defers) = pending.defers.take() {
            self.attach_defers(defers, item, !inner)?;
        }
        if !pending.uses.is_empty() {
            self.attach_uses(std::mem::take(&mut pending.uses), item)?;
        }
        self.expect_end_of_item()?;
        Ok((item, item_height))
    }

    /// Parses one item. Returns it and its height: the taller of its
    /// expression and the positions of its path's index segments, plus two
    /// levels for each segment of its path after the first, or its
    /// annotation's if that is taller.
    pub(super) fn parse_item(&mut self) -> ParseResult<(ItemId, u32)> {
        let token = self.peek();
        let name_token = self.peek_at(1);
        if token.kind == TokenKind::Fn
            && let TokenKind::Ident(name) = name_token.kind
        {
            return self.parse_named_fn(token, (name, name_token.span));
        }
        if token.kind == TokenKind::Def {
            return self.parse_def(token);
        }
        if token.kind == TokenKind::Mut {
            let note = self.starts_field(1).then_some(MUT_FIELD_NOTE);
            return Err(self.misplaced_mut(token, note));
        }
        let (kind, value) = match token.kind {
            TokenKind::Let => {
                let (kind, value, ty_height, path_shape) = self.parse_let(token)?;
                return self.finish_item(token, (kind, value), ty_height, path_shape);
            }
            TokenKind::Break | TokenKind::Continue | TokenKind::Return => {
                return self.parse_jump(token);
            }
            TokenKind::Label(_) => return Err(self.misplaced_label(token)),
            TokenKind::Eq => {
                self.advance();
                let value = self.parse_value()?;
                (ItemKind::Result(value), value)
            }
            TokenKind::DotDotDot => {
                self.advance();
                let value = self.parse_value()?;
                (ItemKind::Spread(value), value)
            }
            _ if self.starts_path_item() => {
                let (kind, value, path, ty_height) = self.parse_path_item()?;
                return self.finish_item(token, (kind, value), ty_height, path);
            }
            _ => {
                let expr = self.parse_expr(0)?;
                let operator = self.peek();
                let rebind = match operator.kind {
                    TokenKind::Eq => false,
                    TokenKind::ColonEq => true,
                    _ => {
                        let kind = (ItemKind::Expr(expr), expr);
                        return self.finish_item(token, kind, 0, PathShape::SINGLE);
                    }
                };
                return Err(self.not_a_path(expr, operator, rebind));
            }
        };
        self.finish_item(token, (kind, value), 0, PathShape::SINGLE)
    }

    /// Parses a `let` item, `let PATH (: TYPE)? = EXPR` or `let mut NAME (:
    /// TYPE)? = EXPR`, starting at its `let`, `token`. Returns its kind, its
    /// value, its annotation's height (0 if it has none) and its path's
    /// shape.
    pub(super) fn parse_let(
        &mut self,
        token: Token,
    ) -> ParseResult<(ItemKind, ExprId, u32, PathShape)> {
        self.advance();
        self.skip_newlines();
        let mutable = self.peek().kind == TokenKind::Mut;
        let keyword = if mutable {
            let keyword = self.peek();
            self.advance();
            self.skip_newlines();
            keyword
        } else {
            token
        };
        let (path, shape) = self.parse_let_path(keyword, mutable)?;
        let (ty, ty_height) = if self.peek().kind == TokenKind::Colon {
            self.advance();
            self.skip_newlines();
            let (ty, height) = self.parse_type()?;
            (Some(ty), height)
        } else {
            (None, 0)
        };
        let eq = self.peek();
        if eq.kind != TokenKind::Eq {
            return Err(self.error("expected `=` after variable name", eq.span));
        }
        self.advance();
        let value = self.parse_value()?;
        let kind = ItemKind::Let {
            path,
            ty,
            value,
            mutable,
            function: false,
        };
        Ok((kind, value, ty_height, shape))
    }

    /// Whether the tokens from `ahead` places after the current one begin a
    /// field item: a path, a name or a plain string literal and then `.`
    /// and a name as many times as they follow, and then `=`.
    pub(super) fn starts_field(&self, ahead: usize) -> bool {
        self.path_end(ahead, false)
            .is_some_and(|end| self.peek_at(end).kind == TokenKind::Eq)
    }

    /// Whether the current token begins a field item or a rebind whose
    /// path is written as one: a name or a plain string literal, then `.`
    /// and a name or `[`, anything and the matching `]` as many times as
    /// they follow, and then `=` or `:=`, or `:` and an annotation followed
    /// by `=` ([`Parser::annotated`]). Such an item is read by
    /// [`Parser::parse_path_item`]; any other item that turns out to be
    /// followed by `=` or `:=` is not a path (see [`Parser::not_a_path`]).
    /// The tokens between brackets are only counted here, and parsed as
    /// the position afterwards, so an item starting with an index is looked
    /// through twice.
    pub(super) fn starts_path_item(&self) -> bool {
        self.path_end(0, true)
            .is_some_and(|end| match self.peek_at(end).kind {
                TokenKind::Eq | TokenKind::ColonEq => true,
                TokenKind::Colon => self.annotated(end + 1),
                _ => false,
            })
    }

    /// Whether the tokens from `ahead` places after the current one, which
    /// follow a path and its `:`, are an annotation followed by the `=` of
    /// an annotated field, rather than the type of an ascription (see
    /// [Paths](crate::parser::paths#paths)): an `=` comes before the item can end, outside any
    /// brackets. The item ends at a `;`, a `:=`, a closing bracket that
    /// opened before it or the end of input, or a newline outside brackets
    /// that does not follow `:`, `|`, `<`, `->` or `->>`, after which a
    /// type goes on. The tokens are only counted here.
    pub(super) fn annotated(&self, ahead: usize) -> bool {
        let mut depth = 0_usize;
        let mut previous = TokenKind::Colon;
        for at in ahead.. {
            let kind = self.peek_at(at).kind;
            match kind {
                TokenKind::Eq if depth == 0 => return true,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    let Some(outer) = depth.checked_sub(1) else {
                        return false;
                    };
                    depth = outer;
                }
                TokenKind::Newline if depth > 0 => continue,
                TokenKind::Newline => {
                    let goes_on = matches!(
                        previous,
                        TokenKind::Colon
                            | TokenKind::Pipe
                            | TokenKind::Lt
                            | TokenKind::Arrow
                            | TokenKind::MemoArrow
                    );
                    if goes_on {
                        continue;
                    }
                    return false;
                }
                TokenKind::Semi | TokenKind::ColonEq if depth == 0 => return false,
                TokenKind::Eof => return false,
                _ => {}
            }
            previous = kind;
        }
        false
    }

    /// How many places after the current token the tokens from `ahead`
    /// places after it stop spelling a path: a name or a plain string
    /// literal, then `.` and a name and, if `indexes`, `[` and everything up
    /// to the matching `]`, as many times as they follow. `None` if they do
    /// not start with a name, or a `[` is never closed.
    pub(super) fn path_end(&self, ahead: usize, indexes: bool) -> Option<usize> {
        let is_name = |token: Token| matches!(token.kind, TokenKind::Ident(_) | TokenKind::Str(_));
        let mut at = ahead;
        if !is_name(self.peek_at(at)) {
            return None;
        }
        at += 1;
        loop {
            match self.peek_at(at).kind {
                TokenKind::Dot if is_name(self.peek_at(at + 1)) => at += 2,
                TokenKind::LBracket if indexes => at = self.after_brackets(at)?,
                _ => return Some(at),
            }
        }
    }

    /// How many places after the current token the token after the `]`
    /// matching the `[` `open` places after it is, counting brackets alone;
    /// `None` if the input ends first.
    pub(super) fn after_brackets(&self, open: usize) -> Option<usize> {
        let mut depth = 0_usize;
        let mut at = open;
        loop {
            match self.peek_at(at).kind {
                TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(at + 1);
                    }
                }
                TokenKind::Eof => return None,
                _ => {}
            }
            at += 1;
        }
    }

    /// Reports `token`, a `mut` where it is not allowed, with `note` if one
    /// says what to write instead.
    pub(super) fn misplaced_mut(&mut self, token: Token, note: Option<&str>) -> Reported {
        let error = Diagnostic::error(MISPLACED_MUT, token.span);
        self.report(match note {
            Some(note) => error.with_note(note),
            None => error,
        })
    }

    /// Allocates the item of `kind`, starting at `token` and ending with its
    /// expression `value`, whose annotation, if any, is `ty_height` tall and
    /// whose path has the shape `path`. Returns it and its height, failing
    /// if that exceeds [`MAX_DEPTH`].
    pub(super) fn finish_item(
        &mut self,
        token: Token,
        (kind, value): (ItemKind, ExprId),
        ty_height: u32,
        path: PathShape,
    ) -> ParseResult<(ItemId, u32)> {
        let span = Span::join(token.span, self.span_of(value));
        let path_height = path.segments.saturating_sub(1).saturating_mul(2);
        let height = self
            .height_of(value)
            .max(path.positions)
            .saturating_add(path_height)
            .max(ty_height);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let item = self.builder.alloc_item(Item { kind, span });
        Ok((item, height))
    }

    /// Checks that the item just parsed is followed by a separator or the
    /// end of its body.
    pub(super) fn expect_end_of_item(&mut self) -> ParseResult<()> {
        let token = self.peek();
        match token.kind {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => Ok(()),
            // `a -b`: the user most likely meant a subtraction. The lexer
            // only makes a `Neg` when an operand follows directly, so show
            // the `-` together with that operand's first token.
            TokenKind::Neg => {
                let span = Span::join(token.span, self.peek_at(1).span);
                let found = self.source.slice(span);
                Err(self.report(
                    Diagnostic::error(format!("expected end of item, found `{found}`"), span)
                        .with_note("write `a - b` with spaces for subtraction"),
                ))
            }
            TokenKind::Mut => Err(self.misplaced_mut(token, None)),
            TokenKind::Label(_) => Err(self.misplaced_label(token)),
            _ => {
                let found = self.describe(token);
                Err(self.error(format!("expected end of item, found {found}"), token.span))
            }
        }
    }
}
